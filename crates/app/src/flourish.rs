//! `[flourish]` (§7, brief arc 4 seam 7): the retro decorations a theme opts
//! into — the perspective grid floor and the sun — drawn by the shell in the
//! **empty units** of a page (units no placement covers), never over a tile.
//! String art per rect, cheap enough not to cache.

use gridwatch_ui::layout::{GridSpec, Page, SolveMode, unit_rect};
use gridwatch_ui::theme::{GradientId, Role, Theme};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;

/// Every grid unit no placement covers, in reading order.
pub fn empty_units(spec: &GridSpec, page: &Page) -> Vec<(u8, u8)> {
    let mut out = Vec::new();
    for y in 0..spec.rows {
        for x in 0..spec.columns {
            let covered = page.place.iter().any(|p| {
                x >= p.at.0 && x < p.at.0 + p.size.0 && y >= p.at.1 && y < p.at.1 + p.size.1
            });
            if !covered {
                out.push((x, y));
            }
        }
    }
    out
}

/// Maximal horizontal runs of empty units per row: one art rect per run so a
/// two-unit hole gets one sun, not two.
pub fn empty_runs(spec: &GridSpec, page: &Page) -> Vec<((u8, u8), (u8, u8))> {
    let empties = empty_units(spec, page);
    let mut runs: Vec<((u8, u8), (u8, u8))> = Vec::new();
    for (x, y) in empties {
        match runs.last_mut() {
            Some(((rx, ry), (rw, _))) if *ry == y && *rx + *rw == x => *rw += 1,
            _ => runs.push(((x, y), (1, 1))),
        }
    }
    // A hole several units tall is one rect (review: an N×2 hole drew two
    // stacked suns): a run merges onto the one directly above with the same
    // span.
    let mut merged: Vec<((u8, u8), (u8, u8))> = Vec::new();
    for ((x, y), (w, h)) in runs {
        match merged
            .iter_mut()
            .find(|((mx, my), (mw, mh))| *mx == x && *mw == w && *my + *mh == y)
        {
            Some((_, (_, mh))) => *mh += h,
            None => merged.push(((x, y), (w, h))),
        }
    }
    merged
}

/// Draw the theme's flourishes into the page's empty runs (nothing in stack
/// mode, which has no units).
pub fn draw(
    theme: &Theme,
    spec: &GridSpec,
    page: &Page,
    body: Rect,
    mode: SolveMode,
    buf: &mut Buffer,
) {
    let f = &theme.flourish;
    // Stack mode has no units; dense mode has no gutters, so the floor lines
    // would abut tile borders (review) — flourishes are a configured-mode
    // decoration.
    if (!f.grid_floor && !f.sun) || mode != SolveMode::Configured {
        return;
    }
    for (at, size) in empty_runs(spec, page) {
        let Some(r) = unit_rect(spec, body, mode, at, size) else {
            continue;
        };
        if r.width < 8 || r.height < 3 {
            continue;
        }
        if f.grid_floor {
            grid_floor(theme, r, buf);
        }
        if f.sun {
            sun(theme, r, buf);
        }
    }
}

/// Perspective floor: horizontal lines converging upward (rows at
/// `h·(1 − 1/k)` from the top for k = 2, 3, …) and verticals fanning out
/// from the vanishing point, all in `TextGhost`.
fn grid_floor(theme: &Theme, r: Rect, buf: &mut Buffer) {
    let style = theme.style(Role::TextGhost);
    let horizon = r.y + r.height / 3;
    let mut k = 2u16;
    loop {
        let dy = (u32::from(r.height - r.height / 3) * (k - 1) as u32 / k as u32) as u16;
        let y = horizon + dy;
        if y >= r.y + r.height {
            break;
        }
        for x in r.x..r.x + r.width {
            if let Some(c) = buf.cell_mut((x, y))
                && c.symbol() == " "
            {
                c.set_char('─');
                c.set_style(style);
            }
        }
        k += 1;
        if k > 12 {
            break;
        }
    }
    // Verticals: from the vanishing point (centre of the horizon row) to the
    // bottom row at evenly spaced x.
    let vx = i32::from(r.x) + i32::from(r.width) / 2;
    let bottom = i32::from(r.y + r.height - 1);
    let top = i32::from(horizon);
    let n = (r.width / 6).max(2);
    for i in 0..=n {
        let bx = i32::from(r.x) + (i32::from(r.width) - 1) * i32::from(i) / i32::from(n);
        for y in (top + 1)..=bottom {
            let t = (y - top) as f32 / (bottom - top).max(1) as f32;
            let x = vx as f32 + (bx - vx) as f32 * t;
            let x = x.round() as i32;
            if x < i32::from(r.x) || x >= i32::from(r.x + r.width) {
                continue;
            }
            if let Some(c) = buf.cell_mut((x as u16, y as u16))
                && c.symbol() == " "
            {
                c.set_char('╱');
                c.set_style(style);
            }
        }
    }
}

/// The sun: stacked `▀`/`█` rows with one-row gaps in the upper half of the
/// rect, coloured by row through the `Title` gradient.
fn sun(theme: &Theme, r: Rect, buf: &mut Buffer) {
    let g = theme.gradient(GradientId::Title);
    let rows = (r.height / 3).max(2);
    let radius = (rows * 2).min(r.width / 2).max(2);
    let cx = i32::from(r.x) + i32::from(r.width) / 2;
    let cy = i32::from(r.y) + i32::from(rows);
    for i in 0..rows {
        let y = cy - i32::from(i);
        if y < i32::from(r.y) {
            break;
        }
        // Circle: half-width at this row.
        let dy = i as f32 / rows as f32;
        let half = (radius as f32 * (1.0 - dy * dy).sqrt()) as i32;
        if half <= 0 {
            continue;
        }
        let glyph = if i % 2 == 0 { '█' } else { '▀' };
        let style = Style::new().fg(g.sample(dy));
        for x in (cx - half)..=(cx + half) {
            if x < i32::from(r.x) || x >= i32::from(r.x + r.width) {
                continue;
            }
            if let Some(c) = buf.cell_mut((x as u16, y as u16))
                && (c.symbol() == " " || c.symbol() == "─" || c.symbol() == "╱")
            {
                c.set_char(glyph);
                c.set_style(style);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gridwatch_ui::layout::{PlaceTarget, Placement};

    fn page_with_hole() -> Page {
        let p = |id: &str, at, size| Placement {
            target: PlaceTarget::Id(id.into()),
            at,
            size,
            view: None,
            priority: 0,
        };
        Page {
            name: "p".into(),
            hotkey: None,
            place: vec![p("a", (0, 0), (12, 3)), p("b", (0, 3), (8, 3))],
        }
    }

    #[test]
    fn empty_runs_are_the_uncovered_units() {
        let spec = GridSpec::default();
        let runs = empty_runs(&spec, &page_with_hole());
        assert_eq!(
            runs,
            vec![((8, 3), (4, 3))],
            "one rect, not three stacked runs"
        );
        let full = Page {
            place: vec![Placement {
                target: PlaceTarget::Id("x".into()),
                at: (0, 0),
                size: (12, 6),
                view: None,
                priority: 0,
            }],
            ..page_with_hole()
        };
        assert!(empty_runs(&spec, &full).is_empty());
    }
}
