//! LaTeX (`.tex`) import and export via `craft-latex`.

use wordcraft_doc::{Document, SectionProps};

pub use craft_latex::{escape_text, latex_to_linear, linear_to_latex};

pub fn import(bytes: &[u8]) -> Document {
    wordcraft_docx::from_shared_model(craft_latex::import(bytes)).unwrap_or_default()
}

pub fn import_on(bytes: &[u8], page: &SectionProps) -> Document {
    let mut d = import(bytes);
    d.last_section = page.clone();
    d
}

pub fn parse(text: &str) -> craft_flow::Flow {
    craft_latex::parse(text)
}

pub fn export(doc: &Document) -> String {
    match wordcraft_docx::to_shared_model(doc) {
        Ok(shared) => craft_latex::export(&shared),
        Err(_) => String::new(),
    }
}
