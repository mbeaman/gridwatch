//! The audio source (§5 cadence row, §8, brief arc 5 seam 2): the default
//! sink's monitor captured through a supervised `pw-record` child, an io
//! thread pumping its stdout into an SPSC ring, and the source thread waking
//! on the fps grid to drain the ring, run the cava-style DSP and publish one
//! batch. Data-driven: no frames is silence (2 Hz, zero bands), never a
//! restart; the child is respawned only on EOF/exit and killed 10 s after
//! the demand drops to `Hidden`. Never `pactl`/`parec`, never a PipeWire
//! crate (D17).

pub mod capture;
pub mod dsp;
pub mod sink;
pub mod supervise;

use std::any::Any;
use std::io::{BufRead, BufReader};
use std::process::Child;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use gridwatch_store::keys::audio::{self, AudioLevel, AudioSink, BANDS, SCOPE_LEN};
use gridwatch_store::{
    Control, Datum, Level, Sample, Source, SourceCtx, SourceInfo, SourceState, SourceStatus, Ts,
    Vec32, demo,
};

use capture::{CaptureArgs, Pulse, Target};
use dsp::{Dsp, DspConfig, PeakHold};
use supervise::{Action, Policy, Silence};

/// `[sources.audio]` (§9).
pub const OPTION_NAMES: &[&str] = &[
    "sink",
    "latency",
    "low_latency",
    "fft",
    "fft_bass",
    "lo_hz",
    "hi_hz",
    "floor_db",
    "tilt_db_oct",
    "fps",
];
pub const FPS_MIN: u64 = 5;
pub const FPS_MAX: u64 = 60;
/// How long an unavailable source waits before re-probing for the binary
/// and the socket.
pub const REPROBE: Duration = Duration::from_secs(10);

pub use gridwatch_store::keys::audio::SetSink;

#[derive(Clone, Debug, PartialEq)]
pub struct Options {
    pub target: Target,
    pub latency: u32,
    pub low_latency: bool,
    pub dsp: DspConfig,
    pub fps: u64,
}

impl Default for Options {
    fn default() -> Options {
        Options {
            target: Target::Auto,
            latency: 1024,
            low_latency: false,
            dsp: DspConfig::default(),
            fps: 30,
        }
    }
}

pub fn clamp_fps(v: i64) -> u64 {
    (v.max(0) as u64).clamp(FPS_MIN, FPS_MAX)
}

impl Options {
    pub fn from_table(t: &toml::Table) -> Options {
        let mut o = Options::default();
        let int = |k: &str| t.get(k).and_then(|v| v.as_integer());
        let float = |k: &str| {
            t.get(k)
                .and_then(|v| v.as_float().or_else(|| v.as_integer().map(|i| i as f64)))
        };
        if let Some(v) = t.get("sink") {
            o.target = match v {
                toml::Value::Integer(n) => Target::Serial((*n).max(0) as u32),
                toml::Value::String(s) => Target::parse(s),
                _ => Target::Auto,
            };
        }
        if let Some(n) = int("latency") {
            o.latency = (n.max(0) as u32).clamp(256, 4096);
            if i64::from(o.latency) != n {
                tracing::warn!("[sources.audio] latency = {n} clamped to {}", o.latency);
            }
        }
        if let Some(b) = t.get("low_latency").and_then(|v| v.as_bool()) {
            o.low_latency = b;
        }
        if let Some(n) = int("fft") {
            o.dsp.fft = n.max(1) as usize;
        }
        if let Some(n) = int("fft_bass") {
            o.dsp.fft_bass = n.max(1) as usize;
        }
        if let Some(f) = float("lo_hz") {
            o.dsp.lo_hz = f;
        }
        if let Some(f) = float("hi_hz") {
            o.dsp.hi_hz = f;
        }
        if let Some(f) = float("floor_db") {
            o.dsp.floor_db = f;
        }
        if let Some(f) = float("tilt_db_oct") {
            o.dsp.tilt_db_oct = f;
        }
        if let Some(n) = int("fps") {
            o.fps = clamp_fps(n);
            if o.fps as i64 != n {
                tracing::warn!("[sources.audio] fps = {n} clamped to {} (5–60)", o.fps);
            }
        }
        o.dsp = o.dsp.clone().normalised();
        o
    }

