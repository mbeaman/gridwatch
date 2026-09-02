//! gridwatch-app: the application shell (ARCHITECTURE §5, §10, §11).
//! Terminal lifecycle, input thread, config, capability probe, frame loop.

#![deny(unsafe_code)] // one documented libc seam lives in sys.rs (dup2, localtime_r)

pub mod app;
pub mod config;
pub mod input;
pub mod probe;
pub mod stats;
pub mod sys;
pub mod terminal;

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

fn load_theme_by_name(name: &str, mode: gridwatch_ui::ColorMode) -> Result<Theme, String> {
    if name.ends_with(".toml") {
        let text = std::fs::read_to_string(name).map_err(|e| format!("{name}: {e}"))?;
        build_theme(&load_theme_file(&text).map_err(|e| e.to_string())?, mode)
            .map_err(|e| e.to_string())
    } else {
        load_builtin(name, mode).map_err(|e| e.to_string())
    }
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

/// `gridwatch config check` / `default` support.
pub fn config_check() -> Result<Vec<String>, String> {
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
        format!("theme: {}", loaded.config.theme),
    ];
    lines.extend(loaded.warnings.iter().map(|w| format!("warning: {w}")));
    Ok(lines)
}

pub fn config_default() -> (&'static str, &'static str) {
    (config::DEFAULT_CONFIG, config::DEFAULT_LAYOUT)
}

/// Capability table for `gridwatch doctor` (full command in arc 3).
pub fn doctor() -> Vec<String> {
    probe::doctor_lines(&probe::probe())
}
