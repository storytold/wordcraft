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

/// Helper that creates a session with a body paragraph and an attached footnote or endnote.
fn session_with_note(body: &str, note_text: &str, endnote: bool) -> Session {
    let mut s = Session::new(Document::from_text(body));
    s.run("select.all", &json!({})).unwrap();
    s.run("caret.end", &json!({})).unwrap();
    let cmd = if endnote { "references.endnote" } else { "references.footnote" };
    s.run(cmd, &json!({"text": note_text})).unwrap();
    s
}

/// #5842: Compare diffs footnote content between the original and revised documents,
/// tracking word deletions and insertions inside the footnote story without marking
/// the unchanged body reference as replaced.
#[test]
fn compare_diffs_footnote_text_changes() {
    let orig = session_with_note("Body paragraph.", "original note text", false);
    let rev = session_with_note("Body paragraph.", "revised note text", false);
    let rev_path = std::env::temp_dir().join(format!("wc-rev-fn-{}.docx", std::process::id()));
    crate::io::save_path(&rev_path, &rev.doc).unwrap();

    let mut s = orig;
    let r = s.run("review.compare", &json!({"path": rev_path.to_string_lossy()})).unwrap();
    let _ = std::fs::remove_file(&rev_path);

    assert!(r["changes"].as_u64().unwrap() >= 1, "footnote changes count: {r}");
    // The body reference is unchanged.
    let p = s.doc.body[0].as_para().unwrap();
    assert!(p.runs.iter().all(|r| r.props.del.is_none() && r.props.ins.is_none()));

    // The footnote story holds the tracked changes.
    let fn_part = s.doc.parts.values().find(|p| p.kind == wordcraft_doc::PartKind::Footnote).unwrap();
    let fn_para = fn_part.blocks[0].as_para().unwrap();
    let deleted: Vec<String> = fn_para.run_ranges().filter(|(_, c)| c.del.is_some()).map(|(r, _)| fn_para.text[r].to_string()).collect();
    let inserted: Vec<String> = fn_para.run_ranges().filter(|(_, c)| c.ins.is_some()).map(|(r, _)| fn_para.text[r].to_string()).collect();
    assert!(deleted.iter().any(|s| s.contains("original")), "deleted text in note: {deleted:?}");
    assert!(inserted.iter().any(|s| s.contains("revised")), "inserted text in note: {inserted:?}");

    // Accepting all changes applies the revised footnote text.
    s.run("review.acceptAll", &json!({})).unwrap();
    let fn_part = s.doc.parts.values().find(|p| p.kind == wordcraft_doc::PartKind::Footnote).unwrap();
    assert!(fn_part.blocks[0].as_para().unwrap().plain_text().contains("revised note text"));

    // Rejecting all changes restores the original footnote text.
    let orig2 = session_with_note("Body paragraph.", "original note text", false);
    let rev2 = session_with_note("Body paragraph.", "revised note text", false);
    let rev_path2 = std::env::temp_dir().join(format!("wc-rev-fn2-{}.docx", std::process::id()));
    crate::io::save_path(&rev_path2, &rev2.doc).unwrap();
    let mut s2 = orig2;
    s2.run("review.compare", &json!({"path": rev_path2.to_string_lossy()})).unwrap();
    let _ = std::fs::remove_file(&rev_path2);
    s2.run("review.rejectAll", &json!({})).unwrap();
    let fn_part2 = s2.doc.parts.values().find(|p| p.kind == wordcraft_doc::PartKind::Footnote).unwrap();
    assert!(fn_part2.blocks[0].as_para().unwrap().plain_text().contains("original note text"));
}

