//! The MPRIS source (§5 cadence row, §8, brief arc 6 seam 2): one
//! `current_thread` tokio runtime on a private thread, hand-rolled zbus 5
//! proxies, discovery by `ListNames` + `NameOwnerChanged`, one task per
//! player with a `select!` over its property stream, the `Seeked` signal and
//! a 1 Hz `Position` poll while Playing, transport commands from the tile as
//! `Control::Domain(MediaCmd)`, and album art fetched and decoded on a
//! blocking task. tokio never leaves this crate; the `mpris` crate and
//! libdbus stay banned (D17).
//!
//! Everything that decides *what the store says* lives in `model.rs` and
//! `meta.rs` — pure, and tested without a bus.

pub mod art;
pub mod meta;
pub mod model;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use gridwatch_store::keys::media::{self, Caps, MediaCmd, PlayStatus};
use gridwatch_store::{
    Cadence, Datum, Sample, Source, SourceCtx, SourceInfo, SourceState, SourceStatus, Ts, demo,
};
use zbus::zvariant::{OwnedValue, Value};

use meta::{MetaValue, Metadata};
use model::{Event, Model};

/// `[sources.mpris]` (§9).
pub const OPTION_NAMES: &[&str] = &["player", "art", "art_max_px", "poll_ms", "history"];
pub const MIN_POLL: Duration = Duration::from_millis(250);
pub const MAX_POLL: Duration = Duration::from_secs(5);
/// How often the source re-tries a bus it could not reach.
pub const RECONNECT: Duration = Duration::from_secs(10);
/// Every D-Bus call is bounded: a player that stops answering must not
/// hold the source thread (and with it `q`, which joins it) — review.
pub const METHOD_TIMEOUT: Duration = Duration::from_secs(2);
/// How many times a missing cover is re-fetched (a player often writes the
/// file after it publishes the metadata).
pub const ART_TRIES: u32 = 3;
const MPRIS_PREFIX: &str = "org.mpris.MediaPlayer2.";
/// The namespace the bus filters `NameOwnerChanged` on.
const MPRIS_NS: &str = "org.mpris.MediaPlayer2";
const PLAYER_PATH: &str = "/org/mpris/MediaPlayer2";

#[derive(Clone, Debug, PartialEq)]
pub struct Options {
    /// `auto`, or a bus name (with or without the MPRIS prefix).
    pub player: String,
    pub art: bool,
    pub art_max_px: u32,
    pub poll: Duration,
    pub history: usize,
}

impl Default for Options {
    fn default() -> Options {
        Options {
            player: String::new(),
            art: true,
            art_max_px: media::ART_MAX_PX,
            poll: Duration::from_secs(1),
            history: 50,
        }
    }
}

impl Options {
    pub fn from_table(t: &toml::Table) -> Options {
        let mut o = Options::default();
        if let Some(p) = t.get("player").and_then(|v| v.as_str())
            && p != "auto"
        {
            o.player = full_bus(p);
        }
        if let Some(b) = t.get("art").and_then(|v| v.as_bool()) {
            o.art = b;
        }
        if let Some(n) = t.get("art_max_px").and_then(|v| v.as_integer()) {
            o.art_max_px = (n.max(0) as u32).clamp(16, 512);
        }
        if let Some(ms) = t.get("poll_ms").and_then(|v| v.as_integer()) {
            let want = Duration::from_millis(ms.max(0) as u64);
            o.poll = want.clamp(MIN_POLL, MAX_POLL);
            if o.poll != want {
                tracing::warn!(
                    "[sources.mpris] poll_ms = {ms} clamped to {} (250–5000)",
                    o.poll.as_millis()
                );
            }
        }
        if let Some(n) = t.get("history").and_then(|v| v.as_integer()) {
            o.history = (n.max(1) as usize).min(500);
        }
        o
    }
}

/// `firefox` → `org.mpris.MediaPlayer2.firefox`; a full name is left alone.
pub fn full_bus(name: &str) -> String {
    if name.starts_with(MPRIS_PREFIX) {
        name.to_string()
    } else {
        format!("{MPRIS_PREFIX}{name}")
    }
}

