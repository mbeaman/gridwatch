//! The journal (§4.5, D47): JSON Lines record and replay.
//!
//! One object per line. The first line is a header; every other line is
//! `{"t": <ns since the run epoch>, "b" | "st" | "al" | "in": …}`:
//!
//! ```text
//! {"v":1,"wall_epoch":1756700000,"host":"torch","size":[250,70],"sources":["cpu"]}
//! {"t":1500000000,"b":{"src":"cpu","s":[["cpu.core_pct{3}",12.5],["cpu.breakdown{3}",{…}]]}}
//! {"t":1500000000,"st":{"src":"cpu","state":"Ok","reason":null,"hint":null,"dropped":0,"restarts":0}}
//! {"t":1500000000,"al":{…AlertEvent…}}
//! {"t":1500000000,"in":{…InputEvent…}}
//! ```
//!
//! A sample is `[name{label}, datum]` and the datum's JSON *type* decides its
//! kind on read: number → `Scalar`, array → `Vector`, object → `Record` revived
//! through the catalogue's `decode`. Names are interned onto the static
//! catalogue by `lookup`; an unknown name is skipped with one warning per name
//! per file (`Decoder::unknown`). `tables = false` (the default) omits samples
//! whose key is `proc.table` or `gpu.procs`.
//!
//! Three actors: `Recorder` (a bounded-channel tee from the frame loop to a
//! writer thread — may drop, counted, never stalls a frame), `Replay` (a whole
//! file in memory for tests and `shot --replay`) and `JournalSource` (a
//! `Source` that drives `Clock::Virtual` and re-emits every line through the
//! normal channels, so nothing downstream can tell replay from live).

use std::collections::BTreeSet;
use std::fmt;
use std::io::{BufRead, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{RecvTimeoutError, SyncSender, sync_channel};
use std::thread::JoinHandle;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::alert::AlertEvent;
use crate::input::InputEvent;
use crate::key::{Datum, DatumKind, MetricId, Vec32, intern_source, lookup, parse_name};
use crate::msg::{Batch, ControlMsg, Msg, Sample};
use crate::source::{Cadence, Source, SourceCtx, SourceId, SourceInfo, SourceState, SourceStatus};
use crate::store::Store;
use crate::ts::Ts;

/// The replay source's own id (§4.3: "replay is one `JournalSource`").
pub const JOURNAL: SourceId = SourceId("journal");

/// The line format version this build writes and reads.
pub const VERSION: u32 = 1;

/// Keys omitted when `tables` is off: the two process tables (§4.5).
pub const TABLE_KEYS: &[&str] = &["proc.table", "gpu.procs"];

#[derive(Debug)]
pub struct JournalError(pub String);

impl fmt::Display for JournalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "journal: {}", self.0)
    }
}

impl std::error::Error for JournalError {}

impl From<serde_json::Error> for JournalError {
    fn from(e: serde_json::Error) -> JournalError {
        JournalError(e.to_string())
    }
}

impl From<std::io::Error> for JournalError {
    fn from(e: std::io::Error) -> JournalError {
        JournalError(e.to_string())
    }
}

/// The first line of every journal.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Header {
    pub v: u32,
    /// Unix seconds when the recording started; `t` values are relative to it.
    pub wall_epoch: u64,
    pub host: String,
    /// Terminal size `[w, h]` at the start of the recording.
    pub size: [u16; 2],
    /// The sources that were running (names, not interned — a journal may name
    /// a source this build does not have).
    pub sources: Vec<String>,
}

impl Header {
    pub fn new(host: impl Into<String>, size: (u16, u16), sources: Vec<String>) -> Header {
        Header {
            v: VERSION,
            wall_epoch: std::time::SystemTime::now()
                .duration_since(std::time::SystemTime::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0),
            host: host.into(),
            size: [size.0, size.1],
            sources,
        }
    }

    pub fn encode(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }
}

/// `/proc/sys/kernel/hostname`, or `?` where there is no procfs.
pub fn hostname() -> String {
    std::fs::read_to_string("/proc/sys/kernel/hostname")
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|_| "?".into())
}

