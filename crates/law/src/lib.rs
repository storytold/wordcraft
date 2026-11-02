//! LawCraft: court profiles for legal documents, as a WordCraft module.
//!
//! A court profile is a data file — one per court, JSON with a versioned schema — that says how
//! a document of a given type is put together: its **sections** (the skeleton, with the caption
//! and its fields), its **styles** (page settings, named WordCraft styles, line numbers) and its
//! **checks** (word limits and the like). Profiles are loaded, listed, created, updated,
//! validated, approved, imported, exported and applied through commands, so the ribbon, the
//! command palette, the CLI, the JSON control channel and the MCP server all reach them.
//!
//! Three profiles ship verified: a Missouri appellate brief (Rules 84.03, 84.04, 84.06), a
//! federal appellate brief (FRAP 32) and Missouri circuit court pleadings (motion, notice, answer,
//! petition), which `law.pleadings.new` writes a filing skeleton for. Every rule in them records
//! the rule it comes from and when it took effect. Other courts are entered by users or their
//! agents — and anything an agent writes is saved unverified until a person approves it.
//!
//! Nothing here formats a document on its own: applying a profile runs WordCraft's own style and
//! layout commands (`styles.create`, `styles.modify`, `layout.size`, `layout.margins`,
//! `layout.lineNumbers`, `insert.pageNumber`). Legal *rules* live in the profile files; the legal
//! *logic* that isn't formatting (checks, citations, proofing) comes in later pieces.
//!
//! The module is off until the user turns it on (`law.enabled`, File › Options › Add-ins), and a
//! host installs the commands only then — see [`install`].
//!
//! # Example
//!
//! ```
//! use serde_json::json;
//! use wordcraft_engine::Session;
//!
//! let mut session = Session::new(wordcraft_doc::Document::new());
//! session.prefs.modules.insert("law".into(), true);
//! wordcraft_law::install(&mut session);
//! let profiles = session.run("law.profiles.list", &json!({})).expect("list");
//! assert_eq!(profiles.as_array().map(Vec::len), Some(3));
//! ```

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod appendix;
mod apply;
mod certificate;
pub mod checks;
pub mod citation_pack;
pub mod citations;
pub mod cmd;
pub mod pleadings;
pub mod profile;
mod proofing;
pub mod store;
pub mod symbols;
mod toa;

use wordcraft_engine::Session;

pub use cmd::{set_profiles_dir, settings_dir, store};
pub use profile::{Profile, SCHEMA};
pub use store::LawError;

/// The module id the settings use (`law.enabled`).
pub const MODULE: &str = "law";

/// The ribbon tab LawCraft shows while it is on (hidden like any tab in Customize Ribbon).
pub const RIBBON_TAB: &str = "Law";

/// Every `law.*` command spec (profiles, checks, appendix).
pub fn specs() -> Vec<wordcraft_engine::CommandSpec> {
    cmd::specs().into_iter().chain(appendix::specs()).collect()
}

/// The Law tab: New Pleading first, then the legal symbols.
pub fn ribbon_groups() -> Vec<wordcraft_engine::ribbon::CustomGroup> {
    let mut groups = vec![wordcraft_engine::ribbon::CustomGroup { name: "Pleadings".into(), commands: vec!["law.pleadings.new".into()] }];
    groups.extend(symbols::ribbon_groups());
    groups
}

/// Turn the module on for a session: installs the `law.*` commands when `law.enabled` is set in
/// the editing preferences, and leaves the session alone when it isn't. A host calls this once,
/// after it has read the user's preferences, and points the profile store at the user's own
/// profiles folder first ([`set_profiles_dir`]).
pub fn install(session: &mut Session) -> usize {
    session.module_en = wordcraft_proof::ModuleEn::default();
    session.relayout();
    session.ribbon.remove_addin_tab(RIBBON_TAB);
    if !session.module_enabled(MODULE) {
        uninstall(session);
        return 0;
    }
    session.module_en = proofing::module_hooks();
    checks::register_document_hook();
    symbols::register_text_hook();
    session.ribbon.set_addin_tab(RIBBON_TAB, ribbon_groups());
    // A host may open the file *before* it installs LawCraft (`Session::new(doc)`), so the
    // document-loaded hook never runs for it. Restore the applied profile's export checks from
    // the document's own metadata at installation, the same way `set_document` would.
    checks::restore_export_registration(session);
    let n = specs().len();
    session.install_commands(specs());
    n
}

/// Take back everything [`install`] added: the `law.*` commands, LawCraft's export checks and
/// its findings. Run when the module is off, so turning it off in the options leaves nothing behind.
fn uninstall(session: &mut Session) {
    let ids: Vec<&'static str> = specs().iter().map(|c| c.id).collect();
    session.uninstall_commands(&ids);
    session.retain_export_check_commands(|id| !id.starts_with("law."));
    session.clear_lint_findings("law.");
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod tests_checks;

#[cfg(test)]
mod tests_proofing;

#[cfg(test)]
mod tests_citations;

#[cfg(test)]
mod tests_toa;

#[cfg(test)]
mod tests_symbols;

#[cfg(test)]
mod tests_certificate;

#[cfg(test)]
mod tests_pleadings;
