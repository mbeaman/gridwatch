//! Reading a plugin's view tree (§4.7, arc 8b).
//!
//! A plugin sends JSON; the host turns it into the same `View` a built-in
//! component returns, and the theme's renderer draws it. The shapes below
//! are the subset this contract accepts — text, key/value pairs, gauges,
//! bars, sparklines, tables, big digits and stacks of those. A shape that
//! is not here is refused by name, so a plugin author reads a sentence
//! rather than seeing a blank tile.
//!
//! Everything here is defensive: the depth is capped, every count is
//! capped, and a number that is not finite is refused rather than drawn.

use gridwatch_ui::theme::{GradientId, Role};
use gridwatch_ui::view::{ColWidth, Column, Constraint, Dir, Line, Span, View};

/// How deep a plugin's tree may nest. Deeper than this is not a layout, it
/// is a way to spend the host's stack.
pub const MAX_DEPTH: usize = 8;
/// The most children, rows, columns or spans in any one node.
pub const MAX_ITEMS: usize = 512;

type R<T> = Result<T, String>;

fn role_of(name: &str) -> R<Role> {
    Ok(match name {
        "text" => Role::Text,
        "text_muted" => Role::TextMuted,
        "text_ghost" => Role::TextGhost,
        "accent_primary" => Role::AccentPrimary,
        "accent_secondary" => Role::AccentSecondary,
        "ok" => Role::Ok,
        "warn" => Role::Warn,
        "crit" => Role::Crit,
        other => {
            return Err(format!(
                "role `{other}`: this contract has text, text_muted, text_ghost, \
                 accent_primary, accent_secondary, ok, warn, crit"
            ));
        }
    })
}

fn gradient_of(name: &str) -> R<GradientId> {
    Ok(match name {
        "load" => GradientId::Load,
        "mem" => GradientId::Mem,
        "temp" => GradientId::Temp,
        "power" => GradientId::Power,
        "net_rx" => GradientId::NetRx,
        "net_tx" => GradientId::NetTx,
        "audio" => GradientId::Audio,
        other => return Err(format!("gradient `{other}` is not one this theme defines")),
    })
}

fn number(v: &serde_json::Value, what: &str) -> R<f32> {
    let n = v
        .as_f64()
        .ok_or_else(|| format!("{what}: expected a number, got {v}"))?;
    if !n.is_finite() {
        return Err(format!("{what}: {n} is not a number anyone can draw"));
    }
    Ok(n as f32)
}

fn text_of(v: &serde_json::Value, what: &str) -> R<String> {
    v.as_str()
        .map(|s| s.to_string())
        .ok_or_else(|| format!("{what}: expected a string"))
}

/// One span: `{"role": "...", "text": "..."}`, or a bare string (which is
/// `text`, the role a plugin means nine times in ten).
fn span(v: &serde_json::Value) -> R<Span> {
    if let Some(s) = v.as_str() {
        return Ok(Span::new(Role::Text, s.to_string()));
    }
    let obj = v.as_object().ok_or("a span is a string or an object")?;
    let role = match obj.get("role") {
        Some(r) => role_of(&text_of(r, "role")?)?,
        None => Role::Text,
    };
    let text = text_of(obj.get("text").ok_or("a span needs `text`")?, "text")?;
    let bold = obj.get("bold").and_then(|b| b.as_bool()).unwrap_or(false);
    Ok(if bold {
        Span::bold(role, text)
    } else {
        Span::new(role, text)
    })
}

fn line(v: &serde_json::Value) -> R<Line> {
    let items = v.as_array().ok_or("a line is an array of spans")?;
    if items.len() > MAX_ITEMS {
        return Err(format!(
            "a line of {} spans (the cap is {MAX_ITEMS})",
            items.len()
        ));
    }
    items.iter().map(span).collect()
}

fn lines(v: &serde_json::Value) -> R<Vec<Line>> {
    let items = v.as_array().ok_or("expected an array of lines")?;
    if items.len() > MAX_ITEMS {
        return Err(format!("{} lines (the cap is {MAX_ITEMS})", items.len()));
    }
    items.iter().map(line).collect()
}

