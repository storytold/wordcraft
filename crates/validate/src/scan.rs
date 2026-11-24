//! Streaming XML scan: well-formedness, namespace-prefix resolution, illegal XML characters,
//! markup-compatibility ignorable prefixes and `mc:AlternateContent` suppression, story-part
//! reference/id collection, structural checks and the small auxiliary parsers for numbering,
//! settings and the comment-threading parts.

use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use crate::xml::XmlReader;
use quick_xml::events::attributes::Attribute;
use quick_xml::events::{BytesStart, Event};
use quick_xml::name::{Namespace, NamespaceResolver, QName, ResolveResult};

use crate::observed::{CompleteSet, Completeness, Observed};
use crate::{Acc, NS_DS, NS_M, NS_MC, NS_R, NS_W, NS_W14, NS_W15, NS_W16CID, NS_WP};

/// Parsed numeric value; invalid spellings remain explicitly separate from valid keys.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum NumericKey {
    Valid(String),
    Invalid(String),
}

macro_rules! numeric_id {
    ($name:ident, $radix:literal) => {
        /// A schema numeric identity, parsed at extraction; raw spelling is diagnostic only.
        #[derive(Clone, Debug)]
        pub(crate) struct $name {
            key: NumericKey,
            raw: String,
        }
        impl $name {
            /// Parse without a machine-integer bound or allocation proportional to numeric value.
            fn parse(raw: String) -> Self {
                observation_contract! {
                    rules: "-";
                    collections: "field.bytes*1";
                    scope: "source-local bounded extraction; no implicit document owner";
                    loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
                    fallback: "retain proven prefix presence and duplicates; return loss to the caller; no negative membership from an incomplete set";
                    package_loss: "independent";
                }

                let lexical = raw.trim_matches([' ', '\t', '\r', '\n']);
                let digits = if $radix == 10 { lexical.strip_prefix(['+', '-']).unwrap_or(lexical) } else { lexical };
                let valid = !digits.is_empty() && digits.bytes().all(|b| if $radix == 10 { b.is_ascii_digit() } else { b.is_ascii_hexdigit() });
                let key = if valid {
                    let digits = digits.trim_start_matches('0').to_ascii_lowercase();
                    NumericKey::Valid(if digits.is_empty() {
                        "0".into()
                    } else if $radix == 10 && lexical.starts_with('-') {
                        format!("-{digits}")
                    } else {
                        digits
                    })
                } else {
                    NumericKey::Invalid(raw.clone())
                };
                Self { key, raw }
            }
        }
        impl PartialEq for $name {
            fn eq(&self, other: &Self) -> bool {
                self.key == other.key
            }
        }
        impl Eq for $name {}
        impl PartialOrd for $name {
            fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
                Some(self.cmp(other))
            }
        }
        impl Ord for $name {
            fn cmp(&self, other: &Self) -> std::cmp::Ordering {
                self.key.cmp(&other.key)
            }
        }
        impl std::hash::Hash for $name {
            fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
                self.key.hash(state);
            }
        }
        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(&self.raw)
            }
        }
        impl From<String> for $name {
            fn from(raw: String) -> Self {
                Self::parse(raw)
            }
        }
        impl From<&str> for $name {
            fn from(raw: &str) -> Self {
                Self::parse(raw.into())
            }
        }
    };
}
numeric_id!(DecimalId, 10);
numeric_id!(HexId, 16);

impl DecimalId {
    /// Numeric zero is the remove-numbering sentinel, regardless of spelling or sign.
    pub(crate) fn is_zero(&self) -> bool {
        matches!(&self.key, NumericKey::Valid(v) if v == "0")
    }
}

impl HexId {
    /// ST_LongHexNumber range validation uses the already parsed identity.
    fn out_of_range(&self) -> bool {
        match &self.key {
            NumericKey::Valid(v) => u32::from_str_radix(v, 16).map_or(true, |n| n == 0 || n >= 0x8000_0000),
            NumericKey::Invalid(_) => true,
        }
    }
}

macro_rules! lexical_id {
    ($name:ident) => {
        /// A lexical identity: numeric-looking spellings deliberately remain distinct.
        #[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub(crate) struct $name(String);
        impl $name {
            /// The exact XML spelling, with no numeric normalization.
            pub(crate) fn as_str(&self) -> &str {
                &self.0
            }
        }
        impl From<String> for $name {
            fn from(raw: String) -> Self {
                Self(raw)
            }
        }
        impl From<&str> for $name {
            fn from(raw: &str) -> Self {
                Self(raw.into())
            }
        }
        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(&self.0)
            }
        }
        impl std::borrow::Borrow<str> for $name {
            fn borrow(&self) -> &str {
                &self.0
            }
        }
        impl std::ops::Deref for $name {
            type Target = str;
            fn deref(&self) -> &str {
                &self.0
            }
        }
    };
}
lexical_id!(RelId);
lexical_id!(StyleId);

/// A role is either established or explicitly conflicting; relationship order never chooses it.
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub(crate) enum PartRole {
    #[default]
    Unknown,
    Main,
    Glossary,
    Story,
    Comments,
    Footnotes,
    Endnotes,
    Settings,
    Conflicting(u8),
}
impl PartRole {
    /// Classification evidence retained for monotone checks even when roles conflict.
    fn bits(self) -> u8 {
        match self {
            Self::Unknown => 0,
            Self::Main => 1,
            Self::Glossary => 2,
            Self::Story => 4,
            Self::Comments => 8,
            Self::Footnotes => 16,
            Self::Endnotes => 32,
            Self::Settings => 64,
            Self::Conflicting(bits) => bits,
        }
    }
    /// Merge all declarations, retaining a conflict instead of whichever was visited first.
    pub(crate) fn merge(self, other: Self) -> Self {
        if self == Self::Unknown {
            other
        } else if other == Self::Unknown || self == other {
            self
        } else {
            Self::Conflicting(self.bits() | other.bits())
        }
    }
    /// Conflicting evidence cannot establish complete definition absence.
    pub(crate) fn conflicting(self) -> bool {
        matches!(self, Self::Conflicting(_))
    }
    pub(crate) fn is_main(self) -> bool {
        self.bits() & 1 != 0
    }
    pub(crate) fn is_glossary(self) -> bool {
        self.bits() & 2 != 0
    }
    pub(crate) fn is_story(self) -> bool {
        self.bits() & 63 != 0
    }
    pub(crate) fn is_comments(self) -> bool {
        self.bits() & 8 != 0
    }
    pub(crate) fn is_footnotes(self) -> bool {
        self.bits() & 16 != 0
    }
    pub(crate) fn is_endnotes(self) -> bool {
        self.bits() & 32 != 0
    }
    pub(crate) fn is_settings(self) -> bool {
        self.bits() & 64 != 0
    }
}

/// Ids collected from the story parts, pooled document-wide.
#[derive(Clone, Default)]
pub(crate) struct StoryFacts {
    pub comment_defs: Observed<Vec<(DecimalId, String)>>,
    pub comment_refs: Observed<Vec<(DecimalId, String)>>,
    /// `w:commentRangeStart` occurrences across this part. A plain counter, not
    /// a pooled fact: 32k ranges must not consume the pooled fact budget.
    pub comment_range_starts: usize,
    /// `w:bookmarkStart` occurrences in this part (compat-count counter).
    pub bookmark_starts: usize,
    /// `w:permStart` occurrences in this part (compat-count counter).
    pub perm_starts: usize,
    /// `w:moveFromRangeStart` + `w:moveToRangeStart` occurrences in this part
    /// (compat-count counter).
    pub move_range_starts: usize,
    /// `w14:paraId` values on paragraphs in comments.xml (for `COMMENTEX-LINK`).
    pub comment_para_ids: Observed<Vec<(HexId, String)>>,
    pub docpr: Observed<Vec<(DecimalId, String)>>,
    pub bookmarks: Observed<Vec<(DecimalId, String)>>,
    pub revisions: Observed<Vec<(DecimalId, String)>>,
    pub footnote_refs: Observed<Vec<(DecimalId, String)>>,
    pub endnote_refs: Observed<Vec<(DecimalId, String)>>,
    pub footnote_defs: Observed<Vec<(DecimalId, String)>>,
    pub endnote_defs: Observed<Vec<(DecimalId, String)>>,
    pub header_refs: Observed<Vec<(RelId, String)>>,
    pub footer_refs: Observed<Vec<(RelId, String)>>,
    pub style_refs: Observed<Vec<(StyleId, String)>>,
    /// `w:numId/@w:val` values used by `w:numPr` in story parts (for `NUM-DANGLING`).
    pub num_ids: Observed<Vec<(DecimalId, String)>>,
}

/// One `r:id`/`r:embed`/`r:link` value found in a part, with the element identity needed to
/// verify typed references (an image element must resolve to an image relationship).
#[derive(Clone)]
pub(crate) struct RefFact {
    pub attr: String,
    pub value: RelId,
    /// The reference is the image target of an image element (`a:blip`, `v:imagedata`).
    pub image: bool,
}

impl StoryFacts {
    /// A source loss affects only the collections extracted from that source.
    pub(crate) fn invalidate(&mut self, reason: &'static str) {
        observation_contract! {
            rules: "-";
            collections: "access.invalidate*14,field.bookmarks*1,field.comment_defs*1,field.comment_para_ids*1,field.comment_refs*1,field.docpr*1,field.endnote_defs*1,field.endnote_refs*1,field.footer_refs*1,field.footnote_defs*1,field.footnote_refs*1,field.header_refs*1,field.num_ids*1,field.revisions*1,field.style_refs*1";
            scope: "source-local bounded extraction; no implicit document owner";
            loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
            fallback: "retain proven prefix presence and duplicates; return loss to the caller; no negative membership from an incomplete set";
            package_loss: "independent";
        }

        self.comment_defs.invalidate(reason);
        self.comment_refs.invalidate(reason);
        self.comment_para_ids.invalidate(reason);
        self.docpr.invalidate(reason);
        self.bookmarks.invalidate(reason);
        self.revisions.invalidate(reason);
        self.footnote_refs.invalidate(reason);
        self.endnote_refs.invalidate(reason);
        self.footnote_defs.invalidate(reason);
        self.endnote_defs.invalidate(reason);
        self.header_refs.invalidate(reason);
        self.footer_refs.invalidate(reason);
        self.style_refs.invalidate(reason);
        self.num_ids.invalidate(reason);
    }
    /// An element whose namespace cannot be expanded may be a definition or a reference of the collection whose
    /// element has its local name (only that one): the collections it could belong to are incomplete, the rest are not.
    pub(crate) fn invalidate_local(&mut self, local: &str) {
        observation_contract! {
            rules: "-";
            collections: "access.invalidate*14,field.bookmarks*1,field.comment_defs*1,field.comment_para_ids*1,field.comment_refs*1,field.docpr*1,field.endnote_defs*1,field.endnote_refs*1,field.footer_refs*1,field.footnote_defs*1,field.footnote_refs*1,field.header_refs*1,field.num_ids*1,field.revisions*1,field.style_refs*1";
            scope: "source-local bounded extraction; no implicit document owner";
            loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
            fallback: "retain proven prefix presence and duplicates; return loss to the caller; no negative membership from an incomplete set";
            package_loss: "independent";
        }

        const REASON: &str = "element namespace unresolved";
        match local {
            "comment" => self.comment_defs.invalidate(REASON),
            "commentReference" => self.comment_refs.invalidate(REASON),
            "p" => self.comment_para_ids.invalidate(REASON),
            "docPr" => self.docpr.invalidate(REASON),
            "bookmarkStart" | "bookmarkEnd" => self.bookmarks.invalidate(REASON),
            "ins" | "del" | "moveFrom" | "moveTo" | "rPrChange" | "pPrChange" | "sectPrChange" | "cellIns" | "cellDel" => {
                self.revisions.invalidate(REASON)
            }
            "footnoteReference" => self.footnote_refs.invalidate(REASON),
            "endnoteReference" => self.endnote_refs.invalidate(REASON),
            "footnote" => self.footnote_defs.invalidate(REASON),
            "endnote" => self.endnote_defs.invalidate(REASON),
            "headerReference" => self.header_refs.invalidate(REASON),
            "footerReference" => self.footer_refs.invalidate(REASON),
            "pStyle" | "rStyle" => self.style_refs.invalidate(REASON),
            "numId" => self.num_ids.invalidate(REASON),
            _ => {}
        }
    }
    /// Preserve the existing package allocation bound while retaining facts per part.
    pub(crate) fn retain_budget(&mut self, held: &mut [usize; 14], cap: usize) -> bool {
        observation_contract! {
            rules: "-";
            collections: "access.retain_budget*14,field.bookmarks*1,field.comment_defs*1,field.comment_para_ids*1,field.comment_refs*1,field.docpr*1,field.endnote_defs*1,field.endnote_refs*1,field.footer_refs*1,field.footnote_defs*1,field.footnote_refs*1,field.header_refs*1,field.num_ids*1,field.revisions*1,field.style_refs*1";
            scope: "source-local bounded extraction; no implicit document owner";
            loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
            fallback: "retain proven prefix presence and duplicates; return loss to the caller; no negative membership from an incomplete set";
            package_loss: "independent";
        }

        let mut dropped = false;
        dropped |= self.comment_defs.retain_budget(&mut held[0], cap);
        dropped |= self.comment_refs.retain_budget(&mut held[1], cap);
        dropped |= self.comment_para_ids.retain_budget(&mut held[2], cap);
        dropped |= self.docpr.retain_budget(&mut held[3], cap);
        dropped |= self.bookmarks.retain_budget(&mut held[4], cap);
        dropped |= self.revisions.retain_budget(&mut held[5], cap);
        dropped |= self.footnote_refs.retain_budget(&mut held[6], cap);
        dropped |= self.endnote_refs.retain_budget(&mut held[7], cap);
        dropped |= self.footnote_defs.retain_budget(&mut held[8], cap);
        dropped |= self.endnote_defs.retain_budget(&mut held[9], cap);
        dropped |= self.header_refs.retain_budget(&mut held[10], cap);
        dropped |= self.footer_refs.retain_budget(&mut held[11], cap);
        dropped |= self.style_refs.retain_budget(&mut held[12], cap);
        dropped |= self.num_ids.retain_budget(&mut held[13], cap);
        dropped
    }
    /// Package-global reductions are deliberate; owner-local joins never use this as definitions.
    pub(crate) fn append(&mut self, from: &Self) {
        observation_contract! {
            rules: "-";
            collections: "access.append*14,field.bookmark_starts*3,field.bookmarks*2,field.comment_defs*2,field.comment_para_ids*2,field.comment_refs*2,field.docpr*2,field.endnote_defs*2,field.endnote_refs*2,field.footer_refs*2,field.footnote_defs*2,field.footnote_refs*2,field.header_refs*2,field.num_ids*2,field.revisions*2,field.style_refs*2";
            scope: "source-local bounded extraction; no implicit document owner";
            loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
            fallback: "retain proven prefix presence and duplicates; return loss to the caller; no negative membership from an incomplete set";
            package_loss: "independent";
        }

        self.comment_defs.append(&from.comment_defs);
        self.comment_refs.append(&from.comment_refs);
        self.comment_para_ids.append(&from.comment_para_ids);
        self.docpr.append(&from.docpr);
        self.bookmarks.append(&from.bookmarks);
        self.revisions.append(&from.revisions);
        self.footnote_refs.append(&from.footnote_refs);
        self.endnote_refs.append(&from.endnote_refs);
        self.footnote_defs.append(&from.footnote_defs);
        self.endnote_defs.append(&from.endnote_defs);
        self.header_refs.append(&from.header_refs);
        self.footer_refs.append(&from.footer_refs);
        self.style_refs.append(&from.style_refs);
        self.num_ids.append(&from.num_ids);
        self.comment_range_starts = self.comment_range_starts.saturating_add(from.comment_range_starts);
        self.bookmark_starts = self.bookmark_starts.saturating_add(from.bookmark_starts);
        self.perm_starts = self.perm_starts.saturating_add(from.perm_starts);
        self.move_range_starts = self.move_range_starts.saturating_add(from.move_range_starts);
    }
}

/// The result of scanning one XML or `.rels` part.
pub(crate) struct PartScan {
    pub malformed: Option<String>,
    pub unknown_prefixes: Vec<String>,
    /// Every `r:id`/`r:embed`/`r:link` value in the part, with element identity.
    pub refs: Vec<RefFact>,
    /// The supported reference attributes cover all observed drawing-reference syntax.
    pub drawing_refs_complete: bool,
    pub facts: StoryFacts,
    /// Relationship ids named by `w:contentPart` elements in this part.
    pub content_parts: Vec<RelId>,
    /// Skipped contentPart ids used only to exempt CT-MISMATCH, never for CONTENTPART-TYPE.
    pub content_part_exemptions: Vec<RelId>,
    /// Native Word 2010 contentPart ids; only resolved InkML targets may exempt CT-MISMATCH.
    pub ink_content_parts: Vec<RelId>,
    /// The fact budget dropped at least one native ink reference.
    pub ink_content_part_capped: bool,
    /// The fact budget dropped at least one exemption-only id.
    pub content_part_exemptions_capped: bool,
    /// The fact budget dropped at least one contentPart id in this part.
    pub content_part_capped: bool,
    /// An `mc:Ignorable` value could not be read (an entity reference): its prefixes were not checked.
    pub ignorable_unread: bool,
    /// An element of unresolved namespace named `contentPart` was met: it may be a consumer of a customXml part.
    pub content_part_unresolved: bool,
    /// This (settings) part carries `w:uiCompat97to2003`.
    pub compat_97_2003: bool,
    /// Special note definitions in this part: (id, is_endnote, w:type).
    pub special_notes: Vec<(DecimalId, bool, String)>,
    /// Ids listed by `w:footnotePr` / `w:endnotePr` elements in this part.
    pub note_listings: (Vec<DecimalId>, Vec<DecimalId>),
    /// A note definition or listing was dropped at the fact budget.
    pub note_capped: bool,
    /// Which note collections an element or attribute of unresolved namespace left unestablished (`NOTE_*`): per
    /// family and per member, never part-wide.
    pub note_loss: u8,
    /// The generic fact budget dropped at least one fact in this part.
    pub fact_capped: bool,
    /// The per-part reference budget dropped at least one `r:id`/`r:embed`/`r:link`.
    pub refs_capped: bool,
    /// All XML events and story facts were examined; absence joins may use this part.
    pub facts_complete: bool,
    /// XML events completed independently of individual fact/reference allocation caps.
    pub syntax_complete: bool,
}

struct ElemId<'a> {
    ns: Option<&'a [u8]>,
    local: String,
    unknown: Option<String>,
    /// The element's namespace is declared but its value could not be expanded (an entity or character
    /// reference that does not resolve). The element may be anything, so it can never establish that
    /// something is absent: not an ancestor, not a counterpart, not a block child.
    unresolved: bool,
}

impl ElemId<'_> {
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
        self.ns == Some(uri.as_bytes())
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
    /// The local names of the definitions this parse collects (empty: none are watched).
    watch: &'static [&'static str],
    /// An element of unknown namespace with a watched local name was met: it may be a definition the parse
    /// could not collect, so the collected set cannot establish an absence.
    unresolved: bool,
}

impl ParseGuard {
    /// Create a new parse guard with the given event and depth limits.
    pub(crate) fn new(max_events: usize, max_depth: usize) -> ParseGuard {
        ParseGuard { max_events, max_depth, events: 0, depth: 0, exhausted: false, reason: None, watch: &[], unresolved: false }
    }

    /// Name the definition elements this parse collects: one of unresolved namespace may be another of them.
    pub(crate) fn watching(mut self, locals: &'static [&'static str]) -> ParseGuard {
        self.watch = locals;
        self
    }

    /// Whether a watched definition element could not be collected because its namespace is unresolved.
    pub(crate) fn unresolved(&self) -> bool {
        self.unresolved
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
        observation_contract! {
            rules: "-";
            collections: "type.Vec*1";
            scope: "source-local bounded extraction; no implicit document owner";
            loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
            fallback: "retain proven prefix presence and duplicates; return loss to the caller; no negative membership from an incomplete set";
            package_loss: "independent";
        }

        AltSuppress { depth: 0, frames: Vec::new() }
    }

    /// Handle an open tag. Returns `true` while inside a subtree that must be skipped; an
    /// `mc:AlternateContent` open tag itself is skipped.
    fn open(&mut self, id: &ElemId, empty: bool) -> bool {
        observation_contract! {
            rules: "-";
            collections: "field.frames*2";
            scope: "source-local bounded extraction; no implicit document owner";
            loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
            fallback: "retain proven prefix presence and duplicates; return loss to the caller; no negative membership from an incomplete set";
            package_loss: "independent";
        }

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
        observation_contract! {
            rules: "-";
            collections: "field.frames*1";
            scope: "source-local bounded extraction; no implicit document owner";
            loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
            fallback: "retain proven prefix presence and duplicates; return loss to the caller; no negative membership from an incomplete set";
            package_loss: "independent";
        }

        if self.frames.pop().unwrap_or(false) {
            self.depth = self.depth.saturating_sub(1);
        }
    }
}

