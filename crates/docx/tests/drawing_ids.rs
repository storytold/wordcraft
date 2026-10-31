//! Drawing ids (`wp:docPr/@id`) stay distinct when a shape already uses the largest id.

use std::io::Read;

use wordcraft_doc::para::ShapeKind;
use wordcraft_doc::{Document, InlineObject, Pos};

fn shape(id: u32) -> InlineObject {
    InlineObject::Shape {
        kind: ShapeKind::TextBox,
        w: 50.0,
        h: 20.0,
        fill: None,
        stroke: None,
        stroke_width: 0.75,
        float: Default::default(),
        story: None,
        effects: Default::default(),
        freeform: None,
        extra: wordcraft_doc::connector::ShapeExtra { id, ..Default::default() },
    }
}

fn docpr_ids(bytes: &[u8]) -> Vec<String> {
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let mut xml = String::new();
    z.by_name("word/document.xml").unwrap().read_to_string(&mut xml).unwrap();
    xml.split("<wp:docPr id=\"").skip(1).map(|s| s.split('"').next().unwrap().to_string()).collect()
}

#[test]
fn fresh_docpr_ids_skip_used_ids_with_a_shape_at_the_largest_id() {
    let mut d = Document::from_text("a\nb\nc\nd\ne");
    // Own ids 4294967295, 1 and 3, then two shapes without one (as after gluing connectors).
    for (i, id) in [u32::MAX, 1, 3, 0, 0].into_iter().enumerate() {
        d.insert_object(&Pos::body(i, 1), shape(id), &Default::default()).unwrap();
    }
    let ids = docpr_ids(&wordcraft_docx::write(&d).unwrap());
    assert_eq!(ids.len(), 5, "{ids:?}");
    let mut sorted = ids.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(sorted.len(), 5, "duplicate docPr ids: {ids:?}");
}
