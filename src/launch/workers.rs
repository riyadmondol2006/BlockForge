//! Part of the smoothness helper, done from outside the game on Linux: Minecraft's world-generation worker threads
//! ("Worker-Main-N") run at a lower priority on computers with 4 processor threads or fewer.
//!
//! There vanilla doesn't raise the priority of its render and server threads, so the workers compete with the server
//! thread on equal terms. Measured (26.3, 4 threads, flying through new terrain): the server tick went from 19-21.5 ms
//! to 13.5-14 ms (median), and the server thread waited for the processor 16-17 % of the time instead of 26-31 %; the
//! render thread was unchanged. New terrain is made about a fifth slower while flying and catches up within 40 s; the
//! world itself comes out the same. Windows is handled by the helper inside the game (Java thread priorities); macOS
//! gets nothing.
//!
//! While the game runs, its threads are looked at every few seconds; new worker threads get a nice value 4 above the
//! game's own (lower priority). Nothing else is touched, and a failure is simply ignored.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// How much lower the workers' priority is (added to the game's nice value).
pub const NICE_STEP: i32 = 4;
/// The highest nice value there is.
pub const MAX_NICE: i32 = 19;
/// How often the game's threads are looked at.
pub const EVERY: Duration = Duration::from_secs(5);
/// Only computers with this many processor threads or fewer.
pub const MAX_CORES: usize = 4;

/// If this is a thread to lower: the game's world-generation workers. The kernel keeps 15 characters of a thread's
/// name, so "Worker-Bootstrap-1" is "Worker-Bootstra". The file workers ("IO-Worker-N") and everything else stay.
pub fn is_worker(name: &str) -> bool {
    let name = name.trim();
    name.starts_with("Worker-Main") || name.starts_with("Worker-Bootstra")
}

/// Whether to do it for a start: on Linux, with the smoothness helper on, on a computer with few processor threads.
pub fn wanted(lite_on: bool, cores: usize) -> bool {
    cfg!(target_os = "linux") && lite_on && cores > 0 && cores <= MAX_CORES
}

/// One thread of the game: its id, name and nice value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Thread {
    pub tid: u32,
    pub name: String,
    pub nice: i32,
}

/// The threads to change and the nice value each gets: workers whose priority isn't lowered yet.
pub fn plan(threads: &[Thread], process_nice: i32) -> Vec<(u32, i32)> {
    let target = (process_nice + NICE_STEP).min(MAX_NICE);
    threads.iter().filter(|t| is_worker(&t.name) && t.nice < target).map(|t| (t.tid, target)).collect()
}

/// The nice value in a `/proc/.../stat` line (the 19th field; the name in brackets may hold spaces).
pub fn nice_from_stat(stat: &str) -> Option<i32> {
    let after_name = &stat[stat.rfind(')')? + 1..];
    // fields after the name start with the 3rd (state); nice is the 19th
    after_name.split_whitespace().nth(16)?.parse().ok()
}

/// The game's threads, and the ids of the processes started under it (a wrapper such as gamemoderun starts Java as
/// its child).
fn threads_of(proc: &Path, pid: u32) -> Vec<Thread> {
    let mut out = Vec::new();
    let mut pids = vec![pid];
    let mut i = 0;
    while i < pids.len() && i < 64 {
        let p = pids[i];
        i += 1;
        let Ok(rd) = std::fs::read_dir(proc.join(p.to_string()).join("task")) else { continue };
        for e in rd.flatten() {
            let Ok(tid) = e.file_name().to_string_lossy().parse::<u32>() else { continue };
            let dir = e.path();
            if let Ok(children) = std::fs::read_to_string(dir.join("children")) {
                pids.extend(children.split_whitespace().filter_map(|c| c.parse::<u32>().ok()));
            }
            let name = std::fs::read_to_string(dir.join("comm")).unwrap_or_default();
            let Some(nice) = std::fs::read_to_string(dir.join("stat")).ok().and_then(|s| nice_from_stat(&s)) else {
                continue;
            };
            out.push(Thread { tid, name: name.trim().to_string(), nice });
        }
    }
    out
}

