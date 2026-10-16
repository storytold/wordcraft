//! Legal spelling supplement and confusion-pair proofing.

use serde_json::json;
use wordcraft_doc::{CharProps, Document, InlineObject, Paragraph, Pos, StoryRef, para_block};
use wordcraft_engine::Session;
use wordcraft_proof::{ModuleEn, Options, ProofLang, check_spelling_with_module, check_text_with_module, is_correct_with_module};

use crate::install;
use crate::proofing::{confusion_findings, confusion_issues, legal_words};
use crate::tests::{law, run};

#[test]
fn legal_words_are_sorted_and_nonempty() {
    let words = legal_words();
    assert!(words.len() > 40);
    assert!(words.windows(2).all(|p| p[0] < p[1]), "sorted, no duplicates");
}

#[test]
fn legal_terms_spell_check_depends_on_lawcraft() {
    let opts = Options::default();
    assert!(!is_correct_with_module("rsmo", &opts, wordcraft_proof::ModuleEn::default()));
    let mut s = law();
    assert!(is_correct_with_module("rsmo", &opts, s.module_en));
    assert!(is_correct_with_module("tortfeasor", &opts, s.module_en));
    s.module_en = wordcraft_proof::ModuleEn::default();
    assert!(!is_correct_with_module("rsmo", &opts, s.module_en));
    assert_eq!(install(&mut s), crate::specs().len());
    assert!(is_correct_with_module("rsmo", &opts, s.module_en));
}

/// Reporter and court abbreviations (`S.W.3d`, `F.4th`, `Mo. App.`) reach proofing through the
/// real spelling path: the tokenizer splits them at their periods, and the resulting pieces are
/// recognised — single letters and digit-bearing tokens by the defaults, the rest by the
/// dictionary — so none is flagged. `RSMo`, a single token the base dictionary does not have, is
/// the term the LawCraft list itself supplies: quiet with the module on, flagged without it.
#[test]
fn reporter_and_court_abbreviations_spell_clean() {
    let opts = Options::default();
    let s = law();
    for text in ["S.W.3d 679", "F.4th 1184", "Mo. App. E.D. 2020"] {
        let issues = check_spelling_with_module(text, &opts, s.module_en);
        assert!(issues.is_empty(), "LawCraft should not flag `{text}`: {issues:?}");
    }
    assert!(!is_correct_with_module("rsmo", &opts, ModuleEn::default()), "RSMo is not plain English");
    assert!(is_correct_with_module("rsmo", &opts, s.module_en), "the legal list supplies RSMo");
}

#[test]
fn law_proofing_does_not_leak_between_sessions() {
    let opts = Options::default();
    let enabled = law();
    let mut disabled = Session::new(Document::new());
    assert!(!disabled.module_enabled(crate::MODULE));
    assert_eq!(install(&mut disabled), 0);
    assert!(is_correct_with_module("rsmo", &opts, enabled.module_en));
    assert!(!is_correct_with_module("rsmo", &opts, disabled.module_en));
    let text = "The statue of limitations has run.";
    let on = check_text_with_module(text, &[(0..text.len(), ProofLang::En)], &opts, true, enabled.module_en);
    assert!(on.iter().any(|i| i.message.contains("statute")));
    let off = check_text_with_module(text, &[(0..text.len(), ProofLang::En)], &opts, true, disabled.module_en);
    assert!(off.is_empty());
}

#[test]
fn law_words_list_command() {
    let mut s = law();
    let list = run(&mut s, "law.words.list", json!({}));
    let words: Vec<&str> = list.as_array().expect("array").iter().filter_map(|v| v.as_str()).collect();
    assert!(words.contains(&"demurrer"));
    assert!(words.windows(2).all(|p| p[0] < p[1]));
}

