//! Deterministic audio synthesis (§12.5, brief arc 5 seam 3): a "song" of
//! three seeded partials with slow envelopes — a bass at 55–110 Hz, a lead
//! sweeping 400 Hz–2 kHz, a hat-like noise burst every 500 ms — rendered as
//! band heights, a scope and levels **from the partials' analytic envelopes**
//! (no FFT: the store depends on nothing but serde). The first 1.5 s are
//! silent so `--demo` covers the silence path. Byte-deterministic per
//! `(seed, Ts)`.

use std::sync::Arc;
use std::time::Duration;

use crate::demo::XorShift;
use crate::key::{Datum, Vec32};
use crate::keys::audio::{self, AudioLevel, AudioSink, BANDS, SCOPE_LEN};
use crate::msg::{Batch, Sample};
use crate::source::{Cadence, Source, SourceCtx, SourceInfo, SourceState, SourceStatus};
use crate::ts::Ts;

/// Silence at the start of every run, so the silence rule is exercised.
pub const SILENT_UNTIL_S: f64 = 1.5;
pub const LO_HZ: f64 = 30.0;
pub const HI_HZ: f64 = 16_000.0;
pub const SAMPLE_RATE: f64 = 48_000.0;

/// The synthetic sink — torch's USB DAC by name.
pub fn audio_sink() -> AudioSink {
    AudioSink {
        name: "alsa_output.usb-Topping_D50s-00.analog-stereo".into(),
        description: "D50s (synthetic)".into(),
        serial: 61,
        state: "running".into(),
        is_default: true,
        rate: 48_000,
        channels: 2,
    }
}

/// The band index a frequency falls in (the same log spacing the DSP uses).
pub fn band_of(hz: f64) -> usize {
    if hz <= LO_HZ {
        return 0;
    }
    let k = (hz / LO_HZ).ln() / (HI_HZ / LO_HZ).ln() * BANDS as f64;
    (k.floor() as usize).min(BANDS - 1)
}

#[derive(Clone, Debug)]
pub struct AudioSynth {
    rng: XorShift,
    sink_sent: bool,
    was_silent: Option<bool>,
}

/// One partial at `t`: frequency, amplitude 0..1, stereo pan −1..1.
fn partials(t: f64) -> [(f64, f64, f64); 3] {
    let beat = (t * 2.0).fract(); // 120 bpm
    let bass_hz = 55.0 * 2f64.powf(((t / 4.0).floor() as u32 % 4) as f64 / 4.0);
    let bass = (1.0 - beat).powf(1.5) * 0.9;
    let lead_hz = 400.0 * 5f64.powf(((t / 8.0) * std::f64::consts::TAU).sin() * 0.5 + 0.5);
    let lead = ((t * 0.5).sin() * 0.5 + 0.5) * 0.6;
    let hat = if beat < 0.08 {
        0.8 * (1.0 - beat / 0.08)
    } else {
        0.0
    };
    [
        (bass_hz, bass, -0.1),
        (lead_hz, lead, 0.3),
        (9_000.0, hat, 0.0),
    ]
}

impl AudioSynth {
    pub fn new(seed: u64) -> AudioSynth {
        AudioSynth {
            rng: XorShift::new(seed.wrapping_add(0x0061_7564)),
            sink_sent: false,
            was_silent: None,
        }
    }

    pub fn silent_at(at: Ts) -> bool {
        at.as_secs_f64() < SILENT_UNTIL_S
    }

