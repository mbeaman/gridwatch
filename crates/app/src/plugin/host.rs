//! Starting the configured plugins and keeping them talking (§4.7, arc 8b).
//!
//! `supervise.rs` knows how to run *one* plugin safely. This module is what
//! turns a `[[plugins]]` block into tiles on the grid:
//!
//! * **Startup spawns every plugin, then waits on them together**, so N
//!   plugins cost the longest `hello_ms` rather than the sum of them.
//! * **An accepted manifest becomes an ordinary `ComponentDef`** under the
//!   kind `<id>.<the kind the manifest declares>`. That is why
//!   `ComponentDef::build` is a closure: it captures the plugin it speaks for
//!   and the channel it speaks over. Nothing downstream — the edit-mode
//!   picker, `config check`, `build_instances` — needs a plugin special case.
//! * **A plugin's samples are an ordinary `Batch`** on the data channel under
//!   its own `SourceId`, so they reach the store, the recorder and `--replay`
//!   by the path every source already uses.
//! * **The host thread owns the children.** Writing to a child's stdin can
//!   block on a full pipe, and the render thread may never block; so the
//!   render thread only ever sends on a channel.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, SyncSender, channel};
use std::time::{Duration, Instant};

use gridwatch_store::{
    ALL_CAPABILITIES, Batch, CapSet, Capability, ControlMsg, Datum, Label, MetricId, Sample,
    Severity, SourceId, SourceState, SourceStatus, Ts,
};
use gridwatch_ui::component::{Chrome, Footprint, KeyHint, Size as TileSize, Tier};
use gridwatch_ui::{Component, ComponentDef, Manifest};

use super::proto::{self, Ask, Hello};
use super::supervise::{self, Plugin, PluginConfig, Report};
use super::tile::{PluginTile, Tell};
use crate::config::PluginSect;

/// The most distinct metric names one plugin may publish. Every name has to
/// become a `&'static str` to enter the store, so an unbounded set of names is
/// an unbounded leak; a plugin that wants more than this is not publishing
/// metrics, it is writing them.
pub const MAX_METRIC_NAMES: usize = 256;

/// What the render thread asks of the host thread.
pub enum Wish {
    /// A tile was built and wants the trees for its instance.
    Attach {
        plugin: usize,
        instance: String,
        to_tile: Sender<Tell>,
    },
    Render {
        plugin: usize,
        instance: String,
        tier: usize,
        inner: proto::Size,
        now: i64,
        focused: bool,
        captured: bool,
    },
    Key {
        plugin: usize,
        instance: String,
        key: String,
        mods: Vec<&'static str>,
    },
    /// Stop, and stop every child on the way out.
    ///
    /// Dropping the last sender would say the same thing, and cannot be
    /// relied on: every build closure holds a clone of it for the life of the
    /// registry, so the host thread's receiver never disconnects while the
    /// process is up. This is the only signal that always arrives.
    Stop,
}

/// What the host thread tells the shell.
pub enum Word {
    /// A tree or a status changed: the next frame is worth drawing. A plugin's
    /// view moves no source generation, so without this a component-only
    /// plugin's tile would update on the 1 Hz heartbeat and not before (§5).
    Landed,
    Toast(Severity, String),
    Page(usize),
    Zoom(bool),
}

/// What `start` gives the app: the handle the render thread keeps, the kinds
/// to register, the source ids to `ensure_source`, and what to say out loud.
pub struct Started {
    pub host: Option<Host>,
    pub defs: Vec<ComponentDef>,
    pub sources: Vec<SourceId>,
    pub warnings: Vec<String>,
}

/// The render thread's half: a sender to the host thread, a receiver for what
/// the shell must act on, and the render throttle. Dropping it stops every
/// plugin.
pub struct Host {
    wishes: Option<Sender<Wish>>,
    words: Receiver<Word>,
    /// kind → the plugin that backs it, so `draw_cell` can tell a plugin tile
    /// from a built-in without downcasting anything.
    kinds: BTreeMap<String, usize>,
    /// The `render_ms` floor per plugin.
    render_ms: Vec<u64>,
    /// instance → what was last asked for, and when.
    asked: BTreeMap<String, (AskShape, Instant)>,
    thread: Option<std::thread::JoinHandle<()>>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct AskShape {
    tier: usize,
    w: u16,
    h: u16,
    focused: bool,
    captured: bool,
}

impl Host {
    /// Is this kind one of ours? The shell asks before it asks for a render.
    pub fn owns(&self, kind: &str) -> bool {
        self.kinds.contains_key(kind)
    }

