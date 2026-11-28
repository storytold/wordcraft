//! Package model: bounded zip loading, content-type resolution, relationship parsing and the
//! reachability graph shared by the checks.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::io::{Cursor, Read};

use crate::xml::XmlReader;
use quick_xml::events::{BytesDecl, BytesStart, Event};
use quick_xml::name::{NamespaceResolver, PrefixDeclaration, ResolveResult};

use crate::Acc;
use crate::resolution::Resolution;
use crate::scan::ParseGuard;

/// The main document part's content type (ECMA-376 Part 1 §11.3.10).
pub(crate) const MAIN_CONTENT_TYPE: &str = "application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml";
/// The template main-document content type (ECMA-376 Part 1 §11.3.10).
pub(crate) const TEMPLATE_MAIN_CONTENT_TYPE: &str = "application/vnd.openxmlformats-officedocument.wordprocessingml.template.main+xml";
/// The macro-enabled main-document content type (`.docm`).
pub(crate) const MACRO_MAIN_CONTENT_TYPE: &str = "application/vnd.ms-word.document.macroEnabled.main+xml";
/// The macro-enabled template main-document content type (`.dotm`).
pub(crate) const MACRO_TEMPLATE_MAIN_CONTENT_TYPE: &str = "application/vnd.ms-word.template.macroEnabled.main+xml";
/// Every content type a package's main document part may carry: document, template, and their
/// macro-enabled variants. They are spellings of one main type, not different kinds of part.
pub(crate) const MAIN_CONTENT_TYPES: &[&str] =
    &[MAIN_CONTENT_TYPE, TEMPLATE_MAIN_CONTENT_TYPE, MACRO_MAIN_CONTENT_TYPE, MACRO_TEMPLATE_MAIN_CONTENT_TYPE];

/// Whether `ct` is one of the [`MAIN_CONTENT_TYPES`].
pub(crate) fn is_main_content_type(ct: &str) -> bool {
    MAIN_CONTENT_TYPES.contains(&ct)
}
/// A header part's content type.
pub(crate) const HEADER_CONTENT_TYPE: &str = "application/vnd.openxmlformats-officedocument.wordprocessingml.header+xml";
/// A footer part's content type.
pub(crate) const FOOTER_CONTENT_TYPE: &str = "application/vnd.openxmlformats-officedocument.wordprocessingml.footer+xml";
/// The comments part's content type.
pub(crate) const COMMENTS_CONTENT_TYPE: &str = "application/vnd.openxmlformats-officedocument.wordprocessingml.comments+xml";
/// The commentsExtended part's content type.
pub(crate) const COMMENTS_EXTENDED_CONTENT_TYPE: &str = "application/vnd.openxmlformats-officedocument.wordprocessingml.commentsExtended+xml";
/// The commentsIds part's content type.
pub(crate) const COMMENTS_IDS_CONTENT_TYPE: &str = "application/vnd.openxmlformats-officedocument.wordprocessingml.commentsIds+xml";
/// The footnotes part's content type.
pub(crate) const FOOTNOTES_CONTENT_TYPE: &str = "application/vnd.openxmlformats-officedocument.wordprocessingml.footnotes+xml";
/// The endnotes part's content type.
pub(crate) const ENDNOTES_CONTENT_TYPE: &str = "application/vnd.openxmlformats-officedocument.wordprocessingml.endnotes+xml";
/// The numbering part's content type.
pub(crate) const NUMBERING_CONTENT_TYPE: &str = "application/vnd.openxmlformats-officedocument.wordprocessingml.numbering+xml";
/// The styles part's content type.
pub(crate) const STYLES_CONTENT_TYPE: &str = "application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml";
/// The settings part's content type.
pub(crate) const SETTINGS_CONTENT_TYPE: &str = "application/vnd.openxmlformats-officedocument.wordprocessingml.settings+xml";
/// The font table part's content type.
pub(crate) const FONT_TABLE_CONTENT_TYPE: &str = "application/vnd.openxmlformats-officedocument.wordprocessingml.fontTable+xml";
/// The theme part's content type.
pub(crate) const THEME_CONTENT_TYPE: &str = "application/vnd.openxmlformats-officedocument.theme+xml";
/// The glossary document part's content type.
pub(crate) const GLOSSARY_CONTENT_TYPE: &str = "application/vnd.openxmlformats-officedocument.wordprocessingml.document.glossary+xml";
/// The web-settings part's content type (ECMA-376 Part 1 §11.3.13).
pub(crate) const WEB_SETTINGS_CONTENT_TYPE: &str = "application/vnd.openxmlformats-officedocument.wordprocessingml.webSettings+xml";
/// The people part's content type (MS-DOCX; absent from ECMA-376 Part 1).
pub(crate) const PEOPLE_CONTENT_TYPE: &str = "application/vnd.openxmlformats-officedocument.wordprocessingml.people+xml";
/// The chart part's content type (ECMA-376 Part 1 §14.2.1).
pub(crate) const CHART_CONTENT_TYPE: &str = "application/vnd.openxmlformats-officedocument.drawingml.chart+xml";
/// The plain-XML content type a custom XML item and a bibliography part require
/// (ECMA-376 Part 1 §§15.2.5, 15.2.3).
pub(crate) const XML_CONTENT_TYPE: &str = "application/xml";
/// A custom-XML item-properties part's content type.
pub(crate) const CUSTOM_XML_PROPS_CONTENT_TYPE: &str = "application/vnd.openxmlformats-officedocument.customXmlProperties+xml";
/// The custom document-properties part's content type (docProps/custom.xml).
pub(crate) const CUSTOM_PROPERTIES_CONTENT_TYPE: &str = "application/vnd.openxmlformats-officedocument.custom-properties+xml";

/// One zip entry, held once for every check.
pub(crate) struct Entry {
    pub name: String,
    /// `None` when the part is a directory, too large, or could not be decompressed.
    pub bytes: Option<Vec<u8>>,
    pub is_dir: bool,
    /// The strictly-decoded text of an XML or `.rels` part; every consumer reads this, never the
    /// raw bytes.
    pub decoded: Option<String>,
    /// Why decoding failed, when it did.
    pub decode_error: Option<crate::scan::DecodeError>,
    /// The part is present in the package but its content could not be read (a blank entry, or one
    /// that failed strict decoding). Cross-part checks that would conclude *absence* from this
    /// part's content must skip instead: absence is not established from unavailable content.
    pub content_unavailable: bool,
}

/// `[Content_Types].xml`: Override by part name, else Default by extension (both case-insensitive).
/// Duplicate declarations are recorded too, for `CT-DUP-DEFAULT`/`CT-DUP-OVERRIDE`.
#[derive(Default)]
pub(crate) struct ContentTypes {
    overrides: HashMap<String, String>,
    /// Later declarations were not examined; absence and Default fallbacks are unestablished.
    pub incomplete: bool,
    defaults: HashMap<String, String>,
    /// Extensions with more than one Default entry (lowercased).
    pub dup_defaults: BTreeSet<String>,
    /// Part names with more than one Override entry (lowercased, leading `/` stripped).
    pub dup_overrides: BTreeSet<String>,
}

impl ContentTypes {
    /// The effective content type of `part` (a package path, leading `/` ignored).
    pub fn of(&self, part: &str) -> Option<&str> {
        observation_contract! {
            rules: "-";
            collections: "field.defaults*1,field.incomplete*1,field.overrides*1";
            scope: "source-local bounded extraction; no implicit document owner";
            loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
            fallback: "retain proven prefix presence and duplicates; return loss to the caller; no negative membership from an incomplete set";
            package_loss: "independent";
        }

        let part = part.trim_start_matches('/').to_ascii_lowercase();
        if let Some(t) = self.overrides.get(&part) {
            return Some(t);
        }
        // An unexamined Override can supersede any recorded Default.
        if self.incomplete {
            return None;
        }
        let ext = part.rsplit_once('.').map(|(_, e)| e)?;
        self.defaults.get(ext).map(String::as_str)
    }
}

/// Recognized relationship classification, parsed once when loading the declaration.
pub(crate) enum RelationKind {
    Supported(String),
    StrictEntry,
    Other,
}
impl RelationKind {
    /// Custom suffix lookalikes never acquire official roles.
    fn parse(uri: &str) -> Self {
        if uri == "http://purl.oclc.org/ooxml/officeDocument/relationships/officeDocument" {
            Self::StrictEntry
        } else if let Some(kind) = relationship_kind(uri) {
            Self::Supported(kind.to_string())
        } else {
            Self::Other
        }
    }
    /// Only the supported official vocabulary supplies document roles.
    pub(crate) fn kind(&self) -> Option<&str> {
        match self {
            Self::Supported(kind) => Some(kind),
            Self::StrictEntry | Self::Other => None,
        }
    }
}

/// One relationship.
pub(crate) struct Rel {
    pub id: crate::scan::RelId,
    pub kind: String,
    pub role: RelationKind,
    /// Resolved package path for internal targets; `None` for external ones.
    pub target_abs: Option<String>,
    pub external: bool,
}

/// Bind lexical relationship IDs consistently: the first declaration in XML order wins.
/// Duplicate declarations still produce REL-DUP-ID, and every declaration retains its
/// independent target/type checks. Reordering duplicate declarations changes binding evidence.
pub(crate) fn relationship_bindings(rp: &RelsPart) -> RelationshipBindings<'_> {
    observation_contract! {
        rules: "-";
        collections: "field.incomplete*1,field.rels*1,type.HashMap*1";
        scope: "per relationships part and lexical ID; first XML declaration binds; duplicates retain independent REL-DUP-ID and per-declaration target checks";
        loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
        fallback: "retained first bindings prove presence even in an incomplete suffix; missing IDs require complete declarations";
        package_loss: "independent";
    }

    let mut bindings = HashMap::new();
    for rel in &rp.rels {
        bindings.entry(rel.id.as_str()).or_insert(rel);
    }
    RelationshipBindings { first: bindings, complete: !rp.incomplete }
}

/// A sealed first-declaration index carrying completeness of the declaring XML source.
pub(crate) struct RelationshipBindings<'a> {
    first: HashMap<&'a str, &'a Rel>,
    complete: bool,
}
impl<'a> RelationshipBindings<'a> {
    /// A missing ID is Absent only after complete declaration discovery; prefix bindings survive.
    pub(crate) fn resolve(&self, id: &str) -> Resolution<&'a Rel> {
        observation_contract! {
            rules: "-";
            collections: "field.first*1";
            scope: "source-local bounded extraction; no implicit document owner";
            loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
            fallback: "retain proven prefix presence and duplicates; return loss to the caller; no negative membership from an incomplete set";
            package_loss: "independent";
        }

        Resolution::discovered(self.first.get(id).copied(), self.complete)
    }
    /// Presence-only traversal of retained bindings, never negative membership evidence.
    pub(crate) fn values(&self) -> impl Iterator<Item = &'a Rel> + '_ {
        observation_contract! {
            rules: "-";
            collections: "field.first*1";
            scope: "source-local bounded extraction; no implicit document owner";
            loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
            fallback: "retain proven prefix presence and duplicates; return loss to the caller; no negative membership from an incomplete set";
            package_loss: "independent";
        }

        self.first.values().copied()
    }
    /// The declaration actually bound for this ID; other duplicates retain their independent checks.
    pub(crate) fn is_first(&self, rel: &Rel) -> bool {
        matches!(self.resolve(rel.id.as_str()), Resolution::One(first) if std::ptr::eq(first, rel))
    }
}

