//! Headers and footers written to the master pages (ODF 1.3 §16.10), read back by the reader
//! that already opens them.

use crate::odt::{export, import};
use wordcraft_doc::para::InlineObject;
use wordcraft_doc::props::CharProps;
use wordcraft_doc::{Document, Paragraph, Part, PartKind, para_block};

fn text_para(s: &str, props: CharProps) -> std::sync::Arc<wordcraft_doc::Block> {
    let mut p = Paragraph::new();
    p.text = s.into();
    p.runs = vec![wordcraft_doc::para::Run { len: s.len(), props }];
    para_block(p)
}

fn page_field_para(before: &str) -> std::sync::Arc<wordcraft_doc::Block> {
    let mut p = Paragraph::new();
    p.text = format!("{before}{}", wordcraft_doc::para::OBJ);
    p.objects.push(InlineObject::Field { instr: "PAGE".into(), result: "1".into(), locked: false, dirty: false, code: false, form: String::new() });
    p.runs = vec![wordcraft_doc::para::Run { len: p.text.len(), props: Default::default() }];
    para_block(p)
}

fn part(kind: PartKind, blocks: Vec<std::sync::Arc<wordcraft_doc::Block>>) -> Part {
    Part { kind, blocks, body: Default::default() }
}

fn part_text(d: &Document, id: Option<u32>) -> String {
    id.and_then(|i| d.parts.get(&i))
        .map(|p| p.blocks.iter().filter_map(|b| b.as_para()).map(|p| p.text.clone()).collect::<Vec<_>>().join("|"))
        .unwrap_or_default()
}

#[test]
fn headers_and_footers_are_written_and_read_back() {
    let mut d = Document::new();
    d.body = vec![text_para("Body", Default::default())];
    d.parts.insert(1, part(PartKind::Header, vec![text_para("Bold header", CharProps { bold: Some(true), ..Default::default() })]));
    d.parts.insert(2, part(PartKind::Footer, vec![page_field_para("Page ")]));
    d.parts.insert(3, part(PartKind::Header, vec![text_para("First page header", Default::default())]));
    d.last_section.headers.default = Some(1);
    d.last_section.footers.default = Some(2);
    d.last_section.headers.first = Some(3);
    d.last_section.title_page = true;
    let back = import(&export(&d).unwrap()).unwrap();
    let s = &back.last_section;
    assert_eq!(part_text(&back, s.headers.default), "Bold header");
    assert_eq!(part_text(&back, s.headers.first), "First page header");
    assert!(s.title_page);
    assert_eq!(part_text(&back, s.footers.default).replace(wordcraft_doc::para::OBJ, "#"), "Page #");
    // The header's text keeps its formatting; the footer its page field.
    let h = back.parts.get(&s.headers.default.unwrap()).unwrap().blocks[0].as_para().unwrap();
    assert_eq!(h.runs[0].props.bold, Some(true));
    let f = back.parts.get(&s.footers.default.unwrap()).unwrap().blocks[0].as_para().unwrap();
    assert!(matches!(&f.objects[..], [InlineObject::Field { instr, .. }] if instr == "PAGE"));
}

#[test]
fn even_page_headers_are_left_page_headers() {
    let mut d = Document::new();
    d.body = vec![text_para("Body", Default::default())];
    d.parts.insert(1, part(PartKind::Header, vec![text_para("Odd", Default::default())]));
    d.parts.insert(2, part(PartKind::Header, vec![text_para("Even", Default::default())]));
    d.last_section.headers.default = Some(1);
    d.last_section.headers.even = Some(2);
    d.settings.even_odd_headers = true;
    let back = import(&export(&d).unwrap()).unwrap();
    assert!(back.settings.even_odd_headers);
    assert_eq!(part_text(&back, back.last_section.headers.default), "Odd");
    assert_eq!(part_text(&back, back.last_section.headers.even), "Even");
}

#[test]
fn each_section_keeps_its_own_header() {
    let mut d = Document::new();
    let mut first = Paragraph::new();
    first.text = "One".into();
    first.runs = vec![wordcraft_doc::para::Run { len: 3, props: Default::default() }];
    let mut sec = d.last_section.clone();
    sec.headers.default = Some(1);
    first.section = Some(Box::new(sec));
    d.body = vec![para_block(first), text_para("Two", Default::default())];
    d.parts.insert(1, part(PartKind::Header, vec![text_para("H1", Default::default())]));
    d.parts.insert(2, part(PartKind::Header, vec![text_para("H2", Default::default())]));
    d.last_section.headers.default = Some(2);
    let back = import(&export(&d).unwrap()).unwrap();
    let firsts: Vec<_> = back.body.iter().filter_map(|b| b.as_para()).filter_map(|p| p.section.as_ref()).collect();
    assert_eq!(firsts.len(), 1);
    assert_eq!(part_text(&back, firsts[0].headers.default), "H1");
    assert_eq!(part_text(&back, back.last_section.headers.default), "H2");
}

