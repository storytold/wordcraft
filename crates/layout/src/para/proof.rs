use super::{CharProps, Document, Paragraph, ResolvedChar};

/// One run's proofing: its bytes in the paragraph, its language (`w:lang`, resolved through the
/// styles: the document default, paragraph and character styles, direct formatting) and whether
/// it is left unchecked ("Do not check spelling or grammar", hidden text, hyperlinks).
#[derive(Clone, Debug, PartialEq)]
pub struct ProofRun {
    pub range: std::ops::Range<usize>,
    pub lang: wordcraft_proof::ProofLang,
    pub skip: bool,
}

impl ProofRun {
    pub(super) fn of(range: std::ops::Range<usize>, rc: &ResolvedChar) -> ProofRun {
        ProofRun { range, lang: wordcraft_proof::ProofLang::from_tag(rc.lang.as_deref()), skip: rc.no_proof || rc.hidden || rc.link.is_some() }
    }
}

/// The proofing runs of a paragraph, in order. `table` is the cell's table-style character
/// formatting ([`CellText::chr`]) for a paragraph in a table, as layout resolves it.
pub fn proof_runs(doc: &Document, p: &Paragraph, table: Option<&CharProps>) -> Vec<ProofRun> {
    let style = p.props.style.as_deref();
    p.run_ranges().map(|(r, c)| ProofRun::of(r, &doc.styles.resolve_char_in(style, table, c))).collect()
}

/// The language of the run holding byte `off` (English past the last run).
pub(super) fn lang_at(runs: &[ProofRun], off: usize) -> wordcraft_proof::ProofLang {
    let k = runs.partition_point(|r| r.range.end <= off);
    runs.get(k).filter(|r| r.range.start <= off).map(|r| r.lang).unwrap_or_default()
}

/// Spelling and grammar issues in a paragraph, in order: each word in its run's language
/// ([`wordcraft_proof::check_text_with`]: what spelling skips, and grammar only with `grammar`),
/// and none touching a run left unchecked.
///
/// `list_item` (see [`is_list_item`]) turns off the capital-first-word grammar rule.
pub fn paragraph_issues(
    p: &Paragraph,
    runs: &[ProofRun],
    spelling: &wordcraft_proof::Options,
    grammar: bool,
    list_item: bool,
) -> Vec<wordcraft_proof::Issue> {
    if p.text.trim().is_empty() || p.text.len() > 100_000 {
        return Vec::new();
    }
    let text = proof_text(p);
    let spans: Vec<(std::ops::Range<usize>, wordcraft_proof::ProofLang)> = runs.iter().map(|r| (r.range.clone(), r.lang)).collect();
    let skipped = skipped_ranges(runs);
    let mut issues = wordcraft_proof::check_text_with(&text, &spans, spelling, grammar, list_item);
    if !skipped.is_empty() {
        issues.retain(|i| !overlaps(&skipped, i.start, i.end));
    }
    issues.retain(|i| !is_german_coordinated_compound(&text, i, spelling));
    issues
}

/// Whether `c` is a hyphen character (ASCII hyphen-minus, Unicode hyphen, non-breaking hyphen, or en-dash).
fn is_hyphen(c: char) -> bool {
    matches!(c, '-' | '\u{2010}' | '\u{2011}' | '\u{2013}')
}

/// Lowercase the first character of `s`, preserving the rest.
fn lowercase_first(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) => {
            let mut res = c.to_lowercase().to_string();
            res.push_str(chars.as_str());
            res
        }
        None => String::new(),
    }
}

/// Uppercase the first character of `s`, preserving the rest.
fn uppercase_first(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) => {
            let mut res = c.to_uppercase().to_string();
            res.push_str(chars.as_str());
            res
        }
        None => String::new(),
    }
}

/// Whether `token` is a German coordination conjunction, separator, or article that can appear
/// between constituents of a coordinated compound.
fn is_german_connector(token: &str) -> bool {
    matches!(
        token,
        "und"
            | "oder"
            | "sowie"
            | "bzw"
            | "bzw."
            | "wie"
            | "als"
            | "auch"
            | "noch"
            | "weder"
            | "entweder"
            | "aber"
            | "sondern"
            | "und/oder"
            | "/"
            | ","
            | "der"
            | "die"
            | "das"
            | "dem"
            | "den"
            | "des"
            | "ein"
            | "eine"
            | "einer"
            | "eines"
            | "einem"
            | "einen"
    )
}

