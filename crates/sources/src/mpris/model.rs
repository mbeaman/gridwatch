//! The player model (brief arc 6 seam 2): every rule about *what the store
//! should say* given what the bus reported, as a pure state machine — the
//! current player's choice, the position bookkeeping, stream mode, the
//! history ring and which samples a change produces. The async task in
//! `mod.rs` only feeds it events and ships what it returns, so discovery,
//! the 1 Hz poll gating and the track-change rules are unit-tested without
//! D-Bus.

use std::collections::BTreeMap;
use std::sync::Arc;

use gridwatch_store::keys::media::{
    self, Caps, History, HistoryItem, NowPlaying, PlayStatus, PlayerInfo, Players,
};
use gridwatch_store::{Datum, Sample, Ts};

use super::meta::{TrackMeta, fallback_title};

/// What one player on the bus looks like to the model.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PlayerState {
    pub bus: String,
    pub identity: String,
    pub status: PlayStatus,
    pub meta: TrackMeta,
    /// `Position` as last read, and when.
    pub pos_us: i64,
    pub read_at: Ts,
    pub rate: f64,
    pub volume: f64,
    pub caps: Caps,
    /// The run-clock instant this player last changed anything: the
    /// tie-breaker when two are Playing.
    pub changed_at: Ts,
    /// Set when the player first reported this track, for stream mode's
    /// local elapsed clock.
    pub track_since: Ts,
}

impl PlayerState {
    pub fn new(bus: &str, at: Ts) -> PlayerState {
        PlayerState {
            bus: bus.to_string(),
            rate: 1.0,
            volume: 1.0,
            changed_at: at,
            track_since: at,
            read_at: at,
            ..PlayerState::default()
        }
    }

    /// The short name a tile shows: the bus name minus the MPRIS prefix.
    pub fn short(&self) -> &str {
        self.bus
            .strip_prefix("org.mpris.MediaPlayer2.")
            .unwrap_or(&self.bus)
    }
}

/// The events the async task feeds in. Everything carries the run-clock
/// instant it happened at, so replay and tests share one clock.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    /// A player appeared (or was found by `ListNames`).
    Added {
        bus: String,
        identity: String,
    },
    /// Its `PlaybackStatus` changed.
    Status {
        bus: String,
        status: PlayStatus,
    },
    /// Its `Metadata` changed.
    Meta {
        bus: String,
        meta: TrackMeta,
    },
    /// `Position` was read (the 1 Hz poll, or a `Seeked` signal).
    Position {
        bus: String,
        pos_us: i64,
    },
    Rate {
        bus: String,
        rate: f64,
    },
    Volume {
        bus: String,
        volume: f64,
    },
    Caps {
        bus: String,
        caps: Caps,
    },
    /// The bus name went away.
    Removed {
        bus: String,
    },
    /// The user picked a player (`""` = automatic).
    Pick(String),
}

/// The model: every known player, the pinned choice and the history.
#[derive(Clone, Debug, Default)]
pub struct Model {
    players: BTreeMap<String, PlayerState>,
    /// A pinned bus name (the `p` key); empty = choose automatically.
    pinned: String,
    /// The bus the store is currently describing.
    current: String,
    history: Vec<HistoryItem>,
    history_cap: usize,
    /// The last `media.now` published, to notice a real change.
    last: Option<NowPlaying>,
    last_players: Option<Players>,
}

impl Model {
    pub fn new(history_cap: usize) -> Model {
        Model {
            history_cap: history_cap.clamp(1, 500),
            ..Model::default()
        }
    }

    pub fn players(&self) -> impl Iterator<Item = &PlayerState> {
        self.players.values()
    }

    pub fn current(&self) -> Option<&PlayerState> {
        self.players.get(&self.current)
    }

    /// Playing beats everything, then the most recently changed, then the
    /// bus name — and a pinned player wins outright while it exists.
    pub fn choose(&self) -> String {
        if !self.pinned.is_empty() && self.players.contains_key(&self.pinned) {
            return self.pinned.clone();
        }
        self.players
            .values()
            .max_by(|a, b| {
                let key = |p: &PlayerState| (p.status == PlayStatus::Playing, p.changed_at.0);
                key(a).cmp(&key(b)).then_with(|| b.bus.cmp(&a.bus))
            })
            .map(|p| p.bus.clone())
            .unwrap_or_default()
    }

    /// Does this player need its `Position` polled at `now`? Only the
    /// current one, only while Playing (§5's cadence row).
    pub fn wants_poll(&self, bus: &str) -> bool {
        self.current == bus
            && self
                .players
                .get(bus)
                .is_some_and(|p| p.status == PlayStatus::Playing)
    }

