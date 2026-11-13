//! Word keeps an inline picture and the word right after it (no space between) on one line: the
//! picture is a character of that word. Synthetic shapes only; no external documents.
use super::*;

/// The paragraph's lines, one char per cluster: `O` inline object, `T` text, `_` space, `m`
/// marker. `objects` are (offset, width, height), inserted from the end so offsets stay those of
/// `text`.
pub(super) fn lines(text: &str, objects: &[(usize, f32, f32)], column: Option<f32>) -> Vec<String> {
    let objects: Vec<_> = objects.iter().map(|&(at, w, h)| (at, picture(w, h, Default::default()))).collect();
    lines_of(text, &objects, column)
}

/// [`lines`] with any inline objects (pictures, bookmarks…).
fn lines_of(text: &str, objects: &[(usize, InlineObject)], column: Option<f32>) -> Vec<String> {
    let para = laid_out(text, objects, column);
    para.lines
        .iter()
        .map(|line| {
            let Some(clusters) = para.clusters.get(line.c0..line.c1) else {
                panic!("line clusters {}..{} out of range ({} clusters)", line.c0, line.c1, para.clusters.len())
            };
            clusters
                .iter()
                .filter_map(|c| match c.kind {
                    para::ClKind::Object(_) => Some('O'),
                    para::ClKind::Text => Some('T'),
                    para::ClKind::Space => Some('_'),
                    para::ClKind::Marker => Some('m'),
                    _ => None,
                })
                .collect()
        })
        .collect()
}

/// The laid-out first paragraph of `text` with `objects` inserted (from the end).
fn laid_out(text: &str, objects: &[(usize, InlineObject)], column: Option<f32>) -> std::sync::Arc<para::ParaLayout> {
    crate::test_fonts();
    let mut d = Document::from_text(text);
    if let Some(w) = column {
        d.last_section.page_w = d.last_section.margin_left + d.last_section.margin_right + w;
    }
    for (at, o) in objects.iter().rev() {
        d.insert_object(&Pos::body(0, *at), o.clone(), &Default::default()).unwrap();
    }
    let l = lay(&d);
    let Some(para) = l.pages[0].items.iter().find_map(|i| if let Placed::Lines { para, .. } = i { Some(para.clone()) } else { None }) else {
        panic!("no lines")
    };
    para
}

#[test]
fn a_long_word_right_after_a_picture_starts_on_the_pictures_line() {
    // picture.doc (Apache POI): a 225 pt picture, then a 1,000-letter word. Word fills the rest of
    // the picture's line with the word's first letters and goes on mid-word on the next line.
    let got = lines(&"qwertyuiop".repeat(60), &[(0, 200.0, 50.0)], Some(400.0));
    // 200 pt beside the picture holds 34 letters; a full 400 pt line 68 or 69 (the letters differ
    // in width), and the last line takes the rest of the 600.
    let lens: Vec<_> = got.iter().map(|l| l.len()).collect();
    assert_eq!(lens, [35, 69, 69, 69, 69, 69, 69, 68, 69, 15], "{got:?}");
    assert_eq!(got[0], format!("O{}", "T".repeat(34)));
    assert!(got[1..].iter().all(|l| l.chars().all(|c| c == 'T')), "{got:?}");
}

#[test]
fn a_picture_moves_down_with_the_word_it_touches() {
    // A 150 pt picture, a space, a 40 pt picture glued to "Glued", in a 200 pt column: the second
    // picture fits on line 1 but its word doesn't, so both go to line 2.
    let got = lines(" Glued", &[(0, 150.0, 20.0), (1, 40.0, 20.0)], Some(200.0));
    assert_eq!(got, ["O_", "OTTTTT"], "{got:?}");
}

#[test]
fn a_space_after_a_picture_is_still_a_place_to_wrap() {
    let got = lines("  Glued", &[(0, 150.0, 20.0), (1, 40.0, 20.0)], Some(200.0));
    assert_eq!(got, ["O_O_", "TTTTT"], "{got:?}");
}

#[test]
fn a_bookmark_between_a_picture_and_its_word_stays_with_them() {
    // A bookmark start between the 40 pt picture and "Glued" takes no room and no break: the
    // picture, the bookmark and the word go to line 2 together.
    let mark = InlineObject::BookmarkStart { name: "glued".into() };
    let objects = [(0, picture(150.0, 20.0, Default::default())), (1, picture(40.0, 20.0, Default::default())), (1, mark)];
    let got = lines_of(" Glued", &objects, Some(200.0));
    assert_eq!(got, ["O_", "OmTTTTT"], "{got:?}");
}

