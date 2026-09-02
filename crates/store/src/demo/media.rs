//! Deterministic media synthesis (§12.5, brief arc 6): a fake player with
//! three tracks — one long, one short, one with no length (a stream) — a
//! title long enough to scroll, and a procedurally drawn cover so the art
//! path is exercised headlessly. The store decodes no images (it depends on
//! nothing but serde), so the synth writes RGB directly (D56 amendment).

use std::sync::Arc;
use std::time::Duration;

use crate::key::Datum;
use crate::keys::media::{
    self, Art, Caps, History, HistoryItem, NowPlaying, PlayStatus, PlayerInfo, Players,
};
use crate::msg::{Batch, Sample};
use crate::source::{Cadence, Source, SourceCtx, SourceInfo, SourceState, SourceStatus};
use crate::ts::Ts;

/// The synthetic player's bus name and identity.
pub const BUS: &str = "org.mpris.MediaPlayer2.demo.synth";

/// Not the app's own name: the identity is printed in the tile, and a test
/// that asks "is the tab bar hidden?" looks for the word `gridwatch`.
pub const IDENTITY: &str = "Demo Player";
/// The cover's size in pixels (square).
pub const ART_PX: u16 = 64;

struct Track {
    title: &'static str,
    artist: &'static str,
    album: &'static str,
    url: &'static str,
    len_us: Option<i64>,
    hue: f64,
}

/// Three tracks: a long one, a short one, and a stream with no length.
const TRACKS: &[Track] = &[
    Track {
        title: "Probably Stolen — ROCK BOTTOM: The First Crate Of Many (Days 51-65)",
        artist: "CthuLuck",
        album: "Crate Digging",
        url: "https://example.invalid/watch?v=one",
        len_us: Some(119 * 60 * 1_000_000),
        hue: 0.58,
    },
    Track {
        title: "Short Interlude",
        artist: "Demo Set",
        album: "Demo Set",
        url: "https://example.invalid/watch?v=two",
        len_us: Some(45 * 1_000_000),
        hue: 0.92,
    },
    Track {
        title: "SomaFM · Groove Salad (live)",
        artist: "SomaFM",
        album: "",
        url: "https://example.invalid/stream",
        len_us: None,
        hue: 0.32,
    },
];

/// Seconds each track plays before the synth moves on.
const TRACK_SECS: f64 = 40.0;

