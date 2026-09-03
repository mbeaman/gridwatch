//! The audio visualizer (§8, brief arc 5 seam 4): the sink monitor's
//! spectrum, scope and levels as a view tree — five cumulative tiers from an
//! 8×3 VU pair to the zoom-only `full`. The first component that animates:
//! `RedrawPolicy::Animated { fps }` while there is sound, `Redraw::No` once
//! silent and settled (the source's 2 Hz cadence still redraws it). The
//! source publishes instantaneous heights; the ballistics here move them.
//! Components never read `/proc`, never spawn: the sink picker asks the
//! source with `Command::Source(audio, Control::Domain(SetSink))`.

pub mod ballistics;
mod scope;
mod view;

use std::borrow::Cow;

use gridwatch_store::keys::audio::{self, AudioSink, BANDS, SetSink};
use gridwatch_store::{Control, Detail, KeyCode, KeyEvent, Ts};
use gridwatch_ui::component::{
    BuildCx, BuildError, Chrome, Command, Component, ComponentDef, Footprint, InputCx, KeyHint,
    Manifest, Outcome, Redraw, RedrawPolicy, RenderCx, Size, TickCx, Tier,
};
use gridwatch_ui::view::View;
use serde::{Deserialize, Serialize};

pub use ballistics::{Bars, Preset, Vu};

pub static MANIFEST: Manifest = Manifest {
    kind: "audio",
    name: "audio visualizer",
    summary: "the default sink's spectrum, scope and VU — Winamp or cava ballistics, a sink picker",
    contract: 1,
    footprints: &[
        Footprint { w: 1, h: 1 },
        Footprint { w: 2, h: 1 },
        Footprint { w: 4, h: 2 },
        Footprint { w: 6, h: 3 },
    ],
    default_footprint: Footprint { w: 4, h: 2 },
    // The source degrades itself (pw-record missing, no socket); the tile
    // renders honestly from the store without either (a replay, `--demo`).
    requires: &[],
    optional: &[
        gridwatch_store::Capability::PwRecord,
        gridwatch_store::Capability::PipeWireSocket,
    ],
    sources: &[audio::SOURCE],
    optional_sources: &[],
    chrome: Chrome::Themed,
    keys: &[
        // Terse: the captured key bar must fit 80 columns with all four
        // (review: `s sink` fell off at 120).
        KeyHint {
            key: "m",
            does: "mode",
        },
        KeyHint {
            key: "g",
            does: "preset",
        },
        KeyHint {
            key: "[ ]",
            does: "window",
        },
        KeyHint {
            key: "s",
            does: "sink",
        },
    ],
    example_options: "options = { preset = \"cava\", mode = \"both\" }",
};

/// Rows in brackets are what the tier occupies (§8).
static TIERS: &[Tier] = &[
    Tier {
        name: "vu",
        min: Size::new(8, 3),
        adds: &["stereo VU pair", "peak"],
        zoom_only: false,
    },
    Tier {
        name: "mini",
        min: Size::new(16, 4),
        adds: &["thin mono bars"],
        zoom_only: false,
    },
    Tier {
        name: "scope",
        min: Size::new(30, 6),
        adds: &["oscilloscope"],
        zoom_only: false,
    },
    Tier {
        name: "spectrum",
        min: Size::new(40, 8),
        adds: &[
            "mirrored stereo bars",
            "peak caps",
            "Hz axis",
            "sink and levels header",
        ],
        zoom_only: false,
    },
    Tier {
        name: "full",
        min: Size::new(100, 24),
        adds: &[
            "spectrum + scope + VU",
            "preset and sink chips",
            "LUFS with audio-lufs",
        ],
        zoom_only: true,
    },
];

pub const TIER_VU: usize = 0;
pub const TIER_MINI: usize = 1;
pub const TIER_SCOPE: usize = 2;
pub const TIER_SPECTRUM: usize = 3;
pub const TIER_FULL: usize = 4;

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    #[default]
    Bars,
    Scope,
    Both,
}

impl Mode {
    pub fn next(self) -> Mode {
        match self {
            Mode::Bars => Mode::Scope,
            Mode::Scope => Mode::Both,
            Mode::Both => Mode::Bars,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Mode::Bars => "bars",
            Mode::Scope => "scope",
            Mode::Both => "both",
        }
    }
}

/// `bars = "auto" | N`.
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum BarCount {
    Fixed(u16),
    #[default]
    #[serde(with = "auto_word")]
    Auto,
}

mod auto_word {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str("auto")
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<(), D::Error> {
        let s = String::deserialize(d)?;
        if s == "auto" {
            Ok(())
        } else {
            Err(serde::de::Error::custom("expected \"auto\" or a bar count"))
        }
    }
}

