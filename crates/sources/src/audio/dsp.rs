//! The spectrum analyser (brief arc 5 seam 2, digest §2): cava's dual FFT —
//! Hann-windowed `realfft` at `fft` for bands from 250 Hz up and at
//! `fft_bass` below — amplitude `|X|·2/Σw` in dBFS, 64 log-spaced bands with
//! per-band max (a band narrower than a bin interpolates between its
//! neighbours), a tilt above 1 kHz, and heights against the floor. Pure and
//! allocation-free per pass: every scratch buffer is made once.
//! Ballistics are the component's; this publishes instantaneous heights.

use std::sync::Arc;

use gridwatch_store::keys::audio::{BANDS, FLOOR_DB};
use realfft::num_complex::Complex;
use realfft::{RealFftPlanner, RealToComplex};

/// The split between the two FFTs (cava: bass gets the long window).
pub const BASS_SPLIT_HZ: f64 = 250.0;
/// RMS window and the peak-hold schedule (digest §3 "VU").
pub const RMS_WINDOW_S: f64 = 0.3;
pub const PEAK_HOLD_S: f64 = 1.5;
pub const PEAK_DECAY_DB_PER_S: f64 = 20.0;

#[derive(Clone, Debug, PartialEq)]
pub struct DspConfig {
    pub rate: u32,
    pub fft: usize,
    pub fft_bass: usize,
    pub lo_hz: f64,
    pub hi_hz: f64,
    pub floor_db: f64,
    pub tilt_db_oct: f64,
}

impl Default for DspConfig {
    fn default() -> DspConfig {
        DspConfig {
            rate: 48_000,
            fft: 2048,
            fft_bass: 8192,
            lo_hz: 30.0,
            hi_hz: 16_000.0,
            floor_db: -65.0,
            tilt_db_oct: 4.0,
        }
    }
}

impl DspConfig {
    /// Clamp into the ranges the brief allows; powers of two for the FFTs.
    pub fn normalised(mut self) -> DspConfig {
        self.fft = self.fft.clamp(256, 16_384).next_power_of_two();
        self.fft_bass = self.fft_bass.clamp(self.fft, 32_768).next_power_of_two();
        self.lo_hz = self.lo_hz.clamp(10.0, 1_000.0);
        self.hi_hz = self
            .hi_hz
            .clamp(self.lo_hz * 4.0, f64::from(self.rate) / 2.0);
        self.floor_db = self.floor_db.clamp(-100.0, -10.0);
        self.tilt_db_oct = self.tilt_db_oct.clamp(0.0, 12.0);
        self
    }

    /// Band edge `k` in Hz: `lo·(hi/lo)^(k/64)`.
    pub fn edge(&self, k: usize) -> f64 {
        self.lo_hz * (self.hi_hz / self.lo_hz).powf(k as f64 / BANDS as f64)
    }

    /// The geometric centre of band `k`.
    pub fn centre(&self, k: usize) -> f64 {
        (self.edge(k) * self.edge(k + 1)).sqrt()
    }
}

struct Stage {
    fft: Arc<dyn RealToComplex<f32>>,
    window: Vec<f32>,
    /// `2/Σw`: the amplitude scale for a windowed sine.
    scale: f32,
    input: Vec<f32>,
    output: Vec<Complex<f32>>,
    /// Bin amplitudes (linear) from the last pass.
    amp: Vec<f32>,
    bin_hz: f64,
}

impl Stage {
    fn new(planner: &mut RealFftPlanner<f32>, n: usize, rate: u32) -> Stage {
        let fft = planner.plan_fft_forward(n);
        let window: Vec<f32> = (0..n)
            .map(|i| {
                let x = i as f64 / n as f64;
                (0.5 - 0.5 * (std::f64::consts::TAU * x).cos()) as f32
            })
            .collect();
        let sum: f32 = window.iter().sum();
        Stage {
            input: fft.make_input_vec(),
            output: fft.make_output_vec(),
            amp: vec![0.0; n / 2 + 1],
            fft,
            window,
            scale: 2.0 / sum,
            bin_hz: f64::from(rate) / n as f64,
        }
    }

    fn n(&self) -> usize {
        self.window.len()
    }