/// One `.rels` part.
pub(crate) struct RelsPart {
    pub name: String,
    pub rels: Vec<Rel>,
    /// Later relationships were not examined, so missing IDs/kinds are unknown.
    pub incomplete: bool,
    /// A negative package-wide target lookup requires a well-formed OPC container with no lost declarations.
    pub inventory_complete: bool,
}

/// Everything loaded from a package once.
pub(crate) struct Package {
    pub entries: Vec<Entry>,
    /// Decoded, leading-slash-trimmed names with unexamined competing records.
    pub collapsed_names: HashSet<String>,
    pub content_types: ContentTypes,
    pub content_types_present: bool,
    pub rels: Vec<RelsPart>,
    pub main: Resolution<String>,
    /// The uniquely selected glossary, from the main document's relationships or content type.
    pub glossary: Resolution<String>,
    /// Competing document roots leave owner-dependent checks without an established scope.
    pub roots_ambiguous: bool,
    /// Lowercased names of parts reachable from the package root.
    pub reachable: HashSet<String>,
    /// Lowercased names of parts classified as XML by the single classification shared by loading,
    /// decoding and the XML pass (J4).
    pub xml_parts: HashSet<String>,
    /// The archive declared more entries than the entry budget: the
    /// examined entry list is a prefix, so a part absent from
    /// `entries` is not proven absent from the archive.
    pub entries_truncated: bool,
    /// An unreadable archive entry had no recoverable identity and could be any source.
    pub identity_incomplete: bool,
}

/// Load a zip into the package model. Returns `None` only when the input is not a readable zip
/// (after a `PKG-NOT-ZIP` finding): every other problem becomes a finding or a coverage note.
pub(crate) fn load(bytes: &[u8], acc: &mut Acc) -> Option<Package> {
    observation_contract! {
        rules: "PKG-NO-CONTENT-TYPES,PKG-NO-MAIN,PKG-NOT-ZIP";
        collections: "field.bytes*1,field.entries*6,field.incomplete*5,field.rels*1,parser.parse_content_types*1,type.Vec*2";
        scope: "source-local bounded extraction; no implicit document owner";
        loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
        fallback: "retain proven prefix presence and duplicates; return loss to the caller; no negative membership from an incomplete set";
        package_loss: "PKG-NO-CONTENT-TYPES,PKG-NO-MAIN";
    }

    acc.ran("PKG-NOT-ZIP");
    let Ok(mut zip) = zip::ZipArchive::new(Cursor::new(bytes)) else {
        acc.add("PKG-NOT-ZIP", None, "the input is not a readable zip archive".to_string());
        return None;
    };
    let limits = acc.limits;
    let entries_truncated = zip.len() > limits.entries;
    if entries_truncated {
        acc.note(format!("the archive declares {} entries; only the first {} were examined", zip.len(), limits.entries));
        acc.set_incomplete();
    }
    let limit = zip.len().min(limits.entries);
    let mut entries = Vec::new();
    let mut total: u64 = 0;
    // Set when an entry that failed to open could not be named at all. Its identity is unknown, so
    // nothing can be concluded about what is absent from the package while it is set.
    let mut unknown_unreadable = false;
    for i in 0..limit {
        // The identity is read before opening: `by_index` holds a mutable borrow of the archive for
        // the whole match, so the name must be cloned first. Both come from the same table, keyed by
        // the same index, so the recovered name can never disagree with the entry that failed.
        let name = zip.name_for_index(i).map(|n| n.trim_start_matches('/').to_string());
        let mut f = match zip.by_index(i) {
            Ok(f) => f,
            Err(_) => {
                let described = match &name {
                    Some(n) => format!("entry {i} ({n})"),
                    None => format!("entry {i}"),
                };
                acc.note(format!("{described} could not be read from the archive and was not checked"));
                acc.set_incomplete();
                match name {
                    // A directory entry: never register it as a part (the readable path skips
                    // directories too, and the unreadable path must not invent one).
                    Some(n) if n.ends_with('/') => entries.push(blank_entry(n, true)),
                    // Register the part as present-but-unchecked. Its content is marked unavailable,
                    // which is what makes every cross-part check that would conclude absence from it
                    // skip; the marker, not the mere registration, is the mechanism.
                    Some(n) => entries.push(blank_entry(n, false)),
                    // `name_for_index` and `by_index` index the same table and the loop is bounded
                    // by `zip.len()`, so this should be unreachable. If it happens, an entry of
                    // unknown identity exists: it could be any part, so no absence conclusion is
                    // established (see `Acc::set_unknown_unreadable` and `ABSENCE_RULES`).
                    None => unknown_unreadable = true,
                }
                continue;
            }
        };
        let name = f.name().trim_start_matches('/').to_string();
        if f.is_dir() {
            entries.push(blank_entry(name, true));
            continue;
        }
        let size = f.size();
        if size > limits.part_bytes {
            acc.note(format!("part {name} is larger than the {}-byte per-part limit and was not checked", limits.part_bytes));
            acc.set_incomplete();
            entries.push(blank_entry(name, false));
            continue;
        }
        if total.saturating_add(size) > limits.total_bytes {
            acc.note(format!("part {name} would take the package past the {}-byte total limit and was not checked", limits.total_bytes));
            acc.set_incomplete();
            entries.push(blank_entry(name, false));
            continue;
        }
        let remaining_total = limits.total_bytes.saturating_sub(total);
        let read_cap = limits.part_bytes.min(remaining_total);
        let mut buf = Vec::with_capacity(size.min(read_cap).min(1 << 20) as usize);
        match (&mut f).take(read_cap.saturating_add(1)).read_to_end(&mut buf) {
            Ok(_) if buf.len() as u64 > limits.part_bytes => {
                total = total.saturating_add(buf.len() as u64);
                acc.note(format!("part {name} decompresses past the {}-byte per-part limit and was not checked", limits.part_bytes));
                acc.set_incomplete();
                entries.push(blank_entry(name, false));
            }
            Ok(_) if total.saturating_add(buf.len() as u64) > limits.total_bytes => {
                total = total.saturating_add(buf.len() as u64);
                acc.note(format!("part {name} would take the package past the {}-byte total limit and was not checked", limits.total_bytes));
                acc.set_incomplete();
                entries.push(blank_entry(name, false));
            }
            Ok(_) => {
                total = total.saturating_add(buf.len() as u64);
                entries.push(text_entry(name, buf));
            }
            Err(_) => {
                // Count partially decompressed bytes from corrupt streams against the package
                // decompression budget so repeated failing entries cannot exceed total_bytes.
                total = total.saturating_add(buf.len() as u64);
                // Any part whose content could not be read leaves the run INCOMPLETE: a check that
                // depended on it could not run. (This subsumes the old main-part special case.)
                acc.note(format!("part {name} could not be decompressed and was not checked"));
                acc.set_incomplete();
                entries.push(blank_entry(name, false));
            }
        }
    }

    let (cd, directory_disagreement) = reader_checked_directory(bytes, &zip, acc.limits.entries, acc);
    unknown_unreadable |= directory_disagreement;
    if directory_disagreement {
        // The reader may have retained an arbitrary decoded-name twin. Even a
        // source-local finding would attribute that record's contents to an
        // unproven part identity, so do not inspect the selected bodies.
        acc.note("the selected entry bodies were not checked because their directory identities could not be established".to_string());
        acc.set_unknown_unreadable();
        acc.set_incomplete();
        return None;
    }
    let (collapsed_names, unexplained) = check_duplicate_entries(cd.as_ref(), zip.len(), acc);
    unknown_unreadable |= unexplained;
    if unknown_unreadable {
        acc.set_unknown_unreadable();
    }
    check_case_collisions(cd.as_ref(), &entries, acc);

    let ct_at = entries.iter().position(|e| !e.is_dir && e.name.eq_ignore_ascii_case("[Content_Types].xml"));
    acc.ran("PKG-NO-CONTENT-TYPES");
    let content_types_present = ct_at.is_some();
    // The archive suffix may hold the manifest; a prefix cannot establish its absence.
    if !content_types_present && !entries_truncated {
        acc.add("PKG-NO-CONTENT-TYPES", None, "the package has no [Content_Types].xml part, so part content types cannot be resolved".to_string());
    }
    let mut content_types = ContentTypes { incomplete: !content_types_present && entries_truncated, ..ContentTypes::default() };
    let mut ct_guard = ParseGuard::new(limits.events, limits.depth);
    if let Some(e) = ct_at.and_then(|i| entries.get(i))
        && let Some(text) = &e.decoded
        && parse_content_types(text, &mut content_types, limits.entries, &mut ct_guard)
    {
        content_types.incomplete = true;
        acc.note(format!("[Content_Types].xml declares more than {} entries; later ones were not recorded", limits.entries));
        acc.set_incomplete();
    }
    if ct_guard.exhausted() {
        content_types.incomplete = true;
        acc.note(format!("[Content_Types].xml exceeded the XML {} budget; later declarations were not recorded", ct_guard.budget()));
        acc.set_incomplete();
    }

    content_types.incomplete |= ct_at.and_then(|i| entries.get(i)).is_some_and(|e| e.content_unavailable);
    if content_types.incomplete {
        // Parser errors suppress manifest-dependent checks just as budget stops do.
        acc.note("[Content_Types].xml declarations were not fully examined; dependent absence checks were skipped".to_string());
        acc.set_incomplete();
    }

    let rels = load_rels(&entries, acc);

    // Classify XML parts once (J4) and decode every classified part, whichever its file name, so a
    // supporting or story part with an unconventional name still has its checks run (H5). The
    // classification is content type (+xml / text/xml / the rels type) OR a recognised relationship
    // role.
    let xml_parts = classified_xml_parts(&entries, &content_types, &rels);
    for e in entries.iter_mut().filter(|e| !e.is_dir) {
        if e.decoded.is_some() || e.decode_error.is_some() || e.bytes.is_none() {
            continue;
        }
        if xml_parts.contains(&e.name.to_ascii_lowercase()) {
            decode_entry(e);
        }
    }

    acc.ran("PKG-NO-MAIN");
    let package_rels = rels.iter().find(|r| r.name.eq_ignore_ascii_case("_rels/.rels"));
    let package_rels_collapsed = collapsed_names.iter().any(|name| name.eq_ignore_ascii_case("_rels/.rels"));
    let main_ambiguous = package_rels_collapsed || competing_targets(package_rels, is_office_document_relationship);
    let main_loss = match (package_rels_collapsed, main_ambiguous) {
        (true, _) => Some("the package main is unproven because _rels/.rels is duplicated; the other record was not examined"),
        (false, true) => Some(
            "ambiguous officeDocument selection for the package: distinct official targets compete; owner-dependent document checks were skipped",
        ),
        (false, false) => None,
    };
    if let Some(note) = main_loss {
        acc.note(note.to_string());
        acc.set_incomplete();
    }
    let main = package_rels
        .and_then(|rp| rp.rels.iter().find(|r| !r.external && is_office_document_relationship(&r.kind)))
        .and_then(|r| r.target_abs.as_deref())
        .and_then(|t| canonical(&entries, t));
    let package_rels_unavailable = entries.iter().any(|e| !e.is_dir && e.name.eq_ignore_ascii_case("_rels/.rels") && e.content_unavailable);
    let discovery_complete = !entries_truncated && !unknown_unreadable && !package_rels_unavailable && !package_rels.is_some_and(|rp| rp.incomplete);
    let main = main_selection(main, package_rels_collapsed, main_ambiguous, discovery_complete);
    if matches!(main, Resolution::Absent) {
        acc.add("PKG-NO-MAIN", None, no_main_message(package_rels));
    }
    let main_name = match &main {
        Resolution::One(name) => Some(name.as_str()),
        Resolution::Absent | Resolution::Ambiguous(_) | Resolution::Unknown(_) => None,
    };
    let glossary = if main_ambiguous {
        Resolution::Unknown("package main is ambiguous")
    } else {
        glossary_part(&entries, &content_types, &rels, main_name, !entries_truncated && !unknown_unreadable)
    };
    let glossary_ambiguous = matches!(glossary, Resolution::Ambiguous(_));
    if let Resolution::Ambiguous(reason) = &glossary {
        acc.note(format!("ambiguous glossaryDocument selection: {reason}; owner-dependent document checks were skipped"));
        acc.set_incomplete();
    } else if let Resolution::Unknown(reason) = &glossary {
        acc.note(format!(
            "unknown glossaryDocument selection: {reason}; no owner was established by selection; independently typed roots remain checkable"
        ));
        acc.set_incomplete();
    }
    let roots_ambiguous = main_ambiguous || glossary_ambiguous;
    let reachable = reachability(&rels, main_name);
    Some(Package {
        entries,
        collapsed_names,
        content_types,
        content_types_present,
        rels,
        main,
        glossary,
        roots_ambiguous,
        reachable,
        xml_parts,
        entries_truncated,
        identity_incomplete: unknown_unreadable,
    })
}

