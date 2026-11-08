use super::*;

/// Referenced color literals cannot hide uncharged scans behind cheap brush lookups.
#[test]
fn padded_referenced_brush_colors_are_bounded() {
    let body = |padding: usize, copies: usize| {
        format!(
            r##"<definitions><brush xml:id="b"><brushProperty name="color" value="{}#000000"/></brush></definitions>{}"##,
            " ".repeat(padding),
            r##"<trace brushRef="#b">0 0</trace>"##.repeat(copies)
        )
    };
    assert!(ink(&body(1_500_000, 2)).is_none(), "reject oversized literals before repeated trimming");
    assert!(ink(&body(1, 8_000)).is_some(), "ordinary referenced colors remain supported");
}

/// One expensive part leaves room for later ink, without refunding completed work.
#[test]
fn each_part_has_a_cap_and_only_completed_reservations_are_charged() {
    let expensive = format!(
        r#"<ink xmlns="http://www.w3.org/2003/InkML"><brush><brushProperty name="fitToCurve" value="true"/><brushProperty name="transparency" value="128"/></brush><trace>{}</trace></ink>"#,
        "0 0,1 1,".repeat(1_000)
    );
    let mut work = MAX_WORK;
    assert!(read(expensive.as_bytes(), &mut work).is_none());
    assert!((MAX_WORK - MAX_PART_WORK..MAX_WORK).contains(&work));
    let simple = br#"<ink xmlns="http://www.w3.org/2003/InkML"><trace>0 0,1 1</trace></ink>"#;
    assert!(read(simple, &mut work).is_some());
    let mut tiny = 7;
    assert!(spend(&mut tiny, 8).is_none());
    assert_eq!(tiny, 7);
    assert!(spend(&mut tiny, 3).is_some());
    assert_eq!(tiny, 4);
}