#[test]
fn header_styles_do_not_clash_with_body_styles() {
    // The header's automatic styles live in styles.xml, the body's in content.xml: their names
    // must differ so neither reader takes one for the other.
    let mut d = Document::new();
    d.body = vec![text_para("Italic body", CharProps { italic: Some(true), ..Default::default() })];
    d.parts.insert(1, part(PartKind::Header, vec![text_para("Bold header", CharProps { bold: Some(true), ..Default::default() })]));
    d.last_section.headers.default = Some(1);
    let back = import(&export(&d).unwrap()).unwrap();
    let body = back.body[0].as_para().unwrap();
    assert_eq!((body.runs[0].props.italic, body.runs[0].props.bold), (Some(true), None));
    let h = back.parts.get(&back.last_section.headers.default.unwrap()).unwrap().blocks[0].as_para().unwrap();
    assert_eq!((h.runs[0].props.bold, h.runs[0].props.italic), (Some(true), None));
}

fn styles_with_header(header: &str) -> String {
    format!(
        r#"<office:document-styles {}><office:automatic-styles><style:page-layout style:name="pm1"><style:page-layout-properties fo:page-width="21cm" fo:page-height="29.7cm" fo:margin-top="2cm" fo:margin-bottom="1.5cm" fo:margin-left="2cm" fo:margin-right="2cm"/><style:header-style><style:header-footer-properties fo:min-height="0.5cm" fo:margin-bottom="0.25cm"/></style:header-style><style:footer-style><style:header-footer-properties fo:min-height="1cm" fo:margin-top="0.5cm"/></style:footer-style></style:page-layout></office:automatic-styles><office:master-styles><style:master-page style:name="Standard" style:page-layout-name="pm1">{header}</style:master-page></office:master-styles></office:document-styles>"#,
        crate::odt::tests::NS_T
    )
}

fn cm(v: f32) -> f32 {
    v * 72.0 / 2.54
}

