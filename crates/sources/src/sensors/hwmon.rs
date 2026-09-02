//! The hwmon walker (brief arc 5 seam 6): `/sys/class/hwmon/hwmon*/name`,
//! every `temp*_input | fan*_input | in*_input | power*_input`, the label
//! from `<stem>_label` else the stem, `max`/`crit` from `<stem>_max` /
//! `<stem>_crit` with the nvme `65261850` m°C sentinel dropped. Chips are
//! keyed by `name`; duplicates of a name are ordered by the **device** they
//! hang off (`hwmonN/device`, e.g. `nvme0`, `0000:00:18.3`) and suffixed
//! `#2`, `#3` — `hwmonN` numbering is not stable across boots, so ordering
//! by it would rename a drive after a reboot (review). A read error or `-ENODATA` means "absent this
//! sample", never a panic. Tested over `fixtures/hwmon/torch/`.

use std::path::{Path, PathBuf};

use gridwatch_store::keys::sensors::{ChipInfo, SENTINEL_MILLI};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Kind {
    Temp,
    Fan,
    In,
    Power,
}

impl Kind {
    fn from_stem(stem: &str) -> Option<(Kind, &str)> {
        for (prefix, kind) in [
            ("temp", Kind::Temp),
            ("fan", Kind::Fan),
            ("in", Kind::In),
            ("power", Kind::Power),
        ] {
            if let Some(rest) = stem.strip_prefix(prefix)
                && !rest.is_empty()
                && rest.bytes().all(|b| b.is_ascii_digit())
            {
                return Some((kind, rest));
            }
        }
        None
    }

    pub fn name(self) -> &'static str {
        match self {
            Kind::Temp => "temp",
            Kind::Fan => "fan",
            Kind::In => "in",
            Kind::Power => "power",
        }
    }

    /// The sysfs ABI's divisor to the published unit (a division, so
    /// `81850` reads exactly as the literal `81.85`).
    fn divisor(self) -> f64 {
        match self {
            Kind::Temp => 1e3,  // m°C → °C
            Kind::Fan => 1.0,   // RPM
            Kind::In => 1e3,    // mV → V
            Kind::Power => 1e6, // µW → W
        }
    }
}