/// Whether a spelling issue is a valid German coordinated compound with ellipsis (*Ergänzungsstrich*).
///
/// In German orthography (§ 98 Amtliche Rechtschreibung), a suspended hyphen marks an omitted
/// compound constituent in coordinated compounds (e.g. `Handlungs- und Zustandssatz`,
/// `Schul- und Universitätsbetrieb`, or `Wasserverbrauch und -qualität`).
///
/// If the flagged word has a trailing or leading ellipsis hyphen in a coordination structure,
/// and combining it with the coordination partner forms a valid compound in German, it is not
/// an orthographic error.
fn is_german_coordinated_compound(text: &str, issue: &wordcraft_proof::Issue, opts: &wordcraft_proof::Options) -> bool {
    if issue.kind != wordcraft_proof::IssueKind::Spelling || issue.lang.code() != "de" {
        return false;
    }
    let Some(word) = text.get(issue.start..issue.end) else {
        return false;
    };
    if word.is_empty() {
        return false;
    }

    if is_forward_coordinated_compound(text, issue.start, issue.end, word, issue.lang, opts) {
        return true;
    }

    if is_backward_coordinated_compound(text, issue.start, issue.end, word, issue.lang, opts) {
        return true;
    }

    false
}

/// Check if a flagged word is the first element of a German coordinated compound with forward
/// ellipsis (e.g. `Handlungs- und Zustandssatz`, `Schul- und Universitätsbetrieb`).
fn is_forward_coordinated_compound(
    text: &str,
    _start: usize,
    end: usize,
    word: &str,
    lang: wordcraft_proof::ProofLang,
    opts: &wordcraft_proof::Options,
) -> bool {
    let Some(after_word) = text.get(end..) else {
        return false;
    };
    let Some(hyphen_char) = after_word.chars().next().filter(|c| is_hyphen(*c)) else {
        return false;
    };
    let Some(after_hyphen) = text.get(end.saturating_add(hyphen_char.len_utf8())..) else {
        return false;
    };
    // Must be followed by whitespace, comma, or slash.
    let is_boundary = after_hyphen.chars().next().is_some_and(|c| c.is_whitespace() || c == ',' || c == '/');
    if !is_boundary {
        return false;
    }

    // Look ahead for candidate partner words.
    let mut saw_connector = false;
    let mut remainder = after_hyphen;
    for _ in 0..12 {
        remainder = remainder.trim_start();
        if remainder.is_empty() {
            break;
        }
        if remainder.starts_with(['.', '!', '?', ';', '\n']) && !remainder.starts_with("bzw.") {
            break;
        }
        if remainder.starts_with('/') {
            saw_connector = true;
            remainder = remainder.get(1..).unwrap_or("");
            continue;
        }
        if remainder.starts_with(',') {
            saw_connector = true;
            remainder = remainder.get(1..).unwrap_or("");
            continue;
        }

        let token_end = remainder.find(|c: char| c.is_whitespace() || c == '/' || c == ',').unwrap_or(remainder.len());
        let Some(raw_token) = remainder.get(..token_end) else {
            break;
        };
        remainder = remainder.get(token_end..).unwrap_or("");

        let token = raw_token.trim_end_matches(|c: char| !c.is_alphabetic() && !is_hyphen(c));
        if token.is_empty() {
            continue;
        }

        if token.ends_with(is_hyphen) {
            saw_connector = true;
            continue;
        }

        let lower = token.to_lowercase();
        if is_german_connector(&lower) {
            saw_connector = true;
            continue;
        }

        if saw_connector {
            let partner = token.trim_matches(|c: char| !c.is_alphabetic());
            if partner.chars().count() >= 2 && test_compound_forward(word, partner, lang, opts) {
                return true;
            }
        }
    }

    false
}

