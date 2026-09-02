//! Shell overlays (§10, §11): help, too-small notice, the F12 stats HUD, and
//! the placeholder chip for missing/starved/unbuilt tiles.

use ratatui_core::buffer::Buffer;
use ratatui_core::layout::Rect;
use ratatui_core::style::Modifier;

use crate::theme::{Role, Theme};

/// Frame statistics for the F12 HUD (filled by the app, P6/P8/P19).
#[derive(Clone, Copy, Debug, Default)]
pub struct HudStats {
    /// P18: shell start → first drawn frame, and → every source's first sample.
    pub first_frame_ms: u64,
    pub sources_live_ms: u64,
    pub frame_p50_us: u64,
    pub frame_p95_us: u64,
    pub changed_cells: u64,
    pub bytes_written: u64,
    pub frames: u64,
    pub redraw_data: u64,
    pub redraw_anim: u64,
    pub redraw_heartbeat: u64,
    pub mode: &'static str,
    /// `--record`: lines written and lines dropped by the tee (§4.5).
    pub recording: Option<(u64, u64)>,
}

pub fn hud(stats: &HudStats, area: Rect, theme: &Theme, buf: &mut Buffer) {
    let lines = [
        format!(
            "frame p50 {:>6}µs  p95 {:>6}µs",
            stats.frame_p50_us, stats.frame_p95_us
        ),
        format!(
            "cells Δ {:>8}   bytes {:>9}",
            stats.changed_cells, stats.bytes_written
        ),
        format!(
            "frames {:>5}  data {} anim {} beat {}",
            stats.frames, stats.redraw_data, stats.redraw_anim, stats.redraw_heartbeat
        ),
        format!("mode {}", stats.mode),
        // P18's two timestamps, so `--stats` shows what its row promises.
        format!(
            "start {:>4}ms  live {:>5}ms",
            stats.first_frame_ms, stats.sources_live_ms
        ),
        match stats.recording {
            Some((lines, dropped)) => format!("rec {lines:>6} lines  dropped {dropped}"),
            None => "rec off".to_string(),
        },
    ];
    let w = lines.iter().map(|l| l.len()).max().unwrap_or(0) as u16 + 2;
    let h = lines.len() as u16 + 2;
    let x = area.x + area.width.saturating_sub(w + 1);
    let y = area.y + 1;
    fill(
        Rect {
            x,
            y,
            width: w.min(area.width),
            height: h.min(area.height),
        },
        theme,
        buf,
    );
    for (i, l) in lines.iter().enumerate() {
        buf.set_stringn(
            x + 1,
            y + 1 + i as u16,
            l,
            (w.saturating_sub(2)) as usize,
            theme.style(Role::Text),
        );
    }
}

/// A placeholder tile: the kind, why it is not live, and — when there is one
/// — the fix (§11: "placeholder tiles with fix text").
pub fn chip(kind: &str, reason: &str, hint: &str, area: Rect, theme: &Theme, buf: &mut Buffer) {
    if area.height == 0 || area.width < 4 {
        return;
    }
    // Centre the block of one to three lines.
    let lines = 1 + u16::from(!reason.is_empty()) + u16::from(!hint.is_empty());
    let y = area.y + area.height.saturating_sub(lines) / 2;
    let label = format!("▪ {kind}");
    let w = area.width.saturating_sub(2) as usize;
    buf.set_stringn(
        area.x + 1,
        y,
        &label,
        w,
        theme.style(Role::TextMuted).add_modifier(Modifier::BOLD),
    );
    let mut row = y + 1;
    if row < area.y + area.height && !reason.is_empty() {
        buf.set_stringn(area.x + 1, row, reason, w, theme.style(Role::TextMuted));
        row += 1;
    }
    if row < area.y + area.height && !hint.is_empty() {
        buf.set_stringn(area.x + 1, row, hint, w, theme.style(Role::Info));
    }
}

/// The staleness badge (§11, seam 10): drawn by the shell at the top right of
/// a tile whose source aged past three cadences, after the tile's cells were
/// dimmed through `Role::TextMuted`.
pub fn stale_badge(age_s: u64, area: Rect, theme: &Theme, buf: &mut Buffer) {
    if area.height == 0 {
        return;
    }
    let text = format!("STALE {}", stale_age_text(age_s));
    let w = text.chars().count() as u16;
    if area.width < w {
        return;
    }
    let x0 = area.x + area.width - w;
    // An explicit style per cell: `set_string` would keep whatever modifiers
    // the covered cells had (a REVERSED table header split the badge in two).
    for (i, ch) in text.chars().enumerate() {
        if let Some(cell) = buf.cell_mut((x0 + i as u16, area.y)) {
            cell.set_char(ch);
            cell.modifier = Modifier::BOLD;
            cell.set_fg(theme.color(Role::Warn));
        }
    }
}

/// `12s` under two minutes, `14m` after — the age of a stale tile.
pub fn stale_age_text(age_s: u64) -> String {
    if age_s < 120 {
        format!("{age_s}s")
    } else if age_s < 3600 * 2 {
        format!("{}m", age_s / 60)
    } else {
        format!("{}h", age_s / 3600)
    }
}

