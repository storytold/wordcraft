//! WordCraft reader for the Word 97-2003 binary file format (`.doc`, [MS-DOC]).
//!
//! [`read`] turns the bytes of a `.doc` file into a [`wordcraft_doc::Document`]. There is no
//! writer: saving always goes through the other formats. The reader is written from the public
//! specification ([MS-DOC], Word (.doc) Binary File Format, Microsoft Open Specifications) and,
//! like every WordCraft parser, is lenient by design (unknown structures are skipped, bad numbers
//! fall back to defaults) and bounded (stream sizes, piece counts and property counts are
//! capped), so hostile input yields an error or a best-effort document, never a panic.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod crypt;
mod drawing;
mod fib;
mod fkp;
mod fmt;
mod list;
mod media;
mod metadata;
mod notes;
mod officeart;
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
/// Hard ceiling on characters materialised across every story walk of one document (body,
/// headers, notes, text boxes), whatever the file's size. The working budget is tied to the
/// WordDocument stream (see [`read`]); this caps even a maximal stream.
const MAX_TOTAL_CHARS: u64 = 64 << 20;
/// Bytes of repeated work per document beyond the text itself: piece Prm grpprls (Prcs, up
/// to 64 KB each) interpreted (#3692), hyperlink targets (#3708), picture alt text (#3709)
/// and floating objects (#3710) copied into every run or anchor. Past it Prms are skipped,
/// copied strings left empty and floating objects dropped, each kind logged once a document
/// ([`Work::log`]).
const MAX_WORK: u64 = 64 << 20;

/// What the [`MAX_WORK`] budget pays for, and leaves out once it is spent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Cost {
    /// A piece's Prm grpprl: past the budget the piece keeps its CHPX formatting only.
    Formatting,
    /// A hyperlink target copied into a run of its result: past it the text is kept, unlinked.
    LinkTarget,
    /// A picture's alt text copied into an anchor: past it the picture has none.
    AltText,
    /// A floating object copied into its anchor: past it the object is left out.
    FloatingObject,
}

impl Cost {
    const ALL: [Cost; 4] = [Cost::Formatting, Cost::LinkTarget, Cost::AltText, Cost::FloatingObject];

    /// What is left out, for the log.
    fn left_out(self) -> &'static str {
        match self {
            Cost::Formatting => "some text keeps only part of its formatting",
            Cost::LinkTarget => "some hyperlinks' text is kept without its link",
            Cost::AltText => "some pictures are left without their alt text",
            Cost::FloatingObject => "some floating shapes and pictures are left out",
        }
    }
}

/// One document's [`MAX_WORK`] budget, and what it has left out (each kind logged once when
/// the document has been read, [`Work::log`]).
struct Work {
    left: std::cell::Cell<u64>,
    dropped: std::cell::Cell<[bool; 4]>,
}

impl Work {
    fn new(budget: u64) -> Self {
        Work { left: std::cell::Cell::new(budget), dropped: std::cell::Cell::new([false; 4]) }
    }

    /// Spends `cost` bytes on `what`: false (and `what` noted as left out) when they aren't left.
    fn charge(&self, cost: u64, what: Cost) -> bool {
        match self.left.get().checked_sub(cost) {
            Some(left) => {
                self.left.set(left);
                true
            }
            None => {
                let mut dropped = self.dropped.get();
                if let Some(d) = Cost::ALL.iter().position(|c| *c == what).and_then(|i| dropped.get_mut(i)) {
                    *d = true;
                }
                self.dropped.set(dropped);
                false
            }
        }
    }

    /// The kinds of work left out so far, in [`Cost::ALL`] order.
    fn dropped(&self) -> Vec<Cost> {
        Cost::ALL.iter().zip(self.dropped.get()).filter(|(_, d)| *d).map(|(c, _)| *c).collect()
    }

    /// Logs one warning for each kind left out.
    fn log(&self) {
        let dropped = self.dropped();
        for c in &dropped {
            log::warn!(".doc: the document repeats more than WordCraft copies ({MAX_WORK} bytes): {}", c.left_out());
        }
        #[cfg(test)]
        LAST_DROPPED.with(|d| *d.borrow_mut() = dropped);
    }
}

