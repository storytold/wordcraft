//! LawCraft's interface: the add-in the desktop app hands WordCraft's interface
//! ([`wordcraft_ui_egui::addins::AddIn`]). File › Options › Add-ins turns it on or off; while it is
//! on it installs the `law.*` commands and the Law ribbon tab, and brings its own File › Options
//! page for court profiles and firm details, and the New Pleading window. WordCraft's interface
//! code never names it.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod page;
pub mod pleading;

use std::ops::{Deref, DerefMut};

use egui::Ui;
use serde_json::Value;
use wordcraft_engine::Session;
use wordcraft_ui_egui::WordApp;
use wordcraft_ui_egui::addins::AddIn;

pub use page::LawUi;

/// The LawCraft add-in: its options page's state.
#[derive(Default)]
pub struct LawAddIn {
    pub state: LawUi,
}

/// The app with the page's state beside it: page code reads `app.law` and uses the rest of the
/// app as usual.
pub(crate) struct Ctx<'a> {
    app: &'a mut WordApp,
    pub law: &'a mut LawUi,
}

impl Deref for Ctx<'_> {
    type Target = WordApp;
    fn deref(&self) -> &WordApp {
        self.app
    }
}

impl DerefMut for Ctx<'_> {
    fn deref_mut(&mut self) -> &mut WordApp {
        self.app
    }
}

impl AddIn for LawAddIn {
    fn id(&self) -> &'static str {
        wordcraft_law::MODULE
    }

    fn name(&self) -> &str {
        "LawCraft"
    }

    fn description(&self) -> &str {
        "Court profiles, legal formatting and filing checks for briefs."
    }

    /// The host points the profile store at the user's folder ([`wordcraft_law::set_profiles_dir`]).
    fn install(&mut self, session: &mut Session) {
        wordcraft_law::install(session);
    }

    fn has_options_page(&self) -> bool {
        true
    }

    fn options_page(&mut self, app: &mut WordApp, ui: &mut Ui) {
        page::page(&mut Ctx { app, law: &mut self.state }, ui);
    }

    fn leave_page(&mut self, app: &mut WordApp) -> bool {
        let mut cx = Ctx { app, law: &mut self.state };
        let saved = page::commit_pending_drafts(&mut cx) && pleading::save_firm(&mut cx);
        if saved {
            // Nothing is pending any more: an error from an earlier attempt is stale.
            self.state.validation = None;
        }
        saved
    }

    fn file_picked(&mut self, app: &mut WordApp, tag: &str, picked: Option<String>) -> Result<Value, String> {
        page::finish_pick(&mut Ctx { app, law: &mut self.state }, tag, picked)
    }

    fn open(&mut self, what: &str) {
        pleading::open(&mut self.state, what);
    }

    fn input_dialog(&self, id: &str, params: &Value) -> Option<&'static str> {
        (id == "law.pleadings.new" && params.as_object().is_none_or(|p| p.is_empty())).then_some("newPleading")
    }

    fn dialog(&mut self, app: &mut WordApp, ctx: &egui::Context) {
        pleading::show(&mut Ctx { app, law: &mut self.state }, ctx);
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}
