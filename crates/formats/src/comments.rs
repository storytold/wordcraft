//! Print Markup for the text formats: HTML, Markdown and plain text copies end with the
//! document's comments, numbered in document order, each with its author, the text it comments
//! on, what it says and its replies (like the notes at the end of a printed document).

use std::collections::{BTreeMap, HashMap, HashSet};

use wordcraft_doc::para::{InlineObject, OBJ};
use wordcraft_doc::{Document, StoryRef};

/// Longest quoted anchor text (characters).
const MAX_QUOTE: usize = 200;
const MAX_COMMENTS: usize = 5_000;
const MAX_REPLIES: usize = 200;
const MAX_PARENT_CHAIN: usize = 64;

/// One comment as written out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Note {
    pub author: String,
    /// The text it comments on (shortened), or empty.
    pub quote: String,
    pub text: String,
    /// (author, text) of each reply, oldest first.
    pub replies: Vec<(String, String)>,
}

fn root(doc: &Document, id: u32) -> u32 {
    let mut cur = id;
    let mut seen = HashSet::new();
    seen.insert(id);
    for _ in 0..MAX_PARENT_CHAIN {
        match doc.comments.get(&cur).and_then(|c| c.parent) {
            Some(p) if p != cur && seen.insert(p) && doc.comments.contains_key(&p) => cur = p,
            _ => break,
        }
    }
    cur
}


fn author(a: &str) -> String {
    if a.trim().is_empty() { "Author".to_string() } else { a.trim().to_string() }
}

