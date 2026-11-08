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

/// An ODT document with a page background and an active header containing a logo
/// preserves the page background (both modern draw:fill and legacy fallback) and the header logo (#5803).
#[test]
fn background_plus_small_header_logo_keeps_background() {
    let png = {
        let img = image::RgbImage::from_pixel(8, 4, image::Rgb([200, 30, 30]));
        let mut out = std::io::Cursor::new(Vec::new());
        img.write_to(&mut out, image::ImageFormat::Png).unwrap();
        out.into_inner()
    };
    let logo_png = {
        let img = image::RgbImage::from_pixel(4, 4, image::Rgb([30, 200, 30]));
        let mut out = std::io::Cursor::new(Vec::new());
        img.write_to(&mut out, image::ImageFormat::Png).unwrap();
        out.into_inner()
    };
    let mut d = Document::new();
    let bg_media = d.add_media(png, "png");
    d.settings.page_picture = Some(wordcraft_doc::PagePicture { media: bg_media, fill_type: "frame".into(), title: "PageBackground".into() });
    let logo_media = d.add_media(logo_png, "png");
    let mut hp = wordcraft_doc::Paragraph::new();
    hp.text = format!("Company {}", wordcraft_doc::para::OBJ);
    hp.objects.push(wordcraft_doc::para::InlineObject::Image {
        media: logo_media,
        w: 50.0,
        h: 20.0,
        alt: "Logo".into(),
        float: Default::default(),
        crop: [0.0; 4],
        fx: Default::default(),
        ole: None,
    });
    hp.runs = vec![
        wordcraft_doc::para::Run { len: 8, props: Default::default() },
        wordcraft_doc::para::Run { len: wordcraft_doc::para::OBJ.len_utf8(), props: Default::default() },
    ];
    d.parts.insert(
        1,
        wordcraft_doc::Part { kind: wordcraft_doc::PartKind::Header, blocks: vec![wordcraft_doc::para_block(hp)], body: Default::default() },
    );
    d.last_section.headers.default = Some(1);

    let saved = export(&d).unwrap();
    let s = crate::odt::tests::entry(&saved, "styles.xml");
    // Verify both draw:fill="bitmap" and style:background-image are emitted consistently
    assert!(s.contains("draw:fill=\"bitmap\""), "page layout must declare draw:fill=\"bitmap\": {s}");
    assert!(s.contains("style:background-image"), "legacy background-image fallback must be present: {s}");
    assert!(s.contains("<draw:fill-image"), "draw:fill-image must be declared in office:styles: {s}");

    // Reading the saved ODT back preserves page_picture
    let back = import(&saved).unwrap();
    assert!(back.settings.page_picture.is_some(), "saved ODT must preserve page background picture");
}

