//! Theme file loading (§7, loader v1): roles, `$palette`, gradients, glyph
//! tiers, borders, titles, widgets, `class`. Self-contained until `inherits`
//! lands (D37); `[flourish]`/`[effects]`/`[ambient]`/`[components]` and
//! `inherits` are parsed and ignored with one warning each.

use std::collections::BTreeMap;

use ratatui_core::style::Color;
use serde::Deserialize;

use super::color::{ColorMode, parse_color, to_mode};
use super::gradient::Gradient;
use super::{
    BarStyle, BorderKind, BorderSpec, GRADIENTS, GaugeStyle, GlyphSet, GlyphTier, HeaderStyle,
    PerfClass, PixelStyle, Role, Theme, ThemeError, TitleSpec, TitleStyle, WidgetSet,
};

#[derive(Debug, Deserialize)]
pub struct ThemeFile {
    pub meta: Meta,
    #[serde(default)]
    pub palette: BTreeMap<String, String>,
    pub colors: ColorsSect,
    pub gradients: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    pub glyphs: GlyphsSect,
    #[serde(default)]
    pub borders: BordersSect,
    #[serde(default)]
    pub title: TitleSect,
    #[serde(default)]
    pub widgets: WidgetsSect,
    /// Tables loader v1 knows about but ignores (flourish, effects, ambient,
    /// components) plus anything unknown — each produces one warning.
    #[serde(flatten)]
    pub extra: toml::Table,
}

