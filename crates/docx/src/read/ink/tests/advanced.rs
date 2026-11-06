//! Specification-shaped synthetic cases for expanded InkML decoding and safe fallback boundaries.

use super::*;

/// Compare finite values after f64 source conversion and f32 model storage.
fn near(actual: f32, expected: f32) {
    assert!((actual - expected).abs() < 0.002, "{actual} != {expected}");
}

/// Build an XY canvas transform around synthetic trace content.
fn mapped(mapping: &str, trace: &str) -> Option<Graphic> {
    ink(&format!("<context><canvasTransform>{mapping}</canvasTransform></context><trace>{trace}</trace>"))
}

/// Intermittent values inherit defaults and prior integration state; unknowns do not corrupt peers.
#[test]
fn intermittent_defaults_booleans_and_unknown_values() {
    let body = r#"<traceFormat><channel name="X" units="pt"/><channel name="Y" units="pt"/><intermittentChannels><channel name="B1" type="boolean" default="F"/><channel name="custom" default="7"/></intermittentChannels></traceFormat><trace>0 0, 1 1 T, 2 2 ? 9, 3 3, 4 4 * ?</trace>"#;
    let graphic = ink(body).unwrap();
    assert_eq!(graphic, ink("<trace>0 0,1 1,2 2,3 3,4 4</trace>").unwrap());
    let root=crate::xml::parse_strict(br#"<traceFormat xmlns="http://www.w3.org/2003/InkML"><channel name="X"/><channel name="Y"/><intermittentChannels><channel name="V" default="2"/></intermittentChannels></traceFormat>"#).unwrap();
    let format = format::read(&root, &mut 10_000).unwrap();
    let values = trace::read("0 0,1 1 '3,2 2,3 3 ?", &format, &mut 1000, &mut 0).unwrap();
    assert_eq!(values.iter().map(|v| v[2]).collect::<Vec<_>>(), [Some(2.0), Some(5.0), Some(8.0), None]);
}

/// Tip-switch samples override both penUp and penDown trace types without joining lifted spans.
#[test]
fn tip_switch_overrides_contact_type_and_breaks_runs() {
    for kind in ["penDown", "penUp", "indeterminate"] {
        let graphic=ink(&format!(r#"<traceFormat><channel name="X" units="pt"/><channel name="Y" units="pt"/><channel name="S" type="boolean"/></traceFormat><trace type="{kind}">0 0 T,1 0 T,100 100 F,10 0 T,11 0 T</trace>"#)).unwrap();
        assert_eq!(paths(&graphic).len(), 2);
        near(graphic.w, 12.0);
        near(graphic.h, 1.0);
    }
}

/// Device resolution supports both units-per-quantum and quanta-per-unit, including negative axes.
#[test]
fn device_resolution_converts_to_points() {
    for (value, units) in [("1000", "1/in"), ("0.001", "in")] {
        let graphic=ink(&format!(r#"<context><inkSource><traceFormat><channel name="X" units="dev"/><channel name="Y" units="dev" orientation="-ve"/></traceFormat><channelProperties><channelProperty channel="X" name="resolution" value="{value}" units="{units}"/><channelProperty channel="Y" name="resolution" value="{value}" units="{units}"/></channelProperties></inkSource></context><trace>0 0,1000 -1000</trace>"#)).unwrap();
        assert_eq!(graphic, ink("<trace>0 0,72 72</trace>").unwrap());
    }
}

/// Definition contexts use declaration defaults; invalid overridden property references are ignored.
#[test]
fn context_declaration_defaults_and_child_precedence() {
    let graphic=ink(r##"<definitions><context xml:id="empty"/></definitions><brush><brushProperty name="color" value="#FF0000"/></brush><trace contextRef="#empty">0 0,1 1</trace>"##).unwrap();
    assert_eq!(paths(&graphic)[0].1, Rgb::BLACK);
    let graphic=ink(r##"<context traceFormatRef="#missing" brushRef="https://example.invalid/#pen" canvasRef="#bad" canvasTransformRef="#bad"><traceFormat><channel name="X" units="pt"/><channel name="Y" units="pt"/></traceFormat><brush/><canvas><traceFormat><channel name="X" units="pt"/><channel name="Y" units="pt"/></traceFormat></canvas><canvasTransform><mapping type="identity"/></canvasTransform></context><trace>0 0,1 1</trace>"##).unwrap();
    assert_eq!(graphic, ink("<trace>0 0,1 1</trace>").unwrap());
}

/// Continuation fragments restart their differential states but share a single visible stroke.
#[test]
fn continuations_connect_without_reusing_channel_velocity() {
    let graphic=ink(r##"<trace xml:id="a" continuation="begin">0 0,'10 '0</trace><trace xml:id="b" continuation="middle" priorRef="#a">20 0,'10 '0</trace><trace continuation="end" priorRef="#b">40 0,'10 '0</trace>"##).unwrap();
    assert_eq!(graphic, ink("<trace>0 0,10 0,20 0,30 0,40 0,50 0</trace>").unwrap());
    for body in [
        r##"<trace xml:id="a" continuation="begin">0 0,1 1</trace><trace continuation="end" priorRef="#a">'1 '1</trace>"##,
        r##"<trace xml:id="a" continuation="begin">0 0,1 1</trace><trace continuation="end" priorRef="#a">2 2</trace><trace continuation="end" priorRef="#a">3 3</trace>"##,
        r##"<trace xml:id="a">0 0,1 1</trace><trace continuation="end" priorRef="#a">2 2</trace>"##,
    ] {
        assert!(ink(body).is_none(), "{body}");
    }
}

/// Affine transforms honor explicit bindings, local references, matrix dimensions and output units.
#[test]
fn affine_canvas_mapping_and_explicit_channel_bindings() {
    let graphic = mapped(
        r#"<mapping type="affine"><bind source="X" column="1"/><bind source="Y" column="2"/><affine>0 -1 0,1 0 200</affine></mapping>"#,
        "0 0,10 20",
    )
    .unwrap();
    assert_eq!(graphic, ink("<trace>0 200,-20 210</trace>").unwrap());
    let graphic=ink(r##"<definitions><mapping xml:id="map" type="affine"><affine>2 0 0,0 3 0</affine></mapping></definitions><context><canvas><traceFormat><channel name="X" units="in"/><channel name="Y" units="in"/></traceFormat></canvas><canvasTransform><mapping mappingRef="#map"/></canvasTransform></context><trace>0 0,1 1</trace>"##).unwrap();
    near(graphic.w, 145.0);
    near(graphic.h, 217.0);
}

/// Mapped visual channels participate in rendering, not just the X/Y outputs.
#[test]
fn canvas_mapping_can_provide_width_and_color_channels() {
    let graphic=ink(r#"<context><canvas><traceFormat><channel name="X" units="pt"/><channel name="Y" units="pt"/><channel name="BW" units="pt"/><channel name="C"/></traceFormat></canvas><canvasTransform><mapping type="affine"><affine>1 0 0,0 1 0,0 0 4,0 0 #FF0000</affine></mapping></canvasTransform></context><trace>0 0,10 10</trace>"#).unwrap();
    assert_eq!(paths(&graphic)[0].1, Rgb(255, 0, 0));
    near(paths(&graphic)[0].2, 4.0);
    near(graphic.w, 14.0);
}

/// Product formulas can transform a custom polar source into the default Cartesian canvas.
#[test]
fn product_mathml_mapping_supports_polar_coordinates() {
    let graphic=ink(r#"<context><traceFormat><channel name="R"/><channel name="Theta"/></traceFormat><canvasTransform><mapping type="product"><mapping type="mathml"><bind target="X"/><bind source="R" variable="r"/><bind source="Theta" variable="t"/><math xmlns="http://www.w3.org/1998/Math/MathML"><apply><times/><ci>r</ci><apply><cos/><ci>t</ci></apply></apply></math></mapping><mapping type="mathml"><bind target="Y"/><bind source="R" variable="r"/><bind source="Theta" variable="t"/><math xmlns="http://www.w3.org/1998/Math/MathML"><apply><times/><ci>r</ci><apply><sin/><ci>t</ci></apply></apply></math></mapping></mapping></canvasTransform></context><trace>0 0,10 0,10 1.5707963267948966</trace>"#).unwrap();
    near(graphic.w, 11.0);
    near(graphic.h, 11.0);
    assert_eq!(paths(&graphic)[0].0.len(), 3);
}

/// Scalar MathML operators enforce arity/domains and consume bounded work.
#[test]
fn mathml_operator_domains_and_qualifiers() {
    for (expression, expected) in [
        ("<apply><root/><degree><cn>3</cn></degree><cn>27</cn></apply>", 3.0),
        ("<apply><log/><logbase><cn>2</cn></logbase><cn>8</cn></apply>", 3.0),
        ("<apply><minus/><cn>5</cn></apply>", -5.0),
        ("<apply><xor/><true/><false/></apply>", 1.0),
    ] {
        let xml = format!(
            r#"<mapping xmlns="http://www.w3.org/2003/InkML" type="mathml"><bind target="X"/><math xmlns="http://www.w3.org/1998/Math/MathML">{expression}</math></mapping>"#
        );
        let root = crate::xml::parse_strict(xml.as_bytes()).unwrap();
        let map = mapping::Mapping::read(&root, &Format::default(), &Format::default(), &HashMap::new(), &mut 1000, 0).unwrap();
        let mut output = vec![None; 2];
        map.apply(&vec![Some(0.0); 2], &mut output, &mut 1000).unwrap();
        assert_eq!(output[0], Some(expected));
    }
    for expression in
        ["<apply><divide/><cn>1</cn><cn>0</cn></apply>", "<apply><arcsin/><cn>7</cn></apply>", "<apply><sin/><cn>1</cn><cn>2</cn></apply>"]
    {
        let mapping = format!(
            r#"<mapping type="product"><mapping type="mathml"><bind target="X"/><math xmlns="http://www.w3.org/1998/Math/MathML">{expression}</math></mapping><mapping type="mathml"><bind target="Y"/><math xmlns="http://www.w3.org/1998/Math/MathML"><cn>0</cn></math></mapping></mapping>"#
        );
        assert!(mapped(&mapping, "0 0,1 1").is_none());
    }
}

/// Numerical tables use complete grids and respect floor/middle/ceiling/linear interpolation.
#[test]
fn table_interpolation_policies() {
    for (policy, expected) in [("floor", 0.0), ("middle", 10.0), ("ceiling", 10.0), ("linear", 7.5)] {
        let mapping = format!(
            r#"<mapping type="product"><mapping type="table"><bind source="X" column="1"/><bind target="X" column="2"/><table interpolation="{policy}">0 0,10 10</table></mapping><mapping type="mathml"><bind target="Y"/><math xmlns="http://www.w3.org/1998/Math/MathML"><cn>0</cn></math></mapping></mapping>"#
        );
        let graphic = mapped(&mapping, "0 0,7.5 0").unwrap();
        near(graphic.w, expected + 1.0);
    }
    let invalid = r#"<mapping type="table"><bind source="X" column="1"/><bind source="Y" column="2"/><bind target="X" column="3"/><table>0 0 0,1 0 1,0 1 1</table></mapping>"#;
    assert!(mapped(invalid, "0 0,1 1").is_none(), "incomplete Cartesian grid");
}

/// Cubic tables use a natural spline and stay bounded at their endpoints and outside their domain.
#[test]
fn cubic_table_interpolation_uses_natural_endpoints() {
    let map = r#"<mapping type="product"><mapping type="table"><bind source="X" column="1"/><bind target="X" column="2"/><table interpolation="cubic">0 0,1 1,2 0,3 1</table></mapping><mapping type="mathml"><bind target="Y"/><math xmlns="http://www.w3.org/1998/Math/MathML"><cn>0</cn></math></mapping></mapping>"#;
    near(mapped(map, "0 0,0.5 0").unwrap().w, 1.75);
    near(mapped(map, "0 0,1.5 0").unwrap().w, 1.5);
    near(mapped(map, "0 0,3 0").unwrap().w, 2.0);
    assert!(mapped(map, "0 0,3.001 0").is_none());
    assert!(mapped(&map.replace("2 0,3 1", "2 0"), "0 0,1 1").is_none());
}

/// Multiple source dimensions interpolate as a complete tensor grid, including relative outputs.
#[test]
fn multivariate_relative_table_interpolation() {
    let map = r#"<mapping type="product"><mapping type="table"><bind source="X" column="1"/><bind source="Y" column="2"/><bind target="X" column="3"/><table apply="relative">0 0 0,1 0 10,0 1 20,1 1 30</table></mapping><mapping type="mathml"><bind target="Y"/><math xmlns="http://www.w3.org/1998/Math/MathML"><cn>0</cn></math></mapping></mapping>"#;
    near(mapped(map, "0 0,0.25 0.5").unwrap().w, 13.75);
}

/// Bounds and invalid references reject a whole preview, including data beyond valid earlier traces.
#[test]
fn mapping_bounds_cycles_and_budget_do_not_produce_partial_vectors() {
    for body in [
        r##"<definitions><mapping xml:id="a" mappingRef="#a"/></definitions><context><canvasTransform><mapping mappingRef="#a"/></canvasTransform></context><trace>0 0,1 1</trace>"##,
        r#"<context><canvas><traceFormat><channel name="X" max="1"/><channel name="Y"/></traceFormat></canvas></context><trace>0 0,2 0</trace>"#,
        r#"<context><canvasTransform><mapping type="affine"><affine>1 0,0 1 0</affine></mapping></canvasTransform></context><trace>0 0,1 1</trace>"#,
        r#"<context><canvasTransform><mapping type="product"><mapping type="identity"/><mapping type="identity"/></mapping></canvasTransform></context><trace>0 0,1 1</trace>"#,
    ] {
        assert!(ink(body).is_none(), "{body}");
    }
    let xml=br#"<ink xmlns="http://www.w3.org/2003/InkML"><context><canvasTransform><mapping type="affine"><affine>1 0 0,0 1 0</affine></mapping></canvasTransform></context><trace>0 0,1 1</trace></ink>"#;
    let mut work = 2_000_000;
    assert!(read(xml, &mut work).is_some());
    assert!(read(xml, &mut (2_000_000 - work - 1)).is_none());
}

/// Views of definitions use 1-based nested ranges, including views of an already trimmed tree.
#[test]
fn nested_trace_views_select_definition_ranges() {
    let graphic=ink(r##"<definitions><traceGroup xml:id="g"><trace>0 0,1 0,2 0</trace><traceGroup><trace>10 0,11 0,12 0</trace><trace>20 0,21 0</trace></traceGroup></traceGroup><traceView xml:id="v" traceDataRef="#g" from="1:2" to="2:1:2"/></definitions><traceView traceDataRef="#v" from="1:2" to="2:1:1"/>"##).unwrap();
    assert_eq!(graphic.items.len(), 2);
    near(graphic.w, 9.0);
    for body in [
        r##"<traceView xml:id="a" traceDataRef="#a"/>"##,
        r##"<definitions><trace xml:id="a">0 0,1 1</trace></definitions><traceView traceDataRef="#a" from="0"/>"##,
        r##"<definitions><trace xml:id="a">0 0,1 1</trace></definitions><traceView traceDataRef="#a" from="1:1"/>"##,
        r##"<definitions><trace xml:id="a">0 0,1 1</trace></definitions><traceView traceDataRef="#a" from="3"/>"##,
    ] {
        assert!(ink(body).is_none(), "{body}");
    }
}

/// Recognition views of existing strokes must not darken their opacity by painting twice.
#[test]
fn annotation_views_do_not_duplicate_existing_traces() {
    let graphic =
        ink(r##"<traceGroup xml:id="g"><trace>0 0,1 1</trace></traceGroup><traceGroup><traceView traceDataRef="#g"/></traceGroup>"##).unwrap();
    assert_eq!(graphic, ink("<trace>0 0,1 1</trace>").unwrap());
}

/// Noncircular and rotated nibs are true filled vector outlines rather than averaged-width lines.
#[test]
fn ellipse_rectangle_drop_and_rotated_nib_dimensions() {
    for tip in ["ellipse", "rectangle", "drop"] {
        let graphic=ink(&format!(r#"<brush><brushProperty name="tip" value="{tip}"/><brushProperty name="width" value="4"/><brushProperty name="height" value="8"/></brush><trace>0 0</trace>"#)).unwrap();
        near(graphic.w, 4.0);
        near(graphic.h, 8.0);
        assert!(matches!(graphic.items.as_slice(), [GraphicItem::InkOutline { .. }]));
    }
    let graphic=ink(r#"<brush><brushProperty name="tip" value="rectangle"/><brushProperty name="width" value="4"/><brushProperty name="height" value="8"/></brush><traceFormat><channel name="X"/><channel name="Y"/><channel name="OR" units="deg"/></traceFormat><trace>0 0 90</trace>"#).unwrap();
    near(graphic.w, 8.0);
    near(graphic.h, 4.0);
}

/// Translucent strokes paint each compound run once, but distinct traces stay separate layers.
#[test]
fn transparent_runs_keep_stroke_boundaries() {
    let graphic =
        ink(r#"<brush><brushProperty name="transparency" value="0.5" units="%"/></brush><trace>0 0,10 0,0 0</trace><trace>0 0,10 0</trace>"#)
            .unwrap();
    assert_eq!(graphic.items.len(), 2);
    for item in graphic.items {
        assert!(matches!(item,GraphicItem::InkOutline {alpha,..} if alpha==0.5));
    }
}

/// Width, packed color, normalized alpha and calibrated pressure can vary independently per sample.
#[test]
fn visual_channels_and_brush_precedence() {
    let format = r#"<traceFormat><channel name="X" units="pt"/><channel name="Y" units="pt"/><channel name="BW" units="pt"/><channel name="F" units="N" min="0" max="100"/><intermittentChannels><channel name="C" default="16711680"/><channel name="A" max="255" default="0"/></intermittentChannels></traceFormat>"#;
    let graphic = ink(&format!("{format}<trace>0 0 4 50,10 0 4 100 #0000FF 128</trace>")).unwrap();
    near(graphic.w, 13.0);
    near(graphic.h, 4.0);
    assert!(matches!(graphic.items.last(),Some(GraphicItem::InkOutline {color:Rgb(0,0,255),alpha,..}) if (*alpha-(127.0/255.0)).abs()<1e-6));
    let brush = r##"<brush><brushProperty name="color" value="#00FF00"/><brushProperty name="width" value="6"/><brushProperty name="transparency" value="0"/><brushProperty name="ignorePressure" value="true"/></brush>"##;
    let graphic = ink(&format!("{format}{brush}<trace>0 0 4 50,10 0 4 100 #0000FF 128</trace>")).unwrap();
    assert_eq!(paths(&graphic)[0].1, Rgb(0, 255, 0));
    near(paths(&graphic)[0].2, 6.0);
}

/// Expanded nib geometry shares the decode budget, including failures after valid early strokes.
#[test]
fn advanced_outline_work_and_reference_expansion_are_bounded() {
    let body = r#"<brush><brushProperty name="tip" value="drop"/><brushProperty name="width" value="4"/><brushProperty name="height" value="8"/><brushProperty name="fitToCurve" value="true"/></brush><trace>0 0,10 10,20 0</trace>"#;
    let xml = format!(r#"<ink xmlns="http://www.w3.org/2003/InkML">{body}</ink>"#);
    let mut work = 2_000_000;
    assert!(read(xml.as_bytes(), &mut work).is_some());
    assert!(read(xml.as_bytes(), &mut (2_000_000 - work - 1)).is_none());
    let huge = format!("{}<trace>{}</trace>", body, "0 0,10 10,".repeat(10_000));
    assert!(ink(&huge).is_none(), "geometry generation must not bypass the work limit");
    let mut definitions = String::from(r#"<trace xml:id="v0">0 0,1 1</trace>"#);
    for i in 1..12 {
        definitions.push_str(&format!(
            r##"<traceGroup xml:id="v{i}"><traceView traceDataRef="#v{p}"/><traceView traceDataRef="#v{p}"/></traceGroup>"##,
            p = i - 1
        ));
    }
    assert!(
        ink(&format!(r##"<definitions>{definitions}</definitions><traceView traceDataRef="#v11"/>"##)).is_none(),
        "exponential view trees must stop at the depth/work bounds"
    );
}

/// Curve fitting and noOperation produce bounded vector items, not picture placeholders.
#[test]
fn fitted_curves_and_no_operation_are_bounded() {
    let graphic = ink(r#"<brush><brushProperty name="fitToCurve" value="true"/></brush><trace>0 0,10 10,20 0</trace>"#).unwrap();
    assert!(matches!(&graphic.items[0],GraphicItem::InkOutline {segs,..} if segs.len()>100 && segs.len()<2000));
    let graphic = ink(r#"<brush><brushProperty name="rasterOp" value="noOperation"/></brush><trace>0 0,1 1</trace>"#).unwrap();
    assert!(matches!(&graphic.items[0],GraphicItem::InkOutline {alpha,..} if *alpha==0.0));
}