/// Inline field results expand in [`Paragraph::plain_text`] but not in [`Paragraph::proof_text`],
/// so confusion findings must use proof text or highlights drift.
#[test]
fn confusion_offsets_survive_inline_fields_and_markers() {
    let mut doc = Document::new();
    let mut p = Paragraph::with_text("The statue of limitations has run.", CharProps::default());
    p.insert_object(
        0,
        InlineObject::Field { instr: "PAGE".into(), result: "99999".into(), locked: false, code: false, dirty: false, form: String::new() },
        &CharProps::default(),
    )
    .unwrap();
    let statue_off = p.text.find("statue").expect("statue in storage");
    let statue_end = statue_off + "statue".len();
    doc.body = vec![para_block(p)];

    let findings = confusion_findings(&doc);
    assert_eq!(findings.len(), 1, "{findings:?}");
    let start = findings[0].start.clone().expect("start");
    let end = findings[0].end.clone().expect("end");
    assert_eq!((start.story, start.off), (StoryRef::Body, statue_off));
    assert_eq!((end.story, end.off), (StoryRef::Body, statue_end));
    assert_eq!(doc.copy_range(&start, &end).plain_text(), "statue");

    let mut s = law();
    s.doc.body = doc.body.clone();
    let r = run(&mut s, "law.proof.confusions", json!({"store": false}));
    let f = r["findings"].as_array().expect("findings").first().expect("one finding");
    let cmd_start = f["start"]["off"].as_u64().expect("start off") as usize;
    let cmd_end = f["end"]["off"].as_u64().expect("end off") as usize;
    assert_eq!((cmd_start, cmd_end), (statue_off, statue_end));
    let start = Pos::body(0, cmd_start);
    let end = Pos::body(0, cmd_end);
    assert_eq!(s.doc.copy_range(&start, &end).plain_text(), "statue");
}

#[test]
fn confusion_check_command_finds_statute_statue() {
    let mut s = law();
    run(&mut s, "text.insert", json!({"text": "The statue of limitations has run."}));
    let r = run(&mut s, "law.proof.confusions", json!({"store": false}));
    let findings = r["findings"].as_array().expect("findings");
    assert!(findings.iter().any(|f| f["message"].as_str().is_some_and(|m| m.contains("statute"))));
}

/// The confusion check reaches the real Review › Proofing path: an enabled session's module hook
/// is carried on `Session::module_en` into layout proofing, so `review.issues` (what the Review
/// pane and F7 read) reports it — and an uninstalled session does not.
#[test]
fn confusion_issues_hook_into_review_proofing() {
    let text = "The statue of limitations has run.";
    let mut s = law();
    run(&mut s, "text.insert", json!({"text": text}));
    let issues = run(&mut s, "review.issues", json!({}));
    assert!(
        issues.as_array().is_some_and(|a| a.iter().any(|i| i["message"].as_str().is_some_and(|m| m.contains("statute")))),
        "review.issues should report the confusion: {issues}"
    );

    let mut off = Session::new(Document::new());
    assert_eq!(install(&mut off), 0);
    run(&mut off, "text.insert", json!({"text": text}));
    let issues = run(&mut off, "review.issues", json!({}));
    assert!(issues.as_array().is_some_and(|a| a.is_empty()), "without LawCraft the same text is quiet: {issues}");
}

/// #5 blocking review: an interrupting appositive must not join two phrase words. The comma after
/// `principle` separates it from `issue`, so the correctly used noun `principle` stays quiet
/// instead of being flagged as `principal`; the same goes for a comma before `council for the
/// plaintiff` and before `principal of law`. A phrase gap must be bare whitespace to match.
#[test]
fn appositive_punctuation_does_not_join_phrase_words() {
    for text in [
        "The governing principle, issue preclusion, bars relitigation.",
        "The city council, for the plaintiff, met yesterday.",
        "The company chose the principal, of law and equity, to lead it.",
    ] {
        let issues = confusion_issues(text);
        assert!(issues.is_empty(), "unexpected finding in: {text} → {issues:?}");
    }
}

/// A firing case must name the exact word highlighted and the exact replacement offered, so a
/// wrong suggestion can't satisfy coverage by merely producing some finding.
macro_rules! confusion_pair {
    ($name:ident, $fires:expr, $quiet:expr, $wrong:expr, $right:expr) => {
        #[test]
        fn $name() {
            let fired = confusion_issues($fires);
            assert_eq!(fired.len(), 1, "expected one finding in: {} → {fired:?}", $fires);
            let issue = &fired[0];
            assert_eq!($fires.get(issue.start..issue.end), Some($wrong), "highlighted word in: {}", $fires);
            assert_eq!(issue.suggestions, vec![$right.to_string()], "replacement in: {}", $fires);
            let quiet = confusion_issues($quiet);
            assert!(quiet.is_empty(), "unexpected finding in: {} → {:?}", $quiet, quiet);
        }
    };
}

