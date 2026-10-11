//! WordArt and connectors through the commands.

use serde_json::json;
use wordcraft_doc::connector::{ConnEnd, ShapeExtra, ends};
use wordcraft_doc::para::{Anchor, Float, InlineObject, ShapeKind, Wrap};
use wordcraft_doc::props::CharProps;
use wordcraft_doc::{Document, Paragraph, Pos, StoryRef, para_block};

use crate::Session;

fn run(s: &mut Session, id: &str, v: serde_json::Value) -> serde_json::Value {
    s.run(id, &v).unwrap_or_else(|e| panic!("{id}: {e}"))
}

/// A page-placed rectangle (id 1) at x 100 and ellipse (id 2) at x 300, 72 × 36 at y 200, and an
/// elbow connector from the rectangle's right side to the ellipse's left side, all in body
/// paragraph 0 after "Body " (objects at 5, 8, 11).
fn joined() -> Session {
    let float = |x: f32| Float { wrap: Wrap::InFrontOfText, h_rel: Anchor::Page, v_rel: Anchor::Page, x, y: 200.0, ..Default::default() };
    let shape = |kind, x, extra| InlineObject::Shape {
        kind,
        w: 72.0,
        h: 36.0,
        fill: None,
        stroke: None,
        stroke_width: 1.0,
        float: float(x),
        story: None,
        freeform: None,
        effects: Default::default(),
        extra,
    };
    let link = ShapeExtra { start: Some(ConnEnd { id: 1, site: 3 }), end: Some(ConnEnd { id: 2, site: 2 }), ..Default::default() };
    let mut p = Paragraph::with_text("Body ", CharProps::default());
    for o in [
        shape(ShapeKind::Rectangle, 100.0, ShapeExtra { id: 1, ..Default::default() }),
        shape(ShapeKind::Ellipse, 300.0, ShapeExtra { id: 2, ..Default::default() }),
        shape(ShapeKind::ElbowConnector, 0.0, link),
    ] {
        let end = p.len();
        p.insert_object(end, o, &CharProps::default()).unwrap();
    }
    let mut d = Document::new();
    d.body = vec![para_block(p)];
    Session::new(d)
}

fn select(s: &mut Session, off: usize) {
    run(s, "select.range", json!({"anchor": Pos::body(0, off), "focus": Pos::body(0, off + 3)}));
}

/// The connector's start and end on the page.
fn connector_ends(s: &mut Session) -> ((f32, f32), (f32, f32)) {
    let pos = Pos::body(0, 11);
    let hit = s.layout().object(&pos, 0).unwrap();
    let Some(InlineObject::Shape { float, .. }) = s.doc.para_at(&pos).and_then(|p| p.object_at(pos.off)) else { panic!() };
    let r = hit.rect;
    let (a, b) = ends(r.x, r.y, r.w, r.h, float.flip_h, float.flip_v);
    let round = |p: (f32, f32)| ((p.0 * 100.0).round() / 100.0, (p.1 * 100.0).round() / 100.0);
    (round(a), round(b))
}

#[test]
fn moving_a_joined_shape_reroutes_its_connector() {
    let mut s = joined();
    select(&mut s, 5);
    run(&mut s, "arrange.nudge", json!({"dx": 0, "dy": 30}));
    // From the rectangle's right side (172, 248) to the ellipse's left side (300, 218).
    assert_eq!(connector_ends(&mut s), ((172.0, 248.0), (300.0, 218.0)));
    // Resizing the ellipse moves the end with its site.
    select(&mut s, 8);
    run(&mut s, "arrange.bounds", json!({"width": 100, "height": 60}));
    assert_eq!(connector_ends(&mut s).1, (300.0, 230.0));
    // One undo takes the resize and the re-route back.
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(connector_ends(&mut s), ((172.0, 248.0), (300.0, 218.0)));
}