/// Dim every cell of `area` to `Role::TextMuted` (a stale tile), keeping
/// backgrounds and glyphs so the shape of the data stays readable.
pub fn dim(area: Rect, theme: &Theme, buf: &mut Buffer) {
    let fg = theme.color(Role::TextMuted);
    for y in area.y..area.y + area.height {
        for x in area.x..area.x + area.width {
            if let Some(cell) = buf.cell_mut((x, y)) {
                cell.set_fg(fg);
                // `set_style` only adds; the modifiers are edited directly.
                // `DIM` gives mono and the terminal palette — where
                // `TextMuted` may be the same colour as `Text` — a cue too.
                cell.modifier.remove(Modifier::BOLD | Modifier::REVERSED);
                cell.modifier.insert(Modifier::DIM);
            }
        }
    }
}

pub fn too_small(
    term_w: u16,
    term_h: u16,
    need_w: u16,
    need_h: u16,
    area: Rect,
    theme: &Theme,
    buf: &mut Buffer,
) {
    // Its only caller is the shell's undersized-terminal path, so say what is
    // actually happening rather than promising a stack that is not coming.
    let msg = format!("{term_w}×{term_h} — gridwatch needs at least {need_w}×{need_h}");
    let y = area.y + area.height / 2;
    let x = area.x + area.width.saturating_sub(msg.len() as u16) / 2;
    buf.set_stringn(x, y, &msg, area.width as usize, theme.style(Role::Warn));
}

pub fn help(entries: &[(&str, &str)], area: Rect, theme: &Theme, buf: &mut Buffer) {
    let w = (area.width * 2 / 3).clamp(20, 60).min(area.width);
    let h = (entries.len() as u16 + 4).min(area.height);
    let x = area.x + (area.width - w) / 2;
    let y = area.y + (area.height.saturating_sub(h)) / 2;
    let r = Rect {
        x,
        y,
        width: w,
        height: h,
    };
    fill(r, theme, buf);
    buf.set_stringn(
        x + 2,
        y + 1,
        "keys",
        (w - 4) as usize,
        theme.style(Role::Title).add_modifier(Modifier::BOLD),
    );
    for (i, (k, d)) in entries.iter().enumerate() {
        let ry = y + 3 + i as u16;
        if ry >= y + h - 1 {
            break;
        }
        buf.set_stringn(x + 2, ry, k, 10, theme.style(Role::AccentPrimary));
        buf.set_stringn(
            x + 13,
            ry,
            d,
            (w.saturating_sub(15)) as usize,
            theme.style(Role::Text),
        );
    }
}

fn fill(r: Rect, theme: &Theme, buf: &mut Buffer) {
    for y in r.y..r.y + r.height {
        for x in r.x..r.x + r.width {
            if let Some(cell) = buf.cell_mut((x, y)) {
                cell.set_char(' ');
                cell.set_style(theme.style(Role::Text).bg(theme.color(Role::Panel)));
            }
        }
    }
}

/// The alert banner row (§4.4, brief arc 3 seam 5): `Role::Crit`, bold, and
/// reversed on the even seconds so it pulses without `SLOW_BLINK`.
pub fn banner(text: &str, pulse_on: bool, row: Rect, theme: &Theme, buf: &mut Buffer) {
    if row.height == 0 || row.width == 0 {
        return;
    }
    let mut style = theme.style(Role::Crit).add_modifier(Modifier::BOLD);
    if pulse_on {
        style = style.add_modifier(Modifier::REVERSED);
    }
    for x in row.x..row.x + row.width {
        if let Some(cell) = buf.cell_mut((x, row.y)) {
            cell.set_char(' ');
            cell.set_style(style);
        }
    }
    buf.set_stringn(
        row.x + 1,
        row.y,
        text,
        row.width.saturating_sub(2) as usize,
        style,
    );
}

/// A centred panel of themed lines with a title — the alerts overlay (`A`).
pub fn panel(
    title: &str,
    lines: &[crate::view::Line],
    area: Rect,
    theme: &Theme,
    buf: &mut Buffer,
) {
    let w = (area.width * 3 / 4).clamp(20, 100).min(area.width);
    let h = (lines.len() as u16 + 4)
        .min(area.height)
        .max(4.min(area.height));
    let x = area.x + (area.width - w) / 2;
    let y = area.y + (area.height.saturating_sub(h)) / 2;
    let r = Rect {
        x,
        y,
        width: w,
        height: h,
    };
    fill(r, theme, buf);
    buf.set_stringn(
        x + 2,
        y + 1,
        title,
        (w.saturating_sub(4)) as usize,
        theme.style(Role::Title).add_modifier(Modifier::BOLD),
    );
    let inner = Rect {
        x: x + 2,
        y: y + 3,
        width: w.saturating_sub(4),
        height: h.saturating_sub(4),
    };
    if inner.height > 0 && inner.width > 0 {
        let view = crate::view::View::Text(lines.to_vec());
        theme.renderer().render(&view, inner, theme, buf);
    }
}
