//! The oscilloscope as a `View::Chart` (braille by default; the renderer's
//! `ChartMarker` decides octants when the VTE marker is on): the latest 512
//! mono samples in −1..1, downsampled to what the rect can show so the
//! braille grid is never asked to draw more points than it has columns.

use gridwatch_ui::theme::GradientId;
use gridwatch_ui::view::{Bounds, MarkerHint, Series, View};

/// The chart over `samples` for a rect `width` cells wide.
pub fn chart(samples: &[f32], width: u16) -> View {
    let cols = usize::from(width).max(1) * 2; // braille: two dots per cell
    let n = samples.len();
    let data: Vec<(f64, f64)> = if n == 0 {
        Vec::new()
    } else if n <= cols {
        samples
            .iter()
            .enumerate()
            .map(|(i, s)| (i as f64, f64::from(*s)))
            .collect()
    } else {
        // Min/max per column so peaks survive the downsample.
        let mut out = Vec::with_capacity(cols * 2);
        for c in 0..cols {
            let a = c * n / cols;
            let b = ((c + 1) * n / cols).max(a + 1).min(n);
            let (lo, hi) = samples[a..b]
                .iter()
                .fold((f32::MAX, f32::MIN), |(lo, hi), s| (lo.min(*s), hi.max(*s)));
            out.push((c as f64 + 0.25, f64::from(lo)));
            out.push((c as f64 + 0.75, f64::from(hi)));
        }
        out
    };
    let x_max = if n <= cols {
        (n.max(2) - 1) as f64
    } else {
        cols as f64
    };
    View::Chart {
        series: vec![Series {
            label: "scope".into(),
            gradient: GradientId::Audio,
            data,
        }],
        bounds: Bounds {
            x: (0.0, x_max),
            y: (-1.0, 1.0),
        },
        marker: MarkerHint::Braille,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn downsamples_keeping_extremes_and_handles_empty() {
        let samples: Vec<f32> = (0..512)
            .map(|i| {
                if i == 100 {
                    1.0
                } else if i == 300 {
                    -1.0
                } else {
                    0.0
                }
            })
            .collect();
        let View::Chart { series, bounds, .. } = chart(&samples, 20) else {
            panic!()
        };
        assert_eq!(series[0].data.len(), 80);
        assert!(series[0].data.iter().any(|(_, y)| *y == 1.0));
        assert!(series[0].data.iter().any(|(_, y)| *y == -1.0));
        assert_eq!(bounds.y, (-1.0, 1.0));
        let View::Chart { series, .. } = chart(&[], 20) else {
            panic!()
        };
        assert!(series[0].data.is_empty());
        let View::Chart { series, .. } = chart(&[0.5, -0.5], 20) else {
            panic!()
        };
        assert_eq!(series[0].data.len(), 2);
    }
}
