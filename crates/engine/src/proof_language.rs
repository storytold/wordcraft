//! Assign proofing tags to Latin/Cyrillic spans. Speech shares classification, not this policy.
use std::ops::Range;

use unicode_script::UnicodeScript;
use wordcraft_doc::{Document, Paragraph, Pos, StoryRef, para::Run};

use crate::scripts::{self, Script};
use crate::{CmdError, Session};

/// A letter or a combining mark: what extends a word to the left or right of an edit.
fn word_char(c: char) -> bool {
    c.is_alphabetic() || c.script() == unicode_script::Script::Inherited
}

/// Language-specific letters constrain candidates; shared і alone does not identify Ukrainian.
#[derive(Default)]
struct Evidence {
    uk: bool,
    be: bool,
    shared_i: bool,
    other: bool,
    candidates: Option<Vec<&'static str>>,
}

impl Evidence {
    /// Collect the Cyrillic letters in `text` that point at, or rule out, particular languages.
    fn of(text: &str) -> Self {
        let mut e = Self::default();
        for c in text.chars() {
            use scripts::CyrillicLetter;
            match scripts::cyrillic_letter(c) {
                Some(CyrillicLetter::Ukrainian) => e.uk = true,
                Some(CyrillicLetter::Belarusian) => e.be = true,
                Some(CyrillicLetter::SharedI) => e.shared_i = true,
                Some(CyrillicLetter::Candidates(tags)) => {
                    e.other = true;
                    if let Some(candidates) = &mut e.candidates {
                        candidates.retain(|t| tags.contains(t));
                    } else {
                        e.candidates = Some(tags.to_vec());
                    }
                }
                _ => {}
            }
        }
        e
    }
    /// Whether `tag` agrees with every piece of evidence: a Ukrainian letter rules out Russian, etc.
    fn permits(&self, tag: &str) -> bool {
        let primary = scripts::primary_language(tag);
        (!self.uk || primary == "uk")
            && (!self.be || primary == "be")
            && (!self.shared_i || matches!(primary.as_str(), "uk" | "be" | "kk"))
            && self.candidates.as_ref().is_none_or(|tags| tags.contains(&primary.as_str()))
    }
    /// The language the letters alone identify, when only Ukrainian or only Belarusian ones occur.
    fn certain(&self) -> Option<&'static str> {
        match (self.uk, self.be, self.other) {
            (true, false, false) => Some("uk-UA"),
            (false, true, false) => Some("be-BY"),
            _ => None,
        }
    }
}

/// The tag for a span of `script`, or `None` to leave it as it is.
///
/// Latin: a compatible existing tag, else the Latin editing language, else `en-US`. Cyrillic: a
/// compatible existing tag, else the Cyrillic editing language, else the first installed
/// dictionary the evidence permits, else the language the letters identify, else `ru-RU`;
/// contradicting or unidentifiable evidence keeps the original tag.
fn choose(e: &Evidence, script: Script, explicit: Option<&str>, editing: Option<&str>, installed: &[String]) -> Option<String> {
    let explicit = explicit.and_then(wordcraft_proof::lang::normalize_tag);
    let editing = editing.and_then(wordcraft_proof::lang::normalize_tag);
    match script {
        Script::Latin => explicit
            .filter(|t| scripts::of_language(t).is_none_or(|s| s == script))
            .or_else(|| editing.filter(|t| scripts::of_language(t) == Some(script)))
            .or_else(|| Some("en-US".into())),
        Script::Cyrillic => {
            if e.uk && e.be {
                return None;
            }
            explicit
                .filter(|t| scripts::of_language(t).is_none_or(|s| s == script) && e.permits(t))
                .or_else(|| editing.filter(|t| scripts::of_language(t) == Some(script) && e.permits(t)))
                .or_else(|| installed.iter().find(|t| e.permits(t)).cloned())
                .or_else(|| e.certain().map(str::to_string))
                .or_else(|| (!e.shared_i && !e.other).then(|| "ru-RU".into()))
        }
        _ => None,
    }
}