    pub fn kinds(&self) -> impl Iterator<Item = &str> {
        self.kinds.keys().map(|s| s.as_str())
    }

    /// Ask for a render if anything the plugin would draw differently changed,
    /// or if the floor has passed. Called from `draw_cell`, which is the only
    /// place that knows the tile's real rect.
    #[allow(clippy::too_many_arguments)]
    pub fn want_render(
        &mut self,
        kind: &str,
        instance: &str,
        tier: usize,
        inner: TileSize,
        focused: bool,
        captured: bool,
        now: Ts,
    ) {
        let Some(&plugin) = self.kinds.get(kind) else {
            return;
        };
        let shape = AskShape {
            tier,
            w: inner.w,
            h: inner.h,
            focused,
            captured,
        };
        let floor = Duration::from_millis(self.render_ms.get(plugin).copied().unwrap_or(1_000));
        let due = match self.asked.get(instance) {
            None => true,
            Some((last, at)) => *last != shape || at.elapsed() >= floor,
        };
        if !due {
            return;
        }
        self.asked
            .insert(instance.to_string(), (shape, Instant::now()));
        if let Some(w) = &self.wishes {
            let _ = w.send(Wish::Render {
                plugin,
                instance: instance.to_string(),
                tier,
                inner: proto::Size {
                    w: inner.w,
                    h: inner.h,
                },
                now: now.0 as i64,
                focused,
                captured,
            });
        }
    }

    /// Everything the host thread has said since the last call. Never blocks.
    pub fn drain(&self) -> Vec<Word> {
        self.words.try_iter().collect()
    }

    /// Wait for the next word, up to `for_`. `shot` uses it to give a plugin
    /// time to answer the render it was just asked for.
    pub fn next_word(&self, for_: Duration) -> Option<Word> {
        self.words.recv_timeout(for_).ok()
    }
}

impl Drop for Host {
    fn drop(&mut self) {
        if let Some(w) = self.wishes.take() {
            let _ = w.send(Wish::Stop);
        }
        if let Some(h) = self.thread.take() {
            let _ = h.join();
        }
    }
}

/// A capability's name on the wire: `I2cNvidia` is `i2c_nvidia`. Derived from
/// the enum rather than a hand table, so the two cannot drift.
pub fn capability_name(c: Capability) -> String {
    let mut out = String::new();
    for (i, ch) in format!("{c:?}").chars().enumerate() {
        if ch.is_ascii_uppercase() && i > 0 {
            out.push('_');
        }
        out.push(ch.to_ascii_lowercase());
    }
    out
}

pub fn capability_named(name: &str) -> Option<Capability> {
    ALL_CAPABILITIES
        .iter()
        .copied()
        .find(|c| capability_name(*c) == name)
}

/// Spawn every configured plugin, wait for their manifests together, and hand
/// back what the app needs to place them.
///
/// `data` and `control` are the app's own channels: a plugin publishes exactly
/// the way a source does, which is what makes recording and replay work
/// without a word of plugin-specific code.
pub fn start(
    plugins: &[PluginSect],
    caps: &CapSet,
    data: SyncSender<Batch>,
    control: Sender<ControlMsg>,
    now: Ts,
) -> Started {
    let mut out = Started {
        host: None,
        defs: Vec::new(),
        sources: Vec::new(),
        warnings: Vec::new(),
    };
    if plugins.is_empty() {
        return out;
    }
    let hello = Hello::new(
        ALL_CAPABILITIES
            .iter()
            .filter(|c| caps.has(**c))
            .map(|c| capability_name(*c))
            .collect(),
        gridwatch_store::CATALOGUE
            .iter()
            .flat_map(|d| d.iter())
            .map(|m| m.name.to_string())
            .collect(),
    );
    // Both channels before anything is spawned, so a build closure can capture
    // the sender it needs and the host thread the receiver.
    let (wishes, wish_rx) = channel();
    let (word_tx, words) = channel();

    // Spawn them all first: the waits below then overlap, so two plugins cost
    // the slower one's `hello_ms` rather than both.
    let mut running: Vec<Plugin> = plugins
        .iter()
        .map(|p| {
            Plugin::spawn(
                PluginConfig {
                    argv: p.argv.clone(),
                    id: p.id.clone(),
                    rss_mb: p.rss_mb,
                    cpu_secs: p.cpu_secs,
                },
                hello.clone(),
            )
        })
        .collect();

    let mut early: Vec<Vec<Report>> = vec![Vec::new(); running.len()];
    let mut states: Vec<PluginState> = Vec::new();
    let mut kinds: BTreeMap<String, usize> = BTreeMap::new();
    for (i, (plugin, sect)) in running.iter().zip(plugins).enumerate() {
        let deadline = Instant::now() + Duration::from_millis(sect.hello_ms);
        let mut manifest = None;
        let mut stopped = None;
        while manifest.is_none() && stopped.is_none() {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                break;
            }
            match plugin.next_report(left) {
                Some(Report::Ready(m)) => manifest = Some(m),
                Some(Report::Stopped(why)) => stopped = Some(why),
                // Anything it said before its manifest is still its to say;
                // it is delivered as soon as the host thread starts.
                Some(other) => early[i].push(other),
                None => break,
            }
        }
        let id: &'static str = Box::leak(sect.id.clone().into_boxed_str());
        let source = SourceId(id);
        match (manifest, stopped) {
            (_, Some(why)) => {
                out.warnings.push(format!("plugin '{}': {why}", sect.id));
                states.push(PluginState::new(source));
            }
            (None, None) => {
                out.warnings.push(format!(
                    "plugin '{}': no manifest within {} ms — it keeps running as a source, \
                     but it has no tile",
                    sect.id, sect.hello_ms
                ));
                states.push(PluginState::new(source));
            }
            (Some(m), None) => {
                let (manifest, tiers, warnings) = to_static(id, &m);
                out.warnings.extend(warnings);
                out.sources.push(source);
                kinds.insert(manifest.kind.to_string(), i);
                out.defs.push(ComponentDef {
                    manifest,
                    build: builder(i, manifest, tiers, wishes.clone()),
                });
                states.push(PluginState::new(source));
            }
        }
    }

