use serde_json::json;
use wordcraft_doc::StoryRef;
use wordcraft_doc::freeform::InkTool;
use wordcraft_doc::para::{InlineObject, ShapeKind, Wrap};

use crate::Session;

fn session() -> Session {
    let mut s = Session::new(wordcraft_doc::Document::new());
    s.run("document.setText", &json!({"text": "First paragraph of text.\nSecond paragraph of text."})).unwrap();
    s
}

fn ink(s: &Session) -> Vec<InlineObject> {
    s.doc.body.iter().filter_map(|b| b.as_para()).flat_map(|p| p.objects.iter()).filter(|o| o.ink().is_some()).cloned().collect()
}

#[test]
fn a_stroke_inserts_floating_ink_through_its_points_in_one_undo_step() {
    let mut s = session();
    let text = s.doc.plain_text(StoryRef::Body);
    let r = s
        .run("draw.stroke", &json!({"page": 0, "points": [[100, 90], [150, 120], [f64::NAN, 3], [200, 95]], "color": "C00000", "width": 2}))
        .unwrap();
    let [x, y] = [r["rect"][0].as_f64().unwrap() as f32, r["rect"][1].as_f64().unwrap() as f32];
    assert_eq!((x, y), (99.0, 89.0), "the points' bounds, grown by half the width");
    let objs = ink(&s);
    let [InlineObject::Shape { stroke, stroke_width, float, freeform: Some(f), .. }] = objs.as_slice() else { panic!("one stroke: {objs:?}") };
    assert_eq!((*stroke, *stroke_width, float.wrap), (Some(wordcraft_doc::Rgb(0xC0, 0, 0)), 2.0, Wrap::InFrontOfText));
    assert_eq!(f.ink, Some(InkTool::Pen));
    assert_eq!(f.paths[0].pts, vec![[1.0, 1.0], [51.0, 31.0], [101.0, 6.0]], "NaN dropped, shape-local");
    // Laid out where it was drawn.
    let at = wordcraft_doc::Pos { story: StoryRef::Body, path: wordcraft_doc::Path::top(0), off: 0 };
    let hit = s.layout().object(&at, 0).unwrap();
    assert!((hit.rect.x - 99.0).abs() < 0.01 && (hit.rect.y - 89.0).abs() < 0.01, "{:?}", hit.rect);
    s.run("edit.undo", &json!({})).unwrap();
    assert!(ink(&s).is_empty());
    assert_eq!(s.doc.plain_text(StoryRef::Body), text);
    // A highlighter goes behind the text, translucent.
    s.run("draw.highlighter", &json!({"color": "00FF00"})).unwrap();
    s.run("draw.stroke", &json!({"page": 0, "points": [[80, 80]]})).unwrap();
    let objs = ink(&s);
    let [InlineObject::Shape { float, freeform: Some(f), stroke, .. }] = objs.as_slice() else { panic!() };
    assert_eq!((float.wrap, f.ink, *stroke), (Wrap::BehindText, Some(InkTool::Highlighter), Some(wordcraft_doc::Rgb(0, 0xFF, 0))));
    assert!(f.alpha < 1.0);
    // Hostile input is refused, not drawn.
    assert!(s.run("draw.stroke", &json!({"page": 99, "points": [[1, 1]]})).is_err());
    assert!(s.run("draw.stroke", &json!({"page": 0, "points": [["x", 1], [1]]})).is_err());
    assert!(s.run("draw.stroke", &json!({"page": 0, "points": [[1, 1]], "tool": "crayon"})).is_err());
}

