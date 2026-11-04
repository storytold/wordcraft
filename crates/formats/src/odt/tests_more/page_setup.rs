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
fn a_document_without_a_page_layout_has_the_default_paper_and_the_profiles_margins() {
    // No styles.xml at all: the region's paper (the standard leaves the size to the
    // application), and the profile's margins (the standard leaves them open too): WordCraft's
    // 0, XSL's initial value; LibreOffice's 2 cm. It is laid out as an ODF document.
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
    let lo = crate::import_with("odt", &crate::odt::tests::package(&content, None), &a4, &wordcraft_doc::odf_profile::OdfProfile::libreoffice())
        .unwrap()
        .unwrap();
    let cm2 = 2.0 * 72.0 / 2.54;
    assert_eq!([lo.last_section.margin_top, lo.last_section.margin_left], [cm2; 2]);
}

#[test]
fn a_page_layout_stating_nothing_has_the_profiles_margins() {
    // LibreOffice's CV template: a page layout with no properties at all. Its margins are the
    // profile's: WordCraft's 0 (XSL's initial value), LibreOffice's 2 cm.
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
    let lo = |styles: &str| {
        crate::odt::tests::import_as(&crate::odt::tests::package(&content, Some(styles)), &wordcraft_doc::odf_profile::OdfProfile::libreoffice())
            .last_section
    };
    let cm2 = 2.0 * 72.0 / 2.54;
    let s = lo(&styles);
    assert_eq!([s.margin_top, s.margin_bottom, s.margin_left, s.margin_right], [cm2; 4]);
    let s = lo(&one);
    assert_eq!((s.margin_left, s.margin_top), (72.0, cm2), "the stated one is the file's under every preset");
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
