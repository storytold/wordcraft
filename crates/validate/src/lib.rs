//! Structural integrity validation for WordCraft OOXML (`.docx`) packages.
//!
//! [`validate_bytes`] examines a `.docx` package as bytes and returns a [`Report`] of every
//! problem it can prove from the package alone: container and content-type declarations,
//! relationship targets and element-side references, XML well-formedness, illegal characters and
//! namespace prefixes, story-part reference and id spaces (comments, bookmarks, footnotes, styles,
//! numbering, comment threading, custom XML), field/table/markup structure, and media magic bytes.
//! It never panics and never fails: a document that is not a zip at all becomes a `PKG-NOT-ZIP`
//! finding.
//!
//! The checks follow the public OPC and WordprocessingML specifications (ECMA-376 / ISO/IEC
//! 29500); they are written from the specification, and rule ids are stable so callers can rely
//! on them. Every rule's severity and evidence grade is recorded once, in [`RULES`], next to the
//! provenance it was calibrated against.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

// Step 0 of the grammar work: the tables land before any check reads them. This allow is
// removed when the first grammar rule uses the module.
#[allow(dead_code)]
mod model;
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod model_tests;
mod package;
mod reserved;
mod scan;

use std::collections::HashMap;
use std::collections::HashSet;
use std::collections::hash_map::Entry;
use std::io::Read;
use std::path::Path;

use serde::Serialize;

use package::{Package, RelsPart, canonical_entry, entry_by_name, rels_part_name, tail};
use scan::{DecodeError, RefFact};

/// The WordprocessingML main namespace.
pub(crate) const NS_W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
/// The officeDocument relationships namespace.
pub(crate) const NS_R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
/// The DrawingML wordprocessingDrawing namespace.
pub(crate) const NS_WP: &str = "http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing";
/// The markup-compatibility namespace.
pub(crate) const NS_MC: &str = "http://schemas.openxmlformats.org/markup-compatibility/2006";
/// The Office Math namespace (`m:oMath` and friends).
pub(crate) const NS_M: &str = "http://schemas.openxmlformats.org/officeDocument/2006/math";
/// The Word 2010 wordml namespace (`w14:paraId`).
pub(crate) const NS_W14: &str = "http://schemas.microsoft.com/office/word/2010/wordml";
/// The Word 2012 wordml namespace (`w15:commentEx`).
pub(crate) const NS_W15: &str = "http://schemas.microsoft.com/office/word/2012/wordml";
/// The Word 2016 comment-id namespace (`w16cid:commentId`).
pub(crate) const NS_W16CID: &str = "http://schemas.microsoft.com/office/word/2016/wordml/cid";
/// The OPC custom-XML datastore namespace (`ds:schemaRef`).
pub(crate) const NS_DS: &str = "http://schemas.openxmlformats.org/officeDocument/2006/customXml";

/// Resource budgets shared by every check. Exhausting a budget stops the affected check, records a
/// coverage note and makes the verdict [`Verdict::Incomplete`] rather than letting the run grow
/// without limit. The defaults are deliberately generous; tests may use smaller ones.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Limits {
    /// Largest input accepted by [`validate_bytes`] (512 MiB).
    pub input_bytes: usize,
    /// Most zip entries examined (20 000).
    pub entries: usize,
    /// Largest single decompressed part examined (256 MiB).
    pub part_bytes: u64,
    /// Largest total of all decompressed parts (512 MiB).
    pub total_bytes: u64,
    /// Deepest XML element nesting examined (512).
    pub depth: usize,
    /// Most XML events examined per part (2 000 000).
    pub events: usize,
    /// Most collected facts per category (200 000).
    pub facts: usize,
    /// Most coverage notes recorded (10 000).
    pub notes: usize,
    /// Most findings recorded per rule (5 000).
    pub findings_per_rule: usize,
}

impl Default for Limits {
    /// Construct default validation limits.
    fn default() -> Self {
        Limits {
            input_bytes: 512 << 20,
            entries: 20_000,
            part_bytes: 256 << 20,
            total_bytes: 512 << 20,
            depth: 512,
            events: 2_000_000,
            facts: 200_000,
            notes: 10_000,
            findings_per_rule: 5_000,
        }
    }
}

/// The overall outcome of a validation run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Verdict {
    /// No Corrupt or Error finding, and every required input was examined.
    Pass,
    /// At least one Corrupt or Error finding.
    Fail,
    /// A required input could not be examined at all.
    Incomplete,
}

/// How serious a finding is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Severity {
    /// The package cannot be a valid document.
    Corrupt,
    /// A rule of the specification is violated.
    Error,
    /// Something Word tolerates, but that is still worth reporting.
    Warning,
}

/// How the rule's truth was established.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Evidence {
    /// The package format forces it: it cannot be otherwise.
    ByConstruction,
    /// Stated in the OOXML specification.
    MsDocumented,
    /// Seen in documents produced by Word.
    Observed,
    /// Concluded from the package contents.
    Inferred,
}

/// One problem found in a document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Finding {
    /// Stable rule id, e.g. `"REL-DANGLING"`.
    pub rule: &'static str,
    pub severity: Severity,
    pub evidence: Evidence,
    /// The package part the finding belongs to, if any.
    pub part: Option<String>,
    /// Human-readable, naming the concrete id or value involved.
    pub message: String,
}

/// The result of validating one document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Report {
    pub verdict: Verdict,
    /// Deterministic order: by part, then rule id, then message.
    pub findings: Vec<Finding>,
    /// Things skipped within a completed run.
    pub coverage_notes: Vec<String>,
    /// Every rule id that executed, in the documented order.
    pub checks_run: Vec<&'static str>,
}

/// A rule's fixed severity and evidence. [`Acc::add`] looks a rule's grade up here, so this table
/// is the single place a rule's severity and evidence are recorded, next to their provenance.
#[derive(Debug, Clone, Copy)]
pub(crate) struct RuleSpec {
    pub id: &'static str,
    pub severity: Severity,
    pub evidence: Evidence,
}

/// `PKG-NOT-ZIP`: the input is not a readable zip archive.
pub(crate) const PKG_NOT_ZIP: RuleSpec = RuleSpec { id: "PKG-NOT-ZIP", severity: Severity::Corrupt, evidence: Evidence::ByConstruction };
/// `PKG-DUP-ENTRY`: the same entry name appears twice. Observed: Word web shows the view-only
/// corruption banner.
pub(crate) const PKG_DUP_ENTRY: RuleSpec = RuleSpec { id: "PKG-DUP-ENTRY", severity: Severity::Error, evidence: Evidence::Observed };
/// `PKG-CASE-COLLISION`: two part names differ only by ASCII case. Observed: view-only banner.
pub(crate) const PKG_CASE_COLLISION: RuleSpec = RuleSpec { id: "PKG-CASE-COLLISION", severity: Severity::Error, evidence: Evidence::Observed };
/// `PKG-NO-CONTENT-TYPES`: `[Content_Types].xml` is missing.
pub(crate) const PKG_NO_CONTENT_TYPES: RuleSpec =
    RuleSpec { id: "PKG-NO-CONTENT-TYPES", severity: Severity::Corrupt, evidence: Evidence::ByConstruction };
/// `PKG-NO-MAIN`: no `officeDocument` relationship, or its target is absent. Observed: Word web
/// refuses.
pub(crate) const PKG_NO_MAIN: RuleSpec = RuleSpec { id: "PKG-NO-MAIN", severity: Severity::Error, evidence: Evidence::Observed };
/// `CT-UNDECLARED`: a part has no effective content type. Observed: view-only corruption banner.
pub(crate) const CT_UNDECLARED: RuleSpec = RuleSpec { id: "CT-UNDECLARED", severity: Severity::Error, evidence: Evidence::Observed };
/// `CT-MAIN-TYPE`: the main part's effective content type is not the WordprocessingML main type.
/// Observed: Word web refuses.
pub(crate) const CT_MAIN_TYPE: RuleSpec = RuleSpec { id: "CT-MAIN-TYPE", severity: Severity::Error, evidence: Evidence::Observed };
/// `CT-DUP-DEFAULT`: two Default entries for one extension (case-insensitive), OPC §7.2.3.2.1.
pub(crate) const CT_DUP_DEFAULT: RuleSpec = RuleSpec { id: "CT-DUP-DEFAULT", severity: Severity::Error, evidence: Evidence::ByConstruction };
/// `CT-DUP-OVERRIDE`: two Override entries for one part name (case-insensitive), OPC §7.2.3.2.1.
pub(crate) const CT_DUP_OVERRIDE: RuleSpec = RuleSpec { id: "CT-DUP-OVERRIDE", severity: Severity::Error, evidence: Evidence::ByConstruction };
/// `XML-MALFORMED`: an XML part does not parse. Severity is graded per part at the finding site
/// (main part Corrupt, reachable Error, unreferenced Warning); the evidence for the unused
/// Warning case is Observed, the referenced case is Inferred, the main part ByConstruction.
pub(crate) const XML_MALFORMED: RuleSpec = RuleSpec { id: "XML-MALFORMED", severity: Severity::Corrupt, evidence: Evidence::ByConstruction };
/// `XML-UNDECLARED-PREFIX`: an element or attribute uses a prefix not in scope.
pub(crate) const XML_UNDECLARED_PREFIX: RuleSpec =
    RuleSpec { id: "XML-UNDECLARED-PREFIX", severity: Severity::Corrupt, evidence: Evidence::ByConstruction };
/// `XML-INVALID-CHAR`: a part contains a character illegal in XML 1.0, or a reference to one.
pub(crate) const XML_INVALID_CHAR: RuleSpec = RuleSpec { id: "XML-INVALID-CHAR", severity: Severity::Corrupt, evidence: Evidence::ByConstruction };
/// `REL-DUP-ID`: the same `Id` twice within one `.rels` part. Observed: view-only banner.
pub(crate) const REL_DUP_ID: RuleSpec = RuleSpec { id: "REL-DUP-ID", severity: Severity::Error, evidence: Evidence::Observed };
/// `REL-DANGLING`: an internal relationship target is not in the package. Observed: Word web opens
/// in editing mode with the referenced content lost.
pub(crate) const REL_DANGLING: RuleSpec = RuleSpec { id: "REL-DANGLING", severity: Severity::Error, evidence: Evidence::Observed };
/// `REL-TYPED`: a header/footer/image relationship targets the wrong content type. Observed.
pub(crate) const REL_TYPED: RuleSpec = RuleSpec { id: "REL-TYPED", severity: Severity::Error, evidence: Evidence::Observed };
/// `REL-REF-MISSING`: an `r:id`/`r:embed`/`r:link` value has no relationship in the part's own
/// `.rels` (the element-side counterpart of `REL-DANGLING`).
pub(crate) const REL_REF_MISSING: RuleSpec = RuleSpec { id: "REL-REF-MISSING", severity: Severity::Error, evidence: Evidence::Inferred };
/// `CT-MISMATCH`: a part reached through a fixed-type relationship has the wrong content type.
/// Error. The pairing table is the ECMA-376 Part 1 part catalogue (see [`FIXED_REL_CONTENT_TYPES`]);
/// findings are graded at the site. `glossaryDocument` is `Observed` (Word for the web, 2026-11-06).
/// The three extension rows (`commentsExtended`, `commentsIds`, `people`) are also `Observed`, but
/// on **one combined A/B observation, not three independent refusal experiments**: two byte-identical
/// files differing only in those three content-type strings together, the `application/vnd.ms-word.*`
/// spellings against the prescribed `…wordprocessingml.*` spellings; the ms-word set was edit-refused
/// as corrupt, so the refusal is observed for the set, not isolated per part. `commentsExtended`
/// additionally has a positive observation: its resolved state was honoured under the accepted
/// spelling. The rest of the Part 1 catalogue is `MsDocumented`.
pub(crate) const CT_MISMATCH: RuleSpec = RuleSpec { id: "CT-MISMATCH", severity: Severity::Error, evidence: Evidence::MsDocumented };
/// `CT-DUP-MAIN`: more than one part carries the package's main-document content type. Observed:
/// the same 2026-11-06 corpus defect (a main-typed glossary part beside the real main part), which
/// Word for the web refused as corrupt.
pub(crate) const CT_DUP_MAIN: RuleSpec = RuleSpec { id: "CT-DUP-MAIN", severity: Severity::Error, evidence: Evidence::Observed };
/// `ID-COMMENT-DUP`: duplicate `w:comment` ids. Observed: Word web opens in Reading View and
/// refuses editing. ECMA-376 §17.13.4.2 defines collapse semantics the web consumer does not
/// follow.
pub(crate) const ID_COMMENT_DUP: RuleSpec = RuleSpec { id: "ID-COMMENT-DUP", severity: Severity::Error, evidence: Evidence::Observed };
/// `ID-COMMENT-REF`: a `w:commentReference` with no matching `w:comment`. Observed: web shows the
/// "Document corrupted" view-only banner.
pub(crate) const ID_COMMENT_REF: RuleSpec = RuleSpec { id: "ID-COMMENT-REF", severity: Severity::Error, evidence: Evidence::Observed };
/// `ID-DOCPR-DUP`: duplicate `wp:docPr` ids. Conflicting observations: Word web tolerates the
/// shape (both pictures render), while desktop field reports of refusal exist (shape-dependent),
/// so it is a Warning.
pub(crate) const ID_DOCPR_DUP: RuleSpec = RuleSpec { id: "ID-DOCPR-DUP", severity: Severity::Warning, evidence: Evidence::Observed };
/// `ID-BOOKMARK-DUP`: duplicate `w:bookmarkStart` ids. Observed (web): tolerated, Warning.
pub(crate) const ID_BOOKMARK_DUP: RuleSpec = RuleSpec { id: "ID-BOOKMARK-DUP", severity: Severity::Warning, evidence: Evidence::Observed };
/// `BOOKMARK-UNPAIRED`: a `w:bookmarkStart` with no matching `w:bookmarkEnd` (same id, same part)
/// or vice versa, in any story part including comments. Observed: field reports of bookmark markup
/// inside comments.xml silently emptying/hiding the comments.
pub(crate) const BOOKMARK_UNPAIRED: RuleSpec = RuleSpec { id: "BOOKMARK-UNPAIRED", severity: Severity::Warning, evidence: Evidence::Observed };
/// `COMMENT-RANGE-UNPAIRED`: a `w:commentRangeStart` with no matching
/// `w:commentRangeEnd` (same id, same part) or vice versa, in the parts
/// where Word honours ranges — the main and glossary stories, outside
/// text boxes. MS-OI29500 Part 1 §17.13.4.4 notes (b)–(c) and §17.13.4.3
/// notes (a)–(b): a lone start or end is re-anchored at the
/// commentReference, so the comment survives but its highlighted range
/// collapses to a point; and Word ignores both elements under a header,
/// footer, text box, footnote, or endnote.
pub(crate) const COMMENT_RANGE_UNPAIRED: RuleSpec =
    RuleSpec { id: "COMMENT-RANGE-UNPAIRED", severity: Severity::Warning, evidence: Evidence::MsDocumented };
/// `PARAID-DUP`: the same `w14:paraId` value on two elements in one
/// document part. MS-DOCX §2.6.2.3: a paraId "is unique within the
/// document part … with the exception that it need not be unique
/// across the choices or fallback of an Alternate Content block".
/// The collection is branch-aware: a repeat is excused exactly when
/// the two occurrences sit in different branch instances of some
/// block enclosing both (two Choices of one block, a Choice and the
/// Fallback, or different branches of an outer block around nested
/// content) — only one branch of a block is ever selected, so such
/// occurrences cannot coexist. Every other repeat is a duplicate.
/// Values are compared as numbers (leading zeros and case are not
/// distinct identities). MS-DOCX states no open-time consequence;
/// the stake is paragraph identity for anything keyed by paraId.
/// Capped collection goes INCOMPLETE rather than silently passing,
/// including when one value has more mutually excused occurrences
/// than the bounded collection retains.
pub(crate) const PARAID_DUP: RuleSpec = RuleSpec { id: "PARAID-DUP", severity: Severity::Warning, evidence: Evidence::MsDocumented };
/// `PARAID-VALUE`: a `w14:paraId` or `w14:textId` value outside the
/// range MS-DOCX requires (greater than 0 and less than 0x80000000,
/// §2.6.2.3 and §2.6.2.4), a value that is not a hex number at all, or
/// a `w14:textId` on an element with no `w14:paraId` (§2.6.2.4: "Any
/// element having this attribute MUST also have the paraId
/// attribute"). MS-DOCX states no reciprocal requirement, so a
/// paraId without a textId is deliberately NOT flagged.
pub(crate) const PARAID_VALUE: RuleSpec = RuleSpec { id: "PARAID-VALUE", severity: Severity::Warning, evidence: Evidence::MsDocumented };
/// `ID-REVISION-DUP`: duplicate revision ids. Observed (web): tolerated, Warning.
pub(crate) const ID_REVISION_DUP: RuleSpec = RuleSpec { id: "ID-REVISION-DUP", severity: Severity::Warning, evidence: Evidence::Observed };
/// `NOTE-DUP`: duplicate footnote or endnote ids within one note part. ECMA-376 §17.11.10.
pub(crate) const NOTE_DUP: RuleSpec = RuleSpec { id: "NOTE-DUP", severity: Severity::Error, evidence: Evidence::MsDocumented };
/// `REF-NOTE`: a note reference with no note definition. Observed: Word web opens and the footnote
/// renders as a phantom — mark present, pane entry present, body empty.
pub(crate) const REF_NOTE: RuleSpec = RuleSpec { id: "REF-NOTE", severity: Severity::Error, evidence: Evidence::Observed };
/// `REF-HEADER`: a header/footer reference with no matching relationship. Observed: editing refused.
pub(crate) const REF_HEADER: RuleSpec = RuleSpec { id: "REF-HEADER", severity: Severity::Error, evidence: Evidence::Observed };
/// `REF-STYLE`: a style reference with no matching style. Observed (web): tolerated, Warning.
pub(crate) const REF_STYLE: RuleSpec = RuleSpec { id: "REF-STYLE", severity: Severity::Warning, evidence: Evidence::Observed };
/// `STYLE-DUP-ID`: the same styleId on two `w:style` definitions in
/// one styles part. ECMA-376 §17.7.4.17 requires the identifier to be
/// unique and spells out the harm (later definitions are reassigned;
/// references are not repaired). Batch 5 (Word for the web): the
/// second definition silently wins, and ids differing only by case
/// are treated as one — comparison folds case.
pub(crate) const STYLE_DUP_ID: RuleSpec = RuleSpec { id: "STYLE-DUP-ID", severity: Severity::Warning, evidence: Evidence::Observed };
/// `STYLE-DUP-NAME`: two styles in one part whose primary names
/// collide after Word's documented trim (§17.7.4.9(c)), compared
/// case-insensitively. No uniqueness statement exists for names;
/// batch 5 observed the collision in Word for the web at the
/// formatting-loss tier. Which definition name-keyed machinery binds
/// is undocumented, and the message does not claim a direction.
pub(crate) const STYLE_DUP_NAME: RuleSpec = RuleSpec { id: "STYLE-DUP-NAME", severity: Severity::Warning, evidence: Evidence::Observed };
/// `STYLE-RESERVED-NAME`: a style whose primary name is one of the
/// 267 reserved built-in names (MS-OI29500 §17.7.4.9(d) table) while
/// its type differs from the reserved type, or its type is omitted —
/// Word renames the style or changes its type on load. A reserved
/// name WITH its reserved type is the normal form and never fires.
/// The table is the en-US table; built-in names are localised, so
/// wrong-type styles named in other languages are missed by design.
pub(crate) const STYLE_RESERVED_NAME: RuleSpec =
    RuleSpec { id: "STYLE-RESERVED-NAME", severity: Severity::Warning, evidence: Evidence::MsDocumented };