#[cfg(test)]
thread_local! {
    /// What the last document read on this thread left out for the work budget ([`Work::log`]).
    static LAST_DROPPED: std::cell::RefCell<Vec<Cost>> = const { std::cell::RefCell::new(Vec::new()) };
}
/// `sprmCPicLocation`: the fc of a picture in the Data stream.
const C_PIC_LOCATION: u16 = 0x6A03;
/// `sprmCSymbol`: the record font and character of a symbol character ([MS-DOC] §2.6.1); the
/// text stream holds the placeholder U+0028 at its position.
const C_SYMBOL: u16 = 0x6A09;
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
    /// The document is password-protected and no password was given.
    #[error("this document is protected with a password")]
    PasswordRequired,
    /// The password does not open the document.
    #[error("the password is incorrect")]
    WrongPassword,
    /// The document is encrypted in a way WordCraft can't read (AES, an external provider, an
    /// unknown header).
    #[error("encrypted document: {0}")]
    Encryption(String),
    /// The input exceeds a safety limit (stream size, piece count…).
    #[error("limit exceeded: {0}")]
    Limit(String),
    /// A structure is malformed beyond recovery.
    #[error("malformed document: {0}")]
    Malformed(String),
}

/// Is this `.doc` password-protected (XOR obfuscation, RC4 or RC4 CryptoAPI)? False for
/// anything that is not a readable Word 97-2003 file.
pub fn is_encrypted(bytes: &[u8]) -> bool {
    let Ok(mut comp) = open_container(bytes) else { return false };
    stream(&mut comp, "WordDocument").is_some_and(|w| fib::peek_encryption(&w).is_some())
}

/// Open the compound file around a `.doc`, after [`wordcraft_doc::cfb_guard::check`].
fn open_container(bytes: &[u8]) -> Result<cfb::CompoundFile<std::io::Cursor<&[u8]>>, DocbinError> {
    wordcraft_doc::cfb_guard::check(bytes).map_err(DocbinError::Container)?;
    cfb::CompoundFile::open(std::io::Cursor::new(bytes)).map_err(|e| DocbinError::Container(e.to_string()))
}

/// Read a `.doc` (Word 97-2003 binary) file into a [`Document`]. A password-protected file
/// gives [`DocbinError::PasswordRequired`]; see [`read_with_password`].
pub fn read(bytes: &[u8]) -> Result<Document, DocbinError> {
    read_with_password(bytes, None)
}

