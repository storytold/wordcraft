use super::doc_state::{Selection, TypedWord, typed_range};
use super::locale::SYSTEM_LOCALE;
use super::snapshot_types::LayoutEntry;
use super::view::DRAFT_WIDTH;
use super::{Arc, DocLayout, Document, LayoutCache, LayoutOptions, Session, StoryRef, Value, ViewMode};

impl Session {
    /// The current layout (recomputed when the document or view changed).
    pub fn layout(&mut self) -> Arc<DocLayout> {
        let typing = self.typing_word();
        self.layout_with(typing)
    }

    /// [`Session::layout`] with `typing` as the word being typed.
    fn layout_with(&mut self, typing: Option<wordcraft_layout::TypingWord>) -> Arc<DocLayout> {
        let mode = if self.view.read_mode { ViewMode::Read } else { self.view.mode };
        let width = match mode {
            ViewMode::Read => self.view.read_width,
            ViewMode::Web => self.view.web_width,
            _ => DRAFT_WIDTH,
        };
        let height = if self.view.read_mode { self.view.read_height } else { 0.0 };
        let spacing = if self.view.read_mode { self.view.read_line_spacing.map(f32::to_bits) } else { None };
        // Deletions in balloons depend on the view (the Comments pane, Read Mode, side by side), so
        // a view change that moves them in or out of the text lays it out again.
        let hide_deleted = !self.view.show_markup;
        let deleted = (hide_deleted, self.prefs.markup.hidden_deletions(), self.revisions_in_balloons());
        let outline = (mode == ViewMode::Outline).then(|| self.view.outline.view());
        let shown = (self.view.marks || self.prefs.marks.hidden, self.view.field_codes);
        let (rev, proofing, numeral) = (self.rev, self.view.proofing, self.view.numeral);
        let fits = |e: &Option<LayoutEntry>| {
            e.as_ref().is_some_and(|(r, w, h, m, sp, _, pf, o, hd, n, t, sh)| {
                *r == rev
                    && (*w == width || mode == ViewMode::Print)
                    && *h == height
                    && *m == mode
                    && *sp == spacing
                    && *pf == proofing
                    && *n == numeral
                    && *hd == deleted
                    && *o == outline
                    && *t == typing
                    && *sh == shown
            })
        };
        if !fits(&self.layout[0]) && fits(&self.layout[1]) {
            self.layout.swap(0, 1);
        }
        if fits(&self.layout[0])
            && let Some(e) = &self.layout[0]
        {
            return e.5.clone();
        }
        let (fn_name, fn_path) = self.resolved_filenames();
        let (filename, filepath): (Option<Arc<str>>, Option<Arc<str>>) = (Some(Arc::from(fn_name.as_str())), Some(Arc::from(fn_path.as_str())));
        let opts = LayoutOptions {
            view: mode,
            web_width: width,
            read_height: height,
            read_line_spacing: spacing.map(f32::from_bits),
            show_hidden: shown.0,
            hide_deleted,
            hide_marked: deleted.1,
            deletions_in_balloons: deleted.2,
            proofing: self.view.proofing,
            numeral: self.view.numeral.resolve(SYSTEM_LOCALE.get().map(String::as_str)),
            comment_balloons: false,
            field_codes: shown.1,
            outline: outline.clone(),
            proof: wordcraft_layout::ProofOptions { grammar: self.prefs.mark_grammar, spelling: self.prefs.spelling() },
            typing_word: typing,
            filename: filename.clone(),
            filepath: filepath.clone(),
        };
        let l = Arc::new(wordcraft_layout::layout(&self.doc, &mut self.cache, &opts));
        let [newest, _] = std::mem::take(&mut self.layout);
        self.layout = [
            Some((self.rev, width, height, mode, spacing, l.clone(), self.view.proofing, outline, deleted, self.view.numeral, typing, shown)),
            newest,
        ];
        l
    }

    /// The recorded word while the caret and the word are as typing left them; else drops it.
    fn typing_word(&mut self) -> Option<wordcraft_layout::TypingWord> {
        let t = self.typing.as_ref()?;
        let p = if self.sel.is_collapsed() && self.sel.focus == t.at { self.doc.para_at(&t.at) } else { None };
        match p {
            Some(p) if typed_range(&p.text, t.at.off) == Some((t.start, t.end)) => {
                Some(wordcraft_layout::TypingWord { rev: p.rev, start: t.start, end: t.end })
            }
            _ => {
                self.typing = None;
                None
            }
        }
    }

    /// After a successful command: a `text.*` edit at the caret records the word being typed;
    /// any other caret move ends it.
    pub(super) fn note_typing(&mut self, id: &str, before: Option<&Document>, sel_before: &Selection) {
        let at = &self.sel.focus;
        let edited = || before.is_some_and(|d| d.para_at(at).map(|p| &p.text) != self.doc.para_at(at).map(|p| &p.text));
        if id.starts_with("text.") && edited() {
            self.typing = self.word_being_typed();
        } else if self.sel != *sel_before {
            self.typing = None;
        }
    }