/// #5842: Compare records paragraph formatting changes within footnotes as `w:pPrChange`.
#[test]
fn compare_diffs_footnote_formatting_changes() {
    use wordcraft_doc::props::Align;
    let orig = session_with_note("Body paragraph.", "formatted note text", false);
    let mut rev = session_with_note("Body paragraph.", "formatted note text", false);
    // Change paragraph alignment in the revised footnote.
    let fn_id = rev.doc.parts.iter().find(|(_, p)| p.kind == wordcraft_doc::PartKind::Footnote).map(|(k, _)| *k).unwrap();
    let p = rev.doc.para_mut(StoryRef::Part(fn_id), &wordcraft_doc::Path::top(0)).unwrap();
    p.props.align = Some(Align::Center);
    let rev_path = std::env::temp_dir().join(format!("wc-rev-fnfmt-{}.docx", std::process::id()));
    crate::io::save_path(&rev_path, &rev.doc).unwrap();

    let mut s = orig;
    let r = s.run("review.compare", &json!({"path": rev_path.to_string_lossy()})).unwrap();
    let _ = std::fs::remove_file(&rev_path);

    assert_eq!(r["changes"], 1, "formatting change counted: {r}");
    let fn_part = s.doc.parts.get(&fn_id).unwrap();
    let fn_para = fn_part.blocks[0].as_para().unwrap();
    assert!(fn_para.props.fmt_change.is_some(), "footnote paragraph has pPrChange");
    assert_eq!(fn_para.props.align, Some(Align::Center));

    // Rejecting restores original paragraph alignment.
    s.run("review.rejectAll", &json!({})).unwrap();
    let fn_part = s.doc.parts.get(&fn_id).unwrap();
    let fn_para = fn_part.blocks[0].as_para().unwrap();
    assert!(fn_para.props.fmt_change.is_none());
    assert_eq!(fn_para.props.align, None);
}

/// #5842: Compare diffs endnote stories identically to footnotes, tracking edits in endnotes.
#[test]
fn compare_diffs_endnotes() {
    let orig = session_with_note("Body paragraph.", "original endnote", true);
    let rev = session_with_note("Body paragraph.", "revised endnote", true);
    let rev_path = std::env::temp_dir().join(format!("wc-rev-en-{}.docx", std::process::id()));
    crate::io::save_path(&rev_path, &rev.doc).unwrap();

    let mut s = orig;
    let r = s.run("review.compare", &json!({"path": rev_path.to_string_lossy()})).unwrap();
    let _ = std::fs::remove_file(&rev_path);

    assert!(r["changes"].as_u64().unwrap() >= 1, "endnote changes counted: {r}");
    let en_part = s.doc.parts.values().find(|p| p.kind == wordcraft_doc::PartKind::Endnote).unwrap();
    let en_para = en_part.blocks[0].as_para().unwrap();
    assert!(en_para.runs.iter().any(|r| r.props.del.is_some() || r.props.ins.is_some()));

    s.run("review.acceptAll", &json!({})).unwrap();
    let en_part = s.doc.parts.values().find(|p| p.kind == wordcraft_doc::PartKind::Endnote).unwrap();
    assert!(en_part.blocks[0].as_para().unwrap().plain_text().contains("revised endnote"));
}

/// #5842: When `footnotes` is explicitly false, footnote diffing is skipped and footnotes are
/// carried over verbatim from the original document, and `footnotes` is not in `notHonoured`.
#[test]
fn compare_honours_footnotes_option_disabled() {
    let orig = session_with_note("Body paragraph.", "original note", false);
    let rev = session_with_note("Body paragraph.", "revised note", false);
    let rev_path = std::env::temp_dir().join(format!("wc-rev-fndis-{}.docx", std::process::id()));
    crate::io::save_path(&rev_path, &rev.doc).unwrap();

    let mut s = orig;
    let r = s.run("review.compare", &json!({"path": rev_path.to_string_lossy(), "footnotes": false})).unwrap();
    let _ = std::fs::remove_file(&rev_path);

    assert_eq!(r["changes"], 0, "no changes when footnotes option disabled");
    let not_honoured = r["notHonoured"].as_array().unwrap();
    assert!(!not_honoured.iter().any(|v| v == "footnotes"), "footnotes option is honoured: {not_honoured:?}");

    let fn_part = s.doc.parts.values().find(|p| p.kind == wordcraft_doc::PartKind::Footnote).unwrap();
    let fn_para = fn_part.blocks[0].as_para().unwrap();
    assert!(fn_para.plain_text().contains("original note"));
    assert!(fn_para.runs.iter().all(|r| r.props.del.is_none() && r.props.ins.is_none()));
}

