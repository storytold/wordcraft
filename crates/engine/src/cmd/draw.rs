//! Draw tab and Review › Ink: pens, the eraser, the lasso and ink strokes.
//!
//! An ink stroke is a floating freeform shape ([`Freeform`] with its pen set) anchored to the
//! paragraph under where it starts, in front of the text (a highlighter's behind it). The pen
//! commands only choose what dragging on the page does (session view state); `draw.stroke` and
//! `draw.erase` change the document.
//!
//! Draw › Lasso Select picks the strokes (and drawings) inside a loop drawn on a page; the
//! selection can then be moved, recoloured, deleted or turned into shapes. Ink to Shape replaces
//! strokes that look like a line, arrow, rectangle, ellipse, triangle or polygon with that shape
//! ([`ink_shape`](super::ink_shape)). Ink Replay plays the strokes back in the order they were
//! drawn (each stroke keeps its place in that order). Add Pen keeps custom pens in a gallery.

use std::sync::Arc;

use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Value, json};
use wordcraft_doc::freeform::{FreePath, Freeform, InkTool, MAX_INK_PER_PAGE, MAX_POINTS};
use wordcraft_doc::para::{Anchor, Float, InlineObject, OBJ, ShapeKind, Wrap};
use wordcraft_doc::props::Rgb;
use wordcraft_doc::{CharProps, Pos, StoryRef};
use wordcraft_geom::Rect;
use wordcraft_layout::{DocLayout, Placed};

use super::ink_shape::{ArrowParts, InkShape, arrow_parts, pair_parts, recognise};
use super::pos_json;
use crate::{CmdError, CmdResult, CommandSpec, Session, p};

/// What dragging on the page does (Draw tab).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DrawMode {
    /// Ordinary selection of text and drawings.
    #[default]
    Select,
    /// Dragging draws a loop that selects the ink inside it; dragging the selection moves it.
    Lasso,
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

/// A pen added to the pen gallery (Draw › Add Pen): its kind, colour and thickness. Saved as
/// `{"kind": "pen", "color": "RRGGBB", "width": 1.5}`.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "Value", into = "Value")]
pub struct CustomPen {
    pub kind: InkTool,
    pub color: Rgb,
    /// Points.
    pub width: f32,
}

impl TryFrom<Value> for CustomPen {
    type Error = CmdError;
    fn try_from(v: Value) -> Result<Self, CmdError> {
        custom_pen(&v)
    }
}

impl From<CustomPen> for Value {
    fn from(p: CustomPen) -> Value {
        json!({"kind": p.kind.name(), "color": p.color.hex(), "width": p.width})
    }
}

/// Most pens the gallery keeps.
pub const MAX_PENS: usize = 24;

/// A pen from `{"kind"?, "color"?, "width"?}` (the kind's defaults for what's left out; the
/// width clamped). Anything but an object, an unknown kind or a malformed colour is refused.
pub fn custom_pen(v: &Value) -> Result<CustomPen, CmdError> {
    if !v.is_object() {
        return Err(CmdError::Params("a pen is {\"kind\", \"color\", \"width\"}".into()));
    }
    let kind = match p::str(v, "kind") {
        Some(k) => InkTool::parse(k).ok_or_else(|| CmdError::Params("`kind` is pen, pencil or highlighter".into()))?,
        None => InkTool::Pen,
    };
    let color = match p::str(v, "color") {
        Some(c) => Rgb::parse(c).ok_or_else(|| CmdError::Params("`color` is RRGGBB".into()))?,
        None => kind.default_color(),
    };
    let width = p::f32(v, "width").filter(|w| w.is_finite()).unwrap_or(kind.default_width()).clamp(MIN_WIDTH, MAX_WIDTH);
    Ok(CustomPen { kind, color, width })
}

/// Read a saved pen gallery, dropping entries that aren't pens (a hand-edited or damaged
/// settings file loses those pens, never the rest of the settings).
pub fn lenient_pens<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<CustomPen>, D::Error> {
    let v = Value::deserialize(d)?;
    Ok(v.as_array().map(|a| a.iter().filter_map(|x| custom_pen(x).ok()).take(MAX_PENS).collect()).unwrap_or_default())
}

/// Strokes and drawings picked with the lasso on `page`. Good while the document is as it was
/// when they were picked (or last moved, recoloured or converted through the lasso).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LassoSelection {
    pub page: usize,
    pub items: Vec<Pos>,
    rev: u64,
}

/// The Draw tab's state: the tool in use, each pen's settings, the pen gallery, Ink to Shape,
/// the lasso selection and whether ink is being replayed.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct DrawState {
    pub mode: DrawMode,
    pub pen: PenSettings,
    pub pencil: PenSettings,
    pub highlighter: PenSettings,
    /// Pens added with Add Pen (kept between runs by the app).
    #[serde(deserialize_with = "lenient_pens")]
    pub pens: Vec<CustomPen>,
    /// Ink to Shape is on: strokes drawn from now on that look like a shape become that shape.
    pub ink_to_shape: bool,
    #[serde(skip)]
    pub lasso: Option<LassoSelection>,
    /// Ink Replay is showing (the app plays the strokes back and turns this off when done).
    #[serde(skip)]
    pub replay: bool,
}