/// Is this file a journal this build can read? The first non-empty line must
/// be a header of the current version. Cheap, and meant to run *before* the
/// terminal is taken over, so the answer reaches the user's stderr.
pub fn check_header(path: &Path) -> Result<Header, JournalError> {
    let file = std::fs::File::open(path)?;
    let mut reader = std::io::BufReader::new(file);
    let mut line = String::new();
    loop {
        line.clear();
        if reader.read_line(&mut line)? == 0 {
            return Err(JournalError("empty file — not a gridwatch journal".into()));
        }
        if line.trim().is_empty() {
            continue;
        }
        break;
    }
    match Decoder::default().decode(&line) {
        Ok(Entry::Header(h)) => Ok(h),
        Ok(Entry::Msg(..)) => Err(JournalError(
            "first line is not a header — not a gridwatch journal".into(),
        )),
        Err(e) => Err(JournalError(format!("not a gridwatch journal ({e})"))),
    }
}

/// One decoded line.
#[derive(Clone, Debug)]
pub enum Entry {
    Header(Header),
    Msg(Ts, Msg),
}

// ───────────────────────────── encode ─────────────────────────────

fn datum_value(d: &Datum) -> Value {
    match d {
        // `json!` writes a non-finite float as `null`; the decoder skips it.
        Datum::Scalar(v) => json!(v),
        Datum::Vector(v) => json!(v.as_ref()),
        Datum::Record(r) => r.to_json(),
    }
}

fn status_value(id: SourceId, s: &SourceStatus) -> Value {
    json!({
        "src": id.0,
        "state": s.state,
        "reason": s.reason.as_deref(),
        "hint": s.hint.as_deref(),
        "dropped": s.dropped,
        "restarts": s.restarts,
    })
}

/// The line for a message at `t`, or `None` for messages the journal does not
/// carry (`Heartbeat`, `Done`, `Reload`). With `tables` off the process-table
/// samples are omitted; a batch left empty by that is still written, so the
/// source's generation advances on replay exactly as it did live.
pub fn encode_at(t: Ts, msg: &Msg, tables: bool) -> Option<String> {
    // The envelope is written by hand so `t` comes first on every line, as
    // §4.5 shows it (`serde_json` sorts object keys); the bodies are plain
    // JSON values and readers never depend on key order.
    let (tag, body) = match msg {
        Msg::Batch(b) => {
            let samples: Vec<String> = b
                .samples
                .iter()
                .filter(|s| tables || !TABLE_KEYS.contains(&s.id.name))
                .map(|s| {
                    format!(
                        "[{},{}]",
                        Value::String(s.id.to_string()),
                        datum_value(&s.datum)
                    )
                })
                .collect();
            (
                "b",
                format!(
                    "{{\"src\":{},\"s\":[{}]}}",
                    Value::String(b.source.0.to_string()),
                    samples.join(",")
                ),
            )
        }
        Msg::Control(ControlMsg::Status(id, s)) => ("st", status_value(*id, s).to_string()),
        Msg::Control(ControlMsg::Alert(a)) => ("al", json!(a).to_string()),
        Msg::Control(_) | Msg::Heartbeat => return None,
        Msg::Input(i) => ("in", json!(i).to_string()),
    };
    Some(format!("{{\"t\":{},\"{tag}\":{body}}}", t.0))
}

/// `encode_at` with the message's own timestamp: a batch's `at`, an alert's
/// `at`, a status' `since`; an input carries none and gets `Ts::ZERO` — the
/// recorder stamps the clock instead.
pub fn encode(msg: &Msg) -> Option<String> {
    let t = match msg {
        Msg::Batch(b) => b.at,
        Msg::Control(ControlMsg::Status(_, s)) => s.since,
        Msg::Control(ControlMsg::Alert(a)) => a.at,
        _ => Ts::ZERO,
    };
    encode_at(t, msg, true)
}

// ───────────────────────────── decode ─────────────────────────────

/// Decodes lines and remembers which unknown names it has already warned
/// about, so a 60 s journal with a key this build lacks logs once, not 40×.
#[derive(Debug, Default)]
pub struct Decoder {
    /// Names with no catalogue row — one entry per name.
    pub unknown: BTreeSet<String>,
    /// Samples of known keys this build could not revive.
    pub undecodable: u64,
    pub lines: u64,
}

enum Skip {
    Unknown,
    Undecodable,
}