    pub fn period(&self) -> Duration {
        Duration::from_millis(1000 / self.fps.clamp(FPS_MIN, FPS_MAX))
    }
}

/// `gridwatch doctor`'s rows (seam 8): the binary answers `--version` and
/// the socket exists. Neither captures anything.
pub fn doctor() -> Vec<(gridwatch_store::Capability, bool, String)> {
    use gridwatch_store::Capability;
    let mut out = Vec::new();
    match std::process::Command::new("pw-record")
        .arg("--version")
        .output()
    {
        Ok(o) if o.status.success() => {
            let text = String::from_utf8_lossy(&o.stdout);
            let ver = text
                .lines()
                .find(|l| l.contains("Compiled with") || l.contains("libpipewire"))
                .or_else(|| text.lines().next())
                .unwrap_or("")
                .trim()
                .to_string();
            out.push((
                Capability::PwRecord,
                true,
                format!("pw-record on PATH ({ver})"),
            ));
        }
        Ok(o) => out.push((
            Capability::PwRecord,
            false,
            format!("pw-record --version exited {}", o.status),
        )),
        Err(e) => out.push((
            Capability::PwRecord,
            false,
            format!("pw-record not found ({e}) — install pipewire-bin"),
        )),
    }
    match capture::socket_path() {
        Some(p) if p.exists() => out.push((
            Capability::PipeWireSocket,
            true,
            format!("{} present", p.display()),
        )),
        Some(p) => out.push((
            Capability::PipeWireSocket,
            false,
            format!("{} missing — is pipewire.service running?", p.display()),
        )),
        None => out.push((
            Capability::PipeWireSocket,
            false,
            "no XDG_RUNTIME_DIR — no PipeWire socket to find".into(),
        )),
    }
    out
}

/// The live child plus its threads and ring.
struct Capture {
    child: Child,
    ring: rtrb::Consumer<f32>,
    pulse: Arc<Pulse>,
    io: Option<JoinHandle<()>>,
    err: Option<JoinHandle<()>>,
    started: Instant,
}

impl Capture {
    fn spawn(args: &CaptureArgs, epoch: Instant) -> std::io::Result<Capture> {
        let mut child = capture::spawn(args)?;
        let stdout = child.stdout.take().expect("piped stdout");
        let stderr = child.stderr.take();
        let (mut prod, cons) =
            rtrb::RingBuffer::<f32>::new(capture::RING_FRAMES * capture::CHANNELS);
        let pulse = Pulse::new(epoch);
        let p2 = Arc::clone(&pulse);
        let io = std::thread::Builder::new()
            .name("gw-audio-io".into())
            .spawn(move || {
                if let Err(e) = capture::pump(stdout, &mut prod, &p2) {
                    tracing::warn!("pw-record stdout: {e}");
                }
            })?;
        let err = stderr.and_then(|e| {
            std::thread::Builder::new()
                .name("gw-audio-err".into())
                .spawn(move || {
                    for line in BufReader::new(e).lines().map_while(Result::ok) {
                        let l = line.trim();
                        if !l.is_empty() {
                            tracing::warn!("pw-record: {l}");
                        }
                    }
                })
                .ok()
        });
        Ok(Capture {
            child,
            ring: cons,
            pulse,
            io: Some(io),
            err,
            started: Instant::now(),
        })
    }

    /// EOF on stdout or the process gone.
    fn exited(&mut self) -> bool {
        !self.pulse.alive.load(Ordering::Acquire) || matches!(self.child.try_wait(), Ok(Some(_)))
    }