/// `gridwatch doctor`'s row (seam 8): the session bus answers and these
/// players are on it. A live probe — it talks to the bus.
pub fn doctor() -> Vec<(gridwatch_store::Capability, bool, String)> {
    use gridwatch_store::Capability;
    let found = std::thread::spawn(|| {
        let rt = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(rt) => rt,
            Err(e) => return Err(format!("no tokio runtime: {e}")),
        };
        rt.block_on(async {
            let conn = zbus::Connection::session()
                .await
                .map_err(|e| format!("no session bus: {e}"))?;
            let dbus = zbus::fdo::DBusProxy::new(&conn)
                .await
                .map_err(|e| e.to_string())?;
            let names = dbus.list_names().await.map_err(|e| e.to_string())?;
            Ok(names
                .into_iter()
                .map(|n| n.as_str().to_string())
                .filter(|n| n.starts_with(MPRIS_PREFIX))
                .collect::<Vec<_>>())
        })
    })
    .join()
    .unwrap_or_else(|_| Err("the probe thread panicked".into()));
    let row = match found {
        Ok(players) if players.is_empty() => (true, "session bus answers; no MPRIS player".into()),
        Ok(players) => (
            true,
            format!("session bus answers; players: {}", players.join(", ")),
        ),
        Err(e) => (false, format!("{e} — is DBUS_SESSION_BUS_ADDRESS set?")),
    };
    vec![(Capability::DbusSession, row.0, row.1)]
}

/// zbus values → the decoder's little enum, so `meta.rs` stays pure.
pub fn to_meta_value(v: &Value<'_>) -> MetaValue {
    match v {
        Value::Str(s) => MetaValue::Str(s.to_string()),
        Value::ObjectPath(p) => MetaValue::Str(p.to_string()),
        Value::Signature(s) => MetaValue::Str(s.to_string()),
        Value::I16(i) => MetaValue::Int(i64::from(*i)),
        Value::U16(i) => MetaValue::Int(i64::from(*i)),
        Value::I32(i) => MetaValue::Int(i64::from(*i)),
        Value::U32(i) => MetaValue::Int(i64::from(*i)),
        Value::I64(i) => MetaValue::Int(*i),
        Value::U64(i) => MetaValue::Int(*i as i64),
        Value::U8(i) => MetaValue::Int(i64::from(*i)),
        Value::F64(f) => MetaValue::Float(*f),
        Value::Bool(b) => MetaValue::Bool(*b),
        Value::Array(a) => {
            let strs: Vec<String> = a
                .iter()
                .filter_map(|v| match v {
                    Value::Str(s) => Some(s.to_string()),
                    Value::ObjectPath(p) => Some(p.to_string()),
                    _ => None,
                })
                .collect();
            if strs.is_empty() {
                MetaValue::Other
            } else {
                MetaValue::Strs(strs)
            }
        }
        Value::Value(inner) => to_meta_value(inner),
        _ => MetaValue::Other,
    }
}

/// The player interface, hand-rolled (no `mpris` crate, D17).
#[zbus::proxy(
    interface = "org.mpris.MediaPlayer2.Player",
    default_path = "/org/mpris/MediaPlayer2"
)]
trait Player {
    fn play_pause(&self) -> zbus::Result<()>;
    fn play(&self) -> zbus::Result<()>;
    fn pause(&self) -> zbus::Result<()>;
    fn stop(&self) -> zbus::Result<()>;
    fn next(&self) -> zbus::Result<()>;
    fn previous(&self) -> zbus::Result<()>;
    fn seek(&self, offset_us: i64) -> zbus::Result<()>;

    #[zbus(property)]
    fn playback_status(&self) -> zbus::Result<String>;
    #[zbus(property)]
    fn metadata(&self) -> zbus::Result<HashMap<String, OwnedValue>>;
    /// MPRIS says `Position` emits no change signal: it is polled.
    #[zbus(property(emits_changed_signal = "false"))]
    fn position(&self) -> zbus::Result<i64>;
    #[zbus(property)]
    fn rate(&self) -> zbus::Result<f64>;
    #[zbus(property)]
    fn volume(&self) -> zbus::Result<f64>;
    #[zbus(property)]
    fn set_volume(&self, v: f64) -> zbus::Result<()>;
    #[zbus(property)]
    fn can_control(&self) -> zbus::Result<bool>;
    #[zbus(property)]
    fn can_play(&self) -> zbus::Result<bool>;
    #[zbus(property)]
    fn can_pause(&self) -> zbus::Result<bool>;
    #[zbus(property)]
    fn can_go_next(&self) -> zbus::Result<bool>;
    #[zbus(property)]
    fn can_go_previous(&self) -> zbus::Result<bool>;
    #[zbus(property)]
    fn can_seek(&self) -> zbus::Result<bool>;

