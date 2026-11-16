//! Links in the plain-text comment editors and margin cards. Link styling and hit testing
//! never write to the document, so imported hyperlink labels keep their original targets.

use std::ops::Range;

use egui::{Color32, FontId, Galley, Pos2, Rect, Response, Stroke, Ui, text::LayoutJob};
use wordcraft_doc::link::{hyperlink_target, is_web_or_mail_link};
use wordcraft_doc::{Block, Blocks, Document, Paragraph, StoryRef, para::OBJ};

#[derive(Clone, Debug)]
struct Link {
    range: Range<usize>,
    target: String,
}

#[derive(Default)]
pub(crate) struct CommentText {
    pub text: String,
    links: Vec<Link>,
}

impl CommentText {
    pub fn new(doc: &Document, part: u32) -> Self {
        let mut out = Self::default();
        if let Some(blocks) = doc.story(StoryRef::Part(part)) {
            out.blocks(blocks, 0);
        }
        if out.text.ends_with('\n') {
            out.text.pop();
        }
        out.detect_addresses();
        out
    }

    /// Drafts are plain text. Once edited, old label offsets must never point at unrelated
    /// characters; detect addresses in the current draft instead.
    pub fn plain(text: String) -> Self {
        let mut out = Self { text, links: Vec::new() };
        out.detect_addresses();
        out
    }

    fn add(&mut self, range: Range<usize>, target: &str) {
        if range.is_empty() {
            return;
        }
        if let Some(last) = self.links.last_mut()
            && last.range.end == range.start
            && last.target == target
        {
            last.range.end = range.end;
        } else {
            self.links.push(Link { range, target: target.to_string() });
        }
    }

    // Match Document::plain_text's paragraph/table separators, including inline field results.
    // The same depth bound prevents a hostile nested table from overflowing the stack.
    fn blocks(&mut self, blocks: &Blocks, depth: usize) {
        if depth > 16 {
            return;
        }
        for block in blocks {
            match &**block {
                Block::Para(p) => {
                    self.paragraph(p);
                    self.text.push('\n');
                }
                Block::Table(table) => {
                    for row in &table.rows {
                        for (i, cell) in row.cells.iter().enumerate() {
                            if i > 0 {
                                self.text.push('\t');
                            }
                            let mut child = Self::default();
                            child.blocks(&cell.blocks, depth + 1);
                            let trimmed = child.text.trim_end_matches('\n').len();
                            let start = self.text.len();
                            self.text.push_str(&child.text.get(..trimmed).unwrap_or_default().replace('\n', " "));
                            for link in child.links {
                                self.add(start + link.range.start.min(trimmed)..start + link.range.end.min(trimmed), &link.target);
                            }
                        }
                        self.text.push('\n');
                    }
                }
            }
        }
    }

    fn paragraph(&mut self, p: &Paragraph) {
        let mut runs = p.run_ranges().peekable();
        let mut objects = p.objects.iter();
        let mut previous: Option<&str> = None;
        for (off, c) in p.text.char_indices() {
            while runs.peek().is_some_and(|(r, _)| r.end <= off) {
                runs.next();
            }
            let props = runs.peek().filter(|(r, _)| r.contains(&off)).map(|(_, p)| *p);
            let start = self.text.len();
            let mut field_target = None;
            if c == OBJ {
                if let Some(object) = objects.next() {
                    self.text.push_str(object.plain_text());
                    if let wordcraft_doc::InlineObject::Field { instr, .. } = object {
                        field_target = hyperlink_target(instr);
                    }
                }
            } else {
                self.text.push(c);
            }
            // Empty markers contribute no link text. Changing the cached target here would
            // make the next character extend a preceding link with a different destination.
            if self.text.len() == start {
                continue;
            }
            if let Some(target) = props.and_then(|p| p.link.as_deref()) {
                // Keep even refused targets here: an unsafe hyperlink with a web-looking label
                // must not silently become a different, auto-detected link.
                if previous.is_some_and(|last| std::ptr::eq(last, target))
                    && let Some(last) = self.links.last_mut().filter(|l| l.range.end == start)
                {
                    // The same borrowed run target: don't compare a potentially huge URL
                    // again for every character of its label.
                    last.range.end = self.text.len();
                } else {
                    self.add(start..self.text.len(), target);
                }
                previous = Some(target);
            } else {
                previous = None;
                if let Some(target) = field_target {
                    self.add(start..self.text.len(), &target);
                }
            }
        }
    }

