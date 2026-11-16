//! HTML import and export via `craft-html`.

use wordcraft_doc::{Document, SectionProps};

pub use craft_html::doc::{ImageLoader, PAGE_FONT, css_len_pt, decode, decode_as, decode_entity, esc, export_flow, parse, parse_with, unescape};

fn shared_section(page: &SectionProps) -> craft_wordmodel::SectionProps {
    let mut d = Document::new();
    d.last_section = page.clone();
    wordcraft_docx::to_shared_model(&d).map(|s| s.last_section).unwrap_or_default()
}

fn normalize_imported_doc(mut doc: Document) -> Document {
    fn normalize_para(p: &mut wordcraft_doc::Paragraph) {
        if p.props.shading.is_some() {
            return;
        }
        let Some(first_shading) = p.runs.first().and_then(|r| r.props.shading) else {
            return;
        };
        if p.runs.iter().all(|r| r.props.shading == Some(first_shading)) {
            p.props.shading = Some(first_shading);
            for r in &mut p.runs {
                r.props.shading = None;
            }
        }
    }

    fn normalize_blocks(blocks: &mut [std::sync::Arc<wordcraft_doc::Block>]) {
        for b in blocks {
            let b = std::sync::Arc::make_mut(b);
            match b {
                wordcraft_doc::Block::Para(p) => normalize_para(p),
                wordcraft_doc::Block::Table(t) => {
                    for row in &mut t.rows {
                        for cell in &mut row.cells {
                            normalize_blocks(&mut cell.blocks);
                        }
                    }
                }
            }
        }
    }

    normalize_blocks(&mut doc.body);
    for part in doc.parts.values_mut() {
        normalize_blocks(&mut part.blocks);
    }
    doc
}

pub fn import(html: &str) -> Document {
    normalize_imported_doc(wordcraft_docx::from_shared_model(craft_html::doc::import(html)).unwrap_or_default())
}

pub fn import_page(html: &str, images: ImageLoader<'_>) -> Document {
    normalize_imported_doc(wordcraft_docx::from_shared_model(craft_html::doc::import_page(html, images)).unwrap_or_default())
}

pub fn import_with(html: &str, images: ImageLoader<'_>) -> Document {
    normalize_imported_doc(wordcraft_docx::from_shared_model(craft_html::doc::import_with(html, images)).unwrap_or_default())
}

pub fn import_on(html: &str, images: ImageLoader<'_>, page: &SectionProps) -> Document {
    normalize_imported_doc(wordcraft_docx::from_shared_model(craft_html::doc::import_on(html, images, &shared_section(page))).unwrap_or_default())
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
