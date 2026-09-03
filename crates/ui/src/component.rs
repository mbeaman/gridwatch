//! The component contract (§4.6): manifest, tiers, demand, view.

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::fmt;
use std::time::SystemTime;

use gridwatch_store::{
    ActionId, AlertId, CapSet, Capability, Control, Detail, KeyEvent, MouseEvent, Severity,
    SourceDef, SourceId, Store, Ts,
};
use ratatui_core::layout::{Position, Rect};

use crate::theme::Theme;
use crate::view::View;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Size {
    pub w: u16,
    pub h: u16,
}

impl Size {
    pub const fn new(w: u16, h: u16) -> Size {
        Size { w, h }
    }

    pub fn fits(self, inner: Size) -> bool {
        self.w <= inner.w && self.h <= inner.h
    }
}

/// Picker hints only — the real rect picks the tier (§4.6, principle 7).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Footprint {
    pub w: u8,
    pub h: u8,
}

pub const TILE: Footprint = Footprint { w: 1, h: 1 };
pub const WIDE: Footprint = Footprint { w: 2, h: 1 };
pub const PANEL: Footprint = Footprint { w: 4, h: 2 };
pub const HERO: Footprint = Footprint { w: 6, h: 3 };

/// Cumulative tiers, poorest first; `zoom_only` tiers form a suffix (§4.6).
#[derive(Clone, Copy, Debug)]
pub struct Tier {
    pub name: &'static str,
    pub min: Size,
    pub adds: &'static [&'static str],
    pub zoom_only: bool,
}

/// Pick the richest tier whose `min` fits, skipping `zoom_only` unless zoomed
/// or named by `view`. Returns `(tier index, view_fallback)`.
pub fn pick_tier(tiers: &[Tier], inner: Size, zoomed: bool, view: Option<&str>) -> (usize, bool) {
    let fits = |t: &Tier| t.min.fits(inner);
    let richest = tiers
        .iter()
        .enumerate()
        .filter(|(_, t)| fits(t) && (zoomed || !t.zoom_only))
        .map(|(i, _)| i)
        .next_back()
        .unwrap_or(0);
    // Unknown view names warn at config load and are ignored (§4.6).
    // Zoom always gives the richest tier; a pinned view applies un-zoomed only.
    if !zoomed
        && let Some(name) = view
        && let Some((i, t)) = tiers.iter().enumerate().find(|(_, t)| t.name == name)
    {
        if fits(t) {
            return (i, false);
        }
        return (richest, true); // preferred tier does not fit: view↓ chip
    }
    (richest, false)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Chrome {
    Themed,
    Borderless,
    Custom,
}

#[derive(Clone, Copy, Debug)]
pub struct KeyHint {
    pub key: &'static str,
    pub does: &'static str,
}

pub struct Manifest {
    pub kind: &'static str,
    pub name: &'static str,
    pub summary: &'static str,
    pub contract: u32,
    pub footprints: &'static [Footprint],
    pub default_footprint: Footprint,
    pub requires: &'static [Capability],
    pub optional: &'static [Capability],
    pub sources: &'static [SourceId],
    pub optional_sources: &'static [SourceId],
    pub chrome: Chrome,
    pub keys: &'static [KeyHint],
    pub example_options: &'static str,
}

#[derive(Debug)]
pub struct BuildError(pub String);

impl fmt::Display for BuildError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "build: {}", self.0)
    }
}

impl std::error::Error for BuildError {}

pub struct BuildCx<'a> {
    pub options: &'a toml::Table,
    pub caps: &'a CapSet,
}

pub struct ComponentDef {
    pub manifest: &'static Manifest,
    pub build: fn(&mut BuildCx<'_>) -> Result<Box<dyn Component>, BuildError>,
}

/// Static registry assembled by the binary from Cargo features (§4.6, §4.7).
#[derive(Default)]
pub struct Registry {
    components: BTreeMap<&'static str, ComponentDef>,
    sources: BTreeMap<&'static str, SourceDef>,
}

impl Registry {
    pub fn register_component(&mut self, def: ComponentDef) {
        self.components.insert(def.manifest.kind, def);
    }

    pub fn register_source(&mut self, def: SourceDef) {
        self.sources.insert(def.info.id.0, def);
    }

    pub fn component(&self, kind: &str) -> Option<&ComponentDef> {
        self.components.get(kind)
    }

    pub fn components(&self) -> impl Iterator<Item = &ComponentDef> {
        self.components.values()
    }

    pub fn source(&self, id: &str) -> Option<&SourceDef> {
        self.sources.get(id)
    }

    pub fn sources(&self) -> impl Iterator<Item = &SourceDef> {
        self.sources.values()
    }
}

pub struct RenderCx<'a> {
    pub inner: Rect,
    pub tier: usize,
    pub view_fallback: bool,
    pub focused: bool,
    pub captured: bool,
    pub zoomed: bool,
    pub dense: bool,
    pub store: &'a Store,
    pub theme: &'a Theme,
    pub now: Ts,
    pub wall: SystemTime,
    /// Local-time offset in seconds (the app computes it once; testkit uses 0),
    /// so wall-clock rendering stays deterministic under replay.
    pub tz_offset_s: i32,
    pub frame: u64,
}

