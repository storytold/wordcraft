//! Drawing shapes (ODF 1.3 §10.3: `draw:rect`, `draw:ellipse`, `draw:line`, and
//! `draw:custom-shape` with its `draw:enhanced-geometry` type): opened as WordCraft shapes with
//! their size, placement, fill, line and text, and written back as the same elements.

use crate::odt::tests::{NS_T, content_of, package};
use crate::odt::{export, import};
use wordcraft_doc::para::{InlineObject, OBJ, ShapeKind, Wrap};
use wordcraft_doc::props::Rgb;
use wordcraft_doc::{Document, Paragraph, para_block};

const NS_D: &str =
    r#"xmlns:draw="urn:oasis:names:tc:opendocument:xmlns:drawing:1.0" xmlns:svg="urn:oasis:names:tc:opendocument:xmlns:svg-compatible:1.0""#;

/// A shape as read: (kind, size, fill, line colour and width, floats, drawn bottom-left to top-right, its text).
type Seen = (ShapeKind, (f32, f32), Option<Rgb>, Option<(Rgb, f32)>, bool, bool, String);

fn shapes(d: &Document) -> Vec<Seen> {
    let mut out = Vec::new();
    for b in d.body.iter() {
        let Some(p) = b.as_para() else { continue };
        for o in &p.objects {
            if let InlineObject::Shape { kind, w, h, fill, stroke, stroke_width, float, story, .. } = o {
                let text = story
                    .and_then(|id| d.parts.get(&id))
                    .map(|part| part.blocks.iter().filter_map(|b| b.as_para()).map(|p| p.text.clone()).collect::<Vec<_>>().join("|"))
                    .unwrap_or_default();
                let size = ((*w * 10.0).round() / 10.0, (*h * 10.0).round() / 10.0);
                let line = stroke.map(|c| (c, (*stroke_width * 100.0).round() / 100.0));
                out.push((*kind, size, fill.as_ref().map(|f| f.main_color()), line, float.wrap != Wrap::Inline, float.flip_h != float.flip_v, text));
            }
        }
    }
    out
}

fn open() -> Document {
    let styles = format!(
        r##"<office:document-styles {NS_T} {NS_D}><office:styles><style:default-style style:family="graphic"><style:graphic-properties svg:stroke-color="#3465a4" draw:fill-color="#729fcf"/></style:default-style></office:styles></office:document-styles>"##
    );
    let content = format!(
        r##"<office:document-content {NS_T} {NS_D}><office:automatic-styles>
<style:style style:name="gr1" style:family="graphic"><style:graphic-properties svg:stroke-width="0.5pt" svg:stroke-color="#0000ff" draw:fill="solid" draw:fill-color="#ffcc00"/></style:style>
<style:style style:name="gr2" style:family="graphic"><style:graphic-properties draw:fill="solid" draw:fill-color="#00aa00"/></style:style>
<style:style style:name="gr3" style:family="graphic"><style:graphic-properties svg:stroke-color="#000080" draw:fill-color="#99ccff" style:wrap="parallel" style:horizontal-pos="from-left" style:horizontal-rel="paragraph" style:vertical-pos="from-top" style:vertical-rel="paragraph"/></style:style>
<style:style style:name="gr4" style:family="graphic"><style:graphic-properties svg:stroke-width="2pt" svg:stroke-color="#000000" style:wrap="run-through" style:run-through="background"/></style:style>
<style:style style:name="gr5" style:family="graphic"><style:graphic-properties draw:stroke="none" draw:fill="none"/></style:style>
</office:automatic-styles><office:body><office:text>
<text:p>A <draw:rect text:anchor-type="as-char" draw:style-name="gr1" svg:width="1in" svg:height="0.5in"><text:p/></draw:rect> b <draw:ellipse text:anchor-type="as-char" draw:style-name="gr2" svg:width="0.5in" svg:height="0.5in"/> c.</text:p>
<text:p><draw:custom-shape text:anchor-type="paragraph" draw:style-name="gr3" svg:width="2in" svg:height="1in" svg:x="1in" svg:y="0in"><text:p>Shape text</text:p><draw:enhanced-geometry svg:viewBox="0 0 21600 21600" draw:type="round-rectangle" draw:modifiers="3600"><draw:equation draw:name="f0" draw:formula="45"/></draw:enhanced-geometry></draw:custom-shape>Beside.</text:p>
<text:p><draw:line text:anchor-type="paragraph" draw:style-name="gr4" svg:x1="3in" svg:y1="0in" svg:x2="1in" svg:y2="1in"><text:p/></draw:line><draw:custom-shape text:anchor-type="paragraph" draw:style-name="gr5" svg:width="1in" svg:height="1in"><draw:enhanced-geometry draw:type="ooxml-heart"/></draw:custom-shape><draw:custom-shape text:anchor-type="paragraph" draw:style-name="gr5" svg:width="1in" svg:height="1in"><draw:enhanced-geometry draw:type="mso-spt100" draw:enhanced-path="M 0 0 L 21600 21600 N"/></draw:custom-shape>Under.</text:p>
</office:text></office:body></office:document-content>"##
    );
    import(&package(&content, Some(&styles))).unwrap()
}

