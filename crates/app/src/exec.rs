//! The action executor (§4.6, seam 11, D58): one thread that runs the
//! `Action`s components ask for, and answers with `ControlMsg::Done` on the
//! control channel the app already drains. Nothing else in the process may
//! change another process.
//!
//! Three rules hold this together:
//!
//! * **One at a time, in arrival order.** An action is a syscall or two; a
//!   queue keeps them off the render thread without a pool to reason about.
//! * **A panicking action is an error, not a dead thread.** `run` is user
//!   code by the time plugins exist (arc 8b), so it is caught.
//! * **The read-only switch is enforced here**, not in each component: a
//!   refusal is a `Done(Err(..))` with the sentence a person will read.
//!
//! `GRIDWATCH_ACTION_ALLOW_PIDS` is the test harness's guard: when it is
//! set, an action whose `pids()` name anything outside that list is
//! refused. No test in this repo may act on a process it did not spawn
//! (D58), and this is what makes that a rule the machine keeps rather than
//! a convention a future test can forget.

use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use gridwatch_store::{ActionId, ControlMsg};
use gridwatch_ui::component::Action;

/// How long an action may take before the executor reports a timeout and
/// moves on. It cannot be killed — a `kill(2)` that hangs is the kernel's
/// business — so this is a report, not a cancellation.
pub const ACTION_TIMEOUT: Duration = Duration::from_secs(5);

/// The env var the tests use to fence the executor to their own children.
pub const ALLOW_PIDS: &str = "GRIDWATCH_ACTION_ALLOW_PIDS";

/// One queued action: what to run, and the id its `Done` will carry.
type Queued = (ActionId, Box<dyn Action>);

pub struct Executor {
    tx: Option<Sender<Queued>>,
    worker: Option<JoinHandle<()>>,
}

/// The pids this run may touch, or `None` for "no fence" (the normal case
/// on a person's machine). Read from the environment once, at startup.
pub fn fence_from_env() -> Option<Vec<u32>> {
    let allowed = std::env::var(ALLOW_PIDS).ok()?;
    Some(
        allowed
            .split(|c: char| !c.is_ascii_digit())
            .filter(|s| !s.is_empty())
            .filter_map(|s| s.parse().ok())
            .collect(),
    )
}

/// Why an action was refused before it ran.
fn refusal(action: &dyn Action, fence: Option<&Vec<u32>>) -> Option<String> {
    let allowed = fence?;
    let named = action.pids();
    let stray: Vec<u32> = named
        .iter()
        .copied()
        .filter(|p| !allowed.contains(p))
        .collect();
    (!stray.is_empty()).then(|| {
        format!("refused: {stray:?} is not a process this run may touch ({ALLOW_PIDS} is set)")
    })
}

impl Executor {
    /// Spawn the thread. `done` is how a finished action gets back to the
    /// store — the same control channel a source's status travels on.
    pub fn new(done: Sender<ControlMsg>) -> Executor {
        Executor::with_fence(done, fence_from_env())
    }

    /// The fence spelled out rather than read from the environment — what
    /// the tests use, since setting an env var is `unsafe` in this edition
    /// and this crate denies it.
    pub fn with_fence(done: Sender<ControlMsg>, fence: Option<Vec<u32>>) -> Executor {
        let (tx, rx): (Sender<Queued>, Receiver<Queued>) = channel();
        let worker = std::thread::Builder::new()
            .name("gw-exec".into())
            .spawn(move || {
                while let Ok((id, action)) = rx.recv() {
                    let what = format!("{action:?}");
                    let result = match refusal(action.as_ref(), fence.as_ref()) {
                        Some(why) => Err(why),
                        None => {
                            let t0 = Instant::now();
                            let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                                action.run()
                            }));
                            let took = t0.elapsed();
                            match r {
                                Ok(Ok(msg)) if took > ACTION_TIMEOUT => {
                                    Err(format!("{msg} — but it took {:.1}s", took.as_secs_f64()))
                                }
                                Ok(other) => other,
                                Err(_) => Err(format!("{what} panicked")),
                            }
                        }
                    };
                    tracing::info!(action = %what, ok = result.is_ok(), "action finished");
                    if done.send(ControlMsg::Done(id, result)).is_err() {
                        return; // the app is gone
                    }
                }
            })
            .ok();
        Executor {
            tx: Some(tx),
            worker,
        }
    }

    /// Queue an action. `Err` means the thread is gone, which is a bug
    /// worth surfacing rather than swallowing.
    pub fn run(&self, id: ActionId, action: Box<dyn Action>) -> Result<(), String> {
        self.tx
            .as_ref()
            .ok_or_else(|| "the executor has stopped".to_string())?
            .send((id, action))
            .map_err(|_| "the executor has stopped".to_string())
    }
}

