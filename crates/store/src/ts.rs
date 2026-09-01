//! Monotonic run/journal time (§4.1): nanoseconds since the run (or journal) epoch.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

/// Nanoseconds since the run/journal epoch. Copy, ordered, serialisable.
#[derive(
    Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Default, Serialize, Deserialize,
)]
pub struct Ts(pub u64);

impl Ts {
    pub const ZERO: Ts = Ts(0);

    pub fn from_duration(d: Duration) -> Ts {
        Ts(u64::try_from(d.as_nanos()).unwrap_or(u64::MAX))
    }

    pub fn plus(self, d: Duration) -> Ts {
        Ts(self
            .0
            .saturating_add(u64::try_from(d.as_nanos()).unwrap_or(u64::MAX)))
    }

    /// Time elapsed from `earlier` to `self`; zero if `earlier` is later.
    pub fn since(self, earlier: Ts) -> Duration {
        Duration::from_nanos(self.0.saturating_sub(earlier.0))
    }

    pub fn as_secs_f64(self) -> f64 {
        self.0 as f64 / 1e9
    }
}

/// The one clock (§4.1): real time for live runs, a shared counter for replay,
/// demo determinism and tests. Everything that needs "now" holds a `Clock`.
#[derive(Clone, Debug)]
pub enum Clock {
    Real { start: Instant },
    Virtual(Arc<AtomicU64>),
}

impl Clock {
    pub fn real_starting_now() -> Clock {
        Clock::Real {
            start: Instant::now(),
        }
    }

    pub fn new_virtual() -> Clock {
        Clock::Virtual(Arc::new(AtomicU64::new(0)))
    }

    pub fn now(&self) -> Ts {
        match self {
            Clock::Real { start } => Ts::from_duration(start.elapsed()),
            Clock::Virtual(v) => Ts(v.load(Ordering::Acquire)),
        }
    }

    /// Advance a virtual clock (no-op on a real one). Replay and tests drive this.
    pub fn set(&self, at: Ts) {
        if let Clock::Virtual(v) = self {
            v.store(at.0, Ordering::Release);
        }
    }
}