/// [`read`] for a file that may be password-protected: it is checked against the file's stored
/// verifier ([`DocbinError::WrongPassword`] when it does not match) and then decrypted. The
/// password is ignored for an unprotected file.
pub fn read_with_password(bytes: &[u8], password: Option<&str>) -> Result<Document, DocbinError> {
    let mut comp = open_container(bytes)?;
    let mut word = stream(&mut comp, "WordDocument").ok_or_else(|| DocbinError::NotWord("no WordDocument stream".into()))?;
    if word.len() as u64 > MAX_STREAM {
        return Err(DocbinError::Limit(format!("WordDocument stream is {} bytes", word.len())));
    }
    let enc = fib::peek_encryption(&word);
    // The FIB names the Table stream (0Table or 1Table); fall back to whichever exists,
    // since files written by other writers sometimes carry the wrong bit.
    let which = if let Some(e) = enc { e.which_tbl_stm } else { fib::Fib::parse(&word)?.which_tbl_stm };
    let named = if which { "1Table" } else { "0Table" };
    let other = if which { "0Table" } else { "1Table" };
    let mut table = stream(&mut comp, named).or_else(|| stream(&mut comp, other)).unwrap_or_default();
    if table.len() as u64 > MAX_STREAM {
        return Err(DocbinError::Limit(format!("Table stream is {} bytes", table.len())));
    }
    let mut data = stream(&mut comp, "Data").unwrap_or_default();
    if data.len() as u64 > MAX_STREAM {
        return Err(DocbinError::Limit(format!("Data stream is {} bytes", data.len())));
    }
    if let Some(e) = enc {
        // Verify the password first; only then is anything decrypted.
        let password = password.ok_or(DocbinError::PasswordRequired)?;
        let cipher = crypt::Cipher::open(e.flags, e.lkey, &table, password)?;
        cipher.decrypt_word(&mut word);
        cipher.decrypt_table(&mut table);
        cipher.decrypt_data(&mut data);
    }
    let fib = fib::Fib::parse(&word)?;

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

    let mut doc = Document::new();
    if let Some(core) = metadata::read(&mut comp) {
        doc.core = core;
    }
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
    let mut pics = media::Pictures::for_input(word.len().saturating_add(data.len()));
    // Every materialised character comes from at least one byte of the WordDocument stream, so a
    // legitimate document's total text (body plus every header, footnote and text box, each read
    // once) cannot exceed its stream. A hostile piece or story table that aliases the same bytes
    // many times can, so the walk is held to twice the stream's size — generous for real files,
    // tight against aliasing — and never past the absolute ceiling.
    let budget = std::cell::Cell::new((word.len() as u64).saturating_mul(2).min(MAX_TOTAL_CHARS));
    let work = Work::new(MAX_WORK);

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
        drawn: &[],
        sect_ends: &[],
        budget: &budget,
        work: &work,
    };

    // Drawings: text box stories become parts first, then the floating shapes and pictures
    // anchored in the main document and the headers, found by their anchor CP while walking.
    let txbx_base = edn_base.saturating_add(fib.ccp.edn);
    let hdr_txbx_base = txbx_base.saturating_add(fib.ccp.txbx);
    let art = fib
        .pair(fib::pair::DGG_INFO)
        .and_then(|(f, l)| table.get(f as usize..(f as usize).saturating_add(l as usize)))
        .map(officeart::content)
        .unwrap_or_default();
    let mut tb_stories = std::collections::HashMap::new();
    for (pair, base, len) in [(fib::pair::PLCF_TXBX_TXT, txbx_base, fib.ccp.txbx), (fib::pair::PLCF_HDR_TXBX_TXT, hdr_txbx_base, fib.ccp.hdr_txbx)] {
        for (spid, ab) in drawing::textbox_stories(&table, fib.pair(pair)) {
            let Some((a, b)) = story(base, len, ab) else { continue };
            if doc.parts.len() >= MAX_PARTS || tb_stories.contains_key(&spid) {
                continue;
            }
            let blocks = table::assemble(walk(&ctx, a, b, &mut pics));
            tb_stories.insert(spid, doc.add_part(PartKind::TextBox, blocks));
        }
    }
    let build = drawing::Build { bstore: &art.bstore, delay: &word, stories: &tb_stories };
    let mut drawn = drawing::anchors(&table, fib.pair(fib::pair::PLC_SPA_MOM), 0, &art.main, &build, &mut pics);
    drawn.extend(drawing::anchors(&table, fib.pair(fib::pair::PLC_SPA_HDR), hdd_base, &art.header, &build, &mut pics));
    drawn.sort_by_key(|(cp, _)| *cp);
    let ctx = WalkCtx { drawn: &drawn, ..ctx };

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
    let mut sect_ends: Vec<u32> = sects.iter().map(|s| s.end_cp).collect();
    sect_ends.sort_unstable();
    let ctx = WalkCtx { notes: &note_links, marks: &marks, sect_ends: &sect_ends, ..ctx };

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
    work.log();
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
    /// Floating shapes and pictures by anchor CP (absolute), sorted.
    drawn: &'a [(u32, InlineObject)],
    /// Where sections end (main-document CPs, sorted): the section mark (0x0C) just before
    /// each ends the section's last paragraph, it isn't a page break inside it.
    sect_ends: &'a [u32],
    /// Characters still allowed across every story walk of this document. Story ranges (headers,
    /// footnotes, text boxes) are hostile and can each span a whole subdocument, so thousands of
    /// them could otherwise materialise the same text thousands of times; this bounds the total.
    budget: &'a std::cell::Cell<u64>,
    /// Repeated-work bytes left ([`MAX_WORK`]), and what they left out.
    work: &'a Work,
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

    /// Append `c` to the current run, whose properties the caller knows it matches.
    fn extend(&mut self, c: char) {
        if let Some(r) = self.runs.last_mut() {
            r.len += c.len_utf8();
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
fn walk<'a>(ctx: &WalkCtx<'a>, cp_start: u32, cp_end: u32, pics: &mut media::Pictures) -> Vec<ParaOut> {
    let cp_end = cp_end.min(ctx.pieces.cp_end());
    let mut out = Vec::new();
    let mut pb = ParaBuild::default();
    let mut cp = cp_start;
    let mut marks = ctx.marks.iter().peekable();
    // Field assembly state: instruction characters are swallowed, the result is kept until
    // the end character, then emitted as one object (hyperlinks keep their result text).
    let mut field: Option<FieldBuild> = None;
    let mut memo: Option<FmtMemo<'a>> = None;
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
        // Every real character counts against the document-wide budget; once it is spent, stop
        // (a hostile file with thousands of story ranges over the same text can't multiply on).
        if ctx.budget.get() == 0 {
            break;
        }
        ctx.budget.set(ctx.budget.get() - 1);
        // A section mark (0x0C) at a section's end is that section's last paragraph mark.
        let section_mark = c == '\u{C}' && ctx.sect_ends.binary_search(&cp.saturating_add(1)).is_ok();
        match c {
            '\r' | '\u{7}' | '\u{C}' if c != '\u{C}' || section_mark => {
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
                        finish_field(&mut pb, f, ctx.work);
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
                    Some(p) => {
                        // Every anchor of a picture gets its own copy of the alt text; the
                        // copies are charged to the work budget (#3709).
                        let alt = if ctx.work.charge(p.alt.len() as u64, Cost::AltText) { p.alt.clone() } else { String::new() };
                        pb.push('\u{FFFC}', CharProps::default());
                        pb.objects.push(InlineObject::Image {
                            media: p.key.clone(),
                            w: p.w,
                            h: p.h,
                            alt,
                            float: Default::default(),
                            crop: p.crop,
                            ole: None,
                        });
                    }
                    None => pics.dropped(cp),
                }
            }
            '\u{8}' => {
                // Drawn object anchor: the floating shape or picture anchored at this CP.
                let at = ctx.drawn.partition_point(|(c, _)| *c < cp);
                // Each copy (a story can be walked more than once) is charged to the work budget
                // by what a clone copies: a picture's alt text, a group with all its members (#3710).
                for (_, obj) in ctx.drawn.get(at..).unwrap_or_default().iter().take_while(|(c, _)| *c == cp) {
                    if !ctx.work.charge(object_cost(obj), Cost::FloatingObject) {
                        continue;
                    }
                    pb.push('\u{FFFC}', CharProps::default());
                    pb.objects.push(obj.clone());
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
            '\t' => push_formatted(&mut pb, cp, '\t', ctx, field.as_mut(), &mut memo),
            '\u{B}' => push_formatted(&mut pb, cp, '\n', ctx, field.as_mut(), &mut memo),
            '\u{C}' => push_formatted(&mut pb, cp, '\u{C}', ctx, field.as_mut(), &mut memo),
            '\u{E}' => push_formatted(&mut pb, cp, '\u{E}', ctx, field.as_mut(), &mut memo),
            '\u{1E}' => push_formatted(&mut pb, cp, '\u{2011}', ctx, field.as_mut(), &mut memo),
            '\u{1F}' => push_formatted(&mut pb, cp, '\u{AD}', ctx, field.as_mut(), &mut memo),
            // Object anchors, comment references and other special characters are dropped
            // (or handled by later passes).
            c if (c as u32) < 0x20 => {}
            c => push_formatted(&mut pb, cp, c, ctx, field.as_mut(), &mut memo),
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

/// Bytes a clone of `obj` copies: the object itself plus a picture's alt text, a group with all
/// its members (#3710).
fn object_cost(obj: &InlineObject) -> u64 {
    let shallow = std::mem::size_of::<InlineObject>() as u64;
    match obj {
        InlineObject::Image { alt, .. } => shallow.saturating_add(alt.len() as u64),
        InlineObject::Group { children, .. } => children.iter().fold(shallow, |n, c| n.saturating_add(object_cost(&c.obj))),
        _ => shallow,
    }
}

/// Emit a finished field: HYPERLINK keeps its result text as a linked run; every other
/// field becomes one inline object with its cached result.
///
/// The target is copied once per run of equally formatted result text, not per character,
/// and each copy is charged to `work` ([`MAX_WORK`]); once that is spent the rest of the
/// result is kept as plain text (#3708).
fn finish_field(pb: &mut ParaBuild, f: FieldBuild, work: &Work) {
    let instr = f.instr.trim().to_string();
    let result = f.sep.unwrap_or_default();
    if let Some(target) = hyperlink_target(&instr) {
        let cost = target.len() as u64;
        let mut run: Option<CharProps> = None;
        for (c, props) in result {
            if run.as_ref() == Some(&props) {
                pb.extend(c);
                continue;
            }
            let mut linked = props.clone();
            if work.charge(cost, Cost::LinkTarget) {
                linked.link = Some(target.clone());
            }
            pb.push(c, linked);
            run = Some(props);
        }
        return;
    }
    let result_text: String = result.iter().map(|(c, _)| *c).collect();
    pb.push('\u{FFFC}', CharProps::default());
    pb.objects.push(InlineObject::Field {
        instr,
        result: result_text,
        locked: false,
        dirty: false,
        code: false,
        form: String::new(),
        code_xml: String::new(),
    });
}

/// `HYPERLINK "url"` / `HYPERLINK url \l "bookmark"` → the model link target
/// (`#bookmark` for internal links); `None` when not a hyperlink.
fn hyperlink_target(instr: &str) -> Option<String> {
    let t = instr.trim();
    let rest = if t.get(..9).is_some_and(|h| h.eq_ignore_ascii_case("hyperlink")) {
        t.get(9..).filter(|r| r.is_empty() || r.starts_with(char::is_whitespace))?.trim()
    } else {
        return None;
    };
    if rest.is_empty() {
        return None;
    }

    let mut url: Option<String> = None;
    let mut local: Option<String> = None;
    let mut switch: Option<&str> = None;

    let mut i = 0;
    let bytes = rest.as_bytes();
    while i < bytes.len() {
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() {
            break;
        }
        if bytes[i] == b'\\' {
            let start = i;
            i += 1;
            while i < bytes.len() && (bytes[i].is_ascii_alphabetic() || bytes[i] == b'*') {
                i += 1;
            }
            switch = Some(rest.get(start..i).unwrap_or(""));
            continue;
        }
        let arg = if bytes[i] == b'"' {
            i += 1;
            let start = i;
            while i < bytes.len() && bytes[i] != b'"' {
                i += 1;
            }
            let val = rest.get(start..i).unwrap_or("");
            if i < bytes.len() && bytes[i] == b'"' {
                i += 1;
            }
            val.to_string()
        } else {
            let start = i;
            while i < bytes.len() && !bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            rest.get(start..i).unwrap_or("").to_string()
        };
        let clean_arg = arg.replace("%22", "\"");
        match switch.take() {
            Some("\\l") => local = Some(clean_arg),
            Some(_) => {}
            None => {
                if url.is_none() {
                    url = Some(clean_arg);
                }
            }
        }
    }

    match (url, local) {
        (Some(u), Some(l)) if !u.is_empty() && !l.is_empty() => {
            let frag = l.trim_start_matches('#');
            if u.contains('#') { Some(u) } else { Some(format!("{u}#{frag}")) }
        }
        (Some(u), _) if !u.is_empty() => Some(u),
        (None, Some(l)) if !l.is_empty() => {
            let frag = l.trim_start_matches('#');
            Some(format!("#{frag}"))
        }
        _ => None,
    }
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

/// `sprmCSymbol` from a CHPX grpprl: the font table index and the character code of a symbol
/// character ([MS-DOC] §2.6.1, CSymbolOperand).
fn symbol_operand(chpx: &[u8]) -> Option<(u16, u16)> {
    for prl in sprm::iter(chpx) {
        if prl.op == C_SYMBOL
            && let [f0, f1, c0, c1, ..] = prl.operand
        {
            return Some((u16::from_le_bytes([*f0, *f1]), u16::from_le_bytes([*c0, *c1])));
        }
    }
    None
}

/// `sprmPIlfo` operand (a signed 16-bit value, [MS-DOC] §2.6.2) → the num id, if the
/// paragraph is in a list. 0x0001-0x07FE is a 1-based LFO index; 0xF802-0xFFFF (-2046..=-1)
/// is the negation of one and keeps the paragraph's own indents, which we do by leaving the
/// level's indents out of the paragraph (they never enter `ParaProps` anyway). 0 and 0xF801
/// mean "not in a list".
fn num_of(ilfo: i32) -> Option<u32> {
    match ilfo {
        0x0001..=0x07FE | -0x07FE..=-1 => Some(ilfo.unsigned_abs()),
        _ => None,
    }
}

/// Resolve the character properties for the character at `cp`: the CHPX grpprl of its FC,
/// plus the piece's Prm, over the paragraph style's base character props. Inside a field,
/// the character is recorded into the instruction or the cached result instead of (or as
/// well as) the visible text.
fn push_formatted<'a>(pb: &mut ParaBuild, cp: u32, c: char, ctx: &WalkCtx<'a>, field: Option<&mut FieldBuild>, memo: &mut Option<FmtMemo<'a>>) {
    let fc = ctx.pieces.fc_of_cp(cp).unwrap_or(0);
    let grpprl = ctx.chpx_bins.chpx(ctx.word, fc);
    let prm = ctx.pieces.prm_of(cp);
    let style = pb.props.style.as_deref();
    let mut props = match memo.as_ref().filter(|m| std::ptr::eq(m.chpx, grpprl) && m.prm == prm && m.style.as_deref() == style) {
        Some(m) => m.props.clone(),
        None => {
            let default;
            let base = match style.and_then(|id| ctx.sheet.get(id)) {
                Some(st) => &st.chr,
                None => {
                    default = CharProps::default();
                    &default
                }
            };
            let mut props = char_props(grpprl, ctx.fonts, base);
            apply_prm(&mut props, ctx.pieces.piece_prm(cp), ctx.fonts, base, ctx.work);
            *memo = Some(FmtMemo { chpx: grpprl, prm, style: style.map(str::to_owned), props: props.clone() });
            props
        }
    };
    // A symbol character (Insert › Symbol) is stored as the placeholder U+0028 with
    // sprmCSymbol carrying the font and the character code ([MS-DOC] §2.6.1).
    let mut c = c;
    if c == '('
        && let Some((ftc, xchar)) = symbol_operand(grpprl)
    {
        if let Some(font) = ctx.fonts.get(usize::from(ftc)) {
            props.font = Some(font.clone());
        }
        if let Some(sym) = char::from_u32(u32::from(xchar)) {
            c = sym;
        }
    }
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

/// The last character formatting resolved, reused while the CHPX, the piece's Prm and the
/// paragraph style stay the same, so a Prm grpprl (up to 64 KB) is interpreted once per
/// formatting change, not once per character (#3692).
struct FmtMemo<'a> {
    /// The CHPX grpprl, compared by address: it is a slice of the WordDocument stream.
    chpx: &'a [u8],
    prm: u16,
    style: Option<String>,
    props: CharProps,
}

/// Apply a piece's Prm (Prm0 short sprm or Prm1 grpprl) to character properties. A Prm1
/// grpprl is charged to `work`; once that is spent the grpprl is skipped.
fn apply_prm(props: &mut CharProps, prm: PrmRef<'_>, fonts: &[String], base: &CharProps, work: &Work) {
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
            if !work.charge(g.len() as u64, Cost::Formatting) {
                return;
            }
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
