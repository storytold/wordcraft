//! Floating pictures (ODF 1.3 §9.2 frames anchored to a paragraph, character or page, §20.3xx
//! `style:wrap`, `style:horizontal-pos/-rel`, `style:vertical-pos/-rel`): placed and wrapped as
//! the file says, and written back the same way.

use crate::odt::tests::{NS_T, zip_of};
use crate::odt::{export, import};
use wordcraft_doc::Document;
use wordcraft_doc::para::{Anchor, Float, FloatAlign, InlineObject, Wrap, WrapText};

const NS_D: &str = r#"xmlns:draw="urn:oasis:names:tc:opendocument:xmlns:drawing:1.0" xmlns:svg="urn:oasis:names:tc:opendocument:xmlns:svg-compatible:1.0" xmlns:xlink="http://www.w3.org/1999/xlink""#;

fn open(graphic: &str, frame: &str) -> Document {
    let png = [b"\x89PNG\r\n\x1a\n".as_slice(), &[7u8; 64]].concat();
    let content = format!(
        r#"<office:document-content {NS_T} {NS_D}><office:automatic-styles><style:style style:name="fr1" style:family="graphic"><style:graphic-properties {graphic}/></style:style></office:automatic-styles><office:body><office:text><text:p><draw:frame draw:style-name="fr1" svg:width="2in" svg:height="1in" {frame}><draw:image xlink:href="Pictures/a.png"/></draw:frame>Text beside.</text:p></office:text></office:body></office:document-content>"#
    );
    crate::odt::import(&zip_of(&[("content.xml", content.as_bytes()), ("Pictures/a.png", &png)])).unwrap()
}

fn float(d: &Document) -> Float {
    d.body[0]
        .as_para()
        .unwrap()
        .objects
        .iter()
        .find_map(|o| if let InlineObject::Image { float, .. } = o { Some(float.clone()) } else { None })
        .expect("a picture")
}

#[test]
fn a_paragraph_anchored_frame_floats_with_text_around_it() {
    let f = float(&open(
        r#"style:wrap="parallel" style:horizontal-pos="right" style:horizontal-rel="paragraph" style:vertical-pos="top" style:vertical-rel="paragraph" fo:margin-left="0.1in" fo:margin-right="0.1in" fo:margin-top="0.05in" fo:margin-bottom="0.05in""#,
        r#"text:anchor-type="paragraph""#,
    ));
    assert_eq!((f.wrap, f.wrap_text), (Wrap::Square, WrapText::BothSides));
    assert_eq!((f.h_rel, f.h_align), (Anchor::Paragraph, Some(FloatAlign::End)));
    assert_eq!((f.v_rel, f.v_align), (Anchor::Paragraph, Some(FloatAlign::Start)));
    assert!((f.dist - 7.2).abs() < 0.01 && (f.dist_top - 3.6).abs() < 0.01, "{f:?}");
}

