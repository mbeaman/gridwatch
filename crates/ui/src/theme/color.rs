//! Colour parsing and downsampling (§7): truecolor first, in-tree nearest-256/16.

use ratatui_core::style::Color;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ColorMode {
    #[default]
    TrueColor,
    Ansi256,
    Ansi16,
    Mono,
}

/// The sixteen terminal colours by name (loader v2, the `terminal` theme):
/// the user's own palette, whatever the terminal maps them to.
const NAMED: [(&str, Color); 16] = [
    ("black", Color::Black),
    ("red", Color::Red),
    ("green", Color::Green),
    ("yellow", Color::Yellow),
    ("blue", Color::Blue),
    ("magenta", Color::Magenta),
    ("cyan", Color::Cyan),
    ("white", Color::Gray),
    ("bright-black", Color::DarkGray),
    ("bright-red", Color::LightRed),
    ("bright-green", Color::LightGreen),
    ("bright-yellow", Color::LightYellow),
    ("bright-blue", Color::LightBlue),
    ("bright-magenta", Color::LightMagenta),
    ("bright-cyan", Color::LightCyan),
    ("bright-white", Color::White),
];

/// Parse `#rrggbb`, `default`, one of the sixteen terminal colour names
/// (`red`, `bright-red`, …) or `ansi:N` (an xterm-256 index); `$name`
/// resolution happens in the loader.
pub fn parse_color(s: &str) -> Result<Color, String> {
    let s = s.trim();
    if s.eq_ignore_ascii_case("default") {
        return Ok(Color::Reset);
    }
    if let Some((_, c)) = NAMED.iter().find(|(n, _)| n.eq_ignore_ascii_case(s)) {
        return Ok(*c);
    }
    if let Some(idx) = s.strip_prefix("ansi:") {
        return idx
            .parse::<u8>()
            .map(Color::Indexed)
            .map_err(|_| format!("expected ansi:0..=255, got '{s}'"));
    }
    let hex = s.strip_prefix('#').ok_or_else(|| {
        format!("expected #rrggbb, 'default', a terminal colour name or ansi:N, got '{s}'")
    })?;
    if hex.len() != 6 {
        return Err(format!("expected 6 hex digits, got '{s}'"));
    }
    let v = u32::from_str_radix(hex, 16).map_err(|e| format!("'{s}': {e}"))?;
    Ok(Color::Rgb((v >> 16) as u8, (v >> 8) as u8, v as u8))
}

/// Apply the colour mode to a parsed colour.
pub fn to_mode(c: Color, mode: ColorMode) -> Color {
    match (mode, c) {
        (ColorMode::TrueColor, c) => c,
        (_, Color::Reset) => Color::Reset,
        (ColorMode::Ansi256, Color::Rgb(r, g, b)) => Color::Indexed(nearest_256(r, g, b)),
        (ColorMode::Ansi16, Color::Rgb(r, g, b)) => nearest_16(r, g, b),
        (ColorMode::Mono, _) => Color::Reset,
        (_, other) => other,
    }
}

/// Nearest xterm-256 index (in-tree, ~25 lines; ansi_colours is LGPL — banned).
pub fn nearest_256(r: u8, g: u8, b: u8) -> u8 {
    // Greyscale ramp 232..=255.
    let grey_idx = ((i32::from(r) + i32::from(g) + i32::from(b)) / 3 - 8).clamp(0, 239) / 10;
    let grey_val = 8 + 10 * grey_idx;
    let grey_dist = dist(r, g, b, grey_val as u8, grey_val as u8, grey_val as u8);
    // 6×6×6 cube 16..=231.
    let q = |v: u8| -> (u8, u8) {
        let steps = [0u8, 95, 135, 175, 215, 255];
        let mut best = (0u8, u32::MAX);
        for (i, s) in steps.iter().enumerate() {
            let d = (i32::from(v) - i32::from(*s)).unsigned_abs();
            if d < best.1 {
                best = (i as u8, d);
            }
        }
        (best.0, steps[best.0 as usize])
    };
    let (ri, rv) = q(r);
    let (gi, gv) = q(g);
    let (bi, bv) = q(b);
    let cube_dist = dist(r, g, b, rv, gv, bv);
    if grey_dist < cube_dist {
        (232 + grey_idx) as u8
    } else {
        16 + 36 * ri + 6 * gi + bi
    }
}

fn dist(r: u8, g: u8, b: u8, r2: u8, g2: u8, b2: u8) -> u32 {
    let dr = i32::from(r) - i32::from(r2);
    let dg = i32::from(g) - i32::from(g2);
    let db = i32::from(b) - i32::from(b2);
    (dr * dr + dg * dg + db * db) as u32
}

pub fn nearest_16(r: u8, g: u8, b: u8) -> Color {
    const BASE: [(Color, (u8, u8, u8)); 16] = [
        (Color::Black, (0, 0, 0)),
        (Color::Red, (205, 49, 49)),
        (Color::Green, (13, 188, 121)),
        (Color::Yellow, (229, 229, 16)),
        (Color::Blue, (36, 114, 200)),
        (Color::Magenta, (188, 63, 188)),
        (Color::Cyan, (17, 168, 205)),
        (Color::Gray, (229, 229, 229)),
        (Color::DarkGray, (102, 102, 102)),
        (Color::LightRed, (241, 76, 76)),
        (Color::LightGreen, (35, 209, 139)),
        (Color::LightYellow, (245, 245, 67)),
        (Color::LightBlue, (59, 142, 234)),
        (Color::LightMagenta, (214, 112, 214)),
        (Color::LightCyan, (41, 184, 219)),
        (Color::White, (255, 255, 255)),
    ];
    BASE.iter()
        .min_by_key(|(_, (r2, g2, b2))| dist(r, g, b, *r2, *g2, *b2))
        .map(|(c, _)| *c)
        .unwrap_or(Color::Reset)
}

/// WCAG 2.1 relative luminance of an `Rgb` colour; `None` for anything the
/// gate cannot judge (`Reset`, a terminal index — the user's palette).
pub fn relative_luminance(c: Color) -> Option<f64> {
    let Color::Rgb(r, g, b) = c else { return None };
    let lin = |v: u8| {
        let c = f64::from(v) / 255.0;
        if c <= 0.03928 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    Some(0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b))
}

/// WCAG 2.1 contrast ratio (1.0–21.0) between two `Rgb` colours; `None` when
/// either is not judgeable.
pub fn contrast_ratio(a: Color, b: Color) -> Option<f64> {
    let (la, lb) = (relative_luminance(a)?, relative_luminance(b)?);
    let (hi, lo) = if la > lb { (la, lb) } else { (lb, la) };
    Some((hi + 0.05) / (lo + 0.05))
}
