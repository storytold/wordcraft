//! Draw tab and Review › Ink: pens, the eraser, the lasso and ink strokes.
//!
//! An ink stroke is a floating freeform shape ([`Freeform`] with its pen set) anchored to the
//! paragraph under where it starts, in front of the text (a highlighter's behind it). The pen
//! commands only choose what dragging on the page does (session view state); `draw.stroke` and
//! `draw.erase` change the document.
//!
//! Draw › Lasso Select picks the strokes (and drawings) inside a loop drawn on a page; the
//! selection can then be moved, recoloured, deleted or turned into shapes. Ink to Shape replaces
//! strokes that look like a line, rectangle, ellipse, triangle or polygon with that shape
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

use super::ink_shape::{InkShape, recognise};
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
    /// Its frame on the page.
    pub rect: Rect,
    pub freeform: Arc<Freeform>,
    pub color: Option<Rgb>,
    pub width: f32,
}

impl PlacedInk {
    /// Its paths in page points.
    pub fn paths(&self) -> Vec<Vec<[f32; 2]>> {
        self.freeform.placed(self.rect.x, self.rect.y, self.rect.w, self.rect.h).into_iter().map(|(pts, _)| pts).collect()
    }
}

/// Ink strokes laid out on `page`, in the page's drawing order.
pub fn ink_on(s: &Session, layout: &DocLayout, page: usize) -> Vec<PlacedInk> {
    let Some(pg) = layout.pages.get(page) else { return Vec::new() };
    pg.items
        .iter()
        .filter_map(|it| match it {
            Placed::Object { rect, story, path, off, .. } => {
                let pos = Pos { story: *story, path: path.clone(), off: *off };
                match s.doc.para_at(&pos).and_then(|p| p.object_at(*off)) {
                    Some(InlineObject::Shape { freeform: Some(f), stroke_width, stroke, .. }) if f.is_ink() => {
                        Some(PlacedInk { pos, rect: *rect, freeform: f.clone(), color: *stroke, width: *stroke_width })
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
        k.freeform.distance(k.rect.x, k.rect.y, k.rect.w, k.rect.h, x, y).filter(|d| *d <= reach).map(|_| k.pos)
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
    };
    let at = place(s, &layout, obj, page, (fx, fy), first[1])?;
    // Ink to Shape: a stroke that looks like a shape becomes it (in the same undo step).
    let shape = if s.view.draw.ink_to_shape && tool != InkTool::Highlighter { convert_at(s, &at)? } else { None };
    Ok(json!({"pos": pos_json(&at), "page": page, "rect": [fx, fy, fw, fh], "shape": shape}))
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

// --- Lasso -------------------------------------------------------------------------------

/// The current lasso selection, if the document hasn't changed under it.
pub fn lasso_selection(s: &Session) -> Option<&LassoSelection> {
    s.view.draw.lasso.as_ref().filter(|l| l.rev == s.rev() && !l.items.is_empty())
}

/// Keep `sel` selected after the mutating command running now (which bumps the revision once).
fn keep_lasso(s: &mut Session, sel: LassoSelection) {
    s.view.draw.lasso = Some(LassoSelection { rev: s.rev().wrapping_add(1), ..sel });
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
        let Placed::Object { rect, story, path, off, .. } = it else { continue };
        let pos = Pos { story: *story, path: path.clone(), off: *off };
        let Some(obj) = s.doc.para_at(&pos).and_then(|p| p.object_at(*off)) else { continue };
        let picked = match obj.ink() {
            Some(_) if s.view.hide_ink => false,
            Some(f) => {
                let pts: Vec<[f32; 2]> = f.placed(rect.x, rect.y, rect.w, rect.h).into_iter().flat_map(|(p, _)| p).collect();
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

/// Turn the lasso-selected strokes that look like shapes into those shapes (one undo step).
fn convert_selection(s: &mut Session, _: &Value) -> CmdResult {
    let sel = lasso_required(s)?;
    let mut converted = Vec::new();
    for pos in &sel.items {
        if let Some(name) = convert_at(s, pos)? {
            converted.push(json!({"pos": pos_json(pos), "shape": name}));
        }
    }
    if converted.is_empty() {
        return Err(CmdError::Failed("none of the selected strokes looks like a shape".into()));
    }
    keep_lasso(s, sel);
    Ok(json!({"count": converted.len(), "converted": converted}))
}

/// Replace the ink stroke at `pos` by the shape it looks like, in its colour and thickness and
/// where it was drawn. `None` (nothing changed) when it isn't a pen or pencil stroke, it is
/// turned or flipped, or it doesn't look like a shape.
fn convert_at(s: &mut Session, pos: &Pos) -> Result<Option<&'static str>, CmdError> {
    let para = s.doc.para_mut(pos.story, &pos.path)?;
    let Some(InlineObject::Shape { kind, w, h, fill, float, freeform, .. }) = para.object_at_mut(pos.off) else { return Ok(None) };
    let Some(f) = freeform.as_deref().filter(|f| matches!(f.ink, Some(InkTool::Pen | InkTool::Pencil)) && f.paths.len() == 1) else {
        return Ok(None);
    };
    if float.rot != 0.0 || float.flip_h || float.flip_v {
        return Ok(None);
    }
    // The stroke in its frame's own points (the frame may have been resized since).
    let Some((pts, _)) = f.placed(0.0, 0.0, *w, *h).into_iter().next() else { return Ok(None) };
    let Some(found) = recognise(&pts) else { return Ok(None) };
    let [bx, by, bw, bh] = found.bounds();
    let (bw, bh) = (bw.max(1.0), bh.max(1.0));
    let outline = |pts: Vec<[f32; 2]>, closed: bool| {
        Some(Arc::new(Freeform { w: bw, h: bh, paths: vec![FreePath { pts, closed }], ..Default::default() }.sanitized()))
    };
    let local = |c: &[[f32; 2]]| c.iter().map(|[x, y]| [x - bx, y - by]).collect::<Vec<_>>();
    let (new_kind, geometry) = match &found {
        InkShape::Rectangle { .. } => (ShapeKind::Rectangle, None),
        InkShape::Ellipse { .. } => (ShapeKind::Ellipse, None),
        InkShape::Line { a, b } => (ShapeKind::Freeform, outline(local(&[*a, *b]), false)),
        InkShape::Polygon(c) => (ShapeKind::Freeform, outline(local(c), true)),
    };
    *kind = new_kind;
    *freeform = geometry;
    *fill = None;
    *w = bw;
    *h = bh;
    float.x += bx;
    float.y += by;
    para.touch();
    Ok(Some(found.name()))
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
mod tests {
    use serde_json::json;
    use wordcraft_doc::StoryRef;
    use wordcraft_doc::freeform::InkTool;
    use wordcraft_doc::para::{InlineObject, ShapeKind, Wrap};

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

    /// `draw.eraser` is a view command, but with a point it erases through `draw.erase`, which
    /// changes the document: that must be one undo step and mark the document changed.
    #[test]
    fn erasing_through_the_eraser_command_is_undoable_and_marks_the_document_dirty() {
        let mut s = session();
        s.run("draw.stroke", &json!({"page": 0, "points": [[100, 100], [200, 100]], "width": 2})).unwrap();
        s.dirty = false;
        let steps = s.undo_depth();
        s.run("draw.eraser", &json!({"page": 0, "x": 150, "y": 101})).unwrap();
        assert!(ink(&s).is_empty());
        assert!(s.dirty, "erasing is an unsaved change");
        assert_eq!(s.undo_depth(), steps + 1, "one undo step");
        s.run("edit.undo", &json!({})).unwrap();
        assert_eq!(ink(&s).len(), 1, "undo brings the stroke back");
        s.run("edit.redo", &json!({})).unwrap();
        assert!(ink(&s).is_empty());
    }

    /// A closed rectangle drawn as page points, `per` points to a side.
    fn rect_stroke(x: f32, y: f32, w: f32, h: f32) -> Vec<[f32; 2]> {
        let c = [[x, y], [x + w, y], [x + w, y + h], [x, y + h]];
        let mut pts = Vec::new();
        for i in 0..4 {
            let (a, b) = (c[i], c[(i + 1) % 4]);
            for k in 0..10 {
                let t = k as f32 / 10.0;
                pts.push([a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]);
            }
        }
        pts.push([x, y]);
        pts
    }

    #[test]
    fn the_lasso_selects_the_ink_inside_and_moves_recolours_and_deletes_it() {
        let mut s = session();
        s.run("draw.stroke", &json!({"page": 0, "points": [[100, 100], [140, 110]]})).unwrap();
        s.run("draw.stroke", &json!({"page": 0, "points": [[300, 300], [340, 310]]})).unwrap();
        // Without points it picks up the lasso; a loop around the first stroke selects only it.
        s.run("draw.lasso", &json!({})).unwrap();
        assert_eq!(s.view.draw.mode, super::DrawMode::Lasso);
        let r = s.run("draw.lasso", &json!({"page": 0, "points": [[80, 80], [200, 80], [200, 150], [80, 150]]})).unwrap();
        assert_eq!(r["count"], 1);
        let before = s.layout().object(&super::lasso_selection(&s).unwrap().items[0], 0).unwrap().rect;
        // Moved, recoloured: each one undo step, and the selection stays.
        let steps = s.undo_depth();
        s.run("draw.lassoMove", &json!({"dx": 20, "dy": -10})).unwrap();
        s.run("draw.lassoColor", &json!({"color": "0070C0"})).unwrap();
        assert_eq!(s.undo_depth(), steps + 2);
        let sel = super::lasso_selection(&s).unwrap().items[0].clone();
        let after = s.layout().object(&sel, 0).unwrap().rect;
        assert!((after.x - before.x - 20.0).abs() < 0.01 && (after.y - before.y + 10.0).abs() < 0.01, "{before:?} -> {after:?}");
        let objs = ink(&s);
        let blue = objs.iter().filter(|o| matches!(o, InlineObject::Shape { stroke: Some(c), .. } if c.hex() == "0070C0")).count();
        assert_eq!(blue, 1);
        // Deleted: only the selected stroke goes, and the selection with it.
        s.run("draw.lassoDelete", &json!({})).unwrap();
        assert_eq!(ink(&s).len(), 1);
        assert!(super::lasso_selection(&s).is_none());
        assert!(s.run("draw.lassoMove", &json!({"dx": 1, "dy": 1})).is_err(), "nothing selected");
        // Hostile loops are refused or select nothing; they never panic.
        assert!(s.run("draw.lasso", &json!({"page": 0, "points": [[1, 1], [2, 2]]})).is_err());
        let huge: Vec<[f64; 2]> = (0..20_000).map(|i| [(i % 600) as f64, (i / 40) as f64]).collect();
        let _ = s.run("draw.lasso", &json!({"page": 0, "points": huge}));
    }

    #[test]
    fn ink_to_shape_turns_shape_like_strokes_into_shapes_and_leaves_scribbles() {
        let mut s = session();
        assert_eq!(s.run("draw.inkToShape", &json!({})).unwrap()["value"], true);
        let steps = s.undo_depth();
        let r = s.run("draw.stroke", &json!({"page": 0, "points": rect_stroke(100.0, 100.0, 120.0, 60.0), "color": "C00000", "width": 2})).unwrap();
        assert_eq!(r["shape"], "rectangle");
        assert_eq!(s.undo_depth(), steps + 1, "drawn and converted in one undo step");
        let at: wordcraft_doc::Pos = serde_json::from_value(r["pos"].clone()).unwrap();
        let obj = s.doc.para_at(&at).and_then(|p| p.object_at(at.off)).cloned().unwrap();
        let InlineObject::Shape { kind, w, h, stroke, stroke_width, freeform, .. } = obj else { panic!("{obj:?}") };
        assert_eq!((kind, stroke, stroke_width, freeform.is_none()), (ShapeKind::Rectangle, Some(wordcraft_doc::Rgb(0xC0, 0, 0)), 2.0, true));
        assert!((w - 120.0).abs() < 2.0 && (h - 60.0).abs() < 2.0, "{w} x {h}");
        let rect = s.layout().object(&at, 0).unwrap().rect;
        assert!((rect.x - 100.0).abs() < 2.0 && (rect.y - 100.0).abs() < 2.0, "where it was drawn: {rect:?}");
        // A scribble stays ink.
        let r = s.run("draw.stroke", &json!({"page": 0, "points": [[300, 300], [320, 340], [340, 300], [360, 340], [380, 300]]})).unwrap();
        assert!(r["shape"].is_null());
        assert_eq!(ink(&s).len(), 1);
        // Off again; a drawn ellipse stays ink until the lasso selects it and Ink to Shape converts it.
        s.run("draw.inkToShape", &json!({"value": false})).unwrap();
        let ring: Vec<[f32; 2]> =
            (0..=48).map(|i| i as f32 / 48.0 * std::f32::consts::TAU).map(|a| [200.0 + 50.0 * a.cos(), 500.0 + 30.0 * a.sin()]).collect();
        s.run("draw.stroke", &json!({"page": 0, "points": ring})).unwrap();
        assert_eq!(ink(&s).len(), 2);
        s.run("draw.lasso", &json!({"page": 0, "points": [[130, 450], [270, 450], [270, 550], [130, 550]]})).unwrap();
        let r = s.run("draw.inkToShape", &json!({})).unwrap();
        assert_eq!(r["converted"][0]["shape"], "ellipse");
        assert_eq!(ink(&s).len(), 1);
        assert!(!s.view.draw.ink_to_shape, "converting the selection doesn't turn the mode on");
    }

    #[test]
    fn ink_replay_plays_strokes_in_the_order_they_were_drawn() {
        let mut s = session();
        // Drawn low on the page first, then higher: document order would be the other way round.
        s.run("draw.stroke", &json!({"page": 0, "points": [[100, 400], [150, 400]]})).unwrap();
        s.run("draw.stroke", &json!({"page": 0, "points": [[100, 90], [150, 90]]})).unwrap();
        let r = s.run("draw.replay", &json!({})).unwrap();
        assert!(s.view.draw.replay);
        let orders: Vec<u64> = r["strokes"].as_array().unwrap().iter().map(|k| k["order"].as_u64().unwrap()).collect();
        assert_eq!(orders, vec![1, 2]);
        let ys: Vec<f32> = super::strokes_in_drawing_order(&mut s).iter().map(|(_, k)| k.rect.y).collect();
        assert!(ys[0] > ys[1], "the lower stroke was drawn first: {ys:?}");
        assert_eq!(s.run("draw.replay", &json!({})).unwrap()["replaying"], false);
        // Nothing to replay without ink.
        assert!(session().run("draw.replay", &json!({})).is_err());
    }

    #[test]
    fn add_pen_validates_and_saved_galleries_drop_junk() {
        let mut s = session();
        let r = s.run("draw.addPen", &json!({"kind": "highlighter", "color": "00B050", "width": 1e9})).unwrap();
        assert_eq!(r["pen"], json!({"kind": "highlighter", "color": "00B050", "width": super::MAX_WIDTH}));
        assert_eq!(s.view.draw.mode, super::DrawMode::Pen(InkTool::Highlighter), "the new pen is picked up");
        for bad in [json!({"kind": "crayon"}), json!({"color": "blue"}), json!(5), json!(null)] {
            assert!(s.run("draw.addPen", &bad).is_err(), "{bad}");
        }
        for _ in 0..super::MAX_PENS + 3 {
            let _ = s.run("draw.addPen", &json!({}));
        }
        assert_eq!(s.view.draw.pens.len(), super::MAX_PENS);
        s.run("draw.removePen", &json!({"index": 0})).unwrap();
        assert!(s.run("draw.removePen", &json!({"index": 999})).is_err());
        // A saved gallery keeps its pens and drops the junk around them.
        let saved =
            json!({"pens": [{"kind": "pencil", "color": "7030A0", "width": 3}, "junk", {"kind": "crayon"}, {"width": "x"}], "inkToShape": true});
        let d: super::DrawState = serde_json::from_value(saved).unwrap();
        assert_eq!(d.pens.len(), 2);
        assert_eq!((d.pens[0].kind, d.pens[0].color.hex(), d.pens[0].width), (InkTool::Pencil, "7030A0".to_string(), 3.0));
        assert!(d.ink_to_shape);
        let back: super::DrawState = serde_json::from_value(serde_json::to_value(&d).unwrap()).unwrap();
        assert_eq!(back.pens, d.pens);
    }
}
