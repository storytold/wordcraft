//! OpenDocument Text (`.odt`, `.ott`, `.fodt`) import and export via `craft-odf`.

use std::sync::Arc;

use wordcraft_doc::{Block, Document, SectionProps};

pub use craft_odf::text::{CryptError, TEMPLATE_MIME, TEXT_MIME, decrypt, encrypt, is_encrypted};

/// Text room, points, that header and footer bands leave on a page (as Page Setup's margins must).
const MIN_TEXT_ROOM: f32 = 36.0;

fn shared_section(page: &SectionProps) -> craft_wordmodel::SectionProps {
    let mut d = Document::new();
    d.last_section = page.clone();
    wordcraft_docx::to_shared_model(&d).map(|s| s.last_section).unwrap_or_default()
}

/// A shared document opened as a WordCraft one, its pages keeping the text room the header and
/// footer bands leave ([`keep_text_room`]).
fn from_shared(shared: craft_wordmodel::Document) -> Result<Document, String> {
    let mut doc = wordcraft_docx::from_shared_model(shared).map_err(|e| e.to_string())?;
    keep_text_room(&mut doc);
    Ok(doc)
}

/// A page's header and footer bands grow its margins, and a band as tall as the page (to fit a
/// full-page picture, #5806) takes all the text's room: every page then draws one line below the
/// paper and the rest of the body is missing from the render and PDF. The bands grow the margins
/// only while the text keeps half an inch (Page Setup's rule for margins): the larger margin, the
/// larger band with it, gives up the excess first, so the smaller band is kept whole. The band's
/// own height is untouched: the header still draws as tall as its content.
///
/// `craft-odf` caps a band at the page's longer side rather than at the room left for text; this
/// is the guard that keeps the text on the page until the shared reader makes the same one.
fn keep_text_room(doc: &mut Document) {
    for b in &mut doc.body {
        if let Block::Para(p) = &mut *Arc::make_mut(b)
            && let Some(sec) = p.section.as_mut()
        {
            keep_section_room(sec);
        }
    }
    keep_section_room(&mut doc.last_section);
}

fn keep_section_room(sec: &mut SectionProps) {
    if !(sec.page_h.is_finite() && sec.margin_top.is_finite() && sec.margin_bottom.is_finite()) {
        return;
    }
    let mut excess = sec.margin_top + sec.margin_bottom - (sec.page_h - MIN_TEXT_ROOM);
    if excess <= 0.0 {
        return;
    }
    let (first, second) = if sec.margin_top >= sec.margin_bottom {
        (&mut sec.margin_top, &mut sec.margin_bottom)
    } else {
        (&mut sec.margin_bottom, &mut sec.margin_top)
    };
    let cut = excess.min(first.max(0.0));
    *first -= cut;
    excess -= cut;
    if excess > 0.0 {
        *second = (*second - excess).max(0.0);
    }
}

pub fn import(bytes: &[u8]) -> Result<Document, String> {
    from_shared(craft_odf::text::import(bytes)?)
}

pub fn import_on(bytes: &[u8], page: &SectionProps) -> Result<Document, String> {
    from_shared(craft_odf::text::import_on(bytes, &shared_section(page))?)
}

pub fn import_flat(bytes: &[u8]) -> Result<Document, String> {
    from_shared(craft_odf::text::import_flat(bytes)?)
}

pub fn import_flat_on(bytes: &[u8], page: &SectionProps) -> Result<Document, String> {
    from_shared(craft_odf::text::import_flat_on(bytes, &shared_section(page))?)
}

pub fn export(doc: &Document) -> Result<Vec<u8>, String> {
    let shared = wordcraft_docx::to_shared_model(doc).map_err(|e| e.to_string())?;
    craft_odf::text::export(&shared)
}

pub fn export_template(doc: &Document) -> Result<Vec<u8>, String> {
    let shared = wordcraft_docx::to_shared_model(doc).map_err(|e| e.to_string())?;
    craft_odf::text::export_template(&shared)
}

pub fn export_flat(doc: &Document) -> Result<Vec<u8>, String> {
    let shared = wordcraft_docx::to_shared_model(doc).map_err(|e| e.to_string())?;
    craft_odf::text::export_flat(&shared)
}
