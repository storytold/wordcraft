//! Comment ranges that run on from an earlier paragraph, so the shade over commented text
//! continues across paragraphs and pages.

use std::collections::{HashMap, HashSet};

use wordcraft_doc::para::InlineObject;
use wordcraft_doc::{Document, Paragraph, Path, StoryRef};

use crate::{Page, Placed};

/// How a paragraph starts inside comment ranges that began in an earlier paragraph of its story
/// (see [`comment_carry`]).
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct CommentCarry {
    /// A range began before the paragraph and ends after it: all of the paragraph is commented.
    pub through: bool,
    /// The comments whose range began before the paragraph and ends in it.
    pub ending: Vec<u32>,
}

/// The paragraphs that start inside a comment range begun in an earlier paragraph, by story and
/// path, so the shade over commented text runs across paragraphs and pages (it stopped at the
/// end of the paragraph the comment starts in). Only ranges of comments the document has count,
/// and only when the end follows the start in the same story: a start whose end was lost shades
/// the rest of its own paragraph, as before, never the rest of the document. One pass over the
/// paragraphs' objects, once per layout and only when the document has comments.
pub fn comment_carry(doc: &Document) -> HashMap<(StoryRef, Path), CommentCarry> {
    let mut out = HashMap::new();
    if doc.comments.is_empty() {
        return out;
    }
    let stories = std::iter::once(StoryRef::Body).chain(doc.parts.keys().map(|k| StoryRef::Part(*k)));
    for story in stories {
        let paths = doc.para_paths(story);
        // Ranges that end in a later paragraph than they start in: (start, end paragraph, id).
        let mut open: HashMap<u32, usize> = HashMap::new();
        let mut spans: Vec<(usize, usize, u32)> = Vec::new();
        for (k, path) in paths.iter().enumerate() {
            let Some(p) = doc.para(story, path) else { continue };
            for o in &p.objects {
                match o {
                    InlineObject::CommentStart { id } if doc.comments.contains_key(id) => {
                        open.entry(*id).or_insert(k);
                    }
                    InlineObject::CommentEnd { id } => {
                        if let Some(start) = open.remove(id)
                            && start < k
                        {
                            spans.push((start, k, *id));
                        }
                    }
                    _ => {}
                }
            }
        }
        if spans.is_empty() {
            continue;
        }
        // The paragraphs strictly inside a span: a running count of the spans open over each.
        let mut delta = vec![0i64; paths.len() + 1];
        let mut ending: HashMap<usize, Vec<u32>> = HashMap::new();
        for (a, b, id) in spans {
            if let Some(d) = delta.get_mut(a + 1) {
                *d += 1;
            }
            if let Some(d) = delta.get_mut(b) {
                *d -= 1;
            }
            ending.entry(b).or_default().push(id);
        }
        let mut inside = 0i64;
        for (k, path) in paths.into_iter().enumerate() {
            inside += delta.get(k).copied().unwrap_or(0);
            let ending = ending.remove(&k).unwrap_or_default();
            if inside > 0 || !ending.is_empty() {
                out.insert((story, path), CommentCarry { through: inside > 0, ending });
            }
        }
    }
    out
}

/// The entries of `carry` for the paragraphs `page` shows (body, header, footer, and the text in
/// a drawing canvas), sorted by story and path, for [`crate::Page::comment_carry`].
pub(crate) fn page_carry(carry: &HashMap<(StoryRef, Path), CommentCarry>, page: &Page) -> Vec<(StoryRef, Path, CommentCarry)> {
    fn collect(items: &[Placed], carry: &HashMap<(StoryRef, Path), CommentCarry>, depth: usize, out: &mut Vec<(StoryRef, Path, CommentCarry)>) {
        for it in items {
            match it {
                Placed::Lines { story, path, .. } => {
                    if let Some(c) = carry.get(&(*story, path.clone())) {
                        out.push((*story, path.clone(), c.clone()));
                    }
                }
                Placed::Clip { items, .. } if depth < 16 => collect(items, carry, depth + 1, out),
                _ => {}
            }
        }
    }
    let mut out = Vec::new();
    for items in [&page.items, &page.header, &page.footer] {
        collect(items, carry, 0, &mut out);
    }
    out.sort_by(|x, y| (x.0, &x.1).cmp(&(y.0, &y.1)));
    out.dedup_by(|x, y| x.0 == y.0 && x.1 == y.1);
    out
}

/// The carry of paragraph `path` of `story` in a page's sorted [`crate::Page::comment_carry`].
pub(super) fn carry_of<'a>(carry: &'a [(StoryRef, Path, CommentCarry)], story: StoryRef, path: &Path) -> Option<&'a CommentCarry> {
    let i = carry.binary_search_by(|(s, q, _)| (*s, q).cmp(&(story, path))).ok()?;
    carry.get(i).map(|(.., c)| c)
}

/// The byte ranges of paragraph `p` inside a comment range, given how it starts (`carry`). An end
/// with no start before it (a comment with no range, or one whose start was lost) shades nothing.
pub(super) fn commented_ranges(doc: &Document, p: &Paragraph, carry: Option<&CommentCarry>) -> Vec<(usize, usize)> {
    if doc.comments.is_empty() {
        return Vec::new();
    }
    if carry.is_some_and(|c| c.through) {
        return vec![(0, p.len())];
    }
    let mut open: HashSet<u32> = carry.map(|c| c.ending.iter().copied().collect()).unwrap_or_default();
    let mut from = 0;
    let mut ranges = Vec::new();
    for off in p.object_offsets() {
        match p.object_at(off) {
            Some(InlineObject::CommentStart { id }) if doc.comments.contains_key(id) => {
                if open.is_empty() {
                    from = off;
                }
                open.insert(*id);
            }
            Some(InlineObject::CommentEnd { id }) if open.remove(id) && open.is_empty() => {
                ranges.push((from, off));
            }
            _ => {}
        }
    }
    if !open.is_empty() {
        ranges.push((from, p.len()));
    }
    ranges
}
