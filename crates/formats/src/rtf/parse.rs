use super::reader::Reader;
use super::state::{CommentData, Dest, HeaderFooter, HfPages, MARK_CLOSE, MARK_OPEN, MAX_PICT, NoteData, PageGeom};
use super::{Arc, Block, Blocks, Cell, Document, FBlock, Flow, HashMap, InlineObject, Para, Paragraph, model};

/// Parse RTF bytes into the flow model.
pub fn parse(bytes: &[u8]) -> Result<Flow, String> {
    parse_with_page(bytes).map(|(f, _, _)| f)
}

/// The flow, the page, and the default font (`\deff`, font 0 when the file names none), which
/// is the font of all text that doesn't select one.
fn parse_with_page(bytes: &[u8]) -> Result<(Flow, PageGeom, Option<String>), String> {
    parse_full(bytes, false).map(|r| (r.flow, r.page, r.default_font))
}

/// Everything an import takes from an RTF file.
struct Parsed {
    flow: Flow,
    page: PageGeom,
    /// The last section's properties (the others are on the paragraphs that end them).
    last_section: wordcraft_doc::SectionProps,
    default_font: Option<String>,
    /// Each section's headers and footers, in order, the last section's last.
    headers_footers: Vec<Vec<(HeaderFooter, Vec<FBlock>)>>,
    notes: Vec<NoteData>,
    comments: Vec<CommentData>,
}

/// `marks`: also read footnotes, endnotes and comments (as place markers in the flow's text, for
/// [`import`] to turn into objects); without it they are skipped.
fn parse_full(bytes: &[u8], marks: bool) -> Result<Parsed, String> {
    let start = bytes.iter().position(|b| !b.is_ascii_whitespace()).unwrap_or(0);
    if !bytes.get(start..).is_some_and(|b| b.starts_with(b"{\\rtf")) {
        return Err("not an RTF document (it doesn't start with {\\rtf)".into());
    }
    let mut r = Reader::default();
    r.marks_on = marks;
    let mut i = start;
    let mut first_in_group = false;
    let mut starred = false;
    while let Some(&c) = bytes.get(i) {
        match c {
            b'{' => {
                r.open();
                first_in_group = true;
                starred = false;
                i += 1;
                continue;
            }
            b'}' => {
                r.close();
                if r.info_depth.is_some_and(|d| r.depth() < d) {
                    r.info_depth = None;
                }
                first_in_group = false;
                starred = false;
                i += 1;
                continue;
            }
            b'\\' => {
                let Some(&n) = bytes.get(i + 1) else { break };
                if n.is_ascii_alphabetic() {
                    let mut j = i + 1;
                    while bytes.get(j).is_some_and(u8::is_ascii_alphabetic) && j - i <= 32 {
                        j += 1;
                    }
                    let word = std::str::from_utf8(bytes.get(i + 1..j).unwrap_or(b"")).unwrap_or("");
                    let mut k = j;
                    let neg = bytes.get(k) == Some(&b'-');
                    if neg {
                        k += 1;
                    }
                    let d0 = k;
                    while bytes.get(k).is_some_and(u8::is_ascii_digit) && k - d0 < 10 {
                        k += 1;
                    }
                    let param = if k > d0 {
                        let v = std::str::from_utf8(bytes.get(d0..k).unwrap_or(b"0")).ok().and_then(|s| s.parse::<i64>().ok()).unwrap_or(0);
                        let v = if neg { -v } else { v };
                        // A date (DTTM) is a 32-bit pattern some writers put unsigned.
                        let v = if matches!(word, "revdttm" | "revdttmdel" | "crdate" | "prdate")
                            && v > i64::from(i32::MAX)
                            && v <= i64::from(u32::MAX)
                        {
                            v - (1i64 << 32)
                        } else {
                            v
                        };
                        Some(v.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32)
                    } else {
                        if neg {
                            k -= 1;
                        }
                        None
                    };
                    if bytes.get(k) == Some(&b' ') {
                        k += 1;
                    }
                    i = k;
                    if word == "bin" {
                        let n = param.unwrap_or(0).max(0) as usize;
                        let end = i.saturating_add(n).min(bytes.len());
                        if r.st.dest == Dest::Pict
                            && let Some(data) = bytes.get(i..end)
                            && r.pict_hex.len() + data.len() <= MAX_PICT
                        {
                            r.pict_hex.extend_from_slice(data);
                        }
                        i = end;
                        continue;
                    }
                    if word == "info" {
                        r.info_depth = Some(r.depth());
                    }
                    if r.skip_chars > 0
                        && matches!(r.st.dest, Dest::Body | Dest::Info(_) | Dest::FldInst | Dest::Bookmark | Dest::FontTbl | Dest::Atn(_))
                        && !matches!(word, "par" | "pard" | "cell" | "row")
                    {
                        r.skip_chars -= 1;
                        first_in_group = false;
                        continue;
                    }
                    r.word(word, param, starred, first_in_group);
                    first_in_group = false;
                    starred = false;
                    continue;
                }
                match n {
                    b'\'' => {
                        let h = bytes.get(i + 2..i + 4).and_then(|h| std::str::from_utf8(h).ok()).and_then(|h| u8::from_str_radix(h, 16).ok());
                        match h {
                            Some(v) => {
                                if r.st.dest == Dest::Pict {
                                    // Hex digits, two at a time, without a String per picture byte.
                                    const HEX: &[u8; 16] = b"0123456789abcdef";
                                    let h = [HEX[usize::from(v >> 4)], HEX[usize::from(v & 0x0f)]];
                                    if let Ok(s) = std::str::from_utf8(&h) {
                                        r.text(s);
                                    }
                                } else {
                                    r.byte(v);
                                }
                                i += 4;
                            }
                            None => i += 2,
                        }
                    }
                    b'*' => {
                        starred = true;
                        i += 2;
                        continue;
                    }
                    b'~' | b' ' => {
                        r.ch('\u{00A0}');
                        i += 2;
                    }
                    b'-' => {
                        r.ch('\u{00AD}');
                        i += 2;
                    }
                    b'_' => {
                        r.ch('\u{2011}');
                        i += 2;
                    }
                    b'\n' | b'\r' => {
                        r.word("par", None, false, false);
                        i += 2;
                    }
                    b'\\' | b'{' | b'}' => {
                        r.ch(n as char);
                        i += 2;
                    }
                    _ => i += 2,
                }
                first_in_group = false;
            }
            b'\r' | b'\n' | 0 => i += 1,
            _ => {
                // A run of plain text.
                let mut j = i;
                while bytes.get(j).is_some_and(|b| !matches!(b, b'{' | b'}' | b'\\' | b'\r' | b'\n' | 0)) {
                    j += 1;
                }
                let chunk = bytes.get(i..j).unwrap_or(b"");
                if r.st.dest == Dest::Pict && r.skip_chars == 0 && chunk.is_ascii() {
                    // Picture hex is not text: it needs no decoder, so neither a pending lead byte
                    // nor the code page's single-byte path applies to it.
                    if let Ok(s) = std::str::from_utf8(chunk) {
                        r.text(s);
                    }
                } else if r.skip_chars == 0 && chunk.is_ascii() && r.mb_decoder.is_none() {
                    // Plain ASCII with no multibyte run open: append it as it is.
                    if let Ok(s) = std::str::from_utf8(chunk) {
                        r.text(s);
                    }
                } else {
                    r.run(chunk);
                }
                i = j;
                first_in_group = false;
            }
        }
    }
    Ok(finalize(r))
}

