//! The four things a person can do to a process from the htop tile (§8,
//! D58 seam 4): signal it, renice it, pin it to CPUs, change its I/O
//! priority. Each is an `Action` — data only, built by the component,
//! `Send`, `Debug`-printable — and the syscall happens here, on the
//! executor thread, beside the other documented libc seam (`sys.rs`).
//!
//! Rules that hold everywhere in this file:
//!
//! * **Never pid 0 or 1, and never this process.** Pid 0 means "every
//!   process in my group" to `kill(2)`, which is not a thing anyone meant
//!   to ask for from a dashboard.
//! * **`EPERM` and `ESRCH` are sentences, not numbers.** "not yours to
//!   change" and "it exited first" are what a person needs.
//! * **Every action confirms** except a one-step renice, which is the one
//!   change that is trivially reversible with the opposite key.

#![allow(unsafe_code)]

use gridwatch_ui::actions::{IoClass, ProcAction};

/// The signals htop's `F9` menu offers, in its order.
pub const SIGNALS: &[(&str, i32)] = &[
    ("SIGTERM", libc::SIGTERM),
    ("SIGKILL", libc::SIGKILL),
    ("SIGHUP", libc::SIGHUP),
    ("SIGINT", libc::SIGINT),
    ("SIGQUIT", libc::SIGQUIT),
    ("SIGSTOP", libc::SIGSTOP),
    ("SIGCONT", libc::SIGCONT),
    ("SIGUSR1", libc::SIGUSR1),
    ("SIGUSR2", libc::SIGUSR2),
];

pub fn signal_name(sig: i32) -> &'static str {
    SIGNALS
        .iter()
        .find(|(_, s)| *s == sig)
        .map(|(n, _)| *n)
        .unwrap_or("signal")
}

/// The last `errno`, as the sentence a person should read.
fn errno_says(what: &str) -> String {
    let e = std::io::Error::last_os_error();
    match e.raw_os_error() {
        Some(libc::EPERM) | Some(libc::EACCES) => format!("{what}: not yours to change (EPERM)"),
        Some(libc::ESRCH) => format!("{what}: it exited first"),
        Some(libc::EINVAL) => format!("{what}: the kernel refused the value (EINVAL)"),
        _ => format!("{what}: {e}"),
    }
}

/// Refuse the pids nobody meant to name.
fn check(pids: &[u32]) -> Result<(), String> {
    if pids.is_empty() {
        return Err("no process selected".into());
    }
    let me = std::process::id();
    for &p in pids {
        if p == 0 {
            // `kill(0, sig)` signals the whole process group.
            return Err("pid 0 means every process in the group — refused".into());
        }
        if p == 1 {
            return Err("pid 1 is init — refused".into());
        }
        if p == me {
            return Err("that is gridwatch itself — refused".into());
        }
    }
    Ok(())
}

fn plural(pids: &[u32]) -> String {
    match pids {
        [one] => format!("{one}"),
        many => format!("{} processes", many.len()),
    }
}

/// The handler `ui::actions` calls on the executor thread. Install it once,
/// at startup, with [`install`].
pub fn run(action: &ProcAction) -> Result<String, String> {
    match action {
        ProcAction::Signal { pids, signal, .. } => signal_them(pids, *signal),
        ProcAction::Renice { pids, delta } => renice(pids, *delta),
        ProcAction::Affinity { pid, cpus } => affinity(*pid, cpus),
        ProcAction::IoPrio { pid, class, level } => ioprio(*pid, *class, *level),
    }
}

/// Called once from `run`/`main`; a second call is refused rather than
/// silently replacing the thing that signals processes.
pub fn install() {
    if let Err(e) = gridwatch_ui::actions::set_process_handler(run) {
        tracing::warn!("{e}");
    }
}

