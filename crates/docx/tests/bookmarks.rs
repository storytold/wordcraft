//! Synthetic OOXML only: bookmark containers and source attributes survive repeated saves.
use std::io::{Cursor, Read, Write};

const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";

fn package(body: &str, header: &str) -> Vec<u8> {
    let document = format!(
        r#"<w:document xmlns:w="{W}" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><w:body>{body}<w:sectPr><w:headerReference w:type="default" r:id="header"/></w:sectPr></w:body></w:document>"#
    );
    let header = format!(r#"<w:hdr xmlns:w="{W}">{header}</w:hdr>"#);
    let rels = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="header" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/header" Target="header1.xml"/></Relationships>"#;
    let root = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="document" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#;
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (name, text) in
        [("_rels/.rels", root), ("word/document.xml", &document), ("word/_rels/document.xml.rels", rels), ("word/header1.xml", &header)]
    {
        zip.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
        zip.write_all(text.as_bytes()).unwrap();
    }
    zip.finish().unwrap().into_inner()
}

fn xml(bytes: &[u8], part: &str) -> String {
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
    let mut out = String::new();
    zip.by_name(part).unwrap().read_to_string(&mut out).unwrap();
    out
}

fn start(id: u32, name: &str, columns: bool) -> String {
    let columns = if columns { r#" w:colFirst="1" w:colLast="2""# } else { "" };
    format!(r#"<w:bookmarkStart w:id="{id}" w:name="{name}"{columns}/>"#)
}
fn end(id: u32) -> String {
    format!(r#"<w:bookmarkEnd w:id="{id}"/>"#)
}
fn para(text: &str) -> String {
    format!("<w:p><w:r><w:t>{text}</w:t></w:r></w:p>")
}
fn cell(text: &str) -> String {
    format!("<w:tc>{}</w:tc>", para(text))
}
fn table(content: &str) -> String {
    format!(r#"<w:tbl><w:tblPr/><w:tblGrid><w:gridCol w:w="1000"/><w:gridCol w:w="1000"/><w:gridCol w:w="1000"/></w:tblGrid>{content}</w:tbl>"#)
}
fn row(content: &str) -> String {
    format!("<w:tr>{content}</w:tr>")
}

/// (parent, number of preceding structural children, tag, sorted attributes), in XML order.
fn markers(xml: &str) -> Vec<(String, usize, String, Vec<(String, String)>)> {
    use quick_xml::events::Event;
    let mut r = quick_xml::Reader::from_str(xml);
    let mut stack: Vec<(String, usize)> = Vec::new();
    let mut result = Vec::new();
    loop {
        match r.read_event().unwrap() {
            Event::Start(e) | Event::Empty(e) if matches!(e.name().as_ref(), b"w:bookmarkStart" | b"w:bookmarkEnd") => {
                let (parent, at) = stack.last().cloned().unwrap();
                let mut attrs: Vec<_> = e
                    .attributes()
                    .map(|a| {
                        let a = a.unwrap();
                        (
                            String::from_utf8(a.key.as_ref().to_vec()).unwrap(),
                            a.normalized_value(quick_xml::XmlVersion::Implicit1_0).unwrap().into_owned(),
                        )
                    })
                    .collect();
                attrs.sort();
                result.push((parent, at, String::from_utf8(e.name().as_ref().to_vec()).unwrap(), attrs));
            }
            Event::Start(e) => {
                if matches!(e.name().as_ref(), b"w:tr" | b"w:tc" | b"w:tbl" | b"w:p")
                    && let Some((_, at)) = stack.last_mut()
                {
                    *at += 1;
                }
                stack.push((String::from_utf8(e.name().as_ref().to_vec()).unwrap(), 0));
            }
            Event::Empty(e) => {
                if matches!(e.name().as_ref(), b"w:tr" | b"w:tc" | b"w:tbl" | b"w:p")
                    && let Some((_, at)) = stack.last_mut()
                {
                    *at += 1;
                }
            }
            Event::End(_) => {
                stack.pop();
            }
            Event::Eof => break,
            _ => {}
        }
    }
    result
}

#[test]
fn row_cell_and_paragraph_bookmarks_keep_ids_names_and_column_bounds() {
    let cells = format!(
        "{}<w:tc>{}{}{}</w:tc><w:tc><w:p>{}<w:r><w:t>paragraph</w:t></w:r>{}</w:p></w:tc>",
        cell("one"),
        start(42, "Cell", false),
        para("two"),
        end(42),
        start(73, "Paragraph", false),
        end(73)
    );
    let first = row(&format!("{}{}{}", start(19, "CellRange", true), cells, end(19)));
    let second = row(&format!("{}{}{}", cell("four"), start(81, "MiddleCell", true), cell("five")));
    let third = row(&format!("{}{}{}", cell("six"), end(81), cell("seven")));
    let body = table(&format!("{}{}{}{}{}", start(93, "Table", true), first, second, third, end(93)));
    let input = package(&format!("{body}{}", para("after")), &para("header"));
    let mut d = wordcraft_docx::read(&input).unwrap();
    let wanted = markers(&xml(&input, "word/document.xml"));
    for _ in 0..2 {
        let output = wordcraft_docx::write(&d).unwrap();
        let actual = markers(&xml(&output, "word/document.xml"));
        assert_eq!(actual, wanted);
        let names: std::collections::BTreeSet<_> = d.bookmarks().into_iter().map(|(name, _)| name).collect();
        assert_eq!(names, ["Table", "CellRange", "Cell", "Paragraph", "MiddleCell"].map(str::to_string).into_iter().collect());
        d = wordcraft_docx::read(&output).unwrap();
    }
}

#[test]
fn body_bookmarks_between_two_tables_stay_in_the_body() {
    let t = table(&row(&cell("table")));
    let body = format!("{t}{}{}{t}{}", start(108, "BetweenTables", true), end(108), para("after"));
    let input = package(&body, &para("header"));
    let wanted = markers(&xml(&input, "word/document.xml"));
    let mut d = wordcraft_docx::read(&input).unwrap();
    for _ in 0..2 {
        let output = wordcraft_docx::write(&d).unwrap();
        assert_eq!(markers(&xml(&output, "word/document.xml")), wanted);
        d = wordcraft_docx::read(&output).unwrap();
    }
}

#[test]
fn header_row_table_and_paragraph_bookmarks_round_trip() {
    let row = row(&format!("{}{}{}", start(204, "HeaderRow", true), cell("header cell"), end(204)));
    let header = format!(
        "{}{}{}{}<w:p>{}<w:r><w:t>header paragraph</w:t></w:r>{}</w:p>",
        table(&format!("{}{}{}", start(205, "HeaderTable", true), row, end(205))),
        start(207, "HeaderBetweenTables", false),
        end(207),
        table(&row_without_bookmarks()),
        start(206, "HeaderParagraph", false),
        end(206)
    );
    let input = package(&para("body"), &header);
    let wanted = markers(&xml(&input, "word/header1.xml"));
    let mut d = wordcraft_docx::read(&input).unwrap();
    for _ in 0..2 {
        let output = wordcraft_docx::write(&d).unwrap();
        assert_eq!(markers(&xml(&output, "word/header1.xml")), wanted);
        d = wordcraft_docx::read(&output).unwrap();
    }
}

fn row_without_bookmarks() -> String {
    row(&cell("second table"))
}

#[test]
fn newly_created_bookmarks_do_not_reuse_imported_or_allocated_ids() {
    use wordcraft_doc::{CharProps, InlineObject, Pos};
    let input = package(
        &format!("<w:p>{}<w:r><w:t>text</w:t></w:r>{}{}{}</w:p>", start(0, "OriginalZero", false), end(0), start(2, "OriginalTwo", false), end(2)),
        &para("header"),
    );
    let mut d = wordcraft_docx::read(&input).unwrap();
    for name in ["NewOne", "NewTwo"] {
        d.insert_object(&Pos::body(0, 0), InlineObject::BookmarkEnd { name: name.into() }, &CharProps::default()).unwrap();
        d.insert_object(&Pos::body(0, 0), InlineObject::BookmarkStart { name: name.into() }, &CharProps::default()).unwrap();
    }
    let output = wordcraft_docx::write(&d).unwrap();
    let actual = markers(&xml(&output, "word/document.xml"));
    let mut ids = std::collections::BTreeSet::new();
    for (_, _, tag, attrs) in &actual {
        if tag != "w:bookmarkStart" {
            continue;
        }
        let value = |key| attrs.iter().find(|a| a.0 == key).unwrap().1.clone();
        let id = value("w:id");
        assert!(ids.insert(id.clone()));
        match value("w:name").as_str() {
            "OriginalZero" => assert_eq!(id, "0"),
            "OriginalTwo" => assert_eq!(id, "2"),
            _ => {}
        }
    }
    assert_eq!(ids.len(), 4);
    assert_eq!(actual.iter().filter(|m| m.2 == "w:bookmarkEnd").count(), 4);
}

#[test]
fn moved_or_removed_bookmarks_are_not_recreated_at_stale_source_boundaries() {
    use wordcraft_doc::{CharProps, Path, StoryRef};
    let input = package(&format!("{}{}{}", start(309, "Editable", false), para("text"), end(309)), &para("header"));
    let mut d = wordcraft_docx::read(&input).unwrap();
    assert_eq!(d.bookmarks().len(), 1);
    let p = d.para_mut(StoryRef::Body, &Path::top(0)).unwrap();
    p.insert_text(0, "prefix ", &CharProps::default()).unwrap();
    let output = wordcraft_docx::write(&d).unwrap();
    let actual = markers(&xml(&output, "word/document.xml"));
    assert_eq!(actual.len(), 2);
    assert_eq!(actual[0].0, "w:p");
    assert_eq!(actual[1].0, "w:body");
    let p = d.para_mut(StoryRef::Body, &Path::top(0)).unwrap();
    for off in p.object_offsets().into_iter().rev() {
        p.delete(off, off + wordcraft_doc::para::OBJ.len_utf8()).unwrap();
    }
    assert!(d.bookmarks().is_empty());
    let output = wordcraft_docx::write(&d).unwrap();
    assert!(markers(&xml(&output, "word/document.xml")).is_empty());
}

#[test]
fn many_row_bookmarks_keep_their_order_and_native_objects() {
    let mut begins = String::new();
    let mut ends = String::new();
    for i in 0..1000 {
        begins.push_str(&start(1000 + i, &format!("RowRange{i}"), true));
        ends.push_str(&end(1000 + i));
    }
    let input = package(&table(&row(&format!("{begins}{}{ends}", cell("text")))), &para("header"));
    let d = wordcraft_docx::read(&input).unwrap();
    assert_eq!(d.bookmarks().len(), 1000);
    let output = wordcraft_docx::write(&d).unwrap();
    assert_eq!(markers(&xml(&output, "word/document.xml")), markers(&xml(&input, "word/document.xml")));
}

#[test]
fn direct_table_and_row_bookmarks_stay_outside_child_content_controls() {
    let control = |id, content| format!(r#"<w:sdt><w:sdtPr><w:id w:val="{id}"/></w:sdtPr><w:sdtContent>{content}</w:sdtContent></w:sdt>"#);
    let cells = format!("{}{}{}", cell("one"), cell("two"), cell("three"));
    let controlled_row = row(&format!("{}{}{}", start(401, "RowOutsideCellControl", true), control(41, cells), end(401)));
    let body = table(&format!("{}{}{}", start(402, "TableOutsideRowControl", true), control(42, controlled_row), end(402)));
    let input = package(&body, &para("header"));
    let wanted = markers(&xml(&input, "word/document.xml"));
    let mut d = wordcraft_docx::read(&input).unwrap();
    for _ in 0..2 {
        let output = wordcraft_docx::write(&d).unwrap();
        assert_eq!(markers(&xml(&output, "word/document.xml")), wanted);
        d = wordcraft_docx::read(&output).unwrap();
    }
}
