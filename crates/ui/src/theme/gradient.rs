//! Oklab-interpolated 64-entry gradient LUTs (§7), pre-downsampled per mode.

use palette::{IntoColor, Mix, Oklab, Srgb, convert::FromColorUnclamped};
use ratatui_core::style::Color;

use super::color::{ColorMode, to_mode};

pub const LUT: usize = 64;

#[derive(Clone, Debug)]
pub struct Gradient {
    lut: [Color; LUT],
}

impl Gradient {
    pub fn from_stops(stops: &[Color], mode: ColorMode) -> Gradient {
        let rgb: Vec<Oklab> = stops
            .iter()
            .map(|c| {
                let (r, g, b) = match c {
                    Color::Rgb(r, g, b) => (*r, *g, *b),
                    _ => (128, 128, 128),
                };
                let s = Srgb::new(
                    f32::from(r) / 255.0,
                    f32::from(g) / 255.0,
                    f32::from(b) / 255.0,
                );
                Oklab::from_color_unclamped(s.into_linear())
            })
            .collect();
        let mut lut = [Color::Reset; LUT];
        if rgb.is_empty() || !stops.iter().all(|c| matches!(c, Color::Rgb(..))) {
            return Gradient { lut }; // mono / "default" stops: no colour at all
        }
        for (i, slot) in lut.iter_mut().enumerate() {
            let t = i as f32 / (LUT - 1) as f32;
            let seg = t * (rgb.len().saturating_sub(1)) as f32;
            let lo = (seg.floor() as usize).min(rgb.len() - 1);
            let hi = (lo + 1).min(rgb.len() - 1);
            let f = seg - seg.floor();
            let mixed = rgb[lo].mix(rgb[hi], f);
            let out: Srgb = palette::LinSrgb::from_color_unclamped(mixed).into_color();
            let c = Color::Rgb(
                (out.red.clamp(0.0, 1.0) * 255.0) as u8,
                (out.green.clamp(0.0, 1.0) * 255.0) as u8,
                (out.blue.clamp(0.0, 1.0) * 255.0) as u8,
            );
            *slot = to_mode(c, mode);
        }
        Gradient { lut }
    }

    /// Sample at `t` in [0, 1].
    pub fn sample(&self, t: f32) -> Color {
        let i = ((t.clamp(0.0, 1.0) * (LUT - 1) as f32).round() as usize).min(LUT - 1);
        self.lut[i]
    }

    pub fn stops8(&self) -> [Color; 8] {
        std::array::from_fn(|i| self.sample(i as f32 / 7.0))
    }
}