#[test]
fn wrap_modes_and_offsets() {
    let f = float(&open(
        r#"style:wrap="none" style:horizontal-pos="from-left" style:vertical-pos="from-top""#,
        r#"text:anchor-type="paragraph" svg:x="1in" svg:y="0.5in""#,
    ));
    assert_eq!(f.wrap, Wrap::TopAndBottom);
    assert_eq!((f.h_align, f.x, f.v_align, f.y), (None, 72.0, None, 36.0));
    let f = float(&open(r#"style:wrap="run-through" style:run-through="background""#, r#"text:anchor-type="page""#));
    assert_eq!((f.wrap, f.h_rel, f.v_rel), (Wrap::BehindText, Anchor::Page, Anchor::Page));
    assert_eq!(float(&open(r#"style:wrap="run-through""#, r#"text:anchor-type="char""#)).wrap, Wrap::InFrontOfText);
    assert_eq!(float(&open(r#"style:wrap="left""#, r#"text:anchor-type="paragraph""#)).wrap_text, WrapText::Left);
    assert_eq!(float(&open(r#"style:wrap="dynamic""#, r#"text:anchor-type="paragraph""#)).wrap_text, WrapText::Largest);
    // In line with the text.
    assert_eq!(float(&open("", r#"text:anchor-type="as-char""#)).wrap, Wrap::Inline);
}

#[test]
fn an_inline_frame_placed_against_the_text_is_raised_from_its_size() {
    // LibreOffice's style templates: a logo centred on the text (`middle` relative to `text`)
    // stands with its middle at the text's (measured: half the text's ascent less its descent
    // above the baseline), in the paragraph's 12 pt text.
    let pic = {
        let img = image::RgbaImage::from_fn(4, 4, |_, _| image::Rgba([10, 20, 30, 255]));
        let mut png = Vec::new();
        img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
        png
    };
    let raise = |vpos: &str, rel: &str| {
        let content = format!(
            r#"<office:document-content {NS} {NS_D}><office:automatic-styles><style:style style:name="P" style:family="paragraph"><style:text-properties fo:font-size="12pt"/></style:style><style:style style:name="g1" style:family="graphic"><style:graphic-properties style:vertical-pos="{vpos}" style:vertical-rel="{rel}"/></style:style></office:automatic-styles><office:body><office:text><text:p text:style-name="P"><draw:frame draw:style-name="g1" text:anchor-type="as-char" svg:width="1in" svg:height="1in"><draw:image xlink:href="Pictures/p.png"/></draw:frame>Title</text:p></office:text></office:body></office:document-content>"#,
            NS = NS_T
        );
        let doc = import(&zip_of(&[("content.xml", content.as_bytes()), ("Pictures/p.png", &pic)])).unwrap();
        let p = doc.body.iter().find_map(|b| b.as_para()).unwrap();
        let (off, _) = p.objects_with_offsets().next().unwrap();
        p.props_at(off + wordcraft_doc::para::OBJ.len_utf8()).position.map_or(0.0, |v| (v * 10.0).round() / 10.0)
    };
    // (0.891 - 0.216) / 2 × 12 − 36.
    assert_eq!(raise("middle", "text"), -32.0);
    assert_eq!(raise("middle", "char"), -32.0);
    assert_eq!(raise("top", "text"), -61.3);
    assert_eq!(raise("bottom", "text"), -2.6);
    // Against the line: on the baseline.
    assert_eq!(raise("middle", "line"), 0.0);
}

#[test]
fn an_inline_picture_is_saved_standing_on_the_baseline() {
    // LibreOffice puts an in-line frame without placement at the line's top (measured): a
    // picture standing on the baseline, as in Word, says so.
    let d = float_doc_inline();
    let out = crate::odt::tests::content_of(&crate::odt::tests::export_model(&d).unwrap());
    assert!(out.contains(r#"style:vertical-pos="top" style:vertical-rel="baseline""#), "{out}");
}

/// A document with one picture in line with its text.
fn float_doc_inline() -> Document {
    open("", r#"text:anchor-type="as-char""#)
}

#[test]
fn a_picture_alone_in_its_paragraph_keeps_its_raise_and_the_paragraphs_size() {
    // LibreOffice's elegant agenda: a rule raised 0.2 in above the baseline in a 9 pt paragraph.
    // Its parent style's `vertical-rel="paragraph"` is no in-line frame's (the standard doesn't
    // define it): it stands on the baseline. Placed against the baseline, it is raised.
    let pic = {
        let img = image::RgbaImage::from_fn(4, 4, |_, _| image::Rgba([10, 20, 30, 255]));
        let mut png = Vec::new();
        img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
        png
    };
    let content = format!(
        r#"<office:document-content {NS} xmlns:draw="urn:oasis:names:tc:opendocument:xmlns:drawing:1.0" xmlns:svg="urn:oasis:names:tc:opendocument:xmlns:svg-compatible:1.0" xmlns:xlink="http://www.w3.org/1999/xlink"><office:automatic-styles><style:style style:name="P2" style:family="paragraph"><style:text-properties fo:font-size="9pt"/></style:style><style:style style:name="g1" style:family="graphic" style:parent-style-name="G"><style:graphic-properties style:vertical-pos="from-top"/></style:style></office:automatic-styles><office:body><office:text><text:p text:style-name="P2"><draw:frame draw:style-name="g1" text:anchor-type="as-char" svg:y="-0.2in" svg:width="6in" svg:height="0.05in"><draw:image xlink:href="Pictures/p.png"/></draw:frame></text:p></office:text></office:body></office:document-content>"#,
        NS = crate::odt::tests::NS_T
    );
    let styles = format!(
        r#"<office:document-styles {NS} xmlns:draw="urn:oasis:names:tc:opendocument:xmlns:drawing:1.0" xmlns:svg="urn:oasis:names:tc:opendocument:xmlns:svg-compatible:1.0"><office:styles><style:style style:name="G" style:family="graphic"><style:graphic-properties text:anchor-type="paragraph" style:vertical-pos="top" style:vertical-rel="paragraph"/></style:style></office:styles></office:document-styles>"#,
        NS = crate::odt::tests::NS_T
    );
    let pkg = crate::odt::tests::zip_of(&[("content.xml", content.as_bytes()), ("styles.xml", styles.as_bytes()), ("Pictures/p.png", &pic)]);
    let look = |doc: &Document| {
        let p = doc.body.iter().find_map(|b| b.as_para()).unwrap();
        let (off, _) = p.objects_with_offsets().next().unwrap();
        let props = p.props_at(off + wordcraft_doc::para::OBJ.len_utf8());
        (props.position.map(|v| (v * 10.0).round() / 10.0), props.size, p.mark.size)
    };
    let doc = crate::odt::import(&pkg).unwrap();
    assert_eq!(look(&doc), (None, Some(9.0), Some(9.0)));
    let baseline = styles.replace(r#"style:vertical-rel="paragraph""#, r#"style:vertical-rel="baseline""#);
    let pkg = crate::odt::tests::zip_of(&[("content.xml", content.as_bytes()), ("styles.xml", baseline.as_bytes()), ("Pictures/p.png", &pic)]);
    let doc = crate::odt::import(&pkg).unwrap();
    assert_eq!(look(&doc), (Some(10.8), Some(9.0), Some(9.0)));
    // Saved, it says so in terms every reader takes the same way.
    let again = crate::odt::import(&crate::odt::tests::export_model(&doc).unwrap()).unwrap();
    assert_eq!(look(&again), (Some(10.8), Some(9.0), Some(9.0)));
}

#[test]
fn a_page_anchored_frames_paragraph_relations_are_its_text_area() {
    // LibreOffice's letter templates: a page-anchored frame "from the paragraph" or "from the
    // paragraph's content" (combinations the standard doesn't define) is placed from the page's
    // text area.
    let f = float(&open(
        r#"style:wrap="run-through" style:horizontal-pos="from-left" style:horizontal-rel="paragraph" style:vertical-pos="from-top" style:vertical-rel="paragraph""#,
        r#"text:anchor-type="page" svg:x="2cm" svg:y="2cm""#,
    ));
    assert_eq!((f.h_rel, f.v_rel), (Anchor::Margin, Anchor::Margin));
    let f = float(&open(
        r#"style:wrap="run-through" style:horizontal-pos="from-left" style:horizontal-rel="paragraph-content" style:vertical-rel="char""#,
        r#"text:anchor-type="page" svg:x="2cm""#,
    ));
    assert_eq!((f.h_rel, f.v_rel), (Anchor::Margin, Anchor::Margin));
    // Anchored to a paragraph, they stay the paragraph's.
    let f = float(&open(r#"style:horizontal-rel="paragraph-content" style:vertical-rel="char""#, r#"text:anchor-type="char""#));
    assert_eq!((f.h_rel, f.v_rel), (Anchor::Column, Anchor::Line));
}

#[test]
fn a_frames_style_gives_the_anchor_and_place_it_leaves_unsaid() {
    // LibreOffice's business card: the Labels style holds the card in line, Graphics puts the
    // credit at the top of the page; neither frame says so itself.
    let f = float(&open(r#"text:anchor-type="page" svg:x="1in" svg:y="0.5in" style:wrap="run-through""#, ""));
    assert_eq!((f.h_rel, f.v_rel, f.x, f.y), (Anchor::Page, Anchor::Page, 72.0, 36.0));
    assert_eq!(float(&open(r#"text:anchor-type="as-char""#, "")).wrap, Wrap::Inline);
    // The frame's own anchor and place win.
    let f = float(&open(r#"text:anchor-type="page" svg:x="1in""#, r#"text:anchor-type="paragraph" svg:x="2in""#));
    assert_eq!((f.h_rel, f.x), (Anchor::Paragraph, 144.0));
}

#[test]
fn floating_pictures_round_trip() {
    for (graphic, frame) in [
        (
            r#"style:wrap="parallel" style:horizontal-pos="right" style:horizontal-rel="paragraph" style:vertical-pos="top" style:vertical-rel="paragraph" fo:margin-left="0.1in" fo:margin-right="0.1in""#,
            r#"text:anchor-type="paragraph""#,
        ),
        (
            r#"style:wrap="none" style:horizontal-pos="from-left" style:horizontal-rel="page" style:vertical-pos="from-top" style:vertical-rel="page""#,
            r#"text:anchor-type="page" svg:x="1in" svg:y="2in""#,
        ),
        (
            r#"style:wrap="run-through" style:run-through="background" style:horizontal-pos="center" style:horizontal-rel="page-content""#,
            r#"text:anchor-type="paragraph""#,
        ),
        ("", r#"text:anchor-type="as-char""#),
    ] {
        let d = open(graphic, frame);
        let back = import(&export(&d).unwrap()).unwrap();
        let (a, b) = (float(&d), float(&back));
        assert_eq!(
            (a.wrap, a.wrap_text, a.h_rel, a.v_rel, a.h_align, a.v_align),
            (b.wrap, b.wrap_text, b.h_rel, b.v_rel, b.h_align, b.v_align),
            "{graphic}"
        );
        assert!((a.x - b.x).abs() < 0.01 && (a.y - b.y).abs() < 0.01 && (a.dist - b.dist).abs() < 0.01, "{graphic}: {a:?} {b:?}");
    }
}

#[test]
fn hostile_frame_positions_are_clamped() {
    let f = float(&open(r#"style:wrap="parallel" fo:margin-left="1e30in""#, r#"text:anchor-type="paragraph" svg:x="-1e30in" svg:y="NaNin""#));
    assert!(f.x.is_finite() && f.x.abs() <= 1584.0 && f.y.is_finite() && f.dist.is_finite() && f.dist <= 1584.0, "{f:?}");
}

#[test]
fn without_a_wrap_text_runs_on_both_sides() {
    // ODF 1.3 §20.400 gives no default; LibreOffice wraps text on both sides of a frame, shape or
    // group whose styles state none.
    let f = float(&open(r#"style:horizontal-pos="from-left""#, r#"text:anchor-type="paragraph" svg:x="1in""#));
    assert_eq!((f.wrap, f.wrap_text), (Wrap::Square, WrapText::BothSides));
    let f = float(&open(r#"style:wrap="biggest""#, r#"text:anchor-type="paragraph""#));
    assert_eq!((f.wrap, f.wrap_text), (Wrap::Square, WrapText::Largest));
}

#[test]
fn an_inline_frame_keeps_its_place_on_the_baseline() {
    // A frame in line with the text, hanging from the baseline (`from-top` 0, as LibreOffice
    // places a shape added by its API), centred on it or hanging from it (`bottom`): lowered by
    // its run's position, as Word lowers a picture, drawn so and written back.
    let pic = {
        let img = image::RgbaImage::from_fn(4, 4, |_, _| image::Rgba([10, 20, 30, 255]));
        let mut png = Vec::new();
        img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
        png
    };
    for (vpos, y, want) in [
        ("from-top", r#" svg:y="0in""#, -72.0),
        ("from-top", r#" svg:y="-0.5in""#, -36.0),
        ("middle", "", -36.0),
        ("bottom", "", -72.0),
        ("top", "", 0.0),
    ] {
        let content = format!(
            r#"<office:document-content {NS} xmlns:draw="urn:oasis:names:tc:opendocument:xmlns:drawing:1.0" xmlns:svg="urn:oasis:names:tc:opendocument:xmlns:svg-compatible:1.0" xmlns:xlink="http://www.w3.org/1999/xlink"><office:automatic-styles><style:style style:name="g1" style:family="graphic"><style:graphic-properties style:vertical-pos="{vpos}"/></style:style></office:automatic-styles><office:body><office:text><text:p>Line <draw:frame draw:style-name="g1" text:anchor-type="as-char" svg:width="1in" svg:height="1in"{y}><draw:image xlink:href="Pictures/p.png"/></draw:frame> text</text:p></office:text></office:body></office:document-content>"#,
            NS = crate::odt::tests::NS_T
        );
        let pkg = crate::odt::tests::zip_of(&[("content.xml", content.as_bytes()), ("Pictures/p.png", &pic)]);
        let doc = crate::odt::import(&pkg).unwrap();
        let p = doc.body.iter().find_map(|b| b.as_para()).unwrap();
        let (off, _) = p.objects_with_offsets().next().unwrap();
        let pos = p.props_at(off + wordcraft_doc::para::OBJ.len_utf8()).position.unwrap_or(0.0);
        assert_eq!((pos * 10.0).round() / 10.0, want, "{vpos}{y}");
        // Saved: a lowered one says how far its top is from the baseline.
        let saved = crate::odt::tests::content_of(&crate::odt::tests::export_model(&doc).unwrap());
        if want != 0.0 {
            assert!(saved.contains(r#"style:vertical-pos="from-top" style:vertical-rel="baseline""#), "{saved}");
            let top = -(72.0 + want) + 0.0;
            assert!(saved.contains(&format!(r#"svg:y="{}pt""#, top)), "{vpos}{y}: {saved}");
        }
        let again = crate::odt::import(&crate::odt::tests::export_model(&doc).unwrap()).unwrap();
        let p2 = again.body.iter().find_map(|b| b.as_para()).unwrap();
        let (off2, _) = p2.objects_with_offsets().next().unwrap();
        assert_eq!(
            p2.props_at(off2 + wordcraft_doc::para::OBJ.len_utf8()).position.map(|v| (v * 10.0).round() / 10.0),
            (want != 0.0).then_some(want)
        );
    }
}

#[test]
fn a_cropped_picture_keeps_its_crop() {
    // LibreOffice's crop (`fo:clip`: top, right, bottom, left, in lengths of the picture's own
    // size — 200 × 100 px is 150 × 75 pt): a quarter off its left and its bottom.
    let png = {
        let img = image::RgbaImage::from_fn(200, 100, |x, _| image::Rgba([if x < 100 { 200 } else { 20 }, 80, 160, 255]));
        let mut b = Vec::new();
        img.write_to(&mut std::io::Cursor::new(&mut b), image::ImageFormat::Png).unwrap();
        b
    };
    let content = format!(
        r#"<office:document-content {NS} xmlns:draw="urn:oasis:names:tc:opendocument:xmlns:drawing:1.0" xmlns:svg="urn:oasis:names:tc:opendocument:xmlns:svg-compatible:1.0" xmlns:xlink="http://www.w3.org/1999/xlink"><office:automatic-styles><style:style style:name="fr1" style:family="graphic"><style:graphic-properties fo:clip="rect(0in, 0in, 0.2602in, 0.5209in)"/></style:style></office:automatic-styles><office:body><office:text><text:p><draw:frame draw:style-name="fr1" text:anchor-type="as-char" svg:width="1.5in" svg:height="0.75in"><draw:image xlink:href="Pictures/q.png"/></draw:frame></text:p></office:text></office:body></office:document-content>"#,
        NS = crate::odt::tests::NS_T
    );
    let doc = crate::odt::import(&crate::odt::tests::zip_of(&[("content.xml", content.as_bytes()), ("Pictures/q.png", &png)])).unwrap();
    let crop = doc
        .body
        .iter()
        .find_map(|b| b.as_para())
        .and_then(|p| p.objects.iter().find_map(|o| if let wordcraft_doc::para::InlineObject::Image { crop, .. } = o { Some(*crop) } else { None }))
        .unwrap();
    assert_eq!(crop.map(|c| (c * 100.0).round() / 100.0), [0.25, 0.0, 0.0, 0.25]);
    let c = crate::odt::tests::content_of(&crate::odt::tests::export_model(&doc).unwrap());
    // 0.2602 in is 18.73 pt, 0.5209 in 37.5 pt.
    assert!(c.contains(r#"fo:clip="rect(0pt, 0pt, 18.73pt, 37.5pt)""#), "{c}");
    // An absurd crop (nothing left) is none.
    assert_eq!(crate::odt::body::drawing::crop_fractions([0.0, 100.0, 0.0, 100.0], &png), [0.0; 4]);
}

#[test]
fn contour_wrapping_and_its_outline_are_kept() {
    // `style:wrap-contour` with `outside` (Word's Tight) or `full` (Through), and the outline
    // (`draw:contour-polygon`) in the frame's view box, as WordCraft's wrap polygon.
    let png = {
        let img = image::RgbaImage::from_fn(10, 10, |_, _| image::Rgba([0, 0, 0, 255]));
        let mut b = Vec::new();
        img.write_to(&mut std::io::Cursor::new(&mut b), image::ImageFormat::Png).unwrap();
        b
    };
    for (mode, want) in [("outside", wordcraft_doc::para::Wrap::Tight), ("full", wordcraft_doc::para::Wrap::Through)] {
        let content = format!(
            r#"<office:document-content {NS} xmlns:draw="urn:oasis:names:tc:opendocument:xmlns:drawing:1.0" xmlns:svg="urn:oasis:names:tc:opendocument:xmlns:svg-compatible:1.0" xmlns:xlink="http://www.w3.org/1999/xlink"><office:automatic-styles><style:style style:name="fr1" style:family="graphic"><style:graphic-properties style:wrap="parallel" style:wrap-contour="true" style:wrap-contour-mode="{mode}" style:horizontal-pos="left" style:horizontal-rel="paragraph"/></style:style></office:automatic-styles><office:body><office:text><text:p><draw:frame draw:style-name="fr1" text:anchor-type="paragraph" svg:width="1in" svg:height="1in"><draw:image xlink:href="Pictures/p.png"/><draw:contour-polygon svg:width="1in" svg:height="1in" svg:viewBox="0 0 1000 1000" draw:points="500,0 1000,1000 0,1000" draw:recreate-on-edit="false"/></draw:frame>Text around it.</text:p></office:text></office:body></office:document-content>"#,
            NS = crate::odt::tests::NS_T
        );
        let doc = crate::odt::import(&crate::odt::tests::zip_of(&[("content.xml", content.as_bytes()), ("Pictures/p.png", &png)])).unwrap();
        let float = |d: &Document| {
            d.body.iter().find_map(|b| b.as_para()).and_then(|p| p.objects.iter().find_map(|o| o.frame().map(|(_, _, f)| f.clone()))).unwrap()
        };
        let f = float(&doc);
        assert_eq!(f.wrap, want, "{mode}");
        assert_eq!(f.wrap_polygon.as_deref().map(|p| p.points.clone()), Some(vec![[10800, 0], [21600, 21600], [0, 21600]]), "{mode}");
        let saved = crate::odt::tests::export_model(&doc).unwrap();
        let c = crate::odt::tests::content_of(&saved);
        assert!(c.contains(&format!(r#"style:wrap-contour="true" style:wrap-contour-mode="{mode}""#)), "{c}");
        assert!(c.contains(r#"draw:points="10800,0 21600,21600 0,21600""#), "{c}");
        let back = float(&crate::odt::import(&saved).unwrap());
        assert_eq!((back.wrap, back.wrap_polygon), (f.wrap, f.wrap_polygon));
    }
}

#[test]
fn a_contour_outside_its_view_box_is_worked_out_instead() {
    // Points far past the view box: not the frame's coordinates, so no polygon (the outline is
    // derived from the object), though the wrap stays a contour one.
    let a: crate::odt::xml::Attrs = vec![("svg:viewBox".into(), "0 0 100 100".into()), ("draw:points".into(), "0,0 100,150 0,100".into())];
    assert!(crate::odt::frames::contour_polygon(&a).is_none());
    let a: crate::odt::xml::Attrs = vec![("svg:viewBox".into(), "0 0 100 100".into()), ("draw:points".into(), "0,0 100,100 0,100".into())];
    assert!(crate::odt::frames::contour_polygon(&a).is_some());
}

#[test]
fn a_drawing_outside_any_paragraph_belongs_to_the_next_one() {
    // LibreOffice puts page-anchored frames before the first paragraph. They join it rather
    // than making a paragraph of their own, which would start the document on the default page
    // style instead of the one the first paragraph asks for.
    let ns = crate::odt::tests::NS_T;
    let styles = format!(
        r#"<office:document-styles {ns}><office:styles><style:style style:name="Opening" style:family="paragraph" style:master-page-name="First"/></office:styles><office:automatic-styles><style:page-layout style:name="pm1"><style:page-layout-properties fo:page-width="8.5in" fo:page-height="11in"/></style:page-layout></office:automatic-styles><office:master-styles><style:master-page style:name="Standard" style:page-layout-name="pm1"><style:header><text:p>Running head</text:p></style:header></style:master-page><style:master-page style:name="First" style:page-layout-name="pm1" style:next-style-name="Standard"/></office:master-styles></office:document-styles>"#
    );
    let content = format!(
        r#"<office:document-content {ns} xmlns:draw="urn:oasis:names:tc:opendocument:xmlns:drawing:1.0" xmlns:svg="urn:oasis:names:tc:opendocument:xmlns:svg-compatible:1.0"><office:body><office:text><draw:frame text:anchor-type="page" text:anchor-page-number="1" svg:x="1in" svg:y="2in" svg:width="2in" svg:height="1in"><draw:text-box><text:p>Address</text:p></draw:text-box></draw:frame><text:p text:style-name="Opening">Dear reader,</text:p><text:p>Body.</text:p></office:text></office:body></office:document-content>"#
    );
    let d = import(&crate::odt::tests::package(&content, Some(&styles))).unwrap();
    let paras: Vec<&wordcraft_doc::Paragraph> = d.body.iter().filter_map(|b| b.as_para()).collect();
    assert_eq!(paras.len(), 2, "no paragraph of the frame's own");
    assert_eq!(paras[0].text.replace(wordcraft_doc::para::OBJ, ""), "Dear reader,");
    assert_eq!(paras[0].objects.len(), 1);
    // One section, starting on the first page style: a different first page without the head.
    assert!(d.last_section.title_page && d.last_section.headers.default.is_some());
    // Drawings with no paragraph after them still get one.
    let tail = content.replace(r#"<text:p text:style-name="Opening">Dear reader,</text:p><text:p>Body.</text:p>"#, "");
    let d = import(&crate::odt::tests::package(&tail, Some(&styles))).unwrap();
    assert_eq!(d.body.iter().filter_map(|b| b.as_para()).map(|p| p.objects.len()).sum::<usize>(), 1);
}

#[test]
fn a_frame_keeps_its_place_in_the_stacking_order() {
    // §19.231 `draw:z-index`: 0 at the bottom; WordCraft's z counts from 1 (0 is unknown).
    let d = open(r#"style:wrap="run-through""#, r#"text:anchor-type="paragraph" draw:z-index="3""#);
    assert_eq!(float(&d).z, 4);
    let saved = crate::odt::tests::export_model(&d).unwrap();
    assert!(crate::odt::tests::entry(&saved, "content.xml").contains(r#"draw:z-index="3""#));
    assert_eq!(float(&import(&saved).unwrap()).z, 4);
    // Unknown (inserted in WordCraft): at the bottom, in document order.
    let d = open(r#"style:wrap="run-through""#, r#"text:anchor-type="paragraph""#);
    assert_eq!(float(&d).z, 0);
    assert!(crate::odt::tests::entry(&crate::odt::tests::export_model(&d).unwrap(), "content.xml").contains(r#"draw:z-index="0""#));
}

#[test]
fn a_frame_offering_several_formats_shows_the_first_it_can_read() {
    // LibreOffice's letter templates: its own metafile, then a PNG. Taking the first left nothing
    // to show, and the picture was gone once the paragraph was written from the model.
    let png = [b"\x89PNG\r\n\x1a\n".as_slice(), &[7u8; 64]].concat();
    let other = [b"\x89PNG\r\n\x1a\n".as_slice(), &[9u8; 64]].concat();
    let svm = b"VCLMTF a metafile WordCraft can't draw".to_vec();
    let frame = |images: &[&str]| {
        let images: String = images.iter().map(|h| format!(r#"<draw:image xlink:href="{h}"/>"#)).collect();
        format!(
            r#"<office:document-content {NS_T} {NS_D}><office:body><office:text><text:p><draw:frame svg:width="2in" svg:height="1in" text:anchor-type="as-char">{images}</draw:frame></text:p></office:text></office:body></office:document-content>"#
        )
    };
    let pictures = |d: &Document| d.body[0].as_para().unwrap().objects.iter().filter(|o| matches!(o, InlineObject::Image { .. })).count();
    let both = frame(&["Pictures/a.svm", "Pictures/a.png"]);
    let d = import(&zip_of(&[("content.xml", both.as_bytes()), ("Pictures/a.svm", &svm), ("Pictures/a.png", &png)])).unwrap();
    assert_eq!(pictures(&d), 1, "the PNG");
    let back = import(&crate::odt::tests::export_model(&d).unwrap()).unwrap();
    assert_eq!(pictures(&back), 1, "written from the model");
    // The first it can read, not the last.
    let order = frame(&["Pictures/a.png", "Pictures/b.png"]);
    let d = import(&zip_of(&[("content.xml", order.as_bytes()), ("Pictures/a.png", &png), ("Pictures/b.png", &other)])).unwrap();
    assert_eq!(pictures(&d), 1);
    assert_eq!(d.media.values().next().and_then(|m| m.get(8).copied()), Some(7), "the first");
    // Nothing it can read: no picture, and no failure.
    let only = frame(&["Pictures/a.svm"]);
    let d = import(&zip_of(&[("content.xml", only.as_bytes()), ("Pictures/a.svm", &svm)])).unwrap();
    assert_eq!(pictures(&d), 0);
}
