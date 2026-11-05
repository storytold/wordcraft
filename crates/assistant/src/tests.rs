//! The agent loop against a headless session, with scripted model replies.

use serde_json::{Value, json};
use wordcraft_mcp::{Backend, Headless};

use crate::{Agent, Config, Error, Event, MAX_STEPS, Observer, Scripted, Turn, text_reply, tool_reply};

fn text(b: &mut Headless) -> String {
    let r = b.call("engine.execute", json!({"command": "document.text"})).unwrap();
    r["text"].as_str().unwrap().to_string()
}

/// Records events and answers confirmations with `yes`.
#[derive(Default)]
struct Log {
    yes: bool,
    asked: Vec<String>,
    events: Vec<String>,
}

impl Observer for Log {
    fn confirm(&mut self, name: &str, _: &Value) -> bool {
        self.asked.push(name.to_string());
        self.yes
    }
    fn event(&mut self, e: Event<'_>) {
        self.events.push(match e {
            Event::Text(t) => format!("text {t}"),
            Event::ToolCall { name, .. } => format!("call {name}"),
            Event::ToolResult { name, ok, .. } => format!("result {name} {ok}"),
            Event::Declined { name } => format!("declined {name}"),
            Event::Stream(crate::Delta::Start) => "stream start".to_string(),
            Event::Stream(crate::Delta::Text(t)) => format!("stream {t}"),
        });
    }
}

#[test]
fn drafts_a_heading_and_a_paragraph_and_reports() {
    let mut b = Headless::default();
    let mut p = Scripted::new([
        tool_reply(&[("a", "get_text", json!({}))]),
        tool_reply(&[
            ("b", "type_text", json!({"text": "Quarterly report"})),
            ("c", "execute", json!({"command": "para.style", "params": {"style": "Heading 1"}})),
        ]),
        tool_reply(&[
            ("d", "execute", json!({"command": "text.newParagraph"})),
            ("e", "type_text", json!({"text": "Sales grew.\nCosts fell.", "paragraphs": true})),
        ]),
        tool_reply(&[("f", "inspect_document", json!({}))]),
        text_reply("Ho scritto il titolo e due paragrafi."),
    ]);
    let mut agent = Agent::new(Config::new("test-model"));
    let mut log = Log::default();
    let turn = agent.send(&mut p, &mut b, "write a short report", &mut log).unwrap();
    assert_eq!(turn.reply, "Ho scritto il titolo e due paragrafi.");
    assert_eq!((turn.steps, turn.tool_calls), (5, 6));
    assert_eq!(text(&mut b), "Quarterly report\nSales grew.\nCosts fell.");
    assert!(log.events.iter().all(|e| !e.ends_with("false")), "{:?}", log.events);
    let inspect = b.call("document.inspect", json!({})).unwrap();
    assert_eq!(inspect["blocks"][0]["style"], "Heading1", "{inspect}");
    // Every request carries the model, the history so far and the command-level tools only.
    let last = p.requests.last().unwrap();
    assert_eq!(last["model"], "test-model");
    let tools: Vec<&str> = last["tools"].as_array().unwrap().iter().map(|t| t["function"]["name"].as_str().unwrap()).collect();
    assert!(tools.contains(&"type_text") && tools.contains(&"execute") && tools.contains(&"select_text"));
    assert!(!tools.contains(&"render_page") && !tools.contains(&"click") && !tools.contains(&"screenshot"));
    // Each tool call is answered by a tool message with its id.
    let msgs = agent.messages();
    assert_eq!(msgs[0]["role"], "system");
    let tool_ids: Vec<&str> = msgs.iter().filter(|m| m["role"] == "tool").map(|m| m["tool_call_id"].as_str().unwrap()).collect();
    assert_eq!(tool_ids, ["a", "b", "c", "d", "e", "f"]);
}

