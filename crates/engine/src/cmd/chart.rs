//! Insert › Chart and the Chart Design tab: insert a chart, change its type, data, title, legend
//! and data labels.
//!
//! A chart is an inline [`InlineObject::Graphic`] whose graphic keeps its model
//! ([`ChartSpec`]); it is drawn by the same code that draws charts read from files, and saved as
//! a chart part with its data in it (see `wordcraft_docx::chart_xml`). Charts made by other
//! programs are shown as they are; the chart commands only edit charts with a model.

use std::sync::Arc;

use serde_json::{Value, json};
use wordcraft_doc::chart::{ChartSeries, ChartSpec, ChartType, LegendPos, MAX_POINTS, MAX_SERIES};
use wordcraft_doc::graphic::{Graphic, GraphicKind};
use wordcraft_doc::para::{Float, InlineObject};
use wordcraft_doc::props::Rgb;

use super::objects::selected;
use super::{delete_selection, sel_result};
use crate::{CmdError, CmdResult, CommandSpec, Selection, Session, p};

/// A new chart's size: 15 × 7.5 cm, points.
pub const DEFAULT_W: f32 = 15.0 / 2.54 * 72.0;
pub const DEFAULT_H: f32 = 7.5 / 2.54 * 72.0;

const TYPES: &str = "column|columnStacked|bar|barStacked|line|area|pie|doughnut|scatter";
const DATA: &str = r#"{"categories": [string], "series": [{"name": string, "values": [number|null]}]}"#;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("insert.chart", "Chart", "Insert › Illustrations", insert).params(
            r#"{"type"?: "column|columnStacked|bar|barStacked|line|area|pie|doughnut|scatter", "data"?: {"categories": [string], "series": [{"name": string, "values": [number|null]}]}, "title"?: string, "legend"?: "right|left|top|bottom|none", "dataLabels"?: bool, "width"?: pt, "height"?: pt} (no params: the chart type picker)"#,
        ),
        CommandSpec::new("chart.type", "Change Chart Type", "Chart Design › Type", |s, v| {
            let kind = chart_type(v)?.ok_or_else(|| CmdError::Params(format!("`type` is required: {TYPES}")))?;
            edit(s, |c| {
                c.kind = kind;
                Ok(())
            })
        })
        .params(r#"{"type": "column|columnStacked|bar|barStacked|line|area|pie|doughnut|scatter"}"#)
        .when(has_chart),
        CommandSpec::new("chart.editData", "Edit Data", "Chart Design › Data", |s, v| {
            let d = v.get("data").unwrap_or(v);
            let (cats, series) = (d.get("categories"), d.get("series"));
            if cats.is_none() && series.is_none() {
                return Err(CmdError::Params(format!("`data` is required: {DATA}")));
            }
            let cats = cats.map(categories).transpose()?;
            let series = series.map(series_list).transpose()?;
            edit(s, move |c| {
                if let Some(cats) = &cats {
                    c.categories = cats.clone();
                }
                if let Some(series) = &series {
                    c.series = series.clone();
                }
                Ok(())
            })
        })
        .params(r#"{"data": {"categories"?: [string], "series"?: [{"name": string, "values": [number|null]}]}}"#)
        .when(has_chart),
        CommandSpec::new("chart.title", "Chart Title", "Chart Design › Chart Layouts › Add Chart Element", |s, v| {
            let text = match v.get("text") {
                Some(Value::String(t)) => Some(Some(t.clone())),
                Some(Value::Null) => Some(None),
                None => None,
                Some(_) => return Err(CmdError::Params("`text`: a string, or null for no title".into())),
            };
            edit(s, move |c| {
                c.title = match &text {
                    Some(t) => t.clone(),
                    // No text given: switch the title on (placeholder) or off.
                    None if c.title.is_some() => None,
                    None => Some("Chart Title".into()),
                };
                Ok(())
            })
        })
        .params(r#"{"text"?: string|null} (no text: switch the title on or off)"#)
        .when(has_chart),
        CommandSpec::new("chart.legend", "Legend", "Chart Design › Chart Layouts › Add Chart Element", |s, v| {
            let pos = match p::str(v, "pos").or_else(|| p::str(v, "position")) {
                Some(id) if id.eq_ignore_ascii_case("none") => Some(None),
                Some(id) => Some(Some(LegendPos::from_id(id).ok_or_else(|| CmdError::Params("`pos`: right|left|top|bottom|none".into()))?)),
                None => None,
            };
            edit(s, move |c| {
                c.legend = match pos {
                    Some(p) => p,
                    None if c.legend.is_some() => None,
                    None => Some(LegendPos::Bottom),
                };
                Ok(())
            })
        })
        .params(r#"{"pos"?: "right|left|top|bottom|none"} (no pos: switch the legend on or off)"#)
        .when(has_chart),
        CommandSpec::new("chart.dataLabels", "Data Labels", "Chart Design › Chart Layouts › Add Chart Element", |s, v| {
            let on = p::bool(v, "on").or_else(|| p::bool(v, "value"));
            edit(s, move |c| {
                c.data_labels = on.unwrap_or(!c.data_labels);
                Ok(())
            })
        })
        .params(r#"{"on"?: bool} (no value: switch the data labels on or off)"#)
        .when(has_chart),
    ]
}

/// The selected chart's model, when it has one.
pub fn selected_chart(s: &Session) -> Option<Arc<ChartSpec>> {
    match selected(s) {
        Some((_, InlineObject::Graphic { graphic, .. })) => graphic.chart.clone(),
        _ => None,
    }
}

fn has_chart(s: &Session) -> Option<&'static str> {
    match selected(s) {
        Some((_, InlineObject::Graphic { graphic, .. })) if graphic.chart.is_some() => None,
        Some((_, InlineObject::Graphic { graphic, .. })) if graphic.kind == GraphicKind::Chart => {
            Some("this chart was made in another program: WordCraft shows it but can't edit it")
        }
        _ => Some("select a chart first"),
    }
}

/// The graphic for `spec` drawn `w` × `h` points in the theme's colours.
pub fn chart_graphic(spec: ChartSpec, theme: &[Rgb], w: f32, h: f32) -> Graphic {
    let spec = spec.sanitized();
    let items = wordcraft_docx::chart_items(&spec, theme, w, h);
    Graphic { kind: GraphicKind::Chart, items, w, h, source: None, chart: Some(Arc::new(spec)) }
}

/// `type` of the params, if given.
fn chart_type(v: &Value) -> Result<Option<ChartType>, CmdError> {
    match p::str(v, "type") {
        Some(id) => ChartType::from_id(id).map(Some).ok_or_else(|| CmdError::Params(format!("unknown chart type `{id}`: {TYPES}"))),
        None if v.get("type").is_some_and(|t| !t.is_null()) => Err(CmdError::Params(format!("`type`: {TYPES}"))),
        None => Ok(None),
    }
}

fn insert(s: &mut Session, v: &Value) -> CmdResult {
    let kind = chart_type(v)?;
    // No type and no data: Insert › Chart's picker, as the ribbon button does.
    if kind.is_none() && v.get("data").is_none() {
        s.ui_requests.push(json!({"open": "insertChart"}));
        return sel_result(s);
    }
    let kind = kind.unwrap_or_default();
    let mut spec = ChartSpec::sample(kind);
    if let Some(d) = v.get("data") {
        spec.categories = d.get("categories").map(categories).transpose()?.unwrap_or_default();
        spec.series = d.get("series").map(series_list).transpose()?.unwrap_or_default();
        if spec.series.is_empty() {
            return Err(CmdError::Params(format!("`data.series` needs at least one series: {DATA}")));
        }
    }
    spec.title = p::str(v, "title").map(str::to_string);
    match p::str(v, "legend") {
        Some(id) if id.eq_ignore_ascii_case("none") => spec.legend = None,
        Some(id) => spec.legend = Some(LegendPos::from_id(id).ok_or_else(|| CmdError::Params("`legend`: right|left|top|bottom|none".into()))?),
        None => {}
    }
    spec.data_labels = p::bool(v, "dataLabels").unwrap_or(false);
    // Fitted to the text width, as Word sizes a new chart.
    let max_w = s.doc.sections().first().map(|(_, sp)| sp.text_width()).unwrap_or(468.0).max(36.0);
    let w = p::f32(v, "width").unwrap_or(DEFAULT_W).clamp(36.0, 2000.0).min(max_w);
    let h = p::f32(v, "height").unwrap_or(DEFAULT_H).clamp(36.0, 2000.0);
    let graphic = chart_graphic(spec, &s.doc.settings.theme_colors, w, h);
    let out = json!({"chart": graphic.chart.as_deref(), "width": w, "height": h});
    let props = s.typing_props();
    let at = delete_selection(s)?;
    let obj = InlineObject::Graphic { w, h, alt: String::new(), float: Float::default(), graphic: Arc::new(graphic) };
    let end = s.doc.insert_object(&at, obj, &props)?;
    s.sel = Selection { anchor: at, focus: end };
    Ok(out)
}

/// Change the selected chart's model with `f`, then draw it again at its size.
fn edit(s: &mut Session, f: impl Fn(&mut ChartSpec) -> Result<(), CmdError>) -> CmdResult {
    let (pos, obj) = selected(s).ok_or_else(|| CmdError::Disabled("select a chart first".into()))?;
    let InlineObject::Graphic { w, h, graphic, .. } = obj else { return Err(CmdError::Disabled("select a chart first".into())) };
    let mut spec = graphic.chart.as_deref().cloned().ok_or_else(|| CmdError::Disabled("this chart can't be edited".into()))?;
    f(&mut spec)?;
    let next = Arc::new(chart_graphic(spec, &s.doc.settings.theme_colors, w, h));
    let out = json!({"chart": next.chart.as_deref()});
    let para = s.doc.para_mut(pos.story, &pos.path)?;
    if let Some(InlineObject::Graphic { graphic, .. }) = para.object_at_mut(pos.off) {
        *graphic = next;
    }
    para.touch();
    Ok(out)
}

/// Draw the chart of `o` again at its current size (after a resize), if it has a model.
pub fn redraw(o: &mut InlineObject, theme: &[Rgb]) {
    if let InlineObject::Graphic { w, h, graphic, .. } = o
        && let Some(spec) = graphic.chart.as_deref()
    {
        *graphic = Arc::new(chart_graphic(spec.clone(), theme, *w, *h));
    }
}

/// Category labels from JSON: strings, or numbers written as text.
fn categories(v: &Value) -> Result<Vec<String>, CmdError> {
    let a = v.as_array().ok_or_else(|| CmdError::Params("`categories`: an array of strings".into()))?;
    Ok(a.iter().take(MAX_POINTS).map(cell_text).collect())
}

fn cell_text(v: &Value) -> String {
    match v {
        Value::String(t) => wordcraft_doc::chart::clean_text(t),
        Value::Null => String::new(),
        other => wordcraft_doc::chart::clean_text(&other.to_string()),
    }
}

/// Series from JSON: `[{"name", "values": [number|null|numeric string]}]`.
fn series_list(v: &Value) -> Result<Vec<ChartSeries>, CmdError> {
    let a = v.as_array().ok_or_else(|| CmdError::Params(format!("`series`: {DATA}")))?;
    a.iter()
        .take(MAX_SERIES)
        .map(|s| {
            let name = s.get("name").map(cell_text).unwrap_or_default();
            let values = match s.get("values") {
                Some(Value::Array(vals)) => vals.iter().take(MAX_POINTS).map(number).collect(),
                None | Some(Value::Null) => Vec::new(),
                Some(_) => return Err(CmdError::Params("`values`: an array of numbers (null for none)".into())),
            };
            Ok(ChartSeries { name, values })
        })
        .collect()
}

/// A value: a number, or text that reads as one; anything else (or not finite) is no value.
fn number(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        Value::String(t) => t.trim().parse::<f64>().ok(),
        _ => None,
    }
    .filter(|x| x.is_finite())
}
