//! Document-wide options ODT keeps in its styles: mirror margins (ODF 1.3 §19.505
//! `style:page-usage="mirrored"`), the most consecutive hyphenated lines (§20.200
//! `fo:hyphenation-ladder-count`), and LibreOffice's hyphenation zone and capitals switch
//! (`loext:hyphenation-zone`, `loext:hyphenation-no-caps`), read into the document's settings and
//! written back.

use crate::odt::tests::{NS_T, package, styles_of};
use crate::odt::{export, import};
use wordcraft_doc::DEFAULT_HYPHENATION_ZONE;
use wordcraft_doc::{Document, Paragraph, para_block};

const LOEXT: &str = r#"xmlns:loext="urn:org:documentfoundation:names:experimental:office:xmlns:loext:1.0""#;

/// A file whose styles.xml has `default` in the default paragraph style (paragraph properties
/// `para`, text properties `text`) and page layout `pm1` with `usage` on it.
fn open(para: &str, text: &str, usage: &str) -> Document {
    let styles = format!(
        r#"<office:document-styles {NS_T} {LOEXT}><office:styles><style:default-style style:family="paragraph"><style:paragraph-properties {para}/><style:text-properties {text}/></style:default-style></office:styles><office:automatic-styles><style:page-layout style:name="pm1" {usage}><style:page-layout-properties fo:page-width="8.5in" fo:page-height="11in" fo:margin-top="1in" fo:margin-bottom="1in" fo:margin-left="1.25in" fo:margin-right="0.75in"/></style:page-layout></office:automatic-styles><office:master-styles><style:master-page style:name="Standard" style:page-layout-name="pm1"/></office:master-styles></office:document-styles>"#
    );
    let content = format!(
        r#"<office:document-content {NS_T}><office:body><office:text><text:p>Body.</text:p></office:text></office:body></office:document-content>"#
    );
    import(&package(&content, Some(&styles))).unwrap()
}

/// (mirror_margins, consecutive_hyphen_limit, hyphenation_zone, hyphenate_caps)
fn opts(d: &Document) -> (bool, u32, f32, bool) {
    let s = &d.settings;
    (s.mirror_margins, s.consecutive_hyphen_limit, s.hyphenation_zone, s.hyphenate_caps)
}

#[test]
fn mirrored_pages_are_mirror_margins() {
    let z = DEFAULT_HYPHENATION_ZONE;
    for (usage, mirrored) in
        [(r#"style:page-usage="mirrored""#, true), (r#"style:page-usage="all""#, false), (r#"style:page-usage="right""#, false), ("", false)]
    {
        let d = open("", "", usage);
        assert_eq!(opts(&d), (mirrored, 0, z, true), "{usage}");
        // Inside and outside margins stay where the file has them.
        let sec = d.sections()[0].1;
        assert_eq!((sec.margin_left, sec.margin_right), (90.0, 54.0), "{usage}");
    }
}

#[test]
fn hyphenation_options_are_read() {
    let z = DEFAULT_HYPHENATION_ZONE;
    let cases: [(&str, &str, (bool, u32, f32, bool)); 7] = [
        (r#"fo:hyphenation-ladder-count="2""#, "", (false, 2, z, true)),
        (r#"fo:hyphenation-ladder-count="no-limit""#, "", (false, 0, z, true)),
        (r#"fo:hyphenation-ladder-count="banana""#, "", (false, 0, z, true)),
        (r#"loext:hyphenation-zone="0.5in""#, "", (false, 0, 36.0, true)),
        (r#"loext:hyphenation-zone="9pt""#, "", (false, 0, 9.0, true)),
        ("", r#"loext:hyphenation-no-caps="true""#, (false, 0, z, false)),
        ("", r#"loext:hyphenation-no-caps="false""#, (false, 0, z, true)),
    ];
    for (para, text, want) in cases {
        assert_eq!(opts(&open(para, text, "")), want, "{para} {text}");
    }
}

#[test]
fn an_absurd_ladder_count_is_capped() {
    let d = open(r#"fo:hyphenation-ladder-count="99999999""#, "", "");
    assert_eq!(d.settings.consecutive_hyphen_limit, 32767);
}

fn model(f: impl FnOnce(&mut wordcraft_doc::Settings)) -> Document {
    let mut d = Document::new();
    let mut p = Paragraph::new();
    p.text = "Body.".into();
    d.body = vec![para_block(p)];
    f(&mut d.settings);
    d
}

#[test]
fn the_options_round_trip_from_the_model() {
    let d = model(|s| {
        s.mirror_margins = true;
        s.auto_hyphenation = true;
        s.consecutive_hyphen_limit = 3;
        s.hyphenation_zone = 27.0;
        s.hyphenate_caps = false;
    });
    let saved = export(&d).unwrap();
    let st = styles_of(&saved);
    for attr in [
        r#"style:page-usage="mirrored""#,
        r#"fo:hyphenation-ladder-count="3""#,
        r#"loext:hyphenation-zone="27pt""#,
        r#"loext:hyphenation-no-caps="true""#,
    ] {
        assert!(st.contains(attr), "{attr} in {st}");
    }
    assert_eq!(opts(&import(&saved).unwrap()), (true, 3, 27.0, false));
}

#[test]
fn default_options_write_nothing() {
    let d = model(|_| {});
    let st = styles_of(&export(&d).unwrap());
    for attr in ["style:page-usage", "fo:hyphenation-ladder-count", "loext:hyphenation-zone", "loext:hyphenation-no-caps"] {
        assert!(!st.contains(attr), "{attr} in {st}");
    }
    assert_eq!(opts(&import(&export(&d).unwrap()).unwrap()), (false, 0, DEFAULT_HYPHENATION_ZONE, true));
}

/// Changes made after opening a file are saved, not the file's own page layout or default
/// style (review of #6218): Mirror margins on and off, consecutive hyphen limit, capitals.
#[test]
fn changes_after_open_are_saved() {
    type Change = fn(&mut wordcraft_doc::Settings);
    let mirrored = r#"style:page-usage="mirrored""#;
    let ladder = r#"fo:hyphenation-ladder-count="2""#;
    let no_caps = r#"loext:hyphenation-no-caps="true""#;
    let cases: [(&str, &str, &str, Change, (bool, u32, bool)); 4] = [
        ("", "", "", |s| s.mirror_margins = true, (true, 0, true)),
        ("", "", mirrored, |s| s.mirror_margins = false, (false, 0, true)),
        (ladder, "", "", |s| s.consecutive_hyphen_limit = 4, (false, 4, true)),
        ("", no_caps, "", |s| s.hyphenate_caps = true, (false, 0, true)),
    ];
    for (i, (para, text, usage, change, (mirror, ladder_count, caps))) in cases.into_iter().enumerate() {
        let mut d = open(para, text, usage);
        change(&mut d.settings);
        let saved = export(&d).unwrap();
        let st = styles_of(&saved);
        assert_eq!(st.contains(mirrored), mirror, "case {i}: page-usage in {st}");
        let back = import(&saved).unwrap();
        let o = opts(&back);
        assert_eq!((o.0, o.1, o.3), (mirror, ladder_count, caps), "case {i}");
        let sec = back.sections()[0].1;
        assert_eq!((sec.margin_left, sec.margin_right), (90.0, 54.0), "case {i}: margins");
    }
}
