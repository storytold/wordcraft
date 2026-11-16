use super::*;

// Find and Replace by formatting (#6147): a `format` filter on the find and replace commands and
// a `withFormat` for Replace with › Format.

/// "alpha beta gamma delta" / "second line" / "third", with `beta` bold, `gamma` bold and italic
/// and `elt` (inside `delta`) bold.
fn sample() -> Session {
    let mut s = s();
    run(&mut s, "document.setText", json!({"text": "alpha beta gamma delta\nsecond line\nthird"}));
    format_text(&mut s, "beta", "format.bold", json!({"value": true}));
    format_text(&mut s, "gamma", "format.bold", json!({"value": true}));
    format_text(&mut s, "gamma", "format.italic", json!({"value": true}));
    format_text(&mut s, "elt", "format.bold", json!({"value": true}));
    s
}

fn format_text(s: &mut Session, needle: &str, command: &str, params: serde_json::Value) {
    run(s, "select.text", json!({"text": needle}));
    run(s, command, params);
}

/// The text of every match of the last search.
fn found(s: &Session) -> Vec<String> {
    s.find
        .results
        .iter()
        .map(|(a, b)| {
            let p = s.doc.para_at(a).expect("a match is in a paragraph");
            assert_eq!(a.path, b.path, "these matches stay in one paragraph");
            p.text[a.off..b.off].to_string()
        })
        .collect()
}

fn find_format(s: &mut Session, text: &str, format: serde_json::Value) -> Vec<String> {
    run(s, "edit.find", json!({"text": text, "format": format}));
    found(s)
}

#[test]
fn empty_text_finds_the_stretches_with_the_formatting() {
    let mut s = sample();
    // A stretch need not be a word: `elt` is part of `delta`, and a stretch ends at a space that is not bold.
    assert_eq!(find_format(&mut s, "", json!({"bold": true})), ["beta", "gamma", "elt"]);
    assert_eq!(find_format(&mut s, "", json!({"italic": true})), ["gamma"]);
    assert_eq!(find_format(&mut s, "", json!({"bold": true, "italic": true})), ["gamma"]);
    // Not bold: the other stretches, a paragraph mark ends each of them.
    assert_eq!(find_format(&mut s, "", json!({"bold": false})), ["alpha ", " ", " d", "a", "second line", "third"]);
    // The first match from the caret on is selected.
    run(&mut s, "select.text", json!({"text": "alpha"}));
    assert_eq!(run(&mut s, "edit.find", json!({"text": "", "format": {"bold": true}}))["count"], 3);
    assert_eq!(s.selected_text(), "beta");
}

#[test]
fn text_with_a_format_needs_every_character_formatted() {
    let mut s = sample();
    // `a` is bold in `beta` and `gamma` (twice), not in `alpha` or `delta`.
    assert_eq!(find_format(&mut s, "a", json!({"bold": true})), ["a", "a", "a"]);
    assert_eq!(find_format(&mut s, "delta", json!({"bold": true})), Vec::<String>::new(), "only `elt` of delta is bold");
    assert_eq!(find_format(&mut s, "elt", json!({"bold": true})), ["elt"]);
    assert_eq!(find_format(&mut s, "elt", json!({"bold": false})), Vec::<String>::new());
    // Without a format the text is found everywhere again.
    run(&mut s, "edit.find", json!({"text": "a", "format": null}));
    assert_eq!(s.find.results.len(), 6);
}

#[test]
fn a_format_is_what_the_text_shows_including_its_style() {
    let mut s = s();
    run(&mut s, "document.setText", json!({"text": "plain strong\nHeading text\nlast"}));
    format_text(&mut s, "strong", "format.charStyle", json!({"style": "Strong"}));
    format_text(&mut s, "Heading text", "para.style", json!({"style": "Heading 1"}));
    // Strong is bold by its style, with no direct bold on the run.
    assert_eq!(find_format(&mut s, "", json!({"bold": true})), ["strong"]);
    assert_eq!(find_format(&mut s, "", json!({"style": "strong"})), ["strong"]);
    // A paragraph style finds the paragraph's text; its name or id works.
    assert_eq!(find_format(&mut s, "", json!({"style": "Heading 1"})), ["Heading text"]);
    assert_eq!(find_format(&mut s, "text", json!({"style": "Heading1"})), ["text"]);
    assert_eq!(find_format(&mut s, "plain", json!({"style": "Heading 1"})), Vec::<String>::new());
}

