use super::{QUIT, WordApp, i18n, theme};

impl WordApp {
    /// Fonts were added or removed (a command, the web shell loading stored fonts): lay out
    /// again and refresh the font lists and their previews.
    pub(crate) fn watch_fonts(&mut self, ctx: &egui::Context) {
        let db = wordcraft_fonts::FontDb::global();
        let generation = db.generation();
        if generation == self.font_generation {
            return;
        }
        self.font_generation = generation;
        self.session.relayout();
        self.previews.fonts_changed(ctx, db.user_fonts().iter().flat_map(|f| f.families()));
        ctx.request_repaint();
    }

    /// The window caption is painted with the semibold interface face (Inter). A file name in a
    /// script those faces lack would otherwise be drawn as `?`. Borrow the document fonts'
    /// fallback faces, as IME composition does (#565). A CJK name uses the CJK UI font instead:
    /// those collections are too large to add here.
    pub(super) fn ensure_caption_fonts(&mut self) {
        let stem = self.title_stem();
        if stem.chars().any(theme::is_hangul_or_cjk) {
            self.want_system_cjk = true;
        }
        let new = theme::faces_for_preedit(&stem, &self.canvas.ime_faces, theme::caption_char_covered, |c| {
            wordcraft_fonts::FontDb::global().fallback_for(c, u32::MAX)
        });
        if !new.is_empty() {
            self.canvas.ime_faces.extend(new);
            // Fonts are already installed for this frame's language; load the new faces too.
            self.styled = false;
        }
    }

    /// Comment balloons and the Comments pane draw comment text and author names with the
    /// interface faces, which have no Arabic, Hebrew, Thai …: each such letter was drawn as a box.
    /// Borrow the document fonts' fallback faces for them, as the caption does. Looked at once per
    /// document revision; CJK text uses the CJK UI font instead.
    pub(super) fn ensure_comment_fonts(&mut self) {
        if !self.want_system_cjk
            && (wordcraft_engine::cmd::smart_art::selected_smart_art(&self.session)
                .is_some_and(|g| g.items.iter().any(|it| it.text.chars().any(theme::is_hangul_or_cjk)))
                || self.session.clip_history.items().iter().any(|i| i.preview().chars().any(theme::is_hangul_or_cjk)))
        {
            self.want_system_cjk = true;
        }
        let key = (self.session.document_id(), self.session.rev());
        if self.comment_fonts_for == Some(key) {
            return;
        }
        self.comment_fonts_for = Some(key);
        let doc = &self.session.doc;
        let mut chars = std::collections::BTreeSet::new();
        for c in doc.comments.values() {
            chars.extend(c.author.chars());
            chars.extend(doc.plain_text(wordcraft_doc::StoryRef::Part(c.part)).chars());
        }
        if chars.iter().any(|c| theme::is_hangul_or_cjk(*c))
            || (!self.want_system_cjk
                && (blocks_have_cjk(&doc.body, 0)
                    || doc.parts.values().any(|part| blocks_have_cjk(&part.blocks, 0))
                    || doc.styles.styles.iter().any(|s| !s.builtin && s.name.chars().any(theme::is_hangul_or_cjk))
                    || doc.revisions.iter().any(|r| r.author.chars().any(theme::is_hangul_or_cjk))))
        {
            self.want_system_cjk = true;
        }
        let text: String = chars.into_iter().collect();
        let new = theme::faces_for_preedit(&text, &self.canvas.ime_faces, theme::caption_char_covered, |c| {
            wordcraft_fonts::FontDb::global().fallback_for(c, u32::MAX)
        });
        if !new.is_empty() {
            self.canvas.ime_faces.extend(new);
            self.styled = false;
        }
    }