impl Decoder {
    pub fn decode(&mut self, line: &str) -> Result<Entry, JournalError> {
        self.lines += 1;
        let v: Value = serde_json::from_str(line)?;
        let Some(obj) = v.as_object() else {
            return Err(JournalError("line is not an object".into()));
        };
        if obj.contains_key("v") {
            let h: Header = serde_json::from_value(v)?;
            if h.v != VERSION {
                return Err(JournalError(format!(
                    "journal version {} (this build reads {VERSION})",
                    h.v
                )));
            }
            return Ok(Entry::Header(h));
        }
        let t = obj
            .get("t")
            .and_then(Value::as_u64)
            .ok_or_else(|| JournalError("line has no `t`".into()))?;
        let t = Ts(t);
        if let Some(b) = obj.get("b") {
            return Ok(Entry::Msg(t, Msg::Batch(self.batch(t, b)?)));
        }
        if let Some(s) = obj.get("st") {
            let src = intern(s.get("src"))?;
            let state: SourceState = serde_json::from_value(
                s.get("state")
                    .cloned()
                    .ok_or_else(|| JournalError("status has no `state`".into()))?,
            )?;
            let text = |k: &str| s.get(k).and_then(Value::as_str).map(Arc::<str>::from);
            let st = SourceStatus {
                state,
                reason: text("reason"),
                hint: text("hint"),
                since: t,
                last_sample: None,
                dropped: s.get("dropped").and_then(Value::as_u64).unwrap_or(0),
                restarts: s.get("restarts").and_then(Value::as_u64).unwrap_or(0) as u32,
            };
            return Ok(Entry::Msg(t, Msg::Control(ControlMsg::Status(src, st))));
        }
        if let Some(a) = obj.get("al") {
            let ev: AlertEvent = serde_json::from_value(a.clone())?;
            return Ok(Entry::Msg(t, Msg::Control(ControlMsg::Alert(ev))));
        }
        if let Some(i) = obj.get("in") {
            let ev: InputEvent = serde_json::from_value(i.clone())?;
            return Ok(Entry::Msg(t, Msg::Input(ev)));
        }
        Err(JournalError("line has none of b / st / al / in".into()))
    }

    fn batch(&mut self, t: Ts, b: &Value) -> Result<Batch, JournalError> {
        let src = intern(b.get("src"))?;
        let list = b
            .get("s")
            .and_then(Value::as_array)
            .ok_or_else(|| JournalError("batch has no `s` array".into()))?;
        let mut samples = Vec::with_capacity(list.len());
        for item in list {
            let (Some(name), Some(datum)) = (item.get(0).and_then(Value::as_str), item.get(1))
            else {
                return Err(JournalError("sample is not [name, datum]".into()));
            };
            match self.sample(name, datum) {
                Ok(s) => samples.push(s),
                Err(Skip::Unknown) => {
                    self.unknown.insert(name.to_string());
                }
                Err(Skip::Undecodable) => self.undecodable += 1,
            }
        }
        Ok(Batch {
            source: src,
            at: t,
            samples,
        })
    }

    /// `Err(Unknown)` = no catalogue row for the name (warned once per file);
    /// `Err(Undecodable)` = a known key whose datum this build cannot revive
    /// (kind mismatch, a record the decoder rejects) — skipped silently, it is
    /// not "unknown". `null` is what `serde_json` writes for a non-finite
    /// float, so it revives as NaN rather than vanishing (§4.4: NaN never
    /// raises or clears a rule). Type-driven per §4.5.
    fn sample(&self, name: &str, datum: &Value) -> Result<Sample, Skip> {
        let (base, label) = parse_name(name);
        let meta = lookup(base).ok_or(Skip::Unknown)?;
        let id = MetricId {
            name: meta.name,
            label,
        };
        let f = |v: &Value| -> Option<f64> {
            match v {
                Value::Number(n) => n.as_f64(),
                Value::Null => Some(f64::NAN),
                _ => None,
            }
        };
        let datum = match (datum, meta.kind) {
            (Value::Number(_) | Value::Null, DatumKind::Scalar) => {
                Datum::Scalar(f(datum).ok_or(Skip::Undecodable)?)
            }
            (Value::Array(items), DatumKind::Vector) => {
                let v: Vec<f32> = items
                    .iter()
                    .map(|x| f(x).map(|v| v as f32))
                    .collect::<Option<Vec<f32>>>()
                    .ok_or(Skip::Undecodable)?;
                Datum::Vector(Vec32::from(v))
            }
            (Value::Object(_), DatumKind::Record) => {
                let decode = meta.decode.ok_or(Skip::Undecodable)?;
                Datum::Record(decode(datum.clone()).map_err(|_| Skip::Undecodable)?)
            }
            _ => return Err(Skip::Undecodable),
        };
        Ok(Sample { id, datum })
    }
}

