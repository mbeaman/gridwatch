//! gridwatch-app: the application shell (ARCHITECTURE §5, §10, §11).
//! Terminal lifecycle, input thread, config, capability probe, frame loop.

#![deny(unsafe_code)] // one documented libc seam lives in sys.rs (dup2, localtime_r)

pub mod ambient;
pub mod app;
pub mod config;
pub mod edit;
pub mod effects;
pub mod flourish;
pub mod input;
pub mod probe;
pub mod save;
pub mod stats;
pub mod sys;
pub mod terminal;
pub mod watch;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use gridwatch_sources::spawn_source;
use gridwatch_store::{Clock, Header, JournalSource, RecordOpts, Recorder, Replay, Ts, channels};
use gridwatch_ui::Registry;
use gridwatch_ui::theme::{Theme, build_theme, load_builtin, load_theme_file};

pub use app::{Shell, feed_synth, run_loop, shot_frame};

#[derive(Clone, Debug, Default)]
pub struct RunOpts {
    pub demo: Option<u64>,
    pub page: Option<usize>,
    pub theme: Option<String>,
    pub fps: Option<u16>,
    pub color: Option<String>,
    pub no_mouse: bool,
    /// `--no-effects`: no tachyonfx hooks and no ambient layer (P20).
    pub no_effects: bool,
    pub stats: bool,
    pub stats_log: Option<std::path::PathBuf>,
    /// `--record FILE`: journal every message the frame loop drains (§4.5).
    pub record: Option<PathBuf>,
    /// `--record-input`: journal input events too.
    pub record_input: bool,
    /// `--tables on|off`: journal the process tables (off by default).
    pub tables: bool,
    /// `--replay FILE`: run from a journal instead of the live sources.
    pub replay: Option<PathBuf>,
    /// `--speed N` for `--replay`; 0 = as fast as possible.
    pub speed: Option<f64>,
}

/// Load a theme by built-in name or `.toml` path (§7, D52). A file may
/// `inherits` a built-in or a sibling file (`<name>` or `<name>.toml` next to
/// it); the parent must not inherit in turn (one level — a chain is an error).
pub fn load_theme_by_name(name: &str, mode: gridwatch_ui::ColorMode) -> Result<Theme, String> {
    if !name.ends_with(".toml") {
        return load_builtin(name, mode).map_err(|e| e.to_string());
    }
    let path = Path::new(name);
    // Errors name the file, not the path: a toast has one row (review).
    let short = short_name(path);
    let text = std::fs::read_to_string(path).map_err(|e| format!("{short}: {e}"))?;
    let file = load_theme_file(&text).map_err(|e| format!("{short}:{e}"))?;
    let parent = match &file.meta.inherits {
        None => None,
        Some(p) => Some(resolve_parent(path, p)?),
    };
    build_theme(&file, parent.as_ref(), mode).map_err(|e| format!("{short}: {e}"))
}

fn short_name(p: &Path) -> String {
    p.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| p.display().to_string())
}

/// The parent a theme file names: a built-in (flattened, so any built-in can
/// be inherited) or a sibling `<name>[.toml]`. A sibling that inherits a
/// built-in is flattened the same way; a sibling that inherits a *file* is a
/// chain (D52: one level of files).
fn resolve_parent(child: &Path, parent: &str) -> Result<gridwatch_ui::theme::ThemeFile, String> {
    if gridwatch_ui::theme::builtin(parent).is_some() {
        return gridwatch_ui::theme::builtin_file(parent).map_err(|e| e.to_string());
    }
    let dir = child.parent().unwrap_or_else(|| Path::new("."));
    let candidate = if parent.ends_with(".toml") {
        dir.join(parent)
    } else {
        dir.join(format!("{parent}.toml"))
    };
    let short = short_name(&candidate);
    let text = std::fs::read_to_string(&candidate).map_err(|e| {
        format!(
            "{}: inherits '{parent}' — not a built-in, and {short} next to it cannot be read: {e}",
            short_name(child),
        )
    })?;
    let mut file = load_theme_file(&text).map_err(|e| format!("{short}:{e}"))?;
    if let Some(grand) = file.meta.inherits.clone()
        && gridwatch_ui::theme::builtin(&grand).is_some()
    {
        let g = gridwatch_ui::theme::builtin_file(&grand).map_err(|e| e.to_string())?;
        file = gridwatch_ui::theme::merge(&file, &g);
        file.meta.inherits = None;
    }
    Ok(file)
}