fn hsv(h: f64, s: f64, v: f64) -> (u8, u8, u8) {
    let i = (h * 6.0).floor();
    let f = h * 6.0 - i;
    let (p, q, t) = (v * (1.0 - s), v * (1.0 - f * s), v * (1.0 - (1.0 - f) * s));
    let (r, g, b) = match (i as i64).rem_euclid(6) {
        0 => (v, t, p),
        1 => (q, v, p),
        2 => (p, v, t),
        3 => (p, q, v),
        4 => (t, p, v),
        _ => (v, p, q),
    };
    ((r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8)
}

/// The cover for a track: a diagonal gradient in the track's hue with a
/// darker border, drawn once per track.
pub fn art_for(track: u64, hue: f64) -> Art {
    let n = usize::from(ART_PX);
    let mut rgb = Vec::with_capacity(n * n * 3);
    for y in 0..n {
        for x in 0..n {
            let edge = x < 3 || y < 3 || x >= n - 3 || y >= n - 3;
            let t = (x + y) as f64 / (2.0 * n as f64);
            let (r, g, b) = if edge {
                (16, 16, 20)
            } else {
                hsv((hue + t * 0.15).fract(), 0.55, 0.35 + 0.5 * t)
            };
            rgb.extend_from_slice(&[r, g, b]);
        }
    }
    Art {
        track,
        w: ART_PX,
        h: ART_PX,
        rgb,
    }
}

#[derive(Clone, Debug, Default)]
pub struct MediaSynth {
    last_track: Option<u64>,
    history: Vec<HistoryItem>,
    players_sent: bool,
}

impl MediaSynth {
    pub fn new(_seed: u64) -> MediaSynth {
        MediaSynth::default()
    }

    fn track_at(at: Ts) -> (&'static Track, f64) {
        let t = at.as_secs_f64();
        let idx = ((t / TRACK_SECS).floor() as usize) % TRACKS.len();
        (&TRACKS[idx], t % TRACK_SECS)
    }

    /// What `media.now` says at `at`.
    pub fn now_at(at: Ts) -> NowPlaying {
        let (track, into) = MediaSynth::track_at(at);
        let hash = NowPlaying::track_hash(track.title, track.artist, track.album, track.url);
        // The second track pauses for its last ten seconds, so a tile's
        // "paused" path and its frozen clock are exercised under `--demo`.
        let status = if track.len_us == Some(45 * 1_000_000) && into > 30.0 {
            PlayStatus::Paused
        } else {
            PlayStatus::Playing
        };
        NowPlaying {
            player: "demo.synth".into(),
            identity: IDENTITY.into(),
            title: track.title.into(),
            artist: track.artist.into(),
            album: track.album.into(),
            url: track.url.into(),
            status,
            pos_us: (into * 1_000_000.0) as i64,
            read_at: at,
            len_us: track.len_us,
            rate: 1.0,
            volume: 0.8,
            caps: Caps {
                play_pause: true,
                next: true,
                prev: true,
                seek: track.len_us.is_some(),
                control: true,
                raise: false,
            },
            track: hash,
        }
    }

    pub fn tick_at(&mut self, at: Ts) -> Batch {
        let now = MediaSynth::now_at(at);
        let (track, _) = MediaSynth::track_at(at);
        let mut samples = Vec::with_capacity(6);
        let changed = self.last_track != Some(now.track);
        samples.push(Sample {
            id: media::NOW.id.clone(),
            datum: Datum::Record(Arc::new(now.clone())),
        });
        if let Some(f) = now.fraction(at) {
            samples.push(Sample {
                id: media::POS_PCT.id.clone(),
                datum: Datum::Scalar(f * 100.0),
            });
        }
        if changed {
            self.last_track = Some(now.track);
            samples.push(Sample {
                id: media::ART.id.clone(),
                datum: Datum::Record(Arc::new(art_for(now.track, track.hue))),
            });
            self.history.push(HistoryItem {
                track: now.track,
                title: now.title.clone(),
                artist: now.artist.clone(),
                at,
            });
            if self.history.len() > 50 {
                self.history.remove(0);
            }
            samples.push(Sample {
                id: media::HISTORY.id.clone(),
                datum: Datum::Record(Arc::new(History {
                    tracks: self.history.clone(),
                })),
            });
        }
        if !self.players_sent || changed {
            self.players_sent = true;
            samples.push(Sample {
                id: media::PLAYERS.id.clone(),
                datum: Datum::Record(Arc::new(Players {
                    list: vec![PlayerInfo {
                        bus: BUS.into(),
                        identity: IDENTITY.into(),
                        status: now.status,
                        is_current: true,
                    }],
                })),
            });
        }
        Batch {
            source: media::SOURCE,
            at,
            samples,
        }
    }
}

/// The mpris source's static info (§5): event-driven, with `Position`
/// polled at 1 Hz while Playing; nothing while hidden.
pub fn media_info() -> SourceInfo {
    SourceInfo {
        id: media::SOURCE,
        produces: &["media.*"],
        cadence: Cadence {
            hidden: None,
            visible: Duration::from_millis(1000),
            focused: Duration::from_millis(1000),
            always_on: false,
        },
        requires: &[],
    }
}

struct MediaDemoSource {
    seed: u64,
}

impl Source for MediaDemoSource {
    fn info(&self) -> SourceInfo {
        media_info()
    }

    fn run(self: Box<Self>, cx: SourceCtx) {
        let mut synth = MediaSynth::new(self.seed);
        cx.status(SourceStatus {
            state: SourceState::Ok,
            reason: Some(Arc::from("synthetic (demo)")),
            hint: None,
            since: cx.clock.now(),
            last_sample: None,
            dropped: 0,
            restarts: 0,
        });
        loop {
            while cx.try_control().is_some() {}
            if cx.stopped() {
                return;
            }
            let Some(cadence) = self.info().cadence.for_level(cx.demand.level()) else {
                if !cx.sleep_until(cx.next_deadline(Duration::from_secs(1))) {
                    return;
                }
                continue;
            };
            if !cx.sleep_until(cx.next_deadline(cadence)) {
                return;
            }
            let at = cx.clock.now();
            let b = synth.tick_at(at);
            cx.emit(at, b.samples);
        }
    }
}

pub fn media_demo(seed: u64) -> Box<dyn Source> {
    Box::new(MediaDemoSource { seed })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn three_tracks_with_art_history_and_a_stream() {
        let mut s = MediaSynth::new(1);
        let mut arts = 0;
        let mut streams = 0;
        // 119 s = the three tracks once (the fourth cycle would repeat
        // the first and publish its cover again).
        for i in 1..=119 {
            let at = Ts(i * 1_000_000_000);
            let b = s.tick_at(at);
            assert_eq!(b.source, media::SOURCE);
            arts += b
                .samples
                .iter()
                .filter(|x| x.id.name == "media.art")
                .count();
            let now = MediaSynth::now_at(at);
            if now.len_us.is_none() {
                streams += 1;
                assert!(now.fraction(at).is_none());
            }
        }
        assert_eq!(arts, 3, "one cover per track");
        assert!(streams > 20, "the stream plays for a while: {streams}");
        // Deterministic per Ts.
        let a = MediaSynth::new(1).tick_at(Ts(5_000_000_000));
        let b = MediaSynth::new(1).tick_at(Ts(5_000_000_000));
        assert_eq!(a.samples.len(), b.samples.len());
        let art = art_for(7, 0.5);
        assert!(art.is_valid());
        assert_eq!((art.w, art.h), (ART_PX, ART_PX));
        assert_eq!(art.pixel(0, 0), (16, 16, 20), "the border");
        assert_ne!(art.pixel(32, 32), art.pixel(40, 40), "a gradient");
        // The second track pauses near its end.
        let paused = MediaSynth::now_at(Ts(75_000_000_000));
        assert_eq!(paused.status, PlayStatus::Paused);
        assert_eq!(paused.pos_at(Ts(80_000_000_000)), paused.pos_us);
    }
}
