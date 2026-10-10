//! Draw tab and Review › Ink: pens, the eraser and ink strokes.
//!
//! An ink stroke is a floating freeform shape ([`Freeform`] with its pen set) anchored to the
//! paragraph under where it starts, in front of the text (a highlighter's behind it). The pen
//! commands only choose what dragging on the page does (session view state); `draw.stroke` and
//! `draw.erase` change the document.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use wordcraft_doc::freeform::{Freeform, InkTool, MAX_INK_PER_PAGE, MAX_POINTS};
use wordcraft_doc::para::{Anchor, Float, InlineObject, OBJ, ShapeKind, Wrap};
use wordcraft_doc::props::Rgb;
use wordcraft_doc::{CharProps, Pos, StoryRef};
use wordcraft_layout::{DocLayout, Placed};

use super::pos_json;
use crate::{CmdError, CmdResult, CommandSpec, Session, p};

/// What dragging on the page does (Draw tab).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DrawMode {
    /// Ordinary selection of text and drawings.
    #[default]
    Select,
    /// Dragging over ink strokes deletes them.
    Eraser,
    /// Dragging draws ink with this pen.
    Pen(InkTool),
}

impl DrawMode {
    /// The pen it draws with, if it's one.
    pub fn pen(self) -> Option<InkTool> {
        match self {
            DrawMode::Pen(t) => Some(t),
            _ => None,
        }
    }
}

/// A pen's colour and thickness.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PenSettings {
    pub color: Rgb,
    /// Points.
    pub width: f32,
}

impl PenSettings {
    fn of(tool: InkTool) -> PenSettings {
        PenSettings { color: tool.default_color(), width: tool.default_width() }
    }
}

/// The Draw tab's state: the tool in use and each pen's settings.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct DrawState {
    pub mode: DrawMode,
    pub pen: PenSettings,
    pub pencil: PenSettings,
    pub highlighter: PenSettings,
}

impl Default for DrawState {
    fn default() -> Self {
        DrawState {
            mode: DrawMode::Select,
            pen: PenSettings::of(InkTool::Pen),
            pencil: PenSettings::of(InkTool::Pencil),
            highlighter: PenSettings::of(InkTool::Highlighter),
        }
    }
}

impl DrawState {
    pub fn settings(&self, tool: InkTool) -> PenSettings {
        match tool {
            InkTool::Pen => self.pen,
            InkTool::Pencil => self.pencil,
            InkTool::Highlighter => self.highlighter,
        }
    }
    fn settings_mut(&mut self, tool: InkTool) -> &mut PenSettings {
        match tool {
            InkTool::Pen => &mut self.pen,
            InkTool::Pencil => &mut self.pencil,
            InkTool::Highlighter => &mut self.highlighter,
        }
    }
}

