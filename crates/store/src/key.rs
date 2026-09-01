//! Typed metric identity and the catalogue (§4.1): every metric is a `Key<T>`
//! constant; sources write it, components read it, `gridwatch keys` documents it.

use std::any::Any;
use std::fmt;
use std::marker::PhantomData;
use std::sync::Arc;

use serde::Serialize;

use crate::journal::JournalError;
use crate::source::SourceId;

/// Display-resolution float vector (audio bands, power traces).
pub type Vec32 = Arc<[f32]>;

/// A metric's label: nothing, a small index (core, pin, device), or a name
/// (interface, `chip:label`). Ordering puts `None < Index < Name`, which the
/// store's `labels()` iteration relies on.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Label {
    None,
    Index(u16),
    Name(Arc<str>),
}

impl fmt::Display for Label {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Label::None => Ok(()),
            Label::Index(i) => write!(f, "{{{i}}}"),
            Label::Name(s) => write!(f, "{{{s}}}"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct MetricId {
    pub name: &'static str,
    pub label: Label,
}

impl fmt::Display for MetricId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}{}", self.name, self.label)
    }
}

/// A typed handle to a metric. `T` is `f64`, `Vec32`, or a Record type.
pub struct Key<T> {
    pub id: MetricId,
    _t: PhantomData<fn() -> T>,
}

impl<T> Clone for Key<T> {
    fn clone(&self) -> Self {
        Key {
            id: self.id.clone(),
            _t: PhantomData,
        }
    }
}

impl<T> fmt::Debug for Key<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Key({}{})", self.id.name, self.id.label)
    }
}

impl<T> Key<T> {
    pub const fn new(name: &'static str) -> Self {
        Key {
            id: MetricId {
                name,
                label: Label::None,
            },
            _t: PhantomData,
        }
    }

    pub fn idx(&self, i: u16) -> Self {
        Key {
            id: MetricId {
                name: self.id.name,
                label: Label::Index(i),
            },
            _t: PhantomData,
        }
    }

    pub fn named(&self, s: &Arc<str>) -> Self {
        Key {
            id: MetricId {
                name: self.id.name,
                label: Label::Name(s.clone()),
            },
            _t: PhantomData,
        }
    }
}

/// Every Record type implements this (blanket impl over `Serialize` types), so
/// the journal can round-trip records and the catalogue can decode them (§4.1).
pub trait RecordValue: Any + Send + Sync + fmt::Debug {
    fn as_any(&self) -> &dyn Any;
    fn to_json(&self) -> serde_json::Value;
}

impl<T> RecordValue for T
where
    T: Any + Send + Sync + fmt::Debug + Serialize,
{
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }
}

#[derive(Clone, Debug)]
pub enum Datum {
    Scalar(f64),
    Vector(Vec32),
    Record(Arc<dyn RecordValue>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DatumKind {
    Scalar,
    Vector,
    Record,
}

/// Unit of a metric, for axis labels and `gridwatch keys`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unit {
    Percent,
    Bytes,
    BytesPerSec,
    Celsius,
    Watts,
    Amps,
    Volts,
    Megahertz,
    Count,
    Seconds,
    Ratio,
    Text,
    None,
}

/// Revives a Record from its journal JSON (§4.1/§4.5).
pub type DecodeFn = fn(serde_json::Value) -> Result<Arc<dyn RecordValue>, JournalError>;

/// One catalogue row per key name (§4.1). `decode` revives a Record from its
/// journal JSON; scalar/vector rows leave it `None`.
pub struct KeyMeta {
    pub name: &'static str,
    pub unit: Unit,
    pub kind: DatumKind,
    pub source: SourceId,
    pub doc: &'static str,
    pub decode: Option<DecodeFn>,
}

/// The whole vocabulary: one slice per `keys/<domain>.rs`.
pub static CATALOGUE: &[&[KeyMeta]] = &[crate::keys::sys::METAS, crate::keys::cpu::METAS];

/// Intern a journal/config name onto the static catalogue; unknown names are
/// skipped by callers with one warning, never leaked.
pub fn lookup(name: &str) -> Option<&'static KeyMeta> {
    CATALOGUE
        .iter()
        .flat_map(|d| d.iter())
        .find(|m| m.name == name)
}