#[test]
fn font_size_colour_highlight_and_the_rest_are_found() {
    let mut s = s();
    run(&mut s, "document.setText", json!({"text": "one two three four five six"}));
    format_text(&mut s, "one", "format.color", json!({"color": "FF0000"}));
    format_text(&mut s, "two", "format.highlight", json!({"color": "yellow"}));
    format_text(&mut s, "three", "format.size", json!({"size": 20}));
    format_text(&mut s, "four", "format.font", json!({"name": "Courier New"}));
    format_text(&mut s, "five", "format.underline", json!({"value": true}));
    format_text(&mut s, "six", "format.strikethrough", json!({"value": true}));
    assert_eq!(find_format(&mut s, "", json!({"color": "ff0000"})), ["one"]);
    assert_eq!(find_format(&mut s, "", json!({"highlight": true})), ["two"]);
    assert_eq!(find_format(&mut s, "", json!({"highlight": "Yellow"})), ["two"]);
    assert_eq!(find_format(&mut s, "", json!({"highlight": "red"})), Vec::<String>::new());
    assert_eq!(find_format(&mut s, "", json!({"size": 20})), ["three"]);
    assert_eq!(find_format(&mut s, "", json!({"font": "courier new"})), ["four"]);
    assert_eq!(find_format(&mut s, "", json!({"underline": true})), ["five"]);
    assert_eq!(find_format(&mut s, "", json!({"strike": true})), ["six"]);
    // Not highlighted: everything around "two".
    assert_eq!(find_format(&mut s, "", json!({"highlight": false})), ["one ", " three four five six"]);
}

#[test]
fn next_and_previous_visit_the_formatted_matches() {
    let mut s = sample();
    run(&mut s, "edit.find", json!({"text": "", "format": {"bold": true}}));
    assert_eq!(s.selected_text(), "beta");
    run(&mut s, "edit.findNext", json!({}));
    assert_eq!(s.selected_text(), "gamma");
    run(&mut s, "edit.findNext", json!({}));
    assert_eq!(s.selected_text(), "elt");
    run(&mut s, "edit.findNext", json!({}));
    assert_eq!(s.selected_text(), "beta");
    run(&mut s, "edit.findPrevious", json!({}));
    assert_eq!(s.selected_text(), "elt");
    // Advanced Find lists them all, without a text.
    let r = run(&mut s, "edit.advancedFind", json!({"format": {"italic": true}}));
    assert_eq!(r["count"], 1);
    assert_eq!(found(&s), ["gamma"]);
}

#[test]
fn replace_all_with_only_formats_changes_the_formatting_and_is_one_undo_step() {
    let mut s = sample();
    let before = text(&s);
    let r = run(&mut s, "edit.replaceAll", json!({"text": "", "format": {"bold": true}, "withFormat": {"underline": true, "bold": false}}));
    assert_eq!(r["replaced"], 3);
    assert_eq!(text(&s), before, "no text is replaced when only a format is given");
    assert_eq!(find_format(&mut s, "", json!({"bold": true})), Vec::<String>::new());
    assert_eq!(find_format(&mut s, "", json!({"underline": true})), ["beta", "gamma", "elt"]);
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(find_format(&mut s, "", json!({"bold": true})), ["beta", "gamma", "elt"]);
    assert_eq!(find_format(&mut s, "", json!({"underline": true})), Vec::<String>::new());
}