/// Check whether an element identifier corresponds to `mc:AlternateContent`.
fn is_alternate_content(id: &ElemId) -> bool {
    id.ns == Some(NS_MC.as_bytes()) && id.local == "AlternateContent"
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
    /// Which ancestor roles this frame could play if its namespace is unresolved (`U_*`): decided by its LOCAL name,
    /// so an unresolved `q:vendor` is no unknown ancestor of anything (see `unresolved_roles`).
    unresolved: u8,
    is_tc: bool,
    /// A `w:p` frame: math placement is judged against paragraph ancestry.
    is_p: bool,
    /// A `w:footnotePr` frame: its `w:footnote` children are listing entries.
    is_footnote_pr: bool,
    /// A `w:mailMerge` frame: a `w:headerSource` child is HEADERSOURCE-PRESENT.
    is_mail_merge: bool,
    /// A `w:txbxContent` frame: Word ignores comment ranges inside a text
    /// box, so COMMENT-RANGE-UNPAIRED does not collect them there.
    is_txbx_content: bool,
    /// A `w:endnotePr` frame: its `w:endnote` children are listing entries.
    is_endnote_pr: bool,
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
    /// A block of known kind (or a skipped `AlternateContent`) was seen directly in this cell.
    tc_known_block: bool,
    /// A direct child of unresolved namespace named like a block element was seen in this cell.
    tc_unresolved_candidate: bool,
    /// None also represents an unknown last block in skipped AlternateContent.
    tc_last_block: Option<TcEnd>,
    is_del: bool,
    is_wrapper: bool,
    /// This frame is an `mc:AlternateContent` element.
    alt: bool,
    /// For an `mc:Choice` / `mc:Fallback` frame that assigned the
    /// enclosing block's current branch: the branch value to restore
    /// when this frame closes.
    alt_branch_prev: Option<u32>,
    /// The element was opened inside a skipped `mc:AlternateContent` subtree, so its closing event
    /// must not touch the frame counters or report structural findings.
    suppressed: bool,
}

#[derive(Default)]
struct State {
    /// Checks whose judgement depended on the identity of an element of unresolved namespace and so were not made
    /// (by rule id): the part reports them as undecided rather than passing them silently.
    withheld: BTreeSet<&'static str>,
    /// This settings part carries `w:uiCompat97to2003` (the Word 97–2003
    /// compatibility flag the count limits key on).
    compat_97: bool,
    /// Special note definitions in this part: (id, is_endnote, w:type).
    special_notes: Vec<(DecimalId, bool, String)>,
    /// Ids listed by `w:footnotePr` elements in this part.
    note_listings_f: Vec<DecimalId>,
    /// Ids listed by `w:endnotePr` elements in this part.
    note_listings_e: Vec<DecimalId>,
    /// A note definition or listing was dropped at the fact budget.
    note_capped: bool,
    /// A `footnote` / `endnote` element of unresolved namespace was met (folded by role into `note_loss`).
    unresolved_note: (bool, bool),
    /// The `NOTE_*` collections an attribute or parent of unresolved namespace left unestablished.
    note_loss: u8,
    declared: Vec<Vec<String>>,
    /// The pairing families (`FAMILY_*`) an element of unresolved namespace could belong to, by its local name: a
    /// counterpart of that family may hide in it, so that family's completion reports cannot run (`ElemId::unresolved`).
    unresolved_families: u8,
    /// An unresolved `fldChar` was seen: a `w:fldChar` end that follows it may close a begin the scan never saw.
    fld_unresolved_seen: bool,

    /// An unresolved `contentPart` was seen (see `PartScan::content_part_unresolved`).
    content_part_unresolved: bool,
    /// A `w:fldChar` end at depth 0 was not counted because an unresolved `fldChar` precedes it.
    fld_end_withheld: bool,
    frames: Vec<Frame>,
    del_depth: i64,
    wrapper_depth: i64,
    fld_depth: i64,
    fld_extra_end: usize,
    fld_state_unknown: bool,
    move_from_start: BTreeSet<DecimalId>,
    move_from_end: BTreeSet<DecimalId>,
    move_to_start: BTreeSet<DecimalId>,
    move_to_end: BTreeSet<DecimalId>,
    move_from_names: BTreeSet<String>,
    move_to_names: BTreeSet<String>,
    move_start_ids: Vec<DecimalId>,
    bookmark_starts: BTreeSet<DecimalId>,
    bookmark_ends: BTreeSet<DecimalId>,
    /// Comment range ids in the parts where Word honours ranges (main and
    /// glossary stories, outside text boxes); paired at scan completion.
    comment_range_start_ids: BTreeSet<DecimalId>,
    comment_range_end_ids: BTreeSet<DecimalId>,
    /// Canonical `w14:paraId` values seen in this part, with their
    /// occurrence contexts (PARAID-DUP; judged at the prefix report —
    /// a repeat across one block's Choice and Fallback is excused,
    /// every other repeat is a duplicate).
    para_ids: BTreeMap<HexId, ParaOcc>,
    /// The stack of enclosing `mc:AlternateContent` blocks: a sequence
    /// number per block and the branch instance the scan is currently
    /// inside it (0 = inside no branch element).
    alt_stack: Vec<AltCtx>,
    /// Sequence number source for `alt_stack` entries.
    alt_seq: u32,
    /// Instance number source for branch elements (`mc:Choice` /
    /// `mc:Fallback`): every branch element gets its own number, so
    /// two Choices of one block are distinct alternatives.
    alt_branch_seq: u32,
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
    let (encoding, rest) = xml_encoding(bytes)?;
    match encoding {
        Encoding::Utf8 => std::str::from_utf8(rest).map(str::to_owned).map_err(|e| DecodeError::Malformed(format!("invalid UTF-8: {e}"))),
        Encoding::Utf16Le => decode_utf16_strict(rest, true),
        Encoding::Utf16Be => decode_utf16_strict(rest, false),
        Encoding::Other(enc) => Err(DecodeError::Unsupported(enc)),
    }
}

fn xml_encoding(bytes: &[u8]) -> Result<(Encoding, &[u8]), DecodeError> {
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
    Ok((encoding, rest))
}

/// Recover the strictly decodable prefix of a part with malformed UTF-8 or UTF-16.
/// The prefix can supply prolog evidence even if the rest of the part is unreadable.
pub(crate) fn decoded_prefix(bytes: &[u8]) -> Option<Cow<'_, str>> {
    let (encoding, rest) = xml_encoding(bytes).ok()?;
    match encoding {
        Encoding::Utf8 => {
            let prefix = match std::str::from_utf8(rest) {
                Ok(text) => text,
                Err(err) => std::str::from_utf8(&rest[..err.valid_up_to()]).ok()?,
            };
            Some(Cow::Borrowed(prefix))
        }
        Encoding::Utf16Le | Encoding::Utf16Be => {
            let le = matches!(encoding, Encoding::Utf16Le);
            let units = rest.as_chunks::<2>().0.iter().map(|unit| if le { u16::from_le_bytes(*unit) } else { u16::from_be_bytes(*unit) });
            Some(Cow::Owned(char::decode_utf16(units).map_while(Result::ok).collect()))
        }
        Encoding::Other(_) => None,
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum DoctypeForm {
    InternalEntity,
    InternalAttlist,
    ExternalOnly,
    Other,
}

/// Look for a declaration at its XML prolog position and identify its form for grading.
pub(crate) fn doctype_in_prolog(text: &str) -> Option<DoctypeForm> {
    let mut reader = quick_xml::Reader::from_str(text);
    loop {
        let offset = reader.buffer_position() as usize;
        match reader.read_event() {
            Ok(Event::Decl(_) | Event::Comment(_) | Event::PI(_) | Event::Text(_)) => {}
            Ok(Event::DocType(decl)) => {
                // The reader accepts a case-insensitive keyword. XML requires the exact keyword
                // followed by XML whitespace.
                if !matches!(text.as_bytes()[offset..].strip_prefix(b"<!DOCTYPE"), Some([b' ' | b'\t' | b'\r' | b'\n', ..])) {
                    return None;
                }
                let form = std::str::from_utf8(&decl).map_or(DoctypeForm::Other, doctype_form);
                return Some(form);
            }
            _ => return None,
        }
    }
}

fn xml_space(ch: char) -> bool {
    matches!(ch, ' ' | '\t' | '\r' | '\n')
}

fn doctype_form(decl: &str) -> DoctypeForm {
    let decl = decl.trim_matches(xml_space);
    let Some(root_end) = decl.find(|ch| xml_space(ch) || ch == '[') else { return DoctypeForm::Other };
    if root_end == 0 {
        return DoctypeForm::Other;
    }
    let tail = decl[root_end..].trim_start_matches(xml_space);
    if let Some(subset) = tail.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
        internal_subset_form(subset)
    } else if external_id_only(tail) {
        DoctypeForm::ExternalOnly
    } else {
        DoctypeForm::Other
    }
}

fn external_id_only(tail: &str) -> bool {
    let Some((keyword, literals)) = tail.split_once(xml_space) else { return false };
    let literals = literals.trim_start_matches(xml_space);
    let system_literal = match keyword {
        "SYSTEM" => literals,
        "PUBLIC" => {
            let Some((_, rest)) = split_quoted_literal(literals) else { return false };
            if !rest.starts_with(xml_space) {
                return false;
            }
            rest.trim_start_matches(xml_space)
        }
        _ => return false,
    };
    split_quoted_literal(system_literal).is_some_and(|(_, rest)| rest.trim_matches(xml_space).is_empty())
}

