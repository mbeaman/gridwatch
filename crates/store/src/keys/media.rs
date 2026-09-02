//! Media keys (§8, brief arc 6 seam 1): what the MPRIS source publishes for
//! the Winamp tile — the current track, the players it can see, the decoded
//! album art and the recent history. `MediaCmd` lives here beside them: the
//! component boxes it into `Control::Domain` and the source downcasts it,
//! and components never depend on the sources crate (D55 amendment 1).

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::journal::JournalError;
use crate::key::{DatumKind, Key, KeyMeta, RecordValue, Unit};
use crate::source::SourceId;
use crate::ts::Ts;

pub const SOURCE: SourceId = SourceId("mpris");

/// `media.now` — the current player's track and position.
pub const NOW: Key<NowPlaying> = Key::new("media.now");
/// `media.players` — every player on the bus (the `p` cycle's list).
pub const PLAYERS: Key<Players> = Key::new("media.players");
/// `media.art` — the current track's cover, decoded to RGB8.
pub const ART: Key<Art> = Key::new("media.art");
/// `media.history` — the last distinct tracks (the playlist pane).
pub const HISTORY: Key<History> = Key::new("media.history");
/// `media.pos_pct` — position as a percentage, so a replayed journal can
/// draw the posbar without interpolating a Record's internals.
pub const POS_PCT: Key<f64> = Key::new("media.pos_pct");

/// The longest side the art is scaled to before it reaches the store.
pub const ART_MAX_PX: u32 = 256;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlayStatus {
    Playing,
    Paused,
    #[default]
    Stopped,
}

impl PlayStatus {
    /// MPRIS spells it `Playing` / `Paused` / `Stopped`.
    pub fn parse(s: &str) -> PlayStatus {
        match s {
            "Playing" => PlayStatus::Playing,
            "Paused" => PlayStatus::Paused,
            _ => PlayStatus::Stopped,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            PlayStatus::Playing => "playing",
            PlayStatus::Paused => "paused",
            PlayStatus::Stopped => "stopped",
        }
    }
}

/// What the player says it can do (`CanPlay`, `CanGoNext`, …): the tile
/// greys out what is unsupported instead of sending it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Caps {
    pub play_pause: bool,
    pub next: bool,
    pub prev: bool,
    pub seek: bool,
    pub control: bool,
    pub raise: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct NowPlaying {
    /// The bus name's suffix (`firefox.instance_1_107`).
    pub player: String,
    /// `org.mpris.MediaPlayer2.Identity` (`Firefox`).
    pub identity: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub url: String,
    pub status: PlayStatus,
    /// `Position` in microseconds, as read at `read_at`.
    pub pos_us: i64,
    /// The run-clock instant `pos_us` was read at: the tile interpolates
    /// from here instead of asking for a faster poll.
    pub read_at: Ts,
    /// `mpris:length` in microseconds; `None` ⇒ stream mode.
    pub len_us: Option<i64>,
    pub rate: f64,
    pub volume: f64,
    pub caps: Caps,
    /// `hash(title|artist|album|url)`: the identity of a track, so a tile
    /// notices a change without comparing four strings.
    pub track: u64,
}

impl NowPlaying {
    /// The track hash the source and the tests agree on.
    pub fn track_hash(title: &str, artist: &str, album: &str, url: &str) -> u64 {
        // FNV-1a over the four fields with a separator — stable across
        // runs and machines (a journal replays to the same number).
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for part in [title, artist, album, url] {
            for b in part.as_bytes().iter().chain(b"\x1f") {
                h ^= u64::from(*b);
                h = h.wrapping_mul(0x1000_0000_01b3);
            }
        }
        h
    }

    /// The position at `now`, interpolated while Playing (µs).
    pub fn pos_at(&self, now: Ts) -> i64 {
        if self.status != PlayStatus::Playing {
            return self.pos_us;
        }
        let dt = now.since(self.read_at).as_micros() as f64 * self.rate.max(0.0);
        let p = self.pos_us.saturating_add(dt as i64);
        match self.len_us {
            Some(len) if len > 0 => p.min(len),
            _ => p,
        }
    }