    #[zbus(signal)]
    fn seeked(&self, position_us: i64) -> zbus::Result<()>;
}

/// The root interface: the identity a tile shows, and `Raise`.
#[zbus::proxy(
    interface = "org.mpris.MediaPlayer2",
    default_path = "/org/mpris/MediaPlayer2"
)]
trait Root {
    fn raise(&self) -> zbus::Result<()>;
    #[zbus(property)]
    fn identity(&self) -> zbus::Result<String>;
    #[zbus(property)]
    fn can_raise(&self) -> zbus::Result<bool>;
}

pub struct MprisSource {
    options: Options,
}

impl MprisSource {
    pub fn new(options: &toml::Table) -> MprisSource {
        MprisSource {
            options: Options::from_table(options),
        }
    }
}

fn status(cx: &SourceCtx, state: SourceState, reason: &str, hint: Option<&str>) {
    cx.status(SourceStatus {
        state,
        reason: Some(Arc::from(reason)),
        hint: hint.map(Arc::from),
        since: cx.clock.now(),
        last_sample: None,
        dropped: 0,
        restarts: cx.restarts,
    });
}

/// The two proxies for one player, built once and kept: zbus's lazy
/// property cache would otherwise `GetAll` and `AddMatch` on every poll
/// (a dozen round-trips per player per second — P5's budget).
struct PlayerProxies<'a> {
    player: PlayerProxy<'a>,
    root: RootProxy<'a>,
    identity: String,
}

async fn build_proxies<'a>(conn: &zbus::Connection, bus: &str) -> zbus::Result<PlayerProxies<'a>> {
    let player = PlayerProxy::builder(conn)
        .destination(bus.to_string())?
        .path(PLAYER_PATH)?
        .cache_properties(zbus::proxy::CacheProperties::No)
        .build()
        .await?;
    let root = RootProxy::builder(conn)
        .destination(bus.to_string())?
        .path(PLAYER_PATH)?
        .cache_properties(zbus::proxy::CacheProperties::No)
        .build()
        .await?;
    let identity = root.identity().await.unwrap_or_default();
    Ok(PlayerProxies {
        player,
        root,
        identity,
    })
}

/// One poll of a player. The **first** read decides whether the player is
/// still there: an error means gone (the name vanished, or it stopped
/// answering), and the caller removes it rather than showing a frozen row
/// with a bus name for an identity (review).
async fn read_player(
    p: &PlayerProxies<'_>,
    bus: &str,
    want_position: bool,
) -> zbus::Result<Vec<Event>> {
    let status = PlayStatus::parse(&p.player.playback_status().await?);
    let mut out = Vec::with_capacity(7);
    if !p.identity.is_empty() {
        out.push(Event::Added {
            bus: bus.to_string(),
            identity: p.identity.clone(),
        });
    }
    out.push(Event::Status {
        bus: bus.to_string(),
        status,
    });
    if let Ok(m) = p.player.metadata().await {
        let mut map = Metadata::new();
        for (k, v) in &m {
            map.insert(k.clone(), to_meta_value(v));
        }
        out.push(Event::Meta {
            bus: bus.to_string(),
            meta: meta::decode(&map),
        });
    }
    // The position is read for the player the tile is showing while it is
    // playing — and on the tick its status changed, so resuming does not
    // show a clock that ran while it was paused (review).
    if want_position && let Ok(pos) = p.player.position().await {
        out.push(Event::Position {
            bus: bus.to_string(),
            pos_us: pos,
        });
    }
    if let Ok(r) = p.player.rate().await {
        out.push(Event::Rate {
            bus: bus.to_string(),
            rate: r,
        });
    }
    if let Ok(v) = p.player.volume().await {
        out.push(Event::Volume {
            bus: bus.to_string(),
            volume: v,
        });
    }
    out.push(Event::Caps {
        bus: bus.to_string(),
        caps: Caps {
            play_pause: p.player.can_play().await.unwrap_or(false)
                || p.player.can_pause().await.unwrap_or(false),
            next: p.player.can_go_next().await.unwrap_or(false),
            prev: p.player.can_go_previous().await.unwrap_or(false),
            seek: p.player.can_seek().await.unwrap_or(false),
            control: p.player.can_control().await.unwrap_or(false),
            raise: p.root.can_raise().await.unwrap_or(false),
        },
    });
    Ok(out)
}

