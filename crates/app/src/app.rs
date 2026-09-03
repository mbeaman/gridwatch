//! The shell (§5, §10): solve → demand → tick → view → render → cache → draw,
//! with pages, focus, capture, zoom, pause, theme cycling and the F12 HUD.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime};

use gridwatch_store::{
    CapSet, Clock, Control, ControlMsg, Detail, Inbox, InputEvent, KeyCode, Level, Msg, Recorder,
    ReloadKind, Severity, SourceId, Store, Ts,
};
use gridwatch_ui::component::{
    Action, BuildCx, Chrome, Command, Component, Outcome, Size, pick_tier,
};
use gridwatch_ui::layout::{
    Cell, Direction, Page, PlaceTarget, SolveMode, Solved, derive_mode, focus_dir, hit, solve,
    unit_at, unit_rect,
};

use crate::ambient::{Ambient, Pins};
use crate::edit::{EditKey, EditState};
use crate::effects::{Effects, Hook};
use gridwatch_ui::theme::{BUILTIN_THEMES, Role, Theme, TitleStyle, load_builtin};
use gridwatch_ui::view::Span as RSpanText;
use gridwatch_ui::{Registry, overlay};
use ratatui::Terminal;
use ratatui::backend::Backend;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::text::{Line as RLine, Span as RSpan};
use ratatui::widgets::Widget;

use crate::config::Loaded;
use crate::stats::FrameStats;

const CHROME_ROWS: u16 = 2;
const HEARTBEAT: Duration = Duration::from_secs(1);
const TOAST_TTL: Duration = Duration::from_secs(4);

struct Instance {
    kind: String,
    /// Seam 5 (arc 5): the last drawn tick returned `Redraw::Yes` under an
    /// `Animated` policy — cleared at the top of every frame, so only
    /// visible, drawn tiles count.
    animated: bool,
    anim_fps: u8,
    /// The `[[components]] options` it was built with: a reload keeps an
    /// instance whose `(kind, options)` did not change (§9).
    options: toml::Table,
    component: Option<Box<dyn Component>>,
    chip_reason: String,
    /// The fix line under the reason (§11 placeholder tiles).
    chip_hint: String,
}

struct Toast {
    text: String,
    severity: Severity,
    /// A `Resolved` transition: drawn in `Role::Ok` with a tick.
    resolved: bool,
    /// On the **run clock**, not `Instant::now()`. Live they are the same
    /// thing; under `--replay` the run clock is the journal's, which is
    /// what makes a replayed frame reproducible — a toast raised by a
    /// recorded status used to expire on wall time, so replaying the same
    /// journal twice could draw it once and not the other time (CI caught
    /// it as a determinism failure; D47's promise is byte-identical
    /// replay).
    expires_at: Ts,
}

/// Per-source `Control` senders (D50 §6): `Command::Source(id, ctl)` lands
/// here; empty in headless shots and tests.
pub type Controls = BTreeMap<&'static str, Arc<dyn Fn(Control) + Send + Sync>>;

#[derive(Clone, PartialEq)]
struct CacheKey {
    gens: Vec<u64>,
    /// The animation frame (§5): `self.frame` while the tile is `Animated`
    /// and moving, else 0 — so an animated tile re-renders every frame and a
    /// quiet one never does.
    anim: u64,
    tier: usize,
    w: u16,
    h: u16,
    theme: String,
    focused: bool,
    zoomed: bool,
    view_hash: u64,
}

/// Three cadences per source is the staleness threshold (§11, seam 10).
const STALE_CADENCES: u32 = 3;

pub struct Shell {
    pub store: Store,
    registry: Registry,
    theme: Theme,
    /// What `theme` was loaded from — a built-in name or a `.toml` path — so
    /// `T` and a theme-file change can reload it.
    theme_ref: String,
    /// `--theme` or `NO_COLOR` on the command line: a `config.toml` reload
    /// does not change the theme (§9 layering — CLI beats the file).
    pub theme_locked: bool,
    /// The theme `config.toml` named at the last load: a reload swaps the
    /// theme only when *that* changes, so an fps edit never undoes `t`.
    config_theme: String,
    /// Visible and focused cadence per source (from `SourceInfo`): the
    /// staleness rule's default and its floor (D53).
    cadences: BTreeMap<&'static str, (Duration, Duration)>,
    /// `[sources.<id>]` as configured: a `refresh_ms`/`interval_ms` there is
    /// the cadence the source actually runs at (review: the registry's
    /// default badged a slower source STALE forever).
    source_options: toml::Table,
    /// `mouse` and `color` as configured — a reload cannot apply them.
    restart_only: (bool, String),
    /// Set on unpause and on focus regained: a source that has not sampled
    /// since then is parked, not stale (D53).
    resumed_at: Option<Ts>,
    /// Under `--replay`, when the journal ended: the virtual clock stops
    /// there, so staleness counts real time from that instant (review).
    journal_ended: Option<Instant>,
    /// Only a live `run --replay` ages a finished journal in real time;
    /// the determinism test and `shot` must stay a pure function of the
    /// journal (CI's slower runner ended the two replays at different ages).
    pub age_after_journal: bool,
    /// The watcher's theme-file sender (§9): the reload target moves with
    /// `t` and with a config edit, and the watched files follow it.
    pub watch_theme_files: Option<std::sync::mpsc::Sender<Vec<crate::watch::Watched>>>,
    /// The watcher's ignore-slot sender: `w` registers its own write's hash
    /// before the file lands (§9, seam 5).
    pub watch_ignore: Option<std::sync::mpsc::Sender<(ReloadKind, u64)>>,
    /// Edit mode (§10, arc 4a): `Some` while `e` is active.
    edit: Option<EditState>,
    /// The effects painter (§7, arc 4b): off in headless shots and tests.
    effects: Effects,
    /// The showcase theme's ambient layer (D28/D31/D34), when the theme has
    /// one and effects are on.
    ambient: Option<Ambient>,
    ambient_on: bool,
    /// The tile under the mouse pointer (a pinned rect under `matrix`).
    hover: Option<usize>,
    /// Bytes written a second ago, for the governor's bytes/s reading.
    bytes_mark: (Instant, u64),
    /// The focus the last frame fired the focus hook for.
    fx_focus: Option<usize>,
    /// Whether the banner was up last frame (the alert pulse's trigger).
    fx_banner: bool,
    fx_started: bool,
    /// The last body rect solved: the mouse's units are read against it.
    last_body: Rect,
    grid: gridwatch_ui::layout::GridSpec,
    pages: Vec<Page>,
    page: usize,
    instances: BTreeMap<String, Instance>,
    caps: CapSet,
    tz_offset_s: i32,
    demands: BTreeMap<&'static str, Arc<gridwatch_store::Demand>>,
    controls: Controls,
    /// Alert ids acknowledged with `a`; a new `Raised` un-acks (§4.4).
    acked: std::collections::BTreeSet<gridwatch_store::AlertId>,
    alerts_overlay: bool,
    focus: Option<usize>,
    captured: bool,
    zoom: Option<usize>,
    dense_override: bool,
    paused: bool,
    pub terminal_focused: bool,
    mode: SolveMode,
    stack_scroll: u16,
    help: bool,
    hud: bool,
    pub quit: bool,
    frame: u64,
    toasts: Vec<Toast>,
    cache: BTreeMap<usize, (CacheKey, Buffer)>,
    last_solved: Option<Solved>,
    last_gens: BTreeMap<&'static str, u64>,
    last_click: Option<(Instant, usize)>,
    last_area: Rect,
    pub stats: FrameStats,
    /// When the process reached the shell — the origin for P18's timestamps.
    pub startup: Instant,
    pub bytes_counter: Option<Arc<AtomicU64>>,
    clock: Clock,
    prev_buf: Option<Buffer>,
    fps: u16,
    fps_max: u16,
    unfocused_fps: u16,
    pub stats_log: Option<std::path::PathBuf>,
    view_warnings: Vec<String>,
    /// `--record`: the journal tee (§4.5). `r` toggles it; the HUD counts it.
    pub recorder: Option<Recorder>,
    /// The action executor (§4.6, seam 11, arc 8a). `None` in `shot` and
    /// in tests that never run one.
    executor: Option<crate::exec::Executor>,
    /// An action waiting on `y`: the question, and what to run if the
    /// answer is yes. Confirmation is the shell's job, not each
    /// component's (D58 seam 2).
    pending_action: Option<(gridwatch_store::ActionId, Box<dyn Action>, String)>,
    /// `--readonly` (or `readonly = true`): actions are refused with a
    /// sentence saying what would have happened.
    pub readonly: bool,
    /// `--readonly` was given on the command line, so a config reload
    /// cannot turn it off (§9's layering: the CLI beats the file).
    readonly_locked: bool,
    /// The id of the next action, so "keys in, commands out" tests can
    /// name one (D42).
    next_action: u64,
    /// What the executor last reported, so a test can drive key → confirm
    /// → executor → toast end to end.
    last_done: Option<(gridwatch_store::ActionId, Result<String, String>)>,
}

