//! Streaming XML scan: well-formedness, namespace-prefix resolution, illegal XML characters,
//! markup-compatibility ignorable prefixes and `mc:AlternateContent` suppression, story-part
//! reference/id collection, structural checks and the small auxiliary parsers for numbering,
//! settings and the comment-threading parts.

use std::collections::{BTreeSet, HashSet};

use quick_xml::events::attributes::Attribute;
use quick_xml::events::{BytesStart, Event};
use quick_xml::name::{Namespace, NamespaceResolver, QName, ResolveResult};
use quick_xml::reader::NsReader;

use crate::{Acc, NS_DS, NS_M, NS_MC, NS_R, NS_W, NS_W14, NS_W15, NS_W16CID, NS_WP};

/// What a part is, for the checks that apply to it.
#[derive(Clone, Copy, Default)]
pub(crate) struct PartRole {
    pub is_main: bool,
    pub is_story: bool,
    pub is_comments: bool,
    pub is_footnotes: bool,
    pub is_endnotes: bool,
}

/// Ids collected from the story parts, pooled document-wide.
#[derive(Default)]
pub(crate) struct StoryFacts {
    pub comment_defs: Vec<(String, String)>,
    pub comment_refs: Vec<(String, String)>,
    /// `w14:paraId` values on paragraphs in comments.xml (for `COMMENTEX-LINK`).
    pub comment_para_ids: Vec<(String, String)>,
    pub docpr: Vec<(String, String)>,
    pub bookmarks: Vec<(String, String)>,
    pub revisions: Vec<(String, String)>,
    pub footnote_refs: Vec<(String, String)>,
    pub endnote_refs: Vec<(String, String)>,
    pub footnote_defs: Vec<(String, String)>,
    pub endnote_defs: Vec<(String, String)>,
    pub header_refs: Vec<(String, String)>,
    pub footer_refs: Vec<(String, String)>,
    pub style_refs: Vec<(String, String)>,
    /// `w:numId/@w:val` values used by `w:numPr` in story parts (for `NUM-DANGLING`).
    pub num_ids: Vec<(String, String)>,
}

/// One `r:id`/`r:embed`/`r:link` value found in a part, with the element identity needed to
/// verify typed references (an image element must resolve to an image relationship).
pub(crate) struct RefFact {
    pub attr: String,
    pub value: String,
    /// The reference is the image target of an image element (`a:blip`, `v:imagedata`).
    pub image: bool,
}

/// Pool another part's story facts into the document-wide collection, honouring the per-category
/// budget. Returns `true` when the budget stopped a category from taking more.
pub(crate) fn merge_facts(into: &mut StoryFacts, from: StoryFacts, cap: usize) -> bool {
    let mut capped = false;
    macro_rules! pool {
        ($field:ident) => {{
            for item in from.$field {
                if into.$field.len() >= cap {
                    capped = true;
                    break;
                }
                into.$field.push(item);
            }
        }};
    }
    pool!(comment_defs);
    pool!(comment_refs);
    pool!(comment_para_ids);
    pool!(docpr);
    pool!(bookmarks);
    pool!(revisions);
    pool!(footnote_refs);
    pool!(endnote_refs);
    pool!(footnote_defs);
    pool!(endnote_defs);
    pool!(header_refs);
    pool!(footer_refs);
    pool!(style_refs);
    pool!(num_ids);
    capped
}

/// The result of scanning one XML or `.rels` part.
pub(crate) struct PartScan {
    pub malformed: Option<String>,
    pub unknown_prefixes: Vec<String>,
    /// Every `r:id`/`r:embed`/`r:link` value in the part, with element identity.
    pub refs: Vec<RefFact>,
    pub facts: StoryFacts,
}

struct ElemId {
    ns: Option<String>,
    local: String,
    unknown: Option<String>,
}

impl ElemId {
    /// Whether this element belongs to the WordprocessingML main namespace (`w:`).
    fn w(&self) -> bool {
        self.local_w(NS_W)
    }
    /// Whether this element belongs to the drawing namespace (`wp:`).
    fn wp(&self) -> bool {
        self.local_w(NS_WP)
    }
    /// Whether this element belongs to the Office Math namespace (`m:`).
    fn m(&self) -> bool {
        self.local_w(NS_M)
    }
    /// Whether this element's namespace URI matches `uri`.
    fn local_w(&self, uri: &str) -> bool {
        self.ns.as_deref() == Some(uri)
    }
}

/// Which budget a [`ParseGuard`] exhausted (J1), so a coverage note can name the mechanism.
#[derive(Clone, Copy)]
enum Exhaustion {
    Events,
    Depth,
}

/// A shared event/depth bound for the auxiliary parsers (H1). The main scan bounds itself inline,
/// but every auxiliary parser re-reads its part, so each must be bounded too. Exhaustion (event OR
/// depth) is terminal: [`tick`] stops the loop before another read and [`enter`] refuses the
/// element that crosses the depth limit (the caller must not process it). Once `exhausted` is set
/// the partial result must not be trusted: the caller records a part-specific note, yields
/// INCOMPLETE and skips any check that depended on the incomplete definitions.
pub(crate) struct ParseGuard {
    max_events: usize,
    max_depth: usize,
    events: usize,
    depth: usize,
    exhausted: bool,
    reason: Option<Exhaustion>,
}

impl ParseGuard {
    /// Create a new parse guard with the given event and depth limits.
    pub(crate) fn new(max_events: usize, max_depth: usize) -> ParseGuard {
        ParseGuard { max_events, max_depth, events: 0, depth: 0, exhausted: false, reason: None }
    }

    /// Count one event; returns `false` once a budget is spent, before another event is read.
    pub(crate) fn tick(&mut self) -> bool {
        if self.exhausted {
            return false;
        }
        if self.events >= self.max_events {
            self.exhausted = true;
            self.reason = Some(Exhaustion::Events);
            return false;
        }
        self.events += 1;
        true
    }

    /// Enter an element. Returns `false` when this element crosses the depth limit; the caller must
    /// stop before reading another event and must not process the crossing element (J1).
    pub(crate) fn enter(&mut self) -> bool {
        self.depth += 1;
        if self.depth > self.max_depth {
            self.exhausted = true;
            if self.reason.is_none() {
                self.reason = Some(Exhaustion::Depth);
            }
            return false;
        }
        true
    }

    /// Leave an element, decrementing current depth.
    pub(crate) fn leave(&mut self) {
        self.depth = self.depth.saturating_sub(1);
    }

    /// Whether any resource budget (event count or recursion depth) has been exhausted.
    pub(crate) fn exhausted(&self) -> bool {
        self.exhausted
    }

    /// Which budget was exhausted, for a coverage note: `"event"`, `"depth"`, or `"event or
    /// depth"` when the reason was not recorded.
    pub(crate) fn budget(&self) -> &'static str {
        match self.reason {
            Some(Exhaustion::Events) => "event",
            Some(Exhaustion::Depth) => "depth",
            None => "event or depth",
        }
    }
}

/// Track `mc:AlternateContent` nesting for an auxiliary parser so it skips skipped-branch content
/// exactly as the main scan does (H3). Without this, a definition or reference inside a branch the
/// main scan reported as skipped would still be collected, producing findings about content that is
/// not read. Suppression is namespace-aware (J5): an open frame per element remembers whether that
/// element was an `mc:AlternateContent`, so a foreign-namespace element merely *named*
/// `AlternateContent` cannot unbalance the suppression depth on close.
struct AltSuppress {
    depth: usize,
    /// One entry per open `Start` element: whether it was an `mc:AlternateContent` frame.
    frames: Vec<bool>,
}

impl AltSuppress {
    /// Create a new alternate content suppression tracker.
    fn new() -> AltSuppress {
        AltSuppress { depth: 0, frames: Vec::new() }
    }

    /// Handle an open tag. Returns `true` while inside a subtree that must be skipped; an
    /// `mc:AlternateContent` open tag itself is skipped.
    fn open(&mut self, id: &ElemId, empty: bool) -> bool {
        let is_alt = is_alternate_content(id);
        if is_alt {
            if !empty {
                self.depth += 1;
                self.frames.push(true);
            }
            return true;
        }
        if !empty {
            self.frames.push(false);
        }
        self.depth > 0
    }

    /// Handle a close tag. The close event carries no resolved namespace, so the matching is done
    /// through the per-element frame pushed on open (J5): only an `mc:AlternateContent` frame
    /// decrements the suppression depth.
    fn close(&mut self, _local: &[u8]) {
        if self.frames.pop().unwrap_or(false) {
            self.depth = self.depth.saturating_sub(1);
        }
    }
}

/// Check whether an element identifier corresponds to `mc:AlternateContent`.
fn is_alternate_content(id: &ElemId) -> bool {
    id.ns.as_deref() == Some(NS_MC) && id.local == "AlternateContent"
}

/// The last observed direct block child of a table cell.
#[derive(Clone, Copy, PartialEq, Eq)]
enum TcEnd {
    /// A paragraph satisfies the cell's terminal-block requirement.
    Paragraph,
    /// A nested table: the one non-paragraph ending calibration shows Word rejecting.
    Other,
}