    /// Fraction of the track played, `None` in stream mode.
    pub fn fraction(&self, now: Ts) -> Option<f64> {
        let len = self.len_us.filter(|l| *l > 0)?;
        Some((self.pos_at(now) as f64 / len as f64).clamp(0.0, 1.0))
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PlayerInfo {
    pub bus: String,
    pub identity: String,
    pub status: PlayStatus,
    pub is_current: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Players {
    pub list: Vec<PlayerInfo>,
}

/// The decoded cover: RGB8, at most `ART_MAX_PX` on the long side.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Art {
    pub track: u64,
    pub w: u16,
    pub h: u16,
    pub rgb: Vec<u8>,
}

impl Art {
    /// The pixel at `(x, y)`, or black outside.
    pub fn pixel(&self, x: u16, y: u16) -> (u8, u8, u8) {
        if x >= self.w || y >= self.h {
            return (0, 0, 0);
        }
        let i = (usize::from(y) * usize::from(self.w) + usize::from(x)) * 3;
        match self.rgb.get(i..i + 3) {
            Some(p) => (p[0], p[1], p[2]),
            None => (0, 0, 0),
        }
    }

    /// A sane Record: the buffer really holds `w × h` RGB triples.
    pub fn is_valid(&self) -> bool {
        self.w > 0 && self.h > 0 && self.rgb.len() == usize::from(self.w) * usize::from(self.h) * 3
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct HistoryItem {
    pub track: u64,
    pub title: String,
    pub artist: String,
    pub at: Ts,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct History {
    pub tracks: Vec<HistoryItem>,
}

/// The first `Control::Domain` payload of arc 6 (seam 2): a transport
/// command from the tile. The component boxes it; the source downcasts.
#[derive(Clone, Debug, PartialEq)]
pub enum MediaCmd {
    PlayPause,
    Play,
    Pause,
    Stop,
    Next,
    Prev,
    /// Seek by microseconds, positive or negative.
    SeekBy(i64),
    /// Absolute volume 0..1.
    SetVolume(f64),
    Raise,
    /// Pin a player by bus name (`""` returns to automatic).
    Pick(String),
}

fn decode<T: for<'de> Deserialize<'de> + RecordValue>(
    v: serde_json::Value,
) -> Result<Arc<dyn RecordValue>, JournalError> {
    serde_json::from_value::<T>(v)
        .map(|t| Arc::new(t) as Arc<dyn RecordValue>)
        .map_err(|e| JournalError(e.to_string()))
}

fn decode_now(v: serde_json::Value) -> Result<Arc<dyn RecordValue>, JournalError> {
    decode::<NowPlaying>(v)
}

fn decode_players(v: serde_json::Value) -> Result<Arc<dyn RecordValue>, JournalError> {
    decode::<Players>(v)
}

fn decode_art(v: serde_json::Value) -> Result<Arc<dyn RecordValue>, JournalError> {
    decode::<Art>(v)
}

fn decode_history(v: serde_json::Value) -> Result<Arc<dyn RecordValue>, JournalError> {
    decode::<History>(v)
}

pub static METAS: &[KeyMeta] = &[
    KeyMeta {
        name: "media.now",
        unit: Unit::None,
        kind: DatumKind::Record,
        source: SOURCE,
        doc: "the current player's track: title/artist/album/url, status, position (pos_us as read at read_at), length (absent = a stream), rate, volume, capabilities and the track hash",
        decode: Some(decode_now),
    },
    KeyMeta {
        name: "media.players",
        unit: Unit::None,
        kind: DatumKind::Record,
        source: SOURCE,
        doc: "every MPRIS player on the session bus with its status, and which one the tile is showing",
        decode: Some(decode_players),
    },
    KeyMeta {
        name: "media.art",
        unit: Unit::None,
        kind: DatumKind::Record,
        source: SOURCE,
        doc: "the current track's cover decoded to RGB8, at most 256 px on the long side; absent when the track has none",
        decode: Some(decode_art),
    },
    KeyMeta {
        name: "media.history",
        unit: Unit::None,
        kind: DatumKind::Record,
        source: SOURCE,
        doc: "the last distinct tracks seen, newest last (the playlist pane)",
        decode: Some(decode_history),
    },
    KeyMeta {
        name: "media.pos_pct",
        unit: Unit::Percent,
        kind: DatumKind::Scalar,
        source: SOURCE,
        doc: "position as a percentage of the track's length; absent in stream mode",
        decode: None,
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn track_hash_is_stable_and_position_interpolates() {
        let a = NowPlaying::track_hash("t", "a", "al", "u");
        assert_eq!(a, NowPlaying::track_hash("t", "a", "al", "u"));
        assert_ne!(a, NowPlaying::track_hash("t", "a", "al", "u2"));
        // The separator keeps `ab|c` from colliding with `a|bc`.
        assert_ne!(
            NowPlaying::track_hash("ab", "c", "", ""),
            NowPlaying::track_hash("a", "bc", "", "")
        );
        let mut now = NowPlaying {
            status: PlayStatus::Playing,
            pos_us: 1_000_000,
            read_at: Ts(1_000_000_000),
            len_us: Some(10_000_000),
            rate: 1.0,
            ..NowPlaying::default()
        };
        assert_eq!(now.pos_at(Ts(3_000_000_000)), 3_000_000);
        assert_eq!(now.fraction(Ts(3_000_000_000)), Some(0.3));
        assert_eq!(
            now.pos_at(Ts(60_000_000_000)),
            10_000_000,
            "clamped to the length"
        );
        now.status = PlayStatus::Paused;
        assert_eq!(
            now.pos_at(Ts(9_000_000_000)),
            1_000_000,
            "paused stands still"
        );
        now.status = PlayStatus::Playing;
        now.len_us = None;
        assert_eq!(now.fraction(Ts(3_000_000_000)), None, "stream mode");
        assert_eq!(now.pos_at(Ts(3_000_000_000)), 3_000_000);
        now.rate = 2.0;
        assert_eq!(now.pos_at(Ts(3_000_000_000)), 5_000_000, "double speed");
    }

    #[test]
    fn art_pixels_and_validity() {
        let art = Art {
            track: 1,
            w: 2,
            h: 1,
            rgb: vec![1, 2, 3, 4, 5, 6],
        };
        assert!(art.is_valid());
        assert_eq!(art.pixel(0, 0), (1, 2, 3));
        assert_eq!(art.pixel(1, 0), (4, 5, 6));
        assert_eq!(art.pixel(2, 0), (0, 0, 0), "outside is black");
        assert!(!Art::default().is_valid());
        assert!(
            !Art {
                w: 4,
                h: 4,
                ..art.clone()
            }
            .is_valid(),
            "a short buffer is not valid"
        );
        assert_eq!(PlayStatus::parse("Playing"), PlayStatus::Playing);
        assert_eq!(PlayStatus::parse("nonsense"), PlayStatus::Stopped);
    }
}
