//! Theme file loading (§7, loader v2 — D52): roles, `$palette`, gradients,
//! glyph tiers, borders, titles, widgets, `class`; `inherits` one level deep
//! (the child overrides its parent key by key); `[components.<kind>]`
//! gradient overrides; the WCAG warn gate. `[flourish]`/`[effects]`/
//! `[ambient]` are parsed and ignored with one warning each until arc 4.

use std::collections::BTreeMap;
use std::sync::Arc;

use ratatui_core::style::Color;
use serde::Deserialize;

use super::color::{ColorMode, contrast_ratio, parse_color, to_mode};
use super::gradient::Gradient;
use super::{
    AmbientSpec, BarStyle, BorderKind, BorderSpec, ChartMarker, EffectHooks, EffectSpec, Flourish,
    GRADIENTS, GaugeStyle, GlyphSet, GlyphTier, HeaderStyle, Light, PerfClass, PixelStyle,
    RainGlyphs, Role, Theme, ThemeError, TitleSpec, TitleStyle, WidgetSet,
};

/// A parsed theme file. Every section is optional at parse time so a child
/// that `inherits` can override one key; `build_theme` requires the merged
/// result to be complete (every role, all eight gradients).
#[derive(Clone, Debug, Deserialize)]
pub struct ThemeFile {
    pub meta: Meta,
    #[serde(default)]
    pub palette: BTreeMap<String, String>,
    #[serde(default)]
    pub colors: ColorsSect,
    #[serde(default)]
    pub gradients: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    pub glyphs: GlyphsSect,
    #[serde(default)]
    pub borders: BordersSect,
    #[serde(default)]
    pub title: TitleSect,
    #[serde(default)]
    pub widgets: WidgetsSect,
    /// `[components.<kind>]` — per-kind gradient overrides (D52).
    #[serde(default)]
    pub components: BTreeMap<String, ComponentSect>,
    /// `[effects]` hooks (arc 4b, D54 seam 6).
    #[serde(default)]
    pub effects: EffectsSect,
    /// `[flourish]` (seam 7).
    #[serde(default)]
    pub flourish: FlourishSect,
    /// `[ambient]` (seam 10) — showcase themes only.
    #[serde(default)]
    pub ambient: Option<AmbientSect>,
    /// `[contrast]` (seam 9): `autofix` moves text roles up to the floor.
    #[serde(default)]
    pub contrast: ContrastSect,
    /// Anything unknown — each key produces one warning.
    #[serde(flatten)]
    pub extra: toml::Table,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct EffectsSect {
    #[serde(default)]
    pub startup: Option<EffectEntry>,
    #[serde(default)]
    pub theme_swap: Option<EffectEntry>,
    #[serde(default)]
    pub focus: Option<EffectEntry>,
    #[serde(default)]
    pub alert: Option<EffectEntry>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct EffectEntry {
    pub kind: String,
    #[serde(default)]
    pub duration_ms: Option<u32>,
    #[serde(default)]
    pub motion: Option<String>,
    #[serde(default)]
    pub lightness: Option<f32>,
    #[serde(default)]
    pub period_ms: Option<u32>,
    #[serde(default)]
    pub target: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct FlourishSect {
    #[serde(default)]
    pub grid_floor: Option<bool>,
    #[serde(default)]
    pub sun: Option<bool>,
    #[serde(default)]
    pub big_clock: Option<BigClockSect>,
    #[serde(default)]
    pub marquee: Option<bool>,
    /// Parsed for the arc-4 `matrix` excerpt in §9; unused (a `decode`
    /// effect is not in this build).
    #[serde(default)]
    pub decode: Option<bool>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct BigClockSect {
    pub pixel: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct AmbientSect {
    pub kind: String,
    #[serde(default)]
    pub fps: Option<u8>,
    #[serde(default)]
    pub density: Option<f32>,
    #[serde(default)]
    pub speed: Option<f32>,
    #[serde(default)]
    pub reveal: Option<Vec<String>>,
    #[serde(default)]
    pub reveal_ms: Option<u32>,
    #[serde(default)]
    pub governor: Option<bool>,
    #[serde(default)]
    pub light: LightSect,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct LightSect {
    #[serde(default)]
    pub fade_s: Option<f32>,
    #[serde(default)]
    pub trail_ms: Option<u32>,
    #[serde(default)]
    pub sweep_s: Option<f32>,
    #[serde(default)]
    pub head: Option<String>,
    #[serde(default)]
    pub floor: Option<String>,
    #[serde(default)]
    pub relight_on_update: Option<bool>,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct ContrastSect {
    #[serde(default)]
    pub autofix: Option<bool>,
}

#[derive(Clone, Debug, Deserialize)]
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

#[derive(Clone, Debug, Default, Deserialize)]
pub struct ColorsSect {
    #[serde(default)]
    pub bg: Option<String>,
    #[serde(default)]
    pub surface: Option<String>,
    #[serde(default)]
    pub panel: Option<String>,
    #[serde(default)]
    pub border: Option<String>,
    #[serde(default)]
    pub border_focused: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub text_muted: Option<String>,
    #[serde(default)]
    pub text_ghost: Option<String>,
    #[serde(default)]
    pub cursor: Option<String>,
    #[serde(default)]
    pub accent: AccentSect,
    #[serde(default)]
    pub severity: SeveritySect,
    #[serde(default)]
    pub selection: SelectionSect,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct AccentSect {
    #[serde(default)]
    pub primary: Option<String>,
    #[serde(default)]
    pub secondary: Option<String>,
    #[serde(default)]
    pub tertiary: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct SeveritySect {
    #[serde(default)]
    pub ok: Option<String>,
    #[serde(default)]
    pub warn: Option<String>,
    #[serde(default)]
    pub crit: Option<String>,
    #[serde(default)]
    pub info: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct SelectionSect {
    #[serde(default)]
    pub fg: Option<String>,
    #[serde(default)]
    pub bg: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct GlyphsSect {
    #[serde(default)]
    pub set: Option<String>,
    #[serde(default)]
    pub nerd: Option<bool>,
    #[serde(default)]
    pub bar: Option<String>,
    #[serde(default)]
    pub chart_marker: Option<String>,
    /// `rain = "katakana" | "ascii"` — the ambient layer's glyphs.
    #[serde(default)]
    pub rain: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct BordersSect {
    #[serde(default)]
    pub set: Option<String>,
    #[serde(default)]
    pub focused_set: Option<String>,
    #[serde(default)]
    pub merge: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct TitleSect {
    #[serde(default)]
    pub style: Option<String>,
    #[serde(default)]
    pub case: Option<String>,
    #[serde(default)]
    pub bold: Option<bool>,
}

#[derive(Clone, Debug, Default, Deserialize)]
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

/// `[components.<kind>]`: `gradients.<id> = [...]` overrides one gradient for
/// one component kind; anything else in the table warns (D52).
#[derive(Clone, Debug, Default, Deserialize)]
pub struct ComponentSect {
    #[serde(default)]
    pub gradients: BTreeMap<String, Vec<String>>,
    #[serde(flatten)]
    pub extra: toml::Table,
}

pub fn load_theme_file(text: &str) -> Result<ThemeFile, ThemeError> {
    toml::from_str(text).map_err(|e| {
        // `line:col: message`, like the config loader (§9) — a toast has one
        // row, not toml's three-line rendering.
        let at = e
            .span()
            .map(|s| {
                let byte = s.start.min(text.len());
                let before = &text[..byte];
                let line = before.matches('\n').count() + 1;
                let col = before.rsplit('\n').next().map_or(0, |l| l.chars().count()) + 1;
                format!("{line}:{col}: ")
            })
            .unwrap_or_default();
        ThemeError(format!("{at}{}", e.message()))
    })
}

/// Built-in themes, embedded at compile time (§7).
pub fn builtin(name: &str) -> Option<&'static str> {
    match name {
        "modern" => Some(include_str!("../../../../themes/modern.toml")),
        "mono" => Some(include_str!("../../../../themes/mono.toml")),
        "retrowave" => Some(include_str!("../../../../themes/retrowave.toml")),
        "terminal" => Some(include_str!("../../../../themes/terminal.toml")),
        "phosphor-green" => Some(include_str!("../../../../themes/phosphor-green.toml")),
        "phosphor-amber" => Some(include_str!("../../../../themes/phosphor-amber.toml")),
        "matrix" => Some(include_str!("../../../../themes/matrix.toml")),
        _ => None,
    }
}

pub const BUILTIN_THEMES: &[&str] = &[
    "retrowave",
    "modern",
    "mono",
    "terminal",
    "phosphor-green",
    "phosphor-amber",
    "matrix",
];

/// The WCAG 2.1 floors the warn gate applies (D52): body text on its two
/// backgrounds, and muted text. `TextGhost` is the decorative role — the
/// renderer fills empty bar cells and gauge tracks with it — so it is
/// reported by `contrast_report` but never warned about.
pub const WCAG_TEXT_MIN: f64 = 4.5;
pub const WCAG_MUTED_MIN: f64 = 3.0;

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

fn raw_color(palette: &BTreeMap<String, String>, v: &str) -> Result<Color, ThemeError> {
    let raw = resolve(palette, v)?;
    parse_color(raw).map_err(ThemeError)
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

fn or<T: Clone>(child: &Option<T>, parent: &Option<T>) -> Option<T> {
    child.clone().or_else(|| parent.clone())
}

/// The child over its parent, key by key (D52 §inherits): palette entries,
/// roles, gradients, glyph/border/title/widget keys and per-kind overrides
/// each fall back to the parent's when the child leaves them out. `[meta]`
/// is the child's (its `inherits` stays, so a chain is still detectable);
/// `extra` is the child's only.
pub fn merge(child: &ThemeFile, parent: &ThemeFile) -> ThemeFile {
    let mut palette = parent.palette.clone();
    palette.extend(child.palette.clone());
    let mut gradients = parent.gradients.clone();
    gradients.extend(child.gradients.clone());
    let mut components = parent.components.clone();
    for (kind, sect) in &child.components {
        let e = components.entry(kind.clone()).or_default();
        e.gradients.extend(sect.gradients.clone());
        e.extra.extend(sect.extra.clone());
    }
    let (c, p) = (&child.colors, &parent.colors);
    ThemeFile {
        // `class` selects the performance ceilings: inherited unless set.
        meta: Meta {
            class: or(&child.meta.class, &parent.meta.class),
            ..child.meta.clone()
        },
        palette,
        colors: ColorsSect {
            bg: or(&c.bg, &p.bg),
            surface: or(&c.surface, &p.surface),
            panel: or(&c.panel, &p.panel),
            border: or(&c.border, &p.border),
            border_focused: or(&c.border_focused, &p.border_focused),
            title: or(&c.title, &p.title),
            text: or(&c.text, &p.text),
            text_muted: or(&c.text_muted, &p.text_muted),
            text_ghost: or(&c.text_ghost, &p.text_ghost),
            cursor: or(&c.cursor, &p.cursor),
            accent: AccentSect {
                primary: or(&c.accent.primary, &p.accent.primary),
                secondary: or(&c.accent.secondary, &p.accent.secondary),
                tertiary: or(&c.accent.tertiary, &p.accent.tertiary),
            },
            severity: SeveritySect {
                ok: or(&c.severity.ok, &p.severity.ok),
                warn: or(&c.severity.warn, &p.severity.warn),
                crit: or(&c.severity.crit, &p.severity.crit),
                info: or(&c.severity.info, &p.severity.info),
            },
            selection: SelectionSect {
                fg: or(&c.selection.fg, &p.selection.fg),
                bg: or(&c.selection.bg, &p.selection.bg),
            },
        },
        gradients,
        glyphs: GlyphsSect {
            set: or(&child.glyphs.set, &parent.glyphs.set),
            nerd: or(&child.glyphs.nerd, &parent.glyphs.nerd),
            bar: or(&child.glyphs.bar, &parent.glyphs.bar),
            chart_marker: or(&child.glyphs.chart_marker, &parent.glyphs.chart_marker),
            rain: or(&child.glyphs.rain, &parent.glyphs.rain),
        },
        effects: EffectsSect {
            startup: or(&child.effects.startup, &parent.effects.startup),
            theme_swap: or(&child.effects.theme_swap, &parent.effects.theme_swap),
            focus: or(&child.effects.focus, &parent.effects.focus),
            alert: or(&child.effects.alert, &parent.effects.alert),
        },
        flourish: FlourishSect {
            grid_floor: or(&child.flourish.grid_floor, &parent.flourish.grid_floor),
            sun: or(&child.flourish.sun, &parent.flourish.sun),
            big_clock: or(&child.flourish.big_clock, &parent.flourish.big_clock),
            marquee: or(&child.flourish.marquee, &parent.flourish.marquee),
            decode: or(&child.flourish.decode, &parent.flourish.decode),
        },
        ambient: or(&child.ambient, &parent.ambient),
        contrast: ContrastSect {
            autofix: or(&child.contrast.autofix, &parent.contrast.autofix),
        },
        borders: BordersSect {
            set: or(&child.borders.set, &parent.borders.set),
            focused_set: or(&child.borders.focused_set, &parent.borders.focused_set),
            merge: or(&child.borders.merge, &parent.borders.merge),
        },
        title: TitleSect {
            style: or(&child.title.style, &parent.title.style),
            case: or(&child.title.case, &parent.title.case),
            bold: or(&child.title.bold, &parent.title.bold),
        },
        widgets: WidgetsSect {
            gauge: or(&child.widgets.gauge, &parent.widgets.gauge),
            bars: or(&child.widgets.bars, &parent.widgets.bars),
            sparkline: or(&child.widgets.sparkline, &parent.widgets.sparkline),
            table_header: or(&child.widgets.table_header, &parent.widgets.table_header),
            big_number: or(&child.widgets.big_number, &parent.widgets.big_number),
        },
        components,
        extra: child.extra.clone(),
    }
}

/// The one rule about `inherits` (D52): one level. `parent` is the file the
/// child's `meta.inherits` names, resolved by the caller (a built-in or a
/// sibling file); a parent that itself inherits is a chain and an error.
fn check_inherits(file: &ThemeFile, parent: Option<&ThemeFile>) -> Result<(), ThemeError> {
    match (&file.meta.inherits, parent) {
        (None, _) => Ok(()),
        (Some(name), None) => Err(ThemeError(format!(
            "'{}' inherits '{name}' but no parent was resolved",
            file.meta.name
        ))),
        (Some(name), Some(p)) if p.meta.name == file.meta.name => Err(ThemeError(format!(
            "'{}' cannot inherit itself ('{name}')",
            file.meta.name
        ))),
        (Some(_), Some(p)) if p.meta.schema != 1 => Err(ThemeError(format!(
            "parent '{}' has unsupported theme schema {}",
            p.meta.name, p.meta.schema
        ))),
        (Some(name), Some(p)) => match &p.meta.inherits {
            Some(grand) => Err(ThemeError(format!(
                "inherits chains are not supported: '{}' inherits '{name}', which inherits '{grand}'",
                file.meta.name
            ))),
            None => Ok(()),
        },
    }
}

/// Build a `Theme` from a parsed file at a colour mode, over its parent when
/// the file `inherits` one (§7, D52). The merged result must be complete:
/// every role and all eight gradients — an inherited theme gets them from
/// its parent, a self-contained one must declare them (D37).
pub fn build_theme(
    file: &ThemeFile,
    parent: Option<&ThemeFile>,
    mode: ColorMode,
) -> Result<Theme, ThemeError> {
    check_inherits(file, parent)?;
    let merged;
    let file = match parent {
        Some(p) => {
            merged = merge(file, p);
            &merged
        }
        None => file,
    };
    let mut warnings = Vec::new();
    if file.meta.schema != 1 {
        return Err(ThemeError(format!(
            "unsupported theme schema {}",
            file.meta.schema
        )));
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
    let need = |v: &Option<String>, key: &str| -> Result<String, ThemeError> {
        v.clone().ok_or_else(|| {
            ThemeError(format!(
                "colors.{key} missing — a theme declares every role or inherits one that does (D37, D52)"
            ))
        })
    };
    let surface = need(&c.surface, "surface")?;
    let entries: [(Role, String); 19] = [
        (Role::Bg, need(&c.bg, "bg")?),
        (Role::Surface, surface.clone()),
        (Role::Panel, c.panel.clone().unwrap_or(surface)),
        (Role::Border, need(&c.border, "border")?),
        (
            Role::BorderFocused,
            need(&c.border_focused, "border_focused")?,
        ),
        (Role::Title, need(&c.title, "title")?),
        (Role::Text, need(&c.text, "text")?),
        (Role::TextMuted, need(&c.text_muted, "text_muted")?),
        (Role::TextGhost, need(&c.text_ghost, "text_ghost")?),
        (
            Role::AccentPrimary,
            need(&c.accent.primary, "accent.primary")?,
        ),
        (
            Role::AccentSecondary,
            need(&c.accent.secondary, "accent.secondary")?,
        ),
        (
            Role::AccentTertiary,
            need(&c.accent.tertiary, "accent.tertiary")?,
        ),
        (Role::Ok, need(&c.severity.ok, "severity.ok")?),
        (Role::Warn, need(&c.severity.warn, "severity.warn")?),
        (Role::Crit, need(&c.severity.crit, "severity.crit")?),
        (Role::Info, need(&c.severity.info, "severity.info")?),
        (Role::SelectionFg, need(&c.selection.fg, "selection.fg")?),
        (Role::SelectionBg, need(&c.selection.bg, "selection.bg")?),
        (Role::Cursor, need(&c.cursor, "cursor")?),
    ];
    // Raw (pre-mode) colours judge contrast; the mode-mapped ones are drawn.
    let mut raw = [Color::Reset; 19];
    let mut colors = [Color::Reset; 19];
    for (role, value) in &entries {
        let rc = raw_color(p, value)?;
        raw[role.index()] = rc;
        colors[role.index()] = to_mode(rc, mode);
    }
    if file.contrast.autofix == Some(true) {
        for (role, fixed) in autofix(&raw, &mut warnings) {
            raw[role.index()] = fixed;
            colors[role.index()] = to_mode(fixed, mode);
        }
    }
    warnings.extend(wcag_warnings(&raw));

    let stops_of = |stops_raw: &[String]| -> Result<Vec<Color>, ThemeError> {
        stops_raw.iter().map(|s| raw_color(p, s)).collect()
    };
    let mut gradients: [Gradient; 8] = std::array::from_fn(|_| Gradient::from_stops(&[], mode));
    for g in GRADIENTS {
        let stops_raw = file.gradients.get(g.name()).ok_or_else(|| {
            ThemeError(format!(
                "gradient '{}' missing — a theme defines all eight or inherits them (D37, D52)",
                g.name()
            ))
        })?;
        gradients[g.index()] = Gradient::from_stops(&stops_of(stops_raw)?, mode);
    }
    for name in file.gradients.keys() {
        if name != "rain" && !GRADIENTS.iter().any(|g| g.name() == name) {
            warnings.push(format!(
                "gradient '{name}' is not one of the eight — ignored"
            ));
        }
    }
    let rain = match file.gradients.get("rain") {
        Some(stops_raw) => Some(Gradient::from_stops(&stops_of(stops_raw)?, mode)),
        None => None,
    };

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

    let glyphs = GlyphSet {
        tier,
        marker: pick(
            &file.glyphs.chart_marker,
            &[
                ("braille", ChartMarker::Braille),
                // §7 names it; VTE's native octants are arc 4's
                // renderer work — braille until then, never an error.
                ("octant_if_vte", ChartMarker::Braille),
                ("block", ChartMarker::Block),
                ("dot", ChartMarker::Dot),
            ],
            "chart marker",
        )?
        .unwrap_or_default(),
    };

    // The 4b tables: hooks, flourishes, the ambient layer (seams 6, 7, 10).
    let effects = parse_effects(&file.effects, &mut warnings);
    let flourish = Flourish {
        grid_floor: file.flourish.grid_floor.unwrap_or(false),
        sun: file.flourish.sun.unwrap_or(false),
        big_clock: match &file.flourish.big_clock {
            None => None,
            Some(b) => pick(
                &Some(b.pixel.clone()),
                &[
                    ("quadrant", PixelStyle::Quadrant),
                    ("sextant", PixelStyle::Sextant),
                    ("full", PixelStyle::Full),
                ],
                "big clock pixel",
            )?,
        },
        marquee: file.flourish.marquee.unwrap_or(false),
    };
    let rain_glyphs = pick(
        &file.glyphs.rain,
        &[
            ("katakana", RainGlyphs::Katakana),
            ("ascii", RainGlyphs::Ascii),
        ],
        "rain glyph set",
    )?
    .unwrap_or_default();
    let ambient = match &file.ambient {
        None => None,
        Some(_) if class != PerfClass::Showcase => {
            warnings.push(format!(
                "`[ambient]` needs `class = \"showcase\"` — ignored in the quiet theme '{}'",
                file.meta.name
            ));
            None
        }
        Some(_) if rain.is_none() => {
            warnings.push(
                "`[ambient]` needs a `rain` gradient for its palette — the layer stays off".into(),
            );
            None
        }
        Some(a) if a.kind != "matrix_rain" => {
            warnings.push(format!(
                "unknown ambient kind '{}' — the layer stays off",
                a.kind
            ));
            None
        }
        Some(a) => {
            let d = Light::default();
            Some(AmbientSpec {
                kind: a.kind.clone(),
                fps: a.fps.unwrap_or(24).clamp(1, 60),
                density: a.density.unwrap_or(0.20).clamp(0.01, 1.0),
                speed: a.speed.unwrap_or(1.0).clamp(0.1, 4.0),
                reveal: a.reveal.clone().unwrap_or_else(|| {
                    ["focus", "alert", "hover", "key"]
                        .iter()
                        .map(|s| s.to_string())
                        .collect()
                }),
                reveal_ms: a.reveal_ms.unwrap_or(2500),
                governor: a.governor.unwrap_or(true),
                light: Light {
                    fade_s: a.light.fade_s.unwrap_or(d.fade_s).max(0.5),
                    trail_ms: a.light.trail_ms.unwrap_or(d.trail_ms).max(50),
                    sweep_s: a.light.sweep_s.unwrap_or(d.sweep_s).max(2.0),
                    head: match &a.light.head {
                        Some(c) => raw_color(p, c)?,
                        None => d.head,
                    },
                    floor: match &a.light.floor {
                        Some(c) => raw_color(p, c)?,
                        None => d.floor,
                    },
                    relight_on_update: a.light.relight_on_update.unwrap_or(true),
                },
            })
        }
    };

    // Per-kind derived themes (D52 §components): the base with one or more
    // gradients replaced, built once and shared; `Theme::for_kind` hands
    // them out and the shell passes the right one in `RenderCx`. The clock
    // kind also takes `[flourish] big_clock.pixel` as its `big_number`.
    let mut kinds = BTreeMap::new();
    for (kind, sect) in &file.components {
        for key in sect.extra.keys() {
            warnings.push(format!(
                "`[components.{kind}] {key}` is not an override this build knows — ignored"
            ));
        }
        let mut derived = gradients.clone();
        let mut any = false;
        for (name, stops_raw) in &sect.gradients {
            match GRADIENTS.iter().find(|g| g.name() == name) {
                Some(g) => {
                    derived[g.index()] = Gradient::from_stops(&stops_of(stops_raw)?, mode);
                    any = true;
                }
                None => warnings.push(format!(
                    "`[components.{kind}] gradients.{name}` is not one of the eight — ignored"
                )),
            }
        }
        let mut w = widgets;
        if kind == "clock"
            && let Some(px) = flourish.big_clock
        {
            w.big_number = px;
            any = true;
        }
        if any {
            kinds.insert(
                kind.clone(),
                Arc::new(Theme::from_parts(
                    file.meta.name.clone(),
                    class,
                    mode,
                    colors,
                    derived,
                    glyphs,
                    borders,
                    title,
                    w,
                    Vec::new(),
                )),
            );
        }
    }
    if let Some(px) = flourish.big_clock
        && !kinds.contains_key("clock")
    {
        let mut w = widgets;
        w.big_number = px;
        kinds.insert(
            "clock".to_string(),
            Arc::new(Theme::from_parts(
                file.meta.name.clone(),
                class,
                mode,
                colors,
                gradients.clone(),
                glyphs,
                borders,
                title,
                w,
                Vec::new(),
            )),
        );
    }

    Ok(Theme::from_parts(
        file.meta.name.clone(),
        class,
        mode,
        colors,
        gradients,
        glyphs,
        borders,
        title,
        widgets,
        warnings,
    )
    .with_kinds(kinds)
    .with_raw_colors(raw)
    .with_ambience(effects, flourish, ambient, rain, rain_glyphs))
}

/// The effect kinds this build maps (D54 seam 6); anything else warns.
pub const EFFECT_KINDS: &[&str] = &["sweep_in", "fade_in", "dissolve", "fade", "hsl_pulse"];

fn parse_effects(sect: &EffectsSect, warnings: &mut Vec<String>) -> EffectHooks {
    let one =
        |hook: &str, e: &Option<EffectEntry>, warnings: &mut Vec<String>| -> Option<EffectSpec> {
            let e = e.as_ref()?;
            if !EFFECT_KINDS.contains(&e.kind.as_str()) {
                warnings.push(format!(
                    "`[effects] {hook}` kind '{}' is not one of {} — ignored",
                    e.kind,
                    EFFECT_KINDS.join("/")
                ));
                return None;
            }
            Some(EffectSpec {
                kind: e.kind.clone(),
                // Event effects are bounded to 600 ms (seam 6).
                duration_ms: e.duration_ms.unwrap_or(400).min(600),
                motion: e.motion.clone(),
                lightness: e.lightness,
                period_ms: e.period_ms,
                target: e.target.clone(),
            })
        };
    EffectHooks {
        startup: one("startup", &sect.startup, warnings),
        theme_swap: one("theme_swap", &sect.theme_swap, warnings),
        focus: one("focus", &sect.focus, warnings),
        alert: one("alert", &sect.alert, warnings),
    }
}

/// `[contrast] autofix` (seam 9): move `text` / `text_muted` toward their
/// WCAG floor in Oklab lightness steps of 0.02 (lighter on a dark theme,
/// darker on a light one — decided by the panel's luminance), at most 20
/// steps; each change is a warning so the author sees it.
pub fn autofix(raw: &[Color; 19], warnings: &mut Vec<String>) -> Vec<(Role, Color)> {
    use palette::{IntoColor, Oklab, Srgb, convert::FromColorUnclamped};
    let mut out = Vec::new();
    let panel = raw[Role::Panel.index()];
    let surface = raw[Role::Surface.index()];
    let Some(panel_l) = super::color::relative_luminance(panel) else {
        return out;
    };
    let lighten = panel_l < 0.5;
    for (role, floor) in [
        (Role::Text, WCAG_TEXT_MIN),
        (Role::TextMuted, WCAG_MUTED_MIN),
    ] {
        let Color::Rgb(r, g, b) = raw[role.index()] else {
            continue;
        };
        let ratio = |c: Color| {
            let a = contrast_ratio(c, panel).unwrap_or(21.0);
            let s = contrast_ratio(c, surface).unwrap_or(21.0);
            a.min(s)
        };
        let original = Color::Rgb(r, g, b);
        if ratio(original) >= floor {
            continue;
        }
        let srgb = Srgb::new(
            f32::from(r) / 255.0,
            f32::from(g) / 255.0,
            f32::from(b) / 255.0,
        );
        let mut lab: Oklab = Oklab::from_color_unclamped(srgb.into_linear());
        let mut fixed = original;
        for _ in 0..20 {
            lab.l = if lighten {
                (lab.l + 0.02).min(1.0)
            } else {
                (lab.l - 0.02).max(0.0)
            };
            let s: Srgb = palette::LinSrgb::from_color_unclamped(lab).into_color();
            fixed = Color::Rgb(
                (s.red.clamp(0.0, 1.0) * 255.0) as u8,
                (s.green.clamp(0.0, 1.0) * 255.0) as u8,
                (s.blue.clamp(0.0, 1.0) * 255.0) as u8,
            );
            if ratio(fixed) >= floor {
                break;
            }
        }
        let hex = |c: Color| match c {
            Color::Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
            other => format!("{other:?}"),
        };
        warnings.push(format!(
            "WCAG autofix: {} {} → {} ({:.2}:1 on panel)",
            role_key(role),
            hex(original),
            hex(fixed),
            contrast_ratio(fixed, panel).unwrap_or(0.0)
        ));
        out.push((role, fixed));
    }
    out
}

/// The pairs the gate judges: `(foreground, background, floor)`.
fn wcag_pairs() -> [(Role, Role, f64); 4] {
    [
        (Role::Text, Role::Panel, WCAG_TEXT_MIN),
        (Role::Text, Role::Surface, WCAG_TEXT_MIN),
        (Role::TextMuted, Role::Panel, WCAG_MUTED_MIN),
        (Role::TextMuted, Role::Surface, WCAG_MUTED_MIN),
    ]
}

/// WCAG warn gate (D52): a pair below its floor produces one warning — the
/// loader warns, it never fails. Pairs the gate cannot judge (`default`, a
/// terminal colour name — the user's own palette) say nothing.
pub fn wcag_warnings(raw: &[Color; 19]) -> Vec<String> {
    let mut out = Vec::new();
    for (fg, bg, floor) in wcag_pairs() {
        if let Some(ratio) = contrast_ratio(raw[fg.index()], raw[bg.index()])
            && ratio < floor
        {
            out.push(format!(
                "WCAG: {} on {} is {ratio:.2}:1 (below {floor:.1}:1)",
                role_key(fg),
                role_key(bg)
            ));
        }
    }
    out
}

/// Every judged pair with its ratio — `config check --theme` prints it,
/// `TextGhost` included as information (it has no floor).
pub fn contrast_report(raw: &[Color; 19]) -> Vec<String> {
    let mut rows: Vec<(Role, Role, Option<f64>)> = wcag_pairs()
        .iter()
        .map(|(f, b, floor)| (*f, *b, Some(*floor)))
        .collect();
    rows.push((Role::TextGhost, Role::Panel, None));
    rows.push((Role::TextGhost, Role::Surface, None));
    rows.into_iter()
        .map(|(fg, bg, floor)| {
            let ratio = contrast_ratio(raw[fg.index()], raw[bg.index()]);
            let verdict = match (ratio, floor) {
                (None, _) => "n/a (terminal palette)".to_string(),
                (Some(r), Some(f)) if r < f => format!("{r:.2}:1  WARN (below {f:.1}:1)"),
                (Some(r), Some(f)) => format!("{r:.2}:1  ok (floor {f:.1}:1)"),
                (Some(r), None) => format!("{r:.2}:1  info (no floor: decorative role)"),
            };
            format!("{} on {}: {verdict}", role_key(fg), role_key(bg))
        })
        .collect()
}

fn role_key(r: Role) -> &'static str {
    match r {
        Role::Text => "text",
        Role::TextMuted => "text_muted",
        Role::TextGhost => "text_ghost",
        Role::Panel => "panel",
        Role::Surface => "surface",
        other => {
            // Only the six above are judged; keep the match total for the
            // compiler without inventing keys for the rest.
            let _ = other;
            "role"
        }
    }
}

/// A built-in, **flattened**: a built-in may inherit another built-in
/// (`phosphor-green` inherits `mono`, `matrix` inherits `phosphor-green`),
/// and the chain is merged here — built-ins are embedded and finite, so the
/// one level of `inherits` D52 allows is the user's, not the binary's.
pub fn builtin_file(name: &str) -> Result<ThemeFile, ThemeError> {
    builtin_file_depth(name, 0)
}

fn builtin_file_depth(name: &str, depth: u8) -> Result<ThemeFile, ThemeError> {
    if depth > 4 {
        return Err(ThemeError(format!(
            "built-in '{name}': inherits chain too deep"
        )));
    }
    let text = builtin(name).ok_or_else(|| ThemeError(format!("no built-in theme '{name}'")))?;
    let mut file =
        load_theme_file(text).map_err(|e| ThemeError(format!("built-in {name}: {e}")))?;
    if let Some(p) = file.meta.inherits.clone() {
        if builtin(&p).is_none() {
            return Err(ThemeError(format!(
                "built-in '{name}' inherits '{p}', which is not a built-in"
            )));
        }
        let parent = builtin_file_depth(&p, depth + 1)?;
        file = merge(&file, &parent);
        file.meta.inherits = None;
    }
    Ok(file)
}

/// Load a built-in theme by name at a mode.
pub fn load_builtin(name: &str, mode: ColorMode) -> Result<Theme, ThemeError> {
    build_theme(&builtin_file(name)?, None, mode)
}
