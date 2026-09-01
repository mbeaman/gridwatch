//! Alert events and the active-alert log (§4.4). Domain alerts arrive on the
//! control channel; the rule engine lands in arc 7, the overlay in arc 3.

use std::collections::BTreeMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::ring::Ring;
use crate::source::SourceId;
use crate::ts::Ts;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Severity {
    Info,
    Warn,
    Crit,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct AlertId(pub Arc<str>);

impl AlertId {
    pub fn new(s: &str) -> AlertId {
        AlertId(Arc::from(s))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Transition {
    Raised,
    Repeated,
    Resolved,
}

#[derive(Clone, Debug)]
pub struct AlertEvent {
    pub id: AlertId,
    pub source: SourceId,
    pub severity: Severity,
    pub transition: Transition,
    pub title: Arc<str>,
    pub detail: Arc<str>,
    pub at: Ts,
}

#[derive(Clone, Debug)]
pub struct ActiveAlert {
    pub severity: Severity,
    pub title: Arc<str>,
    pub detail: Arc<str>,
    pub since: Ts,
    pub last: Ts,
}

const EVENT_RING: usize = 500;

/// Active set keyed by `AlertId` plus a bounded event ring. The overlay keys on
/// transitions, never on samples (§4.4).
#[derive(Debug)]
pub struct AlertLog {
    active: BTreeMap<AlertId, ActiveAlert>,
    ring: Ring<AlertEvent>,
}

impl Default for AlertLog {
    fn default() -> AlertLog {
        AlertLog {
            active: BTreeMap::new(),
            ring: Ring::new(EVENT_RING),
        }
    }
}

impl AlertLog {
    pub fn observe(&mut self, ev: &AlertEvent) {
        match ev.transition {
            Transition::Raised | Transition::Repeated => {
                let entry = self
                    .active
                    .entry(ev.id.clone())
                    .or_insert_with(|| ActiveAlert {
                        severity: ev.severity,
                        title: ev.title.clone(),
                        detail: ev.detail.clone(),
                        since: ev.at,
                        last: ev.at,
                    });
                entry.severity = ev.severity;
                entry.title = ev.title.clone();
                entry.detail = ev.detail.clone();
                entry.last = ev.at;
            }
            Transition::Resolved => {
                self.active.remove(&ev.id);
            }
        }
        self.ring.push(ev.clone());
    }

    pub fn active(&self) -> impl Iterator<Item = (&AlertId, &ActiveAlert)> {
        self.active.iter()
    }

    /// Worst currently-active severity, if any alert is active.
    pub fn worst_active(&self) -> Option<Severity> {
        self.active.values().map(|a| a.severity).max()
    }

    pub fn events(&self) -> impl DoubleEndedIterator<Item = &AlertEvent> {
        self.ring.iter()
    }
}
