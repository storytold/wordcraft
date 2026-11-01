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

/// The body's top-level paragraphs, an object's character shown as ▣.
fn body_text(s: &Session) -> String {
    (0..2)
        .filter_map(|i| s.doc.para(StoryRef::Body, &wordcraft_doc::Path::top(i)))
        .map(|p| p.text.replace(wordcraft_doc::para::OBJ, "▣"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Two paragraphs, the second starting with an inline picture (bold, so its formatting can be
/// followed), selected.
fn inline_picture_session() -> Session {
    let mut s = Session::new(wordcraft_doc::Document::new());
    s.run("text.insert", &json!({"text": "First paragraph."})).unwrap();
    s.run("text.newParagraph", &json!({})).unwrap();
    s.run("format.bold", &json!({})).unwrap();
    let data = super::super::insert::base64_encode(&png(20, 10));
    s.run("insert.picture", &json!({"data": data, "width": 100})).unwrap();
    s.run("format.bold", &json!({})).unwrap();
    s.run("text.insert", &json!({"text": " second paragraph."})).unwrap();
    let pic = Pos::body(1, 0);
    s.run("select.range", &json!({"anchor": pic, "focus": Pos { off: pic.off + wordcraft_doc::para::OBJ.len_utf8(), ..pic.clone() }})).unwrap();
    assert!(selected(&s).is_some_and(|(_, o)| !o.is_floating()));
    s
}

/// #2234: dragging an In Line with Text picture into the text (`to`) moves it there and it stays
/// inline, with its formatting, selected, in one undo step; it never floats.
#[test]
fn an_inline_picture_dragged_into_the_text_stays_inline() {
    let mut s = inline_picture_session();
    let text = body_text;
    assert_eq!(text(&s), "First paragraph.\n▣ second paragraph.");
    // Into the first paragraph, after "First".
    let r = s.run("arrange.bounds", &json!({"to": Pos::body(0, 5)})).unwrap();
    assert_eq!(text(&s), "First▣ paragraph.\n second paragraph.");
    assert_eq!(r["object"]["float"]["wrap"], "inline", "{r}");
    let (pos, obj) = selected(&s).expect("still selected");
    assert_eq!(pos, Pos::body(0, 5));
    assert!(!obj.is_floating());
    assert!(s.doc.para_at(&pos).unwrap().props_of_char(pos.off).bold == Some(true), "keeps its run's formatting");
    // Later in its own paragraph (after "paragraph", counted with it still there): the offset
    // accounts for its own character leaving.
    s.run("arrange.bounds", &json!({"to": Pos::body(0, 18)})).unwrap();
    assert_eq!(text(&s), "First paragraph▣.\n second paragraph.");
    // Each move is one undo step.
    s.run("edit.undo", &json!({})).unwrap();
    assert_eq!(text(&s), "First▣ paragraph.\n second paragraph.");
    s.run("edit.undo", &json!({})).unwrap();
    assert_eq!(text(&s), "First paragraph.\n▣ second paragraph.");
    // Dropped on itself (either side): nothing changes.
    let here = selected(&s).unwrap().0;
    s.run("arrange.bounds", &json!({"to": here})).unwrap();
    s.run("arrange.bounds", &json!({"to": Pos { off: here.off + wordcraft_doc::para::OBJ.len_utf8(), ..here.clone() }})).unwrap();
    assert_eq!(text(&s), "First paragraph.\n▣ second paragraph.");
    assert_eq!(selected(&s).unwrap().0, here);
}

/// `to` is only for In Line with Text objects, in their own text; bad targets change nothing.
#[test]
fn moving_into_the_text_refuses_bad_targets() {
    let mut s = inline_picture_session();
    let before = body_text(&s);
    for bad in [
        json!({"to": "junk"}),
        json!({"to": {"story": "body", "path": [9], "off": 0}}),
        json!({"to": Pos::body(0, 0), "x": 10, "y": 10}),
        json!({"to": {"story": {"part": 99}, "path": [0], "off": 0}}),
    ] {
        assert!(s.run("arrange.bounds", &bad).is_err(), "{bad}");
        assert!(s.run("arrange.copyTo", &bad).is_err(), "{bad}");
    }
    // A huge offset is clamped to the paragraph's end.
    s.run("arrange.bounds", &json!({"to": {"story": "body", "path": [0], "off": 999_999}})).unwrap();
    assert_eq!(body_text(&s), "First paragraph.▣\n second paragraph.");
    assert_ne!(body_text(&s), before);
    // A floating object is moved by x/y, not into the text.
    s.run("arrange.bounds", &json!({"page": 0, "x": 200, "y": 300})).unwrap();
    assert!(s.run("arrange.bounds", &json!({"to": Pos::body(1, 0)})).is_err());
}

/// Ctrl+drag of an inline picture into the text: a copy lands there, inline and selected; the
/// original stays.
#[test]
fn copying_an_inline_picture_into_the_text() {
    let mut s = inline_picture_session();
    s.clipboard_text = "keep".into();
    s.run("arrange.copyTo", &json!({"to": Pos::body(0, 5)})).unwrap();
    assert_eq!(body_text(&s), "First▣ paragraph.\n▣ second paragraph.");
    let (pos, obj) = selected(&s).expect("the copy is selected");
    assert_eq!(pos, Pos::body(0, 5));
    assert!(!obj.is_floating());
    assert_eq!(s.clipboard_text, "keep");
    s.run("edit.undo", &json!({})).unwrap();
    assert_eq!(body_text(&s), "First paragraph.\n▣ second paragraph.");
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

/// #2019: Insert › Shapes adds each new shape beside the selected one, as Word draws it, so
/// shapes inserted in a row are all kept (the second used to replace the first, which stays
/// selected after its insert). Selected text stays too, and the new shape is selected.
#[test]
fn shapes_inserted_in_a_row_are_all_kept_and_keep_selected_text() {
    let objects = |s: &mut Session| s.run("arrange.selectionPane", &json!({})).unwrap().as_array().map(Vec::len);
    let mut s = Session::new(wordcraft_doc::Document::new());
    for kind in ["rectangle", "ellipse", "star"] {
        s.run("insert.shape", &json!({"kind": kind})).unwrap();
    }
    assert_eq!(objects(&mut s), Some(3));

    let mut s = Session::new(wordcraft_doc::Document::from_text("Kiln room"));
    s.run("select.text", &json!({"text": "Kiln"})).unwrap();
    s.run("insert.shape", &json!({"kind": "rectangle"})).unwrap();
    let text = s.doc.plain_text(StoryRef::Body);
    assert!(text.starts_with("Kiln") && text.ends_with(" room"), "{text:?}");
    assert_eq!(objects(&mut s), Some(1));
    let (a, b) = s.sel.ordered();
    let obj = wordcraft_doc::para::OBJ.len_utf8();
    assert_eq!((a.off, b.off), (4, 4 + obj), "the new shape, after the text, is selected");
}

/// Insert › Pictures does replace a selected picture, as Word does (checked in Word for
/// Windows while fixing #2019): only shapes are added beside the selection.
#[test]
fn an_inserted_picture_replaces_a_selected_picture() {
    let data = super::super::insert::base64_encode(&png(20, 10));
    let mut s = Session::new(wordcraft_doc::Document::new());
    s.run("insert.picture", &json!({"data": data})).unwrap();
    s.sel = crate::Selection { anchor: Pos::body(0, 0), focus: Pos::body(0, wordcraft_doc::para::OBJ.len_utf8()) };
    s.run("insert.picture", &json!({"data": data})).unwrap();
    assert_eq!(s.run("arrange.selectionPane", &json!({})).unwrap().as_array().map(Vec::len), Some(1));
}

/// A PNG whose header (valid CRC) declares `w` × `h` pixels, with one pixel of data.
fn header_only_png(w: u32, h: u32) -> Vec<u8> {
    let mut b = png(8, 8);
    b[16..20].copy_from_slice(&w.to_be_bytes());
    b[20..24].copy_from_slice(&h.to_be_bytes());
    let mut crc = 0xFFFF_FFFFu32;
    for &byte in &b[12..29] {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = if crc & 1 == 1 { 0xEDB8_8320 ^ (crc >> 1) } else { crc >> 1 };
        }
    }
    b[29..33].copy_from_slice(&(!crc).to_be_bytes());
    b
}

#[test]
fn picture_edits_refuse_an_over_budget_header_before_decoding() {
    // Inserting reads the size only; every edit then refuses it from the header (#3644, #3647).
    let mut s = Session::new(wordcraft_doc::Document::new());
    s.run("insert.picture", &json!({"data": super::super::insert::base64_encode(&header_only_png(20_000, 20_000))})).unwrap();
    let media = s.doc.media.len();
    for (id, v) in [
        ("picture.corrections", json!({"brightness": 10})),
        ("picture.color", json!({"mode": "sepia"})),
        ("picture.removeBackground", json!({})),
        ("picture.style", json!({"style": "shadow"})),
    ] {
        let e = s.run(id, &v).unwrap_err().to_string();
        assert!(e.contains("too large to edit"), "{id}: {e}");
    }
    assert_eq!(s.doc.media.len(), media, "nothing was added");
}

/// The Shadow style before #3647: the rectangle blurred with `image`'s Gaussian.
fn blurred_shadow(img: &image::DynamicImage) -> image::RgbaImage {
    let (w, h) = (img.width(), img.height());
    let off = (w.max(h) / 40).max(4);
    let mut out = image::RgbaImage::from_pixel(w + off * 2, h + off * 2, image::Rgba([0, 0, 0, 0]));
    let shadow = image::RgbaImage::from_pixel(w, h, image::Rgba([0, 0, 0, 90]));
    let mut padded = image::RgbaImage::from_pixel(w + off * 2, h + off * 2, image::Rgba([0, 0, 0, 0]));
    image::imageops::overlay(&mut padded, &shadow, (off + off / 2) as i64, (off + off / 2) as i64);
    let blurred = image::DynamicImage::ImageRgba8(padded).blur(off as f32 / 2.0).to_rgba8();
    image::imageops::overlay(&mut out, &blurred, 0, 0);
    image::imageops::overlay(&mut out, &img.to_rgba8(), 0, 0);
    out
}

/// #3647: the shadow drawn from two profiles looks like the blur it replaces.
#[test]
fn the_shadow_style_matches_the_blur_it_replaces() {
    for (w, h) in [(80, 40), (400, 220), (37, 300)] {
        let img = image::DynamicImage::ImageRgba8(image::RgbaImage::from_fn(w, h, |x, y| {
            image::Rgba([(x * 7) as u8, (y * 5) as u8, 90, if (x + y) % 3 == 0 { 255 } else { 120 }])
        }));
        let new = picture_style(img.clone(), "shadow").unwrap().to_rgba8();
        let old = blurred_shadow(&img);
        assert_eq!(new.dimensions(), old.dimensions());
        let worst = new.pixels().zip(old.pixels()).flat_map(|(a, b)| a.0.into_iter().zip(b.0).map(|(a, b)| a.abs_diff(b))).max();
        assert!(worst.is_some_and(|d| d <= 2), "{w} x {h}: {worst:?}");
    }
}

/// #3647: Remove Background fills a span at a time, so its stack stays near the border's seeds
/// instead of growing with the picture, and it clears the same region.
#[test]
fn remove_background_keeps_a_small_stack() {
    let mut uniform = image::RgbaImage::from_pixel(2000, 2000, image::Rgba([250, 250, 250, 255]));
    let most = clear_background(&mut uniform, 30.0);
    assert!(most <= 4 * (2000 + 2000) + 8, "{most}");
    assert!(uniform.pixels().all(|p| p.0[3] == 0));
    // A square ring keeps what it encloses: only background reachable from the border goes.
    let mut ring = image::RgbaImage::from_fn(60, 60, |x, y| {
        let d = (x as i32 - 30).abs().max((y as i32 - 30).abs());
        if (10..14).contains(&d) { image::Rgba([200, 20, 20, 255]) } else { image::Rgba([255, 255, 255, 255]) }
    });
    clear_background(&mut ring, 30.0);
    assert_eq!(ring.get_pixel(0, 0).0, [255, 255, 255, 0], "outside: cleared, colour kept");
    assert_eq!(ring.get_pixel(5, 30).0[3], 0);
    assert_eq!(ring.get_pixel(30, 18).0[3], 255, "the ring");
    assert_eq!(ring.get_pixel(30, 30).0[3], 255, "inside the ring");
    assert_eq!(clear_background(&mut image::RgbaImage::new(0, 0), 30.0), 0);
}

/// #3611: a style or border is drawn only when the picture it makes can still be drawn. On a
/// 1 × 2,000,000 picture Thick Frame asked for about 1.8 TB and aborted the process.
#[test]
fn picture_styles_and_borders_check_the_size_they_draw() {
    let mut thin = Vec::new();
    image::GrayImage::new(1, 2_000_000).write_to(&mut std::io::Cursor::new(&mut thin), image::ImageFormat::Png).unwrap();
    let mut s = Session::new(wordcraft_doc::Document::new());
    s.run("insert.picture", &json!({"data": super::super::insert::base64_encode(&thin)})).unwrap();
    let media = s.doc.media.len();
    for (id, v) in [
        ("picture.style", json!({"style": "thickFrame"})),
        ("picture.style", json!({"style": "shadow"})),
        ("picture.style", json!({})),
        ("picture.border", json!({"width": 200})),
    ] {
        let e = s.run(id, &v).unwrap_err().to_string();
        assert!(e.contains("too large to edit"), "{id} {v}: {e}");
    }
    assert_eq!(s.doc.media.len(), media, "nothing was drawn");
    assert_eq!(grown(u32::MAX, 1, 1), None);
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

/// A `sharpen` too small to be a normal blur width must not reach the blur, which asserts on it.
#[test]
fn a_tiny_sharpen_value_does_not_panic() {
    let mut s = Session::new(wordcraft_doc::Document::new());
    let data = super::super::insert::base64_encode(&png(20, 10));
    s.run("insert.picture", &json!({"data": data})).unwrap();
    for sharpen in [-1e-40_f64, -1e-45, -1e-38, 1e-40, -1e-30, 0.0, -0.5] {
        s.run("picture.corrections", &json!({"sharpen": sharpen})).unwrap_or_else(|e| panic!("sharpen {sharpen}: {e:?}"));
    }
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

/// #2052: a picture style picked after another replaces it (Thick Frame then Simple Frame is
/// only the simple frame), a border replaces the border and keeps the style, an artistic
/// effect replaces the effect, and transparency 50% then 10% is 10%, as in Word.
#[test]
fn picture_styles_borders_and_effects_replace_the_last_one() {
    let picture = || {
        let mut s = Session::new(wordcraft_doc::Document::new());
        s.run("insert.picture", &json!({"data": super::super::insert::base64_encode(&png(80, 40))})).unwrap();
        s
    };
    let bitmap = |s: &Session| {
        let (_, o) = selected(s).unwrap();
        let InlineObject::Image { media, .. } = o else { panic!("a picture") };
        image::load_from_memory(&s.doc.media.get(&media).cloned().unwrap()).unwrap().to_rgba8()
    };
    let after = |cmds: &[(&str, Value)]| {
        let mut s = picture();
        for (id, v) in cmds {
            s.run(id, v).unwrap();
        }
        bitmap(&s)
    };
    let (thick, simple) = (json!({"style": "thickFrame"}), json!({"style": "simpleFrame"}));
    assert_eq!(after(&[("picture.style", thick.clone()), ("picture.style", simple.clone())]), after(&[("picture.style", simple.clone())]));
    let (wide, thin) = (json!({"width": 8}), json!({"width": 2}));
    assert_eq!(
        after(&[("picture.style", thick.clone()), ("picture.border", wide.clone()), ("picture.border", thin.clone())]),
        after(&[("picture.style", thick.clone()), ("picture.border", thin.clone())]),
        "a border replaces the border and keeps the style"
    );
    assert_eq!(
        after(&[("picture.border", wide), ("picture.style", simple.clone())]),
        after(&[("picture.style", simple)]),
        "a style replaces the border too"
    );
    assert_eq!(
        after(&[("picture.effects", json!({"effect": "invert"})), ("picture.effects", json!({"effect": "posterize"}))]),
        after(&[("picture.effects", json!({"effect": "posterize"}))])
    );
    assert_eq!(after(&[("picture.effects", json!({"effect": "invert"})), ("picture.effects", json!({"effect": "none"}))]), bitmap(&picture()));
    let fifty_then_ten = after(&[("picture.transparency", json!({"percent": 50})), ("picture.transparency", json!({"percent": 10}))]);
    assert!(fifty_then_ten.pixels().all(|p| p.0[3] == 229), "255 * 0.9");

    // Undo then a new edit reusing the drawn bitmap's media key starts from what is there.
    let mut s = picture();
    s.run("picture.style", &thick).unwrap();
    s.run("edit.undo", &json!({})).unwrap();
    s.run("picture.effects", &json!({"effect": "invert"})).unwrap();
    s.run("picture.border", &json!({"width": 2})).unwrap();
    let (w, h) = bitmap(&s).dimensions();
    assert_eq!((w, h), (84, 44), "the effect and a 2 px border, no thick frame from before the undo");
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

/// Select the objects at `at` in the list of all objects (the first as the selection, the rest added).
fn pick_objects(s: &mut Session, at: &[usize]) {
    let list = all_objects(s);
    let first = &list[at[0]].0;
    s.sel = Selection { anchor: first.clone(), focus: Pos { off: first.off + wordcraft_doc::para::OBJ.len_utf8(), ..first.clone() } };
    s.also_selected = at[1..].iter().map(|i| list[*i].0.clone()).collect();
}

#[test]
fn group_refuses_more_than_the_child_cap() {
    let cap = wordcraft_doc::para::MAX_GROUP_CHILDREN;
    let mut s = Session::new(wordcraft_doc::Document::new());
    s.run("document.setText", &json!({"text": "one"})).unwrap();
    s.sel = Selection::caret(s.doc.start_of(StoryRef::Body));
    for _ in 0..=cap {
        s.run("insert.shape", &json!({"kind": "rectangle", "width": 10, "height": 10})).unwrap();
        s.run("arrange.bounds", &json!({"page": 0, "x": 100, "y": 100})).unwrap();
        s.run("select.collapse", &json!({"end": true})).unwrap();
    }
    let n = all_objects(&s).len();
    assert_eq!(n, cap + 1);

    // Flat: every shape picked.
    pick_objects(&mut s, &(0..n).collect::<Vec<_>>());
    assert_eq!(picked_objects(&s).len(), n);
    let before = s.doc.clone();
    assert!(s.run("arrange.group", &json!({})).is_err());
    assert_eq!(s.doc, before);

    // Nested: a group of cap - 1 and a group of 2; grouping those two would hold cap + 1 members.
    pick_objects(&mut s, &(0..cap - 1).collect::<Vec<_>>());
    s.run("arrange.group", &json!({})).unwrap();
    let rest = all_objects(&s).len();
    assert_eq!(rest, 3);
    pick_objects(&mut s, &[1, 2]);
    s.run("arrange.group", &json!({})).unwrap();
    assert_eq!(all_objects(&s).len(), 2);
    pick_objects(&mut s, &[0, 1]);
    assert_eq!(picked_objects(&s).len(), 2);
    let before = s.doc.clone();
    assert!(s.run("arrange.group", &json!({})).is_err());
    assert_eq!(s.doc, before);
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

/// #2573: Size from the original. `original` alone gives the picture back its original size
/// (its pixels at 96 ppi, less what is cropped) and `scale` is a percentage of it; Compress
/// Pictures doesn't change what the original is. Only a picture has one.
#[test]
fn picture_size_counts_from_the_original() {
    let mut s = Session::new(wordcraft_doc::Document::new());
    let data = super::super::insert::base64_encode(&png(40, 20));
    s.run("insert.picture", &json!({"data": data})).unwrap();
    let size = |s: &Session| obj_size(&selected(s).unwrap().1);
    assert_eq!(size(&s), (30.0, 15.0), "40×20 px at 96 ppi");
    s.run("picture.size", &json!({"width": 300.0, "height": 100.0, "lockAspect": false})).unwrap();
    s.run("picture.compress", &json!({"maxPixels": 32})).unwrap();
    let Some((_, InlineObject::Image { media, .. })) = selected(&s) else { panic!("a picture") };
    assert_eq!(wordcraft_render::image_size(&s.doc.media[&media]), Some((32, 16)), "compressed");
    assert_eq!(original_size(&s, &media), Some(OriginalSize { w: 30.0, h: 15.0, pixels: Some((40, 20)) }));
    s.run("picture.size", &json!({"original": true})).unwrap();
    assert_eq!(size(&s), (30.0, 15.0));
    s.run("picture.size", &json!({"original": true, "scale": 250})).unwrap();
    assert_eq!(size(&s), (75.0, 37.5));
    // Half the width cropped away: half the original width is left.
    s.run("picture.crop", &json!({"left": 0.25, "right": 0.25})).unwrap();
    s.run("picture.size", &json!({"original": true})).unwrap();
    assert_eq!(size(&s), (15.0, 15.0));
    s.run("edit.undo", &json!({})).unwrap();
    assert_eq!(size(&s), (75.0, 37.5), "one undo step");
    s.run("insert.shape", &json!({"kind": "rectangle"})).unwrap();
    assert!(s.run("picture.size", &json!({"original": true})).is_err(), "a shape has no original size");
}

/// #2573: a size past 4000 pt (or under 1 pt) that keeps the proportions scales as a whole
/// instead of flattening against the limit; both sides given are clamped each on its own.
#[test]
fn picture_size_past_the_limit_keeps_the_proportions() {
    let mut s = Session::new(wordcraft_doc::Document::new());
    let data = super::super::insert::base64_encode(&png(40, 20));
    s.run("insert.picture", &json!({"data": data})).unwrap();
    let size = |s: &Session| obj_size(&selected(s).unwrap().1);
    s.run("picture.size", &json!({"width": 9000.0})).unwrap();
    assert_eq!(size(&s), (4000.0, 2000.0));
    s.run("picture.size", &json!({"scale": 1000})).unwrap();
    assert_eq!(size(&s), (4000.0, 2000.0));
    s.run("picture.size", &json!({"width": 9000.0, "height": 10.0})).unwrap();
    assert_eq!(size(&s), (4000.0, 10.0));
    s.run("picture.size", &json!({"height": 30.0, "lockAspect": false})).unwrap();
    assert_eq!(size(&s), (4000.0, 30.0));
    // Below 1 pt as well: a 1% scale of 50×200 pt is 1×4 pt, not 1×2.
    s.run("picture.size", &json!({"width": 50.0, "height": 200.0})).unwrap();
    s.run("picture.size", &json!({"scale": 1})).unwrap();
    assert_eq!(size(&s), (1.0, 4.0));
    // Too long and thin for one factor: side by side. NaN and infinity stay in range.
    assert_eq!(fit_size(0.5, 5000.0, true), (1.0, 4000.0));
    assert_eq!(fit_size(f32::NAN, 10.0, true), (1.0, 10.0));
    assert_eq!(fit_size(f32::INFINITY, 10.0, true), (4000.0, 10.0));
    // Hostile values never leave the 1–4000 pt range.
    for v in [json!({"scale": -50}), json!({"width": -1.0}), json!({"original": "yes", "scale": 1e30}), json!({"height": 1e30})] {
        s.run("picture.size", &v).unwrap();
        let (w, h) = size(&s);
        assert!((1.0..=4000.0).contains(&w) && (1.0..=4000.0).contains(&h), "{v}: {w}×{h}");
    }
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
    // Clamps: each side at most 0.95; hostile sizes are ignored or clamped, never applied raw.
    s.run("picture.crop", &json!({"left": 9.0, "right": -3.0, "width": "x", "height": -5.0})).unwrap();
    let (w, h, crop) = image(&s);
    assert_eq!(crop, [0.95, 0.0, 0.0, 0.0]);
    assert_eq!(w, 200.0);
    assert!(h >= 1.0, "{h}");
}

/// #3135: a picture can be cropped to any part of itself, not just its middle. A side goes to
/// 0.95 (5% stays, as the renderer draws), and the two sides of an axis keep their 5%.
#[test]
fn a_picture_can_be_cropped_to_any_part_of_itself() {
    let mut s = Session::new(wordcraft_doc::Document::new());
    let data = super::super::insert::base64_encode(&png(20, 10));
    s.run("insert.picture", &json!({"data": data})).unwrap();
    s.run("picture.size", &json!({"width": 200.0, "height": 100.0, "lockAspect": false})).unwrap();
    let crop = |s: &Session| match selected(s) {
        Some((_, InlineObject::Image { crop, .. })) => crop,
        other => panic!("expected image, got {other:?}"),
    };
    let left = |s: &Session| {
        let (_, o) = selected(s).unwrap();
        super::cropped_original_size(s, &o).unwrap().0
    };
    // 90% off the left keeps the rightmost tenth: the side is not held at 45%.
    s.run("picture.crop", &json!({"left": 0.9})).unwrap();
    assert_eq!(crop(&s), [0.9, 0.0, 0.0, 0.0]);
    assert!((left(&s) - 1.5).abs() < 1e-3, "a tenth of the 15 pt original: {}", left(&s));
    // With the left at 0.9 the right can take only 5% more, and what the crop leaves of the
    // original follows the crop down to that 5%.
    s.run("picture.crop", &json!({"left": 0.9, "right": 0.9})).unwrap();
    let c = crop(&s);
    assert!((c[0] - 0.9).abs() < 1e-6 && (c[2] - 0.05).abs() < 1e-5 && c[1] == 0.0 && c[3] == 0.0, "{c:?}");
    assert!((left(&s) - 0.75).abs() < 1e-3, "5% of the 15 pt original: {}", left(&s));
    // A single side past the cap stops at 0.95.
    s.run("picture.crop", &json!({"right": 4.0})).unwrap();
    assert_eq!(crop(&s), [0.0, 0.0, 0.95, 0.0]);
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
    assert_eq!(l.pages[0].items.iter().filter(|it| matches!(it, wordcraft_layout::Placed::Object { text_box: Some(t), .. } if *t == a)).count(), 2);
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
        s.run("text.insert", &json!({"text": format!("Paragraph {i} has words enough to run over two or three lines of a narrow column.")})).unwrap();
    }
    s.run("caret.docStart", &json!({})).unwrap();
    s.run("insert.shape", &json!({"kind": "rectangle"})).unwrap();
    s.run("arrange.bounds", &json!({"width": 80, "height": 60})).unwrap();
    let cols: Vec<(f32, f32)> = {
        let layout = s.layout();
        let mut v: Vec<(f32, f32)> = wordcraft_layout::hit::page_lines(&layout.pages[0], StoryRef::Body).iter().map(|l| (l.left, l.right)).collect();
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
    let boxes = s.doc.laid_out_sections()[pg.section].1.column_boxes();
    boxes.iter().map(|(x, w)| (pg.body.x + x, pg.body.x + x + w)).collect()
}

/// #1836 review: under Book fold the columns are those of the half page, so a drop in the
/// second column anchors to a paragraph that starts there, not the first column's.
#[test]
fn book_fold_anchors_in_the_half_page_columns() {
    let mut s = two_column_doc(40);
    s.run("layout.multiplePages", &json!({"mode": "bookFold"})).unwrap();
    s.touch();
    let cols = page_columns(&mut s, 0);
    assert!(cols[1].0 < 300.0, "the second column starts on the half page: {cols:?}");
    let layout = s.layout();
    let (path, top) = layout.pages[0]
        .items
        .iter()
        .find_map(|it| match it {
            wordcraft_layout::Placed::Lines { path, l0: 0, x, y, .. } if path.depth() == 0 && *x >= cols[1].0 - 0.5 => Some((path.clone(), *y)),
            _ => None,
        })
        .unwrap();
    let mid = (cols[1].0 + cols[1].1) / 2.0;
    assert_eq!(anchor_paragraph(&s.doc, &layout, 0, (mid, top - 1.0), StoryRef::Body), Some(path));
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

/// A Word file whose only paragraph holds a group of a rectangle and Word's own ink (with its
/// picture fallback).
fn group_with_word_ink() -> Vec<u8> {
    use std::io::Write;
    let ns = r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:pic="http://schemas.openxmlformats.org/drawingml/2006/picture" xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006" xmlns:wps="http://schemas.microsoft.com/office/word/2010/wordprocessingShape" xmlns:wpg="http://schemas.microsoft.com/office/word/2010/wordprocessingGroup" xmlns:w14="http://schemas.microsoft.com/office/word/2010/wordml" xmlns:wpi="http://schemas.microsoft.com/office/word/2010/wordprocessingInk""#;
    let rect = r#"<wps:wsp><wps:cNvPr id="2" name="Box"/><wps:cNvSpPr/><wps:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="914400" cy="914400"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom></wps:spPr><wps:bodyPr/></wps:wsp>"#;
    let ink = r#"<mc:AlternateContent><mc:Choice Requires="wpi"><w14:contentPart bwMode="auto" r:id="rInk"><w14:nvContentPartPr><w14:cNvPr id="3" name="Ink 3"/><w14:cNvContentPartPr/></w14:nvContentPartPr><w14:xfrm><a:off x="914400" y="0"/><a:ext cx="914400" cy="457200"/></w14:xfrm></w14:contentPart></mc:Choice><mc:Fallback><pic:pic><pic:nvPicPr><pic:cNvPr id="3" name="Ink 3"/><pic:cNvPicPr/></pic:nvPicPr><pic:blipFill><a:blip r:embed="rPreview"/></pic:blipFill><pic:spPr><a:xfrm><a:off x="914400" y="0"/><a:ext cx="914400" cy="457200"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom></pic:spPr></pic:pic></mc:Fallback></mc:AlternateContent>"#;
    let group = format!(
        r#"<w:drawing><wp:anchor behindDoc="0" locked="0" layoutInCell="1" allowOverlap="1" relativeHeight="1" simplePos="0" distT="0" distB="0" distL="0" distR="0"><wp:simplePos x="0" y="0"/><wp:positionH relativeFrom="column"><wp:posOffset>0</wp:posOffset></wp:positionH><wp:positionV relativeFrom="paragraph"><wp:posOffset>0</wp:posOffset></wp:positionV><wp:extent cx="1828800" cy="914400"/><wp:wrapNone/><wp:docPr id="1" name="Group 1"/><a:graphic><a:graphicData uri="http://schemas.microsoft.com/office/word/2010/wordprocessingGroup"><wpg:wgp><wpg:cNvGrpSpPr/><wpg:grpSpPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="1828800" cy="914400"/><a:chOff x="0" y="0"/><a:chExt cx="1828800" cy="914400"/></a:xfrm></wpg:grpSpPr>{rect}{ink}</wpg:wgp></a:graphicData></a:graphic></wp:anchor></w:drawing>"#
    );
    let rel = |id: &str, ty: &str, target: &str| {
        format!(r#"<Relationship Id="{id}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/{ty}" Target="{target}"/>"#)
    };
    let parts = [
        ("_rels/.rels", format!(r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">{}</Relationships>"#, rel("rId1", "officeDocument", "word/document.xml"))),
        ("[Content_Types].xml", r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Override PartName="/word/ink/ink1.xml" ContentType="application/inkml+xml"/><Default Extension="png" ContentType="image/png"/></Types>"#.to_string()),
        ("word/document.xml", format!(r#"<w:document {ns}><w:body><w:p><w:r>{group}</w:r></w:p></w:body></w:document>"#)),
        (
            "word/_rels/document.xml.rels",
            format!(r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">{}{}</Relationships>"#, rel("rInk", "customXml", "ink/ink1.xml"), rel("rPreview", "image", "media/preview.png")),
        ),
        ("word/ink/ink1.xml", r#"<ink xmlns="http://www.w3.org/2003/InkML"><trace>0 0, 1 1</trace></ink>"#.to_string()),
    ];
    let mut zw = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, xml) in parts {
        zw.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
        zw.write_all(xml.as_bytes()).unwrap();
    }
    zw.start_file("word/media/preview.png", zip::write::SimpleFileOptions::default()).unwrap();
    zw.write_all(&png(8, 4)).unwrap();
    zw.finish().unwrap().into_inner()
}

fn ink_members(doc: &wordcraft_doc::Document) -> usize {
    let groups: Vec<_> = doc.body.iter().filter_map(|b| b.as_para()).flat_map(|p| p.objects.iter()).collect();
    let nested = groups
        .iter()
        .filter_map(|o| match o {
            InlineObject::Group { children, .. } => Some(children.iter().filter(|c| matches!(c.obj, InlineObject::Graphic { .. })).count()),
            _ => None,
        })
        .sum::<usize>();
    let alone = groups.iter().filter(|o| matches!(o, InlineObject::Graphic { .. })).count();
    nested + alone
}

/// Gap 2 review: a group holding Word's own ink can't be ungrouped (its ink can't stand alone
/// yet), so open → Ungroup → save → reopen keeps the ink in the group with its part.
#[test]
fn ungroup_refuses_a_group_holding_word_ink_and_the_ink_survives_saving() {
    use std::io::Read;
    let mut s = Session::new(wordcraft_docx::read(&group_with_word_ink()).unwrap());
    assert_eq!(ink_members(&s.doc), 1, "the fixture's ink is read into the group");
    let pos = Pos::body(0, 0);
    s.sel = Selection { anchor: pos.clone(), focus: Pos { off: pos.off + wordcraft_doc::para::OBJ.len_utf8(), ..pos } };
    assert!(matches!(selected(&s), Some((_, InlineObject::Group { .. }))));
    let before = wordcraft_docx::write(&s.doc).unwrap();
    assert!(has_group(&s).is_some(), "Ungroup is greyed out");
    let err = s.run("arrange.ungroup", &json!({})).unwrap_err();
    assert!(matches!(err, CmdError::Disabled(_)), "{err:?}");
    let err = ungroup(&mut s, &json!({})).unwrap_err();
    assert!(matches!(err, CmdError::Disabled(_)), "{err:?}");
    let bytes = wordcraft_docx::write(&s.doc).unwrap();
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes.clone())).unwrap();
    let mut xml = String::new();
    z.by_name("word/document.xml").unwrap().read_to_string(&mut xml).unwrap();
    assert_eq!(xml.matches("<w14:contentPart").count(), 1, "{xml}");
    assert!(xml.contains("<mc:Fallback>"), "{xml}");
    assert_eq!(bytes, before, "the refused ungroup leaves the document unchanged");
    let names: Vec<String> = z.file_names().map(String::from).collect();
    assert_eq!(names.iter().filter(|n| n.starts_with("word/ink/")).count(), 1, "{names:?}");
    let mut rels = String::new();
    z.by_name("word/_rels/document.xml.rels").unwrap().read_to_string(&mut rels).unwrap();
    assert_eq!(rels.matches("/customXml\"").count(), 1, "one ink relationship: {rels}");
    let back = wordcraft_docx::read(&bytes).unwrap();
    assert_eq!(ink_members(&back), 1);
}

/// Review: only Word's ink pins a group; a group of charts or diagrams ungroups as before.
#[test]
fn only_ink_members_pin_a_group() {
    let s = Session::new(wordcraft_docx::read(&group_with_word_ink()).unwrap());
    let pos = Pos::body(0, 0);
    let mut s = s;
    s.sel = Selection { anchor: pos.clone(), focus: Pos { off: pos.off + wordcraft_doc::para::OBJ.len_utf8(), ..pos } };
    let Some((_, mut group)) = selected(&s) else { panic!("no group") };
    assert!(holds_ink(&group));
    let InlineObject::Group { children, .. } = &mut group else { panic!("not a group") };
    for c in children.iter_mut() {
        if let InlineObject::Graphic { graphic, .. } = &mut c.obj {
            std::sync::Arc::make_mut(graphic).kind = GraphicKind::Chart;
        }
    }
    assert!(!holds_ink(&group), "a group of charts is not pinned");
}
