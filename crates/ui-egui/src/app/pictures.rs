use super::{Value, WordApp, json};

/// Whether a pasted picture belongs in the document: the page has keyboard focus, and no
/// Backstage or dialog is open (#1349). A focused text field (comment, Find box) means the page
/// does not have focus, so `canvas_focused` already covers it; egui's own "wants keyboard input"
/// is true for the focused page itself and can't be used.
fn paste_target(canvas_focused: bool, backstage: bool, dialog: bool) -> bool {
    canvas_focused && !backstage && !dialog
}

impl WordApp {
    /// Select the picture or icon the person just inserted or pasted, so it shows its handles.
    /// `insert.picture` leaves the caret after it, which scripts and agents keep so their next
    /// typing doesn't replace it (#965).
    pub(crate) fn select_inserted_picture(&mut self, r: &Result<Value, String>) {
        if !r.as_ref().is_ok_and(|v| v.get("media").is_some() && v.get("canvasMember").is_none()) || !self.session.sel.is_collapsed() {
            return;
        }
        let end = self.session.sel.focus.clone();
        let Some(off) = end.off.checked_sub(wordcraft_doc::para::OBJ.len_utf8()) else { return };
        let start = wordcraft_doc::Pos { off, ..end.clone() };
        if self.session.doc.para_at(&start).and_then(|p| p.object_at(off)).is_some_and(|o| o.is_drawing()) {
            self.session.sel = wordcraft_engine::Selection { anchor: start, focus: end };
        }
    }

    /// Media key of the selected picture, if any.
    pub(crate) fn selected_picture_media(&self) -> Option<String> {
        match wordcraft_engine::cmd::objects::selected(&self.session) {
            Some((_, wordcraft_doc::para::InlineObject::Image { media, .. })) => Some(media),
            _ => None,
        }
    }

    /// Inserting a picture selects it, and the next insert would replace it. When several images
    /// arrive in one drop, put the caret after the picture placed by the previous file (#1592).
    pub(super) fn caret_after_placed_picture(&mut self, placed: bool) {
        if placed {
            self.session.sel.anchor = self.session.sel.focus.clone();
        }
    }

    /// Whether `r` placed a picture in the text (and selected it). One taken into a selected
    /// Drawing Canvas leaves the canvas selected, so the next file of the drop joins it too.
    pub(super) fn placed_in_text(r: &Result<Value, String>) -> bool {
        r.as_ref().is_ok_and(|v| v.get("canvasMember").is_none())
    }

    /// A picked image replaces the pending Change Picture target, else it is inserted.
    pub(crate) fn insert_or_change_picture(&mut self, params: Value) -> Result<Value, String> {
        let target = std::mem::take(&mut self.change_picture_target);
        match (target, self.selected_picture_media()) {
            (Some(t), Some(m)) if t == m => self.run("picture.change", params),
            _ => self.run("insert.picture", params),
        }
    }

    /// Drop a pending Change Picture once the selection moved to another picture.
    pub(crate) fn clear_stale_change_picture(&mut self) {
        let stale = match (&self.change_picture_target, self.selected_picture_media()) {
            (Some(t), Some(m)) => t != &m,
            (Some(_), None) => true,
            _ => false,
        };
        if stale {
            self.change_picture_target = None;
        }
    }

    /// Pictures pasted in the browser: inserted once when the page is the edit target, dropped
    /// otherwise so the focused field's own paste applies (a text field ignores a picture).
    pub(crate) fn drain_pasted(&mut self) {
        let Some(pasted) = self.services.pasted.clone() else { return };
        let pictures = std::mem::take(&mut *pasted.lock().unwrap_or_else(std::sync::PoisonError::into_inner));
        let target = paste_target(self.canvas.focused, self.ui.backstage, self.dialog.is_some() || self.file_dialog_open());
        for (_, bytes) in pictures {
            if !target {
                log::debug!("pasted picture ignored: the document is not the edit target");
                continue;
            }
            let _ = self.insert_or_change_picture(json!({"data": wordcraft_engine::cmd::insert::base64_encode(&bytes)}));
        }
    }
}