confusion_pair!(statute_statue, "The statue of limitations has run.", "The statute of limitations has run.", "statue", "statute");
confusion_pair!(
    statute_of_frauds,
    "The claim is barred by the statue of frauds.",
    "The claim is barred by the statute of frauds.",
    "statue",
    "statute"
);
confusion_pair!(principal_principle_issue, "The principle issue is liability.", "The principal issue is liability.", "principle", "principal");
confusion_pair!(
    principle_place_of_business,
    "The company's principle place of business is Springfield.",
    "The company's principal place of business is Springfield.",
    "principle",
    "principal"
);
confusion_pair!(principle_amount, "The principle amount remains unpaid.", "The principal amount remains unpaid.", "principle", "principal");
confusion_pair!(principal_of_law, "The principal of law is settled.", "The principle of law is settled.", "principal", "principle");
confusion_pair!(counsel_council, "Attorney council of record filed a brief.", "Counsel of record filed a brief.", "council", "counsel");
confusion_pair!(
    counsel_council_ordinance,
    "The council of record filed an appearance.",
    "Counsel of record filed an appearance.",
    "council",
    "counsel"
);
confusion_pair!(
    opposing_council,
    "We conferred with opposing council before trial.",
    "We conferred with opposing counsel before trial.",
    "council",
    "counsel"
);
confusion_pair!(
    council_for_the_plaintiff,
    "Council for the plaintiff filed a brief.",
    "Counsel for the plaintiff filed a brief.",
    "Council",
    "Counsel"
);

/// `council for the <party>` is the common typo for `counsel for the <party>`; every party role the
/// phrase list names fires, and the suggestion is the capitalized `Counsel`.
#[test]
fn council_for_each_party_role_fires() {
    for role in ["plaintiff", "defendant", "appellant", "respondent", "petitioner", "movant"] {
        let text = format!("Council for the {role} filed a brief.");
        let issues = confusion_issues(&text);
        assert_eq!(issues.len(), 1, "expected one finding in: {text} → {issues:?}");
        assert_eq!(text.get(issues[0].start..issues[0].end), Some("Council"));
        assert_eq!(issues[0].suggestions, vec!["Counsel".to_string()], "replacement in: {text}");
    }
}
confusion_pair!(affect_effect, "The affect of the ruling was clear.", "The new rule will affect the outcome.", "affect", "effect");
confusion_pair!(affect_statute, "The affect of the ruling was clear.", "The rule will affect enforcement of the statute.", "affect", "effect");
confusion_pair!(
    precedent_precedence,
    "This is binding precedence in our circuit.",
    "Federal law takes precedence over state law.",
    "precedence",
    "precedent"
);
confusion_pair!(
    controlling_precedence,
    "This decision is controlling precedence here.",
    "The statute takes precedence over the rule.",
    "precedence",
    "precedent"
);
confusion_pair!(
    prescribe_proscribe,
    "The doctor proscribe medication for the patient.",
    "The rules prescribe such conduct by counsel.",
    "proscribe",
    "prescribe"
);

/// #5 review: correct wording must not be flagged. A governing-body `council` — even "council for
/// the district" — and a rule that affirmatively `prescribe`s conduct are both correct; adjacency
/// alone doesn't make the word likely wrong. The clear `council of record`, `council for the
/// plaintiff` and `proscribe medication` cases still fire (see the pairs above).
#[test]
fn correct_council_and_prescribe_contexts_are_quiet() {
    for text in [
        "The city council for the district met yesterday.",
        "The county council for the region approved the plan.",
        "The rules prescribe such conduct by counsel.",
        "The statute prescribe such conduct.",
    ] {
        let issues = confusion_issues(text);
        assert!(issues.is_empty(), "unexpected finding in: {text} → {issues:?}");
    }
}

/// #5 review: a law, rule or regulator `proscribe`s (forbids) things — including medication
/// advertising — so `proscribe medication` is only likely wrong when a medical actor is the
/// immediate subject. The prohibition stays quiet; the doctor fires.
#[test]
fn proscribe_forbids_medication_only_for_medical_actors() {
    let fired = confusion_issues("The doctor will proscribe medication for the patient.");
    assert_eq!(fired.len(), 1, "expected one finding: {fired:?}");
    assert_eq!(fired[0].suggestions, vec!["prescribe".to_string()]);
    for text in [
        "The regulation will proscribe medication advertising to minors.",
        "The statute will proscribe medication by unlicensed sellers.",
        "The board may proscribe medication in prison.",
        "The physician testified that the regulation will proscribe medication advertising to minors.",
    ] {
        let issues = confusion_issues(text);
        assert!(issues.is_empty(), "unexpected finding in: {text} → {issues:?}");
    }
}

