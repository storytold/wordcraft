//! WordCraft reader for the Word 97-2003 binary file format (`.doc`, [MS-DOC]).
//!
//! [`read`] turns the bytes of a `.doc` file into a [`wordcraft_doc::Document`]. There is no
//! writer: saving always goes through the other formats. The reader is written from the public
//! specification ([MS-DOC], Word (.doc) Binary File Format, Microsoft Open Specifications) and,
//! like every WordCraft parser, is lenient by design (unknown structures are skipped, bad numbers
//! fall back to defaults) and bounded (stream sizes, piece counts and property counts are
//! capped), so hostile input yields an error or a best-effort document, never a panic.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod fib;
mod fkp;
mod fmt;
mod list;
mod media;
mod notes;
mod piece;
mod sections;
mod sprm;
mod stsh;
mod table;

use std::io::{Read, Seek};

use wordcraft_doc::para::InlineObject;
use wordcraft_doc::para::NoteKind;
use wordcraft_doc::para::Paragraph;
use wordcraft_doc::para::Run;
use wordcraft_doc::props::CharProps;
use wordcraft_doc::props::NumRef;
use wordcraft_doc::section::HeaderSet;
use wordcraft_doc::styles::StyleSheet;
use wordcraft_doc::{Document, PartKind};

use fkp::Bins;
use piece::{PieceTable, PrmRef};
use sprm::Prl;
use table::ParaOut;

/// Largest stream we materialise (bytes); the engine already caps whole files at 2 GB.
const MAX_STREAM: u64 = 1 << 30;
/// Most header/footer parts we create (matches the docx reader's cap).
const MAX_PARTS: usize = 50_000;
/// `sprmCPicLocation`: the fc of a picture in the Data stream.
const C_PIC_LOCATION: u16 = 0x6A03;
/// `sprmPIlvl` / `sprmPIlfo`: the paragraph's list level and list.
const P_ILVL: u16 = 0x260A;
const P_ILFO: u16 = 0x460B;

/// Errors from reading a Word 97-2003 binary file.
#[derive(Debug, thiserror::Error, Clone, PartialEq)]
pub enum DocbinError {
    /// The bytes are not a readable compound file (OLE2/CFB container).
    #[error("not a Word document (compound file): {0}")]
    Container(String),
    /// The compound file is readable but is not a Word 97-2003 document.
    #[error("not a Word 97-2003 document: {0}")]
    NotWord(String),
    /// The document is password-protected (XOR obfuscation or RC4 encryption).
    #[error("encrypted or password-protected .doc files are not supported")]
    Encrypted,
    /// The input exceeds a safety limit (stream size, piece count…).
    #[error("limit exceeded: {0}")]
    Limit(String),
    /// A structure is malformed beyond recovery.
    #[error("malformed document: {0}")]
    Malformed(String),
}