fn main_selection(main: Option<String>, package_rels_collapsed: bool, main_ambiguous: bool, discovery_complete: bool) -> Resolution<String> {
    if package_rels_collapsed {
        Resolution::Unknown("_rels/.rels is duplicated")
    } else if main_ambiguous {
        Resolution::Ambiguous("distinct official targets compete")
    } else if !discovery_complete {
        // A retained target proves presence, not uniqueness: an unseen declaration
        // may select a competing document owner.
        Resolution::Unknown("officeDocument selection was not fully examined")
    } else {
        Resolution::discovered(main, discovery_complete)
    }
}

/// Whether official relationships for one singleton role name distinct internal target parts.
/// Normalized paths identify parts case-insensitively; repeated edges to one part are harmless.
/// Missing or unreadable targets still compete, independently of their own package/XML findings.
fn competing_targets(rp: Option<&RelsPart>, matches: impl Fn(&str) -> bool) -> bool {
    observation_contract! {
        rules: "-";
        collections: "field.rels*1";
        scope: "source-local bounded extraction; no implicit document owner";
        loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
        fallback: "retain proven prefix presence and duplicates; return loss to the caller; no negative membership from an incomplete set";
        package_loss: "independent";
    }

    let Some(rp) = rp else { return false };
    let mut target: Option<&str> = None;
    for r in &rp.rels {
        if r.external || !matches(&r.kind) {
            continue;
        }
        if let Some(t) = r.target_abs.as_deref() {
            if target.is_some_and(|first| !first.eq_ignore_ascii_case(t)) {
                return true;
            }
            target = Some(t);
        }
    }
    false
}

/// Select the main document's official glossary target, else the unique glossary-content-typed
/// part. Multiple fallback candidates are unknown selection, never a ZIP-order tie-break.
fn glossary_part(entries: &[Entry], ct: &ContentTypes, rels: &[RelsPart], main: Option<&str>, entries_complete: bool) -> Resolution<String> {
    observation_contract! {
        rules: "-";
        collections: "field.incomplete*3,field.rels*1";
        scope: "source-local bounded extraction; no implicit document owner";
        loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
        fallback: "retain proven prefix presence and duplicates; return loss to the caller; no negative membership from an incomplete set";
        package_loss: "independent";
    }

    if let Some(main) = main {
        let want = rels_part_name(main);
        let rp = rels.iter().find(|r| r.name.eq_ignore_ascii_case(&want));
        if competing_targets(rp, |kind| relationship_kind(kind) == Some("glossaryDocument")) {
            return Resolution::Ambiguous("distinct official targets compete");
        }
        if rp.is_some_and(|r| r.incomplete) {
            return Resolution::Unknown("glossaryDocument relationships were not fully examined");
        }
        if let Some(rp) = rp
            && let Some(r) = rp.rels.iter().find(|r| !r.external && r.role.kind() == Some("glossaryDocument"))
            && let Some(t) = &r.target_abs
            && let Some(name) = canonical(entries, t)
        {
            return Resolution::One(name);
        }
    }
    let mut candidates = entries.iter().filter(|e| !e.is_dir && ct.of(&e.name) == Some(GLOSSARY_CONTENT_TYPE));
    let Some(first) = candidates.next() else {
        let complete = entries_complete
            && !ct.incomplete
            && !entries.iter().any(|e| e.name.eq_ignore_ascii_case("[Content_Types].xml") && e.content_unavailable)
            && main.is_none_or(|owner| {
                let want = rels_part_name(owner);
                !rels.iter().any(|r| r.name.eq_ignore_ascii_case(&want) && r.incomplete)
                    && !entries.iter().any(|e| e.name.eq_ignore_ascii_case(&want) && e.content_unavailable)
            });
        return Resolution::discovered(None, complete);
    };
    if candidates.next().is_some() {
        return Resolution::Ambiguous("multiple glossary-content-typed fallback candidates compete");
    }
    Resolution::One(first.name.clone())
}

/// A directory entry, or one whose bytes were skipped. A skipped file entry's content is
/// unavailable (a directory has no content to be unavailable).
fn blank_entry(name: String, is_dir: bool) -> Entry {
    Entry { name, bytes: None, is_dir, decoded: None, decode_error: None, content_unavailable: !is_dir }
}

/// A file entry whose bytes are held: text parts are decoded once, strictly. A decode failure means
/// the content is present but unreadable, so its text is unavailable to any consumer.
fn text_entry(name: String, bytes: Vec<u8>) -> Entry {
    observation_contract! {
        rules: "-";
        collections: "type.Vec*1";
        scope: "source-local bounded extraction; no implicit document owner";
        loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
        fallback: "retain proven prefix presence and duplicates; return loss to the caller; no negative membership from an incomplete set";
        package_loss: "independent";
    }

    let (decoded, decode_error) = if is_text_part(&name) {
        match crate::scan::decode_strict(&bytes) {
            Ok(t) => (Some(t), None),
            Err(e) => (None, Some(e)),
        }
    } else {
        (None, None)
    };
    let content_unavailable = decode_error.is_some();
    Entry { name, bytes: Some(bytes), is_dir: false, decoded, decode_error, content_unavailable }
}

/// Decode a part identified as XML by its effective content type, whichever its file name (H5).
fn decode_entry(e: &mut Entry) {
    observation_contract! {
        rules: "-";
        collections: "field.bytes*1";
        scope: "source-local bounded extraction; no implicit document owner";
        loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
        fallback: "retain proven prefix presence and duplicates; return loss to the caller; no negative membership from an incomplete set";
        package_loss: "independent";
    }

    let Some(bytes) = e.bytes.as_deref() else { return };
    let (decoded, decode_error) = match crate::scan::decode_strict(bytes) {
        Ok(t) => (Some(t), None),
        Err(err) => (None, Some(err)),
    };
    e.content_unavailable = decode_error.is_some();
    e.decoded = decoded;
    e.decode_error = decode_error;
}

/// Whether a content type denotes XML text: a `+xml` suffix, `text/xml` or `application/xml`. The
/// relationships type (`application/vnd.openxmlformats-package.relationships+xml`) matches by
/// suffix.
pub(crate) fn is_xml_content_type(ct: &str) -> bool {
    ct.ends_with("+xml") || ct == "text/xml" || ct == "application/xml"
}

/// Whether a part is XML text (a part the XML pass decodes and parses).
pub(crate) fn is_text_part(name: &str) -> bool {
    let l = name.to_ascii_lowercase();
    l.ends_with(".xml") || l.ends_with(".rels")
}

/// Relationship kinds whose target is an XML part, for the single XML-part classification (J4).
fn is_xml_relationship_kind(kind: &str) -> bool {
    relationship_kind(kind).is_some_and(|kind| {
        matches!(
            kind,
            "officeDocument"
                | "styles"
                | "numbering"
                | "settings"
                | "comments"
                | "commentsExtended"
                | "commentsIds"
                | "footnotes"
                | "endnotes"
                | "header"
                | "footer"
                | "glossaryDocument"
                | "theme"
                | "fontTable"
                | "customXml"
                | "customXmlProps"
        )
    })
}

/// The single classification of XML parts (J4), used by loading, decoding and `check_xml`: a part
/// whose effective content type is XML (`+xml` / `text/xml` / `application/xml`), a `.rels`
/// relationship part, or a part reached through a recognised XML relationship role. Returns the
/// lowercased names.
fn classified_xml_parts(entries: &[Entry], ct: &ContentTypes, rels: &[RelsPart]) -> HashSet<String> {
    observation_contract! {
        rules: "-";
        collections: "field.rels*1,type.HashSet*5";
        scope: "source-local bounded extraction; no implicit document owner";
        loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
        fallback: "retain proven prefix presence and duplicates; return loss to the caller; no negative membership from an incomplete set";
        package_loss: "independent";
    }

    let mut role_targets: HashSet<String> = HashSet::new();
    for rp in rels {
        for r in &rp.rels {
            if is_xml_relationship_kind(&r.kind)
                && let Some(t) = &r.target_abs
            {
                role_targets.insert(t.trim_start_matches('/').to_ascii_lowercase());
            }
        }
    }
    // The manifest is always XML-classified, independently of its own declarations (K1): an
    // Override-only manifest has no effective content type and no relationship role, so without
    // this it would never reach the XML scanner and an illegal reference or malformed syntax
    // inside it would go unreported.
    let mut set: HashSet<String> = HashSet::from(["[content_types].xml".to_string()]);
    for e in entries.iter().filter(|e| !e.is_dir) {
        let l = e.name.to_ascii_lowercase();
        if ct.of(&e.name).is_some_and(is_xml_content_type) || l.ends_with(".rels") || role_targets.contains(&l) {
            set.insert(l);
        }
    }
    set
}

/// Whether `name` is an XML part, by the classification stored on the package (J4).
pub(crate) fn is_xml_part(pkg: &Package, name: &str) -> bool {
    observation_contract! {
        rules: "-";
        collections: "field.xml_parts*1";
        scope: "source-local bounded extraction; no implicit document owner";
        loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
        fallback: "retain proven prefix presence and duplicates; return loss to the caller; no negative membership from an incomplete set";
        package_loss: "independent";
    }

    pkg.xml_parts.contains(&name.to_ascii_lowercase())
}

/// Format an explanatory error message when the package lacks a valid main document part.
fn no_main_message(package_rels: Option<&RelsPart>) -> String {
    observation_contract! {
        rules: "-";
        collections: "field.rels*1";
        scope: "source-local bounded extraction; no implicit document owner";
        loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
        fallback: "retain proven prefix presence and duplicates; return loss to the caller; no negative membership from an incomplete set";
        package_loss: "independent";
    }

    let Some(rp) = package_rels else {
        return "the package has no _rels/.rels part, so it has no main document part".to_string();
    };
    match rp.rels.iter().find(|r| !r.external && is_office_document_relationship(&r.kind)) {
        None => "the package's _rels/.rels has no officeDocument relationship".to_string(),
        Some(r) => format!("the officeDocument relationship targets {}, which is not in the package", r.target_abs.as_deref().unwrap_or("")),
    }
}