#[test]
fn the_eraser_removes_the_stroke_under_the_point_and_hide_ink_hides_it() {
    let mut s = session();
    s.run("draw.stroke", &json!({"page": 0, "points": [[100, 100], [200, 100]], "width": 2})).unwrap();
    assert!(s.run("draw.eraser", &json!({"page": 0, "x": 150, "y": 130})).is_err(), "nothing there");
    assert_eq!(ink(&s).len(), 1);
    // Review › Hide Ink: not drawn, not erasable, still in the document.
    assert_eq!(s.run("review.hideInk", &json!({})).unwrap()["value"], true);
    assert!(s.view.hide_ink);
    assert!(s.run("draw.eraser", &json!({"page": 0, "x": 150, "y": 101})).is_err());
    s.run("review.hideInk", &json!({"value": false})).unwrap();
    s.run("draw.eraser", &json!({"page": 0, "x": 150, "y": 101})).unwrap();
    assert!(ink(&s).is_empty());
    // Without a point it picks up the eraser; Select puts it down.
    s.run("draw.eraser", &json!({})).unwrap();
    assert_eq!(s.view.draw.mode, super::DrawMode::Eraser);
    s.run("draw.pencil", &json!({"width": 1e9})).unwrap();
    assert_eq!((s.view.draw.mode, s.view.draw.pencil.width), (super::DrawMode::Pen(InkTool::Pencil), super::MAX_WIDTH));
    s.run("draw.select", &json!({})).unwrap();
    assert_eq!(s.view.draw.mode, super::DrawMode::Select);
}

/// #445: a turned stroke is erased where it is drawn, not where it was before the turn.
#[test]
fn a_rotated_stroke_is_erased_where_it_is_drawn() {
    let rotated = || {
        let mut s = session();
        s.run("draw.stroke", &json!({"page": 0, "points": [[100, 100], [200, 100]], "width": 2, "color": "000000"})).unwrap();
        s.run("select.objects", &json!({"index": 0})).unwrap();
        s.run("arrange.rotation", &json!({"degrees": 90})).unwrap();
        s
    };
    // Its old horizontal path is blank now: nothing to erase there, and it stays.
    let mut s = rotated();
    assert!(s.run("draw.erase", &json!({"page": 0, "x": 110, "y": 100})).is_err());
    assert_eq!(ink(&s).len(), 1);
    // The visible vertical stroke (centred on x = 150) is erased.
    s.run("draw.erase", &json!({"page": 0, "x": 150, "y": 60})).unwrap();
    assert!(ink(&s).is_empty());
    let mut s = rotated();
    s.run("draw.eraser", &json!({"page": 0, "x": 150, "y": 140})).unwrap();
    assert!(ink(&s).is_empty(), "the point-taking eraser too");
    // Hidden ink stays out of reach.
    let mut s = rotated();
    s.run("review.hideInk", &json!({"value": true})).unwrap();
    assert!(s.run("draw.erase", &json!({"page": 0, "x": 150, "y": 60})).is_err());
}

/// The lasso, like the eraser, finds a turned stroke where it is drawn.
#[test]
fn the_lasso_selects_a_rotated_stroke_where_it_is_drawn() {
    let mut s = session();
    s.run("draw.stroke", &json!({"page": 0, "points": [[100, 100], [200, 100]], "width": 2})).unwrap();
    s.run("select.objects", &json!({"index": 0})).unwrap();
    s.run("arrange.rotation", &json!({"degrees": 90})).unwrap();
    // A loop around its old horizontal path misses it; one around the vertical stroke takes it.
    let r = s.run("draw.lasso", &json!({"page": 0, "points": [[90, 95], [210, 95], [210, 105], [90, 105]]})).unwrap();
    assert_eq!(r["count"], 0);
    let r = s.run("draw.lasso", &json!({"page": 0, "points": [[140, 40], [160, 40], [160, 160], [140, 160]]})).unwrap();
    assert_eq!(r["count"], 1);
}

