//! gridwatch-store: the headless core (ARCHITECTURE §4.1–§4.5).
//!
//! Typed metric catalogue, the single-writer store, the three channels, the
//! source contract with demand-driven cadence and zero-poll sleeps, alerts,
//! and the seeded demo synthesis shared by `--demo` and the ui testkit.
//! No TUI crates, no crossterm, no system dependencies.

#![forbid(unsafe_code)]

pub mod alert;
pub mod capability;
pub mod demo;
pub mod input;
pub mod journal;
pub mod key;
pub mod keys;
pub mod msg;
pub mod ring;
pub mod series;
pub mod source;
pub mod store;
pub mod ts;

pub use alert::{ActiveAlert, AlertEvent, AlertId, AlertLog, Severity, Transition};
pub use capability::{ALL_CAPABILITIES, CapSet, Capability};
pub use input::{InputEvent, KeyCode, KeyEvent, Mods, MouseButton, MouseEvent, MouseKind};
pub use journal::JournalError;
pub use key::{
    CATALOGUE, Datum, DatumKind, Key, KeyMeta, Label, MetricId, RecordValue, Unit, Vec32, lookup,
};
pub use msg::{
    ActionId, Batch, Channels, ControlMsg, DATA_BOUND, Inbox, Msg, Reload, ReloadKind, Sample,
    channels,
};
pub use series::{Agg, Retention};
pub use source::{
    Cadence, Control, Demand, Detail, Level, Sampler, Source, SourceCtx, SourceDef, SourceError,
    SourceId, SourceInfo, SourceState, SourceStatus,
};
pub use store::{SourceOverview, Store};
pub use ts::{Clock, Ts};
