//! Citation finder and form checker (suggestions only; never claims an authority exists).

use std::collections::HashMap;

use wordcraft_doc::{Document, Pos, StoryRef};
use wordcraft_engine::CmdError;
use wordcraft_engine::lint::{Finding, MAX_FINDINGS};

use crate::citation_pack::CompiledPack;

/// Most citation occurrences a single finder scan may collect. The scan reads the document, which
/// is input-derived and therefore hostile: it is bounded so a runaway or crafted document cannot
/// make the table of authorities allocate without limit. Far above any real brief.
pub const MAX_CITATION_OCCURRENCES: usize = 5_000;
/// Most distinct authorities a single table may hold. Together with [`MAX_CITATION_OCCURRENCES`]
/// this bounds the generated table: its lines, bookmarks and `PAGEREF` fields.
pub const MAX_CITATION_ENTRIES: usize = 2_000;

/// One logical authority in the document (first full cite + linked short forms).
#[derive(Clone, Debug, PartialEq)]
pub struct CitationEntry {
    pub id: u32,
    pub kind: String,
    /// Full citation text at first occurrence.
    pub label: String,
    /// Sort key (full citation text at first occurrence, same as the label for sorting).
    pub sort_key: String,
    pub occurrences: Vec<CitationOccurrence>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CitationOccurrence {
    pub start: Pos,
    pub end: Pos,
}

struct RawHit {
    kind: String,
    text: String,
    start: Pos,
    end: Pos,
}

/// Map a pack rule `kind` to a table-of-authorities group heading.
pub fn group_heading(kind: &str) -> &'static str {
    match kind {
        "case" => "Cases",
        "constitutional" | "constitution" => "Constitutional Provisions",
        "statute" => "Statutes",
        "rule" => "Rules",
        _ => "Other Authorities",
    }
}

/// Scan `doc` with `pack` and return authority entries, bounded by the production limits. The
/// document is input-derived, so the result count is capped; see [`find_entries_limited`].
///
/// Every match the pack's existing finder rules report is used as-is: a hit is one authority, and
/// hits whose text and kind are identical collapse into one entry (its first hit supplies the
/// label). No new detection or linking happens here.
pub fn find_entries(doc: &Document, pack: &CompiledPack) -> Result<Vec<CitationEntry>, CmdError> {
    find_entries_limited(doc, pack, MAX_CITATION_OCCURRENCES, MAX_CITATION_ENTRIES)
}

/// [`find_entries`] with explicit caps, so the bounds can be exercised without a huge document.
///
/// Only the main text is scanned (WordCraft's table of authorities lists the body's marks), and
/// generated list entries (a table of authorities' own lines) are skipped. When the scan collects more than `max_occurrences`
/// matches, or the collapse leaves more than `max_entries` authorities, it stops with a clear
/// error rather than allocating without limit. Nothing is written to the document.
pub(crate) fn find_entries_limited(
    doc: &Document,
    pack: &CompiledPack,
    max_occurrences: usize,
    max_entries: usize,
) -> Result<Vec<CitationEntry>, CmdError> {
    let mut raw = Vec::new();
    let story = StoryRef::Body;
    {
        for path in doc.para_paths(story) {
            let Some(para) = doc.para(story, &path) else { continue };
            if para.props.style.as_deref() == Some(wordcraft_engine::cmd::citations::ENTRY_STYLE) {
                continue;
            }
            if para.text.is_empty() {
                continue;
            }
            let plain = plain_spaces(&para.text);
            let text = plain.as_ref().map_or(para.text.as_str(), |(t, _)| t.as_str());
            let orig = |off: usize| plain.as_ref().map_or(off, |(_, map)| map[off]);
            for rule in &pack.rules {
                if !rule.finder {
                    continue;
                }
                for m in rule.detect.find_iter(text) {
                    let end = if rule.extend_tail { extend_span(text, m.end()) } else { m.end() };
                    let slice = text.get(m.start()..end).map_or(m.as_str(), |s| s);
                    if rule.valid.as_ref().is_some_and(|v| !v.is_match(slice)) {
                        continue;
                    }
                    let start = Pos { story, path: path.clone(), off: orig(m.start()) };
                    let end_pos = Pos { story, path: path.clone(), off: orig(end) };
                    raw.push(RawHit { kind: rule.kind.clone(), text: slice.to_string(), start, end: end_pos });
                    if raw.len() > max_occurrences {
                        return Err(too_many("citation occurrences", raw.len(), max_occurrences));
                    }
                }
            }
        }
    }
    raw = dedupe_overlaps(raw);
    raw.sort_by_key(|h| pos_order(&h.start));

    let mut next_id = 1u32;
    let mut key_to_id: HashMap<String, u32> = HashMap::new();
    let mut map: HashMap<u32, CitationEntry> = HashMap::new();
    for h in raw {
        let key = format!("{}:{}", h.kind, h.text.trim().to_ascii_lowercase());
        let id = *key_to_id.entry(key).or_insert_with(|| {
            let id = next_id;
            next_id += 1;
            id
        });
        let entry = map.entry(id).or_insert_with(|| CitationEntry {
            id,
            kind: h.kind.clone(),
            label: h.text.trim().to_string(),
            sort_key: h.text.trim().to_string(),
            occurrences: Vec::new(),
        });
        entry.occurrences.push(CitationOccurrence { start: h.start, end: h.end });
    }
    if map.len() > max_entries {
        return Err(too_many("authorities", map.len(), max_entries));
    }
    let mut out: Vec<CitationEntry> = map.into_values().collect();
    out.sort_by_key(|e| e.id);
    Ok(out)
}

