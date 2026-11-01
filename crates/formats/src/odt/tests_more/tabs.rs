//! Tab stops (ODF 1.3 §17.7–17.8 `style:tab-stops`): position, alignment and leader, read from
//! paragraph styles and written back. LibreOffice measures them from the paragraph's indent when
//! its `TabsRelativeToIndent` setting is on (its default); WordCraft and Word from the margin.

use crate::odt::tests::{NS_T, package};
use crate::odt::{export, import};
use wordcraft_doc::props::{ParaProps, TabAlign, TabLeader, TabStop};
use wordcraft_doc::{Document, Paragraph, para_block};

const TABS: &str = r#"<style:tab-stops><style:tab-stop style:position="1in"/><style:tab-stop style:position="2in" style:type="center" style:leader-style="dotted" style:leader-text="."/><style:tab-stop style:position="3in" style:type="char" style:char="."/><style:tab-stop style:position="4in" style:type="right" style:leader-style="solid" style:leader-text="_"/><style:tab-stop style:position="5in" style:leader-text="-"/></style:tab-stops>"#;

fn with_settings(content: &str, relative: Option<bool>) -> Vec<u8> {
    let mut bytes = package(content, None);
    if let Some(r) = relative {
        // Re-pack with a settings.xml.
        let settings = format!(
            r#"<office:document-settings xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:config="urn:oasis:names:tc:opendocument:xmlns:config:1.0"><office:settings><config:config-item-set config:name="ooo:configuration-settings"><config:config-item config:name="TabsRelativeToIndent" config:type="boolean">{r}</config:config-item></config:config-item-set></office:settings></office:document-settings>"#
        );
        let mut z = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let o = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
        for (name, data) in [("mimetype", "application/vnd.oasis.opendocument.text"), ("content.xml", content), ("settings.xml", settings.as_str())] {
            z.start_file(name, o).unwrap();
            std::io::Write::write_all(&mut z, data.as_bytes()).unwrap();
        }
        bytes = z.finish().unwrap().into_inner();
    }
    bytes
}

fn content(indent: &str) -> String {
    format!(
        r#"<office:document-content {NS_T}><office:automatic-styles><style:style style:name="P1" style:family="paragraph"><style:paragraph-properties {indent}>{TABS}</style:paragraph-properties></style:style></office:automatic-styles><office:body><office:text><text:p text:style-name="P1">a	b</text:p></office:text></office:body></office:document-content>"#
    )
}

fn tabs(d: &Document) -> Vec<TabStop> {
    d.body[0].as_para().unwrap().props.tabs.clone().unwrap_or_default()
}

#[test]
fn tab_stops_are_read() {
    let d = import(&with_settings(&content(""), Some(false))).unwrap();
    let t = tabs(&d);
    assert_eq!(t.len(), 5);
    let pos: Vec<f32> = t.iter().map(|t| t.pos).collect();
    assert_eq!(pos, [72.0, 144.0, 216.0, 288.0, 360.0]);
    let kinds: Vec<(TabAlign, TabLeader)> = t.iter().map(|t| (t.align, t.leader)).collect();
    assert_eq!(
        kinds,
        [
            (TabAlign::Left, TabLeader::None),
            (TabAlign::Center, TabLeader::Dot),
            (TabAlign::Decimal, TabLeader::None),
            (TabAlign::Right, TabLeader::Underscore),
            // A leader text without a leader style draws no leader (§19.489: style none).
            (TabAlign::Left, TabLeader::None),
        ]
    );
}

#[test]
fn tabs_relative_to_the_indent_move_with_it() {
    // LibreOffice's own default: positions from the paragraph's indent.
    for relative in [Some(true), None] {
        let d = import(&with_settings(&content(r#"fo:margin-left="0.5in""#), relative)).unwrap();
        assert_eq!(tabs(&d).first().map(|t| t.pos), Some(108.0), "{relative:?}");
    }
    let d = import(&with_settings(&content(r#"fo:margin-left="0.5in""#), Some(false))).unwrap();
    assert_eq!(tabs(&d).first().map(|t| t.pos), Some(72.0));
}

#[test]
fn tab_stops_round_trip() {
    let want = vec![
        TabStop { pos: 36.0, align: TabAlign::Left, leader: TabLeader::None },
        TabStop { pos: 100.0, align: TabAlign::Center, leader: TabLeader::Dot },
        TabStop { pos: 200.0, align: TabAlign::Decimal, leader: TabLeader::MiddleDot },
        TabStop { pos: 300.0, align: TabAlign::Right, leader: TabLeader::Hyphen },
    ];
    let mut p = Paragraph::new();
    p.text = "a\tb".into();
    p.runs = vec![wordcraft_doc::para::Run { len: 3, props: Default::default() }];
    p.props = ParaProps { tabs: Some(want.clone()), indent_left: Some(18.0), ..Default::default() };
    let mut d = Document::new();
    d.body = vec![para_block(p)];
    let back = import(&export(&d).unwrap()).unwrap();
    assert_eq!(tabs(&back), want);
}

#[test]
fn hostile_tab_stops_are_bounded() {
    let many: String = (0..5000).map(|i| format!(r#"<style:tab-stop style:position="{i}pt"/>"#)).collect();
    let xml = format!(
        r#"<office:document-content {NS_T}><office:automatic-styles><style:style style:name="P1" style:family="paragraph"><style:paragraph-properties><style:tab-stops>{many}<style:tab-stop style:position="1e30in"/><style:tab-stop style:position="NaNpt"/><style:tab-stop/></style:tab-stops></style:paragraph-properties></style:style></office:automatic-styles><office:body><office:text><text:p text:style-name="P1">x</text:p></office:text></office:body></office:document-content>"#
    );
    let d = import(&package(&xml, None)).unwrap();
    let t = tabs(&d);
    assert!(t.len() <= 64, "{}", t.len());
    assert!(t.iter().all(|t| t.pos.is_finite() && t.pos.abs() <= 1584.0));
    let _ = export(&d).unwrap();
}

#[test]
fn the_default_tab_interval_round_trips() {
    let styles = format!(
        r#"<office:document-styles {NS_T}><office:styles><style:default-style style:family="paragraph"><style:paragraph-properties style:tab-stop-distance="0.5in"/></style:default-style></office:styles></office:document-styles>"#
    );
    let content = format!(
        r#"<office:document-content {NS_T}><office:body><office:text><text:p>x</text:p></office:text></office:body></office:document-content>"#
    );
    let mut d = import(&package(&content, Some(&styles))).unwrap();
    assert_eq!(d.settings.default_tab, 36.0);
    d.settings.default_tab = 28.35;
    let back = import(&export(&d).unwrap()).unwrap();
    assert!((back.settings.default_tab - 28.35).abs() < 0.01, "{}", back.settings.default_tab);
}
