use serde_json::json;
use wordcraft_doc::Document;
use wordcraft_layout::display::{DisplayOptions, Draw, page_display};

use crate::Session;

/// What the first page draws with the session's Track Changes Options.
fn draws(s: &mut Session) -> (Vec<Draw>, f32) {
    let l = s.layout();
    let page = &l.pages[0];
    let opts = DisplayOptions { revisions: s.prefs.markup.clone(), ..Default::default() };
    (page_display(&s.doc, page, &opts), page.body.x)
}

fn glyph_colors(d: &[Draw]) -> Vec<(wordcraft_doc::Rgb, bool)> {
    d.iter().filter_map(|x| if let Draw::Glyphs { color, synth_bold, .. } = x { Some((*color, *synth_bold)) } else { None }).collect()
}

fn lines(d: &[Draw]) -> Vec<(f32, f32, f32, f32)> {
    d.iter().filter_map(|x| if let Draw::Line { x0, y0, x1, y1, .. } = x { Some((*x0, *y0, *x1, *y1)) } else { None }).collect()
}

/// #328: Track Changes Options change how an insertion and a deletion are drawn, the bar
/// beside changed lines, and whether deletions take room.
#[test]
fn tracking_options_change_how_revisions_are_drawn() {
    let mut s = Session::new(Document::new());
    // No proofing squiggles among the lines.
    s.view.proofing = false;
    s.run("document.setText", &json!({"text": "keep gone"})).unwrap();
    s.run("review.trackChanges", &json!({"value": true})).unwrap();
    s.run("caret.docStart", &json!({})).unwrap();
    s.run("text.insert", &json!({"text": "new "})).unwrap();
    s.run("select.range", &json!({"anchor": {"block": 0, "off": 9}, "focus": {"block": 0, "off": 13}})).unwrap();
    s.run("text.delete", &json!({})).unwrap();

    // Defaults: underlined insertion and struck deletion in the author's colour, a bar left of
    // the text.
    let (d, body_x) = draws(&mut s);
    let bars: Vec<_> = lines(&d).into_iter().filter(|(x0, _, x1, _)| x0 == x1 && *x0 < body_x).collect();
    assert_eq!(bars.len(), 1, "one changed line, one bar in the left margin");
    let horizontal = lines(&d).into_iter().filter(|(_, y0, _, y1)| y0 == y1).count();
    assert_eq!(horizontal, 2, "an underline and a strikethrough: {:?}", lines(&d));
    let author = glyph_colors(&d).iter().filter(|(c, _)| *c != wordcraft_doc::Rgb::BLACK).count();
    assert!(author >= 2, "insertion and deletion in the author's colour");

    // Bold, fixed-colour insertions; hidden deletions; no bars.
    let r = s
        .run(
            "review.trackingOptions",
            &json!({"insertMark": "bold", "insertColor": "00AA00", "deleteMark": "hidden", "changedLines": "none", "balloons": "inline"}),
        )
        .unwrap();
    assert_eq!((r["insertMark"].as_str(), r["deleteColor"].as_str(), r["balloons"].as_str()), (Some("bold"), Some("byAuthor"), Some("inline")));
    let (d, _) = draws(&mut s);
    assert!(lines(&d).is_empty(), "no underline, no strikethrough, no bar: {:?}", lines(&d));
    assert!(glyph_colors(&d).contains(&(wordcraft_doc::Rgb(0, 0xAA, 0), true)), "a bold green insertion");
    let text: String = d.iter().filter_map(|x| if let Draw::Glyphs { text, .. } = x { Some(text.as_str()) } else { None }).collect();
    assert!(!text.contains("gone"), "the deletion takes no room: {text:?}");

    // Unknown choices are refused; reading changes nothing.
    assert!(s.run("review.trackingOptions", &json!({"deleteMark": "sparkles"})).is_err());
    assert_eq!(s.run("review.trackingOptions", &json!({})).unwrap()["insertColor"], "00AA00");
}

/// #1055: comments show in the margin as in Word unless the option says otherwise.
#[test]
fn comment_display_defaults_to_margin_and_is_validated() {
    let mut s = Session::new(wordcraft_doc::Document::new());
    assert_eq!(s.run("review.trackingOptions", &json!({})).unwrap()["commentDisplay"], "margin");
    assert_eq!(s.run("review.trackingOptions", &json!({"commentDisplay": "selected"})).unwrap()["commentDisplay"], "selected");
    assert_eq!(s.run("review.trackingOptions", &json!({})).unwrap()["commentDisplay"], "selected");
    assert!(s.run("review.trackingOptions", &json!({"commentDisplay": "never"})).is_err());
    assert_eq!(s.run("review.trackingOptions", &json!({"commentDisplay": "margin"})).unwrap()["commentDisplay"], "margin");
}

/// Gap #5 (Track Changes Options › Moves): moved text is drawn with the Moved from / Moved
/// to marks and colours, by default green with a double strikethrough where it was and a
/// double underline where it went.
#[test]
fn move_marks_and_colours_change_how_moved_text_is_drawn() {
    use wordcraft_doc::Rgb;
    use wordcraft_layout::display::{MOVE_COLOR, Stroke};
    let mut s = Session::new(Document::new());
    s.view.proofing = false;
    s.run("document.setText", &json!({"text": "First one.\nSecond one.\nThird one."})).unwrap();
    s.run("review.trackChanges", &json!({"value": true})).unwrap();
    s.run("select.range", &json!({"anchor": {"block": 1, "off": 0}, "focus": {"block": 2, "off": 0}})).unwrap();
    s.run("edit.cut", &json!({})).unwrap();
    s.run("caret.set", &json!({"pos": {"block": 0, "off": 0}})).unwrap();
    s.run("edit.paste", &json!({})).unwrap();
    // Horizontal lines (underlines, strikethroughs) with their colour and stroke.
    let strokes = |d: &[Draw]| -> Vec<(Rgb, Stroke)> {
        d.iter()
            .filter_map(|x| if let Draw::Line { y0, y1, color, stroke, .. } = x { (y0 == y1).then_some((*color, *stroke)) } else { None })
            .collect()
    };

    let r = s.run("review.trackingOptions", &json!({})).unwrap();
    assert_eq!(
        (r["moveFromMark"].as_str(), r["moveFromColor"].as_str(), r["moveToMark"].as_str(), r["moveToColor"].as_str()),
        (Some("doubleStrikethrough"), Some(MOVE_COLOR.hex().as_str()), Some("doubleUnderline"), Some(MOVE_COLOR.hex().as_str())),
    );
    let (d, _) = draws(&mut s);
    let st = strokes(&d);
    assert_eq!(st.iter().filter(|(c, k)| *c == MOVE_COLOR && *k == Stroke::Double).count(), 2, "green double underline and strikethrough: {st:?}");
    assert!(glyph_colors(&d).contains(&(MOVE_COLOR, false)));

    // Red single strikethrough where it was; bold blue where it went.
    let r = s
        .run(
            "review.trackingOptions",
            &json!({"moveFromMark": "strikethrough", "moveFromColor": "FF0000", "moveToMark": "bold", "moveToColor": "0000FF"}),
        )
        .unwrap();
    assert_eq!((r["moveFromMark"].as_str(), r["moveToColor"].as_str()), (Some("strikethrough"), Some("0000FF")));
    let (d, _) = draws(&mut s);
    let st = strokes(&d);
    assert_eq!(st, vec![(Rgb(0xFF, 0, 0), Stroke::Solid)], "only a red strikethrough is left");
    let g = glyph_colors(&d);
    assert!(g.contains(&(Rgb(0, 0, 0xFF), true)) && g.contains(&(Rgb(0xFF, 0, 0), false)), "{g:?}");
    assert!(!g.iter().any(|(c, _)| *c == MOVE_COLOR), "{g:?}");

    // By author: the insertion and deletion colour of the author.
    s.run("review.trackingOptions", &json!({"moveToColor": "byAuthor"})).unwrap();
    let (d, _) = draws(&mut s);
    let author = wordcraft_layout::display::revision_color(0);
    assert!(glyph_colors(&d).contains(&(author, true)), "{:?}", glyph_colors(&d));

    // Unknown choices are refused.
    assert!(s.run("review.trackingOptions", &json!({"moveToMark": "sparkles"})).is_err());
    assert!(s.run("review.trackingOptions", &json!({"moveFromColor": "green"})).is_err());
}

