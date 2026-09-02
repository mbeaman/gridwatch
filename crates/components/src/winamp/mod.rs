//! The Winamp tile (§8, brief arc 6 seam 3): the MPRIS source's current
//! track in classic-skin form — a scrolling marquee, big elapsed digits, the
//! position bar, the volume slider, a transport row that greys what the
//! player cannot do, the album art through the ui crate's halfblock painter,
//! and a spectrum borrowed from the `audio` source when it is running (a
//! static skin when it is not). Components never touch D-Bus: transport keys
//! leave as `Command::Source(mpris, Control::Domain(MediaCmd))`.

pub mod marquee;
mod view;

use std::borrow::Cow;

use gridwatch_store::keys::{audio, media};
use gridwatch_store::{Control, Detail, KeyCode, KeyEvent, Ts};
use gridwatch_ui::component::{
    BuildCx, BuildError, Chrome, Command, Component, ComponentDef, Footprint, InputCx, KeyHint,
    Manifest, Outcome, Redraw, RedrawPolicy, RenderCx, Size, TickCx, Tier,
};
use gridwatch_ui::view::View;
use serde::{Deserialize, Serialize};

pub static MANIFEST: Manifest = Manifest {
    kind: "winamp",
    name: "now playing",
    summary: "the MPRIS player in classic-skin form: marquee, big digits, posbar, transport, art and the spectrum",
    contract: 1,
    footprints: &[
        Footprint { w: 1, h: 1 },
        Footprint { w: 2, h: 1 },
        Footprint { w: 4, h: 2 },
        Footprint { w: 6, h: 3 },
    ],
    default_footprint: Footprint { w: 4, h: 2 },
    requires: &[],
    optional: &[gridwatch_store::Capability::DbusSession],
    sources: &[media::SOURCE],
    // The vis borrows the audio source's bands when it is running.
    optional_sources: &[audio::SOURCE],
    chrome: Chrome::Themed,
    keys: &[
        KeyHint {
            key: "x c v",
            does: "play pause stop",
        },
        KeyHint {
            key: "z b",
            does: "prev next",
        },
        KeyHint {
            key: "← →",
            does: "seek 5 s",
        },
        KeyHint {
            key: "+ -",
            does: "volume",
        },
        KeyHint {
            key: "p",
            does: "player",
        },
        KeyHint {
            key: "r",
            does: "raise",
        },
    ],
    example_options: "options = { art = true, vis = \"bars\" }",
};

static TIERS: &[Tier] = &[
    Tier {
        name: "status",
        min: Size::new(8, 3),
        adds: &["the play glyph", "a marquee", "a two-cell posbar"],
        zoom_only: false,
    },
    Tier {
        name: "shade",
        min: Size::new(24, 3),
        adds: &["the elapsed clock", "a mini spectrum"],
        zoom_only: false,
    },
    Tier {
        name: "main",
        min: Size::new(40, 10),
        adds: &["big digits", "the spectrum", "volume", "the transport row"],
        zoom_only: false,
    },
    Tier {
        name: "main+art",
        min: Size::new(60, 12),
        adds: &["the album art"],
        zoom_only: false,
    },
    Tier {
        name: "full",
        min: Size::new(100, 24),
        adds: &["the playlist", "the player list"],
        zoom_only: true,
    },
];

pub const TIER_STATUS: usize = 0;
pub const TIER_SHADE: usize = 1;
pub const TIER_MAIN: usize = 2;
pub const TIER_ART: usize = 3;
pub const TIER_FULL: usize = 4;

/// The animation the tile asks for while something moves.
pub const FPS: u8 = 10;
/// One `←`/`→` press.
pub const SEEK_US: i64 = 5_000_000;
/// One `+`/`-` press.
pub const VOLUME_STEP: f64 = 0.05;

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Vis {
    #[default]
    Bars,
    Off,
}

