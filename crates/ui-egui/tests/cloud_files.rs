//! Open, Save and Share against cloud storage currently have no backend: the previous one
//! (`craft_ui::cloud`'s rich pages, behind craft-ui's `cloud` feature) only exists on an unmerged
//! fork (storytold/craft-libs#160). Pinning to that fork is exactly the structural problem a
//! storytold maintainer flagged on sibling app PRs, so `wordcraft_ui_egui::cloud` is a small,
//! local, inert stand-in instead, and these tests pin that it stays inert and does not panic or
//! show broken controls: every entry point reports unavailable, and the pages it would have drawn
//! stay out of the way.

use serde_json::json;
use wordcraft_doc::Document;
use wordcraft_engine::Session;
use wordcraft_ui_egui::cloud;
use wordcraft_ui_egui::{Services, WordApp};

fn app() -> WordApp {
    WordApp::new(Session::new(Document::new()), Services::default())
}

#[test]
fn no_backend_is_available() {
    let app = app();
    assert!(!cloud::available(&app));
    assert!(!cloud::shares(&app));
    assert!(!cloud::has_target(&app));
    assert!(!cloud::prefers_cloud(&app));
}

#[test]
fn status_reports_unavailable_rather_than_panicking() {
    let app = app();
    assert_eq!(cloud::status(&app), json!({"available": false}));
}

#[test]
fn every_cloud_command_is_listed_disabled() {
    let app = app();
    let commands = cloud::commands(&app);
    assert!(!commands.is_empty());
    for command in &commands {
        assert_eq!(command["enabled"], false, "{command}");
    }
}

#[test]
fn save_and_save_as_are_refused_rather_than_silently_succeeding() {
    let mut app = app();
    assert_eq!(cloud::save(&mut app), Err(cloud::UNAVAILABLE.to_owned()));
    assert_eq!(cloud::save_as(&mut app, "/", "Document1.docx"), Err(cloud::UNAVAILABLE.to_owned()));
    assert_eq!(cloud::reload_active(&mut app), Err(cloud::UNAVAILABLE.to_owned()));
}

#[test]
fn the_active_document_has_no_cloud_origin() {
    let app = app();
    assert!(cloud::origin(&app).is_none());
}

#[test]
fn the_share_dialog_never_asks_for_a_save() {
    let mut app = app();
    assert!(app.share.take_save_request().is_none());
}

#[test]
fn go_to_cloud_commands_through_ui_command_report_unavailable() {
    let mut app = app();
    for id in ["cloud.open", "cloud.browse", "cloud.shared", "cloud.link", "cloud.saveAs"] {
        let err = app.execute(id, json!({})).unwrap_err();
        assert_eq!(err, cloud::UNAVAILABLE, "{id}");
    }
}
