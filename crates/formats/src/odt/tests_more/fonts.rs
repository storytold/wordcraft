//! Font names (ODF 1.3 §16.23 `style:font-face`): `style:font-name` names a font face declared in
//! `office:font-face-decls`, whose `svg:font-family` is the font.

use crate::odt::import;
use crate::odt::tests::{NS_T, package};

const NS_SVG: &str = r#"xmlns:svg="urn:oasis:names:tc:opendocument:xmlns:svg-compatible:1.0""#;

fn fonts(decls: &str, props: &[&str], styles: Option<&str>) -> Vec<Option<String>> {
    let auto: String = props
        .iter()
        .enumerate()
        .map(|(i, p)| format!(r#"<style:style style:name="T{i}" style:family="text"><style:text-properties {p}/></style:style>"#))
        .collect();
    let body: String = (0..props.len()).map(|i| format!(r#"<text:span text:style-name="T{i}">s{i}</text:span>|"#)).collect();
    let content = format!(
        r#"<office:document-content {NS_T} {NS_SVG}><office:font-face-decls>{decls}</office:font-face-decls><office:automatic-styles>{auto}</office:automatic-styles><office:body><office:text><text:p>{body}</text:p></office:text></office:body></office:document-content>"#
    );
    let d = import(&package(&content, styles)).unwrap();
    let p = d.body[0].as_para().unwrap();
    p.run_ranges()
        .filter(|(r, _)| p.text.get(r.clone()).is_some_and(|t| t.trim_start_matches('|').starts_with('s')))
        .map(|(_, c)| c.font.clone())
        .collect()
}

#[test]
fn a_font_name_is_the_family_its_face_declares() {
    let decls = r#"<style:font-face style:name="FreeSans1" svg:font-family="FreeSans"/><style:font-face style:name="Serif" svg:font-family="&apos;Liberation Serif&apos;"/>"#;
    let got = fonts(decls, &[r#"style:font-name="FreeSans1""#, r#"style:font-name="Serif""#, r#"style:font-name="Undeclared Font""#], None);
    assert_eq!(got, [Some("FreeSans".into()), Some("Liberation Serif".into()), Some("Undeclared Font".into())]);
}

#[test]
fn faces_declared_in_styles_xml_count_too() {
    let styles = format!(
        r#"<office:document-styles {NS_T} {NS_SVG}><office:font-face-decls><style:font-face style:name="F2" svg:font-family="DejaVu Sans"/></office:font-face-decls></office:document-styles>"#
    );
    assert_eq!(fonts("", &[r#"style:font-name="F2""#], Some(&styles)), [Some("DejaVu Sans".into())]);
}

#[test]
fn hostile_font_faces_do_not_panic() {
    let many: String = (0..20_000).map(|i| format!(r#"<style:font-face style:name="F{i}" svg:font-family="X{i}"/>"#)).collect();
    let long = "n".repeat(10_000);
    let decls = format!(r#"{many}<style:font-face style:name="L" svg:font-family="{long}"/><style:font-face svg:font-family="nameless"/>"#);
    let got = fonts(&decls, &[r#"style:font-name="L""#, r#"style:font-name="F5""#], None);
    assert!(got[0].as_ref().is_none_or(|f| f.len() <= 256), "{:?}", got[0].as_ref().map(String::len));
}
