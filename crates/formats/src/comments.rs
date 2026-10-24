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
    for _ in 0..MAX_PARENT_CHAIN {
        match doc.comments.get(&cur).and_then(|c| c.parent) {
            Some(p) if p != cur && doc.comments.contains_key(&p) => cur = p,
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
    let mut ids: Vec<u32> = doc.comments.keys().copied().collect();
    ids.sort_by(|&a, &b| {
        let ca = doc.comments.get(&a);
        let cb = doc.comments.get(&b);
        let da = ca.map(|c| c.date.as_str()).unwrap_or("");
        let db = cb.map(|c| c.date.as_str()).unwrap_or("");
        match (!da.is_empty() && !db.is_empty(), da.cmp(db)) {
            (true, std::cmp::Ordering::Equal) | (false, _) => a.cmp(&b),
            (true, ord) => ord,
        }
    });
    for id in &ids {
        if seen.insert(*id) {
            order.push(*id);
        }
    }
    let mut replies: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
    for id in &ids {
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

/// `bytes` (a copy in format `ext`) with the comments written at its end.
pub fn append(ext: &str, doc: &Document, bytes: Vec<u8>) -> Vec<u8> {
    let notes = notes(doc);
    if notes.is_empty() {
        return bytes;
    }
    match ext {
        "html" | "htm" | "xhtml" => {
            let mut s = match String::from_utf8(bytes) {
                Ok(s) => s,
                Err(e) => return e.into_bytes(),
            };
            let at = s.rfind("</body>").unwrap_or(s.len());
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
    use wordcraft_doc::{Comment, Document, Paragraph, para_block};

    #[test]
    fn replies_are_ordered_oldest_first() {
        let mut doc = Document::new();
        doc.parts.insert(
            1,
            wordcraft_doc::Part { blocks: vec![para_block(Paragraph::with_text("Root comment", Default::default()))], ..Default::default() },
        );
        doc.comments.insert(1, Comment { author: "Alice".into(), date: "2026-10-15T10:00:00Z".into(), parent: None, part: 1, ..Default::default() });

        doc.parts.insert(
            3,
            wordcraft_doc::Part { blocks: vec![para_block(Paragraph::with_text("Second reply", Default::default()))], ..Default::default() },
        );
        doc.comments
            .insert(3, Comment { author: "Charlie".into(), date: "2026-10-15T10:30:00Z".into(), parent: Some(1), part: 3, ..Default::default() });

        doc.parts.insert(
            2,
            wordcraft_doc::Part { blocks: vec![para_block(Paragraph::with_text("First reply", Default::default()))], ..Default::default() },
        );
        doc.comments.insert(2, Comment { author: "Bob".into(), date: "2026-10-15T10:15:00Z".into(), parent: Some(1), part: 2, ..Default::default() });

        let n = notes(&doc);
        assert_eq!(n.len(), 1);
        assert_eq!(n[0].author, "Alice");
        assert_eq!(n[0].replies, vec![("Bob".to_string(), "First reply".to_string()), ("Charlie".to_string(), "Second reply".to_string()),]);
    }
}
