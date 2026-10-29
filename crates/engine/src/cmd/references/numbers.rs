//! Numbered REF results, adapted from @Alyosha-dot-shop's patch in #4533.

use super::*;

/// What a numbered body paragraph shows: the counter value of each level down to its own, the
/// label each of those levels shows at that point (`["14.", "(a)"]` for 14(a)), and the levels
/// each label's template shows (`"%1.%2"` shows levels 0 and 1).
#[derive(Clone, Debug, Default)]
pub(super) struct ParaNumber {
    values: Vec<u32>,
    /// Whether each level has counted a paragraph yet: a level the list never used (an `(a)` item
    /// with no clause above it in its list) shows no number in a reference.
    started: Vec<bool>,
    labels: Vec<Option<String>>,
    shows: Vec<Vec<usize>>,
}

/// Whether the body paragraph at `path` sits in a table cell merged into the one above
/// (`w:vMerge` continue), at any nesting depth. The layout shows nothing of such a cell, so its
/// paragraphs take no list number.
fn in_merged_cell(doc: &wordcraft_doc::Document, path: &Path) -> bool {
    let p = &path.0;
    let mut j = 0usize;
    // [table, row, cell, block, row, cell, block, …]: a table at p[..=j], its row and cell next.
    while j.saturating_add(3) < p.len() {
        let (Some(tp), Some(&r), Some(&c)) = (p.get(..=j), p.get(j + 1), p.get(j + 2)) else { return false };
        let merged = doc
            .table(StoryRef::Body, &Path(tp.to_vec()))
            .and_then(|t| t.rows.get(r as usize))
            .and_then(|row| row.cells.get(c as usize))
            .is_some_and(|cell| cell.props.vmerge == wordcraft_doc::props::VMerge::Continue);
        if merged {
            return true;
        }
        j = j.saturating_add(3);
    }
    false
}

/// Every numbered body paragraph's number, by path, walking the lists in document order as the
/// layout does (cells merged into the one above count nothing). Bullets have no number.
pub(super) fn para_numbers(doc: &wordcraft_doc::Document) -> std::collections::HashMap<Path, ParaNumber> {
    let mut counters = Counters::default();
    let mut out = std::collections::HashMap::new();
    for path in doc.para_paths(StoryRef::Body) {
        if in_merged_cell(doc, &path) {
            continue;
        }
        let Some(p) = doc.para(StoryRef::Body, &path) else { continue };
        let Some(n) = p.props.numbering.or_else(|| doc.styles.resolve_para(&p.props).numbering).filter(|n| n.num != 0) else { continue };
        let Some((_, def)) = counters.next_label(&doc.numbering, n.num, n.level) else { continue };
        if def.format == NumFormat::Bullet {
            continue;
        }
        let level = usize::from(n.level.min(8));
        let values = counters.values(&doc.numbering, n.num).unwrap_or_default();
        let started = counters.started(&doc.numbering, n.num).unwrap_or_default();
        let mut num = ParaNumber::default();
        for k in 0..=level {
            num.values.push(values.get(k).copied().unwrap_or(0));
            num.started.push(started.get(k).copied().unwrap_or(false));
            let def = doc.numbering.level(n.num, k as u8);
            num.labels.push(counters.current_label(&doc.numbering, n.num, k as u8));
            num.shows.push(def.map(|d| wordcraft_doc::numbering::template_levels(&d.text)).unwrap_or_default());
        }
        out.insert(path, num);
    }
    out
}

/// A paragraph number as a cross-reference shows it: without the trailing period ("14." → "14"),
/// keeping a closing bracket ("(a)").
fn ref_part(label: &str) -> &str {
    label.trim().trim_end_matches('.').trim_end()
}

