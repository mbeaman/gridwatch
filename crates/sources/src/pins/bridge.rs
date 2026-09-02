//! The lifecycle bridge (§4.4, brief arc 3 seam 2–3): astral-watch's
//! `Lifecycle` is the only debouncer; its `Event`s become `AlertEvent`s under
//! `pins/<condition>` and the active set the component renders. `Instant` is
//! injected, so tests drive time.

use std::sync::Arc;
use std::time::Instant;

use astral_watch::alert::{Alert, Thresholds, evaluate};
use astral_watch::config::AlertPolicy;
use astral_watch::decode::Reading;
use astral_watch::lifecycle::{Condition, Event, Lifecycle, condition_of};
use gridwatch_store::keys::pins::{self, ActiveCondition};
use gridwatch_store::{AlertEvent, AlertId, Severity, Transition, Ts};

pub fn severity(c: Condition) -> Severity {
    match c {
        Condition::Overload | Condition::Disconnected | Condition::Imbalance => Severity::Crit,
        Condition::ImbalanceAdvisory => Severity::Warn,
        Condition::TelemetryLost => Severity::Info,
    }
}

pub fn alert_id(c: Condition) -> AlertId {
    AlertId(Arc::from(format!("pins/{}", c.id())))
}

/// The lifecycle plus the active set gridwatch tracks itself (D50 §1: no
/// `Lifecycle::active()` upstream).
pub struct Bridge {
    thresholds: Thresholds,
    lifecycle: Lifecycle,
    active: Vec<ActiveCondition>,
}

impl Bridge {
    pub fn new(thresholds: Thresholds, policy: AlertPolicy) -> Bridge {
        Bridge {
            thresholds,
            lifecycle: Lifecycle::new(policy),
            active: Vec::new(),
        }
    }

    pub fn thresholds(&self) -> &Thresholds {
        &self.thresholds
    }

    pub fn active(&self) -> &[ActiveCondition] {
        &self.active
    }

    /// Instantaneous conditions for a plausible reading.
    pub fn conditions(&self, r: &Reading) -> Vec<(Condition, String)> {
        evaluate(r, &self.thresholds)
            .into_iter()
            .map(|a: Alert| (condition_of(&a), a.to_string()))
            .collect()
    }

    /// Feed one sample — `Some(reading)` or `None` with the loss message —
    /// and get the alert events to send.
    pub fn observe(
        &mut self,
        now: Instant,
        at: Ts,
        sample: Result<&Reading, &str>,
    ) -> Vec<AlertEvent> {
        let present = match sample {
            Ok(r) => self.conditions(r),
            Err(msg) => vec![(Condition::TelemetryLost, msg.to_string())],
        };
        self.lifecycle
            .observe(now, &present)
            .into_iter()
            .map(|ev| self.translate(ev, at))
            .collect()
    }

    fn translate(&mut self, ev: Event, at: Ts) -> AlertEvent {
        let c = ev.condition();
        let (transition, detail) = match &ev {
            Event::Raised { detail, .. } => {
                self.active.retain(|a| a.id != c.id());
                self.active.push(ActiveCondition {
                    id: c.id().into(),
                    detail: detail.clone(),
                    since: at,
                });
                (Transition::Raised, detail.clone())
            }
            Event::Repeated { detail, .. } => {
                if let Some(a) = self.active.iter_mut().find(|a| a.id == c.id()) {
                    a.detail = detail.clone();
                }
                (Transition::Repeated, detail.clone())
            }
            Event::Resolved { active_for, .. } => {
                self.active.retain(|a| a.id != c.id());
                (
                    Transition::Resolved,
                    format!(
                        "clear after {}",
                        astral_watch::lifecycle::fmt_duration(*active_for)
                    ),
                )
            }
        };
        AlertEvent {
            id: alert_id(c),
            source: pins::SOURCE,
            severity: severity(c),
            transition,
            title: Arc::from(c.label()),
            detail: Arc::from(detail.as_str()),
            at,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use astral_watch::decode::Pin;
    use std::time::Duration;

    fn reading(amps: [f64; 6]) -> Reading {
        let mut pins = [Pin {
            volts: 12.05,
            amps: 0.0,
        }; 6];
        for (p, a) in pins.iter_mut().zip(amps) {
            p.amps = a;
        }
        Reading { pins }
    }

    #[test]
    fn overload_raises_after_three_of_five_and_resolves_after_twenty_clean() {
        let mut b = Bridge::new(Thresholds::default(), AlertPolicy::default());
        let t0 = Instant::now();
        let hot = reading([9.5, 9.4, 1.5, 1.5, 1.5, 1.5]);
        let ok = reading([1.7, 1.6, 1.5, 1.5, 1.4, 1.4]);
        let mut events = Vec::new();
        for i in 0..3u64 {
            events.extend(b.observe(
                t0 + Duration::from_millis(500 * i),
                Ts(i * 500_000_000),
                Ok(&hot),
            ));
        }
        // 9.5 A on two pins is an overload *and* an alarm-grade imbalance
        // (hi/lo 6.3 with the hottest pin over 7.82 A): two raises, both Crit.
        assert_eq!(events.len(), 2, "{events:?}");
        let over = events
            .iter()
            .find(|e| e.id.0.as_ref() == "pins/overload")
            .expect("overload raised");
        assert_eq!(over.severity, Severity::Crit);
        assert_eq!(over.transition, Transition::Raised);
        assert_eq!(over.title.as_ref(), "OVERLOAD");
        assert!(over.detail.contains("pins 1+2"));
        assert!(events.iter().any(|e| e.id.0.as_ref() == "pins/imbalance"));
        assert_eq!(b.active().len(), 2);
        let mut resolved = Vec::new();
        for i in 3..30u64 {
            resolved.extend(b.observe(
                t0 + Duration::from_millis(500 * i),
                Ts(i * 500_000_000),
                Ok(&ok),
            ));
        }
        assert_eq!(resolved.len(), 2, "{resolved:?}");
        assert!(
            resolved
                .iter()
                .all(|e| e.transition == Transition::Resolved)
        );
        assert!(resolved.iter().all(|e| e.detail.starts_with("clear after")));
        assert!(b.active().is_empty());
    }

    #[test]
    fn telemetry_lost_is_info_and_freezes_the_rest() {
        let mut b = Bridge::new(Thresholds::default(), AlertPolicy::default());
        let t0 = Instant::now();
        let hot = reading([9.5, 9.4, 1.5, 1.5, 1.5, 1.5]);
        for i in 0..3u64 {
            b.observe(t0, Ts(i), Ok(&hot));
        }
        assert!(b.active().iter().any(|a| a.id == "overload"));
        // Lost telemetry: TelemetryLost raises (Info), the overload is frozen —
        // twenty lossy samples never resolve it.
        let mut evs = Vec::new();
        for i in 3..30u64 {
            evs.extend(b.observe(t0, Ts(i), Err("read failed: EIO")));
        }
        assert!(
            evs.iter()
                .any(|e| e.id.0.as_ref() == "pins/telemetry_lost" && e.severity == Severity::Info)
        );
        assert!(
            b.active().iter().any(|a| a.id == "overload"),
            "no-data must not read as health"
        );
    }
}