#[test]
fn shapes_open_with_their_kind_size_look_and_text() {
    let d = open();
    // A line width no style states is the profile's (the standard gives none): WordCraft's 0.25 pt.
    let blue = Some((Rgb(0x34, 0x65, 0xa4), 0.25));
    assert_eq!(
        shapes(&d),
        [
            (ShapeKind::Rectangle, (72.0, 36.0), Some(Rgb(255, 204, 0)), Some((Rgb(0, 0, 255), 0.5)), false, false, String::new()),
            // No line stated: the default graphic style's, a hairline.
            (ShapeKind::Ellipse, (36.0, 36.0), Some(Rgb(0, 170, 0)), blue, false, false, String::new()),
            // No fill stated: a solid fill in the style's fill colour.
            (
                ShapeKind::RoundedRectangle,
                (144.0, 72.0),
                Some(Rgb(0x99, 0xcc, 0xff)),
                Some((Rgb(0, 0, 0x80), 0.25)),
                true,
                false,
                "Shape text".into()
            ),
            // From (3in, 0) to (1in, 1in): 2in × 1in, rising from its bottom left.
            (ShapeKind::Line, (144.0, 72.0), None, Some((Rgb(0, 0, 0), 2.0)), true, true, String::new()),
            (ShapeKind::Heart, (72.0, 72.0), None, None, true, false, String::new()),
            // A geometry WordCraft has no shape for: drawn from its own path.
            (ShapeKind::Freeform, (72.0, 72.0), None, None, true, false, String::new()),
        ]
    );
    // The shape's text isn't the paragraph's.
    assert_eq!(d.body[1].as_para().unwrap().text.replace(OBJ, "#"), "#Beside.");
    assert_eq!(d.body[0].as_para().unwrap().text.replace(OBJ, "#"), "A # b # c.");
    // The line starts at its leftmost point.
    let Some(InlineObject::Shape { float, .. }) = d.body[2].as_para().unwrap().objects.first() else { panic!() };
    assert_eq!((float.x, float.y), (72.0, 0.0));
    assert_eq!(float.wrap, Wrap::BehindText);
}

#[test]
fn shapes_round_trip() {
    let d = open();
    let bytes = crate::odt::tests::export_model(&d).unwrap();
    let c = content_of(&bytes);
    for el in ["<draw:rect ", "<draw:ellipse ", "<draw:line ", r#"draw:type="round-rectangle""#, r#"draw:type="heart""#] {
        assert!(c.contains(el), "{el} in {c}");
    }
    let back = import(&bytes).unwrap();
    assert_eq!(shapes(&back), shapes(&d));
    assert_eq!(back.body[1].as_para().unwrap().text, d.body[1].as_para().unwrap().text);
}

