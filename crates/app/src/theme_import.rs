//! `gridwatch theme import` (§7, D59 seam 2).
//!
//! Turns someone else's colour scheme into a gridwatch theme. Three formats
//! go in — alacritty TOML, wezterm TOML, and base16/base24 YAML — and one
//! `themes/<name>.toml` comes out, which the loader accepts and the schema
//! validates.
//!
//! **It writes a file and never applies one.** A command that silently edits
//! `~/.config/gridwatch/config.toml` because you asked it to look at a colour
//! scheme is not a good citizen; the output goes to stdout unless `-o` names a
//! path, and switching to it is `theme = "..."` or `--theme`, by hand.
//!
//! What a foreign scheme actually gives us is sixteen ANSI colours plus a
//! foreground and a background. gridwatch wants nineteen roles and eight
//! gradients, so the rest is **derived**, and the derivation is written down
//! here rather than left to taste: muted and ghost text are mixes toward the
//! background, the panel is a lift away from it, and the gradients walk the
//! scheme's own ramps. The importer prints the loader's WCAG report so a
//! scheme that cannot make readable muted text says so at import time rather
//! than at first use.

use std::fmt::Write as _;
use std::path::Path;

/// A scheme reduced to what every one of the three formats can express.
#[derive(Clone, Debug, PartialEq)]
pub struct Scheme {
    pub name: String,
    pub fg: Rgb,
    pub bg: Rgb,
    /// The sixteen ANSI colours, in the usual order: black red green yellow
    /// blue magenta cyan white, then the eight bright ones.
    pub ansi: [Rgb; 16],
    /// base16's `base01`, when the source had a second background. `None` for
    /// alacritty and wezterm, which do not carry one.
    pub lift: Option<Rgb>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rgb(pub u8, pub u8, pub u8);

impl std::fmt::Display for Rgb {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "#{:02x}{:02x}{:02x}", self.0, self.1, self.2)
    }
}

impl Rgb {
    /// `#rrggbb`, `0xrrggbb`, `rrggbb` — every spelling the three formats use.
    pub fn parse(s: &str) -> Option<Rgb> {
        let h = s
            .trim()
            .trim_matches(|c| c == '"' || c == '\'')
            .trim_start_matches('#')
            .trim_start_matches("0x")
            .trim_start_matches("0X");
        if h.len() != 6 || !h.chars().all(|c| c.is_ascii_hexdigit()) {
            return None;
        }
        let byte = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).ok();
        Some(Rgb(byte(0)?, byte(2)?, byte(4)?))
    }

    /// `t` of the way from `self` to `other`, in sRGB. Not perceptual — the
    /// two ends are a text colour and its own background, and the point is a
    /// predictable step, not a colour-science claim.
    fn mix(self, other: Rgb, t: f32) -> Rgb {
        let m = |a: u8, b: u8| (f32::from(a) + (f32::from(b) - f32::from(a)) * t).round() as u8;
        Rgb(m(self.0, other.0), m(self.1, other.1), m(self.2, other.2))
    }

    fn luma(self) -> f32 {
        (0.2126 * f32::from(self.0) + 0.7152 * f32::from(self.1) + 0.0722 * f32::from(self.2))
            / 255.0
    }
}

#[derive(Debug)]
pub struct ImportError(pub String);

impl std::fmt::Display for ImportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ImportError {}

type R<T> = Result<T, ImportError>;

/// Read a scheme from a file, deciding the format from its contents rather
/// than its extension: people rename these files, and `.toml` covers two of
/// the three.
pub fn read(path: &Path) -> R<Scheme> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| ImportError(format!("{}: {e}", path.display())))?;
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "imported".into());
    parse(&text, &stem)
}

/// The same, from text. `fallback_name` is used when the scheme does not name
/// itself.
pub fn parse(text: &str, fallback_name: &str) -> R<Scheme> {
    if let Ok(table) = text.parse::<toml::Table>() {
        if let Some(s) = alacritty(&table, fallback_name) {
            return Ok(s);
        }
        if let Some(s) = wezterm(&table, fallback_name) {
            return Ok(s);
        }
    }
    if let Some(s) = base16(text, fallback_name) {
        return Ok(s);
    }
    Err(ImportError(
        "not a colour scheme this knows. It reads an alacritty `[colors]` \
         table, a wezterm `[colors]` table, or a base16/base24 YAML scheme \
         (base00…base0F)."
            .into(),
    ))
}

