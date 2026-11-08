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
    // A style changed in WordCraft: that one is WordCraft's, the others still the file's.
    if let Some(st) = std::sync::Arc::make_mut(&mut d.styles).styles.iter_mut().find(|s| s.id == "Quote") {
        st.chr.bold = Some(true);
    }
    let s = styles_of(&export(&d).unwrap());
    assert!(s.contains(r#"x:odd="1""#), "{s}");
    assert!(
        s.split(r#"<style:style style:name="Quotations""#)
            .nth(1)
            .is_some_and(|q| q.split("</style:style>").next().is_some_and(|q| q.contains(r#"fo:font-weight="bold""#))),
        "{s}"
    );
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

/// A file defining what WordCraft doesn't model: a gradient, a paragraph style no text of its
/// own uses, a default style's writing mode, and a frame style naming them.
fn file_with_definitions() -> Vec<u8> {
    let ns = crate::odt::tests::NS_T;
    let draw =
        r#"xmlns:draw="urn:oasis:names:tc:opendocument:xmlns:drawing:1.0" xmlns:svg="urn:oasis:names:tc:opendocument:xmlns:svg-compatible:1.0""#;
    let styles = format!(
        r##"<office:document-styles {ns} {draw}><office:styles><draw:gradient draw:name="Sunset" draw:style="linear" draw:start-color="#ff8800" draw:end-color="#ffff00"/><style:default-style style:family="paragraph"><style:paragraph-properties style:writing-mode="page" style:line-break="strict"/><style:text-properties fo:language="de" fo:country="DE"/></style:default-style><style:style style:name="Standard" style:family="paragraph" style:class="text"/><style:style style:name="Text_20_body" style:display-name="Text body" style:family="paragraph" style:parent-style-name="Standard"><style:paragraph-properties fo:margin-bottom="7pt"/></style:style><style:style style:name="Eigen" style:display-name="Eigen" style:family="paragraph" style:parent-style-name="Standard"><style:text-properties fo:font-size="9pt"/></style:style><style:style style:name="Frame" style:family="graphic"><style:graphic-properties style:wrap="dynamic" style:vertical-pos="top"/></style:style></office:styles></office:document-styles>"##
    );
    let content = format!(
        r#"<office:document-content {ns} {draw}><office:automatic-styles><style:style style:name="fr1" style:family="graphic" style:parent-style-name="Frame"><style:graphic-properties draw:fill="gradient" draw:fill-gradient-name="Sunset"/></style:style></office:automatic-styles><office:body><office:text><text:p text:style-name="Text_20_body">Text<draw:frame draw:style-name="fr1" text:anchor-type="paragraph" svg:width="2in" svg:height="1in"><draw:text-box><text:p>In</text:p></draw:text-box></draw:frame></text:p><text:p text:style-name="Eigen">Own</text:p></office:text></office:body></office:document-content>"#
    );
    crate::odt::tests::package(&content, Some(&styles))
}

#[test]
fn changing_one_style_leaves_the_files_other_definitions_alone() {
    use std::sync::Arc;
    let bytes = file_with_definitions();
    let as_read = |saved: &[u8]| {
        let s = crate::odt::tests::entry(saved, "styles.xml");
        assert!(s.contains(r#"draw:gradient draw:name="Sunset""#), "its gradient: {s}");
        assert!(s.contains(r#"fo:margin-bottom="7pt""#), "a style WordCraft doesn't model: {s}");
        assert!(s.contains(r#"style:wrap="dynamic""#), "what its frames derive from: {s}");
        assert!(s.contains(r#"style:writing-mode="page""#) && s.contains(r#"fo:language="de""#), "its default style: {s}");
        s
    };
    // A style no text uses.
    let mut d = crate::odt::import(&bytes).unwrap();
    Arc::make_mut(&mut d.styles).get_mut("Quote").unwrap().chr.size = Some(13.0);
    let saved = crate::odt::export(&d).unwrap();
    let s = as_read(&saved);
    assert!(s.contains(r#"fo:font-size="13pt""#), "the change: {s}");
    assert_eq!(crate::odt::import(&saved).unwrap().styles.get("Quote").unwrap().chr.size, Some(13.0));
    // A style the file defined: that one is WordCraft's now, the others still the file's.
    let mut d = crate::odt::import(&bytes).unwrap();
    Arc::make_mut(&mut d.styles).get_mut("Eigen").unwrap().chr.size = Some(21.0);
    let saved = crate::odt::export(&d).unwrap();
    let s = as_read(&saved);
    assert!(s.contains(r#"fo:font-size="21pt""#) && !s.contains(r#"fo:font-size="9pt""#), "{s}");
    // A new list (`para.numbering`): the styles as read, and the list there.
    let mut d = crate::odt::import(&bytes).unwrap();
    Arc::make_mut(&mut d.numbering).add_list(wordcraft_doc::numbering::ListKind::Bullet);
    as_read(&crate::odt::export(&d).unwrap());
    // Normal changed: the default style and Standard are WordCraft's.
    let mut d = crate::odt::import(&bytes).unwrap();
    Arc::make_mut(&mut d.styles).default_chr.size = Some(15.0);
    let s = crate::odt::tests::entry(&crate::odt::export(&d).unwrap(), "styles.xml");
    assert!(s.contains(r#"fo:font-size="15pt""#), "{s}");
    assert!(s.contains(r#"draw:gradient draw:name="Sunset""#) && s.contains(r#"fo:margin-bottom="7pt""#), "the rest stays: {s}");
    // Nothing changed: the file's own, and none of WordCraft's over them.
    let d = crate::odt::import(&bytes).unwrap();
    as_read(&crate::odt::export(&d).unwrap());
}

/// A style deleted in WordCraft stays deleted: the file's own definition doesn't come back with
/// the definitions that stay as read.
#[test]
fn a_deleted_style_stays_deleted() {
    use std::sync::Arc;
    let mut d = crate::odt::import(&file_with_definitions()).unwrap();
    assert!(d.styles.get("Eigen").is_some());
    Arc::make_mut(&mut d.styles).remove("Eigen");
    d.restyle("Eigen", None);
    let saved = crate::odt::export(&d).unwrap();
    let s = crate::odt::tests::entry(&saved, "styles.xml");
    assert!(!s.contains(r#"style:name="Eigen""#), "{s}");
    assert!(s.contains(r#"draw:gradient draw:name="Sunset""#) && s.contains(r#"fo:margin-bottom="7pt""#), "the rest stays: {s}");
    assert!(!crate::odt::tests::entry(&saved, "content.xml").contains("Eigen"));
    assert!(crate::odt::import(&saved).unwrap().styles.get("Eigen").is_none());
}

/// A file's style whose name has a space (`My_20_Custom`, WordCraft's `MyCustom`) is one style. An
/// edit reaches its definition under the file's name, a deletion removes it, and left alone it
/// stays as read with no second definition. A style nothing uses any more keeps its edit.
#[test]
fn a_style_named_with_a_space_is_the_files_style() {
    use std::sync::Arc;
    let s = r#"<style:style style:name="Standard" style:family="paragraph" style:class="text"/><style:style style:name="Base_20_Style" style:display-name="Base Style" style:family="paragraph" style:parent-style-name="Standard"/><style:style style:name="My_20_Custom" style:display-name="My Custom" style:family="paragraph" style:parent-style-name="Base_20_Style"><style:text-properties fo:font-size="9pt"/></style:style>"#;
    let styles = format!(r#"<office:document-styles {NS_T}><office:styles>{s}</office:styles></office:document-styles>"#);
    let content = format!(
        r#"<office:document-content {NS_T}><office:body><office:text><text:p text:style-name="My_20_Custom">One</text:p><text:p text:style-name="My_20_Custom">Two</text:p><text:p text:style-name="Base_20_Style">Three</text:p></office:text></office:body></office:document-content>"#
    );
    let bytes = package(&content, Some(&styles));
    let open = || import(&bytes).unwrap();
    let def = |xml: &str, name: &str| xml.split("<style:style ").find(|d| d.starts_with(&format!(r#"style:name="{name}""#))).map(String::from);
    let id = open().body[0].as_para().unwrap().props.style.clone().unwrap();

    let saved = styles_of(&export(&open()).unwrap());
    assert!(def(&saved, "My_20_Custom").is_some_and(|d| d.contains(r#"fo:font-size="9pt""#)), "as read: {saved}");
    assert!(def(&saved, "MyCustom").is_none(), "no second one: {saved}");

    // Edited: the paragraphs written as read name the file's definition, so that takes the edit.
    let mut d = open();
    Arc::make_mut(&mut d.styles).get_mut(&id).unwrap().chr.size = Some(21.0);
    let bytes_saved = export(&d).unwrap();
    let saved = styles_of(&bytes_saved);
    let mine = def(&saved, "My_20_Custom").unwrap_or_else(|| panic!("the file's name: {saved}"));
    assert!(mine.contains(r#"fo:font-size="21pt""#) && !saved.contains(r#"fo:font-size="9pt""#), "{saved}");
    assert!(mine.contains(r#"style:parent-style-name="Base_20_Style""#), "its parent, by the file's name: {mine}");
    assert!(content_of(&bytes_saved).contains(r#"text:style-name="My_20_Custom""#));
    let back = import(&bytes_saved).unwrap();
    assert_eq!(back.styles.resolve_char(back.body[0].as_para().unwrap().props.style.as_deref(), &Default::default()).size, 21.0);

    // Deleted: it stays deleted.
    let mut d = open();
    Arc::make_mut(&mut d.styles).remove(&id);
    d.restyle(&id, None);
    let bytes_saved = export(&d).unwrap();
    let saved = styles_of(&bytes_saved);
    assert!(def(&saved, "My_20_Custom").is_none() && def(&saved, "MyCustom").is_none(), "{saved}");
    assert!(def(&saved, "Base_20_Style").is_some(), "the others stay: {saved}");
    assert!(import(&bytes_saved).unwrap().styles.get(&id).is_none());

    // Nothing uses it any more, and it is edited: the edit is saved, not the style lost.
    let mut d = open();
    d.restyle(&id, None);
    Arc::make_mut(&mut d.styles).get_mut(&id).unwrap().chr.size = Some(21.0);
    let saved = styles_of(&export(&d).unwrap());
    assert!(def(&saved, "My_20_Custom").is_some_and(|m| m.contains(r#"fo:font-size="21pt""#)), "{saved}");
}

/// Two of the file's names WordCraft has as one style (`My_20_Custom` and `My_Custom` are both
/// `MyCustom`): edited, each is written under its own name, neither lost nor doubled.
#[test]
fn two_names_for_one_style_both_stay_when_it_is_edited() {
    use std::sync::Arc;
    let s = r#"<style:style style:name="Standard" style:family="paragraph" style:class="text"/><style:style style:name="My_20_Custom" style:family="paragraph" style:parent-style-name="Standard"><style:text-properties fo:font-size="9pt"/></style:style><style:style style:name="My_Custom" style:family="paragraph" style:parent-style-name="Standard"><style:text-properties fo:font-size="9pt"/></style:style>"#;
    let styles = format!(r#"<office:document-styles {NS_T}><office:styles>{s}</office:styles></office:document-styles>"#);
    let content = format!(
        r#"<office:document-content {NS_T}><office:body><office:text><text:p text:style-name="My_20_Custom">One</text:p><text:p text:style-name="My_Custom">Two</text:p></office:text></office:body></office:document-content>"#
    );
    let mut d = import(&package(&content, Some(&styles))).unwrap();
    let id = d.body[0].as_para().unwrap().props.style.clone().unwrap();
    Arc::make_mut(&mut d.styles).get_mut(&id).unwrap().chr.size = Some(21.0);
    let saved = styles_of(&export(&d).unwrap());
    for name in ["My_20_Custom", "My_Custom"] {
        assert_eq!(saved.matches(&format!(r#"style:name="{name}""#)).count(), 1, "{name}: {saved}");
        let def = saved.split(&format!(r#"style:name="{name}""#)).nth(1).and_then(|d| d.split("</style:style>").next()).unwrap();
        assert!(def.contains(r#"fo:font-size="21pt""#), "{name}: {def}");
    }
}

/// A parent's name goes by its family: a text style's other name isn't a paragraph style's.
#[test]
fn a_rewritten_styles_parent_keeps_its_own_familys_name() {
    use std::sync::Arc;
    let s = r#"<style:style style:name="Standard" style:family="paragraph" style:class="text"/><style:style style:name="BaseStyle" style:family="paragraph" style:parent-style-name="Standard"><style:text-properties fo:font-size="30pt"/></style:style><style:style style:name="Base_20_Style" style:family="text"><style:text-properties fo:font-weight="bold"/></style:style><style:style style:name="My_20_Custom" style:family="paragraph" style:parent-style-name="BaseStyle"><style:text-properties fo:font-weight="bold"/></style:style>"#;
    let styles = format!(r#"<office:document-styles {NS_T}><office:styles>{s}</office:styles></office:document-styles>"#);
    let content = format!(
        r#"<office:document-content {NS_T}><office:body><office:text><text:p text:style-name="My_20_Custom">One</text:p><text:p>Two <text:span text:style-name="Base_20_Style">three</text:span></text:p></office:text></office:body></office:document-content>"#
    );
    let mut d = import(&package(&content, Some(&styles))).unwrap();
    let id = d.body[0].as_para().unwrap().props.style.clone().unwrap();
    Arc::make_mut(&mut d.styles).get_mut(&id).unwrap().chr.italic = Some(true);
    let bytes = export(&d).unwrap();
    let saved = styles_of(&bytes);
    let mine = saved.split("<style:style ").find(|t| t.starts_with(r#"style:name="My_20_Custom""#)).and_then(|t| t.split('>').next()).unwrap();
    assert!(mine.contains(r#"style:parent-style-name="BaseStyle""#), "{mine}");
    let back = import(&bytes).unwrap();
    let p = back.body[0].as_para().unwrap();
    assert_eq!(back.styles.resolve_char(p.props.style.as_deref(), &Default::default()).size, 30.0, "inherited");
}

#[test]
fn the_files_font_declarations_are_kept() {
    // A reader without a font picks its substitute by how the file declares it. WordCraft's own
    // guess at a font's kind replaced the file's, and a document came back from a save with no
    // edits 13 pages shorter in LibreOffice on a computer without its font.
    let ns = NS_T;
    let face = r#"<style:font-face style:name="Odd Roman" svg:font-family="&apos;Odd Roman&apos;" style:font-family-generic="roman"/>"#;
    let styles = format!(
        r#"<office:document-styles {ns}><office:font-face-decls>{face}</office:font-face-decls><office:styles><style:default-style style:family="paragraph"><style:text-properties style:font-name="Odd Roman" fo:font-size="12pt"/></style:default-style></office:styles></office:document-styles>"#
    );
    let content = format!(
        r#"<office:document-content {ns}><office:font-face-decls>{face}</office:font-face-decls><office:automatic-styles><style:style style:name="P1" style:family="paragraph"><style:text-properties style:font-name="Odd Roman"/></style:style></office:automatic-styles><office:body><office:text><text:p text:style-name="P1">Text</text:p></office:text></office:body></office:document-content>"#
    );
    let check = |saved: &[u8], what: &str| {
        for part in [styles_of(saved), content_of(saved)] {
            assert!(part.contains(face), "{what}: {part}");
            assert_eq!(part.matches(r#"<style:font-face style:name="Odd Roman""#).count(), 1, "{what}: declared once: {part}");
        }
    };
    let mut d = import(&package(&content, Some(&styles))).unwrap();
    check(&export(&d).unwrap(), "no edits");
    // Text edited, written from the model: still the file's declaration.
    if let Some(wordcraft_doc::Block::Para(p)) = d.body.get_mut(0).map(std::sync::Arc::make_mut) {
        p.text.push('!');
        if let Some(r) = p.runs.last_mut() {
            r.len += 1;
        }
    }
    check(&export(&d).unwrap(), "an edit");
}

/// A font's name spelled another way in XML (`O'Brien`, `O&#39;Brien`) is the same font: its
/// declaration is the file's, once.
#[test]
fn a_fonts_name_spelled_another_way_is_declared_once_as_the_file_has_it() {
    for spelled in ["O'Brien", "O&#39;Brien"] {
        let face = format!(r#"<style:font-face style:name="{spelled}" svg:font-family="O&apos;Brien" style:font-family-generic="roman"/>"#);
        let styles = format!(
            r#"<office:document-styles {NS_T}><office:font-face-decls>{face}</office:font-face-decls><office:styles><style:default-style style:family="paragraph"><style:text-properties style:font-name="{spelled}" fo:font-size="12pt"/></style:default-style></office:styles></office:document-styles>"#
        );
        let content = format!(
            r#"<office:document-content {NS_T}><office:font-face-decls>{face}</office:font-face-decls><office:automatic-styles><style:style style:name="T1" style:family="text"><style:text-properties style:font-name="{spelled}"/></style:style></office:automatic-styles><office:body><office:text><text:p><text:span text:style-name="T1">One</text:span></text:p></office:text></office:body></office:document-content>"#
        );
        let mut d = import(&package(&content, Some(&styles))).unwrap();
        // Text edited, written from the model.
        if let Some(wordcraft_doc::Block::Para(p)) = d.body.get_mut(0).map(std::sync::Arc::make_mut) {
            p.text.push('!');
            if let Some(r) = p.runs.last_mut() {
                r.len += 1;
            }
        }
        let saved = export(&d).unwrap();
        for part in [styles_of(&saved), content_of(&saved)] {
            assert!(part.contains(&face), "{spelled}: the file's: {part}");
            let is_it = |t: &&str| {
                let name = t.split('>').next().and_then(|tag| tag.split("style:name=\"").nth(1)).and_then(|v| v.split('"').next());
                name.is_some_and(|n| n == "O'Brien" || n == "O&#39;Brien" || n == "O&apos;Brien")
            };
            assert_eq!(part.split("<style:font-face ").skip(1).filter(is_it).count(), 1, "{spelled}: declared once: {part}");
        }
    }
}

/// A face named other than its family (`F1`, `Mystery`): text written from the model names the
/// family, and that declaration says what the file's does, not WordCraft's guess at the font's kind.
#[test]
fn a_face_named_other_than_its_family_gives_the_family_the_files_declaration() {
    let face =
        r#"<style:font-face style:name="F1" svg:font-family="&apos;Mystery&apos;" style:font-family-generic="roman" style:font-pitch="variable"/>"#;
    let styles = format!(
        r#"<office:document-styles {NS_T}><office:font-face-decls>{face}</office:font-face-decls><office:styles><style:default-style style:family="paragraph"><style:text-properties fo:font-size="12pt"/></style:default-style></office:styles></office:document-styles>"#
    );
    let content = format!(
        r#"<office:document-content {NS_T}><office:font-face-decls>{face}</office:font-face-decls><office:automatic-styles><style:style style:name="T1" style:family="text"><style:text-properties style:font-name="F1"/></style:style></office:automatic-styles><office:body><office:text><text:p><text:span text:style-name="T1">One</text:span></text:p></office:text></office:body></office:document-content>"#
    );
    let mut d = import(&package(&content, Some(&styles))).unwrap();
    if let Some(wordcraft_doc::Block::Para(p)) = d.body.get_mut(0).map(std::sync::Arc::make_mut) {
        p.text.push('!');
        if let Some(r) = p.runs.last_mut() {
            r.len += 1;
        }
    }
    let saved = export(&d).unwrap();
    let part = content_of(&saved);
    assert!(part.contains(r#"style:font-name="Mystery""#), "{part}");
    let mystery = part.split("<style:font-face ").find(|t| t.starts_with(r#"style:name="Mystery""#)).and_then(|t| t.split("/>").next());
    let mystery = mystery.unwrap_or_else(|| panic!("declared: {part}"));
    assert!(mystery.contains(r#"style:font-family-generic="roman""#), "{mystery}");
}

/// Only declarations are declarations: not a script's text that reads like one. And a face's name
/// is its own attribute, not another with `style:name` in its name.
#[test]
fn a_face_is_found_and_named_by_its_own_markup() {
    let script =
        r#"<office:scripts><office:script script:language="x"><![CDATA[<style:font-face style:name="Mystery" />]]></office:script></office:scripts>"#;
    let face = r#"<style:font-face xstyle:name="F1" style:name="F1" svg:font-family="&apos;Mystery&apos;" style:font-family-generic="roman"/>"#;
    let styles = format!(
        r#"<office:document-styles {NS_T}><office:font-face-decls>{face}</office:font-face-decls><office:styles><style:default-style style:family="paragraph"><style:text-properties fo:font-size="12pt"/></style:default-style></office:styles></office:document-styles>"#
    );
    let content = format!(
        r#"<office:document-content {NS_T}>{script}<office:font-face-decls>{face}</office:font-face-decls><office:automatic-styles><style:style style:name="T1" style:family="text"><style:text-properties style:font-name="F1"/></style:style></office:automatic-styles><office:body><office:text><text:p><text:span text:style-name="T1">One</text:span></text:p></office:text></office:body></office:document-content>"#
    );
    let mut d = import(&package(&content, Some(&styles))).unwrap();
    if let Some(wordcraft_doc::Block::Para(p)) = d.body.get_mut(0).map(std::sync::Arc::make_mut) {
        p.text.push('!');
        if let Some(r) = p.runs.last_mut() {
            r.len += 1;
        }
    }
    let saved = export(&d).unwrap();
    let part = content_of(&saved);
    assert!(part.contains(script), "the script as it was: {part}");
    let decls = part.split("<office:font-face-decls>").last().and_then(|t| t.split("</office:font-face-decls>").next()).unwrap();
    assert!(decls.contains(r#"xstyle:name="F1" style:name="Mystery""#), "{decls}");
    assert!(import(&saved).is_ok());
}

/// A font declaration in single quotes is the file's too: it replaces WordCraft's, and isn't
/// declared a second time.
#[test]
fn a_font_declaration_in_single_quotes_is_kept() {
    let ns = format!(r#"{NS_T} xmlns:svg="urn:oasis:names:tc:opendocument:xmlns:svg-compatible:1.0""#);
    let face = r#"<style:font-face style:name='Odd Roman' svg:font-family='Odd Roman' style:font-family-generic='roman'/>"#;
    let styles = format!(
        r#"<office:document-styles {ns}><office:font-face-decls>{face}</office:font-face-decls><office:styles><style:default-style style:family="paragraph"><style:text-properties style:font-name="Odd Roman" fo:font-size="12pt"/></style:default-style></office:styles></office:document-styles>"#
    );
    let content = format!(
        r#"<office:document-content {ns}><office:font-face-decls>{face}</office:font-face-decls><office:automatic-styles><style:style style:name="P1" style:family="paragraph"><style:text-properties style:font-name="Odd Roman"/></style:style></office:automatic-styles><office:body><office:text><text:p text:style-name="P1">x</text:p></office:text></office:body></office:document-content>"#
    );
    let saved = export(&import(&package(&content, Some(&styles))).unwrap()).unwrap();
    for part in [styles_of(&saved), content_of(&saved)] {
        assert!(part.contains(face), "{part}");
        assert_eq!(part.matches("<style:font-face").count(), part.matches("<style:font-face style:name=").count());
        assert_eq!(part.matches(r#"style:name="Odd Roman""#).count(), 0, "declared once: {part}");
        assert_eq!(part.matches("style:name='Odd Roman'").count(), 1, "declared once: {part}");
    }
}

/// A name space only the declarations' container states goes with the declaration: without it the
/// saved part isn't well-formed in its name spaces.
#[test]
fn a_name_space_on_the_font_declarations_goes_with_them() {
    let ns = format!(r#"{NS_T} xmlns:svg="urn:oasis:names:tc:opendocument:xmlns:svg-compatible:1.0""#);
    let decls = r#"<office:font-face-decls xmlns:x="urn:example:x"><style:font-face style:name="Odd Roman" svg:font-family="Odd Roman" x:odd="1"/></office:font-face-decls>"#;
    let styles = format!(
        r#"<office:document-styles {ns}>{decls}<office:styles><style:default-style style:family="paragraph"><style:text-properties style:font-name="Odd Roman" fo:font-size="12pt"/></style:default-style></office:styles></office:document-styles>"#
    );
    let content = format!(
        r#"<office:document-content {ns}>{decls}<office:automatic-styles><style:style style:name="P1" style:family="paragraph"><style:text-properties style:font-name="Odd Roman"/></style:style></office:automatic-styles><office:body><office:text><text:p text:style-name="P1">x</text:p></office:text></office:body></office:document-content>"#
    );
    let saved = export(&import(&package(&content, Some(&styles))).unwrap()).unwrap();
    for part in [styles_of(&saved), content_of(&saved)] {
        assert!(part.contains(r#"x:odd="1""#), "{part}");
        assert!(part.contains(r#"xmlns:x="urn:example:x""#), "{part}");
    }
}
