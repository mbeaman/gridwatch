//! Per-metric history (§4.2): `f64` keys keep bounded history; `Vec32` keys keep
//! latest plus a short ring; Record keys keep latest only.

use std::sync::Arc;
use std::time::Duration;

use crate::key::{Datum, RecordValue, Vec32};
use crate::ring::Ring;
use crate::ts::Ts;

#[derive(Clone, Copy, Debug)]
pub struct Retention {
    pub max_len: usize,
    pub max_age: Duration,
}

impl Default for Retention {
    fn default() -> Retention {
        Retention {
            max_len: 2400,
            max_age: Duration::from_secs(600),
        }
    }
}

/// Short history for vector series (a few seconds of audio bands / power trace).
const VECTOR_HISTORY: usize = 64;

#[derive(Debug)]
pub enum Series {
    Scalar(Ring<(Ts, f64)>),
    Vector {
        latest: Option<(Ts, Vec32)>,
        hist: Ring<(Ts, Vec32)>,
    },
    Record(Option<(Ts, Arc<dyn RecordValue>)>),
}

impl Series {
    pub fn for_datum(d: &Datum, retention: &Retention) -> Series {
        match d {
            Datum::Scalar(_) => Series::Scalar(Ring::new(retention.max_len)),
            Datum::Vector(_) => Series::Vector {
                latest: None,
                hist: Ring::new(VECTOR_HISTORY),
            },
            Datum::Record(_) => Series::Record(None),
        }
    }

    pub fn push(&mut self, at: Ts, d: Datum, retention: &Retention) {
        match (self, d) {
            (Series::Scalar(ring), Datum::Scalar(v)) => {
                ring.push((at, v));
                ring.prune_front(|(t, _)| at.since(*t) > retention.max_age);
            }
            (Series::Vector { latest, hist }, Datum::Vector(v)) => {
                *latest = Some((at, v.clone()));
                hist.push((at, v));
            }
            (Series::Record(slot), Datum::Record(r)) => *slot = Some((at, r)),
            // A key changing kind is a programming error; keep the store total
            // anyway by ignoring the mismatched sample (§4.1: kinds are static).
            _ => {}
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Agg {
    Last,
    Avg,
    Max,
    Min,
}

/// Bucket `points` (ascending by Ts) over `[end - span, end)` into `buckets`
/// slots written to `out` (`None` = no sample in the bucket). The single
/// history API (§4.2); callers own the buffer.
pub fn resample(
    points: impl Iterator<Item = (Ts, f64)>,
    end: Ts,
    span: Duration,
    buckets: usize,
    agg: Agg,
    out: &mut Vec<Option<f64>>,
) {
    out.clear();
    out.resize(buckets, None);
    if buckets == 0 || span.is_zero() {
        return;
    }
    let span_ns = span.as_nanos() as u64;
    let start = Ts(end.0.saturating_sub(span_ns));
    let bucket_ns = (span_ns / buckets as u64).max(1);
    let mut counts = vec![0u32; buckets];
    for (t, v) in points {
        if t < start || t >= end {
            continue;
        }
        let i = (((t.0 - start.0) / bucket_ns) as usize).min(buckets - 1);
        let slot = &mut out[i];
        match (agg, slot.as_mut()) {
            (_, None) => {
                *slot = Some(v);
                counts[i] = 1;
            }
            (Agg::Last, Some(s)) => *s = v,
            (Agg::Max, Some(s)) => *s = s.max(v),
            (Agg::Min, Some(s)) => *s = s.min(v),
            (Agg::Avg, Some(s)) => {
                counts[i] += 1;
                *s += (v - *s) / f64::from(counts[i]);
            }
        }
    }
}
