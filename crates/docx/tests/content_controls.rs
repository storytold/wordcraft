//! Content controls (`w:sdt`) keep their properties, levels and nesting through a round trip.
//! All markup here is synthetic, written from ECMA-376 §17.5.2 and the [MS-DOCX] `w14`/`w15`
//! extensions.

use std::io::Write;

use wordcraft_doc::control::{ControlKind, ControlLock, ListItem};
use wordcraft_doc::{Block, ContentControl, Document, StoryRef};
use wordcraft_docx::{read, write};

const NS: &str = r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:w14="http://schemas.microsoft.com/office/word/2010/wordml" xmlns:w15="http://schemas.microsoft.com/office/word/2012/wordml""#;
const ROOT_RELS: &str = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#;

fn package(body: &str) -> Vec<u8> {
    let doc = format!(r#"<w:document {NS}><w:body>{body}</w:body></w:document>"#);
    let mut zw = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, bytes) in [("_rels/.rels", ROOT_RELS.as_bytes()), ("word/document.xml", doc.as_bytes())] {
        zw.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
        zw.write_all(bytes).unwrap();
    }
    zw.finish().unwrap().into_inner()
}

fn t(s: &str) -> String {
    format!(r#"<w:r><w:t xml:space="preserve">{s}</w:t></w:r>"#)
}
fn p(inner: &str) -> String {
    format!("<w:p>{inner}</w:p>")
}
fn sdt(pr: &str, content: &str) -> String {
    format!("<w:sdt><w:sdtPr>{pr}</w:sdtPr><w:sdtContent>{content}</w:sdtContent></w:sdt>")
}
fn cell(text: &str) -> String {
    format!("<w:tc>{}</w:tc>", p(&t(text)))
}

/// Every kind of control, at every level, with nesting.
fn form() -> Vec<u8> {
    let block = sdt(
        r#"<w:rPr><w:b/></w:rPr><w:alias w:val="Intro"/><w:tag w:val="intro"/><w:id w:val="-1201"/><w:lock w:val="sdtLocked"/><w:placeholder><w:docPart w:val="DefaultPlaceholder_1"/></w:placeholder><w:showingPlcHdr/><w15:appearance w15:val="tags"/>"#,
        &format!("{}{}", p(&t("Write the introduction.")), p(&t("Second paragraph"))),
    );
    let inline = p(&format!(
        "{}{}{}{}{}{}",
        t("Name: "),
        sdt(r#"<w:alias w:val="Name"/><w:id w:val="7"/><w:text w:multiLine="1"/>"#, &t("Ada")),
        t(" Agree: "),
        sdt(
            r#"<w:id w:val="8"/><w14:checkbox><w14:checked w14:val="1"/><w14:checkedState w14:val="2612" w14:font="Symbola"/><w14:uncheckedState w14:val="2610" w14:font="Symbola"/></w14:checkbox>"#,
            &t("\u{2612}")
        ),
        sdt(
            r#"<w:id w:val="9"/><w:lock w:val="contentLocked"/><w:dropDownList w:lastValue="b"><w:listItem w:displayText="Alpha" w:value="a"/><w:listItem w:displayText="Beta" w:value="b"/></w:dropDownList>"#,
            &t("Beta")
        ),
        sdt(
            r#"<w:id w:val="10"/><w:dataBinding w:xpath="/root/when" w:storeItemID="{00000000-0000-0000-0000-000000000001}"/><w:date w:fullDate="2026-10-11T00:00:00Z"><w:dateFormat w:val="d.M.yyyy"/><w:lid w:val="de-DE"/><w:storeMappedDataAs w:val="dateTime"/><w:calendar w:val="gregorian"/></w:date>"#,
            &t("11.10.2026")
        ),
    ));
    // A combo box holding a nested rich text control.
    let nested = p(&sdt(
        r#"<w:id w:val="11"/><w:comboBox><w:listItem w:displayText="One" w:value="1"/></w:comboBox>"#,
        &format!("{}{}", t("Pick "), sdt(r#"<w:id w:val="12"/><w:richText/>"#, &t("inner"))),
    ));
    let gallery = sdt(r#"<w:id w:val="13"/><w:docPartObj><w:docPartGallery w:val="Cover Pages"/><w:docPartUnique/></w:docPartObj>"#, &p(&t("Cover")));
    // A repeating section around table rows, each row an item; a control around one cell; a
    // picture control holding nothing but a paragraph mark's worth of text.
    let row = |a: &str, b: &str| format!("<w:tr>{}{}</w:tr>", cell(a), cell(b));
    let table = format!(
        "<w:tbl><w:tblGrid><w:gridCol w:w=\"2000\"/><w:gridCol w:w=\"2000\"/></w:tblGrid>{}</w:tbl>",
        sdt(
            r#"<w:id w:val="20"/><w15:repeatingSection><w15:sectionTitle w:val="Items"/></w15:repeatingSection>"#,
            &format!(
                "{}{}",
                sdt(r#"<w:id w:val="21"/><w15:repeatingSectionItem/>"#, &row("x", "y")),
                sdt(
                    r#"<w:id w:val="22"/><w15:repeatingSectionItem/>"#,
                    &format!("<w:tr>{}{}</w:tr>", cell("z"), sdt(r#"<w:id w:val="23"/><w:picture/>"#, &cell("pic")))
                )
            )
        )
    );
    // A block control whose content is a table.
    let around_table = sdt(
        r#"<w:id w:val="30"/><w:tag w:val="grid"/>"#,
        &format!("<w:tbl><w:tblGrid><w:gridCol w:w=\"2000\"/></w:tblGrid>{}</w:tbl>", format_args!("<w:tr>{}</w:tr>", cell("t"))),
    );
    package(&format!("{block}{inline}{nested}{gallery}{table}{around_table}{}", p(&t("End"))))
}

fn document_xml(docx: &[u8]) -> String {
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(docx)).unwrap();
    let mut s = String::new();
    std::io::Read::read_to_string(&mut z.by_name("word/document.xml").unwrap(), &mut s).unwrap();
    s
}

fn controls(d: &Document) -> Vec<ContentControl> {
    let mut out = Vec::new();
    wordcraft_doc::control::walk_controls(&d.body, &mut |c| out.push(c.clone()));
    out
}

fn by_id(d: &Document, id: i64) -> ContentControl {
    controls(d).into_iter().find(|c| c.id == Some(id)).unwrap_or_else(|| panic!("control {id}"))
}

#[test]
fn every_kind_reads_with_its_properties() {
    let d = read(&form()).unwrap();
    let intro = controls(&d).into_iter().find(|c| c.title == "Intro").unwrap();
    assert!(intro.block && intro.showing_placeholder);
    assert_eq!((intro.tag.as_str(), intro.id, intro.lock), ("intro", Some(-1201), ControlLock::SdtLocked));
    assert_eq!(intro.placeholder, "DefaultPlaceholder_1");
    assert_eq!(intro.placeholder_text, "Write the introduction.");
    assert!(intro.rpr_xml.contains("w:b"));
    assert_eq!(intro.extra, vec![r#"<w15:appearance w15:val="tags"/>"#.to_string()]);
    let ranges = d.control_ranges(StoryRef::Body);
    let r = ranges.iter().find(|r| r.control.title == "Intro").unwrap();
    assert_eq!(d.control_text(r), "Write the introduction.\nSecond paragraph");

    assert_eq!(by_id(&d, 7).kind, ControlKind::Text { multi_line: true });
    assert!(!by_id(&d, 7).block);
    assert!(
        matches!(by_id(&d, 8).kind, ControlKind::CheckBox { checked: true, checked_char: '\u{2612}', ref checked_font, .. } if checked_font == "Symbola")
    );
    let dd = by_id(&d, 9);
    assert_eq!(dd.lock, ControlLock::ContentLocked);
    assert_eq!(
        dd.kind,
        ControlKind::DropDown {
            items: vec![ListItem { display: "Alpha".into(), value: "a".into() }, ListItem { display: "Beta".into(), value: "b".into() }],
            last_value: "b".into()
        }
    );
    assert!(
        matches!(by_id(&d, 10).kind, ControlKind::Date { ref full_date, ref format, .. } if full_date == "2026-10-11T00:00:00Z" && format == "d.M.yyyy")
    );
    assert_eq!(by_id(&d, 10).extra.len(), 1, "data binding kept");
    assert_eq!(by_id(&d, 12).kind, ControlKind::RichText);
    assert!(matches!(by_id(&d, 13).kind, ControlKind::Gallery { list: false, ref gallery, unique: true, .. } if gallery == "Cover Pages"));
    assert!(matches!(by_id(&d, 20).kind, ControlKind::RepeatingSection { ref title, .. } if title == "Items"));
    assert_eq!(by_id(&d, 21).kind, ControlKind::RepeatingSectionItem);
    assert_eq!(by_id(&d, 23).kind, ControlKind::Picture);
    // Row-level controls sit on the rows, the cell-level one on its cell, the table's on the table.
    let tables: Vec<_> = d.body.iter().filter_map(|b| b.as_table()).collect();
    assert_eq!(tables.len(), 2);
    let rows = &tables[0].rows;
    assert_eq!(rows[0].controls.open.iter().map(|c| c.id).collect::<Vec<_>>(), vec![Some(20), Some(21)]);
    assert_eq!((rows[0].controls.close, rows[1].controls.close), (1, 2));
    assert_eq!(rows[1].cells[1].controls.open[0].id, Some(23));
    assert_eq!(tables[1].controls.open[0].tag, "grid");
    assert_eq!(tables[1].controls.close, 1);
    // The text reads as before.
    assert!(d.plain_text(StoryRef::Body).contains("Name: Ada Agree: \u{2612}Beta11.10.2026"));
}

#[test]
fn controls_round_trip() {
    let d = read(&form()).unwrap();
    let bytes = write(&d).unwrap();
    let back = read(&bytes).unwrap();
    assert_eq!(controls(&back), controls(&d));
    assert_eq!(back.plain_text(StoryRef::Body), d.plain_text(StoryRef::Body));
    assert_eq!(back.body.len(), d.body.len());
    let names: Vec<_> = back.control_ranges(StoryRef::Body).iter().map(|r| (r.control.id, r.start.path.clone(), r.end.path.clone())).collect();
    let before: Vec<_> = d.control_ranges(StoryRef::Body).iter().map(|r| (r.control.id, r.start.path.clone(), r.end.path.clone())).collect();
    assert_eq!(names, before);
    // Written in schema order, with the extension elements the file had.
    let xml = document_xml(&bytes);
    assert!(xml.contains(
        r#"<w:sdtPr><w:rPr><w:b/></w:rPr><w:alias w:val="Intro"/><w:tag w:val="intro"/><w:id w:val="-1201"/><w:lock w:val="sdtLocked"/><w:placeholder><w:docPart w:val="DefaultPlaceholder_1"/></w:placeholder><w:showingPlcHdr/><w15:appearance w15:val="tags"/></w:sdtPr>"#
    ));
    assert!(xml.contains(r#"<w14:checkbox><w14:checked w14:val="1"/><w14:checkedState w14:val="2612" w14:font="Symbola"/>"#));
    // The block control is around the paragraphs, the inline ones inside theirs.
    assert!(xml.contains("<w:sdtContent><w:p>"));
    assert!(xml.contains(r#"<w:dataBinding w:xpath="/root/when" w:storeItemID="{00000000-0000-0000-0000-000000000001}"/><w:date"#));
}

#[test]
fn edited_documents_still_write_whole_controls() {
    let mut d = read(&form()).unwrap();
    // Merge the block control's first paragraph into the one before nothing: delete across the
    // start of the inline name control and the end of the block control.
    let ranges = d.control_ranges(StoryRef::Body);
    let name = ranges.iter().find(|r| r.control.id == Some(7)).unwrap().clone();
    let intro = ranges.iter().find(|r| r.control.title == "Intro").unwrap().clone();
    let from = wordcraft_doc::Pos { off: 3, ..intro.end.clone() };
    let to = wordcraft_doc::Pos { off: name.start.off + 4, ..name.start.clone() };
    d.delete_range(&from, &to).unwrap();
    // Both controls are still there, emptied where the deletion went through them.
    let after = d.control_ranges(StoryRef::Body);
    assert!(after.iter().any(|r| r.control.id == Some(7)));
    assert!(after.iter().any(|r| r.control.title == "Intro"));
    let back = read(&write(&d).unwrap()).unwrap();
    assert_eq!(controls(&back).len(), controls(&d).len());
    // An orphan marker (a table carrying an open that lost its table) is dropped on save.
    if let Some(Block::Table(t)) = d.body.iter_mut().rev().find(|b| b.as_table().is_some()).map(std::sync::Arc::make_mut) {
        t.controls.close = 0;
    }
    assert!(d.has_unbalanced_controls());
    let back = read(&write(&d).unwrap()).unwrap();
    assert!(!back.has_unbalanced_controls());
}

#[test]
fn deeply_nested_controls_are_bounded() {
    let mut s = t("core");
    for i in 0..400 {
        s = sdt(&format!(r#"<w:id w:val="{i}"/>"#), &s);
    }
    let mut b = p(&t("x"));
    for i in 0..400 {
        b = sdt(&format!(r#"<w:id w:val="{i}"/>"#), &b);
    }
    for body in [p(&s), b] {
        // Too deep for the parser: an error or a document, never a panic or a hang.
        if let Ok(d) = read(&package(&body)) {
            let _ = write(&d).unwrap();
        }
    }
    let mut s = t("core");
    for i in 0..60 {
        s = sdt(&format!(r#"<w:id w:val="{i}"/>"#), &s);
    }
    let d = read(&package(&p(&s))).unwrap();
    let back = read(&write(&d).unwrap()).unwrap();
    assert_eq!(controls(&back).len(), 60);
}
