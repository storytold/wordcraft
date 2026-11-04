//! Regression tests for #3749 Stage 0: formatting-preserving compare.
//!
//! `word_diff` now carries source byte ranges through tokenization, so every output segment takes
//! its `CharProps` from the run it came from — the original run for Same/Del, the revised run for
//! Ins — instead of stamping the original paragraph's first run on everything. It also keeps the
//! original paragraph mark, and the original `ParaProps` when the revised side has none.
//!
//! The ins/del segmentation for text-only paragraphs is unchanged: the goldens below were captured
//! from the pre-fix binary (`review.compare` over a plain original and a plain revised text, word
//! and character level) and must keep matching.

use super::*;

/// A run signature: its text, and whether it is marked inserted / deleted.
fn run_sig(p: &Paragraph) -> Vec<(String, bool, bool)> {
    p.run_ranges().map(|(r, c)| (p.text[r].to_string(), c.ins.is_some(), c.del.is_some())).collect()
}

/// The first run whose text contains `needle`.
fn run_with<'a>(p: &'a Paragraph, needle: &str) -> (&'a str, &'a CharProps) {
    p.run_ranges()
        .find(|(r, _)| p.text[r.clone()].contains(needle))
        .map(|(r, c)| (&p.text[r], c))
        .unwrap_or_else(|| panic!("no run contains {needle:?} in {:?}", p.text))
}

// --- mixed character formatting -----------------------------------------------------------------

#[test]
fn mixed_formatting_is_preserved_per_range() {
    let mut s = Session::new(Document::from_text("alpha beta gamma"));
    s.run("select.text", &json!({"text": "alpha"})).unwrap();
    s.run("format.bold", &json!({})).unwrap();
    s.run("select.text", &json!({"text": "gamma"})).unwrap();
    s.run("format.italic", &json!({})).unwrap();
    s.run("review.compare", &json!({"text": "alpha delta gamma"})).unwrap();
    let p = s.doc.body[0].as_para().unwrap();
    // The deleted text stays in the tracked paragraph; the visible result is what Accept shows.
    assert_eq!(p.plain_text(), "alpha beta delta gamma");
    assert_eq!(p.final_text(), "alpha delta gamma");
    let (_, alpha) = run_with(p, "alpha");
    assert_eq!(alpha.bold, Some(true), "unchanged `alpha` keeps the original's bold");
    assert_ne!(alpha.italic, Some(true));
    let (_, gamma) = run_with(p, "gamma");
    assert_eq!(gamma.italic, Some(true), "unchanged `gamma` keeps the original's italic");
    assert_ne!(gamma.bold, Some(true));
    let (_, delta) = run_with(p, "delta");
    assert!(delta.ins.is_some(), "inserted `delta` is marked inserted");
    assert_ne!(delta.bold, Some(true), "inserted text does not inherit the original's bold");
    assert_ne!(delta.italic, Some(true), "inserted text does not inherit the original's italic");
    let (_, beta) = run_with(p, "beta");
    assert!(beta.del.is_some(), "removed `beta` is marked deleted");
    assert_ne!(beta.bold, Some(true));
}

// --- byte-range props lookup --------------------------------------------------------------------

#[test]
fn change_point_on_a_multibyte_char_takes_its_own_run() {
    // The change point is the second character, `ä` (2 bytes). The byte-range lookup must land on
    // `ä`'s own run; a character count used as a byte offset would land inside the preceding `é`
    // and stamp its bold on the deleted `ä`.
    let mut s = Session::new(Document::from_text("éä"));
    s.run("select.text", &json!({"text": "é"})).unwrap();
    s.run("format.bold", &json!({})).unwrap();
    s.run("select.text", &json!({"text": "ä"})).unwrap();
    s.run("format.italic", &json!({})).unwrap();
    s.run("review.compare", &json!({"text": "éö", "level": "character"})).unwrap();
    let p = s.doc.body[0].as_para().unwrap();
    assert_eq!(p.plain_text(), "éäö", "deleted `ä` and inserted `ö`");
    let (_, ae) = run_with(p, "ä");
    assert!(ae.del.is_some(), "the changed multi-byte char is deleted");
    assert_eq!(ae.italic, Some(true), "it takes its own run");
    assert_ne!(ae.bold, Some(true), "and not the preceding multi-byte run's props");
}

