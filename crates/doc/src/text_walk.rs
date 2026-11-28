use super::block::{Block, Blocks};
use super::parts::PartKind;
use super::position::Path;
use super::{InlineObject, Numbering, numbering, para, section, styles};

/// Words in `s`, as Word counts them: tokens separated by whitespace that contain at least one
/// alphanumeric char, plus each Han ideograph and kana as a word of its own (Chinese and Japanese
/// are written without spaces, #4547; Korean is spaced, so Hangul counts by tokens).
pub fn count_words(s: &str) -> usize {
    // The half-width voiced sound marks belong to the kana before them, not a word of their own.
    let word_char = |c: char| c.is_alphanumeric() && !matches!(c, '\u{FF9E}' | '\u{FF9F}');
    let tokens = s.split(|c: char| c.is_whitespace() || c == para::OBJ || is_cjk_word(c)).filter(|w| w.chars().any(word_char)).count();
    tokens.saturating_add(s.chars().filter(|c| is_cjk_word(*c)).count())
}

/// Words in a generated list label: whitespace-separated tokens holding a letter or digit, so a
/// CJK label such as 第十二章 stays one word (how Word counts those is unverified).
fn count_label_words(s: &str) -> usize {
    s.split(char::is_whitespace).filter(|w| w.chars().any(char::is_alphanumeric)).count()
}

/// A Han ideograph or kana letter (not CJK punctuation), which Word counts as one word.
fn is_cjk_word(c: char) -> bool {
    matches!(c,
        '\u{3005}'..='\u{3007}' // 々 〆 〇
        | '\u{3031}'..='\u{3035}' | '\u{303B}' // vertical kana repeat marks, 〻
        | '\u{3040}'..='\u{30FF}' // hiragana, katakana
        | '\u{31F0}'..='\u{31FF}' // katakana phonetic extensions
        | '\u{3400}'..='\u{4DBF}' // extension A
        | '\u{4E00}'..='\u{9FFF}' // unified ideographs
        | '\u{F900}'..='\u{FAFF}' // compatibility ideographs
        | '\u{FF66}'..='\u{FF9D}' // half-width katakana
        | '\u{1B000}'..='\u{1B16F}' // kana supplement and extended
        | '\u{20000}'..='\u{3FFFF}' // extensions B and later
    ) && c.is_alphanumeric()
}

/// Characters in `s`, as the Word Count dialog counts them ("Characters (with spaces)"):
/// paragraph breaks aren't characters (#2797).
pub fn count_chars(s: &str) -> usize {
    s.chars().filter(|c| *c != '\n' && *c != '\r').count()
}

/// Characters in `s` ignoring every whitespace character, as the Word Count dialog counts them
/// ("Characters (no spaces)"; #2797).
pub fn count_chars_no_spaces(s: &str) -> usize {
    s.chars().filter(|c| !c.is_whitespace()).count()
}

/// How deep in nested tables [`Document::para_paths`] goes: cells below this are skipped.
pub const PARA_WALK_DEPTH: usize = 16;

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
        crate::work::steps(1);
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

/// The plain text [`text_of`] writes, less its last line break (as [`crate::Document::plain_text`]
/// drops it) and every U+FFFC, keeping only its first `left` characters. Text is fed one character
/// at a time, so a caller can stop reading its source once [`Self::cut`] is set.
pub(crate) struct CappedText {
    out: String,
    left: usize,
    /// Set when a character came after the last one kept.
    pub(crate) cut: bool,
    /// For each open table cell, the line breaks held back: [`text_of`] trims them from a cell's
    /// end and writes the others as spaces.
    held: Vec<usize>,
    /// A line break held back at the top level, dropped if nothing follows it.
    newline: bool,
}

impl CappedText {
    pub(crate) fn new(max_chars: usize) -> Self {
        Self { out: String::new(), left: max_chars, cut: false, held: Vec::new(), newline: false }
    }

    /// The text kept, and whether more followed it.
    pub(crate) fn finish(self) -> (String, bool) {
        (self.out, self.cut)
    }

    fn push(&mut self, c: char) {
        self.push_at(self.held.len(), c);
    }

    /// `c` written inside the `level` innermost open cells.
    fn push_at(&mut self, level: usize, c: char) {
        if self.cut {
            return;
        }
        let Some(l) = level.checked_sub(1) else {
            self.emit(c);
            return;
        };
        if c == '\n' {
            if let Some(h) = self.held.get_mut(l) {
                *h = h.saturating_add(1);
            }
            return;
        }
        let spaces = self.held.get_mut(l).map_or(0, std::mem::take);
        for _ in 0..spaces {
            if self.cut {
                return;
            }
            self.push_at(l, ' ');
        }
        self.push_at(l, c);
    }

    fn emit(&mut self, c: char) {
        if c == '\n' {
            if std::mem::replace(&mut self.newline, true) {
                self.keep('\n');
            }
            return;
        }
        if std::mem::take(&mut self.newline) {
            self.keep('\n');
        }
        if c != para::OBJ {
            self.keep(c);
        }
    }

    fn keep(&mut self, c: char) {
        if self.left == 0 {
            self.cut = true;
            return;
        }
        self.left -= 1;
        self.out.push(c);
    }

    /// Bytes `from..to` of `p` as [`para::Paragraph::plain_text`] writes them, then a line break.
    pub(crate) fn para(&mut self, p: &para::Paragraph, from: usize, to: usize) {
        let mut k = 0usize;
        for (i, c) in p.text.char_indices() {
            if i >= to || self.cut {
                break;
            }
            crate::work::steps(1);
            if c == para::OBJ {
                if i >= from
                    && let Some(o) = p.objects.get(k)
                {
                    for oc in o.plain_text().chars() {
                        if self.cut {
                            break;
                        }
                        self.push(oc);
                    }
                }
                k += 1;
            } else if i >= from {
                self.push(c);
            }
        }
        self.push('\n');
    }

    /// `b` as [`text_of`] writes it at `depth`.
    pub(crate) fn block(&mut self, b: &Block, depth: usize) {
        if depth > 16 {
            return;
        }
        match b {
            Block::Para(p) => self.para(p, 0, p.len()),
            Block::Table(t) => {
                for row in &t.rows {
                    for (j, c) in row.cells.iter().enumerate() {
                        if self.cut {
                            return;
                        }
                        if j > 0 {
                            self.push('\t');
                        }
                        self.held.push(0);
                        for cb in &c.blocks {
                            if self.cut {
                                break;
                            }
                            self.block(cb, depth + 1);
                        }
                        self.held.pop();
                    }
                    self.push('\n');
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
                    total = total.saturating_add(count_label_words(&label));
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