    /// The 64 band heights per channel at `at`.
    pub fn bands_at(&mut self, at: Ts) -> ([f32; BANDS], [f32; BANDS]) {
        let mut l = [0f32; BANDS];
        let mut r = [0f32; BANDS];
        if Self::silent_at(at) {
            return (l, r);
        }
        let t = at.as_secs_f64();
        for (hz, amp, pan) in partials(t) {
            if amp <= 0.0 {
                continue;
            }
            let k = band_of(hz);
            let (gl, gr) = ((1.0 - pan) / 2.0, (1.0 + pan) / 2.0);
            // The partial's energy and a little spill into the neighbours,
            // plus a hint of harmonics.
            for (d, w) in [(0i32, 1.0), (-1, 0.45), (1, 0.45), (-2, 0.15), (2, 0.15)] {
                let i = k as i32 + d;
                if (0..BANDS as i32).contains(&i) {
                    let v = (amp * w) as f32;
                    l[i as usize] = l[i as usize].max(v * gl.sqrt() as f32);
                    r[i as usize] = r[i as usize].max(v * gr.sqrt() as f32);
                }
            }
            for h in 2..=4 {
                let kh = band_of(hz * f64::from(h));
                let v = (amp / f64::from(h) / 1.5) as f32;
                l[kh] = l[kh].max(v * gl.sqrt() as f32);
                r[kh] = r[kh].max(v * gr.sqrt() as f32);
            }
        }
        // A low noise floor with seeded jitter, so the bars never sit still.
        for i in 0..BANDS {
            let n = (self.rng.f64() * 0.04) as f32;
            l[i] = (l[i] + n).min(1.0);
            r[i] = (r[i] + n).min(1.0);
        }
        (l, r)
    }

    /// The scope: the summed waveform over the last 512 samples at 48 kHz.
    pub fn scope_at(at: Ts) -> [f32; SCOPE_LEN] {
        let mut out = [0f32; SCOPE_LEN];
        if Self::silent_at(at) {
            return out;
        }
        let t0 = at.as_secs_f64();
        let ps = partials(t0);
        for (i, o) in out.iter_mut().enumerate() {
            let t = t0 + (i as f64 - SCOPE_LEN as f64) / SAMPLE_RATE;
            let mut v = 0.0;
            for (hz, amp, _) in ps {
                v += amp * (t * hz * std::f64::consts::TAU).sin();
            }
            *o = (v * 0.5).clamp(-1.0, 1.0) as f32;
        }
        out
    }

    pub fn tick_at(&mut self, at: Ts) -> Batch {
        let (l, r) = self.bands_at(at);
        let mut samples = Vec::with_capacity(12);
        let vec = |a: &[f32]| -> Vec32 { Arc::from(a) };
        samples.push(Sample {
            id: audio::BANDS_KEY.idx(0).id,
            datum: Datum::Vector(vec(&l)),
        });
        samples.push(Sample {
            id: audio::BANDS_KEY.idx(1).id,
            datum: Datum::Vector(vec(&r)),
        });
        samples.push(Sample {
            id: audio::SCOPE.id.clone(),
            datum: Datum::Vector(vec(&Self::scope_at(at))),
        });
        let silent = Self::silent_at(at);
        let level = |bands: &[f32; BANDS]| -> (f64, f64) {
            if silent {
                return (audio::FLOOR_DB, audio::FLOOR_DB);
            }
            let peak = bands.iter().cloned().fold(0f32, f32::max) as f64;
            let rms =
                (bands.iter().map(|v| (*v as f64).powi(2)).sum::<f64>() / BANDS as f64).sqrt();
            let db = |v: f64| (20.0 * v.max(1e-5).log10()).max(audio::FLOOR_DB);
            (db(rms * 2.5), db(peak))
        };
        for (ch, b) in [(0u16, &l), (1u16, &r)] {
            let (rms, peak) = level(b);
            samples.push(Sample {
                id: audio::RMS_DB.idx(ch).id,
                datum: Datum::Scalar(rms),
            });
            samples.push(Sample {
                id: audio::PEAK_DB.idx(ch).id,
                datum: Datum::Scalar(peak),
            });
        }
        samples.push(Sample {
            id: audio::DSP_MS.id.clone(),
            datum: Datum::Scalar(0.3 + self.rng.f64() * 0.1),
        });
        if !self.sink_sent {
            self.sink_sent = true;
            samples.push(Sample {
                id: audio::SINK.id.clone(),
                datum: Datum::Record(Arc::new(audio_sink())),
            });
        }
        if self.was_silent != Some(silent) {
            self.was_silent = Some(silent);
            samples.push(Sample {
                id: audio::LEVEL.id.clone(),
                datum: Datum::Record(Arc::new(AudioLevel { silent, since: at })),
            });
        }
        Batch {
            source: audio::SOURCE,
            at,
            samples,
        }
    }
}

