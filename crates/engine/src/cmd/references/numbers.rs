//! Numbered REF results, adapted from @Alyosha-dot-shop's patch in #4533.

use super::{Counters, NumFormat, Path, Pos, StoryRef};

/// What a numbered body paragraph shows: the counter value of each level down to its own, the
/// label each of those levels shows at that point (`["14.", "(a)"]` for 14(a)), and the levels
/// each label's template shows (`"%1.%2"` shows levels 0 and 1).
#[derive(Clone, Debug, Default)]
pub(super) struct ParaNumber {
    /// Relative context is shared only within the same numbering instance.
    list: u32,
    values: Vec<u32>,
    labels: Vec<Option<String>>,
    shows: Vec<Vec<usize>>,
}

/// The numbered body paragraphs, and where each list stands at any body paragraph.
#[derive(Debug, Default)]
pub(super) struct ParaNumbers {
    numbers: std::collections::HashMap<Path, ParaNumber>,
    /// Each body paragraph's place in document order.
    order: std::collections::HashMap<Path, usize>,
    /// Each list's numbered paragraphs, by their place in document order (ascending).
    lists: std::collections::HashMap<u32, Vec<(usize, Path)>>,
    /// The bullet of each bullet item.
    bullets: std::collections::HashMap<Path, String>,
    /// The literal text of each paragraph on a level whose format is none, when it has some
    /// ("RESPONSE:"), and whether the levels above it count.
    literals: std::collections::HashMap<Path, (String, bool)>,
}

impl ParaNumbers {
    /// Where the body place `target` is from the body place `at` (a paragraph and an offset in
    /// it), as `REF \p` says it: "above" when it comes first, else "below" (Word for Windows,
    /// also within one paragraph). `None` when either is not a body paragraph.
    fn place(&self, target: &Pos, (at, off): (&Path, usize)) -> Option<&'static str> {
        let t = (*self.order.get(&target.path)?, target.off);
        let f = (*self.order.get(at)?, off);
        Some(if t < f { "above" } else { "below" })
    }

    /// The last paragraph of `list` at or before the body paragraph `at`: what `REF \r` there
    /// takes its context from.
    fn context(&self, list: u32, at: &Path) -> Option<&ParaNumber> {
        let i = *self.order.get(at)?;
        let paras = self.lists.get(&list)?;
        let (_, path) = paras.get(paras.partition_point(|(o, _)| *o <= i).checked_sub(1)?)?;
        self.numbers.get(path)
    }
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
/// layout does (cells merged into the one above count nothing), and every bullet item's bullet.
pub(super) fn para_numbers(doc: &wordcraft_doc::Document) -> ParaNumbers {
    let mut counters = Counters::default();
    let mut out = ParaNumbers::default();
    for (i, path) in doc.para_paths(StoryRef::Body).into_iter().enumerate() {
        out.order.insert(path.clone(), i);
        if in_merged_cell(doc, &path) {
            continue;
        }
        let Some(p) = doc.para(StoryRef::Body, &path) else { continue };
        let Some(n) = p.props.numbering.or_else(|| doc.styles.resolve_para(&p.props).numbering).filter(|n| n.num != 0) else { continue };
        let Some((label, def)) = counters.next_label(&doc.numbering, n.num, n.level) else { continue };
        if def.format == NumFormat::Bullet {
            // A reference to a bullet item shows its bullet (Word for Windows), as drawn: a
            // Symbol-font bullet is its Unicode character.
            let font = def.chr.font.as_deref().unwrap_or("");
            out.bullets.insert(path, wordcraft_doc::symbol_fonts::map_text(font, &label));
            continue;
        }
        if def.format == NumFormat::None {
            // A level whose format is none counts (#4985) but has no number of its own: a
            // reference to it shows 0, or its literal text when it has some (Word for Windows).
            if !label.trim().is_empty() {
                out.literals.insert(path, (label.trim().to_string(), n.level > 0));
            }
            continue;
        }
        let level = usize::from(n.level.min(8));
        let values = counters.values(&doc.numbering, n.num).unwrap_or_default();
        let mut num = ParaNumber { list: n.num, ..Default::default() };
        for k in 0..=level {
            num.values.push(values.get(k).copied().unwrap_or(0));
            // A level above that the list never used shows the number it counted from (its
            // start), as Word's references do: (a) with no clause above it is 1(a).
            num.labels.push(counters.current_label(&doc.numbering, n.num, k as u8));
            num.shows.push(counters.current_template_levels(&doc.numbering, n.num, k as u8));
        }
        out.lists.entry(n.num).or_default().push((i, path.clone()));
        out.numbers.insert(path, num);
    }
    out
}

/// A paragraph number as a cross-reference shows it: without the trailing period ("14." → "14"),
/// keeping a closing bracket ("(a)").
fn ref_part(label: &str) -> &str {
    label.trim().trim_end_matches('.').trim_end()
}