fn colour(v: Option<&toml::Value>) -> Option<Rgb> {
    Rgb::parse(v?.as_str()?)
}

/// alacritty: `[colors.primary] foreground/background`, `[colors.normal]` and
/// `[colors.bright]` by colour name.
fn alacritty(t: &toml::Table, fallback: &str) -> Option<Scheme> {
    let colors = t.get("colors")?.as_table()?;
    let primary = colors.get("primary")?.as_table()?;
    let normal = colors.get("normal")?.as_table()?;
    let bright = colors.get("bright")?.as_table()?;
    const NAMES: [&str; 8] = [
        "black", "red", "green", "yellow", "blue", "magenta", "cyan", "white",
    ];
    let mut ansi = [Rgb(0, 0, 0); 16];
    for (i, n) in NAMES.iter().enumerate() {
        ansi[i] = colour(normal.get(*n))?;
        ansi[i + 8] = colour(bright.get(*n))?;
    }
    Some(Scheme {
        name: name_in(t).unwrap_or_else(|| fallback.to_string()),
        fg: colour(primary.get("foreground"))?,
        bg: colour(primary.get("background"))?,
        ansi,
        lift: None,
    })
}

/// wezterm: `[colors] foreground/background`, `ansi = [8]`, `brights = [8]`.
fn wezterm(t: &toml::Table, fallback: &str) -> Option<Scheme> {
    let colors = t.get("colors")?.as_table()?;
    let list = |key: &str| -> Option<Vec<Rgb>> {
        let a = colors.get(key)?.as_array()?;
        a.iter().map(|v| Rgb::parse(v.as_str()?)).collect()
    };
    let normal = list("ansi")?;
    let bright = list("brights")?;
    if normal.len() != 8 || bright.len() != 8 {
        return None;
    }
    let mut ansi = [Rgb(0, 0, 0); 16];
    ansi[..8].copy_from_slice(&normal);
    ansi[8..].copy_from_slice(&bright);
    Some(Scheme {
        name: name_in(t).unwrap_or_else(|| fallback.to_string()),
        fg: colour(colors.get("foreground"))?,
        bg: colour(colors.get("background"))?,
        ansi,
        lift: None,
    })
}

fn name_in(t: &toml::Table) -> Option<String> {
    for key in ["name", "scheme"] {
        if let Some(s) = t.get(key).and_then(|v| v.as_str()) {
            return Some(s.to_string());
        }
    }
    let meta = t.get("metadata")?.as_table()?;
    Some(meta.get("name")?.as_str()?.to_string())
}

/// base16 / base24 YAML.
///
/// A scheme file is a flat map of scalars, so this reads `key: value` lines
/// and nothing more — no anchors, no nesting, no flow collections. That is the
/// whole of the format as it is actually published, and it is a great deal
/// less than a YAML dependency.
fn base16(text: &str, fallback: &str) -> Option<Scheme> {
    let mut map = std::collections::BTreeMap::new();
    for line in text.lines() {
        let line = line.split('#').next().unwrap_or(line);
        let Some((k, v)) = line.split_once(':') else {
            continue;
        };
        let k = k.trim();
        let v = v.trim().trim_matches(|c| c == '"' || c == '\'');
        if k.is_empty() || v.is_empty() || k.contains(' ') {
            continue;
        }
        map.insert(k.to_string(), v.to_string());
    }
    // The `palette:` nesting base16 v2 uses puts the colours one level in;
    // the flat reader above sees the same keys either way.
    let base = |n: u8| -> Option<Rgb> { Rgb::parse(map.get(&format!("base{n:02X}"))?) };
    let b: Vec<Rgb> = (0..16).map(base).collect::<Option<Vec<_>>>()?;
    // base16's own mapping onto a terminal palette, as every base16 terminal
    // template writes it.
    let ansi = [
        b[0x0], b[0x8], b[0xB], b[0xA], b[0xD], b[0xE], b[0xC], b[0x5], //
        b[0x3], b[0x8], b[0xB], b[0xA], b[0xD], b[0xE], b[0xC], b[0x7],
    ];
    Some(Scheme {
        name: map
            .get("scheme")
            .or_else(|| map.get("name"))
            .cloned()
            .unwrap_or_else(|| fallback.to_string()),
        fg: b[0x5],
        bg: b[0x0],
        ansi,
        lift: Some(b[0x1]),
    })
}