#[test]
fn empty_run_at_a_token_start_does_not_shadow_its_props() {
    let mut s = Session::new(Document::from_text("alpha beta gamma"));
    s.run("select.text", &json!({"text": "beta"})).unwrap();
    s.run("format.italic", &json!({})).unwrap();
    {
        let mut p = s.doc.body[0].as_para().unwrap().clone();
        let at = p.text.find("beta").unwrap();
        let mut idx = 0usize;
        let mut start = 0usize;
        for (i, r) in p.runs.iter().enumerate() {
            if start == at {
                idx = i;
                break;
            }
            start += r.len;
        }
        // An empty run with different props exactly at "beta"'s start.
        p.runs.insert(idx, wordcraft_doc::para::Run { len: 0, props: CharProps { bold: Some(true), ..CharProps::default() } });
        s.doc.body[0] = std::sync::Arc::new(Block::Para(p));
    }
    s.run("review.compare", &json!({"text": "alpha delta gamma"})).unwrap();
    let p = s.doc.body[0].as_para().unwrap();
    let (_, beta) = run_with(p, "beta");
    assert!(beta.del.is_some());
    assert_eq!(beta.italic, Some(true), "the token takes the run after the empty one");
    assert_ne!(beta.bold, Some(true), "an empty run at the token's start must not shadow the next run's props");
}

// --- hyperlinks ---------------------------------------------------------------------------------

#[test]
fn middle_run_hyperlink_survives_and_reject_restores_it() {
    let mut s = Session::new(Document::from_text("click here now"));
    s.run("select.text", &json!({"text": "here"})).unwrap();
    s.run("insert.link", &json!({"url": "https://example.com"})).unwrap();
    s.run("review.compare", &json!({"text": "click there now"})).unwrap();
    let p = s.doc.body[0].as_para().unwrap();
    let (_, here) = run_with(p, "here");
    assert!(here.del.is_some(), "the removed word is deleted");
    assert_eq!(here.link.as_deref(), Some("https://example.com"), "the deleted run keeps its hyperlink");
    let (_, there) = run_with(p, "there");
    assert!(there.ins.is_some());
    assert_eq!(there.link, None, "inserted text takes the revised run's formatting (no link)");
    s.run("review.rejectAll", &json!({})).unwrap();
    assert_eq!(s.doc.plain_text(StoryRef::Body), "click here now");
    let p = s.doc.body[0].as_para().unwrap();
    let (_, here) = run_with(p, "here");
    assert_eq!(here.link.as_deref(), Some("https://example.com"), "Reject restores the hyperlink at token level");
}

#[test]
fn first_run_hyperlink_does_not_leak_onto_inserted_text() {
    let mut s = Session::new(Document::from_text("here click now"));
    s.run("select.text", &json!({"text": "here"})).unwrap();
    s.run("insert.link", &json!({"url": "https://example.com"})).unwrap();
    s.run("review.compare", &json!({"text": "here there click now"})).unwrap();
    let p = s.doc.body[0].as_para().unwrap();
    let (_, here) = run_with(p, "here");
    assert_eq!(here.link.as_deref(), Some("https://example.com"), "the unchanged first run keeps its link");
    let (_, there) = run_with(p, "there");
    assert!(there.ins.is_some());
    assert_eq!(there.link, None, "the first run's link must not leak onto inserted text");
}

// --- reject restores formatting -----------------------------------------------------------------

#[test]
fn deleted_segment_keeps_formatting_so_reject_restores_it() {
    let mut s = Session::new(Document::from_text("alpha beta gamma"));
    s.run("select.text", &json!({"text": "alpha"})).unwrap();
    s.run("format.bold", &json!({})).unwrap();
    s.run("select.text", &json!({"text": "beta"})).unwrap();
    s.run("format.italic", &json!({})).unwrap();
    s.run("review.compare", &json!({"text": "alpha delta gamma"})).unwrap();
    s.run("review.rejectAll", &json!({})).unwrap();
    assert_eq!(s.doc.plain_text(StoryRef::Body), "alpha beta gamma");
    let p = s.doc.body[0].as_para().unwrap();
    let (_, alpha) = run_with(p, "alpha");
    assert_eq!(alpha.bold, Some(true));
    let (_, beta) = run_with(p, "beta");
    assert_eq!(beta.italic, Some(true), "the deleted token kept its italic, so Reject restores it at token level");
}

// --- text-only segmentation identity (goldens from the pre-fix binary) --------------------------

