//! A ragged equation matrix doesn't make the saved `document.xml` grow with rows × widest row
//! (#3643): one wide row plus many empty rows used to be padded out to a full grid.

use wordcraft_doc::math::{MNode, MRun, Math};
use wordcraft_doc::para::InlineObject;
use wordcraft_doc::props::CharProps;
use wordcraft_doc::{Document, Paragraph, para_block};

#[test]
fn ragged_matrix_save_stays_small() {
    let n = 5000;
    let mut rows = vec![vec![vec![MNode::Run(MRun::new("a"))]; n]];
    rows.extend(std::iter::repeat_with(Vec::new).take(n));
    let math = Math { nodes: vec![MNode::Matrix { rows, col_jc: Vec::new() }], ..Default::default() };
    let mut p = Paragraph::new();
    p.insert_object(0, InlineObject::Equation { linear: String::new(), display: true, math }, &CharProps::default()).unwrap();
    let mut doc = Document::new();
    doc.body = vec![para_block(p)];
    let bytes = wordcraft_docx::write(&doc).unwrap();
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(&bytes)).unwrap();
    let size = zip.by_name("word/document.xml").unwrap().size();
    assert!(size < 1_000_000, "document.xml is {size} bytes");
}