/// An ODT document with a page background and a full-page behind-text header picture
/// preserves both the header picture and the page background on export and re-import (#5803).
#[test]
fn full_page_header_picture_with_background_round_trips() {
    let bg_png = {
        let img = image::RgbImage::from_pixel(8, 4, image::Rgb([200, 30, 30]));
        let mut out = std::io::Cursor::new(Vec::new());
        img.write_to(&mut out, image::ImageFormat::Png).unwrap();
        out.into_inner()
    };
    let header_png = {
        let img = image::RgbImage::from_pixel(4, 8, image::Rgb([30, 30, 200]));
        let mut out = std::io::Cursor::new(Vec::new());
        img.write_to(&mut out, image::ImageFormat::Png).unwrap();
        out.into_inner()
    };
    let mut d = Document::new();
    let bg_media = d.add_media(bg_png.clone(), "png");
    d.settings.page_picture = Some(wordcraft_doc::PagePicture { media: bg_media, fill_type: "frame".into(), title: "PageBackground".into() });
    let header_media = d.add_media(header_png.clone(), "png");
    let mut hp = wordcraft_doc::Paragraph::new();
    hp.text = format!("{}", wordcraft_doc::para::OBJ);
    hp.objects.push(wordcraft_doc::para::InlineObject::Image {
        media: header_media,
        w: 595.0,
        h: 842.0,
        alt: "Full page header picture".into(),
        float: wordcraft_doc::para::Float { wrap: wordcraft_doc::para::Wrap::BehindText, ..Default::default() },
        crop: [0.0; 4],
        fx: Default::default(),
        ole: None,
    });
    hp.runs = vec![wordcraft_doc::para::Run { len: hp.text.len(), props: Default::default() }];
    d.parts.insert(
        1,
        wordcraft_doc::Part { kind: wordcraft_doc::PartKind::Header, blocks: vec![wordcraft_doc::para_block(hp)], body: Default::default() },
    );
    d.last_section.headers.default = Some(1);

    let saved = export(&d).unwrap();
    let s = crate::odt::tests::entry(&saved, "styles.xml");
    assert!(s.contains("draw:fill=\"bitmap\""), "page layout must declare draw:fill=\"bitmap\": {s}");
    assert!(s.contains("style:background-image"), "legacy background-image fallback must be present: {s}");
    assert!(s.contains("<draw:fill-image"), "draw:fill-image must be declared in office:styles: {s}");

    let fill_name =
        s.split("draw:fill-image-name=\"").nth(1).and_then(|x| x.split('"').next()).expect("styles.xml must contain draw:fill-image-name");
    let draw_fill_pattern = format!("<draw:fill-image draw:name=\"{fill_name}\" xlink:href=\"");
    let draw_href = s.split(&draw_fill_pattern).nth(1).and_then(|x| x.split('"').next()).expect("draw:fill-image must define xlink:href");
    let legacy_href = s
        .split("<style:background-image xlink:href=\"")
        .nth(1)
        .and_then(|x| x.split('"').next())
        .expect("legacy background-image must define xlink:href");
    assert_eq!(draw_href, legacy_href, "modern and legacy background image references must match");

    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(&saved)).unwrap();
    let mut bg_entry = archive.by_name(draw_href).expect("packaged background picture must exist");
    let mut bg_bytes = Vec::new();
    std::io::Read::read_to_end(&mut bg_entry, &mut bg_bytes).unwrap();
    assert_eq!(bg_bytes, bg_png, "packaged background image bytes must match");

    let back = import(&saved).unwrap();
    let bg = back.settings.page_picture.as_ref().expect("page background picture must survive round trip");
    assert_eq!(back.media.get(&bg.media).map(|m| m.as_slice()), Some(bg_png.as_slice()), "imported background bytes must match");
    let header_part = back.last_section.headers.default.and_then(|id| back.parts.get(&id)).expect("header part must survive round trip");
    let header_img_media = header_part.blocks.iter().find_map(|b| {
        b.as_para().and_then(|p| {
            p.objects.iter().find_map(|o| if let wordcraft_doc::para::InlineObject::Image { media, .. } = o { Some(media.as_str()) } else { None })
        })
    });
    assert!(header_img_media.is_some(), "header part must contain image object");
    let header_img_media = header_img_media.unwrap();
    assert_eq!(back.media.get(header_img_media).map(|m| m.as_slice()), Some(header_png.as_slice()), "imported header image bytes must match");
}

/// A table cell style with a background image survives the kept styles path (#5803).
#[test]
fn cell_style_picture_survives_kept_path() {
    let png = {
        let img = image::RgbImage::from_pixel(8, 4, image::Rgb([200, 30, 30]));
        let mut out = std::io::Cursor::new(Vec::new());
        img.write_to(&mut out, image::ImageFormat::Png).unwrap();
        out.into_inner()
    };
    let ns = crate::odt::tests::NS_T;
    let styles = format!(
        r#"<office:document-styles {ns}><office:styles><style:style style:name="CellBg" style:family="table-cell"><style:table-cell-properties><style:background-image xlink:href="Pictures/cell-bg.png" xlink:type="simple" xlink:actuate="onLoad"/></style:table-cell-properties></style:style></office:styles></office:document-styles>"#
    );
    let content = format!(
        r#"<office:document-content {ns}><office:body><office:text><text:p>Cell test</text:p></office:text></office:body></office:document-content>"#
    );
    let pkg = crate::odt::tests::zip_of(&[("content.xml", content.as_bytes()), ("styles.xml", styles.as_bytes()), ("Pictures/cell-bg.png", &png)]);
    let d = import(&pkg).unwrap();
    let saved = export(&d).unwrap();
    let s = crate::odt::tests::entry(&saved, "styles.xml");
    assert!(s.contains("style:background-image"), "cell style background-image must survive on kept path: {s}");
    assert!(s.contains("Pictures/cell-bg.png"), "cell background picture path must survive: {s}");
}

