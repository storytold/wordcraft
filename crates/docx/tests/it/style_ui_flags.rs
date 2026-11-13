//! A style's own flags and extra information (`w:style` children, ECMA-376 Part 1 §17.7.4) are written back
//! as read: `w:semiHidden`, `w:unhideWhenUsed` and `w:hidden` stay three separate flags, and `w:aliases`,
//! `w:autoRedefine`, `w:personal*` and `w:rsid` are kept. Fixtures are hand-written from the spec.

use std::io::{Read, Write};

const W: &str = r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main""#;
const R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/";

fn docx(styles: &str) -> Vec<u8> {
    let root = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="{R}officeDocument" Target="word/document.xml"/></Relationships>"#
    );
    let doc_rels = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="{R}styles" Target="styles.xml"/></Relationships>"#
    );
    let doc = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document {W}><w:body><w:p><w:pPr><w:pStyle w:val="Body"/></w:pPr><w:r><w:t>A</w:t></w:r></w:p></w:body></w:document>"#
    );
    let styles = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:styles {W}><w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:name w:val="Normal"/><w:qFormat/></w:style>{styles}</w:styles>"#
    );
    let mut zw = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, bytes) in [
        ("_rels/.rels", root.as_bytes()),
        ("word/_rels/document.xml.rels", doc_rels.as_bytes()),
        ("word/document.xml", doc.as_bytes()),
        ("word/styles.xml", styles.as_bytes()),
    ] {
        zw.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
        zw.write_all(bytes).unwrap();
    }
    zw.finish().unwrap().into_inner()
}

fn styles_xml(bytes: &[u8]) -> String {
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let mut s = String::new();
    z.by_name("word/styles.xml").unwrap().read_to_string(&mut s).unwrap();
    s
}

fn save(bytes: &[u8]) -> Vec<u8> {
    wordcraft_docx::write(&wordcraft_docx::read(bytes).expect("read")).expect("write")
}