impl Shell {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        registry: Registry,
        loaded: &Loaded,
        theme: Theme,
        caps: CapSet,
        tz_offset_s: i32,
        clock: Clock,
        demands: BTreeMap<&'static str, Arc<gridwatch_store::Demand>>,
        controls: Controls,
        stats_on: bool,
    ) -> Shell {
        let mut store = Store::default();
        let mut cadences = BTreeMap::new();
        for def in registry.sources() {
            store.ensure_source(def.info.id);
            cadences.insert(
                def.info.id.0,
                (def.info.cadence.visible, def.info.cadence.focused),
            );
        }
        // At startup there is nothing to resolve; the events are only
        // non-empty on a reload that removed a rule.
        let _ = store.set_rules(gridwatch_store::rules::Rules::new(loaded.rules.clone()));
        let instances = build_instances(&registry, loaded, &caps, None);
        let view_warnings = view_warnings(loaded, &instances);
        let theme_ref = theme.name.clone();
        Shell {
            executor: None,
            pending_action: None,
            readonly: false,
            readonly_locked: false,
            next_action: 0,
            last_done: None,
            store,
            registry,
            theme,
            theme_ref,
            theme_locked: false,
            config_theme: loaded.config.theme.clone(),
            cadences,
            source_options: loaded.config.sources.clone(),
            restart_only: (loaded.config.mouse, loaded.config.color.clone()),
            resumed_at: None,
            journal_ended: None,
            age_after_journal: false,
            watch_theme_files: None,
            watch_ignore: None,
            edit: None,
            effects: Effects::new(false, 4),
            ambient: None,
            ambient_on: false,
            hover: None,
            bytes_mark: (Instant::now(), 0),
            fx_focus: None,
            fx_banner: false,
            fx_started: false,
            last_body: Rect::default(),
            grid: loaded.grid,
            pages: loaded.pages.clone(),
            page: 0,
            instances,
            caps,
            tz_offset_s,
            demands,
            controls,
            acked: std::collections::BTreeSet::new(),
            alerts_overlay: false,
            focus: Some(0),
            captured: false,
            zoom: None,
            dense_override: false,
            paused: false,
            terminal_focused: true,
            mode: SolveMode::Configured,
            stack_scroll: 0,
            help: false,
            hud: stats_on,
            quit: false,
            frame: 0,
            toasts: Vec::new(),
            cache: BTreeMap::new(),
            last_solved: None,
            last_gens: BTreeMap::new(),
            last_click: None,
            last_area: Rect::default(),
            stats: FrameStats::default(),
            startup: Instant::now(),
            bytes_counter: None,
            clock,
            prev_buf: None,
            fps: loaded.config.fps,
            fps_max: loaded.config.fps_max,
            unfocused_fps: loaded.config.perf.unfocused_fps,
            stats_log: None,
            view_warnings,
            recorder: None,
        }
    }

    /// What the theme was loaded from (a built-in name or a `.toml` path).
    pub fn theme_ref(&self) -> &str {
        &self.theme_ref
    }

    pub fn set_theme_ref(&mut self, r: impl Into<String>) {
        self.theme_ref = r.into();
        self.theme_ref_moved();
    }

    /// The reload target changed: the watcher follows it (a `.toml` theme
    /// and its sibling; nothing for a built-in).
    fn theme_ref_moved(&mut self) {
        if let Some(tx) = &self.watch_theme_files {
            let _ = tx.send(crate::watch::theme_files(&self.theme_ref));
        }
    }

    pub fn theme(&self) -> &Theme {
        &self.theme
    }

    /// Switch the effects painter on (`[effects] enabled` and not
    /// `--no-effects`) with its budget (§7, P20).
    pub fn set_effects(&mut self, on: bool, budget_ms: u32) {
        self.effects = Effects::new(on, budget_ms);
        self.ambient_on = on;
        self.rebuild_ambient();
    }

    /// The ambient layer follows the theme: built for a showcase theme with
    /// a layer, dropped otherwise (§7).
    fn rebuild_ambient(&mut self) {
        self.ambient = match (&self.theme.ambient, &self.theme.rain) {
            (Some(spec), Some(rain)) if self.ambient_on => Some(Ambient::new(
                spec.clone(),
                rain.clone(),
                self.theme.rain_glyphs,
                &self.theme.name,
                self.theme.color(Role::Bg),
            )),
            _ => None,
        };
    }

    /// The layer runs while the terminal is focused and not paused (D28:
    /// it is frozen otherwise, and P4 holds).
    fn ambient_active(&self) -> bool {
        self.ambient.is_some() && self.terminal_focused && !self.paused
    }

    /// The ambient layer's state for the HUD.
    pub fn ambient_hud(&self) -> Option<String> {
        self.ambient.as_ref().map(|a| {
            format!(
                "rain {} fps · governor step {}{}{}",
                a.fps(),
                a.governor.step,
                if a.locked() { " · locked" } else { "" },
                if !self.ambient_active() {
                    " · frozen"
                } else {
                    ""
                }
            )
        })
    }

    /// An effect is mid-flight, or the rain is falling: the frame loop draws
    /// at the animation's fps (§5).
    pub fn effects_running(&self) -> bool {
        self.effects.running() || self.ambient_active()
    }

    /// Swap the theme (§9 hot reload, `t`, `T`): the render cache is keyed by
    /// theme name, and cleared anyway so a same-named edited file redraws.
    pub fn swap_theme(&mut self, theme: Theme) {
        for w in theme.warnings.clone() {
            tracing::warn!("theme {}: {w}", theme.name);
            self.toast(Severity::Warn, w);
        }
        let from = (self.theme.color(Role::Text), self.theme.color(Role::Bg));
        // The swap hook: the old theme's when it declares one (it knows how
        // it wants to leave), else the new theme's (review).
        let hooks = if self.theme.effects.theme_swap.is_some() {
            self.theme.effects.clone()
        } else {
            theme.effects.clone()
        };
        self.theme = theme;
        self.cache.clear();
        // Nothing of the old theme keeps running (an alert pulse over a
        // theme without the hook, the old heartbeat rule).
        self.effects.cancel_all();
        self.fx_banner = false;
        self.rebuild_ambient();
        let body = self.last_body;
        self.effects
            .trigger(Hook::ThemeSwap, &hooks, &self.theme, body, Some(from));
    }

    /// Apply a re-parsed config + layout (§9 hot reload): instances whose
    /// `(kind, options)` did not change keep their state (selection, peaks,
    /// a frozen display); the rest are rebuilt; removed ones go. Pages, the
    /// grid, fps and the unfocused fps follow the files. Returns what
    /// changed, for the toast.
    pub fn apply_loaded(&mut self, loaded: &Loaded) -> Vec<String> {
        let old = std::mem::take(&mut self.instances);
        let mut kept = 0;
        let mut rebuilt = 0;
        let mut removed = 0;
        let mut fresh = build_instances(&self.registry, loaded, &self.caps, Some(&old));
        for (key, inst) in old {
            match fresh.get_mut(&key) {
                Some(new) if new.kind == inst.kind && new.options == inst.options => {
                    // Same kind, same options: the old instance and its state.
                    *new = inst;
                    kept += 1;
                }
                Some(_) => rebuilt += 1,
                None => removed += 1,
            }
        }
        let added = fresh.len().saturating_sub(kept + rebuilt);
        self.instances = fresh;
        self.view_warnings = view_warnings(loaded, &self.instances);
        self.grid = loaded.grid;
        self.pages = loaded.pages.clone();
        let resolved = self
            .store
            .set_rules(gridwatch_store::rules::Rules::new(loaded.rules.clone()));
        self.route_alerts(resolved);
        // `readonly` follows a reload: editing it and being told "reloaded"
        // while actions kept running was the wrong answer (arc 8a review,
        // D58 amendment 14). A `--readonly` on the command line still
        // wins, as every CLI flag does over the file (§9).
        if !self.readonly_locked {
            self.readonly = loaded.config.readonly;
        }
        self.fps = loaded.config.fps; // as at start; `set_fps` is the CLI's clamp
        self.fps_max = loaded.config.fps_max;
        self.unfocused_fps = loaded.config.perf.unfocused_fps;
        if self.page >= self.pages.len() {
            self.page = 0;
        }
        let n = self.pages[self.page].place.len();
        if self.focus.is_some_and(|f| f >= n) {
            self.focus = if n == 0 { None } else { Some(0) };
            self.captured = false;
        }
        if self.zoom.is_some_and(|z| z >= n) {
            self.zoom = None;
        }
        self.stack_scroll = 0;
        self.cache.clear();
        self.last_solved = None;
        // A live edit session was about another page: re-baseline it on the
        // reloaded one (review: a stale `saved` let `y` copy another page's
        // tiles over the reloaded page).
        if self.edit.is_some() {
            self.edit = Some(EditState::new(&self.pages[self.page]));
            self.toast(
                Severity::Warn,
                "layout reloaded under edit mode — undo history and unsaved edits reset",
            );
        }
        let mut what = Vec::new();
        if kept > 0 {
            what.push(format!("{kept} kept"));
        }
        if rebuilt > 0 {
            what.push(format!("{rebuilt} rebuilt"));
        }
        if added > 0 {
            what.push(format!("{added} added"));
        }
        if removed > 0 {
            what.push(format!("{removed} removed"));
        }
        what
    }

    /// Reload config + layout from a pair of texts (§9): the parse and the
    /// validation run here on the render thread; an error keeps the old state
    /// and toasts the file, line and column. `kind` names the file that
    /// changed, for the toast; both are always re-read (one `Loaded`).
    pub fn reload_from_texts(&mut self, kind: ReloadKind, config_text: &str, layout_text: &str) {
        match crate::config::load_texts(config_text, layout_text) {
            Ok(loaded) => {
                let what = self.apply_loaded(&loaded);
                let file = match kind {
                    ReloadKind::Layout => "layout.toml",
                    _ => "config.toml",
                };
                let detail = if what.is_empty() {
                    String::new()
                } else {
                    format!(" · {}", what.join(", "))
                };
                self.toast(
                    Severity::Info,
                    format!("{file} reloaded ({} pages{detail})", self.pages.len()),
                );
                for w in loaded
                    .warnings
                    .iter()
                    .chain(self.view_warnings.clone().iter())
                {
                    self.toast(Severity::Warn, w.clone());
                }
                // What a reload cannot apply says so, instead of a toast that
                // claims success over a no-op (review).
                if loaded.config.sources != self.source_options {
                    self.toast(
                        Severity::Warn,
                        "[sources.*] changed — sources are configured at start; restart to apply",
                    );
                    self.source_options = loaded.config.sources.clone();
                }
                let restart_only = (loaded.config.mouse, loaded.config.color.clone());
                if restart_only != self.restart_only {
                    self.toast(
                        Severity::Warn,
                        "`mouse` / `color` changed — restart to apply",
                    );
                    self.restart_only = restart_only;
                }
                let theme_changed = loaded.config.theme != self.config_theme;
                self.config_theme = loaded.config.theme.clone();
                if !self.theme_locked && theme_changed {
                    self.theme_ref = loaded.config.theme.clone();
                    self.theme_ref_moved();
                    self.reload_theme();
                }
            }
            Err(e) => {
                tracing::warn!("reload: {e}");
                self.toast(Severity::Warn, format!("kept the old config — {e}"));
            }
        }
    }

    /// `T`, and a change to the theme file: re-read `theme_ref` at the
    /// current colour mode; an error keeps the old theme and toasts.
    pub fn reload_theme(&mut self) {
        match crate::load_theme_by_name(&self.theme_ref, self.theme.mode) {
            Ok(t) => {
                let name = t.name.clone();
                self.swap_theme(t);
                self.toast(Severity::Info, format!("theme reloaded: {name}"));
            }
            Err(e) => {
                tracing::warn!("theme reload: {e}");
                self.toast(Severity::Warn, format!("kept the old theme — {e}"));
            }
        }
    }

    /// A `ControlMsg::Reload` from the watcher (§9): read the files the way
    /// startup did and apply them.
    fn handle_reload(&mut self, kind: ReloadKind) {
        match kind {
            ReloadKind::Theme => self.reload_theme(),
            ReloadKind::Config | ReloadKind::Layout => match crate::config::read_texts() {
                Ok((c, l)) => {
                    self.reload_from_texts(kind, &c, &l);
                    // A file that is gone reloads as the embedded default —
                    // say that, not "reloaded" (review).
                    let paths = crate::config::watched_paths();
                    let (file, idx) = match kind {
                        ReloadKind::Layout => ("layout.toml", 1),
                        _ => ("config.toml", 0),
                    };
                    if paths.get(idx).is_some_and(|p| !p.exists()) {
                        self.toast(
                            Severity::Warn,
                            format!("{file} removed — the embedded default is in effect"),
                        );
                    }
                }
                Err(e) => self.toast(Severity::Warn, format!("kept the old config — {e}")),
            },
        }
    }

    /// The run clock's now (§4.1): the recorder stamps inputs with it.
    pub fn now(&self) -> Ts {
        self.clock.now()
    }

    /// Advance a virtual clock (no-op on a real one): `shot --replay --at`.
    pub fn set_clock(&self, at: Ts) {
        self.clock.set(at);
    }

    /// Tee a drained message to the recorder, if one is running (§4.5).
    pub fn tee(&self, t: Ts, msg: &Msg) {
        if let Some(r) = &self.recorder {
            r.record(t, msg);
        }
    }

    /// A recorder whose writer died (disk full, I/O error) says so once on
    /// screen (§11: after the alternate screen, the log *and* the UI).
    fn check_recorder(&mut self) {
        if self
            .recorder
            .as_ref()
            .is_some_and(|r| r.dead() && r.enabled())
        {
            let path = self
                .recorder
                .as_ref()
                .map(|r| r.path().display().to_string())
                .unwrap_or_default();
            if let Some(r) = &self.recorder {
                r.set_enabled(false);
            }
            tracing::error!("recording to {path} stopped: the writer failed");
            self.toast(
                Severity::Crit,
                format!("recording stopped: cannot write {path}"),
            );
        }
    }

    /// The solve mode the last frame was drawn in (§6, with hysteresis).
    pub fn mode(&self) -> SolveMode {
        self.mode
    }

    /// Config warnings that need the registry to detect (§4.6): a placement
    /// naming a tier the component does not have.
    pub fn view_warnings(&self) -> &[String] {
        &self.view_warnings
    }

    /// Surface a config warning in the UI as well as the log — a warning the
    /// user cannot see is not a warning.
    pub fn warn_toast(&mut self, text: impl Into<String>) {
        self.toast(Severity::Warn, text);
    }

    /// Apply one control message and surface what the user should know (§11,
    /// D46): alerts toast by severity, and a source *entering* `Unavailable`
    /// toasts once with its reason — the `sources` tile shows the state, but a
    /// transition the user would otherwise only find in the log is a failure
    /// nobody saw.
    pub fn apply_control(&mut self, c: ControlMsg) {
        if let ControlMsg::Reload(r) = &c {
            self.handle_reload(r.kind);
            return;
        }
        if let ControlMsg::Status(id, st) = &c
            && *id == gridwatch_store::JOURNAL
            && st.state == gridwatch_store::SourceState::Stopped
            && self.journal_ended.is_none()
        {
            self.journal_ended = Some(Instant::now());
        }
        if let ControlMsg::Status(id, st) = &c
            && st.state == gridwatch_store::SourceState::Unavailable
            && self.store.status(*id).state != gridwatch_store::SourceState::Unavailable
        {
            let reason = st.reason.as_deref().unwrap_or("no reason given");
            self.toast(Severity::Warn, format!("{id} unavailable: {reason}"));
        }
        // An action's result (§4.6, arc 8a): the store has nothing to do
        // with it, but the person who pressed `y` is waiting to hear.
        // Before this it was logged and dropped, and D58 claimed
        // otherwise (arc 8a review, D58 amendment 8).
        if let ControlMsg::Done(id, result) = &c {
            match result {
                Ok(msg) => self.toast(Severity::Info, msg.clone()),
                Err(e) => self.toast(Severity::Warn, e.clone()),
            }
            tracing::info!(action = id.0, ok = result.is_ok(), "action reported");
            self.last_done = Some((*id, result.clone()));
            return;
        }
        let events = self.store.apply(&Msg::Control(c));
        self.route_alerts(events);
    }

    /// The last action result, for the tests that drive the whole path.
    pub fn last_done(&self) -> Option<&(gridwatch_store::ActionId, Result<String, String>)> {
        self.last_done.as_ref()
    }

    /// Toast and un-acknowledge whatever the store just raised. A source's
    /// `Alert` control message and a `[[rules]]` alert (arc 7b, raised
    /// inside `apply` over a batch) take exactly this path, so the banner,
    /// the alerts tile and `a` need no special case for either.
    pub fn route_alerts(&mut self, events: impl IntoIterator<Item = gridwatch_store::AlertEvent>) {
        for ev in events {
            match ev.transition {
                gridwatch_store::Transition::Raised => {
                    // A new raise brings an acknowledged banner back.
                    self.acked.remove(&ev.id);
                    self.toast(
                        ev.severity,
                        gridwatch_components::alerts::headline(&ev.title, &ev.detail),
                    );
                }
                gridwatch_store::Transition::Repeated => {
                    self.toast(
                        ev.severity,
                        gridwatch_components::alerts::headline(&ev.title, &ev.detail),
                    );
                }
                gridwatch_store::Transition::Resolved => {
                    self.toasts.push(Toast {
                        text: format!("{} {}", ev.title, ev.detail),
                        severity: Severity::Info,
                        resolved: true,
                        expires_at: self.now().plus(TOAST_TTL),
                    });
                }
            }
        }
    }

    /// One frame's worth of `absent` rules (arc 7b): the store's own clock
    /// cannot notice a key that stopped arriving, so the shell asks.
    pub fn tick_rules(&mut self) {
        if self.store.rules().is_empty() {
            return;
        }
        let events = self.store.tick_rules(self.now());
        self.route_alerts(events);
    }

    pub fn set_fps(&mut self, fps: u16) {
        self.fps = fps.clamp(1, 60);
    }

    pub fn set_page(&mut self, p: usize) {
        if p < self.pages.len() {
            let from = self.page;
            self.page = p;
            self.zoom = None;
            self.focus = Some(0);
            self.stack_scroll = 0;
            self.cache.clear();
            if from != p {
                self.notify_visibility(from, false);
                self.notify_visibility(p, true);
            }
        }
    }

    /// `Component::on_visibility` for every instance placed on a page
    /// (arc 5a, D55 amendment 18): a picker closes when its page is left.
    fn notify_visibility(&mut self, page: usize, visible: bool) {
        let keys: Vec<String> = self
            .pages
            .get(page)
            .map(|pg| {
                pg.place
                    .iter()
                    .map(|pl| self.instance_key(&pl.target))
                    .collect()
            })
            .unwrap_or_default();
        for k in keys {
            if let Some(c) = self
                .instances
                .get_mut(&k)
                .and_then(|i| i.component.as_mut())
            {
                c.on_visibility(visible);
            }
        }
    }

    fn toast(&mut self, severity: Severity, text: impl Into<String>) {
        self.toasts.push(Toast {
            text: text.into(),
            severity,
            resolved: false,
            expires_at: self.now().plus(TOAST_TTL),
        });
    }

    /// The banner's text: every active, unacknowledged `Crit` alert's title
    /// joined with ` + ` (tui.rs's `draw_alarm`); `None` when clear or acked.
    pub fn banner_text(&self) -> Option<String> {
        let titles: Vec<String> = self
            .store
            .alerts()
            .active()
            .filter(|(id, a)| a.severity == Severity::Crit && !self.acked.contains(id))
            .map(|(_, a)| a.title.to_string())
            .collect();
        if titles.is_empty() {
            None
        } else {
            Some(format!(
                "⚠ ALERT: {} ⚠  ·  a to acknowledge",
                titles.join(" + ")
            ))
        }
    }

    /// Warn-only active alerts (an advisory imbalance) get a chip in the key
    /// bar, never the banner.
    fn warn_count(&self) -> usize {
        self.store
            .alerts()
            .active()
            .filter(|(_, a)| a.severity == Severity::Warn)
            .count()
    }

    pub fn alerts_overlay_open(&self) -> bool {
        self.alerts_overlay
    }

    fn instance_key(&self, target: &PlaceTarget) -> String {
        match target {
            PlaceTarget::Id(id) => id.clone(),
            PlaceTarget::Kind(k) => format!("kind:{k}"),
        }
    }

    /// Any source generation moved since the last frame → redraw (§5). Over
    /// the store's sources rather than the registry's: the store is the one
    /// list that also holds the journal source under `--replay`, and it needs
    /// no registry handle here.
    pub fn data_dirty(&mut self) -> bool {
        let mut dirty = false;
        let gens: Vec<(&'static str, u64)> = self
            .store
            .sources()
            .map(|s| (s.id.0, s.generation))
            .collect();
        for (id, g) in gens {
            let e = self.last_gens.entry(id).or_insert(0);
            if *e != g {
                *e = g;
                dirty = true;
            }
        }
        dirty
    }

    /// Write per-source Demand from the current page (§5, P4/P21).
    fn update_demand(&mut self) {
        let mut want: BTreeMap<&'static str, (Level, Detail)> = BTreeMap::new();
        if !self.paused
            && self.terminal_focused
            && let Some(solved) = &self.last_solved
        {
            let page = &self.pages[self.page];
            for cell in &solved.cells {
                let Some(placement) = page.place.get(cell.index) else {
                    continue;
                };
                let key = self.instance_key(&placement.target);
                let Some(inst) = self.instances.get(&key) else {
                    continue;
                };
                let Some(component) = &inst.component else {
                    continue;
                };
                let inner = Size::new(cell.inner.width, cell.inner.height);
                let (tier, _) = pick_tier(
                    component.tiers(),
                    inner,
                    self.zoom == Some(cell.index),
                    placement.view.as_deref(),
                );
                let detail = component.demand(tier);
                let focused_here = self.focus == Some(cell.index);
                let level = if focused_here {
                    Level::Focused
                } else {
                    Level::Visible
                };
                let m = component.manifest();
                for src in m.sources.iter().chain(m.optional_sources) {
                    let e = want.entry(src.0).or_insert((Level::Hidden, Detail::Meters));
                    e.0 = e.0.max(level);
                    e.1 = e.1.max(detail);
                }
            }
        }
        for (name, demand) in &self.demands {
            let (level, detail) = want.get(name).copied().unwrap_or((
                if self.paused {
                    Level::Paused
                } else {
                    Level::Hidden
                },
                Detail::Meters,
            ));
            let level = if !self.terminal_focused {
                Level::Hidden.min(level)
            } else {
                level
            };
            demand.set(level, detail);
        }
    }

    /// Seam 5 (§5): a visible, drawn tile whose `Animated` tick returned
    /// `Redraw::Yes` this frame, or the effects/ambient layers (arc 4b).
    pub fn animated_visible(&self) -> bool {
        self.effects_running() || self.animated_tile_fps().is_some()
    }

    /// The highest fps among the animating tiles, capped by `fps_max`.
    pub fn animated_tile_fps(&self) -> Option<u16> {
        self.instances
            .values()
            .filter(|i| i.animated)
            .map(|i| u16::from(i.anim_fps).max(1))
            .max()
            .map(|f| f.min(self.fps_max.max(1)))
    }

    pub fn effective_fps(&self) -> u16 {
        if !self.terminal_focused {
            return self.unfocused_fps.max(1);
        }
        let layers = match &self.ambient {
            // The rain runs at its own (governed) fps, not the config's.
            Some(a) if self.ambient_active() => u16::from(a.fps()),
            // An event effect gets the full rate for its ≤ 600 ms; the
            // alert pulse alone is drawn at PULSE_FPS (P1 during an alert).
            _ if self.effects.running_event() => self.fps,
            _ if self.effects.running() => crate::effects::PULSE_FPS,
            _ => self.fps,
        };
        // An animating tile raises the rate to what it asks for (≤ fps_max);
        // nothing here lowers what a layer already needs.
        match self.animated_tile_fps() {
            Some(t) if self.effects_running() || self.ambient_active() => layers.max(t),
            Some(t) => t,
            None => layers,
        }
    }

    fn title_line(&self, text: &str, focused: bool) -> RLine<'static> {
        let t = &self.theme;
        let text = if t.title.upper {
            text.to_uppercase()
        } else {
            text.to_string()
        };
        let base = if focused {
            t.style(Role::BorderFocused)
        } else {
            t.style(Role::Title)
        };
        let base = if t.title.bold {
            base.add_modifier(Modifier::BOLD)
        } else {
            base
        };
        match t.title.style {
            TitleStyle::Plain => RLine::from(RSpan::styled(format!(" {text} "), base)),
            TitleStyle::Gradient => {
                let g = t.gradient(gridwatch_ui::GradientId::Title);
                let n = text.chars().count().max(1);
                let mut spans = vec![RSpan::styled(" ".to_string(), base)];
                for (i, ch) in text.chars().enumerate() {
                    let mut st = base.fg(g.sample(i as f32 / n as f32));
                    if t.title.bold {
                        st = st.add_modifier(Modifier::BOLD);
                    }
                    spans.push(RSpan::styled(ch.to_string(), st));
                }
                spans.push(RSpan::styled(" ".to_string(), base));
                RLine::from(spans)
            }
        }
    }

    /// The whole frame (§5): Bg first, tab bar, tiles, status bar, overlays.
    pub fn draw_frame(&mut self, area: Rect, buf: &mut Buffer) {
        self.frame += 1;
        self.last_area = area;
        // Seam 5: only the tiles drawn this frame may report animation.
        for i in self.instances.values_mut() {
            i.animated = false;
        }
        self.check_recorder();
        let t_bg = self.theme.color(Role::Bg);
        for y in area.y..area.y + area.height {
            for x in area.x..area.x + area.width {
                if let Some(cell) = buf.cell_mut((x, y)) {
                    cell.set_char(' ');
                    cell.set_bg(t_bg);
                    cell.set_fg(self.theme.color(Role::Text));
                }
            }
        }
        // §6's too-small notice: the overlay existed but nothing called it, so
        // an undersized terminal showed a blank screen and no reason for it.
        // Derived from what one tile needs, not a constant: the grid's minimum
        // inner unit plus its border, plus the two chrome rows. Below this the
        // body cannot hold a single tile, so the frame would be chrome around
        // nothing — which the D46 lattice lint found at 20×3.
        let (min_w, min_h) = (
            self.grid.min_unit_inner.w + 2,
            self.grid.min_unit_inner.h + 2 + CHROME_ROWS,
        );
        if area.height < min_h || area.width < min_w {
            overlay::too_small(
                area.width,
                area.height,
                min_w,
                min_h,
                area,
                &self.theme,
                buf,
            );
            return;
        }
        // Mode from the terminal size (§6).
        let requested = derive_mode(
            Size::new(area.width, area.height),
            &self.grid,
            CHROME_ROWS,
            self.mode,
        );
        self.mode = if self.dense_override && requested == SolveMode::Configured {
            SolveMode::Dense
        } else {
            requested
        };

        // Tab bar.
        let mut tabs: Vec<RSpan> = vec![RSpan::styled(
            " gridwatch ",
            self.theme
                .style(Role::AccentPrimary)
                .add_modifier(Modifier::BOLD),
        )];
        for (i, p) in self.pages.iter().enumerate() {
            let style = if i == self.page {
                self.theme
                    .style(Role::Title)
                    .add_modifier(Modifier::BOLD | Modifier::REVERSED)
            } else {
                self.theme.style(Role::TextMuted)
            };
            let key = p.hotkey.map(|h| format!("{h} ")).unwrap_or_default();
            tabs.push(RSpan::styled(format!(" {key}{} ", p.name), style));
        }
        tabs.push(RSpan::styled(
            format!("  {} · {:?}", self.theme.name, self.mode).to_lowercase(),
            self.theme.style(Role::TextGhost),
        ));
        let show_tabs = self.mode != SolveMode::Dense; // §6: dense hides the tab bar
        if show_tabs {
            RLine::from(tabs).render(
                Rect {
                    x: area.x,
                    y: area.y,
                    width: area.width,
                    height: 1,
                },
                buf,
            );
        }

        // The alert banner (§4.4): one row under the tab bar on every page
        // while a Crit alert is active and unacknowledged, pulsing on the
        // even seconds of the store clock — no SLOW_BLINK, one row per second.
        let mut top = u16::from(show_tabs);
        // Never at the cost of the only tile: below one tile plus a row the
        // banner yields (the lattice found a 15×7 body reduced to a frame).
        if let Some(text) = self.banner_text()
            && area.height > min_h
        {
            // The heartbeat reverse, unless the theme declares an alert
            // effect (D54 seam 6: then the tachyonfx pulse is the pulse).
            let pulse_on = !(self.effects.enabled() && self.theme.effects.alert.is_some())
                && (self.clock.now().0 / 1_000_000_000).is_multiple_of(2);
            overlay::banner(
                &text,
                pulse_on,
                Rect {
                    x: area.x,
                    y: area.y + top,
                    width: area.width,
                    height: 1,
                },
                &self.theme,
                buf,
            );
            top += 1;
        }
        // Body.
        let body = Rect {
            x: area.x,
            y: area.y + top,
            width: area.width,
            height: area.height - 1 - top,
        };
        let page = self.pages[self.page].clone();
        let solved = solve(
            &self.grid,
            &page,
            body,
            self.mode,
            self.zoom,
            self.stack_scroll,
        );
        if self.mode == SolveMode::Stack
            && self.stack_scroll > 0
            && let Some(max_b) = solved.cells.iter().map(|c| c.outer.bottom()).max()
            && max_b < body.bottom()
        {
            // Content ends above the body bottom: pull the scroll back next frame.
            self.stack_scroll = self.stack_scroll.saturating_sub(body.bottom() - max_b);
        }
        for cell in &solved.cells {
            self.draw_cell(&page, cell, buf);
        }
        // Flourishes in the empty units (seam 7): after the tiles, never over one.
        crate::flourish::draw(&self.theme, &self.grid, &page, body, self.mode, buf);
        self.last_solved = Some(solved);
        self.last_body = body;
        self.update_demand();
        if self.edit.is_some() {
            self.draw_edit_chrome(body, buf);
        }

        // Status bar. A pending action takes it over: the question is the
        // most important thing on the screen while it stands.
        let mut hints = if let Some((_, _, question)) = &self.pending_action {
            format!("{question}   y confirm · any other key cancels")
        } else if let Some(ed) = &self.edit {
            self.edit_hints(ed)
        } else if self.captured {
            // §10: the captured component's keys replace the status bar.
            let keys = self
                .focus
                .and_then(|f| self.pages[self.page].place.get(f))
                .map(|pl| self.instance_key(&pl.target))
                .and_then(|k| self.instances.get(&k))
                .and_then(|i| i.component.as_ref())
                .map(|c| {
                    c.manifest()
                        .keys
                        .iter()
                        .map(|k| format!("{} {}", k.key, k.does))
                        .collect::<Vec<_>>()
                        .join(" · ")
                })
                .unwrap_or_default();
            if keys.is_empty() {
                "Esc release · component keys active".to_string()
            } else {
                format!("Esc release · {keys}")
            }
        } else {
            "q quit · ? help · [ ] pages · hjkl focus · Enter capture · z zoom · d dense · t theme · T reload · space pause · a ack · A alerts · S shot · F12 hud"
                .to_string()
        };
        let warns = self.warn_count();
        if warns > 0 {
            hints = format!("▲ {warns} advisory · {hints}");
        }
        let status = Rect {
            x: area.x,
            y: area.y + area.height - 1,
            width: area.width,
            height: 1,
        };
        let muted = self.theme.style(Role::TextMuted);
        let status_line = match hints.strip_prefix("EDIT") {
            // The mode token stands out (review): bold + reversed, roles only.
            Some(rest) if self.edit.is_some() => RLine::from(vec![
                RSpan::styled(" ", muted),
                RSpan::styled(
                    "EDIT",
                    self.theme
                        .style(Role::AccentPrimary)
                        .add_modifier(Modifier::BOLD | Modifier::REVERSED),
                ),
                RSpan::styled(rest.to_string(), muted),
            ]),
            _ => RLine::from(RSpan::styled(format!(" {hints}"), muted)),
        };
        status_line.render(status, buf);

        // Overlays. Toasts yield on a body too small to share (the lattice
        // found them covering the only tile at 15×7).
        let now = self.now();
        self.toasts.retain(|t| t.expires_at > now);
        let toast_rows = if body.height >= 6 {
            self.toasts.len()
        } else {
            0
        };
        for (i, t) in self.toasts.iter().take(toast_rows).enumerate() {
            let (style, glyph) = if t.resolved {
                (self.theme.style(Role::Ok).add_modifier(Modifier::BOLD), "✓")
            } else {
                self.theme.severity(t.severity)
            };
            let mut text = format!(" {glyph} {} ", t.text);
            let max = usize::from(body.width.saturating_sub(2));
            let n = text.chars().count();
            if n > max && max > 4 {
                // Keep the tail: `file:line:col` leads, the reason ends it,
                // and the reason is what the user needs (review).
                let keep: String = text.chars().skip(n - (max - 2)).collect();
                text = format!(" …{keep}");
            }
            let w = text.chars().count() as u16;
            let y = body.y + body.height.saturating_sub(2 + i as u16);
            let x = body.x + body.width.saturating_sub(w + 1);
            buf.set_string(x, y, &text, style);
        }
        if self.help {
            overlay::help(
                &[
                    ("q / ^q", "quit"),
                    ("1-9 [ ]", "pages"),
                    ("hjkl / arrows", "move focus"),
                    ("Enter / Esc", "capture / release keys"),
                    ("z", "zoom tile"),
                    ("d", "dense mode"),
                    ("t / T", "cycle / reload theme"),
                    ("space", "pause sources"),
                    ("a", "acknowledge the alert banner"),
                    ("A", "alerts: active list and log"),
                    ("V / L", "matrix: re-light the page / lock it lit"),
                    ("e / Esc", "enter / leave edit mode"),
                    ("edit HJKL", "move a unit; ^hjkl widen/narrow/grow/shrink"),
                    ("edit s S", "cycle footprint; S+dir swaps with neighbour"),
                    ("edit a x", "add a tile (picker); remove (also Delete)"),
                    ("edit u ^r w", "undo; redo; save layout.toml (y discards)"),
                    ("S", "screenshot to state dir"),
                    ("F12", "stats HUD"),
                ],
                body,
                &self.theme,
                buf,
            );
        }
        if self.alerts_overlay {
            let now = self.clock.now();
            let mut lines =
                gridwatch_components::alerts::active_lines(&self.store, now, body.width);
            lines.push(vec![RSpanText::bold(Role::TextMuted, "log")]);
            let rows = usize::from(body.height.saturating_sub(lines.len() as u16 + 6));
            lines.extend(gridwatch_components::alerts::log_lines(
                &self.store,
                0,
                rows,
            ));
            overlay::panel("alerts  ·  Esc to close", &lines, body, &self.theme, buf);
        }
        if let Some(pk) = self.edit.as_ref().and_then(|e| e.picker.as_ref()) {
            let mut lines: Vec<Vec<RSpanText>> = vec![vec![RSpanText::new(
                Role::TextMuted,
                format!("filter: {}▏", pk.filter),
            )]];
            // A window around the cursor: the panel has body.height - 6 rows
            // for items, and the cursor stays inside it (review).
            let visible = pk.visible();
            // Title, filter, padding and the two "more" lines take 8 rows.
            let rows = usize::from(body.height.saturating_sub(8)).max(1);
            let first = pk
                .cursor
                .saturating_sub(rows - 1)
                .min(visible.len().saturating_sub(rows));
            let end = (first + rows).min(visible.len());
            let shown = &visible[first.min(visible.len())..end];
            if first > 0 {
                lines.push(vec![RSpanText::new(
                    Role::TextGhost,
                    format!("  … {first} more above"),
                )]);
            }
            for (i, item) in shown.iter().enumerate() {
                let idx = first + i;
                let role = if idx == pk.cursor {
                    Role::AccentPrimary
                } else {
                    Role::Text
                };
                let mark = if idx == pk.cursor { "▶ " } else { "  " };
                lines.push(vec![RSpanText::new(role, format!("{mark}{}", item.label))]);
            }
            let rest = visible.len().saturating_sub(end);
            if rest > 0 {
                lines.push(vec![RSpanText::new(
                    Role::TextGhost,
                    format!("  … {rest} more below"),
                )]);
            }
            if visible.is_empty() {
                lines.push(vec![RSpanText::new(Role::TextGhost, "  (nothing matches)")]);
            }
            overlay::panel(
                "add a tile  ·  ↑/↓ move · type to filter · Enter add · Esc close",
                &lines,
                body,
                &self.theme,
                buf,
            );
        }
        // The effects layer (§7, arc 4b): hooks fire on transitions the
        // shell sees — first frame, a focus move, the banner appearing — and
        // the painter runs over the finished frame, before the HUD.
        if self.effects.enabled() {
            if !self.fx_started {
                self.fx_started = true;
                self.effects
                    .trigger(Hook::Startup, &self.theme.effects, &self.theme, body, None);
            }
            if self.focus != self.fx_focus {
                self.fx_focus = self.focus;
                let outer = self.last_solved.as_ref().and_then(|s| {
                    self.focus
                        .and_then(|f| s.cells.iter().find(|c| c.index == f).map(|c| c.outer))
                });
                if let Some(outer) = outer {
                    self.effects.trigger(
                        Hook::Focus,
                        &self.theme.effects,
                        &self.theme,
                        outer,
                        None,
                    );
                }
            }
            let banner_now = self.banner_text().is_some() && area.height > min_h;
            if banner_now && !self.fx_banner {
                let row = Rect {
                    x: area.x,
                    y: area.y + u16::from(show_tabs),
                    width: area.width,
                    height: 1,
                };
                self.effects
                    .trigger(Hook::Alert, &self.theme.effects, &self.theme, row, None);
            } else if !banner_now && self.fx_banner {
                self.effects.cancel(Hook::Alert);
            }
            self.fx_banner = banner_now;
        }
        let clock_now = Duration::from_nanos(self.clock.now().0);
        self.effects.paint(buf, clock_now);
        self.stats.fx_us = self.effects.last_cost_us();
        self.stats.rain_step = self.ambient.as_ref().map(|a| a.governor.step).unwrap_or(0);
        if let Some(n) = self.effects.notice.take() {
            tracing::warn!("{n}");
            self.toast(Severity::Warn, n);
        }
        // The ambient layer (D34): the frame so far is the mold; the rain
        // writes the screen in place. Pinned: the tab bar, the focused
        // tile, tiles whose sources have an active Warn/Crit alert, the
        // banner, the toasts, the key bar, the hovered tile, and the page
        // under an overlay or in edit mode (per the theme's `reveal` list
        // for focus/alert/hover). Frozen (unfocused, paused): the rain stands
        // still and the same state is drawn, so toasts and a new banner show.
        if self.ambient.is_some() {
            let active = self.ambient_active();
            let reveals = |what: &str| self.ambient.as_ref().is_some_and(|a| a.reveals(what));
            let mut pinned: Vec<Rect> = vec![status];
            if show_tabs {
                pinned.push(Rect {
                    x: area.x,
                    y: area.y,
                    width: area.width,
                    height: 1,
                });
            }
            if self.banner_text().is_some() && area.height > min_h {
                pinned.push(Rect {
                    x: area.x,
                    y: area.y + u16::from(show_tabs),
                    width: area.width,
                    height: 1,
                });
            }
            if toast_rows > 0 {
                pinned.push(Rect {
                    x: body.x,
                    y: body.y + body.height.saturating_sub(1 + toast_rows as u16),
                    width: body.width,
                    height: toast_rows as u16,
                });
            }
            // An alert id is `<source>/<condition>` (D50 §2): the prefix
            // names the source whose tiles are pinned.
            let alerting: std::collections::BTreeSet<String> = self
                .store
                .alerts()
                .active()
                .filter(|(_, a)| a.severity != Severity::Info)
                .map(|(id, _)| id.0.split('/').next().unwrap_or("").to_string())
                .collect();
            let mut tiles: Vec<Rect> = Vec::new();
            if let Some(solved) = &self.last_solved {
                for cell in &solved.cells {
                    tiles.push(cell.outer);
                    let Some(pl) = page.place.get(cell.index) else {
                        continue;
                    };
                    let key = self.instance_key(&pl.target);
                    let alert_tile = reveals("alert")
                        && self
                            .instances
                            .get(&key)
                            .and_then(|i| i.component.as_ref())
                            .is_some_and(|c| {
                                let m = c.manifest();
                                m.sources
                                    .iter()
                                    .chain(m.optional_sources)
                                    .any(|s| alerting.contains(s.0))
                            });
                    let focused_tile = reveals("focus") && self.focus == Some(cell.index);
                    let hovered = reveals("hover") && self.hover == Some(cell.index);
                    if alert_tile || focused_tile || hovered {
                        pinned.push(cell.outer);
                    }
                }
            }
            if self.help || self.alerts_overlay || self.edit.is_some() {
                // Overlays and edit mode are read, not rained on.
                pinned.push(body);
            }
            // The governor's readings, once per second: the last two
            // seconds' frame p95 and the bytes written in the last second.
            let bytes_now = self
                .bytes_counter
                .as_ref()
                .map(|c| c.load(Ordering::Relaxed))
                .unwrap_or(0);
            let (mark_t, mark_b) = self.bytes_mark;
            let reading = if mark_t.elapsed() >= Duration::from_secs(1) {
                let bps = bytes_now.saturating_sub(mark_b);
                self.bytes_mark = (Instant::now(), bytes_now);
                let fps = self.effective_fps().max(1);
                Some((self.stats.p95_recent_us(usize::from(fps) * 2), bps))
            } else {
                None
            };
            if let Some(amb) = self.ambient.as_mut() {
                if active
                    && amb.spec.governor
                    && let Some((p95, bps)) = reading
                {
                    amb.governor.observe(p95, bps);
                }
                amb.frame(
                    buf,
                    Pins {
                        pinned: &pinned,
                        tiles: &tiles,
                    },
                    active,
                );
            }
        }
        if self.hud {
            let bytes = self
                .bytes_counter
                .as_ref()
                .map(|c| c.load(Ordering::Relaxed))
                .unwrap_or(0);
            let stats = overlay::HudStats {
                first_frame_ms: self.stats.first_frame_ms,
                sources_live_ms: self.stats.sources_live_ms,
                frame_p50_us: self.stats.p50_us(),
                frame_p95_us: self.stats.p95_us(),
                changed_cells: self.stats.changed_cells,
                bytes_written: bytes,
                frames: self.stats.frames,
                redraw_data: self.stats.redraw_data,
                redraw_anim: self.stats.redraw_anim,
                redraw_heartbeat: self.stats.redraw_heartbeat,
                mode: match self.mode {
                    SolveMode::Configured => "configured",
                    SolveMode::Dense => "dense",
                    SolveMode::Stack => "stack",
                },
                recording: self.recorder.as_ref().map(|r| (r.written(), r.dropped())),
            };
            overlay::hud(&stats, body, &self.theme, buf);
            // The rain and effects line, right under the HUD box's rows and
            // right-aligned with it.
            let line = self
                .ambient_hud()
                .map(|l| format!(" {l} · fx {} µs ", self.effects.last_cost_us()))
                .unwrap_or_else(|| format!(" fx {} µs ", self.effects.last_cost_us()));
            let w = line.chars().count() as u16;
            let x = body.x + body.width.saturating_sub(w + 1);
            let y = body.y + 8;
            for dx in 0..w {
                if let Some(c) = buf.cell_mut((x + dx, y)) {
                    c.set_char(' ');
                    c.set_style(
                        self.theme
                            .style(Role::Text)
                            .bg(self.theme.color(Role::Panel)),
                    );
                }
            }
            buf.set_string(x, y, &line, self.theme.style(Role::Text));
        }
        // Changed-cell accounting (own diff): only when the HUD or a stats
        // log consumes it — a full clone + compare per frame is not free (P19).
        if self.hud || self.stats_log.is_some() {
            if let Some(prev) = &self.prev_buf
                && prev.area() == buf.area()
            {
                let changed = prev
                    .content()
                    .iter()
                    .zip(buf.content())
                    .filter(|(a, b)| a != b)
                    .count();
                self.stats.changed_cells = changed as u64;
            }
            self.prev_buf = Some(buf.clone());
        } else {
            self.prev_buf = None;
        }
    }

    fn draw_cell(&mut self, page: &Page, cell: &Cell, buf: &mut Buffer) {
        let placement = &page.place[cell.index];
        let key = self.instance_key(&placement.target);
        let focused =
            self.focus == Some(cell.index) && self.zoom.is_none() || self.zoom == Some(cell.index);
        let theme_name = self.theme.name.clone();
        let now = self.store.latest();
        // Ticks run on the run clock (virtual under replay), so an animated
        // tile's ballistics advance at its fps, not at the batch cadence
        // (review, D55 amendment 14); the render context keeps the store's
        // time for ages and wall-clock text.
        let tick_now = self.now();
        let wall = SystemTime::UNIX_EPOCH + Duration::from_nanos(wall_ns(&self.clock));

        // An id no [[components]] entry defines: chip, never a silent hole (§6).
        if !self.instances.contains_key(&key) {
            if let Some(block) =
                self.theme
                    .block(focused, self.mode == SolveMode::Dense, Chrome::Themed)
            {
                block
                    .title(self.title_line(&key, focused))
                    .render(cell.outer, buf);
            }
            if cell.inner.width > 0 && cell.inner.height > 0 {
                overlay::chip(
                    &key,
                    "not in config.toml",
                    "add a [[components]] entry with this id",
                    cell.inner,
                    &self.theme,
                    buf,
                );
            }
            return;
        }

        // Chrome first (the shell owns the frame — §4.6); title() is contained (§11).
        let mut title_panicked = false;
        let (kind, chrome, title) = {
            let inst = &self.instances[&key];
            let tick_cx = gridwatch_ui::component::TickCx {
                store: &self.store,
                now: tick_now,
                visible: true,
                tier: 0,
            };
            let chrome = inst
                .component
                .as_ref()
                .map(|c| c.manifest().chrome)
                .unwrap_or(Chrome::Themed);
            let title = match inst.component.as_ref() {
                Some(c) => {
                    crate::terminal::CONTAINED.with(|f| f.set(true));
                    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        c.title(cell.outer.width.saturating_sub(4), &tick_cx)
                            .into_owned()
                    }));
                    crate::terminal::CONTAINED.with(|f| f.set(false));
                    match r {
                        Ok(t) => t,
                        Err(_) => {
                            title_panicked = true;
                            inst.kind.clone()
                        }
                    }
                }
                None => inst.kind.clone(),
            };
            (inst.kind.clone(), chrome, title)
        };
        if title_panicked {
            self.disable_instance(&key);
        }
        if let Some(block) = self
            .theme
            .block(focused, self.mode == SolveMode::Dense, chrome)
        {
            let block = block.title(self.title_line(&title, focused));
            block.render(cell.outer, buf);
        }

        let inner = cell.inner;
        if inner.width == 0 || inner.height == 0 {
            return;
        }
        let zoomed = self.zoom == Some(cell.index);
        let dense = self.mode == SolveMode::Dense;
        let captured = self.captured && focused;
        let view_pref = placement.view.clone();

        let Some(inst) = self.instances.get_mut(&key) else {
            return;
        };
        let component = match &mut inst.component {
            None => {
                overlay::chip(
                    &kind,
                    &inst.chip_reason,
                    &inst.chip_hint,
                    inner,
                    &self.theme,
                    buf,
                );
                return;
            }
            Some(c) => c,
        };
        if cell.chip {
            overlay::chip(&kind, "starved", "", inner, &self.theme, buf);
            return;
        }
        let inner_size = Size::new(inner.width, inner.height);
        let (tier, fallback) =
            pick_tier(component.tiers(), inner_size, zoomed, view_pref.as_deref());

        // Tick + view every frame, contained (§11). The view's fingerprint is
        // the cache key's backstop (§5): a component whose data changed but
        // whose tree did not is not re-rendered; one whose tree changed for a
        // reason no generation captures (age columns, the wall clock) is.
        let local = Rect {
            x: 0,
            y: 0,
            width: inner.width,
            height: inner.height,
        };
        let render_cx = gridwatch_ui::component::RenderCx {
            inner: local,
            tier,
            view_fallback: fallback,
            focused,
            captured,
            zoomed,
            dense,
            store: &self.store,
            theme: self.theme.for_kind(&kind),
            now,
            wall,
            tz_offset_s: self.tz_offset_s,
            frame: self.frame,
        };
        crate::terminal::CONTAINED.with(|f| f.set(true));
        let viewed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let tick_cx = gridwatch_ui::component::TickCx {
                store: &self.store,
                now: tick_now,
                visible: true,
                tier,
            };
            let redraw = component.tick(&tick_cx);
            let anim = match component.redraw_policy() {
                gridwatch_ui::component::RedrawPolicy::Animated { fps }
                    if redraw == gridwatch_ui::component::Redraw::Yes =>
                {
                    Some(fps)
                }
                _ => None,
            };
            (anim, component.view(&render_cx))
        }));
        crate::terminal::CONTAINED.with(|f| f.set(false));
        let (anim, view) = match viewed {
            Ok(v) => v,
            Err(_) => {
                self.disable_instance(&key);
                overlay::chip(&kind, "panicked — see the log", "", inner, &self.theme, buf);
                return;
            }
        };
        let m = component.manifest();
        // Seam 5: the tile's animation state for this frame (read by the
        // frame loop through `animated_visible`/`effective_fps`).
        inst.animated = anim.is_some();
        inst.anim_fps = anim.unwrap_or(0);
        let frame = self.frame;
        let gens: Vec<u64> = m
            .sources
            .iter()
            .chain(m.optional_sources)
            .map(|s| self.store.generation(*s))
            .collect();
        // §5: the animation-frame term — `self.frame` while the tile is
        // `Animated` and moving, else 0 — so an animated tile re-renders
        // every frame and a quiet one never does; the fingerprint stays the
        // backstop. The fingerprint serialises the whole tree (0.11 ms at
        // the table tier), so it is only computed when the cheap terms
        // already match: a moved generation forces the render regardless.
        let anim_term = if anim.is_some() { frame } else { 0 };
        let cheap_match = self.cache.get(&cell.index).is_some_and(|(k, _)| {
            k.gens == gens
                && k.anim == anim_term
                && k.tier == tier
                && k.w == inner.width
                && k.h == inner.height
                && k.theme == theme_name
                && k.focused == focused
                && k.zoomed == zoomed
        });
        let view_hash = if cheap_match {
            gridwatch_ui::view::fingerprint(&view)
        } else {
            0
        };
        let cache_key = CacheKey {
            gens,
            anim: anim_term,
            tier,
            w: inner.width,
            h: inner.height,
            theme: theme_name,
            focused,
            zoomed,
            view_hash,
        };
        let needs_render = !cheap_match
            || self
                .cache
                .get(&cell.index)
                .map(|(k, _)| k.view_hash != view_hash)
                .unwrap_or(true);
        // A rendered tile always stores its real fingerprint, so the next
        // frame's cheap match has something honest to compare against.
        let cache_key = if needs_render && !cheap_match {
            CacheKey {
                view_hash: gridwatch_ui::view::fingerprint(&view),
                ..cache_key
            }
        } else {
            cache_key
        };
        if needs_render {
            let mut tile = Buffer::empty(local);
            crate::terminal::CONTAINED.with(|f| f.set(true));
            let kind_theme = self.theme.for_kind(&kind);
            let ok = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                kind_theme
                    .renderer()
                    .render(&view, local, kind_theme, &mut tile);
            }))
            .is_ok();
            crate::terminal::CONTAINED.with(|f| f.set(false));
            if !ok {
                self.disable_instance(&key);
                overlay::chip(&kind, "panicked — see the log", "", inner, &self.theme, buf);
                return;
            }
            self.cache.insert(cell.index, (cache_key, tile));
        }
        if let Some((_, tile)) = self.cache.get(&cell.index) {
            blit(tile, inner, buf);
        }
        // Staleness (§11, seam 10): a post-render pass over the tile rect,
        // never inside the component (which does not know cadences) and never
        // in the cache (the tile itself is fine; its age is not).
        if let Some(age) = self.stale_age(m) {
            overlay::dim(inner, &self.theme, buf);
            // On the frame's top border when there is one (the shell owns
            // the chrome), so the tile's own top-right value survives
            // (review: it clobbered `16.7G/91.0G`); on the inner row when
            // the tile is borderless.
            let row = if chrome == Chrome::Themed && cell.outer.height > 0 && cell.outer.width > 2 {
                Rect {
                    x: cell.outer.x + 1,
                    y: cell.outer.y,
                    width: cell.outer.width - 2,
                    height: 1,
                }
            } else {
                inner
            };
            overlay::stale_badge(age.as_secs(), row, &self.theme, buf);
        }
        if fallback {
            // After the blit so it survives (§4.6): the pinned view does not fit.
            let marker = "view↓";
            let x = inner.x + inner.width.saturating_sub(5);
            buf.set_string(x, inner.y, marker, self.theme.style(Role::TextGhost));
        }
    }

    /// The age of the oldest required source's last sample when any of them
    /// is past `STALE_CADENCES` × its visible cadence; `None` while paused
    /// (the pause is deliberate and the key bar says so), for a source that
    /// has not sampled yet (that is `Starting`, on the chip and the sources
    /// tile), or when everything is fresh.
    fn stale_age(&self, m: &gridwatch_ui::Manifest) -> Option<Duration> {
        if self.paused {
            return None;
        }
        let mut now = self.clock.now();
        if let Some(ended) = self.journal_ended
            && self.age_after_journal
        {
            // The journal drove the virtual clock and has stopped: the
            // dashboard is frozen, and its age is real time since then.
            now = Ts(now.0 + ended.elapsed().as_nanos() as u64);
        }
        let mut worst: Option<Duration> = None;
        for src in m.sources {
            let Some(last) = self.store.last_sample(*src) else {
                continue;
            };
            // A source that is not `Ok` says why on the sources tile and in
            // the tile itself (3a); a badge over a `Degraded` backoff would
            // only repeat it. A source parked by a pause or a lost focus
            // has not had its turn yet.
            if self.store.status(*src).state != gridwatch_store::SourceState::Ok
                || self.resumed_at.is_some_and(|r| last < r)
            {
                continue;
            }
            let Some(cadence) = self.cadence_of(src.0) else {
                continue;
            };
            let age = now.since(last);
            if age > cadence * STALE_CADENCES && worst.is_none_or(|w| age > w) {
                worst = Some(age);
            }
        }
        worst
    }

    /// The cadence a source runs at (D53): `[sources.<id>] refresh_ms` /
    /// `interval_ms` when configured (never below the source's focused
    /// cadence, its fastest), the pins source's live `pins.info.interval_ms`
    /// (`+`/`-` change it), else the registry's visible cadence.
    fn cadence_of(&self, id: &str) -> Option<Duration> {
        let (visible, focused) = *self.cadences.get(id)?;
        let configured = self
            .source_options
            .get(id)
            .and_then(|t| t.get("refresh_ms").or_else(|| t.get("interval_ms")))
            .and_then(|v| v.as_integer())
            .map(|ms| Duration::from_millis(ms.max(0) as u64).max(focused));
        let live = (id == gridwatch_store::keys::pins::SOURCE.0)
            .then(|| {
                self.store
                    .record(&gridwatch_store::keys::pins::INFO)
                    .map(|(_, i)| Duration::from_millis(u64::from(i.interval_ms)))
            })
            .flatten();
        if id == gridwatch_store::keys::audio::SOURCE.0 {
            // Arc 5: the audio source runs at `[sources.audio] fps` while
            // there is sound and at 2 Hz under the silence rule — a silent
            // sink is not a stale one.
            let fps = self
                .source_options
                .get(id)
                .and_then(|t| t.get("fps"))
                .and_then(|v| v.as_integer())
                .map(|f| Duration::from_millis(1000 / (f.clamp(5, 60) as u64)))
                .unwrap_or(visible);
            let silent = self
                .store
                .record(&gridwatch_store::keys::audio::LEVEL)
                .is_some_and(|(_, l)| l.silent);
            // Silent: 3 × max(period, 1 s) — one dropped 500 ms tick must
            // not flicker the badge (review).
            return Some(if silent {
                fps.max(Duration::from_secs(1))
            } else {
                fps
            });
        }
        Some(live.or(configured).unwrap_or(visible))
    }

    fn disable_instance(&mut self, key: &str) {
        if let Some(i) = self.instances.get_mut(key) {
            i.component = None;
            i.chip_reason = "panicked — see the log".into();
            i.chip_hint = String::new();
        }
    }

    pub fn handle_input(&mut self, ev: InputEvent) -> bool {
        match ev {
            InputEvent::FocusGained => {
                self.terminal_focused = true;
                self.resumed_at = Some(self.clock.now());
                true
            }
            InputEvent::FocusLost => {
                self.terminal_focused = false;
                self.update_demand();
                true
            }
            InputEvent::Resize(_, _) => {
                self.cache.clear();
                self.prev_buf = None;
                // The banner row moves: the pulse re-triggers there; the rain
                // re-prints the new page (review).
                self.effects.cancel(Hook::Alert);
                self.fx_banner = false;
                if let Some(a) = self.ambient.as_mut() {
                    a.request_sweep();
                }
                true
            }
            InputEvent::Paste(_) => false,
            InputEvent::Mouse(m) => self.handle_mouse(m),
            InputEvent::Key(k) => self.handle_key(k),
        }
    }

    fn handle_mouse(&mut self, m: gridwatch_store::MouseEvent) -> bool {
        use gridwatch_store::MouseKind::*;
        // `reveal = ["hover"]` (D31): the tile under the pointer stays lit.
        // Tracked on every event, never consuming it (review: the first
        // click on an unhovered tile was swallowed).
        let mut hover_changed = false;
        if self.ambient.is_some() {
            let over = self.last_solved.as_ref().and_then(|s| hit(s, m.x, m.y));
            if over != self.hover {
                self.hover = over;
                hover_changed = true;
            }
        }
        if hover_changed && matches!(m.kind, Moved) {
            return true;
        }
        if self.edit.is_some() {
            return self.edit_mouse(m);
        }
        match m.kind {
            Down(gridwatch_store::MouseButton::Left) => {
                if let Some(solved) = &self.last_solved
                    && let Some(idx) = hit(solved, m.x, m.y)
                {
                    let dbl = self
                        .last_click
                        .is_some_and(|(t, i)| i == idx && t.elapsed() < Duration::from_millis(400));
                    if dbl {
                        self.zoom = if self.zoom == Some(idx) {
                            None
                        } else {
                            Some(idx)
                        };
                        self.cache.clear();
                    }
                    self.last_click = Some((Instant::now(), idx));
                    self.focus = Some(idx);
                    return true;
                }
                false
            }
            ScrollUp if self.mode == SolveMode::Stack => {
                self.stack_scroll = self.stack_scroll.saturating_sub(2);
                true
            }
            ScrollDown if self.mode == SolveMode::Stack => {
                self.stack_scroll += 2;
                true
            }
            _ => false,
        }
    }

    fn handle_key(&mut self, k: gridwatch_store::KeyEvent) -> bool {
        // `reveal = ["key"]` (D31): a key keeps the focused tile lit for
        // `reveal_ms`.
        if let (Some(a), Some(s), Some(f)) = (self.ambient.as_mut(), &self.last_solved, self.focus)
            && a.reveals("key")
            && let Some(cell) = s.cells.iter().find(|c| c.index == f)
        {
            a.reveal_for(cell.outer);
        }
        if k.mods.ctrl && k.code == KeyCode::Char('q') {
            self.quit = true;
            return true;
        }
        // A question on the screen takes the next key, whatever it is:
        // nothing else should happen while a person is being asked
        // whether to signal a process.
        if self.confirm_key(k) {
            return true;
        }
        if self.edit.is_some() {
            return self.edit_key(k);
        }
        if self.captured {
            // The component sees `Esc` first (a picker closes on it —
            // review: the shell released capture and the picker stayed);
            // only an ignored `Esc` releases the capture.
            if k.code == KeyCode::Esc {
                if !self.forward_key(k) {
                    self.captured = false;
                }
                return true;
            }
            return self.forward_key(k);
        }
        match k.code {
            KeyCode::Char('q') => {
                self.quit = true;
                true
            }
            KeyCode::Char('?') => {
                self.help = !self.help;
                true
            }
            KeyCode::F(12) => {
                self.hud = !self.hud;
                true
            }
            KeyCode::Char(c @ '1'..='9') => {
                if let Some(i) = self.pages.iter().position(|p| p.hotkey == Some(c)) {
                    self.set_page(i);
                }
                true
            }
            KeyCode::Char('[') => {
                let prev = if self.page == 0 {
                    self.pages.len() - 1
                } else {
                    self.page - 1
                };
                self.set_page(prev);
                true
            }
            KeyCode::Char(']') => {
                self.set_page((self.page + 1) % self.pages.len());
                true
            }
            KeyCode::Tab | KeyCode::BackTab => {
                let n = self.pages[self.page].place.len();
                if n > 0 {
                    let cur = self.focus.unwrap_or(0);
                    self.focus = Some(if k.code == KeyCode::Tab {
                        (cur + 1) % n
                    } else {
                        (cur + n - 1) % n
                    });
                }
                true
            }
            KeyCode::Char('h') | KeyCode::Left => self.move_focus(Direction::Left),
            KeyCode::Char('j') | KeyCode::Down => self.move_focus(Direction::Down),
            KeyCode::Char('k') | KeyCode::Up => self.move_focus(Direction::Up),
            KeyCode::Char('l') | KeyCode::Right => self.move_focus(Direction::Right),
            KeyCode::Enter => {
                if self.focused_component_exists() {
                    self.captured = true;
                }
                true
            }
            KeyCode::Esc => {
                if self.alerts_overlay {
                    self.alerts_overlay = false;
                    return true;
                }
                self.zoom = None;
                self.help = false;
                true
            }
            KeyCode::Char('z') => {
                self.zoom = match (self.zoom, self.focus) {
                    (Some(_), _) => None,
                    (None, Some(f)) => Some(f),
                    _ => None,
                };
                self.cache.clear();
                true
            }
            KeyCode::Char('d') => {
                self.dense_override = !self.dense_override;
                self.cache.clear();
                true
            }
            KeyCode::Char('t') => {
                let cur = BUILTIN_THEMES
                    .iter()
                    .position(|n| *n == self.theme.name)
                    .unwrap_or(0);
                let next = BUILTIN_THEMES[(cur + 1) % BUILTIN_THEMES.len()];
                match load_builtin(next, self.theme.mode) {
                    Ok(t) => {
                        self.theme_ref = next.to_string();
                        self.theme_ref_moved();
                        self.swap_theme(t);
                        // Dense mode hides the tab bar, the one place the
                        // name is shown (review): say it.
                        self.toast(Severity::Info, format!("theme: {next}"));
                    }
                    Err(e) => self.toast(Severity::Warn, format!("theme: {e}")),
                }
                true
            }
            KeyCode::Char('T') => {
                self.reload_theme();
                true
            }
            KeyCode::Char(' ') => {
                self.paused = !self.paused;
                if !self.paused {
                    self.resumed_at = Some(self.clock.now());
                }
                self.update_demand();
                self.toast(
                    Severity::Info,
                    if self.paused { "paused" } else { "resumed" },
                );
                true
            }
            KeyCode::Char('S') => {
                let msg = match self.screenshot() {
                    Ok(p) => format!("shot → {p}"),
                    Err(e) => format!("shot failed: {e}"),
                };
                self.toast(Severity::Info, msg);
                true
            }
            KeyCode::Char('r') => {
                self.execute(Command::Record(
                    !self.recorder.as_ref().is_some_and(|r| r.enabled()),
                ));
                true
            }
            KeyCode::Char('a') => {
                // Every active alert (§4.4), not only the Crit ones the banner shows.
                let ids: Vec<gridwatch_store::AlertId> = self
                    .store
                    .alerts()
                    .active()
                    .map(|(id, _)| id.clone())
                    .collect();
                if ids.is_empty() {
                    self.toast(Severity::Info, "no active alert to acknowledge");
                }
                for id in ids {
                    self.execute(Command::Ack(id));
                }
                true
            }
            KeyCode::Char('A') => {
                self.alerts_overlay = !self.alerts_overlay;
                true
            }
            KeyCode::Char('V') => {
                match self.ambient.as_mut() {
                    Some(a) => a.relight_all(),
                    None => self.toast(
                        Severity::Info,
                        "V re-lights the page under a showcase theme (matrix)",
                    ),
                }
                true
            }
            KeyCode::Char('L') => {
                match self.ambient.as_mut() {
                    Some(a) => {
                        let locked = a.toggle_lock();
                        self.toast(
                            Severity::Info,
                            if locked {
                                "rain: everything lit (L unlocks)"
                            } else {
                                "rain: unlocked"
                            },
                        );
                    }
                    None => self.toast(
                        Severity::Info,
                        "L locks the page lit under a showcase theme (matrix)",
                    ),
                }
                true
            }
            KeyCode::Char('e') => {
                self.enter_edit();
                true
            }
            _ => false,
        }
    }

    fn move_focus(&mut self, dir: Direction) -> bool {
        if let (Some(solved), Some(cur)) = (&self.last_solved, self.focus)
            && let Some(next) = focus_dir(solved, cur, dir)
        {
            self.focus = Some(next);
        }
        true
    }

    fn focused_component_exists(&self) -> bool {
        let Some(f) = self.focus else { return false };
        let page = &self.pages[self.page];
        let Some(p) = page.place.get(f) else {
            return false;
        };
        self.instances
            .get(&self.instance_key(&p.target))
            .map(|i| i.component.is_some())
            .unwrap_or(false)
    }

    fn forward_key(&mut self, k: gridwatch_store::KeyEvent) -> bool {
        let Some(f) = self.focus else { return false };
        let page = self.pages[self.page].clone();
        let Some(p) = page.place.get(f) else {
            return false;
        };
        let key = self.instance_key(&p.target);
        let inner = self
            .last_solved
            .as_ref()
            .and_then(|s| s.cells.iter().find(|c| c.index == f))
            .map(|c| c.inner)
            .unwrap_or_default();
        let caps = self.caps;
        let zoomed = self.zoom == Some(f);
        let readonly = self.readonly;
        let mut outcome = Outcome::Ignored;
        let mut key_panicked = false;
        if let Some(inst) = self.instances.get_mut(&key)
            && let Some(component) = &mut inst.component
        {
            // The tier the component is *drawing* — the same call the
            // render path makes. A zoom-only tier's keys must not answer
            // on the grid, where its chrome is not drawn (arc 8a review).
            let (tier, _) = gridwatch_ui::component::pick_tier(
                component.tiers(),
                Size::new(inner.width, inner.height),
                zoomed,
                p.view.as_deref(),
            );
            let cx = gridwatch_ui::component::InputCx {
                store: &self.store,
                inner,
                caps: &caps,
                readonly,
                zoomed,
                tier,
            };
            crate::terminal::CONTAINED.with(|c| c.set(true));
            let r =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| component.on_key(k, &cx)));
            crate::terminal::CONTAINED.with(|c| c.set(false));
            match r {
                Ok(o) => outcome = o,
                Err(_) => key_panicked = true,
            }
        }
        if key_panicked {
            self.disable_instance(&key);
            self.captured = false;
            return true;
        }
        match outcome {
            Outcome::Ignored => false,
            Outcome::Consumed => true,
            Outcome::Release => {
                self.captured = false;
                true
            }
            Outcome::Command(cmd) => {
                self.execute(cmd);
                true
            }
        }
    }

    /// Run a command as if a component had returned it — the seam the
    /// "keys in, commands out" tests drive (D42).
    pub fn run_command(&mut self, cmd: Command) {
        self.execute(cmd);
    }

    fn execute(&mut self, cmd: Command) {
        match cmd {
            Command::Quit => self.quit = true,
            Command::Page(p) => self.set_page(p),
            Command::Zoom => {
                self.zoom = match (self.zoom, self.focus) {
                    (Some(_), _) => None,
                    (None, Some(f)) => Some(f),
                    _ => None,
                };
                self.cache.clear();
            }
            Command::Toast(s, m) => self.toast(s, m),
            Command::Record(on) => match &self.recorder {
                Some(r) => {
                    r.set_enabled(on);
                    let msg = if on {
                        format!("recording → {}", r.path().display())
                    } else {
                        format!("recording paused ({} lines)", r.written())
                    };
                    self.toast(Severity::Info, msg);
                }
                None => self.toast(
                    Severity::Info,
                    "no journal open — start with `gridwatch run --record FILE`",
                ),
            },
            Command::Ack(id) => {
                self.acked.insert(id);
            }
            Command::SaveLayout => {
                let path = crate::config::layout_path();
                let msg = match path {
                    Some(p) => self.save_layout_to(&p),
                    None => Err("no config directory (HOME unset)".into()),
                };
                match msg {
                    Ok(m) => self.toast(Severity::Info, m),
                    Err(e) => {
                        tracing::warn!("save layout: {e}");
                        self.toast(Severity::Warn, format!("not saved — {e}"));
                    }
                }
            }
            Command::Run(_, action) => self.request_action(action),
            Command::Source(id, ctl) => match self.controls.get(id.0) {
                Some(send) => send(ctl),
                None => self.toast(
                    Severity::Info,
                    format!("{id}: no live source to control (demo, replay or shot)"),
                ),
            },
        }
    }

    /// A component asked to change something. Refuse it under `readonly`,
    /// ask first if the action wants confirming, otherwise queue it.
    fn request_action(&mut self, action: Box<dyn Action>) {
        if self.readonly {
            self.toast(Severity::Warn, format!("read-only: {action:?} was not run"));
            return;
        }
        match action.confirm() {
            Some(question) => {
                let id = self.take_action_id();
                self.pending_action = Some((id, action, question));
            }
            None => self.dispatch_action(action),
        }
    }

    fn take_action_id(&mut self) -> gridwatch_store::ActionId {
        self.next_action += 1;
        gridwatch_store::ActionId(self.next_action)
    }

    fn dispatch_action(&mut self, action: Box<dyn Action>) {
        let id = self.take_action_id();
        let what = format!("{action:?}");
        match self.executor.as_ref() {
            Some(ex) => match ex.run(id, action) {
                Ok(()) => tracing::info!(action = %what, "queued"),
                Err(e) => self.toast(Severity::Warn, format!("{what}: {e}")),
            },
            None => self.toast(
                Severity::Info,
                format!("{what}: no executor in this mode (shot or replay)"),
            ),
        }
    }

    /// The confirm bar's key: `y` (or `Enter`) runs it, anything else
    /// cancels. Returns whether the key was the confirm bar's.
    fn confirm_key(&mut self, k: gridwatch_store::KeyEvent) -> bool {
        let Some((_, action, question)) = self.pending_action.take() else {
            return false;
        };
        match k.code {
            KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => {
                self.dispatch_action(action);
            }
            _ => self.toast(Severity::Info, format!("cancelled: {question}")),
        }
        true
    }

    /// Give the executor somewhere to send its answers, and turn them into
    /// toasts. Called by `run` once the control channel exists.
    pub fn attach_executor(&mut self, done: std::sync::mpsc::Sender<ControlMsg>) {
        self.executor = Some(crate::exec::Executor::new(done));
    }

    /// `locked` is "the command line said so", which a reload may not
    /// undo.
    pub fn set_readonly(&mut self, readonly: bool, locked: bool) {
        self.readonly = readonly;
        self.readonly_locked = locked;
    }

    /// Test seam: is an action waiting for an answer, and what does it ask?
    pub fn pending_question(&self) -> Option<&str> {
        self.pending_action.as_ref().map(|(_, _, q)| q.as_str())
    }

    fn screenshot(&mut self) -> Result<String, String> {
        let area = self.last_area;
        if area.width == 0 || area.height == 0 {
            return Err("no frame drawn yet".into());
        }
        // A fresh render, not a cached prev-buffer: prev_buf only exists while
        // the HUD or a stats log consumes it (P19).
        let mut b = Buffer::empty(area);
        self.draw_frame(area, &mut b);
        let dir = std::env::var_os("XDG_STATE_HOME")
            .map(std::path::PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".local/state"))
            })
            .ok_or("no state dir")?
            .join("gridwatch");
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let secs = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let path = dir.join(format!("shot-{secs}.txt"));
        std::fs::write(&path, gridwatch_ui::dump::ansi(&b)).map_err(|e| e.to_string())?;
        Ok(path.display().to_string())
    }
}