/// Full assembly for the live terminal (§5): config → theme → sources →
/// input thread → raw mode/alt screen → frame loop → restore.
pub fn run_terminal(registry: Registry, opts: RunOpts) -> Result<(), String> {
    // Check this first, and *before* stderr is redirected: crossterm's failure
    // is "No such device or address (os error 6)" into a log file nobody is
    // looking at, which is indistinguishable from the process doing nothing.
    if !std::io::IsTerminal::is_terminal(&std::io::stdout()) {
        return Err(
            "gridwatch needs an interactive terminal — stdout is not a tty. \
                    Run it in a terminal window, or use `gridwatch shot` for a \
                    headless frame."
                .into(),
        );
    }
    let loaded = config::load().map_err(|e| e.to_string())?;
    let (mode, force_mono) = config::resolve_color(
        opts.color.as_deref(),
        &loaded.config.color,
        &config::ColorEnv::capture(),
    );
    let theme_name = if force_mono {
        "mono".to_string()
    } else {
        opts.theme
            .clone()
            .unwrap_or_else(|| loaded.config.theme.clone())
    };
    let theme = load_theme_by_name(&theme_name, mode)?;
    let caps = probe::probe();
    let tz = sys::tz_offset_s();
    // Replay runs on the virtual clock the journal source drives (§4.5, D47
    // seam 2); a live run on real time.
    let clock = if opts.replay.is_some() {
        Clock::new_virtual()
    } else {
        Clock::real_starting_now()
    };
    let (ch, inbox_early) = channels();

    // Sources: live builders or the seeded synth per --demo (§4.3) — or, under
    // --replay, only the journal source: the registry's real sources are not
    // started, and nothing downstream can tell the difference.
    let mut handles = Vec::new();
    let mut demands = BTreeMap::new();
    let mut controls: app::Controls = BTreeMap::new();
    if let Some(path) = &opts.replay {
        if !path.exists() {
            return Err(format!("--replay {}: no such file", path.display()));
        }
        // Before the alternate screen (§11, D46): a file that is not a journal
        // must be refused on the real stderr, not replayed into a dashboard
        // of dashes (the review's user-path lens hit exactly that).
        gridwatch_store::journal::check_header(path)
            .map_err(|e| format!("--replay {}: {e}", path.display()))?;
        let path = path.clone();
        let speed = opts.speed.unwrap_or(1.0);
        let mk = move || {
            Box::new(JournalSource::new(path.clone(), speed)) as Box<dyn gridwatch_store::Source>
        };
        let handle = spawn_source(
            gridwatch_store::JOURNAL,
            mk,
            ch.clone(),
            clock.clone(),
            toml::Table::new(),
        );
        handles.push(handle);
    } else {
        for def in registry.sources() {
            let def = *def;
            let options = loaded
                .config
                .sources
                .get(def.info.id.0)
                .and_then(|v| v.as_table())
                .cloned()
                .unwrap_or_default();
            let demo_seed = opts.demo;
            let ctx_options = options.clone(); // [sources.<id>] reaches SourceCtx::options (§9)
            let mk = move || match demo_seed {
                Some(seed) => (def.demo)(seed),
                None => (def.start)(&options),
            };
            let handle = spawn_source(def.info.id, mk, ch.clone(), clock.clone(), ctx_options);
            demands.insert(def.info.id.0, handle.demand.clone());
            controls.insert(def.info.id.0, std::sync::Arc::new(handle.controller()));
            handles.push(handle);
        }
    }
    // The receivers are re-bound *after* the source handles so that on every
    // exit path — the `?`s below included — they drop first: a journal source
    // blocked in `inject`'s `data.send` wakes only when the receiver is gone,
    // and `SourceHandle::drop` joins it (D48).
    let inbox = inbox_early;
    let _input = input::spawn(ch.clone());

    let mouse = loaded.config.mouse && !opts.no_mouse;
    let effects_on = loaded.config.effects.enabled && !opts.no_effects;
    terminal::install_panic_hook(mouse);
    let (mut term, guard, bytes) = terminal::enter(mouse).map_err(|e| e.to_string())?;
    // Only now: everything above can still report a failure to the real stderr,
    // and from here a library's `eprintln!` would scribble on the UI (§11).
    let log_path = sys::redirect_stderr();
    tracing_subscriber::fmt()
        // `warn` everywhere and `info` for gridwatch's own crates unless
        // RUST_LOG says otherwise (review: the log file was empty by default,
        // so the `stderr →` line and source failures reached nobody).
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn,gridwatch=info")),
        )
        .with_writer(std::io::stderr)
        // The writer is a file, never a tty: no colour escapes in the log.
        .with_ansi(false)
        .init();
    if let Some(p) = &log_path {
        tracing::info!("stderr → {}", p.display());
    }
    for w in theme.warnings.iter().chain(&loaded.warnings) {
        tracing::warn!("{w}");
    }
    let theme_warnings = theme.warnings.clone();

    let source_names: Vec<String> = registry
        .sources()
        .map(|d| d.info.id.0.to_string())
        .collect();
    let mut shell = Shell::new(
        registry, &loaded, theme, caps, tz, clock, demands, controls, opts.stats,
    );
    // Config warnings are otherwise only in the log the user cannot see from
    // inside the alternate screen (§4.6: an unknown `view` name is a warning).
    for w in shell.view_warnings().to_vec() {
        shell.warn_toast(w);
    }
    // The theme's own warnings (the WCAG gate, ignored tables) at start (D52).
    for w in theme_warnings {
        shell.warn_toast(w);
    }
    shell.set_theme_ref(theme_name.clone());
    shell.age_after_journal = true;
    shell.set_effects(effects_on, loaded.config.effects.budget_ms);
    shell.theme_locked = force_mono || opts.theme.is_some();
    // Hot reload (§9, seam 8): the watcher stats the two config files and the
    // theme file (when the theme is a file) once per second; the shell
    // re-parses on `ControlMsg::Reload`. Not under `--replay`, whose frames
    // must be reproducible from the journal alone.
    let watch = (opts.replay.is_none()).then(|| {
        let mut files: Vec<watch::Watched> = config::watched_paths()
            .into_iter()
            .enumerate()
            .map(|(i, path)| watch::Watched {
                kind: if i == 0 {
                    gridwatch_store::ReloadKind::Config
                } else {
                    gridwatch_store::ReloadKind::Layout
                },
                path,
            })
            .collect();
        files.extend(watch::theme_files(&theme_name));
        watch::spawn(files, ch.control.clone())
    });
    if let Some(w) = &watch {
        shell.watch_theme_files = Some(w.theme_files_sender());
        shell.watch_ignore = Some(w.ignore_sender());
    }
    shell.bytes_counter = Some(bytes);
    if let Some(path) = &opts.record {
        let size = term.size().map(|s| (s.width, s.height)).unwrap_or((0, 0));
        let header = Header::new(gridwatch_store::journal::hostname(), size, source_names);
        let rec_opts = RecordOpts {
            tables: opts.tables,
            input: opts.record_input,
        };
        match Recorder::start(path, &header, rec_opts) {
            Ok(r) => {
                shell.warn_toast(format!("recording → {}", path.display()));
                shell.recorder = Some(r);
            }
            Err(e) => {
                // After `enter`: the log *and* the UI (§11, D46).
                tracing::error!("--record {}: {e}", path.display());
                shell.warn_toast(format!("cannot record to {}: {e}", path.display()));
            }
        }
    }
    if opts.replay.is_some() {
        shell.store.ensure_source(gridwatch_store::JOURNAL);
    }
    if let Some(fps) = opts.fps {
        shell.set_fps(fps); // CLI beats config (§9 layering)
    }
    shell.stats_log = opts.stats_log.clone();
    if let Some(p) = opts.page {
        shell.set_page(p.saturating_sub(1));
    }

    let result = run_loop(&mut term, &mut shell, &inbox);
    drop(guard);
    // Receivers gone before the join (see `inbox` above), then the sources.
    drop(inbox);
    for h in handles {
        h.shutdown();
    }
    let mut errors = Vec::new();
    if let Err(e) = &result {
        errors.push(e.clone());
    }
    if let Some(r) = shell.recorder.take() {
        let path = r.path().to_path_buf();
        match r.finish() {
            // stderr is the log from here on (§11), so the summary a person
            // is waiting for goes to stdout, which is the restored terminal.
            Ok(done) => println!(
                "recorded {} lines to {} ({} dropped)",
                done.written,
                path.display(),
                done.dropped
            ),
            Err(e) => errors.push(format!("recording {}: {e}", path.display())),
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("; "))
    }
}

