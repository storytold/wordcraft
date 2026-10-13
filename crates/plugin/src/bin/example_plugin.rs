//! `wordcraft-example-plugin`: the reference plugin, in Rust. It speaks protocol 1 over standard
//! input and output, and doubles as the plugin the host's tests drive.
//!
//! `wordcraft-example-plugin --manifest` prints its `plugin.json`.
//!
//! Commands:
//! - `upperCase`: the selected text in capitals;
//! - `countWords`: words and characters in the selection (or the paragraph), as a message;
//! - `signature {name?}`: a closing line at the end of the document;
//! - `sleep {ms}` and `crash`: for the host's tests (a hang and a process that dies).
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]

use std::io::{BufRead, Write};

use serde_json::{Value as Json, json};

fn manifest() -> Json {
    json!({
        "id": "org.wordcraft.example",
        "name": "WordCraft Example Plugin",
        "version": env!("CARGO_PKG_VERSION"),
        "description": "The reference plugin: capitals, a word count and a signature line, from the ribbon and the right-click menu.",
        "author": "WordCraft contributors",
        "homepage": "https://github.com/storytold/wordcraft/blob/main/docs/plugins.md",
        "apps": ["wordcraft"],
        "run": {"command": std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_else(|_| "wordcraft-example-plugin".into())},
        "commands": [
            {"id": "upperCase", "label": "Upper Case", "description": "Puts the selected text in capitals"},
            {"id": "countWords", "label": "Count Words", "description": "Words and characters in the selection, or in the paragraph"},
            {"id": "signature", "label": "Signature", "description": "Adds a closing line at the end of the document", "params": "{name?: string}"},
            {"id": "sleep", "label": "Sleep", "description": "Waits (for tests)", "params": "{ms: number}"},
            {"id": "crash", "label": "Crash", "description": "Exits at once (for tests)"}
        ],
        "ui": {
            "ribbon": [
                {"tab": "Example", "group": "Text", "buttons": [
                    {"command": "upperCase", "icon": "case"},
                    {"command": "countWords", "icon": "wordCount"},
                    {"command": "signature", "label": "Sign", "size": "small", "params": {"name": "The Example Plugin"}}
                ]},
                {"tab": "Review", "group": "Example", "buttons": [{"command": "countWords", "icon": "wordCount"}]}
            ],
            "menus": [
                {"command": "upperCase", "label": "Upper Case (Example)"},
                {"menu": "table", "command": "countWords", "label": "Count Words in Cell"}
            ]
        }
    })
}

fn text_of(context: &Json) -> String {
    let sel = context.pointer("/selection/text").and_then(Json::as_str).unwrap_or("");
    if sel.is_empty() { context.pointer("/paragraph/text").and_then(Json::as_str).unwrap_or("").to_string() } else { sel.to_string() }
}

/// A command's answer, or an error message.
fn run(command: &str, params: &Json, context: &Json) -> Result<Json, String> {
    match command {
        "upperCase" => {
            let sel = context.pointer("/selection/text").and_then(Json::as_str).unwrap_or("");
            if sel.is_empty() {
                return Err("Select some text first.".into());
            }
            Ok(json!({
                "commands": [{"command": "text.insert", "params": {"text": sel.to_uppercase(), "raw": true}}],
                "message": "Changed to capitals",
            }))
        }
        "countWords" => {
            let text = text_of(context);
            let words = text.split_whitespace().count();
            let chars = text.chars().filter(|c| !c.is_control()).count();
            Ok(json!({"message": format!("{words} words, {chars} characters"), "result": {"words": words, "characters": chars}}))
        }
        "signature" => {
            let name = params.get("name").and_then(Json::as_str).unwrap_or("The Example Plugin");
            Ok(json!({"commands": [
                {"command": "caret.docEnd"},
                {"command": "text.newParagraph"},
                {"command": "text.insert", "params": {"text": format!("— {name}"), "raw": true}}
            ]}))
        }
        "sleep" => {
            let ms = params.get("ms").and_then(Json::as_u64).unwrap_or(100).min(60_000);
            std::thread::sleep(std::time::Duration::from_millis(ms));
            Ok(json!({"message": "awake"}))
        }
        "crash" => std::process::exit(3),
        _ => Err(format!("unknown command {command}")),
    }
}

fn main() {
    if std::env::args().any(|a| a == "--manifest") {
        println!("{:#}", manifest());
        return;
    }
    let mut out = std::io::stdout();
    for line in std::io::stdin().lock().lines() {
        let Ok(line) = line else { break };
        let Ok(msg) = serde_json::from_str::<Json>(&line) else {
            eprintln!("not JSON: {line}");
            continue;
        };
        // Notifications (`shutdown`) carry no id.
        let Some(id) = msg.get("id").cloned() else { break };
        let params = msg.get("params").cloned().unwrap_or(Json::Null);
        let answer = match msg.get("method").and_then(Json::as_str) {
            Some("initialize") => Ok(json!({"protocolVersion": 1})),
            Some("commands/run") => {
                let command = params.get("command").and_then(Json::as_str).unwrap_or("");
                run(command, params.get("params").unwrap_or(&Json::Null), params.get("context").unwrap_or(&Json::Null))
            }
            Some(other) => Err(format!("unknown method {other}")),
            None => Err("no method".into()),
        };
        let reply = match answer {
            Ok(result) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
            Err(message) => json!({"jsonrpc": "2.0", "id": id, "error": {"code": 1, "message": message}}),
        };
        if writeln!(out, "{reply}").and_then(|_| out.flush()).is_err() {
            break;
        }
    }
}