impl Shell {
    // ───────────────────────── edit mode (§10, arc 4a) ─────────────────────────

    pub fn editing(&self) -> bool {
        self.edit.is_some()
    }

    fn enter_edit(&mut self) {
        if self.zoom.is_some() {
            self.zoom = None;
        }
        self.captured = false;
        self.help = false;
        self.alerts_overlay = false;
        self.edit = Some(EditState::new(&self.pages[self.page]));
        self.cache.clear();
        self.toast(
            Severity::Info,
            "edit mode — Esc leaves, w saves layout.toml",
        );
    }

    fn leave_edit(&mut self) {
        self.edit = None;
        self.cache.clear();
    }

    /// The edit key bar: where the focused tile is, the keys, and the last
    /// refusal or the leave confirmation.
    fn edit_hints(&self, ed: &EditState) -> String {
        if ed.confirm_leave {
            return "unsaved changes — w save · y discard · Esc stay".to_string();
        }
        let page = &self.pages[self.page];
        let focus = self
            .focus
            .and_then(|f| page.place.get(f))
            .map(|p| {
                format!(
                    "{} @ ({},{}) {}×{}",
                    p.target.label(),
                    p.at.0,
                    p.at.1,
                    p.size.0,
                    p.size.1
                )
            })
            .unwrap_or_else(|| "no tile".into());
        // 120 columns must hold the way out (review): the keys are terse,
        // a note replaces the key list, `?` has the long form.
        let keys = "HJKL move · ^hjkl size · s size · S swap · a add · x del · u/^r undo · w save · Esc leave";
        let mode_note = match self.mode {
            SolveMode::Stack => "stack mode: edits apply but are not drawn — widen the terminal · ",
            SolveMode::Dense => "dense: no gutters to dot · ",
            SolveMode::Configured => "",
        };
        let empty_note = if self.pages[self.page].place.is_empty() {
            " — a adds one"
        } else {
            ""
        };
        if ed.pending_swap {
            return format!("EDIT · swap with… h/j/k/l (Esc cancels) · {focus}");
        }
        match &ed.note {
            Some(n) => format!("▲ {n} · EDIT · {focus}{empty_note} · w save · Esc leave · ? keys"),
            None => format!("EDIT · {focus}{empty_note} · {mode_note}{keys}"),
        }
    }