struct Frame {
    is_tc: bool,
    /// A `w:p` frame: math placement is judged against paragraph ancestry.
    is_p: bool,
    /// Any Office Math element: an `m:oMathPara` must not descend from one.
    is_math: bool,
    /// An `m:oMath` frame: math content and nested `m:oMath` are judged against it.
    is_omath: bool,
    /// An `m:oMathPara` frame: a legal container whose non-`oMath` math children are
    /// still math content outside any `m:oMath`.
    is_omath_para: bool,
    /// An `m:oMathParaPr` frame: paragraph properties, a legal child of `m:oMathPara`
    /// whose own children (`m:jc` and friends) are not stray math content.
    is_omath_parapr: bool,
    /// This frame's element is one of the block containers `m:oMath` and
    /// `m:oMathPara` must not be a direct child of (MS-OI29500 §22.1.2.77 (f) and
    /// §22.1.2.78 (c)); the value is the element's local name for messages. Only
    /// the inherently block-level names are here: `customXml` and `sdtContent`
    /// are listed at block, cell, and row level only, so they are judged by
    /// context instead (see `wrapper_is_block_level`).
    block_parent_name: Option<&'static str>,
    /// A `w:sdtContent` frame: a prohibited direct parent only at block, cell, or
    /// row level — a run-level `sdtContent` inside a paragraph is not listed.
    is_sdt_content: bool,
    /// A `w:customXml` frame: likewise a prohibited direct parent only at block,
    /// cell, or row level — a run-level `customXml` inside a paragraph is not.
    is_custom_xml: bool,
    tc_has_block: bool,
    /// None also represents an unknown last block in skipped AlternateContent.
    tc_last_block: Option<TcEnd>,
    is_del: bool,
    is_wrapper: bool,
    /// This frame is an `mc:AlternateContent` element.
    alt: bool,
    /// The element was opened inside a skipped `mc:AlternateContent` subtree, so its closing event
    /// must not touch the frame counters or report structural findings.
    suppressed: bool,
}

#[derive(Default)]
struct State {
    declared: Vec<Vec<String>>,
    frames: Vec<Frame>,
    del_depth: i64,
    wrapper_depth: i64,
    fld_depth: i64,
    fld_extra_end: usize,
    move_from_start: BTreeSet<String>,
    move_from_end: BTreeSet<String>,
    move_to_start: BTreeSet<String>,
    move_to_end: BTreeSet<String>,
    move_from_names: BTreeSet<String>,
    move_to_names: BTreeSet<String>,
    move_start_ids: Vec<String>,
    bookmark_starts: BTreeSet<String>,
    bookmark_ends: BTreeSet<String>,
    /// Nonzero while inside an `mc:AlternateContent` subtree.
    alt_depth: usize,
    /// One coverage note per part that contains `mc:AlternateContent`.
    alt_noted: bool,
    /// The per-category fact budget, copied from [`crate::Limits`].
    fact_cap: usize,
    /// A fact budget was exhausted; no further facts are collected.
    fact_capped: bool,
    /// The reference budget was exhausted; no further `r:id`/`r:embed`/`r:link` values are kept.
    refs_capped: bool,
    facts: StoryFacts,
}

/// Why a part could not be decoded to text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum DecodeError {
    /// The bytes are not valid text in the declared encoding (invalid UTF-8, an unpaired surrogate,
    /// or a truncated trailing unit): the part is not well-formed XML.
    Malformed(String),
    /// The part declares an encoding this checker does not support; its checks are skipped.
    Unsupported(String),
    /// The part begins an XML declaration that does not terminate within the scan bound, so the
    /// encoding cannot be decided (H6); the part's checks are skipped and the run is INCOMPLETE.
    IncompleteDeclaration,
}

/// The encoding a part declares (or its shape when it has no declaration).
enum Encoding {
    Utf8,
    Utf16Le,
    Utf16Be,
    Other(String),
}

/// Decode a part to text once, strictly: honour a byte-order mark, then the XML declaration
/// (`UTF-8` / `UTF-16LE` / `UTF-16BE`), defaulting to UTF-8. Invalid UTF-8, an unpaired surrogate or
/// a truncated trailing unit is a decode failure, never a lossy `U+FFFD` substitution. A part
/// declaring an unsupported encoding is reported as such; every consumer reads the one decoded
/// representation.
pub(crate) fn decode_strict(bytes: &[u8]) -> Result<String, DecodeError> {
    let (encoding, rest) = if let Some(rest) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        (Encoding::Utf8, rest)
    } else if let Some(rest) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        (Encoding::Utf16Le, rest)
    } else if let Some(rest) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        (Encoding::Utf16Be, rest)
    } else if bytes.starts_with(&[0x00, b'<']) {
        (Encoding::Utf16Be, bytes)
    } else if bytes.starts_with(&[b'<', 0x00]) {
        (Encoding::Utf16Le, bytes)
    } else {
        match declared_encoding(bytes) {
            Declared::Unterminated => return Err(DecodeError::IncompleteDeclaration),
            Declared::None => (Encoding::Utf8, bytes),
            Declared::Encoding(enc) => match enc.trim().to_ascii_lowercase().as_str() {
                "utf-8" | "utf8" => (Encoding::Utf8, bytes),
                "utf-16le" => (Encoding::Utf16Le, bytes),
                "utf-16be" => (Encoding::Utf16Be, bytes),
                "utf-16" if bytes.starts_with(&[b'<', 0x00]) => (Encoding::Utf16Le, bytes),
                "utf-16" => (Encoding::Utf16Be, bytes),
                other => (Encoding::Other(other.to_string()), bytes),
            },
        }
    };
    match encoding {
        Encoding::Utf8 => std::str::from_utf8(rest).map(str::to_owned).map_err(|e| DecodeError::Malformed(format!("invalid UTF-8: {e}"))),
        Encoding::Utf16Le => decode_utf16_strict(rest, true),
        Encoding::Utf16Be => decode_utf16_strict(rest, false),
        Encoding::Other(enc) => Err(DecodeError::Unsupported(enc)),
    }
}

/// The result of scanning the XML declaration for an encoding.
enum Declared {
    /// No XML declaration is present.
    None,
    /// The declaration names this encoding.
    Encoding(String),
    /// A declaration began but did not terminate within the scan bound (H6).
    Unterminated,
}

/// How far the XML declaration is scanned before a declaration is treated as unterminated (H6).
/// The part itself is already bounded by the per-part byte limit; this bound keeps a part whose
/// declaration is pathological from being read as UTF-8 by guessing.
const DECLARATION_SCAN_BOUND: usize = 1 << 20;

/// Locate the byte offset of `needle` within `hay`.
fn find_bytes(hay: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || hay.len() < needle.len() {
        return None;
    }
    hay.windows(needle.len()).position(|w| w == needle)
}

/// The `encoding` pseudo-attribute of an XML declaration, if the bytes begin with one. The header
/// is ASCII in every encoding we accept, so a lossy read of the declaration slice is safe for
/// detection. The declaration is scanned in full within [`DECLARATION_SCAN_BOUND`], not just its
/// first 512 bytes (H6).
fn declared_encoding(bytes: &[u8]) -> Declared {
    let scan = &bytes[..bytes.len().min(DECLARATION_SCAN_BOUND)];
    let Some(end) = find_bytes(scan, b"?>") else {
        // No closing marker within the bound. If the input extends past the bound and looks like it
        // began a declaration, it is unterminated: do not fall back to guessing UTF-8.
        if bytes.len() > DECLARATION_SCAN_BOUND && find_bytes(scan, b"<?xml").is_some() {
            return Declared::Unterminated;
        }
        return Declared::None;
    };
    let decl = String::from_utf8_lossy(&scan[..end]);
    if !decl.trim_start().starts_with("<?xml") {
        return Declared::None;
    }
    let Some(idx) = decl.find("encoding") else { return Declared::None };
    let after = decl[idx + "encoding".len()..].trim_start();
    let Some(after) = after.strip_prefix('=') else { return Declared::None };
    let after = after.trim_start();
    let Some(quote) = after.chars().next() else { return Declared::None };
    if quote != '"' && quote != '\'' {
        return Declared::None;
    }
    let rest = &after[quote.len_utf8()..];
    let Some(end) = rest.find(quote) else { return Declared::None };
    Declared::Encoding(rest[..end].to_string())
}

/// Decode strict UTF-16 (little-endian or big-endian) into a String, rejecting truncated units or unpaired surrogates.
fn decode_utf16_strict(bytes: &[u8], little_endian: bool) -> Result<String, DecodeError> {
    if !bytes.len().is_multiple_of(2) {
        return Err(DecodeError::Malformed("a UTF-16 part ends with a truncated unit".to_string()));
    }
    let units: Vec<u16> =
        bytes.as_chunks::<2>().0.iter().map(|p| if little_endian { u16::from_le_bytes(*p) } else { u16::from_be_bytes(*p) }).collect();
    String::from_utf16(&units).map_err(|_| DecodeError::Malformed("a UTF-16 part contains an unpaired surrogate".to_string()))
}

