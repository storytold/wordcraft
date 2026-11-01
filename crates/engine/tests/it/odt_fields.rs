//! Fields in an ODT file update in WordCraft as they do in Word: a reference to a bookmark shows
//! the bookmarked text (a bookmark over text, `text:bookmark-start`…`-end`, not a point). The
//! ODT is original and synthetic, shaped like LibreOffice's.

use std::io::Write;

use serde_json::json;
use wordcraft_engine::{Session, io};

const NS: &str = r#"xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0" xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0""#;

fn odt(body: &str) -> Vec<u8> {
    let content = format!(r#"<office:document-content {NS}><office:body><office:text>{body}</office:text></office:body></office:document-content>"#);
    let mut z = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let o = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    for (name, data) in [("mimetype", "application/vnd.oasis.opendocument.text"), ("content.xml", content.as_str())] {
        z.start_file(name, o).unwrap();
        z.write_all(data.as_bytes()).unwrap();
    }
    z.finish().unwrap().into_inner()
}

fn text(s: &Session) -> Vec<String> {
    s.doc.body.iter().filter_map(|b| b.as_para()).map(|p| p.text.replace(wordcraft_doc::para::OBJ, "")).collect()
}

/// The displayed results of the fields in the body.
fn results(s: &Session) -> Vec<String> {
    s.doc
        .body
        .iter()
        .filter_map(|b| b.as_para())
        .flat_map(|p| p.objects.iter())
        .filter_map(|o| match o {
            wordcraft_doc::para::InlineObject::Field { result, .. } => Some(result.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn a_reference_to_a_bookmark_updates_to_its_text() {
    let doc = io::open_bytes(
        "refs.odt",
        &odt(r#"<text:h text:outline-level="1"><text:bookmark-start text:name="__RefHeading___Toc1"/>Planning ahead<text:bookmark-end text:name="__RefHeading___Toc1"/></text:h><text:p>See <text:bookmark-ref text:reference-format="text" text:ref-name="__RefHeading___Toc1">Planning</text:bookmark-ref>.</text:p>"#),
    )
    .unwrap();
    let mut s = Session::new(doc);
    assert_eq!(results(&s), ["Planning"]);
    s.run("references.updateFields", &json!({"all": true})).unwrap();
    assert_eq!(results(&s), ["Planning ahead"], "{:?}", text(&s));
    // And after a save in ODT and opening again.
    let again = io::open_bytes("refs.odt", &io::save_bytes("refs.odt", &s.doc).unwrap()).unwrap();
    let mut s2 = Session::new(again);
    s2.run("references.updateFields", &json!({"all": true})).unwrap();
    assert_eq!(results(&s2), ["Planning ahead"]);
}

#[test]
fn captions_chapters_and_dates_update_as_words() {
    // Stale results, as a file can hold them: they are recomputed.
    let doc = io::open_bytes(
        "fields.odt",
        &odt(r#"<text:h text:outline-level="1">Methods</text:h>
        <text:p>Figure <text:sequence text:ref-name="refFigure0" text:name="Figure" text:formula="ooow:Figure+1" style:num-format="1">7</text:sequence>: a cat</text:p>
        <text:p>Figure <text:sequence text:ref-name="refFigure1" text:name="Figure" text:formula="ooow:Figure+1" style:num-format="1">9</text:sequence>: a dog</text:p>
        <text:p>See <text:sequence-ref text:reference-format="category-and-value" text:ref-name="refFigure1">Figure 9</text:sequence-ref> in <text:chapter text:display="name" text:outline-level="1">Old</text:chapter>.</text:p>"#),
    )
    .unwrap();
    let mut s = Session::new(doc);
    s.run("references.updateFields", &json!({"all": true})).unwrap();
    assert_eq!(results(&s), ["1", "2", "Figure 2", "Methods"], "{:?}", text(&s));
}