/// `draw.eraser` is a view command, but with a point it erases through `draw.erase`, which
/// changes the document: that must be one undo step and mark the document changed.
#[test]
fn erasing_through_the_eraser_command_is_undoable_and_marks_the_document_dirty() {
    let mut s = session();
    s.run("draw.stroke", &json!({"page": 0, "points": [[100, 100], [200, 100]], "width": 2})).unwrap();
    s.dirty = false;
    let steps = s.undo_depth();
    s.run("draw.eraser", &json!({"page": 0, "x": 150, "y": 101})).unwrap();
    assert!(ink(&s).is_empty());
    assert!(s.dirty, "erasing is an unsaved change");
    assert_eq!(s.undo_depth(), steps + 1, "one undo step");
    s.run("edit.undo", &json!({})).unwrap();
    assert_eq!(ink(&s).len(), 1, "undo brings the stroke back");
    s.run("edit.redo", &json!({})).unwrap();
    assert!(ink(&s).is_empty());
}

/// A closed rectangle drawn as page points, `per` points to a side.
fn rect_stroke(x: f32, y: f32, w: f32, h: f32) -> Vec<[f32; 2]> {
    let c = [[x, y], [x + w, y], [x + w, y + h], [x, y + h]];
    let mut pts = Vec::new();
    for i in 0..4 {
        let (a, b) = (c[i], c[(i + 1) % 4]);
        for k in 0..10 {
            let t = k as f32 / 10.0;
            pts.push([a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]);
        }
    }
    pts.push([x, y]);
    pts
}

#[test]
fn the_lasso_selects_the_ink_inside_and_moves_recolours_and_deletes_it() {
    let mut s = session();
    s.run("draw.stroke", &json!({"page": 0, "points": [[100, 100], [140, 110]]})).unwrap();
    s.run("draw.stroke", &json!({"page": 0, "points": [[300, 300], [340, 310]]})).unwrap();
    // Without points it picks up the lasso; a loop around the first stroke selects only it.
    s.run("draw.lasso", &json!({})).unwrap();
    assert_eq!(s.view.draw.mode, super::DrawMode::Lasso);
    let r = s.run("draw.lasso", &json!({"page": 0, "points": [[80, 80], [200, 80], [200, 150], [80, 150]]})).unwrap();
    assert_eq!(r["count"], 1);
    let before = s.layout().object(&super::lasso_selection(&s).unwrap().items[0], 0).unwrap().rect;
    // Moved, recoloured: each one undo step, and the selection stays.
    let steps = s.undo_depth();
    s.run("draw.lassoMove", &json!({"dx": 20, "dy": -10})).unwrap();
    s.run("draw.lassoColor", &json!({"color": "0070C0"})).unwrap();
    assert_eq!(s.undo_depth(), steps + 2);
    let sel = super::lasso_selection(&s).unwrap().items[0].clone();
    let after = s.layout().object(&sel, 0).unwrap().rect;
    assert!((after.x - before.x - 20.0).abs() < 0.01 && (after.y - before.y + 10.0).abs() < 0.01, "{before:?} -> {after:?}");
    let objs = ink(&s);
    let blue = objs.iter().filter(|o| matches!(o, InlineObject::Shape { stroke: Some(c), .. } if c.hex() == "0070C0")).count();
    assert_eq!(blue, 1);
    // Recolouring to the colour it has changes nothing, and the selection stays; an edit
    // elsewhere ends it.
    s.run("draw.lassoColor", &json!({"color": "0070C0"})).unwrap();
    assert_eq!(s.undo_depth(), steps + 2);
    assert_eq!(super::lasso_selection(&s).map(|l| l.items.clone()), Some(vec![sel.clone()]));
    s.run("text.insert", &json!({"text": "x"})).unwrap();
    assert!(super::lasso_selection(&s).is_none());
    let r = s.run("draw.lasso", &json!({"page": 0, "points": [[80, 80], [260, 80], [260, 180], [80, 180]]})).unwrap();
    assert_eq!(r["count"], 1);
    // Deleted: only the selected stroke goes, and the selection with it.
    s.run("draw.lassoDelete", &json!({})).unwrap();
    assert_eq!(ink(&s).len(), 1);
    assert!(super::lasso_selection(&s).is_none());
    assert!(s.run("draw.lassoMove", &json!({"dx": 1, "dy": 1})).is_err(), "nothing selected");
    // Hostile loops are refused or select nothing; they never panic.
    assert!(s.run("draw.lasso", &json!({"page": 0, "points": [[1, 1], [2, 2]]})).is_err());
    let huge: Vec<[f64; 2]> = (0..20_000).map(|i| [(i % 600) as f64, (i / 40) as f64]).collect();
    let _ = s.run("draw.lasso", &json!({"page": 0, "points": huge}));
}

