//! Draw › Pens › Action Pen: the natural gestures, driven through the public commands.
//!
//! Each gesture is checked three ways: the document text it leaves, one undo step back to what
//! there was before, and a DOCX save and reopen with the text kept and no ink left behind.
//! Track Changes records each gesture's edit as a revision. The interaction test at the UI level
//! draws the same strokes with the pointer (`crates/ui-egui/src/tests.rs`).

use serde_json::json;
use wordcraft_doc::para::InlineObject;
use wordcraft_doc::{Document, Path, Pos, StoryRef};

use crate::Session;
use crate::cmd::draw::action_pen::{self, PageText};

fn session(text: &str) -> Session {
    let mut s = Session::new(Document::new());
    s.run("document.setText", &json!({"text": text})).unwrap();
    s
}

/// The characters of page 0 as they are laid out.
fn text_map(s: &mut Session) -> PageText {
    let layout = s.layout();
    action_pen::page_text(s, &layout, 0)
}

fn cells(map: &PageText) -> Vec<wordcraft_geom::Rect> {
    map.rows.first().map(|r| r.cells.iter().map(|c| c.rect).collect()).unwrap_or_default()
}

/// The box covering characters `from..to` of the first line.
fn word_box(map: &PageText, from: usize, to: usize) -> wordcraft_geom::Rect {
    let c = cells(map);
    let (Some(a), Some(b)) = (c.get(from), c.get(to.saturating_sub(1))) else { panic!("no characters {from}..{to}") };
    a.union(b)
}

/// A hand-drawn loop around `r`.
fn around(r: wordcraft_geom::Rect) -> Vec<[f32; 2]> {
    (0..=40)
        .map(|i| {
            let t = i as f32 / 40.0 * std::f32::consts::TAU;
            [r.x + r.w / 2.0 + (r.w / 2.0 + 5.0) * t.cos(), r.y + r.h / 2.0 + (r.h / 2.0 + 5.0) * t.sin()]
        })
        .collect()
}

fn across(x0: f32, x1: f32, y: f32) -> Vec<[f32; 2]> {
    (0..=20).map(|i| [x0 + (x1 - x0) * i as f32 / 20.0, y]).collect()
}

fn down(x: f32, y0: f32, y1: f32) -> Vec<[f32; 2]> {
    (0..=20).map(|i| [x, y0 + (y1 - y0) * i as f32 / 20.0]).collect()
}

