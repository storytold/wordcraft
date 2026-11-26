//! Insert › Chart and the Chart Design tab: insert a chart, change its type, data, title, legend
//! and data labels.
//!
//! A chart is an inline [`InlineObject::Graphic`] whose graphic keeps its model
//! ([`ChartSpec`]); it is drawn by the same code that draws charts read from files, and saved as
//! a chart part with its data in it (see `wordcraft_docx::chart_xml`).
//!
//! A chart made by another program (Word, Excel…) keeps its own chart part, so it looks as it
//! did there. Edit Data, Chart Title, Legend and Data Labels change that part in place (and the
//! chart's embedded workbook, so Word's own Edit Data shows the new data), through
//! `wordcraft_docx::chart_edit`; everything else about its look is kept. Change Chart Type
//! needs WordCraft's model, so it stays with charts made in WordCraft.

use std::sync::Arc;

use serde_json::{Value, json};
use wordcraft_doc::Pos;
use wordcraft_doc::chart::{ChartAxis, ChartSeries, ChartSpec, ChartType, LegendPos, MAX_POINTS, MAX_SERIES, value_axis};
use wordcraft_doc::graphic::{Graphic, GraphicKind};
use wordcraft_doc::para::{Float, InlineObject};
use wordcraft_doc::props::Rgb;
use wordcraft_docx::chart_edit::{self, PartChart, PartEdit, SeriesData};

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
                // A scatter series is drawn as markers without a line; the other types draw it.
                if (c.kind == ChartType::Scatter) != (kind == ChartType::Scatter) {
                    for series in &mut c.series {
                        series.no_line = kind == ChartType::Scatter;
                        if series.no_line && series.markers == Some(false) {
                            series.markers = None;
                        }
                    }
                }
                // Pie and doughnut charts have a plain value axis and the other types a new
                // chart's value axis; an axis left as the old type had it takes the new type's.
                if c.kind.is_round() != kind.is_round() {
                    let (from, to) = if kind.is_round() { (value_axis(), ChartAxis::default()) } else { (ChartAxis::default(), value_axis()) };
                    if c.val_axis == from {
                        c.val_axis = to;
                    }
                }
                c.kind = kind;
                Ok(())
            })
        })
        .params(r#"{"type": "column|columnStacked|bar|barStacked|line|area|pie|doughnut|scatter"}"#)
        .when(has_model_chart),
        CommandSpec::new("chart.editData", "Edit Data", "Chart Design › Data", |s, v| {
            let d = v.get("data").unwrap_or(v);
            let (cats, series) = (d.get("categories"), d.get("series"));
            if cats.is_none() && series.is_none() {
                return Err(CmdError::Params(format!("`data` is required: {DATA}")));
            }
            let cats = cats.map(categories).transpose()?;
            let series = series.map(series_list).transpose()?;
            if selected_kept(s).is_some() {
                let series = series.map(|l| l.into_iter().map(|(name, values)| SeriesData { name, values }).collect());
                return edit_kept(s, &PartEdit::Data { categories: cats, series });
            }
            edit(s, move |c| {
                if let Some(cats) = &cats {
                    c.categories = cats.clone();
                }
                if let Some(series) = &series {
                    // Each series keeps the formatting of the one it replaces.
                    c.series = series
                        .iter()
                        .enumerate()
                        .map(|(i, (name, values))| match c.series.get(i) {
                            Some(old) => ChartSeries { name: name.clone(), values: values.clone(), ..old.clone() },
                            None => ChartSeries::new(c.kind, name.clone(), values.clone()),
                        })
                        .collect();
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
            if let Some((_, kept)) = selected_kept(s) {
                let title = match text {
                    Some(t) => t,
                    None if kept.title.is_some() => None,
                    None => Some("Chart Title".into()),
                };
                return edit_kept(s, &PartEdit::Title(title));
            }
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
            if let Some((_, kept)) = selected_kept(s) {
                let legend = match pos {
                    Some(p) => p,
                    None if kept.legend.is_some() => None,
                    None => Some(LegendPos::Bottom),
                };
                return edit_kept(s, &PartEdit::Legend(legend));
            }
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
            if let Some((_, kept)) = selected_kept(s) {
                return edit_kept(s, &PartEdit::DataLabels(on.unwrap_or(!kept.data_labels)));
            }
            edit(s, move |c| {
                c.data_labels = on.unwrap_or(!c.data_labels);
                // The chart's setting replaces each series' own.
                for series in &mut c.series {
                    series.labels = None;
                }
                Ok(())
            })
        })
        .params(r#"{"on"?: bool} (no value: switch the data labels on or off)"#)
        .when(has_chart),
    ]
}

/// The selected chart as Chart Design shows and edits it: its model, or, for a chart made in
/// another program, a model view of its data, title, legend and data labels (its look stays in
/// its own chart part).
pub fn selected_chart(s: &Session) -> Option<Arc<ChartSpec>> {
    match selected(s) {
        Some((_, InlineObject::Graphic { graphic, .. })) if graphic.chart.is_some() => graphic.chart.clone(),
        _ => selected_kept(s).map(|(_, kept)| Arc::new(kept.view())),
    }
}

/// Whether the selected chart was made in another program and is edited in its own chart part.
pub fn selected_chart_is_kept(s: &Session) -> bool {
    selected_kept(s).is_some()
}

/// The selected chart made by another program, when its chart part can be read: where it is,
/// and what Chart Design shows of it.
fn selected_kept(s: &Session) -> Option<(Pos, PartChart)> {
    let (pos, InlineObject::Graphic { graphic, .. }) = selected(s)? else { return None };
    if graphic.kind != GraphicKind::Chart || graphic.chart.is_some() {
        return None;
    }
    // A member picked inside a drawing canvas isn't the object at its place.
    let here = s.doc.para_at(&pos).and_then(|p| p.object_at(pos.off));
    if !matches!(here, Some(InlineObject::Graphic { graphic: g, .. }) if Arc::ptr_eq(g, &graphic)) {
        return None;
    }
    let src = graphic.source.as_deref()?;
    Some((pos, chart_edit::read_kept(&s.doc, src)?))
}

fn has_chart(s: &Session) -> Option<&'static str> {
    match selected(s) {
        Some((_, InlineObject::Graphic { graphic, .. })) if graphic.chart.is_some() => None,
        Some((_, InlineObject::Graphic { graphic, .. })) if graphic.kind == GraphicKind::Chart => {
            if selected_kept(s).is_some() {
                None
            } else {
                Some("this chart was made in another program and its chart data wasn't kept: WordCraft shows it but can't edit it")
            }
        }
        _ => Some("select a chart first"),
    }
}

/// `has_chart` for what needs WordCraft's model (Change Chart Type).
fn has_model_chart(s: &Session) -> Option<&'static str> {
    if selected_kept(s).is_some() {
        return Some("this chart was made in another program: its data, title, legend and data labels can be edited, its type can't yet");
    }
    has_chart(s)
}

/// Make `edit` to the selected chart made by another program: its chart part (and workbook) are
/// changed, and it is drawn again from the new part at its size.
fn edit_kept(s: &mut Session, edit: &PartEdit) -> CmdResult {
    let (pos, _) = selected_kept(s).ok_or_else(|| CmdError::Disabled("select a chart first".into()))?;
    let Some(InlineObject::Graphic { w, h, graphic, .. }) = s.doc.para_at(&pos).and_then(|p| p.object_at(pos.off)).cloned() else {
        return Err(CmdError::Disabled("select a chart first".into()));
    };
    let src = graphic.source.clone().ok_or_else(|| CmdError::Disabled("this chart can't be edited".into()))?;
    let done = chart_edit::edit_kept(&mut s.doc, &src, edit).map_err(|e| CmdError::Failed(e.0))?;
    let (dw, dh) = if graphic.w > 0.0 && graphic.h > 0.0 { (graphic.w, graphic.h) } else { (w, h) };
    let items = chart_edit::draw_part(&done.part, &s.doc.settings.theme_colors, dw, dh);
    let next = Arc::new(Graphic { items, source: Some(done.source.clone()), w: dw, h: dh, ..(*graphic).clone() });
    let view = chart_edit::read_part(&done.part).map(|c| c.view());
    let para = s.doc.para_mut(pos.story, &pos.path)?;
    if let Some(InlineObject::Graphic { graphic, .. }) = para.object_at_mut(pos.off) {
        *graphic = next;
    }
    para.touch();
    let mut out = json!({"chart": view, "madeElsewhere": true, "workbook": done.workbook});
    if done.workbook == "linked" {
        out["note"] = json!(
            "the chart's data comes from a workbook outside the document, which isn't changed: Word shows the old data again when it updates the link"
        );
    }
    Ok(out)
}

/// The graphic for `spec` drawn `w` × `h` points in the theme's colours.
pub fn chart_graphic(spec: ChartSpec, theme: &[Rgb], w: f32, h: f32) -> Graphic {
    let spec = spec.sanitized();
    let items = wordcraft_docx::chart_items(&spec, theme, w, h);
    Graphic { kind: GraphicKind::Chart, items, w, h, source: None, chart: Some(Arc::new(spec)), smart_art: None }
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
        s.push_ui_request(json!({"open": "insertChart"}));
        return sel_result(s);
    }
    let kind = kind.unwrap_or_default();
    let mut spec = ChartSpec::sample(kind);
    if let Some(d) = v.get("data") {
        spec.categories = d.get("categories").map(categories).transpose()?.unwrap_or_default();
        spec.series = d
            .get("series")
            .map(series_list)
            .transpose()?
            .unwrap_or_default()
            .into_iter()
            .map(|(name, values)| ChartSeries::new(kind, name, values))
            .collect();
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

/// Draw the chart of `o` again at its current size (after a resize, or when it opens), if it
/// has a model. It keeps the chart as read (its `source`): the chart itself is unchanged, and
/// saving writes that back with the look WordCraft doesn't model.
pub fn redraw(o: &mut InlineObject, theme: &[Rgb]) {
    if let InlineObject::Graphic { w, h, graphic, .. } = o
        && let Some(spec) = graphic.chart.as_deref()
    {
        let source = graphic.source.clone();
        *graphic = Arc::new(Graphic { source, ..chart_graphic(spec.clone(), theme, *w, *h) });
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

/// Series names and values from JSON: `[{"name", "values": [number|null|numeric string]}]`.
fn series_list(v: &Value) -> Result<Vec<(String, Vec<Option<f64>>)>, CmdError> {
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
            Ok((name, values))
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
