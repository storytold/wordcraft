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

/// The saved `word/document.xml` of `doc`.
fn document_xml(doc: &Document) -> String {
    let bytes = wordcraft_docx::write(doc).unwrap();
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let mut xml = String::new();
    std::io::Read::read_to_string(&mut z.by_name("word/document.xml").unwrap(), &mut xml).unwrap();
    xml
}

/// The text of every `tag` element of `xml` (`w:t`, `w:delText`), and whether each sits inside a
/// run-level `w:del` (not the empty `w:del` of a deleted paragraph mark), in order.
fn leaf_texts(xml: &str, tag: &str) -> Vec<(String, bool)> {
    let mut out = Vec::new();
    let mut depth = 0usize;
    let mut rest = xml;
    while let Some(i) = rest.find('<') {
        rest = &rest[i..];
        let end = rest.find('>').unwrap();
        let t = &rest[..=end];
        rest = &rest[end + 1..];
        let closing = t.starts_with("</");
        let body = if closing { &t[2..] } else { &t[1..] };
        let name = body.split(|c: char| c.is_whitespace() || c == '>' || c == '/').next().unwrap();
        let empty = t.ends_with("/>");
        if name == "w:del" && !empty {
            if closing {
                depth -= 1;
            } else {
                depth += 1;
            }
        } else if name == tag && !closing && !empty {
            let close = rest.find(&format!("</{tag}>")).unwrap();
            out.push((rest[..close].to_string(), depth > 0));
        }
    }
    out
}

/// Text written as `w:t` inside a run-level `w:del`: deleted text must be `w:delText` instead.
fn plain_text_in_deletions(xml: &str) -> String {
    leaf_texts(xml, "w:t").into_iter().filter(|(_, del)| *del).map(|(t, _)| t).collect()
}

/// A text box whose anchor Compare deletes is written deleted too: its text in `w:delText`, never
/// `w:t` inside the anchor's `w:del` (#5924: most deleted text of some results was unreadable as
/// deleted text). The revised document's new box is inserted text, `w:t`.
#[test]
fn compare_saves_a_deleted_text_box_as_deleted_text() {
    let mut s = Session::new(Document::from_text("Body one.\nKeep."));
    s.run("caret.set", &json!({"pos": Pos::body(0, "Body one.".len())})).unwrap();
    s.run("insert.textBox", &json!({"text": "Alpha boxword gamma."})).unwrap();
    s.run("review.compare", &json!({"text": "Totally new first.\nKeep."})).unwrap();
    let xml = document_xml(&s.doc);
    assert_eq!(plain_text_in_deletions(&xml), "", "{xml}");
    let deleted: String = leaf_texts(&xml, "w:delText").into_iter().map(|(t, _)| t).collect();
    assert!(deleted.contains("Alpha boxword gamma."), "the box's text is not deleted text: {deleted:?}");
    assert!(deleted.contains("Body "), "{deleted:?}");
    // The box's runs carry their own deletion, inside the box.
    let txbx = &xml[xml.find("<w:txbxContent>").unwrap()..xml.find("</w:txbxContent>").unwrap()];
    assert!(txbx.contains("<w:del ") && txbx.contains("gamma.</w:delText>"), "{txbx}");
    // The document itself is unchanged: the box's story isn't marked, only written so.
    let id = *s.doc.parts.keys().find(|id| s.doc.plain_text(StoryRef::Part(**id)) == "Alpha boxword gamma.").unwrap();
    let story = s.doc.parts[&id].blocks[0].as_para().unwrap();
    assert!(story.runs.iter().all(|r| r.props.del.is_none()));
}

/// A paragraph Compare deletes whole loses its mark too (a `w:del` in the mark's `w:rPr`), so
/// accepting removes the paragraph instead of leaving an empty one. The body's last mark stays.
#[test]
fn compare_deletes_the_mark_of_a_paragraph_deleted_whole() {
    let mut s = Session::new(Document::from_text("Keep one.\nGone para.\nKeep two."));
    s.run("review.compare", &json!({"text": "Keep one.\nKeep two."})).unwrap();
    let marks: Vec<bool> = s.doc.body.iter().map(|b| b.as_para().unwrap().mark.del.is_some()).collect();
    assert_eq!(marks, [false, true, false]);
    let xml = document_xml(&s.doc);
    assert!(xml.contains("<w:rPr><w:del "), "no deleted paragraph mark: {xml}");
    assert_eq!(plain_text_in_deletions(&xml), "", "{xml}");
    let deleted: String = leaf_texts(&xml, "w:delText").into_iter().map(|(t, _)| t).collect();
    assert_eq!(deleted, "Gone para.");
    // Read back, the mark is still deleted.
    let back = wordcraft_docx::read(&wordcraft_docx::write(&s.doc).unwrap()).unwrap();
    assert!(back.body[1].as_para().unwrap().mark.del.is_some());
    let mut accepted = Session::new(s.doc.clone());
    accepted.run("review.acceptAll", &json!({})).unwrap();
    assert_eq!(accepted.doc.body.len(), 2, "accepting left the deleted paragraph behind");
    assert_eq!(accepted.doc.plain_text(StoryRef::Body), "Keep one.\nKeep two.");
    s.run("review.rejectAll", &json!({})).unwrap();
    assert_eq!(s.doc.plain_text(StoryRef::Body), "Keep one.\nGone para.\nKeep two.");

    // The last paragraph deleted whole: its text is deleted, its mark (the body's last) is not.
    let mut s = Session::new(Document::from_text("Keep.\nGone at the end."));
    s.run("review.compare", &json!({"text": "Keep."})).unwrap();
    let last = s.doc.body.last().unwrap().as_para().unwrap();
    assert!(last.runs.iter().all(|r| r.props.del.is_some()));
    assert_eq!(last.mark.del, None);
}

/// Text moved away keeps `w:t` (in `w:moveFrom`, §17.13.5.22), and so does the text box of an
/// anchor moved with it: only deletions are written as `w:delText`.
#[test]
fn moved_away_text_and_its_text_box_keep_plain_text() {
    let mut s = Session::new(Document::from_text("Moved words.\nMoved words."));
    s.run("caret.set", &json!({"pos": Pos::body(0, "Moved words.".len())})).unwrap();
    s.run("insert.textBox", &json!({"text": "Boxed movetext."})).unwrap();
    let rev = |kind| wordcraft_doc::Revision { kind, author: "Ana".into(), date: String::new(), move_name: Some("move1".into()) };
    s.doc.revisions.push(rev(RevisionKind::Delete));
    let from = (s.doc.revisions.len() - 1) as u32;
    s.doc.revisions.push(rev(RevisionKind::Insert));
    let to = (s.doc.revisions.len() - 1) as u32;
    for (i, del, ins) in [(0usize, Some(from), None), (1, None, Some(to))] {
        let Block::Para(p) = Arc::make_mut(&mut s.doc.body[i]) else { panic!("not a paragraph") };
        let len = p.len();
        p.format(0, len, &|c| (c.del, c.ins) = (del, ins)).unwrap();
    }
    let xml = document_xml(&s.doc);
    assert!(xml.contains("<w:moveFrom "), "{xml}");
    assert!(leaf_texts(&xml, "w:delText").is_empty(), "moved text written as deleted: {xml}");
    let plain: String = leaf_texts(&xml, "w:t").into_iter().map(|(t, _)| t).collect();
    assert!(plain.contains("Moved words.") && plain.contains("Boxed movetext."), "{plain:?}");
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

