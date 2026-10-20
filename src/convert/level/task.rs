// Derived from Chunker (https://github.com/HiveGamesOSS/Chunker), Copyright (c) 2024 Hive Games, MIT License.
//! The task machinery: Chunker's `scheduling` package without `CompletableFuture`.
//!
//! A conversion is a tree of tasks run by a bounded pool of `std::thread` workers.
//!
//! - **Tree**: a running task spawns children through its [`TaskContext`]; a task completes only when its body and
//!   all its descendants completed. Bodies never block waiting for children (so one worker cannot deadlock); work
//!   that must follow other work is expressed with [`TaskContext::spawn_after`].
//! - **Priority** (Chunker's `(depth << 48) | counter`, highest first): deeper tasks run first and, among equal
//!   depth, the newest one first. Finishing the columns that are already read before reading more regions is what
//!   bounds memory.
//! - **Progress**: a task is 0 until started and 1 when done; a started task is `sum(child * weight) / (1 + sum of
//!   child weights)`. Finished children are folded into one number so the tree stays small.
//! - **Failure**: the first error (or panic) cancels everything that has not started and is the result. A
//!   [`CancelToken`] is checked between tasks and is exposed to long loops through [`TaskContext::is_cancelled`].
//! - **Events**: an optional `std::sync::mpsc` channel receives [`ProgressEvent`]s (throttled to one per 50 ms).
//!
//! Note: the release profile uses `panic = "abort"`; a panic is only caught (and reported as
//! [`ConvertError::Internal`]) in builds that unwind, such as the tests.

use std::any::Any;
use std::cmp::Ordering as CmpOrdering;
use std::collections::{BTreeMap, BinaryHeap, HashMap};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, Weak};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use super::convert_api::ConvertError;

/// Cancels a conversion when set to `true`.
pub type CancelToken = Arc<AtomicBool>;

/// A new, unset cancel token.
pub fn new_cancel_token() -> CancelToken {
    Arc::new(AtomicBool::new(false))
}

/// The number of worker threads used by default: `min(available_parallelism, 8)`.
pub fn default_thread_count() -> usize {
    std::thread::available_parallelism().map_or(1, |n| n.get()).min(8)
}

/// How much a task counts towards its parent's progress (`TaskWeight`). Any number works; these are Chunker's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Weight(pub u32);

impl Weight {
    pub const NONE: Weight = Weight(0);
    pub const LOW: Weight = Weight(1);
    pub const MEDIUM: Weight = Weight(50);
    pub const NORMAL: Weight = Weight(100);
    pub const HIGH: Weight = Weight(200);
    pub const HIGHER: Weight = Weight(500);
    pub const HIGHEST: Weight = Weight(1000);
}

type Body = Box<dyn FnOnce(&TaskContext) -> Result<(), ConvertError> + Send + 'static>;

const PENDING: u8 = 0;
const STARTED: u8 = 1;
const DONE: u8 = 2;

#[derive(Default)]
struct Children {
    live: HashMap<u64, Arc<Node>>,
    /// The summed weight of finished children (each counts as fully done).
    done_weight: u64,
}

struct Node {
    id: u64,
    name: String,
    weight: u32,
    depth: u32,
    /// Creation order, the tie-break of the queue.
    seq: u64,
    state: AtomicU8,
    /// 1 for the body (or placeholder) plus the unfinished children.
    pending: AtomicUsize,
    parent: Weak<Node>,
    children: Mutex<Children>,
    /// Nodes waiting for this one; `None` once this node completed.
    waiters: Mutex<Option<Vec<Arc<Node>>>>,
    /// How many dependencies this node still waits for before it is queued.
    waiting: AtomicUsize,
    body: Mutex<Option<Body>>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl Node {
    fn progress(&self) -> f64 {
        match self.state.load(Ordering::Acquire) {
            PENDING => 0.0,
            DONE => 1.0,
            _ => {
                let children = lock(&self.children);
                let mut numerator = children.done_weight as f64;
                let mut denominator = 1.0 + children.done_weight as f64;
                for child in children.live.values() {
                    let weight = child.weight as f64;
                    if weight > 0.0 {
                        numerator += child.progress() * weight;
                        denominator += weight;
                    }
                }
                numerator / denominator
            }
        }
    }
}

/// A queued node ordered by `(depth, seq)`, the largest first.
struct Item {
    depth: u32,
    seq: u64,
    node: Arc<Node>,
}

impl PartialEq for Item {
    fn eq(&self, other: &Self) -> bool {
        self.depth == other.depth && self.seq == other.seq
    }
}
impl Eq for Item {}
impl PartialOrd for Item {
    fn partial_cmp(&self, other: &Self) -> Option<CmpOrdering> {
        Some(self.cmp(other))
    }
}
impl Ord for Item {
    fn cmp(&self, other: &Self) -> CmpOrdering {
        (self.depth, self.seq).cmp(&(other.depth, other.seq))
    }
}

#[derive(Default)]
struct Queue {
    heap: BinaryHeap<Item>,
    /// Bodies currently running.
    running: usize,
    shutdown: bool,
}

/// How a run ended, sent as the last event.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FinishStatus {
    Done,
    Cancelled,
    Failed(String),
}

/// A copy of the progress at one moment.
#[derive(Clone, Debug, PartialEq)]
pub struct ProgressSnapshot {
    pub fraction: f64,
    pub stage: String,
    pub counters: Vec<(String, u64)>,
}