fn intern(v: Option<&Value>) -> Result<SourceId, JournalError> {
    let name = v
        .and_then(Value::as_str)
        .ok_or_else(|| JournalError("missing `src`".into()))?;
    intern_source(name).ok_or_else(|| JournalError(format!("unknown source `{name}`")))
}

/// One line, stateless (tests and one-offs); `Decoder` for whole files.
pub fn decode(line: &str) -> Result<Entry, JournalError> {
    Decoder::default().decode(line)
}

// ───────────────────────────── recorder ─────────────────────────────

#[derive(Clone, Copy, Debug, Default)]
pub struct RecordOpts {
    /// Write `proc.table` / `gpu.procs` samples (`--tables on`; default off).
    pub tables: bool,
    /// Write input events (`--record-input`).
    pub input: bool,
}

/// Bound of the tee channel: a frame's worth of messages many times over; a
/// stalled disk drops beyond it (counted) rather than stalling a frame.
const TEE_BOUND: usize = 4096;

/// The tee (§4.5, D47 seam 3). `record` clones the message onto a bounded
/// channel; a `gw-record` thread encodes and writes it through a `BufWriter`
/// flushed every second and on `finish`.
pub struct Recorder {
    tx: Option<SyncSender<(Ts, Msg)>>,
    opts: RecordOpts,
    enabled: Arc<AtomicBool>,
    dropped: Arc<AtomicU64>,
    written: Arc<AtomicU64>,
    last_t: AtomicU64,
    dead: AtomicBool,
    path: PathBuf,
    join: Option<JoinHandle<std::io::Result<()>>>,
}

/// What `finish` reports: lines written and lines dropped.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Recorded {
    pub written: u64,
    pub dropped: u64,
}

impl Recorder {
    pub fn start(path: &Path, header: &Header, opts: RecordOpts) -> std::io::Result<Recorder> {
        let file = std::fs::File::create(path)?;
        let mut out = BufWriter::new(file);
        writeln!(out, "{}", header.encode())?;
        let (tx, rx) = sync_channel::<(Ts, Msg)>(TEE_BOUND);
        let written = Arc::new(AtomicU64::new(0));
        let t_written = written.clone();
        let join = std::thread::Builder::new().name("gw-record".into()).spawn(
            move || -> std::io::Result<()> {
                loop {
                    match rx.recv_timeout(Duration::from_secs(1)) {
                        Ok((t, msg)) => {
                            if matches!(msg, Msg::Input(_)) && !opts.input {
                                continue;
                            }
                            if let Some(line) = encode_at(t, &msg, opts.tables) {
                                writeln!(out, "{line}")?;
                                t_written.fetch_add(1, Ordering::Relaxed);
                            }
                        }
                        Err(RecvTimeoutError::Timeout) => out.flush()?,
                        Err(RecvTimeoutError::Disconnected) => {
                            out.flush()?;
                            return Ok(());
                        }
                    }
                }
            },
        )?;
        Ok(Recorder {
            tx: Some(tx),
            opts,
            enabled: Arc::new(AtomicBool::new(true)),
            dropped: Arc::new(AtomicU64::new(0)),
            written,
            last_t: AtomicU64::new(0),
            dead: AtomicBool::new(false),
            path: path.to_path_buf(),
            join: Some(join),
        })
    }