async fn run_command(p: &PlayerProxies<'_>, cmd: &MediaCmd) -> zbus::Result<()> {
    match cmd {
        MediaCmd::PlayPause => p.player.play_pause().await,
        MediaCmd::Play => p.player.play().await,
        MediaCmd::Pause => p.player.pause().await,
        MediaCmd::Stop => p.player.stop().await,
        MediaCmd::Next => p.player.next().await,
        MediaCmd::Prev => p.player.previous().await,
        MediaCmd::SeekBy(us) => p.player.seek(*us).await,
        MediaCmd::SetVolume(v) => p.player.set_volume(v.clamp(0.0, 1.0)).await,
        MediaCmd::Raise => p.root.raise().await,
        MediaCmd::Pick(_) => Ok(()),
    }
}

impl Source for MprisSource {
    fn info(&self) -> SourceInfo {
        SourceInfo {
            cadence: Cadence {
                hidden: None,
                visible: self.options.poll,
                focused: self.options.poll,
                always_on: false,
            },
            ..demo::media_info()
        }
    }

    fn run(self: Box<Self>, cx: SourceCtx) {
        status(&cx, SourceState::Starting, "connecting", None);
        let rt = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(rt) => rt,
            Err(e) => {
                status(
                    &cx,
                    SourceState::Unavailable,
                    &format!("no async runtime: {e}"),
                    None,
                );
                return;
            }
        };
        rt.block_on(run_async(self.options, cx));
    }
}