/// Three paragraphs; with Track Changes on, the second cut and pasted before the first, then
/// the Track Changes Options `opts` applied.
fn moved_with(opts: serde_json::Value) -> Session {
    let mut s = Session::new(Document::new());
    s.view.proofing = false;
    s.run("document.setText", &json!({"text": "First one.\nSecond one.\nThird one."})).unwrap();
    s.run("review.trackChanges", &json!({"value": true})).unwrap();
    s.run("select.range", &json!({"anchor": {"block": 1, "off": 0}, "focus": {"block": 2, "off": 0}})).unwrap();
    s.run("edit.cut", &json!({})).unwrap();
    s.run("caret.set", &json!({"pos": {"block": 0, "off": 0}})).unwrap();
    s.run("edit.paste", &json!({})).unwrap();
    s.run("review.trackingOptions", &opts).unwrap();
    s
}

/// The caret at body paragraph `block`, offset `off`: (x, top).
fn caret_at(s: &mut Session, block: usize, off: usize) -> (f32, f32) {
    let c = s.layout().caret(&wordcraft_doc::Pos::body(block, off)).unwrap();
    (c.x, c.top)
}

/// Review of gap #5: Moved from = Hidden takes the moved-away paragraph out of the layout,
/// as Hidden does for deletions: the text below closes up as if the move were accepted, the
/// caret can't land inside it, and nothing of it is drawn, not even its ¶.
#[test]
fn hidden_moved_from_text_takes_no_room() {
    let mut accepted = moved_with(json!({}));
    accepted.run("review.acceptAll", &json!({})).unwrap();
    let third_accepted = caret_at(&mut accepted, 2, 0).1;

    let mut s = moved_with(json!({"moveFromMark": "hidden"}));
    assert_eq!(s.doc.body.len(), 4, "the moved-away paragraph is still in the document");
    assert!((caret_at(&mut s, 3, 0).1 - third_accepted).abs() < 0.01, "no blank line where it was");
    // Inside it is the same place as its start and as the paragraph after it.
    assert_eq!(caret_at(&mut s, 2, 5), caret_at(&mut s, 2, 0));
    assert_eq!(caret_at(&mut s, 2, 0), caret_at(&mut s, 3, 0));
    let (d, _) = draws(&mut s);
    assert_eq!(glyph_text(&d).matches("Second").count(), 1, "{:?}", glyph_text(&d));
    // Formatting marks shown: one ¶ per visible paragraph.
    let l = s.layout();
    let opts = DisplayOptions { marks: true, revisions: s.prefs.markup.clone(), ..Default::default() };
    let d = page_display(&s.doc, &l.pages[0], &opts);
    assert_eq!(d.iter().filter(|x| matches!(x, Draw::Mark { ch: '¶', .. })).count(), 3);
}

/// Each "Second" run drawn, in page order (the moved-to paragraph first, then the moved-from
/// one): (colour, bold, italic).
fn seconds(d: &[Draw]) -> Vec<(wordcraft_doc::Rgb, bool, bool)> {
    d.iter()
        .filter_map(|x| match x {
            Draw::Glyphs { text, color, synth_bold, synth_italic, .. } if text.starts_with("Second") => Some((*color, *synth_bold, *synth_italic)),
            _ => None,
        })
        .collect()
}

/// Horizontal lines (underlines and strikethroughs): their strokes.
fn h_strokes(d: &[Draw]) -> Vec<wordcraft_layout::display::Stroke> {
    d.iter().filter_map(|x| if let Draw::Line { y0, y1, stroke, .. } = x { (y0 == y1).then_some(*stroke) } else { None }).collect()
}

/// Change bars: vertical lines left of the body.
fn bars(d: &[Draw], body_x: f32) -> usize {
    lines(d).into_iter().filter(|(x0, _, x1, _)| x0 == x1 && *x0 < body_x).count()
}

/// Gap #5, Word for Mac 16.113.4 (2026-10-15): Moved to = (none) draws the moved text as
/// plain text, no mark and no colour, although the colour stays set; the change bar stays.
#[test]
fn moved_to_none_is_plain_text_with_its_change_bar() {
    use wordcraft_layout::display::{MOVE_COLOR, Stroke};
    let mut s = moved_with(json!({}));
    let (d, body_x) = draws(&mut s);
    let default_bars = bars(&d, body_x);
    assert!(default_bars > 0);
    let mut s = moved_with(json!({"moveToMark": "none"}));
    assert_eq!(s.run("review.trackingOptions", &json!({})).unwrap()["moveToColor"], MOVE_COLOR.hex());
    let (d, body_x) = draws(&mut s);
    let sec = seconds(&d);
    assert_eq!(sec.first().map(|c| c.0), Some(wordcraft_doc::Rgb::BLACK), "{sec:?}");
    assert_eq!(sec.get(1).map(|c| c.0), Some(MOVE_COLOR), "the moved-from end keeps its colour");
    assert_eq!(h_strokes(&d), vec![Stroke::Double], "only the moved-from strikethrough");
    assert_eq!(bars(&d, body_x), default_bars, "the change bars stay");
}

/// Gap #5, Word for Mac 16.113.4: Deletions and Moved from offer (none), Bold, Italic and
/// Double underline too. Moved from (none) is plain text, as Moved to (none); an ordinary
/// deletion marked (none) keeps its colour.
#[test]
fn deletion_marks_none_bold_italic_and_double_underline() {
    use wordcraft_layout::display::{MOVE_COLOR, Stroke};
    let plain = json!({"moveToMark": "none", "moveToColor": "byAuthor"});
    let with = |extra: serde_json::Value| {
        let mut o = plain.clone();
        o.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        moved_with(o)
    };
    let mut s = with(json!({"moveFromMark": "none"}));
    let (d, _) = draws(&mut s);
    assert_eq!(seconds(&d).get(1).map(|c| c.0), Some(wordcraft_doc::Rgb::BLACK), "{:?}", seconds(&d));
    assert!(h_strokes(&d).is_empty(), "{:?}", h_strokes(&d));
    let mut s = with(json!({"moveFromMark": "bold"}));
    let (d, _) = draws(&mut s);
    assert_eq!(seconds(&d).get(1), Some(&(MOVE_COLOR, true, false)));
    assert!(h_strokes(&d).is_empty());
    let mut s = with(json!({"moveFromMark": "italic"}));
    let (d, _) = draws(&mut s);
    assert_eq!(seconds(&d).get(1), Some(&(MOVE_COLOR, false, true)));
    let mut s = with(json!({"moveFromMark": "doubleUnderline"}));
    let (d, _) = draws(&mut s);
    assert_eq!(h_strokes(&d), vec![Stroke::Double]);
    let r = s.run("review.trackingOptions", &json!({"deleteMark": "none"})).unwrap();
    assert_eq!((r["deleteMark"].as_str(), r["moveFromMark"].as_str()), (Some("none"), Some("doubleUnderline")));

    // An ordinary deletion marked (none): no line, the author's colour.
    let mut s = Session::new(Document::new());
    s.view.proofing = false;
    s.run("document.setText", &json!({"text": "keep gone"})).unwrap();
    s.run("review.trackChanges", &json!({"value": true})).unwrap();
    s.run("select.range", &json!({"anchor": {"block": 0, "off": 5}, "focus": {"block": 0, "off": 9}})).unwrap();
    s.run("text.delete", &json!({})).unwrap();
    s.run("review.trackingOptions", &json!({"deleteMark": "none"})).unwrap();
    let (d, _) = draws(&mut s);
    assert!(h_strokes(&d).is_empty());
    let gone = d.iter().find_map(|x| if let Draw::Glyphs { text, color, .. } = x { text.contains("gone").then_some(*color) } else { None });
    assert_eq!(gone, Some(wordcraft_layout::display::revision_color(0)));
}

/// Review of gap #5: Deletions = Hidden hides deletions, not moved-away text, which follows
/// Moved from (left at its default: shown, green, double strikethrough) and keeps its room,
/// as in Word for Mac 16.113.4 (user-checked 2026-10-15).
#[test]
fn hidden_deletions_leave_moved_from_text_shown() {
    use wordcraft_layout::display::{MOVE_COLOR, Stroke};
    let mut accepted = moved_with(json!({}));
    accepted.run("review.acceptAll", &json!({})).unwrap();
    let third_accepted = caret_at(&mut accepted, 2, 0).1;
    let mut s = moved_with(json!({"deleteMark": "hidden"}));
    assert!(caret_at(&mut s, 3, 0).1 > third_accepted + 1.0, "the moved-away paragraph keeps its line");
    let (d, _) = draws(&mut s);
    assert_eq!(glyph_text(&d).matches("Second").count(), 2, "{:?}", glyph_text(&d));
    let struck = d.iter().any(|x| matches!(x, Draw::Line { color, stroke: Stroke::Double, y0, y1, .. } if *color == MOVE_COLOR && y0 == y1));
    assert!(struck, "green double lines");
}