/// Scan one part. Well-formedness, illegal characters, namespace findings, structural findings and
/// `XML-INVALID-CHAR` reference findings are added to `acc`; the collected story facts and every
/// `r:id`/`r:embed`/`r:link` value come back on the `PartScan`.
pub(crate) fn scan_part(part: &str, text: &str, role: PartRole, acc: &mut Acc) -> PartScan {
    let mut reader = NsReader::from_reader(text.as_bytes());
    let mut buf = Vec::new();
    let mut state = State { fact_cap: acc.limits.facts, ..State::default() };
    let mut out = PartScan { malformed: None, unknown_prefixes: Vec::new(), refs: Vec::new(), facts: StoryFacts::default() };
    let mut root_count = 0usize;
    let mut events = 0usize;
    let mut budget_stopped = false;
    loop {
        if events >= acc.limits.events {
            acc.note(format!("{part} has more than {} XML events; the rest of the part was not checked", acc.limits.events));
            acc.set_incomplete();
            budget_stopped = true;
            break;
        }
        events += 1;
        match reader.read_event_into(&mut buf) {
            Err(e) => {
                out.malformed = Some(e.to_string());
                break;
            }
            Ok(Event::Eof) => break,
            Ok(Event::Start(e)) => {
                if state.frames.is_empty() {
                    if root_count >= 1 {
                        out.malformed = Some("more than one document element".to_string());
                        break;
                    }
                    root_count += 1;
                }
                if state.frames.len() >= acc.limits.depth {
                    acc.note(format!("{part} nests deeper than {} elements; the rest of the part was not checked", acc.limits.depth));
                    acc.set_incomplete();
                    budget_stopped = true;
                    break;
                }
                let id = resolve_elem(reader.resolver(), e.name());
                if let Some(p) = &id.unknown {
                    out.unknown_prefixes.push(p.clone());
                }
                let Some(attrs) = open_attributes(&e, &mut out) else { break };
                scan_attr_refs(part, &attrs, acc);
                let was_suppressed = state.alt_depth > 0;
                let is_alt = enter_alt(&mut state, &id, part, acc);
                let decls = declarations(&attrs);
                state.declared.push(decls);
                check_prefixes(&state.declared, &attrs, reader.resolver(), &[], &mut out);
                if !was_suppressed {
                    collect_refs(&id, &attrs, reader.resolver(), &mut out.refs, acc.limits.facts, &mut state.refs_capped);
                    on_open(&mut state, &id, &attrs, reader.resolver(), role, part, acc);
                }
                let frame = Frame { alt: is_alt, suppressed: was_suppressed, ..frame_for(&id) };
                state.frames.push(frame);
            }
            Ok(Event::Empty(e)) => {
                if state.frames.is_empty() {
                    if root_count >= 1 {
                        out.malformed = Some("more than one document element".to_string());
                        break;
                    }
                    root_count += 1;
                }
                let id = resolve_elem(reader.resolver(), e.name());
                if let Some(p) = &id.unknown {
                    out.unknown_prefixes.push(p.clone());
                }
                let Some(attrs) = open_attributes(&e, &mut out) else { break };
                scan_attr_refs(part, &attrs, acc);
                let was_suppressed = state.alt_depth > 0;
                let is_alt = enter_alt(&mut state, &id, part, acc);
                let decls = declarations(&attrs);
                check_prefixes(&state.declared, &attrs, reader.resolver(), &decls, &mut out);
                if !was_suppressed {
                    collect_refs(&id, &attrs, reader.resolver(), &mut out.refs, acc.limits.facts, &mut state.refs_capped);
                    on_empty(&mut state, &id, &attrs, reader.resolver(), role, part, acc);
                }
                if is_alt {
                    state.alt_depth = state.alt_depth.saturating_sub(1);
                }
            }
            Ok(Event::End(_)) => {
                state.declared.pop();
                if let Some(f) = state.frames.pop() {
                    if f.alt {
                        state.alt_depth = state.alt_depth.saturating_sub(1);
                    }
                    if !f.suppressed {
                        if f.is_del {
                            state.del_depth -= 1;
                        }
                        if f.is_wrapper {
                            state.wrapper_depth -= 1;
                        }
                        close_cell(&f, part, role, acc);
                    }
                }
            }
            Ok(Event::Text(e)) => {
                if state.frames.is_empty() && !is_whitespace(&e) {
                    let msg = if root_count == 0 { "text before the document element" } else { "content after the document element" };
                    out.malformed = Some(msg.to_string());
                    break;
                }
            }
            Ok(Event::GeneralRef(e)) => {
                if state.frames.is_empty() {
                    let msg = if root_count == 0 { "a reference before the document element" } else { "content after the document element" };
                    out.malformed = Some(msg.to_string());
                    break;
                }
                scan_general_ref(part, e.as_ref(), acc);
            }
            Ok(Event::CData(_)) if state.frames.is_empty() => {
                out.malformed = Some("CDATA outside the document element".to_string());
                break;
            }
            Ok(_) => {}
        }
        buf.clear();
    }
    if out.malformed.is_none() && !budget_stopped {
        if !state.frames.is_empty() {
            out.malformed = Some("unclosed element at end of part".to_string());
        } else if root_count == 0 {
            out.malformed = Some("the part has no document element".to_string());
        }
    }
    // Pairing reports split into two classes. PREFIX-ESTABLISHED reports are true of the scanned
    // prefix alone (monotone), so they are emitted on every scan. COMPLETION-DEPENDENT reports need
    // the whole part; emitting them from a truncated state would invent findings the rest of the
    // part might contradict. They are emitted only on a complete scan. A budget/cap stop already
    // marked INCOMPLETE at its own site; on the malformed path, a discarded completion-dependent set
    // that would have reported something marks the run INCOMPLETE below (the malformed part itself
    // is Warning-only by calibration).
    let scan_complete = out.malformed.is_none() && !budget_stopped && !state.fact_capped && !state.refs_capped;
    finish_prefix(&state, part, acc);
    if scan_complete {
        finish_completion(&state, part, acc);
    } else if out.malformed.is_some() && completion_dependent_pending(&state) {
        // No INCOMPLETE is marked by a malformed part itself, but here a completion-dependent
        // pairing check could not run and would have reported something: the run could not examine
        // everything a finding depends on. Name the part and the check that could not complete.
        acc.note(format!("{part} ends before its field/move/bookmark pairing checks could complete; they were not run"));
        acc.set_incomplete();
    }
    if state.fact_capped || state.refs_capped {
        acc.note(format!("{part} exceeded the {}-entry fact/reference budget; later facts or references were not recorded", acc.limits.facts));
        acc.set_incomplete();
    }
    out.facts = state.facts;
    out.unknown_prefixes.sort_unstable();
    out.unknown_prefixes.dedup();
    out
}

/// Collect an element's attributes, turning an iterator error (a duplicate or malformed attribute)
/// into a malformed-part message rather than silently dropping it.
fn open_attributes<'a>(e: &'a BytesStart<'_>, out: &mut PartScan) -> Option<Vec<Attribute<'a>>> {
    let mut attrs = Vec::new();
    for a in e.attributes() {
        match a {
            Ok(a) => attrs.push(a),
            Err(err) => {
                out.malformed = Some(format!("malformed attribute: {err}"));
                return None;
            }
        }
    }
    Some(attrs)
}

/// Handle an `mc:AlternateContent` opening tag: bump the suppression depth, note it once per part,
/// and stop the immediately enclosing `w:tc` from being reported empty (its block content is in a
/// branch we deliberately do not read).
fn enter_alt(state: &mut State, id: &ElemId, part: &str, acc: &mut Acc) -> bool {
    let is_alt = id.ns.as_deref() == Some(NS_MC) && id.local == "AlternateContent";
    if is_alt {
        state.alt_depth += 1;
        if !state.alt_noted {
            acc.note(format!("{part} contains mc:AlternateContent; its contents were skipped"));
            state.alt_noted = true;
        }
        if let Some(f) = state.frames.last_mut()
            && f.is_tc
        {
            f.tc_has_block = true;
            // The skipped branch may end in a paragraph; do not infer its terminal kind.
            f.tc_last_block = None;
        }
    }
    is_alt
}

/// Construct a stack frame for the element indicating whether it is a table cell, deletion, or revision wrapper.
fn frame_for(id: &ElemId) -> Frame {
    let w = id.w();
    let local = id.local.as_str();
    Frame {
        is_tc: w && local == "tc",
        is_p: w && local == "p",
        is_math: id.m(),
        is_omath: id.m() && local == "oMath",
        is_omath_para: id.m() && local == "oMathPara",
        is_omath_parapr: id.m() && local == "oMathParaPr",
        block_parent_name: if w {
            match local {
                "body" => Some("body"),
                "comment" => Some("comment"),
                "docPartBody" => Some("docPartBody"),
                "endnote" => Some("endnote"),
                "footnote" => Some("footnote"),
                "ftr" => Some("ftr"),
                "hdr" => Some("hdr"),
                "tbl" => Some("tbl"),
                "tc" => Some("tc"),
                "tr" => Some("tr"),
                "txbxContent" => Some("txbxContent"),
                _ => None,
            }
        } else {
            None
        },
        is_sdt_content: w && local == "sdtContent",
        is_custom_xml: w && local == "customXml",
        tc_has_block: false,
        tc_last_block: None,
        is_del: w && local == "del",
        is_wrapper: w && matches!(local, "ins" | "del" | "moveFrom" | "moveTo"),
        alt: false,
        suppressed: false,
    }
}

/// Evaluate a closing table-cell frame in the main part: a cell with no block content is
/// `STRUCT-TC-NO-P`; one whose last block is not a paragraph is `STRUCT-END-P`. A closed
/// cell's last block is known even if later XML is malformed or truncated, so it is
/// evaluated here; an open cell never is, because later content could repair it.
fn close_cell(f: &Frame, part: &str, role: PartRole, acc: &mut Acc) {
    if !f.is_tc || !role.is_main {
        return;
    }
    if f.tc_has_block && f.tc_last_block == Some(TcEnd::Other) {
        acc.add("STRUCT-END-P", Some(part.to_string()), format!("w:tc in {part} does not end with a paragraph"));
    }
    if !f.tc_has_block {
        acc.add("STRUCT-TC-NO-P", Some(part.to_string()), format!("w:tc in {part} has no block-level content"));
    }
}

/// A block-level child satisfies its enclosing `w:tc` (ECMA-376 Part 1 §17.4.65: a cell with no
/// block content is corrupt). Only the immediately enclosing frame is checked, so a run-level
/// `w:sdt` inside a `w:p` does not satisfy the cell. Also record the terminal block kind.
fn mark_block_content(state: &mut State, id: &ElemId) {
    if !id.w() || !matches!(id.local.as_str(), "p" | "tbl" | "sdt" | "customXml" | "altChunk") {
        return;
    }
    if let Some(f) = state.frames.last_mut()
        && f.is_tc
    {
        f.tc_has_block = true;
        // Only a nested table is a *known* non-paragraph ending (calibration fixture 32).
        // The wrapper kinds (sdt, customXml, altChunk) can hold a terminal paragraph the
        // scanner does not descend into, so they leave the terminal kind unknown — the
        // same treatment a skipped AlternateContent branch gets in enter_alt.
        f.tc_last_block = match id.local.as_str() {
            "p" => Some(TcEnd::Paragraph),
            "tbl" => Some(TcEnd::Other),
            _ => None,
        };
    }
}

