use super::*;

#[test]
fn compare_marks_changes() {
    let mut s = Session::new(Document::from_text("The cat sat.\nKeep this.\nOld line."));
    let r = s.run("review.compare", &json!({"text": "The dog sat.\nKeep this.\nNew line here."})).unwrap();
    assert!(r["changes"].as_u64().unwrap() >= 2);
    s.run("review.acceptAll", &json!({})).unwrap();
    assert_eq!(s.doc.plain_text(StoryRef::Body), "The dog sat.\nKeep this.\nNew line here.");
}

#[test]
fn compare_options_ignore_case_and_white_space() {
    let original = "The Cat sat.\nTwo  spaces here.";
    let revised = "The cat sat.\nTwo spaces here.";
    // By default both differences are changes.
    let mut s = Session::new(Document::from_text(original));
    let r = s.run("review.compare", &json!({"text": revised, "author": "Reviewer"})).unwrap();
    assert_eq!(r["changes"], 2);
    assert!(s.doc.revisions.iter().any(|x| x.author == "Reviewer"));
    // Ignoring case and white space: no changes, and the result reads as the revised text.
    let mut s = Session::new(Document::from_text(original));
    let r = s.run("review.compare", &json!({"text": revised, "caseChanges": false, "whiteSpace": false})).unwrap();
    assert_eq!(r["changes"], 0);
    assert_eq!(s.doc.plain_text(StoryRef::Body), revised);
    // Character level marks only the changed letters.
    let mut s = Session::new(Document::from_text("colour"));
    s.run("review.compare", &json!({"text": "color", "level": "character"})).unwrap();
    let p = s.doc.body[0].as_para().unwrap();
    let deleted: String = p.run_ranges().filter(|(_, c)| c.del.is_some()).map(|(r, _)| p.text[r].to_string()).collect();
    assert_eq!(deleted, "u");
    // Two dates that happen to share digits in a different order ("01.09.2026" vs
    // "19.10.2026") must come out as one clean replacement, not a scrambled chain of
    // single-character deletions and insertions wherever a digit coincidentally recurs.
    let mut s = Session::new(Document::from_text("zawarta w dniu 01.09.2026 pomiędzy:"));
    s.run("review.compare", &json!({"text": "zawarta w dniu 19.10.2026 pomiędzy:", "level": "character"})).unwrap();
    let p = s.doc.body[0].as_para().unwrap();
    let deleted: String = p.run_ranges().filter(|(_, c)| c.del.is_some()).map(|(r, _)| p.text[r].to_string()).collect();
    let inserted: String = p.run_ranges().filter(|(_, c)| c.ins.is_some()).map(|(r, _)| p.text[r].to_string()).collect();
    assert_eq!(deleted, "01.09");
    assert_eq!(inserted, "19.10");
    assert_eq!(p.run_ranges().filter(|(_, c)| c.del.is_some()).count(), 1, "one clean deleted run, not several scattered ones");
    assert_eq!(p.run_ranges().filter(|(_, c)| c.ins.is_some()).count(), 1, "one clean inserted run, not several scattered ones");
    // Formatting differences become formatting changes.
    let mut s = Session::new(Document::from_text("Same words."));
    let mut rev = Document::from_text("Same words.");
    rev.format_range(&rev.start_of(StoryRef::Body), &rev.end_of(StoryRef::Body), &|c| c.bold = Some(true)).unwrap();
    let path = std::env::temp_dir().join(format!("wc-compare-{}.docx", std::process::id()));
    crate::io::save_path(&path, &rev).unwrap();
    let r = s.run("review.compare", &json!({"path": path.to_string_lossy()})).unwrap();
    let _ = std::fs::remove_file(&path);
    assert_eq!(r["changes"], 1);
    assert!(s.doc.revisions.iter().any(|x| x.kind == RevisionKind::Format));
}

#[test]
fn collapse_noise_matches_merges_short_interior_runs_but_keeps_prefix_suffix() {
    // "01.09" vs "19.10": a pure LCS finds a scrambled chain of single-character matches
    // ('1', '.', '0') threaded through what a reader expects to be one clean replacement.
    let ops = lcs_ops(&['0', '1', '.', '0', '9'], &['1', '9', '.', '1', '0'], &mut { u64::MAX });
    assert!(ops.contains(&Op::Same), "the raw LCS does find spurious matches here, or this test proves nothing");
    assert_eq!(collapse_noise_matches(&ops), vec![Op::Del, Op::Del, Op::Del, Op::Del, Op::Del, Op::Ins, Op::Ins, Op::Ins, Op::Ins, Op::Ins]);
    // A plain single-character replacement flanked by a real common prefix and suffix has no
    // interior match to begin with (the DP's middle is pure delete/insert), so it's untouched.
    let ops = lcs_ops(&['a', 'b', 'c', 'x', 'd', 'e', 'f'], &['a', 'b', 'c', 'y', 'd', 'e', 'f'], &mut { u64::MAX });
    assert_eq!(collapse_noise_matches(&ops), ops);
}

#[test]
fn compare_at_word_level_still_matches_a_whole_unchanged_word() {
    // `collapse_noise_matches` only runs for character-level compares (wired up in
    // `word_diff`): a single matched *word* between two edits is a real, meaningful match
    // (unlike a single matched character), and must never be folded into a replacement.
    let mut s = Session::new(Document::from_text("the cat sat"));
    s.run("review.compare", &json!({"text": "a cat ran"})).unwrap();
    let p = s.doc.body[0].as_para().unwrap();
    assert!(
        p.run_ranges().any(|(r, c)| c.del.is_none() && c.ins.is_none() && p.text[r].trim() == "cat"),
        "the unchanged word must survive as plain text"
    );
}

#[test]
fn compare_keeps_text_boxes_of_the_original() {
    // Text boxes are anchored in body paragraphs: one after changed text, one alone in an
    // otherwise empty paragraph, one after text the revised document keeps. Their anchors must
    // survive the compare (marked deleted, since the revised text has no boxes), so no text is lost.
    for show_in in ["original", "revised", "new"] {
        let mut s = Session::new(Document::from_text("Body one.\n\nKeep."));
        let mut boxes = Vec::new();
        for (para, off, text) in [(0usize, "Body one.".len(), "Alpha boxword gamma."), (1, 0, "Vml onlyword here."), (2, "Keep.".len(), "Kept box.")]
        {
            s.run("caret.set", &json!({"pos": Pos::body(para, off)})).unwrap();
            let id = s.run("insert.textBox", &json!({"text": text})).unwrap()["story"].as_u64().unwrap() as u32;
            boxes.push((id, text));
        }
        s.run("caret.set", &json!({"pos": Pos::body(0, 0)})).unwrap();
        s.run("review.compare", &json!({"text": "Totally new first.\nSecond new line.\nKeep.", "showIn": show_in})).unwrap();
        for (id, text) in &boxes {
            let anchor = s.doc.text_box_anchor(*id).unwrap_or_else(|| panic!("{show_in}: the box \"{text}\" lost its anchor"));
            assert_eq!(s.doc.plain_text(StoryRef::Part(*id)), *text);
            let p = s.doc.para(anchor.story, &anchor.path).unwrap();
            assert!(
                p.props_of_char(anchor.off - wordcraft_doc::para::OBJ.len_utf8()).del.is_some(),
                "{show_in}: the box \"{text}\" is not marked deleted"
            );
        }
        s.run("review.rejectAll", &json!({})).unwrap();
        assert_eq!(s.doc.plain_text(StoryRef::Body), "Body one.\n\nKeep.");
        for (id, text) in &boxes {
            assert!(s.doc.text_box_anchor(*id).is_some(), "{show_in}: rejecting lost the box \"{text}\"");
            assert_eq!(s.doc.plain_text(StoryRef::Part(*id)), *text);
        }
    }
}

