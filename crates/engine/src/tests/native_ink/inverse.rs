use super::super::{json, run};
use super::{DisplayOptions, INK, Session, blue_pixels, document_with_ink, page_display, strokes};

/// Supported inverse mappings match explicit coordinates across edits and repeated saves.
#[test]
fn inverse_native_ink_matches_explicit_coordinates_and_survives_edits_and_saves() {
    let maps = [
        (
            r#"<mapping type="mathml" xmlns:m="http://www.w3.org/1998/Math/MathML"><bind source="X" variable="x"/><bind target="X"/><m:math><m:apply><m:power/><m:ci>x</m:ci><m:cn>3</m:cn></m:apply></m:math></mapping>"#,
            "0 0,125000 0,125000 25",
            "0 0,50 0,50 25",
        ),
        (
            r#"<mapping type="table"><bind source="X" column="1"/><bind target="X" column="2"/><table interpolation="cubic">0 0,1 1,2 4,3 9</table></mapping>"#,
            ".35 0,2.2 0,6.35 1",
            ".5 0,1.5 0,2.5 1",
        ),
    ];
    for (reverse, recorded, explicit) in maps {
        let base = INK.replace("<trace>0 0, 50 0, 50 25</trace>", &format!("<trace>{explicit}</trace>"));
        let projected=INK.replace("<trace>0 0, 50 0, 50 25</trace>",&format!(r#"<context><canvasTransform><mapping type="unknown"/><mapping type="product">{reverse}<mapping type="affine"><bind source="Y" column="1"/><bind target="Y" column="1"/><affine>1 0</affine></mapping></mapping></canvasTransform></context><trace>{recorded}</trace>"#));
        for placement in ["inline", "floating", "group", "canvas", "direct"] {
            let doc = crate::io::open_bytes("explicit.docx", &document_with_ink(placement, &base)).unwrap();
            let mut reference = Session::new(doc);
            let layout = reference.layout();
            let expected = wordcraft_render::render_page(&reference.doc, &layout.pages[0], 1.0, &Default::default());
            let doc = crate::io::open_bytes("inverse.docx", &document_with_ink(placement, &projected)).unwrap();
            let mut session = Session::new(serde_json::from_slice(&serde_json::to_vec(&doc).unwrap()).unwrap());
            let before = session.doc.body.clone();
            run(&mut session, "text.insert", json!({"text":"Edit "}));
            run(&mut session, "edit.undo", json!({}));
            assert_eq!(session.doc.body, before);
            for _ in 0..3 {
                let layout = session.layout();
                let page = &layout.pages[0];
                assert_eq!(strokes(&page_display(&session.doc, page, &Default::default())), 1);
                let hidden = DisplayOptions { hide_ink: true, ..Default::default() };
                assert_eq!(strokes(&page_display(&session.doc, page, &hidden)), 0);
                let image = wordcraft_render::render_page(&session.doc, page, 1.0, &Default::default());
                assert!(blue_pixels(&image) > 5);
                assert_eq!(image.pixels, expected.pixels, "{placement}: inverse must render the same coordinates");
                let pdf = wordcraft_pdf::export_layout(&session.doc, &layout, &Default::default()).unwrap();
                assert!(!String::from_utf8_lossy(&pdf).contains("/Subtype /Image"));
                assert!(session.doc.passthrough.values().any(|bytes| bytes.as_slice() == projected.as_bytes()));
                let saved = crate::io::save_bytes("inverse.docx", &session.doc).unwrap();
                session = Session::new(crate::io::open_bytes("inverse.docx", &saved).unwrap());
            }
        }
    }
}