/// Gap #5: Track moves is a document setting (`w:doNotTrackMoves`): turning it off is one
/// undo step, and setting it to what it is adds none.
#[test]
fn track_moves_is_a_document_setting_with_one_undo_step() {
    let mut s = Session::new(Document::new());
    assert_eq!(s.run("review.trackingOptions", &json!({})).unwrap()["trackMoves"], true);
    assert!(s.doc.settings.track_moves);
    assert!(!s.can_undo());
    assert_eq!(s.run("review.trackingOptions", &json!({"trackMoves": true})).unwrap()["trackMoves"], true);
    assert!(!s.can_undo(), "no change, no undo step");
    assert_eq!(s.run("review.trackingOptions", &json!({"trackMoves": false})).unwrap()["trackMoves"], false);
    assert!(!s.doc.settings.track_moves && s.dirty);
    s.run("edit.undo", &json!({})).unwrap();
    assert!(s.doc.settings.track_moves);
    assert!(!s.can_undo());
    // A refused parameter changes nothing.
    assert!(s.run("review.trackingOptions", &json!({"trackMoves": false, "moveToMark": "sparkles"})).is_err());
    assert!(s.doc.settings.track_moves);
    // Documents saved before the setting existed keep tracking moves.
    let old: wordcraft_doc::Settings = serde_json::from_value(json!({"trackChanges": true})).unwrap();
    assert!(old.track_moves);
}

/// #1055: the caret's own comment is the one whose range holds it, not any in its paragraph.
#[test]
fn comment_range_at_caret_needs_the_caret_inside_the_range() {
    let mut s = Session::new(wordcraft_doc::Document::from_text("one two three"));
    s.run("select.text", &json!({"text": "two"})).unwrap();
    let id = s.run("review.newComment", &json!({"text": "note"})).unwrap()["id"].as_u64().unwrap() as u32;
    s.run("select.text", &json!({"text": "two"})).unwrap();
    s.run("caret.left", &json!({})).unwrap();
    assert_eq!(super::comment_range_at_caret(&s), Some(id), "inside the commented text");
    s.run("select.text", &json!({"text": "three"})).unwrap();
    assert_eq!(super::comment_range_at_caret(&s), None, "elsewhere in the paragraph");
    assert_eq!(super::comment_at_caret(&s), Some(id), "the older lookup still falls back to the paragraph's comment");
}

fn glyph_text(d: &[Draw]) -> String {
    d.iter().filter_map(|x| if let Draw::Glyphs { text, .. } = x { Some(text.as_str()) } else { None }).collect()
}

/// Show revisions in balloons: a tracked deletion leaves the text for a balloon at its place
/// and keeps its change bar; it comes back inline in the other modes and wherever no balloons
/// are shown (here the Comments pane, which changes the view without a relayout call).
#[test]
fn revisions_in_balloons_take_deletions_out_of_the_text() {
    use super::{DeletionBalloon, deletion_balloons};
    let mut s = Session::new(Document::new());
    s.view.proofing = false;
    s.author = "Ana".into();
    s.run("document.setText", &json!({"text": "keep gone stay"})).unwrap();
    s.run("review.trackChanges", &json!({"value": true})).unwrap();
    s.run("select.range", &json!({"anchor": {"block": 0, "off": 5}, "focus": {"block": 0, "off": 10}})).unwrap();
    s.run("text.delete", &json!({})).unwrap();

    // The default (only comments and formatting in balloons): struck through inline.
    assert!(!s.revisions_in_balloons());
    let (d, _) = draws(&mut s);
    assert!(glyph_text(&d).contains("gone"), "{:?}", glyph_text(&d));

    s.run("review.trackingOptions", &json!({"balloons": "revisions"})).unwrap();
    assert!(s.revisions_in_balloons());
    let (d, body_x) = draws(&mut s);
    assert!(!glyph_text(&d).contains("gone"), "the deletion left the text: {:?}", glyph_text(&d));
    let horizontal = lines(&d).into_iter().filter(|(_, y0, _, y1)| y0 == y1).count();
    assert_eq!(horizontal, 0, "no strikethrough left behind: {:?}", lines(&d));
    assert_eq!(lines(&d).into_iter().filter(|(x0, _, x1, _)| x0 == x1 && *x0 < body_x).count(), 1, "the changed line keeps its bar");
    let at = wordcraft_doc::Pos::body(0, 5);
    assert_eq!(deletion_balloons(&s), vec![DeletionBalloon { at: at.clone(), author: "Ana".into(), text: "gone ".into() }]);
    // The balloon's anchor is where the deletion was: between "keep " and "stay".
    let l = s.layout();
    let (anchor, stay) = (l.caret(&at).unwrap().x, l.caret(&wordcraft_doc::Pos::body(0, 10)).unwrap().x);
    assert!((anchor - stay).abs() < 0.01, "the deletion takes no room: {anchor} vs {stay}");

    // No balloons beside the page while the Comments pane lists comments: inline again.
    s.run("view.commentsPane", &json!({"value": true})).unwrap();
    assert!(!s.revisions_in_balloons());
    assert!(glyph_text(&draws(&mut s).0).contains("gone"));
    s.run("view.commentsPane", &json!({"value": false})).unwrap();
    assert!(!glyph_text(&draws(&mut s).0).contains("gone"));
    s.run("review.trackingOptions", &json!({"balloons": "inline"})).unwrap();
    assert!(!s.revisions_in_balloons());
    assert!(glyph_text(&draws(&mut s).0).contains("gone"));
}

/// Deleted text running on across runs, separate deletions and a paragraph mark is one balloon
/// per author; a long deletion shows its start.
#[test]
fn deletion_balloons_join_runs_and_paragraph_marks_and_stay_short() {
    use super::{DELETION_TEXT_CHARS, deletion_balloons};
    let mut s = Session::new(Document::new());
    s.author = "Ana".into();
    s.run("document.setText", &json!({"text": "one two\nthree four"})).unwrap();
    s.run("review.trackChanges", &json!({"value": true})).unwrap();
    // "two", then the paragraph mark and "three" in a second deletion: one balloon.
    s.run("select.range", &json!({"anchor": {"block": 0, "off": 4}, "focus": {"block": 0, "off": 7}})).unwrap();
    s.run("text.delete", &json!({})).unwrap();
    s.run("select.range", &json!({"anchor": {"block": 0, "off": 7}, "focus": {"block": 1, "off": 5}})).unwrap();
    s.run("text.delete", &json!({})).unwrap();
    // Bo's deletion right after Ana's is a balloon of its own.
    s.author = "Bo".into();
    s.run("select.range", &json!({"anchor": {"block": 1, "off": 5}, "focus": {"block": 1, "off": 10}})).unwrap();
    s.run("text.delete", &json!({})).unwrap();
    let b = deletion_balloons(&s);
    let got: Vec<(&str, &str)> = b.iter().map(|d| (d.author.as_str(), d.text.as_str())).collect();
    assert_eq!(got, vec![("Ana", "two¶three"), ("Bo", " four")]);
    assert_eq!(b[0].at, wordcraft_doc::Pos::body(0, 4));

    // A deletion far longer than a balloon shows: its first characters and an ellipsis.
    let mut s = Session::new(Document::new());
    let long = "word ".repeat(400);
    s.run("document.setText", &json!({"text": long})).unwrap();
    s.run("review.trackChanges", &json!({"value": true})).unwrap();
    s.run("select.all", &json!({})).unwrap();
    s.run("text.delete", &json!({})).unwrap();
    let b = deletion_balloons(&s);
    assert_eq!(b.len(), 1);
    assert_eq!(b[0].text.chars().count(), DELETION_TEXT_CHARS + 1);
    assert!(b[0].text.starts_with("word word") && b[0].text.ends_with('…'));
}