impl Default for DrawState {
    fn default() -> Self {
        DrawState {
            mode: DrawMode::Select,
            pen: PenSettings::of(InkTool::Pen),
            pencil: PenSettings::of(InkTool::Pencil),
            highlighter: PenSettings::of(InkTool::Highlighter),
            pens: Vec::new(),
            ink_to_shape: false,
            lasso: None,
            replay: false,
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
/// Most points of a lasso loop that are kept (a longer loop is thinned out).
const MAX_LASSO_POINTS: usize = 512;
/// Points of a stroke tested against the lasso (spread along it).
const LASSO_SAMPLES: usize = 64;
/// Farthest the lasso moves its selection in one step, points.
const MAX_MOVE: f32 = 10_000.0;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("draw.select", "Select", "Draw › Tools", |s, _| {
            s.view.draw.mode = DrawMode::Select;
            state(s)
        })
        .pure(),
        CommandSpec::new("draw.lasso", "Lasso Select", "Draw › Tools", lasso)
            .params(r#"{"page"?: n, "points"?: [[x, y], …], "clear"?: bool}  (no points: use the lasso; points: select the ink inside that loop; clear: drop the selection)"#)
            .pure(),
        CommandSpec::new("draw.lassoMove", "Move Selected Ink", "Draw › Tools", lasso_move).params(r#"{"dx": pt, "dy": pt}"#),
        CommandSpec::new("draw.lassoDelete", "Delete Selected Ink", "Draw › Tools", lasso_delete),
        CommandSpec::new("draw.lassoColor", "Recolor Selected Ink", "Draw › Tools", lasso_color).params(r#"{"color": "RRGGBB"}"#),
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
        CommandSpec::new("draw.addPen", "Add Pen", "Draw › Pens", add_pen)
            .params(r#"{"kind"?: "pen|pencil|highlighter", "color"?: "RRGGBB", "width"?: pt}  (adds the pen to the gallery and picks it up)"#)
            .pure(),
        CommandSpec::new("draw.removePen", "Remove Pen", "Draw › Pens", |s, v| {
            let i = p::u64(v, "index").and_then(|i| usize::try_from(i).ok()).ok_or_else(|| CmdError::Params("`index` (0-based) is required".into()))?;
            if i >= s.view.draw.pens.len() {
                return Err(CmdError::Params(format!("no pen {i}")));
            }
            let pen = s.view.draw.pens.remove(i);
            Ok(json!({"removed": Value::from(pen), "pens": s.view.draw.pens.len()}))
        })
        .params(r#"{"index": n}"#)
        .pure(),
        CommandSpec::new("draw.stroke", "Draw Ink", "Draw › Pens", stroke).params(
            r#"{"page": n, "points": [[x, y], …] (page points, at most 5000), "tool"?: "pen|pencil|highlighter", "color"?: "RRGGBB", "width"?: pt}"#,
        ),
        CommandSpec::new("draw.erase", "Erase Ink", "Draw › Tools", erase).params(r#"{"page": n, "x": pt, "y": pt}"#),
        CommandSpec::new("draw.inkToShape", "Ink to Shape", "Draw › Convert", |s, v| {
            if let Some(on) = p::bool(v, "value") {
                s.view.draw.ink_to_shape = on;
                return Ok(json!({"value": on}));
            }
            // With strokes lasso-selected it converts them; otherwise it turns the mode on or off.
            let ink_selected = lasso_selection(s).is_some_and(|l| l.items.iter().any(|pos| s.doc.para_at(pos).and_then(|p| p.object_at(pos.off)).and_then(InlineObject::ink).is_some()));
            if ink_selected {
                return s.run("draw.convertInk", &json!({}));
            }
            s.view.draw.ink_to_shape = !s.view.draw.ink_to_shape;
            Ok(json!({"value": s.view.draw.ink_to_shape}))
        })
        .params(r#"{"value"?: bool}  (no value: convert the lasso-selected strokes, or turn Ink to Shape on or off)"#)
        .pure(),
        CommandSpec::new("draw.convertInk", "Convert to Shapes", "Draw › Convert", convert_selection),
        CommandSpec::new("draw.replay", "Ink Replay", "Draw › Replay", replay)
            .params(r#"{"value"?: bool}  (starts or stops Ink Replay; returns the strokes in the order they were drawn)"#)
            .pure(),
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
    if let Some(w) = p::f32(v, "width").filter(|w| w.is_finite()) {
        set.width = w.clamp(MIN_WIDTH, MAX_WIDTH);
    }
    s.view.draw.mode = DrawMode::Pen(tool);
    state(s)
}

/// Add a pen to the gallery and pick it up.
fn add_pen(s: &mut Session, v: &Value) -> CmdResult {
    let pen = custom_pen(v)?;
    if s.view.draw.pens.len() >= MAX_PENS {
        return Err(CmdError::Failed(format!("the pen gallery holds at most {MAX_PENS} pens")));
    }
    s.view.draw.pens.push(pen);
    *s.view.draw.settings_mut(pen.kind) = PenSettings { color: pen.color, width: pen.width };
    s.view.draw.mode = DrawMode::Pen(pen.kind);
    Ok(json!({"pen": Value::from(pen), "index": s.view.draw.pens.len() - 1, "pens": s.view.draw.pens.len()}))
}

/// An ink stroke laid out on a page.
#[derive(Clone, Debug)]
pub struct PlacedInk {
    pub pos: Pos,
    /// Its unturned frame on the page.
    pub rect: Rect,
    /// Its turn about the frame's centre.
    pub spin: wordcraft_geom::Spin,
    pub freeform: Arc<Freeform>,
    pub color: Option<Rgb>,
    pub width: f32,
}

impl PlacedInk {
    /// Its paths in page points, turned as it is drawn.
    pub fn paths(&self) -> Vec<Vec<[f32; 2]>> {
        let r = self.rect;
        let paths = self.freeform.placed(r.x, r.y, r.w, r.h).into_iter().map(|(pts, _)| pts);
        if self.spin.is_identity() {
            return paths.collect();
        }
        let (cx, cy) = (r.x + r.w / 2.0, r.y + r.h / 2.0);
        paths.map(|pts| pts.into_iter().map(|[x, y]| <[f32; 2]>::from(self.spin.apply(cx, cy, x, y))).collect()).collect()
    }
}

/// Ink strokes laid out on `page`, in the page's drawing order: their position, unturned frame,
/// turn and geometry on the page.
pub fn ink_on(s: &Session, layout: &DocLayout, page: usize) -> Vec<PlacedInk> {
    let Some(pg) = layout.pages.get(page) else { return Vec::new() };
    pg.items
        .iter()
        .filter_map(|it| match it {
            Placed::Object { rect, spin, story, path, off, .. } => {
                let pos = Pos { story: *story, path: path.clone(), off: *off };
                match s.doc.para_at(&pos).and_then(|p| p.object_at(*off)) {
                    Some(InlineObject::Shape { freeform: Some(f), stroke_width, stroke, .. }) if f.is_ink() => {
                        Some(PlacedInk { pos, rect: *rect, spin: *spin, freeform: f.clone(), color: *stroke, width: *stroke_width })
                    }
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
    ink_on(s, &layout, page).into_iter().rev().find_map(|k| {
        let reach = (if k.width.is_finite() { k.width.clamp(0.0, MAX_WIDTH) } else { 1.0 }) / 2.0 + ERASE_SLOP;
        // A turned stroke is drawn turned about its frame's centre: test the point turned back
        // into the stroke's own frame (a turn keeps distances, so the reach holds).
        let r = k.rect;
        let (u, v) = k.spin.unapply(r.x + r.w / 2.0, r.y + r.h / 2.0, x, y);
        k.freeform.distance(r.x, r.y, r.w, r.h, u, v).filter(|d| *d <= reach).map(|_| k.pos)
    })
}

fn page_param(s: &mut Session, v: &Value) -> Result<(usize, Arc<DocLayout>), CmdError> {
    let page = p::u64(v, "page").ok_or_else(|| CmdError::Params("`page` (0-based) is required".into()))?;
    let layout = s.layout();
    let page = usize::try_from(page).ok().filter(|i| *i < layout.pages.len()).ok_or_else(|| CmdError::Params(format!("no page {page}")))?;
    Ok((page, layout))
}

/// `points` as page points: at most `max`, NaN, infinities and malformed entries dropped, each
/// clamped to the `pw` × `ph` page.
fn points_param(v: &Value, max: usize, pw: f32, ph: f32) -> Result<Vec<[f32; 2]>, CmdError> {
    Ok(v.get("points")
        .and_then(Value::as_array)
        .ok_or_else(|| CmdError::Params("`points` ([[x, y], …]) is required".into()))?
        .iter()
        .take(max)
        .filter_map(|pt| {
            let a = pt.as_array()?;
            let (x, y) = (a.first()?.as_f64()? as f32, a.get(1)?.as_f64()? as f32);
            (x.is_finite() && y.is_finite()).then(|| [x.clamp(0.0, pw), y.clamp(0.0, ph)])
        })
        .collect())
}

/// Delete the ink stroke under a point.
fn erase(s: &mut Session, v: &Value) -> CmdResult {
    let (page, _) = page_param(s, v)?;
    let (x, y) = (p::req_f32(v, "x")?, p::req_f32(v, "y")?);
    let pos = ink_at(s, page, x, y).ok_or_else(|| CmdError::Failed("no ink stroke there".into()))?;
    delete_object(s, &pos)?;
    Ok(json!({"erased": pos_json(&pos)}))
}

/// Delete the object at `pos`, keeping the text selection on the same text.
fn delete_object(s: &mut Session, pos: &Pos) -> Result<(), CmdError> {
    let len = OBJ.len_utf8();
    s.doc.delete_range(pos, &Pos { off: pos.off + len, ..pos.clone() })?;
    for at in [&mut s.sel.anchor, &mut s.sel.focus] {
        if at.story == pos.story && at.path == pos.path && at.off > pos.off {
            at.off = at.off.saturating_sub(len).max(pos.off);
        }
    }
    Ok(())
}

/// The next stroke's place in the drawing order: one after the latest drawn.
fn next_order(s: &Session) -> u32 {
    let latest = s
        .doc
        .para_paths(StoryRef::Body)
        .iter()
        .filter_map(|path| s.doc.para(StoryRef::Body, path))
        .flat_map(|p| p.objects.iter())
        .filter_map(|o| o.ink().and_then(|f| f.order))
        .max()
        .unwrap_or(0);
    latest.saturating_add(1)
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
    let width = p::f32(v, "width").filter(|w| w.is_finite()).unwrap_or(set.width).clamp(MIN_WIDTH, MAX_WIDTH);
    let pg = layout.pages.get(page).ok_or_else(|| CmdError::Params(format!("no page {page}")))?;
    let (pw, ph) = (pg.w.max(1.0), pg.h.clamp(1.0, 1e5));
    let pts = points_param(v, MAX_POINTS, pw, ph)?;
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
    let order = next_order(s);
    let obj = InlineObject::Shape {
        kind: ShapeKind::Freeform,
        w: fw,
        h: fh,
        fill: None,
        stroke: Some(color),
        stroke_width: width,
        float: Float { wrap, ..Default::default() },
        story: None,
        freeform: Some(Arc::new(Freeform { order: Some(order), ..Freeform::ink(tool, fw, fh, local) })),
        effects: Default::default(),
        extra: Default::default(),
    };
    let at = place(s, &layout, obj, page, (fx, fy), (first[0], first[1]))?;
    // Ink to Shape: a stroke that looks like a shape becomes it, and a line and a head drawn as two
    // strokes become one arrow (in the same undo step).
    let (at, shape, rect) = if s.view.draw.ink_to_shape && tool != InkTool::Highlighter {
        match pair_with_neighbour(s, page, &at)? {
            Some(arrow) => {
                // The arrow is the line drawn earlier (or just now): its frame, not the stroke's.
                s.touch();
                let r = s.layout().object(&arrow, page).map(|o| o.rect);
                let rect = r.map_or([fx, fy, fw, fh], |r| [r.x, r.y, r.w, r.h]);
                (arrow, Some("arrow"), rect)
            }
            None => {
                let shape = convert_at(s, &at)?;
                (at, shape, [fx, fy, fw, fh])
            }
        }
    } else {
        (at, None, [fx, fy, fw, fh])
    };
    Ok(json!({"pos": pos_json(&at), "page": page, "rect": rect, "shape": shape}))
}

/// Insert floating `obj` with its top-left at `(x, y)` on `page`, anchored to the body paragraph
/// under `anchor` (positioned from its column and top), or, where no paragraph starts on the
/// page, to the nearest line and positioned on the page. Returns where its character went.
fn place(s: &mut Session, layout: &DocLayout, obj: InlineObject, page: usize, (x, y): (f32, f32), anchor: (f32, f32)) -> Result<Pos, CmdError> {
    let (at, on_page) = match super::objects::anchor_paragraph(&s.doc, layout, page, anchor, StoryRef::Body) {
        Some(path) => (Pos { story: StoryRef::Body, path, off: 0 }, false),
        None => (
            super::objects::nearest_line(layout, page, anchor.1, StoryRef::Body)
                .ok_or_else(|| CmdError::Params("no text on that page to anchor to".into()))?,
            true,
        ),
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

// --- Lasso -------------------------------------------------------------------------------

/// The current lasso selection, if the document hasn't changed under it.
pub fn lasso_selection(s: &Session) -> Option<&LassoSelection> {
    s.view.draw.lasso.as_ref().filter(|l| l.rev == s.rev() && !l.items.is_empty())
}

/// Keep `sel` selected after the mutating command running now (which bumps the revision once).
fn keep_lasso(s: &mut Session, sel: LassoSelection) {
    s.view.draw.lasso = Some(LassoSelection { rev: s.rev().wrapping_add(1), ..sel });
}

/// After a command that changed nothing, which puts the revision back from `touched` to `rev`: a
/// lasso selection the command kept for the revision it would have reached stays selected.
pub(crate) fn rekey_lasso(s: &mut Session, touched: u64, rev: u64) {
    if let Some(l) = s.view.draw.lasso.as_mut()
        && (l.rev == touched || l.rev == touched.wrapping_add(1))
    {
        l.rev = rev;
    }
}

/// Whether (`x`, `y`) is inside the closed loop `poly` (even–odd rule).
fn inside(poly: &[[f32; 2]], x: f32, y: f32) -> bool {
    let n = poly.len();
    let mut odd = false;
    for i in 0..n {
        let (Some(a), Some(b)) = (poly.get(i), poly.get((i + n - 1) % n)) else { continue };
        if (a[1] > y) != (b[1] > y) && x < (b[0] - a[0]) * (y - a[1]) / (b[1] - a[1]) + a[0] {
            odd = !odd;
        }
    }
    odd
}

/// Select the ink inside a loop (most of a stroke's points inside it) and the drawings whose
/// centre is inside it; without a loop, pick up the lasso.
fn lasso(s: &mut Session, v: &Value) -> CmdResult {
    if p::bool(v, "clear") == Some(true) {
        s.view.draw.lasso = None;
        return Ok(json!({"selected": [], "count": 0}));
    }
    if v.get("points").is_none() {
        s.view.draw.mode = DrawMode::Lasso;
        return state(s);
    }
    let (page, layout) = page_param(s, v)?;
    let pg = layout.pages.get(page).ok_or_else(|| CmdError::Params(format!("no page {page}")))?;
    let mut poly = points_param(v, MAX_POINTS, pg.w.max(1.0), pg.h.clamp(1.0, 1e5))?;
    if poly.len() > MAX_LASSO_POINTS {
        let step = poly.len().div_ceil(MAX_LASSO_POINTS);
        poly = poly.into_iter().step_by(step).collect();
    }
    if poly.len() < 3 {
        return Err(CmdError::Params("a lasso loop needs at least three points".into()));
    }
    let mut items = Vec::new();
    for it in &pg.items {
        let Placed::Object { rect, spin, story, path, off, .. } = it else { continue };
        let pos = Pos { story: *story, path: path.clone(), off: *off };
        let Some(obj) = s.doc.para_at(&pos).and_then(|p| p.object_at(*off)) else { continue };
        let picked = match obj.ink() {
            Some(_) if s.view.hide_ink => false,
            Some(f) => {
                // Where the stroke is drawn: turned about its frame's centre.
                let (cx, cy) = (rect.x + rect.w / 2.0, rect.y + rect.h / 2.0);
                let pts: Vec<[f32; 2]> = f
                    .placed(rect.x, rect.y, rect.w, rect.h)
                    .into_iter()
                    .flat_map(|(p, _)| p)
                    .map(|[x, y]| <[f32; 2]>::from(spin.apply(cx, cy, x, y)))
                    .collect();
                let step = pts.len().div_ceil(LASSO_SAMPLES).max(1);
                let (n, hits) = pts.iter().step_by(step).fold((0usize, 0usize), |(n, h), [x, y]| (n + 1, h + usize::from(inside(&poly, *x, *y))));
                n > 0 && hits * 10 >= n * 6
            }
            None => obj.frame().is_some() && inside(&poly, rect.x + rect.w / 2.0, rect.y + rect.h / 2.0),
        };
        if picked && !items.contains(&pos) {
            items.push(pos);
        }
    }
    let selected: Vec<Value> = items.iter().map(pos_json).collect();
    s.view.draw.lasso = (!items.is_empty()).then(|| LassoSelection { page, items, rev: s.rev() });
    Ok(json!({"page": page, "count": selected.len(), "selected": selected}))
}

fn lasso_required(s: &Session) -> Result<LassoSelection, CmdError> {
    lasso_selection(s).cloned().ok_or_else(|| CmdError::Failed("nothing is selected with the lasso".into()))
}

/// The lasso selection's bounds on its page in `layout` (the document's current layout), for
/// the app to draw and drag.
pub fn lasso_bounds(s: &Session, layout: &DocLayout) -> Option<(usize, Rect)> {
    let sel = lasso_selection(s)?;
    let mut r: Option<Rect> = None;
    for pos in &sel.items {
        if let Some(hit) = layout.object(pos, sel.page) {
            r = Some(r.map_or(hit.rect, |u| u.union(&hit.rect)));
        }
    }
    r.map(|r| (sel.page, r))
}

/// Move the lasso-selected drawings by (`dx`, `dy`) points (one undo step).
fn lasso_move(s: &mut Session, v: &Value) -> CmdResult {
    let sel = lasso_required(s)?;
    let d = |k: &str| p::req_f32(v, k).map(|d| if d.is_finite() { d.clamp(-MAX_MOVE, MAX_MOVE) } else { 0.0 });
    let (dx, dy) = (d("dx")?, d("dy")?);
    let mut moved = 0;
    for pos in &sel.items {
        let para = s.doc.para_mut(pos.story, &pos.path)?;
        let mut hit = false;
        if let Some(o) = para.object_at_mut(pos.off)
            && o.is_floating()
            && let Some(f) = o.float_mut()
        {
            if f.h_align.is_none() {
                f.x += dx;
            }
            if f.v_align.is_none() {
                f.y += dy;
            }
            hit = true;
        }
        if hit {
            para.touch();
            moved += 1;
        }
    }
    if moved == 0 {
        return Err(CmdError::Failed("nothing selected can be moved".into()));
    }
    keep_lasso(s, sel);
    Ok(json!({"moved": moved}))
}

/// Delete the lasso-selected strokes and drawings (one undo step).
fn lasso_delete(s: &mut Session, _: &Value) -> CmdResult {
    let mut items = lasso_required(s)?.items;
    // Last first, so the earlier ones keep their place.
    items.sort();
    items.dedup();
    let mut deleted = 0;
    for pos in items.iter().rev() {
        if s.doc.para_at(pos).and_then(|p| p.object_at(pos.off)).is_some() {
            delete_object(s, pos)?;
            deleted += 1;
        }
    }
    s.view.draw.lasso = None;
    if deleted == 0 {
        return Err(CmdError::Failed("nothing selected to delete".into()));
    }
    Ok(json!({"deleted": deleted}))
}

/// Give the lasso-selected strokes and shapes this outline colour (one undo step).
fn lasso_color(s: &mut Session, v: &Value) -> CmdResult {
    let sel = lasso_required(s)?;
    let color = Rgb::parse(p::req_str(v, "color")?).ok_or_else(|| CmdError::Params("`color` is RRGGBB".into()))?;
    let mut changed = 0;
    for pos in &sel.items {
        let para = s.doc.para_mut(pos.story, &pos.path)?;
        let mut hit = false;
        if let Some(InlineObject::Shape { stroke, .. }) = para.object_at_mut(pos.off) {
            *stroke = Some(color);
            hit = true;
        }
        if hit {
            para.touch();
            changed += 1;
        }
    }
    if changed == 0 {
        return Err(CmdError::Failed("nothing selected has an outline to recolour".into()));
    }
    keep_lasso(s, sel);
    Ok(json!({"recolored": changed, "color": color.hex()}))
}

// --- Ink to Shape --------------------------------------------------------------------------

/// Turn the lasso-selected strokes that look like shapes into those shapes, and a selected line
/// and arrowhead drawn as two strokes into one arrow (one undo step).
fn convert_selection(s: &mut Session, _: &Value) -> CmdResult {
    let sel = lasso_required(s)?;
    let layout = s.layout();
    let pieces = arrow_pieces(s, &layout, sel.page, |pos, _| sel.items.contains(pos));
    let (mut used, mut heads, mut converted): (Vec<Pos>, Vec<Pos>, Vec<(Pos, &'static str)>) = (Vec::new(), Vec::new(), Vec::new());
    // Each piece was recognised once; pairing them is cheap geometry, even for a crowded page.
    for line in pieces.iter().filter(|k| k.parts.line.is_some()) {
        for head in pieces.iter().filter(|k| !k.parts.heads.is_empty()) {
            if line.pos == head.pos || used.contains(&line.pos) || used.contains(&head.pos) || !near(line.rect, head.rect) {
                continue;
            }
            if let Some(arrow) = pair_parts(&line.parts, &head.parts) {
                set_shape(s, &line.pos, &local_to(&arrow, line.rect))?;
                used.extend([line.pos.clone(), head.pos.clone()]);
                heads.push(head.pos.clone());
                converted.push((line.pos.clone(), "arrow"));
            }
        }
    }
    for pos in sel.items.iter().filter(|p| !used.contains(p)) {
        if let Some(name) = convert_at(s, pos)? {
            converted.push((pos.clone(), name));
        }
    }
    if converted.is_empty() {
        return Err(CmdError::Failed("none of the selected strokes looks like a shape".into()));
    }
    // The heads went into their arrows; last first, so the earlier ones keep their place.
    heads.sort();
    for pos in heads.iter().rev() {
        delete_object(s, pos)?;
    }
    let items = sel.items.iter().filter(|p| !heads.contains(p)).map(|p| after_deleting(p, &heads)).collect();
    let converted: Vec<Value> = converted.iter().map(|(p, name)| json!({"pos": pos_json(&after_deleting(p, &heads)), "shape": name})).collect();
    keep_lasso(s, LassoSelection { items, ..sel });
    Ok(json!({"count": converted.len(), "converted": converted}))
}

/// Where `pos` is once the objects at `deleted` (none of them `pos`) are gone.
fn after_deleting(pos: &Pos, deleted: &[Pos]) -> Pos {
    let before = deleted.iter().filter(|d| d.story == pos.story && d.path == pos.path && d.off < pos.off).count();
    Pos { off: pos.off.saturating_sub(before.saturating_mul(OBJ.len_utf8())), ..pos.clone() }
}

/// A stroke (or an Ink to Shape line or triangle) on a page that may be one half of a two-stroke
/// arrow: where it is, when it was drawn (ink's drawing order) and what it can be in an arrow.
struct ArrowPiece {
    pos: Pos,
    rect: Rect,
    order: Option<u32>,
    parts: ArrowParts,
}

/// Whether two objects laid out at `a` and `b` are close enough to make an arrow (a cheap test
/// before recognising them).
fn near(a: Rect, b: Rect) -> bool {
    let reach = 0.2 * a.w.max(a.h).max(b.w).max(b.h);
    reach.is_finite() && a.expand(reach).intersects(&b.expand(reach))
}

/// The pieces of two-stroke arrows on `page` that `keep` (given the position and laid-out rect)
/// accepts: unturned, unflipped pen and pencil strokes, and the open two-point lines and closed
/// triangles Ink to Shape makes. `keep` runs first, so only kept pieces are recognised.
fn arrow_pieces(s: &Session, layout: &DocLayout, page: usize, keep: impl Fn(&Pos, &Rect) -> bool) -> Vec<ArrowPiece> {
    let Some(pg) = layout.pages.get(page) else { return Vec::new() };
    let mut out = Vec::new();
    for it in &pg.items {
        let Placed::Object { rect, spin, story, path, off, .. } = it else { continue };
        let pos = Pos { story: *story, path: path.clone(), off: *off };
        if !spin.is_identity() || !keep(&pos, rect) {
            continue;
        }
        let Some(InlineObject::Shape { kind, freeform: Some(f), float, .. }) = s.doc.para_at(&pos).and_then(|p| p.object_at(*off)) else { continue };
        let Some(fp) = f.paths.first().filter(|_| f.paths.len() == 1) else { continue };
        let pen = matches!(f.ink, Some(InkTool::Pen | InkTool::Pencil)) && !s.view.hide_ink;
        let shape = f.ink.is_none() && *kind == ShapeKind::Freeform && matches!((fp.pts.len(), fp.closed), (2, false) | (3, true));
        if !(pen || shape) || float.rot != 0.0 || float.flip_h || float.flip_v {
            continue;
        }
        let Some((mut pts, closed)) = f.placed(rect.x, rect.y, rect.w, rect.h).into_iter().next() else { continue };
        if closed && let Some(&first) = pts.first() {
            pts.push(first);
        }
        out.push(ArrowPiece { pos, rect: *rect, order: f.order, parts: arrow_parts(&pts) });
    }
    out
}

/// Ink to Shape while drawing: when the stroke just drawn at `new` and another on the page are a
/// line and an arrowhead (either drawn first), make the line an arrow and drop the head. Returns
/// where the arrow is.
fn pair_with_neighbour(s: &mut Session, page: usize, new: &Pos) -> Result<Option<Pos>, CmdError> {
    s.touch(); // lay out the new stroke where it was just placed
    let layout = s.layout();
    let Some(mine) = layout.object(new, page).map(|o| o.rect) else { return Ok(None) };
    // Only the new stroke and its neighbours are recognised, not the whole page.
    let mut pieces = arrow_pieces(s, &layout, page, |pos, rect| pos == new || near(mine, *rect));
    let Some(at) = pieces.iter().position(|k| k.pos == *new) else { return Ok(None) };
    let me = pieces.swap_remove(at);
    // The most recently drawn first (ink records its drawing order; shapes and ink from files
    // don't, and come after, latest on the page first), so the winner doesn't depend on layout.
    pieces.sort_by(|a, b| b.order.cmp(&a.order).then_with(|| b.pos.cmp(&a.pos)));
    for other in &pieces {
        for (line, head) in [(other, &me), (&me, other)] {
            if let Some(arrow) = pair_parts(&line.parts, &head.parts) {
                set_shape(s, &line.pos, &local_to(&arrow, line.rect))?;
                delete_object(s, &head.pos)?;
                return Ok(Some(after_deleting(&line.pos, std::slice::from_ref(&head.pos))));
            }
        }
    }
    Ok(None)
}

/// `shape` (in page points) in the frame of the object laid out at `rect`.
fn local_to(shape: &InkShape, rect: Rect) -> InkShape {
    let m = |[x, y]: [f32; 2]| [x - rect.x, y - rect.y];
    match shape {
        InkShape::Arrow { a, b, head_at_end, head_at_start } => {
            InkShape::Arrow { a: m(*a), b: m(*b), head_at_end: *head_at_end, head_at_start: *head_at_start }
        }
        InkShape::Line { a, b } => InkShape::Line { a: m(*a), b: m(*b) },
        InkShape::Rectangle { x, y, w, h } => InkShape::Rectangle { x: x - rect.x, y: y - rect.y, w: *w, h: *h },
        InkShape::Ellipse { x, y, w, h } => InkShape::Ellipse { x: x - rect.x, y: y - rect.y, w: *w, h: *h },
        InkShape::Polygon(c) => InkShape::Polygon(c.iter().copied().map(m).collect()),
    }
}

/// Replace the ink stroke at `pos` by the shape it looks like, in its colour and thickness and
/// where it was drawn. `None` (nothing changed) when it isn't a pen or pencil stroke, it is
/// turned or flipped, or it doesn't look like a shape.
fn convert_at(s: &mut Session, pos: &Pos) -> Result<Option<&'static str>, CmdError> {
    let Some(InlineObject::Shape { w, h, float, freeform, .. }) = s.doc.para_at(pos).and_then(|p| p.object_at(pos.off)) else { return Ok(None) };
    let Some(f) = freeform.as_deref().filter(|f| matches!(f.ink, Some(InkTool::Pen | InkTool::Pencil)) && f.paths.len() == 1) else {
        return Ok(None);
    };
    if float.rot != 0.0 || float.flip_h || float.flip_v {
        return Ok(None);
    }
    // The stroke in its frame's own points (the frame may have been resized since).
    let Some((pts, _)) = f.placed(0.0, 0.0, *w, *h).into_iter().next() else { return Ok(None) };
    let Some(found) = recognise(&pts) else { return Ok(None) };
    set_shape(s, pos, &found)?;
    Ok(Some(found.name()))
}

/// Make the drawing at `pos` the shape `found` (given in the drawing's own frame), keeping its
/// colour and thickness, where it is.
fn set_shape(s: &mut Session, pos: &Pos, found: &InkShape) -> Result<(), CmdError> {
    let para = s.doc.para_mut(pos.story, &pos.path)?;
    let Some(InlineObject::Shape { kind, w, h, fill, float, freeform, extra, .. }) = para.object_at_mut(pos.off) else {
        return Err(CmdError::Failed("no drawing there".into()));
    };
    let [bx, by, bw, bh] = found.bounds();
    let (bw, bh) = (bw.max(1.0), bh.max(1.0));
    let outline = |pts: Vec<[f32; 2]>, closed: bool| {
        Some(Arc::new(Freeform { w: bw, h: bh, paths: vec![FreePath { pts, closed }], ..Default::default() }.sanitized()))
    };
    let local = |c: &[[f32; 2]]| c.iter().map(|[x, y]| [x - bx, y - by]).collect::<Vec<_>>();
    let (new_kind, geometry) = match found {
        InkShape::Rectangle { .. } => (ShapeKind::Rectangle, None),
        InkShape::Ellipse { .. } => (ShapeKind::Ellipse, None),
        InkShape::Line { a, b } => (ShapeKind::Freeform, outline(local(&[*a, *b]), false)),
        InkShape::Polygon(c) => (ShapeKind::Freeform, outline(local(c), true)),
        // A straight connector runs from its frame's top-left to bottom-right; flips turn it.
        InkShape::Arrow { a, b, head_at_end, head_at_start } => {
            float.flip_h = b[0] < a[0];
            float.flip_v = b[1] < a[1];
            extra.arrow_end = *head_at_end;
            extra.arrow_start = *head_at_start;
            (ShapeKind::StraightConnector, None)
        }
    };
    *kind = new_kind;
    *freeform = geometry;
    *fill = None;
    let [_, _, raw_w, raw_h] = found.bounds();
    (*w, *h) = if matches!(found, InkShape::Arrow { .. }) { (raw_w, raw_h) } else { (bw, bh) };
    float.x += bx;
    float.y += by;
    para.touch();
    Ok(())
}

// --- Ink Replay ----------------------------------------------------------------------------

/// Every ink stroke in the document with its page, in the order it was drawn: strokes that
/// don't record their order (from other files) first, in document order, then the rest by order.
pub fn strokes_in_drawing_order(s: &mut Session) -> Vec<(usize, PlacedInk)> {
    if s.view.hide_ink {
        return Vec::new();
    }
    let layout = s.layout();
    let mut all: Vec<(usize, PlacedInk)> =
        (0..layout.pages.len()).flat_map(|page| ink_on(s, &layout, page).into_iter().map(move |k| (page, k))).collect();
    all.sort_by_key(|(_, k)| k.freeform.order.unwrap_or(0));
    all
}

/// Start or stop Ink Replay; the strokes in the order they'll be replayed.
fn replay(s: &mut Session, v: &Value) -> CmdResult {
    let on = p::bool(v, "value").unwrap_or(!s.view.draw.replay);
    let strokes = if on { strokes_in_drawing_order(s) } else { Vec::new() };
    if on && strokes.is_empty() {
        return Err(CmdError::Failed("there is no ink to replay".into()));
    }
    s.view.draw.replay = on;
    let list: Vec<Value> = strokes
        .iter()
        .map(|(page, k)| {
            json!({"page": page, "pos": pos_json(&k.pos), "order": k.freeform.order, "tool": k.freeform.ink.map(InkTool::name), "points": k.freeform.paths.iter().map(|p| p.pts.len()).sum::<usize>()})
        })
        .collect();
    Ok(json!({"replaying": on, "strokes": list}))
}

#[cfg(test)]
mod tests;