#[test]
fn a_header_starts_at_the_page_margin_and_the_text_after_it() {
    // ODF puts the header at the page margin and the text below the header's band; Word measures
    // both from the page edge. Whether the band's height takes in its spacing the standard
    // doesn't say: WordCraft adds it (a 2 cm header with 1 cm spacing puts the text 3 cm below
    // the margin).
    let content = format!(
        r#"<office:document-content {}><office:body><office:text><text:p>x</text:p></office:text></office:body></office:document-content>"#,
        crate::odt::tests::NS_T
    );
    let with = crate::odt::tests::package(
        &content,
        Some(&styles_with_header("<style:header><text:p>H</text:p></style:header><style:footer><text:p>F</text:p></style:footer>")),
    );
    let near = |a: f32, b: f32| (a - b).abs() < 0.05;
    let s = import(&with).unwrap().last_section;
    assert!(near(s.header, cm(2.0)) && near(s.margin_top, cm(2.75)), "{} {}", s.header, s.margin_top);
    assert!(near(s.footer, cm(1.5)) && near(s.margin_bottom, cm(3.0)), "{} {}", s.footer, s.margin_bottom);
    // No header (or a hidden one): the page margins are the text's.
    let without =
        crate::odt::tests::package(&content, Some(&styles_with_header(r#"<style:header style:display="false"><text:p>H</text:p></style:header>"#)));
    let s = import(&without).unwrap().last_section;
    assert!(near(s.margin_top, cm(2.0)) && near(s.margin_bottom, cm(1.5)), "{} {}", s.margin_top, s.margin_bottom);
}

#[test]
fn a_headers_spacing_stays_below_content_taller_than_it() {
    // LibreOffice's report templates: a header taller than its band keeps its spacing below it,
    // unless the spacing is dynamic (taken by the content first, as in Word). A style that
    // doesn't say keeps it.
    let content = format!(
        r#"<office:document-content {}><office:body><office:text><text:p>x</text:p></office:text></office:body></office:document-content>"#,
        crate::odt::tests::NS_T
    );
    let hf = "<style:header><text:p>H</text:p></style:header><style:footer><text:p>F</text:p></style:footer>";
    let d = import(&crate::odt::tests::package(&content, Some(&styles_with_header(hf)))).unwrap();
    let near = |a: f32, b: f32| (a - b).abs() < 0.05;
    let s = &d.last_section;
    assert!(near(s.header_gap, cm(0.25)) && near(s.footer_gap, cm(0.5)), "{} {}", s.header_gap, s.footer_gap);
    // Written back as the band's spacing, saying it is kept: the same whatever reads it.
    let saved = export(&d).unwrap();
    assert!(crate::odt::tests::entry(&saved, "styles.xml").contains(r#"style:dynamic-spacing="false""#));
    let back = import(&saved).unwrap().last_section;
    assert!(near(back.header_gap, cm(0.25)) && near(back.footer_gap, cm(0.5)) && near(back.margin_top, s.margin_top), "{back:?}");
    let dynamic = styles_with_header(hf).replace(r#"fo:margin-bottom="0.25cm""#, r#"fo:margin-bottom="0.25cm" style:dynamic-spacing="true""#);
    let s = import(&crate::odt::tests::package(&content, Some(&dynamic))).unwrap().last_section;
    assert_eq!((s.header_gap, near(s.footer_gap, cm(0.5))), (0.0, true));
}

#[test]
fn a_header_without_a_stated_height_has_no_band() {
    // LibreOffice's letter templates: a header style stating no height, or none at all (the
    // standard gives none), has no band: the header is as tall as its text.
    let content = format!(
        r#"<office:document-content {}><office:body><office:text><text:p>x</text:p></office:text></office:body></office:document-content>"#,
        crate::odt::tests::NS_T
    );
    let near = |a: f32, b: f32| (a - b).abs() < 0.05;
    for band in ["<style:header-style/>", "", r#"<style:header-style><style:header-footer-properties fo:margin-bottom="0cm"/></style:header-style>"#]
    {
        let styles = format!(
            r#"<office:document-styles {}><office:automatic-styles><style:page-layout style:name="pm1"><style:page-layout-properties fo:page-width="21cm" fo:page-height="29.7cm" fo:margin-top="2cm" fo:margin-bottom="2cm" fo:margin-left="2cm" fo:margin-right="2cm"/>{band}</style:page-layout></office:automatic-styles><office:master-styles><style:master-page style:name="Standard" style:page-layout-name="pm1"><style:header><text:p>H</text:p></style:header></style:master-page></office:master-styles></office:document-styles>"#,
            crate::odt::tests::NS_T
        );
        let pkg = crate::odt::tests::package(&content, Some(&styles));
        let s = import(&pkg).unwrap().last_section;
        assert!(near(s.header, cm(2.0)) && near(s.margin_top, cm(2.0)), "{band}: {} {}", s.header, s.margin_top);
    }
}

#[test]
fn header_and_footer_distances_round_trip() {
    let mut d = Document::new();
    d.body = vec![text_para("Body", Default::default())];
    d.parts.insert(1, part(PartKind::Header, vec![text_para("H", Default::default())]));
    d.parts.insert(2, part(PartKind::Footer, vec![text_para("F", Default::default())]));
    d.last_section.headers.default = Some(1);
    d.last_section.footers.default = Some(2);
    (d.last_section.header, d.last_section.margin_top, d.last_section.footer, d.last_section.margin_bottom) = (30.0, 80.0, 20.0, 70.0);
    let s = import(&export(&d).unwrap()).unwrap().last_section;
    let near = |a: f32, b: f32| (a - b).abs() < 0.05;
    assert!(near(s.header, 30.0) && near(s.margin_top, 80.0) && near(s.footer, 20.0) && near(s.margin_bottom, 70.0), "{s:?}");
}

#[test]
fn a_first_page_master_followed_by_another_is_a_different_first_page() {
    // LibreOffice's "First Page" style (no header) passes to "Standard" (a header) after the
    // first page: one section, the first page's header empty, the others' Standard's.
    let ns = crate::odt::tests::NS_T;
    let styles = format!(
        r#"<office:document-styles {ns}><office:styles><style:style style:name="Opening" style:family="paragraph" style:master-page-name="First_20_Page"/></office:styles><office:automatic-styles><style:page-layout style:name="pm1"><style:page-layout-properties fo:page-width="8.5in" fo:page-height="11in" fo:margin-top="1in"/></style:page-layout><style:page-layout style:name="pm2"><style:page-layout-properties fo:page-width="8.5in" fo:page-height="11in" fo:margin-top="2in"/></style:page-layout></office:automatic-styles><office:master-styles><style:master-page style:name="Standard" style:page-layout-name="pm1"><style:header><text:p>Running head</text:p></style:header></style:master-page><style:master-page style:name="First_20_Page" style:page-layout-name="pm2" style:next-style-name="Standard"/></office:master-styles></office:document-styles>"#
    );
    let content = format!(
        r#"<office:document-content {ns}><office:body><office:text><text:p text:style-name="Opening">Letter.</text:p><text:p>More.</text:p></office:text></office:body></office:document-content>"#
    );
    let d = import(&crate::odt::tests::package(&content, Some(&styles))).unwrap();
    let sec = &d.last_section;
    assert!(sec.title_page, "a different first page");
    let first_text: String = sec
        .headers
        .first
        .and_then(|id| d.parts.get(&id))
        .map(|p| p.blocks.iter().filter_map(|b| b.as_para()).map(|p| p.text.clone()).collect())
        .unwrap_or_default();
    assert_eq!(first_text, "", "{:?}", sec.headers);
    let head = sec.headers.default.and_then(|id| d.parts.get(&id)).expect("the running head");
    assert_eq!(head.blocks[0].as_para().map(|p| p.text.as_str()), Some("Running head"));
    // The first page's own size and margins.
    assert_eq!(sec.margin_top, 144.0);
    // Written back: an empty first-page header.
    let saved = crate::odt::tests::export_model(&d).unwrap();
    let s = crate::odt::tests::entry(&saved, "styles.xml");
    let first = s.split("<style:header-first>").nth(1).and_then(|r| r.split("</style:header-first>").next()).unwrap_or("missing");
    assert!(first.trim().starts_with("<text:p") && !first.contains("Running head") && s.contains("Running head"), "{s}");
}

#[test]
fn master_pages_naming_each_other_take_the_next_ones_own_headers() {
    // LibreOffice's Left Page and Right Page name each other as next: the pages after the first
    // take the other master's own header, whatever order the masters are gone through in (each
    // import hashes them afresh, so a few imports cover both orders).
    let ns = crate::odt::tests::NS_T;
    let styles = format!(
        r#"<office:document-styles {ns}><office:styles><style:style style:name="Opening" style:family="paragraph" style:master-page-name="Right_20_Page"/></office:styles><office:automatic-styles><style:page-layout style:name="pm1"><style:page-layout-properties fo:page-width="8.5in" fo:page-height="11in"/></style:page-layout></office:automatic-styles><office:master-styles><style:master-page style:name="Left_20_Page" style:page-layout-name="pm1" style:next-style-name="Right_20_Page"><style:header><text:p>Left head</text:p></style:header></style:master-page><style:master-page style:name="Right_20_Page" style:page-layout-name="pm1" style:next-style-name="Left_20_Page"><style:header><text:p>Right head</text:p></style:header></style:master-page></office:master-styles></office:document-styles>"#
    );
    let content = format!(
        r#"<office:document-content {ns}><office:body><office:text><text:p text:style-name="Opening">Recto.</text:p><text:p>More.</text:p></office:text></office:body></office:document-content>"#
    );
    let file = crate::odt::tests::package(&content, Some(&styles));
    for _ in 0..24 {
        let d = import(&file).unwrap();
        let text = |id: Option<u32>| -> String {
            id.and_then(|id| d.parts.get(&id))
                .map(|p| p.blocks.iter().filter_map(|b| b.as_para()).map(|p| p.text.clone()).collect())
                .unwrap_or_default()
        };
        let sec = &d.last_section;
        assert_eq!(text(sec.headers.first), "Right head", "the first page is the Right Page's own");
        assert_eq!(text(sec.headers.default), "Left head", "the pages after it the Left Page's");
    }
}

#[test]
fn a_watermark_is_a_shape_in_the_header_and_reads_back_as_one() {
    // Written as LibreOffice and Word write theirs (a fontwork shape behind the text in every
    // header, here one made for it), and read back as the watermark, not a shape.
    let mut d = Document::new();
    d.body = vec![para_block(Paragraph::with_text("Body", CharProps::default()))];
    d.settings.watermark = Some(wordcraft_doc::Watermark { text: "CONFIDENTIAL".into(), ..Default::default() });
    let saved = export(&d).unwrap();
    let styles = crate::odt::tests::entry(&saved, "styles.xml");
    assert!(styles.contains("draw:name=\"PowerPlusWaterMarkObject\"") && styles.contains("draw:text-path=\"true\""), "{styles}");
    assert!(styles.contains("<style:header>"), "a header holds it");
    let back = import(&saved).unwrap();
    let wm = back.settings.watermark.clone().expect("the watermark");
    assert_eq!((wm.text.as_str(), wm.diagonal, wm.semitransparent), ("CONFIDENTIAL", true, true));
    // Not a shape in the header too.
    let header = back.last_section.headers.default.and_then(|id| back.parts.get(&id)).unwrap();
    assert!(header.blocks.iter().filter_map(|b| b.as_para()).all(|p| p.objects.is_empty()));
    // The text doesn't move for it: the header fits in the top margin.
    let mut without = d.clone();
    without.settings.watermark = None;
    let plain = import(&export(&without).unwrap()).unwrap();
    assert!(
        (back.last_section.margin_top - plain.last_section.margin_top).abs() < 0.5,
        "{} {}",
        back.last_section.margin_top,
        plain.last_section.margin_top
    );
}

/// A watermark added, changed or removed in WordCraft reaches a file's page definitions.
#[test]
fn a_watermark_change_to_a_file_is_saved() {
    // A file's own page definitions were written back whatever happened to its watermark, so one
    // added, changed or removed in WordCraft was gone from the saved file.
    let ns = crate::odt::tests::NS_T;
    let styles = format!(
        r#"<office:document-styles {ns}><office:automatic-styles><style:page-layout style:name="pm1" style:page-usage="mirrored"><style:page-layout-properties fo:page-width="21cm" fo:page-height="29.7cm" fo:margin-top="2cm" fo:margin-bottom="2cm" fo:margin-left="2cm" fo:margin-right="2cm"/></style:page-layout></office:automatic-styles><office:master-styles><style:master-page style:name="Standard" style:page-layout-name="pm1"/></office:master-styles></office:document-styles>"#
    );
    let content =
        format!("<office:document-content {ns}><office:body><office:text><text:p>x</text:p></office:text></office:body></office:document-content>");
    let plain = crate::odt::tests::package(&content, Some(&styles));
    let styles_of = |d: &Document| crate::odt::tests::entry(&export(d).unwrap(), "styles.xml");
    let shape = "PowerPlusWaterMarkObject";

    // Added.
    let mut d = import(&plain).unwrap();
    d.settings.watermark = Some(wordcraft_doc::Watermark { text: "DRAFT".into(), ..Default::default() });
    let saved = export(&d).unwrap();
    let s = crate::odt::tests::entry(&saved, "styles.xml");
    assert!(s.contains(shape) && s.contains("DRAFT"), "added: {s}");
    assert_eq!(import(&saved).unwrap().settings.watermark.map(|w| w.text).as_deref(), Some("DRAFT"));

    // Saved again with nothing changed, it stays as the file has it.
    let read = import(&saved).unwrap();
    let again = styles_of(&read);
    assert_eq!(again.matches(shape).count(), s.matches(shape).count(), "{again}");

    // Changed.
    let mut d = import(&saved).unwrap();
    if let Some(w) = d.settings.watermark.as_mut() {
        w.text = "COPY".into();
    }
    let s = styles_of(&d);
    assert!(s.contains("COPY") && !s.contains("DRAFT"), "changed: {s}");

    // Removed.
    let mut d = import(&saved).unwrap();
    d.settings.watermark = None;
    let s = styles_of(&d);
    assert!(!s.contains(shape) && !s.contains("DRAFT"), "removed: {s}");

    // One with no text isn't written, so it's no change to the file's page definitions.
    let mut d = import(&plain).unwrap();
    d.settings.watermark = Some(wordcraft_doc::Watermark { text: "  ".into(), ..Default::default() });
    let s = styles_of(&d);
    assert!(s.contains(r#"style:page-usage="mirrored""#) && !s.contains(shape), "no text: {s}");
}

/// Different Odd & Even Pages turned on or off in WordCraft reaches a file's page definitions.
#[test]
fn different_odd_and_even_pages_changed_in_a_file_is_saved() {
    // A file's own page definitions were written back whatever happened to this setting, so the
    // left pages' header stayed after it was turned off.
    let ns = crate::odt::tests::NS_T;
    let file = |left: &str| {
        let styles = format!(
            r#"<office:document-styles {ns}><office:automatic-styles><style:page-layout style:name="pm1" style:page-usage="mirrored"><style:page-layout-properties fo:page-width="21cm" fo:page-height="29.7cm" fo:margin-top="2cm" fo:margin-bottom="2cm" fo:margin-left="2cm" fo:margin-right="2cm"/><style:header-style><style:header-footer-properties fo:min-height="1cm"/></style:header-style></style:page-layout></office:automatic-styles><office:master-styles><style:master-page style:name="Standard" style:page-layout-name="pm1"><style:header><text:p>Right head</text:p></style:header>{left}</style:master-page></office:master-styles></office:document-styles>"#
        );
        let content = format!(
            "<office:document-content {ns}><office:body><office:text><text:p>x</text:p></office:text></office:body></office:document-content>"
        );
        crate::odt::tests::package(&content, Some(&styles))
    };
    let styles_of = |d: &Document| crate::odt::tests::entry(&export(d).unwrap(), "styles.xml");

    // On in the file, and nothing changed: the file's own.
    let both = file("<style:header-left><text:p>Left head</text:p></style:header-left>");
    let d = import(&both).unwrap();
    assert!(d.settings.even_odd_headers);
    let s = styles_of(&d);
    assert!(s.contains(r#"style:page-usage="mirrored""#) && s.contains("Left head"), "as read: {s}");

    // Turned off.
    let mut d = import(&both).unwrap();
    d.settings.even_odd_headers = false;
    let saved = export(&d).unwrap();
    let s = crate::odt::tests::entry(&saved, "styles.xml");
    assert!(!s.contains("<style:header-left") && s.contains("Right head"), "off: {s}");
    assert!(!import(&saved).unwrap().settings.even_odd_headers);

    // Off in the file, and turned on: left pages have their own header, an empty one so far.
    let mut d = import(&file("")).unwrap();
    assert!(!d.settings.even_odd_headers);
    d.settings.even_odd_headers = true;
    let saved = export(&d).unwrap();
    let s = crate::odt::tests::entry(&saved, "styles.xml");
    assert!(s.contains("<style:header-left") && s.contains("Right head"), "on: {s}");
    assert!(import(&saved).unwrap().settings.even_odd_headers);
}

#[test]
fn libreoffices_watermark_reads_as_the_watermark() {
    // As LibreOffice's Format › Watermark writes it (observed): turned, half see-through.
    let ns = crate::odt::tests::NS_T;
    let nsd =
        r#"xmlns:draw="urn:oasis:names:tc:opendocument:xmlns:drawing:1.0" xmlns:svg="urn:oasis:names:tc:opendocument:xmlns:svg-compatible:1.0""#;
    let styles = format!(
        r##"<office:document-styles {ns} {nsd}><office:automatic-styles><style:style style:name="Mgr1" style:family="graphic"><style:graphic-properties draw:stroke="none" draw:fill="solid" draw:fill-color="#c0c0c0" draw:opacity="50%" style:run-through="background" style:wrap="run-through"/></style:style><style:page-layout style:name="pm1"><style:page-layout-properties fo:page-width="21cm" fo:page-height="29.7cm" fo:margin-top="2cm" fo:margin-bottom="2cm" fo:margin-left="2cm" fo:margin-right="2cm"/></style:page-layout></office:automatic-styles><office:master-styles><style:master-page style:name="Standard" style:page-layout-name="pm1"><style:header><text:p><draw:custom-shape text:anchor-type="char" draw:name="PowerPlusWaterMarkObject" draw:style-name="Mgr1" svg:width="6.69in" svg:height="2.24in" draw:transform="rotate (0.785398163397448) translate (0.19in 6.54in)"><text:p>DRAFT</text:p><draw:enhanced-geometry svg:viewBox="0 0 21600 21600" draw:text-path="true" draw:type="fontwork-plain-text"/></draw:custom-shape></text:p></style:header></style:master-page></office:master-styles></office:document-styles>"##
    );
    let content =
        format!("<office:document-content {ns}><office:body><office:text><text:p>x</text:p></office:text></office:body></office:document-content>");
    let d = import(&crate::odt::tests::package(&content, Some(&styles))).unwrap();
    let wm = d.settings.watermark.expect("the watermark");
    assert_eq!((wm.text.as_str(), wm.color, wm.diagonal, wm.semitransparent), ("DRAFT", wordcraft_doc::Rgb(0xc0, 0xc0, 0xc0), true, true));
    let [w, h] = wm.size.unwrap();
    assert!((w - 6.69 * 72.0).abs() < 0.5 && (h - 2.24 * 72.0).abs() < 0.5);
}