/// #404: the source of a move is a deletion revision with a move name. With revisions in
/// balloons it leaves the text like a deletion but has only its "Moved from" balloon, not a
/// "Deleted: …" one as well; a plain deletion right after it is a balloon of its own.
#[test]
fn a_moves_source_gets_no_deletion_balloon() {
    use super::{DeletionBalloon, deletion_balloons, has_deletions, move_changes};
    use wordcraft_doc::Pos;
    let mut s = Session::new(Document::new());
    s.run("document.setText", &json!({"text": "First one.\nSecond one.\nThird one."})).unwrap();
    s.run("file.setAuthor", &json!({"name": "Ana"})).unwrap();
    s.run("review.trackChanges", &json!({"value": true})).unwrap();
    // The second paragraph, with its mark, cut and pasted before the first: a move.
    s.run("select.range", &json!({"anchor": {"block": 1, "off": 0}, "focus": {"block": 2, "off": 0}})).unwrap();
    s.run("edit.cut", &json!({})).unwrap();
    s.run("caret.set", &json!({"pos": {"block": 0, "off": 0}})).unwrap();
    s.run("edit.paste", &json!({})).unwrap();
    s.run("review.trackingOptions", &json!({"balloons": "revisions"})).unwrap();
    assert!(s.revisions_in_balloons());

    let ends: Vec<(Pos, bool)> = move_changes(&s).into_iter().map(|(at, _, from)| (at, from)).collect();
    assert_eq!(ends, vec![(Pos::body(0, 0), false), (Pos::body(2, 0), true)]);
    assert!(deletion_balloons(&s).is_empty(), "the move's source has its \"Moved from\" balloon only: {:?}", deletion_balloons(&s));
    assert!(!has_deletions(&s.doc), "a move alone needs no deletion balloons");
    assert_eq!(glyph_text(&draws(&mut s).0), "Second one.First one.Third one.", "the moved-away text left the text");

    // "Third " deleted right where the moved-away paragraph ends: not joined to it.
    s.run("select.range", &json!({"anchor": {"block": 3, "off": 0}, "focus": {"block": 3, "off": 6}})).unwrap();
    s.run("text.delete", &json!({})).unwrap();
    assert!(has_deletions(&s.doc));
    assert_eq!(deletion_balloons(&s), vec![DeletionBalloon { at: Pos::body(3, 0), author: "Ana".into(), text: "Third ".into() }]);
    assert_eq!(move_changes(&s).len(), 2, "the move keeps both its balloons");
}

/// Revisions in balloons take deletions out of the body only, where their balloons are:
/// headers, footers, footnotes, endnotes and text boxes get no balloons, so their deletions
/// stay in the text, struck through, as in the other modes.
#[test]
fn deletions_outside_the_body_stay_inline_when_revisions_go_in_balloons() {
    use super::{DeletionBalloon, deletion_balloons};
    use wordcraft_doc::{Path, Pos, StoryRef};
    let mut s = Session::new(Document::new());
    s.view.proofing = false;
    s.author = "Ana".into();
    s.run("document.setText", &json!({"text": "Body zeta keep"})).unwrap();
    let part = |v: serde_json::Value, key: &str| StoryRef::Part(v[key].as_u64().unwrap() as u32);
    let header = part(s.run("insert.header", &json!({"text": "Head alpha"})).unwrap(), "story");
    s.run("insert.closeHeader", &json!({})).unwrap();
    let footer = part(s.run("insert.footer", &json!({"text": "Foot beta"})).unwrap(), "story");
    s.run("insert.closeHeader", &json!({})).unwrap();
    let in_body = json!({"pos": {"block": 0, "off": 4}});
    s.run("caret.set", &in_body).unwrap();
    let footnote = part(s.run("references.footnote", &json!({"text": "Note gamma"})).unwrap(), "id");
    s.run("caret.set", &in_body).unwrap();
    let endnote = part(s.run("references.endnote", &json!({"text": "End delta"})).unwrap(), "id");
    s.run("caret.set", &in_body).unwrap();
    let text_box = part(s.run("insert.textBox", &json!({"text": "Box epsilon"})).unwrap(), "story");

    // With Track Changes on, a word deleted in each story.
    s.run("review.trackChanges", &json!({"value": true})).unwrap();
    let words = [(StoryRef::Body, "zeta"), (header, "alpha"), (footer, "beta"), (footnote, "gamma"), (endnote, "delta"), (text_box, "epsilon")];
    for (story, word) in words {
        let at = s.doc.para(story, &Path::top(0)).and_then(|p| p.text.find(word)).unwrap();
        let pos = |off: usize| json!({"story": story, "path": [0], "off": off});
        s.run("select.range", &json!({"anchor": pos(at), "focus": pos(at + word.len())})).unwrap();
        s.run("text.delete", &json!({})).unwrap();
        let p = s.doc.para(story, &Path::top(0)).unwrap();
        assert!(p.run_ranges().any(|(r, c)| c.del.is_some() && p.text.get(r) == Some(word)), "{word} is a tracked deletion");
    }
    let struck = |d: &[Draw]| lines(d).into_iter().filter(|(_, y0, _, y1)| y0 == y1).count();

    // Comments and formatting in balloons: every deletion struck through where it was.
    let (d, _) = draws(&mut s);
    let text = glyph_text(&d);
    for (_, word) in words {
        assert!(text.contains(word), "{word} in {text:?}");
    }
    let all_struck = struck(&d);

    // Revisions in balloons: only the body's deletion leaves the text, for its balloon.
    s.run("review.trackingOptions", &json!({"balloons": "revisions"})).unwrap();
    assert!(s.revisions_in_balloons());
    let (d, _) = draws(&mut s);
    let text = glyph_text(&d);
    assert!(!text.contains("zeta"), "the body's deletion left the text: {text:?}");
    for (_, word) in &words[1..] {
        assert!(text.contains(word), "{word} stays inline: {text:?}");
    }
    assert_eq!(struck(&d), all_struck - 1, "the other deletions stay struck through");
    // Its balloon is the only one: the notes' and text box's objects sit before "zeta".
    let at = s.doc.para(StoryRef::Body, &Path::top(0)).and_then(|p| p.text.find("zeta")).unwrap();
    assert_eq!(deletion_balloons(&s), vec![DeletionBalloon { at: Pos::body(0, at), author: "Ana".into(), text: "zeta".into() }]);
}

fn body_para(s: &Session) -> &wordcraft_doc::Paragraph {
    s.doc.para(wordcraft_doc::StoryRef::Body, &wordcraft_doc::Path::top(0)).unwrap()
}

/// A comment inside a tracked insertion splits it into several runs; Accept and Reject at the
/// caret resolve the whole insertion, not just the run around the caret.
#[test]
fn accept_and_reject_cover_a_change_split_by_a_comment() {
    for accept in [true, false] {
        let mut s = Session::new(Document::new());
        s.view.proofing = false;
        s.author = "Ana".into();
        s.run("review.trackChanges", &json!({"value": true})).unwrap();
        s.run("text.insert", &json!({"text": "hello world"})).unwrap();
        s.run("select.range", &json!({"anchor": {"block": 0, "off": 3}, "focus": {"block": 0, "off": 8}})).unwrap();
        s.run("review.newComment", &json!({"text": "note"})).unwrap();
        assert!(body_para(&s).runs.len() > 3, "the comment anchors split the insertion");
        s.run("select.range", &json!({"anchor": {"block": 0, "off": 1}, "focus": {"block": 0, "off": 1}})).unwrap();
        s.run(if accept { "review.accept" } else { "review.reject" }, &json!({})).unwrap();
        let p = body_para(&s);
        assert!(p.runs.iter().all(|r| r.props.ins.is_none()), "no tracked insertion is left");
        // The comment's anchors are objects in the text; read past them.
        let shown: String = p.text.chars().filter(|c| *c != wordcraft_doc::para::OBJ).collect();
        assert_eq!(shown.contains("hello"), accept, "{:?}", p.text);
    }
}

/// "one two three" with "new " typed at the start, "two " deleted and " end" typed at the end,
/// under Track Changes: three changes.
fn three_changes() -> Session {
    let mut s = Session::new(Document::new());
    s.view.proofing = false;
    s.author = "Ana".into();
    s.run("document.setText", &json!({"text": "one two three"})).unwrap();
    s.run("review.trackChanges", &json!({"value": true})).unwrap();
    s.run("caret.docStart", &json!({})).unwrap();
    s.run("text.insert", &json!({"text": "new "})).unwrap();
    s.run("select.range", &json!({"anchor": {"block": 0, "off": 8}, "focus": {"block": 0, "off": 12}})).unwrap();
    s.run("text.delete", &json!({})).unwrap();
    s.run("caret.docEnd", &json!({})).unwrap();
    s.run("text.insert", &json!({"text": " end"})).unwrap();
    s
}

fn change_spans(s: &mut Session) -> Vec<(String, u64, u64)> {
    let list = s.run("review.changes", &json!({})).unwrap();
    list.as_array()
        .unwrap()
        .iter()
        .map(|c| (c["kind"].as_str().unwrap().to_string(), c["start"]["off"].as_u64().unwrap(), c["end"]["off"].as_u64().unwrap()))
        .collect()
}

fn selected(s: &Session) -> (usize, usize) {
    let (a, b) = s.sel.ordered();
    (a.off, b.off)
}

