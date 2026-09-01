//! Grid geometry (§6).

use crate::component::Size;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BorderMode {
    Each,
    Shared,
    None,
}

#[derive(Clone, Copy, Debug)]
pub struct GridSpec {
    pub columns: u8,
    pub rows: u8,
    pub gap: u8,
    pub borders: BorderMode,
    pub cell_aspect: f32,
    pub min_unit_inner: Size,
}

impl Default for GridSpec {
    fn default() -> GridSpec {
        GridSpec {
            columns: 12,
            rows: 6,
            gap: 1,
            borders: BorderMode::Each,
            cell_aspect: 0.5,
            min_unit_inner: Size::new(8, 3),
        }
    }
}

/// Split `len` into `n` tracks with `gap` between them: `(start, size)` per
/// track, sizes differing by ≤ 1, exact sum (§6, oracle-tested against
/// ratatui's `Layout` in the crate tests).
pub fn tracks(len: u16, n: u8, gap: u16) -> Vec<(u16, u16)> {
    let n16 = u16::from(n);
    if n == 0 || len == 0 {
        return Vec::new();
    }
    let gaps = gap * (n16 - 1);
    let usable = len.saturating_sub(gaps);
    let base = usable / n16;
    let extra = usable % n16;
    let mut out = Vec::with_capacity(n as usize);
    let mut x = 0u16;
    for i in 0..n16 {
        let w = base + u16::from(i < extra);
        out.push((x, w));
        x += w + gap;
    }
    out
}

/// Minimum terminal sizes for the configured and dense modes (§6): derived
/// from the spec, never constants. Chrome = tab bar + status bar rows.
pub fn thresholds(spec: &GridSpec, chrome_rows: u16) -> (Size, Size) {
    let c = u16::from(spec.columns);
    let r = u16::from(spec.rows);
    let g = u16::from(spec.gap);
    let miw = spec.min_unit_inner.w;
    let mih = spec.min_unit_inner.h;
    let configured = Size::new(
        c * (miw + 2) + (c - 1) * g,
        r * (mih + 2) + (r - 1) * g + chrome_rows,
    );
    let dense = Size::new(c * (miw + 1) + 1, r * (mih + 1) + 1 + chrome_rows);
    (configured, dense)
}