fn constraint(v: &serde_json::Value) -> R<Constraint> {
    let obj = v.as_object().ok_or("a constraint is an object")?;
    if let Some(n) = obj.get("len") {
        return Ok(Constraint::Len(number(n, "len")?.max(0.0) as u16));
    }
    if let Some(n) = obj.get("fill") {
        return Ok(Constraint::Fill(number(n, "fill")?.max(1.0) as u16));
    }
    Err("a constraint is {\"len\": n} or {\"fill\": n}".into())
}

/// The whole tree. `depth` counts down.
pub fn view_from_json(v: &serde_json::Value, depth: usize) -> R<View> {
    if depth == 0 {
        return Err(format!("nested deeper than {MAX_DEPTH}"));
    }
    if v.as_str() == Some("empty") {
        return Ok(View::Empty);
    }
    let obj = v
        .as_object()
        .ok_or("a view is an object, or the string \"empty\"")?;
    let (name, body) = obj
        .iter()
        .next()
        .ok_or("an empty object is not a view; use \"empty\"")?;
    if obj.len() != 1 {
        return Err(format!(
            "a view names one shape; this one names {}",
            obj.keys().cloned().collect::<Vec<_>>().join(", ")
        ));
    }
    match name.as_str() {
        "text" => Ok(View::Text(lines(body)?)),
        "kv" => {
            let items = body.as_array().ok_or("kv is an array of pairs")?;
            if items.len() > MAX_ITEMS {
                return Err(format!("{} pairs (the cap is {MAX_ITEMS})", items.len()));
            }
            let mut out = Vec::with_capacity(items.len());
            for pair in items {
                let p = pair.as_array().ok_or("a kv pair is [key, line]")?;
                let key = text_of(p.first().ok_or("a kv pair needs a key")?, "kv key")?;
                let value = line(p.get(1).ok_or("a kv pair needs a value")?)?;
                out.push((std::borrow::Cow::Owned(key), value, None));
            }
            Ok(View::KeyValue(out))
        }
        "gauge" => {
            let o = body.as_object().ok_or("gauge is an object")?;
            Ok(View::Gauge {
                label: text_of(o.get("label").ok_or("gauge needs a label")?, "label")?.into(),
                value: number(o.get("value").ok_or("gauge needs a value")?, "value")?
                    .clamp(0.0, 1.0),
                gradient: gradient_of(
                    o.get("gradient")
                        .map(|g| text_of(g, "gradient"))
                        .transpose()?
                        .unwrap_or_else(|| "load".into())
                        .as_str(),
                )?,
                text: o
                    .get("text")
                    .map(|t| text_of(t, "text"))
                    .transpose()?
                    .map(Into::into),
            })
        }
        "bars" => {
            let o = body.as_object().ok_or("bars is an object")?;
            let values = o
                .get("values")
                .and_then(|v| v.as_array())
                .ok_or("bars needs values")?;
            if values.len() > MAX_ITEMS {
                return Err(format!("{} bars (the cap is {MAX_ITEMS})", values.len()));
            }
            Ok(View::Bars {
                values: values
                    .iter()
                    .map(|v| number(v, "a bar").map(|n| n.clamp(0.0, 1.0)))
                    .collect::<R<Vec<f32>>>()?,
                gradient: gradient_of(
                    o.get("gradient")
                        .map(|g| text_of(g, "gradient"))
                        .transpose()?
                        .unwrap_or_else(|| "load".into())
                        .as_str(),
                )?,
                labels: None,
                peaks: None,
            })
        }
        "sparkline" => {
            let o = body.as_object().ok_or("sparkline is an object")?;
            let series = o
                .get("series")
                .and_then(|v| v.as_array())
                .ok_or("sparkline needs a series")?;
            if series.len() > MAX_ITEMS {
                return Err(format!("{} points (the cap is {MAX_ITEMS})", series.len()));
            }
            Ok(View::Sparkline {
                series: series
                    .iter()
                    .map(|v| {
                        if v.is_null() {
                            Ok(None)
                        } else {
                            number(v, "a point").map(Some)
                        }
                    })
                    .collect::<R<Vec<Option<f32>>>>()?,
                gradient: gradient_of(
                    o.get("gradient")
                        .map(|g| text_of(g, "gradient"))
                        .transpose()?
                        .unwrap_or_else(|| "load".into())
                        .as_str(),
                )?,
                max: o.get("max").map(|m| number(m, "max")).transpose()?,
            })
        }
        "table" => {
            let o = body.as_object().ok_or("table is an object")?;
            let cols = o
                .get("columns")
                .and_then(|v| v.as_array())
                .ok_or("table needs columns")?;
            if cols.len() > MAX_ITEMS {
                return Err(format!("{} columns (the cap is {MAX_ITEMS})", cols.len()));
            }
            let columns = cols
                .iter()
                .map(|c| -> R<Column> {
                    let co = c.as_object().ok_or("a column is an object")?;
                    Ok(Column {
                        title: text_of(co.get("title").ok_or("a column needs a title")?, "title")?
                            .into(),
                        width: match co.get("width") {
                            Some(w) if w.as_str() == Some("elastic") => ColWidth::Elastic,
                            Some(w) => ColWidth::Fixed(number(w, "width")?.max(0.0) as u16),
                            None => ColWidth::Elastic,
                        },
                        right: co.get("right").and_then(|r| r.as_bool()).unwrap_or(false),
                    })
                })
                .collect::<R<Vec<Column>>>()?;
            let rows_json = o
                .get("rows")
                .and_then(|v| v.as_array())
                .ok_or("table needs rows")?;
            if rows_json.len() > MAX_ITEMS {
                return Err(format!("{} rows (the cap is {MAX_ITEMS})", rows_json.len()));
            }
            let rows = rows_json
                .iter()
                .map(|r| {
                    r.as_array()
                        .ok_or_else(|| "a row is an array of cells".to_string())?
                        .iter()
                        .map(line)
                        .collect::<R<Vec<Line>>>()
                })
                .collect::<R<Vec<Vec<Line>>>>()?;
            Ok(View::Table {
                columns,
                rows,
                selected: o
                    .get("selected")
                    .and_then(|s| s.as_u64())
                    .map(|s| s as usize),
                sort: None,
                scroll: o.get("scroll").and_then(|s| s.as_u64()).unwrap_or(0) as usize,
            })
        }
        "big" => {
            let o = body.as_object().ok_or("big is an object")?;
            Ok(View::BigNumber {
                text: text_of(o.get("text").ok_or("big needs text")?, "text")?.into(),
                role: match o.get("role") {
                    Some(r) => role_of(&text_of(r, "role")?)?,
                    None => Role::Text,
                },
            })
        }
        "stack" => {
            let o = body.as_object().ok_or("stack is an object")?;
            let dir = match o
                .get("dir")
                .map(|d| text_of(d, "dir"))
                .transpose()?
                .as_deref()
            {
                Some("h") | Some("horizontal") => Dir::H,
                _ => Dir::V,
            };
            let kids = o
                .get("children")
                .and_then(|c| c.as_array())
                .ok_or("a stack needs children")?;
            if kids.len() > MAX_ITEMS {
                return Err(format!("{} children (the cap is {MAX_ITEMS})", kids.len()));
            }
            let children = kids
                .iter()
                .map(|k| -> R<(Constraint, View)> {
                    let pair = k.as_array().ok_or("a child is [constraint, view]")?;
                    Ok((
                        constraint(pair.first().ok_or("a child needs a constraint")?)?,
                        view_from_json(pair.get(1).ok_or("a child needs a view")?, depth - 1)?,
                    ))
                })
                .collect::<R<Vec<(Constraint, View)>>>()?;
            Ok(View::Stack { dir, children })
        }
        other => Err(format!(
            "shape `{other}`: this contract draws empty, text, kv, gauge, bars, \
             sparkline, table, big and stack"
        )),
    }
}