/// The loop proper: one connection, `ListNames` + `NameOwnerChanged`, the
/// per-player poll on the option's grid, the tile's commands, and the art
/// fetch on a blocking task.
async fn run_async(options: Options, cx: SourceCtx) {
    let mut model = Model::new(options.history);
    // The config's pin is not the `p` key's pin: a player leaving clears
    // the latter, never the former (review).
    let config_pin = options.player.clone();
    if !config_pin.is_empty() {
        model.apply(Event::Pick(config_pin.clone()), cx.clock.now());
    }
    // The art in flight, and what has already been fetched: keyed by the
    // URL as well as the track, and retried a few times when the file is
    // not there yet (Firefox writes its cover after the metadata).
    let mut art_task: Option<
        tokio::task::JoinHandle<Result<gridwatch_store::keys::media::Art, art::ArtError>>,
    > = None;
    let mut art_done: Option<(u64, String)> = None;
    let mut art_tries: u32 = 0;
    let mut last_state = SourceState::Starting;
    let mut last_reason = String::new();
    'connect: loop {
        if cx.stopped() {
            return;
        }
        // Connect (and reconnect): a desktop session may start after us.
        // Every method call is bounded, or one frozen player would hang the
        // source — and `q` waits on this thread (review).
        let conn =
            match zbus::connection::Builder::session().map(|b| b.method_timeout(METHOD_TIMEOUT)) {
                Ok(b) => match b.build().await {
                    Ok(c) => c,
                    Err(e) => {
                        unavailable(&cx, &mut last_state, &mut last_reason, &e.to_string());
                        if !sleep_or_stop(&cx, RECONNECT).await {
                            return;
                        }
                        continue;
                    }
                },
                Err(e) => {
                    unavailable(&cx, &mut last_state, &mut last_reason, &e.to_string());
                    if !sleep_or_stop(&cx, RECONNECT).await {
                        return;
                    }
                    continue;
                }
            };
        let dbus = match zbus::fdo::DBusProxy::new(&conn).await {
            Ok(d) => d,
            Err(e) => {
                tracing::warn!("mpris: {e}");
                if !sleep_or_stop(&cx, RECONNECT).await {
                    return;
                }
                continue;
            }
        };
        // Only MPRIS names wake us: the bus does the filtering
        // (`arg0namespace`), not this thread.
        let rule = zbus::MatchRule::builder()
            .msg_type(zbus::message::Type::Signal)
            .sender("org.freedesktop.DBus")
            .and_then(|b| b.interface("org.freedesktop.DBus"))
            .and_then(|b| b.member("NameOwnerChanged"))
            .and_then(|b| b.arg0ns(MPRIS_NS))
            .map(|b| b.build());
        let mut owners = match rule {
            Ok(rule) => zbus::MessageStream::for_match_rule(rule, &conn, Some(16))
                .await
                .ok(),
            Err(e) => {
                tracing::debug!("mpris: no arg0ns match rule ({e}); falling back");
                None
            }
        };
        let mut fallback = if owners.is_none() {
            dbus.receive_name_owner_changed().await.ok()
        } else {
            None
        };
        let mut proxies: HashMap<String, PlayerProxies<'_>> = HashMap::new();
        if let Ok(names) = dbus.list_names().await {
            for n in names {
                let name = n.as_str().to_string();
                if name.starts_with(MPRIS_PREFIX) {
                    add_player(&conn, &mut proxies, &mut model, &cx, &name).await;
                }
            }
        }
        ok_status(&cx, &mut last_state, &mut last_reason, &model);
        let mut ticker = tokio::time::interval(options.poll);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            if cx.stopped() {
                return;
            }
            tokio::select! {
                // The art task, when one is in flight: never awaited
                // inline, or a slow cover would freeze the commands and
                // `q` for its whole timeout (review).
                art = async { art_task.as_mut().unwrap().await }, if art_task.is_some() => {
                    art_task = None;
                    match art {
                        Ok(Ok(a)) => {
                            art_tries = 0;
                            let at = cx.clock.now();
                            emit(&cx, at, vec![Sample {
                                id: media::ART.id.clone(),
                                datum: Datum::Record(Arc::new(a)),
                            }]);
                        }
                        Ok(Err(e)) => {
                            // A cover the player has not written yet is
                            // worth another look; anything else is not.
                            if matches!(e, art::ArtError::Io(_)) && art_tries < ART_TRIES {
                                art_done = None;
                            }
                            tracing::debug!("mpris art: {e}");
                        }
                        Err(e) => tracing::debug!("mpris art task: {e}"),
                    }
                }
                _ = ticker.tick() => {
                    // Controls from the tile.
                    while let Some(c) = cx.try_control() {
                        match c {
                            gridwatch_store::Control::Stop => return,
                            gridwatch_store::Control::Domain(b) => {
                                let any: Box<dyn std::any::Any + Send> = b;
                                match any.downcast::<MediaCmd>() {
                                    Ok(cmd) => {
                                        let at = cx.clock.now();
                                        if let MediaCmd::Pick(bus) = cmd.as_ref() {
                                            let bus = if bus.is_empty() {
                                                config_pin.clone()
                                            } else {
                                                full_bus(bus)
                                            };
                                            let s = model.apply(Event::Pick(bus), at);
                                            emit(&cx, at, s);
                                        } else if let Some(p) = model.current() {
                                            let bus = p.bus.clone();
                                            if let Some(px) = proxies.get(&bus)
                                                && let Err(e) = run_command(px, &cmd).await
                                            {
                                                tracing::warn!("mpris {bus}: {e}");
                                            }
                                        } else {
                                            tracing::debug!("mpris: {cmd:?} with no player");
                                        }
                                    }
                                    Err(_) => tracing::debug!("mpris: unknown Domain control"),
                                }
                            }
                            other => tracing::debug!("mpris: ignoring {other:?}"),
                        }
                    }
                    let level = cx.demand.level();
                    if level == gridwatch_store::Level::Hidden
                        || level == gridwatch_store::Level::Paused
                    {
                        continue;
                    }
                    let buses: Vec<String> = proxies.keys().cloned().collect();
                    let mut gone = Vec::new();
                    for bus in buses {
                        let Some(px) = proxies.get(&bus) else { continue };
                        // The current player's position is read while it
                        // plays, and on the tick its status changes.
                        let want = model.wants_poll(&bus);
                        match read_player(px, &bus, want).await {
                            Ok(events) => {
                                let at = cx.clock.now();
                                let was_playing = model.wants_poll(&bus);
                                for e in events {
                                    let s = model.apply(e, at);
                                    emit(&cx, at, s);
                                }
                                // Resumed on this tick: read the position
                                // now rather than a poll later.
                                if !was_playing
                                    && model.wants_poll(&bus)
                                    && let Ok(pos) = px.player.position().await
                                {
                                    let s = model.apply(
                                        Event::Position { bus: bus.clone(), pos_us: pos },
                                        at,
                                    );
                                    emit(&cx, at, s);
                                }
                                // The posbar moves every tick, not only on
                                // a notable change (seam 1's reason).
                                if let Some(now) = model.now()
                                    && let Some(f) = now.fraction(at)
                                {
                                    emit(&cx, at, vec![Sample {
                                        id: media::POS_PCT.id.clone(),
                                        datum: Datum::Scalar(f * 100.0),
                                    }]);
                                }
                            }
                            Err(e) => {
                                tracing::debug!("mpris {bus}: {e}");
                                gone.push(bus);
                            }
                        }
                    }
                    for bus in gone {
                        proxies.remove(&bus);
                        let at = cx.clock.now();
                        let s = model.apply(Event::Removed { bus }, at);
                        emit(&cx, at, s);
                    }
                    ok_status(&cx, &mut last_state, &mut last_reason, &model);
                    // Art for the current track, once per (track, url).
                    if options.art
                        && art_task.is_none()
                        && let Some((track, url)) = model.art_wanted()
                        && art_done.as_ref() != Some(&(track, url.clone()))
                    {
                        art_done = Some((track, url.clone()));
                        art_tries += 1;
                        let max_px = options.art_max_px;
                        art_task = Some(tokio::task::spawn_blocking(move || {
                            art::load(&url, track, max_px)
                        }));
                    }
                }
                Some(sig) = next_owner_change(&mut owners, &mut fallback) => {
                    let at = cx.clock.now();
                    let (name, new_owner) = sig;
                    if name.starts_with(MPRIS_PREFIX) {
                        if new_owner.is_empty() {
                            proxies.remove(&name);
                            let s = model.apply(Event::Removed { bus: name }, at);
                            emit(&cx, at, s);
                        } else {
                            add_player(&conn, &mut proxies, &mut model, &cx, &name).await;
                        }
                        ok_status(&cx, &mut last_state, &mut last_reason, &model);
                    }
                }
                else => {
                    // Every stream ended: the bus went away. Reconnect
                    // rather than polling an empty model for ever (review).
                    tracing::debug!("mpris: the bus streams ended; reconnecting");
                    if !sleep_or_stop(&cx, RECONNECT).await {
                        return;
                    }
                    continue 'connect;
                }
            }
        }
    }
}

