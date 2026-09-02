//! The album-art painter (§4.6, brief arc 6 seam 4): two vertical pixels
//! per cell drawn as `▀` — the top pixel's colour as the foreground, the
//! bottom's as the background. It lives in the **ui** crate because colour
//! is the theme's business: a component asks for it through
//! `View::Custom { paint, .. }` and never names a colour itself.
//!
//! Colour modes: TrueColor uses the pixels as they are; 256- and 16-colour
//! terminals quantise through the theme's own palettes; a theme with no
//! colour to give — `--color never`, or a palette like `mono` whose accents
//! and severities are all the text colour — draws luminance as the glyph
//! tier's block shades, so a cover still reads as an image in a theme that
//! promised no colour (review).

use gridwatch_store::keys::media::Art;
use ratatui_core::buffer::Buffer;
use ratatui_core::layout::Rect;
use ratatui_core::style::{Color, Style};

use crate::theme::{ColorMode, Role, Theme, nearest_16, nearest_256};
use crate::view::Paint;

/// Nearest-neighbour sample of `art` for cell column `x`, pixel row `py`,
/// over a rect `w × ph` pixels.
fn sample(art: &Art, x: u16, py: u16, w: u16, ph: u16) -> (u8, u8, u8) {
    if w == 0 || ph == 0 {
        return (0, 0, 0);
    }
    let sx = (u32::from(x) * u32::from(art.w) / u32::from(w)) as u16;
    let sy = (u32::from(py) * u32::from(art.h) / u32::from(ph)) as u16;
    art.pixel(
        sx.min(art.w.saturating_sub(1)),
        sy.min(art.h.saturating_sub(1)),
    )
}

/// Rec. 601 luma, 0..1.
pub fn luma(p: (u8, u8, u8)) -> f32 {
    (0.299 * f32::from(p.0) + 0.587 * f32::from(p.1) + 0.114 * f32::from(p.2)) / 255.0
}

/// The colour a pixel becomes in this theme's mode.
fn colour(theme: &Theme, p: (u8, u8, u8)) -> Color {
    match theme.mode {
        ColorMode::TrueColor => Color::Rgb(p.0, p.1, p.2),
        // The theme's own quantisers, so a cover lands in the same palette
        // every other colour does.
        ColorMode::Ansi256 => Color::Indexed(nearest_256(p.0, p.1, p.2)),
        ColorMode::Ansi16 => nearest_16(p.0, p.1, p.2),
        ColorMode::Mono => theme.color(Role::Text),
    }
}

/// Paint `art` into `area`. In a single-hue theme the cover is drawn as
/// block shades by luminance instead of colour pairs.
pub fn paint(art: &Art, area: Rect, theme: &Theme, buf: &mut Buffer) {
    if area.width == 0 || area.height == 0 || !art.is_valid() {
        return;
    }
    let ph = area.height.saturating_mul(2);
    // Luminance steps for a single-hue theme: the eighth blocks read as
    // shading when only one colour is available.
    let eighths = theme.glyphs.eighths();
    let shades: Vec<String> = std::iter::once(" ".to_string())
        .chain(eighths.iter().map(|c| c.to_string()))
        .collect();
    for cy in 0..area.height {
        for cx in 0..area.width {
            let top = sample(art, cx, cy * 2, area.width, ph);
            let bottom = sample(art, cx, cy * 2 + 1, area.width, ph);
            let Some(cell) = buf.cell_mut((area.x + cx, area.y + cy)) else {
                continue;
            };
            if theme.monochrome() {
                // One hue: luminance becomes a shade glyph.
                let l = (luma(top) + luma(bottom)) / 2.0;
                let i = ((l * shades.len() as f32).round() as usize).min(shades.len() - 1);
                cell.set_symbol(&shades[i]);
                cell.set_style(Style::new().fg(theme.color(Role::Text)));
            } else {
                cell.set_symbol("▀");
                cell.set_style(
                    Style::new()
                        .fg(colour(theme, top))
                        .bg(colour(theme, bottom)),
                );
            }
        }
    }
}

/// The `View::Custom` painter a component hands to the renderer.
pub struct ArtPainter {
    art: Art,
}

impl ArtPainter {
    pub fn new(art: Art) -> ArtPainter {
        ArtPainter { art }
    }
}