#[test]
fn ink_to_shape_turns_shape_like_strokes_into_shapes_and_leaves_scribbles() {
    let mut s = session();
    assert_eq!(s.run("draw.inkToShape", &json!({})).unwrap()["value"], true);
    let steps = s.undo_depth();
    let r = s.run("draw.stroke", &json!({"page": 0, "points": rect_stroke(100.0, 100.0, 120.0, 60.0), "color": "C00000", "width": 2})).unwrap();
    assert_eq!(r["shape"], "rectangle");
    assert_eq!(s.undo_depth(), steps + 1, "drawn and converted in one undo step");
    let at: wordcraft_doc::Pos = serde_json::from_value(r["pos"].clone()).unwrap();
    let obj = s.doc.para_at(&at).and_then(|p| p.object_at(at.off)).cloned().unwrap();
    let InlineObject::Shape { kind, w, h, stroke, stroke_width, freeform, .. } = obj else { panic!("{obj:?}") };
    assert_eq!((kind, stroke, stroke_width, freeform.is_none()), (ShapeKind::Rectangle, Some(wordcraft_doc::Rgb(0xC0, 0, 0)), 2.0, true));
    assert!((w - 120.0).abs() < 2.0 && (h - 60.0).abs() < 2.0, "{w} x {h}");
    let rect = s.layout().object(&at, 0).unwrap().rect;
    assert!((rect.x - 100.0).abs() < 2.0 && (rect.y - 100.0).abs() < 2.0, "where it was drawn: {rect:?}");
    // A scribble stays ink.
    let r = s.run("draw.stroke", &json!({"page": 0, "points": [[300, 300], [320, 340], [340, 300], [360, 340], [380, 300]]})).unwrap();
    assert!(r["shape"].is_null());
    assert_eq!(ink(&s).len(), 1);
    // Off again; a drawn ellipse stays ink until the lasso selects it and Ink to Shape converts it.
    s.run("draw.inkToShape", &json!({"value": false})).unwrap();
    let ring: Vec<[f32; 2]> =
        (0..=48).map(|i| i as f32 / 48.0 * std::f32::consts::TAU).map(|a| [200.0 + 50.0 * a.cos(), 500.0 + 30.0 * a.sin()]).collect();
    s.run("draw.stroke", &json!({"page": 0, "points": ring})).unwrap();
    assert_eq!(ink(&s).len(), 2);
    s.run("draw.lasso", &json!({"page": 0, "points": [[130, 450], [270, 450], [270, 550], [130, 550]]})).unwrap();
    let r = s.run("draw.inkToShape", &json!({})).unwrap();
    assert_eq!(r["converted"][0]["shape"], "ellipse");
    assert_eq!(ink(&s).len(), 1);
    assert!(!s.view.draw.ink_to_shape, "converting the selection doesn't turn the mode on");
}