fn finalize(mut r: Reader) -> Parsed {
    // A multibyte run ends here, and so does a note, comment, header or footer left open by a
    // truncated file.
    r.finish_multibyte();
    r.end_sub();
    r.end_header_footer();
    if r.para_has || !r.para.inlines.is_empty() {
        r.end_para(false);
    }
    r.close_table();
    if let Some((y, mo, d, h, mi)) = r.created
        && y > 0
    {
        r.meta.created = format!("{y:04}-{:02}-{:02}T{:02}:{:02}:00Z", mo.clamp(1, 12), d.clamp(1, 31), h.min(23), mi.min(59));
    }
    let mut blocks = std::mem::take(&mut r.body);
    // A file that ends with `\sect` ends its last section there: no empty one follows, and
    // header or footer groups after that `\sect` (a section without text) are dropped.
    let mut last_section = r.sec.props(&r.page);
    let mut headers_footers = std::mem::take(&mut r.section_parts);
    if let Some(FBlock::Para(p)) = blocks.last_mut()
        && let Some(s) = p.section.take()
    {
        last_section = *s;
    } else {
        headers_footers.push(std::mem::take(&mut r.hf_parts));
    }
    if blocks.is_empty() {
        blocks.push(FBlock::Para(Para::default()));
    }
    let default_font = r.fonts.get(&r.deff).filter(|n| !model::is_mono(n)).map(|n| n.to_string());
    Parsed { flow: Flow { blocks, meta: r.meta }, page: r.page, last_section, default_font, headers_footers, notes: r.notes, comments: r.comments }
}

