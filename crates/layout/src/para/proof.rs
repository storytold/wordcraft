use super::{CharProps, Document, Paragraph, ResolvedChar};

/// One run's proofing: its bytes in the paragraph, its language (`w:lang`, resolved through the
/// styles: the document default, paragraph and character styles, direct formatting) and whether
/// it is left unchecked ("Do not check spelling or grammar", hidden text, hyperlinks).
#[derive(Clone, Debug, PartialEq)]
pub struct ProofRun {
    pub range: std::ops::Range<usize>,
    pub lang: wordcraft_proof::ProofLang,
    /// The language tag as the document gives it (an external checker is asked in it).
    pub tag: Option<String>,
    pub skip: bool,
}

impl ProofRun {
    pub(super) fn of(range: std::ops::Range<usize>, rc: &ResolvedChar) -> ProofRun {
        ProofRun {
            range,
            lang: wordcraft_proof::ProofLang::from_tag(rc.lang.as_deref()),
            tag: rc.lang.clone(),
            skip: rc.no_proof || rc.hidden || rc.link.is_some(),
        }
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
    external_language: &str,
    spelling: &wordcraft_proof::Options,
    grammar: bool,
    list_item: bool,
) -> Vec<wordcraft_proof::Issue> {
    if p.text.trim().is_empty() || p.text.len() > 100_000 {
        return Vec::new();
    }
    let text = proof_text(p);
    let spans: Vec<(std::ops::Range<usize>, wordcraft_proof::ProofLang)> = runs.iter().map(|r| (r.range.clone(), r.lang)).collect();
    let skip = |a: usize, b: usize| runs.iter().any(|r| r.skip && r.range.start < b && a < r.range.end);
    // The external checker's result for this text when it has one (#575), each issue in the
    // language of the run it is in; grammar marks only with `grammar`.
    if let Some(found) = wordcraft_proof::check_external(&text, external_language) {
        let lang_at = |at: usize| spans.iter().find(|(r, _)| r.contains(&at)).map(|(_, l)| *l).unwrap_or_default();
        return found
            .into_iter()
            .filter(|i| !skip(i.start, i.end) && (grammar || i.kind != wordcraft_proof::IssueKind::Grammar))
            .filter(|i| {
                i.kind != wordcraft_proof::IssueKind::Spelling || !wordcraft_proof::spelling_ignored_by_options(&text, i.start, i.end, spelling)
            })
            .map(|i| wordcraft_proof::Issue { lang: lang_at(i.start), ..i })
            .collect();
    }
    wordcraft_proof::check_text_with(&text, &spans, spelling, grammar, list_item).into_iter().filter(|i| !skip(i.start, i.end)).collect()
}

/// The key an external checker's results for `p` are stored under: the setting, or the language
/// of its first run, and, when the runs carry different language tags, where each tag applies
/// ([`language_key`]), so the same text with other language marks is checked again.
pub fn external_language(doc: &Document, p: &Paragraph) -> String {
    let plain = CharProps::default();
    let first = p.run_ranges().next().map(|(_, c)| c).unwrap_or(&plain);
    let base = wordcraft_proof::external::wanted(doc.styles.resolve_char(p.props.style.as_deref(), first).lang.as_deref());
    let runs = proof_runs(doc, p, None);
    match runs.first() {
        Some(f) if runs.iter().any(|r| r.tag != f.tag) => format!("{base}|{}", language_key(&runs)),
        _ => base,
    }
}

/// The language tags of `runs` with the bytes each covers (adjacent runs of one tag merged).
pub fn language_key(runs: &[ProofRun]) -> String {
    let mut spans: Vec<(&str, usize, usize)> = Vec::new();
    for r in runs {
        let tag = r.tag.as_deref().unwrap_or("");
        match spans.last_mut() {
            Some((t, _, end)) if *t == tag && *end == r.range.start => *end = r.range.end,
            _ => spans.push((tag, r.range.start, r.range.end)),
        }
    }
    spans.iter().map(|(t, a, b)| format!("{t}@{a}-{b}")).collect::<Vec<_>>().join(",")
}

/// Whether the paragraph is a list item: its own numbering, or its style's, names a list (not 0).
pub fn is_list_item(doc: &Document, p: &Paragraph) -> bool {
    p.props.numbering.or_else(|| doc.styles.resolve_para(&p.props).numbering).is_some_and(|n| n.num != 0)
}

/// Spelling and grammar issues as squiggles (start, end, is grammar), less spelling issues overlapping `typing`.
pub(super) fn proof_issues(
    p: &Paragraph,
    runs: &[ProofRun],
    external_language: &str,
    opts: &crate::ProofOptions,
    typing: Option<(usize, usize)>,
    list_item: bool,
) -> Vec<(usize, usize, bool)> {
    paragraph_issues(p, runs, external_language, &opts.spelling, opts.grammar, list_item)
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
    for (i, c) in p.text.char_indices() {
        while run.is_some() && i >= end {
            run = runs.next();
            end = end.saturating_add(run.map_or(0, |r| r.len));
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