#[test]
fn ink_to_shape_turns_an_arrow_stroke_into_a_line_shape_with_a_head_that_survives_docx() {
    let mut s = session();
    s.run("draw.inkToShape", &json!({"value": true})).unwrap();
    // Right to left and downwards: shaft, then both barbs of the head at the tip (100, 300).
    let tail = [300.0f32, 200.0];
    let tip = [100.0f32, 300.0];
    let back = (tail[1] - tip[1]).atan2(tail[0] - tip[0]);
    let barb = |sign: f32| [tip[0] + 30.0 * (back + sign * 0.5).cos(), tip[1] + 30.0 * (back + sign * 0.5).sin()];
    let mut pts: Vec<[f32; 2]> =
        (0..20).map(|i| i as f32 / 20.0).map(|t| [tail[0] + (tip[0] - tail[0]) * t, tail[1] + (tip[1] - tail[1]) * t]).collect();
    pts.extend([tip, barb(1.0), tip, barb(-1.0)]);
    let r = s.run("draw.stroke", &json!({"page": 0, "points": pts, "color": "C00000", "width": 2})).unwrap();
    assert_eq!(r["shape"], "arrow");
    let at: wordcraft_doc::Pos = serde_json::from_value(r["pos"].clone()).unwrap();
    let obj = s.doc.para_at(&at).and_then(|p| p.object_at(at.off)).cloned().unwrap();
    let InlineObject::Shape { kind, w, h, stroke, stroke_width, freeform, extra, float, .. } = obj else { panic!("{obj:?}") };
    assert_eq!((kind, stroke, stroke_width, freeform.is_none()), (ShapeKind::StraightConnector, Some(wordcraft_doc::Rgb(0xC0, 0, 0)), 2.0, true));
    assert!(extra.arrow_end && !extra.arrow_start);
    // Tail at the top right, tip at the bottom left: the frame is flipped across.
    assert!(float.flip_h && !float.flip_v, "{float:?}");
    assert!((w - 200.0).abs() < 1.0 && (h - 100.0).abs() < 1.0, "{w} x {h}");
    // It survives a DOCX save and open.
    let bytes = wordcraft_docx::write(&s.doc).unwrap();
    let back = wordcraft_docx::read(&bytes).unwrap();
    let again: Vec<_> = back.body.iter().filter_map(|b| b.as_para()).flat_map(|p| p.objects.iter()).collect();
    let Some(InlineObject::Shape { extra, float, .. }) =
        again.iter().find(|o| matches!(o, InlineObject::Shape { kind: ShapeKind::StraightConnector, .. }))
    else {
        panic!("{again:?}")
    };
    assert!(extra.arrow_end && !extra.arrow_start && float.flip_h, "{extra:?} {float:?}");
}

/// Every drawing in the body, in document order.
fn drawings(s: &Session) -> Vec<InlineObject> {
    s.doc
        .body
        .iter()
        .filter_map(|b| b.as_para())
        .flat_map(|p| p.objects.iter())
        .filter(|o| matches!(o, InlineObject::Shape { .. }))
        .cloned()
        .collect()
}

/// A straight stroke from (100, 300) to (300, 300).
fn shaft_stroke() -> Vec<[f32; 2]> {
    (0..=40).map(|i| [100.0 + i as f32 * 5.0, 300.0]).collect()
}

