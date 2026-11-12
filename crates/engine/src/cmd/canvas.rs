//! Drawing Canvas (#344): Insert › Illustrations › Shapes › New Drawing Canvas.
//!
//! A canvas is an [`InlineObject::Group`] with a [`CanvasStyle`]: a frame with its own fill and
//! outline that holds pictures, shapes and text boxes at offsets in the canvas (points from its
//! top-left corner, unscaled). While a canvas is selected, Insert › Pictures, Shapes and Text Box
//! put the new object inside it ([`add_member`]). Fit, Expand and Scale Drawing change its frame;
//! Shape Fill and Shape Outline colour its background and border.
//!
//! A member can be picked on its own (#2637, `select.member`; a click on it does this): Shape
//! Format, Size, Rotate, Alt Text, Delete, Cut and Copy then act on that member, and Move or
//! Resize and Nudge move it inside the canvas. The canvas stays the selection in the text.

use serde_json::{Value, json};
use wordcraft_doc::para::{CanvasStyle, Float, GroupChild, InlineObject, MAX_GROUP_CHILDREN};
use wordcraft_doc::{PartKind, Pos, StoryRef};

use super::objects::object_selection;
use super::{delete_selection, sel_result};
use crate::{CmdError, CmdResult, CommandSpec, Selection, Session, p};

/// A member of the selected canvas picked on its own: its index, and the selection it was picked
/// in (it counts only while that selection stands).
#[derive(Clone, Debug, PartialEq)]
pub struct CanvasMember {
    sel: Selection,
    pub index: usize,
}

/// The canvas member picked on its own: the canvas's position, the member's index and the member.
pub fn selected_member(s: &Session) -> Option<(Pos, usize, &InlineObject)> {
    let m = s.member.as_ref().filter(|m| m.sel == s.sel)?;
    let (pos, canvas) = selected_canvas(s)?;
    let InlineObject::Group { children, .. } = canvas else { return None };
    let c = children.get(m.index).filter(|c| pickable(c))?;
    Some((pos, m.index, &c.obj))
}

/// Whether a member can be picked on its own: a picture, shape or text box that is drawn (a
/// group inside a canvas isn't, nor a hidden member).
pub fn pickable(c: &GroupChild) -> bool {
    c.obj.is_drawing() && !matches!(c.obj, InlineObject::Group { .. }) && !c.obj.frame().is_some_and(|(_, _, f)| f.hidden)
}

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
        CommandSpec::new("select.member", "Select Canvas Member", "Shape Format › Drawing Canvas", select_member)
            .params(r#"{"index": n | null}  (picks member `index` of the selected drawing canvas on its own: Shape Format, Size, Rotate, Alt Text, Delete, Cut and Copy then act on it, and Move or Resize and Nudge move it inside the canvas; null: the canvas itself again)"#)
            .when(has_canvas)
            .pure(),
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
        fill: p::str(v, "fill").and_then(wordcraft_doc::props::Rgb::parse).map(wordcraft_doc::fill::Fill::solid),
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
    // The indices after a removed member shift down, so a picked member would name another one.
    if delete {
        s.member = None;
    }
    result(&o)
}

/// `select.member`: pick a member of the selected canvas on its own, or (`null`) the canvas again.
fn select_member(s: &mut Session, v: &Value) -> CmdResult {
    let Some((_, InlineObject::Group { children, .. })) = selected_canvas(s) else {
        return Err(CmdError::Disabled("select a drawing canvas first".into()));
    };
    let index = match v.get("index") {
        None | Some(Value::Null) => None,
        Some(_) => {
            Some(p::u64(v, "index").and_then(|i| usize::try_from(i).ok()).ok_or_else(|| CmdError::Params("`index` is a member number".into()))?)
        }
    };
    let Some(i) = index else {
        s.member = None;
        return sel_result(s);
    };
    let count = children.len();
    let c = children.get(i).ok_or_else(|| CmdError::Params(format!("no member {i} (the canvas has {count})")))?;
    if !pickable(c) {
        return Err(CmdError::Params(format!("member {i} can't be selected on its own")));
    }
    let (w, h) = c.obj.frame().map(|(w, h, _)| (w, h)).unwrap_or((0.0, 0.0));
    let out = json!({"index": i, "x": c.x, "y": c.y, "width": w, "height": h, "object": serde_json::to_value(&c.obj).unwrap_or(Value::Null)});
    s.member = Some(CanvasMember { sel: s.sel.clone(), index: i });
    Ok(out)
}

