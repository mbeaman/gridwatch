<!-- Research digest. Generated 2026-08-30 by the opsTui design workflow (research agents ran read-only against this machine and docs.rs). Version numbers and API names were verified on that date; re-check before pinning.  -->

# Winamp-inspired MPRIS2 "now playing" component for opsTui: MPRIS2/zbus 5 client design, Firefox quirks, Winamp classic-skin anatomy mapped to grid size classes, album-art pipeline on Ptyxis/VTE 0.84

# Winamp-style MPRIS2 "Now Playing" component — research write-up

## 0. What is live on this machine right now (verified 2026-08-30)

- Session bus has exactly one MPRIS name: `org.mpris.MediaPlayer2.firefox.instance_1_107` (owned by pid 9649, unique name `:1.107`). The `instance_1_107` suffix is Firefox's *sanitised unique bus name* (`:1.107` → `1_107`), **not** the pid — do not parse it as one. `mpris-proxy` (bluez, pid 3473) is on the bus as `:1.3` only; it registers `org.mpris.MediaPlayer2.<alias>` names only while a Bluetooth AVRCP device is connected (none now: `busctl --system tree org.bluez` shows just `/org/bluez/hci0`).
- Firefox is the **snap** (`firefox 154.0-1`, rev 8763). Root interface GetAll: `Identity="Mozilla firefox_firefox"`, `DesktopEntry="firefox_firefox"` (→ `/var/lib/snapd/desktop/applications/firefox_firefox.desktop`), `CanRaise=true`, `CanQuit=false`, `HasTrackList=false`, `SupportedUriSchemes/MimeTypes=[]`.
- Player GetAll while a YouTube tab is Playing: `PlaybackStatus="Playing"`, `Rate=1`, `Volume=1`, `Position=0` (polled 3× over 3 s: always 0), `CanGoNext/Previous/Play/Pause/Seek/Control=true`. **`LoopStatus`, `Shuffle`, `MinimumRate`, `MaximumRate` are absent** — `Get` returns an error ("No such property"/"not supported"), although introspection XML lists Min/MaximumRate.
- Metadata had **6 keys only**: `mpris:trackid="/org/mpris/MediaPlayer2/firefox"` (constant!), `xesam:title`, `xesam:album=""`, `xesam:artist=["Insomniac"]`, `mpris:artUrl="file:///home/mattbeam/snap/firefox/common/.mozilla/firefox/firefox-mpris/9649_19.png"`, `xesam:url=https://www.youtube.com/watch?...`. **No `mpris:length`.** The art file is a real 336×188 8-bit RGBA PNG (16:9 YouTube thumbnail), mode 0600, owned by the user, readable from this session.
- A 6 s `busctl --user monitor` on `:1.107` signals captured nothing (no `Seeked`, no `PropertiesChanged`) — consistent with "no position state" (weak evidence, short window).
- PipeWire (`pw-dump`): Firefox's stream node 84 `Stream/Output/Audio` is `48000 Hz, 2 ch, F32LE`; default sink node 61 runs at 48000/2/S32LE. This is the only source for "kHz / stereo" fields — MPRIS carries no bitrate/sample-rate keys.

## 1. MPRIS2 essentials (spec: specifications.freedesktop.org/mpris/latest)

- **Bus-name policy:** `org.mpris.MediaPlayer2.<name>`, optional `.instance<id>` for multiple instances; `<id>` uses `[A-Za-z0-9_-]`, must not start with a digit. Object path is always `/org/mpris/MediaPlayer2`. Discovery = `org.freedesktop.DBus.ListNames` filtered by the `org.mpris.MediaPlayer2.` prefix, then follow `NameOwnerChanged` (use the `arg0namespace='org.mpris.MediaPlayer2'` match rule — the D-Bus daemon does the prefix filtering).
- **Root `org.mpris.MediaPlayer2`:** `Raise()`, `Quit()`; props `CanQuit`, `CanRaise`, `CanSetFullscreen`, `Fullscreen` (rw), `HasTrackList`, `Identity`, `DesktopEntry`, `SupportedUriSchemes`, `SupportedMimeTypes`; all emit PropertiesChanged.
- **`org.mpris.MediaPlayer2.Player` methods:** `Next`, `Previous`, `Pause`, `PlayPause`, `Stop`, `Play`, `Seek(x offset_µs)` (negative = backwards; past end acts like Next), `SetPosition(o trackid, x pos_µs)` (ignored unless trackid == current track; 0..length), `OpenUri(s)`.
- **Properties:** `PlaybackStatus` s (Playing|Paused|Stopped), `LoopStatus` s rw (None|Track|Playlist), `Rate` d rw (never 0), `Shuffle` b rw, `Metadata` a{sv}, `Volume` d rw (0..1), `Position` x µs — **PropertiesChanged is NOT emitted for Position; it must be polled** —, `MinimumRate`/`MaximumRate` d, `CanGoNext/CanGoPrevious/CanPlay/CanPause/CanSeek` b (emit), `CanControl` b (does not emit; "not expected to change"). Signal `Seeked(x)` fires when position changes discontinuously (many players — Spotify historically, Firefox in the no-position-state case — never send it).
- **Metadata keys (full standard list, matches mpris-server 0.10's `Metadata` accessors):** `mpris:trackid` (o), `mpris:length` (x, µs — some players send u64/i32; decode leniently), `mpris:artUrl` (s, `file://`, `https://` or `data:`), `xesam:album` s, `xesam:albumArtist` as, `xesam:artist` as, `xesam:asText` s, `xesam:audioBPM` i, `xesam:autoRating` d, `xesam:comment` as, `xesam:composer` as, `xesam:contentCreated` s(ISO 8601), `xesam:discNumber` i, `xesam:firstUsed` s, `xesam:genre` as, `xesam:lastUsed` s, `xesam:lyricist` as, `xesam:title` s, `xesam:trackNumber` i, `xesam:url` s, `xesam:useCount` i, `xesam:userRating` d (0..1). There is **no bitrate / sample-rate / channel key** and no EQ anywhere in MPRIS.

## 2. Firefox's MPRIS implementation (widget/gtk/MPRISServiceHandler.cpp, main branch ≈ Fx 155/156; behaviour matches Fx 154 observed)

- Bus name = `org.mpris.MediaPlayer2.firefox` + `.instance` + sanitised unique name; owned with `G_BUS_NAME_OWNER_FLAGS_NONE`.
- `Position` = `mPositionState ? CurrentPlaybackPosition()*1e6 : 0`. `mpris:length` is emitted **only when `mPositionState.isSome()`**. `Rate` = `mPositionState->mPlaybackRate` or 1.0; `Set Rate` is not honoured. `Volume` set IS honoured (emits Setvolume/Unmute media keys). `Seeked` is emitted from `SetPositionState()` when the position state changes. `Raise` = focus tab.
- **Where the position state comes from:** (a) the page's `navigator.mediaSession.setPositionState()` or (b) Firefox's *guessed* state: `HTMLMediaElement::MediaControlKeyListener::NotifyMediaPositionState()` builds `PositionState(duration, paused?0:playbackRate, currentTime, now)` on play/pause transitions (and on a couple of other hooks) and `MediaPlaybackStatus::GuessedPositionState()` returns it **only if exactly one controlled media element is registered for that browsing context** (`if (mGuessedPositionStateMap.Count() != 1) return Nothing()`). YouTube pages typically hold several `<video>` elements (preview/ad/inline players), which is the most plausible reason this machine sees `Position=0` and no `mpris:length` on YouTube. Expect real position/length on single-element pages (Bandcamp, SoundCloud, plain `<audio>`), and real `Seeked` there too.
- `SetPosition`→`Seekto` (absolute seconds), `Seek(±off)`→`Seekforward/Seekbackward` with |offset| seconds; both work even when Position reads 0 (CanSeek is derived from the supported media keys bitmask, hence `true`).
- `mpris:trackid` is the constant `/org/mpris/MediaPlayer2/firefox` → track changes must be detected by diffing title/artist/url/artUrl, not trackid.
- Art: Firefox downloads the page's MediaSession artwork, converts to PNG and writes `<XRE_USER_APP_DATA_DIR>/firefox-mpris/<pid>_<counter>.png` (snap: `~/snap/firefox/common/.mozilla/firefox/firefox-mpris/`; Flatpak: `$XDG_DATA_HOME/firefox-mpris`). The counter bumps on every image (to defeat GNOME's icon cache), and `RemoveAllLocalImages()` deletes the whole folder when metadata clears — read the file immediately on the Metadata change and tolerate ENOENT.
- Identity is "Vendor DesktopEntry" (`Mozilla firefox_firefox` for the snap) — map DesktopEntry→friendly name (`firefox_firefox`→"Firefox") in the widget.