/// #5842: Compare correctly handles inserted and deleted footnotes across documents,
/// tracking the deletion of the body reference and adopting newly added notes.
#[test]
fn compare_handles_inserted_and_deleted_footnotes() {
    // Original has 2 footnotes.
    let mut orig = Session::new(Document::from_text("Para one.\nPara two."));
    orig.run("caret.set", &json!({"pos": Pos::body(0, "Para one.".len())})).unwrap();
    orig.run("references.footnote", &json!({"text": "First note."})).unwrap();
    orig.run("caret.set", &json!({"pos": Pos::body(1, "Para two.".len())})).unwrap();
    orig.run("references.footnote", &json!({"text": "Second note."})).unwrap();

    // Revised deletes the first footnote, keeps the second, and inserts a third.
    let mut rev = Session::new(Document::from_text("Para one.\nPara two.\nPara three."));
    rev.run("caret.set", &json!({"pos": Pos::body(1, "Para two.".len())})).unwrap();
    rev.run("references.footnote", &json!({"text": "Second note."})).unwrap();
    rev.run("caret.set", &json!({"pos": Pos::body(2, "Para three.".len())})).unwrap();
    rev.run("references.footnote", &json!({"text": "Third note."})).unwrap();

    let fn1_id = orig.doc.body[0]
        .as_para()
        .unwrap()
        .objects
        .iter()
        .find_map(|o| if let InlineObject::NoteRef { id, .. } = o { Some(*id) } else { None })
        .unwrap();
    let fn2_id = orig.doc.body[1]
        .as_para()
        .unwrap()
        .objects
        .iter()
        .find_map(|o| if let InlineObject::NoteRef { id, .. } = o { Some(*id) } else { None })
        .unwrap();

    let rev_path = std::env::temp_dir().join(format!("wc-rev-fndel-{}.docx", std::process::id()));
    crate::io::save_path(&rev_path, &rev.doc).unwrap();

    let mut s = orig;
    let r = s.run("review.compare", &json!({"path": rev_path.to_string_lossy()})).unwrap();
    let _ = std::fs::remove_file(&rev_path);

    assert!(r["changes"].as_u64().unwrap() >= 1);
    // Para one has deleted footnote reference.
    let p1 = s.doc.body[0].as_para().unwrap();
    let deleted_ref = p1.runs.iter().any(|r| r.props.del.is_some());
    assert!(deleted_ref, "deleted footnote reference mark is tracked as del");

    // Para two has unchanged footnote reference.
    let p2 = s.doc.body[1].as_para().unwrap();
    let unchanged_ref = p2.runs.iter().all(|r| r.props.del.is_none() && r.props.ins.is_none());
    assert!(unchanged_ref, "unchanged footnote reference is retained");

    // Deleted footnote part content is marked with del.
    let fn1_part = s.doc.parts.get(&fn1_id).unwrap();
    assert!(fn1_part.blocks[0].as_para().unwrap().runs.iter().all(|r| r.props.del.is_some()), "deleted note content marked with del");

    // Unchanged footnote part content has no del or ins.
    let fn2_part = s.doc.parts.get(&fn2_id).unwrap();
    assert!(
        fn2_part.blocks[0].as_para().unwrap().runs.iter().all(|r| r.props.del.is_none() && r.props.ins.is_none()),
        "unchanged note content has no tracked marks"
    );

    // Para three has inserted footnote reference, and inserted footnote part content is marked with ins.
    let p3 = s.doc.body.iter().filter_map(|b| b.as_para()).find(|p| p.plain_text().contains("Para three")).unwrap();
    let fn3_id = p3.objects.iter().find_map(|o| if let InlineObject::NoteRef { id, .. } = o { Some(*id) } else { None }).unwrap();
    let fn3_part = s.doc.parts.get(&fn3_id).unwrap();
    assert!(fn3_part.blocks[0].as_para().unwrap().runs.iter().all(|r| r.props.ins.is_some()), "inserted note content marked with ins");
}

