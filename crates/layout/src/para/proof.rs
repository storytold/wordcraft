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
    filter_german_coordinated_compounds(&text, &mut issues, spelling);
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

/// Whether `token` is a known German abbreviation ending with a period that does not terminate a sentence.
fn is_german_abbreviation(token: &str) -> bool {
    let lower = token.to_lowercase();
    matches!(
        lower.as_str(),
        "bzw." | "z.b." | "u.a." | "d.h." | "usw." | "etc." | "evtl." | "ggf." | "v.a." | "insb." | "ca." | "sog." | "inkl." | "exkl." | "bzgl."
    )
}

/// Whether `s` starts with a known German abbreviation ending with a period.
fn starts_with_german_abbreviation(s: &str) -> bool {
    let s_trimmed = s.trim_start();
    if s_trimmed.is_empty() {
        return false;
    }
    let token_end = s_trimmed.find(|c: char| c.is_whitespace() || c == '/' || c == ',').unwrap_or(s_trimmed.len());
    let Some(raw_token) = s_trimmed.get(..token_end) else {
        return false;
    };
    is_german_abbreviation(raw_token)
}

/// Whether `s` ends with a known German abbreviation ending with a period.
fn ends_with_german_abbreviation(s: &str) -> bool {
    let s_trimmed = s.trim_end();
    if s_trimmed.is_empty() {
        return false;
    }
    let sep_match = s_trimmed.char_indices().rfind(|(_, c)| c.is_whitespace() || *c == '/' || *c == ',');
    let token_start = match sep_match {
        Some((idx, c)) => idx.saturating_add(c.len_utf8()),
        None => 0,
    };
    let Some(raw_token) = s_trimmed.get(token_start..) else {
        return false;
    };
    is_german_abbreviation(raw_token)
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

/// Whether `lang` is German (standard German [`wordcraft_proof::ProofLang::De`], or a German pack locale such as `de-CH`).
fn is_german_lang(lang: wordcraft_proof::ProofLang) -> bool {
    lang == wordcraft_proof::ProofLang::De
        || lang.pack_locale().is_some_and(|l| {
            let s = l.as_str();
            s == "de" || s.starts_with("de-") || s.starts_with("de_")
        })
}

/// Filter out valid German coordinated compounds with ellipsis (*Ergänzungsstrich*).
///
/// In German orthography (§ 98 Amtliche Rechtschreibung), a suspended hyphen marks an omitted
/// compound constituent in coordinated compounds (e.g. `Handlungs- und Zustandssatz`,
/// `Schul- und Universitätsbetrieb`, or `Wasserverbrauch und -qualität`).
///
/// Scans are cached across the paragraph to avoid quadratic rescanning for repeated or long lists.
fn filter_german_coordinated_compounds(text: &str, issues: &mut Vec<wordcraft_proof::Issue>, opts: &wordcraft_proof::Options) {
    let mut cached_forward: Option<(std::ops::Range<usize>, Option<String>)> = None;
    let mut backward_state = BackwardScanState::default();

    issues.retain(|issue| {
        if issue.kind != wordcraft_proof::IssueKind::Spelling || !is_german_lang(issue.lang) {
            return true;
        }
        let Some(word) = text.get(issue.start..issue.end) else {
            return true;
        };
        if word.is_empty() {
            return true;
        }

        if check_forward_coordinated(text, issue.start, issue.end, word, issue.lang, opts, &mut cached_forward) {
            return false;
        }

        if check_backward_coordinated(text, issue.start, issue.end, word, issue.lang, opts, &mut backward_state) {
            return false;
        }

        true
    });
}

/// Check if a flagged word is part of a valid German forward coordinated compound, reusing scan results.
fn check_forward_coordinated(
    text: &str,
    start: usize,
    end: usize,
    word: &str,
    lang: wordcraft_proof::ProofLang,
    opts: &wordcraft_proof::Options,
    cache: &mut Option<(std::ops::Range<usize>, Option<String>)>,
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
    let is_boundary = after_hyphen.chars().next().is_some_and(|c| c.is_whitespace() || c == ',' || c == '/');
    if !is_boundary {
        return false;
    }

    if let Some((range, partner_opt)) = cache
        && range.contains(&start)
    {
        return partner_opt.as_deref().is_some_and(|partner| test_compound_forward(word, partner, lang, opts));
    }

    let (scan_end, partner_opt) = scan_forward_partner(text, word, end.saturating_add(hyphen_char.len_utf8()), lang, opts);
    *cache = Some((start..scan_end, partner_opt.clone()));

    partner_opt.as_deref().is_some_and(|partner| test_compound_forward(word, partner, lang, opts))
}

/// Scan forward from `after_hyphen_offset` to find the terminal candidate partner word of a coordination chain.
///
/// Returns the byte offset up to which the scan reached and the candidate partner word, if found.
fn scan_forward_partner(
    text: &str,
    word: &str,
    after_hyphen_offset: usize,
    lang: wordcraft_proof::ProofLang,
    opts: &wordcraft_proof::Options,
) -> (usize, Option<String>) {
    let mut saw_connector = false;
    let mut modifier_count: usize = 0;
    let mut remainder = text.get(after_hyphen_offset..).unwrap_or("");
    let mut current_offset = after_hyphen_offset;

    while !remainder.is_empty() {
        crate::work::steps(1);
        let prev_len = remainder.len();
        let trimmed = remainder.trim_start();
        current_offset = current_offset.saturating_add(remainder.len().saturating_sub(trimmed.len()));
        remainder = trimmed;
        if remainder.is_empty() {
            break;
        }
        if remainder.starts_with(['.', '!', '?', ';', ':', '\n']) && !starts_with_german_abbreviation(remainder) {
            break;
        }
        if remainder.starts_with('/') || remainder.starts_with(',') {
            saw_connector = true;
            let first_char_len = remainder.chars().next().map_or(1, |c| c.len_utf8());
            remainder = remainder.get(first_char_len..).unwrap_or("");
            current_offset = current_offset.saturating_add(first_char_len);
            if remainder.len() >= prev_len {
                break;
            }
            continue;
        }

        let token_end = remainder.find(|c: char| c.is_whitespace() || c == '/' || c == ',').unwrap_or(remainder.len());
        let Some(raw_token) = remainder.get(..token_end) else {
            break;
        };
        remainder = remainder.get(token_end..).unwrap_or("");
        current_offset = current_offset.saturating_add(token_end);
        if remainder.len() >= prev_len {
            break;
        }

        let has_sentence_punct = raw_token.ends_with(['.', '!', '?', ';', ':']) && !is_german_abbreviation(raw_token);

        let token = raw_token.trim_end_matches(|c: char| !c.is_alphabetic() && !is_hyphen(c));
        if token.is_empty() {
            if has_sentence_punct {
                break;
            }
            continue;
        }

        if token.ends_with(is_hyphen) {
            saw_connector = true;
            if has_sentence_punct {
                break;
            }
            continue;
        }

        let lower = token.to_lowercase();
        if is_german_connector(&lower) {
            saw_connector = true;
            if has_sentence_punct {
                break;
            }
            continue;
        }

        if saw_connector {
            let partner = token.trim_matches(|c: char| !c.is_alphabetic());
            if partner.chars().count() >= 2 {
                let is_noun = word.chars().next().is_some_and(|c| c.is_uppercase());
                let token_is_noun = token.chars().next().is_some_and(|c| c.is_uppercase());

                if is_noun && !token_is_noun {
                    if !has_sentence_punct && modifier_count < 2 {
                        modifier_count = modifier_count.saturating_add(1);
                        continue;
                    }
                    break;
                }

                if test_compound_forward(word, partner, lang, opts) {
                    return (current_offset, Some(partner.to_string()));
                }

                let is_lowercase = token.chars().next().is_some_and(|c| c.is_lowercase());
                if !has_sentence_punct && is_lowercase && modifier_count < 2 {
                    modifier_count = modifier_count.saturating_add(1);
                    continue;
                }

                if is_noun && token_is_noun {
                    return (current_offset, Some(partner.to_string()));
                }
            }

            break;
        }

        if has_sentence_punct {
            break;
        }
    }

    (current_offset, None)
}

/// Test whether `word` plus any valid suffix/base of `partner` forms a valid German compound.
fn test_compound_forward(word: &str, partner: &str, lang: wordcraft_proof::ProofLang, opts: &wordcraft_proof::Options) -> bool {
    let word_is_upper = word.chars().next().is_some_and(|c| c.is_uppercase());
    let word_upper = uppercase_first(word);

    let mut window_start = partner.len().saturating_sub(40);
    while !partner.is_char_boundary(window_start) {
        window_start = window_start.saturating_add(1);
    }
    let Some(window) = partner.get(window_start..) else {
        return false;
    };

    for (k, _) in window.char_indices() {
        crate::work::steps(1);
        let Some(suffix) = window.get(k..) else {
            continue;
        };
        if suffix.chars().count() < 2 {
            continue;
        }

        let suffix_lower = lowercase_first(suffix);
        let suffix_upper = uppercase_first(suffix);

        let mut candidates = Vec::with_capacity(3);
        if word_is_upper {
            candidates.push(format!("{word}{suffix_lower}"));
            candidates.push(format!("{word}-{suffix_upper}"));
            if suffix != suffix_lower && suffix != suffix_upper {
                candidates.push(format!("{word}{suffix}"));
            }
        } else {
            candidates.push(format!("{word}{suffix_lower}"));
            candidates.push(format!("{word_upper}{suffix_lower}"));
            candidates.push(format!("{word}-{suffix_lower}"));
        }

        for cand in &candidates {
            if wordcraft_proof::is_correct_in_with(cand, lang, opts) {
                return true;
            }
        }
    }

    false
}

/// State for caching backward coordination scans across consecutive issues in a paragraph.
#[derive(Default)]
struct BackwardScanState {
    /// The candidate partner word found for this chain, if any.
    partner: Option<String>,
    /// The end byte offset of the previous issue inspected in this paragraph.
    last_issue_end: usize,
    /// The maximum offset up to which backward scans have searched with no partner found.
    scanned_up_to: usize,
}

/// The outcome of scanning backward for a coordination partner word.
#[derive(Debug, PartialEq, Eq)]
enum BackwardScanResult {
    /// Found a candidate partner word.
    Found(String),
    /// Reached the stop offset through a valid, unbroken coordination sequence.
    ReachedStop,
    /// Hit a sentence boundary, punctuation, or invalid token breaking coordination.
    Broken,
}

/// Check if a flagged word is part of a valid German backward coordinated compound, reusing scan results.
fn check_backward_coordinated(
    text: &str,
    start: usize,
    end: usize,
    word: &str,
    lang: wordcraft_proof::ProofLang,
    opts: &wordcraft_proof::Options,
    state: &mut BackwardScanState,
) -> bool {
    let Some(before_word) = text.get(..start) else {
        return false;
    };
    let Some(hyphen_char) = before_word.chars().next_back().filter(|c| is_hyphen(*c)) else {
        return false;
    };
    let before_hyphen_offset = start.saturating_sub(hyphen_char.len_utf8());
    let Some(before_hyphen) = text.get(..before_hyphen_offset) else {
        return false;
    };
    let is_boundary = before_hyphen.chars().next_back().is_some_and(|c| c.is_whitespace() || c == '/');
    if !is_boundary {
        return false;
    }

    if state.partner.is_none() && before_hyphen_offset <= state.scanned_up_to && state.scanned_up_to > 0 {
        state.last_issue_end = end;
        return false;
    }

    let stop_offset = if state.last_issue_end > 0 { state.last_issue_end } else { state.scanned_up_to };
    let (_, result) = scan_backward_partner(text, before_hyphen_offset, stop_offset);

    match result {
        BackwardScanResult::Found(partner) => {
            state.partner = Some(partner.clone());
            state.last_issue_end = end;
            test_compound_backward(&partner, word, lang, opts)
        }
        BackwardScanResult::ReachedStop => {
            state.last_issue_end = end;
            if let Some(ref partner) = state.partner {
                test_compound_backward(partner, word, lang, opts)
            } else {
                state.scanned_up_to = state.scanned_up_to.max(before_hyphen_offset);
                false
            }
        }
        BackwardScanResult::Broken => {
            state.partner = None;
            state.last_issue_end = end;
            state.scanned_up_to = state.scanned_up_to.max(before_hyphen_offset);
            false
        }
    }
}

/// Scan backward from `before_hyphen_offset` down to `stop_offset` to find the candidate partner word.
///
/// Returns the byte offset down to which the scan reached and the [`BackwardScanResult`].
fn scan_backward_partner(text: &str, before_hyphen_offset: usize, stop_offset: usize) -> (usize, BackwardScanResult) {
    let mut saw_connector = false;
    let mut modifier_count: usize = 0;
    let mut remainder = text.get(..before_hyphen_offset).unwrap_or("");
    let mut broken = false;

    while !remainder.is_empty() && remainder.len() > stop_offset {
        crate::work::steps(1);
        let prev_len = remainder.len();
        remainder = remainder.trim_end();
        if remainder.is_empty() || remainder.len() <= stop_offset {
            break;
        }
        if remainder.ends_with(['.', '!', '?', ';', ':', '\n']) && !ends_with_german_abbreviation(remainder) {
            broken = true;
            break;
        }
        if remainder.ends_with('/') || remainder.ends_with(',') {
            saw_connector = true;
            let last_char_len = remainder.chars().next_back().map_or(1, |c| c.len_utf8());
            remainder = remainder.get(..remainder.len().saturating_sub(last_char_len)).unwrap_or("");
            if remainder.len() >= prev_len {
                broken = true;
                break;
            }
            continue;
        }

        let sep_match = remainder.char_indices().rfind(|(_, c)| c.is_whitespace() || *c == '/' || *c == ',');
        let (token_start, prev_end) = match sep_match {
            Some((idx, c)) => (idx.saturating_add(c.len_utf8()), idx),
            None => (0, 0),
        };
        let Some(raw_token) = remainder.get(token_start..) else {
            broken = true;
            break;
        };
        remainder = remainder.get(..prev_end).unwrap_or("");
        if remainder.len() >= prev_len {
            broken = true;
            break;
        }

        let has_sentence_punct = raw_token.ends_with(['.', '!', '?', ';', ':']) && !is_german_abbreviation(raw_token);

        let token = raw_token.trim_start_matches(|c: char| !c.is_alphabetic() && !is_hyphen(c));
        let token = token.trim_end_matches(|c: char| !c.is_alphabetic() && !is_hyphen(c));
        if token.is_empty() {
            if has_sentence_punct {
                broken = true;
                break;
            }
            continue;
        }

        if token.starts_with(is_hyphen) {
            saw_connector = true;
            if has_sentence_punct {
                broken = true;
                break;
            }
            continue;
        }

        let lower = token.to_lowercase();
        if is_german_connector(&lower) {
            saw_connector = true;
            if has_sentence_punct {
                broken = true;
                break;
            }
            continue;
        }

        if saw_connector {
            if has_sentence_punct {
                broken = true;
                break;
            }
            let partner = token.trim_matches(|c: char| !c.is_alphabetic());
            if partner.chars().count() >= 2 {
                return (remainder.len(), BackwardScanResult::Found(partner.to_string()));
            }

            let is_lowercase = token.chars().next().is_some_and(|c| c.is_lowercase());
            if is_lowercase && modifier_count < 2 {
                modifier_count = modifier_count.saturating_add(1);
                continue;
            }

            broken = true;
            break;
        }

        let is_lowercase = token.chars().next().is_some_and(|c| c.is_lowercase());
        if !has_sentence_punct && is_lowercase && modifier_count < 2 {
            modifier_count = modifier_count.saturating_add(1);
            continue;
        }

        broken = true;
        break;
    }

    if broken { (remainder.len(), BackwardScanResult::Broken) } else { (remainder.len(), BackwardScanResult::ReachedStop) }
}

/// Test whether any valid prefix of `partner` plus `word` forms a valid German compound.
fn test_compound_backward(partner: &str, word: &str, lang: wordcraft_proof::ProofLang, opts: &wordcraft_proof::Options) -> bool {
    let word_lower = lowercase_first(word);
    let word_upper = uppercase_first(word);

    for (k, _) in partner.char_indices().take_while(|&(k, _)| k <= 40) {
        crate::work::steps(1);
        if k < 2 {
            continue;
        }
        let Some(prefix) = partner.get(..k) else {
            continue;
        };
        if prefix.chars().count() < 2 {
            continue;
        }

        let prefix_is_upper = prefix.chars().next().is_some_and(|c| c.is_uppercase());
        let prefix_upper = uppercase_first(prefix);

        let mut candidates = Vec::with_capacity(3);
        if prefix_is_upper {
            candidates.push(format!("{prefix}{word_lower}"));
            candidates.push(format!("{prefix}-{word_upper}"));
            if word != word_lower && word != word_upper {
                candidates.push(format!("{prefix}{word}"));
            }
        } else {
            candidates.push(format!("{prefix}{word_lower}"));
            candidates.push(format!("{prefix_upper}{word_lower}"));
            candidates.push(format!("{prefix}-{word_lower}"));
        }

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

    struct TestDictGuard {
        temp_dir: std::path::PathBuf,
        _lock: std::sync::MutexGuard<'static, ()>,
    }

    impl Drop for TestDictGuard {
        fn drop(&mut self) {
            wordcraft_proof::set_dictionaries_folder(None);
            let _ = std::fs::remove_dir_all(&self.temp_dir);
        }
    }

    /// German coordinated compounds with ellipsis (e.g. `Handlungs- und Zustandssatz`,
    /// `Schul- und Universitätsbetrieb`, or `Wasserverbrauch und -qualität`) are valid
    /// compound forms (§ 98) and must not be flagged as spelling errors (#5903).
    #[test]
    fn german_coordinated_compounds_with_ellipsis_are_accepted() {
        let temp_dir = std::env::temp_dir().join(format!("wordcraft-test-de-{}", std::process::id()));
        let lock = crate::test_dict_lock();
        let _guard = TestDictGuard { temp_dir: temp_dir.clone(), _lock: lock };

        let _ = std::fs::create_dir_all(&temp_dir);
        let aff = "SET UTF-8\nCOMPOUNDFLAG X\n";
        let dic = "38\nHandlung/X\nHandlungssatz/X\nZustandssatz/X\nSchulbetrieb/X\nUniversitätsbetrieb/X\nArbeitsrecht/X\nSozialrecht/X\nFamilienrecht/X\nSteuerrecht/X\nVertragsrecht/X\nVersicherungsrecht/X\nVerkehrsrecht/X\nMietrecht/X\nForschungsabteilung/X\nEntwicklungsabteilung/X\nWirtschaftspolitik/X\nFinanzpolitik/X\nBildungsauftrag/X\nErziehungsauftrag/X\nBewegungsverb/X\nWahrnehmungsverb/X\nOrdnungsdienst/X\nSicherheitsdienst/X\nGestaltungssatz/X\nWasserverbrauch/X\nWasserqualität/X\nFriedensforschung/X\nKonfliktforschung/X\nLiebesbeziehung/X\nEhebeziehung/X\nLebensbedingungen/X\nArbeitsbedingungen/X\nund\noder\nder\nneue\nsehr\ngut\n";
        let _ = std::fs::write(temp_dir.join("de_DE.aff"), aff);
        let _ = std::fs::write(temp_dir.join("de_DE.dic"), dic);
        let _ = std::fs::write(temp_dir.join("de_CH.aff"), aff);
        let _ = std::fs::write(temp_dir.join("de_CH.dic"), dic);
        wordcraft_proof::set_dictionaries_folder(Some(temp_dir));

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
            "Arbeits-, Sozial-, Familien-, Steuer-, Vertrags-, Versicherungs-, Verkehrs- und Mietrecht",
            "der Schul- und der neue Universitätsbetrieb",
        ];

        for text in cases {
            let mut d = Document::from_text(text);
            d.format_range(&wordcraft_doc::Pos::body(0, 0), &wordcraft_doc::Pos::body(0, text.len()), &|c| c.lang = Some("de-DE".into())).unwrap();
            let p = d.para(wordcraft_doc::StoryRef::Body, &wordcraft_doc::Path::top(0)).unwrap();
            let issues = paragraph_issues(p, &proof_runs(&d, p, None), &Default::default(), true, false);
            let spelling_issues: Vec<_> = issues.iter().filter(|i| i.kind == wordcraft_proof::IssueKind::Spelling).collect();
            assert!(spelling_issues.is_empty(), "expected no spelling issues for {text:?}, got: {spelling_issues:?}");
        }

        // German regional variants such as Swiss and Liechtenstein German (de-CH, de-LI)
        // must also go through the filter.
        for lang_tag in ["de-CH", "de-LI"] {
            let text = "Handlungs- und Zustandssatz";
            let mut d = Document::from_text(text);
            d.format_range(&wordcraft_doc::Pos::body(0, 0), &wordcraft_doc::Pos::body(0, text.len()), &|c| c.lang = Some(lang_tag.into())).unwrap();
            let p = d.para(wordcraft_doc::StoryRef::Body, &wordcraft_doc::Path::top(0)).unwrap();
            let issues = paragraph_issues(p, &proof_runs(&d, p, None), &Default::default(), true, false);
            let spelling_issues: Vec<_> = issues.iter().filter(|i| i.kind == wordcraft_proof::IssueKind::Spelling).collect();
            assert!(spelling_issues.is_empty(), "expected no spelling issues for {lang_tag} on {text:?}, got: {spelling_issues:?}");
        }

        // Coordinated lists beyond the previous 64-token scan limit are supported.
        let long_list = format!("{}und Sozialrecht", "Arbeits-, ".repeat(70));
        let mut d = Document::from_text(&long_list);
        d.format_range(&wordcraft_doc::Pos::body(0, 0), &wordcraft_doc::Pos::body(0, long_list.len()), &|c| c.lang = Some("de-DE".into())).unwrap();
        let p = d.para(wordcraft_doc::StoryRef::Body, &wordcraft_doc::Path::top(0)).unwrap();
        let issues = paragraph_issues(p, &proof_runs(&d, p, None), &Default::default(), true, false);
        let spelling_issues: Vec<_> = issues.iter().filter(|i| i.kind == wordcraft_proof::IssueKind::Spelling).collect();
        assert!(spelling_issues.is_empty(), "expected long coordinated list beyond 64 tokens to be accepted, got: {spelling_issues:?}");

        // Backward coordinated lists beyond 64 tokens are also supported without unbounded work.
        let long_backward_list = format!("Wasserverbrauch, {}und -qualität", "-qualität, ".repeat(70));
        let mut d = Document::from_text(&long_backward_list);
        d.format_range(&wordcraft_doc::Pos::body(0, 0), &wordcraft_doc::Pos::body(0, long_backward_list.len()), &|c| c.lang = Some("de-DE".into()))
            .unwrap();
        let p = d.para(wordcraft_doc::StoryRef::Body, &wordcraft_doc::Path::top(0)).unwrap();
        let issues = paragraph_issues(p, &proof_runs(&d, p, None), &Default::default(), true, false);
        let spelling_issues: Vec<_> = issues.iter().filter(|i| i.kind == wordcraft_proof::IssueKind::Spelling).collect();
        assert!(spelling_issues.is_empty(), "expected long backward coordinated list to be accepted, got: {spelling_issues:?}");

        // Sentence boundaries on modifiers must not allow matching across sentences.
        let invalid_sentence = "Handlungs- und sehr gut. Der Zustandssatz.";
        let mut d = Document::from_text(invalid_sentence);
        d.format_range(&wordcraft_doc::Pos::body(0, 0), &wordcraft_doc::Pos::body(0, invalid_sentence.len()), &|c| c.lang = Some("de-DE".into()))
            .unwrap();
        let p = d.para(wordcraft_doc::StoryRef::Body, &wordcraft_doc::Path::top(0)).unwrap();
        let issues = paragraph_issues(p, &proof_runs(&d, p, None), &Default::default(), true, false);
        assert!(
            issues.iter().any(|i| i.kind == wordcraft_proof::IssueKind::Spelling && p.text.get(i.start..i.end) == Some("Handlungs")),
            "expected Handlungs- not to match across sentence boundaries, got issues: {issues:?}"
        );

        // Invalid compounds must still be flagged.
        let invalid = "Xyzqwe- und Zustandssatz";
        let mut d = Document::from_text(invalid);
        d.format_range(&wordcraft_doc::Pos::body(0, 0), &wordcraft_doc::Pos::body(0, invalid.len()), &|c| c.lang = Some("de-DE".into())).unwrap();
        let p = d.para(wordcraft_doc::StoryRef::Body, &wordcraft_doc::Path::top(0)).unwrap();
        let issues = paragraph_issues(p, &proof_runs(&d, p, None), &Default::default(), true, false);
        assert!(!issues.is_empty(), "expected invalid compound to be flagged");
    }

    /// Words ending with abbreviation letters (e.g. `Mallorca.` ending in `ca.`) must not be treated as abbreviations.
    #[test]
    fn words_ending_with_abbreviation_letters_are_not_abbreviations() {
        assert!(!ends_with_german_abbreviation("Wir fliegen nach Mallorca."));
        assert!(!starts_with_german_abbreviation("Mallorca. Wir fliegen"));
        assert!(ends_with_german_abbreviation("ca."));
        assert!(ends_with_german_abbreviation("Test ca."));
        assert!(starts_with_german_abbreviation("bzw. Test"));
    }

    /// Repeated rejected hyphenated tokens must take linear work ([`crate::work`]) in [`paragraph_issues`].
    #[test]
    fn repeated_rejected_hyphenated_tokens_stay_linear() {
        let temp_dir = std::env::temp_dir().join(format!("wordcraft-test-linear-{}", std::process::id()));
        let lock = crate::test_dict_lock();
        let _guard = TestDictGuard { temp_dir: temp_dir.clone(), _lock: lock };

        let _ = std::fs::create_dir_all(&temp_dir);
        let aff = "SET UTF-8\n";
        let long_partner = "Donaudampfschiffahrtsgesellschaftskapitaenswitwenrentenversicherungsgrundgesetzabwicklungsabteilungsleiter";
        let dic = format!("2\nund\n{long_partner}\n");
        let _ = std::fs::write(temp_dir.join("de_DE.aff"), aff);
        let _ = std::fs::write(temp_dir.join("de_DE.dic"), dic);
        wordcraft_proof::set_dictionaries_folder(Some(temp_dir));

        let make_doc_forward = |n: usize| {
            let text = format!("{}und {long_partner}", "Xyzq- ".repeat(n));
            let mut d = Document::from_text(&text);
            d.format_range(&wordcraft_doc::Pos::body(0, 0), &wordcraft_doc::Pos::body(0, text.len()), &|c| c.lang = Some("de-DE".into())).unwrap();
            d
        };

        let make_doc_backward = |n: usize| {
            let text = format!("{long_partner} und {}", "-Xyzq ".repeat(n));
            let mut d = Document::from_text(&text);
            d.format_range(&wordcraft_doc::Pos::body(0, 0), &wordcraft_doc::Pos::body(0, text.len()), &|c| c.lang = Some("de-DE".into())).unwrap();
            d
        };

        let run_proof = |d: &Document| {
            let p = d.para(wordcraft_doc::StoryRef::Body, &wordcraft_doc::Path::top(0)).unwrap();
            let runs = proof_runs(d, p, None);
            paragraph_issues(p, &runs, &Default::default(), false, false)
        };

        // Forward coordinated compounds stay linear.
        let (small_f, big_f) = (make_doc_forward(50), make_doc_forward(400));
        let (sf, bf) = (
            crate::work::count(|| {
                let issues = run_proof(&small_f);
                assert_eq!(issues.len(), 50, "expected 50 rejected forward issues for small document");
            }),
            crate::work::count(|| {
                let issues = run_proof(&big_f);
                assert_eq!(issues.len(), 400, "expected 400 rejected forward issues for big document");
            }),
        );
        let ratio_f = bf as f64 / sf.max(1) as f64;
        assert!(sf > 0, "no steps counted, so work is not being measured");
        assert!(ratio_f <= 12.0, "repeated forward rejected hyphenated tokens took {ratio_f:.1}x steps (8x input: {sf} -> {bf}), not linear");

        // Backward coordinated compounds with preceding partner stay linear.
        let (small_b, big_b) = (make_doc_backward(50), make_doc_backward(400));
        let (sb, bb) = (
            crate::work::count(|| {
                let issues = run_proof(&small_b);
                assert_eq!(issues.len(), 50, "expected 50 rejected backward issues for small document");
            }),
            crate::work::count(|| {
                let issues = run_proof(&big_b);
                assert_eq!(issues.len(), 400, "expected 400 rejected backward issues for big document");
            }),
        );
        let ratio_b = bb as f64 / sb.max(1) as f64;
        assert!(sb > 0, "no steps counted, so work is not being measured");
        assert!(ratio_b <= 12.0, "repeated backward rejected hyphenated tokens took {ratio_b:.1}x steps (8x input: {sb} -> {bb}), not linear");
    }

    /// Candidate loops in [`test_compound_forward`] and [`test_compound_backward`] must remain bounded
    /// to the 40-byte window rather than scanning the full length of a long partner.
    #[test]
    fn partner_candidate_loops_stay_bounded_on_long_partners() {
        let temp_dir = std::env::temp_dir().join(format!("wordcraft-test-bounded-{}", std::process::id()));
        let lock = crate::test_dict_lock();
        let _guard = TestDictGuard { temp_dir: temp_dir.clone(), _lock: lock };

        let _ = std::fs::create_dir_all(&temp_dir);
        let aff = "SET UTF-8\n";
        let short_partner = "Rechtsanwalt";
        let long_partner = "Donaudampfschiffahrtsgesellschaftskapitaenswitwenrentenversicherungsgrundgesetzabwicklungsabteilungsleiter";
        let dic = format!("3\nund\n{short_partner}\n{long_partner}\n");
        let _ = std::fs::write(temp_dir.join("de_DE.aff"), aff);
        let _ = std::fs::write(temp_dir.join("de_DE.dic"), dic);
        wordcraft_proof::set_dictionaries_folder(Some(temp_dir));

        let opts = wordcraft_proof::Options::default();
        let lang = wordcraft_proof::ProofLang::De;

        let steps_short_b = crate::work::count(|| {
            assert!(!test_compound_backward(short_partner, "Xyzq", lang, &opts));
        });
        let steps_long_b = crate::work::count(|| {
            assert!(!test_compound_backward(long_partner, "Xyzq", lang, &opts));
        });
        assert!(steps_short_b > 0, "work must be counted in test_compound_backward");
        assert!(steps_long_b <= 45, "test_compound_backward scanned beyond the 40-byte candidate window: {steps_long_b} steps");

        let steps_short_f = crate::work::count(|| {
            assert!(!test_compound_forward("Xyzq", short_partner, lang, &opts));
        });
        let steps_long_f = crate::work::count(|| {
            assert!(!test_compound_forward("Xyzq", long_partner, lang, &opts));
        });
        assert!(steps_short_f > 0, "work must be counted in test_compound_forward");
        assert!(steps_long_f <= 45, "test_compound_forward scanned beyond the 40-byte candidate window: {steps_long_f} steps");
    }
}
