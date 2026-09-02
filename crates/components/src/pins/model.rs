//! The tile's derived state (§8.1's "derive in `tick`"): the latest reading
//! from the store, the session peaks (`r` resets), the static row and the
//! active set. History is the store's own (`window`/`resample`); nothing here
//! keeps a second ring.

use gridwatch_store::keys::pins::{self, PinsInfo, PinsState};
use gridwatch_store::{Store, Ts};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Model {
    pub at: Option<Ts>,
    pub amps: [Option<f64>; 6],
    pub volts: [Option<f64>; 6],
    pub total_a: Option<f64>,
    pub total_w: Option<f64>,
    pub balance: Option<f64>,
    pub peaks: [f64; 6],
    pub peak_w: f64,
    pub info: Option<PinsInfo>,
    pub state: Option<PinsState>,
    /// Samples seen this session (tui.rs prints `samples N`).
    pub samples: u64,
    /// When the pins last *read* (a loss tick publishes no amps), for the
    /// stale rule (review: a loss counted as a fresh sample).
    pub last_reading: Option<Ts>,
}

impl Model {
    pub fn refresh(&mut self, store: &Store, at: Ts) {
        self.at = Some(at);
        for pin in 1..=pins::PIN_COUNT {
            let i = usize::from(pin - 1);
            self.amps[i] = store.last(&pins::AMPS.idx(pin)).map(|(_, v)| v);
            self.volts[i] = store.last(&pins::VOLTS.idx(pin)).map(|(_, v)| v);
        }
        if let Some((t, _)) = store.last(&pins::AMPS.idx(1)) {
            self.last_reading = Some(t);
        }
        self.total_a = store.last(&pins::TOTAL_A).map(|(_, v)| v);
        self.total_w = store.last(&pins::TOTAL_W).map(|(_, v)| v);
        // `pins.balance` is absent when undefined; a stale value from an
        // earlier sample must not survive, so the timestamp is checked.
        self.balance = store
            .last(&pins::BALANCE)
            .filter(|(t, _)| *t == at)
            .map(|(_, v)| v);
        self.info = store.record(&pins::INFO).map(|(_, i)| i.clone());
        self.state = store.record(&pins::STATE).map(|(_, s)| s.clone());
        self.samples += 1;
        self.observe_peaks(store);
    }

    pub fn observe_peaks(&mut self, store: &Store) {
        for pin in 1..=pins::PIN_COUNT {
            let i = usize::from(pin - 1);
            if let Some((_, a)) = store.last(&pins::AMPS.idx(pin)) {
                self.peaks[i] = self.peaks[i].max(a);
            }
        }
        if let Some((_, w)) = store.last(&pins::TOTAL_W) {
            self.peak_w = self.peak_w.max(w);
        }
    }

    pub fn reset_peaks(&mut self) {
        self.peaks = [0.0; 6];
        self.peak_w = 0.0;
    }

    /// Thresholds in force: the source's (`pins.info`) or astral-watch's constants.
    pub fn overload_a(&self) -> f64 {
        self.info
            .as_ref()
            .map(|i| i.overload_a)
            .unwrap_or(pins::OVERLOAD_A)
    }

    pub fn imbalance_ratio(&self) -> f64 {
        self.info
            .as_ref()
            .map(|i| i.imbalance_ratio)
            .unwrap_or(pins::IMBALANCE_RATIO)
    }

    pub fn min_load_a(&self) -> f64 {
        self.info
            .as_ref()
            .map(|i| i.min_load_a)
            .unwrap_or(pins::MIN_LOAD_A)
    }

    /// The imbalance-alarm band: `IMBALANCE_ALARM_PIN_FRAC × overload` (7.82 A).
    pub fn warn_a(&self) -> f64 {
        self.overload_a() * pins::IMBALANCE_ALARM_PIN_FRAC
    }

    pub fn telemetry_lost(&self) -> bool {
        self.state.as_ref().is_some_and(|s| s.telemetry_lost)
    }

    /// `(min, max)` pin voltage among the pins that read.
    pub fn volt_range(&self) -> Option<(f64, f64)> {
        let vs: Vec<f64> = self.volts.iter().flatten().copied().collect();
        if vs.is_empty() {
            return None;
        }
        Some((
            vs.iter().copied().fold(f64::MAX, f64::min),
            vs.iter().copied().fold(0.0, f64::max),
        ))
    }
}

/// tui.rs's balance gauge classes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BalanceClass {
    Idle,
    Normal,
    Warn,
    Alarm,
    Unknown,
}

pub fn balance_class(
    balance: Option<f64>,
    total_a: Option<f64>,
    min_load_a: f64,
    alarm_ratio: f64,
) -> BalanceClass {
    match (balance, total_a) {
        (_, Some(t)) if t <= min_load_a => BalanceClass::Idle,
        (Some(b), _) if b > alarm_ratio => BalanceClass::Alarm,
        (Some(b), _) if b > pins::BALANCE_WARN => BalanceClass::Warn,
        (Some(_), _) => BalanceClass::Normal,
        (None, _) => BalanceClass::Unknown,
    }
}

impl BalanceClass {
    pub fn label(self) -> &'static str {
        match self {
            BalanceClass::Idle => "idle",
            BalanceClass::Normal => "NORMAL",
            BalanceClass::Warn => "WARN",
            BalanceClass::Alarm => "ALARM",
            BalanceClass::Unknown => "—",
        }
    }
}
