//! OpenDocument Text (`.odt`, `.ott`, `.fodt`) import and export via `craft-odf`.

use wordcraft_doc::{Document, SectionProps};

pub use craft_odf::text::{CryptError, TEMPLATE_MIME, TEXT_MIME, decrypt, encrypt, is_encrypted};

fn shared_section(page: &SectionProps) -> craft_wordmodel::SectionProps {
    let mut d = Document::new();
    d.last_section = page.clone();
    wordcraft_docx::to_shared_model(&d).map(|s| s.last_section).unwrap_or_default()
}

pub fn import(bytes: &[u8]) -> Result<Document, String> {
    let shared = craft_odf::text::import(bytes)?;
    wordcraft_docx::from_shared_model(shared).map_err(|e| e.to_string())
}

pub fn import_on(bytes: &[u8], page: &SectionProps) -> Result<Document, String> {
    let shared = craft_odf::text::import_on(bytes, &shared_section(page))?;
    wordcraft_docx::from_shared_model(shared).map_err(|e| e.to_string())
}

pub fn import_flat(bytes: &[u8]) -> Result<Document, String> {
    let shared = craft_odf::text::import_flat(bytes)?;
    wordcraft_docx::from_shared_model(shared).map_err(|e| e.to_string())
}

pub fn import_flat_on(bytes: &[u8], page: &SectionProps) -> Result<Document, String> {
    let shared = craft_odf::text::import_flat_on(bytes, &shared_section(page))?;
    wordcraft_docx::from_shared_model(shared).map_err(|e| e.to_string())
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