/// #5842: When original has two footnotes (note 1 and note 2), and revised deletes note 1
/// and lightly edits note 2, note 2 must be paired with note 2 (not note 1).
/// Paragraph 2 retains its unchanged footnote reference, and note 2 content diffs cleanly.
#[test]
fn compare_pairs_notes_across_deletion_and_edit() {
    let mut orig = Session::new(Document::from_text("Para one.\nPara two."));
    orig.run("caret.set", &json!({"pos": Pos::body(0, "Para one.".len())})).unwrap();
    orig.run("references.footnote", &json!({"text": "First footnote content."})).unwrap();
    orig.run("caret.set", &json!({"pos": Pos::body(1, "Para two.".len())})).unwrap();
    orig.run("references.footnote", &json!({"text": "Second footnote content."})).unwrap();

    // Revised deletes footnote 1 from Para one and lightly edits footnote 2 in Para two.
    let mut rev = Session::new(Document::from_text("Para one.\nPara two."));
    rev.run("caret.set", &json!({"pos": Pos::body(1, "Para two.".len())})).unwrap();
    rev.run("references.footnote", &json!({"text": "Second footnote content revised."})).unwrap();

    let rev_path = std::env::temp_dir().join(format!("wc-rev-fnpair-{}.docx", std::process::id()));
    crate::io::save_path(&rev_path, &rev.doc).unwrap();

    let mut s = orig;
    let r = s.run("review.compare", &json!({"path": rev_path.to_string_lossy()})).unwrap();
    let _ = std::fs::remove_file(&rev_path);

    assert!(r["changes"].as_u64().unwrap() >= 1);

    // Para one: footnote reference was deleted.
    let p1 = s.doc.body[0].as_para().unwrap();
    let p1_has_del_ref = p1.runs.iter().any(|r| r.props.del.is_some());
    assert!(p1_has_del_ref, "Para one footnote reference is tracked as deleted");

    // Para two: footnote reference to Note 2 must be retained without deletion/insertion markup!
    let p2 = s.doc.body[1].as_para().unwrap();
    let p2_ref_clean = p2.runs.iter().all(|r| r.props.del.is_none() && r.props.ins.is_none());
    assert!(p2_ref_clean, "Para two retains footnote reference cleanly without del/ins markup");

    // Note 2 part: must contain the edit ("revised" inserted), rather than note 1 being rewritten.
    let fn2_id = p2.objects.iter().find_map(|o| if let InlineObject::NoteRef { id, .. } = o { Some(*id) } else { None }).unwrap();
    let fn2_part = s.doc.parts.get(&fn2_id).unwrap();
    let fn2_text = fn2_part.blocks[0].as_para().unwrap().plain_text();
    assert!(fn2_text.contains("Second footnote content"), "Note 2 content retained: {fn2_text}");
    let has_ins = fn2_part.blocks[0].as_para().unwrap().runs.iter().any(|r| r.props.ins.is_some());
    assert!(has_ins, "Note 2 tracks insertion of revised text");
}

/// #5842: DOCX roundtrip preserves footnote tracked revisions (`w:ins`, `w:del`) produced by compare.
#[test]
fn compare_footnote_docx_roundtrip() {
    let orig = session_with_note("Body sentence.", "alpha note content", false);
    let rev = session_with_note("Body sentence.", "beta note content", false);
    let orig_path = std::env::temp_dir().join(format!("wc-orig-rt-{}.docx", std::process::id()));
    let rev_path = std::env::temp_dir().join(format!("wc-rev-rt-{}.docx", std::process::id()));
    let out_path = std::env::temp_dir().join(format!("wc-out-rt-{}.docx", std::process::id()));

    crate::io::save_path(&orig_path, &orig.doc).unwrap();
    crate::io::save_path(&rev_path, &rev.doc).unwrap();

    let mut s = Session::new(Document::new());
    s.run(
        "review.compare",
        &json!({
            "original": orig_path.to_string_lossy(),
            "path": rev_path.to_string_lossy()
        }),
    )
    .unwrap();

    crate::io::save_path(&out_path, &s.doc).unwrap();
    let loaded = crate::io::open_path(&out_path).unwrap();

    let _ = std::fs::remove_file(&orig_path);
    let _ = std::fs::remove_file(&rev_path);
    let _ = std::fs::remove_file(&out_path);

    let fn_part = loaded.parts.values().find(|p| p.kind == wordcraft_doc::PartKind::Footnote).unwrap();
    let fn_para = fn_part.blocks[0].as_para().unwrap();
    assert!(fn_para.runs.iter().any(|r| r.props.del.is_some()), "del preserved in docx footnotes.xml");
    assert!(fn_para.runs.iter().any(|r| r.props.ins.is_some()), "ins preserved in docx footnotes.xml");
}

