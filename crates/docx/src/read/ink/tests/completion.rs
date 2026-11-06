use super::super::{format::Format, mapping::Mapping, units};
use super::{GraphicItem, PathSeg, Rgb};
use std::collections::HashMap;

fn ink(body: &str) -> String {
    format!(r#"<ink xmlns="http://www.w3.org/2003/InkML">{body}</ink>"#)
}
fn read(xml: &str, work: &mut usize) -> Option<wordcraft_doc::graphic::Graphic> {
    super::super::read(xml.as_bytes(), work)
}
fn paths(items: &[GraphicItem]) -> Vec<(&Vec<PathSeg>, f32)> {
    items
        .iter()
        .filter_map(|item| match item {
            GraphicItem::Path { segs, stroke_width, .. } => Some((segs, *stroke_width)),
            _ => None,
        })
        .collect()
}

fn mapped(mapping: &str, samples: &str) -> String {
    ink(&format!(
        r#"<context><canvas><traceFormat><channel name="X"/><channel name="Y"/></traceFormat></canvas><canvasTransform>{mapping}</canvasTransform></context><trace>{samples}</trace>"#
    ))
}

fn inverse(mapping: &str, samples: &str) -> Option<wordcraft_doc::graphic::Graphic> {
    read(&mapped(&format!(r#"<mapping type="unknown"/>{mapping}"#), samples), &mut 1_000_000)
}

fn formula(expression: &str) -> Option<f64> {
    let xml = ink(&format!(
        r#"<mapping xmlns:m="http://www.w3.org/1998/Math/MathML" type="mathml"><bind source="X" variable="x"/><bind target="Y" variable="result"/><m:math>{expression}</m:math></mapping>"#
    ));
    let root = crate::xml::parse_strict(xml.as_bytes()).ok()?;
    let format = Format::default();
    let map = Mapping::read(root.child("ink:mapping")?, &format, &format, &HashMap::new(), &mut 100_000, 0)?;
    let mut output = vec![None; 2];
    map.apply(&vec![Some(1.0), Some(9.0)], &mut output, &mut 100_000)?;
    output.get(1).copied().flatten()
}

#[test]
fn compound_units_are_dimension_checked_and_bounded() {
    for unit in ["mm", "mm*mm/mm", "(kg*m/(s*s))/(kg/(s*s))*mm/m", "1/(1/mm)"] {
        assert!((units::length(Some(unit)).unwrap() - 72.0 / 25.4).abs() < 1e-9, "{unit}");
    }
    assert!((units::resolution(254.0, "1/(mm*mm/mm)", units::LENGTH).unwrap() - 72.0 / 6451.6).abs() < 1e-9);
    for unit in ["mm/s", "mm**mm", "1/", "(mm", "mm)", "mm/0", "nan", "mml:pi", "%", "dev", "mm mm", "em*mm/mm"] {
        assert!(units::length(Some(unit)).is_none(), "{unit}");
    }
    assert!(units::parse(&format!("{}mm{}", "(".repeat(17), ")".repeat(17))).is_none());
    assert!(units::parse(&"mm/".repeat(100)).is_none());
    let xml = ink(r#"<traceFormat><channel name="X" units="mm*mm/mm"/><channel name="Y" units="mm"/></traceFormat><trace>0 0, 25.4 25.4</trace>"#);
    let g = read(&xml, &mut 1_000_000).unwrap();
    assert_eq!((g.w, g.h), (73.0, 73.0));
}

#[test]
fn percentage_lengths_use_explicit_ranges_not_absolute_sample_limits() {
    let xml = ink(
        r#"<traceFormat><channel name="X" units="%" min="20" max="120"/><channel name="Y" units="%" min="-50" max="50" orientation="-ve"/></traceFormat><trace>0 0, .5 .5, 1 1</trace>"#,
    );
    let g = read(&xml, &mut 1_000_000).unwrap();
    assert_eq!((g.w, g.h), (101.0, 101.0));
    assert!(
        matches!(paths(&g.items).as_slice(),[(p,_)] if matches!(p.as_slice(),[PathSeg::Move(0.5,100.5),PathSeg::Line(50.5,50.5),PathSeg::Line(100.5,0.5)]))
    );
    assert!(read(&xml.replace("1 1", "1.1 1"), &mut 1_000_000).is_none());
    assert!(read(&xml.replace(" max=\"120\"", ""), &mut 1_000_000).is_none());
}

#[test]
fn percentage_rotation_and_device_angle_resolutions_rotate_nibs() {
    let brush = r#"<brush><brushProperty name="tip" value="rectangle"/><brushProperty name="width" value="8" units="pt"/><brushProperty name="height" value="4" units="pt"/></brush>"#;
    for context in [
        r#"<traceFormat><channel name="X"/><channel name="Y"/><channel name="OR" units="%" min="0" max="180"/></traceFormat>"#,
        r#"<context><inkSource><traceFormat><channel name="X"/><channel name="Y"/><channel name="OR" units="dev"/></traceFormat><channelProperties><channelProperty channel="OR" name="resolution" value="90" units="deg*deg/deg"/></channelProperties></inkSource></context>"#,
    ] {
        let sample = if context.contains("inkSource") { "1" } else { ".5" };
        let g = read(&ink(&format!("{context}{brush}<trace>0 0 {sample}</trace>")), &mut 1_000_000).unwrap();
        assert!((g.w - 4.0).abs() < 1e-5 && (g.h - 8.0).abs() < 1e-5);
    }
}

#[test]
fn inverse_only_affine_transforms_are_solved_with_offsets_and_pivoting() {
    for (map, samples) in [
        (r#"<mapping type="affine"><affine>2 0 10, 0 3 -6</affine></mapping>"#, "10 -6, 30 24"),
        (r#"<mapping type="affine"><affine>0 2 10, -3 0 -6</affine></mapping>"#, "10 -6, 30 -36"),
    ] {
        let g = inverse(map, samples).unwrap();
        assert_eq!((g.w, g.h), (11.0, 11.0));
        assert_eq!(paths(&g.items)[0].0.len(), 2);
    }
}

#[test]
fn inverse_affine_product_handles_cross_coupled_channel_bindings() {
    let map = r#"<mapping type="product"><mapping type="affine"><bind source="X" column="1"/><bind source="Y" column="2"/><bind target="X" column="1"/><affine>2 1 0</affine></mapping><mapping type="affine"><bind source="X" column="1"/><bind source="Y" column="2"/><bind target="Y" column="1"/><affine>1 -1 0</affine></mapping></mapping>"#;
    let g = inverse(map, "0 0, 6 0").unwrap();
    assert_eq!((g.w, g.h), (3.0, 3.0));
}

#[test]
fn inverse_monotone_tables_support_both_directions_and_products() {
    for rows in ["0 10,10 30", "10 10,0 30"] {
        let map = format!(
            r#"<mapping type="product"><mapping type="table"><bind source="X" column="1"/><bind target="X" column="2"/><table interpolation="linear">{rows}</table></mapping><mapping type="affine"><bind source="Y" column="1"/><bind target="Y" column="1"/><affine>2 0</affine></mapping></mapping>"#
        );
        let g = inverse(&map, "10 0,30 20").unwrap();
        assert_eq!((g.w, g.h), (11.0, 11.0));
    }
}

#[test]
fn inverse_unknown_references_resolve_locally_and_forward_maps_take_precedence() {
    let xml = ink(
        r##"<definitions><mapping xml:id="unknown" type="unknown"/><mapping xml:id="reverse" type="affine"><affine>2 0 0,0 2 0</affine></mapping></definitions><context><canvasTransform><mapping mappingRef="#unknown"/><mapping mappingRef="#reverse"/></canvasTransform></context><trace>0 0,20 20</trace>"##,
    );
    assert_eq!(read(&xml, &mut 1_000_000).unwrap().w, 11.0);
    let xml = mapped(r#"<mapping type="identity"/><mapping type="affine"><affine>0 0 0,0 0 0</affine></mapping>"#, "0 0,20 20");
    assert_eq!(read(&xml, &mut 1_000_000).unwrap().w, 21.0);
}

#[test]
fn noninvertible_or_unbounded_reverse_maps_retain_the_fallback() {
    for map in [
        r#"<mapping type="unknown"/>"#,
        r#"<mapping type="affine"><affine>1 1 0,2 2 0</affine></mapping>"#,
        r#"<mapping type="affine"><affine>1e-12 0 0,0 1 0</affine></mapping>"#,
        r#"<mapping type="table"><bind source="X" column="1"/><bind target="X" column="2"/><table interpolation="linear">0 0,1 2,2 0</table></mapping>"#,
        r#"<mapping type="table"><bind source="X" column="1"/><bind target="X" column="2"/><table interpolation="floor">0 0,1 2</table></mapping>"#,
        r#"<mapping mappingRef="https://example.invalid/reverse"/>"#,
    ] {
        assert!(inverse(map, "0 0,10 10").is_none(), "{map}");
    }
    assert!(read(&mapped(r#"<mapping type="unknown"/><mapping type="identity"/><mapping type="identity"/>"#, "0 0"), &mut 1_000_000).is_none());
}

#[test]
fn rational_mathml_literals_and_nary_relations_are_evaluated() {
    for expression in [r#"<m:cn type="rational">1<m:sep/>2</m:cn>"#, r#"<m:cn type="rational">1/2</m:cn>"#, r#"<m:cn type="rational">1 2</m:cn>"#] {
        assert_eq!(formula(expression), Some(0.5));
    }
    for (expression, value) in [
        ("<m:apply><m:plus/></m:apply>", 0.0),
        ("<m:apply><m:times/></m:apply>", 1.0),
        ("<m:apply><m:and/></m:apply>", 1.0),
        ("<m:apply><m:or/></m:apply>", 0.0),
        ("<m:apply><m:lt/><m:cn>1</m:cn><m:cn>2</m:cn><m:cn>3</m:cn></m:apply>", 1.0),
        ("<m:apply><m:gt/><m:cn>3</m:cn><m:cn>1</m:cn><m:cn>2</m:cn></m:apply>", 0.0),
        ("<m:apply><m:eq/><m:cn>1</m:cn><m:cn>1</m:cn><m:cn>1</m:cn></m:apply>", 1.0),
    ] {
        assert_eq!(formula(expression), Some(value), "{expression}");
    }
    for expression in [
        r#"<m:cn type="rational">1<m:sep/>0</m:cn>"#,
        r#"<m:cn type="rational">1<m:sep/><m:sep/>2</m:cn>"#,
        r#"<m:cn type="rational">1.5 2</m:cn>"#,
        "<m:apply><m:divide/><m:cn>0</m:cn><m:cn>0</m:cn></m:apply>",
        "<m:apply><m:lt/><m:cn>1</m:cn></m:apply>",
        "<m:apply><m:sin/></m:apply>",
    ] {
        assert!(formula(expression).is_none(), "{expression}");
    }
}

#[test]
fn mathml_output_variables_need_not_exist_in_the_input_format() {
    let xml = ink(
        r#"<context><traceFormat><channel name="a"/><channel name="b"/></traceFormat><canvasTransform><mapping type="product"><mapping type="mathml" xmlns:m="http://www.w3.org/1998/Math/MathML"><bind source="a" variable="a"/><bind target="X" variable="out"/><m:math><m:ci>a</m:ci></m:math></mapping><mapping type="affine"><bind source="b" column="1"/><bind target="Y" column="1"/><affine>1 0</affine></mapping></mapping></canvasTransform></context><trace>0 0,10 10</trace>"#,
    );
    let g = read(&xml, &mut 1_000_000).unwrap();
    assert_eq!((g.w, g.h), (11.0, 11.0));
    assert!(read(&xml.replace("<m:ci>a</m:ci>", "<m:ci>out</m:ci>"), &mut 1_000_000).is_none());
}

#[test]
fn referenced_mappings_can_rebind_variables_and_columns() {
    for definition in [
        r#"<mapping xml:id="m" type="affine"><bind source="X" column="1"/><bind target="X" column="1"/><affine>2 0</affine></mapping>"#,
        r#"<mapping xml:id="m" type="mathml" xmlns:m="http://www.w3.org/1998/Math/MathML"><bind source="X" variable="v"/><bind target="X"/><m:math><m:apply><m:times/><m:ci>v</m:ci><m:cn>2</m:cn></m:apply></m:math></mapping>"#,
        r#"<mapping xml:id="m" type="table"><bind source="X" column="1"/><bind target="X" column="2"/><table interpolation="linear">0 0,10 20</table></mapping>"#,
    ] {
        let target_column = if definition.contains("type=\"table\"") { 2 } else { 1 };
        let xml = ink(&format!(
            r##"<definitions>{definition}</definitions><mapping mappingRef="#m"><bind source="Y" column="1" variable="v"/><bind target="X" column="{target_column}"/></mapping>"##
        ));
        let root = crate::xml::parse_strict(xml.as_bytes()).unwrap();
        let ids = HashMap::from([("m", root.child("ink:definitions").unwrap().child("ink:mapping").unwrap())]);
        let f = Format::default();
        let m = Mapping::read(root.child("ink:mapping").unwrap(), &f, &f, &ids, &mut 100_000, 0).unwrap();
        let mut output = vec![None; 2];
        m.apply(&vec![Some(1.0), Some(3.0)], &mut output, &mut 100_000).unwrap();
        assert_eq!(output[0], Some(6.0));
    }
}

#[test]
fn cropped_views_join_only_adjacent_continuation_boundaries() {
    let definitions = r##"<definitions><traceGroup xml:id="g"><trace xml:id="a" continuation="begin">0 0,1 0,2 0</trace><trace xml:id="b" continuation="middle" priorRef="#a">3 0,4 0</trace><trace xml:id="c" continuation="end" priorRef="#b">5 0,6 0</trace></traceGroup></definitions>"##;
    let xml = ink(&format!(r##"{definitions}<traceView traceDataRef="#g" from="1:2" to="3:1"/>"##));
    let g = read(&xml, &mut 1_000_000).unwrap();
    assert_eq!((g.w, g.h), (5.0, 1.0));
    assert_eq!(paths(&g.items)[0].0.len(), 5);
    let xml =
        ink(&format!(r##"{definitions}<traceGroup><traceView traceDataRef="#a" to="1"/><traceView traceDataRef="#b" from="2"/></traceGroup>"##));
    let g = read(&xml, &mut 1_000_000).unwrap();
    assert_eq!(g.items.len(), 2);
    assert!(g.items.iter().all(|i| matches!(i, GraphicItem::Shape { .. })));
}

#[test]
fn view_continuation_state_is_isolated_from_the_live_stream() {
    let xml = ink(
        r##"<definitions><trace xml:id="view" continuation="begin">100 0,101 0</trace></definitions><trace xml:id="live" continuation="begin">0 0,1 0</trace><traceView traceDataRef="#view"/><trace continuation="end" priorRef="#live">2 0</trace>"##,
    );
    let g = read(&xml, &mut 1_000_000).unwrap();
    assert_eq!(paths(&g.items).iter().map(|(p, _)| p.len()).collect::<Vec<_>>(), [3, 2]);
    let cyclic = ink(
        r##"<definitions><trace xml:id="a" continuation="middle" priorRef="#b">0 0</trace><trace xml:id="b" continuation="middle" priorRef="#a">1 0</trace></definitions><traceView traceDataRef="#a"/>"##,
    );
    assert!(read(&cyclic, &mut 1_000_000).is_none());
}

#[test]
fn mask_pen_outlines_and_copy_pen_resets_keep_original_stroke_boundaries() {
    let xml = ink(
        r##"<brush><brushProperty name="rasterOp" value="maskPen"/><brushProperty name="color" value="#ffff00"/></brush><trace>0 0,10 0,10 10</trace><trace>0 0,10 10</trace><brush><brushProperty name="rasterOp" value="copyPen"/></brush><trace>0 10,10 10</trace>"##,
    );
    let g = read(&xml, &mut 1_000_000).unwrap();
    assert_eq!(g.items.len(), 3);
    for item in &g.items[..2] {
        assert!(matches!(item, GraphicItem::InkOutline { mask_pen: true, color: Rgb(255, 255, 0), .. }));
    }
    assert!(matches!(&g.items[2], GraphicItem::Path { .. }));
    assert!(read(&xml.replace("maskPen", "xorPen"), &mut 1_000_000).is_none());
    assert!(read(&xml.replace("#ffff00", "#ffee00"), &mut 1_000_000).is_none());
}

#[test]
fn all_new_compilation_and_expansion_work_is_charged() {
    let xml = mapped(r#"<mapping type="unknown"/><mapping type="affine"><affine>2 0 0,0 2 0</affine></mapping>"#, "0 0,10 10");
    let mut budget = 1_000_000;
    assert!(read(&xml, &mut budget).is_some());
    let used = 1_000_000 - budget;
    assert!(read(&xml, &mut (used - 1)).is_none());
    let mut exact = used;
    assert!(read(&xml, &mut exact).is_some());
}