/// Delete, Backspace (and their word forms), Cut and Copy with a member picked act on the member
/// alone; the canvas stays selected. Typing and pasting, which would replace the whole canvas,
/// are disabled. `None`: another command, or no member picked.
pub(crate) fn member_command(s: &mut Session, id: &str, _: &Value) -> Option<CmdResult> {
    let replaces = (id.starts_with("text.") && id != "text.overtype")
        || matches!(
            id,
            "edit.paste" | "edit.pasteText" | "edit.pasteMerge" | "edit.pasteSpecial" | "edit.pasteClipboardItem" | "edit.pasteAllClipboard"
        );
    if !replaces && !matches!(id, "edit.cut" | "edit.copy") {
        return None;
    }
    let (pos, index, _) = selected_member(s)?;
    Some(match id {
        "edit.copy" => copy_member(s, &pos, index),
        "edit.cut" => copy_member(s, &pos, index).and_then(|r| remove_member(s, &pos, index).map(|_| r)),
        "text.delete" | "text.backspace" | "text.deleteWordBack" | "text.deleteWordForward" => {
            remove_member(s, &pos, index).and_then(|_| sel_result(s))
        }
        _ => Err(CmdError::Disabled("a shape inside a drawing canvas is selected".into())),
    })
}

/// Copy member `index` of the canvas at `pos` alone: pasted, it is a picture or shape of its own.
fn copy_member(s: &mut Session, pos: &Pos, index: usize) -> CmdResult {
    let end = Pos { off: pos.off + wordcraft_doc::para::OBJ.len_utf8(), ..pos.clone() };
    let mut f = s.doc.copy_range(pos, &end);
    let mut copied = false;
    for b in &mut f.blocks {
        let wordcraft_doc::Block::Para(p) = b else { continue };
        for o in &mut p.objects {
            if let InlineObject::Group { children, canvas: Some(_), .. } = o
                && let Some(c) = children.get(index)
            {
                *o = c.obj.clone();
                copied = true;
            }
        }
    }
    if !copied {
        return Err(CmdError::Failed("the member vanished".into()));
    }
    s.clipboard_text = f.clipboard_text();
    s.clip_history.push(&f, &s.clipboard_text);
    s.clipboard = Some(f);
    Ok(super::edit::copy_result(s))
}

/// Take member `index` out of the canvas at `pos`; the canvas stays selected.
fn remove_member(s: &mut Session, pos: &Pos, index: usize) -> Result<(), CmdError> {
    edit_canvas(s, pos, |_, _, children| {
        if index < children.len() {
            children.remove(index);
        }
    })?;
    s.member = None;
    Ok(())
}

/// Move or Resize with a member picked: `x`/`y` (the page point of its top-left, as dragging its
/// frame gives it) and the size become its place and size in the canvas. `None`: no member.
pub(crate) fn member_bounds(s: &mut Session, v: &Value) -> Option<CmdResult> {
    let (pos, index, obj) = selected_member(s)?;
    let (w0, h0, _) = obj.frame()?;
    let min = super::objects::min_size(obj);
    let to = match (p::f32(v, "x"), p::f32(v, "y")) {
        (Some(x), Some(y)) => Some((x, y)),
        (None, None) => None,
        _ => return Some(Err(CmdError::Params("`x` and `y` go together".into()))),
    };
    // `x`/`y` are on the canvas's page: a drop on another page would put the member outside it.
    if to.is_some()
        && let Some(page) = p::u64(v, "page").and_then(|n| usize::try_from(n).ok())
        && s.layout().object(&pos, s.page_hint).is_some_and(|o| o.page != page)
    {
        return Some(Err(CmdError::Disabled("a shape can't leave its drawing canvas".into())));
    }
    let (nw, nh) = (p::f32(v, "width"), p::f32(v, "height"));
    Some(edit_member(s, &pos, index, |frame, (sx, sy), c| {
        if let Some((x, y)) = to {
            c.x = ((x - frame.x) / sx).clamp(-MAX_SIDE, MAX_SIDE);
            c.y = ((y - frame.y) / sy).clamp(-MAX_SIDE, MAX_SIDE);
        }
        let w = nw.map_or(w0, |w| w / sx).clamp(min, MAX_SIDE);
        let h = nh.map_or(h0, |h| h / sy).clamp(min, MAX_SIDE);
        if (w, h) != (w0, h0) {
            c.obj.set_size(w, h);
        }
    }))
}

