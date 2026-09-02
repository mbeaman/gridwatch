//! Bar ballistics (brief arc 5 seam 4, digest §3): the source publishes
//! instantaneous heights; how they move is the component's. Two presets —
//! Winamp (instant rise, a fixed fall per frame, accelerating peak caps) and
//! cava (gravity fall from the last peak, an integral smoother, the
//! monstercat neighbour filter). Frame-rate normalised: every step takes
//! `dt` and scales to the 30 fps reference, so 60 fps looks the same.
//! Pure over its inputs; unit-tested.

use gridwatch_store::keys::audio::FLOOR_DB;
use serde::{Deserialize, Serialize};

/// The frame the constants are written for.
pub const REF_FPS: f64 = 30.0;

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Preset {
    #[default]
    Winamp,
    Cava,
}

impl Preset {
    pub fn next(self) -> Preset {
        match self {
            Preset::Winamp => Preset::Cava,
            Preset::Cava => Preset::Winamp,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Preset::Winamp => "winamp",
            Preset::Cava => "cava",
        }
    }
}

/// Winamp's falloff (of the 3/6/12/16/32 set): sixteenths of full height
/// per frame.
pub const WINAMP_FALLOFF: f64 = 12.0;
/// Peak caps: the first step in height per frame, then ×1.1 per frame.
pub const WINAMP_PEAK_V0: f64 = 0.003;
pub const WINAMP_PEAK_ACCEL: f64 = 1.1;
/// Frames a cap holds before it starts falling.
pub const WINAMP_PEAK_HOLD_FRAMES: f64 = 12.0;

pub const CAVA_GRAVITY: f64 = 1.0;
pub const CAVA_FALL_STEP: f64 = 0.028;
pub const CAVA_NOISE_REDUCTION: f64 = 0.77;
pub const CAVA_MONSTERCAT: f64 = 1.5;

/// One bar's moving state.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Bar {
    /// The displayed height.
    pub out: f64,
    /// The peak cap's height.
    pub peak: f64,
    /// Winamp: the cap's velocity; cava: the gravity `fall` counter.
    v: f64,
    /// Winamp: frames the cap has held.
    hold: f64,
    /// cava: the height the fall started from, and the smoothed input.
    from: f64,
    smooth: f64,
}

/// The ballistics over a set of bars.
#[derive(Clone, Debug)]
pub struct Bars {
    pub preset: Preset,
    pub bars: Vec<Bar>,
}

impl Bars {
    pub fn new(preset: Preset, n: usize) -> Bars {
        Bars {
            preset,
            bars: vec![Bar::default(); n],
        }
    }

    pub fn set_preset(&mut self, p: Preset) {
        if p != self.preset {
            self.preset = p;
            for b in &mut self.bars {
                *b = Bar {
                    out: b.out,
                    peak: b.peak,
                    ..Bar::default()
                };
            }
        }
    }

    /// Feed the instantaneous heights after `dt` seconds. `input.len()`
    /// must match; a mismatch resizes (a resample changed the bar count).
    pub fn step(&mut self, input: &[f32], dt: f64) {
        if input.len() != self.bars.len() {
            self.bars = vec![Bar::default(); input.len()];
        }
        let frames = (dt * REF_FPS).clamp(0.0, 8.0);
        match self.preset {
            Preset::Winamp => {
                for (b, x) in self.bars.iter_mut().zip(input) {
                    let x = f64::from(*x).clamp(0.0, 1.0);
                    if x >= b.out {
                        b.out = x;
                    } else {
                        b.out = (b.out - WINAMP_FALLOFF / 16.0 * frames).max(x);
                    }
                    if b.out >= b.peak {
                        b.peak = b.out;
                        b.v = WINAMP_PEAK_V0;
                        b.hold = 0.0;
                    } else if b.hold < WINAMP_PEAK_HOLD_FRAMES {
                        b.hold += frames;
                    } else {
                        b.peak = (b.peak - b.v * frames).max(b.out);
                        b.v *= WINAMP_PEAK_ACCEL.powf(frames);
                    }
                }
            }
            Preset::Cava => {
                // The integral smoother on the input, then monstercat.
                let a = CAVA_NOISE_REDUCTION.powf(frames);
                let mut sm: Vec<f64> = self
                    .bars
                    .iter()
                    .zip(input)
                    .map(|(b, x)| {
                        let x = f64::from(*x).clamp(0.0, 1.0);
                        b.smooth * a + x * (1.0 - a)
                    })
                    .collect();
                for b in self.bars.iter_mut().zip(&sm) {
                    b.0.smooth = *b.1;
                }
                let raw = sm.clone();
                for (i, s) in sm.iter_mut().enumerate() {
                    for (j, r) in raw.iter().enumerate() {
                        if i != j {
                            let d = i.abs_diff(j) as f64;
                            *s = s.max(r / CAVA_MONSTERCAT.powf(d));
                        }
                    }
                }
                for (b, x) in self.bars.iter_mut().zip(&sm) {
                    if *x >= b.out {
                        b.out = *x;
                        b.v = 0.0;
                        b.from = *x;
                    } else {
                        b.v += CAVA_FALL_STEP * frames;
                        b.out = (b.from * (1.0 - b.v * b.v * CAVA_GRAVITY)).max(*x);
                    }
                    b.peak = b.out;
                }
            }
        }
    }