    fn kill(mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(h) = self.io.take() {
            let _ = h.join();
        }
        if let Some(h) = self.err.take() {
            let _ = h.join();
        }
    }
}

/// Per-channel history and meters.
struct Channel {
    history: Vec<f32>,
    peak: PeakHold,
}

impl Channel {
    fn new(cap: usize) -> Channel {
        Channel {
            history: Vec::with_capacity(cap * 2),
            peak: PeakHold::default(),
        }
    }

    fn push(&mut self, v: f32, keep: usize) {
        self.history.push(v);
        if self.history.len() >= keep * 2 {
            let n = self.history.len();
            self.history.copy_within(n - keep.., 0);
            self.history.truncate(keep);
        }
    }
}

pub struct AudioSource {
    options: Options,
}

impl AudioSource {
    pub fn new(options: &toml::Table) -> AudioSource {
        AudioSource {
            options: Options::from_table(options),
        }
    }
}

fn status(cx: &SourceCtx, state: SourceState, reason: Option<&str>, hint: Option<&str>) {
    cx.status(SourceStatus {
        state,
        reason: reason.map(Arc::from),
        hint: hint.map(Arc::from),
        since: cx.clock.now(),
        last_sample: None,
        dropped: 0,
        restarts: cx.restarts,
    });
}

fn sink_sample(s: &AudioSink) -> Sample {
    Sample {
        id: audio::SINK.id.clone(),
        datum: Datum::Record(Arc::new(s.clone())),
    }
}

/// The sink Record for a target when `pw-dump` cannot say (no `pw-dump`, or
/// the target is not in the list): the target text itself.
fn sink_fallback(target: &Target) -> AudioSink {
    AudioSink {
        name: target.arg(),
        description: match target {
            Target::Auto => "default sink".into(),
            Target::Name(n) => n.clone(),
            Target::Serial(n) => format!("sink #{n}"),
        },
        serial: match target {
            Target::Serial(n) => *n,
            _ => 0,
        },
        state: "unknown".into(),
        is_default: matches!(target, Target::Auto),
        rate: capture::RATE,
        channels: capture::CHANNELS as u8,
    }
}

/// What one loop pass emits.
struct Publisher {
    dsp: Dsp,
    chans: [Channel; 2],
    silence: Silence,
    fps: u64,
    /// Samples drained since the last publish, per channel.
    drained: [Vec<f32>; 2],
}

impl Publisher {
    fn new(o: &Options) -> Publisher {
        let dsp = Dsp::new(o.dsp.clone());
        let keep = dsp
            .history_len()
            .max((capture::RATE as f64 * dsp::RMS_WINDOW_S) as usize);
        Publisher {
            dsp,
            chans: [Channel::new(keep), Channel::new(keep)],
            silence: Silence::default(),
            fps: o.fps,
            drained: [Vec::new(), Vec::new()],
        }
    }

    fn keep(&self) -> usize {
        self.dsp
            .history_len()
            .max((capture::RATE as f64 * dsp::RMS_WINDOW_S) as usize)
    }

    /// Drain the ring into the histories; returns the frames drained.
    fn drain(&mut self, ring: &mut rtrb::Consumer<f32>) -> usize {
        let keep = self.keep();
        self.drained[0].clear();
        self.drained[1].clear();
        let mut ch = 0;
        let mut frames = 0;
        while let Ok(v) = ring.pop() {
            self.chans[ch].push(v, keep);
            self.drained[ch].push(v);
            if ch == 1 {
                frames += 1;
            }
            ch ^= 1;
        }
        frames
    }

