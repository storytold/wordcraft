use super::*;
use harper_core::Span;

fn lint(start: usize, end: usize, kind: LintKind, priority: u8, suggestions: Vec<Suggestion>) -> Lint {
    Lint { span: Span::new(start, end), lint_kind: kind, suggestions, message: "Use `have` here.".into(), priority }
}

fn replace(s: &str) -> Suggestion {
    Suggestion::ReplaceWith(s.chars().collect())
}

/// Harper's issues in `text`, waiting for the background thread when they aren't ready yet.
fn checked(text: &str, lang: ProofLang) -> Vec<Issue> {
    if let Some(found) = issues(text, lang) {
        return found;
    }
    assert!(wait(Duration::from_secs(300)), "Harper didn't finish");
    issues(text, lang).unwrap()
}

/// [`crate::check_text_with`] with Harper on, once Harper has checked the text.
fn with_harper(text: &str, spans: &[(std::ops::Range<usize>, ProofLang)]) -> Vec<Issue> {
    let check = || crate::check_text_with(text, spans, &crate::Options::default(), true, false, true);
    check();
    assert!(wait(Duration::from_secs(300)), "Harper didn't finish");
    check()
}

#[test]
fn lint_spans_in_characters_become_byte_ranges() {
    let text = "😀 日本 I could of done it.";
    let start = text.chars().position(|c| c == 'c').unwrap();
    let found = to_issues(text, vec![lint(start, start + 8, LintKind::WordChoice, 31, vec![replace("could have")])], ProofLang::En);
    assert_eq!(found.len(), 1);
    assert_eq!(&text[found[0].start..found[0].end], "could of");
    assert_eq!(found[0].suggestions, ["could have"]);
    assert_eq!(found[0].kind, IssueKind::Grammar);
    assert_eq!(found[0].message, "Use \"have\" here.");
    // A span past the text, or an empty one, is dropped rather than cut into.
    assert!(
        to_issues(text, vec![lint(30, 40, LintKind::Grammar, 0, Vec::new()), lint(3, 3, LintKind::Grammar, 0, Vec::new())], ProofLang::En).is_empty()
    );
}

#[test]
fn spelling_placeholders_and_overlaps_are_left_out() {
    let text = "ab cd\u{1} ef gh";
    let lints = vec![
        // Spelling is WordCraft's dictionaries' to mark, and style suggestions aren't errors.
        lint(0, 2, LintKind::Spelling, 0, Vec::new()),
        lint(0, 2, LintKind::Style, 0, Vec::new()),
        lint(9, 12, LintKind::Readability, 0, Vec::new()),
        lint(10, 12, LintKind::Enhancement, 0, Vec::new()),
        // An inline object or tracked deletion.
        lint(3, 6, LintKind::Grammar, 0, Vec::new()),
        // Two lints on one word: the more important (lower priority) one stays.
        lint(7, 9, LintKind::Typo, 63, vec![replace("if")]),
        lint(7, 12, LintKind::Grammar, 31, vec![replace("ef, gh")]),
    ];
    let found = to_issues(text, lints, ProofLang::EnGb);
    assert_eq!(found.iter().map(|i| &text[i.start..i.end]).collect::<Vec<_>>(), ["ef gh"]);
    assert_eq!(found[0].lang, ProofLang::EnGb);
}

#[test]
fn suggestions_are_the_text_that_goes_in() {
    assert_eq!(suggestion("its", &replace("it's")).as_deref(), Some("it's"));
    assert_eq!(suggestion("However", &Suggestion::InsertAfter(vec![','])).as_deref(), Some("However,"));
    assert_eq!(suggestion("very", &Suggestion::Remove), None);
    let text = "to to";
    let found =
        to_issues(text, vec![lint(0, 5, LintKind::Typo, 0, vec![replace("to"), replace("to"), replace("to do"), Suggestion::Remove])], ProofLang::En);
    assert_eq!(found[0].suggestions, ["to", "to do"]);
}

#[test]
fn the_newest_layout_pass_goes_first_and_in_order() {
    let mut st = State::default();
    st.request("one", ProofLang::En);
    st.request("two", ProofLang::En);
    // Asked again while waiting: still one request.
    st.request("one", ProofLang::En);
    st.pass += 1;
    st.request("edited", ProofLang::En);
    st.request("edited too", ProofLang::En);
    // The same text in another language is another request.
    st.request("one", ProofLang::EnGb);
    let order: Vec<(String, ProofLang)> = std::iter::from_fn(|| st.take().map(|j| (j.text, j.lang))).collect();
    let order: Vec<(&str, ProofLang)> = order.iter().map(|(t, l)| (t.as_str(), *l)).collect();
    assert_eq!(
        order,
        [("edited", ProofLang::En), ("edited too", ProofLang::En), ("one", ProofLang::EnGb), ("one", ProofLang::En), ("two", ProofLang::En)]
    );
    assert_eq!(st.waiting_bytes, 0);
    assert!(st.queued.is_empty());
}