/// The scheme as a gridwatch theme file.
///
/// Everything gridwatch needs beyond the sixteen colours is derived here, and
/// each derivation is one line so it can be argued with:
/// * `panel` / `surface` lift away from the background (base16's `base01`
///   when the scheme has one, else a step toward the foreground),
/// * `text_muted` and `text_ghost` are the foreground mixed toward the
///   background — the two roles a foreign scheme never provides,
/// * accents are the bright cyan / magenta / blue, which is what a terminal
///   scheme's designer tuned for highlights,
/// * gradients walk the scheme's own ramps rather than inventing hues.
pub fn to_theme(s: &Scheme, name: &str) -> String {
    let dark = s.bg.luma() < s.fg.luma();
    let lift = s
        .lift
        .unwrap_or_else(|| s.bg.mix(s.fg, if dark { 0.10 } else { 0.06 }));
    let muted = s.fg.mix(s.bg, 0.35);
    let ghost = s.fg.mix(s.bg, 0.70);
    let a = |i: usize| s.ansi[i];
    let mut out = String::new();
    let _ = write!(
        out,
        "# Imported by `gridwatch theme import` from `{src}` — edit freely.\n\
         #\n\
         # A foreign scheme carries sixteen colours, a foreground and a\n\
         # background. Everything else here is derived (see the importer's\n\
         # module docs): `panel`/`surface` lift off the background,\n\
         # `text_muted` and `text_ghost` are the foreground mixed toward it,\n\
         # and the gradients walk this scheme's own ramps.\n\
         [meta]\n\
         name = \"{name}\"\n\
         schema = 1\n\
         variant = \"{variant}\"\n\
         \n\
         [colors]\n\
         bg = \"{bg}\"\n\
         surface = \"{surface}\"\n\
         panel = \"{panel}\"\n\
         border = \"{border}\"\n\
         border_focused = \"{focused}\"\n\
         title = \"{title}\"\n\
         text = \"{text}\"\n\
         text_muted = \"{muted}\"\n\
         text_ghost = \"{ghost}\"\n\
         cursor = \"{focused}\"\n\
         \n\
         [colors.accent]\n\
         primary = \"{a14}\"\n\
         secondary = \"{a13}\"\n\
         tertiary = \"{a12}\"\n\
         \n\
         [colors.severity]\n\
         ok = \"{a2}\"\n\
         warn = \"{a3}\"\n\
         crit = \"{a1}\"\n\
         info = \"{a6}\"\n\
         \n\
         [colors.selection]\n\
         fg = \"{bg}\"\n\
         bg = \"{a14}\"\n\
         \n\
         [gradients]\n\
         load = [\"{a2}\", \"{a3}\", \"{a11}\", \"{a1}\"]\n\
         temp = [\"{a6}\", \"{a2}\", \"{a3}\", \"{a1}\"]\n\
         power = [\"{a6}\", \"{a3}\", \"{a1}\"]\n\
         mem = [\"{a4}\", \"{a12}\", \"{a5}\"]\n\
         netrx = [\"{a6}\", \"{a14}\"]\n\
         nettx = [\"{a5}\", \"{a13}\"]\n\
         audio = [\"{a4}\", \"{a6}\", \"{a14}\", \"{a15}\"]\n\
         title = [\"{title}\", \"{a14}\"]\n\
         \n\
         [glyphs]\n\
         set = \"unicode\"\n\
         \n\
         [borders]\n\
         set = \"plain\"\n\
         focused_set = \"thick\"\n\
         \n\
         [title]\n\
         style = \"plain\"\n\
         bold = true\n\
         \n\
         [widgets]\n\
         gauge = \"bar\"\n\
         bars = \"eighths\"\n\
         sparkline = \"eighths\"\n\
         table_header = \"reverse\"\n\
         big_number = \"quadrant\"\n",
        src = s.name,
        name = name,
        variant = if dark { "dark" } else { "light" },
        bg = s.bg,
        surface = lift,
        panel = lift,
        border = a(8),
        focused = a(14),
        title = s.fg,
        text = s.fg,
        muted = muted,
        ghost = ghost,
        a1 = a(1),
        a2 = a(2),
        a3 = a(3),
        a4 = a(4),
        a5 = a(5),
        a6 = a(6),
        a11 = a(11),
        a12 = a(12),
        a13 = a(13),
        a14 = a(14),
        a15 = a(15),
    );
    out
}