/// The audio source's static info (§5 cadence row): data-driven at the
/// visualizer's fps while visible; nothing while hidden (the child is killed
/// after 10 s).
pub fn audio_info() -> SourceInfo {
    SourceInfo {
        id: audio::SOURCE,
        produces: &["audio.*"],
        cadence: Cadence {
            hidden: None,
            visible: Duration::from_millis(33),
            focused: Duration::from_millis(33),
            always_on: false,
        },
        requires: &[],
    }
}

struct AudioDemoSource {
    seed: u64,
}

impl Source for AudioDemoSource {
    fn info(&self) -> SourceInfo {
        audio_info()
    }

    fn run(self: Box<Self>, cx: SourceCtx) {
        let mut synth = AudioSynth::new(self.seed);
        let fps = cx
            .options
            .get("fps")
            .and_then(|v| v.as_integer())
            .map(|f| f.clamp(5, 60) as u64)
            .unwrap_or(30);
        let period = Duration::from_millis(1000 / fps);
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
            let level = cx.demand.level();
            let Some(mut cadence) = self.info().cadence.for_level(level) else {
                // Hidden: nothing to publish; park.
                if !cx.sleep_until(cx.next_deadline(Duration::from_secs(1))) {
                    return;
                }
                continue;
            };
            cadence = cadence.max(period);
            let at = cx.clock.now();
            // Silent: 2 Hz, like the live source's silence rule.
            if AudioSynth::silent_at(at) {
                cadence = cadence.max(Duration::from_millis(500));
            }
            if !cx.sleep_until(cx.next_deadline(cadence)) {
                return;
            }
            let at = cx.clock.now();
            let b = synth.tick_at(at);
            cx.emit(at, b.samples);
        }
    }
}

/// The seeded demo source for `--demo` and `SourceDef.demo` (§4.3).
pub fn audio_demo(seed: u64) -> Box<dyn Source> {
    Box::new(AudioDemoSource { seed })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bands_are_deterministic_silent_first_and_in_range() {
        let (mut a, mut b) = (AudioSynth::new(7), AudioSynth::new(7));
        for i in 0..40 {
            let at = Ts(i * 250_000_000);
            let x = a.tick_at(at);
            let y = b.tick_at(at);
            assert_eq!(x.samples.len(), y.samples.len());
            for (p, q) in x.samples.iter().zip(&y.samples) {
                assert_eq!(p.id, q.id);
                match (&p.datum, &q.datum) {
                    (Datum::Vector(u), Datum::Vector(v)) => {
                        assert_eq!(u, v);
                        assert!(u.iter().all(|s| (-1.0..=1.0).contains(s)));
                    }
                    (Datum::Scalar(u), Datum::Scalar(v)) => assert_eq!(u, v),
                    _ => {}
                }
            }
        }
        let mut s = AudioSynth::new(1);
        let (l, _) = s.bands_at(Ts(500_000_000));
        assert!(l.iter().all(|v| *v == 0.0), "silent at 0.5 s");
        let (l, _) = s.bands_at(Ts(5_000_000_000));
        assert!(l.iter().any(|v| *v > 0.3), "lit at 5 s: {l:?}");
        assert_eq!(band_of(LO_HZ), 0);
        assert_eq!(band_of(HI_HZ), BANDS - 1);
        assert!(band_of(1_000.0) > band_of(100.0));
    }
}
