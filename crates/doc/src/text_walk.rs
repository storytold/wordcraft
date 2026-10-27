use super::block::{Block, Blocks};
use super::parts::PartKind;
use super::position::Path;
use super::*;

/// Words in `s` (tokens separated by whitespace that contain at least one alphanumeric char).
pub fn count_words(s: &str) -> usize {
    s.split(|c: char| c.is_whitespace() || c == para::OBJ).filter(|w| w.chars().any(char::is_alphanumeric)).count()
}

/// How deep in nested tables [`Document::para_paths`] goes: cells below this are skipped.
pub(crate) const PARA_WALK_DEPTH: usize = 16;

/// The note and text box stories an object shows, for `Document::reachable`.
pub(crate) fn notes_and_boxes(o: &InlineObject) -> Vec<(u32, &'static [PartKind])> {
    match o {
        InlineObject::NoteRef { id, .. } => vec![(*id, &[PartKind::Footnote, PartKind::Endnote][..])],
        o => o.text_boxes().into_iter().map(|id| (id, &[PartKind::TextBox][..])).collect(),
    }
}

pub(crate) fn walk(bl: &Blocks, prefix: &mut Vec<u32>, out: &mut Vec<Path>, depth: usize) {
    if depth > PARA_WALK_DEPTH {
        return;
    }
    for (i, b) in bl.iter().enumerate() {
        prefix.push(i as u32);
        match &**b {
            Block::Para(_) => out.push(Path(prefix.clone())),
            Block::Table(t) => {
                for (r, row) in t.rows.iter().enumerate() {
                    for (c, cell) in row.cells.iter().enumerate() {
                        prefix.push(r as u32);
                        prefix.push(c as u32);
                        walk(&cell.blocks, prefix, out, depth + 1);
                        prefix.pop();
                        prefix.pop();
                    }
                }
            }
        }
        prefix.pop();
    }
}

/// [`walk`] for the paragraphs from path `a` to `b` inclusive, skipping the blocks outside.
pub(crate) fn walk_between(bl: &Blocks, prefix: &mut Vec<u32>, out: &mut Vec<Path>, depth: usize, (a, b): (&[u32], &[u32])) {
    use std::cmp::Ordering::{Greater, Less};
    if depth > PARA_WALK_DEPTH {
        return;
    }
    // Where the paths starting with `q` lie against `x`: all before, all after, or either.
    let side = |q: &[u32], x: &[u32]| {
        let m = q.len().min(x.len());
        q[..m].cmp(&x[..m])
    };
    for (i, blk) in bl.iter().enumerate() {
        prefix.push(i as u32);
        if side(prefix, b) == Greater {
            prefix.pop();
            break;
        }
        if side(prefix, a) != Less {
            match &**blk {
                Block::Para(_) => {
                    if prefix.as_slice() >= a && prefix.as_slice() <= b {
                        out.push(Path(prefix.clone()));
                    }
                }
                Block::Table(t) => {
                    for (r, row) in t.rows.iter().enumerate() {
                        for (c, cell) in row.cells.iter().enumerate() {
                            prefix.push(r as u32);
                            prefix.push(c as u32);
                            if side(prefix, a) != Less && side(prefix, b) != Greater {
                                walk_between(&cell.blocks, prefix, out, depth + 1, (a, b));
                            }
                            prefix.pop();
                            prefix.pop();
                        }
                    }
                }
            }
        }
        prefix.pop();
    }
}

/// Which text of a paragraph [`text_of`] collects.
#[derive(Clone, Copy)]
pub(crate) enum TextMode {
    /// Every run, tracked deletions included ([`Paragraph::plain_text`]).
    Plain,
    /// As shown on screen ([`Paragraph::shown_text`]).
    Shown,
    /// Without tracked deletions ([`Paragraph::final_text`]).
    Final,
}

pub(crate) fn text_of(bl: &Blocks, out: &mut String, depth: usize, mode: TextMode) {
    if depth > 16 {
        return;
    }
    for b in bl {
        match &**b {
            Block::Para(p) => {
                out.push_str(&match mode {
                    TextMode::Plain => p.plain_text(),
                    TextMode::Shown => p.shown_text(),
                    TextMode::Final => p.final_text(),
                });
                out.push('\n');
            }
            Block::Table(t) => {
                for row in &t.rows {
                    let mut cells = Vec::new();
                    for c in &row.cells {
                        let mut s = String::new();
                        text_of(&c.blocks, &mut s, depth + 1, mode);
                        cells.push(s.trim_end_matches('\n').replace('\n', " "));
                    }
                    out.push_str(&cells.join("\t"));
                    out.push('\n');
                }
            }
        }
    }
}

pub(crate) fn story_word_count_of(
    bl: &Blocks,
    numbering: &Numbering,
    styles: &styles::StyleSheet,
    counters: &mut numbering::Counters,
    depth: usize,
) -> usize {
    if depth > 16 {
        return 0;
    }
    let mut total = 0usize;
    for b in bl {
        match &**b {
            Block::Para(p) => {
                let n = p.props.numbering.or_else(|| styles.resolve_para(&p.props).numbering).filter(|n| n.num != 0);
                if let Some(n) = n
                    && let Some((label, def)) = counters.next_label(numbering, n.num, n.level)
                    && !matches!(def.format, section::NumFormat::Bullet | section::NumFormat::None)
                {
                    total = total.saturating_add(count_words(&label));
                }
                total = total.saturating_add(count_words(&p.plain_text()));
            }
            Block::Table(t) => {
                for row in &t.rows {
                    for c in &row.cells {
                        total = total.saturating_add(story_word_count_of(&c.blocks, numbering, styles, counters, depth + 1));
                    }
                }
            }
        }
    }
    total
}