    /// The dotted unit grid in the gutters and the ghost of a refused or
    /// dragged op (seam 1, 3).
    fn draw_edit_chrome(&self, body: Rect, buf: &mut Buffer) {
        let (cols, rows) = gridwatch_ui::layout::unit_tracks(&self.grid, body, self.mode);
        let ghost_style = self.theme.style(Role::TextGhost);
        // Gutters only (review: a gutter line crosses a multi-unit tile's
        // interior, which must stay untouched), and only where the grid has
        // gutters at all — dense mode shares borders, gap 0 has none.
        if self.mode == SolveMode::Configured && self.grid.gap > 0 {
            let covered: Vec<Rect> = self
                .last_solved
                .as_ref()
                .map(|s| s.cells.iter().map(|c| c.outer).collect())
                .unwrap_or_default();
            let inside = |x: u16, y: u16| {
                covered
                    .iter()
                    .any(|r| x >= r.x && x < r.x + r.width && y >= r.y && y < r.y + r.height)
            };
            let gutter_cols: Vec<u16> = cols
                .windows(2)
                .flat_map(|w| (w[0].0 + w[0].1)..w[1].0)
                .map(|x| body.x + x)
                .collect();
            let gutter_rows: Vec<u16> = rows
                .windows(2)
                .flat_map(|w| (w[0].0 + w[0].1)..w[1].0)
                .map(|y| body.y + y)
                .collect();
            let dot = |x: u16, y: u16, buf: &mut Buffer| {
                if inside(x, y) {
                    return;
                }
                if let Some(c) = buf.cell_mut((x, y))
                    && c.symbol() == " "
                {
                    c.set_char('·');
                    c.set_style(ghost_style);
                }
            };
            for &x in &gutter_cols {
                for y in body.y..body.y + body.height {
                    dot(x, y, buf);
                }
            }
            for &y in &gutter_rows {
                for x in body.x..body.x + body.width {
                    dot(x, y, buf);
                }
            }
        }
        if let Some(g) = self.edit.as_ref().and_then(|e| e.ghost)
            && let Some(r) = unit_rect(&self.grid, body, self.mode, g.at, g.size)
        {
            // The theme's own severity rule: a refusal is Crit and carries
            // BOLD|REVERSED, so mono can tell red from green (review).
            let style = if g.ok {
                self.theme.style(Role::Ok).add_modifier(Modifier::BOLD)
            } else {
                self.theme.severity(Severity::Crit).0
            };
            ratatui::widgets::Block::new()
                .borders(ratatui::widgets::Borders::ALL)
                .border_type(ratatui::widgets::BorderType::Double)
                .border_style(style)
                .render(r, buf);
        }
    }

