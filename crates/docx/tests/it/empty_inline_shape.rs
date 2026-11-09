//! An invisible inline shape (a `.doc` picture shape with no picture): saved as `wp:inline` +
//! `wps:wsp` with `a:noFill` and `a:ln/a:noFill` (ECMA-376 §20.4.2.8, §20.1.2.2.24), with no
//! image part and no relationship, and it reopens at its size, still invisible.
use std::io::Read;

use wordcraft_doc::{Document, InlineObject, Pos};

fn empty_shape() -> InlineObject {
    InlineObject::Shape {
        kind: Default::default(),
        w: 453.55,
        h: 152.0,
        fill: None,
        stroke: None,
        stroke_width: 0.0,
        float: Default::default(),
        story: None,
        freeform: None,
        effects: Default::default(),
        extra: Default::default(),
    }
}

fn parts(bytes: &[u8]) -> Vec<(String, String)> {
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    (0..zip.len())
        .map(|i| {
            let mut f = zip.by_index(i).unwrap();
            let mut s = String::new();
            let _ = f.read_to_string(&mut s);
            (f.name().to_string(), s)
        })
        .collect()
}

fn rels(parts: &[(String, String)]) -> usize {
    parts.iter().find(|(n, _)| n == "word/_rels/document.xml.rels").map_or(0, |(_, r)| r.matches("<Relationship ").count())
}

#[test]
fn an_empty_inline_shape_round_trips_invisible_at_its_size() {
    let mut doc = Document::new();
    let plain = parts(&wordcraft_docx::write(&doc).unwrap());
    doc.insert_object(&Pos::default(), empty_shape(), &Default::default()).unwrap();
    let bytes = wordcraft_docx::write(&doc).unwrap();
    let saved = parts(&bytes);
    assert!(saved.iter().all(|(n, _)| !n.starts_with("word/media/")), "no image part");
    assert_eq!(rels(&saved), rels(&plain), "no relationship added");
    let (_, xml) = saved.iter().find(|(n, _)| n == "word/document.xml").unwrap();
    assert!(xml.contains("<wp:inline") && xml.contains("<wps:wsp"), "{xml}");
    assert!(xml.contains("<a:noFill/></a:ln>") || xml.contains("<a:ln><a:noFill/>"), "{xml}");

    let back = wordcraft_docx::read(&bytes).unwrap();
    let p = back.body.first().and_then(|b| b.as_para()).unwrap();
    let shapes: Vec<_> = p.objects.iter().filter(|o| matches!(o, InlineObject::Shape { .. })).collect();
    assert_eq!(shapes.len(), 1);
    let InlineObject::Shape { w, h, fill, stroke, float, .. } = shapes[0] else { unreachable!() };
    assert!((w - 453.55).abs() <= 0.05 && (h - 152.0).abs() <= 0.05, "{w} x {h}");
    assert_eq!((fill, stroke), (&None, &None), "still invisible");
    assert_eq!(float.wrap, wordcraft_doc::para::Wrap::Inline);
    assert!(back.media.is_empty());
}