#[test]
fn replacing_text_can_format_the_new_text() {
    let mut s = sample();
    let r = run(&mut s, "edit.replaceAll", json!({"text": "alpha", "with": "omega", "withFormat": {"color": "0000FF", "size": 18}}));
    assert_eq!(r["replaced"], 1);
    assert!(text(&s).starts_with("omega beta"));
    assert_eq!(find_format(&mut s, "", json!({"color": "0000ff"})), ["omega"]);
    assert_eq!(find_format(&mut s, "", json!({"size": 18})), ["omega"]);
    // With a format on the match too: only bold `beta` is replaced, not other text.
    let r = run(&mut s, "edit.replaceAll", json!({"text": "a", "format": {"bold": true}, "with": "A", "withFormat": null}));
    assert_eq!(r["replaced"], 3);
    assert_eq!(text(&s), "omega betA gAmmA delta\nsecond line\nthird");
    // The new text takes the formatting of what it replaced.
    assert_eq!(find_format(&mut s, "A", json!({"bold": true})), ["A", "A", "A"]);
}

#[test]
fn with_format_and_no_text_keeps_the_found_text_and_can_set_a_paragraph_style() {
    let mut s = sample();
    let before = text(&s);
    let r = run(&mut s, "edit.replaceAll", json!({"text": "third", "with": "", "withFormat": {"style": "Heading 2"}}));
    assert_eq!(r["replaced"], 1);
    assert_eq!(text(&s), before);
    let id = s.doc.styles.find("Heading 2").map(|st| st.id.clone());
    let last = s.doc.para_paths(StoryRef::Body).pop().and_then(|p| s.doc.para(StoryRef::Body, &p).map(|p| p.props.style.clone()));
    assert_eq!(last, Some(id));
    // One match at a time (Replace) does the same.
    let mut s = sample();
    run(&mut s, "edit.find", json!({"text": "", "format": {"italic": true}}));
    run(&mut s, "edit.replace", json!({"text": "", "with": "", "withFormat": {"italic": false, "highlight": "yellow"}}));
    assert_eq!(find_format(&mut s, "", json!({"italic": true})), Vec::<String>::new());
    assert_eq!(find_format(&mut s, "", json!({"highlight": true})), ["gamma"]);
}

#[test]
fn with_format_belongs_to_one_call() {
    let mut s = sample();
    run(&mut s, "edit.replaceAll", json!({"text": "alpha", "with": "omega", "withFormat": {"color": "0000FF"}}));
    // A later plain Replace All formats nothing, and an empty replacement deletes as it always did.
    run(&mut s, "edit.replaceAll", json!({"text": "beta", "with": "B"}));
    assert_eq!(find_format(&mut s, "", json!({"color": "0000ff"})), ["omega"]);
    run(&mut s, "edit.find", json!({"text": "x", "format": null}));
    run(&mut s, "edit.replaceAll", json!({"text": "B", "with": ""}));
    assert!(text(&s).starts_with("omega  gamma"));
}

#[test]
fn replacing_found_formatting_needs_something_to_put_there() {
    let mut s = sample();
    let before = text(&s);
    // Found bold text with no replacement text or format would be deleted: refused.
    assert!(s.run("edit.replaceAll", &json!({"text": "", "format": {"bold": true}})).is_err());
    assert!(s.run("edit.replace", &json!({"text": "", "format": {"bold": true}})).is_err());
    assert_eq!(text(&s), before);
    // Replacement text works, as in Word.
    assert_eq!(run(&mut s, "edit.replaceAll", json!({"text": "", "format": {"italic": true}, "with": "G"}))["replaced"], 1);
    assert!(text(&s).contains(" G "));
}

