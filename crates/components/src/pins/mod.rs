//! The pins component (§8, brief arc 3 seam 4): astral-watch's per-pin
//! 12V-2x6 telemetry as a view tree — five cumulative tiers from an 8×3 watts
//! badge to the zoom-only `full` with tui.rs parity. Every number comes from
//! the pins source through the store; the device header reads the gpu
//! source's keys (optional), never sysfs or `nvidia-smi` (§4.6).

mod limit;
pub mod model;
mod view;

use std::borrow::Cow;

use gridwatch_store::keys::{gpu, pins};
use gridwatch_store::{Control, Detail, KeyCode, KeyEvent, Ts};
use gridwatch_ui::component::{
    BuildCx, BuildError, Chrome, Command, Component, ComponentDef, Footprint, InputCx, KeyHint,
    Manifest, Outcome, Redraw, RenderCx, Size, TickCx, Tier,
};
use gridwatch_ui::view::View;
use serde::{Deserialize, Serialize};

pub use model::Model;

pub static MANIFEST: Manifest = Manifest {
    kind: "pins",
    name: "12V-2x6 pins",
    summary: "astral-watch's per-pin amperage: bars, balance, the watts trend and the alert log",
    contract: 1,
    footprints: &[
        Footprint { w: 1, h: 1 },
        Footprint { w: 2, h: 1 },
        Footprint { w: 4, h: 2 },
        Footprint { w: 6, h: 3 },
    ],
    default_footprint: Footprint { w: 4, h: 2 },
    // The source needs the chip or the exporter; the tile renders honestly
    // from the store without either (a replay, `--demo`).
    requires: &[],
    optional: &[
        gridwatch_store::Capability::I2cNvidia,
        gridwatch_store::Capability::AstralExporter,
    ],
    sources: &[pins::SOURCE],
    optional_sources: &[gpu::SOURCE],
    chrome: Chrome::Themed,
    keys: &[
        KeyHint {
            key: "p",
            does: "freeze the display (the source keeps sampling)",
        },
        KeyHint {
            key: "r",
            does: "reset the session peaks",
        },
        KeyHint {
            key: "+ -",
            does: "faster / slower sampling by 100 ms (500–5000 ms, as tui.rs)",
        },
        KeyHint {
            key: "↑/↓ PgUp/PgDn",
            does: "scroll the alert log",
        },
    ],
    example_options: "options = { history = 300 }",
};

/// Rows in brackets are what the tier occupies (§8).
static TIERS: &[Tier] = &[
    Tier {
        name: "watts-badge",
        min: Size::new(8, 3),
        adds: &["total W", "balance badge", "alert glyph"],
        zoom_only: false,
    },
    Tier {
        name: "mini-bars",
        min: Size::new(20, 4),
        adds: &["six eighth-block bars", "9.2 A limit line"],
        zoom_only: false,
    },
    Tier {
        name: "bars",
        min: Size::new(40, 8),
        adds: &["peak caps", "per-pin values", "balance gauge", "totals"],
        zoom_only: false,
    },
    Tier {
        name: "trend",
        min: Size::new(60, 14),
        adds: &["watts sparkline", "alert log", "active-alert row"],
        zoom_only: false,
    },
    Tier {
        name: "full",
        min: Size::new(100, 24),
        adds: &[
            "device header from the gpu source",
            "six-pin braille trend",
            "scrollable log",
        ],
        zoom_only: true,
    },
];

pub const TIER_BADGE: usize = 0;
pub const TIER_MINI: usize = 1;
pub const TIER_BARS: usize = 2;
pub const TIER_TREND: usize = 3;
pub const TIER_FULL: usize = 4;

/// tui.rs's history: 300 samples.
pub const HISTORY_DEFAULT: u16 = 300;
/// Log lines kept by the tile (tui.rs `LOG_CAP` is 200; the store's alert
/// ring is 500 — the tile shows a window of it).
pub const LOG_CAP: usize = 200;

/// View-only instance options (§9).
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Options {
    /// Samples of history the trend and sparkline show.
    pub history: u16,
}

impl Default for Options {
    fn default() -> Options {
        Options {
            history: HISTORY_DEFAULT,
        }
    }
}

pub const OPTION_NAMES: &[&str] = &["history"];

pub struct Pins {
    options: Options,
    model: Model,
    /// The store generation the model was derived from.
    seen: Option<Ts>,
    /// `p`: the display is frozen at `frozen_at`; the source keeps sampling.
    frozen: Option<Ts>,
    log_scroll: usize,
    /// The interval `+`/`−` asked for, until `pins.info` reports it (review:
    /// reading it back from the store lagged a tick).
    pending_interval: Option<i64>,
}

impl Pins {
    pub fn new(options: Options) -> Pins {
        Pins {
            options: Options {
                history: options.history.clamp(30, 3000),
            },
            model: Model::default(),
            seen: None,
            frozen: None,
            log_scroll: 0,
            pending_interval: None,
        }
    }

