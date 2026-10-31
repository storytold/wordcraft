//! The control channel (including MCP `--connect`) runs commands as automation: profile
//! verification cannot be granted or approved through it.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use serde_json::json;
use wordcraft_doc::Document;
use wordcraft_engine::Session;
use wordcraft_law::{MODULE, install};
use wordcraft_ui_egui::control::{ControlRequest, Outcome, handle};
use wordcraft_ui_egui::{Services, WordApp};

fn law_app() -> WordApp {
    let mut s = Session::new(Document::new());
    s.prefs.modules.insert(MODULE.into(), true);
    install(&mut s);
    WordApp::new(s, Services::default())
}

fn execute(app: &mut WordApp, ctx: &egui::Context, command: &str, params: serde_json::Value) -> serde_json::Value {
    let (req, _rx) = ControlRequest::new("engine.execute", json!({"command": command, "params": params}));
    let Outcome::Done(v) = handle(app, ctx, &req) else { panic!("screenshot") };
    v
}

fn execute_ok(app: &mut WordApp, ctx: &egui::Context, command: &str, params: serde_json::Value) -> serde_json::Value {
    let v = execute(app, ctx, command, params);
    assert_eq!(v["ok"], true, "{command}: {v}");
    v["result"].clone()
}

fn execute_err(app: &mut WordApp, ctx: &egui::Context, command: &str, params: serde_json::Value) -> String {
    let v = execute(app, ctx, command, params);
    assert_eq!(v["ok"], false, "{command} should fail: {v}");
    v["error"].as_str().unwrap_or("").to_string()
}

#[test]
fn control_channel_cannot_verify_court_profiles() {
    let ctx = egui::Context::default();
    let mut app = law_app();
    let id = "control-law-verify-test";
    execute_ok(
        &mut app,
        &ctx,
        "law.profiles.create",
        json!({
            "schema": 1,
            "id": id,
            "name": "Control test court",
            "jurisdiction": "Test",
            "docs": {
                "brief": {
                    "styles": [{"name": "Normal"}],
                    "sections": [{"id": "cover", "name": "Cover"}],
                    "source": "Test"
                }
            }
        }),
    );
    let e = execute_err(&mut app, &ctx, "law.profiles.approve", json!({"id": id}));
    assert!(e.contains("outside automation"), "{e}");
    let dir = std::env::temp_dir().join(format!("wc-ui-law-control-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("verified.json");
    execute_ok(&mut app, &ctx, "law.profiles.export", json!({"id": id, "path": path.to_string_lossy()}));
    let imported = execute_ok(&mut app, &ctx, "law.profiles.import", json!({"path": path.to_string_lossy(), "verified": true}));
    assert_eq!(imported["verified"], json!(false), "control import cannot grant verification");
    let _ = std::fs::remove_dir_all(&dir);
}