    fn detect_addresses(&mut self) {
        let mut offset = 0;
        let mut detected = Vec::new();
        for token in self.text.split_inclusive(char::is_whitespace) {
            let address = token.trim().trim_start_matches(['(', '[', '{', '<', '"', '\'', '«', '“']);
            let mut address = address.trim_end_matches(['.', ',', '!', '?', ';', ':', '،', '؛', '>', '"', '\'', '»', '”']);
            // A URL may contain balanced parentheses; surrounding prose punctuation isn't part
            // of its target. Count once, so a long run of closing brackets stays linear.
            for (open, close) in [('(', ')'), ('[', ']'), ('{', '}')] {
                let mut extra = address.chars().filter(|c| *c == close).count().saturating_sub(address.chars().filter(|c| *c == open).count());
                while extra > 0 && address.ends_with(close) {
                    address = address.strip_suffix(close).unwrap_or(address);
                    extra -= 1;
                }
            }
            // `address` is a subslice, but no pointer arithmetic is needed for its byte offset.
            let start = offset + token.find(address).unwrap_or(0);
            let range = start..start + address.len();
            if is_web_or_mail_link(address)
                && !address.chars().any(char::is_control)
                && !self.links.get(self.links.partition_point(|l| l.range.end <= range.start)).is_some_and(|l| l.range.start < range.end)
            {
                detected.push(Link { range, target: address.to_string() });
            }
            offset += token.len();
        }
        self.links.extend(detected);
        self.links.sort_by_key(|l| l.range.start);
    }

    pub fn trimmed(mut self) -> Self {
        let start = self.text.len() - self.text.trim_start().len();
        let end = self.text.trim_end().len().max(start);
        for link in &mut self.links {
            link.range = link.range.start.clamp(start, end) - start..link.range.end.clamp(start, end) - start;
        }
        self.links.retain(|l| !l.range.is_empty());
        self.text = self.text.get(start..end).unwrap_or_default().to_string();
        self
    }

    pub fn job(&self, font: FontId, color: Color32, link_color: Color32, width: f32) -> LayoutJob {
        let mut job = LayoutJob::default();
        job.wrap.max_width = width;
        let mut offset = 0;
        for link in self.links.iter().filter(|l| is_web_or_mail_link(&l.target)) {
            let Some(before) = self.text.get(offset..link.range.start) else { continue };
            let Some(label) = self.text.get(link.range.clone()) else { continue };
            job.append(before, 0.0, egui::TextFormat { font_id: font.clone(), color, ..Default::default() });
            job.append(
                label,
                0.0,
                egui::TextFormat { font_id: font.clone(), color: link_color, underline: Stroke::new(1.0, link_color), ..Default::default() },
            );
            offset = link.range.end;
        }
        job.append(self.text.get(offset..).unwrap_or_default(), 0.0, egui::TextFormat { font_id: font, color, ..Default::default() });
        job
    }

    /// Only an actual visible glyph is a hit: nearest-cursor snapping would open links from
    /// empty space beside a short line, and ignoring the clip would activate hidden rows.
    pub fn at(&self, galley: &Galley, origin: Pos2, clip: Rect, pointer: Pos2) -> Option<&str> {
        if !clip.contains(pointer) {
            return None;
        }
        let mut chars = self.text.char_indices();
        for row in &galley.rows {
            for glyph in &row.glyphs {
                let (byte, _) = chars.next()?;
                if glyph.logical_rect().translate(origin.to_vec2() + row.pos.to_vec2()).contains(pointer) {
                    return self.links.iter().find(|l| l.range.contains(&byte) && is_web_or_mail_link(&l.target)).map(|l| l.target.as_str());
                }
            }
            if row.ends_with_newline {
                chars.next();
            }
        }
        None
    }
}

