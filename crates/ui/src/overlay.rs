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

/// A placeholder tile: the kind plus why it is not live (§6, §11).
pub fn chip(kind: &str, reason: &str, area: Rect, theme: &Theme, buf: &mut Buffer) {
    if area.height == 0 || area.width < 4 {
        return;
    }
    let y = area.y + area.height / 2;
    let label = format!("▪ {kind}");
    buf.set_stringn(
        area.x + 1,
        y,
        &label,
        area.width.saturating_sub(2) as usize,
        theme.style(Role::TextMuted).add_modifier(Modifier::BOLD),
    );
    if area.height >= 2 && !reason.is_empty() {
        buf.set_stringn(
            area.x + 1,
            y + 1,
            reason,
            area.width.saturating_sub(2) as usize,
            theme.style(Role::TextGhost),
        );
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
    let msg = format!("{term_w}×{term_h} — needs {need_w}×{need_h}; stacking");
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
