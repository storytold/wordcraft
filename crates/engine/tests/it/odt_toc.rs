//! A table of contents in an ODT file (ODF 1.3 §8.3 `text:table-of-content`) opens as one
//! WordCraft updates: References › Update Table regenerates its entries from the document's
//! headings, with or without a title, and it is saved back as an ODF table of contents. The ODT
//! is original and synthetic, shaped like LibreOffice's.

use std::io::Write;

use serde_json::json;
use wordcraft_engine::{Session, io};

const NS: &str = r#"xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0" xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0""#;

/// A document with a table of contents whose entries are stale (made before the headings were
/// renamed), titled or not.
fn odt(titled: bool) -> Vec<u8> {
    let title = if titled {
        r#"<text:index-title text:name="T_Head"><text:p text:style-name="Contents_20_Heading">Contents</text:p></text:index-title>"#
    } else {
        ""
    };
    let content = format!(
        r#"<office:document-content {NS}><office:body><office:text>
        <text:table-of-content text:name="T"><text:table-of-content-source text:outline-level="2"/><text:index-body>{title}<text:p text:style-name="Contents_20_1">Old chapter<text:tab/>1</text:p></text:index-body></text:table-of-content>
        <text:h text:outline-level="1">Planning</text:h><text:p>Text.</text:p>
        <text:h text:outline-level="2">Budget</text:h><text:p>More text.</text:p>
        <text:h text:outline-level="3">Too deep</text:h><text:p>Last.</text:p>
        </office:text></office:body></office:document-content>"#
    );
    let mut z = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let o = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    for (name, data) in [("mimetype", "application/vnd.oasis.opendocument.text"), ("content.xml", content.as_str())] {
        z.start_file(name, o).unwrap();
        z.write_all(data.as_bytes()).unwrap();
    }
    z.finish().unwrap().into_inner()
}

/// The text of the body's paragraphs in the TOC styles (without an untitled table's field
/// paragraph, which the engine keeps where the table started).
fn entries(s: &Session) -> Vec<String> {
    s.doc
        .body
        .iter()
        .filter_map(|b| b.as_para())
        .filter(|p| p.props.style.as_deref().is_some_and(|st| st.starts_with("TOC") && st != "TOCHeading"))
        .map(|p| p.text.split('\t').next().unwrap_or("").trim_matches(wordcraft_doc::para::OBJ).to_string())
        .filter(|t| !t.is_empty())
        .collect()
}

#[test]
fn an_odt_table_of_contents_updates_from_the_headings() {
    for titled in [true, false] {
        let doc = io::open_bytes("toc.odt", &odt(titled)).unwrap();
        let mut s = Session::new(doc);
        assert_eq!(entries(&s), ["Old chapter"], "titled: {titled}");
        s.run("references.updateToc", &json!({})).unwrap();
        // Two levels, as the file's source says.
        assert_eq!(entries(&s), ["Planning", "Budget"], "titled: {titled}");
        // Saved and opened again, it is still a table of contents.
        let again = io::open_bytes("toc.odt", &io::save_bytes("toc.odt", &s.doc).unwrap()).unwrap();
        let mut s2 = Session::new(again);
        assert_eq!(entries(&s2), ["Planning", "Budget"], "titled: {titled}");
        s2.run("references.updateToc", &json!({})).unwrap();
        assert_eq!(entries(&s2), ["Planning", "Budget"], "titled: {titled}");
    }
}

#[test]
fn an_odt_index_updates_from_its_marks() {
    // An alphabetical index whose entries are index marks in the text (`XE` fields once open):
    // Update Index lists them, rather than emptying the index.
    let content = format!(
        r#"<office:document-content {NS}><office:body><office:text>
        <text:alphabetical-index text:name="A"><text:alphabetical-index-source/><text:index-body><text:p>stale, 9</text:p></text:index-body></text:alphabetical-index>
        <text:p>Clay <text:alphabetical-index-mark text:string-value="kilns"/>is fired; <text:alphabetical-index-mark-start text:id="m"/>glazes<text:alphabetical-index-mark-end text:id="m"/> shine.</text:p>
        </office:text></office:body></office:document-content>"#
    );
    let mut z = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let o = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    for (name, data) in [("mimetype", "application/vnd.oasis.opendocument.text"), ("content.xml", content.as_str())] {
        z.start_file(name, o).unwrap();
        z.write_all(data.as_bytes()).unwrap();
    }
    let mut s = Session::new(io::open_bytes("index.odt", &z.finish().unwrap().into_inner()).unwrap());
    s.run("references.updateIndex", &json!({})).unwrap();
    let text: Vec<String> = s.doc.body.iter().filter_map(|b| b.as_para()).map(|p| p.text.replace(wordcraft_doc::para::OBJ, "")).collect();
    assert!(text.iter().any(|t| t.starts_with("glazes")), "{text:?}");
    assert!(text.iter().any(|t| t.starts_with("kilns")), "{text:?}");
    assert!(!text.iter().any(|t| t.contains("stale")), "{text:?}");
}

#[test]
fn an_odt_bibliography_updates_from_its_citations_sources() {
    // A bibliography and a citation carrying its source: Update lists the source.
    let content = format!(
        r#"<office:document-content {NS}><office:body><office:text>
        <text:p>Clay <text:bibliography-mark text:identifier="Smi20" text:bibliography-type="book" text:author="Smith, Jane" text:title="Kilns and Fire" text:year="2020">[Smi20]</text:bibliography-mark>.</text:p>
        <text:bibliography text:name="B"><text:bibliography-source/><text:index-body><text:p>stale</text:p></text:index-body></text:bibliography>
        </office:text></office:body></office:document-content>"#
    );
    let mut z = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let o = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    for (name, data) in [("mimetype", "application/vnd.oasis.opendocument.text"), ("content.xml", content.as_str())] {
        z.start_file(name, o).unwrap();
        z.write_all(data.as_bytes()).unwrap();
    }
    let mut s = Session::new(io::open_bytes("bib.odt", &z.finish().unwrap().into_inner()).unwrap());
    s.run("references.updateIndex", &json!({})).unwrap();
    let text: Vec<String> = s.doc.body.iter().filter_map(|b| b.as_para()).map(|p| p.text.replace(wordcraft_doc::para::OBJ, "")).collect();
    assert!(text.iter().any(|t| t.contains("Kilns and Fire")), "{text:?}");
    assert!(!text.iter().any(|t| t.contains("stale")), "{text:?}");
}