    let render_ms: Vec<u64> = plugins.iter().map(|p| p.render_ms).collect();
    let thread = std::thread::Builder::new()
        .name("gw-plugins".into())
        .spawn(move || {
            let mut hosted = Hosted {
                plugins: std::mem::take(&mut running),
                states,
                tiles: BTreeMap::new(),
                data,
                control,
                words: word_tx,
                started: now,
            };
            for (i, reports) in early.into_iter().enumerate() {
                for r in reports {
                    hosted.report(i, r);
                }
            }
            hosted.run(wish_rx);
        })
        .expect("spawn the plugin host thread");

    out.host = Some(Host {
        wishes: Some(wishes),
        words,
        kinds,
        render_ms,
        asked: BTreeMap::new(),
        thread: Some(thread),
    });
    out
}

/// Per-plugin state the host thread keeps.
struct PluginState {
    source: SourceId,
    /// Metric names interned for this plugin, bounded by `MAX_METRIC_NAMES`.
    names: BTreeMap<String, &'static str>,
    capped: bool,
    /// The runaway check's window: when it opened and the child's CPU then.
    /// `None` until the first reading, and reset whenever the child drops
    /// back under the share.
    window: Option<(Instant, Duration)>,
    stopped: bool,
    /// How many reports the queue had dropped when we last said so, so the
    /// warning is raised once per rise rather than once per second.
    said_dropped: u64,
}

impl PluginState {
    fn new(source: SourceId) -> PluginState {
        PluginState {
            source,
            names: BTreeMap::new(),
            capped: false,
            window: None,
            stopped: false,
            said_dropped: 0,
        }
    }

    /// A plugin's metric name as a `&'static str`, interned once and capped.
    fn intern(&mut self, name: &str) -> Option<&'static str> {
        if let Some(s) = self.names.get(name) {
            return Some(s);
        }
        if self.names.len() >= MAX_METRIC_NAMES {
            if !self.capped {
                self.capped = true;
                tracing::warn!(
                    target: "gridwatch::plugin",
                    "{}: more than {MAX_METRIC_NAMES} distinct metric names — the rest are dropped",
                    self.source.0
                );
            }
            return None;
        }
        let s: &'static str = Box::leak(name.to_string().into_boxed_str());
        self.names.insert(name.to_string(), s);
        Some(s)
    }
}

