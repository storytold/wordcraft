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

/// RAII guard for unique temporary `.docx` files created during comparison tests.
/// Deletes the temporary file on drop even if an assertion or unwrap panics.
struct TempDocx(std::path::PathBuf);

impl TempDocx {
    fn new(prefix: &str) -> Self {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("wc-{prefix}-{}-{n}.docx", std::process::id()));
        Self(path)
    }

    fn save(&self, doc: &Document) -> &Self {
        crate::io::save_path(&self.0, doc).expect("failed to save temp docx");
        self
    }

    fn path_str(&self) -> String {
        self.0.to_string_lossy().into_owned()
    }
}

impl Drop for TempDocx {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// #5837: Compare diffs header stories, recording insertions, deletions, and formatting changes
/// rather than copying the original header verbatim.
#[test]
fn compare_tracks_header_changes() {
    let mut orig = Session::new(Document::from_text("Body text in original."));
    let mut rev = Session::new(Document::from_text("Body text in original."));
    orig.run("insert.header", &json!({"text": "Original Corporation Confidential Header"})).unwrap();
    rev.run("insert.header", &json!({"text": "Revised Corporation Public Header"})).unwrap();

    let rev_file = TempDocx::new("rev-hdr");
    rev_file.save(&rev.doc);

    let mut s = orig;
    let r = s.run("review.compare", &json!({"path": rev_file.path_str()})).unwrap();

    assert_eq!(r["changes"], 1, "Expected exactly 1 change in header, got: {r}");
    let not_honoured = r["notHonoured"].as_array().unwrap();
    assert!(!not_honoured.contains(&json!("headersFooters")), "headersFooters should be honoured");

    let hdr_id = s.doc.last_section.headers.default.unwrap();
    let part = s.doc.parts.get(&hdr_id).unwrap();
    let para = part.blocks[0].as_para().unwrap();
    assert!(para.run_ranges().any(|(_, c)| c.del.is_some()), "Header must have deleted run");
    assert!(para.run_ranges().any(|(_, c)| c.ins.is_some()), "Header must have inserted run");

    s.run("review.acceptAll", &json!({})).unwrap();
    assert_eq!(s.doc.plain_text(StoryRef::Part(hdr_id)), "Revised Corporation Public Header");
}

/// #5837: Compare diffs footer stories, tracking deletions and insertions.
#[test]
fn compare_tracks_footer_changes() {
    let mut orig = Session::new(Document::from_text("Body paragraph."));
    let mut rev = Session::new(Document::from_text("Body paragraph."));
    orig.run("insert.footer", &json!({"text": "Confidential - Page 1"})).unwrap();
    rev.run("insert.footer", &json!({"text": "Public - Page 1"})).unwrap();

    let rev_file = TempDocx::new("rev-ftr");
    rev_file.save(&rev.doc);

    let mut s = orig;
    let r = s.run("review.compare", &json!({"path": rev_file.path_str()})).unwrap();

    assert_eq!(r["changes"], 1, "Expected exactly 1 change in footer, got: {r}");
    let ftr_id = s.doc.last_section.footers.default.unwrap();
    let part = s.doc.parts.get(&ftr_id).unwrap();
    let para = part.blocks[0].as_para().unwrap();
    assert!(para.run_ranges().any(|(_, c)| c.del.is_some()), "Footer must track deletion");
    assert!(para.run_ranges().any(|(_, c)| c.ins.is_some()), "Footer must track insertion");

    s.run("review.acceptAll", &json!({})).unwrap();
    assert_eq!(s.doc.plain_text(StoryRef::Part(ftr_id)), "Public - Page 1");
}

/// #5837: When "headersFooters": false is passed, header and footer differences are ignored.
#[test]
fn compare_honours_headers_footers_false() {
    let mut orig = Session::new(Document::from_text("Body paragraph."));
    let mut rev = Session::new(Document::from_text("Body paragraph."));
    orig.run("insert.header", &json!({"text": "Original Header Text"})).unwrap();
    rev.run("insert.header", &json!({"text": "Revised Header Text"})).unwrap();

    let rev_file = TempDocx::new("rev-hf-opt");
    rev_file.save(&rev.doc);

    let mut s = orig;
    let r = s.run("review.compare", &json!({"path": rev_file.path_str(), "headersFooters": false})).unwrap();

    assert_eq!(r["changes"], 0, "No changes should be reported when headersFooters is false");
    let hdr_id = s.doc.last_section.headers.default.unwrap();
    assert_eq!(s.doc.plain_text(StoryRef::Part(hdr_id)), "Original Header Text");
}

/// #5837: Explicit "headersFooters": true compares headers and footers identically to the default.
#[test]
fn compare_honours_headers_footers_true_explicit() {
    let mut orig = Session::new(Document::from_text("Body paragraph."));
    let mut rev = Session::new(Document::from_text("Body paragraph."));
    orig.run("insert.header", &json!({"text": "Original Header Text"})).unwrap();
    rev.run("insert.header", &json!({"text": "Revised Header Text"})).unwrap();

    let rev_file = TempDocx::new("rev-hf-opt-true");
    rev_file.save(&rev.doc);

    let mut s = orig;
    let r = s.run("review.compare", &json!({"path": rev_file.path_str(), "headersFooters": true})).unwrap();

    assert_eq!(r["changes"], 1, "Exactly 1 change expected when headersFooters is explicitly true");
    let hdr_id = s.doc.last_section.headers.default.unwrap();
    s.run("review.acceptAll", &json!({})).unwrap();
    assert_eq!(s.doc.plain_text(StoryRef::Part(hdr_id)), "Revised Header Text");
}

/// #5837: Formatting differences in headers produce tracked formatting changes (rPrChange / pPrChange).
#[test]
fn compare_tracks_header_formatting_changes() {
    let mut orig = Session::new(Document::from_text("Body text."));
    let mut rev = Session::new(Document::from_text("Body text."));
    orig.run("insert.header", &json!({"text": "Annual Report 2026"})).unwrap();
    rev.run("insert.header", &json!({"text": "Annual Report 2026"})).unwrap();

    let hdr_id = rev.doc.last_section.headers.default.unwrap();
    rev.doc.format_range(&rev.doc.start_of(StoryRef::Part(hdr_id)), &rev.doc.end_of(StoryRef::Part(hdr_id)), &|c| c.bold = Some(true)).unwrap();

    let rev_file = TempDocx::new("rev-hdr-fmt");
    rev_file.save(&rev.doc);

    let mut s = orig;
    let r = s.run("review.compare", &json!({"path": rev_file.path_str()})).unwrap();

    assert_eq!(r["changes"], 1, "Header formatting change must be counted");
    assert!(s.doc.revisions.iter().any(|x| x.kind == RevisionKind::Format), "Must produce Format revision");
}

/// #5837: When formatting=false is passed, formatting differences in headers do not record Format revisions.
#[test]
fn compare_tracks_header_formatting_disabled() {
    let mut orig = Session::new(Document::from_text("Body text."));
    let mut rev = Session::new(Document::from_text("Body text."));
    orig.run("insert.header", &json!({"text": "Annual Report 2026"})).unwrap();
    rev.run("insert.header", &json!({"text": "Annual Report 2026"})).unwrap();

    let hdr_id = rev.doc.last_section.headers.default.unwrap();
    rev.doc.format_range(&rev.doc.start_of(StoryRef::Part(hdr_id)), &rev.doc.end_of(StoryRef::Part(hdr_id)), &|c| c.bold = Some(true)).unwrap();

    let rev_file = TempDocx::new("rev-hdr-no-fmt");
    rev_file.save(&rev.doc);

    let mut s = orig;
    let r = s.run("review.compare", &json!({"path": rev_file.path_str(), "formatting": false})).unwrap();

    assert_eq!(r["changes"], 0, "No formatting changes should be reported when formatting is false");
    assert!(!s.doc.revisions.iter().any(|x| x.kind == RevisionKind::Format), "No Format revision produced");
}

/// #5837: Headers comparison respects level="character", generating character-level diffs in headers.
#[test]
fn compare_tracks_header_changes_at_character_level() {
    let mut orig = Session::new(Document::from_text("Body text."));
    let mut rev = Session::new(Document::from_text("Body text."));
    orig.run("insert.header", &json!({"text": "Color Selection"})).unwrap();
    rev.run("insert.header", &json!({"text": "Colour Selection"})).unwrap();

    let rev_file = TempDocx::new("rev-hdr-char");
    rev_file.save(&rev.doc);

    let mut s = orig;
    let r = s.run("review.compare", &json!({"path": rev_file.path_str(), "level": "character"})).unwrap();

    assert_eq!(r["changes"], 1);
    let hdr_id = s.doc.last_section.headers.default.unwrap();
    let part = s.doc.parts.get(&hdr_id).unwrap();
    let para = part.blocks[0].as_para().unwrap();
    let runs: Vec<_> = para.run_ranges().collect();
    // Character level must isolate the inserted 'u':
    assert!(runs.iter().any(|(range, c)| c.ins.is_some() && &para.text[range.start..range.end] == "u"));

    s.run("review.acceptAll", &json!({})).unwrap();
    assert_eq!(s.doc.plain_text(StoryRef::Part(hdr_id)), "Colour Selection");
}

/// #5837: Compare tracks differences in both body and header stories, correctly summing total changes.
#[test]
fn compare_tracks_both_body_and_header_changes_summed() {
    let mut orig = Session::new(Document::from_text("Original body paragraph."));
    let mut rev = Session::new(Document::from_text("Revised body paragraph."));
    orig.run("insert.header", &json!({"text": "Original Header Text"})).unwrap();
    rev.run("insert.header", &json!({"text": "Revised Header Text"})).unwrap();

    let rev_file = TempDocx::new("rev-body-hdr");
    rev_file.save(&rev.doc);

    let mut s = orig;
    let r = s.run("review.compare", &json!({"path": rev_file.path_str()})).unwrap();

    // 1 body block change + 1 header block change = 2 total changes
    assert_eq!(r["changes"], 2, "Expected exactly 2 changes (1 body + 1 header), got: {r}");

    let body_para = s.doc.body[0].as_para().unwrap();
    assert!(body_para.run_ranges().any(|(_, c)| c.del.is_some()), "Body has deletion");
    assert!(body_para.run_ranges().any(|(_, c)| c.ins.is_some()), "Body has insertion");

    let hdr_id = s.doc.last_section.headers.default.unwrap();
    let hdr_para = s.doc.parts.get(&hdr_id).unwrap().blocks[0].as_para().unwrap();
    assert!(hdr_para.run_ranges().any(|(_, c)| c.del.is_some()), "Header has deletion");
    assert!(hdr_para.run_ranges().any(|(_, c)| c.ins.is_some()), "Header has insertion");

    s.run("review.acceptAll", &json!({})).unwrap();
    assert_eq!(s.doc.plain_text(StoryRef::Body), "Revised body paragraph.");
    assert_eq!(s.doc.plain_text(StoryRef::Part(hdr_id)), "Revised Header Text");
}

/// #5837: Headers on first page (Different First Page) are compared and tracked.
#[test]
fn compare_tracks_first_page_header_changes() {
    let mut orig = Session::new(Document::from_text("Page 1 body.\n\nPage 2 body."));
    let mut rev = Session::new(Document::from_text("Page 1 body.\n\nPage 2 body."));

    let orig_first = orig.doc.add_part(
        wordcraft_doc::PartKind::Header,
        vec![para_block(Paragraph::with_text("Original First Page Header", CharProps::default()).styled("Header"))],
    );
    orig.doc.last_section.title_page = true;
    orig.doc.last_section.headers.first = Some(orig_first);

    let rev_first = rev.doc.add_part(
        wordcraft_doc::PartKind::Header,
        vec![para_block(Paragraph::with_text("Revised First Page Header", CharProps::default()).styled("Header"))],
    );
    rev.doc.last_section.title_page = true;
    rev.doc.last_section.headers.first = Some(rev_first);

    let rev_file = TempDocx::new("rev-first-hdr");
    rev_file.save(&rev.doc);

    let mut s = orig;
    let r = s.run("review.compare", &json!({"path": rev_file.path_str()})).unwrap();

    assert_eq!(r["changes"], 1);
    let part = s.doc.parts.get(&orig_first).unwrap();
    let para = part.blocks[0].as_para().unwrap();
    assert!(para.run_ranges().any(|(_, c)| c.del.is_some()));
    assert!(para.run_ranges().any(|(_, c)| c.ins.is_some()));

    s.run("review.acceptAll", &json!({})).unwrap();
    assert_eq!(s.doc.plain_text(StoryRef::Part(orig_first)), "Revised First Page Header");
}

/// #5837: Footers on first page and even pages are compared and tracked.
#[test]
fn compare_tracks_first_page_and_even_footer_changes() {
    let mut orig = Session::new(Document::from_text("Page 1 body.\n\nPage 2 body."));
    let mut rev = Session::new(Document::from_text("Page 1 body.\n\nPage 2 body."));

    let orig_first = orig.doc.add_part(
        wordcraft_doc::PartKind::Footer,
        vec![para_block(Paragraph::with_text("Original First Page Footer", CharProps::default()).styled("Footer"))],
    );
    let orig_even = orig.doc.add_part(
        wordcraft_doc::PartKind::Footer,
        vec![para_block(Paragraph::with_text("Original Even Page Footer", CharProps::default()).styled("Footer"))],
    );
    orig.doc.last_section.title_page = true;
    orig.doc.settings.even_odd_headers = true;
    orig.doc.last_section.footers.first = Some(orig_first);
    orig.doc.last_section.footers.even = Some(orig_even);

    let rev_first = rev.doc.add_part(
        wordcraft_doc::PartKind::Footer,
        vec![para_block(Paragraph::with_text("Revised First Page Footer", CharProps::default()).styled("Footer"))],
    );
    let rev_even = rev.doc.add_part(
        wordcraft_doc::PartKind::Footer,
        vec![para_block(Paragraph::with_text("Revised Even Page Footer", CharProps::default()).styled("Footer"))],
    );
    rev.doc.last_section.title_page = true;
    rev.doc.settings.even_odd_headers = true;
    rev.doc.last_section.footers.first = Some(rev_first);
    rev.doc.last_section.footers.even = Some(rev_even);

    let rev_file = TempDocx::new("rev-first-even-ftr");
    rev_file.save(&rev.doc);

    let mut s = orig;
    let r = s.run("review.compare", &json!({"path": rev_file.path_str()})).unwrap();

    assert_eq!(r["changes"], 2, "Expected 2 changes for First and Even footers");

    let p_first = s.doc.parts.get(&orig_first).unwrap().blocks[0].as_para().unwrap();
    assert!(p_first.run_ranges().any(|(_, c)| c.del.is_some()));
    assert!(p_first.run_ranges().any(|(_, c)| c.ins.is_some()));

    let p_even = s.doc.parts.get(&orig_even).unwrap().blocks[0].as_para().unwrap();
    assert!(p_even.run_ranges().any(|(_, c)| c.del.is_some()));
    assert!(p_even.run_ranges().any(|(_, c)| c.ins.is_some()));

    s.run("review.acceptAll", &json!({})).unwrap();
    assert_eq!(s.doc.plain_text(StoryRef::Part(orig_first)), "Revised First Page Footer");
    assert_eq!(s.doc.plain_text(StoryRef::Part(orig_even)), "Revised Even Page Footer");
}

/// #5837: When revised adds a header or original header is removed, revisions are generated,
/// and both acceptAll and rejectAll produce the exact expected content and section references.
#[test]
fn compare_tracks_added_and_deleted_headers() {
    // 1. Added header: original has none, revised adds one.
    let orig = Session::new(Document::from_text("Body without header."));
    let mut rev = Session::new(Document::from_text("Body without header."));
    rev.run("insert.header", &json!({"text": "Newly Added Header"})).unwrap();

    let rev_file = TempDocx::new("rev-add-hdr");
    rev_file.save(&rev.doc);

    // Verify accept:
    {
        let mut s = Session::new(orig.doc.clone());
        let r = s.run("review.compare", &json!({"path": rev_file.path_str()})).unwrap();
        assert_eq!(r["changes"], 1);
        let new_hdr_id = s.doc.last_section.headers.default.expect("Added header must be assigned");
        let part = s.doc.parts.get(&new_hdr_id).unwrap();
        let para = part.blocks[0].as_para().unwrap();
        assert!(para.run_ranges().any(|(_, c)| c.ins.is_some()), "Added header must be marked inserted");

        s.run("review.acceptAll", &json!({})).unwrap();
        assert_eq!(s.doc.plain_text(StoryRef::Part(new_hdr_id)), "Newly Added Header");
    }

    // Verify reject:
    {
        let mut s = Session::new(orig.doc.clone());
        let r = s.run("review.compare", &json!({"path": rev_file.path_str()})).unwrap();
        assert_eq!(r["changes"], 1);
        let new_hdr_id = s.doc.last_section.headers.default.expect("Added header must be assigned");
        s.run("review.rejectAll", &json!({})).unwrap();
        let text = s.doc.plain_text(StoryRef::Part(new_hdr_id));
        assert!(text.is_empty(), "Rejecting added header leaves story text empty, got: {text}");
    }

    // 2. Deleted header: original has header, revised has none.
    let mut orig_with_hdr = Session::new(Document::from_text("Body text."));
    orig_with_hdr.run("insert.header", &json!({"text": "Header to be removed"})).unwrap();
    let rev_without_hdr = Session::new(Document::from_text("Body text."));

    let rev_file2 = TempDocx::new("rev-del-hdr");
    rev_file2.save(&rev_without_hdr.doc);

    // Verify accept of deletion:
    {
        let mut s2 = Session::new(orig_with_hdr.doc.clone());
        let r2 = s2.run("review.compare", &json!({"path": rev_file2.path_str()})).unwrap();
        assert_eq!(r2["changes"], 1);
        let orig_hdr_id = s2.doc.last_section.headers.default.unwrap();
        let part2 = s2.doc.parts.get(&orig_hdr_id).unwrap();
        let para2 = part2.blocks[0].as_para().unwrap();
        assert!(para2.run_ranges().any(|(_, c)| c.del.is_some()), "Deleted header must be marked deleted");

        s2.run("review.acceptAll", &json!({})).unwrap();
        let text = s2.doc.plain_text(StoryRef::Part(orig_hdr_id));
        assert!(text.is_empty(), "Accepted deleted header must leave story text empty, got: {text}");
    }

    // Verify reject of deletion:
    {
        let mut s2 = Session::new(orig_with_hdr.doc.clone());
        let r2 = s2.run("review.compare", &json!({"path": rev_file2.path_str()})).unwrap();
        assert_eq!(r2["changes"], 1);
        let orig_hdr_id = s2.doc.last_section.headers.default.unwrap();
        s2.run("review.rejectAll", &json!({})).unwrap();
        assert_eq!(s2.doc.plain_text(StoryRef::Part(orig_hdr_id)), "Header to be removed");
    }
}

/// #5837: When revised adds a footer or original footer is removed, revisions are generated,
/// and both acceptAll and rejectAll produce the exact expected content and section references.
#[test]
fn compare_tracks_added_and_deleted_footers() {
    // 1. Added footer: original has none, revised adds one.
    let orig = Session::new(Document::from_text("Body without footer."));
    let mut rev = Session::new(Document::from_text("Body without footer."));
    rev.run("insert.footer", &json!({"text": "Newly Added Footer"})).unwrap();

    let rev_file = TempDocx::new("rev-add-ftr");
    rev_file.save(&rev.doc);

    {
        let mut s = Session::new(orig.doc.clone());
        let r = s.run("review.compare", &json!({"path": rev_file.path_str()})).unwrap();
        assert_eq!(r["changes"], 1);
        let new_ftr_id = s.doc.last_section.footers.default.expect("Added footer must be assigned");
        let part = s.doc.parts.get(&new_ftr_id).unwrap();
        let para = part.blocks[0].as_para().unwrap();
        assert!(para.run_ranges().any(|(_, c)| c.ins.is_some()), "Added footer must be marked inserted");

        s.run("review.acceptAll", &json!({})).unwrap();
        assert_eq!(s.doc.plain_text(StoryRef::Part(new_ftr_id)), "Newly Added Footer");
    }

    {
        let mut s = Session::new(orig.doc.clone());
        let r = s.run("review.compare", &json!({"path": rev_file.path_str()})).unwrap();
        assert_eq!(r["changes"], 1);
        let new_ftr_id = s.doc.last_section.footers.default.expect("Added footer must be assigned");
        s.run("review.rejectAll", &json!({})).unwrap();
        let text = s.doc.plain_text(StoryRef::Part(new_ftr_id));
        assert!(text.is_empty(), "Rejecting added footer leaves story text empty, got: {text}");
    }

    // 2. Deleted footer: original has footer, revised has none.
    let mut orig_with_ftr = Session::new(Document::from_text("Body text."));
    orig_with_ftr.run("insert.footer", &json!({"text": "Footer to be removed"})).unwrap();
    let rev_without_ftr = Session::new(Document::from_text("Body text."));

    let rev_file2 = TempDocx::new("rev-del-ftr");
    rev_file2.save(&rev_without_ftr.doc);

    {
        let mut s2 = Session::new(orig_with_ftr.doc.clone());
        let r2 = s2.run("review.compare", &json!({"path": rev_file2.path_str()})).unwrap();
        assert_eq!(r2["changes"], 1);
        let orig_ftr_id = s2.doc.last_section.footers.default.unwrap();
        let part2 = s2.doc.parts.get(&orig_ftr_id).unwrap();
        let para2 = part2.blocks[0].as_para().unwrap();
        assert!(para2.run_ranges().any(|(_, c)| c.del.is_some()), "Deleted footer must be marked deleted");

        s2.run("review.acceptAll", &json!({})).unwrap();
        let text = s2.doc.plain_text(StoryRef::Part(orig_ftr_id));
        assert!(text.is_empty(), "Accepted deleted footer must leave story text empty, got: {text}");
    }

    {
        let mut s2 = Session::new(orig_with_ftr.doc.clone());
        let r2 = s2.run("review.compare", &json!({"path": rev_file2.path_str()})).unwrap();
        assert_eq!(r2["changes"], 1);
        let orig_ftr_id = s2.doc.last_section.footers.default.unwrap();
        s2.run("review.rejectAll", &json!({})).unwrap();
        assert_eq!(s2.doc.plain_text(StoryRef::Part(orig_ftr_id)), "Footer to be removed");
    }
}

/// #5837: Multi-section document header comparison traverses sections and diffs each section's header.
#[test]
fn compare_tracks_multi_section_headers() {
    let mut orig = Session::new(Document::from_text("Section 1 body.\nSection 2 body."));
    let mut rev = Session::new(Document::from_text("Section 1 body.\nSection 2 body."));

    let h1_orig = orig.doc.add_part(
        wordcraft_doc::PartKind::Header,
        vec![para_block(Paragraph::with_text("Section 1 Original Header", CharProps::default()).styled("Header"))],
    );
    let h2_orig = orig.doc.add_part(
        wordcraft_doc::PartKind::Header,
        vec![para_block(Paragraph::with_text("Section 2 Original Header", CharProps::default()).styled("Header"))],
    );

    let mut sec1_props = wordcraft_doc::section::SectionProps::default();
    sec1_props.headers.default = Some(h1_orig);
    if let Block::Para(p) = Arc::make_mut(&mut orig.doc.body[0]) {
        p.section = Some(Box::new(sec1_props));
    }
    orig.doc.last_section.headers.default = Some(h2_orig);

    let h1_rev = rev.doc.add_part(
        wordcraft_doc::PartKind::Header,
        vec![para_block(Paragraph::with_text("Section 1 Revised Header", CharProps::default()).styled("Header"))],
    );
    let h2_rev = rev.doc.add_part(
        wordcraft_doc::PartKind::Header,
        vec![para_block(Paragraph::with_text("Section 2 Revised Header", CharProps::default()).styled("Header"))],
    );

    let mut sec1_rev_props = wordcraft_doc::section::SectionProps::default();
    sec1_rev_props.headers.default = Some(h1_rev);
    if let Block::Para(p) = Arc::make_mut(&mut rev.doc.body[0]) {
        p.section = Some(Box::new(sec1_rev_props));
    }
    rev.doc.last_section.headers.default = Some(h2_rev);

    let rev_file = TempDocx::new("rev-multi-sec-hdr");
    rev_file.save(&rev.doc);

    let mut s = orig;
    let r = s.run("review.compare", &json!({"path": rev_file.path_str()})).unwrap();

    assert_eq!(r["changes"], 2, "Expected exactly 2 changes (1 in each section header), got: {r}");

    let p1 = s.doc.parts.get(&h1_orig).unwrap().blocks[0].as_para().unwrap();
    assert!(p1.run_ranges().any(|(_, c)| c.del.is_some()), "Section 1 header has del");
    assert!(p1.run_ranges().any(|(_, c)| c.ins.is_some()), "Section 1 header has ins");

    let p2 = s.doc.parts.get(&h2_orig).unwrap().blocks[0].as_para().unwrap();
    assert!(p2.run_ranges().any(|(_, c)| c.del.is_some()), "Section 2 header has del");
    assert!(p2.run_ranges().any(|(_, c)| c.ins.is_some()), "Section 2 header has ins");

    s.run("review.acceptAll", &json!({})).unwrap();
    assert_eq!(s.doc.plain_text(StoryRef::Part(h1_orig)), "Section 1 Revised Header");
    assert_eq!(s.doc.plain_text(StoryRef::Part(h2_orig)), "Section 2 Revised Header");
}

/// #5837: Multi-section document footer comparison traverses sections and diffs each section's footer.
#[test]
fn compare_tracks_multi_section_footers() {
    let mut orig = Session::new(Document::from_text("Section 1 body.\nSection 2 body."));
    let mut rev = Session::new(Document::from_text("Section 1 body.\nSection 2 body."));

    let f1_orig = orig.doc.add_part(
        wordcraft_doc::PartKind::Footer,
        vec![para_block(Paragraph::with_text("Section 1 Original Footer", CharProps::default()).styled("Footer"))],
    );
    let f2_orig = orig.doc.add_part(
        wordcraft_doc::PartKind::Footer,
        vec![para_block(Paragraph::with_text("Section 2 Original Footer", CharProps::default()).styled("Footer"))],
    );

    let mut sec1_props = wordcraft_doc::section::SectionProps::default();
    sec1_props.footers.default = Some(f1_orig);
    if let Block::Para(p) = Arc::make_mut(&mut orig.doc.body[0]) {
        p.section = Some(Box::new(sec1_props));
    }
    orig.doc.last_section.footers.default = Some(f2_orig);

    let f1_rev = rev.doc.add_part(
        wordcraft_doc::PartKind::Footer,
        vec![para_block(Paragraph::with_text("Section 1 Revised Footer", CharProps::default()).styled("Footer"))],
    );
    let f2_rev = rev.doc.add_part(
        wordcraft_doc::PartKind::Footer,
        vec![para_block(Paragraph::with_text("Section 2 Revised Footer", CharProps::default()).styled("Footer"))],
    );

    let mut sec1_rev_props = wordcraft_doc::section::SectionProps::default();
    sec1_rev_props.footers.default = Some(f1_rev);
    if let Block::Para(p) = Arc::make_mut(&mut rev.doc.body[0]) {
        p.section = Some(Box::new(sec1_rev_props));
    }
    rev.doc.last_section.footers.default = Some(f2_rev);

    let rev_file = TempDocx::new("rev-multi-sec-ftr");
    rev_file.save(&rev.doc);

    let mut s = orig;
    let r = s.run("review.compare", &json!({"path": rev_file.path_str()})).unwrap();

    assert_eq!(r["changes"], 2, "Expected exactly 2 changes (1 in each section footer), got: {r}");

    let p1 = s.doc.parts.get(&f1_orig).unwrap().blocks[0].as_para().unwrap();
    assert!(p1.run_ranges().any(|(_, c)| c.del.is_some()), "Section 1 footer has del");
    assert!(p1.run_ranges().any(|(_, c)| c.ins.is_some()), "Section 1 footer has ins");

    let p2 = s.doc.parts.get(&f2_orig).unwrap().blocks[0].as_para().unwrap();
    assert!(p2.run_ranges().any(|(_, c)| c.del.is_some()), "Section 2 footer has del");
    assert!(p2.run_ranges().any(|(_, c)| c.ins.is_some()), "Section 2 footer has ins");

    s.run("review.acceptAll", &json!({})).unwrap();
    assert_eq!(s.doc.plain_text(StoryRef::Part(f1_orig)), "Section 1 Revised Footer");
    assert_eq!(s.doc.plain_text(StoryRef::Part(f2_orig)), "Section 2 Revised Footer");
}

/// #5837: Deletion of a header in a non-final section preserves the header reference in that section
/// so the tracked deletion remains referenced and visible. Both acceptAll and rejectAll are verified.
#[test]
fn compare_tracks_deleted_header_in_non_final_section() {
    let mut orig = Session::new(Document::from_text("Section 1 body.\nSection 2 body."));
    let mut rev = Session::new(Document::from_text("Section 1 body.\nSection 2 body."));

    let h1_orig = orig.doc.add_part(
        wordcraft_doc::PartKind::Header,
        vec![para_block(Paragraph::with_text("Non-final section header to delete", CharProps::default()).styled("Header"))],
    );
    let mut sec1_props = wordcraft_doc::section::SectionProps::default();
    sec1_props.headers.default = Some(h1_orig);
    if let Block::Para(p) = Arc::make_mut(&mut orig.doc.body[0]) {
        p.section = Some(Box::new(sec1_props));
    }

    // Revised keeps the section break but removes the header from Section 1:
    let sec1_rev_props = wordcraft_doc::section::SectionProps::default();
    if let Block::Para(p) = Arc::make_mut(&mut rev.doc.body[0]) {
        p.section = Some(Box::new(sec1_rev_props));
    }

    let rev_file = TempDocx::new("rev-del-nonfinal");
    rev_file.save(&rev.doc);

    // Verify accept of deletion:
    {
        let mut s = Session::new(orig.doc.clone());
        let r = s.run("review.compare", &json!({"path": rev_file.path_str()})).unwrap();
        assert_eq!(r["changes"], 1, "Expected exactly 1 change for deleted non-final header");

        let sec1 = s.doc.sections()[0].1;
        assert_eq!(sec1.headers.default, Some(h1_orig), "Non-final section must retain header reference");

        let part = s.doc.parts.get(&h1_orig).unwrap();
        let para = part.blocks[0].as_para().unwrap();
        assert!(para.run_ranges().any(|(_, c)| c.del.is_some()), "Header must have deleted run");

        s.run("review.acceptAll", &json!({})).unwrap();
        let text = s.doc.plain_text(StoryRef::Part(h1_orig));
        assert!(text.is_empty(), "Accepted deleted header story text is empty, got: {text}");
    }

    // Verify reject of deletion:
    {
        let mut s = orig;
        let r = s.run("review.compare", &json!({"path": rev_file.path_str()})).unwrap();
        assert_eq!(r["changes"], 1);
        s.run("review.rejectAll", &json!({})).unwrap();
        assert_eq!(s.doc.plain_text(StoryRef::Part(h1_orig)), "Non-final section header to delete");
    }
}

/// #5837: Overlapping ID numbering between original and revised does not swap or clobber references.
#[test]
fn compare_headers_preserves_distinct_id_spaces() {
    let mut orig = Session::new(Document::from_text("Body text."));
    let mut rev = Session::new(Document::from_text("Body text."));

    // Orig has only default header (id 1)
    let h_orig_def = orig.doc.add_part(
        wordcraft_doc::PartKind::Header,
        vec![para_block(Paragraph::with_text("Original Default Header", CharProps::default()).styled("Header"))],
    );
    orig.doc.last_section.headers.default = Some(h_orig_def);

    // Rev has first-page header (id 1 in rev) and default header (id 2 in rev)
    let h_rev_first = rev.doc.add_part(
        wordcraft_doc::PartKind::Header,
        vec![para_block(Paragraph::with_text("Revised First Header", CharProps::default()).styled("Header"))],
    );
    let h_rev_def = rev.doc.add_part(
        wordcraft_doc::PartKind::Header,
        vec![para_block(Paragraph::with_text("Revised Default Header", CharProps::default()).styled("Header"))],
    );
    rev.doc.last_section.title_page = true;
    rev.doc.last_section.headers.first = Some(h_rev_first);
    rev.doc.last_section.headers.default = Some(h_rev_def);

    let rev_file = TempDocx::new("rev-ids");
    rev_file.save(&rev.doc);

    let mut s = orig;
    let r = s.run("review.compare", &json!({"path": rev_file.path_str()})).unwrap();

    assert_eq!(r["changes"], 2, "Expected 2 changes (1 in default header, 1 added first header)");
    // Default header must still be h_orig_def, diffed with h_rev_def:
    assert_eq!(s.doc.last_section.headers.default, Some(h_orig_def));
    // First-page header was newly added:
    let first_id = s.doc.last_section.headers.first.expect("First header added");
    assert_ne!(first_id, h_orig_def, "First header ID must not collide with default header ID");

    let p_def = s.doc.parts.get(&h_orig_def).unwrap().blocks[0].as_para().unwrap();
    assert!(p_def.run_ranges().any(|(_, c)| c.del.is_some()));
    assert!(p_def.run_ranges().any(|(_, c)| c.ins.is_some()));

    let p_first = s.doc.parts.get(&first_id).unwrap().blocks[0].as_para().unwrap();
    assert!(p_first.run_ranges().any(|(_, c)| c.ins.is_some()));

    s.run("review.acceptAll", &json!({})).unwrap();
    assert_eq!(s.doc.plain_text(StoryRef::Part(h_orig_def)), "Revised Default Header");
    assert_eq!(s.doc.plain_text(StoryRef::Part(first_id)), "Revised First Header");
}

/// #5837: When revised adds an extra section and changes the final section header,
/// body correspondence maps both sections correctly so both headers retain their tracked changes.
#[test]
fn compare_extra_revised_section_with_changed_final_header() {
    let mut orig = Session::new(Document::from_text("Final section body."));
    let mut rev = Session::new(Document::from_text("Section 1 body.\nFinal section body."));

    let h_orig_final = orig.doc.add_part(
        wordcraft_doc::PartKind::Header,
        vec![para_block(Paragraph::with_text("Original Final Header", CharProps::default()).styled("Header"))],
    );
    orig.doc.last_section.headers.default = Some(h_orig_final);

    let h_rev_sec1 = rev.doc.add_part(
        wordcraft_doc::PartKind::Header,
        vec![para_block(Paragraph::with_text("Revised Section 1 Header", CharProps::default()).styled("Header"))],
    );
    let mut sec1_props = wordcraft_doc::section::SectionProps::default();
    sec1_props.headers.default = Some(h_rev_sec1);
    if let Block::Para(p) = Arc::make_mut(&mut rev.doc.body[0]) {
        p.section = Some(Box::new(sec1_props));
    }

    let h_rev_final = rev.doc.add_part(
        wordcraft_doc::PartKind::Header,
        vec![para_block(Paragraph::with_text("Changed Final Header", CharProps::default()).styled("Header"))],
    );
    rev.doc.last_section.headers.default = Some(h_rev_final);

    let rev_file = TempDocx::new("rev-extra-sec");
    rev_file.save(&rev.doc);

    let mut s = orig;
    let r = s.run("review.compare", &json!({"path": rev_file.path_str()})).unwrap();

    // 1 inserted Section 1 body block + 1 inserted Section 1 header + 1 diffed final header = 3 changes
    assert_eq!(r["changes"], 3, "Expected exactly 3 changes (body + sec1 header + final header), got: {r}");

    let sections = s.doc.sections();
    assert_eq!(sections.len(), 2, "Result document must have 2 sections");

    // Section 0 must have the inserted Revised Section 1 Header:
    let sec0_h = sections[0].1.headers.default.expect("Section 0 header present");
    assert_ne!(sec0_h, h_orig_final);
    let p0 = s.doc.parts.get(&sec0_h).unwrap().blocks[0].as_para().unwrap();
    assert!(p0.run_ranges().any(|(_, c)| c.ins.is_some()), "Section 0 header has tracked insertion");

    // Section 1 (final section) must have h_orig_final diffed with Changed Final Header:
    let sec1_h = s.doc.last_section.headers.default.expect("Section 1 header present");
    assert_eq!(sec1_h, h_orig_final);
    let p1 = s.doc.parts.get(&sec1_h).unwrap().blocks[0].as_para().unwrap();
    assert!(p1.run_ranges().any(|(_, c)| c.del.is_some()), "Section 1 header has tracked deletion");
    assert!(p1.run_ranges().any(|(_, c)| c.ins.is_some()), "Section 1 header has tracked insertion");

    // Verify both headers reflect their revised text when accepted:
    s.run("review.acceptAll", &json!({})).unwrap();
    assert_eq!(s.doc.plain_text(StoryRef::Part(sec0_h)), "Revised Section 1 Header");
    assert_eq!(s.doc.plain_text(StoryRef::Part(sec1_h)), "Changed Final Header");
}

/// #5837: Inserting First and Even headers preserves selection flags (title_page, even_odd_headers)
/// so the inserted stories are selectable under layout.
#[test]
fn compare_inserted_first_and_even_headers_enable_layout_flags() {
    let mut orig = Session::new(Document::from_text("Body text."));
    orig.doc.last_section.title_page = false;
    orig.doc.settings.even_odd_headers = false;

    let h_orig_def = orig.doc.add_part(
        wordcraft_doc::PartKind::Header,
        vec![para_block(Paragraph::with_text("Original Default Header", CharProps::default()).styled("Header"))],
    );
    orig.doc.last_section.headers.default = Some(h_orig_def);

    let mut rev = Session::new(Document::from_text("Body text."));
    rev.doc.last_section.title_page = true;
    rev.doc.settings.even_odd_headers = true;

    let h_rev_def = rev.doc.add_part(
        wordcraft_doc::PartKind::Header,
        vec![para_block(Paragraph::with_text("Original Default Header", CharProps::default()).styled("Header"))],
    );
    let h_rev_first = rev.doc.add_part(
        wordcraft_doc::PartKind::Header,
        vec![para_block(Paragraph::with_text("Inserted First Header", CharProps::default()).styled("Header"))],
    );
    let h_rev_even = rev.doc.add_part(
        wordcraft_doc::PartKind::Header,
        vec![para_block(Paragraph::with_text("Inserted Even Header", CharProps::default()).styled("Header"))],
    );
    rev.doc.last_section.headers.default = Some(h_rev_def);
    rev.doc.last_section.headers.first = Some(h_rev_first);
    rev.doc.last_section.headers.even = Some(h_rev_even);

    let rev_file = TempDocx::new("rev-first-even-hdr");
    rev_file.save(&rev.doc);

    let mut s = orig;
    let r = s.run("review.compare", &json!({"path": rev_file.path_str()})).unwrap();

    assert_eq!(r["changes"], 2, "Expected exactly 2 changes for First and Even headers");

    // Verify layout flags were enabled:
    assert!(s.doc.last_section.title_page, "title_page must be enabled for inserted First header");
    assert!(s.doc.settings.even_odd_headers, "even_odd_headers must be enabled for inserted Even header");

    let first_id = s.doc.last_section.headers.first.expect("First header assigned");
    let even_id = s.doc.last_section.headers.even.expect("Even header assigned");

    let first_part = s.doc.parts.get(&first_id).unwrap();
    let first_para = first_part.blocks[0].as_para().unwrap();
    assert!(first_para.run_ranges().any(|(_, c)| c.ins.is_some()), "First header has tracked insertion");

    let even_part = s.doc.parts.get(&even_id).unwrap();
    let even_para = even_part.blocks[0].as_para().unwrap();
    assert!(even_para.run_ranges().any(|(_, c)| c.ins.is_some()), "Even header has tracked insertion");
}

/// #5837: Two original sections sharing the same header part ID with distinct revised headers
/// clone the original part for the second pairing so revisions are not nested and both headers are diffed cleanly.
#[test]
fn compare_shared_original_header_with_distinct_revised_headers() {
    let mut orig = Session::new(Document::from_text("Section 1 body.\nSection 2 body."));
    let mut rev = Session::new(Document::from_text("Section 1 body.\nSection 2 body."));

    let h_orig = orig.doc.add_part(
        wordcraft_doc::PartKind::Header,
        vec![para_block(Paragraph::with_text("Shared Original Header", CharProps::default()).styled("Header"))],
    );
    let mut sec1_orig_props = wordcraft_doc::section::SectionProps::default();
    sec1_orig_props.headers.default = Some(h_orig);
    if let Block::Para(p) = Arc::make_mut(&mut orig.doc.body[0]) {
        p.section = Some(Box::new(sec1_orig_props));
    }
    orig.doc.last_section.headers.default = Some(h_orig);

    let h_rev1 = rev.doc.add_part(
        wordcraft_doc::PartKind::Header,
        vec![para_block(Paragraph::with_text("Revised Section 1 Header", CharProps::default()).styled("Header"))],
    );
    let mut sec1_rev_props = wordcraft_doc::section::SectionProps::default();
    sec1_rev_props.headers.default = Some(h_rev1);
    if let Block::Para(p) = Arc::make_mut(&mut rev.doc.body[0]) {
        p.section = Some(Box::new(sec1_rev_props));
    }

    let h_rev2 = rev.doc.add_part(
        wordcraft_doc::PartKind::Header,
        vec![para_block(Paragraph::with_text("Revised Section 2 Header", CharProps::default()).styled("Header"))],
    );
    rev.doc.last_section.headers.default = Some(h_rev2);

    let rev_file = TempDocx::new("rev-shared-hdr");
    rev_file.save(&rev.doc);

    let mut s = orig;
    let r = s.run("review.compare", &json!({"path": rev_file.path_str()})).unwrap();

    assert_eq!(r["changes"], 2, "Expected 2 changes across the two section headers");

    let sec1_h = s.doc.sections()[0].1.headers.default.expect("Section 1 header present");
    let sec2_h = s.doc.last_section.headers.default.expect("Section 2 header present");
    assert_ne!(sec1_h, sec2_h, "Shared header must be cloned into distinct parts when revised headers differ");

    let p1 = s.doc.parts.get(&sec1_h).unwrap().blocks[0].as_para().unwrap();
    assert!(p1.run_ranges().any(|(_, c)| c.del.is_some()));
    assert!(p1.run_ranges().any(|(_, c)| c.ins.is_some()));

    let p2 = s.doc.parts.get(&sec2_h).unwrap().blocks[0].as_para().unwrap();
    assert!(p2.run_ranges().any(|(_, c)| c.del.is_some()));
    assert!(p2.run_ranges().any(|(_, c)| c.ins.is_some()));

    s.run("review.acceptAll", &json!({})).unwrap();
    assert_eq!(s.doc.plain_text(StoryRef::Part(sec1_h)), "Revised Section 1 Header");
    assert_eq!(s.doc.plain_text(StoryRef::Part(sec2_h)), "Revised Section 2 Header");
}

/// #5837: Two original sections sharing the same footer part ID with distinct revised footers
/// clone the original part for the second pairing so revisions are not nested and both footers are diffed cleanly.
#[test]
fn compare_shared_original_footer_with_distinct_revised_footers() {
    let mut orig = Session::new(Document::from_text("Section 1 body.\nSection 2 body."));
    let mut rev = Session::new(Document::from_text("Section 1 body.\nSection 2 body."));

    let f_orig = orig.doc.add_part(
        wordcraft_doc::PartKind::Footer,
        vec![para_block(Paragraph::with_text("Shared Original Footer", CharProps::default()).styled("Footer"))],
    );
    let mut sec1_orig_props = wordcraft_doc::section::SectionProps::default();
    sec1_orig_props.footers.default = Some(f_orig);
    if let Block::Para(p) = Arc::make_mut(&mut orig.doc.body[0]) {
        p.section = Some(Box::new(sec1_orig_props));
    }
    orig.doc.last_section.footers.default = Some(f_orig);

    let f_rev1 = rev.doc.add_part(
        wordcraft_doc::PartKind::Footer,
        vec![para_block(Paragraph::with_text("Revised Section 1 Footer", CharProps::default()).styled("Footer"))],
    );
    let mut sec1_rev_props = wordcraft_doc::section::SectionProps::default();
    sec1_rev_props.footers.default = Some(f_rev1);
    if let Block::Para(p) = Arc::make_mut(&mut rev.doc.body[0]) {
        p.section = Some(Box::new(sec1_rev_props));
    }

    let f_rev2 = rev.doc.add_part(
        wordcraft_doc::PartKind::Footer,
        vec![para_block(Paragraph::with_text("Revised Section 2 Footer", CharProps::default()).styled("Footer"))],
    );
    rev.doc.last_section.footers.default = Some(f_rev2);

    let rev_file = TempDocx::new("rev-shared-ftr");
    rev_file.save(&rev.doc);

    let mut s = orig;
    let r = s.run("review.compare", &json!({"path": rev_file.path_str()})).unwrap();

    assert_eq!(r["changes"], 2, "Expected 2 changes across the two section footers");

    let sec1_f = s.doc.sections()[0].1.footers.default.expect("Section 1 footer present");
    let sec2_f = s.doc.last_section.footers.default.expect("Section 2 footer present");
    assert_ne!(sec1_f, sec2_f, "Shared footer must be cloned into distinct parts when revised footers differ");

    let p1 = s.doc.parts.get(&sec1_f).unwrap().blocks[0].as_para().unwrap();
    assert!(p1.run_ranges().any(|(_, c)| c.del.is_some()));
    assert!(p1.run_ranges().any(|(_, c)| c.ins.is_some()));

    let p2 = s.doc.parts.get(&sec2_f).unwrap().blocks[0].as_para().unwrap();
    assert!(p2.run_ranges().any(|(_, c)| c.del.is_some()));
    assert!(p2.run_ranges().any(|(_, c)| c.ins.is_some()));

    s.run("review.acceptAll", &json!({})).unwrap();
    assert_eq!(s.doc.plain_text(StoryRef::Part(sec1_f)), "Revised Section 1 Footer");
    assert_eq!(s.doc.plain_text(StoryRef::Part(sec2_f)), "Revised Section 2 Footer");
}

/// #5837: Two sections sharing the same (o, r) pairing reuse a single result part without redundant cloning.
#[test]
fn compare_shared_original_header_with_shared_revised_header_reuses_part() {
    let mut orig = Session::new(Document::from_text("Section 1 body.\nSection 2 body."));
    let mut rev = Session::new(Document::from_text("Section 1 body.\nSection 2 body."));

    let h_orig = orig.doc.add_part(
        wordcraft_doc::PartKind::Header,
        vec![para_block(Paragraph::with_text("Shared Original Header", CharProps::default()).styled("Header"))],
    );
    let mut sec1_orig_props = wordcraft_doc::section::SectionProps::default();
    sec1_orig_props.headers.default = Some(h_orig);
    if let Block::Para(p) = Arc::make_mut(&mut orig.doc.body[0]) {
        p.section = Some(Box::new(sec1_orig_props));
    }
    orig.doc.last_section.headers.default = Some(h_orig);

    let h_rev = rev.doc.add_part(
        wordcraft_doc::PartKind::Header,
        vec![para_block(Paragraph::with_text("Shared Revised Header", CharProps::default()).styled("Header"))],
    );
    let mut sec1_rev_props = wordcraft_doc::section::SectionProps::default();
    sec1_rev_props.headers.default = Some(h_rev);
    if let Block::Para(p) = Arc::make_mut(&mut rev.doc.body[0]) {
        p.section = Some(Box::new(sec1_rev_props));
    }
    rev.doc.last_section.headers.default = Some(h_rev);

    let rev_file = TempDocx::new("rev-shared-pair");
    rev_file.save(&rev.doc);

    let mut s = orig;
    let r = s.run("review.compare", &json!({"path": rev_file.path_str()})).unwrap();

    assert_eq!(r["changes"], 1, "Expected 1 change for shared header diff");

    let sec1_h = s.doc.sections()[0].1.headers.default.expect("Section 1 header present");
    let sec2_h = s.doc.last_section.headers.default.expect("Section 2 header present");
    assert_eq!(sec1_h, sec2_h, "Shared header across sections must reuse the same target part ID");

    let p = s.doc.parts.get(&sec1_h).unwrap().blocks[0].as_para().unwrap();
    assert!(p.run_ranges().any(|(_, c)| c.del.is_some()));
    assert!(p.run_ranges().any(|(_, c)| c.ins.is_some()));

    s.run("review.acceptAll", &json!({})).unwrap();
    assert_eq!(s.doc.plain_text(StoryRef::Part(sec1_h)), "Shared Revised Header");
}

/// #5837: Two original sections share a header part ID. Section 1 matches a revised header,
/// and Section 2 in revised links to previous (headers.default = None). Compare resolves Section 2's
/// effective revised header as inherited from Section 1, correctly diffing and accepting/rejecting both.
#[test]
fn compare_shared_original_header_matched_first_then_deleted() {
    let mut orig = Session::new(Document::from_text("Section 1 body.\nSection 2 body."));
    let mut rev = Session::new(Document::from_text("Section 1 body.\nSection 2 body."));

    let h_orig = orig.doc.add_part(
        wordcraft_doc::PartKind::Header,
        vec![para_block(Paragraph::with_text("Shared Original Header", CharProps::default()).styled("Header"))],
    );
    let mut sec1_orig_props = wordcraft_doc::section::SectionProps::default();
    sec1_orig_props.headers.default = Some(h_orig);
    if let Block::Para(p) = Arc::make_mut(&mut orig.doc.body[0]) {
        p.section = Some(Box::new(sec1_orig_props));
    }
    orig.doc.last_section.headers.default = Some(h_orig);

    let h_rev = rev.doc.add_part(
        wordcraft_doc::PartKind::Header,
        vec![para_block(Paragraph::with_text("Revised Section 1 Header", CharProps::default()).styled("Header"))],
    );
    let mut sec1_rev_props = wordcraft_doc::section::SectionProps::default();
    sec1_rev_props.headers.default = Some(h_rev);
    if let Block::Para(p) = Arc::make_mut(&mut rev.doc.body[0]) {
        p.section = Some(Box::new(sec1_rev_props));
    }
    // Section 2 in revised links to Section 1 (inherits h_rev).
    rev.doc.last_section.headers.default = None;

    let rev_file = TempDocx::new("rev-match-del");
    rev_file.save(&rev.doc);

    // Test accept:
    {
        let mut s = Session::new(orig.doc.clone());
        let r = s.run("review.compare", &json!({"path": rev_file.path_str()})).unwrap();
        assert_eq!(r["changes"], 1);

        let sec1_h = s.doc.sections()[0].1.headers.default.expect("Section 1 header present");
        s.run("review.acceptAll", &json!({})).unwrap();
        assert_eq!(s.doc.plain_text(StoryRef::Part(sec1_h)), "Revised Section 1 Header");

        // Section 2 inherits Section 1's header:
        let sec2_eff = s.doc.last_section.headers.default.unwrap_or(sec1_h);
        assert_eq!(s.doc.plain_text(StoryRef::Part(sec2_eff)), "Revised Section 1 Header");
    }

    // Test reject:
    {
        let mut s = orig;
        let r = s.run("review.compare", &json!({"path": rev_file.path_str()})).unwrap();
        assert_eq!(r["changes"], 1);

        let sec1_h = s.doc.sections()[0].1.headers.default.expect("Section 1 header present");
        s.run("review.rejectAll", &json!({})).unwrap();
        assert_eq!(s.doc.plain_text(StoryRef::Part(sec1_h)), "Shared Original Header");

        let sec2_eff = s.doc.last_section.headers.default.unwrap_or(sec1_h);
        assert_eq!(s.doc.plain_text(StoryRef::Part(sec2_eff)), "Shared Original Header");
    }
}

/// #5837: Two original sections share a header part ID, Section 1 deletes the header,
/// and Section 2 matches a revised header. The original part marked deleted for Section 1
/// must not be clobbered by Section 2's diff; Section 2 gets its own cloned target part.
#[test]
fn compare_shared_original_header_deleted_first_then_matched() {
    let mut orig = Session::new(Document::from_text("Section 1 body.\nSection 2 body."));
    let mut rev = Session::new(Document::from_text("Section 1 body.\nSection 2 body."));

    let h_orig = orig.doc.add_part(
        wordcraft_doc::PartKind::Header,
        vec![para_block(Paragraph::with_text("Shared Original Header", CharProps::default()).styled("Header"))],
    );
    let mut sec1_orig_props = wordcraft_doc::section::SectionProps::default();
    sec1_orig_props.headers.default = Some(h_orig);
    if let Block::Para(p) = Arc::make_mut(&mut orig.doc.body[0]) {
        p.section = Some(Box::new(sec1_orig_props));
    }
    orig.doc.last_section.headers.default = Some(h_orig);

    // Section 1 in revised has NO header (deleted).
    let mut sec1_rev_props = wordcraft_doc::section::SectionProps::default();
    sec1_rev_props.headers.default = None;
    if let Block::Para(p) = Arc::make_mut(&mut rev.doc.body[0]) {
        p.section = Some(Box::new(sec1_rev_props));
    }

    let h_rev = rev.doc.add_part(
        wordcraft_doc::PartKind::Header,
        vec![para_block(Paragraph::with_text("Revised Section 2 Header", CharProps::default()).styled("Header"))],
    );
    rev.doc.last_section.headers.default = Some(h_rev);

    let rev_file = TempDocx::new("rev-del-match");
    rev_file.save(&rev.doc);

    // Test accept:
    {
        let mut s = Session::new(orig.doc.clone());
        let r = s.run("review.compare", &json!({"path": rev_file.path_str()})).unwrap();
        assert_eq!(r["changes"], 2, "Expected 2 changes (1 deleted header + 1 diffed header)");

        let sec1_h = s.doc.sections()[0].1.headers.default.expect("Section 1 header present");
        let sec2_h = s.doc.last_section.headers.default.expect("Section 2 header present");
        assert_ne!(sec1_h, sec2_h, "Section 1 and Section 2 must have distinct parts");

        // Section 1 header must be marked deleted:
        let sec1_part = s.doc.parts.get(&sec1_h).unwrap();
        let sec1_para = sec1_part.blocks[0].as_para().unwrap();
        assert!(sec1_para.run_ranges().all(|(_, c)| c.del.is_some()), "Section 1 header must be marked deleted");

        s.run("review.acceptAll", &json!({})).unwrap();
        let text1 = s.doc.plain_text(StoryRef::Part(sec1_h));
        assert!(text1.is_empty(), "Accepted deleted header story text is empty, got: {text1}");
        assert_eq!(s.doc.plain_text(StoryRef::Part(sec2_h)), "Revised Section 2 Header");
    }

    // Test reject:
    {
        let mut s = orig;
        let r = s.run("review.compare", &json!({"path": rev_file.path_str()})).unwrap();
        assert_eq!(r["changes"], 2);

        let sec1_h = s.doc.sections()[0].1.headers.default.expect("Section 1 header present");
        let sec2_h = s.doc.last_section.headers.default.expect("Section 2 header present");
        s.run("review.rejectAll", &json!({})).unwrap();
        assert_eq!(s.doc.plain_text(StoryRef::Part(sec1_h)), "Shared Original Header");
        assert_eq!(s.doc.plain_text(StoryRef::Part(sec2_h)), "Shared Original Header");
    }
}

/// #5837: When comparing documents where a result section has no body blocks (such as empty bodies),
/// both `orig_si` and `rev_si` fall back to the section index so header/footer diffs are paired and tracked.
#[test]
fn compare_empty_body_documents_compare_headers() {
    let mut orig = Session::new(Document::default());
    let mut rev = Session::new(Document::default());

    let h_orig = orig
        .doc
        .add_part(wordcraft_doc::PartKind::Header, vec![para_block(Paragraph::with_text("Original Header", CharProps::default()).styled("Header"))]);
    orig.doc.last_section.headers.default = Some(h_orig);

    let h_rev = rev
        .doc
        .add_part(wordcraft_doc::PartKind::Header, vec![para_block(Paragraph::with_text("Revised Header", CharProps::default()).styled("Header"))]);
    rev.doc.last_section.headers.default = Some(h_rev);

    let rev_file = TempDocx::new("rev-empty-body");
    rev_file.save(&rev.doc);

    let mut s = orig;
    let r = s.run("review.compare", &json!({"path": rev_file.path_str()})).unwrap();

    assert_eq!(r["changes"], 1, "Expected 1 change in header diff");

    let sec_h = s.doc.last_section.headers.default.expect("Header assigned");
    let p = s.doc.parts.get(&sec_h).unwrap().blocks[0].as_para().unwrap();
    assert!(p.run_ranges().any(|(_, c)| c.del.is_some()));
    assert!(p.run_ranges().any(|(_, c)| c.ins.is_some()));

    s.run("review.acceptAll", &json!({})).unwrap();
    assert_eq!(s.doc.plain_text(StoryRef::Part(sec_h)), "Revised Header");
}

/// #5837: Two-section document where one side has a linked/inherited header (slot is None)
/// and the other side defines an explicit header. Compare resolves effective stories,
/// correctly tracking revisions and preserving the inherited or explicit story on accept/reject.
#[test]
fn compare_inherited_header_on_one_side_accept_and_reject() {
    // Case 1: Original Section 2 inherits Section 1's header; Revised Section 2 unlinks with a custom header.
    let mut orig = Session::new(Document::from_text("Section 1 body.\nSection 2 body."));
    let mut rev = Session::new(Document::from_text("Section 1 body.\nSection 2 body."));

    let h0 = orig
        .doc
        .add_part(wordcraft_doc::PartKind::Header, vec![para_block(Paragraph::with_text("Base Header", CharProps::default()).styled("Header"))]);
    let mut sec1_props = wordcraft_doc::section::SectionProps::default();
    sec1_props.headers.default = Some(h0);
    if let Block::Para(p) = Arc::make_mut(&mut orig.doc.body[0]) {
        p.section = Some(Box::new(sec1_props.clone()));
    }
    // orig Section 2 leaves headers.default = None (inherits h0)

    let h0_rev = rev
        .doc
        .add_part(wordcraft_doc::PartKind::Header, vec![para_block(Paragraph::with_text("Base Header", CharProps::default()).styled("Header"))]);
    let mut rev_sec1_props = wordcraft_doc::section::SectionProps::default();
    rev_sec1_props.headers.default = Some(h0_rev);
    if let Block::Para(p) = Arc::make_mut(&mut rev.doc.body[0]) {
        p.section = Some(Box::new(rev_sec1_props));
    }
    let h_custom = rev
        .doc
        .add_part(wordcraft_doc::PartKind::Header, vec![para_block(Paragraph::with_text("Custom Header", CharProps::default()).styled("Header"))]);
    rev.doc.last_section.headers.default = Some(h_custom);

    let rev_file = TempDocx::new("rev-inherit-hdr");
    rev_file.save(&rev.doc);

    // Test accept:
    {
        let mut s = Session::new(orig.doc.clone());
        let r = s.run("review.compare", &json!({"path": rev_file.path_str()})).unwrap();
        assert_eq!(r["changes"], 1);
        let sec2_h = s.doc.last_section.headers.default.expect("Section 2 header assigned");
        s.run("review.acceptAll", &json!({})).unwrap();
        assert_eq!(s.doc.plain_text(StoryRef::Part(sec2_h)), "Custom Header");
    }

    // Test reject:
    {
        let mut s = orig;
        let r = s.run("review.compare", &json!({"path": rev_file.path_str()})).unwrap();
        assert_eq!(r["changes"], 1);
        let sec2_h = s.doc.last_section.headers.default.expect("Section 2 header assigned");
        s.run("review.rejectAll", &json!({})).unwrap();
        assert_eq!(s.doc.plain_text(StoryRef::Part(sec2_h)), "Base Header");
    }
}

/// #5837: Two-section document where one side has a linked/inherited footer (slot is None)
/// and the other side defines an explicit footer. Compare resolves effective stories,
/// correctly tracking revisions and preserving the inherited or explicit story on accept/reject.
#[test]
fn compare_inherited_footer_on_one_side_accept_and_reject() {
    let mut orig = Session::new(Document::from_text("Section 1 body.\nSection 2 body."));
    let mut rev = Session::new(Document::from_text("Section 1 body.\nSection 2 body."));

    let f0 = orig
        .doc
        .add_part(wordcraft_doc::PartKind::Footer, vec![para_block(Paragraph::with_text("Base Footer", CharProps::default()).styled("Footer"))]);
    let mut sec1_props = wordcraft_doc::section::SectionProps::default();
    sec1_props.footers.default = Some(f0);
    if let Block::Para(p) = Arc::make_mut(&mut orig.doc.body[0]) {
        p.section = Some(Box::new(sec1_props.clone()));
    }
    // orig Section 2 leaves footers.default = None (inherits f0)

    let f0_rev = rev
        .doc
        .add_part(wordcraft_doc::PartKind::Footer, vec![para_block(Paragraph::with_text("Base Footer", CharProps::default()).styled("Footer"))]);
    let mut rev_sec1_props = wordcraft_doc::section::SectionProps::default();
    rev_sec1_props.footers.default = Some(f0_rev);
    if let Block::Para(p) = Arc::make_mut(&mut rev.doc.body[0]) {
        p.section = Some(Box::new(rev_sec1_props));
    }
    let f_custom = rev
        .doc
        .add_part(wordcraft_doc::PartKind::Footer, vec![para_block(Paragraph::with_text("Custom Footer", CharProps::default()).styled("Footer"))]);
    rev.doc.last_section.footers.default = Some(f_custom);

    let rev_file = TempDocx::new("rev-inherit-ftr");
    rev_file.save(&rev.doc);

    // Test accept:
    {
        let mut s = Session::new(orig.doc.clone());
        let r = s.run("review.compare", &json!({"path": rev_file.path_str()})).unwrap();
        assert_eq!(r["changes"], 1);
        let sec2_f = s.doc.last_section.footers.default.expect("Section 2 footer assigned");
        s.run("review.acceptAll", &json!({})).unwrap();
        assert_eq!(s.doc.plain_text(StoryRef::Part(sec2_f)), "Custom Footer");
    }

    // Test reject:
    {
        let mut s = orig;
        let r = s.run("review.compare", &json!({"path": rev_file.path_str()})).unwrap();
        assert_eq!(r["changes"], 1);
        let sec2_f = s.doc.last_section.footers.default.expect("Section 2 footer assigned");
        s.run("review.rejectAll", &json!({})).unwrap();
        assert_eq!(s.doc.plain_text(StoryRef::Part(sec2_f)), "Base Footer");
    }
}

/// #5837: A revised header containing an inline text box has its text box story adopted
/// into the result document, tracked with revisions, and surviving acceptAll.
#[test]
fn compare_tracks_text_box_inside_header() {
    let mut orig = Session::new(Document::from_text("Body text."));
    orig.run("insert.header", &json!({"text": "Original Header Without Box"})).unwrap();

    let mut rev = Session::new(Document::from_text("Body text."));
    rev.run("insert.header", &json!({"text": "Revised Header Before Box "})).unwrap();
    let rev_hdr_id = rev.doc.last_section.headers.default.unwrap();

    // Insert text box anchored in the header:
    rev.sel = crate::session::Selection::caret(rev.doc.end_of(StoryRef::Part(rev_hdr_id)));
    let _ = rev.run("insert.textBox", &json!({"text": "Floating Box Text Inside Header"})).unwrap();

    let rev_file = TempDocx::new("rev-hdr-box");
    rev_file.save(&rev.doc);

    let mut s = orig;
    let r = s.run("review.compare", &json!({"path": rev_file.path_str()})).unwrap();

    assert_eq!(r["changes"], 1, "Expected 1 header block change containing the text box");

    // The text box must be adopted into s.doc.parts:
    let box_id = s
        .doc
        .parts
        .iter()
        .find_map(|(&id, p)| (p.kind == wordcraft_doc::PartKind::TextBox).then_some(id))
        .expect("Text box from header must be adopted into result document parts");

    let res_hdr_id = s.doc.last_section.headers.default.unwrap();
    let hdr_part = s.doc.parts.get(&res_hdr_id).unwrap();
    let hdr_para = hdr_part.blocks[0].as_para().unwrap();

    // Verify header paragraph tracks insertion and retains text box reference:
    assert!(hdr_para.objects.iter().any(|o| o.text_boxes().contains(&box_id)));

    s.run("review.acceptAll", &json!({})).unwrap();
    assert!(s.doc.plain_text(StoryRef::Part(res_hdr_id)).contains("Revised Header Before Box"));
    assert_eq!(s.doc.plain_text(StoryRef::Part(box_id)), "Floating Box Text Inside Header");
}

/// #5837: When "headersFooters": false is passed, inserted and matched section breaks with
/// revised-only section properties are sanitized to keep the original document's part space
/// and effective headers/footers instead of retaining foreign revised part IDs or collapsing to None.
#[test]
fn compare_headers_footers_false_preserves_original_part_space_on_new_sections() {
    let mut orig = Session::new(Document::from_text("Para 1.\nPara 2."));
    let mut rev = Session::new(Document::from_text("Para 1.\nInserted with section.\nPara 2 edited."));

    // Set up original document with Section 0 (header "Header 0") and Section 1 (header "Header 1").
    let h0 = orig.doc.add_part(wordcraft_doc::PartKind::Header, vec![Arc::new(Block::Para(Paragraph::with_text("Header 0", CharProps::default())))]);
    let h1 = orig.doc.add_part(wordcraft_doc::PartKind::Header, vec![Arc::new(Block::Para(Paragraph::with_text("Header 1", CharProps::default())))]);

    // Give Para 1 in original a section break pointing to h0.
    if let Block::Para(p) = Arc::make_mut(&mut orig.doc.body[0]) {
        let mut sec = wordcraft_doc::section::SectionProps::default();
        sec.headers.default = Some(h0);
        p.section = Some(Box::new(sec));
    }
    // Give last_section in original header h1.
    orig.doc.last_section.headers.default = Some(h1);

    // In revised document, create a foreign part ID (e.g. 9999) that does not exist in orig.
    let rev_h = 9999;
    rev.doc.parts.insert(
        rev_h,
        wordcraft_doc::Part {
            kind: wordcraft_doc::PartKind::Header,
            blocks: vec![Arc::new(Block::Para(Paragraph::with_text("Foreign Revised Header", CharProps::default())))],
            body: wordcraft_doc::TextBody::default(),
        },
    );

    // In revised, "Inserted with section" has a section break with rev_h.
    if let Block::Para(p) = Arc::make_mut(&mut rev.doc.body[1]) {
        let mut sec = wordcraft_doc::section::SectionProps::default();
        sec.headers.default = Some(rev_h);
        p.section = Some(Box::new(sec));
    }

    // In revised, "Para 2 edited" (which word-diffs with Para 2) also has a section break with rev_h.
    if let Block::Para(p) = Arc::make_mut(&mut rev.doc.body[2]) {
        let mut sec = wordcraft_doc::section::SectionProps::default();
        sec.headers.default = Some(rev_h);
        p.section = Some(Box::new(sec));
    }

    let rev_file = TempDocx::new("rev-hf-partspace");
    rev_file.save(&rev.doc);

    let mut s = orig;
    let _ = s.run("review.compare", &json!({"path": rev_file.path_str(), "headersFooters": false})).unwrap();

    // Verify all sections in the result document only reference valid original parts.
    let mut effective = None;
    for (idx, sec) in s.doc.sections() {
        if let Some(hdr) = sec.headers.default {
            assert!(s.doc.parts.contains_key(&hdr), "Section at block {} references non-existent part {} when headersFooters is false", idx, hdr);
            assert_ne!(hdr, rev_h, "Section at block {} retained foreign revised part ID", idx);
            effective = Some(hdr);
        }
        let hdr = effective.expect("section must resolve an effective header, not collapse to None");
        assert!(hdr == h0 || hdr == h1, "Section at block {} must use an original header part", idx);
        let text = s.doc.plain_text(StoryRef::Part(hdr));
        assert!(text == "Header 0" || text == "Header 1", "Section at block {} header text changed: {}", idx, text);
    }
    assert_eq!(s.doc.last_section.headers.default, Some(h1));
}

/// #5837: Matched section break paragraphs with identical formatting and text do not produce
/// false formatting changes merely because their header part IDs differed before sanitization.
/// Both `headersFooters: false` and default `headersFooters: true` report zero changes.
#[test]
fn compare_matched_section_break_does_not_report_false_formatting_change() {
    let mut orig = Session::new(Document::from_text("Section 1 body.\nSection 2 body."));
    let mut rev = Session::new(Document::from_text("Section 1 body.\nSection 2 body."));

    let h_orig =
        orig.doc.add_part(wordcraft_doc::PartKind::Header, vec![Arc::new(Block::Para(Paragraph::with_text("Header", CharProps::default())))]);
    if let Block::Para(p) = Arc::make_mut(&mut orig.doc.body[0]) {
        let mut sec = wordcraft_doc::section::SectionProps::default();
        sec.headers.default = Some(h_orig);
        p.section = Some(Box::new(sec));
    }

    let h_rev = rev.doc.add_part(wordcraft_doc::PartKind::Header, vec![Arc::new(Block::Para(Paragraph::with_text("Header", CharProps::default())))]);
    if let Block::Para(p) = Arc::make_mut(&mut rev.doc.body[0]) {
        let mut sec = wordcraft_doc::section::SectionProps::default();
        sec.headers.default = Some(h_rev);
        p.section = Some(Box::new(sec));
    }

    let rev_file = TempDocx::new("rev-no-false-fmt");
    rev_file.save(&rev.doc);

    // 1. With headersFooters: false
    {
        let mut s = Session::new(orig.doc.clone());
        let r = s.run("review.compare", &json!({"path": rev_file.path_str(), "headersFooters": false, "formatting": true})).unwrap();
        assert_eq!(r["changes"], 0, "No changes should be reported when text, formatting, and headers are identical (headersFooters: false)");
    }

    // 2. With default headersFooters: true
    {
        let mut s = orig;
        let r = s.run("review.compare", &json!({"path": rev_file.path_str(), "formatting": true})).unwrap();
        assert_eq!(r["changes"], 0, "No changes should be reported when text, formatting, and headers are identical (default headersFooters)");
    }
}

/// #5837: When revised reuses a header part `r` across a matched slot (which diffs against `o`)
/// and a newly added slot (which had no original story), the newly added slot receives an insertion-only
/// part so rejecting all changes leaves no residual text from `o`.
#[test]
fn compare_revised_part_shared_between_matched_and_inserted_slots_does_not_leak_deletions() {
    let mut orig = Session::new(Document::from_text("Section 1 body.\nSection 2 body."));
    let mut rev = Session::new(Document::from_text("Section 1 body.\nSection 2 body."));

    // Original has header "Original Section 1 Header" only on section 1 (default):
    let h_orig = orig.doc.add_part(
        wordcraft_doc::PartKind::Header,
        vec![para_block(Paragraph::with_text("Original Section 1 Header", CharProps::default()).styled("Header"))],
    );
    let mut sec1_orig_props = wordcraft_doc::section::SectionProps::default();
    sec1_orig_props.headers.default = Some(h_orig);
    if let Block::Para(p) = Arc::make_mut(&mut orig.doc.body[0]) {
        p.section = Some(Box::new(sec1_orig_props));
    }
    // Section 2 in orig has NO first-page header (title_page is false, headers.first is None).

    // Revised creates header `r` ("Shared Revised Header"):
    let h_rev = rev.doc.add_part(
        wordcraft_doc::PartKind::Header,
        vec![para_block(Paragraph::with_text("Shared Revised Header", CharProps::default()).styled("Header"))],
    );
    // Revised section 1 links `r` as Default:
    let mut sec1_rev_props = wordcraft_doc::section::SectionProps::default();
    sec1_rev_props.headers.default = Some(h_rev);
    if let Block::Para(p) = Arc::make_mut(&mut rev.doc.body[0]) {
        p.section = Some(Box::new(sec1_rev_props));
    }

    // Revised section 2 links the SAME `r` as First (title_page = true):
    rev.doc.last_section.title_page = true;
    rev.doc.last_section.headers.first = Some(h_rev);

    let rev_file = TempDocx::new("rev-matched-inserted-leak");
    rev_file.save(&rev.doc);

    let mut s = orig;
    let r = s.run("review.compare", &json!({"path": rev_file.path_str()})).unwrap();

    assert_eq!(r["changes"], 2, "Expected 2 changes (1 in section 1 default header, 1 added section 2 first header)");

    // Before reject, verify section 2 first header is marked inserted:
    let sec2_first = s.doc.last_section.headers.first.expect("Section 2 first header present");
    let p_first = s.doc.parts.get(&sec2_first).unwrap().blocks[0].as_para().unwrap();
    assert!(p_first.run_ranges().any(|(_, c)| c.ins.is_some()), "Section 2 first header has insertion");
    assert!(!p_first.run_ranges().any(|(_, c)| c.del.is_some()), "Section 2 first header must NOT contain deletions from original");

    // After rejectAll, section 2 first header must NOT contain text from h_orig:
    s.run("review.rejectAll", &json!({})).unwrap();
    let text = s.doc.plain_text(StoryRef::Part(sec2_first));
    assert!(!text.contains("Original Section 1 Header"), "Section 2 first header must not retain original text after reject, got: {text}");
    assert!(text.is_empty(), "Section 2 first header text must be empty after reject, got: {text}");
}

/// #5837: When original and revised headers have identical paragraphs referencing the same
/// text box part ID but the text box story has different text, the comparison does not skip
/// diffing, tracks the change, and acceptAll retains the revised text box content.
#[test]
fn compare_tracks_text_box_content_changes_with_matching_ids() {
    let mut orig = Session::new(Document::from_text("Body text."));
    orig.run("insert.header", &json!({"text": "Header with box: "})).unwrap();
    let orig_hdr_id = orig.doc.last_section.headers.default.unwrap();
    orig.sel = crate::session::Selection::caret(orig.doc.end_of(StoryRef::Part(orig_hdr_id)));
    let orig_box = orig.run("insert.textBox", &json!({"text": "Original Box Content"})).unwrap()["story"].as_u64().unwrap() as u32;

    let mut rev = Session::new(Document::from_text("Body text."));
    rev.run("insert.header", &json!({"text": "Header with box: "})).unwrap();
    let rev_hdr_id = rev.doc.last_section.headers.default.unwrap();
    rev.sel = crate::session::Selection::caret(rev.doc.end_of(StoryRef::Part(rev_hdr_id)));
    let rev_box = rev.run("insert.textBox", &json!({"text": "Revised Box Content"})).unwrap()["story"].as_u64().unwrap() as u32;

    // Both documents assigned the exact same part IDs:
    assert_eq!(orig_hdr_id, rev_hdr_id);
    assert_eq!(orig_box, rev_box);

    let rev_file = TempDocx::new("rev-box-same-id");
    rev_file.save(&rev.doc);

    let mut s = orig;
    let r = s.run("review.compare", &json!({"path": rev_file.path_str()})).unwrap();

    // Must not skip diffing: changes must be > 0:
    assert!(r["changes"].as_u64().unwrap() > 0, "Expected changes when text box content differs, got: {r}");

    s.run("review.acceptAll", &json!({})).unwrap();

    // After acceptAll, the revised text box content must be preserved:
    let has_revised_box = s.doc.parts.values().any(|p| {
        p.kind == wordcraft_doc::PartKind::TextBox
            && p.blocks.iter().any(|b| match &**b {
                Block::Para(p) => p.plain_text().contains("Revised Box Content"),
                _ => false,
            })
    });
    assert!(has_revised_box, "Revised text box content must survive acceptAll");
}