/// Partition at strong-script changes. Neutral characters follow the preceding strong letter;
/// leading neutrals follow the first one. Unsupported letters delimit spans but keep their tag.
fn spans(text: &str, base: usize) -> Vec<(Range<usize>, Script)> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut current = None;
    for (off, c) in text.char_indices().filter(|(_, c)| c.is_alphabetic() && c.script() != unicode_script::Script::Inherited) {
        let script = scripts::of_char(c);
        if let Some(previous) = current
            && previous != script
        {
            out.push((base + start..base + off, previous));
            start = off;
        }
        current = Some(script);
    }
    if let Some(script) = current {
        out.push((base + start..base + text.len(), script));
    }
    out
}

/// A table's style properties that matter for proofing, resolved once per table.
struct TableProofing {
    style: wordcraft_doc::styles::TableStyleProps,
    // Conditional styles need the grid width and each cell's starting column, computed once.
    columns: Option<Vec<Vec<usize>>>,
    width: usize,
}

type TableCache = std::collections::BTreeMap<(StoryRef, wordcraft_doc::Path), Option<TableProofing>>;

impl TableProofing {
    /// `None` when the table has no style, or its style sets neither `lang` nor `noProof`, so its
    /// cells need no lookup.
    fn new(doc: &Document, table: &wordcraft_doc::Table) -> Option<Self> {
        let style = doc.styles.table_style(table.props.style.as_deref()?)?;
        let relevant = |c: &wordcraft_doc::CharProps| c.lang.is_some() || c.no_proof.is_some();
        let conditional = wordcraft_doc::styles::TablePart::ALL
            .into_iter()
            .filter(|p| *p != wordcraft_doc::styles::TablePart::WholeTable)
            .any(|p| relevant(&style.part(p).chr));
        if !conditional && !relevant(&style.chr) {
            return None;
        }
        let columns = conditional.then(|| {
            table
                .rows
                .iter()
                .map(|row| {
                    let mut grid = row.first_grid();
                    row.cells
                        .iter()
                        .map(|cell| {
                            let start = grid;
                            grid = grid.saturating_add(cell.span());
                            start
                        })
                        .collect()
                })
                .collect()
        });
        let width = if conditional { table.cols().max(1) } else { 0 };
        Some(Self { style, columns, width })
    }
}

/// Resolve table-style character properties with the same conditional parts as layout.
fn table_char_props(doc: &Document, at: &Pos, cache: &mut TableCache) -> Option<wordcraft_doc::CharProps> {
    let (path, ri, ci) = at.path.cell()?;
    let table = doc.table(at.story, &path)?;
    let context = cache.entry((at.story, path)).or_insert_with(|| TableProofing::new(doc, table)).as_ref()?;
    let Some(columns) = &context.columns else {
        return Some(context.style.chr.clone());
    };
    let cell = table.rows.get(ri)?.cells.get(ci)?;
    let grid = *columns.get(ri)?.get(ci)?;
    let style = &context.style;
    let parts = wordcraft_doc::styles::PartSet::of_cell(
        &table.props.look,
        (style.parts.band_size.unwrap_or(0), style.parts.col_band_size.unwrap_or(0)),
        (ri, table.rows.len()),
        (grid, cell.span(), context.width),
    );
    let mut chr = wordcraft_doc::CharProps::default();
    for part in parts.iter() {
        chr.overlay(&style.part(part).chr);
    }
    Some(chr)
}

