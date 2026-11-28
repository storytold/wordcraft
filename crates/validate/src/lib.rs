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

/// An executable-shape declaration audited and rendered by observations/generate.py.
/// No runtime code is emitted; collection footprints and fallbacks are checked by the gate.
macro_rules! observation_contract {
    (rules: $rules:literal; collections: $collections:literal; scope: $scope:literal;
     loss: $loss:literal; fallback: $fallback:literal; package_loss: $package_loss:literal;) => {};
}
#[cfg(test)]
mod contract_tests;
mod contracts_generated;

// Step 0 of the grammar work: the tables land before any check reads them. This allow is
// removed when the first grammar rule uses the module.
#[cfg(test)]
mod glossary_descendant_tests;
#[cfg(test)]
mod glossary_tests;
#[allow(dead_code)]
mod model;
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod model_tests;
mod observed;
mod ownership;
mod package;
use observed::{CompleteSet, Completeness, Observed};
use ownership::{OwnerContext, OwnerId, PartGraph, PartId, owner_scopes};
mod reserved;
mod resolution;
#[cfg(test)]
mod resolution_tests;
use resolution::Resolution;
mod media_boundary;
mod media_workspace;
mod scan;
mod tiff_admission;
mod xml;

use scan::{DecimalId, HexId, RelId, StyleId};

use std::collections::HashMap;
use std::collections::HashSet;
use std::collections::hash_map::Entry;
use std::io::Read;
use std::path::Path;

use craft_image_codecs::{self as codecs, CodecError, DecodeOptions, DecodeWarning, Format, Limits as CodecLimits};
use serde::Serialize;

#[cfg(test)]
use package::relationship_kind;
use package::{Package, RelsPart, canonical_entry, entry_by_name, rels_part_name};
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

/// Consumer-impact grade, calibrated per rule against the named consumers in corpus/EVIDENCE.md.
/// A specification violation alone does not determine the grade.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Severity {
    /// Structurally unusable as a document (for example, not a ZIP or missing its manifest).
    Corrupt,
    /// Consumer refusal, repair, or loss of referenced content, as calibrated for this rule.
    Error,
    /// A tolerated defect or degradation worth reporting, including some specification violations.
    Warning,
}

/// How the integrity violation was established; consumer impact is attributed separately.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Evidence {
    /// Structurally certain at the XML/package level; does not establish a consumer reaction.
    ByConstruction,
    /// Documented by OOXML or Microsoft's format/implementation specifications.
    MsDocumented,
    /// Consumer reaction observed on a defective fixture in a named Word oracle, with fixture,
    /// date, and consumer/open-path details recorded in corpus/EVIDENCE.md. One consumer's reaction
    /// does not establish another's.
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
    /// Informational notes about skipped checks and observations from completed checks,
    /// such as unattached custom XML schemas.
    pub coverage_notes: Vec<String>,
    /// Every rule id that executed, in the documented order.
    pub checks_run: Vec<&'static str>,
}

/// A rule's default severity and evidence. [`Acc::add`] looks the grade up here, next to its
/// provenance. Rules whose grade depends on the observation can use [`Acc::add_graded`].
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
/// `CT-MAIN-TYPE`: the main part's effective content type is not one of the WordprocessingML main
/// types (document, template, or a macro-enabled variant).
/// Observed: Word web refuses.
pub(crate) const CT_MAIN_TYPE: RuleSpec = RuleSpec { id: "CT-MAIN-TYPE", severity: Severity::Error, evidence: Evidence::Observed };
/// `CT-DUP-DEFAULT`: two Default entries for one extension (case-insensitive), OPC §7.2.3.2.1.
pub(crate) const CT_DUP_DEFAULT: RuleSpec = RuleSpec { id: "CT-DUP-DEFAULT", severity: Severity::Error, evidence: Evidence::ByConstruction };
/// `CT-DUP-OVERRIDE`: two Override entries for one part name (case-insensitive), OPC §7.2.3.2.1.
pub(crate) const CT_DUP_OVERRIDE: RuleSpec = RuleSpec { id: "CT-DUP-OVERRIDE", severity: Severity::Error, evidence: Evidence::ByConstruction };
/// `XML-DOCTYPE`: a declaration in an XML part's original prolog. Consumer outcomes are
/// recorded in corpus/EVIDENCE.md. See the observation contract for grading.
pub(crate) const XML_DOCTYPE: RuleSpec = RuleSpec { id: "XML-DOCTYPE", severity: Severity::Error, evidence: Evidence::Observed };
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
/// documents case-insensitivity). Desktop and web Word accepted the measured duplicate
/// groups, including case variants. Word for the web also saved an edit to the case-variant probe.
pub(crate) const FONT_DUP_NAME: RuleSpec = RuleSpec { id: "FONT-DUP-NAME", severity: Severity::Warning, evidence: Evidence::Observed };
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
/// unit is enforced there. Batch Mac-2 (Word for Mac, 2026-11-08): the
/// same fixture wedged Word — no window, sustained full CPU, killed at
/// the 3-minute observation limit. Verdicts combine by union across
/// consumers, so the rule reports at Error.
pub(crate) const MOVE_RANGE_COUNT: RuleSpec = RuleSpec { id: "MOVE-RANGE-COUNT", severity: Severity::Error, evidence: Evidence::Observed };
/// `NOTE-PR-LISTING`: a special footnote/endnote (separator,
/// continuationSeparator, continuationNotice) whose id no `footnotePr` /
/// `endnotePr` lists. ECMA-376 Part 1 §17.11.9: it "shall not be loaded",
/// and the document is non-conformant when the note "is required by the
/// document content" — a conformance claim, graded ByConstruction.
/// Batch b6 (Word for the web, 2026-11-07): an unlisted separator set
/// opened clean and the footnote rendered with its separator, so the
/// load failure the standard describes is not observed there and every
/// case reports at Warning there. Batch Mac-2 (Word for Mac,
/// 2026-11-08): the same fixture rendered its first screenful and then
/// left Word unresponsive — killed at the 3-minute observation limit,
/// the footnote check uncompletable. Verdicts combine by union across
/// consumers, so the rule reports at Error.
pub(crate) const NOTE_PR_LISTING: RuleSpec = RuleSpec { id: "NOTE-PR-LISTING", severity: Severity::Error, evidence: Evidence::Observed };
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
/// `CUSTOMXML-SCHEMA`: inline `w:customXml` inside a revision or move wrapper. MS-OI29500
/// documents that Word will not open it. Desktop Word opens files with unattached custom XML
/// schema references cleanly, so those references are reported as informational coverage.
pub(crate) const CUSTOMXML_SCHEMA: RuleSpec = RuleSpec { id: "CUSTOMXML-SCHEMA", severity: Severity::Error, evidence: Evidence::MsDocumented };
/// `MEDIA-MAGIC`: an image whose bytes do not match its extension/type. Word for the web loses
/// a drawn picture with unusable bytes. Unused images and usable alternate encodings warn.
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
    XML_DOCTYPE,
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
    /// absence conclusion is established while this is set (see [`contracts_generated::PACKAGE_ABSENCE_RULES`]).
    unknown_unreadable: bool,
}

impl Acc {
    /// Create a new accumulator initialized with the specified resource limits.
    pub(crate) fn with_limits(limits: Limits) -> Acc {
        Acc { limits, ..Acc::default() }
    }

    /// Record that a validation rule was executed.
    pub(crate) fn ran(&mut self, rule: &'static str) {
        observation_contract! {
            rules: "-";
            collections: "field.checks*1";
            scope: "report accumulator and emission budgets; no input definition collection";
            loss: "per-rule findings, total findings, or coverage-note budget";
            fallback: "preserve emitted evidence and record truncation/Incomplete; never supplies definition absence";
            package_loss: "independent";
        }

        self.checks.push(rule);
    }

    /// Record that the archive holds an entry whose identity could not be recovered. Absence
    /// conclusions are suppressed from here on (see [`contracts_generated::PACKAGE_ABSENCE_RULES`]).
    pub(crate) fn set_unknown_unreadable(&mut self) {
        self.unknown_unreadable = true;
    }

    /// Whether an absence-class finding must be suppressed because an entry of unknown identity
    /// exists: absence is not established. Suppression happens at emission, so no count is spent and
    /// no per-rule budget is touched.
    fn absence_suppressed(&self, rule: &str, severity: Severity) -> bool {
        self.unknown_unreadable
            && matches!(severity, Severity::Corrupt | Severity::Error)
            && contracts_generated::PACKAGE_ABSENCE_RULES.contains(&rule)
    }

    /// Whether another finding for `rule` may be recorded; once the per-rule budget is spent the
    /// run is marked incomplete and further findings of that rule are dropped.
    fn allow(&mut self, rule: &'static str) -> bool {
        observation_contract! {
            rules: "-";
            collections: "field.coverage*2,field.incomplete*1,field.rule_counts*1";
            scope: "report accumulator and emission budgets; no input definition collection";
            loss: "per-rule findings, total findings, or coverage-note budget";
            fallback: "preserve emitted evidence and record truncation/Incomplete; never supplies definition absence";
            package_loss: "independent";
        }

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
        observation_contract! {
            rules: "-";
            collections: "field.findings*1";
            scope: "report accumulator and emission budgets; no input definition collection";
            loss: "per-rule findings, total findings, or coverage-note budget";
            fallback: "preserve emitted evidence and record truncation/Incomplete; never supplies definition absence";
            package_loss: "independent";
        }

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
        observation_contract! {
            rules: "-";
            collections: "field.findings*1";
            scope: "report accumulator and emission budgets; no input definition collection";
            loss: "per-rule findings, total findings, or coverage-note budget";
            fallback: "preserve emitted evidence and record truncation/Incomplete; never supplies definition absence";
            package_loss: "independent";
        }

        if self.absence_suppressed(rule, severity) {
            return;
        }
        if !self.allow(rule) {
            return;
        }
        self.findings.push(Finding { rule, severity, evidence, part, message });
    }

    /// Record a proven finding, bypassing package-identity absence suppression.
    pub(crate) fn add_proven(&mut self, rule: &'static str, part: Option<String>, message: String) {
        observation_contract! {
            rules: "-";
            collections: "field.findings*1";
            scope: "report accumulator and emission budgets; no input definition collection";
            loss: "per-rule findings, total findings, or coverage-note budget";
            fallback: "preserve emitted evidence and record truncation/Incomplete; never supplies definition absence";
            package_loss: "independent";
        }

        let (severity, evidence) = rule_spec(rule).map_or((Severity::Error, Evidence::Inferred), |r| (r.severity, r.evidence));
        if !self.allow(rule) {
            return;
        }
        self.findings.push(Finding { rule, severity, evidence, part, message });
    }

    /// Record a coverage note. Exhausting the note budget is itself a budget exhaustion: the run is
    /// marked incomplete so it cannot pass silently, and the last slot is replaced with a
    /// deterministic truncation marker so the INCOMPLETE is always explained (H7).
    pub(crate) fn note(&mut self, message: String) {
        observation_contract! {
            rules: "-";
            collections: "field.coverage*5,field.incomplete*1";
            scope: "report accumulator and emission budgets; no input definition collection";
            loss: "per-rule findings, total findings, or coverage-note budget";
            fallback: "preserve emitted evidence and record truncation/Incomplete; never supplies definition absence";
            package_loss: "independent";
        }

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
        observation_contract! {
            rules: "-";
            collections: "field.incomplete*1";
            scope: "report accumulator and emission budgets; no input definition collection";
            loss: "per-rule findings, total findings, or coverage-note budget";
            fallback: "preserve emitted evidence and record truncation/Incomplete; never supplies definition absence";
            package_loss: "independent";
        }

        self.incomplete = true;
    }

    /// Sort findings deterministically and produce the final validation [`Report`].
    fn finish(mut self) -> Report {
        observation_contract! {
            rules: "-";
            collections: "field.checks*1,field.coverage*1,field.findings*3,field.incomplete*1";
            scope: "report accumulator and emission budgets; no input definition collection";
            loss: "per-rule findings, total findings, or coverage-note budget";
            fallback: "preserve emitted evidence and record truncation/Incomplete; never supplies definition absence";
            package_loss: "independent";
        }

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
    check_doctypes(&pkg, &mut acc);
    check_document(&pkg, &mut acc);
    acc.finish()
}

/// Apply document checks after the package-wide prolog scan.
fn check_document(pkg: &Package, acc: &mut Acc) {
    observation_contract! {
        rules: "-";
        collections: "field.content_parts*1";
        scope: "source-local bounded extraction; no implicit document owner";
        loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
        fallback: "retain proven prefix presence and duplicates; return loss to the caller; no negative membership from an incomplete set";
        package_loss: "independent";
    }

    if !pkg.roots_ambiguous {
        match package::entry_point_profile(pkg) {
            Resolution::Ambiguous(_) => {
                acc.note(
                    "mixed Transitional/Strict officeDocument entry points leave the package profile uncertain; document checks were skipped"
                        .to_string(),
                );
                acc.set_incomplete();
                return;
            }
            Resolution::One(package::EntryPointProfile::Strict) => {
                // The XML scanner understands Transitional WML only. Running it on Strict markup would
                // silently skip its elements and could manufacture a PASS without checking that profile.
                acc.note("the Strict OOXML profile is unsupported; document checks require the Transitional profile".to_string());
                acc.set_incomplete();
                return;
            }
            Resolution::One(package::EntryPointProfile::Transitional) | Resolution::Absent | Resolution::Unknown(_) => {}
        }
        if !owner_roots_supported(pkg, acc) {
            return;
        }
    }
    check_content_types(pkg, acc);
    let xml = check_xml(pkg, acc);
    check_rels(pkg, &xml, acc);
    check_content_part_types(pkg, &xml.content_parts(), acc);
    if !pkg.roots_ambiguous {
        check_story(pkg, &xml, acc);
        check_note_pr(pkg, &xml, acc);
    }
    check_media(pkg, &xml, acc);
}

/// A declaration in the original prolog is sufficient evidence, whatever the document profile.
fn check_doctypes(pkg: &Package, acc: &mut Acc) {
    observation_contract! {
        rules: "XML-DOCTYPE";
        collections: "field.bytes*1,field.content_types*3,field.entries*1";
        scope: "original prolog of every XML and relationships part, independent of ownership and profile; canonical footnotes relationships paths use existing story classification, diagram layout/colors/quickStyle paths use part identity, XHTML altChunk uses relationship and content-type evidence, custom XML uses relationship targets, the main document uses the resolved package owner and ordinary document content type, and other probed roles use part or relationships-owner content types";
        loss: "unavailable part bytes, undecidable or unsupported encoding, or unreadable prolog";
        fallback: "The probed matrix contains 56 part-role and declaration-form combinations, 47 with an adverse outcome and 9 tolerated (corpus/EVIDENCE.md). The tolerated combinations are footnotes relationships and diagram layout, colors, and quickStyle parts in the internal entity and ATTLIST forms, and XHTML altChunk in external identifier form without an internal subset. These combinations receive a structural Warning with ByConstruction evidence. Measured adverse combinations receive an Error with Observed evidence. All unmeasured part-role and declaration-form combinations receive a Warning with ByConstruction evidence as an expedient interim pending the measurements tracked in issue #7735. Existing coverage checks handle unavailable evidence";
        package_loss: "independent";
    }

    acc.ran("XML-DOCTYPE");
    let mut roles = None;
    let mut alt_chunk_targets = None;
    let mut custom_xml_targets = None;
    for entry in pkg.entries.iter().filter(|entry| !entry.is_dir && package::is_xml_part(pkg, &entry.name)) {
        let declared = match entry.decoded.as_deref() {
            Some(text) => scan::doctype_in_prolog(text),
            None if matches!(entry.decode_error, Some(DecodeError::Malformed(_))) => {
                entry.bytes.as_deref().and_then(scan::decoded_prefix).and_then(|text| scan::doctype_in_prolog(&text))
            }
            None => None,
        };
        if let Some(form) = declared {
            use scan::DoctypeForm::{ExternalOnly, InternalAttlist, InternalEntity, Other};

            let rels_owner = rels_owner_name(&entry.name).filter(|owner| entry.name.eq_ignore_ascii_case(&rels_part_name(owner)));
            let internal_form = matches!(form, InternalEntity | InternalAttlist);
            let is_tolerated = (internal_form
                && (is_diagram_definition_path(&entry.name)
                    || rels_owner
                        .as_ref()
                        .is_some_and(|owner| roles.get_or_insert_with(|| story_roles(pkg)).get(owner) == Some(&scan::PartRole::Footnotes))))
                || (form == ExternalOnly
                    && pkg.content_types.of(&entry.name) == Some("application/xhtml+xml")
                    && alt_chunk_targets
                        .get_or_insert_with(|| relationship_targets(pkg, "aFChunk"))
                        .contains(entry.name.to_ascii_lowercase().as_str()));
            let message = format!("{} contains a DOCTYPE declaration", entry.name);
            if is_tolerated {
                acc.add_graded("XML-DOCTYPE", Severity::Warning, Evidence::ByConstruction, Some(entry.name.clone()), message);
                continue;
            }

            let is_main_document = |name: &str, content_type| {
                content_type == Some(package::MAIN_CONTENT_TYPE) && matches!(&pkg.main, Resolution::One(main) if main.eq_ignore_ascii_case(name))
            };
            let is_measured_adverse = match (form, rels_owner.as_deref()) {
                (Other, _) => false,
                (_, _) if entry.name.eq_ignore_ascii_case("_rels/.rels") => internal_form,
                (InternalEntity | InternalAttlist, Some(owner)) => {
                    let content_type = pkg.content_types.of(owner);
                    (form == InternalEntity && is_main_document(owner, content_type))
                        || content_type == Some(package::CHART_CONTENT_TYPE)
                        || custom_xml_targets.get_or_insert_with(|| relationship_targets(pkg, "customXml")).contains(owner)
                        || (form == InternalEntity
                            && matches!(
                                content_type,
                                Some(
                                    package::HEADER_CONTENT_TYPE
                                        | package::FOOTER_CONTENT_TYPE
                                        | package::ENDNOTES_CONTENT_TYPE
                                        | package::COMMENTS_CONTENT_TYPE
                                )
                            ))
                }
                (ExternalOnly, Some(_)) => false,
                (_, None) => {
                    let content_type = pkg.content_types.of(&entry.name);
                    is_main_document(&entry.name, content_type)
                        || is_measured_adverse_doctype(form, content_type)
                        || (form == InternalEntity
                            && custom_xml_targets
                                .get_or_insert_with(|| relationship_targets(pkg, "customXml"))
                                .contains(entry.name.to_ascii_lowercase().as_str()))
                }
            };
            if is_measured_adverse {
                acc.add("XML-DOCTYPE", Some(entry.name.clone()), format!("{message}; see corpus/EVIDENCE.md for measured outcomes"));
            } else {
                acc.add_graded("XML-DOCTYPE", Severity::Warning, Evidence::ByConstruction, Some(entry.name.clone()), message);
            }
        }
    }
}

/// The adverse non-relationships roles in the isolated, closure, and forms probe series.
fn is_measured_adverse_doctype(form: scan::DoctypeForm, content_type: Option<&str>) -> bool {
    match content_type {
        Some(
            package::STYLES_CONTENT_TYPE
            | package::SETTINGS_CONTENT_TYPE
            | package::FONT_TABLE_CONTENT_TYPE
            | package::THEME_CONTENT_TYPE
            | package::NUMBERING_CONTENT_TYPE
            | "application/vnd.ms-word.stylesWithEffects+xml",
        ) => true,
        Some(
            package::HEADER_CONTENT_TYPE
            | package::FOOTER_CONTENT_TYPE
            | package::FOOTNOTES_CONTENT_TYPE
            | package::ENDNOTES_CONTENT_TYPE
            | package::COMMENTS_CONTENT_TYPE
            | package::GLOSSARY_CONTENT_TYPE
            | package::CHART_CONTENT_TYPE
            | package::COMMENTS_EXTENDED_CONTENT_TYPE
            | package::COMMENTS_IDS_CONTENT_TYPE
            | package::PEOPLE_CONTENT_TYPE
            | "application/vnd.openxmlformats-package.core-properties+xml"
            | "application/vnd.openxmlformats-officedocument.drawingml.diagramData+xml"
            | "application/vnd.ms-office.classificationlabels+xml"
            | "application/vnd.ms-office.webextension+xml",
        ) => form == scan::DoctypeForm::InternalEntity,
        _ => false,
    }
}

fn is_diagram_definition_path(path: &str) -> bool {
    let path = path.to_ascii_lowercase();
    let Some(("word/diagrams", name)) = path.rsplit_once('/') else { return false };
    name.ends_with(".xml") && (name.starts_with("layout") || name.starts_with("colors") || name.starts_with("quickstyle"))
}

/// Reject owner roots the Transitional scanner cannot interpret. Malformed or unavailable XML
/// remains the ordinary XML check's responsibility; this gate checks only an observed root.
fn owner_roots_supported(pkg: &Package, acc: &mut Acc) -> bool {
    observation_contract! {
        rules: "-";
        collections: "lookup.entry_by_name*1,type.Vec*1";
        scope: "every independently established main/glossary owner root; Transitional namespace and expected root element";
        loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
        fallback: "retain proven prefix presence and duplicates; return loss to the caller; no negative membership from an incomplete set";
        package_loss: "independent";
    }

    use quick_xml::{events::Event, name::ResolveResult};
    let mut supported = true;
    for owner in owner_scopes(pkg) {
        let (name, expected) = match &owner {
            OwnerContext::Main(name) => (name, "document"),
            OwnerContext::Glossary(name) => (name, "glossaryDocument"),
            OwnerContext::Fallback => continue,
        };
        let Resolution::One(entry) = entry_by_name(pkg, name) else { continue };
        let Some(text) = entry.decoded.as_deref() else { continue };
        let mut reader = xml::XmlReader::new(text);
        let mut buf = Vec::new();
        for _ in 0..acc.limits.events {
            match reader.read_resolved_event_into(&mut buf) {
                Ok((namespace, Event::Start(element) | Event::Empty(element))) => {
                    let unresolved = matches!(namespace, ResolveResult::Unknown(_));
                    let ns = match namespace {
                        ResolveResult::Bound(ns) => String::from_utf8_lossy(ns.as_ref()).into_owned(),
                        ResolveResult::Unbound | ResolveResult::Unknown(_) => String::new(),
                    };
                    let local = element.local_name();
                    if unresolved {
                        // An unresolved namespace neither satisfies nor fails the owner check.
                        acc.note(format!(
                            "{} has a root in a namespace that is not resolved (an undeclared prefix or a declaration that cannot be expanded); its ownership is unknown and document checks were not run",
                            entry.name
                        ));
                        acc.set_incomplete();
                        supported = false;
                    } else if ns != NS_W || local.as_ref() != expected.as_bytes() {
                        let actual = format!("{{{ns}}}{}", String::from_utf8_lossy(local.as_ref()));
                        let profile =
                            if ns == "http://purl.oclc.org/ooxml/wordprocessingml/main" { "the Strict OOXML profile is unsupported; " } else { "" };
                        acc.note(format!("{profile}{} has root {actual}; document checks require {{{NS_W}}}{expected}", entry.name));
                        acc.set_incomplete();
                        supported = false;
                    }
                    break;
                }
                Ok((_, Event::Eof)) | Err(_) => break,
                _ => {}
            }
            buf.clear();
        }
    }
    supported
}

/// Validate a document read from a path. An I/O error reading the file is returned; every
/// problem found inside the file is a finding. A file larger than the input budget is not read:
/// it yields an `INCOMPLETE` report instead.
pub fn validate_path(path: &Path) -> std::io::Result<Report> {
    observation_contract! {
        rules: "-";
        collections: "type.Vec*1";
        scope: "source-local bounded extraction; no implicit document owner";
        loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
        fallback: "retain proven prefix presence and duplicates; return loss to the caller; no negative membership from an incomplete set";
        package_loss: "independent";
    }

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
    observation_contract! {
        rules: "CT-DUP-DEFAULT,CT-DUP-MAIN,CT-DUP-OVERRIDE,CT-MAIN-TYPE,CT-UNDECLARED";
        collections: "access.missing*1,field.bytes*1,field.content_types*7,field.dup_defaults*1,field.dup_overrides*1,field.entries*1,field.incomplete*2,lookup.content_unavailable*1";
        scope: "source-local bounded extraction; no implicit document owner";
        loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
        fallback: "retain proven prefix presence and duplicates; return loss to the caller; no negative membership from an incomplete set";
        package_loss: "CT-UNDECLARED,CT-MAIN-TYPE";
    }

    if !pkg.content_types_present {
        return;
    }
    // The manifest is present but its content could not be read: no declaration is known, so no
    // part is established as undeclared (or mistyped). Absence is not established.
    if match package::content_unavailable(pkg, "[Content_Types].xml") {
        Resolution::One(unavailable) => unavailable,
        Resolution::Absent => false,
        Resolution::Unknown(_) => true,
        Resolution::Ambiguous(_) => true,
    } {
        return;
    }
    acc.ran("CT-UNDECLARED");
    let mut internal_targets = None;
    for e in pkg.entries.iter().filter(|e| !e.is_dir) {
        let l = e.name.to_ascii_lowercase();
        if l == "[content_types].xml" || l.ends_with(".rels") {
            continue;
        }
        if !pkg.content_types.incomplete && pkg.content_types.of(&e.name).is_none() {
            let message = format!("part {} has no effective content type", e.name);
            if is_tombstone_name(&e.name)
                && e.bytes.as_deref().is_some_and(|bytes| bytes.starts_with(&[0xff; 4]))
                && internal_targets.get_or_insert_with(|| complete_internal_targets(pkg)).as_ref().is_some_and(|targets| targets.missing(&l))
            {
                if !acc.absence_suppressed("CT-UNDECLARED", Severity::Error) {
                    acc.note(format!("CT-UNDECLARED: {message}"));
                }
            } else {
                acc.add("CT-UNDECLARED", Some(e.name.clone()), message);
            }
        }
    }
    acc.ran("CT-MAIN-TYPE");
    if let Resolution::One(main) = &pkg.main
        && (!pkg.content_types.incomplete || pkg.content_types.of(main).is_some())
        && !is_main_typed(pkg, main)
    {
        let got = pkg.content_types.of(main).unwrap_or("(none)");
        acc.add("CT-MAIN-TYPE", Some(main.clone()), format!("main part {main} has content type {got}, not a main-document content type"));
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

fn is_tombstone_name(name: &str) -> bool {
    name.strip_prefix("[trash]/")
        .and_then(|name| name.strip_suffix(".dat"))
        .is_some_and(|digits| digits.len() == 4 && digits.as_bytes().iter().all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f')))
}

/// Inventory every internal target only after complete relationship discovery. A `w:contentPart`
/// can consume a part only through a relationship. Inspect every declaration, including those
/// outside root reachability and those that do not win first-ID binding.
fn complete_internal_targets(pkg: &Package) -> Option<CompleteSet<String>> {
    observation_contract! {
        rules: "-";
        collections: "access.into_complete_set*1,field.collapsed_names*1,field.rels*3,type.CompleteSet*1,type.Observed*1";
        scope: "all package relationship declarations; conservative superset of explicit and implicit consumers";
        loss: "entry discovery incomplete, collapsed relationship records, or a malformed, unrecognized or incomplete relationship source";
        fallback: "withhold all non-consumption conclusions";
        package_loss: "independent";
    }

    if !ownership_discovery_complete(pkg) || !pkg.collapsed_names.is_empty() || pkg.rels.iter().any(|rp| !rp.inventory_complete) {
        return None;
    }
    let targets = pkg.rels.iter().flat_map(|rp| &rp.rels).filter_map(|rel| rel.target_abs.as_deref().map(str::to_ascii_lowercase));
    Observed::complete(targets.collect()).into_complete_set()
}

/// Whether `name`'s effective content type is one of the package's main-document content types
/// ([`package::MAIN_CONTENT_TYPES`]: document, template, and their macro-enabled variants).
fn is_main_typed(pkg: &Package, name: &str) -> bool {
    observation_contract! {
        rules: "-";
        collections: "field.content_types*1";
        scope: "source-local bounded extraction; no implicit document owner";
        loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
        fallback: "retain proven prefix presence and duplicates; return loss to the caller; no negative membership from an incomplete set";
        package_loss: "independent";
    }

    pkg.content_types.of(name).is_some_and(package::is_main_content_type)
}

/// `CT-DUP-MAIN`: at most one part may carry the main-document content type (ECMA-376 Part 1
/// §11.3.10). Every main-typed part other than the one the package's `officeDocument` relationship
/// reaches is reported. An untargeted ordinary document beside an established ordinary main is
/// inert and appears in coverage instead of findings.
fn check_dup_main(pkg: &Package, acc: &mut Acc) {
    observation_contract! {
        rules: "CT-DUP-MAIN";
        collections: "access.missing*1,field.content_types*2,field.entries*1,type.Vec*1";
        scope: "main-typed OPC parts; inert classification requires a selected ordinary main and complete internal target discovery";
        loss: "selection unavailable or ambiguous; incomplete content-type or relationship discovery";
        fallback: "retain proven cardinality findings when inertness cannot be established";
        package_loss: "independent";
    }

    // The manifest supplies types to parts but is itself package metadata.
    let main_typed: Vec<&str> = pkg
        .entries
        .iter()
        .filter(|e| !e.is_dir && !e.name.eq_ignore_ascii_case("[Content_Types].xml") && is_main_typed(pkg, &e.name))
        .map(|e| e.name.as_str())
        .collect();
    if main_typed.len() <= 1 {
        return;
    }
    let anchor = match &pkg.main {
        Resolution::One(name) => Some(name.as_str()),
        Resolution::Absent | Resolution::Ambiguous(_) | Resolution::Unknown(_) => None,
    };
    let ordinary_main = anchor.filter(|main| {
        pkg.content_types.of(main) == Some(package::MAIN_CONTENT_TYPE)
            && main_typed.iter().filter(|candidate| candidate.eq_ignore_ascii_case(main)).count() == 1
    });
    let internal_targets = ordinary_main.and_then(|_| complete_internal_targets(pkg));
    for name in main_typed {
        if anchor.is_some_and(|a| a.eq_ignore_ascii_case(name)) {
            continue;
        }
        if let (Some(main), Some(targets)) = (ordinary_main, &internal_targets)
            && pkg.content_types.of(name) == Some(package::MAIN_CONTENT_TYPE)
            && targets.missing(&name.to_ascii_lowercase())
        {
            acc.note(format!(
                "CT-DUP-MAIN: {name} also carries the main document content type; the package officeDocument relationship selects {main}"
            ));
            continue;
        }
        let message = if anchor.is_some() {
            format!(
                "{name} carries the main document content type, but the package's officeDocument relationship reaches a different part; a package may have only one main document"
            )
        } else {
            format!("{name} carries a main document content type alongside another part; a package may have only one main document")
        };
        acc.add("CT-DUP-MAIN", Some(name.to_string()), message);
    }
}

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
    graph: PartGraph,
    parts: std::collections::BTreeMap<PartId, PartFacts>,
    compat: Compat,
}

/// Per-part facts retain completeness beside each collection, rather than in a global pool.
struct PartFacts {
    name: String,
    syntax: Completeness,
    references: Observed<Vec<RefFact>>,
    drawing_refs_complete: bool,
    content: Observed<Vec<RelId>>,
    exemptions: Observed<Vec<RelId>>,
    facts: scan::StoryFacts,
    listings: (Observed<Vec<(DecimalId, String)>>, Observed<Vec<(DecimalId, String)>>),
    special_notes: Observed<Vec<(String, DecimalId, bool, String)>>,
}
impl PartFacts {
    /// Create empty auxiliary observations with the exact source availability before distributing budgeted values.
    fn new(
        name: &str,
        facts: scan::StoryFacts,
        syntax_complete: bool,
        note_role_known: bool,
        references_complete: bool,
        drawing_refs_complete: bool,
    ) -> Self {
        observation_contract! {
            rules: "-";
            collections: "type.Observed*6,type.Vec*6";
            scope: "source-local bounded extraction; no implicit document owner";
            loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
            fallback: "retain proven prefix presence and duplicates; return loss to the caller; no negative membership from an incomplete set";
            package_loss: "independent";
        }

        let syntax = if syntax_complete { Completeness::Complete } else { Completeness::Incomplete("XML source incomplete") };
        let note_status = if note_role_known { syntax } else { Completeness::Incomplete("conflicting note role") };
        Self {
            name: name.to_string(),
            syntax,
            facts,
            references: Observed::with_status(
                Vec::new(),
                if references_complete { Completeness::Complete } else { Completeness::Incomplete("relationship reference source or budget") },
            ),
            drawing_refs_complete,
            content: Observed::with_status(Vec::new(), syntax),
            exemptions: Observed::with_status(Vec::new(), syntax),
            listings: (Observed::with_status(Vec::new(), note_status), Observed::with_status(Vec::new(), note_status)),
            special_notes: Observed::with_status(Vec::new(), note_status),
        }
    }
}
impl XmlScan {
    /// Retained references, assembled from their contributing parts for presence checks.
    fn references(&self) -> Vec<(String, Vec<RefFact>)> {
        observation_contract! {
            rules: "-";
            collections: "access.prefix*2,field.parts*1,field.references*2,type.Vec*2";
            scope: "source-local bounded extraction; no implicit document owner";
            loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
            fallback: "retain proven prefix presence and duplicates; return loss to the caller; no negative membership from an incomplete set";
            package_loss: "independent";
        }

        self.parts.values().filter(|p| !p.references.prefix().is_empty()).map(|p| (p.name.clone(), p.references.prefix().clone())).collect()
    }
    /// The positive contentPart scope, excluding skipped and native-ink exemptions.
    fn content_parts(&self) -> Vec<(String, Vec<RelId>)> {
        observation_contract! {
            rules: "-";
            collections: "access.prefix*2,field.content*2,field.parts*1,type.Vec*2";
            scope: "source-local bounded extraction; no implicit document owner";
            loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
            fallback: "retain proven prefix presence and duplicates; return loss to the caller; no negative membership from an incomplete set";
            package_loss: "independent";
        }

        self.parts.values().filter(|p| !p.content.prefix().is_empty()).map(|p| (p.name.clone(), p.content.prefix().clone())).collect()
    }
    /// Explicit package-global scope for counts and legacy document-wide duplicate rules.
    fn package_facts(&self) -> scan::StoryFacts {
        observation_contract! {
            rules: "-";
            collections: "access.append*1,field.parts*1";
            scope: "intentional package-wide reductions for counts and document-wide duplicates, including unreached stories; definition duplicates remain per definition part";
            loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
            fallback: "retain count/duplicate evidence from observed prefixes; loss remains recorded at each source; owner-local joins assemble their own scope";
            package_loss: "independent";
        }

        let mut result = scan::StoryFacts::default();
        for part in self.parts.values() {
            result.append(&part.facts);
        }
        result
    }
    /// Assemble the settings-listing rule's explicit source-scoped view.
    fn note_listings(&self) -> (Vec<(DecimalId, String)>, Vec<(DecimalId, String)>) {
        observation_contract! {
            rules: "-";
            collections: "access.prefix*2,field.listings*2,field.parts*1,type.Vec*4";
            scope: "source-local bounded extraction; no implicit document owner";
            loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
            fallback: "retain proven prefix presence and duplicates; return loss to the caller; no negative membership from an incomplete set";
            package_loss: "independent";
        }

        let mut result = (Vec::new(), Vec::new());
        for part in self.parts.values() {
            result.0.extend_from_slice(part.listings.0.prefix());
            result.1.extend_from_slice(part.listings.1.prefix());
        }
        result
    }
    /// Special-note presence remains usable even when another collection is incomplete.
    fn special_notes(&self) -> Vec<(String, DecimalId, bool, String)> {
        observation_contract! {
            rules: "-";
            collections: "access.prefix*1,field.parts*1,field.special_notes*1,type.Vec*1";
            scope: "source-local bounded extraction; no implicit document owner";
            loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
            fallback: "retain proven prefix presence and duplicates; return loss to the caller; no negative membership from an incomplete set";
            package_loss: "independent";
        }

        self.parts.values().flat_map(|p| p.special_notes.prefix().iter().cloned()).collect()
    }
    /// Completeness of the selected settings/listing category, independent of unrelated collections.
    fn notes_complete(&self, part: &str) -> bool {
        observation_contract! {
            rules: "-";
            collections: "access.is_complete*3,field.listings*2,field.parts*1,field.special_notes*1";
            scope: "source-local bounded extraction; no implicit document owner";
            loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
            fallback: "retain proven prefix presence and duplicates; return loss to the caller; no negative membership from an incomplete set";
            package_loss: "independent";
        }

        self.parts.get(&PartId::new(part)).is_some_and(|p| p.listings.0.is_complete() && p.listings.1.is_complete() && p.special_notes.is_complete())
    }
    /// Completeness of one family's settings listing (`end` selects endnotes) and of the part's note definitions.
    fn listing_complete(&self, part: &str, end: bool) -> bool {
        observation_contract! {
            rules: "-";
            collections: "access.is_complete*2,field.listings*2,field.parts*1,field.special_notes*1";
            scope: "source-local bounded extraction; no implicit document owner";
            loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
            fallback: "retain proven prefix presence and duplicates; return loss to the caller; no negative membership from an incomplete set";
            package_loss: "independent";
        }

        self.parts
            .get(&PartId::new(part))
            .is_some_and(|p| (if end { &p.listings.1 } else { &p.listings.0 }).is_complete() && p.special_notes.is_complete())
    }
    /// A fact family carries the completeness of its exact source, including pooled-budget loss.
    fn category_complete<K>(&self, part: &str, category: fn(&scan::StoryFacts) -> &Observed<Vec<(K, String)>>) -> bool {
        observation_contract! {
            rules: "-";
            collections: "access.is_complete*1,field.parts*1,type.Observed*1,type.Vec*1";
            scope: "source-local bounded extraction; no implicit document owner";
            loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
            fallback: "retain proven prefix presence and duplicates; return loss to the caller; no negative membership from an incomplete set";
            package_loss: "independent";
        }

        self.parts.get(&PartId::new(part)).is_some_and(|p| category(&p.facts).is_complete())
    }
}

/// Auxiliary collectors check their own category budgets after completed XML syntax observation.
fn xml_source_complete(xml: &XmlScan, entry: &package::Entry) -> bool {
    observation_contract! {
        rules: "-";
        collections: "field.parts*1";
        scope: "source-local bounded extraction; no implicit document owner";
        loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
        fallback: "retain proven prefix presence and duplicates; return loss to the caller; no negative membership from an incomplete set";
        package_loss: "independent";
    }

    !entry.content_unavailable && xml.parts.get(&PartId::new(&entry.name)).is_some_and(|p| p.syntax == Completeness::Complete)
}

/// Scan all XML parts for well-formedness, undeclared prefixes, illegal characters, and extract story facts.
fn check_xml(pkg: &Package, acc: &mut Acc) -> XmlScan {
    observation_contract! {
        rules: "HEADERSOURCE-PRESENT,MATH-PLACEMENT,PROTECTION-PASSWORD,STRUCT-END-P,XML-INVALID-CHAR,XML-MALFORMED,XML-UNDECLARED-PREFIX";
        collections: "access.prefix*1,access.retain_budget*1,field.comment_para_ids*1,field.content_part_exemptions*2,field.content_parts*2,field.entries*1,field.ink_content_parts*2,field.note_listings*1,field.refs*5,field.special_notes*1,field.unknown_prefixes*1,type.BTreeMap*1,type.HashMap*2,type.HashSet*2,type.Vec*13";
        scope: "source-local bounded extraction; no implicit document owner";
        loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
        fallback: "retain proven prefix presence and duplicates; return loss to the caller; no negative membership from an incomplete set";
        package_loss: "independent";
    }

    acc.ran("XML-MALFORMED");
    acc.ran("XML-UNDECLARED-PREFIX");
    acc.ran("XML-INVALID-CHAR");
    acc.ran("STRUCT-END-P");
    acc.ran("MATH-PLACEMENT");
    acc.ran("PROTECTION-PASSWORD");
    acc.ran("HEADERSOURCE-PRESENT");
    let roles = story_roles(pkg);
    let graph = PartGraph::new(pkg);
    if graph.budget_exhausted() {
        acc.note(
            "the owner graph exceeded its traversal budget; established memberships were retained and unreached memberships remain unknown".into(),
        );
        acc.set_incomplete();
    }
    let mut parts = std::collections::BTreeMap::new();
    let mut fact_budget = [0usize; 14];

    let mut refs: Vec<(String, Vec<RefFact>)> = Vec::new();
    let mut content_parts: Vec<(String, Vec<RelId>)> = Vec::new();
    let mut content_part_total = 0usize;
    let mut content_part_capped_any = false;
    let mut content_part_exemptions: Vec<(String, Vec<RelId>)> = Vec::new();
    let mut content_part_exemptions_total = 0usize;
    let mut content_part_incomplete_owners: HashSet<String> = HashSet::new();
    let mut compat_on = false;
    let mut compat_unknown = false;
    let mut note_listings = (Vec::new(), Vec::new());
    let mut special_notes: Vec<(String, DecimalId, bool, String)> = Vec::new();
    let mut note_incomplete_parts: HashMap<String, u8> = HashMap::new();
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
        if role.is_settings() && e.decode_error.is_some() {
            compat_unknown = true;
            *note_incomplete_parts.entry(l.clone()).or_default() |= scan::NOTE_ALL;
        }
        if (role.is_footnotes() || role.is_endnotes()) && e.decode_error.is_some() {
            *note_incomplete_parts.entry(l.clone()).or_default() |= scan::NOTE_ALL;
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
        let mut scanned = scan::scan_part(&e.name, text, role, acc);

        *note_incomplete_parts.entry(l.clone()).or_default() |= scanned.note_loss;
        if append_note_items(&mut note_listings, &mut special_notes, &e.name, scanned.note_listings, scanned.special_notes, acc.limits.facts) {
            *note_incomplete_parts.entry(l.clone()).or_default() |= scan::NOTE_ALL;
        }
        if role.is_settings() {
            if scanned.malformed.is_some() {
                compat_unknown = true;
                *note_incomplete_parts.entry(l.clone()).or_default() |= scan::NOTE_ALL;
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
        if role.is_story() && fact_budget[2].saturating_add(scanned.facts.comment_para_ids.prefix().len()) > acc.limits.facts {
            acc.note(format!("{} lost pooled comment paragraph IDs at the {}-entry fact budget; comment-threading paragraph absence was not checked for its owners", e.name, acc.limits.facts));
        }
        if role.is_story() && scanned.facts.retain_budget(&mut fact_budget, acc.limits.facts) {
            pooled_capped = true;
            if facts_pooled_part.is_none() {
                facts_pooled_part = Some(e.name.clone());
            }
        }
        let references_complete = scanned.facts_complete && !refs_pooled_capped && pooled_refs.saturating_add(scanned.refs.len()) <= acc.limits.facts;
        let drawing_refs_complete = scanned.drawing_refs_complete && role.is_story() && !role.conflicting();
        parts.insert(
            PartId::new(&e.name),
            PartFacts::new(&e.name, scanned.facts, scanned.syntax_complete, !role.conflicting(), references_complete, drawing_refs_complete),
        );
        if scanned.content_part_capped {
            content_part_capped_any = true;
            content_part_incomplete_owners.insert(e.name.clone());
        }
        if !scanned.content_parts.is_empty()
            && append_content_parts(&mut content_parts, &mut content_part_total, &e.name, scanned.content_parts, acc.limits.facts)
        {
            content_part_capped_any = true;
            content_part_incomplete_owners.insert(e.name.clone());
        }
        retain_skipped_text_xml_refs(pkg, &e.name, &mut scanned.content_part_exemptions);
        let ink_targets = retain_native_ink_refs(pkg, &e.name, &mut scanned.ink_content_parts, scanned.ink_content_part_capped);
        let mut exemptions_capped = append_content_parts(
            &mut content_part_exemptions,
            &mut content_part_exemptions_total,
            &e.name,
            scanned.content_part_exemptions,
            acc.limits.facts,
        );
        exemptions_capped |= append_content_parts(
            &mut content_part_exemptions,
            &mut content_part_exemptions_total,
            &e.name,
            scanned.ink_content_parts,
            acc.limits.facts,
        );
        if scanned.content_part_exemptions_capped || (scanned.ink_content_part_capped && ink_targets) || exemptions_capped {
            // Lost references cannot establish non-consumption; native caps matter only where InkML targets exist.
            content_part_incomplete_owners.insert(e.name.clone());
            acc.note(format!(
                "the fact budget stopped collection of contentPart pairing exemptions in {}; its customXml pairing was not checked",
                e.name
            ));
            acc.set_incomplete();
        }
        if !scanned.syntax_complete || scanned.fact_capped || scanned.refs_capped || scanned.content_part_unresolved {
            // An element of unresolved namespace named `contentPart` may be the consumer that exempts a customXml
            // part, so it too cannot establish non-consumption. A scan stopped by events/depth cannot establish non-consumption, even when no
            // contentPart id was reached. Fact caps also conservatively invalidate this source.
            content_part_incomplete_owners.insert(e.name.clone());
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
    if content_part_capped_any {
        acc.note(
            "the fact budget stopped collection of some w:contentPart references, so CONTENTPART-TYPE did not check every reference, and CT-MISMATCH's customXml pairing was not applied to owners whose collection stopped".to_string(),
        );
        acc.set_incomplete();
    }
    store_note_observations(&mut parts, note_listings, special_notes, note_incomplete_parts, acc.limits.facts);
    store_relationship_observations(&mut parts, refs, content_parts, content_part_exemptions, content_part_incomplete_owners);
    XmlScan { graph, parts, compat }
}

/// Attach each relationship fact collection's loss to its contributing source.
fn store_relationship_observations(
    parts: &mut std::collections::BTreeMap<PartId, PartFacts>,
    refs: Vec<(String, Vec<RefFact>)>,
    content: Vec<(String, Vec<RelId>)>,
    exemptions: Vec<(String, Vec<RelId>)>,
    incomplete: HashSet<String>,
) {
    observation_contract! {
        rules: "-";
        collections: "access.invalidate*2,access.project*3,field.content*3,field.exemptions*3,field.references*2,type.BTreeMap*1,type.HashSet*1,type.Vec*6";
        scope: "source-local bounded extraction; no implicit document owner";
        loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
        fallback: "retain proven prefix presence and duplicates; return loss to the caller; no negative membership from an incomplete set";
        package_loss: "independent";
    }

    for (name, values) in refs {
        if let Some(part) = parts.get_mut(&PartId::new(&name)) {
            part.references = part.references.project(|_| values);
        }
    }
    for (name, values) in content {
        if let Some(part) = parts.get_mut(&PartId::new(&name)) {
            part.content = part.content.project(|_| values);
        }
    }
    for (name, values) in exemptions {
        if let Some(part) = parts.get_mut(&PartId::new(&name)) {
            part.exemptions = part.exemptions.project(|prev| {
                let mut merged = prev.clone();
                merged.extend(values);
                merged
            });
        }
    }
    for name in incomplete {
        if let Some(part) = parts.get_mut(&PartId::new(&name)) {
            part.content.invalidate("source consumption incomplete");
            part.exemptions.invalidate("source consumption incomplete");
        }
    }
}

/// Distribute the budgeted note categories back to their contributing part, retaining source loss.
fn store_note_observations(
    parts: &mut std::collections::BTreeMap<PartId, PartFacts>,
    note_listings: (Vec<(DecimalId, String)>, Vec<(DecimalId, String)>),
    special_notes: Vec<(String, DecimalId, bool, String)>,
    note_incomplete_parts: HashMap<String, u8>,
    cap: usize,
) {
    observation_contract! {
        rules: "-";
        collections: "access.invalidate*3,access.push_bounded*3,field.listings*4,field.special_notes*2,type.BTreeMap*1,type.HashMap*1,type.Vec*3";
        scope: "source-local bounded extraction; no implicit document owner";
        loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
        fallback: "retain proven prefix presence and duplicates; return loss to the caller; no negative membership from an incomplete set";
        package_loss: "independent";
    }

    for (id, part) in note_listings.0 {
        if let Some(facts) = parts.get_mut(&PartId::new(&part)) {
            facts.listings.0.push_bounded((id, part), cap);
        }
    }
    for (id, part) in note_listings.1 {
        if let Some(facts) = parts.get_mut(&PartId::new(&part)) {
            facts.listings.1.push_bounded((id, part), cap);
        }
    }
    for (part, id, end, kind) in special_notes {
        if let Some(facts) = parts.get_mut(&PartId::new(&part)) {
            facts.special_notes.push_bounded((part, id, end, kind), cap);
        }
    }
    for (name, loss) in note_incomplete_parts {
        if let Some(part) = parts.get_mut(&PartId::new(&name)) {
            if loss & scan::NOTE_LIST_F != 0 {
                part.listings.0.invalidate("note collection budget or source");
            }
            if loss & scan::NOTE_LIST_E != 0 {
                part.listings.1.invalidate("note collection budget or source");
            }
            if loss & scan::NOTE_DEFS != 0 {
                part.special_notes.invalidate("note collection budget or source");
            }
        }
    }
}

/// Append one part's note listings and special definitions under the
/// pooled fact budget, enforcing the allowance while appending: the
/// accumulated collections never exceed `cap`, however many parts
/// contribute. Returns true when any entry was discarded.
fn append_note_items(
    note_listings: &mut (Vec<(DecimalId, String)>, Vec<(DecimalId, String)>),
    special_notes: &mut Vec<(String, DecimalId, bool, String)>,
    part: &str,
    listings: (Vec<DecimalId>, Vec<DecimalId>),
    specials: Vec<(DecimalId, bool, String)>,
    cap: usize,
) -> bool {
    observation_contract! {
        rules: "-";
        collections: "type.Vec*6";
        scope: "source-local bounded extraction; no implicit document owner";
        loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
        fallback: "retain proven prefix presence and duplicates; return loss to the caller; no negative membership from an incomplete set";
        package_loss: "independent";
    }

    let mut capped = false;
    let room = cap.saturating_sub(note_listings.0.len() + note_listings.1.len());
    let take_f = room.min(listings.0.len());
    let take_e = (room - take_f).min(listings.1.len());
    if take_f < listings.0.len() || take_e < listings.1.len() {
        capped = true;
    }
    note_listings.0.extend(listings.0.into_iter().take(take_f).map(|id| (id, part.to_string())));
    note_listings.1.extend(listings.1.into_iter().take(take_e).map(|id| (id, part.to_string())));
    for (id, is_end, t) in specials {
        if special_notes.len() >= cap {
            capped = true;
            break;
        }
        special_notes.push((part.to_string(), id, is_end, t));
    }
    capped
}

/// Append one part's contentPart validation or exemption ids under the pooled fact
/// budget, enforcing the allowance while appending (see
/// `append_note_items`). Returns true when any id was discarded.
fn append_content_parts(content_parts: &mut Vec<(String, Vec<RelId>)>, total: &mut usize, part: &str, mut ids: Vec<RelId>, cap: usize) -> bool {
    observation_contract! {
        rules: "-";
        collections: "type.Vec*3";
        scope: "source-local bounded extraction; no implicit document owner";
        loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
        fallback: "retain proven prefix presence and duplicates; return loss to the caller; no negative membership from an incomplete set";
        package_loss: "independent";
    }

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

/// Retain only skipped references whose first-wins relationship resolves to text/xml.
/// Word for the web opens that pairing but refuses editing an application/octet-stream target;
/// skipped content must retain CT-MISMATCH for other types without gaining CONTENTPART-TYPE.
fn retain_skipped_text_xml_refs(pkg: &Package, owner: &str, ids: &mut Vec<RelId>) {
    observation_contract! {
        rules: "-";
        collections: "field.content_types*1,field.rels*1,type.Vec*1";
        scope: "source-local bounded extraction; no implicit document owner";
        loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
        fallback: "retain proven prefix presence and duplicates; return loss to the caller; no negative membership from an incomplete set";
        package_loss: "independent";
    }

    if ids.is_empty() {
        return;
    }
    let rels_name = rels_part_name(owner);
    let Some(rp) = pkg.rels.iter().find(|rp| rp.name.eq_ignore_ascii_case(&rels_name)) else {
        ids.clear();
        return;
    };
    let by_id = package::relationship_bindings(rp);
    ids.retain(|id| match by_id.resolve(id.as_str()) {
        Resolution::One(rel) => {
            rel.target_abs.as_deref().and_then(|target| pkg.content_types.of(target)).is_some_and(|ct| ct.eq_ignore_ascii_case("text/xml"))
        }
        Resolution::Absent | Resolution::Ambiguous(_) | Resolution::Unknown(_) => false,
    });
}

/// Retain first-wins native references to InkML (MS-ODRAWXML §2.1.4) and report whether this owner has any InkML target.
/// A capped native collection matters only for such an owner; documents without ink keep their existing checks and verdicts.
fn retain_native_ink_refs(pkg: &Package, owner: &str, ids: &mut Vec<RelId>, capped: bool) -> bool {
    observation_contract! {
        rules: "-";
        collections: "field.content_types*1,field.rels*1,type.Vec*1";
        scope: "source-local bounded extraction; no implicit document owner";
        loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
        fallback: "retain proven prefix presence and duplicates; return loss to the caller; no negative membership from an incomplete set";
        package_loss: "independent";
    }

    if ids.is_empty() && !capped {
        return false;
    }
    let rels_name = rels_part_name(owner);
    let Some(rp) = pkg.rels.iter().find(|rp| rp.name.eq_ignore_ascii_case(&rels_name)) else {
        ids.clear();
        return false;
    };
    let by_id = package::relationship_bindings(rp);
    let is_ink = |rel: &package::Rel| {
        rel.target_abs.as_deref().and_then(|target| pkg.content_types.of(target)).is_some_and(|ct| ct.eq_ignore_ascii_case("application/inkml+xml"))
    };
    let ink_targets = by_id.values().any(&is_ink);
    ids.retain(|id| matches!(by_id.resolve(id.as_str()), Resolution::One(rel) if is_ink(rel)));
    ink_targets
}

/// `NOTE-PR-LISTING` (ECMA-376 Part 1 §17.11.9) and its reverse,
/// Join special-note definitions and settings listings within each document owner's ID space.
/// Each direction requires its absence-producing source to be complete. Unavailable or capped
/// sources produce explicit INCOMPLETE coverage while independently established findings survive.
/// Unlisted special notes remain Errors under the existing union-of-consumers calibration:
/// Word for the web loads them, but Word for Mac wedged on the same shape (batch Mac-2).
fn check_note_pr(pkg: &Package, xml: &XmlScan, acc: &mut Acc) {
    observation_contract! {
        rules: "NOTE-PR-DANGLING,NOTE-PR-LISTING";
        collections: "field.endnote_defs*1,field.footnote_defs*1,field.note_listings*1,field.special_notes*1";
        scope: "each established owner and its selected supporting sources; shared parts in every owner; unowned references excluded; explicit no-root fallback only";
        loss: "source unavailable or malformed; XML event/depth budget; category allocation cap; unknown or ambiguous source discovery; source-local pooled-budget loss";
        fallback: "suppress only dependent absence directions and report Incomplete; independent owner and prefix findings survive";
        package_loss: "independent";
    }

    let note_listings = xml.note_listings();
    let special_notes = xml.special_notes();

    acc.ran("NOTE-PR-LISTING");
    acc.ran("NOTE-PR-DANGLING");
    let owner_map = &xml.graph;
    let roles = story_roles(pkg);
    for (index, owner) in owner_scopes(pkg).iter().enumerate() {
        let index = OwnerId(index);
        let selected = [
            supporting_part(pkg, owner.root(), "settings", package::SETTINGS_CONTENT_TYPE, acc),
            supporting_part(pkg, owner.root(), "footnotes", package::FOOTNOTES_CONTENT_TYPE, acc),
            supporting_part(pkg, owner.root(), "endnotes", package::ENDNOTES_CONTENT_TYPE, acc),
        ];
        let discovered = !discovery_unavailable(pkg, owner.root());
        let complete =
            |source: Option<&package::Entry>| source.is_none_or(|entry| xml_source_complete(xml, entry) && xml.notes_complete(&entry.name));
        let (forward_ok, reverse_ok) = if owner.root().is_some() {
            // Each family's listing is established on its own: an unresolved `endnote` in settings says nothing about
            // the footnote listing.
            let forward_of = |end: bool| {
                discovered
                    && matches!(selected[0], Resolution::One(_) | Resolution::Absent)
                    && match selected[0] {
                        Resolution::One(entry) => xml_source_complete(xml, entry) && xml.listing_complete(&entry.name, end),
                        Resolution::Absent => true,
                        Resolution::Ambiguous(_) | Resolution::Unknown(_) => false,
                    }
            };
            let forward = [forward_of(false), forward_of(true)];
            (
                forward,
                [
                    forward[0]
                        && matches!(selected[1], Resolution::One(_) | Resolution::Absent)
                        && match selected[1] {
                            Resolution::One(entry) => complete(Some(entry)) && xml.category_complete(&entry.name, |f| &f.footnote_defs),
                            Resolution::Absent => true,
                            Resolution::Ambiguous(_) | Resolution::Unknown(_) => false,
                        },
                    forward[1]
                        && matches!(selected[2], Resolution::One(_) | Resolution::Absent)
                        && match selected[2] {
                            Resolution::One(entry) => complete(Some(entry)) && xml.category_complete(&entry.name, |f| &f.endnote_defs),
                            Resolution::Absent => true,
                            Resolution::Ambiguous(_) | Resolution::Unknown(_) => false,
                        },
                ],
            )
        } else {
            let discovered = discovered && !descendant_discovery_incomplete(pkg, owner_map, index);
            let forward = discovered && note_sources_complete(pkg, xml, owner_map, &roles, index, |r| r.is_settings());
            (
                [forward; 2],
                [
                    forward && note_sources_complete(pkg, xml, owner_map, &roles, index, |r| r.is_footnotes()),
                    forward && note_sources_complete(pkg, xml, owner_map, &roles, index, |r| r.is_endnotes()),
                ],
            )
        };
        if !(forward_ok[0] && forward_ok[1]) || reverse_ok.contains(&false) {
            acc.note(format!("the footnotePr/endnotePr listings for {} could not be fully joined (ownership, declarations, or source facts were not fully examined)", owner.root().unwrap_or("the package")));
            acc.set_incomplete();
        }
        check_owner_note_pr(
            xml,
            owner_map,
            index,
            owner.root().is_some().then_some(selected.map(|source| match source {
                Resolution::One(entry) => Some(entry),
                Resolution::Absent => None,
                Resolution::Ambiguous(_) | Resolution::Unknown(_) => None,
            })),
            [
                forward_ok[0] && matches!(selected[1], Resolution::One(_) | Resolution::Absent),
                forward_ok[1] && matches!(selected[2], Resolution::One(_) | Resolution::Absent),
            ],
            reverse_ok,
            acc,
        );
    }
    if owner_scopes(pkg) == [OwnerContext::Fallback]
        && (special_notes.iter().any(|(part, ..)| part_owners(owner_map, part).is_empty())
            || note_listings.0.iter().chain(&note_listings.1).any(|(_, part)| part_owners(owner_map, part).is_empty()))
    {
        acc.note("the footnotePr/endnotePr join skipped facts whose document ownership could not be established".to_string());
        acc.set_incomplete();
    }
}

/// Whether every relevant settings or notes source for an owner is available and uncapped.
/// Used only by the no-owner fallback pool; established owners check their selected sources.
fn note_sources_complete(
    pkg: &Package,
    xml: &XmlScan,
    owner_map: &PartGraph,
    roles: &HashMap<String, scan::PartRole>,
    owner: OwnerId,
    relevant: fn(&scan::PartRole) -> bool,
) -> bool {
    observation_contract! {
        rules: "-";
        collections: "field.endnote_defs*1,field.entries*1,field.footnote_defs*1,type.HashMap*1";
        scope: "each established owner and its selected supporting sources; shared parts in every owner; unowned references excluded; explicit no-root fallback only";
        loss: "source unavailable or malformed; XML event/depth budget; category allocation cap; unknown or ambiguous source discovery; source-local pooled-budget loss";
        fallback: "suppress only dependent absence directions and report Incomplete; independent owner and prefix findings survive";
        package_loss: "independent";
    }

    pkg.entries.iter().filter(|e| !e.is_dir).all(|e| {
        let key = e.name.to_ascii_lowercase();
        !roles.get(&key).is_some_and(relevant)
            || (!unresolved_fallback_contributor(owner_map, owner, &e.name)
                && (!part_owners(owner_map, &e.name).contains(&owner)
                    || (xml_source_complete(xml, e)
                        && xml.notes_complete(&e.name)
                        && (!roles.get(&key).is_some_and(|r| r.is_footnotes()) || xml.category_complete(&e.name, |f| &f.footnote_defs))
                        && (!roles.get(&key).is_some_and(|r| r.is_endnotes()) || xml.category_complete(&e.name, |f| &f.endnote_defs)))))
    })
}

/// Apply both special-note join directions to retained facts from one established owner.
fn check_owner_note_pr(
    xml: &XmlScan,
    owner_map: &PartGraph,
    owner: OwnerId,
    selected: Option<[Option<&package::Entry>; 3]>,
    forward_ok: [bool; 2],
    reverse_ok: [bool; 2],
    acc: &mut Acc,
) {
    observation_contract! {
        rules: "NOTE-PR-DANGLING,NOTE-PR-LISTING";
        collections: "access.into_complete_set*2,access.missing*2,access.prefix*2,field.endnote_defs*1,field.footnote_defs*1,field.note_listings*1,field.special_notes*1,type.HashSet*3,type.Observed*3";
        scope: "each established owner and its selected supporting sources; shared parts in every owner; unowned references excluded; explicit no-root fallback only";
        loss: "source unavailable or malformed; XML event/depth budget; category allocation cap; unknown or ambiguous source discovery; source-local pooled-budget loss";
        fallback: "suppress only dependent absence directions and report Incomplete; independent owner and prefix findings survive";
        package_loss: "NOTE-PR-LISTING,NOTE-PR-DANGLING";
    }

    let note_listings = xml.note_listings();
    let special_notes = xml.special_notes();
    let note_facts = xml.package_facts();

    let owned = |part: &str, role: usize| match selected {
        Some(sources) => sources[role].is_some_and(|entry| entry.name.eq_ignore_ascii_case(part)),
        None => part_owners(owner_map, part).contains(&owner),
    };
    let listed_f: HashSet<DecimalId> = note_listings.0.iter().filter(|(_, part)| owned(part, 0)).map(|(id, _)| id.clone()).collect();
    let listed_e: HashSet<DecimalId> = note_listings.1.iter().filter(|(_, part)| owned(part, 0)).map(|(id, _)| id.clone()).collect();
    let listing_set = |items, complete| {
        Observed::with_status(items, if complete { Completeness::Complete } else { Completeness::Incomplete("selected note listings unavailable") })
            .into_complete_set()
    };
    let listed_f = listing_set(listed_f, forward_ok[0]);
    let listed_e = listing_set(listed_e, forward_ok[1]);
    for (part, id, is_end, t) in
        special_notes.iter().filter(|(part, _, end, _)| forward_ok[usize::from(*end)] && owned(part, if *end { 2 } else { 1 }))
    {
        let listed = if *is_end { &listed_e } else { &listed_f };
        if listed.as_ref().is_some_and(|set| set.missing(id)) {
            let (note, pr) = if *is_end { ("endnote", "endnotePr") } else { ("footnote", "footnotePr") };
            acc.add_graded(
                "NOTE-PR-LISTING", Severity::Error, Evidence::Observed, Some(part.clone()),
                format!("the {note} with id {id} in {part} has type {t} but is not listed in its owner's {pr}; ECMA-376 says such a note shall not be loaded"),
            );
        }
    }
    for (role, (complete, (note, listings, defs))) in reverse_ok
        .into_iter()
        .zip([("footnote", &note_listings.0, &note_facts.footnote_defs.prefix()), ("endnote", &note_listings.1, &note_facts.endnote_defs.prefix())])
        .enumerate()
    {
        if complete {
            let defined = Observed::complete(defs.iter().filter(|(_, part)| owned(part, role + 1)).map(|(id, _)| id.clone()).collect::<HashSet<_>>())
                .into_complete_set();
            for (id, _) in listings.iter().filter(|(_, part)| owned(part, 0)) {
                if defined.as_ref().is_some_and(|set| set.missing(id)) {
                    acc.add(
                        "NOTE-PR-DANGLING",
                        None,
                        format!("a {note}Pr lists {note} id {id}, but no {note} with that id is defined for its owner"),
                    );
                }
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
fn check_content_part_types(pkg: &Package, content_parts: &[(String, Vec<RelId>)], acc: &mut Acc) {
    observation_contract! {
        rules: "CONTENTPART-TYPE";
        collections: "field.content_types*1,field.rels*1,type.HashSet*2,type.Vec*1";
        scope: "source-local bounded extraction; no implicit document owner";
        loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
        fallback: "retain proven prefix presence and duplicates; return loss to the caller; no negative membership from an incomplete set";
        package_loss: "independent";
    }

    acc.ran("CONTENTPART-TYPE");
    for (owner, ids) in content_parts {
        let rels_name = package::rels_part_name(owner);
        let Some(rp) = pkg.rels.iter().find(|rp| rp.name.eq_ignore_ascii_case(&rels_name)) else { continue };
        let by_id = package::relationship_bindings(rp);
        let mut seen: std::collections::HashSet<&RelId> = std::collections::HashSet::new();
        for id in ids {
            if !seen.insert(id) {
                continue;
            }
            let Resolution::One(rel) = by_id.resolve(id.as_str()) else { continue };
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
    observation_contract! {
        rules: "XML-MALFORMED";
        collections: "field.reachable*1,type.Observed*1";
        scope: "source-local bounded extraction; no implicit document owner";
        loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
        fallback: "retain proven prefix presence and duplicates; return loss to the caller; no negative membership from an incomplete set";
        package_loss: "independent";
    }

    let lower = name.to_ascii_lowercase();
    let is_main = (match &pkg.main {
        Resolution::One(name) => Some(name.as_str()),
        Resolution::Absent | Resolution::Ambiguous(_) | Resolution::Unknown(_) => None,
    })
    .is_some_and(|m| m.eq_ignore_ascii_case(name));
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
    observation_contract! {
        rules: "-";
        collections: "field.content_types*1,field.entries*1,field.rels*2,lookup.canonical_entry*1,type.HashMap*2";
        scope: "source-local bounded extraction; no implicit document owner";
        loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
        fallback: "retain proven prefix presence and duplicates; return loss to the caller; no negative membership from an incomplete set";
        package_loss: "independent";
    }

    let mut roles = HashMap::new();
    if let Resolution::One(main) = &pkg.main {
        roles.insert(main.to_ascii_lowercase(), scan::PartRole::Main);
    }
    for e in pkg.entries.iter().filter(|e| !e.is_dir) {
        if let Some(role) = content_type_role(pkg.content_types.of(&e.name)) {
            roles.entry(e.name.to_ascii_lowercase()).and_modify(|existing| *existing = existing.merge(role)).or_insert(role);
        }
    }
    for rp in &pkg.rels {
        for r in &rp.rels {
            let Some(role) = relationship_role(r.role.kind()) else { continue };
            let Some(target) = &r.target_abs else { continue };
            if let Resolution::One(name) = package::canonical_entry(pkg, target) {
                roles.entry(name.to_ascii_lowercase()).and_modify(|existing| *existing = existing.merge(role)).or_insert(role);
            }
        }
    }
    roles
}

/// Classify a part role from its declared content type.
fn content_type_role(ct: Option<&str>) -> Option<scan::PartRole> {
    let role = match ct? {
        package::COMMENTS_CONTENT_TYPE => scan::PartRole::Comments,
        package::FOOTNOTES_CONTENT_TYPE => scan::PartRole::Footnotes,
        package::ENDNOTES_CONTENT_TYPE => scan::PartRole::Endnotes,
        package::HEADER_CONTENT_TYPE | package::FOOTER_CONTENT_TYPE => scan::PartRole::Story,
        package::GLOSSARY_CONTENT_TYPE => scan::PartRole::Glossary,
        package::SETTINGS_CONTENT_TYPE => scan::PartRole::Settings,
        _ => return None,
    };
    Some(role)
}

/// Classify a part role from the relationship type that targets it.
fn relationship_role(kind: Option<&str>) -> Option<scan::PartRole> {
    Some(match kind? {
        "settings" => scan::PartRole::Settings,
        "glossaryDocument" => scan::PartRole::Glossary,
        "comments" => scan::PartRole::Comments,
        "footnotes" => scan::PartRole::Footnotes,
        "endnotes" => scan::PartRole::Endnotes,
        "header" | "footer" => scan::PartRole::Story,
        _ => return None,
    })
}

/// Verify relationships: duplicate IDs, dangling targets, typed targets, and content type matches.
fn check_rels(pkg: &Package, xml: &XmlScan, acc: &mut Acc) {
    observation_contract! {
        rules: "REL-DANGLING,REL-DUP-ID,REL-REF-MISSING,REL-TYPED";
        collections: "field.references*1,field.rels*3,lookup.canonical_entry*1";
        scope: "source-local bounded extraction; no implicit document owner";
        loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
        fallback: "retain proven prefix presence and duplicates; return loss to the caller; no negative membership from an incomplete set";
        package_loss: "REL-DANGLING";
    }

    let refs = xml.references();
    acc.ran("REL-DUP-ID");
    for rp in &pkg.rels {
        report_duplicate_rel_ids(rp, acc);
    }
    acc.ran("REL-DANGLING");
    for rp in &pkg.rels {
        for r in &rp.rels {
            let Some(target) = &r.target_abs else { continue };
            // An unexamined archive suffix may still contain this target.
            if match package::canonical_entry(pkg, target) {
                Resolution::One(_) => false,
                Resolution::Absent => true,
                Resolution::Unknown(_) => false,
                Resolution::Ambiguous(_) => false,
            } {
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
    check_typed_image_refs(pkg, &refs, acc);
    if check_part_refs(pkg, xml, acc).ran() {
        acc.ran("REL-REF-MISSING");
    }
    check_typed_content(pkg, xml, acc);
}

/// Find duplicate relationship IDs within a single `.rels` part and report `REL-DUP-ID` findings.
fn report_duplicate_rel_ids(rp: &RelsPart, acc: &mut Acc) {
    observation_contract! {
        rules: "REL-DUP-ID";
        collections: "field.rels*1,type.HashMap*2,type.Vec*1";
        scope: "source-local bounded extraction; no implicit document owner";
        loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
        fallback: "retain proven prefix presence and duplicates; return loss to the caller; no negative membership from an incomplete set";
        package_loss: "independent";
    }

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
fn check_part_refs(pkg: &Package, xml: &XmlScan, acc: &mut Acc) -> JoinCompletion {
    observation_contract! {
        rules: "REL-REF-MISSING";
        collections: "access.into_complete_set*1,access.is_complete*1,access.missing*1,access.prefix*1,field.collapsed_names*2,field.parts*1,field.references*2,field.rels*1,type.HashSet*2,type.Observed*1";
        scope: "source-local bounded extraction; no implicit document owner";
        loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
        fallback: "retain proven prefix presence and duplicates; return loss to the caller; no negative membership from an incomplete set";
        package_loss: "REL-REF-MISSING";
    }

    let mut completion = JoinCompletion::vacuous();
    // An unreadable selected document is absent from xml.parts, but still owes a reference
    // observation. Seed its skipped scope; independently completed parts retain their credit.
    completion.skipped = owner_scopes(pkg).iter().any(|owner| owner_document_unreadable(pkg, owner));
    // Empty reference sets are obligations too: they cannot launder a skipped active part.
    for facts in xml.parts.values() {
        let part = &facts.name;
        // The part's own `.rels` could not be read: which relationships it declares is unknown, so
        // its `r:id`/`r:embed`/`r:link` values are not established as unresolved.
        if pkg.collapsed_names.contains(part)
            || pkg.collapsed_names.iter().any(|name| name.eq_ignore_ascii_case(&rels_part_name(part)))
            || package::relationships_incomplete(pkg, part)
            || !facts.references.is_complete()
        {
            completion.skipped = true;
            continue;
        }
        let Some(ids) = Observed::complete(part_rel_ids(pkg, part)).into_complete_set() else {
            completion.skipped = true;
            continue;
        };
        let values = facts.references.prefix();
        completion.active |= !values.is_empty() || pkg.rels.iter().any(|rp| rp.name.eq_ignore_ascii_case(&rels_part_name(part)));
        let mut reported: HashSet<&str> = HashSet::new();
        for r in values {
            if ids.missing(&r.value) && reported.insert(r.value.as_str()) {
                acc.add(
                    "REL-REF-MISSING",
                    Some(part.clone()),
                    format!("the r:{} value {} in {part} does not resolve to a relationship in that part's .rels", r.attr, r.value),
                );
            }
        }
    }
    completion
}

/// An image element (`a:blip`, `v:imagedata`) must resolve to an image relationship whose target is
/// an image part; anything else is `REL-TYPED` at the referencing part. The check keeps the element
/// identity, so it applies to every owner (including a header's own image references, resolved
/// against the header's `.rels`).
fn check_typed_image_refs(pkg: &Package, refs: &[(String, Vec<RefFact>)], acc: &mut Acc) {
    observation_contract! {
        rules: "REL-TYPED";
        collections: "field.content_types*2,field.incomplete*1,field.rels*1,lookup.content_unavailable*1,type.HashSet*2,type.Vec*1";
        scope: "source-local bounded extraction; no implicit document owner";
        loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
        fallback: "retain proven prefix presence and duplicates; return loss to the caller; no negative membership from an incomplete set";
        package_loss: "REL-TYPED";
    }

    for (part, values) in refs {
        let Some(rp) = pkg.rels.iter().find(|r| r.name.eq_ignore_ascii_case(&rels_part_name(part))) else { continue };
        let by_id = package::relationship_bindings(rp);
        let mut reported: HashSet<(&str, &str)> = HashSet::new();
        for r in values.iter().filter(|r| r.image) {
            if !reported.insert((r.attr.as_str(), r.value.as_str())) {
                continue;
            }
            let Resolution::One(rel) = by_id.resolve(r.value.as_str()) else { continue };
            if rel.role.kind() != Some("image") {
                acc.add_proven(
                    "REL-TYPED",
                    Some(part.clone()),
                    format!(
                        "an image element in {part} references relationship {}, which is a {} relationship, not an image relationship",
                        r.value, rel.kind
                    ),
                );
            } else if let Some(target) = &rel.target_abs {
                // The relationship resolves and targets an internal part. Only the next conclusion
                // depends on the manifest: when its content could not be read the target's declared
                // type is unknown, not absent, so it is skipped. (The non-image-relationship and
                // external-relationship conclusions above and below do not depend on the manifest.)
                if !(match package::content_unavailable(pkg, "[Content_Types].xml") {
                    Resolution::One(unavailable) => unavailable,
                    Resolution::Absent => false,
                    Resolution::Unknown(_) => true,
                    Resolution::Ambiguous(_) => true,
                }) {
                    let ct = pkg.content_types.of(target);
                    if (!pkg.content_types.incomplete || ct.is_some()) && !ct.is_some_and(|c| c.starts_with("image/")) {
                        let msg = format!(
                            "an image element in {part} references relationship {}, whose target {target} has content type {}",
                            r.value,
                            ct.unwrap_or("(none)")
                        );
                        if ct.is_some() {
                            acc.add_proven("REL-TYPED", Some(part.clone()), msg);
                        } else {
                            acc.add("REL-TYPED", Some(part.clone()), msg);
                        }
                    }
                }
            } else if r.attr != "link" {
                // `r:embed` (and `v:imagedata`'s `r:id`) name an embedded image and must resolve to
                // a part inside the package; only `r:link` may legitimately point at an external
                // resource (H4).
                acc.add_proven(
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
fn part_rel_ids(pkg: &Package, part: &str) -> HashSet<RelId> {
    observation_contract! {
        rules: "-";
        collections: "field.rels*2,type.HashSet*2";
        scope: "source-local bounded extraction; no implicit document owner";
        loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
        fallback: "retain proven prefix presence and duplicates; return loss to the caller; no negative membership from an incomplete set";
        package_loss: "independent";
    }

    let want = rels_part_name(part);
    match pkg.rels.iter().find(|r| r.name.eq_ignore_ascii_case(&want)) {
        Some(rp) => rp.rels.iter().map(|r| r.id.clone()).collect(),
        None => HashSet::new(),
    }
}

/// Verify that typed relationships in the main document part target parts with matching content types.
fn check_typed_targets(pkg: &Package, acc: &mut Acc) {
    observation_contract! {
        rules: "REL-TYPED";
        collections: "field.content_types*2,field.incomplete*1,field.rels*2,lookup.content_unavailable*1";
        scope: "source-local bounded extraction; no implicit document owner";
        loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
        fallback: "retain proven prefix presence and duplicates; return loss to the caller; no negative membership from an incomplete set";
        package_loss: "REL-TYPED";
    }

    // Every conclusion here compares a target's declared content type; when the manifest's content
    // could not be read, a target's type is UNKNOWN, not absent, so no type conclusion is
    // established. (The run is already INCOMPLETE from loading the manifest.)
    if match package::content_unavailable(pkg, "[Content_Types].xml") {
        Resolution::One(unavailable) => unavailable,
        Resolution::Absent => false,
        Resolution::Unknown(_) => true,
        Resolution::Ambiguous(_) => true,
    } {
        return;
    }
    let Resolution::One(main) = &pkg.main else { return };
    let want = rels_part_name(main);
    let Some(rp) = pkg.rels.iter().find(|r| r.name.eq_ignore_ascii_case(&want)) else {
        acc.note(format!("the main part {main} has no relationships part; typed targets were not checked"));
        return;
    };
    for r in &rp.rels {
        let Some(target) = &r.target_abs else { continue };
        let ct = pkg.content_types.of(target);
        if ct.is_none() && pkg.content_types.incomplete {
            continue;
        }
        let (matched, message) = match r.role.kind() {
            Some("header") if ct != Some(package::HEADER_CONTENT_TYPE) => (true, header_footer_message("header", &r.id, target, ct)),
            Some("footer") if ct != Some(package::FOOTER_CONTENT_TYPE) => (true, header_footer_message("footer", &r.id, target, ct)),
            Some("image") if !ct.is_some_and(|c| c.starts_with("image/")) => {
                (true, format!("image relationship {} targets {target}, whose content type is {}", r.id, ct.unwrap_or("(none)")))
            }
            _ => (false, String::new()),
        };
        if matched {
            if ct.is_some() {
                acc.add_proven("REL-TYPED", Some(rp.name.clone()), message);
            } else {
                acc.add("REL-TYPED", Some(rp.name.clone()), message);
            }
        }
    }
}

/// Format an error message for mismatched header or footer relationship targets.
fn header_footer_message(kind: &str, id: &str, target: &str, ct: Option<&str>) -> String {
    format!("{kind} relationship {id} targets {target}, whose content type is {}", ct.unwrap_or("(none)"))
}

/// The content type each fixed-type relationship requires of its internal target, keyed by the
/// official relationship's namespace-checked local kind (see [`package::relationship_kind`]). This is the
/// ECMA-376 Part 1 part catalogue (§11.3 and §15.2, plus the MS-DOCX extension parts), so
/// CT-MISMATCH is a table lookup rather than a
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
/// `w:contentPart` element or a native `w14:contentPart` reaching InkML (see [`check_typed_content`]).
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

/// The part that owns a relationships part, derived from the rels
/// part's name (`word/_rels/document.xml.rels` becomes
/// `word/document.xml`, lowercased). `None` for the package
/// relationships part (`_rels/.rels`), which has no containing part.
fn rels_owner_name(rels_name: &str) -> Option<String> {
    let lower = rels_name.to_ascii_lowercase();
    let (dir, file) = match lower.rsplit_once("/_rels/") {
        Some((d, f)) => (d, f),
        None => ("", lower.strip_prefix("_rels/")?),
    };
    let owner_file = file.strip_suffix(".rels")?;
    if owner_file.is_empty() {
        return None;
    }
    Some(if dir.is_empty() { owner_file.to_string() } else { format!("{dir}/{owner_file}") })
}

/// Per-part consumption evidence; no caller reconstructs completeness with a parallel ledger.
fn content_consumption(pkg: &Package, xml: &XmlScan, rp: &RelsPart) -> Observed<HashSet<RelId>> {
    observation_contract! {
        rules: "-";
        collections: "access.invalidate*1,access.is_complete*1,access.prefix*1,access.project*1,field.content*1,field.content_types*1,field.exemptions*2,field.parts*1,lookup.entry_by_name*1,type.HashSet*3,type.Observed*3";
        scope: "source-local bounded extraction; no implicit document owner";
        loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
        fallback: "retain proven prefix presence and duplicates; return loss to the caller; no negative membership from an incomplete set";
        package_loss: "independent";
    }

    let Some(owner) = rels_owner_name(&rp.name) else { return Observed::complete(HashSet::new()) };
    if let Some(part) = xml.parts.get(&PartId::new(&owner)) {
        let mut observed = part.content.project(|ids| ids.iter().chain(part.exemptions.prefix()).cloned().collect());
        if !part.exemptions.is_complete() {
            observed.invalidate("contentPart exemptions incomplete");
        }
        return observed;
    }
    let unknown = match entry_by_name(pkg, &owner) {
        Resolution::One(_) => package::is_xml_part(pkg, &owner) || pkg.content_types.of(&owner).is_none(),
        Resolution::Absent => package::is_xml_part(pkg, &owner),
        Resolution::Unknown(_) => true,
        Resolution::Ambiguous(_) => true,
    };
    Observed::with_status(
        HashSet::new(),
        if unknown { Completeness::Incomplete("XML consumption source unavailable") } else { Completeness::Complete },
    )
}

/// Verify that fixed-type relationships in the package match their prescribed part content types.
///
/// One row is consumption-sensitive (#5982): the `customXml` row prescribes
/// `application/xml`, but a relationship of that type consumed by a
/// `w:contentPart` element is governed by CONTENTPART-TYPE instead —
/// ECMA-376 §17.3.3.2's own contentPart example reaches a `text/xml` part
/// through exactly this relationship type, and Word for the web opens
/// such a file cleanly. The exemption tracks consumption per owner: it
/// applies to the relationship CONTENTPART-TYPE itself resolves for the
/// id (the first with that id in the owner's `.rels`, matching its
/// first-wins index), and for owners whose consumption collection is
/// incomplete (budget-capped or unparseable) the row is not applied at
/// all — non-consumption is not established there, and the run is
/// already INCOMPLETE. Owners with complete collections are checked
/// normally. Skipped contentPart references exempt their first relationship only when
/// its target is text/xml, and remain separate so CONTENTPART-TYPE never validates skipped content. Native
/// w14:contentPart references to InkML join only this exemption collection (MS-ODRAWXML §2.1.4).
fn check_typed_content(pkg: &Package, xml: &XmlScan, acc: &mut Acc) {
    observation_contract! {
        rules: "CT-MISMATCH";
        collections: "access.into_complete_set*1,access.missing*1,field.content_types*1,field.rels*2,lookup.canonical_entry*1,type.HashSet*2,type.Observed*1";
        scope: "per owning XML part consumption; first-bound contentPart relationship only; skipped text/xml and native InkML exemptions separate";
        loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
        fallback: "suppress customXml pairing when consumption unavailable; keep other relationships and independent contentPart presence/type findings";
        package_loss: "independent";
    }

    acc.ran("CT-MISMATCH");
    let main_rels = (match &pkg.main {
        Resolution::One(name) => Some(name.as_str()),
        Resolution::Absent | Resolution::Ambiguous(_) | Resolution::Unknown(_) => None,
    })
    .map(rels_part_name);
    let non_main_owner_rels: HashSet<String> = owner_scopes(pkg)
        .iter()
        .filter_map(|owner| owner.root())
        .filter(|owner| !main_rels.as_deref().is_some_and(|m| m.eq_ignore_ascii_case(&rels_part_name(owner))))
        .map(|owner| rels_part_name(owner).to_ascii_lowercase())
        .collect();
    let mut skipped_consumption = HashSet::new();
    for rp in &pkg.rels {
        let is_non_main_owner = non_main_owner_rels.contains(&rp.name.to_ascii_lowercase());
        let rp_key = rp.name.to_ascii_lowercase();
        let consumed = content_consumption(pkg, xml, rp).into_complete_set();
        let bindings = package::relationship_bindings(rp);
        for r in &rp.rels {
            let first_with_id = bindings.is_first(r);
            let Some(target) = &r.target_abs else { continue };
            let Some(kind) = r.role.kind() else { continue };
            let Some(want) = fixed_rel_content_type(kind) else { continue };
            if matches!(kind, "header" | "footer") && !is_non_main_owner {
                continue;
            }
            if kind == "customXml" {
                if consumed.is_none() {
                    // Explain only an actual skipped pairing; unrelated malformed parts have no
                    // consumption check to lose and must retain their existing coverage behavior.
                    if skipped_consumption.insert(rp_key.clone()) {
                        let source = rels_owner_name(&rp.name).unwrap_or_else(|| rp.name.clone());
                        acc.note(format!("{source} was not fully examined as XML; contentPart pairing absence was not checked"));
                        acc.set_incomplete();
                    }
                    continue;
                }
                if first_with_id && consumed.as_ref().is_some_and(|ids| !ids.missing(&r.id)) {
                    continue;
                }
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
            let part = match canonical_entry(pkg, target) {
                Resolution::One(name) => name,
                Resolution::Absent => target.clone(),
                Resolution::Unknown(_) => target.clone(),
                Resolution::Ambiguous(_) => target.clone(),
            };
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
fn check_story(pkg: &Package, xml: &XmlScan, acc: &mut Acc) {
    observation_contract! {
        rules: "BOOKMARK-COUNT,BOOKMARK-UNPAIRED,COMMENT-RANGE-COUNT,COMMENT-RANGE-UNPAIRED,FLD-UNBALANCED,ID-BOOKMARK-DUP,ID-COMMENT-DUP,ID-COMMENT-REF,ID-DOCPR-DUP,ID-REVISION-DUP,MOVE-RANGE-COUNT,NOTE-DUP,PARAID-DUP,PARAID-VALUE,PERM-COUNT,REF-HEADER,REF-NOTE,REF-STYLE,STRUCT-DELTEXT,STRUCT-MOVE-UNPAIRED,STRUCT-TC-NO-P";
        collections: "access.prefix*6,field.bookmark_starts*2,field.bookmarks*1,field.comment_defs*2,field.comment_refs*1,field.docpr*1,field.endnote_defs*2,field.endnote_refs*1,field.footnote_defs*2,field.footnote_refs*1,field.revisions*1";
        scope: "source-local bounded extraction; no implicit document owner";
        loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
        fallback: "retain proven prefix presence and duplicates; return loss to the caller; no negative membership from an incomplete set";
        package_loss: "independent";
    }

    let facts = &xml.package_facts();
    let compat = xml.compat;
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
    report_definition_dups(acc, "ID-COMMENT-DUP", facts.comment_defs.prefix(), "w:comment id");
    if check_owned_ids(pkg, xml, "comments", package::COMMENTS_CONTENT_TYPE, |f| &f.comment_defs, |f| &f.comment_refs, acc).ran() {
        acc.ran("ID-COMMENT-REF");
    }
    report_unreferenced_comments(pkg, xml, acc);
    acc.ran("ID-DOCPR-DUP");
    report_dups(acc, "ID-DOCPR-DUP", facts.docpr.prefix(), "wp:docPr id");
    acc.ran("ID-BOOKMARK-DUP");
    report_dups(acc, "ID-BOOKMARK-DUP", facts.bookmarks.prefix(), "w:bookmarkStart id");
    // BOOKMARK-UNPAIRED findings are produced by the streaming scan; the rule is registered here.
    acc.ran("BOOKMARK-UNPAIRED");
    // COMMENT-RANGE-UNPAIRED likewise.
    acc.ran("COMMENT-RANGE-UNPAIRED");
    // PARAID-DUP and PARAID-VALUE findings are produced by the streaming scan; registered here.
    acc.ran("PARAID-DUP");
    acc.ran("PARAID-VALUE");
    acc.ran("ID-REVISION-DUP");
    report_dups(acc, "ID-REVISION-DUP", facts.revisions.prefix(), "revision id");
    acc.ran("NOTE-DUP");
    report_definition_dups(acc, "NOTE-DUP", facts.footnote_defs.prefix(), "w:footnote id");
    report_definition_dups(acc, "NOTE-DUP", facts.endnote_defs.prefix(), "w:endnote id");
    let footnotes = check_owned_ids(pkg, xml, "footnotes", package::FOOTNOTES_CONTENT_TYPE, |f| &f.footnote_defs, |f| &f.footnote_refs, acc);
    let endnotes = check_owned_ids(pkg, xml, "endnotes", package::ENDNOTES_CONTENT_TYPE, |f| &f.endnote_defs, |f| &f.endnote_refs, acc);
    if footnotes.merge(endnotes).ran() {
        acc.ran("REF-NOTE");
    }
    acc.ran("REF-HEADER");
    check_header_footer_refs(pkg, facts, acc);
    acc.ran("REF-STYLE");
    check_style_refs(pkg, xml, acc);
    check_vocabulary(pkg, acc);
    check_numbering(pkg, xml, acc);
    check_commentex(pkg, xml, acc);
    acc.ran("FLD-UNBALANCED");
    acc.ran("STRUCT-TC-NO-P");
    acc.ran("STRUCT-DELTEXT");
    acc.ran("STRUCT-MOVE-UNPAIRED");
    check_custom_xml(pkg, xml, acc);
    report_unowned_reference_scope(xml, acc);
}

/// One bounded count explains established exclusions without treating non-ownership as loss.
fn report_unowned_reference_scope(xml: &XmlScan, acc: &mut Acc) {
    observation_contract! {
        rules: "-";
        collections: "access.prefix*5,field.comment_refs*1,field.endnote_refs*1,field.footnote_refs*1,field.num_ids*1,field.parts*1,field.style_refs*1";
        scope: "established-unowned reference exclusions from owner-local note/comment/style/numbering joins; out of scope";
        loss: "unknown ownership is not an exclusion; only retained positive references are counted";
        fallback: "one bounded coverage count; established non-ownership never marks Incomplete";
        package_loss: "independent";
    }

    let excluded = xml
        .parts
        .values()
        .filter(|part| {
            xml.graph.owners(&part.name).is_empty()
                && !xml.graph.unknown(&part.name)
                && (!part.facts.footnote_refs.prefix().is_empty()
                    || !part.facts.endnote_refs.prefix().is_empty()
                    || !part.facts.comment_refs.prefix().is_empty()
                    || !part.facts.style_refs.prefix().is_empty()
                    || !part.facts.num_ids.prefix().is_empty())
        })
        .count();
    if excluded > 0 {
        acc.note(format!(
            "{excluded} established-unowned parts contain references excluded from owner-local note/comment/style/numbering joins (out of scope)"
        ));
    }
}

/// Duplicate decimal IDs are forbidden within a definition part, not between independent owners.
fn report_definition_dups(acc: &mut Acc, rule: &'static str, items: &[(DecimalId, String)], what: &str) {
    observation_contract! {
        rules: "ID-COMMENT-DUP,NOTE-DUP";
        collections: "type.HashSet*2";
        scope: "source-local bounded extraction; no implicit document owner";
        loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
        fallback: "retain proven prefix presence and duplicates; return loss to the caller; no negative membership from an incomplete set";
        package_loss: "independent";
    }

    let mut seen = HashSet::new();
    let mut reported = HashSet::new();
    for (id, part) in items {
        if !seen.insert((part, id)) && reported.insert((part, id)) {
            acc.add(rule, Some(part.clone()), format!("duplicate {what} {id} in {part}"));
        }
    }
}

/// A completed active scope counts as a run even when another scope was skipped. Vacuous
/// scopes count only if none were skipped; in particular empty endnotes cannot launder footnote loss.
struct JoinCompletion {
    active: bool,
    skipped: bool,
}
impl JoinCompletion {
    fn vacuous() -> Self {
        Self { active: false, skipped: false }
    }
    fn merge(self, other: Self) -> Self {
        Self { active: self.active || other.active, skipped: self.skipped || other.skipped }
    }
    fn ran(self) -> bool {
        self.active || !self.skipped
    }
}

/// Only a selected document root carries this owner-wide skipped reference obligation.
/// Never-loaded supporting stories retain amendment 4's independent-category policy.
fn owner_document_unreadable(pkg: &Package, owner: &OwnerContext) -> bool {
    observation_contract! {
        rules: "-";
        collections: "lookup.entry_by_name*1";
        scope: "the selected main or glossary document root for one owner";
        loss: "present document root excluded from scanning by unreadable bytes or text";
        fallback: "mark its reference joins skipped; supporting parts retain their existing policy";
        package_loss: "independent";
    }

    let Some(root) = owner.root() else { return false };
    match package::entry_by_name(pkg, root) {
        Resolution::One(entry) => entry.content_unavailable,
        Resolution::Absent => false,
        Resolution::Unknown(_) => false,
        Resolution::Ambiguous(_) => false,
    }
}

/// Join each owner's references only to the definition part reached by its own relationships.
/// Unavailable or truncated discovery/definition sources cannot establish missing IDs.
fn check_owned_ids(
    pkg: &Package,
    xml: &XmlScan,
    kind: &str,
    ct: &str,
    definitions: fn(&scan::StoryFacts) -> &Observed<Vec<(DecimalId, String)>>,
    references: fn(&scan::StoryFacts) -> &Observed<Vec<(DecimalId, String)>>,
    acc: &mut Acc,
) -> JoinCompletion {
    observation_contract! {
        rules: "ID-COMMENT-REF,REF-NOTE";
        collections: "access.append*1,access.into_complete_set*1,access.invalidate*1,access.is_complete*1,access.prefix*1,access.project*1,field.collapsed_names*2,field.content_types*1,field.entries*1,field.parts*2,type.HashSet*2,type.Observed*4,type.Vec*2";
        scope: "each established owner and its selected supporting sources; shared parts in every owner; unowned references excluded; explicit no-root fallback only";
        loss: "source unavailable or malformed; XML event/depth budget; category allocation cap; unknown or ambiguous source discovery; source-local pooled-budget loss";
        fallback: "suppress only dependent absence directions and report Incomplete; independent owner and prefix findings survive";
        package_loss: "ID-COMMENT-REF,REF-NOTE";
    }

    let owners = owner_scopes(pkg);
    let owner_map = &xml.graph;
    let mut completion = JoinCompletion::vacuous();
    for (index, owner) in owners.iter().enumerate() {
        let index = OwnerId(index);
        // An unknown main can leave a typed or conventional document root outside
        // the fallback scope. Its references were never joined, so vacuous success
        // would overclaim coverage even when its content type is absent.
        let unselected_root = matches!(owner, OwnerContext::Fallback)
            && matches!(&pkg.main, Resolution::Unknown(_))
            && pkg.entries.iter().any(|e| {
                !e.is_dir
                    && (is_main_typed(pkg, &e.name)
                        || pkg.content_types.of(&e.name) == Some(package::GLOSSARY_CONTENT_TYPE)
                        || e.name.eq_ignore_ascii_case("word/document.xml")
                        || e.name.to_ascii_lowercase().ends_with("/glossary/document.xml"))
            });
        if owner_document_unreadable(pkg, owner) || unselected_root {
            completion.skipped = true;
            continue;
        }
        let source = supporting_part(pkg, owner.root(), kind, ct, acc);
        let observed = match source {
            Resolution::One(entry) => {
                if pkg.collapsed_names.contains(&entry.name) || discovery_unavailable(pkg, owner.root()) {
                    completion.skipped = true;
                    continue;
                }
                let Some(part) = xml.parts.get(&PartId::new(&entry.name)) else {
                    completion.skipped = true;
                    continue;
                };
                definitions(&part.facts).project(|items| items.iter().map(|(id, _)| id.clone()).collect::<HashSet<_>>())
            }
            Resolution::Absent => Observed::complete(HashSet::new()),
            Resolution::Ambiguous(_) | Resolution::Unknown(_) => {
                completion.skipped = true;
                continue;
            }
        };
        let Some(defs) = observed.into_complete_set() else {
            completion.skipped = true;
            continue;
        };
        let mut refs = Observed::default();
        for (part, facts) in &xml.parts {
            if owner_map.owners(part.as_str()).contains(&index) {
                // A mid-scan stop invalidates every contributed category. Never-loaded supporting
                // parts contribute nothing; selected unreadable document roots were skipped above.
                refs.append(references(&facts.facts));
                // The retained copy cannot establish all references from a competing record.
                // Keep its positive facts, but decide join completeness from the duplicate group.
                if pkg.collapsed_names.contains(&facts.name) {
                    refs.invalidate("duplicate story reference source");
                }
            }
        }
        if !refs.is_complete() {
            completion.skipped = true;
        }
        let refs = refs.prefix();
        completion.active |= matches!(source, Resolution::One(_)) || !refs.is_empty();
        match kind {
            "comments" => report_missing(acc, "ID-COMMENT-REF", refs, &defs, "w:commentReference id"),
            "footnotes" => report_missing_note(acc, refs, &defs, "footnote"),
            "endnotes" => report_missing_note(acc, refs, &defs, "endnote"),
            _ => {}
        }
    }
    completion
}

/// Record unreferenced comments only when the owner's entire reference set was observed.
fn report_unreferenced_comments(pkg: &Package, xml: &XmlScan, acc: &mut Acc) {
    observation_contract! {
        rules: "ID-COMMENT-REF";
        collections: "access.into_complete_set*1,access.missing*1,access.prefix*2,field.comment_defs*1,field.comment_refs*2,field.contexts*3,field.entries*1,type.HashMap*1,type.HashSet*3,type.Observed*1";
        scope: "DECLARED EXCEPTION: PackageRegistryContext for established-unowned own declarations; reached declarations keep their selected registry; never grants story ownership";
        loss: "source unavailable or malformed; XML event/depth budget; category allocation cap; ownership discovery loss; graph traversal/fanout budget";
        fallback: "unknown membership or registry source suppresses dependent absence; informational coverage retains the package-registry policy";
        package_loss: "independent";
    }

    let facts = xml.package_facts();
    let owner_map = &xml.graph;
    let package_registry = ownership::PackageRegistryContext::new(owner_map);
    let roles = story_roles(pkg);
    let mut incomplete_owners = HashSet::new();
    // A retained membership is positive evidence only: traversal loss can hide another owner
    // whose story contains the reference. Never turn that prefix into a complete reference set.
    let mut unknown = !owner_map.memberships_complete();
    if owner_map.budget_exhausted() {
        acc.note("ownership graph memberships are incomplete; unreferenced-comment absence was not checked".to_string());
    }
    for e in pkg.entries.iter().filter(|e| !e.is_dir) {
        if roles.get(&e.name.to_ascii_lowercase()).is_some_and(|r| r.is_story()) {
            let contexts = package_registry.contexts(&e.name);
            unknown |= owner_map.unknown(&e.name);
            if !xml.category_complete(&e.name, |f| &f.comment_refs) {
                incomplete_owners.extend(contexts.iter().copied());
            }
        }
    }
    for (index, owner) in owner_scopes(pkg).iter().enumerate() {
        let index = OwnerId(index);
        if matches!(
            supporting_part(pkg, owner.root(), "comments", package::COMMENTS_CONTENT_TYPE, acc),
            Resolution::Ambiguous(_) | Resolution::Unknown(_)
        ) {
            // Neither candidate's use as the owner's comments source is established.
            incomplete_owners.insert(index);
        }
    }
    let mut reference_sets = HashMap::new();
    for (index, _) in owner_scopes(pkg).iter().enumerate() {
        let owner = OwnerId(index);
        let observed = Observed::with_status(
            facts
                .comment_refs
                .prefix()
                .iter()
                .filter(|(_, part)| package_registry.contexts(part).contains(&owner))
                .map(|(id, _)| id.clone())
                .collect::<HashSet<_>>(),
            if !unknown && !incomplete_owners.contains(&owner) {
                Completeness::Complete
            } else {
                Completeness::Incomplete("registry references unavailable")
            },
        );
        reference_sets.insert(owner, observed.into_complete_set());
    }
    let mut seen = HashSet::new();
    for (id, part) in facts.comment_defs.prefix() {
        let contexts = package_registry.contexts(part);
        if !contexts.is_empty()
            && contexts.iter().all(|owner| reference_sets.get(owner).and_then(Option::as_ref).is_some_and(|set| set.missing(id)))
            && seen.insert((part, id))
        {
            acc.note(format!("w:comment id {id} in {part} is never referenced"));
        }
    }
}

/// Verify header/footer references together: the element reference, the relationship it names and
/// that relationship's kind and target content type. A reference to a non-header relationship (say
/// an image) is a `REF-HEADER` finding, not a pass.
fn check_header_footer_refs(pkg: &Package, facts: &scan::StoryFacts, acc: &mut Acc) {
    observation_contract! {
        rules: "REF-HEADER";
        collections: "access.prefix*4,field.content_types*2,field.footer_refs*2,field.header_refs*2,field.incomplete*1,field.rels*1,type.HashSet*2";
        scope: "main-only header/footer reference category, enforced at extraction; first XML declaration binds duplicate relationship IDs";
        loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
        fallback: "retain proven prefix presence and duplicates; return loss to the caller; no negative membership from an incomplete set";
        package_loss: "REF-HEADER";
    }

    let Resolution::One(main) = &pkg.main else { return };
    let want = rels_part_name(main);
    // The main part's relationships could not be read: its header/footer relationships are unknown,
    // not absent, so a reference must not be reported as having no relationship.
    if package::relationships_incomplete(pkg, main) {
        return;
    }
    let Some(rp) = pkg.rels.iter().find(|r| r.name.eq_ignore_ascii_case(&want)) else {
        for (id, part) in facts.header_refs.prefix().iter().chain(facts.footer_refs.prefix().iter()) {
            acc.add("REF-HEADER", Some(part.clone()), format!("the main part {main} has a header/footer reference {id} but no relationships part"));
        }
        return;
    };
    let by_id = package::relationship_bindings(rp);
    let groups: [(&str, &str, &str, &[(RelId, String)]); 2] = [
        ("header", package::HEADER_CONTENT_TYPE, "headerReference", facts.header_refs.prefix()),
        ("footer", package::FOOTER_CONTENT_TYPE, "footerReference", facts.footer_refs.prefix()),
    ];
    for (kind, want_ct, element, refs) in groups {
        let mut reported: HashSet<&str> = HashSet::new();
        for (id, part) in refs {
            if !reported.insert(id.as_str()) {
                continue;
            }
            match by_id.resolve(id.as_str()) {
                Resolution::Unknown(_) | Resolution::Ambiguous(_) => {}
                Resolution::Absent => {
                    acc.add("REF-HEADER", Some(part.clone()), format!("w:{element} r:id {id} in {part} has no matching relationship"))
                }
                Resolution::One(rel) if rel.role.kind() != Some(kind) => acc.add(
                    "REF-HEADER",
                    Some(part.clone()),
                    format!(
                        "w:{element} r:id {id} in {part} resolves to a {} relationship, not a {kind} relationship",
                        rel.role.kind().unwrap_or(&rel.kind)
                    ),
                ),
                Resolution::One(rel) if rel.external || rel.target_abs.is_none() => acc.add(
                    "REF-HEADER",
                    Some(part.clone()),
                    format!("w:{element} r:id {id} in {part} resolves to an external relationship with no internal target"),
                ),
                Resolution::One(rel) => {
                    if let Some(target) = &rel.target_abs {
                        let ct = pkg.content_types.of(target);
                        if (!pkg.content_types.incomplete || ct.is_some()) && ct != Some(want_ct) {
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

/// Select an owner's official role target, deduplicating canonical part identities. Even an
/// unreadable candidate competes: its bytes cannot justify choosing a different source for Word.
fn owner_rel_part<'a>(pkg: &'a Package, owner: &str, kind: &str) -> Resolution<&'a package::Entry> {
    observation_contract! {
        rules: "-";
        collections: "field.rels*2,lookup.entry_by_name*1";
        scope: "source-local bounded extraction; no implicit document owner";
        loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
        fallback: "retain proven prefix presence and duplicates; return loss to the caller; no negative membership from an incomplete set";
        package_loss: "independent";
    }

    let want = rels_part_name(owner);
    let Some(rp) = pkg.rels.iter().find(|r| r.name.eq_ignore_ascii_case(&want)) else {
        return Resolution::discovered(None, !discovery_unavailable(pkg, Some(owner)));
    };
    let mut target: Option<&str> = None;
    for r in &rp.rels {
        if r.external || r.role.kind() != Some(kind) {
            continue;
        }
        if let Some(t) = r.target_abs.as_deref() {
            if target.is_some_and(|first| !first.eq_ignore_ascii_case(t)) {
                return Resolution::Ambiguous("distinct official targets compete");
            }
            target = Some(t);
        }
    }
    match target {
        Some(name) => match entry_by_name(pkg, name) {
            Resolution::One(entry) => Resolution::One(entry),
            Resolution::Absent => Resolution::discovered(None, !discovery_unavailable(pkg, Some(owner))),
            Resolution::Unknown(reason) => Resolution::Unknown(reason),
            Resolution::Ambiguous(reason) => Resolution::Ambiguous(reason),
        },
        None => Resolution::discovered(None, !discovery_unavailable(pkg, Some(owner))),
    }
}

/// Resolve a role without recording coverage, including the explicit no-owner fallback.
fn owner_rel_selection<'a>(pkg: &'a Package, owner: Option<&str>, kind: &str, ct: &str) -> Resolution<&'a package::Entry> {
    match owner {
        Some(owner) => owner_rel_part(pkg, owner, kind),
        None => find_by_type(pkg, ct),
    }
}

/// Select only the owner's own role target (J2), with an explicit unknown outcome when distinct
/// official targets compete. The no-owner unique-content-type fallback remains separate: it
/// serves lone-supporting-part fixtures and does not assert an owner relationship selection.
fn supporting_part<'a>(pkg: &'a Package, owner: Option<&str>, kind: &str, ct: &str, acc: &mut Acc) -> Resolution<&'a package::Entry> {
    observation_contract! {
        rules: "-";
        collections: "field.coverage*1";
        scope: "source-local bounded extraction; no implicit document owner";
        loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
        fallback: "retain proven prefix presence and duplicates; return loss to the caller; no negative membership from an incomplete set";
        package_loss: "independent";
    }

    let selection = owner_rel_selection(pkg, owner, kind, ct);
    let note = match selection {
        Resolution::One(_) | Resolution::Absent => None,
        Resolution::Ambiguous(reason) => {
            Some(format!("ambiguous {kind} selection for {}: {reason}; dependent checks were skipped", owner.unwrap_or("the package")))
        }
        Resolution::Unknown(reason) => {
            Some(format!("unknown {kind} selection for {}: {reason}; dependent checks were skipped", owner.unwrap_or("the package")))
        }
    };
    if let Some(note) = note {
        if !acc.coverage.contains(&note) {
            acc.note(note);
        }
        acc.set_incomplete();
    }
    selection
}

/// Whether the source that would discover an owner's supporting part could not be read, so the
/// part's existence is unestablished rather than established-absent. In an established owner scope
/// that source is the owner's `.rels`; in the no-owner fallback scope it is the content-type
/// manifest that [`find_by_type`] reads.
fn discovery_unavailable(pkg: &Package, owner: Option<&str>) -> bool {
    observation_contract! {
        rules: "-";
        collections: "field.content_types*1,field.incomplete*1,lookup.content_unavailable*1";
        scope: "source-local bounded extraction; no implicit document owner";
        loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
        fallback: "retain proven prefix presence and duplicates; return loss to the caller; no negative membership from an incomplete set";
        package_loss: "independent";
    }

    // An unexamined archive suffix may hold the target or its discovery declarations.
    if pkg.entries_truncated || pkg.identity_incomplete {
        return true;
    }
    match owner {
        Some(owner) => package::relationships_incomplete(pkg, owner),
        None => {
            (match package::content_unavailable(pkg, "[Content_Types].xml") {
                Resolution::One(unavailable) => unavailable,
                Resolution::Absent => false,
                Resolution::Unknown(_) => true,
                Resolution::Ambiguous(_) => true,
            }) || pkg.content_types.incomplete
        }
    }
}

/// Build explicit memberships; no traversal may absorb a different glossary root.
#[cfg(test)]
fn owner_index_map(pkg: &Package) -> PartGraph {
    PartGraph::new(pkg)
}

/// Whether all sources that can discover owner subtrees have been examined.
fn ownership_discovery_complete(pkg: &Package) -> bool {
    observation_contract! {
        rules: "-";
        collections: "field.content_types*1,field.incomplete*2,field.rels*1,lookup.content_unavailable*1";
        scope: "source-local bounded extraction; no implicit document owner";
        loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
        fallback: "retain proven prefix presence and duplicates; return loss to the caller; no negative membership from an incomplete set";
        package_loss: "independent";
    }

    !pkg.entries_truncated
        && !pkg.identity_incomplete
        && !pkg.content_types.incomplete
        && !(match package::content_unavailable(pkg, "[Content_Types].xml") {
            Resolution::One(unavailable) => unavailable,
            Resolution::Absent => false,
            Resolution::Unknown(_) => true,
            Resolution::Ambiguous(_) => true,
        })
        && !pkg.rels.iter().any(|rp| rp.incomplete)
}

/// Established contexts only. Unowned and unknown are retained separately in the graph.
fn part_owners<'a>(graph: &'a PartGraph, part: &str) -> &'a [OwnerId] {
    graph.owners(part)
}

/// Only the explicit fallback context can have undiscovered contributors.
fn unresolved_fallback_contributor(graph: &PartGraph, _owner: OwnerId, part: &str) -> bool {
    graph.unknown(part)
}

/// Discovery loss affects the contexts whose reached descendants have unread relationships.
fn descendant_discovery_incomplete(pkg: &Package, graph: &PartGraph, owner: OwnerId) -> bool {
    observation_contract! {
        rules: "-";
        collections: "field.parts*1";
        scope: "source-local bounded extraction; no implicit document owner";
        loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
        fallback: "retain proven prefix presence and duplicates; return loss to the caller; no negative membership from an incomplete set";
        package_loss: "independent";
    }

    if owner_scopes(pkg) == [OwnerContext::Fallback] {
        return !ownership_discovery_complete(pkg);
    }
    graph.parts(owner).any(|part| package::relationships_incomplete(pkg, part))
}

/// Verify that style references in story parts resolve to a defined style in their owner scope.
fn check_style_refs(pkg: &Package, xml: &XmlScan, acc: &mut Acc) {
    observation_contract! {
        rules: "REF-STYLE";
        collections: "access.into_complete_set*1,access.missing*1,access.prefix*3,field.style_refs*3,parser.parse_style_ids*1,type.CompleteSet*1,type.HashSet*2,type.Observed*1,type.Vec*2";
        scope: "each established owner and its selected supporting sources; shared parts in every owner; unowned references excluded; explicit no-root fallback only";
        loss: "source unavailable or malformed; XML event/depth budget; category allocation cap; unknown or ambiguous source discovery; source-local pooled-budget loss";
        fallback: "suppress only dependent absence directions and report Incomplete; independent owner and prefix findings survive";
        package_loss: "REF-STYLE";
    }

    let facts = &xml.package_facts();
    let cap = acc.limits.facts;
    let owners = owner_scopes(pkg);
    let owner_map = &xml.graph;
    let mut per_owner: Vec<Option<CompleteSet<StyleId>>> = Vec::with_capacity(owners.len());
    let mut present = false;
    let mut capped = false;
    let mut parsed_any = false;
    let mut selection_ambiguous = false;
    for owner in &owners {
        let selection = supporting_part(pkg, owner.root(), "styles", package::STYLES_CONTENT_TYPE, acc);
        selection_ambiguous |= matches!(selection, Resolution::Ambiguous(_) | Resolution::Unknown(_));
        let entry = match selection {
            Resolution::One(entry) => entry,
            Resolution::Absent | Resolution::Ambiguous(_) | Resolution::Unknown(_) => {
                per_owner.push(None);
                continue;
            }
        };
        present = true;
        if !xml_source_complete(xml, entry) {
            acc.note(format!("{} was not fully examined; style-reference absence was not checked", entry.name));
            acc.set_incomplete();
            per_owner.push(None);
            continue;
        }
        let mut guard = scan::ParseGuard::new(acc.limits.events, acc.limits.depth).watching(&["style"]);
        let parsed = entry.decoded.as_deref().map(|text| scan::parse_style_ids(text, cap, &mut guard));
        if !guard.exhausted() && guard.unresolved() {
            acc.note(format!("{} has a style definition whose namespace is not resolved; style-reference absence was not checked", entry.name));
            acc.set_incomplete();
            per_owner.push(None);
            continue;
        }
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
                if c {
                    acc.note(format!("{} exceeded the {cap}-entry budget; style-reference absence was not checked", entry.name));
                }
                per_owner.push(
                    Observed::with_status(
                        ids,
                        if c || discovery_unavailable(pkg, owner.root()) {
                            Completeness::Incomplete("style source or definition budget")
                        } else {
                            Completeness::Complete
                        },
                    )
                    .into_complete_set(),
                );
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
        if !selection_ambiguous && !facts.style_refs.prefix().is_empty() {
            acc.note("no styles part is reachable; w:pStyle/w:rStyle references were not checked".to_string());
        }
        return;
    }
    if !parsed_any {
        if !selection_ambiguous && !facts.style_refs.prefix().is_empty() {
            acc.note("a styles part is present but does not parse; w:pStyle/w:rStyle references were not checked".to_string());
        }
        return;
    }
    // Check each reference against its owner's own definitions only (H2).
    let mut reported: HashSet<(&str, &str)> = HashSet::new();
    for (id, part) in facts.style_refs.prefix() {
        for &owner in part_owners(owner_map, part) {
            let Some(Some(defs)) = per_owner.get(owner.0) else { continue };
            if defs.missing(id) && reported.insert((id.as_str(), part.as_str())) {
                acc.add("REF-STYLE", Some(part.clone()), format!("style id {id} in {part} has no definition in the package"));
            }
        }
    }
}

/// The vocabulary family: id and name collisions inside the styles,
/// fontTable, and custom-properties parts. Each part is judged in its
/// own scope; a part that is present but cannot be read or parsed
/// makes the run INCOMPLETE for these rules rather than passing them
/// silently (absence of the part itself is legitimate and silent).
fn check_vocabulary(pkg: &Package, acc: &mut Acc) {
    observation_contract! {
        rules: "FONT-DUP-NAME,PROP-DUP-NAME,STYLE-COUNT,STYLE-DUP-ID,STYLE-DUP-NAME,STYLE-ID-LENGTH,STYLE-RESERVED-ID,STYLE-RESERVED-NAME";
        collections: "-";
        scope: "source-local bounded extraction; no implicit document owner";
        loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
        fallback: "retain proven prefix presence and duplicates; return loss to the caller; no negative membership from an incomplete set";
        package_loss: "independent";
    }

    let cap = acc.limits.facts;
    let owners = owner_scopes(pkg);
    for owner in &owners {
        if let Resolution::One(entry) = supporting_part(pkg, owner.root(), "styles", package::STYLES_CONTENT_TYPE, acc) {
            check_styles_vocabulary(entry, cap, acc);
        }
        if let Resolution::One(entry) = supporting_part(pkg, owner.root(), "fontTable", package::FONT_TABLE_CONTENT_TYPE, acc) {
            check_name_dups(entry, cap, acc, true);
        }
    }
    let custom_properties_checked = match find_by_type(pkg, package::CUSTOM_PROPERTIES_CONTENT_TYPE) {
        Resolution::One(entry) => {
            check_name_dups(entry, cap, acc, false);
            true
        }
        Resolution::Absent => true,
        Resolution::Ambiguous(reason) => {
            acc.note(format!("custom-properties selection is ambiguous ({reason}); PROP-DUP-NAME was not checked"));
            acc.set_incomplete();
            false
        }
        Resolution::Unknown(reason) => {
            acc.note(format!("custom-properties selection is unknown ({reason}); PROP-DUP-NAME was not checked"));
            acc.set_incomplete();
            false
        }
    };
    acc.ran("STYLE-DUP-ID");
    acc.ran("STYLE-DUP-NAME");
    acc.ran("STYLE-RESERVED-NAME");
    acc.ran("STYLE-RESERVED-ID");
    acc.ran("STYLE-COUNT");
    acc.ran("STYLE-ID-LENGTH");
    acc.ran("FONT-DUP-NAME");
    if custom_properties_checked {
        acc.ran("PROP-DUP-NAME");
    }
}

/// Evaluate the six styles-part vocabulary rules for one styles part.
fn check_styles_vocabulary(entry: &package::Entry, cap: usize, acc: &mut Acc) {
    observation_contract! {
        rules: "STYLE-COUNT,STYLE-DUP-ID,STYLE-DUP-NAME,STYLE-ID-LENGTH,STYLE-RESERVED-ID,STYLE-RESERVED-NAME";
        collections: "parser.parse_style_defs*1,type.HashMap*4,type.HashSet*2";
        scope: "source-local bounded extraction; no implicit document owner";
        loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
        fallback: "retain proven prefix presence and duplicates; return loss to the caller; no negative membership from an incomplete set";
        package_loss: "independent";
    }

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
    let mut ids: HashMap<String, StyleId> = HashMap::new();
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
    observation_contract! {
        rules: "FONT-DUP-NAME,PROP-DUP-NAME";
        collections: "parser.parse_custom_prop_names*1,parser.parse_font_names*1,type.HashMap*1,type.Vec*1";
        scope: "source-local bounded extraction; no implicit document owner";
        loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
        fallback: "retain proven prefix presence and duplicates; return loss to the caller; no negative membership from an incomplete set";
        package_loss: "independent";
    }

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
    let mut groups = HashMap::new();
    let mut duplicates = Vec::new();
    for name in &names {
        let (first, count) = groups.entry(name.to_lowercase()).or_insert((name, 0usize));
        *count = count.saturating_add(1);
        if *count == 2 {
            // Retain the first two observed spellings and the order in which groups were proven.
            duplicates.push((*first, name));
        }
    }
    for (first, second) in duplicates {
        let message = if fonts {
            format!("font name {first} appears more than once in {part} (again as {second})")
        } else {
            format!(
                "custom property name {first} appears more than once in {part} (again as {second}); property names must be case-insensitively unique, and Word shows a corruption notice for this collision"
            )
        };
        acc.add(rule, Some(part.clone()), message);
    }
}

/// The part whose effective content type is `ct`, if any. Supporting parts are found by type, not
/// by their conventional file name or folder.
fn find_by_type<'a>(pkg: &'a Package, ct: &str) -> Resolution<&'a package::Entry> {
    observation_contract! {
        rules: "-";
        collections: "field.content_types*1,field.entries*1";
        scope: "source-local bounded extraction; no implicit document owner";
        loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
        fallback: "retain proven prefix presence and duplicates; return loss to the caller; no negative membership from an incomplete set";
        package_loss: "independent";
    }

    let mut candidates = pkg.entries.iter().filter(|e| !e.is_dir && pkg.content_types.of(&e.name) == Some(ct));
    let first = candidates.next();
    if candidates.next().is_some() {
        return Resolution::Ambiguous("multiple content-type candidates compete");
    }
    Resolution::discovered(first, !discovery_unavailable(pkg, None))
}

/// Verify numbering definitions and references in story and styles parts for each owner scope.
fn check_numbering(pkg: &Package, xml: &XmlScan, acc: &mut Acc) {
    observation_contract! {
        rules: "NUM-DANGLING";
        collections: "access.into_complete_set*1,type.CompleteSet*4,type.HashMap*2,type.HashSet*2,type.Observed*1,type.Vec*4";
        scope: "each established owner and its selected supporting sources; shared parts in every owner; unowned references excluded; explicit no-root fallback only";
        loss: "source unavailable or malformed; XML event/depth budget; category allocation cap; unknown or ambiguous source discovery; source-local pooled-budget loss";
        fallback: "suppress only dependent absence directions and report Incomplete; independent owner and prefix findings survive";
        package_loss: "independent";
    }

    let facts = &xml.package_facts();
    acc.ran("NUM-DANGLING");
    let cap = acc.limits.facts;
    let owners = owner_scopes(pkg);
    let owner_map = &xml.graph;
    let mut numbering_cache: HashMap<String, Option<CompleteSet<DecimalId>>> = HashMap::new();
    // Each owner's numbering set, in owner order. `Some(set)` = the owner's numbering part was
    // examined, so `set` is the definitions it declares (empty when the owner has no numbering part
    // — established absent, so its references are genuinely dangling). `None` = the owner's
    // numbering could not be established: the part exists but could not be examined, or the
    // discovery source (its `.rels`, or the manifest in the no-owner scope) could not be read, so
    // that owner's references are not established as dangling and its dependent checks are skipped
    // (H8).
    let mut num_sets: Vec<Option<CompleteSet<DecimalId>>> = Vec::with_capacity(owners.len());
    let mut style_checks: Vec<(String, CompleteSet<DecimalId>)> = Vec::new();
    for owner in &owners {
        let selection = supporting_part(pkg, owner.root(), "numbering", package::NUMBERING_CONTENT_TYPE, acc);
        let num_part = match selection {
            Resolution::One(e) => Some(e.name.clone()),
            Resolution::Absent | Resolution::Ambiguous(_) | Resolution::Unknown(_) => None,
        };
        let num_set: Option<CompleteSet<DecimalId>> = match &num_part {
            Some(np) => {
                if let Some(cached) = numbering_cache.get(np) {
                    cached.clone()
                } else {
                    let parsed = process_numbering_part(pkg, xml, np, cap, acc);
                    numbering_cache.insert(np.clone(), parsed.clone());
                    parsed
                }
            }
            None => {
                // A readable discovery source that reaches no numbering part establishes that the
                // owner has none (an empty set, exactly as before). When the discovery source
                // itself could not be read, the owner's numbering is unknown, not absent: its
                // references must not be reported against an empty set (H8).
                if matches!(selection, Resolution::Ambiguous(_) | Resolution::Unknown(_)) || discovery_unavailable(pkg, owner.root()) {
                    None
                } else {
                    Observed::complete(HashSet::new()).into_complete_set()
                }
            }
        };
        let num_set = if discovery_unavailable(pkg, owner.root()) { None } else { num_set };
        // Each owner's styles are checked against that owner's own numbering set, and only when it
        // could actually be examined.
        if let Resolution::One(entry) = supporting_part(pkg, owner.root(), "styles", package::STYLES_CONTENT_TYPE, acc)
            && let Some(set) = &num_set
        {
            style_checks.push((entry.name.clone(), set.clone()));
        }
        num_sets.push(num_set);
    }
    report_num_refs(acc, facts, &num_sets, owner_map);
    // A shared styles source can be valid for one owner and dangling for another. Deduplicate
    // only established findings, after evaluating every available owner/definition-set pairing.
    let mut reported = HashSet::new();
    for (name, num_set) in &style_checks {
        report_style_num_refs(pkg, name, num_set, cap, &mut reported, acc);
    }
}

/// Parse a numbering part, reporting duplicate definitions and dangling `w:abstractNumId`
/// references. `None` when the part could not be read, did not parse, or exceeded a budget (its
/// references must not be checked).
fn process_numbering_part(pkg: &Package, xml: &XmlScan, name: &str, cap: usize, acc: &mut Acc) -> Option<CompleteSet<DecimalId>> {
    observation_contract! {
        rules: "NUM-DANGLING";
        collections: "access.into_complete_set*2,access.missing*1,field.abstract_ids*2,field.num_ids*2,field.num_to_abstract*1,lookup.entry_by_name*1,parser.parse_numbering*1,type.CompleteSet*1,type.HashSet*2,type.Observed*2";
        scope: "source-local bounded extraction; no implicit document owner";
        loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
        fallback: "retain proven prefix presence and duplicates; return loss to the caller; no negative membership from an incomplete set";
        package_loss: "NUM-DANGLING";
    }

    let Resolution::One(entry) = entry_by_name(pkg, name) else { return None };
    if !xml_source_complete(xml, entry) {
        acc.note(format!("{name} was not fully examined; list numbering absence was not checked"));
        acc.set_incomplete();
        return None;
    }
    let Some(text) = entry.decoded.as_deref() else {
        acc.note(format!("{name} could not be read; list numbering references were not checked"));
        return None;
    };
    let mut guard = scan::ParseGuard::new(acc.limits.events, acc.limits.depth).watching(&["num", "abstractNum", "abstractNumId"]);
    let defs = scan::parse_numbering(text, cap, &mut guard);
    if !guard.exhausted() && guard.unresolved() {
        acc.note(format!("{name} has a numbering definition whose namespace is not resolved; list numbering absence was not checked"));
        acc.set_incomplete();
        return None;
    }
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
        acc.note(format!("{name} exceeded the {cap}-entry budget; numbering definition absence was not checked"));
        acc.set_incomplete();
    }
    report_dups_values(acc, &defs.num_ids, &entry.name, "w:num numId");
    report_dups_values(acc, &defs.abstract_ids, &entry.name, "w:abstractNum abstractNumId");
    let abstract_set = Observed::with_status(
        defs.abstract_ids.iter().cloned().collect::<HashSet<_>>(),
        if defs.capped { Completeness::Incomplete("abstract numbering budget") } else { Completeness::Complete },
    )
    .into_complete_set();
    for (num, abs) in &defs.num_to_abstract {
        match abs {
            Some(a) if abstract_set.as_ref().is_some_and(|set| set.missing(a)) => acc.add(
                "NUM-DANGLING",
                Some(entry.name.clone()),
                format!("w:num numId {num} references w:abstractNumId {a}, which has no w:abstractNum"),
            ),
            None => acc.add("NUM-DANGLING", Some(entry.name.clone()), format!("w:num numId {num} has no w:abstractNumId")),
            _ => {}
        }
    }
    // A retained reference is still checkable, but a truncated definition set cannot prove absence.
    Observed::with_status(
        defs.num_ids.into_iter().collect::<HashSet<_>>(),
        if defs.capped { Completeness::Incomplete("numbering definition budget") } else { Completeness::Complete },
    )
    .into_complete_set()
}

/// `NUM-DANGLING` for `w:numPr/w:numId` references in a styles part, checked against its owner's
/// numbering part. `numId` `0` is the "remove numbering" sentinel and is never a reference.
fn report_style_num_refs(
    pkg: &Package,
    styles_name: &str,
    num_set: &CompleteSet<DecimalId>,
    cap: usize,
    reported: &mut HashSet<(String, DecimalId)>,
    acc: &mut Acc,
) {
    observation_contract! {
        rules: "NUM-DANGLING";
        collections: "access.missing*1,lookup.entry_by_name*1,parser.parse_style_num_ids*1,type.CompleteSet*1,type.HashSet*1";
        scope: "each established owner and its selected supporting sources; shared parts in every owner; unowned references excluded; explicit no-root fallback only";
        loss: "source unavailable or malformed; XML event/depth budget; category allocation cap; unknown or ambiguous source discovery; source-local pooled-budget loss";
        fallback: "suppress only dependent absence directions and report Incomplete; independent owner and prefix findings survive";
        package_loss: "NUM-DANGLING";
    }

    let Resolution::One(entry) = entry_by_name(pkg, styles_name) else { return };
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
    for val in &vals {
        if val.is_zero() {
            continue;
        }
        if num_set.missing(val) && reported.insert((entry.name.to_ascii_lowercase(), val.clone())) {
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
fn report_num_refs(acc: &mut Acc, facts: &scan::StoryFacts, num_sets: &[Option<CompleteSet<DecimalId>>], owner_map: &PartGraph) {
    observation_contract! {
        rules: "NUM-DANGLING";
        collections: "access.missing*1,access.prefix*1,field.num_ids*1,type.CompleteSet*1,type.HashSet*2";
        scope: "each established owner and its selected supporting sources; shared parts in every owner; unowned references excluded; explicit no-root fallback only";
        loss: "source unavailable or malformed; XML event/depth budget; category allocation cap; unknown or ambiguous source discovery; source-local pooled-budget loss";
        fallback: "suppress only dependent absence directions and report Incomplete; independent owner and prefix findings survive";
        package_loss: "NUM-DANGLING";
    }

    let mut reported: HashSet<(&DecimalId, &str)> = HashSet::new();
    for (val, part) in facts.num_ids.prefix() {
        if val.is_zero() {
            continue;
        }
        for &owner in part_owners(owner_map, part) {
            let Some(Some(set)) = num_sets.get(owner.0) else { continue };
            if set.missing(val) && reported.insert((val, part.as_str())) {
                acc.add("NUM-DANGLING", Some(part.clone()), format!("w:numPr references numId {val}, which has no w:num definition"));
            }
        }
    }
}

/// Report duplicate numeric identities within a part for a specified attribute/construct name.
fn report_dups_values(acc: &mut Acc, values: &[DecimalId], part: &str, what: &str) {
    observation_contract! {
        rules: "NUM-DANGLING";
        collections: "type.HashMap*2,type.Vec*1";
        scope: "source-local bounded extraction; no implicit document owner";
        loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
        fallback: "retain proven prefix presence and duplicates; return loss to the caller; no negative membership from an incomplete set";
        package_loss: "independent";
    }

    let mut counts: HashMap<&DecimalId, usize> = HashMap::new();
    for v in values {
        *counts.entry(v).or_default() += 1;
    }
    let mut dups: Vec<&DecimalId> = counts.into_iter().filter(|(_, c)| *c > 1).map(|(v, _)| v).collect();
    dups.sort_unstable();
    for v in dups {
        acc.add("NUM-DANGLING", Some(part.to_string()), format!("duplicate {what} {v} in {part}"));
    }
}

/// Verify comment-threading relationships between comments, commentsExtended, and commentsIds parts.
fn check_commentex(pkg: &Package, xml: &XmlScan, acc: &mut Acc) {
    observation_contract! {
        rules: "COMMENTEX-LINK";
        collections: "access.into_complete_set*3,access.missing*4,access.prefix*1,field.comment_para_ids*2,field.entries*4,type.HashSet*3,type.Observed*3";
        scope: "each established owner and its selected supporting sources; shared parts in every owner; unowned references excluded; explicit no-root fallback only";
        loss: "source unavailable or malformed; XML event/depth budget; category allocation cap; unknown or ambiguous source discovery; source-local pooled-budget loss";
        fallback: "suppress only dependent absence directions and report Incomplete; independent owner and prefix findings survive";
        package_loss: "COMMENTEX-LINK";
    }

    let facts = &xml.package_facts();
    let cap = acc.limits.facts;
    let owners = owner_scopes(pkg);
    let owner_map = &xml.graph;
    let roles = story_roles(pkg);
    let active = |owner: &OwnerContext| {
        !matches!(owner_rel_selection(pkg, owner.root(), "commentsExtended", package::COMMENTS_EXTENDED_CONTENT_TYPE), Resolution::Absent)
            || !matches!(owner_rel_selection(pkg, owner.root(), "commentsIds", package::COMMENTS_IDS_CONTENT_TYPE), Resolution::Absent)
    };
    if !owners.iter().any(active) {
        return;
    }
    acc.ran("COMMENTEX-LINK");
    for (i, owner) in owners.iter().enumerate() {
        if !active(owner) {
            continue;
        }
        let i = OwnerId(i);
        let comments_source = supporting_part(pkg, owner.root(), "comments", package::COMMENTS_CONTENT_TYPE, acc);
        let selected = owner.root().is_some();
        let contributes = |part: &str| {
            if selected {
                match comments_source {
                    Resolution::One(entry) => entry.name.eq_ignore_ascii_case(part),
                    Resolution::Absent | Resolution::Ambiguous(_) | Resolution::Unknown(_) => false,
                }
            } else {
                part_owners(owner_map, part).contains(&i)
            }
        };
        let mut comments_complete = matches!(comments_source, Resolution::One(_) | Resolution::Absent)
            && !discovery_unavailable(pkg, owner.root())
            && (selected || !descendant_discovery_incomplete(pkg, owner_map, i));
        for entry in pkg.entries.iter().filter(|e| {
            !e.is_dir
                && if selected {
                    contributes(&e.name)
                } else {
                    roles.get(&e.name.to_ascii_lowercase()).is_some_and(|r| r.is_comments())
                        && (contributes(&e.name) || unresolved_fallback_contributor(owner_map, i, &e.name))
                }
        }) {
            if (!selected && unresolved_fallback_contributor(owner_map, i, &entry.name))
                || !xml.category_complete(&entry.name, |f| &f.comment_para_ids)
            {
                comments_complete = false;
                acc.note(format!(
                    "comment-threading paragraphs for {} could not be fully checked: contributing source {} had unresolved ownership or was not fully examined",
                    owner.root().unwrap_or("the package"), entry.name
                ));
            }
        }
        if !comments_complete {
            acc.note(format!(
                "comment-threading paragraphs for {} could not be fully checked (comments ownership or source facts were not fully examined)",
                owner.root().unwrap_or("the package")
            ));
            acc.set_incomplete();
        }
        // Paragraphs come from the owner's selected comments target. A shared target is checked
        // independently in every owner that selects it; unrelated discovery cannot add sources.
        let comment_para = Observed::with_status(
            facts.comment_para_ids.prefix().iter().filter(|(_, p)| contributes(p)).map(|(p, _)| p.clone()).collect::<HashSet<_>>(),
            if comments_complete { Completeness::Complete } else { Completeness::Incomplete("comments paragraphs unavailable") },
        )
        .into_complete_set();
        let ext_source = supporting_part(pkg, owner.root(), "commentsExtended", package::COMMENTS_EXTENDED_CONTENT_TYPE, acc);
        let ext = match ext_source {
            Resolution::One(entry) => read_comments_ext(entry, xml, cap, acc).map(|e| (entry.name.clone(), e)),
            Resolution::Absent | Resolution::Ambiguous(_) | Resolution::Unknown(_) => None,
        };
        let ex_para = ext.as_ref().and_then(|(_, e)| {
            Observed::with_status(
                e.entries.iter().map(|(p, _)| p.clone()).collect::<HashSet<_>>(),
                if e.capped { Completeness::Incomplete("commentEx budget") } else { Completeness::Complete },
            )
            .into_complete_set()
        });
        if let Some((name, e)) = &ext {
            for (para, parent) in &e.entries {
                if comment_para.as_ref().is_some_and(|set| set.missing(para)) {
                    acc.add(
                        "COMMENTEX-LINK",
                        Some(name.clone()),
                        format!("w15:commentEx paraId {para} has no paragraph with that w14:paraId in comments.xml"),
                    );
                }
                if let Some(p) = parent
                    && ex_para.as_ref().is_some_and(|set| set.missing(p))
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
        let cid = match supporting_part(pkg, owner.root(), "commentsIds", package::COMMENTS_IDS_CONTENT_TYPE, acc) {
            Resolution::One(entry) => read_comments_ids(entry, xml, cap, acc),
            Resolution::Absent | Resolution::Ambiguous(_) | Resolution::Unknown(_) => None,
        };
        let Some((cid_name, cid_ids, cid_complete)) = &cid else { continue };
        let cid_set = Observed::with_status(
            cid_ids.iter().cloned().collect::<HashSet<_>>(),
            if *cid_complete && !discovery_unavailable(pkg, owner.root()) {
                Completeness::Complete
            } else {
                Completeness::Incomplete("commentsIds budget")
            },
        )
        .into_complete_set();
        let counterpart_ext = if discovery_unavailable(pkg, owner.root()) { None } else { ex_para.as_ref() };
        for id in cid_ids {
            if counterpart_ext.as_ref().is_some_and(|set| set.missing(id)) {
                acc.add("COMMENTEX-LINK", Some(cid_name.clone()), format!("commentsIds paraId {id} has no commentEx counterpart"));
            }
        }
        if let Some((name, e)) = &ext {
            for (para, _) in &e.entries {
                if cid_set.as_ref().is_some_and(|set| set.missing(para)) {
                    acc.add("COMMENTEX-LINK", Some(name.clone()), format!("commentEx paraId {para} has no commentsIds paraId counterpart"));
                }
            }
        }
    }
}

/// Parse a commentsExtended part, recording a note (and INCOMPLETE for a budget exhaustion) when it
/// cannot be checked. `None` when the part's link checks must be skipped.
fn read_comments_ext(entry: &package::Entry, xml: &XmlScan, cap: usize, acc: &mut Acc) -> Option<scan::CommentEx> {
    observation_contract! {
        rules: "COMMENTEX-LINK";
        collections: "parser.parse_comments_extended*1";
        scope: "source-local bounded extraction; no implicit document owner";
        loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
        fallback: "retain proven prefix presence and duplicates; return loss to the caller; no negative membership from an incomplete set";
        package_loss: "independent";
    }

    let name = &entry.name;
    if !xml_source_complete(xml, entry) {
        acc.note(format!("{name} was not fully examined; commentEx definition absence was not checked"));
        acc.set_incomplete();
        return None;
    }
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
        acc.note(format!(
            "{name} exceeded the {cap}-entry budget; later commentEx entries were not recorded and commentEx definition absence was not checked"
        ));
        acc.set_incomplete();
    }
    Some(ext)
}

/// Parse a commentsIds part, returning its part name and `w16cid:paraId` values. `None` when the part's link
/// checks must be skipped. The returned boolean says whether the definition set is complete.
fn read_comments_ids(entry: &package::Entry, xml: &XmlScan, cap: usize, acc: &mut Acc) -> Option<(String, Vec<HexId>, bool)> {
    observation_contract! {
        rules: "COMMENTEX-LINK";
        collections: "parser.parse_comments_ids*1,type.Vec*1";
        scope: "source-local bounded extraction; no implicit document owner";
        loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
        fallback: "retain proven prefix presence and duplicates; return loss to the caller; no negative membership from an incomplete set";
        package_loss: "independent";
    }

    let name = &entry.name;
    if !xml_source_complete(xml, entry) {
        acc.note(format!("{name} was not fully examined; commentsIds definition absence was not checked"));
        acc.set_incomplete();
        return None;
    }
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
        acc.note(format!(
            "{name} exceeded the {cap}-entry budget; later paraIds were not recorded and commentsIds definition absence was not checked"
        ));
        acc.set_incomplete();
    }
    Some((entry.name.clone(), ids, !capped))
}

/// Report custom XML item-properties schema references missing from settings.xml as coverage.
fn check_custom_xml(pkg: &Package, xml: &XmlScan, acc: &mut Acc) {
    observation_contract! {
        rules: "CUSTOMXML-SCHEMA";
        collections: "access.into_complete_set*2,access.missing*1,field.content_types*3,field.contexts*2,field.entries*3,lookup.content_unavailable*1,parser.parse_attached_schemas*1,parser.parse_custom_xml_schemas*1,type.CompleteSet*1,type.HashSet*3,type.Observed*2,type.Vec*3";
        scope: "DECLARED EXCEPTION: PackageRegistryContext for established-unowned own declarations; reached declarations keep their selected registry; never grants story ownership";
        loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
        fallback: "unknown membership or registry source suppresses dependent absence; informational coverage retains the package-registry policy";
        package_loss: "CUSTOMXML-SCHEMA";
    }

    acc.ran("CUSTOMXML-SCHEMA");
    let cap = acc.limits.facts;
    let owners = owner_scopes(pkg);
    let owner_map = &xml.graph;
    let package_registry = ownership::PackageRegistryContext::new(owner_map);
    // Schema availability and definitions are tracked per owner (J3): a glossary-only attached
    // schema cannot satisfy a main-owned reference, and an unreadable settings part disables the
    // checks for its owner alone. `None` = that owner's settings could not be established, so its
    // customXml parts are skipped.
    let mut per_owner: Vec<Option<CompleteSet<String>>> = Vec::with_capacity(owners.len());
    // The owners whose customXml item-properties parts exist, so whose schema references would have
    // been checked. A note about a skipped check is only worth recording for such an owner; an owner
    // with no customXml parts loses nothing when the check is skipped (Opus N5). When the manifest
    // that types the parts could not be read the attribution itself is unavailable, so the note is
    // kept.
    let custom_xml_owners: HashSet<OwnerId> = pkg
        .entries
        .iter()
        .filter(|e| !e.is_dir && pkg.content_types.of(&e.name) == Some(package::CUSTOM_XML_PROPS_CONTENT_TYPE))
        .flat_map(|e| package_registry.contexts(&e.name).iter().copied())
        .collect();
    if pkg
        .entries
        .iter()
        .any(|e| !e.is_dir && pkg.content_types.of(&e.name) == Some(package::CUSTOM_XML_PROPS_CONTENT_TYPE) && owner_map.unknown(&e.name))
    {
        acc.note("custom XML ownership could not be established; custom XML schema references were not checked".to_string());
        acc.set_incomplete();
    }
    for (i, owner) in owners.iter().enumerate() {
        let i = OwnerId(i);
        // The settings part is discovered through the owner's own `.rels`, or in the no-owner
        // fallback scope through the manifest (`find_by_type`). When that source could not be read,
        // which settings part (and so which schemas) the owner reaches is UNKNOWN, not absent: skip
        // the owner rather than substituting an empty schema set, which would report a valid schema
        // reference as unattached. Applying the same predicate in both scopes keeps established
        // absence (a readable source that reaches no settings) a genuine empty set. (The run is
        // already INCOMPLETE from loading the unreadable source.)
        if discovery_unavailable(pkg, owner.root()) {
            if custom_xml_owners.contains(&i)
                || (match package::content_unavailable(pkg, "[Content_Types].xml") {
                    Resolution::One(unavailable) => unavailable,
                    Resolution::Absent => false,
                    Resolution::Unknown(_) => false,
                    Resolution::Ambiguous(_) => false,
                })
            {
                let source = match owner.root() {
                    Some(o) => rels_part_name(o),
                    None => "[Content_Types].xml".to_string(),
                };
                acc.note(format!("{source} could not be read; custom XML schema references were not checked"));
            }
            acc.set_incomplete();
            per_owner.push(None);
            continue;
        }
        let selection = supporting_part(pkg, owner.root(), "settings", package::SETTINGS_CONTENT_TYPE, acc);
        if matches!(selection, Resolution::Ambiguous(_) | Resolution::Unknown(_)) {
            per_owner.push(None);
            continue;
        }
        let entry = match selection {
            Resolution::One(entry) => entry,
            Resolution::Ambiguous(_) | Resolution::Unknown(_) => {
                per_owner.push(None);
                continue;
            }
            Resolution::Absent => {
                // The owner's `.rels` was readable and holds no settings relationship: there is
                // genuinely no settings part for this owner, so the attached-schema set is empty.
                per_owner.push(Observed::complete(HashSet::new()).into_complete_set());
                continue;
            }
        };
        if !xml_source_complete(xml, entry) {
            acc.note(format!("{} was not fully examined; custom XML schema references were not checked", entry.name));
            acc.set_incomplete();
            per_owner.push(None);
            continue;
        }
        let Some(text) = entry.decoded.as_deref() else {
            acc.note(format!("{} could not be read; custom XML schema references were not checked", entry.name));
            acc.set_incomplete();
            per_owner.push(None);
            continue;
        };
        let mut guard = scan::ParseGuard::new(acc.limits.events, acc.limits.depth).watching(&["attachedSchema"]);
        let parsed = scan::parse_attached_schemas(text, cap, &mut guard);
        if parsed.is_some() && !guard.exhausted() && guard.unresolved() {
            acc.note(format!(
                "{} has an attached-schema element whose namespace is not resolved; attached-schema absence was not checked",
                entry.name
            ));
            acc.set_incomplete();
            per_owner.push(None);
            continue;
        }
        match parsed {
            Some((vals, capped)) if !guard.exhausted() => {
                if capped {
                    acc.note(format!("{} exceeded the {cap}-entry budget; attached-schema absence was not checked", entry.name));
                    acc.set_incomplete();
                }
                per_owner.push(
                    Observed::with_status(
                        vals.into_iter().collect::<HashSet<_>>(),
                        if capped { Completeness::Incomplete("schema budget") } else { Completeness::Complete },
                    )
                    .into_complete_set(),
                );
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
        let schemas: Vec<_> =
            package_registry.contexts(&e.name).iter().filter_map(|&owner| per_owner.get(owner.0).and_then(Option::as_ref)).collect();
        if schemas.is_empty() {
            continue;
        }
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
            // Unknown package identity still prevents proving that a schema is unattached.
            if schemas.iter().any(|set| set.missing(&uri)) && !acc.absence_suppressed(CUSTOMXML_SCHEMA.id, CUSTOMXML_SCHEMA.severity) {
                acc.note(format!("custom XML part {} declares schema {uri}, which is not attached in settings.xml", e.name));
            }
        }
    }
}

/// Report duplicate identifier findings for a given rule across items.
fn report_dups<K: Eq + std::hash::Hash + std::fmt::Display>(acc: &mut Acc, rule: &'static str, items: &[(K, String)], what: &str) {
    observation_contract! {
        rules: "ID-BOOKMARK-DUP,ID-DOCPR-DUP,ID-REVISION-DUP";
        collections: "type.HashMap*2,type.HashSet*2";
        scope: "intentional package-wide reductions for counts and document-wide duplicates, including unreached stories; definition duplicates remain per definition part";
        loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
        fallback: "retain count/duplicate evidence from observed prefixes; loss remains recorded at each source; owner-local joins assemble their own scope";
        package_loss: "independent";
    }

    let mut first: HashMap<&K, &str> = HashMap::new();
    let mut reported: HashSet<&K> = HashSet::new();
    for (id, part) in items {
        match first.entry(id) {
            Entry::Vacant(v) => {
                v.insert(part.as_str());
            }
            Entry::Occupied(o) => {
                if reported.insert(id) {
                    acc.add(rule, Some(part.clone()), format!("duplicate {what} {id} (first in {})", o.get()));
                }
            }
        }
    }
}

/// Report missing definitions for references collected from story elements.
fn report_missing(acc: &mut Acc, rule: &'static str, refs: &[(DecimalId, String)], defs: &CompleteSet<DecimalId>, what: &str) {
    observation_contract! {
        rules: "ID-COMMENT-REF";
        collections: "access.missing*1,type.CompleteSet*1,type.HashSet*2";
        scope: "source-local bounded extraction; no implicit document owner";
        loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
        fallback: "retain proven prefix presence and duplicates; return loss to the caller; no negative membership from an incomplete set";
        package_loss: "ID-COMMENT-REF";
    }

    let mut reported: HashSet<&DecimalId> = HashSet::new();
    for (id, part) in refs {
        if defs.missing(id) && reported.insert(id) {
            acc.add(rule, Some(part.clone()), format!("{what} {id} in {part} has no definition in the package"));
        }
    }
}

/// Report missing footnote or endnote definitions.
fn report_missing_note(acc: &mut Acc, refs: &[(DecimalId, String)], defs: &CompleteSet<DecimalId>, kind: &str) {
    observation_contract! {
        rules: "REF-NOTE";
        collections: "access.missing*1,type.CompleteSet*1,type.HashSet*2";
        scope: "source-local bounded extraction; no implicit document owner";
        loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
        fallback: "retain proven prefix presence and duplicates; return loss to the caller; no negative membership from an incomplete set";
        package_loss: "REF-NOTE";
    }

    let mut reported: HashSet<&DecimalId> = HashSet::new();
    for (id, part) in refs {
        if defs.missing(id) && reported.insert(id) {
            acc.add(
                "REF-NOTE",
                Some(part.clone()),
                format!("w:{kind}Reference id {id} in {part} has no w:{kind} definition; the note body is absent"),
            );
        }
    }
}

/// Compare the expected media signature and grade mismatches by observed use or decodability.
fn check_media(pkg: &Package, xml: &XmlScan, acc: &mut Acc) {
    observation_contract! {
        rules: "MEDIA-MAGIC";
        collections: "access.missing*1,field.bytes*1,field.content_types*1,field.entries*1,type.Vec*1";
        scope: "existing media candidates and signatures; prove non-consumption from drawing-reference coverage or verify alternate raster bytes with the shared bounded decoder";
        loss: "incomplete relationship or reference discovery, unsupported alternate encoding, or decode budget";
        fallback: "retain the signature Error when neither basis for a Warning is established and explain decoder budget loss, incomplete TIFF directories, or skipped assessment of a recognized encoding";
        package_loss: "independent";
    }

    acc.ran("MEDIA-MAGIC");
    let image_targets = relationship_targets(pkg, "image");
    let mut drawing_targets = None;
    let mut decode_bytes = acc.limits.total_bytes;
    let mut decode_limited_at = None;
    let mut tiff_directory_incomplete_at = None;
    let mut warnings = Vec::new();
    for e in pkg.entries.iter().filter(|e| !e.is_dir) {
        let l = e.name.to_ascii_lowercase();
        if !l.starts_with("word/media/") && !image_targets.contains(&l) {
            continue;
        }
        let Some(bytes) = &e.bytes else { continue };
        let Some(kind) = media_kind(&e.name, pkg.content_types.of(&e.name)) else { continue };
        if magic_matches(kind, bytes) {
            continue;
        }
        let unused = drawing_targets.get_or_insert_with(|| possible_drawing_targets(pkg, xml)).as_ref().is_some_and(|targets| targets.missing(&l));
        let warn = unused
            || match codecs::detect(bytes) {
                Some(format) if !format.caps().read => {
                    acc.note(format!("MEDIA-MAGIC: {} encoding in {} was not assessed because its decoder is unavailable", format.name(), e.name));
                    false
                }
                Some(format) => match decode_alternate_image(format, bytes, acc.limits.part_bytes, &mut decode_bytes) {
                    Ok(AlternateAssessment::Usable) => true,
                    Ok(AlternateAssessment::Unusable) => false,
                    Ok(AlternateAssessment::Unassessed) => {
                        acc.note(format!("MEDIA-MAGIC: {} encoding in {} was not assessed by the validator", format.name(), e.name));
                        false
                    }
                    Ok(AlternateAssessment::IncompleteTiffDirectory) => {
                        tiff_directory_incomplete_at.get_or_insert(&e.name);
                        false
                    }
                    // A budget stop is a lost assessment, never evidence the pixels failed to decode: no verdict
                    // is made from it in either direction (neither the Error nor the Warning grade).
                    Err(CodecError::LimitExceeded(reason)) => {
                        decode_limited_at.get_or_insert((&e.name, reason));
                        continue;
                    }
                    // Compare variants without binding the external enum's named fields.
                    // The observation recognizer only admits patterns that keep those fields opaque.
                    Err(error)
                        if std::mem::discriminant(&error) == std::mem::discriminant(&CodecError::Unsupported { format, reason: String::new() }) =>
                    {
                        acc.note(format!("MEDIA-MAGIC: pixels in {} were not assessed because the codec declined the layout ({error})", e.name));
                        false
                    }
                    Err(_) => {
                        // An adapter can report a refusal as plain malformed data. Name the layouts
                        // that the header alone shows the codec declines by construction.
                        if let Some(reason) = media_boundary::declined_layout(format, media_boundary::coded_image(format, bytes)) {
                            acc.note(format!("MEDIA-MAGIC: pixels in {} were not assessed because the codec declined the layout ({reason})", e.name));
                        }
                        false
                    }
                },
                None => false,
            };
        if warn {
            // Save one extra Warning to exercise the existing overflow path. Emit Errors first
            // so a mismatch that loses a picture cannot be hidden by earlier Warnings.
            if warnings.len() <= acc.limits.findings_per_rule {
                warnings.push((e, kind));
            }
        } else {
            acc.add("MEDIA-MAGIC", Some(e.name.clone()), format!("{} does not have the magic bytes of a {kind} file", e.name));
        }
    }
    if let Some((part, reason)) = decode_limited_at {
        acc.note(format!(
            "MEDIA-MAGIC: alternate image assessment stopped at {part} at a validator bound ({reason}); the pixels there were not assessed, so the signature mismatch was not graded"
        ));
        acc.set_incomplete();
    }
    if let Some(part) = tiff_directory_incomplete_at {
        // Only a malformed or cyclic graph reaches here: every bound of the walk is a lost assessment above.
        acc.note(format!(
            "MEDIA-MAGIC: TIFF directory assessment stopped at {part} because its directories are malformed or cyclic; affected signature mismatches kept their Error grade"
        ));
        acc.set_incomplete();
    }
    for (e, kind) in warnings {
        acc.add_graded(
            MEDIA_MAGIC.id,
            Severity::Warning,
            MEDIA_MAGIC.evidence,
            Some(e.name.clone()),
            format!("{} does not have the magic bytes of a {kind} file", e.name),
        );
    }
}

/// Use only complete, unambiguous story-source reference sets for negative conclusions.
/// References outside the narrow typed-image vocabulary can still consume image data (for
/// example, an extension's embedded image), so every retained relationship ID counts here.
fn possible_drawing_targets(pkg: &Package, xml: &XmlScan) -> Option<CompleteSet<String>> {
    observation_contract! {
        rules: "-";
        collections: "access.into_complete_set*2,access.is_complete*1,access.missing*1,access.prefix*1,field.collapsed_names*1,field.entries*1,field.parts*1,field.references*2,field.rels*2,type.CompleteSet*1,type.HashMap*2,type.HashSet*1,type.Observed*2";
        scope: "package relationship targets possibly consumed by drawings; join complete per-story reference sets by owning relationship part";
        loss: "package relationship discovery incomplete; source missing or unexamined, duplicate owner identity, skipped markup, unmodeled relationship attributes, or reference budget";
        fallback: "unknown owner usage leaves every internal target of its declarations possible; incomplete package discovery withholds non-consumption entirely";
        package_loss: "independent";
    }

    if !ownership_discovery_complete(pkg) || !pkg.collapsed_names.is_empty() {
        return None;
    }
    let mut sources: HashMap<String, Option<&PartFacts>> = HashMap::new();
    for e in pkg.entries.iter().filter(|e| !e.is_dir && !e.name.is_empty()) {
        let facts = xml.parts.get(&PartId::new(&e.name)).filter(|f| f.drawing_refs_complete && f.references.is_complete());
        sources.entry(rels_part_name(&e.name).to_ascii_lowercase()).and_modify(|source| *source = None).or_insert(facts);
    }
    let mut targets = HashSet::new();
    for rp in &pkg.rels {
        let ids = sources
            .get(&rp.name.to_ascii_lowercase())
            .copied()
            .flatten()
            .and_then(|facts| Observed::complete(facts.references.prefix().iter().map(|r| &r.value).collect()).into_complete_set());
        for rel in &rp.rels {
            if let Some(target) = &rel.target_abs
                && !ids.as_ref().is_some_and(|ids| ids.missing(&&rel.id))
            {
                targets.insert(target.to_ascii_lowercase());
            }
        }
    }
    Observed::complete(targets).into_complete_set()
}

enum AlternateAssessment {
    Usable,
    Unusable,
    Unassessed,
    IncompleteTiffDirectory,
}

/// Verify the raster with a bounded decoder and charge the assessment even on failure.
fn decode_alternate_image(format: Format, bytes: &[u8], per_part: u64, remaining: &mut u64) -> Result<AlternateAssessment, CodecError> {
    observation_contract! {
        rules: "-";
        collections: "type.Vec*1";
        scope: "the admitted TIFF directory graph's main chain, within the part byte and metadata limits";
        loss: "directory walk incomplete, or the alternate image decode budget";
        fallback: "leave the image unassessed";
        package_loss: "independent";
    }

    let budget = per_part.min(*remaining);
    let over_budget = || CodecError::LimitExceeded("alternate image decode budget".into());
    let mut preflight = 0u64;
    let mut reserve = |cost: u64| {
        if cost > budget - preflight {
            return Err(over_budget());
        }
        preflight += cost;
        *remaining -= cost;
        Ok(())
    };
    // Reserve before inspecting headers. Keep the debit on any early return.
    let mut main_chain = Vec::new();
    let bytes = if format == Format::Tiff {
        reserve(8192)?;
        let Some((payload, chain)) = tiff_admission::payload(bytes, reserve)? else {
            return Ok(AlternateAssessment::IncompleteTiffDirectory);
        };
        main_chain = chain;
        payload
    } else {
        let factor = match format {
            Format::Jpeg => 4,
            Format::Png | Format::Gif | Format::Bmp | Format::WebP => 2,
            _ => return Ok(AlternateAssessment::Unassessed),
        };
        let payload = media_boundary::coded_image(format, bytes);
        reserve((payload.len() as u64).saturating_mul(factor).max(8192))?;
        payload
    };
    let encoded = bytes.len() as u64;
    let mut tiff_page = None;
    let mut tiff_index = None;
    let (width, height, coefficients) = match format {
        Format::Png => {
            // Read the pixel model without inflating compressed metadata under default limits.
            let model = codecs::png_probe::pixel_model(bytes)?;
            (model.width, model.height, 0)
        }
        Format::Jpeg => {
            let (width, height) = codecs::probe_allocation_size(bytes)?;
            (width, height, codecs::jpeg_worst_case_coefficients(bytes))
        }
        Format::Gif | Format::WebP => {
            // This walks container bounds without asking a decoder to allocate metadata.
            let (width, height) = codecs::probe_allocation_size(bytes)?;
            (width, height, 0)
        }
        Format::Bmp => {
            let (width, height) = codecs::bmp_probe::dimensions(bytes).ok_or(CodecError::UnknownFormat)?;
            (width, height, 0)
        }
        Format::Tiff => {
            let Some((index, page)) = probe_alternate_tiff(bytes, &main_chain)? else {
                return Ok(AlternateAssessment::IncompleteTiffDirectory);
            };
            let dimensions = (page.width, page.height, 0);
            tiff_page = Some(page);
            tiff_index = Some(index);
            dimensions
        }
        _ => return Ok(AlternateAssessment::Unassessed),
    };
    let pixels = u64::from(width) * u64::from(height);
    // Allow one output buffer plus decoder working space, encoded metadata and JPEG coefficients.
    // TIFF also supports wider samples and CMYK with alpha.
    let bytes_per_pixel = if format == Format::Tiff { 20 } else { 8 };
    let allocation = pixels.saturating_mul(bytes_per_pixel).max(encoded).max(4096);
    // A limit of the header inspection is a lost assessment, never a grade.
    let Some(workspace) = media_workspace::decode(format, bytes, width, height, tiff_page.as_ref()) else {
        return Err(CodecError::LimitExceeded("alternate image header inspection limit".into()));
    };
    let decode_charge = allocation.saturating_mul(2).saturating_add(encoded).saturating_add(coefficients).saturating_add(workspace);
    let charge = if format == Format::Tiff { preflight.saturating_add(decode_charge) } else { decode_charge.max(preflight) };
    if charge > budget {
        return Err(over_budget());
    }
    // TIFF decoding rebuilds its directory inventory alongside the raster. Other decoders can
    // reuse the reservation after their header inspection ends.
    *remaining -= charge - preflight;
    if format == Format::Png {
        // The encoded-byte reserve covers raw ancillary chunks alongside pixel working space.
        return decode_png_pixels(bytes, allocation.saturating_add(encoded)).map(|()| AlternateAssessment::Usable);
    }
    let side = width.max(height);
    let options = DecodeOptions { limits: CodecLimits::new(side, side, pixels, allocation.saturating_add(coefficients)), keep_orientation: true };
    // The page that was charged is the page that is decoded.
    let image = match tiff_index {
        Some(index) => codecs::decode_tiff_page(bytes, index, &options)?,
        None => codecs::decode_as_with(format, bytes, &options)?,
    };
    // The codec can return synthesized pixels for a damaged scan. Its complete warning list
    // is available once decoding succeeds, and a repaired scan cannot prove usability here.
    for warning in image.warnings {
        if warning == (DecodeWarning::Truncated { format }) {
            return Ok(AlternateAssessment::Unusable);
        }
    }
    Ok(AlternateAssessment::Usable)
}

/// The page to assess (its index in the codec's listing) and its description, for an image whose directory graph
/// `tiff_admission` already admitted. That graph walk is the only authority for completeness AND for which
/// directories are main-chain pages (`main_chain`: the first directory and each `next` target, in order): the
/// codec's own walk lists a directory once, so one first reached as a SubIFD is listed as a child, and its
/// default page would then depend on the order directories were met in. The page is the first full-resolution
/// directory of the main chain (reduced-resolution copies and masks are skipped), else the first directory; the
/// decoder is asked for exactly that page. A main-chain directory the codec did not list leaves it undecided.
fn probe_alternate_tiff(bytes: &[u8], main_chain: &[u64]) -> Result<Option<(usize, codecs::TiffPage)>, CodecError> {
    observation_contract! {
        rules: "-";
        collections: "field.first*1,type.Vec*1";
        scope: "the admitted TIFF directory graph's main chain against the codec's own page listing";
        loss: "a main-chain directory the codec did not list";
        fallback: "leave the image unassessed";
        package_loss: "independent";
    }

    let directories = codecs::tiff_info(bytes)?;
    let listed: Vec<Option<usize>> = main_chain.iter().map(|&offset| directories.pages.iter().position(|page| page.ifd_offset == offset)).collect();
    // The codec's listing stopped short of a directory the admission walk reached: which page to assess is not
    // established, a lost assessment rather than evidence of a failed decode.
    if listed.iter().any(Option::is_none) {
        return Err(CodecError::LimitExceeded("TIFF page listing".into()));
    }
    let full = |index: &usize| {
        directories.pages.get(*index).is_some_and(|page| page.kind == codecs::TiffPageKind::Page && page.width > 0 && page.height > 0)
    };
    let index = listed.iter().flatten().copied().find(full).or_else(|| listed.first().copied().flatten());
    Ok(index.and_then(|index| directories.pages.into_iter().nth(index).map(|page| (index, page))))
}

/// Check the first PNG image's rows, including Adam7 passes, without inflating text or ICC data.
fn decode_png_pixels(bytes: &[u8], allocation: u64) -> Result<(), CodecError> {
    let map_error = |error: png::DecodingError| match error {
        png::DecodingError::LimitsExceeded => CodecError::LimitExceeded("PNG decoder byte limit".into()),
        error => CodecError::Malformed { format: Format::Png, message: error.to_string() },
    };
    let limits = png::Limits { bytes: usize::try_from(allocation).unwrap_or(usize::MAX) };
    let mut decoder = png::Decoder::new_with_limits(std::io::Cursor::new(bytes), limits);
    decoder.set_ignore_text_chunk(true);
    decoder.set_ignore_iccp_chunk(true);
    decoder.set_transformations(png::Transformations::EXPAND);
    let mut reader = decoder.read_info().map_err(map_error)?;
    while reader.next_row().map_err(map_error)?.is_some() {}
    Ok(())
}

/// Collect lowercased internal target paths for an official relationship kind.
fn relationship_targets(pkg: &Package, kind: &str) -> HashSet<String> {
    observation_contract! {
        rules: "-";
        collections: "field.rels*2,type.HashSet*2";
        scope: "source-local bounded extraction; no implicit document owner";
        loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
        fallback: "retain proven prefix presence and duplicates; return loss to the caller; no negative membership from an incomplete set";
        package_loss: "independent";
    }

    let mut set = HashSet::new();
    for rp in &pkg.rels {
        for r in &rp.rels {
            if r.role.kind() == Some(kind)
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

    /// Exercise document diagnostics on DTD fixtures independently of the prolog policy.
    fn document_check_report(bytes: &[u8]) -> Report {
        let mut acc = Acc::default();
        let pkg = package::load(bytes, &mut acc).unwrap();
        check_document(&pkg, &mut acc);
        acc.finish()
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

    fn media_zip(files: &[(&str, &[u8])], body: &str, kind: &str) -> Vec<u8> {
        let main = package::MAIN_CONTENT_TYPE;
        let types = format!(
            r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="{kind}" ContentType="image/{kind}"/><Override PartName="/word/document.xml" ContentType="{main}"/></Types>"#
        );
        let doc = format!(r#"<w:document xmlns:w="{NS_W}" xmlns:r="{NS_R}" xmlns:wp="{NS_WP}"><w:body>{body}</w:body></w:document>"#);
        let declarations = rels(&format!(
            r#"<Relationship Id="rImg" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="media/drawn.{kind}"/>"#,
        ));
        let mut entries = vec![
            ("[Content_Types].xml", types.into_bytes()),
            ("_rels/.rels", package_rels()),
            ("word/document.xml", doc.into_bytes()),
            ("word/_rels/document.xml.rels", declarations),
        ];
        entries.extend(files.iter().map(|(name, bytes)| (*name, bytes.to_vec())));
        tiny_zip(&entries)
    }

    const DRAWN: &str = r#"<w:p><w:r><w:drawing><wp:inline><wp:extent cx="4000" cy="4000"/><wp:docPr id="1" name="Drawing"/><a:graphic xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/picture"><pic:pic xmlns:pic="http://schemas.openxmlformats.org/drawingml/2006/picture"><pic:blipFill><a:blip r:embed="rImg"/></pic:blipFill></pic:pic></a:graphicData></a:graphic></wp:inline></w:drawing></w:r></w:p>"#;

    #[test]
    fn media_magic_grades_drawn_and_unused_bytes_separately() {
        let jpeg = test_jpeg(4);
        let truncated = &jpeg[..jpeg.len() - 2];
        let stub = b"\xff\xd8\xff\xe0\x00\x00";
        let text = b"not an image";
        let vml = r#"<w:p><w:r><w:pict><v:imagedata xmlns:v="urn:schemas-microsoft-com:vml" xmlns:o="urn:schemas-microsoft-com:office:office" o:relid="rImg"/></w:pict></w:r></w:p>"#;
        let skipped = format!(
            r#"<mc:AlternateContent xmlns:mc="{NS_MC}" xmlns:x="urn:unknown"><mc:Choice Requires="x">{DRAWN}</mc:Choice></mc:AlternateContent>"#
        );
        for (body, bytes, severity) in [
            (DRAWN, jpeg.as_slice(), Severity::Warning),
            (DRAWN, truncated, Severity::Error),
            (DRAWN, stub.as_slice(), Severity::Error),
            (DRAWN, text.as_slice(), Severity::Error),
            ("<w:p/>", stub.as_slice(), Severity::Warning),
            ("<w:p/>", text.as_slice(), Severity::Warning),
            (vml, stub.as_slice(), Severity::Error),
            (&skipped, stub.as_slice(), Severity::Error),
        ] {
            let report = validate_bytes(&media_zip(&[("word/media/drawn.png", bytes)], body, "png"));
            let mut findings = report.findings.iter().filter(|f| f.rule == "MEDIA-MAGIC");
            assert_eq!(findings.next().unwrap().severity, severity, "{report:?}");
            assert!(findings.next().is_none());
            assert_eq!(report.verdict, if severity == Severity::Error { Verdict::Fail } else { Verdict::Pass }, "{report:?}");
        }
    }

    #[test]
    fn media_magic_png_charge_includes_fixed_workspace() {
        let mut bytes = Vec::new();
        let mut encoder = png::Encoder::new(&mut bytes, 32, 32);
        encoder.set_depth(png::BitDepth::One);
        encoder.write_header().unwrap().write_image_data(&[0; 4 * 32]).unwrap();
        let budget = 128 * 1024;
        let mut remaining = budget;
        decode_alternate_image(Format::Png, &bytes, budget, &mut remaining).unwrap();
        assert!(budget - remaining >= (4096 + 512) * 4 + (4 + 1) * 32);
    }

    #[test]
    fn media_magic_apng_debits_overlapping_inflater_state() {
        let mut bytes = Vec::new();
        let mut encoder = png::Encoder::new(&mut bytes, 1, 1);
        encoder.set_animated(1, 0).unwrap();
        encoder.write_header().unwrap().write_image_data(&[128]).unwrap();
        assert_decode_charge(Format::Png, &bytes, 2 * (4096 + 512) * 4 + 2);
    }

    #[test]
    fn media_magic_tiff_debits_deflate_buffer_and_window() {
        let bytes = test_tiff(1, &[], None, 8, b"\x78\x9c\xab\x07\0\0\x80\0\x80");
        assert_decode_charge(Format::Tiff, &bytes, 2 * 32_768 + 1);
    }

    #[test]
    fn media_magic_webp_debits_rounded_vp8_planes() {
        // A single black 1×512 VP8 keyframe in a WebP container.
        let bytes = b"RIFF:\0\0\0WEBPVP8 -\0\0\0\xd0\x03\0\x9d\x01*\x01\0\0\x02\0G\x08\x85\x85\x88\x85\x84\x88\x02\x02\0\x06@\x1e\x19U\xf2\xc0=\xe4\xffF\xd4U|\xb0\x0f@\0\xfe\xff\xabP\x80\0";
        assert_decode_charge(Format::WebP, bytes, 16 * 512 + 2 * 8 * 256 + 3 * 512);
    }

    fn assert_decode_charge(format: Format, bytes: &[u8], minimum: u64) {
        let budget = 1024 * 1024;
        let mut remaining = budget;
        assert!(matches!(decode_alternate_image(format, bytes, budget, &mut remaining), Ok(AlternateAssessment::Usable)));
        assert!(budget - remaining >= minimum, "{format:?} charged {} bytes, expected at least {minimum}", budget - remaining);
    }

    #[test]
    fn media_magic_drawn_png_grade_ignores_compressed_description() {
        for description in [String::new(), "November field notes. ".repeat(650)] {
            let mut bytes = Vec::new();
            let mut encoder = png::Encoder::new(&mut bytes, 32, 32);
            encoder.set_depth(png::BitDepth::One);
            if !description.is_empty() {
                encoder.add_ztxt_chunk("Description".into(), description).unwrap();
            }
            encoder.write_header().unwrap().write_image_data(&[0; 4 * 32]).unwrap();
            let report = validate_bytes(&media_zip(&[("word/media/drawn.jpeg", &bytes)], DRAWN, "jpeg"));
            assert_eq!(report.verdict, Verdict::Pass, "{report:?}");
            assert_eq!(report.findings.len(), 1, "{report:?}");
            assert_eq!((report.findings[0].rule, report.findings[0].severity), ("MEDIA-MAGIC", Severity::Warning));
        }
    }

    fn test_tiff(pages: u32, description: &[u8], sample_format: Option<u32>, compression: u32, strip: &[u8]) -> Vec<u8> {
        let mut bytes = b"II*\0\x08\0\0\0".to_vec();
        let entries = 9 + u32::from(!description.is_empty()) + u32::from(sample_format.is_some());
        let directory_size = 6 + 12 * entries;
        let values = 8 + directory_size * pages;
        let pixels = values + description.len() as u32;
        for page in 0..pages {
            bytes.extend((entries as u16).to_le_bytes());
            for (tag, ty, count, value) in [
                (256u16, 4u16, 1u32, 1u32),
                (257, 4, 1, 1),
                (258, 4, 1, 8),
                (259, 4, 1, compression),
                (262, 4, 1, 1),
                (270, 2, description.len() as u32, values),
                (273, 4, 1, pixels + page * strip.len() as u32),
                (277, 4, 1, 1),
                (278, 4, 1, 1),
                (279, 4, 1, strip.len() as u32),
                (339, 3, 1, sample_format.unwrap_or(0)),
            ] {
                if count == 0 || tag == 339 && sample_format.is_none() {
                    continue;
                }
                bytes.extend(tag.to_le_bytes());
                bytes.extend(ty.to_le_bytes());
                bytes.extend(count.to_le_bytes());
                bytes.extend(value.to_le_bytes());
            }
            let next = if page + 1 == pages { 0 } else { 8 + directory_size * (page + 1) };
            bytes.extend(next.to_le_bytes());
        }
        bytes.extend(description);
        bytes.extend(strip.repeat(pages as usize));
        bytes
    }

    #[test]
    fn media_magic_tiff_directory_budget_is_explicit() {
        let tiff = |pages| test_tiff(pages, &[], None, 1, &[128]);
        let validate = |bytes: &[u8]| validate_bytes(&media_zip(&[("word/media/drawn.png", bytes)], DRAWN, "png"));
        let compact = tiff(4);
        assert_eq!(compact.len(), 468);
        let mut padded = compact.clone();
        padded.extend([0; 128]);
        let report = validate(&compact);
        assert_eq!(report.verdict, Verdict::Pass, "{report:?}");
        assert_eq!(report.findings.len(), 1);
        assert_eq!((report.findings[0].rule, report.findings[0].severity), ("MEDIA-MAGIC", Severity::Warning));
        assert_eq!(serde_json::to_value(report).unwrap(), serde_json::to_value(validate(&padded)).unwrap());

        // A directory bound is a lost assessment, not a failed decode: no grade, and the note names the bound.
        let report = validate(&tiff(1025));
        assert_eq!((report.verdict, report.findings.len()), (Verdict::Incomplete, 0), "{report:?}");
        assert!(report.coverage_notes.iter().any(|note| note.contains("TIFF directory") && note.contains("bound")), "{report:?}");
    }

    /// A classic little-endian TIFF built from explicit directories. A `None` value is
    /// patched with the offset of the directory in the paired slot.
    fn graph_tiff(directories: &[Vec<(u16, u16, Option<usize>, u32)>]) -> Vec<u8> {
        let size = |entries: usize| 6 + 12 * entries as u32;
        let mut offsets = Vec::new();
        let mut at = 8;
        for entries in directories {
            offsets.push(at);
            at += size(entries.len());
        }
        let mut bytes = b"II*\0\x08\0\0\0".to_vec();
        for (index, entries) in directories.iter().enumerate() {
            bytes.extend((entries.len() as u16).to_le_bytes());
            for &(tag, ty, link, value) in entries {
                bytes.extend(tag.to_le_bytes());
                bytes.extend(ty.to_le_bytes());
                bytes.extend(1u32.to_le_bytes());
                bytes.extend(link.map_or(value, |target| offsets[target]).to_le_bytes());
            }
            // Directory chains are explicit `next` entries below; the last field is rewritten.
            let next = entries.iter().find(|entry| entry.0 == 0xffff).and_then(|entry| entry.2).map_or(0, |target| offsets[target]);
            bytes.extend(next.to_le_bytes());
            assert_eq!(bytes.len() as u32, offsets[index] + size(entries.len()));
        }
        bytes.push(0x7f);
        bytes
    }

    #[test]
    fn media_magic_tiff_admission_is_a_function_of_the_directory_graph() {
        // Every graph over three image directories: each has an optional `next` and one optional
        // SubIFD, Exif, GPS or Interop pointer to any directory (shared targets, diamonds, three-page
        // chains with metadata back edges, a directory that is a SubIFD and an Exif target). The oracle
        // is independent of the walk: reachability, then a transitive closure. A directory that reaches
        // itself rejects; every other graph admits, whatever order it is visited in, and both the
        // admission and the report agree.
        let nexts: [Option<usize>; 4] = [None, Some(0), Some(1), Some(2)];
        let pointers: Vec<Option<(u16, usize)>> =
            std::iter::once(None).chain([330u16, 34665, 34853, 40965].into_iter().flat_map(|tag| (0..3).map(move |to| Some((tag, to))))).collect();
        let cyclic = |graph: &[(Option<usize>, Option<(u16, usize)>); 3]| {
            let (mut reached, mut chain) = ([true, false, false], [true, false, false]);
            loop {
                let before = (reached, chain);
                for (at, &(next, pointer)) in graph.iter().enumerate() {
                    if !reached[at] {
                        continue;
                    }
                    if let Some((tag, to)) = pointer {
                        reached[to] = true;
                        chain[to] |= tag == 330;
                    }
                    if let (Some(to), true) = (next, chain[at]) {
                        (reached[to], chain[to]) = (true, true);
                    }
                }
                if (reached, chain) == before {
                    break;
                }
            }
            let mut path = [[false; 3]; 3];
            for (at, &(next, pointer)) in graph.iter().enumerate().filter(|&(at, _)| reached[at]) {
                for to in pointer.map(|(_, to)| to).into_iter().chain(next.filter(|_| chain[at])) {
                    path[at][to] = true;
                }
            }
            for via in 0..3 {
                for from in 0..3 {
                    for to in 0..3 {
                        path[from][to] |= path[from][via] && path[via][to];
                    }
                }
            }
            (0..3).any(|at| reached[at] && path[at][at])
        };
        let node = |next: Option<usize>, pointer: Option<(u16, usize)>| {
            let mut entries = vec![(256u16, 4u16, None, 1u32), (257, 4, None, 1), (258, 4, None, 8), (259, 4, None, 1), (262, 4, None, 1)];
            entries.extend([(273, 4, None, 0), (277, 4, None, 1), (278, 4, None, 1), (279, 4, None, 1)]);
            entries.extend(pointer.map(|(tag, to)| (tag, 4, Some(to), 0)));
            entries.extend(next.map(|to| (0xffff, 4, Some(to), 0)));
            entries
        };
        let build = |graph: &[(Option<usize>, Option<(u16, usize)>); 3]| {
            let mut directories: Vec<_> = graph.iter().map(|&(next, pointer)| node(next, pointer)).collect();
            let strip = 8 + directories.iter().map(|entries| 6 + 12 * entries.len() as u32).sum::<u32>();
            for entry in directories.iter_mut().flatten().filter(|entry| entry.0 == 273) {
                entry.3 = strip;
            }
            graph_tiff(&directories)
        };
        let slots: Vec<_> = nexts.iter().flat_map(|&next| pointers.iter().map(move |&pointer| (next, pointer))).collect();
        let mut rejected = 0usize;
        for index in 0..slots.len().pow(3) {
            let graph = [slots[index / slots.len() / slots.len()], slots[index / slots.len() % slots.len()], slots[index % slots.len()]];
            let bytes = build(&graph);
            let admitted = tiff_admission::payload(&bytes, |_| Ok(())).unwrap().is_some();
            assert_eq!(admitted, !cyclic(&graph), "{graph:?}");
            // The report level is the same function: a sample of every shape.
            if index % 97 == 0 {
                let report = validate_bytes(&media_zip(&[("word/media/drawn.png", &bytes)], DRAWN, "png"));
                let grade = (report.verdict, report.findings.first().map(|f| (f.rule, f.severity)));
                let expected = if admitted { (Verdict::Pass, Severity::Warning) } else { (Verdict::Fail, Severity::Error) };
                assert_eq!(grade, (expected.0, Some(("MEDIA-MAGIC", expected.1))), "{graph:?}: {report:?}");
            }
            rejected += usize::from(!admitted);
        }
        let count = slots.len().pow(3);
        assert!(rejected > 0 && rejected < count, "{rejected} of {count}");
    }

    #[test]
    fn bmp_boundary_comes_from_the_layout_not_the_declared_size() {
        // A Windows-header BMP in RLE8, RLE4 and uncompressed rows, and an OS/2 core-header BMP whose
        // pixel bytes read as nonsense when taken for Windows-header fields. Each is a complete, valid
        // image. Its boundary is the end of the pixel data whatever size it declares (exact, zero, one
        // byte into the pixels, or past them) and whatever follows; a size past the pixels may extend it.
        let le = |value: u32| value.to_le_bytes();
        let windows = |bits: u16, compression: u32, colors: u32, width: i32, pixels: &[u8]| {
            let offset = 14 + 40 + 4 * colors;
            let mut bytes =
                [b"BM".as_slice(), &le(offset + pixels.len() as u32), &[0; 4], &le(offset), &le(40), &width.to_le_bytes(), &le(1)].concat();
            bytes.extend([1, 0, bits as u8, (bits >> 8) as u8]);
            bytes.extend([le(compression), le(pixels.len() as u32), [0; 4], [0; 4], le(colors), le(0)].concat());
            bytes.extend(vec![0; 4 * colors as usize]);
            bytes.extend(pixels);
            bytes
        };
        let core = {
            let mut bytes = [b"BM".as_slice(), &le(34), &[0; 4], &le(26), &le(12), &[2, 0, 1, 0, 1, 0, 24, 0]].concat();
            bytes.extend([0, 0, 0x20, 0, 0, 0, 0, 0]);
            bytes
        };
        let outcome = |bytes: &[u8]| {
            let mut remaining = 1u64 << 20;
            let result = decode_alternate_image(Format::Bmp, bytes, 1 << 20, &mut remaining);
            (result.map(|assessment| matches!(assessment, AlternateAssessment::Usable)).map_err(|error| format!("{error:?}")), remaining)
        };
        for plain in [
            windows(8, 1, 2, 2, &[2, 1, 0, 0, 0, 1]),
            windows(4, 2, 2, 4, &[4, 0x12, 0, 0, 0, 1]),
            windows(24, 0, 0, 2, &[9, 8, 7, 6, 5, 4, 0, 0]),
            core,
        ] {
            assert_eq!(outcome(&plain).0, Ok(true), "{plain:?}");
            for declared in [plain.len(), 0, 30, plain.len() - 1] {
                for trailer in [&b""[..], b"X", b"BM\0\0\0\0", &[0u8; 300]] {
                    let mut bytes = plain.clone();
                    bytes[2..6].copy_from_slice(&le(declared as u32));
                    bytes.extend(trailer);
                    assert_eq!(media_boundary::coded_image(Format::Bmp, &bytes), &bytes[..plain.len()], "{declared} {trailer:?} {plain:?}");
                    assert_eq!(outcome(&bytes), outcome(&bytes[..plain.len()]), "{declared} {trailer:?} {plain:?}");
                }
            }
            // A declared size that reaches past the pixels, into bytes the part holds, is kept.
            let mut longer = plain.clone();
            longer.extend([0; 6]);
            let declared = longer.len() as u32 - 2;
            longer[2..6].copy_from_slice(&le(declared));
            longer.extend([0; 9]);
            assert_eq!(media_boundary::coded_image(Format::Bmp, &longer).len(), plain.len() + 4, "{plain:?}");
        }
        // Overlapping layout: the pixel data starts INSIDE the two-entry colour table the decoder also reads (the
        // command stream ends at byte 58, the table at 62). The boundary covers what the decoder reads, whatever the
        // declared size says, so the image is as decodable as with a declared size of 62.
        let mut overlap = windows(8, 1, 2, 2, &[2, 1, 0, 0, 0, 1]);
        overlap.truncate(62);
        overlap[10..14].copy_from_slice(&le(54));
        overlap[54..58].copy_from_slice(&[2, 1, 0, 1]);
        for declared in [0, 58, 62] {
            overlap[2..6].copy_from_slice(&le(declared));
            let mut bytes = overlap.clone();
            bytes.extend(b"XXXX");
            assert_eq!(media_boundary::coded_image(Format::Bmp, &bytes), &overlap[..], "declared {declared}");
            assert_eq!(outcome(&bytes), outcome(&overlap), "declared {declared}");
            assert_eq!(outcome(&overlap).0, Ok(true), "declared {declared}");
        }
    }

    #[test]
    fn alternate_image_assessment_and_debit_ignore_every_trailer() {
        // The assessment is a function of the coded image. Whatever follows its own boundary, of any
        // content and length, changes neither the outcome nor the budget charged for the image.
        let image = codecs::Image::from_u8(8, 8, codecs::ChannelLayout::Rgb, vec![90; 192]).unwrap();
        let outcome = |format: Format, bytes: &[u8]| {
            let mut remaining = 1u64 << 20;
            let result = match decode_alternate_image(format, bytes, 1 << 20, &mut remaining) {
                Ok(AlternateAssessment::Usable) => "usable".to_string(),
                Ok(AlternateAssessment::Unusable) => "unusable".to_string(),
                Ok(AlternateAssessment::Unassessed) => "unassessed".to_string(),
                Ok(AlternateAssessment::IncompleteTiffDirectory) => "directory".to_string(),
                Err(error) => format!("{error:?}"),
            };
            (result, (1u64 << 20) - remaining)
        };
        let mut state = 0x2545_f491_4f6c_dd1du64;
        let mut random = |len: usize| {
            (0..len)
                .map(|_| {
                    state ^= state << 13;
                    state ^= state >> 7;
                    state ^= state << 17;
                    (state >> 24) as u8
                })
                .collect::<Vec<u8>>()
        };
        let trailers = [
            vec![0],
            vec![b'X'],
            vec![0; 5000],
            [vec![b'X'], vec![0; 700_000]].concat(),
            b"trailing note\n".repeat(60_000),
            random(3000),
            random(600_000),
            [vec![0; 100], b"tail".to_vec(), vec![0; 700_000]].concat(),
            // Bytes that look like the end of an image or the start of another.
            b"\xff\xd9\xff\xd8\xff\xd9".repeat(4000),
            [b"\0\0\0\0IEND\xaeB`\x82".as_slice(), b";", b"GIF89a", b"BM", b"RIFF", b"II*\0"].concat().repeat(1000),
        ];
        let mut formats = Vec::new();
        for format in [Format::Png, Format::Jpeg, Format::Gif, Format::Bmp, Format::WebP, Format::Tiff] {
            if let Ok(bytes) = codecs::encode(&image, format, &codecs::EncodeOptions::default()) {
                formats.push((format, bytes));
            }
        }
        // The synthetic encoders cover PNG, JPEG, BMP and TIFF; these byte fixtures cover GIF and WebP.
        formats.push((
            Format::Gif,
            b"GIF89a\x01\0\x01\0\x80\0\0\0\0\0\xff\xff\xff!\xf9\x04\x01\0\0\0\0,\0\0\0\0\x01\0\x01\0\0\x02\x02D\x01\0;".to_vec(),
        ));
        formats.push((Format::WebP, b"RIFF:\0\0\0WEBPVP8 -\0\0\0\xd0\x03\0\x9d\x01*\x01\0\0\x02\0G\x08\x85\x85\x88\x85\x84\x88\x02\x02\0\x06@\x1e\x19U\xf2\xc0=\xe4\xffF\xd4U|\xb0\x0f@\0\xfe\xff\xabP\x80\0".to_vec()));
        let covered: Vec<Format> = formats.iter().map(|(format, _)| *format).collect();
        for format in [Format::Png, Format::Jpeg, Format::Gif, Format::Bmp, Format::WebP, Format::Tiff] {
            assert!(covered.contains(&format), "no sample for {format:?}");
        }
        for (format, coded) in formats {
            let plain = outcome(format, &coded);
            assert_eq!(plain.0, "usable", "{format:?}: {plain:?}");
            for trailer in &trailers {
                let padded = [coded.as_slice(), trailer.as_slice()].concat();
                assert_eq!(
                    outcome(format, &padded),
                    plain,
                    "{format:?} with a {}-byte trailer starting {:02x?}",
                    trailer.len(),
                    &trailer[..trailer.len().min(4)]
                );
            }
            // Hostile structure never panics and never extends the input: every cut-off, and a
            // corrupted byte at every position, leaves a prefix of the part.
            for cut in 0..=coded.len() {
                let prefix = coded.get(..cut).unwrap_or_default();
                assert!(media_boundary::coded_image(format, prefix).len() <= prefix.len());
                let mut corrupt = coded.clone();
                if let Some(byte) = corrupt.get_mut(cut) {
                    *byte = !*byte;
                    assert!(media_boundary::coded_image(format, &corrupt).len() <= corrupt.len());
                }
            }
        }
    }

    #[test]
    fn coded_image_boundary_skips_marker_lookalikes_inside_segments() {
        // A marker, chunk name or trailer byte inside a length-delimited segment is data.
        let tail = b"\xff\xd9trailing\0\0\0\0IEND\xaeB`\x82;";
        let jpeg = test_jpeg(4);
        let mut spliced = jpeg[..2].to_vec();
        spliced.extend(b"\xff\xe1\x00\x08\xff\xd9\xff\xd9\xff\xd9");
        spliced.extend(&jpeg[2..]);
        let padded = [spliced.as_slice(), tail].concat();
        assert_eq!(media_boundary::coded_image(Format::Jpeg, &padded), spliced.as_slice());

        let image = codecs::Image::from_u8(2, 2, codecs::ChannelLayout::Rgb, vec![9; 12]).unwrap();
        let png = codecs::encode(&image, Format::Png, &codecs::EncodeOptions::default()).unwrap();
        let mut chunk = b"\0\0\0\x0c".to_vec();
        chunk.extend(b"tEXt\0\0\0\0IEND\xaeB`\x82\0\0\0\0");
        let mut spliced = png[..33].to_vec();
        spliced.extend(&chunk);
        spliced.extend(&png[33..]);
        let padded = [spliced.as_slice(), tail].concat();
        assert_eq!(media_boundary::coded_image(Format::Png, &padded), spliced.as_slice());

        let mut gif = b"GIF89a\x01\0\x01\0\0\0\0!\xfe\x03;;;\0,\0\0\0\0\x01\0\x01\0\0\x02\x02D\x01\0;".to_vec();
        let end = gif.len();
        gif.extend(tail);
        assert_eq!(media_boundary::coded_image(Format::Gif, &gif).len(), end);
        // Without a boundary every byte is data.
        assert_eq!(media_boundary::coded_image(Format::Gif, &gif[..end - 1]).len(), end - 1);
    }

    /// A 1×`rows` uncompressed grayscale TIFF with one row per strip.
    fn strips_tiff(rows: u32) -> Vec<u8> {
        let table = 4 * rows;
        let offsets_at = 8 + 2 + 12 * 9 + 4;
        let counts_at = offsets_at + table;
        let pixels = counts_at + table;
        let mut bytes = b"II*\0\x08\0\0\0\x09\0".to_vec();
        for (tag, count, value) in [
            (256u16, 1u32, 1u32),
            (257, 1, rows),
            (258, 1, 8),
            (259, 1, 1),
            (262, 1, 1),
            (273, rows, offsets_at),
            (277, 1, 1),
            (278, 1, 1),
            (279, rows, counts_at),
        ] {
            bytes.extend(tag.to_le_bytes());
            bytes.extend(4u16.to_le_bytes());
            bytes.extend(count.to_le_bytes());
            bytes.extend(value.to_le_bytes());
        }
        bytes.extend(0u32.to_le_bytes());
        bytes.extend((0..rows).flat_map(|row| (pixels + row).to_le_bytes()));
        bytes.extend((0..rows).flat_map(|_| 1u32.to_le_bytes()));
        bytes.extend((0..rows).map(|_| 0x7f));
        bytes
    }

    /// The page the validator assesses, from the admission walk's main chain.
    fn probe_page(bytes: &[u8]) -> codecs::TiffPage {
        let (_, chain) = tiff_admission::payload(bytes, |_| Ok(())).unwrap().unwrap();
        probe_alternate_tiff(bytes, &chain).unwrap().unwrap().1
    }

    #[test]
    fn media_magic_tiff_page_authority_is_main_chain_membership() {
        // A -> B on the main chain; A is a preview (NewSubfileType 1) and B the full-resolution page. B may ALSO be
        // A's SubIFD, which makes the codec's own walk list it as a child of A before the chain reaches it. Drawn
        // either way, the page assessed is B, and each page's assessment is its own: damage to the preview does not
        // fail the image, and damage to the full page is not blessed by a clean preview. A damaged strip points one
        // byte past the only pixel byte.
        let page = |strip: u32, preview: bool, subifd: bool, next: bool| {
            let mut entries = vec![(256u16, 4u16, None, 1u32), (257, 4, None, 1), (258, 3, None, 8), (259, 3, None, 1), (262, 3, None, 1)];
            entries.extend([(273, 4, None, strip), (277, 3, None, 1), (278, 4, None, 1), (279, 4, None, 1)]);
            entries.splice(0..0, preview.then_some((254, 4, None, 1)));
            entries.extend(subifd.then_some((330, 4, Some(1), 0)));
            entries.extend(next.then_some((0xffff, 4, Some(1), 0)));
            entries
        };
        for subifd in [false, true] {
            let layout = |strips: [u32; 2]| graph_tiff(&[page(strips[0], true, subifd, true), page(strips[1], false, false, false)]);
            // The pixel byte is the last byte of the file.
            let pixel = layout([0, 0]).len() as u32 - 1;
            for damaged in [None, Some(0), Some(1)] {
                let strips = [pixel + u32::from(damaged == Some(0)), pixel + u32::from(damaged == Some(1))];
                let bytes = layout(strips);
                let (_, chain) = tiff_admission::payload(&bytes, |_| Ok(())).unwrap().unwrap();
                let (_, selected) = probe_alternate_tiff(&bytes, &chain).unwrap().unwrap();
                assert_eq!(selected.ifd_offset, chain[1], "subifd {subifd}: the full-resolution main-chain page is selected");
                let report = validate_bytes(&media_zip(&[("word/media/drawn.png", &bytes)], DRAWN, "png"));
                let expected = if damaged == Some(1) { (Verdict::Fail, Severity::Error) } else { (Verdict::Pass, Severity::Warning) };
                assert_eq!(
                    (report.verdict, report.findings.first().map(|f| f.severity)),
                    (expected.0, Some(expected.1)),
                    "subifd {subifd}, damaged {damaged:?}: {report:?}"
                );
            }
        }
    }

    #[test]
    fn media_magic_tiff_depth_is_the_minimum_over_routes() {
        // A and B are main-chain pages; B points to the leaf L (depth 1). From A a chain of SubIFD edges leads to
        // D1..Dn. Every directory is within the depth bound by its NEAREST route, so one more edge D10 -> L (which
        // reaches L at depth 11) changes nothing, while a directory whose nearest route is too deep still rejects.
        let node = |sub: Option<usize>, next: Option<usize>| {
            let mut entries = vec![(40000u16, 4u16, None, 0u32)];
            entries.extend(sub.map(|to| (330, 4, Some(to), 0)));
            entries.extend(next.map(|to| (0xffff, 4, Some(to), 0)));
            entries
        };
        let graph = |chain: usize, back: bool| {
            // 0 = A, 1 = B, 2 = L, 3.. = D1..Dn
            let mut directories = vec![node(Some(3), Some(1)), node(Some(2), None), node(None, None)];
            directories.extend((1..=chain).map(|n| node((n < chain).then_some(3 + n), None)));
            if back && let Some(last) = directories.last_mut() {
                last.push((330, 4, Some(2), 0));
            }
            graph_tiff(&directories)
        };
        // The depth bound is a bound of the assessment: past it the graph is not assessed (a lost assessment, never a
        // grade), however valid its pages are.
        for (chain, back, admitted) in [(10, false, true), (10, true, true), (11, false, false), (11, true, false)] {
            let bytes = graph(chain, back);
            match tiff_admission::payload(&bytes, |_| Ok(())) {
                Ok(admission) => assert!(admitted && admission.is_some(), "{chain} edges, back edge {back}"),
                Err(error) => assert!(!admitted && matches!(error, CodecError::LimitExceeded(_)), "{chain} edges, back edge {back}: {error}"),
            }
            let report = validate_bytes(&media_zip(&[("word/media/drawn.png", &bytes)], DRAWN, "png"));
            assert_eq!(report.verdict == Verdict::Incomplete && report.findings.is_empty(), !admitted, "{chain} edges, back edge {back}: {report:?}");
        }
    }

    #[test]
    fn media_magic_tiff_jpeg_worker_is_charged_from_the_jpeg_header_not_the_directory() {
        // zune-jpeg allocates from the JPEG's own header (SOF dimensions, component layout, scan kind), not from the
        // strip or tile size the TIFF directory declares: an 8x8 strip holding a 64x64 progressive JPEG costs what a
        // 64x64 one does, an 8192x8192 tile holding an 8x8 JPEG costs what an 8x8 one does, and a JPEG whose
        // geometry matches the directory (a single-scan baseline strip has no coefficient planes) stays a Pass.
        let header = |marker: u8, side: u16| {
            [&[0xff, 0xd8, 0xff, marker, 0, 11, 8][..], &side.to_be_bytes(), &side.to_be_bytes(), &[1, 1, 0x11, 0, 0xff, 0xd9]].concat()
        };
        let gray = |width: u32, height: u32| {
            let image = codecs::Image::from_u8(width, height, codecs::ChannelLayout::Gray, vec![128; (width * height) as usize]).unwrap();
            codecs::encode(&image, Format::Jpeg, &codecs::EncodeOptions::default()).unwrap()
        };
        // A one-chunk TIFF: a `width` x `height` strip, or (`tile`) a 1x1 page with a `width` x `height` tile.
        let tiff = |width: u32, height: u32, tile: bool, jpeg: &[u8]| {
            let directory = |at: u32| {
                let mut entries = vec![(256u16, 4u16, None, if tile { 1 } else { width }), (257, 4, None, if tile { 1 } else { height })];
                entries.extend([(258, 3, None, 8), (259, 3, None, 7), (262, 3, None, 1)]);
                if tile {
                    entries.extend([
                        (277, 3, None, 1),
                        (322, 4, None, width),
                        (323, 4, None, height),
                        (324, 4, None, at),
                        (325, 4, None, jpeg.len() as u32),
                    ]);
                } else {
                    entries.extend([(273, 4, None, at), (277, 3, None, 1), (278, 4, None, height), (279, 4, None, jpeg.len() as u32)]);
                }
                graph_tiff(&[entries])
            };
            let mut bytes = directory(directory(0).len() as u32 - 1);
            bytes.pop();
            bytes.extend(jpeg);
            bytes
        };
        let debit = |bytes: &[u8]| {
            let page = probe_page(bytes);
            rayon::ThreadPoolBuilder::new()
                .num_threads(1)
                .build()
                .unwrap()
                .install(|| media_workspace::decode(Format::Tiff, bytes, page.width, page.height, Some(&page)).unwrap())
        };
        let (small, large) = (debit(&tiff(8, 8, false, &header(0xc2, 8))), debit(&tiff(8, 8, false, &header(0xc2, 64))));
        // An output byte and a 16-bit coefficient for every sample of the larger frame, however small the strip.
        assert!(large - small >= 3 * (64 * 64 - 8 * 8), "{small} {large}");
        // The coefficient planes are pinned on their own: the same 64x64 frame as a progressive and as a baseline
        // image has the same width state and decoded output, so the difference is the planes (two bytes a sample).
        let baseline = debit(&tiff(8, 8, false, &header(0xc0, 64)));
        assert!(large - baseline >= 2 * 64 * 64, "{baseline} {large}");
        // The marker walk runs to the SOF wherever it lies: a comment pushing it past the first 64 KiB changes nothing
        // (the same bytes, in another order, are charged the same), where a fixed prefix fell back to the directory.
        let comment = [&[0xff, 0xfe][..], &65535u16.to_be_bytes(), &vec![0; 65533]].concat();
        let ordered = |marker: u8, late: bool| {
            let frame = header(marker, 64);
            let (soi, rest) = frame.split_at(2);
            let (frame, eoi) = rest.split_at(rest.len() - 2);
            tiff(8, 8, false, &if late { [soi, &comment, frame, eoi].concat() } else { [soi, frame, &comment, eoi].concat() })
        };
        let (late, early) = (debit(&ordered(0xc2, true)), debit(&ordered(0xc2, false)));
        assert_eq!(late, early, "the header past the 64 KiB mark is read");
        assert!(late - debit(&ordered(0xc0, true)) >= 2 * 64 * 64, "{late}");
        let (read, unread) = (debit(&tiff(8192, 8192, true, &gray(8, 8))), debit(&tiff(8192, 8192, true, b"not a jpeg")));
        assert!(unread - read >= 2 * 8192 * 8192, "{read} {unread}");
        let validate = |bytes: &[u8]| validate_bytes(&media_zip(&[("word/media/drawn.png", bytes)], DRAWN, "png"));
        for bytes in [tiff(8192, 8192, true, &gray(8, 8)), tiff(2048, 3072, false, &gray(2048, 3072))] {
            let report = validate(&bytes);
            assert_eq!((report.verdict, report.findings.first().map(|f| f.severity)), (Verdict::Pass, Some(Severity::Warning)), "{report:?}");
        }
    }

    #[test]
    fn media_magic_tiff_workspace_charges_each_worker_its_decoder_state() {
        // The codec opens one `tiff` decoder per band worker, and each rebuilds the directory and
        // both strip tables (and expands them through 32-byte values on the way). The debit must
        // grow with every worker by at least those tables, at every concurrency the pool allows.
        for rows in [1u32, 100, 10_000] {
            let bytes = strips_tiff(rows);
            let page = probe_page(&bytes);
            assert_eq!((page.width, page.height), (1, rows));
            let in_pool =
                |threads: usize, work: &(dyn Fn() -> u64 + Sync)| rayon::ThreadPoolBuilder::new().num_threads(threads).build().unwrap().install(work);
            let debit = |threads: usize| in_pool(threads, &|| media_workspace::decode(Format::Tiff, &bytes, 1, rows, Some(&page)).unwrap());
            let workers = |threads: usize| threads.min(rows as usize) as u64;
            let one = debit(1);
            for threads in [2usize, 4, 16] {
                let extra = workers(threads) - 1;
                let grown = debit(threads) - one;
                if extra == 0 {
                    assert_eq!(grown, 0);
                    continue;
                }
                assert_eq!(grown % extra, 0, "{rows} rows, {threads} threads: the debit is linear in the worker count");
                // Offsets and byte counts are 8 bytes a strip each, and expand through 32-byte values.
                assert!(grown / extra >= 48 * u64::from(rows), "{rows} rows, {threads} threads: {} bytes per added worker", grown / extra);
            }
            // The report follows the debit: a budget one byte short of it refuses, and the debit passes.
            let exact = in_pool(16, &|| {
                let mut remaining = 1u64 << 32;
                decode_alternate_image(Format::Tiff, &bytes, 1 << 32, &mut remaining).map_or(0, |_| (1u64 << 32) - remaining)
            });
            assert!(exact >= debit(16), "{exact} charged against a {} byte workspace", debit(16));
            let attempt = |budget: u64| {
                in_pool(16, &|| {
                    let mut remaining = budget;
                    u64::from(decode_alternate_image(Format::Tiff, &bytes, budget, &mut remaining).is_ok())
                })
            };
            assert_eq!(attempt(exact - 1), 0, "{rows} rows");
            assert_eq!(attempt(exact), 1, "{rows} rows");
        }
    }

    #[test]
    fn media_magic_tiff_workspace_charges_the_jpeg_strip_decoder_each_worker_holds() {
        // Inside a JPEG strip zune-jpeg holds the decoded output (a byte a sample) and, for a
        // progressive image, 16-bit coefficients for every padded sample: three bytes a sample. Each
        // worker holds them, so the JPEG debit exceeds the same page uncompressed by at least that.
        let tiff = |compression: u32, rows_per_strip: u32| {
            let mut bytes = strips_tiff(1);
            for (at, value) in [(18, 256), (30, 1024), (54, compression), (102, rows_per_strip)] {
                bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
            }
            bytes
        };
        for (rows_per_strip, bands) in [(1024u32, 1usize), (512, 2)] {
            for threads in [1usize, 2] {
                let debit = |compression: u32| {
                    let bytes = tiff(compression, rows_per_strip);
                    let page = probe_page(&bytes);
                    assert_eq!((page.width, page.height, page.compression), (256, 1024, compression as u16));
                    rayon::ThreadPoolBuilder::new()
                        .num_threads(threads)
                        .build()
                        .unwrap()
                        .install(|| media_workspace::decode(Format::Tiff, &bytes, 256, 1024, Some(&page)).unwrap())
                };
                let held = 3 * 256 * u64::from(rows_per_strip) * threads.min(bands) as u64;
                assert!(debit(7) - debit(1) >= held, "{rows_per_strip} rows a strip, {threads} threads: {} < {held}", debit(7) - debit(1));
            }
        }
    }

    #[test]
    fn declined_layout_walkers_mirror_the_decoders_dispatch() {
        let chunk = |tag: &[u8; 4], data: &[u8]| [&tag[..], &(data.len() as u32).to_le_bytes(), data, &[0u8; 1][..data.len() % 2]].concat();
        let riff = |chunks: &[Vec<u8>]| {
            let body = [b"WEBP".to_vec(), chunks.concat()].concat();
            [b"RIFF".to_vec(), (body.len() as u32).to_le_bytes().to_vec(), body].concat()
        };
        let (mut key, mut inter) = (vec![0u8; 10], vec![0u8; 10]);
        inter[0] = 1;
        key[3..6].copy_from_slice(&[0x9d, 0x01, 0x2a]);
        let (vp8_key, vp8_inter, vp8l, junk) =
            (chunk(b"VP8 ", &key), chunk(b"VP8 ", &inter), chunk(b"VP8L", &[0x2f, 0, 0, 0, 0]), chunk(b"JUNK", &[]));
        let canvas = [0u8, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        let (still, animated) = (chunk(b"VP8X", &canvas), chunk(b"VP8X", &[&[2u8][..], &canvas[1..]].concat()));
        let anmf = |frame: &[u8]| chunk(b"ANMF", &[&[0u8; 16][..], frame].concat());
        let named = |bytes: Vec<u8>| media_boundary::declined_layout(Format::WebP, &bytes).is_some();
        // The frame the decoder selects is named: a plain file's first chunk, a still file's only image, the first
        // animation frame's first child (or the chunk after an alpha chunk, decoded as VP8).
        assert!(named(riff(std::slice::from_ref(&vp8_inter))) && !named(riff(std::slice::from_ref(&vp8_key))));
        assert!(named(riff(&[still.clone(), vp8_inter.clone()])));
        assert!(named(riff(&[animated.clone(), chunk(b"ANIM", &[0; 6]), anmf(&vp8_inter)])));
        assert!(named(riff(&[animated.clone(), chunk(b"ANIM", &[0; 6]), anmf(&[chunk(b"ALPH", &[0; 4]), vp8_inter.clone()].concat())])));
        // Nothing else is: a first image that is not VP8 (the later chunk is never decoded), a top-level chunk of an
        // animated file, a first child the decoder rejects, a still file with two images.
        assert!(!named(riff(&[vp8l.clone(), vp8_inter.clone()])));
        assert!(!named(riff(&[still.clone(), vp8l.clone(), vp8_inter.clone()])));
        assert!(!named(riff(&[animated.clone(), vp8_inter.clone()])));
        assert!(!named(riff(&[animated.clone(), chunk(b"ANIM", &[0; 6]), anmf(&vp8l), vp8_inter.clone()])));
        assert!(!named(riff(&[animated, chunk(b"ANIM", &[0; 6]), anmf(&[junk, vp8_inter].concat())])));
        // A frame header names a marker only with its whole declared component table inside the segment.
        let frame = |count: u8, descriptors: usize| {
            [&[0xff, 0xd8, 0xff, 0xc5, 0, 8 + 3 * descriptors as u8, 8, 0, 8, 0, 8, count][..], &[1, 0x11, 0].repeat(descriptors)].concat()
        };
        let named = |bytes: Vec<u8>| media_boundary::declined_layout(Format::Jpeg, &bytes).is_some();
        assert!(named(frame(1, 1)) && named(frame(3, 3)));
        assert!(!named(frame(3, 1)) && !named(frame(0, 1)));
    }

    #[test]
    fn media_magic_names_header_declined_layouts_the_adapter_flattens() {
        let grade = |bytes: &[u8]| {
            let report = validate_bytes(&media_zip(&[("word/media/drawn.png", bytes)], DRAWN, "png"));
            let notes = report.coverage_notes.iter().filter(|note| note.contains("codec declined")).count();
            (report.verdict, report.findings.iter().map(|f| (f.rule, f.severity)).collect::<Vec<_>>(), notes)
        };
        let declined = |notes| (Verdict::Fail, vec![("MEDIA-MAGIC", Severity::Error)], notes);
        // A single black 1x512 VP8 keyframe in a WebP container.
        let keyframe = b"RIFF:\0\0\0WEBPVP8 -\0\0\0\xd0\x03\0\x9d\x01*\x01\0\0\x02\0G\x08\x85\x85\x88\x85\x84\x88\x02\x02\0\x06@\x1e\x19U\xf2\xc0=\xe4\xffF\xd4U|\xb0\x0f@\0\xfe\xff\xabP\x80\0".to_vec();
        assert_eq!(grade(&keyframe), (Verdict::Pass, vec![("MEDIA-MAGIC", Severity::Warning)], 0));
        // The frame tag's low bit marks an interframe, whether the chunk is first, after a VP8X
        // header, or inside an animation frame.
        let mut interframe = keyframe.clone();
        interframe[20] |= 1;
        assert_eq!(grade(&interframe), declined(1));
        // Containers are rebuilt with their declared sizes: a RIFF whose size is stale hides chunks.
        // Every chunk is padded to an even length.
        let even = |chunk: &[u8]| [chunk, &[0u8; 1][..chunk.len() % 2]].concat();
        let (frame, key) = (&even(&interframe[12..])[..], &even(&keyframe[12..])[..]);
        let riff = |chunks: &[&[u8]]| {
            let body = [b"WEBP".as_slice()].iter().chain(chunks).flat_map(|chunk| chunk.iter().copied()).collect::<Vec<u8>>();
            [b"RIFF".as_slice(), &(body.len() as u32).to_le_bytes(), &body].concat()
        };
        let anmf = |frame: &[u8]| {
            [b"ANMF".as_slice(), &(16 + frame.len() as u32).to_le_bytes(), &[0, 0, 0, 0, 0, 0, 0, 0, 0, 0xff, 1, 0, 0, 0, 0, 0], frame].concat()
        };
        // A 1x512 canvas; flag 2 announces an animation.
        let vp8x = |flags: u8| [b"VP8X".as_slice(), &10u32.to_le_bytes(), &[flags, 0, 0, 0, 0, 0, 0, 0xff, 1, 0]].concat();
        let (still, vp8x) = (vp8x(0), vp8x(2));
        let anim = [b"ANIM".as_slice(), &6u32.to_le_bytes(), &[0; 6]].concat();
        assert_eq!(grade(&riff(&[&still, frame])), declined(1));
        assert_eq!(grade(&riff(&[&vp8x, &anim, &anmf(frame)])), declined(1));
        // The first frame of an animation is the one assessed: a later interframe is neither claimed
        // nor denied by the note, and the verdict is the first frame's.
        assert_eq!(grade(&riff(&[&vp8x, &anim, &anmf(key), &anmf(frame)])), (Verdict::Pass, vec![("MEDIA-MAGIC", Severity::Warning)], 0));
        // The note is a function of the bytes the decoder was given: a VP8-looking chunk after the
        // declared RIFF end is padding, and an image-less RIFF is a plain error.
        let empty = riff(&[b"JUNK\x02\0\0\0xx"]);
        assert_eq!(grade(&empty), declined(0));
        assert_eq!(grade(&[empty.clone(), frame.to_vec()].concat()), declined(0));
        // A keyframe whose data is damaged is genuinely malformed, and stays unexplained.
        let mut damaged = keyframe.clone();
        let last = damaged.len() - 1;
        damaged.truncate(last - 20);
        assert_eq!(grade(&damaged), declined(0));
        damaged = keyframe.clone();
        damaged[23] ^= 0xff;
        assert_eq!(grade(&damaged), declined(0));

        // Differential JPEG frames are refused by every backend; the supported frame markers are not.
        let jpeg = test_jpeg(8);
        let at = jpeg.windows(2).position(|pair| pair == [0xff, 0xc0]).unwrap() + 1;
        for marker in 0xc0..=0xcfu8 {
            if matches!(marker, 0xc4 | 0xc8 | 0xcc) {
                continue;
            }
            let mut changed = jpeg.clone();
            changed[at] = marker;
            let (_, _, notes) = grade(&changed);
            // Lossless and arithmetic frames are the codec's own business: it reports its refusals
            // through its unsupported error. This mechanism covers the markers it flattens.
            match marker {
                0xc5..=0xc7 | 0xcd..=0xcf => assert_eq!(notes, 1, "SOF marker {marker:#x}"),
                0xc0..=0xc2 => assert_eq!(notes, 0, "SOF marker {marker:#x}"),
                _ => {}
            }
        }
        let truncated = &jpeg[..jpeg.len() / 2];
        assert_eq!(grade(truncated), declined(0));
        // A frame marker needs its 0xFF introducer: a bare 0xC5 between segments is not one, and a
        // frame marker inside a segment's payload is skipped with it.
        let after_app0 = 4 + usize::from(u16::from_be_bytes([jpeg[4], jpeg[5]]));
        for stray in [0xc5u8, 0xc7, 0xcf] {
            assert_eq!(grade(&[&jpeg[..after_app0], &[stray], &jpeg[after_app0..]].concat()).2, 0, "bare {stray:#x}");
        }
        // A frame header is a whole segment: a bare `FF D8 FF C5` earns no note, a complete one does.
        assert_eq!(grade(b"\xff\xd8\xff\xc5").2, 0);
        assert_eq!(grade(b"\xff\xd8\xff\xc5\x00\x0b\x08\x00\x08\x00\x08\x01\x01\x11\x00").2, 1);
        // The WebP note comes only from the SELECTED frame's complete header. A first frame that is not a VP8 chunk (a
        // truncated VP8L) is the frame that fails whatever a later frame holds, and an animation frame whose body is
        // too short, or holds no image, names nothing: the walk never leaves it for the chunks that follow.
        let vp8l = [b"VP8L".as_slice(), &5u32.to_le_bytes(), &[0x2f, 0, 0xc0, 0x7f, 0], &[0]].concat();
        for later in [key, frame] {
            assert_eq!(grade(&riff(&[&vp8x, &anim, &anmf(&vp8l), &anmf(later)])).2, 0);
        }
        let one_byte_body = [b"ANMF".as_slice(), &1u32.to_le_bytes(), &[0, 0]].concat();
        let no_image = [b"ANMF".as_slice(), &16u32.to_le_bytes(), &[0; 16]].concat();
        for first in [one_byte_body, no_image] {
            assert_eq!(grade(&riff(&[&vp8x, &anim, &first, frame])).2, 0);
        }
    }

    #[test]
    fn media_magic_shared_tiff_values_ignore_padding() {
        let mut description = [b'A'; 256];
        description[255] = 0;
        let compact = test_tiff(4, &description, None, 1, &[0]);
        assert_eq!(compact.len(), 772);
        let mut padded = compact.clone();
        padded.extend([0; 512]);
        let validate = |bytes: &[u8]| validate_bytes(&media_zip(&[("word/media/drawn.png", bytes)], DRAWN, "png"));
        let report = validate(&compact);
        assert_eq!(report.verdict, Verdict::Pass, "{report:?}");
        assert_eq!(report.findings.len(), 1);
        assert_eq!((report.findings[0].rule, report.findings[0].severity), ("MEDIA-MAGIC", Severity::Warning));
        assert_eq!(serde_json::to_value(report).unwrap(), serde_json::to_value(validate(&padded)).unwrap());
    }

    #[test]
    fn media_magic_keeps_error_and_explains_codec_declined_layout() {
        for (sample_format, verdict, severity) in [(1, Verdict::Pass, Severity::Warning), (2, Verdict::Fail, Severity::Error)] {
            let tiff = test_tiff(1, &[], Some(sample_format), 1, &[0x7f]);
            let report = validate_bytes(&media_zip(&[("word/media/drawn.png", &tiff)], DRAWN, "png"));
            assert_eq!(report.verdict, verdict, "{report:?}");
            assert_eq!(report.findings.len(), 1, "{report:?}");
            assert_eq!((report.findings[0].rule, report.findings[0].severity), ("MEDIA-MAGIC", severity));
            assert_eq!(report.coverage_notes.len(), usize::from(sample_format == 2), "{report:?}");
            if sample_format == 2 {
                assert!(report.coverage_notes[0].contains("codec declined") && report.coverage_notes[0].contains("signed"), "{report:?}");
            }
        }
    }

    #[test]
    fn media_magic_distinguishes_decoded_failed_and_unassessed_encodings() {
        let image = codecs::Image::from_u8(10, 10, codecs::ChannelLayout::Rgb, vec![80; 300]).unwrap();
        let tiff = codecs::encode(&image, Format::Tiff, &codecs::EncodeOptions::default()).unwrap();
        for (bytes, verdict, severity, unassessed) in [
            (tiff.as_slice(), Verdict::Pass, Severity::Warning, false),
            (&tiff[..tiff.len() / 2], Verdict::Fail, Severity::Error, false),
            (b"\xff\x0a".as_slice(), Verdict::Fail, Severity::Error, true),
        ] {
            let report = validate_bytes(&media_zip(&[("word/media/drawn.png", bytes)], DRAWN, "png"));
            assert_eq!(report.verdict, verdict, "{report:?}");
            assert_eq!(report.findings.len(), 1, "{report:?}");
            assert_eq!((report.findings[0].rule, report.findings[0].severity), ("MEDIA-MAGIC", severity));
            assert_eq!(report.coverage_notes.len(), usize::from(unassessed), "{report:?}");
            if unassessed {
                assert!(report.coverage_notes[0].contains("JPEG XL") && report.coverage_notes[0].contains("was not assessed"));
            }
        }
    }

    #[test]
    fn media_magic_explains_formats_skipped_by_the_validator() {
        for (bytes, format) in [(b"P6\n1 1\n255\n\xc8((".as_slice(), Format::Pnm), (b"qoif".as_slice(), Format::Qoi)] {
            assert_eq!(codecs::detect(bytes), Some(format));
            assert!(format.caps().read);
            let report = validate_bytes(&media_zip(&[("word/media/drawn.png", bytes)], DRAWN, "png"));
            assert_eq!(report.verdict, Verdict::Fail, "{report:?}");
            assert_eq!(report.findings.len(), 1);
            assert_eq!((report.findings[0].rule, report.findings[0].severity), ("MEDIA-MAGIC", Severity::Error));
            assert_eq!(report.coverage_notes.len(), 1, "{report:?}");
            assert!(report.coverage_notes[0].contains(format.name()) && report.coverage_notes[0].contains("was not assessed"));
        }
    }

    #[test]
    fn media_magic_budget_keeps_errors_ahead_of_warnings_in_both_orders() {
        let used = ("word/media/drawn.png", b"bad".as_slice());
        let unused = ("word/media/unused.png", b"also bad".as_slice());
        let limits = Limits { findings_per_rule: 1, ..Limits::default() };
        for files in [[unused, used], [used, unused]] {
            let report = validate_bytes_limited(&media_zip(&files, DRAWN, "png"), limits);
            assert_eq!(report.verdict, Verdict::Fail);
            assert_eq!(report.findings.len(), 1);
            assert_eq!(report.findings[0].rule, "MEDIA-MAGIC");
            assert_eq!(report.findings[0].part.as_deref(), Some(used.0));
            assert_eq!(report.findings[0].severity, Severity::Error);
            assert!(report.coverage_notes.iter().any(|n| n.starts_with("more than 1 MEDIA-MAGIC")));
        }

        let mut files = vec![unused, used];
        let only_warnings = validate_bytes_limited(&media_zip(&files, "<w:p/>", "png"), limits);
        assert_eq!(only_warnings.verdict, Verdict::Incomplete);
        assert_eq!(only_warnings.findings.len(), 1);
        assert_eq!(only_warnings.findings[0].severity, Severity::Warning);
        files.push(("word/undeclared.blob", b"independent"));
        let mixed = validate_bytes_limited(&media_zip(&files, "<w:p/>", "png"), limits);
        assert_eq!(mixed.verdict, Verdict::Fail);
        assert!(mixed.findings.iter().any(|f| f.rule == "CT-UNDECLARED" && f.severity == Severity::Error));
        assert!(mixed.findings.iter().any(|f| f.rule == "MEDIA-MAGIC" && f.severity == Severity::Warning));
    }

    #[test]
    fn media_magic_webp_preflight_respects_and_charges_decode_budget() {
        let malformed = b"RIFF\x04\0\0\0WEBP";
        for (per_part, mut remaining) in [(1, 64 * 1024), (64 * 1024, 1), (64 * 1024, 0)] {
            let before = remaining;
            assert!(matches!(decode_alternate_image(Format::WebP, malformed, per_part, &mut remaining), Err(CodecError::LimitExceeded(_))));
            assert_eq!(remaining, before);
        }

        let image = codecs::Image::from_u8(2, 2, codecs::ChannelLayout::Rgb, vec![42; 12]).unwrap();
        let webp = codecs::encode(&image, Format::WebP, &codecs::EncodeOptions::default()).unwrap();
        let budget = 1024 * 1024;
        let mut fresh = budget;
        assert!(matches!(decode_alternate_image(Format::WebP, &webp, budget, &mut fresh), Ok(AlternateAssessment::Usable)));
        let cost = budget - fresh;
        let mut remaining = cost + 8192;
        assert!(decode_alternate_image(Format::WebP, malformed, budget, &mut remaining).is_err());
        assert_eq!(remaining, cost, "an admitted failed probe consumes its reservation");
        assert!(matches!(decode_alternate_image(Format::WebP, &webp, budget, &mut remaining), Ok(AlternateAssessment::Usable)));
        assert_eq!(remaining, 0);
        assert!(matches!(decode_alternate_image(Format::WebP, &webp, budget, &mut remaining), Err(CodecError::LimitExceeded(_))));
    }

    #[test]
    fn media_magic_explains_an_alternate_decode_budget_stop() {
        let jpeg = test_jpeg(128);
        assert!(jpeg.len() < 4096);
        let zip = media_zip(&[("word/media/drawn.png", &jpeg)], DRAWN, "png");
        let report = validate_bytes_limited(&zip, Limits { part_bytes: 4096, ..Limits::default() });
        // A budget stop is a lost assessment: neither the Error nor the Warning grade is made from it.
        assert_eq!(report.verdict, Verdict::Incomplete, "{report:?}");
        assert!(report.findings.iter().all(|f| f.rule != "MEDIA-MAGIC"), "{report:?}");
        assert!(report.coverage_notes.iter().any(|n| n.starts_with("MEDIA-MAGIC: alternate image assessment stopped")));
    }

    #[test]
    fn font_dup_name_grades_each_folded_group_once_in_either_order_and_in_a_capped_prefix() {
        for names in [["A", "A", "a"], ["A", "a", "A"]] {
            let elements = names.map(|name| format!(r#"<w:font w:name="{name}"/>"#)).concat();
            let fonts = format!(r#"<w:fonts xmlns:w="{NS_W}">{elements}</w:fonts>"#);
            let part = "word/fontTable.xml";
            let zip = tiny_zip(&[
                ("[Content_Types].xml", content_types_with_override(part, package::FONT_TABLE_CONTENT_TYPE)),
                ("_rels/.rels", package_rels()),
                ("word/document.xml", document()),
                (
                    "word/_rels/document.xml.rels",
                    rels(
                        r#"<Relationship Id="rFonts" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/fontTable" Target="fontTable.xml"/>"#,
                    ),
                ),
                (part, fonts.into_bytes()),
            ]);
            for (facts, verdict) in [(2, Verdict::Incomplete), (3, Verdict::Pass)] {
                let report = validate_bytes_limited(&zip, Limits { facts, ..Limits::default() });
                assert_eq!(report.verdict, verdict, "{report:?}");
                assert!(report.checks_run.contains(&"FONT-DUP-NAME"));
                assert_eq!(report.findings.len(), 1);
                let finding = &report.findings[0];
                assert_eq!(finding.rule, "FONT-DUP-NAME");
                assert_eq!(finding.severity, Severity::Warning);
                assert_eq!(finding.message, format!("font name A appears more than once in {part} (again as {})", names[1]));
                assert_eq!(report.coverage_notes.iter().any(|note| note.contains("later names were not recorded")), facts == 2);
            }
        }
    }

    fn test_jpeg(side: u32) -> Vec<u8> {
        let pixels = vec![128; (side * side * 3) as usize];
        let image = codecs::Image::from_u8(side, side, codecs::ChannelLayout::Rgb, pixels).unwrap();
        codecs::encode(&image, Format::Jpeg, &codecs::EncodeOptions::default()).unwrap()
    }

    #[test]
    fn undeclared_tombstone_requires_the_exact_shape_and_complete_relationships() {
        let binary = vec![0xff; 4];
        let content_part = format!(
            r#"<w:document xmlns:w="{NS_W}" xmlns:r="{NS_R}"><w:body><w:p><w:r><w:contentPart r:id="cp"/></w:r></w:p></w:body></w:document>"#
        );
        let consumed = rels(
            r#"<Relationship Id="cp" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/customXml" Target="../%5Btrash%5D/00af.dat"/>"#,
        );
        for (name, bytes, owner, doc, note) in [
            ("[trash]/00af.dat", binary.clone(), doc_rels(), document(), true),
            ("[trash]/00AF.dat", binary.clone(), doc_rels(), document(), false),
            ("[trash]/0af.dat", binary.clone(), doc_rels(), document(), false),
            ("word/00af.dat", binary.clone(), doc_rels(), document(), false),
            ("[trash]/00af.dat", b"text".to_vec(), doc_rels(), document(), false),
            ("[trash]/00af.dat", vec![0xff; 3], doc_rels(), document(), false),
            ("[trash]/00af.dat", binary.clone(), consumed, content_part.into_bytes(), false),
            ("[trash]/00af.dat", binary, b"<broken".to_vec(), document(), false),
        ] {
            let zip = tiny_zip(&[
                ("[Content_Types].xml", content_types()),
                ("_rels/.rels", package_rels()),
                ("word/document.xml", doc),
                ("word/_rels/document.xml.rels", owner),
                (name, bytes),
            ]);
            let report = validate_bytes(&zip);
            let finding = report.findings.iter().find(|f| f.rule == "CT-UNDECLARED");
            assert_eq!(finding.map(|f| (f.part.as_deref(), f.severity)), (!note).then_some((Some(name), Severity::Error)), "{report:?}");
            assert_eq!(report.coverage_notes.iter().any(|n| n.starts_with("CT-UNDECLARED:")), note, "{report:?}");
            assert_eq!(report.verdict, if note { Verdict::Pass } else { Verdict::Fail });
        }
        let zip = tiny_zip(&[
            ("[Content_Types].xml", content_types_with_override("[trash]/00af.dat", "application/octet-stream")),
            ("_rels/.rels", package_rels()),
            ("word/document.xml", document()),
            ("[trash]/00af.dat", b"text".to_vec()),
        ]);
        assert_eq!(validate_bytes(&zip).verdict, Verdict::Pass);

        let zip = tiny_zip(&[
            ("[Content_Types].xml", content_types()),
            ("_rels/.rels", package_rels()),
            ("word/document.xml", document()),
            ("[trash]/00af.dat", vec![0xff; 1024]),
        ]);
        let report = validate_bytes_limited(&zip, Limits { part_bytes: 512, ..Limits::default() });
        assert_eq!(report.verdict, Verdict::Fail);
        assert!(report.findings.iter().any(|f| f.rule == "CT-UNDECLARED" && f.severity == Severity::Error));
    }

    #[test]
    fn undeclared_tombstone_note_obeys_identity_suppression_and_shared_budget() {
        let entries = [
            ("[Content_Types].xml", content_types()),
            ("_rels/.rels", package_rels()),
            ("word/document.xml", document()),
            ("[trash]/beef.dat", vec![0xff; 4]),
        ];
        let zip = tiny_zip(&entries);
        let pkg = package::load(&zip, &mut Acc::default()).unwrap();
        let mut control = Acc::default();
        check_content_types(&pkg, &mut control);
        assert_eq!(control.coverage, ["CT-UNDECLARED: part [trash]/beef.dat has no effective content type"]);

        let mut unknown = Acc::default();
        unknown.set_unknown_unreadable();
        check_content_types(&pkg, &mut unknown);
        assert!(unknown.coverage.is_empty() && unknown.findings.is_empty());
        assert_eq!(unknown.checks, control.checks);

        let limits = Limits { notes: 1, ..Limits::default() };
        let report = validate_bytes_limited(&zip, limits);
        assert_eq!(report.verdict, Verdict::Incomplete);
        assert_eq!(report.coverage_notes, ["coverage-note budget exhausted; additional explanations omitted"]);
        let mut mixed = Vec::from(entries);
        mixed.push(("undeclared.bin", vec![0]));
        let zip = tiny_zip(&mixed);
        let report = validate_bytes(&zip);
        assert_eq!(report.coverage_notes[0], control.coverage[0]);
        assert!(report.coverage_notes[1].contains("has no relationships part"));
        let capped = validate_bytes_limited(&zip, limits);
        assert_eq!(capped.verdict, Verdict::Fail);
        assert_eq!(capped.findings.len(), 1);
        assert_eq!(capped.findings[0].part.as_deref(), Some("undeclared.bin"));
        assert_eq!(capped.findings[0].severity, Severity::Error);
    }

    #[test]
    fn escaped_opc_namespaces_preserve_ct_inventory() {
        for (manifest, part, bytes, rule, target) in [
            (content_types(), "[trash]/00af.dat", vec![0xff; 4], "CT-UNDECLARED", "../[trash]/00af.dat"),
            (content_types_with_override("word/extra.xml", package::MAIN_CONTENT_TYPE), "word/extra.xml", document(), "CT-DUP-MAIN", "extra.xml"),
        ] {
            for (relationships, verdict) in [
                (
                    rels(&format!(
                        r#"<p:Relationship xmlns:p="http://schemas.openxmlformats.org/package/2&#48;06/relationships" Id="rOne" Type="urn:test" Target="{target}"/>"#
                    )),
                    Verdict::Fail,
                ),
                (String::from_utf8(doc_rels()).unwrap().replace("2006", "2&#48;06").into_bytes(), Verdict::Pass),
            ] {
                let report = validate_bytes(&tiny_zip(&[
                    ("[Content_Types].xml", manifest.clone()),
                    ("_rels/.rels", package_rels()),
                    ("word/document.xml", document()),
                    ("word/_rels/document.xml.rels", relationships),
                    (part, bytes.clone()),
                ]));
                assert_eq!(report.verdict, verdict, "{report:?}");
                assert_eq!(report.findings.iter().any(|f| f.rule == rule && f.severity == Severity::Error), verdict == Verdict::Fail);
                assert_eq!(report.coverage_notes.iter().any(|n| n.starts_with(rule)), verdict == Verdict::Pass);
            }
        }
    }

    #[test]
    fn escaped_opc_namespaces_preserve_drawing_and_manifest_declarations() {
        let uri = "http://schemas.openxmlformats.org/package/2&#48;06";
        let types = String::from_utf8(content_types()).unwrap().replace("http://schemas.openxmlformats.org/package/2006", uri);
        let doc = format!(r#"<w:document xmlns:w="{NS_W}" xmlns:r="{NS_R}" xmlns:wp="{NS_WP}"><w:body>{DRAWN}</w:body></w:document>"#);
        let declarations =
            rels(&format!(r#"<p:Relationship xmlns:p="{uri}/relationships" Id="rImg" Type="{NS_R}/image" Target="media/drawn.png"/>"#));
        let report = validate_bytes(&tiny_zip(&[
            ("[Content_Types].xml", types.replace("</Types>", r#"<Default Extension="png" ContentType="image/png"/></Types>"#).into_bytes()),
            ("_rels/.rels", package_rels()),
            ("word/document.xml", doc.into_bytes()),
            ("word/_rels/document.xml.rels", declarations),
            ("word/media/drawn.png", b"text".to_vec()),
        ]));
        assert_eq!(report.verdict, Verdict::Fail, "{report:?}");
        assert_eq!(report.findings.len(), 1, "{report:?}");
        assert_eq!((report.findings[0].rule, report.findings[0].severity), ("MEDIA-MAGIC", Severity::Error));
    }

    #[test]
    fn escaped_document_namespaces_match_plain_drawing_report() {
        let doc = format!(r#"<w:document xmlns:w="{NS_W}" xmlns:r="{NS_R}" xmlns:wp="{NS_WP}"><w:body>{DRAWN}</w:body></w:document>"#);
        let validate = |document: &str| {
            validate_bytes(&tiny_zip(&[
                ("[Content_Types].xml", content_types_with_override("word/media/drawn.png", "image/png")),
                ("_rels/.rels", package_rels()),
                ("word/document.xml", document.as_bytes().to_vec()),
                ("word/_rels/document.xml.rels", rels(&format!(r#"<Relationship Id="rImg" Type="{NS_R}/image" Target="media/drawn.png"/>"#))),
                ("word/media/drawn.png", b"text".to_vec()),
            ]))
        };
        let plain = validate(&doc);
        assert_eq!(plain.verdict, Verdict::Fail);
        assert_eq!(plain.findings.len(), 1);
        assert_eq!((plain.findings[0].rule, plain.findings[0].severity), ("MEDIA-MAGIC", Severity::Error));
        for ns in [NS_W, NS_R, NS_WP, "http://schemas.openxmlformats.org/drawingml/2006/main"] {
            let escaped = doc.replace(ns, &ns.replace("2006", "2&#48;06"));
            assert_eq!(serde_json::to_value(validate(&escaped)).unwrap(), serde_json::to_value(&plain).unwrap(), "{ns}");
        }
    }

    #[test]
    fn unresolved_namespace_declarations_are_never_bindings() {
        // Every namespace the drawing report depends on is declared with a value that cannot be
        // expanded, spelled several ways. An unresolved value is unknown: it must neither refute a
        // drawing reference (the drawn image stays an Error) nor satisfy or fail the owner root.
        let doc = |declarations: &str, body: &str, doctype: &str| {
            format!(r#"{doctype}<w:document xmlns:w="{NS_W}" xmlns:r="{NS_R}" xmlns:wp="{NS_WP}"{declarations}><w:body>{body}</w:body></w:document>"#)
        };
        let validate = |document: &str| {
            document_check_report(&tiny_zip(&[
                ("[Content_Types].xml", content_types_with_override("word/media/drawn.png", "image/png")),
                ("_rels/.rels", package_rels()),
                ("word/document.xml", document.as_bytes().to_vec()),
                ("word/_rels/document.xml.rels", rels(&format!(r#"<Relationship Id="rImg" Type="{NS_R}/image" Target="media/drawn.png"/>"#))),
                ("word/media/drawn.png", b"text".to_vec()),
            ]))
        };
        let a = "http://schemas.openxmlformats.org/drawingml/2006/main";
        let pic = "http://schemas.openxmlformats.org/drawingml/2006/picture";
        let plain = validate(&doc("", DRAWN, ""));
        assert_eq!((plain.verdict, plain.findings.len()), (Verdict::Fail, 1), "{plain:?}");
        let year = "<!DOCTYPE w:document [<!ENTITY year \"2006\">]>";
        let declared = validate(&doc("", DRAWN, year));
        assert_eq!((declared.verdict, declared.findings.len()), (Verdict::Fail, 1), "{declared:?}");
        for spelling in ["&year;", "&undefined;", "&#xZZ;", "&#x110000;", "2&year;06"] {
            for (uri, owner) in [(NS_W, true), (NS_R, false), (NS_WP, false), (a, false), (pic, false)] {
                let body = DRAWN.replace(&format!(r#"xmlns:a="{a}""#), &format!(r#"xmlns:a="{}""#, if uri == a { spelling } else { a }));
                let body = body.replace(&format!(r#"xmlns:pic="{pic}""#), &format!(r#"xmlns:pic="{}""#, if uri == pic { spelling } else { pic }));
                let mut document = doc("", &body, year);
                if matches!(uri, NS_W | NS_R | NS_WP) {
                    document = document.replace(&format!("=\"{uri}\""), &format!("=\"{spelling}\""));
                }
                let report = validate(&document);
                let context = format!("{uri} as {spelling}: {report:?}");
                // The prefix is declared, only unresolved: it is not reported as undeclared.
                assert!(!report.findings.iter().any(|f| f.rule == "XML-UNDECLARED-PREFIX"), "{context}");
                if owner {
                    assert_eq!(report.verdict, Verdict::Incomplete, "{context}");
                    assert!(
                        report.coverage_notes.iter().any(|note| note.contains("not resolved") && note.contains("ownership is unknown")),
                        "{context}"
                    );
                } else {
                    assert_eq!(report.verdict, Verdict::Fail, "{context}");
                    assert!(report.findings.iter().any(|f| (f.rule, f.severity) == ("MEDIA-MAGIC", Severity::Error)), "{context}");
                }
            }
            // An inner unresolved declaration shadows the real outer binding, and an unresolved
            // default namespace never becomes "no namespace".
            let shadow = doc("", &DRAWN.replace(r#"<a:blip r:embed"#, &format!(r#"<a:blip xmlns:r="{spelling}" r:embed"#)), year);
            let report = validate(&shadow);
            assert!(report.findings.iter().any(|f| (f.rule, f.severity) == ("MEDIA-MAGIC", Severity::Error)), "shadow {spelling}: {report:?}");
            let report = validate(&doc("", &DRAWN.replace(r#"<a:blip r:embed"#, &format!(r#"<a:blip xmlns:r="{spelling}" r:embed"#)), year));
            assert!(!report.findings.iter().any(|f| f.rule == "XML-UNDECLARED-PREFIX"), "shadow {spelling}: {report:?}");
            // An unprefixed root in an unresolved default namespace has unknown ownership.
            let unprefixed = format!(r#"{year}<document xmlns="{spelling}"><body/></document>"#);
            let report = validate(&unprefixed);
            assert_eq!(report.verdict, Verdict::Incomplete, "unprefixed {spelling}: {report:?}");
            assert!(report.coverage_notes.iter().any(|note| note.contains("not resolved") && note.contains("ownership is unknown")), "{report:?}");
            let report = validate(&doc(&format!(r#" xmlns="{spelling}""#), DRAWN, year));
            assert!(report.findings.iter().any(|f| (f.rule, f.severity) == ("MEDIA-MAGIC", Severity::Error)), "default {spelling}: {report:?}");
        }
    }

    #[test]
    fn prefix_use_is_undeclared_only_by_its_nearest_declaration() {
        // `q` is declared with an unexpandable value (`&year;` is not an entity here), a value, or an
        // empty value that un-declares it. Only a use whose NEAREST declaration is missing or empty is
        // reported; the default namespace is never a diagnostic prefix.
        let year = "<!DOCTYPE w:document [<!ENTITY year \"2006\">]>";
        for (body, undeclared) in [
            (r#"<w:p xmlns:q="&year;"><q:a/><z:b/></w:p>"#, "z"),
            (r#"<w:p xmlns:q="&year;"><w:r xmlns:q=""><q:t/></w:r></w:p>"#, "q"),
            (r#"<w:p xmlns:q="&year;"><w:r xmlns:q="" q:a="1"/></w:p>"#, "q"),
            (r#"<w:p xmlns:q="urn:q"><w:r xmlns:q="&year;"><w:t xmlns:q=""><q:t/></w:t></w:r></w:p>"#, "q"),
            (r#"<w:p><z:b/></w:p>"#, "z"),
            (r#"<w:p xmlns:q="&year;"><q:a/></w:p>"#, ""),
            (r#"<w:p xmlns:q="&year;"><w:r xmlns:q="urn:q"><q:t/></w:r></w:p>"#, ""),
            (r#"<w:p xmlns:q="urn:q"><w:r xmlns:q="&year;"><q:t/></w:r></w:p>"#, ""),
            (r#"<w:p xmlns:q="&year;"><w:r xmlns:q=""><w:t xmlns:q="&year;"><q:t/></w:t></w:r></w:p>"#, ""),
            (r#"<w:p xmlns="&year;"><x/><y z="1"/></w:p>"#, ""),
            (r#"<w:p xmlns="&year;"><w:r xmlns="urn:d"><x/></w:r></w:p>"#, ""),
        ] {
            let document = format!(r#"{year}<w:document xmlns:w="{NS_W}"><w:body>{body}</w:body></w:document>"#);
            let report = validate_bytes(&tiny_zip(&[
                ("[Content_Types].xml", content_types()),
                ("_rels/.rels", package_rels()),
                ("word/document.xml", document.into_bytes()),
            ]));
            let reported: Vec<_> = report.findings.iter().filter(|f| f.rule == "XML-UNDECLARED-PREFIX").collect();
            assert_eq!(reported.len(), usize::from(!undeclared.is_empty()), "{body}: {report:?}");
            assert!(reported.iter().all(|f| f.message.contains(&format!("`{undeclared}`"))), "{body}: {report:?}");
        }
    }

    #[test]
    fn unresolved_element_namespaces_never_establish_an_absence() {
        // `u` is the main namespace spelled through an entity this part cannot expand, so a `u:` element
        // may be any main-namespace element. Each shape claims something is absent (outside a
        // paragraph, unpaired, no block content, outside a deletion, no field begin) once it is
        // spelled plainly, and must not once the element that could refute it is unresolved: by its
        // LOCAL name. An unrelated unresolved element (`u:vendor`) is no counterpart of anything, so the
        // claim holds beside it, in the same part and in every story.
        let year = format!("<!DOCTYPE w:document [<!ENTITY ns \"{NS_W}\">]>");
        let validate = |body: &str| {
            let document = format!(r#"{year}<w:document xmlns:w="{NS_W}" xmlns:m="{NS_M}" xmlns:r="{NS_R}"><w:body>{body}</w:body></w:document>"#);
            document_check_report(&tiny_zip(&[
                ("[Content_Types].xml", content_types()),
                ("_rels/.rels", package_rels()),
                ("word/document.xml", document.into_bytes()),
            ]))
        };
        let u = r#"xmlns:u="&ns;""#;
        for (rule, claimed, refuted) in [
            ("MATH-PLACEMENT", "<m:oMath/>".to_string(), format!("<u:p {u}><m:oMath/></u:p>")),
            (
                "BOOKMARK-UNPAIRED",
                r#"<w:p><w:bookmarkStart w:id="1" w:name="b"/></w:p>"#.into(),
                format!(r#"<w:p><w:bookmarkStart w:id="1" w:name="b"/><u:bookmarkEnd {u} w:id="1"/></w:p>"#),
            ),
            (
                "STRUCT-TC-NO-P",
                "<w:tbl><w:tr><w:tc><w:r/></w:tc></w:tr></w:tbl>".into(),
                format!("<w:tbl><w:tr><w:tc><u:p {u}/></w:tc></w:tr></w:tbl>"),
            ),
            ("STRUCT-DELTEXT", "<w:p><w:r><w:delText/></w:r></w:p>".into(), format!("<w:p><u:del {u}><w:r><w:delText/></w:r></u:del></w:p>")),
            (
                "FLD-UNBALANCED",
                r#"<w:p><w:fldChar w:fldCharType="end"/></w:p>"#.into(),
                format!(r#"<w:p><u:fldChar {u} w:fldCharType="begin"/><w:fldChar w:fldCharType="end"/></w:p>"#),
            ),
        ] {
            let claim = validate(&claimed);
            assert!(claim.findings.iter().any(|f| f.rule == rule), "{rule} control: {claim:?}");
            let unknown = validate(&refuted);
            assert!(!unknown.findings.iter().any(|f| f.rule == rule), "{rule}: {unknown:?}");
            let unrelated = validate(&format!("{claimed}<u:vendor {u}/>"));
            assert!(unrelated.findings.iter().any(|f| f.rule == rule), "{rule} beside an unrelated element: {unrelated:?}");
        }
        // A reference whose ID and complete declaration set are both established still fires.
        let hyperlink = validate(&format!(r#"<w:p><w:hyperlink r:id="rAbsent"/></w:p><u:vendor {u}/>"#));
        assert!(hyperlink.findings.iter().any(|f| f.rule == "REL-REF-MISSING"), "{hyperlink:?}");
        // Another story: an unrelated element in the comments part does not hide the dangling reference, and one
        // that may be the definition (`u:comment`) does.
        for (element, reported) in [("u:vendor", true), ("u:comment", false)] {
            let comments = format!(r#"{year}<w:comments xmlns:w="{NS_W}"><w:comment w:id="1"/><{element} {u} w:id="5"/></w:comments>"#);
            let report = validate_bytes(&tiny_zip(&[
                (
                    "[Content_Types].xml",
                    content_types_with_override("word/comments.xml", "application/vnd.openxmlformats-officedocument.wordprocessingml.comments+xml"),
                ),
                ("_rels/.rels", package_rels()),
                (
                    "word/document.xml",
                    format!(r#"<w:document xmlns:w="{NS_W}"><w:body><w:p><w:r><w:commentReference w:id="5"/></w:r></w:p></w:body></w:document>"#)
                        .into_bytes(),
                ),
                ("word/_rels/document.xml.rels", rels(&format!(r#"<Relationship Id="rC" Type="{NS_R}/comments" Target="comments.xml"/>"#))),
                ("word/comments.xml", comments.into_bytes()),
            ]));
            assert_eq!(report.findings.iter().any(|f| f.rule == "ID-COMMENT-REF"), reported, "{element}: {report:?}");
        }
        // An unresolved element never converts a media part between consumed and unused: with no
        // relationship-valued attribute in the story the drawn image stays unused (a Warning, not an Error).
        for body in ["<w:p/>", "<w:p/><u:vendor xmlns:u=\"&ns;\"/>"] {
            let document = format!(r#"{year}<w:document xmlns:w="{NS_W}"><w:body>{body}</w:body></w:document>"#);
            let report = validate_bytes(&tiny_zip(&[
                ("[Content_Types].xml", content_types_with_override("word/media/drawn.png", "image/png")),
                ("_rels/.rels", package_rels()),
                ("word/document.xml", document.into_bytes()),
                ("word/_rels/document.xml.rels", rels(&format!(r#"<Relationship Id="rImg" Type="{NS_R}/image" Target="media/drawn.png"/>"#))),
                ("word/media/drawn.png", b"text".to_vec()),
            ]));
            assert!(report.findings.iter().any(|f| (f.rule, f.severity) == ("MEDIA-MAGIC", Severity::Warning)), "{body}: {report:?}");
        }
        // An unresolved `fldChar` AFTER a known end cannot be its begin: the end is established, and an unresolved
        // one BEFORE it is undecidable (Incomplete, with a note).
        let after = validate(&format!(r#"<w:p><w:fldChar w:fldCharType="end"/><u:fldChar {u} w:fldCharType="begin"/></w:p>"#));
        assert!(after.findings.iter().any(|f| f.rule == "FLD-UNBALANCED"), "{after:?}");
        let before = validate(&format!(r#"<w:p><u:fldChar {u} w:fldCharType="begin"/><w:fldChar w:fldCharType="end"/></w:p>"#));
        assert!(before.coverage_notes.iter().any(|n| n.contains("namespace is not resolved")), "{before:?}");
        assert_eq!(before.verdict, Verdict::Incomplete, "{before:?}");
        // An unresolved `Relationship` element may be a relationship the drawn image uses, so its
        // target is not "missing": the drawn image keeps its Error instead of degrading to a Warning.
        let rel = format!(r#"<Relationship Id="rImg" Type="{NS_R}/image" Target="media/drawn.png" xmlns="&ns;"/>"#);
        let rels_ns = "http://schemas.openxmlformats.org/package/2006/relationships";
        let document = format!(r#"<w:document xmlns:w="{NS_W}" xmlns:r="{NS_R}" xmlns:wp="{NS_WP}"><w:body>{DRAWN}</w:body></w:document>"#);
        for (rels, missing) in [
            (format!(r#"<Relationships xmlns="{rels_ns}"/>"#), true),
            (format!(r#"<!DOCTYPE Relationships [<!ENTITY ns "{rels_ns}">]><Relationships xmlns="{rels_ns}">{rel}</Relationships>"#), false),
        ] {
            let report = validate_bytes(&tiny_zip(&[
                ("[Content_Types].xml", content_types_with_override("word/media/drawn.png", "image/png")),
                ("_rels/.rels", package_rels()),
                ("word/document.xml", document.clone().into_bytes()),
                ("word/_rels/document.xml.rels", rels.into_bytes()),
                ("word/media/drawn.png", b"text".to_vec()),
            ]));
            assert_eq!(report.findings.iter().any(|f| f.rule == "REL-REF-MISSING"), missing, "{report:?}");
            let grade = if missing { Severity::Warning } else { Severity::Error };
            assert!(report.findings.iter().any(|f| (f.rule, f.severity) == ("MEDIA-MAGIC", grade)), "{report:?}");
        }
    }

    #[test]
    fn unresolved_definitions_never_establish_an_absence_in_auxiliary_joins() {
        // `q` is the main namespace with only its year spelled through an entity this part cannot expand, so a `q:`
        // element may be any main-namespace element. Each join claims a definition or consumer is absent. Spelled
        // `w:` the element is there (no finding); spelled `q:` it may be there (no finding, and the run is
        // Incomplete rather than a silent Pass); replaced by an unrelated element the claim is produced.
        let dtd = r#"<!DOCTYPE w:document [<!ENTITY year "2006">]>"#;
        let q = r#"xmlns:q="http://schemas.openxmlformats.org/wordprocessingml/&year;/main""#;
        let wml = "application/vnd.openxmlformats-officedocument.wordprocessingml.";
        // (rule, the element's local name, document body, [(part, content type, relationship kind, template)])
        let cases = [
            (
                "NOTE-PR-LISTING",
                "footnote",
                "<w:p/>",
                vec![
                    (
                        "word/footnotes.xml",
                        "footnotes+xml",
                        "footnotes",
                        r#"<w:footnotes xmlns:w="@W@"><w:footnote w:type="separator" w:id="0"><w:p/></w:footnote></w:footnotes>"#,
                    ),
                    (
                        "word/settings.xml",
                        "settings+xml",
                        "settings",
                        r#"@DTD@<w:settings xmlns:w="@W@" @Q@><w:footnotePr><@E@ w:id="0"/></w:footnotePr></w:settings>"#,
                    ),
                ],
            ),
            ("CT-MISMATCH", "contentPart", r#"<w:p/><@E@ r:id="data" @Q@/>"#, vec![("customXml/item1.xml", "", "customXml", "<root/>")]),
            (
                "REF-STYLE",
                "style",
                r#"<w:p><w:pPr><w:pStyle w:val="A"/></w:pPr></w:p>"#,
                vec![("word/styles.xml", "styles+xml", "styles", r#"@DTD@<w:styles xmlns:w="@W@" @Q@><@E@ w:styleId="A"/></w:styles>"#)],
            ),
            (
                "NUM-DANGLING",
                "num",
                r#"<w:p><w:pPr><w:numPr><w:numId w:val="1"/></w:numPr></w:pPr></w:p>"#,
                vec![(
                    "word/numbering.xml",
                    "numbering+xml",
                    "numbering",
                    r#"@DTD@<w:numbering xmlns:w="@W@" @Q@><w:abstractNum w:abstractNumId="0"/><@E@ w:numId="1"><w:abstractNumId w:val="0"/></@E@></w:numbering>"#,
                )],
            ),
        ];
        for (rule, local, body, parts) in &cases {
            for (prefix, local, claimed, incomplete) in [("w", *local, false, false), ("q", local, false, true), ("w", "vendor", true, false)] {
                let fill = |template: &str| {
                    let (dtd, declaration) = if prefix == "q" { (dtd, q) } else { ("", "") };
                    template.replace("@DTD@", dtd).replace("@Q@", declaration).replace("@W@", NS_W).replace("@E@", &format!("{prefix}:{local}"))
                };
                let mut types = String::new();
                let mut declarations = String::new();
                let mut entries = vec![("_rels/.rels", package_rels())];
                for (part, ct, kind, template) in parts {
                    let ct = if ct.is_empty() { "text/xml".to_string() } else { format!("{wml}{ct}") };
                    types.push_str(&format!(r#"<Override PartName="/{part}" ContentType="{ct}"/>"#));
                    let (id, target) = if *kind == "customXml" { ("data", format!("../{part}")) } else { (*kind, part.replace("word/", "")) };
                    declarations.push_str(&format!(r#"<Relationship Id="{id}" Type="{NS_R}/{kind}" Target="{target}"/>"#));
                    entries.push((*part, fill(template).into_bytes()));
                }
                let document = format!(r#"<w:document xmlns:w="{NS_W}" xmlns:r="{NS_R}"><w:body>{}</w:body></w:document>"#, fill(body));
                let types = format!(
                    r#"<?xml version="1.0"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="{}"/>{types}</Types>"#,
                    package::MAIN_CONTENT_TYPE
                );
                entries.extend([
                    ("[Content_Types].xml", types.into_bytes()),
                    ("word/document.xml", document.into_bytes()),
                    ("word/_rels/document.xml.rels", rels(&declarations)),
                ]);
                let report = document_check_report(&tiny_zip(&entries));
                assert_eq!(report.findings.iter().any(|f| f.rule == *rule), claimed, "{rule} spelled {prefix}:{local}: {report:?}");
                assert_eq!(report.verdict == Verdict::Incomplete, incomplete, "{rule} spelled {prefix}:{local}: {report:?}");
            }
        }
        // A relationship declaration spelled through an internal entity is examined as expat delivers it: the report
        // is the literal spelling's (the drawn image's Error, no missing relationship, no note).
        let declaration = format!(r#"<Relationship Id="rImg" Type="{NS_R}/image" Target="media/drawn.png"/>"#);
        let rels_ns = "http://schemas.openxmlformats.org/package/2006/relationships";
        let document = format!(r#"<w:document xmlns:w="{NS_W}" xmlns:r="{NS_R}" xmlns:wp="{NS_WP}"><w:body>{DRAWN}</w:body></w:document>"#);
        let report = |relationships: String| {
            document_check_report(&tiny_zip(&[
                ("[Content_Types].xml", content_types_with_override("word/media/drawn.png", "image/png")),
                ("_rels/.rels", package_rels()),
                ("word/document.xml", document.clone().into_bytes()),
                ("word/_rels/document.xml.rels", relationships.into_bytes()),
                ("word/media/drawn.png", b"text".to_vec()),
            ]))
        };
        let literal = report(format!(r#"<Relationships xmlns="{rels_ns}">{declaration}</Relationships>"#));
        let entity =
            report(format!(r#"<!DOCTYPE Relationships [<!ENTITY item '{declaration}'>]><Relationships xmlns="{rels_ns}">&item;</Relationships>"#));
        assert!(literal.findings.iter().any(|f| (f.rule, f.severity) == ("MEDIA-MAGIC", Severity::Error)), "{literal:?}");
        assert_eq!(serde_json::to_value(&entity).unwrap(), serde_json::to_value(&literal).unwrap());
    }

    #[test]
    fn attlist_defaults_prevent_expansion_and_preserve_external_mode() {
        // An ATTLIST default is not modeled, so the part is unexamined rather than silently altered.
        // The external relationship with a defaulted TargetMode must not become a dangling internal.
        let rels_ns = "http://schemas.openxmlformats.org/package/2006/relationships";
        let document =
            format!(r#"<w:document xmlns:w="{NS_W}" xmlns:r="{NS_R}"><w:body><w:p><w:r><w:t>Hello</w:t></w:r></w:p></w:body></w:document>"#);
        let report = |relationships: String| {
            document_check_report(&tiny_zip(&[
                ("[Content_Types].xml", content_types()),
                ("_rels/.rels", package_rels()),
                ("word/document.xml", document.clone().into_bytes()),
                ("word/_rels/document.xml.rels", relationships.into_bytes()),
            ]))
        };
        // Literal external relationship passes.
        let literal = report(format!(
            r#"<Relationships xmlns="{rels_ns}"><Relationship Id="rExt" Type="{NS_R}/hyperlink" Target="https://example.com/" TargetMode="External"/></Relationships>"#
        ));
        assert_eq!(literal.verdict, Verdict::Pass, "{literal:?}");
        assert!(!literal.findings.iter().any(|f| f.rule == "REL-DANGLING"), "{literal:?}");
        // Entity-spelled relationship with ATTLIST-defaulted TargetMode is unexamined (Incomplete),
        // not silently converted to internal and then marked dangling.
        let entity_with_attlist = report(format!(
            r#"<!DOCTYPE Relationships [
<!ENTITY item '<Relationship Id="rExt" Type="{NS_R}/hyperlink" Target="https://example.com/"/>'>
<!ATTLIST Relationship TargetMode CDATA "External">
]><Relationships xmlns="{rels_ns}">&item;</Relationships>"#
        ));
        // The ATTLIST makes the subset unresolvable, so expansion does not happen.
        // Without the fix, the TargetMode would be lost and the relationship treated as internal.
        assert!(
            !entity_with_attlist.findings.iter().any(|f| f.rule == "REL-DANGLING"),
            "ATTLIST default should not be lost: {entity_with_attlist:?}"
        );
        assert_eq!(entity_with_attlist.verdict, Verdict::Incomplete, "{entity_with_attlist:?}");
        // The diagnostics read the original source: a second DOCTYPE is still reported after expansion.
        let doubled = report(format!(
            r#"<!DOCTYPE Relationships [<!ENTITY item '<Relationship Id="rExt" Type="{NS_R}/hyperlink" Target="https://example.com/" TargetMode="External"/>'>]><!DOCTYPE Relationships []><Relationships xmlns="{rels_ns}">&item;</Relationships>"#
        ));
        assert!(doubled.findings.iter().any(|f| f.rule == "XML-MALFORMED"), "{doubled:?}");
    }

    #[test]
    fn unexamined_opc_content_yields_no_definite_findings() {
        let rels_ns = "http://schemas.openxmlformats.org/package/2006/relationships";
        let document = format!(
            r#"<w:document xmlns:w="{NS_W}" xmlns:r="{NS_R}"><w:body><w:p><w:hyperlink r:id="rExt"><w:r><w:t>x</w:t></w:r></w:hyperlink></w:p></w:body></w:document>"#
        );
        let ext = format!(r#"<Relationship Id="rExt" Type="{NS_R}/hyperlink" Target="other.docx" TargetMode="External"/>"#);
        let styles = format!(r#"<w:styles xmlns:w="{NS_W}"/>"#);
        let report = |types: Vec<u8>, relationships: String| {
            document_check_report(&tiny_zip(&[
                ("[Content_Types].xml", types),
                ("_rels/.rels", package_rels()),
                ("word/document.xml", document.clone().into_bytes()),
                ("word/_rels/document.xml.rels", relationships.into_bytes()),
                ("word/styles.xml", styles.clone().into_bytes()),
            ]))
        };
        let plain = content_types_with_override(
            "word/styles.xml",
            &format!("{}styles+xml", "application/vnd.openxmlformats-officedocument.wordprocessingml."),
        );
        // The relationship the document names is delivered through an entity this walk does not expand (an inert
        // parameter entity keeps the subset unmodelled): its absence is not established.
        let unmodelled = report(
            plain.clone(),
            format!(r#"<!DOCTYPE Relationships [<!ENTITY % pe "x"><!ENTITY item '{ext}'>]><Relationships xmlns="{rels_ns}">&item;</Relationships>"#),
        );
        // A relationship attribute spelled through an entity is an unknown value, not a missing target.
        let valued = report(
            plain.clone(),
            format!(
                r#"<!DOCTYPE Relationships [<!ENTITY t "nowhere.xml">]><Relationships xmlns="{rels_ns}">{ext}<Relationship Id="rS" Type="{NS_R}/styles" Target="&t;"/></Relationships>"#
            ),
        );
        // A content-types Override delivered through an entity leaves the part's type unknown, not mismatched.
        let types = format!(
            r#"<!DOCTYPE Types [<!ENTITY ov '<Override PartName="/word/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml"/>'>]><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="{}"/>&ov;</Types>"#,
            package::MAIN_CONTENT_TYPE
        );
        let wrapped = report(
            types.into_bytes(),
            format!(r#"<Relationships xmlns="{rels_ns}">{ext}<Relationship Id="rS" Type="{NS_R}/styles" Target="styles.xml"/></Relationships>"#),
        );
        for (name, report, forbidden) in [
            ("unmodelled", &unmodelled, &["REL-REF-MISSING", "REL-DANGLING"][..]),
            ("valued", &valued, &["REL-DANGLING", "REL-REF-MISSING"][..]),
            ("wrapped", &wrapped, &["CT-MISMATCH"][..]),
        ] {
            assert_eq!(report.verdict, Verdict::Incomplete, "{name}: {report:?}");
            assert!(!report.findings.iter().any(|f| forbidden.contains(&f.rule)), "{name}: {report:?}");
        }
    }

    #[test]
    fn an_unresolved_ancestor_decides_only_the_checks_that_depend_on_its_identity() {
        let wml = "application/vnd.openxmlformats-officedocument.wordprocessingml.";
        let dtd = r#"<!DOCTYPE w:document [<!ENTITY year "2006"><!ENTITY mcns "http://schemas.openxmlformats.org/markup-compatibility/2006">]>"#;
        let q = r#"xmlns:q="http://schemas.openxmlformats.org/wordprocessingml/&year;/main""#;
        let ns = format!(
            r#"xmlns:w="{NS_W}" xmlns:r="{NS_R}" xmlns:m="http://schemas.openxmlformats.org/officeDocument/2006/math" xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006" xmlns:w14="http://schemas.microsoft.com/office/word/2010/wordml""#
        );
        let story = |body: &str| {
            document_check_report(&tiny_zip(&[
                ("[Content_Types].xml", content_types()),
                ("_rels/.rels", package_rels()),
                ("word/document.xml", format!("<?xml version=\"1.0\"?>{dtd}<w:document {ns}><w:body>{body}</w:body></w:document>").into_bytes()),
                ("word/_rels/document.xml.rels", doc_rels()),
            ]))
        };
        let settings = |body: &str| {
            document_check_report(&tiny_zip(&[
                ("[Content_Types].xml", content_types_with_override("word/settings.xml", &format!("{wml}settings+xml"))),
                ("_rels/.rels", package_rels()),
                ("word/document.xml", document()),
                ("word/_rels/document.xml.rels", rels(&format!(r#"<Relationship Id="rS" Type="{NS_R}/settings" Target="settings.xml"/>"#))),
                ("word/settings.xml", format!("<?xml version=\"1.0\"?>{dtd}<w:settings xmlns:w=\"{NS_W}\" {q}>{body}</w:settings>").into_bytes()),
            ]))
        };
        let rules = |report: &Report| report.findings.iter().map(|f| f.rule).collect::<Vec<_>>();
        let alt = |attr: &str, fallback: &str| {
            format!(
                r#"<mc:AlternateContent {attr}><mc:Choice Requires="w14"><w:p/></mc:Choice><mc:Fallback>{fallback}</mc:Fallback></mc:AlternateContent>"#
            )
        };
        // An `AlternateContent` of unresolved namespace is the literal one: its cell has block content, and nothing
        // in its Fallback is established (no missing relationship).
        for attr in ["", r#"xmlns:mc="&mcns;""#] {
            let cell = story(&format!("<w:tbl><w:tr><w:tc>{}</w:tc></w:tr></w:tbl>", alt(attr, "<w:p/>")));
            assert_eq!((rules(&cell), cell.verdict), (vec![], Verdict::Pass), "{attr}: {cell:?}");
            let reference = story(&alt(attr, r#"<w:p><w:hyperlink r:id="rAbsent"/></w:p>"#));
            assert_eq!((rules(&reference), reference.verdict), (vec![], Verdict::Pass), "{attr}: {reference:?}");
        }
        // A check whose answer an unresolved element decides is undecided (Incomplete), never silently passed; the same
        // check against a literal foreign spelling is made. An unrelated unresolved wrapper still decides nothing.
        let cell = |tag: &str| story(&format!("<w:tbl><w:tr><w:tc><{tag}/></w:tc></w:tr></w:tbl>"));
        assert_eq!(rules(&cell(r#"q:p xmlns:q="urn:2006""#)), ["STRUCT-TC-NO-P"]);
        assert_eq!(cell(&format!("q:p {q}")).verdict, Verdict::Incomplete);
        let math = |wrapper: &str| story(&format!(r#"<w:p><{wrapper} xmlns:q="urn:&year;"><m:f/></{wrapper}></w:p>"#));
        assert_eq!(rules(&math("q:vendor")), ["MATH-PLACEMENT"]);
        assert_eq!(math("q:num").verdict, Verdict::Incomplete);
        // The same holds where a block level or a deletion is decided by an ancestor: undecided, never silent.
        let deleted = |wrapper: &str| {
            let name = wrapper.split(' ').next().unwrap_or_default();
            story(&format!(r#"<w:p><{wrapper}><w:r><w:delText/></w:r></{name}></w:p>"#))
        };
        assert_eq!(rules(&deleted("q:del xmlns:q=\"urn:2006\"")), ["STRUCT-DELTEXT"]);
        assert_eq!(deleted(&format!("q:del {q}")).verdict, Verdict::Incomplete);
        let sdt = |wrapper: &str| {
            let name = wrapper.split(' ').next().unwrap_or_default();
            story(&format!(r#"<w:p><{wrapper}><w:sdtContent><m:oMath/></w:sdtContent></{name}></w:p>"#))
        };
        assert_eq!(rules(&sdt("q:tbl xmlns:q=\"urn:2006\"")), Vec::<&str>::new());
        assert_eq!(sdt(&format!("q:tbl {q}")).verdict, Verdict::Incomplete);
        let merge = |open: &str, close: &str| settings(&format!("<{open}><w:headerSource/></{close}>"));
        assert_eq!(rules(&merge("w:mailMerge", "w:mailMerge")), ["HEADERSOURCE-PRESENT"]);
        assert_eq!(merge(&format!("q:mailMerge {q}"), "q:mailMerge").verdict, Verdict::Incomplete);
        let range = |open: &str, close: &str| {
            story(&format!(r#"<w:p><w:r><w:pict><{open}><w:p><w:commentRangeStart w:id="1"/></w:p></{close}></w:pict></w:r></w:p>"#))
        };
        assert_eq!(rules(&range("w:txbxContent", "w:txbxContent")), Vec::<&str>::new());
        let hidden = range(&format!("q:txbxContent {q}"), "q:txbxContent");
        assert_eq!((rules(&hidden), hidden.verdict), (vec![], Verdict::Incomplete), "{hidden:?}");
    }

    #[test]
    fn auxiliary_joins_are_decided_per_family_and_per_value() {
        let wml = "application/vnd.openxmlformats-officedocument.wordprocessingml.";
        let dtd = |root: &str| format!(r#"<!DOCTYPE {root} [<!ENTITY year "2006"><!ENTITY a "A"><!ENTITY one "1">]>"#);
        let q = r#"xmlns:q="http://schemas.openxmlformats.org/wordprocessingml/&year;/main""#;
        let package = |body: &str, kind: &str, part: &str, xml: String, extra: Option<(&str, String)>| {
            let mut types = content_types_with_override(&format!("word/{part}.xml"), &format!("{wml}{part}+xml"));
            let mut declarations = format!(r#"<Relationship Id="r1" Type="{NS_R}/{kind}" Target="{part}.xml"/>"#);
            let mut entries = vec![
                ("_rels/.rels", package_rels()),
                ("word/document.xml", format!(r#"<w:document xmlns:w="{NS_W}"><w:body>{body}</w:body></w:document>"#).into_bytes()),
            ];
            if let Some((name, text)) = extra {
                types = String::from_utf8(types)
                    .unwrap()
                    .replace("</Types>", &format!(r#"<Override PartName="/word/{name}.xml" ContentType="{wml}{name}+xml"/></Types>"#))
                    .into_bytes();
                declarations.push_str(&format!(r#"<Relationship Id="r2" Type="{NS_R}/{name}" Target="{name}.xml"/>"#));
                entries.push((if name == "footnotes" { "word/footnotes.xml" } else { "word/settings.xml" }, text.into_bytes()));
            }
            entries.extend([
                ("[Content_Types].xml", types),
                ("word/_rels/document.xml.rels", rels(&declarations)),
                (
                    if part == "styles" {
                        "word/styles.xml"
                    } else if part == "numbering" {
                        "word/numbering.xml"
                    } else {
                        "word/settings.xml"
                    },
                    xml.into_bytes(),
                ),
            ]);
            document_check_report(&tiny_zip(&entries))
        };
        let rules = |report: &Report| report.findings.iter().map(|f| f.rule).collect::<Vec<_>>();
        // A footnote listing is missing (its absence is established); only a footnote-family element could be it.
        let footnotes = format!(r#"<w:footnotes xmlns:w="{NS_W}"><w:footnote w:type="separator" w:id="0"><w:p/></w:footnote></w:footnotes>"#);
        let settings = |inner: &str| format!(r#"<?xml version="1.0"?>{}<w:settings xmlns:w="{NS_W}" {q}>{inner}</w:settings>"#, dtd("w:settings"));
        let listing = |inner: &str| package("<w:p/>", "settings", "settings", settings(inner), Some(("footnotes", footnotes.clone())));
        assert_eq!(rules(&listing("")), ["NOTE-PR-LISTING"]);
        assert_eq!(rules(&listing("<q:endnote/>")), ["NOTE-PR-LISTING"], "an unresolved endnote is nothing to the footnote listing");
        assert_eq!(listing("<q:footnote/>").verdict, Verdict::Incomplete);
        // The parent's spelling and the identifier's spelling are join inputs too: unknown, not absent.
        for inner in [r#"<q:footnotePr><w:footnote w:id="0"/></q:footnotePr>"#, r#"<w:footnotePr><w:footnote q:id="0"/></w:footnotePr>"#] {
            let report = listing(inner);
            assert_eq!((rules(&report), report.verdict), (vec![], Verdict::Incomplete), "{inner}: {report:?}");
        }
        // An identifier spelled through an entity is an unknown definition, in either direction.
        let styles = |style: &str| format!(r#"<?xml version="1.0"?>{}<w:styles xmlns:w="{NS_W}">{style}</w:styles>"#, dtd("w:styles"));
        let used = |value: &str| format!(r#"<w:p><w:pPr><w:pStyle w:val="{value}"/></w:pPr></w:p>"#);
        assert_eq!(rules(&package(&used("A"), "styles", "styles", styles(""), None)), ["REF-STYLE"]);
        let defined = package(&used("A"), "styles", "styles", styles(r#"<w:style w:styleId="&a;"/>"#), None);
        assert_eq!((rules(&defined), defined.verdict), (vec![], Verdict::Incomplete), "{defined:?}");
        let referenced = package(&used("&a;"), "styles", "styles", styles(r#"<w:style w:styleId="B"/>"#), None);
        assert!(!rules(&referenced).contains(&"REF-STYLE"), "{referenced:?}");
        let numbering = |num: &str| format!(r#"<?xml version="1.0"?>{}<w:numbering xmlns:w="{NS_W}">{num}</w:numbering>"#, dtd("w:numbering"));
        let numbered = r#"<w:p><w:pPr><w:numPr><w:numId w:val="1"/></w:numPr></w:pPr></w:p>"#;
        assert_eq!(rules(&package(numbered, "numbering", "numbering", numbering(""), None)), ["NUM-DANGLING"]);
        let defined = package(numbered, "numbering", "numbering", numbering(r#"<w:num w:numId="&one;"><w:abstractNumId w:val="0"/></w:num>"#), None);
        assert_eq!((rules(&defined), defined.verdict), (vec![], Verdict::Incomplete), "{defined:?}");
    }

    #[test]
    fn branch_bookkeeping_keeps_the_established_portion_and_delivered_values_are_values() {
        let ns = format!(
            r#"xmlns:w="{NS_W}" xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006" xmlns:w14="http://schemas.microsoft.com/office/word/2010/wordml""#
        );
        let dtd = r#"<!DOCTYPE w:document [<!ENTITY mcns "http://schemas.openxmlformats.org/markup-compatibility/2006"><!ENTITY u "w14">]>"#;
        let story = |attrs: &str, body: &str| {
            document_check_report(&tiny_zip(&[
                ("[Content_Types].xml", content_types()),
                ("_rels/.rels", package_rels()),
                (
                    "word/document.xml",
                    format!("<?xml version=\"1.0\"?>{dtd}<w:document {ns} {attrs}><w:body>{body}</w:body></w:document>").into_bytes(),
                ),
                ("word/_rels/document.xml.rels", doc_rels()),
            ]))
        };
        let (id, q) = (r#"<w:p w14:paraId="00000012"/>"#, r#"xmlns:q="&mcns;""#);
        let rules = |report: &Report| report.findings.iter().map(|f| f.rule).collect::<Vec<_>>();
        // Outer literal branches prove the copies exclusive whatever an inner wrapper's spelling is.
        let wrapped = story(
            "",
            &format!(
                r#"<mc:AlternateContent><mc:Choice Requires="w14"><q:Choice {q}>{id}</q:Choice></mc:Choice><mc:Fallback>{id}</mc:Fallback></mc:AlternateContent>"#
            ),
        );
        assert_eq!((rules(&wrapped), wrapped.verdict), (vec![], Verdict::Pass), "{wrapped:?}");
        // Two copies in one literal Choice coexist, beneath an unresolved AlternateContent as beneath a literal one.
        let same = story(
            "",
            &format!(r#"<mc:AlternateContent xmlns:mc="&mcns;"><mc:Choice Requires="w14">{id}{id}</mc:Choice><mc:Fallback/></mc:AlternateContent>"#),
        );
        assert_eq!(rules(&same), ["PARAID-DUP"], "{same:?}");
        // A branch element of unresolved namespace excuses nothing by itself: undecided, not a duplicate.
        let unknown =
            story("", &format!(r#"<mc:AlternateContent><q:Choice {q}>{id}</q:Choice><mc:Fallback>{id}</mc:Fallback></mc:AlternateContent>"#));
        assert_eq!((rules(&unknown), unknown.verdict), (vec![], Verdict::Incomplete), "{unknown:?}");
        // `mc:Ignorable="&u;"` is delivered as `w14`, a declared prefix: established, not unread. A delivered value
        // naming an undeclared prefix is reported as such.
        let ignorable = story(r#"mc:Ignorable="&u;""#, "<w:p/>");
        assert_eq!((rules(&ignorable), ignorable.verdict), (vec![], Verdict::Pass), "{ignorable:?}");
        let undeclared = story(r#"mc:Ignorable="&u; zz""#, "<w:p/>");
        assert_eq!(rules(&undeclared), ["XML-UNDECLARED-PREFIX"], "{undeclared:?}");
    }

    #[test]
    fn diagnostics_are_never_made_from_unknowns() {
        // `q` is bound to a namespace spelled through an entity this part cannot expand; `mc` to the markup
        // compatibility namespace through another. A diagnostic that needs the unknown value is not made.
        let mc = "http://schemas.openxmlformats.org/markup-compatibility/2006";
        let entities = format!(r#"<!DOCTYPE w:document [<!ENTITY year "2006"><!ENTITY u "zz"><!ENTITY mcns "{mc}">]>"#);
        let run = |root: &str, body: &str| {
            let text = format!(
                r#"{entities}<w:document xmlns:w="{NS_W}" xmlns:mc="{mc}" xmlns:w14="http://schemas.microsoft.com/office/word/2010/wordml" {root}><w:body>{body}</w:body></w:document>"#
            );
            document_check_report(&tiny_zip(&[
                ("[Content_Types].xml", content_types()),
                ("_rels/.rels", package_rels()),
                ("word/document.xml", text.into_bytes()),
            ]))
        };
        let rules = |report: &Report| report.findings.iter().map(|f| f.rule).collect::<Vec<_>>();
        // The element's own declaration scopes its own name even though collecting its attributes failed.
        let broken = run("", r#"<w:p/><q:vendor xmlns:q="urn:&year;" a="1" a="2"/>"#);
        assert_eq!(rules(&broken), ["XML-MALFORMED"], "{broken:?}");
        // An `mc:Ignorable` value is read as the parser delivers it (`&u;` is `zz`, an undeclared prefix); one it cannot
        // deliver (a replacement with a character reference) is undecided, never a silent pass or a false failure.
        assert_eq!(rules(&run(r#"mc:Ignorable="&u;""#, "<w:p/>")), ["XML-UNDECLARED-PREFIX"]);
        assert_eq!(rules(&run(r#"mc:Ignorable="zz""#, "<w:p/>")), ["XML-UNDECLARED-PREFIX"]);
        // A repeated paraId in a Choice and its Fallback is excused under every spelling of the markup
        // compatibility namespace (undecided where the spelling is unknown), and a repeat in one branch is not.
        let branches = |alt: &str, choice: &str, fallback: &str| {
            let paragraph = r#"<w:p w14:paraId="00000012"/>"#;
            format!(
                r#"<mc:AlternateContent {alt}><mc:Choice {choice} Requires="w14">{paragraph}</mc:Choice><mc:Fallback {fallback}>{paragraph}</mc:Fallback></mc:AlternateContent>"#
            )
        };
        let spelled = r#"xmlns:mc="&mcns;""#;
        let literal = run("", &branches("", "", ""));
        assert!(rules(&literal).is_empty() && literal.verdict == Verdict::Pass, "{literal:?}");
        for body in [branches(spelled, "", ""), branches("", spelled, ""), branches("", "", spelled)] {
            let report = run("", &body);
            assert!(rules(&report).is_empty() && report.verdict == Verdict::Incomplete, "{body}: {report:?}");
        }
        let same = run(
            "",
            r#"<mc:AlternateContent><mc:Choice Requires="w14"><w:p w14:paraId="00000012"/><w:p w14:paraId="00000012"/></mc:Choice></mc:AlternateContent>"#,
        );
        assert_eq!(rules(&same), ["PARAID-DUP"], "{same:?}");
        // A password attribute of unresolved namespace is not a missing one.
        let protection = |attribute: &str| {
            let settings = format!(
                r#"{entities}<w:settings xmlns:w="{NS_W}" xmlns:q="{NS_W}&year;"><w:documentProtection w:algorithmName="SHA-512" w:hashValue="AA==" {attribute}/></w:settings>"#
            );
            document_check_report(&tiny_zip(&[
                (
                    "[Content_Types].xml",
                    content_types_with_override("word/settings.xml", "application/vnd.openxmlformats-officedocument.wordprocessingml.settings+xml"),
                ),
                ("_rels/.rels", package_rels()),
                ("word/document.xml", document()),
                ("word/_rels/document.xml.rels", rels(&format!(r#"<Relationship Id="rS" Type="{NS_R}/settings" Target="settings.xml"/>"#))),
                ("word/settings.xml", settings.into_bytes()),
            ]))
        };
        assert_eq!(rules(&protection("")), ["PROTECTION-PASSWORD"]);
        let unknown = protection(r#"q:spinCount="1""#);
        assert!(rules(&unknown).is_empty() && unknown.verdict == Verdict::Incomplete, "{unknown:?}");
    }

    #[test]
    fn malformed_relationship_sources_withhold_both_ct_demotions() {
        for (manifest, part, bytes, rule, target) in [
            (content_types(), "[trash]/00af.dat", vec![0xff; 4], "CT-UNDECLARED", "../[trash]/00af.dat"),
            (content_types_with_override("word/extra.xml", package::MAIN_CONTENT_TYPE), "word/extra.xml", document(), "CT-DUP-MAIN", "extra.xml"),
        ] {
            let root = String::from_utf8(doc_rels()).unwrap();
            // Legal references and same local names in distinct namespaces preserve inventory.
            let control = validate_bytes(&tiny_zip(&[
                ("[Content_Types].xml", manifest.clone()),
                ("_rels/.rels", package_rels()),
                ("word/document.xml", document()),
                ("word/_rels/document.xml.rels", rels(r#"<vendor xmlns:a="urn:a" xmlns:b="urn:b" a:y="1" b:y="2">&amp;&#48;</vendor>"#)),
                (part, bytes.clone()),
            ]));
            assert_eq!(control.verdict, Verdict::Pass, "{control:?}");
            assert!(control.coverage_notes.iter().any(|note| note.starts_with(rule)));
            let rel_ns = "http://schemas.openxmlformats.org/package/2006/relationships";
            for (relationships, xml_warning) in [
                (format!("x{}", root.split_once("?>").unwrap().1).into_bytes(), true),
                (rels(&format!(r#"<Relationship Type="urn:test" Target="{target}"/>"#)), false),
                (rels("<vendor>&undeclared;</vendor>"), true),
                (rels(r#"<vendor xmlns:a="urn:test" xmlns:b="urn:tes&#116;" a:y="1" b:y="2"/>"#), false),
                (format!(r#"<Relationships xmlns="{rel_ns}" xmlns:a="urn:x" xmlns:b="urn:x" a:y="1" b:y="2"/>"#).into_bytes(), false),
                (rels(&format!(r#"<vendor><Relationship Id="nested" Type="urn:test" Target="{target}"/></vendor>"#)), false),
                (rels(r#"<vendor xmlns:a="urn:x"><other a:y="&undefined;"/></vendor>"#), false),
                (rels(r#"<vendor xmlns:a="urn:x"><other b:y="1"/></vendor>"#), false),
                (rels("<vendor><!-- bad--comment --></vendor>"), false),
                (rels(r#"<vendor xmlns="http://www.w3.org/XML/1998/namespace"/>"#), false),
            ] {
                let report = validate_bytes(&tiny_zip(&[
                    ("[Content_Types].xml", manifest.clone()),
                    ("_rels/.rels", package_rels()),
                    ("word/document.xml", document()),
                    ("word/_rels/document.xml.rels", relationships),
                    (part, bytes.clone()),
                ]));
                assert_eq!(report.verdict, Verdict::Fail, "{report:?}");
                assert!(report.findings.iter().any(|f| f.rule == rule && f.part.as_deref() == Some(part) && f.severity == Severity::Error));
                assert!(!report.coverage_notes.iter().any(|note| note.starts_with(rule)));
                assert_eq!(report.findings.iter().any(|f| f.rule == "XML-MALFORMED" && f.severity == Severity::Warning), xml_warning);
            }
        }
    }

    #[test]
    fn relationship_inventory_requires_separators_in_every_source() {
        for (manifest, part, data, rule) in [
            (content_types(), "[trash]/00af.dat", vec![0xff; 4], "CT-UNDECLARED"),
            (content_types_with_override("word/extra.xml", package::MAIN_CONTENT_TYPE), "word/extra.xml", document(), "CT-DUP-MAIN"),
        ] {
            for source in ["word/_rels/document.xml.rels", "word/_rels/impostor.xml.rels"] {
                for (bytes, clean) in [
                    (rels("<vendor a='1'b='2'/>"), false),
                    (rels("<vendor><nested a = '1'b = \"2\"/></vendor>"), false),
                    (rels("<?x:y z?><vendor/>"), false),
                    (rels("<?xy z?><vendor/>"), true),
                    (rels("<xmlns:vendor/>"), false),
                    (rels("<vendor><xmlns:nested/></vendor>"), false),
                    (rels("<xml:vendor xml:lang='en'/>"), true),
                    (rels("<v xmlns:xml='urn:x'/>"), false),
                    (rels("<v xmlns:xmlns='urn:x'/>"), false),
                    (rels("<v xmlns:a='http://www.w3.org/XML/1998/namespace'/>"), false),
                    (rels("<v xmlns:xml='http://www.w3.org/XML/1998/namespace'/>"), true),
                    (rels("<vendor><nested a = '1'\r\nb = \"2\"/></vendor>"), true),
                    (String::from_utf8(rels("")).unwrap().replace("?>", "encoding='UTF-8'?>").into_bytes(), false),
                ] {
                    let report = validate_bytes(&tiny_zip(&[
                        ("[Content_Types].xml", manifest.clone()),
                        ("_rels/.rels", package_rels()),
                        ("word/document.xml", document()),
                        (source, bytes),
                        (part, data.clone()),
                    ]));
                    assert_eq!(report.findings.iter().any(|f| f.rule == rule && f.severity == Severity::Error), !clean, "{source}: {report:?}");
                    assert_eq!(report.coverage_notes.iter().any(|note| note.starts_with(rule)), clean, "{source}: {report:?}");
                }
            }
        }
    }

    #[test]
    fn unrecognized_relationship_root_withholds_both_ct_demotions() {
        for (manifest, part, bytes, rule) in [
            (content_types(), "[trash]/00af.dat", vec![0xff; 4], "CT-UNDECLARED"),
            (content_types_with_override("word/extra.xml", package::MAIN_CONTENT_TYPE), "word/extra.xml", document(), "CT-DUP-MAIN"),
        ] {
            let report = validate_bytes(&tiny_zip(&[
                ("[Content_Types].xml", manifest),
                ("_rels/.rels", package_rels()),
                ("word/document.xml", document()),
                ("word/_rels/document.xml.rels", b"<settings/>".to_vec()),
                (part, bytes),
            ]));
            assert_eq!(report.verdict, Verdict::Fail, "{report:?}");
            assert!(report.findings.iter().any(|f| f.rule == rule && f.part.as_deref() == Some(part) && f.severity == Severity::Error));
            assert!(!report.coverage_notes.iter().any(|note| note.starts_with(rule)));
        }
    }

    #[test]
    fn dup_main_withholds_inert_classification_when_discovery_is_unknown() {
        let zip = tiny_zip(&[
            ("[Content_Types].xml", content_types_with_override("word/extra.xml", package::MAIN_CONTENT_TYPE)),
            ("_rels/.rels", package_rels()),
            ("word/document.xml", document()),
            ("word/_rels/document.xml.rels", doc_rels()),
            ("word/extra.xml", document()),
        ]);
        let mut pkg = package::load(&zip, &mut Acc::default()).unwrap();
        let mut control = Acc::default();
        check_dup_main(&pkg, &mut control);
        assert_eq!(control.coverage.len(), 1);
        assert!(control.findings.is_empty());

        pkg.rels.iter_mut().find(|r| r.name == "word/_rels/document.xml.rels").unwrap().incomplete = true;
        let mut partial = Acc::default();
        check_dup_main(&pkg, &mut partial);
        assert!(partial.coverage.is_empty());
        assert_eq!(partial.findings.len(), 1);
        assert_eq!(partial.findings[0].severity, Severity::Error);

        pkg.identity_incomplete = true;
        pkg.main = Resolution::Unknown("identity incomplete");
        let mut unknown = Acc::default();
        unknown.set_unknown_unreadable();
        check_dup_main(&pkg, &mut unknown);
        assert!(unknown.coverage.is_empty());
        assert_eq!(unknown.findings.len(), 2);
        assert!(unknown.findings.iter().all(|f| f.severity == Severity::Error && !f.message.contains("relationship reaches")));
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
        let role = scan::PartRole::Main;
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
        let role = scan::PartRole::Main;
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
            ("word/_rels/document.xml.rels", br#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rfootnotes" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/footnotes" Target="footnotes.xml"/><Relationship Id="rsettings" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/settings" Target="settings.xml"/></Relationships>"#.to_vec()),
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
            ("word/_rels/document.xml.rels", br#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rsettings" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/settings" Target="settings3.xml"/></Relationships>"#.to_vec()),
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

    /// Exemptions obey both fact budgets; capped native references without InkML do not change the verdict.
    #[test]
    fn ct_mismatch_contentpart_exemptions_honour_fact_budgets() {
        for (native, target_ct, limited_verdict) in
            [(false, "text/xml", Verdict::Incomplete), (true, "application/inkml+xml", Verdict::Incomplete), (true, "application/xml", Verdict::Pass)]
        {
            for (main_count, header_count) in [(3, 0), (1, 1)] {
                let tag = if native { "w14:contentPart" } else { "w:contentPart" };
                let elements = |count| format!(r#"<{tag} r:id="cp"/>"#).repeat(count);
                let main = format!(
                    r#"<w:document xmlns:w="{NS_W}" xmlns:r="{NS_R}" xmlns:mc="{NS_MC}" xmlns:w14="{NS_W14}"><w:body><w:p><mc:AlternateContent><mc:Fallback>{}</mc:Fallback></mc:AlternateContent></w:p></w:body></w:document>"#,
                    elements(main_count)
                );
                let header = format!(
                    r#"<w:hdr xmlns:w="{NS_W}" xmlns:r="{NS_R}" xmlns:mc="{NS_MC}" xmlns:w14="{NS_W14}"><w:p><mc:AlternateContent><mc:Fallback>{}</mc:Fallback></mc:AlternateContent></w:p></w:hdr>"#,
                    elements(header_count)
                );
                let cp_rels = rels(
                    r#"<Relationship Id="cp" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/customXml" Target="data.xml"/>"#,
                );
                let ct = String::from_utf8(content_types_with_override("word/data.xml", target_ct)).unwrap().replace(
                    "</Types>",
                    &format!(r#"<Override PartName="/word/header.xml" ContentType="{}"/></Types>"#, package::HEADER_CONTENT_TYPE),
                );
                let bytes = tiny_zip(&[
                    ("[Content_Types].xml", ct.into_bytes()),
                    ("_rels/.rels", package_rels()),
                    ("word/document.xml", main.into_bytes()),
                    ("word/_rels/document.xml.rels", cp_rels.clone()),
                    ("word/header.xml", header.into_bytes()),
                    ("word/_rels/header.xml.rels", if header_count > 0 { cp_rels } else { doc_rels() }),
                    ("word/data.xml", b"<root/>".to_vec()),
                ]);
                let report = validate_bytes_limited(&bytes, Limits { facts: 1, ..Limits::default() });
                assert_eq!(report.verdict, limited_verdict, "{main_count}/{header_count}: {:?}", report.findings);
                assert!(report.findings.is_empty(), "{:?}", report.findings);
                assert_eq!(
                    report.coverage_notes.iter().any(|n| n.contains("contentPart") && n.contains("fact budget")),
                    limited_verdict == Verdict::Incomplete,
                    "{:?}",
                    report.coverage_notes
                );
                assert_eq!(validate_bytes(&bytes).verdict, Verdict::Pass);
            }
        }
    }

    /// The #5982 exemption is per-owner: when the MAIN part's
    /// contentPart collection caps out, a different owner whose
    /// collection completed (here a header with no contentParts at
    /// all) still gets its customXml pairing judged.
    #[test]
    fn ct_mismatch_survives_another_owners_capped_contentpart_collection() {
        let ct = format!(
            r#"<?xml version="1.0"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="{}"/><Override PartName="/word/header1.xml" ContentType="{}"/><Override PartName="/word/data/bad.xml" ContentType="text/xml"/></Types>"#,
            package::MAIN_CONTENT_TYPE,
            package::HEADER_CONTENT_TYPE
        );
        let elems: String = (0..6).map(|i| format!(r#"<w:p><w:r><w:contentPart r:id="cp{i}"/></w:r></w:p>"#)).collect();
        let doc = format!(
            r#"<?xml version="1.0"?><w:document xmlns:w="{NS_W}" xmlns:r="{NS_R}"><w:body>{elems}<w:sectPr><w:headerReference w:type="default" r:id="rIdH"/></w:sectPr></w:body></w:document>"#
        )
        .into_bytes();
        let mut rel_body: String = (0..6)
            .map(|i| format!(r#"<Relationship Id="cp{i}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/customXml" Target="data.xml"/>"#))
            .collect();
        rel_body.push_str(
            r#"<Relationship Id="rIdH" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/header" Target="header1.xml"/>"#,
        );
        let doc_rels = rels(&rel_body);
        let header = format!(r#"<?xml version="1.0"?><w:hdr xmlns:w="{NS_W}"><w:p/></w:hdr>"#).into_bytes();
        let header_rels = rels(
            r#"<Relationship Id="rIdX" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/customXml" Target="data/bad.xml"/>"#,
        );
        let bytes = tiny_zip(&[
            ("[Content_Types].xml", ct.into_bytes()),
            ("_rels/.rels", package_rels()),
            ("word/document.xml", doc),
            ("word/_rels/document.xml.rels", doc_rels),
            ("word/header1.xml", header),
            ("word/_rels/header1.xml.rels", header_rels),
            ("word/data/bad.xml", b"<root/>".to_vec()),
            ("word/data.xml", b"<root/>".to_vec()),
        ]);
        let report = validate_bytes_limited(&bytes, Limits { facts: 4, ..Limits::default() });
        let f = report
            .findings
            .iter()
            .find(|f| f.rule == "CT-MISMATCH")
            .unwrap_or_else(|| panic!("CT-MISMATCH was suppressed by another owner's cap; findings: {:?}", report.findings));
        assert_eq!(f.part.as_deref(), Some("word/data/bad.xml"));
        assert!(
            report.coverage_notes.iter().any(|n| n.contains("contentPart")),
            "the capped collection must still be disclosed; notes: {:?}",
            report.coverage_notes
        );
    }

    /// An owner that was never scanned (here the main part exceeds the
    /// per-part byte budget) has unestablished contentPart consumption:
    /// the customXml pairing must not fire for its relationships.
    #[test]
    fn ct_mismatch_skips_an_unscanned_owner() {
        let padding = "x".repeat(5000);
        let doc = format!(
            r#"<?xml version="1.0"?><w:document xmlns:w="{NS_W}" xmlns:r="{NS_R}"><w:body><w:p><w:r><w:contentPart r:id="cp"/></w:r></w:p><!-- {padding} --></w:body></w:document>"#
        )
        .into_bytes();
        let doc_rels =
            rels(r#"<Relationship Id="cp" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/customXml" Target="data.xml"/>"#);
        let bytes = tiny_zip(&[
            ("[Content_Types].xml", content_types_with_override("word/data.xml", "text/xml")),
            ("_rels/.rels", package_rels()),
            ("word/document.xml", doc),
            ("word/_rels/document.xml.rels", doc_rels),
            ("word/data.xml", b"<root/>".to_vec()),
        ]);
        let report = validate_bytes_limited(&bytes, Limits { part_bytes: 4096, ..Limits::default() });
        assert!(!report.findings.iter().any(|f| f.rule == "CT-MISMATCH"), "findings: {:?}", report.findings);
        assert_eq!(report.verdict, Verdict::Incomplete, "findings: {:?}", report.findings);
    }

    /// CodeRabbit PR#18 (CWE-409): successful reads exceeding `part_bytes` must still charge `total_bytes`.
    #[test]
    fn coderabbit_oversized_part_decompression_charges_total_budget() {
        use std::io::{Cursor, Write};
        use zip::write::SimpleFileOptions;

        let ct = content_types();
        let rels = package_rels();
        let doc = format!(r#"<?xml version="1.0"?><w:document xmlns:w="{NS_W}"><w:body><w:p/></w:body></w:document>"#).into_bytes();
        let prefix = (ct.len() + rels.len() + doc.len()) as u64;
        let part_bytes = 450u64;
        let over1 = format!(r#"<?xml version="1.0"?><root><!-- {} --></root>"#, "x".repeat(700)).into_bytes();
        let over2 = format!(r#"<?xml version="1.0"?><root><!-- {} --></root>"#, "y".repeat(170)).into_bytes();

        let mut w = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let opts = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        for (name, data) in [
            ("[Content_Types].xml", &ct),
            ("_rels/.rels", &rels),
            ("word/document.xml", &doc),
            ("word/over1.xml", &over1),
            ("word/over2.xml", &over2),
        ] {
            w.start_file(name, opts).unwrap();
            w.write_all(data).unwrap();
        }
        let mut bytes = w.finish().unwrap().into_inner();
        // Forge `word/over1.xml`'s declared uncompressed size in both local and central headers so
        // it bypasses the header pre-check and trips `buf.len() > limits.part_bytes` during inflate.
        for (sig, usize_at, nlen_at, name_at) in [(b"PK\x03\x04", 22, 26, 30), (b"PK\x01\x02", 24, 28, 46)] {
            for i in 0..bytes.len().saturating_sub(name_at) {
                if bytes.get(i..i + 4) != Some(sig) {
                    continue;
                }
                let n = u16::from_le_bytes(bytes[i + nlen_at..i + nlen_at + 2].try_into().unwrap()) as usize;
                if bytes.get(i + name_at..i + name_at + n) == Some(b"word/over1.xml".as_slice()) {
                    bytes[i + usize_at..i + usize_at + 4].copy_from_slice(&1u32.to_le_bytes());
                }
            }
        }

        let report = validate_bytes_limited(&bytes, Limits { part_bytes, total_bytes: prefix + 550, ..Limits::default() });
        assert_eq!(report.verdict, Verdict::Incomplete, "{report:?}");
        assert!(
            report.coverage_notes.iter().any(|n| n.contains("word/over1.xml") && n.contains("decompresses past the 450-byte per-part limit")),
            "{report:?}"
        );
        assert!(report.coverage_notes.iter().any(|n| n.contains("word/over2.xml") && n.contains("total limit")), "{report:?}");
    }

    /// The byte-limit test's twin at the archive entry limit: the
    /// owner part falls beyond the examined prefix, so it is not
    /// proven absent and its consumption is unestablished — the
    /// customXml pairing must not fire for its relationships.
    #[test]
    fn ct_mismatch_skips_an_owner_beyond_the_entry_limit() {
        let ct = format!(
            r#"<?xml version="1.0"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/document.xml" ContentType="{}"/><Override PartName="/data.xml" ContentType="text/xml"/></Types>"#,
            package::MAIN_CONTENT_TYPE
        );
        let root_rels = rels(
            r#"<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="document.xml"/>"#,
        );
        let extra_rels =
            rels(r#"<Relationship Id="cp" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/customXml" Target="data.xml"/>"#);
        let extra = format!(
            r#"<?xml version="1.0"?><w:document xmlns:w="{NS_W}" xmlns:r="{NS_R}"><w:body><w:p><w:r><w:contentPart r:id="cp"/></w:r></w:p></w:body></w:document>"#
        )
        .into_bytes();
        let bytes = tiny_zip(&[
            ("[Content_Types].xml", ct.into_bytes()),
            ("_rels/.rels", root_rels),
            ("document.xml", document()),
            ("_rels/extra.xml.rels", extra_rels),
            ("data.xml", b"<root/>".to_vec()),
            ("extra.xml", extra),
        ]);
        let report = validate_bytes_limited(&bytes, Limits { entries: 5, ..Limits::default() });
        assert!(!report.findings.iter().any(|f| f.rule == "CT-MISMATCH"), "findings: {:?}", report.findings);
        assert_eq!(report.verdict, Verdict::Incomplete, "findings: {:?}", report.findings);
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
            ("word/_rels/document.xml.rels", br#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rC" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/comments" Target="comments.xml"/></Relationships>"#.to_vec()),
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
    /// Finding 1: event and depth limits leave declaration lookups unknown, just like entry caps.
    #[test]
    fn fredapps_1_xml_budget_declaration_completeness() {
        let doc = format!(r#"<w:document xmlns:w="{NS_W}" xmlns:r="{NS_R}"><w:body><w:p><w:hyperlink r:id="last"/></w:p></w:body></w:document>"#)
            .into_bytes();
        let last = r#"<Relationship Id="last" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink" Target="https://example.org" TargetMode="External"/>"#;
        for (prefix, limits) in [
            (
                (0..40)
                    .map(|i| format!(r#"<Relationship Id="r{i}" Type="urn:custom" Target="https://example.org" TargetMode="External"/>"#))
                    .collect::<String>(),
                Limits { events: 15, ..Limits::default() },
            ),
            ("<nested><nested><nested></nested></nested></nested>".to_string(), Limits { depth: 3, ..Limits::default() }),
        ] {
            let zip = tiny_zip(&[
                ("[Content_Types].xml", content_types()),
                ("_rels/.rels", package_rels()),
                ("word/document.xml", doc.clone()),
                (
                    "word/_rels/document.xml.rels",
                    format!(r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">{prefix}{last}</Relationships>"#)
                        .into_bytes(),
                ),
            ]);
            let report = validate_bytes_limited(&zip, limits);
            assert_eq!(report.verdict, Verdict::Incomplete, "{report:?}");
            assert!(!report.findings.iter().any(|f| f.rule == "REL-REF-MISSING"), "{report:?}");
        }
    }
}

#[cfg(test)]
mod round2_reference_tests {
    //! Reference incompleteness includes both per-part and pooled fact drops.
    use super::*;
    use std::io::{Cursor, Write};

    /// A dropped reference must not turn the corresponding comment into a never-referenced note.
    #[test]
    fn round2_capped_comment_references() {
        let ns = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let ct = format!(
            r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="{}"/><Override PartName="/word/comments.xml" ContentType="{}"/><Override PartName="/word/header.xml" ContentType="{}"/></Types>"#,
            package::MAIN_CONTENT_TYPE,
            package::COMMENTS_CONTENT_TYPE,
            package::HEADER_CONTENT_TYPE
        );
        let rels = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rC" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/comments" Target="comments.xml"/><Relationship Id="rH" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/header" Target="header.xml"/></Relationships>"#;
        for pooled in [false, true] {
            let refs = if pooled { r#"<w:commentReference w:id="1"/>"# } else { r#"<w:commentReference w:id="1"/><w:commentReference w:id="2"/>"# };
            let doc = format!(r#"<w:document xmlns:w="{ns}"><w:body><w:p><w:r>{refs}</w:r></w:p></w:body></w:document>"#);
            let header_ref = if pooled { r#"<w:commentReference w:id="2"/>"# } else { "" };
            let header = format!(r#"<w:hdr xmlns:w="{ns}"><w:p><w:r>{header_ref}</w:r></w:p></w:hdr>"#);
            let comments = format!(r#"<w:comments xmlns:w="{ns}"><w:comment w:id="2"><w:p/></w:comment></w:comments>"#);
            let root = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rM" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#;
            for (name, data) in [
                ("[Content_Types].xml", ct.as_str()),
                ("_rels/.rels", root),
                ("word/_rels/document.xml.rels", rels),
                ("word/document.xml", &doc),
                ("word/header.xml", &header),
                ("word/comments.xml", &comments),
            ] {
                zip.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
                zip.write_all(data.as_bytes()).unwrap();
            }
            let bytes = zip.finish().unwrap().into_inner();
            let report = validate_bytes_limited(&bytes, Limits { facts: 1, ..Limits::default() });
            assert!(
                !report.coverage_notes.iter().any(|n| n.contains("id 2 in word/comments.xml is never referenced")),
                "pooled {pooled}: {report:?}"
            );
            zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        }
    }
}

#[cfg(test)]
mod round3_note_cap_tests {
    //! Caps invalidate only the owner whose supporting collection lost facts.
    use super::*;
    use std::io::{Cursor, Write};

    /// A glossary per-part or pooled cap must preserve the main owner's established note join.
    #[test]
    fn round3_note_caps_preserve_other_owner() {
        let ns = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
        let relns = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
        for (kind, ct) in [("footnote", package::FOOTNOTES_CONTENT_TYPE), ("endnote", package::ENDNOTES_CONTENT_TYPE)] {
            for dangling in [false, true] {
                for pooled in [false, true] {
                    let mut entries = vec![
                        (
                            "_rels/.rels".to_string(),
                            format!(
                                r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rM" Type="{relns}/officeDocument" Target="word/document.xml"/></Relationships>"#
                            ),
                        ),
                        ("word/document.xml".to_string(), format!(r#"<w:document xmlns:w="{ns}"><w:body><w:p/></w:body></w:document>"#)),
                        ("word/glossary.xml".to_string(), format!(r#"<w:glossaryDocument xmlns:w="{ns}"><w:docParts/></w:glossaryDocument>"#)),
                    ];
                    let mut overrides = format!(
                        r#"<Override PartName="/word/document.xml" ContentType="{}"/><Override PartName="/word/glossary.xml" ContentType="{}"/>"#,
                        package::MAIN_CONTENT_TYPE,
                        package::GLOSSARY_CONTENT_TYPE
                    );
                    for gloss in [false, true] {
                        let prefix = if gloss { "gloss-" } else { "" };
                        let owner = if gloss { "glossary" } else { "document" };
                        let glossary_rel = if gloss {
                            String::new()
                        } else {
                            format!(r#"<Relationship Id="rG" Type="{relns}/glossaryDocument" Target="glossary.xml"/>"#)
                        };
                        entries.push((format!("word/_rels/{owner}.xml.rels"), format!(r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">{glossary_rel}<Relationship Id="rN" Type="{relns}/{kind}s" Target="{prefix}{kind}s.xml"/><Relationship Id="rS" Type="{relns}/settings" Target="{prefix}settings.xml"/></Relationships>"#)));
                        let count = if pooled { 2 } else { 3 };
                        let padding = (1..=count).map(|id| format!(r#"<w:{kind} w:id="{id}"><w:p/></w:{kind}>"#)).collect::<String>();
                        let notes = if gloss && dangling {
                            padding
                        } else if !gloss && (!dangling || pooled) {
                            format!(r#"<w:{kind} w:id="-1" w:type="separator"><w:p/></w:{kind}>"#)
                        } else {
                            String::new()
                        };
                        let listing = if gloss && !dangling {
                            (1..=count).map(|id| format!(r#"<w:{kind} w:id="{id}"/>"#)).collect::<String>()
                        } else if !gloss && (dangling || pooled) {
                            format!(r#"<w:{kind} w:id="{}"/>"#, if pooled { "-2" } else { "-1" })
                        } else {
                            String::new()
                        };
                        entries.push((format!("word/{prefix}{kind}s.xml"), format!(r#"<w:{kind}s xmlns:w="{ns}">{notes}</w:{kind}s>"#)));
                        entries.push((
                            format!("word/{prefix}settings.xml"),
                            format!(r#"<w:settings xmlns:w="{ns}"><w:{kind}Pr>{listing}</w:{kind}Pr></w:settings>"#),
                        ));
                        overrides.push_str(&format!(r#"<Override PartName="/word/{prefix}{kind}s.xml" ContentType="{ct}"/><Override PartName="/word/{prefix}settings.xml" ContentType="{}"/>"#, package::SETTINGS_CONTENT_TYPE));
                    }
                    entries.push(("[Content_Types].xml".to_string(), format!(r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/>{overrides}</Types>"#)));
                    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
                    for (name, text) in entries {
                        zip.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
                        zip.write_all(text.as_bytes()).unwrap();
                    }
                    let bytes = zip.finish().unwrap().into_inner();
                    let limited = validate_bytes_limited(&bytes, Limits { facts: 2, ..Limits::default() });
                    let rule = if dangling { "NOTE-PR-DANGLING" } else { "NOTE-PR-LISTING" };
                    assert_eq!(
                        limited.findings.iter().filter(|f| f.rule == rule).count(),
                        1,
                        "{kind}, dangling {dangling}, pooled {pooled}: {limited:?}"
                    );
                    if !dangling {
                        assert_eq!(limited.verdict, Verdict::Fail, "{limited:?}");
                        assert!(
                            limited.findings.iter().any(|f| f.rule == rule && f.part.as_deref() == Some(&format!("word/{kind}s.xml"))),
                            "{limited:?}"
                        );
                    }
                    assert!(limited.coverage_notes.iter().any(|n| n.contains("footnotePr/endnotePr") && n.contains("glossary")), "{limited:?}");
                    // The unlimited control has no skipped special-note joins.
                    let full = validate_bytes(&bytes);
                    assert!(full.findings.iter().any(|f| f.rule == rule), "{full:?}");
                    assert!(!full.coverage_notes.iter().any(|n| n.contains("could not be fully joined")), "{full:?}");
                }
            }
        }
    }
}

#[cfg(test)]
mod round4_comment_para_tests {
    //! Pooled paragraph loss must not manufacture owner-local comment-threading absence.
    use super::*;
    use std::io::{Cursor, Write};

    /// Build the reviewer's shape with three paragraph slots and one paragraph in the other owner.
    fn package(main_count: usize, missing_main: bool, missing_parent: bool) -> Vec<u8> {
        let relns = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
        let mut parts = vec![
            (
                "_rels/.rels".to_string(),
                format!(
                    r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rM" Type="{relns}/officeDocument" Target="word/document.xml"/></Relationships>"#
                ),
            ),
            ("word/document.xml".to_string(), format!(r#"<w:document xmlns:w="{NS_W}"><w:body><w:p/></w:body></w:document>"#)),
            ("word/glossary.xml".to_string(), format!(r#"<w:glossaryDocument xmlns:w="{NS_W}"><w:docParts/></w:glossaryDocument>"#)),
        ];
        let mut overrides = format!(
            r#"<Override PartName="/word/document.xml" ContentType="{}"/><Override PartName="/word/glossary.xml" ContentType="{}"/>"#,
            package::MAIN_CONTENT_TYPE,
            package::GLOSSARY_CONTENT_TYPE
        );
        for gloss in [false, true] {
            let prefix = if gloss { "gloss-" } else { "" };
            let owner = if gloss { "glossary" } else { "document" };
            let extra =
                if gloss { String::new() } else { format!(r#"<Relationship Id="rG" Type="{relns}/glossaryDocument" Target="glossary.xml"/>"#) };
            parts.push((format!("word/_rels/{owner}.xml.rels"), format!(r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">{extra}<Relationship Id="rC" Type="{relns}/comments" Target="{prefix}comments.xml"/><Relationship Id="rE" Type="{relns}/commentsExtended" Target="{prefix}commentsExtended.xml"/></Relationships>"#)));
            let paragraphs = if gloss {
                r#"<w:p w14:paraId="00030D41"/>"#.to_string()
            } else {
                (1..=main_count).map(|id| format!(r#"<w:p w14:paraId="{id:08X}"/>"#)).collect()
            };
            parts.push((
                format!("word/{prefix}comments.xml"),
                format!(r#"<w:comments xmlns:w="{NS_W}" xmlns:w14="{NS_W14}"><w:comment w:id="1">{paragraphs}</w:comment></w:comments>"#),
            ));
            let id = if gloss {
                "00030D41"
            } else if missing_main {
                "00000009"
            } else {
                "00000001"
            };
            let parent = if gloss && missing_parent { r#" w15:paraIdParent="00000008""# } else { "" };
            parts.push((
                format!("word/{prefix}commentsExtended.xml"),
                format!(r#"<w15:commentsEx xmlns:w15="{NS_W15}"><w15:commentEx w15:paraId="{id}"{parent}/></w15:commentsEx>"#),
            ));
            overrides.push_str(&format!(r#"<Override PartName="/word/{prefix}comments.xml" ContentType="{}"/><Override PartName="/word/{prefix}commentsExtended.xml" ContentType="{}"/>"#, package::COMMENTS_CONTENT_TYPE, package::COMMENTS_EXTENDED_CONTENT_TYPE));
        }
        parts.push(("[Content_Types].xml".to_string(), format!(r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/>{overrides}</Types>"#)));
        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for (name, text) in parts {
            zip.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
            zip.write_all(text.as_bytes()).unwrap();
        }
        zip.finish().unwrap().into_inner()
    }

    /// The exact pooled-loss reproduction; reducing main's paragraph count restores PASS.
    #[test]
    fn round4_pooled_comment_paragraph_loss() {
        let limits = Limits { facts: 3, ..Limits::default() };
        let control = validate_bytes_limited(&package(2, false, false), limits);
        assert_eq!(control.verdict, Verdict::Pass, "{control:?}");
        assert!(!control.findings.iter().any(|f| f.rule == "COMMENTEX-LINK"), "{control:?}");
        let report = validate_bytes_limited(&package(3, false, false), limits);
        assert_eq!(report.verdict, Verdict::Incomplete, "{report:?}");
        assert!(!report.findings.iter().any(|f| f.rule == "COMMENTEX-LINK"), "{report:?}");
        assert!(
            report.coverage_notes.iter().any(|n| n.contains("word/gloss-comments.xml") && n.contains("pooled comment paragraph IDs")),
            "{report:?}"
        );
        assert!(report.coverage_notes.iter().any(|n| n.contains("comment-threading") && n.contains("word/glossary.xml")), "{report:?}");
    }

    /// A complete owner's real absence and the incomplete owner's independent parent check survive.
    #[test]
    fn round4_pooled_comment_loss_preserves_independent_links() {
        let report = validate_bytes_limited(&package(3, true, true), Limits { facts: 3, ..Limits::default() });
        let links: Vec<_> = report.findings.iter().filter(|f| f.rule == "COMMENTEX-LINK").collect();
        assert_eq!(links.len(), 2, "{report:?}");
        assert!(links.iter().any(|f| f.part.as_deref() == Some("word/commentsExtended.xml") && f.message.contains("no paragraph")), "{report:?}");
        assert!(
            links.iter().any(|f| f.part.as_deref() == Some("word/gloss-commentsExtended.xml") && f.message.contains("paraIdParent")),
            "{report:?}"
        );
        assert!(
            !links.iter().any(|f| f.part.as_deref() == Some("word/gloss-commentsExtended.xml") && f.message.contains("no paragraph")),
            "{report:?}"
        );
    }
}

#[cfg(test)]
mod round5_completeness_tests {
    //! Absence predicates cover every source in their joined definition set.
    use super::*;
    use std::io::{Cursor, Write};

    /// Package synthetic parts in their supplied order, including typed nonstandard names.
    fn package(parts: Vec<(&str, &str, String)>, main_rels: &str) -> Vec<u8> {
        let relns = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let mut overrides = format!(r#"<Override PartName="/word/document.xml" ContentType="{}"/>"#, package::MAIN_CONTENT_TYPE);
        for (name, ct, _) in &parts {
            if !ct.is_empty() {
                overrides.push_str(&format!(r#"<Override PartName="/{name}" ContentType="{ct}"/>"#));
            }
        }
        let ct = format!(
            r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/>{overrides}</Types>"#
        );
        let root = format!(
            r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rM" Type="{relns}/officeDocument" Target="word/document.xml"/></Relationships>"#
        );
        let rels = relationships(main_rels);
        let doc = format!(r#"<w:document xmlns:w="{NS_W}"><w:body><w:p/></w:body></w:document>"#);
        for (name, text) in [("[Content_Types].xml", ct), ("_rels/.rels", root), ("word/document.xml", doc), ("word/_rels/document.xml.rels", rels)]
            .into_iter()
            .chain(parts.into_iter().map(|(name, _, text)| (name, text)))
        {
            zip.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
            zip.write_all(text.as_bytes()).unwrap();
        }
        zip.finish().unwrap().into_inner()
    }

    /// Wrap owner-local relationships.
    fn relationships(children: &str) -> String {
        format!(r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">{children}</Relationships>"#)
    }

    /// One internal relationship with a stable unique id per target.
    fn rel(kind: &str, target: &str) -> String {
        format!(
            r#"<Relationship Id="{target}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/{kind}" Target="{target}"/>"#
        )
    }

    /// Comment paragraphs use the same ids as the threading records.
    fn comments(ids: &[u32]) -> String {
        let paragraphs = ids.iter().map(|id| format!(r#"<w:p w14:paraId="{id:08X}"/>"#)).collect::<String>();
        format!(r#"<w:comments xmlns:w="{NS_W}" xmlns:w14="{NS_W14}"><w:comment w:id="1">{paragraphs}</w:comment></w:comments>"#)
    }

    /// The first glossary comments source is complete; the second, shared source loses id 3.
    fn shared(pooled: bool, loss: bool) -> Vec<u8> {
        let own_ids: &[u32] = if pooled { &[4] } else { &[] };
        let shared_ids: &[u32] = if !loss {
            &[1, 3]
        } else if pooled {
            &[1, 2, 3]
        } else {
            &[1, 2, 4, 3]
        };
        let gloss_rels = rel("comments", "comments.xml") + &rel("commentsExtended", "gloss-ext.xml");
        package(
            vec![
                (
                    "word/glossary.xml",
                    package::GLOSSARY_CONTENT_TYPE,
                    format!(r#"<w:glossaryDocument xmlns:w="{NS_W}"><w:docParts/></w:glossaryDocument>"#),
                ),
                ("word/_rels/glossary.xml.rels", "", relationships(&gloss_rels)),
                ("word/gloss-comments.xml", package::COMMENTS_CONTENT_TYPE, comments(own_ids)),
                ("word/comments.xml", package::COMMENTS_CONTENT_TYPE, comments(shared_ids)),
                (
                    "word/gloss-ext.xml",
                    package::COMMENTS_EXTENDED_CONTENT_TYPE,
                    format!(
                        r#"<w15:commentsEx xmlns:w15="{NS_W15}"><w15:commentEx w15:paraId="00000003" w15:paraIdParent="00000008"/></w15:commentsEx>"#
                    ),
                ),
                (
                    "word/main-ext.xml",
                    package::COMMENTS_EXTENDED_CONTENT_TYPE,
                    format!(r#"<w15:commentsEx xmlns:w15="{NS_W15}"><w15:commentEx w15:paraId="00000009"/></w15:commentsEx>"#),
                ),
            ],
            &(rel("glossaryDocument", "glossary.xml") + &rel("comments", "comments.xml") + &rel("commentsExtended", "main-ext.xml")),
        )
    }

    /// Both loss paths must suppress glossary absence and preserve independent parent checks.
    fn assert_shared(pooled: bool) {
        let limits = Limits { facts: 3, ..Limits::default() };
        let control = validate_bytes_limited(&shared(pooled, false), limits);
        assert!(control.findings.iter().any(|f| f.part.as_deref() == Some("word/main-ext.xml") && f.message.contains("no paragraph")), "{control:?}");
        assert!(
            !control.findings.iter().any(|f| f.part.as_deref() == Some("word/gloss-ext.xml") && f.message.contains("no paragraph")),
            "{control:?}"
        );
        let report = validate_bytes_limited(&shared(pooled, true), limits);
        eprintln!("shared pooled={pooled}: {report:?}");
        assert_eq!(report.verdict, Verdict::Incomplete, "{report:?}");
        assert!(report.findings.iter().any(|f| f.part.as_deref() == Some("word/gloss-ext.xml") && f.message.contains("paraIdParent")), "{report:?}");
        assert!(!report.findings.iter().any(|f| f.message.contains("no paragraph")), "{report:?}");
        assert!(
            report
                .coverage_notes
                .iter()
                .any(|n| n.contains("comment-threading") && n.contains("word/glossary.xml") && n.contains("word/comments.xml")),
            "{report:?}"
        );
        assert_eq!(report.coverage_notes.iter().any(|n| n.contains("pooled comment paragraph IDs")), pooled, "{report:?}");
    }

    /// Pooled loss in the second contributing comments source.
    #[test]
    fn round5_shared_pooled_loss() {
        assert_shared(true);
    }

    /// Pure per-part loss in the second contributing comments source, with the first empty.
    #[test]
    fn round5_shared_per_part_loss() {
        assert_shared(false);
    }

    /// Definition caps cannot manufacture style-reference absence.
    #[test]
    fn round5_style_definition_cap() {
        let bytes = package(
            vec![
                (
                    "word/styles.xml",
                    package::STYLES_CONTENT_TYPE,
                    format!(r#"<w:styles xmlns:w="{NS_W}"><w:style w:styleId="A"/><w:style w:styleId="B"/></w:styles>"#),
                ),
                (
                    "word/header.xml",
                    package::HEADER_CONTENT_TYPE,
                    format!(r#"<w:hdr xmlns:w="{NS_W}"><w:p><w:pPr><w:pStyle w:val="B"/></w:pPr></w:p></w:hdr>"#),
                ),
            ],
            &(rel("styles", "styles.xml") + &rel("header", "header.xml")),
        );
        let report = validate_bytes_limited(&bytes, Limits { facts: 1, ..Limits::default() });
        eprintln!("style cap: {report:?}");
        assert!(!report.findings.iter().any(|f| f.rule == "REF-STYLE"), "{report:?}");
        assert!(!validate_bytes(&bytes).findings.iter().any(|f| f.rule == "REF-STYLE"));
    }

    /// Numbering caps must guard the abstract set and both story/style joins.
    #[test]
    fn round5_numbering_definition_cap() {
        let bytes = package(
            vec![
                (
                    "word/numbering.xml",
                    package::NUMBERING_CONTENT_TYPE,
                    format!(
                        r#"<w:numbering xmlns:w="{NS_W}"><w:abstractNum w:abstractNumId="1"/><w:abstractNum w:abstractNumId="2"/><w:num w:numId="1"><w:abstractNumId w:val="2"/></w:num><w:num w:numId="2"><w:abstractNumId w:val="2"/></w:num></w:numbering>"#
                    ),
                ),
                (
                    "word/styles.xml",
                    package::STYLES_CONTENT_TYPE,
                    format!(
                        r#"<w:styles xmlns:w="{NS_W}"><w:style w:styleId="A"><w:pPr><w:numPr><w:numId w:val="2"/></w:numPr></w:pPr></w:style></w:styles>"#
                    ),
                ),
                (
                    "word/header.xml",
                    package::HEADER_CONTENT_TYPE,
                    format!(r#"<w:hdr xmlns:w="{NS_W}"><w:p><w:pPr><w:numPr><w:numId w:val="2"/></w:numPr></w:pPr></w:p></w:hdr>"#),
                ),
            ],
            &(rel("numbering", "numbering.xml") + &rel("styles", "styles.xml") + &rel("header", "header.xml")),
        );
        let report = validate_bytes_limited(&bytes, Limits { facts: 1, ..Limits::default() });
        eprintln!("numbering cap: {report:?}");
        assert!(!report.findings.iter().any(|f| f.rule == "NUM-DANGLING"), "{report:?}");
        assert!(!validate_bytes(&bytes).findings.iter().any(|f| f.rule == "NUM-DANGLING"));
    }

    /// Capped attached-schema definitions cannot establish an unattached reference.
    #[test]
    fn round5_attached_schema_cap() {
        let bytes = package(
            vec![
                (
                    "word/settings.xml",
                    package::SETTINGS_CONTENT_TYPE,
                    format!(r#"<w:settings xmlns:w="{NS_W}"><w:attachedSchema w:val="urn:A"/><w:attachedSchema w:val="urn:B"/></w:settings>"#),
                ),
                (
                    "word/props.xml",
                    package::CUSTOM_XML_PROPS_CONTENT_TYPE,
                    format!(
                        r#"<ds:datastoreItem xmlns:ds="{NS_DS}"><ds:schemaRefs><ds:schemaRef ds:uri="urn:B"/></ds:schemaRefs></ds:datastoreItem>"#
                    ),
                ),
            ],
            &(rel("settings", "settings.xml") + &rel("customXmlProps", "props.xml")),
        );
        let report = validate_bytes_limited(&bytes, Limits { facts: 1, ..Limits::default() });
        eprintln!("schema cap: {report:?}");
        assert!(!report.findings.iter().any(|f| f.rule == "CUSTOMXML-SCHEMA"), "{report:?}");
        assert!(!report.coverage_notes.iter().any(|n| n.ends_with("which is not attached in settings.xml")), "{report:?}");
        let complete = validate_bytes(&bytes);
        assert!(!complete.findings.iter().any(|f| f.rule == "CUSTOMXML-SCHEMA"), "{complete:?}");
        assert!(!complete.coverage_notes.iter().any(|n| n.ends_with("which is not attached in settings.xml")), "{complete:?}");
    }

    #[test]
    fn custom_xml_schema_note_suppressed_for_unknown_entry_identity() {
        let bytes = package(
            vec![
                (
                    "word/settings.xml",
                    package::SETTINGS_CONTENT_TYPE,
                    format!(r#"<w:settings xmlns:w="{NS_W}"><w:attachedSchema w:val="urn:A"/></w:settings>"#),
                ),
                (
                    "word/props.xml",
                    package::CUSTOM_XML_PROPS_CONTENT_TYPE,
                    format!(
                        r#"<ds:datastoreItem xmlns:ds="{NS_DS}"><ds:schemaRefs><ds:schemaRef ds:uri="urn:B"/></ds:schemaRefs></ds:datastoreItem>"#
                    ),
                ),
            ],
            &(rel("settings", "settings.xml") + &rel("customXmlProps", "props.xml")),
        );
        let mut setup = Acc::default();
        let pkg = package::load(&bytes, &mut setup).unwrap();
        let xml = check_xml(&pkg, &mut setup);

        let mut control = Acc::default();
        check_custom_xml(&pkg, &xml, &mut control);
        assert_eq!(control.coverage, ["custom XML part word/props.xml declares schema urn:B, which is not attached in settings.xml"]);

        let mut suppressed = Acc::default();
        suppressed.set_unknown_unreadable();
        check_custom_xml(&pkg, &xml, &mut suppressed);
        assert!(suppressed.coverage.is_empty(), "{:?}", suppressed.coverage);
    }

    /// Threading joins use each direction's definition completeness, including missing parse results.
    fn assert_threading_loss(mode: &str) {
        let ext = if mode == "ext-malformed" {
            "<".to_string()
        } else {
            let second = if mode == "ext-cap" { r#"<w15:commentEx w15:paraId="00000002"/>"# } else { "" };
            format!(
                r#"<w15:commentsEx xmlns:w15="{NS_W15}"><w15:commentEx w15:paraId="00000001" w15:paraIdParent="00000002"/>{second}</w15:commentsEx>"#
            )
        };
        let ids = if mode == "ids-cap" { vec![2, 1] } else { vec![2] };
        let ids = ids.into_iter().map(|id| format!(r#"<w16cid:commentId w16cid:paraId="{id:08X}"/>"#)).collect::<String>();
        let bytes = package(
            vec![
                ("word/comments.xml", package::COMMENTS_CONTENT_TYPE, comments(&[1])),
                ("word/ext.xml", package::COMMENTS_EXTENDED_CONTENT_TYPE, ext),
                (
                    "word/ids.xml",
                    package::COMMENTS_IDS_CONTENT_TYPE,
                    format!(r#"<w16cid:commentsIds xmlns:w16cid="{NS_W16CID}">{ids}</w16cid:commentsIds>"#),
                ),
            ],
            &(rel("comments", "comments.xml") + &rel("commentsExtended", "ext.xml") + &rel("commentsIds", "ids.xml")),
        );
        let report = validate_bytes_limited(&bytes, Limits { facts: 1, ..Limits::default() });
        eprintln!("threading {mode}: {report:?}");
        if mode != "ids-cap" {
            assert!(
                !report
                    .findings
                    .iter()
                    .any(|f| f.rule == "COMMENTEX-LINK" && (f.message.contains("paraIdParent") || f.message.contains("no commentEx counterpart"))),
                "{mode}: {report:?}"
            );
        } else {
            assert!(!report.findings.iter().any(|f| f.message.contains("no commentsIds paraId counterpart")), "{report:?}");
            assert!(report.findings.iter().any(|f| f.message.contains("paraIdParent")), "{report:?}");
        }
    }

    /// The parent and commentsIds checks cannot draw absence from capped commentEx definitions.
    #[test]
    fn round5_threading_ext_cap() {
        assert_threading_loss("ext-cap");
    }

    /// Only the check against capped commentsIds is suppressed; the complete parent set still warns.
    #[test]
    fn round5_threading_ids_cap() {
        assert_threading_loss("ids-cap");
    }

    /// A failed commentEx parse cannot be replaced with an empty definition set.
    #[test]
    fn round5_threading_ext_malformed() {
        assert_threading_loss("ext-malformed");
    }

    /// An auxiliary parser's EOF result cannot override the XML scan's incomplete source.
    #[test]
    fn round5_unclosed_definition_sources() {
        let mut manufactured = Vec::new();
        for kind in ["styles", "numbering", "settings", "ext"] {
            let (ct, text, ref_ct, reference, reference_kind, rule) = match kind {
                "styles" => (
                    package::STYLES_CONTENT_TYPE,
                    format!(r#"<w:styles xmlns:w="{NS_W}"><w:style w:styleId="A"/>"#),
                    package::HEADER_CONTENT_TYPE,
                    format!(r#"<w:hdr xmlns:w="{NS_W}"><w:p><w:pPr><w:pStyle w:val="B"/></w:pPr></w:p></w:hdr>"#),
                    "header",
                    "REF-STYLE",
                ),
                "numbering" => (
                    package::NUMBERING_CONTENT_TYPE,
                    format!(r#"<w:numbering xmlns:w="{NS_W}"><w:abstractNum w:abstractNumId="1"/>"#),
                    package::HEADER_CONTENT_TYPE,
                    format!(r#"<w:hdr xmlns:w="{NS_W}"><w:p><w:pPr><w:numPr><w:numId w:val="2"/></w:numPr></w:pPr></w:p></w:hdr>"#),
                    "header",
                    "NUM-DANGLING",
                ),
                "settings" => (
                    package::SETTINGS_CONTENT_TYPE,
                    format!(r#"<w:settings xmlns:w="{NS_W}"><w:attachedSchema w:val="urn:A"/>"#),
                    package::CUSTOM_XML_PROPS_CONTENT_TYPE,
                    format!(
                        r#"<ds:datastoreItem xmlns:ds="{NS_DS}"><ds:schemaRefs><ds:schemaRef ds:uri="urn:B"/></ds:schemaRefs></ds:datastoreItem>"#
                    ),
                    "customXmlProps",
                    "CUSTOMXML-SCHEMA",
                ),
                _ => (
                    package::COMMENTS_EXTENDED_CONTENT_TYPE,
                    format!(r#"<w15:commentsEx xmlns:w15="{NS_W15}"><w15:commentEx w15:paraId="00000001" w15:paraIdParent="00000002"/>"#),
                    package::COMMENTS_IDS_CONTENT_TYPE,
                    format!(r#"<w16cid:commentsIds xmlns:w16cid="{NS_W16CID}"><w16cid:commentId w16cid:paraId="00000002"/></w16cid:commentsIds>"#),
                    "commentsIds",
                    "COMMENTEX-LINK",
                ),
            };
            let bytes = package(
                vec![("word/defs.xml", ct, text), ("word/refs.xml", ref_ct, reference)],
                &(rel(if kind == "ext" { "commentsExtended" } else { kind }, "defs.xml") + &rel(reference_kind, "refs.xml")),
            );
            let report = validate_bytes(&bytes);
            eprintln!("unclosed {kind}: {report:?}");
            if report.findings.iter().any(|f| f.rule == rule) {
                manufactured.push(kind);
            }
            if kind == "settings" {
                assert!(!report.coverage_notes.iter().any(|n| n.ends_with("which is not attached in settings.xml")), "{report:?}");
            }
            assert!(report.findings.iter().any(|f| f.rule == "XML-MALFORMED"), "{report:?}");
        }
        assert!(manufactured.is_empty(), "false absences from unclosed sources: {manufactured:?}");
    }

    /// A relationship's target can lie beyond the examined archive prefix.
    #[test]
    fn round5_archive_prefix_absence() {
        let bytes = package(
            vec![("word/header.xml", package::HEADER_CONTENT_TYPE, format!(r#"<w:hdr xmlns:w="{NS_W}"><w:p/></w:hdr>"#))],
            &rel("header", "header.xml"),
        );
        let report = validate_bytes_limited(&bytes, Limits { entries: 4, ..Limits::default() });
        eprintln!("archive prefix: {report:?}");
        assert!(!report.findings.iter().any(|f| f.rule == "REL-DANGLING"), "{report:?}");
        assert_eq!(report.verdict, Verdict::Incomplete, "{report:?}");
        assert_eq!(validate_bytes(&bytes).verdict, Verdict::Pass);
    }

    /// Unexamined declarations and owners cannot establish missing parts or relationship ids.
    #[test]
    fn round5_archive_prefix_declarations() {
        let mut failures = Vec::new();
        let root = relationships(&rel("officeDocument", "word/document.xml"));
        let doc = format!(r#"<w:document xmlns:w="{NS_W}"><w:body><w:p/></w:body></w:document>"#);
        let ct = format!(
            r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Override PartName="/word/document.xml" ContentType="{}"/></Types>"#,
            package::MAIN_CONTENT_TYPE
        );
        for (label, parts) in [
            ("manifest", vec![("_rels/.rels", root.as_str()), ("word/document.xml", doc.as_str()), ("[Content_Types].xml", ct.as_str())]),
            ("main", vec![("[Content_Types].xml", ct.as_str()), ("_rels/.rels", root.as_str()), ("word/document.xml", doc.as_str())]),
        ] {
            let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
            for (name, text) in parts {
                zip.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
                zip.write_all(text.as_bytes()).unwrap();
            }
            let report = validate_bytes_limited(&zip.finish().unwrap().into_inner(), Limits { entries: 2, ..Limits::default() });
            eprintln!("archive prefix {label}: {report:?}");
            if report.findings.iter().any(|f| matches!(f.rule, "PKG-NO-CONTENT-TYPES" | "PKG-NO-MAIN")) {
                failures.push(label);
            }
        }
        let bytes = package(
            vec![
                (
                    "word/header.xml",
                    package::HEADER_CONTENT_TYPE,
                    format!(r#"<w:hdr xmlns:w="{NS_W}" xmlns:r="{NS_R}"><w:p><w:r><w:object r:id="x"/></w:r></w:p></w:hdr>"#),
                ),
                ("word/pad.xml", "", "<pad/>".to_string()),
                ("word/_rels/header.xml.rels", "", relationships(r#"<Relationship Id="x" Type="urn:test" Target="pad.xml"/>"#)),
            ],
            &rel("header", "header.xml"),
        );
        let report = validate_bytes_limited(&bytes, Limits { entries: 6, ..Limits::default() });
        eprintln!("archive prefix relationships: {report:?}");
        if report.findings.iter().any(|f| f.rule == "REL-REF-MISSING") {
            failures.push("relationships");
        }
        assert!(failures.is_empty(), "false absences from archive prefixes: {failures:?}");
    }

    /// Repack one synthetic part while preserving archive ordering and all other payloads.
    fn replace_part(bytes: &[u8], target: &str, text: &str) -> Vec<u8> {
        use std::io::Read;
        let mut input = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
        let mut output = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for i in 0..input.len() {
            let mut entry = input.by_index(i).unwrap();
            let mut data = Vec::new();
            entry.read_to_end(&mut data).unwrap();
            output.start_file(entry.name(), zip::write::SimpleFileOptions::default()).unwrap();
            output.write_all(if entry.name() == target { text.as_bytes() } else { &data }).unwrap();
        }
        output.finish().unwrap().into_inner()
    }

    /// Unreadable or parser-failed second contributors have the same source-set requirements as caps.
    #[test]
    fn round5_shared_unavailable_source() {
        for text in [r#"<?xml version="1.0" encoding="X-unknown"?><comments/>"#, "<"] {
            let bytes = replace_part(&shared(true, false), "word/comments.xml", text);
            let report = validate_bytes(&bytes);
            eprintln!("shared unavailable {text}: {report:?}");
            assert!(!report.findings.iter().any(|f| f.rule == "COMMENTEX-LINK" && f.message.contains("no paragraph")), "{report:?}");
            assert!(report.findings.iter().any(|f| f.message.contains("paraIdParent")), "{report:?}");
            assert!(report.coverage_notes.iter().any(|n| n.contains("word/glossary.xml") && n.contains("word/comments.xml")), "{report:?}");
        }
    }

    /// A truncated consumption scan cannot establish that the customXml relationship is unused.
    #[test]
    fn round5_content_part_scan_stop() {
        let mut failures = Vec::new();
        for depth in [false, true] {
            let padding = if depth { "<x>".repeat(8) + &"</x>".repeat(8) } else { "<x/>".repeat(40) };
            let bytes = package(
                vec![
                    (
                        "word/header.xml",
                        package::HEADER_CONTENT_TYPE,
                        format!(
                            r#"<w:hdr xmlns:w="{NS_W}" xmlns:r="{NS_R}">{padding}<w:p><w:r><w:contentPart r:id="data.xml"/></w:r></w:p></w:hdr>"#
                        ),
                    ),
                    ("word/_rels/header.xml.rels", "", relationships(&rel("customXml", "data.xml"))),
                    ("word/data.xml", "text/xml", "<data/>".to_string()),
                ],
                &rel("header", "header.xml"),
            );
            let limits = if depth { Limits { depth: 6, ..Limits::default() } } else { Limits { events: 25, ..Limits::default() } };
            let report = validate_bytes_limited(&bytes, limits);
            eprintln!("contentPart depth={depth}: {report:?}");
            if report.findings.iter().any(|f| f.rule == "CT-MISMATCH") {
                failures.push(depth);
            }
            assert_eq!(validate_bytes(&bytes).verdict, Verdict::Pass);
        }
        assert!(failures.is_empty(), "false non-consumption from scan stops: depth={failures:?}");
    }

    /// The specialized header join needs the same missing-relationships guard as generic r:id joins.
    #[test]
    fn round5_header_archive_prefix() {
        let ct = format!(
            r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="{}"/><Override PartName="/word/header.xml" ContentType="{}"/></Types>"#,
            package::MAIN_CONTENT_TYPE,
            package::HEADER_CONTENT_TYPE
        );
        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for (name, text) in [
            ("[Content_Types].xml", ct),
            ("_rels/.rels", relationships(&rel("officeDocument", "word/document.xml"))),
            (
                "word/document.xml",
                format!(
                    r#"<w:document xmlns:w="{NS_W}" xmlns:r="{NS_R}"><w:body><w:p/><w:sectPr><w:headerReference w:type="default" r:id="header.xml"/></w:sectPr></w:body></w:document>"#
                ),
            ),
            ("word/header.xml", format!(r#"<w:hdr xmlns:w="{NS_W}"><w:p/></w:hdr>"#)),
            ("word/_rels/document.xml.rels", relationships(&rel("header", "header.xml"))),
        ] {
            zip.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
            zip.write_all(text.as_bytes()).unwrap();
        }
        let bytes = zip.finish().unwrap().into_inner();
        let report = validate_bytes_limited(&bytes, Limits { entries: 4, ..Limits::default() });
        eprintln!("header archive prefix: {report:?}");
        assert!(!report.findings.iter().any(|f| f.rule == "REF-HEADER"), "{report:?}");
        assert_eq!(report.verdict, Verdict::Incomplete, "{report:?}");
        assert_eq!(validate_bytes(&bytes).verdict, Verdict::Pass);
    }

    /// Complete definition sources still expose genuine absences in every newly guarded family.
    #[test]
    fn round5_complete_definition_absence_controls() {
        let bytes = package(
            vec![
                (
                    "word/styles.xml",
                    package::STYLES_CONTENT_TYPE,
                    format!(
                        r#"<w:styles xmlns:w="{NS_W}"><w:style w:styleId="A"><w:pPr><w:numPr><w:numId w:val="9"/></w:numPr></w:pPr></w:style></w:styles>"#
                    ),
                ),
                (
                    "word/numbering.xml",
                    package::NUMBERING_CONTENT_TYPE,
                    format!(r#"<w:numbering xmlns:w="{NS_W}"><w:num w:numId="1"><w:abstractNumId w:val="9"/></w:num></w:numbering>"#),
                ),
                (
                    "word/header.xml",
                    package::HEADER_CONTENT_TYPE,
                    format!(
                        r#"<w:hdr xmlns:w="{NS_W}"><w:p><w:pPr><w:pStyle w:val="B"/><w:numPr><w:numId w:val="9"/></w:numPr></w:pPr></w:p></w:hdr>"#
                    ),
                ),
                (
                    "word/settings.xml",
                    package::SETTINGS_CONTENT_TYPE,
                    format!(r#"<w:settings xmlns:w="{NS_W}"><w:attachedSchema w:val="urn:A"/></w:settings>"#),
                ),
                (
                    "word/props.xml",
                    package::CUSTOM_XML_PROPS_CONTENT_TYPE,
                    format!(
                        r#"<ds:datastoreItem xmlns:ds="{NS_DS}"><ds:schemaRefs><ds:schemaRef ds:uri="urn:B"/></ds:schemaRefs></ds:datastoreItem>"#
                    ),
                ),
            ],
            &(rel("styles", "styles.xml")
                + &rel("numbering", "numbering.xml")
                + &rel("header", "header.xml")
                + &rel("settings", "settings.xml")
                + &rel("customXmlProps", "props.xml")),
        );
        let report = validate_bytes_limited(&bytes, Limits { facts: 1, ..Limits::default() });
        assert!(report.findings.iter().any(|f| f.rule == "REF-STYLE"), "{report:?}");
        for part in ["word/header.xml", "word/styles.xml", "word/numbering.xml"] {
            assert!(report.findings.iter().any(|f| f.rule == "NUM-DANGLING" && f.part.as_deref() == Some(part)), "{report:?}");
        }
        assert!(
            report.coverage_notes.iter().any(|n| n == "custom XML part word/props.xml declares schema urn:B, which is not attached in settings.xml"),
            "{report:?}"
        );
    }

    /// An unexamined content-type declaration cannot establish that an owner is non-XML.
    #[test]
    fn round5_content_part_unknown_owner_type() {
        let bytes = package(
            vec![
                (
                    "word/extra.dat",
                    "application/xml",
                    format!(r#"<w:root xmlns:w="{NS_W}" xmlns:r="{NS_R}"><w:contentPart r:id="data.xml"/></w:root>"#),
                ),
                ("word/_rels/extra.dat.rels", "", relationships(&rel("customXml", "data.xml"))),
                ("word/data.xml", "text/xml", "<data/>".to_string()),
            ],
            &rel("unknown", "extra.dat"),
        );
        let padding = (0..10).map(|i| format!(r#"<Default Extension="unused{i}" ContentType="application/xml"/>"#)).collect::<String>();
        let ct = format!(
            r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Override PartName="/word/document.xml" ContentType="{}"/><Override PartName="/word/data.xml" ContentType="text/xml"/>{padding}<Override PartName="/word/extra.dat" ContentType="application/xml"/></Types>"#,
            package::MAIN_CONTENT_TYPE
        );
        let report = validate_bytes_limited(&replace_part(&bytes, "[Content_Types].xml", &ct), Limits { entries: 10, ..Limits::default() });
        eprintln!("unknown contentPart owner type: {report:?}");
        assert!(!report.findings.iter().any(|f| f.rule == "CT-MISMATCH"), "{report:?}");
        assert_eq!(validate_bytes(&bytes).verdict, Verdict::Pass);
    }
}

/// Round-6 probes for ownership and classification not exposed by the public report.
#[cfg(test)]
mod round6_tests {
    use super::*;
    use std::io::{Cursor, Write};

    /// A minimal package with one optional typed relationship to a custom binary part.
    fn fixture(kind: &str, ct: &str, target: &[u8]) -> Vec<u8> {
        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let types = format!(
            r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="{}"/><Override PartName="/word/vendor.dat" ContentType="{ct}"/></Types>"#,
            package::MAIN_CONTENT_TYPE
        );
        let root = format!(
            r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="main" Type="{NS_R}/officeDocument" Target="word/document.xml"/></Relationships>"#
        );
        let rels = format!(
            r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="vendor" Type="{kind}" Target="vendor.dat"/></Relationships>"#
        );
        let doc = format!(r#"<w:document xmlns:w="{NS_W}"><w:body><w:p/></w:body></w:document>"#);
        for (name, bytes) in [
            ("[Content_Types].xml", types.as_bytes()),
            ("_rels/.rels", root.as_bytes()),
            ("word/document.xml", doc.as_bytes()),
            ("word/_rels/document.xml.rels", rels.as_bytes()),
            ("word/vendor.dat", target),
        ] {
            zip.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
            zip.write_all(bytes).unwrap();
        }
        zip.finish().unwrap().into_inner()
    }

    /// A custom glossary suffix cannot create a second owner or cut its subtree out of main.
    #[test]
    fn round6_custom_glossary_does_not_create_owner_scope() {
        let bytes = fixture("urn:vendor/glossaryDocument", "application/octet-stream", b"payload");
        let mut acc = Acc::default();
        let pkg = package::load(&bytes, &mut acc).unwrap();
        eprintln!("custom glossary scope: {:?}", owner_scopes(&pkg));
        assert!(!matches!(pkg.glossary, Resolution::One(_)));
        assert_eq!(owner_scopes(&pkg), vec![OwnerContext::Main("word/document.xml".into())]);
        assert_eq!(part_owners(&owner_index_map(&pkg), "word/vendor.dat"), vec![OwnerId(0)]);
    }

    /// Actual glossary types still establish and isolate the glossary's owner context.
    #[test]
    fn round6_official_glossary_still_resolves() {
        let bytes = fixture(
            &format!("{NS_R}/glossaryDocument"),
            package::GLOSSARY_CONTENT_TYPE,
            format!(r#"<w:glossaryDocument xmlns:w="{NS_W}"/>"#).as_bytes(),
        );
        let mut acc = Acc::default();
        let pkg = package::load(&bytes, &mut acc).unwrap();
        assert_eq!(
            (match &pkg.glossary {
                Resolution::One(name) => Some(name.as_str()),
                Resolution::Absent | Resolution::Ambiguous(_) | Resolution::Unknown(_) => None,
            }),
            Some("word/vendor.dat")
        );
        assert_eq!(part_owners(&owner_index_map(&pkg), "word/vendor.dat"), vec![OwnerId(1)]);
    }

    /// A vendor comments edge grants neither a story role nor XML decoding by suffix.
    #[test]
    fn round6_custom_comment_role_and_xml_classification_are_unknown() {
        assert!(relationship_role(package::relationship_kind("urn:vendor/comments")).is_none());
        let bytes = fixture("urn:vendor/comments", "application/octet-stream", &[0xff]);
        let mut acc = Acc::default();
        let pkg = package::load(&bytes, &mut acc).unwrap();
        assert!(!pkg.xml_parts.contains("word/vendor.dat"));
        assert!(matches!(entry_by_name(&pkg, "word/vendor.dat"), Resolution::One(entry) if entry.decode_error.is_none()));
    }

    /// Absence suppression cannot hide an observed count; an unknown entry could supply a listing.
    #[test]
    fn round6_absence_table_matches_its_contract() {
        let mut acc = Acc::default();
        acc.set_unknown_unreadable();
        acc.add("STYLE-COUNT", None, "observed 4079 styles".into());
        acc.add("NOTE-PR-LISTING", None, "unknown settings could supply the listing".into());
        eprintln!("unknown identity accumulator: {:?}", acc.findings);
        assert_eq!(acc.findings.len(), 1);
        assert_eq!(acc.findings[0].rule, "STYLE-COUNT");
    }

    /// Element-side image and header references require the real relationship type, not a suffix.
    #[test]
    fn round6_custom_image_and_header_types_do_not_satisfy_references() {
        for (kind, ct) in [("image", "image/png"), ("header", package::HEADER_CONTENT_TYPE)] {
            for (namespace, expected) in [("urn:vendor", 1), (NS_R, 0)] {
                let bytes = fixture(&format!("{namespace}/{kind}"), ct, b"payload");
                let mut acc = Acc::default();
                let pkg = package::load(&bytes, &mut acc).unwrap();
                if kind == "image" {
                    check_typed_image_refs(
                        &pkg,
                        &[("word/document.xml".into(), vec![RefFact { attr: "embed".into(), value: "vendor".into(), image: true }])],
                        &mut acc,
                    );
                } else {
                    let facts = scan::StoryFacts {
                        header_refs: Observed::complete(vec![("vendor".into(), "word/document.xml".into())]),
                        ..scan::StoryFacts::default()
                    };
                    check_header_footer_refs(&pkg, &facts, &mut acc);
                }
                assert_eq!(acc.findings.len(), expected, "{namespace}/{kind}: {:?}", acc.findings);
            }
        }
    }

    /// A custom image suffix must not opt an unrelated binary target into magic-byte checking.
    #[test]
    fn round6_custom_image_does_not_classify_media_target() {
        for (namespace, expected) in [("urn:vendor", false), (NS_R, true)] {
            let bytes = fixture(&format!("{namespace}/image"), "application/octet-stream", b"payload");
            let mut acc = Acc::default();
            let pkg = package::load(&bytes, &mut acc).unwrap();
            assert_eq!(relationship_targets(&pkg, "image").contains("word/vendor.dat"), expected);
        }
    }

    /// Transitional and the Microsoft extension namespaces retain catalogue pairing behavior.
    #[test]
    fn round6_official_content_pairings_still_classify() {
        for namespace in [
            NS_R,
            "http://schemas.microsoft.com/office/2006/relationships",
            "http://schemas.microsoft.com/office/2007/relationships",
            "http://schemas.microsoft.com/office/2011/relationships",
            "http://schemas.microsoft.com/office/2016/09/relationships",
        ] {
            for (kind, ct) in FIXED_REL_CONTENT_TYPES.iter().filter(|(k, _)| !matches!(*k, "header" | "footer")) {
                // Pairing checks need a real owner root to pass the document-profile gate.
                let body = if *kind == "glossaryDocument" { format!(r#"<w:glossaryDocument xmlns:w="{NS_W}"/>"#) } else { "<root/>".into() };
                let bytes = fixture(&format!("{namespace}/{kind}"), ct, body.as_bytes());
                let report = validate_bytes(&bytes);
                assert_eq!(report.verdict, Verdict::Pass, "{namespace}/{kind}: {report:?}");
                let bytes = fixture(&format!("{namespace}/{kind}"), "application/xml", body.as_bytes());
                let report = validate_bytes(&bytes);
                assert_eq!(report.findings.iter().any(|f| f.rule == "CT-MISMATCH"), *ct != "application/xml", "{namespace}/{kind}: {report:?}");
            }
        }
    }

    /// Official-looking prefixes, nested paths, and scheme changes are still custom type URIs.
    #[test]
    fn round6_relationship_kind_rejects_namespace_lookalikes() {
        for uri in [
            "styles",
            "urn:vendor/styles",
            "https://schemas.openxmlformats.org/officeDocument/2006/relationships/styles",
            "http://schemas.openxmlformats.org/officeDocument/2006/relationships/vendor/styles",
            "http://schemas.microsoft.com/office/2011/relationships/vendor/commentsExtended",
            "http://schemas.microsoft.com/office/2011/relationships/",
        ] {
            assert_eq!(relationship_kind(uri), None, "{uri}");
        }
    }
}

#[cfg(test)]
mod round7_owner_discovery_tests {
    //! Descendant discovery and shared auxiliary sources retain each owner's absence predicate.
    use super::*;
    use std::io::{Cursor, Write};

    /// Wrap relationship declarations without relying on conventional supporting-part names.
    fn rels(children: &str) -> String {
        format!(r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">{children}</Relationships>"#)
    }

    /// A unique internal relationship in the supported namespace.
    fn rel(kind: &str, target: &str) -> String {
        format!(r#"<Relationship Id="{target}" Type="{NS_R}/{kind}" Target="{target}"/>"#)
    }

    /// Build an entirely synthetic package with explicit content types.
    fn fixture(parts: Vec<(&str, &str, String)>, main_rels: String) -> Vec<u8> {
        let mut overrides = format!(r#"<Override PartName="/word/document.xml" ContentType="{}"/>"#, package::MAIN_CONTENT_TYPE);
        for (name, ct, _) in &parts {
            if !ct.is_empty() {
                overrides.push_str(&format!(r#"<Override PartName="/{name}" ContentType="{ct}"/>"#));
            }
        }
        let manifest = format!(
            r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/>{overrides}</Types>"#
        );
        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for (name, text) in [
            ("[Content_Types].xml", manifest),
            ("_rels/.rels", rels(&rel("officeDocument", "word/document.xml"))),
            ("word/document.xml", format!(r#"<w:document xmlns:w="{NS_W}"><w:body><w:p/></w:body></w:document>"#)),
            ("word/_rels/document.xml.rels", rels(&main_rels)),
        ]
        .into_iter()
        .chain(parts.into_iter().map(|(name, _, text)| (name, text)))
        {
            zip.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
            zip.write_all(text.as_bytes()).unwrap();
        }
        zip.finish().unwrap().into_inner()
    }

    /// Fail before the downstream edge, or retain it in the complete control.
    fn discovery(mode: &str, children: &str) -> String {
        match mode {
            "encoding" => r#"<?xml version="1.0" encoding="unsupported-test-encoding"?><Relationships/>"#.into(),
            "malformed" => "<Relationships><broken></Relationships>".into(),
            _ => rels(children),
        }
    }

    /// Selected paragraphs remain authoritative while a descendant branch loses discovery.
    fn comment_case(mode: &str, genuine: bool, shared: bool) -> Report {
        let para = if genuine { "00000009" } else { "00000003" };
        let mut glossary_rels = rel("commentsExtended", "gloss-ex.xml");
        if shared {
            glossary_rels += &rel("header", "header.xml");
            glossary_rels += &rel("comments", "comments.xml");
        }
        validate_bytes(&fixture(
            vec![
                ("word/header.xml", package::HEADER_CONTENT_TYPE, format!(r#"<w:hdr xmlns:w="{NS_W}"><w:p/></w:hdr>"#)),
                ("word/_rels/header.xml.rels", "", discovery(mode, &rel("customXml", "bridge.xml"))),
                ("word/bridge.xml", "application/xml", "<bridge/>".into()),
                ("word/_rels/bridge.xml.rels", "", rels(&rel("comments", "comments.xml"))),
                (
                    "word/comments.xml",
                    package::COMMENTS_CONTENT_TYPE,
                    format!(
                        r#"<w:comments xmlns:w="{NS_W}" xmlns:w14="{NS_W14}"><w:comment w:id="1"><w:p w14:paraId="00000003"/></w:comment></w:comments>"#
                    ),
                ),
                (
                    "word/ex.xml",
                    package::COMMENTS_EXTENDED_CONTENT_TYPE,
                    format!(
                        r#"<w15:commentsEx xmlns:w15="{NS_W15}"><w15:commentEx w15:paraId="{para}" w15:paraIdParent="00000008"/></w15:commentsEx>"#
                    ),
                ),
                (
                    "word/glossary.xml",
                    package::GLOSSARY_CONTENT_TYPE,
                    format!(r#"<w:glossaryDocument xmlns:w="{NS_W}"><w:docParts/></w:glossaryDocument>"#),
                ),
                ("word/_rels/glossary.xml.rels", "", rels(&glossary_rels)),
                (
                    "word/gloss-ex.xml",
                    package::COMMENTS_EXTENDED_CONTENT_TYPE,
                    format!(r#"<w15:commentsEx xmlns:w15="{NS_W15}"><w15:commentEx w15:paraId="00000009"/></w15:commentsEx>"#),
                ),
            ],
            rel("header", "header.xml")
                + &rel("comments", "comments.xml")
                + &rel("commentsExtended", "ex.xml")
                + &rel("glossaryDocument", "glossary.xml"),
        ))
    }

    /// The affected owner's paragraph absence is unknown; its parent and the other owner survive.
    fn assert_comments(mode: &str) {
        let report = comment_case(mode, false, false);
        eprintln!("comments {mode}: {report:?}");
        assert!(!report.findings.iter().any(|f| f.part.as_deref() == Some("word/ex.xml") && f.message.contains("no paragraph")), "{report:?}");
        assert!(
            report.findings.iter().any(|f| f.part.as_deref() == Some("word/ex.xml") && f.message.contains("paraIdParent 00000008")),
            "{report:?}"
        );
        assert!(report.findings.iter().any(|f| f.part.as_deref() == Some("word/gloss-ex.xml") && f.message.contains("no paragraph")), "{report:?}");
        if mode != "complete" {
            assert!(!report.coverage_notes.iter().any(|n| n.contains("comment-threading paragraphs for word/document.xml")), "{report:?}");
        }
    }

    /// Unsupported encoding on a descendant cuts off a deeper comments branch.
    #[test]
    fn round7_comments_descendant_encoding() {
        assert_comments("encoding");
    }

    /// Malformed descendant relationships leave the same branch undiscovered.
    #[test]
    fn round7_comments_descendant_malformed() {
        assert_comments("malformed");
    }

    /// Complete traversal resolves the actual paragraph; genuine missing paragraphs still warn.
    #[test]
    fn round7_comments_complete_and_genuine_absence() {
        assert_comments("complete");
        let report = comment_case("complete", true, false);
        assert!(report.findings.iter().any(|f| f.part.as_deref() == Some("word/ex.xml") && f.message.contains("no paragraph")), "{report:?}");
    }

    /// A failed descendant shared by both contexts invalidates both pooled paragraph joins.
    #[test]
    fn round7_shared_descendant_affects_each_owner() {
        let report = comment_case("encoding", false, true);
        assert!(report.findings.iter().any(|f| f.part.as_deref() == Some("word/gloss-ex.xml") && f.message.contains("no paragraph")), "{report:?}");
        assert!(report.findings.iter().any(|f| f.message.contains("paraIdParent")), "{report:?}");
    }

    /// Both join sources are selected; descendant discovery loss is independent in each direction.
    fn note_case(mode: &str, reverse: bool, genuine: bool, glossary: bool) -> Report {
        let id = if genuine { "2" } else { "1" };
        let notes = format!(r#"<w:footnotes xmlns:w="{NS_W}"><w:footnote w:id="1" w:type="separator"><w:p/></w:footnote></w:footnotes>"#);
        let settings = format!(r#"<w:settings xmlns:w="{NS_W}"><w:footnotePr><w:footnote w:id="{id}"/></w:footnotePr></w:settings>"#);
        let mut parts = vec![
            ("word/header.xml", package::HEADER_CONTENT_TYPE, format!(r#"<w:hdr xmlns:w="{NS_W}"><w:p/></w:hdr>"#)),
            (
                "word/_rels/header.xml.rels",
                "",
                discovery(mode, &if reverse { rel("footnotes", "notes.xml") } else { rel("settings", "settings.xml") }),
            ),
            ("word/notes.xml", package::FOOTNOTES_CONTENT_TYPE, notes),
            ("word/settings.xml", package::SETTINGS_CONTENT_TYPE, settings),
        ];
        let mut main_rels = rel("header", "header.xml") + &rel("settings", "settings.xml") + &rel("footnotes", "notes.xml");
        if glossary {
            main_rels += &rel("glossaryDocument", "glossary.xml");
            parts.extend([
                (
                    "word/glossary.xml",
                    package::GLOSSARY_CONTENT_TYPE,
                    format!(r#"<w:glossaryDocument xmlns:w="{NS_W}"><w:docParts/></w:glossaryDocument>"#),
                ),
                ("word/_rels/glossary.xml.rels", "", rels(&(rel("footnotes", "gloss-notes.xml") + &rel("settings", "gloss-settings.xml")))),
                (
                    "word/gloss-notes.xml",
                    package::FOOTNOTES_CONTENT_TYPE,
                    format!(r#"<w:footnotes xmlns:w="{NS_W}"><w:footnote w:id="7" w:type="separator"><w:p/></w:footnote></w:footnotes>"#),
                ),
                (
                    "word/gloss-settings.xml",
                    package::SETTINGS_CONTENT_TYPE,
                    format!(r#"<w:settings xmlns:w="{NS_W}"><w:footnotePr><w:footnote w:id="8"/></w:footnotePr></w:settings>"#),
                ),
            ]);
        }
        validate_bytes(&fixture(parts, main_rels))
    }

    /// No manufactured main-owned absence; a complete glossary retains both genuine directions.
    fn assert_notes(mode: &str, reverse: bool) {
        let report = note_case(mode, reverse, false, false);
        eprintln!("notes {mode} reverse={reverse}: {report:?}");
        assert!(!report.findings.iter().any(|f| f.rule.starts_with("NOTE-PR-")), "{report:?}");
        assert_eq!(report.verdict, Verdict::Incomplete, "{report:?}");
        let independent = note_case(mode, reverse, false, true);
        let findings: Vec<_> = independent.findings.iter().filter(|f| f.rule.starts_with("NOTE-PR-")).collect();
        assert_eq!(findings.len(), 2, "{independent:?}");
        assert!(findings.iter().any(|f| f.rule == "NOTE-PR-LISTING" && f.part.as_deref() == Some("word/gloss-notes.xml")), "{independent:?}");
        assert!(findings.iter().any(|f| f.rule == "NOTE-PR-DANGLING" && f.message.contains("id 8,")), "{independent:?}");
    }

    /// Unreadable indirect settings cannot establish an unlisted special note.
    #[test]
    fn round7_note_listing_descendant_encoding() {
        assert_notes("encoding", false);
    }

    /// Malformed indirect settings discovery has the same absence guard.
    #[test]
    fn round7_note_listing_descendant_malformed() {
        assert_notes("malformed", false);
    }

    /// Unreadable indirect definitions cannot establish a dangling settings listing.
    #[test]
    fn round7_note_dangling_descendant_encoding() {
        assert_notes("encoding", true);
    }

    /// Malformed indirect definition discovery has the same absence guard.
    #[test]
    fn round7_note_dangling_descendant_malformed() {
        assert_notes("malformed", true);
    }

    /// Both complete-discovery controls PASS and both genuine-absence directions remain active.
    #[test]
    fn round7_notes_complete_and_genuine_absence() {
        for reverse in [false, true] {
            let report = note_case("complete", reverse, false, false);
            assert_eq!(report.verdict, Verdict::Pass, "{report:?}");
            let report = note_case("complete", reverse, true, false);
            assert!(report.findings.iter().any(|f| f.rule == "NOTE-PR-LISTING"), "{report:?}");
            assert!(report.findings.iter().any(|f| f.rule == "NOTE-PR-DANGLING"), "{report:?}");
        }
    }

    /// A single styles source is shared, but each owner's selected numbering set is independent.
    fn shared_styles(main: &str, glossary: &str) -> Report {
        let numbering = |mode| match mode {
            "unavailable" => r#"<?xml version="1.0" encoding="unsupported-test-encoding"?><w:numbering/>"#.into(),
            "defined" => format!(
                r#"<w:numbering xmlns:w="{NS_W}"><w:abstractNum w:abstractNumId="1"/><w:num w:numId="2"><w:abstractNumId w:val="1"/></w:num></w:numbering>"#
            ),
            _ => format!(r#"<w:numbering xmlns:w="{NS_W}"/>"#),
        };
        validate_bytes(&fixture(
            vec![
                (
                    "word/glossary.xml",
                    package::GLOSSARY_CONTENT_TYPE,
                    format!(r#"<w:glossaryDocument xmlns:w="{NS_W}"><w:docParts/></w:glossaryDocument>"#),
                ),
                ("word/_rels/glossary.xml.rels", "", rels(&(rel("styles", "shared.xml") + &rel("numbering", "gloss-num.xml")))),
                (
                    "word/shared.xml",
                    package::STYLES_CONTENT_TYPE,
                    format!(
                        r#"<w:styles xmlns:w="{NS_W}"><w:style w:styleId="A"><w:pPr><w:numPr><w:numId w:val="2"/></w:numPr></w:pPr></w:style></w:styles>"#
                    ),
                ),
                ("word/main-num.xml", package::NUMBERING_CONTENT_TYPE, numbering(main)),
                ("word/gloss-num.xml", package::NUMBERING_CONTENT_TYPE, numbering(glossary)),
            ],
            rel("glossaryDocument", "glossary.xml") + &rel("styles", "shared.xml") + &rel("numbering", "main-num.xml"),
        ))
    }

    /// Each owner/definition pairing is checked regardless of which owner is first.
    #[test]
    fn round7_shared_styles_each_numbering_pairing() {
        for (main, gloss, count) in [("defined", "empty", 1), ("empty", "defined", 1), ("defined", "defined", 0), ("empty", "empty", 1)] {
            let report = shared_styles(main, gloss);
            eprintln!("shared styles {main}/{gloss}: {report:?}");
            assert_eq!(
                report.findings.iter().filter(|f| f.rule == "NUM-DANGLING" && f.part.as_deref() == Some("word/shared.xml")).count(),
                count,
                "{main}/{gloss}: {report:?}"
            );
        }
    }

    /// An unavailable first set must never consume scheduling for the second complete owner.
    #[test]
    fn round7_shared_styles_unavailable_first_owner() {
        let report = shared_styles("unavailable", "empty");
        eprintln!("shared styles unavailable/empty: {report:?}");
        assert_eq!(report.findings.iter().filter(|f| f.rule == "NUM-DANGLING").count(), 1, "{report:?}");
        let report = shared_styles("unavailable", "defined");
        assert!(!report.findings.iter().any(|f| f.rule == "NUM-DANGLING"), "{report:?}");
    }
    /// Directly selected definitions remain authoritative despite a different failed descendant.
    #[test]
    fn round7_selected_sources_keep_independent_absence_checks() {
        for mode in ["complete", "encoding", "malformed"] {
            let bytes = fixture(vec![
                ("word/header.xml", package::HEADER_CONTENT_TYPE, format!(r#"<w:hdr xmlns:w="{NS_W}"><w:p><w:pPr><w:pStyle w:val="Missing"/><w:numPr><w:numId w:val="2"/></w:numPr></w:pPr><w:r><w:commentReference w:id="3"/><w:footnoteReference w:id="3"/><w:endnoteReference w:id="3"/></w:r></w:p></w:hdr>"#)),
                ("word/_rels/header.xml.rels", "", discovery(mode, "")),
                ("word/styles.xml", package::STYLES_CONTENT_TYPE, format!(r#"<w:styles xmlns:w="{NS_W}"/>"#)),
                ("word/numbering.xml", package::NUMBERING_CONTENT_TYPE, format!(r#"<w:numbering xmlns:w="{NS_W}"><w:num w:numId="1"><w:abstractNumId w:val="9"/></w:num><w:num w:numId="4"/></w:numbering>"#)),
                ("word/comments.xml", package::COMMENTS_CONTENT_TYPE, format!(r#"<w:comments xmlns:w="{NS_W}"/>"#)),
                ("word/footnotes.xml", package::FOOTNOTES_CONTENT_TYPE, format!(r#"<w:footnotes xmlns:w="{NS_W}"/>"#)),
                ("word/endnotes.xml", package::ENDNOTES_CONTENT_TYPE, format!(r#"<w:endnotes xmlns:w="{NS_W}"/>"#)),
                ("word/settings.xml", package::SETTINGS_CONTENT_TYPE, format!(r#"<w:settings xmlns:w="{NS_W}"/>"#)),
                ("word/props.xml", package::CUSTOM_XML_PROPS_CONTENT_TYPE, format!(r#"<ds:datastoreItem xmlns:ds="{NS_DS}"><ds:schemaRefs><ds:schemaRef ds:uri="urn:Missing"/></ds:schemaRefs></ds:datastoreItem>"#)),
                ("word/ex.xml", package::COMMENTS_EXTENDED_CONTENT_TYPE, format!(r#"<w15:commentsEx xmlns:w15="{NS_W15}"><w15:commentEx w15:paraId="00000003" w15:paraIdParent="00000008"/></w15:commentsEx>"#)),
                ("word/ids.xml", package::COMMENTS_IDS_CONTENT_TYPE, r#"<w16cid:commentsIds xmlns:w16cid="http://schemas.microsoft.com/office/word/2016/wordml/cid"><w16cid:commentId w16cid:paraId="00000004" w16cid:durableId="00000001"/></w16cid:commentsIds>"#.into()),
            ], rel("header", "header.xml") + &rel("styles", "styles.xml") + &rel("numbering", "numbering.xml") + &rel("comments", "comments.xml") + &rel("footnotes", "footnotes.xml") + &rel("endnotes", "endnotes.xml") + &rel("settings", "settings.xml") + &rel("customXmlProps", "props.xml") + &rel("commentsExtended", "ex.xml") + &rel("commentsIds", "ids.xml"));
            let report = validate_bytes(&bytes);
            for (rule, count) in [("ID-COMMENT-REF", 1), ("REF-NOTE", 2), ("REF-STYLE", 1), ("NUM-DANGLING", 3)] {
                assert_eq!(report.findings.iter().filter(|f| f.rule == rule).count(), count, "{mode}: {report:?}");
            }
            assert_eq!(
                report
                    .coverage_notes
                    .iter()
                    .filter(|n| *n == "custom XML part word/props.xml declares schema urn:Missing, which is not attached in settings.xml")
                    .count(),
                1,
                "{mode}: {report:?}"
            );
            for message in ["paraIdParent 00000008", "commentsIds paraId 00000004 has no commentEx", "commentEx paraId 00000003 has no commentsIds"] {
                assert!(report.findings.iter().any(|f| f.message.contains(message)), "{mode}: {report:?}");
            }
        }
    }

    /// Reverse comment accounting already rejects unresolved discovery, even behind several edges.
    #[test]
    fn round7_unreferenced_comments_require_descendant_discovery() {
        for mode in ["complete", "encoding", "malformed"] {
            let bytes = fixture(
                vec![
                    (
                        "word/comments.xml",
                        package::COMMENTS_CONTENT_TYPE,
                        format!(
                            r#"<w:comments xmlns:w="{NS_W}"><w:comment w:id="1"><w:p/></w:comment><w:comment w:id="2"><w:p/></w:comment></w:comments>"#
                        ),
                    ),
                    ("word/header.xml", package::HEADER_CONTENT_TYPE, format!(r#"<w:hdr xmlns:w="{NS_W}"><w:p/></w:hdr>"#)),
                    ("word/_rels/header.xml.rels", "", discovery(mode, &rel("header", "deeper.xml"))),
                    (
                        "word/deeper.xml",
                        package::HEADER_CONTENT_TYPE,
                        format!(r#"<w:hdr xmlns:w="{NS_W}"><w:p><w:r><w:commentReference w:id="1"/></w:r></w:p></w:hdr>"#),
                    ),
                ],
                rel("comments", "comments.xml") + &rel("header", "header.xml"),
            );
            let report = validate_bytes(&bytes);
            assert!(!report.coverage_notes.iter().any(|n| n.contains("id 1 in word/comments.xml is never referenced")), "{mode}: {report:?}");
            assert_eq!(
                report.coverage_notes.iter().any(|n| n.contains("id 2 in word/comments.xml is never referenced")),
                mode == "complete",
                "{mode}: {report:?}"
            );
        }
    }
    /// Without an established document, the fallback context includes all parts only after discovery.
    fn fallback_comments(mode: &str) -> Report {
        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let manifest = format!(
            r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/comments.xml" ContentType="{}"/><Override PartName="/ex.xml" ContentType="{}"/></Types>"#,
            package::COMMENTS_CONTENT_TYPE,
            package::COMMENTS_EXTENDED_CONTENT_TYPE
        );
        for (name, text) in [
            ("[Content_Types].xml", manifest),
            (
                "comments.xml",
                format!(
                    r#"<w:comments xmlns:w="{NS_W}" xmlns:w14="{NS_W14}"><w:comment w:id="1"><w:p w14:paraId="00000003"/></w:comment></w:comments>"#
                ),
            ),
            (
                "ex.xml",
                format!(
                    r#"<w15:commentsEx xmlns:w15="{NS_W15}"><w15:commentEx w15:paraId="00000003" w15:paraIdParent="00000008"/></w15:commentsEx>"#
                ),
            ),
            ("header.xml", format!(r#"<w:hdr xmlns:w="{NS_W}"><w:p/></w:hdr>"#)),
            ("_rels/header.xml.rels", discovery(mode, &rel("comments", "comments.xml"))),
        ] {
            zip.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
            zip.write_all(text.as_bytes()).unwrap();
        }
        let bytes = zip.finish().unwrap().into_inner();
        let mut acc = Acc::default();
        let pkg = package::load(&bytes, &mut acc).unwrap();
        assert_eq!(owner_scopes(&pkg), vec![OwnerContext::Fallback]);
        validate_bytes(&bytes)
    }

    /// Unknown fallback ownership must not turn an existing paragraph into an absent one.
    fn assert_fallback_comments(mode: &str) {
        let report = fallback_comments(mode);
        eprintln!("fallback comments {mode}: {report:?}");
        assert!(!report.findings.iter().any(|f| f.message.contains("no paragraph")), "{report:?}");
        assert!(report.findings.iter().any(|f| f.message.contains("paraIdParent 00000008")), "{report:?}");
        if mode != "complete" {
            assert!(report.coverage_notes.iter().any(|n| n.contains("comment-threading paragraphs for the package")), "{report:?}");
        }
    }

    /// Unsupported descendant declarations invalidate the fallback ownership assignment too.
    #[test]
    fn round7_fallback_descendant_encoding() {
        assert_fallback_comments("encoding");
    }

    /// Malformed descendant declarations leave the fallback's contributing set unknown.
    #[test]
    fn round7_fallback_descendant_malformed() {
        assert_fallback_comments("malformed");
    }

    /// Complete fallback discovery still resolves the paragraph and checks the genuine parent link.
    #[test]
    fn round7_fallback_complete_control() {
        assert_fallback_comments("complete");
    }
}

#[cfg(test)]
mod round8_fallback_contributor_tests {
    //! Unknown fallback sources must not shrink a primary owner's pooled definition sets.
    use super::*;
    use std::io::{Cursor, Write};
    /// Wrap relationship declarations without relying on conventional supporting-part names.
    fn rels(children: &str) -> String {
        format!(r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">{children}</Relationships>"#)
    }

    /// A unique internal relationship in the supported namespace.
    fn rel(kind: &str, target: &str) -> String {
        format!(r#"<Relationship Id="{target}" Type="{NS_R}/{kind}" Target="{target}"/>"#)
    }

    /// Build an entirely synthetic package with explicit content types.
    fn fixture(parts: Vec<(&str, &str, String)>, main_rels: String) -> Vec<u8> {
        let mut overrides = format!(r#"<Override PartName="/word/document.xml" ContentType="{}"/>"#, package::MAIN_CONTENT_TYPE);
        for (name, ct, _) in &parts {
            if !ct.is_empty() {
                overrides.push_str(&format!(r#"<Override PartName="/{name}" ContentType="{ct}"/>"#));
            }
        }
        let manifest = format!(
            r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/>{overrides}</Types>"#
        );
        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for (name, text) in [
            ("[Content_Types].xml", manifest),
            ("_rels/.rels", rels(&rel("officeDocument", "word/document.xml"))),
            ("word/document.xml", format!(r#"<w:document xmlns:w="{NS_W}"><w:body><w:p/></w:body></w:document>"#)),
            ("word/_rels/document.xml.rels", rels(&main_rels)),
        ]
        .into_iter()
        .chain(parts.into_iter().map(|(name, _, text)| (name, text)))
        {
            zip.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
            zip.write_all(text.as_bytes()).unwrap();
        }
        zip.finish().unwrap().into_inner()
    }

    /// Fail before the downstream edge, or retain it in the complete control.
    fn discovery(mode: &str, children: &str) -> String {
        match mode {
            "encoding" => r#"<?xml version="1.0" encoding="unsupported-test-encoding"?><Relationships/>"#.into(),
            "malformed" => "<Relationships><broken></Relationships>".into(),
            _ => rels(children),
        }
    }

    /// The failed relationship part and its owner are outside both reached document graphs.
    fn unrelated(mode: &str) -> Vec<(&'static str, &'static str, String)> {
        vec![("word/unrelated.xml", "application/xml", "<unrelated/>".into()), ("word/_rels/unrelated.xml.rels", "", discovery(mode, ""))]
    }

    /// Both consumed sources are selected, with unrelated fallback discovery loss retained.
    fn notes(mode: &str, reverse: bool, end: bool, genuine: bool, glossary: bool) -> Report {
        let (note, ct) = if end { ("endnote", package::ENDNOTES_CONTENT_TYPE) } else { ("footnote", package::FOOTNOTES_CONTENT_TYPE) };
        let listing = if genuine { "2" } else { "1" };
        let mut parts = unrelated(mode);
        parts.extend([
            ("word/notes.xml", ct, format!(r#"<w:{note}s xmlns:w="{NS_W}"><w:{note} w:id="1" w:type="separator"><w:p/></w:{note}></w:{note}s>"#)),
            (
                "word/settings.xml",
                package::SETTINGS_CONTENT_TYPE,
                format!(r#"<w:settings xmlns:w="{NS_W}"><w:{note}Pr><w:{note} w:id="{listing}"/></w:{note}Pr></w:settings>"#),
            ),
        ]);
        let mut main = if reverse {
            rel("settings", "settings.xml") + &rel(&format!("{note}s"), "notes.xml")
        } else {
            rel(&format!("{note}s"), "notes.xml") + &rel("settings", "settings.xml")
        };
        if glossary {
            main += &rel("glossaryDocument", "glossary.xml");
            parts.extend([
                (
                    "word/glossary.xml",
                    package::GLOSSARY_CONTENT_TYPE,
                    format!(r#"<w:glossaryDocument xmlns:w="{NS_W}"><w:docParts/></w:glossaryDocument>"#),
                ),
                ("word/_rels/glossary.xml.rels", "", rels(&(rel(&format!("{note}s"), "gloss-notes.xml") + &rel("settings", "gloss-settings.xml")))),
                (
                    "word/gloss-notes.xml",
                    ct,
                    format!(r#"<w:{note}s xmlns:w="{NS_W}"><w:{note} w:id="7" w:type="separator"><w:p/></w:{note}></w:{note}s>"#),
                ),
                (
                    "word/gloss-settings.xml",
                    package::SETTINGS_CONTENT_TYPE,
                    format!(r#"<w:settings xmlns:w="{NS_W}"><w:{note}Pr><w:{note} w:id="8"/></w:{note}Pr></w:settings>"#),
                ),
            ]);
        }
        validate_bytes(&fixture(parts, main))
    }

    /// Print the complete observed report before checking the regression expectation.
    fn assert_notes(mode: &str, reverse: bool, end: bool) {
        let report = notes(mode, reverse, end, false, false);
        eprintln!("round8 notes {mode} reverse={reverse} end={end}: {report:?}");
        assert!(!report.findings.iter().any(|f| f.rule.starts_with("NOTE-PR-")), "{report:?}");
        assert_eq!(report.verdict, Verdict::Incomplete, "{report:?}");
    }

    /// A directly reached extension joins fallback paragraphs, but its parent/counterparts are local.
    fn comments(mode: &str, genuine: bool, glossary: bool) -> Report {
        let para = if genuine { "00000009" } else { "00000003" };
        let mut parts = unrelated(mode);
        parts.extend([
            ("word/comments.xml", package::COMMENTS_CONTENT_TYPE, format!(r#"<w:comments xmlns:w="{NS_W}" xmlns:w14="{NS_W14}"><w:comment w:id="1"><w:p w14:paraId="00000003"/></w:comment></w:comments>"#)),
            ("word/ex.xml", package::COMMENTS_EXTENDED_CONTENT_TYPE, format!(r#"<w15:commentsEx xmlns:w15="{NS_W15}"><w15:commentEx w15:paraId="{para}" w15:paraIdParent="00000008"/></w15:commentsEx>"#)),
            ("word/ids.xml", package::COMMENTS_IDS_CONTENT_TYPE, r#"<w16cid:commentsIds xmlns:w16cid="http://schemas.microsoft.com/office/word/2016/wordml/cid"><w16cid:commentId w16cid:paraId="00000004" w16cid:durableId="00000001"/></w16cid:commentsIds>"#.into()),
        ]);
        let mut main = rel("comments", "comments.xml") + &rel("commentsExtended", "ex.xml") + &rel("commentsIds", "ids.xml");
        if glossary {
            main += &rel("glossaryDocument", "glossary.xml");
            parts.extend([
                (
                    "word/glossary.xml",
                    package::GLOSSARY_CONTENT_TYPE,
                    format!(r#"<w:glossaryDocument xmlns:w="{NS_W}"><w:docParts/></w:glossaryDocument>"#),
                ),
                ("word/_rels/glossary.xml.rels", "", rels(&rel("commentsExtended", "gloss-ex.xml"))),
                (
                    "word/gloss-ex.xml",
                    package::COMMENTS_EXTENDED_CONTENT_TYPE,
                    format!(r#"<w15:commentsEx xmlns:w15="{NS_W15}"><w15:commentEx w15:paraId="00000009"/></w15:commentsEx>"#),
                ),
            ]);
        }
        validate_bytes(&fixture(parts, main))
    }

    fn assert_comments(mode: &str) {
        let report = comments(mode, false, false);
        eprintln!("round8 comments {mode}: {report:?}");
        assert!(!report.findings.iter().any(|f| f.message.contains("no paragraph")), "{report:?}");
        for message in ["paraIdParent 00000008", "commentsIds paraId 00000004 has no commentEx", "commentEx paraId 00000003 has no commentsIds"] {
            assert!(report.findings.iter().any(|f| f.message.contains(message)), "{report:?}");
        }
        if mode != "complete" {
            assert_eq!(report.verdict, Verdict::Incomplete, "{report:?}");
            assert!(!report.coverage_notes.iter().any(|n| n.contains("comment-threading paragraphs for word/document.xml")), "{report:?}");
        }
    }
    #[test]
    fn round8_footnote_listing_encoding() {
        assert_notes("encoding", false, false);
    }
    #[test]
    fn round8_footnote_listing_malformed() {
        assert_notes("malformed", false, false);
    }
    #[test]
    fn round8_footnote_dangling_encoding() {
        assert_notes("encoding", true, false);
    }
    #[test]
    fn round8_footnote_dangling_malformed() {
        assert_notes("malformed", true, false);
    }
    #[test]
    fn round8_endnote_listing_encoding() {
        assert_notes("encoding", false, true);
    }
    #[test]
    fn round8_endnote_listing_malformed() {
        assert_notes("malformed", false, true);
    }
    #[test]
    fn round8_endnote_dangling_encoding() {
        assert_notes("encoding", true, true);
    }
    #[test]
    fn round8_endnote_dangling_malformed() {
        assert_notes("malformed", true, true);
    }

    #[test]
    fn round8_commentex_encoding() {
        assert_comments("encoding");
    }
    #[test]
    fn round8_commentex_malformed() {
        assert_comments("malformed");
    }
    #[test]
    fn round8_complete_notes_and_genuine_findings() {
        for end in [false, true] {
            for reverse in [false, true] {
                let report = notes("complete", reverse, end, false, false);
                eprintln!("round8 complete notes reverse={reverse} end={end}: {report:?}");
                assert_eq!(report.verdict, Verdict::Pass, "{report:?}");
                let report = notes("complete", reverse, end, true, false);
                for rule in ["NOTE-PR-LISTING", "NOTE-PR-DANGLING"] {
                    assert!(report.findings.iter().any(|f| f.rule == rule), "{report:?}");
                }
            }
        }
    }
    #[test]
    fn round8_complete_comments_and_genuine_findings() {
        assert_comments("complete");
        let report = comments("complete", true, false);
        assert!(report.findings.iter().any(|f| f.message.contains("no paragraph")), "{report:?}");
    }
    #[test]
    fn round8_disjoint_glossary_notes_survive() {
        for mode in ["complete", "encoding", "malformed"] {
            for end in [false, true] {
                for reverse in [false, true] {
                    let report = notes(mode, reverse, end, false, true);
                    let findings: Vec<_> = report.findings.iter().filter(|f| f.rule.starts_with("NOTE-PR-")).collect();
                    assert_eq!(findings.len(), 2, "{report:?}");
                    assert!(findings.iter().any(|f| f.rule == "NOTE-PR-LISTING" && f.part.as_deref() == Some("word/gloss-notes.xml")), "{report:?}");
                    assert!(findings.iter().any(|f| f.rule == "NOTE-PR-DANGLING" && f.message.contains("id 8,")), "{report:?}");
                }
            }
        }
    }
    #[test]
    fn round8_disjoint_glossary_comments_survive() {
        for mode in ["complete", "encoding", "malformed"] {
            let report = comments(mode, false, true);
            let findings: Vec<_> = report.findings.iter().filter(|f| f.message.contains("no paragraph")).collect();
            assert_eq!(findings.len(), 1, "{report:?}");
            assert_eq!(findings[0].part.as_deref(), Some("word/gloss-ex.xml"));
        }
    }
    /// Directly selected definitions remain authoritative despite a different failed descendant.
    #[test]
    fn round8_selected_sources_keep_independent_absence_checks() {
        for mode in ["complete", "encoding", "malformed"] {
            let bytes = fixture(vec![
                ("word/header.xml", package::HEADER_CONTENT_TYPE, format!(r#"<w:hdr xmlns:w="{NS_W}"><w:p><w:pPr><w:pStyle w:val="Missing"/><w:numPr><w:numId w:val="2"/></w:numPr></w:pPr><w:r><w:commentReference w:id="3"/><w:footnoteReference w:id="3"/><w:endnoteReference w:id="3"/></w:r></w:p></w:hdr>"#)),
                ("word/unrelated.xml", "application/xml", "<unrelated/>".into()),
                ("word/_rels/unrelated.xml.rels", "", discovery(mode, "")),
                ("word/styles.xml", package::STYLES_CONTENT_TYPE, format!(r#"<w:styles xmlns:w="{NS_W}"/>"#)),
                ("word/numbering.xml", package::NUMBERING_CONTENT_TYPE, format!(r#"<w:numbering xmlns:w="{NS_W}"><w:num w:numId="1"><w:abstractNumId w:val="9"/></w:num><w:num w:numId="4"/></w:numbering>"#)),
                ("word/comments.xml", package::COMMENTS_CONTENT_TYPE, format!(r#"<w:comments xmlns:w="{NS_W}"/>"#)),
                ("word/footnotes.xml", package::FOOTNOTES_CONTENT_TYPE, format!(r#"<w:footnotes xmlns:w="{NS_W}"/>"#)),
                ("word/endnotes.xml", package::ENDNOTES_CONTENT_TYPE, format!(r#"<w:endnotes xmlns:w="{NS_W}"/>"#)),
                ("word/settings.xml", package::SETTINGS_CONTENT_TYPE, format!(r#"<w:settings xmlns:w="{NS_W}"/>"#)),
                ("word/props.xml", package::CUSTOM_XML_PROPS_CONTENT_TYPE, format!(r#"<ds:datastoreItem xmlns:ds="{NS_DS}"><ds:schemaRefs><ds:schemaRef ds:uri="urn:Missing"/></ds:schemaRefs></ds:datastoreItem>"#)),
                ("word/ex.xml", package::COMMENTS_EXTENDED_CONTENT_TYPE, format!(r#"<w15:commentsEx xmlns:w15="{NS_W15}"><w15:commentEx w15:paraId="00000003" w15:paraIdParent="00000008"/></w15:commentsEx>"#)),
                ("word/ids.xml", package::COMMENTS_IDS_CONTENT_TYPE, r#"<w16cid:commentsIds xmlns:w16cid="http://schemas.microsoft.com/office/word/2016/wordml/cid"><w16cid:commentId w16cid:paraId="00000004" w16cid:durableId="00000001"/></w16cid:commentsIds>"#.into()),
            ], rel("header", "header.xml") + &rel("styles", "styles.xml") + &rel("numbering", "numbering.xml") + &rel("comments", "comments.xml") + &rel("footnotes", "footnotes.xml") + &rel("endnotes", "endnotes.xml") + &rel("settings", "settings.xml") + &rel("customXmlProps", "props.xml") + &rel("commentsExtended", "ex.xml") + &rel("commentsIds", "ids.xml"));
            let report = validate_bytes(&bytes);
            for (rule, count) in [("ID-COMMENT-REF", 1), ("REF-NOTE", 2), ("REF-STYLE", 1), ("NUM-DANGLING", 3)] {
                assert_eq!(report.findings.iter().filter(|f| f.rule == rule).count(), count, "{mode}: {report:?}");
            }
            assert_eq!(
                report
                    .coverage_notes
                    .iter()
                    .filter(|n| *n == "custom XML part word/props.xml declares schema urn:Missing, which is not attached in settings.xml")
                    .count(),
                1,
                "{mode}: {report:?}"
            );
            for message in ["paraIdParent 00000008", "commentsIds paraId 00000004 has no commentEx", "commentEx paraId 00000003 has no commentsIds"] {
                assert!(report.findings.iter().any(|f| f.message.contains(message)), "{mode}: {report:?}");
            }
        }
    }

    /// An unknown noncontributing source must not disable complete primary-owner pools.
    #[test]
    fn round8_unknown_irrelevant_parts_preserve_primary_findings() {
        for mode in ["encoding", "malformed"] {
            let mut parts = unrelated(mode);
            parts.extend([
                (
                    "word/notes.xml",
                    package::FOOTNOTES_CONTENT_TYPE,
                    format!(r#"<w:footnotes xmlns:w="{NS_W}"><w:footnote w:id="1" w:type="separator"><w:p/></w:footnote></w:footnotes>"#),
                ),
                (
                    "word/settings.xml",
                    package::SETTINGS_CONTENT_TYPE,
                    format!(r#"<w:settings xmlns:w="{NS_W}"><w:footnotePr><w:footnote w:id="2"/></w:footnotePr></w:settings>"#),
                ),
                (
                    "word/ex.xml",
                    package::COMMENTS_EXTENDED_CONTENT_TYPE,
                    format!(r#"<w15:commentsEx xmlns:w15="{NS_W15}"><w15:commentEx w15:paraId="00000009"/></w15:commentsEx>"#),
                ),
            ]);
            let report = validate_bytes(&fixture(
                parts,
                rel("footnotes", "notes.xml") + &rel("settings", "settings.xml") + &rel("commentsExtended", "ex.xml"),
            ));
            for rule in ["NOTE-PR-LISTING", "NOTE-PR-DANGLING", "COMMENTEX-LINK"] {
                assert!(report.findings.iter().any(|f| f.rule == rule), "{report:?}");
            }
        }
    }
}