/// Accept and Move to Next, Reject and Move to Next: the change at the caret is resolved and
/// the next one is selected; without `next` the caret stays where the change was.
#[test]
fn accept_and_reject_can_move_to_the_next_change() {
    for (cmd, accept) in [("review.accept", true), ("review.reject", false)] {
        let mut s = three_changes();
        let before = change_spans(&mut s);
        assert_eq!(before.iter().map(|c| c.0.as_str()).collect::<Vec<_>>(), ["insert", "delete", "insert"], "{cmd}");
        s.run("caret.docStart", &json!({})).unwrap();
        s.run(cmd, &json!({"next": true})).unwrap();
        let after = change_spans(&mut s);
        assert_eq!(after.len(), 2, "{cmd}: the first insertion is resolved");
        assert_eq!(after[0].0, "delete", "{cmd}");
        let deletion = (after[0].1 as usize, after[0].2 as usize);
        assert_eq!(selected(&s), deletion, "{cmd}: the deletion is selected next");
        assert_eq!(s.doc.plain_text(wordcraft_doc::StoryRef::Body).starts_with("new "), accept, "{cmd}");

        // Without `next`, the caret stays at the resolved change.
        let mut s = three_changes();
        s.run("caret.docStart", &json!({})).unwrap();
        s.run(cmd, &json!({})).unwrap();
        assert_eq!(selected(&s), (0, 0), "{cmd}");
    }
}

/// With the caret on no change, Accept and Move to Next selects the next change without
/// resolving it, as Accept and Next Change do.
#[test]
fn accept_and_move_to_next_off_a_change_selects_it_unresolved() {
    let mut s = three_changes();
    s.run("select.range", &json!({"anchor": {"block": 0, "off": 5}, "focus": {"block": 0, "off": 5}})).unwrap();
    s.run("review.accept", &json!({"next": true})).unwrap();
    let after = change_spans(&mut s);
    assert_eq!(after.len(), 3, "nothing is resolved");
    assert_eq!(selected(&s), (after[1].1 as usize, after[1].2 as usize), "the deletion after the caret is selected");
}

/// Reject and Move to Next on the only text of a centered paragraph: the paragraph's formatting
/// change, now empty at the caret, is selected next, not the insertion after it.
#[test]
fn move_to_next_selects_a_change_left_at_the_caret() {
    let mut s = Session::new(Document::new());
    s.view.proofing = false;
    s.author = "Ana".into();
    s.run("document.setText", &json!({"text": "one\n\nthree"})).unwrap();
    s.run("review.trackChanges", &json!({"value": true})).unwrap();
    s.run("select.range", &json!({"anchor": {"block": 1, "off": 0}, "focus": {"block": 1, "off": 0}})).unwrap();
    s.run("para.align", &json!({"value": "center"})).unwrap();
    s.run("text.insert", &json!({"text": "x"})).unwrap();
    s.run("caret.docEnd", &json!({})).unwrap();
    s.run("text.insert", &json!({"text": "!"})).unwrap();
    let before = change_spans(&mut s);
    assert_eq!(before.iter().map(|c| c.0.as_str()).collect::<Vec<_>>(), ["format", "insert", "insert"], "{before:?}");
    s.run("select.range", &json!({"anchor": {"block": 1, "off": 1}, "focus": {"block": 1, "off": 1}})).unwrap();
    s.run("review.reject", &json!({"next": true})).unwrap();
    let after = change_spans(&mut s);
    assert_eq!(after.iter().map(|c| c.0.as_str()).collect::<Vec<_>>(), ["format", "insert"], "{after:?}");
    let (a, b) = s.sel.ordered();
    let at = (wordcraft_doc::Path::top(1), 0);
    assert_eq!(((a.path.clone(), a.off), (b.path.clone(), b.off)), (at.clone(), at), "the centering is selected");
}

/// Accept All Changes and Stop Tracking, Reject All Changes and Stop Tracking: every change is
/// resolved and Track Changes is off; plain Accept All and Reject All leave it on.
#[test]
fn accept_and_reject_all_can_stop_tracking() {
    for cmd in ["review.acceptAll", "review.rejectAll"] {
        let mut s = three_changes();
        s.run(cmd, &json!({"stopTracking": true})).unwrap();
        assert!(change_spans(&mut s).is_empty(), "{cmd}");
        assert!(!s.doc.settings.track_changes, "{cmd}: Track Changes is off");
        s.run("edit.undo", &json!({})).unwrap();
        assert!(s.doc.settings.track_changes, "{cmd}: one undo step brings tracking and the changes back");
        assert_eq!(change_spans(&mut s).len(), 3, "{cmd}");

        let mut s = three_changes();
        s.run(cmd, &json!({})).unwrap();
        assert!(change_spans(&mut s).is_empty(), "{cmd}");
        assert!(s.doc.settings.track_changes, "{cmd}: Track Changes stays on");
    }
}

/// "plain words here" with "words" made bold, then 14 pt, and the paragraph centred, under
/// Track Changes by Ana.
fn formatted_while_tracking() -> Session {
    let mut s = Session::new(Document::new());
    s.view.proofing = false;
    s.author = "Ana".into();
    s.run("document.setText", &json!({"text": "plain words here"})).unwrap();
    // Untracked formatting is no revision.
    s.run("select.range", &json!({"anchor": {"block": 0, "off": 12}, "focus": {"block": 0, "off": 16}})).unwrap();
    s.run("format.italic", &json!({})).unwrap();
    assert!(body_para(&s).runs.iter().all(|r| r.props.fmt_change.is_none()));
    s.run("review.trackChanges", &json!({"value": true})).unwrap();
    s.run("select.range", &json!({"anchor": {"block": 0, "off": 6}, "focus": {"block": 0, "off": 11}})).unwrap();
    s.run("format.bold", &json!({})).unwrap();
    s.run("format.size", &json!({"size": 14})).unwrap();
    s.run("para.alignCenter", &json!({})).unwrap();
    s
}

/// #41: formatting with Track Changes on records the formatting before the author's first
/// change; it's listed in the Reviewing Pane and marked by a bar beside the line.
#[test]
fn formatting_is_tracked_while_tracking_changes() {
    let mut s = formatted_while_tracking();
    let p = body_para(&s);
    let ch = p.props_of_char(6).fmt_change.as_deref().expect("a formatting change on \"words\"");
    assert_eq!((ch.old.bold, ch.old.size), (None, None), "the formatting before the first change is kept");
    let rev = &s.doc.revisions[ch.rev as usize];
    assert_eq!((rev.kind, rev.author.as_str()), (wordcraft_doc::RevisionKind::Format, "Ana"));
    assert!(p.props_of_char(0).fmt_change.is_none() && p.props_of_char(13).fmt_change.is_none());
    assert_eq!(p.props.fmt_change.as_deref().map(|c| c.old.align), Some(None));

    let list = s.run("review.changes", &json!({})).unwrap();
    let desc: Vec<&str> = list.as_array().unwrap().iter().filter(|c| c["kind"] == "format").filter_map(|c| c["description"].as_str()).collect();
    assert_eq!(desc, ["Formatted: Align: center", "Formatted: Bold, Bold cs, Size: 14, Size cs: 14"], "{list:#}");

    let (d, body_x) = draws(&mut s);
    let bars = lines(&d).into_iter().filter(|(x0, _, x1, _)| x0 == x1 && *x0 < body_x).count();
    assert_eq!(bars, 1, "one changed line, one bar");

    // Formatting changed back is no change any more.
    s.run("select.range", &json!({"anchor": {"block": 0, "off": 6}, "focus": {"block": 0, "off": 11}})).unwrap();
    s.run("format.set", &json!({"props": {"bold": false}})).unwrap();
    s.run("format.size", &json!({"size": 11})).unwrap();
    assert!(body_para(&s).props_of_char(6).fmt_change.is_some(), "explicit Not Bold and 11 pt differ from inherited");
}