    /// Window the latest `n` samples of `history` and transform.
    fn run(&mut self, history: &[f32]) {
        let n = self.n();
        let tail = &history[history.len() - n..];
        for ((o, s), w) in self.input.iter_mut().zip(tail).zip(&self.window) {
            *o = s * w;
        }
        // `process` only fails on a length mismatch, which the scratch
        // buffers rule out.
        if self.fft.process(&mut self.input, &mut self.output).is_err() {
            self.amp.iter_mut().for_each(|a| *a = 0.0);
            return;
        }
        for (a, c) in self.amp.iter_mut().zip(&self.output) {
            *a = c.norm() * self.scale;
        }
    }

    /// The amplitude at `hz`, interpolated between the two bins around it.
    fn at(&self, hz: f64) -> f32 {
        let x = hz / self.bin_hz;
        let i = x.floor() as usize;
        if i + 1 >= self.amp.len() {
            return *self.amp.last().unwrap_or(&0.0);
        }
        let t = (x - i as f64) as f32;
        self.amp[i] * (1.0 - t) + self.amp[i + 1] * t
    }

    /// The largest amplitude of the bins whose centre lies in `[lo, hi)`;
    /// `None` when no bin does (the band is narrower than a bin).
    fn max_in(&self, lo: f64, hi: f64) -> Option<f32> {
        let first = (lo / self.bin_hz).ceil() as usize;
        let last = (hi / self.bin_hz).ceil() as usize; // exclusive
        if last <= first || first >= self.amp.len() {
            return None;
        }
        self.amp[first..last.min(self.amp.len())]
            .iter()
            .cloned()
            .reduce(f32::max)
    }
}

/// Per-band precomputation: which stage, its edges, the tilt in dB.
#[derive(Clone, Copy, Debug)]
struct Band {
    bass: bool,
    lo: f64,
    hi: f64,
    tilt_db: f32,
}

pub struct Dsp {
    cfg: DspConfig,
    main: Stage,
    bass: Stage,
    bands: [Band; BANDS],
}

impl Dsp {
    pub fn new(cfg: DspConfig) -> Dsp {
        let cfg = cfg.normalised();
        let mut planner = RealFftPlanner::<f32>::new();
        let main = Stage::new(&mut planner, cfg.fft, cfg.rate);
        let bass = Stage::new(&mut planner, cfg.fft_bass, cfg.rate);
        let bands = std::array::from_fn(|k| {
            let (lo, hi) = (cfg.edge(k), cfg.edge(k + 1));
            let centre = cfg.centre(k);
            let tilt_db = if centre > 1_000.0 {
                (cfg.tilt_db_oct * (centre / 1_000.0).log2()) as f32
            } else {
                0.0
            };
            Band {
                bass: lo < BASS_SPLIT_HZ,
                lo,
                hi,
                tilt_db,
            }
        });
        Dsp {
            cfg,
            main,
            bass,
            bands,
        }
    }

    pub fn config(&self) -> &DspConfig {
        &self.cfg
    }

    /// Samples of history a pass needs (the long window).
    pub fn history_len(&self) -> usize {
        self.cfg.fft_bass
    }

    /// One pass over a channel's history (its latest samples at the end;
    /// shorter than `history_len()` is zero-padded at the front): the 64
    /// band heights in 0..1.
    pub fn bands(&mut self, history: &[f32]) -> [f32; BANDS] {
        let need = self.history_len();
        let mut out = [0f32; BANDS];
        if history.len() < need {
            // Cold start: pad rather than allocate per frame later — a
            // one-off Vec until the ring has filled.
            let mut padded = vec![0f32; need];
            let n = history.len();
            padded[need - n..].copy_from_slice(history);
            return self.bands(&padded);
        }
        self.main.run(history);
        self.bass.run(history);
        let floor = self.cfg.floor_db as f32;
        for (k, b) in self.bands.iter().enumerate() {
            let stage = if b.bass { &self.bass } else { &self.main };
            let amp = stage
                .max_in(b.lo, b.hi)
                .unwrap_or_else(|| stage.at((b.lo * b.hi).sqrt()));
            let db = 20.0 * amp.max(1e-12).log10() + b.tilt_db;
            out[k] = ((db - floor) / (0.0 - floor)).clamp(0.0, 1.0);
        }
        out
    }
}