/// Whether the wrapper frame at `idx` (a `w:sdtContent` or `w:customXml`) sits
/// at block level: walking outward past it, an inherently block-level container
/// is met before any `w:p` (or no `w:p` is met at all). Wrapper frames
/// themselves are transparent to the walk — a run-level wrapper inside a
/// paragraph does not make a nested wrapper block-level. MS-OI29500
/// §22.1.2.77 (f) / §22.1.2.78 (c) list these parents at block, cell, and row
/// level only.
fn wrapper_is_block_level(frames: &[Frame], idx: usize) -> bool {
    for f in frames[..idx].iter().rev() {
        if f.is_p {
            return false;
        }
        if f.block_parent_name.is_some() {
            return true;
        }
    }
    true
}

/// The prohibited direct parent of an `m:oMath` or `m:oMathPara`, when the
/// innermost open frame is one: an inherently block-level container, or a
/// `w:sdtContent` / `w:customXml` sitting at block level. Returns the parent's
/// local name for the finding message.
fn prohibited_parent(state: &State) -> Option<&'static str> {
    let parent = state.frames.last()?;
    if let Some(name) = parent.block_parent_name {
        return Some(name);
    }
    let idx = state.frames.len() - 1;
    if parent.is_sdt_content && wrapper_is_block_level(&state.frames, idx) {
        return Some("sdtContent");
    }
    if parent.is_custom_xml && wrapper_is_block_level(&state.frames, idx) {
        return Some("customXml");
    }
    None
}

/// `MATH-PLACEMENT` (MS-OI29500 Part 1 §22.1.2.77 notes (b), (d), (e), (f) and
/// §22.1.2.78 notes (b), (c), (d)): judge a math element against its ancestors,
/// which are exactly the frames already open — this runs before the element's own
/// frame is pushed. Story parts only: the documented refusals concern document
/// content. A stray math island (math content with no `m:oMath` anywhere above it)
/// is reported once, at its root element, not once per descendant.
fn check_math_placement(state: &State, id: &ElemId, role: PartRole, part: &str, acc: &mut Acc) {
    if !role.is_story || !id.m() {
        return;
    }
    let in_omath = state.frames.iter().any(|f| f.is_omath);
    let in_math = state.frames.iter().any(|f| f.is_math);
    let in_stray = state.frames.iter().any(|f| f.is_math && !f.is_omath && !f.is_omath_para);
    let in_omath_para = state.frames.iter().any(|f| f.is_omath_para);
    let in_parapr = state.frames.iter().any(|f| f.is_omath_parapr);
    let in_p = state.frames.iter().any(|f| f.is_p);
    match id.local.as_str() {
        "oMath" => {
            if in_omath {
                acc.add("MATH-PLACEMENT", Some(part.to_string()), format!("m:oMath in {part} is nested inside another m:oMath"));
            }
            if !in_p {
                acc.add("MATH-PLACEMENT", Some(part.to_string()), format!("m:oMath in {part} occurs outside any w:p"));
            }
            if let Some(parent) = prohibited_parent(state) {
                acc.add("MATH-PLACEMENT", Some(part.to_string()), format!("m:oMath in {part} is a direct child of w:{parent}"));
            }
        }
        "oMathPara" => {
            if in_math {
                acc.add("MATH-PLACEMENT", Some(part.to_string()), format!("m:oMathPara in {part} is a descendant of a math element"));
            }
            if !in_p {
                acc.add("MATH-PLACEMENT", Some(part.to_string()), format!("m:oMathPara in {part} occurs outside any w:p"));
            }
            if let Some(parent) = prohibited_parent(state) {
                acc.add("MATH-PLACEMENT", Some(part.to_string()), format!("m:oMathPara in {part} is a direct child of w:{parent}"));
            }
        }
        // Paragraph properties are a legal child of oMathPara, not stray content.
        "oMathParaPr" if in_omath_para => {}
        _ if !in_omath && !in_stray && !in_parapr => {
            acc.add("MATH-PLACEMENT", Some(part.to_string()), format!("m:{} in {part} is math content outside any m:oMath", id.local));
        }
        _ => {}
    }
}

/// Handle element start tags: update structural flags, deletion depth, IDs, and element structure.
fn on_open(state: &mut State, id: &ElemId, attrs: &[Attribute], res: &NamespaceResolver, role: PartRole, part: &str, acc: &mut Acc) {
    mark_block_content(state, id);
    check_math_placement(state, id, role, part, acc);
    if id.w() && id.local == "del" {
        state.del_depth += 1;
    }
    record_ids(state, id, attrs, res, role, part);
    record_structure(state, id, attrs, res, part, acc, false);
}

/// Handle self-closing/empty tags: check structural requirements like non-empty table cells, and record IDs.
fn on_empty(state: &mut State, id: &ElemId, attrs: &[Attribute], res: &NamespaceResolver, role: PartRole, part: &str, acc: &mut Acc) {
    mark_block_content(state, id);
    check_math_placement(state, id, role, part, acc);
    if id.w() && id.local == "tc" && role.is_main {
        acc.add("STRUCT-TC-NO-P", Some(part.to_string()), format!("w:tc in {part} has no block-level content"));
    }
    record_ids(state, id, attrs, res, role, part);
    record_structure(state, id, attrs, res, part, acc, true);
}

/// Record defined and referenced IDs across story parts (comments, footnotes, endnotes, bookmarks, revisions).
fn record_ids(state: &mut State, id: &ElemId, attrs: &[Attribute], res: &NamespaceResolver, role: PartRole, part: &str) {
    if !role.is_story {
        return;
    }
    record_comment_ids(state, id, attrs, res, role, part);
    record_note_ids(state, id, attrs, res, role, part);
    record_bookmark_ids(state, id, attrs, res, part);
    record_revision_ids(state, id, attrs, res, part);
    record_reference_ids(state, id, attrs, res, role, part);
    if id.wp() && id.local == "docPr" {
        let cap = state.fact_cap;
        push_capped(&mut state.facts.docpr, cap, &mut state.fact_capped, attr_plain(attrs, b"id"), part);
    }
}

/// Record comment definitions, references, and paragraph IDs within comments parts.
fn record_comment_ids(state: &mut State, id: &ElemId, attrs: &[Attribute], res: &NamespaceResolver, role: PartRole, part: &str) {
    let cap = state.fact_cap;
    match id.local.as_str() {
        "comment" if id.w() && role.is_comments => {
            push_capped(&mut state.facts.comment_defs, cap, &mut state.fact_capped, attr_ns(attrs, res, NS_W, b"id"), part);
        }
        "commentReference" if id.w() => {
            push_capped(&mut state.facts.comment_refs, cap, &mut state.fact_capped, attr_ns(attrs, res, NS_W, b"id"), part);
        }
        "p" if id.w() && role.is_comments => {
            push_capped(&mut state.facts.comment_para_ids, cap, &mut state.fact_capped, attr_ns(attrs, res, NS_W14, b"paraId").map(hex_ident), part);
        }
        _ => {}
    }
}

/// Record footnote and endnote definitions and references.
fn record_note_ids(state: &mut State, id: &ElemId, attrs: &[Attribute], res: &NamespaceResolver, role: PartRole, part: &str) {
    let cap = state.fact_cap;
    match id.local.as_str() {
        "footnoteReference" if id.w() => {
            push_capped(&mut state.facts.footnote_refs, cap, &mut state.fact_capped, attr_ns(attrs, res, NS_W, b"id"), part);
        }
        "endnoteReference" if id.w() => {
            push_capped(&mut state.facts.endnote_refs, cap, &mut state.fact_capped, attr_ns(attrs, res, NS_W, b"id"), part);
        }
        "footnote" if id.w() && role.is_footnotes => {
            push_capped(&mut state.facts.footnote_defs, cap, &mut state.fact_capped, attr_ns(attrs, res, NS_W, b"id"), part);
        }
        "endnote" if id.w() && role.is_endnotes => {
            push_capped(&mut state.facts.endnote_defs, cap, &mut state.fact_capped, attr_ns(attrs, res, NS_W, b"id"), part);
        }
        _ => {}
    }
}

/// Record bookmark start and end tags, tracking paired IDs up to the fact budget.
fn record_bookmark_ids(state: &mut State, id: &ElemId, attrs: &[Attribute], res: &NamespaceResolver, part: &str) {
    match id.local.as_str() {
        "bookmarkStart" if id.w() => {
            if state.bookmark_starts.len() >= state.fact_cap {
                state.fact_capped = true;
                return;
            }
            if let Some(x) = attr_ns(attrs, res, NS_W, b"id") {
                state.bookmark_starts.insert(x.clone());
                push_capped(&mut state.facts.bookmarks, state.fact_cap, &mut state.fact_capped, Some(x), part);
            }
        }
        "bookmarkEnd" if id.w() => {
            if let Some(x) = attr_ns(attrs, res, NS_W, b"id") {
                if state.bookmark_ends.len() >= state.fact_cap {
                    state.fact_capped = true;
                } else {
                    state.bookmark_ends.insert(x);
                }
            }
        }
        _ => {}
    }
}