    /// File › Recover draws the recovered documents' names with the interface faces too: a
    /// Chinese name was a row of boxes, an Arabic or Hebrew one likewise. The same treatment as
    /// the caption (and as the Recent list, `ensure_recent_fonts`): CJK loads the CJK UI font,
    /// other scripts borrow the document fallback faces. Looked at again only when the names change.
    pub(super) fn ensure_recover_fonts(&mut self) {
        use std::hash::{Hash, Hasher};
        let docs = self.session.recovery_documents();
        let mut hasher = std::hash::DefaultHasher::new();
        docs.iter().for_each(|d| d.name.hash(&mut hasher));
        let key = hasher.finish();
        if self.recover_fonts_for == Some(key) {
            return;
        }
        self.recover_fonts_for = Some(key);
        let chars: std::collections::BTreeSet<char> = docs.iter().flat_map(|d| d.name.chars()).collect();
        if chars.iter().any(|c| theme::is_hangul_or_cjk(*c)) {
            self.want_system_cjk = true;
        }
        let text: String = chars.into_iter().collect();
        let new = theme::faces_for_preedit(&text, &self.canvas.ime_faces, theme::caption_char_covered, |c| {
            wordcraft_fonts::FontDb::global().fallback_for(c, u32::MAX)
        });
        if !new.is_empty() {
            self.canvas.ime_faces.extend(new);
            self.styled = false;
        }
    }

    /// Recent documents in Backstage (and the Start screen) show file names and paths with the
    /// interface faces. Borrow document fallback faces for characters in non-Latin scripts (#2067),
    /// as the caption and comments do.
    pub(super) fn ensure_recent_fonts(&mut self) {
        if self.recent_fonts_for.as_slice() == self.ui.recent.as_slice() {
            return;
        }
        self.recent_fonts_for = self.ui.recent.clone();
        for p in &self.ui.recent {
            if p.chars().any(theme::is_hangul_or_cjk) {
                self.want_system_cjk = true;
            }
            let new = theme::faces_for_preedit(p, &self.canvas.ime_faces, theme::caption_char_covered, |c| {
                wordcraft_fonts::FontDb::global().fallback_for(c, u32::MAX)
            });
            if !new.is_empty() {
                for face in new {
                    if !self.canvas.ime_faces.iter().any(|f| f.id() == face.id()) {
                        self.canvas.ime_faces.push(face);
                    }
                }
                self.styled = false;
            }
        }
    }

    /// The Find box and the Navigation pane's Results list draw the typed query and document
    /// excerpts around each match with the interface faces, which have no Georgian, Armenian,
    /// Thai … (#3248): each such letter was drawn as a box. Borrow the document fonts' fallback
    /// faces for them, as comments and the caption do. Looked at once per query/result-count/
    /// revision change (an edit can change a result's surrounding text — adding Georgian beside
    /// an existing Latin match — without changing the query or how many matches there are, #3257
    /// review); CJK text uses the CJK UI font instead.
    pub(super) fn ensure_find_fonts(&mut self) {
        let key = (self.session.find.query.clone(), self.session.find.results.len(), self.session.rev());
        if self.find_fonts_for.as_ref() == Some(&key) {
            return;
        }
        self.find_fonts_for = Some(key);
        let mut text = self.session.find.query.clone();
        for (a, b) in self.session.find.results.iter().take(500) {
            let Some(p) = self.session.doc.para_at(a) else { continue };
            let from = p.clamp(a.off.saturating_sub(30));
            let to = p.clamp((b.off + 40).min(p.len()));
            if let Some(s) = p.text.get(from..to) {
                text.push_str(s);
            }
        }
        if text.chars().any(theme::is_hangul_or_cjk) {
            self.want_system_cjk = true;
        }
        let new = theme::faces_for_preedit(&text, &self.canvas.ime_faces, theme::caption_char_covered, |c| {
            wordcraft_fonts::FontDb::global().fallback_for(c, u32::MAX)
        });
        if !new.is_empty() {
            self.canvas.ime_faces.extend(new);
            self.styled = false;
        }
    }

