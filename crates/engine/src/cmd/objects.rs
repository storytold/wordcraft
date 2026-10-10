//! Picture Format, Shape Format and Arrange: commands on the selected inline object.
//!
//! The selected object is the first picture/shape inside the selection (or right before the
//! caret). Picture adjustments re-encode the bitmap (originals are kept for Reset Picture).

use serde_json::{Value, json};
use wordcraft_doc::para::{Float, InlineObject, Wrap};
use wordcraft_doc::props::Rgb;
use wordcraft_doc::{Pos, StoryRef};

use super::sel_result;
use crate::{CmdError, CmdResult, CommandSpec, Selection, Session, p};

fn has_picture(s: &Session) -> Option<&'static str> {
    match selected(s) {
        Some((_, InlineObject::Image { .. })) => None,
        _ => Some("select a picture first"),
    }
}
fn has_object(s: &Session) -> Option<&'static str> {
    if selected(s).is_some() { None } else { Some("select a picture or shape first") }
}
fn has_floating(s: &Session) -> Option<&'static str> {
    match selected(s) {
        Some((_, o)) if o.is_floating() => None,
        _ => Some("select a floating picture or shape first"),
    }
}
fn has_group(s: &Session) -> Option<&'static str> {
    match selected(s) {
        Some((_, InlineObject::Group { .. })) => None,
        _ => Some("select a group first"),
    }
}
fn can_group(s: &Session) -> Option<&'static str> {
    if picked_objects(s).len() >= 2 { None } else { Some("select two or more pictures or shapes first (Shift+click)") }
}
fn has_shape(s: &Session) -> Option<&'static str> {
    match selected(s) {
        Some((_, InlineObject::Shape { .. })) => None,
        _ => Some("select a shape first"),
    }
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("picture.size", "Size", "Picture Format › Size", size).params(r#"{"width"?: pt, "height"?: pt, "lockAspect"?: bool, "scale"?: percent}"#).when(has_object),
        CommandSpec::new("picture.crop", "Crop", "Picture Format › Size", |s, v| {
            let c = [p::f32(v, "left"), p::f32(v, "top"), p::f32(v, "right"), p::f32(v, "bottom")].map(|x| x.unwrap_or(0.0).clamp(0.0, 0.45));
            with_obj(s, |o| {
                if let InlineObject::Image { crop, .. } = o {
                    *crop = c;
                }
            })
        })
        .params(r#"{"left"?, "top"?, "right"?, "bottom"? (fractions 0–0.45)}"#)
        .when(has_picture),
        CommandSpec::new("picture.altText", "Alt Text", "Picture Format › Accessibility", |s, v| {
            let t = p::req_str(v, "text")?.to_string();
            with_obj(s, |o| {
                if let InlineObject::Image { alt, .. } = o {
                    *alt = t.clone();
                }
            })
        })
        .params(r#"{"text": string}"#)
        .when(has_picture),
        CommandSpec::new("picture.corrections", "Corrections", "Picture Format › Adjust", |s, v| {
            let b = p::f32(v, "brightness").unwrap_or(0.0).clamp(-100.0, 100.0);
            let c = p::f32(v, "contrast").unwrap_or(0.0).clamp(-100.0, 100.0);
            let sharpen = p::f32(v, "sharpen").unwrap_or(0.0).clamp(-100.0, 100.0);
            adjust(s, move |img| {
                let mut img = img;
                if b != 0.0 {
                    img = image::DynamicImage::ImageRgba8(image::imageops::colorops::brighten(&img, (b * 2.55) as i32));
                }
                if c != 0.0 {
                    img = image::DynamicImage::ImageRgba8(image::imageops::colorops::contrast(&img, c));
                }
                if sharpen > 0.0 {
                    img = img.unsharpen(1.0 + sharpen / 50.0, 2);
                } else if sharpen < 0.0 {
                    img = img.blur(-sharpen / 25.0);
                }
                img
            })
        })
        .params(r#"{"brightness"?: -100..100, "contrast"?: -100..100, "sharpen"?: -100..100}"#)
        .when(has_picture),
        CommandSpec::new("picture.color", "Color", "Picture Format › Adjust", |s, v| {
            let mode = p::str(v, "mode").unwrap_or("grayscale").to_string();
            let sat = p::f32(v, "saturation").unwrap_or(100.0);
            adjust(s, move |img| recolor(img, &mode, sat))
        })
        .params(r#"{"mode": "grayscale|sepia|washout|blackAndWhite|saturation|tint", "saturation"?: 0..400}"#)
        .when(has_picture),
        CommandSpec::new("picture.effects", "Artistic Effects", "Picture Format › Adjust", |s, v| {
            let e = p::str(v, "effect").unwrap_or("blur").to_string();
            adjust(s, move |img| match e.as_str() {
                "blur" => img.blur(3.0),
                "sharpen" => img.unsharpen(2.0, 2),
                "invert" => {
                    let mut i = img;
                    i.invert();
                    i
                }
                "posterize" => {
                    let mut rgba = img.to_rgba8();
                    for px in rgba.pixels_mut() {
                        for c in px.0.iter_mut().take(3) {
                            *c = (*c / 64) * 85;
                        }
                    }
                    image::DynamicImage::ImageRgba8(rgba)
                }
                "pixelate" => {
                    let (w, h) = (img.width().max(1), img.height().max(1));
                    img.resize_exact((w / 12).max(1), (h / 12).max(1), image::imageops::FilterType::Nearest).resize_exact(w, h, image::imageops::FilterType::Nearest)
                }
                _ => img,
            })
        })
        .params(r#"{"effect": "blur|sharpen|invert|posterize|pixelate"}"#)
        .when(has_picture),
        CommandSpec::new("picture.transparency", "Transparency", "Picture Format › Adjust", |s, v| {
            let t = p::f32(v, "percent").unwrap_or(50.0).clamp(0.0, 100.0);
            adjust(s, move |img| {
                let mut rgba = img.to_rgba8();
                for px in rgba.pixels_mut() {
                    px.0[3] = (px.0[3] as f32 * (1.0 - t / 100.0)) as u8;
                }
                image::DynamicImage::ImageRgba8(rgba)
            })
        })
        .params(r#"{"percent": 0..100}"#)
        .when(has_picture),
        CommandSpec::new("picture.removeBackground", "Remove Background", "Picture Format › Adjust", |s, v| {
            let tol = p::f32(v, "tolerance").unwrap_or(30.0).clamp(0.0, 255.0);
            adjust(s, move |img| remove_background(img, tol))
        })
        .params(r#"{"tolerance"?: 0..255}"#)
        .when(has_picture),
        CommandSpec::new("picture.compress", "Compress Pictures", "Picture Format › Adjust", |s, v| {
            let max = p::u64(v, "maxPixels").unwrap_or(1600).clamp(32, 20000) as u32;
            adjust(s, move |img| if img.width().max(img.height()) > max { img.resize(max, max, image::imageops::FilterType::Lanczos3) } else { img })
        })
        .params(r#"{"maxPixels"?: n}"#)
        .when(has_picture),
        CommandSpec::new("picture.reset", "Reset Picture", "Picture Format › Adjust", reset).when(has_picture),
        CommandSpec::new("picture.change", "Change Picture", "Picture Format › Adjust", |s, v| {
            let bytes = if let Some(d) = p::str(v, "data") {
                super::insert::base64_decode(d).ok_or_else(|| CmdError::Params("bad base64".into()))?
            } else if let Some(path) = p::str(v, "path") {
                #[cfg(not(target_arch = "wasm32"))]
                {
                    std::fs::read(path).map_err(|e| CmdError::Failed(format!("{path}: {e}")))?
                }
                #[cfg(target_arch = "wasm32")]
                {
                    let _ = path;
                    return Err(CmdError::Failed("paths aren't available on the web".into()));
                }
            } else {
                return Err(CmdError::Params("`path` or `data` required".into()));
            };
            wordcraft_render::image_size(&bytes).ok_or_else(|| CmdError::Failed("not a supported image".into()))?;
            let key = s.doc.add_media(bytes, "png");
            with_obj(s, |o| {
                if let InlineObject::Image { media, .. } = o {
                    *media = key.clone();
                }
            })
        })
        .params(r#"{"path"?: string, "data"?: base64}"#)
        .when(has_picture),
        CommandSpec::new("picture.style", "Picture Styles", "Picture Format › Picture Styles", |s, v| {
            let style = p::str(v, "style").unwrap_or("simpleFrame").to_string();
            adjust(s, move |img| picture_style(img, &style))
        })
        .params(r#"{"style": "simpleFrame|thickFrame|rounded|softEdge|shadow"}"#)
        .when(has_picture),
        CommandSpec::new("picture.border", "Picture Border", "Picture Format › Picture Styles", |s, v| {
            let c = p::str(v, "color").and_then(Rgb::parse).unwrap_or(Rgb::BLACK);
            let w = p::u64(v, "width").unwrap_or(4).clamp(1, 200) as u32;
            adjust(s, move |img| frame(img, w, [c.0, c.1, c.2, 255]))
        })
        .params(r#"{"color"?: "RRGGBB", "width"?: px}"#)
        .when(has_picture),
        CommandSpec::new("arrange.rotate", "Rotate", "Layout › Arrange", |s, v| {
            let how = p::str(v, "direction").unwrap_or("right").to_string();
            let swap = how == "right" || how == "left";
            let r = adjust(s, move |img| match how.as_str() {
                "left" => img.rotate270(),
                "flipH" => img.fliph(),
                "flipV" => img.flipv(),
                _ => img.rotate90(),
            })?;
            if swap {
                with_obj(s, |o| {
                    if let InlineObject::Image { w, h, .. } = o {
                        std::mem::swap(w, h);
                    }
                })?;
            }
            Ok(r)
        })
        .params(r#"{"direction": "right|left|flipH|flipV"}"#)
        .when(has_picture),
        CommandSpec::new("arrange.wrap", "Wrap Text", "Layout › Arrange", |s, v| {
            let wrap: Wrap = serde_json::from_value(v.get("wrap").cloned().unwrap_or(json!("square"))).map_err(|e| CmdError::Params(e.to_string()))?;
            with_float(s, |f| f.wrap = wrap)
        })
        .params(r#"{"wrap": "inline|square|tight|through|topAndBottom|behindText|inFrontOfText"}"#)
        .when(has_object),
        CommandSpec::new("arrange.position", "Position", "Layout › Arrange", |s, v| {
            let preset = p::str(v, "preset").unwrap_or("middleCenter").to_string();
            let (x, y) = (p::f32(v, "x"), p::f32(v, "y"));
            let sect = super::page::sect(s);
            let size = selected(s).map(|(_, o)| obj_size(&o)).unwrap_or((72.0, 72.0));
            with_float(s, |f| {
                if f.wrap == Wrap::Inline {
                    f.wrap = Wrap::Square;
                }
                f.h_rel = wordcraft_doc::para::Anchor::Margin;
                f.v_rel = wordcraft_doc::para::Anchor::Margin;
                (f.h_align, f.v_align) = (None, None);
                let (tw, th) = (sect.text_width(), sect.text_height());
                let col = if preset.ends_with("Left") { 0.0 } else if preset.ends_with("Right") { tw - size.0 } else { (tw - size.0) / 2.0 };
                let row = if preset.starts_with("top") { 0.0 } else if preset.starts_with("bottom") { th - size.1 } else { (th - size.1) / 2.0 };
                f.x = x.unwrap_or(col);
                f.y = y.unwrap_or(row);
            })
        })
        .params(r#"{"preset"?: "topLeft|topCenter|topRight|middleLeft|middleCenter|middleRight|bottomLeft|bottomCenter|bottomRight", "x"?: pt, "y"?: pt}"#)
        .when(has_object),
        CommandSpec::new("arrange.bringForward", "Bring Forward", "Layout › Arrange", |s, _| with_float(s, |f| f.wrap = Wrap::InFrontOfText)).when(has_object),
        CommandSpec::new("arrange.sendBackward", "Send Backward", "Layout › Arrange", |s, _| with_float(s, |f| f.wrap = Wrap::BehindText)).when(has_object),
        CommandSpec::new("arrange.align", "Align", "Layout › Arrange", |s, v| {
            let h = p::str(v, "value").unwrap_or("center").to_string();
            let tw = super::page::sect(s).text_width();
            let w = selected(s).map(|(_, o)| obj_size(&o).0).unwrap_or(72.0);
            with_float(s, |f| {
                if f.wrap == Wrap::Inline {
                    f.wrap = Wrap::Square;
                }
                f.h_rel = wordcraft_doc::para::Anchor::Margin;
                f.h_align = None;
                f.x = match h.as_str() {
                    "left" => 0.0,
                    "right" => tw - w,
                    _ => (tw - w) / 2.0,
                };
            })
        })
        .params(r#"{"value": "left|center|right"}"#)
        .when(has_object),
        CommandSpec::new("arrange.selectionPane", "Selection Pane", "Layout › Arrange", objects_list).pure(),
        CommandSpec::new("arrange.bounds", "Move or Resize", "Layout › Arrange", bounds)
            .params(r#"{"width"?: pt, "height"?: pt, "x"?: pt, "y"?: pt, "page"?: n}  (x/y: the top-left on page `page` (0-based, default its page); moving an inline object floats it)"#)
            .when(has_object),
        CommandSpec::new("arrange.nudge", "Nudge", "Layout › Arrange", |s, v| {
            let d = |k| p::f32(v, k).unwrap_or(0.0).clamp(-MAX_OFFSET, MAX_OFFSET);
            let (dx, dy) = (d("dx"), d("dy"));
            unalign(s)?;
            with_float(s, |f| {
                f.x = (f.x + dx).clamp(-MAX_OFFSET, MAX_OFFSET);
                f.y = (f.y + dy).clamp(-MAX_OFFSET, MAX_OFFSET);
            })
        })
        .params(r#"{"dx"?: pt, "dy"?: pt}"#)
        .when(has_floating),
        CommandSpec::new("arrange.group", "Group", "Layout › Arrange", group)
            .params(r#"{}  (groups the selected pictures, shapes and text boxes: the selected one plus those added with select.addObject)"#)
            .when(can_group),
        CommandSpec::new("arrange.ungroup", "Ungroup", "Layout › Arrange", ungroup).when(has_group),
        CommandSpec::new("select.addObject", "Add Object to Selection", "Home › Editing › Select", add_object)
            .params(r#"{"pos"?: Pos, "index"?: n, "objects"?: [Pos | n]}  (n: its Selection Pane index; with no object selected yet, the first one becomes the selection)"#)
            .pure(),
        CommandSpec::new("select.objects", "Select Objects", "Home › Editing › Select", |s, v| {
            let n = p::u64(v, "index").unwrap_or(0) as usize;
            let list = all_objects(s);
            let (pos, _) = list.into_iter().nth(n).ok_or_else(|| CmdError::Failed("no such object".into()))?;
            let end = Pos { off: pos.off + wordcraft_doc::para::OBJ.len_utf8(), ..pos.clone() };
            s.sel = Selection { anchor: pos, focus: end };
            sel_result(s)
        })
        .params(r#"{"index"?: n}"#)
        .pure(),
        CommandSpec::new("shape.fill", "Shape Fill", "Shape Format › Shape Styles", |s, v| {
            let c = p::str(v, "color").and_then(Rgb::parse);
            with_obj(s, |o| {
                if let InlineObject::Shape { fill, .. } = o {
                    *fill = c;
                }
            })
        })
        .params(r#"{"color": "RRGGBB" | null}"#)
        .when(has_shape),
        CommandSpec::new("shape.outline", "Shape Outline", "Shape Format › Shape Styles", |s, v| {
            let c = p::str(v, "color").and_then(Rgb::parse);
            let w = p::f32(v, "width");
            with_obj(s, |o| {
                if let InlineObject::Shape { stroke, stroke_width, .. } = o {
                    *stroke = c;
                    if let Some(w) = w {
                        *stroke_width = w.clamp(0.0, 72.0);
                    }
                }
            })
        })
        .params(r#"{"color": "RRGGBB" | null, "width"?: pt}"#)
        .when(has_shape),
        CommandSpec::new("shape.effects", "Shape Effects", "Shape Format › Shape Styles", shape_effects)
            .params(
                r#"{"shadow"?: preset|{"preset"?, "color"?: "RRGGBB", "transparency"?: %, "blur"?: pt, "distance"?: pt, "angle"?: deg}|null, "glow"?: pt|{"color"?: "RRGGBB", "size"?: pt, "transparency"?: %}|null, "softEdge"?: pt|null}  (shadow presets: offsetBottomRight, offsetBottom, offsetBottomLeft, offsetRight, offsetCenter, offsetLeft, offsetTopRight, offsetTop, offsetTopLeft; an omitted key is left as it is; applies to every shape in the selection)"#,
            )
            .when(has_shape),
        CommandSpec::new("shape.change", "Change Shape", "Shape Format › Insert Shapes", |s, v| {
            let kind: wordcraft_doc::para::ShapeKind = serde_json::from_value(v.get("kind").cloned().unwrap_or(json!("rectangle"))).map_err(|e| CmdError::Params(e.to_string()))?;
            with_obj(s, |o| {
                if let InlineObject::Shape { kind: k, .. } = o {
                    *k = kind;
                }
            })
        })
        .params(r#"{"kind": string}"#)
        .when(has_shape),
    ]
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

/// Most shapes one `shape.effects` changes.
const MAX_EFFECT_SHAPES: usize = 10_000;

/// `shape.effects`: set or clear the shadow, glow and soft edges of the selected shapes.
fn shape_effects(s: &mut Session, v: &Value) -> CmdResult {
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

/// Smallest width/height an object can be resized to, points (a text box keeps room for a line).
pub fn min_size(o: &InlineObject) -> f32 {
    if matches!(o, InlineObject::Shape { story: Some(_), .. }) { 18.0 } else { 4.0 }
}

/// Largest object offset or move, points.
const MAX_OFFSET: f32 = 4000.0;

/// Resize and/or move the selected object (dragging its frame or handles).
fn bounds(s: &mut Session, v: &Value) -> CmdResult {
    let (pos, obj) = selected(s).ok_or_else(|| CmdError::Disabled("no picture or shape selected".into()))?;
    let to = match (p::f32(v, "x"), p::f32(v, "y")) {
        (Some(x), Some(y)) => Some((x, y)),
        (None, None) => None,
        _ => return Err(CmdError::Params("`x` and `y` go together".into())),
    };
    let (w0, h0) = obj_size(&obj);
    let min = min_size(&obj);
    let w = p::f32(v, "width").unwrap_or(w0).clamp(min, MAX_OFFSET);
    let h = p::f32(v, "height").unwrap_or(h0).clamp(min, MAX_OFFSET);
    if (w, h) != (w0, h0) {
        with_obj(s, |o| o.set_size(w, h))?;
    }
    let pos = match to {
        Some((x, y)) => move_object(s, pos, p::u64(v, "page"), x, y, (w, h))?,
        None => pos,
    };
    s.sel = Selection { anchor: pos.clone(), focus: Pos { off: pos.off + wordcraft_doc::para::OBJ.len_utf8(), ..pos.clone() } };
    let o = s.doc.para_at(&pos).and_then(|p| p.object_at(pos.off)).cloned();
    Ok(json!({"object": serde_json::to_value(o).unwrap_or(Value::Null), "pos": super::pos_json(&pos)}))
}

/// Put the object at `pos` with its top-left at (x, y) on `page` (default: where it is), as a
/// drag does in Word: it floats (an inline object gets Square wrapping) and anchors to the
/// paragraph under its top edge, positioned relative to that paragraph's column and top. So the
/// text it lands on flows around it, and it moves with that text. On a page where no paragraph
/// starts, it anchors to the nearest line and is positioned on the page. Returns the object's
/// position afterwards.
fn move_object(s: &mut Session, pos: Pos, page: Option<u64>, x: f32, y: f32, (w, h): (f32, f32)) -> Result<Pos, CmdError> {
    use wordcraft_doc::para::Anchor;
    if pos.story != StoryRef::Body || pos.path.depth() > 0 {
        return Err(CmdError::Disabled("only objects in the body text can be moved".into()));
    }
    let layout = s.layout();
    let cur = layout.object(&pos, s.page_hint).ok_or_else(|| CmdError::Failed("the object isn't laid out".into()))?;
    let page = page.map_or(cur.page, |n| usize::try_from(n).unwrap_or(usize::MAX));
    let pg = layout.pages.get(page).ok_or_else(|| CmdError::Params(format!("no page {page}")))?;
    // Keep a corner of it on the page.
    let x = x.max(12.0 - w).min(pg.w - 12.0);
    let y = y.max(12.0 - h).min(pg.h.min(MAX_OFFSET) - 12.0);
    let (anchor, on_page) = match anchor_paragraph(&layout, page, y) {
        Some(path) => (Pos { story: StoryRef::Body, path, off: 0 }, false),
        None => (nearest_line(&layout, page, y).ok_or_else(|| CmdError::Params("no text on that page to anchor to".into()))?, true),
    };
    // Move its character to the anchor (unless it's already in that paragraph).
    let at = if anchor.path == pos.path && !on_page {
        pos
    } else {
        let obj = s.doc.para_at(&pos).and_then(|p| p.object_at(pos.off)).cloned().ok_or_else(|| CmdError::Failed("object vanished".into()))?;
        let len = wordcraft_doc::para::OBJ.len_utf8();
        s.doc.delete_range(&pos, &Pos { off: pos.off + len, ..pos.clone() })?;
        let mut at = anchor;
        if at.path == pos.path && at.off > pos.off {
            at.off = at.off.saturating_sub(len);
        }
        let at = s.doc.clamp(&at);
        s.doc.insert_object(&at, obj, &wordcraft_doc::CharProps::default())?;
        at
    };
    let (h_rel, v_rel) = if on_page { (Anchor::Page, Anchor::Page) } else { (Anchor::Column, Anchor::Paragraph) };
    edit_float(s, &at, |f| {
        if f.wrap == Wrap::Inline {
            f.wrap = Wrap::Square;
        }
        f.h_rel = h_rel;
        f.v_rel = v_rel;
        f.h_align = None;
        f.v_align = None;
        f.x = 0.0;
        f.y = 0.0;
    })?;
    // Offsets from where the anchor puts it (its paragraph may have moved with the edit).
    let origin = if on_page {
        wordcraft_geom::Point::new(0.0, 0.0)
    } else {
        s.touch(); // lay out the edit so far
        let layout = s.layout();
        layout.object(&at, page).ok_or_else(|| CmdError::Failed("the object isn't laid out".into()))?.origin
    };
    edit_float(s, &at, |f| {
        f.x = (x - origin.x).clamp(-MAX_OFFSET, MAX_OFFSET);
        f.y = (y - origin.y).clamp(-MAX_OFFSET, MAX_OFFSET);
    })?;
    Ok(at)
}

/// Turn the selected floating object's alignment (left, centred, …) into an offset from its
/// column / paragraph that keeps it where it is, so it can be moved by an offset.
fn unalign(s: &mut Session) -> Result<(), CmdError> {
    use wordcraft_doc::para::Anchor;
    let (pos, obj) = selected(s).ok_or_else(|| CmdError::Disabled("no picture or shape selected".into()))?;
    let Some((_, _, float)) = obj.frame() else { return Ok(()) };
    if float.h_align.is_none() && float.v_align.is_none() {
        return Ok(());
    }
    let hit = s.layout().object(&pos, s.page_hint).ok_or_else(|| CmdError::Failed("the object isn't laid out".into()))?;
    let (ox, oy) = (hit.rect.x - hit.origin.x, hit.rect.y - hit.origin.y);
    edit_float(s, &pos, |f| {
        if f.h_align.is_some() {
            f.h_align = None;
            f.h_rel = Anchor::Column;
            f.x = ox.clamp(-MAX_OFFSET, MAX_OFFSET);
        }
        if f.v_align.is_some() {
            f.v_align = None;
            f.v_rel = Anchor::Paragraph;
            f.y = oy.clamp(-MAX_OFFSET, MAX_OFFSET);
        }
    })?;
    Ok(())
}

/// The top-level body paragraph under `y` on `page`: the last one starting at or above it (else
/// the first starting on the page).
fn anchor_paragraph(layout: &wordcraft_layout::DocLayout, page: usize, y: f32) -> Option<wordcraft_doc::Path> {
    let starts: Vec<(&wordcraft_doc::Path, f32)> = layout
        .pages
        .get(page)?
        .items
        .iter()
        .filter_map(|it| match it {
            wordcraft_layout::Placed::Lines { story: StoryRef::Body, path, l0: 0, y: top, .. } if path.depth() == 0 => Some((path, *top)),
            _ => None,
        })
        .collect();
    starts.iter().rev().find(|(_, top)| *top <= y + 0.5).or(starts.first()).map(|(p, _)| (*p).clone())
}

/// The start of the top-level body line on `page` nearest to `y`.
fn nearest_line(layout: &wordcraft_layout::DocLayout, page: usize, y: f32) -> Option<Pos> {
    let lines = wordcraft_layout::hit::page_lines(layout.pages.get(page)?, StoryRef::Body);
    let best = lines.iter().filter(|l| l.path.depth() == 0).min_by(|a, b| {
        let d = |l: &wordcraft_layout::hit::LineHit| if y < l.top { l.top - y } else { (y - l.bottom).max(0.0) };
        d(a).total_cmp(&d(b))
    })?;
    let off = best.para.lines.get(best.li)?.start;
    Some(Pos { story: StoryRef::Body, path: best.path.clone(), off })
}

fn obj_size(o: &InlineObject) -> (f32, f32) {
    o.frame().map_or((0.0, 0.0), |(w, h, _)| (w, h))
}

fn with_obj(s: &mut Session, f: impl Fn(&mut InlineObject)) -> CmdResult {
    let (pos, _) = selected(s).ok_or_else(|| CmdError::Disabled("no picture or shape selected".into()))?;
    let o = edit_obj(s, &pos, f)?;
    Ok(json!({"object": serde_json::to_value(o).unwrap_or(Value::Null)}))
}

/// Change the object at `pos`; returns it afterwards.
fn edit_obj(s: &mut Session, pos: &Pos, f: impl Fn(&mut InlineObject)) -> Result<InlineObject, CmdError> {
    let para = s.doc.para_mut(pos.story, &pos.path)?;
    let o = para.object_at_mut(pos.off).ok_or_else(|| CmdError::Failed("object vanished".into()))?;
    f(o);
    let o = o.clone();
    para.touch();
    Ok(o)
}

fn edit_float(s: &mut Session, pos: &Pos, f: impl Fn(&mut Float)) -> Result<InlineObject, CmdError> {
    edit_obj(s, pos, |o| {
        if let Some(float) = o.float_mut() {
            f(float);
        }
    })
}

fn with_float(s: &mut Session, f: impl Fn(&mut Float)) -> CmdResult {
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

fn size(s: &mut Session, v: &Value) -> CmdResult {
    let (_, o) = selected(s).ok_or_else(|| CmdError::Disabled("no object".into()))?;
    let (w0, h0) = obj_size(&o);
    let lock = p::bool(v, "lockAspect").unwrap_or(true);
    let (mut w, mut h) = (w0, h0);
    if let Some(sc) = p::f32(v, "scale") {
        w *= sc / 100.0;
        h *= sc / 100.0;
    }
    match (p::f32(v, "width"), p::f32(v, "height")) {
        (Some(nw), Some(nh)) => {
            w = nw;
            h = nh;
        }
        (Some(nw), None) => {
            if lock && w0 > 0.0 {
                h = h0 * nw / w0;
            }
            w = nw;
        }
        (None, Some(nh)) => {
            if lock && h0 > 0.0 {
                w = w0 * nh / h0;
            }
            h = nh;
        }
        _ => {}
    }
    let (w, h) = (w.clamp(1.0, 4000.0), h.clamp(1.0, 4000.0));
    with_obj(s, |o| match o {
        InlineObject::Image { w: ow, h: oh, .. } | InlineObject::Shape { w: ow, h: oh, .. } => {
            *ow = w;
            *oh = h;
        }
        _ => {}
    })
}

/// Apply an image operation to the selected picture's bitmap (stored as a new PNG).
fn adjust(s: &mut Session, f: impl Fn(image::DynamicImage) -> image::DynamicImage) -> CmdResult {
    let (_, o) = selected(s).ok_or_else(|| CmdError::Disabled("no picture".into()))?;
    let InlineObject::Image { media, .. } = o else { return Err(CmdError::Disabled("not a picture".into())) };
    let bytes = s.doc.media.get(&media).cloned().ok_or_else(|| CmdError::Failed("picture data missing".into()))?;
    let img = image::load_from_memory(&bytes).map_err(|e| CmdError::Failed(format!("can't decode the picture: {e}")))?;
    if (img.width() as u64) * (img.height() as u64) > 80_000_000 {
        return Err(CmdError::Failed("picture is too large to edit".into()));
    }
    let out = f(img);
    let mut png = Vec::new();
    out.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).map_err(|e| CmdError::Failed(e.to_string()))?;
    let key = s.doc.add_media(png, "png");
    let orig = s.originals.get(&media).cloned().unwrap_or(media.clone());
    s.originals.entry(key.clone()).or_insert(orig);
    with_obj(s, |o| {
        if let InlineObject::Image { media, .. } = o {
            *media = key.clone();
        }
    })
}

fn reset(s: &mut Session, _: &Value) -> CmdResult {
    let (_, o) = selected(s).ok_or_else(|| CmdError::Disabled("no picture".into()))?;
    let InlineObject::Image { media, .. } = o else { return Err(CmdError::Disabled("not a picture".into())) };
    let orig = s.originals.get(&media).cloned().unwrap_or(media);
    with_obj(s, |o| {
        if let InlineObject::Image { media, crop, .. } = o {
            *media = orig.clone();
            *crop = [0.0; 4];
        }
    })
}

fn recolor(img: image::DynamicImage, mode: &str, sat: f32) -> image::DynamicImage {
    let mut rgba = img.to_rgba8();
    for px in rgba.pixels_mut() {
        let [r, g, b, a] = px.0;
        let (rf, gf, bf) = (r as f32, g as f32, b as f32);
        let l = 0.299 * rf + 0.587 * gf + 0.114 * bf;
        let out = match mode {
            "grayscale" => [l, l, l],
            "sepia" => [
                (0.393 * rf + 0.769 * gf + 0.189 * bf).min(255.0),
                (0.349 * rf + 0.686 * gf + 0.168 * bf).min(255.0),
                (0.272 * rf + 0.534 * gf + 0.131 * bf).min(255.0),
            ],
            "washout" => [rf * 0.35 + 255.0 * 0.65, gf * 0.35 + 255.0 * 0.65, bf * 0.35 + 255.0 * 0.65],
            "blackAndWhite" => {
                let v = if l > 127.0 { 255.0 } else { 0.0 };
                [v, v, v]
            }
            "tint" => [l * 0.6 + 0.4 * 0x3B as f32, l * 0.6 + 0.4 * 0x5B as f32, l * 0.6 + 0.4 * 0xDB as f32],
            _ => {
                let k = sat / 100.0;
                [l + (rf - l) * k, l + (gf - l) * k, l + (bf - l) * k]
            }
        };
        px.0 = [out[0].clamp(0.0, 255.0) as u8, out[1].clamp(0.0, 255.0) as u8, out[2].clamp(0.0, 255.0) as u8, a];
    }
    image::DynamicImage::ImageRgba8(rgba)
}

/// Flood the border-connected region close to the corner colour with transparency.
fn remove_background(img: image::DynamicImage, tol: f32) -> image::DynamicImage {
    let mut rgba = img.to_rgba8();
    let (w, h) = rgba.dimensions();
    if w == 0 || h == 0 {
        return image::DynamicImage::ImageRgba8(rgba);
    }
    let bg = rgba.get_pixel(0, 0).0;
    let close = |p: [u8; 4]| (0..3).map(|i| (p[i] as f32 - bg[i] as f32).abs()).fold(0.0f32, f32::max) <= tol;
    let mut seen = vec![false; (w as usize) * (h as usize)];
    let mut stack: Vec<(u32, u32)> = Vec::new();
    for x in 0..w {
        stack.push((x, 0));
        stack.push((x, h - 1));
    }
    for y in 0..h {
        stack.push((0, y));
        stack.push((w - 1, y));
    }
    while let Some((x, y)) = stack.pop() {
        let i = (y as usize) * (w as usize) + x as usize;
        if seen.get(i).copied().unwrap_or(true) {
            continue;
        }
        if let Some(sv) = seen.get_mut(i) {
            *sv = true;
        }
        let p = rgba.get_pixel(x, y).0;
        if !close(p) {
            continue;
        }
        rgba.put_pixel(x, y, image::Rgba([p[0], p[1], p[2], 0]));
        if x > 0 {
            stack.push((x - 1, y));
        }
        if x + 1 < w {
            stack.push((x + 1, y));
        }
        if y > 0 {
            stack.push((x, y - 1));
        }
        if y + 1 < h {
            stack.push((x, y + 1));
        }
    }
    image::DynamicImage::ImageRgba8(rgba)
}

fn frame(img: image::DynamicImage, border: u32, color: [u8; 4]) -> image::DynamicImage {
    let (w, h) = (img.width(), img.height());
    let mut out = image::RgbaImage::from_pixel(w + 2 * border, h + 2 * border, image::Rgba(color));
    image::imageops::overlay(&mut out, &img.to_rgba8(), border as i64, border as i64);
    image::DynamicImage::ImageRgba8(out)
}

fn picture_style(img: image::DynamicImage, style: &str) -> image::DynamicImage {
    let m = (img.width().max(img.height()) / 40).max(4);
    match style {
        "thickFrame" => frame(frame(img, m * 2, [255, 255, 255, 255]), m / 2, [40, 40, 40, 255]),
        "rounded" | "softEdge" => {
            let mut rgba = img.to_rgba8();
            let (w, h) = rgba.dimensions();
            let r = (w.min(h) as f32) * if style == "rounded" { 0.12 } else { 0.0 };
            let feather = if style == "softEdge" { (w.min(h) as f32 * 0.08).max(2.0) } else { 1.5 };
            for y in 0..h {
                for x in 0..w {
                    let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
                    let dx = (r - fx).max(fx - (w as f32 - r)).max(0.0);
                    let dy = (r - fy).max(fy - (h as f32 - r)).max(0.0);
                    let edge = (fx.min(w as f32 - fx)).min(fy.min(h as f32 - fy));
                    let inside = if r > 0.0 && dx > 0.0 && dy > 0.0 { r - (dx * dx + dy * dy).sqrt() } else { edge };
                    let k = (inside / feather).clamp(0.0, 1.0);
                    let p = rgba.get_pixel_mut(x, y);
                    p.0[3] = (p.0[3] as f32 * k) as u8;
                }
            }
            image::DynamicImage::ImageRgba8(rgba)
        }
        "shadow" => {
            let (w, h) = (img.width(), img.height());
            let off = m;
            let mut out = image::RgbaImage::from_pixel(w + off * 2, h + off * 2, image::Rgba([0, 0, 0, 0]));
            let shadow = image::RgbaImage::from_pixel(w, h, image::Rgba([0, 0, 0, 90]));
            let shadow = image::DynamicImage::ImageRgba8(shadow);
            let mut padded = image::RgbaImage::from_pixel(w + off * 2, h + off * 2, image::Rgba([0, 0, 0, 0]));
            image::imageops::overlay(&mut padded, &shadow.to_rgba8(), (off + off / 2) as i64, (off + off / 2) as i64);
            let blurred = image::DynamicImage::ImageRgba8(padded).blur(off as f32 / 2.0).to_rgba8();
            image::imageops::overlay(&mut out, &blurred, 0, 0);
            image::imageops::overlay(&mut out, &img.to_rgba8(), 0, 0);
            image::DynamicImage::ImageRgba8(out)
        }
        _ => frame(img, m, [255, 255, 255, 255]),
    }
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
fn object_ref(s: &Session, v: &Value) -> Result<Pos, CmdError> {
    let pos = match v.as_u64() {
        Some(n) => all_objects(s).into_iter().nth(usize::try_from(n).unwrap_or(usize::MAX)).map(|(p, _)| p),
        None => super::parse_pos(v),
    };
    pos.filter(|p| p.story == StoryRef::Body && s.doc.para_at(p).and_then(|q| q.object_at(p.off)).is_some_and(InlineObject::is_drawing))
        .ok_or_else(|| CmdError::Params("no picture or shape there".into()))
}

/// Add objects to the selection (Shift+click): the first one is selected if no object is yet;
/// one already selected is taken out again.
fn add_object(s: &mut Session, v: &Value) -> CmdResult {
    let mut items: Vec<Value> = match v.get("objects").and_then(Value::as_array) {
        Some(a) => a.iter().take(1000).cloned().collect(),
        None => Vec::new(),
    };
    if let Some(p) = v.get("pos") {
        items.push(p.clone());
    }
    if let Some(i) = v.get("index") {
        items.push(i.clone());
    }
    if items.is_empty() {
        return Err(CmdError::Params("`pos`, `index` or `objects` required".into()));
    }
    for it in &items {
        let pos = object_ref(s, it)?;
        match object_selection(s).map(|(p, _)| p) {
            None => {
                s.also_selected.clear();
                s.sel = Selection { anchor: pos.clone(), focus: Pos { off: pos.off + wordcraft_doc::para::OBJ.len_utf8(), ..pos } };
            }
            Some(first) if first == pos => {}
            Some(_) => {
                if let Some(i) = s.also_selected.iter().position(|p| *p == pos) {
                    s.also_selected.remove(i);
                } else {
                    s.also_selected.push(pos);
                }
            }
        }
    }
    Ok(json!({"selected": picked_objects(s).iter().map(super::pos_json).collect::<Vec<_>>()}))
}

/// Group the selected objects (Layout › Arrange › Group): one floating group object, anchored
/// where the first of them was, in the box around them all; they keep their places and sizes.
/// A group among them is merged in (its members join the new group).
fn group(s: &mut Session, _: &Value) -> CmdResult {
    use wordcraft_doc::para::GroupChild;
    let mut picks = picked_objects(s);
    if picks.len() < 2 {
        return Err(CmdError::Disabled("select two or more pictures or shapes first".into()));
    }
    picks.sort();
    let layout = s.layout();
    let mut found = Vec::new();
    for pos in &picks {
        if pos.story != StoryRef::Body || pos.path.depth() > 0 {
            return Err(CmdError::Disabled("only objects in the body text (not in tables) can be grouped".into()));
        }
        let obj = s.doc.para_at(pos).and_then(|p| p.object_at(pos.off)).cloned().ok_or_else(|| CmdError::Failed("object vanished".into()))?;
        if !obj.is_floating() {
            return Err(CmdError::Disabled("objects in line with text can't be grouped: choose a text wrapping for them first".into()));
        }
        let hit = layout.object(pos, s.page_hint).ok_or_else(|| CmdError::Failed("the object isn't laid out".into()))?;
        found.push((obj, hit));
    }
    let page = found.first().map(|(_, h)| h.page).unwrap_or(0);
    if found.iter().any(|(_, h)| h.page != page) {
        return Err(CmdError::Disabled("objects on different pages can't be grouped".into()));
    }
    let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for (_, h) in &found {
        (x0, y0) = (x0.min(h.rect.x), y0.min(h.rect.y));
        (x1, y1) = (x1.max(h.rect.right()), y1.max(h.rect.bottom()));
    }
    let (w, h) = ((x1 - x0).clamp(1.0, MAX_OFFSET), (y1 - y0).clamp(1.0, MAX_OFFSET));
    let mut children = Vec::new();
    let member = |obj: &InlineObject, r: [f32; 4]| {
        let mut obj = obj.clone();
        obj.set_size(r[2], r[3]);
        if let Some(f) = obj.float_mut() {
            *f = Float::default();
        }
        GroupChild { x: r[0] - x0, y: r[1] - y0, obj }
    };
    for (obj, hit) in &found {
        let r = hit.rect;
        match obj {
            InlineObject::Group { .. } => children.extend(obj.group_rects(r.x, r.y, r.w, r.h).into_iter().map(|(cr, c)| member(c, cr))),
            _ => children.push(member(obj, [r.x, r.y, r.w, r.h])),
        }
    }
    let float = found.first().and_then(|(o, _)| o.frame()).map(|(_, _, f)| *f).unwrap_or_default();
    let grouped = InlineObject::Group { w, h, float, ch_w: w, ch_h: h, children };
    // Take the members out, last first so the earlier positions hold, and put the group where
    // the first one was.
    let len = wordcraft_doc::para::OBJ.len_utf8();
    for pos in picks.iter().rev() {
        s.doc.delete_range(pos, &Pos { off: pos.off + len, ..pos.clone() })?;
    }
    let first = picks.first().cloned().ok_or_else(|| CmdError::Failed("nothing to group".into()))?;
    let at = s.doc.clamp(&first);
    s.doc.insert_object(&at, grouped, &wordcraft_doc::CharProps::default())?;
    s.touch();
    let pos = move_object(s, at, Some(page as u64), x0, y0, (w, h))?;
    s.also_selected.clear();
    s.sel = Selection { anchor: pos.clone(), focus: Pos { off: pos.off + len, ..pos.clone() } };
    let o = s.doc.para_at(&pos).and_then(|p| p.object_at(pos.off)).cloned();
    Ok(json!({"object": serde_json::to_value(o).unwrap_or(Value::Null), "pos": super::pos_json(&pos)}))
}

/// Split the selected group back into its pictures and shapes (Layout › Arrange › Ungroup), in
/// the places and sizes the group shows them, anchored where the group was. They stay
/// selected, so Group puts them back together.
fn ungroup(s: &mut Session, _: &Value) -> CmdResult {
    let (pos, obj) = selected(s).ok_or_else(|| CmdError::Disabled("select a group first".into()))?;
    let InlineObject::Group { w, h, float, .. } = &obj else { return Err(CmdError::Disabled("select a group first".into())) };
    // Its offsets, explicit: the members are placed from the same anchor.
    let pos = if float.wrap == Wrap::Inline {
        if pos.story != StoryRef::Body || pos.path.depth() > 0 {
            return Err(CmdError::Disabled("only groups in the body text can be ungrouped in line with text".into()));
        }
        let hit = s.layout().object(&pos, s.page_hint).ok_or_else(|| CmdError::Failed("the object isn't laid out".into()))?;
        move_object(s, pos, Some(hit.page as u64), hit.rect.x, hit.rect.y, (*w, *h))?
    } else {
        unalign(s)?;
        pos
    };
    let obj = s.doc.para_at(&pos).and_then(|p| p.object_at(pos.off)).cloned().ok_or_else(|| CmdError::Failed("object vanished".into()))?;
    let Some((w, h, float)) = obj.frame() else { return Err(CmdError::Failed("object vanished".into())) };
    let float = *float;
    let members: Vec<InlineObject> = obj
        .group_rects(0.0, 0.0, w, h)
        .into_iter()
        .map(|([x, y, cw, ch], c)| {
            let mut c = c.clone();
            c.set_size(cw.max(min_size(&c)), ch.max(min_size(&c)));
            if let Some(f) = c.float_mut() {
                *f = Float { x: (float.x + x).clamp(-MAX_OFFSET, MAX_OFFSET), y: (float.y + y).clamp(-MAX_OFFSET, MAX_OFFSET), ..float };
            }
            c
        })
        .collect();
    if members.is_empty() {
        return Err(CmdError::Failed("the group is empty".into()));
    }
    let len = wordcraft_doc::para::OBJ.len_utf8();
    s.doc.delete_range(&pos, &Pos { off: pos.off + len, ..pos.clone() })?;
    let mut at = s.doc.clamp(&pos);
    let mut placed = Vec::new();
    for m in members {
        placed.push(at.clone());
        at = s.doc.insert_object(&at, m, &wordcraft_doc::CharProps::default())?;
    }
    let first = placed.first().cloned().unwrap_or(pos);
    s.sel = Selection { anchor: first.clone(), focus: Pos { off: first.off + len, ..first } };
    s.also_selected = placed.into_iter().skip(1).collect();
    sel_result(s)
}

fn all_objects(s: &Session) -> Vec<(Pos, InlineObject)> {
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

fn objects_list(s: &mut Session, _: &Value) -> CmdResult {
    Ok(Value::Array(
        all_objects(s)
            .into_iter()
            .enumerate()
            .map(|(i, (pos, o))| {
                let (kind, name) = match &o {
                    InlineObject::Image { alt, .. } => ("picture", if alt.is_empty() { format!("Picture {}", i + 1) } else { alt.clone() }),
                    InlineObject::Shape { kind, .. } => ("shape", format!("{kind:?} {}", i + 1)),
                    InlineObject::Group { .. } => ("group", format!("Group {}", i + 1)),
                    _ => ("object", format!("Object {}", i + 1)),
                };
                json!({"index": i, "kind": kind, "name": name, "pos": super::pos_json(&pos), "size": obj_size(&o)})
            })
            .collect(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(w: u32, h: u32) -> Vec<u8> {
        let img = image::RgbaImage::from_fn(w, h, |x, y| {
            if x > 2 && x < w - 3 && y > 2 && y < h - 3 { image::Rgba([200, 30, 30, 255]) } else { image::Rgba([255, 255, 255, 255]) }
        });
        let mut b = Vec::new();
        image::DynamicImage::ImageRgba8(img).write_to(&mut std::io::Cursor::new(&mut b), image::ImageFormat::Png).unwrap();
        b
    }
    /// The laid-out area of the only picture or shape (not a text box).
    fn object_area(s: &mut Session) -> wordcraft_layout::hit::ObjectHit {
        s.layout().find_object(0, |o| o.text_box.is_none()).unwrap()
    }

    /// What a click and a handle or body drag do in the editor (#142, #82): hit-test the object,
    /// select it, then resize and move it with `arrange.bounds`.
    fn click_resize_and_drag(insert: &str, params: Value) {
        use wordcraft_doc::para::Wrap;
        let mut s = Session::new(wordcraft_doc::Document::new());
        s.run("text.insert", &json!({"text": "Some text "})).unwrap();
        s.run(insert, &params).unwrap();
        assert!(selected(&s).is_some(), "{insert}: a new object is selected");
        s.run("select.collapse", &json!({"end": true})).unwrap();
        s.run("text.insert", &json!({"text": " more text"})).unwrap();
        assert!(selected(&s).is_none(), "{insert}: typing moved off the object");
        // Click: the hit test finds it, and selecting its character selects the object.
        let o = object_area(&mut s);
        let (cx, cy) = (o.rect.x + o.rect.w / 2.0, o.rect.y + o.rect.h / 2.0);
        let hit = s.layout().object_at(o.page, cx, cy, 4.0).unwrap_or_else(|| panic!("{insert}: click misses it"));
        assert_eq!(hit.pos(), o.pos());
        let end = Pos { off: o.off + wordcraft_doc::para::OBJ.len_utf8(), ..o.pos() };
        s.run("select.range", &json!({"anchor": o.pos(), "focus": end})).unwrap();
        assert!(selected(&s).is_some(), "{insert}: not selected");
        // Resize by a handle: stays inline.
        s.run("arrange.bounds", &json!({"width": 160, "height": 90})).unwrap();
        let o = object_area(&mut s);
        assert_eq!((o.rect.w, o.rect.h, o.wrap), (160.0, 90.0, Wrap::Inline), "{insert}");
        // Drag the body: it floats with Square wrap where it was dropped, still selected.
        let r = s.run("arrange.bounds", &json!({"page": 0, "x": 250, "y": 300})).unwrap();
        assert_eq!(r["object"]["float"]["wrap"], "square", "{insert}");
        let o = object_area(&mut s);
        assert_eq!((o.page, o.rect.x, o.rect.y, o.rect.w, o.rect.h), (0, 250.0, 300.0, 160.0, 90.0), "{insert}");
        assert!(selected(&s).is_some());
        // Arrow keys nudge it once it floats.
        s.run("arrange.nudge", &json!({"dx": 6, "dy": -2})).unwrap();
        let n = object_area(&mut s).rect;
        assert!((n.x - 256.0).abs() < 0.01 && (n.y - 298.0).abs() < 0.01, "{insert}: {n:?}");
        // The text around it is untouched, and each change is one undo step.
        assert_eq!(s.doc.plain_text(StoryRef::Body).replace(wordcraft_doc::para::OBJ, ""), "Some text  more text");
        s.run("edit.undo", &json!({})).unwrap();
        assert_eq!((object_area(&mut s).rect.x, object_area(&mut s).rect.y), (250.0, 300.0), "{insert}");
    }

    #[test]
    fn shapes_can_be_clicked_resized_and_dragged() {
        for kind in ["rectangle", "ellipse", "star", "arrow"] {
            click_resize_and_drag("insert.shape", json!({"kind": kind}));
        }
    }

    #[test]
    fn pictures_can_be_clicked_resized_and_dragged() {
        let data = super::super::insert::base64_encode(&png(20, 10));
        click_resize_and_drag("insert.picture", json!({"data": data, "width": 100}));
    }

    #[test]
    fn picture_pipeline() {
        let mut s = Session::new(wordcraft_doc::Document::new());
        let data = super::super::insert::base64_encode(&png(20, 10));
        s.run("insert.picture", &json!({"data": data})).unwrap();
        assert!(selected(&s).is_some());
        s.run("picture.size", &json!({"width": 100})).unwrap();
        let (_, o) = selected(&s).unwrap();
        assert_eq!(obj_size(&o), (100.0, 50.0));
        let before = s.doc.media.len();
        s.run("picture.color", &json!({"mode": "grayscale"})).unwrap();
        s.run("picture.removeBackground", &json!({})).unwrap();
        assert!(s.doc.media.len() > before);
        s.run("arrange.rotate", &json!({"direction": "right"})).unwrap();
        assert_eq!(obj_size(&selected(&s).unwrap().1), (50.0, 100.0));
        s.run("picture.reset", &json!({})).unwrap();
        s.run("arrange.wrap", &json!({"wrap": "square"})).unwrap();
        s.run("arrange.position", &json!({"preset": "topRight"})).unwrap();
        let l = s.run("arrange.selectionPane", &json!({})).unwrap();
        assert_eq!(l.as_array().unwrap().len(), 1);
        let _ = s.layout();
        s.run("select.collapse", &json!({"end": true})).unwrap();
        s.run("text.insert", &json!({"text": "x"})).unwrap();
        assert!(s.run("picture.crop", &json!({"left": 0.1})).is_err());
    }

    /// The page rectangle of the object at `pos`.
    fn rect_of(s: &mut Session, pos: &Pos) -> [f32; 4] {
        let r = s.layout().object(pos, 0).unwrap().rect;
        [r.x, r.y, r.w, r.h]
    }

    fn near(a: [f32; 4], b: [f32; 4]) -> bool {
        a.iter().zip(b).all(|(x, y)| (x - y).abs() < 0.5)
    }

    /// Two floating shapes, Shift+clicked together and grouped: one object that moves as one with
    /// its members in place; ungrouping puts them back where the group showed them, still
    /// selected, so Group regroups them.
    #[test]
    fn group_moves_as_one_and_ungroups() {
        let mut s = Session::new(wordcraft_doc::Document::new());
        s.run("document.setText", &json!({"text": "one\ntwo\nthree\nfour"})).unwrap();
        s.sel = Selection::caret(s.doc.start_of(StoryRef::Body));
        s.run("insert.shape", &json!({"kind": "rectangle", "width": 100, "height": 50})).unwrap();
        s.run("arrange.bounds", &json!({"page": 0, "x": 100, "y": 150})).unwrap();
        s.run("select.collapse", &json!({"end": true})).unwrap();
        s.run("insert.shape", &json!({"kind": "ellipse", "width": 60, "height": 40})).unwrap();
        s.run("arrange.bounds", &json!({"page": 0, "x": 300, "y": 200})).unwrap();
        // One shape selected: nothing to group yet.
        assert!(s.run("arrange.group", &json!({})).is_err());
        let r = s.run("select.addObject", &json!({"index": 0})).unwrap();
        assert_eq!(r["selected"].as_array().unwrap().len(), 2);
        let g = s.run("arrange.group", &json!({})).unwrap();
        let pos: Pos = serde_json::from_value(g["pos"].clone()).unwrap();
        let list = s.run("arrange.selectionPane", &json!({})).unwrap();
        assert_eq!(list.as_array().unwrap().len(), 1, "{list}");
        assert_eq!(list[0]["kind"], "group");
        assert!(s.also_selected.is_empty());
        assert!(near(rect_of(&mut s, &pos), [100.0, 150.0, 260.0, 90.0]), "{:?}", rect_of(&mut s, &pos));

        // Moving the group moves its members with it.
        s.run("arrange.nudge", &json!({"dx": 20, "dy": 10})).unwrap();
        let (pos, obj) = selected(&s).unwrap();
        let [x, y, w, h] = rect_of(&mut s, &pos);
        let members: Vec<[f32; 4]> = obj.group_rects(x, y, w, h).into_iter().map(|(r, _)| r).collect();
        assert!(near(members[0], [120.0, 160.0, 100.0, 50.0]) && near(members[1], [320.0, 210.0, 60.0, 40.0]), "{members:?}");
        // Drawn there too: the page shows both shapes inside the group's frame.
        let shapes = s.layout().pages[0].items.iter().filter(|i| matches!(i, wordcraft_layout::Placed::Shape { .. })).count();
        assert_eq!(shapes, 2);

        // Ungroup: two shapes again, where the group showed them, both still selected.
        s.run("arrange.ungroup", &json!({})).unwrap();
        let list = all_objects(&s);
        assert_eq!(list.len(), 2);
        let rects: Vec<[f32; 4]> = list.iter().map(|(p, _)| rect_of(&mut s, p)).collect();
        assert!(rects.iter().any(|r| near(*r, [120.0, 160.0, 100.0, 50.0])), "{rects:?}");
        assert!(rects.iter().any(|r| near(*r, [320.0, 210.0, 60.0, 40.0])), "{rects:?}");
        assert_eq!(picked_objects(&s).len(), 2);
        s.run("arrange.group", &json!({})).unwrap();
        assert_eq!(all_objects(&s).len(), 1);
        // Undo goes back step by step.
        s.run("edit.undo", &json!({})).unwrap();
        assert_eq!(all_objects(&s).len(), 2);
    }

    #[test]
    fn picture_alt_and_crop_round_trip() {
        let mut s = Session::new(wordcraft_doc::Document::new());
        let data = super::super::insert::base64_encode(&png(20, 10));
        s.run("insert.picture", &json!({"data": data})).unwrap();
        s.run("picture.altText", &json!({"text": "A red box"})).unwrap();
        s.run("picture.crop", &json!({"left": 0.1, "top": 0.2, "right": 0.05, "bottom": 0.0})).unwrap();
        let (_, o) = selected(&s).unwrap();
        let InlineObject::Image { alt, crop, .. } = o else { panic!("expected image") };
        assert_eq!(alt, "A red box");
        assert_eq!(crop, [0.1, 0.2, 0.05, 0.0]);
    }
}