/// Build a player's proxies and fold its first reading into the model.
async fn add_player<'a>(
    conn: &zbus::Connection,
    proxies: &mut HashMap<String, PlayerProxies<'a>>,
    model: &mut Model,
    cx: &SourceCtx,
    bus: &str,
) {
    let Ok(px) = build_proxies(conn, bus).await else {
        return;
    };
    let at = cx.clock.now();
    match read_player(&px, bus, true).await {
        Ok(events) => {
            for e in events {
                let s = model.apply(e, at);
                emit(cx, at, s);
            }
            proxies.insert(bus.to_string(), px);
        }
        Err(e) => tracing::debug!("mpris {bus}: {e}"),
    }
}

fn unavailable(cx: &SourceCtx, last: &mut SourceState, reason: &mut String, e: &str) {
    let text = format!("no session bus: {e}");
    if *last != SourceState::Unavailable || *reason != text {
        *last = SourceState::Unavailable;
        reason.clone_from(&text);
        status(
            cx,
            SourceState::Unavailable,
            &text,
            Some("start a desktop session, or set DBUS_SESSION_BUS_ADDRESS"),
        );
    }
}

/// `Ok` with what the source can see — "no player" is not a failure.
fn ok_status(cx: &SourceCtx, last: &mut SourceState, reason: &mut String, model: &Model) {
    let n = model.players().count();
    let text = if n == 0 {
        "no player".to_string()
    } else {
        format!("{n} player{}", if n == 1 { "" } else { "s" })
    };
    if *last != SourceState::Ok || *reason != text {
        *last = SourceState::Ok;
        reason.clone_from(&text);
        status(cx, SourceState::Ok, &text, None);
    }
}

/// The next `NameOwnerChanged` as `(name, new owner)`, from the filtered
/// match rule when the bus accepted one, else the unfiltered stream.
async fn next_owner_change(
    filtered: &mut Option<zbus::MessageStream>,
    fallback: &mut Option<zbus::fdo::NameOwnerChangedStream>,
) -> Option<(String, String)> {
    use futures_lite::StreamExt;
    if let Some(s) = filtered.as_mut() {
        let msg = s.next().await?.ok()?;
        let body = msg.body();
        let (name, _old, new): (String, String, String) = body.deserialize().ok()?;
        return Some((name, new));
    }
    let s = fallback.as_mut()?;
    let sig = s.next().await?;
    let args = sig.args().ok()?;
    Some((
        args.name().to_string(),
        args.new_owner()
            .as_ref()
            .map(|o| o.to_string())
            .unwrap_or_default(),
    ))
}