/// `STYLE-RESERVED-ID`: one of the three styleIds MS-OI29500
/// §17.7.4.17(a) documents as special — `NoList`,
/// `DefaultParagraphFont`, `TableNormal`, whose child elements Word
/// ignores — declared with a type different from its reserved type.
/// Payload under a correctly typed reserved id is the normal form
/// (Word's own files carry it) and never fires; an omitted type is
/// not flagged either (the narrowed, documented trigger only).
pub(crate) const STYLE_RESERVED_ID: RuleSpec = RuleSpec { id: "STYLE-RESERVED-ID", severity: Severity::Warning, evidence: Evidence::MsDocumented };
/// `STYLE-COUNT`: more than 4,079 styles in one styles part.
/// MS-OI29500 §17.7.4.17(c): "Word does not open a document
/// containing more than 4,079 styles." Batch 5 found Word for the
/// web opening 4,080 styles cleanly, so the rule is a Warning with
/// that caveat, mirroring the compat-count family.
pub(crate) const STYLE_COUNT: RuleSpec = RuleSpec { id: "STYLE-COUNT", severity: Severity::Warning, evidence: Evidence::MsDocumented };
/// `STYLE-ID-LENGTH`: a styleId over 253 characters. MS-OI29500
/// §17.7.4.17(b) says Word ignores such a value, leaving the style
/// unreferenceable; batch 5 found Word for the web applying a
/// 254-character id, so the grade stays Inferred.
pub(crate) const STYLE_ID_LENGTH: RuleSpec = RuleSpec { id: "STYLE-ID-LENGTH", severity: Severity::Warning, evidence: Evidence::Inferred };
/// `FONT-DUP-NAME`: the same font name twice in one fontTable part,
/// compared case-insensitively (ECMA-376 §17.8.3.10 makes the name
/// the join key for the font record; MS-OI29500 §17.8.3.10(b)
/// documents case-insensitivity). Batch 5: Word for the web showed
/// the corruption banner — Error grade.
pub(crate) const FONT_DUP_NAME: RuleSpec = RuleSpec { id: "FONT-DUP-NAME", severity: Severity::Error, evidence: Evidence::Observed };
/// `PROP-DUP-NAME`: the same custom-property name twice in
/// docProps/custom.xml, compared case-insensitively (ECMA-376 §22.3
/// identifies properties by name; MS-OI29500 §22.3.2.2(g) requires a
/// "case-insensitive unique name"). Batch 5: Word for the web showed
/// the corruption banner — Error grade.
pub(crate) const PROP_DUP_NAME: RuleSpec = RuleSpec { id: "PROP-DUP-NAME", severity: Severity::Error, evidence: Evidence::Observed };
/// `NUM-DANGLING`: a list-numbering reference or abstract-number reference with no definition, or a
/// duplicate definition. Inferred (the compare/merge residue class).
pub(crate) const NUM_DANGLING: RuleSpec = RuleSpec { id: "NUM-DANGLING", severity: Severity::Warning, evidence: Evidence::Inferred };
/// `COMMENTEX-LINK`: a comment-threading link with no counterpart — a commentEx `paraId` with no
/// comments paragraph, a `paraIdParent` with no commentEx `paraId`, a commentsIds `paraId` with
/// no commentEx `paraId`, or a commentEx `paraId` with no commentsIds `paraId`. Inferred (resolved
/// state / reply threading silently drop).
pub(crate) const COMMENTEX_LINK: RuleSpec = RuleSpec { id: "COMMENTEX-LINK", severity: Severity::Warning, evidence: Evidence::Inferred };
/// `FLD-UNBALANCED`: unbalanced field characters within one story part. Observed: Word web refuses
/// editing. §17.16.18's graceful-degradation text is not what the web consumer does.
pub(crate) const FLD_UNBALANCED: RuleSpec = RuleSpec { id: "FLD-UNBALANCED", severity: Severity::Error, evidence: Evidence::Observed };
/// `STRUCT-TC-NO-P`: a table cell with no block-level content. ECMA-376 Part 1 §17.4.65 calls a
/// cell with no block content corrupt. Evidence stays Inferred: Word web tolerated the no-content
/// fixture, desktop behaviour is unknown, and neighbouring field reports repair prompts in this
/// family.
pub(crate) const STRUCT_TC_NO_P: RuleSpec = RuleSpec { id: "STRUCT-TC-NO-P", severity: Severity::Error, evidence: Evidence::Inferred };
/// `STRUCT-END-P`: a nonempty main-part cell whose last block is a nested table rather
/// than a paragraph (ECMA-376 Part 1 §17.4.65). Calibration fixture 32 produced a
/// corruption banner and view-only Word-web document; paragraph-ending control 33
/// opened clean. Wrapper endings (sdt, customXml, altChunk) are not flagged: they can
/// hold a terminal paragraph, so their ending is treated as unknown. Desktop behaviour
/// unrecorded.
pub(crate) const STRUCT_END_P: RuleSpec = RuleSpec { id: "STRUCT-END-P", severity: Severity::Error, evidence: Evidence::Observed };
/// `MATH-PLACEMENT`: math elements where Word refuses them — an `m:oMath` nested in
/// another `m:oMath`, math content outside any `m:oMath`, an `m:oMath` or `m:oMathPara`
/// outside any `w:p`, or an `m:oMathPara` descended from a math element. MS-OI29500
/// Part 1 §22.1.2.77 notes (b), (d), (e), (f) and §22.1.2.78 notes (b), (c), (d) each
/// state Word fails to open the file; the base standard allows every one of these
/// constructs, so this is a documented-divergence rule. Batch b6 (Word for the web,
/// 2026-11-07): the nested-oMath, outside-any-p and oMathPara-in-math cases all drew
/// the corruption banner (Error confirmed), but a lone stray math element outside any
/// oMath opened clean and rendered as plain text — that one case is reported at
/// Warning instead (see scan.rs).
pub(crate) const MATH_PLACEMENT: RuleSpec = RuleSpec { id: "MATH-PLACEMENT", severity: Severity::Error, evidence: Evidence::MsDocumented };
/// `COMMENT-RANGE-COUNT`: more than 32,767 comment ranges in the document.
/// MS-OI29500 Part 1 §17.13.4.4 note (a): the standard allows an unbounded
/// number of comment ranges, but "Word will not open a file with more than
/// 32767 comment ranges". Ranges are counted across every story part.
pub(crate) const COMMENT_RANGE_COUNT: RuleSpec = RuleSpec { id: "COMMENT-RANGE-COUNT", severity: Severity::Error, evidence: Evidence::MsDocumented };
/// `PROTECTION-PASSWORD`: a `w:documentProtection` or `w:writeProtection`
/// element in settings carrying any AG_Password attribute (algorithmName,
/// hashValue, saltValue, spinCount) while missing at least one of
/// algorithmName, hashValue, spinCount. MS-OI29500 Part 1 §17.15.1.93
/// note (c): if any AG_Password attribute is present, Word requires those
/// three, and otherwise will not open the file. Batch b6 (Word for the
/// web, 2026-11-07): a partial set opened clean and stayed editable, so
/// the documented claim is not enforced there and the rule reports at
/// Warning. Desktop behaviour unrecorded.
pub(crate) const PROTECTION_PASSWORD: RuleSpec =
    RuleSpec { id: "PROTECTION-PASSWORD", severity: Severity::Warning, evidence: Evidence::MsDocumented };
/// `HEADERSOURCE-PRESENT`: a `w:headerSource` element under `w:mailMerge`
/// in a settings part. MS-OI29500 Part 1 §17.14.16 note (a): "Word fails
/// to open WordprocessingML documents in which this element is present."
/// Batch b6 (Word for the web, 2026-11-07): corruption banner, view-only
/// — observed.
pub(crate) const HEADERSOURCE_PRESENT: RuleSpec = RuleSpec { id: "HEADERSOURCE-PRESENT", severity: Severity::Error, evidence: Evidence::Observed };
/// `CONTENTPART-TYPE`: a `w:contentPart` element whose relationship target's
/// effective content type is not `text/xml`. MS-OI29500 Part 1 §17.3.3.2
/// note (a): "Word will fail to load the document if the content part's
/// content type is not text/xml." Batch b6 (Word for the web, 2026-11-07):
/// an image/png target drew the corruption banner while the text/xml twin
/// opened clean — observed.
pub(crate) const CONTENTPART_TYPE: RuleSpec = RuleSpec { id: "CONTENTPART-TYPE", severity: Severity::Error, evidence: Evidence::Observed };
/// `BOOKMARK-COUNT`: more than 16,380 bookmarks while settings carries
/// `w:uiCompat97to2003`. MS-OI29500 Part 1 §17.13.6 note (b): Word fails to
/// open such a file (the limit without the flag is 2,147,483,647). Batch
/// b6 (Word for the web, 2026-11-07): 16,381 bookmarks with the flag
/// opened clean, so the documented claim is not enforced there and the
/// rule reports at Warning. Desktop behaviour unrecorded.
pub(crate) const BOOKMARK_COUNT: RuleSpec = RuleSpec { id: "BOOKMARK-COUNT", severity: Severity::Warning, evidence: Evidence::MsDocumented };
/// `PERM-COUNT`: more than 32,752 range permissions while settings carries
/// `w:uiCompat97to2003`. MS-OI29500 Part 1 §17.13.7.1 note (b) and
/// §17.13.7.2 note (d). Batch b6 (Word for the web, 2026-11-07): 32,753
/// range permissions with the flag opened clean — Warning, as for
/// BOOKMARK-COUNT.
pub(crate) const PERM_COUNT: RuleSpec = RuleSpec { id: "PERM-COUNT", severity: Severity::Warning, evidence: Evidence::MsDocumented };
/// `MOVE-RANGE-COUNT`: more than 32,751 moveFrom/moveTo ranges while
/// settings carries `w:uiCompat97to2003`. MS-OI29500 Part 1 §17.13.5.24
/// note (a). Batch b6 (Word for the web, 2026-11-07): 32,752 move ranges
/// (65,504 start markers) with the flag opened clean, so neither counting
/// unit is enforced there — Warning, as for BOOKMARK-COUNT.
pub(crate) const MOVE_RANGE_COUNT: RuleSpec = RuleSpec { id: "MOVE-RANGE-COUNT", severity: Severity::Warning, evidence: Evidence::MsDocumented };
/// `NOTE-PR-LISTING`: a special footnote/endnote (separator,
/// continuationSeparator, continuationNotice) whose id no `footnotePr` /
/// `endnotePr` lists. ECMA-376 Part 1 §17.11.9: it "shall not be loaded",
/// and the document is non-conformant when the note "is required by the
/// document content" — a conformance claim, graded ByConstruction.
/// Batch b6 (Word for the web, 2026-11-07): an unlisted separator set
/// opened clean and the footnote rendered with its separator, so the
/// load failure the standard describes is not observed there and every
/// case reports at Warning. Desktop behaviour unrecorded.
pub(crate) const NOTE_PR_LISTING: RuleSpec = RuleSpec { id: "NOTE-PR-LISTING", severity: Severity::Warning, evidence: Evidence::ByConstruction };
/// `NOTE-PR-DANGLING`: a `footnotePr` / `endnotePr` listing names a note
/// id that no footnote/endnote defines. Nothing documents Word loading or
/// rejecting such a listing; it is dead configuration, reported at the
/// lowest grade.
pub(crate) const NOTE_PR_DANGLING: RuleSpec = RuleSpec { id: "NOTE-PR-DANGLING", severity: Severity::Warning, evidence: Evidence::Inferred };
/// `STRUCT-DELTEXT`: a `w:t` inside `w:del`, or a `w:delText` outside one. Observed (web): lenient
/// recovery, the text renders as a tracked deletion anyway, so Warning.
pub(crate) const STRUCT_DELTEXT: RuleSpec = RuleSpec { id: "STRUCT-DELTEXT", severity: Severity::Warning, evidence: Evidence::Observed };
/// `STRUCT-MOVE-UNPAIRED`: unpaired move range starts/ends, mismatched move names, or duplicate
/// move range-start ids. Observed (web): move semantics silently dropped, so Warning.
pub(crate) const STRUCT_MOVE_UNPAIRED: RuleSpec = RuleSpec { id: "STRUCT-MOVE-UNPAIRED", severity: Severity::Warning, evidence: Evidence::Observed };
/// `CUSTOMXML-SCHEMA`: a custom-XML schema reference not attached in settings, or inline
/// `w:customXml` inside a revision/move wrapper. MS-OI29500: Word will not open.
pub(crate) const CUSTOMXML_SCHEMA: RuleSpec = RuleSpec { id: "CUSTOMXML-SCHEMA", severity: Severity::Error, evidence: Evidence::MsDocumented };
/// `MEDIA-MAGIC`: an image whose bytes do not match its extension/type. Observed: Word web opens
/// but the picture does not render — content lost, no format-sniffing rescue.
pub(crate) const MEDIA_MAGIC: RuleSpec = RuleSpec { id: "MEDIA-MAGIC", severity: Severity::Error, evidence: Evidence::Observed };

/// Every rule, in the documented order; [`Acc::add`] grades findings from this table.
pub(crate) const RULES: &[RuleSpec] = &[
    PKG_NOT_ZIP,
    PKG_DUP_ENTRY,
    PKG_CASE_COLLISION,
    PKG_NO_CONTENT_TYPES,
    PKG_NO_MAIN,
    CT_UNDECLARED,
    CT_MAIN_TYPE,
    CT_DUP_MAIN,
    CT_DUP_DEFAULT,
    CT_DUP_OVERRIDE,
    XML_MALFORMED,
    XML_UNDECLARED_PREFIX,
    XML_INVALID_CHAR,
    STRUCT_END_P,
    MATH_PLACEMENT,
    COMMENT_RANGE_COUNT,
    BOOKMARK_COUNT,
    PERM_COUNT,
    MOVE_RANGE_COUNT,
    NOTE_PR_LISTING,
    NOTE_PR_DANGLING,
    PROTECTION_PASSWORD,
    HEADERSOURCE_PRESENT,
    CONTENTPART_TYPE,
    REL_DUP_ID,
    REL_DANGLING,
    REL_TYPED,
    REL_REF_MISSING,
    CT_MISMATCH,
    ID_COMMENT_DUP,
    ID_COMMENT_REF,
    ID_DOCPR_DUP,
    ID_BOOKMARK_DUP,
    BOOKMARK_UNPAIRED,
    COMMENT_RANGE_UNPAIRED,
    PARAID_DUP,
    PARAID_VALUE,
    ID_REVISION_DUP,
    NOTE_DUP,
    REF_NOTE,
    REF_HEADER,
    REF_STYLE,
    STYLE_DUP_ID,
    STYLE_DUP_NAME,
    STYLE_RESERVED_NAME,
    STYLE_RESERVED_ID,
    STYLE_COUNT,
    STYLE_ID_LENGTH,
    FONT_DUP_NAME,
    PROP_DUP_NAME,
    NUM_DANGLING,
    COMMENTEX_LINK,
    FLD_UNBALANCED,
    STRUCT_TC_NO_P,
    STRUCT_DELTEXT,
    STRUCT_MOVE_UNPAIRED,
    CUSTOMXML_SCHEMA,
    MEDIA_MAGIC,
];

/// Look up a rule definition by its stable string identifier.
fn rule_spec(id: &str) -> Option<&'static RuleSpec> {
    RULES.iter().find(|r| r.id == id)
}

#[derive(Default)]
pub(crate) struct Acc {
    findings: Vec<Finding>,
    coverage: Vec<String>,
    checks: Vec<&'static str>,
    incomplete: bool,
    limits: Limits,
    rule_counts: HashMap<&'static str, usize>,
    /// An entry was found whose identity could not be recovered. It could be any part, so no
    /// absence conclusion is established while this is set (see [`ABSENCE_RULES`]).
    unknown_unreadable: bool,
}

/// Rules whose truth depends on something being *absent* from the package (a relationship target, a
/// content-type declaration, a cross-part reference's definition, or the main-part resolution). When
/// the package holds an entry of unknown identity, whatever it might have contained is unknown, so
/// none of these can be concluded: absence is not established. Presence-based rules are unaffected.
const ABSENCE_RULES: &[&str] = &[
    "PKG-NO-CONTENT-TYPES",
    "PKG-NO-MAIN",
    "CT-UNDECLARED",
    "CT-MAIN-TYPE",
    "REL-DANGLING",
    "REL-TYPED",
    "REL-REF-MISSING",
    "ID-COMMENT-REF",
    "REF-NOTE",
    "REF-HEADER",
    "REF-STYLE",
    "STYLE-DUP-ID",
    "STYLE-DUP-NAME",
    "STYLE-RESERVED-NAME",
    "STYLE-RESERVED-ID",
    "STYLE-COUNT",
    "STYLE-ID-LENGTH",
    "FONT-DUP-NAME",
    "PROP-DUP-NAME",
    "NUM-DANGLING",
    "COMMENTEX-LINK",
    "CUSTOMXML-SCHEMA",
];

impl Acc {
    /// Create a new accumulator initialized with the specified resource limits.
    pub(crate) fn with_limits(limits: Limits) -> Acc {
        Acc { limits, ..Acc::default() }
    }

    /// Record that a validation rule was executed.
    pub(crate) fn ran(&mut self, rule: &'static str) {
        self.checks.push(rule);
    }

    /// Record that the archive holds an entry whose identity could not be recovered. Absence
    /// conclusions are suppressed from here on (see [`ABSENCE_RULES`]).
    pub(crate) fn set_unknown_unreadable(&mut self) {
        self.unknown_unreadable = true;
    }

    /// Whether an absence-class finding must be suppressed because an entry of unknown identity
    /// exists: absence is not established. Suppression happens at emission, so no count is spent and
    /// no per-rule budget is touched.
    fn absence_suppressed(&self, rule: &str, severity: Severity) -> bool {
        self.unknown_unreadable && matches!(severity, Severity::Corrupt | Severity::Error) && ABSENCE_RULES.contains(&rule)
    }

    /// Whether another finding for `rule` may be recorded; once the per-rule budget is spent the
    /// run is marked incomplete and further findings of that rule are dropped.
    fn allow(&mut self, rule: &'static str) -> bool {
        let cap = self.limits.findings_per_rule;
        let count = self.rule_counts.entry(rule).or_insert(0);
        if *count >= cap {
            if *count == cap {
                self.incomplete = true;
                if self.coverage.len() < self.limits.notes {
                    self.coverage.push(format!("more than {cap} {rule} findings were found; further ones were not recorded"));
                }
            }
            return false;
        }
        *count += 1;
        true
    }

    /// Record a finding, grading it from [`RULES`].
    pub(crate) fn add(&mut self, rule: &'static str, part: Option<String>, message: String) {
        let (severity, evidence) = rule_spec(rule).map_or((Severity::Error, Evidence::Inferred), |r| (r.severity, r.evidence));
        if self.absence_suppressed(rule, severity) {
            return;
        }
        if !self.allow(rule) {
            return;
        }
        self.findings.push(Finding { rule, severity, evidence, part, message });
    }

    /// Record a finding with an explicit grade, for the few rules whose grade depends on where the
    /// finding is (`XML-MALFORMED`).
    pub(crate) fn add_graded(&mut self, rule: &'static str, severity: Severity, evidence: Evidence, part: Option<String>, message: String) {
        if self.absence_suppressed(rule, severity) {
            return;
        }
        if !self.allow(rule) {
            return;
        }
        self.findings.push(Finding { rule, severity, evidence, part, message });
    }

    /// Record a coverage note. Exhausting the note budget is itself a budget exhaustion: the run is
    /// marked incomplete so it cannot pass silently, and the last slot is replaced with a
    /// deterministic truncation marker so the INCOMPLETE is always explained (H7).
    pub(crate) fn note(&mut self, message: String) {
        const MARKER: &str = "coverage-note budget exhausted; additional explanations omitted";
        if self.coverage.len() >= self.limits.notes {
            self.incomplete = true;
            if self.limits.notes > 0 {
                let last = self.coverage.len() - 1;
                if self.coverage[last] != MARKER {
                    self.coverage[last] = MARKER.to_string();
                }
            }
            return;
        }
        self.coverage.push(message);
    }

    /// Mark the validation outcome as incomplete due to an unreadable part or resource exhaustion.
    pub(crate) fn set_incomplete(&mut self) {
        self.incomplete = true;
    }

    /// Sort findings deterministically and produce the final validation [`Report`].
    fn finish(mut self) -> Report {
        self.findings.sort_by(|a, b| {
            let ka = (a.part.as_deref().unwrap_or(""), a.rule, a.message.as_str());
            let kb = (b.part.as_deref().unwrap_or(""), b.rule, b.message.as_str());
            ka.cmp(&kb)
        });
        let failed = self.findings.iter().any(|f| matches!(f.severity, Severity::Corrupt | Severity::Error));
        let verdict = if failed {
            Verdict::Fail
        } else if self.incomplete {
            Verdict::Incomplete
        } else {
            Verdict::Pass
        };
        Report { verdict, findings: self.findings, coverage_notes: self.coverage, checks_run: self.checks }
    }
}

/// Validate a document held in memory. Never fails: every problem becomes a finding.
pub fn validate_bytes(bytes: &[u8]) -> Report {
    validate_bytes_limited(bytes, Limits::default())
}

/// [`validate_bytes`] with explicit [`Limits`] (used by the budget tests).
pub(crate) fn validate_bytes_limited(bytes: &[u8], limits: Limits) -> Report {
    let mut acc = Acc::with_limits(limits);
    if bytes.len() > limits.input_bytes {
        acc.note(format!("the input is {} bytes, over the {}-byte examination limit; it was not examined", bytes.len(), limits.input_bytes));
        acc.set_incomplete();
        return acc.finish();
    }
    let Some(pkg) = package::load(bytes, &mut acc) else {
        return acc.finish();
    };
    check_content_types(&pkg, &mut acc);
    let xml = check_xml(&pkg, &mut acc);
    check_rels(&pkg, &xml.refs, &xml.content_parts, xml.content_part_capped_any, &mut acc);
    check_content_part_types(&pkg, &xml.content_parts, xml.content_part_capped_any, &mut acc);
    check_story(&pkg, &xml.facts, xml.compat, &mut acc);
    check_note_pr(&xml.facts, &xml, &mut acc);
    check_media(&pkg, &mut acc);
    acc.finish()
}

/// Validate a document read from a path. An I/O error reading the file is returned; every
/// problem found inside the file is a finding. A file larger than the input budget is not read:
/// it yields an `INCOMPLETE` report instead.
pub fn validate_path(path: &Path) -> std::io::Result<Report> {
    let limits = Limits::default();
    let meta = std::fs::metadata(path)?;
    if meta.len() > limits.input_bytes as u64 {
        let mut acc = Acc::with_limits(limits);
        acc.note(format!("{} is {} bytes, over the {}-byte examination limit; it was not examined", path.display(), meta.len(), limits.input_bytes));
        acc.set_incomplete();
        return Ok(acc.finish());
    }
    // Read with an explicit byte limit: a file that grows between the metadata check and the read,
    // or that lies about its size, must not be pulled into memory unbounded.
    let mut reader = std::fs::File::open(path)?.take(limits.input_bytes as u64 + 1);
    let mut bytes = Vec::with_capacity(meta.len().min(1 << 20) as usize);
    reader.read_to_end(&mut bytes)?;
    if bytes.len() > limits.input_bytes {
        let mut acc = Acc::with_limits(limits);
        acc.note(format!("{} grew past the {}-byte examination limit while being read; it was not examined", path.display(), limits.input_bytes));
        acc.set_incomplete();
        return Ok(acc.finish());
    }
    Ok(validate_bytes_limited(&bytes, limits))
}