    /// Feed one event; returns the samples the store should get. An event
    /// that changes nothing returns none.
    pub fn apply(&mut self, ev: Event, at: Ts) -> Vec<Sample> {
        match ev {
            Event::Added { bus, identity } => {
                let p = self
                    .players
                    .entry(bus.clone())
                    .or_insert_with(|| PlayerState::new(&bus, at));
                p.identity = identity;
                p.changed_at = at;
            }
            Event::Removed { bus } => {
                self.players.remove(&bus);
                if self.pinned == bus {
                    self.pinned.clear();
                }
            }
            Event::Pick(bus) => {
                self.pinned = bus;
            }
            Event::Status { bus, status } => {
                if let Some(p) = self.players.get_mut(&bus) {
                    if p.status != status {
                        p.changed_at = at;
                    }
                    p.status = status;
                }
            }
            Event::Meta { bus, meta } => {
                if let Some(p) = self.players.get_mut(&bus) {
                    if p.meta.track != meta.track {
                        p.track_since = at;
                        // A new track starts at zero until the player says
                        // otherwise: keeping the old position would show
                        // the previous track's clock.
                        p.pos_us = 0;
                        p.read_at = at;
                    }
                    p.meta = meta;
                    p.changed_at = at;
                }
            }
            Event::Position { bus, pos_us } => {
                if let Some(p) = self.players.get_mut(&bus) {
                    p.pos_us = pos_us.max(0);
                    p.read_at = at;
                }
            }
            Event::Rate { bus, rate } => {
                if let Some(p) = self.players.get_mut(&bus) {
                    p.rate = if rate.is_finite() && rate > 0.0 {
                        rate
                    } else {
                        1.0
                    };
                }
            }
            Event::Volume { bus, volume } => {
                if let Some(p) = self.players.get_mut(&bus) {
                    p.volume = volume.clamp(0.0, 1.0);
                }
            }
            Event::Caps { bus, caps } => {
                if let Some(p) = self.players.get_mut(&bus) {
                    p.caps = caps;
                }
            }
        }
        self.current = self.choose();
        self.samples(at)
    }

    /// `media.now` for the current player. The position it carries is the
    /// one the player reported (with its `read_at`); the tile interpolates,
    /// so this needs no clock of its own.
    pub fn now(&self) -> Option<NowPlaying> {
        let p = self.current()?;
        // A player reporting Position 0 for a track with no length is a
        // stream: the elapsed clock runs from when we first saw the track
        // (the digest's Firefox case).
        let (pos_us, read_at) = if p.meta.len_us.is_none() && p.pos_us == 0 {
            (0, p.track_since)
        } else {
            (p.pos_us, p.read_at)
        };
        Some(NowPlaying {
            player: p.short().to_string(),
            identity: p.identity.clone(),
            title: fallback_title(&p.meta, &p.identity),
            artist: p.meta.artist.clone(),
            album: p.meta.album.clone(),
            url: p.meta.url.clone(),
            status: p.status,
            pos_us,
            read_at,
            len_us: p.meta.len_us,
            rate: p.rate,
            volume: p.volume,
            caps: p.caps,
            track: p.meta.track,
        })
    }

    fn players_record(&self) -> Players {
        Players {
            list: self
                .players
                .values()
                .map(|p| PlayerInfo {
                    bus: p.bus.clone(),
                    identity: p.identity.clone(),
                    status: p.status,
                    is_current: p.bus == self.current,
                })
                .collect(),
        }
    }

    /// What changed since the last call, as samples.
    fn samples(&mut self, at: Ts) -> Vec<Sample> {
        let mut out = Vec::with_capacity(4);
        let players = self.players_record();
        if self.last_players.as_ref() != Some(&players) {
            self.last_players = Some(players.clone());
            out.push(Sample {
                id: media::PLAYERS.id.clone(),
                datum: Datum::Record(Arc::new(players)),
            });
        }
        let Some(now) = self.now() else {
            // Every player left: say so once.
            if self.last.take().is_some() {
                out.push(Sample {
                    id: media::NOW.id.clone(),
                    datum: Datum::Record(Arc::new(NowPlaying::default())),
                });
            }
            return out;
        };
        let track_changed = self.last.as_ref().map(|l| l.track) != Some(now.track);
        // Position moves every second; only publish `media.now` when
        // something a viewer would notice changed (the tile interpolates).
        let notable = match &self.last {
            None => true,
            Some(l) => {
                l.track != now.track
                    || l.status != now.status
                    || l.player != now.player
                    || (l.volume - now.volume).abs() > 0.001
                    || (l.rate - now.rate).abs() > 0.001
                    || (now.pos_at(at) - l.pos_at(at)).abs() > 1_000_000
            }
        };
        if notable {
            if track_changed && !now.title.is_empty() {
                self.history.retain(|h| h.track != now.track);
                self.history.push(HistoryItem {
                    track: now.track,
                    title: now.title.clone(),
                    artist: now.artist.clone(),
                    at,
                });
                let cap = self.history_cap;
                if self.history.len() > cap {
                    let excess = self.history.len() - cap;
                    self.history.drain(0..excess);
                }
                out.push(Sample {
                    id: media::HISTORY.id.clone(),
                    datum: Datum::Record(Arc::new(History {
                        tracks: self.history.clone(),
                    })),
                });
            }
            if let Some(f) = now.fraction(at) {
                out.push(Sample {
                    id: media::POS_PCT.id.clone(),
                    datum: Datum::Scalar(f * 100.0),
                });
            }
            out.push(Sample {
                id: media::NOW.id.clone(),
                datum: Datum::Record(Arc::new(now.clone())),
            });
            self.last = Some(now);
        }
        out
    }

