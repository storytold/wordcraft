//! Drawing Canvas (#344): Insert › Illustrations › Shapes › New Drawing Canvas.
//!
//! A canvas is an [`InlineObject::Group`] with a [`CanvasStyle`]: a frame with its own fill and
//! outline that holds pictures, shapes and text boxes at offsets in the canvas (points from its
//! top-left corner, unscaled). While a canvas is selected, Insert › Pictures, Shapes and Text Box
//! put the new object inside it ([`add_member`]). Fit, Expand and Scale Drawing change its frame;
//! Shape Fill and Shape Outline colour its background and border.

use serde_json::{Value, json};
use wordcraft_doc::para::{CanvasStyle, Float, GroupChild, InlineObject, MAX_GROUP_CHILDREN};
use wordcraft_doc::{PartKind, Pos, StoryRef};

use super::objects::object_selection;
use super::{delete_selection, sel_result};
use crate::{CmdError, CmdResult, CommandSpec, Selection, Session, p};

/// Largest canvas side and member offset, points (well past any page).
const MAX_SIDE: f32 = 4000.0;
/// Smallest canvas side, points.
const MIN_SIDE: f32 = 9.0;
/// Room Fit leaves around the members, points (none: Word's Fit hugs the drawing).
const FIT_MARGIN: f32 = 0.0;

fn has_canvas(s: &Session) -> Option<&'static str> {
    if selected_canvas(s).is_some() { None } else { Some("select a drawing canvas first") }
}