/// Record revision IDs on change-tracking elements (`ins`, `del`, `moveFrom`, `moveTo`, etc.).
fn record_revision_ids(state: &mut State, id: &ElemId, attrs: &[Attribute], res: &NamespaceResolver, part: &str) {
    let is_revision =
        matches!(id.local.as_str(), "ins" | "del" | "moveFrom" | "moveTo" | "rPrChange" | "pPrChange" | "sectPrChange" | "cellIns" | "cellDel");
    if id.w() && is_revision {
        push_capped(&mut state.facts.revisions, state.fact_cap, &mut state.fact_capped, attr_ns(attrs, res, NS_W, b"id"), part);
    }
}

/// Record header, footer, style, and numbering references found in elements.
fn record_reference_ids(state: &mut State, id: &ElemId, attrs: &[Attribute], res: &NamespaceResolver, role: PartRole, part: &str) {
    let cap = state.fact_cap;
    match id.local.as_str() {
        "headerReference" if id.w() && role.is_main => {
            push_capped(&mut state.facts.header_refs, cap, &mut state.fact_capped, attr_ns(attrs, res, NS_R, b"id"), part);
        }
        "footerReference" if id.w() && role.is_main => {
            push_capped(&mut state.facts.footer_refs, cap, &mut state.fact_capped, attr_ns(attrs, res, NS_R, b"id"), part);
        }
        "pStyle" | "rStyle" if id.w() => {
            push_capped(&mut state.facts.style_refs, cap, &mut state.fact_capped, attr_ns(attrs, res, NS_W, b"val"), part);
        }
        "numId" if id.w() => {
            push_capped(&mut state.facts.num_ids, cap, &mut state.fact_capped, attr_ns(attrs, res, NS_W, b"val"), part);
        }
        _ => {}
    }
}

/// Insert a collected id/name into a set, honouring the fact budget.
fn insert_name(into: &mut BTreeSet<String>, cap: usize, capped: &mut bool, value: Option<String>) {
    if let Some(v) = value {
        if into.len() >= cap {
            *capped = true;
        } else {
            into.insert(v);
        }
    }
}

/// Push a `(value, part)` pair into a vector, bounded by the fact capacity limit.
fn push_capped(into: &mut Vec<(String, String)>, cap: usize, capped: &mut bool, value: Option<String>, part: &str) {
    if into.len() >= cap {
        *capped = true;
        return;
    }
    if let Some(v) = value {
        into.push((v, part.to_string()));
    }
}

/// Examine structural constraints such as fields, moves, deleted text, and custom XML elements.
fn record_structure(state: &mut State, id: &ElemId, attrs: &[Attribute], res: &NamespaceResolver, part: &str, acc: &mut Acc, empty: bool) {
    if !id.w() {
        return;
    }
    match id.local.as_str() {
        "fldChar" => {
            if let Some(t) = attr_ns(attrs, res, NS_W, b"fldCharType") {
                match t.as_str() {
                    "begin" => state.fld_depth += 1,
                    "end" => {
                        if state.fld_depth == 0 {
                            state.fld_extra_end += 1;
                        } else {
                            state.fld_depth -= 1;
                        }
                    }
                    _ => {}
                }
            }
        }
        "moveFromRangeStart" => {
            if let Some(x) = attr_ns(attrs, res, NS_W, b"id") {
                if state.move_from_start.len() >= state.fact_cap || state.move_start_ids.len() >= state.fact_cap {
                    state.fact_capped = true;
                } else {
                    state.move_from_start.insert(x.clone());
                    state.move_start_ids.push(x);
                }
            }
            insert_name(&mut state.move_from_names, state.fact_cap, &mut state.fact_capped, attr_ns(attrs, res, NS_W, b"name"));
        }
        "moveFromRangeEnd" => {
            insert_name(&mut state.move_from_end, state.fact_cap, &mut state.fact_capped, attr_ns(attrs, res, NS_W, b"id"));
        }
        "moveToRangeStart" => {
            if let Some(x) = attr_ns(attrs, res, NS_W, b"id") {
                if state.move_to_start.len() >= state.fact_cap || state.move_start_ids.len() >= state.fact_cap {
                    state.fact_capped = true;
                } else {
                    state.move_to_start.insert(x.clone());
                    state.move_start_ids.push(x);
                }
            }
            insert_name(&mut state.move_to_names, state.fact_cap, &mut state.fact_capped, attr_ns(attrs, res, NS_W, b"name"));
        }
        "moveToRangeEnd" => {
            insert_name(&mut state.move_to_end, state.fact_cap, &mut state.fact_capped, attr_ns(attrs, res, NS_W, b"id"));
        }
        "ins" | "del" | "moveFrom" | "moveTo" => {
            if !empty {
                state.wrapper_depth += 1;
            }
        }
        "customXml" => {
            if state.wrapper_depth > 0 {
                acc.add(
                    "CUSTOMXML-SCHEMA",
                    Some(part.to_string()),
                    format!("a w:customXml appears inside a revision or move wrapper in {part}; MS-OI29500 says Word will not open such a document"),
                );
            }
        }
        "t" if state.del_depth > 0 => {
            acc.add("STRUCT-DELTEXT", Some(part.to_string()), format!("a w:t element appears inside a w:del in {part}"));
        }
        "delText" if state.del_depth == 0 => {
            acc.add("STRUCT-DELTEXT", Some(part.to_string()), format!("a w:delText element appears outside a w:del in {part}"));
        }
        _ => {}
    }
}

/// The DrawingML main namespace (`a:blip`).
const NS_A: &str = "http://schemas.openxmlformats.org/drawingml/2006/main";
/// The VML namespace (`v:imagedata`).
const NS_V: &str = "urn:schemas-microsoft-com:vml";

/// Collect relationship references (`r:id`, `r:embed`, `r:link`) from element attributes up to the fact limit.
fn collect_refs(id: &ElemId, attrs: &[Attribute], res: &NamespaceResolver, out: &mut Vec<RefFact>, cap: usize, capped: &mut bool) {
    // headerReference/footerReference are checked by REF-HEADER, not REL-REF-MISSING.
    if id.w() && matches!(id.local.as_str(), "headerReference" | "footerReference") {
        return;
    }
    let image_elem = (id.ns.as_deref() == Some(NS_A) && id.local == "blip") || (id.ns.as_deref() == Some(NS_V) && id.local == "imagedata");
    for a in attrs {
        let key = a.key;
        let local = key.local_name();
        let l = local.as_ref();
        if !matches!(l, b"id" | b"embed" | b"link") {
            continue;
        }
        if !ns_matches(res.resolve_attribute(key).0, NS_R) {
            continue;
        }
        if let Ok(v) = a.normalized_value(quick_xml::XmlVersion::Implicit1_0) {
            let image = image_elem && matches!(l, b"embed" | b"link" | b"id");
            if out.len() >= cap {
                *capped = true;
                return;
            }
            out.push(RefFact { attr: String::from_utf8_lossy(l).into_owned(), value: v.into_owned(), image });
        }
    }
}

/// Report `STRUCT-MOVE-UNPAIRED` findings for items present in `set` but missing from `counterpart`.
fn report_unpaired(acc: &mut Acc, part: &str, tag: &str, set: &BTreeSet<String>, counterpart: &BTreeSet<String>, want: &str) {
    for id in set.difference(counterpart) {
        acc.add("STRUCT-MOVE-UNPAIRED", Some(part.to_string()), format!("{tag} with w:id {id} in {part} has no matching {want}"));
    }
}

/// Report `STRUCT-MOVE-UNPAIRED` findings for move names present in `names` without a counterpart.
fn report_name_pairs(acc: &mut Acc, part: &str, tag: &str, names: &BTreeSet<String>, want: &str, other: &BTreeSet<String>) {
    for name in names.difference(other) {
        acc.add("STRUCT-MOVE-UNPAIRED", Some(part.to_string()), format!("{tag} name {name} in {part} has no matching {want} name"));
    }
}

/// Report `STRUCT-MOVE-UNPAIRED` findings when a move start ID is reused within a single part.
fn report_duplicate_move_ids(acc: &mut Acc, part: &str, ids: &[String]) {
    let mut seen = BTreeSet::new();
    let mut reported = BTreeSet::new();
    for id in ids {
        if !seen.insert(id.as_str()) && reported.insert(id.as_str()) {
            acc.add("STRUCT-MOVE-UNPAIRED", Some(part.to_string()), format!("move range-start id {id} appears more than once in {part}"));
        }
    }
}

/// Report `BOOKMARK-UNPAIRED` findings for bookmarks that have a start tag without an end or vice versa.
fn report_bookmarks(acc: &mut Acc, part: &str, starts: &BTreeSet<String>, ends: &BTreeSet<String>) {
    for id in starts.difference(ends) {
        acc.add("BOOKMARK-UNPAIRED", Some(part.to_string()), format!("w:bookmarkStart with w:id {id} in {part} has no matching w:bookmarkEnd"));
    }
    for id in ends.difference(starts) {
        acc.add("BOOKMARK-UNPAIRED", Some(part.to_string()), format!("w:bookmarkEnd with w:id {id} in {part} has no matching w:bookmarkStart"));
    }
}

/// Prefix-established pairing reports: their truth is decided by the scanned prefix alone, so they
/// are monotone (a longer scan cannot make them false) and are emitted on every scan, complete or
/// not. The rules in this class are:
///
/// - `FLD-UNBALANCED` for a `w:fldChar` `end` seen at field depth 0 — field nesting is ordered, so
///   the prefix already proves the end is unbalanced whatever follows.
/// - `STRUCT-MOVE-UNPAIRED` for a duplicate move range-start id — once an id has appeared twice in
///   the prefix, the duplicate exists regardless of the rest.
fn finish_prefix(state: &State, part: &str, acc: &mut Acc) {
    report_duplicate_move_ids(acc, part, &state.move_start_ids);
    if state.fld_extra_end > 0 {
        acc.add("FLD-UNBALANCED", Some(part.to_string()), format!("{part} has {} w:fldChar end(s) with no open begin", state.fld_extra_end));
    }
}

