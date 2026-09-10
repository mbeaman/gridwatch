//! The default renderer (§4.6, D32): draws a `View` with the theme's widget
//! forms. Themes own form and paint; components own content.

use std::borrow::Cow;

use ratatui_core::buffer::Buffer;
use ratatui_core::layout::Rect;
use ratatui_core::style::{Modifier, Style};
use ratatui_core::text::Line as RLine;
use tui_big_text::{BigText, PixelSize};
use unicode_width::UnicodeWidthStr;

use crate::theme::{GaugeStyle, HeaderStyle, PixelStyle, Role, Theme};
use crate::view::{ColWidth, Constraint, Dir, Line, Renderer, SortDir, View};

pub struct DefaultRenderer;

pub static DEFAULT_RENDERER: DefaultRenderer = DefaultRenderer;

impl Renderer for DefaultRenderer {
    fn render(&self, view: &View, area: Rect, theme: &Theme, buf: &mut Buffer) {
        if area.width == 0 || area.height == 0 {
            return;
        }
        match view {
            View::Empty => {}
            View::Text(lines) => text(lines, area, theme, buf),
            View::KeyValue(rows) => key_value(rows, area, theme, buf),
            View::Gauge {
                label,
                value,
                gradient,
                text: t,
            } => gauge(label, *value, *gradient, t.as_deref(), area, theme, buf),
            View::Segmented {
                label,
                segments,
                text: t,
            } => segmented(label, segments, t.as_deref(), area, theme, buf),
            View::Bars {
                values,
                gradient,
                labels,
                peaks,
            } => bars(
                values,
                *gradient,
                labels.as_deref(),
                peaks.as_deref(),
                area,
                theme,
                buf,
            ),
            View::Sparkline {
                series,
                gradient,
                max,
            } => sparkline(series, *gradient, *max, area, theme, buf),
            View::Chart {
                series,
                bounds,
                marker,
            } => chart(series, bounds, *marker, area, theme, buf),
            View::Table {
                columns,
                rows,
                selected,
                sort,
                scroll,
            } => {
                table(columns, rows, *selected, *sort, *scroll, area, theme, buf);
            }
            View::BigNumber { text: t, role } => big_number(t, *role, area, theme, buf),
            View::Stack { dir, children } => stack(*dir, children, area, theme, buf),
            View::Custom { paint, .. } => paint.paint(area, theme, buf),
        }
    }
}

fn put_line(line: &Line, x: u16, y: u16, max_w: u16, theme: &Theme, buf: &mut Buffer) {
    let mut cx = x;
    let end = x + max_w;
    for span in line {
        if cx >= end {
            break;
        }
        let style = theme.span_style(span);
        let avail = (end - cx) as usize;
        buf.set_stringn(cx, y, span.text.as_ref(), avail, style);
        cx += (span.text.as_ref().width() as u16).min(end - cx);
    }
}

fn text(lines: &[Line], area: Rect, theme: &Theme, buf: &mut Buffer) {
    for (i, line) in lines.iter().enumerate() {
        if i as u16 >= area.height {
            break;
        }
        put_line(line, area.x, area.y + i as u16, area.width, theme, buf);
    }
}