/// What the event channel receives.
#[derive(Clone, Debug, PartialEq)]
pub enum ProgressEvent {
    /// Progress changed (at most every 50 ms, and the final 1.0).
    Progress(ProgressSnapshot),
    /// A named state change such as `("Compacting output", true)`.
    Signal { name: String, active: bool },
    /// A non-fatal problem.
    Warning(String),
    /// The run ended; always the last event.
    Finished(FinishStatus),
}

/// A live view of the progress of a run; cheap to clone and safe to poll from a GUI thread.
#[derive(Clone)]
pub struct Progress {
    shared: Arc<Shared>,
}

impl Progress {
    /// Overall progress in `0.0..=1.0`; exactly 1.0 once the run succeeded.
    pub fn fraction(&self) -> f64 {
        match self.shared.root.get() {
            Some(root) => root.progress().clamp(0.0, 1.0),
            None => 0.0,
        }
    }

    /// The current stage text.
    pub fn stage(&self) -> String {
        lock(&self.shared.stage).clone()
    }

    /// The named counters, sorted by name.
    pub fn counters(&self) -> Vec<(String, u64)> {
        lock(&self.shared.counters).iter().map(|(name, value)| (name.clone(), value.load(Ordering::Relaxed))).collect()
    }

    /// The value of one counter (0 when it does not exist).
    pub fn counter(&self, name: &str) -> u64 {
        lock(&self.shared.counters).get(name).map_or(0, |c| c.load(Ordering::Relaxed))
    }

    /// Everything at once.
    pub fn snapshot(&self) -> ProgressSnapshot {
        ProgressSnapshot { fraction: self.fraction(), stage: self.stage(), counters: self.counters() }
    }
}

/// A named counter; clone it into tasks and call [`Counter::add`].
#[derive(Clone)]
pub struct Counter(Arc<AtomicU64>);

impl Counter {
    pub fn add(&self, amount: u64) {
        self.0.fetch_add(amount, Ordering::Relaxed);
    }
    pub fn get(&self) -> u64 {
        self.0.load(Ordering::Relaxed)
    }
}

struct Shared {
    queue: Mutex<Queue>,
    work: Condvar,
    seq: AtomicU64,
    node_ids: AtomicU64,
    cancel: CancelToken,
    failure: Mutex<Option<ConvertError>>,
    aborted: AtomicBool,
    root: std::sync::OnceLock<Arc<Node>>,
    finished: Mutex<bool>,
    finished_cv: Condvar,
    events: Option<Mutex<Sender<ProgressEvent>>>,
    emit_state: Mutex<Instant>,
    emit_interval: Duration,
    stage: Mutex<String>,
    counters: Mutex<BTreeMap<String, Arc<AtomicU64>>>,
    warnings: Mutex<Vec<String>>,
    tasks_run: AtomicU64,
    started: Instant,
}

impl Shared {
    fn send(&self, event: ProgressEvent) {
        if let Some(events) = &self.events {
            // A dropped receiver is not an error.
            let _ = lock(events).send(event);
        }
    }

    fn snapshot(self: &Arc<Self>) -> ProgressSnapshot {
        Progress { shared: self.clone() }.snapshot()
    }

    /// Sends a progress event unless one was sent in the last interval (`force` skips the throttle).
    fn emit(self: &Arc<Self>, force: bool) {
        if self.events.is_none() {
            return;
        }
        {
            let mut last = lock(&self.emit_state);
            if !force && last.elapsed() < self.emit_interval {
                return;
            }
            *last = Instant::now();
        }
        let snapshot = self.snapshot();
        self.send(ProgressEvent::Progress(snapshot));
    }

    fn new_node(&self, name: String, weight: u32, parent: Option<&Arc<Node>>, body: Option<Body>) -> Arc<Node> {
        Arc::new(Node {
            id: self.node_ids.fetch_add(1, Ordering::Relaxed),
            name,
            weight,
            depth: parent.map_or(0, |p| p.depth + 1),
            seq: self.seq.fetch_add(1, Ordering::Relaxed),
            state: AtomicU8::new(PENDING),
            pending: AtomicUsize::new(1),
            parent: parent.map_or_else(Weak::new, Arc::downgrade),
            children: Mutex::new(Children::default()),
            waiters: Mutex::new(Some(Vec::new())),
            waiting: AtomicUsize::new(0),
            body: Mutex::new(body),
        })
    }

    fn enqueue(&self, node: Arc<Node>) {
        if self.aborted.load(Ordering::Acquire) {
            return;
        }
        let mut queue = lock(&self.queue);
        queue.heap.push(Item { depth: node.depth, seq: node.seq, node });
        drop(queue);
        self.work.notify_one();
    }

    /// Records the first failure and aborts: nothing that has not started will run.
    fn fail(&self, error: ConvertError) {
        {
            let mut failure = lock(&self.failure);
            if failure.is_none() {
                *failure = Some(error);
            }
        }
        self.cancel.store(true, Ordering::Release);
        self.aborted.store(true, Ordering::Release);
        self.work.notify_all();
    }

    /// Shuts the pool down when the root is done, or when aborted and nothing is running. Call with the queue locked;
    /// returns true for the one caller that made the transition, which must then call `finish_run`.
    fn check_finish(&self, queue: &mut Queue) -> bool {
        if queue.shutdown {
            return false;
        }
        let root_done = self.root.get().is_some_and(|r| r.state.load(Ordering::Acquire) == DONE);
        if root_done || (self.aborted.load(Ordering::Acquire) && queue.running == 0) {
            queue.shutdown = true;
            queue.heap.clear();
            self.work.notify_all();
            return true;
        }
        false
    }