/// #5842: Verifies that `lcs_len` returns genuine in-order common subsequences even when
/// the comparison cell budget is exhausted (`budget = 0`), without allocating DP tables.
#[test]
fn lcs_len_exhausted_budget_bounded_fallback() {
    let mut zero_budget = 0u64;

    // 1. Identical slices: matches completely via prefix/suffix.
    let identical_a = vec!["alpha", "beta", "gamma", "delta"];
    let identical_b = vec!["alpha", "beta", "gamma", "delta"];
    assert_eq!(super::lcs_len(&identical_a, &identical_b, &mut zero_budget), 4);
    assert_eq!(zero_budget, 0);

    // 2. Single word substitution in middle: prefix and suffix match surrounding tokens.
    let mid_a = vec!["start", "word_a", "middle", "end"];
    let mid_b = vec!["start", "word_b", "middle", "end"];
    assert_eq!(super::lcs_len(&mid_a, &mid_b, &mut zero_budget), 3);

    // 3. Edits at both ends: greedy scan matches unchanged core tokens in order.
    let core_a = vec!["head1", "core_x", "core_y", "tail1"];
    let core_b = vec!["head2", "core_x", "core_y", "tail2"];
    assert_eq!(super::lcs_len(&core_a, &core_b, &mut zero_budget), 2);

    // 4. Unrelated slices: returns 0 matches without false positives.
    let unrelated_a = vec!["apple", "banana", "cherry"];
    let unrelated_b = vec!["dog", "elephant", "fox"];
    assert_eq!(super::lcs_len(&unrelated_a, &unrelated_b, &mut zero_budget), 0);

    // 5. Normal budget debits only core DP cells, not whole sequence length.
    let mut ample_budget = 100u64;
    // Prefix 1 ("p"), suffix 1 ("s"), core 1x1 ("a" vs "b"). Cells = 1.
    let budgeted_a = vec!["p", "a", "s"];
    let budgeted_b = vec!["p", "b", "s"];
    assert_eq!(super::lcs_len(&budgeted_a, &budgeted_b, &mut ample_budget), 2);
    assert_eq!(ample_budget, 99);
}

/// #5842: CodeRabbit Major finding scenario: when a document has many footnotes (30+)
/// and both reference paragraphs and note contents are edited, note similarity scoring
/// falls back to bounded greedy alignment after `MAX_NOTE_ALIGN_BUDGET` is exhausted,
/// correctly pairing all 30 corresponding notes rather than treating them as whole deletions and insertions.
#[test]
fn compare_pairs_many_notes_when_budget_exhausted() {
    let note_count = 30usize;
    let mut orig_text = String::new();
    let mut rev_text = String::new();

    for i in 0..note_count {
        if i > 0 {
            orig_text.push('\n');
            rev_text.push('\n');
        }
        orig_text.push_str(&format!("Body paragraph {i} discussing original thesis topic."));
        rev_text.push_str(&format!("Body paragraph {i} discussing revised thesis subject."));
    }

    let mut orig = Session::new(Document::from_text(&orig_text));
    let mut rev = Session::new(Document::from_text(&rev_text));

    for i in 0..note_count {
        let orig_para_len = orig.doc.body[i].as_para().unwrap().text.len();
        orig.run("caret.set", &json!({"pos": Pos::body(i, orig_para_len)})).unwrap();
        orig.run("references.footnote", &json!({"text": format!("Footnote entry {i} citing detailed historical observations and background data.")}))
            .unwrap();

        let rev_para_len = rev.doc.body[i].as_para().unwrap().text.len();
        rev.run("caret.set", &json!({"pos": Pos::body(i, rev_para_len)})).unwrap();
        rev.run("references.footnote", &json!({"text": format!("Footnote entry {i} citing detailed historical commentary and background data.")}))
            .unwrap();
    }

    let rev_path = std::env::temp_dir().join(format!("wc-rev-manyfn-{}.docx", std::process::id()));
    crate::io::save_path(&rev_path, &rev.doc).unwrap();

    let mut s = orig;
    let r = s.run("review.compare", &json!({"path": rev_path.to_string_lossy()})).unwrap();
    let _ = std::fs::remove_file(&rev_path);

    assert!(r["changes"].as_u64().unwrap() >= note_count as u64);

    // Ensure all 30 footnotes survived as paired footnotes in the result document.
    let footnotes: Vec<_> = s.doc.parts.values().filter(|p| p.kind == wordcraft_doc::PartKind::Footnote).collect();
    assert_eq!(footnotes.len(), note_count, "All 30 footnotes must be paired rather than deleted and re-inserted");

    // Verify every note paired with its counterpart and tracked the internal word substitution.
    for part in &footnotes {
        let para = part.blocks[0].as_para().unwrap();
        let has_ins = para.run_ranges().any(|(r, c)| c.ins.is_some() && para.text[r].contains("commentary"));
        let has_del = para.run_ranges().any(|(r, c)| c.del.is_some() && para.text[r].contains("observations"));
        assert!(has_ins, "Note tracks insertion of 'commentary': {}", para.plain_text());
        assert!(has_del, "Note tracks deletion of 'observations': {}", para.plain_text());
    }
}