/// Validate `[Content_Types].xml` integrity: undeclared parts, duplicate main parts, and duplicate overrides/defaults.
fn check_content_types(pkg: &Package, acc: &mut Acc) {
    if !pkg.content_types_present {
        return;
    }
    // The manifest is present but its content could not be read: no declaration is known, so no
    // part is established as undeclared (or mistyped). Absence is not established.
    if package::content_unavailable(pkg, "[Content_Types].xml") {
        return;
    }
    acc.ran("CT-UNDECLARED");
    for e in pkg.entries.iter().filter(|e| !e.is_dir) {
        let l = e.name.to_ascii_lowercase();
        if l == "[content_types].xml" || l.ends_with(".rels") {
            continue;
        }
        if pkg.content_types.of(&e.name).is_none() {
            acc.add("CT-UNDECLARED", Some(e.name.clone()), format!("part {} has no effective content type", e.name));
        }
    }
    acc.ran("CT-MAIN-TYPE");
    if let Some(main) = &pkg.main
        && pkg.content_types.of(main) != Some(package::MAIN_CONTENT_TYPE)
    {
        let got = pkg.content_types.of(main).unwrap_or("(none)");
        acc.add("CT-MAIN-TYPE", Some(main.clone()), format!("main part {main} has content type {got}, not {}", package::MAIN_CONTENT_TYPE));
    }
    acc.ran("CT-DUP-MAIN");
    check_dup_main(pkg, acc);
    acc.ran("CT-DUP-DEFAULT");
    for ext in &pkg.content_types.dup_defaults {
        acc.add(
            "CT-DUP-DEFAULT",
            Some("[Content_Types].xml".to_string()),
            format!("[Content_Types].xml declares two Default entries for extension {ext}"),
        );
    }
    acc.ran("CT-DUP-OVERRIDE");
    for part in &pkg.content_types.dup_overrides {
        acc.add(
            "CT-DUP-OVERRIDE",
            Some("[Content_Types].xml".to_string()),
            format!("[Content_Types].xml declares two Override entries for part {part}"),
        );
    }
}

/// Whether `name`'s effective content type is one of the package's main-document content types
/// (`…document.main+xml` or the template variant).
fn is_main_typed(pkg: &Package, name: &str) -> bool {
    matches!(pkg.content_types.of(name), Some(package::MAIN_CONTENT_TYPE) | Some(package::TEMPLATE_MAIN_CONTENT_TYPE))
}

/// `CT-DUP-MAIN`: at most one part may carry the main-document content type (ECMA-376 Part 1
/// §11.3.10). Every main-typed part other than the one the package's `officeDocument` relationship
/// reaches is reported — including a main-typed part no relationship reaches at all, which the
/// pairing check (`CT-MISMATCH`) cannot see.
fn check_dup_main(pkg: &Package, acc: &mut Acc) {
    let main_typed: Vec<&str> = pkg.entries.iter().filter(|e| !e.is_dir && is_main_typed(pkg, &e.name)).map(|e| e.name.as_str()).collect();
    if main_typed.len() <= 1 {
        return;
    }
    let anchor = pkg.main.as_deref();
    for name in main_typed {
        if anchor.is_some_and(|a| a.eq_ignore_ascii_case(name)) {
            continue;
        }
        acc.add(
            "CT-DUP-MAIN",
            Some(name.to_string()),
            format!("{name} carries the main document content type, but the package's officeDocument relationship reaches a different part; a package may have only one main document"),
        );
    }
}

/// What the XML pass gathered: pooled story facts, and every `r:id`/`r:embed`/`r:link` value seen
/// per part (for `REL-REF-MISSING`).
/// Whether settings carries `w:uiCompat97to2003`, the flag the compat count
/// limits (BOOKMARK-COUNT, PERM-COUNT, MOVE-RANGE-COUNT) key on. `Unknown`
/// when a settings part exists but could not be read: the limits then cannot
/// be applied either way, and the run reports INCOMPLETE rather than guess.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Compat {
    Off,
    On,
    Unknown,
}

struct XmlScan {
    facts: scan::StoryFacts,
    refs: Vec<(String, Vec<RefFact>)>,
    /// `w:contentPart` relationship ids per owning part (for `CONTENTPART-TYPE`).
    content_parts: Vec<(String, Vec<String>)>,
    /// A contentPart collection (per part or pooled) hit the fact budget.
    content_part_capped_any: bool,
    compat: Compat,
    /// Ids listed by `w:footnotePr` / `w:endnotePr` in settings parts.
    note_listings: (Vec<String>, Vec<String>),
    /// Special note definitions: (owning part, id, is_endnote, w:type).
    special_notes: Vec<(String, String, bool, String)>,
    /// A settings part (a listing source) could not be read.
    settings_unreadable: bool,
    /// A note definition or listing collection hit the fact budget.
    note_capped_any: bool,
    /// A notes part (a definition source) could not be read.
    notes_unreadable: bool,
    /// The definition side of the note join is complete: no notes part
    /// unreadable or fact-capped, and pooled definitions did not cap.
    defs_complete: bool,
}

/// Scan all XML parts for well-formedness, undeclared prefixes, illegal characters, and extract story facts.
fn check_xml(pkg: &Package, acc: &mut Acc) -> XmlScan {
    acc.ran("XML-MALFORMED");
    acc.ran("XML-UNDECLARED-PREFIX");
    acc.ran("XML-INVALID-CHAR");
    acc.ran("STRUCT-END-P");
    acc.ran("MATH-PLACEMENT");
    acc.ran("PROTECTION-PASSWORD");
    acc.ran("HEADERSOURCE-PRESENT");
    let roles = story_roles(pkg);
    let mut facts = scan::StoryFacts::default();
    let mut refs: Vec<(String, Vec<RefFact>)> = Vec::new();
    let mut content_parts: Vec<(String, Vec<String>)> = Vec::new();
    let mut content_part_total = 0usize;
    let mut content_part_capped_any = false;
    let mut compat_on = false;
    let mut compat_unknown = false;
    let mut note_listings: (Vec<String>, Vec<String>) = (Vec::new(), Vec::new());
    let mut special_notes: Vec<(String, String, bool, String)> = Vec::new();
    let mut settings_unreadable = false;
    let mut note_capped_any = false;
    let mut notes_unreadable = false;
    let mut defs_ok_parts = true;
    let mut pooled_capped = false;
    let mut facts_pooled_part: Option<String> = None;
    let mut pooled_refs = 0usize;
    let mut refs_pooled_capped = false;
    let mut refs_pooled_part: Option<String> = None;
    for e in pkg.entries.iter().filter(|e| !e.is_dir) {
        if !package::is_xml_part(pkg, &e.name) {
            continue;
        }
        let l = e.name.to_ascii_lowercase();
        let role = roles.get(&l).copied().unwrap_or_default();
        if role.is_settings && e.decode_error.is_some() {
            compat_unknown = true;
            settings_unreadable = true;
        }
        if (role.is_footnotes || role.is_endnotes) && e.decode_error.is_some() {
            notes_unreadable = true;
            defs_ok_parts = false;
        }
        if let Some(err) = &e.decode_error {
            match err {
                DecodeError::Malformed(msg) => {
                    report_malformed(pkg, &e.name, msg, acc);
                }
                DecodeError::Unsupported(enc) => {
                    acc.note(format!("{} declares the unsupported encoding {enc}; its checks were skipped", e.name));
                    acc.set_incomplete();
                }
                DecodeError::IncompleteDeclaration => {
                    acc.note(format!("{} begins an XML declaration that does not terminate within the scan bound; its encoding could not be decided, so its checks were skipped", e.name));
                    acc.set_incomplete();
                }
            }
            continue;
        }
        let Some(text) = &e.decoded else { continue };
        let scanned = scan::scan_part(&e.name, text, role, acc);
        if scanned.note_capped {
            note_capped_any = true;
        }
        if append_note_items(&mut note_listings, &mut special_notes, &e.name, scanned.note_listings, scanned.special_notes, acc.limits.facts) {
            note_capped_any = true;
        }
        if (role.is_footnotes || role.is_endnotes) && (scanned.malformed.is_some() || scanned.fact_capped) {
            defs_ok_parts = false;
            if scanned.malformed.is_some() {
                notes_unreadable = true;
            }
        }
        if role.is_settings {
            if scanned.malformed.is_some() {
                compat_unknown = true;
                settings_unreadable = true;
            }
            // Only a settings part that scanned clean establishes On: a flag
            // seen before a malformation proves nothing about the document.
            if scanned.compat_97_2003 && scanned.malformed.is_none() {
                compat_on = true;
            }
        }
        if let Some(err) = &scanned.malformed {
            report_malformed(pkg, &e.name, err, acc);
        }
        for prefix in &scanned.unknown_prefixes {
            acc.add("XML-UNDECLARED-PREFIX", Some(e.name.clone()), format!("the namespace prefix `{prefix}` used in {} is not declared", e.name));
        }
        if role.is_story && scan::merge_facts(&mut facts, scanned.facts, acc.limits.facts) {
            pooled_capped = true;
            if facts_pooled_part.is_none() {
                facts_pooled_part = Some(e.name.clone());
            }
        }
        if scanned.content_part_capped {
            content_part_capped_any = true;
        }
        if !scanned.content_parts.is_empty()
            && append_content_parts(&mut content_parts, &mut content_part_total, &e.name, scanned.content_parts, acc.limits.facts)
        {
            content_part_capped_any = true;
        }
        if !scanned.refs.is_empty() && !refs_pooled_capped {
            if pooled_refs.saturating_add(scanned.refs.len()) > acc.limits.facts {
                refs_pooled_capped = true;
                if refs_pooled_part.is_none() {
                    refs_pooled_part = Some(e.name.clone());
                }
            } else {
                pooled_refs += scanned.refs.len();
                refs.push((e.name.clone(), scanned.refs));
            }
        }
        if let Some(desc) = scan::invalid_literal_char(text) {
            acc.add("XML-INVALID-CHAR", Some(e.name.clone()), format!("{} contains {desc}, which is illegal in XML 1.0", e.name));
        }
    }
    if pooled_capped {
        let part = facts_pooled_part.as_deref().unwrap_or("an unknown part");
        acc.note(format!("the pooled story facts exceeded {} entries at {part}; later facts were not recorded", acc.limits.facts));
        acc.set_incomplete();
    }
    if refs_pooled_capped {
        let part = refs_pooled_part.as_deref().unwrap_or("an unknown part");
        acc.note(format!("the package's relationship references exceeded {} entries at {part}; later ones were not recorded", acc.limits.facts));
        acc.set_incomplete();
    }
    let compat = if compat_on {
        Compat::On
    } else if compat_unknown {
        Compat::Unknown
    } else {
        Compat::Off
    };
    // The note collections and the contentPart ids are pooled-budgeted
    // across parts as well as per part; the allowances were enforced
    // while appending in the loop above, so the accumulated collections
    // never exceeded the fact budget at any point.
    let defs_complete = defs_ok_parts && !pooled_capped;
    XmlScan {
        facts,
        refs,
        content_parts,
        content_part_capped_any,
        compat,
        note_listings,
        special_notes,
        settings_unreadable,
        note_capped_any,
        notes_unreadable,
        defs_complete,
    }
}

/// Append one part's note listings and special definitions under the
/// pooled fact budget, enforcing the allowance while appending: the
/// accumulated collections never exceed `cap`, however many parts
/// contribute. Returns true when any entry was discarded.
fn append_note_items(
    note_listings: &mut (Vec<String>, Vec<String>),
    special_notes: &mut Vec<(String, String, bool, String)>,
    part: &str,
    listings: (Vec<String>, Vec<String>),
    specials: Vec<(String, bool, String)>,
    cap: usize,
) -> bool {
    let mut capped = false;
    let room = cap.saturating_sub(note_listings.0.len() + note_listings.1.len());
    let take_f = room.min(listings.0.len());
    let take_e = (room - take_f).min(listings.1.len());
    if take_f < listings.0.len() || take_e < listings.1.len() {
        capped = true;
    }
    note_listings.0.extend(listings.0.into_iter().take(take_f));
    note_listings.1.extend(listings.1.into_iter().take(take_e));
    for (id, is_end, t) in specials {
        if special_notes.len() >= cap {
            capped = true;
            break;
        }
        special_notes.push((part.to_string(), id, is_end, t));
    }
    capped
}

/// Append one part's `w:contentPart` ids under the pooled fact
/// budget, enforcing the allowance while appending (see
/// `append_note_items`). Returns true when any id was discarded.
fn append_content_parts(content_parts: &mut Vec<(String, Vec<String>)>, total: &mut usize, part: &str, mut ids: Vec<String>, cap: usize) -> bool {
    let room = cap.saturating_sub(*total);
    let capped = ids.len() > room;
    if capped {
        ids.truncate(room);
    }
    *total += ids.len();
    if !ids.is_empty() {
        content_parts.push((part.to_string(), ids));
    }
    capped
}

/// `NOTE-PR-LISTING` (ECMA-376 Part 1 §17.11.9) and its reverse,
/// `NOTE-PR-DANGLING`: join the special note definitions (separator,
/// continuationSeparator, continuationNotice) against the
/// `footnotePr`/`endnotePr` listings in settings parts (the listing
/// child exists only in the document-wide form of those elements).
/// Each direction reports only when its own side of the join is
/// complete: the forward direction needs every listing source read
/// and uncapped; the reverse additionally needs the definition sets
/// complete (no notes part unreadable or capped, no pooled cap). An
/// incomplete join with anything at stake — an unreadable source, a
/// cap, a definition, or a listing — yields a coverage note and an
/// INCOMPLETE run instead of a guess.
///
/// Severity: the cited text makes the finding a conformance claim
/// ("shall not be loaded"; non-conformant when the note "is required
/// by the document content"). Batch b6 found Word for the web loading
/// and rendering an unlisted separator set normally, so no case is
/// graded above Warning on this evidence; necessity is not graded
/// per note.
fn check_note_pr(facts: &scan::StoryFacts, xml: &XmlScan, acc: &mut Acc) {
    acc.ran("NOTE-PR-LISTING");
    acc.ran("NOTE-PR-DANGLING");
    let forward_ok = !xml.settings_unreadable && !xml.note_capped_any;
    let reverse_ok = forward_ok && xml.defs_complete;
    if !forward_ok || !reverse_ok {
        let anything = xml.settings_unreadable
            || xml.notes_unreadable
            || xml.note_capped_any
            || !xml.defs_complete
            || !xml.special_notes.is_empty()
            || !xml.note_listings.0.is_empty()
            || !xml.note_listings.1.is_empty();
        if anything {
            acc.note("the footnotePr/endnotePr listings could not be fully joined (a part exceeded the fact budget or a settings or notes part could not be read)".to_string());
            acc.set_incomplete();
        }
    }
    let defs_f: HashSet<&str> = facts.footnote_defs.iter().map(|(id, _)| id.as_str()).collect();
    let defs_e: HashSet<&str> = facts.endnote_defs.iter().map(|(id, _)| id.as_str()).collect();
    if forward_ok {
        let listed_f: HashSet<&str> = xml.note_listings.0.iter().map(String::as_str).collect();
        let listed_e: HashSet<&str> = xml.note_listings.1.iter().map(String::as_str).collect();
        for (part, id, is_end, t) in &xml.special_notes {
            let listed = if *is_end { listed_e.contains(id.as_str()) } else { listed_f.contains(id.as_str()) };
            if !listed {
                let (note, pr) = if *is_end { ("endnote", "endnotePr") } else { ("footnote", "footnotePr") };
                acc.add_graded(
                    "NOTE-PR-LISTING",
                    Severity::Warning,
                    Evidence::ByConstruction,
                    Some(part.clone()),
                    format!(
                        "the {note} with id {id} in {part} has type {t} but is not listed in any {pr}; ECMA-376 says such a note shall not be loaded"
                    ),
                );
            }
        }
    }
    if reverse_ok {
        for id in &xml.note_listings.0 {
            if !defs_f.contains(id.as_str()) {
                acc.add("NOTE-PR-DANGLING", None, format!("a footnotePr lists footnote id {id}, but no footnote with that id is defined"));
            }
        }
        for id in &xml.note_listings.1 {
            if !defs_e.contains(id.as_str()) {
                acc.add("NOTE-PR-DANGLING", None, format!("an endnotePr lists endnote id {id}, but no endnote with that id is defined"));
            }
        }
    }
}

/// `CONTENTPART-TYPE` (MS-OI29500 Part 1 §17.3.3.2 note (a)): each
/// `w:contentPart` element's relationship must reach a part whose effective
/// content type is `text/xml`; Word fails to load the document otherwise.
/// The effective type comes from `ContentTypes::of`, where a part Override
/// beats the extension Default (OPC §9.1.2). A relationship id the owner
/// does not define is REL-REF-MISSING's finding, and an undeclared target
/// is CT-UNDECLARED's; this check judges only a resolved, declared type.
fn check_content_part_types(pkg: &Package, content_parts: &[(String, Vec<String>)], capped: bool, acc: &mut Acc) {
    acc.ran("CONTENTPART-TYPE");
    if capped {
        acc.note(
            "the fact budget stopped collection of some w:contentPart references, so CONTENTPART-TYPE did not check every reference".to_string(),
        );
        acc.set_incomplete();
    }
    for (owner, ids) in content_parts {
        let rels_name = package::rels_part_name(owner);
        let Some(rp) = pkg.rels.iter().find(|rp| rp.name.eq_ignore_ascii_case(&rels_name)) else { continue };
        // Index the owner's relationships by id once (first wins, as a
        // linear find would return), and dedupe ids in a set: with a
        // Vec both were quadratic in the number of references.
        let by_id: std::collections::HashMap<&str, &package::Rel> = {
            let mut map = std::collections::HashMap::new();
            for r in &rp.rels {
                map.entry(r.id.as_str()).or_insert(r);
            }
            map
        };
        let mut seen: std::collections::HashSet<&String> = std::collections::HashSet::new();
        for id in ids {
            if !seen.insert(id) {
                continue;
            }
            let Some(rel) = by_id.get(id.as_str()) else { continue };
            let Some(target) = &rel.target_abs else { continue };
            let Some(ct) = pkg.content_types.of(target) else { continue };
            if !ct.eq_ignore_ascii_case("text/xml") {
                acc.add(
                    "CONTENTPART-TYPE",
                    Some(owner.clone()),
                    format!("w:contentPart in {owner} references {target} (relationship {id}), whose content type is {ct}, not text/xml"),
                );
            }
        }
    }
}

/// Report an `XML-MALFORMED` finding with severity graded by whether the malformed part is main, reachable, or unused.
fn report_malformed(pkg: &Package, name: &str, err: &str, acc: &mut Acc) {
    let lower = name.to_ascii_lowercase();
    let is_main = pkg.main.as_deref().is_some_and(|m| m.eq_ignore_ascii_case(name));
    let (severity, evidence) = if is_main {
        (Severity::Corrupt, Evidence::ByConstruction)
    } else if pkg.reachable.contains(&lower) {
        (Severity::Error, Evidence::Inferred)
    } else {
        (Severity::Warning, Evidence::Observed)
    };
    acc.add_graded("XML-MALFORMED", severity, evidence, Some(name.to_string()), format!("{name} is not well-formed XML: {err}"));
}

/// Classify the story parts from their effective content type and the relationships that reach
/// them, never from their file names or folder. A comments part stored under an unusual name is
/// still found when a comments relationship or the comments content type names it.
fn story_roles(pkg: &Package) -> HashMap<String, scan::PartRole> {
    let mut roles = HashMap::new();
    if let Some(main) = &pkg.main {
        roles.insert(main.to_ascii_lowercase(), scan::PartRole { is_main: true, is_story: true, ..scan::PartRole::default() });
    }
    for e in pkg.entries.iter().filter(|e| !e.is_dir) {
        if let Some(role) = content_type_role(pkg.content_types.of(&e.name)) {
            roles.entry(e.name.to_ascii_lowercase()).or_insert(role);
        }
    }
    for rp in &pkg.rels {
        for r in &rp.rels {
            let Some(role) = relationship_role(&r.kind) else { continue };
            let Some(target) = &r.target_abs else { continue };
            if let Some(name) = package::canonical_entry(pkg, target) {
                roles.entry(name.to_ascii_lowercase()).or_insert(role);
            }
        }
    }
    roles
}

/// Classify a part role from its declared content type.
fn content_type_role(ct: Option<&str>) -> Option<scan::PartRole> {
    let role = match ct? {
        package::COMMENTS_CONTENT_TYPE => scan::PartRole { is_story: true, is_comments: true, ..scan::PartRole::default() },
        package::FOOTNOTES_CONTENT_TYPE => scan::PartRole { is_story: true, is_footnotes: true, ..scan::PartRole::default() },
        package::ENDNOTES_CONTENT_TYPE => scan::PartRole { is_story: true, is_endnotes: true, ..scan::PartRole::default() },
        package::HEADER_CONTENT_TYPE | package::FOOTER_CONTENT_TYPE => scan::PartRole { is_story: true, ..scan::PartRole::default() },
        package::GLOSSARY_CONTENT_TYPE => scan::PartRole { is_story: true, is_glossary: true, ..scan::PartRole::default() },
        package::SETTINGS_CONTENT_TYPE => scan::PartRole { is_settings: true, ..scan::PartRole::default() },
        _ => return None,
    };
    Some(role)
}