/// Unknown contact refuses the whole preview, while known pen-up traces remain nonpainting.
#[test]
fn unknown_contact_rejects_the_complete_preview() {
    let switch = r#"<traceFormat><channel name="X" units="pt"/><channel name="Y" units="pt"/><intermittentChannels><channel name="S" type="boolean"/></intermittentChannels></traceFormat>"#;
    for body in [
        r#"<trace type="indeterminate">0 0,10 10</trace>"#.to_string(),
        r#"<trace>0 0,10 10</trace><trace type="indeterminate">0 10,10 0</trace>"#.to_string(),
        format!(r#"{switch}<trace>0 0 T,5 5 ?,10 10 T</trace>"#),
        format!(r#"{switch}<trace type="penUp">0 0 F,5 5 ?</trace>"#),
    ] {
        assert!(ink(&body).is_none(), "{body}");
    }
    assert!(ink(&format!(r#"{switch}<trace type="indeterminate">0 0 T,1 1 T,2 2 F</trace>"#)).is_some());
    let up = ink(r#"<trace type="penUp">0 0,10 10</trace>"#).unwrap();
    assert!(matches!(up.items.as_slice(), [GraphicItem::InkOutline { segs, .. }] if segs.is_empty()));
}

/// Continued tip changes form outlines, but incompatible curve-fitting changes refuse preview.
#[test]
fn continuation_tip_changes_are_outlined_and_curve_changes_refuse_preview() {
    let definitions = r##"<definitions><brush xml:id="ellipse"><brushProperty name="width" value="4" units="pt"/></brush><brush xml:id="square"><brushProperty name="width" value="4" units="pt"/><brushProperty name="tip" value="rectangle"/></brush><brush xml:id="smooth"><brushProperty name="width" value="4" units="pt"/><brushProperty name="fitToCurve" value="true"/></brush></definitions>"##;
    let mixed = format!(
        r##"{definitions}<trace xml:id="first" brushRef="#ellipse" continuation="begin">0 0,10 0</trace><trace brushRef="#square" continuation="end" priorRef="#first">10 0,20 0</trace>"##
    );
    let graphic = ink(&mixed).unwrap();
    assert_eq!((graphic.w, graphic.h), (24.0, 4.0));
    let [GraphicItem::InkOutline { segs, .. }] = graphic.items.as_slice() else { panic!("{:?}", graphic.items) };
    assert!(segs.iter().any(|p| matches!(p, PathSeg::Move(x, y) | PathSeg::Line(x, y) if *x == 24.0 && (*y == 0.0 || *y == 4.0))));
    let smooth = mixed.replace("#square", "#smooth");
    assert!(ink(&smooth).is_none());
    let both_smooth = smooth.replace("#ellipse\" continuation", "#smooth\" continuation");
    assert!(ink(&both_smooth).is_some());
    let separate = smooth.replace("continuation=\"end\" priorRef=\"#first\"", "type=\"penUp\"");
    assert!(ink(&separate).is_some());
}

/// Pressure must be known and normalizable unless the brush explicitly ignores it.
#[test]
fn pressure_must_be_known_and_normalizable_unless_explicitly_ignored() {
    let format = r#"<traceFormat><channel name="X" units="pt"/><channel name="Y" units="pt"/><intermittentChannels><channel name="F"/></intermittentChannels></traceFormat>"#;
    let missing = format!(r#"{format}<trace>0 0 .5,5 5 ?,10 10 .5</trace>"#);
    assert!(ink(&missing).is_none());
    let no_range = r#"<traceFormat><channel name="X" units="pt"/><channel name="Y" units="pt"/><channel name="F" units="N"/></traceFormat><trace>0 0 1,10 10 100</trace>"#;
    assert!(ink(no_range).is_none());
    let ignored = r#"<brush><brushProperty name="ignorePressure" value="true"/></brush>"#;
    for (body, control) in [(missing.as_str(), "<trace>0 0,5 5,10 10</trace>"), (no_range, "<trace>0 0,10 10</trace>")] {
        let result = ink(&format!("{ignored}{body}")).unwrap();
        assert_eq!(result, ink(control).unwrap());
    }
    assert!(ink(&missing.replace("5 5 ?", "5 5 .25")).is_some());
}

/// Collapsed swept nibs are nonpainting, without discarding the live ends of a pressure-varying run.
#[test]
fn zero_pressure_outlines_use_the_nonpainting_sentinel() {
    let format = r#"<traceFormat><channel name="X" units="pt"/><channel name="Y" units="pt"/><channel name="F"/></traceFormat>"#;
    for samples in ["0 0 0,10 10 0", "5 5 0"] {
        let graphic = ink(&format!("{format}<trace>{samples}</trace>")).unwrap();
        assert!(matches!(graphic.items.as_slice(), [GraphicItem::InkOutline { segs, alpha: 0.0, .. }] if segs.is_empty()));
    }
    let graphic = ink(&format!("{format}<trace>0 0 0.5,10 10 0,20 0 0.5</trace>")).unwrap();
    assert!(matches!(graphic.items.as_slice(), [GraphicItem::InkOutline { segs, alpha: 1.0, .. }] if !segs.is_empty()));
    // A zero-height nib can still sweep a filled area: classify the swept polygon,
    // not the area of each individual nib or the presence of a zero dimension.
    let swept = ink(r#"<traceFormat><channel name="X" units="pt"/><channel name="Y" units="pt"/><channel name="BW" units="pt"/><channel name="BH" units="pt"/></traceFormat><trace>0 0 4 0,0 10 4 0</trace>"#).unwrap();
    assert!(matches!(swept.items.as_slice(), [GraphicItem::InkOutline { segs, alpha: 1.0, .. }] if !segs.is_empty()));
}

/// Repeated nonuniform fits compose before averaging circular widths, including extreme aspect ratios.
#[test]
fn taps_and_centerlines_keep_the_same_width_during_frame_fitting() {
    let mut graphic = ink("<trace>0 0,10 0</trace><trace>5 5</trace>").unwrap();
    let initial = (graphic.w, graphic.h);
    fit_frame(&mut graphic, initial.0 * 4.0, initial.1);
    let [GraphicItem::Path { stroke_width, .. }, GraphicItem::Shape { rect, .. }] = graphic.items.as_slice() else { panic!("{:?}", graphic.items) };
    assert!((*stroke_width - 2.5).abs() < 1e-6);
    assert_eq!((rect[2], rect[3]), (*stroke_width, *stroke_width));
    let center = (rect[0] + rect[2] / 2.0, rect[1] + rect[3] / 2.0);
    assert_eq!(center, (22.0, 5.5));
    let prior = (graphic.w, graphic.h);
    fit_frame(&mut graphic, prior.0 / 2.0, prior.1 * 3.0);
    let [GraphicItem::Path { stroke_width, .. }, GraphicItem::Shape { rect, .. }] = graphic.items.as_slice() else { panic!() };
    assert!((*stroke_width - 2.5).abs() < 1e-6, "mean(4 * 0.5, 1 * 3), not mean(4, 1) * mean(0.5, 3)");
    assert_eq!((rect[2], rect[3]), (*stroke_width, *stroke_width));
    assert_eq!((rect[0] + rect[2] / 2.0, rect[1] + rect[3] / 2.0), (11.0, 16.5));

    let mut extreme = ink("<trace>0 0,10 0</trace><trace>5 5</trace>").unwrap();
    let initial = (extreme.w, extreme.h);
    fit_frame(&mut extreme, crate::units::MAX_LEN_PT, 0.001);
    fit_frame(&mut extreme, crate::units::MAX_LEN_PT, crate::units::MAX_LEN_PT);
    let [GraphicItem::Path { stroke_width, .. }, GraphicItem::Shape { rect, .. }] = extreme.items.as_slice() else { panic!() };
    let expected = (crate::units::MAX_LEN_PT / initial.0 + crate::units::MAX_LEN_PT / initial.1) / 2.0;
    assert!((*stroke_width - expected).abs() < 0.001, "{stroke_width} != {expected}");
    assert_eq!((rect[2], rect[3]), (*stroke_width, *stroke_width));
}

/// Valid very thin and wide pens retain their widths through decoding and frame fitting.
#[test]
fn valid_extreme_pen_widths_survive_decoding_and_fitting() {
    for width in [0.01f32, 400.0] {
        let body = format!(r#"<brush><brushProperty name="width" value="{width}" units="pt"/></brush><trace>0 0,10 0</trace><trace>5 5</trace>"#);
        let mut graphic = ink(&body).unwrap();
        let before = (graphic.w, graphic.h);
        let [GraphicItem::Path { stroke_width, .. }, GraphicItem::Shape { rect, .. }] = graphic.items.as_slice() else { panic!() };
        assert!((*stroke_width - width).abs() < 1e-5);
        assert!((rect[2] - width).abs() < 1e-5);
        fit_frame(&mut graphic, before.0 * 2.0, before.1 * 2.0);
        let [GraphicItem::Path { stroke_width, .. }, GraphicItem::Shape { rect, .. }] = graphic.items.as_slice() else { panic!() };
        assert!((*stroke_width - width * 2.0).abs() < 1e-4);
        assert_eq!((rect[2], rect[3]), (*stroke_width, *stroke_width));
    }
}
