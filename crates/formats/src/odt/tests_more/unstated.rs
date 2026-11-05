//! WordCraft's choices where the standard leaves something open (`docs/odf-interpretation.md`)
//! fill in only what a file leaves unsaid: a value the file states is the file's.

use crate::odt::import;
use crate::odt::tests::NS_T;
use wordcraft_doc::para::InlineObject;
use wordcraft_doc::props::Rgb;

const NS_D: &str =
    r#"xmlns:draw="urn:oasis:names:tc:opendocument:xmlns:drawing:1.0" xmlns:svg="urn:oasis:names:tc:opendocument:xmlns:svg-compatible:1.0""#;

fn cm(v: f32) -> f32 {
    v * 72.0 / 2.54
}

#[test]
fn values_a_file_states_are_kept() {
    // A file that states every value WordCraft could fill in: header height and spacing and
    // whether it is dynamic, page columns and their gap, tabs from the indent, a shape's fill and
    // line.
    let styles = format!(
        r##"<office:document-styles {NS_T} {NS_D}><office:automatic-styles><style:page-layout style:name="pm1"><style:page-layout-properties fo:page-width="21cm" fo:page-height="29.7cm" fo:margin-top="2cm" fo:margin-bottom="2cm" fo:margin-left="2cm" fo:margin-right="2cm"><style:columns fo:column-count="2" fo:column-gap="1cm"/></style:page-layout-properties><style:header-style><style:header-footer-properties fo:min-height="1cm" fo:margin-bottom="0.5cm" style:dynamic-spacing="false"/></style:header-style></style:page-layout></office:automatic-styles><office:master-styles><style:master-page style:name="Standard" style:page-layout-name="pm1"><style:header><text:p>H</text:p></style:header></style:master-page></office:master-styles></office:document-styles>"##
    );
    let content = format!(
        r##"<office:document-content {NS_T} {NS_D}><office:automatic-styles><style:style style:name="P1" style:family="paragraph"><style:paragraph-properties fo:margin-left="0.5in"><style:tab-stops><style:tab-stop style:position="1in"/></style:tab-stops></style:paragraph-properties></style:style><style:style style:name="gr1" style:family="graphic"><style:graphic-properties draw:fill="solid" draw:fill-color="#112233" draw:stroke="solid" svg:stroke-color="#445566" svg:stroke-width="2pt"/></style:style></office:automatic-styles><office:body><office:text><text:p text:style-name="P1">a<draw:rect draw:style-name="gr1" text:anchor-type="as-char" svg:width="1in" svg:height="1in"/></text:p></office:text></office:body></office:document-content>"##
    );
    let mut z = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let o = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    let settings = r#"<office:document-settings xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:config="urn:oasis:names:tc:opendocument:xmlns:config:1.0"><office:settings><config:config-item-set config:name="ooo:configuration-settings"><config:config-item config:name="TabsRelativeToIndent" config:type="boolean">true</config:config-item></config:config-item-set></office:settings></office:document-settings>"#;
    for (name, data) in [
        ("mimetype", "application/vnd.oasis.opendocument.text"),
        ("content.xml", content.as_str()),
        ("styles.xml", styles.as_str()),
        ("settings.xml", settings),
    ] {
        z.start_file(name, o).unwrap();
        std::io::Write::write_all(&mut z, data.as_bytes()).unwrap();
    }
    let bytes = z.finish().unwrap().into_inner();
    let near = |a: f32, b: f32| (a - b).abs() < 0.05;
    let d = import(&bytes).unwrap();
    let s = &d.last_section;
    assert!(near(s.header_gap, cm(0.5)), "the stated spacing, kept: {}", s.header_gap);
    assert!(near(s.margin_top, cm(3.5)), "the stated height and spacing under the margin: {}", s.margin_top);
    assert!(near(s.margin_left, cm(2.0)) && near(s.margin_bottom, cm(2.0)), "the stated margins");
    assert_eq!(s.columns.count, 2);
    assert!(near(s.columns.space, cm(1.0)), "{}", s.columns.space);
    let p = d.body[0].as_para().unwrap();
    assert_eq!(p.props.tabs.clone().unwrap_or_default().first().map(|t| t.pos), Some(108.0));
    let shape = p.objects.iter().find_map(|o| match o {
        InlineObject::Shape { fill, stroke, stroke_width, .. } => Some((fill.as_ref().map(|f| f.main_color()), *stroke, *stroke_width)),
        _ => None,
    });
    assert_eq!(shape, Some((Some(Rgb(0x11, 0x22, 0x33)), Some(Rgb(0x44, 0x55, 0x66)), 2.0)));
}