/// Classify a part role from the relationship type that targets it.
fn relationship_role(kind: &str) -> Option<scan::PartRole> {
    // The settings role is matched on the complete relationship type, not the
    // tail: PROTECTION-PASSWORD emits Error findings from it, and a custom
    // relationship type that merely ends in "/settings" must not turn an
    // application-owned part into Word settings.
    if kind == "http://schemas.openxmlformats.org/officeDocument/2006/relationships/settings" {
        return Some(scan::PartRole { is_settings: true, ..scan::PartRole::default() });
    }
    // The glossary role is matched on the complete relationship type for the
    // same reason: COMMENT-RANGE-UNPAIRED adds a part to its scope from it,
    // and a custom relationship type that merely ends in "/glossaryDocument"
    // must not turn an application-owned part into a Word glossary.
    if kind == "http://schemas.openxmlformats.org/officeDocument/2006/relationships/glossaryDocument" {
        return Some(scan::PartRole { is_story: true, is_glossary: true, ..scan::PartRole::default() });
    }
    Some(match tail(kind) {
        "comments" => scan::PartRole { is_story: true, is_comments: true, ..scan::PartRole::default() },
        "footnotes" => scan::PartRole { is_story: true, is_footnotes: true, ..scan::PartRole::default() },
        "endnotes" => scan::PartRole { is_story: true, is_endnotes: true, ..scan::PartRole::default() },
        "header" | "footer" => scan::PartRole { is_story: true, ..scan::PartRole::default() },
        _ => return None,
    })
}

/// Verify relationships: duplicate IDs, dangling targets, typed targets, and content type matches.
fn check_rels(pkg: &Package, refs: &[(String, Vec<RefFact>)], content_parts: &[(String, Vec<String>)], content_part_capped: bool, acc: &mut Acc) {
    acc.ran("REL-DUP-ID");
    for rp in &pkg.rels {
        report_duplicate_rel_ids(rp, acc);
    }
    acc.ran("REL-DANGLING");
    for rp in &pkg.rels {
        for r in &rp.rels {
            let Some(target) = &r.target_abs else { continue };
            if package::canonical_entry(pkg, target).is_none() {
                acc.add(
                    "REL-DANGLING",
                    Some(rp.name.clone()),
                    format!(
                        "relationship {} in {} targets {target}, which is missing from the package; the referenced content is lost",
                        r.id, rp.name
                    ),
                );
            }
        }
    }
    acc.ran("REL-TYPED");
    check_typed_targets(pkg, acc);
    check_typed_image_refs(pkg, refs, acc);
    check_part_refs(pkg, refs, acc);
    check_typed_content(pkg, content_parts, content_part_capped, acc);
}

/// Find duplicate relationship IDs within a single `.rels` part and report `REL-DUP-ID` findings.
fn report_duplicate_rel_ids(rp: &RelsPart, acc: &mut Acc) {
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for r in &rp.rels {
        *counts.entry(r.id.as_str()).or_default() += 1;
    }
    let mut dupes: Vec<&str> = counts.into_iter().filter(|(_, c)| *c > 1).map(|(id, _)| id).collect();
    dupes.sort_unstable();
    for id in dupes {
        acc.add("REL-DUP-ID", Some(rp.name.clone()), format!("relationship id {id} appears more than once in {}", rp.name));
    }
}

/// Every `r:id`/`r:embed`/`r:link` value in a part must resolve in that part's own `.rels`.
fn check_part_refs(pkg: &Package, refs: &[(String, Vec<RefFact>)], acc: &mut Acc) {
    acc.ran("REL-REF-MISSING");
    for (part, values) in refs {
        // The part's own `.rels` could not be read: which relationships it declares is unknown, so
        // its `r:id`/`r:embed`/`r:link` values are not established as unresolved.
        if package::content_unavailable(pkg, &rels_part_name(part)) {
            continue;
        }
        let ids = part_rel_ids(pkg, part);
        let mut reported: HashSet<&str> = HashSet::new();
        for r in values {
            if !ids.contains(&r.value) && reported.insert(r.value.as_str()) {
                acc.add(
                    "REL-REF-MISSING",
                    Some(part.clone()),
                    format!("the r:{} value {} in {part} does not resolve to a relationship in that part's .rels", r.attr, r.value),
                );
            }
        }
    }
}

/// An image element (`a:blip`, `v:imagedata`) must resolve to an image relationship whose target is
/// an image part; anything else is `REL-TYPED` at the referencing part. The check keeps the element
/// identity, so it applies to every owner (including a header's own image references, resolved
/// against the header's `.rels`).
fn check_typed_image_refs(pkg: &Package, refs: &[(String, Vec<RefFact>)], acc: &mut Acc) {
    for (part, values) in refs {
        let Some(rp) = pkg.rels.iter().find(|r| r.name.eq_ignore_ascii_case(&rels_part_name(part))) else { continue };
        let by_id: HashMap<&str, &package::Rel> = rp.rels.iter().map(|r| (r.id.as_str(), r)).collect();
        let mut reported: HashSet<&str> = HashSet::new();
        for r in values.iter().filter(|r| r.image) {
            if !reported.insert(r.value.as_str()) {
                continue;
            }
            let Some(rel) = by_id.get(r.value.as_str()) else { continue };
            if tail(&rel.kind) != "image" {
                acc.add(
                    "REL-TYPED",
                    Some(part.clone()),
                    format!(
                        "an image element in {part} references relationship {}, which is a {} relationship, not an image relationship",
                        r.value,
                        tail(&rel.kind)
                    ),
                );
            } else if let Some(target) = &rel.target_abs {
                // The relationship resolves and targets an internal part. Only the next conclusion
                // depends on the manifest: when its content could not be read the target's declared
                // type is unknown, not absent, so it is skipped. (The non-image-relationship and
                // external-relationship conclusions above and below do not depend on the manifest.)
                if !package::content_unavailable(pkg, "[Content_Types].xml") {
                    let ct = pkg.content_types.of(target);
                    if !ct.is_some_and(|c| c.starts_with("image/")) {
                        acc.add(
                            "REL-TYPED",
                            Some(part.clone()),
                            format!(
                                "an image element in {part} references relationship {}, whose target {target} has content type {}",
                                r.value,
                                ct.unwrap_or("(none)")
                            ),
                        );
                    }
                }
            } else if r.attr != "link" {
                // `r:embed` (and `v:imagedata`'s `r:id`) name an embedded image and must resolve to
                // a part inside the package; only `r:link` may legitimately point at an external
                // resource (H4).
                acc.add(
                    "REL-TYPED",
                    Some(part.clone()),
                    format!(
                        "an image element in {part} references relationship {} with r:{}, which is an external relationship with no internal image target",
                        r.value, r.attr
                    ),
                );
            }
        }
    }
}

/// Collect the set of relationship IDs declared in `part`'s `.rels` file.
fn part_rel_ids(pkg: &Package, part: &str) -> HashSet<String> {
    let want = rels_part_name(part);
    match pkg.rels.iter().find(|r| r.name.eq_ignore_ascii_case(&want)) {
        Some(rp) => rp.rels.iter().map(|r| r.id.clone()).collect(),
        None => HashSet::new(),
    }
}

/// Verify that typed relationships in the main document part target parts with matching content types.
fn check_typed_targets(pkg: &Package, acc: &mut Acc) {
    // Every conclusion here compares a target's declared content type; when the manifest's content
    // could not be read, a target's type is UNKNOWN, not absent, so no type conclusion is
    // established. (The run is already INCOMPLETE from loading the manifest.)
    if package::content_unavailable(pkg, "[Content_Types].xml") {
        return;
    }
    let Some(main) = &pkg.main else { return };
    let want = rels_part_name(main);
    let Some(rp) = pkg.rels.iter().find(|r| r.name.eq_ignore_ascii_case(&want)) else {
        acc.note(format!("the main part {main} has no relationships part; typed targets were not checked"));
        return;
    };
    for r in &rp.rels {
        let Some(target) = &r.target_abs else { continue };
        let ct = pkg.content_types.of(target);
        match tail(&r.kind) {
            "header" if ct != Some(package::HEADER_CONTENT_TYPE) => {
                acc.add("REL-TYPED", Some(rp.name.clone()), header_footer_message("header", &r.id, target, ct));
            }
            "footer" if ct != Some(package::FOOTER_CONTENT_TYPE) => {
                acc.add("REL-TYPED", Some(rp.name.clone()), header_footer_message("footer", &r.id, target, ct));
            }
            "image" if !ct.is_some_and(|c| c.starts_with("image/")) => {
                acc.add(
                    "REL-TYPED",
                    Some(rp.name.clone()),
                    format!("image relationship {} targets {target}, whose content type is {}", r.id, ct.unwrap_or("(none)")),
                );
            }
            _ => {}
        }
    }
}

/// Format an error message for mismatched header or footer relationship targets.
fn header_footer_message(kind: &str, id: &str, target: &str, ct: Option<&str>) -> String {
    format!("{kind} relationship {id} targets {target}, whose content type is {}", ct.unwrap_or("(none)"))
}

/// The content type each fixed-type relationship requires of its internal target, keyed by the
/// relationship type's last path segment. This is the ECMA-376 Part 1 part catalogue (§11.3 and
/// §15.2, plus the MS-DOCX extension parts), so CT-MISMATCH is a table lookup rather than a
/// hand-picked list.
///
/// `officeDocument` is absent: the main part's type is pinned by `CT-MAIN-TYPE`. `header` and
/// `footer` are present because the Part 1 catalogue permits the glossary document to own them
/// (§11.3.9, §11.3.6), but the pairing check applies them only to a **non-main** owner's
/// relationships part (see [`check_typed_content`]): the main part's header/footer relationships
/// stay `REL-TYPED`'s alone, so the same part is never reported twice. Kinds the catalogue leaves
/// type-free or extension-dependent — image / media, embedded package, OLE, altChunk, content
/// parts, recipient data — prescribe no type and are deliberately absent; no pairing is invented
/// for them. The `customXml` row is applied only when the relationship is not consumed by a
/// `w:contentPart` element (see [`check_typed_content`]).
pub(crate) const FIXED_REL_CONTENT_TYPES: &[(&str, &str)] = &[
    ("glossaryDocument", package::GLOSSARY_CONTENT_TYPE),
    ("styles", package::STYLES_CONTENT_TYPE),
    ("numbering", package::NUMBERING_CONTENT_TYPE),
    ("settings", package::SETTINGS_CONTENT_TYPE),
    ("fontTable", package::FONT_TABLE_CONTENT_TYPE),
    ("theme", package::THEME_CONTENT_TYPE),
    ("comments", package::COMMENTS_CONTENT_TYPE),
    ("commentsExtended", package::COMMENTS_EXTENDED_CONTENT_TYPE),
    ("commentsIds", package::COMMENTS_IDS_CONTENT_TYPE),
    ("people", package::PEOPLE_CONTENT_TYPE),
    ("footnotes", package::FOOTNOTES_CONTENT_TYPE),
    ("endnotes", package::ENDNOTES_CONTENT_TYPE),
    ("header", package::HEADER_CONTENT_TYPE),
    ("footer", package::FOOTER_CONTENT_TYPE),
    ("webSettings", package::WEB_SETTINGS_CONTENT_TYPE),
    ("chart", package::CHART_CONTENT_TYPE),
    ("customXml", package::XML_CONTENT_TYPE),
    ("customXmlProps", package::CUSTOM_XML_PROPS_CONTENT_TYPE),
    ("bibliography", package::XML_CONTENT_TYPE),
];

/// The content type a fixed-type relationship requires, if the relationship type is one.
fn fixed_rel_content_type(kind: &str) -> Option<&'static str> {
    FIXED_REL_CONTENT_TYPES.iter().find(|(k, _)| *k == kind).map(|(_, ct)| *ct)
}

/// Verify that fixed-type relationships in the package match their prescribed part content types.
///
/// One row is consumption-sensitive (#5982): the `customXml` row prescribes
/// `application/xml`, but a relationship of that type consumed by a
/// `w:contentPart` element is governed by CONTENTPART-TYPE instead —
/// ECMA-376 §17.3.3.2's own contentPart example reaches a `text/xml` part
/// through exactly this relationship type, and Word for the web opens
/// such a file cleanly. When the contentPart collection was capped, the
/// consumed set is not established, so the customXml row is skipped
/// rather than risk the false positive (the run is already INCOMPLETE
/// from the capped collection).
fn check_typed_content(pkg: &Package, content_parts: &[(String, Vec<String>)], content_part_capped: bool, acc: &mut Acc) {
    acc.ran("CT-MISMATCH");
    // Relationship ids consumed by a `w:contentPart`, keyed by the
    // owning part's relationships-part name (compared case-insensitively).
    let consumed: HashMap<String, HashSet<&str>> =
        content_parts.iter().map(|(owner, ids)| (rels_part_name(owner).to_ascii_lowercase(), ids.iter().map(String::as_str).collect())).collect();
    // `header` and `footer` pairings are checked for a non-main owner's relationships part only
    // (the glossary document today, but computed from the owner scopes rather than hardcoded). The
    // main part's header/footer relationships belong to `REL-TYPED`, which already checks them from
    // the main part's content, so they are not reported here. The owner set is derived generically
    // from [`owner_scopes`], so a future owner is covered without another special case.
    let main_rels = pkg.main.as_deref().map(rels_part_name);
    let non_main_owner_rels: HashSet<String> = owner_scopes(pkg)
        .iter()
        .filter_map(|owner| owner.as_deref())
        .filter(|owner| !main_rels.as_deref().is_some_and(|m| m.eq_ignore_ascii_case(&rels_part_name(owner))))
        .map(|owner| rels_part_name(owner).to_ascii_lowercase())
        .collect();
    for rp in &pkg.rels {
        let is_non_main_owner = non_main_owner_rels.contains(&rp.name.to_ascii_lowercase());
        for r in &rp.rels {
            let Some(target) = &r.target_abs else { continue };
            let kind = tail(&r.kind);
            let Some(want) = fixed_rel_content_type(kind) else { continue };
            if matches!(kind, "header" | "footer") && !is_non_main_owner {
                continue;
            }
            if kind == "customXml"
                && (content_part_capped || consumed.get(&rp.name.to_ascii_lowercase()).is_some_and(|ids| ids.contains(r.id.as_str())))
            {
                continue;
            }
            let Some(ct) = pkg.content_types.of(target) else { continue };
            if ct == want {
                continue;
            }
            // The finding belongs to the mistyped part itself. The glossaryDocument,
            // commentsExtended, commentsIds and people pairings carry the 2026-11-06 observations
            // (the three extension rows on one combined A/B observation: see the rule's doc
            // comment); every other row rests on the Part 1 catalogue.
            let evidence = if matches!(kind, "glossaryDocument" | "commentsExtended" | "commentsIds" | "people") {
                Evidence::Observed
            } else {
                Evidence::MsDocumented
            };
            let part = canonical_entry(pkg, target).unwrap_or_else(|| target.clone());
            acc.add_graded(
                "CT-MISMATCH",
                Severity::Error,
                evidence,
                Some(part.clone()),
                format!("relationship {} ({kind}) in {} targets {part}, whose content type {ct} is not the required {want}", r.id, rp.name),
            );
        }
    }
}

/// Check cross-document story identifier families (comments, docPr, bookmarks, revisions, notes, styles, headers).
fn check_story(pkg: &Package, facts: &scan::StoryFacts, compat: Compat, acc: &mut Acc) {
    acc.ran("COMMENT-RANGE-COUNT");
    if facts.comment_range_starts > 32_767 {
        acc.add(
            "COMMENT-RANGE-COUNT",
            None,
            format!("the document contains {} comment ranges (w:commentRangeStart), more than the 32767 Word will open", facts.comment_range_starts),
        );
    }
    acc.ran("BOOKMARK-COUNT");
    acc.ran("PERM-COUNT");
    acc.ran("MOVE-RANGE-COUNT");
    match compat {
        Compat::Unknown => {
            acc.note("a settings part could not be read, so the Word 97-2003 compatibility limits on bookmarks, range permissions, and move ranges could not be applied".to_string());
            acc.set_incomplete();
        }
        Compat::On => {
            if facts.bookmark_starts > 16_380 {
                acc.add(
                    "BOOKMARK-COUNT",
                    None,
                    format!(
                        "the document contains {} bookmarks, more than the 16380 documented for Word when uiCompat97to2003 is set",
                        facts.bookmark_starts
                    ),
                );
            }
            if facts.perm_starts > 32_752 {
                acc.add(
                    "PERM-COUNT",
                    None,
                    format!(
                        "the document contains {} range permissions, more than the 32752 documented for Word when uiCompat97to2003 is set",
                        facts.perm_starts
                    ),
                );
            }
            if facts.move_range_starts > 32_751 {
                acc.add(
                    "MOVE-RANGE-COUNT",
                    None,
                    format!(
                        "the document contains {} move ranges, more than the 32751 documented for Word when uiCompat97to2003 is set",
                        facts.move_range_starts
                    ),
                );
            }
        }
        Compat::Off => {}
    }
    acc.ran("ID-COMMENT-DUP");
    report_dups(acc, "ID-COMMENT-DUP", &facts.comment_defs, "w:comment id");
    acc.ran("ID-COMMENT-REF");
    let comment_defs = id_set(&facts.comment_defs);
    // A comments part that could not be read leaves the pooled definitions incomplete, so a
    // reference must not be reported as dangling: absence is not established.
    if !kind_source_unavailable(pkg, "comments", package::COMMENTS_CONTENT_TYPE) {
        report_missing(acc, "ID-COMMENT-REF", &facts.comment_refs, &comment_defs, "w:commentReference id");
    }
    report_unreferenced_comments(facts, acc);
    acc.ran("ID-DOCPR-DUP");
    report_dups(acc, "ID-DOCPR-DUP", &facts.docpr, "wp:docPr id");
    acc.ran("ID-BOOKMARK-DUP");
    report_dups(acc, "ID-BOOKMARK-DUP", &facts.bookmarks, "w:bookmarkStart id");
    // BOOKMARK-UNPAIRED findings are produced by the streaming scan; the rule is registered here.
    acc.ran("BOOKMARK-UNPAIRED");
    // COMMENT-RANGE-UNPAIRED likewise.
    acc.ran("COMMENT-RANGE-UNPAIRED");
    // PARAID-DUP and PARAID-VALUE findings are produced by the streaming scan; registered here.
    acc.ran("PARAID-DUP");
    acc.ran("PARAID-VALUE");
    acc.ran("ID-REVISION-DUP");
    report_dups(acc, "ID-REVISION-DUP", &facts.revisions, "revision id");
    acc.ran("NOTE-DUP");
    report_dups(acc, "NOTE-DUP", &facts.footnote_defs, "w:footnote id");
    report_dups(acc, "NOTE-DUP", &facts.endnote_defs, "w:endnote id");
    acc.ran("REF-NOTE");
    let footnote_defs = id_set(&facts.footnote_defs);
    let endnote_defs = id_set(&facts.endnote_defs);
    // An unreadable footnotes/endnotes part leaves the definitions incomplete, so a reference is
    // not established as dangling.
    if !kind_source_unavailable(pkg, "footnotes", package::FOOTNOTES_CONTENT_TYPE) {
        report_missing_note(acc, &facts.footnote_refs, &footnote_defs, "footnote");
    }
    if !kind_source_unavailable(pkg, "endnotes", package::ENDNOTES_CONTENT_TYPE) {
        report_missing_note(acc, &facts.endnote_refs, &endnote_defs, "endnote");
    }
    acc.ran("REF-HEADER");
    check_header_footer_refs(pkg, facts, acc);
    acc.ran("REF-STYLE");
    check_style_refs(pkg, facts, acc);
    check_vocabulary(pkg, acc);
    check_numbering(pkg, facts, acc);
    check_commentex(pkg, facts, acc);
    acc.ran("FLD-UNBALANCED");
    acc.ran("STRUCT-TC-NO-P");
    acc.ran("STRUCT-DELTEXT");
    acc.ran("STRUCT-MOVE-UNPAIRED");
    check_custom_xml(pkg, acc);
}

/// Collect a set of unique identifier strings from `(id, part)` pairs.
fn id_set(items: &[(String, String)]) -> HashSet<String> {
    items.iter().map(|(id, _)| id.clone()).collect()
}

/// Whether the definitions a story kind would contribute to any owner are unestablished: either the
/// supporting part resolves to one whose content could not be read, or the discovery source that
/// would find it (the owner's `.rels`, or the manifest in the no-owner fallback scope) could not be
/// read. In both cases the part's existence is unestablished, not established-absent, so references
/// checked against it are not reported as dangling.
fn kind_source_unavailable(pkg: &Package, kind: &str, ct: &str) -> bool {
    owner_scopes(pkg).iter().any(|owner| {
        discovery_unavailable(pkg, owner.as_deref()) || supporting_part(pkg, owner.as_deref(), kind, ct).is_some_and(|e| e.content_unavailable)
    })
}

/// Record a coverage note for each comment definition that is never referenced in the document.
fn report_unreferenced_comments(facts: &scan::StoryFacts, acc: &mut Acc) {
    let refs = id_set(&facts.comment_refs);
    let mut seen = HashSet::new();
    for (id, part) in &facts.comment_defs {
        if !refs.contains(id) && seen.insert(id.as_str()) {
            acc.note(format!("w:comment id {id} in {part} is never referenced"));
        }
    }
}