/// The same discoverable modifier-click as hyperlinks in the document body. Plain clicks
/// remain available for moving the text caret and selecting/editing the comment.
pub(crate) fn interact(ui: &Ui, response: &Response, target: Option<&str>) -> Option<String> {
    let target = target.filter(|_| response.hovered() && !response.secondary_clicked() && !ui.input(|i| i.pointer.secondary_down()))?;
    let key = if cfg!(target_os = "macos") { "⌘" } else { "Ctrl" };
    response.clone().on_hover_text_at_pointer(format!("{target}\n{}", crate::i18n::fmt(tl!("{key}+Click to follow link"), &[("key", key)])));
    if ui.input(|i| i.modifiers.command) {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        if response.clicked() {
            return Some(target.to_string());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use wordcraft_doc::{InlineObject, para_block, props::CharProps};

    #[test]
    fn addresses_exclude_prose_punctuation_but_keep_balanced_url_parentheses() {
        let text = "ببین (https://example.com/a(b)).\n<HTTPS://EXAMPLE.COM/x?q=1#top> mailto:a@example.com، file:///tmp/x javascript:alert(1)";
        let content = CommentText::plain(text.into());
        let targets: Vec<_> = content.links.iter().map(|l| l.target.as_str()).collect();
        assert_eq!(targets, ["https://example.com/a(b)", "HTTPS://EXAMPLE.COM/x?q=1#top", "mailto:a@example.com"]);
        for link in &content.links {
            assert_eq!(content.text.get(link.range.clone()), Some(link.target.as_str()));
        }
    }

    #[test]
    fn imported_field_results_and_run_labels_use_their_declared_targets() {
        let mut p = Paragraph::with_text("نام ", CharProps::default());
        p.append(Paragraph::with_text("https://label.example", CharProps { link: Some("file:///tmp/blocked".into()), ..Default::default() }));
        p.insert_object(
            p.len(),
            InlineObject::Field {
                instr: "HYPERLINK \"https://example.com/field\"".into(),
                result: "Field label".into(),
                locked: false,
                dirty: false,
                code: false,
                form: String::new(),
            },
            &CharProps::default(),
        )
        .unwrap();
        let mut content = CommentText::default();
        content.paragraph(&p);
        content.detect_addresses();
        assert_eq!(content.text, p.plain_text());
        assert_eq!(content.links.len(), 2, "a web-looking label cannot bypass a refused target");
        let job = content.job(FontId::proportional(12.0), Color32::BLACK, Color32::BLUE, 250.0);
        let linked: Vec<_> =
            job.sections.iter().filter(|s| s.format.underline.width > 0.0).map(|s| &job.text[s.byte_range.start.0..s.byte_range.end.0]).collect();
        assert_eq!(linked, ["Field label"]);
    }

    #[test]
    fn empty_bookmarks_do_not_join_adjacent_labels_with_different_targets() {
        for marker in [InlineObject::BookmarkStart { name: "second".into() }, InlineObject::BookmarkEnd { name: "second".into() }] {
            for target in ["https://second.example", "mailto:second@example.com", "file:///tmp/blocked"] {
                let props = CharProps { link: Some(target.into()), ..Default::default() };
                let mut second = Paragraph::with_text("بتا", props.clone());
                second.insert_object(0, marker.clone(), &props).unwrap();
                let mut p = Paragraph::with_text("Alpha", CharProps { link: Some("https://first.example".into()), ..Default::default() });
                p.append(second);

                let mut content = CommentText::default();
                content.paragraph(&p);
                content.detect_addresses();
                assert_eq!(content.text, "Alphaبتا");
                let links: Vec<_> = content.links.iter().map(|l| (content.text.get(l.range.clone()).unwrap(), l.target.as_str())).collect();
                assert_eq!(links, [("Alpha", "https://first.example"), ("بتا", target)], "{marker:?}");
                let job = content.job(FontId::proportional(12.0), Color32::BLACK, Color32::BLUE, 250.0);
                let linked = job.sections.iter().filter(|s| s.format.underline.width > 0.0).count();
                assert_eq!(linked, if is_web_or_mail_link(target) { 2 } else { 1 });
            }
        }
    }

    #[test]
    fn linked_glyph_hit_testing_respects_wrapping_bidi_and_clipping() {
        let content = CommentText::plain("سلام https://example.com/long\nplain".into());
        let ctx = egui::Context::default();
        ctx.run_ui(egui::RawInput::default(), |ui| {
            let galley = ui.fonts_mut(|f| f.layout_job(content.job(FontId::proportional(14.0), Color32::BLACK, Color32::BLUE, 100.0)));
            let origin = Pos2::new(30.0, 40.0);
            let mut chars = content.text.char_indices();
            for row in &galley.rows {
                for glyph in &row.glyphs {
                    let (byte, _) = chars.next().unwrap();
                    let at = origin + row.pos.to_vec2() + glyph.logical_rect().center().to_vec2();
                    let want = content.links.iter().find(|l| l.range.contains(&byte)).map(|l| l.target.as_str());
                    assert_eq!(content.at(&galley, origin, Rect::EVERYTHING, at), want);
                    assert_eq!(content.at(&galley, origin, Rect::NOTHING, at), None);
                }
                if row.ends_with_newline {
                    chars.next();
                }
                let outside = origin + egui::vec2(galley.size().x + 30.0, row.rect().center().y);
                assert_eq!(content.at(&galley, origin, Rect::EVERYTHING, outside), None);
            }
        })
        .drop_without_applying_deltas();
    }

    #[test]
    fn table_comment_text_and_link_offsets_match_the_document_plain_text() {
        let mut table = wordcraft_doc::Table::new(1, 2, 200.0);
        let linked = Paragraph::with_text("Label", CharProps { link: Some("https://example.com".into()), ..Default::default() });
        table.rows[0].cells[0].blocks = vec![para_block(Paragraph::with_text("First", Default::default())), para_block(linked)];
        table.rows[0].cells[1].blocks = vec![para_block(Paragraph::with_text("Last", Default::default()))];
        let mut doc = Document::new();
        doc.body = vec![std::sync::Arc::new(Block::Table(table))];
        let mut content = CommentText::default();
        content.blocks(&doc.body, 0);
        content.text.pop();
        assert_eq!(content.text, doc.plain_text(StoryRef::Body));
        let link = &content.links[0];
        assert_eq!(content.text.get(link.range.clone()), Some("Label"));
    }
}
