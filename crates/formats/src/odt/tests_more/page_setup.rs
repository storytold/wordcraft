//! Page-level settings (ODF 1.3 §17.2 `style:page-layout-properties`, §16.29
//! `text:notes-configuration`, §16.30 `text:linenumbering-configuration`): page number format,
//! page colour and borders, footnote and endnote numbering, and line numbering, read into the
//! document's and its sections' settings and written back.

use crate::odt::tests::{NS_T, package};
use crate::odt::{export, import};
use wordcraft_doc::Document;
use wordcraft_doc::props::{BorderStyle, Rgb};
use wordcraft_doc::section::{LineNumberRestart, NoteRestart, NumFormat};

fn open() -> Document {
    let styles = format!(
        r##"<office:document-styles {NS_T}><office:styles>
<text:notes-configuration text:note-class="footnote" style:num-format="a" text:start-value="4" text:footnotes-position="page" text:start-numbering-at="page"/>
<text:notes-configuration text:note-class="endnote" style:num-format="I" text:start-value="0"/>
<text:linenumbering-configuration text:number-lines="true" text:offset="0.25in" style:num-format="1" text:number-position="left" text:increment="5" text:restart-on-page="true"/>
</office:styles><office:automatic-styles><style:page-layout style:name="pm1"><style:page-layout-properties fo:page-width="8.5in" fo:page-height="11in" fo:margin-top="1in" fo:margin-bottom="1in" fo:margin-left="1in" fo:margin-right="1in" style:num-format="i" fo:background-color="#ffeecc" fo:border="0.5pt solid #ff0000" fo:padding="10pt"/></style:page-layout></office:automatic-styles><office:master-styles><style:master-page style:name="Standard" style:page-layout-name="pm1"/></office:master-styles></office:document-styles>"##
    );
    let content = format!(
        r#"<office:document-content {NS_T}><office:body><office:text><text:p>Body.</text:p></office:text></office:body></office:document-content>"#
    );
    import(&package(&content, Some(&styles))).unwrap()
}

/// What these settings come to: (page number format, page colour, a page border's style, width,
/// colour and space, footnote format, start and restart, endnote format, line numbering).
#[allow(clippy::type_complexity)]
fn settings(
    d: &Document,
) -> (
    NumFormat,
    Option<Rgb>,
    Option<(BorderStyle, f32, Option<Rgb>, f32)>,
    NumFormat,
    Option<u32>,
    Option<NoteRestart>,
    NumFormat,
    Option<(u32, f32, LineNumberRestart)>,
) {
    let sec = &d.last_section;
    let top = sec.page_borders.and_then(|b| b.top).map(|b| (b.style, b.width, b.color, b.space));
    let lines = sec.line_numbers.as_ref().map(|l| (l.count_by, l.distance, l.restart));
    (
        sec.page_num_format,
        d.settings.page_color,
        top,
        d.settings.footnote_format,
        d.settings.footnote_pr.num_start,
        d.settings.footnote_pr.num_restart,
        d.settings.endnote_format,
        lines,
    )
}

#[test]
fn page_settings_open() {
    assert_eq!(
        settings(&open()),
        (
            NumFormat::LowerRoman,
            Some(Rgb(0xff, 0xee, 0xcc)),
            // At the margin, 1in from the edge (WordCraft measures from the edge).
            Some((BorderStyle::Single, 0.5, Some(Rgb(255, 0, 0)), 72.0)),
            NumFormat::LowerLetter,
            // ODF counts from 0: start value 4 is the fifth.
            Some(5),
            Some(NoteRestart::EachPage),
            NumFormat::UpperRoman,
            Some((5, 18.0, LineNumberRestart::Page)),
        )
    );
}

#[test]
fn the_text_starts_inside_a_page_border() {
    // ODF's margin is the border's distance from the edge; the text is inside its padding.
    let d = open();
    assert_eq!(d.last_section.margin_left, 72.0 + 0.5 + 10.0);
    assert_eq!(d.last_section.margin_top, 72.0 + 0.5 + 10.0);
}

#[test]
fn page_settings_round_trip() {
    let d = open();
    assert_eq!(settings(&import(&export(&d).unwrap()).unwrap()), settings(&d));
    // Defaults stay defaults.
    let plain = Document::new();
    assert_eq!(settings(&import(&export(&plain).unwrap()).unwrap()), settings(&plain));
}

