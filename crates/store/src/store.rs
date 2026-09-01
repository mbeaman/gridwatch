//! The single-writer store (§4.2): owned by the render thread; the only
//! mutation is `apply(&Msg)`. Deterministic iteration everywhere (BTreeMap).

use std::collections::BTreeMap;
use std::sync::LazyLock;
use std::time::Duration;

use smallvec::SmallVec;

use crate::alert::{AlertEvent, AlertLog};
use crate::key::{Key, MetricId, RecordValue, Vec32};
use crate::msg::{ControlMsg, Msg};
use crate::series::{Agg, Retention, Series, resample};
use crate::source::{SourceId, SourceStatus};
use crate::ts::Ts;

#[derive(Debug)]
struct PerSource {
    id: SourceId,
    generation: u64,
    last_sample: Option<Ts>,
    status: SourceStatus,
}

/// A read-only row for the `sources` tile.
pub struct SourceOverview<'a> {
    pub id: SourceId,
    pub status: &'a SourceStatus,
    pub generation: u64,
    pub last_sample: Option<Ts>,
}

static DEFAULT_STATUS: LazyLock<SourceStatus> = LazyLock::new(|| SourceStatus::starting(Ts::ZERO));

#[derive(Debug)]
pub struct Store {
    latest: Ts,
    retention: Retention,
    series: BTreeMap<MetricId, Series>,
    sources: BTreeMap<&'static str, PerSource>,
    alerts: AlertLog,
}

impl Default for Store {
    fn default() -> Store {
        Store::new(Retention::default())
    }
}

impl Store {
    pub fn new(retention: Retention) -> Store {
        Store {
            latest: Ts::ZERO,
            retention,
            series: BTreeMap::new(),
            sources: BTreeMap::new(),
            alerts: AlertLog::default(),
        }
    }

    /// Register a source so `status()` has a row before its first message.
    pub fn ensure_source(&mut self, id: SourceId) {
        self.sources.entry(id.0).or_insert_with(|| PerSource {
            id,
            generation: 0,
            last_sample: None,
            status: SourceStatus::starting(Ts::ZERO),
        });
    }

    /// The only mutation (§4.2). Returns alert events for the overlay.
    pub fn apply(&mut self, msg: &Msg) -> SmallVec<[AlertEvent; 2]> {
        let mut out = SmallVec::new();
        match msg {
            Msg::Batch(b) => {
                self.latest = self.latest.max(b.at);
                for s in &b.samples {
                    let series = self
                        .series
                        .entry(s.id.clone())
                        .or_insert_with(|| Series::for_datum(&s.datum, &self.retention));
                    series.push(b.at, s.datum.clone(), &self.retention);
                }
                let per = self.sources.entry(b.source.0).or_insert_with(|| PerSource {
                    id: b.source,
                    generation: 0,
                    last_sample: None,
                    status: SourceStatus::starting(b.at),
                });
                per.generation += 1;
                per.last_sample = Some(b.at);
                per.status.last_sample = Some(b.at);
            }
            Msg::Control(ControlMsg::Status(id, st)) => {
                let per = self.sources.entry(id.0).or_insert_with(|| PerSource {
                    id: *id,
                    generation: 0,
                    last_sample: None,
                    status: st.clone(),
                });
                per.status = st.clone();
                if per.status.last_sample.is_none() {
                    per.status.last_sample = per.last_sample;
                }
            }
            Msg::Control(ControlMsg::Alert(ev)) => {
                self.latest = self.latest.max(ev.at);
                self.alerts.observe(ev);
                out.push(ev.clone());
            }
            // Done / Reload are consumed by the app before/around apply (§4.2).
            Msg::Control(_) | Msg::Input(_) | Msg::Heartbeat => {}
        }
        out
    }

    /// The most recent Ts the store has seen; resample windows end here.
    pub fn latest(&self) -> Ts {
        self.latest
    }

    pub fn last(&self, k: &Key<f64>) -> Option<(Ts, f64)> {
        match self.series.get(&k.id) {
            Some(Series::Scalar(ring)) => ring.back().copied(),
            _ => None,
        }
    }

    pub fn window<'a>(
        &'a self,
        k: &Key<f64>,
        span: Duration,
    ) -> impl Iterator<Item = (Ts, f64)> + 'a {
        let start = Ts(self.latest.0.saturating_sub(span.as_nanos() as u64));
        let ring = match self.series.get(&k.id) {
            Some(Series::Scalar(r)) => Some(r),
            _ => None,
        };
        ring.into_iter()
            .flat_map(|r| r.iter())
            .copied()
            .filter(move |(t, _)| *t >= start)
    }

    /// The single history API (§4.2): bucket the key's window into `out`.
    pub fn resample(
        &self,
        k: &Key<f64>,
        span: Duration,
        buckets: usize,
        agg: Agg,
        out: &mut Vec<Option<f64>>,
    ) {
        resample(self.window(k, span), self.latest, span, buckets, agg, out);
    }

    pub fn vector(&self, k: &Key<Vec32>) -> Option<(Ts, &Vec32)> {
        match self.series.get(&k.id) {
            Some(Series::Vector {
                latest: Some((t, v)),
                ..
            }) => Some((*t, v)),
            _ => None,
        }
    }

    pub fn record<T: RecordValue>(&self, k: &Key<T>) -> Option<(Ts, &T)> {
        match self.series.get(&k.id) {
            Some(Series::Record(Some((t, r)))) => r.as_any().downcast_ref::<T>().map(|v| (*t, v)),
            _ => None,
        }
    }

    /// All labels present for a key name, in deterministic (BTreeMap) order.
    pub fn labels<'a>(&'a self, name: &'static str) -> impl Iterator<Item = &'a crate::key::Label> {
        let from = MetricId {
            name,
            label: crate::key::Label::None,
        };
        self.series
            .range(from..)
            .take_while(move |(id, _)| id.name == name)
            .map(|(id, _)| &id.label)
    }

    pub fn status(&self, id: SourceId) -> &SourceStatus {
        self.sources
            .get(id.0)
            .map(|p| &p.status)
            .unwrap_or(&DEFAULT_STATUS)
    }

    pub fn generation(&self, id: SourceId) -> u64 {
        self.sources.get(id.0).map(|p| p.generation).unwrap_or(0)
    }

    pub fn last_sample(&self, id: SourceId) -> Option<Ts> {
        self.sources.get(id.0).and_then(|p| p.last_sample)
    }

    pub fn sources(&self) -> impl Iterator<Item = SourceOverview<'_>> {
        self.sources.values().map(|p| SourceOverview {
            id: p.id,
            status: &p.status,
            generation: p.generation,
            last_sample: p.last_sample,
        })
    }

    pub fn alerts(&self) -> &AlertLog {
        &self.alerts
    }
}
