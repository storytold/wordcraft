//! What WordCraft doesn't model survives a save that doesn't change it: a paragraph nothing
//! edited is written back as it was read, with the definitions it names (`wordcraft_doc::kept`,
//! `odt::kept`).
//!
//! Every sample here (and any sample file added for the no-edit loss gate) is written by us for
//! the purpose, with our own text: never a file or text taken from LibreOffice, a bug tracker,
//! another application's templates or anyone else's document, so every sample can be committed.

use crate::odt::tests::{NS_T, content_of, package, styles_of};
use crate::odt::{export, import};
use wordcraft_doc::Pos;

/// A paragraph with an element and an attribute WordCraft doesn't read, in automatic styles
/// naming a common style WordCraft doesn't model either, and a font face.
const CONTENT: &str = r#"<office:font-face-decls><style:font-face style:name="Odd Font" svg:font-family="'Odd Font'"/></office:font-face-decls><office:automatic-styles><style:style style:name="P1" style:family="paragraph" style:parent-style-name="Plaque"><style:paragraph-properties fo:margin-left="1in" loext:odd="yes"/></style:style><style:style style:name="T1" style:family="text"><style:text-properties style:font-name="Odd Font" fo:font-weight="bold"/></style:style></office:automatic-styles>"#;
const PARA: &str = r#"<text:p text:style-name="P1" x:unknown="1">Keep <text:span text:style-name="T1">this</text:span><text:hidden-text text:condition="ooow:1" text:string-value="secret"/> as it is.</text:p>"#;

fn doc() -> wordcraft_doc::Document {
    let styles = format!(
        r#"<office:document-styles {NS_T}><office:styles><style:style style:name="Plaque" style:family="paragraph" style:class="odd"><style:text-properties fo:font-size="14pt"/></style:style></office:styles></office:document-styles>"#
    );
    let content = format!(
        r#"<office:document-content {NS_T} xmlns:x="urn:example:x" xmlns:loext="urn:org:documentfoundation:names:experimental:office:xmlns:loext:1.0">{CONTENT}<office:body><office:text>{PARA}<text:p>Other.</text:p></office:text></office:body></office:document-content>"#
    );
    import(&package(&content, Some(&styles))).unwrap()
}