/// Build the headless shell every `shot` variant shares: embedded defaults,
/// no env, TrueColor, virtual clock (§12.5, D41).
fn headless_shell(registry: Registry, theme_name: &str, page: usize) -> Result<Shell, String> {
    let loaded = config::load_embedded().map_err(|e| e.to_string())?;
    let theme = load_theme_by_name(theme_name, gridwatch_ui::ColorMode::TrueColor)?;
    let demands = BTreeMap::new();
    let clock = Clock::new_virtual();
    let mut shell = Shell::new(
        registry,
        &loaded,
        theme,
        probe::probe(),
        0,
        clock,
        demands,
        BTreeMap::new(),
        false,
    );
    shell.set_page(page.saturating_sub(1));
    Ok(shell)
}

fn dump(buf: &ratatui::buffer::Buffer, format: &str) -> String {
    match format {
        "cells" => gridwatch_ui::dump::cells(buf),
        "svg" => gridwatch_ui::dump::svg(buf),
        _ => gridwatch_ui::dump::ansi(buf),
    }
}

/// `shot --replay FILE --at SECS`: apply the journal up to `at` on the virtual
/// clock and render one frame — byte-deterministic, the D41 property extended
/// to replay (§4.5).
#[allow(clippy::too_many_arguments)]
pub fn shot_replay(
    registry: Registry,
    path: &Path,
    at_secs: f64,
    w: u16,
    h: u16,
    theme_name: &str,
    page: usize,
    format: &str,
) -> Result<String, String> {
    let mut replay = Replay::load(path).map_err(|e| e.to_string())?;
    let mut shell = headless_shell(registry, theme_name, page)?;
    let at = Ts((at_secs.max(0.0) * 1e9) as u64);
    replay.apply_until(at, &mut shell.store);
    shell.set_clock(at);
    let buf = shot_frame(&mut shell, w, h);
    Ok(dump(&buf, format))
}