/// Test whether `word` plus any valid suffix/base of `partner` forms a valid German compound.
fn test_compound_forward(word: &str, partner: &str, lang: wordcraft_proof::ProofLang, opts: &wordcraft_proof::Options) -> bool {
    let word_lower = lowercase_first(word);
    let word_upper = uppercase_first(word);

    for (k, _) in partner.char_indices() {
        let Some(suffix) = partner.get(k..) else {
            continue;
        };
        if suffix.chars().count() < 2 || suffix.len() > 40 {
            continue;
        }

        let suffix_lower = lowercase_first(suffix);
        let suffix_upper = uppercase_first(suffix);

        let candidates = [
            format!("{word}{suffix_lower}"),
            format!("{word}{suffix}"),
            format!("{word_upper}{suffix_lower}"),
            format!("{word_lower}{suffix_lower}"),
            format!("{word}-{suffix}"),
            format!("{word}-{suffix_upper}"),
            format!("{word_upper}-{suffix_upper}"),
        ];

        for cand in &candidates {
            if wordcraft_proof::is_correct_in_with(cand, lang, opts) {
                return true;
            }
        }
    }

    false
}

/// Check if a flagged word is the second element of a German coordinated compound with backward
/// ellipsis (e.g. `Wasserverbrauch und -qualität`).
fn is_backward_coordinated_compound(
    text: &str,
    start: usize,
    _end: usize,
    word: &str,
    lang: wordcraft_proof::ProofLang,
    opts: &wordcraft_proof::Options,
) -> bool {
    let Some(before_word) = text.get(..start) else {
        return false;
    };
    let Some(hyphen_char) = before_word.chars().next_back().filter(|c| is_hyphen(*c)) else {
        return false;
    };
    let Some(before_hyphen) = text.get(..start.saturating_sub(hyphen_char.len_utf8())) else {
        return false;
    };
    let is_boundary = before_hyphen.chars().next_back().is_some_and(|c| c.is_whitespace() || c == '/');
    if !is_boundary {
        return false;
    }

    let mut saw_connector = false;
    let mut remainder = before_hyphen;
    for _ in 0..12 {
        remainder = remainder.trim_end();
        if remainder.is_empty() {
            break;
        }
        if remainder.ends_with('/') {
            saw_connector = true;
            remainder = remainder.get(..remainder.len().saturating_sub(1)).unwrap_or("");
            continue;
        }
        if remainder.ends_with(',') {
            saw_connector = true;
            remainder = remainder.get(..remainder.len().saturating_sub(1)).unwrap_or("");
            continue;
        }

        let token_start = remainder.rfind(|c: char| c.is_whitespace() || c == '/' || c == ',').map_or(0, |i| i + 1);
        let Some(raw_token) = remainder.get(token_start..) else {
            break;
        };
        remainder = remainder.get(..token_start.saturating_sub(1)).unwrap_or("");

        let token = raw_token.trim_start_matches(|c: char| !c.is_alphabetic() && !is_hyphen(c));
        if token.is_empty() {
            continue;
        }

        if token.starts_with(is_hyphen) {
            saw_connector = true;
            continue;
        }

        let lower = token.to_lowercase();
        if is_german_connector(&lower) {
            saw_connector = true;
            continue;
        }

        if saw_connector {
            let partner = token.trim_matches(|c: char| !c.is_alphabetic());
            if partner.chars().count() >= 2 && test_compound_backward(partner, word, lang, opts) {
                return true;
            }
        }
    }

    false
}

/// Test whether any valid prefix of `partner` plus `word` forms a valid German compound.
fn test_compound_backward(partner: &str, word: &str, lang: wordcraft_proof::ProofLang, opts: &wordcraft_proof::Options) -> bool {
    let word_lower = lowercase_first(word);
    let word_upper = uppercase_first(word);

    for (k, _) in partner.char_indices() {
        if !(2..=40).contains(&k) {
            continue;
        }
        let Some(prefix) = partner.get(..k) else {
            continue;
        };
        if prefix.chars().count() < 2 {
            continue;
        }

        let prefix_upper = uppercase_first(prefix);
        let prefix_lower = lowercase_first(prefix);

        let candidates = [
            format!("{prefix}{word_lower}"),
            format!("{prefix}{word}"),
            format!("{prefix_upper}{word_lower}"),
            format!("{prefix_lower}{word_lower}"),
            format!("{prefix}-{word}"),
            format!("{prefix}-{word_upper}"),
            format!("{prefix_upper}-{word_upper}"),
        ];

        for cand in &candidates {
            if wordcraft_proof::is_correct_in_with(cand, lang, opts) {
                return true;
            }
        }
    }

    false
}