fn text_of(doc: &Document, part: u32) -> String {
    doc.plain_text(StoryRef::Part(part)).split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The document's top-level comments: those anchored in the body in document order, then the
/// rest; replies go under the comment they answer.
pub fn notes(doc: &Document) -> Vec<Note> {
    if doc.comments.is_empty() {
        return Vec::new();
    }
    // Anchor order and the text between each comment's start and end.
    let mut order: Vec<u32> = Vec::new();
    let mut seen = HashSet::new();
    let mut quotes: HashMap<u32, String> = HashMap::new();
    let mut open: Vec<u32> = Vec::new();
    for path in doc.para_paths(StoryRef::Body) {
        let Some(p) = doc.para(StoryRef::Body, &path) else { continue };
        for (off, c) in p.text.char_indices() {
            if c == OBJ {
                match p.object_at(off) {
                    Some(InlineObject::CommentStart { id }) => {
                        if seen.insert(*id) {
                            order.push(*id);
                        }
                        if !open.contains(id) && open.len() < 64 {
                            open.push(*id);
                        }
                    }
                    Some(InlineObject::CommentEnd { id }) => open.retain(|x| x != id),
                    _ => {}
                }
                continue;
            }
            for id in &open {
                let q = quotes.entry(*id).or_default();
                if q.chars().count() <= MAX_QUOTE {
                    q.push(c);
                }
            }
        }
        for id in &open {
            quotes.entry(*id).or_default().push(' ');
        }
    }
    for id in doc.comments.keys() {
        if seen.insert(*id) {
            order.push(*id);
        }
    }
    let mut replies: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
    for id in doc.comments.keys() {
        let r = root(doc, *id);
        if r != *id {
            let v = replies.entry(r).or_default();
            if v.len() < MAX_REPLIES {
                v.push(*id);
            }
        }
    }
    let mut out = Vec::new();
    for id in order {
        let Some(c) = doc.comments.get(&id) else { continue };
        if root(doc, id) != id {
            continue;
        }
        let quote = quotes.get(&id).map(|q| q.split_whitespace().collect::<Vec<_>>().join(" ")).unwrap_or_default();
        let quote = if quote.chars().count() > MAX_QUOTE { format!("{}…", quote.chars().take(MAX_QUOTE).collect::<String>()) } else { quote };
        let replies =
            replies.get(&id).into_iter().flatten().filter_map(|r| doc.comments.get(r)).map(|r| (author(&r.author), text_of(doc, r.part))).collect();
        out.push(Note { author: author(&c.author), quote, text: text_of(doc, c.part), replies });
        if out.len() >= MAX_COMMENTS {
            break;
        }
    }
    out
}

fn rfind_case_insensitive(haystack: &str, needle: &str) -> Option<usize> {
    let n_len = needle.len();
    if haystack.len() < n_len {
        return None;
    }
    for i in (0..=haystack.len() - n_len).rev() {
        if haystack.is_char_boundary(i) && haystack.is_char_boundary(i + n_len) {
            if haystack[i..i + n_len].eq_ignore_ascii_case(needle) {
                return Some(i);
            }
        }
    }
    None
}

/// `bytes` (a copy in format `ext`) with the comments written at its end.
pub fn append(ext: &str, doc: &Document, bytes: Vec<u8>) -> Vec<u8> {
    let notes = notes(doc);
    if notes.is_empty() {
        return bytes;
    }
    let clean = ext.trim_start_matches('.').to_ascii_lowercase();
    match clean.as_str() {
        "html" | "htm" | "xhtml" => {
            let mut s = match String::from_utf8(bytes) {
                Ok(s) => s,
                Err(e) => return e.into_bytes(),
            };
            let at = rfind_case_insensitive(&s, "</body>").unwrap_or(s.len());
            s.insert_str(at, &html(&notes));
            s.into_bytes()
        }
        "md" | "markdown" => {
            let mut s = bytes;
            s.extend_from_slice(markdown(&notes).as_bytes());
            s
        }
        "txt" | "text" => {
            let mut s = bytes;
            s.extend_from_slice(text(&notes).as_bytes());
            s
        }
        _ => bytes,
    }
}

fn html(notes: &[Note]) -> String {
    use crate::html::esc;
    let mut s = String::from("<section class=\"comments\">\n<h2>Comments</h2>\n<ol>\n");
    for (i, n) in notes.iter().enumerate() {
        s.push_str(&format!("<li id=\"comment-{}\"><p><strong>{}</strong>", i + 1, esc(&n.author)));
        if !n.quote.is_empty() {
            s.push_str(&format!(" on <q>{}</q>", esc(&n.quote)));
        }
        s.push_str(&format!(": {}</p>", esc(&n.text)));
        if !n.replies.is_empty() {
            s.push_str("<ul>");
            for (a, t) in &n.replies {
                s.push_str(&format!("<li><strong>{}</strong>: {}</li>", esc(a), esc(t)));
            }
            s.push_str("</ul>");
        }
        s.push_str("</li>\n");
    }
    s.push_str("</ol>\n</section>\n");
    s
}

fn markdown(notes: &[Note]) -> String {
    use crate::markdown::escape;
    let mut s = String::from("\n\n## Comments\n\n");
    for (i, n) in notes.iter().enumerate() {
        s.push_str(&format!("{}. **{}**", i + 1, escape(&n.author)));
        if !n.quote.is_empty() {
            s.push_str(&format!(" on “{}”", escape(&n.quote)));
        }
        s.push_str(&format!(": {}\n", escape(&n.text)));
        for (a, t) in &n.replies {
            s.push_str(&format!("    - **{}**: {}\n", escape(a), escape(t)));
        }
    }
    s
}

fn text(notes: &[Note]) -> String {
    let mut s = String::from("\r\n\r\nComments\r\n");
    for (i, n) in notes.iter().enumerate() {
        s.push_str(&format!("\r\n[{}] {}", i + 1, n.author));
        if !n.quote.is_empty() {
            s.push_str(&format!(" on “{}”", n.quote));
        }
        s.push_str(&format!(": {}\r\n", n.text));
        for (a, t) in &n.replies {
            s.push_str(&format!("    {a}: {t}\r\n"));
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use wordcraft_doc::{Block, Comment, Document, Paragraph, Part};

    #[test]
    fn append_handles_case_insensitive_extensions_and_body_tag() {
        let mut doc = Document::default();
        let part_id = 100;
        let mut p = Paragraph::default();
        p.text = "This is a comment".into();
        doc.parts.insert(
            part_id,
            Part {
                blocks: vec![Arc::new(Block::Para(p))],
                ..Default::default()
            },
        );
        doc.comments.insert(
            1,
            Comment {
                author: "Alice".into(),
                part: part_id,
                parent: None,
                ..Default::default()
            },
        );

        let html_input = b"<!DOCTYPE html><html><head><title>Test</title></head><BODY><p>Content</p></BODY></html>".to_vec();
        let out_html = append(".HTML", &doc, html_input);
        let s = String::from_utf8(out_html).unwrap();
        assert!(s.contains("<section class=\"comments\">"));
        assert!(s.ends_with("</BODY></html>"));

        let md_input = b"# Title\n\nBody".to_vec();
        let out_md = append(".MD", &doc, md_input);
        let s_md = String::from_utf8(out_md).unwrap();
        assert!(s_md.contains("## Comments"));

        let txt_input = b"Plain text".to_vec();
        let out_txt = append(".TXT", &doc, txt_input);
        let s_txt = String::from_utf8(out_txt).unwrap();
        assert!(s_txt.contains("Comments"));
    }

    #[test]
    fn comment_cycle_does_not_loop_or_falsely_become_root() {
        let mut doc = Document::default();
        let p1 = 101;
        let p2 = 102;
        doc.parts.insert(
            p1,
            Part {
                blocks: vec![Arc::new(Block::Para(Paragraph::default()))],
                ..Default::default()
            },
        );
        doc.parts.insert(
            p2,
            Part {
                blocks: vec![Arc::new(Block::Para(Paragraph::default()))],
                ..Default::default()
            },
        );

        // 1 -> 2 -> 1 cycle
        doc.comments.insert(
            1,
            Comment {
                author: "A".into(),
                part: p1,
                parent: Some(2),
                ..Default::default()
            },
        );
        doc.comments.insert(
            2,
            Comment {
                author: "B".into(),
                part: p2,
                parent: Some(1),
                ..Default::default()
            },
        );

        let r1 = root(&doc, 1);
        let r2 = root(&doc, 2);
        assert_ne!(r1, 1);
        assert_ne!(r2, 2);

        let n = notes(&doc);
        assert!(n.is_empty());
    }
}

