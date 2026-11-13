use super::select::{all_objects, edit_obj, picked_objects, selected, selected_all, with_obj};
use super::{CmdError, CmdResult, InlineObject, Pos, Rgb, Session, StoryRef, TextBody, Value, json, p};

pub(super) const NO_TEXT_BOX: &str = "select a text box first";

pub(super) fn has_text_box(s: &Session) -> Option<&'static str> {
    if text_box_target(s).is_some() { None } else { Some(NO_TEXT_BOX) }
}

fn is_text_box(s: &Session, id: u32) -> bool {
    s.doc.parts.get(&id).is_some_and(|p| p.kind == wordcraft_doc::PartKind::TextBox)
}

/// The text box the Shape Format › Text commands work on: the selected one, or the one whose
/// text the caret is in. Its U+FFFC's position and its story.
pub fn text_box_target(s: &Session) -> Option<(Pos, u32)> {
    if let Some((pos, InlineObject::Shape { story: Some(id), .. })) = selected(s)
        && is_text_box(s, id)
    {
        return Some((pos, id));
    }
    let StoryRef::Part(id) = s.sel.focus.story else { return None };
    if !is_text_box(s, id) {
        return None;
    }
    let after = s.doc.text_box_anchor(id)?;
    let off = after.off.checked_sub(wordcraft_doc::para::OBJ.len_utf8())?;
    Some((Pos { off, ..after }, id))
}

/// Change the target text box's text direction, alignment or link.
pub(crate) fn set_text_body(s: &mut Session, f: impl Fn(&mut TextBody)) -> CmdResult {
    let (pos, id) = text_box_target(s).ok_or_else(|| CmdError::Disabled(NO_TEXT_BOX.into()))?;
    let part = s.doc.parts.get_mut(&id).ok_or_else(|| CmdError::Failed("text box vanished".into()))?;
    f(&mut part.body);
    let body = part.body;
    // Its paragraph is laid out again.
    edit_obj(s, &pos, |_| {})?;
    Ok(json!({"textBox": id, "body": body}))
}

pub(super) fn can_link(s: &Session) -> Option<&'static str> {
    match text_box_target(s) {
        None => Some(NO_TEXT_BOX),
        Some((_, id)) if s.doc.parts.get(&id).is_some_and(|p| p.body.next.is_some()) => Some("this text box is already linked: break the link first"),
        Some(_) => None,
    }
}

/// `shape.link`: the selected text box's text continues in the target box, which must be an
/// empty text box no other box links to, and not one of those the text already runs through.
pub(super) fn link_text_box(s: &mut Session, v: &Value) -> CmdResult {
    let (pos, from) = text_box_target(s).ok_or_else(|| CmdError::Disabled(NO_TEXT_BOX.into()))?;
    let target = v.get("target").ok_or_else(|| CmdError::Params("`target`: the text box to link to".into()))?;
    let at = match target.as_u64() {
        Some(n) => all_objects(s).into_iter().nth(usize::try_from(n).unwrap_or(usize::MAX)).map(|(p, _)| p),
        None => crate::cmd::parse_pos(target),
    }
    .ok_or_else(|| CmdError::Params("no object there".into()))?;
    let to = match s.doc.para_at(&at).and_then(|q| q.object_at(at.off)) {
        Some(InlineObject::Shape { story: Some(id), .. }) if is_text_box(s, *id) => *id,
        _ => return Err(CmdError::Params("the target isn't a text box".into())),
    };
    if s.doc.parts.get(&from).is_some_and(|p| p.body.next.is_some()) {
        return Err(CmdError::Failed("this text box is already linked: break the link first".into()));
    }
    if to == from || s.doc.text_box_chain(from).is_some_and(|c| c.contains(&to)) {
        return Err(CmdError::Failed("a text box can't link to itself or to a box its text already runs through".into()));
    }
    if s.doc.text_box_chains().iter().any(|c| c.iter().skip(1).any(|id| *id == to)) {
        return Err(CmdError::Failed("another text box already links to that one".into()));
    }
    if !s.doc.plain_text(StoryRef::Part(to)).trim().is_empty() {
        return Err(CmdError::Failed("the text box to link to must be empty".into()));
    }
    if let Some(p) = s.doc.parts.get_mut(&from) {
        p.body.next = Some(to);
    }
    edit_obj(s, &pos, |_| {})?;
    edit_obj(s, &at, |_| {})?;
    Ok(json!({"textBox": from, "next": to}))
}

/// Most shapes one `shape.effects` changes.
const MAX_EFFECT_SHAPES: usize = 10_000;