    pub fn heights(&self) -> Vec<f32> {
        self.bars.iter().map(|b| b.out as f32).collect()
    }

    pub fn peaks(&self) -> Vec<f32> {
        self.bars.iter().map(|b| b.peak as f32).collect()
    }

    /// Anything still above zero (the tile keeps animating while decaying).
    pub fn moving(&self) -> bool {
        self.bars.iter().any(|b| b.out > 0.001 || b.peak > 0.001)
    }
}

/// The VU's display range: −60 dBFS at the left, 0 at the right.
pub const VU_MIN_DB: f64 = -60.0;
/// The VU falls this fast (dB per second) — the digest's meter ballistics.
pub const VU_FALL_DB_PER_S: f64 = 20.0;

/// One VU channel: instant rise, a linear dB fall, and the source's held peak.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Vu {
    pub rms_db: f64,
    pub peak_db: f64,
}

impl Default for Vu {
    fn default() -> Vu {
        Vu {
            rms_db: FLOOR_DB,
            peak_db: FLOOR_DB,
        }
    }
}

impl Vu {
    pub fn step(&mut self, rms_db: f64, peak_db: f64, dt: f64) {
        if rms_db >= self.rms_db {
            self.rms_db = rms_db;
        } else {
            self.rms_db = (self.rms_db - VU_FALL_DB_PER_S * dt).max(rms_db);
        }
        self.peak_db = peak_db;
    }

    /// Height fraction of the RMS in the VU range.
    pub fn level(&self) -> f32 {
        db_frac(self.rms_db)
    }

    pub fn peak(&self) -> f32 {
        db_frac(self.peak_db)
    }

    pub fn moving(&self) -> bool {
        self.rms_db > VU_MIN_DB
    }
}