/// Read a `.doc` (Word 97-2003 binary) file into a [`Document`].
pub fn read(bytes: &[u8]) -> Result<Document, DocbinError> {
    let mut comp = cfb::CompoundFile::open(std::io::Cursor::new(bytes)).map_err(|e| DocbinError::Container(e.to_string()))?;
    let word = stream(&mut comp, "WordDocument").ok_or_else(|| DocbinError::NotWord("no WordDocument stream".into()))?;
    if word.len() as u64 > MAX_STREAM {
        return Err(DocbinError::Limit(format!("WordDocument stream is {} bytes", word.len())));
    }
    let fib = fib::Fib::parse(&word)?;
    if fib.encrypted {
        return Err(DocbinError::Encrypted);
    }
    // The FIB names the Table stream (0Table or 1Table); fall back to whichever exists,
    // since files written by other writers sometimes carry the wrong bit.
    let named = if fib.which_tbl_stm { "1Table" } else { "0Table" };
    let other = if fib.which_tbl_stm { "0Table" } else { "1Table" };
    let table = stream(&mut comp, named).or_else(|| stream(&mut comp, other)).unwrap_or_default();
    if table.len() as u64 > MAX_STREAM {
        return Err(DocbinError::Limit(format!("Table stream is {} bytes", table.len())));
    }

    // The piece table locates every character in the WordDocument stream.
    let (fc_clx, lcb_clx) =
        fib.pair(fib::pair::CLX).filter(|(_, lcb)| *lcb > 0).ok_or_else(|| DocbinError::Malformed("the FIB has no piece table (Clx)".into()))?;
    let pieces = piece::PieceTable::parse(&table, fc_clx, lcb_clx)?;

    // Styles, fonts and the formatting bin tables; each is optional in broken files.
    let fonts = fib.pair(fib::pair::STTBF_FFN).map(|(f, l)| stsh::fonts(&table, f, l)).unwrap_or_default();
    let raw_styles = match fib.pair(fib::pair::STSHF) {
        Some((f, l)) => stsh::parse(&table, f, l)?,
        None => Vec::new(),
    };
    let chpx_bins = match fib.pair(fib::pair::PLCF_BTE_CHPX) {
        Some((f, l)) => Bins::parse(&table, f, l)?,
        None => Bins::parse(&[], 0, 0)?,
    };
    let papx_bins = match fib.pair(fib::pair::PLCF_BTE_PAPX) {
        Some((f, l)) => Bins::parse(&table, f, l)?,
        None => Bins::parse(&[], 0, 0)?,
    };

    let data = stream(&mut comp, "Data").unwrap_or_default();
    if data.len() as u64 > MAX_STREAM {
        return Err(DocbinError::Limit(format!("Data stream is {} bytes", data.len())));
    }

    let mut doc = Document::new();
    doc.body.clear();
    doc.styles = fmt::stylesheet(&raw_styles, &fonts);
    doc.numbering = list::parse(&table, &fib, &fonts);
    let sheet: StyleSheet = doc.styles.clone();
    let sects = sections::parse(&word, &table, &fib);
    let stories = sections::header_stories(&table, &fib);
    // Subdocument layout in the CP space: main document, footnotes, headers, comments,
    // endnotes, text boxes, header text boxes.
    // Story ranges from the Table stream are hostile: each is clamped to its subdocument's
    // length here and to the last piece's CP inside `walk`.
    let ftn_base = fib.ccp.text;
    let hdd_base = fib.ccp.text.saturating_add(fib.ccp.ftn);
    let edn_base = hdd_base.saturating_add(fib.ccp.hdd).saturating_add(fib.ccp.atn);
    let mut pics = media::Pictures::default();

    let ctx = WalkCtx {
        word: &word,
        data: &data,
        pieces: &pieces,
        chpx_bins: &chpx_bins,
        papx_bins: &papx_bins,
        fonts: &fonts,
        raw_styles: &raw_styles,
        sheet: &sheet,
        notes: &[],
        marks: &[],
    };

    // Header and footer parts: skip the six separator stories, then six stories per section
    // in the order even header, odd (default) header, even footer, odd (default) footer,
    // first header, first footer. Empty stories inherit from the previous section.
    let mut hf_ids: Vec<(usize, usize, u32)> = Vec::new();
    for si in 0..sects.len() {
        for k in 0..6usize {
            let Some(&ab) = stories.get(6 + si * 6 + k) else { break };
            let Some((a, b)) = story(hdd_base, fib.ccp.hdd, ab) else { continue };
            if doc.parts.len() >= MAX_PARTS {
                continue;
            }
            let kind = match k {
                0 | 1 | 4 => PartKind::Header,
                _ => PartKind::Footer,
            };
            let blocks = table::assemble(walk(&ctx, a, b, &mut pics));
            if !blocks.is_empty() {
                let id = doc.add_part(kind, blocks);
                hf_ids.push((si, k, id));
            }
        }
    }

    // Footnote and endnote stories: each text range becomes a part; the reference CPs then
    // link to the part ids during the main walk.
    let mut note_links: Vec<(u32, NoteKind, u32)> = Vec::new();
    for (cp, ab) in notes::parse_notes(&table, &fib, fib::pair::PLCF_FND_REF, fib::pair::PLCF_FND_TXT) {
        let Some((a, b)) = story(ftn_base, fib.ccp.ftn, ab) else { continue };
        let blocks = table::assemble(walk(&ctx, a, b, &mut pics));
        if !blocks.is_empty() && doc.parts.len() < MAX_PARTS {
            let id = doc.add_part(PartKind::Footnote, blocks);
            note_links.push((cp, NoteKind::Footnote, id));
        }
    }
    for (cp, ab) in notes::parse_notes(&table, &fib, fib::pair::PLCF_END_REF, fib::pair::PLCF_END_TXT) {
        let Some((a, b)) = story(edn_base, fib.ccp.edn, ab) else { continue };
        let blocks = table::assemble(walk(&ctx, a, b, &mut pics));
        if !blocks.is_empty() && doc.parts.len() < MAX_PARTS {
            let id = doc.add_part(PartKind::Endnote, blocks);
            note_links.push((cp, NoteKind::Endnote, id));
        }
    }
    // Sorted so the main walk finds each reference by binary search.
    note_links.sort_by_key(|(cp, _, _)| *cp);
    let marks = notes::parse_bookmarks(&table, &fib);
    let ctx = WalkCtx { notes: &note_links, marks: &marks, ..ctx };

    // Sections: each section's properties attach to the paragraph that ends it (its story of
    // header/footer parts included); the section reaching the end of the text is the final one.
    let mut paras = walk(&ctx, 0, fib.ccp.text, &mut pics);
    let last = sects.last().map(|s| s.props.clone());
    for (si, sec) in sects.iter().enumerate() {
        let mut props = sec.props.clone();
        let id_of = |k: usize| hf_ids.iter().find(|(s, kk, _)| *s == si && *kk == k).map(|(_, _, id)| *id);
        props.headers = HeaderSet { even: id_of(0), default: id_of(1), first: id_of(4) };
        props.footers = HeaderSet { even: id_of(2), default: id_of(3), first: id_of(5) };
        let is_final = si + 1 == sects.len() || sec.end_cp >= fib.ccp.text;
        if is_final {
            continue; // handled below through `last`
        }
        for p in paras.iter_mut() {
            if p.end_cp == sec.end_cp {
                p.para.section = Some(Box::new(props));
                break;
            }
        }
    }
    if let Some(mut l) = last {
        let si = sects.len().saturating_sub(1);
        let id_of = |k: usize| hf_ids.iter().find(|(s, kk, _)| *s == si && *kk == k).map(|(_, _, id)| *id);
        l.headers = HeaderSet { even: id_of(0), default: id_of(1), first: id_of(4) };
        l.footers = HeaderSet { even: id_of(2), default: id_of(3), first: id_of(5) };
        doc.last_section = l;
    }

    doc.media.extend(pics.media);
    doc.body = table::assemble(paras);
    doc.ensure_nonempty();
    Ok(doc)
}

