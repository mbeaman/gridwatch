//! Process actions as **data** (§4.6, D58 amendment 1).
//!
//! A component may not make a syscall — it describes and never does — but
//! it is the only thing that knows which row the cursor is on. So the four
//! process actions are data types here in `ui`, constructed by the
//! component and handed to the shell as `Command::Run`; the shell installs
//! a **handler** at startup and the executor thread calls it.
//!
//! The alternative would have been to define these in `app`, where the
//! syscalls live — but `components` cannot depend on `app` (the crate
//! direction is `store ← ui ← components ← app`), so the type has to be
//! visible here. A function pointer keeps the direction intact and keeps
//! `unsafe` in the one crate that already carries a documented allowance.

use std::sync::OnceLock;

use crate::component::Action;

/// The I/O scheduling classes, as `ioprio_set` numbers them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IoClass {
    None = 0,
    RealTime = 1,
    BestEffort = 2,
    Idle = 3,
}

impl IoClass {
    pub fn name(self) -> &'static str {
        match self {
            IoClass::None => "none",
            IoClass::RealTime => "realtime",
            IoClass::BestEffort => "best-effort",
            IoClass::Idle => "idle",
        }
    }
}

/// What a person asked to have done to a process.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProcAction {
    /// `F9` / `k`. The signal number is the platform's; the component
    /// gets it from its own table (`components::htop::SIGNALS`), because
    /// `ui` may not call libc.
    Signal {
        pids: Vec<u32>,
        signal: i32,
        /// The signal's name, so the question reads like a sentence
        /// without `ui` knowing the platform's numbers.
        signal_name: String,
        /// What the rows were called.
        names: Vec<String>,
    },
    /// `F7` / `F8`.
    Renice { pids: Vec<u32>, delta: i32 },
    /// `a`.
    Affinity { pid: u32, cpus: Vec<usize> },
    /// `i`.
    IoPrio { pid: u32, class: IoClass, level: u8 },
}

impl ProcAction {
    pub fn pids(&self) -> Vec<u32> {
        match self {
            ProcAction::Signal { pids, .. } | ProcAction::Renice { pids, .. } => pids.clone(),
            ProcAction::Affinity { pid, .. } | ProcAction::IoPrio { pid, .. } => vec![*pid],
        }
    }
}

fn plural(pids: &[u32]) -> String {
    match pids {
        [one] => format!("{one}"),
        many => format!("{} processes", many.len()),
    }
}

impl Action for ProcAction {
    fn run(self: Box<Self>) -> Result<String, String> {
        match HANDLER.get() {
            Some(h) => h(&self),
            // Only reachable in a build that never installed one — a unit
            // test, or `shot`. Saying so beats a panic or a silent Ok.
            None => Err(format!("{self:?}: no process handler installed")),
        }
    }

    fn confirm(&self) -> Option<String> {
        match self {
            ProcAction::Signal {
                pids,
                signal_name,
                names,
                ..
            } => {
                let who = match (names.as_slice(), pids.as_slice()) {
                    ([one], [pid]) => format!("{one} ({pid})"),
                    _ => plural(pids),
                };
                Some(format!("{} {who}?", signal_name.to_lowercase()))
            }
            // One step is undone by the opposite key; a bigger jump asks.
            ProcAction::Renice { pids, delta } => {
                (delta.abs() > 1).then(|| format!("renice {} by {delta:+}?", plural(pids)))
            }
            ProcAction::Affinity { pid, cpus } => {
                Some(format!("pin {pid} to {} cpu(s)?", cpus.len()))
            }
            ProcAction::IoPrio { pid, class, level } => {
                Some(format!("set {pid} to {} {level}?", class.name()))
            }
        }
    }

    fn pids(&self) -> Option<Vec<u32>> {
        Some(ProcAction::pids(self))
    }
}

/// The handler the shell installs. It runs on the executor thread.
pub type ProcHandler = fn(&ProcAction) -> Result<String, String>;

static HANDLER: OnceLock<ProcHandler> = OnceLock::new();

/// Install the handler. The first call wins: a second is a programming
/// error, and quietly replacing the thing that signals processes would be
/// a poor way to find out.
pub fn set_process_handler(h: ProcHandler) -> Result<(), &'static str> {
    HANDLER
        .set(h)
        .map_err(|_| "a process handler is already installed")
}

pub fn has_process_handler() -> bool {
    HANDLER.get().is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_questions_read_like_sentences() {
        let one = ProcAction::Signal {
            pids: vec![4242],
            signal: 15,
            signal_name: "SIGTERM".into(),
            names: vec!["firefox".into()],
        };
        assert_eq!(one.confirm().as_deref(), Some("sigterm firefox (4242)?"));
        assert_eq!(Action::pids(&one), Some(vec![4242]));
        let many = ProcAction::Signal {
            pids: vec![1, 2, 3],
            signal: 9,
            signal_name: "SIGKILL".into(),
            names: vec!["a".into(), "b".into(), "c".into()],
        };
        assert_eq!(many.confirm().as_deref(), Some("sigkill 3 processes?"));
        // A one-step renice does not ask; a five-step one does.
        assert_eq!(
            ProcAction::Renice {
                pids: vec![9],
                delta: 1
            }
            .confirm(),
            None
        );
        assert!(
            ProcAction::Renice {
                pids: vec![9],
                delta: -5
            }
            .confirm()
            .is_some()
        );
        assert_eq!(
            ProcAction::IoPrio {
                pid: 7,
                class: IoClass::Idle,
                level: 0
            }
            .confirm()
            .as_deref(),
            Some("set 7 to idle 0?")
        );
        assert_eq!(IoClass::BestEffort.name(), "best-effort");
    }

    /// With no handler (a unit test, or `shot`), running says so rather
    /// than pretending it worked.
    #[test]
    fn without_a_handler_an_action_says_so() {
        // This test process may have one installed by another test in the
        // same binary; both outcomes are honest, only silence is not.
        let r = Box::new(ProcAction::Renice {
            pids: vec![std::process::id()],
            delta: 0,
        })
        .run();
        match r {
            Err(e) => assert!(
                e.contains("no process handler") || e.contains("gridwatch"),
                "{e}"
            ),
            Ok(msg) => assert!(!msg.is_empty()),
        }
    }
}
