//! gridwatch — a modular, themeable ops dashboard for the terminal.

use std::path::PathBuf;

use clap::{Parser, Subcommand};
use gridwatch_app::{RunOpts, run_terminal, shot, shot_replay};
use gridwatch_ui::Registry;

#[derive(Parser)]
#[command(
    name = "gridwatch",
    version,
    about = "A modular, themeable ops dashboard for the terminal"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Cmd>,
}

#[derive(Subcommand)]
enum Cmd {
    /// Run the dashboard (default).
    Run {
        /// Synthetic data with an optional seed (no hardware needed).
        #[arg(long, num_args = 0..=1, default_missing_value = "1")]
        demo: Option<u64>,
        /// Start on page N (1-based).
        #[arg(long)]
        page: Option<usize>,
        /// Theme name or a .toml path.
        #[arg(long)]
        theme: Option<String>,
        #[arg(long)]
        fps: Option<u16>,
        /// auto | always | never | 16 | 256 | truecolor
        #[arg(long)]
        color: Option<String>,
        #[arg(long)]
        no_mouse: bool,
        /// No theme effects and no ambient layer (matrix's rain).
        #[arg(long)]
        no_effects: bool,
        /// Show the stats HUD from the start (F12 toggles).
        #[arg(long)]
        stats: bool,
        /// Append per-second stats JSON lines to a file.
        #[arg(long)]
        stats_log: Option<PathBuf>,
        /// Record every message to a JSON Lines journal (`r` pauses/resumes).
        #[arg(long, value_name = "FILE")]
        record: Option<PathBuf>,
        /// With --record: journal key/mouse/resize/focus events too.
        #[arg(long, requires = "record")]
        record_input: bool,
        /// With --record: journal the process tables (`on`), or not (`off`).
        #[arg(long, default_value = "off", value_parser = ["on", "off"], requires = "record")]
        tables: String,
        /// Replay a journal instead of running the live sources.
        #[arg(long, value_name = "FILE", conflicts_with_all = ["demo", "record"])]
        replay: Option<PathBuf>,
        /// With --replay: time multiplier (0 = as fast as possible).
        #[arg(long, requires = "replay")]
        speed: Option<f64>,
        /// Refuse every action that would change another process, saying
        /// what it would have done.
        #[arg(long)]
        readonly: bool,
    },
    /// Render one frame headlessly from synthetic data or a journal.
    Shot {
        /// ansi | cells | svg
        #[arg(long, default_value = "ansi", value_parser = ["ansi", "cells", "svg"])]
        format: String,
        /// WxH, e.g. 250x70.
        #[arg(long, default_value = "250x70")]
        size: String,
        #[arg(long, default_value = "retrowave")]
        theme: String,
        #[arg(long, default_value_t = 1)]
        seed: u64,
        #[arg(long, default_value_t = 1)]
        page: usize,
        /// Render from a journal instead of the synth (see --at).
        #[arg(long, value_name = "FILE")]
        replay: Option<PathBuf>,
        /// With --replay: seconds into the journal to render at (≥ 0).
        #[arg(long, default_value_t = 60.0, requires = "replay", value_parser = parse_at)]
        at: f64,
        /// Read config.toml + layout.toml from this directory, and start the
        /// plugins it names. Without it a shot is the embedded default and
        /// byte-deterministic across machines (§12.5, D41).
        #[arg(long, value_name = "DIR", conflicts_with = "replay")]
        config: Option<PathBuf>,
    },
    /// Print the metric catalogue as docs/KEYS.md (D33).
    Keys,
    /// Component manifests.
    Component {
        #[command(subcommand)]
        what: ComponentCmd,
    },
    /// Validate or print configuration.
    Config {
        #[command(subcommand)]
        what: ConfigCmd,
    },
    /// Every capability with a reason and a fix, plus the sources' live probes.
    Doctor {
        /// Skip the live probes (the exporter GET and i2c detect_bus).
        #[arg(long)]
        offline: bool,
    },
}

#[derive(Subcommand)]
enum ComponentCmd {
    /// Every manifest and tier ladder, as docs/COMPONENTS.md.
    List,
    /// One component's manifest and tier ladder.
    Info { kind: String },
}