/// `shape.effects`: set or clear the shadow, glow and soft edges of the selected shapes.
pub(super) fn shape_effects(s: &mut Session, v: &Value) -> CmdResult {
    use wordcraft_doc::effects::{Glow, Shadow, ShapeEffects};
    let color = |o: &Value| p::str(o, "color").and_then(Rgb::parse);
    // Each change is `None` (leave as it is) or `Some(new value)`.
    let shadow: Option<Option<Shadow>> = match v.get("shadow") {
        None => None,
        Some(Value::Null) => Some(None),
        Some(Value::Bool(b)) => Some(b.then(Shadow::default)),
        Some(Value::String(id)) if id == "none" => Some(None),
        Some(Value::String(id)) => Some(Some(Shadow::preset(id).ok_or_else(|| CmdError::Params(format!("unknown shadow preset `{id}`")))?)),
        Some(o @ Value::Object(_)) => {
            let mut sh = match p::str(o, "preset") {
                Some(id) => Shadow::preset(id).ok_or_else(|| CmdError::Params(format!("unknown shadow preset `{id}`")))?,
                None => Shadow::default(),
            };
            sh.color = color(o).unwrap_or(sh.color);
            sh.transparency = p::f32(o, "transparency").unwrap_or(sh.transparency);
            sh.blur = p::f32(o, "blur").unwrap_or(sh.blur);
            sh.distance = p::f32(o, "distance").unwrap_or(sh.distance);
            sh.angle = p::f32(o, "angle").unwrap_or(sh.angle);
            Some(Some(sh.sanitized()))
        }
        Some(_) => return Err(CmdError::Params("`shadow`: a preset name, an object or null".into())),
    };
    let glow: Option<Option<Glow>> = match v.get("glow") {
        None => None,
        Some(Value::Null) => Some(None),
        Some(Value::String(id)) if id == "none" => Some(None),
        Some(n @ Value::Number(_)) => Some(Some(Glow { size: n.as_f64().unwrap_or(0.0) as f32, ..Glow::default() }.sanitized())),
        Some(o @ Value::Object(_)) => {
            let d = Glow::default();
            Some(Some(
                Glow {
                    color: color(o).unwrap_or(d.color),
                    size: p::f32(o, "size").unwrap_or(d.size),
                    transparency: p::f32(o, "transparency").unwrap_or(d.transparency),
                }
                .sanitized(),
            ))
        }
        Some(_) => return Err(CmdError::Params("`glow`: a size in points, an object or null".into())),
    };
    let soft: Option<Option<f32>> = match v.get("softEdge") {
        None => None,
        Some(Value::Null) => Some(None),
        Some(n @ Value::Number(_)) => Some(n.as_f64().map(|x| x as f32)),
        Some(_) => return Err(CmdError::Params("`softEdge`: a radius in points or null".into())),
    };
    // The shapes in the selection and those added to it (Shift+click).
    let mut targets: Vec<Pos> =
        selected_all(s, MAX_EFFECT_SHAPES).into_iter().filter(|(_, o)| matches!(o, InlineObject::Shape { .. })).map(|(pos, _)| pos).collect();
    for pos in picked_objects(s) {
        if !targets.contains(&pos) && matches!(s.doc.para_at(&pos).and_then(|q| q.object_at(pos.off)), Some(InlineObject::Shape { .. })) {
            targets.push(pos);
        }
    }
    if targets.is_empty() {
        return Err(CmdError::Disabled("no shape selected".into()));
    }
    let mut last = ShapeEffects::default();
    for pos in &targets {
        let o = edit_obj(s, pos, |o| {
            if let InlineObject::Shape { effects, float, .. } = o {
                let mut e = *effects;
                if let Some(x) = shadow {
                    e.shadow = x;
                }
                if let Some(x) = glow {
                    e.glow = x;
                }
                if let Some(x) = soft {
                    e.soft_edge = x;
                }
                *effects = e.sanitized();
                // Room around the shape for its shadow and glow, as Word's effect extent.
                float.effect = effects.extent();
            }
        })?;
        if let InlineObject::Shape { effects, .. } = o {
            last = effects;
        }
    }
    Ok(json!({"shapes": targets.len(), "effects": serde_json::to_value(last).unwrap_or(Value::Null)}))
}

/// Largest picture a shape fill takes (as Insert › Pictures).
const MAX_FILL_PICTURE: usize = 200 << 20;

/// Shape Fill › Picture: fill the selected shape with the picture at `path` or in base64 `data`,
/// in place of its colour. Without either, the app is asked to pick one.
pub(crate) fn picture_fill(s: &mut Session, v: &Value) -> CmdResult {
    if !v.is_object() {
        return Err(CmdError::Params("`picture`: {\"path\": string} or {\"data\": base64}".into()));
    }
    if !matches!(selected(s), Some((_, InlineObject::Shape { .. }))) {
        return Err(CmdError::Disabled("select a shape first".into()));
    }
    let bytes = match (v.get("data"), v.get("path")) {
        (Some(d), _) => {
            let d = d.as_str().ok_or_else(|| CmdError::Params("`data`: base64 text".into()))?;
            crate::cmd::insert::base64_decode(d).ok_or_else(|| CmdError::Params("bad base64 data".into()))?
        }
        (None, Some(path)) => {
            let path = path.as_str().ok_or_else(|| CmdError::Params("`path`: text".into()))?;
            read_fill_picture(path)?
        }
        (None, None) => {
            s.ui_requests.push(json!({"open": "shapeFillPicture"}));
            return crate::cmd::sel_result(s);
        }
    };
    if bytes.len() > MAX_FILL_PICTURE {
        return Err(CmdError::Failed("image is larger than 200 MB".into()));
    }
    // Only pictures the renderers can draw (bitmaps and metafiles) are kept.
    let ext = if wordcraft_render::image_size(&bytes).is_some() {
        crate::cmd::insert::image_ext(&bytes)
    } else if let Some((_, _, ext)) = wordcraft_render::metafile_size(&bytes) {
        ext
    } else {
        return Err(CmdError::Failed("not a supported image (PNG, JPEG, GIF, WebP, BMP, WMF, EMF)".into()));
    };
    let key = s.doc.add_media(bytes, ext);
    with_obj(s, |o| {
        if let InlineObject::Shape { fill, extra, .. } = o {
            *fill = None;
            extra.picture = Some(key.clone());
        }
    })
}

/// The picture file at `path`, refused by its size before it is read.
#[cfg(not(target_arch = "wasm32"))]
fn read_fill_picture(path: &str) -> Result<Vec<u8>, CmdError> {
    crate::io::read_file_capped(path, MAX_FILL_PICTURE as u64, "picture").map_err(CmdError::Failed)
}

#[cfg(target_arch = "wasm32")]
fn read_fill_picture(path: &str) -> Result<Vec<u8>, CmdError> {
    crate::io::read_file(path).map_err(CmdError::Failed)
}