/// Correct noun `principle` before a verb must stay quiet (there is no `principle claim` entry):
/// `claim` and `places` are verbs here, not the listed legal phrases.
#[test]
fn principle_as_noun_before_a_verb_is_quiet() {
    for text in [
        "The principle issues from natural law.",
        "Does the principle claim our attention?",
        "The principle places the burden on the movant.",
        "The principle claim for damages was dismissed.",
    ] {
        let issues = confusion_issues(text);
        assert!(issues.is_empty(), "unexpected finding in: {text} → {issues:?}");
    }
}

// `averse possession` is the typo for the hostile-claim term `adverse possession`; the correct
// `averse to risk` use stays quiet (see the pair above).
confusion_pair!(averse_possession, "The tenant claimed averse possession.", "The tenant claimed adverse possession.", "averse", "adverse");
confusion_pair!(adverse_to_risk, "The clause is adverse to risk.", "The investor was averse to risk.", "adverse", "averse");
confusion_pair!(
    elicit_illicit,
    "The deposition will illicit testimony about the contract.",
    "The deposition will elicit testimony about the contract.",
    "illicit",
    "elicit"
);
confusion_pair!(
    elicit_illicit_response,
    "The witness could illicit a response on cross-examination.",
    "The witness could elicit a response on cross-examination.",
    "illicit",
    "elicit"
);
confusion_pair!(complement_compliment, "The exhibit compliment the record.", "The exhibit complement the record.", "compliment", "complement");
confusion_pair!(
    discrete_discreet,
    "There are three discreet issues before us.",
    "The agency opened a discrete investigation into each transaction.",
    "discreet",
    "discrete"
);
confusion_pair!(effect_affect_outcome, "The new rule will effect the outcome.", "The new rule will affect the outcome.", "effect", "affect");

/// #5 review: noun `effect` before another noun phrase must not suggest `affect`.
#[test]
fn effect_as_noun_before_the_outcome_is_quiet() {
    let text = "We assessed the effect the outcome had on the parties.";
    let issues = confusion_issues(text);
    assert!(issues.is_empty(), "unexpected finding in: {text} → {issues:?}");
}

/// #5 review: a correct `complement` (to complete or balance) must not be flagged, and ordinary
/// legal prose that uses the verb `elicit` correctly stays quiet. `illicit testimony` is the typo
/// the phrase list names, so it fires (see the pair above); the correct `elicit` does not.
#[test]
fn correct_elicit_and_complement_contexts_are_quiet() {
    for text in [
        "The prosecutor elicited testimony from the witness.",
        "The court allowed counsel to elicit testimony on redirect.",
        "The new tribunal will complement the court.",
        "These judges complement the court well.",
    ] {
        let issues = confusion_issues(text);
        assert!(issues.is_empty(), "unexpected finding in: {text} → {issues:?}");
    }
}

/// #5 review: `discrete investigation` means separate investigations and is correct, so it must
/// never suggest `discreet`; a genuinely tactful `discreet investigation` is correct too. Only the
/// `discreet issues` direction (separate issues) fires, and removing the reverse rule keeps both
/// correct uses quiet.
#[test]
fn discrete_investigation_is_quiet() {
    for text in [
        "The agency opened a discrete investigation into each transaction.",
        "The court ran a discrete investigation of each charge.",
        "The firm conducted a discreet investigation.",
    ] {
        let issues = confusion_issues(text);
        assert!(issues.is_empty(), "unexpected finding in: {text} → {issues:?}");
    }
}

/// #5 review: the mixed-case `RSMo` users actually type is clean with LawCraft on, through the
/// same paragraph spelling path the editor uses, and flagged when it is off.
#[test]
fn mixed_case_rsmo_spells_clean_only_with_lawcraft() {
    let opts = Options::default();
    let text = "The RSMo governs this appeal under S.W.3d.";
    let on = law();
    let issues = check_spelling_with_module(text, &opts, on.module_en);
    assert!(issues.is_empty(), "LawCraft should not flag `{text}`: {issues:?}");
    let off = check_spelling_with_module(text, &opts, ModuleEn::default());
    assert!(off.iter().any(|i| text.get(i.start..i.end) == Some("RSMo")), "without LawCraft, RSMo is flagged: {off:?}");
}