#[test]
fn rewrites_the_selection_with_the_context_it_was_given() {
    let mut b = Headless::default();
    b.call("engine.execute", json!({"command": "text.insert", "params": {"text": "The quick brown fox jumps over the lazy dog."}})).unwrap();
    b.call("engine.execute", json!({"command": "select.text", "params": {"text": "quick brown fox"}})).unwrap();
    let mut p = Scripted::new([tool_reply(&[("a", "type_text", json!({"text": "nimble red fox"}))]), text_reply("Done.")]);
    let mut agent = Agent::new(Config::new("m"));
    let context = "Selected text (type_text replaces it): «quick brown fox»";
    agent.send_with_context(&mut p, &mut b, "make it more vivid", Some(context), &mut ()).unwrap();
    assert_eq!(text(&mut b), "The nimble red fox jumps over the lazy dog.");
    // The context travels with the prompt, in the same user message.
    let user = &p.requests[0]["messages"][1];
    assert_eq!(user["role"], "user");
    let content = user["content"].as_str().unwrap();
    assert!(content.starts_with("make it more vivid") && content.ends_with(context), "{content}");
    // An empty context adds nothing.
    let mut p = Scripted::new([text_reply("Hi.")]);
    Agent::new(Config::new("m")).send_with_context(&mut p, &mut b, "hello", Some("  "), &mut ()).unwrap();
    assert_eq!(p.requests[0]["messages"][1]["content"], "hello");
}

#[test]
fn a_streaming_endpoint_delivers_the_text_in_pieces_before_the_reply() {
    let mut b = Headless::default();
    let mut p = Scripted::streaming([tool_reply(&[("a", "get_text", json!({}))]), text_reply("Hello there, friend.")]);
    let mut log = Log::default();
    let turn = Agent::new(Config::new("m")).send(&mut p, &mut b, "hi", &mut log).unwrap();
    assert_eq!(turn.reply, "Hello there, friend.");
    // A tool-call step has no text to stream; the final step streams in pieces, then the text.
    assert_eq!(
        log.events,
        [
            "call get_text",
            "result get_text true",
            "stream start",
            "stream Hell",
            "stream o th",
            "stream ere,",
            "stream  fri",
            "stream end.",
            "text Hello there, friend."
        ]
    );
    // An endpoint that can't stream answers whole: the default sends no pieces.
    let mut p = Scripted::new([text_reply("Whole.")]);
    let mut log = Log::default();
    Agent::new(Config::new("m")).send(&mut p, &mut b, "hi", &mut log).unwrap();
    assert_eq!(log.events, ["text Whole."]);
}

#[test]
fn the_conversation_carries_over_to_the_next_turn() {
    let mut b = Headless::default();
    let mut p = Scripted::new([text_reply("Hi."), text_reply("Still here.")]);
    let mut agent = Agent::new(Config::new("m"));
    agent.send(&mut p, &mut b, "hello", &mut ()).unwrap();
    agent.send(&mut p, &mut b, "again", &mut ()).unwrap();
    let roles: Vec<&str> = p.requests[1]["messages"].as_array().unwrap().iter().map(|m| m["role"].as_str().unwrap()).collect();
    assert_eq!(roles, ["system", "user", "assistant", "user"]);
    agent.reset();
    assert_eq!(agent.messages().len(), 1);
    // The model and the instructions can change mid-conversation; the history stays.
    agent.send(&mut Scripted::new([text_reply("ok")]), &mut b, "x", &mut ()).unwrap();
    agent.set_instructions(Some("Always reply in Italian.".into()));
    agent.set_model("other");
    assert!(agent.messages()[0]["content"].as_str().unwrap().ends_with("Always reply in Italian."));
    assert_eq!(agent.config().model, "other");
    assert_eq!(agent.messages().len(), 3);
}