fn too_many(what: &str, got: usize, max: usize) -> CmdError {
    CmdError::Failed(format!("too many {what} in the document ({got}, at most {max}); narrow the document or turn off the table of authorities"))
}

fn pos_order(start: &Pos) -> (i64, Vec<u32>, usize) {
    (story_rank(start.story), start.path.0.clone(), start.off)
}

fn para_key(story: StoryRef, path: &wordcraft_doc::Path) -> (i64, Vec<u32>) {
    (story_rank(story), path.0.clone())
}

fn story_rank(story: StoryRef) -> i64 {
    match story {
        StoryRef::Body => 0,
        StoryRef::Part(id) => id as i64 + 1,
    }
}

fn dedupe_overlaps(hits: Vec<RawHit>) -> Vec<RawHit> {
    let mut hits = hits;
    hits.sort_by(|a, b| b.text.len().cmp(&a.text.len()).then_with(|| pos_order(&a.start).cmp(&pos_order(&b.start))));
    let mut taken: HashMap<(i64, Vec<u32>), Vec<(usize, usize)>> = HashMap::new();
    let mut out = Vec::new();
    for h in hits {
        let key = para_key(h.start.story, &h.start.path);
        let ranges = taken.entry(key).or_default();
        if ranges.iter().any(|(a, b)| h.start.off < *b && h.end.off > *a) {
            continue;
        }
        ranges.push((h.start.off, h.end.off));
        out.push(h);
    }
    out.sort_by_key(|h| pos_order(&h.start));
    out
}

/// Extend a match through optional pin cites (including page ranges) and a parenthetical so
/// validation sees the full cite.
fn extend_span(text: &str, end: usize) -> usize {
    let bytes = text.as_bytes();
    let len = bytes.len();
    let mut pos = end.min(len);
    while pos < len {
        let mut i = pos;
        while i < len && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= len {
            pos = i;
            break;
        }
        if bytes[i] == b',' {
            i += 1;
            while i < len && bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            let digit_start = i;
            while i < len && bytes[i].is_ascii_digit() {
                i += 1;
            }
            if i == digit_start {
                break;
            }
            pos = extend_page_range(bytes, i);
            continue;
        }
        if bytes[i] == b'(' {
            i += 1;
            while i < len && bytes[i] != b')' {
                i += 1;
            }
            if i < len && bytes[i] == b')' {
                i += 1;
            }
            pos = i;
            break;
        }
        break;
    }
    pos
}