#[test]
fn format_stays_until_cleared_and_hostile_values_are_refused() {
    let mut s = sample();
    run(&mut s, "edit.find", json!({"text": "a", "format": {"bold": true}}));
    assert_eq!(found(&s).len(), 3);
    // It is kept, like the other search options.
    run(&mut s, "edit.find", json!({"text": "a"}));
    assert_eq!(found(&s).len(), 3);
    run(&mut s, "edit.find", json!({"text": "a", "format": {}}));
    assert_eq!(found(&s).len(), 6);
    assert!(s.find.format.is_none(), "an empty format is No Formatting");
    run(&mut s, "edit.find", json!({"text": "a", "format": {"bold": true}}));
    run(&mut s, "edit.find", json!({"text": "a", "format": null}));
    assert!(s.find.format.is_none());

    let junk = [
        json!(5),
        json!("bold"),
        json!([1]),
        json!({"bold": "yes"}),
        json!({"nope": true}),
        json!({"size": 1e300}),
        json!({"size": -1}),
        json!({"size": "big"}),
        json!({"color": "zzz"}),
        json!({"highlight": 3}),
        json!({"highlight": "mauve"}),
        json!({"style": "No Such Style"}),
        json!({"style": "Table Grid"}),
        json!({"font": ""}),
        json!({"font": "x".repeat(500)}),
    ];
    for key in ["format", "withFormat"] {
        for j in &junk {
            // `withFormat` is read by the replace commands only.
            let commands: &[&str] = if key == "format" {
                &["edit.find", "edit.advancedFind", "edit.replace", "edit.replaceAll"]
            } else {
                &["edit.replace", "edit.replaceAll"]
            };
            for command in commands.iter().copied() {
                let mut p = json!({"text": "a", "with": "b"});
                p[key] = j.clone();
                assert!(s.run(command, &p).is_err(), "{command} {key} {j}");
            }
        }
    }
    // `format: null` with no text still has nothing to replace.
    assert!(self::s().run("edit.replaceAll", &json!({"format": null, "with": "x"})).is_err());
    // A refused value leaves the kept format as it was.
    run(&mut s, "edit.find", json!({"text": "a", "format": {"bold": true}}));
    assert!(s.run("edit.find", &json!({"text": "a", "format": {"bold": "yes"}})).is_err());
    assert_eq!(s.find.format.as_ref().and_then(|f| f.bold), Some(true));
    // Nothing above changed the text.
    assert_eq!(text(&s), "alpha beta gamma delta\nsecond line\nthird");
}

#[test]
fn formatting_changes_under_track_changes_replace_no_text() {
    let mut s = sample();
    let before = text(&s);
    run(&mut s, "review.trackChanges", json!({"value": true}));
    run(&mut s, "edit.replaceAll", json!({"text": "", "format": {"italic": true}, "withFormat": {"italic": false}}));
    assert_eq!(text(&s), before);
    assert!(!s.doc.revisions.is_empty(), "the formatting change is tracked");
    assert!(s.doc.revisions.iter().all(|r| r.kind == wordcraft_doc::RevisionKind::Format), "only formatting changes are tracked");
    assert_eq!(find_format(&mut s, "", json!({"italic": true})), Vec::<String>::new());
    // Replacing text tracks the replacement as usual, and the new text is formatted.
    run(&mut s, "edit.replaceAll", json!({"text": "second", "with": "2nd", "format": null, "withFormat": {"color": "00FF00"}}));
    assert!(!s.doc.revisions.is_empty());
    assert!(s.doc.revisions.iter().any(|r| r.kind == wordcraft_doc::RevisionKind::Insert));
    assert_eq!(find_format(&mut s, "", json!({"color": "00ff00"})), ["2nd"]);
    // The text marked deleted keeps its own formatting.
    let path = s.doc.para_paths(StoryRef::Body).get(1).cloned().unwrap();
    let para = s.doc.para(StoryRef::Body, &path).unwrap();
    assert!(para.run_ranges().any(|(_, c)| c.del.is_some()));
    assert!(para.run_ranges().filter(|(_, c)| c.del.is_some()).all(|(_, c)| c.color.is_none()));
}

#[test]
fn format_search_and_replace_all_cover_every_paragraph_of_a_big_document() {
    let mut s = s();
    let n = 6000;
    run(&mut s, "document.setText", json!({"text": vec!["ab cd"; n].join("\n")}));
    run(&mut s, "select.all", json!({}));
    run(&mut s, "format.bold", json!({"value": true}));
    run(&mut s, "edit.find", json!({"text": "", "format": {"bold": true}}));
    assert_eq!(s.find.results.len(), n, "one stretch per paragraph");
    run(&mut s, "edit.find", json!({"text": "cd", "format": {"bold": true}}));
    assert_eq!(s.find.results.len(), n);
    let r = run(&mut s, "edit.replaceAll", json!({"text": "", "format": {"bold": true}, "withFormat": {"italic": true}}));
    assert_eq!(r["replaced"], n);
    assert_eq!(find_format(&mut s, "cd", json!({"italic": true})).len(), n);
}