    /// Tee one message at `t`. Never blocks: a full channel drops and counts.
    /// Inputs are filtered *here* when `--record-input` is off, so a mouse
    /// storm cannot spend the bound on messages the writer would discard.
    /// `t` is clamped so the file is monotone in `t` whatever order the loop
    /// drained things in (a batch's `at` predates the status drained before
    /// it): §4.5's readers rely on order.
    pub fn record(&self, t: Ts, msg: &Msg) {
        if !self.enabled.load(Ordering::Relaxed) {
            return;
        }
        match msg {
            Msg::Heartbeat => return,
            Msg::Input(_) if !self.opts.input => return,
            _ => {}
        }
        let last = self.last_t.fetch_max(t.0, Ordering::AcqRel);
        let t = Ts(t.0.max(last));
        if let Some(tx) = &self.tx {
            match tx.try_send((t, msg.clone())) {
                Ok(()) => {}
                Err(std::sync::mpsc::TrySendError::Full(_)) => {
                    self.dropped.fetch_add(1, Ordering::Relaxed);
                }
                Err(std::sync::mpsc::TrySendError::Disconnected(_)) => {
                    // The writer died (an I/O error): every later message is
                    // lost, which the app must say, not count as a drop.
                    self.dead.store(true, Ordering::Release);
                }
            }
        }
    }

    /// True once the writer thread has stopped on an I/O error; `finish`
    /// surfaces the error itself.
    pub fn dead(&self) -> bool {
        self.dead.load(Ordering::Acquire)
    }

    pub fn set_enabled(&self, on: bool) {
        self.enabled.store(on, Ordering::Relaxed);
    }

    pub fn enabled(&self) -> bool {
        self.enabled.load(Ordering::Relaxed)
    }

    pub fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }

    /// Lines the writer thread has written so far.
    pub fn written(&self) -> u64 {
        self.written.load(Ordering::Relaxed)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Close the channel, wait for the writer, surface its I/O error. The
    /// counts are read *after* the join, so they include the queued tail.
    pub fn finish(mut self) -> std::io::Result<Recorded> {
        self.tx.take();
        let r = match self.join.take() {
            Some(j) => j
                .join()
                .unwrap_or_else(|_| Err(std::io::Error::other("the recorder thread panicked"))),
            None => Ok(()),
        };
        r.map(|()| Recorded {
            written: self.written(),
            dropped: self.dropped(),
        })
    }
}

impl Drop for Recorder {
    fn drop(&mut self) {
        self.tx.take();
        if let Some(j) = self.join.take() {
            let _ = j.join();
        }
    }
}

// ───────────────────────────── replay (in memory) ─────────────────────────────

/// A whole journal in memory, for tests and `shot --replay --at` (§4.5).
#[derive(Debug, Default)]
pub struct Replay {
    pub header: Option<Header>,
    pub entries: Vec<(Ts, Msg)>,
    /// Sample names this build skipped (one per name).
    pub unknown: BTreeSet<String>,
    /// Lines that did not parse (a truncated tail, a corrupt line).
    pub malformed: u64,
    cursor: usize,
}

impl Replay {
    pub fn load(path: &Path) -> Result<Replay, JournalError> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| JournalError(format!("{}: {e}", path.display())))?;
        Replay::parse(&text)
    }

    /// A malformed line is skipped and counted, as `JournalSource` skips it:
    /// a `SIGKILL`ed recorder leaves a truncated last line and the file must
    /// still replay. Only an unreadable *header* (a newer version) is fatal.
    /// Entries are stably sorted by `t`: the recorder writes them monotone,
    /// but a file from an older build may interleave a batch's earlier `at`
    /// after a status stamped at drain time, and `apply_until` needs order.
    pub fn parse(text: &str) -> Result<Replay, JournalError> {
        let mut dec = Decoder::default();
        let mut out = Replay::default();
        for (i, line) in text.lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            match dec.decode(line) {
                Ok(Entry::Header(h)) => out.header = Some(h),
                Ok(Entry::Msg(t, m)) => out.entries.push((t, m)),
                Err(e) if i == 0 => {
                    return Err(JournalError(format!("line 1: {e}")));
                }
                Err(_) => out.malformed += 1,
            }
        }
        out.entries.sort_by_key(|(t, _)| *t);
        out.unknown = dec.unknown;
        Ok(out)
    }

    /// The last timestamp in the file.
    pub fn end(&self) -> Ts {
        self.entries.last().map(|(t, _)| *t).unwrap_or(Ts::ZERO)
    }

    /// Apply every remaining batch/control line to the store; inputs are not
    /// applied (a store has no input). Returns the number applied.
    pub fn apply_all(&mut self, store: &mut Store) -> usize {
        self.apply_until(Ts(u64::MAX), store)
    }

    /// Apply the remaining lines with `t <= ts`, in order. Idempotent per
    /// entry: a second call continues where the first stopped.
    pub fn apply_until(&mut self, ts: Ts, store: &mut Store) -> usize {
        let mut n = 0;
        while let Some((t, msg)) = self.entries.get(self.cursor) {
            if *t > ts {
                break;
            }
            if !matches!(msg, Msg::Input(_)) {
                store.apply(msg);
            }
            self.cursor += 1;
            n += 1;
        }
        n
    }

    pub fn rewind(&mut self) {
        self.cursor = 0;
    }
}