/// The byte ranges of the runs left unchecked, in order, those that overlap merged: their
/// starts and ends both rise, so [`overlaps`] finds an issue's by binary search instead of
/// looking at every run for every issue.
fn skipped_ranges(runs: &[ProofRun]) -> Vec<(usize, usize)> {
    let mut ranges: Vec<(usize, usize)> = runs.iter().filter(|r| r.skip).map(|r| (r.range.start, r.range.end)).collect();
    ranges.sort_unstable();
    let mut merged: Vec<(usize, usize)> = Vec::with_capacity(ranges.len());
    for (s, e) in ranges {
        crate::work::steps(1);
        match merged.last_mut() {
            Some(last) if s < last.1 => last.1 = last.1.max(e),
            _ => merged.push((s, e)),
        }
    }
    merged
}

/// Whether bytes `a..b` overlap one of `skipped` ([`skipped_ranges`]): the first range ending
/// after `a` is the only one that can, as every later one starts no earlier.
fn overlaps(skipped: &[(usize, usize)], a: usize, b: usize) -> bool {
    crate::work::steps(1);
    let k = skipped.partition_point(|&(_, e)| e <= a);
    skipped.get(k).is_some_and(|&(s, _)| s < b)
}

/// Whether the paragraph is a list item: its own numbering, or its style's, names a list (not 0).
pub fn is_list_item(doc: &Document, p: &Paragraph) -> bool {
    p.props.numbering.or_else(|| doc.styles.resolve_para(&p.props).numbering).is_some_and(|n| n.num != 0)
}

/// Spelling and grammar issues as squiggles (start, end, is grammar), less spelling issues overlapping `typing`.
pub(super) fn proof_issues(
    p: &Paragraph,
    runs: &[ProofRun],
    opts: &crate::ProofOptions,
    typing: Option<(usize, usize)>,
    list_item: bool,
) -> Vec<(usize, usize, bool)> {
    paragraph_issues(p, runs, &opts.spelling, opts.grammar, list_item)
        .into_iter()
        .map(|i| (i.start, i.end, i.kind == wordcraft_proof::IssueKind::Grammar))
        .filter(|&(a, b, grammar)| grammar || !typing.is_some_and(|(s, e)| a < e && s < b))
        .collect()
}