#[test]
fn an_unedited_paragraph_is_written_as_it_was_read() {
    let d = doc();
    let saved = export(&d).unwrap();
    let c = content_of(&saved);
    assert!(c.contains(PARA), "{c}");
    // What it names, as read: its automatic styles and font face, the common style its style is
    // based on.
    assert!(c.contains(r#"<style:style style:name="P1" style:family="paragraph" style:parent-style-name="Plaque"><style:paragraph-properties fo:margin-left="1in" loext:odd="yes"/></style:style>"#), "{c}");
    assert!(c.contains(r#"<style:style style:name="T1" style:family="text">"#), "{c}");
    assert!(c.contains(r#"style:name="Odd Font""#), "{c}");
    let s = styles_of(&saved);
    assert!(s.contains(r#"style:name="Plaque""#), "{s}");
    // WordCraft's own automatic styles can't take the same names.
    assert!(!c.contains(r#"style:name="P1" style:family="paragraph"><"#) || c.matches(r#"style:name="P1""#).count() == 1, "{c}");
    // And it reads back the same.
    let back = import(&saved).unwrap();
    assert_eq!(back.body[0].as_para().unwrap().text, d.body[0].as_para().unwrap().text);
    // Saved again, still as read.
    assert!(content_of(&export(&back).unwrap()).contains(PARA));
}

#[test]
fn an_edited_paragraph_is_written_from_the_model() {
    let mut d = doc();
    d.insert_text(&Pos::body(0, 0), "Now ", &Default::default()).unwrap();
    let c = content_of(&export(&d).unwrap());
    assert!(!c.contains("text:hidden-text") && !c.contains("x:unknown"), "{c}");
    assert!(c.contains("Now Keep"), "{c}");
    // The paragraph nobody touched still is as read.
    assert!(c.contains("<text:p>Other.</text:p>"), "{c}");
    // WordCraft's names keep clear of the file's.
    assert!(c.contains(r#"text:style-name="wcP"#) || !c.contains(r#"style:name="P1""#), "{c}");
}

#[test]
fn a_style_edited_in_wordcraft_is_written_as_edited() {
    // A custom style a kept paragraph names: WordCraft's version, not the one read.
    let styles = format!(
        r#"<office:document-styles {NS_T}><office:styles><style:style style:name="Note" style:family="paragraph"><style:text-properties fo:font-size="9pt"/></style:style></office:styles></office:document-styles>"#
    );
    let content = format!(
        r#"<office:document-content {NS_T}><office:body><office:text><text:p text:style-name="Note">Small.</text:p></office:text></office:body></office:document-content>"#
    );
    let mut d = import(&package(&content, Some(&styles))).unwrap();
    let id = d.body[0].as_para().unwrap().props.style.clone().unwrap();
    if let Some(st) = std::sync::Arc::make_mut(&mut d.styles).styles.iter_mut().find(|s| s.id == id) {
        st.chr.size = Some(20.0);
    }
    let saved = export(&d).unwrap();
    assert!(content_of(&saved).contains(r#"<text:p text:style-name="Note">Small.</text:p>"#));
    let s = styles_of(&saved);
    assert!(s.contains(r#"fo:font-size="20pt""#) && !s.contains(r#"fo:font-size="9pt""#), "{s}");
}

#[test]
fn a_kept_sequence_is_declared() {
    let content = format!(
        r#"<office:document-content {NS_T}><office:body><office:text><text:p>Figure <text:sequence text:name="Figure" text:formula="ooow:Figure+1" style:num-format="1">1</text:sequence>: a cat</text:p></office:text></office:body></office:document-content>"#
    );
    let d = import(&package(&content, None)).unwrap();
    let c = content_of(&export(&d).unwrap());
    assert!(c.contains(r#"<text:sequence-decl text:display-outline-level="0" text:name="Figure"/>"#), "{c}");
}

#[test]
fn hostile_kept_markup_does_not_panic() {
    for body in [
        r#"<text:p text:style-name="P1">a<text:span text:style-name="P1">b</text:span></text:p>"#,
        r#"<text:p text:style-name="">x</text:p><text:p text:style-name="&amp;&lt;">y</text:p>"#,
        r#"<text:p><text:sequence text:name="">1</text:sequence><text:sequence>2</text:sequence></text:p>"#,
        // Sections left open, closed twice, inside a list, empty.
        r#"<text:section text:name="a"><text:p>x</text:p>"#,
        r#"<text:p>y</text:p></text:section></text:section><text:p>z</text:p>"#,
        r#"<text:list><text:list-item><text:section text:name="b"><text:p>w</text:p></text:section></text:list-item></text:list>"#,
        r#"<text:section text:name="c"/><text:section text:name="d"></text:section><text:p>v</text:p>"#,
    ] {
        let content = format!(
            r#"<office:document-content {NS_T}><office:automatic-styles><style:style style:name="P1" style:family="paragraph" style:parent-style-name="P1" style:list-style-name="P1"/></office:automatic-styles><office:body><office:text>{body}</office:text></office:body></office:document-content>"#
        );
        let d = import(&package(&content, None)).unwrap();
        let saved = export(&d).unwrap();
        let _ = import(&saved).unwrap();
    }
}

#[test]
fn unedited_style_definitions_are_written_as_read() {
    // What the model has no place for (an unused list style, the footnote configuration, a
    // style attribute nobody reads) survives while nobody changes a style.
    let styles = format!(
        r#"<office:document-styles {NS_T}><office:styles><style:default-style style:family="paragraph"><style:paragraph-properties style:tab-stop-distance="0.5in"/></style:default-style><style:style style:name="Standard" style:family="paragraph" style:class="text" x:odd="1"><style:paragraph-properties fo:text-align="justify"/></style:style><text:list-style style:name="Unused"><text:list-level-style-bullet text:level="1" text:bullet-char="✓"/></text:list-style><text:notes-configuration text:note-class="footnote" style:num-format="a" text:start-value="0" text:footnotes-position="page" text:start-numbering-at="document"/></office:styles></office:document-styles>"#
    );
    let content = format!(
        r#"<office:document-content {NS_T}><office:body><office:text><text:p text:style-name="Standard">Text.</text:p></office:text></office:body></office:document-content>"#
    );
    let pkg = package(&content, Some(&styles.replace("<office:document-styles ", r#"<office:document-styles xmlns:x="urn:example:x" "#)));
    let mut d = import(&pkg).unwrap();
    let s = styles_of(&export(&d).unwrap());
    assert!(s.contains(r#"<text:list-style style:name="Unused">"#), "{s}");
    assert!(s.contains(r#"x:odd="1""#) && s.contains(r#"xmlns:x="urn:example:x""#), "{s}");
    assert_eq!(s.matches("<text:notes-configuration text:note-class=\"footnote\"").count(), 1, "{s}");
    assert_eq!(s.matches(r#"<style:style style:name="Standard""#).count(), 1, "{s}");
    // A style changed in WordCraft: the definitions are WordCraft's.
    if let Some(st) = std::sync::Arc::make_mut(&mut d.styles).styles.iter_mut().find(|s| s.id == "Normal") {
        st.chr.bold = Some(true);
    }
    let s = styles_of(&export(&d).unwrap());
    assert!(!s.contains(r#"x:odd="1""#), "{s}");
    assert_eq!(s.matches(r#"<style:style style:name="Standard""#).count(), 1, "{s}");
}

#[test]
fn a_file_a_kept_definition_names_is_written_with_it() {
    // LibreOffice's templates fill pages with a bitmap their styles name (`draw:fill-image`);
    // written as read without the picture, LibreOffice refused the whole file.
    let png = [b"\x89PNG\r\n\x1a\n".as_slice(), &[3u8; 64]].concat();
    let styles = format!(
        r#"<office:document-styles {NS_T} xmlns:draw="urn:oasis:names:tc:opendocument:xmlns:drawing:1.0" xmlns:xlink="http://www.w3.org/1999/xlink"><office:styles><draw:fill-image draw:name="Paper" xlink:href="Pictures/paper.png" xlink:type="simple"/><style:style style:name="Standard" style:family="paragraph"/></office:styles></office:document-styles>"#
    );
    let content = format!(
        r#"<office:document-content {NS_T}><office:body><office:text><text:p>Text.</text:p></office:text></office:body></office:document-content>"#
    );
    let pkg = crate::odt::tests::zip_of(&[("content.xml", content.as_bytes()), ("styles.xml", styles.as_bytes()), ("Pictures/paper.png", &png)]);
    let d = import(&pkg).unwrap();
    let saved = export(&d).unwrap();
    assert!(styles_of(&saved).contains(r#"xlink:href="Pictures/paper.png""#));
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(saved.clone())).unwrap();
    let mut got = Vec::new();
    std::io::Read::read_to_end(&mut z.by_name("Pictures/paper.png").unwrap(), &mut got).unwrap();
    assert_eq!(got, png);
    let m = crate::odt::tests::entry(&saved, "META-INF/manifest.xml");
    assert!(m.contains(r#"manifest:full-path="Pictures/paper.png""#), "{m}");
}
