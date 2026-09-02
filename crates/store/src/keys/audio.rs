//! Audio keys (§8, brief arc 5 seam 1): the sink monitor's spectrum as 64
//! display-resolution bands per channel, the scope, the levels, the sink and
//! the silence state. The source publishes instantaneous heights; ballistics
//! are the component's.

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::journal::JournalError;
use crate::key::{DatumKind, Key, KeyMeta, RecordValue, Unit, Vec32};
use crate::source::SourceId;
use crate::ts::Ts;

pub const SOURCE: SourceId = SourceId("audio");

/// Bands per channel: display resolution, not spectral (the component
/// resamples to its bar count).
pub const BANDS: usize = 64;
/// Scope samples published per tick.
pub const SCOPE_LEN: usize = 512;

/// `audio.bands{ch}` — 64 heights in 0..1, channels `0` (left) and `1`.
pub const BANDS_KEY: Key<Vec32> = Key::new("audio.bands");
/// `audio.scope` — the latest 512 mono samples in −1..1.
pub const SCOPE: Key<Vec32> = Key::new("audio.scope");
/// `audio.rms_db{ch}` — RMS over 300 ms in dBFS, clamped at −100.
pub const RMS_DB: Key<f64> = Key::new("audio.rms_db");
/// `audio.peak_db{ch}` — sample peak with a 1.5 s hold, dBFS.
pub const PEAK_DB: Key<f64> = Key::new("audio.peak_db");
/// `audio.lufs_m` / `audio.lufs_s` — EBU R128 momentary / short-term
/// loudness (feature `audio-lufs`; absent otherwise).
pub const LUFS_M: Key<f64> = Key::new("audio.lufs_m");
pub const LUFS_S: Key<f64> = Key::new("audio.lufs_s");
/// `audio.dsp_ms` — the last DSP pass's wall ms (P16's evidence).
pub const DSP_MS: Key<f64> = Key::new("audio.dsp_ms");
pub const SINK: Key<AudioSink> = Key::new("audio.sink");
pub const SINKS: Key<AudioSinks> = Key::new("audio.sinks");
pub const LEVEL: Key<AudioLevel> = Key::new("audio.level");

/// The dBFS floor the source and the synth agree on (below it, silence).
pub const FLOOR_DB: f64 = -100.0;

/// The first `Control::Domain` payload (brief arc 5 seam 4): the picker's
/// choice — a `node.name` or an `object.serial` as text. The component boxes
/// it, the source downcasts it; it lives here because components never
/// depend on sources.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SetSink(pub String);

/// The sink being captured (once per generation and on every sink change).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct AudioSink {
    /// PipeWire `node.name`.
    pub name: String,
    /// `node.description` — what a person calls it.
    pub description: String,
    /// `object.serial` — the only stable target id (never the node id).
    pub serial: u32,
    /// `running | suspended | idle`.
    pub state: String,
    pub is_default: bool,
    pub rate: u32,
    pub channels: u8,
}

/// Every sink `pw-dump` lists, while enumeration is on (the picker).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct AudioSinks {
    pub sinks: Vec<AudioSink>,
}

/// The silence rule's state: `silent` after 250 ms without a frame or
/// 500 ms below the floor; the DSP then publishes at 2 Hz.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct AudioLevel {
    pub silent: bool,
    pub since: Ts,
}

fn decode<T: for<'de> Deserialize<'de> + RecordValue>(
    v: serde_json::Value,
) -> Result<Arc<dyn RecordValue>, JournalError> {
    serde_json::from_value::<T>(v)
        .map(|t| Arc::new(t) as Arc<dyn RecordValue>)
        .map_err(|e| JournalError(e.to_string()))
}

fn decode_sink(v: serde_json::Value) -> Result<Arc<dyn RecordValue>, JournalError> {
    decode::<AudioSink>(v)
}

fn decode_sinks(v: serde_json::Value) -> Result<Arc<dyn RecordValue>, JournalError> {
    decode::<AudioSinks>(v)
}

fn decode_level(v: serde_json::Value) -> Result<Arc<dyn RecordValue>, JournalError> {
    decode::<AudioLevel>(v)
}

macro_rules! meta {
    ($name:expr, $unit:ident, $kind:ident, $doc:expr) => {
        KeyMeta {
            name: $name,
            unit: Unit::$unit,
            kind: DatumKind::$kind,
            source: SOURCE,
            doc: $doc,
            decode: None,
        }
    };
}

pub static METAS: &[KeyMeta] = &[
    meta!(
        "audio.bands",
        Ratio,
        Vector,
        "64 log-spaced band heights 0..1 per channel {0|1} (display resolution, not spectral); the component resamples"
    ),
    meta!(
        "audio.scope",
        Ratio,
        Vector,
        "the latest 512 mono samples in −1..1 (the oscilloscope)"
    ),
    meta!(
        "audio.rms_db",
        Ratio,
        Scalar,
        "RMS over 300 ms per channel {ch}, dBFS (−100 = silence)"
    ),
    meta!(
        "audio.peak_db",
        Ratio,
        Scalar,
        "sample peak per channel {ch} with a 1.5 s hold, dBFS"
    ),
    meta!(
        "audio.lufs_m",
        Ratio,
        Scalar,
        "EBU R128 momentary loudness, LUFS (feature audio-lufs)"
    ),
    meta!(
        "audio.lufs_s",
        Ratio,
        Scalar,
        "EBU R128 short-term loudness, LUFS (feature audio-lufs)"
    ),
    meta!(
        "audio.dsp_ms",
        Milliseconds,
        Scalar,
        "wall ms of the last DSP pass (P16 evidence)"
    ),
    KeyMeta {
        name: "audio.sink",
        unit: Unit::None,
        kind: DatumKind::Record,
        source: SOURCE,
        doc: "the captured sink: node.name, description, object.serial, state, default flag, rate, channels; once per generation and on change",
        decode: Some(decode_sink),
    },
    KeyMeta {
        name: "audio.sinks",
        unit: Unit::None,
        kind: DatumKind::Record,
        source: SOURCE,
        doc: "every Audio/Sink pw-dump lists, while the picker enumerates",
        decode: Some(decode_sinks),
    },
    KeyMeta {
        name: "audio.level",
        unit: Unit::None,
        kind: DatumKind::Record,
        source: SOURCE,
        doc: "the silence rule's state (silent, since); the DSP publishes at 2 Hz while silent",
        decode: Some(decode_level),
    },
];