/// Calculate patches before touching the paragraph; split/merge the runs in a single pass.
fn patches(
    doc: &Document,
    p: &Paragraph,
    range: Range<usize>,
    editing: Option<&str>,
    installed: &[String],
    table: Option<&wordcraft_doc::CharProps>,
) -> Vec<(Range<usize>, String)> {
    if p.text.get(range.clone()).is_none() {
        return Vec::new();
    }
    let mut eligible = Vec::new();
    let mut groups: Vec<Range<usize>> = Vec::new();
    for (r, props) in p.run_ranges() {
        if r.end <= range.start {
            continue;
        }
        if r.start >= range.end {
            break;
        }
        let resolved = doc.styles.resolve_char_in(p.props.style.as_deref(), table, props);
        if resolved.no_proof || props.del.is_some() {
            continue;
        }
        let r = r.start.max(range.start)..r.end.min(range.end);
        if let Some(group) = groups.last_mut().filter(|group| group.end == r.start) {
            group.end = r.end;
        } else {
            groups.push(r.clone());
        }
        eligible.push((r, resolved));
    }
    // Skipped/deleted runs are evidence barriers, including when the script on both sides matches.
    let spans: Vec<_> = groups
        .into_iter()
        .flat_map(|r| spans(p.text.get(r.clone()).unwrap_or(""), r.start))
        .map(|(r, script)| {
            let evidence = Evidence::of(p.text.get(r.clone()).unwrap_or(""));
            (r, script, evidence)
        })
        .collect();
    let mut changes = Vec::new();
    for (r, resolved) in eligible {
        let first = spans.partition_point(|(s, _, _)| s.end <= r.start);
        for (span, script, evidence) in spans.iter().skip(first).take_while(|(s, _, _)| s.start < r.end) {
            let overlap = r.start.max(span.start)..r.end.min(span.end);
            if let Some(lang) = choose(evidence, *script, resolved.lang.as_deref(), editing, installed)
                && resolved.lang.as_deref() != Some(lang.as_str())
            {
                changes.push((overlap, lang));
            }
        }
    }
    changes
}

/// Set the language of the sorted, disjoint byte ranges in `changes`, splitting runs at their
/// edges and keeping every other property; adjacent equal runs merge again afterwards.
fn apply(p: &mut Paragraph, changes: &[(Range<usize>, String)]) {
    if changes.is_empty() {
        return;
    }
    let mut out = Vec::new();
    let mut off = 0;
    let mut index = 0;
    for run in &p.runs {
        let end = off + run.len;
        while off < end {
            while changes.get(index).is_some_and(|(r, _)| r.end <= off) {
                index += 1;
            }
            let mut props = run.props.clone();
            let next = if let Some((r, tag)) = changes.get(index).filter(|(r, _)| r.start < end) {
                if r.start <= off {
                    props.lang = Some(tag.clone());
                    r.end.min(end)
                } else {
                    r.start
                }
            } else {
                end
            };
            out.push(Run { len: next - off, props });
            off = next;
        }
    }
    p.runs = out;
    p.normalize();
    p.touch();
}

/// Detect the language of `range` in the paragraph at `at`, widened to the whole words it cuts
/// through. Returns whether any tag changed.
fn paragraph(
    doc: &mut Document,
    at: &Pos,
    mut range: Range<usize>,
    editing: Option<&str>,
    installed: &[String],
    tables: &mut TableCache,
) -> Result<bool, CmdError> {
    let Some(p) = doc.para_at(at) else { return Ok(false) };
    // Only extend to adjoining word pieces, never to the rest of the document on a keystroke.
    let Some(before) = p.text.get(..range.start) else { return Ok(false) };
    range.start -= before.chars().rev().take_while(|c| word_char(*c)).map(char::len_utf8).sum::<usize>();
    let Some(after) = p.text.get(range.end..) else { return Ok(false) };
    range.end += after.chars().take_while(|c| word_char(*c)).map(char::len_utf8).sum::<usize>();
    let table = table_char_props(doc, at, tables);
    let changes = patches(doc, p, range, editing, installed, table.as_ref());
    if changes.is_empty() {
        return Ok(false);
    }
    apply(doc.para_mut(at.story, &at.path)?, &changes);
    Ok(true)
}

impl Session {
    /// `review.language {"detect": true}`: process the current document and update the caret's
    /// pending language to match.
    pub(crate) fn detect_current_language(&mut self) -> Result<(), CmdError> {
        let mut tables = TableCache::new();
        let previous = self.actual_language(&self.sel.focus, &mut tables);
        let mut doc = self.doc.clone();
        if self.detect_document_language(&mut doc)? {
            self.doc = doc;
        }
        self.sync_detected_pending(&self.sel.focus.clone(), previous.as_deref(), &mut tables);
        Ok(())
    }

