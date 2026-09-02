//! The MPRIS metadata decoder (brief arc 6 seam 2): `a{sv}` reduced to the
//! shapes players actually send, so decoding is a pure function over a small
//! value enum and every recorded fixture is a test — no bus required.
//!
//! Verified on torch: Firefox sends `xesam:artist` as a **one-element array**,
//! `xesam:album` as an **empty string**, `mpris:trackid` as an **object
//! path** (not a string), `mpris:length` in **microseconds**, and
//! `mpris:artUrl` as a `file://` PNG.

use std::collections::BTreeMap;

use gridwatch_store::keys::media::NowPlaying;
use serde::Deserialize;

/// The subset of D-Bus values MPRIS metadata uses. A fixture is JSON in
/// exactly this shape; the zbus adapter converts `OwnedValue` into it.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MetaValue {
    /// `s` and `o` (an object path is a string to everyone but D-Bus).
    Str(String),
    /// `as` — a string array (`xesam:artist`, `xesam:genre`).
    Strs(Vec<String>),
    /// `x`, `t`, `i`, `u` — every integer flavour.
    Int(i64),
    /// `d`.
    Float(f64),
    Bool(bool),
    /// Anything else, kept so a fixture can record it without the decoder
    /// caring.
    Other,
}

impl MetaValue {
    pub fn as_str(&self) -> Option<&str> {
        match self {
            MetaValue::Str(s) => Some(s),
            MetaValue::Strs(v) => v.first().map(String::as_str),
            _ => None,
        }
    }

    /// Every artist joined, the way a player would print them.
    pub fn as_joined(&self) -> Option<String> {
        match self {
            MetaValue::Str(s) => Some(s.clone()),
            MetaValue::Strs(v) if !v.is_empty() => Some(v.join(", ")),
            _ => None,
        }
    }

    pub fn as_int(&self) -> Option<i64> {
        match self {
            MetaValue::Int(i) => Some(*i),
            MetaValue::Float(f) if f.is_finite() => Some(*f as i64),
            MetaValue::Str(s) => s.parse().ok(),
            _ => None,
        }
    }
}

pub type Metadata = BTreeMap<String, MetaValue>;

/// What a tile needs out of one metadata dictionary.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TrackMeta {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub url: String,
    pub art_url: String,
    /// `mpris:length` in µs; `None` (or ≤ 0) means a stream.
    pub len_us: Option<i64>,
    pub track_id: String,
    pub track: u64,
}

/// Decode `a{sv}` into what the store publishes. Missing keys are empty,
/// never an error: half the players in the world omit half of them.
pub fn decode(meta: &Metadata) -> TrackMeta {
    let s = |k: &str| {
        meta.get(k)
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string()
    };
    let title = s("xesam:title");
    let artist = meta
        .get("xesam:artist")
        .or_else(|| meta.get("xesam:albumArtist"))
        .and_then(|v| v.as_joined())
        .unwrap_or_default();
    let album = s("xesam:album");
    let url = s("xesam:url");
    let len_us = meta
        .get("mpris:length")
        .and_then(|v| v.as_int())
        .filter(|l| *l > 0);
    TrackMeta {
        track: NowPlaying::track_hash(&title, &artist, &album, &url),
        title,
        artist,
        album,
        url,
        art_url: s("mpris:artUrl"),
        len_us,
        track_id: s("mpris:trackid"),
    }
}

/// A title for a track that has none: the URL's last useful part, else the
/// player's identity. A blank tile is worse than a guess that says where it
/// came from.
pub fn fallback_title(meta: &TrackMeta, identity: &str) -> String {
    if !meta.title.is_empty() {
        return meta.title.clone();
    }
    let from_url = meta
        .url
        .rsplit('/')
        .find(|p| !p.is_empty())
        .unwrap_or_default();
    if from_url.is_empty() {
        identity.to_string()
    } else {
        from_url.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> Metadata {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/mpris")
            .join(name);
        let text =
            std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        serde_json::from_str(&text).expect("the fixture parses")
    }

    /// The Firefox/YouTube dictionary recorded on torch (`busctl --user`).
    #[test]
    fn firefox_youtube_decodes() {
        let m = decode(&fixture("firefox-youtube.json"));
        assert_eq!(
            m.title,
            "Probably Stolen ROCK BOTTOM: The First Crate Of Many (Days 51-65)"
        );
        assert_eq!(m.artist, "CthuLuck", "a one-element `as`");
        assert_eq!(m.album, "", "Firefox sends an empty album");
        assert_eq!(m.url, "https://www.youtube.com/watch?v=wkWwIgUmtrk");
        assert_eq!(m.len_us, Some(7_170_000_000));
        assert!(m.art_url.starts_with("file:///home/"));
        assert_eq!(
            m.track_id, "/org/mpris/MediaPlayer2/firefox",
            "an object path decodes like a string"
        );
        assert_eq!(
            m.track,
            NowPlaying::track_hash(&m.title, &m.artist, &m.album, &m.url)
        );
    }

    #[test]
    fn a_stream_has_no_length_and_a_title_falls_back() {
        let m = decode(&fixture("stream-no-length.json"));
        assert_eq!(m.len_us, None, "0 or absent is a stream");
        assert_eq!(m.artist, "SomaFM");
        let m = decode(&fixture("no-title.json"));
        assert_eq!(m.title, "");
        assert_eq!(fallback_title(&m, "Firefox"), "track.mp3");
        let bare = TrackMeta::default();
        assert_eq!(fallback_title(&bare, "Firefox"), "Firefox");
    }

    #[test]
    fn several_artists_join_and_odd_shapes_survive() {
        let m = decode(&fixture("multi-artist.json"));
        assert_eq!(m.artist, "Boards of Canada, Autechre");
        // A player that sends length as a double or a string.
        let mut meta = Metadata::new();
        meta.insert("mpris:length".into(), MetaValue::Float(1.5e7));
        assert_eq!(decode(&meta).len_us, Some(15_000_000));
        meta.insert("mpris:length".into(), MetaValue::Str("42".into()));
        assert_eq!(decode(&meta).len_us, Some(42));
        meta.insert("mpris:length".into(), MetaValue::Int(0));
        assert_eq!(decode(&meta).len_us, None, "zero is not a length");
        meta.insert("mpris:length".into(), MetaValue::Other);
        assert_eq!(decode(&meta).len_us, None);
        // An empty dictionary decodes to empty strings, not a panic.
        let empty = decode(&Metadata::new());
        assert_eq!(empty.title, "");
        assert_eq!(empty.len_us, None);
        // albumArtist stands in for a missing artist.
        let mut meta = Metadata::new();
        meta.insert(
            "xesam:albumArtist".into(),
            MetaValue::Strs(vec!["Various".into()]),
        );
        assert_eq!(decode(&meta).artist, "Various");
    }
}
