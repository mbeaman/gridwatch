//! Config loading (§9): two files, layered defaults ← file ← `GRIDWATCH_*` env
//! ← CLI; strict parsing with spans; the layout file is the only one edit mode
//! will ever write (arc 4).

use std::path::PathBuf;

use gridwatch_ui::component::Size;
use gridwatch_ui::layout::{BorderMode, GridSpec, Page, PlaceTarget, Placement};
use serde::Deserialize;

pub const DEFAULT_CONFIG: &str = include_str!("defaults/config.toml");
pub const DEFAULT_LAYOUT: &str = include_str!("defaults/layout.toml");

#[derive(Debug)]
pub struct ConfigError(pub String);

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "config: {}", self.0)
    }
}

impl std::error::Error for ConfigError {}

fn parse<T: serde::de::DeserializeOwned>(what: &str, text: &str) -> Result<T, ConfigError> {
    toml::from_str(text).map_err(|e| {
        let span = e
            .span()
            .map(|s| format!(" (bytes {}..{})", s.start, s.end))
            .unwrap_or_default();
        ConfigError(format!("{what}{span}: {e}"))
    })
}

#[derive(Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct ConfigFile {
    pub schema: u32,
    pub theme: String,
    pub fps: u16,
    pub fps_max: u16,
    pub color: String,
    pub mouse: bool,
    pub readonly: bool,
    pub confirm_kill: bool,
    pub store: StoreSect,
    pub effects: EffectsSect,
    pub perf: PerfSect,
    #[serde(rename = "sources")]
    pub sources: toml::Table,
    #[serde(rename = "components")]
    pub components: Vec<InstanceSect>,
    /// Parsed and ignored until the journal arc (§9): recording config.
    pub record: toml::Table,
    /// Parsed and ignored until the rules arc (§9): alert rules.
    pub rules: Vec<toml::Table>,
}

impl Default for ConfigFile {
    // Structural, NOT parsed from `DEFAULT_CONFIG`: serde's container-level
    // `default` calls `Self::default()` inside `visit_map`, so a Default that
    // parses TOML recurses forever. A test below pins TOML == this.
    fn default() -> ConfigFile {
        let mut cpu = toml::Table::new();
        cpu.insert("refresh_ms".into(), toml::Value::Integer(1500));
        let mut sources = toml::Table::new();
        sources.insert("cpu".into(), toml::Value::Table(cpu));
        let inst = |id: &str, kind: &str| InstanceSect {
            id: id.into(),
            kind: kind.into(),
            options: toml::Table::new(),
        };
        ConfigFile {
            schema: 1,
            theme: "retrowave".into(),
            fps: 30,
            fps_max: 60,
            color: "auto".into(),
            mouse: true,
            readonly: false,
            confirm_kill: true,
            store: StoreSect::default(),
            effects: EffectsSect::default(),
            perf: PerfSect::default(),
            sources,
            record: toml::Table::new(),
            rules: Vec::new(),
            components: vec![
                inst("cpu", "htop"),
                inst("gpu", "gpu"),
                inst("pins", "pins"),
                inst("lan", "net"),
                inst("viz", "audio"),
                inst("amp", "winamp"),
                inst("temps", "sensors"),
            ],
        }
    }
}

#[derive(Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct StoreSect {
    pub history: String,
    pub max_mb: u32,
}

impl Default for StoreSect {
    fn default() -> StoreSect {
        StoreSect {
            history: "10m".into(),
            max_mb: 32,
        }
    }
}

#[derive(Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct EffectsSect {
    pub enabled: bool,
    pub budget_ms: u32,
}

impl Default for EffectsSect {
    fn default() -> EffectsSect {
        EffectsSect {
            enabled: true,
            budget_ms: 4,
        }
    }
}

#[derive(Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct PerfSect {
    pub unfocused_fps: u16,
    pub phase_ms: u64,
}