/// Headless screenshot (§12.5): demo store, one solved frame, ANSI, cells or SVG.
pub fn shot(
    registry: Registry,
    seed: u64,
    w: u16,
    h: u16,
    theme_name: &str,
    page: usize,
    format: &str,
) -> Result<String, String> {
    // Embedded defaults, no env: byte-deterministic across machines (§12.5).
    let mut shell = headless_shell(registry, theme_name, page)?;
    // 40 ticks ≈ a minute of synthetic history, so sparklines and the CCD bars
    // have something to show in a screenshot; still byte-deterministic (§12.5).
    feed_synth(&mut shell, seed, 40);
    let buf = shot_frame(&mut shell, w, h);
    Ok(dump(&buf, format))
}

/// `gridwatch keys` (§4.1, D33): the catalogue as the `docs/KEYS.md` table.
/// CI regenerates the file and fails on drift.
pub fn keys_doc() -> String {
    let mut out = String::new();
    out.push_str("# Metric catalogue\n\n");
    out.push_str("> **Generated by `gridwatch keys` (D33) — do not edit.** CI regenerates this file and fails on drift. Labels: `{n}` is an index (core, device, pin), `{text}` a name (`chip:label`, an interface).\n\n");
    out.push_str("| key | kind | unit | source | meaning |\n|---|---|---|---|---|\n");
    for meta in gridwatch_store::CATALOGUE.iter().flat_map(|d| d.iter()) {
        out.push_str(&format!(
            "| `{}` | {:?} | {:?} | `{}` | {} |\n",
            meta.name,
            meta.kind,
            meta.unit,
            meta.source.0,
            meta.doc.replace('|', "\\|")
        ));
    }
    out
}