/// View-only instance options (§9).
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Options {
    pub art: bool,
    pub vis: Vis,
    pub fps: u8,
}

impl Default for Options {
    fn default() -> Options {
        Options {
            art: true,
            vis: Vis::Bars,
            fps: FPS,
        }
    }
}

pub const OPTION_NAMES: &[&str] = &["art", "vis", "fps"];

pub struct Winamp {
    options: Options,
    /// The track the marquee is scrolling, so a change restarts it.
    track: u64,
    /// `cx.now` when this track's marquee started.
    since: Ts,
    /// The playlist's scroll (the `full` tier).
    scroll: usize,
    /// The last tick's animation decision, so a settled tile stops asking
    /// for frames (the 5a rule).
    moving: bool,
    seen: Option<Ts>,
}

impl Winamp {
    pub fn new(options: Options) -> Winamp {
        Winamp {
            options: Options {
                fps: options.fps.clamp(1, 30),
                ..options
            },
            track: 0,
            since: Ts::ZERO,
            scroll: 0,
            moving: false,
            seen: None,
        }
    }

    pub fn from_table(options: &toml::Table) -> Result<Winamp, BuildError> {
        let parsed: Options = options
            .clone()
            .try_into()
            .map_err(|e| BuildError(format!("[[components]] options: {e}")))?;
        Ok(Winamp::new(parsed))
    }

    pub fn options(&self) -> &Options {
        &self.options
    }

    pub fn scroll(&self) -> usize {
        self.scroll
    }

    /// The marquee's clock: time since this track started scrolling, so a
    /// new track starts at the beginning of its title.
    pub fn marquee_at(&self, now: Ts) -> Ts {
        Ts(now.0.saturating_sub(self.since.0))
    }

    fn cmd(cmd: media::MediaCmd) -> Outcome {
        Outcome::Command(Command::Source(
            media::SOURCE,
            Control::Domain(Box::new(cmd)),
        ))
    }
}

impl Default for Winamp {
    fn default() -> Winamp {
        Winamp::new(Options::default())
    }
}

fn build(cx: &mut BuildCx<'_>) -> Result<Box<dyn Component>, BuildError> {
    Ok(Box::new(Winamp::from_table(cx.options)?))
}

pub const DEF: fn() -> ComponentDef = || ComponentDef {
    manifest: &MANIFEST,
    build,
};

