//! HTML import and export via `craft-html`.

use wordcraft_doc::{Document, SectionProps};

pub use craft_html::doc::{ImageLoader, PAGE_FONT, css_len_pt, decode, decode_as, decode_entity, esc, export_flow, parse, parse_with, unescape};

fn shared_section(page: &SectionProps) -> craft_wordmodel::SectionProps {
    let mut d = Document::new();
    d.last_section = page.clone();
    wordcraft_docx::to_shared_model(&d).map(|s| s.last_section).unwrap_or_default()
}

pub fn import(html: &str) -> Document {
    wordcraft_docx::from_shared_model(craft_html::doc::import(html)).unwrap_or_default()
}

pub fn import_page(html: &str, images: ImageLoader<'_>) -> Document {
    wordcraft_docx::from_shared_model(craft_html::doc::import_page(html, images)).unwrap_or_default()
}

pub fn import_with(html: &str, images: ImageLoader<'_>) -> Document {
    wordcraft_docx::from_shared_model(craft_html::doc::import_with(html, images)).unwrap_or_default()
}

pub fn import_on(html: &str, images: ImageLoader<'_>, page: &SectionProps) -> Document {
    wordcraft_docx::from_shared_model(craft_html::doc::import_on(html, images, &shared_section(page))).unwrap_or_default()
}

pub fn export(doc: &Document) -> String {
    match wordcraft_docx::to_shared_model(doc) {
        Ok(shared) => craft_html::doc::export(&shared),
        Err(_) => String::new(),
    }
}

pub fn export_fragment(doc: &Document) -> String {
    match wordcraft_docx::to_shared_model(doc) {
        Ok(shared) => craft_html::doc::export_fragment(&shared),
        Err(_) => String::new(),
    }
}