fn emit(cx: &SourceCtx, at: Ts, samples: Vec<Sample>) {
    if !samples.is_empty() {
        cx.emit(at, samples);
    }
}

/// Sleep, but wake on stop.
async fn sleep_or_stop(cx: &SourceCtx, d: Duration) -> bool {
    let step = Duration::from_millis(200);
    let mut left = d;
    while left > Duration::ZERO {
        if cx.stopped() {
            return false;
        }
        let this = step.min(left);
        tokio::time::sleep(this).await;
        left -= this;
    }
    !cx.stopped()
}

pub fn start(options: &toml::Table) -> Box<dyn Source> {
    Box::new(MprisSource::new(options))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn options_parse_and_clamp() {
        let t: toml::Table = toml::from_str(
            r#"player = "firefox"
art = false
art_max_px = 4000
poll_ms = 10
history = 9000"#,
        )
        .unwrap();
        let o = Options::from_table(&t);
        assert_eq!(o.player, "org.mpris.MediaPlayer2.firefox");
        assert!(!o.art);
        assert_eq!(o.art_max_px, 512);
        assert_eq!(o.poll, MIN_POLL);
        assert_eq!(o.history, 500);
        let t: toml::Table = toml::from_str(r#"player = "auto""#).unwrap();
        assert_eq!(Options::from_table(&t).player, "");
        assert_eq!(Options::from_table(&toml::Table::new()), Options::default());
        assert_eq!(
            full_bus("org.mpris.MediaPlayer2.vlc"),
            "org.mpris.MediaPlayer2.vlc"
        );
        assert_eq!(OPTION_NAMES.len(), 5);
    }

    #[test]
    fn zbus_values_become_the_decoder_s_shapes() {
        assert_eq!(
            to_meta_value(&Value::from("hi")),
            MetaValue::Str("hi".into())
        );
        assert_eq!(to_meta_value(&Value::from(7i64)), MetaValue::Int(7));
        assert_eq!(to_meta_value(&Value::from(7u32)), MetaValue::Int(7));
        assert_eq!(to_meta_value(&Value::from(1.5f64)), MetaValue::Float(1.5));
        assert_eq!(to_meta_value(&Value::from(true)), MetaValue::Bool(true));
        let path = zbus::zvariant::ObjectPath::try_from("/org/mpris/MediaPlayer2").unwrap();
        assert_eq!(
            to_meta_value(&Value::ObjectPath(path)),
            MetaValue::Str("/org/mpris/MediaPlayer2".into()),
            "an object path decodes like a string"
        );
        let arr = zbus::zvariant::Array::from(vec!["a", "b"]);
        assert_eq!(
            to_meta_value(&Value::Array(arr)),
            MetaValue::Strs(vec!["a".into(), "b".into()])
        );
        let nums = zbus::zvariant::Array::from(vec![1i32, 2]);
        assert_eq!(to_meta_value(&Value::Array(nums)), MetaValue::Other);
    }

    /// Lists the session bus's MPRIS players and prints what they say — a
    /// read-only probe (MACHINE.md); an agent never starts or controls a
    /// player, so this stays ignored in CI.
    #[test]
    #[ignore]
    fn live_mpris_lists_players() {
        for (cap, ok, what) in doctor() {
            println!("{cap:?} {ok} {what}");
        }
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            let Ok(conn) = zbus::Connection::session().await else {
                println!("no session bus");
                return;
            };
            let dbus = zbus::fdo::DBusProxy::new(&conn).await.unwrap();
            for n in dbus.list_names().await.unwrap() {
                let name = n.as_str().to_string();
                if !name.starts_with(MPRIS_PREFIX) {
                    continue;
                }
                match build_proxies(&conn, &name).await {
                    Ok(px) => match read_player(&px, &name, true).await {
                        Ok(events) => {
                            println!("== {name}");
                            for e in events {
                                println!("   {e:?}");
                            }
                        }
                        Err(e) => println!("== {name}: {e}"),
                    },
                    Err(e) => println!("== {name}: {e}"),
                }
            }
        });
    }
}