/// Nudge with a member picked: it moves inside the canvas. `None`: no member.
pub(crate) fn member_nudge(s: &mut Session, dx: f32, dy: f32) -> Option<CmdResult> {
    let (pos, index, _) = selected_member(s)?;
    Some(edit_member(s, &pos, index, |_, (sx, sy), c| {
        c.x = (c.x + dx / sx).clamp(-MAX_SIDE, MAX_SIDE);
        c.y = (c.y + dy / sy).clamp(-MAX_SIDE, MAX_SIDE);
    }))
}

/// Change member `index` of the canvas at `pos`: `f` gets the canvas's frame on the page, the
/// scale of its members' space and the member.
fn edit_member(s: &mut Session, pos: &Pos, index: usize, f: impl FnOnce(wordcraft_geom::Rect, (f32, f32), &mut GroupChild)) -> CmdResult {
    let frame = s.layout().object(pos, s.page_hint).ok_or_else(|| CmdError::Failed("the drawing canvas isn't laid out".into()))?.rect;
    let canvas =
        s.doc.para_at(pos).and_then(|p| p.object_at(pos.off)).cloned().ok_or_else(|| CmdError::Failed("the drawing canvas vanished".into()))?;
    let scale = canvas.group_scale(frame.w, frame.h);
    let mut f = Some(f);
    let canvas = edit_canvas(s, pos, |_, _, children| {
        if let (Some(c), Some(f)) = (children.get_mut(index), f.take()) {
            f(frame, scale, c);
        }
    })?;
    let InlineObject::Group { children, .. } = &canvas else { return result(&canvas) };
    let member = children.get(index).map(|c| serde_json::to_value(&c.obj).unwrap_or(Value::Null)).unwrap_or(Value::Null);
    Ok(json!({"object": member, "pos": super::pos_json(pos), "member": index}))
}

