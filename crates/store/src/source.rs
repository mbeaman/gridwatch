//! The source contract (§4.3): singletons per kind, demand-driven cadence,
//! zero-poll sleeps on the shared phase grid, statuses that are never dropped.

use std::any::Any;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::alert::AlertEvent;
use crate::capability::Capability;
use crate::msg::{Batch, Channels, ControlMsg, Sample};
use crate::ts::{Clock, Ts};

/// A source's identity: a static name, so a `SourceId` in a journal is
/// *interned* back onto the catalogue on read (`key::intern_source`) rather
/// than leaked — an unknown source name fails to deserialise (§4.1, §4.5).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub struct SourceId(pub &'static str);

impl<'de> Deserialize<'de> for SourceId {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<SourceId, D::Error> {
        let name = String::deserialize(d)?;
        crate::key::intern_source(&name)
            .ok_or_else(|| serde::de::Error::custom(format!("unknown source `{name}`")))
    }
}

impl std::fmt::Display for SourceId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}

/// How much a source is wanted, written by the app after every layout solve (§5).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
pub enum Level {
    Paused = 0,
    Hidden = 1,
    Visible = 2,
    Focused = 3,
}

impl Level {
    pub fn from_u8(v: u8) -> Level {
        match v {
            0 => Level::Paused,
            1 => Level::Hidden,
            2 => Level::Visible,
            _ => Level::Focused,
        }
    }
}

/// What the richest visible tier needs from a source (§4.3, D39): meters only,
/// a pid-level process scan, or per-column gated files.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
pub enum Detail {
    #[default]
    Meters = 0,
    Table = 1,
    Columns = 2,
}

impl Detail {
    pub fn from_u8(v: u8) -> Detail {
        match v {
            0 => Detail::Meters,
            1 => Detail::Table,
            _ => Detail::Columns,
        }
    }
}

#[derive(Debug)]
pub struct Demand {
    level: AtomicU8,
    detail: AtomicU8,
}

impl Default for Demand {
    fn default() -> Demand {
        Demand {
            level: AtomicU8::new(Level::Hidden as u8),
            detail: AtomicU8::new(Detail::Meters as u8),
        }
    }
}

impl Demand {
    pub fn set(&self, level: Level, detail: Detail) {
        self.level.store(level as u8, Ordering::Release);
        self.detail.store(detail as u8, Ordering::Release);
    }

    pub fn level(&self) -> Level {
        Level::from_u8(self.level.load(Ordering::Acquire))
    }

