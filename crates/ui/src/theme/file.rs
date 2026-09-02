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
    BarStyle, BorderKind, BorderSpec, ChartMarker, GRADIENTS, GaugeStyle, GlyphSet, GlyphTier,
    HeaderStyle, PerfClass, PixelStyle, Role, Theme, ThemeError, TitleSpec, TitleStyle, WidgetSet,
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
    /// Tables the loader knows about but ignores (flourish, effects, ambient)
    /// plus anything unknown — each produces one warning.
    #[serde(flatten)]
    pub extra: toml::Table,
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
        _ => None,
    }
}

pub const BUILTIN_THEMES: &[&str] = &["retrowave", "modern", "mono", "terminal", "phosphor-green"];

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
        if !GRADIENTS.iter().any(|g| g.name() == name) {
            warnings.push(format!(
                "gradient '{name}' is not one of the eight — ignored"
            ));
        }
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

    // Per-kind derived themes (D52 §components): the base with one or more
    // gradients replaced, built once and shared; `Theme::for_kind` hands
    // them out and the shell passes the right one in `RenderCx`.
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
                    widgets,
                    Vec::new(),
                )),
            );
        }
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
    .with_raw_colors(raw))
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
/// (`phosphor-green` inherits `mono`), and the result is merged here with
/// `inherits` cleared, so a user file may inherit any built-in — the one
/// level of `inherits` is the user's (D52).
pub fn builtin_file(name: &str) -> Result<ThemeFile, ThemeError> {
    let text = builtin(name).ok_or_else(|| ThemeError(format!("no built-in theme '{name}'")))?;
    let mut file =
        load_theme_file(text).map_err(|e| ThemeError(format!("built-in {name}: {e}")))?;
    if let Some(p) = file.meta.inherits.clone() {
        let parent = load_theme_file(builtin(&p).ok_or_else(|| {
            ThemeError(format!(
                "built-in '{name}' inherits '{p}', which is not a built-in"
            ))
        })?)?;
        if parent.meta.inherits.is_some() {
            return Err(ThemeError(format!(
                "built-in '{name}' inherits '{p}', which inherits in turn — not supported"
            )));
        }
        file = merge(&file, &parent);
        file.meta.inherits = None;
    }
    Ok(file)
}

/// Load a built-in theme by name at a mode.
pub fn load_builtin(name: &str, mode: ColorMode) -> Result<Theme, ThemeError> {
    build_theme(&builtin_file(name)?, None, mode)
}