fn signal_them(pids: &[u32], signal: i32) -> Result<String, String> {
    check(pids)?;
    let name = signal_name(signal);
    let mut sent = 0;
    for pid in pids {
        // SAFETY: kill(2) with a pid `check` proved is not 0, 1 or
        // ourselves, and a signal number from the table above. It passes no
        // pointers and touches nothing of ours.
        let rc = unsafe { libc::kill(*pid as libc::pid_t, signal) };
        if rc != 0 {
            return Err(errno_says(&format!("{name} to {pid}")));
        }
        sent += 1;
    }
    Ok(format!("{name} sent to {sent} of {}", pids.len()))
}

fn renice(pids: &[u32], delta: i32) -> Result<String, String> {
    check(pids)?;
    let mut last = 0;
    for pid in pids {
        // SAFETY: getpriority/setpriority on PRIO_PROCESS with a checked
        // pid. `getpriority` may legitimately return -1, so errno is
        // cleared first, as its man page requires.
        let now = unsafe {
            *libc::__errno_location() = 0;
            let v = libc::getpriority(libc::PRIO_PROCESS, *pid);
            if v == -1 && *libc::__errno_location() != 0 {
                return Err(errno_says(&format!("read the nice value of {pid}")));
            }
            v
        };
        let want = (now + delta).clamp(-20, 19);
        // SAFETY: as above; setpriority returns 0, or -1 with errno set.
        let rc = unsafe { libc::setpriority(libc::PRIO_PROCESS, *pid, want) };
        if rc != 0 {
            return Err(errno_says(&format!("renice {pid} to {want}")));
        }
        last = want;
    }
    Ok(format!("nice {last} for {}", plural(pids)))
}

fn affinity(pid: u32, cpus: &[usize]) -> Result<String, String> {
    check(&[pid])?;
    if cpus.is_empty() {
        return Err("a process must be allowed at least one CPU".into());
    }
    // SAFETY: a zeroed `cpu_set_t` is a valid empty set; `CPU_SET` only
    // writes bits inside it, and each index is checked against CPU_SETSIZE.
    let mut set: libc::cpu_set_t = unsafe { std::mem::zeroed() };
    for &c in cpus {
        if c >= libc::CPU_SETSIZE as usize {
            return Err(format!("cpu {c} is beyond CPU_SETSIZE"));
        }
        // SAFETY: `c` is in range and `set` is a valid set.
        unsafe { libc::CPU_SET(c, &mut set) };
    }
    // SAFETY: sched_setaffinity with a valid set, its true size, and a
    // checked pid.
    let rc = unsafe {
        libc::sched_setaffinity(
            pid as libc::pid_t,
            std::mem::size_of::<libc::cpu_set_t>(),
            &set,
        )
    };
    if rc != 0 {
        return Err(errno_says(&format!("set the affinity of {pid}")));
    }
    Ok(format!(
        "{pid} pinned to {} cpu{}",
        cpus.len(),
        if cpus.len() == 1 { "" } else { "s" }
    ))
}

/// `IOPRIO_WHO_PROCESS`, and the class's position in the 16-bit value.
const IOPRIO_WHO_PROCESS: libc::c_int = 1;
const IOPRIO_CLASS_SHIFT: u32 = 13;