/// A theme name that is a filename and a config value: lower case, `-` for
/// anything else, never empty.
pub fn slug(name: &str) -> String {
    let mut out = String::new();
    let mut dash = false;
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
            dash = false;
        } else if !out.is_empty() && !dash {
            out.push('-');
            dash = true;
        }
    }
    let out = out.trim_end_matches('-').to_string();
    if out.is_empty() {
        "imported".into()
    } else {
        out
    }
}

/// Import a file and report on the result: the theme text, and what the
/// loader thought of it.
#[derive(Debug)]
pub struct Imported {
    pub name: String,
    pub toml: String,
    /// The loader's own warnings — the WCAG gate among them.
    pub warnings: Vec<String>,
    pub contrast: Vec<String>,
}

/// Read, convert, and **load what was written**, so an import that produces a
/// theme gridwatch would refuse fails here rather than at first use.
pub fn import(path: &Path, name: Option<&str>) -> R<Imported> {
    let scheme = read(path)?;
    let name = slug(name.unwrap_or(&scheme.name));
    let toml = to_theme(&scheme, &name);
    let file = gridwatch_ui::theme::load_theme_file(&toml)
        .map_err(|e| ImportError(format!("the imported theme does not parse: {e}")))?;
    let theme = gridwatch_ui::theme::build_theme(&file, None, gridwatch_ui::ColorMode::TrueColor)
        .map_err(|e| ImportError(format!("the imported theme does not load: {e}")))?;
    Ok(Imported {
        name,
        toml,
        warnings: theme.warnings.clone(),
        contrast: theme.contrast_report(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALACRITTY: &str = r##"
[colors.primary]
background = "#1e1e2e"
foreground = "#cdd6f4"
[colors.normal]
black   = "#45475a"
red     = "#f38ba8"
green   = "#a6e3a1"
yellow  = "#f9e2af"
blue    = "#89b4fa"
magenta = "#f5c2e7"
cyan    = "#94e2d5"
white   = "#bac2de"
[colors.bright]
black   = "#585b70"
red     = "#f38ba8"
green   = "#a6e3a1"
yellow  = "#f9e2af"
blue    = "#89b4fa"
magenta = "#f5c2e7"
cyan    = "#94e2d5"
white   = "#a6adc8"
"##;

    const WEZTERM: &str = r##"
[colors]
foreground = "#c0caf5"
background = "#1a1b26"
ansi = ["#15161e", "#f7768e", "#9ece6a", "#e0af68", "#7aa2f7", "#bb9af7", "#7dcfff", "#a9b1d6"]
brights = ["#414868", "#f7768e", "#9ece6a", "#e0af68", "#7aa2f7", "#bb9af7", "#7dcfff", "#c0caf5"]
"##;

    const BASE16: &str = r##"
scheme: "Gruvbox dark, medium"
author: "Dawid Kurek"
base00: "282828"
base01: "3c3836"
base02: "504945"
base03: "665c54"
base04: "bdae93"
base05: "d5c4a1"
base06: "ebdbb2"
base07: "fbf1c7"
base08: "fb4934"
base09: "fe8019"
base0A: "fabd2f"
base0B: "b8bb26"
base0C: "8ec07c"
base0D: "83a598"
base0E: "d3869b"
base0F: "d65d0e"
"##;

    #[test]
    fn every_format_reads() {
        let a = parse(ALACRITTY, "cat").expect("alacritty");
        assert_eq!(a.bg, Rgb(0x1e, 0x1e, 0x2e));
        assert_eq!(a.ansi[1], Rgb(0xf3, 0x8b, 0xa8));
        assert_eq!(a.lift, None);

        let w = parse(WEZTERM, "tokyo").expect("wezterm");
        assert_eq!(w.fg, Rgb(0xc0, 0xca, 0xf5));
        assert_eq!(w.ansi[8], Rgb(0x41, 0x48, 0x68));

        let b = parse(BASE16, "gruvbox").expect("base16");
        assert_eq!(b.name, "Gruvbox dark, medium");
        assert_eq!(b.bg, Rgb(0x28, 0x28, 0x28));
        // base16's own terminal mapping: ansi 1 is base08.
        assert_eq!(b.ansi[1], Rgb(0xfb, 0x49, 0x34));
        assert_eq!(b.lift, Some(Rgb(0x3c, 0x38, 0x36)));
    }

    #[test]
    fn something_else_is_refused_by_name() {
        let e = parse("hello = 1\n", "x").expect_err("not a scheme");
        assert!(
            e.0.contains("alacritty") && e.0.contains("base16"),
            "{}",
            e.0
        );
    }

    /// The point of the whole command: what comes out is a theme this program
    /// will load. Anything less and the import has only moved the failure.
    #[test]
    fn what_it_writes_is_a_theme_gridwatch_loads() {
        for (text, who) in [(ALACRITTY, "cat"), (WEZTERM, "tokyo"), (BASE16, "gruv")] {
            let s = parse(text, who).expect(who);
            let toml = to_theme(&s, &slug(&s.name));
            let file = gridwatch_ui::theme::load_theme_file(&toml)
                .unwrap_or_else(|e| panic!("{who}: {e}"));
            let theme =
                gridwatch_ui::theme::build_theme(&file, None, gridwatch_ui::ColorMode::TrueColor)
                    .unwrap_or_else(|e| panic!("{who}: {e}"));
            assert_eq!(theme.name, slug(&s.name));
            // A derived muted text that fails the readability floor is the
            // thing the import is supposed to *report*, not to produce
            // silently — so assert the report exists either way.
            assert!(
                theme
                    .contrast_report()
                    .iter()
                    .any(|r| r.contains("text on panel")),
                "{who}: no contrast report"
            );
        }
    }

    #[test]
    fn derived_roles_sit_between_the_foreground_and_the_background() {
        let s = parse(WEZTERM, "tokyo").unwrap();
        let toml = to_theme(&s, "tokyo");
        // muted is nearer the text than ghost is, and both are between.
        let muted = s.fg.mix(s.bg, 0.35);
        let ghost = s.fg.mix(s.bg, 0.70);
        assert!(toml.contains(&format!("text_muted = \"{muted}\"")));
        assert!(toml.contains(&format!("text_ghost = \"{ghost}\"")));
        let d = |c: Rgb| (c.luma() - s.bg.luma()).abs();
        assert!(
            d(muted) > d(ghost),
            "ghost should be closer to the background"
        );
    }

    #[test]
    fn slugs_are_filenames() {
        assert_eq!(slug("Gruvbox dark, medium"), "gruvbox-dark-medium");
        assert_eq!(slug("  "), "imported");
        assert_eq!(slug("Solarized!!"), "solarized");
    }

    #[test]
    fn hex_is_read_in_every_spelling() {
        for s in ["#a1b2c3", "a1b2c3", "0xa1b2c3", "\"#a1b2c3\""] {
            assert_eq!(Rgb::parse(s), Some(Rgb(0xa1, 0xb2, 0xc3)), "{s}");
        }
        assert_eq!(Rgb::parse("#abc"), None);
        assert_eq!(Rgb::parse("nope!!"), None);
    }
}