/// A story's CP range `[a, b)`, relative to a subdocument starting at `base` with `len`
/// CPs, as absolute CPs clamped to that subdocument; `None` when empty.
fn story(base: u32, len: u32, (a, b): (u32, u32)) -> Option<(u32, u32)> {
    let (a, b) = (a.min(len), b.min(len));
    if b <= a {
        return None;
    }
    Some((base.checked_add(a)?, base.checked_add(b)?))
}

/// Everything the walker needs besides its CP range.
struct WalkCtx<'a> {
    word: &'a [u8],
    data: &'a [u8],
    pieces: &'a PieceTable,
    chpx_bins: &'a Bins,
    papx_bins: &'a Bins,
    fonts: &'a [String],
    raw_styles: &'a [stsh::RawStyle],
    sheet: &'a StyleSheet,
    /// (reference CP in the main document, kind, part id) for notes.
    notes: &'a [(u32, NoteKind, u32)],
    /// Bookmark boundaries (main-document CPs), sorted for a sequential walk.
    marks: &'a [(u32, notes::Mark)],
}

/// State while assembling one paragraph from per-CP formatting.
#[derive(Default)]
struct ParaBuild {
    text: String,
    runs: Vec<Run>,
    props: wordcraft_doc::props::ParaProps,
    mark: CharProps,
    /// The current run's properties, so runs only break when formatting changes.
    cur: Option<CharProps>,
    /// Inline objects, each anchored at a U+FFFC pushed into `text`.
    objects: Vec<InlineObject>,
}

impl ParaBuild {
    fn push(&mut self, c: char, props: CharProps) {
        let len = c.len_utf8();
        match &self.cur {
            Some(p) if *p == props => {
                if let Some(r) = self.runs.last_mut() {
                    r.len += len;
                }
            }
            _ => {
                self.runs.push(Run { len, props: props.clone() });
                self.cur = Some(props);
            }
        }
        self.text.push(c);
    }
}