/// Verify header/footer references together: the element reference, the relationship it names and
/// that relationship's kind and target content type. A reference to a non-header relationship (say
/// an image) is a `REF-HEADER` finding, not a pass.
fn check_header_footer_refs(pkg: &Package, facts: &scan::StoryFacts, acc: &mut Acc) {
    let Some(main) = &pkg.main else { return };
    let want = rels_part_name(main);
    // The main part's relationships could not be read: its header/footer relationships are unknown,
    // not absent, so a reference must not be reported as having no relationship.
    if package::content_unavailable(pkg, &want) {
        return;
    }
    let Some(rp) = pkg.rels.iter().find(|r| r.name.eq_ignore_ascii_case(&want)) else {
        for (id, part) in facts.header_refs.iter().chain(facts.footer_refs.iter()) {
            acc.add("REF-HEADER", Some(part.clone()), format!("the main part {main} has a header/footer reference {id} but no relationships part"));
        }
        return;
    };
    let by_id: HashMap<&str, &package::Rel> = rp.rels.iter().map(|r| (r.id.as_str(), r)).collect();
    let groups: [(&str, &str, &str, &[(String, String)]); 2] = [
        ("header", package::HEADER_CONTENT_TYPE, "headerReference", &facts.header_refs),
        ("footer", package::FOOTER_CONTENT_TYPE, "footerReference", &facts.footer_refs),
    ];
    for (kind, want_ct, element, refs) in groups {
        let mut reported: HashSet<&str> = HashSet::new();
        for (id, part) in refs {
            if !reported.insert(id.as_str()) {
                continue;
            }
            match by_id.get(id.as_str()) {
                None => acc.add("REF-HEADER", Some(part.clone()), format!("w:{element} r:id {id} in {part} has no matching relationship")),
                Some(rel) if tail(&rel.kind) != kind => acc.add(
                    "REF-HEADER",
                    Some(part.clone()),
                    format!("w:{element} r:id {id} in {part} resolves to a {} relationship, not a {kind} relationship", tail(&rel.kind)),
                ),
                Some(rel) if rel.external || rel.target_abs.is_none() => acc.add(
                    "REF-HEADER",
                    Some(part.clone()),
                    format!("w:{element} r:id {id} in {part} resolves to an external relationship with no internal target"),
                ),
                Some(rel) => {
                    if let Some(target) = &rel.target_abs {
                        let ct = pkg.content_types.of(target);
                        if ct != Some(want_ct) {
                            acc.add(
                                "REF-HEADER",
                                Some(part.clone()),
                                format!("w:{element} r:id {id} in {part} targets {target}, whose content type is {}", ct.unwrap_or("(none)")),
                            );
                        }
                    }
                }
            }
        }
    }
}

/// The document-level story parts whose relationships own the supporting parts: the main document
/// and, when present, the glossary document.
fn owner_scopes(pkg: &Package) -> Vec<Option<String>> {
    let mut v: Vec<Option<String>> = Vec::new();
    if let Some(m) = &pkg.main {
        v.push(Some(m.clone()));
    }
    if let Some(g) = &pkg.glossary
        && !v.iter().flatten().any(|o| o.eq_ignore_ascii_case(g))
    {
        v.push(Some(g.clone()));
    }
    if v.is_empty() {
        v.push(None);
    }
    v
}

/// The part `owner` reaches through a relationship of `kind`. `None` when the owner has no such
/// relationship.
fn owner_rel_part<'a>(pkg: &'a Package, owner: &str, kind: &str) -> Option<&'a package::Entry> {
    let want = rels_part_name(owner);
    let rp = pkg.rels.iter().find(|r| r.name.eq_ignore_ascii_case(&want))?;
    for r in &rp.rels {
        if tail(&r.kind) != kind {
            continue;
        }
        if let Some(t) = &r.target_abs
            && let Some(name) = canonical_entry(pkg, t)
            && let Some(e) = entry_by_name(pkg, &name)
        {
            return Some(e);
        }
    }
    None
}

/// The part an owner resolves `kind` to. Inside an established owner scope this is the owner's own
/// relationship target and nothing else (J2): a first-by-content-type fallback would let ZIP order
/// change which references are accepted, and a glossary's own parts could be mistaken for the main
/// document's. The package-wide first-by-type fallback is a separate, explicit path used only when
/// no owner scope exists at all (`owner` is `None`), preserving the lone-supporting-part fixtures.
fn supporting_part<'a>(pkg: &'a Package, owner: Option<&str>, kind: &str, ct: &str) -> Option<&'a package::Entry> {
    if let Some(owner) = owner {
        return owner_rel_part(pkg, owner, kind);
    }
    find_by_type(pkg, ct)
}

/// Whether the source that would discover an owner's supporting part could not be read, so the
/// part's existence is unestablished rather than established-absent. In an established owner scope
/// that source is the owner's `.rels`; in the no-owner fallback scope it is the content-type
/// manifest that [`find_by_type`] reads.
fn discovery_unavailable(pkg: &Package, owner: Option<&str>) -> bool {
    match owner {
        Some(owner) => package::content_unavailable(pkg, &rels_part_name(owner)),
        None => package::content_unavailable(pkg, "[Content_Types].xml"),
    }
}

/// Map each part (lowercased) reachable from an owner scope to that owner's index in
/// [`owner_scopes`]. A glossary subtree is attributed to the glossary even though the glossary part
/// is reached from the main document, so its references are never checked against main definitions.
/// When there is no owner scope at all, every part belongs to the single `None` scope.
fn owner_index_map(pkg: &Package) -> HashMap<String, usize> {
    let owners = owner_scopes(pkg);
    let mut map: HashMap<String, usize> = HashMap::new();
    if owners.len() == 1 && owners[0].is_none() {
        for e in pkg.entries.iter().filter(|e| !e.is_dir) {
            map.insert(e.name.to_ascii_lowercase(), 0);
        }
        return map;
    }
    for (i, owner) in owners.iter().enumerate() {
        let Some(owner) = owner else { continue };
        let mut queue = vec![owner.clone()];
        let mut visited: HashSet<String> = HashSet::new();
        while let Some(part) = queue.pop() {
            let key = part.to_ascii_lowercase();
            if !visited.insert(key.clone()) {
                continue;
            }
            map.entry(key).or_insert(i);
            let want = rels_part_name(&part);
            if let Some(rp) = pkg.rels.iter().find(|r| r.name.eq_ignore_ascii_case(&want)) {
                for r in &rp.rels {
                    if tail(&r.kind) == "glossaryDocument" {
                        continue;
                    }
                    if let Some(t) = &r.target_abs
                        && let Some(name) = canonical_entry(pkg, t)
                    {
                        queue.push(name);
                    }
                }
            }
        }
    }
    // A part not reached from any owner scope (for example a lone supporting part discovered only
    // by its content type) belongs to the primary scope, so the simple fixtures keep working.
    for e in pkg.entries.iter().filter(|e| !e.is_dir) {
        map.entry(e.name.to_ascii_lowercase()).or_insert(0);
    }
    map
}

/// The owner index a part belongs to, if it is reachable from an owner scope.
fn part_owner(owner_map: &HashMap<String, usize>, part: &str) -> Option<usize> {
    owner_map.get(&part.to_ascii_lowercase()).copied()
}

/// Verify that style references in story parts resolve to a defined style in their owner scope.
fn check_style_refs(pkg: &Package, facts: &scan::StoryFacts, acc: &mut Acc) {
    let cap = acc.limits.facts;
    let owners = owner_scopes(pkg);
    let owner_map = owner_index_map(pkg);
    let mut per_owner: Vec<Option<HashSet<String>>> = Vec::with_capacity(owners.len());
    let mut present = false;
    let mut capped = false;
    let mut parsed_any = false;
    for owner in &owners {
        let Some(entry) = supporting_part(pkg, owner.as_deref(), "styles", package::STYLES_CONTENT_TYPE) else {
            per_owner.push(None);
            continue;
        };
        present = true;
        let mut guard = scan::ParseGuard::new(acc.limits.events, acc.limits.depth);
        let parsed = entry.decoded.as_deref().map(|text| scan::parse_style_ids(text, cap, &mut guard));
        if guard.exhausted() {
            acc.note(format!("{} exceeded the XML {} budget; style definitions were not checked", entry.name, guard.budget()));
            acc.set_incomplete();
            per_owner.push(None);
            continue;
        }
        match parsed {
            Some(Some((ids, c))) => {
                parsed_any = true;
                capped |= c;
                per_owner.push(Some(ids));
            }
            Some(None) => per_owner.push(None),
            None => per_owner.push(None),
        }
    }
    if capped {
        acc.note(format!("a styles part exceeded the {cap}-entry budget; later style ids were not recorded"));
        acc.set_incomplete();
    }
    if !present {
        if !facts.style_refs.is_empty() {
            acc.note("no styles part is reachable; w:pStyle/w:rStyle references were not checked".to_string());
        }
        return;
    }
    if !parsed_any {
        if !facts.style_refs.is_empty() {
            acc.note("a styles part is present but does not parse; w:pStyle/w:rStyle references were not checked".to_string());
        }
        return;
    }
    // Check each reference against its owner's own definitions only (H2).
    let mut reported: HashSet<(&str, &str)> = HashSet::new();
    for (id, part) in &facts.style_refs {
        let Some(owner) = part_owner(&owner_map, part) else { continue };
        let Some(Some(defs)) = per_owner.get(owner) else { continue };
        if !defs.contains(id) && reported.insert((id.as_str(), part.as_str())) {
            acc.add("REF-STYLE", Some(part.clone()), format!("style id {id} in {part} has no definition in the package"));
        }
    }
}

/// The vocabulary family: id and name collisions inside the styles,
/// fontTable, and custom-properties parts. Each part is judged in its
/// own scope; a part that is present but cannot be read or parsed
/// makes the run INCOMPLETE for these rules rather than passing them
/// silently (absence of the part itself is legitimate and silent).
fn check_vocabulary(pkg: &Package, acc: &mut Acc) {
    let cap = acc.limits.facts;
    let owners = owner_scopes(pkg);
    for owner in &owners {
        if let Some(entry) = supporting_part(pkg, owner.as_deref(), "styles", package::STYLES_CONTENT_TYPE) {
            check_styles_vocabulary(entry, cap, acc);
        }
        if let Some(entry) = supporting_part(pkg, owner.as_deref(), "fontTable", package::FONT_TABLE_CONTENT_TYPE) {
            check_name_dups(entry, cap, acc, true);
        }
    }
    if let Some(entry) = find_by_type(pkg, package::CUSTOM_PROPERTIES_CONTENT_TYPE) {
        check_name_dups(entry, cap, acc, false);
    }
    acc.ran("STYLE-DUP-ID");
    acc.ran("STYLE-DUP-NAME");
    acc.ran("STYLE-RESERVED-NAME");
    acc.ran("STYLE-RESERVED-ID");
    acc.ran("STYLE-COUNT");
    acc.ran("STYLE-ID-LENGTH");
    acc.ran("FONT-DUP-NAME");
    acc.ran("PROP-DUP-NAME");
}

/// Evaluate the six styles-part vocabulary rules for one styles part.
fn check_styles_vocabulary(entry: &package::Entry, cap: usize, acc: &mut Acc) {
    let part = entry.name.clone();
    let mut guard = scan::ParseGuard::new(acc.limits.events, acc.limits.depth);
    let parsed = entry.decoded.as_deref().map(|text| scan::parse_style_defs(text, cap, &mut guard));
    if guard.exhausted() {
        acc.note(format!("{part} exceeded the XML {} budget; style vocabulary was not checked", guard.budget()));
        acc.set_incomplete();
        return;
    }
    let Some(Some((defs, capped, total))) = parsed else {
        acc.note(format!("{part} is present but does not parse; style vocabulary was not checked"));
        acc.set_incomplete();
        return;
    };
    if capped {
        acc.note(format!("{part} exceeded the {cap}-entry budget; later style definitions were not recorded"));
        acc.set_incomplete();
    }
    if total > 4079 {
        acc.add(
            "STYLE-COUNT",
            Some(part.clone()),
            format!("{part} defines {total} styles; MS-OI29500 states Word does not open a document containing more than 4,079 styles (Word for the web was observed opening 4,080 cleanly)"),
        );
    }
    let mut ids: HashMap<String, String> = HashMap::new();
    let mut names: HashMap<String, String> = HashMap::new();
    let mut reported_keys: HashSet<(&'static str, String)> = HashSet::new();
    for def in &defs {
        if let Some(id) = &def.id {
            if id.chars().count() > 253 {
                acc.add(
                    "STYLE-ID-LENGTH",
                    Some(part.clone()),
                    format!("styleId {id} in {part} is longer than 253 characters; MS-OI29500 states Word ignores such a value, leaving the style unreferenceable"),
                );
            }
            let folded = id.to_lowercase();
            if let Some(first) = ids.get(&folded) {
                if reported_keys.insert(("STYLE-DUP-ID", folded.clone())) {
                    acc.add(
                        "STYLE-DUP-ID",
                        Some(part.clone()),
                        format!("styleId {first} is defined more than once in {part} (again as {id}); duplicate definitions are reassigned arbitrarily and their references are not repaired"),
                    );
                }
            } else {
                ids.insert(folded.clone(), id.clone());
            }
            let reserved_type = match folded.as_str() {
                "nolist" => Some("numbering"),
                "defaultparagraphfont" => Some("character"),
                "tablenormal" => Some("table"),
                _ => None,
            };
            if let Some(required) = reserved_type
                && let Some(declared) = &def.style_type
                && declared.to_lowercase() != required
                && reported_keys.insert(("STYLE-RESERVED-ID", folded.clone()))
            {
                acc.add(
                    "STYLE-RESERVED-ID",
                    Some(part.clone()),
                    format!("style {id} in {part} has type {declared}; MS-OI29500 documents this id as a {required} style whose child elements Word ignores"),
                );
            }
        }
        if let Some(name) = &def.name {
            let folded = name.trim().to_lowercase();
            if let Some(first) = names.get(&folded) {
                if reported_keys.insert(("STYLE-DUP-NAME", folded.clone())) {
                    acc.add(
                        "STYLE-DUP-NAME",
                        Some(part.clone()),
                        format!("primary style name {first} is used by more than one style in {part} (again as {})", name.trim()),
                    );
                }
            } else {
                names.insert(folded.clone(), name.trim().to_string());
            }
            if let Some(required) = reserved::reserved_style_type(&folded)
                && !def.style_type.as_deref().is_some_and(|t| t.to_lowercase() == required)
                && reported_keys.insert(("STYLE-RESERVED-NAME", folded.clone()))
            {
                let declared = def.style_type.as_deref().unwrap_or("(omitted)");
                acc.add(
                    "STYLE-RESERVED-NAME",
                    Some(part.clone()),
                    format!("style name {} in {part} is a reserved built-in name requiring type {required}, but the style has type {declared}; Word renames the style or changes its type on load", name.trim()),
                );
            }
        }
    }
}

/// FONT-DUP-NAME / PROP-DUP-NAME: one folded-name collision report
/// per duplicated name in a fontTable or custom-properties part.
fn check_name_dups(entry: &package::Entry, cap: usize, acc: &mut Acc, fonts: bool) {
    let part = entry.name.clone();
    let rule = if fonts { "FONT-DUP-NAME" } else { "PROP-DUP-NAME" };
    let mut guard = scan::ParseGuard::new(acc.limits.events, acc.limits.depth);
    let parsed = entry
        .decoded
        .as_deref()
        .map(|text| if fonts { scan::parse_font_names(text, cap, &mut guard) } else { scan::parse_custom_prop_names(text, cap, &mut guard) });
    if guard.exhausted() {
        acc.note(format!("{part} exceeded the XML {} budget; {rule} was not checked", guard.budget()));
        acc.set_incomplete();
        return;
    }
    let Some(Some((names, capped))) = parsed else {
        acc.note(format!("{part} is present but does not parse; {rule} was not checked"));
        acc.set_incomplete();
        return;
    };
    if capped {
        acc.note(format!("{part} exceeded the {cap}-entry budget; later names were not recorded"));
        acc.set_incomplete();
    }
    let mut seen: HashMap<String, String> = HashMap::new();
    let mut reported: HashSet<String> = HashSet::new();
    for name in &names {
        let folded = name.to_lowercase();
        if let Some(first) = seen.get(&folded) {
            if reported.insert(folded) {
                let message = if fonts {
                    format!(
                        "font name {first} appears more than once in {part} (again as {name}); font names are case-insensitive join keys, and Word shows a corruption notice for this collision"
                    )
                } else {
                    format!(
                        "custom property name {first} appears more than once in {part} (again as {name}); property names must be case-insensitively unique, and Word shows a corruption notice for this collision"
                    )
                };
                acc.add(rule, Some(part.clone()), message);
            }
        } else {
            seen.insert(folded, name.clone());
        }
    }
}

/// The part whose effective content type is `ct`, if any. Supporting parts are found by type, not
/// by their conventional file name or folder.
fn find_by_type<'a>(pkg: &'a Package, ct: &str) -> Option<&'a package::Entry> {
    pkg.entries.iter().find(|e| !e.is_dir && pkg.content_types.of(&e.name) == Some(ct))
}

/// Verify numbering definitions and references in story and styles parts for each owner scope.
fn check_numbering(pkg: &Package, facts: &scan::StoryFacts, acc: &mut Acc) {
    acc.ran("NUM-DANGLING");
    let cap = acc.limits.facts;
    let owners = owner_scopes(pkg);
    let owner_map = owner_index_map(pkg);
    let mut numbering_cache: HashMap<String, Option<HashSet<String>>> = HashMap::new();
    // Each owner's numbering set, in owner order. `Some(set)` = the owner's numbering part was
    // examined, so `set` is the definitions it declares (empty when the owner has no numbering part
    // — established absent, so its references are genuinely dangling). `None` = the owner's
    // numbering could not be established: the part exists but could not be examined, or the
    // discovery source (its `.rels`, or the manifest in the no-owner scope) could not be read, so
    // that owner's references are not established as dangling and its dependent checks are skipped
    // (H8).
    let mut num_sets: Vec<Option<HashSet<String>>> = Vec::with_capacity(owners.len());
    let mut style_checks: Vec<(String, HashSet<String>)> = Vec::new();
    let mut seen_styles: HashSet<String> = HashSet::new();
    for owner in &owners {
        let num_part = supporting_part(pkg, owner.as_deref(), "numbering", package::NUMBERING_CONTENT_TYPE).map(|e| e.name.clone());
        let num_set: Option<HashSet<String>> = match &num_part {
            Some(np) => {
                if let Some(cached) = numbering_cache.get(np) {
                    cached.clone()
                } else {
                    let parsed = process_numbering_part(pkg, np, cap, acc);
                    numbering_cache.insert(np.clone(), parsed.clone());
                    parsed
                }
            }
            None => {
                // A readable discovery source that reaches no numbering part establishes that the
                // owner has none (an empty set, exactly as before). When the discovery source
                // itself could not be read, the owner's numbering is unknown, not absent: its
                // references must not be reported against an empty set (H8).
                if discovery_unavailable(pkg, owner.as_deref()) { None } else { Some(HashSet::new()) }
            }
        };
        // Each owner's styles are checked against that owner's own numbering set, and only when it
        // could actually be examined.
        if let Some(entry) = supporting_part(pkg, owner.as_deref(), "styles", package::STYLES_CONTENT_TYPE)
            && seen_styles.insert(entry.name.to_ascii_lowercase())
            && let Some(set) = &num_set
        {
            style_checks.push((entry.name.clone(), set.clone()));
        }
        num_sets.push(num_set);
    }
    report_num_refs(acc, facts, &num_sets, &owner_map);
    for (name, num_set) in &style_checks {
        report_style_num_refs(pkg, name, num_set, cap, acc);
    }
}

/// Parse a numbering part, reporting duplicate definitions and dangling `w:abstractNumId`
/// references. `None` when the part could not be read, did not parse, or exceeded a budget (its
/// references must not be checked).
fn process_numbering_part(pkg: &Package, name: &str, cap: usize, acc: &mut Acc) -> Option<HashSet<String>> {
    let entry = entry_by_name(pkg, name)?;
    let Some(text) = entry.decoded.as_deref() else {
        acc.note(format!("{name} could not be read; list numbering references were not checked"));
        return None;
    };
    let mut guard = scan::ParseGuard::new(acc.limits.events, acc.limits.depth);
    let defs = scan::parse_numbering(text, cap, &mut guard);
    if guard.exhausted() {
        acc.note(format!("{name} exceeded the XML {} budget; list numbering references were not checked", guard.budget()));
        acc.set_incomplete();
        return None;
    }
    let Some(defs) = defs else {
        acc.note(format!("{name} does not parse; list numbering references were not checked"));
        return None;
    };
    if defs.capped {
        acc.note(format!("{name} exceeded the {cap}-entry budget; later numbering definitions were not recorded"));
        acc.set_incomplete();
    }
    report_dups_values(acc, &defs.num_ids, &entry.name, "w:num numId");
    report_dups_values(acc, &defs.abstract_ids, &entry.name, "w:abstractNum abstractNumId");
    let abstract_set: HashSet<&str> = defs.abstract_ids.iter().map(String::as_str).collect();
    for (num, abs) in &defs.num_to_abstract {
        match abs {
            Some(a) if !abstract_set.contains(a.as_str()) => acc.add(
                "NUM-DANGLING",
                Some(entry.name.clone()),
                format!("w:num numId {num} references w:abstractNumId {a}, which has no w:abstractNum"),
            ),
            None => acc.add("NUM-DANGLING", Some(entry.name.clone()), format!("w:num numId {num} has no w:abstractNumId")),
            _ => {}
        }
    }
    Some(defs.num_ids.into_iter().collect())
}

/// `NUM-DANGLING` for `w:numPr/w:numId` references in a styles part, checked against its owner's
/// numbering part. `numId` `0` is the "remove numbering" sentinel and is never a reference.
fn report_style_num_refs(pkg: &Package, styles_name: &str, num_set: &HashSet<String>, cap: usize, acc: &mut Acc) {
    let Some(entry) = entry_by_name(pkg, styles_name) else { return };
    let Some(text) = entry.decoded.as_deref() else {
        acc.note(format!("{styles_name} could not be read; style numbering references were not checked"));
        return;
    };
    let mut guard = scan::ParseGuard::new(acc.limits.events, acc.limits.depth);
    let parsed = scan::parse_style_num_ids(text, cap, &mut guard);
    if guard.exhausted() {
        acc.note(format!("{styles_name} exceeded the XML {} budget; style numbering references were not checked", guard.budget()));
        acc.set_incomplete();
        return;
    }
    let Some((vals, capped)) = parsed else {
        acc.note(format!("{styles_name} does not parse; style numbering references were not checked"));
        return;
    };
    if capped {
        acc.note(format!("{styles_name} exceeded the {cap}-entry budget; later numbering references were not recorded"));
        acc.set_incomplete();
    }
    let mut reported: HashSet<&str> = HashSet::new();
    for val in &vals {
        if val == "0" {
            continue;
        }
        if !num_set.contains(val.as_str()) && reported.insert(val.as_str()) {
            acc.add(
                "NUM-DANGLING",
                Some(entry.name.clone()),
                format!("a w:numPr in {} references numId {val}, which has no w:num definition", entry.name),
            );
        }
    }
}