    fn edit_key(&mut self, k: gridwatch_store::KeyEvent) -> bool {
        let Some(ed) = self.edit.as_mut() else {
            return false;
        };
        // The picker eats every key while open (seam 2).
        if let Some(pk) = ed.picker.as_mut() {
            // Arrows (or Ctrl-n/p, Ctrl-j/k) move; every plain letter filters,
            // so a filter may start with `j` or `k` (review).
            match k.code {
                KeyCode::Esc => ed.picker = None,
                KeyCode::Down | KeyCode::Tab => pk.down(),
                KeyCode::Up | KeyCode::BackTab => pk.up(),
                KeyCode::Char('n') | KeyCode::Char('j') if k.mods.ctrl => pk.down(),
                KeyCode::Char('p') | KeyCode::Char('k') if k.mods.ctrl => pk.up(),
                KeyCode::Enter => {
                    let item = pk.selected();
                    ed.picker = None;
                    if let Some(item) = item {
                        self.edit_insert(item);
                    }
                }
                KeyCode::Backspace => pk.backspace(),
                KeyCode::Char(c) if !k.mods.ctrl => pk.type_char(c),
                _ => {}
            }
            return true;
        }
        if ed.confirm_leave {
            let pending_page = ed.pending_page;
            match crate::edit::decode(k) {
                EditKey::Save => {
                    ed.confirm_leave = false;
                    ed.pending_page = None;
                    self.execute(Command::SaveLayout);
                    if self
                        .edit
                        .as_ref()
                        .is_some_and(|e| !e.dirty(&self.pages[self.page]))
                    {
                        self.finish_leave(pending_page);
                    }
                }
                EditKey::Discard => {
                    let saved = ed.saved.clone();
                    self.pages[self.page] = saved;
                    self.clamp_focus();
                    self.finish_leave(pending_page);
                }
                EditKey::Other if k.code == KeyCode::Char('q') => {
                    self.quit = true;
                }
                _ => {
                    ed.confirm_leave = false;
                    ed.pending_page = None;
                    ed.note = None;
                }
            }
            return true;
        }
        let pending = std::mem::take(&mut ed.pending_swap);
        ed.note = None;
        ed.ghost = None;
        // Esc closes what is open first: a pending swap, the help panel.
        if k.code == KeyCode::Esc && (pending || self.help) {
            self.help = false;
            return true;
        }
        match crate::edit::decode(k) {
            EditKey::Move(dx, dy) => self.edit_op_at(
                |spec, page, f| gridwatch_ui::layout::move_by(spec, page, f, dx, dy),
                |at, size| (shift(at, dx, dy, 0), size),
            ),
            EditKey::Resize(dw, dh) => self.edit_op_at(
                |spec, page, f| gridwatch_ui::layout::resize_by(spec, page, f, dw, dh),
                |at, size| (at, shift(size, dw, dh, 1)),
            ),
            EditKey::Footprint => {
                let next = self.focus.and_then(|f| {
                    let p = self.pages[self.page].place.get(f)?;
                    let key = self.instance_key(&p.target);
                    let kind = self.instances.get(&key).map(|i| i.kind.clone())?;
                    let fps: Vec<(u8, u8)> = self
                        .registry
                        .component(&kind)?
                        .manifest
                        .footprints
                        .iter()
                        .map(|f| (f.w, f.h))
                        .collect();
                    gridwatch_ui::layout::footprint_cycle(&fps, p.size)
                });
                match next {
                    Some((w, h)) => self.edit_op_at(
                        |spec, page, f| {
                            let cur = page.place[f].size;
                            gridwatch_ui::layout::resize_by(
                                spec,
                                page,
                                f,
                                i16::from(w) as i8 - i16::from(cur.0) as i8,
                                i16::from(h) as i8 - i16::from(cur.1) as i8,
                            )
                        },
                        move |at, _| (at, (w, h)),
                    ),
                    None => {
                        if let Some(ed) = self.edit.as_mut() {
                            ed.note = Some("no footprints to cycle".into());
                        }
                    }
                }
            }
            EditKey::SwapPrefix => {
                if let Some(ed) = self.edit.as_mut() {
                    ed.pending_swap = true;
                }
            }
            EditKey::Dir(dir) => {
                if pending {
                    let other = match (&self.last_solved, self.focus) {
                        (Some(s), Some(f)) => focus_dir(s, f, dir),
                        _ => None,
                    };
                    match other {
                        Some(o) => {
                            let target = self.pages[self.page].place.get(o).map(|p| (p.at, p.size));
                            self.edit_op_at(
                                |spec, page, f| gridwatch_ui::layout::swap(spec, page, f, o),
                                move |at, size| target.unwrap_or((at, size)),
                            )
                        }
                        None => {
                            if let Some(ed) = self.edit.as_mut() {
                                ed.note = Some("no tile in that direction".into());
                            }
                        }
                    }
                } else {
                    self.move_focus(dir);
                }
            }
            EditKey::Picker => {
                let page = &self.pages[self.page];
                let configured: Vec<(String, String)> = self
                    .instances
                    .iter()
                    .filter(|(k, _)| !k.starts_with("kind:"))
                    .map(|(k, i)| (k.clone(), i.kind.clone()))
                    .collect();
                let kinds: Vec<String> = self
                    .registry
                    .components()
                    .map(|d| d.manifest.kind.to_string())
                    .collect();
                let items = crate::edit::picker_items(
                    configured.iter().map(|(a, b)| (a.as_str(), b.as_str())),
                    kinds.iter().map(String::as_str),
                    page,
                );
                if let Some(ed) = self.edit.as_mut() {
                    ed.picker = Some(crate::edit::Picker {
                        items,
                        ..Default::default()
                    });
                }
            }
            EditKey::Remove => {
                let n_before = self.pages[self.page].place.len();
                self.edit_op(|_, page, f| gridwatch_ui::layout::remove(page, f));
                if self.pages[self.page].place.len() < n_before {
                    self.clamp_focus();
                }
            }
            EditKey::Undo => {
                let page = &mut self.pages[self.page];
                if let Some(ed) = self.edit.as_mut()
                    && ed.undo(page)
                {
                    self.after_edit();
                }
            }
            EditKey::Redo => {
                let page = &mut self.pages[self.page];
                if let Some(ed) = self.edit.as_mut()
                    && ed.redo(page)
                {
                    self.after_edit();
                }
            }
            EditKey::Save => self.execute(Command::SaveLayout),
            EditKey::Leave => {
                let dirty = self
                    .edit
                    .as_ref()
                    .is_some_and(|e| e.dirty(&self.pages[self.page]));
                if dirty {
                    if let Some(ed) = self.edit.as_mut() {
                        ed.confirm_leave = true;
                    }
                } else {
                    self.leave_edit();
                }
            }
            EditKey::Discard | EditKey::Other => {
                // Pages, quit, theme, pause, HUD keep working in edit mode.
                return self.edit_passthrough(k);
            }
        }
        true
    }