    /// Ensure interface/fallback fonts for `text` (an input method's composition or incoming
    /// text) are installed in `ctx` so Korean/CJK and non-Latin compositions render cleanly even
    /// when the interface language is English.
    pub(crate) fn ensure_text_fonts(&mut self, ctx: &egui::Context, text: &str) {
        if text.is_empty() || text.is_ascii() {
            return;
        }
        if text.chars().any(theme::is_hangul_or_cjk) {
            self.want_system_cjk = true;
        }
        let new = theme::faces_for_preedit(text, &self.canvas.ime_faces, theme::caption_char_covered, |c| {
            wordcraft_fonts::FontDb::global().fallback_for(c, u32::MAX)
        });
        let need_system_cjk = self.want_system_cjk && !self.fonts_system_cjk;
        if !new.is_empty() || need_system_cjk {
            for face in new {
                if !self.canvas.ime_faces.iter().any(|f| f.id() == face.id()) {
                    self.canvas.ime_faces.push(face);
                }
            }
            self.styled = false;
            let lang = i18n::Lang::from_pref(&self.ui.language);
            self.install_ui_fonts(ctx, lang);
            ctx.request_repaint();
        }
    }

    /// Ensure interface/fallback fonts for the active canvas IME pre-edit are installed in `ctx`.
    pub(crate) fn ensure_preedit_fonts(&mut self, ctx: &egui::Context) {
        if self.canvas.ime_preedit.is_empty() || self.canvas.ime_preedit.is_ascii() {
            return;
        }
        let preedit = self.canvas.ime_preedit.clone();
        self.ensure_text_fonts(ctx, &preedit);
    }

    /// Install the interface fonts when the language, the CJK fallback, or a caption fallback
    /// face has changed since the last install.
    pub(super) fn install_ui_fonts(&mut self, ctx: &egui::Context, lang: i18n::Lang) {
        let system_cjk = self.fonts_system_cjk || self.want_system_cjk || lang.uses_cjk();
        if !self.styled
            || lang.prefers_hans() != self.fonts_hans
            || lang.prefers_korean() != self.fonts_korean
            || lang.prefers_hant() != self.fonts_hant
            || system_cjk != self.fonts_system_cjk
        {
            // Every window's borrowed faces: the fonts are the app's.
            let mut faces = self.canvas.ime_faces.clone();
            for w in self.other_windows() {
                for face in &w.canvas.ime_faces {
                    if !faces.iter().any(|f| f.id() == face.id()) {
                        faces.push(face.clone());
                    }
                }
            }
            theme::install_fonts_language_extra(ctx, lang.prefers_hans(), lang.prefers_korean(), lang.prefers_hant(), system_cjk, &faces);
            self.fonts_hans = lang.prefers_hans();
            self.fonts_korean = lang.prefers_korean();
            self.fonts_hant = lang.prefers_hant();
            self.fonts_system_cjk = system_cjk;
            // Mod with -, = and 0 are Word shortcuts (optional hyphen, subscript, paragraph spacing);
            // egui's keyboard zoom would also scale the whole window on them. Zoom is View › Zoom.
            // egui quits on Command-Q, which is Ctrl+Q off the Mac: Word's Reset Paragraph
            // Formatting there (#1218). Keep ⌘Q, which only the Mac sends. egui would close only the
            // window it was pressed in, so with native windows the app quits itself
            // ([`WordApp::window_logic_before`]).
            let quit = (!self.windows.native()).then_some(QUIT).into_iter().collect();
            ctx.options_mut(|o| {
                o.zoom_with_keyboard = false;
                o.quit_shortcuts = quit;
            });
            self.styled = true;
        }
    }
}

/// Zero-allocation in-place scan for Hangul/CJK characters across `blocks`, skipping pure-ASCII
/// paragraphs without building temporary story `String`s.
fn blocks_have_cjk(blocks: &[std::sync::Arc<wordcraft_doc::Block>], depth: usize) -> bool {
    if depth >= 16 {
        return false;
    }
    blocks.iter().any(|b| match b.as_ref() {
        wordcraft_doc::Block::Para(p) => !p.text.is_ascii() && p.text.chars().any(theme::is_hangul_or_cjk),
        wordcraft_doc::Block::Table(t) => t.rows.iter().flat_map(|r| &r.cells).any(|c| blocks_have_cjk(&c.blocks, depth + 1)),
    })
}