/// A fresh scratch directory for the file round trips.
fn scratch(tag: &str) -> std::path::PathBuf {
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let dir = std::env::temp_dir().join(format!("wordcraft-action-pen-{tag}-{}-{nanos}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    dir
}

/// Save the document, open it again, and check the text and that no ink came along.
fn round_trip(s: &mut Session, tag: &str, expect: &str) {
    let dir = scratch(tag);
    let path = dir.join("gesture.docx");
    s.run("file.saveAs", &json!({"path": path.to_string_lossy()})).unwrap();
    let mut opened = Session::new(Document::new());
    opened.run("file.open", &json!({"path": path.to_string_lossy()})).unwrap();
    assert_eq!(opened.doc.plain_text(StoryRef::Body), expect, "{tag}: the saved file keeps the gesture's text");
    assert!(super::ink(&opened).is_empty(), "{tag}: no ink is left in the saved file");
    let _ = std::fs::remove_dir_all(&dir);
}

fn word_offsets(s: &Session) -> (usize, usize) {
    (s.sel.anchor.off.min(s.sel.focus.off), s.sel.anchor.off.max(s.sel.focus.off))
}

#[test]
fn circle_select_is_a_normal_selection_formatting_uses() {
    let mut s = session("alpha beta gamma");
    let r = word_box(&text_map(&mut s), 0, 5);
    let pts = around(r.expand(4.0));
    let out = s.run("draw.actionPenStroke", &json!({"page": 0, "points": pts})).unwrap();
    assert_eq!(out["gesture"], json!("select"), "{out}");
    assert_eq!(word_offsets(&s), (0, 5), "the word alpha");
    assert_eq!(s.sel.anchor.path, Path::top(0));
    // A normal selection: Home › Bold bolds exactly it.
    s.run("format.bold", &json!({})).unwrap();
    let para = s.doc.para(StoryRef::Body, &Path::top(0)).unwrap();
    let bold: usize = para.runs.iter().filter(|r| r.props.bold == Some(true)).map(|r| r.len).sum();
    assert_eq!(bold, 5, "bold covers the five selected characters: {:?}", para.runs);
    assert!(super::ink(&s).is_empty());
    // Undo, then the gesture again (the two steps undo in order).
    s.run("edit.undo", &json!({})).unwrap();
    s.run("edit.undo", &json!({})).unwrap();
    assert!(s.sel.is_collapsed());
    assert!(!s.doc.para(StoryRef::Body, &Path::top(0)).unwrap().runs.iter().any(|r| r.props.bold == Some(true)));
    s.run("draw.actionPenStroke", &json!({"page": 0, "points": pts})).unwrap();
    s.run("format.bold", &json!({})).unwrap();
    round_trip(&mut s, "circle", "alpha beta gamma");
    let bold: usize = s.doc.para(StoryRef::Body, &Path::top(0)).unwrap().runs.iter().filter(|r| r.props.bold == Some(true)).map(|r| r.len).sum();
    assert_eq!(bold, 5, "the saved selection's formatting is still there");
}

#[test]
fn a_line_through_text_deletes_what_it_crosses_with_one_undo_and_survives_a_save() {
    let c = cells(&text_map(&mut session("alpha beta gamma")));
    let y = (c[0].y + c[0].h / 2.0).round();
    let pts = across(c[0].x - 2.0, c[4].right() + 2.0, y);
    gesture_deletes(pts, " beta gamma", "line");
}

#[test]
fn a_diagonal_through_text_deletes_what_it_crosses() {
    let c = cells(&text_map(&mut session("alpha beta gamma")));
    let pts: Vec<[f32; 2]> = (0..=20)
        .map(|i| {
            let t = i as f32 / 20.0;
            [c[0].x + (c[4].right() - c[0].x) * t, c[0].y + 1.0 + (c[0].h - 2.0) * t]
        })
        .collect();
    // A diagonal crosses the middle of the line's band, so only the letters it passes through go.
    gesture_deletes(pts, "aa beta gamma", "diagonal");
}

#[test]
fn a_squiggle_through_text_deletes_what_it_crosses() {
    let c = cells(&text_map(&mut session("alpha beta gamma")));
    let y = c[0].y + c[0].h / 2.0;
    let pts: Vec<[f32; 2]> = (0..=60)
        .map(|i| {
            let t = i as f32 / 60.0;
            [c[0].x + (c[9].right() - c[0].x) * t, y + (t * 12.0).sin() * 2.5]
        })
        .collect();
    gesture_deletes(pts, " gamma", "squiggle");
}

/// Run a deletion gesture: the text it leaves, no ink, one undo step, and a save and reopen.
fn gesture_deletes(pts: Vec<[f32; 2]>, expect: &str, tag: &str) {
    let mut s = session("alpha beta gamma");
    let before = s.doc.plain_text(StoryRef::Body);
    let out = s.run("draw.actionPenStroke", &json!({"page": 0, "points": pts})).unwrap();
    assert_eq!(out["gesture"], json!("delete"), "{tag}: {out}");
    assert_eq!(s.doc.plain_text(StoryRef::Body), expect, "{tag}");
    assert!(super::ink(&s).is_empty(), "{tag}: no ink left");
    // One undo step per gesture.
    s.run("edit.undo", &json!({})).unwrap();
    assert_eq!(s.doc.plain_text(StoryRef::Body), before, "{tag}: one undo step");
    // Again, and through a file.
    s.run("draw.actionPenStroke", &json!({"page": 0, "points": pts})).unwrap();
    assert_eq!(s.doc.plain_text(StoryRef::Body), expect);
    round_trip(&mut s, tag, expect);
}

#[test]
fn a_curve_touching_two_words_joins_them_and_keeps_fields_and_runs() {
    let mut s = session("alpha beta gamma");
    // A field after the words: the join must leave it where it is.
    s.run("caret.docEnd", &json!({})).unwrap();
    s.run("insert.field", &json!({"instr": "PAGE"})).unwrap();
    let before = s.doc.plain_text(StoryRef::Body);
    let c = cells(&text_map(&mut s));
    // From inside "alpha", sagging below the line, up into "beta": a curve touching both.
    let pts: Vec<[f32; 2]> = (0..=30)
        .map(|i| {
            let t = i as f32 / 30.0;
            let x = c[2].x + (c[7].x - c[2].x) * t;
            [x, c[0].bottom() + 8.0 + (t * std::f32::consts::PI).sin() * 14.0]
        })
        .collect();
    let out = s.run("draw.actionPenStroke", &json!({"page": 0, "points": pts})).unwrap();
    assert_eq!(out["gesture"], json!("join"), "{out}");
    assert_eq!(s.doc.plain_text(StoryRef::Body), before.replacen("alpha beta", "alphabeta", 1));
    assert_eq!(fields(&s), 1, "the field is still there");
    // Undo brings back the space and the same field.
    s.run("edit.undo", &json!({})).unwrap();
    assert_eq!(s.doc.plain_text(StoryRef::Body), before);
    assert_eq!(fields(&s), 1);
    s.run("draw.actionPenStroke", &json!({"page": 0, "points": pts})).unwrap();
    round_trip(&mut s, "join", &before.replacen("alpha beta", "alphabeta", 1));
}

#[test]
fn a_vertical_line_inside_a_word_splits_it() {
    let mut s = session("alpha beta");
    let c = cells(&text_map(&mut s));
    let x = (c[2].right() + c[3].x) / 2.0;
    let pts = down(x, c[0].y - 1.0, c[0].bottom() + 1.0);
    let out = s.run("draw.actionPenStroke", &json!({"page": 0, "points": pts})).unwrap();
    assert_eq!(out["gesture"], json!("split"), "{out}");
    assert_eq!(s.doc.plain_text(StoryRef::Body), "alp ha beta");
    s.run("edit.undo", &json!({})).unwrap();
    assert_eq!(s.doc.plain_text(StoryRef::Body), "alpha beta");
    // A field after the word is kept, and the split is one undo step.
    s.run("caret.docEnd", &json!({})).unwrap();
    s.run("insert.field", &json!({"instr": "PAGE"})).unwrap();
    let before = s.doc.plain_text(StoryRef::Body);
    s.run("draw.actionPenStroke", &json!({"page": 0, "points": pts})).unwrap();
    let after = before.replacen("alpha", "alp ha", 1);
    assert_eq!(s.doc.plain_text(StoryRef::Body), after);
    assert_eq!(fields(&s), 1);
    round_trip(&mut s, "split", &after);
}

#[test]
fn a_caret_between_words_puts_the_caret_there_and_typing_inserts_the_words() {
    let mut s = session("alpha beta gamma");
    let c = cells(&text_map(&mut s));
    let gap_x = (c[4].right() + c[6].x) / 2.0;
    let y = c[0].bottom() + 8.0;
    let pts: Vec<[f32; 2]> = (0..=10)
        .map(|i| {
            let t = i as f32 / 10.0;
            [gap_x - 9.0 + 9.0 * t, c[0].y - 3.0 + (y - c[0].y + 3.0) * t]
        })
        .chain((1..=10).map(|i| {
            let t = i as f32 / 10.0;
            [gap_x + 9.0 * t, y - (y - c[0].y + 3.0) * t]
        }))
        .collect();
    let out = s.run("draw.actionPenStroke", &json!({"page": 0, "points": pts})).unwrap();
    assert_eq!(out["gesture"], json!("insert"), "{out}");
    assert!(s.sel.is_collapsed(), "a caret, not a selection");
    assert_eq!(s.sel.focus.off, 6, "before the next word, past the gap's space");
    // The ordinary typing path (what the keyboard and IME do) inserts the words.
    s.run("text.insert", &json!({"text": "new "})).unwrap();
    assert_eq!(s.doc.plain_text(StoryRef::Body), "alpha new beta gamma");
    s.run("edit.undo", &json!({})).unwrap();
    s.run("edit.undo", &json!({})).unwrap();
    assert_eq!(s.doc.plain_text(StoryRef::Body), "alpha beta gamma");
    // A recogniser seam can hand the words in with the gesture: one undo step for both, with no
    // model or network in the build.
    let out = s.run("draw.actionPenStroke", &json!({"page": 0, "points": pts, "text": "new "})).unwrap();
    assert_eq!(out["gesture"], json!("insert"));
    assert_eq!(s.doc.plain_text(StoryRef::Body), "alpha new beta gamma");
    assert!(super::ink(&s).is_empty());
    s.run("edit.undo", &json!({})).unwrap();
    assert_eq!(s.doc.plain_text(StoryRef::Body), "alpha beta gamma", "gesture and text are one step");
    s.run("draw.actionPenStroke", &json!({"page": 0, "points": pts, "text": "new "})).unwrap();
    round_trip(&mut s, "caret", "alpha new beta gamma");
}

#[test]
fn a_backwards_l_moves_the_following_text_to_a_new_line() {
    let mut s = session("alpha beta gamma");
    let c = cells(&text_map(&mut s));
    let x = (c[4].right() + c[6].x) / 2.0;
    let mut pts = down(x, c[0].y - 6.0, c[0].bottom() + 10.0);
    pts.extend(across(x, x - 26.0, c[0].bottom() + 10.0).into_iter().skip(1));
    let out = s.run("draw.actionPenStroke", &json!({"page": 0, "points": pts})).unwrap();
    assert_eq!(out["gesture"], json!("newLine"), "{out}");
    assert_eq!(s.doc.plain_text(StoryRef::Body), "alpha \nbeta gamma");
    s.run("edit.undo", &json!({})).unwrap();
    assert_eq!(s.doc.plain_text(StoryRef::Body), "alpha beta gamma");
    s.run("draw.actionPenStroke", &json!({"page": 0, "points": pts})).unwrap();
    round_trip(&mut s, "newline", "alpha \nbeta gamma");
}

/// Track Changes: each gesture's edit is a revision the review commands can accept or reject.
#[test]
fn tracked_gestures_record_revisions() {
    let mut s = session("alpha beta gamma");
    s.run("review.trackChanges", &json!({"value": true})).unwrap();
    let c = cells(&text_map(&mut s));
    let y = c[0].y + c[0].h / 2.0;
    let pts = across(c[0].x - 2.0, c[4].right() + 2.0, y);
    let out = s.run("draw.actionPenStroke", &json!({"page": 0, "points": pts})).unwrap();
    assert_eq!(out["gesture"], json!("delete"));
    assert!(!s.doc.revisions.is_empty(), "the deletion is a revision");
    assert_eq!(s.doc.final_text(StoryRef::Body), " beta gamma", "it reads as deleted");
    assert_eq!(s.doc.plain_text(StoryRef::Body), "alpha beta gamma", "the text is still there to accept or reject");
    s.run("edit.undo", &json!({})).unwrap();
    assert!(s.doc.revisions.is_empty(), "one undo step takes the revision back");
    assert!(s.doc.settings.track_changes, "the setting is not the edit");
    // Split under tracking inserts its space as a tracked insertion.
    s.run("draw.actionPenStroke", &json!({"page": 0, "points": pts})).unwrap();
    s.run("review.acceptAll", &json!({})).unwrap();
    assert_eq!(s.doc.final_text(StoryRef::Body), " beta gamma", "accepted: alpha is gone");
    assert!(s.doc.revisions.is_empty());
    let c = cells(&text_map(&mut s));
    let x = (c[1].right() + c[2].x) / 2.0;
    let split = down(x, c[0].y - 1.0, c[0].bottom() + 1.0);
    s.run("draw.actionPenStroke", &json!({"page": 0, "points": split})).unwrap();
    assert!(!s.doc.revisions.is_empty(), "the split's space is a revision");
    s.run("review.rejectAll", &json!({})).unwrap();
    assert_eq!(s.doc.final_text(StoryRef::Body), " beta gamma", "rejecting the split takes the space out");
}

/// Fields in the body: how many inline field objects the first paragraph holds.
fn fields(s: &Session) -> usize {
    s.doc.para(StoryRef::Body, &Path::top(0)).map_or(0, |p| p.objects.iter().filter(|o| matches!(o, InlineObject::Field { .. })).count())
}

/// A tiny, hostile stroke never edits anything and never panics.
#[test]
fn tiny_ambiguous_and_hostile_strokes_stay_ink_or_fail_cleanly() {
    let mut s = session("alpha beta");
    let before = s.doc.plain_text(StoryRef::Body);
    // Too small: ink stays ink.
    let out = s.run("draw.actionPenStroke", &json!({"page": 0, "points": [[100, 100], [101, 100]]})).unwrap();
    assert_eq!(out["gesture"], json!("ink"), "{out}");
    assert_eq!(super::ink(&s).len(), 1, "an unrecognised stroke is drawn as ink");
    assert_eq!(s.doc.plain_text(StoryRef::Body), before, "and changes no text");
    s.run("edit.undo", &json!({})).unwrap();
    assert!(super::ink(&s).is_empty());
    // Non-finite or malformed points never panic.
    assert!(s.run("draw.actionPenStroke", &json!({"page": 0, "points": [[1e30, 1e30], [f64::NAN, 2], ["x", 3]]})).is_ok());
    s.run("edit.undo", &json!({})).ok();
    // A stroke on no page is refused.
    assert!(s.run("draw.actionPenStroke", &json!({"page": 99, "points": [[1, 1], [2, 2]]})).is_err());
    // The caret command toggles the tool.
    assert_eq!(s.run("draw.actionPen", &json!({})).unwrap()["value"], json!(true));
    assert_eq!(s.view.draw.mode, crate::cmd::draw::DrawMode::ActionPen);
    assert_eq!(s.run("draw.actionPen", &json!({})).unwrap()["value"], json!(false));
    assert_eq!(s.view.draw.mode, crate::cmd::draw::DrawMode::Select);
    // Pen and stroke behaviour is untouched by the Action Pen.
    s.run("draw.pen", &json!({})).unwrap();
    s.run("draw.stroke", &json!({"page": 0, "points": [[80, 80], [200, 90]]})).unwrap();
    assert_eq!(super::ink(&s).len(), 1);
    assert_eq!((s.sel.anchor.off, s.sel.focus.off), (0, 0), "a pen stroke selects nothing");
    let _ = Pos::body(0, 0);
}

/// The gesture map builds only the lines a stroke's vertical band can reach: on a page of
/// several paragraphs the map holds the one the stroke is drawn on, so the count cap can't leave
/// the text under the stroke out of a dense page.
#[test]
fn the_gesture_map_keeps_only_the_lines_near_the_stroke() {
    let mut s = session("alpha\n\nbeta\n\ngamma");
    let layout = s.layout();
    let full = action_pen::page_text(&s, &layout, 0);
    assert_eq!(full.rows.len(), 3, "three paragraphs are laid out");
    let second = full.rows.get(1).unwrap().clone();
    let near = action_pen::page_text_near(&s, &layout, 0, second.top, second.bottom);
    assert_eq!(near.rows.len(), 1, "only the paragraph the band holds");
    let row = near.rows.first().unwrap();
    let (first, last) = (second.cells.first().unwrap().pos.off, second.cells.last().unwrap().end);
    assert!(row.cells.iter().all(|c| c.pos.off >= first && c.end <= last), "beta's characters only: {:?}", row.cells.first().map(|c| c.pos.off));
    // The live preview maps the same band and still reads the stroke.
    let cells = cells(&near);
    let (a, b) = (cells.first().unwrap(), cells.get(2).unwrap());
    let stroke = across(a.x - 1.0, b.right() + 1.0, (a.y + b.bottom()) / 2.0);
    assert_eq!(action_pen::preview(&s, &layout, 0, &stroke).map(|g| g.name()), Some("delete"));
    // A stroke drawn far below every line reaches none of them.
    let below = full.rows.last().map_or(0.0, |r| r.bottom);
    assert!(action_pen::preview(&s, &layout, 0, &across(100.0, 200.0, below + 200.0)).is_none());
}