/// One input the walker found.
#[derive(Clone, Debug, PartialEq)]
pub struct Sensor {
    /// `chip:label` — the store label.
    pub key: String,
    pub chip: String,
    pub label: String,
    pub kind: Kind,
    pub path: PathBuf,
    pub max: Option<f64>,
    pub crit: Option<f64>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Inventory {
    pub chips: Vec<ChipInfo>,
    pub sensors: Vec<Sensor>,
}

fn read_trim(p: &Path) -> Option<String> {
    std::fs::read_to_string(p)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn read_i64(p: &Path) -> Option<i64> {
    read_trim(p)?.parse().ok()
}

/// The device a chip hangs off: the `device` symlink's target name
/// (`nvme0`, `0000:00:18.3`, `8-0051`), or the fixture's `device_name`
/// file, or the hwmon directory's own name. Stable across boots where the
/// hwmon number is not.
pub fn device_of(dir: &Path) -> String {
    std::fs::canonicalize(dir.join("device"))
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
        .or_else(|| read_trim(&dir.join("device_name")))
        .unwrap_or_else(|| {
            dir.file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default()
        })
}

/// A threshold in the input's raw unit, dropping driver sentinels.
fn threshold(p: &Path, kind: Kind) -> Option<f64> {
    let raw = read_i64(p)?;
    if kind == Kind::Temp && raw > SENTINEL_MILLI {
        return None;
    }
    Some(raw as f64 / kind.divisor())
}

/// Glob-lite chip filter: `*` matches anything, a leading or trailing `*`
/// a suffix or prefix, `*x*` a substring, else exact.
pub fn chip_matches(pattern: &str, name: &str) -> bool {
    match (pattern.strip_prefix('*'), pattern.strip_suffix('*')) {
        (Some("") | None, Some("")) | (Some(""), None) => true,
        (Some(rest), None) => name.ends_with(rest),
        (None, Some(rest)) => name.starts_with(rest),
        (Some(a), Some(_)) => name.contains(a.strip_suffix('*').unwrap_or(a)),
        (None, None) => pattern == name,
    }
}

/// Walk `<sys>/class/hwmon` once: the inventory.
pub fn walk(sys: &Path, chips: &[String]) -> Inventory {
    let hwmon = sys.join("class/hwmon");
    let Ok(entries) = std::fs::read_dir(&hwmon) else {
        return Inventory::default();
    };
    let mut dirs: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
    // Ordered by (chip name, device): the suffix a duplicate gets then
    // follows the hardware, not the boot's hwmon numbering (review).
    dirs.sort_by_key(|d| {
        (
            read_trim(&d.join("name")).unwrap_or_default(),
            device_of(d),
            d.file_name()
                .and_then(|n| n.to_str())
                .map(String::from)
                .unwrap_or_default(),
        )
    });
    let mut inv = Inventory::default();
    let mut seen: std::collections::HashMap<String, u32> = std::collections::HashMap::new();
    for dir in dirs {
        let Some(raw_name) = read_trim(&dir.join("name")) else {
            continue;
        };
        let n = seen.entry(raw_name.clone()).or_insert(0);
        *n += 1;
        let name = if *n == 1 {
            raw_name.clone()
        } else {
            format!("{raw_name}#{n}")
        };
        // The filter sees both the bare name and the suffixed one, so
        // `chips = ["nvme#2"]` and `chips = ["nvme*"]` both work.
        if !chips
            .iter()
            .any(|p| chip_matches(p, &raw_name) || chip_matches(p, &name))
        {
            continue;
        }
        let Ok(files) = std::fs::read_dir(&dir) else {
            continue;
        };
        let mut inputs: Vec<(Kind, u32, String)> = Vec::new();
        for f in files.flatten() {
            let fname = f.file_name();
            let Some(fname) = fname.to_str() else {
                continue;
            };
            let Some(stem) = fname.strip_suffix("_input") else {
                continue;
            };
            if let Some((kind, idx)) = Kind::from_stem(stem)
                && let Ok(i) = idx.parse::<u32>()
            {
                inputs.push((kind, i, stem.to_string()));
            }
        }
        if inputs.is_empty() {
            // The attribute-less `asus` chip: listed nowhere.
            continue;
        }
        inputs.sort();
        let mut kinds: Vec<String> = Vec::new();
        for (kind, _, stem) in inputs {
            let label =
                read_trim(&dir.join(format!("{stem}_label"))).unwrap_or_else(|| stem.clone());
            let kname = kind.name().to_string();
            if !kinds.contains(&kname) {
                kinds.push(kname);
            }
            inv.sensors.push(Sensor {
                key: format!("{name}:{label}"),
                chip: name.clone(),
                label,
                kind,
                path: dir.join(format!("{stem}_input")),
                max: threshold(&dir.join(format!("{stem}_max")), kind),
                crit: threshold(&dir.join(format!("{stem}_crit")), kind),
            });
        }
        inv.chips.push(ChipInfo {
            name,
            path: dir.display().to_string(),
            device: device_of(&dir),
            kinds,
        });
    }
    inv
}

/// One input's current value in its published unit; `None` when the read
/// fails (`-ENODATA`, a device gone).
pub fn read(s: &Sensor) -> Option<f64> {
    read_i64(&s.path).map(|raw| raw as f64 / s.kind.divisor())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn torch() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/hwmon/torch")
    }

    /// The fixture tree mirrors `/sys/class/hwmon` under `class/hwmon`? No —
    /// it holds the `hwmonN` directories directly; the walker takes `<sys>`
    /// and joins `class/hwmon`, so the test builds that shape in a temp dir.
    fn sys_root() -> PathBuf {
        let root = std::env::temp_dir().join(format!("gw-hwmon-{}", std::process::id()));
        let hw = root.join("class/hwmon");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&hw).unwrap();
        for e in std::fs::read_dir(torch()).unwrap().flatten() {
            let dst = hw.join(e.file_name());
            std::fs::create_dir_all(&dst).unwrap();
            for f in std::fs::read_dir(e.path()).unwrap().flatten() {
                std::fs::copy(f.path(), dst.join(f.file_name())).unwrap();
            }
        }
        root
    }