/// The entry point: the whole tree, at full depth.
pub fn read(v: &serde_json::Value) -> R<View> {
    view_from_json(v, MAX_DEPTH)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn json(s: &str) -> serde_json::Value {
        serde_json::from_str(s).expect("valid json in the test")
    }

    #[test]
    fn the_shapes_a_plugin_may_draw() {
        assert!(matches!(read(&json(r#""empty""#)).unwrap(), View::Empty));
        // A bare string is a span with the plain text role.
        let v = read(&json(
            r#"{"text":[["hello",{"role":"ok","text":"!","bold":true}]]}"#,
        ))
        .unwrap();
        let View::Text(ls) = v else { panic!() };
        assert_eq!(ls[0].len(), 2);
        assert_eq!(ls[0][0].text.as_ref(), "hello");
        assert_eq!(ls[0][1].role, Role::Ok);
        // A gauge clamps its value rather than drawing off the end.
        let v = read(&json(
            r#"{"gauge":{"label":"x","value":2.5,"gradient":"temp"}}"#,
        ))
        .unwrap();
        let View::Gauge { value, .. } = v else {
            panic!()
        };
        assert_eq!(value, 1.0);
        // Nested stacks, and a table.
        let v = read(&json(
            r#"{"stack":{"dir":"v","children":[
                 [{"len":1},{"text":[["top"]]}],
                 [{"fill":1},{"table":{"columns":[{"title":"a"},{"title":"b","width":4,"right":true}],
                                       "rows":[[["1"],["2"]]],"selected":0}}]]}}"#,
        ))
        .unwrap();
        let View::Stack { children, .. } = v else {
            panic!()
        };
        assert_eq!(children.len(), 2);
        assert!(matches!(children[1].1, View::Table { .. }));
        // Sparkline gaps are nulls, as they are in the host's own views.
        let v = read(&json(
            r#"{"sparkline":{"series":[1,null,3],"gradient":"net_rx"}}"#,
        ))
        .unwrap();
        let View::Sparkline { series, .. } = v else {
            panic!()
        };
        assert_eq!(series, vec![Some(1.0), None, Some(3.0)]);
    }

    #[test]
    fn what_a_plugin_may_not_draw_is_refused_by_name() {
        let refuse = |s: &str| read(&json(s)).unwrap_err();
        assert!(refuse(r#"{"custom":"anything"}"#).contains("shape `custom`"));
        assert!(refuse(r#"{"text":[["x"]],"gauge":{}}"#).contains("names one shape"));
        assert!(refuse(r#"{"text":[[{"role":"neon","text":"x"}]]}"#).contains("role `neon`"));
        assert!(
            refuse(r#"{"gauge":{"label":"x","value":1,"gradient":"rainbow"}}"#)
                .contains("gradient `rainbow`")
        );
        assert!(refuse(r#"{"gauge":{"value":1}}"#).contains("label"));
        assert!(refuse(r#"[]"#).contains("a view is an object"));
        assert!(refuse(r#"{}"#).contains("empty object"));
        // A number nobody can draw. serde_json refuses an out-of-range
        // literal itself, so this builds the value directly — which is
        // also how it would arrive from a plugin whose language has a NaN.
        let mut bars = serde_json::Map::new();
        let mut body = serde_json::Map::new();
        body.insert(
            "values".into(),
            serde_json::Value::Array(vec![serde_json::Value::String("not a number".into())]),
        );
        bars.insert("bars".into(), serde_json::Value::Object(body));
        let e = read(&serde_json::Value::Object(bars)).unwrap_err();
        assert!(e.contains("expected a number"), "{e}");
    }

    #[test]
    fn a_tree_cannot_nest_the_host_to_death() {
        // One deeper than the cap.
        let mut s = r#"{"text":[["deep"]]}"#.to_string();
        for _ in 0..=MAX_DEPTH {
            s = format!(r#"{{"stack":{{"dir":"v","children":[[{{"fill":1}},{s}]]}}}}"#);
        }
        let e = read(&json(&s)).unwrap_err();
        assert!(e.contains("nested deeper"), "{e}");
        // And a node with too many children.
        let many: Vec<String> = (0..MAX_ITEMS + 1)
            .map(|_| r#"[{"fill":1},"empty"]"#.to_string())
            .collect();
        let s = format!(
            r#"{{"stack":{{"dir":"v","children":[{}]}}}}"#,
            many.join(",")
        );
        assert!(read(&json(&s)).unwrap_err().contains("the cap is"));
    }

    /// The example plugin's own tree, read by the host that will draw it.
    #[test]
    fn the_example_plugins_tree_reads() {
        let tree = json(
            r#"{"stack":{"dir":"v","children":[
                [{"len":1},{"text":[[{"role":"accent_primary","text":"14.9°C"}]]}],
                [{"fill":1},{"text":[[{"role":"text_muted","text":"/tmp/gridwatch-weather"}]]}]]}}"#,
        );
        let v = read(&tree).expect("the example draws");
        assert!(matches!(v, View::Stack { .. }));
    }
}