    fn finish_run(self: &Arc<Self>) {
        let status = match lock(&self.failure).as_ref() {
            Some(ConvertError::Cancelled) => FinishStatus::Cancelled,
            Some(error) => FinishStatus::Failed(error.to_string()),
            None if self.aborted.load(Ordering::Acquire) => FinishStatus::Cancelled,
            None => FinishStatus::Done,
        };
        if status == FinishStatus::Done {
            self.emit(true);
        }
        self.send(ProgressEvent::Finished(status));
        *lock(&self.finished) = true;
        self.finished_cv.notify_all();
    }

    /// One unit of a node's pending count is done; completes the node (and its ancestors) at zero.
    fn release(self: &Arc<Self>, node: &Arc<Node>) {
        let mut current = node.clone();
        loop {
            if current.pending.fetch_sub(1, Ordering::AcqRel) != 1 {
                return;
            }
            current.state.store(DONE, Ordering::Release);
            let waiters = lock(&current.waiters).take();
            for waiter in waiters.into_iter().flatten() {
                if waiter.waiting.fetch_sub(1, Ordering::AcqRel) == 1 {
                    self.enqueue(waiter);
                }
            }
            match current.parent.upgrade() {
                Some(parent) => {
                    {
                        let mut children = lock(&parent.children);
                        children.live.remove(&current.id);
                        children.done_weight += current.weight as u64;
                    }
                    self.emit(false);
                    current = parent;
                }
                None => {
                    let transitioned = self.check_finish(&mut lock(&self.queue));
                    if transitioned {
                        self.finish_run();
                    }
                    return;
                }
            }
        }
    }
}

fn panic_message(payload: Box<dyn Any + Send>) -> String {
    if let Some(text) = payload.downcast_ref::<&str>() {
        (*text).to_owned()
    } else if let Some(text) = payload.downcast_ref::<String>() {
        text.clone()
    } else {
        "unknown panic".to_owned()
    }
}

fn worker(shared: Arc<Shared>) {
    loop {
        let popped = {
            let mut queue = lock(&shared.queue);
            loop {
                if queue.shutdown {
                    return;
                }
                if shared.cancel.load(Ordering::Acquire) {
                    shared.aborted.store(true, Ordering::Release);
                }
                if shared.aborted.load(Ordering::Acquire) {
                    queue.heap.clear();
                    if shared.check_finish(&mut queue) {
                        break None;
                    }
                } else if let Some(item) = queue.heap.pop() {
                    queue.running += 1;
                    break Some(item.node);
                }
                let (guard, _) =
                    shared.work.wait_timeout(queue, Duration::from_millis(20)).unwrap_or_else(|e| e.into_inner());
                queue = guard;
            }
        };
        match popped {
            None => {
                shared.finish_run();
                return;
            }
            Some(node) => {
                if !shared.cancel.load(Ordering::Acquire) {
                    run_node(&shared, &node);
                }
                let transitioned = {
                    let mut queue = lock(&shared.queue);
                    queue.running -= 1;
                    shared.check_finish(&mut queue)
                };
                if transitioned {
                    shared.finish_run();
                }
            }
        }
    }
}

/// Starts building a run.
pub struct TaskRunner {
    name: String,
    threads: usize,
    cancel: CancelToken,
    events: Option<Sender<ProgressEvent>>,
    emit_interval: Duration,
}

impl TaskRunner {
    /// A runner whose root task is called `name`.
    pub fn new(name: impl Into<String>) -> TaskRunner {
        TaskRunner {
            name: name.into(),
            threads: default_thread_count(),
            cancel: new_cancel_token(),
            events: None,
            emit_interval: Duration::from_millis(50),
        }
    }

    /// The number of worker threads (at least 1).
    pub fn threads(mut self, threads: usize) -> TaskRunner {
        self.threads = threads.max(1);
        self
    }

    /// Uses an existing cancel token (to cancel from elsewhere).
    pub fn cancel_token(mut self, token: CancelToken) -> TaskRunner {
        self.cancel = token;
        self
    }

    /// Sends [`ProgressEvent`]s to this channel.
    pub fn events(mut self, sender: Sender<ProgressEvent>) -> TaskRunner {
        self.events = Some(sender);
        self
    }

    /// The minimum time between progress events (default 50 ms).
    pub fn emit_interval(mut self, interval: Duration) -> TaskRunner {
        self.emit_interval = interval;
        self
    }

    /// Starts the run in the background; `root` is the body of the root task.
    pub fn start<F>(self, root: F) -> Job
    where
        F: FnOnce(&TaskContext) -> Result<(), ConvertError> + Send + 'static,
    {
        let shared = Arc::new(Shared {
            queue: Mutex::new(Queue::default()),
            work: Condvar::new(),
            seq: AtomicU64::new(0),
            node_ids: AtomicU64::new(0),
            cancel: self.cancel,
            failure: Mutex::new(None),
            aborted: AtomicBool::new(false),
            root: std::sync::OnceLock::new(),
            finished: Mutex::new(false),
            finished_cv: Condvar::new(),
            events: self.events.map(Mutex::new),
            emit_state: Mutex::new(Instant::now() - self.emit_interval),
            emit_interval: self.emit_interval,
            stage: Mutex::new(self.name.clone()),
            counters: Mutex::new(BTreeMap::new()),
            warnings: Mutex::new(Vec::new()),
            tasks_run: AtomicU64::new(0),
            started: Instant::now(),
        });
        let root_node = shared.new_node(self.name, Weight::NORMAL.0, None, Some(Box::new(root)));
        let _ = shared.root.set(root_node.clone());
        shared.enqueue(root_node);
        let handles = (0..self.threads)
            .map(|index| {
                let shared = shared.clone();
                std::thread::Builder::new()
                    .name(format!("bf-convert-{index}"))
                    .stack_size(8 * 1024 * 1024)
                    .spawn(move || worker(shared))
                    .expect("could not start a worker thread")
            })
            .collect();
        Job { shared, threads: handles }
    }