/// Recognize the measured forms from top-level declarations, skipping comments and PIs.
fn internal_subset_form(mut subset: &str) -> DoctypeForm {
    let mut form = DoctypeForm::Other;
    loop {
        subset = subset.trim_start_matches(xml_space);
        if subset.is_empty() {
            return form;
        }
        if let Some(rest) = subset.strip_prefix("<!--") {
            let Some((_, after)) = rest.split_once("-->") else { return DoctypeForm::Other };
            subset = after;
        } else if let Some(rest) = subset.strip_prefix("<?") {
            let Some((_, after)) = rest.split_once("?>") else { return DoctypeForm::Other };
            subset = after;
        } else if let Some(rest) = subset.strip_prefix("<!") {
            let Some(end) = find_decl_end(rest.as_bytes(), 0) else { return DoctypeForm::Other };
            let Some((keyword, tail)) = rest[..end].split_once(xml_space) else { return DoctypeForm::Other };
            let declaration = match keyword {
                "ENTITY" if internal_general_entity(tail).is_some() => DoctypeForm::InternalEntity,
                "ATTLIST" if !tail.trim_matches(xml_space).is_empty() => DoctypeForm::InternalAttlist,
                _ => return DoctypeForm::Other,
            };
            if form != DoctypeForm::Other && form != declaration {
                return DoctypeForm::Other;
            }
            form = declaration;
            subset = &rest[end + 1..];
        } else {
            return DoctypeForm::Other;
        }
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
    observation_contract! {
        rules: "-";
        collections: "field.bytes*1";
        scope: "source-local bounded extraction; no implicit document owner";
        loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
        fallback: "retain proven prefix presence and duplicates; return loss to the caller; no negative membership from an incomplete set";
        package_loss: "independent";
    }

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
    observation_contract! {
        rules: "-";
        collections: "type.Vec*1";
        scope: "source-local bounded extraction; no implicit document owner";
        loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
        fallback: "retain proven prefix presence and duplicates; return loss to the caller; no negative membership from an incomplete set";
        package_loss: "independent";
    }

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
    observation_contract! {
        rules: "STRUCT-TC-NO-P";
        collections: "access.into_complete_set*2,access.invalidate*2,field.alt_stack*2,field.content_part_exemptions*2,field.content_parts*2,field.declared*8,field.frames*10,field.note_listings*1,field.note_listings_e*1,field.note_listings_f*1,field.para_ids*1,field.special_notes*2,field.unknown_prefixes*2,parser.parse_doctype_entities*1,type.BTreeMap*1,type.HashMap*1,type.HashSet*1,type.Observed*1,type.Vec*9";
        scope: "source-local bounded extraction; no implicit document owner";
        loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
        fallback: "retain proven prefix presence and duplicates; return loss to the caller; no negative membership from an incomplete set";
        package_loss: "independent";
    }

    let mut reader = XmlReader::new(text);
    let mut buf = Vec::new();
    let mut state = State { fact_cap: acc.limits.facts, ..State::default() };
    let mut out = PartScan {
        malformed: None,
        unknown_prefixes: Vec::new(),
        refs: Vec::new(),
        drawing_refs_complete: true,
        facts: StoryFacts::default(),
        content_parts: Vec::new(),
        content_part_exemptions: Vec::new(),
        ink_content_parts: Vec::new(),
        ink_content_part_capped: false,
        content_part_exemptions_capped: false,
        content_part_capped: false,
        ignorable_unread: false,
        content_part_unresolved: false,
        compat_97_2003: false,
        special_notes: Vec::new(),
        note_listings: (Vec::new(), Vec::new()),
        note_capped: false,
        note_loss: 0,
        fact_capped: false,
        refs_capped: false,
        facts_complete: false,
        syntax_complete: false,
    };
    let mut root_count = 0usize;
    let mut events = 0usize;
    let mut budget_stopped = false;
    let mut doctype_seen = false;
    let mut complete_entities = Observed::complete(HashSet::new()).into_complete_set();
    let mut entity_decls = BTreeMap::new();
    let mut entity_status = HashMap::new();
    let mut entity_work = acc.limits.events;
    let mut entity_notes = (false, false);
    // A relationships part's entity-wrapped content is examined by the relationship walk, which reads the
    // expansion (`package::load_rels`); the markup it delivers is not unexamined here.
    let rels_content_modelled = part.to_ascii_lowercase().ends_with(".rels") && expand_content_entities(text).is_some();
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
                let mut id = resolve_elem(reader.resolver(), e.name());
                let Some(attrs) = open_attributes(&e, &mut out) else {
                    push_unknown_prefix(&mut out, &state.declared, &own_declarations(&e), id.unknown.as_deref());
                    break;
                };
                scan_attr_refs(part, &attrs, acc);
                let was_suppressed = state.alt_depth > 0;
                let decls = declarations(&attrs);
                state.declared.push(decls);
                note_unresolved_element(&mut state, &mut id, &[]);
                let is_alt = enter_alt(&mut state, &id, part, acc);
                let branch_prev = assign_alt_branch(&mut state, &id);
                push_unknown_prefix(&mut out, &state.declared, &[], id.unknown.as_deref());
                check_prefixes(&state.declared, &attrs, reader.resolver(), (&[], complete_entities.as_ref().map(|_| &entity_decls)), &mut out);
                collect_ink_content_part(&id, &attrs, reader.resolver(), &mut out, acc.limits.facts);
                if !was_suppressed {
                    collect_refs(&id, &attrs, reader.resolver(), &mut out, acc.limits.facts, &mut state.refs_capped);
                    collect_content_part(&id, &attrs, reader.resolver(), &mut out.content_parts, acc.limits.facts, &mut out.content_part_capped);
                    on_open(&mut state, &id, &attrs, reader.resolver(), role, part, acc);
                } else {
                    // Only exemption ids and the branch-aware paraId family are collected in skipped content.
                    // Consumption still exempts the pairing, but skipped content must not gain type findings.
                    collect_content_part(
                        &id,
                        &attrs,
                        reader.resolver(),
                        &mut out.content_part_exemptions,
                        acc.limits.facts,
                        &mut out.content_part_exemptions_capped,
                    );
                    check_para_ids(&mut state, &id, &attrs, reader.resolver(), role, part, acc);
                }
                let frame = Frame { alt: is_alt, suppressed: was_suppressed, alt_branch_prev: branch_prev, ..frame_for(&id) };
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
                let mut id = resolve_elem(reader.resolver(), e.name());
                let Some(attrs) = open_attributes(&e, &mut out) else {
                    push_unknown_prefix(&mut out, &state.declared, &own_declarations(&e), id.unknown.as_deref());
                    break;
                };
                scan_attr_refs(part, &attrs, acc);
                let was_suppressed = state.alt_depth > 0;
                let decls = declarations(&attrs);
                note_unresolved_element(&mut state, &mut id, &decls);
                let is_alt = enter_alt(&mut state, &id, part, acc);
                let branch_prev = assign_alt_branch(&mut state, &id);
                push_unknown_prefix(&mut out, &state.declared, &decls, id.unknown.as_deref());
                check_prefixes(&state.declared, &attrs, reader.resolver(), (&decls, complete_entities.as_ref().map(|_| &entity_decls)), &mut out);
                collect_ink_content_part(&id, &attrs, reader.resolver(), &mut out, acc.limits.facts);
                if !was_suppressed {
                    collect_refs(&id, &attrs, reader.resolver(), &mut out, acc.limits.facts, &mut state.refs_capped);
                    collect_content_part(&id, &attrs, reader.resolver(), &mut out.content_parts, acc.limits.facts, &mut out.content_part_capped);
                    on_empty(&mut state, &id, &attrs, reader.resolver(), role, part, acc);
                } else {
                    // Consumption still exempts the pairing, but skipped content must not gain type findings.
                    collect_content_part(
                        &id,
                        &attrs,
                        reader.resolver(),
                        &mut out.content_part_exemptions,
                        acc.limits.facts,
                        &mut out.content_part_exemptions_capped,
                    );
                    check_para_ids(&mut state, &id, &attrs, reader.resolver(), role, part, acc);
                }
                if is_alt {
                    state.alt_depth = state.alt_depth.saturating_sub(1);
                    state.alt_stack.pop();
                }
                // An empty branch element opens and closes at once:
                // restore the block's previous branch value.
                restore_branch(&mut state, branch_prev);
            }
            Ok(Event::End(_)) => {
                state.declared.pop();
                if let Some(f) = state.frames.pop() {
                    if f.alt {
                        state.alt_depth = state.alt_depth.saturating_sub(1);
                        state.alt_stack.pop();
                    }
                    // A closing branch element restores the block's
                    // previous branch value (branch membership is
                    // scoped to the branch element's subtree).
                    restore_branch(&mut state, f.alt_branch_prev);
                    if !f.suppressed {
                        if f.is_del {
                            state.del_depth -= 1;
                        }
                        if f.is_wrapper {
                            state.wrapper_depth -= 1;
                        }
                        close_cell(&mut state, &f, part, role, acc);
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
            Ok(Event::DocType(e)) => {
                if doctype_seen || !state.frames.is_empty() || root_count > 0 {
                    out.malformed = Some("unexpected DOCTYPE declaration".to_string());
                    break;
                }
                doctype_seen = true;
                let (declared, decl_texts) = parse_doctype_entities(e.as_ref(), acc.limits.facts);
                complete_entities = declared.into_complete_set();
                entity_decls = decl_texts;
            }
            Ok(Event::GeneralRef(e)) => {
                let outside_err = if state.frames.is_empty() {
                    Some(if root_count == 0 { "a reference before the document element" } else { "content after the document element" })
                } else {
                    None
                };
                if handle_general_ref_event(part, outside_err, &mut entity_notes, &mut state.fld_state_unknown, &mut out, acc, |acc| {
                    match scan_general_ref(part, e.as_ref(), complete_entities.as_ref(), &entity_decls, &mut entity_status, &mut entity_work, acc) {
                        GeneralRefOutcome::UncheckedReplacement if rels_content_modelled => GeneralRefOutcome::Ok,
                        outcome => outcome,
                    }
                }) {
                    break;
                }
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
    let skipped_content = state.alt_noted;
    let scan_complete = out.malformed.is_none() && !budget_stopped && !state.fact_capped && !state.refs_capped && !skipped_content && !entity_notes.1;
    out.facts_complete = scan_complete;
    out.syntax_complete = out.malformed.is_none() && !budget_stopped && !entity_notes.1;
    if !out.syntax_complete || role.conflicting() {
        state.facts.invalidate("XML source or role incomplete");
    } else if skipped_content {
        state.facts.invalidate("mc:AlternateContent subtree skipped");
    }
    if out.ignorable_unread {
        acc.note(format!("{part} has an mc:Ignorable value that could not be read; its prefixes were not checked"));
        acc.set_incomplete();
    }
    if state.para_ids.values().any(|occ| occ.undecided && !occ.conflict) {
        acc.note(format!(
            "{part} repeats a w14:paraId value in content under an mc:AlternateContent, mc:Choice or mc:Fallback of unresolved namespace; whether the repeats can coexist was not decided"
        ));
        acc.set_incomplete();
    }
    note_withheld(&state, part, acc);
    finish_prefix(&state, part, acc);
    note_scan_incomplete(part, &state, &out, scan_complete, skipped_content, acc);
    out.note_loss = note_loss(&state, role);
    out.facts = state.facts;
    out.compat_97_2003 = state.compat_97;
    out.special_notes = std::mem::take(&mut state.special_notes);
    out.note_listings = (std::mem::take(&mut state.note_listings_f), std::mem::take(&mut state.note_listings_e));
    out.note_capped = state.note_capped;
    out.content_part_unresolved = state.content_part_unresolved;
    out.fact_capped = state.fact_capped;
    out.refs_capped = state.refs_capped;
    out.unknown_prefixes.sort_unstable();
    out.unknown_prefixes.dedup();
    out
}

/// Apply entity reference validation and update malformed/incomplete status; returns `true` to stop scanning.
fn handle_general_ref_event(
    part: &str,
    outside_err: Option<&'static str>,
    entity_notes: &mut (bool, bool),
    fld_state_unknown: &mut bool,
    out: &mut PartScan,
    acc: &mut Acc,
    resolve: impl FnOnce(&mut Acc) -> GeneralRefOutcome,
) -> bool {
    if let Some(msg) = outside_err {
        out.malformed = Some(msg.to_string());
        return true;
    }
    match resolve(acc) {
        GeneralRefOutcome::Ok => false,
        GeneralRefOutcome::Malformed(msg) => {
            out.malformed = Some(msg);
            true
        }
        GeneralRefOutcome::UnresolvedSubset => {
            if !entity_notes.0 {
                acc.note(format!(
                    "{part} uses a named entity reference with an unresolvable DOCTYPE subset; entity declarations could not be verified"
                ));
                acc.set_incomplete();
                entity_notes.0 = true;
            }
            false
        }
        GeneralRefOutcome::UncheckedReplacement => {
            if !entity_notes.1 {
                acc.note(format!("{part} uses a named entity reference whose replacement text contains markup or references and was not expanded"));
                acc.set_incomplete();
                entity_notes.1 = true;
            }
            *fld_state_unknown = true;
            false
        }
    }
}

/// Emit completion-dependent pairing or budget incompleteness notes once scanning stops.
fn note_scan_incomplete(part: &str, state: &State, out: &PartScan, scan_complete: bool, skipped_content: bool, acc: &mut Acc) {
    if scan_complete {
        finish_completion(state, part, acc);
    } else if out.malformed.is_some() && completion_dependent_pending(state) {
        // No INCOMPLETE is marked by a malformed part itself, but here a completion-dependent
        // pairing check could not run and would have reported something: the run could not examine
        // everything a finding depends on. Name the part and the check that could not complete.
        acc.note(format!("{part} ends before its field/move/bookmark pairing checks could complete; they were not run"));
        acc.set_incomplete();
    } else if skipped_content && completion_dependent_pending(state) {
        acc.note(format!("{part} has open field/move/bookmark ranges across skipped mc:AlternateContent; pairing checks were not run"));
        acc.set_incomplete();
    }
    if unresolved_pairing_pending(state) {
        acc.note(format!(
            "{part} has field/move/bookmark ranges that depend on elements whose namespace is not resolved; those pairing checks were not run"
        ));
        acc.set_incomplete();
    }
    if state.fact_capped || state.refs_capped {
        acc.note(format!("{part} exceeded the {}-entry fact/reference budget; later facts or references were not recorded", acc.limits.facts));
        acc.set_incomplete();
    }
}

/// The namespace declarations an element carries even when collecting its attributes failed (a duplicate or
/// malformed attribute): they still scope its own name, so its prefix is not reported undeclared on their account.
fn own_declarations(e: &BytesStart<'_>) -> Vec<String> {
    observation_contract! {
        rules: "-";
        collections: "type.Vec*2";
        scope: "source-local bounded extraction; no implicit document owner";
        loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
        fallback: "retain proven prefix presence and duplicates; return loss to the caller; no negative membership from an incomplete set";
        package_loss: "independent";
    }

    let attrs: Vec<Attribute> = e.attributes().with_checks(false).flatten().collect();
    declarations(&attrs)
}

/// Collect an element's attributes, turning an iterator error (a duplicate or malformed attribute)
/// into a malformed-part message rather than silently dropping it.
fn open_attributes<'a>(e: &'a BytesStart<'_>, out: &mut PartScan) -> Option<Vec<Attribute<'a>>> {
    observation_contract! {
        rules: "-";
        collections: "type.Vec*2";
        scope: "source-local bounded extraction; no implicit document owner";
        loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
        fallback: "retain proven prefix presence and duplicates; return loss to the caller; no negative membership from an incomplete set";
        package_loss: "independent";
    }

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
    observation_contract! {
        rules: "-";
        collections: "field.alt_stack*1,field.frames*1";
        scope: "source-local bounded extraction; no implicit document owner";
        loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
        fallback: "retain proven prefix presence and duplicates; return loss to the caller; no negative membership from an incomplete set";
        package_loss: "independent";
    }

    // An `AlternateContent` of unresolved namespace may be the markup-compatibility one: it is treated as that,
    // so its branches are skipped exactly as the literal spelling's are (nothing in them is established).
    let is_alt = id.local == "AlternateContent" && (id.ns == Some(NS_MC.as_bytes()) || id.unresolved);
    if is_alt {
        state.alt_depth += 1;
        state.alt_seq += 1;
        state.alt_stack.push(AltCtx { seq: state.alt_seq, branch: 0 });
        if !state.alt_noted {
            acc.note(format!("{part} contains mc:AlternateContent; its contents were skipped"));
            state.alt_noted = true;
        }
        if let Some(f) = state.frames.last_mut()
            && f.is_tc
        {
            f.tc_has_block = true;
            f.tc_known_block = true;
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
        unresolved: unresolved_roles(id),
        is_tc: w && local == "tc",
        is_p: w && local == "p",
        is_footnote_pr: w && local == "footnotePr",
        is_mail_merge: w && local == "mailMerge",
        is_txbx_content: w && local == "txbxContent",
        is_endnote_pr: w && local == "endnotePr",
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
        tc_known_block: false,
        tc_unresolved_candidate: false,
        tc_last_block: None,
        is_del: w && local == "del",
        is_wrapper: w && matches!(local, "ins" | "del" | "moveFrom" | "moveTo"),
        alt: false,
        alt_branch_prev: None,
        suppressed: false,
    }
}

/// Evaluate a closing table-cell frame in the main part: a cell with no block content is
/// `STRUCT-TC-NO-P`; one whose last block is not a paragraph is `STRUCT-END-P`. A closed
/// cell's last block is known even if later XML is malformed or truncated, so it is
/// evaluated here; an open cell never is, because later content could repair it.
fn close_cell(state: &mut State, f: &Frame, part: &str, role: PartRole, acc: &mut Acc) {
    observation_contract! {
        rules: "STRUCT-END-P,STRUCT-TC-NO-P";
        collections: "field.withheld*1";
        scope: "source-local bounded extraction; no implicit document owner";
        loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
        fallback: "retain proven prefix presence and duplicates; return loss to the caller; no negative membership from an incomplete set";
        package_loss: "independent";
    }

    if !f.is_tc || !role.is_main() {
        return;
    }
    if f.tc_has_block && f.tc_last_block == Some(TcEnd::Other) {
        acc.add("STRUCT-END-P", Some(part.to_string()), format!("w:tc in {part} does not end with a paragraph"));
    }
    if !f.tc_has_block {
        acc.add("STRUCT-TC-NO-P", Some(part.to_string()), format!("w:tc in {part} has no block-level content"));
    }
    // A cell whose only block-level candidate is an element of unresolved namespace was not judged.
    if f.tc_unresolved_candidate && !f.tc_known_block {
        state.withheld.insert("STRUCT-TC-NO-P");
    }
}

/// Report the checks an unresolved element left undecided: the part is incomplete, never silently passed.
fn note_withheld(state: &State, part: &str, acc: &mut Acc) {
    observation_contract! {
        rules: "-";
        collections: "field.withheld*2,type.Vec*1";
        scope: "source-local bounded extraction; no implicit document owner";
        loss: "an element of unresolved namespace that a check's inputs depend on";
        fallback: "withhold the check; note it and take the incomplete path";
        package_loss: "independent";
    }

    if !state.withheld.is_empty() {
        let rules: Vec<&str> = state.withheld.iter().copied().collect();
        acc.note(format!(
            "{part}: {} could not be decided because an element of unresolved namespace may be one the check depends on",
            rules.join(", ")
        ));
        acc.set_incomplete();
    }
}

/// A block-level child satisfies its enclosing `w:tc` (ECMA-376 Part 1 §17.4.65: a cell with no
/// block content is corrupt). Only the immediately enclosing frame is checked, so a run-level
/// `w:sdt` inside a `w:p` does not satisfy the cell. Also record the terminal block kind.
fn mark_block_content(state: &mut State, id: &ElemId) {
    observation_contract! {
        rules: "-";
        collections: "field.frames*2";
        scope: "source-local bounded extraction; no implicit document owner";
        loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
        fallback: "retain proven prefix presence and duplicates; return loss to the caller; no negative membership from an incomplete set";
        package_loss: "independent";
    }

    if id.unresolved {
        // It may be a paragraph or a table (by its local name): the cell has block content, of unknown last kind.
        if matches!(id.local.as_str(), "p" | "tbl" | "sdt" | "customXml" | "altChunk")
            && let Some(f) = state.frames.last_mut().filter(|f| f.is_tc)
        {
            f.tc_has_block = true;
            f.tc_unresolved_candidate = true;
            f.tc_last_block = None;
        }
        return;
    }
    if !id.w() || !matches!(id.local.as_str(), "p" | "tbl" | "sdt" | "customXml" | "altChunk") {
        return;
    }
    if let Some(f) = state.frames.last_mut()
        && f.is_tc
    {
        f.tc_has_block = true;
        f.tc_known_block = true;
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
        // An unresolved ancestor that may be the paragraph or a block container: block level is unknown.
        if f.is_p || f.unresolved & (U_P | U_BLOCK) != 0 {
            return false;
        }
        if f.block_parent_name.is_some() {
            return true;
        }
    }
    true
}

/// Whether the direct parent is a `w:sdtContent` / `w:customXml` whose block level an unresolved ancestor decides: the walk
/// of `wrapper_is_block_level` reaches an element of unresolved namespace named like a paragraph or a block container
/// before it reaches a known one.
fn wrapper_block_level_undecided(state: &State) -> bool {
    observation_contract! {
        rules: "-";
        collections: "field.frames*1";
        scope: "source-local bounded extraction; no implicit document owner";
        loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
        fallback: "retain proven prefix presence and duplicates; return loss to the caller; no negative membership from an incomplete set";
        package_loss: "independent";
    }

    let Some((parent, outer)) = state.frames.split_last() else { return false };
    if !(parent.is_sdt_content || parent.is_custom_xml) {
        return false;
    }
    for f in outer.iter().rev() {
        if f.is_p || f.block_parent_name.is_some() {
            return false;
        }
        if f.unresolved & (U_P | U_BLOCK) != 0 {
            return true;
        }
    }
    false
}

/// The prohibited direct parent of an `m:oMath` or `m:oMathPara`, when the
/// innermost open frame is one: an inherently block-level container, or a
/// `w:sdtContent` / `w:customXml` sitting at block level. Returns the parent's
/// local name for the finding message.
fn prohibited_parent(state: &State) -> Option<&'static str> {
    observation_contract! {
        rules: "-";
        collections: "field.frames*4";
        scope: "source-local bounded extraction; no implicit document owner";
        loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
        fallback: "retain proven prefix presence and duplicates; return loss to the caller; no negative membership from an incomplete set";
        package_loss: "independent";
    }

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
fn check_math_placement(state: &mut State, id: &ElemId, role: PartRole, part: &str, acc: &mut Acc) {
    observation_contract! {
        rules: "MATH-PLACEMENT";
        collections: "field.frames*7,field.withheld*1";
        scope: "source-local bounded extraction; no implicit document owner";
        loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
        fallback: "retain proven prefix presence and duplicates; return loss to the caller; no negative membership from an incomplete set";
        package_loss: "independent";
    }

    if !role.is_story() || !id.m() {
        return;
    }
    let in_omath = state.frames.iter().any(|f| f.is_omath);
    let in_math = state.frames.iter().any(|f| f.is_math);
    let in_stray = state.frames.iter().any(|f| f.is_math && !f.is_omath && !f.is_omath_para);
    let in_omath_para = state.frames.iter().any(|f| f.is_omath_para);
    let in_parapr = state.frames.iter().any(|f| f.is_omath_parapr);
    let in_p = state.frames.iter().any(|f| f.is_p);
    // An ancestor whose namespace is unresolved withholds an absence only when its local name is the missing ancestor's.
    let unresolved = |roles: u8| state.frames.iter().any(|f| f.unresolved & roles != 0);
    // A finding an unresolved ancestor could be hiding is reported as undecided, never dropped silently.
    let mut undecided = false;
    match id.local.as_str() {
        "oMath" => {
            if in_omath {
                acc.add("MATH-PLACEMENT", Some(part.to_string()), format!("m:oMath in {part} is nested inside another m:oMath"));
            }
            if !in_p {
                if unresolved(U_P) {
                    undecided = true;
                } else {
                    acc.add("MATH-PLACEMENT", Some(part.to_string()), format!("m:oMath in {part} occurs outside any w:p"));
                }
            }
            if let Some(parent) = prohibited_parent(state) {
                acc.add("MATH-PLACEMENT", Some(part.to_string()), format!("m:oMath in {part} is a direct child of w:{parent}"));
            } else if wrapper_block_level_undecided(state) {
                undecided = true;
            }
        }
        "oMathPara" => {
            if in_math {
                acc.add("MATH-PLACEMENT", Some(part.to_string()), format!("m:oMathPara in {part} is a descendant of a math element"));
            }
            if !in_p {
                if unresolved(U_P) {
                    undecided = true;
                } else {
                    acc.add("MATH-PLACEMENT", Some(part.to_string()), format!("m:oMathPara in {part} occurs outside any w:p"));
                }
            }
            if let Some(parent) = prohibited_parent(state) {
                acc.add("MATH-PLACEMENT", Some(part.to_string()), format!("m:oMathPara in {part} is a direct child of w:{parent}"));
            } else if wrapper_block_level_undecided(state) {
                undecided = true;
            }
        }
        // Paragraph properties are a legal child of oMathPara, not stray content.
        "oMathParaPr" if in_omath_para => {}
        // Only an unresolved ancestor named like a math element can be the `m:oMath` family this is judged against.
        _ if !in_omath && !in_stray && !in_parapr && unresolved(U_MATH) => undecided = true,
        _ if !in_omath && !in_stray && !in_parapr => {
            // Batch b6 (Word for the web, 2026-11-07): a lone stray math
            // element opened clean and rendered as plain text, unlike the
            // placement cases above, which drew the corruption banner —
            // so this one case reports at Warning, not the rule's Error.
            acc.add_graded(
                "MATH-PLACEMENT",
                crate::Severity::Warning,
                crate::Evidence::MsDocumented,
                Some(part.to_string()),
                format!("m:{} in {part} is math content outside any m:oMath", id.local),
            );
        }
        _ => {}
    }
    if undecided {
        state.withheld.insert("MATH-PLACEMENT");
    }
}

/// `PROTECTION-PASSWORD` (MS-OI29500 Part 1 §17.15.1.93 note (c)): in a settings
/// part, a `w:documentProtection` or `w:writeProtection` element carrying any
/// AG_Password attribute must carry algorithmName, hashValue, and spinCount
/// together, or Word will not open the file. (Batch b6: Word for the web
/// opens a partial set clean, so the finding reports at Warning.)
/// AG_Password is exactly the four
/// attributes algorithmName, hashValue, saltValue, spinCount in both the Strict
/// and the Transitional wml.xsd; the separate AG_TransitionalPassword group
/// (cryptProviderType, cryptAlgorithmClass, …, hash, salt) is the legacy
/// protection format Word itself writes and opens, so it never triggers this
/// rule. A password attribute repeated under a second prefix bound to the
/// same namespace is reported here rather than silently read as present (a
/// same-prefix duplicate is rejected earlier, as a malformed attribute).
fn check_protection(id: &ElemId, attrs: &[Attribute], res: &NamespaceResolver, role: PartRole, part: &str, acc: &mut Acc) {
    observation_contract! {
        rules: "PROTECTION-PASSWORD";
        collections: "type.Vec*1";
        scope: "source-local bounded extraction; no implicit document owner";
        loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
        fallback: "retain proven prefix presence and duplicates; return loss to the caller; no negative membership from an incomplete set";
        package_loss: "independent";
    }

    if !role.is_settings() || !id.w() || !matches!(id.local.as_str(), "documentProtection" | "writeProtection") {
        return;
    }
    let count = |name: &str| -> usize {
        let w_form =
            attrs.iter().filter(|a| ns_matches(res.resolve_attribute(a.key).0, NS_W) && a.key.local_name().as_ref() == name.as_bytes()).count();
        let plain_form = attrs
            .iter()
            .filter(|a| {
                let k = a.key.as_ref();
                !k.contains(&b':') && k == name.as_bytes()
            })
            .count();
        w_form + plain_form
    };
    let names = ["algorithmName", "hashValue", "saltValue", "spinCount"];
    if !names.iter().any(|n| count(n) > 0) {
        return;
    }
    if let Some(dup) = names.iter().find(|n| count(n) > 1) {
        acc.add(
            "PROTECTION-PASSWORD",
            Some(part.to_string()),
            format!("w:{} in {part} carries the password attribute {dup} more than once", id.local),
        );
        return;
    }
    // An attribute of unresolved namespace may be the one that is "missing": its presence is not decided.
    let unknown = |name: &str| {
        attrs.iter().any(|a| matches!(res.resolve_attribute(a.key).0, ResolveResult::Unknown(_)) && a.key.local_name().as_ref() == name.as_bytes())
    };
    if ["algorithmName", "hashValue", "spinCount"].into_iter().any(|n| count(n) == 0 && unknown(n)) {
        acc.note(format!(
            "w:{} in {part} has a password attribute of unresolved namespace; whether a required one is missing was not decided",
            id.local
        ));
        acc.set_incomplete();
    }
    let missing: Vec<&str> = ["algorithmName", "hashValue", "spinCount"].into_iter().filter(|n| count(n) == 0 && !unknown(n)).collect();
    if !missing.is_empty() {
        acc.add(
            "PROTECTION-PASSWORD",
            Some(part.to_string()),
            format!(
                "w:{} in {part} carries password attributes without {}; MS-OI29500 documents algorithmName, hashValue, and spinCount as required together",
                id.local,
                missing.join(", ")
            ),
        );
    }
}

/// Handle element start tags: update structural flags, deletion depth, IDs, and element structure.
/// Record the Word 97–2003 compatibility flag from a settings part. The
/// `w:uiCompat97to2003` element is an on/off setting (ST_OnOff): presence
/// with no `w:val`, or a val of true/1/on, sets it; an explicit false/0/off
/// leaves it unset.
fn record_compat_flag(state: &mut State, id: &ElemId, attrs: &[Attribute], res: &NamespaceResolver, role: PartRole) {
    if role.is_settings() && id.w() && id.local == "uiCompat97to2003" {
        let off = matches!(attr_ns(attrs, res, NS_W, b"val").as_deref(), Some(v) if v.eq_ignore_ascii_case("false") || v == "0" || v.eq_ignore_ascii_case("off"));
        if !off {
            state.compat_97 = true;
        }
    }
}

/// Collect the two sides of the footnotePr/endnotePr join: special note
/// definitions (a `w:footnote`/`w:endnote` in a notes part whose `w:type`
/// is separator, continuationSeparator, or continuationNotice) and listing
/// entries (a `w:footnote`/`w:endnote` whose parent is `w:footnotePr` /
/// `w:endnotePr` in a SETTINGS part — the listing child exists only in
/// the document-wide form, CT_FtnDocProps; the section-wide form,
/// CT_FtnProps, admits only numbering/positioning children).
fn record_note_pr(state: &mut State, id: &ElemId, attrs: &[Attribute], res: &NamespaceResolver, role: PartRole) {
    observation_contract! {
        rules: "-";
        collections: "field.frames*2,field.note_listings_e*1,field.note_listings_f*1,field.special_notes*2";
        scope: "source-local bounded extraction; no implicit document owner";
        loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
        fallback: "retain proven prefix presence and duplicates; return loss to the caller; no negative membership from an incomplete set";
        package_loss: "independent";
    }

    if !id.w() || (id.local != "footnote" && id.local != "endnote") {
        return;
    }
    let is_end = id.local == "endnote";
    // An identifier this part could not read is an unknown identifier: the collection it belongs to is unestablished.
    if attr_unknown(attrs, res, NS_W, b"id") {
        state.note_loss |= if role.is_settings() { if is_end { NOTE_LIST_E } else { NOTE_LIST_F } } else { NOTE_DEFS };
    }
    if role.is_settings() && state.frames.last().is_some_and(|f| f.unresolved & if is_end { U_ENDNOTEPR } else { U_FOOTNOTEPR } != 0) {
        state.note_loss |= if is_end { NOTE_LIST_E } else { NOTE_LIST_F };
    }
    if ((is_end && role.is_endnotes()) || (!is_end && role.is_footnotes()))
        && let Some(t) = attr_ns(attrs, res, NS_W, b"type")
        && matches!(t.as_str(), "separator" | "continuationSeparator" | "continuationNotice")
        && let Some(x) = attr_ns(attrs, res, NS_W, b"id").map(DecimalId::parse)
    {
        if state.special_notes.len() >= state.fact_cap {
            state.note_capped = true;
        } else {
            state.special_notes.push((x, is_end, t));
        }
    }
    if role.is_settings()
        && let Some(parent) = state.frames.last()
    {
        let listed = if is_end { parent.is_endnote_pr } else { parent.is_footnote_pr };
        if listed && let Some(x) = attr_ns(attrs, res, NS_W, b"id").map(DecimalId::parse) {
            let list = if is_end { &mut state.note_listings_e } else { &mut state.note_listings_f };
            if list.len() >= state.fact_cap {
                state.note_capped = true;
            } else {
                list.push(x);
            }
        }
    }
}

/// `HEADERSOURCE-PRESENT` (MS-OI29500 §17.14.16 note (a)): a
/// `w:headerSource` under `w:mailMerge` in a settings part — "Word
/// fails to open WordprocessingML documents in which this element is
/// present." The element's only schema location is as a mailMerge
/// child, so presence is judged there.
fn check_header_source(state: &mut State, id: &ElemId, role: PartRole, part: &str, acc: &mut Acc) {
    observation_contract! {
        rules: "HEADERSOURCE-PRESENT";
        collections: "field.frames*2,field.withheld*1";
        scope: "source-local bounded extraction; no implicit document owner";
        loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
        fallback: "retain proven prefix presence and duplicates; return loss to the caller; no negative membership from an incomplete set";
        package_loss: "independent";
    }

    if role.is_settings() && id.w() && id.local == "headerSource" && state.frames.last().is_some_and(|f| f.unresolved & U_MAILMERGE != 0) {
        // The parent may be the `w:mailMerge` this is judged against: not decided.
        state.withheld.insert("HEADERSOURCE-PRESENT");
    }
    if role.is_settings() && id.w() && id.local == "headerSource" && state.frames.last().is_some_and(|f| f.is_mail_merge) {
        acc.add(
            "HEADERSOURCE-PRESENT",
            Some(part.to_string()),
            format!("w:headerSource is present under w:mailMerge in {part}; Word fails to open documents in which this element is present"),
        );
    }
}

/// The AlternateContent ancestry of one `w14:paraId` occurrence: one
/// `(block, branch instance)` pair per enclosing `mc:AlternateContent`
/// block, outermost first. Branch instance 0 means the occurrence is
/// inside the block but inside none of its branch elements.
type ParaPath = Vec<(u32, u32)>;

/// A branch instance whose element is an `mc:Choice` / `mc:Fallback` of unresolved namespace carries this bit: it
/// is a branch only if the element is the markup-compatibility one.
const BRANCH_UNKNOWN: u32 = 1 << 31;

/// How two occurrences of one paraId relate.
#[derive(PartialEq)]
enum Coexistence {
    /// Some enclosing block holds them in two different known branch instances: only one is ever selected.
    Excused,
    /// Nothing excuses them: both are in the content a reader selects.
    Coexist,
    /// They are excused only if an element of unresolved namespace is a branch element.
    Undecided,
}

/// Whether two occurrences can coexist in the content a reader
/// selects. They cannot — and the repeat is excused — exactly when
/// some block enclosing both holds them in two different branch
/// instances (two Choices, or a Choice and the Fallback): only one
/// branch of a block is ever selected. A branch instance of 0 (the
/// occurrence sits directly under the block) excuses nothing. Only the
/// established part of each path decides: a branch element of unresolved
/// namespace excuses nothing by itself and leaves the pair undecided
/// unless an established branch pair excuses it.
fn paths_relation(a: &ParaPath, b: &ParaPath) -> Coexistence {
    let (mut i, mut j) = (0usize, 0usize);
    let mut undecided = false;
    while let (Some(&(seq_a, branch_a)), Some(&(seq_b, branch_b))) = (a.get(i), b.get(j)) {
        match seq_a.cmp(&seq_b) {
            std::cmp::Ordering::Less => i += 1,
            std::cmp::Ordering::Greater => j += 1,
            std::cmp::Ordering::Equal => {
                if branch_a != 0 && branch_b != 0 && branch_a != branch_b {
                    if (branch_a | branch_b) & BRANCH_UNKNOWN == 0 {
                        return Coexistence::Excused;
                    }
                    undecided = true;
                }
                i += 1;
                j += 1;
            }
        }
    }
    if undecided { Coexistence::Undecided } else { Coexistence::Coexist }
}

#[cfg(test)]
fn paths_conflict(a: &ParaPath, b: &ParaPath) -> bool {
    paths_relation(a, b) == Coexistence::Coexist
}

/// How many mutually excused occurrences of one value are retained.
/// Beyond this the collection cannot prove uniqueness, and the scan
/// reports the part capped (INCOMPLETE) instead of guessing.
const PARA_OCC_RETAINED: usize = 4;

/// The occurrences of one paraId value in a part, in bounded form: a
/// set of representative occurrence paths that are pairwise mutually
/// excused, plus whether a conflicting pair has been seen.
#[derive(Default)]
struct ParaOcc {
    /// Retained representative paths, pairwise non-conflicting.
    reps: Vec<ParaPath>,
    /// A pair of occurrences that can coexist was seen: a duplicate.
    conflict: bool,
    /// More mutually excused occurrences arrived than are retained;
    /// uniqueness is unprovable for this value.
    overflow: bool,
    /// A pair was excused only if a branch element of unresolved namespace is a branch: whether they can coexist is
    /// undecided.
    undecided: bool,
}

impl ParaOcc {
    /// Record one occurrence at the given ancestry path (the established branches are in the path, unresolved ones
    /// marked `BRANCH_UNKNOWN`).
    fn record(&mut self, path: ParaPath) {
        observation_contract! {
            rules: "-";
            collections: "field.reps*3";
            scope: "source-local bounded extraction; no implicit document owner";
            loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
            fallback: "retain proven prefix presence and duplicates; return loss to the caller; no negative membership from an incomplete set";
            package_loss: "independent";
        }

        if self.conflict {
            return;
        }
        for rep in &self.reps {
            match paths_relation(rep, &path) {
                Coexistence::Coexist => {
                    self.conflict = true;
                    return;
                }
                Coexistence::Undecided => self.undecided = true,
                Coexistence::Excused => {}
            }
        }
        if self.reps.len() < PARA_OCC_RETAINED {
            self.reps.push(path);
        } else {
            self.overflow = true;
        }
    }

    /// Whether a coexisting repeat of this value was seen.
    fn is_duplicate(&self) -> bool {
        self.conflict
    }
}

/// One enclosing `mc:AlternateContent` block on the scan stack.
struct AltCtx {
    /// This block's per-part sequence number.
    seq: u32,
    /// The branch instance the scan is currently inside: 0 = inside
    /// no branch element, otherwise the instance number assigned to
    /// the enclosing `mc:Choice` / `mc:Fallback` element.
    branch: u32,
}

/// The `w14:paraId` / `w14:textId` family (MS-DOCX §2.6.2.3 and
/// §2.6.2.4): PARAID-VALUE findings are emitted on the spot;
/// PARAID-DUP occurrences are collected for the prefix report.
/// Story parts only — paragraphs live in stories. The function runs
/// for content inside `mc:AlternateContent` too (the event loop
/// calls it for suppressed elements): value checks apply everywhere,
/// and duplicate collection tracks each occurrence's full branch
/// ancestry — a repeat is excused exactly when the two occurrences
/// sit in different branch instances of some block enclosing both.
fn check_para_ids(state: &mut State, id: &ElemId, attrs: &[Attribute], res: &NamespaceResolver, role: PartRole, part: &str, acc: &mut Acc) {
    observation_contract! {
        rules: "PARAID-VALUE";
        collections: "field.alt_stack*1,field.para_ids*3";
        scope: "source-local bounded extraction; no implicit document owner";
        loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
        fallback: "retain proven prefix presence and duplicates; return loss to the caller; no negative membership from an incomplete set";
        package_loss: "independent";
    }

    if !role.is_story() {
        return;
    }
    let para = attr_ns(attrs, res, NS_W14, b"paraId").map(HexId::parse);
    let text = attr_ns(attrs, res, NS_W14, b"textId").map(HexId::parse);
    if let Some(raw) = &para {
        if raw.out_of_range() {
            acc.add(
                "PARAID-VALUE",
                Some(part.to_string()),
                format!(
                    "w14:paraId value {raw} on w:{} in {part} is out of range; MS-DOCX requires values greater than 0 and less than 0x80000000",
                    id.local
                ),
            );
        }
        let key = raw.clone();
        if state.para_ids.len() >= state.fact_cap && !state.para_ids.contains_key(&key) {
            state.fact_capped = true;
        } else {
            let path: ParaPath = state.alt_stack.iter().map(|ctx| (ctx.seq, ctx.branch)).collect();
            let occ = state.para_ids.entry(key).or_default();
            occ.record(path);
            if occ.overflow {
                state.fact_capped = true;
            }
        }
    }
    if let Some(raw) = &text {
        if raw.out_of_range() {
            acc.add(
                "PARAID-VALUE",
                Some(part.to_string()),
                format!(
                    "w14:textId value {raw} on w:{} in {part} is out of range; MS-DOCX requires values greater than 0 and less than 0x80000000",
                    id.local
                ),
            );
        }
        if para.is_none() {
            acc.add(
                "PARAID-VALUE",
                Some(part.to_string()),
                format!(
                    "w:{} in {part} carries w14:textId without w14:paraId; MS-DOCX requires any element with a textId to also have a paraId",
                    id.local
                ),
            );
        }
    }
}

/// Assign a branch instance for an `mc:Choice` / `mc:Fallback` opening.
///
/// Only a DIRECT child of an `mc:AlternateContent` element assigns:
/// the element frame below the new one must be the block itself. A
/// branch element nested deeper (inside a paragraph, say) is ordinary
/// content of whatever branch encloses it and assigns nothing. Each
/// assignment numbers a fresh instance — two Choices of one block
/// are different alternatives — and returns the block's previous
/// branch value, which the caller restores when the branch element
/// closes (its frame carries it; an empty element restores at once).
fn assign_alt_branch(state: &mut State, id: &ElemId) -> Option<u32> {
    observation_contract! {
        rules: "-";
        collections: "field.alt_stack*1,field.frames*1";
        scope: "source-local bounded extraction; no implicit document owner";
        loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
        fallback: "retain proven prefix presence and duplicates; return loss to the caller; no negative membership from an incomplete set";
        package_loss: "independent";
    }

    // A branch element of unresolved namespace is a branch only if it is the markup-compatibility one: its instance
    // is marked, and decides nothing by itself.
    let unresolved = id.unresolved;
    if !(id.ns == Some(NS_MC.as_bytes()) || id.unresolved) || (id.local != "Choice" && id.local != "Fallback") {
        return None;
    }
    if !state.frames.last().is_some_and(|f| f.alt) {
        return None;
    }
    let ctx = state.alt_stack.last_mut()?;
    let prev = ctx.branch;
    state.alt_branch_seq += 1;
    ctx.branch = state.alt_branch_seq | if unresolved { BRANCH_UNKNOWN } else { 0 };
    Some(prev)
}

/// Restore the innermost block's branch value saved by
/// `assign_alt_branch` when its branch element closes.
fn restore_branch(state: &mut State, prev: Option<u32>) {
    observation_contract! {
        rules: "-";
        collections: "field.alt_stack*1";
        scope: "source-local bounded extraction; no implicit document owner";
        loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
        fallback: "retain proven prefix presence and duplicates; return loss to the caller; no negative membership from an incomplete set";
        package_loss: "independent";
    }

    if let Some(prev) = prev
        && let Some(ctx) = state.alt_stack.last_mut()
    {
        ctx.branch = prev;
    }
}

fn on_open(state: &mut State, id: &ElemId, attrs: &[Attribute], res: &NamespaceResolver, role: PartRole, part: &str, acc: &mut Acc) {
    mark_block_content(state, id);
    check_math_placement(state, id, role, part, acc);
    check_protection(id, attrs, res, role, part, acc);
    check_header_source(state, id, role, part, acc);
    check_para_ids(state, id, attrs, res, role, part, acc);
    record_compat_flag(state, id, attrs, res, role);
    record_note_pr(state, id, attrs, res, role);
    if id.w() && id.local == "del" {
        state.del_depth += 1;
    }
    record_ids(state, id, attrs, res, role, part);
    record_structure(state, id, attrs, res, part, acc, false);
}

/// Handle self-closing/empty tags: check structural requirements like non-empty table cells, and record IDs.
fn on_empty(state: &mut State, id: &ElemId, attrs: &[Attribute], res: &NamespaceResolver, role: PartRole, part: &str, acc: &mut Acc) {
    observation_contract! {
        rules: "STRUCT-TC-NO-P";
        collections: "-";
        scope: "source-local bounded extraction; no implicit document owner";
        loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
        fallback: "retain proven prefix presence and duplicates; return loss to the caller; no negative membership from an incomplete set";
        package_loss: "independent";
    }

    mark_block_content(state, id);
    check_math_placement(state, id, role, part, acc);
    check_protection(id, attrs, res, role, part, acc);
    check_header_source(state, id, role, part, acc);
    check_para_ids(state, id, attrs, res, role, part, acc);
    record_compat_flag(state, id, attrs, res, role);
    record_note_pr(state, id, attrs, res, role);
    if id.w() && id.local == "tc" && role.is_main() {
        acc.add("STRUCT-TC-NO-P", Some(part.to_string()), format!("w:tc in {part} has no block-level content"));
    }
    record_ids(state, id, attrs, res, role, part);
    record_structure(state, id, attrs, res, part, acc, true);
}

/// Record defined and referenced IDs across story parts (comments, footnotes, endnotes, bookmarks, revisions).
fn record_ids(state: &mut State, id: &ElemId, attrs: &[Attribute], res: &NamespaceResolver, role: PartRole, part: &str) {
    observation_contract! {
        rules: "-";
        collections: "field.docpr*1";
        scope: "source-local bounded extraction; no implicit document owner";
        loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
        fallback: "retain proven prefix presence and duplicates; return loss to the caller; no negative membership from an incomplete set";
        package_loss: "independent";
    }

    if !role.is_story() {
        return;
    }
    record_comment_ids(state, id, attrs, res, role, part);
    record_note_ids(state, id, attrs, res, role, part);
    record_bookmark_ids(state, id, attrs, res, part);
    if id.w() && id.local == "permStart" {
        state.facts.perm_starts += 1;
    }
    record_revision_ids(state, id, attrs, res, part);
    record_reference_ids(state, id, attrs, res, role, part);
    if id.wp() && id.local == "docPr" {
        let cap = state.fact_cap;
        push_capped(&mut state.facts.docpr, cap, &mut state.fact_capped, attr_plain(attrs, b"id").map(DecimalId::parse), part);
    }
}

/// Record comment definitions, references, and paragraph IDs within comments parts.
/// Whether a comment range at this position is one Word honours: the main
/// or glossary story, outside any text box. MS-OI29500 Part 1 §17.13.4.4
/// note (c) and §17.13.4.3 note (b): Word ignores commentRangeStart and
/// commentRangeEnd under a header, footer, text box, footnote, or endnote.
fn range_scope(state: &mut State, role: PartRole) -> bool {
    observation_contract! {
        rules: "COMMENT-RANGE-UNPAIRED";
        collections: "field.frames*2,field.withheld*1";
        scope: "source-local bounded extraction; no implicit document owner";
        loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
        fallback: "retain proven prefix presence and duplicates; return loss to the caller; no negative membership from an incomplete set";
        package_loss: "independent";
    }

    if !(role.is_main() || role.is_glossary()) || state.frames.iter().any(|f| f.is_txbx_content) {
        return false;
    }
    // Inside an unresolved `txbxContent` the range may be one Word ignores: neither collected nor judged.
    if state.frames.iter().any(|f| f.unresolved & U_TXBX != 0) {
        state.withheld.insert("COMMENT-RANGE-UNPAIRED");
        return false;
    }
    true
}

/// Collect one comment range id under the fact budget (the bookmark
/// pairing sets' discipline: at the cap, record the cap and stop).
fn collect_range_id(set: &mut BTreeSet<DecimalId>, cap: usize, capped: &mut bool, id: Option<DecimalId>) {
    observation_contract! {
        rules: "-";
        collections: "type.BTreeSet*1";
        scope: "source-local bounded extraction; no implicit document owner";
        loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
        fallback: "retain proven prefix presence and duplicates; return loss to the caller; no negative membership from an incomplete set";
        package_loss: "independent";
    }

    if let Some(x) = id {
        if set.len() >= cap {
            *capped = true;
        } else {
            set.insert(x);
        }
    }
}

/// Collect canonical decimal comment IDs and range endpoints within the scan budget.
fn record_comment_ids(state: &mut State, id: &ElemId, attrs: &[Attribute], res: &NamespaceResolver, role: PartRole, part: &str) {
    observation_contract! {
        rules: "-";
        collections: "field.comment_defs*1,field.comment_para_ids*1,field.comment_range_end_ids*1,field.comment_range_start_ids*1,field.comment_refs*1";
        scope: "source-local bounded extraction; no implicit document owner";
        loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
        fallback: "retain proven prefix presence and duplicates; return loss to the caller; no negative membership from an incomplete set";
        package_loss: "independent";
    }

    let cap = state.fact_cap;
    match id.local.as_str() {
        "comment" if id.w() && role.is_comments() => {
            push_capped(&mut state.facts.comment_defs, cap, &mut state.fact_capped, attr_ns(attrs, res, NS_W, b"id").map(DecimalId::parse), part);
        }
        "commentReference" if id.w() => {
            push_capped(&mut state.facts.comment_refs, cap, &mut state.fact_capped, attr_ns(attrs, res, NS_W, b"id").map(DecimalId::parse), part);
        }
        "commentRangeStart" if id.w() => {
            state.facts.comment_range_starts += 1;
            if range_scope(state, role) {
                collect_range_id(
                    &mut state.comment_range_start_ids,
                    state.fact_cap,
                    &mut state.fact_capped,
                    attr_ns(attrs, res, NS_W, b"id").map(DecimalId::parse),
                );
            }
        }
        "commentRangeEnd" if id.w() => {
            if range_scope(state, role) {
                collect_range_id(
                    &mut state.comment_range_end_ids,
                    state.fact_cap,
                    &mut state.fact_capped,
                    attr_ns(attrs, res, NS_W, b"id").map(DecimalId::parse),
                );
            }
        }
        "p" if id.w() && role.is_comments() => {
            push_capped(
                &mut state.facts.comment_para_ids,
                cap,
                &mut state.fact_capped,
                attr_ns(attrs, res, NS_W14, b"paraId").map(HexId::parse),
                part,
            );
        }
        _ => {}
    }
}

/// Record footnote and endnote definitions and references.
fn record_note_ids(state: &mut State, id: &ElemId, attrs: &[Attribute], res: &NamespaceResolver, role: PartRole, part: &str) {
    observation_contract! {
        rules: "-";
        collections: "field.endnote_defs*1,field.endnote_refs*1,field.footnote_defs*1,field.footnote_refs*1";
        scope: "source-local bounded extraction; no implicit document owner";
        loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
        fallback: "retain proven prefix presence and duplicates; return loss to the caller; no negative membership from an incomplete set";
        package_loss: "independent";
    }

    let cap = state.fact_cap;
    match id.local.as_str() {
        "footnoteReference" if id.w() => {
            push_capped(&mut state.facts.footnote_refs, cap, &mut state.fact_capped, attr_ns(attrs, res, NS_W, b"id").map(DecimalId::parse), part);
        }
        "endnoteReference" if id.w() => {
            push_capped(&mut state.facts.endnote_refs, cap, &mut state.fact_capped, attr_ns(attrs, res, NS_W, b"id").map(DecimalId::parse), part);
        }
        "footnote" if id.w() && role.is_footnotes() => {
            push_capped(&mut state.facts.footnote_defs, cap, &mut state.fact_capped, attr_ns(attrs, res, NS_W, b"id").map(DecimalId::parse), part);
        }
        "endnote" if id.w() && role.is_endnotes() => {
            push_capped(&mut state.facts.endnote_defs, cap, &mut state.fact_capped, attr_ns(attrs, res, NS_W, b"id").map(DecimalId::parse), part);
        }
        _ => {}
    }
}

/// Record bookmark start and end tags, tracking paired IDs up to the fact budget.
fn record_bookmark_ids(state: &mut State, id: &ElemId, attrs: &[Attribute], res: &NamespaceResolver, part: &str) {
    observation_contract! {
        rules: "-";
        collections: "field.bookmark_ends*2,field.bookmark_starts*3,field.bookmarks*1";
        scope: "source-local bounded extraction; no implicit document owner";
        loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
        fallback: "retain proven prefix presence and duplicates; return loss to the caller; no negative membership from an incomplete set";
        package_loss: "independent";
    }

    match id.local.as_str() {
        "bookmarkStart" if id.w() => {
            // Counted before any budget logic: the compat counter must be exact
            // even when the paired-id set caps out.
            state.facts.bookmark_starts += 1;
            if state.bookmark_starts.len() >= state.fact_cap {
                state.fact_capped = true;
                return;
            }
            if let Some(x) = attr_ns(attrs, res, NS_W, b"id").map(DecimalId::parse) {
                state.bookmark_starts.insert(x.clone());
                push_capped(&mut state.facts.bookmarks, state.fact_cap, &mut state.fact_capped, Some(x), part);
            }
        }
        "bookmarkEnd" if id.w() => {
            if let Some(x) = attr_ns(attrs, res, NS_W, b"id").map(DecimalId::parse) {
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
    observation_contract! {
        rules: "-";
        collections: "field.revisions*1";
        scope: "source-local bounded extraction; no implicit document owner";
        loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
        fallback: "retain proven prefix presence and duplicates; return loss to the caller; no negative membership from an incomplete set";
        package_loss: "independent";
    }

    let is_revision =
        matches!(id.local.as_str(), "ins" | "del" | "moveFrom" | "moveTo" | "rPrChange" | "pPrChange" | "sectPrChange" | "cellIns" | "cellDel");
    if id.w() && is_revision {
        push_capped(&mut state.facts.revisions, state.fact_cap, &mut state.fact_capped, attr_ns(attrs, res, NS_W, b"id").map(DecimalId::parse), part);
    }
}

/// Record header, footer, style, and numbering references found in elements.
fn record_reference_ids(state: &mut State, id: &ElemId, attrs: &[Attribute], res: &NamespaceResolver, role: PartRole, part: &str) {
    observation_contract! {
        rules: "-";
        collections: "field.footer_refs*1,field.header_refs*1,field.num_ids*1,field.style_refs*1";
        scope: "source-local bounded extraction; no implicit document owner";
        loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
        fallback: "retain proven prefix presence and duplicates; return loss to the caller; no negative membership from an incomplete set";
        package_loss: "independent";
    }

    let cap = state.fact_cap;
    match id.local.as_str() {
        "headerReference" if id.w() && role.is_main() => {
            push_capped(&mut state.facts.header_refs, cap, &mut state.fact_capped, attr_ns(attrs, res, NS_R, b"id").map(RelId::from), part);
        }
        "footerReference" if id.w() && role.is_main() => {
            push_capped(&mut state.facts.footer_refs, cap, &mut state.fact_capped, attr_ns(attrs, res, NS_R, b"id").map(RelId::from), part);
        }
        "pStyle" | "rStyle" if id.w() => {
            push_capped(&mut state.facts.style_refs, cap, &mut state.fact_capped, attr_ns(attrs, res, NS_W, b"val").map(StyleId::from), part);
        }
        "numId" if id.w() => {
            push_capped(&mut state.facts.num_ids, cap, &mut state.fact_capped, attr_ns(attrs, res, NS_W, b"val").map(DecimalId::parse), part);
        }
        _ => {}
    }
}

/// Insert a collected id/name into a set, honouring the fact budget.
fn insert_name<T: Ord>(into: &mut BTreeSet<T>, cap: usize, capped: &mut bool, value: Option<T>) {
    observation_contract! {
        rules: "-";
        collections: "type.BTreeSet*1";
        scope: "source-local bounded extraction; no implicit document owner";
        loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
        fallback: "retain proven prefix presence and duplicates; return loss to the caller; no negative membership from an incomplete set";
        package_loss: "independent";
    }

    if let Some(v) = value {
        if into.len() >= cap {
            *capped = true;
        } else {
            into.insert(v);
        }
    }
}

/// Push a `(value, part)` pair into a vector, bounded by the fact capacity limit.
fn push_capped<T>(into: &mut Observed<Vec<(T, String)>>, cap: usize, capped: &mut bool, value: Option<T>, part: &str) {
    observation_contract! {
        rules: "-";
        collections: "access.invalidate*1,access.push_bounded*1,type.Observed*1,type.Vec*1";
        scope: "source-local bounded extraction; no implicit document owner";
        loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
        fallback: "retain proven prefix presence and duplicates; return loss to the caller; no negative membership from an incomplete set";
        package_loss: "independent";
    }

    if into.len() >= cap {
        *capped = true;
        if value.is_some() {
            into.invalidate("collection fact budget");
        }
        return;
    }
    if let Some(value) = value {
        *capped |= into.push_bounded((value, part.to_string()), cap);
    }
}

/// Examine structural constraints such as fields, moves, deleted text, and custom XML elements.
fn record_structure(state: &mut State, id: &ElemId, attrs: &[Attribute], res: &NamespaceResolver, part: &str, acc: &mut Acc, empty: bool) {
    observation_contract! {
        rules: "CUSTOMXML-SCHEMA,STRUCT-DELTEXT";
        collections: "field.frames*1,field.move_from_end*1,field.move_from_names*1,field.move_from_start*2,field.move_start_ids*4,field.move_to_end*1,field.move_to_names*1,field.move_to_start*2,field.withheld*1";
        scope: "source-local bounded extraction; no implicit document owner";
        loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
        fallback: "retain proven prefix presence and duplicates; return loss to the caller; no negative membership from an incomplete set";
        package_loss: "independent";
    }

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
                            if state.fld_state_unknown {
                            } else if state.fld_unresolved_seen {
                                state.fld_end_withheld = true;
                            } else {
                                state.fld_extra_end += 1;
                            }
                        } else {
                            state.fld_depth -= 1;
                        }
                    }
                    _ => {}
                }
            }
        }
        "moveFromRangeStart" => {
            state.facts.move_range_starts += 1;
            if let Some(x) = attr_ns(attrs, res, NS_W, b"id").map(DecimalId::parse) {
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
            insert_name(&mut state.move_from_end, state.fact_cap, &mut state.fact_capped, attr_ns(attrs, res, NS_W, b"id").map(DecimalId::parse));
        }
        "moveToRangeStart" => {
            state.facts.move_range_starts += 1;
            if let Some(x) = attr_ns(attrs, res, NS_W, b"id").map(DecimalId::parse) {
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
            insert_name(&mut state.move_to_end, state.fact_cap, &mut state.fact_capped, attr_ns(attrs, res, NS_W, b"id").map(DecimalId::parse));
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
            if state.frames.iter().any(|f| f.unresolved & U_DEL != 0) {
                // The enclosing element may be the `w:del` this is judged against: not decided.
                state.withheld.insert("STRUCT-DELTEXT");
            } else {
                acc.add("STRUCT-DELTEXT", Some(part.to_string()), format!("a w:delText element appears outside a w:del in {part}"));
            }
        }
        _ => {}
    }
}

/// The DrawingML main namespace (`a:blip`).
const NS_A: &str = "http://schemas.openxmlformats.org/drawingml/2006/main";
/// The VML namespace (`v:imagedata`).
const NS_V: &str = "urn:schemas-microsoft-com:vml";

/// Collect the relationship id named by a `w:contentPart` element, if any.
fn collect_content_part(id: &ElemId, attrs: &[Attribute], res: &NamespaceResolver, out: &mut Vec<RelId>, cap: usize, capped: &mut bool) {
    observation_contract! {
        rules: "-";
        collections: "type.Vec*1";
        scope: "source-local bounded extraction; no implicit document owner";
        loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
        fallback: "retain proven prefix presence and duplicates; return loss to the caller; no negative membership from an incomplete set";
        package_loss: "independent";
    }

    if id.w()
        && id.local == "contentPart"
        && let Some(v) = attr_ns(attrs, res, NS_R, b"id")
    {
        if out.len() >= cap {
            *capped = true;
            return;
        }
        out.push(v.into());
    }
}

/// Collect native Word 2010 references in either branch for pairing exemptions, never the w:contentPart type check.
fn collect_ink_content_part(id: &ElemId, attrs: &[Attribute], res: &NamespaceResolver, out: &mut PartScan, cap: usize) {
    observation_contract! {
        rules: "-";
        collections: "field.ink_content_parts*2";
        scope: "source-local bounded extraction; no implicit document owner";
        loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
        fallback: "retain proven prefix presence and duplicates; return loss to the caller; no negative membership from an incomplete set";
        package_loss: "independent";
    }

    if id.ns == Some(NS_W14.as_bytes())
        && id.local == "contentPart"
        && let Some(value) = attr_ns(attrs, res, NS_R, b"id")
    {
        if out.ink_content_parts.len() >= cap {
            out.ink_content_part_capped = true;
        } else {
            out.ink_content_parts.push(value.into());
        }
    }
}

/// Collect relationship references (`r:id`, `r:embed`, `r:link`) from element attributes up to the fact limit.
fn collect_refs(id: &ElemId, attrs: &[Attribute], res: &NamespaceResolver, out: &mut PartScan, cap: usize, capped: &mut bool) {
    observation_contract! {
        rules: "-";
        collections: "field.refs*2";
        scope: "bounded source-local relationship IDs for existing typed checks and conservative media use";
        loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
        fallback: "preserve the existing reference facts; unsupported drawing-reference attributes only disable media absence";
        package_loss: "independent";
    }

    // headerReference/footerReference are checked by REF-HEADER, not REL-REF-MISSING.
    if id.w() && matches!(id.local.as_str(), "headerReference" | "footerReference") {
        return;
    }
    let image_elem = (id.ns == Some(NS_A.as_bytes()) && id.local == "blip") || (id.ns == Some(NS_V.as_bytes()) && id.local == "imagedata");
    for a in attrs {
        let key = a.key;
        let local = key.local_name();
        let l = local.as_ref();
        let namespace = match res.resolve_attribute(key).0 {
            ResolveResult::Bound(namespace) => namespace,
            // An unknown prefix cannot show this attribute is not a drawing reference.
            ResolveResult::Unknown(_) => {
                out.drawing_refs_complete = false;
                continue;
            }
            ResolveResult::Unbound => continue,
        };
        if namespace.as_ref() != NS_R.as_bytes() {
            // Legacy VML and Strict relationship attributes do not participate in the
            // Transitional reference checks but can prevent proving a drawing is absent.
            if (namespace.as_ref() == b"urn:schemas-microsoft-com:office:office" && l == b"relid")
                || namespace.as_ref() == b"http://purl.oclc.org/ooxml/officeDocument/relationships"
            {
                out.drawing_refs_complete = false;
            }
            continue;
        }
        if !matches!(l, b"id" | b"embed" | b"link") {
            out.drawing_refs_complete = false;
            continue;
        }
        let Ok(v) = a.normalized_value(quick_xml::XmlVersion::Implicit1_0) else {
            out.drawing_refs_complete = false;
            continue;
        };
        if out.refs.len() >= cap {
            *capped = true;
            return;
        }
        out.refs.push(RefFact { attr: String::from_utf8_lossy(l).into_owned(), value: v.into_owned().into(), image: image_elem });
    }
}

/// Report `STRUCT-MOVE-UNPAIRED` findings for items present in `set` but missing from `counterpart`.
fn report_unpaired(acc: &mut Acc, part: &str, tag: &str, set: &BTreeSet<DecimalId>, counterpart: &CompleteSet<&DecimalId>, want: &str) {
    observation_contract! {
        rules: "STRUCT-MOVE-UNPAIRED";
        collections: "access.missing*1,type.BTreeSet*1,type.CompleteSet*1";
        scope: "intentional per-part pairing; counterpart CompleteSet constructed only behind completed uncapped scanner barrier";
        loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
        fallback: "suppress completion-dependent absence on scan loss; independent finish_prefix duplicate and extra-field-end findings survive";
        package_loss: "independent";
    }

    for id in set.iter().filter(|id| counterpart.missing(id)) {
        acc.add("STRUCT-MOVE-UNPAIRED", Some(part.to_string()), format!("{tag} with w:id {id} in {part} has no matching {want}"));
    }
}

/// Move names are paired only against proof-bearing complete counterpart collections.
fn report_name_pairs(acc: &mut Acc, part: &str, tag: &str, names: &BTreeSet<String>, want: &str, other: &CompleteSet<&String>) {
    observation_contract! {
        rules: "STRUCT-MOVE-UNPAIRED";
        collections: "access.missing*1,type.BTreeSet*1,type.CompleteSet*1";
        scope: "intentional per-part pairing; counterpart CompleteSet constructed only behind completed uncapped scanner barrier";
        loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
        fallback: "suppress completion-dependent absence on scan loss; independent finish_prefix duplicate and extra-field-end findings survive";
        package_loss: "independent";
    }

    for name in names.iter().filter(|name| other.missing(name)) {
        acc.add("STRUCT-MOVE-UNPAIRED", Some(part.to_string()), format!("{tag} name {name} in {part} has no matching {want} name"));
    }
}

/// Report `STRUCT-MOVE-UNPAIRED` findings when a move start ID is reused within a single part.
fn report_duplicate_move_ids(acc: &mut Acc, part: &str, ids: &[DecimalId]) {
    observation_contract! {
        rules: "STRUCT-MOVE-UNPAIRED";
        collections: "type.BTreeSet*2";
        scope: "source-local bounded extraction; no implicit document owner";
        loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
        fallback: "retain proven prefix presence and duplicates; return loss to the caller; no negative membership from an incomplete set";
        package_loss: "independent";
    }

    let mut seen = BTreeSet::new();
    let mut reported = BTreeSet::new();
    for id in ids {
        if !seen.insert(id) && reported.insert(id) {
            acc.add("STRUCT-MOVE-UNPAIRED", Some(part.to_string()), format!("move range-start id {id} appears more than once in {part}"));
        }
    }
}

/// Report `COMMENT-RANGE-UNPAIRED` findings for comment ranges that have a
/// start without an end or vice versa (collected only where Word honours
/// ranges — see `range_scope`).
fn report_comment_ranges(
    acc: &mut Acc,
    part: &str,
    starts: &BTreeSet<DecimalId>,
    ends: &BTreeSet<DecimalId>,
    complete_starts: &CompleteSet<&DecimalId>,
    complete_ends: &CompleteSet<&DecimalId>,
) {
    observation_contract! {
        rules: "COMMENT-RANGE-UNPAIRED";
        collections: "access.missing*2,type.BTreeSet*2,type.CompleteSet*2";
        scope: "intentional per-part pairing; counterpart CompleteSet constructed only behind completed uncapped scanner barrier";
        loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
        fallback: "suppress completion-dependent absence on scan loss; independent finish_prefix duplicate and extra-field-end findings survive";
        package_loss: "independent";
    }

    for id in starts.iter().filter(|id| complete_ends.missing(id)) {
        acc.add("COMMENT-RANGE-UNPAIRED", Some(part.to_string()), format!("w:commentRangeStart with w:id {id} in {part} has no matching w:commentRangeEnd; Word anchors the comment at its commentReference instead of the range"));
    }
    for id in ends.iter().filter(|id| complete_starts.missing(id)) {
        acc.add("COMMENT-RANGE-UNPAIRED", Some(part.to_string()), format!("w:commentRangeEnd with w:id {id} in {part} has no matching w:commentRangeStart; Word anchors the comment at its commentReference instead of the range"));
    }
}

/// Bookmark endpoint absence requires complete counterparts in both directions.
fn report_bookmarks(
    acc: &mut Acc,
    part: &str,
    starts: &BTreeSet<DecimalId>,
    ends: &BTreeSet<DecimalId>,
    complete_starts: &CompleteSet<&DecimalId>,
    complete_ends: &CompleteSet<&DecimalId>,
) {
    observation_contract! {
        rules: "BOOKMARK-UNPAIRED";
        collections: "access.missing*2,type.BTreeSet*2,type.CompleteSet*2";
        scope: "intentional per-part pairing; counterpart CompleteSet constructed only behind completed uncapped scanner barrier";
        loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
        fallback: "suppress completion-dependent absence on scan loss; independent finish_prefix duplicate and extra-field-end findings survive";
        package_loss: "independent";
    }

    for id in starts.iter().filter(|id| complete_ends.missing(id)) {
        acc.add("BOOKMARK-UNPAIRED", Some(part.to_string()), format!("w:bookmarkStart with w:id {id} in {part} has no matching w:bookmarkEnd"));
    }
    for id in ends.iter().filter(|id| complete_starts.missing(id)) {
        acc.add("BOOKMARK-UNPAIRED", Some(part.to_string()), format!("w:bookmarkEnd with w:id {id} in {part} has no matching w:bookmarkStart"));
    }
}

/// Materialize scanner-completion evidence for a counterpart set, after finish_completion's barrier.
fn completed_pairs<K: Eq + std::hash::Hash>(values: &BTreeSet<K>) -> Option<CompleteSet<&K>> {
    observation_contract! {
        rules: "-";
        collections: "access.into_complete_set*1,type.BTreeSet*1,type.CompleteSet*1,type.HashSet*1,type.Observed*1";
        scope: "intentional per-part pairing; counterpart CompleteSet constructed only behind completed uncapped scanner barrier";
        loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
        fallback: "suppress completion-dependent absence on scan loss; independent finish_prefix duplicate and extra-field-end findings survive";
        package_loss: "independent";
    }

    Observed::complete(values.iter().collect::<HashSet<_>>()).into_complete_set()
}

/// Prefix-established pairing reports: their truth is decided by the scanned prefix alone, so they
/// are monotone (a longer scan cannot make them false) and are emitted on every scan, complete or
/// not. The rules in this class are:
///
/// - `FLD-UNBALANCED` for a `w:fldChar` `end` seen at field depth 0 — field nesting is ordered, so
///   the prefix already proves the end is unbalanced whatever follows.
/// - `STRUCT-MOVE-UNPAIRED` for a duplicate move range-start id — once an id has appeared twice in
///   the prefix, the duplicate exists regardless of the rest.
/// - `PARAID-DUP` for a repeated `w14:paraId` value in the part — same reasoning.
fn finish_prefix(state: &State, part: &str, acc: &mut Acc) {
    observation_contract! {
        rules: "FLD-UNBALANCED,PARAID-DUP";
        collections: "field.move_start_ids*1,field.para_ids*1";
        scope: "source-local bounded extraction; no implicit document owner";
        loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
        fallback: "retain proven prefix presence and duplicates; return loss to the caller; no negative membership from an incomplete set";
        package_loss: "independent";
    }

    report_duplicate_move_ids(acc, part, &state.move_start_ids);
    for (key, occ) in &state.para_ids {
        if occ.is_duplicate() {
            acc.add(
                "PARAID-DUP",
                Some(part.to_string()),
                format!(
                    "w14:paraId value {key} appears on more than one element in {part}; MS-DOCX requires a paraId to be unique within the document part"
                ),
            );
        }
    }
    if state.fld_extra_end > 0 {
        acc.add("FLD-UNBALANCED", Some(part.to_string()), format!("{part} has {} w:fldChar end(s) with no open begin", state.fld_extra_end));
    }
}

/// Completion-dependent pairing reports: their truth needs the whole part, so they run only on a
/// complete scan. An unclosed `w:fldChar` begin, an unpaired move range start/end, a mismatched move
/// name or an unpaired bookmark could still be paired by content the scan has not read.
fn finish_completion(state: &State, part: &str, acc: &mut Acc) {
    observation_contract! {
        rules: "FLD-UNBALANCED";
        collections: "field.bookmark_ends*2,field.bookmark_starts*2,field.comment_range_end_ids*2,field.comment_range_start_ids*2,field.move_from_end*2,field.move_from_names*2,field.move_from_start*2,field.move_to_end*2,field.move_to_names*2,field.move_to_start*2";
        scope: "intentional per-part pairing; counterpart CompleteSet constructed only behind completed uncapped scanner barrier";
        loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
        fallback: "suppress completion-dependent absence on scan loss; independent finish_prefix duplicate and extra-field-end findings survive";
        package_loss: "independent";
    }

    // Called only after a completed, uncapped source scan. A partial scan uses finish_prefix.
    let Some(from_start) = completed_pairs(&state.move_from_start) else { return };
    let Some(from_end) = completed_pairs(&state.move_from_end) else { return };
    let Some(to_start) = completed_pairs(&state.move_to_start) else { return };
    let Some(to_end) = completed_pairs(&state.move_to_end) else { return };
    let Some(from_names) = completed_pairs(&state.move_from_names) else { return };
    let Some(to_names) = completed_pairs(&state.move_to_names) else { return };
    let Some(bookmark_starts) = completed_pairs(&state.bookmark_starts) else { return };
    let Some(bookmark_ends) = completed_pairs(&state.bookmark_ends) else { return };
    let Some(comment_starts) = completed_pairs(&state.comment_range_start_ids) else { return };
    let Some(comment_ends) = completed_pairs(&state.comment_range_end_ids) else { return };
    let open = |family: u8| state.unresolved_families & family == 0;
    if open(FAMILY_MOVE) {
        report_unpaired(acc, part, "w:moveFromRangeStart", &state.move_from_start, &from_end, "w:moveFromRangeEnd");
        report_unpaired(acc, part, "w:moveFromRangeEnd", &state.move_from_end, &from_start, "w:moveFromRangeStart");
        report_unpaired(acc, part, "w:moveToRangeStart", &state.move_to_start, &to_end, "w:moveToRangeEnd");
        report_unpaired(acc, part, "w:moveToRangeEnd", &state.move_to_end, &to_start, "w:moveToRangeStart");
        report_name_pairs(acc, part, "w:moveFromRangeStart", &state.move_from_names, "w:moveToRangeStart", &to_names);
        report_name_pairs(acc, part, "w:moveToRangeStart", &state.move_to_names, "w:moveFromRangeStart", &from_names);
    }
    if open(FAMILY_BOOKMARK) {
        report_bookmarks(acc, part, &state.bookmark_starts, &state.bookmark_ends, &bookmark_starts, &bookmark_ends);
    }
    if open(FAMILY_COMMENT) {
        report_comment_ranges(acc, part, &state.comment_range_start_ids, &state.comment_range_end_ids, &comment_starts, &comment_ends);
    }
    if state.fld_depth > 0 && open(FAMILY_FIELD) {
        acc.add("FLD-UNBALANCED", Some(part.to_string()), format!("{part} has {} unclosed w:fldChar begin(s)", state.fld_depth));
    }
}

/// Whether running [`finish_completion`] on the accumulated state would emit at least one report.
/// A matched pair before the break emits nothing; an unmatched begin or end does. (A duplicate move
/// id is prefix-established, so it is not part of this test.)
fn completion_dependent_pending(state: &State) -> bool {
    observation_contract! {
        rules: "-";
        collections: "field.bookmark_ends*1,field.bookmark_starts*1,field.comment_range_end_ids*1,field.comment_range_start_ids*1,field.move_from_end*1,field.move_from_names*1,field.move_from_start*1,field.move_to_end*1,field.move_to_names*1,field.move_to_start*1";
        scope: "source-local bounded extraction; no implicit document owner";
        loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
        fallback: "retain proven prefix presence and duplicates; return loss to the caller; no negative membership from an incomplete set";
        package_loss: "independent";
    }

    state.fld_depth > 0
        || state.move_from_start != state.move_from_end
        || state.move_to_start != state.move_to_end
        || state.move_from_names != state.move_to_names
        || state.bookmark_starts != state.bookmark_ends
        || state.comment_range_start_ids != state.comment_range_end_ids
}

/// A pairing family: the range elements whose counterparts one completion report joins.
const FAMILY_FIELD: u8 = 1;
const FAMILY_BOOKMARK: u8 = 2;
const FAMILY_COMMENT: u8 = 4;
const FAMILY_MOVE: u8 = 8;

/// The pairing family an element with this local name belongs to (0: none).
fn pairing_family(local: &str) -> u8 {
    match local {
        "fldChar" => FAMILY_FIELD,
        "bookmarkStart" | "bookmarkEnd" => FAMILY_BOOKMARK,
        "commentRangeStart" | "commentRangeEnd" => FAMILY_COMMENT,
        "moveFromRangeStart" | "moveFromRangeEnd" | "moveToRangeStart" | "moveToRangeEnd" => FAMILY_MOVE,
        _ => 0,
    }
}

/// Whether a completion report was withheld for a family an unresolved element could belong to AND something
/// in that family is unpaired or undecided: the part could not be examined for it (an unrelated unresolved
/// element, or a family with nothing pending, loses nothing).
fn unresolved_pairing_pending(state: &State) -> bool {
    observation_contract! {
        rules: "-";
        collections: "field.bookmark_ends*1,field.bookmark_starts*1,field.comment_range_end_ids*1,field.comment_range_start_ids*1,field.move_from_end*1,field.move_from_names*1,field.move_from_start*1,field.move_to_end*1,field.move_to_names*1,field.move_to_start*1";
        scope: "source-local bounded extraction; no implicit document owner";
        loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
        fallback: "retain proven prefix presence and duplicates; return loss to the caller; no negative membership from an incomplete set";
        package_loss: "independent";
    }

    let families = state.unresolved_families;
    (families & FAMILY_FIELD != 0 && (state.fld_depth > 0 || state.fld_end_withheld))
        || (families & FAMILY_MOVE != 0
            && (state.move_from_start != state.move_from_end
                || state.move_to_start != state.move_to_end
                || state.move_from_names != state.move_to_names))
        || (families & FAMILY_BOOKMARK != 0 && state.bookmark_starts != state.bookmark_ends)
        || (families & FAMILY_COMMENT != 0 && state.comment_range_start_ids != state.comment_range_end_ids)
}

/// Ancestor roles (`U_*`) an element of unresolved namespace could play, from its local name alone.
const U_P: u8 = 1;
const U_DEL: u8 = 2;
/// A block container (or the `sdtContent`/`customXml` wrappers judged by context): the block level of a child is unknown.
const U_BLOCK: u8 = 4;
/// An unresolved element whose local name is an Office Math element: it may be the `m:*` ancestor of math content.
const U_MATH: u8 = 8;
/// An unresolved `mailMerge`: it may be the parent a `w:headerSource` is judged against.
const U_MAILMERGE: u8 = 16;
/// An unresolved `txbxContent`: comment ranges inside it may be ones Word ignores.
const U_TXBX: u8 = 32;
/// An unresolved `footnotePr` / `endnotePr`: it may be the parent whose `w:footnote` / `w:endnote` children are listings.
const U_FOOTNOTEPR: u8 = 64;
const U_ENDNOTEPR: u8 = 128;

/// Note collections left unestablished (`PartScan::note_loss`): the footnote listing, the endnote listing, and the
/// special note definitions of the part.
pub(crate) const NOTE_LIST_F: u8 = 1;
pub(crate) const NOTE_LIST_E: u8 = 2;
pub(crate) const NOTE_DEFS: u8 = 4;
pub(crate) const NOTE_ALL: u8 = 7;

/// What an unresolved `footnote` / `endnote` element could be in a part of this role: a listing entry in settings, a
/// definition in the matching notes part. An unresolved `endnote` is nothing to the footnote listing.
fn note_loss(state: &State, role: PartRole) -> u8 {
    let (footnote, endnote) = state.unresolved_note;
    let any = role.conflicting();
    let mut loss = state.note_loss | if state.note_capped { NOTE_ALL } else { 0 };
    if role.is_settings() || any {
        loss |= if footnote { NOTE_LIST_F } else { 0 } | if endnote { NOTE_LIST_E } else { 0 };
    }
    if (footnote && (role.is_footnotes() || any)) || (endnote && (role.is_endnotes() || any)) {
        loss |= NOTE_DEFS;
    }
    loss
}

/// Whether an attribute with this local name is present but cannot be read as the namespace's: an unresolved prefix, or
/// a value spelled through an entity this scan does not expand. Its value is unknown, never absent.
fn attr_unknown(attrs: &[Attribute], res: &NamespaceResolver, uri: &str, local: &[u8]) -> bool {
    attrs.iter().any(|a| {
        a.key.as_namespace_binding().is_none()
            && a.key.local_name().as_ref() == local
            && match res.resolve_attribute(a.key).0 {
                ResolveResult::Unknown(_) => true,
                r => ns_matches(r, uri) && a.normalized_value(quick_xml::XmlVersion::Implicit1_0).is_err(),
            }
    })
}

/// The local names of Office Math (ECMA-376 Part 1 §22.1.2) elements: an unresolved ancestor is a possible math
/// ancestor of a stray `m:*` element only when it is named like one.
fn is_omml_name(local: &str) -> bool {
    matches!(
        local,
        "acc"
            | "accPr"
            | "aln"
            | "alnScr"
            | "argPr"
            | "argSz"
            | "bar"
            | "barPr"
            | "baseJc"
            | "begChr"
            | "borderBox"
            | "borderBoxPr"
            | "box"
            | "boxPr"
            | "brk"
            | "brkBin"
            | "brkBinSub"
            | "cGp"
            | "cGpRule"
            | "chr"
            | "count"
            | "cSp"
            | "ctrlPr"
            | "d"
            | "degHide"
            | "den"
            | "diff"
            | "dispDef"
            | "dPr"
            | "e"
            | "endChr"
            | "eqArr"
            | "eqArrPr"
            | "f"
            | "fName"
            | "fPr"
            | "func"
            | "funcPr"
            | "gc"
            | "groupChr"
            | "groupChrPr"
            | "grow"
            | "hideBot"
            | "hideLeft"
            | "hideRight"
            | "hideTop"
            | "interSp"
            | "intLim"
            | "intraSp"
            | "jc"
            | "lim"
            | "limLoc"
            | "limLow"
            | "limLowPr"
            | "limUpp"
            | "limUppPr"
            | "lit"
            | "lMargin"
            | "m"
            | "mathFont"
            | "mathPr"
            | "maxDist"
            | "mc"
            | "mcJc"
            | "mcPr"
            | "mcs"
            | "mPr"
            | "mr"
            | "nary"
            | "naryLim"
            | "naryPr"
            | "noBreak"
            | "nor"
            | "num"
            | "objDist"
            | "oMath"
            | "oMathPara"
            | "oMathParaPr"
            | "opEmu"
            | "phant"
            | "phantPr"
            | "plcHide"
            | "pos"
            | "postSp"
            | "preSp"
            | "r"
            | "rad"
            | "radPr"
            | "rMargin"
            | "rPr"
            | "rSp"
            | "rSpRule"
            | "scr"
            | "sepChr"
            | "show"
            | "shp"
            | "smallFrac"
            | "sPre"
            | "sPrePr"
            | "sSub"
            | "sSubPr"
            | "sSubSup"
            | "sSubSupPr"
            | "sSup"
            | "sSupPr"
            | "strikeBLTR"
            | "strikeH"
            | "strikeTLBR"
            | "strikeV"
            | "sty"
            | "sub"
            | "subHide"
            | "sup"
            | "supHide"
            | "t"
            | "transp"
            | "type"
            | "vertJc"
            | "wrap"
            | "zeroAsc"
            | "zeroDesc"
            | "zeroWid"
    )
}

fn unresolved_roles(id: &ElemId) -> u8 {
    if !id.unresolved {
        return 0;
    }
    let local = id.local.as_str();
    let math = if is_omml_name(local) { U_MATH } else { 0 };
    math | match local {
        "p" => U_P,
        "del" => U_DEL,
        "mailMerge" => U_MAILMERGE,
        "footnotePr" => U_FOOTNOTEPR,
        "endnotePr" => U_ENDNOTEPR,
        "txbxContent" => U_BLOCK | U_TXBX,
        "body" | "comment" | "docPartBody" | "endnote" | "footnote" | "ftr" | "hdr" | "tbl" | "tc" | "tr" | "sdtContent" | "customXml" => U_BLOCK,
        _ => 0,
    }
}

/// The value of an attribute as a parser delivers it, when it holds references only to predefined entities, characters
/// and plain-text internal entities of a resolved DOCTYPE subset (a bounded, non-recursive expansion); `None` otherwise.
fn delivered_attribute_value(raw: &[u8], decls: &BTreeMap<String, String>) -> Option<String> {
    observation_contract! {
        rules: "-";
        collections: "type.BTreeMap*1";
        scope: "one attribute value and the declarations of a resolved internal subset";
        loss: "a reference to anything but a predefined, character or plain internal entity; output bound";
        fallback: "return no value";
        package_loss: "independent";
    }

    let text = std::str::from_utf8(raw).ok()?;
    let mut out = String::new();
    let mut rest = text;
    while let Some(at) = rest.find('&') {
        out.push_str(rest.get(..at)?);
        let after = rest.get(at + 1..)?;
        let semi = after.find(';')?;
        let name = after.get(..semi)?;
        match name {
            "lt" => out.push('<'),
            "gt" => out.push('>'),
            "amp" => out.push('&'),
            "apos" => out.push('\''),
            "quot" => out.push('"'),
            _ => {
                let expansion = if let Some(number) = name.strip_prefix('#') {
                    let code = match number.strip_prefix(['x', 'X']) {
                        Some(hex) => u32::from_str_radix(hex, 16).ok()?,
                        None => number.parse().ok()?,
                    };
                    char::from_u32(code)?.to_string()
                } else {
                    decls.get(name).filter(|text| text.find(['&', '<']).is_none())?.clone()
                };
                out.push_str(&expansion);
            }
        }
        if out.len() > 8192 {
            return None;
        }
        rest = after.get(semi + 1..)?;
    }
    out.push_str(rest);
    Some(out)
}

/// Detect attribute prefixes that are undeclared or referenced in `mc:Ignorable` without a namespace declaration.
fn check_prefixes(
    declared: &[Vec<String>],
    attrs: &[Attribute],
    res: &NamespaceResolver,
    (extra, entities): (&[String], Option<&BTreeMap<String, String>>),
    out: &mut PartScan,
) {
    observation_contract! {
        rules: "-";
        collections: "field.unknown_prefixes*1,type.BTreeMap*1,type.Vec*1";
        scope: "source-local bounded extraction; no implicit document owner";
        loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
        fallback: "retain proven prefix presence and duplicates; return loss to the caller; no negative membership from an incomplete set";
        package_loss: "independent";
    }

    for a in attrs {
        match res.resolve_attribute(a.key).0 {
            ResolveResult::Unknown(p) => push_unknown_prefix(out, declared, extra, Some(&lossy(&p))),
            ResolveResult::Bound(Namespace(ns)) if ns == NS_MC.as_bytes() && a.key.local_name().as_ref() == b"Ignorable" => {
                // The value is what the parser delivers: an entity of a resolved subset is read as its replacement text.
                let delivered = a
                    .normalized_value(quick_xml::XmlVersion::Implicit1_0)
                    .ok()
                    .map(|v| v.into_owned())
                    .or_else(|| entities.and_then(|decls| delivered_attribute_value(&a.value, decls)));
                if let Some(v) = delivered {
                    for pfx in v.split_whitespace() {
                        if pfx != "xml" && nearest_binding(declared, extra, pfx) == Binding::Undeclared {
                            out.unknown_prefixes.push(pfx.to_string());
                        }
                    }
                } else {
                    out.ignorable_unread = true;
                }
            }
            _ => {}
        }
    }
}

/// What the nearest declaration of a prefix says about it.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Binding {
    /// No declaration in scope, or the nearest one is an empty reset: the prefix is not declared.
    Undeclared,
    /// The nearest declaration has a value.
    Resolved,
    /// The nearest declaration exists, but its value cannot be expanded: the prefix is unknown.
    Unresolved,
}

/// Decide what `pfx` is from its NEAREST declaration: the element's own (`extra`), then each
/// enclosing scope outward. Declarations further out never speak for it.
fn nearest_binding(declared: &[Vec<String>], extra: &[String], pfx: &str) -> Binding {
    observation_contract! {
        rules: "-";
        collections: "type.Vec*1";
        scope: "source-local bounded extraction; no implicit document owner";
        loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
        fallback: "retain proven prefix presence and duplicates; return loss to the caller; no negative membership from an incomplete set";
        package_loss: "independent";
    }

    let marked = |scope: &[String], mark: &str| scope.iter().any(|d| d.strip_prefix(mark) == Some(pfx));
    for scope in std::iter::once(extra).chain(declared.iter().rev().map(|scope| scope.as_slice())) {
        if marked(scope, UNRESOLVED_MARK) {
            return Binding::Unresolved;
        }
        if marked(scope, RESET_MARK) {
            return Binding::Undeclared;
        }
        if scope.iter().any(|d| d == pfx) {
            return Binding::Resolved;
        }
    }
    Binding::Undeclared
}

/// Extract XML namespace prefixes declared with `xmlns:prefix` in the given attributes.
fn declarations(attrs: &[Attribute]) -> Vec<String> {
    observation_contract! {
        rules: "-";
        collections: "type.Vec*2";
        scope: "source-local bounded extraction; no implicit document owner";
        loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
        fallback: "retain proven prefix presence and duplicates; return loss to the caller; no negative membership from an incomplete set";
        package_loss: "independent";
    }

    let mut v = Vec::new();
    for a in attrs {
        if let Some(p) = a.key.as_ref().strip_prefix(b"xmlns:")
            && !p.is_empty()
        {
            v.push(lossy(p));
            // The declaration exists but its value cannot be expanded: unknown, not undeclared. An empty
            // value un-declares the prefix for the scope it opens.
            match a.normalized_value(quick_xml::XmlVersion::Implicit1_0) {
                Err(_) => v.push(format!("{UNRESOLVED_MARK}{}", lossy(p))),
                Ok(value) if value.is_empty() => v.push(format!("{RESET_MARK}{}", lossy(p))),
                Ok(_) => {}
            }
        }
    }
    v
}

/// Marks a prefix, among a scope's declarations, whose declared value could not be resolved.
const UNRESOLVED_MARK: &str = "?";
/// Marks a prefix, among a scope's declarations, that an empty value un-declares. Neither mark can
/// begin an XML name, so a mark never collides with a prefix.
const RESET_MARK: &str = "!";

/// Record a prefix the resolver reports as unknown, unless its nearest declaration has a value that
/// cannot be resolved: that declaration is present, so the prefix is unknown, not "not declared".
/// The default namespace has no prefix to report: an unresolved default is unknown through element
/// resolution and never surfaces here (its empty prefix is not a diagnostic subject).
fn push_unknown_prefix(out: &mut PartScan, declared: &[Vec<String>], extra: &[String], prefix: Option<&str>) {
    observation_contract! {
        rules: "-";
        collections: "field.unknown_prefixes*1,type.Vec*1";
        scope: "source-local bounded extraction; no implicit document owner";
        loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
        fallback: "retain proven prefix presence and duplicates; return loss to the caller; no negative membership from an incomplete set";
        package_loss: "independent";
    }

    if let Some(prefix) = prefix
        && !prefix.is_empty()
        && nearest_binding(declared, extra, prefix) != Binding::Unresolved
    {
        out.unknown_prefixes.push(prefix.to_string());
    }
}

/// Resolve a qualified XML element name against the namespace resolver into an [`ElemId`].
fn resolve_elem<'a>(res: &'a NamespaceResolver, name: QName<'_>) -> ElemId<'a> {
    let (r, local) = crate::xml::resolve_element(res, name);
    let local = lossy(local.as_ref());
    match r {
        ResolveResult::Unknown(p) => ElemId { ns: None, local, unknown: Some(lossy(&p)), unresolved: false },
        ResolveResult::Bound(Namespace(ns)) => ElemId { ns: Some(ns), local, unknown: None, unresolved: false },
        ResolveResult::Unbound => ElemId { ns: None, local, unknown: None, unresolved: false },
    }
}

/// Record that an element's namespace cannot be resolved. Nothing in the part may then be reported
/// absent on its account, and a drawing reference may hide in it.
fn note_unresolved_element(state: &mut State, id: &mut ElemId, own: &[String]) {
    observation_contract! {
        rules: "-";
        collections: "field.declared*1";
        scope: "source-local bounded extraction; no implicit document owner";
        loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
        fallback: "retain proven prefix presence and duplicates; return loss to the caller; no negative membership from an incomplete set";
        package_loss: "independent";
    }

    id.unresolved = namespace_unresolved(id, &state.declared, own);
    if id.unresolved {
        // The local name is known whatever the namespace is, and only an element with the counterpart's local
        // name can be that counterpart: an unrelated unresolved element establishes and refutes nothing.
        state.unresolved_families |= pairing_family(&id.local);
        state.fld_unresolved_seen |= id.local == "fldChar";
        state.content_part_unresolved |= id.local == "contentPart";
        // A note definition or listing entry cannot be told from the unresolved element: the note join is incomplete.
        state.unresolved_note.0 |= id.local == "footnote";
        state.unresolved_note.1 |= id.local == "endnote";
        state.facts.invalidate_local(&id.local);
    }
}

/// Whether the element's namespace is unknown because its declaration cannot be expanded, as opposed to
/// undeclared. An unresolved default namespace reaches here as the empty prefix; a named prefix is
/// unresolved when its NEAREST declaration is.
fn namespace_unresolved(id: &ElemId, declared: &[Vec<String>], extra: &[String]) -> bool {
    observation_contract! {
        rules: "-";
        collections: "type.Vec*1";
        scope: "source-local bounded extraction; no implicit document owner";
        loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
        fallback: "retain proven prefix presence and duplicates; return loss to the caller; no negative membership from an incomplete set";
        package_loss: "independent";
    }

    id.unknown.as_deref().is_some_and(|prefix| prefix.is_empty() || nearest_binding(declared, extra, prefix) == Binding::Unresolved)
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

/// Check whether the text event consists entirely of XML whitespace characters.
pub(crate) fn is_whitespace(e: &quick_xml::events::BytesText) -> bool {
    e.as_ref().iter().all(|b| matches!(b, b' ' | b'\t' | b'\n' | b'\r'))
}

enum GeneralRefOutcome {
    Ok,
    Malformed(String),
    UnresolvedSubset,
    UncheckedReplacement,
}

fn has_unquoted_keyword(text: &str, keyword: &str) -> bool {
    let mut quote: Option<char> = None;
    let mut token = String::new();
    for ch in text.chars() {
        if let Some(q) = quote {
            if ch == q {
                quote = None;
            }
        } else if ch == '"' || ch == '\'' {
            if token == keyword {
                return true;
            }
            token.clear();
            quote = Some(ch);
        } else if ch.is_ascii_whitespace() {
            if token == keyword {
                return true;
            }
            token.clear();
        } else {
            token.push(ch);
        }
    }
    quote.is_none() && token == keyword
}

fn find_decl_end(bytes: &[u8], start: usize) -> Option<usize> {
    let mut quote: Option<u8> = None;
    let mut i = start;
    while let Some(&b) = bytes.get(i) {
        if let Some(q) = quote {
            if b == q {
                quote = None;
            }
        } else if b == b'"' || b == b'\'' {
            quote = Some(b);
        } else if b == b'>' {
            return Some(i);
        }
        i += 1;
    }
    None
}

/// Extract declared general entities and their replacement text from a part's DOCTYPE internal subset.
/// Returns an incomplete `Observed` when an external subset, parameter entity,
/// external general entity, or declaration budget prevents full resolution.
pub(crate) fn parse_doctype_entities(raw: &[u8], cap: usize) -> (Observed<HashSet<String>>, BTreeMap<String, String>) {
    observation_contract! {
        rules: "-";
        collections: "access.invalidate*2,parser.parse_doctype_entities*1,type.BTreeMap*5,type.HashSet*5,type.Observed*6";
        scope: "source-local bounded extraction; no implicit document owner";
        loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
        fallback: "retain proven prefix presence and duplicates; return loss to the caller; no negative membership from an incomplete set";
        package_loss: "independent";
    }

    let text = String::from_utf8_lossy(raw);
    let bytes = text.as_bytes();
    let mut quote: Option<u8> = None;
    let mut open_bracket = None;
    let mut i = 0;
    while let Some(&b) = bytes.get(i) {
        if let Some(q) = quote {
            if b == q {
                quote = None;
            }
        } else if b == b'"' || b == b'\'' {
            quote = Some(b);
        } else if b == b'[' {
            open_bracket = Some(i);
            break;
        }
        i += 1;
    }
    let header = open_bracket.and_then(|idx| text.get(..idx)).unwrap_or(text.as_ref());
    if quote.is_some() || has_unquoted_keyword(header, "SYSTEM") || has_unquoted_keyword(header, "PUBLIC") {
        let mut obs = Observed::complete(HashSet::new());
        obs.invalidate("external DOCTYPE subset");
        return (obs, BTreeMap::new());
    }
    let Some(open) = open_bracket else {
        return (Observed::complete(HashSet::new()), BTreeMap::new());
    };
    let Some(close) = text.rfind(']').filter(|&c| c > open) else {
        let mut obs = Observed::complete(HashSet::new());
        obs.invalidate("unterminated DOCTYPE internal subset");
        return (obs, BTreeMap::new());
    };
    let subset = text.get(open + 1..close).unwrap_or_default();
    let sb = subset.as_bytes();
    let mut set = HashSet::new();
    let mut decl_texts = BTreeMap::new();
    let mut unresolved = false;
    let mut pos = 0;
    while let Some(&b) = sb.get(pos) {
        if b.is_ascii_whitespace() {
            pos += 1;
            continue;
        }
        if sb.get(pos..pos + 4) == Some(b"<!--") {
            let Some(rel_end) = subset.get(pos + 4..).and_then(|s| s.find("-->")) else {
                unresolved = true;
                break;
            };
            pos += 4 + rel_end + 3;
            continue;
        }
        if sb.get(pos..pos + 2) == Some(b"<?") {
            let Some(rel_end) = subset.get(pos + 2..).and_then(|s| s.find("?>")) else {
                unresolved = true;
                break;
            };
            pos += 2 + rel_end + 2;
            continue;
        }
        if b == b'%' {
            unresolved = true;
            break;
        }
        if sb.get(pos..pos + 2) == Some(b"<!") {
            let Some(decl_end) = find_decl_end(sb, pos + 2) else {
                unresolved = true;
                break;
            };
            let decl = subset.get(pos + 2..decl_end).unwrap_or_default();
            if let Some(rest) = decl.strip_prefix("ENTITY")
                && rest.starts_with(|c: char| c.is_ascii_whitespace())
            {
                let Some((name, replacement)) = internal_general_entity(rest) else {
                    unresolved = true;
                    break;
                };
                if set.len() >= cap {
                    unresolved = true;
                    break;
                }
                set.insert(name.to_string());
                decl_texts.entry(name.to_string()).or_insert_with(|| replacement.to_string());
            } else {
                // Any declaration other than <!ENTITY> makes the subset unresolvable
                // (ATTLIST defaults, parameter entities, etc. are not modelled).
                unresolved = true;
                break;
            }
            pos = decl_end + 1;
            continue;
        }
        unresolved = true;
        break;
    }
    let declared =
        if unresolved { Observed::with_status(set, Completeness::Incomplete("unresolvable DOCTYPE subset")) } else { Observed::complete(set) };
    (declared, decl_texts)
}

/// Expand the internal general entities a part references in element CONTENT, so a part whose declarations are
/// spelled through an entity (`<!ENTITY item '<Relationship .../>'>` ... `&item;`) is read as expat delivers it.
/// `None` (the part is left as it is) when anything lies outside what this models: an external or parameter
/// entity, a declaration other than `<!ENTITY>`, a reference inside a tag, an undeclared entity, a replacement
/// text with a character reference, recursion, or an expansion past the bound.
pub(crate) fn expand_content_entities(text: &str) -> Option<String> {
    observation_contract! {
        rules: "-";
        collections: "access.is_complete*1,parser.parse_doctype_entities*1,type.Vec*1";
        scope: "source-local bounded extraction; no implicit document owner";
        loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
        fallback: "retain proven prefix presence and duplicates; return loss to the caller; no negative membership from an incomplete set";
        package_loss: "independent";
    }

    let mut reader = quick_xml::Reader::from_reader(text.as_bytes());
    let mut buf = Vec::new();
    let (entities, start, end) = loop {
        let at = usize::try_from(reader.buffer_position()).ok()?;
        match reader.read_event_into(&mut buf).ok()? {
            Event::DocType(e) => {
                let (declared, entities) = parse_doctype_entities(e.as_ref(), 4096);
                if !declared.is_complete() {
                    return None;
                }
                break (entities, at, usize::try_from(reader.buffer_position()).ok()?);
            }
            Event::Eof | Event::Start(_) | Event::Empty(_) => return None,
            _ => {}
        }
        buf.clear();
    };
    let mut out = String::from(text.get(..start)?);
    // Work bound: total bytes walked including intermediates, not just output size.
    let work_limit = text.len().saturating_mul(2).saturating_add(8192);
    let mut work_done = out.len();
    expand_into(text.get(end..)?, &entities, 0, work_limit, &mut work_done, &mut out)?;
    Some(out)
}

/// Append `src` with its entity references in content replaced by their replacement text (see
/// [`expand_content_entities`]); `None` when it cannot be done within the bounds.
/// `work_done` is incremented by the total bytes walked (source + intermediate expansions),
/// and must not exceed `work_limit`.
fn expand_into(
    src: &str,
    entities: &BTreeMap<String, String>,
    depth: usize,
    work_limit: usize,
    work_done: &mut usize,
    out: &mut String,
) -> Option<()> {
    observation_contract! {
        rules: "-";
        collections: "field.bytes*1,type.BTreeMap*1";
        scope: "source-local bounded extraction; no implicit document owner";
        loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
        fallback: "retain proven prefix presence and duplicates; return loss to the caller; no negative membership from an incomplete set";
        package_loss: "independent";
    }

    let predefined = |name: &str| name.starts_with('#') || matches!(name, "lt" | "gt" | "amp" | "apos" | "quot");
    if depth > 4 {
        return None;
    }
    let mut at = 0;
    while at < src.len() {
        let rest = src.get(at..)?;
        let span = if let Some(body) = rest.strip_prefix("<!--") {
            body.find("-->")? + 7
        } else if let Some(body) = rest.strip_prefix("<![CDATA[") {
            body.find("]]>")? + 12
        } else if let Some(body) = rest.strip_prefix("<?") {
            body.find("?>")? + 4
        } else if rest.starts_with('<') {
            // A tag: a reference in an attribute value is not modelled.
            let mut quote = None;
            let mut length = None;
            for (index, byte) in rest.bytes().enumerate().skip(1) {
                match (quote, byte) {
                    (None, b'>') => {
                        length = Some(index + 1);
                        break;
                    }
                    (None, b'"' | b'\'') => quote = Some(byte),
                    (Some(q), _) if q == byte => quote = None,
                    (Some(_), b'&') => {
                        let name = rest.get(index + 1..)?.split(';').next()?;
                        if !predefined(name) {
                            return None;
                        }
                    }
                    _ => {}
                }
            }
            length?
        } else if rest.starts_with('&') {
            let semi = rest.find(';')?;
            let name = rest.get(1..semi)?;
            if !predefined(name) {
                let value = entities.get(name)?;
                if value.contains('%') || value.contains("&#") {
                    return None;
                }
                // Charge work for the source reference and the replacement text.
                *work_done = work_done.saturating_add(semi + 1).saturating_add(value.len());
                if *work_done > work_limit {
                    return None;
                }
                expand_into(value, entities, depth + 1, work_limit, work_done, out)?;
                at += semi + 1;
                continue;
            }
            semi + 1
        } else {
            rest.find(['<', '&']).unwrap_or(rest.len())
        };
        out.push_str(rest.get(..span)?);
        // Charge work for bytes walked in source.
        *work_done = work_done.saturating_add(span);
        if *work_done > work_limit {
            return None;
        }
        at += span;
    }
    Some(())
}

/// Extract the name and replacement text of a literal internal general entity.
fn internal_general_entity(tail: &str) -> Option<(&str, &str)> {
    let tail = tail.trim_start();
    if tail.starts_with('%') {
        return None;
    }
    let (name, value) = tail.split_once(|c: char| c.is_ascii_whitespace())?;
    let (literal, trailing) = split_quoted_literal(value.trim_start())?;
    (!name.is_empty() && trailing.trim().is_empty()).then_some((name, literal))
}

/// Extract a complete quoted literal and the remaining declaration text.
fn split_quoted_literal(text: &str) -> Option<(&str, &str)> {
    let (quote, rest) = if let Some(rest) = text.strip_prefix('"') {
        ('"', rest)
    } else {
        let rest = text.strip_prefix('\'')?;
        ('\'', rest)
    };
    rest.split_once(quote)
}

/// Scan the next named general entity reference (`&name;`) starting at or after `offset`, returning
/// `(name, next_offset)` while skipping numeric character references (`&#...;`) and predefined XML entities.
fn next_named_entity_ref(text: &str, mut offset: usize) -> Option<(&str, usize)> {
    while let Some(rel_amp) = text.get(offset..)?.find('&') {
        let amp = offset + rel_amp;
        let after_amp = amp + 1;
        let tail = text.get(after_amp..)?;
        let token_len = tail.find(|c: char| c == '&' || c == '<' || c.is_ascii_whitespace()).unwrap_or(tail.len());
        let Some(rel_semi) = tail.get(..token_len)?.find(';') else {
            offset = after_amp;
            continue;
        };
        let end = after_amp + rel_semi;
        let candidate = text.get(after_amp..end)?;
        offset = end + 1;
        if !candidate.is_empty()
            && !candidate.starts_with('#')
            && !candidate.chars().any(|c| c.is_ascii_whitespace() || c == '&' || c == '<')
            && !matches!(candidate, "lt" | "gt" | "amp" | "apos" | "quot")
        {
            return Some((candidate, end + 1));
        }
    }
    None
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum EntityValueStatus {
    Plain,
    Unchecked,
    Recursive,
}

/// Check an entity's replacement text for direct/indirect recursion and unexpanded markup/references,
/// memoizing verified acyclic nodes and charging a shared per-part work budget.
fn check_entity_value(
    start: &str,
    decl_texts: &BTreeMap<String, String>,
    entity_status: &mut HashMap<String, EntityValueStatus>,
    work_remaining: &mut usize,
) -> EntityValueStatus {
    observation_contract! {
        rules: "-";
        collections: "type.BTreeMap*1,type.HashMap*3,type.Vec*1";
        scope: "source-local bounded extraction; no implicit document owner";
        loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
        fallback: "retain proven prefix presence and duplicates; return loss to the caller; no negative membership from an incomplete set";
        package_loss: "independent";
    }

    if let Some(cached) = entity_status.get(start) {
        return *cached;
    }
    let Some((start_key, _)) = decl_texts.get_key_value(start) else {
        return EntityValueStatus::Unchecked;
    };
    let mut stack: Vec<(&str, usize)> = vec![(start_key.as_str(), 0)];
    let mut visit_state: HashMap<&str, bool> = HashMap::from([(start_key.as_str(), true)]);
    while let Some((cur, offset)) = stack.last_mut() {
        let Some(val) = decl_texts.get(*cur) else {
            visit_state.insert(*cur, false);
            stack.pop();
            continue;
        };
        if *work_remaining == 0 {
            return EntityValueStatus::Unchecked;
        }
        *work_remaining -= 1;
        if let Some((next, next_offset)) = next_named_entity_ref(val, *offset) {
            *offset = next_offset;
            match visit_state.get(next).copied() {
                Some(true) => return EntityValueStatus::Recursive,
                Some(false) => {}
                None => {
                    if let Some(cached) = entity_status.get(next) {
                        if *cached == EntityValueStatus::Recursive {
                            return EntityValueStatus::Recursive;
                        }
                        visit_state.insert(next, false);
                    } else if let Some((next_key, _)) = decl_texts.get_key_value(next) {
                        visit_state.insert(next_key.as_str(), true);
                        stack.push((next_key.as_str(), 0));
                    }
                }
            }
        } else {
            let finished = *cur;
            stack.pop();
            visit_state.insert(finished, false);
            let status = if val.find(['<', '&']).is_some() { EntityValueStatus::Unchecked } else { EntityValueStatus::Plain };
            entity_status.insert(finished.to_string(), status);
        }
    }
    entity_status.get(start).copied().unwrap_or(EntityValueStatus::Unchecked)
}

/// Check a general entity reference: numeric character references for `XML-INVALID-CHAR`, and named
/// references against the five predefined XML 1.0 entities and complete DOCTYPE internal subset declarations.
fn scan_general_ref(
    part: &str,
    content: &[u8],
    entities: Option<&CompleteSet<String>>,
    decl_texts: &BTreeMap<String, String>,
    entity_status: &mut HashMap<String, EntityValueStatus>,
    work_remaining: &mut usize,
    acc: &mut Acc,
) -> GeneralRefOutcome {
    observation_contract! {
        rules: "XML-INVALID-CHAR";
        collections: "access.missing*1,field.first*1,type.BTreeMap*1,type.CompleteSet*1,type.HashMap*1";
        scope: "source-local bounded extraction; no implicit document owner";
        loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
        fallback: "retain proven prefix presence and duplicates; return loss to the caller; no negative membership from an incomplete set";
        package_loss: "independent";
    }

    let body = String::from_utf8_lossy(content);
    if content.first() == Some(&b'#') {
        if let Some(desc) = bad_char_ref(&format!("&{body};")) {
            acc.add("XML-INVALID-CHAR", Some(part.to_string()), format!("{part} contains {desc}, which is illegal in XML 1.0"));
        }
        return GeneralRefOutcome::Ok;
    }
    if matches!(body.as_ref(), "lt" | "gt" | "amp" | "apos" | "quot") {
        return GeneralRefOutcome::Ok;
    }
    let name = body.into_owned();
    if decl_texts.get(&name).is_some() {
        return match check_entity_value(&name, decl_texts, entity_status, work_remaining) {
            EntityValueStatus::Plain => GeneralRefOutcome::Ok,
            EntityValueStatus::Unchecked => GeneralRefOutcome::UncheckedReplacement,
            EntityValueStatus::Recursive => GeneralRefOutcome::Malformed(format!("recursive entity reference `&{name};`")),
        };
    }
    let Some(declared) = entities else {
        return GeneralRefOutcome::UnresolvedSubset;
    };
    if declared.missing(&name) {
        return GeneralRefOutcome::Malformed(format!("undefined entity reference `&{name};`"));
    }
    GeneralRefOutcome::Ok
}

/// `XML-INVALID-CHAR` for an illegal character reference inside any attribute value.
fn scan_attr_refs(part: &str, attrs: &[Attribute], acc: &mut Acc) {
    observation_contract! {
        rules: "XML-INVALID-CHAR";
        collections: "-";
        scope: "source-local bounded extraction; no implicit document owner";
        loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
        fallback: "retain proven prefix presence and duplicates; return loss to the caller; no negative membership from an incomplete set";
        package_loss: "independent";
    }

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
    observation_contract! {
        rules: "-";
        collections: "field.first*1";
        scope: "source-local bounded extraction; no implicit document owner";
        loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
        fallback: "retain proven prefix presence and duplicates; return loss to the caller; no negative membership from an incomplete set";
        package_loss: "independent";
    }

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
    pub num_ids: Vec<DecimalId>,
    pub abstract_ids: Vec<DecimalId>,
    /// Each `w:num`'s `numId` and the `w:abstractNumId` it references, if any.
    pub num_to_abstract: Vec<(DecimalId, Option<DecimalId>)>,
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
    Process(ElemId<'a>, Vec<Attribute<'a>>),
}

/// Resolve an open tag for an auxiliary parser: apply the depth bound (H1/J1), the AlternateContent
/// suppression (H3/J5), and read its attributes. `None` means the attribute list is malformed, which
/// the caller treats exactly as a parse failure. `Stop` means the element crossed the depth limit;
/// the caller must stop before reading another event and must not process the element.
fn aux_open<'a>(
    guard: &mut ParseGuard,
    alt: &mut AltSuppress,
    res: &'a NamespaceResolver,
    e: &'a BytesStart<'_>,
    empty: bool,
) -> Option<AuxOpen<'a>> {
    if !empty && !guard.enter() {
        return Some(AuxOpen::Stop);
    }
    let id = resolve_elem(res, e.name());
    if alt.open(&id, empty) {
        return Some(AuxOpen::Skip);
    }
    let watched = guard.watch.iter().any(|w| *w == id.local);
    guard.unresolved |= id.unknown.is_some() && watched;
    let attrs = attrs_of(e)?;
    // The identifier of a definition is a join input: one this parse cannot read (an unresolved prefix, or a value
    // spelled through an entity) is an unknown definition, never an absent one.
    guard.unresolved |= watched
        && id.w()
        && attrs.iter().any(|a| {
            a.key.as_namespace_binding().is_none()
                && (matches!(res.resolve_attribute(a.key).0, ResolveResult::Unknown(_))
                    || a.normalized_value(quick_xml::XmlVersion::Implicit1_0).is_err())
        });
    Some(AuxOpen::Process(id, attrs))
}

/// Parse a numbering part. `None` when it does not parse. `guard.exhausted()` marks the result
/// partial (H1).
pub(crate) fn parse_numbering(text: &str, cap: usize, guard: &mut ParseGuard) -> Option<NumberingDefs> {
    observation_contract! {
        rules: "NUM-DANGLING";
        collections: "parser.parse_numbering*1,type.Vec*3";
        scope: "source-local bounded extraction; no implicit document owner";
        loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
        fallback: "retain proven prefix presence and duplicates; return loss to the caller; no negative membership from an incomplete set";
        package_loss: "independent";
    }

    let mut reader = XmlReader::new(text);
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
    observation_contract! {
        rules: "-";
        collections: "field.abstract_ids*2,field.num_ids*2,field.num_to_abstract*3";
        scope: "source-local bounded extraction; no implicit document owner";
        loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
        fallback: "retain proven prefix presence and duplicates; return loss to the caller; no negative membership from an incomplete set";
        package_loss: "independent";
    }

    if !id.w() {
        return false;
    }
    match id.local.as_str() {
        "abstractNum" => {
            if let Some(x) = attr_ns(attrs, res, NS_W, b"abstractNumId").map(DecimalId::parse) {
                if defs.abstract_ids.len() >= cap {
                    defs.capped = true;
                } else {
                    defs.abstract_ids.push(x);
                }
            }
            false
        }
        "num" => {
            if let Some(x) = attr_ns(attrs, res, NS_W, b"numId").map(DecimalId::parse) {
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
            if let (Some(i), Some(v)) = (*in_num, attr_ns(attrs, res, NS_W, b"val").map(DecimalId::parse))
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
pub(crate) fn parse_style_num_ids(text: &str, cap: usize, guard: &mut ParseGuard) -> Option<(Vec<DecimalId>, bool)> {
    observation_contract! {
        rules: "NUM-DANGLING";
        collections: "parser.parse_style_num_ids*1,type.Vec*3";
        scope: "source-local bounded extraction; no implicit document owner";
        loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
        fallback: "retain proven prefix presence and duplicates; return loss to the caller; no negative membership from an incomplete set";
        package_loss: "independent";
    }

    let mut reader = XmlReader::new(text);
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
fn push_style_num_id(out: &mut Vec<DecimalId>, capped: &mut bool, id: &ElemId, attrs: &[Attribute], res: &NamespaceResolver, cap: usize) {
    observation_contract! {
        rules: "-";
        collections: "type.Vec*1";
        scope: "source-local bounded extraction; no implicit document owner";
        loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
        fallback: "retain proven prefix presence and duplicates; return loss to the caller; no negative membership from an incomplete set";
        package_loss: "independent";
    }

    if id.w()
        && id.local == "numId"
        && let Some(v) = attr_ns(attrs, res, NS_W, b"val").map(DecimalId::parse)
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
pub(crate) fn parse_style_ids(text: &str, cap: usize, guard: &mut ParseGuard) -> Option<(HashSet<StyleId>, bool)> {
    observation_contract! {
        rules: "REF-STYLE";
        collections: "parser.parse_style_ids*1,type.HashSet*2,type.Vec*1";
        scope: "source-local bounded extraction; no implicit document owner";
        loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
        fallback: "retain proven prefix presence and duplicates; return loss to the caller; no negative membership from an incomplete set";
        package_loss: "independent";
    }

    let mut reader = XmlReader::new(text);
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

/// One `w:style` definition from a styles part: its id, primary name
/// (the first `w:name` child's value), and declared type, as written.
#[derive(Default)]
pub(crate) struct StyleDef {
    /// The `w:style/@w:styleId` value, if present.
    pub id: Option<StyleId>,
    /// The first `w:name/@w:val` inside the style, if present.
    pub name: Option<String>,
    /// Whether a `w:name` child was seen at all: the FIRST name
    /// element owns the primary-name slot even when it carries no
    /// value, so a later sibling never replaces it.
    pub name_seen: bool,
    /// The `w:style/@w:type` value, if present.
    pub style_type: Option<String>,
}

/// The style definitions of a styles part, for the vocabulary rules:
/// the definitions (capped at `cap`), whether the cap dropped any,
/// and the total number of `w:style` elements seen. `None` when the
/// part does not parse. Definitions inside a skipped
/// `mc:AlternateContent` branch are not collected.
pub(crate) fn parse_style_defs(text: &str, cap: usize, guard: &mut ParseGuard) -> Option<(Vec<StyleDef>, bool, usize)> {
    observation_contract! {
        rules: "-";
        collections: "parser.parse_style_defs*1,type.Vec*4";
        scope: "source-local bounded extraction; no implicit document owner";
        loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
        fallback: "retain proven prefix presence and duplicates; return loss to the caller; no negative membership from an incomplete set";
        package_loss: "independent";
    }

    let mut reader = XmlReader::new(text);
    let mut buf = Vec::new();
    let mut defs: Vec<StyleDef> = Vec::new();
    let mut capped = false;
    let mut total = 0usize;
    // Frame discipline: a definition opens only for a `w:style` that is
    // a direct child of the part root (depth 2), and only its own
    // matching end tag closes it. A nested or foreign-namespace element
    // also named `style`, and anything inside a suppressed
    // AlternateContent branch, can neither open nor close a definition,
    // and a `w:name` counts only as the open style's direct child
    // (depth 3) - so names are never transferred between styles.
    let mut depth = 0usize;
    let mut current: Option<StyleDef> = None;
    let mut alt = AltSuppress::new();
    loop {
        if !guard.tick() {
            break;
        }
        match reader.read_event_into(&mut buf) {
            Err(_) => return None,
            Ok(Event::Eof) => {
                // An unclosed style, or any unbalanced frame, means the
                // part did not parse: report failure rather than
                // silently discarding the unfinished definition.
                if current.is_some() || depth != 0 {
                    return None;
                }
                break;
            }
            Ok(Event::Start(e)) => {
                depth += 1;
                match aux_open(guard, &mut alt, reader.resolver(), &e, false) {
                    None => return None,
                    Some(AuxOpen::Stop) => break,
                    Some(AuxOpen::Skip) => {}
                    Some(AuxOpen::Process(id, attrs)) => {
                        if id.w() && id.local == "style" && depth == 2 {
                            total += 1;
                            current = Some(StyleDef {
                                id: attr_ns(&attrs, reader.resolver(), NS_W, b"styleId").map(StyleId::from),
                                style_type: attr_ns(&attrs, reader.resolver(), NS_W, b"type"),
                                ..StyleDef::default()
                            });
                        } else if id.w()
                            && id.local == "name"
                            && depth == 3
                            && let Some(def) = current.as_mut()
                            && !def.name_seen
                        {
                            def.name_seen = true;
                            def.name = attr_ns(&attrs, reader.resolver(), NS_W, b"val");
                        }
                    }
                }
            }
            Ok(Event::Empty(e)) => {
                depth += 1;
                let outcome = aux_open(guard, &mut alt, reader.resolver(), &e, true);
                depth -= 1;
                match outcome {
                    None => return None,
                    Some(AuxOpen::Stop) => break,
                    Some(AuxOpen::Skip) => {}
                    Some(AuxOpen::Process(id, attrs)) => {
                        if id.w() && id.local == "style" && depth + 1 == 2 {
                            total += 1;
                            if defs.len() >= cap {
                                capped = true;
                            } else {
                                defs.push(StyleDef {
                                    id: attr_ns(&attrs, reader.resolver(), NS_W, b"styleId").map(StyleId::from),
                                    style_type: attr_ns(&attrs, reader.resolver(), NS_W, b"type"),
                                    ..StyleDef::default()
                                });
                            }
                        } else if id.w()
                            && id.local == "name"
                            && depth + 1 == 3
                            && let Some(def) = current.as_mut()
                            && !def.name_seen
                        {
                            def.name_seen = true;
                            def.name = attr_ns(&attrs, reader.resolver(), NS_W, b"val");
                        }
                    }
                }
            }
            Ok(Event::End(e)) => {
                let id = resolve_elem(reader.resolver(), e.name());
                alt.close(e.name().local_name().as_ref());
                guard.leave();
                if id.w()
                    && id.local == "style"
                    && depth == 2
                    && let Some(def) = current.take()
                {
                    if defs.len() >= cap {
                        capped = true;
                    } else {
                        defs.push(def);
                    }
                }
                depth = depth.saturating_sub(1);
            }
            _ => {}
        }
        buf.clear();
    }
    Some((defs, capped, total))
}

/// The `w:font/@w:name` values of a fontTable part (capped), or
/// `None` when the part does not parse.
pub(crate) fn parse_font_names(text: &str, cap: usize, guard: &mut ParseGuard) -> Option<(Vec<String>, bool)> {
    observation_contract! {
        rules: "-";
        collections: "parser.parse_font_names*1,parser.parse_part_names*1,type.Vec*1";
        scope: "source-local bounded extraction; no implicit document owner";
        loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
        fallback: "retain proven prefix presence and duplicates; return loss to the caller; no negative membership from an incomplete set";
        package_loss: "independent";
    }

    parse_part_names(text, cap, guard, NS_W, "font", true)
}

/// The `property/@name` values of a custom-properties part (capped),
/// or `None` when the part does not parse. The elements live in the
/// custom-properties namespace and the attribute is unprefixed.
pub(crate) fn parse_custom_prop_names(text: &str, cap: usize, guard: &mut ParseGuard) -> Option<(Vec<String>, bool)> {
    observation_contract! {
        rules: "-";
        collections: "parser.parse_custom_prop_names*1,parser.parse_part_names*1,type.Vec*1";
        scope: "source-local bounded extraction; no implicit document owner";
        loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
        fallback: "retain proven prefix presence and duplicates; return loss to the caller; no negative membership from an incomplete set";
        package_loss: "independent";
    }

    parse_part_names(text, cap, guard, NS_CUSTOM_PROPS, "property", false)
}

/// The custom-properties namespace (docProps/custom.xml).
const NS_CUSTOM_PROPS: &str = "http://schemas.openxmlformats.org/officeDocument/2006/custom-properties";

/// Shared walker for the two name-list parts: collect the name
/// attribute of every `elem` element in namespace `ns` — as a
/// `w:`-style namespaced attribute when `namespaced_attr`, else as
/// the unprefixed attribute of the same local name.
fn parse_part_names(text: &str, cap: usize, guard: &mut ParseGuard, ns: &str, elem: &str, namespaced_attr: bool) -> Option<(Vec<String>, bool)> {
    observation_contract! {
        rules: "-";
        collections: "parser.parse_part_names*1,type.Vec*5";
        scope: "source-local bounded extraction; no implicit document owner";
        loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
        fallback: "retain proven prefix presence and duplicates; return loss to the caller; no negative membership from an incomplete set";
        package_loss: "independent";
    }

    let mut reader = XmlReader::new(text);
    let mut buf = Vec::new();
    let mut names: Vec<String> = Vec::new();
    let mut capped = false;
    let mut alt = AltSuppress::new();
    // Raw frame depth, so an unbalanced (truncated) part reports a
    // parse failure at EOF instead of a partial name list as success.
    let mut depth = 0usize;
    loop {
        if !guard.tick() {
            break;
        }
        let handle = |id: &ElemId, attrs: &[Attribute], res: &NamespaceResolver, names: &mut Vec<String>, capped: &mut bool| {
            if id.ns == Some(ns.as_bytes()) && id.local == elem {
                let value = if namespaced_attr { attr_ns(attrs, res, NS_W, b"name") } else { attr_plain(attrs, b"name") };
                if let Some(v) = value {
                    if names.len() >= cap {
                        *capped = true;
                    } else {
                        names.push(v);
                    }
                }
            }
        };
        match reader.read_event_into(&mut buf) {
            Err(_) => return None,
            Ok(Event::Eof) => {
                if depth != 0 {
                    return None;
                }
                break;
            }
            Ok(Event::Start(e)) => {
                depth += 1;
                match aux_open(guard, &mut alt, reader.resolver(), &e, false) {
                    None => return None,
                    Some(AuxOpen::Stop) => break,
                    Some(AuxOpen::Skip) => {}
                    Some(AuxOpen::Process(id, attrs)) => handle(&id, &attrs, reader.resolver(), &mut names, &mut capped),
                }
            }
            Ok(Event::Empty(e)) => match aux_open(guard, &mut alt, reader.resolver(), &e, true) {
                None => return None,
                Some(AuxOpen::Stop) => break,
                Some(AuxOpen::Skip) => {}
                Some(AuxOpen::Process(id, attrs)) => handle(&id, &attrs, reader.resolver(), &mut names, &mut capped),
            },
            Ok(Event::End(e)) => {
                alt.close(e.name().local_name().as_ref());
                guard.leave();
                depth = depth.saturating_sub(1);
            }
            _ => {}
        }
        buf.clear();
    }
    Some((names, capped))
}

/// Extract and insert a `w:style/@w:styleId` definition into the set, respecting the capacity cap.
fn push_style_id(defs: &mut HashSet<StyleId>, capped: &mut bool, id: &ElemId, attrs: &[Attribute], res: &NamespaceResolver, cap: usize) {
    observation_contract! {
        rules: "-";
        collections: "type.HashSet*1";
        scope: "source-local bounded extraction; no implicit document owner";
        loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
        fallback: "retain proven prefix presence and duplicates; return loss to the caller; no negative membership from an incomplete set";
        package_loss: "independent";
    }

    if id.w()
        && id.local == "style"
        && let Some(v) = attr_ns(attrs, res, NS_W, b"styleId")
    {
        if defs.len() >= cap {
            *capped = true;
        } else {
            defs.insert(v.into());
        }
    }
}

/// The `w:attachedSchema/@w:val` values of a settings part. `None` when it does not parse.
pub(crate) fn parse_attached_schemas(text: &str, cap: usize, guard: &mut ParseGuard) -> Option<(Vec<String>, bool)> {
    observation_contract! {
        rules: "CUSTOMXML-SCHEMA";
        collections: "parser.parse_attached_schemas*1,type.Vec*3";
        scope: "source-local bounded extraction; no implicit document owner";
        loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
        fallback: "retain proven prefix presence and duplicates; return loss to the caller; no negative membership from an incomplete set";
        package_loss: "independent";
    }

    let mut reader = XmlReader::new(text);
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
    observation_contract! {
        rules: "-";
        collections: "type.Vec*1";
        scope: "source-local bounded extraction; no implicit document owner";
        loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
        fallback: "retain proven prefix presence and duplicates; return loss to the caller; no negative membership from an incomplete set";
        package_loss: "independent";
    }

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
    observation_contract! {
        rules: "CUSTOMXML-SCHEMA";
        collections: "parser.parse_custom_xml_schemas*1,type.Vec*4";
        scope: "source-local bounded extraction; no implicit document owner";
        loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
        fallback: "retain proven prefix presence and duplicates; return loss to the caller; no negative membership from an incomplete set";
        package_loss: "independent";
    }

    let mut reader = XmlReader::new(text);
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
    observation_contract! {
        rules: "-";
        collections: "type.Vec*1";
        scope: "source-local bounded extraction; no implicit document owner";
        loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
        fallback: "retain proven prefix presence and duplicates; return loss to the caller; no negative membership from an incomplete set";
        package_loss: "independent";
    }

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
    pub entries: Vec<(HexId, Option<HexId>)>,
    /// The per-category budget was exhausted while parsing.
    pub capped: bool,
}

/// Parse a commentsExtended part. `None` when it does not parse.
pub(crate) fn parse_comments_extended(text: &str, cap: usize, guard: &mut ParseGuard) -> Option<CommentEx> {
    observation_contract! {
        rules: "COMMENTEX-LINK";
        collections: "parser.parse_comments_extended*1,type.Vec*1";
        scope: "source-local bounded extraction; no implicit document owner";
        loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
        fallback: "retain proven prefix presence and duplicates; return loss to the caller; no negative membership from an incomplete set";
        package_loss: "independent";
    }

    let mut reader = XmlReader::new(text);
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
    observation_contract! {
        rules: "-";
        collections: "field.entries*2";
        scope: "source-local bounded extraction; no implicit document owner";
        loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
        fallback: "retain proven prefix presence and duplicates; return loss to the caller; no negative membership from an incomplete set";
        package_loss: "independent";
    }

    if id.local_w(NS_W15)
        && id.local == "commentEx"
        && let Some(para) = attr_ns(attrs, res, NS_W15, b"paraId")
    {
        let para = HexId::parse(para);
        let parent = attr_ns(attrs, res, NS_W15, b"paraIdParent").map(HexId::parse);
        if out.entries.len() >= cap {
            out.capped = true;
        } else {
            out.entries.push((para, parent));
        }
    }
}

/// The `w16cid:commentId/@w16cid:paraId` values of a commentsIds part. `None` when it does not
/// parse.
pub(crate) fn parse_comments_ids(text: &str, cap: usize, guard: &mut ParseGuard) -> Option<(Vec<HexId>, bool)> {
    observation_contract! {
        rules: "COMMENTEX-LINK";
        collections: "parser.parse_comments_ids*1,type.Vec*3";
        scope: "source-local bounded extraction; no implicit document owner";
        loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
        fallback: "retain proven prefix presence and duplicates; return loss to the caller; no negative membership from an incomplete set";
        package_loss: "independent";
    }

    let mut reader = XmlReader::new(text);
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
fn push_comment_id(out: &mut Vec<HexId>, capped: &mut bool, id: &ElemId, attrs: &[Attribute], res: &NamespaceResolver, cap: usize) {
    observation_contract! {
        rules: "-";
        collections: "type.Vec*1";
        scope: "source-local bounded extraction; no implicit document owner";
        loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
        fallback: "retain proven prefix presence and duplicates; return loss to the caller; no negative membership from an incomplete set";
        package_loss: "independent";
    }

    if id.local_w(NS_W16CID)
        && id.local == "commentId"
        && let Some(v) = attr_ns(attrs, res, NS_W16CID, b"paraId")
    {
        if out.len() >= cap {
            *capped = true;
        } else {
            out.push(HexId::parse(v));
        }
    }
}

/// An element's attributes, or `None` when the attribute list is malformed (a duplicate or an
/// unterminated value).
fn attrs_of<'a>(e: &'a BytesStart<'_>) -> Option<Vec<Attribute<'a>>> {
    observation_contract! {
        rules: "-";
        collections: "type.Vec*2";
        scope: "source-local bounded extraction; no implicit document owner";
        loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
        fallback: "retain proven prefix presence and duplicates; return loss to the caller; no negative membership from an incomplete set";
        package_loss: "independent";
    }

    let mut attrs = Vec::new();
    for a in e.attributes() {
        attrs.push(a.ok()?);
    }
    Some(attrs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_conflict_deep_paths_compare_linearly_and_branch_zero_conflicts() {
        let len = 100_000u32;
        let a: ParaPath = (0..len).map(|i| (i * 2 + 1, if i + 1 == len { 0 } else { 1 })).collect();
        let b: ParaPath = (0..len).map(|i| (i * 2 + 2, 2)).collect();
        let start = std::time::Instant::now();
        assert!(paths_conflict(&a, &b));
        let mut disjoint_b = a.clone();
        disjoint_b[len as usize - 2].1 = 2;
        assert!(!paths_conflict(&a, &disjoint_b));
        let mut branch_zero_pair = a.clone();
        branch_zero_pair[len as usize - 1].1 = 1;
        assert!(paths_conflict(&a, &branch_zero_pair), "branch 0 in either occurrence does not excuse a collision");
        assert!(start.elapsed() < std::time::Duration::from_secs(1), "deep path comparison took {:?}", start.elapsed());
    }

    #[test]
    fn entity_replacement_with_markup_or_refs_marks_syntax_and_facts_incomplete() {
        let mut acc = Acc::with_limits(crate::Limits::default());
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?><!DOCTYPE w:document [<!ENTITY m "<w:p/>">]><w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:r><w:t>&m;</w:t></w:r></w:p></w:body></w:document>"#;
        let scan = scan_part("word/document.xml", xml, PartRole::Main, &mut acc);
        assert!(scan.malformed.is_none());
        assert!(!scan.syntax_complete);
        assert!(!scan.facts_complete);
        assert!(!scan.facts.bookmarks.is_complete());
    }

    #[test]
    fn unchecked_entity_replacement_suppresses_later_extra_field_end_but_keeps_earlier() {
        // Entity supplies begin, visible markup supplies end -> no FLD-UNBALANCED, verdict Incomplete.
        let mut acc = Acc::with_limits(crate::Limits::default());
        let balanced_via_entity = r#"<?xml version="1.0" encoding="UTF-8"?><!DOCTYPE w:document [<!ENTITY b "<w:fldChar w:fldCharType='begin'/>">]><w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:r>&b;<w:fldChar w:fldCharType="end"/></w:r></w:p></w:body></w:document>"#;
        let _ = scan_part("word/document.xml", balanced_via_entity, PartRole::Main, &mut acc);
        let report = acc.finish();
        assert_eq!(report.verdict, crate::Verdict::Incomplete, "{report:?}");
        assert!(!report.findings.iter().any(|f| f.rule == "FLD-UNBALANCED"), "{report:?}");

        // Extra end BEFORE unchecked replacement stays proven (1 extra end, not 2).
        let mut acc2 = Acc::with_limits(crate::Limits::default());
        let earlier_and_later = r#"<?xml version="1.0" encoding="UTF-8"?><!DOCTYPE w:document [<!ENTITY b "<w:fldChar w:fldCharType='begin'/>">]><w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:r><w:fldChar w:fldCharType="end"/>&b;<w:fldChar w:fldCharType="end"/></w:r></w:p></w:body></w:document>"#;
        let _ = scan_part("word/document.xml", earlier_and_later, PartRole::Main, &mut acc2);
        let report2 = acc2.finish();
        let f = report2.findings.iter().find(|f| f.rule == "FLD-UNBALANCED").expect("earlier extra end survives");
        assert!(f.message.contains("1 w:fldChar end(s)"), "unexpected count: {f:?}");
    }

    #[test]
    fn entity_chain_memoization_and_work_budget_scale_linearly() {
        let mut decls = String::new();
        for i in 0..9_999usize {
            decls.push_str(&format!(r#"<!ENTITY e{i} "&e{};">"#, i + 1));
        }
        decls.push_str(r#"<!ENTITY e9999 "x">"#);
        let refs = "&e0;".repeat(1_000);
        let xml = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?><!DOCTYPE w:document [{decls}]><w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:r><w:t>{refs}</w:t></w:r></w:p></w:body></w:document>"#
        );
        let start = std::time::Instant::now();
        let mut acc = Acc::with_limits(crate::Limits::default());
        let scan = scan_part("word/document.xml", &xml, PartRole::Main, &mut acc);
        assert!(scan.malformed.is_none(), "{:?}", scan.malformed);
        assert!(start.elapsed() < std::time::Duration::from_secs(2), "entity traversal took {:?}", start.elapsed());

        // Exhausted work budget yields Unchecked (Incomplete) rather than unbounded work or false pass.
        let mut tiny_decls = BTreeMap::new();
        for i in 0..20usize {
            tiny_decls.insert(format!("k{i}"), format!("&k{};", i + 1));
        }
        tiny_decls.insert("k20".to_string(), "ok".to_string());
        let mut cache = HashMap::new();
        let mut budget = 5usize;
        assert!(matches!(check_entity_value("k0", &tiny_decls, &mut cache, &mut budget), EntityValueStatus::Unchecked));
        assert_eq!(budget, 0);
    }

    #[test]
    fn next_named_entity_ref_repeated_ampersand_scales_linearly() {
        let text = format!("{}&target;", "& ".repeat(50_000));
        let start = std::time::Instant::now();
        let found = next_named_entity_ref(&text, 0);
        assert_eq!(found, Some(("target", text.len())));
        assert!(start.elapsed() < std::time::Duration::from_millis(200), "scanning took {:?}", start.elapsed());
    }
}