    pub fn from_table(options: &toml::Table) -> Result<Pins, BuildError> {
        let parsed: Options = options
            .clone()
            .try_into()
            .map_err(|e| BuildError(format!("[[components]] options: {e}")))?;
        Ok(Pins::new(parsed))
    }

    pub fn options(&self) -> &Options {
        &self.options
    }

    pub fn model(&self) -> &Model {
        &self.model
    }

    pub fn frozen(&self) -> bool {
        self.frozen.is_some()
    }

    pub fn log_scroll(&self) -> usize {
        self.log_scroll
    }
}

impl Default for Pins {
    fn default() -> Pins {
        Pins::new(Options::default())
    }
}

fn build(cx: &mut BuildCx<'_>) -> Result<Box<dyn Component>, BuildError> {
    Ok(Box::new(Pins::from_table(cx.options)?))
}

pub const DEF: fn() -> ComponentDef = || ComponentDef {
    manifest: &MANIFEST,
    build,
};

impl Component for Pins {
    fn manifest(&self) -> &'static Manifest {
        &MANIFEST
    }

    fn title(&self, _max_width: u16, _cx: &TickCx<'_>) -> Cow<'static, str> {
        Cow::Borrowed("pins")
    }

    fn tiers(&self) -> &'static [Tier] {
        TIERS
    }

    fn demand(&self, _tier: usize) -> Detail {
        Detail::Meters
    }

    /// Derive once per pins generation: the latest reading, the session peaks
    /// and the active set. Frozen: keep the model as it was.
    fn tick(&mut self, cx: &TickCx<'_>) -> Redraw {
        let Some(at) = cx.store.last_sample(pins::SOURCE) else {
            return Redraw::No;
        };
        if self.seen == Some(at) {
            return Redraw::No;
        }
        self.seen = Some(at);
        if self.frozen.is_some() {
            // Peaks still accrue while frozen (tui.rs pauses sampling; we
            // only pause the picture), so a spike during a freeze is kept.
            self.model.observe_peaks(cx.store);
            return Redraw::No;
        }
        self.model.refresh(cx.store, at);
        if let (Some(p), Some(i)) = (self.pending_interval, self.model.info.as_ref())
            && i64::from(i.interval_ms) == p
        {
            self.pending_interval = None;
        }
        Redraw::Yes
    }

    fn on_key(&mut self, key: KeyEvent, cx: &InputCx<'_>) -> Outcome {
        match key.code {
            KeyCode::Char('p') => {
                self.frozen = match self.frozen {
                    Some(_) => {
                        self.seen = None; // refresh on the next tick
                        None
                    }
                    None => Some(cx.store.latest()),
                };
                Outcome::Consumed
            }
            KeyCode::Char('r') => {
                self.model.reset_peaks();
                Outcome::Consumed
            }
            KeyCode::Char('+') | KeyCode::Char('=') | KeyCode::Char('-') | KeyCode::Char('_') => {
                let cur = self.pending_interval.unwrap_or_else(|| {
                    self.model
                        .info
                        .as_ref()
                        .map(|i| i.interval_ms)
                        .unwrap_or(500) as i64
                });
                // tui.rs: `+` is a faster rate (a shorter interval), `-` slower.
                let delta = if matches!(key.code, KeyCode::Char('+') | KeyCode::Char('=')) {
                    -100
                } else {
                    100
                };
                let next = (cur + delta).clamp(500, 5000);
                if next == cur {
                    return Outcome::Consumed;
                }
                self.pending_interval = Some(next);
                Outcome::Command(Command::Source(
                    pins::SOURCE,
                    Control::SetOption("interval_ms".into(), toml::Value::Integer(next)),
                ))
            }
            KeyCode::Up => {
                self.log_scroll = self.log_scroll.saturating_add(1);
                Outcome::Consumed
            }
            KeyCode::Down => {
                self.log_scroll = self.log_scroll.saturating_sub(1);
                Outcome::Consumed
            }
            KeyCode::PageUp => {
                self.log_scroll = self.log_scroll.saturating_add(10);
                Outcome::Consumed
            }
            KeyCode::PageDown => {
                self.log_scroll = self.log_scroll.saturating_sub(10);
                Outcome::Consumed
            }
            KeyCode::Home => {
                self.log_scroll = 0;
                Outcome::Consumed
            }
            _ => Outcome::Ignored,
        }
    }

    fn view(&self, cx: &RenderCx<'_>) -> View {
        view::render(self, cx)
    }

    fn signature(&self, tier: usize) -> &'static [&'static str] {
        match tier {
            TIER_BADGE | TIER_MINI => &["W"],
            TIER_BARS => &["balance", "W"],
            _ => &["balance", "log"],
        }
    }
}