/// View-only instance options (§9).
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Options {
    pub preset: Preset,
    pub bars: BarCount,
    pub mode: Mode,
    /// The animation frame rate the tile asks the shell for (5–60).
    pub fps: u8,
}

impl Default for Options {
    fn default() -> Options {
        Options {
            preset: Preset::Winamp,
            bars: BarCount::Auto,
            mode: Mode::Bars,
            fps: 30,
        }
    }
}

pub const OPTION_NAMES: &[&str] = &["preset", "bars", "mode", "fps"];

/// No sample for this long and the input counts as silence (a dead child,
/// a finished replay) — comfortably above the 500 ms silence cadence.
pub const STALL_AFTER: std::time::Duration = std::time::Duration::from_millis(1_500);

/// The display window over the 64 bands, in band indices (`[`/`]`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Window {
    pub lo: usize,
    pub hi: usize,
}

impl Default for Window {
    fn default() -> Window {
        Window { lo: 0, hi: BANDS }
    }
}

impl Window {
    pub const STEP: usize = 4;
    pub const MIN: usize = 8;

    pub fn narrow(self) -> Window {
        if self.hi - self.lo <= Window::MIN {
            return self;
        }
        Window {
            lo: self.lo + Window::STEP,
            hi: (self.hi - Window::STEP).max(self.lo + Window::STEP + Window::MIN),
        }
    }

    pub fn widen(self) -> Window {
        Window {
            lo: self.lo.saturating_sub(Window::STEP),
            hi: (self.hi + Window::STEP).min(BANDS),
        }
    }

    pub fn len(&self) -> usize {
        self.hi - self.lo
    }

    pub fn is_empty(&self) -> bool {
        self.hi <= self.lo
    }

    /// The window's edges in Hz over the source's default 30 Hz–16 kHz span.
    pub fn hz(&self, k: usize) -> f64 {
        30.0 * (16_000.0f64 / 30.0).powf(k as f64 / BANDS as f64)
    }
}

/// The sink picker's state.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Picker {
    pub selected: usize,
    pub sinks: Vec<AudioSink>,
}

pub struct Audio {
    options: Options,
    mode: Mode,
    preset: Preset,
    window: Window,
    /// The latest instantaneous heights per channel from the store.
    bands: [[f32; BANDS]; 2],
    /// Ballistics per channel over the 64 bands (resampled at view time).
    bars: [Bars; 2],
    vu: [Vu; 2],
    scope: Vec<f32>,
    sink: Option<AudioSink>,
    silent: bool,
    seen: Option<Ts>,
    last_tick: Option<Ts>,
    picker: Option<Picker>,
    /// Set by `tick` when something on screen moved.
    moving: bool,
}

impl Audio {
    pub fn new(options: Options) -> Audio {
        let options = Options {
            fps: options.fps.clamp(5, 60),
            ..options
        };
        Audio {
            mode: options.mode,
            preset: options.preset,
            bars: [
                Bars::new(options.preset, BANDS),
                Bars::new(options.preset, BANDS),
            ],
            options,
            window: Window::default(),
            bands: [[0.0; BANDS]; 2],
            vu: [Vu::default(); 2],
            scope: Vec::new(),
            sink: None,
            silent: true,
            seen: None,
            last_tick: None,
            picker: None,
            moving: false,
        }
    }

    pub fn from_table(options: &toml::Table) -> Result<Audio, BuildError> {
        let parsed: Options = options
            .clone()
            .try_into()
            .map_err(|e| BuildError(format!("[[components]] options: {e}")))?;
        Ok(Audio::new(parsed))
    }

    pub fn options(&self) -> &Options {
        &self.options
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }

    pub fn preset(&self) -> Preset {
        self.preset
    }

    pub fn window(&self) -> Window {
        self.window
    }

    pub fn picker(&self) -> Option<&Picker> {
        self.picker.as_ref()
    }

    pub fn sink(&self) -> Option<&AudioSink> {
        self.sink.as_ref()
    }

    pub fn silent(&self) -> bool {
        self.silent
    }

    /// The displayed heights of channel `ch` resampled to `n` bars over the
    /// window: each bar is the max of the bands it covers.
    pub fn resampled(&self, ch: usize, n: usize) -> (Vec<f32>, Vec<f32>) {
        let heights = self.bars[ch].heights();
        let peaks = self.bars[ch].peaks();
        let (h, p) = (
            resample_max(&heights[self.window.lo..self.window.hi], n),
            resample_max(&peaks[self.window.lo..self.window.hi], n),
        );
        (h, p)
    }