#[test]
fn ink_to_shape_turns_a_line_and_a_head_drawn_after_it_into_one_arrow_in_one_undo_step() {
    let mut s = session();
    s.run("draw.inkToShape", &json!({"value": true})).unwrap();
    let r = s.run("draw.stroke", &json!({"page": 0, "points": shaft_stroke(), "color": "C00000", "width": 2})).unwrap();
    assert_eq!(r["shape"], "line");
    let steps = s.undo_depth();
    // An open V at the right-hand end, drawn as a second stroke.
    let head = [[274.0, 285.0], [287.0, 292.5], [300.0, 300.0], [287.0, 307.5], [274.0, 315.0]];
    let r = s.run("draw.stroke", &json!({"page": 0, "points": head})).unwrap();
    assert_eq!(r["shape"], "arrow");
    assert_eq!(s.undo_depth(), steps + 1, "paired in one undo step");
    assert!(ink(&s).is_empty(), "the head stroke is gone");
    let all = drawings(&s);
    let [InlineObject::Shape { kind, w, stroke, stroke_width, extra, float, .. }] = all.as_slice() else { panic!("one arrow: {all:?}") };
    assert_eq!((*kind, *stroke, *stroke_width), (ShapeKind::StraightConnector, Some(wordcraft_doc::Rgb(0xC0, 0, 0)), 2.0));
    assert!(extra.arrow_end && !extra.arrow_start && !float.flip_h && !float.flip_v, "{extra:?} {float:?}");
    assert!((w - 200.0).abs() < 1.0, "{w}");
    let at: wordcraft_doc::Pos = serde_json::from_value(r["pos"].clone()).unwrap();
    assert!(matches!(s.doc.para_at(&at).and_then(|p| p.object_at(at.off)), Some(InlineObject::Shape { kind: ShapeKind::StraightConnector, .. })));
    let rect = s.layout().object(&at, 0).unwrap().rect;
    assert!((rect.x - 100.0).abs() < 1.0 && (rect.y - 300.0).abs() < 1.0, "where the line was: {rect:?}");
    let got: Vec<f32> = r["rect"].as_array().unwrap().iter().map(|v| v.as_f64().unwrap() as f32).collect();
    assert!(
        got.len() == 4 && [rect.x, rect.y, rect.w, rect.h].iter().zip(&got).all(|(a, b)| (a - b).abs() < 0.5),
        "the arrow's rect, not the deleted head's: {got:?} vs {rect:?}"
    );
    // It survives a DOCX save and open.
    let back = wordcraft_docx::read(&wordcraft_docx::write(&s.doc).unwrap()).unwrap();
    let again: Vec<_> = back.body.iter().filter_map(|b| b.as_para()).flat_map(|p| p.objects.iter()).collect();
    assert!(
        again
            .iter()
            .any(|o| matches!(o, InlineObject::Shape { kind: ShapeKind::StraightConnector, extra, .. } if extra.arrow_end && !extra.arrow_start)),
        "{again:?}"
    );
    // Undo brings back the line, without the head.
    s.run("edit.undo", &json!({})).unwrap();
    let all = drawings(&s);
    let [InlineObject::Shape { kind: ShapeKind::Freeform, extra, .. }] = all.as_slice() else { panic!("the line: {all:?}") };
    assert!(!extra.arrow_end);
    assert!(ink(&s).is_empty());
}

#[test]
fn ink_to_shape_pairs_a_closed_triangle_head_at_the_start_and_a_head_drawn_first() {
    let mut s = session();
    s.run("draw.inkToShape", &json!({"value": true})).unwrap();
    s.run("draw.stroke", &json!({"page": 0, "points": shaft_stroke()})).unwrap();
    // A closed triangle at the left end, where the line began: the arrow points left.
    let tri = [[100.0, 300.0], [113.0, 292.5], [126.0, 285.0], [126.0, 300.0], [126.0, 315.0], [113.0, 307.5], [100.0, 300.0]];
    let r = s.run("draw.stroke", &json!({"page": 0, "points": tri})).unwrap();
    assert_eq!(r["shape"], "arrow");
    let all = drawings(&s);
    let [InlineObject::Shape { kind: ShapeKind::StraightConnector, extra, float, .. }] = all.as_slice() else { panic!("one arrow: {all:?}") };
    assert!(extra.arrow_end && float.flip_h && !float.flip_v, "tip at the left: {extra:?} {float:?}");
    // Head first (it stays ink on its own), then the shaft that ends at its apex.
    let mut s = session();
    s.run("draw.inkToShape", &json!({"value": true})).unwrap();
    let r = s.run("draw.stroke", &json!({"page": 0, "points": [[274.0, 285.0], [300.0, 300.0], [274.0, 315.0]]})).unwrap();
    assert!(r["shape"].is_null());
    let r = s.run("draw.stroke", &json!({"page": 0, "points": shaft_stroke()})).unwrap();
    assert_eq!(r["shape"], "arrow");
    assert!(ink(&s).is_empty());
    let all = drawings(&s);
    let [InlineObject::Shape { kind: ShapeKind::StraightConnector, extra, float, .. }] = all.as_slice() else { panic!("one arrow: {all:?}") };
    assert!(extra.arrow_end && !float.flip_h, "{extra:?} {float:?}");
    // A V far from the line's ends stays ink, and the line stays a line.
    let mut s = session();
    s.run("draw.inkToShape", &json!({"value": true})).unwrap();
    s.run("draw.stroke", &json!({"page": 0, "points": shaft_stroke()})).unwrap();
    let r = s.run("draw.stroke", &json!({"page": 0, "points": [[174.0, 385.0], [200.0, 400.0], [174.0, 415.0]]})).unwrap();
    assert!(r["shape"].is_null());
    assert_eq!(ink(&s).len(), 1);
    assert!(drawings(&s).iter().all(|o| !matches!(o, InlineObject::Shape { kind: ShapeKind::StraightConnector, .. })));
}

