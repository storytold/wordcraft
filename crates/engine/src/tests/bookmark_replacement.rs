//! Synthetic OOXML: replacing an imported cell-range bookmark must create a plain range.
use super::{Session, json, run};
use std::io::{Cursor, Write};

/// Build a synthetic column bookmark around a single-row table.
fn source() -> wordcraft_doc::Document {
    let xml = r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:tbl><w:tblPr/><w:tblGrid><w:gridCol w:w="2000"/></w:tblGrid><w:bookmarkStart w:id="5" w:name="Cols" w:colFirst="0" w:colLast="0"/><w:tr><w:tc><w:p><w:r><w:t>abc</w:t></w:r></w:p></w:tc></w:tr><w:bookmarkEnd w:id="5"/></w:tbl><w:p><w:r><w:t>bodytarget</w:t></w:r></w:p><w:sectPr/></w:body></w:document>"#;
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    zip.start_file("word/document.xml", zip::write::SimpleFileOptions::default()).unwrap();
    zip.write_all(xml.as_bytes()).unwrap();
    wordcraft_docx::read(&zip.finish().unwrap().into_inner()).unwrap()
}

/// Check cell/body replacement, deletion, save/reopen and undo/redo of an imported column bookmark.
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

/// Check that a recreated name cannot inherit metadata left behind by direct marker deletion.
#[test]
fn readding_a_name_after_its_markers_were_removed_drops_orphaned_attrs() {
    use wordcraft_doc::StoryRef;
    let mut doc = source();
    for path in doc.para_paths(StoryRef::Body) {
        let p = doc.para_mut(StoryRef::Body, &path).unwrap();
        for off in p.object_offsets().into_iter().rev() {
            p.delete(off, off + wordcraft_doc::para::OBJ.len_utf8()).unwrap();
        }
    }
    assert!(doc.bookmarks().is_empty());
    assert!(doc.bookmark_attrs.contains_key("Cols"));
    let mut s = Session::new(doc);
    run(&mut s, "select.text", json!({"text":"abc"}));
    run(&mut s, "insert.bookmark", json!({"name":"Cols"}));
    assert!(!s.doc.bookmark_attrs.contains_key("Cols"));
    let saved = wordcraft_docx::read(&wordcraft_docx::write(&s.doc).unwrap()).unwrap();
    let attrs = saved.bookmark_attrs.get("Cols").unwrap();
    assert!(attrs.start.is_none() && attrs.col_first.is_none() && attrs.col_last.is_none());
}