/// Completion-dependent pairing reports: their truth needs the whole part, so they run only on a
/// complete scan. An unclosed `w:fldChar` begin, an unpaired move range start/end, a mismatched move
/// name or an unpaired bookmark could still be paired by content the scan has not read.
fn finish_completion(state: &State, part: &str, acc: &mut Acc) {
    report_unpaired(acc, part, "w:moveFromRangeStart", &state.move_from_start, &state.move_from_end, "w:moveFromRangeEnd");
    report_unpaired(acc, part, "w:moveFromRangeEnd", &state.move_from_end, &state.move_from_start, "w:moveFromRangeStart");
    report_unpaired(acc, part, "w:moveToRangeStart", &state.move_to_start, &state.move_to_end, "w:moveToRangeEnd");
    report_unpaired(acc, part, "w:moveToRangeEnd", &state.move_to_end, &state.move_to_start, "w:moveToRangeStart");
    report_name_pairs(acc, part, "w:moveFromRangeStart", &state.move_from_names, "w:moveToRangeStart", &state.move_to_names);
    report_name_pairs(acc, part, "w:moveToRangeStart", &state.move_to_names, "w:moveFromRangeStart", &state.move_from_names);
    report_bookmarks(acc, part, &state.bookmark_starts, &state.bookmark_ends);
    if state.fld_depth > 0 {
        acc.add("FLD-UNBALANCED", Some(part.to_string()), format!("{part} has {} unclosed w:fldChar begin(s)", state.fld_depth));
    }
}

/// Whether running [`finish_completion`] on the accumulated state would emit at least one report.
/// A matched pair before the break emits nothing; an unmatched begin or end does. (A duplicate move
/// id is prefix-established, so it is not part of this test.)
fn completion_dependent_pending(state: &State) -> bool {
    state.fld_depth > 0
        || state.move_from_start != state.move_from_end
        || state.move_to_start != state.move_to_end
        || state.move_from_names != state.move_to_names
        || state.bookmark_starts != state.bookmark_ends
}

/// Detect attribute prefixes that are undeclared or referenced in `mc:Ignorable` without a namespace declaration.
fn check_prefixes(declared: &[Vec<String>], attrs: &[Attribute], res: &NamespaceResolver, extra: &[String], out: &mut PartScan) {
    for a in attrs {
        match res.resolve_attribute(a.key).0 {
            ResolveResult::Unknown(p) => out.unknown_prefixes.push(lossy(&p)),
            ResolveResult::Bound(Namespace(ns)) if ns == NS_MC.as_bytes() && a.key.local_name().as_ref() == b"Ignorable" => {
                if let Ok(v) = a.normalized_value(quick_xml::XmlVersion::Implicit1_0) {
                    for pfx in v.split_whitespace() {
                        if pfx != "xml" && !declared_has(declared, extra, pfx) {
                            out.unknown_prefixes.push(pfx.to_string());
                        }
                    }
                }
            }
            _ => {}
        }
    }
}

/// Check if prefix `pfx` exists among declared prefixes in current element stack or extras.
fn declared_has(declared: &[Vec<String>], extra: &[String], pfx: &str) -> bool {
    declared.iter().flatten().any(|d| d == pfx) || extra.iter().any(|d| d == pfx)
}

/// Extract XML namespace prefixes declared with `xmlns:prefix` in the given attributes.
fn declarations(attrs: &[Attribute]) -> Vec<String> {
    let mut v = Vec::new();
    for a in attrs {
        if let Some(p) = a.key.as_ref().strip_prefix(b"xmlns:")
            && !p.is_empty()
        {
            v.push(lossy(p));
        }
    }
    v
}

/// Resolve a qualified XML element name against the namespace resolver into an [`ElemId`].
fn resolve_elem(res: &NamespaceResolver, name: QName) -> ElemId {
    let (r, local) = res.resolve_element(name);
    let local = lossy(local.as_ref());
    match r {
        ResolveResult::Unknown(p) => ElemId { ns: None, local, unknown: Some(lossy(&p)) },
        ResolveResult::Bound(Namespace(ns)) => ElemId { ns: Some(lossy(ns)), local, unknown: None },
        ResolveResult::Unbound => ElemId { ns: None, local, unknown: None },
    }
}

/// Find an attribute with the given namespace URI and local name, returning its normalized value.
fn attr_ns(attrs: &[Attribute], res: &NamespaceResolver, uri: &str, name: &[u8]) -> Option<String> {
    attrs
        .iter()
        .find(|a| ns_matches(res.resolve_attribute(a.key).0, uri) && a.key.local_name().as_ref() == name)
        .and_then(|a| a.normalized_value(quick_xml::XmlVersion::Implicit1_0).ok().map(|c| c.into_owned()))
}

/// Check whether a namespace resolve result matches the expected URI.
fn ns_matches(r: ResolveResult<'_>, uri: &str) -> bool {
    matches!(r, ResolveResult::Bound(Namespace(ns)) if ns == uri.as_bytes())
}

/// Find an unprefixed attribute by exact name, returning its normalized value.
fn attr_plain(attrs: &[Attribute], name: &[u8]) -> Option<String> {
    attrs
        .iter()
        .find(|a| {
            let k = a.key.as_ref();
            !k.contains(&b':') && k == name
        })
        .and_then(|a| a.normalized_value(quick_xml::XmlVersion::Implicit1_0).ok().map(|c| c.into_owned()))
}