/// After a pin-cite page ends at `from`, consume a `-`, `–` or `—` page range and its end page.
fn extend_page_range(bytes: &[u8], from: usize) -> usize {
    let len = bytes.len();
    let dash_end = if bytes.get(from) == Some(&b'-') {
        from + 1
    } else if bytes.get(from) == Some(&0xE2) && bytes.get(from + 1) == Some(&0x80) && matches!(bytes.get(from + 2), Some(&0x93) | Some(&0x94)) {
        from + 3
    } else {
        return from;
    };
    let mut i = dash_end;
    while i < len && bytes[i].is_ascii_whitespace() {
        i += 1;
    }
    let digit_start = i;
    while i < len && bytes[i].is_ascii_digit() {
        i += 1;
    }
    if i == digit_start { from } else { i }
}

/// `text` with every non-ASCII space (a nonbreaking space between `S.W.` and `3d`, a thin or
/// narrow space) read as a plain space, so a rule's literal space and the pin-cite scan match it
/// as a reader would. Returns the text to scan and, for each of its byte offsets (and its end),
/// the offset in `text`; `None` when there is nothing to change.
fn plain_spaces(text: &str) -> Option<(String, Vec<usize>)> {
    let odd = |c: char| !c.is_ascii() && c.is_whitespace();
    if !text.chars().any(odd) {
        return None;
    }
    let mut out = String::with_capacity(text.len());
    let mut map = Vec::with_capacity(text.len() + 1);
    for (i, c) in text.char_indices() {
        let c = if odd(c) { ' ' } else { c };
        for _ in 0..c.len_utf8() {
            map.push(i);
        }
        out.push(c);
    }
    map.push(text.len());
    Some((out, map))
}

/// Scan the document with `pack` and return form suggestions.
pub fn check_document(doc: &Document, pack: &CompiledPack) -> Vec<Finding> {
    let mut out = Vec::new();
    for story in doc.counted_stories() {
        for path in doc.para_paths(story) {
            let Some(para) = doc.para(story, &path) else { continue };
            if para.text.is_empty() {
                continue;
            }
            let plain = plain_spaces(&para.text);
            let text = plain.as_ref().map_or(para.text.as_str(), |(t, _)| t.as_str());
            let orig = |off: usize| plain.as_ref().map_or(off, |(_, map)| map[off]);
            for rule in &pack.rules {
                for m in rule.detect.find_iter(text) {
                    let end = if rule.extend_tail { extend_span(text, m.end()) } else { m.end() };
                    let slice = text.get(m.start()..end).map_or(m.as_str(), |s| s);
                    if rule.valid.as_ref().is_some_and(|v| v.is_match(slice)) {
                        continue;
                    }
                    let start = Pos { story, path: path.clone(), off: orig(m.start()) };
                    let end_pos = Pos { story, path: path.clone(), off: orig(end) };
                    let msg = format!("{} ({})", rule.message, rule.source);
                    out.push(Finding::at(rule.id.clone(), "warning", msg, start, end_pos));
                    if out.len() >= MAX_FINDINGS {
                        return out;
                    }
                }
            }
        }
    }
    out
}