/// The host thread.
struct Hosted {
    plugins: Vec<Plugin>,
    states: Vec<PluginState>,
    /// instance → the plugin that backs it and the way to reach its tile.
    tiles: BTreeMap<String, (usize, Sender<Tell>)>,
    data: SyncSender<Batch>,
    control: Sender<ControlMsg>,
    words: Sender<Word>,
    started: Ts,
}

impl Hosted {
    fn run(&mut self, wishes: Receiver<Wish>) {
        let mut supervised = Instant::now();
        loop {
            // Park rather than spin: a plugin's reports arrive on its own
            // channel and this loop is the only thing that reads them, so the
            // timeout is what bounds how long a sample waits.
            match wishes.recv_timeout(Duration::from_millis(50)) {
                Ok(Wish::Stop) => break,
                Ok(w) => self.wish(w),
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            }
            let mut stop = false;
            while let Ok(w) = wishes.try_recv() {
                match w {
                    Wish::Stop => stop = true,
                    w => self.wish(w),
                }
            }
            if stop {
                break;
            }
            for i in 0..self.plugins.len() {
                for r in self.plugins[i].drain() {
                    self.report(i, r);
                }
            }
            if supervised.elapsed() >= Duration::from_secs(1) {
                supervised = Instant::now();
                self.supervise();
            }
        }
        for p in &mut self.plugins {
            p.stop();
        }
    }

    /// Once a second: stop a plugin that is spinning, and say when the queue
    /// has had to drop (D58 seam 7). The rate budget already makes a plugin
    /// that *writes* too much free; this is the one that simply burns.
    fn supervise(&mut self) {
        for i in 0..self.plugins.len() {
            if self.states[i].stopped {
                continue;
            }
            let dropped = self.plugins[i].dropped();
            if dropped > self.states[i].said_dropped {
                self.states[i].said_dropped = dropped;
                tracing::warn!(
                    target: "gridwatch::plugin",
                    "{}: {dropped} messages dropped to keep the queue bounded",
                    self.states[i].source.0
                );
            }
            let Some(used) = self.plugins[i].cpu_used() else {
                self.states[i].window = None;
                continue;
            };
            let now = Instant::now();
            let (since, then) = *self.states[i].window.get_or_insert((now, used));
            let elapsed = now.duration_since(since);
            let Some(why) = supervise::runaway(elapsed, used.saturating_sub(then)) else {
                // Back under the share (or the window is not up yet): open a
                // fresh window rather than averaging a quiet minute against a
                // busy second.
                if elapsed >= supervise::RUNAWAY_WINDOW {
                    self.states[i].window = Some((now, used));
                }
                continue;
            };
            tracing::warn!(target: "gridwatch::plugin", "{}: {why}", self.states[i].source.0);
            self.plugins[i].stop();
            self.states[i].stopped = true;
            let _ = self.words.send(Word::Toast(
                Severity::Crit,
                format!("plugin '{}' {why}", self.states[i].source.0),
            ));
            self.status(i, proto::State::Stopped, Some(why.clone()), None);
            self.tell_plugin(
                i,
                &Tell::Status {
                    state: proto::State::Stopped,
                    reason: Some(why),
                    hint: Some("a plugin is stopped, not retried, when it runs away".into()),
                },
            );
        }
    }

    fn wish(&mut self, w: Wish) {
        match w {
            // Handled by the loop, which has to leave it rather than route it.
            Wish::Stop => {}
            Wish::Attach {
                plugin,
                instance,
                to_tile,
            } => {
                self.tiles.insert(instance, (plugin, to_tile));
            }
            Wish::Render {
                plugin,
                instance,
                tier,
                inner,
                now,
                focused,
                captured,
            } => {
                if let Some(p) = self.plugins.get_mut(plugin) {
                    p.ask(&Ask::Render {
                        instance,
                        tier,
                        inner,
                        now,
                        focused,
                        captured,
                    });
                }
            }
            Wish::Key {
                plugin,
                instance,
                key,
                mods,
            } => {
                if let Some(p) = self.plugins.get_mut(plugin) {
                    p.ask(&Ask::Key {
                        instance,
                        key,
                        mods,
                    });
                }
            }
        }
    }

    /// One tile, by the instance name it attached under.
    fn tell(&self, instance: &str, tell: Tell) {
        if let Some((_, tx)) = self.tiles.get(instance) {
            let _ = tx.send(tell);
        }
        let _ = self.words.send(Word::Landed);
    }