#[test]
fn format_only_replacement_with_matched_text_code_is_refused_and_changes_nothing() {
    let mut s = sample();
    let before = text(&s);
    for with in ["^&", "^&!"] {
        assert!(s.run("edit.replaceAll", &json!({"text": "", "format": {"bold": true}, "with": with})).is_err(), "{with}");
        assert!(s.run("edit.replace", &json!({"text": "", "format": {"bold": true}, "with": with})).is_err(), "{with}");
        assert_eq!(text(&s), before, "{with}: nothing is deleted");
    }
    // `^&` still works when there is text to find, and without special codes it is plain text.
    assert_eq!(run(&mut s, "edit.replaceAll", json!({"text": "beta", "format": {"bold": true}, "with": "^&!"}))["replaced"], 1);
    assert!(text(&s).contains("beta!"));
    assert_eq!(run(&mut s, "edit.replaceAll", json!({"text": "", "format": {"italic": true}, "with": "^&", "special": false}))["replaced"], 1);
    assert!(text(&s).contains(" ^& "));
}

#[test]
fn a_late_formatted_match_is_found_after_many_unformatted_candidates() {
    let mut s = s();
    run(&mut s, "document.setText", json!({"text": format!("{}\nQa", "a".repeat(100_000))}));
    format_text(&mut s, "Qa", "format.bold", json!({"value": true}));
    let r = run(&mut s, "edit.find", json!({"text": "a", "format": {"bold": true}}));
    assert_eq!(r["count"], 1, "the bold `a` comes after 100,000 plain ones");
    let r = run(&mut s, "edit.replaceAll", json!({"text": "a", "format": {"bold": true}, "with": "b"}));
    assert_eq!(r["replaced"], 1);
    assert!(text(&s).ends_with("\nQb"));
}

#[test]
fn a_paragraph_mark_match_needs_the_mark_to_have_the_formatting() {
    let mut s = s();
    run(&mut s, "document.setText", json!({"text": "one\ntwo\nthree"}));
    format_text(&mut s, "on", "format.bold", json!({"value": true}));
    format_text(&mut s, "hre", "format.bold", json!({"value": true}));
    // The text around the marks is bold in places, but no mark is bold.
    run(&mut s, "edit.find", json!({"text": "^p", "format": {"bold": true}}));
    assert!(s.find.results.is_empty());
    let before = text(&s);
    assert_eq!(run(&mut s, "edit.replaceAll", json!({"text": "^p", "format": {"bold": true}, "with": ""}))["replaced"], 0);
    assert_eq!(text(&s), before);
    // Once the first paragraph's mark is bold, only that mark matches.
    let path = s.doc.para_paths(StoryRef::Body).first().cloned().unwrap();
    s.doc.para_mut(StoryRef::Body, &path).unwrap().mark.bold = Some(true);
    run(&mut s, "edit.find", json!({"text": "^p", "format": {"bold": true}}));
    assert_eq!(s.find.results.len(), 1);
    assert_eq!(s.find.results[0].0.path, path);
}

/// The Replace with text is kept in the find state like the other search options, so a caller that
/// sends only `withFormat` after an earlier replacement still replaces with that text; only an
/// explicit `with: ""` makes the call format-only.
#[test]
fn with_format_alone_keeps_the_earlier_replace_with_text_and_an_empty_with_clears_it() {
    let format_only = json!({"text": "", "format": {"bold": true}, "withFormat": {"underline": true}});
    // 1. An earlier call left "X" as the Replace with text: the bold stretches become "X", underlined.
    let mut s = sample();
    run(&mut s, "edit.replaceAll", json!({"text": "second", "with": "X"}));
    let r = run(&mut s, "edit.replaceAll", format_only.clone());
    assert_eq!(r["replaced"], 3);
    assert_eq!(text(&s), "alpha X X dXa\nX line\nthird");
    assert_eq!(find_format(&mut s, "X", json!({"underline": true})).len(), 3);
    // 2. The same sequence with an explicit empty `with`: the text stays, only the formatting changes.
    let mut s = sample();
    let before = text(&s);
    run(&mut s, "edit.replaceAll", json!({"text": "second", "with": "X"}));
    let after_first = text(&s);
    assert_ne!(after_first, before);
    let r = run(&mut s, "edit.replaceAll", json!({"text": "", "format": {"bold": true}, "with": "", "withFormat": {"underline": true}}));
    assert_eq!(r["replaced"], 3);
    assert_eq!(text(&s), after_first, "an explicit empty `with` keeps the found text");
    assert_eq!(find_format(&mut s, "", json!({"underline": true})), ["beta", "gamma", "elt"]);
}

