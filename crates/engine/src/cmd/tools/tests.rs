use super::*;
use std::collections::BTreeSet;

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

// #5571: what Compare adopts from the revised document is relinked to the result. Each pair is
// saved and compared as files (`original` and `path`), so both are read the same way and their
// ids (media keys, story parts, comments, lists, revisions) collide as two versions' do.

/// Compare `orig` with `rev` as .docx files; the result's session.
fn compare_files(tag: &str, orig: &Document, rev: &Document) -> Session {
    let dir = std::env::temp_dir();
    let (o, r) = (dir.join(format!("wc-5571-{tag}-o-{}.docx", std::process::id())), dir.join(format!("wc-5571-{tag}-r-{}.docx", std::process::id())));
    crate::io::save_path(&o, orig).unwrap();
    crate::io::save_path(&r, rev).unwrap();
    let mut s = Session::new(Document::new());
    let res = s.run("review.compare", &json!({"original": o.to_string_lossy(), "path": r.to_string_lossy(), "author": "Comparer"}));
    let _ = std::fs::remove_file(&o);
    let _ = std::fs::remove_file(&r);
    res.unwrap();
    s
}

/// A 2×2 PNG of one colour.
#[cfg(not(target_arch = "wasm32"))]
fn png(rgb: [u8; 3]) -> Vec<u8> {
    let mut png = Vec::new();
    image::RgbaImage::from_pixel(2, 2, image::Rgba([rgb[0], rgb[1], rgb[2], 255]))
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .unwrap();
    png
}

/// Paragraphs of `text`, with the picture of `bytes` at the end of the paragraphs that have one.
#[cfg(not(target_arch = "wasm32"))]
fn with_pictures(paras: &[(&str, Option<&[u8]>)]) -> Document {
    let mut d = Document::from_text(&paras.iter().map(|p| p.0).collect::<Vec<_>>().join("\n"));
    for (i, (text, bytes)) in paras.iter().enumerate() {
        if let Some(bytes) = bytes {
            let media = d.add_media(bytes.to_vec(), "png");
            let img = InlineObject::Image { media, w: 10.0, h: 10.0, alt: String::new(), float: Default::default(), crop: [0.0; 4], ole: None };
            d.para_mut(StoryRef::Body, &Pos::body(i, 0).path).unwrap().insert_object(text.len(), img, &CharProps::default()).unwrap();
        }
    }
    d
}

/// Every object in a story's paragraphs with whether compare deleted it (its char props' `del`).
fn objects_in(d: &Document, story: StoryRef) -> Vec<(InlineObject, bool)> {
    let mut out = Vec::new();
    for path in d.para_paths(story) {
        let p = d.para(story, &path).unwrap();
        for (off, o) in p.objects_with_offsets() {
            out.push((o.clone(), p.props_of_char(off).del.is_some()));
        }
    }
    out
}

