//! Buffer and view dumps (§12): run-length styled cells for snapshots, ANSI
//! for screenshots, SVG for the README images CI regenerates (arc 2a), and
//! the semantic view tree as JSON for the YAML snapshots.

use ratatui_core::buffer::Buffer;
use ratatui_core::style::{Color, Modifier};
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

/// Cell size of the SVG dump in px: an 8×16 monospace cell.
pub const SVG_CELL_W: u32 = 8;
pub const SVG_CELL_H: u32 = 16;

fn svg_color(c: Color, default: &str) -> String {
    match c {
        Color::Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
        // The 16/256-colour ladders are never what `shot` renders (TrueColor
        // is forced, D41); map them to the defaults rather than guess a palette.
        _ => default.to_string(),
    }
}

fn xml_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            c => out.push(c),
        }
    }
    out
}

/// Hand-written SVG (§12.5, brief 2a task 5): one `<rect>` per background run
/// and one `<text>` per foreground run, `xml:space="preserve"`, a monospace
/// font stack, cells 8×16 px. Deterministic for a deterministic buffer, so CI
/// can diff it. No crate.
pub fn svg(buf: &Buffer) -> String {
    let area = *buf.area();
    let w = u32::from(area.width) * SVG_CELL_W;
    let h = u32::from(area.height) * SVG_CELL_H;
    const DEFAULT_FG: &str = "#c8c8c8";
    const DEFAULT_BG: &str = "#101010";
    let mut out = String::new();
    out.push_str(&format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{w}\" height=\"{h}\" viewBox=\"0 0 {w} {h}\" font-family=\"DejaVu Sans Mono, Menlo, Consolas, monospace\" font-size=\"13\">\n"
    ));
    out.push_str(&format!(
        "<rect width=\"{w}\" height=\"{h}\" fill=\"{DEFAULT_BG}\"/>\n"
    ));
    // Backgrounds: runs of equal bg per row.
    for y in 0..area.height {
        let mut run_start = 0u16;
        let mut run_bg: Option<String> = None;
        let flush = |out: &mut String, bg: &Option<String>, x0: u16, x1: u16, y: u16| {
            if let Some(bg) = bg
                && bg != DEFAULT_BG
                && x1 > x0
            {
                out.push_str(&format!(
                    "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" fill=\"{bg}\"/>\n",
                    u32::from(x0) * SVG_CELL_W,
                    u32::from(y) * SVG_CELL_H,
                    u32::from(x1 - x0) * SVG_CELL_W,
                    SVG_CELL_H
                ));
            }
        };
        for x in 0..area.width {
            let Some(cell) = buf.cell((area.x + x, area.y + y)) else {
                continue;
            };
            let reversed = cell.modifier.contains(Modifier::REVERSED);
            let bg = if reversed {
                svg_color(cell.fg, DEFAULT_FG)
            } else {
                svg_color(cell.bg, DEFAULT_BG)
            };
            if run_bg.as_deref() != Some(bg.as_str()) {
                flush(&mut out, &run_bg, run_start, x, y);
                run_bg = Some(bg);
                run_start = x;
            }
        }
        flush(&mut out, &run_bg, run_start, area.width, y);
    }
    // Text: runs of equal fg + weight per row; blanks break a run.
    for y in 0..area.height {
        // (x0, fg, bold, text, cells): a wide glyph's continuation cell has an
        // empty symbol and still counts, so the run's width stays honest.
        let mut run: Option<(u16, String, bool, String, u32)> = None;
        let flush = |out: &mut String, run: &Option<(u16, String, bool, String, u32)>, y: u16| {
            if let Some((x0, fg, bold, text, cells)) = run
                && !text.trim().is_empty()
            {
                // `textLength` pins the run to its cells whatever the viewer's
                // monospace advance is (DejaVu's is 7.83 px at 13 px, which
                // drifted a cell every ~46 characters without it).
                out.push_str(&format!(
                    "<text x=\"{}\" y=\"{}\" fill=\"{fg}\"{} textLength=\"{}\" lengthAdjust=\"spacingAndGlyphs\" xml:space=\"preserve\">{}</text>\n",
                    u32::from(*x0) * SVG_CELL_W,
                    u32::from(y) * SVG_CELL_H + 12,
                    if *bold { " font-weight=\"bold\"" } else { "" },
                    *cells * SVG_CELL_W,
                    xml_escape(text)
                ));
            }
        };
        for x in 0..area.width {
            let Some(cell) = buf.cell((area.x + x, area.y + y)) else {
                continue;
            };
            let reversed = cell.modifier.contains(Modifier::REVERSED);
            let fg = if reversed {
                svg_color(cell.bg, DEFAULT_BG)
            } else {
                svg_color(cell.fg, DEFAULT_FG)
            };
            let bold = cell.modifier.contains(Modifier::BOLD);
            let sym = cell.symbol();
            match &mut run {
                Some((_, rfg, rbold, text, cells)) if *rfg == fg && *rbold == bold => {
                    text.push_str(sym);
                    *cells += 1;
                }
                _ => {
                    flush(&mut out, &run, y);
                    run = Some((x, fg, bold, sym.to_string(), 1));
                }
            }
        }
        flush(&mut out, &run, y);
    }
    out.push_str("</svg>\n");
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
