//! Headless MCP without automation roots still treats tool calls as automation: profile approval
//! and imported verification are refused.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use serde_json::{Value, json};
use wordcraft_law::{MODULE, install, specs};
use wordcraft_mcp::{Headless, Server};

fn law_headless() -> Headless {
    let mut h = Headless::default();
    h.session.prefs.modules.insert(MODULE.into(), true);
    assert_eq!(install(&mut h.session), specs().len());
    h
}

fn call(server: &mut Server, method: &str, params: Value) -> Value {
    let line = json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}).to_string();
    serde_json::from_str(&server.handle_line(&line).unwrap()).unwrap()
}

fn execute(server: &mut Server, command: &str, params: Value) -> Value {
    let r = call(server, "tools/call", json!({"name": "execute", "arguments": {"command": command, "params": params}}));
    let res = &r["result"];
    assert_eq!(res["isError"], false, "{command}: {res}");
    let text = res["content"][0]["text"].as_str().unwrap_or("null");
    serde_json::from_str(text).unwrap_or(Value::String(text.to_string()))
}

fn execute_err(server: &mut Server, command: &str, params: Value) -> String {
    let r = call(server, "tools/call", json!({"name": "execute", "arguments": {"command": command, "params": params}}));
    let res = &r["result"];
    assert_eq!(res["isError"], true, "{command} should fail: {res}");
    res["content"][0]["text"].as_str().unwrap_or("").to_string()
}

#[test]
fn mcp_without_roots_cannot_verify_profiles() {
    let mut server = Server::new(Box::new(law_headless()));
    call(&mut server, "initialize", json!({"protocolVersion": "2025-06-18"}));
    let id = "mcp-law-verify-test";
    execute(
        &mut server,
        "law.profiles.create",
        json!({
            "schema": 1,
            "id": id,
            "name": "MCP test court",
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
    let e = execute_err(&mut server, "law.profiles.approve", json!({"id": id}));
    assert!(e.contains("outside automation"), "{e}");
    let dir = std::env::temp_dir().join(format!("wc-cli-mcp-law-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("verified.json");
    execute(&mut server, "law.profiles.export", json!({"id": id, "path": path.to_string_lossy()}));
    let imported = execute(&mut server, "law.profiles.import", json!({"path": path.to_string_lossy(), "verified": true}));
    assert_eq!(imported["verified"], json!(false), "MCP import cannot grant verification");
    let _ = std::fs::remove_dir_all(&dir);
}
