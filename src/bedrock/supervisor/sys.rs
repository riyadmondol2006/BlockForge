//! The few operating-system calls the supervisor needs and the standard library doesn't have. Each is a plain system
//! call on numbers, wrapped so the rest of the supervisor stays free of `unsafe`. Linux only.

use std::ffi::{c_int, c_long, c_ulong, c_void};
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};

unsafe extern "C" {
    fn syscall(number: c_long, ...) -> c_long;
    fn fcntl(fd: c_int, cmd: c_int, ...) -> c_int;
    fn prctl(option: c_int, arg2: c_ulong, arg3: c_ulong, arg4: c_ulong, arg5: c_ulong) -> c_int;
    fn waitpid(pid: c_int, status: *mut c_int, options: c_int) -> c_int;
    fn signal(signum: c_int, handler: usize) -> usize;
    fn statvfs(path: *const std::ffi::c_char, buf: *mut c_void) -> c_int;
    fn kill(pid: c_int, sig: c_int) -> c_int;
}

pub const SIGHUP: i32 = 1;
pub const SIGTERM: i32 = 15;
pub const SIGKILL: i32 = 9;
pub const SIGCONT: i32 = 18;

// the same numbers on every architecture Linux has had since the pidfd calls were added
const SYS_PIDFD_SEND_SIGNAL: c_long = 424;
const SYS_PIDFD_OPEN: c_long = 434;
const F_GETFD: c_int = 1;
const F_SETFD: c_int = 2;
const FD_CLOEXEC: c_int = 1;
const PR_SET_CHILD_SUBREAPER: c_int = 36;
const WNOHANG: c_int = 1;
const ENOSYS: i32 = 38;

/// Let a descriptor survive an `exec`, so the game's wrapper inherits it (a lock held through it lasts as long as the
/// game does, even if the supervisor is killed). Only meant for the supervisor, which starts nothing else after
/// it has taken its leases.
pub fn keep_across_exec(fd: RawFd) -> io::Result<()> {
    // SAFETY: fcntl on a descriptor number; a bad number is reported as EBADF
    unsafe {
        let flags = fcntl(fd, F_GETFD);
        if flags < 0 {
            return Err(io::Error::last_os_error());
        }
        if fcntl(fd, F_SETFD, flags & !FD_CLOEXEC) < 0 {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}

/// Make this process the parent of every orphaned descendant, so the services Wine leaves behind can be found as our
/// own and reaped, rather than becoming children of whatever init is. Best effort.
pub fn become_subreaper() -> bool {
    // SAFETY: prctl with a documented option and plain integers
    unsafe { prctl(PR_SET_CHILD_SUBREAPER, 1, 0, 0, 0) == 0 }
}

/// Collect a finished child of this process by number, if it is one and has ended. Never blocks.
pub fn reap(pid: u32) -> bool {
    let mut status: c_int = 0;
    // SAFETY: waitpid with a valid out pointer; WNOHANG never blocks
    unsafe { waitpid(pid as c_int, &mut status, WNOHANG) == pid as c_int }
}

/// Whether a process by this number exists (a process of another user counts: it exists).
pub fn pid_exists(pid: u32) -> bool {
    if pid == 0 || pid > i32::MAX as u32 {
        return false;
    }
    // SAFETY: signal 0 only checks that the process can be signalled
    let r = unsafe { kill(pid as c_int, 0) };
    r == 0 || io::Error::last_os_error().raw_os_error() == Some(1)
}

/// Ask for `signum` to be delivered to `handler` (an `extern "C" fn(i32)`), the old BSD way: restarts system calls.
pub fn on_signal(signum: i32, handler: extern "C" fn(i32)) {
    // SAFETY: installs a handler that only stores to an atomic
    unsafe {
        signal(signum, handler as usize);
    }
}

/// Send `sig` to the process `pid` is *now*, but only if `still_ours` says so after the process is pinned: a process
/// descriptor is opened first, so the number can't mean a different process by the time the signal is sent.
/// Falls back to a plain `kill` on a kernel without process descriptors.
pub fn signal_pinned(pid: u32, sig: i32, still_ours: &mut dyn FnMut() -> bool) -> io::Result<bool> {
    if pid == 0 || pid > i32::MAX as u32 {
        return Ok(false);
    }
    // SAFETY: pidfd_open takes a number and flags and returns a new descriptor or -1
    let raw = unsafe { syscall(SYS_PIDFD_OPEN, pid as c_long, 0 as c_long) };
    if raw < 0 {
        let err = io::Error::last_os_error();
        return match err.raw_os_error() {
            Some(ENOSYS) => {
                if !still_ours() {
                    return Ok(false);
                }
                // SAFETY: a plain kill on a number we just checked
                let r = unsafe { kill(pid as c_int, sig) };
                if r == 0 { Ok(true) } else { Err(io::Error::last_os_error()) }
            }
            // gone already
            Some(3) => Ok(false),
            _ => Err(err),
        };
    }
    // SAFETY: the descriptor was just returned by the kernel and nothing else owns it
    let pidfd = unsafe { OwnedFd::from_raw_fd(raw as RawFd) };
    if !still_ours() {
        return Ok(false);
    }
    // SAFETY: pidfd_send_signal on our own descriptor with no siginfo
    let r = unsafe {
        syscall(
            SYS_PIDFD_SEND_SIGNAL,
            pidfd.as_raw_fd() as c_long,
            sig as c_long,
            std::ptr::null::<c_void>(),
            0 as c_long,
        )
    };
    if r == 0 {
        Ok(true)
    } else {
        let err = io::Error::last_os_error();
        // the process ended between the check and the signal
        if err.raw_os_error() == Some(3) { Ok(false) } else { Err(err) }
    }
}

/// Free and total bytes of the file system `path` is on (what an ordinary program may use), or `None`.
#[cfg(target_pointer_width = "64")]
pub fn space(path: &std::path::Path) -> Option<(u64, u64)> {
    use std::os::unix::ffi::OsStrExt;
    let c = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
    // struct statvfs on 64-bit glibc and musl: f_bsize, f_frsize, f_blocks, f_bfree, f_bavail ... as unsigned longs
    let mut buf = [0u64; 24];
    // SAFETY: the buffer is larger than the structure and the path is NUL-terminated
    let r = unsafe { statvfs(c.as_ptr(), buf.as_mut_ptr().cast()) };
    if r != 0 {
        return None;
    }
    let fragment = buf[1].max(1);
    Some((buf[4].saturating_mul(fragment), buf[2].saturating_mul(fragment)))
}

#[cfg(not(target_pointer_width = "64"))]
pub fn space(_path: &std::path::Path) -> Option<(u64, u64)> {
    None
}