    pub fn detail(&self) -> Detail {
        Detail::from_u8(self.detail.load(Ordering::Acquire))
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Cadence {
    pub hidden: Option<Duration>,
    pub visible: Duration,
    pub focused: Duration,
    pub always_on: bool,
}

impl Cadence {
    /// The polling period at a demand level; `None` = do not poll.
    pub fn for_level(&self, level: Level) -> Option<Duration> {
        match level {
            // always_on keeps alert rules fed, but at the *hidden* cadence —
            // an unwatched source never earns its visible budget (§5 table).
            Level::Paused => {
                if self.always_on {
                    self.hidden.or(Some(self.visible))
                } else {
                    None
                }
            }
            Level::Hidden => {
                if self.always_on {
                    self.hidden.or(Some(self.visible))
                } else {
                    self.hidden
                }
            }
            Level::Visible => Some(self.visible),
            Level::Focused => Some(self.focused),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct SourceInfo {
    pub id: SourceId,
    pub produces: &'static [&'static str],
    pub cadence: Cadence,
    pub requires: &'static [Capability],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SourceState {
    Starting,
    Ok,
    Degraded,
    Unavailable,
    Stopped,
}

#[derive(Clone, Debug)]
pub struct SourceStatus {
    pub state: SourceState,
    pub reason: Option<Arc<str>>,
    pub hint: Option<Arc<str>>,
    pub since: Ts,
    pub last_sample: Option<Ts>,
    pub dropped: u64,
    pub restarts: u32,
}

impl SourceStatus {
    pub fn starting(at: Ts) -> SourceStatus {
        SourceStatus {
            state: SourceState::Starting,
            reason: None,
            hint: None,
            since: at,
            last_sample: None,
            dropped: 0,
            restarts: 0,
        }
    }
}

/// Control messages into a source. `Stop` also flips the shared stop flag.
pub enum Control {
    Stop,
    SetOption(String, toml::Value),
    Restart,
    Domain(Box<dyn Any + Send>),
}

impl std::fmt::Debug for Control {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Control::Stop => f.write_str("Stop"),
            Control::SetOption(k, v) => write!(f, "SetOption({k}, {v})"),
            Control::Restart => f.write_str("Restart"),
            Control::Domain(_) => f.write_str("Domain(..)"),
        }
    }
}

/// Everything a source thread needs (§4.3).
pub struct SourceCtx {
    pub id: SourceId,
    ch: Channels,
    pub clock: Clock,
    pub stop: Arc<AtomicBool>,
    pub demand: Arc<Demand>,
    ctl: Receiver<Control>,
    pending: Mutex<VecDeque<Control>>,
    dropped: AtomicU64,
    pub options: toml::Table,
    /// Restart count owned by the supervisor; stamped onto every status so a
    /// recreated source cannot regress the counter to 0.
    pub restarts: u32,
}

impl SourceCtx {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: SourceId,
        ch: Channels,
        clock: Clock,
        stop: Arc<AtomicBool>,
        demand: Arc<Demand>,
        ctl: Receiver<Control>,
        options: toml::Table,
        restarts: u32,
    ) -> SourceCtx {
        SourceCtx {
            id,
            ch,
            clock,
            stop,
            demand,
            ctl,
            pending: Mutex::new(VecDeque::new()),
            dropped: AtomicU64::new(0),
            options,
            restarts,
        }
    }

    pub fn stopped(&self) -> bool {
        self.stop.load(Ordering::Acquire)
    }

    /// Telemetry: lossy by design; a drop increments the counter carried by the
    /// next status (§4.3).
    pub fn emit(&self, at: Ts, samples: Vec<Sample>) {
        let batch = Batch {
            source: self.id,
            at,
            samples,
        };
        if self.ch.data.try_send(batch).is_err() {
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }

    pub fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }

    /// Control plane: never dropped.
    pub fn status(&self, mut s: SourceStatus) {
        s.dropped = self.dropped();
        s.restarts = self.restarts;
        let _ = self.ch.control.send(ControlMsg::Status(self.id, s));
    }

    pub fn alert(&self, e: AlertEvent) {
        let _ = self.ch.control.send(ControlMsg::Alert(e));
    }

    /// Re-emit a journaled message through the normal channels, as the source
    /// it was recorded from (§4.5, D47 seam 2): batches go on `data`
    /// **blocking** — replay must never drop a line, it waits for the render
    /// thread instead — statuses and alerts on `control`, inputs on `input`.
    /// Only `JournalSource` calls this; a live source has `emit`/`status`.
    /// A blocked `send` returns only when a receiver takes a slot or every
    /// receiver is gone, so the app **drops its `Inbox` before joining** the
    /// journal source (D48) — otherwise a full channel at quit is a deadlock.
    pub fn inject(&self, msg: crate::msg::Msg) {
        use crate::msg::Msg;
        match msg {
            Msg::Batch(b) => {
                let _ = self.ch.data.send(b);
            }
            Msg::Control(c) => {
                let _ = self.ch.control.send(c);
            }
            Msg::Input(i) => {
                let _ = self.ch.input.send(i);
            }
            Msg::Heartbeat => {}
        }
    }

    /// Non-blocking control read; `Stop` flips the stop flag and is returned.
    pub fn try_control(&self) -> Option<Control> {
        if let Some(c) = self
            .pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .pop_front()
        {
            return Some(self.note_stop(c));
        }
        self.ctl.try_recv().ok().map(|c| self.note_stop(c))
    }

    fn note_stop(&self, c: Control) -> Control {
        if matches!(c, Control::Stop) {
            self.stop.store(true, Ordering::Release);
        }
        c
    }

    /// Zero-poll wait (§4.3, D39): parks on the control receiver until the
    /// deadline or `Control::Stop`. Non-stop controls received while parked are
    /// queued for `try_control`. Returns `false` when stopped.
    pub fn sleep_until(&self, deadline: Ts) -> bool {
        loop {
            if self.stopped() {
                return false;
            }
            let now = self.clock.now();
            if now >= deadline {
                return true;
            }
            let wait = deadline.since(now);
            match self.ctl.recv_timeout(wait) {
                Ok(Control::Stop) => {
                    self.stop.store(true, Ordering::Release);
                    return false;
                }
                Ok(c) => self
                    .pending
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .push_back(c),
                Err(RecvTimeoutError::Timeout) => return !self.stopped(),
                Err(RecvTimeoutError::Disconnected) => {
                    // App gone; behave like a plain sleep so the loop can notice stop.
                    std::thread::sleep(wait.min(Duration::from_millis(200)));
                }
            }
        }
    }

    /// Next deadline on the shared phase grid: the next multiple of `cadence`
    /// from the epoch, so sources with common cadences wake together (P5).
    pub fn next_deadline(&self, cadence: Duration) -> Ts {
        let c = cadence.as_nanos().max(1) as u64;
        let now = self.clock.now().0;
        Ts(((now / c) + 1) * c)
    }
}

/// A blocking source: owns its thread's loop.
pub trait Source: Send + 'static {
    fn info(&self) -> SourceInfo;
    fn run(self: Box<Self>, cx: SourceCtx);
}

#[derive(Clone, Debug)]
pub struct SourceError {
    pub reason: String,
    pub hint: Option<String>,
}

/// Poll-style helper: the supervisor drives cadence and backoff (§4.3).
pub trait Sampler: Send + 'static {
    fn sample(&mut self, now: Ts, detail: Detail) -> Result<Vec<Sample>, SourceError>;
}

/// How the registry builds a source: live, or seeded synthetic for `--demo` (§4.3).
#[derive(Clone, Copy)]
pub struct SourceDef {
    pub info: SourceInfo,
    pub start: fn(&toml::Table) -> Box<dyn Source>,
    pub demo: fn(u64) -> Box<dyn Source>,
}

impl std::fmt::Debug for SourceDef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "SourceDef({})", self.info.id)
    }
}