/// Strip leading ASCII slash bytes (`/`) from a raw entry name.
fn trim_leading_slash(mut b: &[u8]) -> &[u8] {
    while let Some(rest) = b.strip_prefix(b"/") {
        b = rest;
    }
    b
}

/// Inspect the raw zip central directory for duplicate entry names and report `PKG-DUP-ENTRY` findings.
fn check_duplicate_entries(dir: Option<&CentralDirectory>, unique_count: usize, acc: &mut Acc) -> (HashSet<String>, bool) {
    observation_contract! {
        rules: "PKG-DUP-ENTRY";
        collections: "field.decoded_names*1,field.entries*1,type.HashMap*2,type.HashSet*3,type.Vec*1";
        scope: "source-local bounded extraction; no implicit document owner";
        loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
        fallback: "retain proven prefix presence and duplicates; return loss to the caller; no negative membership from an incomplete set";
        package_loss: "independent";
    }

    // The zip reader collapses duplicate names into one entry, so the central directory itself is
    // read to see every stored name (a package may legally list the same name twice). The parse is
    // bounded by the entry budget, exactly as the entry loop is.
    let Some(dir) = dir else {
        // The selection helper has already distinguished unparsed from disputed bytes in
        // its note. Neither path ran duplicate checking, so neither may claim a clean PASS.
        acc.set_incomplete();
        return (HashSet::new(), false);
    };
    acc.ran("PKG-DUP-ENTRY");
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for n in &dir.decoded_names {
        *counts.entry(n.trim_start_matches('/')).or_default() += 1;
    }
    let excess = counts.values().map(|c| c.saturating_sub(1) as u64).sum();
    let mut dups: Vec<&str> = counts.into_iter().filter(|(_, c)| *c > 1).map(|(n, _)| n).collect();
    dups.sort_unstable();
    let mut collapsed_names = HashSet::new();
    for n in dups {
        acc.add("PKG-DUP-ENTRY", None, format!("the entry {n} appears more than once in the archive"));
        acc.note(format!("duplicate records for {n}: the other record's content was not examined; dependent absence checks were skipped"));
        collapsed_names.insert(n.to_string());
    }
    let unexplained = directory_residual(dir.total, unique_count, excess, dir.truncated).is_some_and(|n| n != 0);
    if unexplained {
        acc.note(
            "the zip directory record count disagrees with the reader after decoded duplicates; unaccounted records were not examined".to_string(),
        );
        acc.set_incomplete();
    }
    // The raw directory holds more records than the budget allowed the parse to examine. The
    // unexamined records' names (and so any duplicate among them) are unknown, so duplicate
    // checking did not cover the whole directory: mark it incomplete rather than let a truncated
    // scan return a clean PASS. This is where the loader's deduplicated `zip.len()` check cannot
    // see the truncation: a directory whose duplicates deduplicate to the entry budget passes that
    // check while raw records remain.
    if dir.truncated {
        acc.note(format!(
            "the zip central directory holds {} records, more than the {} examined; duplicate entry checking stopped early and later names were not checked",
            dir.total, acc.limits.entries
        ));
        acc.set_incomplete();
    }
    (collapsed_names, unexplained)
}

/// A complete directory must account for every reader entry and decoded duplicate.
/// A capped prefix cannot establish a count disagreement about its unexamined suffix.
fn directory_residual(total: u64, unique_count: usize, excess: u64, truncated: bool) -> Option<i128> {
    if truncated { None } else { Some(i128::from(total) - unique_count as i128 - i128::from(excess)) }
}

/// IEEE CRC32 for the Info-ZIP Unicode Path field (APPNOTE 4.6.9).
/// Names and extra fields are bounded by their 16-bit central-directory lengths.
fn name_crc32(bytes: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &byte in bytes {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xEDB8_8320u32 & 0u32.wrapping_sub(crc & 1));
        }
    }
    !crc
}

/// Decode a central name as zip 3.0.0 does, before the package's leading-slash trim.
/// Unicode Path fields replace the current raw name in order, even with the UTF-8 flag.
/// Bad CRCs and non-UTF-8 replacements are rejected, never treated as fallback names.
fn decode_zip_name<'a>(mut raw: &'a [u8], utf8: bool, extra: &'a [u8]) -> Option<String> {
    let mut unicode = false;
    let mut offset = 0usize;
    while offset < extra.len() {
        let tag = u16_at(extra, offset)?;
        let len = usize::from(u16_at(extra, offset + 2)?);
        offset = offset.checked_add(4)?;
        let end = offset.checked_add(len)?;
        let field = extra.get(offset..end)?;
        if tag == 0x7075 {
            // The version byte is deliberately unvalidated, matching the archive reader.
            let content = field.get(5..)?;
            if u32_at(field, 1)? != name_crc32(raw) {
                return None;
            }
            std::str::from_utf8(content).ok()?;
            raw = content;
            unicode = true;
        }
        offset = end;
    }
    if utf8 || unicode {
        Some(String::from_utf8_lossy(raw).into_owned())
    } else {
        Some(raw.iter().map(|&b| if b < 128 { char::from(b) } else { CP437_HIGH.get(usize::from(b - 128)).copied().unwrap_or('�') }).collect())
    }
}

/// Reject records the reader cannot include in its central table. `by_index_raw`
/// would parse local headers, initialize data-start caches and seek the archive;
/// do not make directory identity depend on opening a part a second time.
/// The walk already bounds these fields to their 16-bit lengths. Reader-rejected
/// metadata leaves identity unknown rather than allowing a retry to invent duplicates.
fn reader_accepts_central_record(header: &[u8], extra: &[u8], comment: &[u8], directory_start: u64) -> Option<()> {
    let mut local = u64::from(u32_at(header, 42)?);
    let mut compressed = u64::from(u32_at(header, 20)?);
    let mut uncompressed = u64::from(u32_at(header, 24)?);
    let mut method = u16_at(header, 10)?;
    let mut aes = false;
    let mut decoded_comment = None;
    let mut offset = 0usize;
    'extras: while offset < extra.len() {
        let tag = u16_at(extra, offset)?;
        let len = u16_at(extra, offset + 2)?;
        offset = offset.checked_add(4)?;
        let end = offset.checked_add(usize::from(len))?;
        let field = extra.get(offset..end)?;
        match tag {
            0x0001 => {
                // ZIP64 consumes each sentinel field, or all three for len >= 24.
                let mut at = 0usize;
                for value in [&mut uncompressed, &mut compressed, &mut local] {
                    if len >= 24 || *value == u64::from(u32::MAX) {
                        // The reader ignores extra-parser I/O errors, retaining
                        // any ZIP64 values read before the short read.
                        let Some(next) = u64_at(extra, offset.checked_add(at)?) else { break 'extras };
                        *value = next;
                        at = at.checked_add(8)?;
                    }
                }
                field.get(at..)?;
            }
            0x000a => match zip::extra_fields::Ntfs::try_from_reader(&mut Cursor::new(extra.get(offset..)?), len) {
                Ok(_) => {}
                Err(zip::result::ZipError::Io(_)) => break,
                Err(_) => return None,
            },
            0x9901 => {
                if len != 7 || u16_at(field, 2)? != 0x4541 || !matches!(u16_at(field, 0)?, 1 | 2) || !matches!(*field.get(4)?, 1..=3) {
                    return None;
                }
                aes = true;
                method = u16_at(field, 5)?;
            }
            0x5455 => match zip::extra_fields::ExtendedTimestamp::try_from_reader(&mut Cursor::new(extra.get(offset..)?), len) {
                Ok(_) => {}
                Err(zip::result::ZipError::Io(_)) => break,
                Err(_) => return None,
            },
            0x6375 => {
                // Unicode Comment checks the current decoded comment, unlike Path,
                // which checks raw name bytes in decode_zip_name.
                // Its version and CRC are read before the length is validated;
                // a short read ends extra parsing without rejecting the record.
                if extra.get(offset..offset.checked_add(5)?).is_none() {
                    break;
                }
                let current = match decoded_comment.take() {
                    Some(current) => current,
                    None => decode_zip_name(comment, u16_at(header, 8)? & 0x800 != 0, &[])?,
                };
                if u32_at(field, 1)? != name_crc32(current.as_bytes()) {
                    return None;
                }
                decoded_comment = Some(std::str::from_utf8(field.get(5..)?).ok()?.to_string());
            }
            _ => {}
        }
        offset = end;
    }
    // The reader checks the final method after applying AES extra fields and the
    // final ZIP64 local offset before accepting this record into its table.
    if (method == 99 && !aes) || local >= directory_start { None } else { Some(()) }
}

/// CP437's upper half; ASCII/control bytes in the lower half keep their byte values.
const CP437_HIGH: [char; 128] = [
    'Ç', 'ü', 'é', 'â', 'ä', 'à', 'å', 'ç', 'ê', 'ë', 'è', 'ï', 'î', 'ì', 'Ä', 'Å', 'É', 'æ', 'Æ', 'ô', 'ö', 'ò', 'û', 'ù', 'ÿ', 'Ö', 'Ü', '¢', '£',
    '¥', '₧', 'ƒ', 'á', 'í', 'ó', 'ú', 'ñ', 'Ñ', 'ª', 'º', '¿', '⌐', '¬', '½', '¼', '¡', '«', '»', '░', '▒', '▓', '│', '┤', '╡', '╢', '╖', '╕', '╣',
    '║', '╗', '╝', '╜', '╛', '┐', '└', '┴', '┬', '├', '─', '┼', '╞', '╟', '╚', '╔', '╩', '╦', '╠', '═', '╬', '╧', '╨', '╤', '╥', '╙', '╘', '╒', '╓',
    '╫', '╪', '┘', '┌', '█', '▄', '▌', '▐', '▀', 'α', 'ß', 'Γ', 'π', 'Σ', 'σ', 'µ', 'τ', 'Φ', 'Θ', 'Ω', 'δ', '∞', 'φ', 'ε', '∩', '≡', '±', '≥', '≤',
    '⌠', '⌡', '÷', '≈', '°', '∙', '·', '√', 'ⁿ', '²', '■', ' ',
];

/// Read an unsigned 16-bit little-endian integer at byte offset `i`.
fn u16_at(b: &[u8], i: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(i..i + 2)?.try_into().ok()?))
}

/// Read an unsigned 32-bit little-endian integer at byte offset `i`.
fn u32_at(b: &[u8], i: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(i..i + 4)?.try_into().ok()?))
}

/// Read an unsigned 64-bit little-endian integer at byte offset `i`.
fn u64_at(b: &[u8], i: usize) -> Option<u64> {
    Some(u64::from_le_bytes(b.get(i..i + 8)?.try_into().ok()?))
}

/// The parsed zip central directory: the raw records examined within the entry budget, and
/// whether the budget stopped the parse before the last raw record.
struct CentralDirectory<'a> {
    /// Absolute byte offset actually used by the bounded parser (archive offset zero).
    start: u64,
    /// The selected footer's disk and count fields are consistent.
    consistent: bool,
    /// The raw entry names examined, in stored order, including duplicate names.
    names: Vec<&'a [u8]>,
    /// Reader-equivalent decoded names, in the same bounded directory order.
    decoded_names: Vec<String>,
    /// The total raw record count the directory declares.
    total: u64,
    /// True when fewer than `total` records were examined because the entry budget stopped the
    /// parse; duplicates among the unexamined records are then unknown.
    truncated: bool,
}