/// The saved `w:style` element whose `w:styleId` is `id`.
fn style_el(xml: &str, id: &str) -> String {
    let key = format!(r#"w:styleId="{id}""#);
    let at = xml.find(&key).unwrap_or_else(|| panic!("style {id} missing from {xml}"));
    let start = xml[..at].rfind("<w:style ").unwrap();
    let end = start + xml[start..].find("</w:style>").expect("closed style") + "</w:style>".len();
    xml[start..end].to_string()
}

/// Position of `<tag` (as an element: followed by `/`, `>` or a space) in `el`.
fn pos(el: &str, tag: &str) -> Option<usize> {
    ["/>", ">", " "].iter().filter_map(|t| el.find(&format!("<{tag}{t}"))).min()
}

fn has(el: &str, tag: &str) -> bool {
    pos(el, tag).is_some()
}

fn para(id: &str, inner: &str) -> String {
    format!(r#"<w:style w:type="paragraph" w:customStyle="1" w:styleId="{id}"><w:name w:val="{id}"/><w:basedOn w:val="Normal"/>{inner}</w:style>"#)
}

#[test]
fn semi_hidden_alone_is_not_given_unhide_when_used() {
    let out = styles_xml(&save(&docx(&para("Body", "<w:semiHidden/>"))));
    let el = style_el(&out, "Body");
    assert!(has(&el, "w:semiHidden"), "{el}");
    assert!(!has(&el, "w:unhideWhenUsed"), "{el}");
    assert!(!has(&el, "w:hidden"), "{el}");
}

#[test]
fn semi_hidden_with_unhide_when_used_stays_both() {
    let out = styles_xml(&save(&docx(&para("Body", "<w:semiHidden/><w:unhideWhenUsed/>"))));
    let el = style_el(&out, "Body");
    assert!(has(&el, "w:semiHidden") && has(&el, "w:unhideWhenUsed"), "{el}");
    assert!(!has(&el, "w:hidden"), "{el}");
}

#[test]
fn unhide_when_used_alone_is_kept() {
    let out = styles_xml(&save(&docx(&para("Body", "<w:unhideWhenUsed/>"))));
    let el = style_el(&out, "Body");
    assert!(has(&el, "w:unhideWhenUsed"), "{el}");
    assert!(!has(&el, "w:semiHidden"), "{el}");
}

#[test]
fn hidden_stays_hidden_not_semi_hidden() {
    let out = styles_xml(&save(&docx(&para("Body", "<w:hidden/>"))));
    let el = style_el(&out, "Body");
    assert!(has(&el, "w:hidden"), "{el}");
    assert!(!has(&el, "w:semiHidden"), "{el}");
    assert!(!has(&el, "w:unhideWhenUsed"), "{el}");
}

#[test]
fn flags_set_off_are_not_written() {
    let inner = r#"<w:hidden w:val="0"/><w:semiHidden w:val="false"/><w:autoRedefine w:val="0"/><w:personal w:val="off"/>"#;
    let out = styles_xml(&save(&docx(&para("Body", inner))));
    let el = style_el(&out, "Body");
    for tag in ["w:hidden", "w:semiHidden", "w:unhideWhenUsed", "w:autoRedefine", "w:personal"] {
        assert!(!has(&el, tag), "{tag} written: {el}");
    }
}

#[test]
fn aliases_auto_redefine_personal_and_rsid_are_kept_in_schema_order() {
    let inner = concat!(
        r#"<w:aliases w:val="H1,Head"/><w:next w:val="Normal"/><w:autoRedefine/><w:uiPriority w:val="9"/>"#,
        r#"<w:semiHidden/><w:qFormat/><w:personal/><w:personalCompose/><w:personalReply/>"#,
        r#"<w:rsid w:val="00ab12cd"/><w:pPr><w:spacing w:after="120"/></w:pPr>"#
    );
    let out = styles_xml(&save(&docx(&para("Body", inner))));
    let el = style_el(&out, "Body");
    assert!(el.contains(r#"<w:aliases w:val="H1,Head"/>"#), "{el}");
    assert!(el.contains(r#"<w:rsid w:val="00AB12CD"/>"#), "{el}");
    // CT_Style order: name, aliases, basedOn, next, link, autoRedefine, hidden, uiPriority, semiHidden,
    // unhideWhenUsed, qFormat, locked, personal, personalCompose, personalReply, rsid, pPr, rPr ...
    let order = [
        "w:name",
        "w:aliases",
        "w:basedOn",
        "w:next",
        "w:autoRedefine",
        "w:uiPriority",
        "w:semiHidden",
        "w:qFormat",
        "w:personal",
        "w:personalCompose",
        "w:personalReply",
        "w:rsid",
        "w:pPr",
    ];
    let at: Vec<usize> = order.iter().map(|t| pos(&el, t).unwrap_or_else(|| panic!("{t} missing: {el}"))).collect();
    for (i, w) in at.windows(2).enumerate() {
        assert!(w[0] < w[1], "{} must come before {}: {el}", order[i], order[i + 1]);
    }
}

#[test]
fn a_bad_rsid_is_dropped() {
    let out = styles_xml(&save(&docx(&para("Body", r#"<w:rsid w:val="xyz"/>"#))));
    let el = style_el(&out, "Body");
    assert!(!has(&el, "w:rsid"), "{el}");
}

#[test]
fn a_second_save_writes_the_same_style() {
    let inner = r#"<w:aliases w:val="A1"/><w:autoRedefine/><w:hidden/><w:personalReply/><w:rsid w:val="0012ABCD"/>"#;
    let once = save(&docx(&para("Body", inner)));
    let twice = save(&once);
    let el = style_el(&styles_xml(&once), "Body");
    assert!(el.contains(r#"<w:aliases w:val="A1"/>"#) && has(&el, "w:hidden") && has(&el, "w:personalReply"), "{el}");
    assert_eq!(el, style_el(&styles_xml(&twice), "Body"));
}

#[test]
fn a_style_hidden_in_the_app_still_writes_semi_hidden_and_unhide_when_used() {
    // A style read with no visibility flags and then hidden through the model keeps today's output.
    let mut d = wordcraft_docx::read(&docx(&para("Body", ""))).expect("read");
    let st = std::sync::Arc::make_mut(&mut d.styles).styles.iter_mut().find(|s| s.id == "Body").expect("Body style");
    st.hidden = true;
    let el = style_el(&styles_xml(&wordcraft_docx::write(&d).expect("write")), "Body");
    assert!(has(&el, "w:semiHidden") && has(&el, "w:unhideWhenUsed"), "{el}");
}
