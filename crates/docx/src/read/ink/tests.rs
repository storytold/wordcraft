//! Unit coverage for the supported InkML subset and its hostile-input/resource limits.

use super::*;
use wordcraft_doc::para::ShapeKind;

mod advanced;
mod completion;
mod defaults;
mod discrete;
mod inverse;
mod xml;

/// Wrap InkML body markup with the real namespace and decode it under a fresh ample work budget.
fn ink(body: &str) -> Option<Graphic> {
    let xml = format!(r#"<ink xmlns="http://www.w3.org/2003/InkML">{body}</ink>"#);
    read(xml.as_bytes(), &mut 2_000_000)
}

/// Extract vector strokes for compact geometry, colour, and pen-width assertions.
fn paths(graphic: &Graphic) -> Vec<(&[PathSeg], Rgb, f32)> {
    graphic
        .items
        .iter()
        .map(|item| match item {
            GraphicItem::Path { segs, stroke: Some(color), stroke_width, .. } => (segs.as_slice(), *color, *stroke_width),
            _ => panic!("expected a stroke: {item:?}"),
        })
        .collect()
}

/// Equivalent explicit, first-difference, second-difference, and repeated-value data draw alike.
#[test]
fn absolute_differential_and_second_differential_traces_agree() {
    let absolute = ink("<trace>0 0, 10 20, 25 35, 45 45, 65 55</trace>").unwrap();
    for text in ["0 0,'10'20,\"5\"-5,5 -5,'20'10", "0 0, '10 '20, !25 !35, '20 '10, * *"] {
        assert_eq!(ink(&format!("<trace>{text}</trace>")), Some(absolute.clone()), "{text}");
    }
    let [(segs, color, width)] = paths(&absolute).try_into().unwrap();
    assert_eq!(color, Rgb::BLACK);
    assert_eq!(width, 1.0);
    assert_eq!(segs[0], PathSeg::Move(0.5, 0.5));
    assert_eq!(segs[4], PathSeg::Line(65.5, 55.5));
    assert_eq!((absolute.w, absolute.h), (66.0, 56.0));
}

/// Accept InkML's compact signed, fractional, hexadecimal, and exponent-valued tokens.
#[test]
fn compact_numbers_hex_exponents_trailing_comma_and_cdata() {
    let expected = ink("<trace>0.5 -2, 16 20, 3.25 -4.5</trace>").unwrap();
    let compact = ink("<trace><![CDATA[.5- 2,#10 2e1,3.25-4.5,]]></trace>").unwrap();
    assert_eq!(compact, expected);
}

/// Resolve chained contexts and brushes, reordered axes, negative orientation, and physical units.
#[test]
fn context_and_brush_references_reordered_channels_and_units() {
    let graphic = ink(r##"
        <definitions>
          <traceFormat xml:id="format"><channel name="T"/><channel name="Y" units="in" orientation="-ve"/><channel name="X" units="cm"/><channel name="F" units="dev"/></traceFormat>
          <context xml:id="parent" traceFormatRef="#format" brushRef="#pen"/>
          <context xml:id="child" contextRef="#parent"/>
          <brush xml:id="base"><brushProperty name="color" value="#2563EB"/><brushProperty name="width" value="0.1" units="in"/></brush>
          <brush xml:id="pen" brushRef="#base"><brushProperty name="color" value="#FF0000"/></brush>
        </definitions>
        <trace contextRef="#child">0 1 0 100, 1 0 2.54 200</trace>
    "##).unwrap();
    let [(segs, color, width)] = paths(&graphic).try_into().unwrap();
    assert_eq!(color, Rgb(255, 0, 0));
    assert!((width - 7.2).abs() < 0.001);
    assert!((graphic.w - 79.2).abs() < 0.001 && (graphic.h - 79.2).abs() < 0.001);
    assert!(
        matches!(segs, [PathSeg::Move(x, y), PathSeg::Line(a, b)] if (x - 3.6).abs() < 0.001 && (y - 3.6).abs() < 0.001 && (a - 75.6).abs() < 0.001 && (b - 75.6).abs() < 0.001)
    );
}

/// Inherit local sources and nested group brushes without joining separate traces or pen-up data.
#[test]
fn inline_ink_source_and_nested_groups_keep_separate_strokes() {
    let graphic = ink(r##"
        <context><inkSource><traceFormat><channel name="X" units="pt"/><channel name="Y" units="pt"/></traceFormat></inkSource></context>
        <definitions><brush xml:id="red"><brushProperty name="color" value="#FF0000"/></brush></definitions>
        <traceGroup brushRef="#red"><traceGroup><trace>0 0, 10 10</trace></traceGroup></traceGroup>
        <trace type="penUp">1000 1000, 2000 2000</trace>
        <trace>20 10, 30 0</trace>
    "##)
    .unwrap();
    let strokes = paths(&graphic);
    assert_eq!(strokes.len(), 2);
    assert_eq!(strokes[0].1, Rgb(255, 0, 0));
    assert_eq!(strokes[1].1, Rgb::BLACK);
    assert_eq!((graphic.w, graphic.h), (31.0, 11.0));
}

/// Resolve an explicit default-context reference using the InkML stream's current properties.
#[test]
fn default_context_reference_uses_the_current_stream_defaults() {
    let graphic = ink(r##"<brush><brushProperty name="color" value="#FF0000"/></brush><traceGroup brushRef="#DefaultBrush"><trace contextRef="#DefaultContext">0 0, 10 10</trace></traceGroup>"##).unwrap();
    assert_eq!(paths(&graphic)[0].1, Rgb(255, 0, 0));
}

/// Subtract the source-space origin in f64 so nearby large coordinates remain distinct.
#[test]
fn subtract_large_origins_before_converting_coordinates_to_f32() {
    let graphic = ink("<trace>999999998 999999998, 999999999 999999999</trace>").unwrap();
    assert_eq!((graphic.w, graphic.h), (2.0, 2.0));
    assert_eq!(paths(&graphic)[0].0, [PathSeg::Move(0.5, 0.5), PathSeg::Line(1.5, 1.5)]);
}

/// Represent a single-point pen tap as a visible filled dot in the correct brush colour.
#[test]
fn tap_is_a_visible_filled_circle() {
    let graphic = ink("<trace>123 -456</trace>").unwrap();
    assert_eq!((graphic.w, graphic.h), (1.0, 1.0));
    assert!(
        matches!(graphic.items.as_slice(), [GraphicItem::Shape { kind: ShapeKind::Ellipse, rect, fill: Some(Rgb::BLACK), stroke: None, .. }] if *rect == [0.0, 0.0, 1.0, 1.0])
    );
}

/// Match the InkML namespace independent of its prefix and refuse lookalikes or broken markup.
#[test]
fn namespace_aliases_work_but_lookalikes_and_malformed_xml_do_not() {
    let expected = ink("<trace>0 0, 1 1</trace>").unwrap();
    let alias = br#"<i:ink xmlns:i="http://www.w3.org/2003/InkML"><i:trace>0 0, 1 1</i:trace></i:ink>"#;
    assert_eq!(read(alias, &mut 2_000_000), Some(expected));
    for xml in [
        "<ink><trace>0 0, 1 1</trace></ink>",
        r#"<ink xmlns="urn:impostor"><trace>0 0, 1 1</trace></ink>"#,
        r#"<ink xmlns="http://www.w3.org/2003/InkML"><trace>0 0, 1 1</ink>"#,
        r#"<ink xmlns="http://www.w3.org/2003/InkML"><trace>0 0, 1 1</trace>"#,
    ] {
        assert!(read(xml.as_bytes(), &mut 2_000_000).is_none(), "{xml}");
    }
}

/// Refuse the complete vector preview rather than partially drawing unsupported visual semantics.
#[test]
fn unsupported_visual_semantics_refuse_the_whole_preview() {
    for body in [
        r##"<trace contextRef="https://example.invalid/ink#ctx">0 0, 1 1</trace>"##,
        r##"<trace contextRef="#missing">0 0, 1 1</trace>"##,
        r##"<context><canvasTransform/></context><trace>0 0, 1 1</trace>"##,
        r##"<context canvasRef="#other"/><trace>0 0, 1 1</trace>"##,
        r##"<brush><brushProperty name="rasterOp" value="xorPen"/></brush><trace>0 0, 1 1</trace>"##,
        r##"<brush><brushProperty name="tip" value="custom"/></brush><trace>0 0, 1 1</trace>"##,
        r##"<traceFormat><channel name="X" units="dev"/><channel name="Y" units="dev"/></traceFormat><trace>0 0, 1 1</trace>"##,
        r##"<traceFormat><channel name="X"/><channel name="Y"/><channel name="S"/></traceFormat><trace>0 0 T, 1 1 F</trace>"##,
        r##"<trace type="unknown">0 0, 1 1</trace>"##,
        r##"<trace continuation="middle" priorRef="#prior">0 0, 1 1</trace>"##,
        r##"<traceView traceDataRef="#earlier"/>"##,
        "<trace>0 0, 10 10</trace><trace>1 1, broken</trace>",
    ] {
        assert!(ink(body).is_none(), "{body}");
    }
}

/// Invalid tokens, duplicate IDs, missing references, and cyclic references fail without a preview.
#[test]
fn malformed_coordinates_and_reference_cycles_are_bounded() {
    for value in [
        "",
        "NaN 1",
        "inf 1",
        "1e999 0",
        "1 2 3",
        "1",
        "'1 0",
        "\"1 0",
        "* 0",
        "0 0,,1 1",
        "0 0,\"1\"1",
        "0 0,'1 '1,1e9 1",
        "#FFFFFFFF 0",
        "0 0,? ?",
        "é 1",
    ] {
        assert!(ink(&format!("<trace>{value}</trace>")).is_none(), "{value}");
    }
    for body in [
        r##"<definitions><context xml:id="a" contextRef="#b"/><context xml:id="b" contextRef="#a"/></definitions><trace contextRef="#a">0 0, 1 1</trace>"##,
        r##"<definitions><brush xml:id="a" brushRef="#a"/></definitions><trace brushRef="#a">0 0, 1 1</trace>"##,
        r##"<definitions><brush xml:id="a"/><brush xml:id="a"/></definitions><trace>0 0, 1 1</trace>"##,
    ] {
        assert!(ink(body).is_none());
    }
}

/// Enforce byte, element, nesting, sample, and shared work budgets without partial output.
#[test]
fn byte_node_depth_point_and_shared_work_limits_refuse_partial_drawings() {
    let mut work = usize::MAX;
    assert!(read(&vec![b' '; MAX_BYTES + 1], &mut work).is_none());
    assert!(ink(&format!("{}<trace>0 0, 1 1</trace>", "<annotation/>".repeat(MAX_NODES))).is_none());
    assert!(ink(&format!("{}<trace>0 0, 1 1</trace>{}", "<traceGroup>".repeat(MAX_DEPTH + 1), "</traceGroup>".repeat(MAX_DEPTH + 1))).is_none());
    let many = format!("<trace>{}</trace>", "0 0,".repeat(MAX_POINTS + 1));
    assert!(ink(&many).is_none());
    let xml = br#"<ink xmlns="http://www.w3.org/2003/InkML"><trace>0 0, 1 1</trace></ink>"#;
    let mut work = 1;
    assert!(read(xml, &mut work).is_none());
    assert_eq!(work, 0);
    let mut work = 2_000_000;
    assert!(read(xml, &mut work).is_some());
    assert!(work < 2_000_000);
    let spent = 2_000_000 - work;
    assert!(read(xml, &mut (spent - 1)).is_none());
}

/// Large but accepted coordinates are rescaled to finite drawing bounds and stroke widths.
#[test]
fn very_large_finite_coordinates_fit_a_bounded_graphic() {
    let graphic = ink("<trace>-1000000000 -1000000000, 1000000000 1000000000</trace>").unwrap();
    assert_eq!((graphic.w, graphic.h), (crate::units::MAX_LEN_PT, crate::units::MAX_LEN_PT));
    for (segs, _, width) in paths(&graphic) {
        assert!(width.is_finite() && width > 0.0);
        for seg in segs {
            let (PathSeg::Move(x, y) | PathSeg::Line(x, y)) = seg else { panic!() };
            assert!((0.0..=graphic.w).contains(x) && (0.0..=graphic.h).contains(y));
        }
    }
}

proptest::proptest! {
    /// Arbitrary short XML-escaped trace content never panics or bypasses the decode boundary.
    #[test]
    fn arbitrary_trace_text_never_panics(text in ".{0,2048}") {
        let _ = ink(&format!("<trace>{}</trace>", crate::xml::esc(&text)));
    }
}