/// The selected Drawing Canvas: its position and the canvas.
pub fn selected_canvas(s: &Session) -> Option<(Pos, &InlineObject)> {
    object_selection(s).filter(|(_, o)| o.is_canvas())
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("insert.canvas", "Drawing Canvas", "Insert › Illustrations", insert)
            .params(r#"{"width"?: pt (default 432 = 6"), "height"?: pt (default 216 = 3"), "fill"?: "RRGGBB", "outline"?: "RRGGBB"}  (in line with text at the caret; it stays selected, so the next Pictures, Shapes or Text Box goes inside it)"#)
            .when(|s| {
                matches!(s.sel.focus.story, StoryRef::Part(id) if s.doc.parts.get(&id).is_some_and(|p| p.kind == PartKind::TextBox))
                    .then_some("a drawing canvas can't go inside a text box")
            }),
        CommandSpec::new("canvas.fit", "Fit", "Shape Format › Drawing Canvas", fit)
            .params(r#"{}  (shrinks the selected canvas's frame to its drawing: the members move to its top-left corner)"#)
            .when(has_canvas),
        CommandSpec::new("canvas.expand", "Expand", "Shape Format › Drawing Canvas", expand)
            .params(r#"{"by"?: pt (default 36)}  (grows the frame right and down; at least enough to show every member)"#)
            .when(has_canvas),
        CommandSpec::new("canvas.scale", "Scale Drawing", "Shape Format › Drawing Canvas", scale)
            .params(r#"{"factor": number (0.1–10)}  (scales the frame and every member together)"#)
            .when(has_canvas),
        CommandSpec::new("canvas.member", "Canvas Member", "Shape Format › Drawing Canvas", member)
            .params(r#"{"index": n, "x"?: pt, "y"?: pt, "width"?: pt, "height"?: pt, "delete"?: bool}  (moves, resizes or removes member `index` of the selected canvas; x/y from the canvas's top-left; with no change, returns the members)"#)
            .when(has_canvas),
    ]
}

/// `insert.canvas`: a new empty canvas in line with text at the caret, selected.
fn insert(s: &mut Session, v: &Value) -> CmdResult {
    let w = p::f32(v, "width").unwrap_or(432.0).clamp(MIN_SIDE, MAX_SIDE);
    let h = p::f32(v, "height").unwrap_or(216.0).clamp(MIN_SIDE, MAX_SIDE);
    let style = CanvasStyle {
        fill: p::str(v, "fill").and_then(wordcraft_doc::props::Rgb::parse),
        stroke: p::str(v, "outline").and_then(wordcraft_doc::props::Rgb::parse),
        stroke_width: 0.75,
    };
    let props = s.typing_props();
    let at = delete_selection(s)?;
    let obj = InlineObject::Group { w, h, float: Float::default(), ch_w: w, ch_h: h, children: Vec::new(), canvas: Some(style) };
    let end = s.doc.insert_object(&at, obj, &props)?;
    s.sel = Selection { anchor: at, focus: end };
    sel_result(s)
}

/// With a Drawing Canvas selected, put `obj` inside it rather than at the caret: centred, each
/// later one a little lower and to the right, shrunk (keeping its proportions) to fit. Returns
/// the member's index, or `None` when no canvas is selected (insert as usual).
pub fn add_member(s: &mut Session, obj: InlineObject) -> Result<Option<usize>, CmdError> {
    let Some((pos, canvas)) = selected_canvas(s) else { return Ok(None) };
    let InlineObject::Group { w: cw, h: ch, children, .. } = canvas else { return Ok(None) };
    if children.len() >= MAX_GROUP_CHILDREN {
        return Err(CmdError::Failed("the drawing canvas is full".into()));
    }
    let (cw, ch, n) = (*cw, *ch, children.len());
    let mut obj = obj;
    let (ow, oh, _) = obj.frame().ok_or_else(|| CmdError::Params("not a picture or shape".into()))?;
    let k = (cw / ow.max(1.0)).min(ch / oh.max(1.0)).min(1.0);
    let (ow, oh) = ((ow * k).max(1.0), (oh * k).max(1.0));
    obj.set_size(ow, oh);
    if let Some(f) = obj.float_mut() {
        *f = Float::default();
    }
    let step = 12.0 * (n % 6) as f32;
    let x = ((cw - ow) / 2.0 + step).clamp(0.0, (cw - ow).max(0.0));
    let y = ((ch - oh) / 2.0 + step).clamp(0.0, (ch - oh).max(0.0));
    edit_canvas(s, &pos, |_, _, children| children.push(GroupChild { x, y, obj: obj.clone() }))?;
    Ok(Some(n))
}

/// Change the canvas at `pos`: `f` gets its frame size and members (and may change them).
fn edit_canvas(s: &mut Session, pos: &Pos, f: impl FnOnce(&mut f32, &mut f32, &mut Vec<GroupChild>)) -> Result<InlineObject, CmdError> {
    let para = s.doc.para_mut(pos.story, &pos.path)?;
    let o = para.object_at_mut(pos.off).ok_or_else(|| CmdError::Failed("object vanished".into()))?;
    let InlineObject::Group { w, h, ch_w, ch_h, children, canvas: Some(_), .. } = o else {
        return Err(CmdError::Disabled("select a drawing canvas first".into()));
    };
    f(w, h, children);
    *w = clean(*w).clamp(MIN_SIDE, MAX_SIDE);
    *h = clean(*h).clamp(MIN_SIDE, MAX_SIDE);
    (*ch_w, *ch_h) = (*w, *h);
    let o = o.clone();
    para.touch();
    s.touch();
    Ok(o)
}

fn clean(v: f32) -> f32 {
    if v.is_finite() { v } else { 0.0 }
}

fn canvas_pos(s: &Session) -> Result<Pos, CmdError> {
    selected_canvas(s).map(|(p, _)| p).ok_or_else(|| CmdError::Disabled("select a drawing canvas first".into()))
}

fn result(o: &InlineObject) -> CmdResult {
    Ok(json!({"object": serde_json::to_value(o).unwrap_or(Value::Null)}))
}

/// The members' bounds (x0, y0, x1, y1) in the canvas, or `None` when it is empty.
fn bounds(children: &[GroupChild]) -> Option<[f32; 4]> {
    children
        .iter()
        .filter_map(|c| c.obj.frame().map(|(w, h, _)| [clean(c.x), clean(c.y), clean(c.x) + clean(w), clean(c.y) + clean(h)]))
        .reduce(|a, b| [a[0].min(b[0]), a[1].min(b[1]), a[2].max(b[2]), a[3].max(b[3])])
}

/// `canvas.fit`: the frame shrinks (or grows) to the drawing, which moves to its corner.
fn fit(s: &mut Session, _: &Value) -> CmdResult {
    let pos = canvas_pos(s)?;
    let o = edit_canvas(s, &pos, |w, h, children| {
        let Some([x0, y0, x1, y1]) = bounds(children) else { return };
        for c in children.iter_mut() {
            c.x = (c.x - x0 + FIT_MARGIN).clamp(-MAX_SIDE, MAX_SIDE);
            c.y = (c.y - y0 + FIT_MARGIN).clamp(-MAX_SIDE, MAX_SIDE);
        }
        *w = x1 - x0 + 2.0 * FIT_MARGIN;
        *h = y1 - y0 + 2.0 * FIT_MARGIN;
    })?;
    result(&o)
}

/// `canvas.expand`: more room to the right and below, at least enough for every member.
fn expand(s: &mut Session, v: &Value) -> CmdResult {
    let by = p::f32(v, "by").unwrap_or(36.0).clamp(0.0, MAX_SIDE);
    let pos = canvas_pos(s)?;
    let o = edit_canvas(s, &pos, |w, h, children| {
        let [_, _, x1, y1] = bounds(children).unwrap_or([0.0; 4]);
        *w = (*w + by).max(x1);
        *h = (*h + by).max(y1);
    })?;
    result(&o)
}

/// `canvas.scale`: the frame and the drawing scale together.
fn scale(s: &mut Session, v: &Value) -> CmdResult {
    let k = p::req_f32(v, "factor")?;
    if !(0.1..=10.0).contains(&k) {
        return Err(CmdError::Params("`factor` must be between 0.1 and 10".into()));
    }
    let pos = canvas_pos(s)?;
    let o = edit_canvas(s, &pos, |w, h, children| {
        *w *= k;
        *h *= k;
        for c in children.iter_mut() {
            c.x = (c.x * k).clamp(-MAX_SIDE, MAX_SIDE);
            c.y = (c.y * k).clamp(-MAX_SIDE, MAX_SIDE);
            if let Some((mw, mh, _)) = c.obj.frame() {
                c.obj.set_size((mw * k).clamp(1.0, MAX_SIDE), (mh * k).clamp(1.0, MAX_SIDE));
            }
        }
    })?;
    result(&o)
}

/// `canvas.member`: move, resize or remove one member; with no change, list the members.
fn member(s: &mut Session, v: &Value) -> CmdResult {
    let pos = canvas_pos(s)?;
    let Some(i) = p::u64(v, "index").and_then(|i| usize::try_from(i).ok()) else {
        let Some((_, InlineObject::Group { children, .. })) = selected_canvas(s) else {
            return Err(CmdError::Disabled("select a drawing canvas first".into()));
        };
        let list: Vec<Value> = children
            .iter()
            .enumerate()
            .map(|(i, c)| {
                let (w, h) = c.obj.frame().map(|(w, h, _)| (w, h)).unwrap_or((0.0, 0.0));
                json!({"index": i, "x": c.x, "y": c.y, "width": w, "height": h})
            })
            .collect();
        return Ok(json!({"members": list}));
    };
    let count = match selected_canvas(s) {
        Some((_, InlineObject::Group { children, .. })) => children.len(),
        _ => 0,
    };
    if i >= count {
        return Err(CmdError::Params(format!("no member {i} (the canvas has {count})")));
    }
    let (x, y, nw, nh) = (p::f32(v, "x"), p::f32(v, "y"), p::f32(v, "width"), p::f32(v, "height"));
    let delete = p::bool(v, "delete").unwrap_or(false);
    let o = edit_canvas(s, &pos, |_, _, children| {
        if delete {
            if i < children.len() {
                children.remove(i);
            }
            return;
        }
        let Some(c) = children.get_mut(i) else { return };
        if let Some(x) = x {
            c.x = x.clamp(-MAX_SIDE, MAX_SIDE);
        }
        if let Some(y) = y {
            c.y = y.clamp(-MAX_SIDE, MAX_SIDE);
        }
        if let Some((w0, h0, _)) = c.obj.frame() {
            c.obj.set_size(nw.unwrap_or(w0).clamp(1.0, MAX_SIDE), nh.unwrap_or(h0).clamp(1.0, MAX_SIDE));
        }
    })?;
    result(&o)
}

/// Shape Fill and Shape Outline on a selected canvas colour its background and border.
pub fn style_mut(o: &mut InlineObject) -> Option<&mut CanvasStyle> {
    match o {
        InlineObject::Group { canvas: Some(c), .. } => Some(c),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn canvas(s: &Session) -> InlineObject {
        selected_canvas(s).map(|(_, o)| o.clone()).unwrap()
    }
    fn members(o: &InlineObject) -> Vec<(f32, f32, f32, f32)> {
        let InlineObject::Group { children, .. } = o else { panic!("{o:?}") };
        children
            .iter()
            .map(|c| {
                let (w, h, _) = c.obj.frame().unwrap();
                (c.x, c.y, w, h)
            })
            .collect()
    }

    /// Insert › Drawing Canvas, then Shapes and Text Box: the new objects go inside the
    /// selected canvas, not into the text, and the canvas's frame keeps its size (#344).
    #[test]
    fn shapes_inserted_with_a_canvas_selected_go_inside_it() {
        let mut s = Session::new(wordcraft_doc::Document::new());
        s.run("text.insert", &json!({"text": "Before "})).unwrap();
        s.run("insert.canvas", &json!({})).unwrap();
        let c = canvas(&s);
        assert_eq!(c.frame().map(|(w, h, _)| (w, h)), Some((432.0, 216.0)));
        s.run("insert.shape", &json!({"kind": "ellipse", "width": 100, "height": 50})).unwrap();
        s.run("insert.shape", &json!({"kind": "star", "width": 60, "height": 60})).unwrap();
        let c = canvas(&s);
        let m = members(&c);
        assert_eq!(m.len(), 2);
        assert_eq!(m[0], (166.0, 83.0, 100.0, 50.0), "centred in the canvas");
        assert_eq!((m[1].2, m[1].3), (60.0, 60.0));
        // Only the canvas sits in the text.
        let para = s.doc.body.first().and_then(|b| b.as_para()).unwrap();
        assert_eq!(para.objects.len(), 1);
        // Fill and outline colour the canvas.
        s.run("shape.fill", &json!({"color": "FFF2CC"})).unwrap();
        s.run("shape.outline", &json!({"color": "1F4E79", "width": 1.5})).unwrap();
        let style = *canvas(&s).canvas_style().unwrap();
        assert_eq!(
            style,
            CanvasStyle { fill: wordcraft_doc::props::Rgb::parse("FFF2CC"), stroke: wordcraft_doc::props::Rgb::parse("1F4E79"), stroke_width: 1.5 }
        );
        // Members move, resize and go within the canvas.
        s.run("canvas.member", &json!({"index": 1, "x": 10, "y": 20, "width": 30})).unwrap();
        assert_eq!(members(&canvas(&s))[1], (10.0, 20.0, 30.0, 60.0));
        // A text box goes in too, with its own story.
        s.run("select.objects", &json!({"index": 0})).unwrap();
        s.run("insert.textBox", &json!({"text": "Label"})).unwrap();
        let para = s.doc.body.first().and_then(|b| b.as_para()).unwrap();
        assert_eq!(para.objects.len(), 1);
        assert_eq!(para.objects[0].text_boxes().len(), 1);
        // Laid out: the background, then the members clipped to the frame.
        let pages = s.layout().pages.clone();
        assert!(!pages.is_empty());
    }

    /// Fit shrinks the frame to the drawing and moves the drawing to its corner; Expand grows it;
    /// Scale Drawing scales frame and members together.
    #[test]
    fn fit_expand_and_scale_change_the_frame() {
        let mut s = Session::new(wordcraft_doc::Document::new());
        s.run("insert.canvas", &json!({"width": 400, "height": 300})).unwrap();
        s.run("insert.shape", &json!({"kind": "rectangle", "width": 100, "height": 40})).unwrap();
        s.run("insert.shape", &json!({"kind": "ellipse", "width": 50, "height": 50})).unwrap();
        s.run("canvas.member", &json!({"index": 0, "x": 40, "y": 30})).unwrap();
        s.run("canvas.member", &json!({"index": 1, "x": 200, "y": 100})).unwrap();
        s.run("canvas.fit", &json!({})).unwrap();
        let c = canvas(&s);
        assert_eq!(c.frame().map(|(w, h, _)| (w, h)), Some((210.0, 120.0)));
        assert_eq!(members(&c), [(0.0, 0.0, 100.0, 40.0), (160.0, 70.0, 50.0, 50.0)]);
        s.run("canvas.expand", &json!({"by": 20})).unwrap();
        assert_eq!(canvas(&s).frame().map(|(w, h, _)| (w, h)), Some((230.0, 140.0)));
        s.run("canvas.scale", &json!({"factor": 0.5})).unwrap();
        let c = canvas(&s);
        assert_eq!(c.frame().map(|(w, h, _)| (w, h)), Some((115.0, 70.0)));
        assert_eq!(members(&c)[1], (80.0, 35.0, 25.0, 25.0));
        assert!(s.run("canvas.scale", &json!({"factor": 0.0})).is_err());
        // Undo puts the drawing back as it was before the scale.
        s.run("edit.undo", &json!({})).unwrap();
        assert_eq!(canvas_in_text(&s).frame().map(|(w, h, _)| (w, h)), Some((230.0, 140.0)));
    }

    fn canvas_in_text(s: &Session) -> InlineObject {
        s.doc.body.first().and_then(|b| b.as_para()).and_then(|p| p.objects.first()).cloned().unwrap()
    }
}
