//! The pin bars with tui.rs's limit line (§4.6 `View::Custom`): the bars are
//! the theme's own `View::Bars` rendering, then a dim `┄` at the overload
//! amperage over every cell the bars left empty. Styled only through the
//! theme (`Role::Crit`); describes itself for the text dumps.

use std::borrow::Cow;

use gridwatch_ui::theme::{GradientId, Role, Theme};
use gridwatch_ui::view::{Paint, View};
use ratatui_core::buffer::Buffer;
use ratatui_core::layout::Rect;

pub struct PinBars {
    pub values: Vec<f32>,
    pub peaks: Vec<f32>,
    pub labels: Option<Vec<Cow<'static, str>>>,
    /// The limit as a fraction of the bar ceiling (9.2 / 10).
    pub limit_frac: f32,
}

impl Paint for PinBars {
    fn paint(&self, area: Rect, theme: &Theme, buf: &mut Buffer) {
        if area.height == 0 || area.width == 0 {
            return;
        }
        // Six bars share the width: `bw` columns each with one gap (review:
        // one-cell bars jammed at the left edge). A zero peak draws no cap.
        let n = self.values.len().max(1) as u16;
        let bw = ((area.width + 1) / (n + 1)).max(1);
        let mut values = Vec::new();
        let mut peaks = Vec::new();
        let mut labels: Option<Vec<Cow<'static, str>>> = self.labels.as_ref().map(|_| Vec::new());
        for (i, v) in self.values.iter().enumerate() {
            if i > 0 {
                values.push(0.0);
                peaks.push(0.0);
                if let Some(l) = labels.as_mut() {
                    l.push(Cow::Borrowed(" "));
                }
            }
            for c in 0..bw {
                values.push(*v);
                peaks.push(self.peaks.get(i).copied().unwrap_or(0.0));
                if let Some(l) = labels.as_mut() {
                    l.push(if c == 0 {
                        self.labels
                            .as_ref()
                            .and_then(|ls| ls.get(i).cloned())
                            .unwrap_or(Cow::Borrowed(" "))
                    } else {
                        Cow::Borrowed(" ")
                    });
                }
            }
        }
        let bars = View::Bars {
            values,
            gradient: GradientId::Power,
            labels,
            peaks: Some(peaks),
        };
        theme.renderer().render(&bars, area, theme, buf);
        // The bar rows exclude the label row the renderer adds when it fits.
        let label_rows = u16::from(self.labels.is_some() && area.height >= 2);
        let rows = area.height - label_rows;
        if rows == 0 {
            return;
        }
        let row_from_bottom =
            ((self.limit_frac.clamp(0.0, 1.0) * f32::from(rows)).floor() as u16).min(rows - 1);
        let y = area.y + rows - 1 - row_from_bottom;
        let style = theme.style(Role::Crit);
        for x in area.x..area.x + area.width {
            if let Some(cell) = buf.cell_mut((x, y))
                && cell.symbol().trim().is_empty()
            {
                cell.set_symbol("┄");
                cell.set_style(style);
            }
        }
    }
}
