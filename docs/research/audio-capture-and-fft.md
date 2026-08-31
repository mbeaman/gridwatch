<!-- Research digest. Generated 2026-08-30 by the opsTui design workflow (research agents ran read-only against this machine and docs.rs). Version numbers and API names were verified on that date; re-check before pinning.  -->

# Playback-audio capture on PipeWire 1.6 (no dev headers) + spectrum/oscilloscope/VU DSP and ratatui rendering for the opsTui audio component

# Audio capture and visualization for opsTui (PipeWire 1.6.2, no dev headers)

## 1. Capture backends (ranked)

### 1a. `pw-record` subprocess — recommended default, verified end-to-end today
`pw-record` (pipewire-bin 1.6.2, compiled+linked against libpipewire 1.6.2) already produces exactly what we need: raw little-endian f32 interleaved stereo at 48 kHz on stdout. Verified command:

```
pw-record --target <object.serial|node.name> --format f32 --rate 48000 --channels 2 --raw -n 4800 -
```
returned 38 400 bytes (4800 frames × 2 ch × 4 B) with non-zero RMS (0.019–0.044) while Firefox + Cyberpunk were playing to the USB DAC. Facts established on this machine:

- **Targeting a sink captures its monitor.** A capture stream whose `target.object` is an `Audio/Sink` is linked by WirePlumber to the sink's `monitor_FL/FR` ports (ports 69/70 of node 61 have `port.monitor=true`). No `-C`/`stream.capture.sink` needed when the target is explicit. Local 1.6.2 has no `-C` flag; upstream master added one, but `-P '{ stream.capture.sink = true }'` works today with `--target auto` (verified, 19 200 B for 2400 frames) and is the form that **follows default-sink changes automatically**: `rescan.lua` schedules a relink on `default.audio.sink` metadata changes and the `linking.follow-default-target` setting ("Streams connected to the default device follow when default changes") is on. So no metadata watcher is required for the common case.
- **Name resolution:** `find-defined-target.lua` matches numeric `target.object` against `object.serial` (never `node.id`) and strings against `node.name` / `object.path`. `wpctl status` prints node *ids*, which only coincidentally equal serials for early nodes (node 61 → serial 61, but HDMI node 75 → serial 366). Use `pw-dump` JSON and read `info.props["object.serial"]` / `node.name`.
- **Fallback trap:** an unresolvable target silently falls back to the default source (`--target 99999` produced live audio, RMS 0.018). Add `-P '{ node.dont-fallback = true }'` to make it fail loudly instead (verified: "defined target not found", stream error, pw-record exits) — or accept the fallback deliberately.
- **Idle/suspended sinks:** with a normal (driving) stream, capturing the suspended HDMI sink still delivered zero-valued frames at full rate (RMS 0.0) — but it wakes the sink and keeps it running. With `-P '{ node.passive = true }'` the stream does not drive the graph: on the active sink it delivered data; on the idle sink it delivered **nothing** (4 s timeout, 0 bytes). Recommendation: `node.passive = true` + a 250 ms "no data → treat as silence, decay bars" rule, so opsTui never keeps a DAC awake.
- **Latency/cadence:** default `--latency 100ms` and `--latency 1024` both delivered 4096-byte chunks (512 frames = 10.7 ms) every ~10.6 ms, first data at ~14 ms — stdio block buffering on a pipe dominates. `stdbuf -o0 pw-record --latency 256 …` delivered 2048-byte chunks (256 frames) every 5.3 ms. Current graph quantum on the headphones sink is already 256 (the game requests it; `pw-top`), default is 1024; requesting `--latency` below the running quantum forces the whole graph down, so use 512–1024 and don't chase sub-10 ms.
- **Defaults to know:** format `s16`, rate 48000, channels 2, category `Capture`, role `Music`, `--quality 0..15` resampler (4). Settings: `clock.rate=48000`, `clock.allowed-rates=[48000]`, so no resampling happens at 48 k.
- **Lifecycle:** `pw-cat.c` quits the loop on `PW_STREAM_STATE_ERROR`/`UNCONNECTED` and on core errors, i.e. the process exits when PipeWire restarts or the target is destroyed with dont-fallback. Supervise: respawn with exponential backoff (250 ms → 5 s), kill on widget removal, read stdout on a dedicated thread.

Enumeration for the picker: `pw-dump` (≈280 KB JSON, ~10 ms) → objects of `type == "PipeWire:Interface:Node"` with `media.class == "Audio/Sink"` → `node.name`, `node.description`, `object.serial`, `info.state` (`running|suspended|idle`). Default sink: metadata object `metadata.name == "default"`, key `default.audio.sink` → `{"name": …}`. `pw-metadata -m` is block-buffered when piped (no output within 3 s; content only appeared after SIGINT flushed it), so poll `pw-dump` every ~2 s instead if you need the name for the title bar.

Rust shape:
```rust
let mut child = Command::new("pw-record")
    .args(["--format","f32","--rate","48000","--channels","2","--raw","--latency","512",
           "--target","auto","-P",
           r#"{ stream.capture.sink = true, node.passive = true, node.name = "opsTui audio", application.name = "opsTui" }"#,
           "-"])
    .stdout(Stdio::piped()).stderr(Stdio::piped()).spawn()?;
// reader thread: read into [u8; 16384], for c in buf.chunks_exact(8) { l = f32::from_le_bytes(c[0..4]), r = ... }
// push interleaved f32 into an rtrb::Producer<f32> (SPSC); UI thread drains into a 8192-frame ring per channel.
```

### 1b. `pulseaudio` 0.3.1 crate over pipewire-pulse — best in-process option with zero apt packages
`pulseaudio` (colinmarc, MIT) is a pure-Rust implementation of the PulseAudio native protocol (no libpulse). It connects to `$PULSE_SERVER`, `$PULSE_RUNTIME_PATH/pulse/native`, then `$XDG_RUNTIME_DIR/pulse/native` — `/run/user/1000/pulse/native` exists (pipewire-pulse 1.6.2). API: `Client::from_env(c"opsTui")` (sync), `client.list_sources().await -> Vec<SourceInfo>` (fields include `monitor_of_sink_index`, `monitor_of_sink_name`, `state`, `sample_spec`), `client.create_record_stream(RecordStreamParams { source_name: Some(c"@DEFAULT_MONITOR@"), sample_spec: SampleSpec{ format: SampleFormat::Float32Le, channels: 2, sample_rate: 48000 }, buffer_attr, flags, ..Default::default() }, |bytes: &[u8]| { … }).await`. `RecordSink` is implemented for `FnMut(&[u8]) + Send + 'static` and for `RecordBuffer` (AsyncRead). A fully synchronous protocol-level path (`protocol::Command::Auth`, `GetSourceInfo`, `read_descriptor`, examples/record.rs) exists if we don't want an executor on the audio thread. pipewire-pulse names monitors `<node.name>.monitor` and honours `@DEFAULT_MONITOR@` (strings in `libpipewire-module-protocol-pulse.so`; verified live with `ffmpeg -f pulse -i @DEFAULT_MONITOR@`, RMS 0.024). cpal's own PulseAudio host is built on this crate. Maturity: 0.3.x, shm/memfd zero-copy not implemented (irrelevant for a visualizer).