#[test]
fn dragging_a_connector_end_glues_it_to_the_nearest_site() {
    let mut s = joined();
    select(&mut s, 11);
    // Dropped near the ellipse's top: glued there.
    run(&mut s, "shape.connect", json!({"end": {"x": 340.0, "y": 195.0}}));
    let ext = |s: &Session| match s.doc.para_at(&Pos::body(0, 11)).and_then(|p| p.object_at(11)) {
        Some(InlineObject::Shape { extra, .. }) => extra.clone(),
        o => panic!("{o:?}"),
    };
    assert_eq!(ext(&s).end, Some(ConnEnd { id: 2, site: 0 }));
    assert_eq!(connector_ends(&mut s).1, (336.0, 200.0));
    // Dropped in the open: a free end there.
    run(&mut s, "shape.connect", json!({"start": {"x": 20.0, "y": 30.0}}));
    assert_eq!(ext(&s).start, None);
    assert_eq!(connector_ends(&mut s).0, (20.0, 30.0));
    // Glued by reference; a site the shape doesn't have is refused.
    run(&mut s, "shape.connect", json!({"start": {"path": [0], "off": 5, "site": 1}}));
    assert_eq!(connector_ends(&mut s).0, (100.0, 218.0));
    assert!(s.run("shape.connect", &json!({"start": {"path": [0], "off": 5, "site": 40}})).is_err());
    assert!(s.run("shape.connect", &json!({"start": {"path": [0], "off": 11}})).is_err(), "not to itself");
    // A connection to a shape that is gone is ignored.
    select(&mut s, 8);
    run(&mut s, "text.delete", json!({}));
    select(&mut s, 5);
    run(&mut s, "arrange.nudge", json!({"dx": 1}));
}

#[test]
fn wordart_is_inserted_styled_and_bent() {
    let mut s = Session::new(Document::new());
    let r = run(&mut s, "insert.wordArt", json!({"text": "Hello", "style": "gradient", "transform": "textArchUp"}));
    let id = r["story"].as_u64().unwrap() as u32;
    assert_eq!(s.doc.plain_text(StoryRef::Part(id)), "Hello");
    let props = s.doc.para_at(&s.doc.start_of(StoryRef::Part(id))).unwrap().props_at(0).clone();
    assert!(matches!(props.text_effects.as_deref().and_then(|f| f.fill.as_ref()), Some(wordcraft_doc::wordart::TextFill::Gradient { .. })));
    // Its text is drawn from outlines, bent.
    let layout = s.layout();
    let draws = wordcraft_layout::display::page_display(&s.doc, &layout.pages[0], &Default::default());
    assert!(draws.iter().any(|d| matches!(d, wordcraft_layout::display::Draw::Art { .. })));
    // The PDF draws the gradient and keeps the text.
    let pdf = wordcraft_pdf::export(&s.doc, &Default::default()).unwrap();
    assert!(pdf.starts_with(b"%PDF"));
    // Effects and transforms on the selected text box's text; hostile values are refused or capped.
    run(&mut s, "format.textEffects", json!({"outline": {"color": "C00000", "width": 1e9}, "glow": 5}));
    let props = s.doc.para_at(&s.doc.start_of(StoryRef::Part(id))).unwrap().props_at(0).clone();
    let fx = props.text_effects.unwrap();
    assert_eq!(fx.outline.map(|o| o.width), Some(wordcraft_doc::wordart::MAX_OUTLINE));
    assert!(fx.glow.is_some() && fx.fill.is_some());
    run(&mut s, "wordArt.transform", json!({"preset": "textWave1", "adj": 1e300}));
    assert!(s.run("wordArt.transform", &json!({"preset": "textBogus"})).is_err());
    assert!(s.run("format.textEffects", &json!({"style": "word"})).is_err());
    assert!(s.run("format.textEffects", &json!({"glow": "much"})).is_err());
    let warp = s.doc.para_at(&Pos::body(0, 0)).and_then(|p| p.object_at(0)).and_then(|o| match o {
        InlineObject::Shape { extra, .. } => extra.warp.clone(),
        _ => None,
    });
    assert_eq!(
        warp.map(|w| (w.adj_value("adj"), w.preset)).map(|(a, p)| (p, a)),
        Some(("textWave1".into(), Some(wordcraft_doc::wordart::MAX_ADJ_VALUE)))
    );
    run(&mut s, "wordArt.transform", json!({"preset": "none"}));
}