    #[test]
    fn walks_torch_with_sentinels_dropped_and_duplicates_numbered() {
        let root = sys_root();
        let inv = walk(&root, &["*".to_string()]);
        let names: Vec<&str> = inv.chips.iter().map(|c| c.name.as_str()).collect();
        // Ordered by (name, device) — the suffix follows the hardware.
        assert_eq!(
            names,
            [
                "k10temp",
                "mt7925_phy0",
                "nvme",
                "nvme#2",
                "nvme#3",
                "r8169_0_b00:00",
                "r8169_0_c00:00",
                "spd5118",
                "spd5118#2"
            ],
            "asus (no inputs) is skipped"
        );
        // `nvme` is the drive at `nvme0` (hwmon1 on this boot), so its
        // thresholds are that drive's — the point of keying by device.
        let comp = inv
            .sensors
            .iter()
            .find(|s| s.key == "nvme:Composite")
            .expect("nvme composite");
        assert_eq!(comp.max, Some(83.85));
        assert_eq!(comp.crit, Some(87.85));
        let third = inv
            .sensors
            .iter()
            .find(|s| s.key == "nvme#3:Composite")
            .unwrap();
        assert_eq!((third.max, third.crit), (Some(81.85), Some(84.85)));
        let s1 = inv
            .sensors
            .iter()
            .find(|s| s.key == "nvme:Sensor 1")
            .unwrap();
        assert_eq!(s1.max, None, "65261850 m°C is a sentinel");
        let tctl = inv
            .sensors
            .iter()
            .find(|s| s.key == "k10temp:Tctl")
            .unwrap();
        assert_eq!(read(tctl), Some(50.125));
        let wifi = inv
            .sensors
            .iter()
            .find(|s| s.chip == "mt7925_phy0")
            .unwrap();
        assert_eq!(wifi.label, "temp1", "no label file → the stem");
        assert_eq!(wifi.key, "mt7925_phy0:temp1");
        let dimm2 = inv
            .sensors
            .iter()
            .find(|s| s.key == "spd5118#2:temp1")
            .unwrap();
        assert_eq!(read(dimm2), Some(39.0));
        assert_eq!(dimm2.max, Some(55.0));
        assert!(inv.sensors.iter().all(|s| s.kind == Kind::Temp));
        assert_eq!(inv.chips[4].kinds, ["temp"]);
        // The device each chip hangs off (the suffix follows the hardware,
        // not the boot's hwmon numbering).
        let by_name: std::collections::HashMap<&str, &str> = inv
            .chips
            .iter()
            .map(|c| (c.name.as_str(), c.device.as_str()))
            .collect();
        assert_eq!(by_name["nvme"], "nvme0", "hwmon1 sorts first by device");
        assert_eq!(by_name["nvme#2"], "nvme1");
        assert_eq!(by_name["nvme#3"], "nvme2");
        assert_eq!(by_name["k10temp"], "0000:00:18.3");
        assert_eq!(by_name["spd5118#2"], "8-0053");
        // A filter, on the bare name and on the suffixed one.
        let only = walk(&root, &["nvme*".to_string(), "k10temp".to_string()]);
        assert_eq!(only.chips.len(), 4);
        let one = walk(&root, &["nvme#2".to_string()]);
        assert_eq!(one.chips.len(), 1);
        assert_eq!(one.chips[0].device, "nvme1");
        assert!(walk(&root, &["nct6798".to_string()]).chips.is_empty());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn kinds_units_and_globs() {
        assert_eq!(Kind::from_stem("temp12"), Some((Kind::Temp, "12")));
        assert_eq!(Kind::from_stem("in0"), Some((Kind::In, "0")));
        assert_eq!(Kind::from_stem("power1"), Some((Kind::Power, "1")));
        assert_eq!(Kind::from_stem("fan"), None);
        assert_eq!(Kind::from_stem("pwm1"), None);
        assert_eq!(Kind::Power.divisor(), 1e6);
        assert!(chip_matches("*", "anything"));
        assert!(chip_matches("nvme*", "nvme"));
        assert!(!chip_matches("nvme", "nvme#2"));
        assert!(chip_matches("nvme*", "nvme#2"));
        assert!(chip_matches("*temp", "k10temp"), "a leading star");
        assert!(chip_matches("*5118*", "spd5118#2"), "a substring");
        assert!(!chip_matches("*temp", "spd5118"));
        let missing = walk(Path::new("/nonexistent"), &["*".to_string()]);
        assert!(missing.chips.is_empty());
    }
}
