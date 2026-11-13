//! Floating tables (`w:tblpPr`, ECMA-376 Part 1 §17.4.58). Fixtures are hand-written.

use std::io::Write;

use wordcraft_doc::Block;
use wordcraft_doc::para::{Anchor, FloatAlign};

fn docx(body: &str) -> Vec<u8> {
    let root = r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#;
    let doc = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>{body}</w:body></w:document>"#
    );
    let mut zw = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, bytes) in [("_rels/.rels", root.as_bytes()), ("word/document.xml", doc.as_bytes())] {
        zw.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
        zw.write_all(bytes).unwrap();
    }
    zw.finish().unwrap().into_inner()
}

/// #5669: a table aligned to the bottom (`w:tblpYSpec="bottom"`) with no `w:vertAnchor` sits at
/// the bottom of the margin in Word, so it reads as anchored to the margin, not the paragraph
/// (whose zero-height area put it at the top of the page).
#[test]
fn floating_table_aligned_without_vert_anchor_is_on_the_margin() {
    let table = |pr: &str| {
        format!(
            r#"<w:tbl><w:tblPr>{pr}<w:tblW w:w="2000" w:type="dxa"/></w:tblPr><w:tblGrid><w:gridCol w:w="2000"/></w:tblGrid><w:tr><w:tc><w:p><w:r><w:t>x</w:t></w:r></w:p></w:tc></w:tr></w:tbl><w:p/>"#
        )
    };
    let body = [
        table(r#"<w:tblpPr w:topFromText="284" w:horzAnchor="margin" w:tblpYSpec="bottom"/><w:tblOverlap w:val="never"/>"#),
        table(r#"<w:tblpPr w:horzAnchor="margin" w:tblpY="200"/>"#),
        table(r#"<w:tblpPr w:vertAnchor="text" w:tblpYSpec="bottom"/>"#),
    ]
    .concat();
    let d = wordcraft_docx::read(&docx(&body)).unwrap();
    let floats: Vec<_> = d.body.iter().filter_map(|b| if let Block::Table(t) = &**b { t.props.float } else { None }).collect();
    assert_eq!(floats.len(), 3);
    let bottom = floats[0];
    assert_eq!((bottom.h_rel, bottom.v_rel, bottom.v_align, bottom.overlap), (Anchor::Margin, Anchor::Margin, Some(FloatAlign::End), false));
    // An offset without an alignment keeps the paragraph, and so does an explicit "text".
    assert_eq!(floats[1].v_rel, Anchor::Paragraph);
    assert_eq!(floats[2].v_rel, Anchor::Paragraph);
}
