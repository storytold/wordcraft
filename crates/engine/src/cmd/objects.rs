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
        Some((_, InlineObject::Image { float, .. } | InlineObject::Shape { float, .. })) if float.wrap != Wrap::Inline => None,
        _ => Some("select a floating picture or shape first"),
    }
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
            with_float(s, |f| {
                f.x = (f.x + dx).clamp(-MAX_OFFSET, MAX_OFFSET);
                f.y = (f.y + dy).clamp(-MAX_OFFSET, MAX_OFFSET);
            })
        })
        .params(r#"{"dx"?: pt, "dy"?: pt}"#)
        .when(has_floating),
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

/// The first picture/shape in the selection, or just before a collapsed caret.
pub fn selected(s: &Session) -> Option<(Pos, InlineObject)> {
    let (a, b) = s.sel.ordered();
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
            if let Some(o @ (InlineObject::Image { .. } | InlineObject::Shape { .. })) = p.object_at(off) {
                return Some((Pos { story, path: path.clone(), off }, o.clone()));
            }
        }
    }
    None
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
        with_obj(s, |o| {
            if let InlineObject::Image { w: ow, h: oh, .. } | InlineObject::Shape { w: ow, h: oh, .. } = o {
                *ow = w;
                *oh = h;
            }
        })?;
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
    let cur = layout.object(&pos.path, pos.off, s.page_hint).ok_or_else(|| CmdError::Failed("the object isn't laid out".into()))?;
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
        f.x = 0.0;
        f.y = 0.0;
    })?;
    // Offsets from where the anchor puts it (its paragraph may have moved with the edit).
    let origin = if on_page {
        wordcraft_geom::Point::new(0.0, 0.0)
    } else {
        s.touch(); // lay out the edit so far
        let layout = s.layout();
        layout.object(&at.path, at.off, page).ok_or_else(|| CmdError::Failed("the object isn't laid out".into()))?.origin
    };
    edit_float(s, &at, |f| {
        f.x = (x - origin.x).clamp(-MAX_OFFSET, MAX_OFFSET);
        f.y = (y - origin.y).clamp(-MAX_OFFSET, MAX_OFFSET);
    })?;
    Ok(at)
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
    match o {
        InlineObject::Image { w, h, .. } | InlineObject::Shape { w, h, .. } => (*w, *h),
        _ => (0.0, 0.0),
    }
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
        if let InlineObject::Image { float, .. } | InlineObject::Shape { float, .. } = o {
            f(float);
        }
    })
}

fn with_float(s: &mut Session, f: impl Fn(&mut Float)) -> CmdResult {
    with_obj(s, |o| match o {
        InlineObject::Image { float, .. } | InlineObject::Shape { float, .. } => f(float),
        _ => {}
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

fn all_objects(s: &Session) -> Vec<(Pos, InlineObject)> {
    let mut v = Vec::new();
    for path in s.doc.para_paths(StoryRef::Body) {
        let Some(p) = s.doc.para(StoryRef::Body, &path) else { continue };
        for off in p.object_offsets() {
            if let Some(o @ (InlineObject::Image { .. } | InlineObject::Shape { .. })) = p.object_at(off) {
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
}