#[test]
fn converting_a_lasso_selection_pairs_a_line_and_a_head_stroke_into_an_arrow() {
    let mut s = session();
    s.run("draw.stroke", &json!({"page": 0, "points": shaft_stroke(), "color": "0070C0"})).unwrap();
    s.run("draw.stroke", &json!({"page": 0, "points": [[274.0, 285.0], [300.0, 300.0], [274.0, 315.0]]})).unwrap();
    s.run("draw.stroke", &json!({"page": 0, "points": rect_stroke(100.0, 400.0, 120.0, 60.0)})).unwrap();
    assert_eq!(ink(&s).len(), 3);
    let steps = s.undo_depth();
    s.run("draw.lasso", &json!({"page": 0, "points": [[80, 260], [340, 260], [340, 480], [80, 480]]})).unwrap();
    let r = s.run("draw.inkToShape", &json!({})).unwrap();
    let names: Vec<&str> = r["converted"].as_array().unwrap().iter().filter_map(|c| c["shape"].as_str()).collect();
    assert!(names.contains(&"arrow") && names.contains(&"rectangle"), "{names:?}");
    assert!(ink(&s).is_empty(), "the head stroke went into the arrow");
    let all = drawings(&s);
    assert_eq!(all.len(), 2, "{all:?}");
    let arrow = all.iter().find(|o| matches!(o, InlineObject::Shape { kind: ShapeKind::StraightConnector, .. }));
    let Some(InlineObject::Shape { extra, stroke, .. }) = arrow else { panic!("{all:?}") };
    assert!(extra.arrow_end && *stroke == Some(wordcraft_doc::Rgb(0, 0x70, 0xC0)));
    // The selection still holds what's left: deleting it removes both shapes.
    s.run("draw.lassoDelete", &json!({})).unwrap();
    assert!(drawings(&s).is_empty());
    s.run("edit.undo", &json!({})).unwrap();
    assert_eq!(drawings(&s).len(), 2);
    s.run("edit.undo", &json!({})).unwrap();
    assert_eq!(ink(&s).len(), 3, "one undo step brings all three strokes back");
    assert_eq!(s.undo_depth(), steps);
}

