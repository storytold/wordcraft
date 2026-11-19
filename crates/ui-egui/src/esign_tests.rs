//! Tests against the real craft-ui esign contract (storytold/craft-libs#661), wired through
//! [`crate::WordApp`]'s `esign` field and a small in-process mock [`Service`].
//!
//! None of this mocks `EsignDialog` or `EsignBackend` away: each test connects the mock to the
//! real dialog and drives real frames through `egui_kittest::Harness`, the same pattern
//! `documents.rs`'s own tests use for `WordApp`.

use std::sync::{Arc, Mutex};

use craft_esign::EsignCapability;
use craft_ui::esign::{Answer, EsignBackend, Replier, Request};
use egui_kittest::kittest::Queryable;
use wordcraft_engine::Session;

use crate::{Services, WordApp};

type Harness = egui_kittest::Harness<'static, WordApp>;

#[derive(Default)]
struct State {
    linked: bool,
}

#[derive(Clone)]
struct Service(Arc<Mutex<State>>);

impl EsignBackend for Service {
    fn submit(&self, request: Request, reply: Replier) {
        let state = self.0.lock().unwrap();
        match request {
            Request::Capabilities => reply.answer(Answer::Capabilities {
                capabilities: vec![EsignCapability::Envelopes],
                responsible_party: "Stub Signing".to_owned(),
                linked: state.linked,
            }),
            Request::Lookup { .. } => reply.answer(Answer::Lookup(Vec::new())),
            Request::Create { .. } | Request::Status { .. } | Request::Completed { .. } | Request::Void { .. } | Request::Remind { .. } => {
                reply.refuse("not wired in this test");
            }
        }
    }
}

fn app() -> WordApp {
    WordApp::new(Session::new(wordcraft_doc::Document::new()), Services::default())
}

fn harness(a: WordApp) -> Harness {
    egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        a,
    )
}

fn harness_with(service: Service) -> Harness {
    let mut a = app();
    a.esign.set_backend(service);
    harness(a)
}

// ---------------------------------------------------------------- independence from cloud

#[test]
fn esign_hides_with_no_backend_connected() {
    let mut h = harness(app());
    h.run_steps(2);
    assert!(!h.state().esign.available());
}

#[test]
fn esign_becomes_available_from_its_own_backend_alone() {
    let state = Arc::new(Mutex::new(State { linked: true }));
    let mut h = harness_with(Service(Arc::clone(&state)));
    h.run_steps(2);
    // The capability probe answers inline, same thread, same frame: one connected backend is
    // enough. WordCraft has no cloud feature at all, so there is nothing else esign's own
    // availability could be accidentally tied to.
    assert!(h.state().esign.available());
}

// ---------------------------------------------------------------- the title bar's own gate

#[test]
fn the_title_bar_offers_nothing_with_no_backend() {
    let mut h = harness(app());
    h.run_steps(3);
    assert!(h.query_by_label("Request signatures").is_none());
}

#[test]
fn the_title_bar_offers_request_signatures_once_available() {
    let state = Arc::new(Mutex::new(State { linked: true }));
    let mut h = harness_with(Service(Arc::clone(&state)));
    h.run_steps(3);
    h.get_by_label("Request signatures");
}

// ---------------------------------------------------------------- pump wiring

#[test]
fn pump_runs_every_frame_whether_or_not_the_dialog_is_open() {
    let state = Arc::new(Mutex::new(State { linked: true }));
    let mut h = harness_with(Service(Arc::clone(&state)));
    assert!(!h.state().esign.is_open());
    h.run_steps(3);
    // The capability probe ran with the dialog closed the whole time: `show` (and this crate's
    // own `pump` wrapper around it, called from `WordApp::window_work`) must drive it regardless
    // of `is_open`, the same way Zotero's and Read Aloud's own polls run alongside it.
    assert!(h.state().esign.available());
    assert!(!h.state().esign.is_open());
}

#[test]
fn opening_the_dialog_uses_the_real_craft_cloud_identity() {
    let state = Arc::new(Mutex::new(State { linked: true }));
    let mut h = harness_with(Service(Arc::clone(&state)));
    h.run_steps(2);
    let sender = craft_cloud::Identity { id: "a".to_owned(), display: "Ada Lovelace".to_owned(), email: None, avatar: None };
    h.state_mut().esign.open("doc-1".to_owned(), "report.docx".to_owned(), None, sender);
    h.run_steps(2);
    assert!(h.state().esign.is_open());
}

// ---------------------------------------------------------------- the title bar click opens it

#[test]
fn clicking_request_signatures_opens_the_dialog_for_the_active_document() {
    let state = Arc::new(Mutex::new(State { linked: true }));
    let mut a = app();
    a.esign.set_backend(Service(Arc::clone(&state)));
    a.run("text.insert", serde_json::json!({"text": "Hello"})).unwrap();
    let mut h = harness(a);
    h.run_steps(3);
    assert!(!h.state().esign.is_open());
    h.get_by_label("Request signatures").click();
    h.run_steps(2);
    assert!(h.state().esign.is_open());
}