fn key_value(
    rows: &[(Cow<'static, str>, Line, Option<gridwatch_store::Severity>)],
    area: Rect,
    theme: &Theme,
    buf: &mut Buffer,
) {
    let key_w = rows
        .iter()
        .map(|(k, _, _)| k.width() as u16)
        .max()
        .unwrap_or(0)
        .min(area.width / 2);
    for (i, (k, v, sev)) in rows.iter().enumerate() {
        if i as u16 >= area.height {
            break;
        }
        let y = area.y + i as u16;
        buf.set_stringn(
            area.x,
            y,
            k.as_ref(),
            key_w as usize,
            theme.style(Role::TextMuted),
        );
        let vx = area.x + key_w + 1;
        if vx < area.x + area.width {
            put_line(v, vx, y, area.x + area.width - vx, theme, buf);
        }
        if let Some(s) = sev {
            let (style, glyph) = theme.severity(*s);
            let gx = (area.x + area.width).saturating_sub(2);
            if gx >= vx {
                buf.set_string(gx, y, glyph, style);
            }
        }
    }
}

fn gauge(
    label: &str,
    value: f32,
    gradient: crate::theme::GradientId,
    txt: Option<&str>,
    area: Rect,
    theme: &Theme,
    buf: &mut Buffer,
) {
    let v = value.clamp(0.0, 1.0);
    let y = area.y;
    let label_w = (label.width() as u16).min(area.width / 3);
    buf.set_stringn(
        area.x,
        y,
        label,
        label_w as usize,
        theme.style(Role::TextMuted),
    );
    let text_str = txt.unwrap_or("");
    let text_w = text_str.width() as u16;
    let bar_x = area.x + label_w + u16::from(label_w > 0);
    let bar_end = (area.x + area.width).saturating_sub(text_w + u16::from(text_w > 0));
    if bar_end <= bar_x {
        return;
    }
    let bar_w = bar_end - bar_x;
    let g = theme.gradient(gradient);
    let filled = (f32::from(bar_w) * v).round() as u16;
    for i in 0..bar_w {
        let t = f32::from(i) / f32::from(bar_w.max(1));
        let (ch, style) = if i < filled {
            let c = g.sample(t);
            match theme.widgets.gauge {
                GaugeStyle::Bar => (theme.glyphs.full(), Style::new().fg(c)),
                GaugeStyle::Line => ('━', Style::new().fg(c)),
                GaugeStyle::Block => (theme.glyphs.full(), Style::new().fg(c)),
            }
        } else {
            match theme.widgets.gauge {
                GaugeStyle::Line => ('─', theme.style(Role::TextGhost)),
                _ => (theme.glyphs.empty(), theme.style(Role::TextGhost)),
            }
        };
        buf.set_string(bar_x + i, y, ch.to_string(), style);
    }
    if text_w > 0 {
        buf.set_string(bar_end + 1, y, text_str, theme.style(Role::Text));
    }
}

fn segmented(
    label: &str,
    segments: &[(Role, f32)],
    txt: Option<&str>,
    area: Rect,
    theme: &Theme,
    buf: &mut Buffer,
) {
    let y = area.y;
    let label_w = (label.width() as u16).min(area.width / 3);
    buf.set_stringn(
        area.x,
        y,
        label,
        label_w as usize,
        theme.style(Role::TextMuted),
    );
    let text_str = txt.unwrap_or("");
    let text_w = text_str.width() as u16;
    let bar_x = area.x + label_w + u16::from(label_w > 0);
    let bar_end = (area.x + area.width).saturating_sub(text_w + u16::from(text_w > 0));
    if bar_end <= bar_x {
        return;
    }
    let bar_w = bar_end - bar_x;
    // htop-style: '[' fill ']' when width allows.
    let (fill_x, fill_w, bracket) = if bar_w >= 6 {
        (bar_x + 1, bar_w - 2, true)
    } else {
        (bar_x, bar_w, false)
    };
    if bracket {
        buf.set_string(bar_x, y, "[", theme.style(Role::TextMuted));
        buf.set_string(bar_x + bar_w - 1, y, "]", theme.style(Role::TextMuted));
    }
    let mut cursor = 0u16;
    let mut acc = 0.0f32;
    // Every segment used to draw the same `|`, so with no colour the
    // boundaries vanished and the meter read as one solid bar — far fuller
    // than it was (arc-1b review, fixed in arc 10a, D60). A theme with no
    // colour to give gets a glyph per segment instead; `[widgets] segmented`
    // can force either, and `auto` decides from the *resolved* theme, because
    // `--color mono` and `NO_COLOR` reach every theme.
    let per_segment = theme.segmented_glyphs();
    let mut glyph = [0u8; 4];
    for (i, (role, frac)) in segments.iter().enumerate() {
        acc += frac.clamp(0.0, 1.0);
        let upto = (f32::from(fill_w) * acc.clamp(0.0, 1.0)).round() as u16;
        let style = theme.style(*role);
        let fill: &str = if per_segment {
            theme.glyphs.segment(i).encode_utf8(&mut glyph)
        } else {
            "|"
        };
        while cursor < upto.min(fill_w) {
            buf.set_string(fill_x + cursor, y, fill, style);
            cursor += 1;
        }
    }
    while cursor < fill_w {
        buf.set_string(fill_x + cursor, y, " ", theme.style(Role::TextGhost));
        cursor += 1;
    }
    if text_w > 0 {
        buf.set_string(bar_end + 1, y, text_str, theme.style(Role::Text));
    }
}

fn bars(
    values: &[f32],
    gradient: crate::theme::GradientId,
    labels: Option<&[Cow<'static, str>]>,
    peaks: Option<&[f32]>,
    area: Rect,
    theme: &Theme,
    buf: &mut Buffer,
) {
    let label_rows = u16::from(labels.is_some() && area.height >= 2);
    let h = area.height - label_rows;
    if h == 0 {
        return;
    }
    let g = theme.gradient(gradient);
    let eighths = theme.glyphs.eighths();
    let n = values.len().min(area.width as usize);
    for (i, v) in values.iter().take(n).enumerate() {
        let v = v.clamp(0.0, 1.0);
        let x = area.x + i as u16;
        let total8 = (v * f32::from(h) * 8.0).round() as u16;
        let colour = Style::new().fg(g.sample(v));
        for row in 0..h {
            let y = area.y + h - 1 - row;
            let cell8 = total8.saturating_sub(row * 8);
            if cell8 == 0 {
                continue;
            }
            let ch = if cell8 >= 8 {
                eighths[7]
            } else {
                eighths[(cell8 as usize) - 1]
            };
            buf.set_string(x, y, ch.to_string(), colour);
        }
        if let Some(pk) = peaks.and_then(|p| p.get(i)).filter(|pk| **pk > 0.0) {
            let p8 = (pk.clamp(0.0, 1.0) * f32::from(h) * 8.0).round() as u16;
            let row = (p8 / 8).min(h.saturating_sub(1));
            let y = area.y + h - 1 - row;
            buf.set_string(x, y, "▔", theme.style(Role::Text));
        }
    }
    if label_rows == 1
        && let Some(ls) = labels
    {
        for (i, l) in ls.iter().take(n).enumerate() {
            buf.set_stringn(
                area.x + i as u16,
                area.y + h,
                l.as_ref(),
                1,
                theme.style(Role::TextMuted),
            );
        }
    }
}

fn sparkline(
    series: &[Option<f32>],
    gradient: crate::theme::GradientId,
    max: Option<f32>,
    area: Rect,
    theme: &Theme,
    buf: &mut Buffer,
) {
    let g = theme.gradient(gradient);
    let eighths = theme.glyphs.eighths();
    let top = max.unwrap_or_else(|| {
        series
            .iter()
            .flatten()
            .fold(0.0f32, |a, b| a.max(*b))
            .max(f32::EPSILON)
    });
    let w = area.width as usize;
    let take = series.len().min(w);
    let offset = series.len() - take;
    // One sample per column, htop's and nvtop's way (D62): a column with no
    // sample of its own draws the newest sample before it, so a tile wider
    // than the sample count is a meter and not a picket fence. Only the
    // columns before the first sample stay empty.
    let mut last: Option<f32> = None;
    for (i, v) in series[offset..].iter().enumerate() {
        let x = area.x + (w - take + i) as u16;
        if v.is_some() {
            last = *v;
        }
        let Some(v) = last else { continue };
        let frac = (v / top).clamp(0.0, 1.0);
        let total8 = (frac * f32::from(area.height) * 8.0).round() as u16;
        let colour = Style::new().fg(g.sample(frac));
        for row in 0..area.height {
            let y = area.y + area.height - 1 - row;
            let cell8 = total8.saturating_sub(row * 8);
            if cell8 == 0 {
                continue;
            }
            let ch = if cell8 >= 8 {
                eighths[7]
            } else {
                eighths[(cell8 as usize) - 1]
            };
            buf.set_string(x, y, ch.to_string(), colour);
        }
    }
}

/// The cell row a normalised height lands on, mapped exactly the way the
/// marker plots a point: the braille pass divides its 4-per-cell sub-row by
/// four, the block pass fills from the floor in eighths, and the dot pass
/// spreads over `height - 1`. A gridline computed any other way disagrees
/// with the ink by a row, which is only ever noticed once something rests on
/// it — the net tile's mirrored chart takes the midpoint line as its zero.
fn value_row(fy: f64, area: Rect, marker: crate::theme::ChartMarker) -> u16 {
    use crate::theme::ChartMarker;
    let h = area.height.max(1);
    match marker {
        ChartMarker::Braille => {
            let h4 = u32::from(h) * 4;
            let dy = ((1.0 - fy) * f64::from(h4 - 1)).round().max(0.0) as u32;
            area.y + u16::try_from(dy / 4).unwrap_or(0).min(h - 1)
        }
        ChartMarker::Block => {
            let total8 = (fy * f64::from(h) * 8.0).round().max(0.0) as u32;
            let row = u16::try_from(total8.saturating_sub(1) / 8).unwrap_or(0);
            area.y + h - 1 - row.min(h - 1)
        }
        ChartMarker::Dot => {
            let up = (fy * f64::from(h - 1)).round().max(0.0) as u32;
            area.y + h - 1 - u16::try_from(up).unwrap_or(0).min(h - 1)
        }
    }
}

/// The quarters of `bounds.y` as horizontal rules, height-gated: eight rows
/// or more get 25/50/75, four get the midpoint alone, and a shorter band gets
/// none — four rules in a four-row band is a box of dashes, not an axis.
fn gridlines(area: Rect, marker: crate::theme::ChartMarker, theme: &Theme, buf: &mut Buffer) {
    let fracs: &[f64] = if area.height >= 8 {
        &[0.25, 0.5, 0.75]
    } else if area.height >= 4 {
        &[0.5]
    } else {
        &[]
    };
    let style = theme.style(Role::TextGhost);
    let glyph = theme.glyphs.gridline().to_string();
    for f in fracs {
        let y = value_row(*f, area, marker);
        for x in area.x..area.x + area.width {
            buf.set_string(x, y, &glyph, style);
        }
    }
}

/// Each series' name at its newest point, in that series' gradient sampled at
/// that point's height (D65 §4). `Series.label` has travelled in every
/// `View::Chart` since arc 2b and been read by nothing; a legend at the top
/// of a tall band whose ink sits at the bottom is not an answer, so the name
/// goes on the line. "Newest" is the last point the component supplied, which
/// is the only definition the renderer can have — a reversed chart reverses
/// its own data. Clipped to the rect, and drawn after the series so the name
/// is never half-eaten by its own line.
fn series_labels(
    series: &[crate::view::Series],
    area: Rect,
    marker: crate::theme::ChartMarker,
    norm: impl Fn(&crate::view::Series) -> Vec<(f64, f64)>,
    theme: &Theme,
    buf: &mut Buffer,
) {
    let right = area.x + area.width;
    for s in series {
        if s.label.is_empty() {
            continue;
        }
        let pts = norm(s);
        let Some((fx, fy)) = pts.last().copied() else {
            continue;
        };
        let x = area.x + (fx * f64::from(area.width.saturating_sub(1))).round() as u16;
        let y = value_row(fy, area, marker);
        let w = s.label.as_ref().width() as u16;
        let start = if x + 1 + w <= right {
            x + 1
        } else {
            right.saturating_sub(w).max(area.x)
        };
        let avail = usize::from(right.saturating_sub(start));
        if avail == 0 {
            continue;
        }
        let colour = theme.gradient(s.gradient).sample(fy as f32);
        buf.set_stringn(start, y, s.label.as_ref(), avail, Style::new().fg(colour));
    }
}

/// `View::Chart` (§4.6, arc 2b): a real line chart. The theme's `chart_marker`
/// picks the form — braille dots (2×4 per cell) drawing connected segments,
/// lower-eighth block columns, or one dot per point; the ascii tier always
/// gets `*`. Series are drawn in order, so the last one wins a contested cell;
/// colour comes from each series' gradient sampled at the point's height.
fn chart(
    series: &[crate::view::Series],
    bounds: &crate::view::Bounds,
    hint: crate::view::MarkerHint,
    area: Rect,
    theme: &Theme,
    buf: &mut Buffer,
) {
    use crate::theme::{ChartMarker, GlyphTier};
    use crate::view::MarkerHint;
    if area.width == 0 || area.height == 0 {
        return;
    }
    let marker = match (theme.glyphs.tier, hint, theme.glyphs.marker) {
        (GlyphTier::Ascii, _, _) => ChartMarker::Dot,
        (_, MarkerHint::Braille, _) => ChartMarker::Braille,
        (_, MarkerHint::Block, _) => ChartMarker::Block,
        (_, MarkerHint::Auto, m) => m,
    };
    let (x0, x1) = bounds.x;
    let (y0, y1) = bounds.y;
    let xr = (x1 - x0).max(f64::EPSILON);
    let yr = (y1 - y0).max(f64::EPSILON);
    // Normalised (0..=1) coordinates, x ascending as the series supplies them.
    let norm = |s: &crate::view::Series| -> Vec<(f64, f64)> {
        s.data
            .iter()
            .map(|(px, py)| {
                (
                    ((px - x0) / xr).clamp(0.0, 1.0),
                    ((py - y0) / yr).clamp(0.0, 1.0),
                )
            })
            .collect()
    };
    // The axis, before any series (D65 §4). `PARITY.md` recorded nvtop's
    // "fixed 0-100 % axis" as *in* while gridwatch drew only its range; these
    // are the ticks. Under the series on purpose — the braille mask is
    // written afterwards, so ink wins a contested cell and a gridline hidden
    // by the line it belongs to is correct. Unlabelled: `Bounds` carries no
    // unit, and giving it one is a §4.6 change.
    gridlines(area, marker, theme, buf);
    match marker {
        ChartMarker::Braille => {
            let w = usize::from(area.width) * 2;
            let h = usize::from(area.height) * 4;
            let mut mask = vec![0u8; usize::from(area.width) * usize::from(area.height)];
            let mut color: Vec<Option<ratatui_core::style::Color>> = vec![None; mask.len()];
            for s in series {
                let g = theme.gradient(s.gradient);
                let pts: Vec<(i64, i64)> = norm(s)
                    .iter()
                    .map(|(fx, fy)| {
                        (
                            (fx * (w - 1) as f64).round() as i64,
                            ((1.0 - fy) * (h - 1) as f64).round() as i64,
                        )
                    })
                    .collect();
                let heights: Vec<f64> = norm(s).iter().map(|(_, fy)| *fy).collect();
                let mut plot = |dx: i64, dy: i64, fy: f64| {
                    if dx < 0 || dy < 0 || dx >= w as i64 || dy >= h as i64 {
                        return;
                    }
                    let (cx, cy) = ((dx / 2) as usize, (dy / 4) as usize);
                    let bit = match (dx % 2, dy % 4) {
                        (0, 0) => 0x01,
                        (0, 1) => 0x02,
                        (0, 2) => 0x04,
                        (0, 3) => 0x40,
                        (1, 0) => 0x08,
                        (1, 1) => 0x10,
                        (1, 2) => 0x20,
                        _ => 0x80,
                    };
                    let i = cy * usize::from(area.width) + cx;
                    mask[i] |= bit;
                    color[i] = Some(g.sample(fy as f32));
                };
                if pts.len() == 1 {
                    plot(pts[0].0, pts[0].1, heights[0]);
                }
                for (i, pair) in pts.windows(2).enumerate() {
                    let ((ax, ay), (bx, by)) = (pair[0], pair[1]);
                    let fy = (heights[i] + heights[i + 1]) / 2.0;
                    // Bresenham between consecutive points.
                    let (dx, dy) = ((bx - ax).abs(), -(by - ay).abs());
                    let (sx, sy) = ((bx - ax).signum(), (by - ay).signum());
                    let (mut x, mut y, mut err) = (ax, ay, dx + dy);
                    loop {
                        plot(x, y, fy);
                        if x == bx && y == by {
                            break;
                        }
                        let e2 = 2 * err;
                        if e2 >= dy {
                            err += dy;
                            x += sx;
                        }
                        if e2 <= dx {
                            err += dx;
                            y += sy;
                        }
                    }
                }
            }
            for cy in 0..usize::from(area.height) {
                for cx in 0..usize::from(area.width) {
                    let i = cy * usize::from(area.width) + cx;
                    if mask[i] == 0 {
                        continue;
                    }
                    let ch = char::from_u32(0x2800 + u32::from(mask[i])).unwrap_or('•');
                    let style = color[i].map(|c| Style::new().fg(c)).unwrap_or_default();
                    buf.set_string(
                        area.x + cx as u16,
                        area.y + cy as u16,
                        ch.to_string(),
                        style,
                    );
                }
            }
        }
        ChartMarker::Block => {
            let eighths = theme.glyphs.eighths();
            let cols = usize::from(area.width);
            for s in series {
                let g = theme.gradient(s.gradient);
                // The highest point landing in each column.
                let mut top: Vec<Option<f64>> = vec![None; cols];
                for (fx, fy) in norm(s) {
                    let c = ((fx * (cols - 1) as f64).round() as usize).min(cols - 1);
                    top[c] = Some(top[c].map_or(fy, |t: f64| t.max(fy)));
                }
                for (c, v) in top.iter().enumerate() {
                    let Some(v) = v else { continue };
                    let total8 = (*v * f64::from(area.height) * 8.0).round() as u16;
                    let colour = Style::new().fg(g.sample(*v as f32));
                    for row in 0..area.height {
                        let y = area.y + area.height - 1 - row;
                        let cell8 = total8.saturating_sub(row * 8);
                        if cell8 == 0 {
                            continue;
                        }
                        let ch = if cell8 >= 8 {
                            eighths[7]
                        } else {
                            eighths[(cell8 as usize) - 1]
                        };
                        buf.set_string(area.x + c as u16, y, ch.to_string(), colour);
                    }
                }
            }
        }
        ChartMarker::Dot => {
            let glyph = if theme.glyphs.tier == GlyphTier::Ascii {
                "*"
            } else {
                "•"
            };
            for s in series {
                let g = theme.gradient(s.gradient);
                for (fx, fy) in norm(s) {
                    let x = area.x + (fx * f64::from(area.width.saturating_sub(1))).round() as u16;
                    let y = area.y + area.height
                        - 1
                        - (fy * f64::from(area.height.saturating_sub(1))).round() as u16;
                    buf.set_string(x, y, glyph, Style::new().fg(g.sample(fy as f32)));
                }
            }
        }
    }
    series_labels(series, area, marker, norm, theme, buf);
}

/// The width each column *wants*: the widest of its cells over **every** row
/// and its own title (§4.6, D65 §1). The title is in the maximum because the
/// header is printed into the column's own width — a column capped below its
/// title would lose the name of what is under it.
///
/// Measured over every row and never the visible page: a column that changes
/// width as you scroll is worse than one that stretches (D65 trap 7).
pub fn table_natural_widths(columns: &[crate::view::Column], rows: &[Vec<Line>]) -> Vec<u16> {
    columns
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let widest = rows
                .iter()
                .filter_map(|r| r.get(i))
                .map(|cell| cell.iter().map(|s| s.text.as_ref().width()).sum::<usize>())
                .max()
                .unwrap_or(0);
            u16::try_from(widest.max(c.title.as_ref().width())).unwrap_or(u16::MAX)
        })
        .collect()
}

/// The width each column is actually **drawn** at inside `width`: the fixed
/// widths as declared, then the spare shared between every elastic column
/// (D57 amendment 19) and each capped at its natural width (D65 §3). The
/// leftover is neither redistributed nor drawn, so a table ends where its
/// content ends. Below the content width the cap never binds and the result
/// is byte-identical to the share alone.
pub fn table_widths(columns: &[crate::view::Column], rows: &[Vec<Line>], width: u16) -> Vec<u16> {
    let mut widths: Vec<u16> = columns
        .iter()
        .map(|c| match c.width {
            ColWidth::Fixed(w) => w,
            ColWidth::Elastic => 0,
        })
        .collect();
    let fixed: u16 = widths.iter().sum::<u16>() + columns.len().saturating_sub(1) as u16;
    let spare = width.saturating_sub(fixed);
    let elastic: Vec<usize> = columns
        .iter()
        .enumerate()
        .filter(|(_, c)| c.width == ColWidth::Elastic)
        .map(|(i, _)| i)
        .collect();
    if !elastic.is_empty() {
        let each = spare / elastic.len() as u16;
        for &i in &elastic {
            widths[i] = each;
        }
        // The remainder goes to the last, so the row still fills the rect.
        widths[*elastic.last().expect("non-empty")] += spare % elastic.len() as u16;
        let natural = table_natural_widths(columns, rows);
        for &i in &elastic {
            widths[i] = widths[i].min(natural[i]);
        }
    }
    widths
}

#[allow(clippy::too_many_arguments)] // internal helper mirroring View::Table's fields
fn table(
    columns: &[crate::view::Column],
    rows: &[Vec<Line>],
    selected: Option<usize>,
    sort: Option<(usize, SortDir)>,
    scroll: usize,
    area: Rect,
    theme: &Theme,
    buf: &mut Buffer,
) {
    if area.height == 0 {
        return;
    }
    // Column x positions: the fixed widths, then the spare shared between
    // **every** elastic column (the last one takes the remainder). Giving
    // it all to the last left an earlier elastic at zero, which this
    // function then skipped entirely — the net tile's connection table
    // drew its remote address under a `local` header and nobody could see
    // the local one (arc 7a review, D57 amendment 19). Each elastic column
    // is then capped at the widest cell it holds (D65 §3), so a table ends
    // where its content ends instead of pushing its numbers eighty-seven
    // cells away from the row they belong to.
    let widths = table_widths(columns, rows, area.width);
    // Header.
    let header_style = match theme.widgets.table_header {
        HeaderStyle::Reverse => theme.style(Role::Text).add_modifier(Modifier::REVERSED),
        HeaderStyle::Underline => theme.style(Role::Text).add_modifier(Modifier::UNDERLINED),
        HeaderStyle::Plain => theme.style(Role::TextMuted).add_modifier(Modifier::BOLD),
    };
    let mut x = area.x;
    for (i, (c, w)) in columns.iter().zip(&widths).enumerate() {
        if *w == 0 || x >= area.x + area.width {
            x += 1;
            continue;
        }
        let title = c.title.to_string();
        let pad_w = *w as usize;
        let cell = if c.right {
            format!("{title:>pad_w$}")
        } else {
            format!("{title:<pad_w$}")
        };
        buf.set_stringn(x, area.y, &cell, pad_w, header_style);
        // The sort column's *separator* carries htop's glyph (§8.1), so a
        // four-wide `CPU%` keeps all four characters.
        if let Some((si, dir)) = sort
            && si == i
            && x + w < area.x + area.width
        {
            let glyph = match dir {
                SortDir::Desc => "▽",
                SortDir::Asc => "△",
            };
            buf.set_string(x + w, area.y, glyph, header_style);
        }
        x += w + 1;
    }
    // Rows.
    let body_h = area.height.saturating_sub(1) as usize;
    for (ri, row) in rows.iter().skip(scroll).take(body_h).enumerate() {
        let y = area.y + 1 + ri as u16;
        let absolute = ri + scroll;
        let sel = selected == Some(absolute);
        // A theme whose selection colours are the terminal defaults (`mono`)
        // would make the selected row invisible: reverse video carries it.
        let sel_style = || {
            let s = Style::new()
                .fg(theme.color(Role::SelectionFg))
                .bg(theme.color(Role::SelectionBg));
            if theme.color(Role::SelectionBg) == ratatui_core::style::Color::Reset {
                s.add_modifier(Modifier::REVERSED)
            } else {
                s
            }
        };
        if sel {
            let style = sel_style();
            for cx in area.x..area.x + area.width {
                buf.set_string(cx, y, " ", style);
            }
        }
        let mut x = area.x;
        for (cell, (c, w)) in row.iter().zip(columns.iter().zip(&widths)) {
            if *w == 0 || x >= area.x + area.width {
                x += 1;
                continue;
            }
            if c.right {
                let cell_w: u16 = cell.iter().map(|s| s.text.as_ref().width() as u16).sum();
                let pad = w.saturating_sub(cell_w);
                put_line(cell, x + pad, y, w.saturating_sub(pad), theme, buf);
            } else {
                put_line(cell, x, y, *w, theme, buf);
            }
            if sel {
                for cx in x..x + w {
                    if let Some(cell) = buf.cell_mut((cx, y)) {
                        cell.set_style(sel_style());
                    }
                }
            }
            x += w + 1;
        }
    }
}

fn big_number(t: &str, role: Role, area: Rect, theme: &Theme, buf: &mut Buffer) {
    let pixel = match theme.widgets.big_number {
        PixelStyle::Quadrant => PixelSize::Quadrant,
        PixelStyle::Sextant => PixelSize::Sextant,
        PixelStyle::Full => PixelSize::Full,
    };
    let widget = BigText::builder()
        .pixel_size(pixel)
        .style(theme.style(role))
        .lines(vec![RLine::from(t.to_string())])
        .build();
    ratatui_core::widgets::Widget::render(widget, area, buf);
}

fn stack(dir: Dir, children: &[(Constraint, View)], area: Rect, theme: &Theme, buf: &mut Buffer) {
    // The arithmetic lives in `layout::split_rects`, which is also what
    // `layout::leaves` walks (D65 §10): one implementation, so the per-leaf
    // growth oracle cannot measure a rect the renderer never drew into.
    let constraints: Vec<Constraint> = children.iter().map(|(c, _)| *c).collect();
    for ((_, view), sub) in children
        .iter()
        .zip(crate::layout::split_rects(dir, &constraints, area))
    {
        if sub.width == 0 || sub.height == 0 {
            continue;
        }
        DEFAULT_RENDERER.render(view, sub, theme, buf);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::{ColorMode, load_builtin};

    fn th() -> Theme {
        load_builtin("modern", ColorMode::TrueColor).expect("built-in theme loads")
    }

    /// Non-blank cells per column, left to right.
    fn columns(buf: &Buffer) -> Vec<usize> {
        let area = *buf.area();
        (0..area.width)
            .map(|x| {
                (0..area.height)
                    .filter(|y| {
                        buf.cell((x, *y))
                            .is_some_and(|c| !c.symbol().trim().is_empty())
                    })
                    .count()
            })
            .collect()
    }

    fn draw(series: &[Option<f32>], w: u16, h: u16) -> Vec<usize> {
        let area = Rect {
            x: 0,
            y: 0,
            width: w,
            height: h,
        };
        let mut buf = Buffer::empty(area);
        sparkline(
            series,
            crate::theme::GradientId::Load,
            Some(1.0),
            area,
            &th(),
            &mut buf,
        );
        columns(&buf)
    }

    /// D62 §1: a column with no sample of its own holds the newest sample
    /// before it; the columns before the first sample stay empty.
    #[test]
    fn a_sparkline_holds_across_empty_columns() {
        assert_eq!(
            draw(&[Some(1.0), None, None, Some(0.5), None], 5, 4),
            vec![4, 4, 4, 2, 2]
        );
    }

    #[test]
    fn a_sparkline_is_empty_before_its_first_sample() {
        assert_eq!(draw(&[None, None, Some(1.0)], 3, 4), vec![0, 0, 4]);
    }

    /// The hold does not disturb a fully-populated series, and a series wider
    /// than the area still shows its newest `width` samples (the `offset`).
    #[test]
    fn a_full_sparkline_is_unchanged_and_a_long_one_still_scrolls() {
        assert_eq!(
            draw(&[Some(1.0), Some(0.5), Some(1.0)], 3, 4),
            vec![4, 2, 4]
        );
        assert_eq!(
            draw(&[Some(1.0), Some(1.0), Some(0.5), Some(0.25)], 2, 4),
            vec![2, 1]
        );
    }
}

#[cfg(test)]
mod table_and_chart_tests {
    use super::*;
    use crate::theme::{ColorMode, load_builtin};
    use crate::view::{Bounds, Column, MarkerHint, Series, Span};

    fn th() -> Theme {
        load_builtin("modern", ColorMode::TrueColor).expect("built-in theme loads")
    }

    fn col(title: &'static str, width: ColWidth) -> Column {
        Column {
            title: title.into(),
            width,
            right: false,
        }
    }

    fn cell(t: &'static str) -> Line {
        vec![Span::new(Role::Text, t)]
    }

    fn widths(columns: &[Column], rows: &[Vec<Line>], w: u16) -> Vec<u16> {
        table_widths(columns, rows, w)
    }

    // ------------------------------------------------ the elastic cap (D65 §3)

    /// A single elastic column stops at the widest cell it holds; the leftover
    /// is neither redistributed nor drawn, so the table ends at its content.
    #[test]
    fn one_elastic_column_ends_at_its_content() {
        let cols = [
            col("pid", ColWidth::Fixed(5)),
            col("cmd", ColWidth::Elastic),
        ];
        let rows = vec![
            vec![cell("1"), cell("bash")],
            vec![cell("2"), cell("nvtop")],
        ];
        assert_eq!(widths(&cols, &rows, 100), vec![5, 5]);
    }

    /// The invariant that keeps every compliant table byte-identical to `main`:
    /// below the content width the share is smaller than the cap, so the cap
    /// never binds and the arithmetic is D57 amendment 19's, untouched.
    #[test]
    fn a_table_narrower_than_its_content_is_unchanged() {
        let cols = [
            col("pid", ColWidth::Fixed(5)),
            col("cmd", ColWidth::Elastic),
        ];
        let rows = vec![vec![
            cell("1"),
            cell("/usr/lib/firefox/firefox -contentproc"),
        ]];
        // 20 cells: 5 fixed + 1 separator leaves 14 for the elastic.
        assert_eq!(widths(&cols, &rows, 20), vec![5, 14]);
    }

    /// Two elastic columns: each is capped at its own content, and the room
    /// the short one gives back is *not* handed to the long one — the leftover
    /// is left empty (D65 §1, "what is still spare is left empty").
    #[test]
    fn two_elastic_columns_are_capped_independently() {
        let cols = [
            col("local", ColWidth::Elastic),
            col("remote", ColWidth::Elastic),
        ];
        let rows = vec![vec![cell("10.0.0.2:22"), cell("1.1.1.1:443")]];
        assert_eq!(widths(&cols, &rows, 200), vec![11, 11]);
    }

    /// One short, one long: the long one still gets no more than its share.
    #[test]
    fn a_short_elastic_column_does_not_feed_a_long_one() {
        let cols = [col("a", ColWidth::Elastic), col("b", ColWidth::Elastic)];
        let rows = vec![vec![cell("xy"), cell(LONG)]];
        const LONG: &str = "0123456789012345678901234567890123456789";
        // 41 cells of rect: one separator, 20 each; `a` caps at 2, `b` keeps 20.
        assert_eq!(widths(&cols, &rows, 41), vec![2, 20]);
    }

    /// The header is printed into the column's own width, so a column whose
    /// cells are shorter than its title keeps the title's width — capping to
    /// the cells alone would truncate the name of what is under it.
    #[test]
    fn a_column_never_caps_below_its_own_title() {
        let cols = [col("interface", ColWidth::Elastic)];
        let rows = vec![vec![cell("lo")]];
        assert_eq!(widths(&cols, &rows, 80), vec![9]);
        let mut buf = Buffer::empty(Rect {
            x: 0,
            y: 0,
            width: 80,
            height: 2,
        });
        let area = *buf.area();
        table(&cols, &rows, None, None, 0, area, &th(), &mut buf);
        let head: String = (0..9)
            .map(|x| buf.cell((x, 0)).expect("a cell").symbol().to_string())
            .collect();
        assert_eq!(head, "interface");
    }

    /// Measured over **every** row, never the visible page: a column that
    /// changes width as you scroll is worse than one that stretches (D65 trap
    /// 7). The widest cell here is on a row the rect cannot show.
    #[test]
    fn the_cap_measures_rows_below_the_fold() {
        let cols = [col("cmd", ColWidth::Elastic)];
        let rows: Vec<Vec<Line>> = (0..40)
            .map(|i| vec![cell(if i == 39 { "a-very-long-command" } else { "sh" })])
            .collect();
        assert_eq!(widths(&cols, &rows, 80), vec![19]);
    }

    // ------------------------------------------------ chart furniture (D65 §4)

    fn chart_buf(w: u16, h: u16, series: Vec<Series>) -> Buffer {
        let area = Rect {
            x: 0,
            y: 0,
            width: w,
            height: h,
        };
        let mut buf = Buffer::empty(area);
        chart(
            &series,
            &Bounds {
                x: (0.0, 9.0),
                y: (0.0, 100.0),
            },
            MarkerHint::Braille,
            area,
            &th(),
            &mut buf,
        );
        buf
    }

    /// Rows made entirely of the gridline glyph.
    fn ruled_rows(buf: &Buffer) -> Vec<u16> {
        let area = *buf.area();
        (0..area.height)
            .filter(|y| {
                (0..area.width).all(|x| buf.cell((x, *y)).is_some_and(|c| c.symbol() == "─"))
            })
            .collect()
    }

    /// Eight rows or more: the quarters of `bounds.y`, 25/50/75.
    #[test]
    fn a_tall_chart_carries_three_gridlines() {
        assert_eq!(ruled_rows(&chart_buf(20, 8, vec![])).len(), 3);
        assert_eq!(ruled_rows(&chart_buf(20, 40, vec![])).len(), 3);
    }

    /// Four to seven rows: the midpoint alone.
    #[test]
    fn a_short_chart_carries_only_its_midpoint() {
        for h in 4..8 {
            assert_eq!(
                ruled_rows(&chart_buf(20, h, vec![])).len(),
                1,
                "a {h}-row band should carry one gridline"
            );
        }
    }

    /// Below four rows: none. Four rules in a four-row band is a box of dashes.
    #[test]
    fn a_tiny_chart_carries_no_gridlines() {
        for h in 1..4 {
            assert!(ruled_rows(&chart_buf(20, h, vec![])).is_empty());
        }
    }

    /// The midpoint line sits where a series at the middle of `bounds.y`
    /// draws, which is what lets the net tile use it as a zero line.
    #[test]
    fn the_midpoint_gridline_is_where_a_midpoint_series_draws() {
        let mid = Series {
            label: "".into(),
            gradient: crate::theme::GradientId::Load,
            data: (0..10).map(|i| (f64::from(i), 50.0)).collect(),
        };
        let ruled = ruled_rows(&chart_buf(20, 8, vec![]));
        let buf = chart_buf(20, 8, vec![mid]);
        let area = *buf.area();
        let inked: Vec<u16> = (0..area.height)
            .filter(|y| {
                (0..area.width).any(|x| {
                    buf.cell((x, *y))
                        .is_some_and(|c| !c.symbol().trim().is_empty())
                })
            })
            .collect();
        assert!(
            inked.contains(&ruled[1]),
            "the series at the midpoint draws on rows {inked:?}, the midpoint rule is {}",
            ruled[1]
        );
    }

    /// Ink wins a contested cell: the mask is written after the rules, so a
    /// gridline under a line disappears where the line covers it.
    #[test]
    fn a_series_covers_the_gridline_it_crosses() {
        let flat = Series {
            label: "".into(),
            gradient: crate::theme::GradientId::Load,
            data: (0..10).map(|i| (f64::from(i), 50.0)).collect(),
        };
        let buf = chart_buf(20, 8, vec![flat]);
        let row = ruled_rows(&chart_buf(20, 8, vec![]))[1];
        let still_ruled = (0..20).all(|x| buf.cell((x, row)).is_some_and(|c| c.symbol() == "─"));
        assert!(!still_ruled, "the series did not cover its gridline");
    }

    /// `Series.label` at the series' newest point — carried since arc 2b and
    /// read by nothing until D65.
    #[test]
    fn each_series_is_named_at_its_newest_point() {
        let s = Series {
            label: "util".into(),
            gradient: crate::theme::GradientId::Load,
            data: (0..10)
                .map(|i| (f64::from(i), 10.0 * f64::from(i)))
                .collect(),
        };
        let buf = chart_buf(40, 10, vec![s]);
        let text: String = (0..10)
            .flat_map(|y| (0..40).map(move |x| (x, y)))
            .map(|(x, y)| buf.cell((x, y)).expect("a cell").symbol().to_string())
            .collect();
        assert!(text.contains("util"), "the series name is not drawn");
    }

    /// A label that would run past the right edge is pulled back inside it.
    #[test]
    fn a_label_at_the_right_edge_is_clipped_inside_the_rect() {
        let s = Series {
            label: "a-very-long-series-name".into(),
            gradient: crate::theme::GradientId::Load,
            data: (0..10).map(|i| (f64::from(i), 50.0)).collect(),
        };
        let buf = chart_buf(30, 8, vec![s]);
        let area = *buf.area();
        assert_eq!(area.width, 30);
        let row: String = (0..30)
            .map(|x| {
                buf.cell((x, 4))
                    .map(|c| c.symbol().to_string())
                    .unwrap_or_default()
            })
            .collect();
        assert!(
            row.contains("a-very-long-series-nam") || row.contains("a-very-long-series-name"),
            "the label was not clipped into the rect: {row:?}"
        );
    }
}
