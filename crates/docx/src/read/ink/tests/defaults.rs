use super::{ink, paths};

const XY: &str = r#"<traceFormat><channel name="X"/><channel name="Y"/></traceFormat>"#;

fn near(actual: f32, expected: f32) {
    assert!((actual - expected).abs() < 0.001, "{actual} != {expected}");
}

#[test]
fn declared_length_channels_use_millimeters_when_units_are_omitted() {
    let body = format!("{XY}<trace>0 0,25.4 25.4</trace>");
    let implicit = ink(&body).unwrap();
    let explicit = ink(&body.replace("name=\"X\"", "name=\"X\" units=\"mm\"").replace("name=\"Y\"", "name=\"Y\" units=\"mm\"")).unwrap();
    assert_eq!(implicit, explicit);
    near(implicit.w, 73.0);
    near(implicit.h, 73.0);
    for channels in [r#"<channel name="W"/>"#, r#"<channel name="BW"/>"#, r#"<channel name="BW"/><channel name="BH"/>"#] {
        let sample = if channels.contains("BH") { "0 0 25.4 12.7" } else { "0 0 25.4" };
        let body = format!("{}<trace>{sample}</trace>", XY.replace("</traceFormat>", &format!("{channels}</traceFormat>")));
        let graphic = ink(&body).unwrap();
        near(graphic.w, 72.0);
        near(graphic.h, if channels.contains("BH") { 36.0 } else { 72.0 });
        let explicit = body
            .replace("name=\"W\"", "name=\"W\" units=\"mm\"")
            .replace("name=\"BW\"", "name=\"BW\" units=\"mm\"")
            .replace("name=\"BH\"", "name=\"BH\" units=\"mm\"");
        assert_eq!(graphic, ink(&explicit).unwrap());
    }
}

#[test]
fn declared_source_and_canvas_defaults_do_not_change_application_defaults() {
    near(ink("<trace>0 0,25.4 25.4</trace>").unwrap().w, 26.4);
    near(ink(&format!(r##"{XY}<context traceFormatRef="#DefaultTraceFormat"/><trace>0 0,25.4 25.4</trace>"##)).unwrap().w, 26.4);
    near(ink(&format!("<context><inkSource>{XY}</inkSource></context><trace>0 0,25.4 25.4</trace>")).unwrap().w, 73.0);
    for transform in [
        "",
        r#"<canvasTransform><mapping type="identity"/></canvasTransform>"#,
        r#"<canvasTransform><mapping type="unknown"/><mapping type="affine"><affine>1 0 0,0 1 0</affine></mapping></canvasTransform>"#,
    ] {
        let graphic = ink(&format!("<context><canvas>{XY}</canvas>{transform}</context><trace>0 0,25.4 25.4</trace>")).unwrap();
        near(graphic.w, 73.0);
        near(graphic.h, 73.0);
    }
    let body = format!(r#"{XY}<context><canvasTransform><mapping type="identity"/></canvasTransform></context><trace>0 0,25.4 25.4</trace>"#);
    near(ink(&body).unwrap().w, 26.4);
}

#[test]
fn explicit_units_orientation_and_brush_properties_override_channel_defaults() {
    let body = r#"<traceFormat><channel name="X" orientation="-ve"/><channel name="Y" units="in"/></traceFormat><trace>0 0,-25.4 1</trace>"#;
    assert_eq!(ink(body), ink("<trace>0 0,72 72</trace>"));
    let body = format!(r#"{XY}<brush><brushProperty name="width" value="4"/></brush><trace>0 0,25.4 0</trace>"#);
    let graphic = ink(&body).unwrap();
    near(graphic.w, 76.0);
    near(paths(&graphic)[0].2, 4.0);
    assert_eq!(graphic, ink(&body.replace("value=\"4\"", "value=\"4\" units=\"pt\"")).unwrap());
    let body = format!(
        r#"<context><inkSource>{XY}<channelProperties><channelProperty channel="X" name="resolution" value="1000" units="1/in"/></channelProperties></inkSource></context><trace>0 0,25.4 0</trace>"#
    );
    near(ink(&body).unwrap().w, 73.0);
}

#[test]
fn force_defaults_to_percentage_and_explicit_physical_units_keep_range_normalization() {
    let format = r#"<traceFormat><channel name="X" units="pt"/><channel name="Y" units="pt"/><channel name="BW" units="pt"/><channel name="F" min="10" max="110"/></traceFormat>"#;
    let body = format!("{format}<trace>0 0 4 .5,10 0 4 1</trace>");
    let graphic = ink(&body).unwrap();
    assert_eq!(graphic, ink(&body.replace("name=\"F\"", "name=\"F\" units=\"%\"")).unwrap());
    near(graphic.w, 13.0);
    near(graphic.h, 4.0);
    let physical = body.replace("name=\"F\"", "name=\"F\" units=\"N\"").replace("4 .5", "4 60").replace("4 1</trace>", "4 110</trace>");
    assert_eq!(graphic, ink(&physical).unwrap());
    let invalid = format!("{body}<trace>0 0 4 1.01</trace>");
    assert!(ink(&invalid).is_none());
    let uncalibrated = r#"<traceFormat><channel name="X" units="pt"/><channel name="Y" units="pt"/><channel name="F" units="N"/></traceFormat><trace>0 0 50,10 0 100</trace>"#;
    assert_eq!(ink(uncalibrated), ink("<trace>0 0,10 0</trace>"));
}

#[test]
fn rotation_defaults_remain_degrees_and_custom_channel_names_are_not_aliases() {
    let body = format!(
        r#"{}<brush><brushProperty name="tip" value="rectangle"/><brushProperty name="width" value="8"/><brushProperty name="height" value="4"/></brush><trace>0 0 90</trace>"#,
        XY.replace("</traceFormat>", r#"<channel name="OR"/></traceFormat>"#)
    );
    let graphic = ink(&body).unwrap();
    near(graphic.w, 4.0);
    near(graphic.h, 8.0);
    assert_eq!(graphic, ink(&body.replace("name=\"OR\"", "name=\"OR\" units=\"deg\"")).unwrap());
    let body = r#"<traceFormat><channel name="X" units="pt"/><channel name="Y" units="pt"/><channel name="f"/><channel name="bw"/></traceFormat><trace>0 0 100 100,10 0 100 100</trace>"#;
    assert_eq!(ink(body), ink("<trace>0 0,10 0</trace>"));
    assert!(ink(&format!("{}<trace>0 0,1 1</trace>", XY.replace("name=\"X\"", "name=\"X\" units=\"\""))).is_none());
}