/// `NUM-DANGLING` for `w:numPr/w:numId` references in story parts, each checked against its owner's
/// own numbering set. A reference whose owner's numbering could not be examined is skipped, so an
/// unavailable numbering part never manufactures a dangling finding (H2/H8).
fn report_num_refs(acc: &mut Acc, facts: &scan::StoryFacts, num_sets: &[Option<HashSet<String>>], owner_map: &HashMap<String, usize>) {
    let mut reported: HashSet<(&str, &str)> = HashSet::new();
    for (val, part) in &facts.num_ids {
        if val == "0" {
            continue;
        }
        let Some(owner) = part_owner(owner_map, part) else { continue };
        let Some(Some(set)) = num_sets.get(owner) else { continue };
        if !set.contains(val) && reported.insert((val.as_str(), part.as_str())) {
            acc.add("NUM-DANGLING", Some(part.clone()), format!("w:numPr references numId {val}, which has no w:num definition"));
        }
    }
}

/// Report duplicate string values within a part for a specified attribute/construct name.
fn report_dups_values(acc: &mut Acc, values: &[String], part: &str, what: &str) {
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for v in values {
        *counts.entry(v.as_str()).or_default() += 1;
    }
    let mut dups: Vec<&str> = counts.into_iter().filter(|(_, c)| *c > 1).map(|(v, _)| v).collect();
    dups.sort_unstable();
    for v in dups {
        acc.add("NUM-DANGLING", Some(part.to_string()), format!("duplicate {what} {v} in {part}"));
    }
}

/// Verify comment-threading relationships between comments, commentsExtended, and commentsIds parts.
fn check_commentex(pkg: &Package, facts: &scan::StoryFacts, acc: &mut Acc) {
    let cap = acc.limits.facts;
    let owners = owner_scopes(pkg);
    let owner_map = owner_index_map(pkg);
    let any_ext = owners.iter().any(|o| supporting_part(pkg, o.as_deref(), "commentsExtended", package::COMMENTS_EXTENDED_CONTENT_TYPE).is_some());
    if !any_ext {
        return;
    }
    acc.ran("COMMENTEX-LINK");
    for (i, owner) in owners.iter().enumerate() {
        // If the owner's comments part could not be read, its paragraph ids are unknown, so no
        // commentEx entry is established as unlinked; likewise, if its commentsExtended part could
        // not be read, its entries and the commentsIds join cannot be checked. Absence is not
        // established from unavailable content.
        if supporting_part(pkg, owner.as_deref(), "comments", package::COMMENTS_CONTENT_TYPE).is_some_and(|e| e.content_unavailable) {
            continue;
        }
        if supporting_part(pkg, owner.as_deref(), "commentsExtended", package::COMMENTS_EXTENDED_CONTENT_TYPE).is_some_and(|e| e.content_unavailable)
        {
            continue;
        }
        // Each owner's comment-threading is checked within its own scope: its commentEx entries
        // against its own comments part, and its commentsIds paraIds against its own commentEx.
        let comment_para: HashSet<&str> =
            facts.comment_para_ids.iter().filter(|(_, p)| part_owner(&owner_map, p) == Some(i)).map(|(p, _)| p.as_str()).collect();
        let ext = supporting_part(pkg, owner.as_deref(), "commentsExtended", package::COMMENTS_EXTENDED_CONTENT_TYPE)
            .and_then(|entry| read_comments_ext(entry, cap, acc).map(|e| (entry.name.clone(), e)));
        let ex_para: HashSet<&str> = match &ext {
            Some((_, e)) => e.entries.iter().map(|(p, _)| p.as_str()).collect(),
            None => HashSet::new(),
        };
        if let Some((name, e)) = &ext {
            for (para, parent) in &e.entries {
                if !comment_para.contains(para.as_str()) {
                    acc.add(
                        "COMMENTEX-LINK",
                        Some(name.clone()),
                        format!("w15:commentEx paraId {para} has no paragraph with that w14:paraId in comments.xml"),
                    );
                }
                if let Some(p) = parent
                    && !ex_para.contains(p.as_str())
                {
                    acc.add(
                        "COMMENTEX-LINK",
                        Some(name.clone()),
                        format!("w15:commentEx paraId {para} has paraIdParent {p}, which matches no commentEx paraId"),
                    );
                }
            }
        }
        // commentsIds ids keep their own part name, so an unmatched id is reported at the part it
        // came from, never at another owner's (H9).
        let cid = supporting_part(pkg, owner.as_deref(), "commentsIds", package::COMMENTS_IDS_CONTENT_TYPE)
            .and_then(|entry| read_comments_ids(entry, cap, acc));
        let Some((cid_name, cid_ids)) = &cid else { continue };
        let cid_set: HashSet<&str> = cid_ids.iter().map(String::as_str).collect();
        for id in cid_ids {
            if !ex_para.contains(id.as_str()) {
                acc.add("COMMENTEX-LINK", Some(cid_name.clone()), format!("commentsIds paraId {id} has no commentEx counterpart"));
            }
        }
        if let Some((name, e)) = &ext {
            for (para, _) in &e.entries {
                if !cid_set.contains(para.as_str()) {
                    acc.add("COMMENTEX-LINK", Some(name.clone()), format!("commentEx paraId {para} has no commentsIds paraId counterpart"));
                }
            }
        }
    }
}

/// Parse a commentsExtended part, recording a note (and INCOMPLETE for a budget exhaustion) when it
/// cannot be checked. `None` when the part's link checks must be skipped.
fn read_comments_ext(entry: &package::Entry, cap: usize, acc: &mut Acc) -> Option<scan::CommentEx> {
    let name = &entry.name;
    let Some(text) = entry.decoded.as_deref() else {
        acc.note(format!("{name} could not be read; comment-threading links were not checked"));
        return None;
    };
    let mut guard = scan::ParseGuard::new(acc.limits.events, acc.limits.depth);
    let ext = scan::parse_comments_extended(text, cap, &mut guard);
    if guard.exhausted() {
        acc.note(format!("{name} exceeded the XML {} budget; comment-threading links were not checked", guard.budget()));
        acc.set_incomplete();
        return None;
    }
    let Some(ext) = ext else {
        acc.note(format!("{name} does not parse; comment-threading links were not checked"));
        return None;
    };
    if ext.capped {
        acc.note(format!("{name} exceeded the {cap}-entry budget; later commentEx entries were not recorded"));
        acc.set_incomplete();
    }
    Some(ext)
}

/// Parse a commentsIds part, returning its part name and `w16cid:paraId` values. `None` when the part's link
/// checks must be skipped.
fn read_comments_ids(entry: &package::Entry, cap: usize, acc: &mut Acc) -> Option<(String, Vec<String>)> {
    let name = &entry.name;
    let text = entry.decoded.as_deref()?;
    let mut guard = scan::ParseGuard::new(acc.limits.events, acc.limits.depth);
    let parsed = scan::parse_comments_ids(text, cap, &mut guard);
    if guard.exhausted() {
        acc.note(format!("{name} exceeded the XML {} budget; comment-threading links were not checked", guard.budget()));
        acc.set_incomplete();
        return None;
    }
    let (ids, capped) = parsed?;
    if capped {
        acc.note(format!("{name} exceeded the {cap}-entry budget; later paraIds were not recorded"));
        acc.set_incomplete();
    }
    Some((entry.name.clone(), ids))
}

/// Verify that custom XML itemProperties parts only reference schemas declared in settings.xml.
fn check_custom_xml(pkg: &Package, acc: &mut Acc) {
    acc.ran("CUSTOMXML-SCHEMA");
    let cap = acc.limits.facts;
    let owners = owner_scopes(pkg);
    let owner_map = owner_index_map(pkg);
    // Schema availability and definitions are tracked per owner (J3): a glossary-only attached
    // schema cannot satisfy a main-owned reference, and an unreadable settings part disables the
    // checks for its owner alone. `None` = that owner's settings could not be established, so its
    // customXml parts are skipped.
    let mut per_owner: Vec<Option<HashSet<String>>> = Vec::with_capacity(owners.len());
    // The owners whose customXml item-properties parts exist, so whose schema references would have
    // been checked. A note about a skipped check is only worth recording for such an owner; an owner
    // with no customXml parts loses nothing when the check is skipped (Opus N5). When the manifest
    // that types the parts could not be read the attribution itself is unavailable, so the note is
    // kept.
    let custom_xml_owners: HashSet<usize> = pkg
        .entries
        .iter()
        .filter(|e| !e.is_dir && pkg.content_types.of(&e.name) == Some(package::CUSTOM_XML_PROPS_CONTENT_TYPE))
        .filter_map(|e| part_owner(&owner_map, &e.name))
        .collect();
    for (i, owner) in owners.iter().enumerate() {
        // The settings part is discovered through the owner's own `.rels`, or in the no-owner
        // fallback scope through the manifest (`find_by_type`). When that source could not be read,
        // which settings part (and so which schemas) the owner reaches is UNKNOWN, not absent: skip
        // the owner rather than substituting an empty schema set, which would report a valid schema
        // reference as unattached. Applying the same predicate in both scopes keeps established
        // absence (a readable source that reaches no settings) a genuine empty set. (The run is
        // already INCOMPLETE from loading the unreadable source.)
        if discovery_unavailable(pkg, owner.as_deref()) {
            if custom_xml_owners.contains(&i) || package::content_unavailable(pkg, "[Content_Types].xml") {
                let source = match owner.as_deref() {
                    Some(o) => rels_part_name(o),
                    None => "[Content_Types].xml".to_string(),
                };
                acc.note(format!("{source} could not be read; custom XML schema references were not checked"));
            }
            acc.set_incomplete();
            per_owner.push(None);
            continue;
        }
        let Some(entry) = supporting_part(pkg, owner.as_deref(), "settings", package::SETTINGS_CONTENT_TYPE) else {
            // The owner's `.rels` was readable and holds no settings relationship: there is
            // genuinely no settings part for this owner, so the attached-schema set is empty.
            per_owner.push(Some(HashSet::new()));
            continue;
        };
        let Some(text) = entry.decoded.as_deref() else {
            acc.note(format!("{} could not be read; custom XML schema references were not checked", entry.name));
            acc.set_incomplete();
            per_owner.push(None);
            continue;
        };
        let mut guard = scan::ParseGuard::new(acc.limits.events, acc.limits.depth);
        match scan::parse_attached_schemas(text, cap, &mut guard) {
            Some((vals, capped)) if !guard.exhausted() => {
                if capped {
                    acc.note(format!("{} exceeded the {cap}-entry budget; later attached schemas were not recorded", entry.name));
                    acc.set_incomplete();
                }
                per_owner.push(Some(vals.into_iter().collect()));
            }
            result => {
                if guard.exhausted() {
                    acc.note(format!("{} exceeded the XML {} budget; custom XML schema references were not checked", entry.name, guard.budget()));
                } else {
                    debug_assert!(result.is_none());
                    acc.note(format!("{} does not parse; custom XML schema references were not checked", entry.name));
                }
                acc.set_incomplete();
                per_owner.push(None);
            }
        }
    }
    for e in pkg.entries.iter().filter(|e| !e.is_dir) {
        if pkg.content_types.of(&e.name) != Some(package::CUSTOM_XML_PROPS_CONTENT_TYPE) {
            continue;
        }
        let Some(owner) = part_owner(&owner_map, &e.name) else { continue };
        let Some(Some(schemas)) = per_owner.get(owner) else { continue };
        let Some(text) = e.decoded.as_deref() else { continue };
        let mut guard = scan::ParseGuard::new(acc.limits.events, acc.limits.depth);
        let (uris, capped) = scan::parse_custom_xml_schemas(text, cap, &mut guard);
        if guard.exhausted() {
            acc.note(format!("{} exceeded the XML {} budget; custom XML schema references were not checked", e.name, guard.budget()));
            acc.set_incomplete();
            continue;
        }
        if capped {
            acc.note(format!("{} exceeded the {cap}-entry budget; later schema references were not recorded", e.name));
            acc.set_incomplete();
        }
        for uri in uris {
            if !schemas.contains(&uri) {
                acc.add(
                    "CUSTOMXML-SCHEMA",
                    Some(e.name.clone()),
                    format!("custom XML part {} declares schema {uri}, which is not attached in settings.xml", e.name),
                );
            }
        }
    }
}

/// Report duplicate identifier findings for a given rule across items.
fn report_dups(acc: &mut Acc, rule: &'static str, items: &[(String, String)], what: &str) {
    let mut first: HashMap<&str, &str> = HashMap::new();
    let mut reported: HashSet<&str> = HashSet::new();
    for (id, part) in items {
        match first.entry(id.as_str()) {
            Entry::Vacant(v) => {
                v.insert(part.as_str());
            }
            Entry::Occupied(o) => {
                if reported.insert(id.as_str()) {
                    acc.add(rule, Some(part.clone()), format!("duplicate {what} {id} (first in {})", o.get()));
                }
            }
        }
    }
}

/// Report missing definitions for references collected from story elements.
fn report_missing(acc: &mut Acc, rule: &'static str, refs: &[(String, String)], defs: &HashSet<String>, what: &str) {
    let mut reported: HashSet<&str> = HashSet::new();
    for (id, part) in refs {
        if !defs.contains(id) && reported.insert(id.as_str()) {
            acc.add(rule, Some(part.clone()), format!("{what} {id} in {part} has no definition in the package"));
        }
    }
}

/// Report missing footnote or endnote definitions.
fn report_missing_note(acc: &mut Acc, refs: &[(String, String)], defs: &HashSet<String>, kind: &str) {
    let mut reported: HashSet<&str> = HashSet::new();
    for (id, part) in refs {
        if !defs.contains(id) && reported.insert(id.as_str()) {
            acc.add(
                "REF-NOTE",
                Some(part.clone()),
                format!("w:{kind}Reference id {id} in {part} has no w:{kind} definition; the note body is absent"),
            );
        }
    }
}

/// Verify that media files in `word/media/` or targeted by image relationships match magic byte signatures.
fn check_media(pkg: &Package, acc: &mut Acc) {
    acc.ran("MEDIA-MAGIC");
    let image_targets = image_rel_targets(pkg);
    for e in pkg.entries.iter().filter(|e| !e.is_dir) {
        let l = e.name.to_ascii_lowercase();
        if !l.starts_with("word/media/") && !image_targets.contains(&l) {
            continue;
        }
        let Some(bytes) = &e.bytes else { continue };
        let Some(kind) = media_kind(&e.name, pkg.content_types.of(&e.name)) else { continue };
        if !magic_matches(kind, bytes) {
            acc.add("MEDIA-MAGIC", Some(e.name.clone()), format!("{} does not have the magic bytes of a {kind} file", e.name));
        }
    }
}

/// Collect lowercased target paths for all image relationships in the package.
fn image_rel_targets(pkg: &Package) -> HashSet<String> {
    let mut set = HashSet::new();
    for rp in &pkg.rels {
        for r in &rp.rels {
            if tail(&r.kind) == "image"
                && let Some(t) = &r.target_abs
            {
                set.insert(t.to_ascii_lowercase());
            }
        }
    }
    set
}

/// Determine the expected media format kind from file extension or content type.
fn media_kind(name: &str, ct: Option<&str>) -> Option<&'static str> {
    let ext = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase());
    match ext.as_deref() {
        Some("png") => Some("png"),
        Some("jpg" | "jpeg") => Some("jpeg"),
        Some("gif") => Some("gif"),
        Some("bmp") => Some("bmp"),
        Some("tif" | "tiff") => Some("tiff"),
        _ => ct.and_then(|c| match c {
            "image/png" => Some("png"),
            "image/jpeg" => Some("jpeg"),
            "image/gif" => Some("gif"),
            "image/bmp" => Some("bmp"),
            "image/tiff" => Some("tiff"),
            _ => None,
        }),
    }
}