impl Component for Winamp {
    fn manifest(&self) -> &'static Manifest {
        &MANIFEST
    }

    fn title(&self, _max_width: u16, _cx: &TickCx<'_>) -> Cow<'static, str> {
        Cow::Borrowed("now playing")
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

    /// The tile animates while the marquee scrolls or the clock advances;
    /// a paused track with a title that fits asks for no frames.
    fn tick(&mut self, cx: &TickCx<'_>) -> Redraw {
        let now = cx.store.record(&media::NOW).map(|(_, n)| n.clone());
        let at = cx.store.last_sample(media::SOURCE);
        let fresh = at != self.seen;
        self.seen = at;
        let Some(now) = now else {
            let was = self.moving;
            self.moving = false;
            return if was { Redraw::Yes } else { Redraw::No };
        };
        if now.track != self.track {
            self.track = now.track;
            self.since = cx.now;
            self.scroll = 0;
        }
        // Playing means the clock advances and the marquee scrolls; paused
        // or stopped, nothing on the tile moves by itself (`view` knows the
        // width the marquee would need, `tick` does not, so "playing" is
        // the whole rule).
        let was = self.moving;
        self.moving = now.status == media::PlayStatus::Playing;
        if self.moving || was || fresh {
            Redraw::Yes
        } else {
            Redraw::No
        }
    }

    fn on_key(&mut self, key: KeyEvent, cx: &InputCx<'_>) -> Outcome {
        let now = cx.store.record(&media::NOW).map(|(_, n)| n.clone());
        let caps = now.as_ref().map(|n| n.caps).unwrap_or_default();
        match key.code {
            KeyCode::Char('x') if caps.play_pause => Winamp::cmd(media::MediaCmd::Play),
            KeyCode::Char('c') if caps.play_pause => Winamp::cmd(media::MediaCmd::Pause),
            KeyCode::Char('v') if caps.control => Winamp::cmd(media::MediaCmd::Stop),
            KeyCode::Char('b') if caps.next => Winamp::cmd(media::MediaCmd::Next),
            KeyCode::Char('z') if caps.prev => Winamp::cmd(media::MediaCmd::Prev),
            KeyCode::Left if caps.seek => Winamp::cmd(media::MediaCmd::SeekBy(-SEEK_US)),
            KeyCode::Right if caps.seek => Winamp::cmd(media::MediaCmd::SeekBy(SEEK_US)),
            KeyCode::Char('+') | KeyCode::Char('=') => {
                let v = now.as_ref().map(|n| n.volume).unwrap_or(0.0);
                Winamp::cmd(media::MediaCmd::SetVolume((v + VOLUME_STEP).min(1.0)))
            }
            KeyCode::Char('-') | KeyCode::Char('_') => {
                let v = now.as_ref().map(|n| n.volume).unwrap_or(0.0);
                Winamp::cmd(media::MediaCmd::SetVolume((v - VOLUME_STEP).max(0.0)))
            }
            KeyCode::Char('r') if caps.raise => Winamp::cmd(media::MediaCmd::Raise),
            KeyCode::Char('p') => {
                // Cycle: the next player after the current one, wrapping.
                let players = cx.store.record(&media::PLAYERS).map(|(_, p)| p.clone());
                let Some(players) = players.filter(|p| p.list.len() > 1) else {
                    return Outcome::Consumed;
                };
                let cur = players.list.iter().position(|p| p.is_current).unwrap_or(0);
                let next = &players.list[(cur + 1) % players.list.len()];
                Winamp::cmd(media::MediaCmd::Pick(next.bus.clone()))
            }
            KeyCode::Up => {
                self.scroll = self.scroll.saturating_sub(1);
                Outcome::Consumed
            }
            KeyCode::Down => {
                self.scroll = self.scroll.saturating_add(1);
                Outcome::Consumed
            }
            // A key the player cannot honour is still ours: the tile says
            // so rather than letting the shell reuse the letter.
            KeyCode::Char('x' | 'c' | 'v' | 'b' | 'z' | 'r') => Outcome::Consumed,
            _ => Outcome::Ignored,
        }
    }

    fn view(&self, cx: &RenderCx<'_>) -> View {
        view::render(self, cx)
    }

    fn signature(&self, tier: usize) -> &'static [&'static str] {
        match tier {
            // The elapsed clock: the one thing both small tiers always
            // draw, whatever the title's width does to the marquee.
            TIER_STATUS | TIER_SHADE => &[":"],
            TIER_MAIN | TIER_ART => &["vol"],
            _ => &["playlist"],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn options_and_the_marquee_clock() {
        let t: toml::Table = toml::from_str("art = false\nvis = \"off\"\nfps = 60").unwrap();
        let o: Options = t.try_into().unwrap();
        assert!(!o.art);
        assert_eq!(o.vis, Vis::Off);
        let w = Winamp::new(o);
        assert_eq!(w.options().fps, 30, "clamped");
        assert_eq!(w.redraw_policy(), RedrawPolicy::Animated { fps: 30 });
        let t: toml::Table = toml::from_str("colour = 1").unwrap();
        assert!(Winamp::from_table(&t).is_err());
        let w = Winamp {
            since: Ts(5_000_000_000),
            ..Winamp::default()
        };
        assert_eq!(w.marquee_at(Ts(7_000_000_000)), Ts(2_000_000_000));
        assert_eq!(w.marquee_at(Ts(1_000_000_000)), Ts(0), "never negative");
    }
}
