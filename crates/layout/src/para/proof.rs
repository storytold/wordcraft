use super::{CharProps, Document, Paragraph, ResolvedChar};

/// One run's proofing: its bytes in the paragraph, its language (`w:lang`, resolved through the
/// styles: the document default, paragraph and character styles, direct formatting) and whether
/// it is left unchecked ("Do not check spelling or grammar", hidden text, hyperlinks).
#[derive(Clone, Debug, PartialEq)]
pub struct ProofRun {
    pub range: std::ops::Range<usize>,
    pub lang: wordcraft_proof::ProofLang,
    /// Resolved East Asian language; `None` still leaves CJK text out of English proofing.
    pub lang_ea: Option<wordcraft_proof::ProofLang>,
    pub skip: bool,
}

impl ProofRun {
    pub(super) fn of(range: std::ops::Range<usize>, rc: &ResolvedChar) -> ProofRun {
        ProofRun {
            range,
            lang: wordcraft_proof::ProofLang::from_tag(rc.lang.as_deref()),
            lang_ea: rc.lang_ea.as_deref().map(|tag| wordcraft_proof::ProofLang::from_tag(Some(tag))),
            skip: rc.no_proof || rc.hidden || rc.link.is_some(),
        }
    }
}

/// The proofing runs of a paragraph, in order. `table` is the cell's table-style character
/// formatting ([`CellText::chr`]) for a paragraph in a table, as layout resolves it.
pub fn proof_runs(doc: &Document, p: &Paragraph, table: Option<&CharProps>) -> Vec<ProofRun> {
    let style = p.props.style.as_deref();
    let raw: Vec<_> = p.run_ranges().map(|(r, c)| ProofRun::of(r, &doc.styles.resolve_char_in(style, table, c))).collect();
    scripted_runs(&p.text, &raw)
}

/// Assign each character its script's proofing language. Spaces and punctuation touching
/// Japanese text inherit its language, so English double-space grammar does not mark form fields.
pub(super) fn scripted_runs(text: &str, raw: &[ProofRun]) -> Vec<ProofRun> {
    // OOXML stores Western and East Asian proofing languages separately. A Japanese form
    // commonly has en-US as its Western default and ja-JP for East Asian text; treating
    // its Japanese spacing as English produces spurious blue double grammar lines.
    // Proofing skips paragraphs above this limit, so avoid expanding those into chars.
    if text.len() > 100_000 {
        return raw.to_vec();
    }
    let mut chars = Vec::new();
    for run in raw {
        for (offset, ch) in text.get(run.range.clone()).unwrap_or("").char_indices() {
            let start = run.range.start + offset;
            let script = if super::grid::is_east_asian(ch) {
                Some(true)
            } else if ch.is_alphabetic() {
                Some(false)
            } else {
                None
            };
            chars.push((start..start + ch.len_utf8(), run.lang, run.lang_ea, run.skip, script));
        }
    }
    let mut right = vec![None; chars.len()];
    let mut next = None;
    for (i, (_, _, _, _, script)) in chars.iter().enumerate().rev() {
        if let Some(slot) = right.get_mut(i) {
            *slot = next;
        }
        if script.is_some() {
            next = *script;
        }
    }
    let mut runs: Vec<ProofRun> = Vec::new();
    let mut previous_script = None;
    for (i, (range, base, east, skip, script)) in chars.into_iter().enumerate() {
        // A neutral character between Japanese words belongs to the Japanese span;
        // double spaces between Latin words still receive their normal proofing.
        let japanese = script.unwrap_or(previous_script == Some(true) || right.get(i).copied().flatten() == Some(true));
        if script.is_some() {
            previous_script = script;
        }
        let lang = if japanese { east.unwrap_or(wordcraft_proof::ProofLang::Other) } else { base };
        match runs.last_mut() {
            Some(prev) if prev.range.end == range.start && prev.lang == lang && prev.skip == skip => prev.range.end = range.end,
            _ => runs.push(ProofRun { range, lang, lang_ea: east, skip }),
        }
    }
    runs
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
    let skip = |a: usize, b: usize| {
        let k = runs.partition_point(|r| r.range.end <= a);
        runs.get(k..).unwrap_or(&[]).iter().take_while(|r| r.range.start < b).any(|r| r.skip)
    };
    wordcraft_proof::check_text_with(&text, &spans, spelling, grammar, list_item).into_iter().filter(|i| !skip(i.start, i.end)).collect()
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
    for (i, c) in p.text.char_indices() {
        crate::work::steps(1); // Inspect this character.
        while run.is_some() && i >= end {
            crate::work::steps(1); // Advance past this run.
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