    /// The art URL the current track wants, when it changed.
    pub fn art_wanted(&self) -> Option<(u64, String)> {
        let p = self.current()?;
        (!p.meta.art_url.is_empty()).then(|| (p.meta.track, p.meta.art_url.clone()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta(title: &str, len: Option<i64>) -> TrackMeta {
        TrackMeta {
            track: NowPlaying::track_hash(title, "a", "", "u"),
            title: title.into(),
            artist: "a".into(),
            url: "u".into(),
            len_us: len,
            ..TrackMeta::default()
        }
    }

    fn names(out: &[Sample]) -> Vec<&str> {
        out.iter().map(|s| s.id.name).collect()
    }

    #[test]
    fn playing_wins_then_the_most_recent_then_the_name() {
        let mut m = Model::new(50);
        m.apply(
            Event::Added {
                bus: "org.mpris.MediaPlayer2.b".into(),
                identity: "B".into(),
            },
            Ts(1_000_000_000),
        );
        m.apply(
            Event::Added {
                bus: "org.mpris.MediaPlayer2.a".into(),
                identity: "A".into(),
            },
            Ts(2_000_000_000),
        );
        // Nothing is playing: the most recently changed wins.
        assert_eq!(m.choose(), "org.mpris.MediaPlayer2.a");
        m.apply(
            Event::Status {
                bus: "org.mpris.MediaPlayer2.b".into(),
                status: PlayStatus::Playing,
            },
            Ts(3_000_000_000),
        );
        assert_eq!(m.choose(), "org.mpris.MediaPlayer2.b", "playing wins");
        // A pin overrides both, and survives the other going away.
        m.apply(
            Event::Pick("org.mpris.MediaPlayer2.a".into()),
            Ts(4_000_000_000),
        );
        assert_eq!(m.choose(), "org.mpris.MediaPlayer2.a");
        m.apply(
            Event::Removed {
                bus: "org.mpris.MediaPlayer2.a".into(),
            },
            Ts(5_000_000_000),
        );
        assert_eq!(m.choose(), "org.mpris.MediaPlayer2.b", "the pin lapsed");
        // The last player leaving publishes an empty `now` once.
        let out = m.apply(
            Event::Removed {
                bus: "org.mpris.MediaPlayer2.b".into(),
            },
            Ts(6_000_000_000),
        );
        assert!(names(&out).contains(&"media.now"));
        assert!(m.now().is_none());
        assert!(
            m.apply(Event::Pick(String::new()), Ts(7_000_000_000))
                .iter()
                .all(|s| s.id.name != "media.now"),
            "and not again"
        );
    }

    #[test]
    fn only_the_current_playing_player_is_polled() {
        let mut m = Model::new(50);
        for (bus, status) in [("a", PlayStatus::Playing), ("b", PlayStatus::Paused)] {
            let bus = format!("org.mpris.MediaPlayer2.{bus}");
            m.apply(
                Event::Added {
                    bus: bus.clone(),
                    identity: bus.clone(),
                },
                Ts(1),
            );
            m.apply(Event::Status { bus, status }, Ts(2));
        }
        assert!(m.wants_poll("org.mpris.MediaPlayer2.a"));
        assert!(!m.wants_poll("org.mpris.MediaPlayer2.b"), "not current");
        m.apply(
            Event::Status {
                bus: "org.mpris.MediaPlayer2.a".into(),
                status: PlayStatus::Paused,
            },
            Ts(3),
        );
        assert!(
            !m.wants_poll("org.mpris.MediaPlayer2.a"),
            "paused is not polled"
        );
    }

    #[test]
    fn a_track_change_resets_the_clock_and_writes_history() {
        let mut m = Model::new(2);
        let bus = "org.mpris.MediaPlayer2.p".to_string();
        m.apply(
            Event::Added {
                bus: bus.clone(),
                identity: "P".into(),
            },
            Ts(0),
        );
        m.apply(
            Event::Status {
                bus: bus.clone(),
                status: PlayStatus::Playing,
            },
            Ts(0),
        );
        let out = m.apply(
            Event::Meta {
                bus: bus.clone(),
                meta: meta("one", Some(10_000_000)),
            },
            Ts(1_000_000_000),
        );
        assert!(names(&out).contains(&"media.history"));
        assert!(names(&out).contains(&"media.pos_pct"));
        m.apply(
            Event::Position {
                bus: bus.clone(),
                pos_us: 5_000_000,
            },
            Ts(2_000_000_000),
        );
        assert_eq!(m.now().unwrap().pos_us, 5_000_000);
        // A new track: the clock restarts even before a Position arrives.
        m.apply(
            Event::Meta {
                bus: bus.clone(),
                meta: meta("two", Some(10_000_000)),
            },
            Ts(3_000_000_000),
        );
        let now = m.now().unwrap();
        assert_eq!(now.pos_us, 0);
        assert_eq!(now.title, "two");
        // The history is capped and keeps the newest.
        m.apply(
            Event::Meta {
                bus: bus.clone(),
                meta: meta("three", Some(10_000_000)),
            },
            Ts(4_000_000_000),
        );
        let out = m.apply(
            Event::Meta {
                bus,
                meta: meta("four", Some(10_000_000)),
            },
            Ts(5_000_000_000),
        );
        let hist = out
            .iter()
            .find(|s| s.id.name == "media.history")
            .expect("history");
        let Datum::Record(r) = &hist.datum else {
            panic!()
        };
        let h = r.as_any().downcast_ref::<History>().unwrap();
        assert_eq!(h.tracks.len(), 2, "capped");
        assert_eq!(h.tracks[1].title, "four");
    }

    #[test]
    fn a_stream_runs_a_local_clock_and_position_noise_is_not_published() {
        let mut m = Model::new(50);
        let bus = "org.mpris.MediaPlayer2.ff".to_string();
        m.apply(
            Event::Added {
                bus: bus.clone(),
                identity: "Firefox".into(),
            },
            Ts(0),
        );
        m.apply(
            Event::Status {
                bus: bus.clone(),
                status: PlayStatus::Playing,
            },
            Ts(0),
        );
        m.apply(
            Event::Meta {
                bus: bus.clone(),
                meta: meta("live", None),
            },
            Ts(10_000_000_000),
        );
        // Firefox reports Position 0 for a stream: the elapsed clock counts
        // from the track's first sighting.
        let now = m.now().unwrap();
        assert_eq!(now.len_us, None);
        assert_eq!(now.read_at, Ts(10_000_000_000));
        assert_eq!(now.pos_at(Ts(40_000_000_000)), 30_000_000, "30 s in");
        assert_eq!(now.fraction(Ts(40_000_000_000)), None);
        // A position that matches the interpolation is not republished.
        let out = m.apply(
            Event::Position {
                bus: bus.clone(),
                pos_us: 0,
            },
            Ts(41_000_000_000),
        );
        assert!(!names(&out).contains(&"media.now"), "{:?}", names(&out));
        // A real seek is.
        let out = m.apply(
            Event::Position {
                bus,
                pos_us: 90_000_000,
            },
            Ts(42_000_000_000),
        );
        assert!(names(&out).contains(&"media.now"));
    }

    #[test]
    fn art_is_wanted_once_per_track_and_odd_values_are_clamped() {
        let mut m = Model::new(50);
        let bus = "org.mpris.MediaPlayer2.p".to_string();
        m.apply(
            Event::Added {
                bus: bus.clone(),
                identity: "P".into(),
            },
            Ts(0),
        );
        assert_eq!(m.art_wanted(), None, "no track yet");
        let mut with_art = meta("one", Some(1));
        with_art.art_url = "file:///tmp/a.png".into();
        m.apply(
            Event::Meta {
                bus: bus.clone(),
                meta: with_art.clone(),
            },
            Ts(1),
        );
        assert_eq!(
            m.art_wanted(),
            Some((with_art.track, "file:///tmp/a.png".to_string()))
        );
        m.apply(
            Event::Rate {
                bus: bus.clone(),
                rate: f64::NAN,
            },
            Ts(2),
        );
        m.apply(Event::Volume { bus, volume: 4.0 }, Ts(2));
        let now = m.now().unwrap();
        assert_eq!(now.rate, 1.0, "a nonsense rate is 1.0");
        assert_eq!(now.volume, 1.0, "volume is a fraction");
    }
}
