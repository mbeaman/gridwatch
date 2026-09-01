//! The two OS calls that need libc (documented deviation from the no-unsafe
//! rule, which binds the store's data structures — D26): stderr redirection
//! before the alternate screen (§11) and the local-time offset for the clock.

#![allow(unsafe_code)]

use std::fs::OpenOptions;
use std::os::fd::AsRawFd;
use std::path::PathBuf;

/// `dup2` stderr into `$XDG_STATE_HOME/gridwatch/gridwatch.log` (§11): library
/// `eprintln!`s must never scribble on the alternate screen.
pub fn redirect_stderr() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/state")))?;
    let dir = base.join("gridwatch");
    std::fs::create_dir_all(&dir).ok()?;
    let path = dir.join("gridwatch.log");
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .ok()?;
    // SAFETY: dup2 with two valid fds; the file outlives the call and fd 2 is
    // owned by the process for its lifetime.
    let rc = unsafe { libc::dup2(file.as_raw_fd(), 2) };
    std::mem::forget(file); // fd 2 now aliases it; keep it open for the process lifetime
    (rc != -1).then_some(path)
}

/// Local-time offset in seconds east of UTC, computed once at startup so the
/// clock component renders deterministically from `wall + offset` (§8).
pub fn tz_offset_s() -> i32 {
    // SAFETY: time/localtime_r with valid pointers; tm is zero-initialised.
    unsafe {
        let now = libc::time(std::ptr::null_mut());
        let mut tm: libc::tm = std::mem::zeroed();
        if libc::localtime_r(&now, &mut tm).is_null() {
            0
        } else {
            tm.tm_gmtoff as i32
        }
    }
}
