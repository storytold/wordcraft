use super::doc_state::{CellRange, EDIT_MARKS, Selection};
use super::{CharProps, Document, PartKind, Path, Pos, Session, StoryRef, ViewMode, untitled_name};

impl Session {
    /// Remember the caret as the place of an edit, for Go Back. Edits in the same paragraph as
    /// the last one (typing on) update that place rather than adding one.
    pub(super) fn note_edit(&mut self) {
        let at = self.sel.focus.clone();
        match self.edit_marks.last_mut() {
            Some(last) if last.story == at.story && last.path == at.path => *last = at,
            _ => {
                self.edit_marks.push(at);
                if self.edit_marks.len() > EDIT_MARKS {
                    self.edit_marks.remove(0);
                }
            }
        }
        self.go_back_step = 0;
    }

    /// Go Back (Shift+F5): the next of the last edit places, most recent first, cycling; a place
    /// the caret is already at is skipped. `None` before any edit.
    pub fn go_back(&mut self) -> Option<Pos> {
        let n = self.edit_marks.len();
        for _ in 0..n {
            let i = n - 1 - self.go_back_step % n;
            self.go_back_step = self.go_back_step.wrapping_add(1);
            let Some(p) = self.edit_marks.get(i) else { continue };
            let p = self.doc.clamp(p);
            if p != self.sel.focus || n == 1 {
                return Some(p);
            }
        }
        self.edit_marks.last().map(|p| self.doc.clamp(p))
    }

    /// The line pieces of the current column selection, if there is one (the selection hasn't
    /// moved since the block was made).
    pub fn column_segments(&self) -> Option<&[(Pos, Pos)]> {
        // Cells that are one run of the text are an ordinary stream selection.
        self.column.as_ref().filter(|c| c.sel == self.sel && !c.cells.as_ref().is_some_and(|r| r.stream)).map(|c| c.segments.as_slice())
    }

    /// The separate pieces of text selected, if the selection is that (see [`ColumnBlock::pieces`]).
    pub fn selection_pieces(&self) -> Option<&[(Pos, Pos)]> {
        self.column.as_ref().filter(|c| c.sel == self.sel).and_then(|c| c.pieces.as_deref())
    }

    /// The whole table cells selected, if the selection is that (see [`CellRange`]).
    pub fn cell_range(&self) -> Option<&CellRange> {
        self.column.as_ref().filter(|c| c.sel == self.sel).and_then(|c| c.cells.as_ref())
    }

    /// The selection as it was made: where a drag, Shift+click or Shift+arrows started and where
    /// the caret went, even when Word's table rule widened it to whole cells or rows (`sel` holds
    /// those).
    pub fn sel_ends(&self) -> &Selection {
        self.widened.as_ref().filter(|w| w.sel == self.sel).map_or(&self.sel, |w| &w.ends)
    }

    pub fn selects_whole_story(&self) -> bool {
        let st = self.sel.focus.story;
        let (a, b) = self.sel.ordered();
        a == self.doc.start_of(st) && b == self.doc.end_of(st)
    }

    /// Whether the caret is in a header or footer.
    pub fn in_header_footer(&self) -> bool {
        let StoryRef::Part(id) = self.sel.focus.story else { return false };
        self.doc.parts.get(&id).is_some_and(|p| matches!(p.kind, PartKind::Header | PartKind::Footer))
    }

    /// Document revision (bumped by every change).
    pub fn rev(&self) -> u64 {
        self.rev
    }

    /// Which document is open (see `document_id`): lets a caller tell an edit from a replacement.
    pub fn document_id(&self) -> u64 {
        self.document_id
    }

    /// How many times the body was replaced under this [`Self::document_id`].
    ///
    /// Front ends use it as the body's IME replacement epoch and to tell a body swap from an
    /// ordinary edit; it advances for `document.setText`, a version restore, a compare and
    /// Undo/Redo, never for typing.
    pub fn ime_body_epoch(&self) -> u64 {
        self.ime_body_epoch
    }

    /// Note that a command replaced the document or its whole body under this document id.
    ///
    /// The counter fails closed: if it were ever exhausted (unreachable in practice), the
    /// document owner id advances too, so a cached IME context still cannot be reused.
    pub fn note_body_replaced(&mut self) {
        self.ime_body_epoch = match self.ime_body_epoch.checked_add(1) {
            Some(epoch) => epoch,
            None => {
                self.document_id = self.document_id.saturating_add(1);
                u64::MAX
            }
        };
    }