#[test]
fn every_wordcraft_shape_kind_is_written_and_read_back() {
    use ShapeKind::*;
    let kinds = [
        Rectangle,
        RoundedRectangle,
        Ellipse,
        FlowchartOr,
        Triangle,
        RightTriangle,
        Diamond,
        Line,
        Arrow,
        ArrowLeft,
        ArrowUp,
        ArrowDown,
        Star,
        Heart,
    ];
    let mut p = Paragraph::new();
    for k in kinds {
        p.text.push(OBJ);
        p.objects.push(InlineObject::Shape {
            kind: k,
            w: 50.0,
            h: 40.0,
            fill: Some(wordcraft_doc::fill::Fill::solid(Rgb(1, 2, 3))),
            stroke: Some(Rgb(4, 5, 6)),
            stroke_width: 1.5,
            float: Default::default(),
            story: None,
            freeform: None,
            effects: Default::default(),
            extra: Default::default(),
        });
    }
    p.runs = vec![wordcraft_doc::para::Run { len: p.text.len(), props: Default::default() }];
    let mut d = Document::new();
    d.body = vec![para_block(p)];
    let back = import(&export(&d).unwrap()).unwrap();
    let fill = |k| if k == Line { None } else { Some(Rgb(1, 2, 3)) };
    assert_eq!(shapes(&back), kinds.map(|k| (k, (50.0, 40.0), fill(k), Some((Rgb(4, 5, 6), 1.5)), false, false, String::new())).to_vec());
}

#[test]
fn hostile_shapes_do_not_panic() {
    let deep = r#"<draw:custom-shape svg:width="1in" svg:height="1in"><text:p>"#.repeat(100);
    for inner in [
        deep.as_str(),
        r#"<draw:rect svg:width="-5in" svg:height="NaNin"><text:p>unclosed"#,
        r#"<draw:line svg:x1="1e30in" svg:y1="0" svg:x2="-1e30in"/>"#,
        r#"<draw:rect svg:width="1in" svg:height="1in"><text:p>a<text:note><text:note-body><text:p>n</text:p></text:note-body></text:note></text:p></draw:rect>"#,
        r#"<text:note><text:note-body><text:p><draw:ellipse svg:width="1in" svg:height="1in"><text:p>in a note</text:p></draw:ellipse></text:p></text:note-body></text:note>"#,
    ] {
        let content = format!(
            r#"<office:document-content {NS_T} {NS_D}><office:body><office:text><text:p>x{inner}</text:p></office:text></office:body></office:document-content>"#
        );
        let d = import(&package(&content, None)).unwrap();
        let _ = export(&d).unwrap();
    }
}

#[test]
fn a_shape_keeps_its_size_unless_it_grows_to_its_text() {
    let content = format!(
        r#"<office:document-content {NS_T} {NS_D}><office:automatic-styles><style:style style:name="g" style:family="graphic"><style:graphic-properties draw:auto-grow-height="true"/></style:style></office:automatic-styles><office:body><office:text><text:p><draw:rect svg:width="1in" svg:height="1in"><text:p>fixed</text:p></draw:rect><draw:rect draw:style-name="g" svg:width="1in" svg:height="1in"><text:p>grows</text:p></draw:rect></text:p></office:text></office:body></office:document-content>"#
    );
    let d = import(&package(&content, None)).unwrap();
    let fits = |d: &Document| -> Vec<bool> {
        d.body[0].as_para().unwrap().objects.iter().filter_map(|o| o.text_box()).map(|id| d.parts.get(&id).unwrap().body.auto_fit).collect()
    };
    assert_eq!(fits(&d), [false, true]);
    let bytes = crate::odt::tests::export_model(&d).unwrap();
    // LibreOffice resizes a shape with text to it unless told not to.
    assert!(content_of(&bytes).contains(r#"draw:auto-grow-height="false" draw:auto-grow-width="false""#));
    assert_eq!(fits(&import(&bytes).unwrap()), [false, true]);
}

#[test]
fn a_shape_with_only_an_empty_paragraph_has_no_text() {
    // LibreOffice writes `<text:p/>` in every shape; only the round rectangle holds text.
    let d = open();
    let stories: Vec<bool> = d.body.iter().filter_map(|b| b.as_para()).flat_map(|p| p.objects.iter()).map(|o| o.text_box().is_some()).collect();
    assert_eq!(stories, [false, false, true, false, false, false]);
}