    /// The keys edit mode leaves to the normal handler.
    fn edit_passthrough(&mut self, k: gridwatch_store::KeyEvent) -> bool {
        match k.code {
            KeyCode::Char('q') => {
                self.quit = true;
                true
            }
            KeyCode::Char('?')
            | KeyCode::F(12)
            | KeyCode::Char(' ')
            | KeyCode::Char('t')
            | KeyCode::Char('T')
            | KeyCode::Char('d')
            | KeyCode::Char('A')
            | KeyCode::Tab
            | KeyCode::BackTab => {
                // Run the normal handler with edit mode set aside, then restore.
                let ed = self.edit.take();
                let r = self.handle_key(k);
                self.edit = ed;
                r
            }
            KeyCode::Char(c @ '1'..='9') => {
                if let Some(i) = self.pages.iter().position(|p| p.hotkey == Some(c))
                    && i != self.page
                {
                    self.edit_change_page(i);
                }
                true
            }
            KeyCode::Char('[') | KeyCode::Char(']') => {
                let n = self.pages.len();
                let next = if k.code == KeyCode::Char('[') {
                    (self.page + n - 1) % n
                } else {
                    (self.page + 1) % n
                };
                if next != self.page {
                    self.edit_change_page(next);
                }
                true
            }
            _ => false,
        }
    }