    pub fn touch(&mut self) {
        self.rev = self.rev.wrapping_add(1);
        self.dirty = true;
    }

    /// Reading Highlight ranges for the current document (searched again after edits); empty when it's off.
    pub fn find_highlights(&mut self) -> &[(Pos, Pos)] {
        if !self.find.highlight {
            return &[];
        }
        if self.find.highlights.as_ref().is_none_or(|(r, _)| *r != self.rev) {
            let found = crate::cmd::edit::search(self, StoryRef::Body).unwrap_or_default();
            self.find.highlights = Some((self.rev, found));
        }
        self.find.highlights.as_ref().map(|(_, v)| v.as_slice()).unwrap_or(&[])
    }

    /// Whether the markup area beside the page can show balloons: markup shown in Print Layout
    /// (not Read Mode or pages side by side), comments not listed in the Comments pane instead,
    /// and Track Changes Options not set to show every revision inline.
    pub fn balloons_shown(&self) -> bool {
        let v = &self.view;
        v.show_markup
            && !v.comments_pane
            && !v.read_mode
            && !v.multi_page
            && v.mode == ViewMode::Print
            && self.prefs.markup.balloons != wordcraft_layout::display::BalloonMode::Inline
    }

    /// Show revisions in balloons, with the balloons on screen: tracked deletions in the body leave
    /// the text for balloons in the markup area. Where no balloons can be shown they stay inline,
    /// as in Word; so do deletions in headers, footers, notes and text boxes, which get no balloons.
    pub fn revisions_in_balloons(&self) -> bool {
        let m = &self.prefs.markup;
        self.balloons_shown() && m.balloons == wordcraft_layout::display::BalloonMode::Revisions && m.insertions_deletions
    }

    /// Initials for new comments: the ones set in File › Options, else the user name's.
    pub fn user_initials(&self) -> String {
        let set = self.initials.trim();
        if set.is_empty() { self.author.split_whitespace().filter_map(|w| w.chars().next()).collect() } else { set.to_string() }
    }