## 3. mpris-proxy (bluez `tools/mpris-proxy.c`)
Registers `org.mpris.MediaPlayer2.` + `g_strcanon(alias)` (prefixed `bt_` if the alias starts with a digit) for each connected AVRCP source (phone). Exposes `Position` (ms→µs), `Metadata` (title/artist/album/length/track number), `LoopStatus`, `Shuffle`, `CanSeek/CanPlay/CanPause=true`, emits `Seeked`. In the other direction it watches `NameOwnerChanged` and re-exports session-bus players to bluez (`--export`). For our client it is just another MPRIS player that appears/disappears with the phone — the generic discovery loop covers it.

## 4. zbus 5.19 client shape (pure Rust, no libdbus; verified against the cached crate sources)

```toml
zbus = { version = "5.19", default-features = false, features = ["tokio"] }  # README: disable async-io when on tokio
```
Blocking alternative: `zbus::blocking::Connection::session()` + `PlayerProxyBlocking` (generated by the same macro when `blocking-api` is on) if opsTui stays on std threads like astral-watch.

```rust
use std::collections::HashMap;
use zbus::{proxy, zvariant::{ObjectPath, OwnedValue}};

#[proxy(interface = "org.mpris.MediaPlayer2.Player",
        default_path = "/org/mpris/MediaPlayer2", assume_defaults = false)]
pub trait Player {
    fn next(&self) -> zbus::Result<()>;
    fn previous(&self) -> zbus::Result<()>;
    fn play(&self) -> zbus::Result<()>;
    fn pause(&self) -> zbus::Result<()>;
    fn play_pause(&self) -> zbus::Result<()>;
    fn stop(&self) -> zbus::Result<()>;
    fn seek(&self, offset_us: i64) -> zbus::Result<()>;
    fn set_position(&self, track_id: &ObjectPath<'_>, position_us: i64) -> zbus::Result<()>;
    fn open_uri(&self, uri: &str) -> zbus::Result<()>;
    #[zbus(signal)] fn seeked(&self, position_us: i64) -> zbus::Result<()>;   // -> receive_seeked()
    #[zbus(property)] fn playback_status(&self) -> zbus::Result<String>;      // -> receive_playback_status_changed()
    #[zbus(property)] fn loop_status(&self) -> zbus::Result<String>;
    #[zbus(property)] fn set_loop_status(&self, v: &str) -> zbus::Result<()>;
    #[zbus(property)] fn rate(&self) -> zbus::Result<f64>;
    #[zbus(property)] fn shuffle(&self) -> zbus::Result<bool>;
    #[zbus(property)] fn set_shuffle(&self, v: bool) -> zbus::Result<()>;
    #[zbus(property)] fn metadata(&self) -> zbus::Result<HashMap<String, OwnedValue>>;
    #[zbus(property)] fn volume(&self) -> zbus::Result<f64>;
    #[zbus(property)] fn set_volume(&self, v: f64) -> zbus::Result<()>;
    #[zbus(property(emits_changed_signal = "false"))]   // REQUIRED: disables the proxy cache so every call is a real Get
    fn position(&self) -> zbus::Result<i64>;
    #[zbus(property)] fn minimum_rate(&self) -> zbus::Result<f64>;
    #[zbus(property)] fn maximum_rate(&self) -> zbus::Result<f64>;
    #[zbus(property)] fn can_go_next(&self) -> zbus::Result<bool>;
    #[zbus(property)] fn can_go_previous(&self) -> zbus::Result<bool>;
    #[zbus(property)] fn can_play(&self) -> zbus::Result<bool>;
    #[zbus(property)] fn can_pause(&self) -> zbus::Result<bool>;
    #[zbus(property)] fn can_seek(&self) -> zbus::Result<bool>;
    #[zbus(property(emits_changed_signal = "const"))] fn can_control(&self) -> zbus::Result<bool>;
}

#[proxy(interface = "org.mpris.MediaPlayer2",
        default_path = "/org/mpris/MediaPlayer2", assume_defaults = false)]
pub trait MediaPlayer2 {
    fn raise(&self) -> zbus::Result<()>;
    #[zbus(property)] fn identity(&self) -> zbus::Result<String>;
    #[zbus(property)] fn desktop_entry(&self) -> zbus::Result<String>;
    #[zbus(property)] fn can_raise(&self) -> zbus::Result<bool>;
    #[zbus(property)] fn has_track_list(&self) -> zbus::Result<bool>;
}
```
Key semantics (from `zbus/src/proxy/mod.rs`, `zbus_macros/src/proxy.rs`): the macro generates `PlayerProxy` (+ `PlayerProxyBlocking`); property getters go through a cache (`CacheProperties::Lazily` default — first read does `GetAll`, then `PropertiesChanged` keeps it fresh); `emits_changed_signal = "false"` puts the property on the `uncached_properties` list. `receive_<prop>_changed()` returns `PropertyStream<T>`; each item is `PropertyChanged<T>` with `.name()` and `.get().await -> Result<T>`; the stream yields the current value first; zbus does **not** queue updates (slow consumers see only the latest — fine for UI). `receive_seeked()` → `SignalStream` of `Seeked` messages with `.args()?.position_us`. `proxy.inner().receive_owner_changed()` yields `Option<OwnedUniqueName>` and `None` when the player quits — that is the "player disappeared" hook per proxy.