/// The number `REF \n`, `\r` or `\w` shows for the bookmarked paragraph `target` (ECMA-376 Part 1
/// §17.16.5.51), as Word computes it:
/// - `\n` (no context): the paragraph's own label.
/// - `\w` (full context): the own label, with the label of each level above it put in front when
///   no label already in the result shows that level: under `%1.` / `%1.%2` / `(%3)`, clause (a)
///   of 1.1 is `1.1(a)`, not `1.1.1(a)`.
/// - `\r` (relative context): like `\w`, but leaving out the leading levels whose counters the
///   target's list shows at the field's paragraph (`from`) as well: the counters of the last
///   paragraph of that list at or before the field, whether the field's own paragraph is in the
///   list, in another list, or unnumbered. From inside clause 14, or from an unnumbered paragraph
///   after 14(a), 14(a) is `(a)`; from clause 5, or before the list, it is `14(a)`. Another
///   numbering instance of the same abstract list is another list here, though it continues the
///   counters. Context already displayed by the own or a needed ancestor label stays.
///
/// A level above the target that its list never used shows the number it counted from: Word shows
/// `1.1(a)` for an `(a)` item whose list has no clause above it. All measured in Word for Windows.
/// A bullet item shows its bullet, and a body paragraph with no number (none at all, or a level
/// whose format is none and no text) shows 0, whatever the switch, as in Word. A level whose
/// format is none but has literal text shows that text ("RESPONSE:"); below a numbered level,
/// only `\n` does, since Word's `\r` and `\w` there aren't stable (it put a stray number in
/// front of "Note"), so those keep the stored result. `None` (the stored result stays) when the
/// number can't be put together without inventing a delimiter.
pub(super) fn ref_number(numbers: &ParaNumbers, target: &Path, from: Option<&Path>, switch: char) -> Option<String> {
    let Some(t) = numbers.numbers.get(target) else {
        if let Some((text, below)) = numbers.literals.get(target) {
            return (switch == 'n' || !below).then(|| text.clone());
        }
        return Some(numbers.bullets.get(target).cloned().unwrap_or_else(|| "0".into()));
    };
    let level = t.values.len().checked_sub(1)?;
    let own = t.labels.get(level)?.as_deref()?;
    let mut out = ref_part(own).to_string();
    if switch == 'n' {
        return Some(out);
    }
    let first = match (switch, from.and_then(|p| numbers.context(t.list, p))) {
        ('r', Some(f)) => (0..level).take_while(|&k| f.values.get(k).is_some() && f.values.get(k) == t.values.get(k)).count(),
        _ => 0,
    };
    // Relative context suppresses added ancestors, never text already rendered in a label.
    // Word's measured legal-template cases retain 2.2.2 even when 2 or 2.2 is shared.
    let mut shown = t.shows.get(level).cloned().unwrap_or_default();
    for k in (first..level).rev() {
        if shown.contains(&k) {
            continue;
        }
        if t.labels.get(k).is_some_and(Option::is_some) {
            let prefix = ref_part(t.labels.get(k)?.as_deref()?);
            // Without a delimiter, joining separate numeric labels would invent a number.
            if prefix.chars().next_back().is_some_and(char::is_alphanumeric) && out.chars().next().is_some_and(char::is_alphanumeric) {
                return None;
            }
            out.insert_str(0, prefix);
            shown.extend(t.shows.get(k).into_iter().flatten().copied());
        }
    }
    Some(out)
}

/// What a REF to the body bookmark starting at `target` shows with the paragraph number switch
/// `number` (`\r`, `\n` or `\w`) and, with `position`, `\p`: the number, "above" or "below", or
/// the number then the position ("(a) above"), as Word for Windows shows them. `from` is the
/// field's place, a body paragraph and the field's offset in it (`None` elsewhere: no relative
/// context, and no position). `None` keeps the stored result.
pub(super) fn ref_paragraph_result(
    numbers: &ParaNumbers,
    target: &Pos,
    from: Option<(&Path, usize)>,
    number: Option<char>,
    position: bool,
) -> Option<String> {
    let number = match number {
        Some(sw) => Some(ref_number(numbers, &target.path, from.map(|(p, _)| p), sw)?),
        None => None,
    };
    let place = if position { Some(numbers.place(target, from?)?) } else { None };
    match (number, place) {
        (Some(n), Some(p)) => Some(format!("{n} {p}")),
        (Some(n), None) => Some(n),
        (None, Some(p)) => Some(p.to_string()),
        (None, None) => None,
    }
}

/// Where each bookmark name first starts in the body, retaining document-order lookup semantics.
pub(super) fn body_bookmarks(doc: &wordcraft_doc::Document) -> std::collections::HashMap<String, Pos> {
    let mut starts = std::collections::HashMap::new();
    for (name, pos) in doc.bookmarks() {
        if pos.story == StoryRef::Body {
            starts.entry(name).or_insert(pos);
        }
    }
    starts
}