    /// The document's file name and full path for fields (`FILENAME`) and layout context.
    /// Returns `(filename, filepath)`. Unsaved documents with no explicit file name default
    /// consistently to their name, `("Document1", "Document1")` for the first. Document metadata title is never used
    /// as a filename fallback (#3449).
    pub fn resolved_filenames(&self) -> (String, String) {
        if let Some(path) = &self.path {
            let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| path.to_string_lossy().to_string());
            let full = path.to_string_lossy().to_string();
            (name, full)
        } else if let Some(name) = &self.file_name {
            (name.clone(), name.clone())
        } else {
            let name = untitled_name(self.untitled);
            (name.clone(), name)
        }
    }

    /// The region in effect (#838): the one the user chose ([`Prefs::region`]), else the
    /// system's.
    pub fn region(&self) -> wordcraft_geom::Region {
        match self.prefs.region.as_deref() {
            Some(tag) => wordcraft_geom::Region::from_tag(tag),
            None => self.system_region.clone(),
        }
    }

    /// The English of the region in effect (`en-GB` for a British region), or `None` when the
    /// region's language isn't English, it names no territory, or it is the United States (US
    /// English is the default anyway). An English interface writes new documents in it, as Word
    /// does on a British or Canadian system (#1045).
    pub fn region_english(&self) -> Option<String> {
        let r = self.region();
        (r.language == "en" && !r.territory.is_empty() && r.territory != "US")
            .then(|| format!("en-{}", r.territory))
            .and_then(|t| wordcraft_proof::lang::normalize_tag(&t))
    }

    /// The Spanish of the region in effect (`es-MX` on a Mexican system, `es-UY` on a Uruguayan
    /// one; the ISO 639-2 `spa` counts as Spanish), or Spain's (`es-ES`) when the region's
    /// language isn't Spanish or it names no territory. A Spanish interface writes new documents in it, so they are proofed in the
    /// writer's Spanish from the first word (#3360).
    pub fn region_spanish(&self) -> String {
        let r = self.region();
        (matches!(r.language.as_str(), "es" | "spa") && !r.territory.is_empty())
            .then(|| format!("es-{}", r.territory))
            .and_then(|t| wordcraft_proof::lang::normalize_tag(&t))
            .unwrap_or_else(|| "es-ES".into())
    }

    /// The unit rulers and dialogs show lengths in: the user's choice, else the region's.
    pub fn units(&self) -> wordcraft_geom::Unit {
        self.prefs.units.unwrap_or_else(|| self.region().units())
    }

    /// `pt` in [`Session::units`], written the region's way (`2,54 cm` in Poland).
    pub fn format_length(&self, pt: f32) -> String {
        self.region().format(self.units(), pt)
    }

    /// A new blank document (the Normal template) on the region's paper: Letter in the US and
    /// other Letter regions, A4 elsewhere (#838). Margins stay 2.54 cm (1") all round.
    pub fn blank_document(&self) -> Document {
        blank_document(&self.region())
    }

    /// The paragraph path the caret is in.
    pub fn caret_path(&self) -> &Path {
        &self.sel.focus.path
    }

    pub fn story(&self) -> StoryRef {
        self.sel.focus.story
    }

    /// Formatting typing would use at the caret: the caret's, except run direction. New text
    /// takes its direction from its own letters, as on screen, so Latin typed or inserted into
    /// reopened Hebrew isn't saved as right-to-left; the DOCX writer marks right-to-left pieces
    /// itself (#1543). The bidi language stays (it only applies to complex-script text).
    /// Over a selection, as in Word, typing takes the first selected character's formatting,
    /// whichever way the selection was made (#2879).
    pub fn typing_props(&self) -> CharProps {
        let (a, _) = self.sel.ordered();
        let first = match self.doc.para_at(&a) {
            Some(p) if self.pending.is_none() && !self.sel.is_collapsed() && a.off < p.len() => Some(p.props_of_char(a.off)),
            _ => None,
        };
        let mut c = match first {
            Some(props) => {
                // The selection is replaced, so the new text isn't part of a link or a note number.
                let mut out = if props.link.is_some() { props.unlinked() } else { props.clone() };
                if matches!(out.style.as_deref(), Some("FootnoteReference" | "EndnoteReference")) {
                    out.style = None;
                }
                out
            }
            None => self.caret_props(),
        };
        c.rtl = None;
        c
    }

    /// The caret's formatting, as Reveal Formatting shows it.
    pub fn caret_props(&self) -> CharProps {
        if let Some(p) = &self.pending {
            return p.clone();
        }
        let f = &self.sel.focus;
        let Some(p) = self.doc.para_at(f) else { return CharProps::default() };
        let props = p.props_at(f.off);
        // As in Word, text typed at either end of a hyperlink isn't part of it; only typing
        // inside the link extends it.
        let inside_link = props.link.is_some() && f.off > 0 && f.off < p.len() && p.props_of_char(f.off).link == props.link;
        // Text typed next to a symbol is in the text's font, not the symbol's (`w:sym`).
        let mut out = (if props.link.is_some() && !inside_link { props.unlinked() } else { props.clone() }).without_symbol();
        // A footnote or endnote number is raised by its character style; text typed after it is body text.
        if matches!(out.style.as_deref(), Some("FootnoteReference" | "EndnoteReference")) {
            out.style = None;
        }
        out
    }

    /// Words in the document, as the status bar shows them (see [`Prefs::count_notes`]).
    pub fn word_count(&self) -> usize {
        if self.prefs.count_notes { self.doc.word_count_including_notes() } else { self.doc.word_count() }
    }

    /// Selected plain text.
    pub fn selected_text(&self) -> String {
        // A column block or whole cells that aren't one run of the text: each piece's own text,
        // a line each, so the status bar and Word Count count those cells only.
        if let Some(segs) = self.column_segments() {
            return segs.iter().map(|(a, b)| self.doc.copy_range(a.min(b), a.max(b)).plain_text()).collect::<Vec<_>>().join("\n");
        }
        let (a, b) = self.sel.ordered();
        self.doc.copy_range(&a, &b).plain_text()
    }
}

/// A new blank document on `region`'s paper (see [`Session::blank_document`]).
pub fn blank_document(region: &wordcraft_geom::Region) -> Document {
    let mut d = Document::new();
    let (_, w, h) = region.paper();
    d.last_section.page_w = w;
    d.last_section.page_h = h;
    d
}