#[cfg(target_os = "linux")]
fn set_nice(tid: u32, nice: i32) -> bool {
    unsafe extern "C" {
        fn setpriority(which: i32, who: u32, prio: i32) -> i32;
    }
    const PRIO_PROCESS: i32 = 0;
    // SAFETY: a plain system call on numbers; on Linux PRIO_PROCESS with a thread id changes that one thread
    unsafe { setpriority(PRIO_PROCESS, tid, nice) == 0 }
}

#[cfg(not(target_os = "linux"))]
fn set_nice(_tid: u32, _nice: i32) -> bool {
    false
}

/// Lowers the workers of the game with this process id once. Returns how many threads were changed.
pub fn lower_once(pid: u32) -> usize {
    let proc = Path::new("/proc");
    let Some(process_nice) =
        std::fs::read_to_string(proc.join(pid.to_string()).join("stat")).ok().and_then(|s| nice_from_stat(&s))
    else {
        return 0;
    };
    plan(&threads_of(proc, pid), process_nice).into_iter().filter(|(tid, nice)| set_nice(*tid, *nice)).count()
}

/// Looks after the game's workers until it is dropped or the game is gone.
pub struct Watcher {
    stop: Arc<AtomicBool>,
}

impl Watcher {
    pub fn start(pid: u32) -> Option<Watcher> {
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        std::thread::Builder::new()
            .name("worker-priority".into())
            .spawn(move || {
                let alive = || Path::new("/proc").join(pid.to_string()).exists();
                while !flag.load(Ordering::Relaxed) && alive() {
                    lower_once(pid);
                    // wait in small steps, so stopping is quick
                    for _ in 0..(EVERY.as_millis() / 250) {
                        if flag.load(Ordering::Relaxed) {
                            return;
                        }
                        std::thread::sleep(Duration::from_millis(250));
                    }
                }
            })
            .ok()?;
        Some(Watcher { stop })
    }
}

impl Drop for Watcher {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(tid: u32, name: &str, nice: i32) -> Thread {
        Thread { tid, name: name.into(), nice }
    }

    #[test]
    fn only_the_world_generation_workers_are_picked() {
        assert!(is_worker("Worker-Main-3") && is_worker("Worker-Main-12\n") && is_worker("Worker-Bootstra"));
        for other in ["IO-Worker-1", "Render thread", "Server thread", "java", "Netty Client IO", "worker-main-1"] {
            assert!(!is_worker(other), "{other}");
        }
    }

    #[test]
    fn workers_get_four_more_once_and_never_above_19() {
        let threads =
            [t(10, "Render thread", 0), t(11, "Worker-Main-1", 0), t(12, "Worker-Main-2", 4), t(13, "IO-Worker-1", 0)];
        assert_eq!(plan(&threads, 0), vec![(11, 4)]);
        // the launcher itself runs niced: the workers go 4 below the game
        assert_eq!(plan(&threads, 10), vec![(11, 14), (12, 14)]);
        assert_eq!(plan(&threads, 17), vec![(11, 19), (12, 19)]);
        assert!(plan(&[t(1, "Worker-Main-1", 19)], 17).is_empty());
    }

    #[test]
    fn nice_values_are_read_from_stat_lines() {
        let line = "4242 (Worker-Main 1) S 1 4242 4242 0 -1 4194368 100 0 0 0 5 3 0 0 20 0 30 0 123 456789 789 \
                    18446744073709551615 1 1 0 0 0 0 0 0 0 0 0 0 17 3 0 0 0 0 0";
        assert_eq!(nice_from_stat(line), Some(0));
        let niced = line.replacen(" 20 0 30 ", " 30 10 30 ", 1);
        assert_eq!(nice_from_stat(&niced), Some(10));
        assert_eq!(nice_from_stat("garbage"), None);
    }

    #[test]
    fn only_small_linux_computers_with_the_helper_on() {
        assert_eq!(wanted(true, 4), cfg!(target_os = "linux"));
        assert!(!wanted(true, 8));
        assert!(!wanted(false, 2));
        assert!(!wanted(true, 0));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn this_process_can_be_read() {
        let threads = threads_of(Path::new("/proc"), std::process::id());
        assert!(!threads.is_empty());
        assert!(threads.iter().all(|t| (-20..=19).contains(&t.nice)));
    }
}