    /// A page change in edit mode: leaving a dirty page asks first (seam 4:
    /// the stacks are per page and clear on a change).
    fn edit_change_page(&mut self, next: usize) {
        let dirty = self
            .edit
            .as_ref()
            .is_some_and(|e| e.dirty(&self.pages[self.page]));
        if dirty {
            if let Some(ed) = self.edit.as_mut() {
                ed.confirm_leave = true;
                ed.pending_page = Some(next);
            }
            return;
        }
        self.set_page(next);
        self.edit = Some(EditState::new(&self.pages[self.page]));
    }

    /// Leaving after a confirm: either out of edit mode, or — when a page
    /// change was what the user asked for — onto that page, still editing.
    fn finish_leave(&mut self, pending_page: Option<usize>) {
        match pending_page {
            Some(p) => {
                self.set_page(p);
                self.edit = Some(EditState::new(&self.pages[self.page]));
                self.cache.clear();
            }
            None => self.leave_edit(),
        }
    }

    /// Run a page op on the focused placement; commit or refuse (seam 1).
    fn edit_op(
        &mut self,
        op: impl FnOnce(
            &gridwatch_ui::layout::GridSpec,
            &Page,
            usize,
        ) -> Result<Page, gridwatch_ui::layout::EditError>,
    ) {
        self.edit_op_at(op, |at, size| (at, size));
    }

    /// Like `edit_op`, with the geometry the op *tried* for the ghost
    /// (review: the ghost sat on the tile itself): `attempted(at, size)`
    /// maps the current placement to the attempted one, clamped at 0/1.
    fn edit_op_at(
        &mut self,
        op: impl FnOnce(
            &gridwatch_ui::layout::GridSpec,
            &Page,
            usize,
        ) -> Result<Page, gridwatch_ui::layout::EditError>,
        attempted: impl FnOnce((u8, u8), (u8, u8)) -> ((u8, u8), (u8, u8)),
    ) {
        let Some(f) = self.focus else {
            if let Some(ed) = self.edit.as_mut() {
                ed.note = Some("no tile focused".into());
            }
            return;
        };
        let page = self.pages[self.page].clone();
        let Some(p) = page.place.get(f) else { return };
        let (at, size) = attempted(p.at, p.size);
        let result = op(&self.grid, &page, f);
        let Some(ed) = self.edit.as_mut() else { return };
        match result {
            Ok(next) if next == page => {
                // A no-op (a clamped drag): nothing to remember.
            }
            Ok(next) => {
                ed.commit(&mut self.pages[self.page], next);
                self.after_edit();
            }
            Err(e) => ed.refuse(e, at, size),
        }
    }

    fn after_edit(&mut self) {
        self.cache.clear();
        // Re-solve now: the next key in the same input batch (key repeat, a
        // paste, a pty write) needs the new geometry, not a frame later.
        if self.last_body.width > 0 && self.last_body.height > 0 {
            self.last_solved = Some(solve(
                &self.grid,
                &self.pages[self.page],
                self.last_body,
                self.mode,
                self.zoom,
                self.stack_scroll,
            ));
        } else {
            self.last_solved = None;
        }
        self.clamp_focus();
    }

    fn clamp_focus(&mut self) {
        let n = self.pages[self.page].place.len();
        self.focus = match (self.focus, n) {
            (_, 0) => None,
            (Some(f), n) if f >= n => Some(n - 1),
            (None, _) => Some(0),
            (f, _) => f,
        };
    }

    /// The picker chose an item: first fit at the kind's default footprint;
    /// an anonymous `kind:` target gets its instance built now (seam 2).
    fn edit_insert(&mut self, item: crate::edit::PickItem) {
        let footprint = self
            .registry
            .component(&item.kind)
            .map(|d| {
                (
                    d.manifest.default_footprint.w,
                    d.manifest.default_footprint.h,
                )
            })
            .unwrap_or((2, 1)); // a kind this build lacks: a chip slot, not a 1x1 badge
        let placement = crate::edit::placement_for(&item, footprint);
        let page = self.pages[self.page].clone();
        let result = gridwatch_ui::layout::insert_first_fit(&self.grid, &page, placement);
        match result {
            Ok(next) => {
                let key = self.instance_key(&item.target);
                if !self.instances.contains_key(&key) {
                    let inst =
                        build_instance(&self.registry, &item.kind, &toml::Table::new(), &self.caps);
                    self.instances.insert(key, inst);
                }
                let new_focus = next.place.len() - 1;
                if let Some(ed) = self.edit.as_mut() {
                    ed.commit(&mut self.pages[self.page], next);
                }
                self.after_edit();
                self.focus = Some(new_focus);
            }
            Err(e) => {
                self.toast(Severity::Warn, format!("cannot add {}: {e}", item.label));
            }
        }
    }

    /// The mouse in edit mode (seam 3): press focuses and starts a drag —
    /// a move, or a resize from the bottom-right corner — the drag previews
    /// a ghost, release applies it as one undo step.
    fn edit_mouse(&mut self, m: gridwatch_store::MouseEvent) -> bool {
        use gridwatch_store::MouseKind::*;
        let body = self.last_body;
        match m.kind {
            Down(gridwatch_store::MouseButton::Left) => {
                let Some(solved) = &self.last_solved else {
                    return false;
                };
                let Some(idx) = hit(solved, m.x, m.y) else {
                    return false;
                };
                let outer = solved
                    .cells
                    .iter()
                    .find(|c| c.index == idx)
                    .map(|c| c.outer)
                    .unwrap_or_default();
                let Some(press) = unit_at(&self.grid, body, self.mode, m.x, m.y) else {
                    return false;
                };
                let p = &self.pages[self.page].place[idx];
                let resize = m.x + 1 == outer.x + outer.width && m.y + 1 == outer.y + outer.height;
                self.focus = Some(idx);
                if let Some(ed) = self.edit.as_mut() {
                    ed.drag = Some(crate::edit::Drag {
                        index: idx,
                        press,
                        origin_at: p.at,
                        origin_size: p.size,
                        resize,
                        last: press,
                    });
                    ed.ghost = None;
                    ed.note = None;
                }
                true
            }
            Drag(gridwatch_store::MouseButton::Left) => {
                let Some(cur) = unit_at(&self.grid, body, self.mode, m.x, m.y) else {
                    return false;
                };
                let Some(drag) = self.edit.as_ref().and_then(|e| e.drag) else {
                    return false;
                };
                let (at, size) = drag.proposed(cur);
                let ok = self.drag_result(drag, cur).is_ok();
                if let Some(ed) = self.edit.as_mut() {
                    ed.ghost = Some(crate::edit::Ghost { at, size, ok });
                    if let Some(d) = ed.drag.as_mut() {
                        d.last = cur;
                    }
                }
                true
            }
            Up(gridwatch_store::MouseButton::Left) => {
                let Some(drag) = self.edit.as_mut().and_then(|e| e.drag.take()) else {
                    return false;
                };
                let cur = unit_at(&self.grid, body, self.mode, m.x, m.y).unwrap_or(drag.last);
                if cur == drag.press {
                    if let Some(ed) = self.edit.as_mut() {
                        ed.ghost = None;
                    }
                    return true;
                }
                let (at, size) = drag.proposed(cur);
                match self.drag_result(drag, cur) {
                    Ok(next) if next == self.pages[self.page] => {
                        if let Some(ed) = self.edit.as_mut() {
                            ed.ghost = None;
                        }
                    }
                    Ok(next) => {
                        if let Some(ed) = self.edit.as_mut() {
                            ed.commit(&mut self.pages[self.page], next);
                        }
                        self.after_edit();
                    }
                    Err(e) => {
                        if let Some(ed) = self.edit.as_mut() {
                            ed.refuse(e, at, size);
                        }
                    }
                }
                true
            }
            _ => false,
        }
    }

    fn drag_result(
        &self,
        drag: crate::edit::Drag,
        cur: (u8, u8),
    ) -> Result<Page, gridwatch_ui::layout::EditError> {
        let page = &self.pages[self.page];
        let (at, size) = drag.proposed(cur);
        if drag.resize {
            let dw = i16::from(size.0) - i16::from(drag.origin_size.0);
            let dh = i16::from(size.1) - i16::from(drag.origin_size.1);
            gridwatch_ui::layout::resize_by(&self.grid, page, drag.index, dw as i8, dh as i8)
        } else {
            let dx = i16::from(at.0) - i16::from(drag.origin_at.0);
            let dy = i16::from(at.1) - i16::from(drag.origin_at.1);
            gridwatch_ui::layout::move_by(&self.grid, page, drag.index, dx as i8, dy as i8)
        }
    }

    /// `w` (seam 5): render the file's text with the in-memory pages, verify
    /// it re-parses to them beside the current `config.toml`, register the
    /// hash with the watcher, write atomically. Returns the toast.
    pub fn save_layout_to(&mut self, path: &std::path::Path) -> Result<String, String> {
        let existing = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                crate::config::DEFAULT_LAYOUT.to_string()
            }
            Err(e) => return Err(format!("{}: {e}", path.display())),
        };
        let text = crate::save::render_layout(&existing, &self.pages)?;
        if path.exists() && text == existing {
            return Ok("layout.toml unchanged — nothing to save".into());
        }
        let (config_text, _) = crate::config::read_texts().map_err(|e| e.to_string())?;
        crate::save::verify(&text, &config_text, &self.pages)?;
        if let Some(tx) = &self.watch_ignore {
            let _ = tx.send((ReloadKind::Layout, crate::save::hash_of(&text)));
        }
        if let Err(e) = crate::save::write_atomic(path, &text) {
            // Withdraw the hash: nothing landed.
            if let Some(tx) = &self.watch_ignore {
                let _ = tx.send((ReloadKind::Layout, 0));
            }
            return Err(e);
        }
        if let Some(ed) = self.edit.as_mut() {
            ed.mark_saved(&self.pages[self.page]);
        }
        Ok(format!("layout.toml saved ({} pages)", self.pages.len()))
    }
}

/// `(x, y)` moved by `(dx, dy)`, clamped at `floor` — the geometry an edit
/// op attempted, for the ghost.
fn shift(v: (u8, u8), dx: i8, dy: i8, floor: u8) -> (u8, u8) {
    let f = i16::from(floor);
    (
        (i16::from(v.0) + i16::from(dx)).max(f) as u8,
        (i16::from(v.1) + i16::from(dy)).max(f) as u8,
    )
}

