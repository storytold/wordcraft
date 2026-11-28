//! Rich Text Format (`.rtf`) import and export via `craft-rtf`.

use wordcraft_doc::Document;

pub fn import(bytes: &[u8]) -> Result<Document, String> {
    let shared = craft_rtf::doc::import(bytes)?;
    wordcraft_docx::from_shared_model(shared).map_err(|e| e.to_string())
}

pub fn parse(bytes: &[u8]) -> Result<crate::model::Flow, String> {
    let shared = craft_rtf::doc::import(bytes)?;
    Ok(craft_flow::from_doc(&shared))
}

pub fn export(doc: &Document) -> String {
    let Ok(shared) = wordcraft_docx::to_shared_model(doc) else {
        return String::new();
    };
    craft_rtf::doc::export(&shared)
}