#[test]
fn hostile_page_settings_do_not_panic() {
    let styles = format!(
        r#"<office:document-styles {NS_T}><office:styles><text:notes-configuration text:note-class="footnote" text:start-value="99999999999" style:num-format="zz"/><text:linenumbering-configuration text:number-lines="true" text:increment="-4" text:offset="1e30in"/></office:styles><office:automatic-styles><style:page-layout style:name="pm1"><style:page-layout-properties fo:border="1e30pt solid" fo:padding="-5in" fo:background-color="transparent"/></style:page-layout></office:automatic-styles><office:master-styles><style:master-page style:name="Standard" style:page-layout-name="pm1"/></office:master-styles></office:document-styles>"#
    );
    let content = format!(
        r#"<office:document-content {NS_T}><office:body><office:text><text:p>x</text:p></office:text></office:body></office:document-content>"#
    );
    let d = import(&package(&content, Some(&styles))).unwrap();
    let s = settings(&d);
    assert!(s.4.is_none_or(|n| n <= wordcraft_doc::section::MAX_NOTE_START), "{s:?}");
    assert!(s.7.as_ref().is_none_or(|l| l.0 >= 1 && l.1.is_finite() && l.1 <= 1584.0), "{s:?}");
    let _ = export(&d).unwrap();
}

