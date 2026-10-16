//! The LawCraft commands: `law.profiles.*`, `law.checks.*`, `law.pleadings.new` and `law.firm.*`.
//!
//! Everything a person or an agent can do with court profiles is a command, so the ribbon, the
//! command palette, shortcuts, the CLI, the JSON control channel and the MCP server all reach
//! them the same way. The commands hold no legal rules themselves: they read, write and apply
//! the profile files ([`crate::profile`], [`crate::store`], [`crate::apply`]).
//!
//! A profile created or updated through these commands is always saved unverified: only
//! `law.profiles.approve`, a person's action, verifies one.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard, OnceLock};

use serde_json::{Value, json};
use wordcraft_engine::{CmdError, CommandSpec, Session, p};

use crate::apply;
use crate::certificate;
use crate::checks;
use crate::citation_pack::CompiledPack;
use crate::citations;
use crate::pleadings;
use crate::profile::{Patch, Profile};
use crate::proofing;
use crate::store::{LawError, Store};
use crate::symbols;

/// Where the profile commands live in the ribbon.
const LOC: &str = "Law › Court Profiles";
const CHECKS_LOC: &str = "Law › Court Checks";
const PROOF_LOC: &str = "Law › Proofing";
const CITATIONS_LOC: &str = "Law › Citations";
const TOA_LOC: &str = "Law › Table of Authorities";

static PACKS: OnceLock<BTreeMap<String, CompiledPack>> = OnceLock::new();

fn packs() -> &'static BTreeMap<String, CompiledPack> {
    PACKS.get_or_init(crate::citation_pack::bundled_map)
}

pub(crate) fn pack_by_id(id: &str) -> Result<&'static CompiledPack, CmdError> {
    packs().get(id).ok_or_else(|| CmdError::Failed(format!("no citation pack `{id}`")))
}
const CERT_LOC: &str = "Law › Certificates";
const PLEADINGS_LOC: &str = "Law";

/// The profiles WordCraft knows: the ones it ships and the user's own.
static STORE: OnceLock<Mutex<Store>> = OnceLock::new();

/// The shared store.
pub fn store() -> MutexGuard<'static, Store> {
    STORE.get_or_init(|| Mutex::new(Store::default())).lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Point the profile store at the user's profiles folder. The host calls this once at start-up;
/// without one, the bundled profiles and anything saved in this run are all there is.
pub fn set_profiles_dir(dir: Option<PathBuf>) {
    store().set_dir(dir);
}

/// The user's profiles folder inside WordCraft's settings folder (`<settings>/law/profiles`).
pub fn settings_dir() -> Option<PathBuf> {
    wordcraft_control_key::settings_dir().map(|d| d.join("law").join("profiles"))
}