/// Walk a CP range, splitting paragraphs at 0x0D/0x07 marks, applying the direct character
/// formatting of each CP and the paragraph formatting of each mark. Works for the main
/// document and for any subdocument range (headers, footers, notes…). Field characters,
/// note references, bookmarks and inline pictures become inline objects; pictures go into
/// `pics`, which shares one media entry per Data-stream location.
///
/// The range is clamped to the last piece's CP, and unreadable stretches (gaps, truncated
/// pieces) are skipped a whole piece at a time, so the work is bounded by the text that
/// really exists, not by the CP numbers a file claims.
fn walk(ctx: &WalkCtx, cp_start: u32, cp_end: u32, pics: &mut media::Pictures) -> Vec<ParaOut> {
    let cp_end = cp_end.min(ctx.pieces.cp_end());
    let mut out = Vec::new();
    let mut pb = ParaBuild::default();
    let mut cp = cp_start;
    let mut marks = ctx.marks.iter().peekable();
    // Field assembly state: instruction characters are swallowed, the result is kept until
    // the end character, then emitted as one object (hyperlinks keep their result text).
    let mut field: Option<FieldBuild> = None;
    while cp < cp_end {
        // Bookmark boundaries at this CP (main-document walks only; subdocument walks get
        // an empty table and `cp` never matches the main-document CPs).
        // `<=` so a boundary inside a skipped stretch is still emitted (late) rather than
        // blocking every later one.
        while marks.peek().is_some_and(|&(mcp, _)| *mcp <= cp) {
            if let Some((_, m)) = marks.next() {
                let obj = match m {
                    notes::Mark::Start(name) => InlineObject::BookmarkStart { name: name.clone() },
                    notes::Mark::End(name) => InlineObject::BookmarkEnd { name: name.clone() },
                };
                pb.push('\u{FFFC}', CharProps::default());
                pb.objects.push(obj);
            }
        }
        let (c, units) = match ctx.pieces.char_at(ctx.word, cp) {
            Ok(found) => found,
            Err(next) => {
                cp = next;
                continue;
            }
        };
        match c {
            '\r' | '\u{7}' => {
                // Paragraph mark: its PAPX covers the FC range of the paragraph it ends, so
                // querying at the mark's own FC finds that paragraph's properties.
                let fc_mark = ctx.pieces.fc_of_cp(cp).unwrap_or(0);
                let (istd, papx) = ctx.papx_bins.papx(ctx.word, fc_mark);
                if let Some(st) = ctx.raw_styles.get(istd as usize).filter(|s| !s.name.is_empty()) {
                    pb.props.style = Some(fmt::style_id(&st.name));
                }
                let mut row = table::decode(papx);
                row.cell_mark = c == '\u{7}';
                let mut ilvl = 0u8;
                let mut ilfo = 0i32;
                for prl in sprm::iter(papx) {
                    if prl.op == P_ILVL {
                        ilvl = prl.operand.first().copied().unwrap_or(0).min(8);
                        continue;
                    }
                    if prl.op == P_ILFO {
                        ilfo = match prl.operand {
                            [b0, b1, ..] => i16::from_le_bytes([*b0, *b1]) as i32,
                            _ => 0,
                        };
                        continue;
                    }
                    if prl.sgc() == 1 {
                        fmt::apply_para(&mut pb.props, &prl);
                    }
                }
                if let Some(num) = num_of(ilfo) {
                    pb.props.numbering = Some(NumRef { num, level: ilvl });
                }
                let fc = ctx.pieces.fc_of_cp(cp).unwrap_or(0);
                pb.mark = char_props(ctx.chpx_bins.chpx(ctx.word, fc), ctx.fonts, &CharProps::default());
                let done = std::mem::take(&mut pb);
                out.push(ParaOut {
                    para: Paragraph {
                        text: done.text,
                        runs: done.runs,
                        props: done.props,
                        mark: done.mark,
                        objects: done.objects,
                        ..Default::default()
                    },
                    end_cp: cp.saturating_add(1),
                    row,
                });
            }
            // Field begin/separator/end ([MS-DOC] §2.9.84 Plcfld).
            '\u{13}' => {
                match field.as_mut() {
                    Some(f) => {
                        f.depth = f.depth.saturating_add(1);
                    }
                    None => field = Some(FieldBuild { depth: 1, instr: String::new(), sep: None }),
                }
                cp = cp.saturating_add(1);
                continue;
            }
            '\u{14}' => {
                if let Some(f) = field.as_mut()
                    && f.depth == 1
                    && f.sep.is_none()
                {
                    f.sep = Some(Vec::new());
                }
                cp = cp.saturating_add(1);
                continue;
            }
            '\u{15}' => {
                if let Some(mut f) = field.take() {
                    if f.depth > 1 {
                        f.depth -= 1;
                        field = Some(f);
                    } else {
                        finish_field(&mut pb, f);
                    }
                }
                cp = cp.saturating_add(1);
                continue;
            }
            '\u{1}' => {
                // Inline picture: the CHPX of the anchor carries sprmCPicLocation.
                let fc = ctx.pieces.fc_of_cp(cp).unwrap_or(0);
                let fc_pic = pic_location(ctx.chpx_bins.chpx(ctx.word, fc));
                match fc_pic.and_then(|at| pics.get(ctx.data, at)) {
                    Some((key, w, h)) => {
                        pb.push('\u{FFFC}', CharProps::default());
                        pb.objects.push(InlineObject::Image { media: key, w, h, alt: String::new(), float: Default::default(), crop: [0.0; 4] });
                    }
                    None => pics.dropped(cp),
                }
            }
            '\u{2}' => {
                // Footnote or endnote reference; custom symbols share the same character.
                let at = ctx.notes.partition_point(|(c, _, _)| *c < cp);
                if let Some((_, kind, id)) = ctx.notes.get(at).filter(|(c, _, _)| *c == cp) {
                    pb.push('\u{FFFC}', CharProps::default());
                    pb.objects.push(InlineObject::NoteRef { kind: *kind, id: *id, custom: String::new() });
                }
            }
            '\t' => push_formatted(&mut pb, cp, '\t', ctx, field.as_mut()),
            '\u{B}' => push_formatted(&mut pb, cp, '\n', ctx, field.as_mut()),
            '\u{C}' => push_formatted(&mut pb, cp, '\u{C}', ctx, field.as_mut()),
            '\u{E}' => push_formatted(&mut pb, cp, '\u{E}', ctx, field.as_mut()),
            '\u{1E}' => push_formatted(&mut pb, cp, '\u{2011}', ctx, field.as_mut()),
            '\u{1F}' => push_formatted(&mut pb, cp, '\u{AD}', ctx, field.as_mut()),
            // Object anchors, comment references and other special characters are dropped
            // (or handled by later passes).
            c if (c as u32) < 0x20 => {}
            c => push_formatted(&mut pb, cp, c, ctx, field.as_mut()),
        }
        cp = cp.saturating_add(units);
    }
    if !pb.text.is_empty() {
        let done = pb;
        out.push(ParaOut {
            para: Paragraph { text: done.text, runs: done.runs, props: done.props, mark: done.mark, objects: done.objects, ..Default::default() },
            end_cp: cp_end,
            row: table::RowInfo::default(),
        });
    }
    out
}

