//! gridwatch — a modular, themeable ops dashboard for the terminal.

use clap::{Parser, Subcommand};
use gridwatch_app::{RunOpts, run_terminal, shot};
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
        /// Show the stats HUD from the start (F12 toggles).
        #[arg(long)]
        stats: bool,
        /// Append per-second stats JSON lines to a file.
        #[arg(long)]
        stats_log: Option<std::path::PathBuf>,
    },
    /// Render one frame headlessly from synthetic data.
    Shot {
        #[arg(long, default_value = "ansi")]
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
    },
    /// Validate or print configuration.
    Config {
        #[command(subcommand)]
        what: ConfigCmd,
    },
    /// Show the capability probe.
    Doctor,
}

#[derive(Subcommand)]
enum ConfigCmd {
    /// Parse and validate config.toml + layout.toml.
    Check,
    /// Print the embedded defaults.
    Default,
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
        stats: false,
        stats_log: None,
    }) {
        Cmd::Run {
            demo,
            page,
            theme,
            fps,
            color,
            no_mouse,
            stats,
            stats_log,
        } => run_terminal(
            registry(),
            RunOpts {
                demo,
                page,
                theme,
                fps,
                color,
                no_mouse,
                stats,
                stats_log,
            },
        ),
        Cmd::Shot {
            format,
            size,
            theme,
            seed,
            page,
        } => {
            let (w, h) = size
                .split_once('x')
                .and_then(|(w, h)| Some((w.parse().ok()?, h.parse().ok()?)))
                .unwrap_or((250, 70));
            shot(registry(), seed, w, h, &theme, page, &format).map(|s| {
                // `shot | head` must not panic: swallow EPIPE on stdout.
                use std::io::Write as _;
                let _ = std::io::stdout().write_all(s.as_bytes());
            })
        }
        Cmd::Config { what } => match what {
            ConfigCmd::Check => gridwatch_app::config_check().map(|lines| {
                for l in lines {
                    println!("{l}");
                }
            }),
            ConfigCmd::Default => {
                let (c, l) = gridwatch_app::config_default();
                println!("# ---- config.toml ----\n{c}\n# ---- layout.toml ----\n{l}");
                Ok(())
            }
        },
        Cmd::Doctor => {
            for l in gridwatch_app::doctor() {
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