Discovery:
```rust
let conn = zbus::Connection::session().await?;
let dbus = zbus::fdo::DBusProxy::new(&conn).await?;                      // default_service/path baked in
let names = dbus.list_names().await?;                                    // Vec<OwnedBusName>
let players = names.iter().filter(|n| n.as_str().starts_with("org.mpris.MediaPlayer2."));
// hot-plug: arg0namespace match, daemon-side filtered
let rule = zbus::MatchRule::builder().msg_type(zbus::message::Type::Signal)
    .sender("org.freedesktop.DBus")?.interface("org.freedesktop.DBus")?
    .member("NameOwnerChanged")?.arg0ns("org.mpris.MediaPlayer2")?.build();
let mut stream = zbus::MessageStream::for_match_rule(rule, &conn, None).await?;
while let Some(msg) = stream.try_next().await? {
    let (name, old, new): (String, String, String) = msg.body().deserialize()?;
    if new.is_empty() { drop_player(&name) } else { add_player(&name) }
}
// simpler alt: dbus.receive_name_owner_changed().await? and filter by prefix in Rust
let player = PlayerProxy::builder(&conn).destination(name.as_str())?.build().await?;
```
Metadata decoding: `HashMap<String, OwnedValue>`; use `String::try_from(v.clone())`, `Vec::<String>::try_from(v.clone())`, `i64::try_from(..).or(u64..)` for length, `ObjectPath::try_from(..)` for trackid. `mpris-server 0.10`'s `Metadata` derives only `Serialize`/`Type` (no `Deserialize`) so it cannot be the proxy return type; write a ~60-line `TrackMeta` of our own. Position model: keep `(pos_us, read_at: Instant, rate)`; `now_us = pos_us + elapsed*rate` while Playing; resync on the 1–2 Hz poll (only while Playing; Firefox answers 0 instantly), on `Seeked`, and on any `PlaybackStatus`/`Metadata` change. Treat `Get` errors on `LoopStatus`/`Shuffle`/`Min/MaxRate` as "unsupported" (Firefox) and grey out those toggles.

