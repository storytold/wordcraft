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

#[cfg(test)]
mod tests {
    use super::*;

    /// #5537: the clipboard's HTML carries each paragraph's spacing and indents, so a mail client
    /// lays the text out as WordCraft does. A page saved as HTML doesn't write them (yet).
    #[test]
    fn the_clipboard_fragment_writes_a_paragraphs_spacing_and_indents() {
        let chr = wordcraft_doc::CharProps::default();
        let mut spaced = wordcraft_doc::Paragraph::with_text("Quote", chr.clone());
        spaced.props.space_before = Some(6.0);
        spaced.props.space_after = Some(12.0);
        spaced.props.indent_left = Some(36.0);
        spaced.props.indent_first = Some(-18.0);
        let mut plain = wordcraft_doc::Paragraph::with_text("Plain", chr);
        plain.props.indent_right = Some(-0.0);
        let mut d = Document::new();
        d.body = vec![wordcraft_doc::para_block(spaced), wordcraft_doc::para_block(plain)];
        let fragment = export_fragment(&d);
        assert!(fragment.contains("<p style=\"margin-top:6pt;margin-bottom:12pt;margin-left:36pt;text-indent:-18pt\">"), "{fragment}");
        assert!(fragment.contains("<p style=\"margin-right:0pt\">"), "{fragment}");
        let page = export(&d);
        assert!(!page.contains("margin-") && !page.contains("text-indent"), "{page}");
    }

    /// #5537: quotes and list items in a mixed document stay delegated without paragraph spacing,
    /// while adjacent normal paragraphs still emit their spacing and indents.
    #[test]
    fn the_clipboard_fragment_delegates_non_spaced_blocks_cleanly() {
        let chr = wordcraft_doc::CharProps::default();
        let mut quote = wordcraft_doc::Paragraph::with_text("Quoted", chr.clone()).styled("Quote");
        quote.props.space_after = Some(10.0);
        let mut body = wordcraft_doc::Paragraph::with_text("Body", chr.clone());
        body.props.space_after = Some(8.0);
        let mut d = Document::new();
        let num = std::sync::Arc::make_mut(&mut d.numbering).add_list(wordcraft_doc::ListKind::Bullet);
        let mut item = wordcraft_doc::Paragraph::with_text("Item", chr).styled("ListParagraph");
        item.props.numbering = Some(wordcraft_doc::props::NumRef { num, level: 0 });
        item.props.space_after = Some(6.0);
        d.body = vec![wordcraft_doc::para_block(quote), wordcraft_doc::para_block(body), wordcraft_doc::para_block(item)];
        let fragment = export_fragment(&d);
        assert!(fragment.contains("<blockquote>\n<p>Quoted</p>\n</blockquote>"), "{fragment}");
        assert!(fragment.contains("<p style=\"margin-bottom:8pt\">Body</p>"), "{fragment}");
        assert!(fragment.contains("<ul><li>Item</li></ul>"), "{fragment}");
    }
}
