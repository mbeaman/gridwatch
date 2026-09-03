//! The signals a process picker offers (arc 8a).
//!
//! htop's `F9` menu lists all 31; these are the nine anyone reaches for,
//! in htop's order. The numbers are Linux's — `ui` and `components` may not
//! call libc, so they are written down here and the shell's handler passes
//! them to `kill(2)` unchanged.
//!
//! Both process tables use this list, and neither owns it: the gpu tile is
//! buildable without the htop feature.

pub const SIGNALS: &[(&str, i32)] = &[
    ("SIGTERM", 15),
    ("SIGKILL", 9),
    ("SIGHUP", 1),
    ("SIGINT", 2),
    ("SIGQUIT", 3),
    ("SIGSTOP", 19),
    ("SIGCONT", 18),
    ("SIGUSR1", 10),
    ("SIGUSR2", 12),
];