/// The column grid of a table from its rows' cell right edges: every distinct edge starts a grid
/// column, and each cell spans the grid columns between its left and right edges, as Word lays
/// out RTF rows whose cells don't line up. Returns the widths in points, or none (equal columns)
/// when the edges don't make a grid.
pub(super) fn column_grid(rows: &mut [Vec<Cell>], rights: &[Vec<i32>]) -> Vec<f32> {
    let mut edges: Vec<i32> = rights.iter().flatten().copied().filter(|&x| x > 0).collect();
    edges.sort_unstable();
    edges.dedup();
    let ok = rights.len() == rows.len()
        && rows
            .iter()
            .zip(rights)
            .all(|(r, x)| r.len() == x.len() && x.windows(2).all(|w| w.first() < w.get(1)) && x.first().is_some_and(|&v| v > 0))
        && !edges.is_empty()
        && edges.len() <= 63;
    if !ok {
        return Vec::new();
    }
    for (row, x) in rows.iter_mut().zip(rights) {
        let mut left = 0;
        for (c, &right) in row.iter_mut().zip(x) {
            let span = edges.iter().filter(|&&e| e > left && e <= right).count().max(1);
            c.colspan = span as u32;
            left = right;
        }
    }
    let mut prev = 0;
    edges
        .iter()
        .map(|&e| {
            let w = (e - prev) as f32 / 20.0;
            prev = e;
            w
        })
        .collect()
}

/// Parse RTF bytes into a document.
pub fn import(bytes: &[u8]) -> Result<Document, String> {
    model::begin_import();
    let Parsed { flow, last_section, default_font, headers_footers, notes, comments, .. } = parse_full(bytes, true)?;
    let mut doc = model::to_doc_in(&flow, last_section);
    // Each section's own headers and footers; those it leaves out stay linked to the previous.
    let mut sets = Vec::with_capacity(headers_footers.len());
    for parts in &headers_footers {
        let (mut headers, mut footers) = (wordcraft_doc::section::HeaderSet::default(), wordcraft_doc::section::HeaderSet::default());
        for (which, blocks) in parts {
            let blocks = model::part_blocks(&mut doc, blocks);
            let id = doc.add_part(if which.footer { wordcraft_doc::PartKind::Footer } else { wordcraft_doc::PartKind::Header }, blocks);
            let set = if which.footer { &mut footers } else { &mut headers };
            match which.pages {
                HfPages::Default => set.default = Some(id),
                HfPages::First => set.first = Some(id),
                HfPages::Even => set.even = Some(id),
            }
        }
        sets.push((headers, footers));
    }
    let mut sets = sets.into_iter();
    for b in &mut doc.body {
        if let wordcraft_doc::Block::Para(p) = &**b
            && p.section.is_some()
            && let wordcraft_doc::Block::Para(p) = std::sync::Arc::make_mut(b)
            && let Some(s) = p.section.as_deref_mut()
            && let Some((h, f)) = sets.next()
        {
            (s.headers, s.footers) = (h, f);
        }
    }
    if let Some((h, f)) = sets.next() {
        (doc.last_section.headers, doc.last_section.footers) = (h, f);
    }
    place_notes_and_comments(&mut doc, notes, comments);
    // RTF's paragraph defaults are single spacing and no space around paragraphs; what a file
    // wants beyond that it states on each paragraph.
    let dp = &mut doc.styles.default_para;
    dp.space_before = Some(0.0);
    dp.space_after = Some(0.0);
    dp.line_spacing = Some(wordcraft_doc::props::LineSpacing::Multiple(1.0));
    if default_font.is_some() {
        doc.styles.default_chr.font = default_font;
    }
    Ok(doc)
}