    /// The batch for this tick: bands, scope, levels, dsp_ms — and the
    /// silence Record when it flipped.
    fn samples(&mut self, now_s: f64, silent_changed: bool, at: Ts) -> Vec<Sample> {
        let t0 = Instant::now();
        let mut out = Vec::with_capacity(12);
        let vec = |a: &[f32]| -> Vec32 { Arc::from(a) };
        let silent = self.silence.silent;
        let zero = [0f32; BANDS];
        for ch in 0..2 {
            let bands = if silent {
                zero
            } else {
                self.dsp.bands(&self.chans[ch].history)
            };
            out.push(Sample {
                id: audio::BANDS_KEY.idx(ch as u16).id,
                datum: Datum::Vector(vec(&bands)),
            });
        }
        // The scope: the latest 512 mono samples.
        let mut scope = [0f32; SCOPE_LEN];
        let (l, r) = (&self.chans[0].history, &self.chans[1].history);
        let n = l.len().min(r.len()).min(SCOPE_LEN);
        if !silent && n > 0 {
            let (ls, rs) = (&l[l.len() - n..], &r[r.len() - n..]);
            for i in 0..n {
                scope[SCOPE_LEN - n + i] = (ls[i] + rs[i]) * 0.5;
            }
        }
        out.push(Sample {
            id: audio::SCOPE.id.clone(),
            datum: Datum::Vector(vec(&scope)),
        });
        for ch in 0..2 {
            let (rms, peak) = if silent {
                (audio::FLOOR_DB, audio::FLOOR_DB)
            } else {
                let rms = dsp::rms_db(&self.chans[ch].history, capture::RATE, dsp::RMS_WINDOW_S);
                let inst = dsp::peak_db(&self.drained[ch]);
                (rms, self.chans[ch].peak.feed(inst, now_s))
            };
            out.push(Sample {
                id: audio::RMS_DB.idx(ch as u16).id,
                datum: Datum::Scalar(rms),
            });
            out.push(Sample {
                id: audio::PEAK_DB.idx(ch as u16).id,
                datum: Datum::Scalar(peak),
            });
        }
        if silent_changed {
            out.push(Sample {
                id: audio::LEVEL.id.clone(),
                datum: Datum::Record(Arc::new(AudioLevel { silent, since: at })),
            });
        }
        out.push(Sample {
            id: audio::DSP_MS.id.clone(),
            datum: Datum::Scalar(t0.elapsed().as_secs_f64() * 1000.0),
        });
        out
    }

    /// RMS of what arrived this tick (both channels), for the silence rule.
    fn drained_rms_db(&self) -> Option<f64> {
        let n = self.drained[0].len() + self.drained[1].len();
        if n == 0 {
            return None;
        }
        let ms = self
            .drained
            .iter()
            .flatten()
            .map(|s| f64::from(*s).powi(2))
            .sum::<f64>()
            / n as f64;
        Some((10.0 * ms.max(1e-20).log10()).max(audio::FLOOR_DB))
    }
}

impl Source for AudioSource {
    fn info(&self) -> SourceInfo {
        demo::audio_info()
    }