/// Findings as JSON (same shape as court checks).
pub fn findings_json(findings: &[Finding]) -> Vec<serde_json::Value> {
    findings.iter().map(|f| wordcraft_engine::lint::finding_json(f, wordcraft_engine::cmd::pos_json)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::citation_pack::bundled_map;
    use serde_json::json;
    use wordcraft_engine::Session;

    fn check_text(pack_id: &str, text: &str) -> Vec<String> {
        let packs = bundled_map();
        let pack = packs.get(pack_id).expect("pack");
        let mut s = Session::new(wordcraft_doc::Document::new());
        s.run("text.insert", &json!({"text": text})).expect("insert");
        check_document(&s.doc, pack).into_iter().map(|f| f.check_id).collect()
    }

    fn findings(pack_id: &str, text: &str) -> Vec<Finding> {
        let packs = bundled_map();
        let pack = packs.get(pack_id).expect("pack");
        let mut s = Session::new(wordcraft_doc::Document::new());
        s.run("text.insert", &json!({"text": text})).expect("insert");
        check_document(&s.doc, pack)
    }

    #[test]
    fn extend_pin_without_parenthetical_is_flagged() {
        let ids = check_text("missouri", "Thummel v. King, 570 S.W.2d 679, 680");
        assert!(ids.iter().any(|id| id == "moCaseParenthetical"));
    }

    #[test]
    fn non_adversarial_case_missing_parenthetical_is_flagged() {
        for cite in ["In re Smith, 570 S.W.2d 679", "Ex parte Jones, 600 S.W.3d 100", "State ex rel. Roe, 590 S.W.3d 200"] {
            let ids = check_text("missouri", cite);
            assert!(ids.iter().any(|id| id == "moCaseParenthetical"), "`{cite}` must be flagged: {ids:?}");
        }
    }

    #[test]
    fn non_adversarial_case_with_parenthetical_is_ok() {
        let ids = check_text("missouri", "In re Smith, 570 S.W.2d 679 (Mo. banc 1978)");
        assert!(!ids.iter().any(|id| id == "moCaseParenthetical"), "unexpected: {ids:?}");
    }

    #[test]
    fn pin_page_range_is_checked_as_one_span() {
        let text = "Thummel v. King, 570 S.W.2d 679, 680-681 (Mo. banc 1978)";
        let fs = findings("missouri", text);
        assert!(!fs.iter().any(|f| f.check_id == "moCaseParenthetical"), "{fs:?}");

        let bad = "Thummel v. King, 570 S.W.2d 679, 680-681";
        let fs = findings("missouri", bad);
        let f = fs.iter().find(|f| f.check_id == "moCaseParenthetical").expect("flagged with a pin range");
        let end = f.end.as_ref().expect("span end").off;
        assert_eq!(end, bad.len(), "the span must reach the end of the pin range");
    }

    #[test]
    fn double_space_before_parenthetical_is_ok() {
        let ids = check_text("missouri", "Thummel v. King, 570 S.W.2d 679  (Mo. banc 1978)");
        assert!(!ids.iter().any(|id| id == "moCaseParenthetical"));
    }

    #[test]
    fn nonbreaking_spaces_read_as_spaces() {
        // A correct cite typed with nonbreaking spaces is still correct.
        let good = "Thummel v. King, 570\u{a0}S.W.2d\u{a0}679,\u{a0}680 (Mo.\u{a0}banc 1978)";
        let ids = check_text("missouri", good);
        assert!(!ids.iter().any(|id| id == "moCaseParenthetical"), "{ids:?}");
        let ids = check_text("federal", "Smith v. Jones, 100 F.\u{a0}Supp.\u{a0}3d 200 (D.\u{a0}Mo. 2015)");
        assert!(!ids.iter().any(|id| id == "federalCourtParen"), "{ids:?}");
        // A rule written with a plain space still finds the cite typed with nonbreaking ones.
        let ids = check_text("federal", "Smith v. Jones, 100 F.\u{a0}Supp.\u{a0}3d 200");
        assert!(ids.iter().any(|id| id == "federalCourtParen"), "{ids:?}");
        // A bad one is flagged, and the span lands on the original text's offsets.
        let bad = "See\u{a0}Thummel v. King, 570\u{a0}S.W.2d 679,\u{a0}680";
        let fs = findings("missouri", bad);
        let f = fs.iter().find(|f| f.check_id == "moCaseParenthetical").expect("flagged");
        assert_eq!(f.end.as_ref().expect("end").off, bad.len());
        let start = f.start.as_ref().expect("start").off;
        assert!(bad.is_char_boundary(start) && bad[start..].starts_with("570"), "{start}");
    }

    #[test]
    fn mo_banc_without_year_is_flagged() {
        let ids = check_text("missouri", "Thummel v. King, 570 S.W.2d 679 (Mo. banc)");
        assert!(ids.iter().any(|id| id == "moBancMissingYear"));
    }

    #[test]
    fn federal_pin_without_parenthetical_is_flagged() {
        let ids = check_text("federal", "Smith v. Jones, 567 F.3d 890, 891");
        assert!(ids.iter().any(|id| id == "federalCourtParen"));
    }

    #[test]
    fn us_without_parenthetical_is_flagged() {
        let ids = check_text("federal", "Brown v. Board, 347 U.S. 483");
        assert!(ids.iter().any(|id| id == "usReporter"));
    }

    #[test]
    fn s_ct_spacing_without_space_is_flagged() {
        let ids = check_text("federal", "Roe v. Wade, 410 S.Ct. 113 (1973)");
        assert!(ids.iter().any(|id| id == "sCtReporterTight"));
    }
}
