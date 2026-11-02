use super::{CmdError, CmdResult, Float, GraphicKind, InlineObject, Pos, Selection, Session, Spin, StoryRef, Value, Wrap, json, p};

pub(super) fn has_picture(s: &Session) -> Option<&'static str> {
    match selected(s) {
        Some((_, InlineObject::Image { .. })) => None,
        _ => Some("select a picture first"),
    }
}

pub(super) fn has_object(s: &Session) -> Option<&'static str> {
    if selected(s).is_some() { None } else { Some("select a picture or shape first") }
}

pub(super) fn has_floating(s: &Session) -> Option<&'static str> {
    match selected(s) {
        Some((_, InlineObject::Graphic { .. })) => Some(FROZEN),
        Some((_, o)) if o.is_floating() => None,
        _ => Some("select a floating picture or shape first"),
    }
}

/// Stories whose objects can be moved, resized and arranged: the body and headers/footers.
/// Objects in table cells (deeper paths), notes and text boxes stay where their text is.
pub(super) fn movable_story(s: &Session, story: StoryRef) -> bool {
    match story {
        StoryRef::Body => true,
        StoryRef::Part(id) => {
            s.doc.parts.get(&id).is_some_and(|p| matches!(p.kind, wordcraft_doc::PartKind::Header | wordcraft_doc::PartKind::Footer))
        }
    }
}

/// Charts and diagrams are not written back on save yet, so their size, position and wrapping stay
/// as imported: they can be selected and deleted, not moved, resized or arranged.
pub(super) const FROZEN: &str = "charts and diagrams can't be moved or resized yet";

/// `has_movable` for Size: charts and SmartArt WordCraft can edit are laid out again at their
/// new size.
pub(super) fn has_sizable(s: &Session) -> Option<&'static str> {
    match selected(s) {
        Some((_, InlineObject::Graphic { graphic, .. })) if graphic.chart.is_some() || graphic.smart_art.is_some() => None,
        _ => has_movable(s),
    }
}

/// `has_object` for the geometry commands: a chart or diagram is frozen.
pub(super) fn has_movable(s: &Session) -> Option<&'static str> {
    match selected(s) {
        Some((_, InlineObject::Graphic { .. })) => Some(FROZEN),
        _ => has_object(s),
    }
}

/// Word's own ink in a group is kept in the form a group member takes; it can't stand on its own
/// yet, so such a group stays whole.
pub(super) const INK_GROUP: &str = "a group holding Word ink can't be ungrouped yet";

/// Does group `o` hold a member that can't stand on its own (Word's ink)?
pub(super) fn holds_ink(o: &InlineObject) -> bool {
    matches!(o, InlineObject::Group { children, .. } if children.iter().any(|c| matches!(&c.obj, InlineObject::Graphic { graphic, .. } if graphic.kind == GraphicKind::Ink)))
}

pub(super) fn has_group(s: &Session) -> Option<&'static str> {
    match selected(s) {
        Some((_, o)) if o.is_canvas() => Some("a drawing canvas can't be ungrouped"),
        Some((_, o)) if holds_ink(&o) => Some(INK_GROUP),
        Some((_, InlineObject::Group { .. })) => None,
        _ => Some("select a group first"),
    }
}

/// Shape Fill and Shape Outline: a shape, or a Drawing Canvas's background and border.
pub(crate) fn has_shape_or_canvas(s: &Session) -> Option<&'static str> {
    match selected(s) {
        Some((_, o)) if o.is_canvas() => None,
        _ => has_shape(s),
    }
}

pub(super) fn can_group(s: &Session) -> Option<&'static str> {
    if picked_objects(s).len() >= 2 { None } else { Some("select two or more pictures or shapes first (Shift+click)") }
}

pub(super) fn has_shape(s: &Session) -> Option<&'static str> {
    match selected(s) {
        Some((_, InlineObject::Shape { .. })) => None,
        _ => Some("select a shape first"),
    }
}

/// The selection when it is exactly one picture, shape or text box (its U+FFFC), rather than
/// text: what clicking an object selects.
pub fn object_selection(s: &Session) -> Option<(Pos, &InlineObject)> {
    let (a, b) = s.sel.ordered();
    if a.story != b.story || a.path != b.path || b.off != a.off + wordcraft_doc::para::OBJ.len_utf8() {
        return None;
    }
    let o = s.doc.para_at(&a)?.object_at(a.off)?;
    o.is_drawing().then_some((a, o))
}