    /// The word the collapsed caret ends or sits in, as [`TypedWord`].
    fn word_being_typed(&self) -> Option<TypedWord> {
        if !self.sel.is_collapsed() {
            return None;
        }
        let at = self.sel.focus.clone();
        let (start, end) = typed_range(&self.doc.para_at(&at)?.text, at.off)?;
        Some(TypedWord { at, start, end })
    }

    /// The page as command `id` would leave it, for a live preview (Insert › Table's grid, #3931):
    /// the command runs on a copy of the document, with this caret and view, and the copy is laid
    /// out. The copy comes back with its layout, which is drawn from it: the layout's paragraph
    /// paths name the copy's paragraphs, not this document's, and so is the copy's selection,
    /// which is where the caret would be. The document, the selection, the undo history and Repeat
    /// are untouched. `None` when the command fails or isn't allowed here.
    pub fn preview_layout(&mut self, id: &str, params: &Value) -> Option<(Document, Selection, Arc<DocLayout>)> {
        // The word being typed keeps going without a squiggle on the preview: the command moves
        // the copy's caret off it and may split its paragraph, so it is found again by its place.
        let typing = self.typing_word();
        let at = self.sel.focus.clone();
        let mut copy = Session::with_registry(self.doc.clone(), self.registry.clone());
        copy.sel = self.sel.clone();
        copy.column = self.column.clone();
        copy.widened = self.widened.clone();
        copy.view = self.view.clone();
        copy.prefs = self.prefs.clone();
        copy.path = self.path.clone();
        copy.file_name = self.file_name.clone();
        copy.author = self.author.clone();
        copy.last_highlight = self.last_highlight;
        copy.system_region = self.system_region.clone();
        copy.editing_lang = self.editing_lang.clone();
        // The copy shares the paragraph layouts, so only what the command changed is laid out.
        std::mem::swap(&mut copy.cache, &mut self.cache);
        let layout = copy.run(id, params).is_ok().then(|| {
            let typing = typing.and_then(|t| {
                let p = copy.doc.para_at(&at)?;
                (t.end <= p.len()).then_some(wordcraft_layout::TypingWord { rev: p.rev, ..t })
            });
            copy.layout_with(typing)
        });
        std::mem::swap(&mut copy.cache, &mut self.cache);
        layout.map(|l| (copy.doc, copy.sel, l))
    }

    /// A layout for output (PDF, images, print): no proofing marks, print view.
    pub fn export_layout(&self) -> Arc<DocLayout> {
        self.export_layout_with(false)
    }

    /// [`Session::export_layout`]; with `markup` (Print Markup) each page also has the markup
    /// area with the comments in balloons.
    pub fn export_layout_with(&self, markup: bool) -> Arc<DocLayout> {
        let (fn_name, fn_path) = self.resolved_filenames();
        let opts = LayoutOptions {
            view: ViewMode::Print,
            comment_balloons: markup && self.prefs.markup.comments,
            numeral: self.view.numeral.resolve(SYSTEM_LOCALE.get().map(String::as_str)),
            filename: Some(Arc::from(fn_name.as_str())),
            filepath: Some(Arc::from(fn_path.as_str())),
            ..Default::default()
        };
        Arc::new(wordcraft_layout::layout(&self.doc, &mut LayoutCache::new(), &opts))
    }

    /// The paragraph layout cache's running (hits, misses): a miss is a paragraph laid out
    /// afresh. Benchmarks and performance tests read it to count work, not wall time.
    pub fn layout_cache_stats(&self) -> (u64, u64) {
        (self.cache.hits, self.cache.misses)
    }

    /// Invalidate the cached layout (fonts changed etc.).
    pub fn relayout(&mut self) {
        self.layout = Default::default();
        self.cache.clear();
    }

    /// Set the language new documents are written in ([`Session::editing_lang`]; `None` or a
    /// malformed tag: US English). A document nobody has touched yet — untitled, unchanged, no
    /// undo steps, no text — switches too, so the blank document a session starts with follows
    /// the saved proofing default, or the interface language when none is saved. Nothing else
    /// changes and no undo step is added.
    pub fn set_editing_language(&mut self, tag: Option<&str>) {
        let tag = self
            .prefs
            .default_language
            .as_deref()
            .and_then(wordcraft_proof::lang::normalize_tag)
            .or_else(|| tag.and_then(wordcraft_proof::lang::normalize_tag));
        if self.editing_lang == tag {
            return;
        }
        self.editing_lang = tag;
        let untouched = self.path.is_none() && !self.dirty && self.history.is_empty() && self.redo.is_empty();
        if untouched && self.doc.plain_text(StoryRef::Body).trim().is_empty() && self.doc.paragraph_count() <= 1 {
            self.doc.styles.default_chr.lang = Some(self.editing_lang.clone().unwrap_or_else(|| "en-US".into()));
            self.rev = self.rev.wrapping_add(1);
            // Not an edit: the document is as untouched as it was.
            if self.made_rev == self.rev.wrapping_sub(1) {
                self.made_rev = self.rev;
            }
            self.relayout();
        }
    }
}