    fn run(mut self: Box<Self>, cx: SourceCtx) {
        status(&cx, SourceState::Starting, None, None);
        let epoch = Instant::now();
        let mut policy = Policy::default();
        let mut publisher = Publisher::new(&self.options);
        let mut capture: Option<Capture> = None;
        let mut enumerate = false;
        let mut last_enumerate: Option<Instant> = None;
        let mut dump: Option<sink::Dump> = None;
        let mut sink_published = false;
        let mut available = false;
        let mut degraded_reason: Option<String> = None;
        loop {
            if cx.stopped() {
                break;
            }
            // Controls first: the picker's choice, live fps, enumeration.
            let mut restart = false;
            while let Some(c) = cx.try_control() {
                match c {
                    Control::Stop => break,
                    Control::Restart => restart = true,
                    Control::SetOption(k, v) => match k.as_str() {
                        "fps" => {
                            if let Some(n) = v.as_integer() {
                                self.options.fps = clamp_fps(n);
                                publisher.fps = self.options.fps;
                            }
                        }
                        "enumerate" => {
                            enumerate = v.as_bool().unwrap_or(false);
                            if enumerate {
                                last_enumerate = None;
                            }
                        }
                        "sink" => {
                            if let Some(s) = v.as_str() {
                                self.options.target = Target::parse(s);
                                restart = true;
                            }
                        }
                        other => tracing::debug!("[sources.audio] ignoring option {other}"),
                    },
                    Control::Domain(b) => {
                        let any: Box<dyn Any + Send> = b;
                        match any.downcast::<SetSink>() {
                            Ok(s) => {
                                self.options.target = Target::parse(&s.0);
                                restart = true;
                            }
                            Err(_) => tracing::debug!("[sources.audio] unknown Domain control"),
                        }
                    }
                }
            }
            if cx.stopped() {
                break;
            }
            if restart {
                if let Some(c) = capture.take() {
                    c.kill();
                }
                policy.on_killed();
                sink_published = false;
            }
            let now = Instant::now();
            let level = cx.demand.level();

            // Availability: the binary and the socket, re-probed while absent.
            if !available {
                if !capture::on_path("pw-record") {
                    status(
                        &cx,
                        SourceState::Unavailable,
                        Some("pw-record not found"),
                        Some("install pipewire-bin"),
                    );
                } else if !capture::socket_present() {
                    status(
                        &cx,
                        SourceState::Unavailable,
                        Some("no PipeWire socket"),
                        Some("is pipewire.service running? ($XDG_RUNTIME_DIR/pipewire-0)"),
                    );
                } else {
                    available = true;
                }
                if !available {
                    if !cx.sleep_until(cx.next_deadline(REPROBE)) {
                        break;
                    }
                    continue;
                }
            }

            // The child's lifecycle.
            let running = capture.as_mut().is_some_and(|c| !c.exited());
            if let Some(c) = capture.as_mut()
                && c.exited()
            {
                let age = c.started.elapsed();
                if let Some(c) = capture.take() {
                    c.kill();
                }
                if let Action::RespawnAt(at) = policy.on_exit(now) {
                    let reason = format!(
                        "pw-record exited after {:.1} s; retry in {} ms",
                        age.as_secs_f64(),
                        at.saturating_duration_since(now).as_millis()
                    );
                    tracing::warn!("{reason}");
                    degraded_reason = Some(reason);
                }
            }
            match policy.decide(level, running, now) {
                Action::Spawn => {
                    let args = CaptureArgs {
                        target: self.options.target.clone(),
                        latency: self.options.latency,
                        low_latency: self.options.low_latency && level != Level::Hidden,
                    };
                    match Capture::spawn(&args, epoch) {
                        Ok(c) => {
                            capture = Some(c);
                            degraded_reason = None;
                            status(&cx, SourceState::Ok, Some("pw-record"), None);
                            // The sink Record for this generation.
                            if dump.is_none() {
                                dump = sink::enumerate().ok();
                            }
                            let rec = dump
                                .as_ref()
                                .and_then(|d| d.resolve(&self.options.target).cloned())
                                .unwrap_or_else(|| sink_fallback(&self.options.target));
                            cx.emit(cx.clock.now(), vec![sink_sample(&rec)]);
                            sink_published = true;
                        }
                        Err(e) => {
                            let reason = format!("pw-record failed to start: {e}");
                            tracing::warn!("{reason}");
                            policy.on_exit(now);
                            degraded_reason = Some(reason);
                        }
                    }
                }
                Action::Kill => {
                    if let Some(c) = capture.take() {
                        tracing::info!("pw-record stopped: hidden for 10 s");
                        c.kill();
                    }
                }
                Action::Keep | Action::RespawnAt(_) => {}
            }
            if let Some(r) = degraded_reason.as_deref()
                && capture.is_none()
                && level != Level::Hidden
            {
                status(&cx, SourceState::Degraded, Some(r), None);
            }

            // Hidden: nothing to publish; park a second at a time.
            if level == Level::Hidden {
                if !cx.sleep_until(cx.next_deadline(Duration::from_secs(1))) {
                    break;
                }
                continue;
            }

            // Enumeration for the picker / the sink name.
            if enumerate
                && last_enumerate
                    .is_none_or(|t| now.saturating_duration_since(t) >= sink::ENUMERATE_EVERY)
            {
                last_enumerate = Some(now);
                if let Ok(d) = sink::enumerate() {
                    let at = cx.clock.now();
                    let mut samples = vec![Sample {
                        id: audio::SINKS.id.clone(),
                        datum: Datum::Record(Arc::new(d.record())),
                    }];
                    if !sink_published && let Some(s) = d.resolve(&self.options.target) {
                        samples.push(sink_sample(s));
                        sink_published = true;
                    }
                    cx.emit(at, samples);
                    dump = Some(d);
                }
            }

            // Drain, judge silence, publish.
            let frames = match capture.as_mut() {
                Some(c) => publisher.drain(&mut c.ring),
                None => 0,
            };
            if frames > 0 {
                policy.on_frames();
            }
            let age = capture.as_ref().and_then(|c| c.pulse.age(now));
            let rms = publisher.drained_rms_db();
            let changed = publisher
                .silence
                .observe(now, age, rms, self.options.dsp.floor_db);
            let at = cx.clock.now();
            let samples = publisher.samples(
                now.saturating_duration_since(epoch).as_secs_f64(),
                changed,
                at,
            );
            cx.emit(at, samples);

            let period = if publisher.silence.silent {
                supervise::SILENT_PERIOD
            } else {
                self.options.period()
            };
            if !cx.sleep_until(cx.next_deadline(period)) {
                break;
            }
        }
        if let Some(c) = capture.take() {
            c.kill();
        }
    }
}