#[test]
fn line_numbering_is_on_unless_it_says_otherwise() {
    // ODF 1.3 §19.845: `text:number-lines` defaults to true; LibreOffice leaves it out.
    for (attr, on) in [("", true), (r#"text:number-lines="false""#, false)] {
        let styles = format!(
            r#"<office:document-styles {NS_T}><office:styles><text:linenumbering-configuration {attr} text:increment="5"/></office:styles><office:automatic-styles><style:page-layout style:name="pm1"><style:page-layout-properties fo:page-width="8.5in" fo:page-height="11in"/></style:page-layout></office:automatic-styles><office:master-styles><style:master-page style:name="Standard" style:page-layout-name="pm1"/></office:master-styles></office:document-styles>"#
        );
        let content = format!(
            r#"<office:document-content {NS_T}><office:body><office:text><text:p>x</text:p></office:text></office:body></office:document-content>"#
        );
        let d = import(&package(&content, Some(&styles))).unwrap();
        assert_eq!(d.last_section.line_numbers.is_some(), on, "{attr}");
    }
}

#[test]
fn a_page_layout_without_a_size_is_the_default_pages_size() {
    // LibreOffice's templates often state margins but no page size: it is the default paper's
    // (the user's region's in WordCraft, as LibreOffice uses its own default).
    let styles = format!(
        r#"<office:document-styles {}><office:automatic-styles><style:page-layout style:name="pm1"><style:page-layout-properties fo:margin-top="1in" fo:margin-bottom="1in" fo:margin-left="1.25in" fo:margin-right="1.25in"/></style:page-layout></office:automatic-styles><office:master-styles><style:master-page style:name="Standard" style:page-layout-name="pm1"/></office:master-styles></office:document-styles>"#,
        crate::odt::tests::NS_T
    );
    let content = format!(
        "<office:document-content {}><office:body><office:text><text:p>x</text:p></office:text></office:body></office:document-content>",
        crate::odt::tests::NS_T
    );
    let bytes = crate::odt::tests::package(&content, Some(&styles));
    let a4 = wordcraft_doc::SectionProps { page_w: 595.3, page_h: 841.9, ..Default::default() };
    let d = crate::import_on("odt", &bytes, &a4).unwrap().unwrap();
    assert_eq!((d.last_section.page_w, d.last_section.page_h), (595.3, 841.9));
    assert_eq!((d.last_section.margin_top, d.last_section.margin_left), (72.0, 90.0));
    // A stated size is the file's.
    let sized = styles.replace(r#"<style:page-layout-properties "#, r#"<style:page-layout-properties fo:page-width="5in" fo:page-height="7in" "#);
    let d = crate::import_on("odt", &crate::odt::tests::package(&content, Some(&sized)), &a4).unwrap().unwrap();
    assert_eq!((d.last_section.page_w, d.last_section.page_h), (360.0, 504.0));
}

#[test]
fn a_document_without_a_page_layout_has_the_default_paper_and_no_margins() {
    // No styles.xml at all: the region's paper (the standard leaves the size to the
    // application), and no margins (the standard leaves them open too; 0 is XSL's initial value).
    // It is laid out as an ODF document.
    let content = format!(
        "<office:document-content {}><office:body><office:text><text:p>x</text:p></office:text></office:body></office:document-content>",
        crate::odt::tests::NS_T
    );
    let a4 = wordcraft_doc::SectionProps { page_w: 595.3, page_h: 841.9, ..Default::default() };
    let d = crate::import_on("odt", &crate::odt::tests::package(&content, None), &a4).unwrap().unwrap();
    let s = &d.last_section;
    assert_eq!((s.page_w, s.page_h), (595.3, 841.9));
    assert_eq!([s.margin_top, s.margin_bottom, s.margin_left, s.margin_right], [0.0; 4], "{s:?}");
    assert!(d.settings.odf.is_some());
}

#[test]
fn a_page_layout_stating_nothing_has_no_margins() {
    // LibreOffice's CV template: a page layout with no properties at all. Its margins are XSL's
    // initial value, 0.
    let ns = crate::odt::tests::NS_T;
    let styles = format!(
        r#"<office:document-styles {ns}><office:automatic-styles><style:page-layout style:name="Mpm1"><style:header-style/><style:footer-style/></style:page-layout></office:automatic-styles><office:master-styles><style:master-page style:name="Standard" style:page-layout-name="Mpm1"/></office:master-styles></office:document-styles>"#
    );
    let content =
        format!("<office:document-content {ns}><office:body><office:text><text:p>x</text:p></office:text></office:body></office:document-content>");
    let d = crate::import("odt", &crate::odt::tests::package(&content, Some(&styles))).unwrap().unwrap();
    let s = &d.last_section;
    assert_eq!([s.margin_top, s.margin_bottom, s.margin_left, s.margin_right], [0.0; 4]);
    // One margin stated: that one is the file's.
    let one = styles.replace("<style:header-style/>", r#"<style:page-layout-properties fo:margin-left="1in"/><style:header-style/>"#);
    let d = crate::import("odt", &crate::odt::tests::package(&content, Some(&one))).unwrap().unwrap();
    assert_eq!((d.last_section.margin_left, d.last_section.margin_top), (72.0, 0.0));
}

#[test]
fn a_page_background_picture_is_read_and_written() {
    // LibreOffice's report templates: a picture centred on the page, not repeated.
    let png = {
        let img = image::RgbImage::from_pixel(8, 4, image::Rgb([200, 30, 30]));
        let mut out = std::io::Cursor::new(Vec::new());
        img.write_to(&mut out, image::ImageFormat::Png).unwrap();
        out.into_inner()
    };
    let ns = crate::odt::tests::NS_T;
    let layout = |repeat: &str| {
        format!(
            r#"<office:document-styles {ns} xmlns:xlink="http://www.w3.org/1999/xlink"><office:automatic-styles><style:page-layout style:name="pm1"><style:page-layout-properties fo:page-width="8.5in" fo:page-height="11in"><style:background-image xlink:href="Pictures/map.png" style:position="center center" {repeat}/></style:page-layout-properties></style:page-layout></office:automatic-styles><office:master-styles><style:master-page style:name="Standard" style:page-layout-name="pm1"/></office:master-styles></office:document-styles>"#
        )
    };
    let content =
        format!("<office:document-content {ns}><office:body><office:text><text:p>x</text:p></office:text></office:body></office:document-content>");
    for (repeat, fill) in [(r#"style:repeat="no-repeat""#, "center"), (r#"style:repeat="stretch""#, "frame"), ("", "tile")] {
        let styles = layout(repeat);
        let pkg = crate::odt::tests::zip_of(&[("content.xml", content.as_bytes()), ("styles.xml", styles.as_bytes()), ("Pictures/map.png", &png)]);
        let d = crate::odt::import(&pkg).unwrap();
        let pic = d.settings.page_picture.as_ref().expect("a page picture");
        assert_eq!(pic.fill_type, fill);
        assert_eq!(d.media.get(&pic.media).map(|b| b.as_slice()), Some(png.as_slice()));
        // Written back in the page layout, the picture in the package.
        let saved = crate::odt::export(&d).unwrap();
        let s = crate::odt::tests::entry(&saved, "styles.xml");
        let want_repeat = match fill {
            "center" => "no-repeat",
            "frame" => "stretch",
            _ => "repeat",
        };
        assert!(s.contains(&format!(r#"style:repeat="{want_repeat}""#)) && s.contains("Pictures/page-background.png"), "{s}");
        let again = crate::odt::import(&saved).unwrap();
        assert_eq!(again.settings.page_picture.as_ref().map(|p| p.fill_type.as_str()), Some(fill));
    }
}

#[test]
fn a_page_borders_padding_keeps_the_text_where_it_was() {
    // Design › Page Borders around a page with 1 inch margins: the border 24 pt from the edge,
    // 45 pt of padding inside it (more than Word's 31 pt border spacing, which is measured
    // differently): the text stays 72 pt from the edge, written and read back.
    let mut d = Document::new();
    d.body = vec![wordcraft_doc::para_block(wordcraft_doc::Paragraph::with_text("x", Default::default()))];
    let edge = wordcraft_doc::props::Border { style: BorderStyle::Single, width: 3.0, color: Some(Rgb(0x2f, 0x55, 0x97)), space: 24.0 };
    d.last_section.page_borders =
        Some(wordcraft_doc::props::Borders { top: Some(edge), bottom: Some(edge), left: Some(edge), right: Some(edge), ..Default::default() });
    let back = crate::odt::import(&crate::odt::export(&d).unwrap()).unwrap();
    let s = &back.last_section;
    for m in [s.margin_top, s.margin_bottom, s.margin_left, s.margin_right] {
        assert!((m - 72.0).abs() < 0.05, "{s:?}");
    }
    assert_eq!(s.page_borders.and_then(|b| b.left).map(|b| (b.width, b.space)), Some((3.0, 24.0)));
}

/// A file with what WordCraft doesn't model of a page: how its pages are used, the footnote line,
/// a header's background.
fn page_file(extra_layouts: &str, masters: &str, body: &str) -> Vec<u8> {
    let ns = crate::odt::tests::NS_T;
    let styles = format!(
        r##"<office:document-styles {ns}><office:automatic-styles><style:page-layout style:name="pm1" style:page-usage="mirrored"><style:page-layout-properties fo:page-width="21cm" fo:page-height="29.7cm" fo:margin-top="2cm" fo:margin-bottom="2cm" fo:margin-left="3cm" fo:margin-right="2cm" style:print-orientation="portrait"><style:footnote-sep style:width="0.05cm" style:color="#ff0000"/></style:page-layout-properties><style:header-style><style:header-footer-properties fo:min-height="1cm" fo:margin-bottom="0.5cm" fo:background-color="#eeffee" style:dynamic-spacing="false"/></style:header-style></style:page-layout>{extra_layouts}</office:automatic-styles><office:master-styles><style:master-page style:name="Standard" style:page-layout-name="pm1"><style:header><text:p>Kopf</text:p></style:header></style:master-page>{masters}</office:master-styles></office:document-styles>"##
    );
    let content = format!("<office:document-content {ns}><office:body><office:text>{body}</office:text></office:body></office:document-content>");
    crate::odt::tests::package(&content, Some(&styles))
}

#[test]
fn a_page_setup_change_leaves_the_rest_of_the_files_page_definitions() {
    // Changing one margin wrote every page layout and master page from the model, so a header's
    // background and the footnote line were gone in LibreOffice.
    let bytes = page_file("", "", "<text:p>x</text:p>");
    let mut d = crate::odt::import(&bytes).unwrap();
    d.last_section.margin_left = 100.0;
    let saved = crate::odt::export(&d).unwrap();
    let s = crate::odt::tests::styles_of(&saved);
    assert!(s.contains(r#"style:page-layout style:name="pm1" style:page-usage="mirrored""#), "the file's layout: {s}");
    assert!(s.contains(r##"style:color="#ff0000""##) && s.contains(r##"fo:background-color="#eeffee""##), "what WordCraft doesn't model: {s}");
    assert!(s.contains(r#"fo:margin-left="100pt""#) && !s.contains(r#"fo:margin-left="3cm""#), "the change: {s}");
    assert!(s.contains(r#"fo:page-width="21cm""#) && s.contains(r#"fo:margin-right="2cm""#), "the rest, in the file's units: {s}");
    assert!(
        s.contains("<text:p>Kopf</text:p>") && s.contains(r#"style:master-page style:name="Standard" style:page-layout-name="pm1""#),
        "its header: {s}"
    );
    assert_eq!(crate::odt::import(&saved).unwrap().last_section.margin_left, 100.0);
    // Size and orientation.
    let mut d = crate::odt::import(&bytes).unwrap();
    let (w, h) = (d.last_section.page_h, d.last_section.page_w);
    (d.last_section.page_w, d.last_section.page_h) = (w, h);
    let s = crate::odt::tests::styles_of(&crate::odt::export(&d).unwrap());
    assert!(s.contains(r#"style:print-orientation="landscape""#) && s.contains(r##"style:color="#ff0000""##), "{s}");
    // Columns, in the place the schema gives them, before the footnote line.
    let mut d = crate::odt::import(&bytes).unwrap();
    d.last_section.columns.count = 2;
    let saved = crate::odt::export(&d).unwrap();
    let s = crate::odt::tests::styles_of(&saved);
    assert!(s.find("<style:columns").is_some_and(|c| s.find("<style:footnote-sep").is_some_and(|f| c < f)), "{s}");
    assert_eq!(crate::odt::import(&saved).unwrap().last_section.columns.count, 2);
    // Changed back, nothing is left of the change.
    let mut d = crate::odt::import(&bytes).unwrap();
    let was = d.last_section.margin_left;
    d.last_section.margin_left = 50.0;
    d.last_section.margin_left = was;
    assert!(crate::odt::tests::styles_of(&crate::odt::export(&d).unwrap()).contains(r#"fo:margin-left="3cm""#));
}

#[test]
fn a_first_pages_change_goes_on_to_the_pages_after_it() {
    // WordCraft reads a first page style and the pages after it as one section.
    let second = r#"<style:page-layout style:name="pm2"><style:page-layout-properties fo:page-width="21cm" fo:page-height="29.7cm" fo:margin-left="3cm"/></style:page-layout>"#;
    let first = r#"<style:master-page style:name="First" style:page-layout-name="pm2" style:next-style-name="Standard"/>"#;
    let bytes = page_file(second, first, "<text:p>x</text:p>");
    let mut d = crate::odt::import(&bytes).unwrap();
    d.last_section.margin_left = 100.0;
    let s = crate::odt::tests::styles_of(&crate::odt::export(&d).unwrap());
    assert!(s.contains(r#"fo:margin-left="100pt""#) && s.contains(r##"style:color="#ff0000""##), "{s}");
}

#[test]
fn a_first_pages_header_measure_stays_off_later_pages_without_one() {
    // A first page with a header, and later pages with none: a top margin change moves the first
    // page's header band, and gives the later pages' layout no header measurements.
    let ns = crate::odt::tests::NS_T;
    let styles = format!(
        r#"<office:document-styles {ns}><office:automatic-styles><style:page-layout style:name="later"><style:page-layout-properties fo:page-width="21cm" fo:page-height="29.7cm" fo:margin-top="2cm" fo:margin-bottom="2cm" fo:margin-left="2cm" fo:margin-right="2cm"/><style:header-style/><style:footer-style/></style:page-layout><style:page-layout style:name="first"><style:page-layout-properties fo:page-width="21cm" fo:page-height="29.7cm" fo:margin-top="3cm" fo:margin-bottom="2cm" fo:margin-left="2cm" fo:margin-right="2cm"/><style:header-style><style:header-footer-properties fo:min-height="3cm" fo:margin-bottom="0.4cm"/></style:header-style><style:footer-style/></style:page-layout></office:automatic-styles><office:master-styles><style:master-page style:name="Standard" style:page-layout-name="later"/><style:master-page style:name="First" style:page-layout-name="first" style:next-style-name="Standard"><style:header><text:p>Kopf</text:p></style:header></style:master-page></office:master-styles></office:document-styles>"#
    );
    let content = format!(
        r#"<office:document-content {ns}><office:automatic-styles><style:style style:name="P1" style:family="paragraph" style:master-page-name="First"/></office:automatic-styles><office:body><office:text><text:p text:style-name="P1">x</text:p></office:text></office:body></office:document-content>"#
    );
    let bytes = crate::odt::tests::package(&content, Some(&styles));
    let mut d = crate::odt::import(&bytes).unwrap();
    assert!(d.last_section.margin_top > 150.0, "the header's reach is the top margin: {}", d.last_section.margin_top);
    d.last_section.margin_top = 150.0;
    let s = crate::odt::tests::styles_of(&crate::odt::export(&d).unwrap());
    let later = s.split(r#"style:name="later""#).nth(1).and_then(|r| r.split("</style:page-layout>").next()).unwrap();
    assert!(later.contains("<style:header-style/>") && later.contains(r#"fo:margin-top="2cm""#), "later pages: {later}");
    let first = s.split(r#"style:name="first""#).nth(1).and_then(|r| r.split("</style:page-layout>").next()).unwrap();
    assert!(first.contains(r#"fo:margin-top="3cm""#) && !first.contains(r#"fo:min-height="3cm""#), "first page: {first}");
}

#[test]
fn later_pages_header_keeps_its_distance_when_the_first_page_has_none() {
    // A first page with no header, and later pages with one: the later pages' top margin measures
    // to the header, not the text, so the first page's new text margin isn't theirs.
    let ns = crate::odt::tests::NS_T;
    let styles = format!(
        r#"<office:document-styles {ns}><office:automatic-styles><style:page-layout style:name="later"><style:page-layout-properties fo:page-width="21cm" fo:page-height="29.7cm" fo:margin-top="1cm" fo:margin-bottom="2cm" fo:margin-left="2cm" fo:margin-right="2cm"/><style:header-style><style:header-footer-properties fo:min-height="1cm"/></style:header-style></style:page-layout><style:page-layout style:name="first"><style:page-layout-properties fo:page-width="21cm" fo:page-height="29.7cm" fo:margin-top="2cm" fo:margin-bottom="2cm" fo:margin-left="2cm" fo:margin-right="2cm"/><style:header-style/></style:page-layout></office:automatic-styles><office:master-styles><style:master-page style:name="Standard" style:page-layout-name="later"><style:header><text:p>Kopf</text:p></style:header></style:master-page><style:master-page style:name="First" style:page-layout-name="first" style:next-style-name="Standard"/></office:master-styles></office:document-styles>"#
    );
    let content = format!(
        r#"<office:document-content {ns}><office:automatic-styles><style:style style:name="P1" style:family="paragraph" style:master-page-name="First"/></office:automatic-styles><office:body><office:text><text:p text:style-name="P1">x</text:p></office:text></office:body></office:document-content>"#
    );
    let bytes = crate::odt::tests::package(&content, Some(&styles));
    let mut d = crate::odt::import(&bytes).unwrap();
    d.last_section.margin_top = 100.0;
    let saved = crate::odt::export(&d).unwrap();
    let s = crate::odt::tests::styles_of(&saved);
    let later = s.split(r#"style:name="later""#).nth(1).and_then(|r| r.split("</style:page-layout>").next()).unwrap();
    assert!(later.contains(r#"fo:margin-top="1cm""#) && later.contains(r#"fo:min-height="71.65pt""#), "later pages: {later}");
    let first = s.split(r#"style:name="first""#).nth(1).and_then(|r| r.split("</style:page-layout>").next()).unwrap();
    assert!(first.contains(r#"fo:margin-top="100pt""#) && first.contains("<style:header-style/>"), "first page: {first}");
    // Reopened, the change is still there.
    let back = crate::odt::import(&saved).unwrap();
    assert!((back.last_section.margin_top - 100.0).abs() < 0.01, "{}", back.last_section.margin_top);
}

#[test]
fn a_bands_spacing_counts_once_when_the_page_setup_changes() {
    // The band's height and its spacing together reach the text. Written as the height alone,
    // with the spacing again after it, each save moved the text down by the spacing.
    let ns = crate::odt::tests::NS_T;
    let styles = format!(
        r#"<office:document-styles {ns}><office:automatic-styles><style:page-layout style:name="pm1"><style:page-layout-properties fo:page-width="21cm" fo:page-height="29.7cm" fo:margin-top="1cm" fo:margin-bottom="1cm" fo:margin-left="2cm" fo:margin-right="2cm"/><style:header-style><style:header-footer-properties fo:min-height="1cm" fo:margin-bottom="1cm"/></style:header-style><style:footer-style><style:header-footer-properties fo:min-height="1cm" fo:margin-top="0.5cm"/></style:footer-style></style:page-layout></office:automatic-styles><office:master-styles><style:master-page style:name="Standard" style:page-layout-name="pm1"><style:header><text:p>Kopf</text:p></style:header><style:footer><text:p>Fuss</text:p></style:footer></style:master-page></office:master-styles></office:document-styles>"#
    );
    let content =
        format!("<office:document-content {ns}><office:body><office:text><text:p>x</text:p></office:text></office:body></office:document-content>");
    let bytes = crate::odt::tests::package(&content, Some(&styles));
    // The file's page layout, changed.
    let mut d = crate::odt::import(&bytes).unwrap();
    let was = d.last_section.margin_top;
    d.last_section.margin_top = 150.0;
    d.last_section.margin_bottom = 120.0;
    let saved = crate::odt::export(&d).unwrap();
    let s = crate::odt::tests::styles_of(&saved);
    assert!(s.contains(r#"fo:margin-bottom="1cm""#) && s.contains(r#"fo:margin-top="0.5cm""#), "the spacing stays: {s}");
    let back = crate::odt::import(&saved).unwrap();
    assert!((back.last_section.margin_top - 150.0).abs() < 0.02, "{}", back.last_section.margin_top);
    assert!((back.last_section.margin_bottom - 120.0).abs() < 0.02, "{}", back.last_section.margin_bottom);
    // Less room than the spacing takes it from the spacing.
    let mut d = crate::odt::import(&bytes).unwrap();
    d.last_section.margin_top = d.last_section.header + 10.0;
    let back = crate::odt::import(&crate::odt::export(&d).unwrap()).unwrap();
    assert!((back.last_section.margin_top - (back.last_section.header + 10.0)).abs() < 0.02, "{}", back.last_section.margin_top);
    // WordCraft's own page layout, written whole when the kind of page changes.
    let mut d = crate::odt::import(&bytes).unwrap();
    d.last_section.page_num_start = Some(5);
    let back = crate::odt::import(&crate::odt::export(&d).unwrap()).unwrap();
    assert!((back.last_section.margin_top - was).abs() < 0.02, "{} after {was}", back.last_section.margin_top);
    let again = crate::odt::import(&crate::odt::export(&back).unwrap()).unwrap();
    assert!((again.last_section.margin_top - was).abs() < 0.02, "{} after {was}", again.last_section.margin_top);
}

#[test]
fn a_fixed_height_band_stays_one() {
    let ns = crate::odt::tests::NS_T;
    let styles = format!(
        r#"<office:document-styles {ns}><office:automatic-styles><style:page-layout style:name="pm1"><style:page-layout-properties fo:page-width="21cm" fo:page-height="29.7cm" fo:margin-top="1cm" fo:margin-bottom="1cm" fo:margin-left="2cm" fo:margin-right="2cm"/><style:header-style><style:header-footer-properties svg:height="1cm"/></style:header-style></style:page-layout></office:automatic-styles><office:master-styles><style:master-page style:name="Standard" style:page-layout-name="pm1"><style:header><text:p>Kopf</text:p></style:header></style:master-page></office:master-styles></office:document-styles>"#
    );
    let content =
        format!("<office:document-content {ns}><office:body><office:text><text:p>x</text:p></office:text></office:body></office:document-content>");
    let bytes = crate::odt::tests::package(&content, Some(&styles));
    let mut d = crate::odt::import(&bytes).unwrap();
    d.last_section.margin_top = 100.0;
    let saved = crate::odt::export(&d).unwrap();
    let s = crate::odt::tests::styles_of(&saved);
    let layout = s.split(r#"style:name="pm1""#).nth(1).and_then(|r| r.split("</style:page-layout>").next()).unwrap();
    assert!(layout.contains(r#"svg:height="71.65pt""#) && !layout.contains("fo:min-height"), "{layout}");
    assert!((crate::odt::import(&saved).unwrap().last_section.margin_top - 100.0).abs() < 0.02);
}

#[test]
fn later_pages_with_their_own_margin_keep_it() {
    // A first page set apart: what it changes isn't what the later pages say.
    let ns = crate::odt::tests::NS_T;
    let styles = format!(
        r#"<office:document-styles {ns}><office:automatic-styles><style:page-layout style:name="later"><style:page-layout-properties fo:page-width="21cm" fo:page-height="29.7cm" fo:margin-top="2cm" fo:margin-bottom="2cm" fo:margin-left="2cm" fo:margin-right="2cm"/></style:page-layout><style:page-layout style:name="first"><style:page-layout-properties fo:page-width="21cm" fo:page-height="29.7cm" fo:margin-top="5cm" fo:margin-bottom="2cm" fo:margin-left="4cm" fo:margin-right="2cm"/></style:page-layout></office:automatic-styles><office:master-styles><style:master-page style:name="Standard" style:page-layout-name="later"/><style:master-page style:name="First" style:page-layout-name="first" style:next-style-name="Standard"/></office:master-styles></office:document-styles>"#
    );
    let content = format!(
        r#"<office:document-content {ns}><office:automatic-styles><style:style style:name="P1" style:family="paragraph" style:master-page-name="First"/></office:automatic-styles><office:body><office:text><text:p text:style-name="P1">x</text:p></office:text></office:body></office:document-content>"#
    );
    let bytes = crate::odt::tests::package(&content, Some(&styles));
    let mut d = crate::odt::import(&bytes).unwrap();
    d.last_section.margin_top = 100.0;
    d.last_section.margin_left = 90.0;
    d.last_section.margin_right = 80.0;
    let s = crate::odt::tests::styles_of(&crate::odt::export(&d).unwrap());
    let layout = |name: &str| {
        let key = format!(r#"style:name="{name}""#);
        s.split(key.as_str()).nth(1).and_then(|r| r.split("</style:page-layout>").next()).unwrap().to_string()
    };
    let (first, later) = (layout("first"), layout("later"));
    assert!(first.contains(r#"fo:margin-top="100pt""#) && first.contains(r#"fo:margin-left="90pt""#), "{first}");
    assert!(later.contains(r#"fo:margin-top="2cm""#) && later.contains(r#"fo:margin-left="2cm""#), "{later}");
    // Where both said the same, the change is the later pages' too.
    assert!(first.contains(r#"fo:margin-right="80pt""#) && later.contains(r#"fo:margin-right="80pt""#), "{first} {later}");
}

#[test]
fn sections_sharing_a_page_layout_can_still_differ() {
    // One section changed: the page layout they share can't say both, so each gets its own.
    let style = r#"<office:automatic-styles><style:style style:name="P1" style:family="paragraph" style:master-page-name="Standard"/></office:automatic-styles>"#;
    let ns = crate::odt::tests::NS_T;
    let content = format!(
        r#"<office:document-content {ns}>{style}<office:body><office:text><text:p>one</text:p><text:p text:style-name="P1">two</text:p></office:text></office:body></office:document-content>"#
    );
    let styles = r##"<office:automatic-styles><style:page-layout style:name="pm1"><style:page-layout-properties fo:page-width="21cm" fo:page-height="29.7cm" fo:margin-left="3cm" fo:margin-right="2cm" fo:margin-top="2cm" fo:margin-bottom="2cm"/></style:page-layout></office:automatic-styles><office:master-styles><style:master-page style:name="Standard" style:page-layout-name="pm1"/></office:master-styles>"##;
    let bytes = crate::odt::tests::package(&content, Some(&format!("<office:document-styles {ns}>{styles}</office:document-styles>")));
    let mut d = crate::odt::import(&bytes).unwrap();
    assert!(d.body.iter().filter_map(|b| b.as_para()).any(|p| p.section.is_some()), "two sections");
    d.last_section.margin_left = 100.0;
    let back = crate::odt::import(&crate::odt::export(&d).unwrap()).unwrap();
    assert_eq!(back.last_section.margin_left, 100.0);
    let first = back.body.iter().filter_map(|b| b.as_para()).find_map(|p| p.section.as_ref()).map(|s| s.margin_left);
    assert!(first.is_some_and(|m| (m - 3.0 * 72.0 / 2.54).abs() < 0.1), "{first:?}");
}

#[test]
fn hostile_page_definitions_do_not_stop_a_page_setup_change() {
    let layouts =
        r#"<style:page-layout style:name="odd"><style:page-layout-properties fo:margin-left='1in' fo:margin-left2="x" a="b"/></style:page-layout>"#;
    let masters = r#"<style:master-page style:name="A" style:page-layout-name="odd" style:next-style-name="B"/><style:master-page style:name="B" style:page-layout-name="odd" style:next-style-name="A"/><style:master-page style:name="C" style:page-layout-name="none" style:next-style-name="C"/>"#;
    let bytes = page_file(layouts, masters, "<text:p>x</text:p>");
    let mut d = crate::odt::import(&bytes).unwrap();
    d.last_section.margin_left = f32::NAN;
    let _ = crate::odt::export(&d).unwrap();
    d.last_section.margin_left = 1e30;
    d.last_section.page_w = -5.0;
    let saved = crate::odt::export(&d).unwrap();
    assert!(crate::odt::import(&saved).is_ok());
}

/// A margin changed in the same save as the watermark or odd and even pages: the margin alone could
/// stay a change to the file's page layouts, but those two aren't in them, and would be lost.
#[test]
fn a_margin_changed_with_the_watermark_or_odd_and_even_pages_keeps_them() {
    let file = |left: &str| {
        let styles = format!(
            r#"<office:document-styles {NS_T}><office:automatic-styles><style:page-layout style:name="pm1" style:page-usage="mirrored"><style:page-layout-properties fo:page-width="21cm" fo:page-height="29.7cm" fo:margin-top="2cm" fo:margin-bottom="2cm" fo:margin-left="2cm" fo:margin-right="2cm"/><style:header-style><style:header-footer-properties fo:min-height="1cm"/></style:header-style></style:page-layout></office:automatic-styles><office:master-styles><style:master-page style:name="Standard" style:page-layout-name="pm1"><style:header><text:p>Right head</text:p></style:header>{left}</style:master-page></office:master-styles></office:document-styles>"#
        );
        let content = format!(
            r#"<office:document-content {NS_T}><office:body><office:text><text:p>x</text:p></office:text></office:body></office:document-content>"#
        );
        package(&content, Some(&styles))
    };
    let mut d = import(&file("")).unwrap();
    d.last_section.margin_left += 30.0;
    d.settings.watermark = Some(wordcraft_doc::Watermark { text: "DRAFT".into(), ..Default::default() });
    let saved = export(&d).unwrap();
    let s = crate::odt::tests::entry(&saved, "styles.xml");
    assert!(s.contains("PowerPlusWaterMarkObject") && s.contains("DRAFT"), "the watermark: {s}");
    let back = import(&saved).unwrap();
    assert_eq!(back.settings.watermark.map(|w| w.text).as_deref(), Some("DRAFT"));
    assert!((back.last_section.margin_left - d.last_section.margin_left).abs() < 0.5, "the margin: {s}");

    let mut d = import(&file("<style:header-left><text:p>Left head</text:p></style:header-left>")).unwrap();
    assert!(d.settings.even_odd_headers);
    d.last_section.margin_left += 30.0;
    d.settings.even_odd_headers = false;
    let saved = export(&d).unwrap();
    let s = crate::odt::tests::entry(&saved, "styles.xml");
    assert!(!s.contains("<style:header-left"), "the left pages' header: {s}");
    let back = import(&saved).unwrap();
    assert!(!back.settings.even_odd_headers);
    assert!((back.last_section.margin_left - d.last_section.margin_left).abs() < 0.5, "the margin: {s}");
}