impl Default for PerfSect {
    fn default() -> PerfSect {
        PerfSect {
            unfocused_fps: 2,
            phase_ms: 250,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstanceSect {
    pub id: String,
    pub kind: String,
    #[serde(default)]
    pub options: toml::Table,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LayoutFile {
    pub schema: u32,
    pub grid: GridSect,
    pub pages: Vec<PageSect>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct GridSect {
    pub columns: u8,
    pub rows: u8,
    pub gap: u8,
    pub borders: String,
    pub cell_aspect: f32,
    pub min_unit_inner: MinUnit,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MinUnit {
    pub cols: u16,
    pub rows: u16,
}

impl Default for GridSect {
    fn default() -> GridSect {
        GridSect {
            columns: 12,
            rows: 6,
            gap: 1,
            borders: "each".into(),
            cell_aspect: 0.5,
            min_unit_inner: MinUnit { cols: 8, rows: 3 },
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PageSect {
    pub name: String,
    #[serde(default)]
    pub hotkey: Option<String>,
    pub place: Vec<PlaceSect>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlaceSect {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub kind: Option<String>,
    pub at: [u8; 2],
    pub size: [u8; 2],
    #[serde(default)]
    pub view: Option<String>,
    #[serde(default)]
    pub priority: Option<i32>,
}

pub struct Loaded {
    pub config: ConfigFile,
    pub grid: GridSpec,
    pub pages: Vec<Page>,
    pub warnings: Vec<String>,
    pub config_path: Option<PathBuf>,
    pub layout_path: Option<PathBuf>,
}

fn config_dir() -> Option<PathBuf> {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .map(|p| p.join("gridwatch"))
}

pub fn load() -> Result<Loaded, ConfigError> {
    let dir = config_dir();
    let config_path = dir
        .as_ref()
        .map(|d| d.join("config.toml"))
        .filter(|p| p.exists());
    let layout_path = dir
        .as_ref()
        .map(|d| d.join("layout.toml"))
        .filter(|p| p.exists());
    let config_text = match &config_path {
        Some(p) => {
            std::fs::read_to_string(p).map_err(|e| ConfigError(format!("{}: {e}", p.display())))?
        }
        None => DEFAULT_CONFIG.to_string(),
    };
    let layout_text = match &layout_path {
        Some(p) => {
            std::fs::read_to_string(p).map_err(|e| ConfigError(format!("{}: {e}", p.display())))?
        }
        None => DEFAULT_LAYOUT.to_string(),
    };
    load_from(&config_text, &layout_text, config_path, layout_path, true)
}

/// Embedded defaults only — hermetic and env-free, so `shot --seed N` stays
/// byte-deterministic across machines (§12.5, D41).
pub fn load_embedded() -> Result<Loaded, ConfigError> {
    load_from(DEFAULT_CONFIG, DEFAULT_LAYOUT, None, None, false)
}

fn load_from(
    config_text: &str,
    layout_text: &str,
    config_path: Option<PathBuf>,
    layout_path: Option<PathBuf>,
    env_layer: bool,
) -> Result<Loaded, ConfigError> {
    let mut warnings = Vec::new();
    let mut config: ConfigFile = parse("config.toml", config_text)?;
    let layout: LayoutFile = parse("layout.toml", layout_text)?;
    if !config.record.is_empty() {
        warnings.push("[record] arrives in arc 2 — ignored for now".into());
    }
    if !config.rules.is_empty() {
        warnings.push("[[rules]] arrive in arc 7 — ignored for now".into());
    }
    if config.schema != 1 || layout.schema != 1 {
        return Err(ConfigError("unsupported schema (expected 1)".into()));
    }
    // Env layer (D39: env between file and CLI; skipped for embedded loads).
    if env_layer {
        if let Ok(t) = std::env::var("GRIDWATCH_THEME") {
            config.theme = t;
        }
        if let Ok(f) = std::env::var("GRIDWATCH_FPS")
            && let Ok(v) = f.parse()
        {
            config.fps = v;
        }
        if let Ok(c) = std::env::var("GRIDWATCH_COLOR") {
            config.color = c;
        }
    }

    let borders = match layout.grid.borders.as_str() {
        "each" => BorderMode::Each,
        "shared" => BorderMode::Shared,
        "none" => BorderMode::None,
        other => return Err(ConfigError(format!("unknown borders mode '{other}'"))),
    };
    let grid = GridSpec {
        columns: layout.grid.columns,
        rows: layout.grid.rows,
        gap: layout.grid.gap,
        borders,
        cell_aspect: layout.grid.cell_aspect,
        min_unit_inner: Size::new(
            layout.grid.min_unit_inner.cols,
            layout.grid.min_unit_inner.rows,
        ),
    };
    if grid.columns == 0 || grid.rows == 0 {
        return Err(ConfigError("grid columns/rows must be ≥ 1".into()));
    }
    let known_kinds: Vec<&str> = Vec::new(); // filled by the shell against the registry
    let _ = known_kinds;
    let mut pages = Vec::new();
    for p in &layout.pages {
        let mut place = Vec::new();
        for (i, ps) in p.place.iter().enumerate() {
            let target = match (&ps.id, &ps.kind) {
                (Some(id), None) => PlaceTarget::Id(id.clone()),
                (None, Some(k)) => PlaceTarget::Kind(k.clone()),
                _ => {
                    return Err(ConfigError(format!(
                        "page '{}' placement {i}: exactly one of `id` or `kind`",
                        p.name
                    )));
                }
            };
            let placement = Placement {
                target,
                at: (ps.at[0], ps.at[1]),
                size: (ps.size[0], ps.size[1]),
                view: ps.view.clone(),
                priority: ps.priority.unwrap_or(0),
            };
            if !placement.in_bounds(grid.columns, grid.rows) {
                return Err(ConfigError(format!(
                    "page '{}' placement '{}' out of the {}x{} grid",
                    p.name,
                    placement.target.label(),
                    grid.columns,
                    grid.rows
                )));
            }
            for prev in &place {
                if placement.overlaps(prev) {
                    return Err(ConfigError(format!(
                        "page '{}': '{}' overlaps '{}'",
                        p.name,
                        placement.target.label(),
                        Placement::target_label(prev)
                    )));
                }
            }
            place.push(placement);
        }
        pages.push(Page {
            name: p.name.clone(),
            hotkey: p.hotkey.as_ref().and_then(|h| h.chars().next()),
            place,
        });
    }
    if pages.is_empty() {
        return Err(ConfigError("layout has no pages".into()));
    }
    // Duplicate instance ids are a config error; instances of unknown kinds
    // become placeholder chips at solve time (§6).
    let mut seen = std::collections::BTreeSet::new();
    for inst in &config.components {
        if !seen.insert(inst.id.clone()) {
            return Err(ConfigError(format!("duplicate component id '{}'", inst.id)));
        }
    }
    if config.fps == 0 || config.fps > 120 {
        warnings.push(format!("fps {} clamped into 1..=120", config.fps));
        config.fps = config.fps.clamp(1, 120);
    }
    Ok(Loaded {
        config,
        grid,
        pages,
        warnings,
        config_path,
        layout_path,
    })
}

trait TargetLabel {
    fn target_label(p: &Placement) -> String;
}

impl TargetLabel for Placement {
    fn target_label(p: &Placement) -> String {
        p.target.label().to_string()
    }
}

/// The colour ladder (§7): CLI > config > NO_COLOR (→ mono theme) > COLORTERM > TERM.
/// The environment is snapshotted into `ColorEnv` so the ladder is unit-testable.
pub fn resolve_color(
    cli: Option<&str>,
    cfg: &str,
    env: &ColorEnv,
) -> (gridwatch_ui::ColorMode, bool) {
    use gridwatch_ui::ColorMode::*;
    let pick = |s: &str| match s {
        "truecolor" | "always" => Some(TrueColor),
        "256" => Some(Ansi256),
        "16" => Some(Ansi16),
        "never" | "mono" => Some(Mono),
        _ => None,
    };
    if let Some(m) = cli.and_then(pick) {
        return (m, m == Mono);
    }
    if cfg != "auto"
        && let Some(m) = pick(cfg)
    {
        return (m, m == Mono);
    }
    if env.no_color {
        return (Mono, true);
    }
    if env
        .colorterm
        .as_deref()
        .is_some_and(|v| v.contains("truecolor") || v.contains("24bit"))
    {
        return (TrueColor, false);
    }
    if env.term.as_deref().is_some_and(|v| v.contains("256")) {
        return (Ansi256, false);
    }
    (Ansi16, false)
}

#[derive(Clone, Debug, Default)]
pub struct ColorEnv {
    pub no_color: bool,
    pub colorterm: Option<String>,
    pub term: Option<String>,
}

impl ColorEnv {
    pub fn capture() -> ColorEnv {
        ColorEnv {
            no_color: std::env::var_os("NO_COLOR").is_some_and(|v| !v.is_empty()),
            colorterm: std::env::var("COLORTERM").ok(),
            term: std::env::var("TERM").ok(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The embedded TOML and the structural Default must never drift (§9).
    #[test]
    fn embedded_default_matches_structural() {
        let parsed: ConfigFile = parse("embedded default config", DEFAULT_CONFIG).unwrap();
        assert_eq!(parsed, ConfigFile::default());
    }

    /// Every rung of the §7 ladder, driven through the injected env snapshot.
    #[test]
    fn color_ladder_env_rungs() {
        use gridwatch_ui::ColorMode::*;
        let noisy = ColorEnv {
            no_color: true,
            colorterm: Some("truecolor".into()),
            term: Some("xterm-256color".into()),
        };
        assert_eq!(resolve_color(None, "auto", &noisy), (Mono, true));
        let tc = ColorEnv {
            no_color: false,
            colorterm: Some("truecolor".into()),
            term: None,
        };
        assert_eq!(resolve_color(None, "auto", &tc), (TrueColor, false));
        let c256 = ColorEnv {
            no_color: false,
            colorterm: None,
            term: Some("xterm-256color".into()),
        };
        assert_eq!(resolve_color(None, "auto", &c256), (Ansi256, false));
        assert_eq!(
            resolve_color(Some("16"), "truecolor", &noisy),
            (Ansi16, false)
        );
    }

    /// A zero-column grid would underflow thresholds(); it must be rejected.
    #[test]
    fn zero_grid_rejected() {
        let layout = "schema = 1\npages = []\n[grid]\ncolumns = 0\n";
        let Err(err) = load_from(DEFAULT_CONFIG, layout, None, None, false) else {
            panic!("a zero-column grid was accepted");
        };
        assert!(err.to_string().contains("columns"), "{err}");
    }

    /// §9: [record] and [[rules]] parse today and warn until their arcs.
    #[test]
    fn record_and_rules_parse_with_warning() {
        let cfg = "schema = 1\n[record]\nring_mb = 8\n[[rules]]\nid = \"x\"\n";
        let loaded = load_from(cfg, DEFAULT_LAYOUT, None, None, false).unwrap();
        assert!(loaded.warnings.iter().any(|w| w.contains("[record]")));
        assert!(loaded.warnings.iter().any(|w| w.contains("rules")));
    }

    /// A partial user config layers over the defaults without recursing.
    #[test]
    fn partial_config_layers() {
        let cfg: ConfigFile = parse("partial", "schema = 1\ntheme = \"mono\"\n").unwrap();
        assert_eq!(cfg.theme, "mono");
        assert_eq!(cfg.fps, 30);
        assert_eq!(cfg.components.len(), 7);
    }
}