    /// Runs to the end on the calling thread's behalf and returns the result.
    pub fn run<F>(self, root: F) -> Result<TaskReport, ConvertError>
    where
        F: FnOnce(&TaskContext) -> Result<(), ConvertError> + Send + 'static,
    {
        self.start(root).wait()
    }
}

/// What a successful run reports.
#[derive(Clone, Debug)]
pub struct TaskReport {
    pub elapsed: Duration,
    /// How many task bodies ran.
    pub tasks_run: u64,
    /// The warnings raised with [`TaskContext::warn`], in order.
    pub warnings: Vec<String>,
    /// The final counters.
    pub counters: Vec<(String, u64)>,
}

/// A run in progress.
pub struct Job {
    shared: Arc<Shared>,
    threads: Vec<JoinHandle<()>>,
}

impl Job {
    /// A live view of the progress.
    pub fn progress(&self) -> Progress {
        Progress { shared: self.shared.clone() }
    }

    /// The token that cancels this run.
    pub fn cancel_token(&self) -> CancelToken {
        self.shared.cancel.clone()
    }

    /// Cancels the run: queued tasks are dropped and running ones see [`TaskContext::is_cancelled`].
    pub fn cancel(&self) {
        self.shared.cancel.store(true, Ordering::Release);
        self.shared.work.notify_all();
    }

    /// Whether the run has ended.
    pub fn is_finished(&self) -> bool {
        *lock(&self.shared.finished)
    }

    /// Waits for the end: `Ok` when every task completed, else the first error ([`ConvertError::Cancelled`] after a
    /// cancel).
    pub fn wait(mut self) -> Result<TaskReport, ConvertError> {
        {
            let mut finished = lock(&self.shared.finished);
            while !*finished {
                finished = self.shared.finished_cv.wait(finished).unwrap_or_else(|e| e.into_inner());
            }
        }
        for handle in self.threads.drain(..) {
            let _ = handle.join();
        }
        let shared = &self.shared;
        if let Some(error) = lock(&shared.failure).take() {
            return Err(error);
        }
        if shared.aborted.load(Ordering::Acquire) {
            return Err(ConvertError::Cancelled);
        }
        Ok(TaskReport {
            elapsed: shared.started.elapsed(),
            tasks_run: shared.tasks_run.load(Ordering::Relaxed),
            warnings: lock(&shared.warnings).clone(),
            counters: self.progress().counters(),
        })
    }
}

impl Drop for Job {
    fn drop(&mut self) {
        if !self.threads.is_empty() {
            // Dropped without `wait`: stop the work; the workers end on their own.
            self.cancel();
        }
    }
}

/// The handle a running task uses to spawn children and report.
pub struct TaskContext {
    shared: Arc<Shared>,
    node: Arc<Node>,
}

/// A reference to a spawned task, for [`TaskContext::spawn_after`] and placeholders.
#[derive(Clone)]
pub struct TaskHandle {
    shared: Arc<Shared>,
    node: Arc<Node>,
}

impl TaskHandle {
    /// Whether the task and all its descendants completed.
    pub fn is_done(&self) -> bool {
        self.node.state.load(Ordering::Acquire) == DONE
    }

    pub fn name(&self) -> &str {
        &self.node.name
    }

    /// The progress of this task alone.
    pub fn progress(&self) -> f64 {
        self.node.progress()
    }

    /// Completes a task made by [`TaskContext::placeholder`]. Call it exactly once, and only on placeholders.
    pub fn complete(&self) {
        self.shared.release(&self.node);
    }
}

impl TaskContext {
    /// The name of this task.
    pub fn name(&self) -> &str {
        &self.node.name
    }

    /// Distance from the root (the root is 0).
    pub fn depth(&self) -> u32 {
        self.node.depth
    }

    /// A live view of the overall progress.
    pub fn progress(&self) -> Progress {
        Progress { shared: self.shared.clone() }
    }

    /// The cancel token, for long loops that run in one task.
    pub fn cancel_token(&self) -> CancelToken {
        self.shared.cancel.clone()
    }

    /// Whether the run was cancelled or failed; check it in long loops.
    pub fn is_cancelled(&self) -> bool {
        self.shared.cancel.load(Ordering::Acquire)
    }

    /// `Err(Cancelled)` once the run was cancelled; use with `?` in long loops.
    pub fn check_cancelled(&self) -> Result<(), ConvertError> {
        if self.is_cancelled() { Err(ConvertError::Cancelled) } else { Ok(()) }
    }

    /// Sets the stage text shown to the user.
    pub fn set_stage(&self, text: impl Into<String>) {
        let text = text.into();
        *lock(&self.shared.stage) = text;
        self.shared.emit(false);
    }

    /// A named counter (created at 0 when new).
    pub fn counter(&self, name: &str) -> Counter {
        let mut counters = lock(&self.shared.counters);
        Counter(counters.entry(name.to_owned()).or_default().clone())
    }

    /// Sends a named state change to the event channel (Chunker's `Task.signal`).
    pub fn signal(&self, name: &str, active: bool) {
        self.shared.send(ProgressEvent::Signal { name: name.to_owned(), active });
    }

    /// Records a non-fatal problem; it reaches the report and the event channel.
    pub fn warn(&self, message: impl Into<String>) {
        let message = message.into();
        lock(&self.shared.warnings).push(message.clone());
        self.shared.send(ProgressEvent::Warning(message));
    }