#[test]
fn waiting_text_is_capped() {
    let mut st = State::default();
    let big = "a".repeat(MAX_WAITING_BYTES / 2 + 1);
    st.request(&big, ProofLang::En);
    st.request(&format!("b{big}"), ProofLang::En);
    // The older one was dropped to stay under the cap.
    assert_eq!(st.waiting.len(), 1);
    assert!(st.waiting_bytes <= MAX_WAITING_BYTES);
    assert!(!st.queued.contains_key(&big));
}

#[test]
fn a_stored_result_changes_the_stamp() {
    let mut st = State::default();
    assert_eq!(st.stamp("text"), 0);
    assert_eq!(st.found("text", ProofLang::En), None);
    st.put("text".into(), ProofLang::En, Vec::new());
    let first = st.stamp("text");
    assert_ne!(first, 0);
    assert_eq!(st.found("text", ProofLang::En), Some(Vec::new()));
    assert_eq!(st.found("text", ProofLang::EnGb), None);
    st.put("text".into(), ProofLang::EnGb, Vec::new());
    assert!(st.stamp("text") > first);
}

#[test]
fn results_used_longest_ago_are_forgotten_first() {
    let mut st = State::default();
    for k in 0..MAX_RESULTS {
        st.put(format!("p{k}"), ProofLang::En, Vec::new());
    }
    // Layout still shows the first paragraph.
    st.stamp("p0");
    st.put("new".into(), ProofLang::En, Vec::new());
    assert!(st.results.len() <= MAX_RESULTS / 2 + 1, "{}", st.results.len());
    assert_ne!(st.stamp("p0"), 0);
    assert_ne!(st.stamp("new"), 0);
    assert_eq!(st.stamp("p1"), 0);
}

#[test]
fn only_english_paragraphs_of_checkable_length_are_queued() {
    assert_eq!(issues("Ich habe es gemacht.", ProofLang::De), Some(Vec::new()));
    assert_eq!(issues("   ", ProofLang::En), Some(Vec::new()));
    assert_eq!(issues(&"word ".repeat(MAX_TEXT_BYTES / 5 + 1), ProofLang::En), Some(Vec::new()));
}

#[test]
fn harper_marks_grammar_the_built_in_rules_miss() {
    let text = "I could of done it yesterday.";
    let found = checked(text, ProofLang::En);
    let could = found.iter().find(|i| &text[i.start..i.end] == "could of").unwrap_or_else(|| panic!("{found:?}"));
    assert_eq!(could.kind, IssueKind::Grammar);
    assert!(could.suggestions.iter().any(|s| s == "could have"), "{could:?}");
    assert!(!could.message.contains('`'), "{}", could.message);
    // British English is checked too, and says so.
    let gb = checked("We could of gone home.", ProofLang::EnGb);
    assert!(gb.iter().any(|i| i.lang == ProofLang::EnGb), "{gb:?}");
    // A style choice, like leaving out the Oxford comma, isn't marked.
    let style = checked("We bought apples, pears and plums at the market.", ProofLang::En);
    assert!(style.is_empty(), "{style:?}");
}

#[test]
fn harper_adds_to_the_built_in_checks_without_marking_twice() {
    let text = "This is an test of the the rules. its a nice day, and we could of gone.";
    let without = crate::check_text_with(text, &[], &crate::Options::default(), true, false, false);
    let with = with_harper(text, &[]);
    let marked = |v: &[Issue]| v.iter().map(|i| text[i.start..i.end].to_string()).collect::<Vec<_>>();
    // Harper's spell check and its copies of the built-in rules are off: every built-in mark is
    // still there once, and no two marks overlap.
    for i in &without {
        assert_eq!(with.iter().filter(|o| o.start < i.end && i.start < o.end).count(), 1, "{:?} in {:?}", &text[i.start..i.end], marked(&with));
    }
    assert!(with.windows(2).all(|w| w[0].end <= w[1].start), "{:?}", marked(&with));
    assert!(marked(&with).iter().any(|m| m == "could of"), "{:?}", marked(&with));
    assert!(with.len() > without.len());
}

#[test]
fn harper_checks_only_the_english_parts() {
    let text = "Mówię, że could of. I could of done it.";
    let polish = 0..text.find('.').unwrap();
    let found = with_harper(text, &[(polish.clone(), ProofLang::Pl)]);
    let harper: Vec<&Issue> = found.iter().filter(|i| &text[i.start..i.end] == "could of").collect();
    assert_eq!(harper.len(), 1, "{found:?}");
    assert!(harper[0].start >= polish.end);
}

#[test]
fn harper_leaves_words_in_the_user_dictionary_alone() {
    let text = "Its a fine morning for a walk outside.";
    assert!(with_harper(text, &[]).iter().any(|i| &text[i.start..i.end] == "Its"));
    crate::add_word("its");
    let after = with_harper(text, &[]);
    crate::remove_word("its");
    assert!(!after.iter().any(|i| &text[i.start..i.end] == "Its"), "{after:?}");
}
