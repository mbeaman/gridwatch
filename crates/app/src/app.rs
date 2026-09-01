//! The shell (§5, §10): solve → demand → tick → view → render → cache → draw,
//! with pages, focus, capture, zoom, pause, theme cycling and the F12 HUD.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime};

use gridwatch_store::{
    CapSet, Clock, ControlMsg, Detail, Inbox, InputEvent, KeyCode, Level, Msg, Severity, SourceId,
    Store, Ts,
};
use gridwatch_ui::component::{BuildCx, Chrome, Command, Component, Outcome, Size, pick_tier};
use gridwatch_ui::layout::{
    Cell, Direction, Page, PlaceTarget, SolveMode, Solved, derive_mode, focus_dir, hit, solve,
};
use gridwatch_ui::theme::{Role, Theme, TitleStyle, load_builtin};
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
    component: Option<Box<dyn Component>>,
    chip_reason: String,
}

struct Toast {
    text: String,
    severity: Severity,
    expires_at: Instant,
}

#[derive(Clone, PartialEq)]
struct CacheKey {
    gens: Vec<u64>,
    tier: usize,
    w: u16,
    h: u16,
    theme: String,
    focused: bool,
    zoomed: bool,
    view_hash: u64,
}

pub struct Shell {
    pub store: Store,
    registry: Registry,
    theme: Theme,
    theme_cycle: Vec<&'static str>,
    grid: gridwatch_ui::layout::GridSpec,
    pages: Vec<Page>,
    page: usize,
    instances: BTreeMap<String, Instance>,
    caps: CapSet,
    tz_offset_s: i32,
    demands: BTreeMap<&'static str, Arc<gridwatch_store::Demand>>,
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
    unfocused_fps: u16,
    pub stats_log: Option<std::path::PathBuf>,
    view_warnings: Vec<String>,
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
        stats_on: bool,
    ) -> Shell {
        let mut store = Store::default();
        for def in registry.sources() {
            store.ensure_source(def.info.id);
        }
        let mut instances = BTreeMap::new();
        let build = |kind: &str, options: &toml::Table| -> Instance {
            match registry.component(kind) {
                None => Instance {
                    kind: kind.to_string(),
                    component: None,
                    chip_reason: "arrives in a later arc".into(),
                },
                Some(def) => {
                    let mut cx = BuildCx {
                        options,
                        caps: &caps,
                    };
                    match (def.build)(&mut cx) {
                        Ok(c) => Instance {
                            kind: kind.to_string(),
                            component: Some(c),
                            chip_reason: String::new(),
                        },
                        Err(e) => Instance {
                            kind: kind.to_string(),
                            component: None,
                            chip_reason: e.0,
                        },
                    }
                }
            }
        };
        for inst in &loaded.config.components {
            instances.insert(inst.id.clone(), build(&inst.kind, &inst.options));
        }
        let empty = toml::Table::new();
        for page in &loaded.pages {
            for p in &page.place {
                if let PlaceTarget::Kind(k) = &p.target {
                    let key = format!("kind:{k}");
                    instances.entry(key).or_insert_with(|| build(k, &empty));
                }
            }
        }
        // §4.6: a placement may name a preferred tier; an unknown name is a
        // config warning and is ignored (the richest fitting tier is used).
        // Only here can it be checked — the tier list lives on the component.
        let mut view_warnings = Vec::new();
        for page in &loaded.pages {
            for p in &page.place {
                let Some(view) = &p.view else { continue };
                let key = match &p.target {
                    PlaceTarget::Id(id) => id.clone(),
                    PlaceTarget::Kind(k) => format!("kind:{k}"),
                };
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
                    view_warnings.push(msg);
                }
            }
        }
        Shell {
            store,
            registry,
            theme,
            theme_cycle: vec!["retrowave", "modern", "mono"],
            grid: loaded.grid,
            pages: loaded.pages.clone(),
            page: 0,
            instances,
            caps,
            tz_offset_s,
            demands,
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
            unfocused_fps: loaded.config.perf.unfocused_fps,
            stats_log: None,
            view_warnings,
        }
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

    pub fn set_fps(&mut self, fps: u16) {
        self.fps = fps.clamp(1, 60);
    }

    pub fn set_page(&mut self, p: usize) {
        if p < self.pages.len() {
            self.page = p;
            self.zoom = None;
            self.focus = Some(0);
            self.stack_scroll = 0;
            self.cache.clear();
        }
    }

    fn toast(&mut self, severity: Severity, text: impl Into<String>) {
        self.toasts.push(Toast {
            text: text.into(),
            severity,
            expires_at: Instant::now() + TOAST_TTL,
        });
    }

    fn instance_key(&self, target: &PlaceTarget) -> String {
        match target {
            PlaceTarget::Id(id) => id.clone(),
            PlaceTarget::Kind(k) => format!("kind:{k}"),
        }
    }

    /// Any visible source generation moved since the last frame → redraw (§5).
    pub fn data_dirty(&mut self) -> bool {
        let mut dirty = false;
        for def in self.registry.sources() {
            let g = self.store.generation(def.info.id);
            let e = self.last_gens.entry(def.info.id.0).or_insert(0);
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

    pub fn animated_visible(&self) -> bool {
        false // arc 5: the audio tile reports Animated; nothing animates in 1a
    }

    pub fn effective_fps(&self) -> u16 {
        if self.terminal_focused {
            self.fps
        } else {
            self.unfocused_fps.max(1)
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
        if area.height <= CHROME_ROWS || area.width < 20 {
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

        // Body.
        let top = u16::from(show_tabs);
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
        self.last_solved = Some(solved);
        self.update_demand();

        // Status bar.
        let hints = if self.captured {
            "Esc release · component keys active".to_string()
        } else {
            "q quit · ? help · [ ] pages · hjkl focus · Enter capture · z zoom · d dense · t theme · space pause · S shot · F12 hud"
                .to_string()
        };
        let status = Rect {
            x: area.x,
            y: area.y + area.height - 1,
            width: area.width,
            height: 1,
        };
        RLine::from(RSpan::styled(
            format!(" {hints}"),
            self.theme.style(Role::TextMuted),
        ))
        .render(status, buf);

        // Overlays.
        self.toasts.retain(|t| t.expires_at > Instant::now());
        for (i, t) in self.toasts.iter().enumerate() {
            let (style, glyph) = self.theme.severity(t.severity);
            let text = format!(" {glyph} {} ", t.text);
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
                    ("t", "cycle theme"),
                    ("space", "pause sources"),
                    ("S", "screenshot to state dir"),
                    ("F12", "stats HUD"),
                ],
                body,
                &self.theme,
                buf,
            );
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
            };
            overlay::hud(&stats, body, &self.theme, buf);
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
                overlay::chip(&key, "not in config.toml", cell.inner, &self.theme, buf);
            }
            return;
        }

        // Chrome first (the shell owns the frame — §4.6); title() is contained (§11).
        let mut title_panicked = false;
        let (kind, chrome, title) = {
            let inst = &self.instances[&key];
            let tick_cx = gridwatch_ui::component::TickCx {
                store: &self.store,
                now,
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
                overlay::chip(&kind, &inst.chip_reason, inner, &self.theme, buf);
                return;
            }
            Some(c) => c,
        };
        if cell.chip {
            overlay::chip(&kind, "starved", inner, &self.theme, buf);
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
            theme: &self.theme,
            now,
            wall,
            tz_offset_s: self.tz_offset_s,
            frame: self.frame,
        };
        crate::terminal::CONTAINED.with(|f| f.set(true));
        let viewed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let tick_cx = gridwatch_ui::component::TickCx {
                store: &self.store,
                now,
                visible: true,
                tier,
            };
            component.tick(&tick_cx);
            component.view(&render_cx)
        }));
        crate::terminal::CONTAINED.with(|f| f.set(false));
        let view = match viewed {
            Ok(v) => v,
            Err(_) => {
                self.disable_instance(&key);
                overlay::chip(&kind, "panicked — see the log", inner, &self.theme, buf);
                return;
            }
        };
        let m = component.manifest();
        let gens: Vec<u64> = m
            .sources
            .iter()
            .chain(m.optional_sources)
            .map(|s| self.store.generation(*s))
            .collect();
        // (§5: an `animation frame` term joins this key with `Animated` in arc 5.)
        let cache_key = CacheKey {
            gens,
            tier,
            w: inner.width,
            h: inner.height,
            theme: theme_name,
            focused,
            zoomed,
            view_hash: gridwatch_ui::view::fingerprint(&view),
        };
        let needs_render = self
            .cache
            .get(&cell.index)
            .map(|(k, _)| *k != cache_key)
            .unwrap_or(true);
        if needs_render {
            let mut tile = Buffer::empty(local);
            crate::terminal::CONTAINED.with(|f| f.set(true));
            let ok = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                self.theme
                    .renderer()
                    .render(&view, local, &self.theme, &mut tile);
            }))
            .is_ok();
            crate::terminal::CONTAINED.with(|f| f.set(false));
            if !ok {
                self.disable_instance(&key);
                overlay::chip(&kind, "panicked — see the log", inner, &self.theme, buf);
                return;
            }
            self.cache.insert(cell.index, (cache_key, tile));
        }
        if let Some((_, tile)) = self.cache.get(&cell.index) {
            blit(tile, inner, buf);
        }
        if fallback {
            // After the blit so it survives (§4.6): the pinned view does not fit.
            let marker = "view↓";
            let x = inner.x + inner.width.saturating_sub(5);
            buf.set_string(x, inner.y, marker, self.theme.style(Role::TextGhost));
        }
    }

    fn disable_instance(&mut self, key: &str) {
        if let Some(i) = self.instances.get_mut(key) {
            i.component = None;
            i.chip_reason = "panicked — see the log".into();
        }
    }

    pub fn handle_input(&mut self, ev: InputEvent) -> bool {
        match ev {
            InputEvent::FocusGained => {
                self.terminal_focused = true;
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
                true
            }
            InputEvent::Paste(_) => false,
            InputEvent::Mouse(m) => self.handle_mouse(m),
            InputEvent::Key(k) => self.handle_key(k),
        }
    }

    fn handle_mouse(&mut self, m: gridwatch_store::MouseEvent) -> bool {
        use gridwatch_store::MouseKind::*;
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
        if k.mods.ctrl && k.code == KeyCode::Char('q') {
            self.quit = true;
            return true;
        }
        if self.captured {
            if k.code == KeyCode::Esc {
                self.captured = false;
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
                let cur = self
                    .theme_cycle
                    .iter()
                    .position(|n| *n == self.theme.name)
                    .unwrap_or(0);
                let next = self.theme_cycle[(cur + 1) % self.theme_cycle.len()];
                match load_builtin(next, self.theme.mode) {
                    Ok(t) => {
                        self.theme = t;
                        self.cache.clear();
                    }
                    Err(e) => self.toast(Severity::Warn, format!("theme: {e}")),
                }
                true
            }
            KeyCode::Char('T') => {
                self.toast(Severity::Info, "hot reload arrives in arc 3");
                true
            }
            KeyCode::Char(' ') => {
                self.paused = !self.paused;
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
                self.toast(Severity::Info, "recording arrives in arc 2");
                true
            }
            KeyCode::Char('a') | KeyCode::Char('A') => {
                self.toast(Severity::Info, "alerts overlay arrives in arc 3");
                true
            }
            KeyCode::Char('V') | KeyCode::Char('L') => {
                self.toast(Severity::Info, "showcase themes arrive in arc 4");
                true
            }
            KeyCode::Char('e') => {
                self.toast(Severity::Info, "edit mode arrives in arc 4");
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
        let mut outcome = Outcome::Ignored;
        let mut key_panicked = false;
        if let Some(inst) = self.instances.get_mut(&key)
            && let Some(component) = &mut inst.component
        {
            let cx = gridwatch_ui::component::InputCx {
                store: &self.store,
                inner,
                caps: &caps,
                readonly: false,
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
            other => self.toast(Severity::Info, format!("{other:?} arrives in a later arc")),
        }
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

fn blit(tile: &Buffer, dest: Rect, buf: &mut Buffer) {
    let src = *tile.area();
    for y in 0..src.height.min(dest.height) {
        for x in 0..src.width.min(dest.width) {
            if let (Some(from), Some(to)) =
                (tile.cell((x, y)), buf.cell_mut((dest.x + x, dest.y + y)))
            {
                *to = from.clone();
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
        match inbox.input.recv_timeout(timeout) {
            Ok(ev) => dirty |= shell.handle_input(ev),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return Ok(()),
        }
        while let Ok(ev) = inbox.input.try_recv() {
            dirty |= shell.handle_input(ev);
        }
        // Control: never dropped, drained before data (§4.2).
        while let Ok(c) = inbox.control.try_recv() {
            let events = shell.store.apply(&Msg::Control(c));
            for ev in events {
                shell.toast(ev.severity, format!("{}: {}", ev.id.0, ev.title));
            }
            dirty = true;
        }
        // Data: at most ~3 ms per frame (§5).
        let t0 = Instant::now();
        while let Ok(b) = inbox.data.try_recv() {
            shell.store.apply(&Msg::Batch(b));
            if t0.elapsed() > Duration::from_millis(3) {
                break;
            }
        }
        if shell.quit {
            return Ok(());
        }
        let mut cause_data = false;
        let mut cause_beat = false;
        if shell.data_dirty() {
            dirty = true;
            cause_data = true;
        }
        if last_draw.elapsed() >= HEARTBEAT {
            dirty = true;
            cause_beat = true;
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
/// `starting` — including the dump in the README.
pub fn feed_synth(shell: &mut Shell, seed: u64, ticks: usize) {
    let mut synth = gridwatch_store::demo::CpuSynth::new(seed);
    let src = gridwatch_store::keys::cpu::SOURCE;
    for i in 0..ticks {
        let at = Ts((i as u64 + 1) * 1_500_000_000);
        let b = synth.tick(at);
        shell.store.apply(&Msg::Batch(b));
        if i == 0 {
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
