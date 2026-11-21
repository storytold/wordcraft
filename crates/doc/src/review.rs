use super::{Blocks, Deserialize, Document, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct Comment {
    pub author: String,
    pub initials: String,
    /// ISO 8601.
    pub date: String,
    /// Reply to this comment id.
    pub parent: Option<u32>,
    pub resolved: bool,
    /// `Document::parts` id of the comment text.
    pub part: u32,
    /// Word's durable id for the comment (`w16cid:durableId`, hex), kept across saves so Word
    /// keeps tracking the same comment. `None` for comments made here: the writer assigns one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub durable_id: Option<String>,
    /// The comment's UTC timestamp: from the file (`w16cex:dateUtc`, where `date` is local time),
    /// or the time WordCraft stamped the comment with.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date_utc: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum RevisionKind {
    #[default]
    Insert,
    Delete,
    Format,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Revision {
    pub kind: RevisionKind,
    pub author: String,
    pub date: String,
    /// A move (ECMA-376 §17.13.5.21–28): the name linking the source (an `Delete` revision,
    /// `w:moveFrom`) with the destination (an `Insert` revision, `w:moveTo`). `None` = a plain
    /// insertion or deletion.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub move_name: Option<String>,
}

impl Revision {
    pub fn new(kind: RevisionKind, author: impl Into<String>, date: impl Into<String>) -> Revision {
        Revision { kind, author: author.into(), date: date.into(), move_name: None }
    }
}

impl Document {
    /// The move name of revision `idx`, when it is one end of a move.
    pub fn move_name(&self, idx: Option<u32>) -> Option<&str> {
        idx.and_then(|i| self.revisions.get(i as usize)).and_then(|r| r.move_name.as_deref())
    }
}

/// The header/footer selection flags Compare changed for the First and Even stories it diffed,
/// with the values they had before, so `Reject All` can put them back (#5837).
///
/// It belongs to the document, not the session: a comparison result that is saved, reopened and
/// then rejected must restore the flags the original had.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct CompareHfFlags {
    /// Sections whose `title_page` compare changed: the section's ordinal index in
    /// `Document::sections()` (`None` = the last section) and the value it had.
    pub title_page: Vec<(Option<usize>, bool)>,
    /// `even_odd_headers` before compare changed it.
    pub even_odd: Option<bool>,
}

impl Document {
    /// Applies saved `(section_index, orig_title_page)` entries to the document's sections.
    ///
    /// Stale indices (`Some(si)` past the current non-final section count) are skipped without
    /// consuming the restoration budget; duplicate valid keys are ignored; iteration stops once
    /// every current section has been handled at most once.
    ///
    /// When `stamp_only` is true (DOCX reopen), stamps each valid section's `compare_title_page`
    /// tag without changing its live `title_page`. When false (`Reject All`), restores
    /// `title_page` directly.
    pub fn apply_compare_title_page_entries(&mut self, entries: &[(Option<usize>, bool)], stamp_only: bool) {
        let non_final_ends: Vec<usize> =
            self.body.iter().enumerate().filter_map(|(i, b)| matches!(&**b, crate::Block::Para(p) if p.section.is_some()).then_some(i)).collect();
        let non_final_count = non_final_ends.len();
        let total_sections = non_final_count.saturating_add(1);
        let mut seen_non_final = vec![false; non_final_count];
        let mut seen_non_final_count = 0usize;
        let mut seen_last = false;

        for &(sec_idx, orig_tp) in entries {
            if seen_non_final_count.saturating_add(usize::from(seen_last)) >= total_sections {
                break;
            }
            match sec_idx {
                Some(si) if si < non_final_count => {
                    if let Some(done) = seen_non_final.get_mut(si) {
                        if *done {
                            continue;
                        }
                        *done = true;
                        seen_non_final_count += 1;
                        if let Some(&block_idx) = non_final_ends.get(si)
                            && let Some(crate::Block::Para(p)) = self.body.get_mut(block_idx).map(std::sync::Arc::make_mut)
                            && let Some(sec) = p.section.as_deref_mut()
                        {
                            if stamp_only {
                                sec.compare_title_page = crate::section::CompareTitlePage(Some(orig_tp));
                            } else {
                                sec.title_page = orig_tp;
                                sec.compare_title_page = crate::section::CompareTitlePage(None);
                            }
                        }
                    }
                }
                Some(_) => continue,
                None => {
                    if seen_last {
                        continue;
                    }
                    seen_last = true;
                    if stamp_only {
                        self.last_section.compare_title_page = crate::section::CompareTitlePage(Some(orig_tp));
                    } else {
                        self.last_section.title_page = orig_tp;
                        self.last_section.compare_title_page = crate::section::CompareTitlePage(None);
                    }
                }
            }
        }
    }

    /// Returns whether any section currently carries a live `compare_title_page` tag.
    pub fn has_live_compare_title_page(&self) -> bool {
        self.last_section.compare_title_page.0.is_some()
            || self.body.iter().any(|b| matches!(&**b, crate::Block::Para(p) if p.section.as_ref().is_some_and(|s| s.compare_title_page.0.is_some())))
    }

    /// Collects `(section_key, orig_title_page)` from every section currently carrying a live
    /// `compare_title_page` tag.
    fn live_compare_title_page(&self) -> Vec<(Option<usize>, bool)> {
        let mut out = Vec::new();
        let mut sec_idx = 0usize;
        for b in &self.body {
            if let crate::Block::Para(p) = &**b
                && let Some(sec) = p.section.as_deref()
            {
                if let Some(orig) = sec.compare_title_page.0 {
                    out.push((Some(sec_idx), orig));
                }
                sec_idx += 1;
            }
        }
        if let Some(orig) = self.last_section.compare_title_page.0 {
            out.push((None, orig));
        }
        out
    }

    /// Rebuilds `self.compare_hf`'s `title_page` list from live section tags, dropping
    /// `compare_hf` when nothing remains tracked.
    pub fn sync_compare_hf_title_page(&mut self) {
        let title_page = self.live_compare_title_page();
        if let Some(flags) = self.compare_hf.as_mut() {
            flags.title_page = title_page;
            if flags.title_page.is_empty() && flags.even_odd.is_none() {
                self.compare_hf = None;
            }
        }
    }

    /// Builds the [`CompareHfFlags`] snapshot to serialize on DOCX save.
    ///
    /// Uses live section tags when present so section-break edits after Compare keep their exact
    /// owning sections across save and reopen; falls back to `self.compare_hf` otherwise.
    pub fn compare_hf_for_save(&self) -> Option<CompareHfFlags> {
        let live = self.live_compare_title_page();
        if !live.is_empty() {
            Some(CompareHfFlags { title_page: live, even_odd: self.compare_hf.as_ref().and_then(|f| f.even_odd) })
        } else {
            self.compare_hf.clone()
        }
    }

    /// Hydrates section-level `compare_title_page` tags from `flags` when reopening a DOCX.
    pub fn hydrate_compare_hf(&mut self, flags: CompareHfFlags) {
        self.apply_compare_title_page_entries(&flags.title_page, true);
        self.compare_hf = Some(flags);
    }

    /// Clears all Compare header/footer flag tracking on the document and its sections.
    pub fn clear_compare_hf(&mut self) {
        self.compare_hf = None;
        for b in &mut self.body {
            if matches!(&**b, crate::Block::Para(p) if p.section.as_ref().is_some_and(|s| s.compare_title_page.0.is_some()))
                && let crate::Block::Para(p) = std::sync::Arc::make_mut(b)
                && let Some(sec) = p.section.as_deref_mut()
            {
                sec.compare_title_page = crate::section::CompareTitlePage(None);
            }
        }
        self.last_section.compare_title_page = crate::section::CompareTitlePage(None);
    }

    /// Hydrates unhydrated `compare_hf.title_page` entries onto their current sections before
    /// `Reject All` removes rejected paragraphs or section breaks.
    pub fn prepare_compare_hf_for_reject(&mut self) {
        if self.has_live_compare_title_page() {
            return;
        }
        if let Some(entries) = self.compare_hf.as_ref().filter(|f| !f.title_page.is_empty()).map(|f| f.title_page.clone()) {
            self.apply_compare_title_page_entries(&entries, true);
        }
    }

    /// Restores original `title_page` and `even_odd_headers` flags recorded by Compare,
    /// consuming the tracking state.
    pub fn restore_compare_hf(&mut self) {
        let flags = self.compare_hf.take();
        let mut restored_any_tagged = false;
        for b in &mut self.body {
            if matches!(&**b, crate::Block::Para(p) if p.section.as_ref().is_some_and(|s| s.compare_title_page.0.is_some()))
                && let crate::Block::Para(p) = std::sync::Arc::make_mut(b)
                && let Some(sec) = p.section.as_deref_mut()
                && let Some(orig) = sec.compare_title_page.0.take()
            {
                sec.title_page = orig;
                restored_any_tagged = true;
            }
        }
        if let Some(orig) = self.last_section.compare_title_page.0.take() {
            self.last_section.title_page = orig;
            restored_any_tagged = true;
        }
        if let Some(flags) = flags {
            if !restored_any_tagged && !flags.title_page.is_empty() {
                self.apply_compare_title_page_entries(&flags.title_page, false);
            }
            if let Some(even_odd) = flags.even_odd {
                self.settings.even_odd_headers = even_odd;
            }
        }
    }
}

/// A building block (Insert › Quick Parts, AutoText…) kept in the document's glossary
/// (ECMA-376 §17.12: `w:docPart`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct BuildingBlock {
    /// The entry's name (`w:docPartPr/w:name`).
    pub name: String,
    /// Its gallery (`w:category/w:gallery`: `docParts` for Quick Parts, `autoTxt` for AutoText…).
    pub gallery: String,
    /// Its category within the gallery (`w:category/w:name`, e.g. "General").
    pub category: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub description: String,
    /// The entry's identity (`w:guid`), kept as read.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub guid: String,
    /// The paragraph style it's associated with (`w:style`), kept as read.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub style: String,
    /// Its types (`w:types/w:type`: `normal`, `bbPlcHdr` for a content control's placeholder…).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub types: Vec<String>,
    /// How it's inserted (`w:behaviors/w:behavior`: `content`, `p`, `pg`).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub behaviors: Vec<String>,
    /// The content (`w:docPartBody`).
    pub blocks: Blocks,
}

/// The gallery Quick Parts saves to (`w:gallery w:val="docParts"`).
pub const QUICK_PARTS_GALLERY: &str = "docParts";

/// The AutoText gallery (`w:gallery w:val="autoTxt"`).
pub const AUTO_TEXT_GALLERY: &str = "autoTxt";