### 1c. cpal 0.18.2 — not buildable here without `libasound2-dev`
On Linux cpal depends unconditionally on `alsa 0.11` → `alsa-sys` whose `build.rs` runs `pkg_config::Config::new().probe("alsa")` and **panics** ("Pkg-config failed - usually this is because alsa development headers are not installed"); `pkg-config --modversion alsa` fails on torch. With that one package installed, `features = ["pulseaudio"]` is pure Rust (host order PipeWire > PulseAudio > ALSA; `HostId::PulseAudio`; input devices = all `list_sources()` incl. `.monitor`, `default_input_device` = `@DEFAULT_SOURCE@`). `features = ["pipewire"]` pulls the `pipewire` crate (see 1d) and its host exposes `Audio/Sink` nodes as Duplex devices and sets `STREAM_CAPTURE_SINK` on input streams — proper monitor capture. The ALSA host can only reach PipeWire through the `pipewire`/`default` PCM (pipewire-alsa), whose capture target is the default source; here `default.audio.source` currently equals the headphones **sink** name (so `arecord -D pipewire` captured the monitor, RMS 0.014) but that's not portable; `pipewire:NODE=<serial>` does target a specific sink monitor (verified silence from NODE=366).

### 1d. `pipewire` 0.10.1 (pipewire-rs) — feature-gated upgrade later
`pipewire-sys` build.rs: `system_deps::Config::new().probe()` (pkg-config `libpipewire-0.3`) + bindgen → needs `libpipewire-0.3-dev` (candidate 1.6.2-1ubuntu1.1, depends on `libspa-0.2-dev`) and libclang. MSRV 1.80. Code shape (docs.rs example `audio-capture.rs`): `pw::init()`, `MainLoop::new(None)`, `Context::new(&mainloop)`, `core = context.connect(None)`, `properties!{ *pw::keys::MEDIA_TYPE => "Audio", *pw::keys::MEDIA_CATEGORY => "Capture", *pw::keys::MEDIA_ROLE => "Music", *pw::keys::STREAM_CAPTURE_SINK => "true" }` (+ `TARGET_OBJECT`), `Stream::new(&core, "opsTui", props)`, `.add_local_listener_with_user_data(d).param_changed(|_, d, id, pod| AudioInfoRaw::parse(pod))` `.process(|stream, d| { let mut b = stream.dequeue_buffer(); let datas = b.datas_mut(); let n = datas[0].chunk().size(); datas[0].data() … })` `.register()`, format pod via `AudioInfoRaw` + `PodSerializer`, `stream.connect(Direction::Input, None, StreamFlags::AUTOCONNECT | MAP_BUFFERS | RT_PROCESS, &mut [pod])`, `mainloop.run()` on its own thread. Worth it only for sub-5 ms latency or to drop the subprocess; `pipewire-native` 0.1.4 (freedesktop's pure-Rust client, Feb 2026) cannot stream audio yet. `parec` is not installed; `ffmpeg -f pulse` works but is heavyweight.

## 2. DSP

Sample rate 48 kHz. Bin width Δf = fs/N: N=1024 → 46.9 Hz (21 ms), 2048 → 23.4 Hz (43 ms), 4096 → 11.7 Hz (85 ms). Default N=2048 with a per-frame hop (recompute on the latest N samples every render tick, ≥50 % overlap at 60 fps); use N=4096 for bass-heavy/wide footprints, or cava's trick of a longer FFT for the lowest bars.

```rust
use realfft::{RealFftPlanner, RealToComplex};             // realfft 3.5.0 (rustfft 6.4.1, AVX/SSE runtime-detected)
let fft = RealFftPlanner::<f32>::new().plan_fft_forward(N); // Arc<dyn RealToComplex<f32>>
let mut inp = fft.make_input_vec(); let mut out = fft.make_output_vec(); // N, N/2+1
let hann: Vec<f32> = (0..N).map(|i| 0.5 - 0.5*(2.0*PI*i as f32/(N as f32-1.0)).cos()).collect();
let wsum: f32 = hann.iter().sum();                          // ≈ N/2
for (d, (x, w)) in inp.iter_mut().zip(ring.latest(N).zip(&hann)) { *d = x*w; }
fft.process(&mut inp, &mut out)?;                           // unnormalized
let amp = |k: usize| out[k].norm() * 2.0 / wsum;            // full-scale sine -> 1.0
let dbfs = |a: f32| 20.0*(a.max(1e-9)).log10();
```
Log-spaced bars (20 Hz–20 kHz, or cava's 50–10 000 default): edges `f_k = f_lo·(f_hi/f_lo)^(k/B)`, bin `i_k = round(f_k·N/fs)`; per bar take max (punchy) or mean (smooth) of amplitudes in `[i_k, i_{k+1})`; when a bar spans <1 bin, interpolate between neighbouring bins Winamp-style (webamp uses `scaled = 0.09·linear + 0.91·log` index blending over 512 bins). cava's equivalent: `cut_off[n] = f_hi·10^(c·(n+1)/(B+1) − c)`, `c = log10(f_lo/f_hi)/(1/(B+1) − 1)`, magnitude `hypot(re,im)` summed per bar, then `eq[n] = 2^-28 · f_{n+1}^0.85 / log2(N) / bins_in_bar` — i.e. a ~5 dB/octave tilt; expose tilt as a theme/DSP knob (3–6 dB/oct) or A-weighting `A(f)=20log10(R_A(f))+2.0`, `R_A = 12194²f⁴/((f²+20.6²)√((f²+107.7²)(f²+737.9²))(f²+12194²))` (lookas does mel + A-weighting). Height: `h = clamp((dbfs − floor)/(0 − floor), 0, 1)` with floor −60…−70 dB; optional cava-style autosens: on any bar > 1.0 `sens *= 1 − 0.02·fm`, else when not silent `sens *= 1 + 0.001·fm·autosens` (fm = frame-rate mod).

Smoothing per bar (all frame-rate normalised with `a = 1 − exp(−dt/τ)`):
- Attack/release EMA: `y += (x>y ? a_att : a_rel)·(x−y)`, τ_att ≈ 10 ms, τ_rel ≈ 150–300 ms.
- Winamp gravity: rise instantly, fall `falloff/16` of full height per frame (options 3/6/12/16/32, default 12; vis is 16 px tall); peak: `if peak <= bar { peak = bar; v = 3.0 }`, each frame `peak -= v; v *= peak_falloff` (1.05/1.1/1.2/1.4/1.6, default 1.1) → accelerating drop.
- cava: on decrease `out = peak·(1 − fall²·g)`, `fall += 0.028`, `g = fm^2.5·2/noise_reduction`; then integral `out = mem·nr/fm^0.1 + out`, `noise_reduction` default 0.77.
- Monstercat: for each bar z and distance d: `bars[z±d] = max(bars[z±d], bars[z]/(monstercat·1.5)^d)`; "waves": `bars[z] /= 1.25; bars[z±d] = max(bars[z±d], bars[z] − hn·d²)`.
Stereo: separate FFT per channel; cava's default layout is left bars reversed on the left half, right bars on the right (mirrored, bass in the middle); mono = (L+R)/2.

Oscilloscope: latest `W·cols_per_cell` samples where a braille/octant canvas has 2 px per cell horizontally; for wider spans decimate by min/max envelope per column (Winamp: 576 samples → 75 columns, one sample per column, y quantised to 16 px, `oscStyle` lines/dots/solid; colours 18–22 by distance from centre). Optional rising-zero-crossing trigger to stabilise tones.

VU/peak: RMS over 300 ms (VU ballistics) or 50 ms, `dBFS = 20log10(rms)` (+3.01 dB sine reference if desired); sample peak with 1.5 s hold then 20 dB/s decay; clip lamp ≥ −0.1 dBFS. Loudness: `ebur128 0.1.10` (pure-Rust libebur128 port, MSRV 1.60): `EbuR128::new(2, 48000, Mode::M|Mode::S)`, `add_frames_f32(&interleaved)`, `loudness_momentary()/loudness_shortterm()` → LUFS.

## 3. Rendering (ratatui 0.30.2, Ptyxis 50.1 / VTE 0.84.0)
- Bars: `symbols::bar::NINE_LEVELS` ("▁▂▃▄▅▆▇█" + empty); height v∈[0,1] over H rows → `e = round(v·H·8)`; row r (from bottom) is `█` if `e ≥ 8(r+1)`, else partial `e − 8r`, else empty. Colour each row from the theme gradient (Winamp default viscolor.txt: indices 2–17 = red→orange→yellow→green bottom-to-top, 18–22 osc greys, 23 peak `rgb(150,150,150)`; retrowave = magenta→cyan). Peak marker: `▔`(U+2594) or `─` in the peak colour at the peak row. `Sparkline` (per-bar `SparklineBar` styles, `bar_set`, `direction`) and `BarChart` (`bar_width/bar_gap/bar_set/direction`, per-`Bar` style) can prototype, but a custom `Widget` writing `Buffer` cells is simpler for gradients + peaks.
- VTE draws Block Elements (U+2580–259F), sextants, and **octants (U+1CD00–1CDE5)** itself (minifont.cc), so `Marker::Octant` (2×4/cell, solid pixels) works in Ptyxis without any font; DejaVu Sans Mono lacks Braille (U+2800) — braille falls back to DejaVu Sans / Noto Sans Symbols2 (installed), which renders but from a proportional face. Use Octant on VTE/kitty, Braille elsewhere (`Canvas::marker`), `HalfBlock` when two colours per cell matter (Braille/Octant = one fg colour per cell). Liberation Mono lacks ▁–▇, so document the font requirement.
- Bars per width: thin (1 col, no gap) = width; Winamp-thick (2 cols + 1 gap, cava default) = ⌊(w+1)/3⌋: 20 cols → 20/7, 40 → 40/13, 80 → 80/26, 120 → 120/40, 200 → 200/66; stereo mirrored needs an even count; Winamp classic is 75 thin or 19 wide. A 1×1 grid cell can show a VU pair or a 6–10-bar mini spectrum; 2×1 a scope; 4×2 full mirrored spectrum + VU + title.
- Frame rate: VTE ≥ 0.76 paints on the GTK frame clock (60 Hz) instead of the old 40 Hz timer; target 30 fps for the dashboard tick, 60 fps for the audio widget (data arrives every 10.7 ms, so every frame is fresh). Don't exceed the terminal's refresh; skip DSP when the widget is hidden.

## 4. Prior art
cava (C) uses libpipewire directly: `pw_stream_new_simple` with `media.type=Audio, media.category=Capture, media.role=Music`, `stream.capture.sink=true` for `auto` or a `.monitor` suffix, else `target.object`; `node.latency = pow2(buffer)/rate`; `PW_KEY_NODE_ALWAYS_PROCESS` vs `NODE_PASSIVE`; `AUTOCONNECT|MAP_BUFFERS|RT_PROCESS`. catnip (Go) spawns `pw-cat --record --format f32 --rate R --latency N --channels C --target T --quality 0 --media-category Capture --media-role DSP --properties JSON --raw -`. lookas 1.10.0 (Rust, cpal 0.15 + realfft, mel/A-weighting/spring-damper) actually spawns `parec` on Linux after `pactl list short sources | grep .monitor`. terminal-vibes 1.6.6 (ratatui 0.29, rustfft, ringbuf) uses libpulse-binding (needs libpulse-dev). rmpc embeds cava; spotify_player has a visualizer. `cavacore` 2.0.2 is a Rust rewrite of cava's core (archived 2025-03) worth cribbing; `audioviz` 0.6.0 and `spectrum-analyzer` 1.8.0 (microfft, power-of-two only) add little over ~60 lines of realfft code.

## Recommendations

- **Default capture backend = spawn `pw-record --format f32 --rate 48000 --channels 2 --raw --latency 512 --target auto -P '{ stream.capture.sink = true, node.passive = true, node.name = "opsTui audio" }' -` and read f32le stereo from its stdout on a dedicated thread into an SPSC ring (rtrb 0.4).** — Verified on torch today: produces live monitor audio of the default sink with ~11 ms pipe latency (512-frame chunks every 10.6 ms), needs no headers or extra packages, follows default-sink changes automatically via WirePlumber's follow-default-target relinking, and node.passive avoids waking idle DACs. pw-record exits on PipeWire errors so a supervisor with backoff is required.
  - alternatives: `arecord -D pipewire[:NODE=<serial>] -f FLOAT_LE -c2 -r48000 -t raw -` (verified) or `ffmpeg -f pulse -i @DEFAULT_MONITOR@ -f f32le -` (verified) as emergency fallbacks; explicit `--target <object.serial|node.name>` plus `node.dont-fallback = true` when the user pins a sink.
- **First in-process upgrade (feature `audio-native`, no apt needed) = `pulseaudio` 0.3.1 pure-Rust protocol crate talking to pipewire-pulse's socket, recording `@DEFAULT_MONITOR@` (or `<node.name>.monitor`) as Float32Le/48k/2ch.** — Zero C dependencies; socket /run/user/1000/pulse/native exists; pipewire-pulse resolves @DEFAULT_MONITOR@ (verified via ffmpeg) and lists monitors with monitor_of_sink_name; API has both a sync protocol path and an async Client (tokio-compatible, matches the planned runtime). cpal's PulseAudio host is built on the same crate.
  - alternatives: `pipewire` 0.10.1 (pipewire-rs) behind a `audio-pipewire` feature once `libpipewire-0.3-dev libspa-0.2-dev clang` are installed — lowest latency, native stream.capture.sink, but bindgen at build time and CI complexity; pipewire-native 0.1.4 once it can stream.
- **Do NOT use cpal 0.18.2 in the default build; treat it (with `features=["pulseaudio"]` or `["pipewire"]`) as an optional host-abstraction only if libasound2-dev is accepted as a build prerequisite.** — cpal's Linux target depends unconditionally on `alsa 0.11` → `alsa-sys`, whose build.rs panics when `pkg-config alsa` is missing; torch has libasound.so.2 but no alsa.pc/headers. The abstraction adds nothing for a single-platform monitor capture.
  - alternatives: Install `libasound2-dev` (candidate 1.2.15.3-1ubuntu1.1) and use cpal's pure-Rust `pulseaudio` feature; the `pipewire` feature additionally needs libpipewire-0.3-dev.
- **DSP core: realfft 3.5.0 (rustfft 6.4.1) real-to-complex FFT, N=2048 default (4096 option), Hann window, log-spaced bars between configurable f_lo/f_hi (default 30 Hz–16 kHz, cava-like 50–10 k as preset), amplitude→dBFS with configurable floor (−65 dB) and tilt (+4 dB/oct), per-bar attack/release EMA + selectable 'winamp' gravity/peak-hold and 'cava' gravity/integral/monstercat filters, stereo mirrored layout.** — Concrete, well-understood formulas (cited from cava and webamp sources) with frame-rate-normalised coefficients; realfft avoids the unnecessary complex input and is unnormalised so scaling by 2/Σw yields dBFS directly; no allocation per frame using make_*_vec scratch buffers.
  - alternatives: spectrum-analyzer 1.8.0 (microfft, power-of-two only, allocates per call, MSRV 1.85.1), audioviz 0.6.0 (cpal-coupled), cavacore 2.0.2 (archived Rust rewrite of cava core — good reference).
- **Rendering: custom ratatui Widget writing NINE_LEVELS (U+2581–2588) bar cells with per-row theme gradient and a U+2594 peak marker; oscilloscope via Canvas with Marker::Octant on VTE/kitty and Marker::Braille fallback (auto-detect via VTE_VERSION/TERM_PROGRAM, user-overridable); HalfBlock where two colours per cell are needed; 30 fps dashboard tick, 60 fps when the audio widget is focused.** — VTE 0.84 (Ptyxis 50.1) draws block elements and octants internally (minifont.cc), so eighth-blocks and octants are pixel-perfect without Nerd Fonts; DejaVu Sans Mono lacks braille so braille depends on font fallback. VTE ≥0.76 renders at the GTK frame clock (60 Hz), and pw-record delivers new data every ~10 ms so 60 fps is always fresh.
  - alternatives: ratatui Sparkline (per-bar SparklineBar styles) and BarChart for quick prototypes; Marker::HalfBlock everywhere for maximum portability.
- **Component footprints: 1x1 = stereo VU/peak pair or 8–10 thin bars; 2x1 = scope or mono spectrum; 4x2+ = mirrored stereo spectrum (⌊(w+1)/3⌋ thick bars or w thin bars) with VU strip and sink name; 'winamp' skin preset = 19 thick / 75 thin bars, 16-level red→green viscolor gradient, grey peaks, osc modes lines/dots/solid.** — Matches cava defaults (bar_width 2, spacing 1) and classic Winamp geometry (76×16 px vis, 75/19 bands, falloff/peak-falloff options) verified from webamp's VisPainter.ts and the baseSkin colour table.
  - alternatives: Fixed bar counts per footprint; log-only vs 0.91 log/linear blend for the frequency axis.
- **Sink enumeration and default detection via `pw-dump` JSON (nodes with media.class Audio/Sink: node.name, node.description, object.serial, state; metadata 'default' → default.audio.sink) polled every ~2 s only when the picker is open or the title needs it; never rely on `wpctl status` ids for targeting.** — pw-dump is ~10 ms/280 KB; `target.object` must be an object.serial or node.name (WirePlumber find-defined-target.lua), and node ids differ from serials (HDMI sink id 75 vs serial 366). pw-metadata -m is block-buffered when piped and unusable as a follower.
  - alternatives: pw-mon -p event stream (verbose text, also stdio-buffered) or, in the pipewire-rs path, a Registry/Metadata listener.

## Crates

| crate | version | purpose | system deps | confidence |
|---|---|---|---|---|
| `realfft` | 3.5.0 | Real-to-complex FFT (N/2+1 bins) for spectrum bars; make_input_vec/make_output_vec/process_with_scratch, unnormalised output | none | verified |
| `rustfft` | 6.4.1 | FFT engine underneath realfft; default features avx/sse with runtime detection; also usable directly (FftPlanner::plan_fft_forward, Fft::process) | none | verified |
| `rtrb` | 0.4.0 | Lock-free SPSC ring buffer to hand f32 frames from the pw-record reader thread to the UI/DSP thread (alternative: ringbuf 0.5.1) | none | verified |
| `pulseaudio` | 0.3.1 | Pure-Rust PulseAudio native protocol client (Client::from_env, list_sources, create_record_stream with RecordStreamParams{source_name: @DEFAULT_MONITOR@ / <sink>.monitor}) over pipewire-pulse socket — in-process capture without libpulse | none (runtime: pipewire-pulse socket /run/user/1000/pulse/native, present) | verified |
| `ebur128` | 0.1.10 | Optional EBU R128 momentary/short-term loudness (LUFS) and true peak; pure-Rust port of libebur128 (cc only behind c-tests feature), MSRV 1.60 | none | verified |
| `pipewire` | 0.10.1 | Feature-gated native PipeWire capture stream later (Stream::new, properties!{STREAM_CAPTURE_SINK}, connect(Direction::Input, AUTOCONNECT/MAP_BUFFERS/RT_PROCESS)); MSRV 1.80 | apt: libpipewire-0.3-dev (1.6.2-1ubuntu1.1, depends on libspa-0.2-dev) + libclang for bindgen (system-deps/pkg-config probe of libpipewire-0.3) | verified |
| `cpal` | 0.18.2 | NOT recommended for default build: cross-platform host abstraction; Linux hosts PipeWire (feature pipewire) > PulseAudio (feature pulseaudio, pure Rust) > ALSA; PipeWire host exposes sinks as Duplex devices with STREAM_CAPTURE_SINK | apt: libasound2-dev always (non-optional alsa 0.11 → alsa-sys pkg-config probe panics without alsa.pc); plus libpipewire-0.3-dev for the pipewire feature | verified |
| `ratatui` | 0.30.2 | symbols::bar::NINE_LEVELS eighth-block bars, Sparkline (SparklineBar per-bar style), BarChart (bar_set/bar_width/bar_gap), Canvas with Marker::{Braille, Octant, Sextant, Quadrant, HalfBlock} | none | verified |
| `spectrum-analyzer` | 1.8.0 | Alternative one-call FFT+scaling (samples_fft_to_spectrum, hann_window, scale_20_times_log10); microfft backend, power-of-two only, MSRV 1.85.1 — not needed | none | verified |
| `cavacore` | 2.0.2 | Rust rewrite of cava's core (Cava::new(CavaOpts), execute(&samples,&mut bars)) — reference for cava algorithms; repository archived 2025-03-10 | none | verified |
| `pipewire-native` | 0.1.4 | freedesktop's pure-Rust PipeWire client (registry/introspection); cannot send/receive audio yet — watch for later arcs | none | verified |
| `audioviz` | 0.6.0 | cpal-based visualizer library (spectrum/processor/lissajous) — low doc coverage, cpal-coupled; skip | none (cpal feature would need libasound2-dev) | likely |

## Risks

- **pw-record silently falls back to the default source when an explicit --target serial/name disappears or is mistyped (verified with --target 99999), so a pinned-sink widget could quietly show a different sink's audio.** → Pass `node.dont-fallback = true` for pinned targets (stream errors, pw-record exits, supervisor shows 'sink gone' and re-enumerates), or use target auto + stream.capture.sink for 'follow default'.
- **With node.passive=true the pipe delivers nothing while the sink is idle (verified 0 bytes in 4 s), which looks identical to a hung child.** → Treat >250 ms without data as silence (decay bars) and only respawn on EOF/exit; optionally poll pw-dump state to display 'sink idle'.
- **pw-record exits when PipeWire/WirePlumber restarts, on stream errors, or if stdout closes; a crash loop could spam processes.** → Supervisor thread with exponential backoff (250 ms → 5 s), kill_on_drop, stderr captured into the widget's status line; audio component must degrade to 'no capture' without affecting the rest of the TUI.
- **Requesting --latency below the running graph quantum (currently 256 because the game asks for it; default 1024) forces the whole PipeWire graph to a smaller quantum, increasing CPU wakeups for every playing app.** → Use --latency 512–1024; latency floor is dominated by pipe/stdio (~11 ms) anyway; expose stdbuf -o0 + 256 only as an 'ultra-low-latency' option.
- **Braille glyphs are not in DejaVu Sans Mono/Noto Sans Mono/Liberation Mono; rendering relies on fontconfig fallback (DejaVu Sans, Noto Sans Symbols2) and may look uneven; Liberation Mono also lacks eighth blocks U+2581–2587.** → Prefer Marker::Octant on VTE ≥0.78/kitty (drawn by the terminal), keep Braille/HalfBlock fallbacks selectable, document DejaVu Sans Mono or a Nerd Font as the recommended terminal font.
- **`pulseaudio` crate is young (0.3.1, Jun 2026); protocol edge cases with pipewire-pulse (e.g. buffer_attr negotiation, cookie handling) could surface.** → Keep it behind a feature with the pw-record path as default; add an integration test that records 0.5 s from @DEFAULT_MONITOR@ and checks frame count.
- **Adding a capture stream at 60 fps FFT (N=2048–4096, 2 channels) plus per-frame redraw could cost noticeable CPU on a dashboard meant to run beside games.** → FFT only on visible widgets, cap widget fps (30 default), reuse realfft scratch buffers, run DSP on the reader thread and publish bar arrays via a triple buffer/Mutex; measure with `perf stat`-free wall-clock timing in debug overlay.
- **default.audio.source on this machine currently points at the headphones sink node name (arecord -D pipewire captured the monitor), which is unusual and may confuse any 'default source' logic and voice apps.** → Never depend on default.audio.source; always use stream.capture.sink=true or explicit sink targets; surface the active target name in the widget title.
- **MSRV drift: spectrum-analyzer 1.85.1, libloading 1.88, pulseaudio/realfft unspecified; opsTui's MSRV CI job could break if these are pulled in.** → Pin an MSRV (e.g. 1.85 like astral-watch or higher) and run `cargo +<msrv> check --all-features` in CI; keep optional backends behind features.

## Verified facts

- pw-record 1.6.2 flags (pw-record --help on torch): --target (object.serial or node.name, 'auto', '0'), --latency (units s/ms/us/ns or samples, default 100ms), -P/--properties JSON, --format (f32 supported; default s16), --rate (48000), --channels (2), --raw/-a, -n sample count, --quality 0-15
- `pw-record --target 61 --format f32 --rate 48000 --channels 2 --raw -n 4800 -` produced 38400 bytes with RMS L=0.024 R=0.044 (live playback of Firefox + Cyberpunk 2077) — monitor capture of a sink works by targeting the sink (ran on torch)
- `pw-record -P '{ stream.capture.sink = true }' --format f32 --raw -n 2400 -` (target auto) produced 19200 bytes (ran on torch)
- `pw-record --target alsa_output.usb-Generic_USB_Audio-00.HiFi__Headphones__sink …` works by node.name (19200 bytes) (ran on torch)
- Target resolution: --target 366 (HDMI sink object.serial, suspended) gave RMS 0.0; --target 75 (its node.id) and --target 99999 gave live audio (RMS 0.018) — i.e. numeric targets match object.serial only and unknown targets fall back to default (ran on torch; matches /usr/share/wireplumber/scripts/linking/find-defined-target.lua which constrains on object.serial / node.name / object.path)
- `-P '{ node.dont-fallback = true }'` with bad target → 'stream node 97 error: defined target not found', pw-record exits with no data (ran on torch; logic in find-defined-target.lua)
- `-P '{ node.passive = true }'`: 38400 bytes from active sink 61; 0 bytes in 4 s from suspended HDMI sink 366 (ran on torch). Non-passive capture of the suspended sink delivers zero-valued frames at full rate.
- pw-record stdout cadence: 4096-byte chunks (512 frames) every ~10.6 ms with --latency 100ms or 1024; with `stdbuf -o0` and --latency 256: 2048-byte chunks every ~5.3 ms (measured on torch with a Python reader)
- pw-top on torch: headphones sink node 61 running at QUANT 256 / 48000 (Cyberpunk 2077 stream 256, Firefox 3600); pw-metadata settings: clock.rate=48000, clock.allowed-rates=[48000], clock.quantum=1024, min 32, max 2048
- Node 61 props (wpctl inspect / pw-dump): node.name alsa_output.usb-Generic_USB_Audio-00.HiFi__Headphones__sink, object.serial 61, media.class Audio/Sink, state running, ports 67/68 playback_FL/FR (in), 69/70 monitor_FL/FR (out, port.monitor=true); HDMI sink node id 75 has object.serial 366
- Metadata 'default' on torch: default.audio.sink = {name: alsa_output.usb-Generic_USB_Audio-00.HiFi__Headphones__sink}; default.audio.source ALSO = that sink name; default.configured.audio.sink = alsa_output.pci-0000_0d_00.4.analog-stereo (pw-dump)
- WirePlumber 0.5.13 scripts: rescan.lua hooks metadata-changed on default.audio.sink/default.audio.source to schedule relinking; `wpctl settings` lists linking.follow-default-target ('Streams connected to the default device follow when default changes') and linking.allow-moving-streams; common-utils.lua getTargetDirection treats input streams with stream.capture.sink=true as targeting sinks
- pw-metadata -m produced no output to a pipe (even under stdbuf -oL / script pty) within 2-3 s; output appeared only in a file after SIGINT — block-buffered (ran on torch)
- pw-dump: ~280 KB JSON, ~0.00-0.01 s wall time, 6.4 MB RSS; wpctl status ~0.00 s (measured on torch)
- No dev headers: pkg-config alsa / libpipewire-0.3 / libpulse / libpulse-simple / jack / dbus-1 all 'not found'; /usr/include/alsa/asoundlib.h, pipewire-0.3/pipewire/pipewire.h, pulse/simple.h absent; runtime libs present: libasound.so.2, libpipewire-0.3.so.0 (1.6.2), libpulse.so.0 (17.0), libpulse-simple.so.0, libjack.so.0 (checked on torch)
- apt candidates available: libpipewire-0.3-dev 1.6.2-1ubuntu1.1 (Depends libspa-0.2-dev), libasound2-dev 1.2.15.3-1ubuntu1.1, libpulse-dev 1:17.0+dfsg1-2ubuntu4 (apt-cache policy/show on torch); clang not on PATH
- pipewire-alsa plugin installed (/usr/lib/x86_64-linux-gnu/alsa-lib/libasound_module_pcm_pipewire.so; 50-pipewire.conf defines pcm.pipewire with NODE arg → playback_node/capture_node; 99-pipewire-default.conf makes pcm.!default type pipewire). `arecord -q -D pipewire -f FLOAT_LE -c2 -r48000 -t raw -d1 -` gave 384000 bytes RMS 0.014; `-D pipewire:NODE=366` gave silence (ran on torch)
- ffmpeg on torch has the 'pulse' input device (links libpulse.so.0): `ffmpeg -f pulse -i @DEFAULT_MONITOR@ -t 0.5 -f f32le -ac 2 -ar 48000 -` produced 192768 bytes RMS 0.024 via pipewire-pulse (ran on torch); libpipewire-module-protocol-pulse.so contains strings @DEFAULT_MONITOR@, @DEFAULT_SINK@, @DEFAULT_SOURCE@, '%s.monitor'
- pipewire-pulse socket /run/user/1000/pulse/native exists; /run/user/1000/pipewire-0 and pipewire-0-manager sockets exist (ls on torch)
- cpal 0.18.2 Cargo.toml (docs.rs source): rust-version 1.85; Linux deps: alsa 0.11 non-optional, alsa-sys 0.4 optional (realtime), jack 0.13.5 optional, pipewire 0.10 (feature v0_3_53) optional, pulseaudio 0.3 optional; features pipewire=[dep:pipewire], pulseaudio=[dep:pulseaudio,dep:futures,dep:portable-atomic]
- cpal src/platform/mod.rs (GitHub master): Linux host list PipeWire (feature pipewire) > PulseAudio (feature pulseaudio) > Jack (feature jack) > Alsa; default_host() tries PipeWire, then PulseAudio, then ALSA. CHANGELOG: 0.18.0 (2026-06-06) added PipeWire and PulseAudio hosts; 0.18.1 excluded pipewire from docs.rs build; 0.18.2 (2026-08-16) PipeWire xrun reporting
- cpal PulseAudio host (src/host/pulseaudio/mod.rs): availability = pulseaudio::socket_path_from_env().is_some(); Client::from_env with 2 s timeout; input devices = all client.list_sources(); default input = protocol::DEFAULT_SOURCE; formats U8/I16/I24/I32/F32; record via stream::Stream::new_record; cpal PipeWire host (src/host/pipewire/device.rs): Audio/Sink nodes exposed as Duplex and input streams on them set STREAM_CAPTURE_SINK; default input from default.audio.source metadata; device id from node.name, display name node.description, object_serial used for TARGET_OBJECT
- alsa-sys build.rs (GitHub diwic/alsa-sys master): pkg_config::Config::new().statik(false).probe("alsa"), panics 'Pkg-config failed - usually this is because alsa development headers are not installed', no fallback
- pipewire-sys 0.10.1 build.rs (docs.rs source): system_deps::Config::new().probe().expect("Cannot find libpipewire") + bindgen builder.generate() — needs libpipewire-0.3 pkg-config, headers, libclang; cargo info: pipewire 0.10.1 rust-version 1.80
- libpulse-sys build.rs (GitHub jnqnfe/pulse-binding-rust): pkg-config 'libpulse' first, fallback to direct link 'pulse::libpulse.so.0' on Linux (no headers needed); libpulse-simple-binding 2.29.0 rust-version 1.63
- pulseaudio 0.3.1 (docs.rs, GitHub colinmarc/pulseaudio-rs): native Rust PulseAudio protocol; socket_path_from_env checks $PULSE_SERVER, $PULSE_RUNTIME_PATH/pulse/native, $XDG_RUNTIME_DIR/pulse/native; Client::from_env(client_name) sync, list_sources/list_sinks/source_info_by_name/server_info/create_record_stream(params, impl RecordSink) async; RecordSink = FnMut(&[u8])+Send+'static or RecordBuffer (AsyncRead); protocol::command::RecordStreamParams fields sample_spec, channel_map, source_index, source_name, buffer_attr, flags, cvolume, props, formats (Default impl); SourceInfo has monitor_of_sink_index / monitor_of_sink_name / state; examples: record.rs (sync protocol) and record_async.rs (#[tokio::main])
- pipewire-native 0.1.4 (docs.rs, 2026-02-23): pure-Rust PipeWire client by freedesktop (ford-prefect); registry/introspection only, 'further work is required for sending and receiving audio/video'
- cava input/pipewire.c (GitHub master): pw_stream_new_simple with media.type Audio / category Capture / role Music; source ending '.monitor' → strip + stream.capture.sink=true; 'auto' → stream.capture.sink=true; else target.object; node.latency = pow2/rate; NODE_ALWAYS_PROCESS vs NODE_PASSIVE; connect INPUT with AUTOCONNECT|MAP_BUFFERS|RT_PROCESS; S8/16/24/32 formats
- cava cavacore.c/cava.c/example config (GitHub master): cut-off frequency formula with log10(lower/upper)/(1/(bars+1)-1), hypot magnitude summed per bar, eq = 2^-28 * f^0.85 / log2(N) / bins; gravity_mod = fm^2.5*2/noise_reduction, fall += 0.028, integral out = mem*nr/fm^0.1 + out; autosens ×(1-0.02fm) on overshoot, ×(1+0.001fm*autosens) otherwise; Hann window; monstercat bars[m] = max(bars[z]/pow(monstercat*1.5, d), bars[m]); waves bars[z]/=1.25 and bars[m]=max(bars[z]-hn*d², bars[m]); defaults bars 0(auto), framerate 60, autosens 1, lower 50 Hz, higher 10000 Hz, noise_reduction 77, bar_width 2, bar_spacing 1, channels stereo, mono_option average
- webamp VisPainter.ts/Vis.tsx (GitHub master): fftSize 1024 / 512 bins, vis 76×16 px, 75 thin bars, 'wide' averages 4 → 19 bands, index blend scale=0.91 log / 0.09 linear, bar fall saFalloff -= falloff/16 (options 3,6,12,16,32; default 12), peaks: if peak<=bar {peak=bar; v=3.0}; peak -= v; v *= peakFalloff (1.05,1.1,1.2,1.4,1.6; default 1.1); bar colours viscolor indices 2..17, osc 18..22, peak 23; osc 576 samples → 75 columns; baseSkin.json colours: 2..17 = rgb(239,49,16) … rgb(24,132,8), 18..22 greys from rgb(255,255,255), 23 = rgb(150,150,150)
- ratatui 0.30.2 docs: symbols::bar::Set{full,seven_eighths,three_quarters,five_eighths,half,three_eighths,one_quarter,one_eighth,empty}, NINE_LEVELS = █▇▆▅▄▃▂▁ + empty, THREE_LEVELS; Marker::{Dot,Block,Bar,Braille(2x4),HalfBlock,Quadrant(2x2),Sextant(2x3),Octant(2x4),Custom}; Canvas marker()/x_bounds/y_bounds/paint(ctx.draw/layer/print), Braille = one fg colour per cell; Sparkline data(SparklineBar per-bar style)/max/bar_set/direction/absent_value_style; BarChart bar_width/bar_gap/group_gap/bar_set/direction/max, per-Bar style
- VTE minifont.cc (GitHub GNOME/vte master) draws internally: U+2500–257F box drawing, U+2580–259F block elements, U+25E2–25E5 triangles, U+1FB00–1FB3B sextants, U+1FB3C–1FB9F etc., U+1CD00–1CDE5 octants, U+1CE90–1CEAF sixteenths — braille is NOT in the list; torch has libvte-2.91 0.84.0-2 and ptyxis 50.1-1ubuntu2 (dpkg); VTE ≥ GNOME 46 renders via GdkFrameClock (~60 Hz) instead of the old ~40 fps timer (Phoronix / GNOME discourse)
- Font coverage on torch (fc-query/fc-list): DejaVu Sans Mono and Noto Sans Mono cover U+2500–25FF (incl. ▁–█); Liberation Mono only U+2580/2584/2588/258C/2590–2593; U+2800/U+28FF (braille) only in DejaVu Sans, DejaVu Serif, Noto Sans Symbols2; U+1FB00/1FB3C only in Noto Sans Symbols2
- cargo info (crates.io, 2026-08-30): cpal 0.18.2 (rust 1.85), alsa-sys 0.6.1, alsa 0.12.1, pipewire/pipewire-sys/libspa-sys 0.10.1 (rust 1.80), libpulse-sys 1.23.0, libpulse-simple-binding 2.29.0, rustfft 6.4.1 (default = avx,sse,neon; rust 1.61), realfft 3.5.0, spectrum-analyzer 1.8.0 (rust 1.85.1), audioviz 0.6.0, pulseaudio 0.3.1, pipewire-native 0.1.4, ebur128 0.1.10 (rust 1.60, features c-tests/capi/precision-true-peak), ringbuf 0.5.1, rtrb 0.4.0 (rust 1.38), apodize 1.0.0, libloading 0.9.0 (rust 1.88), cavacore 2.0.2, lookas 1.10.0, terminal-vibes 1.6.6
- Prior art: lookas 1.10.0 Linux backend spawns parec after `pactl list short sources` filtering '.monitor' (RUNNING first, then <default-sink>.monitor), README recommends libasound2-dev + pulseaudio-utils (GitHub rccyx/lookas src/audio/system/linux.rs); terminal-vibes 1.6.6 uses libpulse-binding/libpulse-simple-binding (needs libpulse-dev), ratatui 0.29, rustfft, ringbuf, HalfBlock/Braille canvases, 60 fps (lib.rs); catnip (Go) spawns `pw-cat --record --format f32 --rate --latency --channels --target --quality 0 --media-category Capture --media-role DSP --properties JSON --raw -` (GitHub noriah/catnip); rmpc integrates cava; cavacore-rs archived 2025-03-10
- astral-watch Cargo.toml: ratatui 0.29 optional behind 'tui', MSRV 1.85, std threads; tui.rs uses Sparkline, Gauge, Chart with symbols::Marker::Braille (read locally)

## Open questions

- Does a `--target auto` + `stream.capture.sink=true` pw-record stream really get moved when the user switches the default sink in GNOME? Script logic (rescan.lua + linking.follow-default-target) says yes, but it was not exercised live because switching the default sink would have interrupted the running game's audio.
- Why is default.audio.source on torch set to the headphones *sink* node name (so microphone-targeting apps capture the monitor)? WirePlumber quirk vs. user config — worth checking `wpctl status`/`~/.config/wireplumber` in a quiet moment; opsTui should not depend on it either way.
- Exact pipe latency budget target: is ~11 ms (stdio-buffered pw-record) acceptable for the scope/VU, or should the supervisor use `stdbuf -o0` + `--latency 256` (5 ms) when the widget is focused? Needs a perceptual check with the real widget.
- pw-metadata -m never flushed to a pipe within 3 s even under `stdbuf -oL`/pty; unclear whether it is a buffering quirk or event timing — pw-dump polling is the safe path, but a cheaper follower (pw-mon -p, or the pipewire-rs Metadata listener later) could be validated in an implementation session.
- `pulseaudio` 0.3.1 buffer_attr/latency behaviour with pipewire-pulse (fragment sizes, whether adjust_latency is honoured) is undocumented; needs an integration test before promoting it above pw-record.
- Whether cpal's ALSA host can open an arbitrary PCM string such as `pipewire:NODE=<serial>` (it enumerates snd_device_name_hint entries) — only relevant if libasound2-dev is ever adopted.
- cava's exact FFT-size-vs-rate table at 48 kHz (base 512 doubled per rate band; bass buffer 2×) was summarised, not quoted; confirm if replicating cava's dual-FFT bass path.
- MSRV of realfft 3.5.0 and pulseaudio 0.3.1 are not declared (cargo info rust-version unknown); confirm against the opsTui MSRV CI job.
- Whether Ptyxis honours `Marker::Octant` cell widths identically to braille when mixing octant scope and block bars in one widget (VTE draws octants itself, but wcwidth of U+1CD00.. in the Rust unicode-width crate should be checked = 1).

## Sources

- Local: pw-record --help / --version, pw-record --list-formats, man pw-cat (pipewire 1.6.2 on torch)
- Local: wpctl status, wpctl inspect 61, wpctl settings, pw-dump, pw-top -b, pw-metadata -m, pw-mon, pw-cli info 0
- Local: /usr/share/wireplumber/scripts/linking/find-defined-target.lua, find-default-target.lua, rescan.lua, lib/common-utils.lua (WirePlumber 0.5.13)
- Local: /usr/share/alsa/alsa.conf.d/50-pipewire.conf, 99-pipewire-default.conf; /usr/lib/x86_64-linux-gnu/alsa-lib/; strings of libpipewire-module-protocol-pulse.so
- Local: fc-query/fc-list glyph coverage of DejaVu Sans Mono, Noto Sans Mono, Liberation Mono; dpkg -l (libvte-2.91 0.84.0, ptyxis 50.1); apt-cache policy/show for -dev packages
- Local: /home/mattbeam/workspace/astral-watch/Cargo.toml and src/tui.rs
- https://raw.githubusercontent.com/PipeWire/pipewire/master/src/tools/pw-cat.c
- https://docs.pipewire.org/page_man_pw-cat_1.html
- https://raw.githubusercontent.com/karlstav/cava/master/input/pipewire.c
- https://raw.githubusercontent.com/karlstav/cava/master/cavacore.c
- https://raw.githubusercontent.com/karlstav/cava/master/cava.c
- https://raw.githubusercontent.com/karlstav/cava/master/example_files/config
- https://docs.rs/crate/cpal/0.18.2/source/Cargo.toml
- https://raw.githubusercontent.com/RustAudio/cpal/master/CHANGELOG.md
- https://raw.githubusercontent.com/RustAudio/cpal/master/src/platform/mod.rs
- https://raw.githubusercontent.com/RustAudio/cpal/master/src/host/pulseaudio/mod.rs
- https://raw.githubusercontent.com/RustAudio/cpal/master/src/host/pipewire/device.rs
- https://raw.githubusercontent.com/RustAudio/cpal/master/src/host/pipewire/stream.rs
- https://docs.rs/cpal/0.18.2/cpal/platform/enum.HostId.html
- https://raw.githubusercontent.com/diwic/alsa-sys/master/build.rs
- https://raw.githubusercontent.com/jnqnfe/pulse-binding-rust/master/pulse-sys/build.rs
- https://docs.rs/crate/pipewire-sys/0.10.1/source/build.rs
- https://docs.rs/crate/pipewire/0.10.1/source/examples/audio-capture.rs
- https://acalustra.com/playing-with-pipewire-audio-streams-and-rust.html
- https://github.com/colinmarc/pulseaudio-rs and examples/record.rs, examples/record_async.rs
- https://docs.rs/pulseaudio/0.3.1/pulseaudio/ (Client, RecordStream, RecordSink, socket_path_from_env, protocol::command::RecordStreamParams, SourceInfo, all.html)
- https://docs.rs/crate/pipewire-native/0.1.4
- https://docs.rs/realfft/3.5.0/realfft/
- https://docs.rs/spectrum-analyzer/1.8.0/spectrum_analyzer/
- https://docs.rs/audioviz/0.6.0/audioviz/
- https://docs.rs/ebur128/0.1.10/ebur128/
- https://raw.githubusercontent.com/ratatui/ratatui/main/ratatui-core/src/symbols/bar.rs
- https://docs.rs/ratatui/0.30.2/ratatui/symbols/enum.Marker.html
- https://docs.rs/ratatui/0.30.2/ratatui/widgets/canvas/struct.Canvas.html
- https://docs.rs/ratatui/0.30.2/ratatui/widgets/struct.Sparkline.html
- https://docs.rs/ratatui/0.30.2/ratatui/widgets/struct.BarChart.html
- https://raw.githubusercontent.com/GNOME/vte/master/src/minifont.cc
- https://arewelegacycomputingyet.com/
- https://www.phoronix.com/news/GNOME-Terminal-GTK4-WIP and https://discourse.gnome.org/t/terminal-and-vte-news/20030 (VTE frame clock)
- https://raw.githubusercontent.com/captbaritone/webamp/master/packages/webamp/js/components/VisPainter.ts
- https://raw.githubusercontent.com/captbaritone/webamp/master/packages/webamp/js/components/Vis.tsx
- https://raw.githubusercontent.com/captbaritone/webamp/master/packages/webamp/js/baseSkin.json
- https://www.meggamusic.co.uk/winamp/docs/help/Skins_Preferences.htm (Winamp analyzer falloff / peak falloff options)
- https://lib.rs/crates/lookas and https://raw.githubusercontent.com/rccyx/lookas/main/src/audio/system/linux.rs
- https://lib.rs/crates/terminal-vibes
- https://raw.githubusercontent.com/noriah/catnip/master/input/pipewire/pipewire.go
- https://github.com/TornaxO7/cavacore-rs
- https://github.com/mierak/rmpc (cava integration)
- cargo search / cargo info on crates.io (2026-08-30) for cpal, alsa-sys, pipewire, pulseaudio, rustfft, realfft, spectrum-analyzer, audioviz, ebur128, rtrb, ringbuf, cavacore, lookas, terminal-vibes, pipewire-native