/// Text for proofing with the same byte offsets: inline objects and tracked deletions become
/// U+0001 bytes (neither words nor spaces).
pub fn proof_text(p: &wordcraft_doc::Paragraph) -> String {
    let mut out = String::with_capacity(p.text.len());
    // The run holding each character ([`Paragraph::props_of_char`]), found by walking the runs
    // along with the text rather than from the first run for every character.
    let mut runs = p.runs.iter();
    let mut run = runs.next();
    let mut end = run.map_or(0, |r| r.len);
    // Each run this walk looks at is a step ([`crate::work`]), so the linear-time test counts
    // them instead of timing the walk: a caller that went back to the first run for every
    // character would count the runs once per character and show up as a quadratic count (#5185).
    crate::work::steps(usize::from(run.is_some()));
    for (i, c) in p.text.char_indices() {
        while run.is_some() && i >= end {
            run = runs.next();
            end = end.saturating_add(run.map_or(0, |r| r.len));
            crate::work::steps(usize::from(run.is_some()));
        }
        let props = run.map_or(&p.mark, |r| &r.props);
        let deleted = c == wordcraft_doc::para::OBJ || props.del.is_some();
        if deleted {
            for _ in 0..c.len_utf8() {
                out.push('\u{1}');
            }
        } else {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(range: std::ops::Range<usize>, skip: bool) -> ProofRun {
        ProofRun { range, lang: Default::default(), skip }
    }

    /// The lookup finds what a look at every run finds, overlapping and empty runs included.
    #[test]
    fn skipped_lookup_matches_a_scan_of_every_run() {
        let runs = [run(0..4, false), run(4..9, true), run(5..6, true), run(9..9, true), run(9..12, false), run(12..20, true), run(14..30, true)];
        let skipped = skipped_ranges(&runs);
        for a in 0..32 {
            for b in a..32 {
                let scan = runs.iter().any(|r| r.skip && r.range.start < b && a < r.range.end);
                assert_eq!(overlaps(&skipped, a, b), scan, "{a}..{b}");
            }
        }
        assert!(skipped_ranges(&[run(0..5, false)]).is_empty());
    }

    /// German coordinated compounds with ellipsis (e.g. `Handlungs- und Zustandssatz`,
    /// `Schul- und Universitätsbetrieb`, or `Wasserverbrauch und -qualität`) are valid
    /// compound forms (§ 98) and must not be flagged as spelling errors (#5903).
    #[test]
    fn german_coordinated_compounds_with_ellipsis_are_accepted() {
        let snap_dir = std::path::PathBuf::from("/snap/kf6-core24/69/usr/share/hunspell");
        let system_dir = std::path::PathBuf::from("/usr/share/hunspell");
        let temp_dir = std::env::temp_dir().join(format!("wordcraft-test-de-{}", std::process::id()));
        if snap_dir.join("de_DE.dic").is_file() {
            wordcraft_proof::set_dictionaries_folder(Some(snap_dir));
        } else if system_dir.join("de_DE.dic").is_file() {
            wordcraft_proof::set_dictionaries_folder(Some(system_dir));
        } else {
            let _ = std::fs::create_dir_all(&temp_dir);
            let aff = "SET UTF-8\nCOMPOUNDFLAG X\n";
            let dic = "22\nHandlung/X\nHandlungssatz/X\nZustandssatz/X\nSchulbetrieb/X\nUniversitätsbetrieb/X\nArbeitsrecht/X\nSozialrecht/X\nForschungsabteilung/X\nEntwicklungsabteilung/X\nWirtschaftspolitik/X\nFinanzpolitik/X\nVersicherungsrecht/X\nVertragsrecht/X\nBildungsauftrag/X\nErziehungsauftrag/X\nBewegungsverb/X\nWahrnehmungsverb/X\nOrdnungsdienst/X\nSicherheitsdienst/X\nGestaltungssatz/X\nWasserverbrauch/X\nWasserqualität/X\n";
            let _ = std::fs::write(temp_dir.join("de_DE.aff"), aff);
            let _ = std::fs::write(temp_dir.join("de_DE.dic"), dic);
            wordcraft_proof::set_dictionaries_folder(Some(temp_dir.clone()));
        }

        assert!(wordcraft_proof::has_dictionary(Some("de-DE")), "German dictionary should be available");

        let cases = [
            "Handlungs- und Zustandssatz",
            "Schul- und Universitätsbetrieb",
            "Arbeits- und Sozialrecht",
            "Forschungs- und Entwicklungsabteilung",
            "Wirtschafts- und Finanzpolitik",
            "Versicherungs- und Vertragsrecht",
            "Bildungs- und Erziehungsauftrag",
            "Bewegungs- und Wahrnehmungsverb",
            "Ordnungs- und Sicherheitsdienst",
            "Friedens- und Konfliktforschung",
            "Liebes- und Ehebeziehung",
            "Lebens- und Arbeitsbedingungen",
            "Handlungs-, Gestaltungs- und Zustandssatz",
            "Schul-/Universitätsbetrieb",
            "Wasserverbrauch und -qualität",
        ];

        for text in cases {
            let mut d = Document::from_text(text);
            d.format_range(&wordcraft_doc::Pos::body(0, 0), &wordcraft_doc::Pos::body(0, text.len()), &|c| c.lang = Some("de-DE".into())).unwrap();
            let p = d.para(wordcraft_doc::StoryRef::Body, &wordcraft_doc::Path::top(0)).unwrap();
            let issues = paragraph_issues(p, &proof_runs(&d, p, None), &Default::default(), true, false);
            let spelling_issues: Vec<_> = issues.iter().filter(|i| i.kind == wordcraft_proof::IssueKind::Spelling).collect();
            assert!(spelling_issues.is_empty(), "expected no spelling issues for {text:?}, got: {spelling_issues:?}");
        }

        // Invalid compounds must still be flagged.
        let invalid = "Xyzqwe- und Zustandssatz";
        let mut d = Document::from_text(invalid);
        d.format_range(&wordcraft_doc::Pos::body(0, 0), &wordcraft_doc::Pos::body(0, invalid.len()), &|c| c.lang = Some("de-DE".into())).unwrap();
        let p = d.para(wordcraft_doc::StoryRef::Body, &wordcraft_doc::Path::top(0)).unwrap();
        let issues = paragraph_issues(p, &proof_runs(&d, p, None), &Default::default(), true, false);
        assert!(!issues.is_empty(), "expected invalid compound to be flagged");

        let _ = std::fs::remove_dir_all(&temp_dir);
    }
}