/// Reject disputed directory identities before deriving duplicates or absence from them.
fn reader_checked_directory<'a>(
    bytes: &'a [u8],
    zip: &zip::ZipArchive<Cursor<&[u8]>>,
    limit: usize,
    acc: &mut Acc,
) -> (Option<CentralDirectory<'a>>, bool) {
    let selection = find_eocd(bytes);
    let cd = selection.as_ref().and_then(|selected| central_directory(bytes, limit, selected));
    let structural_disagreement = selection.as_ref().is_some_and(|selected| {
        selected.disputed
            || (cd.is_none()
                && selected
                    .directory
                    .as_ref()
                    .is_some_and(|directory| usize::try_from(directory.start).ok().and_then(|start| u32_at(bytes, start)) == Some(0x0201_4B50)))
    });
    if structural_disagreement || cd.as_ref().is_some_and(|dir| !dir.consistent || !directory_matches_reader(dir, zip)) {
        // A different directory or an inconsistent count can invent duplicates even when
        // decoded names and the residual agree. No finding may rely on those identities.
        acc.note("the zip central directory's selection or decoded names disagree with the reader; entry identities are unknown and dependent absence checks were skipped".to_string());
        (None, true)
    } else if cd.is_none() {
        // Retain the unsupported-layout path, including trailing data and prepended stubs.
        // Unknown multiplicity can conceal a competing source, so suppress absence too.
        acc.note("the zip central directory could not be parsed; duplicate entry names were not checked".to_string());
        (None, true)
    } else {
        (cd, false)
    }
}

/// Compare the reader's exact decoded-name keys, before the package's slash trim.
/// The reader retains one record per decoded name; collapse that same multiplicity here.
fn directory_matches_reader(dir: &CentralDirectory, zip: &zip::ZipArchive<Cursor<&[u8]>>) -> bool {
    observation_contract! {
        rules: "-";
        collections: "field.decoded_names*1,type.HashSet*1";
        scope: "bounded directory identities compared with the reader's decoded-name table";
        loss: "directory selection or decoded-name disagreement";
        fallback: "unknown entry identity; no duplicate finding from the disputed directory; global absence suppression";
        package_loss: "independent";
    }

    // Both starts are absolute: read.rs adds zip.offset() to the footer's relative
    // offset before exposing central_directory_start(). This parser uses footer offsets
    // as absolute and assumes archive offset zero; do not add the reader offset twice.
    // Check selection even for capped prefixes, whose names cannot prove suffix equality.
    if dir.start != zip.central_directory_start() || zip.offset() != 0 {
        return false;
    }
    // A capped prefix cannot establish equality with the unexamined suffix. Its existing
    // truncation handling retains prefix duplicate checks and reports incomplete coverage.
    if dir.truncated {
        return true;
    }
    let mut names: HashSet<&str> = dir.decoded_names.iter().map(String::as_str).collect();
    if names.len() != zip.len() {
        return false;
    }
    (0..zip.len()).all(|i| zip.name_for_index(i).is_some_and(|name| names.remove(name))) && names.is_empty()
}

/// The raw entry name bytes in the zip central directory, in stored order, including duplicates,
/// stopping after `limit` names (the entry budget), together with whether any raw record remained
/// unexamined. `None` when the directory cannot be parsed (not a zip, or a layout this reader does
/// not understand), in which case duplicate names are simply not checked.
///
/// Raw bytes remain available for the separate ASCII case-collision check. The duplicate key
/// decodes non-flagged names as CP437 (never lossy UTF-8), flagged names as lossy UTF-8,
/// and applies Unicode Path overrides exactly as the reader does. Records past the entry budget
/// remain unexamined and report INCOMPLETE rather than claiming coverage.
fn central_directory<'a>(bytes: &'a [u8], limit: usize, selected: &EocdSelection) -> Option<CentralDirectory<'a>> {
    observation_contract! {
        rules: "-";
        collections: "type.Vec*2";
        scope: "source-local bounded extraction; no implicit document owner";
        loss: "source unavailable or malformed; reader-rejected central record; XML event/depth budget; category allocation cap";
        fallback: "retain proven prefix presence and duplicates; return loss to the caller; no negative membership from an incomplete set";
        package_loss: "independent";
    }

    let directory = selected.directory.as_ref()?;
    let count = directory.count;
    let cd_off = directory.start;
    if count > 1_000_000 {
        return None;
    }
    let limit = u64::try_from(limit).unwrap_or(u64::MAX);
    let truncated = count > limit;
    let examined = count.min(limit);
    let mut names = Vec::with_capacity(examined.min(1 << 16) as usize);
    let mut decoded_names = Vec::with_capacity(examined.min(1 << 16) as usize);
    let mut i = usize::try_from(cd_off).ok()?;
    for _ in 0..examined {
        if u32_at(bytes, i)? != 0x0201_4B50 {
            return None;
        }
        let name_len = usize::from(u16_at(bytes, i + 28)?);
        let extra_len = usize::from(u16_at(bytes, i + 30)?);
        let comment_len = usize::from(u16_at(bytes, i + 32)?);
        let name = bytes.get(i + 46..i + 46 + name_len)?;
        let extra = bytes.get(i + 46 + name_len..i + 46 + name_len + extra_len)?;
        let comment = bytes.get(i + 46 + name_len + extra_len..i + 46 + name_len + extra_len + comment_len)?;
        reader_accepts_central_record(bytes.get(i..i + 46)?, extra, comment, cd_off)?;
        // Validate the name metadata against the reader before trusting this directory record.
        decoded_names.push(decode_zip_name(name, u16_at(bytes, i + 8)? & 0x800 != 0, extra)?);
        names.push(name);
        i = i.checked_add(46 + name_len + extra_len + comment_len)?;
        bytes.get(..i)?;
    }
    // Only the declared count selects records: directory size and bytes before the
    // footer do not bound the reader's walk. A capped prefix keeps its duplicate policy.
    let consistent = directory.footer_consistent && (truncated || u64::try_from(names.len()).ok()? == count);
    Some(CentralDirectory { start: cd_off, consistent, names, decoded_names, total: count, truncated })
}

/// A footer's count-driven directory selection. Size is not a reader selection parameter.
struct DirectoryView {
    start: u64,
    count: u64,
    zip64: bool,
    footer_consistent: bool,
}
impl DirectoryView {
    fn tuple(&self) -> (u64, u64, bool) {
        (self.start, self.count, self.zip64)
    }
}

struct EocdSelection {
    directory: Option<DirectoryView>,
    disputed: bool,
}

/// The locator-targeted ZIP64 record only; never follow the reader's forward scan.
fn zip64_directory(bytes: &[u8], eocd: usize) -> Option<DirectoryView> {
    let locator = eocd.checked_sub(20)?;
    if u32_at(bytes, locator)? != 0x0706_4B50 {
        return None;
    }
    let off = usize::try_from(u64_at(bytes, locator + 8)?).ok()?;
    let expected = u64::try_from(locator.checked_sub(off)?).ok()?;
    if u32_at(bytes, off)? != 0x0606_4B50 {
        return None;
    }
    let size = u64_at(bytes, off + 4)?;
    let count = u64_at(bytes, off + 32)?;
    let cd_off = u64_at(bytes, off + 48)?;
    // The locator is only a hint to the reader. A non-adjacent record can steer this
    // parser to a decoy; reject it rather than following the reader's forward scan.
    if size < 44
        || size.checked_add(12)? != expected
        || u32_at(bytes, locator + 16)? > 1
        || u32_at(bytes, off + 20)? != u32_at(bytes, locator + 4)?
        || u32_at(bytes, off + 16)? != u32_at(bytes, off + 20)?
        || u64_at(bytes, off + 24)? > count
        || u16_at(bytes, off + 14)? > u16_at(bytes, off + 12)?
        || u64::try_from(off).ok()? < count.saturating_mul(46).saturating_add(cd_off)
    {
        return None;
    }
    Some(DirectoryView { start: cd_off, count, zip64: true, footer_consistent: true })
}

/// Select in the reader's backward order, then reject competing selection tuples.
/// Exact-comment-end remains a layout eligibility guard, never a selection rule:
/// removing it would newly parse trailing-data archives frozen as unsupported.
fn find_eocd(bytes: &[u8]) -> Option<EocdSelection> {
    const SIG: [u8; 4] = [0x50, 0x4B, 0x05, 0x06];
    const CDFH: [u8; 4] = [0x50, 0x4B, 0x01, 0x02];
    let start = bytes.len().saturating_sub(22 + 0xFFFF);
    let end = bytes.len().checked_sub(22)?;
    let mut exact_end = false;
    let mut selected: Option<EocdSelection> = None;
    // Existence of a CDFH in [relative offset, EOCD) is the ZIP32 finder predicate.
    // Move this cursor only backward so many footer candidates cannot multiply a
    // whole-file signature scan. We do not use the found position to parse records.
    let mut last_header = bytes.get(..end)?.windows(4).rposition(|w| w == CDFH);
    for i in (start..=end).rev() {
        if bytes.get(i..i + 4) != Some(&SIG) {
            continue;
        }
        let comment_end = i.checked_add(22)?.checked_add(usize::from(u16_at(bytes, i + 20)?))?;
        exact_end |= comment_end == bytes.len();
        if comment_end > bytes.len() {
            continue;
        }
        if last_header.is_some_and(|header| header.checked_add(4).is_none_or(|after| after > i)) {
            last_header = bytes.get(..i)?.windows(4).rposition(|w| w == CDFH);
        }
        let total = u64::from(u16_at(bytes, i + 10)?);
        let offset = u64::from(u32_at(bytes, i + 16)?);
        let zip64 = (total == 0xFFFF || offset == 0xFFFF_FFFF) && i.checked_sub(20).and_then(|locator| u32_at(bytes, locator)) == Some(0x0706_4B50);
        let directory = if zip64 {
            // An unsupported locator target is not permission to choose an earlier
            // footer: the reader may accept it by scanning. Keep identity unknown.
            zip64_directory(bytes, i)
        } else {
            if total != 0
                && (offset >= u64::try_from(i).ok()? || !last_header.is_some_and(|header| u64::try_from(header).is_ok_and(|header| header >= offset)))
            {
                continue;
            }
            // read.rs uses files-on-this-disk for ZIP32, not the total field.
            let count = u64::from(u16_at(bytes, i + 8)?);
            Some(DirectoryView {
                start: offset,
                count,
                zip64: false,
                // Disk mismatch is rejected only after spec.rs selects this footer.
                // Reject its identities rather than silently selecting an earlier one.
                footer_consistent: count == total && u16_at(bytes, i + 4)? == u16_at(bytes, i + 6)?,
            })
        };
        if let Some(first) = selected.as_mut() {
            // No tuple can be established for an unsupported ZIP64 target. It
            // cannot certify agreement with another candidate either.
            if first.directory.as_ref().map(DirectoryView::tuple) != directory.as_ref().map(DirectoryView::tuple) || directory.is_none() {
                first.disputed = true;
            }
        } else {
            selected = Some(EocdSelection { directory, disputed: false });
        }
    }
    if exact_end { selected } else { None }
}