    fn enumerate(&self, on: bool) -> Command {
        Command::Source(
            audio::SOURCE,
            Control::SetOption("enumerate".into(), toml::Value::Boolean(on)),
        )
    }
}

/// Max-pool `src` into `n` cells.
pub fn resample_max(src: &[f32], n: usize) -> Vec<f32> {
    if n == 0 || src.is_empty() {
        return vec![0.0; n];
    }
    (0..n)
        .map(|i| {
            let a = i * src.len() / n;
            let b = (((i + 1) * src.len()) / n).max(a + 1).min(src.len());
            src[a..b].iter().cloned().fold(0f32, f32::max)
        })
        .collect()
}

/// The spectrum's mirrored bar values for a width (tests).
pub fn mirrored_for_test(a: &Audio, width: u16) -> (Vec<f32>, Vec<f32>) {
    view::mirrored(a, width)
}

impl Default for Audio {
    fn default() -> Audio {
        Audio::new(Options::default())
    }
}

fn build(cx: &mut BuildCx<'_>) -> Result<Box<dyn Component>, BuildError> {
    Ok(Box::new(Audio::from_table(cx.options)?))
}

pub const DEF: fn() -> ComponentDef = || ComponentDef {
    manifest: &MANIFEST,
    build: Box::new(build),
};

