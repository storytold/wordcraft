//! HTML import and export via `craft-html`.

use craft_flow::{FBlock, Flow, Kind, Para, Spacing};
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

/// Format a point measurement without trailing decimal zeros.
fn trim_num(v: f32) -> String {
    let s = format!("{:.2}", v);
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// CSS declaration for a paragraph's horizontal alignment, when not left-aligned.
fn align_css(a: Option<craft_wordmodel::Align>) -> Option<&'static str> {
    match a? {
        craft_wordmodel::Align::Center => Some("text-align:center"),
        craft_wordmodel::Align::Right => Some("text-align:right"),
        craft_wordmodel::Align::Justify | craft_wordmodel::Align::Distribute => Some("text-align:justify"),
        craft_wordmodel::Align::Left => None,
    }
}

/// A paragraph's spacing and indents as CSS declarations in points: margins for the space before
/// and after and the left and right indents, and `text-indent` for a first-line indent (negative:
/// hanging).
fn spacing_css(s: &Spacing) -> Vec<String> {
    let mut css = Vec::new();
    for (name, v) in [("margin-top", s.before), ("margin-right", s.right), ("margin-bottom", s.after), ("margin-left", s.left)] {
        if let Some(v) = v.filter(|v| v.is_finite()) {
            // `+ 0.0` turns -0.0 into 0.0, which would otherwise print as "-0".
            css.push(format!("{name}:{}pt", trim_num(v + 0.0)));
        }
    }
    if let Some(v) = s.first.filter(|v| v.is_finite() && *v != 0.0) {
        css.push(format!("text-indent:{}pt", trim_num(v)));
    }
    css
}

/// A paragraph's ` style="…"` attribute (empty when it has nothing to say): its alignment, its
/// spacing and indents, and a page break before it.
fn spaced_para_attrs(p: &Para) -> String {
    let mut css = Vec::new();
    if let Some(a) = align_css(p.align) {
        css.push(a.to_string());
    }
    css.extend(spacing_css(&p.spacing));
    if p.page_break {
        css.push("break-before:page;page-break-before:always".to_string());
    }
    if css.is_empty() { String::new() } else { format!(" style=\"{}\"", css.join(";")) }
}

/// Render a top-level plain paragraph, heading or rule for a clipboard fragment, keeping its
/// spacing and indents.
fn spaced_para_html(p: &Para) -> String {
    if p.kind == Kind::Rule {
        return "<hr>".to_string();
    }
    let body = craft_html::doc::inlines_html(&p.inlines);
    let attrs = spaced_para_attrs(p);
    match p.kind {
        Kind::Heading(n) => format!("<h{n}{attrs}>{body}</h{n}>", n = n.clamp(1, 6)),
        Kind::Title => format!("<h1{attrs}>{body}</h1>"),
        _ if body.is_empty() => format!("<p{attrs}>&nbsp;</p>"),
        _ => format!("<p{attrs}>{body}</p>"),
    }
}

/// Append the rendered HTML body for contiguous non-spaced blocks (tables, lists, code, quotes)
/// from `craft-html` so list nesting and table layout stay delegated upstream.
fn push_craft_blocks(blocks: &[FBlock], out: &mut String) {
    let html = export_flow(&Flow { blocks: blocks.to_vec(), ..Default::default() }, "en");
    if let Some((_, body)) = html.split_once("<body>\n")
        && let Some(content) = body.strip_suffix("</body>\n</html>\n")
    {
        out.push_str(content);
    }
}

/// `doc`'s content as an HTML fragment (no head or style sheet): what a copy puts on the system
/// clipboard beside its text, equations as MathML (#816). With no style sheet to give them, its
/// top-level plain paragraphs and headings carry their own spacing and indents (#5537).
pub fn export_fragment(doc: &Document) -> String {
    let Ok(shared) = wordcraft_docx::to_shared_model(doc) else {
        return String::new();
    };
    let flow = craft_flow::from_doc(&shared);
    let mut out = String::from("<meta charset=\"utf-8\">");
    let is_spaced = |b: &FBlock| matches!(b, FBlock::Para(p) if p.list.is_none() && !matches!(p.kind, Kind::Code | Kind::Quote));
    let mut i = 0;
    while let Some(b) = flow.blocks.get(i) {
        if let FBlock::Para(p) = b
            && is_spaced(b)
        {
            out.push_str(&spaced_para_html(p));
            out.push('\n');
            i += 1;
        } else {
            let start = i;
            while flow.blocks.get(i).is_some_and(|blk| !is_spaced(blk)) {
                i += 1;
            }
            if let Some(slice) = flow.blocks.get(start..i) {
                push_craft_blocks(slice, &mut out);
            }
        }
    }
    out
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
