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
fn a_page_layout_stating_nothing_has_libreoffices_margins() {
    // LibreOffice's CV template: a page layout with no properties at all. LibreOffice draws it
    // on its default paper with 2 cm margins.
    let ns = crate::odt::tests::NS_T;
    let styles = format!(
        r#"<office:document-styles {ns}><office:automatic-styles><style:page-layout style:name="Mpm1"><style:header-style/><style:footer-style/></style:page-layout></office:automatic-styles><office:master-styles><style:master-page style:name="Standard" style:page-layout-name="Mpm1"/></office:master-styles></office:document-styles>"#
    );
    let content =
        format!("<office:document-content {ns}><office:body><office:text><text:p>x</text:p></office:text></office:body></office:document-content>");
    let d = crate::import("odt", &crate::odt::tests::package(&content, Some(&styles))).unwrap().unwrap();
    let s = &d.last_section;
    let cm2 = 2.0 * 72.0 / 2.54;
    assert_eq!([s.margin_top, s.margin_bottom, s.margin_left, s.margin_right], [cm2; 4]);
    // One margin stated: that one is the file's.
    let one = styles.replace("<style:header-style/>", r#"<style:page-layout-properties fo:margin-left="1in"/><style:header-style/>"#);
    let d = crate::import("odt", &crate::odt::tests::package(&content, Some(&one))).unwrap().unwrap();
    assert_eq!((d.last_section.margin_left, d.last_section.margin_top), (72.0, cm2));
}
