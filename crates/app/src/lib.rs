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

use gridwatch_sources::spawn_source;
use gridwatch_store::{Clock, channels};
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
    let log_path = sys::redirect_stderr();
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_writer(std::io::stderr)
        .init();
    if let Some(p) = &log_path {
        tracing::info!("stderr → {}", p.display());
    }
    for w in theme.warnings.iter().chain(&loaded.warnings) {
        tracing::warn!("{w}");
    }
    let caps = probe::probe();
    let tz = sys::tz_offset_s();
    let clock = Clock::real_starting_now();
    let (ch, inbox) = channels();

    // Sources: live builders or the seeded synth per --demo (§4.3).
    let mut handles = Vec::new();
    let mut demands = BTreeMap::new();
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
        handles.push(handle);
    }
    let _input = input::spawn(ch.clone());

    let mouse = loaded.config.mouse && !opts.no_mouse;
    terminal::install_panic_hook(mouse);
    let (mut term, guard, bytes) = terminal::enter(mouse).map_err(|e| e.to_string())?;

    let mut shell = Shell::new(
        registry, &loaded, theme, caps, tz, clock, demands, opts.stats,
    );
    shell.bytes_counter = Some(bytes);
    if let Some(fps) = opts.fps {
        shell.set_fps(fps); // CLI beats config (§9 layering)
    }
    shell.stats_log = opts.stats_log.clone();
    if let Some(p) = opts.page {
        shell.set_page(p.saturating_sub(1));
    }

    let result = run_loop(&mut term, &mut shell, &inbox);
    drop(guard);
    for h in handles {
        h.shutdown();
    }
    result
}

/// Headless screenshot (§12.5): demo store, one solved frame, ANSI or cells.
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
        false,
    );
    shell.set_page(page.saturating_sub(1));
    feed_synth(&mut shell, seed, 3);
    let buf = shot_frame(&mut shell, w, h);
    Ok(match format {
        "cells" => gridwatch_ui::dump::cells(&buf),
        _ => gridwatch_ui::dump::ansi(&buf),
    })
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
