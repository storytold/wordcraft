//! The ODF interpretation profile (`docs/odf-interpretation.md`) fills in only what a file leaves
//! unsaid: whatever the preset, a value the file states is the file's.

use crate::odt::tests::{NS_T, import_as};
use wordcraft_doc::odf_profile::{OdfPreset, OdfProfile};
use wordcraft_doc::para::InlineObject;
use wordcraft_doc::props::Rgb;

const NS_D: &str =
    r#"xmlns:draw="urn:oasis:names:tc:opendocument:xmlns:drawing:1.0" xmlns:svg="urn:oasis:names:tc:opendocument:xmlns:svg-compatible:1.0""#;

fn cm(v: f32) -> f32 {
    v * 72.0 / 2.54
}

#[test]
fn values_a_file_states_are_kept_under_every_preset() {
    // A file that states every value a preset could fill in: header height and spacing and
    // whether it is dynamic, page columns and their gap, tabs from the margin, a shape's fill and
    // line.
    let styles = format!(
        r##"<office:document-styles {NS_T} {NS_D}><office:automatic-styles><style:page-layout style:name="pm1"><style:page-layout-properties fo:page-width="21cm" fo:page-height="29.7cm" fo:margin-top="2cm" fo:margin-bottom="2cm" fo:margin-left="2cm" fo:margin-right="2cm"><style:columns fo:column-count="2" fo:column-gap="1cm"/></style:page-layout-properties><style:header-style><style:header-footer-properties fo:min-height="1cm" fo:margin-bottom="0.5cm" style:dynamic-spacing="false"/></style:header-style></style:page-layout></office:automatic-styles><office:master-styles><style:master-page style:name="Standard" style:page-layout-name="pm1"><style:header><text:p>H</text:p></style:header></style:master-page></office:master-styles></office:document-styles>"##
    );
    let content = format!(
        r##"<office:document-content {NS_T} {NS_D}><office:automatic-styles><style:style style:name="P1" style:family="paragraph"><style:paragraph-properties fo:margin-left="0.5in"><style:tab-stops><style:tab-stop style:position="1in"/></style:tab-stops></style:paragraph-properties></style:style><style:style style:name="gr1" style:family="graphic"><style:graphic-properties draw:fill="solid" draw:fill-color="#112233" draw:stroke="solid" svg:stroke-color="#445566" svg:stroke-width="2pt"/></style:style></office:automatic-styles><office:body><office:text><text:p text:style-name="P1">a<draw:rect draw:style-name="gr1" text:anchor-type="as-char" svg:width="1in" svg:height="1in"/></text:p></office:text></office:body></office:document-content>"##
    );
    let mut z = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let o = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    let settings = r#"<office:document-settings xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:config="urn:oasis:names:tc:opendocument:xmlns:config:1.0"><office:settings><config:config-item-set config:name="ooo:configuration-settings"><config:config-item config:name="TabsRelativeToIndent" config:type="boolean">false</config:config-item></config:config-item-set></office:settings></office:document-settings>"#;
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
    let odd = OdfProfile {
        preset: OdfPreset::Custom,
        page_margin: 99.0,
        header_min_height: 99.0,
        dynamic_spacing: true,
        tabs_from_indent: true,
        column_gap: 99.0,
        shape_fill: Rgb(1, 2, 3),
        shape_line: Rgb(4, 5, 6),
        hairline: 9.0,
        ..OdfProfile::wordcraft()
    };
    let near = |a: f32, b: f32| (a - b).abs() < 0.05;
    for profile in [OdfProfile::wordcraft(), OdfProfile::libreoffice(), odd] {
        let d = import_as(&bytes, &profile);
        let s = &d.last_section;
        assert!(near(s.header_gap, cm(0.5)), "{:?}: the stated spacing, kept: {}", profile.preset, s.header_gap);
        assert!(s.margin_top >= cm(3.0) - 0.05, "{:?}: at least the stated height under the margin: {}", profile.preset, s.margin_top);
        assert!(near(s.margin_left, cm(2.0)) && near(s.margin_bottom, cm(2.0)), "{:?}: the stated margins", profile.preset);
        assert_eq!(s.columns.count, 2);
        assert!(near(s.columns.space, cm(1.0)), "{:?}: {}", profile.preset, s.columns.space);
        let p = d.body[0].as_para().unwrap();
        assert_eq!(p.props.tabs.clone().unwrap_or_default().first().map(|t| t.pos), Some(72.0), "{:?}", profile.preset);
        let shape = p.objects.iter().find_map(|o| match o {
            InlineObject::Shape { fill, stroke, stroke_width, .. } => Some((fill.as_ref().map(|f| f.main_color()), *stroke, *stroke_width)),
            _ => None,
        });
        assert_eq!(shape, Some((Some(Rgb(0x11, 0x22, 0x33)), Some(Rgb(0x44, 0x55, 0x66)), 2.0)), "{:?}", profile.preset);
    }
}

#[test]
fn a_sections_column_balance_is_its_own_else_the_profiles_and_is_written() {
    // §20.428 `text:dont-balance-text-columns`: read as its name says; unstated, the profile's
    // (LibreOffice balances; WordCraft, as Word, only when text follows on the page).
    let doc = |props: &str| {
        let content = format!(
            r#"<office:document-content {NS_T}><office:automatic-styles><style:style style:name="S" style:family="section"><style:section-properties {props}><style:columns fo:column-count="2" fo:column-gap="0.5cm"/></style:section-properties></style:style></office:automatic-styles><office:body><office:text><text:p>a</text:p><text:section text:name="S1" text:style-name="S"><text:p>b</text:p></text:section></office:text></office:body></office:document-content>"#
        );
        crate::odt::tests::package(&content, None)
    };
    let balance = |props: &str, profile: &OdfProfile| import_as(&doc(props), profile).last_section.balance_columns;
    for p in [OdfProfile::wordcraft(), OdfProfile::libreoffice()] {
        assert_eq!(balance(r#"text:dont-balance-text-columns="true""#, &p), Some(false));
        assert_eq!(balance(r#"text:dont-balance-text-columns="false""#, &p), Some(true));
    }
    assert_eq!(balance("", &OdfProfile::libreoffice()), Some(true));
    assert_eq!(balance("", &OdfProfile::wordcraft()), None);
    // Written as WordCraft lays it out, so every reader balances it the same.
    for (props, written) in [("", "true"), (r#"text:dont-balance-text-columns="false""#, "false")] {
        let d = import_as(&doc(props), &OdfProfile::wordcraft());
        let saved = crate::odt::export(&d).unwrap();
        let content = crate::odt::tests::entry(&saved, "content.xml");
        assert!(content.contains(&format!(r#"text:dont-balance-text-columns="{written}""#)), "{props}: {content}");
    }
}