/// Turn the place markers the reader left in the text into note references and comment ranges,
/// and add the notes' and comments' stories to `doc`.
fn place_notes_and_comments(doc: &mut Document, notes: Vec<NoteData>, comments: Vec<CommentData>) {
    use wordcraft_doc::para::NoteKind;
    use wordcraft_doc::{Comment, PartKind};
    if notes.is_empty() && comments.is_empty() {
        return;
    }
    // (kind, part id) per note, by index.
    let mut note_parts: Vec<(NoteKind, u32)> = Vec::new();
    for n in notes {
        let (pk, nk, style) =
            if n.endnote { (PartKind::Endnote, NoteKind::Endnote, "EndnoteText") } else { (PartKind::Footnote, NoteKind::Footnote, "FootnoteText") };
        let mut blocks = model::part_blocks(doc, &n.blocks);
        for b in &mut blocks {
            if let Block::Para(p) = Arc::make_mut(b)
                && p.props.style.is_none()
            {
                p.props.style = Some(style.to_string());
            }
        }
        note_parts.push((nk, doc.add_part(pk, blocks)));
    }
    // RTF range id to the comment's id in the document.
    let mut by_rtf: HashMap<i64, u32> = HashMap::new();
    for (k, c) in comments.into_iter().enumerate() {
        let id = k as u32;
        let blocks = model::part_blocks(doc, &c.blocks);
        let part = doc.add_part(PartKind::Comment, blocks);
        doc.comments
            .insert(id, Comment { author: c.author, initials: c.initials, date: c.date, parent: None, resolved: false, part, ..Default::default() });
        by_rtf.insert(i64::from(c.rtf_id), id);
    }
    let ctx = Placer { note_parts, by_rtf };
    let mut body = std::mem::take(&mut doc.body);
    ctx.blocks(&mut body, None, 0);
    doc.body = body;
    let ids: Vec<u32> = doc.parts.keys().copied().collect();
    for id in ids {
        let Some(part) = doc.parts.get_mut(&id) else { continue };
        let own = ctx.note_parts.iter().find(|(_, p)| *p == id).copied();
        ctx.blocks(&mut part.blocks, own, 0);
        // A note starts with its own number, as Word's do.
        if let Some((kind, pid)) = own
            && let Some(first) = part.blocks.first_mut()
            && let Block::Para(p) = Arc::make_mut(first)
            && !p.objects.iter().any(|o| matches!(o, InlineObject::NoteRef { id, .. } if *id == pid))
        {
            let props = Placer::ref_props(kind);
            if p.insert_object(0, InlineObject::NoteRef { kind, id: pid, custom: String::new() }, &props).is_err() {
                // The paragraph can't take it; the note simply has no number mark.
            }
        }
    }
}

struct Placer {
    note_parts: Vec<(wordcraft_doc::para::NoteKind, u32)>,
    by_rtf: HashMap<i64, u32>,
}

impl Placer {
    fn ref_props(kind: wordcraft_doc::para::NoteKind) -> wordcraft_doc::CharProps {
        let style = if kind == wordcraft_doc::para::NoteKind::Endnote { "EndnoteReference" } else { "FootnoteReference" };
        wordcraft_doc::CharProps { style: Some(style.into()), ..Default::default() }
    }

    fn blocks(&self, bl: &mut Blocks, own: Option<(wordcraft_doc::para::NoteKind, u32)>, depth: usize) {
        for b in bl.iter_mut() {
            let needs = match &**b {
                Block::Para(p) => p.text.contains(MARK_OPEN),
                Block::Table(_) => depth < 8,
            };
            if !needs {
                continue;
            }
            match Arc::make_mut(b) {
                Block::Para(p) => self.para(p, own),
                Block::Table(t) => {
                    for r in &mut t.rows {
                        for c in &mut r.cells {
                            self.blocks(&mut c.blocks, own, depth + 1);
                        }
                    }
                }
            }
        }
    }

    fn para(&self, p: &mut Paragraph, own: Option<(wordcraft_doc::para::NoteKind, u32)>) {
        // Every pass removes one marker, so this ends; the bound is only a seat belt.
        for _ in 0..100_000 {
            let Some(a) = p.text.find(MARK_OPEN) else { return };
            let Some(len) = p.text.get(a..).and_then(|t| t.find(MARK_CLOSE)) else { return };
            let end = a + len + MARK_CLOSE.len_utf8();
            let inner = p.text.get(a + MARK_OPEN.len_utf8()..a + len).unwrap_or("").to_string();
            let mut chars = inner.chars();
            let tag = chars.next().unwrap_or(' ');
            let n = chars.as_str().parse::<i64>().unwrap_or(i64::MIN);
            let (obj, props) = match tag {
                'N' => match usize::try_from(n).ok().and_then(|i| self.note_parts.get(i)) {
                    Some(&(kind, id)) => (Some(InlineObject::NoteRef { kind, id, custom: String::new() }), Self::ref_props(kind)),
                    None => (None, Default::default()),
                },
                'S' => match own {
                    Some((kind, id)) => (Some(InlineObject::NoteRef { kind, id, custom: String::new() }), Self::ref_props(kind)),
                    None => (None, Default::default()),
                },
                'B' => (self.by_rtf.get(&n).map(|&id| InlineObject::CommentStart { id }), Default::default()),
                'E' => (self.by_rtf.get(&n).map(|&id| InlineObject::CommentEnd { id }), Default::default()),
                _ => (None, Default::default()),
            };
            if p.delete(a, end).is_err() {
                return;
            }
            if let Some(o) = obj
                && p.insert_object(a, o, &props).is_err()
            {
                return;
            }
        }
    }
}