/// The commands, for [`wordcraft_engine::Session::install_commands`].
/// Profile and check commands (appendix commands are in [`crate::appendix`]).
pub fn specs() -> Vec<CommandSpec> {
    let mut v = symbols::specs();
    v.extend([
        CommandSpec::new("law.profiles.list", "Court Profiles", LOC, |_, _| {
            let store = store();
            Ok(Value::Array(store.list().iter().map(|p| p.summary()).collect()))
        })
        .params(r#"{} → [{id, name, jurisdiction, verified, citationPack, docs: [document type]}]"#)
        .pure(),
        CommandSpec::new("law.profiles.get", "Get Court Profile", LOC, |_, v| {
            let id = p::req_str(v, "id")?;
            store().get(id).map(|p| p.to_value()).ok_or_else(|| not_found(id))
        })
        .params(r#"{"id": string} → the profile (see law.profiles.create for the shape)"#)
        .pure(),
        CommandSpec::new("law.profiles.create", "Create Court Profile", LOC, |_, v| {
            let mut p = from_value(v)?;
            if store().get(&p.id).is_some() {
                return Err(CmdError::Failed(format!("a court profile `{}` already exists (use law.profiles.update)", p.id)));
            }
            // Whoever wrote it, it is unverified until a person approves it.
            store().save(&mut p).map_err(invalid)?;
            Ok(p.to_value())
        })
        .params(
            r#"the profile: {"schema": 1, "id": string, "name": string, "jurisdiction": string, "verified"?: bool, "citationPack"?: string, "docs": {<document type>: {"label"?, "page": Page, "lineNumbers"?: LineNumbers, "pageNumbers"?: PageNumbers, "styles": [Style], "sections": [Section], "checks": [Check], "source": string, "effective"?: "YYYY-MM-DD"}}}"#,
        )
        .pure(),
        CommandSpec::new("law.profiles.update", "Update Court Profile", LOC, |_, v| {
            let patch = Patch::from_value(v.clone()).map_err(|bad| invalid(LawError::invalid(&bad)))?;
            let id = patch.id.as_deref().ok_or_else(|| CmdError::Params("`id` (string) is required".into()))?.to_string();
            let Some(base) = store().get(&id).cloned() else { return Err(not_found(&id)) };
            // A changed profile is unverified again, whoever changed it. Nothing is written
            // when the result isn't a valid profile.
            let mut p = patch.apply(&base);
            store().save(&mut p).map_err(invalid)?;
            Ok(p.to_value())
        })
        .params(r#"{"id": string, "name"?, "jurisdiction"?, "citationPack"?, "docs"?} — the fields given replace the stored profile's; the rest stay"#)
        .pure(),
        CommandSpec::new("law.profiles.validate", "Validate Court Profile", LOC, |_, v| {
            let p = from_value(v)?;
            Ok(json!({"ok": true, "id": p.id, "name": p.name, "docs": p.doc_types()}))
        })
        .params(r#"a profile as law.profiles.create takes it (or {"path": string} for a file) → {"ok": true, …}; every problem found is reported at once, and nothing is saved"#)
        .pure(),
        CommandSpec::new("law.profiles.approve", "Approve Court Profile", LOC, |_, v| {
            let id = p::req_str(v, "id")?;
            // A person's action: the only thing that verifies a profile.
            Ok(store().approve(id).map_err(invalid)?.to_value())
        })
        .params(r#"{"id": string} → the profile, verified"#)
        .pure(),
        CommandSpec::new("law.profiles.import", "Import Court Profile", LOC, |_, v| {
            let path = p::req_str(v, "path")?;
            let mut p = Store::read_file(std::path::Path::new(path)).map_err(invalid)?;
            // Automation never grants verification: only a person importing outside MCP or a
            // confined agent session may keep or set the verified flag from the file. A confined
            // agent may reach a file that says `"verified": true` (or pass the override) without
            // the automation marker, so confinement counts too.
            if wordcraft_engine::file_access::automation_invoked() || wordcraft_engine::file_access::automation_confined() {
                p.verified = false;
            } else if let Some(verified) = p::bool(v, "verified") {
                p.verified = verified;
            }
            store().insert(p.clone()).map_err(invalid)?;
            Ok(p.to_value())
        })
        .params(r#"{"path": string, "verified"?: bool (outside automation only: override the file's flag)} → the profile as it is now in the store; importing under MCP or a confined agent session always saves it unverified"#)
        .pure(),
        CommandSpec::new("law.profiles.export", "Export Court Profile", LOC, |_, v| {
            let id = p::req_str(v, "id")?;
            let path = p::req_str(v, "path")?;
            let Some(p) = store().get(id).cloned() else { return Err(not_found(id)) };
            Store::write_file(std::path::Path::new(path), &p).map_err(invalid)?;
            Ok(json!({"path": path, "id": id}))
        })
        .params(r#"{"id": string, "path": string} → {"path", "id"}"#)
        .pure(),
        CommandSpec::new("law.profiles.apply", "Apply Court Profile", LOC, |s, v| {
            let id = p::req_str(v, "id")?;
            let Some(p) = store().get(id).cloned() else { return Err(not_found(id)) };
            let doc = match p::str(v, "doc") {
                Some(d) => d.to_string(),
                // One document type goes without saying; several, and the caller names one.
                None => {
                    let mut types = p.doc_types();
                    if types.len() != 1 {
                        return Err(CmdError::Params(format!("`doc` is required: `{}` has {}", p.name, types.join(", "))));
                    }
                    types.remove(0).to_string()
                }
            };
            // Usable even unverified: the flag says whether a person checked the rules.
            apply::apply(s, &p, &doc).map_err(|e| CmdError::Failed(e.to_string()))
        })
        .params(r#"{"id": string, "doc"?: string (a document type of the profile)} → {"profile", "doc", "applied": [command], "styles": [style id]}"#),
        CommandSpec::new("law.checks.run", "Run Court Checks", CHECKS_LOC, |s, v| {
            // An export check names no profile and follows the document: when no profile is
            // applied any more (it was undone), there is nothing to check.
            if p::str(v, "id").is_none() && s.doc.custom_prop(checks::PROP_PROFILE).is_none() && p::bool(v, "forExport") == Some(true) {
                s.set_lint_findings(checks::FINDINGS_SOURCE, Vec::new());
                return Ok(json!({"findings": []}));
            }
            let profile_id = match p::str(v, "id") {
                Some(id) => id.to_string(),
                None => s
                    .doc
                    .custom_prop(checks::PROP_PROFILE)
                    .ok_or_else(|| CmdError::Params("`id` (profile id) is required when no court profile is applied".into()))?
                    .to_string(),
            };
            let Some(p) = store().get(&profile_id).cloned() else { return Err(not_found(&profile_id)) };
            let doc_type = match p::str(v, "doc") {
                Some(d) => d.to_string(),
                None => s
                    .doc
                    .custom_prop(checks::PROP_DOC_TYPE)
                    .or_else(|| {
                        let mut types = p.doc_types();
                        (types.len() == 1).then(|| types.remove(0))
                    })
                    .ok_or_else(|| CmdError::Params(format!("`doc` is required: `{}` has {}", p.name, p.doc_types().join(", "))))?
                    .to_string(),
            };
            let for_export = p::bool(v, "forExport").unwrap_or(false);
            // The output format an export is writing. On demand the caller names one (`"pdf"`
            // when omitted), and a PDF-only check such as `textSearchablePdf` runs only for a
            // PDF: saving a `.docx` does not make the copy a PDF.
            let format = p::str(v, "format").unwrap_or("pdf").to_ascii_lowercase();
            let pdf_output = format == "pdf";
            let export_opts = if for_export { Some(wordcraft_engine::export_options(s, v)) } else { None };
            let findings = checks::evaluate(&s.doc, &p, &doc_type, for_export, pdf_output, export_opts.as_ref())
                .map_err(|e| CmdError::Failed(e.to_string()))?;
            let json_findings = checks::findings_json(&findings);
            if p::bool(v, "store").unwrap_or(true) {
                s.set_lint_findings(checks::FINDINGS_SOURCE, findings);
            }
            Ok(json!({"profile": profile_id, "doc": doc_type, "findings": json_findings}))
        })
        .params(
            r#"{"id"?: string, "doc"?: string, "forExport"?: bool (run export-only checks such as textSearchablePdf), "format"?: string (the output extension an export is writing; `textSearchablePdf` runs only for "pdf"), "store"?: bool (default true: show findings in Review › Proofing)} → {"profile", "doc", "findings": [{checkId, severity, message, start?, end?}]}"#,
        )
        .pure(),
        // The court-check findings last stored for this document. LawCraft's own command, so
        // core's Review menu and its interface catalogs carry no court-specific entry.
        CommandSpec::new("law.pleadings.new", "New Pleading", PLEADINGS_LOC, pleadings::new_pleading).params(
            r#"{} opens the New Pleading dialog; or {"type": "motion|notice|answer|petition", "title": string, "courtType"?: string (default "CIRCUIT"), "county": string, "caseNumber": string (may be empty for a petition), "firstSide": {"names": [string], "role": string}, "secondSide": {"names": [string], "role": string}, "weRepresent": "first|second", "service": "efileCounsel|efileUsps" (not for a petition), "servedParty"?: string, "servedAddress"?: string (both required for efileUsps), "filingDate"?: "YYYY-MM-DD" (default today, UTC), "verified"?: bool (petition only), "firm"?: Firm (default: law.firm.get)} → {"profile", "type", "sections": [section id]}; writes into a blank document only"#,
        ),
        CommandSpec::new("law.firm.get", "Firm Details", PLEADINGS_LOC, pleadings::firm_get)
            .params(r#"{} → the saved firm details {"firmName", "address1", "address2"?, "city", "state", "zip", "phone", "fax"?, "email", "attorneyName", "barNumber"}, or {}"#)
            .pure(),
        CommandSpec::new("law.firm.set", "Save Firm Details", PLEADINGS_LOC, pleadings::firm_set)
            .params(r#"{"firmName"?, "address1"?, "address2"?, "city"?, "state"?, "zip"?, "phone"?, "fax"?, "email"?, "attorneyName"?, "barNumber"?} (strings of at most 200 characters; they replace the saved details) → the saved details"#)
            .pure(),
        CommandSpec::new("law.checks.findings", "Court Check Findings", CHECKS_LOC, |s, _| Ok(serde_json::Value::Array(s.lint_findings_of(checks::FINDINGS_SOURCE).to_vec())))
            .params(r#"{} → [{checkId, severity, message, start?, end?}] (the findings the last law.checks.run stored)"#)
            .pure(),
        CommandSpec::new("law.words.list", "Legal Word List", PROOF_LOC, |_, _| {
            Ok(Value::Array(proofing::legal_words().iter().map(|w| json!(w)).collect()))
        })
        .params(r#"{} → [string] (legal terms added to English spelling when LawCraft is on)"#)
        .pure(),
        CommandSpec::new("law.proof.confusions", "Legal Confusion Check", PROOF_LOC, |s, v| {
            let store = p::bool(v, "store").unwrap_or(true);
            Ok(proofing::run_confusion_checks(s, store))
        })
        .params(r#"{"store"?: bool (default true: show findings in Review › Proofing)} → {"findings": [{checkId, severity, message, start?, end?}]}"#)
        .pure(),
        CommandSpec::new("law.citations.check", "Check Citation Form", CITATIONS_LOC, |s, v| {
            let pack_id = match p::str(v, "pack") {
                Some(id) => id.to_string(),
                None => {
                    let profile_id = s.doc.custom_prop(checks::PROP_PROFILE).ok_or_else(|| {
                        CmdError::Params("`pack` (citation pack id) is required when no court profile is applied".into())
                    })?;
                    let store = store();
                    let Some(profile) = store.get(profile_id) else {
                        return Err(not_found(profile_id));
                    };
                    profile.citation_pack.clone().ok_or_else(|| {
                        CmdError::Params(format!("court profile `{profile_id}` has no `citationPack`"))
                    })?
                }
            };
            let pack = pack_by_id(&pack_id)?;
            let findings = citations::check_document(&s.doc, pack);
            let json_findings = citations::findings_json(&findings);
            if p::bool(v, "store").unwrap_or(true) {
                s.set_lint_findings("law.citations", findings);
            }
            Ok(json!({"pack": pack_id, "findings": json_findings}))
        })
        .params(
            r#"{"pack"?: string (defaults to the applied court profile's citationPack), "store"?: bool (default true: show findings in Review › Proofing)} → {"pack", "findings": [{checkId, severity, message, start?, end?}]}"#,
        )
        .pure(),
        CommandSpec::new("law.toa.build", "Build Table of Authorities", TOA_LOC, crate::toa::build).params(
            r#"{"pack"?: string (defaults to the applied court profile's citationPack), "id"?: profile id, "doc"?: document type, "at"?: "cursor" (default: after the profile's Table of Authorities heading when the document has one, else the caret), "passim"?: bool (default: the profile's toaPassim, else true)} → {"pack", "authorities", "marked", "inserted": [category], "updated"}: marks the citations the finder accepts with TA fields, then inserts a TOA field per category or updates the table already there"#,
        ),
        CommandSpec::new("law.certificate.compliance", "Certificate of Compliance", CERT_LOC, |s, v| {
            let (profile, doc_type) = profile_and_doc(s, v)?;
            certificate::insert_or_refresh(s, &profile, &doc_type).map_err(|e| CmdError::Failed(e.to_string()))
        })
        .params(
            r#"{"id"?: string, "doc"?: string} → {"profile", "doc", "wordCount", "wordLimit", "section", "findings"?: [{checkId, severity, message}]} (includes a word-limit finding when over the limit)"#,
        ),
        CommandSpec::new("law.certificate.refresh", "Refresh Certificate Word Count", CERT_LOC, |s, _| {
            certificate::refresh_wordcount_fields(s)?;
            Ok(json!({"refreshed": true}))
        })
        .params(r#"{} → {"refreshed": true} (updates LAWWORDCOUNT fields; runs before export when a profile is applied)"#),
    ]);
    v
}

fn profile_and_doc(s: &Session, v: &Value) -> Result<(Profile, String), CmdError> {
    let profile_id = match p::str(v, "id") {
        Some(id) => id.to_string(),
        None => s
            .doc
            .custom_prop(checks::PROP_PROFILE)
            .ok_or_else(|| CmdError::Params("`id` (profile id) is required when no court profile is applied".into()))?
            .to_string(),
    };
    let profile = store().get(&profile_id).cloned().ok_or_else(|| not_found(&profile_id))?;
    let doc_type = match p::str(v, "doc") {
        Some(d) => d.to_string(),
        None => s
            .doc
            .custom_prop(checks::PROP_DOC_TYPE)
            .or_else(|| {
                let mut types = profile.doc_types();
                (types.len() == 1).then(|| types.remove(0))
            })
            .ok_or_else(|| CmdError::Params(format!("`doc` is required: `{}` has {}", profile.name, profile.doc_types().join(", "))))?
            .to_string(),
    };
    Ok((profile, doc_type))
}

fn not_found(id: &str) -> CmdError {
    CmdError::Failed(format!("no court profile `{id}`"))
}

/// A command parameter that isn't a profile: every problem, reported together.
fn invalid(e: LawError) -> CmdError {
    match e {
        LawError::Invalid(problems) => CmdError::Params(format!("the profile is not valid:\n  {problems}")),
        LawError::HumanOnly(msg) => CmdError::Disabled(msg),
        other => CmdError::Failed(other.to_string()),
    }
}

/// Read a profile out of command parameters (or a file, with `{"path": …}`).
fn from_value(v: &Value) -> Result<Profile, CmdError> {
    if let Some(path) = p::str(v, "path") {
        return Store::read_file(std::path::Path::new(path)).map_err(invalid);
    }
    let mut v = v.clone();
    // A `path` is how a file is asked for; anything else must be the profile itself.
    if let Some(obj) = v.as_object_mut() {
        obj.remove("path");
    }
    Profile::from_value(v).map_err(|bad| invalid(LawError::invalid(&bad)))
}