pub fn db_frac(db: f64) -> f32 {
    ((db - VU_MIN_DB) / (0.0 - VU_MIN_DB)).clamp(0.0, 1.0) as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn winamp_rises_at_once_and_falls_twelve_sixteenths_per_frame() {
        let mut b = Bars::new(Preset::Winamp, 1);
        b.step(&[1.0], 1.0 / 30.0);
        assert_eq!(b.heights(), [1.0]);
        b.step(&[0.0], 1.0 / 30.0);
        assert!((b.heights()[0] - 0.25).abs() < 1e-6, "{:?}", b.heights());
        b.step(&[0.0], 1.0 / 30.0);
        assert_eq!(b.heights(), [0.0]);
        // 60 fps: two half-frames equal one frame.
        let mut c = Bars::new(Preset::Winamp, 1);
        c.step(&[1.0], 1.0 / 60.0);
        c.step(&[0.0], 1.0 / 60.0);
        c.step(&[0.0], 1.0 / 60.0);
        assert!((c.heights()[0] - 0.25).abs() < 1e-6, "{:?}", c.heights());
    }

    #[test]
    fn winamp_peaks_hold_then_accelerate_down() {
        let mut b = Bars::new(Preset::Winamp, 1);
        b.step(&[0.8], 1.0 / 30.0);
        assert_eq!(b.peaks(), [0.8]);
        for _ in 0..12 {
            b.step(&[0.0], 1.0 / 30.0);
        }
        assert!((b.peaks()[0] - 0.8).abs() < 1e-9, "held: {:?}", b.peaks());
        b.step(&[0.0], 1.0 / 30.0);
        let p1 = f64::from(b.peaks()[0]);
        assert!((p1 - (0.8 - WINAMP_PEAK_V0)).abs() < 1e-6, "{p1}");
        b.step(&[0.0], 1.0 / 30.0);
        let p2 = f64::from(b.peaks()[0]);
        assert!(
            (p1 - p2 - WINAMP_PEAK_V0 * 1.1).abs() < 1e-6,
            "accelerates: {p2}"
        );
        for _ in 0..200 {
            b.step(&[0.0], 1.0 / 30.0);
        }
        assert_eq!(b.peaks(), [0.0]);
        assert!(!b.moving());
        b.step(&[0.5], 1.0 / 30.0);
        assert_eq!(b.peaks(), [0.5], "a new maximum re-arms the cap");
    }

    #[test]
    fn cava_gravity_integral_and_monstercat() {
        let mut b = Bars::new(Preset::Cava, 3);
        // The integral: 23 % of the way per frame at 30 fps.
        b.step(&[1.0, 0.0, 0.0], 1.0 / 30.0);
        let h = b.heights();
        assert!(
            (f64::from(h[0]) - (1.0 - CAVA_NOISE_REDUCTION)).abs() < 1e-6,
            "{h:?}"
        );
        // Monstercat: the neighbours follow at /1.5 and /2.25.
        assert!((h[1] - h[0] / 1.5).abs() < 1e-6, "{h:?}");
        assert!((h[2] - h[0] / 2.25).abs() < 1e-6, "{h:?}");
        // Settle, then drop: the fall is quadratic from the last height.
        for _ in 0..200 {
            b.step(&[1.0, 0.0, 0.0], 1.0 / 30.0);
        }
        let top = b.heights()[0];
        assert!(top > 0.99, "{top}");
        b.step(&[0.0; 3], 1.0 / 30.0);
        let s1 = b.heights()[0];
        b.step(&[0.0; 3], 1.0 / 30.0);
        let s2 = b.heights()[0];
        b.step(&[0.0; 3], 1.0 / 30.0);
        let s3 = b.heights()[0];
        assert!(
            top - s1 < s1 - s2 && s1 - s2 < s2 - s3,
            "accelerating: {top} {s1} {s2} {s3}"
        );
        for _ in 0..200 {
            b.step(&[0.0; 3], 1.0 / 30.0);
        }
        assert!(b.heights().iter().all(|h| *h < 1e-3), "{:?}", b.heights());
        assert!(!b.moving());
        // 60 fps matches 30 fps after the same wall time.
        let mut a = Bars::new(Preset::Cava, 1);
        let mut c = Bars::new(Preset::Cava, 1);
        for _ in 0..30 {
            a.step(&[1.0], 1.0 / 30.0);
        }
        for _ in 0..60 {
            c.step(&[1.0], 1.0 / 60.0);
        }
        assert!((a.heights()[0] - c.heights()[0]).abs() < 1e-3);
    }

    #[test]
    fn vu_falls_twenty_db_per_second() {
        let mut v = Vu::default();
        v.step(-6.0, -3.0, 0.033);
        assert_eq!(v.rms_db, -6.0);
        v.step(-60.0, -60.0, 0.5);
        assert!((v.rms_db + 16.0).abs() < 1e-9, "{}", v.rms_db);
        assert!((f64::from(v.level()) - 44.0 / 60.0).abs() < 1e-6);
        v.step(-60.0, -60.0, 10.0);
        assert!(!v.moving());
        assert_eq!(db_frac(0.0), 1.0);
        assert_eq!(db_frac(-100.0), 0.0);
    }

    #[test]
    fn a_resample_resizes_and_a_preset_swap_keeps_heights() {
        let mut b = Bars::new(Preset::Winamp, 4);
        b.step(&[0.5, 0.5], 0.033);
        assert_eq!(b.bars.len(), 2);
        b.set_preset(Preset::Cava);
        assert_eq!(b.heights(), [0.5, 0.5]);
        assert_eq!(Preset::Winamp.next(), Preset::Cava);
        assert_eq!(Preset::Cava.next().name(), "winamp");
    }
}