/// One rendered level without already shared context, retaining cached values if its prefix is ambiguous.
fn relative_part(t: &ParaNumber, level: usize, first: usize) -> Option<String> {
    let mut out = ref_part(t.labels.get(level)?.as_deref()?).to_string();
    let levels = t.shows.get(level).cloned().unwrap_or_default();
    // Full templates such as %1.%2.%3 already contain the shared context. Remove one
    // complete matching prefix; never strip successive labels (4.4.4 could lose its own 4).
    let shared: Vec<_> = levels.iter().copied().take_while(|k| *k < first).collect();
    if !shared.is_empty() {
        let relative = (0..first).rev().find_map(|k| {
            let prefix_levels = t.shows.get(k)?;
            if !shared.iter().all(|n| prefix_levels.contains(n)) {
                return None;
            }
            let prefix = ref_part(t.labels.get(k)?.as_deref()?);
            out.strip_prefix(prefix)
                .map(|rest| rest.trim_start_matches(|c: char| c == '.' || c.is_whitespace()).to_string())
                .filter(|s| !s.is_empty())
        });
        // A differently decorated prefix cannot safely be guessed: retain its cached result.
        out = relative?;
    }
    Some(out)
}

/// The number `REF \n`, `\r` or `\w` shows for the bookmarked paragraph `target` (ECMA-376 Part 1
/// §17.16.5.51), as Word computes it:
/// - `\n` (no context): the paragraph's own label.
/// - `\w` (full context): the own label, with the label of each level above it put in front when
///   no label already in the result shows that level: under `%1.` / `%1.%2` / `(%3)`, clause (a)
///   of 1.1 is `1.1(a)`, not `1.1.1(a)`.
/// - `\r` (relative context): like `\w`, but leaving out the leading levels whose counters the
///   field's own paragraph (`from`) shares with the target, compared level by level whichever list
///   it is in. From inside clause 14, 14(a) is `(a)`; from clause 5 or an unnumbered paragraph it
///   is `14(a)`.
///
/// A level above the target that its list never used adds nothing (Word shows `(a)` for an item
/// whose list has no clause above it, not a made-up `1.1(a)`). `None` when the target paragraph
/// has no number.
pub(super) fn ref_number(numbers: &std::collections::HashMap<Path, ParaNumber>, target: &Path, from: Option<&Path>, switch: char) -> Option<String> {
    let t = numbers.get(target)?;
    let level = t.values.len().checked_sub(1)?;
    let own = t.labels.get(level)?.as_deref()?;
    let mut out = ref_part(own).to_string();
    if switch == 'n' {
        return Some(out);
    }
    let used = |k: usize| t.started.get(k).copied().unwrap_or(false);
    let first = match (switch, from.and_then(|p| numbers.get(p))) {
        ('r', Some(f)) => (0..level).take_while(|&k| !used(k) || (f.values.get(k).is_some() && f.values.get(k) == t.values.get(k))).count(),
        _ => 0,
    };
    out = relative_part(t, level, first)?;
    let mut shown = t.shows.get(level).cloned().unwrap_or_default();
    for k in (first..level).rev() {
        if shown.contains(&k) || !used(k) {
            continue;
        }
        if t.labels.get(k).is_some_and(Option::is_some) {
            let prefix = relative_part(t, k, first)?;
            // Without a delimiter, joining separate numeric labels would invent a number.
            if prefix.chars().next_back().is_some_and(char::is_alphanumeric) && out.chars().next().is_some_and(char::is_alphanumeric) {
                return None;
            }
            out.insert_str(0, &prefix);
            shown.extend(t.shows.get(k).into_iter().flatten().copied());
        }
    }
    Some(out)
}

/// Index the first body paragraph for each bookmark name, retaining document-order lookup semantics.
pub(super) fn body_bookmark_paths(doc: &wordcraft_doc::Document) -> std::collections::HashMap<String, Path> {
    let mut paths = std::collections::HashMap::new();
    for (name, pos) in doc.bookmarks() {
        if pos.story == StoryRef::Body {
            paths.entry(name).or_insert(pos.path);
        }
    }
    paths
}