/// Check for package entries that differ only by ASCII case and report `PKG-CASE-COLLISION` findings.
///
/// Groups the raw central-directory names, folding ASCII case on the bytes,
/// so legacy-encoded names are compared as stored. When the central
/// directory cannot be parsed, falls back to the zip reader's entry list
/// (whose names the reader decoded) rather than not checking at all; the
/// run is already INCOMPLETE in that case via the duplicate check.
fn check_case_collisions(dir: Option<&CentralDirectory>, entries: &[Entry], acc: &mut Acc) {
    observation_contract! {
        rules: "PKG-CASE-COLLISION";
        collections: "field.decoded_names*1,field.names*1,type.HashMap*4,type.Vec*4";
        scope: "source-local bounded extraction; no implicit document owner";
        loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
        fallback: "retain proven prefix presence and duplicates; return loss to the caller; no negative membership from an incomplete set";
        package_loss: "independent";
    }

    acc.ran("PKG-CASE-COLLISION");
    let mut collisions: Vec<Vec<String>> = match dir {
        Some(dir) => {
            let mut groups: HashMap<String, Vec<(&str, &[u8])>> = HashMap::new();
            for (&raw, decoded) in dir.names.iter().zip(&dir.decoded_names) {
                let decoded_trimmed = decoded.trim_start_matches('/');
                if decoded_trimmed.ends_with('/') {
                    continue;
                }
                groups.entry(decoded_trimmed.to_ascii_lowercase()).or_default().push((decoded_trimmed, trim_leading_slash(raw)));
            }
            groups
                .into_values()
                .filter_map(|mut v| {
                    v.sort_unstable_by(|a, b| a.0.cmp(b.0).then_with(|| a.1.cmp(b.1)));
                    v.dedup_by(|a, b| a.0 == b.0);
                    (v.len() > 1).then(|| v.into_iter().map(|(_, raw)| String::from_utf8_lossy(raw).into_owned()).collect())
                })
                .collect()
        }
        None => {
            let mut groups: HashMap<String, Vec<&str>> = HashMap::new();
            for e in entries.iter().filter(|e| !e.is_dir) {
                groups.entry(e.name.to_ascii_lowercase()).or_default().push(e.name.as_str());
            }
            groups
                .into_values()
                .filter_map(|mut v| {
                    v.sort_unstable();
                    v.dedup();
                    (v.len() > 1).then(|| v.into_iter().map(str::to_string).collect())
                })
                .collect()
        }
    };
    collisions.sort();
    for v in collisions {
        acc.add("PKG-CASE-COLLISION", None, format!("part names differ only by ASCII case: {}", v.join(", ")));
    }
}

/// Parse all `.rels` relationship parts within the package entries up to configured resource limits.
fn load_rels(entries: &[Entry], acc: &mut Acc) -> Vec<RelsPart> {
    observation_contract! {
        rules: "-";
        collections: "field.entries*1,parser.parse_rels*1,type.Vec*3";
        scope: "source-local bounded extraction; no implicit document owner";
        loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
        fallback: "retain proven prefix presence and duplicates; return loss to the caller; no negative membership from an incomplete set";
        package_loss: "independent";
    }

    let cap = acc.limits.entries;
    let limits = acc.limits;
    let mut rels = Vec::new();
    for e in entries.iter().filter(|e| !e.is_dir) {
        if !e.name.to_ascii_lowercase().ends_with(".rels") {
            continue;
        }
        let Some(dir) = rels_dir(&e.name) else { continue };
        let Some(text) = &e.decoded else {
            // An unreadable declaration source disables relationship-dependent joins too.
            acc.note(format!("{} declarations could not be read; dependent absence checks were skipped", e.name));
            acc.set_incomplete();
            rels.push(RelsPart { name: e.name.clone(), rels: Vec::new(), incomplete: true, inventory_complete: false });
            continue;
        };
        let mut guard = ParseGuard::new(limits.events, limits.depth);
        // A relationships part is read as expat delivers it: an internal entity whose replacement text is a
        // declaration is that declaration. The expansion is a read-only view for this walk; the part's decoded
        // source (what the XML diagnostics and the duplicate-DOCTYPE check read) stays the original.
        let expanded = crate::scan::expand_content_entities(text);
        let (parsed, c, complete, inventory_complete) = parse_rels(&dir, expanded.as_deref().unwrap_or(text), cap, &mut guard);
        if c {
            acc.note(format!("{} declares more than {cap} relationships; later ones were not recorded", e.name));
            acc.set_incomplete();
        }
        if guard.exhausted() {
            acc.note(format!("{} exceeded the XML {} budget; later relationships were not recorded", e.name, guard.budget()));
            acc.set_incomplete();
        }
        if !complete {
            // Suppression without report-level incompleteness would manufacture PASS.
            acc.note(format!("{} declarations were not fully examined; dependent absence checks were skipped", e.name));
            acc.set_incomplete();
        }
        rels.push(RelsPart { name: e.name.clone(), rels: parsed, incomplete: !complete, inventory_complete });
    }
    rels
}

/// The source directory of a `.rels` part, or `None` when it is not a relationship part.
pub(crate) fn rels_dir(name: &str) -> Option<String> {
    let lower = name.to_ascii_lowercase();
    if lower == "_rels/.rels" {
        return Some(String::new());
    }
    if let Some(i) = lower.rfind("/_rels/") {
        return Some(name.get(..i).unwrap_or("").to_string());
    }
    lower.starts_with("_rels/").then(String::new)
}

/// The `.rels` part name that holds `part`'s relationships.
pub(crate) fn rels_part_name(part: &str) -> String {
    let p = part.trim_start_matches('/');
    match p.rsplit_once('/') {
        Some((dir, file)) => format!("{dir}/_rels/{file}.rels"),
        None => format!("_rels/{p}.rels"),
    }
}

/// Find the stored case-preserving entry name matching `name` (case-insensitive, leading slash stripped).
fn canonical(entries: &[Entry], name: &str) -> Option<String> {
    observation_contract! {
        rules: "-";
        collections: "-";
        scope: "internal presence-only entry-name extraction; only package lookup and reader assembly may call canonical";
        loss: "raw entries may be an examined prefix";
        fallback: "None is no observed candidate, never established package absence";
        package_loss: "independent";
    }

    let n = name.trim_start_matches('/');
    entries.iter().find(|e| !e.is_dir && e.name.eq_ignore_ascii_case(n)).map(|e| e.name.clone())
}

/// The stored spelling of the part at `name` (case-insensitive, leading `/` ignored).
/// A missing candidate is Absent only after complete entry-identity discovery; otherwise Unknown.
pub(crate) fn canonical_entry(pkg: &Package, name: &str) -> Resolution<String> {
    observation_contract! {
        rules: "-";
        collections: "field.entries*1";
        scope: "package entry identity and stored spelling; named unreadable entries remain present";
        loss: "archive entry cap or unreadable entry with unrecoverable identity; named byte/decode loss preserves presence";
        fallback: "One for observed identity; Absent only after complete entry discovery; otherwise Unknown";
        package_loss: "independent";
    }

    Resolution::discovered(canonical(&pkg.entries, name), !pkg.entries_truncated && !pkg.identity_incomplete)
}

/// The entry at `name` (case-insensitive, leading `/` ignored), with entry-identity completeness.
/// Named unreadable entries remain One; an unexamined suffix or unnamed entry prevents Absent.
pub(crate) fn entry_by_name<'a>(pkg: &'a Package, name: &str) -> Resolution<&'a Entry> {
    observation_contract! {
        rules: "-";
        collections: "field.entries*1";
        scope: "package entry identity and retained entry reference; named unreadable entries remain present";
        loss: "archive entry cap or unreadable entry with unrecoverable identity; named byte/decode loss preserves presence";
        fallback: "One for observed identity; Absent only after complete entry discovery; otherwise Unknown";
        package_loss: "independent";
    }

    let n = name.trim_start_matches('/');
    Resolution::discovered(
        pkg.entries.iter().find(|e| !e.is_dir && e.name.eq_ignore_ascii_case(n)),
        !pkg.entries_truncated && !pkg.identity_incomplete,
    )
}

/// Whether the part at `name` is present in the package but its content could not be read. A
/// cross-part check that would conclude *absence* from this part's content must skip instead:
/// absence is not established from unavailable content. Part identity is matched exactly as part
/// lookup does (OPC names are case-insensitive), so `_RELS/.RELS` is recognised the same way.
/// One(false) is observed readability, One(true) observed unavailability, Absent a fully searched
/// missing entry, and Unknown neither readability nor unavailability. Callers must distinguish them.
pub(crate) fn content_unavailable(pkg: &Package, name: &str) -> Resolution<bool> {
    observation_contract! {
        rules: "-";
        collections: "lookup.entry_by_name*1";
        scope: "entry content availability; missing entries retain package completeness";
        loss: "archive entry cap; unknown entry identity; unreadable or undecodable bytes";
        fallback: "Unknown supplies neither availability nor unavailability";
        package_loss: "independent";
    }

    match entry_by_name(pkg, name) {
        Resolution::One(entry) => Resolution::One(entry.content_unavailable),
        Resolution::Absent => Resolution::discovered(None, !pkg.entries_truncated && !pkg.identity_incomplete),
        Resolution::Unknown(reason) => Resolution::Unknown(reason),
        Resolution::Ambiguous(reason) => Resolution::Ambiguous(reason),
    }
}

/// Compute the set of lowercased package part paths reachable from the package relationships root and main document.
fn reachability(rels: &[RelsPart], main: Option<&str>) -> HashSet<String> {
    observation_contract! {
        rules: "-";
        collections: "field.rels*2,type.HashMap*1,type.HashSet*4,type.Vec*2";
        scope: "source-local bounded extraction; no implicit document owner";
        loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
        fallback: "retain proven prefix presence and duplicates; return loss to the caller; no negative membership from an incomplete set";
        package_loss: "independent";
    }

    let by_name: HashMap<String, usize> = rels.iter().enumerate().map(|(i, r)| (r.name.to_ascii_lowercase(), i)).collect();
    let mut seen = HashSet::new();
    seen.insert("[content_types].xml".to_string());
    seen.insert("_rels/.rels".to_string());
    let mut queue: Vec<String> = Vec::new();
    let mut visited: HashSet<String> = HashSet::new();
    for rp in rels.iter().filter(|r| r.name.eq_ignore_ascii_case("_rels/.rels")) {
        for r in &rp.rels {
            if let Some(t) = &r.target_abs {
                seen.insert(t.to_ascii_lowercase());
                queue.push(t.clone());
            }
        }
    }
    if let Some(m) = main {
        queue.push(m.to_string());
    }
    while let Some(part) = queue.pop() {
        if !visited.insert(part.to_ascii_lowercase()) {
            continue;
        }
        if let Some(i) = by_name.get(&rels_part_name(&part).to_ascii_lowercase()) {
            for r in &rels[*i].rels {
                if let Some(t) = &r.target_abs {
                    seen.insert(t.to_ascii_lowercase());
                    queue.push(t.clone());
                }
            }
        }
    }
    seen
}

/// Resolve an internal relationship target against a source directory into a package path,
/// following the OPC resolution steps: strip any query/fragment (OPC §§6.3.2, 6.4),
/// percent-decode each segment, and root-clamp `..` segments (a leading `/` is
/// package-absolute). Part-name equivalence itself is ASCII case-insensitive (§6.2.2.3) and is
/// applied by the caller when it looks the result up.
pub(crate) fn resolve(dir: &str, target: &str) -> String {
    observation_contract! {
        rules: "-";
        collections: "type.Vec*2";
        scope: "source-local bounded extraction; no implicit document owner";
        loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
        fallback: "retain proven prefix presence and duplicates; return loss to the caller; no negative membership from an incomplete set";
        package_loss: "independent";
    }

    let target = target.split(['?', '#']).next().unwrap_or(target);
    let mut parts: Vec<String> =
        if target.starts_with('/') { Vec::new() } else { dir.split('/').filter(|s| !s.is_empty()).map(percent_decode).collect() };
    for seg in target.split('/') {
        let seg = percent_decode(seg);
        match seg.as_str() {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            _ => parts.push(seg),
        }
    }
    parts.join("/")
}