/// One instance: built, or a chip with the reason (and the fix when a
/// required capability is missing — §11).
fn build_instance(
    registry: &Registry,
    kind: &str,
    options: &toml::Table,
    caps: &CapSet,
) -> Instance {
    let mk = |component, reason: String, hint: String| Instance {
        animated: false,
        anim_fps: 0,
        kind: kind.to_string(),
        options: options.clone(),
        component,
        chip_reason: reason,
        chip_hint: hint,
    };
    match registry.component(kind) {
        None => mk(None, "arrives in a later arc".into(), String::new()),
        Some(def) => {
            if let Some(missing) = caps.missing(def.manifest.requires).first() {
                let (reason, hint) = crate::probe::missing_lines(*missing);
                return mk(None, reason, hint);
            }
            let mut cx = BuildCx { options, caps };
            match (def.build)(&mut cx) {
                Ok(c) => mk(Some(c), String::new(), String::new()),
                Err(e) => mk(None, e.0, String::new()),
            }
        }
    }
}

/// Build every configured instance plus the anonymous `kind:` placements
/// (§6, §9). A kind the registry lacks chips "arrives in a later arc"; a
/// missing *required* capability skips `build` and chips the reason and the
/// fix (§11); a `build` error chips its message. `previous` lets a reload
/// skip building an instance it is going to keep anyway.
fn build_instances(
    registry: &Registry,
    loaded: &Loaded,
    caps: &CapSet,
    previous: Option<&BTreeMap<String, Instance>>,
) -> BTreeMap<String, Instance> {
    let build = |kind: &str, options: &toml::Table| build_instance(registry, kind, options, caps);
    // A stand-in for an instance the caller is about to replace with the one
    // it kept: same kind and options, never drawn.
    let stand_in = |kind: &str, options: &toml::Table| Instance {
        animated: false,
        anim_fps: 0,
        kind: kind.to_string(),
        options: options.clone(),
        component: None,
        chip_reason: String::new(),
        chip_hint: String::new(),
    };
    let unchanged = |key: &str, kind: &str, options: &toml::Table| -> bool {
        previous
            .and_then(|p| p.get(key))
            .is_some_and(|i| i.kind == kind && i.options == *options)
    };
    let mut instances = BTreeMap::new();
    for inst in &loaded.config.components {
        let value = if unchanged(&inst.id, &inst.kind, &inst.options) {
            stand_in(&inst.kind, &inst.options)
        } else {
            build(&inst.kind, &inst.options)
        };
        instances.insert(inst.id.clone(), value);
    }
    let empty = toml::Table::new();
    for page in &loaded.pages {
        for p in &page.place {
            if let PlaceTarget::Kind(k) = &p.target {
                let key = format!("kind:{k}");
                if instances.contains_key(&key) {
                    continue;
                }
                let value = if unchanged(&key, k, &empty) {
                    stand_in(k, &empty)
                } else {
                    build(k, &empty)
                };
                instances.insert(key, value);
            }
        }
    }
    instances
}

/// §4.6: a placement may name a preferred tier; an unknown name is a config
/// warning and is ignored (the richest fitting tier is used). Only here can
/// it be checked — the tier list lives on the component.
fn view_warnings(loaded: &Loaded, instances: &BTreeMap<String, Instance>) -> Vec<String> {
    let mut out = Vec::new();
    for page in &loaded.pages {
        for p in &page.place {
            let key = match &p.target {
                PlaceTarget::Id(id) => id.clone(),
                PlaceTarget::Kind(k) => format!("kind:{k}"),
            };
            // An id no [[components]] entry defines draws as a chip (§6);
            // a reload that introduces one should say so, like `view` does.
            if let PlaceTarget::Id(id) = &p.target
                && !instances.contains_key(&key)
            {
                out.push(format!(
                    "page '{}': id \"{id}\" is not in config.toml — drawn as a chip",
                    page.name
                ));
                continue;
            }
            let Some(view) = &p.view else { continue };
            let Some(component) = instances.get(&key).and_then(|i| i.component.as_ref()) else {
                continue;
            };
            if !component.tiers().iter().any(|t| t.name == view) {
                let known: Vec<&str> = component.tiers().iter().map(|t| t.name).collect();
                let msg = format!(
                    "{key}: view = \"{view}\" is not a tier of `{}` (have {}) — ignored",
                    component.manifest().kind,
                    known.join(" ")
                );
                tracing::warn!("{msg}");
                out.push(msg);
            }
        }
    }
    out
}

/// The journal timestamp of a drained message (§4.5): a status keeps `since`
/// and an alert `at` — the moment the source meant, not the moment the render
/// thread got round to it; everything else is stamped `now`.
fn stamp(msg: &Msg, now: Ts) -> Ts {
    match msg {
        Msg::Control(ControlMsg::Status(_, s)) => s.since,
        Msg::Control(ControlMsg::Alert(a)) => a.at,
        _ => now,
    }
}

fn wall_ns(clock: &Clock) -> u64 {
    // Real runs: wall = actual unix time. Virtual runs (shot/replay/tests):
    // wall = the virtual clock, so output is byte-deterministic (§12.5).
    match clock {
        Clock::Virtual(_) => clock.now().0,
        _ => SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0),
    }
}

/// Copy a rendered tile into the frame. A cell the tile left unpainted has the
/// `Reset` background; it keeps the frame's themed background underneath
/// instead of punching a terminal-default hole in it (the SVG dumps showed
/// every tile interior in a stand-in black inside `#0b0324` chrome).
fn blit(tile: &Buffer, dest: Rect, buf: &mut Buffer) {
    let src = *tile.area();
    for y in 0..src.height.min(dest.height) {
        for x in 0..src.width.min(dest.width) {
            if let (Some(from), Some(to)) =
                (tile.cell((x, y)), buf.cell_mut((dest.x + x, dest.y + y)))
            {
                let bg = to.bg;
                *to = from.clone();
                if from.bg == ratatui::style::Color::Reset {
                    to.set_bg(bg);
                }
            }
        }
    }
}

/// The frame loop (§5), generic over the backend so `TestBackend` drives the
/// whole app (§11).
pub fn run_loop<B>(
    terminal: &mut Terminal<B>,
    shell: &mut Shell,
    inbox: &Inbox,
) -> Result<(), String>
where
    B: Backend,
    B::Error: std::fmt::Display,
{
    let mut last_draw = Instant::now() - HEARTBEAT;
    // The stats log samples on its own 1 Hz wall clock, not on the heartbeat:
    // the heartbeat only fires when *nothing else* drew for a second, so a busy
    // app — the one whose P6/P8/P19 rows we need — would never log a line.
    let mut last_stats = Instant::now();
    let mut dirty = true;
    // P18's two timestamps: exec (process start, as close as the loop can see
    // it) → first frame, and → every source's first sample.
    let started = shell.startup;
    let mut first_frame_ms: Option<u64> = None;
    loop {
        // Park on input; sources wake us within a frame via the 250 ms poll (P5 budget: render ≤ 4/s idle).
        // The first pass does not park at all: P18 gates the first frame at
        // 300 ms, and parking for 250 ms before drawing spent most of it.
        let timeout = if first_frame_ms.is_none() {
            Duration::ZERO
        } else if shell.animated_visible() && shell.terminal_focused {
            Duration::from_millis(1000 / u64::from(shell.effective_fps().max(1)))
        } else {
            Duration::from_millis(250)
        };
        // Every drained message is also teed to the recorder (§4.2, §4.5):
        // inputs and controls stamped with the run clock, batches with their
        // own `at`. The tee never blocks; a full channel drops and counts.
        match inbox.input.recv_timeout(timeout) {
            Ok(ev) => {
                shell.tee(shell.now(), &Msg::Input(ev.clone()));
                dirty |= shell.handle_input(ev);
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return Ok(()),
        }
        while let Ok(ev) = inbox.input.try_recv() {
            shell.tee(shell.now(), &Msg::Input(ev.clone()));
            dirty |= shell.handle_input(ev);
        }
        // Control: never dropped, drained before data (§4.2).
        while let Ok(c) = inbox.control.try_recv() {
            let msg = Msg::Control(c);
            shell.tee(stamp(&msg, shell.now()), &msg);
            let Msg::Control(c) = msg else { unreachable!() };
            shell.apply_control(c);
            dirty = true;
        }
        // Data: at most ~3 ms per frame (§5).
        let t0 = Instant::now();
        while let Ok(b) = inbox.data.try_recv() {
            let msg = Msg::Batch(b);
            let Msg::Batch(b) = &msg else { unreachable!() };
            shell.tee(b.at, &msg);
            let events = shell.store.apply(&msg);
            if !events.is_empty() {
                shell.route_alerts(events);
            }
            if t0.elapsed() > Duration::from_millis(3) {
                break;
            }
        }
        shell.tick_rules();
        if shell.quit {
            // Finish the drain before leaving: the 3 ms cap above exists to
            // keep frames short, not to lose data, and a recorder attached to
            // this loop must see every message that reached the channels
            // (the replay determinism test found eight batches missing here).
            while let Ok(c) = inbox.control.try_recv() {
                let msg = Msg::Control(c);
                shell.tee(stamp(&msg, shell.now()), &msg);
                let Msg::Control(c) = msg else { unreachable!() };
                shell.apply_control(c);
            }
            while let Ok(b) = inbox.data.try_recv() {
                let msg = Msg::Batch(b);
                let Msg::Batch(b) = &msg else { unreachable!() };
                shell.tee(b.at, &msg);
                shell.store.apply(&msg);
            }
            return Ok(());
        }
        let mut cause_data = false;
        let mut cause_beat = false;
        let mut cause_anim = false;
        if shell.data_dirty() {
            dirty = true;
            cause_data = true;
        }
        if last_draw.elapsed() >= HEARTBEAT {
            dirty = true;
            cause_beat = true;
        }
        // An animation is due (§5): a running effect or the rain, at its fps,
        // while focused — never while frozen (D28: P4 holds when unfocused).
        if shell.animated_visible()
            && shell.terminal_focused
            && last_draw.elapsed()
                >= Duration::from_millis(1000 / u64::from(shell.effective_fps().max(1)))
        {
            dirty = true;
            cause_anim = true;
        }
        if last_stats.elapsed() >= HEARTBEAT
            && let Some(path) = shell.stats_log.clone()
        {
            last_stats = Instant::now();
            let line = shell.stats.json_line(
                shell
                    .bytes_counter
                    .as_ref()
                    .map(|c| c.load(std::sync::atomic::Ordering::Relaxed))
                    .unwrap_or(0),
            );
            if let Ok(mut f) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
            {
                use std::io::Write as _;
                let _ = writeln!(f, "{line}");
            }
        }
        if dirty {
            let t = Instant::now();
            terminal
                .draw(|f| {
                    let area = f.area();
                    shell.draw_frame(area, f.buffer_mut());
                })
                .map_err(|e| e.to_string())?;
            shell.stats.record_frame(t.elapsed());
            if first_frame_ms.is_none() {
                let ms = started.elapsed().as_millis() as u64;
                first_frame_ms = Some(ms);
                shell.stats.first_frame_ms = ms;
            }
            if shell.stats.sources_live_ms == 0 && !shell.demands.is_empty() {
                let live = shell
                    .demands
                    .keys()
                    .filter(|name| shell.store.last_sample(SourceId(name)).is_some())
                    .count();
                if live == shell.demands.len() {
                    shell.stats.sources_live_ms = started.elapsed().as_millis() as u64;
                }
            }
            if cause_data {
                shell.stats.redraw_data += 1;
            }
            if cause_beat {
                shell.stats.redraw_heartbeat += 1;
            }
            if cause_anim {
                shell.stats.redraw_anim += 1;
            }
            last_draw = Instant::now();
            dirty = false;
        }
    }
}

/// Headless one-frame render for `shot` and the determinism test (§12.5).
pub fn shot_frame(shell: &mut Shell, w: u16, h: u16) -> Buffer {
    let area = Rect {
        x: 0,
        y: 0,
        width: w,
        height: h,
    };
    let mut buf = Buffer::empty(area);
    shell.draw_frame(area, &mut buf);
    buf
}

/// Feed the seeded synth straight into the shell's store (demo/headless).
/// The status message is part of the feed: a source that is publishing batches
/// is `Ok`, and without it every headless shot showed the one working source as
/// `starting` — including the dump in the README. Fed at `Detail::Table`, as
/// the Overview's 6x3 tile demands it, so a shot shows the process table.
pub fn feed_synth(shell: &mut Shell, seed: u64, ticks: usize) {
    let mut synth = gridwatch_store::demo::CpuSynth::new(seed);
    let mut gpu = gridwatch_store::demo::GpuSynth::new(seed);
    let mut pins = gridwatch_store::demo::PinsSynth::new(seed);
    let mut audio = gridwatch_store::demo::AudioSynth::new(seed);
    let mut sensors = gridwatch_store::demo::SensorsSynth::new(seed);
    let mut mediasynth = gridwatch_store::demo::MediaSynth::new(seed);
    let mut netsynth = gridwatch_store::demo::NetSynth::new(seed);
    for i in 0..ticks {
        let at = Ts((i as u64 + 1) * 1_500_000_000);
        let b = synth.tick_at(at, Detail::Table);
        shell.store.apply(&Msg::Batch(b));
        // The audio synth (arc 5a): silent for its first 1.5 s, then the song.
        shell.store.apply(&Msg::Batch(audio.tick_at(at)));
        shell.store.apply(&Msg::Batch(sensors.tick_at(at)));
        shell.store.apply(&Msg::Batch(mediasynth.tick_at(at)));
        shell
            .store
            .apply(&Msg::Batch(netsynth.tick_at(at, Detail::Table)));
        // Every synth, as `--demo` runs every source (arcs 2b, 3a); the pins
        // synth's scripted alert events go through `apply_control` so the
        // banner and the toasts see them exactly as the frame loop would.
        let b = gpu.tick_at(at, Detail::Table);
        shell.store.apply(&Msg::Batch(b));
        let tick = pins.tick_at(at);
        shell.store.apply(&Msg::Batch(tick.batch));
        // Straight into the store: a headless shot must not carry 8 s toasts
        // (the banner and the alerts tile read the store; toasts are the
        // frame loop's, exercised by the shell tests through `apply_control`).
        for a in tick.alerts {
            shell.store.apply(&Msg::Control(ControlMsg::Alert(a)));
        }
        if i == 0 {
            for src in [
                gridwatch_store::keys::cpu::SOURCE,
                gridwatch_store::keys::gpu::SOURCE,
                gridwatch_store::keys::pins::SOURCE,
                gridwatch_store::keys::audio::SOURCE,
                gridwatch_store::keys::sensors::SOURCE,
                gridwatch_store::keys::media::SOURCE,
                gridwatch_store::keys::net::SOURCE,
            ] {
                shell.store.apply(&Msg::Control(ControlMsg::Status(
                    src,
                    gridwatch_store::SourceStatus {
                        state: gridwatch_store::SourceState::Ok,
                        reason: Some(std::sync::Arc::from("synthetic (demo)")),
                        hint: None,
                        since: at,
                        last_sample: Some(at),
                        dropped: 0,
                        restarts: 0,
                    },
                )));
            }
        }
    }
}
