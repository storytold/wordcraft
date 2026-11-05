//! Synthetic OOXML: replacing an imported cell-range bookmark must create a plain range.
use super::{Session, json, run};
use std::io::{Cursor, Write};

fn source() -> wordcraft_doc::Document {
    let xml = r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:tbl><w:tblPr/><w:tblGrid><w:gridCol w:w="2000"/></w:tblGrid><w:bookmarkStart w:id="5" w:name="Cols" w:colFirst="0" w:colLast="0"/><w:tr><w:tc><w:p><w:r><w:t>abc</w:t></w:r></w:p></w:tc></w:tr><w:bookmarkEnd w:id="5"/></w:tbl><w:p><w:r><w:t>bodytarget</w:t></w:r></w:p><w:sectPr/></w:body></w:document>"#;
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    zip.start_file("word/document.xml", zip::write::SimpleFileOptions::default()).unwrap();
    zip.write_all(xml.as_bytes()).unwrap();
    wordcraft_docx::read(&zip.finish().unwrap().into_inner()).unwrap()
}

#[test]
fn replacing_or_readding_a_column_bookmark_discards_source_placement() {
    for delete_first in [false, true] {
        for target in ["abc", "bodytarget"] {
            let mut s = Session::new(source());
            let source_attrs = s.doc.bookmark_attrs.get("Cols").unwrap().clone();
            assert!(source_attrs.start.is_some() && source_attrs.col_first.is_some());
            if delete_first {
                run(&mut s, "insert.deleteBookmark", json!({"name":"Cols"}));
                assert!(!s.doc.bookmark_attrs.contains_key("Cols"));
            }
            run(&mut s, "select.text", json!({"text": target}));
            run(&mut s, "insert.bookmark", json!({"name":"Cols"}));
            assert!(!s.doc.bookmark_attrs.contains_key("Cols"));
            let bytes = wordcraft_docx::write(&s.doc).unwrap();
            let saved = wordcraft_docx::read(&bytes).unwrap();
            let attrs = saved.bookmark_attrs.get("Cols").unwrap();
            assert!(attrs.start.is_none() && attrs.end.is_none());
            assert!(attrs.col_first.is_none() && attrs.col_last.is_none());
            run(&mut s, "edit.undo", json!({}));
            if delete_first {
                assert!(!s.doc.bookmark_attrs.contains_key("Cols"));
                run(&mut s, "edit.undo", json!({}));
            }
            assert_eq!(s.doc.bookmark_attrs.get("Cols"), Some(&source_attrs));
            run(&mut s, "edit.redo", json!({}));
            if delete_first {
                run(&mut s, "edit.redo", json!({}));
            }
            assert!(!s.doc.bookmark_attrs.contains_key("Cols"));
        }
    }
}