/// An ODT document with a text-only header retains its legacy page background image
/// and writes the image into the package archive.
#[test]
fn text_header_keeps_page_background_picture() {
    let png = {
        let img = image::RgbImage::from_pixel(8, 4, image::Rgb([200, 30, 30]));
        let mut out = std::io::Cursor::new(Vec::new());
        img.write_to(&mut out, image::ImageFormat::Png).unwrap();
        out.into_inner()
    };
    let mut d = Document::new();
    let bg_media = d.add_media(png, "png");
    d.settings.page_picture = Some(wordcraft_doc::PagePicture { media: bg_media, fill_type: "frame".into(), title: String::new() });
    let mut hp = wordcraft_doc::Paragraph::new();
    hp.text = "Header text".into();
    hp.runs = vec![wordcraft_doc::para::Run { len: hp.text.len(), props: Default::default() }];
    d.parts.insert(
        1,
        wordcraft_doc::Part { kind: wordcraft_doc::PartKind::Header, blocks: vec![wordcraft_doc::para_block(hp)], body: Default::default() },
    );
    d.last_section.headers.default = Some(1);

    let saved = export(&d).unwrap();
    let s = crate::odt::tests::entry(&saved, "styles.xml");
    assert!(s.contains("style:background-image"), "page background must be written when header has no picture: {s}");
    assert!(s.contains("Pictures/page-background.png"));
}

/// Every picture written into the ODT zip archive must be referenced by styles.xml or content.xml (#5803).
#[test]
fn no_unreferenced_images_left_in_package() {
    let png = {
        let img = image::RgbImage::from_pixel(8, 4, image::Rgb([200, 30, 30]));
        let mut out = std::io::Cursor::new(Vec::new());
        img.write_to(&mut out, image::ImageFormat::Png).unwrap();
        out.into_inner()
    };
    let ns = crate::odt::tests::NS_T;
    let nsd = r#"xmlns:draw="urn:oasis:names:tc:opendocument:xmlns:drawing:1.0" xmlns:xlink="http://www.w3.org/1999/xlink""#;
    // An imported ODT with modern draw:fill="bitmap" and draw:fill-image
    let styles = format!(
        r#"<office:document-styles {ns} {nsd}><office:styles><draw:fill-image draw:name="Bg1" xlink:href="Pictures/original.png" xlink:type="simple"/><style:style style:name="Standard" style:family="paragraph"/></office:styles><office:automatic-styles><style:page-layout style:name="pm1"><style:page-layout-properties fo:page-width="8.5in" fo:page-height="11in" draw:fill="bitmap" draw:fill-image-name="Bg1"><style:background-image xlink:href="Pictures/original.png" xlink:type="simple" xlink:actuate="onLoad" style:repeat="stretch"/></style:page-layout-properties></style:page-layout></office:automatic-styles><office:master-styles><style:master-page style:name="Standard" style:page-layout-name="pm1"/></office:master-styles></office:document-styles>"#
    );
    let content = format!(
        r#"<office:document-content {ns}><office:body><office:text><text:p>Body text.</text:p></office:text></office:body></office:document-content>"#
    );
    let pkg = crate::odt::tests::zip_of(&[("content.xml", content.as_bytes()), ("styles.xml", styles.as_bytes()), ("Pictures/original.png", &png)]);
    let mut d = import(&pkg).unwrap();

    // Now insert a header picture into the document, changing page setup
    let header_media = d.add_media(png.clone(), "png");
    let mut hp = wordcraft_doc::Paragraph::new();
    hp.text = format!("{}", wordcraft_doc::para::OBJ);
    hp.objects.push(wordcraft_doc::para::InlineObject::Image {
        media: header_media,
        w: 100.0,
        h: 50.0,
        alt: "Header logo".into(),
        float: Default::default(),
        crop: [0.0; 4],
        fx: Default::default(),
        ole: None,
    });
    hp.runs = vec![wordcraft_doc::para::Run { len: hp.text.len(), props: Default::default() }];
    d.parts.insert(
        1,
        wordcraft_doc::Part { kind: wordcraft_doc::PartKind::Header, blocks: vec![wordcraft_doc::para_block(hp)], body: Default::default() },
    );
    d.last_section.headers.default = Some(1);

    let saved = export(&d).unwrap();
    let styles_xml = crate::odt::tests::entry(&saved, "styles.xml");
    let content_xml = crate::odt::tests::entry(&saved, "content.xml");

    // Verify modern draw:fill bitmap and legacy style:background-image resolve to the same image target
    let fill_name =
        styles_xml.split("draw:fill-image-name=\"").nth(1).and_then(|s| s.split('"').next()).expect("styles.xml must contain draw:fill-image-name");
    let draw_fill_pattern = format!("<draw:fill-image draw:name=\"{fill_name}\" xlink:href=\"");
    let draw_href = styles_xml.split(&draw_fill_pattern).nth(1).and_then(|s| s.split('"').next()).expect("draw:fill-image must define xlink:href");
    let legacy_href = styles_xml
        .split("<style:background-image xlink:href=\"")
        .nth(1)
        .and_then(|s| s.split('"').next())
        .expect("legacy background-image must define xlink:href");
    assert_eq!(draw_href, legacy_href, "modern and legacy background image references must match");

    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(&saved)).unwrap();
    let file_names: Vec<String> = (0..archive.len()).map(|i| archive.by_index(i).unwrap().name().to_string()).collect();

    let pic_files: Vec<&String> = file_names.iter().filter(|n| n.starts_with("Pictures/")).collect();
    assert!(!pic_files.is_empty(), "package should contain picture files");

    for pic in pic_files {
        let is_referenced = styles_xml.contains(pic.as_str()) || content_xml.contains(pic.as_str());
        assert!(is_referenced, "package image {pic} must be referenced in styles.xml or content.xml: styles={styles_xml}");
    }
}

