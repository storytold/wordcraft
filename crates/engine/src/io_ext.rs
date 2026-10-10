//! Format crates registered with the I/O dispatcher.

use wordcraft_doc::Document;

/// Open a format other than plain text / JSON.
pub fn open(ext: &str, bytes: &[u8]) -> Option<Result<Document, String>> {
    match ext {
        "docx" | "docm" | "dotx" | "dotm" => Some(wordcraft_docx::read(bytes).map_err(|e| e.to_string())),
        "doc" | "dot" => Some(wordcraft_docbin::read(bytes).map_err(|e| e.to_string())),
        other => wordcraft_formats::import(other, bytes),
    }
}

/// Save to a format other than plain text / JSON.
pub fn save(ext: &str, doc: &Document) -> Option<Result<Vec<u8>, String>> {
    // .docx/.docm/.dotx/.dotm: the extension picks the package flavour Word expects.
    if let Some(flavor) = wordcraft_docx::Flavor::from_ext(ext) {
        return Some(wordcraft_docx::write_as(doc, flavor).map_err(|e| e.to_string()));
    }
    match ext {
        "png" => Some(render_png(doc, 0, 2.0)),
        "pdf" => Some(wordcraft_pdf::export(doc, &Default::default()).map_err(|e| e.to_string())),
        other => wordcraft_formats::export(other, doc),
    }
}

/// Render page `page` as PNG at `scale` px/pt.
pub fn render_png(doc: &Document, page: usize, scale: f32) -> Result<Vec<u8>, String> {
    let l = wordcraft_layout::layout(doc, &mut wordcraft_layout::LayoutCache::new(), &Default::default());
    let p = l.pages.get(page).ok_or_else(|| format!("no page {}", page + 1))?;
    let img = wordcraft_render::render_page(doc, p, scale, &Default::default());
    Ok(img.to_png())
}