#[test]
fn compare_with_a_revised_file_keeps_its_text_boxes_backed() {
    // Text boxes in both documents, the revised one from a file: every anchor in the result
    // shows a text box story of the result, and both documents' boxes are there, marked.
    fn with_boxes(text: &str, boxes: &[(usize, usize, &str)]) -> Session {
        let mut s = Session::new(Document::from_text(text));
        for (para, off, text) in boxes {
            s.run("caret.set", &json!({"pos": Pos::body(*para, *off)})).unwrap();
            s.run("insert.textBox", &json!({"text": text})).unwrap();
        }
        s
    }
    /// Each text box anchored in the body: its story's text, and whether its anchor is deleted
    /// or inserted.
    fn boxes(d: &Document) -> Vec<(String, bool, bool)> {
        let mut out = Vec::new();
        for path in d.para_paths(StoryRef::Body) {
            let p = d.para(StoryRef::Body, &path).unwrap();
            for (off, o) in p.objects_with_offsets() {
                for id in o.text_boxes() {
                    let part = d.parts.get(&id).unwrap_or_else(|| panic!("an anchor shows story {id}, which the result lacks"));
                    assert_eq!(part.kind, wordcraft_doc::PartKind::TextBox, "story {id}");
                    let c = p.props_of_char(off);
                    out.push((d.plain_text(StoryRef::Part(id)), c.del.is_some(), c.ins.is_some()));
                }
            }
        }
        out.sort();
        out
    }
    let rev = with_boxes("Body one.\nNew middle.\nKeep.", &[(1, "New middle.".len(), "Revised box."), (2, "Keep.".len(), "Kept box.")]);
    let path = std::env::temp_dir().join(format!("wc-compare-boxes-{}.docx", std::process::id()));
    crate::io::save_path(&path, &rev.doc).unwrap();
    let t = |t: &str, del, ins| (t.to_string(), del, ins);
    for show_in in ["original", "revised", "new"] {
        for (then, left) in [("review.acceptAll", "Revised box."), ("review.rejectAll", "Original box.")] {
            let mut s = with_boxes("Body one.\nOld middle.\nKeep.", &[(1, "Old middle.".len(), "Original box."), (2, "Keep.".len(), "Kept box.")]);
            s.run("review.compare", &json!({"path": path.to_string_lossy(), "showIn": show_in})).unwrap();
            assert_eq!(boxes(&s.doc), [t("Kept box.", false, false), t("Original box.", true, false), t("Revised box.", false, true)], "{show_in}");
            s.run(then, &json!({})).unwrap();
            assert_eq!(boxes(&s.doc), [t("Kept box.", false, false), t(left, false, false)], "{show_in} {then}");
        }
    }
    let _ = std::fs::remove_file(&path);
}