/// `gridwatch component list`: every manifest as `docs/COMPONENTS.md` — a
/// summary table, then one section per kind with what `component info`
/// prints. Built with default options against the real capability probe is
/// *not* what we want here: the tier ladder is static, so an empty `BuildCx`
/// with every capability is used, and a kind that refuses to build says so.
pub fn components_doc(registry: &Registry) -> String {
    let mut out = String::new();
    out.push_str("# Components\n\n");
    out.push_str("> **Generated by `gridwatch component list` (D33) — do not edit.** CI regenerates this file and fails on drift. Tiers are cumulative in information, poorest first (D45); `min` is the inner size a tier needs; the first tier of every component fits 8×3.\n\n");
    out.push_str("| kind | name | sources | default footprint | tiers | summary |\n|---|---|---|---|---|---|\n");
    let kinds: Vec<&str> = registry.components().map(|d| d.manifest.kind).collect();
    for kind in &kinds {
        let def = registry.component(kind).expect("listed kind");
        let m = def.manifest;
        let tiers = build_for_doc(def)
            .map(|c| {
                c.tiers()
                    .iter()
                    .map(|t| t.name.to_string())
                    .collect::<Vec<_>>()
                    .join(" → ")
            })
            .unwrap_or_else(|e| format!("(does not build: {e})"));
        let sources: Vec<String> = m
            .sources
            .iter()
            .map(|s| format!("`{}`", s.0))
            .chain(m.optional_sources.iter().map(|s| format!("`{}`?", s.0)))
            .collect();
        out.push_str(&format!(
            "| `{}` | {} | {} | {}x{} | {} | {} |\n",
            m.kind,
            m.name,
            if sources.is_empty() {
                "—".to_string()
            } else {
                sources.join(", ")
            },
            m.default_footprint.w,
            m.default_footprint.h,
            tiers,
            m.summary
        ));
    }
    for kind in &kinds {
        let def = registry.component(kind).expect("listed kind");
        out.push('\n');
        out.push_str(&component_info(def));
    }
    out
}

fn build_for_doc(
    def: &gridwatch_ui::ComponentDef,
) -> Result<Box<dyn gridwatch_ui::Component>, String> {
    let options = toml::Table::new();
    let caps: gridwatch_store::CapSet = gridwatch_store::ALL_CAPABILITIES.iter().copied().collect();
    let mut cx = gridwatch_ui::BuildCx {
        options: &options,
        caps: &caps,
    };
    (def.build)(&mut cx).map_err(|e| e.0)
}

/// `gridwatch component info <kind>`: one manifest and its tier ladder.
pub fn component_info(def: &gridwatch_ui::ComponentDef) -> String {
    let m = def.manifest;
    let mut out = String::new();
    out.push_str(&format!(
        "## `{}` — {}

{}

",
        m.kind, m.name, m.summary
    ));
    out.push_str(&format!(
        "- contract {} · chrome {:?} · footprints {} · default {}x{}
",
        m.contract,
        m.chrome,
        m.footprints
            .iter()
            .map(|f| format!("{}x{}", f.w, f.h))
            .collect::<Vec<_>>()
            .join(" "),
        m.default_footprint.w,
        m.default_footprint.h
    ));
    let list = |caps: &[gridwatch_store::Capability]| -> String {
        if caps.is_empty() {
            "none".into()
        } else {
            caps.iter()
                .map(|c| format!("{c:?}"))
                .collect::<Vec<_>>()
                .join(", ")
        }
    };
    out.push_str(&format!(
        "- requires {} · optional {}
",
        list(m.requires),
        list(m.optional)
    ));
    let srcs = |s: &[gridwatch_store::SourceId]| -> String {
        if s.is_empty() {
            "none".into()
        } else {
            s.iter()
                .map(|s| format!("`{}`", s.0))
                .collect::<Vec<_>>()
                .join(", ")
        }
    };
    out.push_str(&format!(
        "- sources {} · optional sources {}
",
        srcs(m.sources),
        srcs(m.optional_sources)
    ));
    if !m.example_options.is_empty() {
        out.push_str(&format!(
            "- example `{}`
",
            m.example_options
        ));
    }
    match build_for_doc(def) {
        Ok(c) => {
            out.push_str(
                "
| tier | min | demand | adds | signature |
|---|---|---|---|---|
",
            );
            for (i, t) in c.tiers().iter().enumerate() {
                out.push_str(&format!(
                    "| `{}`{} | {}×{} | {:?} | {} | {} |
",
                    t.name,
                    if t.zoom_only { " (zoom)" } else { "" },
                    t.min.w,
                    t.min.h,
                    c.demand(i),
                    if t.adds.is_empty() {
                        "—".to_string()
                    } else {
                        t.adds.join(", ")
                    },
                    if c.signature(i).is_empty() {
                        "non-blank".to_string()
                    } else {
                        c.signature(i)
                            .iter()
                            .map(|s| format!("`{s}`"))
                            .collect::<Vec<_>>()
                            .join(" ")
                    }
                ));
            }
        }
        Err(e) => out.push_str(&format!(
            "
(does not build with default options: {e})
"
        )),
    }
    if !m.keys.is_empty() {
        out.push_str(
            "
Keys once captured with `Enter`:

",
        );
        for k in m.keys {
            out.push_str(&format!(
                "- `{}` — {}
",
                k.key, k.does
            ));
        }
    }
    out
}