impl Drop for Executor {
    fn drop(&mut self) {
        // Close the queue, then wait for the action in flight.
        self.tx = None;
        if let Some(h) = self.worker.take() {
            let _ = h.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug)]
    struct Says(&'static str, bool);

    impl Action for Says {
        fn run(self: Box<Self>) -> Result<String, String> {
            if self.1 {
                Ok(self.0.to_string())
            } else {
                Err(self.0.to_string())
            }
        }
    }

    #[derive(Debug)]
    struct Panics;

    impl Action for Panics {
        fn run(self: Box<Self>) -> Result<String, String> {
            panic!("the action exploded");
        }
    }

    #[derive(Debug)]
    struct Touches(u32);

    impl Action for Touches {
        fn run(self: Box<Self>) -> Result<String, String> {
            Ok(format!("touched {}", self.0))
        }

        fn pids(&self) -> Vec<u32> {
            vec![self.0]
        }
    }

    fn drain(rx: &Receiver<ControlMsg>, n: usize) -> Vec<(ActionId, Result<String, String>)> {
        let mut out = Vec::new();
        for _ in 0..n {
            match rx.recv_timeout(Duration::from_secs(5)) {
                Ok(ControlMsg::Done(id, r)) => out.push((id, r)),
                other => panic!("expected Done, got {other:?}"),
            }
        }
        out
    }

    #[test]
    fn actions_run_in_order_and_report_both_ways() {
        let (tx, rx) = channel();
        let ex = Executor::new(tx);
        ex.run(ActionId(1), Box::new(Says("first", true))).unwrap();
        ex.run(ActionId(2), Box::new(Says("second", false)))
            .unwrap();
        let done = drain(&rx, 2);
        assert_eq!(done[0].0, ActionId(1));
        assert_eq!(done[0].1, Ok("first".into()));
        assert_eq!(done[1].0, ActionId(2));
        assert_eq!(done[1].1, Err("second".into()));
    }

    /// A component's action is user code; the thread has to survive it.
    #[test]
    fn a_panicking_action_is_an_error_and_the_thread_lives() {
        let (tx, rx) = channel();
        let ex = Executor::new(tx);
        ex.run(ActionId(1), Box::new(Panics)).unwrap();
        ex.run(ActionId(2), Box::new(Says("still here", true)))
            .unwrap();
        let done = drain(&rx, 2);
        assert!(
            done[0].1.as_ref().unwrap_err().contains("panicked"),
            "{:?}",
            done[0].1
        );
        assert_eq!(done[1].1, Ok("still here".into()));
    }

    /// The guard that makes "no test touches a process it did not spawn"
    /// a rule the code keeps.
    #[test]
    fn the_pid_fence_refuses_anything_it_was_not_told_about() {
        // Not this test's own pid: a number nothing here spawned.
        let (tx, rx) = channel();
        // Not this test's own pid: a number nothing here spawned.
        let ex = Executor::with_fence(tx, Some(vec![424_242]));
        ex.run(ActionId(1), Box::new(Touches(1))).unwrap();
        ex.run(ActionId(2), Box::new(Touches(424_242))).unwrap();
        // An action that names no pid at all is unaffected.
        ex.run(ActionId(3), Box::new(Says("no pids", true)))
            .unwrap();
        let done = drain(&rx, 3);
        assert!(
            done[0].1.as_ref().unwrap_err().contains("refused"),
            "pid 1 must be refused: {:?}",
            done[0].1
        );
        assert_eq!(done[1].1, Ok("touched 424242".into()));
        assert_eq!(done[2].1, Ok("no pids".into()));
        // With no fence, the same action runs.
        let (tx, rx) = channel();
        let open = Executor::with_fence(tx, None);
        open.run(ActionId(4), Box::new(Touches(1))).unwrap();
        assert_eq!(drain(&rx, 1)[0].1, Ok("touched 1".into()));
    }

    /// Dropping the executor waits for the action in flight rather than
    /// leaving a thread writing into a channel nobody reads.
    #[test]
    fn dropping_joins_the_thread() {
        let (tx, rx) = channel();
        let ex = Executor::new(tx);
        ex.run(ActionId(7), Box::new(Says("done", true))).unwrap();
        drop(ex);
        // The answer arrived before the drop returned.
        assert!(rx.try_recv().is_ok());
    }
}