#[test]
fn text_only_segmentation_matches_pre_fix_golden() {
    // (original, revised, level, expected (text, ins, del) runs)
    let cases: &[(&str, &str, &str, &[(&str, bool, bool)])] = &[
        (
            "The dog sat.",
            "The cat sat.",
            "word",
            &[("The ", false, false), ("dog ", false, true), ("cat ", true, false), ("sat.", false, false)],
        ),
        ("a cat ran", "a dog ran", "word", &[("a ", false, false), ("cat ", false, true), ("dog ", true, false), ("ran", false, false)]),
        ("colour", "color", "word", &[("colour", false, true), ("color", true, false)]),
        (
            "zawarta w dniu 01.09.2026 pomiędzy:",
            "zawarta w dniu 19.10.2026 pomiędzy:",
            "word",
            &[
                ("zawarta w dniu ", false, false),
                ("01.09.2026 ", false, true),
                ("19.10.2026 ", true, false),
                ("pomiędzy:", false, false),
            ],
        ),
        (
            "The dog sat.",
            "The cat sat.",
            "character",
            &[("The ", false, false), ("dog", false, true), ("cat", true, false), (" sat.", false, false)],
        ),
        (
            "a cat ran",
            "a dog ran",
            "character",
            &[("a ", false, false), ("cat", false, true), ("dog", true, false), (" ran", false, false)],
        ),
        ("colour", "color", "character", &[("colo", false, false), ("u", false, true), ("r", false, false)]),
        (
            "zawarta w dniu 01.09.2026 pomiędzy:",
            "zawarta w dniu 19.10.2026 pomiędzy:",
            "character",
            &[("zawarta w dniu ", false, false), ("01.09", false, true), ("19.10", true, false), (".2026 pomiędzy:", false, false)],
        ),
    ];
    for (orig, rev, level, want) in cases {
        let mut s = Session::new(Document::from_text(orig));
        s.run("review.compare", &json!({"text": rev, "level": level})).unwrap();
        let p = s.doc.body[0].as_para().unwrap();
        let want: Vec<(String, bool, bool)> = want.iter().map(|(t, i, d)| ((*t).to_string(), *i, *d)).collect();
        assert_eq!(run_sig(p), want, "{orig:?} -> {rev:?} [{level}]");
    }
}

// --- paragraph mark and paragraph properties ----------------------------------------------------

#[test]
fn paragraph_mark_is_preserved() {
    let mut s = Session::new(Document::from_text("alpha beta gamma"));
    {
        let mut p = s.doc.body[0].as_para().unwrap().clone();
        p.mark.bold = Some(true);
        s.doc.body[0] = std::sync::Arc::new(Block::Para(p));
    }
    s.run("review.compare", &json!({"text": "alpha delta gamma"})).unwrap();
    let p = s.doc.body[0].as_para().unwrap();
    assert_eq!(p.mark.bold, Some(true), "the rebuilt paragraph keeps the original paragraph mark");
}

#[test]
fn paragraph_props_kept_when_revised_is_plain() {
    let mut s = Session::new(Document::from_text("alpha beta gamma"));
    s.run("para.style", &json!({"style": "Heading1"})).unwrap();
    s.run("para.alignCenter", &json!({})).unwrap();
    s.run("review.compare", &json!({"text": "alpha delta gamma"})).unwrap();
    let p = s.doc.body[0].as_para().unwrap();
    assert_eq!(p.props.style.as_deref(), Some("Heading1"), "the original style survives a plain revised side");
    assert!(p.props.align.is_some(), "the original alignment survives a plain revised side");
}

// --- object pass-through (not flattened away by the rebuild) -------------------------------------

/// A session whose single paragraph is `alpha beta gamma` with a picture after "beta ", already
/// compared against `alpha delta gamma`: the picture's anchor sits in a deleted run.
fn compared_with_picture() -> Session {
    let mut s = Session::new(Document::from_text("alpha beta gamma"));
    s.run("caret.set", &json!({"pos": Pos::body(0, "alpha beta ".len())})).unwrap();
    let png = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAAC0lEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==";
    s.run("insert.picture", &json!({"data": png})).unwrap();
    assert_eq!(s.doc.body[0].as_para().unwrap().objects.len(), 1, "the picture was inserted");
    s.run("review.compare", &json!({"text": "alpha delta gamma"})).unwrap();
    s
}

#[test]
fn picture_anchor_survives_a_changed_paragraph() {
    let s = compared_with_picture();
    let p = s.doc.body[0].as_para().unwrap();
    assert_eq!(p.objects.len(), 1, "the changed paragraph's picture anchor survives the rebuild");
    let off = p.object_offsets().first().copied().expect("the picture anchor is in the text");
    assert!(p.props_of_char(off).del.is_some(), "the picture anchor sits in a deleted run");
}

#[test]
fn accepting_removes_the_deleted_picture_and_rejecting_restores_it() {
    let mut accepted = compared_with_picture();
    accepted.run("review.acceptAll", &json!({})).unwrap();
    assert_eq!(accepted.doc.body[0].as_para().unwrap().objects.len(), 0, "accepting the deletion removes the picture");

    let mut rejected = compared_with_picture();
    rejected.run("review.rejectAll", &json!({})).unwrap();
    assert_eq!(rejected.doc.body[0].as_para().unwrap().objects.len(), 1, "rejecting the deletion restores the picture");
}