/// #41: rejecting restores the formatting before the change; accepting keeps the new
/// formatting and drops the record. Both find formatting changes at the caret too.
#[test]
fn formatting_changes_are_accepted_and_rejected() {
    let mut s = formatted_while_tracking();
    s.run("review.rejectAll", &json!({})).unwrap();
    let p = body_para(&s);
    let c = p.props_of_char(6);
    assert_eq!((c.bold, c.size, c.fmt_change.is_none()), (None, None, true));
    assert_eq!((p.props.align, p.props.fmt_change.is_none()), (None, true));
    assert_eq!(p.props_of_char(13).italic, Some(true), "untracked formatting stays");

    s.run("edit.undo", &json!({})).unwrap();
    // Accept at the caret: just the run's change.
    s.run("select.range", &json!({"anchor": {"block": 0, "off": 8}, "focus": {"block": 0, "off": 8}})).unwrap();
    s.run("review.accept", &json!({})).unwrap();
    let p = body_para(&s);
    assert_eq!((p.props_of_char(6).bold, p.props_of_char(6).fmt_change.is_none()), (Some(true), true));
    assert!(p.props.fmt_change.is_some(), "the paragraph's change is still there");
    s.run("review.acceptAll", &json!({})).unwrap();
    let p = body_para(&s);
    assert_eq!((p.props.align, p.props.fmt_change.is_none()), (Some(wordcraft_doc::props::Align::Center), true));
    assert!(s.run("review.changes", &json!({})).unwrap().as_array().unwrap().is_empty());
}

/// Clearing formatting in the Style Inspector keeps pending formatting revisions.
#[test]
fn inspector_clear_keeps_formatting_changes() {
    let mut s = formatted_while_tracking();
    s.run("review.trackChanges", &json!({"value": false})).unwrap();
    s.run("styles.inspectorClear", &json!({"level": "paragraphFormatting"})).unwrap();
    assert!(body_para(&s).props.fmt_change.is_some(), "paragraph formatting change survives");

    let mut s = formatted_while_tracking();
    s.run("review.trackChanges", &json!({"value": false})).unwrap();
    s.run("select.range", &json!({"anchor": {"block": 0, "off": 6}, "focus": {"block": 0, "off": 11}})).unwrap();
    s.run("styles.inspectorClear", &json!({"level": "characterFormatting"})).unwrap();
    assert!(body_para(&s).props_of_char(6).fmt_change.is_some(), "character formatting change survives");
}

/// Comment anchors left in any story (the body, headers, footers, notes and text boxes), with
/// the story each sits in.
fn anchors(s: &Session) -> Vec<(wordcraft_doc::StoryRef, u32)> {
    use wordcraft_doc::StoryRef;
    use wordcraft_doc::para::InlineObject;
    let stories = std::iter::once(StoryRef::Body).chain(s.doc.parts.keys().map(|k| StoryRef::Part(*k)));
    let mut ids = Vec::new();
    for story in stories {
        for path in s.doc.para_paths(story) {
            let p = s.doc.para(story, &path).unwrap();
            for o in p.object_offsets() {
                if let Some(InlineObject::CommentStart { id } | InlineObject::CommentEnd { id }) = p.object_at(o) {
                    ids.push((story, *id));
                }
            }
        }
    }
    ids
}

/// #1345: deleting a comment anchored in a header removes its anchors there, not just in the
/// body; Undo brings the comment and its anchors back.
#[test]
fn deleting_a_header_comment_removes_its_anchors() {
    let mut s = Session::new(Document::new());
    s.view.proofing = false;
    s.run("document.setText", &json!({"text": "Body text"})).unwrap();
    let hdr = s.run("insert.header", &json!({"text": "Header text"})).unwrap()["story"].as_u64().unwrap() as u32;
    s.run("select.all", &json!({})).unwrap();
    let id = s.run("review.newComment", &json!({"text": "in the header"})).unwrap()["id"].as_u64().unwrap() as u32;
    let in_header = vec![(wordcraft_doc::StoryRef::Part(hdr), id); 2];
    assert_eq!(anchors(&s), in_header, "both anchors sit in the header");
    s.run("review.deleteComment", &json!({"id": id})).unwrap();
    assert!(anchors(&s).is_empty(), "no anchor points at the deleted comment");
    assert!(s.doc.comments.is_empty());
    s.run("edit.undo", &json!({})).unwrap();
    assert_eq!(anchors(&s), in_header);
    assert!(s.doc.comments.contains_key(&id));
}

/// #1345: deleting a comment deletes the whole thread below it, replies to replies included,
/// with their text parts; Undo restores all of them.
#[test]
fn deleting_a_comment_deletes_replies_to_replies() {
    let mut s = Session::new(Document::new());
    s.view.proofing = false;
    s.run("document.setText", &json!({"text": "hello world"})).unwrap();
    s.run("select.range", &json!({"anchor": {"block": 0, "off": 0}, "focus": {"block": 0, "off": 5}})).unwrap();
    let root = s.run("review.newComment", &json!({"text": "root"})).unwrap()["id"].as_u64().unwrap();
    let reply = s.run("review.reply", &json!({"id": root, "text": "reply"})).unwrap()["id"].as_u64().unwrap();
    let nested = s.run("review.reply", &json!({"id": reply, "text": "reply to reply"})).unwrap()["id"].as_u64().unwrap();
    let parts = |s: &Session| s.doc.comments.values().map(|c| c.part).collect::<Vec<_>>();
    let before = parts(&s);
    let deleted = s.run("review.deleteComment", &json!({"id": root})).unwrap();
    assert_eq!(deleted["deleted"], json!([root, reply, nested]));
    assert!(s.doc.comments.is_empty(), "no reply is left without its parent: {:?}", s.doc.comments);
    assert!(before.iter().all(|p| !s.doc.parts.contains_key(p)), "the comments' text parts are gone");
    assert!(anchors(&s).is_empty());
    s.run("edit.undo", &json!({})).unwrap();
    assert_eq!(s.doc.comments.keys().map(|k| u64::from(*k)).collect::<Vec<_>>(), vec![root, reply, nested]);
    assert_eq!(parts(&s), before);
    assert_eq!(anchors(&s).len(), 2);
}

/// A parent cycle (possible in a hostile file) doesn't hang Delete; both comments go.
#[test]
fn deleting_a_comment_in_a_parent_cycle_ends() {
    let mut s = Session::new(Document::new());
    s.view.proofing = false;
    s.run("document.setText", &json!({"text": "hello"})).unwrap();
    s.run("select.all", &json!({})).unwrap();
    let a = s.run("review.newComment", &json!({"text": "a"})).unwrap()["id"].as_u64().unwrap() as u32;
    let b = s.run("review.reply", &json!({"id": a, "text": "b"})).unwrap()["id"].as_u64().unwrap() as u32;
    s.doc.comments.get_mut(&a).unwrap().parent = Some(b);
    s.run("review.deleteComment", &json!({"id": a})).unwrap();
    assert!(s.doc.comments.is_empty());
}

/// A very long reply chain (as a hostile file could hold) is deleted in one pass over the
/// comments, not one pass per reply: 50,000 replies would otherwise freeze the app.
#[test]
fn deleting_a_long_reply_chain_stays_fast() {
    let mut s = Session::new(Document::new());
    s.view.proofing = false;
    s.run("document.setText", &json!({"text": "hello"})).unwrap();
    s.run("select.all", &json!({})).unwrap();
    let root = s.run("review.newComment", &json!({"text": "root"})).unwrap()["id"].as_u64().unwrap() as u32;
    let template = s.doc.comments.get(&root).unwrap().clone();
    for k in 1..=50_000u32 {
        s.doc.comments.insert(root + k, wordcraft_doc::Comment { parent: Some(root + k - 1), ..template.clone() });
    }
    let started = std::time::Instant::now();
    s.run("review.deleteComment", &json!({"id": root})).unwrap();
    assert!(s.doc.comments.is_empty());
    assert!(started.elapsed() < std::time::Duration::from_secs(5), "took {:?}", started.elapsed());
}

/// A comment on the whole of the story the caret is in; its id and that story.
fn comment_here(s: &mut Session, text: &str) -> (u64, wordcraft_doc::StoryRef) {
    let story = s.sel.focus.story;
    s.run("select.all", &json!({})).unwrap();
    (s.run("review.newComment", &json!({"text": text})).unwrap()["id"].as_u64().unwrap(), story)
}

/// Comments in a header, a footer, a text box and a footnote, made in that order, then one
/// on a word of the body. Returns each comment's id and story in reading order, which isn't
/// the order they were made: the body, the text box and footnote it shows, then the header
/// and the footer.
fn comments_in_every_story(s: &mut Session) -> Vec<(u64, wordcraft_doc::StoryRef)> {
    s.view.proofing = false;
    s.run("document.setText", &json!({"text": "Body text with a comment on it."})).unwrap();
    s.run("insert.header", &json!({"text": "header"})).unwrap();
    let header = comment_here(s, "header");
    s.run("insert.closeHeader", &json!({})).unwrap();
    s.run("insert.footer", &json!({"text": "footer"})).unwrap();
    let footer = comment_here(s, "footer");
    s.run("insert.closeHeader", &json!({})).unwrap();
    s.run("caret.docEnd", &json!({})).unwrap();
    s.run("insert.textBox", &json!({"text": "boxed"})).unwrap();
    let text_box = comment_here(s, "text box");
    s.run("insert.closeHeader", &json!({})).unwrap();
    s.run("caret.docEnd", &json!({})).unwrap();
    s.run("references.footnote", &json!({"text": "noted"})).unwrap();
    let note = comment_here(s, "footnote");
    s.run("insert.closeHeader", &json!({})).unwrap();
    s.run("select.text", &json!({"text": "comment"})).unwrap();
    let body = (s.run("review.newComment", &json!({"text": "body"})).unwrap()["id"].as_u64().unwrap(), wordcraft_doc::StoryRef::Body);
    s.run("caret.docStart", &json!({})).unwrap();
    vec![body, text_box, note, header, footer]
}