/// RMS of the latest `window_s` of `history`, in dBFS (clamped at the floor).
pub fn rms_db(history: &[f32], rate: u32, window_s: f64) -> f64 {
    let n = ((window_s * f64::from(rate)) as usize)
        .max(1)
        .min(history.len());
    if n == 0 {
        return FLOOR_DB;
    }
    let tail = &history[history.len() - n..];
    let ms = tail.iter().map(|s| f64::from(*s).powi(2)).sum::<f64>() / n as f64;
    (10.0 * ms.max(1e-20).log10()).max(FLOOR_DB)
}

/// The sample peak of `samples` in dBFS.
pub fn peak_db(samples: &[f32]) -> f64 {
    let p = samples.iter().fold(0f32, |m, s| m.max(s.abs()));
    (20.0 * f64::from(p).max(1e-10).log10()).max(FLOOR_DB)
}

/// A peak meter with a hold and a linear dB decay (digest §3).
#[derive(Clone, Debug)]
pub struct PeakHold {
    pub value_db: f64,
    held_since_s: f64,
}

impl Default for PeakHold {
    fn default() -> PeakHold {
        PeakHold {
            value_db: FLOOR_DB,
            held_since_s: 0.0,
        }
    }
}

impl PeakHold {
    /// Feed the new instantaneous peak at `now_s`; returns the displayed peak.
    pub fn feed(&mut self, peak_db: f64, now_s: f64) -> f64 {
        if peak_db >= self.value_db {
            self.value_db = peak_db;
            self.held_since_s = now_s;
        } else {
            let over = now_s - self.held_since_s - PEAK_HOLD_S;
            if over > 0.0 {
                self.value_db = (self.value_db - over * PEAK_DECAY_DB_PER_S).max(peak_db);
                // Keep decaying from here rather than restarting the hold.
                self.held_since_s = now_s - PEAK_HOLD_S;
            }
        }
        self.value_db.max(FLOOR_DB)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(hz: f64, amp: f64, n: usize, rate: u32) -> Vec<f32> {
        (0..n)
            .map(|i| (amp * (std::f64::consts::TAU * hz * i as f64 / f64::from(rate)).sin()) as f32)
            .collect()
    }

    fn band_for(cfg: &DspConfig, hz: f64) -> usize {
        (0..BANDS)
            .find(|k| cfg.edge(k + 1) > hz)
            .unwrap_or(BANDS - 1)
    }

    #[test]
    fn full_scale_1khz_sine_lights_its_band_and_reads_minus_3_db_rms() {
        let mut dsp = Dsp::new(DspConfig::default());
        let x = sine(1_000.0, 1.0, dsp.history_len(), 48_000);
        let bands = dsp.bands(&x);
        let k = band_for(dsp.config(), 1_000.0);
        assert!(
            bands[k] >= 0.97,
            "1 kHz band {k} at {} (scalloping ≤ 1.42 dB)",
            bands[k]
        );
        let (argmax, _) = bands
            .iter()
            .enumerate()
            .fold((0, -1.0), |m, (i, v)| if *v > m.1 { (i, *v) } else { m });
        assert!(argmax.abs_diff(k) <= 1, "argmax {argmax} vs {k}: {bands:?}");
        let rms = rms_db(&x, 48_000, RMS_WINDOW_S);
        assert!((rms + 3.01).abs() < 0.05, "rms {rms}");
        assert!((peak_db(&x) - 0.0).abs() < 0.01);
    }

    #[test]
    fn a_50hz_sine_has_exactly_one_dominant_band_below_100hz() {
        let mut dsp = Dsp::new(DspConfig::default());
        // Bin 9 of the 8192-point bass FFT: 52.7 Hz, bin-centred so the
        // Hann main lobe stays inside one 5 Hz band.
        let hz = 9.0 * 48_000.0 / 8192.0;
        let x = sine(hz, 0.5, dsp.history_len(), 48_000);
        let bands = dsp.bands(&x);
        let cfg = dsp.config().clone();
        let low: Vec<(usize, f32)> = (0..BANDS)
            .filter(|k| cfg.centre(*k) < 100.0)
            .map(|k| (k, bands[k]))
            .collect();
        let (best, top) = low
            .iter()
            .cloned()
            .fold((0, -1.0), |m, (i, v)| if v > m.1 { (i, v) } else { m });
        let c = cfg.centre(best);
        assert!(
            (44.0..=56.0).contains(&c),
            "dominant band centred at {c} Hz: {low:?}"
        );
        let others = low
            .iter()
            .filter(|(k, v)| *k != best && *v > top - 0.05)
            .count();
        assert_eq!(others, 0, "one dominant band: {low:?}");
        // Nothing lit above 1 kHz.
        assert!(
            (0..BANDS)
                .filter(|k| cfg.centre(*k) > 1_000.0)
                .all(|k| bands[k] < 0.05),
            "{bands:?}"
        );
    }

    #[test]
    fn silence_is_all_zeros_and_short_history_pads() {
        let mut dsp = Dsp::new(DspConfig::default());
        let z = vec![0f32; dsp.history_len()];
        assert!(dsp.bands(&z).iter().all(|v| *v == 0.0));
        assert_eq!(rms_db(&z, 48_000, RMS_WINDOW_S), FLOOR_DB);
        assert_eq!(peak_db(&z), FLOOR_DB);
        assert_eq!(rms_db(&[], 48_000, RMS_WINDOW_S), FLOOR_DB);
        let short = sine(1_000.0, 1.0, 1024, 48_000);
        let b = dsp.bands(&short);
        assert!(b[band_for(dsp.config(), 1_000.0)] > 0.5, "{b:?}");
    }

    #[test]
    fn hann_scaling_identity_reads_one_for_a_bin_centred_sine() {
        let mut planner = RealFftPlanner::<f32>::new();
        let mut st = Stage::new(&mut planner, 2048, 48_000);
        // Bin 64 = 1500 Hz exactly.
        let x = sine(64.0 * 48_000.0 / 2048.0, 1.0, 2048, 48_000);
        st.run(&x);
        assert!((st.amp[64] - 1.0).abs() < 1e-3, "{}", st.amp[64]);
        assert!(st.amp[62] < 1e-3 && st.amp[66] < 1e-3);
        let sum: f32 = st.window.iter().sum();
        assert!((sum - 1024.0).abs() < 1e-2, "Σw = N/2");
    }

    #[test]
    fn tilt_and_edges_follow_the_config() {
        let cfg = DspConfig::default();
        assert!((cfg.edge(0) - 30.0).abs() < 1e-9);
        assert!((cfg.edge(BANDS) - 16_000.0).abs() < 1e-6);
        let mut flat = Dsp::new(DspConfig {
            tilt_db_oct: 0.0,
            ..DspConfig::default()
        });
        let mut tilted = Dsp::new(DspConfig::default());
        let x = sine(8_000.0, 0.1, flat.history_len(), 48_000);
        let k = band_for(&cfg, 8_000.0);
        let (a, b) = (flat.bands(&x)[k], tilted.bands(&x)[k]);
        // +4 dB/oct × 3 octaves = 12 dB over a 65 dB range ≈ 0.18.
        assert!((b - a - 12.0 / 65.0).abs() < 0.03, "flat {a} tilted {b}");
        let n = DspConfig {
            fft: 1000,
            fft_bass: 100,
            hi_hz: 90_000.0,
            ..DspConfig::default()
        }
        .normalised();
        assert_eq!((n.fft, n.fft_bass), (1024, 1024));
        assert_eq!(n.hi_hz, 24_000.0);
    }

    #[test]
    fn peak_hold_then_decays_at_20_db_per_second() {
        let mut p = PeakHold::default();
        assert_eq!(p.feed(-6.0, 0.0), -6.0);
        assert_eq!(p.feed(-40.0, 1.0), -6.0, "held");
        assert_eq!(p.feed(-40.0, 1.5), -6.0, "held to 1.5 s");
        let v = p.feed(-40.0, 2.5);
        assert!((v + 26.0).abs() < 1e-9, "1 s past the hold: {v}");
        let v = p.feed(-40.0, 4.0);
        assert!((v + 40.0).abs() < 1e-9, "never below the input: {v}");
        assert_eq!(p.feed(-3.0, 4.1), -3.0, "a louder peak resets");
    }
}
