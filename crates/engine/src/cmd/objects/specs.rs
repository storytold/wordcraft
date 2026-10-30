use super::arrange::{MAX_OFFSET, bounds, copy_to, move_with_text, unalign};
use super::group::{add_object, group, ungroup};
use super::picture::{adjust, adjust_media, crop, recolor, relook, remove_background, reset, size};
use super::select::{
    all_objects, alt_text, can_group, edit_obj, has_floating, has_group, has_movable, has_object, has_picture, has_shape, has_shape_or_canvas,
    has_sizable, obj_size, objects_list, selected, with_float, with_obj,
};
use super::shape::{NO_TEXT_BOX, can_link, edit_shape_text, has_text_box, link_text_box, set_text_body, shape_effects, text_box_target};
use super::{CmdError, CommandSpec, InlineObject, Pos, Rgb, Selection, Spin, TextVert, VAlign, Wrap, WrapText, json, p, sel_result};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("picture.size", "Size", "Picture Format › Size", size)
            .params(
                r#"{"width"?: pt, "height"?: pt, "lockAspect"?: bool, "scale"?: percent, "original"?: bool} (scale: of the current size, or with original of the picture's original size, its pixels at 96 ppi less what is cropped; original alone restores that size; one side alone keeps the proportions unless lockAspect is false; 1–4000 pt: when the proportions are kept, a side past 4000 pt shrinks the other with it)"#,
            )
            .when(has_sizable),
        CommandSpec::new("picture.crop", "Crop", "Picture Format › Size", crop)
            .params(
                r#"{"left"?, "top"?, "right"?, "bottom"? (fractions of the source 0–0.95; the two sides of an axis keep at least 5% between them; a side left out is uncropped), "width"?: pt, "height"?: pt (the new display size, in the same undo step; a floating picture also moves by what a left/top crop trims, so the visible part stays put)}"#,
            )
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
                } else if sharpen < 0.0 && (-sharpen / 25.0).is_normal() {
                    // `blur` asserts on a subnormal width.
                    img = img.blur(-sharpen / 25.0);
                }
                Ok(img)
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
            adjust_media(s, &base, move |img| Ok(recolor(img, &mode, sat)))
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
                Ok(image::DynamicImage::ImageRgba8(rgba))
            })
        })
        .params(r#"{"color": "RRGGBB"}"#)
        .when(has_picture),
        CommandSpec::new("picture.effects", "Artistic Effects", "Picture Format › Adjust", |s, v| {
            // One effect at a time, as in Word: a second one replaces the first (#2052).
            let e = p::str(v, "effect").unwrap_or("blur").to_string();
            relook(s, move |l| l.effect = (e != "none").then_some(e))
        })
        .params(r#"{"effect": "blur|sharpen|invert|posterize|pixelate|none"} (replaces the picture's effect)"#)
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
                Ok(image::DynamicImage::ImageRgba8(rgba))
            })
        })
        .params(r#"{"percent": 0..100}"#)
        .when(has_picture),
        CommandSpec::new("picture.removeBackground", "Remove Background", "Picture Format › Adjust", |s, v| {
            let tol = p::f32(v, "tolerance").unwrap_or(30.0).clamp(0.0, 255.0);
            adjust(s, move |img| Ok(remove_background(img, tol)))
        })
        .params(r#"{"tolerance"?: 0..255}"#)
        .when(has_picture),
        CommandSpec::new("picture.compress", "Compress Pictures", "Picture Format › Adjust", |s, v| {
            let max = p::u64(v, "maxPixels").unwrap_or(1600).clamp(32, 20000) as u32;
            adjust(s, move |img| Ok(if img.width().max(img.height()) > max { img.resize(max, max, image::imageops::FilterType::Lanczos3) } else { img }))
        })
        .params(r#"{"maxPixels"?: n}"#)
        .when(has_picture),
        CommandSpec::new("picture.reset", "Reset Picture", "Picture Format › Adjust", reset).when(has_picture),
        CommandSpec::new("picture.change", "Change Picture", "Picture Format › Adjust", |s, v| {
            let bytes = if let Some(d) = p::str(v, "data") {
                crate::cmd::insert::base64_decode(d).ok_or_else(|| CmdError::Params("bad base64".into()))?
            } else if let Some(path) = p::str(v, "path") {
                crate::io::read_file(path).map_err(CmdError::Failed)?
            } else {
                return Err(CmdError::Params("`path` or `data` required".into()));
            };
            wordcraft_geom::picture::probe(&bytes).map_err(|e| match e {
                wordcraft_geom::picture::Refusal::Unsupported(why) => CmdError::Failed(why.into()),
                _ => CmdError::Failed("not a supported image".into()),
            })?;
            let key = s.doc.add_media(bytes.clone(), crate::cmd::insert::image_ext(&bytes));
            with_obj(s, |o| {
                if let InlineObject::Image { media, .. } = o {
                    *media = key.clone();
                }
            })
        })
        .params(r#"{"path"?: string, "data"?: base64}"#)
        .when(has_picture),
        CommandSpec::new("picture.style", "Picture Styles", "Picture Format › Picture Styles", |s, v| {
            // A style replaces the picture's style and border, as Word's gallery does (#2052).
            let style = p::str(v, "style").unwrap_or("simpleFrame").to_string();
            relook(s, move |l| {
                l.style = Some(style);
                l.border = None;
            })
        })
        .params(r#"{"style": "simpleFrame|thickFrame|rounded|softEdge|shadow"} (replaces the picture's style and border)"#)
        .when(has_picture),
        CommandSpec::new("picture.border", "Picture Border", "Picture Format › Picture Styles", |s, v| {
            let c = p::str(v, "color").and_then(Rgb::parse).unwrap_or(Rgb::BLACK);
            let w = p::u64(v, "width").unwrap_or(4).clamp(1, 200) as u32;
            // Replaces the picture's border, keeping its style (#2052).
            relook(s, move |l| l.border = Some((c, w)))
        })
        .params(r#"{"color"?: "RRGGBB", "width"?: px} (replaces the picture's border)"#)
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
            let sect = crate::cmd::page::sect(s);
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
            let tw = crate::cmd::page::sect(s).text_width();
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
            .params(r#"{"width"?: pt, "height"?: pt, "x"?: pt, "y"?: pt, "page"?: n, "to"?: Pos}  (x/y: the top-left on page `page` (0-based, default its page); moving an inline object by x/y floats it. to: an In Line with Text object moves to that text position and stays inline, as a drag does)"#)
            .when(has_movable),
        CommandSpec::new("arrange.copyTo", "Copy to", "Layout › Arrange", copy_to)
            .params(r#"{"x": pt, "y": pt, "page"?: n} | {"to": Pos}  (Ctrl+drag: a copy of the selected object lands with its top-left there, or an In Line with Text one at the text position `to`, and is selected; the original and the clipboard stay as they were)"#)
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
        CommandSpec::new("shape.outline", "Shape Outline", "Shape Format › Shape Styles", |s, v| {
            let c = p::str(v, "color").and_then(Rgb::parse);
            let w = p::f32(v, "width").map(|w| w.clamp(0.0, 72.0));
            with_obj(s, |o| {
                if let InlineObject::Shape { stroke, stroke_width, extra, .. } = o {
                    *stroke = c;
                    // A colour chosen here replaces an outline gradient from a file.
                    extra.outline_fill = None;
                    if let Some(w) = w {
                        *stroke_width = w;
                    }
                } else if let Some(style) = crate::cmd::canvas::style_mut(o) {
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
        CommandSpec::new("shape.editText", "Edit Text", "Shape Format › Text", edit_shape_text)
            .params(r#"{"pos"?: Pos}  (the shape or text box at `pos`, else the selected one: the caret goes into its text, a shape with none gets an empty centred paragraph; not for lines and connectors)"#)
            // Entering text that exists only moves the caret: no undo step, the document isn't
            // edited. Giving a shape its first (empty) story takes its own checkpoint below.
            .pure(),
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