    fn add_child(&self, name: String, weight: Weight, body: Option<Body>) -> Arc<Node> {
        let child = self.shared.new_node(name, weight.0, Some(&self.node), body);
        // Count the child on the parent before it can possibly finish.
        self.node.pending.fetch_add(1, Ordering::AcqRel);
        lock(&self.node.children).live.insert(child.id, child.clone());
        child
    }

    /// Spawns a child task. It runs on any worker; this task completes only after it did.
    pub fn spawn<F>(&self, name: impl Into<String>, weight: Weight, body: F) -> TaskHandle
    where
        F: FnOnce(&TaskContext) -> Result<(), ConvertError> + Send + 'static,
    {
        let child = self.add_child(name.into(), weight, Some(Box::new(body)));
        self.shared.enqueue(child.clone());
        TaskHandle { shared: self.shared.clone(), node: child }
    }

    /// Spawns a child that is queued only once every task in `after` (with all its descendants) completed. It counts
    /// as a child of this task from the start, so this task cannot complete (and progress cannot reach 1) before it.
    pub fn spawn_after<F>(&self, after: &[TaskHandle], name: impl Into<String>, weight: Weight, body: F) -> TaskHandle
    where
        F: FnOnce(&TaskContext) -> Result<(), ConvertError> + Send + 'static,
    {
        let child = self.add_child(name.into(), weight, Some(Box::new(body)));
        child.waiting.store(after.len(), Ordering::Release);
        if after.is_empty() {
            self.shared.enqueue(child.clone());
        }
        for dependency in after {
            let mut waiters = lock(&dependency.node.waiters);
            match waiters.as_mut() {
                Some(list) => list.push(child.clone()),
                None => {
                    drop(waiters);
                    // Already complete.
                    if child.waiting.fetch_sub(1, Ordering::AcqRel) == 1 {
                        self.shared.enqueue(child.clone());
                    }
                }
            }
        }
        TaskHandle { shared: self.shared.clone(), node: child }
    }

    /// Reserves progress weight for work that is not spawned yet (Chunker's reader reserves the weight of the
    /// batches it has not started). The placeholder keeps this task open until [`TaskHandle::complete`] is called.
    pub fn placeholder(&self, name: impl Into<String>, weight: Weight) -> TaskHandle {
        let child = self.add_child(name.into(), weight, None);
        TaskHandle { shared: self.shared.clone(), node: child }
    }
}