impl Paint for ArtPainter {
    fn paint(&self, area: Rect, theme: &Theme, buf: &mut Buffer) {
        paint(&self.art, area, theme, buf);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::load_builtin;

    fn art(w: u16, h: u16) -> Art {
        let mut rgb = Vec::new();
        for y in 0..h {
            for x in 0..w {
                // A left-to-right red ramp with a green top row.
                let r = (u32::from(x) * 255 / u32::from(w.max(1))) as u8;
                rgb.extend_from_slice(&[r, if y == 0 { 200 } else { 0 }, 0]);
            }
        }
        Art {
            track: 1,
            w,
            h,
            rgb,
        }
    }

    #[test]
    fn halfblocks_carry_two_pixels_per_cell() {
        let th = load_builtin("modern", ColorMode::TrueColor).unwrap();
        let a = art(8, 8);
        let mut buf = Buffer::empty(Rect::new(0, 0, 8, 4));
        paint(&a, Rect::new(0, 0, 8, 4), &th, &mut buf);
        let cell = buf.cell((0, 0)).unwrap();
        assert_eq!(cell.symbol(), "▀");
        // Row 0 of the art is the green row → the cell's fg; row 1 its bg.
        assert_eq!(cell.fg, Color::Rgb(0, 200, 0));
        assert_eq!(cell.bg, Color::Rgb(0, 0, 0));
        // The ramp runs left to right.
        let right = buf.cell((7, 0)).unwrap();
        let Color::Rgb(r, _, _) = right.fg else {
            panic!("truecolor")
        };
        assert!(r > 200, "the ramp reaches its end: {r}");
        // A smaller rect scales down without panicking.
        let mut small = Buffer::empty(Rect::new(0, 0, 3, 2));
        paint(&a, Rect::new(0, 0, 3, 2), &th, &mut small);
        assert_eq!(small.cell((2, 1)).unwrap().symbol(), "▀");
        // A bigger one scales up.
        let mut big = Buffer::empty(Rect::new(0, 0, 20, 10));
        paint(&a, Rect::new(0, 0, 20, 10), &th, &mut big);
        assert_eq!(big.cell((19, 9)).unwrap().symbol(), "▀");
    }

    #[test]
    fn quantises_for_poorer_terminals_and_shades_in_mono() {
        let a = art(8, 8);
        for mode in [ColorMode::Ansi256, ColorMode::Ansi16] {
            let th = load_builtin("modern", mode).unwrap();
            let mut buf = Buffer::empty(Rect::new(0, 0, 8, 4));
            paint(&a, Rect::new(0, 0, 8, 4), &th, &mut buf);
            let cell = buf.cell((7, 0)).unwrap();
            assert_eq!(cell.symbol(), "▀");
            assert!(
                !matches!(cell.fg, Color::Rgb(..)),
                "{mode:?} quantises: {:?}",
                cell.fg
            );
        }
        // The `mono` theme has no colour to give even in a truecolor
        // terminal, so the cover is shaded either way (review).
        for mode in [ColorMode::Mono, ColorMode::TrueColor] {
            let th = load_builtin("mono", mode).unwrap();
            assert!(th.monochrome(), "{mode:?}");
            let mut buf = Buffer::empty(Rect::new(0, 0, 8, 4));
            paint(&a, Rect::new(0, 0, 8, 4), &th, &mut buf);
            let syms: Vec<&str> = (0..8).map(|x| buf.cell((x, 1)).unwrap().symbol()).collect();
            assert!(
                syms.iter().any(|s| *s != syms[0]),
                "luminance varies: {syms:?}"
            );
            assert!(!syms.contains(&"▀"), "a mono theme shades, not halfblocks");
        }
        assert!(
            !load_builtin("modern", ColorMode::TrueColor)
                .unwrap()
                .monochrome(),
            "a colourful theme is not monochrome"
        );
    }

    #[test]
    fn nothing_is_drawn_for_an_invalid_or_empty_rect() {
        let th = load_builtin("modern", ColorMode::TrueColor).unwrap();
        let mut buf = Buffer::empty(Rect::new(0, 0, 4, 2));
        paint(&Art::default(), Rect::new(0, 0, 4, 2), &th, &mut buf);
        assert_eq!(buf.cell((0, 0)).unwrap().symbol(), " ");
        paint(&art(4, 4), Rect::new(0, 0, 0, 0), &th, &mut buf);
        assert_eq!(buf.cell((0, 0)).unwrap().symbol(), " ");
        assert!((luma((255, 255, 255)) - 1.0).abs() < 1e-6);
        assert_eq!(luma((0, 0, 0)), 0.0);
    }
}