/// An inactive header (e.g. `first` when `title_page` is false, or `even` when `even_odd_headers`
/// is false) containing an image does NOT suppress the page background picture (#5803).
#[test]
fn inactive_header_picture_does_not_suppress_page_background() {
    let png = {
        let img = image::RgbImage::from_pixel(8, 4, image::Rgb([200, 30, 30]));
        let mut out = std::io::Cursor::new(Vec::new());
        img.write_to(&mut out, image::ImageFormat::Png).unwrap();
        out.into_inner()
    };
    let mut d = Document::new();
    let bg_media = d.add_media(png.clone(), "png");
    d.settings.page_picture = Some(wordcraft_doc::PagePicture { media: bg_media, fill_type: "frame".into(), title: String::new() });

    let header_media = d.add_media(png, "png");
    let mut hp = wordcraft_doc::Paragraph::new();
    hp.text = format!("{}", wordcraft_doc::para::OBJ);
    hp.objects.push(wordcraft_doc::para::InlineObject::Image {
        media: header_media,
        w: 100.0,
        h: 100.0,
        alt: "Inactive header picture".into(),
        float: Default::default(),
        crop: [0.0; 4],
        fx: Default::default(),
        ole: None,
    });
    hp.runs = vec![wordcraft_doc::para::Run { len: hp.text.len(), props: Default::default() }];
    d.parts.insert(
        100,
        wordcraft_doc::Part { kind: wordcraft_doc::PartKind::Header, blocks: vec![wordcraft_doc::para_block(hp)], body: Default::default() },
    );

    // Point headers.first to the header with picture, but title_page is FALSE.
    d.last_section.headers.first = Some(100);
    d.last_section.title_page = false;

    let saved = export(&d).unwrap();
    let s = crate::odt::tests::entry(&saved, "styles.xml");
    assert!(s.contains("style:background-image"), "background-image must be preserved when header picture is in an inactive header: {s}");
    assert!(s.contains("Pictures/page-background.png"));
}

