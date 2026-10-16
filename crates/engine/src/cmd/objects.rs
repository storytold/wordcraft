//! Picture Format, Shape Format and Arrange: commands on the selected inline object.
//!
//! The selected object is the first picture/shape inside the selection (or right before the
//! caret). Picture adjustments re-encode the bitmap (originals are kept for Reset Picture).

use serde_json::{Value, json};
use wordcraft_doc::para::{Float, InlineObject, Wrap, WrapText};
use wordcraft_doc::props::{Rgb, TextVert, VAlign};
use wordcraft_doc::{Pos, StoryRef, TextBody};
use wordcraft_geom::Spin;

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
        Some((_, InlineObject::Graphic { .. })) => Some(FROZEN),
        Some((_, o)) if o.is_floating() => None,
        _ => Some("select a floating picture or shape first"),
    }
}
/// Stories whose objects can be moved, resized and arranged: the body and headers/footers.
/// Objects in table cells (deeper paths), notes and text boxes stay where their text is.
fn movable_story(s: &Session, story: StoryRef) -> bool {
    match story {
        StoryRef::Body => true,
        StoryRef::Part(id) => {
            s.doc.parts.get(&id).is_some_and(|p| matches!(p.kind, wordcraft_doc::PartKind::Header | wordcraft_doc::PartKind::Footer))
        }
    }
}
/// Charts and diagrams are not written back on save yet, so their size, position and wrapping stay
/// as imported: they can be selected and deleted, not moved, resized or arranged.
const FROZEN: &str = "charts and diagrams can't be moved or resized yet";
/// `has_movable` for Size: charts and SmartArt WordCraft can edit are laid out again at their
/// new size.
fn has_sizable(s: &Session) -> Option<&'static str> {
    match selected(s) {
        Some((_, InlineObject::Graphic { graphic, .. })) if graphic.chart.is_some() || graphic.smart_art.is_some() => None,
        _ => has_movable(s),
    }
}
/// `has_object` for the geometry commands: a chart or diagram is frozen.
fn has_movable(s: &Session) -> Option<&'static str> {
    match selected(s) {
        Some((_, InlineObject::Graphic { .. })) => Some(FROZEN),
        _ => has_object(s),
    }
}
fn has_group(s: &Session) -> Option<&'static str> {
    match selected(s) {
        Some((_, o)) if o.is_canvas() => Some("a drawing canvas can't be ungrouped"),
        Some((_, InlineObject::Group { .. })) => None,
        _ => Some("select a group first"),
    }
}
/// Shape Fill and Shape Outline: a shape, or a Drawing Canvas's background and border.
fn has_shape_or_canvas(s: &Session) -> Option<&'static str> {
    match selected(s) {
        Some((_, o)) if o.is_canvas() => None,
        _ => has_shape(s),
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
        CommandSpec::new("picture.size", "Size", "Picture Format › Size", size).params(r#"{"width"?: pt, "height"?: pt, "lockAspect"?: bool, "scale"?: percent}"#).when(has_sizable),
        CommandSpec::new("picture.crop", "Crop", "Picture Format › Size", crop)
            .params(
                r#"{"left"?, "top"?, "right"?, "bottom"? (fractions of the source 0–0.45; a side left out is uncropped), "width"?: pt, "height"?: pt (the new display size, in the same undo step; a floating picture also moves by what a left/top crop trims, so the visible part stays put)}"#,
            )
            .when(has_picture),
        CommandSpec::new("picture.replaceImage", "Replace Image", "Picture Format › Adjust", replace_image)
            .params(r#"{"data": base64 (the new picture; its size follows the old width)}"#)
            .when(has_picture),
        // Kept for scripts written before every object had alt text: `object.altText` on a picture.
        CommandSpec::new("picture.altText", "Alt Text", "Picture Format › Accessibility", |s, v| {
            p::req_str(v, "text")?;
            alt_text(s, v)
        })
        .params(r#"{"text": string, "title"?: string, "decorative"?: bool}"#)
        .when(has_picture),
        CommandSpec::new("object.altText", "Alt Text", "Shape Format › Accessibility", alt_text)
            .params(r#"{"text"?: string, "title"?: string, "decorative"?: bool} (the selected picture, shape, text box, group or chart)"#)
            .when(has_object),
        CommandSpec::new("picture.corrections", "Corrections", "Picture Format › Adjust", |s, v| {
            let b = p::f32(v, "brightness").unwrap_or(0.0).clamp(-100.0, 100.0);
            let c = p::f32(v, "contrast").unwrap_or(0.0).clamp(-100.0, 100.0);
            let sharpen = p::f32(v, "sharpen").unwrap_or(0.0).clamp(-100.0, 100.0);
            // Absolute from the original (like transparency): re-setting all three to 0
            // restores the original pixels instead of stacking on the last pass.
            let (_, o) = selected(s).ok_or_else(|| CmdError::Disabled("no picture".into()))?;
            let InlineObject::Image { media, .. } = o else { return Err(CmdError::Disabled("not a picture".into())) };
            let base = s.originals.get(&media).cloned().unwrap_or(media.clone());
            adjust_media(s, &base, move |img| {
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
            // Absolute from the original (like transparency): saturation 100 restores it.
            let (_, o) = selected(s).ok_or_else(|| CmdError::Disabled("no picture".into()))?;
            let InlineObject::Image { media, .. } = o else { return Err(CmdError::Disabled("not a picture".into())) };
            let base = s.originals.get(&media).cloned().unwrap_or(media.clone());
            adjust_media(s, &base, move |img| recolor(img, &mode, sat))
        })
        .params(r#"{"mode": "grayscale|sepia|washout|blackAndWhite|saturation|tint", "saturation"?: 0..400}"#)
        .when(has_picture),
        CommandSpec::new("picture.iconColor", "Icon Color", "Picture Format › Adjust", |s, v| {
            let c = p::str(v, "color").and_then(Rgb::parse).ok_or_else(|| CmdError::Params("`color` (RRGGBB) is required".into()))?;
            // One colour everywhere, transparency kept: recolours an inserted icon or other
            // one-colour line art without redrawing it.
            adjust(s, move |img| {
                let mut rgba = img.to_rgba8();
                for px in rgba.pixels_mut() {
                    px.0 = [c.0, c.1, c.2, px.0[3]];
                }
                image::DynamicImage::ImageRgba8(rgba)
            })
        })
        .params(r#"{"color": "RRGGBB"}"#)
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
            // Absolute, not compounding: 0% is always the original pixels fully opaque.
            // The base is the pre-edit original (kept for Reset Picture), so dragging a
            // slider back and forth never fades what a previous pass already faded.
            // (Combining with other pixel edits then re-setting transparency drops those
            // edits; a full live-effect stack is future work.)
            let (_, o) = selected(s).ok_or_else(|| CmdError::Disabled("no picture".into()))?;
            let InlineObject::Image { media, .. } = o else { return Err(CmdError::Disabled("not a picture".into())) };
            let base = s.originals.get(&media).cloned().unwrap_or(media.clone());
            adjust_media(s, &base, move |img| {
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
                crate::io::read_file(path).map_err(CmdError::Failed)?
            } else {
                return Err(CmdError::Params("`path` or `data` required".into()));
            };
            wordcraft_render::image_size(&bytes).ok_or_else(|| CmdError::Failed("not a supported image".into()))?;
            let key = s.doc.add_media(bytes.clone(), super::insert::image_ext(&bytes));
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
            // Turns and flips are the object's angle and mirroring (DrawingML `a:xfrm`): the
            // picture's pixels are never rewritten. A flip mirrors what is on the page, so a
            // rotated object's angle reverses too.
            let by = match p::str(v, "direction").unwrap_or("right") {
                "left" => Spin::new(270.0, false, false),
                "flipHorizontal" | "flipH" => Spin::new(0.0, true, false),
                "flipVertical" | "flipV" => Spin::new(0.0, false, true),
                "right" => Spin::new(90.0, false, false),
                other => return Err(CmdError::Params(format!("unknown direction `{other}`"))),
            };
            with_float(s, |f| f.set_spin(f.spin().within(by)))
        })
        .params(r#"{"direction": "right|left|flipVertical|flipHorizontal"}  (turns 90° or mirrors the selected picture, shape or text box; its pixels are kept)"#)
        .when(has_movable),
        CommandSpec::new("arrange.rotation", "Rotation", "Layout › Arrange", |s, v| {
            let deg = p::req_f32(v, "degrees")?;
            let (fh, fv) = (p::bool(v, "flipH"), p::bool(v, "flipV"));
            with_float(s, |f| {
                let cur = f.spin();
                f.set_spin(Spin::new(deg, fh.unwrap_or(cur.flip_h), fv.unwrap_or(cur.flip_v)));
            })
        })
        .params(r#"{"degrees": number, "flipH"?: bool, "flipV"?: bool}  (the exact angle, clockwise, of the selected picture, shape or text box; normalised to 0–360)"#)
        .when(has_movable),
        CommandSpec::new("arrange.wrap", "Wrap Text", "Layout › Arrange", |s, v| {
            let wrap: Wrap = serde_json::from_value(v.get("wrap").cloned().unwrap_or(json!("square"))).map_err(|e| CmdError::Params(e.to_string()))?;
            with_float(s, |f| f.wrap = wrap)
        })
        .params(r#"{"wrap": "inline|square|tight|through|topAndBottom|behindText|inFrontOfText"}"#)
        .when(has_movable),
        CommandSpec::new("arrange.wrapText", "Wrap Text Sides", "Layout › Arrange", |s, v| {
            let side = p::str(v, "side").unwrap_or("bothSides");
            let side = WrapText::from_ooxml(side).ok_or_else(|| CmdError::Params(format!("unknown side {side:?}")))?;
            with_float(s, |f| f.wrap_text = side)
        })
        .params(r#"{"side": "bothSides|left|right|largest"}  (the sides of the selected Square, Tight or Through object text flows on; largest: the wider side, line by line)"#)
        .when(has_movable),
        CommandSpec::new("arrange.moveWithText", "Move with Text", "Layout › Arrange › Wrap Text", |s, v| move_with_text(s, p::bool(v, "value").unwrap_or(true)))
            .params(r#"{"value": bool}  (true: Move with Text, the floating object's vertical position follows its paragraph; false: Fix Position on Page. It stays where it is)"#)
            .when(has_floating),
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
        .when(has_movable),
        CommandSpec::new("arrange.bringForward", "Bring Forward", "Layout › Arrange", |s, _| with_float(s, |f| f.wrap = Wrap::InFrontOfText)).when(has_movable),
        CommandSpec::new("arrange.sendBackward", "Send Backward", "Layout › Arrange", |s, _| with_float(s, |f| f.wrap = Wrap::BehindText)).when(has_movable),
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
        .when(has_movable),
        CommandSpec::new("arrange.selectionPane", "Selection Pane", "Layout › Arrange", objects_list).pure(),
        CommandSpec::new("arrange.bounds", "Move or Resize", "Layout › Arrange", bounds)
            .params(r#"{"width"?: pt, "height"?: pt, "x"?: pt, "y"?: pt, "page"?: n}  (x/y: the top-left on page `page` (0-based, default its page); moving an inline object floats it)"#)
            .when(has_movable),
        CommandSpec::new("arrange.copyTo", "Copy to", "Layout › Arrange", copy_to)
            .params(r#"{"x": pt, "y": pt, "page"?: n}  (Ctrl+drag: a copy of the selected object lands with its top-left there and is selected; the original and the clipboard stay as they were)"#)
            .when(has_movable),
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
                } else if let Some(style) = super::canvas::style_mut(o) {
                    style.fill = c;
                }
            })
        })
        .params(r#"{"color": "RRGGBB" | null}  (a selected drawing canvas: its background)"#)
        .when(has_shape_or_canvas),
        CommandSpec::new("shape.outline", "Shape Outline", "Shape Format › Shape Styles", |s, v| {
            let c = p::str(v, "color").and_then(Rgb::parse);
            let w = p::f32(v, "width").map(|w| w.clamp(0.0, 72.0));
            with_obj(s, |o| {
                if let InlineObject::Shape { stroke, stroke_width, .. } = o {
                    *stroke = c;
                    if let Some(w) = w {
                        *stroke_width = w;
                    }
                } else if let Some(style) = super::canvas::style_mut(o) {
                    style.stroke = c;
                    if let Some(w) = w {
                        style.stroke_width = w;
                    }
                }
            })
        })
        .params(r#"{"color": "RRGGBB" | null, "width"?: pt}  (a selected drawing canvas: its border)"#)
        .when(has_shape_or_canvas),
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
        CommandSpec::new("shape.textDirection", "Text Direction", "Shape Format › Text", |s, v| {
            let dir = p::str(v, "dir").unwrap_or("horz");
            let vert = match dir {
                "horz" | "horizontal" => TextVert::Horz,
                "vert" | "rotate90" => TextVert::Vert,
                "vert270" | "rotate270" => TextVert::Vert270,
                "wordArtVert" | "stacked" => TextVert::Stacked,
                other => return Err(CmdError::Params(format!("unknown text direction `{other}`"))),
            };
            set_text_body(s, |b| b.vert = vert)
        })
        .params(r#"{"dir": "horz|vert|vert270|wordArtVert"}  (horizontal, rotate all text 90°, 270°, stacked; the selected text box or the one the caret is in)"#)
        .when(has_text_box),
        CommandSpec::new("shape.alignText", "Align Text", "Shape Format › Text", |s, v| {
            let anchor = match p::str(v, "anchor").unwrap_or("t") {
                "t" | "top" => VAlign::Top,
                "ctr" | "middle" | "center" => VAlign::Center,
                "b" | "bottom" => VAlign::Bottom,
                other => return Err(CmdError::Params(format!("unknown text alignment `{other}`"))),
            };
            set_text_body(s, |b| b.anchor = anchor)
        })
        .params(r#"{"anchor": "t|ctr|b"}  (top, middle, bottom of the box: for turned text, from the side its lines start on)"#)
        .when(has_text_box),
        CommandSpec::new("shape.link", "Create Link", "Shape Format › Text", link_text_box)
            .params(r#"{"target": Pos | n}  (the empty text box the selected box's text continues in: its position, or its Selection Pane index)"#)
            .when(can_link),
        CommandSpec::new("shape.breakLink", "Break Link", "Shape Format › Text", |s, _| {
            let (pos, id) = text_box_target(s).ok_or_else(|| CmdError::Disabled(NO_TEXT_BOX.into()))?;
            let part = s.doc.parts.get_mut(&id).ok_or_else(|| CmdError::Failed("text box vanished".into()))?;
            if part.body.next.take().is_none() {
                return Err(CmdError::Disabled("this text box isn't linked to another".into()));
            }
            edit_obj(s, &pos, |_| {})?;
            Ok(json!({"textBox": id}))
        })
        .params(r#"{}  (the text stops at the end of the selected box; the boxes after it go back to their own text)"#)
        .when(|s| match text_box_target(s) {
            Some((_, id)) if s.doc.parts.get(&id).is_some_and(|p| p.body.next.is_some()) => None,
            _ => Some("select a text box linked to another first"),
        }),
    ]
}

const NO_TEXT_BOX: &str = "select a text box first";

fn has_text_box(s: &Session) -> Option<&'static str> {
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

fn can_link(s: &Session) -> Option<&'static str> {
    match text_box_target(s) {
        None => Some(NO_TEXT_BOX),
        Some((_, id)) if s.doc.parts.get(&id).is_some_and(|p| p.body.next.is_some()) => Some("this text box is already linked: break the link first"),
        Some(_) => None,
    }
}

/// `shape.link`: the selected text box's text continues in the target box, which must be an
/// empty text box no other box links to, and not one of those the text already runs through.
fn link_text_box(s: &mut Session, v: &Value) -> CmdResult {
    let (pos, from) = text_box_target(s).ok_or_else(|| CmdError::Disabled(NO_TEXT_BOX.into()))?;
    let target = v.get("target").ok_or_else(|| CmdError::Params("`target`: the text box to link to".into()))?;
    let at = match target.as_u64() {
        Some(n) => all_objects(s).into_iter().nth(usize::try_from(n).unwrap_or(usize::MAX)).map(|(p, _)| p),
        None => super::parse_pos(target),
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

/// Ctrl+drag: copy the selected object (a text box gets its own story) and move the copy to
/// (x, y), leaving the original in place and the clipboard alone. The copy is selected.
fn copy_to(s: &mut Session, v: &Value) -> CmdResult {
    let (pos, _) = selected(s).ok_or_else(|| CmdError::Disabled("no picture or shape selected".into()))?;
    if p::f32(v, "x").is_none() || p::f32(v, "y").is_none() {
        return Err(CmdError::Params("`x` and `y` are required".into()));
    }
    let len = wordcraft_doc::para::OBJ.len_utf8();
    let end = Pos { off: pos.off + len, ..pos.clone() };
    let frag = s.doc.copy_range(&pos, &end);
    s.sel = Selection::caret(end.clone());
    super::paste::insert(s, frag)?;
    s.touch(); // lay out the copy before moving it
    // The copy sits right after the original; select it and move it like a drag would.
    s.sel = Selection { anchor: end.clone(), focus: Pos { off: end.off + len, ..end } };
    bounds(s, v)
}

/// Put the object at `pos` with its top-left at (x, y) on `page` (default: where it is), as a
/// drag does in Word: it floats (an inline object gets Square wrapping) and anchors to the
/// paragraph under its top edge, positioned relative to that paragraph's column and top. So the
/// text it lands on flows around it, and it moves with that text. On a page where no paragraph
/// starts, it anchors to the nearest line and is positioned on the page. Returns the object's
/// position afterwards.
fn move_object(s: &mut Session, pos: Pos, page: Option<u64>, x: f32, y: f32, (w, h): (f32, f32)) -> Result<Pos, CmdError> {
    use wordcraft_doc::para::Anchor;
    if pos.path.depth() > 0 || !movable_story(s, pos.story) {
        return Err(CmdError::Disabled("only objects in the body text, headers and footers can be moved".into()));
    }
    let story = pos.story;
    let layout = s.layout();
    let cur = layout.object(&pos, s.page_hint).ok_or_else(|| CmdError::Failed("the object isn't laid out".into()))?;
    let page = page.map_or(cur.page, |n| usize::try_from(n).unwrap_or(usize::MAX));
    let pg = layout.pages.get(page).ok_or_else(|| CmdError::Params(format!("no page {page}")))?;
    // Keep a corner of it on the page.
    let x = x.max(12.0 - w).min(pg.w - 12.0);
    let y = y.max(12.0 - h).min(pg.h.min(MAX_OFFSET) - 12.0);
    // The paragraph under the middle of its top edge: in the column it mostly lands in.
    let (anchor, on_page) = match anchor_paragraph(&s.doc, &layout, page, (x + w / 2.0, y), story) {
        Some(path) => (Pos { story, path, off: 0 }, false),
        None => (nearest_line(&layout, page, y, story).ok_or_else(|| CmdError::Params("no text on that page to anchor to".into()))?, true),
    };
    // Move its character to the anchor (unless it's already in that paragraph).
    let at = if anchor.story == pos.story && anchor.path == pos.path && !on_page {
        pos
    } else {
        let obj = s.doc.para_at(&pos).and_then(|p| p.object_at(pos.off)).cloned().ok_or_else(|| CmdError::Failed("object vanished".into()))?;
        let len = wordcraft_doc::para::OBJ.len_utf8();
        s.doc.delete_range(&pos, &Pos { off: pos.off + len, ..pos.clone() })?;
        let mut at = anchor;
        if at.story == pos.story && at.path == pos.path && at.off > pos.off {
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

/// Whether a floating object moves with its text (Word's "Move with Text"): its vertical position
/// follows its paragraph (as layout places any anchor but the page and its margins), rather than
/// being fixed on the page ("Fix Position on Page").
pub fn moves_with_text(f: &Float) -> bool {
    use wordcraft_doc::para::Anchor;
    !matches!(f.v_rel, Anchor::Page | Anchor::Margin | Anchor::TopMargin | Anchor::BottomMargin | Anchor::InsideMargin | Anchor::OutsideMargin)
}

/// Arrange › Wrap Text › Move with Text / Fix Position on Page: anchor the selected floating
/// object's vertical position to its paragraph or to the page, keeping it where it is now.
fn move_with_text(s: &mut Session, with_text: bool) -> CmdResult {
    use wordcraft_doc::para::Anchor;
    let (pos, obj) = selected(s).ok_or_else(|| CmdError::Disabled("no picture or shape selected".into()))?;
    let Some((_, _, float)) = obj.frame() else { return Err(CmdError::Disabled("select a floating picture or shape first".into())) };
    if moves_with_text(float) == with_text {
        return Ok(json!({"object": serde_json::to_value(&obj).unwrap_or(Value::Null)}));
    }
    let hit = s.layout().object(&pos, s.page_hint).ok_or_else(|| CmdError::Failed("the object isn't laid out".into()))?;
    // Paragraph offsets start at the paragraph's top (the hit's origin), page offsets at the page's.
    let y = if with_text { hit.rect.y - hit.origin.y } else { hit.rect.y };
    let o = edit_float(s, &pos, |f| {
        f.v_rel = if with_text { Anchor::Paragraph } else { Anchor::Page };
        f.v_align = None;
        f.y = y.clamp(-MAX_OFFSET, MAX_OFFSET);
    })?;
    Ok(json!({"object": serde_json::to_value(o).unwrap_or(Value::Null)}))
}

/// The top-level paragraph of `story` under `(x, y)` on `page`: in the column under `x` (from
/// the section's column geometry), the last one starting at or above `y` (else the first
/// starting in that column; else, with no paragraph starting in that column, the last starting
/// at or above `y` anywhere on the page, else the first). Body paragraphs come from the page's
/// items; header/footer ones from its header and footer. The column matters (#884): layout
/// places a floating object when it lays out its anchor, so text laid out before that (an
/// earlier column) can't wrap around it. (Nor do lines continuing a paragraph from the column
/// before: layout breaks them once, in the column the paragraph starts in.)
pub(crate) fn anchor_paragraph(
    doc: &wordcraft_doc::Document,
    layout: &wordcraft_layout::DocLayout,
    page: usize,
    (x, y): (f32, f32),
    story: StoryRef,
) -> Option<wordcraft_doc::Path> {
    let pg = layout.pages.get(page)?;
    let items: Box<dyn Iterator<Item = &wordcraft_layout::Placed> + '_> = match story {
        StoryRef::Body => Box::new(pg.items.iter()),
        StoryRef::Part(_) => Box::new(pg.header.iter().chain(pg.footer.iter())),
    };
    // Top-level text on the page: (path, left, top, starts the paragraph).
    let pieces: Vec<(&wordcraft_doc::Path, f32, f32, bool)> = items
        .filter_map(|it| match it {
            wordcraft_layout::Placed::Lines { story: s, path, l0, x, y: top, .. } if *s == story && path.depth() == 0 => {
                Some((path, *x, *top, *l0 == 0))
            }
            _ => None,
        })
        .collect();
    let starts: Vec<(&wordcraft_doc::Path, f32)> = pieces.iter().filter(|p| p.3).map(|p| (p.0, p.2)).collect();
    // The page's columns (left, right), from its section's column geometry (headers and footers
    // span the text area); the column under `x` is the rightmost one starting at or left of it.
    let boxes = match story {
        StoryRef::Body => doc.sections().get(pg.section).map(|(_, sp)| sp.column_boxes()).unwrap_or_default(),
        StoryRef::Part(_) => Vec::new(),
    };
    let mut cols: Vec<(f32, f32)> =
        boxes.iter().map(|(cx, w)| (pg.body.x + cx, pg.body.x + cx + w)).filter(|c| c.0.is_finite() && c.1.is_finite()).collect();
    cols.sort_by(|a, b| a.0.total_cmp(&b.0));
    let col_of = |px: f32| cols.iter().rposition(|c| c.0 <= px + 0.5).unwrap_or(0);
    let col = col_of(x);
    let in_col: Vec<(&wordcraft_doc::Path, f32)> = pieces.iter().filter(|p| p.3 && col_of(p.1) == col).map(|p| (p.0, p.2)).collect();
    let pick = |v: &[(&wordcraft_doc::Path, f32)]| v.iter().rev().find(|(_, top)| *top <= y + 0.5).or(v.first()).map(|(p, _)| (*p).clone());
    pick(&in_col).or_else(|| pick(&starts))
}

/// The start of the top-level line of `story` on `page` nearest to `y`.
pub(crate) fn nearest_line(layout: &wordcraft_layout::DocLayout, page: usize, y: f32, story: StoryRef) -> Option<Pos> {
    let lines = wordcraft_layout::hit::page_lines(layout.pages.get(page)?, story);
    let best = lines.iter().filter(|l| l.path.depth() == 0).min_by(|a, b| {
        let d = |l: &wordcraft_layout::hit::LineHit| if y < l.top { l.top - y } else { (y - l.bottom).max(0.0) };
        d(a).total_cmp(&d(b))
    })?;
    let off = best.para.lines.get(best.li)?.start;
    Some(Pos { story, path: best.path.clone(), off })
}

fn obj_size(o: &InlineObject) -> (f32, f32) {
    o.frame().map_or((0.0, 0.0), |(w, h, _)| (w, h))
}

/// Alt Text: the selected object's description, title and decorative mark, in one undo step.
fn alt_text(s: &mut Session, v: &Value) -> CmdResult {
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

/// `picture.crop`: the crop, and optionally the new display size, as one edit. With a size, a
/// floating picture positioned by offsets moves by what the crop on its left/top side on the page
/// trims (or restores) — for a flipped picture that's the source's right/bottom — so what stays
/// visible keeps its place on the page: a crop handle drop is one command.
fn crop(s: &mut Session, v: &Value) -> CmdResult {
    let c = [p::f32(v, "left"), p::f32(v, "top"), p::f32(v, "right"), p::f32(v, "bottom")].map(|x| x.unwrap_or(0.0).clamp(0.0, 0.45));
    let size = |k: &str| p::f32(v, k).map(|x| x.clamp(1.0, 4000.0));
    let (nw, nh) = (size("width"), size("height"));
    with_obj(s, |o| {
        if let InlineObject::Image { crop, w, h, float, .. } = o {
            // The source's full size at the new display size (the visible part is 1-lo-hi of it).
            let full = |len: f32, lo: f32, hi: f32| len / (1.0 - lo - hi).max(0.1);
            // Flipped, the source's right (bottom) edge is the one on the page's left (top).
            let (left, top) = (if float.flip_h { 2 } else { 0 }, if float.flip_v { 3 } else { 1 });
            if let Some(nw) = nw {
                if float.wrap != Wrap::Inline && float.h_align.is_none() {
                    float.x += (c[left] - crop[left]) * full(nw, c[0], c[2]);
                }
                *w = nw;
            }
            if let Some(nh) = nh {
                if float.wrap != Wrap::Inline && float.v_align.is_none() {
                    float.y += (c[top] - crop[top]) * full(nh, c[1], c[3]);
                }
                *h = nh;
            }
            *crop = c;
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
    // Any drawing: a group's members scale with its frame (its child space is kept); a chart or
    // SmartArt graphic lays itself out again at the new size.
    let theme = s.doc.settings.theme_colors.clone();
    with_obj(s, |o| {
        o.set_size(w, h);
        super::chart::redraw(o, &theme);
        super::smart_art::redraw(o, &theme);
    })
}

/// Apply an image operation to the selected picture's bitmap (stored as a new PNG).
fn adjust(s: &mut Session, f: impl Fn(image::DynamicImage) -> image::DynamicImage) -> CmdResult {
    let (_, o) = selected(s).ok_or_else(|| CmdError::Disabled("no picture".into()))?;
    let InlineObject::Image { media, .. } = o else { return Err(CmdError::Disabled("not a picture".into())) };
    adjust_media(s, &media, f)
}

/// Like [`adjust`], but decodes `base` (a media key, usually the pre-edit original) instead
/// of the picture's current bitmap. The result still chains onto the originals map, so Reset
/// Picture keeps working.
fn adjust_media(s: &mut Session, base: &str, f: impl Fn(image::DynamicImage) -> image::DynamicImage) -> CmdResult {
    let bytes = s.doc.media.get(base).cloned().ok_or_else(|| CmdError::Failed("picture data missing".into()))?;
    let img = image::load_from_memory(&bytes).map_err(|e| CmdError::Failed(format!("can't decode the picture: {e}")))?;
    if (img.width() as u64) * (img.height() as u64) > 80_000_000 {
        return Err(CmdError::Failed("picture is too large to edit".into()));
    }
    let out = f(img);
    let mut png = Vec::new();
    out.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).map_err(|e| CmdError::Failed(e.to_string()))?;
    let key = s.doc.add_media(png, "png");
    // Chain onto the ultimate original (the base's, or the current picture's when the base
    // is itself derived), so Reset Picture always finds it.
    let cur = selected(s).and_then(|(_, o)| match o {
        InlineObject::Image { media, .. } => Some(media),
        _ => None,
    });
    let orig = s
        .originals
        .get(base)
        .cloned()
        .or_else(|| cur.as_ref().and_then(|m| s.originals.get(m).cloned()))
        .unwrap_or_else(|| cur.clone().unwrap_or_else(|| key.clone()));
    s.originals.entry(key.clone()).or_insert(orig);
    with_obj(s, |o| {
        if let InlineObject::Image { media, .. } = o {
            *media = key.clone();
        }
    })
}

/// The encoded bytes of the selected picture.
pub fn selected_picture_bytes(s: &Session) -> Option<std::sync::Arc<Vec<u8>>> {
    let (_, o) = selected(s)?;
    let InlineObject::Image { media, .. } = o else { return None };
    s.doc.media.get(&media).cloned()
}

/// `picture.replaceImage`: swaps the selected picture's image for `data`, keeping its width.
fn replace_image(s: &mut Session, v: &Value) -> CmdResult {
    let d = p::req_str(v, "data")?;
    let bytes = crate::cmd::insert::base64_decode(d).ok_or_else(|| CmdError::Params("bad base64 data".into()))?;
    let (pw, ph) = wordcraft_render::image_size(&bytes).ok_or_else(|| CmdError::Failed("not a supported image".into()))?;
    let (_, o) = selected(s).ok_or_else(|| CmdError::Disabled("no picture".into()))?;
    let InlineObject::Image { media, .. } = o else { return Err(CmdError::Disabled("not a picture".into())) };
    let key = s.doc.add_media(bytes, "png");
    let orig = s.originals.get(&media).cloned().unwrap_or(media);
    s.originals.entry(key.clone()).or_insert(orig);
    let ratio = ph as f32 / (pw as f32).max(1.0);
    with_obj(s, |o| {
        if let InlineObject::Image { media, w, h, crop, .. } = o {
            *media = key.clone();
            *h = *w * ratio;
            *crop = [0.0; 4];
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
    pos.filter(|p| movable_story(s, p.story) && s.doc.para_at(p).and_then(|q| q.object_at(p.off)).is_some_and(InlineObject::is_drawing))
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
    let story = picks.first().cloned().map(|p| p.story).ok_or_else(|| CmdError::Failed("nothing to group".into()))?;
    if picks.iter().any(|p| p.story != story) {
        return Err(CmdError::Disabled("objects in different stories (body, header, footer) can't be grouped".into()));
    }
    for pos in &picks {
        if pos.path.depth() > 0 || !movable_story(s, pos.story) {
            return Err(CmdError::Disabled("only objects in the body text, headers and footers (not in tables) can be grouped".into()));
        }
        let obj = s.doc.para_at(pos).and_then(|p| p.object_at(pos.off)).cloned().ok_or_else(|| CmdError::Failed("object vanished".into()))?;
        if matches!(obj, InlineObject::Graphic { .. }) {
            return Err(CmdError::Disabled(FROZEN.into()));
        }
        if obj.is_canvas() {
            return Err(CmdError::Disabled("a drawing canvas can't be grouped".into()));
        }
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
    // The members as they show on the page: each one's unturned frame and turn (a group among
    // them gives its members, its turn taken into theirs).
    let mut shown: Vec<([f32; 4], Spin, &InlineObject)> = Vec::new();
    for (obj, hit) in &found {
        let r = hit.rect;
        match obj {
            InlineObject::Group { .. } => shown.extend(shown_members(obj, [r.x, r.y, r.w, r.h])),
            _ => shown.push(([r.x, r.y, r.w, r.h], hit.spin, obj)),
        }
    }
    // The group's box takes in what they cover, turned.
    let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for ([x, y, w, h], spin, _) in &shown {
        let b = spin.bounds(wordcraft_geom::Rect::new(*x, *y, *w, *h));
        (x0, y0) = (x0.min(b.x), y0.min(b.y));
        (x1, y1) = (x1.max(b.right()), y1.max(b.bottom()));
    }
    if shown.is_empty() {
        return Err(CmdError::Failed("nothing to group".into()));
    }
    let (w, h) = ((x1 - x0).clamp(1.0, MAX_OFFSET), (y1 - y0).clamp(1.0, MAX_OFFSET));
    // Each member keeps its turn: the group itself starts unturned.
    let children: Vec<GroupChild> = shown
        .into_iter()
        .map(|([x, y, cw, ch], spin, obj)| {
            let mut obj = obj.clone();
            obj.set_size(cw, ch);
            if let Some(f) = obj.float_mut() {
                // Each member keeps its own alt text.
                let alt = std::mem::take(&mut f.alt);
                *f = Float { alt, ..Float::default() };
                f.set_spin(spin);
            }
            GroupChild { x: x - x0, y: y - y0, obj }
        })
        .collect();
    // The group takes the first member's placement; its members keep their own alt text.
    let mut float = found.first().and_then(|(o, _)| o.frame()).map(|(_, _, f)| Float { alt: Default::default(), ..f.clone() }).unwrap_or_default();
    float.set_spin(Spin::default());
    let grouped = InlineObject::Group { w, h, float, ch_w: w, ch_h: h, children, canvas: None };
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
        if pos.path.depth() > 0 || !movable_story(s, pos.story) {
            return Err(CmdError::Disabled("only groups in the body text, headers and footers can be ungrouped in line with text".into()));
        }
        let hit = s.layout().object(&pos, s.page_hint).ok_or_else(|| CmdError::Failed("the object isn't laid out".into()))?;
        move_object(s, pos, Some(hit.page as u64), hit.rect.x, hit.rect.y, (*w, *h))?
    } else {
        unalign(s)?;
        pos
    };
    let obj = s.doc.para_at(&pos).and_then(|p| p.object_at(pos.off)).cloned().ok_or_else(|| CmdError::Failed("object vanished".into()))?;
    let Some((w, h, float)) = obj.frame() else { return Err(CmdError::Failed("object vanished".into())) };
    let float = float.clone();
    // Each member where the group shows it: the group's turn moves its centre about the group's
    // centre and adds to its own turn.
    let members: Vec<InlineObject> = shown_members(&obj, [0.0, 0.0, w, h])
        .into_iter()
        .map(|([x, y, cw, ch], spin, c)| {
            let mut c = c.clone();
            let (mw, mh) = (cw.max(min_size(&c)), ch.max(min_size(&c)));
            c.set_size(mw, mh);
            // Grown to its smallest size about its centre, so it stays centred where it was.
            let (x, y) = (x - (mw - cw) / 2.0, y - (mh - ch) / 2.0);
            if let Some(f) = c.float_mut() {
                // Each member keeps its own alt text; the group's placement is shared out.
                let alt = std::mem::take(&mut f.alt);
                *f = Float { x: (float.x + x).clamp(-MAX_OFFSET, MAX_OFFSET), y: (float.y + y).clamp(-MAX_OFFSET, MAX_OFFSET), alt, ..float.clone() };
                f.set_spin(spin);
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

/// A group's members as it shows them in its frame `[x, y, w, h]`: each member's unturned frame
/// and the turn it is drawn with (its own inside the group's). The group's turn, about its
/// centre, moves each member's centre. Empty for anything but a group.
fn shown_members(group: &InlineObject, [x, y, w, h]: [f32; 4]) -> Vec<([f32; 4], Spin, &InlineObject)> {
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
                    InlineObject::Graphic { graphic, .. } => ("graphic", format!("{:?} {}", graphic.kind, i + 1)),
                    InlineObject::Group { .. } => ("group", format!("Group {}", i + 1)),
                    _ => ("object", format!("Object {}", i + 1)),
                };
                let hidden = o.frame().is_some_and(|(_, _, f)| f.hidden);
                json!({"index": i, "kind": kind, "name": name, "pos": super::pos_json(&pos), "size": obj_size(&o), "hidden": hidden})
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

    /// Ctrl+drag: a copy lands where dropped and is selected; the original stays, the clipboard
    /// is untouched, and it is one undo step.
    #[test]
    fn copy_to_drops_a_copy_and_leaves_the_original() {
        let mut s = Session::new(wordcraft_doc::Document::new());
        s.run("text.insert", &json!({"text": "Some text "})).unwrap();
        s.run("insert.shape", &json!({"kind": "rectangle"})).unwrap();
        s.run("arrange.bounds", &json!({"width": 100, "height": 60, "page": 0, "x": 100, "y": 200})).unwrap();
        s.clipboard_text = "keep".into();
        let count = |s: &Session| s.doc.para(StoryRef::Body, &wordcraft_doc::Path::top(0)).unwrap().objects.len();
        assert_eq!(count(&s), 1);
        s.run("arrange.copyTo", &json!({"page": 0, "x": 300, "y": 200})).unwrap();
        assert_eq!(count(&s), 2);
        assert_eq!(s.clipboard_text, "keep");
        let (_, sel) = selected(&s).unwrap();
        assert!(matches!(sel, InlineObject::Shape { .. }));
        let at = s.layout().find_object(0, |o| (o.rect.x - 300.0).abs() < 0.5).expect("the copy is at the drop point");
        assert_eq!(at.pos(), selected(&s).unwrap().0);
        assert!(s.layout().find_object(0, |o| (o.rect.x - 100.0).abs() < 0.5).is_some(), "the original stays");
        assert!(s.run("arrange.copyTo", &json!({"page": 0})).is_err());
        s.run("edit.undo", &json!({})).unwrap();
        assert_eq!(count(&s), 1);
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

    /// #965: a picture or icon inserted by a command leaves the caret after it, so a script's next
    /// Enter or typing keeps it (and the saved file has it); Picture Format commands still reach it.
    #[test]
    fn typing_after_an_inserted_picture_keeps_it() {
        let data = super::super::insert::base64_encode(&png(20, 10));
        for (id, params) in [("insert.picture", json!({"data": data, "width": 100})), ("insert.icon", json!({"id": "sun"}))] {
            let mut s = Session::new(wordcraft_doc::Document::new());
            s.run("text.insert", &json!({"text": "x"})).unwrap();
            s.run("text.newParagraph", &json!({})).unwrap();
            s.run(id, &params).unwrap();
            assert!(s.sel.is_collapsed(), "{id}: caret, not a selection");
            assert!(selected(&s).is_some(), "{id}: still the object Picture Format acts on");
            s.run("text.newParagraph", &json!({})).unwrap();
            s.run("text.insert", &json!({"text": "cap"})).unwrap();
            assert_eq!(s.run("arrange.selectionPane", &json!({})).unwrap().as_array().map(Vec::len), Some(1), "{id}");
            let paras: Vec<String> = s
                .doc
                .body
                .iter()
                .filter_map(|b| match &**b {
                    wordcraft_doc::Block::Para(p) => Some(p.text.replace(wordcraft_doc::para::OBJ, "#")),
                    _ => None,
                })
                .collect();
            assert_eq!(paras, ["X", "#", "cap"], "{id}");
            let docx = crate::io::save_bytes("p.docx", &s.doc).unwrap();
            assert!(docx.windows(11).any(|w| w == b"word/media/"), "{id}: the saved file keeps the picture");
        }
    }

    /// #965: two pictures inserted by commands in a row are both kept; the second doesn't
    /// replace the first.
    #[test]
    fn two_inserted_pictures_in_a_row_are_both_kept() {
        let data = super::super::insert::base64_encode(&png(20, 10));
        let mut s = Session::new(wordcraft_doc::Document::new());
        s.run("insert.picture", &json!({"data": data})).unwrap();
        s.run("insert.picture", &json!({"data": data})).unwrap();
        assert_eq!(s.run("arrange.selectionPane", &json!({})).unwrap().as_array().map(Vec::len), Some(2));
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
        // The frame keeps its size; it turns.
        assert_eq!(obj_size(&selected(&s).unwrap().1), (100.0, 50.0));
        s.run("picture.reset", &json!({})).unwrap();
        s.run("arrange.wrap", &json!({"wrap": "square"})).unwrap();
        // Text on one side only (#354), one undo step; an unknown side is refused.
        let side = |s: &Session| selected(s).unwrap().1.frame().unwrap().2.wrap_text;
        s.run("arrange.wrapText", &json!({"side": "largest"})).unwrap();
        assert_eq!(side(&s), WrapText::Largest);
        assert!(s.run("arrange.wrapText", &json!({"side": "middle"})).is_err());
        s.run("edit.undo", &json!({})).unwrap();
        assert_eq!(side(&s), WrapText::BothSides);
        s.run("arrange.position", &json!({"preset": "topRight"})).unwrap();
        let l = s.run("arrange.selectionPane", &json!({})).unwrap();
        assert_eq!(l.as_array().unwrap().len(), 1);
        let _ = s.layout();
        s.run("select.collapse", &json!({"end": true})).unwrap();
        s.run("text.insert", &json!({"text": "x"})).unwrap();
        assert!(s.run("picture.crop", &json!({"left": 0.1})).is_err());
    }

    /// Transparency is absolute from the original pixels: 80% then 0% is opaque again
    /// (it used to stay faded), and repeating a value is idempotent. Corrections and color
    /// re-set the same way: brightness 0 and saturation 100 restore the original.
    #[test]
    fn transparency_is_absolute_from_the_original() {
        let mut s = Session::new(wordcraft_doc::Document::new());
        let data = super::super::insert::base64_encode(&png(20, 10));
        s.run("insert.picture", &json!({"data": data})).unwrap();
        let rgba = |s: &Session| {
            let (_, o) = selected(s).unwrap();
            let InlineObject::Image { media, .. } = o else { panic!("a picture") };
            let bytes = s.doc.media.get(&media).cloned().unwrap();
            image::load_from_memory(&bytes).unwrap().to_rgba8().pixels().map(|p| p.0).collect::<Vec<_>>()
        };
        let alpha = |s: &Session| rgba(s).iter().map(|p| p[3]).collect::<Vec<_>>();
        let orig = rgba(&s);
        assert!(orig.iter().all(|p| p[3] == 255));
        s.run("picture.transparency", &json!({"percent": 80})).unwrap();
        assert!(alpha(&s).iter().all(|&a| a == 50), "255 * 0.2 truncated");
        s.run("picture.transparency", &json!({"percent": 0})).unwrap();
        assert_eq!(rgba(&s), orig, "back to opaque");
        s.run("picture.transparency", &json!({"percent": 50})).unwrap();
        let once = rgba(&s);
        assert!(once.iter().all(|p| p[3] == 127), "255 * 0.5");
        s.run("picture.transparency", &json!({"percent": 50})).unwrap();
        assert_eq!(rgba(&s), once, "repeating is idempotent");
        s.run("picture.corrections", &json!({"brightness": 20})).unwrap();
        assert_ne!(rgba(&s), orig);
        s.run("picture.corrections", &json!({"brightness": 0})).unwrap();
        assert_eq!(rgba(&s), orig, "brightness 0 restores the original");
        s.run("picture.color", &json!({"mode": "saturation", "saturation": 0})).unwrap();
        assert!(rgba(&s).iter().all(|p| p[0] == p[1] && p[1] == p[2]), "grayscale");
        s.run("picture.color", &json!({"mode": "saturation", "saturation": 100})).unwrap();
        assert!(
            rgba(&s).iter().zip(orig.iter()).all(|(a, b)| a.iter().zip(b.iter()).all(|(x, y)| (*x as i16 - *y as i16).abs() <= 2)),
            "saturation 100 restores within float rounding"
        );
        s.run("picture.reset", &json!({})).unwrap();
        assert_eq!(rgba(&s), orig, "reset still finds the original");
    }

    /// #513: Wrap Text › Fix Position on Page / Move with Text re-anchor a floating picture
    /// vertically without moving it; one undo step each; an inline picture can't.
    #[test]
    fn move_with_text_and_fix_position_keep_the_picture_in_place() {
        use wordcraft_doc::para::Anchor;
        let mut s = Session::new(wordcraft_doc::Document::new());
        s.run("text.insert", &json!({"text": "First paragraph\nSecond"})).unwrap();
        let data = super::super::insert::base64_encode(&png(20, 10));
        s.run("insert.picture", &json!({"data": data, "width": 100})).unwrap();
        assert!(s.run("arrange.moveWithText", &json!({"value": false})).is_err(), "inline: no anchoring");
        s.run("arrange.wrap", &json!({"wrap": "square"})).unwrap();
        let v_rel = |s: &Session| selected(s).unwrap().1.frame().unwrap().2.v_rel;
        let with_text = |s: &Session| moves_with_text(selected(s).unwrap().1.frame().unwrap().2);
        let rect = |s: &mut Session| {
            let pos = selected(s).unwrap().0;
            s.layout().object(&pos, 0).unwrap().rect
        };
        let before = rect(&mut s);
        s.run("arrange.moveWithText", &json!({"value": false})).unwrap();
        assert_eq!(v_rel(&s), Anchor::Page);
        assert!(!with_text(&s));
        assert!((rect(&mut s).y - before.y).abs() < 0.01, "it stays put");
        s.run("arrange.moveWithText", &json!({"value": true})).unwrap();
        assert!(with_text(&s));
        assert!((rect(&mut s).y - before.y).abs() < 0.01, "it stays put");
        s.run("edit.undo", &json!({})).unwrap();
        assert_eq!(v_rel(&s), Anchor::Page);
    }

    /// #332: Rotate and Flip turn the frame (an angle and mirroring), never the pixels; an exact
    /// angle is normalised; each is one undo step; the rotated frame takes its rotated room.
    #[test]
    fn rotate_and_flip_turn_the_frame_not_the_pixels() {
        let mut s = Session::new(wordcraft_doc::Document::new());
        let data = super::super::insert::base64_encode(&png(20, 10));
        s.run("insert.picture", &json!({"data": data, "width": 200})).unwrap();
        let (pos, before) = selected(&s).unwrap();
        let InlineObject::Image { media, .. } = &before else { panic!("a picture") };
        let (media, media_count) = (media.clone(), s.doc.media.len());
        let spin = |s: &Session| selected(s).unwrap().1.frame().unwrap().2.spin();
        s.run("arrange.rotate", &json!({"direction": "right"})).unwrap();
        assert_eq!(spin(&s).deg, 90.0);
        let (_, o) = selected(&s).unwrap();
        assert!(matches!(&o, InlineObject::Image { media: m, w: 200.0, h: 100.0, .. } if *m == media), "{o:?}");
        assert_eq!(s.doc.media.len(), media_count, "no new bitmap");
        // Inline, the line makes room for the turned picture: 100 wide, 200 tall.
        let r = s.layout().object(&pos, 0).unwrap();
        let b = r.bounds();
        assert!((b.w - 100.0).abs() < 0.5 && (b.h - 200.0).abs() < 0.5, "{b:?}");
        s.run("arrange.rotate", &json!({"direction": "left"})).unwrap();
        s.run("arrange.rotate", &json!({"direction": "left"})).unwrap();
        assert_eq!(spin(&s).deg, 270.0);
        // A flip mirrors the page view: the angle reverses.
        s.run("arrange.rotate", &json!({"direction": "flipHorizontal"})).unwrap();
        assert_eq!(spin(&s), wordcraft_geom::Spin::new(90.0, true, false));
        s.run("arrange.rotation", &json!({"degrees": -30.0})).unwrap();
        assert_eq!(spin(&s), wordcraft_geom::Spin::new(330.0, true, false));
        assert!(s.run("arrange.rotation", &json!({"degrees": "NaN"})).is_err());
        assert!(s.run("arrange.rotate", &json!({"direction": "sideways"})).is_err());
        s.run("edit.undo", &json!({})).unwrap();
        assert_eq!(spin(&s), wordcraft_geom::Spin::new(90.0, true, false));
        // Shapes and text boxes turn too.
        s.run("select.collapse", &json!({"end": true})).unwrap();
        s.run("insert.shape", &json!({"kind": "textBox"})).unwrap();
        s.run("arrange.rotation", &json!({"degrees": 45})).unwrap();
        assert_eq!(spin(&s).deg, 45.0);
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

    /// Page 1 as exported (`file.exportPng` at scale 1), its pixels.
    fn page_pixels(s: &Session) -> Vec<u8> {
        let l = s.export_layout();
        wordcraft_render::render_page(&s.doc, &l.pages[0], 1.0, &Default::default()).pixels
    }

    /// How many pixels of two renders clearly differ (not just in antialiasing).
    fn pixels_differing(a: &[u8], b: &[u8]) -> usize {
        assert_eq!(a.len(), b.len());
        a.chunks(4).zip(b.chunks(4)).filter(|(p, q)| p.iter().zip(q.iter()).any(|(x, y)| x.abs_diff(*y) > 96)).count()
    }

    /// A rectangle at (100, 100) and an arrow at (250, 100), both 80 × 40, on a blank page; the
    /// arrow turned `arrow_deg`.
    fn rectangle_and_arrow(arrow_deg: f32) -> Session {
        let mut s = Session::new(wordcraft_doc::Document::new());
        s.run("insert.shape", &json!({"kind": "rectangle", "width": 80, "height": 40})).unwrap();
        s.run("arrange.bounds", &json!({"x": 100, "y": 100})).unwrap();
        s.run("select.collapse", &json!({"end": true})).unwrap();
        s.run("insert.shape", &json!({"kind": "arrow", "width": 80, "height": 40})).unwrap();
        s.run("arrange.bounds", &json!({"x": 250, "y": 100})).unwrap();
        s.run("arrange.rotation", &json!({"degrees": arrow_deg})).unwrap();
        s
    }

    fn select_both(s: &mut Session) {
        s.run("select.objects", &json!({"index": 0})).unwrap();
        s.run("select.addObject", &json!({"index": 1})).unwrap();
    }

    /// #441: grouping keeps a turned member's orientation, through save and reopen too.
    #[test]
    fn group_keeps_a_rotated_members_orientation() {
        for deg in [90.0, 0.0] {
            let mut s = rectangle_and_arrow(deg);
            let before = page_pixels(&s);
            select_both(&mut s);
            s.run("arrange.group", &json!({})).unwrap();
            let (_, g) = selected(&s).unwrap();
            let InlineObject::Group { children, float, .. } = &g else { panic!("a group: {g:?}") };
            assert_eq!(children.len(), 2, "both members stay shapes");
            assert!(children.iter().all(|c| matches!(c.obj, InlineObject::Shape { .. })));
            let rots: Vec<f32> = children.iter().filter_map(|c| c.obj.frame().map(|(_, _, f)| f.rot)).collect();
            assert_eq!(rots, vec![0.0, deg], "the rectangle unturned, the arrow keeps its turn");
            assert_eq!(float.rot, 0.0, "the group itself is unturned");
            assert!(pixels_differing(&before, &page_pixels(&s)) < 20, "{deg}°: grouping changes nothing on the page");
            // Saved and reopened, it looks the same.
            let back = wordcraft_docx::read(&wordcraft_docx::write(&s.doc).unwrap()).unwrap();
            let reopened = Session::new(back);
            assert!(pixels_differing(&before, &page_pixels(&reopened)) < 20, "{deg}°: reopened");
            // Undo gives back the two objects.
            s.run("edit.undo", &json!({})).unwrap();
            assert_eq!(all_objects(&s).len(), 2);
            assert!(pixels_differing(&before, &page_pixels(&s)) < 20);
        }
    }

    /// #442: Size resizes a selected group, its members scaled with it, as Move or Resize does.
    #[test]
    fn size_resizes_a_group() {
        let grouped = || {
            let mut s = Session::new(wordcraft_doc::Document::new());
            s.run("insert.shape", &json!({"kind": "rectangle", "width": 80, "height": 40})).unwrap();
            s.run("arrange.bounds", &json!({"x": 100, "y": 100})).unwrap();
            s.run("select.collapse", &json!({"end": true})).unwrap();
            s.run("insert.shape", &json!({"kind": "rectangle", "width": 80, "height": 40})).unwrap();
            s.run("arrange.bounds", &json!({"x": 250, "y": 100})).unwrap();
            s.run("select.addObject", &json!({"index": 0})).unwrap();
            s.run("arrange.group", &json!({})).unwrap();
            assert_eq!(s.run("arrange.selectionPane", &json!({})).unwrap()[0]["size"], json!([230.0, 40.0]));
            s
        };
        let mut sized = grouped();
        sized.run("picture.size", &json!({"width": 400, "height": 200})).unwrap();
        let list = sized.run("arrange.selectionPane", &json!({})).unwrap();
        assert_eq!((list.as_array().unwrap().len(), &list[0]["kind"], &list[0]["size"]), (1, &json!("group"), &json!([400.0, 200.0])));
        let mut moved = grouped();
        moved.run("arrange.bounds", &json!({"width": 400, "height": 200})).unwrap();
        let (_, a) = selected(&sized).unwrap();
        let (_, b) = selected(&moved).unwrap();
        assert_eq!(a, b, "the same group, members and all, as Move or Resize makes");
        let InlineObject::Group { children, .. } = &a else { panic!() };
        assert_eq!(children.len(), 2);
        let members: Vec<[f32; 4]> = a.group_rects(0.0, 0.0, 400.0, 200.0).into_iter().map(|(r, _)| r).collect();
        assert!(near(members[1], [400.0 * 150.0 / 230.0, 0.0, 400.0 * 80.0 / 230.0, 200.0]), "scaled with the group: {members:?}");
    }

    /// #443: ungrouping a turned group keeps its members where, and as turned as, it showed them.
    #[test]
    fn ungroup_keeps_a_rotated_groups_arrangement() {
        for turn in [true, false] {
            let mut s = rectangle_and_arrow(0.0);
            select_both(&mut s);
            s.run("arrange.group", &json!({})).unwrap();
            if turn {
                s.run("arrange.rotation", &json!({"degrees": 90})).unwrap();
            }
            let before = page_pixels(&s);
            s.run("arrange.ungroup", &json!({})).unwrap();
            let list = all_objects(&s);
            assert_eq!(list.len(), 2, "two separate objects");
            let want = if turn { 90.0 } else { 0.0 };
            assert!(list.iter().all(|(_, o)| o.frame().is_some_and(|(_, _, f)| f.rot == want)));
            assert!(pixels_differing(&before, &page_pixels(&s)) < 20, "turned {turn}: ungrouping changes nothing on the page");
            if turn {
                // The rectangle is above the arrow now, as the turned group showed it.
                let r: Vec<[f32; 4]> = list.iter().map(|(p, _)| rect_of(&mut s, p)).collect();
                assert!((r[0][0] - r[1][0]).abs() < 0.5 && r[0][1] < r[1][1], "{r:?}");
            }
            s.run("edit.undo", &json!({})).unwrap();
            assert!(matches!(all_objects(&s).as_slice(), [(_, InlineObject::Group { .. })]));
            assert_eq!(pixels_differing(&before, &page_pixels(&s)), 0);
        }
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

    /// A crop handle drop: `picture.crop` with the new size is one edit, one undo step.
    #[test]
    fn picture_crop_with_size_is_one_undo_step() {
        let mut s = Session::new(wordcraft_doc::Document::new());
        let data = super::super::insert::base64_encode(&png(20, 10));
        s.run("insert.picture", &json!({"data": data})).unwrap();
        s.run("picture.size", &json!({"width": 200.0, "height": 100.0, "lockAspect": false})).unwrap();
        let image = |s: &Session| match selected(s) {
            Some((_, InlineObject::Image { w, h, crop, .. })) => (w, h, crop),
            other => panic!("expected image, got {other:?}"),
        };
        s.run("picture.crop", &json!({"left": 0.25, "top": 0.0, "right": 0.0, "bottom": 0.1, "width": 150.0, "height": 90.0})).unwrap();
        assert_eq!(image(&s), (150.0, 90.0, [0.25, 0.0, 0.0, 0.1]));
        s.run("edit.undo", &json!({})).unwrap();
        assert_eq!(image(&s), (200.0, 100.0, [0.0; 4]));
        // Clamps: each side at most 0.45; hostile sizes are ignored or clamped, never applied raw.
        s.run("picture.crop", &json!({"left": 9.0, "right": -3.0, "width": "x", "height": -5.0})).unwrap();
        let (w, h, crop) = image(&s);
        assert_eq!(crop, [0.45, 0.0, 0.0, 0.0]);
        assert_eq!(w, 200.0);
        assert!(h >= 1.0, "{h}");
    }

    /// Cropping a floating picture's left or top with its new size keeps the visible part where
    /// it is on the page: the frame moves by the trimmed amount, in the same undo step.
    #[test]
    fn cropping_a_floating_picture_keeps_the_visible_part_in_place() {
        let mut s = Session::new(wordcraft_doc::Document::new());
        let data = super::super::insert::base64_encode(&png(20, 10));
        s.run("insert.picture", &json!({"data": data})).unwrap();
        s.run("picture.size", &json!({"width": 200.0, "height": 100.0, "lockAspect": false})).unwrap();
        s.run("arrange.wrap", &json!({"wrap": "square"})).unwrap();
        let at = |s: &Session| match selected(s) {
            Some((_, InlineObject::Image { float, .. })) => (float.x, float.y),
            other => panic!("expected image, got {other:?}"),
        };
        let (x0, y0) = at(&s);
        s.run("picture.crop", &json!({"left": 0.25, "top": 0.1, "width": 150.0, "height": 90.0})).unwrap();
        let (x1, y1) = at(&s);
        assert!((x1 - (x0 + 50.0)).abs() < 1e-3 && (y1 - (y0 + 10.0)).abs() < 1e-3, "{x0},{y0} -> {x1},{y1}");
        s.run("edit.undo", &json!({})).unwrap();
        assert_eq!(at(&s), (x0, y0));
    }

    /// Flipped left to right, the source's right edge is on the page's left: cropping it moves
    /// the frame; cropping the source's left (now on the right) doesn't.
    #[test]
    fn cropping_a_flipped_floating_picture_moves_it_by_the_side_on_its_left() {
        let mut s = Session::new(wordcraft_doc::Document::new());
        let data = super::super::insert::base64_encode(&png(20, 10));
        s.run("insert.picture", &json!({"data": data})).unwrap();
        s.run("picture.size", &json!({"width": 200.0, "height": 100.0, "lockAspect": false})).unwrap();
        s.run("arrange.wrap", &json!({"wrap": "square"})).unwrap();
        s.run("arrange.rotation", &json!({"degrees": 0.0, "flipH": true, "flipV": true})).unwrap();
        let at = |s: &Session| match selected(s) {
            Some((_, InlineObject::Image { float, .. })) => (float.x, float.y),
            other => panic!("expected image, got {other:?}"),
        };
        let (x0, y0) = at(&s);
        s.run("picture.crop", &json!({"right": 0.25, "bottom": 0.1, "width": 150.0, "height": 90.0})).unwrap();
        let (x1, y1) = at(&s);
        assert!((x1 - (x0 + 50.0)).abs() < 1e-3 && (y1 - (y0 + 10.0)).abs() < 1e-3, "{x0},{y0} -> {x1},{y1}");
        s.run("picture.crop", &json!({"left": 0.25, "right": 0.25, "bottom": 0.1, "width": 100.0, "height": 90.0})).unwrap();
        assert!((at(&s).0 - x1).abs() < 1e-3, "the source's left is on the right: no move");
    }

    /// A header picture resizes and drags like a body one: the move stays in the header story
    /// and lands where dropped; the header hit test finds it.
    #[test]
    fn replace_image_swaps_the_bitmap_keeps_the_width_and_undoes() {
        let mut s = Session::new(wordcraft_doc::Document::new());
        let first = super::super::insert::base64_encode(&png(20, 10));
        s.run("insert.picture", &json!({"data": first, "width": 100})).unwrap();
        assert_eq!(selected_picture_bytes(&s).map(|b| b.len()), Some(png(20, 10).len()));
        let second = super::super::insert::base64_encode(&png(40, 40));
        let r = s.run("picture.replaceImage", &json!({"data": second})).unwrap();
        assert_eq!(r["object"]["w"], 100.0);
        assert_eq!(r["object"]["h"], 100.0);
        assert_eq!(selected_picture_bytes(&s).map(|b| b.to_vec()), Some(png(40, 40)));
        s.run("edit.undo", &json!({})).unwrap();
        assert_eq!(selected_picture_bytes(&s).map(|b| b.to_vec()), Some(png(20, 10)));
    }

    #[test]
    fn header_pictures_can_be_resized_and_dragged() {
        let mut s = Session::new(wordcraft_doc::Document::new());
        s.run("insert.editHeader", &json!({})).unwrap();
        let story = s.sel.focus.story;
        assert!(matches!(story, StoryRef::Part(_)), "editing the header, got {story:?}");
        let data = super::super::insert::base64_encode(&png(20, 10));
        s.run("insert.picture", &json!({"data": data, "width": 100})).unwrap();
        let (pos, _) = selected(&s).expect("a new header picture is selected");
        assert_eq!(pos.story, story);
        s.run("arrange.bounds", &json!({"width": 160, "height": 90})).unwrap();
        let r = s.run("arrange.bounds", &json!({"page": 0, "x": 200, "y": 60})).unwrap();
        assert_eq!(r["object"]["float"]["wrap"], "square");
        let o = s.layout().object(&selected(&s).unwrap().0, 0).expect("laid out");
        assert_eq!(o.story, story);
        assert!((o.rect.x - 200.0).abs() < 1.0 && (o.rect.y - 60.0).abs() < 1.0, "{:?}", o.rect);
        let hit = s.layout().object_at_in(o.page, o.rect.x + 5.0, o.rect.y + 5.0, 4.0, story).expect("header hit");
        assert_eq!(hit.pos(), o.pos());
        s.run("arrange.nudge", &json!({"dx": 6, "dy": 4})).unwrap();
        let n = s.layout().object(&selected(&s).unwrap().0, 0).expect("laid out");
        assert!((n.rect.x - 206.0).abs() < 1.0 && (n.rect.y - 64.0).abs() < 1.0, "{:?}", n.rect);
        s.run("edit.undo", &json!({})).unwrap();
        let u = s.layout().object(&selected(&s).unwrap().0, 0).expect("laid out");
        assert!((u.rect.x - 200.0).abs() < 1.0 && (u.rect.y - 60.0).abs() < 1.0, "{:?}", u.rect);
    }

    /// Shape Format › Text (#369): direction and alignment on the box the caret is in, then
    /// Create Link to an empty box and Break Link, each one undo step.
    #[test]
    fn text_direction_alignment_and_links() {
        let mut s = Session::new(wordcraft_doc::Document::new());
        s.run("insert.textBox", &json!({"text": "Some text that flows on"})).unwrap();
        let a = text_box_target(&s).unwrap().1;
        s.run("shape.textDirection", &json!({"dir": "vert270"})).unwrap();
        s.run("shape.alignText", &json!({"anchor": "ctr"})).unwrap();
        let body = |s: &Session, id: u32| s.doc.parts.get(&id).unwrap().body;
        assert_eq!((body(&s, a).vert, body(&s, a).anchor), (TextVert::Vert270, VAlign::Center));
        assert!(s.run("shape.textDirection", &json!({"dir": "sideways"})).is_err());
        assert_eq!(body(&s, a).vert, TextVert::Vert270, "a bad value changes nothing");

        // A second, empty box after the first; link the first to it by its pane index.
        s.run("caret.set", &json!({"pos": Pos::body(0, 3)})).unwrap();
        s.run("insert.textBox", &json!({})).unwrap();
        let b = text_box_target(&s).unwrap().1;
        s.run("select.objects", &json!({"index": 0})).unwrap();
        assert_eq!(text_box_target(&s).map(|t| t.1), Some(a));
        s.run("shape.link", &json!({"target": 1})).unwrap();
        assert_eq!(s.doc.text_box_chains(), vec![vec![a, b]]);
        assert!(can_link(&s).is_some(), "linked once: Break Link first");
        // The second box shows the first one's story.
        let l = s.layout();
        assert_eq!(
            l.pages[0].items.iter().filter(|it| matches!(it, wordcraft_layout::Placed::Object { text_box: Some(t), .. } if *t == a)).count(),
            2
        );
        // Linking back to the start, or to a box with text, is refused.
        s.run("select.objects", &json!({"index": 1})).unwrap();
        assert!(s.run("shape.link", &json!({"target": 0})).is_err());
        s.run("select.objects", &json!({"index": 0})).unwrap();
        s.run("shape.breakLink", &json!({})).unwrap();
        assert!(s.doc.text_box_chains().is_empty());
        s.run("edit.undo", &json!({})).unwrap();
        assert_eq!(s.doc.text_box_chains(), vec![vec![a, b]]);
        s.run("edit.undo", &json!({})).unwrap();
        assert!(s.doc.text_box_chains().is_empty());
    }

    #[test]
    fn shape_alt_text_and_decorative_are_one_undo_step_each() {
        let mut s = Session::new(wordcraft_doc::Document::new());
        s.run("insert.shape", &json!({"kind": "ellipse"})).unwrap();
        s.run("object.altText", &json!({"text": "A green circle", "title": "Logo"})).unwrap();
        let (_, o) = selected(&s).unwrap();
        assert_eq!((o.alt_text(), o.frame().map(|f| f.2.alt.title.as_str()), o.is_decorative()), ("A green circle", Some("Logo"), false));
        s.run("object.altText", &json!({"decorative": true})).unwrap();
        assert!(selected(&s).unwrap().1.is_decorative());
        s.run("edit.undo", &json!({})).unwrap();
        let (_, o) = selected(&s).unwrap();
        assert!(!o.is_decorative() && o.alt_text() == "A green circle");
        s.run("edit.undo", &json!({})).unwrap();
        assert_eq!(selected(&s).unwrap().1.alt_text(), "");
        // Hostile input is capped, and nothing to set is an error.
        s.run("object.altText", &json!({"text": "x".repeat(50_000)})).unwrap();
        assert_eq!(selected(&s).unwrap().1.alt_text().len(), wordcraft_doc::para::MAX_ALT_TEXT);
        assert!(s.run("object.altText", &json!({})).is_err());
        // A picture-only alias stays a picture-only alias.
        assert!(s.run("picture.altText", &json!({"text": "no"})).is_err());
    }

    #[test]
    fn table_alt_text_sets_title_and_description() {
        let mut s = Session::new(wordcraft_doc::Document::new());
        s.run("insert.table", &json!({"rows": 2, "cols": 2})).unwrap();
        s.run("table.altText", &json!({"title": "Prices", "text": "Prices by year"})).unwrap();
        let t = s.doc.body.iter().find_map(|b| if let wordcraft_doc::Block::Table(t) = &**b { Some(t.clone()) } else { None }).unwrap();
        assert_eq!((t.props.caption.as_deref(), t.props.description.as_deref()), (Some("Prices"), Some("Prices by year")));
        s.run("table.altText", &json!({"text": ""})).unwrap();
        let t = s.doc.body.iter().find_map(|b| if let wordcraft_doc::Block::Table(t) = &**b { Some(t.clone()) } else { None }).unwrap();
        assert_eq!((t.props.caption.as_deref(), t.props.description.as_deref()), (Some("Prices"), None));
    }

    /// The lines of `page` beside `r` vertically (top above its bottom, bottom below its top).
    fn lines_beside(s: &mut Session, page: usize, r: wordcraft_geom::Rect) -> Vec<(f32, f32, f32)> {
        let layout = s.layout();
        wordcraft_layout::hit::page_lines(&layout.pages[page], StoryRef::Body)
            .iter()
            .filter(|l| l.top < r.bottom() - 0.5 && l.bottom > r.y + 0.5)
            .map(|l| (l.top, l.left, l.right))
            .collect()
    }

    /// #884: a Square-wrapped shape dragged across a two-column page: the text in whichever
    /// column it lands in wraps around it there, and its old place closes up.
    #[test]
    fn dragged_square_shape_wraps_text_in_its_new_column() {
        let mut s = Session::new(wordcraft_doc::Document::new());
        s.run("layout.columns", &json!({"count": 2})).unwrap();
        for i in 0..40 {
            if i > 0 {
                s.run("text.newParagraph", &json!({})).unwrap();
            }
            s.run("text.insert", &json!({"text": format!("Paragraph {i} has words enough to run over two or three lines of a narrow column.")}))
                .unwrap();
        }
        s.run("caret.docStart", &json!({})).unwrap();
        s.run("insert.shape", &json!({"kind": "rectangle"})).unwrap();
        s.run("arrange.bounds", &json!({"width": 80, "height": 60})).unwrap();
        let cols: Vec<(f32, f32)> = {
            let layout = s.layout();
            let mut v: Vec<(f32, f32)> =
                wordcraft_layout::hit::page_lines(&layout.pages[0], StoryRef::Body).iter().map(|l| (l.left, l.right)).collect();
            v.sort_by(|a, b| a.0.total_cmp(&b.0));
            v.dedup_by(|a, b| (a.0 - b.0).abs() < 1.0);
            v
        };
        assert_eq!(cols.len(), 2, "two columns: {cols:?}");
        let mid = |c: (f32, f32)| (c.0 + c.1) / 2.0 - 40.0;
        let wrapped = |s: &mut Session, r: wordcraft_geom::Rect| {
            let beside = lines_beside(s, 0, r);
            assert!(!beside.is_empty(), "text beside {r:?}");
            for (top, left, right) in beside {
                assert!(right <= r.x + 0.5 || left >= r.right() - 0.5, "line at {top} ({left}..{right}) runs into {r:?}");
            }
        };
        let full = |s: &mut Session, r: wordcraft_geom::Rect, c: (f32, f32)| {
            for (top, left, right) in lines_beside(s, 0, r) {
                if left < c.1 && right > c.0 {
                    assert!((left - c.0).abs() < 0.5 && (right - c.1).abs() < 0.5, "line at {top} ({left}..{right}) still kept clear in {c:?}");
                }
            }
        };
        // Into the second column, then back into the first, then again to the second, lower.
        for (col, y, from) in [(1, 300.0, None), (0, 200.0, Some(1)), (1, 400.0, Some(0))] {
            let old = object_area(&mut s).rect;
            s.run("arrange.bounds", &json!({"page": 0, "x": mid(cols[col]), "y": y})).unwrap();
            let o = object_area(&mut s);
            assert_eq!((o.page, o.rect.x, o.rect.y), (0, mid(cols[col]), y), "dropped where asked");
            wrapped(&mut s, o.rect);
            if let Some(c) = from {
                full(&mut s, old, cols[c]);
            }
        }
    }

    /// Two columns of `n` paragraphs.
    fn two_column_doc(n: usize) -> Session {
        let mut s = Session::new(wordcraft_doc::Document::new());
        s.run("layout.columns", &json!({"count": 2})).unwrap();
        for i in 0..n {
            if i > 0 {
                s.run("text.newParagraph", &json!({})).unwrap();
            }
            let text = format!("Paragraph {i} has words enough to run over several lines of a narrow column, and then some more words. ");
            s.run("text.insert", &json!({"text": text.repeat(3)})).unwrap();
        }
        s
    }

    /// The page's columns (left, right) from its section's geometry.
    fn page_columns(s: &mut Session, page: usize) -> Vec<(f32, f32)> {
        let layout = s.layout();
        let pg = &layout.pages[page];
        let boxes = s.doc.sections()[pg.section].1.column_boxes();
        boxes.iter().map(|(x, w)| (pg.body.x + x, pg.body.x + x + w)).collect()
    }

    /// #884 review: mirrored indents shift some paragraphs on even pages; that shift isn't taken
    /// for a column of its own, so the drop anchors to the paragraph under it, mirrored or not.
    #[test]
    fn mirrored_indents_make_no_extra_column() {
        let mut s = two_column_doc(40);
        for i in (1..40).step_by(2) {
            let p = s.doc.para_mut(StoryRef::Body, &wordcraft_doc::Path::top(i)).unwrap();
            p.props.mirror_indents = Some(true);
            p.props.indent_right = Some(36.0);
            p.touch();
        }
        s.touch();
        assert!(s.layout().pages.len() > 1);
        let cols = page_columns(&mut s, 1);
        // On page 2, a plain paragraph starting in the first column below a mirrored one.
        let layout = s.layout();
        let starts: Vec<(usize, f32, f32)> = layout.pages[1]
            .items
            .iter()
            .filter_map(|it| match it {
                wordcraft_layout::Placed::Lines { path, l0: 0, x, y, .. } if path.depth() == 0 && *x < cols[1].0 - 0.5 => Some((path.last(), *x, *y)),
                _ => None,
            })
            .collect();
        assert!(starts.iter().any(|p| p.1 > cols[0].0 + 1.0), "mirrored paragraphs shift right: {starts:?}");
        let &(plain, _, top) = starts.iter().skip(1).find(|p| p.0 % 2 == 0).unwrap();
        let mid = (cols[0].0 + cols[0].1) / 2.0;
        let got = anchor_paragraph(&s.doc, &layout, 1, (mid, top + 1.0), StoryRef::Body);
        assert_eq!(got, Some(wordcraft_doc::Path::top(plain)));
    }
}