/// Percent-decode a URI path segment (`%20` → space); invalid escapes are left as written.
fn percent_decode(s: &str) -> String {
    observation_contract! {
        rules: "-";
        collections: "type.Vec*1";
        scope: "source-local bounded extraction; no implicit document owner";
        loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
        fallback: "retain proven prefix presence and duplicates; return loss to the caller; no negative membership from an incomplete set";
        package_loss: "independent";
    }

    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && let (Some(hi), Some(lo)) = (b.get(i + 1).and_then(|c| hex(*c)), b.get(i + 2).and_then(|c| hex(*c)))
        {
            out.push(hi * 16 + lo);
            i += 3;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Convert an ASCII hexadecimal byte to its numeric nibble value (0..=15).
fn hex(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

/// Only the Transitional and Strict officeDocument URIs establish a package entry point.
fn is_office_document_relationship(kind: &str) -> bool {
    matches!(
        kind,
        "http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument"
            | "http://purl.oclc.org/ooxml/officeDocument/relationships/officeDocument"
    )
}

/// Whether an owner's relationship declarations are unavailable or only partially examined.
pub(crate) fn relationships_incomplete(pkg: &Package, part: &str) -> bool {
    observation_contract! {
        rules: "-";
        collections: "field.incomplete*1,field.rels*1,lookup.content_unavailable*1";
        scope: "source-local bounded extraction; no implicit document owner";
        loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
        fallback: "retain proven prefix presence and duplicates; return loss to the caller; no negative membership from an incomplete set";
        package_loss: "independent";
    }

    let name = rels_part_name(part);
    // An unexamined archive suffix may hold the missing relationships part.
    (match content_unavailable(pkg, &name) {
        Resolution::One(unavailable) => unavailable,
        Resolution::Absent => false,
        Resolution::Unknown(_) => true,
        Resolution::Ambiguous(_) => true,
    }) || pkg.rels.iter().any(|rp| rp.name.eq_ignore_ascii_case(&name) && rp.incomplete)
}

/// The local kind of a relationship in a supported official namespace. A matching suffix alone
/// cannot grant a custom relationship a Word role, select a supporting part, or prescribe its type.
pub(crate) fn relationship_kind(uri: &str) -> Option<&str> {
    let (namespace, kind) = uri.rsplit_once('/')?;
    let official = matches!(
        namespace,
        crate::NS_R
            | "http://schemas.microsoft.com/office/2006/relationships"
            | "http://schemas.microsoft.com/office/2007/relationships"
            | "http://schemas.microsoft.com/office/2011/relationships"
            | "http://schemas.microsoft.com/office/2016/09/relationships"
    );
    (official && !kind.is_empty()).then_some(kind)
}

/// Agreement of the recognized package entry points about the document profile.
pub(crate) enum EntryPointProfile {
    /// Every recognized entry point declares Transitional.
    Transitional,
    /// Every recognized entry point declares Strict, whose markup is unsupported.
    Strict,
}

/// Resolve the profile across all recognized internal entry points, without choosing a target.
/// Repeated targets cannot excuse profile disagreement: first-match scanning could manufacture
/// a Transitional verdict for an uncertain package. The relationship loader bounds this walk.
/// Recognition remains separate from supported kind classification and Strict markup support.
pub(crate) fn entry_point_profile(pkg: &Package) -> Resolution<EntryPointProfile> {
    observation_contract! {
        rules: "-";
        collections: "field.incomplete*1,field.rels*3,lookup.content_unavailable*1";
        scope: "source-local bounded extraction; no implicit document owner";
        loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
        fallback: "retain proven prefix presence and duplicates; return loss to the caller; no negative membership from an incomplete set";
        package_loss: "independent";
    }

    let complete = !pkg.entries_truncated
        && !pkg.identity_incomplete
        && (match content_unavailable(pkg, "_rels/.rels") {
            Resolution::One(unavailable) => !unavailable,
            Resolution::Absent => true,
            Resolution::Unknown(_) => false,
            Resolution::Ambiguous(_) => false,
        })
        && !pkg.rels.iter().any(|rp| rp.name.eq_ignore_ascii_case("_rels/.rels") && rp.incomplete);
    let Some(rp) = pkg.rels.iter().find(|rp| rp.name.eq_ignore_ascii_case("_rels/.rels")) else {
        return Resolution::discovered(None, complete);
    };
    let mut transitional = false;
    let mut strict = false;
    for r in rp.rels.iter().filter(|r| !r.external && is_office_document_relationship(&r.kind)) {
        if r.kind == "http://purl.oclc.org/ooxml/officeDocument/relationships/officeDocument" {
            strict = true;
        } else {
            transitional = true;
        }
    }
    match (transitional, strict) {
        (true, true) => Resolution::Ambiguous("mixed Transitional/Strict entry points"),
        (true, false) => Resolution::One(EntryPointProfile::Transitional),
        (false, true) => Resolution::One(EntryPointProfile::Strict),
        (false, false) => Resolution::discovered(None, complete),
    }
}

/// Parse `[Content_Types].xml`. Returns `true` when the declaration budget stopped it early.
fn parse_content_types(text: &str, ct: &mut ContentTypes, cap: usize, guard: &mut ParseGuard) -> bool {
    observation_contract! {
        rules: "-";
        collections: "field.incomplete*1,parser.parse_content_types*1,parser.parse_opc_children*1";
        scope: "source-local bounded extraction; no implicit document owner";
        loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
        fallback: "retain proven prefix presence and duplicates; return loss to the caller; no negative membership from an incomplete set";
        package_loss: "independent";
    }

    let mut count = 0usize;
    let mut capped = false;
    let (complete, _) = parse_opc_children(text, b"Types", b"http://schemas.openxmlformats.org/package/2006/content-types", guard, |e| {
        if !matches!(e.local_name().as_ref(), b"Default" | b"Override") {
            return true;
        }
        if count >= cap {
            capped = true;
        } else {
            record_content_type(e, ct);
            count += 1;
        }
        !capped
    });
    ct.incomplete |= capped || !complete;
    capped
}

/// Visit only namespace-qualified direct children of the expected OPC root container.
/// All XML events still consume the guard, including ignored foreign or nested elements.
/// Returns traversal completeness and whether the traversal can certify an OPC inventory.
/// Other checks can still diagnose the declarations obtained from a source with inventory loss.
fn parse_opc_children(
    text: &str,
    container: &[u8],
    namespace: &[u8],
    guard: &mut ParseGuard,
    mut visit: impl FnMut(&BytesStart) -> bool,
) -> (bool, bool) {
    observation_contract! {
        rules: "-";
        collections: "access.is_complete*1,parser.parse_doctype_entities*1,parser.parse_opc_children*1,type.Vec*1";
        scope: "source-local bounded extraction; no implicit document owner";
        loss: "malformed XML or discarded declarations; XML event, depth, namespace and declaration budgets";
        fallback: "retain delivered declarations for positive checks; no negative membership from an incomplete inventory";
        package_loss: "independent";
    }

    let mut reader = XmlReader::new(text);
    let mut buf = Vec::new();
    let mut depth = 0usize;
    let mut valid_root = false;
    let mut roots = 0usize;
    let mut inventory_complete = text.chars().all(craft_xml::is_xml_char);
    let mut at_start = true;
    // An element whose namespace is unknown may be a declaration the walk did not visit, so a traversal
    // that met one (by a declaration's local name) is not complete.
    // The same holds for content the walk cannot read as expat delivers it: a DOCTYPE declaring more than plain
    // internal entities (an ATTLIST default changes attributes without any reference), a reference to an entity
    // this walk does not expand, and an attribute value that uses one. Nothing absent from an unexamined part
    // is established.
    let mut unknown_namespace = false;
    // Hashing expanded attribute names can rescan an inherited URI. Bound that work
    // separately from reading namespace declarations, which are expanded once.
    let mut namespace_work = text.len().saturating_mul(16);
    loop {
        if !guard.tick() {
            return (false, false);
        }
        let Ok(event) = reader.read_event_into(&mut buf) else { return (false, false) };
        let opens = matches!(event, Event::Start(_));
        match event {
            Event::Start(e) | Event::Empty(e) => {
                if opens && !guard.enter() {
                    return (false, false);
                }
                let res = reader.resolver();
                let ns = crate::xml::resolve_element(res, e.name()).0;
                let local = e.local_name();
                unknown_namespace |= references_unexpanded_entity(e.attributes_raw());
                let recognized = matches!(&ns, ResolveResult::Bound(uri) if uri.as_ref() == namespace);
                // Only the root and a depth-1 element with a declaration's own local name could be one the walk did
                // not visit; an unresolved element of any other name is not a missing declaration.
                unknown_namespace |= matches!(&ns, ResolveResult::Unknown(_))
                    && (depth == 0 || (depth == 1 && matches!(local.as_ref(), b"Relationship" | b"Override" | b"Default")));
                if inventory_complete {
                    inventory_complete = !matches!(&ns, ResolveResult::Unknown(_))
                        && valid_element_name(e.name().as_ref())
                        && opc_attributes_valid(res, &e, &mut namespace_work);
                }
                if depth == 0 {
                    roots += 1;
                    valid_root = recognized && local.as_ref() == container;
                } else if valid_root && recognized {
                    if depth == 1 {
                        inventory_complete &= visit(&e);
                    } else if container == b"Relationships" && local.as_ref() == b"Relationship" {
                        // A known declaration in the wrong location was discarded.
                        inventory_complete = false;
                    }
                }
                depth += usize::from(opens);
            }
            Event::End(_) => {
                guard.leave();
                depth = depth.saturating_sub(1);
            }
            Event::Text(e) => {
                inventory_complete &= if depth == 0 { crate::scan::is_whitespace(&e) } else { !e.windows(3).any(|bytes| bytes == b"]]>") };
            }
            Event::GeneralRef(e) => {
                let defined = matches!(e.as_ref(), b"lt" | b"gt" | b"amp" | b"apos" | b"quot");
                unknown_namespace |= !defined && !e.as_ref().starts_with(b"#");
                inventory_complete &= depth > 0 && (defined || e.resolve_char_ref().is_ok_and(|c| c.is_some_and(craft_xml::is_xml_char)));
            }
            Event::CData(_) => inventory_complete &= depth > 0,
            Event::Comment(e) => inventory_complete &= !e.ends_with(b"-") && !e.windows(2).any(|bytes| bytes == b"--"),
            Event::PI(e) => {
                let target = e.target();
                inventory_complete &= !target.eq_ignore_ascii_case(b"xml")
                    && target.iter().all(|&b| b != b':')
                    && std::str::from_utf8(target).is_ok_and(craft_xml::is_name);
            }
            Event::Decl(e) => inventory_complete &= at_start && opc_xml_decl_valid(&e),
            // An unevaluated DTD can change declarations or present replacement markup
            // that this walk does not traverse.
            Event::DocType(e) => {
                inventory_complete = false;
                unknown_namespace |= !crate::scan::parse_doctype_entities(e.as_ref(), 4096).0.is_complete();
            }
            Event::Eof => return (depth == 0 && !unknown_namespace, depth == 0 && roots == 1 && valid_root && inventory_complete),
        }
        at_start = false;
        buf.clear();
    }
}

/// Whether raw start-tag attribute bytes contain a reference to anything but a predefined entity or a character.
fn references_unexpanded_entity(raw: &[u8]) -> bool {
    raw.split(|&byte| byte == b'&')
        .skip(1)
        .any(|rest| !(rest.starts_with(b"#") || [&b"lt;"[..], b"gt;", b"amp;", b"apos;", b"quot;"].iter().any(|name| rest.starts_with(name))))
}

/// Validate the lexical QName as well as its in-scope prefix, which the resolver checks separately.
fn valid_qname(raw: &[u8]) -> bool {
    std::str::from_utf8(raw).is_ok_and(|name| {
        let mut parts = name.split(':');
        parts.next().is_some_and(craft_xml::is_name) && parts.next().is_none_or(craft_xml::is_name) && parts.next().is_none()
    })
}

/// An element name is a legal QName whose prefix is not `xmlns`. quick-xml pre-installs the `xmlns`
/// binding, so the resolver reports `xmlns:x` as bound, but Namespaces in XML 1.0 (and expat) reject it.
/// The `xml` prefix is pre-bound to its own URI, which is legal.
fn valid_element_name(raw: &[u8]) -> bool {
    valid_qname(raw) && !raw.starts_with(b"xmlns:")
}

/// Whether a namespace declaration binds a reserved spelling illegally: `xmlns` is never declared,
/// `xml` is bound only to its own URI, and no other prefix or the default namespace is bound to the
/// `xml` or `xmlns` namespace URIs. (Attributes never carry the `xmlns` prefix except as declarations.)
fn illegal_binding(prefix: &PrefixDeclaration<'_>, value: &str) -> bool {
    const XMLNS_NS: &str = "http://www.w3.org/2000/xmlns/";
    match prefix {
        PrefixDeclaration::Default => matches!(value, craft_xml::XML_NS | XMLNS_NS),
        PrefixDeclaration::Named(b"xmlns") => true,
        PrefixDeclaration::Named(b"xml") => value != craft_xml::XML_NS,
        PrefixDeclaration::Named(_) => value.is_empty() || matches!(value, craft_xml::XML_NS | XMLNS_NS),
    }
}

/// quick-xml reports malformed attributes but accepts adjacent quoted attributes.
/// Consume the corresponding XML grammar from the original bytes as well, so
/// even the ignored attributes of foreign elements count towards inventory loss.
fn opc_attribute_tail<'a>(raw: &'a [u8], attr: &quick_xml::events::attributes::Attribute<'_>) -> Option<&'a [u8]> {
    let trim = |bytes: &'a [u8]| bytes.get(bytes.iter().position(|byte| !xml_space(byte)).unwrap_or(bytes.len())..);
    if !matches!(raw, [b' ' | b'\t' | b'\r' | b'\n', ..]) {
        return None;
    }
    let rest = trim(trim(raw)?.strip_prefix(attr.key.as_ref())?)?.strip_prefix(b"=")?;
    let (&quote, rest) = trim(rest)?.split_first()?;
    if !matches!(quote, b'\'' | b'"') {
        return None;
    }
    rest.strip_prefix(attr.value.as_ref())?.strip_prefix(&[quote])
}

fn xml_space(byte: &u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\r' | b'\n')
}

/// Ensure raw, unqualified and expanded names are unique and all values are observable.
fn opc_attributes_valid(res: &NamespaceResolver, element: &BytesStart, work: &mut usize) -> bool {
    observation_contract! {
        rules: "-";
        collections: "type.HashSet*1";
        scope: "attributes on one byte-bounded OPC element";
        loss: "malformed names or values, unresolved prefixes, duplicate expanded names, or namespace work budget";
        fallback: "return inventory loss; retain declarations for independent positive checks";
        package_loss: "independent";
    }

    let mut names = HashSet::new();
    let mut raw = element.attributes_raw();
    for attr in element.attributes().with_checks(false) {
        let Ok(attr) = attr else { return false };
        let Some(tail) = opc_attribute_tail(raw, &attr) else { return false };
        raw = tail;
        let key = attr.key.into_inner();
        if !valid_qname(key) || attr.value.split(|&byte| byte == b'<').nth(1).is_some() {
            return false;
        }
        let Ok(value) = attr.normalized_value(quick_xml::XmlVersion::Implicit1_0) else { return false };
        if !value.chars().all(craft_xml::is_xml_char) {
            return false;
        }
        if let Some(prefix) = attr.key.as_namespace_binding() {
            if illegal_binding(&prefix, value.as_ref()) || !names.insert((None, key)) {
                return false;
            }
            continue;
        }
        let (ns, local) = res.resolve_attribute(attr.key);
        let uri = match ns {
            ResolveResult::Unbound => None,
            ResolveResult::Unknown(_) => return false,
            ResolveResult::Bound(uri) => {
                let Some(left) = work.checked_sub(uri.as_ref().len()) else { return false };
                *work = left;
                Some(uri.into_inner())
            }
        };
        if !names.insert((uri, local.into_inner())) {
            return false;
        }
    }
    raw.iter().all(xml_space)
}

fn opc_xml_decl_valid(decl: &BytesDecl) -> bool {
    let Ok(raw) = std::str::from_utf8(decl.as_ref()) else { return false };
    let declaration = BytesStart::from_content(raw, 3);
    let mut tail = declaration.attributes_raw();
    for attr in declaration.attributes().take(3) {
        let Ok(attr) = attr else { return false };
        let Some(rest) = opc_attribute_tail(tail, &attr) else { return false };
        tail = rest;
    }
    if !tail.iter().all(xml_space) {
        return false;
    }
    let mut attrs = declaration.attributes();
    let Some(Ok(version)) = attrs.next() else { return false };
    if version.key.as_ref() != b"version" || version.value.as_ref() != b"1.0" {
        return false;
    }
    let mut next = attrs.next();
    if let Some(Ok(encoding)) = &next
        && encoding.key.as_ref() == b"encoding"
    {
        let mut bytes = encoding.value.iter();
        if !bytes.next().is_some_and(u8::is_ascii_alphabetic) || !bytes.all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-')) {
            return false;
        }
        next = attrs.next();
    }
    if let Some(Ok(standalone)) = &next
        && standalone.key.as_ref() == b"standalone"
        && matches!(standalone.value.as_ref(), b"yes" | b"no")
    {
        next = attrs.next();
    }
    next.is_none()
}

/// Extract and record an `Override` or `Default` content type declaration from an XML element.
fn record_content_type(e: &BytesStart, ct: &mut ContentTypes) {
    observation_contract! {
        rules: "-";
        collections: "field.defaults*1,field.dup_defaults*1,field.dup_overrides*1,field.overrides*1";
        scope: "source-local bounded extraction; no implicit document owner";
        loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
        fallback: "retain proven prefix presence and duplicates; return loss to the caller; no negative membership from an incomplete set";
        package_loss: "independent";
    }

    let local = e.local_name();
    let l = local.as_ref();
    if l != b"Override" && l != b"Default" {
        return;
    }
    let key = attr_plain(e, b"PartName").or_else(|| attr_plain(e, b"Extension"));
    let Some(key) = key else { return };
    let Some(value) = attr_plain(e, b"ContentType") else { return };
    let key = key.trim_start_matches('/').to_ascii_lowercase();
    let (map, dups) = if l == b"Override" { (&mut ct.overrides, &mut ct.dup_overrides) } else { (&mut ct.defaults, &mut ct.dup_defaults) };
    if map.insert(key.clone(), value).is_some() {
        dups.insert(key);
    }
}

/// Parse a `.rels` part. Returns `(relationships, capped, complete, inventory_complete)` where `capped` is set when the budget
/// stopped further relationships from being recorded. The depth bound is tracked like every other
/// parser (J1), so a deeply nested relationships part stops before a later (possibly dangling)
/// relationship is read.
fn parse_rels(dir: &str, text: &str, cap: usize, guard: &mut ParseGuard) -> (Vec<Rel>, bool, bool, bool) {
    observation_contract! {
        rules: "-";
        collections: "parser.parse_opc_children*1,parser.parse_rels*1,type.Vec*2";
        scope: "source-local bounded extraction; no implicit document owner";
        loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
        fallback: "retain proven prefix presence and duplicates; return loss to the caller; no negative membership from an incomplete set";
        package_loss: "independent";
    }

    let mut rels = Vec::new();
    let mut capped = false;
    let (complete, inventory_complete) =
        parse_opc_children(text, b"Relationships", b"http://schemas.openxmlformats.org/package/2006/relationships", guard, |e| {
            push_rel(dir, e, &mut rels, cap, &mut capped)
        });
    (rels, capped, complete && !capped, inventory_complete)
}

/// Record one `<Relationship>` element, returning false when a declaration is lost or partial.
fn push_rel(dir: &str, e: &BytesStart, rels: &mut Vec<Rel>, cap: usize, capped: &mut bool) -> bool {
    observation_contract! {
        rules: "-";
        collections: "parser.parse_rel*1,type.Vec*1";
        scope: "source-local bounded extraction; no implicit document owner";
        loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
        fallback: "retain proven prefix presence and duplicates; return loss to the caller; no negative membership from an incomplete set";
        package_loss: "independent";
    }

    if e.local_name().as_ref() != b"Relationship" {
        return true;
    }
    let Some((rel, complete)) = parse_rel(dir, e) else { return false };
    if rels.len() >= cap {
        *capped = true;
        return false;
    }
    rels.push(rel);
    complete
}

/// Parse attributes of a `<Relationship>` element into a [`Rel`] struct, resolving internal targets.
fn parse_rel(dir: &str, e: &BytesStart) -> Option<(Rel, bool)> {
    observation_contract! {
        rules: "-";
        collections: "parser.parse_rel*1";
        scope: "source-local bounded extraction; no implicit document owner";
        loss: "source unavailable or malformed; XML event/depth budget; category allocation cap";
        fallback: "retain proven prefix presence and duplicates; return loss to the caller; no negative membership from an incomplete set";
        package_loss: "independent";
    }

    let id = attr_plain(e, b"Id")?;
    let kind = attr_plain(e, b"Type").unwrap_or_default();
    let target = attr_plain(e, b"Target").unwrap_or_default();
    // A value this walk could not read (an entity it does not expand) is unknown, not absent: neither the mode nor the
    // target of such a declaration is established, so the declaration is not recorded (the part is then incomplete).
    let unread = |name: &[u8]| {
        e.attributes().any(|a| a.is_ok_and(|a| a.key.as_ref() == name && a.normalized_value(quick_xml::XmlVersion::Implicit1_0).is_err()))
    };
    if unread(b"TargetMode") || unread(b"Target") {
        return None;
    }
    let complete = !id.is_empty() && !kind.is_empty() && !target.is_empty();
    let external = attr_plain(e, b"TargetMode").is_some_and(|m| m.eq_ignore_ascii_case("External"));
    let target_abs = (!external).then(|| resolve(dir, &target));
    Some((Rel { id: id.into(), role: RelationKind::parse(&kind), kind, target_abs, external }, complete))
}

/// An unqualified OPC attribute; foreign qualified lookalikes never declare OPC metadata.
fn attr_plain(e: &BytesStart, name: &[u8]) -> Option<String> {
    for a in e.attributes() {
        let a = a.ok()?;
        if a.key.as_ref() == name {
            return a.normalized_value(quick_xml::XmlVersion::Implicit1_0).ok().map(|c| c.into_owned());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    include!("zip_name_tests.rs");
}