#[derive(Debug, Deserialize)]
pub struct Meta {
    pub name: String,
    pub schema: u32,
    #[serde(default)]
    pub variant: Option<String>,
    #[serde(default)]
    pub inherits: Option<String>,
    #[serde(default)]
    pub class: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ColorsSect {
    pub bg: String,
    pub surface: String,
    #[serde(default)]
    pub panel: Option<String>,
    pub border: String,
    pub border_focused: String,
    pub title: String,
    pub text: String,
    pub text_muted: String,
    pub text_ghost: String,
    pub cursor: String,
    pub accent: AccentSect,
    pub severity: SeveritySect,
    pub selection: SelectionSect,
}

#[derive(Debug, Deserialize)]
pub struct AccentSect {
    pub primary: String,
    pub secondary: String,
    pub tertiary: String,
}

#[derive(Debug, Deserialize)]
pub struct SeveritySect {
    pub ok: String,
    pub warn: String,
    pub crit: String,
    pub info: String,
}

#[derive(Debug, Deserialize)]
pub struct SelectionSect {
    pub fg: String,
    pub bg: String,
}

#[derive(Debug, Default, Deserialize)]
pub struct GlyphsSect {
    #[serde(default)]
    pub set: Option<String>,
    #[serde(default)]
    pub nerd: Option<bool>,
    #[serde(default)]
    pub bar: Option<String>,
    #[serde(default)]
    pub chart_marker: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
pub struct BordersSect {
    #[serde(default)]
    pub set: Option<String>,
    #[serde(default)]
    pub focused_set: Option<String>,
    #[serde(default)]
    pub merge: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
pub struct TitleSect {
    #[serde(default)]
    pub style: Option<String>,
    #[serde(default)]
    pub case: Option<String>,
    #[serde(default)]
    pub bold: Option<bool>,
}

#[derive(Debug, Default, Deserialize)]
pub struct WidgetsSect {
    #[serde(default)]
    pub gauge: Option<String>,
    #[serde(default)]
    pub bars: Option<String>,
    #[serde(default)]
    pub sparkline: Option<String>,
    #[serde(default)]
    pub table_header: Option<String>,
    #[serde(default)]
    pub big_number: Option<String>,
}

pub fn load_theme_file(text: &str) -> Result<ThemeFile, ThemeError> {
    toml::from_str(text).map_err(|e| ThemeError(e.to_string()))
}

/// Built-in themes, embedded at compile time (§7).
pub fn builtin(name: &str) -> Option<&'static str> {
    match name {
        "modern" => Some(include_str!("../../../../themes/modern.toml")),
        "mono" => Some(include_str!("../../../../themes/mono.toml")),
        "retrowave" => Some(include_str!("../../../../themes/retrowave.toml")),
        _ => None,
    }
}

pub const BUILTIN_THEMES: &[&str] = &["modern", "retrowave", "mono"];

fn resolve<'a>(palette: &'a BTreeMap<String, String>, v: &'a str) -> Result<&'a str, ThemeError> {
    if let Some(name) = v.strip_prefix('$') {
        palette
            .get(name)
            .map(|s| s.as_str())
            .ok_or_else(|| ThemeError(format!("palette entry '${name}' not found")))
    } else {
        Ok(v)
    }
}

fn color_of(
    palette: &BTreeMap<String, String>,
    v: &str,
    mode: ColorMode,
) -> Result<Color, ThemeError> {
    let raw = resolve(palette, v)?;
    parse_color(raw)
        .map(|c| to_mode(c, mode))
        .map_err(ThemeError)
}

fn pick<T: Copy>(
    v: &Option<String>,
    table: &[(&str, T)],
    what: &str,
) -> Result<Option<T>, ThemeError> {
    match v {
        None => Ok(None),
        Some(s) => table
            .iter()
            .find(|(k, _)| *k == s)
            .map(|(_, t)| Some(*t))
            .ok_or_else(|| ThemeError(format!("unknown {what} '{s}'"))),
    }
}

/// Build a `Theme` from a parsed file at a colour mode (§7). Every role and
/// all eight gradients are required — theme files are self-contained (D37).
pub fn build_theme(file: &ThemeFile, mode: ColorMode) -> Result<Theme, ThemeError> {
    let mut warnings = Vec::new();
    if file.meta.schema != 1 {
        return Err(ThemeError(format!(
            "unsupported theme schema {}",
            file.meta.schema
        )));
    }
    if file.meta.inherits.is_some() {
        warnings.push("`inherits` is ignored until arc 3 — the file must be self-contained".into());
    }
    for key in file.extra.keys() {
        warnings.push(format!("`[{key}]` is parsed but ignored by this build"));
    }

    let class = match file.meta.class.as_deref() {
        None | Some("quiet") => PerfClass::Quiet,
        Some("showcase") => PerfClass::Showcase,
        Some(other) => return Err(ThemeError(format!("unknown class '{other}'"))),
    };

    let p = &file.palette;
    let c = &file.colors;
    let mut colors = [Color::Reset; 19];
    let entries: [(Role, &str); 19] = [
        (Role::Bg, &c.bg),
        (Role::Surface, &c.surface),
        (Role::Panel, c.panel.as_deref().unwrap_or(&c.surface)),
        (Role::Border, &c.border),
        (Role::BorderFocused, &c.border_focused),
        (Role::Title, &c.title),
        (Role::Text, &c.text),
        (Role::TextMuted, &c.text_muted),
        (Role::TextGhost, &c.text_ghost),
        (Role::AccentPrimary, &c.accent.primary),
        (Role::AccentSecondary, &c.accent.secondary),
        (Role::AccentTertiary, &c.accent.tertiary),
        (Role::Ok, &c.severity.ok),
        (Role::Warn, &c.severity.warn),
        (Role::Crit, &c.severity.crit),
        (Role::Info, &c.severity.info),
        (Role::SelectionFg, &c.selection.fg),
        (Role::SelectionBg, &c.selection.bg),
        (Role::Cursor, &c.cursor),
    ];
    for (role, raw) in entries {
        colors[role.index()] = color_of(p, raw, mode)?;
    }

    let mut gradients: [Gradient; 8] = std::array::from_fn(|_| Gradient::from_stops(&[], mode));
    for g in GRADIENTS {
        let stops_raw = file.gradients.get(g.name()).ok_or_else(|| {
            ThemeError(format!(
                "gradient '{}' missing — theme files define all eight (D37)",
                g.name()
            ))
        })?;
        let stops: Vec<Color> = stops_raw
            .iter()
            .map(|s| {
                let raw = resolve(p, s)?;
                parse_color(raw).map_err(ThemeError)
            })
            .collect::<Result<_, _>>()?;
        gradients[g.index()] = Gradient::from_stops(&stops, mode);
    }

    let tier = match (file.glyphs.set.as_deref(), file.glyphs.nerd) {
        (_, Some(true)) => GlyphTier::Nerd,
        (Some("ascii"), _) => GlyphTier::Ascii,
        (None | Some("unicode"), _) => GlyphTier::Unicode,
        (Some(other), _) => return Err(ThemeError(format!("unknown glyph set '{other}'"))),
    };

    let border_table: &[(&str, BorderKind)] = &[
        ("plain", BorderKind::Plain),
        ("rounded", BorderKind::Rounded),
        ("double", BorderKind::Double),
        ("thick", BorderKind::Thick),
    ];
    let borders = BorderSpec {
        set: pick(&file.borders.set, border_table, "border set")?.unwrap_or(BorderKind::Rounded),
        focused: pick(&file.borders.focused_set, border_table, "border set")?
            .unwrap_or(BorderKind::Thick),
    };

    let title = TitleSpec {
        style: pick(
            &file.title.style,
            &[
                ("plain", TitleStyle::Plain),
                ("gradient", TitleStyle::Gradient),
            ],
            "title style",
        )?
        .unwrap_or(TitleStyle::Plain),
        upper: matches!(file.title.case.as_deref(), Some("upper")),
        bold: file.title.bold.unwrap_or(true),
    };

    let widgets = WidgetSet {
        gauge: pick(
            &file.widgets.gauge,
            &[
                ("bar", GaugeStyle::Bar),
                ("line", GaugeStyle::Line),
                ("block", GaugeStyle::Block),
            ],
            "gauge style",
        )?
        .unwrap_or_default(),
        bars: pick(
            &file.widgets.bars,
            &[
                ("eighths", BarStyle::Eighths),
                ("shade", BarStyle::Shade),
                ("dots", BarStyle::Dots),
            ],
            "bar style",
        )?
        .unwrap_or_default(),
        sparkline: pick(
            &file.widgets.sparkline,
            &[
                ("eighths", BarStyle::Eighths),
                ("braille", BarStyle::Eighths),
                ("shade", BarStyle::Shade),
            ],
            "sparkline style",
        )?
        .unwrap_or_default(),
        table_header: pick(
            &file.widgets.table_header,
            &[
                ("underline", HeaderStyle::Underline),
                ("reverse", HeaderStyle::Reverse),
                ("plain", HeaderStyle::Plain),
            ],
            "table header style",
        )?
        .unwrap_or_default(),
        big_number: pick(
            &file.widgets.big_number,
            &[
                ("quadrant", PixelStyle::Quadrant),
                ("sextant", PixelStyle::Sextant),
                ("full", PixelStyle::Full),
            ],
            "big number style",
        )?
        .unwrap_or_default(),
    };

    Ok(Theme::from_parts(
        file.meta.name.clone(),
        class,
        mode,
        colors,
        gradients,
        GlyphSet { tier },
        borders,
        title,
        widgets,
        warnings,
    ))
}

/// Load a built-in theme by name at a mode.
pub fn load_builtin(name: &str, mode: ColorMode) -> Result<Theme, ThemeError> {
    let text = builtin(name).ok_or_else(|| ThemeError(format!("no built-in theme '{name}'")))?;
    build_theme(&load_theme_file(text)?, mode)
}