/// The first picture/shape in the selection, or just before a collapsed caret.
pub fn selected(s: &Session) -> Option<(Pos, InlineObject)> {
    selected_all(s, 1).into_iter().next()
}

/// The pictures and shapes in the selection (at most `max`), or the one just before a collapsed
/// caret.
pub fn selected_all(s: &Session, max: usize) -> Vec<(Pos, InlineObject)> {
    let mut out = Vec::new();
    let (a, b) = s.sel.ordered();
    let max = if a == b { 1 } else { max };
    let story = a.story;
    let paths = if a == b { vec![a.path.clone()] } else { s.doc.paths_between(&a, &b) };
    for path in paths {
        let Some(p) = s.doc.para(story, &path) else { continue };
        for off in p.object_offsets() {
            let inside = if a == b {
                off + wordcraft_doc::para::OBJ.len_utf8() == a.off || off == a.off
            } else {
                (path != a.path || off >= a.off) && (path != b.path || off < b.off)
            };
            if !inside {
                continue;
            }
            if let Some(o) = p.object_at(off).filter(|o| o.is_drawing()) {
                out.push((Pos { story, path: path.clone(), off }, o.clone()));
                if out.len() >= max {
                    return out;
                }
            }
        }
    }
    out
}

/// Smallest width/height an object can be resized to, points (a text box keeps room for a line).
pub fn min_size(o: &InlineObject) -> f32 {
    if matches!(o, InlineObject::Shape { story: Some(_), .. }) { 18.0 } else { 4.0 }
}

/// Select the object whose character is at `pos`.
pub(super) fn select_object(s: &mut Session, pos: &Pos) {
    s.sel = Selection { anchor: pos.clone(), focus: Pos { off: pos.off + wordcraft_doc::para::OBJ.len_utf8(), ..pos.clone() } };
}

pub(super) fn obj_size(o: &InlineObject) -> (f32, f32) {
    o.frame().map_or((0.0, 0.0), |(w, h, _)| (w, h))
}

/// Alt Text: the selected object's description, title and decorative mark, in one undo step.
pub(super) fn alt_text(s: &mut Session, v: &Value) -> CmdResult {
    let (text, title, decorative) = (p::str(v, "text"), p::str(v, "title"), p::bool(v, "decorative"));
    if text.is_none() && title.is_none() && decorative.is_none() {
        return Err(CmdError::Params("`text`, `title` or `decorative` is required".into()));
    }
    with_obj(s, |o| {
        if let Some(t) = text {
            o.set_alt_text(t);
        }
        if let Some(f) = o.float_mut() {
            if let Some(t) = title {
                f.alt.title = wordcraft_doc::para::cap_alt(t);
            }
            if let Some(d) = decorative {
                f.alt.decorative = d;
            }
        }
    })
}

pub(super) fn with_obj(s: &mut Session, f: impl Fn(&mut InlineObject)) -> CmdResult {
    let (pos, _) = selected(s).ok_or_else(|| CmdError::Disabled("no picture or shape selected".into()))?;
    let o = edit_obj(s, &pos, f)?;
    Ok(json!({"object": serde_json::to_value(o).unwrap_or(Value::Null)}))
}

/// Change the object at `pos`; returns it afterwards.
pub(crate) fn edit_obj(s: &mut Session, pos: &Pos, f: impl Fn(&mut InlineObject)) -> Result<InlineObject, CmdError> {
    let para = s.doc.para_mut(pos.story, &pos.path)?;
    let o = para.object_at_mut(pos.off).ok_or_else(|| CmdError::Failed("object vanished".into()))?;
    f(o);
    let o = o.clone();
    para.touch();
    Ok(o)
}

pub(super) fn edit_float(s: &mut Session, pos: &Pos, f: impl Fn(&mut Float)) -> Result<InlineObject, CmdError> {
    edit_obj(s, pos, |o| {
        if let Some(float) = o.float_mut() {
            f(float);
        }
    })
}

