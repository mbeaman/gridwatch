//! `w` in edit mode (§9, brief arc 4 seam 5): `layout.toml` — and only
//! `layout.toml` — rewritten through `toml_edit` so every key, comment and
//! blank line outside the `place` arrays survives. Atomic write, re-parse
//! check, the content hash for the watcher.

use std::hash::{Hash, Hasher};
use std::path::Path;

use gridwatch_ui::layout::{Page, PlaceTarget, Placement};
use toml_edit::{Array, ArrayOfTables, DocumentMut, InlineTable, Item, Table, Value};

fn inline(p: &Placement) -> Value {
    let mut t = InlineTable::new();
    match &p.target {
        PlaceTarget::Id(id) => t.insert("id", Value::from(id.as_str())),
        PlaceTarget::Kind(k) => t.insert("kind", Value::from(k.as_str())),
    };
    let mut at = Array::new();
    at.push(i64::from(p.at.0));
    at.push(i64::from(p.at.1));
    t.insert("at", Value::Array(at));
    let mut size = Array::new();
    size.push(i64::from(p.size.0));
    size.push(i64::from(p.size.1));
    t.insert("size", Value::Array(size));
    if let Some(v) = &p.view {
        t.insert("view", Value::from(v.as_str()));
    }
    if p.priority != 0 {
        t.insert("priority", Value::from(i64::from(p.priority)));
    }
    Value::InlineTable(t)
}

fn place_array(page: &Page) -> Array {
    let mut arr = Array::new();
    for p in &page.place {
        arr.push_formatted(inline(p));
    }
    // One placement per line, as the shipped default is written.
    for (i, v) in arr.iter_mut().enumerate() {
        v.decor_mut().set_prefix("\n  ");
        if i + 1 == page.place.len() {
            v.decor_mut().set_suffix(",\n");
        }
    }
    arr
}

fn page_table(page: &Page) -> Table {
    let mut t = Table::new();
    t.insert("name", Item::Value(Value::from(page.name.as_str())));
    if let Some(h) = page.hotkey {
        t.insert("hotkey", Item::Value(Value::from(h.to_string())));
    }
    t.insert("place", Item::Value(Value::Array(place_array(page))));
    t
}

/// The new file text: `existing` (the current file, or the embedded default
/// when there is none) with each `[[pages]]` table's `place` replaced by the
/// in-memory placements. When the page count differs the whole `pages`
/// array is rebuilt (names and hotkeys from memory); otherwise every other
/// key of every page — and every comment outside `place` — is untouched.
pub fn render_layout(existing: &str, pages: &[Page]) -> Result<String, String> {
    let mut doc: DocumentMut = existing
        .parse()
        .map_err(|e: toml_edit::TomlError| format!("layout.toml: {e}"))?;
    if !doc.contains_key("schema") {
        doc.insert("schema", Item::Value(Value::from(1)));
    }
    if !doc.contains_key("grid") {
        // `[grid]` is required by the layout schema; its keys all default.
        doc.insert("grid", Item::Table(Table::new()));
    }
    let same_count = doc
        .get("pages")
        .and_then(Item::as_array_of_tables)
        .is_some_and(|a| a.len() == pages.len());
    if same_count {
        let arr = doc["pages"]
            .as_array_of_tables_mut()
            .ok_or("pages is not an array of tables")?;
        for (table, page) in arr.iter_mut().zip(pages) {
            table.insert("place", Item::Value(Value::Array(place_array(page))));
        }
    } else {
        let mut arr = ArrayOfTables::new();
        for p in pages {
            arr.push(page_table(p));
        }
        doc.insert("pages", Item::ArrayOfTables(arr));
    }
    Ok(doc.to_string())
}

/// The re-parse check (seam 5): the text must load beside the current
/// `config.toml` text and yield exactly the pages in memory.
pub fn verify(text: &str, config_text: &str, pages: &[Page]) -> Result<(), String> {
    let loaded = crate::config::load_texts(config_text, text).map_err(|e| e.to_string())?;
    if loaded.pages != pages {
        return Err("the written layout re-parses differently — not saved".into());
    }
    Ok(())
}

/// Temp file in the same directory, then rename: readers never see a torn
/// file, and the watcher sees one mtime change.
pub fn write_atomic(path: &Path, text: &str) -> Result<(), String> {
    let dir = path.parent().ok_or("layout path has no parent")?;
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let tmp = dir.join(format!(
        ".{}.tmp-{}",
        path.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "layout.toml".into()),
        std::process::id()
    ));
    std::fs::write(&tmp, text).map_err(|e| format!("{}: {e}", tmp.display()))?;
    if let Err(e) = std::fs::rename(&tmp, path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("{}: {e}", path.display()));
    }
    Ok(())
}