/// Thinnest and thickest ink, points.
const MIN_WIDTH: f32 = 0.25;
const MAX_WIDTH: f32 = 72.0;
/// How far from a stroke (beyond its half width) the eraser still takes it, points.
const ERASE_SLOP: f32 = 3.0;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("draw.select", "Select", "Draw › Tools", |s, _| {
            s.view.draw.mode = DrawMode::Select;
            state(s)
        })
        .pure(),
        CommandSpec::new("draw.eraser", "Eraser", "Draw › Tools", |s, v| {
            // With a point, erase there (as a script would); without, pick up the eraser.
            if v.get("page").is_some() || v.get("x").is_some() || v.get("y").is_some() {
                return s.run("draw.erase", v);
            }
            s.view.draw.mode = DrawMode::Eraser;
            state(s)
        })
        .params(r#"{"page"?: n, "x"?: pt, "y"?: pt}  (no point: use the eraser; a point: delete the ink stroke there)"#)
        .pure(),
        CommandSpec::new("draw.pen", "Pen", "Draw › Pens", |s, v| pen(s, v, InkTool::Pen)).params(PEN_PARAMS).pure(),
        CommandSpec::new("draw.pencil", "Pencil", "Draw › Pens", |s, v| pen(s, v, InkTool::Pencil)).params(PEN_PARAMS).pure(),
        CommandSpec::new("draw.highlighter", "Highlighter", "Draw › Pens", |s, v| pen(s, v, InkTool::Highlighter)).params(PEN_PARAMS).pure(),
        CommandSpec::new("draw.stroke", "Draw Ink", "Draw › Pens", stroke).params(
            r#"{"page": n, "points": [[x, y], …] (page points, at most 5000), "tool"?: "pen|pencil|highlighter", "color"?: "RRGGBB", "width"?: pt}"#,
        ),
        CommandSpec::new("draw.erase", "Erase Ink", "Draw › Tools", erase).params(r#"{"page": n, "x": pt, "y": pt}"#),
        CommandSpec::new("review.hideInk", "Hide Ink", "Review › Ink", |s, v| {
            s.view.hide_ink = p::bool(v, "value").unwrap_or(!s.view.hide_ink);
            Ok(json!({"value": s.view.hide_ink}))
        })
        .params(r#"{"value"?: bool}"#)
        .pure(),
    ]
}

const PEN_PARAMS: &str = r#"{"color"?: "RRGGBB", "width"?: pt}"#;

fn state(s: &Session) -> CmdResult {
    serde_json::to_value(&s.view.draw).map_err(|e| CmdError::Failed(e.to_string()))
}

/// Pick up a pen, optionally changing its colour and thickness.
fn pen(s: &mut Session, v: &Value, tool: InkTool) -> CmdResult {
    let color = p::str(v, "color").map(|c| Rgb::parse(c).ok_or_else(|| CmdError::Params("`color` is RRGGBB".into()))).transpose()?;
    let set = s.view.draw.settings_mut(tool);
    if let Some(c) = color {
        set.color = c;
    }
    if let Some(w) = p::f32(v, "width") {
        set.width = w.clamp(MIN_WIDTH, MAX_WIDTH);
    }
    s.view.draw.mode = DrawMode::Pen(tool);
    state(s)
}

/// Ink strokes laid out on `page`: their position and geometry on the page.
fn ink_on(s: &Session, layout: &DocLayout, page: usize) -> Vec<(Pos, wordcraft_geom::Rect, Arc<Freeform>, f32)> {
    let Some(pg) = layout.pages.get(page) else { return Vec::new() };
    pg.items
        .iter()
        .filter_map(|it| match it {
            Placed::Object { rect, story, path, off, .. } => {
                let pos = Pos { story: *story, path: path.clone(), off: *off };
                match s.doc.para_at(&pos).and_then(|p| p.object_at(*off)) {
                    Some(InlineObject::Shape { freeform: Some(f), stroke_width, .. }) if f.is_ink() => Some((pos, *rect, f.clone(), *stroke_width)),
                    _ => None,
                }
            }
            _ => None,
        })
        .collect()
}

/// The ink stroke on `page` under (`x`, `y`), topmost first; none while ink is hidden.
pub fn ink_at(s: &mut Session, page: usize, x: f32, y: f32) -> Option<Pos> {
    if s.view.hide_ink || !x.is_finite() || !y.is_finite() {
        return None;
    }
    let layout = s.layout();
    ink_on(s, &layout, page).into_iter().rev().find_map(|(pos, r, f, sw)| {
        let reach = (if sw.is_finite() { sw.clamp(0.0, MAX_WIDTH) } else { 1.0 }) / 2.0 + ERASE_SLOP;
        f.distance(r.x, r.y, r.w, r.h, x, y).filter(|d| *d <= reach).map(|_| pos)
    })
}

fn page_param(s: &mut Session, v: &Value) -> Result<(usize, Arc<DocLayout>), CmdError> {
    let page = p::u64(v, "page").ok_or_else(|| CmdError::Params("`page` (0-based) is required".into()))?;
    let layout = s.layout();
    let page = usize::try_from(page).ok().filter(|i| *i < layout.pages.len()).ok_or_else(|| CmdError::Params(format!("no page {page}")))?;
    Ok((page, layout))
}

/// Delete the ink stroke under a point.
fn erase(s: &mut Session, v: &Value) -> CmdResult {
    let (page, _) = page_param(s, v)?;
    let (x, y) = (p::req_f32(v, "x")?, p::req_f32(v, "y")?);
    let pos = ink_at(s, page, x, y).ok_or_else(|| CmdError::Failed("no ink stroke there".into()))?;
    let len = OBJ.len_utf8();
    s.doc.delete_range(&pos, &Pos { off: pos.off + len, ..pos.clone() })?;
    for at in [&mut s.sel.anchor, &mut s.sel.focus] {
        if at.story == pos.story && at.path == pos.path && at.off > pos.off {
            at.off = at.off.saturating_sub(len).max(pos.off);
        }
    }
    Ok(json!({"erased": pos_json(&pos)}))
}

/// Draw an ink stroke through page points.
fn stroke(s: &mut Session, v: &Value) -> CmdResult {
    let (page, layout) = page_param(s, v)?;
    let tool = match p::str(v, "tool") {
        Some(t) => InkTool::parse(t).ok_or_else(|| CmdError::Params("`tool` is pen, pencil or highlighter".into()))?,
        None => s.view.draw.mode.pen().unwrap_or(InkTool::Pen),
    };
    let set = s.view.draw.settings(tool);
    let color = match p::str(v, "color") {
        Some(c) => Rgb::parse(c).ok_or_else(|| CmdError::Params("`color` is RRGGBB".into()))?,
        None => set.color,
    };
    let width = p::f32(v, "width").unwrap_or(set.width).clamp(MIN_WIDTH, MAX_WIDTH);
    let pg = layout.pages.get(page).ok_or_else(|| CmdError::Params(format!("no page {page}")))?;
    let (pw, ph) = (pg.w.max(1.0), pg.h.clamp(1.0, 1e5));
    // Points on the page; NaN, infinities and malformed entries are dropped.
    let pts: Vec<[f32; 2]> = v
        .get("points")
        .and_then(Value::as_array)
        .ok_or_else(|| CmdError::Params("`points` ([[x, y], …]) is required".into()))?
        .iter()
        .take(MAX_POINTS)
        .filter_map(|pt| {
            let a = pt.as_array()?;
            let (x, y) = (a.first()?.as_f64()? as f32, a.get(1)?.as_f64()? as f32);
            (x.is_finite() && y.is_finite()).then(|| [x.clamp(0.0, pw), y.clamp(0.0, ph)])
        })
        .collect();
    let first = *pts.first().ok_or_else(|| CmdError::Params("`points` has no usable point".into()))?;
    if ink_on(s, &layout, page).len() >= MAX_INK_PER_PAGE {
        return Err(CmdError::Failed(format!("page {page} already has {MAX_INK_PER_PAGE} ink strokes")));
    }
    // The frame: the points' bounds, grown by half the width so the stroke's ends fit.
    let pad = width / 2.0;
    let (mut x0, mut y0, mut x1, mut y1) = (first[0], first[1], first[0], first[1]);
    for [x, y] in &pts {
        (x0, y0, x1, y1) = (x0.min(*x), y0.min(*y), x1.max(*x), y1.max(*y));
    }
    let (fx, fy) = (x0 - pad, y0 - pad);
    let (fw, fh) = ((x1 - x0 + width).max(1.0), (y1 - y0 + width).max(1.0));
    let local: Vec<[f32; 2]> = pts.iter().map(|[x, y]| [x - fx, y - fy]).collect();
    let wrap = if tool == InkTool::Highlighter { Wrap::BehindText } else { Wrap::InFrontOfText };
    let obj = InlineObject::Shape {
        kind: ShapeKind::Freeform,
        w: fw,
        h: fh,
        fill: None,
        stroke: Some(color),
        stroke_width: width,
        float: Float { wrap, ..Default::default() },
        story: None,
        freeform: Some(Arc::new(Freeform::ink(tool, fw, fh, local))),
    };
    let at = place(s, &layout, obj, page, (fx, fy), first[1])?;
    Ok(json!({"pos": pos_json(&at), "page": page, "rect": [fx, fy, fw, fh]}))
}

/// Insert floating `obj` with its top-left at `(x, y)` on `page`, anchored to the body paragraph
/// under `anchor_y` (positioned from its column and top), or, where no paragraph starts on the
/// page, to the nearest line and positioned on the page. Returns where its character went.
fn place(s: &mut Session, layout: &DocLayout, obj: InlineObject, page: usize, (x, y): (f32, f32), anchor_y: f32) -> Result<Pos, CmdError> {
    let (at, on_page) = match super::objects::anchor_paragraph(layout, page, anchor_y) {
        Some(path) => (Pos { story: StoryRef::Body, path, off: 0 }, false),
        None => {
            (super::objects::nearest_line(layout, page, anchor_y).ok_or_else(|| CmdError::Params("no text on that page to anchor to".into()))?, true)
        }
    };
    s.doc.insert_object(&at, obj, &CharProps::default())?;
    let len = OBJ.len_utf8();
    for p in [&mut s.sel.anchor, &mut s.sel.focus] {
        if p.story == at.story && p.path == at.path && p.off > at.off {
            p.off += len;
        }
    }
    let (rel, origin) = if on_page {
        (Anchor::Page, wordcraft_geom::Point::new(0.0, 0.0))
    } else {
        s.touch(); // lay out the new object to find where its paragraph puts it
        let origin = s.layout().object(&at, page).ok_or_else(|| CmdError::Failed("the stroke isn't laid out".into()))?.origin;
        (Anchor::Paragraph, origin)
    };
    let para = s.doc.para_mut(at.story, &at.path)?;
    if let Some(InlineObject::Shape { float, .. }) = para.object_at_mut(at.off) {
        float.h_rel = if on_page { Anchor::Page } else { Anchor::Column };
        float.v_rel = rel;
        float.x = x - origin.x;
        float.y = y - origin.y;
    }
    para.touch();
    Ok(at)
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use wordcraft_doc::StoryRef;
    use wordcraft_doc::freeform::InkTool;
    use wordcraft_doc::para::{InlineObject, Wrap};

    use crate::Session;

    fn session() -> Session {
        let mut s = Session::new(wordcraft_doc::Document::new());
        s.run("document.setText", &json!({"text": "First paragraph of text.\nSecond paragraph of text."})).unwrap();
        s
    }

    fn ink(s: &Session) -> Vec<InlineObject> {
        s.doc.body.iter().filter_map(|b| b.as_para()).flat_map(|p| p.objects.iter()).filter(|o| o.ink().is_some()).cloned().collect()
    }

    #[test]
    fn a_stroke_inserts_floating_ink_through_its_points_in_one_undo_step() {
        let mut s = session();
        let text = s.doc.plain_text(StoryRef::Body);
        let r = s
            .run("draw.stroke", &json!({"page": 0, "points": [[100, 90], [150, 120], [f64::NAN, 3], [200, 95]], "color": "C00000", "width": 2}))
            .unwrap();
        let [x, y] = [r["rect"][0].as_f64().unwrap() as f32, r["rect"][1].as_f64().unwrap() as f32];
        assert_eq!((x, y), (99.0, 89.0), "the points' bounds, grown by half the width");
        let objs = ink(&s);
        let [InlineObject::Shape { stroke, stroke_width, float, freeform: Some(f), .. }] = objs.as_slice() else { panic!("one stroke: {objs:?}") };
        assert_eq!((*stroke, *stroke_width, float.wrap), (Some(wordcraft_doc::Rgb(0xC0, 0, 0)), 2.0, Wrap::InFrontOfText));
        assert_eq!(f.ink, Some(InkTool::Pen));
        assert_eq!(f.paths[0].pts, vec![[1.0, 1.0], [51.0, 31.0], [101.0, 6.0]], "NaN dropped, shape-local");
        // Laid out where it was drawn.
        let at = wordcraft_doc::Pos { story: StoryRef::Body, path: wordcraft_doc::Path::top(0), off: 0 };
        let hit = s.layout().object(&at, 0).unwrap();
        assert!((hit.rect.x - 99.0).abs() < 0.01 && (hit.rect.y - 89.0).abs() < 0.01, "{:?}", hit.rect);
        s.run("edit.undo", &json!({})).unwrap();
        assert!(ink(&s).is_empty());
        assert_eq!(s.doc.plain_text(StoryRef::Body), text);
        // A highlighter goes behind the text, translucent.
        s.run("draw.highlighter", &json!({"color": "00FF00"})).unwrap();
        s.run("draw.stroke", &json!({"page": 0, "points": [[80, 80]]})).unwrap();
        let objs = ink(&s);
        let [InlineObject::Shape { float, freeform: Some(f), stroke, .. }] = objs.as_slice() else { panic!() };
        assert_eq!((float.wrap, f.ink, *stroke), (Wrap::BehindText, Some(InkTool::Highlighter), Some(wordcraft_doc::Rgb(0, 0xFF, 0))));
        assert!(f.alpha < 1.0);
        // Hostile input is refused, not drawn.
        assert!(s.run("draw.stroke", &json!({"page": 99, "points": [[1, 1]]})).is_err());
        assert!(s.run("draw.stroke", &json!({"page": 0, "points": [["x", 1], [1]]})).is_err());
        assert!(s.run("draw.stroke", &json!({"page": 0, "points": [[1, 1]], "tool": "crayon"})).is_err());
    }

    #[test]
    fn the_eraser_removes_the_stroke_under_the_point_and_hide_ink_hides_it() {
        let mut s = session();
        s.run("draw.stroke", &json!({"page": 0, "points": [[100, 100], [200, 100]], "width": 2})).unwrap();
        assert!(s.run("draw.eraser", &json!({"page": 0, "x": 150, "y": 130})).is_err(), "nothing there");
        assert_eq!(ink(&s).len(), 1);
        // Review › Hide Ink: not drawn, not erasable, still in the document.
        assert_eq!(s.run("review.hideInk", &json!({})).unwrap()["value"], true);
        assert!(s.view.hide_ink);
        assert!(s.run("draw.eraser", &json!({"page": 0, "x": 150, "y": 101})).is_err());
        s.run("review.hideInk", &json!({"value": false})).unwrap();
        s.run("draw.eraser", &json!({"page": 0, "x": 150, "y": 101})).unwrap();
        assert!(ink(&s).is_empty());
        // Without a point it picks up the eraser; Select puts it down.
        s.run("draw.eraser", &json!({})).unwrap();
        assert_eq!(s.view.draw.mode, super::DrawMode::Eraser);
        s.run("draw.pencil", &json!({"width": 1e9})).unwrap();
        assert_eq!((s.view.draw.mode, s.view.draw.pencil.width), (super::DrawMode::Pen(InkTool::Pencil), super::MAX_WIDTH));
        s.run("draw.select", &json!({})).unwrap();
        assert_eq!(s.view.draw.mode, super::DrawMode::Select);
    }
}