#[test]
fn compare_bounds_total_diff_work() {
    // Many long, slightly different paragraphs: each pair alone fits one comparison table,
    // but the work over all pairs must stay bounded. Pairs past the budget are compared
    // whole (deleted and inserted), and every pair still counts as a change.
    let words = 3000;
    let paras = 12;
    let doc = |rev: bool| -> String {
        (0..paras)
            .map(|p| (0..words).map(|w| if w % 10 == 0 && rev { format!("x{p}w{w}") } else { format!("p{p}w{w}") }).collect::<Vec<_>>().join(" "))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let (original, revised) = (doc(false), doc(true));
    let mut s = Session::new(Document::from_text(&original));
    let r = s.run("review.compare", &json!({"text": revised})).unwrap();
    assert_eq!(r["changes"], paras);
    // A pair diffed word by word keeps most of its text unmarked; one replaced whole keeps
    // only its common start and end.
    let word_diffed = |b: &Arc<Block>| {
        let p = b.as_para().unwrap();
        let same: usize = p.run_ranges().filter(|(_, c)| c.ins.is_none() && c.del.is_none()).map(|(r, _)| r.len()).sum();
        same * 2 > p.text.len()
    };
    assert!(word_diffed(&s.doc.body[0]));
    let diffed = s.doc.body.iter().filter(|b| word_diffed(b)).count() as u64;
    let cells = (words as u64) * (words as u64);
    assert!(diffed * cells <= 50_000_000 + cells, "{diffed} pairs diffed word by word");
    assert!(!word_diffed(s.doc.body.last().unwrap()));
    s.run("review.acceptAll", &json!({})).unwrap();
    assert_eq!(s.doc.plain_text(StoryRef::Body), revised);
}

#[test]
fn macros_record_and_run() {
    let mut s = Session::new(Document::new());
    s.run("tools.recordMacro", &json!({"name": "hi"})).unwrap();
    s.run("text.insert", &json!({"text": "Hi "})).unwrap();
    s.run("format.bold", &json!({})).unwrap();
    s.run("tools.recordMacro", &json!({})).unwrap();
    s.run("tools.macros", &json!({"run": "hi"})).unwrap();
    assert_eq!(s.doc.plain_text(StoryRef::Body), "Hi Hi ");
    s.run("edit.repeat", &json!({})).unwrap();
}

#[test]
fn autocorrect_on_space() {
    let mut s = Session::new(Document::new());
    for t in ["teh", " ", "Teh", " ", "(c)", " "] {
        s.run("text.insert", &json!({"text": t})).unwrap();
    }
    // The first word of a sentence is capitalised too.
    assert_eq!(s.doc.plain_text(StoryRef::Body), "The The © ");
}

#[test]
fn protection_blocks_edits() {
    let mut s = Session::new(Document::from_text("locked"));
    s.run("review.restrict", &json!({"mode": "readOnly"})).unwrap();
    assert!(s.run("text.insert", &json!({"text": "x"})).is_err());
    // The "none" mode is the shorthand for stopping the protection, and it doesn't stop a
    // protected document: only the explicit stop does (#1327 follow-up).
    assert!(s.run("review.restrict", &json!({"mode": "none"})).is_err());
    assert!(s.run("text.insert", &json!({"text": "x"})).is_err());
    s.run("review.restrict", &json!({"stop": true})).unwrap();
    assert!(s.run("text.insert", &json!({"text": "x"})).is_ok());
}

#[test]
fn accessibility_and_quick_parts() {
    let mut s = Session::new(crate::sample::sample_document());
    let r = s.run("file.accessibility", &json!({})).unwrap();
    assert!(r["issues"].is_array());
    s.run("select.text", &json!({"text": "Membership"})).unwrap();
    s.run("insert.quickParts", &json!({"save": "m"})).unwrap();
    s.run("caret.docEnd", &json!({})).unwrap();
    s.run("insert.quickParts", &json!({"insert": "m"})).unwrap();
    assert!(s.doc.plain_text(StoryRef::Body).ends_with("Membership"));
}

/// #407 Language: the selection's language and "do not check", the default language and
/// the detect preference; a bad tag is refused.
#[test]
fn proofing_language_options() {
    let mut s = Session::new(Document::from_text("Bonjour"));
    s.run("select.all", &json!({})).unwrap();
    s.run("review.language", &json!({"lang": "fr-FR", "noProof": true, "detect": false})).unwrap();
    let rc = s.doc.para_at(&Pos::body(0, 1)).map(|p| p.props_at(1).clone()).unwrap();
    assert_eq!((rc.lang.as_deref(), rc.no_proof), (Some("fr-FR"), Some(true)));
    assert!(!s.prefs.detect_language);
    s.run("review.language", &json!({"lang": "de-DE", "default": true})).unwrap();
    assert_eq!(s.doc.styles.default_chr.lang.as_deref(), Some("de-DE"));
    assert!(s.run("review.language", &json!({"lang": "<script>"})).is_err());
}

/// #2242: `insert.spreadsheet` fills a table from `values`, with a repeating header and a style.
#[test]
fn spreadsheet_takes_values_header_and_style() {
    let mut s = Session::new(Document::from_text(""));
    s.run("insert.spreadsheet", &json!({"values": [["Month", "Cases"], ["Jan", 12], ["Feb", null]], "header": true, "style": "TableGrid"})).unwrap();
    let Some(Block::Table(t)) = s.doc.body.iter().map(|b| &**b).find(|b| matches!(b, Block::Table(_))) else { panic!("no table") };
    assert_eq!((t.rows.len(), t.rows[0].cells.len()), (3, 2));
    assert!(t.rows[0].props.header && !t.rows[1].props.header);
    assert_eq!(t.rows[1].cells[1].blocks[0].as_para().unwrap().text, "12");
    assert_eq!(t.props.style.as_deref(), Some("TableGrid"));
}

/// An explicit `values: null` is a parameter error, not the legacy CSV sample table: the document
/// and the undo history stay as they were. Only an absent `values` falls back to `csv`.
#[test]
fn spreadsheet_rejects_an_explicit_null_values_without_changing_anything() {
    let mut s = Session::new(Document::from_text("keep"));
    let depth = s.undo_depth();
    for args in [json!({"values": null}), json!({"values": null, "csv": "a,b\n1,2"})] {
        assert!(s.run("insert.spreadsheet", &args).is_err(), "{args}");
        assert_eq!(s.doc.plain_text(StoryRef::Body), "keep");
        assert!(!s.doc.body.iter().any(|b| matches!(&**b, Block::Table(_))));
        assert_eq!(s.undo_depth(), depth, "{args}: no undo step");
    }
    s.run("insert.spreadsheet", &json!({"csv": "a,b\n1,2"})).unwrap();
    let Some(Block::Table(t)) = s.doc.body.iter().map(|b| &**b).find(|b| matches!(b, Block::Table(_))) else { panic!("no table") };
    assert_eq!((t.rows.len(), t.rows[0].cells.len()), (2, 2), "an absent `values` still takes the csv");
}

/// #2242: malformed `values` is an error, never a panic, and leaves the document alone.
#[test]
fn spreadsheet_rejects_hostile_values() {
    let wide: Vec<Value> = (0..64).map(|i| json!(i)).collect();
    let many: Vec<Value> = (0..1001).map(|_| json!(["x"])).collect();
    for bad in [
        json!("x"),
        json!(5),
        json!([]),
        json!(["a"]),
        json!([["a", "b"], ["c"]]),
        json!([wide]),
        json!(many),
        json!([[{"a": 1}]]),
        json!([[[1]]]),
        json!([[]]),
    ] {
        let mut s = Session::new(Document::from_text("keep"));
        assert!(s.run("insert.spreadsheet", &json!({"values": bad})).is_err(), "{bad}");
        assert_eq!(s.doc.plain_text(StoryRef::Body), "keep");
        assert!(!s.doc.body.iter().any(|b| matches!(&**b, Block::Table(_))));
    }
}

/// The inspector's removals delete straight from the document, so they aren't tracked: on a
/// document that tracks changes the text would go with no revision to show for it, and the
/// revision trail would be broken behind the user's back. The removal is refused instead,
/// and the document is left as it was.
#[test]
fn inspect_removals_are_refused_while_the_document_tracks_changes() {
    let mut s = Session::new(Document::from_text("Visible text.\nHidden text."));
    s.run("select.text", &json!({"text": "Hidden"})).unwrap();
    s.run("format.hidden", &json!({"value": true})).unwrap();
    // Tracking without Restrict Editing isolates the Inspector guard from the protected
    // document check added separately in #3285.
    s.run("review.trackChanges", &json!({"value": true})).unwrap();
    assert!(s.doc.settings.track_changes, "the document doesn't track changes");
    let before = s.doc.clone();
    let err = s.run("file.inspect", &json!({"remove": ["hidden"]})).unwrap_err();
    assert!(matches!(err, CmdError::Disabled(_)), "{err}");
    assert_eq!(s.doc, before, "the document changed despite the refusal");
    // Reading the document stays allowed.
    let found = s.run("file.inspect", &json!({})).unwrap();
    assert_eq!(found["found"]["hiddenText"], true, "{found}");
}

/// A document that doesn't track changes is inspected as before: the hidden text goes.
#[test]
fn inspect_removals_work_when_the_document_does_not_track_changes() {
    let mut s = Session::new(Document::from_text("Visible text.\nHidden text."));
    s.run("select.text", &json!({"text": "Hidden"})).unwrap();
    s.run("format.hidden", &json!({"value": true})).unwrap();
    assert!(!s.doc.settings.track_changes);
    let out = s.run("file.inspect", &json!({"remove": ["hidden"]})).unwrap();
    assert_eq!(out["residual"], json!([]), "{out}");
    let text = s.doc.plain_text(StoryRef::Body);
    assert!(!text.contains("Hidden"), "the hidden text is still there: {text}");
    assert!(text.contains("Visible text."), "{text}");
}

/// Save `revised` and Compare Documents against `original` (in memory), returning the result
/// session and the command's JSON. `tag` names the temp file (tests run in parallel).
fn compare_saved(tag: &str, original: Document, revised: &Document, opts: Value) -> (Session, Value) {
    let path = std::env::temp_dir().join(format!("wc-compare-fmt-{}-{tag}.docx", std::process::id()));
    crate::io::save_path(&path, revised).unwrap();
    let mut args = opts;
    if let Value::Object(o) = &mut args {
        o.insert("path".into(), json!(path.to_string_lossy()));
    }
    let mut s = Session::new(original);
    let r = s.run("review.compare", &args).unwrap();
    let _ = std::fs::remove_file(&path);
    (s, r)
}

/// A numbering-only difference emits `w:numberingChange` (`ParaProps.num_change`) instead of
/// `w:pPrChange` (`ParaProps.fmt_change`). The snapshot keeps the original numbering and
/// Reject restores it, in both directions (list added, list removed).
#[test]
fn compare_emits_numbering_change_for_numbering_only_differences() {
    use wordcraft_doc::props::NumRef;

    // Unit test: list added
    let old = Paragraph::with_text("A", CharProps::default());
    let mut new = Paragraph::with_text("A", CharProps::default());
    new.props.numbering = Some(NumRef { num: 1, level: 0 });
    let mut revs = Vec::new();
    crate::cmd::fmt_revisions::compare_formatting(&old, &mut new, &mut revs, None, "Reviewer", "2026-01-01T00:00:00Z");
    assert!(new.props.fmt_change.is_none(), "numbering-only difference must not emit w:pPrChange");
    let ch = new.props.num_change.as_ref().expect("numbering-only difference must emit w:numberingChange");
    assert_eq!(ch.previous_numbering(), None, "snapshot keeps previous un-numbered state");
    crate::cmd::fmt_revisions::resolve_para(&mut new.props, false);
    assert!(new.props.num_change.is_none(), "reject drops the change record");
    assert_eq!(new.props.numbering, None, "reject restores previous un-numbered state");

    // Unit test: list removed
    let mut old = Paragraph::with_text("B", CharProps::default());
    old.props.numbering = Some(NumRef { num: 2, level: 1 });
    let mut new = Paragraph::with_text("B", CharProps::default());
    let mut revs = Vec::new();
    crate::cmd::fmt_revisions::compare_formatting(&old, &mut new, &mut revs, None, "Reviewer", "2026-01-01T00:00:00Z");
    assert!(new.props.fmt_change.is_none(), "numbering-only difference must not emit w:pPrChange");
    let ch = new.props.num_change.as_ref().expect("numbering-only difference must emit w:numberingChange");
    assert_eq!(ch.previous_numbering(), Some(NumRef { num: 2, level: 1 }), "snapshot keeps exact original NumRef");
    crate::cmd::fmt_revisions::resolve_para(&mut new.props, false);
    assert!(new.props.num_change.is_none(), "reject drops the change record");
    assert_eq!(new.props.numbering, Some(NumRef { num: 2, level: 1 }), "reject restores exact original NumRef");

    // End to end: list applied to otherwise identical text
    let mut rev = Session::new(Document::from_text("A"));
    let r = rev.run("list.define", &json!({"levels": [{"format": "decimal", "text": "%1."}], "apply": false})).unwrap();
    let num = r["num"].as_u64().unwrap() as u32;
    rev.doc.para_mut(StoryRef::Body, &wordcraft_doc::Path::top(0)).unwrap().props.numbering = Some(NumRef { num, level: 0 });
    let (mut s, r) = compare_saved("num_added", Document::from_text("A"), &rev.doc, json!({}));
    assert_eq!(r["changes"], 1, "numbering change is one change: {r}");
    let p = s.doc.para(StoryRef::Body, &wordcraft_doc::Path::top(0)).unwrap();
    assert!(p.props.numbering.is_some(), "the revised list survives the comparison");
    assert!(p.props.fmt_change.is_none(), "list-only change does not emit w:pPrChange");
    let ch = p.props.num_change.as_ref().expect("list-only change emits w:numberingChange");
    assert_eq!(ch.previous_numbering(), None, "snapshot keeps previous un-numbered state");
    s.run("review.rejectAll", &json!({})).unwrap();
    let p = s.doc.para(StoryRef::Body, &wordcraft_doc::Path::top(0)).unwrap();
    assert!(p.props.numbering.is_none(), "reject restores the original un-numbered paragraph");
    assert!(p.props.num_change.is_none(), "reject drops the change record");

    // End to end: list removed from paragraph
    let numbered = |text: &str| {
        let mut s = Session::new(Document::from_text(text));
        let r = s.run("list.define", &json!({"levels": [{"format": "decimal", "text": "%1."}], "apply": false})).unwrap();
        let num = r["num"].as_u64().unwrap() as u32;
        s.doc.para_mut(StoryRef::Body, &wordcraft_doc::Path::top(0)).unwrap().props.numbering = Some(NumRef { num, level: 0 });
        s.doc
    };
    let original = numbered("A");
    let original_num =
        original.para(StoryRef::Body, &wordcraft_doc::Path::top(0)).unwrap().props.numbering.expect("the original paragraph is numbered");
    let (mut s, r) = compare_saved("num_removed", original, &Document::from_text("A"), json!({}));
    assert_eq!(r["changes"], 1, "removing a list is one change: {r}");
    let p = s.doc.para(StoryRef::Body, &wordcraft_doc::Path::top(0)).unwrap();
    assert!(p.props.numbering.is_none(), "the revised un-numbered paragraph survives");
    assert!(p.props.fmt_change.is_none(), "removing a list does not emit w:pPrChange");
    let ch = p.props.num_change.as_ref().expect("removing a list emits w:numberingChange");
    assert_eq!(ch.previous_numbering(), Some(original_num), "snapshot carries exactly the original numbering");
    s.run("review.rejectAll", &json!({})).unwrap();
    let p = s.doc.para(StoryRef::Body, &wordcraft_doc::Path::top(0)).unwrap();
    assert_eq!(p.props.numbering, Some(original_num), "reject restores exactly the original numbering");
    assert!(p.props.num_change.is_none(), "reject drops the change record");

    // DOCX roundtrip: list-removed paragraph with w:numberingChange serializes without w:ilvl/w:numId
    let mut roundtrip_doc = Document::from_text("List removed");
    roundtrip_doc.revisions.push(wordcraft_doc::Revision {
        kind: RevisionKind::Format,
        author: "Tester".into(),
        date: "2026-01-01T00:00:00Z".into(),
        move_name: None,
    });
    let mut nc = wordcraft_doc::props::NumChange::for_numbering(0, Some(NumRef { num: 42, level: 3 }));
    nc.original = "1.".into();
    roundtrip_doc.para_mut(StoryRef::Body, &wordcraft_doc::Path::top(0)).unwrap().props.num_change = Some(Box::new(nc));
    let docx_bytes = wordcraft_docx::write(&roundtrip_doc).expect("docx writes successfully");
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(&docx_bytes)).unwrap();
    let mut xml = String::new();
    std::io::Read::read_to_string(&mut z.by_name("word/document.xml").unwrap(), &mut xml).unwrap();
    assert!(xml.contains("<w:numberingChange"), "exported xml contains w:numberingChange: {xml}");
    assert!(xml.contains(r#"w:original="1.""#), "exported xml serializes original numbering string: {xml}");

    let read_doc = wordcraft_docx::read(&docx_bytes).expect("docx reads back successfully");
    let read_p = read_doc.para(StoryRef::Body, &wordcraft_doc::Path::top(0)).unwrap();
    assert!(read_p.props.numbering.is_none(), "read paragraph has no active numbering");
    let read_ch = read_p.props.num_change.as_ref().expect("read paragraph preserves num_change");
    assert_eq!(read_ch.original, "1.", "read paragraph preserves original numbering string");
    assert_eq!(read_ch.snapshot, None, "arbitrary original string does not create snapshot");

    // parse_original strictly accepts <digits>:<0-8> and rejects legacy Word formats
    assert_eq!(wordcraft_doc::props::NumChange::parse_original("42:3"), Some(NumRef { num: 42, level: 3 }));
    assert_eq!(wordcraft_doc::props::NumChange::parse_original("%1:1:0:."), None, "rejects legacy Word level format");
    assert_eq!(wordcraft_doc::props::NumChange::parse_original("12:%1:1:0:."), None, "rejects Word string starting with list id");
    assert_eq!(wordcraft_doc::props::NumChange::parse_original("1."), None, "rejects plain formatted number string");
}

/// Review checks: numbering changes interact cleanly with format changes, unnumbered baselines,
/// legacy unparseable original strings, and formatting balloon detection (`has_format_changes`).
#[test]
fn numbering_change_review_edge_cases() {
    use wordcraft_doc::props::{NumChange, NumRef};

    // 1. has_format_changes returns true when only num_change is present
    let mut doc = Document::from_text("Test");
    assert!(!crate::cmd::review::has_format_changes(&doc));
    doc.para_mut(StoryRef::Body, &wordcraft_doc::Path::top(0)).unwrap().props.num_change = NumChange::boxed(0, Some(NumRef { num: 1, level: 0 }));
    assert!(crate::cmd::review::has_format_changes(&doc), "has_format_changes detects num_change");

    // 2. resolve_para preserves active numbering when original is unparseable legacy format
    let mut p = Paragraph::with_text("Legacy", CharProps::default());
    p.props.numbering = Some(NumRef { num: 5, level: 1 });
    p.props.num_change = Some(Box::new(NumChange { rev: 0, original: "%1:1:0:.".into(), snapshot: None }));
    crate::cmd::fmt_revisions::resolve_para(&mut p.props, false);
    assert_eq!(p.props.numbering, Some(NumRef { num: 5, level: 1 }), "unparseable legacy original preserves active numbering");
    assert!(p.props.num_change.is_none());

    // 3. resolve_para clears numbering when original is explicitly empty or "none"
    let mut p = Paragraph::with_text("None", CharProps::default());
    p.props.numbering = Some(NumRef { num: 5, level: 1 });
    p.props.num_change = Some(Box::new(NumChange { rev: 0, original: "none".into(), snapshot: None }));
    crate::cmd::fmt_revisions::resolve_para(&mut p.props, false);
    assert_eq!(p.props.numbering, None, "explicitly 'none' original clears numbering");

    // 4. Preserve existing formatting baseline alongside numbering change
    let mut old_p = Paragraph::with_text("Formatted", CharProps::default());
    old_p.props.align = Some(wordcraft_doc::props::Align::Center);
    let mut new_p = Paragraph::with_text("Formatted", CharProps::default());
    new_p.props.align = Some(wordcraft_doc::props::Align::Center);
    new_p.props.fmt_change = wordcraft_doc::props::PropChange::boxed(0, wordcraft_doc::props::ParaProps::default());
    new_p.props.numbering = Some(NumRef { num: 1, level: 0 });

    let mut revs = Vec::new();
    crate::cmd::fmt_revisions::compare_formatting(&old_p, &mut new_p, &mut revs, None, "Reviewer", "2026-01-01T00:00:00Z");
    assert!(new_p.props.fmt_change.is_some(), "recording num_change preserves earlier fmt_change baseline");
    assert!(new_p.props.num_change.is_some(), "num_change is recorded");

    // 5. Compare baseline uses original numbering even if original has a same-author revision
    let mut revs = Vec::new();
    let mut orig_para = Paragraph::with_text("Step", CharProps::default());
    orig_para.props.numbering = Some(NumRef { num: 1, level: 1 });
    // orig_para had a prior revision where previous numbering was None
    orig_para.props.num_change = Some(Box::new(NumChange::for_numbering(0, None)));
    revs.push(wordcraft_doc::Revision { kind: RevisionKind::Format, author: "Author".into(), date: "2026-01-01T00:00:00Z".into(), move_name: None });

    let mut revised_para = Paragraph::with_text("Step", CharProps::default());
    revised_para.props.numbering = None; // Matches prior revision baseline, but differs from active numbering!

    crate::cmd::fmt_revisions::compare_formatting(&orig_para, &mut revised_para, &mut revs, None, "Author", "2026-01-01T00:00:00Z");
    assert!(revised_para.props.num_change.is_some(), "Compare does not clear num_change when revised matches prior revision baseline");
    assert_eq!(
        revised_para.props.num_change.as_ref().unwrap().previous_numbering(),
        Some(NumRef { num: 1, level: 1 }),
        "Compare baseline is original paragraph's active numbering"
    );
    crate::cmd::fmt_revisions::resolve_para(&mut revised_para.props, false);
    assert_eq!(revised_para.props.numbering, Some(NumRef { num: 1, level: 1 }), "reject restores original paragraph active numbering");
}

/// Verify independent resolution of numbering and formatting revisions, and edit coalescing.
#[test]
fn numbering_change_independent_resolution_and_coalescing() {
    use wordcraft_doc::props::{NumChange, NumRef};

    // 6. Restoring both baselines when a paragraph carries both fmt_change and num_change
    let mut p_both = Paragraph::with_text("Both", CharProps::default());
    p_both.props.numbering = Some(NumRef { num: 1, level: 0 });
    p_both.props.num_change = Some(Box::new(NumChange::for_numbering(0, None)));
    let old_fmt =
        wordcraft_doc::props::ParaProps { numbering: Some(NumRef { num: 1, level: 0 }), align: None, ..wordcraft_doc::props::ParaProps::default() };
    p_both.props.align = Some(wordcraft_doc::props::Align::Center);
    p_both.props.fmt_change = wordcraft_doc::props::PropChange::boxed(1, old_fmt);

    crate::cmd::fmt_revisions::resolve_para(&mut p_both.props, false);
    assert_eq!(p_both.props.align, None, "reject restores old alignment baseline");
    assert_eq!(p_both.props.numbering, None, "reject restores unnumbered baseline from num_change");

    // 7. review.changes emits both num_change and fmt_change independently before resolving
    let mut doc_both = Document::from_text("Both changes");
    doc_both.revisions.push(wordcraft_doc::Revision {
        kind: RevisionKind::Format,
        author: "Alice".into(),
        date: "2026-01-01T00:00:00Z".into(),
        move_name: None,
    });
    doc_both.revisions.push(wordcraft_doc::Revision {
        kind: RevisionKind::Format,
        author: "Bob".into(),
        date: "2026-01-01T00:00:00Z".into(),
        move_name: None,
    });
    let p = doc_both.para_mut(StoryRef::Body, &wordcraft_doc::Path::top(0)).unwrap();
    p.props.numbering = Some(NumRef { num: 1, level: 1 });
    p.props.num_change = Some(Box::new(NumChange::for_numbering(0, Some(NumRef { num: 1, level: 0 }))));
    p.props.align = Some(wordcraft_doc::props::Align::Center);
    p.props.fmt_change = wordcraft_doc::props::PropChange::boxed(1, wordcraft_doc::props::ParaProps::default());
    let mut s_both = Session::new(doc_both);
    let chs = s_both.run("review.changes", &json!({})).unwrap();
    let ch_array = chs.as_array().expect("array of changes");
    assert_eq!(ch_array.len(), 2, "both numbering and alignment changes are emitted");
    assert_eq!(ch_array[0]["author"], "Alice", "numbering change author");
    assert_eq!(ch_array[1]["author"], "Bob", "formatting change author");

    // 7a. Accept numbering change independently, keeping formatting change pending
    s_both.sel = Selection::caret(wordcraft_doc::Pos { story: StoryRef::Body, path: wordcraft_doc::Path::top(0), off: 0 });
    s_both.run("review.accept", &json!({"next": false})).unwrap();
    let p_after_accept_num = s_both.doc.para(StoryRef::Body, &wordcraft_doc::Path::top(0)).unwrap();
    assert!(p_after_accept_num.props.num_change.is_none(), "numbering change was accepted and cleared");
    assert!(p_after_accept_num.props.fmt_change.is_some(), "formatting change remains pending");
    assert_eq!(p_after_accept_num.props.align, Some(wordcraft_doc::props::Align::Center));

    // 7b. Reject formatting change independently, restoring alignment without affecting numbering
    s_both.sel = Selection {
        anchor: wordcraft_doc::Pos { story: StoryRef::Body, path: wordcraft_doc::Path::top(0), off: 0 },
        focus: wordcraft_doc::Pos { story: StoryRef::Body, path: wordcraft_doc::Path::top(0), off: p_after_accept_num.len() },
    };
    s_both.run("review.reject", &json!({"next": false})).unwrap();
    let p_after_reject_fmt = s_both.doc.para(StoryRef::Body, &wordcraft_doc::Path::top(0)).unwrap();
    assert!(p_after_reject_fmt.props.fmt_change.is_none(), "formatting change resolved");
    assert_eq!(p_after_reject_fmt.props.align, None, "alignment restored to None");
    assert_eq!(p_after_reject_fmt.props.numbering, Some(NumRef { num: 1, level: 1 }), "numbering remains active");

    // 8. ID collision through adoption: Reject restores remapped list ID and save persists correct list
    let mut our_doc = Document::from_text("Our text");
    let our_list = Arc::make_mut(&mut our_doc.numbering).add_list(wordcraft_doc::numbering::ListKind::Bullet);
    let mut their_doc = Document::from_text("Their text");
    their_doc.revisions.push(wordcraft_doc::Revision {
        kind: RevisionKind::Format,
        author: "Other".into(),
        date: "2026-01-01T00:00:00Z".into(),
        move_name: None,
    });
    let their_list = Arc::make_mut(&mut their_doc.numbering).add_list(wordcraft_doc::numbering::ListKind::Numbered);
    assert_eq!(their_list, our_list, "deliberate source and target list ID collision");
    let tp = their_doc.para_mut(StoryRef::Body, &wordcraft_doc::Path::top(0)).unwrap();
    tp.props.numbering = None;
    tp.props.num_change = Some(Box::new(NumChange::for_numbering(0, Some(NumRef { num: their_list, level: 0 }))));

    let frag = our_doc.adopt(their_doc);
    let mut s = Session::new(our_doc);
    s.doc.body = frag.blocks.into_iter().map(std::sync::Arc::new).collect();
    s.run("review.rejectAll", &json!({})).unwrap();

    let rejected_para = s.doc.para(StoryRef::Body, &wordcraft_doc::Path::top(0)).unwrap();
    let restored_num = rejected_para.props.numbering.expect("reject restored numbering");
    assert_ne!(restored_num.num, our_list, "restored numbering avoids collision with our list");

    let docx_bytes = wordcraft_docx::write(&s.doc).expect("DOCX save succeeds");
    let reread = wordcraft_docx::read(&docx_bytes).expect("DOCX read succeeds");
    let reread_para = reread.para(StoryRef::Body, &wordcraft_doc::Path::top(0)).unwrap();
    assert_eq!(reread_para.props.numbering, Some(restored_num), "persisted list assignment roundtrips cleanly");

    // 9. Coalescing successive numbering edits in a session preserves original baseline
    let mut doc_track = Document::from_text("List item");
    doc_track.para_mut(StoryRef::Body, &wordcraft_doc::Path::top(0)).unwrap().props.numbering = Some(NumRef { num: 1, level: 0 });
    let mut s_track = Session::new(doc_track);
    s_track.doc.settings.track_changes = true;
    s_track.prefs.markup.track_formatting = true;
    s_track.author = "Author".into();
    // Edit 1: change level from 0 to 1
    s_track.run("para.listLevel", &json!({"level": 1})).unwrap();
    let p1 = s_track.doc.para(StoryRef::Body, &wordcraft_doc::Path::top(0)).unwrap();
    assert_eq!(p1.props.num_change.as_ref().unwrap().previous_numbering(), Some(NumRef { num: 1, level: 0 }));
    // Edit 2: change level to 2 (coalesces with edit 1, keeping baseline level 0)
    s_track.run("para.listLevel", &json!({"level": 2})).unwrap();
    let p2 = s_track.doc.para(StoryRef::Body, &wordcraft_doc::Path::top(0)).unwrap();
    assert_eq!(
        p2.props.num_change.as_ref().unwrap().previous_numbering(),
        Some(NumRef { num: 1, level: 0 }),
        "coalesced edit preserves original baseline level 0"
    );
    // Edit 3: restore baseline level 0 (clears num_change)
    s_track.run("para.listLevel", &json!({"level": 0})).unwrap();
    let p3 = s_track.doc.para(StoryRef::Body, &wordcraft_doc::Path::top(0)).unwrap();
    assert!(p3.props.num_change.is_none(), "restoring baseline clears num_change");

    // 10. Successive edits preserve unparseable legacy original
    let mut s_legacy = Session::new(Document::from_text("Legacy item"));
    s_legacy.doc.settings.track_changes = true;
    s_legacy.prefs.markup.track_formatting = true;
    s_legacy.author = "Author".into();
    s_legacy.doc.revisions.push(wordcraft_doc::Revision {
        kind: RevisionKind::Format,
        author: "Author".into(),
        date: "2026-01-01T00:00:00Z".into(),
        move_name: None,
    });
    let p_legacy = s_legacy.doc.para_mut(StoryRef::Body, &wordcraft_doc::Path::top(0)).unwrap();
    p_legacy.props.numbering = Some(NumRef { num: 1, level: 0 });
    p_legacy.props.num_change = Some(Box::new(NumChange { rev: 0, original: "%1:1:0:.".into(), snapshot: None }));
    // Author modifies list level
    s_legacy.run("para.listLevel", &json!({"level": 1})).unwrap();
    let mut p_legacy_after = s_legacy.doc.para(StoryRef::Body, &wordcraft_doc::Path::top(0)).unwrap().clone();
    let nc_legacy = p_legacy_after.props.num_change.as_ref().expect("num_change preserved");
    assert_eq!(nc_legacy.original, "%1:1:0:.", "unparseable original preserved across edits");
    crate::cmd::fmt_revisions::resolve_para(&mut p_legacy_after.props, false);
    assert_eq!(p_legacy_after.props.numbering, Some(NumRef { num: 1, level: 1 }), "unparseable original preserves active numbering on reject");

    // 11. Coalescing list additions, formatting changes, and list removals preserves unnumbered baseline
    let mut s_coalesce = Session::new(Document::from_text("Unnumbered initially"));
    s_coalesce.doc.settings.track_changes = true;
    s_coalesce.prefs.markup.track_formatting = true;
    s_coalesce.author = "Author".into();
    // Step 1: Add bullets
    s_coalesce.run("para.bullets", &json!({})).unwrap();
    let p_c1 = s_coalesce.doc.para(StoryRef::Body, &wordcraft_doc::Path::top(0)).unwrap();
    assert!(p_c1.props.numbering.is_some());
    assert!(p_c1.props.num_change.is_some());
    // Step 2: Format paragraph (center alignment)
    s_coalesce.run("para.alignCenter", &json!({})).unwrap();
    let p_c2 = s_coalesce.doc.para(StoryRef::Body, &wordcraft_doc::Path::top(0)).unwrap();
    assert_eq!(p_c2.props.align, Some(wordcraft_doc::props::Align::Center));
    assert!(p_c2.props.fmt_change.is_some());
    // Verify fmt_change.old does not retain intermediate numbering
    assert!(p_c2.props.fmt_change.as_ref().unwrap().old.numbering.is_none(), "fmt_change baseline strips numbering");
    // Step 3: Remove bullets (toggle off)
    s_coalesce.run("para.bullets", &json!({"off": true})).unwrap();
    let p_c3 = s_coalesce.doc.para(StoryRef::Body, &wordcraft_doc::Path::top(0)).unwrap();
    assert!(p_c3.props.numbering.is_none_or(|n| n.num == 0), "numbering removed");
    assert!(p_c3.props.num_change.is_none(), "num_change coalesced back to None baseline");
    assert!(p_c3.props.fmt_change.is_some(), "alignment fmt_change remains active");
    // Step 4: Reject all restores unnumbered and left-aligned baseline
    s_coalesce.run("review.rejectAll", &json!({})).unwrap();
    let p_c4 = s_coalesce.doc.para(StoryRef::Body, &wordcraft_doc::Path::top(0)).unwrap();
    assert!(p_c4.props.numbering.is_none_or(|n| n.num == 0), "baseline unnumbered is preserved after rejection");
    assert_eq!(p_c4.props.align, None, "alignment restored to default");
}

/// Verification for DOCX roundtrip of snapshot-only list removals and imported w:pPrChange rejection.
#[test]
fn numbering_change_snapshot_persistence_and_imported_ppr_change() {
    use wordcraft_doc::props::{NumChange, NumRef, ParaProps, PropChange};

    // 1. Snapshot-only list removal survives DOCX save, reload, and Reject restores the removed list.
    let mut doc = Document::from_text("List removal roundtrip");
    doc.revisions.push(wordcraft_doc::Revision {
        kind: RevisionKind::Format,
        author: "Alice".into(),
        date: "2026-01-01T00:00:00Z".into(),
        move_name: None,
    });
    let prior_num = NumRef { num: 42, level: 3 };
    let nc = wordcraft_doc::props::NumChange::for_numbering(0, Some(prior_num));
    assert_eq!(nc.original, "42:3", "for_numbering populates original with digits:level");
    doc.para_mut(StoryRef::Body, &wordcraft_doc::Path::top(0)).unwrap().props.num_change = Some(Box::new(nc));

    let bytes = wordcraft_docx::write(&doc).expect("docx writes successfully");
    let reloaded = wordcraft_docx::read(&bytes).expect("docx reads back successfully");
    let reloaded_p = reloaded.para(StoryRef::Body, &wordcraft_doc::Path::top(0)).unwrap();
    assert!(reloaded_p.props.numbering.is_none(), "reloaded paragraph is unnumbered");
    let reloaded_nc = reloaded_p.props.num_change.as_ref().expect("num_change preserved after reload");
    assert_eq!(reloaded_nc.original, "42:3");
    assert_eq!(reloaded_nc.snapshot, Some(prior_num), "snapshot recovered from original across reload");

    let mut session = Session::new(reloaded);
    session.run("review.rejectAll", &json!({})).unwrap();
    let rejected_p = session.doc.para(StoryRef::Body, &wordcraft_doc::Path::top(0)).unwrap();
    assert_eq!(rejected_p.props.numbering, Some(prior_num), "rejecting restores removed list from snapshot");
    assert!(rejected_p.props.num_change.is_none(), "num_change cleared after reject");

    // Also test when c.original was empty but c.snapshot was Some(prior_num):
    // writer serializes snapshot into w:original and reload restores it.
    let mut doc_empty_orig = Document::from_text("Empty original");
    doc_empty_orig.revisions.push(wordcraft_doc::Revision {
        kind: RevisionKind::Format,
        author: "Alice".into(),
        date: "2026-01-01T00:00:00Z".into(),
        move_name: None,
    });
    doc_empty_orig.para_mut(StoryRef::Body, &wordcraft_doc::Path::top(0)).unwrap().props.num_change =
        Some(Box::new(NumChange { rev: 0, original: String::new(), snapshot: Some(prior_num) }));
    let bytes_empty = wordcraft_docx::write(&doc_empty_orig).expect("docx writes empty orig");
    let reloaded_empty = wordcraft_docx::read(&bytes_empty).expect("docx reads empty orig");
    let reloaded_empty_nc = reloaded_empty.para(StoryRef::Body, &wordcraft_doc::Path::top(0)).unwrap().props.num_change.as_ref().unwrap();
    assert_eq!(reloaded_empty_nc.original, "42:3");
    assert_eq!(reloaded_empty_nc.snapshot, Some(prior_num));

    // 2. Imported w:pPrChange with prior numbering and no num_change restores numbering on reject.
    let mut p_imported = ParaProps {
        numbering: Some(NumRef { num: 5, level: 1 }),
        fmt_change: Some(Box::new(PropChange { rev: 0, old: ParaProps { numbering: Some(NumRef { num: 3, level: 0 }), ..ParaProps::default() } })),
        ..ParaProps::default()
    };
    crate::cmd::fmt_revisions::resolve_para_fmt(&mut p_imported, false);
    assert_eq!(p_imported.numbering, Some(NumRef { num: 3, level: 0 }), "imported prior numbering restored on reject");
    assert!(p_imported.fmt_change.is_none());

    // 3. Imported w:pPrChange when num_change IS present preserves independent num_change
    let mut p_both = ParaProps {
        numbering: Some(NumRef { num: 5, level: 1 }),
        fmt_change: Some(Box::new(PropChange { rev: 0, old: ParaProps { numbering: Some(NumRef { num: 3, level: 0 }), ..ParaProps::default() } })),
        num_change: Some(Box::new(NumChange::for_numbering(0, Some(NumRef { num: 7, level: 2 })))),
        ..ParaProps::default()
    };
    crate::cmd::fmt_revisions::resolve_para_fmt(&mut p_both, false);
    assert_eq!(p_both.numbering, Some(NumRef { num: 5, level: 1 }), "active numbering preserved for independent num_change");
    assert!(p_both.num_change.is_some(), "num_change preserved");
    crate::cmd::fmt_revisions::resolve_para_num(&mut p_both, false);
    assert_eq!(p_both.numbering, Some(NumRef { num: 7, level: 2 }), "num_change resolved independently");

    // 4. Engine-recorded formatting baseline (ch.old.numbering == None) never clears active numbering
    let mut p_engine = ParaProps {
        numbering: Some(NumRef { num: 10, level: 0 }),
        fmt_change: Some(Box::new(PropChange {
            rev: 0,
            old: ParaProps { align: Some(wordcraft_doc::props::Align::Center), numbering: None, ..ParaProps::default() },
        })),
        ..ParaProps::default()
    };
    crate::cmd::fmt_revisions::resolve_para_fmt(&mut p_engine, false);
    assert_eq!(p_engine.numbering, Some(NumRef { num: 10, level: 0 }), "active numbering preserved against stripped baseline");
}

/// Verification for tracking removal of style-inherited numbering and restoring fmt_change during Compare.
#[test]
fn style_inherited_numbering_and_compare_fmt_change_preservation() {
    use wordcraft_doc::props::{NumRef, ParaProps, PropChange};
    use wordcraft_doc::styles::{Style, StyleKind};

    // 1. Style-inherited numbering removal emits num_change
    let mut doc_styled = Document::from_text("List style text");
    Arc::make_mut(&mut doc_styled.styles).upsert(Style {
        id: "NumberedStyle".into(),
        name: "NumberedStyle".into(),
        kind: StyleKind::Paragraph,
        para: ParaProps { numbering: Some(NumRef { num: 1, level: 0 }), ..ParaProps::default() },
        ..Default::default()
    });

    let p = doc_styled.para_mut(StoryRef::Body, &wordcraft_doc::Path::top(0)).unwrap();
    p.props.style = Some("NumberedStyle".into());
    p.props.numbering = None;

    let mut session = Session::new(doc_styled);
    session.doc.settings.track_changes = true;
    session.prefs.markup.track_formatting = true;
    session.author = "Author".into();

    session.run("para.bullets", &json!({"off": true})).unwrap();
    let p_after = session.doc.para(StoryRef::Body, &wordcraft_doc::Path::top(0)).unwrap();
    assert_eq!(p_after.props.numbering, Some(NumRef { num: 0, level: 0 }));
    assert!(p_after.props.num_change.is_some(), "removing style-inherited numbering tracks num_change");

    // Coalescing when neither value inherits a list: no spurious num_change
    let doc_plain = Document::from_text("Plain text");
    let mut s_plain = Session::new(doc_plain);
    s_plain.doc.settings.track_changes = true;
    s_plain.prefs.markup.track_formatting = true;
    s_plain.author = "Author".into();
    s_plain.run("para.bullets", &json!({"off": true})).unwrap();
    let p_plain = s_plain.doc.para(StoryRef::Body, &wordcraft_doc::Path::top(0)).unwrap();
    assert!(p_plain.props.num_change.is_none(), "no num_change when neither value inherits a list");

    // 2. Compare preserves old.fmt_change when new has no fmt_change and formatting matches
    let mut old_p = Paragraph::with_text("Formatted", CharProps::default());
    old_p.props.align = Some(wordcraft_doc::props::Align::Center);
    old_p.props.fmt_change = Some(Box::new(PropChange { rev: 0, old: ParaProps { align: None, ..ParaProps::default() } }));
    let mut new_p = Paragraph::with_text("Formatted", CharProps::default());
    new_p.props.align = Some(wordcraft_doc::props::Align::Center);
    new_p.props.fmt_change = None;

    let mut revs =
        vec![wordcraft_doc::Revision { kind: RevisionKind::Format, author: "Author".into(), date: "2026-01-01T00:00:00Z".into(), move_name: None }];
    crate::cmd::fmt_revisions::compare_formatting(&old_p, &mut new_p, &mut revs, None, "Reviewer", "2026-01-01T00:00:00Z");
    assert!(new_p.props.fmt_change.is_some(), "Compare preserves original fmt_change when revised formatting matches");
    assert_eq!(new_p.props.fmt_change.as_ref().unwrap().rev, 0);

    // 3. Compare with styles: original with numbered style and no direct numbering, revised with num: 0
    let mut styles = wordcraft_doc::StyleSheet::default();
    styles.upsert(Style {
        id: "NumberedStyle2".into(),
        name: "NumberedStyle2".into(),
        kind: StyleKind::Paragraph,
        para: ParaProps { numbering: Some(NumRef { num: 1, level: 0 }), ..ParaProps::default() },
        ..Default::default()
    });
    let mut old_styled_p = Paragraph::with_text("Inherited", CharProps::default());
    old_styled_p.props.style = Some("NumberedStyle2".into());
    old_styled_p.props.numbering = None;

    let mut new_styled_p = Paragraph::with_text("Inherited", CharProps::default());
    new_styled_p.props.style = Some("NumberedStyle2".into());
    new_styled_p.props.numbering = Some(NumRef { num: 0, level: 0 });

    let mut revs_styled = Vec::new();
    crate::cmd::fmt_revisions::compare_formatting(
        &old_styled_p,
        &mut new_styled_p,
        &mut revs_styled,
        Some(&styles),
        "Reviewer",
        "2026-01-01T00:00:00Z",
    );
    assert!(new_styled_p.props.num_change.is_some(), "Compare with styles detects removal of inherited numbering");
    assert_eq!(
        new_styled_p.props.num_change.as_ref().unwrap().previous_numbering(),
        None,
        "snapshot records None baseline to restore inherited numbering"
    );
    crate::cmd::fmt_revisions::resolve_para(&mut new_styled_p.props, false);
    assert_eq!(new_styled_p.props.numbering, None, "Reject restores None so paragraph inherits style numbering again");
    assert!(new_styled_p.props.num_change.is_none(), "num_change cleared on reject");
}