pub struct TickCx<'a> {
    pub store: &'a Store,
    pub now: Ts,
    pub visible: bool,
    pub tier: usize,
}

pub struct InputCx<'a> {
    pub store: &'a Store,
    pub inner: Rect,
    pub caps: &'a CapSet,
    /// `--readonly` or `readonly = true`: the shell refuses every action,
    /// and a component that offers one should say so rather than pretend.
    pub readonly: bool,
    /// Whether this tile is zoomed, and which tier it is therefore
    /// drawing (arc 8a review, D58 amendment 7).
    ///
    /// A component cannot infer either from `inner`: a 6x3 tile on a big
    /// screen is 122x31, which clears every `zoom_only` tier's minimum, so
    /// size alone told htop it was zoomed when it was not — and the
    /// zoom-only tier's *keys* (which include renicing a process) answered
    /// on the grid, where its F-key bar and its pickers were not drawn.
    /// `view` and `demand` were always given the real tier; now `on_key`
    /// is too.
    pub zoomed: bool,
    pub tier: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Redraw {
    No,
    Yes,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RedrawPolicy {
    OnChange,
    Animated { fps: u8 },
}

/// Open-ended side effects, executed on the executor thread (§4.6, D58).
///
/// An action is built by a component — capturing whatever it needs, so it
/// stays `Send` and testable with no machine — and run by the app. It says
/// for itself whether a person should be asked first, and which processes
/// it will touch.
pub trait Action: std::any::Any + fmt::Debug + Send {
    fn run(self: Box<Self>) -> Result<String, String>;

    /// The question to put in the confirm bar, or `None` to run at once.
    /// The default asks, using the action's own `Debug`: anything that
    /// changes another process should be confirmed, and a new action that
    /// forgets to say so gets the careful behaviour rather than the
    /// dangerous one.
    fn confirm(&self) -> Option<String> {
        Some(format!("{self:?}?"))
    }

    /// The processes this action will act on, and whether it knows.
    ///
    /// `None` means "this action does not say", which the executor's
    /// fence treats as **refusable** — the same direction `confirm()`
    /// fails in. An action that forgets to implement this must not slip
    /// past a fence that exists to stop exactly that (arc 8a review, D58
    /// amendment 9); an action that genuinely touches nothing outside
    /// this process says so with `Some(vec![])`.
    fn pids(&self) -> Option<Vec<u32>> {
        None
    }
}

pub enum Command {
    Quit,
    Page(usize),
    Zoom,
    Ack(AlertId),
    Toast(Severity, String),
    Record(bool),
    SaveLayout,
    Source(SourceId, Control),
    Run(ActionId, Box<dyn Action>),
}

impl fmt::Debug for Command {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Command::Quit => f.write_str("Quit"),
            Command::Page(p) => write!(f, "Page({p})"),
            Command::Zoom => f.write_str("Zoom"),
            Command::Ack(id) => write!(f, "Ack({:?})", id.0),
            Command::Toast(s, m) => write!(f, "Toast({s:?}, {m})"),
            Command::Record(b) => write!(f, "Record({b})"),
            Command::SaveLayout => f.write_str("SaveLayout"),
            Command::Source(id, c) => write!(f, "Source({id}, {c:?})"),
            Command::Run(id, a) => write!(f, "Run({id:?}, {a:?})"),
        }
    }
}

pub enum Outcome {
    Ignored,
    Consumed,
    Command(Command),
    Release,
}

/// The component contract (§4.6): describe, never do.
pub trait Component: Send {
    fn manifest(&self) -> &'static Manifest;

    fn title(&self, max_width: u16, cx: &TickCx<'_>) -> Cow<'static, str>;

    /// Poorest first; `tiers()[0].min` must fit the grid's `min_unit_inner`;
    /// `zoom_only` tiers form a suffix.
    fn tiers(&self) -> &'static [Tier];

    /// What this tier needs from every source in `sources ∪ optional_sources`.
    fn demand(&self, tier: usize) -> Detail {
        let _ = tier;
        Detail::Meters
    }

    /// Derive per-generation state, advance animations; no I/O.
    fn tick(&mut self, cx: &TickCx<'_>) -> Redraw;

    /// Pure over store + now; the theme's renderer draws the result.
    fn view(&self, cx: &RenderCx<'_>) -> View;

    /// Strings that must appear in the rendered cells whenever `tier` is
    /// chosen and the store has data (D46): `Tier.adds` as a checked claim, not
    /// a comment. The testkit's sweep asserts every one of them at every size
    /// that picks the tier. Empty means "non-blank is enough".
    fn signature(&self, tier: usize) -> &'static [&'static str] {
        let _ = tier;
        &[]
    }

    fn redraw_policy(&self) -> RedrawPolicy {
        RedrawPolicy::OnChange
    }

    fn on_key(&mut self, key: KeyEvent, cx: &InputCx<'_>) -> Outcome {
        let _ = (key, cx);
        Outcome::Ignored
    }

    fn on_mouse(&mut self, ev: MouseEvent, local: Position, cx: &InputCx<'_>) -> Outcome {
        let _ = (ev, local, cx);
        Outcome::Ignored
    }

    fn on_visibility(&mut self, visible: bool) {
        let _ = visible;
    }
}
