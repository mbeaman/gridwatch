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
            View::Chart { series, bounds, .. } => chart(series, bounds, area, theme, buf),
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
    for (role, frac) in segments {
        acc += frac.clamp(0.0, 1.0);
        let upto = (f32::from(fill_w) * acc.clamp(0.0, 1.0)).round() as u16;
        let style = theme.style(*role);
        while cursor < upto.min(fill_w) {
            buf.set_string(fill_x + cursor, y, "|", style);
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
        if let Some(pk) = peaks.and_then(|p| p.get(i)) {
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
    for (i, v) in series[offset..].iter().enumerate() {
        let x = area.x + (w - take + i) as u16;
        let Some(v) = v else { continue };
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

fn chart(
    series: &[crate::view::Series],
    bounds: &crate::view::Bounds,
    area: Rect,
    theme: &Theme,
    buf: &mut Buffer,
) {
    // Minimal in arc 1a (real charts land with the GPU arc): scaled dots.
    let (x0, x1) = bounds.x;
    let (y0, y1) = bounds.y;
    let xr = (x1 - x0).max(f64::EPSILON);
    let yr = (y1 - y0).max(f64::EPSILON);
    for s in series {
        let g = theme.gradient(s.gradient);
        for (px, py) in &s.data {
            let fx = ((px - x0) / xr).clamp(0.0, 1.0);
            let fy = ((py - y0) / yr).clamp(0.0, 1.0);
            let x = area.x + (fx * f64::from(area.width.saturating_sub(1))) as u16;
            let y =
                area.y + area.height - 1 - (fy * f64::from(area.height.saturating_sub(1))) as u16;
            buf.set_string(x, y, "•", Style::new().fg(g.sample(fy as f32)));
        }
    }
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
    // Column x positions: fixed widths + one elastic (the last Elastic gets the rest).
    let mut widths: Vec<u16> = columns
        .iter()
        .map(|c| match c.width {
            ColWidth::Fixed(w) => w,
            ColWidth::Elastic => 0,
        })
        .collect();
    let fixed: u16 = widths.iter().sum::<u16>() + columns.len().saturating_sub(1) as u16;
    let spare = area.width.saturating_sub(fixed);
    if let Some(i) = columns.iter().rposition(|c| c.width == ColWidth::Elastic) {
        widths[i] = spare;
    }
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
        let mut title = c.title.to_string();
        if let Some((si, dir)) = sort
            && si == i
        {
            title.push(match dir {
                SortDir::Desc => '▽',
                SortDir::Asc => '△',
            });
        }
        let pad_w = *w as usize;
        let cell = if c.right {
            format!("{title:>pad_w$}")
        } else {
            format!("{title:<pad_w$}")
        };
        buf.set_stringn(x, area.y, &cell, pad_w, header_style);
        x += w + 1;
    }
    // Rows.
    let body_h = area.height.saturating_sub(1) as usize;
    for (ri, row) in rows.iter().skip(scroll).take(body_h).enumerate() {
        let y = area.y + 1 + ri as u16;
        let absolute = ri + scroll;
        let sel = selected == Some(absolute);
        if sel {
            let style = Style::new()
                .fg(theme.color(Role::SelectionFg))
                .bg(theme.color(Role::SelectionBg));
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
                        cell.set_style(
                            Style::new()
                                .fg(theme.color(Role::SelectionFg))
                                .bg(theme.color(Role::SelectionBg)),
                        );
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
    let total = if dir == Dir::V {
        area.height
    } else {
        area.width
    };
    // First pass: fixed sizes; Fill shares the remainder by weight.
    let mut sizes = vec![0u16; children.len()];
    let mut used = 0u16;
    let mut fill_weight = 0u16;
    for (i, (c, _)) in children.iter().enumerate() {
        match c {
            Constraint::Len(l) | Constraint::Min(l) => {
                sizes[i] = (*l).min(total.saturating_sub(used));
                used += sizes[i];
            }
            Constraint::Fill(w) => fill_weight += *w.max(&1),
        }
    }
    let mut remaining = total.saturating_sub(used);
    for (i, (c, _)) in children.iter().enumerate() {
        if let Constraint::Fill(w) = c {
            let share = (remaining * w.max(&1))
                .checked_div(fill_weight)
                .unwrap_or(0);
            sizes[i] = share;
            fill_weight -= w.max(&1);
            remaining -= share;
        }
    }
    let mut offset = 0u16;
    for ((_, view), size) in children.iter().zip(sizes) {
        if size == 0 {
            continue;
        }
        let sub = if dir == Dir::V {
            Rect {
                x: area.x,
                y: area.y + offset,
                width: area.width,
                height: size,
            }
        } else {
            Rect {
                x: area.x + offset,
                y: area.y,
                width: size,
                height: area.height,
            }
        };
        DEFAULT_RENDERER.render(view, sub, theme, buf);
        offset += size;
    }
}