    /// Every tile this plugin backs — a status belongs to all of them.
    fn tell_plugin(&self, plugin: usize, tell: &Tell) {
        for (p, tx) in self.tiles.values() {
            if *p == plugin {
                let _ = tx.send(tell.clone());
            }
        }
        let _ = self.words.send(Word::Landed);
    }

    fn report(&mut self, i: usize, r: Report) {
        match r {
            // The handshake already took it; a second one is refused upstream.
            Report::Ready(_) => {}
            Report::Sample {
                key,
                label,
                at,
                value,
            } => {
                let source = self.states[i].source;
                let Some(name) = self.states[i].intern(&key) else {
                    return;
                };
                let label = match label {
                    None => Label::None,
                    Some(proto::WireLabel::Index(n)) => Label::Index(n),
                    Some(proto::WireLabel::Name(s)) => Label::Name(Arc::from(s.as_str())),
                };
                let at = at
                    .filter(|n| *n >= 0)
                    .map(|n| Ts(n as u64))
                    .unwrap_or(self.started);
                // A full data channel drops, as it does for every source: the
                // channel is lossy by design (§4.2) and a plugin is not the
                // thing to make it block for.
                let _ = self.data.try_send(Batch {
                    source,
                    at,
                    samples: vec![Sample {
                        id: MetricId { name, label },
                        datum: Datum::Scalar(value),
                    }],
                });
            }
            Report::View { instance, tree } => self.tell(&instance, Tell::Tree(tree)),
            Report::Command(cmd) => {
                let word = match cmd {
                    proto::Cmd::Toast { severity, text } => {
                        Word::Toast(severity_of(severity), text)
                    }
                    proto::Cmd::Page(p) => Word::Page(p),
                    proto::Cmd::Zoom(z) => Word::Zoom(z),
                };
                let _ = self.words.send(word);
            }
            Report::Status {
                state,
                reason,
                hint,
            } => {
                self.status(i, state, reason.clone(), hint.clone());
                self.tell_plugin(
                    i,
                    &Tell::Status {
                        state,
                        reason,
                        hint,
                    },
                );
            }
            Report::Log { level, text } => {
                tracing::info!(
                    target: "gridwatch::plugin",
                    "{}: [{}] {text}",
                    self.states[i].source.0,
                    level.as_deref().unwrap_or("info")
                );
            }
            Report::Refused { why, strike } => {
                tracing::warn!(
                    target: "gridwatch::plugin",
                    "{}: refused ({strike}/3): {why}",
                    self.states[i].source.0
                );
            }
            Report::Stopped(why) => {
                self.states[i].stopped = true;
                self.status(
                    i,
                    proto::State::Stopped,
                    Some(why.clone()),
                    Some("see the log — a plugin that cannot speak is stopped, not retried".into()),
                );
                self.tell_plugin(
                    i,
                    &Tell::Status {
                        state: proto::State::Stopped,
                        reason: Some(why),
                        hint: None,
                    },
                );
            }
        }
    }

    fn status(&self, i: usize, state: proto::State, reason: Option<String>, hint: Option<String>) {
        let s = SourceStatus {
            state: match state {
                proto::State::Starting => SourceState::Starting,
                proto::State::Ok => SourceState::Ok,
                proto::State::Degraded => SourceState::Degraded,
                proto::State::Unavailable | proto::State::Stopped => SourceState::Unavailable,
            },
            reason: reason.map(|r| Arc::from(r.as_str())),
            hint: hint.map(|h| Arc::from(h.as_str())),
            since: self.started,
            last_sample: None,
            dropped: 0,
            restarts: 0,
        };
        let _ = self
            .control
            .send(ControlMsg::Status(self.states[i].source, s));
    }
}

fn severity_of(s: proto::Severity) -> Severity {
    match s {
        proto::Severity::Info => Severity::Info,
        proto::Severity::Warn => Severity::Warn,
        proto::Severity::Crit => Severity::Crit,
    }
}

/// The build closure for one plugin kind (§4.6): it captures the plugin it
/// speaks for, its statics, and the channel to the host thread — none of which
/// a bare `fn` pointer could carry.
fn builder(
    plugin: usize,
    manifest: &'static Manifest,
    tiers: &'static [Tier],
    wishes: Sender<Wish>,
) -> gridwatch_ui::Build {
    Box::new(move |cx| {
        if !cx.options.is_empty() {
            // Contract 1 has no message that carries instance options, and
            // inventing one is a contract change rather than an implementation
            // detail (§4.7). Say so rather than ignoring them silently.
            tracing::warn!(
                target: "gridwatch::plugin",
                "{}: options are not delivered to a plugin in contract 1 — ignored",
                manifest.kind
            );
        }
        let (to_tile, from_host) = channel();
        let _ = wishes.send(Wish::Attach {
            plugin,
            instance: cx.instance.to_string(),
            to_tile,
        });
        Ok(Box::new(PluginTile::new(
            plugin,
            manifest,
            tiers,
            cx.instance.to_string(),
            wishes.clone(),
            from_host,
        )) as Box<dyn Component>)
    })
}