/// Change the picked member's own object (Shape Format, Size, Rotate, Alt Text) when `pos` is its
/// canvas. `None`: no member picked there.
pub(crate) fn edit_member_object(s: &mut Session, pos: &Pos, f: impl Fn(&mut InlineObject)) -> Option<Result<InlineObject, CmdError>> {
    let (at, index, _) = selected_member(s).filter(|(at, ..)| at == pos)?;
    let mut out = None;
    let r = edit_canvas(s, &at, |_, _, children| {
        if let Some(c) = children.get_mut(index) {
            f(&mut c.obj);
            out = Some(c.obj.clone());
        }
    });
    Some(r.and_then(|_| out.ok_or_else(|| CmdError::Failed("the member vanished".into()))))
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
        let style = canvas(&s).canvas_style().unwrap().clone();
        assert_eq!(
            style,
            CanvasStyle {
                fill: wordcraft_doc::props::Rgb::parse("FFF2CC").map(wordcraft_doc::fill::Fill::solid),
                stroke: wordcraft_doc::props::Rgb::parse("1F4E79"),
                stroke_width: 1.5
            }
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

    fn member_fill(o: &InlineObject, i: usize) -> Option<wordcraft_doc::fill::Fill> {
        let InlineObject::Group { children, .. } = o else { panic!("{o:?}") };
        match &children[i].obj {
            InlineObject::Shape { fill, .. } => fill.clone(),
            o => panic!("{o:?}"),
        }
    }

    /// #2637: a shape inside a drawing canvas can be picked on its own, as in Word. Shape Format,
    /// Move or Resize, Nudge, Delete and Copy then act on it rather than on the whole canvas,
    /// which stays the selection in the text; the commands that place an object in the text
    /// (wrapping, position) don't apply to it.
    #[test]
    fn a_canvas_member_is_selected_and_changed_on_its_own() {
        let mut s = Session::new(wordcraft_doc::Document::new());
        s.run("insert.canvas", &json!({"width": 400, "height": 300})).unwrap();
        s.run("insert.shape", &json!({"kind": "rectangle", "width": 100, "height": 40})).unwrap();
        s.run("insert.shape", &json!({"kind": "ellipse", "width": 50, "height": 50})).unwrap();
        s.run("canvas.member", &json!({"index": 0, "x": 40, "y": 30})).unwrap();
        s.run("canvas.member", &json!({"index": 1, "x": 200, "y": 100})).unwrap();
        let canvas_sel = s.sel.clone();
        assert!(s.run("select.member", &json!({"index": 5})).is_err(), "no such member");
        s.run("select.member", &json!({"index": 1})).unwrap();
        assert_eq!(selected_member(&s).map(|(_, i, _)| i), Some(1));
        assert_eq!(s.sel, canvas_sel, "the canvas stays the selection in the text");

        // Shape Fill colours the member, not the canvas or the other member.
        let other = member_fill(&canvas(&s), 0);
        s.run("shape.fill", &json!({"color": "FF0000"})).unwrap();
        let c = canvas(&s);
        assert_eq!(member_fill(&c, 1), wordcraft_doc::props::Rgb::parse("FF0000").map(wordcraft_doc::fill::Fill::solid));
        assert_eq!(member_fill(&c, 0), other);
        assert_eq!(c.canvas_style().and_then(|st| st.fill.clone()), None);
        assert!(selected_member(&s).is_some(), "still picked after a change");

        // Dragging its frame: the page point becomes its place in the canvas.
        let frame = s.layout().object(&selected_canvas(&s).unwrap().0, 0).unwrap().rect;
        s.run("arrange.bounds", &json!({"page": 0, "x": frame.x + 10.0, "y": frame.y + 20.0, "width": 60, "height": 30})).unwrap();
        assert_eq!(members(&canvas(&s))[1], (10.0, 20.0, 60.0, 30.0));
        assert_eq!(canvas(&s).frame().map(|(w, h, _)| (w, h)), Some((400.0, 300.0)), "the canvas keeps its frame");
        s.run("arrange.nudge", &json!({"dx": 6, "dy": -6})).unwrap();
        assert_eq!(members(&canvas(&s))[1], (16.0, 14.0, 60.0, 30.0));
        assert!(s.run("arrange.wrap", &json!({"wrap": "square"})).is_err(), "the canvas places its members");

        // Copy puts the member alone on the clipboard; Delete takes it out of the canvas.
        s.run("edit.copy", &json!({})).unwrap();
        let clip = s.clipboard.clone().unwrap();
        let copied: Vec<&InlineObject> = clip.blocks.iter().filter_map(|b| b.as_para()).flat_map(|p| p.objects.iter()).collect();
        assert!(matches!(copied.as_slice(), [InlineObject::Shape { kind: wordcraft_doc::para::ShapeKind::Ellipse, .. }]), "{copied:?}");
        s.run("text.delete", &json!({})).unwrap();
        assert_eq!(members(&canvas(&s)), [(40.0, 30.0, 100.0, 40.0)]);
        assert!(selected_member(&s).is_none());
        assert_eq!(s.sel, canvas_sel, "the canvas is still selected");
        // Undo puts the member back.
        s.run("edit.undo", &json!({})).unwrap();
        assert_eq!(members(&canvas_in_text(&s)).len(), 2);

        // Picking the canvas again (or anything else) ends the member's selection.
        s.run("select.objects", &json!({"index": 0})).unwrap();
        s.run("select.member", &json!({"index": 0})).unwrap();
        s.run("select.member", &json!({"index": null})).unwrap();
        assert!(selected_member(&s).is_none());
        s.run("select.member", &json!({"index": 0})).unwrap();
        s.run("caret.docEnd", &json!({})).unwrap();
        assert!(selected_member(&s).is_none());
        // Pasted outside the canvas, the copied member is a shape of its own in the text.
        s.run("edit.paste", &json!({})).unwrap();
        let para = s.doc.body.first().and_then(|b| b.as_para()).unwrap();
        assert!(matches!(para.objects.last(), Some(InlineObject::Shape { kind: wordcraft_doc::para::ShapeKind::Ellipse, .. })), "{:?}", para.objects);
    }

    #[test]
    fn typing_or_removing_a_member_never_hits_another_shape_or_the_canvas() {
        let mut s = Session::new(wordcraft_doc::Document::new());
        s.run("insert.canvas", &json!({"width": 400, "height": 300})).unwrap();
        s.run("insert.shape", &json!({"kind": "rectangle", "width": 100, "height": 40})).unwrap();
        s.run("insert.shape", &json!({"kind": "ellipse", "width": 50, "height": 50})).unwrap();
        s.run("select.member", &json!({"index": 1})).unwrap();

        // Typing or pasting would replace the whole canvas: refused, and the member stays picked.
        for (id, v) in [
            ("text.insert", json!({"text": "x"})),
            ("text.newParagraph", json!({})),
            ("edit.paste", json!({"text": "x"})),
            ("edit.pasteText", json!({"text": "x"})),
        ] {
            assert!(matches!(s.run(id, &v), Err(CmdError::Disabled(_))), "{id}");
            assert_eq!(members(&canvas(&s)).len(), 2, "{id}");
        }
        assert_eq!(selected_member(&s).map(|(_, i, _)| i), Some(1));
        // Ctrl+Backspace removes the member alone, like Backspace.
        s.run("text.deleteWordBack", &json!({})).unwrap();
        let sizes: Vec<(f32, f32)> = members(&canvas(&s)).iter().map(|m| (m.2, m.3)).collect();
        assert_eq!(sizes, [(100.0, 40.0)], "the ellipse went, the rectangle stays");
        assert!(selected_member(&s).is_none());

        // Removing an earlier member through `canvas.member` drops the pick: its index would now
        // name the next member.
        s.run("insert.shape", &json!({"kind": "ellipse", "width": 50, "height": 50})).unwrap();
        s.run("insert.shape", &json!({"kind": "rectangle", "width": 20, "height": 20})).unwrap();
        s.run("select.member", &json!({"index": 1})).unwrap();
        s.run("canvas.member", &json!({"index": 0, "delete": true})).unwrap();
        assert!(selected_member(&s).is_none(), "the pick doesn't slide onto the member that moved up");

        // Commands that replace the selection (the whole canvas) are refused too.
        s.run("select.member", &json!({"index": 0})).unwrap();
        for (id, v) in [
            ("insert.table", json!({"rows": 1, "cols": 1})),
            ("table.quick", json!({})),
            ("insert.equation", json!({})),
            ("insert.symbol", json!({"char": "§"})),
            ("insert.field", json!({"instr": "AUTHOR"})),
            ("insert.pageBreak", json!({})),
        ] {
            assert!(s.run(id, &v).is_err(), "{id}");
            assert_eq!(members(&canvas(&s)).len(), 2, "{id}: the canvas and its members stay");
        }
        // Moving the member: on the canvas's page it moves; a drop on another page is refused.
        let frame = s.layout().object(&selected_canvas(&s).unwrap().0, 0).unwrap();
        let before = members(&canvas(&s));
        let other = frame.page + 1;
        assert!(matches!(s.run("arrange.bounds", &json!({"page": other, "x": 100, "y": 100})), Err(CmdError::Disabled(_))));
        assert_eq!(members(&canvas(&s)), before);
        s.run("arrange.bounds", &json!({"page": frame.page, "x": frame.rect.x + 5.0, "y": frame.rect.y + 5.0})).unwrap();
        assert_ne!(members(&canvas(&s)), before);
    }
}