/// A noninitial continuous section header containing a picture does NOT produce a master page
/// and does not suppress the page background (#5803).
#[test]
fn continuous_section_header_picture_does_not_suppress_page_background() {
    let png = {
        let img = image::RgbImage::from_pixel(8, 4, image::Rgb([200, 30, 30]));
        let mut out = std::io::Cursor::new(Vec::new());
        img.write_to(&mut out, image::ImageFormat::Png).unwrap();
        out.into_inner()
    };
    let mut d = Document::new();
    let bg_media = d.add_media(png.clone(), "png");
    d.settings.page_picture = Some(wordcraft_doc::PagePicture { media: bg_media, fill_type: "frame".into(), title: String::new() });

    // Body with a first section ending paragraph
    let mut p1 = wordcraft_doc::Paragraph::new();
    p1.text = "First paragraph in section 1".into();
    p1.runs = vec![wordcraft_doc::para::Run { len: p1.text.len(), props: Default::default() }];
    p1.section = Some(Box::new(wordcraft_doc::SectionProps::default()));
    d.body.push(wordcraft_doc::para_block(p1));

    // Second section (last_section) is continuous
    let header_media = d.add_media(png, "png");
    let mut hp = wordcraft_doc::Paragraph::new();
    hp.text = format!("{}", wordcraft_doc::para::OBJ);
    hp.objects.push(wordcraft_doc::para::InlineObject::Image {
        media: header_media,
        w: 100.0,
        h: 100.0,
        alt: "Continuous section header picture".into(),
        float: Default::default(),
        crop: [0.0; 4],
        fx: Default::default(),
        ole: None,
    });
    hp.runs = vec![wordcraft_doc::para::Run { len: hp.text.len(), props: Default::default() }];
    d.parts.insert(
        200,
        wordcraft_doc::Part { kind: wordcraft_doc::PartKind::Header, blocks: vec![wordcraft_doc::para_block(hp)], body: Default::default() },
    );

    d.last_section.start = wordcraft_doc::section::SectionStart::Continuous;
    d.last_section.headers.default = Some(200);

    let saved = export(&d).unwrap();
    let s = crate::odt::tests::entry(&saved, "styles.xml");
    assert!(
        s.contains("style:background-image"),
        "page background must not be suppressed by continuous section header that emits no master page: {s}"
    );
    assert!(s.contains("Pictures/page-background.png"));
}

/// A non-continuous `last_section` without any body blocks following the section break
/// does not produce an exported page and does not suppress the page background (#5803).
#[test]
fn trailing_empty_last_section_header_picture_does_not_suppress_page_background() {
    let png = {
        let img = image::RgbImage::from_pixel(8, 4, image::Rgb([200, 30, 30]));
        let mut out = std::io::Cursor::new(Vec::new());
        img.write_to(&mut out, image::ImageFormat::Png).unwrap();
        out.into_inner()
    };
    let mut d = Document::new();
    let bg_media = d.add_media(png.clone(), "png");
    d.settings.page_picture = Some(wordcraft_doc::PagePicture { media: bg_media, fill_type: "frame".into(), title: String::new() });

    // Body with a single paragraph ending section 1 (nothing follows it)
    let mut p1 = wordcraft_doc::Paragraph::new();
    p1.text = "Sole paragraph ending section 1".into();
    p1.runs = vec![wordcraft_doc::para::Run { len: p1.text.len(), props: Default::default() }];
    p1.section = Some(Box::new(wordcraft_doc::SectionProps::default()));
    d.body.push(wordcraft_doc::para_block(p1));

    // last_section is NextPage and has a header with a picture, but has no blocks following
    let header_media = d.add_media(png, "png");
    let mut hp = wordcraft_doc::Paragraph::new();
    hp.text = format!("{}", wordcraft_doc::para::OBJ);
    hp.objects.push(wordcraft_doc::para::InlineObject::Image {
        media: header_media,
        w: 100.0,
        h: 100.0,
        alt: "Unused trailing section header picture".into(),
        float: Default::default(),
        crop: [0.0; 4],
        fx: Default::default(),
        ole: None,
    });
    hp.runs = vec![wordcraft_doc::para::Run { len: hp.text.len(), props: Default::default() }];
    d.parts.insert(
        300,
        wordcraft_doc::Part { kind: wordcraft_doc::PartKind::Header, blocks: vec![wordcraft_doc::para_block(hp)], body: Default::default() },
    );

    d.last_section.start = wordcraft_doc::section::SectionStart::NextPage;
    d.last_section.headers.default = Some(300);

    let saved = export(&d).unwrap();
    let s = crate::odt::tests::entry(&saved, "styles.xml");
    assert!(s.contains("style:background-image"), "page background must not be suppressed by an unexported trailing last_section header: {s}");
    assert!(s.contains("Pictures/page-background.png"));
}