// ───────────────────────────── replay (as a source) ─────────────────────────────

/// Replay as a `Source` (§4.3, D47 seam 2): owns the file, advances the shared
/// `Clock::Virtual` to each line's `t` (sleeping `(t − prev) / speed` of wall
/// time between lines; `speed = 0` means as fast as possible) and re-emits the
/// line through the normal channels via `SourceCtx::inject`. A replay run
/// registers only this source; the registry's real sources are not started.
pub struct JournalSource {
    path: PathBuf,
    speed: f64,
}

impl JournalSource {
    pub fn new(path: impl Into<PathBuf>, speed: f64) -> JournalSource {
        JournalSource {
            path: path.into(),
            speed: if speed.is_finite() && speed >= 0.0 {
                speed
            } else {
                1.0
            },
        }
    }

    pub fn info_static() -> SourceInfo {
        SourceInfo {
            id: JOURNAL,
            produces: &["*"],
            cadence: Cadence {
                hidden: Some(Duration::from_secs(1)),
                visible: Duration::from_secs(1),
                focused: Duration::from_secs(1),
                // Demand never gates a replay: the file is the schedule.
                always_on: true,
            },
            requires: &[],
        }
    }

    fn status(cx: &SourceCtx, state: SourceState, reason: String) {
        cx.status(SourceStatus {
            state,
            reason: Some(Arc::from(reason.as_str())),
            hint: None,
            since: cx.clock.now(),
            last_sample: None,
            dropped: 0,
            restarts: cx.restarts,
        });
    }
}

impl Source for JournalSource {
    fn info(&self) -> SourceInfo {
        Self::info_static()
    }

    fn run(self: Box<Self>, cx: SourceCtx) {
        let file = match std::fs::File::open(&self.path) {
            Ok(f) => f,
            Err(e) => {
                Self::status(
                    &cx,
                    SourceState::Unavailable,
                    format!("{}: {e}", self.path.display()),
                );
                return;
            }
        };
        Self::status(
            &cx,
            SourceState::Ok,
            format!("replaying {}", self.path.display()),
        );
        let reader = std::io::BufReader::new(file);
        let mut dec = Decoder::default();
        let mut prev: Option<Ts> = None;
        for (i, line) in reader.lines().enumerate() {
            if cx.stopped() {
                return;
            }
            let Ok(line) = line else { break };
            if line.trim().is_empty() {
                continue;
            }
            let entry = match dec.decode(&line) {
                Ok(e) => e,
                Err(e) => {
                    tracing::warn!("journal line {}: {e}", i + 1);
                    continue;
                }
            };
            let Entry::Msg(t, msg) = entry else { continue };
            if let Some(p) = prev
                && self.speed > 0.0
                && t > p
            {
                let wait = t.since(p).div_f64(self.speed);
                // Interruptible: never sleep past a Stop.
                let mut slept = Duration::ZERO;
                while slept < wait {
                    if cx.stopped() {
                        return;
                    }
                    let step = (wait - slept).min(Duration::from_millis(100));
                    std::thread::sleep(step);
                    slept += step;
                }
            }
            prev = Some(t);
            // Never backwards: `Store::apply` keeps `latest` monotone, the
            // clock must too, whatever order an older file interleaved.
            cx.clock.set(t.max(cx.clock.now()));
            cx.inject(msg);
        }
        for name in &dec.unknown {
            tracing::warn!("journal: skipped unknown key `{name}`");
        }
        Self::status(&cx, SourceState::Stopped, "end of journal".into());
    }
}