impl Component for Audio {
    fn manifest(&self) -> &'static Manifest {
        &MANIFEST
    }

    fn title(&self, _max_width: u16, _cx: &TickCx<'_>) -> Cow<'static, str> {
        Cow::Borrowed("audio")
    }

    fn tiers(&self) -> &'static [Tier] {
        TIERS
    }

    fn demand(&self, _tier: usize) -> Detail {
        Detail::Meters
    }

    fn redraw_policy(&self) -> RedrawPolicy {
        RedrawPolicy::Animated {
            fps: self.options.fps,
        }
    }

    /// Pull the latest generation, then advance the ballistics by the time
    /// since the last tick (`cx.now`, never a wall clock).
    fn tick(&mut self, cx: &TickCx<'_>) -> Redraw {
        let dt = self
            .last_tick
            .map(|t| cx.now.since(t).as_secs_f64())
            .unwrap_or(0.0)
            .clamp(0.0, 0.5);
        self.last_tick = Some(cx.now);
        let at = cx.store.last_sample(audio::SOURCE);
        if at.is_some() && self.seen != at {
            self.seen = at;
            for ch in 0..2 {
                if let Some((_, v)) = cx.store.vector(&audio::BANDS_KEY.idx(ch as u16)) {
                    for (o, s) in self.bands[ch].iter_mut().zip(v.iter()) {
                        *o = *s;
                    }
                }
            }
            if let Some((_, s)) = cx.store.vector(&audio::SCOPE) {
                self.scope.clear();
                self.scope.extend_from_slice(s);
            }
            self.sink = cx.store.record(&audio::SINK).map(|(_, s)| s.clone());
            self.silent = cx
                .store
                .record(&audio::LEVEL)
                .map(|(_, l)| l.silent)
                .unwrap_or(false);
            if let Some(p) = self.picker.as_mut()
                && let Some((_, list)) = cx.store.record(&audio::SINKS)
            {
                p.sinks = list.sinks.clone();
                p.selected = p.selected.min(p.sinks.len().saturating_sub(1));
            }
        }
        // A source that stopped publishing (pw-record died, a replay ended)
        // is silent input: the bars decay instead of animating a still
        // picture at 30 fps (review).
        let stalled = at.is_some_and(|t| cx.now.since(t) > STALL_AFTER);
        let quiet = self.silent || stalled;
        let mut moving = false;
        for ch in 0..2 {
            let input = if quiet { [0.0; BANDS] } else { self.bands[ch] };
            self.bars[ch].step(&input, dt);
            let (rms, peak) = if quiet {
                (audio::FLOOR_DB, audio::FLOOR_DB)
            } else {
                (
                    cx.store
                        .last(&audio::RMS_DB.idx(ch as u16))
                        .map(|(_, v)| v)
                        .unwrap_or(audio::FLOOR_DB),
                    cx.store
                        .last(&audio::PEAK_DB.idx(ch as u16))
                        .map(|(_, v)| v)
                        .unwrap_or(audio::FLOOR_DB),
                )
            };
            self.vu[ch].step(rms, peak, dt);
            moving |= self.bars[ch].moving() || self.vu[ch].moving();
        }
        // Animate while anything on screen is still moving; one more frame
        // after it settles so the resting picture is drawn.
        let was = self.moving;
        self.moving = moving;
        if moving || was {
            Redraw::Yes
        } else {
            Redraw::No
        }
    }

    fn on_key(&mut self, key: KeyEvent, _cx: &InputCx<'_>) -> Outcome {
        if let Some(p) = self.picker.as_mut() {
            return match key.code {
                KeyCode::Esc | KeyCode::Char('s') => {
                    self.picker = None;
                    Outcome::Command(self.enumerate(false))
                }
                KeyCode::Up => {
                    p.selected = p.selected.saturating_sub(1);
                    Outcome::Consumed
                }
                KeyCode::Down => {
                    p.selected = (p.selected + 1).min(p.sinks.len().saturating_sub(1));
                    Outcome::Consumed
                }
                KeyCode::Enter => {
                    let Some(s) = p.sinks.get(p.selected) else {
                        return Outcome::Consumed;
                    };
                    let name = s.name.clone();
                    self.picker = None;
                    Outcome::Command(Command::Source(
                        audio::SOURCE,
                        Control::Domain(Box::new(SetSink(name))),
                    ))
                }
                _ => Outcome::Consumed,
            };
        }
        match key.code {
            KeyCode::Char('m') => {
                self.mode = self.mode.next();
                Outcome::Consumed
            }
            KeyCode::Char('g') => {
                self.preset = self.preset.next();
                for b in &mut self.bars {
                    b.set_preset(self.preset);
                }
                Outcome::Consumed
            }
            KeyCode::Char('[') => {
                self.window = self.window.narrow();
                Outcome::Consumed
            }
            KeyCode::Char(']') => {
                self.window = self.window.widen();
                Outcome::Consumed
            }
            KeyCode::Char('s') => {
                self.picker = Some(Picker {
                    selected: 0,
                    sinks: self.sink.iter().cloned().collect(),
                });
                Outcome::Command(self.enumerate(true))
            }
            _ => Outcome::Ignored,
        }
    }

    fn view(&self, cx: &RenderCx<'_>) -> View {
        view::render(self, cx)
    }

    fn signature(&self, tier: usize) -> &'static [&'static str] {
        match tier {
            TIER_VU => &["L", "R"],
            TIER_MINI => &["L"],
            TIER_SCOPE => &["scope"],
            TIER_SPECTRUM => &["Hz"],
            // `LUFS` only appears with the `audio-lufs` feature's values;
            // the preset chip is always there.
            _ => &["preset"],
        }
    }

    fn on_visibility(&mut self, visible: bool) {
        if !visible {
            self.picker = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn options_parse_the_documented_forms() {
        let t: toml::Table = toml::from_str(
            r#"preset = "cava"
bars = 24
mode = "both"
fps = 60"#,
        )
        .unwrap();
        let o: Options = t.try_into().unwrap();
        assert_eq!(o.preset, Preset::Cava);
        assert_eq!(o.bars, BarCount::Fixed(24));
        assert_eq!(o.mode, Mode::Both);
        assert_eq!(o.fps, 60);
        let t: toml::Table = toml::from_str(r#"bars = "auto""#).unwrap();
        assert_eq!(t.try_into::<Options>().unwrap().bars, BarCount::Auto);
        let t: toml::Table = toml::from_str(r#"bars = "many""#).unwrap();
        assert!(t.try_into::<Options>().is_err());
        let t: toml::Table = toml::from_str(r#"colour = "red""#).unwrap();
        assert!(
            Audio::from_table(&t).is_err(),
            "unknown options are rejected"
        );
        assert_eq!(
            Audio::new(Options {
                fps: 200,
                ..Options::default()
            })
            .options()
            .fps,
            60
        );
    }

    #[test]
    fn the_window_narrows_and_widens_within_the_bands() {
        let w = Window::default();
        let n = w.narrow();
        assert_eq!((n.lo, n.hi), (4, 60));
        assert_eq!(n.widen(), w);
        assert_eq!(w.widen(), w, "already the whole span");
        let mut t = w;
        for _ in 0..40 {
            t = t.narrow();
        }
        assert!(t.len() >= Window::MIN, "{t:?}");
        assert!((w.hz(0) - 30.0).abs() < 1e-9);
        assert!((w.hz(BANDS) - 16_000.0).abs() < 1e-6);
    }

    #[test]
    fn resample_max_pools_and_pads() {
        assert_eq!(resample_max(&[0.1, 0.9, 0.2, 0.3], 2), [0.9, 0.3]);
        assert_eq!(resample_max(&[0.5], 3), [0.5, 0.5, 0.5]);
        assert_eq!(resample_max(&[], 2), [0.0, 0.0]);
        assert_eq!(resample_max(&[1.0, 2.0], 0), Vec::<f32>::new());
    }
}