pub(super) fn with_float(s: &mut Session, f: impl Fn(&mut Float)) -> CmdResult {
    with_obj(s, |o| {
        if let Some(float) = o.float_mut() {
            let was_inline = float.wrap == Wrap::Inline;
            f(float);
            // Word's distance from text for a newly wrapped object: 0.125" at the sides.
            if was_inline && float.wrap != Wrap::Inline && float.dist == 0.0 {
                float.dist = 9.0;
            }
        }
    })
}

/// The selected pictures, shapes, text boxes and groups: the one the selection holds, then
/// those added to it ([`Session::also_selected`]), each once, in the body.
pub fn picked_objects(s: &Session) -> Vec<Pos> {
    let Some((first, _)) = object_selection(s) else { return Vec::new() };
    let mut out = vec![first];
    for p in &s.also_selected {
        if !out.contains(p) && s.doc.para_at(p).and_then(|q| q.object_at(p.off)).is_some_and(InlineObject::is_drawing) {
            out.push(p.clone());
        }
    }
    out
}

/// The object a `select.addObject` item names: a position, or a Selection Pane index.
pub(super) fn object_ref(s: &Session, v: &Value) -> Result<Pos, CmdError> {
    let pos = match v.as_u64() {
        Some(n) => all_objects(s).into_iter().nth(usize::try_from(n).unwrap_or(usize::MAX)).map(|(p, _)| p),
        None => crate::cmd::parse_pos(v),
    };
    pos.filter(|p| movable_story(s, p.story) && s.doc.para_at(p).and_then(|q| q.object_at(p.off)).is_some_and(InlineObject::is_drawing))
        .ok_or_else(|| CmdError::Params("no picture or shape there".into()))
}

/// A group's members as it shows them in its frame `[x, y, w, h]`: each member's unturned frame
/// and the turn it is drawn with (its own inside the group's). The group's turn, about its
/// centre, moves each member's centre. Empty for anything but a group.
pub(super) fn shown_members(group: &InlineObject, [x, y, w, h]: [f32; 4]) -> Vec<([f32; 4], Spin, &InlineObject)> {
    let outer = group.frame().map(|(_, _, f)| f.spin()).unwrap_or_default();
    let [x, y, w, h] = [x, y, w, h].map(wordcraft_geom::finite);
    let (gx, gy) = (x + w / 2.0, y + h / 2.0);
    group
        .group_rects(x, y, w, h)
        .into_iter()
        .map(|([mx, my, mw, mh], c)| {
            let own = c.frame().map(|(_, _, f)| f.spin()).unwrap_or_default();
            let (cx, cy) = outer.apply(gx, gy, mx + mw / 2.0, my + mh / 2.0);
            ([cx - mw / 2.0, cy - mh / 2.0, mw, mh], own.within(outer), c)
        })
        .collect()
}

pub(crate) fn all_objects(s: &Session) -> Vec<(Pos, InlineObject)> {
    let mut v = Vec::new();
    for path in s.doc.para_paths(StoryRef::Body) {
        let Some(p) = s.doc.para(StoryRef::Body, &path) else { continue };
        for off in p.object_offsets() {
            if let Some(o) = p.object_at(off).filter(|o| o.is_drawing()) {
                v.push((Pos { story: StoryRef::Body, path: path.clone(), off }, o.clone()));
            }
        }
    }
    v
}

pub(super) fn objects_list(s: &mut Session, _: &Value) -> CmdResult {
    Ok(Value::Array(
        all_objects(s)
            .into_iter()
            .enumerate()
            .map(|(i, (pos, o))| {
                let (kind, name) = match &o {
                    InlineObject::Image { alt, .. } => ("picture", if alt.is_empty() { format!("Picture {}", i + 1) } else { alt.clone() }),
                    InlineObject::Shape { kind, .. } => ("shape", format!("{kind:?} {}", i + 1)),
                    InlineObject::Graphic { graphic, .. } => ("graphic", format!("{:?} {}", graphic.kind, i + 1)),
                    InlineObject::Group { .. } => ("group", format!("Group {}", i + 1)),
                    _ => ("object", format!("Object {}", i + 1)),
                };
                let hidden = o.frame().is_some_and(|(_, _, f)| f.hidden);
                json!({"index": i, "kind": kind, "name": name, "pos": crate::cmd::pos_json(&pos), "size": obj_size(&o), "hidden": hidden})
            })
            .collect(),
    ))
}