/// `review.comments` as (id, the anchor's story).
fn listed(s: &mut Session) -> Vec<(u64, Option<wordcraft_doc::StoryRef>)> {
    let list = s.run("review.comments", &json!({})).unwrap();
    list.as_array().unwrap().iter().map(|c| (c["id"].as_u64().unwrap(), serde_json::from_value(c["anchor"]["story"].clone()).ok())).collect()
}

/// The stories Next Comment visits, `n` presses from the caret.
fn next_stories(s: &mut Session, n: usize) -> Vec<wordcraft_doc::StoryRef> {
    (0..n)
        .map(|_| {
            s.run("review.nextComment", &json!({})).unwrap();
            s.sel.focus.story
        })
        .collect()
}

/// #2289: comments in headers, footers, text boxes and footnotes are listed with their
/// anchors, in reading order rather than the order their stories were made.
#[test]
fn comments_outside_the_body_are_listed_with_their_anchors() {
    let mut s = Session::new(Document::new());
    let want: Vec<_> = comments_in_every_story(&mut s).into_iter().map(|(id, st)| (id, Some(st))).collect();
    assert_eq!(listed(&mut s), want);
}

/// #2289: Next and Previous Comment go through every story's comments in reading order and
/// wrap around.
#[test]
fn next_and_previous_comment_reach_comments_outside_the_body() {
    let mut s = Session::new(Document::new());
    let want: Vec<_> = comments_in_every_story(&mut s).into_iter().map(|(_, st)| st).collect();
    let mut wrapped = want.clone();
    wrapped.push(want[0]);
    assert_eq!(next_stories(&mut s, want.len() + 1), wrapped, "Next visits each comment, then wraps to the first");
    let back: Vec<_> = (0..want.len())
        .map(|_| {
            s.run("review.previousComment", &json!({})).unwrap();
            s.sel.focus.story
        })
        .collect();
    let mut rev = want.clone();
    rev.reverse();
    assert_eq!(back, rev, "Previous from the first comment wraps to the last");
}

/// #2289: a header comes before the footer whichever was made first.
#[test]
fn next_comment_reaches_the_header_before_the_footer() {
    let mut s = Session::new(Document::new());
    s.view.proofing = false;
    s.run("document.setText", &json!({"text": "Body"})).unwrap();
    s.run("insert.footer", &json!({"text": "footer"})).unwrap();
    let footer = comment_here(&mut s, "footer").1;
    s.run("insert.closeHeader", &json!({})).unwrap();
    s.run("insert.header", &json!({"text": "header"})).unwrap();
    let header = comment_here(&mut s, "header").1;
    s.run("insert.closeHeader", &json!({})).unwrap();
    assert_eq!(next_stories(&mut s, 2), vec![header, footer]);
}

/// #2289: a comment left in a story nothing shows any more (a removed header, a deleted
/// footnote's text) has no anchor, and Next never takes the caret into that hidden story.
#[test]
fn comments_in_hidden_stories_are_not_navigated_to() {
    let mut s = Session::new(Document::new());
    let all = comments_in_every_story(&mut s);
    let (note, header) = (all[2], all[3]);
    s.run("insert.removeHeader", &json!({})).unwrap();
    s.run("caret.docEnd", &json!({})).unwrap();
    s.run("text.backspace", &json!({})).unwrap();
    assert!(!s.doc.shown_stories().contains(&note.1), "the footnote's reference is gone");
    let list = listed(&mut s);
    for hidden in [note, header] {
        assert!(list.contains(&(hidden.0, None)), "comment {} sits in a hidden story: {list:?}", hidden.0);
    }
    let shown = [all[0].1, all[1].1, all[4].1];
    for st in next_stories(&mut s, all.len()) {
        assert!(shown.contains(&st), "Next went to hidden {st:?}");
    }
}

/// A body comment, then, with `option` on, a comment in the header of page `page`; `option`
/// is turned off again. Returns the header comment's id and story.
fn comment_in_header_version(s: &mut Session, option: &str, page: usize) -> (u64, wordcraft_doc::StoryRef) {
    s.view.proofing = false;
    s.run("document.setText", &json!({"text": "Body text with a comment on it."})).unwrap();
    s.run("select.text", &json!({"text": "comment"})).unwrap();
    s.run("review.newComment", &json!({"text": "body"})).unwrap();
    s.run("caret.docEnd", &json!({})).unwrap();
    s.run("insert.pageBreak", &json!({})).unwrap();
    s.run(option, &json!({"value": true})).unwrap();
    s.run("insert.editHeader", &json!({"page": page})).unwrap();
    s.run("text.insert", &json!({"text": "header"})).unwrap();
    let header = comment_here(s, "header");
    s.run("insert.closeHeader", &json!({})).unwrap();
    s.run(option, &json!({"value": false})).unwrap();
    s.run("caret.docStart", &json!({})).unwrap();
    header
}

/// A first-page or even-page header is hidden while its option is off: its comments have no
/// anchor and Next skips them.
#[test]
fn comments_in_header_versions_whose_option_is_off_are_not_navigated_to() {
    for (option, page, slot) in [("layout.differentFirstPage", 0, "first"), ("layout.differentOddEven", 1, "even")] {
        let mut s = Session::new(Document::new());
        let (id, story) = comment_in_header_version(&mut s, option, page);
        let headers = s.doc.sections().first().map(|(_, sec)| sec.headers).unwrap();
        let want = if slot == "first" { headers.first } else { headers.even };
        assert_eq!(want.map(wordcraft_doc::StoryRef::Part), Some(story), "{option}: the comment is in the {slot} header");
        assert!(listed(&mut s).contains(&(id, None)), "{option} off: no anchor");
        for st in next_stories(&mut s, 3) {
            assert_eq!(st, wordcraft_doc::StoryRef::Body, "{option} off: Next went to the hidden {slot} header");
        }
        s.run(option, &json!({"value": true})).unwrap();
        assert!(listed(&mut s).contains(&(id, Some(story))), "{option} on: anchored again");
    }
}

/// With Different First Page on, the first page's header comes before the later pages'
/// header, in the list and for Next.
#[test]
fn first_page_header_comes_before_the_default_header() {
    let mut s = Session::new(Document::new());
    s.view.proofing = false;
    s.run("document.setText", &json!({"text": "Page one"})).unwrap();
    s.run("caret.docEnd", &json!({})).unwrap();
    s.run("insert.pageBreak", &json!({})).unwrap();
    s.run("layout.differentFirstPage", &json!({"value": true})).unwrap();
    let mut header = |page: usize| {
        s.run("insert.editHeader", &json!({"page": page})).unwrap();
        s.run("text.insert", &json!({"text": "header"})).unwrap();
        let c = comment_here(&mut s, "header");
        s.run("insert.closeHeader", &json!({})).unwrap();
        c
    };
    let (first, default) = (header(0), header(1));
    assert_ne!(first.1, default.1);
    s.run("caret.docStart", &json!({})).unwrap();
    assert_eq!(listed(&mut s), vec![(first.0, Some(first.1)), (default.0, Some(default.1))]);
    assert_eq!(next_stories(&mut s, 2), vec![first.1, default.1]);
}

