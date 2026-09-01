//! Buffer and view dumps (§12): run-length styled cells for snapshots, ANSI
//! for screenshots, and the semantic view tree as JSON for the YAML snapshots.

use ratatui_core::buffer::Buffer;
use ratatui_core::style::Color;
use serde_json::{Value, json};

use crate::view::View;

fn color_code(c: Color) -> String {
    match c {
        Color::Reset => "-".into(),
        Color::Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
        Color::Indexed(i) => format!("i{i}"),
        other => format!("{other:?}"),
    }
}

/// Run-length-encoded styled dump: one line per row, `[fg/bg/mods]text` runs.
pub fn cells(buf: &Buffer) -> String {
    let area = *buf.area();
    let mut out = String::new();
    for y in area.y..area.y + area.height {
        let mut run_style = String::new();
        let mut run_text = String::new();
        for x in area.x..area.x + area.width {
            let Some(cell) = buf.cell((x, y)) else {
                continue;
            };
            let style = format!(
                "{}/{}/{:?}",
                color_code(cell.fg),
                color_code(cell.bg),
                cell.modifier
            );
            if style != run_style {
                if !run_text.is_empty() {
                    out.push_str(&format!("[{run_style}]{run_text}"));
                }
                run_style = style;
                run_text = String::new();
            }
            run_text.push_str(cell.symbol());
        }
        if !run_text.is_empty() {
            out.push_str(&format!("[{run_style}]{run_text}"));
        }
        out.push('\n');
    }
    out
}

/// Plain ANSI (truecolor) dump for `shot --format ansi` and README captures.
pub fn ansi(buf: &Buffer) -> String {
    let area = *buf.area();
    let mut out = String::new();
    for y in area.y..area.y + area.height {
        for x in area.x..area.x + area.width {
            let Some(cell) = buf.cell((x, y)) else {
                continue;
            };
            match cell.fg {
                Color::Rgb(r, g, b) => out.push_str(&format!("\x1b[38;2;{r};{g};{b}m")),
                Color::Indexed(i) => out.push_str(&format!("\x1b[38;5;{i}m")),
                _ => out.push_str("\x1b[39m"),
            }
            match cell.bg {
                Color::Rgb(r, g, b) => out.push_str(&format!("\x1b[48;2;{r};{g};{b}m")),
                Color::Indexed(i) => out.push_str(&format!("\x1b[48;5;{i}m")),
                _ => out.push_str("\x1b[49m"),
            }
            out.push_str(cell.symbol());
        }
        out.push_str("\x1b[0m\n");
    }
    out
}

/// The semantic tree as JSON (YAML snapshots, the wire protocol in arc 8).
pub fn view_value(view: &View) -> Value {
    match view {
        View::Empty => json!("empty"),
        View::Text(lines) => json!({
            "text": lines.iter().map(|l| l.iter().map(|s| json!({"role": format!("{:?}", s.role), "s": s.text, "bold": s.bold})).collect::<Vec<_>>()).collect::<Vec<_>>()
        }),
        View::KeyValue(rows) => json!({
            "kv": rows.iter().map(|(k, v, sev)| json!({
                "k": k,
                "v": v.iter().map(|s| s.text.clone()).collect::<Vec<_>>().join(""),
                "sev": sev.map(|s| format!("{s:?}")),
            })).collect::<Vec<_>>()
        }),
        View::Gauge {
            label,
            value,
            gradient,
            text,
        } => json!({
            "gauge": {"label": label, "value": value, "gradient": format!("{gradient:?}"), "text": text}
        }),
        View::Segmented {
            label,
            segments,
            text,
        } => json!({
            "segmented": {
                "label": label,
                "segments": segments.iter().map(|(r, f)| json!([format!("{r:?}"), f])).collect::<Vec<_>>(),
                "text": text,
            }
        }),
        View::Bars {
            values,
            gradient,
            labels,
            peaks,
        } => json!({
            "bars": {"n": values.len(), "values": values, "gradient": format!("{gradient:?}"),
                      "labels": labels.as_ref().map(|l| l.len()), "peaks": peaks.is_some()}
        }),
        View::Sparkline {
            series,
            gradient,
            max,
        } => json!({
            "sparkline": {"n": series.len(), "gradient": format!("{gradient:?}"), "max": max,
                           "series": series}
        }),
        View::Chart { series, .. } => json!({
            "chart": series.iter().map(|s| json!({"label": s.label, "points": s.data.len()})).collect::<Vec<_>>()
        }),
        View::Table {
            columns,
            rows,
            selected,
            sort,
            scroll,
        } => json!({
            "table": {
                "columns": columns.iter().map(|c| c.title.clone()).collect::<Vec<_>>(),
                "rows": rows.iter().map(|r| r.iter().map(|l| l.iter().map(|s| s.text.clone()).collect::<Vec<_>>().join("")).collect::<Vec<_>>()).collect::<Vec<_>>(),
                "selected": selected, "sort": sort.map(|(i, d)| json!([i, format!("{d:?}")])), "scroll": scroll,
            }
        }),
        View::BigNumber { text, role } => {
            json!({"big": {"text": text, "role": format!("{role:?}")}})
        }
        View::Stack { dir, children } => json!({
            "stack": {"dir": format!("{dir:?}"), "children": children.iter().map(|(c, v)| json!([format!("{c:?}"), view_value(v)])).collect::<Vec<_>>()}
        }),
        View::Custom { describe, .. } => json!({"custom": describe}),
    }
}