/// `execute` and `batch` are no way around the confirmation, and the interface's own commands
/// are not the assistant's.
#[test]
fn file_commands_through_execute_or_batch_ask_and_ui_commands_are_refused() {
    let dir = std::env::temp_dir().join(format!("wordcraft-assistant-exec-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("via-execute.docx");
    let path_s = path.to_string_lossy().to_string();
    let save = json!({"command": "file.saveAs", "params": {"path": path_s}});
    let calls = [
        ("a", "execute", save.clone()),
        ("b", "batch", json!({"commands": [{"command": "format.bold"}, save]})),
        ("c", "execute", json!({"command": " ui.assistant.ask ", "params": {"prompt": "again"}})),
        ("d", "batch", json!({"commands": [{"command": "format.bold"}, {"command": "caret.docEnd"}]})),
    ];
    let mut log = Log::default();
    let mut p = Scripted::new([tool_reply(&calls), text_reply("ok")]);
    Agent::new(Config::new("m")).send(&mut p, &mut Headless::default(), "save it", &mut log).unwrap();
    // Asked for both file commands, declined both: nothing written. Plain edits ask nothing.
    assert_eq!(log.asked, ["execute", "batch"]);
    assert!(!path.exists());
    // The interface command never runs: an error result, not a question.
    assert!(log.events.contains(&"result execute false".to_string()), "{:?}", log.events);
    let results: Vec<String> = p.requests[1]["messages"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|m| m["role"] == "tool")
        .map(|m| m["content"].as_str().unwrap_or("").to_string())
        .collect();
    assert!(results[2].contains("interface command"), "{results:?}");
    // With a yes the file command runs.
    let mut log = Log { yes: true, ..Log::default() };
    let mut p = Scripted::new([tool_reply(&calls[..1]), text_reply("ok")]);
    Agent::new(Config::new("m")).send(&mut p, &mut Headless::default(), "save it", &mut log).unwrap();
    assert!(path.exists(), "{:?}", log.events);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn tools_that_replace_the_document_or_write_files_need_confirmation() {
    let dir = std::env::temp_dir().join(format!("wordcraft-assistant-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("out.docx");
    let path_s = path.to_string_lossy().to_string();
    let script = || {
        Scripted::new([
            tool_reply(&[("a", "type_text", json!({"text": "Hello"}))]),
            tool_reply(&[("b", "save_document", json!({"path": path_s}))]),
            text_reply("done"),
        ])
    };

    // Declined: nothing is written and the model is told not to retry.
    let mut b = Headless::default();
    let mut log = Log::default();
    let mut p = script();
    Agent::new(Config::new("m")).send(&mut p, &mut b, "write and save", &mut log).unwrap();
    assert_eq!(log.asked, ["save_document"]);
    assert!(log.events.contains(&"declined save_document".to_string()));
    assert!(!path.exists());
    let told = p.requests[2]["messages"].as_array().unwrap().iter().any(|m| m["content"].as_str().is_some_and(|c| c.contains("declined")));
    assert!(told);

    // The default observer declines too.
    let mut p = script();
    Agent::new(Config::new("m")).send(&mut p, &mut Headless::default(), "write and save", &mut ()).unwrap();
    assert!(!path.exists());

    // Confirmed: the file is written.
    let mut log = Log { yes: true, ..Log::default() };
    let mut p = script();
    Agent::new(Config::new("m")).send(&mut p, &mut Headless::default(), "write and save", &mut log).unwrap();
    assert!(path.exists(), "{:?}", log.events);

    // A new document asks too.
    let mut log = Log::default();
    let mut b = Headless::default();
    b.call("engine.execute", json!({"command": "text.insert", "params": {"text": "keep me"}})).unwrap();
    let mut p = Scripted::new([tool_reply(&[("a", "new_document", json!({"template": "blank"}))]), text_reply("ok")]);
    Agent::new(Config::new("m")).send(&mut p, &mut b, "start over", &mut log).unwrap();
    assert_eq!(log.asked, ["new_document"]);
    assert_eq!(text(&mut b), "keep me");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn hostile_model_replies_never_panic() {
    let huge = "x".repeat(100_000);
    let many: Vec<(String, &str, Value)> = (0..50).map(|i| (format!("c{i}"), "get_text", json!({}))).collect();
    let many: Vec<(&str, &str, Value)> = many.iter().map(|(id, n, a)| (id.as_str(), *n, a.clone())).collect();
    let replies = vec![
        json!({"choices": [{"message": {"role": "assistant", "tool_calls": [
            {"id": "1", "function": {"name": "execute", "arguments": "{not json"}},
            {"id": "2", "function": {"name": "rm_rf", "arguments": "{}"}},
            {"function": {"name": "type_text", "arguments": {"text": "ok"}}},
            {"id": "4", "function": {"name": "execute", "arguments": "[1,2]"}},
            {"id": "5", "function": {"name": "select_text", "arguments": "{\"text\": 7, \"occurrence\": -3}"}},
            {"id": "6", "function": {"name": "type_text", "arguments": json!({"text": huge}).to_string()}},
            {"id": "7", "function": {"name": "execute", "arguments": "{\"command\": \"insert.table\", \"params\": {\"rows\": 1e12, \"cols\": -1}}"}},
            {"id": "8", "function": {}},
            {"id": "9"},
            7,
            null,
        ]}}]}),
        tool_reply(&many),
        json!({"choices": [{"message": {"content": 42}}]}),
    ];
    let mut b = Headless::default();
    let mut p = Scripted::new(replies);
    let mut agent = Agent::new(Config::new("m"));
    let turn = agent.send(&mut p, &mut b, "go", &mut ()).unwrap();
    assert_eq!(turn.reply, "");
    // The well-formed calls ran despite their neighbours.
    assert!(text(&mut b).starts_with("ok"), "{}", text(&mut b).chars().take(40).collect::<String>());
    // At most 16 calls per reply; every recorded call has its answer.
    let msgs = agent.messages();
    let asked: usize = msgs.iter().filter_map(|m| m["tool_calls"].as_array()).map(Vec::len).sum();
    let answered = msgs.iter().filter(|m| m["role"] == "tool").count();
    assert_eq!(asked, answered);
    assert!(answered <= 32);
    // Long results are cut.
    assert!(msgs.iter().all(|m| m["content"].as_str().is_none_or(|c| c.chars().count() < 25_000)));

    for bad in [json!({}), json!([]), json!({"choices": []}), json!({"choices": [{"message": null}]}), json!("text"), Value::Null] {
        let r = Agent::new(Config::new("m")).send(&mut Scripted::new([bad.clone()]), &mut Headless::default(), "go", &mut ());
        assert!(matches!(r, Err(Error::BadResponse(_))), "{bad}: {r:?}");
    }
    let r = Agent::new(Config::new("m")).send(
        &mut Scripted::new([json!({"error": {"message": "rate limited"}})]),
        &mut Headless::default(),
        "go",
        &mut (),
    );
    assert_eq!(r, Err(Error::Provider("rate limited".into())));
    let r = Agent::new(Config::new("m")).send(&mut Scripted::default(), &mut Headless::default(), "go", &mut ());
    assert!(matches!(r, Err(Error::Provider(_))));
    let r = Agent::new(Config::new("m")).send(&mut Scripted::default(), &mut Headless::default(), "  \n", &mut ());
    assert_eq!(r, Err(Error::EmptyPrompt));
}

#[test]
fn a_model_that_never_stops_calling_tools_is_stopped() {
    let endless = (0..100).map(|i| tool_reply(&[("x", "list_commands", json!({"query": format!("zz{i}")}))]));
    let mut p = Scripted::new(endless);
    let mut cfg = Config::new("m");
    cfg.max_steps = 1000;
    let r = Agent::new(cfg).send(&mut p, &mut Headless::default(), "loop", &mut ());
    assert_eq!(r, Err(Error::StepLimit { steps: MAX_STEPS }));
    // The tool-calling steps, then one wrap-up call that the model wasted on another tool call.
    assert_eq!(p.requests.len(), MAX_STEPS + 1);
}

#[test]
fn out_of_steps_the_model_gets_one_call_without_tools_to_say_what_it_did() {
    let mut p = Scripted::new([
        tool_reply(&[("a", "list_commands", json!({"query": "zz"}))]),
        tool_reply(&[("b", "list_commands", json!({"query": "zz"}))]),
        text_reply("Ho elencato i comandi; non ho scritto nulla."),
        text_reply("Ciao."),
    ]);
    let mut cfg = Config::new("m");
    cfg.max_steps = 2;
    let mut agent = Agent::new(cfg);
    let turn = agent.send(&mut p, &mut Headless::default(), "loop", &mut ()).unwrap();
    assert_eq!(turn, Turn { reply: "Ho elencato i comandi; non ho scritto nulla.".into(), steps: 3, tool_calls: 2, stopped: true });
    // The wrap-up request offers no tools; the nudge stays out of the history, the answer goes in.
    assert!(p.requests[2].get("tools").is_none());
    assert!(agent.messages().iter().all(|m| m["content"].as_str().is_none_or(|c| !c.contains("steps allowed"))));
    assert_eq!(agent.messages().last().unwrap()["content"], "Ho elencato i comandi; non ho scritto nulla.");
    // The conversation carries on.
    let turn = agent.send(&mut p, &mut Headless::default(), "ciao", &mut ()).unwrap();
    assert_eq!((turn.reply.as_str(), turn.stopped), ("Ciao.", false));
}

#[test]
fn old_turns_are_forgotten_in_whole() {
    let mut b = Headless::default();
    let mut agent = Agent::new(Config::new("m"));
    for i in 0..150 {
        let mut p = Scripted::new([tool_reply(&[("t", "list_commands", json!({"query": "zzz"}))]), text_reply(&format!("turn {i}"))]);
        agent.send(&mut p, &mut b, &format!("prompt {i}"), &mut ()).unwrap();
    }
    let msgs = agent.messages();
    assert!(msgs.len() <= 404, "{}", msgs.len());
    assert_eq!(msgs[0]["role"], "system");
    assert_eq!(msgs[1]["role"], "user");
    assert_eq!(msgs.last().unwrap()["content"], "turn 149");
}