## 5. Winamp 2.x classic-skin anatomy (Webamp `skinSprites.ts`, `main-window.css`, `equalizer-window.css`, `VisPainter.ts`, `Marquee.tsx`)
All three windows are 275×116 px (shade mode 275×14). Main window placement (left,top,w,h): title bar 0,0,275,14; **time** 39,26,59,13 with 9×13 digits at x=9/21/39/51 (mm:ss; `-` sign 5×1 px at x=-1,y=6 for "remaining" mode; a *blinking colon* is implied by the paused state — Winamp blinks the whole time display when paused); **marquee** 111,24,154,6 using the 5×6 px `text.bmp` font → 31 visible chars, text = `"N. Artist - Title (m:ss)"`, scrolls 1 char every 220 ms, wrapped with the separator `"  ***  "` (Webamp's constant; Winamp shows ` *** `), no scroll when it fits (padded); **kbps** 111,43,15,6 (3 chars) and **kHz** 156,43,10,6 (2 chars); **mono/stereo** 212,41 (mono 27×12, stereo 29×12, lit one is bright); **visualizer** 24,43,76,16 — spectrum draws 75 1-px bars (or 19 "thick" bands of 3 px + 1 gap), hybrid linear/log bin mapping `scale=0.91`, bar fall-off `falloff/16` per frame (slower 3 … faster 32) and peak markers decaying by 1.05–1.6× ; **viscolor.txt** = 24 RGB lines: 0–1 background/grid dots, 2–17 spectrum gradient top→bottom (16 rows = 16 px), 18–22 oscilloscope, 23 peak marker; **play/pause indicator** 26,28,9,9 and "work" indicator 24,28,3,9; **clutter bar** (O A I D V) 10,22,8,43; **posbar** 16,72,248,10 with a 29×10 thumb (28 positions); **volume** 107,57,68,13 (28 background frames = colour sweep green→yellow→red) and **balance** 177,57,38,13 (centre-detent); **EQ/PL toggle buttons** 219,58 / 242,58 (23×12); **transport** row at y=88: prev 16, play 39, pause 62, stop 85 (23×18 each), next 108 (22×18), eject 136,89 (22×16); **shuffle** 164,89,47,15 and **repeat** 210,89,28,15.
**EQ window:** ON 14,18 (26 w), AUTO 40,18 (32 w), PRESETS 217,18 (44 w); graph 86,17,113×19 (curve of the 10 gains); +12/0/-12 dB labels at x=45; preamp slider at 21,38; bands at top 38 with 18 px pitch starting x=78: 60, 170, 310, 600, 1k, 3k, 6k, 12k, 14k, 16k Hz; sliders are 14×63 with 28 thumb positions, range ±12 dB.
**Playlist window:** 275×116 minimum, resizable in 25×29 px steps; 20 px title bar; rows 13 px, 9 px font; columns "N. Artist - Title" left, "m:ss" right-aligned; colours from `PLEDIT.txt [Text]`: `Normal`, `Current` (playing row), `NormalBG`, `SelectedBG`, `Font`; 38 px bottom bar with add/rem/sel/misc/list buttons, a mini transport, and the running-time display `"selected/total"` (e.g. `0:00/3:45`).

## 6. Mapping to opsTui size classes (grid unit TBD; assume ≈ 20 cols × 5 rows per unit)
- **1×1:** status glyph (▶/‖/■ from DejaVu Sans Mono — U+25B6/U+25A0 verified present; U+23EE-23F9 media glyphs exist only in Noto Sans Symbols2/Color Emoji, so avoid them or rely on fontconfig fallback), 1-row marquee, 1-row posbar (`━━━●────`) with `m:ss`/`-m:ss`. Mouse: click posbar → `SetPosition`.
- **2×1:** Winamp *shade mode*: marquee + time + a 4–8 bar mini spectrum from the audio-visualizer component's FFT; transport keys.
- **4×2:** the main window: 7-seg time (hand-drawn 3×3 or 4×5 glyphs with ▀▄█ — Winamp digits are 9×13, not font8x8; `tui-big-text 0.8.9` `PixelSize::Quadrant` (4 cols×4 rows/glyph) or `Sextant` (4×3, needs U+1FB00 blocks — only Noto Sans Symbols2 has them here) is the quick fallback), marquee, kbps/kHz/mono-stereo, 19-band spectrum, posbar, volume (`set_volume`) + balance (decorative unless we wire it to PipeWire), transport row, shuffle/repeat (disabled when the player lacks the props), EQ/PL badges.
- **6×3:** main + EQ side-by-side, or main + album art (art uses the 16-row-ish square on the left); EQ sliders are decorative or, better, weight the visualizer's FFT bins near the 10 centre frequencies (MPRIS has no EQ; real EQ would need a PipeWire filter-chain — out of scope).
- **full:** main + EQ + playlist stack. Playlist source: `TrackList` interface is optional and Firefox/Spotify/mpv don't implement it, so build a local "recently played" list from Metadata transitions (`title|artist|album|url` key, dedup, keep length when known), highlight current, show `elapsed/total` in the bottom bar.
- **kbps/kHz:** take from the PipeWire stream node of the same app (`application.name` ≈ Identity, `format.rate`/`channels`) or from whatever rate the visualizer captures at; show PCM-style kbps like Winamp did for WAV (48000×2×32/1000 = 3072, or 1536 for S16) or `---`.

## 7. Album art pipeline

- **Terminal graphics verdict (verified):** Ptyxis 50.1 links `libvte-2.91-gtk4.so.0` (0.84.0-2) and **does not import `vte_terminal_set_enable_sixel`** (`objdump -T`), and VTE's meson `sixel` option defaults to `false` with Ubuntu's `debian/rules` not overriding it — so **Sixel is unavailable in this terminal** and VTE has no Kitty/iTerm2 protocol either. `TIOCGWINSZ` on the user's ptys reports `xpixel=ypixel=0`, so ratatui-image's font-size fallback also yields nothing. Practical result: **Unicode half-blocks (`▀` fg/bg truecolor) are the only rendering path here** (COLORTERM=truecolor is set). Keep protocol auto-detect for other terminals (foot/kitty/wezterm/ghostty) but default this machine to `Picker::halfblocks()`.
- **ratatui-image 11.0.6 gotcha:** default features include `chafa-dyn`, whose `build.rs` `pkg-config`-probes `chafa >= 1.8` and **panics at build time** — `libchafa-dev` is not installed here (`pkg-config --exists chafa` fails). Use `ratatui-image = { version = "11.0.6", default-features = false, features = ["image-defaults", "crossterm"] }` (or `image` with only `png,jpeg,webp` to trim deps). It requires `ratatui ^0.30.1` — matches ratatui 0.30.2.
- API: `Picker::halfblocks()` / `Picker::from_query_stdio()` (writes `ESC[c`, `ESC[16t`, Kitty query, `ESC[5n` to stdout and reads stdin — must run **before** the crossterm event reader, in raw mode); `picker.new_protocol(img, Size{width: cols, height: rows}, Resize::Fit(None)) -> Protocol` rendered with stateless `Image::new(&proto)` (zero per-frame cost — ideal for discrete size classes: pre-encode one `Protocol` per (art, class) on the worker); or `picker.new_resize_protocol(img) -> StatefulProtocol` + `StatefulImage::new().resize(Resize::Fit/Crop/Scale)` with `ThreadProtocol` for off-thread resize. `FontSize` defaults to 10×20 when unknown, giving the ≈1:2 cell aspect: a square cover in an N-col area needs N/2 rows; half-blocks give 2 image px per cell vertically. Firefox's YouTube thumbnails are 16:9 — letterbox (Fit) or centre-crop (Crop) by theme.
- Fetch/decode (worker thread or `spawn_blocking`): `file://` → percent-decode (`url::Url::to_file_path`) and `std::fs::read`; `https://` → `ureq 3.4` (rustls, already used by astral-watch; set a 5 s timeout and a byte cap), `data:` → base64; decode with `image::load_from_memory` (PNG/JPEG/WebP in default formats); downscale once to ≤256 px; cache `Arc<DynamicImage>` in a small LRU keyed by artUrl (Firefox changes the filename per image so the key is naturally unique); drop art on `None`/ENOENT, show a placeholder tile.

## 8. Event flow

Spawn one tokio task per player (`select!` over `receive_playback_status_changed`, `receive_metadata_changed`, `receive_volume_changed`, `receive_can_*_changed`, `receive_seeked`, `inner().receive_owner_changed`, and a 1 Hz `Position` poll gated on Playing) feeding `NowPlayingEvent` into the UI channel; a supervisor task owns discovery and picks the "active" player (Playing > most-recently-changed > alphabetical; user can cycle). Controls from the UI go back as `PlayerCmd` (`PlayPause`, `Next`, `Prev`, `SeekRel(i64)`, `SeekAbs(i64)`, `Volume(f64)`, `Raise`, `Select(name)`).

## Recommendations

- **Hand-roll two small zbus 5 proxies (Player + root) instead of pulling an MPRIS crate** — The whole surface is ~80 lines; `mpris 2.1` needs libdbus, `mpris-server 0.10` is server-side (its Metadata lacks Deserialize), `doobs-mpris 0.2.0` is young and MPL-2.0. Hand-rolled proxies let us set `emits_changed_signal = "false"` on Position (mandatory to bypass the cache) and `"const"` on CanControl exactly as the spec says.
  - alternatives: doobs-mpris 0.2.0 (zbus ^5.14, has an Enumerator) if the user accepts MPL; zbus blocking API + std threads if opsTui does not adopt tokio.
- **Use `zbus = { default-features = false, features = ["tokio"] }` and a per-player tokio task; poll Position at 1 Hz only while Playing and interpolate locally** — Position never emits PropertiesChanged; Seeked is unreliable (Firefox on multi-element pages never sends it). Local clock + periodic resync gives a smooth seek bar with negligible bus traffic.
  - alternatives: 4 Hz polling for pixel-smooth posbar; blocking API with a dedicated thread.
- **Discover players with ListNames + a `NameOwnerChanged` match rule using `arg0ns("org.mpris.MediaPlayer2")`, and detect death per proxy with `inner().receive_owner_changed()`** — Daemon-side filtering, zero polling, and it covers Firefox tabs, Spotify, mpv, and mpris-proxy Bluetooth phones uniformly. zbus's own builder docs use this exact namespace example.
  - alternatives: `DBusProxy::receive_name_owner_changed()` unfiltered and prefix-match in Rust (simpler, marginally more traffic).
- **Default album art to half-blocks via `Picker::halfblocks()` on this machine, keep `from_query_stdio()` behind a config flag (`art.protocol = auto|halfblocks|sixel|kitty|off`)** — Verified: Ptyxis 50.1 never enables VTE sixel and Ubuntu's VTE 0.84 is built without it; VTE has no Kitty protocol. The stdio query is also risky in-process (it writes/reads the tty). Auto-detect still pays off if the user runs foot/kitty/wezterm.
  - alternatives: Own 2×2 quadrant/sextant mosaic encoder later (chafa-like) for higher perceived resolution; installing libchafa-dev and enabling `chafa-dyn`.
- **Depend on `ratatui-image` with `default-features = false, features = ["image-defaults","crossterm"]`; pre-encode a stateless `Protocol` per (art, size class) off-thread and render with `Image`** — The default `chafa-dyn` feature fails the build here (no libchafa .pc). Size classes are discrete so a fixed `Protocol` per class avoids per-frame resize work and the `ThreadProtocol` plumbing.
  - alternatives: `StatefulImage` + `ThreadProtocol` if free-form resizing is added later.
- **Fetch remote art with ureq 3.4 (rustls) on a worker thread with a timeout and byte cap; decode with `image` 0.25; LRU keyed by artUrl** — astral-watch already standardises on ureq/rustls (no OpenSSL headers); Firefox art is a local PNG anyway; Spotify/Chromium give https URLs.
  - alternatives: reqwest 0.13 async (heavier: hyper + tokio TLS).
- **Build the playlist pane from a local history of Metadata transitions, not the MPRIS TrackList interface; make EQ sliders drive the visualizer's FFT band weighting (or be purely decorative)** — Firefox reports HasTrackList=false and most players don't implement TrackList; MPRIS has no EQ at all. History + weighting keep the Winamp look honest without faking data.
  - alternatives: Implement TrackList for players that expose it (VLC) as a later enhancement.
- **Source kHz/mono-stereo (and a PCM-style kbps) from the PipeWire node of the playing app via the visualizer/pw-dump path, and detect track changes by hashing title|artist|album|url rather than trackid** — MPRIS has no audio-format keys; Firefox's trackid is a constant path so it cannot identify tracks.
  - alternatives: Show `---` for kbps; use trackid where the player provides unique ones (Spotify, mpv).

## Crates

| crate | version | purpose | system deps | confidence |
|---|---|---|---|---|
| `zbus` | 5.19.0 | Pure-Rust D-Bus client: session connection, #[proxy] macros for org.mpris.MediaPlayer2(.Player), property streams, NameOwnerChanged match rules | none (no libdbus); use default-features=false + tokio feature (MSRV 1.87) | verified |
| `zvariant` | 5.15.0 | OwnedValue/ObjectPath types for decoding the Metadata a{sv} map (re-exported by zbus as zbus::zvariant) | none | verified |
| `tokio` | 1.53.1 | Runtime for the per-player tasks, select! loops, interval polling, spawn_blocking for art decode | none | verified |
| `ratatui-image` | 11.0.6 | Album-art widget (half-blocks here; Sixel/Kitty/iTerm2 elsewhere). Requires ratatui ^0.30.1 | libchafa-dev ONLY if the default chafa-dyn feature is kept — disable it (default-features=false, features=["image-defaults","crossterm"]); libchafa is NOT installed here | verified |
| `image` | 0.25.10 | Decode PNG (Firefox art), JPEG/WebP (Spotify/Chromium), resize thumbnails; pulled transitively by ratatui-image | none (MSRV 1.88) | verified |
| `ureq` | 3.4.0 | Blocking https fetch of remote mpris:artUrl (rustls, gzip); already used by astral-watch | none with rustls feature | verified |
| `url` | 2.x | Parse file:// art URLs with percent-decoding (Url::to_file_path) and classify http(s)/data schemes | none | likely |
| `tui-big-text` | 0.8.9 | Optional big time display fallback (font8x8 glyphs; PixelSize::Quadrant = 4×4 cells per glyph); a hand-drawn 7-segment glyph table is closer to Winamp | none (MSRV 1.88; Sextant/Octant modes need U+1FB00 glyphs, only in Noto Sans Symbols2 here) | verified |
| `icy_sixel` | 0.6.0 | Pure-Rust Sixel encoder used by ratatui-image (transitive; only active on Sixel-capable terminals) | none | verified |
| `doobs-mpris` | 0.2.0 | Alternative ready-made zbus 5 MPRIS client (PlayerProxy, Enumerator) — MPL-2.0; not recommended over hand-rolled proxies | none | likely |
| `mpris-server` | 0.10.0 | Reference only for metadata key names/types; its Metadata type is Serialize-only, unusable as a client return type | none | verified |
| `mpris` | 2.1.0 | AVOID — links libdbus (libdbus-1-dev not installed) | libdbus-1-dev | verified |

## Risks

- **Firefox (YouTube) reports Position=0, no mpris:length, and never emits Seeked when the page has several media elements — the seek bar and time display have nothing to show** → Treat length=None as 'stream' mode: show elapsed-since-play from a local clock (reset on PlaybackStatus/Metadata change), hide the thumb, label kbps/kHz from PipeWire; SetPosition/Seek still work because CanSeek=true
- **Property cache staleness: with zbus's default Lazily cache a plain `#[zbus(property)] fn position()` would return the first-read value forever** → Annotate Position with `emits_changed_signal = "false"` (verified: macro adds it to uncached_properties) and CanControl with "const"; cover with an integration test against a fake player implemented with mpris-server or zbus's #[interface]
- **Missing optional properties (Firefox has no LoopStatus/Shuffle/Min/MaxRate; Get errors) could bubble up as fatal errors** → Model them as Option<_>; map Get errors (fdo UnknownProperty / generic Failed) to None and disable the corresponding Winamp toggles
- **ratatui-image's default `chafa-dyn` feature panics at build time on this machine (no libchafa.pc) and would break CI/musl release builds** → default-features=false in Cargo.toml; add a CI job that builds without pkg-config extras; document in README
- **Album-art protocol query (`Picker::from_query_stdio`) writes to the tty and reads stdin; if run after crossterm's event reader starts it can swallow responses or hang until timeout, and Sixel is unavailable on Ptyxis/VTE anyway** → Run the query once before the event loop (raw mode on), gate it behind config, default to `Picker::halfblocks()` when TERM/VTE_VERSION indicate VTE; never re-query on resize
- **Firefox deletes its firefox-mpris folder when metadata clears and rotates filenames per image; reading the file late yields ENOENT** → Read the art file synchronously-ish on the Metadata event (worker thread), keep decoded bytes in memory, tolerate missing files with a placeholder
- **Firefox's mpris:trackid is a constant path, so trackid-based change detection and SetPosition's trackid check are meaningless** → Detect track changes by hashing title|artist|album|url; always pass the trackid currently in Metadata to SetPosition
- **Multiple players (Firefox tab + Spotify + Bluetooth phone via mpris-proxy) fight for the single 'now playing' slot; players can vanish mid-command** → Supervisor picks Playing > most recent > alphabetical, user can cycle; per-player owner_changed stream drops the task; commands return Result and are logged, never unwrap
- **Glyph coverage without Nerd Fonts: ⏮⏭⏯⏸ (U+23EE–23F9) and sextant/octant blocks are not in DejaVu Sans Mono (only Noto Sans Symbols2/Color Emoji here), risking tofu or emoji-width cells** → Use ▶ ◀ ■ ▮ ⏏ (U+25B6/25C0/25A0/23CF verified in DejaVu Sans Mono) and ▀▄█▌▐ half-blocks; make glyph sets a theme property with an ASCII fallback (`|< > || [] >|`)
- **Remote art fetch (https) can block or download huge files; some players hand out data: URIs or unreachable hosts** → Worker thread, 5 s timeout, byte cap (~8 MB), content sniffing via image::guess_format, LRU cache, never fetch on the render thread

## Verified facts

- Session bus MPRIS names right now: only org.mpris.MediaPlayer2.firefox.instance_1_107 (pid 9649, unique :1.107); mpris-proxy is :1.3 with no MPRIS name — verified with `busctl --user list` and ListNames
- Firefox root props: Identity="Mozilla firefox_firefox", DesktopEntry="firefox_firefox", CanRaise=true, CanQuit=false, HasTrackList=false, empty SupportedUriSchemes/MimeTypes — `busctl call ... Properties.GetAll s org.mpris.MediaPlayer2`
- Firefox Player props while Playing YouTube: Position=0 on three consecutive 1 s polls, Rate=1, Volume=1, CanSeek=CanControl=true; LoopStatus/Shuffle Get -> 'No such property'; MinimumRate/MaximumRate Get -> 'not supported' — `busctl get-property`
- Firefox Metadata had exactly 6 keys with constant mpris:trackid=/org/mpris/MediaPlayer2/firefox, no mpris:length, artUrl=file:///home/mattbeam/snap/firefox/common/.mozilla/firefox/firefox-mpris/9649_19.png — busctl GetAll
- That art file is a 336x188 8-bit RGBA non-interlaced PNG, 135115 bytes, mode 0600 owned by mattbeam, readable — `file` + PNG header parse
- Firefox is the snap: firefox 154.0-1 rev 8763 (`snap list firefox`); desktop file /var/lib/snapd/desktop/applications/firefox_firefox.desktop exists
- No Seeked/PropertiesChanged signals from :1.107 during a 6 s `busctl --user monitor` window while Playing
- PipeWire: Firefox stream node 84 = 48000 Hz / 2 ch / F32LE; default sink node 61 (Headphones) running at 48000/2/S32LE; Cyberpunk 2077 stream node 90 also active — `pw-dump`
- Firefox source (mozilla-firefox/firefox main, widget/gtk/MPRISServiceHandler.cpp): Position = mPositionState ? CurrentPlaybackPosition()*1e6 : 0; mpris:length only when mPositionState.isSome(); art written as {pid}_{counter}.png under <app data>/firefox-mpris (XDG_DATA_HOME under Flatpak); Seeked emitted from SetPositionState(); Rate not settable; SetVolume honoured; Raise=focus — raw GitHub fetch
- Firefox guessed position state: HTMLMediaElement MediaControlKeyListener::NotifyMediaPositionState builds PositionState(duration, paused?0:rate, currentTime, now) on play/pause; MediaPlaybackStatus::GuessedPositionState returns Nothing unless exactly one element is registered (Count() != 1) — `gh api` raw fetch of dom/media/mediaelement/HTMLMediaElement.cpp and WebFetch of MediaPlaybackStatus.cpp
- Firefox bus name = org.mpris.MediaPlayer2.firefox + ".instance" + sanitised unique bus name (':' and '.' -> '_'), owned with G_BUS_NAME_OWNER_FLAGS_NONE — MPRISServiceHandler.cpp
- MPRIS Player spec: Position does not emit PropertiesChanged; CanControl does not emit; SetPosition ignored on trackid mismatch; Seek past end acts as Next — specifications.freedesktop.org/mpris/latest/Player_Interface.html
- MPRIS bus-name policy and object path /org/mpris/MediaPlayer2 — specifications.freedesktop.org/mpris/latest (Bus_Name_Policy)
- zbus 5.19.0 (cached source): default features = async-io + blocking-api; `tokio` feature exists; README says use default-features=false + tokio; CacheProperties {Yes, No, Lazily(default)}; PropertyStream yields current value first and does not queue; PropertyChanged::get().await; Proxy::receive_owner_changed() -> Option<OwnedUniqueName>; fdo::DBusProxy has list_names(), name_owner_changed signal, receive_name_owner_changed_with_args; MatchRule builder has arg0ns() with the doc example "org.mpris.MediaPlayer2"; MessageStream::for_match_rule exists
- zbus_macros 5.19.0: `#[zbus(property(emits_changed_signal = "false"))]` pushes the property into uncached_properties; "const" suppresses the receive_*_changed generator; assume_defaults defaults to false — cached source
- mpris-server 0.10.0: Metadata derives Serialize+Type only (no Deserialize); PlaybackStatus/LoopStatus derive Type only; Time(i64) derives Serialize+Deserialize — cached source
- ratatui-image 11.0.6: default features = image-defaults, crossterm, chafa-dyn; build.rs panics if pkg-config cannot find chafa >= 1.8.0; depends on ratatui ^0.30.1; query sends ESC _G (kitty), ESC[c (DA1 ';4' => Sixel), ESC[16t, ESC[5n; font_size_fallback uses TIOCGWINSZ and returns None if xpixel/ypixel==0; DEFAULT_PICKER = Halfblocks 10x20; Resize::{Fit,Crop,Scale}; Image (stateless) vs StatefulImage + ThreadProtocol — cached source
- `pkg-config --exists chafa` fails and no libchafa package is installed on this machine
- Ptyxis 50.1 links libvte-2.91-gtk4.so.0 (VTE 0.84.0-2) and its dynamic symbol table contains no sixel symbol (no vte_terminal_set_enable_sixel import); ptyxis binary has no 'sixel'/'kitty' strings; libvte only contains the enable-sixel API names — objdump -T / ldd / strings
- VTE meson_options.txt: option 'sixel' default false; Debian and Ubuntu (resolute) debian/rules pass only -Ddocs/-Dgir/-Dgnutls/-Dgtk4, never -Dsixel — GitLab raw + salsa + launchpad fetches
- Ptyxis NEWS up to 50.1 has no mention of sixel/kitty/image protocols — GitLab raw fetch
- TIOCGWINSZ on the user's ptys (/dev/pts/0, /dev/pts/1) reports xpixel=0 ypixel=0 (rows 75/72, cols 146) — read-only ioctl
- Environment: TERM=xterm-256color, COLORTERM=truecolor, VTE_VERSION=8400, PTYXIS_VERSION=50.1; Ptyxis font 'Monospace 10' (gsettings)
- Font coverage (fc-list :charset=): DejaVu Sans Mono has ▶ ◀ ■ ⏏ and all half/quadrant blocks (U+2580-259F); ⏮⏭⏯⏸⏹ (U+23EE-23F9) only in Noto Sans Symbols2/Noto Color Emoji; sextant/octant blocks (U+1FB00..) only in Noto Sans Symbols2
- Winamp geometry (Webamp skinSprites.ts / main-window.css / equalizer-window.css): 275x116 windows, digits 9x13 in numbers.bmp, text font 5x6, marquee 111,24 154x6 (31 chars, 220 ms step, separator "  ***  "), vis 76x16 at 24,43, posbar 248x10 at 16,72 with 29x10 thumb, volume 68x13 / balance 38x13, transport 23x18 buttons at y=88, EQ bands 60..16k at 18 px pitch from x=78, EQ graph 113x19 at 86,17, viscolor indices 2..17 spectrum / 18..22 scope / 23 peak, playlist rows 13 px
- bluez mpris-proxy names players org.mpris.MediaPlayer2.<g_strcanon(alias)> (bt_ prefix if alias starts with a digit), exposes Position(ms->us), LoopStatus, Shuffle, CanSeek=true, emits Seeked, watches NameOwnerChanged — tools/mpris-proxy.c
- tui-big-text 0.8.9: PixelSize variants Full(1x1 cell/px), HalfHeight, HalfWidth, Quadrant(2x2 px/cell), ThirdHeight, Sextant(2x3), QuarterHeight, Octant(2x4) over font8x8 glyphs — docs.rs
- doobs-mpris 0.2.0 exists (zbus ^5.14, MPL-2.0, PlayerProxy + Enumerator) — docs.rs
- astral-watch Cargo.toml: ureq 3 with rustls/json/gzip, ratatui 0.29 optional, nvml-wrapper 0.12.1 optional, MSRV 1.85 — read locally

## Open questions

- Does Firefox report Position/mpris:length/Seeked on single-media-element pages (Bandcamp, SoundCloud, a raw <audio> URL) as the guessed-state code suggests? Needs a 2-minute live test with `busctl get-property`/`busctl monitor` once the YouTube tab is not the only player.
- Does VTE 0.84 answer `CSI 16 t` (cell size)? If not, ratatui-image uses the 10x20 default; irrelevant for half-blocks but affects Sixel/Kitty terminals the user might switch to.
- What is the opsTui grid unit (cols x rows per 1x1 cell)? The size-class content plan above assumes ~20x5; the 4x2 'main window' needs ≥ 40 cols x 10 rows to hold time + marquee + spectrum + sliders + transport.
- Will opsTui's core loop be tokio (then zbus async with the tokio feature) or std threads like astral-watch (then zbus::blocking + a dedicated thread)? The proxies are identical either way.
- Should the audio-visualizer component expose its FFT bands and capture rate to the Winamp widget (for the mini spectrum and kHz field), or should the widget query PipeWire itself via `pw-dump` JSON?
- Is the user willing to run a Sixel/Kitty-capable terminal (foot, kitty, wezterm, ghostty) for album art, or is half-block art on Ptyxis the accepted baseline?
- Should EQ sliders be persisted per theme and actually weight the visualizer, or stay purely decorative?
- License choice for opsTui matters if doobs-mpris (MPL-2.0) were ever adopted; hand-rolled proxies avoid the question.

## Sources

- http://specifications.freedesktop.org/mpris/latest/Player_Interface.html
- http://specifications.freedesktop.org/mpris/latest/Media_Player.html
- https://specifications.freedesktop.org/mpris/latest/ (Bus_Name_Policy)
- https://docs.rs/zbus/latest/zbus/attr.proxy.html
- https://z-galaxy.github.io/zbus/client.html
- ~/.cargo/registry/src/*/zbus-5.19.0/{README.md,src/proxy/mod.rs,src/proxy/builder.rs,src/fdo/dbus.rs,src/match_rule/builder.rs,src/message_stream.rs}
- ~/.cargo/registry/src/*/zbus_macros-5.19.0/src/{lib.rs,proxy.rs}
- ~/.cargo/registry/src/*/mpris-server-0.10.0/src/{metadata.rs,playback_status.rs,track_id.rs,time.rs}
- https://docs.rs/ratatui-image/latest/ratatui_image/ and ~/.cargo/registry/src/*/ratatui-image-11.0.6/{README.md,build.rs,Cargo.toml,src/lib.rs,src/picker.rs,src/picker/cap_parser.rs,src/thread.rs,src/protocol/halfblocks.rs,src/protocol/sixel.rs}
- https://docs.rs/tui-big-text/latest/tui_big_text/ and enum.PixelSize.html
- https://docs.rs/doobs-mpris/latest/doobs_mpris/
- https://raw.githubusercontent.com/mozilla-firefox/firefox/main/widget/gtk/MPRISServiceHandler.cpp
- https://raw.githubusercontent.com/mozilla-firefox/firefox/main/widget/gtk/MPRISServiceHandler.h
- https://raw.githubusercontent.com/mozilla-firefox/firefox/main/dom/media/mediacontrol/MediaStatusManager.cpp
- https://raw.githubusercontent.com/mozilla-firefox/firefox/main/dom/media/mediacontrol/MediaPlaybackStatus.cpp
- https://raw.githubusercontent.com/mozilla-firefox/firefox/main/dom/media/mediasession/MediaSession.h
- gh api repos/mozilla-firefox/firefox/contents/dom/media/mediaelement/HTMLMediaElement.cpp (raw)
- https://bugzilla.mozilla.org/show_bug.cgi?id=1659199
- https://raw.githubusercontent.com/bluez/bluez/master/tools/mpris-proxy.c
- https://raw.githubusercontent.com/captbaritone/webamp/master/packages/webamp/js/skinSprites.ts
- https://raw.githubusercontent.com/captbaritone/webamp/master/packages/webamp/js/constants.ts
- https://raw.githubusercontent.com/captbaritone/webamp/master/packages/webamp/js/selectors.ts
- https://raw.githubusercontent.com/captbaritone/webamp/master/packages/webamp/js/components/Vis.tsx
- https://raw.githubusercontent.com/captbaritone/webamp/master/packages/webamp/js/components/VisPainter.ts
- https://raw.githubusercontent.com/captbaritone/webamp/master/packages/webamp/js/components/MainWindow/Marquee.tsx
- https://raw.githubusercontent.com/captbaritone/webamp/master/packages/webamp/css/main-window.css
- https://raw.githubusercontent.com/captbaritone/webamp/master/packages/webamp/css/equalizer-window.css
- https://raw.githubusercontent.com/captbaritone/webamp/master/packages/webamp/css/playlist-window.css
- https://raw.githubusercontent.com/captbaritone/webamp/master/packages/webamp/js/skinParserUtils.ts
- https://gitlab.gnome.org/GNOME/vte/-/raw/master/meson_options.txt
- https://salsa.debian.org/gnome-team/vte2.91/-/raw/debian/latest/debian/rules
- https://git.launchpad.net/ubuntu/+source/vte2.91/plain/debian/rules
- https://gitlab.gnome.org/chergert/ptyxis/-/raw/main/NEWS
- https://gitlab.gnome.org/chergert/ptyxis/-/raw/main/src/ptyxis-terminal.c
- Local read-only commands: busctl --user (list/introspect/get-property/call/monitor), pw-dump, snap list, objdump -T /usr/bin/ptyxis, ldd, strings on libvte, dpkg -l, gsettings list-recursively org.gnome.Ptyxis, fc-list :charset=, python TIOCGWINSZ ioctl on /dev/pts/*, cargo search / cargo info, /home/mattbeam/workspace/astral-watch/Cargo.toml