pub fn start(options: &toml::Table) -> Box<dyn Source> {
    Box::new(AudioSource::new(options))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn options_parse_and_clamp() {
        let t: toml::Table = toml::from_str(
            r#"sink = 61
latency = 100000
low_latency = true
fft = 4096
fft_bass = 100
lo_hz = 20
hi_hz = 20000
floor_db = -70
tilt_db_oct = 3
fps = 200"#,
        )
        .unwrap();
        let o = Options::from_table(&t);
        assert_eq!(o.target, Target::Serial(61));
        assert_eq!(o.latency, 4096);
        assert!(o.low_latency);
        assert_eq!((o.dsp.fft, o.dsp.fft_bass), (4096, 4096));
        assert_eq!(o.dsp.lo_hz, 20.0);
        assert_eq!(o.dsp.hi_hz, 20_000.0);
        assert_eq!(o.dsp.floor_db, -70.0);
        assert_eq!(o.fps, 60);
        assert_eq!(o.period(), Duration::from_millis(16));
        let t: toml::Table = toml::from_str(r#"sink = "alsa_output.x""#).unwrap();
        assert_eq!(
            Options::from_table(&t).target,
            Target::Name("alsa_output.x".into())
        );
        assert_eq!(Options::from_table(&toml::Table::new()), Options::default());
        assert_eq!(OPTION_NAMES.len(), 10);
    }

    #[test]
    fn set_sink_downcasts_from_a_domain_control() {
        let c = Control::Domain(Box::new(SetSink("61".into())));
        let Control::Domain(b) = c else { panic!() };
        let s = b.downcast::<SetSink>().expect("SetSink");
        assert_eq!(Target::parse(&s.0), Target::Serial(61));
    }

    /// The publisher over a generated stream: bands light for a 1 kHz tone,
    /// the silence rule flips on a quiet stream, and nothing allocates
    /// beyond the histories (checked by hand: `samples` builds Arcs only).
    #[test]
    fn the_publisher_runs_the_dsp_over_a_ring() {
        let o = Options::default();
        let mut p = Publisher::new(&o);
        let (mut prod, mut cons) = rtrb::RingBuffer::<f32>::new(capture::RING_FRAMES * 2);
        let n = 4096;
        for i in 0..n {
            let v = (std::f64::consts::TAU * 1_000.0 * i as f64 / 48_000.0).sin() as f32;
            prod.push(v).unwrap();
            prod.push(v * 0.5).unwrap();
        }
        assert_eq!(p.drain(&mut cons), n);
        let now = Instant::now();
        let changed = p
            .silence
            .observe(now, Some(Duration::ZERO), p.drained_rms_db(), -65.0);
        assert!(!changed, "starts not-silent with a loud stream");
        assert!(!p.silence.silent);
        let s = p.samples(1.0, changed, Ts(1));
        let bands: Vec<&Sample> = s.iter().filter(|x| x.id.name == "audio.bands").collect();
        assert_eq!(bands.len(), 2);
        let Datum::Vector(l) = &bands[0].datum else {
            panic!()
        };
        assert_eq!(l.len(), BANDS);
        assert!(l.iter().cloned().fold(0f32, f32::max) > 0.9, "{l:?}");
        let peaks: Vec<f64> = s
            .iter()
            .filter(|x| x.id.name == "audio.peak_db")
            .map(|x| match x.datum {
                Datum::Scalar(v) => v,
                _ => panic!(),
            })
            .collect();
        assert!(
            peaks[0] > -0.1 && peaks[1] < -5.9 && peaks[1] > -6.1,
            "{peaks:?}"
        );
        assert!(s.iter().any(|x| x.id.name == "audio.scope"));
        assert!(s.iter().any(|x| x.id.name == "audio.dsp_ms"));
        assert!(!s.iter().any(|x| x.id.name == "audio.level"));
        // Silence: no frames for 300 ms.
        assert_eq!(p.drain(&mut cons), 0);
        let changed = p.silence.observe(
            now + Duration::from_millis(300),
            Some(Duration::from_millis(300)),
            None,
            -65.0,
        );
        assert!(changed && p.silence.silent);
        let s = p.samples(1.3, changed, Ts(2));
        assert!(s.iter().any(|x| x.id.name == "audio.level"));
        let Datum::Vector(l) = &s[0].datum else {
            panic!()
        };
        assert!(l.iter().all(|v| *v == 0.0), "silent bands are zeros");
    }

    #[test]
    fn channel_history_keeps_the_latest_without_growing() {
        let mut c = Channel::new(100);
        for i in 0..1000 {
            c.push(i as f32, 100);
        }
        assert!(c.history.len() < 200);
        assert_eq!(*c.history.last().unwrap(), 999.0);
        let tail = &c.history[c.history.len() - 100..];
        assert_eq!(tail[0], 900.0);
    }

    /// Runs `pw-record` for two seconds on torch and prints the chunk cadence
    /// and RMS. A safe read-only probe (MACHINE.md); ignored in CI.
    #[test]
    #[ignore]
    fn live_pw_record_delivers_frames() {
        let epoch = Instant::now();
        let args = CaptureArgs {
            target: Target::Auto,
            latency: 1024,
            low_latency: false,
        };
        let mut c = Capture::spawn(&args, epoch).expect("pw-record spawns");
        let mut p = Publisher::new(&Options::default());
        let mut ticks = 0;
        let mut frames = 0;
        while epoch.elapsed() < Duration::from_secs(2) {
            std::thread::sleep(Duration::from_millis(33));
            frames += p.drain(&mut c.ring);
            ticks += 1;
        }
        let rms = dsp::rms_db(&p.chans[0].history, capture::RATE, dsp::RMS_WINDOW_S);
        println!(
            "pw-record: {frames} frames in {ticks} ticks ({:.1} frames/tick), age {:?}, rms {rms:.1} dBFS, alive {}",
            frames as f64 / ticks as f64,
            c.pulse.age(Instant::now()),
            !c.exited()
        );
        c.kill();
    }
}