/// Does this kind look like it belongs to a configured plugin that is not
/// running? The chip a placement gets should say that, not "arrives in a later
/// arc" (§6).
pub fn looks_like_plugin_kind(kind: &str, ids: &BTreeSet<String>) -> bool {
    kind.split_once('.').is_some_and(|(id, _)| ids.contains(id))
}

/// The wire manifest as the `&'static` one the registry holds. Exactly one is
/// leaked per configured plugin — which is what makes `[[plugins]]`
/// restart-only (§4.7): a reload that rebuilt them would leak per reload.
fn to_static(
    id: &'static str,
    m: &proto::Manifest,
) -> (&'static Manifest, &'static [Tier], Vec<String>) {
    let mut warnings = Vec::new();
    let leak = |s: String| -> &'static str { Box::leak(s.into_boxed_str()) };
    let kind = leak(format!("{id}.{}", m.kind));
    let mut requires = Vec::new();
    for name in &m.requires {
        match capability_named(name) {
            Some(c) => requires.push(c),
            None => warnings.push(format!(
                "plugin '{id}': requires '{name}', which is not a capability this host probes \
                 — ignored"
            )),
        }
    }
    let optional: Vec<Capability> = m
        .optional
        .iter()
        .filter_map(|n| capability_named(n))
        .collect();
    let footprints: Vec<Footprint> = m
        .footprints
        .iter()
        .map(|f| Footprint {
            w: f.w.clamp(1, 255) as u8,
            h: f.h.clamp(1, 255) as u8,
        })
        .collect();
    let footprints: &'static [Footprint] = if footprints.is_empty() {
        &[
            gridwatch_ui::TILE,
            gridwatch_ui::WIDE,
            gridwatch_ui::PANEL,
            gridwatch_ui::HERO,
        ]
    } else {
        Box::leak(footprints.into_boxed_slice())
    };
    let default_footprint = m
        .default_footprint
        .map(|f| Footprint {
            w: f.w.clamp(1, 255) as u8,
            h: f.h.clamp(1, 255) as u8,
        })
        .unwrap_or(footprints[0]);
    let keys: &'static [KeyHint] = Box::leak(
        m.keys
            .iter()
            .map(|k| KeyHint {
                key: leak(k.key.clone()),
                does: leak(k.does.clone()),
            })
            .collect::<Vec<_>>()
            .into_boxed_slice(),
    );
    // A plugin reads no store, so the only source its tile depends on is
    // itself — under the id *this config* gave it, not the name the manifest
    // wrote. That is what makes its tile re-render when it publishes (§5).
    let sources: &'static [SourceId] = if m.produces.is_empty() && m.sources.is_empty() {
        &[]
    } else {
        Box::leak(vec![SourceId(id)].into_boxed_slice())
    };
    let tiers: &'static [Tier] = Box::leak(
        m.tiers
            .iter()
            .map(|t| Tier {
                name: leak(t.name.clone()),
                min: TileSize::new(t.min.w, t.min.h),
                // A stranger's tree is not something this host can vouch for,
                // so a plugin's tiers make no `signature` claim (D46).
                adds: &[],
                zoom_only: t.zoom_only,
            })
            .collect::<Vec<_>>()
            .into_boxed_slice(),
    );
    let manifest: &'static Manifest = Box::leak(Box::new(Manifest {
        kind,
        name: leak(m.name.clone()),
        summary: leak(
            m.summary
                .clone()
                .unwrap_or_else(|| format!("the '{}' plugin", m.kind)),
        ),
        contract: m.contract,
        footprints,
        default_footprint,
        requires: Box::leak(requires.into_boxed_slice()),
        optional: Box::leak(optional.into_boxed_slice()),
        sources,
        optional_sources: &[],
        chrome: Chrome::Themed,
        keys,
        example_options: "",
    }));
    (manifest, tiers, warnings)
}