/// Field assembly: `instr` collects everything before the separator (nested fields
/// flattened), `sep` holds the cached result once the separator was seen.
struct FieldBuild {
    depth: u32,
    instr: String,
    sep: Option<Vec<(char, CharProps)>>,
}

/// Emit a finished field: HYPERLINK keeps its result text as a linked run; every other
/// field becomes one inline object with its cached result.
fn finish_field(pb: &mut ParaBuild, f: FieldBuild) {
    let instr = f.instr.trim().to_string();
    let result = f.sep.unwrap_or_default();
    let result_text: String = result.iter().map(|(c, _)| *c).collect();
    if let Some(target) = hyperlink_target(&instr) {
        for (c, mut props) in result {
            props.link = Some(target.clone());
            pb.push(c, props);
        }
        return;
    }
    pb.push('\u{FFFC}', CharProps::default());
    pb.objects.push(InlineObject::Field { instr, result: result_text, locked: false });
}

/// `HYPERLINK "url"` / `HYPERLINK url \l "bookmark"` → the model link target
/// (`#bookmark` for internal links); `None` when not a hyperlink.
fn hyperlink_target(instr: &str) -> Option<String> {
    let rest = instr.strip_prefix("HYPERLINK").or_else(|| instr.strip_prefix("hyperlink"))?;
    let rest = rest.strip_prefix(' ').unwrap_or(rest);
    if rest.is_empty() {
        return None;
    }
    let mut quoted = None;
    let mut local = None;
    let mut tokens = rest.split_whitespace().peekable();
    while let Some(tok) = tokens.next() {
        match tok {
            "\\l" => {
                local = Some(tokens.next().unwrap_or_default().trim_matches('"').to_string());
            }
            "\\o" | "\\m" | "\\n" | "\\t" => {
                let _ = tokens.next();
            }
            _ if quoted.is_none() => quoted = Some(tok.trim_matches('"').to_string()),
            _ => {}
        }
    }
    Some(match local {
        Some(name) if !name.is_empty() => format!("#{name}"),
        _ => quoted.unwrap_or_default(),
    })
}