#[derive(Subcommand)]
enum ConfigCmd {
    /// Parse and validate config.toml + layout.toml, then the theme (the
    /// config's, or --theme NAME) with its WCAG contrast report.
    Check {
        #[arg(long)]
        theme: Option<String>,
    },
    /// Print the embedded defaults.
    Default,
}

fn parse_at(s: &str) -> Result<f64, String> {
    let v: f64 = s.parse().map_err(|e| format!("{e}"))?;
    if v.is_finite() && v >= 0.0 {
        Ok(v)
    } else {
        Err("must be a non-negative number of seconds".into())
    }
}

fn registry() -> Registry {
    let mut reg = Registry::default();
    gridwatch_components::builtin_components(&mut reg);
    gridwatch_sources::builtin_sources(&mut reg);
    reg
}

fn main() -> std::process::ExitCode {
    let cli = Cli::parse();
    let result = match cli.command.unwrap_or(Cmd::Run {
        demo: None,
        page: None,
        theme: None,
        fps: None,
        color: None,
        no_mouse: false,
        no_effects: false,
        stats: false,
        stats_log: None,
        record: None,
        record_input: false,
        tables: "off".into(),
        replay: None,
        speed: None,
        readonly: false,
    }) {
        Cmd::Run {
            demo,
            page,
            theme,
            fps,
            color,
            no_mouse,
            no_effects,
            stats,
            stats_log,
            record,
            record_input,
            tables,
            replay,
            speed,
            readonly,
        } => run_terminal(
            registry(),
            RunOpts {
                demo,
                page,
                theme,
                fps,
                color,
                no_mouse,
                no_effects,
                stats,
                stats_log,
                record,
                record_input,
                tables: tables == "on",
                replay,
                speed,
                readonly,
            },
        ),
        Cmd::Shot {
            format,
            size,
            theme,
            seed,
            page,
            replay,
            at,
            config,
        } => {
            let (w, h) = size
                .split_once('x')
                .and_then(|(w, h)| Some((w.parse().ok()?, h.parse().ok()?)))
                .unwrap_or((250, 70));
            let frame = match replay {
                Some(path) => shot_replay(registry(), &path, at, w, h, &theme, page, &format),
                None => shot(
                    registry(),
                    seed,
                    w,
                    h,
                    &theme,
                    page,
                    &format,
                    config.as_deref(),
                ),
            };
            frame.map(|s| {
                // `shot | head` must not panic: swallow EPIPE on stdout.
                use std::io::Write as _;
                let _ = std::io::stdout().write_all(s.as_bytes());
            })
        }
        Cmd::Keys => {
            print!("{}", gridwatch_app::keys_doc());
            Ok(())
        }
        Cmd::Component { what } => match what {
            ComponentCmd::List => {
                print!("{}", gridwatch_app::components_doc(&registry()));
                Ok(())
            }
            ComponentCmd::Info { kind } => {
                let reg = registry();
                match reg.component(&kind) {
                    Some(def) => {
                        print!("{}", gridwatch_app::component_info(def));
                        Ok(())
                    }
                    None => Err(format!(
                        "no component kind `{kind}` (have {})",
                        reg.components()
                            .map(|d| d.manifest.kind)
                            .collect::<Vec<_>>()
                            .join(" ")
                    )),
                }
            }
        },
        Cmd::Config { what } => match what {
            ConfigCmd::Check { theme } => {
                gridwatch_app::config_check(theme.as_deref()).map(|lines| {
                    for l in lines {
                        println!("{l}");
                    }
                })
            }
            ConfigCmd::Default => {
                let (c, l) = gridwatch_app::config_default();
                println!("# ---- config.toml ----\n{c}\n# ---- layout.toml ----\n{l}");
                Ok(())
            }
        },
        Cmd::Doctor { offline } => {
            for l in gridwatch_app::doctor(offline) {
                println!("{l}");
            }
            Ok(())
        }
    };
    match result {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("gridwatch: {e}");
            std::process::ExitCode::FAILURE
        }
    }
}