/// A formatted `^p` search asks for the paragraphs of each match: the work stays linear in the
/// number of paragraphs (counted steps, not time).
#[test]
fn paragraph_mark_search_by_format_does_work_linear_in_the_paragraphs() {
    let steps = |n: usize| {
        let mut s = s();
        run(&mut s, "document.setText", json!({"text": vec!["ab"; n].join("\n")}));
        for path in s.doc.para_paths(StoryRef::Body) {
            if let Ok(p) = s.doc.para_mut(StoryRef::Body, &path) {
                p.mark.bold = Some(true);
            }
        }
        run(&mut s, "select.all", json!({}));
        run(&mut s, "format.bold", json!({"value": true}));
        let mut count = 0;
        let work = wordcraft_doc::work::count(|| {
            count = run(&mut s, "edit.find", json!({"text": "^p", "format": {"bold": true}}))["count"].as_u64().unwrap_or(0);
        });
        assert_eq!(count as usize, n - 1, "every mark but the last one matches");
        work
    };
    let (small, big) = (steps(400), steps(3200));
    assert!(big < small * 16, "{small} steps for 400 paragraphs, {big} for 3200");
}

/// Bold the bytes `a..b` of the first paragraph.
fn bold_bytes(s: &mut Session, a: usize, b: usize) {
    s.sel = crate::Selection { anchor: Pos::body(0, a), focus: Pos::body(0, b) };
    run(s, "format.bold", json!({"value": true}));
}

fn spans(s: &Session) -> Vec<(usize, usize)> {
    s.find.results.iter().map(|(a, b)| (a.off, b.off)).collect()
}

/// After a candidate is dropped for its formatting, the search goes on one character after its
/// start, so a match that overlaps the dropped one is still found.
#[test]
fn a_candidate_overlapping_a_rejected_one_is_still_found() {
    let mut s = s();
    run(&mut s, "document.setText", json!({"text": "aaa"}));
    bold_bytes(&mut s, 1, 3);
    run(&mut s, "edit.find", json!({"text": "aa", "format": {"bold": true}}));
    assert_eq!(spans(&s), [(1, 3)], "`aa` at 0..2 is not all bold, 1..3 is");
    // Without a filter the matches stay non-overlapping, as before.
    run(&mut s, "edit.find", json!({"text": "aa", "format": null}));
    assert_eq!(spans(&s), [(0, 2)]);
}

/// An accepted match still ends the search window: Replace All never replaces overlapping text.
#[test]
fn replace_all_with_a_format_filter_does_not_overlap_accepted_matches() {
    let mut s = s();
    run(&mut s, "document.setText", json!({"text": "aaaa"}));
    bold_bytes(&mut s, 0, 4);
    assert_eq!(run(&mut s, "edit.find", json!({"text": "aa", "format": {"bold": true}}))["count"], 2);
    assert_eq!(spans(&s), [(0, 2), (2, 4)]);
    assert_eq!(run(&mut s, "edit.replaceAll", json!({"text": "aa", "format": {"bold": true}, "with": "b"}))["replaced"], 2);
    assert_eq!(text(&s), "bb");
}

/// Multi-byte text: the step after a rejected candidate stays on a character boundary.
#[test]
fn a_rejected_candidate_in_multibyte_text_steps_by_a_whole_character() {
    let mut s = s();
    run(&mut s, "document.setText", json!({"text": "äää"}));
    bold_bytes(&mut s, 2, 6);
    run(&mut s, "edit.find", json!({"text": "ää", "format": {"bold": true}}));
    assert_eq!(spans(&s), [(2, 6)], "the step after the rejected 0..4 is one `ä` (2 bytes)");
}