fn run_node(shared: &Arc<Shared>, node: &Arc<Node>) {
    let body = lock(&node.body).take();
    let Some(body) = body else {
        return;
    };
    node.state.store(STARTED, Ordering::Release);
    shared.tasks_run.fetch_add(1, Ordering::Relaxed);
    let context = TaskContext { shared: shared.clone(), node: node.clone() };
    match catch_unwind(AssertUnwindSafe(|| body(&context))) {
        Ok(Ok(())) => shared.release(node),
        Ok(Err(error)) => shared.fail(error),
        Err(payload) => {
            shared.fail(ConvertError::Internal(format!("task \"{}\" panicked: {}", node.name, panic_message(payload))))
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicUsize;
    use std::sync::mpsc;

    use super::*;

    type Log = Arc<Mutex<Vec<String>>>;

    fn log() -> Log {
        Arc::new(Mutex::new(Vec::new()))
    }

    fn push(log: &Log, text: &str) {
        log.lock().unwrap().push(text.to_owned());
    }

    fn one() -> TaskRunner {
        TaskRunner::new("root").threads(1)
    }

    #[test]
    fn deeper_first_then_newest_first() {
        let order = log();
        let o = order.clone();
        one()
            .run(move |ctx| {
                push(&o, "root");
                let (a, b, c) = (o.clone(), o.clone(), o.clone());
                ctx.spawn("A", Weight::NORMAL, move |ctx| {
                    push(&a, "A");
                    let (a1, a2) = (a.clone(), a.clone());
                    ctx.spawn("A1", Weight::NORMAL, move |_| {
                        push(&a1, "A1");
                        Ok(())
                    });
                    ctx.spawn("A2", Weight::NORMAL, move |_| {
                        push(&a2, "A2");
                        Ok(())
                    });
                    Ok(())
                });
                ctx.spawn("B", Weight::NORMAL, move |_| {
                    push(&b, "B");
                    Ok(())
                });
                ctx.spawn("C", Weight::NORMAL, move |_| {
                    push(&c, "C");
                    Ok(())
                });
                Ok(())
            })
            .unwrap();
        // Newest depth-1 task first (C, B, A); A's children are deeper, so they go before anything older.
        assert_eq!(*order.lock().unwrap(), ["root", "C", "B", "A", "A2", "A1"]);
    }

    #[test]
    fn depth_beats_age() {
        let order = log();
        let o = order.clone();
        one()
            .run(move |ctx| {
                let (x, y) = (o.clone(), o.clone());
                // X is spawned first (runs last), and spawns a deeper task that must run before Y.
                ctx.spawn("X", Weight::NORMAL, move |ctx| {
                    push(&x, "X");
                    let x2 = x.clone();
                    ctx.spawn("X1", Weight::NORMAL, move |_| {
                        push(&x2, "X1");
                        Ok(())
                    });
                    Ok(())
                });
                ctx.spawn("Y", Weight::NORMAL, move |_| {
                    push(&y, "Y");
                    Ok(())
                });
                Ok(())
            })
            .unwrap();
        assert_eq!(*order.lock().unwrap(), ["Y", "X", "X1"]);
    }

    #[test]
    fn parent_completes_after_children() {
        let order = log();
        let o = order.clone();
        TaskRunner::new("root")
            .threads(3)
            .run(move |ctx| {
                let o2 = o.clone();
                let parent = ctx.spawn("parent", Weight::NORMAL, move |ctx| {
                    for i in 0..5 {
                        let o3 = o2.clone();
                        ctx.spawn(format!("child {i}"), Weight::LOW, move |ctx| {
                            std::thread::sleep(Duration::from_millis(5));
                            ctx.spawn("grandchild", Weight::LOW, {
                                let o4 = o3.clone();
                                move |_| {
                                    std::thread::sleep(Duration::from_millis(5));
                                    push(&o4, "grandchild");
                                    Ok(())
                                }
                            });
                            push(&o3, "child");
                            Ok(())
                        });
                    }
                    push(&o2, "parent body");
                    Ok(())
                });
                let o5 = o.clone();
                let after = ctx.spawn_after(&[parent], "after", Weight::LOW, move |_| {
                    push(&o5, "after");
                    Ok(())
                });
                assert!(!after.is_done());
                Ok(())
            })
            .unwrap();
        let order = order.lock().unwrap();
        assert_eq!(order.len(), 12);
        assert_eq!(order.last().map(String::as_str), Some("after"));
        assert_eq!(order.iter().filter(|e| *e == "grandchild").count(), 5);
        assert!(order.iter().any(|e| e == "parent body"));
    }

    #[test]
    fn error_aborts_siblings() {
        let ran = Arc::new(AtomicUsize::new(0));
        let r = ran.clone();
        let result = one().run(move |ctx| {
            for _ in 0..50 {
                let r = r.clone();
                ctx.spawn("work", Weight::NORMAL, move |_| {
                    r.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                });
            }
            // Spawned last = newest = runs first on the single worker.
            ctx.spawn("boom", Weight::NORMAL, |_| Err(ConvertError::Malformed("bad region".into())));
            Ok(())
        });
        match result {
            Err(ConvertError::Malformed(text)) => assert_eq!(text, "bad region"),
            other => panic!("unexpected {other:?}"),
        }
        assert_eq!(ran.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn first_error_wins() {
        let result = TaskRunner::new("root").threads(4).run(|ctx| {
            ctx.spawn("a", Weight::NORMAL, |_| Err(ConvertError::Unsupported("first".into())));
            ctx.spawn("b", Weight::NORMAL, |ctx| {
                // Wait until the first failure cancelled the run, then fail differently.
                while !ctx.is_cancelled() {
                    std::thread::sleep(Duration::from_millis(1));
                }
                Err(ConvertError::Cancelled)
            });
            Ok(())
        });
        assert!(matches!(result, Err(ConvertError::Unsupported(_)) | Err(ConvertError::Cancelled)));
        // On one thread the order is fixed: b (newest) runs first and sees no failure yet, so give it a quick error.
        let result = one().run(|ctx| {
            ctx.spawn("late", Weight::NORMAL, |_| Err(ConvertError::Unsupported("late".into())));
            ctx.spawn("early", Weight::NORMAL, |_| Err(ConvertError::Malformed("early".into())));
            Ok(())
        });
        assert!(matches!(result, Err(ConvertError::Malformed(_))));
    }

    #[test]
    fn panic_is_reported_not_propagated() {
        let result = TaskRunner::new("root").threads(2).run(|ctx| {
            ctx.spawn("bad", Weight::NORMAL, |_| panic!("kaboom {}", 7));
            Ok(())
        });
        match result {
            Err(ConvertError::Internal(text)) => {
                assert!(text.contains("kaboom 7") && text.contains("bad"), "{text}");
            }
            other => panic!("unexpected {other:?}"),
        }
        let result = one().run(|_| std::panic::panic_any(5u8));
        assert!(matches!(result, Err(ConvertError::Internal(t)) if t.contains("unknown panic")));
    }

    #[test]
    fn cancel_from_outside_stops_promptly() {
        let started = Arc::new(AtomicUsize::new(0));
        let s = started.clone();
        let job = TaskRunner::new("root").threads(2).start(move |ctx| {
            for _ in 0..10_000 {
                let s = s.clone();
                ctx.spawn("slow", Weight::LOW, move |ctx| {
                    s.fetch_add(1, Ordering::SeqCst);
                    // A long loop that honours the token.
                    for _ in 0..1000 {
                        ctx.check_cancelled()?;
                        std::thread::sleep(Duration::from_millis(1));
                    }
                    Ok(())
                });
            }
            Ok(())
        });
        while started.load(Ordering::SeqCst) < 2 {
            std::thread::sleep(Duration::from_millis(1));
        }
        let before = Instant::now();
        job.cancel();
        let result = job.wait();
        assert!(matches!(result, Err(ConvertError::Cancelled)));
        assert!(before.elapsed() < Duration::from_secs(2), "took {:?}", before.elapsed());
        assert!(started.load(Ordering::SeqCst) < 100);
    }

    #[test]
    fn cancel_token_set_before_start() {
        let token = new_cancel_token();
        token.store(true, Ordering::SeqCst);
        let ran = Arc::new(AtomicBool::new(false));
        let r = ran.clone();
        let result = TaskRunner::new("root").cancel_token(token).run(move |_| {
            r.store(true, Ordering::SeqCst);
            Ok(())
        });
        assert!(matches!(result, Err(ConvertError::Cancelled)));
        assert!(!ran.load(Ordering::SeqCst));
    }

    #[test]
    fn cancel_from_inside_a_task() {
        let result = one().run(|ctx| {
            for _ in 0..20 {
                ctx.spawn("t", Weight::NORMAL, |ctx| {
                    ctx.cancel_token().store(true, Ordering::SeqCst);
                    Ok(())
                });
            }
            Ok(())
        });
        assert!(matches!(result, Err(ConvertError::Cancelled)));
    }

    #[test]
    fn progress_reaches_one_and_weights_count() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let s = seen.clone();
        let job = one().start(move |ctx| {
            let s = s.clone();
            // B is spawned first, so A (newest) runs first and is done when B looks at the progress.
            ctx.spawn("B", Weight::NORMAL, move |ctx| {
                s.lock().unwrap().push(ctx.progress().fraction());
                Ok(())
            });
            ctx.spawn("A", Weight::NORMAL, |_| Ok(()));
            Ok(())
        });
        let progress = job.progress();
        let report = job.wait().unwrap();
        assert_eq!(report.tasks_run, 3);
        assert_eq!(progress.fraction(), 1.0);
        // root: A done (100), B live and started with no children (0): 100 / (1 + 100 + 100).
        let at_b = seen.lock().unwrap()[0];
        assert!((at_b - 100.0 / 201.0).abs() < 1e-12, "{at_b}");
    }

    #[test]
    fn placeholder_reserves_weight_and_holds_the_parent() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let s = seen.clone();
        let report = one()
            .run(move |ctx| {
                let reserved = ctx.placeholder("later batches", Weight(300));
                let r2 = reserved.clone();
                let s = s.clone();
                ctx.spawn("batch", Weight(100), move |ctx| {
                    s.lock().unwrap().push(ctx.progress().fraction());
                    r2.complete();
                    Ok(())
                });
                assert!(!reserved.is_done());
                Ok(())
            })
            .unwrap();
        assert_eq!(report.tasks_run, 2);
        // The placeholder (0 of 300) and the started batch (0 of 100) are open: 0 / 401.
        assert_eq!(seen.lock().unwrap()[0], 0.0);
    }

    #[test]
    fn spawn_after_waits_for_all_dependencies_including_done_ones() {
        let order = log();
        let o = order.clone();
        TaskRunner::new("root")
            .threads(2)
            .run(move |ctx| {
                let (o1, o2) = (o.clone(), o.clone());
                let a = ctx.spawn("a", Weight::LOW, move |_| {
                    std::thread::sleep(Duration::from_millis(30));
                    push(&o1, "a");
                    Ok(())
                });
                let b = ctx.spawn("b", Weight::LOW, move |_| {
                    push(&o2, "b");
                    Ok(())
                });
                let o3 = o.clone();
                ctx.spawn_after(&[a.clone(), b.clone()], "c", Weight::LOW, move |_| {
                    push(&o3, "c");
                    Ok(())
                });
                // No dependencies: queued right away.
                let o4 = o.clone();
                ctx.spawn_after(&[], "d", Weight::LOW, move |_| {
                    push(&o4, "d");
                    Ok(())
                });
                Ok(())
            })
            .unwrap();
        let order = order.lock().unwrap();
        let pos = |name: &str| order.iter().position(|e| e == name).unwrap();
        assert!(pos("c") > pos("a") && pos("c") > pos("b"));
        assert_eq!(order.len(), 4);
    }

    #[test]
    fn spawn_after_on_a_finished_task() {
        let order = log();
        let o = order.clone();
        one()
            .run(move |ctx| {
                let o1 = o.clone();
                let first = ctx.spawn("first", Weight::LOW, move |ctx| {
                    push(&o1, "first");
                    // From inside a finished sibling's perspective: the handle is done by the time "second" runs.
                    let _ = ctx;
                    Ok(())
                });
                let o2 = o.clone();
                let chain = ctx.spawn_after(&[first], "second", Weight::LOW, move |ctx| {
                    push(&o2, "second");
                    let o3 = o2.clone();
                    ctx.spawn("third", Weight::LOW, move |_| {
                        push(&o3, "third");
                        Ok(())
                    });
                    Ok(())
                });
                let o4 = o.clone();
                ctx.spawn_after(&[chain], "fourth", Weight::LOW, move |_| {
                    push(&o4, "fourth");
                    Ok(())
                });
                Ok(())
            })
            .unwrap();
        // `fourth` waits for `second` and its descendant `third`.
        assert_eq!(*order.lock().unwrap(), ["first", "second", "third", "fourth"]);
    }

    #[test]
    fn single_worker_does_not_deadlock() {
        // A deep chain plus a wide fan-out, all on one thread.
        fn nest(ctx: &TaskContext, depth: u32, ran: Arc<AtomicUsize>) {
            if depth == 0 {
                return;
            }
            for _ in 0..3 {
                let ran = ran.clone();
                ctx.spawn("n", Weight::LOW, move |ctx| {
                    ran.fetch_add(1, Ordering::SeqCst);
                    nest(ctx, depth - 1, ran);
                    Ok(())
                });
            }
        }
        let ran = Arc::new(AtomicUsize::new(0));
        let r = ran.clone();
        let report = one()
            .run(move |ctx| {
                nest(ctx, 6, r);
                Ok(())
            })
            .unwrap();
        // 3 + 9 + ... + 3^6 bodies plus the root.
        let expected: usize = (1..=6).map(|d| 3usize.pow(d)).sum();
        assert_eq!(ran.load(Ordering::SeqCst), expected);
        assert_eq!(report.tasks_run as usize, expected + 1);
    }

    #[test]
    fn many_tasks_on_many_threads() {
        let sum = Arc::new(AtomicU64::new(0));
        let s = sum.clone();
        let report = TaskRunner::new("root")
            .threads(8)
            .run(move |ctx| {
                let counter = ctx.counter("columns");
                for i in 0..2000u64 {
                    let (s, counter) = (s.clone(), counter.clone());
                    ctx.spawn("column", Weight::LOW, move |_| {
                        s.fetch_add(i, Ordering::SeqCst);
                        counter.add(1);
                        Ok(())
                    });
                }
                Ok(())
            })
            .unwrap();
        assert_eq!(sum.load(Ordering::SeqCst), (0..2000u64).sum::<u64>());
        assert_eq!(report.counters, [("columns".to_owned(), 2000)]);
        assert_eq!(report.tasks_run, 2001);
    }

    #[test]
    fn events_stream_ends_with_finished() {
        let (tx, rx) = mpsc::channel();
        let report = TaskRunner::new("Converting")
            .threads(2)
            .events(tx)
            .emit_interval(Duration::ZERO)
            .run(|ctx| {
                ctx.set_stage("Reading Level");
                ctx.signal("Compacting output", true);
                ctx.warn("something odd");
                ctx.spawn("work", Weight::NORMAL, |ctx| {
                    ctx.signal("Compacting output", false);
                    Ok(())
                });
                Ok(())
            })
            .unwrap();
        assert_eq!(report.warnings, ["something odd"]);
        let events: Vec<ProgressEvent> = rx.try_iter().collect();
        assert_eq!(events.last(), Some(&ProgressEvent::Finished(FinishStatus::Done)));
        assert!(events.contains(&ProgressEvent::Signal { name: "Compacting output".into(), active: true }));
        assert!(events.contains(&ProgressEvent::Signal { name: "Compacting output".into(), active: false }));
        assert!(events.contains(&ProgressEvent::Warning("something odd".into())));
        let fractions: Vec<f64> = events
            .iter()
            .filter_map(|e| if let ProgressEvent::Progress(p) = e { Some(p.fraction) } else { None })
            .collect();
        assert_eq!(fractions.last(), Some(&1.0));
        assert!(fractions.windows(2).all(|w| w[0] <= w[1] + 1e-12), "{fractions:?}");
        // The signal order is kept on the channel.
        let signals: Vec<bool> = events
            .iter()
            .filter_map(|e| if let ProgressEvent::Signal { active, .. } = e { Some(*active) } else { None })
            .collect();
        assert_eq!(signals, [true, false]);
    }

    #[test]
    fn events_report_failure_and_cancel() {
        let (tx, rx) = mpsc::channel();
        let _ = TaskRunner::new("root").threads(1).events(tx).run(|_| Err(ConvertError::Malformed("nope".into())));
        let last = rx.try_iter().last();
        assert!(matches!(last, Some(ProgressEvent::Finished(FinishStatus::Failed(m))) if m.contains("nope")));

        let (tx, rx) = mpsc::channel();
        let token = new_cancel_token();
        token.store(true, Ordering::SeqCst);
        let _ = TaskRunner::new("root").cancel_token(token).events(tx).run(|_| Ok(()));
        assert_eq!(rx.try_iter().last(), Some(ProgressEvent::Finished(FinishStatus::Cancelled)));
    }

    #[test]
    fn events_are_throttled() {
        let (tx, rx) = mpsc::channel();
        TaskRunner::new("root")
            .threads(2)
            .events(tx)
            .emit_interval(Duration::from_secs(3600))
            .run(|ctx| {
                for _ in 0..200 {
                    ctx.spawn("t", Weight::LOW, |_| Ok(()));
                }
                Ok(())
            })
            .unwrap();
        let progress_events = rx.try_iter().filter(|e| matches!(e, ProgressEvent::Progress(_))).count();
        // The first one and the final forced one at most.
        assert!(progress_events <= 2, "{progress_events}");
    }

    #[test]
    fn progress_view_reports_stage_and_counters() {
        let job = one().start(|ctx| {
            ctx.set_stage("Writing World");
            ctx.counter("columns").add(5);
            ctx.counter("columns").add(2);
            Ok(())
        });
        let progress = job.progress();
        job.wait().unwrap();
        assert_eq!(progress.stage(), "Writing World");
        assert_eq!(progress.counter("columns"), 7);
        assert_eq!(progress.counter("nothing"), 0);
        assert_eq!(progress.snapshot().counters, [("columns".to_owned(), 7)]);
    }

    #[test]
    fn dropping_a_job_cancels_it() {
        let job = TaskRunner::new("root").threads(1).start(|ctx| {
            for _ in 0..1000 {
                ctx.spawn("slow", Weight::LOW, |ctx| {
                    while !ctx.is_cancelled() {
                        std::thread::sleep(Duration::from_millis(1));
                    }
                    Ok(())
                });
            }
            Ok(())
        });
        let token = job.cancel_token();
        drop(job);
        assert!(token.load(Ordering::SeqCst));
    }

    #[test]
    fn thread_count_is_bounded() {
        let n = default_thread_count();
        assert!((1..=8).contains(&n));
        let seen = Arc::new(Mutex::new(std::collections::HashSet::new()));
        let s = seen.clone();
        TaskRunner::new("root")
            .threads(3)
            .run(move |ctx| {
                for _ in 0..60 {
                    let s = s.clone();
                    ctx.spawn("t", Weight::LOW, move |_| {
                        s.lock().unwrap().insert(std::thread::current().id());
                        std::thread::sleep(Duration::from_millis(1));
                        Ok(())
                    });
                }
                Ok(())
            })
            .unwrap();
        assert!(seen.lock().unwrap().len() <= 3);
    }
}