/// Convert a byte slice to a UTF-8 String, replacing invalid sequences lossily.
fn lossy(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// Canonicalise a hex identifier value (`w14:paraId`, `w15:paraIdParent`, `w16cid:paraId`:
/// ST_LongHexNumber). The same number may be written in either case, so it is case-folded at
/// collection. The joins that compare these values are the ones `COMMENTEX-LINK` actually performs:
/// a commentEx `paraId` against a comments paragraph's `w14:paraId`; a commentEx `paraIdParent`
/// against another commentEx `paraId`; and a commentsIds `w16cid:paraId` against a commentEx
/// `paraId`. The `w16cid:durableId` attribute is not read and no durable-id value is compared.
fn hex_ident(v: String) -> String {
    v.to_ascii_lowercase()
}

/// Check whether the text event consists entirely of XML whitespace characters.
fn is_whitespace(e: &quick_xml::events::BytesText) -> bool {
    e.as_ref().iter().all(|b| matches!(b, b' ' | b'\t' | b'\n' | b'\r'))
}

/// `XML-INVALID-CHAR` for a general entity reference: a character reference (`&#…;`) to a
/// character illegal in XML 1.0. Named entities are ignored.
fn scan_general_ref(part: &str, content: &[u8], acc: &mut Acc) {
    if content.first() != Some(&b'#') {
        return;
    }
    let body = String::from_utf8_lossy(content);
    if let Some(desc) = bad_char_ref(&format!("&{body};")) {
        acc.add("XML-INVALID-CHAR", Some(part.to_string()), format!("{part} contains {desc}, which is illegal in XML 1.0"));
    }
}

/// `XML-INVALID-CHAR` for an illegal character reference inside any attribute value.
fn scan_attr_refs(part: &str, attrs: &[Attribute], acc: &mut Acc) {
    for a in attrs {
        let value = String::from_utf8_lossy(a.value.as_ref()).into_owned();
        let mut rest: &str = &value;
        while let Some(idx) = rest.find("&#") {
            let after = &rest[idx..];
            if let Some(desc) = bad_char_ref(after) {
                acc.add("XML-INVALID-CHAR", Some(part.to_string()), format!("{part} contains {desc}, which is illegal in XML 1.0"));
                break;
            }
            rest = &after[2..];
        }
    }
}

/// Return a description of the first character in `text` that is illegal in XML 1.0. XML 1.0 §2.2
/// allows only tab, LF, CR, U+0020–U+D7FF, U+E000–U+FFFD and U+10000–U+10FFFF. The scan runs over
/// the decoded content, so it never rejects a legal part for its encoding.
pub(crate) fn invalid_literal_char(text: &str) -> Option<String> {
    for c in text.chars() {
        if !xml_char_ok(c as u32) {
            return Some(format!("character U+{:04X}", c as u32));
        }
    }
    None
}

/// Determine whether a Unicode codepoint is a legal XML 1.0 character.
fn xml_char_ok(u: u32) -> bool {
    u == 0x9 || u == 0xA || u == 0xD || (0x20..=0xD7FF).contains(&u) || (0xE000..=0xFFFD).contains(&u) || (0x10000..=0x10FFFF).contains(&u)
}

/// If `s` starts with a `&#…;` character reference to an illegal character, describe it.
fn bad_char_ref(s: &str) -> Option<String> {
    let b = s.as_bytes();
    if b.first() != Some(&b'&') || b.get(1) != Some(&b'#') {
        return None;
    }
    let (radix, mut i) = match b.get(2) {
        Some(b'x' | b'X') => (16u32, 3),
        _ => (10u32, 2),
    };
    let mut value: u32 = 0;
    let mut any = false;
    while let Some(&c) = b.get(i) {
        let Some(d) = digit(c, radix) else { break };
        value = value.saturating_mul(radix).saturating_add(d);
        any = true;
        i += 1;
    }
    if !any || b.get(i) != Some(&b';') {
        return None;
    }
    if xml_char_ok(value) { None } else { Some(format!("a character reference to U+{value:04X}")) }
}

/// Parse an ASCII byte as a digit in the given radix (up to 16).
fn digit(c: u8, radix: u32) -> Option<u32> {
    let v = match c {
        b'0'..=b'9' => u32::from(c - b'0'),
        b'a'..=b'f' => u32::from(c - b'a') + 10,
        b'A'..=b'F' => u32::from(c - b'A') + 10,
        _ => return None,
    };
    (v < radix).then_some(v)
}

/// `w:num`/`w:abstractNum` definitions from a numbering part.
#[derive(Default)]
pub(crate) struct NumberingDefs {
    pub num_ids: Vec<String>,
    pub abstract_ids: Vec<String>,
    /// Each `w:num`'s `numId` and the `w:abstractNumId` it references, if any.
    pub num_to_abstract: Vec<(String, Option<String>)>,
    /// The per-category budget was exhausted while parsing.
    pub capped: bool,
}

/// Whether an open tag in an auxiliary parser should be processed or skipped (H3).
enum AuxOpen<'a> {
    /// The element lies inside a skipped `mc:AlternateContent` subtree.
    Skip,
    /// The element crosses the depth limit: stop parsing and do not process it (J1).
    Stop,
    /// The element's resolved identity and attributes.
    Process(ElemId, Vec<Attribute<'a>>),
}

/// Resolve an open tag for an auxiliary parser: apply the depth bound (H1/J1), the AlternateContent
/// suppression (H3/J5), and read its attributes. `None` means the attribute list is malformed, which
/// the caller treats exactly as a parse failure. `Stop` means the element crossed the depth limit;
/// the caller must stop before reading another event and must not process the element.
fn aux_open<'a>(guard: &mut ParseGuard, alt: &mut AltSuppress, res: &NamespaceResolver, e: &'a BytesStart<'_>, empty: bool) -> Option<AuxOpen<'a>> {
    if !empty && !guard.enter() {
        return Some(AuxOpen::Stop);
    }
    let id = resolve_elem(res, e.name());
    if alt.open(&id, empty) {
        return Some(AuxOpen::Skip);
    }
    let attrs = attrs_of(e)?;
    Some(AuxOpen::Process(id, attrs))
}

/// Parse a numbering part. `None` when it does not parse. `guard.exhausted()` marks the result
/// partial (H1).
pub(crate) fn parse_numbering(text: &str, cap: usize, guard: &mut ParseGuard) -> Option<NumberingDefs> {
    let mut reader = NsReader::from_reader(text.as_bytes());
    let mut buf = Vec::new();
    let mut defs = NumberingDefs::default();
    let mut in_num: Option<usize> = None;
    let mut alt = AltSuppress::new();
    // One entry per open `Start` element: whether it was an active `w:num` whose context must be
    // cleared on its matching close. A skipped (suppressed) `w:num` leaves `in_num` untouched (J5).
    let mut num_frames: Vec<bool> = Vec::new();
    loop {
        if !guard.tick() {
            break;
        }
        match reader.read_event_into(&mut buf) {
            Err(_) => return None,
            Ok(Event::Eof) => break,
            Ok(Event::Start(e)) => {
                let mut opened_num = false;
                match aux_open(guard, &mut alt, reader.resolver(), &e, false) {
                    None => return None,
                    Some(AuxOpen::Stop) => break,
                    Some(AuxOpen::Skip) => {}
                    Some(AuxOpen::Process(id, attrs)) => {
                        opened_num = numbering_open(&mut defs, &mut in_num, &id, &attrs, reader.resolver(), false, cap);
                    }
                }
                num_frames.push(opened_num);
            }
            Ok(Event::Empty(e)) => match aux_open(guard, &mut alt, reader.resolver(), &e, true) {
                None => return None,
                Some(AuxOpen::Stop) => break,
                Some(AuxOpen::Skip) => {}
                Some(AuxOpen::Process(id, attrs)) => {
                    numbering_open(&mut defs, &mut in_num, &id, &attrs, reader.resolver(), true, cap);
                }
            },
            Ok(Event::End(e)) => {
                alt.close(e.name().local_name().as_ref());
                guard.leave();
                if num_frames.pop().unwrap_or(false) {
                    in_num = None;
                }
            }
            _ => {}
        }
        buf.clear();
    }
    Some(defs)
}

/// Handle one open tag in the numbering parser. Returns `true` when it opened an active `w:num`
/// context (whose matching close must clear `in_num`), so a suppressed `w:num` cannot erase an
/// outer active context (J5).
fn numbering_open(
    defs: &mut NumberingDefs,
    in_num: &mut Option<usize>,
    id: &ElemId,
    attrs: &[Attribute],
    res: &NamespaceResolver,
    empty: bool,
    cap: usize,
) -> bool {
    if !id.w() {
        return false;
    }
    match id.local.as_str() {
        "abstractNum" => {
            if let Some(x) = attr_ns(attrs, res, NS_W, b"abstractNumId") {
                if defs.abstract_ids.len() >= cap {
                    defs.capped = true;
                } else {
                    defs.abstract_ids.push(x);
                }
            }
            false
        }
        "num" => {
            if let Some(x) = attr_ns(attrs, res, NS_W, b"numId") {
                if defs.num_ids.len() >= cap {
                    defs.capped = true;
                    *in_num = None;
                    return false;
                }
                defs.num_ids.push(x.clone());
                defs.num_to_abstract.push((x, None));
                if !empty {
                    *in_num = Some(defs.num_to_abstract.len().saturating_sub(1));
                }
                !empty
            } else {
                *in_num = None;
                false
            }
        }
        "abstractNumId" => {
            if let (Some(i), Some(v)) = (*in_num, attr_ns(attrs, res, NS_W, b"val"))
                && let Some(slot) = defs.num_to_abstract.get_mut(i)
            {
                slot.1 = Some(v);
            }
            false
        }
        _ => false,
    }
}

/// The `w:numId/@w:val` values referenced by `w:numPr` in a styles part. `None` when it does not
/// parse. `w:numId` `0` is handled by the caller (it is a sentinel, not a reference). References
/// inside a skipped `mc:AlternateContent` branch are not collected (H3).
pub(crate) fn parse_style_num_ids(text: &str, cap: usize, guard: &mut ParseGuard) -> Option<(Vec<String>, bool)> {
    let mut reader = NsReader::from_reader(text.as_bytes());
    let mut buf = Vec::new();
    let mut out = Vec::new();
    let mut capped = false;
    let mut alt = AltSuppress::new();
    loop {
        if !guard.tick() {
            break;
        }
        match reader.read_event_into(&mut buf) {
            Err(_) => return None,
            Ok(Event::Eof) => break,
            Ok(Event::Start(e)) => match aux_open(guard, &mut alt, reader.resolver(), &e, false) {
                None => return None,
                Some(AuxOpen::Stop) => break,
                Some(AuxOpen::Skip) => {}
                Some(AuxOpen::Process(id, attrs)) => {
                    push_style_num_id(&mut out, &mut capped, &id, &attrs, reader.resolver(), cap);
                }
            },
            Ok(Event::Empty(e)) => match aux_open(guard, &mut alt, reader.resolver(), &e, true) {
                None => return None,
                Some(AuxOpen::Stop) => break,
                Some(AuxOpen::Skip) => {}
                Some(AuxOpen::Process(id, attrs)) => {
                    push_style_num_id(&mut out, &mut capped, &id, &attrs, reader.resolver(), cap);
                }
            },
            Ok(Event::End(e)) => {
                alt.close(e.name().local_name().as_ref());
                guard.leave();
            }
            _ => {}
        }
        buf.clear();
    }
    Some((out, capped))
}

/// Extract and append a `w:numId/@w:val` reference to the list, respecting the capacity cap.
fn push_style_num_id(out: &mut Vec<String>, capped: &mut bool, id: &ElemId, attrs: &[Attribute], res: &NamespaceResolver, cap: usize) {
    if id.w()
        && id.local == "numId"
        && let Some(v) = attr_ns(attrs, res, NS_W, b"val")
    {
        if out.len() >= cap {
            *capped = true;
        } else {
            out.push(v);
        }
    }
}

/// The `w:style/@w:styleId` values of a styles part. `None` when it does not parse. Definitions
/// inside a skipped `mc:AlternateContent` branch are not collected (H3).
pub(crate) fn parse_style_ids(text: &str, cap: usize, guard: &mut ParseGuard) -> Option<(HashSet<String>, bool)> {
    let mut reader = NsReader::from_reader(text.as_bytes());
    let mut buf = Vec::new();
    let mut defs = HashSet::new();
    let mut capped = false;
    let mut alt = AltSuppress::new();
    loop {
        if !guard.tick() {
            break;
        }
        match reader.read_event_into(&mut buf) {
            Err(_) => return None,
            Ok(Event::Eof) => break,
            Ok(Event::Start(e)) => match aux_open(guard, &mut alt, reader.resolver(), &e, false) {
                None => return None,
                Some(AuxOpen::Stop) => break,
                Some(AuxOpen::Skip) => {}
                Some(AuxOpen::Process(id, attrs)) => {
                    push_style_id(&mut defs, &mut capped, &id, &attrs, reader.resolver(), cap);
                }
            },
            Ok(Event::Empty(e)) => match aux_open(guard, &mut alt, reader.resolver(), &e, true) {
                None => return None,
                Some(AuxOpen::Stop) => break,
                Some(AuxOpen::Skip) => {}
                Some(AuxOpen::Process(id, attrs)) => {
                    push_style_id(&mut defs, &mut capped, &id, &attrs, reader.resolver(), cap);
                }
            },
            Ok(Event::End(e)) => {
                alt.close(e.name().local_name().as_ref());
                guard.leave();
            }
            _ => {}
        }
        buf.clear();
    }
    Some((defs, capped))
}

/// Extract and insert a `w:style/@w:styleId` definition into the set, respecting the capacity cap.
fn push_style_id(defs: &mut HashSet<String>, capped: &mut bool, id: &ElemId, attrs: &[Attribute], res: &NamespaceResolver, cap: usize) {
    if id.w()
        && id.local == "style"
        && let Some(v) = attr_ns(attrs, res, NS_W, b"styleId")
    {
        if defs.len() >= cap {
            *capped = true;
        } else {
            defs.insert(v);
        }
    }
}

/// The `w:attachedSchema/@w:val` values of a settings part. `None` when it does not parse.
pub(crate) fn parse_attached_schemas(text: &str, cap: usize, guard: &mut ParseGuard) -> Option<(Vec<String>, bool)> {
    let mut reader = NsReader::from_reader(text.as_bytes());
    let mut buf = Vec::new();
    let mut out = Vec::new();
    let mut capped = false;
    let mut alt = AltSuppress::new();
    loop {
        if !guard.tick() {
            break;
        }
        match reader.read_event_into(&mut buf) {
            Err(_) => return None,
            Ok(Event::Eof) => break,
            Ok(Event::Start(e)) => match aux_open(guard, &mut alt, reader.resolver(), &e, false) {
                None => return None,
                Some(AuxOpen::Stop) => break,
                Some(AuxOpen::Skip) => {}
                Some(AuxOpen::Process(id, attrs)) => {
                    push_attached_schema(&mut out, &mut capped, &id, &attrs, reader.resolver(), cap);
                }
            },
            Ok(Event::Empty(e)) => match aux_open(guard, &mut alt, reader.resolver(), &e, true) {
                None => return None,
                Some(AuxOpen::Stop) => break,
                Some(AuxOpen::Skip) => {}
                Some(AuxOpen::Process(id, attrs)) => {
                    push_attached_schema(&mut out, &mut capped, &id, &attrs, reader.resolver(), cap);
                }
            },
            Ok(Event::End(e)) => {
                alt.close(e.name().local_name().as_ref());
                guard.leave();
            }
            _ => {}
        }
        buf.clear();
    }
    Some((out, capped))
}

/// Extract and append a `w:attachedSchema/@w:val` target to the list, respecting the capacity cap.
fn push_attached_schema(out: &mut Vec<String>, capped: &mut bool, id: &ElemId, attrs: &[Attribute], res: &NamespaceResolver, cap: usize) {
    if id.w()
        && id.local == "attachedSchema"
        && let Some(v) = attr_ns(attrs, res, NS_W, b"val")
    {
        if out.len() >= cap {
            *capped = true;
        } else {
            out.push(v);
        }
    }
}

/// The `ds:schemaRef/@ds:uri` values of a customXml itemProperties part (empty when it does not
/// parse).
pub(crate) fn parse_custom_xml_schemas(text: &str, cap: usize, guard: &mut ParseGuard) -> (Vec<String>, bool) {
    let mut reader = NsReader::from_reader(text.as_bytes());
    let mut buf = Vec::new();
    let mut out = Vec::new();
    let mut capped = false;
    let mut alt = AltSuppress::new();
    loop {
        if !guard.tick() {
            break;
        }
        match reader.read_event_into(&mut buf) {
            Err(_) => return (Vec::new(), false),
            Ok(Event::Eof) => break,
            Ok(Event::Start(e)) => match aux_open(guard, &mut alt, reader.resolver(), &e, false) {
                None | Some(AuxOpen::Skip) => {}
                Some(AuxOpen::Stop) => break,
                Some(AuxOpen::Process(id, attrs)) => {
                    push_custom_xml_schema(&mut out, &mut capped, &id, &attrs, reader.resolver(), cap);
                }
            },
            Ok(Event::Empty(e)) => match aux_open(guard, &mut alt, reader.resolver(), &e, true) {
                None | Some(AuxOpen::Skip) => {}
                Some(AuxOpen::Stop) => break,
                Some(AuxOpen::Process(id, attrs)) => {
                    push_custom_xml_schema(&mut out, &mut capped, &id, &attrs, reader.resolver(), cap);
                }
            },
            Ok(Event::End(e)) => {
                alt.close(e.name().local_name().as_ref());
                guard.leave();
            }
            _ => {}
        }
        buf.clear();
    }
    (out, capped)
}

/// Extract and append a `ds:schemaRef/@ds:uri` target to the list, respecting the capacity cap.
fn push_custom_xml_schema(out: &mut Vec<String>, capped: &mut bool, id: &ElemId, attrs: &[Attribute], res: &NamespaceResolver, cap: usize) {
    if id.local_w(NS_DS)
        && id.local == "schemaRef"
        && let Some(v) = attr_ns(attrs, res, NS_DS, b"uri")
    {
        if out.len() >= cap {
            *capped = true;
        } else {
            out.push(v);
        }
    }
}

/// `w15:commentEx` entries: `(paraId, paraIdParent)`.
#[derive(Default)]
pub(crate) struct CommentEx {
    pub entries: Vec<(String, Option<String>)>,
    /// The per-category budget was exhausted while parsing.
    pub capped: bool,
}

/// Parse a commentsExtended part. `None` when it does not parse.
pub(crate) fn parse_comments_extended(text: &str, cap: usize, guard: &mut ParseGuard) -> Option<CommentEx> {
    let mut reader = NsReader::from_reader(text.as_bytes());
    let mut buf = Vec::new();
    let mut out = CommentEx::default();
    let mut alt = AltSuppress::new();
    loop {
        if !guard.tick() {
            break;
        }
        match reader.read_event_into(&mut buf) {
            Err(_) => return None,
            Ok(Event::Eof) => break,
            Ok(Event::Start(e)) => match aux_open(guard, &mut alt, reader.resolver(), &e, false) {
                None => return None,
                Some(AuxOpen::Stop) => break,
                Some(AuxOpen::Skip) => {}
                Some(AuxOpen::Process(id, attrs)) => {
                    push_comment_ex(&mut out, &id, &attrs, reader.resolver(), cap);
                }
            },
            Ok(Event::Empty(e)) => match aux_open(guard, &mut alt, reader.resolver(), &e, true) {
                None => return None,
                Some(AuxOpen::Stop) => break,
                Some(AuxOpen::Skip) => {}
                Some(AuxOpen::Process(id, attrs)) => {
                    push_comment_ex(&mut out, &id, &attrs, reader.resolver(), cap);
                }
            },
            Ok(Event::End(e)) => {
                alt.close(e.name().local_name().as_ref());
                guard.leave();
            }
            _ => {}
        }
        buf.clear();
    }
    Some(out)
}

/// Extract and append a `w15:commentEx` entry with paragraph and parent IDs, respecting capacity.
fn push_comment_ex(out: &mut CommentEx, id: &ElemId, attrs: &[Attribute], res: &NamespaceResolver, cap: usize) {
    if id.local_w(NS_W15)
        && id.local == "commentEx"
        && let Some(para) = attr_ns(attrs, res, NS_W15, b"paraId")
    {
        let para = hex_ident(para);
        let parent = attr_ns(attrs, res, NS_W15, b"paraIdParent").map(hex_ident);
        if out.entries.len() >= cap {
            out.capped = true;
        } else {
            out.entries.push((para, parent));
        }
    }
}

/// The `w16cid:commentId/@w16cid:paraId` values of a commentsIds part. `None` when it does not
/// parse.
pub(crate) fn parse_comments_ids(text: &str, cap: usize, guard: &mut ParseGuard) -> Option<(Vec<String>, bool)> {
    let mut reader = NsReader::from_reader(text.as_bytes());
    let mut buf = Vec::new();
    let mut out = Vec::new();
    let mut capped = false;
    let mut alt = AltSuppress::new();
    loop {
        if !guard.tick() {
            break;
        }
        match reader.read_event_into(&mut buf) {
            Err(_) => return None,
            Ok(Event::Eof) => break,
            Ok(Event::Start(e)) => match aux_open(guard, &mut alt, reader.resolver(), &e, false) {
                None => return None,
                Some(AuxOpen::Stop) => break,
                Some(AuxOpen::Skip) => {}
                Some(AuxOpen::Process(id, attrs)) => {
                    push_comment_id(&mut out, &mut capped, &id, &attrs, reader.resolver(), cap);
                }
            },
            Ok(Event::Empty(e)) => match aux_open(guard, &mut alt, reader.resolver(), &e, true) {
                None => return None,
                Some(AuxOpen::Stop) => break,
                Some(AuxOpen::Skip) => {}
                Some(AuxOpen::Process(id, attrs)) => {
                    push_comment_id(&mut out, &mut capped, &id, &attrs, reader.resolver(), cap);
                }
            },
            Ok(Event::End(e)) => {
                alt.close(e.name().local_name().as_ref());
                guard.leave();
            }
            _ => {}
        }
        buf.clear();
    }
    Some((out, capped))
}

/// Extract and append a `w16cid:commentId/@w16cid:paraId` entry to the list, respecting capacity.
fn push_comment_id(out: &mut Vec<String>, capped: &mut bool, id: &ElemId, attrs: &[Attribute], res: &NamespaceResolver, cap: usize) {
    if id.local_w(NS_W16CID)
        && id.local == "commentId"
        && let Some(v) = attr_ns(attrs, res, NS_W16CID, b"paraId")
    {
        if out.len() >= cap {
            *capped = true;
        } else {
            out.push(hex_ident(v));
        }
    }
}

/// An element's attributes, or `None` when the attribute list is malformed (a duplicate or an
/// unterminated value).
fn attrs_of<'a>(e: &'a BytesStart<'_>) -> Option<Vec<Attribute<'a>>> {
    let mut attrs = Vec::new();
    for a in e.attributes() {
        attrs.push(a.ok()?);
    }
    Some(attrs)
}