fn ioprio(pid: u32, class: IoClass, level: u8) -> Result<String, String> {
    check(&[pid])?;
    let value = ((class as i32) << IOPRIO_CLASS_SHIFT) | i32::from(level.min(7));
    // SAFETY: SYS_ioprio_set with WHO_PROCESS, a checked pid and a value
    // built from a 2-bit class and a 3-bit level. It passes no pointers.
    // There is no `nix` binding and no libc wrapper for this call, which is
    // why this file carries the `unsafe` allowance (D58).
    let rc = unsafe {
        libc::syscall(
            libc::SYS_ioprio_set,
            IOPRIO_WHO_PROCESS,
            pid as libc::c_int,
            value,
        )
    };
    if rc != 0 {
        return Err(errno_says(&format!("set the I/O priority of {pid}")));
    }
    Ok(format!("{pid} is now {} {level}", class.name()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every action refuses the pids nobody meant to name, before any
    /// syscall happens.
    #[test]
    fn the_dangerous_pids_are_refused_without_a_syscall() {
        let refuse = |pid: u32| -> String {
            run(&ProcAction::Signal {
                pids: vec![pid],
                signal: libc::SIGTERM,
                signal_name: "SIGTERM".into(),
                names: vec!["x".into()],
            })
            .unwrap_err()
        };
        assert!(refuse(0).contains("group"));
        assert!(refuse(1).contains("init"));
        assert!(refuse(std::process::id()).contains("gridwatch itself"));
        assert!(
            run(&ProcAction::Signal {
                pids: Vec::new(),
                signal: libc::SIGTERM,
                signal_name: "SIGTERM".into(),
                names: Vec::new()
            })
            .unwrap_err()
            .contains("no process selected")
        );
        assert!(
            run(&ProcAction::Affinity {
                pid: std::process::id(),
                cpus: vec![0]
            })
            .unwrap_err()
            .contains("gridwatch itself")
        );
        assert!(
            run(&ProcAction::Affinity {
                pid: 424_242,
                cpus: Vec::new()
            })
            .unwrap_err()
            .contains("at least one CPU")
        );
        assert_eq!(signal_name(libc::SIGKILL), "SIGKILL");
        assert_eq!(signal_name(9999), "signal");
    }

    /// The real syscalls, against a child **this test spawned** — the only
    /// process any test in this project may touch (D58).
    #[test]
    fn the_syscalls_work_on_a_child_of_our_own() {
        let mut child = std::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .expect("sleep is on the path");
        let pid = child.id();

        // Renice, read back from /proc rather than trusting the return.
        // `stat`'s comm field is parenthesised and may contain spaces, so
        // the numbered fields are counted from the last `)`.
        let nice_of = |pid: u32| -> i64 {
            let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).expect("stat");
            let after = stat.rsplit(')').next().expect("fields");
            after
                .split_whitespace()
                .nth(16)
                .and_then(|s| s.parse().ok())
                .expect("the nice field")
        };
        let before = nice_of(pid);
        run(&ProcAction::Renice {
            pids: vec![pid],
            delta: 2,
        })
        .expect("renicing our own child is allowed");
        assert_eq!(nice_of(pid), before + 2, "the nice value moved");

        // Affinity: pin it to cpu 0 and read the mask back.
        run(&ProcAction::Affinity { pid, cpus: vec![0] })
            .expect("pinning our own child is allowed");
        let status = std::fs::read_to_string(format!("/proc/{pid}/status")).expect("status");
        let mask = status
            .lines()
            .find(|l| l.starts_with("Cpus_allowed:"))
            .expect("Cpus_allowed");
        assert_eq!(
            mask.split_whitespace()
                .nth(1)
                .map(|m| m.trim_start_matches('0')),
            Some("1"),
            "one cpu allowed: {mask}"
        );

        // I/O priority.
        run(&ProcAction::IoPrio {
            pid,
            class: IoClass::Idle,
            level: 0,
        })
        .expect("our own child's io priority is ours to set");

        // The signal, which is also how this test cleans up.
        run(&ProcAction::Signal {
            pids: vec![pid],
            signal: libc::SIGTERM,
            signal_name: "SIGTERM".into(),
            names: vec!["sleep".into()],
        })
        .expect("terminating our own child is allowed");
        let status = child.wait().expect("it exits");
        assert!(!status.success(), "it was signalled, not finished");
        // Once it is gone, the same action says so rather than lying.
        let after = run(&ProcAction::Signal {
            pids: vec![pid],
            signal: libc::SIGTERM,
            signal_name: "SIGTERM".into(),
            names: vec!["sleep".into()],
        });
        assert!(
            after.as_ref().is_err_and(|e| e.contains("exited first")),
            "{after:?}"
        );
    }
}
