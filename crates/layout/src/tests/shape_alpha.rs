use super::*;

#[test]
fn a_translucent_shape_fill_is_drawn_translucent() {
    let mut d = Document::from_text("Wash");
    let wash = InlineObject::Shape {
        kind: wordcraft_doc::para::ShapeKind::Rectangle,
        w: 100.0,
        h: 50.0,
        fill: Some(wordcraft_doc::fill::Fill::Solid { color: wordcraft_doc::props::Rgb::WHITE, transparency: 69.0 }),
        stroke: None,
        stroke_width: 0.0,
        float: Default::default(),
        story: None,
        freeform: None,
        effects: Default::default(),
        extra: Default::default(),
    };
    d.insert_object(&Pos::body(0, 0), wash, &Default::default()).unwrap();
    let draws = flat(display::page_display(&d, &lay(&d).pages[0], &Default::default()));
    let alpha =
        draws.iter().find_map(|x| if let display::Draw::Shape { fill: Some(display::Paint::Solid(_, alpha)), .. } = x { Some(*alpha) } else { None });
    assert!(alpha.is_some_and(|a| (a - 0.31).abs() < 0.001), "fill opacity {alpha:?}");
}
