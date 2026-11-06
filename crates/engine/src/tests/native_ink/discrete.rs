use super::super::{json, run};
use super::{DisplayOptions, INK, Session, blue_pixels, document_with_ink};

fn examples() -> Vec<(String, String)> {
    let xy = r#"<channel name="X" units="pt"/><channel name="Y" units="pt"/>"#;
    let control = format!(r#"<traceFormat>{xy}<channel name="S" type="boolean"/></traceFormat><trace>0 0 T,50 0 T,100 100 F,50 25 T,0 25 T</trace>"#);
    let coordinates = r#"<mapping type="affine"><bind source="X" column="1"/><bind source="Y" column="2"/><bind target="X" column="1"/><bind target="Y" column="2"/><affine>1 0 0,0 1 0</affine></mapping>"#;
    let forward = format!(
        r#"<traceFormat>{xy}<channel name="L" type="boolean"/></traceFormat><context><canvas><traceFormat>{xy}<channel name="S" type="boolean"/></traceFormat></canvas><canvasTransform><mapping type="product">{coordinates}<mapping type="table"><bind source="L" column="1"/><bind target="S" column="2"/><table>F T,T F</table></mapping></mapping></canvasTransform></context><trace>0 0 F,50 0 F,100 100 T,50 25 F,0 25 F</trace>"#
    );
    let reverse = format!(
        r#"<traceFormat><channel name="U"/><channel name="V"/><channel name="P" type="boolean"/><channel name="Q" type="boolean"/></traceFormat><context><canvas><traceFormat>{xy}<channel name="S" type="boolean"/><channel name="B1" type="boolean"/></traceFormat></canvas><canvasTransform><mapping type="unknown"/><mapping type="product"><mapping type="affine"><bind source="X" column="1"/><bind source="Y" column="2"/><bind target="U" column="1"/><bind target="V" column="2"/><affine>2 1 0,1 -1 0</affine></mapping><mapping type="mathml" xmlns:m="http://www.w3.org/1998/Math/MathML"><bind source="S" variable="s"/><bind source="B1" variable="b"/><bind target="P"/><m:math><m:apply><m:xor/><m:ci>s</m:ci><m:ci>b</m:ci></m:apply></m:math></mapping><mapping type="mathml" xmlns:m="http://www.w3.org/1998/Math/MathML"><bind source="B1" variable="b"/><bind target="Q"/><m:math><m:apply><m:not/><m:ci>b</m:ci></m:apply></m:math></mapping></mapping></canvasTransform></context><trace>0 0 T T,100 50 T T,300 0 T F,125 25 F F,25 -25 F F</trace>"#
    );
    let integers = r#"<channel name="X" type="integer" min="0" max="3" units="pt"/><channel name="Y" units="pt"/>"#;
    let integer_control = format!(r#"<traceFormat>{integers}</traceFormat><trace>0 0,3 0,3 2</trace>"#);
    let integer_reverse = format!(
        r#"<traceFormat><channel name="P" type="integer"/><channel name="Q" type="integer"/><channel name="Y" units="pt"/></traceFormat><context><canvas><traceFormat>{integers}</traceFormat></canvas><canvasTransform><mapping type="unknown"/><mapping type="product"><mapping type="affine"><bind source="Y" column="1"/><bind target="Y" column="1"/><affine>1 0</affine></mapping><mapping type="mathml" xmlns:m="http://www.w3.org/1998/Math/MathML"><bind source="X" variable="x"/><bind target="P"/><m:math><m:apply><m:quotient/><m:ci>x</m:ci><m:cn>2</m:cn></m:apply></m:math></mapping><mapping type="mathml" xmlns:m="http://www.w3.org/1998/Math/MathML"><bind source="X" variable="x"/><bind target="Q"/><m:math><m:apply><m:rem/><m:ci>x</m:ci><m:cn>2</m:cn></m:apply></m:math></mapping></mapping></canvasTransform></context><trace>0 0 0,1 1 0,1 1 2</trace>"#
    );
    let table_reverse = format!(
        r#"<context><canvas><traceFormat>{integers}</traceFormat></canvas><canvasTransform><mapping type="unknown"/><mapping type="product"><mapping type="affine"><bind source="Y" column="1"/><bind target="Y" column="1"/><affine>1 0</affine></mapping><mapping type="table"><bind source="X" column="1"/><bind target="X" column="2"/><table interpolation="floor">0 2,1 0,2 3,3 1</table></mapping></mapping></canvasTransform></context><trace>2 0,1 0,1 2</trace>"#
    );
    vec![(forward, control.clone()), (reverse, control), (integer_reverse, integer_control.clone()), (table_reverse, integer_control)]
}

#[test]
fn native_ink_discrete_maps_match_controls_and_preserve_contact_and_originals() {
    for (body, control) in examples() {
        let ink = INK.replace("<trace>0 0, 50 0, 50 25</trace>", &body);
        let explicit = INK.replace("<trace>0 0, 50 0, 50 25</trace>", &control);
        for placement in ["inline", "floating", "group", "canvas", "direct"] {
            let doc = crate::io::open_bytes("control.docx", &document_with_ink(placement, &explicit)).unwrap();
            let mut reference = Session::new(doc);
            let layout = reference.layout();
            let expected = wordcraft_render::render_page(&reference.doc, &layout.pages[0], 1.0, &Default::default());
            let doc = crate::io::open_bytes("discrete.docx", &document_with_ink(placement, &ink)).unwrap();
            let mut session = Session::new(serde_json::from_slice(&serde_json::to_vec(&doc).unwrap()).unwrap());
            let before = session.doc.body.clone();
            run(&mut session, "text.insert", json!({"text":"Edit "}));
            run(&mut session, "edit.undo", json!({}));
            assert_eq!(session.doc.body, before);
            for _ in 0..3 {
                let layout = session.layout();
                let page = &layout.pages[0];
                let image = wordcraft_render::render_page(&session.doc, page, 1.0, &Default::default());
                assert!(blue_pixels(&image) > 20);
                assert_eq!(image.pixels, expected.pixels, "{placement}: discrete projection must not invent geometry or pen-down bridges");
                let hidden = wordcraft_render::render_page(
                    &session.doc,
                    page,
                    1.0,
                    &wordcraft_render::RenderOptions { display: DisplayOptions { hide_ink: true, ..Default::default() }, ..Default::default() },
                );
                assert_eq!(blue_pixels(&hidden), 0);
                let pdf = wordcraft_pdf::export_layout(&session.doc, &layout, &Default::default()).unwrap();
                assert!(!String::from_utf8_lossy(&pdf).contains("/Subtype /Image"));
                assert!(session.doc.passthrough.values().any(|bytes| bytes.as_slice() == ink.as_bytes()));
                let saved = crate::io::save_bytes("discrete.docx", &session.doc).unwrap();
                session = Session::new(crate::io::open_bytes("discrete.docx", &saved).unwrap());
            }
        }
    }
}
