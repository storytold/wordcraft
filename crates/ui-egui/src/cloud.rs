//! WordCraft's seam for Open, Save and Share against cloud storage.
//!
//! **No real backend ships here.** This used to draw on `craft_ui::cloud`'s rich, provider-neutral
//! pages, but that module only exists behind craft-ui's `cloud` feature, which in turn only exists
//! on an unmerged fork (storytold/craft-libs#160, the CollabCraft fold-in; see also
//! storytold/craft-libs#123 for the collab-craft transfer). Pinning to that fork is exactly the
//! structural problem a storytold maintainer flagged on sibling app PRs, so this crate carries a
//! small, local, inert stand-in instead: every function here reports "unavailable", every page is
//! hidden rather than broken, and the shapes mirror craft-ui's `CloudBackend`/`ShareBackend` so a
//! real connection can be wired back in later with minimal churn once #160 lands.
//!
//! `available()` returning `false` is load-bearing: every caller gates on it (or on a type that is
//! always empty/`None`), so the Open page's Cloud section, the Save As destination picker, the
//! title bar's presence/share controls and the `cloud.*` commands all go quiet rather than panic
//! or show broken controls.

use std::collections::HashMap;

use egui::Ui;
use serde_json::{Value as Json, json};

use crate::WordApp;

/// Reported by every `cloud.*` command while no backend is connected.
pub const UNAVAILABLE: &str = "cloud storage is not available in this build";

/// A vendor-neutral cloud file reference. Minimal placeholder: a real backend would carry an
/// etag, capabilities and more, but nothing here ever constructs one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub id: String,
    pub name: String,
}

/// What kind of thing an [`Entry`] names. Unused while no backend lists anything.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntryKind {
    File,
    Folder,
}

/// A capability a provider might grant. Unused while no backend reports any.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Capability(pub String);

/// A folder listing. Always empty.
#[derive(Default, Clone)]
pub struct Folder {
    pub entries: Vec<Entry>,
}

/// Where an open document came from, when it came from the cloud. Never constructed while no
/// backend is connected.
#[derive(Clone)]
pub struct Origin {
    pub entry: Entry,
}

/// A cloud transfer in flight. Never constructed while no backend is connected.
pub struct Busy;

impl Busy {
    /// What the status bar's spinner would say. Never shown: [`CloudFiles::busy`] is always
    /// `None`.
    pub fn text(&self) -> String {
        String::new()
    }
}

/// The Save a Copy formats the Share menu would offer under a real share button.
#[derive(Clone)]
pub struct CopyFormat {
    pub id: String,
    pub label: String,
}

impl CopyFormat {
    pub fn new(id: &str, label: &str) -> Self {
        Self { id: id.to_owned(), label: label.to_owned() }
    }
}

/// The Open/Save/Share state a real backend would populate. Every field stays at its default:
/// no transfer in flight, no listed files, nothing to report.
#[derive(Default)]
pub struct CloudFiles {
    pub busy: Option<Busy>,
    pub mine: Folder,
    pub shared: Folder,
    pub save_name: String,
    pub sharing_save: bool,
    pub status: Option<String>,
}

impl CloudFiles {
    pub fn new() -> Self {
        Self::default()
    }
}

/// What each save in flight encoded. Always empty while no backend is connected.
pub type Sent = HashMap<u64, (u64, String)>;

/// A request the share dialog made for a save, before offering a link. Never produced by
/// [`ShareState::take_save_request`].
pub struct SaveRequest {
    pub name: String,
}

/// Local stand-in for `craft_ui::share::ShareDialog`. Draws nothing and answers nothing while no
/// backend is connected; kept so `WordApp` and its callers don't change shape again once a real
/// one lands.
#[derive(Default)]
pub struct ShareState;

impl ShareState {
    pub fn new() -> Self {
        Self
    }

    /// Would draw the share dialog and collect its answers. No-op: there is nothing to show.
    pub fn show(&mut self, _ctx: &egui::Context) {}

    /// Would report a save the dialog asked for. Always `None`: the dialog never asks, because
    /// it is never shown.
    pub fn take_save_request(&mut self) -> Option<SaveRequest> {
        None
    }

    /// Would tell the dialog a save it asked for landed. Never called while
    /// `take_save_request` never returns anything.
    pub fn saved(&mut self, _id: String, _name: String) {}

    /// Would tell the dialog a save it asked for failed. Never called while
    /// `take_save_request` never returns anything.
    pub fn save_failed(&mut self, _reason: String) {}
}

/// Whether any cloud backend is connected. Always `false` until storytold/craft-libs#160 lands
/// and this module is wired back up to a real one.
pub fn available(_app: &WordApp) -> bool {
    false
}