/// Check if the beginning bytes match the expected magic signature for `kind`.
fn magic_matches(kind: &str, bytes: &[u8]) -> bool {
    match kind {
        "png" => bytes.starts_with(b"\x89PNG"),
        "jpeg" => bytes.starts_with(b"\xFF\xD8\xFF"),
        "gif" => bytes.starts_with(b"GIF8"),
        "bmp" => bytes.starts_with(b"BM"),
        "tiff" => bytes.starts_with(b"\x49\x49\x2A\x00") || bytes.starts_with(b"\x4D\x4D\x00\x2A"),
        _ => true,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod budget_tests {
    //! F1/F2 canaries: every budget has a test that proves exhausting it yields INCOMPLETE with a
    //! coverage note rather than an unbounded run or a silent pass.

    use super::*;
    use crate::{package, scan};

    /// Build an uncompressed in-memory zip archive with the specified entry names and payloads.
    fn tiny_zip(entries: &[(&str, Vec<u8>)]) -> Vec<u8> {
        use std::io::{Cursor, Write};
        use zip::write::SimpleFileOptions;
        let mut w = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let opts = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
        for (name, data) in entries {
            w.start_file(*name, opts).unwrap();
            w.write_all(data).unwrap();
        }
        w.finish().unwrap().into_inner()
    }

    /// Build minimal `[Content_Types].xml` bytes declaring default rels/xml and the main document override.
    fn content_types() -> Vec<u8> {
        format!(
            r#"<?xml version="1.0"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="{}"/></Types>"#,
            package::MAIN_CONTENT_TYPE
        )
        .into_bytes()
    }

    /// Build root `_rels/.rels` bytes with an `officeDocument` relationship.
    fn package_rels() -> Vec<u8> {
        br#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#.to_vec()
    }

    /// Build an empty `word/_rels/document.xml.rels` relationships part.
    fn doc_rels() -> Vec<u8> {
        br#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"/>"#.to_vec()
    }

    /// Build a relationships part XML with the given body content.
    fn rels(body: &str) -> Vec<u8> {
        format!(r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">{body}</Relationships>"#)
            .into_bytes()
    }

    /// Build a minimal `word/document.xml` part containing an empty paragraph.
    fn document() -> Vec<u8> {
        format!(r#"<?xml version="1.0"?><w:document xmlns:w="{}"><w:body><w:p/></w:body></w:document>"#, NS_W).into_bytes()
    }

    /// Build `[Content_Types].xml` bytes including an override for `part`.
    fn content_types_with_override(part: &str, ct: &str) -> Vec<u8> {
        format!(
            r#"<?xml version="1.0"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="{}"/><Override PartName="/{part}" ContentType="{ct}"/></Types>"#,
            package::MAIN_CONTENT_TYPE
        )
        .into_bytes()
    }

    /// Build a minimal valid package zip as raw bytes.
    fn minimal_zip() -> Vec<u8> {
        tiny_zip(&[
            ("[Content_Types].xml", content_types()),
            ("_rels/.rels", package_rels()),
            ("word/document.xml", document()),
            ("word/_rels/document.xml.rels", doc_rels()),
        ])
    }

    #[test]
    /// A main part over the per-part byte budget is unreadable content: the verdict is INCOMPLETE, never a manufactured verdict on a part that was not read.
    fn oversized_main_part_is_incomplete() {
        let zip = tiny_zip(&[
            ("[Content_Types].xml", content_types()),
            ("_rels/.rels", package_rels()),
            ("word/document.xml", vec![b'x'; 5000]),
            ("word/_rels/document.xml.rels", doc_rels()),
        ]);
        let mut acc = Acc::with_limits(Limits { part_bytes: 4096, ..Limits::default() });
        let _ = package::load(&zip, &mut acc);
        let report = acc.finish();
        assert_eq!(report.verdict, Verdict::Incomplete, "findings: {:?}", report.findings);
        assert!(report.coverage_notes.iter().any(|n| n.contains("document.xml")), "{:?}", report.coverage_notes);
    }

    #[test]
    /// A central directory longer than the entry cap truncates the raw duplicate scan, so the run is INCOMPLETE rather than a duplicate-clean PASS.
    fn entry_cap_truncation_is_incomplete() {
        let zip = minimal_zip();
        let mut acc = Acc::with_limits(Limits { entries: 3, ..Limits::default() });
        let _ = package::load(&zip, &mut acc);
        let report = acc.finish();
        assert_eq!(report.verdict, Verdict::Incomplete, "findings: {:?}", report.findings);
    }

    #[test]
    /// Input over the total byte budget is INCOMPLETE.
    fn input_over_the_byte_budget_is_incomplete() {
        let report = validate_bytes_limited(b"definitely not a zip", Limits { input_bytes: 4, ..Limits::default() });
        assert_eq!(report.verdict, Verdict::Incomplete);
        assert!(report.findings.is_empty(), "{:?}", report.findings);
    }

    #[test]
    /// A part nested past the depth limit stops the scan and marks the run INCOMPLETE instead of recursing without bound.
    fn deeply_nested_part_is_incomplete_without_hanging() {
        let mut xml = format!(r#"<w:document xmlns:w="{}">"#, NS_W);
        xml.push_str(&"<w:p>".repeat(2000));
        xml.push_str(&"</w:p>".repeat(2000));
        xml.push_str("</w:document>");
        let mut acc = Acc::with_limits(Limits { depth: 512, ..Limits::default() });
        let role = scan::PartRole { is_main: true, is_story: true, ..scan::PartRole::default() };
        let _ = scan::scan_part("word/document.xml", &xml, role, &mut acc);
        let report = acc.finish();
        assert_eq!(report.verdict, Verdict::Incomplete, "findings: {:?}", report.findings);
        assert!(!report.findings.iter().any(|f| f.rule == "XML-MALFORMED"), "{:?}", report.findings);
    }

    #[test]
    /// A part with more elements than the event budget stops the scan and marks the run INCOMPLETE.
    fn extreme_element_count_stops_within_budget() {
        let xml = format!(r#"<w:document xmlns:w="{}">{}</w:document>"#, NS_W, "<w:p/>".repeat(10_000));
        let mut acc = Acc::with_limits(Limits { events: 1000, ..Limits::default() });
        let role = scan::PartRole { is_main: true, is_story: true, ..scan::PartRole::default() };
        let _ = scan::scan_part("word/document.xml", &xml, role, &mut acc);
        let report = acc.finish();
        assert_eq!(report.verdict, Verdict::Incomplete, "findings: {:?}", report.findings);
    }

    /// Finding 2 canary: the pairing reports must not run on a partial scan. A field left open when
    /// the event budget stops the scan is not an unbalanced field; the same document under normal
    /// limits still reports `FLD-UNBALANCED`.
    #[test]
    fn budget_stopped_scan_does_not_report_a_truncated_field() {
        let body = format!(r#"<w:p><w:r><w:fldChar w:fldCharType="begin"/></w:r>{}</w:p>"#, "<w:r><w:t>x</w:t></w:r>".repeat(80));
        let doc = format!(r#"<?xml version="1.0"?><w:document xmlns:w="{}"><w:body>{body}</w:body></w:document>"#, NS_W).into_bytes();
        let zip = tiny_zip(&[
            ("[Content_Types].xml", content_types()),
            ("_rels/.rels", package_rels()),
            ("word/document.xml", doc),
            ("word/_rels/document.xml.rels", doc_rels()),
        ]);
        let stopped = validate_bytes_limited(&zip, Limits { events: 25, ..Limits::default() });
        assert_eq!(stopped.verdict, Verdict::Incomplete, "findings: {:?}", stopped.findings);
        assert!(
            !stopped.findings.iter().any(|f| f.rule == "FLD-UNBALANCED"),
            "a budget-stopped scan must not report the truncated field: {:?}",
            stopped.findings
        );
        // Regression control: fully scanned, the unclosed field is reported.
        let full = validate_bytes(&zip);
        assert!(full.findings.iter().any(|f| f.rule == "FLD-UNBALANCED"), "findings: {:?}", full.findings);
    }

    /// NOTE-PR-DANGLING must not run on a truncated definition set: with
    /// a fact budget of 1, the footnotes part's first definition (id 2)
    /// fills the budget and the separator's definition (id 0) is dropped
    /// from the pooled facts — the settings listing for id 0 is then NOT
    /// dangling, and the run must not claim it is.
    #[test]
    fn note_pr_dangling_suppressed_when_definitions_capped() {
        let ct = format!(
            r#"<?xml version="1.0"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="{}"/><Override PartName="/word/footnotes.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.footnotes+xml"/><Override PartName="/word/settings.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.settings+xml"/></Types>"#,
            package::MAIN_CONTENT_TYPE
        );
        let doc = format!(r#"<?xml version="1.0"?><w:document xmlns:w="{NS_W}"><w:body><w:p/></w:body></w:document>"#).into_bytes();
        let footnotes = format!(
            r#"<?xml version="1.0"?><w:footnotes xmlns:w="{NS_W}"><w:footnote w:id="2"><w:p/></w:footnote><w:footnote w:type="separator" w:id="0"><w:p/></w:footnote></w:footnotes>"#
        )
        .into_bytes();
        let settings =
            format!(r#"<?xml version="1.0"?><w:settings xmlns:w="{NS_W}"><w:footnotePr><w:footnote w:id="0"/></w:footnotePr></w:settings>"#)
                .into_bytes();
        let zip = tiny_zip(&[
            ("[Content_Types].xml", ct.into_bytes()),
            ("_rels/.rels", package_rels()),
            ("word/document.xml", doc),
            ("word/footnotes.xml", footnotes),
            ("word/settings.xml", settings),
        ]);
        let report = validate_bytes_limited(&zip, Limits { facts: 1, ..Limits::default() });
        assert_eq!(report.verdict, Verdict::Incomplete, "findings: {:?}", report.findings);
        assert!(
            !report.findings.iter().any(|f| f.rule == "NOTE-PR-DANGLING"),
            "a capped definition set must not produce dangling claims: {:?}",
            report.findings
        );
        // Control: at full limits the same package is clean for both rules.
        let full = validate_bytes(&zip);
        assert!(!full.findings.iter().any(|f| f.rule == "NOTE-PR-DANGLING" || f.rule == "NOTE-PR-LISTING"), "findings: {:?}", full.findings);
    }

    /// The note collections are pooled-budgeted across parts: three
    /// settings parts listing two ids each overflow a fact budget of 3
    /// in aggregate even though no single part caps — the join must
    /// report incompleteness and make no dangling claims.
    #[test]
    fn note_pr_listings_pooled_cap_suppresses_the_join() {
        let settings_ct = "application/vnd.openxmlformats-officedocument.wordprocessingml.settings+xml";
        let ct = format!(
            r#"<?xml version="1.0"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="{}"/><Override PartName="/word/settings.xml" ContentType="{settings_ct}"/><Override PartName="/word/settings2.xml" ContentType="{settings_ct}"/><Override PartName="/word/settings3.xml" ContentType="{settings_ct}"/></Types>"#,
            package::MAIN_CONTENT_TYPE
        );
        let doc = format!(r#"<?xml version="1.0"?><w:document xmlns:w="{NS_W}"><w:body><w:p/></w:body></w:document>"#).into_bytes();
        let listing = |a: u8, b: u8| {
            format!(r#"<?xml version="1.0"?><w:settings xmlns:w="{NS_W}"><w:footnotePr><w:footnote w:id="{a}"/><w:footnote w:id="{b}"/></w:footnotePr></w:settings>"#).into_bytes()
        };
        let zip = tiny_zip(&[
            ("[Content_Types].xml", ct.into_bytes()),
            ("_rels/.rels", package_rels()),
            ("word/document.xml", doc),
            ("word/settings.xml", listing(1, 2)),
            ("word/settings2.xml", listing(3, 4)),
            ("word/settings3.xml", listing(5, 6)),
        ]);
        let report = validate_bytes_limited(&zip, Limits { facts: 3, ..Limits::default() });
        assert!(
            !report.findings.iter().any(|f| f.rule == "NOTE-PR-DANGLING" || f.rule == "NOTE-PR-LISTING"),
            "a pooled-capped join must not judge listings: {:?}",
            report.findings
        );
        assert!(report.coverage_notes.iter().any(|n| n.contains("footnotePr")), "notes: {:?}", report.coverage_notes);
    }

    /// Build a package whose main part carries `n` contentPart
    /// references, all resolving to an application/xml target (so each
    /// collected id is a CONTENTPART-TYPE finding).
    fn content_part_zip(n: usize) -> Vec<u8> {
        let ct = format!(
            r#"<?xml version="1.0"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="{}"/><Override PartName="/word/data.xml" ContentType="application/xml"/></Types>"#,
            package::MAIN_CONTENT_TYPE
        );
        let elems: String = (0..n).map(|i| format!(r#"<w:p><w:r><w:contentPart r:id="cp{i}"/></w:r></w:p>"#)).collect();
        let doc = format!(r#"<?xml version="1.0"?><w:document xmlns:w="{NS_W}" xmlns:r="{NS_R}"><w:body>{elems}</w:body></w:document>"#).into_bytes();
        let rels: String = (0..n)
            .map(|i| format!(r#"<Relationship Id="cp{i}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/customXml" Target="data.xml"/>"#))
            .collect();
        let doc_rels = format!(
            r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">{rels}</Relationships>"#
        )
        .into_bytes();
        tiny_zip(&[
            ("[Content_Types].xml", ct.into_bytes()),
            ("_rels/.rels", package_rels()),
            ("word/document.xml", doc),
            ("word/_rels/document.xml.rels", doc_rels),
            ("word/data.xml", b"<root/>".to_vec()),
        ])
    }

    /// The contentPart collection honours the fact budget (C3 review):
    /// at a budget of 4 with 6 references, exactly the 4 collected ids
    /// are judged, the run is marked incomplete, and the shortfall is
    /// disclosed — collection does not run past the budget.
    #[test]
    fn content_part_collection_honours_the_fact_budget() {
        let report = validate_bytes_limited(&content_part_zip(6), Limits { facts: 4, ..Limits::default() });
        let fired = report.findings.iter().filter(|f| f.rule == "CONTENTPART-TYPE").count();
        assert_eq!(fired, 4, "findings: {:?}", report.findings);
        assert!(report.coverage_notes.iter().any(|n| n.contains("contentPart")), "notes: {:?}", report.coverage_notes);
        // Established errors keep FAIL precedence over the incomplete
        // marking (the note-join rules differ: there an omission can
        // invalidate a "missing" claim; a collected contentPart
        // resolving to a wrong type is a defect on its own).
        assert_eq!(report.verdict, Verdict::Fail, "findings: {:?}", report.findings);
        // Control: at full limits all six are judged, with no note.
        let full = validate_bytes(&content_part_zip(6));
        assert_eq!(full.findings.iter().filter(|f| f.rule == "CONTENTPART-TYPE").count(), 6);
        assert!(!full.coverage_notes.iter().any(|n| n.contains("contentPart")));
    }

    /// The mirror image of the budget test above: when every collected
    /// reference is valid (a text/xml target) and the budget still
    /// truncates collection, the verdict is INCOMPLETE — not PASS,
    /// since not every reference was checked, and not FAIL, since no
    /// defect was established.
    #[test]
    fn content_part_capped_with_valid_references_is_incomplete() {
        let ct = format!(
            r#"<?xml version="1.0"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="{}"/><Override PartName="/word/data.xml" ContentType="text/xml"/></Types>"#,
            package::MAIN_CONTENT_TYPE
        );
        let elems: String = (0..6).map(|i| format!(r#"<w:p><w:r><w:contentPart r:id="cp{i}"/></w:r></w:p>"#)).collect();
        let doc = format!(r#"<?xml version="1.0"?><w:document xmlns:w="{NS_W}" xmlns:r="{NS_R}"><w:body>{elems}</w:body></w:document>"#).into_bytes();
        // A non-catalogue relationship type keeps CT-MISMATCH out of
        // the scenario (it prescribes application/xml for customXml
        // relationships); CONTENTPART-TYPE resolves by id regardless.
        let rels: String =
            (0..6).map(|i| format!(r#"<Relationship Id="cp{i}" Type="https://example.com/relationships/contentPart" Target="data.xml"/>"#)).collect();
        let doc_rels = format!(
            r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">{rels}</Relationships>"#
        )
        .into_bytes();
        let zip = tiny_zip(&[
            ("[Content_Types].xml", ct.into_bytes()),
            ("_rels/.rels", package_rels()),
            ("word/document.xml", doc),
            ("word/_rels/document.xml.rels", doc_rels),
            ("word/data.xml", b"<root/>".to_vec()),
        ]);
        let report = validate_bytes_limited(&zip, Limits { facts: 4, ..Limits::default() });
        assert_eq!(report.findings.iter().filter(|f| f.rule == "CONTENTPART-TYPE").count(), 0, "findings: {:?}", report.findings);
        assert_eq!(report.verdict, Verdict::Incomplete, "findings: {:?}", report.findings);
        assert!(report.coverage_notes.iter().any(|n| n.contains("contentPart")), "notes: {:?}", report.coverage_notes);
    }

    /// The pooled budget binds across owners: two parts with three
    /// references each stay under a per-part budget of 4 individually
    /// but overflow it pooled, so the pooled collection truncates and
    /// the shortfall is disclosed the same way.
    #[test]
    fn content_part_pooled_budget_binds_across_owners() {
        let ct = format!(
            r#"<?xml version="1.0"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="{}"/><Override PartName="/word/data.xml" ContentType="application/xml"/></Types>"#,
            package::MAIN_CONTENT_TYPE
        );
        let elems = |prefix: &str| -> String { (0..3).map(|i| format!(r#"<w:p><w:r><w:contentPart r:id="{prefix}{i}"/></w:r></w:p>"#)).collect() };
        let rels = |prefix: &str| -> Vec<u8> {
            let rs: String = (0..3)
                .map(|i| format!(r#"<Relationship Id="{prefix}{i}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/customXml" Target="data.xml"/>"#))
                .collect();
            format!(
                r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">{rs}</Relationships>"#
            )
            .into_bytes()
        };
        let doc = format!(r#"<?xml version="1.0"?><w:document xmlns:w="{NS_W}" xmlns:r="{NS_R}"><w:body>{}</w:body></w:document>"#, elems("a"))
            .into_bytes();
        let other = format!(r#"<?xml version="1.0"?><w:other xmlns:w="{NS_W}" xmlns:r="{NS_R}">{}</w:other>"#, elems("b")).into_bytes();
        let zip = tiny_zip(&[
            ("[Content_Types].xml", ct.into_bytes()),
            ("_rels/.rels", package_rels()),
            ("word/document.xml", doc),
            ("word/_rels/document.xml.rels", rels("a")),
            ("word/other.xml", other),
            ("word/_rels/other.xml.rels", rels("b")),
            ("word/data.xml", b"<root/>".to_vec()),
        ]);
        let report = validate_bytes_limited(&zip, Limits { facts: 4, ..Limits::default() });
        let fired = report.findings.iter().filter(|f| f.rule == "CONTENTPART-TYPE").count();
        assert_eq!(fired, 4, "findings: {:?}", report.findings);
        assert!(report.coverage_notes.iter().any(|n| n.contains("contentPart")), "notes: {:?}", report.coverage_notes);
    }

    /// Many distinct ids are all judged (the dedupe/index path is not
    /// quadratic-gated by a finding cap) — 2,000 references resolve and
    /// report individually.
    #[test]
    fn content_part_many_distinct_ids_all_judged() {
        let report = validate_bytes(&content_part_zip(2_000));
        assert_eq!(report.findings.iter().filter(|f| f.rule == "CONTENTPART-TYPE").count(), 2_000);
    }

    /// §3 canary (a): a budget stop must not hide a defect the scanned prefix already establishes.
    /// An extra `fldChar end` at depth 0 is unbalanced whatever follows, so the run FAILs (not merely
    /// INCOMPLETE) even though the event budget stopped the scan.
    #[test]
    fn budget_stopped_scan_still_reports_a_prefix_established_field_imbalance() {
        let body = format!(r#"<w:p><w:r><w:fldChar w:fldCharType="end"/></w:r></w:p>{}"#, "<w:r><w:t>x</w:t></w:r>".repeat(80));
        let doc = format!(r#"<?xml version="1.0"?><w:document xmlns:w="{}"><w:body>{body}</w:body></w:document>"#, NS_W).into_bytes();
        let zip = tiny_zip(&[
            ("[Content_Types].xml", content_types()),
            ("_rels/.rels", package_rels()),
            ("word/document.xml", doc),
            ("word/_rels/document.xml.rels", doc_rels()),
        ]);
        let stopped = validate_bytes_limited(&zip, Limits { events: 25, ..Limits::default() });
        // Independently assert the budget really stopped the scan: the part's event-budget note is
        // emitted only when the scan breaks early on the ceiling, so it cannot be produced by a scan
        // that read the whole part. Without this the test would not discriminate a real stop from a
        // complete scan that merely skipped the prefix report.
        assert!(
            stopped.coverage_notes.iter().any(|n| n.contains("word/document.xml") && n.contains("XML events")),
            "the event budget must be shown to have stopped the scan: {:?}",
            stopped.coverage_notes
        );
        assert!(
            stopped.findings.iter().any(|f| f.rule == "FLD-UNBALANCED" && f.part.as_deref() == Some("word/document.xml")),
            "a prefix-established imbalance must survive the budget stop: {:?}",
            stopped.findings
        );
        assert_eq!(stopped.verdict, Verdict::Fail, "an established Error outranks the stop's INCOMPLETE: {:?}", stopped.findings);
    }

    /// §3 canary (b): a malformed part whose scanned prefix leaves a `fldChar begin` unclosed marks
    /// the run INCOMPLETE with a note naming the part, and reports no FLD-UNBALANCED Error, because
    /// the completion-dependent check that would decide it could not run.
    #[test]
    fn malformed_part_with_unmatched_begin_is_incomplete_without_a_field_error() {
        // An unreferenced part: malformed is Warning-only by calibration, so INCOMPLETE is decisive.
        let doc = format!(r#"<?xml version="1.0"?><w:document xmlns:w="{}"><w:body><w:p><w:r><w:fldChar w:fldCharType="begin"/></w:r></w:p>"#, NS_W)
            .into_bytes();
        let zip = tiny_zip(&[
            ("[Content_Types].xml", content_types()),
            ("_rels/.rels", package_rels()),
            ("word/document.xml", document()),
            ("word/_rels/document.xml.rels", doc_rels()),
            ("word/unused.xml", doc),
        ]);
        let report = validate_bytes(&zip);
        assert_eq!(report.verdict, Verdict::Incomplete, "findings: {:?}", report.findings);
        assert!(
            !report.findings.iter().any(|f| f.rule == "FLD-UNBALANCED"),
            "the completion-dependent field check must not run on the malformed prefix: {:?}",
            report.findings
        );
        assert!(
            report.coverage_notes.iter().any(|n| n.contains("word/unused.xml") && n.contains("pairing")),
            "the INCOMPLETE must be explained by a note naming the part and the check: {:?}",
            report.coverage_notes
        );
    }

    /// §3 canary (c): a malformed part whose prefix holds a fully matched pair changes nothing — no
    /// completion-dependent report would have fired, so no INCOMPLETE is added. A matched **bookmark**
    /// pair is used deliberately: it leaves both sets non-empty but equal, so a trigger written as
    /// "sets are non-empty" would wrongly fire, while the correct "sets differ" trigger does not.
    #[test]
    fn malformed_part_with_a_matched_pair_is_unchanged() {
        let doc = format!(
            r#"<?xml version="1.0"?><w:document xmlns:w="{}"><w:body><w:p><w:bookmarkStart w:id="1" w:name="a"/><w:bookmarkEnd w:id="1"/></w:p>"#,
            NS_W
        )
        .into_bytes();
        let zip = tiny_zip(&[
            ("[Content_Types].xml", content_types_with_override("word/unused.xml", package::COMMENTS_CONTENT_TYPE)),
            ("_rels/.rels", package_rels()),
            ("word/document.xml", document()),
            ("word/_rels/document.xml.rels", doc_rels()),
            ("word/unused.xml", doc),
        ]);
        let report = validate_bytes(&zip);
        assert_eq!(report.verdict, Verdict::Pass, "findings: {:?}", report.findings);
        assert!(
            report.findings.iter().any(|f| f.rule == "XML-MALFORMED" && f.severity == Severity::Warning),
            "the malformed unreferenced part is still a Warning: {:?}",
            report.findings
        );
        assert!(!report.coverage_notes.iter().any(|n| n.contains("pairing")), "a matched pair leaves nothing pending: {:?}", report.coverage_notes);
    }

    /// Finding 2 canary: a bookmark pair split by the fact budget must not be reported as unpaired
    /// on the partial scan; the same document under normal limits still reports `BOOKMARK-UNPAIRED`.
    #[test]
    fn fact_capped_scan_does_not_report_a_split_bookmark() {
        let body = concat!(
            r#"<w:p><w:bookmarkEnd w:id="5"/></w:p>"#,
            r#"<w:p><w:bookmarkStart w:id="1" w:name="a"/></w:p>"#,
            r#"<w:p><w:bookmarkStart w:id="2" w:name="b"/></w:p>"#,
            r#"<w:p><w:bookmarkStart w:id="3" w:name="c"/></w:p>"#,
            r#"<w:p><w:bookmarkStart w:id="4" w:name="d"/></w:p>"#,
            r#"<w:p><w:bookmarkEnd w:id="1"/></w:p>"#,
            r#"<w:p><w:bookmarkEnd w:id="2"/></w:p>"#,
        );
        let doc = format!(r#"<?xml version="1.0"?><w:document xmlns:w="{}"><w:body>{body}</w:body></w:document>"#, NS_W).into_bytes();
        let zip = tiny_zip(&[
            ("[Content_Types].xml", content_types()),
            ("_rels/.rels", package_rels()),
            ("word/document.xml", doc),
            ("word/_rels/document.xml.rels", doc_rels()),
        ]);
        let capped = validate_bytes_limited(&zip, Limits { facts: 3, ..Limits::default() });
        assert_eq!(capped.verdict, Verdict::Incomplete, "findings: {:?}", capped.findings);
        assert!(
            !capped.findings.iter().any(|f| f.rule == "BOOKMARK-UNPAIRED"),
            "a fact-capped scan must not report a split bookmark: {:?}",
            capped.findings
        );
        // Regression control: fully scanned, the unpaired bookmark is reported.
        let full = validate_bytes(&zip);
        assert!(full.findings.iter().any(|f| f.rule == "BOOKMARK-UNPAIRED"), "findings: {:?}", full.findings);
    }

    #[test]
    /// The comment-range mirror of the split-bookmark canary: a range pair
    /// split by the fact budget must not be reported as unpaired on the
    /// partial scan; the same document under normal limits still reports
    /// `COMMENT-RANGE-UNPAIRED`.
    fn fact_capped_scan_does_not_report_a_split_comment_range() {
        let body = concat!(
            r#"<w:p><w:commentRangeEnd w:id="5"/></w:p>"#,
            r#"<w:p><w:commentRangeStart w:id="1"/></w:p>"#,
            r#"<w:p><w:commentRangeStart w:id="2"/></w:p>"#,
            r#"<w:p><w:commentRangeStart w:id="3"/></w:p>"#,
            r#"<w:p><w:commentRangeStart w:id="4"/></w:p>"#,
            r#"<w:p><w:commentRangeEnd w:id="1"/></w:p>"#,
            r#"<w:p><w:commentRangeEnd w:id="2"/></w:p>"#,
        );
        let doc = format!(r#"<?xml version="1.0"?><w:document xmlns:w="{}"><w:body>{body}</w:body></w:document>"#, NS_W).into_bytes();
        let zip = tiny_zip(&[
            ("[Content_Types].xml", content_types()),
            ("_rels/.rels", package_rels()),
            ("word/document.xml", doc),
            ("word/_rels/document.xml.rels", doc_rels()),
        ]);
        let capped = validate_bytes_limited(&zip, Limits { facts: 3, ..Limits::default() });
        assert_eq!(capped.verdict, Verdict::Incomplete, "findings: {:?}", capped.findings);
        assert!(
            !capped.findings.iter().any(|f| f.rule == "COMMENT-RANGE-UNPAIRED"),
            "a fact-capped scan must not report a split comment range: {:?}",
            capped.findings
        );
        // Regression control: fully scanned, the unpaired ranges are reported.
        let full = validate_bytes(&zip);
        assert!(full.findings.iter().any(|f| f.rule == "COMMENT-RANGE-UNPAIRED"), "findings: {:?}", full.findings);
    }

    #[test]
    /// The paraId collection rides the fact budget: more distinct values
    /// than the cap makes the verdict INCOMPLETE, never a silent pass —
    /// a duplicate hiding beyond the cap must not be reported as absent.
    fn paraid_collection_beyond_the_fact_cap_is_incomplete() {
        let mut body = String::new();
        for i in 1..=6u32 {
            body.push_str(&format!(r#"<w:p xmlns:w14="{NS_W14}" w14:paraId="{i:08X}" w14:textId="11111111"/>"#));
        }
        let doc = format!(r#"<?xml version="1.0"?><w:document xmlns:w="{NS_W}"><w:body>{body}</w:body></w:document>"#).into_bytes();
        let zip = tiny_zip(&[
            ("[Content_Types].xml", content_types()),
            ("_rels/.rels", package_rels()),
            ("word/document.xml", doc),
            ("word/_rels/document.xml.rels", doc_rels()),
        ]);
        let capped = validate_bytes_limited(&zip, Limits { facts: 3, ..Limits::default() });
        assert_eq!(capped.verdict, Verdict::Incomplete, "findings: {:?}", capped.findings);
        // Control: the same document under normal limits is a clean PASS.
        let full = validate_bytes(&zip);
        assert_eq!(full.verdict, Verdict::Pass, "findings: {:?}", full.findings);
    }

    #[test]
    /// At the cap boundary the collection stays honest both ways: a
    /// duplicate among the values that WERE collected is still
    /// reported, and a duplicate among dropped values is not invented
    /// (the verdict is INCOMPLETE instead).
    fn paraid_cap_boundary_reports_kept_dups_and_invents_none() {
        let doc_with = |ids: &[u32]| {
            let mut body = String::new();
            for i in ids {
                body.push_str(&format!(r#"<w:p xmlns:w14="{NS_W14}" w14:paraId="{i:08X}" w14:textId="11111111"/>"#));
            }
            let doc = format!(r#"<?xml version="1.0"?><w:document xmlns:w="{NS_W}"><w:body>{body}</w:body></w:document>"#).into_bytes();
            tiny_zip(&[
                ("[Content_Types].xml", content_types()),
                ("_rels/.rels", package_rels()),
                ("word/document.xml", doc),
                ("word/_rels/document.xml.rels", doc_rels()),
            ])
        };
        // 1,2,3,1 at cap 3: the repeat value was collected — reported.
        let kept = validate_bytes_limited(&doc_with(&[1, 2, 3, 1]), Limits { facts: 3, ..Limits::default() });
        assert!(kept.findings.iter().any(|f| f.rule == "PARAID-DUP"), "findings: {:?}", kept.findings);
        // 1,2,3,4,4 at cap 3: value 4 was dropped — no invented dup.
        let dropped = validate_bytes_limited(&doc_with(&[1, 2, 3, 4, 4]), Limits { facts: 3, ..Limits::default() });
        assert!(!dropped.findings.iter().any(|f| f.rule == "PARAID-DUP"), "findings: {:?}", dropped.findings);
        assert_eq!(dropped.verdict, Verdict::Incomplete, "findings: {:?}", dropped.findings);
    }

    #[test]
    /// Mutation: cutting the pooled fact cap so a dangling comment reference is dropped flips the clean verdict to INCOMPLETE (undropped, the same package FAILs).
    fn dropped_dangling_comment_reference_is_incomplete() {
        let mut refs = String::new();
        for _ in 0..3 {
            refs.push_str(r#"<w:commentReference w:id="1"/>"#);
        }
        refs.push_str(r#"<w:commentReference w:id="9"/>"#);
        let doc =
            format!(r#"<?xml version="1.0"?><w:document xmlns:w="{}"><w:body><w:p><w:r>{refs}</w:r></w:p></w:body></w:document>"#, NS_W).into_bytes();
        let comments =
            format!(r#"<?xml version="1.0"?><w:comments xmlns:w="{}"><w:comment w:id="1"><w:p/></w:comment></w:comments>"#, NS_W).into_bytes();
        let zip = tiny_zip(&[
            ("[Content_Types].xml", content_types_with_override("word/comments.xml", package::COMMENTS_CONTENT_TYPE)),
            ("_rels/.rels", package_rels()),
            ("word/document.xml", doc),
            ("word/_rels/document.xml.rels", doc_rels()),
            ("word/comments.xml", comments),
        ]);
        let report = validate_bytes_limited(&zip, Limits { facts: 3, ..Limits::default() });
        assert_eq!(report.verdict, Verdict::Incomplete, "findings: {:?}", report.findings);
        assert!(
            report.coverage_notes.iter().any(|n| n.contains("fact/reference budget") && n.contains("word/document.xml")),
            "{:?}",
            report.coverage_notes
        );
    }

    #[test]
    /// Mutation: the same dropped-fact flip for a dangling style reference.
    fn dropped_dangling_style_reference_is_incomplete() {
        let mut body = String::new();
        for _ in 0..3 {
            body.push_str(r#"<w:p><w:pPr><w:pStyle w:val="Normal"/></w:pPr></w:p>"#);
        }
        body.push_str(r#"<w:p><w:pPr><w:pStyle w:val="Missing"/></w:pPr></w:p>"#);
        let doc = format!(r#"<?xml version="1.0"?><w:document xmlns:w="{}"><w:body>{body}</w:body></w:document>"#, NS_W).into_bytes();
        let styles = format!(
            r#"<?xml version="1.0"?><w:styles xmlns:w="{}"><w:style w:type="paragraph" w:styleId="Normal"><w:name w:val="Normal"/></w:style></w:styles>"#,
            NS_W
        )
        .into_bytes();
        let zip = tiny_zip(&[
            ("[Content_Types].xml", content_types_with_override("word/styles.xml", package::STYLES_CONTENT_TYPE)),
            ("_rels/.rels", package_rels()),
            ("word/document.xml", doc),
            ("word/_rels/document.xml.rels", doc_rels()),
            ("word/styles.xml", styles),
        ]);
        let report = validate_bytes_limited(&zip, Limits { facts: 3, ..Limits::default() });
        assert_eq!(report.verdict, Verdict::Incomplete, "findings: {:?}", report.findings);
        assert!(
            report.coverage_notes.iter().any(|n| n.contains("fact/reference budget") && n.contains("word/document.xml")),
            "the dropped-reference canary must assert its note and location: {:?}",
            report.coverage_notes
        );
    }

    #[test]
    /// A supporting part whose own parser exhausts the event budget marks the run INCOMPLETE.
    fn auxiliary_parser_event_budget_exhaustion_is_incomplete() {
        // A numbering part named without the `.xml` suffix is not read by the main scan, so only
        // the auxiliary numbering parser can hit the event budget (H1).
        let mut numbering = format!(r#"<w:numbering xmlns:w="{}">"#, NS_W);
        for i in 0..300 {
            numbering.push_str(&format!(r#"<w:num w:numId="{i}"><w:abstractNumId w:val="0"/></w:num>"#));
        }
        numbering.push_str("</w:numbering>");
        let zip = tiny_zip(&[
            ("[Content_Types].xml", content_types_with_override("word/numbering.dat", package::NUMBERING_CONTENT_TYPE)),
            ("_rels/.rels", package_rels()),
            ("word/document.xml", document()),
            (
                "word/_rels/document.xml.rels",
                rels(
                    r#"<Relationship Id="rN" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/numbering" Target="numbering.dat"/>"#,
                ),
            ),
            ("word/numbering.dat", numbering.into_bytes()),
        ]);
        let report = validate_bytes_limited(&zip, Limits { events: 60, ..Limits::default() });
        assert_eq!(report.verdict, Verdict::Incomplete, "findings: {:?}", report.findings);
        assert!(
            report.coverage_notes.iter().any(|n| n.contains("word/numbering.dat") && (n.contains("event") || n.contains("depth"))),
            "the auxiliary parser's exhaustion must identify the event/depth mechanism and its part: {:?}",
            report.coverage_notes
        );
    }

    #[test]
    /// Move range-start ids past the fact budget exhaust the pool and mark the run INCOMPLETE.
    fn repeated_move_start_ids_exhaust_the_move_budget() {
        let mut body = String::new();
        for _ in 0..8 {
            body.push_str(r#"<w:p><w:moveFromRangeStart w:id="1" w:name="A"/></w:p>"#);
        }
        let doc = format!(r#"<?xml version="1.0"?><w:document xmlns:w="{}"><w:body>{body}</w:body></w:document>"#, NS_W).into_bytes();
        let zip = tiny_zip(&[
            ("[Content_Types].xml", content_types()),
            ("_rels/.rels", package_rels()),
            ("word/document.xml", doc),
            ("word/_rels/document.xml.rels", doc_rels()),
        ]);
        let report = validate_bytes_limited(&zip, Limits { facts: 3, ..Limits::default() });
        assert_eq!(report.verdict, Verdict::Incomplete, "findings: {:?}", report.findings);
        assert!(
            report.coverage_notes.iter().any(|n| n.contains("fact/reference budget") && n.contains("word/document.xml")),
            "the move-budget exhaustion must name the mechanism and the affected part: {:?}",
            report.coverage_notes
        );
    }

    #[test]
    /// A supporting part nested past the depth limit fails its parse outright, so dependent absence-judging checks skip rather than report.
    fn auxiliary_parser_depth_exhaustion_is_terminal() {
        // J1: a low-depth auxiliary parser must stop at the crossing. A duplicate Override placed
        // AFTER the crossing would fire CT-DUP-OVERRIDE if the content-type parser kept reading;
        // early termination means it is never seen — INCOMPLETE, a depth-budget note naming the
        // part, and no CT-DUP-OVERRIDE.
        let deep = format!(
            r#"<?xml version="1.0"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="{}"/>{}{}<Override PartName="/word/document.xml" ContentType="{}"/></Types>"#,
            package::MAIN_CONTENT_TYPE,
            "<x>".repeat(8),
            "</x>".repeat(8),
            package::MAIN_CONTENT_TYPE,
        );
        let zip = tiny_zip(&[
            ("[Content_Types].xml", deep.into_bytes()),
            ("_rels/.rels", package_rels()),
            ("word/document.xml", document()),
            ("word/_rels/document.xml.rels", doc_rels()),
        ]);
        let report = validate_bytes_limited(&zip, Limits { depth: 3, ..Limits::default() });
        assert_eq!(report.verdict, Verdict::Incomplete, "findings: {:?}", report.findings);
        assert!(
            report.coverage_notes.iter().any(|n| n.contains("[Content_Types].xml") && (n.contains("depth") || n.contains("event"))),
            "the depth exhaustion note must name the mechanism and the part: {:?}",
            report.coverage_notes
        );
        assert!(
            !report.findings.iter().any(|f| f.rule == "CT-DUP-OVERRIDE"),
            "early termination must drop the declaration after the crossing: {:?}",
            report.findings
        );
    }

    #[test]
    /// A .rels part nested past the depth limit fails its parse outright, so reference checks against it skip.
    fn deep_rels_depth_exhaustion_is_terminal() {
        // J1: a deeply nested .rels part must stop at the depth crossing, so a dangling
        // relationship placed AFTER it is never recorded — INCOMPLETE, a depth-budget note naming
        // the rels part, and no REL-DANGLING for the later relationship.
        let deep = format!(
            r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">{}{}<Relationship Id="rB" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="media/missing.png"/></Relationships>"#,
            "<x>".repeat(8),
            "</x>".repeat(8),
        );
        let zip = tiny_zip(&[
            ("[Content_Types].xml", content_types()),
            ("_rels/.rels", package_rels()),
            ("word/document.xml", document()),
            ("word/_rels/document.xml.rels", doc_rels()),
            ("word/_rels/extra.xml.rels", deep.into_bytes()),
        ]);
        let report = validate_bytes_limited(&zip, Limits { depth: 6, ..Limits::default() });
        assert_eq!(report.verdict, Verdict::Incomplete, "findings: {:?}", report.findings);
        assert!(
            report.coverage_notes.iter().any(|n| n.contains("word/_rels/extra.xml.rels") && (n.contains("depth") || n.contains("event"))),
            "the deep-rels exhaustion note must name the mechanism and the part: {:?}",
            report.coverage_notes
        );
        assert!(
            !report.findings.iter().any(|f| f.rule == "REL-DANGLING" && f.part.as_deref() == Some("word/_rels/extra.xml.rels")),
            "early termination must drop the relationship after the crossing: {:?}",
            report.findings
        );
    }

    #[test]
    /// When the footnote fact budget cuts off collection, a coverage note records that later notes went unrecorded.
    fn note_budget_exhaustion_leaves_a_marker() {
        let comments = format!(
            r#"<?xml version="1.0"?><w:comments xmlns:w="{}"><w:comment w:id="1"><w:p/></w:comment><w:comment w:id="2"><w:p/></w:comment><w:comment w:id="3"><w:p/></w:comment></w:comments>"#,
            NS_W
        )
        .into_bytes();
        let zip = tiny_zip(&[
            ("[Content_Types].xml", content_types_with_override("word/comments.xml", package::COMMENTS_CONTENT_TYPE)),
            ("_rels/.rels", package_rels()),
            ("word/document.xml", document()),
            ("word/_rels/document.xml.rels", doc_rels()),
            ("word/comments.xml", comments),
        ]);
        let report = validate_bytes_limited(&zip, Limits { notes: 1, ..Limits::default() });
        assert_eq!(report.verdict, Verdict::Incomplete, "findings: {:?}", report.findings);
        assert!(
            report.coverage_notes.iter().any(|n| n.contains("coverage-note budget exhausted")),
            "exhausting the note budget must leave a marker: {:?}",
            report.coverage_notes
        );
    }

    #[test]
    /// Pooled story-fact exhaustion produces one coverage note naming the first part whose facts were cut.
    fn pooled_story_fact_exhaustion_names_the_first_affected_part() {
        // K2: the pooled story-fact budget is aggregate across parts. Two comments parts each hold
        // fewer than the cap, so neither hits its own per-part budget; only the pooled budget is
        // exceeded, and the note must name the mechanism and the first part that crossed it.
        let comments_a = format!(
            r#"<?xml version="1.0"?><w:comments xmlns:w="{}"><w:comment w:id="1"><w:p/></w:comment><w:comment w:id="2"><w:p/></w:comment></w:comments>"#,
            NS_W
        )
        .into_bytes();
        let comments_b = format!(
            r#"<?xml version="1.0"?><w:comments xmlns:w="{}"><w:comment w:id="3"><w:p/></w:comment><w:comment w:id="4"><w:p/></w:comment></w:comments>"#,
            NS_W
        )
        .into_bytes();
        let zip = tiny_zip(&[
            (
                "[Content_Types].xml",
                format!(
                    r#"<?xml version="1.0"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="{}"/><Override PartName="/word/commentsA.xml" ContentType="{}"/><Override PartName="/word/commentsB.xml" ContentType="{}"/></Types>"#,
                    package::MAIN_CONTENT_TYPE,
                    package::COMMENTS_CONTENT_TYPE,
                    package::COMMENTS_CONTENT_TYPE,
                )
                .into_bytes(),
            ),
            ("_rels/.rels", package_rels()),
            ("word/document.xml", document()),
            ("word/_rels/document.xml.rels", doc_rels()),
            ("word/commentsA.xml", comments_a),
            ("word/commentsB.xml", comments_b),
        ]);
        let report = validate_bytes_limited(&zip, Limits { facts: 3, ..Limits::default() });
        assert_eq!(report.verdict, Verdict::Incomplete, "findings: {:?}", report.findings);
        assert!(
            report.coverage_notes.iter().any(|n| n.contains("pooled story facts") && n.contains("word/commentsB.xml")),
            "the pooled-fact note must name the mechanism and the first affected part: {:?}",
            report.coverage_notes
        );
    }

    #[test]
    /// Pooled relationship-reference exhaustion produces one coverage note naming the first part whose references were cut.
    fn pooled_relationship_reference_exhaustion_names_the_first_affected_part() {
        // K2: the pooled relationship-reference budget is aggregate across parts. The main part's
        // two refs resolve; adding the header's two takes the pool past the cap, so the note must
        // name the mechanism and the header, the first part that crossed it.
        let doc = format!(
            r#"<?xml version="1.0"?><w:document xmlns:w="{}" xmlns:r="{}"><w:body><w:p><w:hyperlink r:id="rL1"/><w:hyperlink r:id="rL2"/></w:p></w:body></w:document>"#,
            NS_W, NS_R
        )
        .into_bytes();
        let header = format!(
            r#"<?xml version="1.0"?><w:hdr xmlns:w="{}" xmlns:r="{}"><w:p><w:hyperlink r:id="rL1"/><w:hyperlink r:id="rL2"/></w:p></w:hdr>"#,
            NS_W, NS_R
        )
        .into_bytes();
        let hyperlink_rels = rels(concat!(
            r#"<Relationship Id="rL1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink" Target="https://one.example/" TargetMode="External"/>"#,
            r#"<Relationship Id="rL2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink" Target="https://two.example/" TargetMode="External"/>"#,
        ));
        let zip = tiny_zip(&[
            (
                "[Content_Types].xml",
                format!(
                    r#"<?xml version="1.0"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="{}"/><Override PartName="/word/header1.xml" ContentType="{}"/></Types>"#,
                    package::MAIN_CONTENT_TYPE,
                    package::HEADER_CONTENT_TYPE,
                )
                .into_bytes(),
            ),
            ("_rels/.rels", package_rels()),
            ("word/document.xml", doc),
            ("word/_rels/document.xml.rels", hyperlink_rels.clone()),
            ("word/header1.xml", header),
            ("word/_rels/header1.xml.rels", hyperlink_rels),
        ]);
        let report = validate_bytes_limited(&zip, Limits { facts: 3, ..Limits::default() });
        assert_eq!(report.verdict, Verdict::Incomplete, "findings: {:?}", report.findings);
        assert!(
            report.coverage_notes.iter().any(|n| n.contains("relationship references") && n.contains("word/header1.xml")),
            "the pooled-reference note must name the mechanism and the first affected part: {:?}",
            report.coverage_notes
        );
    }

    /// Replace every occurrence of the same-length `from` with `to` in raw bytes (used to force a
    /// duplicate entry name the zip writer itself refuses to create).
    fn replace_same_len(data: &[u8], from: &[u8], to: &[u8]) -> Vec<u8> {
        assert_eq!(from.len(), to.len(), "replacement must preserve length");
        let mut out = Vec::with_capacity(data.len());
        let mut i = 0;
        while i < data.len() {
            if data[i..].starts_with(from) {
                out.extend_from_slice(to);
                i += from.len();
            } else {
                out.push(data[i]);
                i += 1;
            }
        }
        out
    }

    /// §2 canary: four unique parts plus a duplicate of one, with `entries` equal to the
    /// deduplicated count. The zip reader collapses the duplicate, so the loader's `zip.len()` check
    /// does not fire; the raw central directory holds five records, and the budgeted parse examines
    /// only four. The run must not return a clean PASS: duplicate checking learns a raw record went
    /// unexamined and marks the run INCOMPLETE with a note naming itself. Under a generous budget the
    /// same package still reports the duplicate exactly as before.
    #[test]
    fn duplicate_checking_knows_when_it_was_truncated() {
        let parts = [
            ("[Content_Types].xml", content_types()),
            ("_rels/.rels", package_rels()),
            ("word/document.xml", document()),
            ("word/_rels/document.xml.rels", doc_rels()),
        ];
        // A fifth raw record duplicates `_rels/.rels`: its name is the same 11 bytes, so the reader
        // deduplicates it away while the central directory still lists five records.
        let mut all: Vec<(&str, Vec<u8>)> = parts.to_vec();
        all.push(("_rels/.xels", package_rels()));
        let zip = replace_same_len(&tiny_zip(&all), b"_rels/.xels", b"_rels/.rels");

        let capped = validate_bytes_limited(&zip, Limits { entries: 4, ..Limits::default() });
        assert_ne!(capped.verdict, Verdict::Pass, "a truncated duplicate scan must not pass silently: {:?}", capped.findings);
        assert_eq!(capped.verdict, Verdict::Incomplete, "findings: {:?}", capped.findings);
        assert!(
            capped.coverage_notes.iter().any(|n| n.contains("duplicate entry checking")),
            "the coverage note must name duplicate checking as the incomplete check: {:?}",
            capped.coverage_notes
        );

        // Regression control: under a generous budget the duplicate is reported exactly as before.
        let full = validate_bytes_limited(&zip, Limits::default());
        assert!(
            full.findings.iter().any(|f| f.rule == "PKG-DUP-ENTRY"),
            "the duplicate must still be reported under a generous budget: {:?}",
            full.findings
        );
    }
}