/// `sprmCPicLocation` from a CHPX grpprl: the fc of the picture in the Data stream.
fn pic_location(chpx: &[u8]) -> Option<u32> {
    for prl in sprm::iter(chpx) {
        if prl.op == C_PIC_LOCATION {
            return match prl.operand {
                [b0, b1, b2, b3, ..] => Some(u32::from_le_bytes([*b0, *b1, *b2, *b3])),
                _ => None,
            };
        }
    }
    None
}

/// `sprmPIlfo` operand → the num id, if the paragraph is in a list. Values 0xF802-0xFFFF are
/// the negation of a 1-based index and keep the paragraph's own indents, which we do by
/// leaving the level's indents out of the paragraph (they never enter `ParaProps` anyway).
fn num_of(ilfo: i32) -> Option<u32> {
    match ilfo {
        0x0001..=0x07FE => Some(ilfo as u32),
        0xF802..=0xFFFF => Some((-(ilfo as i16)) as i32 as u32),
        _ => None,
    }
}

/// Resolve the character properties for the character at `cp`: the CHPX grpprl of its FC,
/// plus the piece's Prm, over the paragraph style's base character props. Inside a field,
/// the character is recorded into the instruction or the cached result instead of (or as
/// well as) the visible text.
fn push_formatted(pb: &mut ParaBuild, cp: u32, c: char, ctx: &WalkCtx, field: Option<&mut FieldBuild>) {
    let fc = ctx.pieces.fc_of_cp(cp).unwrap_or(0);
    let grpprl = ctx.chpx_bins.chpx(ctx.word, fc);
    let default;
    let base = match pb.props.style.as_deref().and_then(|id| ctx.sheet.get(id)) {
        Some(st) => &st.chr,
        None => {
            default = CharProps::default();
            &default
        }
    };
    let mut props = char_props(grpprl, ctx.fonts, base);
    apply_prm(&mut props, ctx.pieces.piece_prm(cp), ctx.fonts, base);
    match field {
        Some(f) => match f.sep.as_mut() {
            // Inside a field, characters are recorded for the field object instead of shown;
            // `finish_field` emits the visible text (hyperlink runs or one object anchor).
            Some(result) => result.push((c, props)),
            None => f.instr.push(c),
        },
        None => pb.push(c, props),
    }
}

/// Apply a piece's Prm (Prm0 short sprm or Prm1 grpprl) to character properties.
fn apply_prm(props: &mut CharProps, prm: PrmRef<'_>, fonts: &[String], base: &CharProps) {
    match prm {
        PrmRef::None => {}
        PrmRef::Sprm0 { isprm, val } => {
            if let Some(op) = sprm::from_prm0(isprm) {
                let operand = [val];
                let prl = Prl { op, operand: &operand };
                if prl.sgc() == 2 {
                    fmt::apply_char(props, &prl, base, fonts);
                }
            }
        }
        PrmRef::Grpprl(Some(g)) => {
            for prl in sprm::iter(g) {
                if prl.sgc() == 2 {
                    fmt::apply_char(props, &prl, base, fonts);
                }
            }
        }
        PrmRef::Grpprl(None) => {}
    }
}

/// Character props from a CHPX grpprl applied over `base`.
fn char_props(grpprl: &[u8], fonts: &[String], base: &CharProps) -> CharProps {
    let mut p = base.clone();
    for prl in sprm::iter(grpprl) {
        if prl.sgc() == 2 {
            fmt::apply_char(&mut p, &prl, base, fonts);
        }
    }
    p
}

/// Read a root-level stream fully, or `None` if it is absent.
fn stream<F: Read + Seek>(comp: &mut cfb::CompoundFile<F>, name: &str) -> Option<Vec<u8>> {
    let mut s = comp.open_stream(format!("/{name}")).ok()?;
    let mut v = Vec::new();
    // Read with a cap one byte above the limit so oversized streams are still caught by the
    // caller instead of exhausting memory here.
    s.by_ref().take(MAX_STREAM + 1).read_to_end(&mut v).ok()?;
    Some(v)
}

#[cfg(test)]
mod tests;