    /// The effective language at `at`: direct formatting over the table and paragraph styles.
    fn actual_language(&self, at: &Pos, tables: &mut TableCache) -> Option<String> {
        let p = self.doc.para_at(at)?;
        self.doc.styles.resolve_char_in(p.props.style.as_deref(), table_char_props(&self.doc, at, tables).as_ref(), p.props_at(at.off)).lang
    }

    /// Pending font formatting must not keep reporting the previous script's language.
    fn sync_detected_pending(&mut self, at: &Pos, previous: Option<&str>, tables: &mut TableCache) {
        if !self.prefs.detect_language || crate::cmd::protect::enforced(&self.doc) {
            return;
        }
        let Some(p) = self.doc.para_at(at) else { return };
        if p.text.is_empty() {
            return;
        }
        let resolved =
            self.doc.styles.resolve_char_in(p.props.style.as_deref(), table_char_props(&self.doc, at, tables).as_ref(), p.props_at(at.off));
        if resolved.no_proof {
            return;
        }
        if let Some(pending) = self.pending.as_mut()
            && resolved.lang.as_deref().and_then(scripts::of_language).is_some_and(|script| {
                matches!(script, Script::Latin | Script::Cyrillic)
                    && (previous != resolved.lang.as_deref() || pending.lang.as_deref().and_then(scripts::of_language) != Some(script))
            })
        {
            pending.lang = resolved.lang;
        }
    }

    /// After typing or paste: detect the language of the inserted `range` of the paragraph at `at`,
    /// as part of the same edit (and undo step).
    pub(crate) fn detect_inserted_language(&mut self, at: &Pos, range: Range<usize>) -> Result<(), CmdError> {
        if !self.prefs.detect_language || crate::cmd::protect::enforced(&self.doc) || range.is_empty() {
            return Ok(());
        }
        let mut tables = TableCache::new();
        let previous = self.actual_language(at, &mut tables);
        let editing = self.prefs.default_language.as_deref().or(self.editing_lang.as_deref());
        let installed = wordcraft_proof::installed_cyrillic_languages();
        paragraph(&mut self.doc, at, range, editing, &installed, &mut tables)?;
        self.sync_detected_pending(at, previous.as_deref(), &mut tables);
        Ok(())
    }

    /// Normalize existing text before it becomes an opened document, or on explicit enable.
    pub(crate) fn detect_document_language(&self, doc: &mut Document) -> Result<bool, CmdError> {
        if !self.prefs.detect_language || crate::cmd::protect::enforced(doc) {
            return Ok(false);
        }
        let editing = self.prefs.default_language.as_deref().or(self.editing_lang.as_deref());
        let installed = wordcraft_proof::installed_cyrillic_languages();
        let stories: Vec<_> = std::iter::once(StoryRef::Body).chain(doc.parts.keys().copied().map(StoryRef::Part)).collect();
        let mut changed = false;
        let mut tables = TableCache::new();
        for story in stories {
            for path in doc.para_paths(story) {
                let at = Pos { story, path, off: 0 };
                let len = doc.para_at(&at).map_or(0, Paragraph::len);
                changed |= paragraph(doc, &at, 0..len, editing, &installed, &mut tables)?;
            }
        }
        Ok(changed)
    }

    /// Detect the language of a fragment about to be pasted, with this document's styles, so the
    /// inserted runs arrive tagged.
    pub(crate) fn detect_fragment_language(&self, frag: &mut wordcraft_doc::edit::Fragment) -> Result<(), CmdError> {
        if !self.prefs.detect_language || crate::cmd::protect::enforced(&self.doc) {
            return Ok(());
        }
        let mut doc = Document::new();
        doc.styles = self.doc.styles.clone();
        doc.body = std::mem::take(&mut frag.blocks).into_iter().map(std::sync::Arc::new).collect();
        doc.parts = std::mem::take(&mut frag.parts);
        self.detect_document_language(&mut doc)?;
        frag.blocks = doc.body.into_iter().map(std::sync::Arc::unwrap_or_clone).collect();
        frag.parts = doc.parts;
        Ok(())
    }
}

#[cfg(test)]
mod tests;