#[test]
fn a_sections_column_balance_is_its_own_and_is_written() {
    // §20.428 `text:dont-balance-text-columns`: read as its name says; unstated, as Word: balanced
    // only when text follows on the page.
    let doc = |props: &str| {
        let content = format!(
            r#"<office:document-content {NS_T}><office:automatic-styles><style:style style:name="S" style:family="section"><style:section-properties {props}><style:columns fo:column-count="2" fo:column-gap="0.5cm"/></style:section-properties></style:style></office:automatic-styles><office:body><office:text><text:p>a</text:p><text:section text:name="S1" text:style-name="S"><text:p>b</text:p></text:section></office:text></office:body></office:document-content>"#
        );
        crate::odt::tests::package(&content, None)
    };
    let balance = |props: &str| import(&doc(props)).unwrap().last_section.balance_columns;
    assert_eq!(balance(r#"text:dont-balance-text-columns="true""#), Some(false));
    assert_eq!(balance(r#"text:dont-balance-text-columns="false""#), Some(true));
    assert_eq!(balance(""), None);
    // Written as WordCraft lays it out, so every reader balances it the same.
    for (props, written) in [("", "true"), (r#"text:dont-balance-text-columns="false""#, "false")] {
        let d = import(&doc(props)).unwrap();
        let saved = crate::odt::export(&d).unwrap();
        let content = crate::odt::tests::entry(&saved, "content.xml");
        assert!(content.contains(&format!(r#"text:dont-balance-text-columns="{written}""#)), "{props}: {content}");
    }
}

#[test]
fn paragraph_spacing_adds_up_as_the_document_says_else_takes_the_larger() {
    // LibreOffice's `AddParaTableSpacing` (settings.xml; false: the larger of the two, observed)
    // says how the space between paragraphs adds up; without it, the larger, as Word. Written back
    // as WordCraft lays the document out, in a set LibreOffice reads (the `ooo` namespace declared).
    let content =
        format!("<office:document-content {NS_T}><office:body><office:text><text:p>a</text:p></office:text></office:body></office:document-content>");
    let with = |add: Option<bool>| {
        let mut z = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let o = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
        let mut parts = vec![("mimetype", "application/vnd.oasis.opendocument.text".to_string()), ("content.xml", content.clone())];
        if let Some(m) = add {
            parts.push(("settings.xml", format!(r#"<office:document-settings xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:config="urn:oasis:names:tc:opendocument:xmlns:config:1.0" xmlns:ooo="http://openoffice.org/2004/office"><office:settings><config:config-item-set config:name="ooo:configuration-settings"><config:config-item config:name="AddParaTableSpacing" config:type="boolean">{m}</config:config-item></config:config-item-set></office:settings></office:document-settings>"#)));
        }
        for (name, data) in parts {
            z.start_file(name, o).unwrap();
            std::io::Write::write_all(&mut z, data.as_bytes()).unwrap();
        }
        z.finish().unwrap().into_inner()
    };
    assert!(import(&with(Some(true))).unwrap().settings.sum_paragraph_spacing);
    assert!(!import(&with(Some(false))).unwrap().settings.sum_paragraph_spacing);
    assert!(!import(&with(None)).unwrap().settings.sum_paragraph_spacing);
    for (add, written) in [(Some(true), "true"), (None, "false")] {
        let d = import(&with(add)).unwrap();
        let saved = crate::odt::export(&d).unwrap();
        let settings = crate::odt::tests::entry(&saved, "settings.xml");
        assert!(settings.contains(&format!(r#"config:name="AddParaTableSpacing" config:type="boolean">{written}<"#)), "{add:?}: {settings}");
        assert_eq!(settings.matches("\"AddParaTableSpacing\"").count(), 1);
        assert!(settings.contains("xmlns:ooo="), "LibreOffice reads the set only with its namespace");
    }
}

#[test]
fn widow_control_is_the_files_else_on() {
    // `fo:widows`/`fo:orphans` refer to XSL's (initial value 2); LibreOffice reads none stated as
    // none. Stated in the default style (or Standard), it is the file's; unstated, on; written
    // from the model (after an edit), stated either way, so every reader takes it the same. An
    // unedited file keeps its own statement, or its silence (see `kept`).
    let doc = |default: &str| {
        let styles = format!(
            r#"<office:document-styles {NS_T}><office:styles><style:default-style style:family="paragraph"><style:paragraph-properties {default}/></style:default-style></office:styles></office:document-styles>"#
        );
        let content = format!(
            "<office:document-content {NS_T}><office:body><office:text><text:p>a</text:p></office:text></office:body></office:document-content>"
        );
        crate::odt::tests::package(&content, Some(&styles))
    };
    let on = |bytes: &[u8]| import(bytes).unwrap().styles.resolve_para(&Default::default()).widow_control;
    let stated_off = r#"fo:widows="0" fo:orphans="0""#;
    assert!(on(&doc(r#"fo:widows="2" fo:orphans="2""#)));
    assert!(!on(&doc(stated_off)));
    assert!(on(&doc("")));
    for (default, written, widows) in [("", r#"fo:widows="2" fo:orphans="2""#, true), (stated_off, stated_off, false)] {
        let saved = crate::odt::tests::export_model(&import(&doc(default)).unwrap()).unwrap();
        let styles = crate::odt::tests::entry(&saved, "styles.xml");
        assert!(styles.contains(written), "{default}: {styles}");
        assert_eq!(on(&saved), widows);
    }
}
