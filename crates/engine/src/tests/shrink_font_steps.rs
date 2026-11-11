//! Decrease Font Size steps one point at a time below 8 pt and by 10 pt above 72 pt, as in Word.

use serde_json::json;
use wordcraft_doc::{Document, Pos};

use crate::Session;

fn at(size: f32) -> Session {
    let mut doc = Document::from_text("Sample text");
    let end = doc.para_at(&Pos::body(0, 0)).unwrap().len();
    doc.format_range(&Pos::body(0, 0), &Pos::body(0, end), &|c| {
        c.size = Some(size);
        c.size_cs = Some(size);
    })
    .unwrap();
    let mut s = Session::new(doc);
    s.run("select.all", &json!({})).unwrap();
    s
}

fn size(s: &Session) -> f32 {
    let p = s.doc.para_at(&Pos::body(0, 0)).unwrap();
    s.doc.styles.resolve_char(p.props.style.as_deref(), p.props_of_char(0)).size
}

fn shrink(from: f32) -> f32 {
    let mut s = at(from);
    s.run("format.shrinkFont", &json!({})).unwrap();
    size(&s)
}

fn grow(from: f32) -> f32 {
    let mut s = at(from);
    s.run("format.growFont", &json!({})).unwrap();
    size(&s)
}

#[test]
fn shrink_below_eight_steps_one_point() {
    for (from, to) in [(8.0, 7.0), (7.0, 6.0), (5.0, 4.0), (2.0, 1.0), (1.0, 1.0)] {
        assert_eq!(shrink(from), to, "Decrease Font Size from {from}");
    }
}

#[test]
fn shrink_from_eight_reaches_one_in_seven_steps() {
    let mut s = at(8.0);
    let mut seen = Vec::new();
    for _ in 0..8 {
        s.run("format.shrinkFont", &json!({})).unwrap();
        seen.push(size(&s));
    }
    assert_eq!(seen, [7.0, 6.0, 5.0, 4.0, 3.0, 2.0, 1.0, 1.0]);
}

#[test]
fn shrink_above_seventy_two_steps_ten_points() {
    for (from, to) in [(100.0, 90.0), (90.0, 80.0), (85.0, 80.0), (80.0, 72.0), (75.0, 72.0), (1638.0, 1630.0), (1630.0, 1620.0)] {
        assert_eq!(shrink(from), to, "Decrease Font Size from {from}");
    }
}

#[test]
fn shrink_inside_the_list_is_unchanged() {
    for (from, to) in [(72.0, 48.0), (12.0, 11.0), (9.0, 8.0), (11.0, 10.5)] {
        assert_eq!(shrink(from), to, "Decrease Font Size from {from}");
    }
}

#[test]
fn grow_and_shrink_mirror_above_seventy_two() {
    assert_eq!(grow(72.0), 80.0);
    assert_eq!(grow(80.0), 90.0);
    assert_eq!(shrink(grow(80.0)), 80.0);
    assert_eq!(shrink(grow(72.0)), 72.0);
}