/// The hash the watcher's ignore slot compares against (`watch::content_hash`
/// over the same bytes).
pub fn hash_of(text: &str) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    text.as_bytes().hash(&mut h);
    h.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{DEFAULT_CONFIG, DEFAULT_LAYOUT, load_texts};

    const COMMENTED: &str = r#"# my layout
schema = 1

[grid]   # twelve by six
columns = 12
rows = 6

[[pages]]
name = "Overview"   # the first page
hotkey = "1"
place = [
  { id = "cpu", at = [0, 0], size = [6, 3], priority = 100 },
  { id = "gpu", at = [6, 0], size = [6, 3] },
]

# the second page
[[pages]]
name = "Audio"
hotkey = "2"
place = [
  { id = "cpu", at = [0, 0], size = [12, 3], view = "meters" },
]
"#;

    #[test]
    fn comments_outside_place_survive_and_place_is_replaced() {
        let mut pages = load_texts(DEFAULT_CONFIG, COMMENTED).unwrap().pages;
        pages[0].place[1].at = (4, 3);
        pages[0].place.remove(0);
        pages[1].place.push(Placement {
            target: PlaceTarget::Kind("clock".into()),
            at: (10, 5),
            size: (2, 1),
            view: None,
            priority: 0,
        });
        let out = render_layout(COMMENTED, &pages).unwrap();
        assert!(out.contains("# my layout"), "{out}");
        assert!(out.contains("[grid]   # twelve by six"), "{out}");
        assert!(out.contains("# the first page"), "{out}");
        assert!(out.contains("# the second page"), "{out}");
        assert!(
            out.contains(r#"{ id = "gpu", at = [4, 3], size = [6, 3] }"#),
            "{out}"
        );
        assert!(
            !out.contains(r#"id = "cpu", at = [0, 0], size = [6, 3]"#),
            "{out}"
        );
        assert!(
            out.contains(r#"{ kind = "clock", at = [10, 5], size = [2, 1] }"#),
            "{out}"
        );
        assert!(out.contains(r#"view = "meters""#), "{out}");
        verify(&out, DEFAULT_CONFIG, &pages).unwrap();
    }

    #[test]
    fn a_changed_page_count_rebuilds_the_array_and_the_default_round_trips() {
        let mut pages = load_texts(DEFAULT_CONFIG, DEFAULT_LAYOUT).unwrap().pages;
        pages.truncate(1);
        let out = render_layout(DEFAULT_LAYOUT, &pages).unwrap();
        verify(&out, DEFAULT_CONFIG, &pages).unwrap();
        assert_eq!(out.matches("[[pages]]").count(), 1, "{out}");
        // Untouched pages re-render byte-for-byte equal in meaning.
        let pages = load_texts(DEFAULT_CONFIG, DEFAULT_LAYOUT).unwrap().pages;
        let out = render_layout(DEFAULT_LAYOUT, &pages).unwrap();
        verify(&out, DEFAULT_CONFIG, &pages).unwrap();
        // From nothing: schema and pages appear; grid falls back to defaults.
        let out = render_layout("", &pages).unwrap();
        assert!(out.starts_with("schema = 1"), "{out}");
        verify(&out, DEFAULT_CONFIG, &pages).unwrap();
    }

    #[test]
    fn verify_refuses_a_mismatch_and_write_is_atomic() {
        let pages = load_texts(DEFAULT_CONFIG, DEFAULT_LAYOUT).unwrap().pages;
        let mut other = pages.clone();
        other[0].place[0].at = (1, 1);
        assert!(verify(DEFAULT_LAYOUT, DEFAULT_CONFIG, &other).is_err());
        let dir = std::env::temp_dir().join(format!("gridwatch-save-{}", std::process::id()));
        let path = dir.join("nested/layout.toml");
        write_atomic(&path, DEFAULT_LAYOUT).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), DEFAULT_LAYOUT);
        assert!(
            std::fs::read_dir(path.parent().unwrap())
                .unwrap()
                .all(|e| !e.unwrap().file_name().to_string_lossy().contains(".tmp-")),
            "temp file left behind"
        );
        assert_eq!(
            hash_of(DEFAULT_LAYOUT),
            crate::watch::content_hash(DEFAULT_LAYOUT.as_bytes())
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