/// `gridwatch config check [--theme NAME]`: the two files, then the theme —
/// the config's or the named one — with its loader warnings and the WCAG
/// contrast report (D52).
pub fn config_check(theme: Option<&str>) -> Result<Vec<String>, String> {
    let loaded = config::load().map_err(|e| e.to_string())?;
    let mut lines = vec![
        format!(
            "config: {}",
            loaded
                .config_path
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| "embedded default".into())
        ),
        format!(
            "layout: {} ({} pages)",
            loaded
                .layout_path
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| "embedded default".into()),
            loaded.pages.len()
        ),
    ];
    lines.extend(loaded.warnings.iter().map(|w| format!("warning: {w}")));
    // The rules that parsed, in the words the engine will use (arc 7b): a
    // check that only counted them would not tell anyone what will fire.
    if !loaded.rules.is_empty() {
        lines.push(format!("rules: {}", loaded.rules.len()));
        for r in &loaded.rules {
            let rhs = match &r.rhs {
                gridwatch_store::rules::Rhs::Value(v) => format!("{v}"),
                gridwatch_store::rules::Rhs::Key(k) => k.clone(),
            };
            let label = if r.label == "*" {
                String::new()
            } else {
                format!("{{{}}}", r.label)
            };
            // `absent` has no right-hand side, and resolves the moment
            // the key comes back rather than after `clear_s`.
            if r.op == gridwatch_store::rules::Op::Absent {
                lines.push(format!(
                    "  {} — {}{} absent for {:.0}s, resolves as soon as it returns ({:?})",
                    r.name,
                    r.key,
                    label,
                    r.for_s.as_secs_f64(),
                    r.severity,
                ));
                if r.label.contains('*') {
                    lines.push(format!(
                        "      note: `{}` is a pattern, so this can only fire for a label that \
                         has been seen at least once; name one exactly to catch a key that \
                         never arrives",
                        r.label
                    ));
                }
                continue;
            }
            lines.push(format!(
                "  {} — {}{} {} {rhs} for {:.0}s, clears after {:.0}s ({:?})",
                r.name,
                r.key,
                label,
                r.op.symbol(),
                r.for_s.as_secs_f64(),
                r.clear_s.as_secs_f64(),
                r.severity,
            ));
        }
    }
    let name = theme.unwrap_or(&loaded.config.theme);
    // A theme that does not load is a failed check (exit 1), as `run` would
    // fail on it — a check that prints "error" and exits 0 is no check.
    let t = load_theme_by_name(name, gridwatch_ui::ColorMode::TrueColor)
        .map_err(|e| format!("theme {name}: {e}"))?;
    lines.push(format!("theme: {name} ({}, {:?})", t.name, t.class));
    lines.extend(t.warnings.iter().map(|w| format!("warning: {w}")));
    lines.push("contrast (WCAG 2.1):".into());
    lines.extend(t.contrast_report().iter().map(|r| format!("  {r}")));
    let kinds: Vec<&str> = t.overridden_kinds().collect();
    if !kinds.is_empty() {
        lines.push(format!("component overrides: {}", kinds.join(", ")));
    }
    Ok(lines)
}

pub fn config_default() -> (&'static str, &'static str) {
    (config::DEFAULT_CONFIG, config::DEFAULT_LAYOUT)
}

/// `gridwatch doctor [--offline]`: every capability with a reason and a fix,
/// plus the live probes the sources own — the exporter is asked once and
/// `detect_bus` runs — unless `offline` (§11, seam 10).
pub fn doctor(offline: bool) -> Vec<String> {
    let caps = probe::probe();
    // sysfs-only probes are safe offline (the hwmon walk, the RAPL state).
    let mut live = gridwatch_sources::doctor_offline();
    if !offline {
        let exporter = config::load().ok().and_then(|l| {
            l.config
                .sources
                .get("pins")
                .and_then(|v| v.get("exporter"))
                .and_then(|v| v.as_str().map(str::to_string))
        });
        live.extend(gridwatch_sources::doctor(exporter.as_deref()));
    }
    let mut lines = probe::doctor_lines(&caps, &live);
    lines.push(String::new());
    lines.push(if offline {
        "live probes skipped (--offline): the astral-watch exporter, i2c detect_bus and pw-record --version (the hwmon walk is a sysfs read and ran)".into()
    } else {
        "live probes ran: the astral-watch exporter (one GET), i2c detect_bus and pw-record --version".into()
    });
    lines
}