/// A linked text box shows the first box's text: comments in its own text have no anchor and
/// Next skips them.
#[test]
fn comments_in_a_linked_text_box_own_text_are_not_navigated_to() {
    let mut s = Session::new(Document::new());
    s.view.proofing = false;
    s.run("document.setText", &json!({"text": "Body text with a comment on it."})).unwrap();
    s.run("select.text", &json!({"text": "comment"})).unwrap();
    s.run("review.newComment", &json!({"text": "body"})).unwrap();
    s.run("caret.docEnd", &json!({})).unwrap();
    s.run("insert.textBox", &json!({"text": "first box"})).unwrap();
    let first = s.sel.focus.story;
    s.run("insert.closeHeader", &json!({})).unwrap();
    s.run("caret.docEnd", &json!({})).unwrap();
    // Only a blank box can be linked to.
    s.run("insert.textBox", &json!({"text": " "})).unwrap();
    let (id, follower) = comment_here(&mut s, "follower");
    s.run("insert.closeHeader", &json!({})).unwrap();
    s.run("caret.set", &json!({"pos": {"story": first, "path": [0], "off": 0}})).unwrap();
    s.run("shape.link", &json!({"target": 1})).unwrap();
    assert_eq!(s.doc.text_box_chains().first().and_then(|c| c.get(1)).copied().map(wordcraft_doc::StoryRef::Part), Some(follower));
    s.run("caret.docStart", &json!({})).unwrap();
    assert!(listed(&mut s).contains(&(id, None)), "linked: no anchor");
    for st in next_stories(&mut s, 3) {
        assert_ne!(st, follower, "Next went into the linked box's own text");
    }
    s.run("caret.set", &json!({"pos": {"story": first, "path": [0], "off": 0}})).unwrap();
    s.run("shape.breakLink", &json!({})).unwrap();
    assert!(listed(&mut s).contains(&(id, Some(follower))), "unlinked: anchored again");
}

/// Text inside a comment isn't an anchor: a comment marker in a comment's own story is not
/// listed as that comment's place in the document.
#[test]
fn comment_list_skips_comment_text() {
    let mut s = Session::new(Document::new());
    s.view.proofing = false;
    s.run("document.setText", &json!({"text": "hello"})).unwrap();
    s.run("select.all", &json!({})).unwrap();
    let id = s.run("review.newComment", &json!({"text": "note"})).unwrap()["id"].as_u64().unwrap() as u32;
    let part = s.doc.comments.get(&id).unwrap().part;
    let at = wordcraft_doc::Pos { story: wordcraft_doc::StoryRef::Part(part), path: wordcraft_doc::Path::top(0), off: 0 };
    s.doc.insert_object(&at, wordcraft_doc::InlineObject::CommentStart { id }, &Default::default()).unwrap();
    let list = super::comment_list(&s);
    assert_eq!(list.iter().filter(|(x, _)| *x == id).count(), 1);
    assert_eq!(list.first().and_then(|(_, p)| p.as_ref()).map(|p| p.story), Some(wordcraft_doc::StoryRef::Body));
}

#[test]
fn rtf_tracked_changes_accept_and_reject_after_opening_and_round_trip() {
    // Gap 19: an RTF file's tracked insertions and deletions open as tracked changes, also
    // after a WordCraft → RTF → WordCraft round trip.
    let rtf: &[u8] = br"{\rtf1\ansi\deff0{\fonttbl{\f0 Arial;}}{\*\revtbl {Unknown;}{Alice;}{Bob;}}
\pard Keep {\revised\revauth1\revdttm-1480361314 added }text{\deleted\revauthdel2\revdttmdel-2016341499  gone} end{\revised\revauth1\revdttm-1480361314\deleted\revauthdel2\revdttmdel-2016341499  both}.\par}";
    let opened = wordcraft_formats::rtf::import(rtf).unwrap();
    let again = wordcraft_formats::rtf::import(wordcraft_formats::rtf::export(&opened).as_bytes()).unwrap();
    for doc in [opened, again] {
        for (cmd, want) in [("review.acceptAll", "Keep added text end."), ("review.rejectAll", "Keep text gone end.")] {
            let mut s = Session::new(doc.clone());
            assert!(!s.run("review.changes", &json!({})).unwrap().as_array().unwrap().is_empty(), "{cmd}");
            s.run(cmd, &json!({})).unwrap();
            assert_eq!(s.doc.plain_text(wordcraft_doc::StoryRef::Body).trim_end(), want, "{cmd}");
            assert!(s.run("review.changes", &json!({})).unwrap().as_array().unwrap().is_empty(), "{cmd}");
        }
    }
}

/// #1317: rejecting a row formatting change keeps the row's own insertion mark, so Reject All
/// on an inserted row with a formatting change removes it instead of leaving an empty table.
#[test]
fn reject_all_removes_an_inserted_row_with_a_formatting_change() {
    use std::sync::Arc;
    use wordcraft_doc::{Block, Revision, RevisionKind, Table, props::PropChange, props::RowProps};
    let mut d = Document::from_text("Before\nAfter");
    d.revisions.push(Revision { kind: RevisionKind::Insert, author: "A".into(), date: String::new(), move_name: None });
    d.revisions.push(Revision { kind: RevisionKind::Format, author: "A".into(), date: String::new(), move_name: None });
    let mut t = Table::new(1, 1, 100.0);
    t.rows[0].props.ins = Some(0);
    t.rows[0].props.height = Some(30.0);
    t.rows[0].props.fmt_change = PropChange::boxed(1, RowProps::default());
    d.body.insert(1, Arc::new(Block::Table(t)));
    let mut s = Session::new(d);
    s.run("review.rejectAll", &json!({})).unwrap();
    assert!(s.doc.body.iter().all(|b| !matches!(**b, Block::Table(_))), "the inserted row and its table are gone");
}

/// #1317 (#10): a comment whose marks Accept All deletes with the text around them goes too,
/// with its text, rather than staying as a comment on nothing.
#[test]
fn accept_all_removes_a_comment_whose_anchors_were_deleted() {
    use wordcraft_doc::para::InlineObject;
    use wordcraft_doc::{CharProps, Comment, Paragraph, PartKind, Revision, RevisionKind, para_block};
    let mut d = Document::from_text("Intro");
    d.revisions.push(Revision { kind: RevisionKind::Delete, author: "A".into(), date: String::new(), move_name: None });
    let part = d.try_add_part(PartKind::Comment, vec![para_block(Paragraph::with_text("Note", CharProps::default()))]).unwrap();
    d.comments.insert(0, Comment { author: "A".into(), part, ..Default::default() });
    let mut p = Paragraph::with_text("Keep ", CharProps::default());
    let n = p.len();
    p.insert_object(n, InlineObject::CommentStart { id: 0 }, &CharProps::default()).unwrap();
    let n = p.len();
    p.insert_text(n, "gone", &CharProps::default()).unwrap();
    let n = p.len();
    p.insert_object(n, InlineObject::CommentEnd { id: 0 }, &CharProps::default()).unwrap();
    let n = p.len();
    p.format(5, n, &|c| c.del = Some(0)).unwrap();
    d.body.push(para_block(p));
    let mut s = Session::new(d);
    s.run("review.acceptAll", &json!({})).unwrap();
    assert!(s.doc.comments.is_empty());
    assert!(!s.doc.parts.contains_key(&part));
}

/// #1317 (N3): a reply that keeps its own marks stays when Accept All removes its parent's.
#[test]
fn accept_all_keeps_a_reply_that_still_has_marks() {
    use wordcraft_doc::para::InlineObject;
    use wordcraft_doc::{CharProps, Comment, Paragraph, PartKind, Revision, RevisionKind, para_block};
    let mut d = Document::from_text("Intro");
    d.revisions.push(Revision { kind: RevisionKind::Delete, author: "A".into(), date: String::new(), move_name: None });
    for id in [0u32, 1] {
        let part = d.try_add_part(PartKind::Comment, vec![para_block(Paragraph::with_text("Note", CharProps::default()))]).unwrap();
        d.comments.insert(id, Comment { author: "A".into(), part, parent: (id == 1).then_some(0), ..Default::default() });
    }
    let plain = CharProps::default();
    let mut p = Paragraph::with_text("Keep", plain.clone());
    for (obj, text) in [(InlineObject::CommentStart { id: 1 }, ""), (InlineObject::CommentEnd { id: 1 }, " ")] {
        let n = p.len();
        p.insert_object(n, obj, &plain).unwrap();
        let n = p.len();
        p.insert_text(n, text, &plain).unwrap();
    }
    let start = p.len();
    for (obj, text) in [(InlineObject::CommentStart { id: 0 }, "gone"), (InlineObject::CommentEnd { id: 0 }, "")] {
        let n = p.len();
        p.insert_object(n, obj, &plain).unwrap();
        let n = p.len();
        p.insert_text(n, text, &plain).unwrap();
    }
    let n = p.len();
    p.format(start, n, &|c| c.del = Some(0)).unwrap();
    d.body.push(para_block(p));
    let mut s = Session::new(d);
    s.run("review.acceptAll", &json!({})).unwrap();
    assert!(!s.doc.comments.contains_key(&0));
    assert!(s.doc.comments.contains_key(&1), "the reply with its own marks stays");
}