#[test]
fn no_break_after_a_picture_or_a_marker_before_its_word() {
    let mark = InlineObject::BookmarkStart { name: "glued".into() };
    let para = laid_out("Glued", &[(0, picture(40.0, 20.0, Default::default())), (0, mark)], None);
    let got: Vec<_> = para.clusters.iter().map(|c| (c.kind, c.break_after)).collect();
    assert!(matches!(got.first(), Some((para::ClKind::Object(_), false))), "{got:?}");
    assert!(matches!(got.get(1), Some((para::ClKind::Marker, false))), "{got:?}");
    assert!(matches!(got.get(2), Some((para::ClKind::Text, false))), "{got:?}");
}

#[test]
fn a_zero_size_object_keeps_its_break() {
    // A hidden picture takes no room on the line (a 0x0 cluster, like a floating object's anchor):
    // it isn't part of the word after it, so the word wraps alone and the object stays on line 1.
    let hidden = wordcraft_doc::para::Float { hidden: true, ..Default::default() };
    let objects = [(0, picture(175.0, 20.0, Default::default())), (1, picture(40.0, 20.0, hidden))];
    let got = lines_of(" Glued", &objects, Some(200.0));
    assert_eq!(got, ["O_O", "TTTTT"], "{got:?}");
}

// An inline shape is laid out at least 1 pt each way, so the next two lay out as 40x1 and 1x20;
// `glue_runs_of_sized_objects_before_text` checks the exact one-dimension clusters.

#[test]
fn a_picture_with_only_a_width_is_glued_to_its_word() {
    let got = lines(" Glued", &[(0, 150.0, 20.0), (1, 40.0, 0.0)], Some(200.0));
    assert_eq!(got, ["O_", "OTTTTT"], "{got:?}");
}

#[test]
fn a_picture_with_only_a_height_is_glued_to_its_word() {
    // Next to no width, so it would fit on line 1 after the 175 pt picture; "Glued" doesn't,
    // and the picture goes down with it.
    let got = lines(" Glued", &[(0, 175.0, 20.0), (1, 0.0, 20.0)], Some(200.0));
    assert_eq!(got, ["O_", "OTTTTT"], "{got:?}");
}

/// Clusters from a pattern: `O` a 20x20 object, `w` a 20x0 one, `h` a 0x20 one, `z` a 0x0 one,
/// `m` a marker, `T` text, `_` a space; every one a break opportunity to start with.
fn clusters(pattern: &str) -> Vec<para::Cluster> {
    pattern
        .chars()
        .enumerate()
        .map(|(i, ch)| {
            let (kind, adv, obj_h) = match ch {
                'O' => (para::ClKind::Object(i), 20.0, 20.0),
                'w' => (para::ClKind::Object(i), 20.0, 0.0),
                'h' => (para::ClKind::Object(i), 0.0, 20.0),
                'z' => (para::ClKind::Object(i), 0.0, 0.0),
                'm' => (para::ClKind::Marker, 0.0, 0.0),
                '_' => (para::ClKind::Space, 3.0, 0.0),
                _ => (para::ClKind::Text, 6.0, 0.0),
            };
            para::Cluster { start: i, end: i + 1, adv, kind, style: 0, g0: 0, g1: 0, break_after: true, obj_h, obj_d: 0.0, dot: false }
        })
        .collect()
}

/// The clusters of `pattern` left without a break after them, as `-`, the others as `|`.
fn glued(pattern: &str) -> String {
    let mut cs = clusters(pattern);
    para::glue_objects(&mut cs);
    cs.iter().map(|c| if c.break_after { '|' } else { '-' }).collect()
}

#[test]
fn glue_runs_of_sized_objects_before_text() {
    // Object, marker, text: neither the object nor the marker keeps a break.
    assert_eq!(glued("OmT"), "--|");
    // Touching objects, markers among them, go with the word as one unit.
    assert_eq!(glued("OOT"), "--|");
    assert_eq!(glued("OmOmT"), "----|");
    // One dimension is enough to count as sized.
    assert_eq!(glued("wT"), "-|");
    assert_eq!(glued("hT"), "-|");
    assert_eq!(glued("whT"), "--|");
    // A 0x0 object, a space or a marker before the run ends it; markers before it keep theirs.
    assert_eq!(glued("zT"), "||");
    assert_eq!(glued("OzOT"), "||-|");
    assert_eq!(glued("O_OT"), "||-|");
    assert_eq!(glued("mOT"), "|-|");
    // No text after: nothing is glued; text before a picture isn't glued to it.
    assert_eq!(glued("OO_"), "|||");
    assert_eq!(glued("TOT"), "|-|");
}
