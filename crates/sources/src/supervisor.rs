//! The source supervisor (§4.3, §11): one std thread per source kind, panic
//! containment with restart counters and 250 ms → 30 s backoff. The handle's
//! control sender is re-pointed at every restart so a recreated source keeps
//! its zero-poll parking and still receives SetOption/Restart controls.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::{Sender, channel};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use gridwatch_store::{
    Channels, Clock, Control, Demand, Source, SourceCtx, SourceId, SourceState, SourceStatus,
};

pub struct SourceHandle {
    pub id: SourceId,
    pub demand: Arc<Demand>,
    /// Re-pointed by the supervisor at every restart (never replaced by anyone
    /// else); a control sent in the instant between generations is dropped,
    /// which only telemetry-tuning controls can afford — Stop rides the flag.
    ctl: Arc<Mutex<Sender<Control>>>,
    stop: Arc<AtomicBool>,
    join: Option<JoinHandle<()>>,
}

impl SourceHandle {
    pub fn control(&self, c: Control) {
        if matches!(c, Control::Stop) {
            self.stop.store(true, std::sync::atomic::Ordering::Release);
        }
        let _ = self.ctl.lock().unwrap_or_else(|e| e.into_inner()).send(c);
    }

    /// Stop the thread and wait for it.
    pub fn shutdown(mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::Release);
        let _ = self
            .ctl
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .send(Control::Stop);
        if let Some(j) = self.join.take() {
            let _ = j.join();
        }
    }
}

impl Drop for SourceHandle {
    fn drop(&mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::Release);
        let _ = self
            .ctl
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .send(Control::Stop);
        if let Some(j) = self.join.take() {
            let _ = j.join();
        }
    }
}

/// Spawn a supervised source thread: `mk` rebuilds the source after a panic;
/// backoff runs 250 ms → 30 s; every restart is counted into every status.
pub fn spawn_source(
    id: SourceId,
    mk: impl Fn() -> Box<dyn Source> + Send + 'static,
    ch: Channels,
    clock: Clock,
    options: toml::Table,
) -> SourceHandle {
    let stop = Arc::new(AtomicBool::new(false));
    let demand = Arc::new(Demand::default());
    let (ctl_tx, ctl_rx) = channel::<Control>();
    let ctl_slot = Arc::new(Mutex::new(ctl_tx));
    // Thread names are capped at 15 bytes on Linux; "gw-" keeps room for ids.
    let name = format!("gw-{}", id.0);
    let t_stop = stop.clone();
    let t_demand = demand.clone();
    let t_slot = ctl_slot.clone();
    let join = std::thread::Builder::new()
        .name(name.chars().take(15).collect())
        .spawn(move || {
            let mut restarts: u32 = 0;
            let mut backoff = Duration::from_millis(250);
            let mut ctl_rx = Some(ctl_rx);
            loop {
                if t_stop.load(std::sync::atomic::Ordering::Acquire) {
                    return;
                }
                let src = mk();
                // First generation takes the original receiver; every restart
                // gets a fresh pair whose sender replaces the handle's slot,
                // so controls and zero-poll parking survive the panic (§4.3).
                let rx = ctl_rx.take().unwrap_or_else(|| {
                    let (tx, rx) = channel::<Control>();
                    *t_slot.lock().unwrap_or_else(|e| e.into_inner()) = tx;
                    rx
                });
                let cx = SourceCtx::new(
                    id,
                    ch.clone(),
                    clock.clone(),
                    t_stop.clone(),
                    t_demand.clone(),
                    rx,
                    options.clone(),
                    restarts,
                );
                let result = catch_unwind(AssertUnwindSafe(|| src.run(cx)));
                match result {
                    Ok(()) => return, // clean exit (stop)
                    Err(payload) => {
                        restarts += 1;
                        let reason: String = if let Some(s) = payload.downcast_ref::<&str>() {
                            (*s).to_string()
                        } else if let Some(s) = payload.downcast_ref::<String>() {
                            s.clone()
                        } else {
                            "panic".to_string()
                        };
                        tracing::error!(source = id.0, restarts, "source panicked: {reason}");
                        let _ = ch.control.send(gridwatch_store::ControlMsg::Status(
                            id,
                            SourceStatus {
                                state: SourceState::Unavailable,
                                reason: Some(Arc::from(format!("panicked: {reason}").as_str())),
                                hint: Some(Arc::from("restarting with backoff")),
                                since: clock.now(),
                                last_sample: None,
                                dropped: 0,
                                restarts,
                            },
                        ));
                        // Interruptible backoff (failure path only; steady state
                        // uses the zero-poll sleep inside the source).
                        let mut waited = Duration::ZERO;
                        while waited < backoff {
                            if t_stop.load(std::sync::atomic::Ordering::Acquire) {
                                return;
                            }
                            let step = Duration::from_millis(200).min(backoff - waited);
                            std::thread::sleep(step);
                            waited += step;
                        }
                        backoff = (backoff * 2).min(Duration::from_secs(30));
                    }
                }
            }
        })
        .expect("spawn source thread");
    SourceHandle {
        id,
        demand,
        ctl: ctl_slot,
        stop,
        join: Some(join),
    }
}