/// Whether the active document has a cloud destination already. Always `false`: nothing ever
/// opens from, or is pinned to, the cloud right now.
pub fn has_target(_app: &WordApp) -> bool {
    false
}

/// Whether Share has anything to offer beyond the ordinary export menu. Always `false`.
pub fn shares(_app: &WordApp) -> bool {
    false
}

/// The Share menu's export-format list, for a caller that still wants the shape even though the
/// split button it would feed is never drawn.
pub fn copies() -> Vec<CopyFormat> {
    Vec::new()
}

/// Would draw the split share button and return the export format the user picked, if any.
/// Never called: gated behind [`shares`], which is always `false`.
pub fn share_button(_app: &mut WordApp, _ui: &mut Ui, _formats: &[CopyFormat]) -> Option<String> {
    None
}

/// Would draw who is signed in and who else is here. Draws nothing.
pub fn who(_app: &mut WordApp, _ui: &mut Ui) {}

/// Would draw the Open page's Cloud section. Never called: gated behind [`available`].
pub fn open_page(_app: &mut WordApp, _ui: &mut Ui) {}

/// Whether Save As should default to the cloud destination. Always `false`.
pub fn prefers_cloud(_app: &WordApp) -> bool {
    false
}

/// Starts (or keeps alive) whatever listing a real backend would need before drawing. No-op.
pub fn start(_app: &mut WordApp) {}

/// The account name Save As would show beside "This PC". Empty: never drawn.
pub fn destination_name(_app: &WordApp) -> String {
    String::new()
}

/// Would draw the Save As cloud destination picker and report whether it saved. Never called:
/// gated behind [`available`].
pub fn save_page(_app: &mut WordApp, _ui: &mut Ui) -> bool {
    false
}

/// Would draw the refused-save conflict modal. Draws nothing.
pub fn conflict_modal(_app: &mut WordApp, _ctx: &egui::Context) {}

/// Answers from a cloud worker, and anything still in flight. Nothing to drain.
pub fn poll(_app: &mut WordApp, _ctx: &egui::Context) {}

/// The active document's cloud origin, when it has one. Always `None`.
pub fn origin(_app: &WordApp) -> Option<Origin> {
    None
}

/// Whether WordCraft can open this entry. Always `false`: nothing ever lists one.
pub fn opens(_entry: &Entry) -> bool {
    false
}

/// Would open a listed entry. Never called: nothing is ever listed.
pub fn open_entry(_app: &mut WordApp, _entry: Entry, _into: Option<u64>) {}

/// Reads the current folder again. No-op.
pub fn browse(_app: &mut WordApp, _path: &str) {}

/// Loads what others shared. No-op.
pub fn load_shared(_app: &mut WordApp) {}

/// Links a cloud account. No-op.
pub fn link(_app: &mut WordApp) {}

/// Goes to the Save As page's cloud destination picker. No-op: there is nothing to pick from.
pub fn go_to_save_as(_app: &mut WordApp) {}

/// The whole of the cloud state, for the control channel and for tests.
pub fn status(_app: &WordApp) -> Json {
    json!({"available": false})
}

/// The cloud commands, for the control channel's own listing (as `printing::commands` does).
/// All reported disabled: there is no backend to run them against.
pub fn commands(_app: &WordApp) -> Vec<Json> {
    [
        ("cloud.open", "Open from the Cloud"),
        ("cloud.browse", "Browse a Cloud Folder"),
        ("cloud.shared", "List Shared With Me"),
        ("cloud.openFile", "Open a Cloud File"),
        ("cloud.save", "Save to the Cloud"),
        ("cloud.saveAs", "Save to the Cloud As"),
        ("cloud.reload", "Reload from the Cloud"),
        ("cloud.link", "Link a Cloud Account"),
        ("cloud.status", "Cloud Status"),
    ]
    .into_iter()
    .map(|(id, label)| json!({"id": id, "label": label, "enabled": false}))
    .collect()
}

/// Saves the active document to its cloud destination. Always refused: there is no backend and
/// no document ever has a cloud destination (see [`has_target`]).
pub fn save(_app: &mut WordApp) -> Result<Json, String> {
    Err(UNAVAILABLE.to_owned())
}

/// Saves the active document to a new cloud destination. Always refused.
pub fn save_as(_app: &mut WordApp, _folder: &str, _name: &str) -> Result<Json, String> {
    Err(UNAVAILABLE.to_owned())
}

/// Reads the active document's stored copy again. Always refused.
pub fn reload_active(_app: &mut WordApp) -> Result<Json, String> {
    Err(UNAVAILABLE.to_owned())
}
