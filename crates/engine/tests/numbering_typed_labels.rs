use serde_json::{Value, json};
use wordcraft_doc::Pos;
use wordcraft_engine::Session;

/// Dispatch an engine command and report its ID if it fails.
fn run(s: &mut Session, id: &str, params: Value) {
    s.run(id, &params).unwrap_or_else(|e| panic!("{id}: {e}"));
}

fn doc(text: &str) -> Session {
    Session::new(wordcraft_doc::Document::from_text(text))
}

fn body(s: &Session) -> Vec<String> {
    s.doc.body.iter().filter_map(|b| b.as_para()).map(|p| p.text.clone()).collect()
}

/// Select everything and apply a list of `kind` (`numbered`, `numberedParen`, `lowerLetterDot`,
/// `bullet`).
fn number(s: &mut Session, kind: &str) {
    run(s, "select.all", json!({}));
    let params = if kind == "numbered" { json!({}) } else { json!({"kind": kind}) };
    run(s, "para.numbering", params);
}

/// The labels the page shows, in document order.
fn labels(s: &Session) -> Vec<String> {
    let mut c = wordcraft_doc::numbering::Counters::default();
    s.doc
        .para_paths(wordcraft_doc::StoryRef::Body)
        .into_iter()
        .filter_map(|p| {
            s.doc.para(wordcraft_doc::StoryRef::Body, &p).and_then(|x| x.props.numbering).and_then(|n| c.next_label(&s.doc.numbering, n.num, n.level))
        })
        .map(|(l, _)| l)
        .collect()
}

/// #6017: Numbering over paragraphs with typed numbers in front of them removes the typed ones,
/// as Word does, so the list reads `1. alpha` rather than `1. 1. alpha`.
#[test]
fn numbering_removes_the_typed_numbers_it_repeats() {
    let mut s = doc("1. alpha\n2. beta\n3. gamma\n4. delta\n5. epsilon");
    number(&mut s, "numbered");
    assert_eq!(body(&s), ["alpha", "beta", "gamma", "delta", "epsilon"]);
    assert_eq!(labels(&s), ["1.", "2.", "3.", "4.", "5."]);
}

/// The other spellings Word accepts, and a paragraph that already starts with white space.
#[test]
fn numbering_removes_typed_paren_dot_and_letter_labels() {
    for (text, kind, after) in [
        ("1) a\n2) b", "numberedParen", vec!["a", "b"]),
        ("a. a\nb. b", "lowerLetterDot", vec!["a", "b"]),
        ("a) a\nb) b", "lowerLetter", vec!["a", "b"]),
        ("  1. a", "numbered", vec!["a"]),
        ("1.\ta", "numbered", vec!["a"]),
    ] {
        let mut s = doc(text);
        number(&mut s, kind);
        assert_eq!(body(&s), after, "{text:?}");
    }
}

/// Word keeps a typed number the list does not give that paragraph: a jump, an out-of-range value
/// or a spelling the list does not use stays as text, so the author's own numbering is never
/// silently lost.
///
/// Word's own rule for a run that does not match is fuzzier — it also removes some labels one past
/// the number it gives (typing `2.` in a two-paragraph run loses the second paragraph's `3.`), and
/// leaves the first one — so this removes exactly the labels the list repeats and keeps every
/// other typed number. No typed labeling is lost that way, and the reported duplication is gone.
#[test]
fn numbering_keeps_a_typed_number_the_list_does_not_give() {
    let mut jump = doc("1. a\n5. b");
    number(&mut jump, "numbered");
    assert_eq!(body(&jump), ["a", "5. b"]);

    let mut later = doc("2. a\n3. b");
    number(&mut later, "numbered");
    assert_eq!(body(&later), ["2. a", "3. b"]);

    let mut alone = doc("2. a");
    number(&mut alone, "numbered");
    assert_eq!(body(&alone), ["2. a"]);

    // A version number ("1.2.3. Foo") is one word, not a label.
    let mut version = doc("1.2.3. Foo");
    number(&mut version, "numbered");
    assert_eq!(body(&version), ["1.2.3. Foo"]);
}

/// Bullets keep a typed label the way Word does: `-`, `+` and the numeric spellings stay.
#[test]
fn bullets_keep_typed_labels() {
    for text in ["- a\n- b", "1. a\n2. b", "a. a\nb. b"] {
        let mut s = doc(text);
        run(&mut s, "select.all", json!({}));
        run(&mut s, "para.bullets", json!({}));
        assert_eq!(body(&s), text.split('\n').collect::<Vec<_>>(), "{text:?}");
    }
}

/// A paragraph holding only its typed label stays as it is, as does one with no space after the
/// label.
#[test]
fn numbering_leaves_paragraphs_without_a_usable_label_alone() {
    let mut lone = doc("1.\nalpha");
    number(&mut lone, "numbered");
    assert_eq!(body(&lone), ["1.", "alpha"]);

    let mut joined = doc("1.Foo\nbar");
    number(&mut joined, "numbered");
    assert_eq!(body(&joined), ["1.Foo", "bar"], "no space after the dot: not a label");
}

/// One undo step takes the whole thing back, text and numbering together.
#[test]
fn undo_after_numbering_puts_the_typed_numbers_back() {
    let mut s = doc("1. alpha\n2. beta");
    let before = s.doc.clone();
    number(&mut s, "numbered");
    assert_eq!(body(&s), ["alpha", "beta"]);
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(s.doc, before);
}

/// The removal is an edit like any other for Track Changes: the label is marked deleted, not
/// removed, so Reject All brings it back.
#[test]
fn numbering_strips_a_typed_label_as_a_tracked_deletion() {
    let mut s = doc("1. alpha");
    run(&mut s, "review.trackChanges", json!({"value": true}));
    number(&mut s, "numbered");
    let p = s.doc.para_at(&Pos::body(0, 0)).unwrap();
    assert_eq!(p.text, "1. alpha");
    assert!(p.run_ranges().any(|(r, c)| r.start == 0 && r.end == 3 && c.del.is_some()), "{:?}", p.runs);
}