#[test]
fn converting_a_crowded_lasso_selection_recognises_each_stroke_a_bounded_number_of_times() {
    let mut s = session();
    s.run("draw.stroke", &json!({"page": 0, "points": shaft_stroke()})).unwrap();
    s.run("draw.stroke", &json!({"page": 0, "points": [[274.0, 285.0], [300.0, 300.0], [274.0, 315.0]]})).unwrap();
    // A stack of 40 lines close together: every pair is near, none makes an arrow.
    let n = 40;
    for i in 0..n {
        let y = 420.0 + i as f32 * 2.0;
        let pts: Vec<[f32; 2]> = (0..=40).map(|k| [100.0 + k as f32 * 5.0, y]).collect();
        s.run("draw.stroke", &json!({"page": 0, "points": pts})).unwrap();
    }
    s.run("draw.lasso", &json!({"page": 0, "points": [[80, 260], [340, 260], [340, 560], [80, 560]]})).unwrap();
    super::super::ink_shape::RECOGNISED.with(|c| c.set(0));
    let r = s.run("draw.inkToShape", &json!({})).unwrap();
    let calls = super::super::ink_shape::RECOGNISED.with(|c| c.get());
    let strokes = n + 2;
    assert!(calls <= 2 * strokes, "{calls} recognitions for {strokes} strokes");
    let names: Vec<&str> = r["converted"].as_array().unwrap().iter().filter_map(|c| c["shape"].as_str()).collect();
    assert_eq!(names.iter().filter(|n| **n == "arrow").count(), 1, "{names:?}");
    assert_eq!(names.iter().filter(|n| **n == "line").count(), n, "{names:?}");
    assert!(ink(&s).is_empty());
}

#[test]
fn ink_replay_plays_strokes_in_the_order_they_were_drawn() {
    let mut s = session();
    // Drawn low on the page first, then higher: document order would be the other way round.
    s.run("draw.stroke", &json!({"page": 0, "points": [[100, 400], [150, 400]]})).unwrap();
    s.run("draw.stroke", &json!({"page": 0, "points": [[100, 90], [150, 90]]})).unwrap();
    let r = s.run("draw.replay", &json!({})).unwrap();
    assert!(s.view.draw.replay);
    let orders: Vec<u64> = r["strokes"].as_array().unwrap().iter().map(|k| k["order"].as_u64().unwrap()).collect();
    assert_eq!(orders, vec![1, 2]);
    let ys: Vec<f32> = super::strokes_in_drawing_order(&mut s).iter().map(|(_, k)| k.rect.y).collect();
    assert!(ys[0] > ys[1], "the lower stroke was drawn first: {ys:?}");
    assert_eq!(s.run("draw.replay", &json!({})).unwrap()["replaying"], false);
    // Nothing to replay without ink.
    assert!(session().run("draw.replay", &json!({})).is_err());
}

#[test]
fn add_pen_validates_and_saved_galleries_drop_junk() {
    let mut s = session();
    let r = s.run("draw.addPen", &json!({"kind": "highlighter", "color": "00B050", "width": 1e9})).unwrap();
    assert_eq!(r["pen"], json!({"kind": "highlighter", "color": "00B050", "width": super::MAX_WIDTH}));
    assert_eq!(s.view.draw.mode, super::DrawMode::Pen(InkTool::Highlighter), "the new pen is picked up");
    for bad in [json!({"kind": "crayon"}), json!({"color": "blue"}), json!(5), json!(null)] {
        assert!(s.run("draw.addPen", &bad).is_err(), "{bad}");
    }
    for _ in 0..super::MAX_PENS + 3 {
        let _ = s.run("draw.addPen", &json!({}));
    }
    assert_eq!(s.view.draw.pens.len(), super::MAX_PENS);
    s.run("draw.removePen", &json!({"index": 0})).unwrap();
    assert!(s.run("draw.removePen", &json!({"index": 999})).is_err());
    // A saved gallery keeps its pens and drops the junk around them.
    let saved = json!({"pens": [{"kind": "pencil", "color": "7030A0", "width": 3}, "junk", {"kind": "crayon"}, {"width": "x"}], "inkToShape": true});
    let d: super::DrawState = serde_json::from_value(saved).unwrap();
    assert_eq!(d.pens.len(), 2);
    assert_eq!((d.pens[0].kind, d.pens[0].color.hex(), d.pens[0].width), (InkTool::Pencil, "7030A0".to_string(), 3.0));
    assert!(d.ink_to_shape);
    let back: super::DrawState = serde_json::from_value(serde_json::to_value(&d).unwrap()).unwrap();
    assert_eq!(back.pens, d.pens);
}