/// The bytes of every picture in the body, and whether it is deleted, sorted.
#[cfg(not(target_arch = "wasm32"))]
fn pictures(d: &Document) -> Vec<(Vec<u8>, bool)> {
    let mut out: Vec<(Vec<u8>, bool)> = objects_in(d, StoryRef::Body)
        .into_iter()
        .filter_map(|(o, del)| match o {
            InlineObject::Image { media, .. } => {
                Some((d.media.get(&media).unwrap_or_else(|| panic!("a picture shows {media}, which the result lacks")).to_vec(), del))
            }
            _ => None,
        })
        .collect();
    out.sort();
    out
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn compare_shows_each_documents_own_pictures() {
    // Both documents key their first picture `image1.png`, with different bytes; the revised one
    // also has a picture only it has, in an inserted paragraph.
    let (a, b, c) = (png([200, 0, 0]), png([0, 200, 0]), png([0, 0, 200]));
    let orig = with_pictures(&[("Pic: ", Some(&a)), ("Same.", None)]);
    let rev = with_pictures(&[("Pic: ", Some(&b)), ("Same.", None), ("Added: ", Some(&c))]);
    let mut s = compare_files("pictures", &orig, &rev);
    let mut want = vec![(a.clone(), true), (b.clone(), false), (c.clone(), false)];
    want.sort();
    assert_eq!(pictures(&s.doc), want, "the revised pictures show their own bytes, the original's is deleted");
    let path = std::env::temp_dir().join(format!("wc-5571-pictures-out-{}.docx", std::process::id()));
    let back = crate::io::save_path(&path, &s.doc).and_then(|_| crate::io::open_path(&path));
    let _ = std::fs::remove_file(&path);
    assert_eq!(pictures(&back.unwrap()), want, "the saved result keeps each picture's bytes and deletion state");
    s.run("review.acceptAll", &json!({})).unwrap();
    let mut kept = vec![(b, false), (c, false)];
    kept.sort();
    assert_eq!(pictures(&s.doc), kept);
}

#[test]
fn compare_keeps_the_revised_documents_tracked_changes_attributed() {
    // The revised document has tracked insertions by Alice: in a paragraph both documents have
    // (matched whole) and in one only it has (inserted by the comparison).
    let orig = Document::from_text("Intro.\nKeep.");
    let mut rev = Document::from_text("Intro.\nKeep.\nAlice added.");
    rev.revisions.push(wordcraft_doc::Revision {
        kind: RevisionKind::Insert,
        author: "Alice".into(),
        date: "2026-01-02T03:04:00Z".into(),
        move_name: None,
    });
    for i in [1usize, 2] {
        let len = rev.para(StoryRef::Body, &Pos::body(i, 0).path).unwrap().len();
        rev.format_range(&Pos::body(i, 0), &Pos::body(i, len), &|c| c.ins = Some(0)).unwrap();
    }
    let s = compare_files("revisions", &orig, &rev);
    for path in s.doc.para_paths(StoryRef::Body) {
        let p = s.doc.para(StoryRef::Body, &path).unwrap();
        for (r, c) in p.run_ranges() {
            for id in [c.ins, c.del].into_iter().flatten() {
                let rv = s
                    .doc
                    .revisions
                    .get(id as usize)
                    .unwrap_or_else(|| panic!("run {:?} names revision {id}, which the result lacks", &p.text[r.clone()]));
                assert!(!rv.author.is_empty(), "revision {id} has no author");
            }
            let text = &p.text[r];
            if text.contains("Keep") || text.contains("Alice") {
                let rv = c.ins.and_then(|id| s.doc.revisions.get(id as usize)).unwrap_or_else(|| panic!("{text:?} lost its insertion"));
                assert_eq!(rv.author, "Alice", "{text:?}");
                assert!(rv.date.starts_with("2026-01-02"), "{text:?}: {}", rv.date);
            }
        }
    }
}

/// Select `text` and comment on it.
fn comment_on(s: &mut Session, text: &str, body: &str) {
    s.run("select.text", &json!({"text": text})).unwrap();
    s.run("review.newComment", &json!({"text": body})).unwrap();
}

/// The text of every comment anchored in the body (by its start), with whether it is deleted.
fn anchored_comments(d: &Document) -> Vec<(String, bool)> {
    let mut out: Vec<(String, bool)> = objects_in(d, StoryRef::Body)
        .into_iter()
        .filter_map(|(o, del)| match o {
            InlineObject::CommentStart { id } => {
                let c = d.comments.get(&id).unwrap_or_else(|| panic!("an anchor names comment {id}, which the result lacks"));
                Some((d.plain_text(StoryRef::Part(c.part)).trim().to_string(), del))
            }
            _ => None,
        })
        .collect();
    out.sort();
    out
}

#[test]
fn compare_adopts_the_revised_documents_comments() {
    // Comment 0 is on "Alpha" in both, with different text; both have the same comment on
    // "Beta"; only the revised document has one on "Gamma" (an id the original doesn't have).
    let mut o = Session::new(Document::from_text("Alpha one.\nBeta two."));
    comment_on(&mut o, "Alpha", "original note");
    comment_on(&mut o, "Beta", "shared note");
    let mut r = Session::new(Document::from_text("Alpha one.\nBeta two.\nGamma three."));
    comment_on(&mut r, "Alpha", "revised note");
    comment_on(&mut r, "Beta", "shared note");
    comment_on(&mut r, "Gamma", "gamma note");
    let s = compare_files("comments", &o.doc, &r.doc);
    let t = |t: &str, del| (t.to_string(), del);
    assert_eq!(
        anchored_comments(&s.doc),
        [t("gamma note", false), t("original note", true), t("revised note", false), t("shared note", false)],
        "each anchor shows its own document's comment"
    );
    // The original's copy of the shared comment isn't left over, anchored nowhere.
    let texts: Vec<String> = s.doc.comments.values().map(|c| s.doc.plain_text(StoryRef::Part(c.part)).trim().to_string()).collect();
    assert_eq!(texts.iter().filter(|t| *t == "shared note").count(), 1, "{texts:?}");
    assert_eq!(texts.len(), 4, "{texts:?}");
}

#[test]
fn compare_keeps_a_reply_the_result_still_anchors_when_its_root_goes() {
    // The original's root comment on "Alpha" is the same in both documents, so the result
    // shows the revised one's and the original's goes. Its reply carries its own range over
    // "Beta" (a comment read from a file can have both a parent and a range), and that range
    // survives in the result — so the reply stays, detached, instead of leaving its anchor
    // naming a comment the result lacks.
    let mut o = Session::new(Document::from_text("Alpha one.\nBeta two."));
    comment_on(&mut o, "Alpha", "root note");
    let reply = o.run("review.reply", &json!({"id": 0, "text": "anchored reply"})).unwrap()["id"].as_u64().unwrap() as u32;
    let props = Default::default();
    o.doc.insert_object(&Pos::body(1, 4), InlineObject::CommentEnd { id: reply }, &props).unwrap();
    o.doc.insert_object(&Pos::body(1, 0), InlineObject::CommentStart { id: reply }, &props).unwrap();
    let mut r = Session::new(Document::from_text("Alpha one.\nBeta 2."));
    comment_on(&mut r, "Alpha", "root note");
    let s = compare_files("anchoredreply", &o.doc, &r.doc);
    // anchored_comments panics if an anchor names a comment the result lacks.
    let t = |t: &str, del| (t.to_string(), del);
    assert_eq!(anchored_comments(&s.doc), [t("anchored reply", true), t("root note", false)]);
    let kept =
        s.doc.comments.values().find(|c| s.doc.plain_text(StoryRef::Part(c.part)).trim() == "anchored reply").expect("the anchored reply is kept");
    assert_eq!(kept.parent, None, "its root went, so the parent clearing detaches it");
}

/// Every comment's text with its parent's text (`None` for a root), sorted.
fn comment_threads(d: &Document) -> Vec<(String, Option<String>)> {
    let text = |c: &wordcraft_doc::Comment| d.plain_text(StoryRef::Part(c.part)).trim().to_string();
    let mut out: Vec<(String, Option<String>)> = d
        .comments
        .values()
        .map(|c| (text(c), c.parent.map(|p| text(d.comments.get(&p).unwrap_or_else(|| panic!("a reply names parent {p}, which is gone")))))
        .collect();
    out.sort();
    out
}

#[test]
fn compare_keeps_the_fixed_in_v2_reply_the_revised_copy_lacks() {
    // #5571 review: the original's comment on "Alpha" has the reply "Fixed in v2" (no range of
    // its own); the revised copy has only the comment. The root is deduplicated (the result
    // shows the revised one's), but the reply is the original's alone: it stays, under the
    // surviving root, and Save writes it.
    let mut o = Session::new(Document::from_text("Alpha one."));
    comment_on(&mut o, "Alpha", "root note");
    o.run("review.reply", &json!({"id": 0, "text": "Fixed in v2"})).unwrap();
    let mut r = Session::new(Document::from_text("Alpha one."));
    comment_on(&mut r, "Alpha", "root note");
    let s = compare_files("fixedinv2", &o.doc, &r.doc);
    let t = |t: &str, del| (t.to_string(), del);
    assert_eq!(anchored_comments(&s.doc), [t("root note", false)], "the root shows once");
    let thread = |s: &[(&str, Option<&str>)]| s.iter().map(|(a, b)| (a.to_string(), b.map(str::to_string))).collect::<Vec<_>>();
    let want = thread(&[("Fixed in v2", Some("root note")), ("root note", None)]);
    assert_eq!(comment_threads(&s.doc), want, "the reply stays with its thread");
    let path = std::env::temp_dir().join(format!("wc-5571-fixedinv2-out-{}.docx", std::process::id()));
    let saved = crate::io::save_path(&path, &s.doc);
    let back = saved.and_then(|_| crate::io::open_path(&path));
    let _ = std::fs::remove_file(&path);
    assert_eq!(comment_threads(&back.unwrap()), want, "the saved result has the reply, in its thread");
}

#[test]
fn compare_drops_a_reply_the_revised_thread_also_has() {
    // Both documents' comment on "Alpha" has the reply "Fixed in v2"; only the original's has
    // "Thanks" too. The revised thread's copy of "Fixed in v2" stands for both; "Thanks" moves
    // under the surviving root.
    let mut o = Session::new(Document::from_text("Alpha one."));
    comment_on(&mut o, "Alpha", "root note");
    o.run("review.reply", &json!({"id": 0, "text": "Fixed in v2"})).unwrap();
    o.run("review.reply", &json!({"id": 0, "text": "Thanks"})).unwrap();
    let mut r = Session::new(Document::from_text("Alpha one."));
    comment_on(&mut r, "Alpha", "root note");
    r.run("review.reply", &json!({"id": 0, "text": "Fixed in v2"})).unwrap();
    let s = compare_files("dupreply", &o.doc, &r.doc);
    let root = Some("root note".to_string());
    assert_eq!(
        comment_threads(&s.doc),
        [("Fixed in v2".to_string(), root.clone()), ("Thanks".to_string(), root), ("root note".to_string(), None)]
    );
}

#[test]
fn compare_keeps_a_styles_list_when_only_its_definition_changed() {
    use wordcraft_doc::styles::{Style, StyleKind};
    // Both documents define the style "Listy" numbering by list 1; only the definition behind
    // that list differs (bullets in the original, numbers in the revised). The style reference
    // alone compares equal, so the definition has to be what renames it.
    let build = |list: &str| {
        let mut s = Session::new(Document::from_text("Styled.\nItem."));
        s.run("caret.set", &json!({"pos": Pos::body(1, 0)})).unwrap();
        s.run(list, &json!({})).unwrap();
        let num = s.doc.para(StoryRef::Body, &Pos::body(1, 0).path).unwrap().props.numbering.expect("the list paragraph is numbered");
        Arc::make_mut(&mut s.doc.styles).upsert(Style {
            id: "Listy".into(),
            name: "Listy".into(),
            kind: StyleKind::Paragraph,
            para: wordcraft_doc::ParaProps { numbering: Some(num), ..Default::default() },
            ..Default::default()
        });
        s.doc.para_mut(StoryRef::Body, &Pos::body(0, 0).path).unwrap().props.style = Some("Listy".into());
        s.doc
    };
    let s = compare_files("stylelist", &build("para.bullets"), &build("para.numbering"));
    let para0 = s.doc.para(StoryRef::Body, &Pos::body(0, 0).path).unwrap();
    let style_id = para0.props.style.as_deref().expect("the styled paragraph keeps its style");
    assert_ne!(style_id, "Listy", "the adopted paragraph's style is renamed: its list definition differs");
    let style = s.doc.styles.get(style_id).expect("the renamed style is in the result");
    let num = style.para.numbering.as_ref().expect("the renamed style keeps its numbering");
    let level = s.doc.numbering.level(num.num, num.level).expect("its list is in the result");
    assert!(level.text.contains("%1"), "the revised list numbers: {:?}", level.text);
    let orig_style = s.doc.styles.get("Listy").expect("the original's style stays");
    let onum = orig_style.para.numbering.as_ref().unwrap();
    let olevel = s.doc.numbering.level(onum.num, onum.level).unwrap();
    assert!(!olevel.text.contains("%1"), "the original's list stays bullets: {:?}", olevel.text);
}

#[test]
fn compare_shows_each_documents_own_notes() {
    // Both documents' first footnote is on "One." (the same story id when read), with different
    // text; the revised one adds a note on a new paragraph.
    let note = |s: &mut Session, para: usize, text: &str| {
        let len = s.doc.para(StoryRef::Body, &Pos::body(para, 0).path).unwrap().len();
        s.run("caret.set", &json!({"pos": Pos::body(para, len)})).unwrap();
        s.run("references.footnote", &json!({"text": text})).unwrap();
    };
    let mut o = Session::new(Document::from_text("One.\nTwo."));
    note(&mut o, 0, "original foot");
    let mut r = Session::new(Document::from_text("One.\nTwo.\nThree."));
    note(&mut r, 0, "revised foot");
    note(&mut r, 2, "new foot");
    let s = compare_files("notes", &o.doc, &r.doc);
    let mut notes: Vec<(String, bool)> = objects_in(&s.doc, StoryRef::Body)
        .into_iter()
        .filter_map(|(o, del)| match o {
            InlineObject::NoteRef { id, .. } => {
                let part = s.doc.parts.get(&id).unwrap_or_else(|| panic!("a note mark names story {id}, which the result lacks"));
                assert_eq!(part.kind, wordcraft_doc::PartKind::Footnote);
                Some((s.doc.plain_text(StoryRef::Part(id)).trim().to_string(), del))
            }
            _ => None,
        })
        .collect();
    notes.sort();
    let t = |t: &str, del| (t.to_string(), del);
    assert_eq!(notes, [t("new foot", false), t("original foot", true), t("revised foot", false)]);
}

#[test]
fn compare_resolves_the_revised_documents_styles_and_lists() {
    use wordcraft_doc::styles::{Style, StyleKind};
    // "Fancy" is bold in the original and italic in the revised document; "Plain" is the same in
    // both. Each has a list on "Item." under the same id: bullets in one, numbers in the other.
    let build = |chr: CharProps, list: &str| {
        let mut s = Session::new(Document::from_text("Styled.\nPlain.\nItem."));
        for (id, chr) in [("Fancy", chr), ("Plain", CharProps { strike: Some(true), ..Default::default() })] {
            Arc::make_mut(&mut s.doc.styles).upsert(Style { id: id.into(), name: id.into(), kind: StyleKind::Paragraph, chr, ..Default::default() });
        }
        s.doc.para_mut(StoryRef::Body, &Pos::body(0, 0).path).unwrap().props.style = Some("Fancy".into());
        s.doc.para_mut(StoryRef::Body, &Pos::body(1, 0).path).unwrap().props.style = Some("Plain".into());
        s.run("caret.set", &json!({"pos": Pos::body(2, 0)})).unwrap();
        s.run(list, &json!({})).unwrap();
        s.doc
    };
    let orig = build(CharProps { bold: Some(true), ..Default::default() }, "para.bullets");
    let rev = build(CharProps { italic: Some(true), ..Default::default() }, "para.numbering");
    let s = compare_files("styles", &orig, &rev);
    let para = |i: usize| s.doc.para(StoryRef::Body, &Pos::body(i, 0).path).unwrap();
    let style = |i: usize| s.doc.styles.get(para(i).props.style.as_deref().unwrap()).unwrap();
    assert_eq!(style(0).chr.italic, Some(true), "the revised paragraph shows the revised style");
    assert_ne!(style(0).chr.bold, Some(true));
    assert!(s.doc.styles.get("Fancy").is_some_and(|st| st.chr.bold == Some(true)), "the original's style stays the original's");
    assert_eq!(para(1).props.style.as_deref(), Some("Plain"), "a style both define alike stays one");
    let num = para(2).props.numbering.as_ref().expect("the list paragraph keeps its list");
    let level = s.doc.numbering.level(num.num, num.level).expect("its list is in the result");
    let orig_level = orig.numbering.level(1, 0).map(|l| l.text.clone());
    assert!(level.text.contains("%1"), "the revised list numbers: {:?} (the original's: {orig_level:?})", level.text);
}

#[test]
fn compare_never_bookmarks_a_name_twice() {
    // Both documents bookmark "x", in paragraphs the comparison deletes and inserts whole.
    let mark = |text: &str, para: usize| {
        let mut s = Session::new(Document::from_text(text));
        s.run("caret.set", &json!({"pos": Pos::body(para, 0)})).unwrap();
        s.run("insert.bookmark", &json!({"name": "x"})).unwrap();
        s.doc
    };
    let s = compare_files("bookmarks", &mark("A.\nGone para.\nB.", 1), &mark("A.\nB.\nFresh para.", 2));
    let starts: Vec<(String, bool)> = objects_in(&s.doc, StoryRef::Body)
        .into_iter()
        .filter_map(|(o, del)| match o {
            InlineObject::BookmarkStart { name } => Some((name, del)),
            _ => None,
        })
        .collect();
    assert_eq!(starts.len(), 2, "{starts:?}");
    assert!(starts.contains(&("x".to_string(), false)), "the revised bookmark keeps its name: {starts:?}");
    let names: BTreeSet<&String> = starts.iter().map(|(n, _)| n).collect();
    assert_eq!(names.len(), 2, "{starts:?}");
    // Saved and read back, the result still has both, under distinct names.
    let path = std::env::temp_dir().join(format!("wc-5571-bookmarks-out-{}.docx", std::process::id()));
    crate::io::save_path(&path, &s.doc).unwrap();
    let back = crate::io::open_path(&path);
    let _ = std::fs::remove_file(&path);
    let names: Vec<String> = back.unwrap().bookmarks().into_iter().map(|(n, _)| n).collect();
    assert_eq!(names.iter().collect::<BTreeSet<_>>().len(), names.len(), "{names:?}");
    assert_eq!(names.len(), 2, "{names:?}");
}

#[test]
fn compare_deleted_bookmark_names_avoid_text_box_bookmarks() {
    // Both documents have a text box (in the kept first paragraph) holding a live bookmark
    // `x_1`. The original bookmarks `x` in a paragraph the comparison deletes whole; the
    // revised one bookmarks `x` in its inserted paragraph. The deleted `x` needs a name of
    // its own: `x_1` is taken by the box, so it must become `x_2`, not collide with the box.
    let with_box = |text: &str| {
        let mut s = Session::new(Document::from_text(text));
        s.run("caret.set", &json!({"pos": Pos::body(0, 0)})).unwrap();
        let id = s.run("insert.textBox", &json!({"text": "Boxed "})).unwrap()["story"].as_u64().unwrap() as u32;
        let p = s.doc.para_mut(StoryRef::Part(id), &Pos::body(0, 0).path).unwrap();
        p.insert_object(0, InlineObject::BookmarkStart { name: "x_1".into() }, &CharProps::default()).unwrap();
        let at = p.len();
        p.insert_object(at, InlineObject::BookmarkEnd { name: "x_1".into() }, &CharProps::default()).unwrap();
        s
    };
    let mut orig = with_box("A.\nGone para.\nB.");
    orig.run("caret.set", &json!({"pos": Pos::body(1, 0)})).unwrap();
    orig.run("insert.bookmark", &json!({"name": "x"})).unwrap();
    let mut rev = with_box("A.\nB.\nFresh para.");
    rev.run("caret.set", &json!({"pos": Pos::body(2, 0)})).unwrap();
    rev.run("insert.bookmark", &json!({"name": "x"})).unwrap();
    let s = compare_files("bookmark-box", &orig.doc, &rev.doc);
    let starts: Vec<(String, bool)> = objects_in(&s.doc, StoryRef::Body)
        .into_iter()
        .filter_map(|(o, del)| match o {
            InlineObject::BookmarkStart { name } => Some((name, del)),
            _ => None,
        })
        .collect();
    assert!(starts.contains(&("x".to_string(), false)), "the revised bookmark keeps its name: {starts:?}");
    assert!(starts.contains(&("x_2".to_string(), true)), "the deleted bookmark is renamed past the box's x_1: {starts:?}");
    let mut all: Vec<String> = starts.iter().map(|(n, _)| n.clone()).collect();
    for (o, _) in objects_in(&s.doc, StoryRef::Body) {
        for id in o.text_boxes() {
            for (inner, del) in objects_in(&s.doc, StoryRef::Part(id)) {
                if let InlineObject::BookmarkStart { name } = inner {
                    assert!(!del, "the box's bookmark stays live");
                    all.push(name);
                }
            }
        }
    }
    assert!(all.contains(&"x_1".to_string()), "the box's bookmark is still there: {all:?}");
    assert_eq!(all.iter().collect::<BTreeSet<_>>().len(), all.len(), "no bookmark name is used twice: {all:?}");
}

#[test]
fn compare_gives_each_deleted_copy_of_a_name_its_own_name() {
    // The original carries `_GoBack` twice (Word writes one per document, and merged
    // files can carry two), both in paragraphs the comparison deletes whole; the revised
    // document's `_GoBack` stays live. Each deleted copy needs a name of its own: renamed
    // to one shared fresh name they would bookmark that name twice, and the writer keys
    // bookmark ids by name, so they would share an id as well.
    let bookmarked = |text: &str, paras: &[usize]| {
        let mut s = Session::new(Document::from_text(text));
        for &para in paras {
            let p = s.doc.para_mut(StoryRef::Body, &Pos::body(para, 0).path).unwrap();
            p.insert_object(0, InlineObject::BookmarkStart { name: "_GoBack".into() }, &CharProps::default()).unwrap();
            let at = p.len();
            p.insert_object(at, InlineObject::BookmarkEnd { name: "_GoBack".into() }, &CharProps::default()).unwrap();
        }
        s.doc
    };
    let s = compare_files("bookmarks-twice", &bookmarked("A.\nGone one.\nGone two.\nB.", &[1, 2]), &bookmarked("A.\nB.\nFresh para.", &[2]));
    let marks: Vec<(String, bool, bool)> = objects_in(&s.doc, StoryRef::Body)
        .into_iter()
        .filter_map(|(o, del)| match o {
            InlineObject::BookmarkStart { name } => Some((name, del, true)),
            InlineObject::BookmarkEnd { name } => Some((name, del, false)),
            _ => None,
        })
        .collect();
    let starts: Vec<&(String, bool, bool)> = marks.iter().filter(|m| m.2).collect();
    assert_eq!(starts.len(), 3, "{marks:?}");
    assert!(starts.contains(&&("_GoBack".to_string(), false, true)), "the revised bookmark keeps its name: {marks:?}");
    let names: BTreeSet<&String> = starts.iter().map(|(n, _, _)| n).collect();
    assert_eq!(names.len(), 3, "each copy has a name of its own: {marks:?}");
    // Every start's end carries the same name, so the pairs survive as pairs.
    let ends: BTreeSet<&String> = marks.iter().filter(|m| !m.2).map(|(n, _, _)| n).collect();
    assert_eq!(ends, names, "{marks:?}");
    // Saved and read back, the three names are still distinct.
    let path = std::env::temp_dir().join(format!("wc-5571-bookmarks-twice-out-{}.docx", std::process::id()));
    crate::io::save_path(&path, &s.doc).unwrap();
    let back = crate::io::open_path(&path);
    let _ = std::fs::remove_file(&path);
    let names: Vec<String> = back.unwrap().bookmarks().into_iter().map(|(n, _)| n).collect();
    assert_eq!(names.len(), 3, "{names:?}");
    assert_eq!(names.iter().collect::<BTreeSet<_>>().len(), 3, "{names:?}");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn compare_relinks_what_text_box_stories_refer_to() {
    // Both documents have a text box with the same text; inside it a picture both key
    // `image1.png`, with different bytes.
    let (a, b) = (png([200, 0, 0]), png([0, 200, 0]));
    let build = |bytes: &[u8]| {
        let mut s = Session::new(Document::from_text("Box here."));
        s.run("caret.set", &json!({"pos": Pos::body(0, "Box here.".len())})).unwrap();
        let id = s.run("insert.textBox", &json!({"text": "Inside "})).unwrap()["story"].as_u64().unwrap() as u32;
        let media = s.doc.add_media(bytes.to_vec(), "png");
        let img = InlineObject::Image { media, w: 10.0, h: 10.0, alt: String::new(), float: Default::default(), crop: [0.0; 4], ole: None };
        let p = s.doc.para_mut(StoryRef::Part(id), &Pos::body(0, 0).path).unwrap();
        let at = p.len();
        p.insert_object(at, img, &CharProps::default()).unwrap();
        s.doc
    };
    let s = compare_files("boxes", &build(&a), &build(&b));
    let mut shown = Vec::new();
    for (o, del) in objects_in(&s.doc, StoryRef::Body) {
        for id in o.text_boxes() {
            for (inner, _) in objects_in(&s.doc, StoryRef::Part(id)) {
                if let InlineObject::Image { media, .. } = inner {
                    let bytes = s.doc.media.get(&media).unwrap_or_else(|| panic!("a box's picture shows {media}, which the result lacks"));
                    shown.push((bytes.to_vec(), del));
                }
            }
        }
    }
    assert!(shown.contains(&(b.clone(), false)), "the revised box shows its own picture: {shown:?}");
    assert!(shown.contains(&(a.clone(), true)), "the original's picture is kept, marked deleted: {shown:?}");
    assert!(!shown.contains(&(a, false)), "the original's picture never shows as live: {shown:?}");
}

#[test]
fn compare_of_a_document_with_itself_changes_nothing() {
    // The adoption shares what both versions define alike: lists, styles, pictures, notes and
    // comments that are the same compare as the same.
    let mut o = Session::new(Document::from_text("Alpha one.\nBeta two.\nItem."));
    comment_on(&mut o, "Alpha", "a note");
    let len = o.doc.para(StoryRef::Body, &Pos::body(1, 0).path).unwrap().len();
    o.run("caret.set", &json!({"pos": Pos::body(1, len)})).unwrap();
    o.run("references.footnote", &json!({"text": "foot"})).unwrap();
    o.run("caret.set", &json!({"pos": Pos::body(2, 0)})).unwrap();
    o.run("para.numbering", &json!({})).unwrap();
    let s = compare_files("self", &o.doc, &o.doc);
    for path in s.doc.para_paths(StoryRef::Body) {
        let p = s.doc.para(StoryRef::Body, &path).unwrap();
        assert!(p.run_ranges().all(|(_, c)| c.ins.is_none() && c.del.is_none() && c.fmt_change.is_none()), "{:?} is marked", p.text);
        assert!(p.props.fmt_change.is_none(), "{:?} has a formatting change", p.text);
    }
    assert_eq!(s.doc.comments.len(), 1, "one comment, not the original's left over beside the revised one's");
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
