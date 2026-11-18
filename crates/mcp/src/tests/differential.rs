//! The shared server against the hand-written one it replaced (`oracle`): the same scripted
//! JSON-RPC session goes through both and the replies must be equal, after normalising only the
//! differences listed in `normalise`. Differences that cannot be normalised away are pinned by
//! their own tests below. Document ids (a process-wide counter) are zeroed before comparing.

use serde_json::{Value, json};

use super::oracle;
use crate::{Headless, Server};

/// One input line of the session; `free_text` marks a tool call whose failure text is the
/// validator's own words (both sides must still say `isError: true`).
struct Step {
    line: Vec<u8>,
    free_text: bool,
}

fn req(id: Value, method: &str, params: Value) -> Vec<u8> {
    json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}).to_string().into_bytes()
}

fn step(line: Vec<u8>) -> Step {
    Step { line, free_text: false }
}

fn call(id: u64, name: &str, args: Value) -> Step {
    step(req(json!(id), "tools/call", json!({"name": name, "arguments": args})))
}

fn call_free(id: u64, name: &str, args: Value) -> Step {
    Step { free_text: true, ..call(id, name, args) }
}

fn script() -> Vec<Step> {
    let mut s = Vec::new();
    // initialize with each supported version, an unknown one, an odd one, and none.
    for (i, v) in [json!("2025-06-18"), json!("2025-03-26"), json!("2024-11-05"), json!("1999-01-01"), json!(7), Value::Null].into_iter().enumerate()
    {
        s.push(step(req(json!(i + 1), "initialize", json!({"protocolVersion": v, "capabilities": {}}))));
    }
    s.push(step(req(json!(20), "initialize", json!({}))));
    s.push(step(req(json!(21), "initialize", Value::Null)));
    s.push(step(br#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#.to_vec()));
    s.push(step(req(json!(30), "ping", json!({}))));
    s.push(step(req(json!(31), "tools/list", json!({}))));
    s.push(step(req(json!(32), "resources/templates/list", json!({}))));
    // tools: a realistic session, an image, an error result, a doc-addressed call.
    for (i, (name, args)) in [
        ("new_document", json!({})),
        ("new_document", json!({"template": "sample"})),
        ("type_text", json!({"text": "Hello there"})),
        ("type_text", json!({"text": "a\nb", "paragraphs": true, "doc": null})),
        ("select_text", json!({"text": "Hello", "occurrence": 1})),
        ("execute", json!({"command": "format.bold"})),
        ("execute", json!({"command": "no.such.command"})),
        ("execute", json!({"command": "text.insert", "params": {"text": "x"}, "doc": 99})),
        ("batch", json!({"commands": [{"command": "caret.docEnd"}, {"command": "text.insert", "params": {"text": "!"}}]})),
        ("batch", json!({"commands": [{"command": "no.such"}, {"command": "caret.docEnd"}], "keepGoing": true})),
        ("insert_table", json!({"values": [["a", 1], ["b", 2]], "header": true})),
        ("list_commands", json!({"query": "bold"})),
        ("list_documents", json!({})),
        ("get_text", json!({})),
        ("inspect_document", json!({})),
        ("render_page", json!({"page": 1, "scale": 0.2})),
        ("render_page", json!({"page": 99})),
        ("screenshot", json!({})),
        ("click", json!({"x": 1, "y": 2})),
        ("key", json!({"key": "B", "cmd": true})),
        ("parity", json!({})),
        ("save_document", json!({"path": "/nonexistent-dir-wc/x.docx"})),
    ]
    .into_iter()
    .enumerate()
    {
        s.push(call(100 + i as u64, name, args));
    }
    // bad arguments: both sides fail with isError; the words are the validator's own.
    s.push(call_free(200, "execute", json!({})));
    s.push(call_free(201, "execute", json!({"command": 5})));
    s.push(call_free(203, "insert_table", json!({"values": "x"})));
    s.push(call_free(204, "insert_table", json!({"values": null})));
    s.push(call_free(205, "get_text", json!({"doc": "1"})));
    s.push(call_free(206, "render_page", json!({"doc": 1.5})));
    s.push(call_free(207, "open_document", json!({})));
    s.push(call_free(208, "execute", json!("not an object")));
    // A missing or null `arguments` is `{}` to the shared server (it was null).
    for (id, params) in [(209, json!({"name": "execute"})), (210, json!({"name": "execute", "arguments": null}))] {
        s.push(Step { free_text: true, ..step(req(json!(id), "tools/call", params)) });
    }
    s.push(step(req(json!(211), "tools/call", json!({"name": "nope", "arguments": {}}))));
    s.push(step(req(json!(212), "tools/call", json!({}))));
    s.push(step(req(json!(213), "tools/call", json!({"name": 5}))));
    s.push(step(req(json!(214), "tools/call", Value::Null)));
    // odd ids.
    for id in [json!("abc"), json!(0), json!(-3), json!(1.5), json!(u64::MAX), json!(null), json!(true), json!([1]), json!({"a": 1})] {
        s.push(step(req(id, "ping", json!({}))));
    }
    s.push(step(br#"{"jsonrpc":"2.0","method":"ping"}"#.to_vec()));
    // resources.
    s.push(step(req(json!(300), "resources/list", json!({}))));
    s.push(step(req(json!(301), "resources/read", json!({"uri": "wordcraft://document"}))));
    s.push(step(req(json!(302), "resources/read", json!({"uri": "wordcraft://commands"}))));
    s.push(step(req(json!(303), "resources/read", json!({"uri": "wordcraft://nothing"}))));
    // unknown methods, malformed messages, notifications.
    s.push(step(req(json!(310), "nope", json!({}))));
    s.push(step(req(json!(311), "tools/nope", Value::Null)));
    for raw in [
        &br#"{"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":1}}"#[..],
        br#"{"jsonrpc":"2.0","method":"notifications/progress"}"#,
        br#"{"jsonrpc":"2.0","method":"notifications/whatever"}"#,
        br#"{"jsonrpc":"2.0","id":1,"result":{}}"#,
        br#"{"jsonrpc":"2.0","id":1,"error":{"code":1,"message":"x"}}"#,
        br#"{"jsonrpc":"2.0","id":7}"#,
        br#"{"jsonrpc":"2.0","id":7,"method":5}"#,
        br#"{"id":8,"method":"ping"}"#,
        br#"[]"#,
        br#"42"#,
        br#""text""#,
        br#"null"#,
        br#"{not json"#,
        br#"   "#,
        b"",
    ] {
        s.push(step(raw.to_vec()));
    }
    // batches: mixed requests and notifications, only notifications, bad members.
    s.push(step(
        json!([
            {"jsonrpc": "2.0", "id": 400, "method": "ping"},
            {"jsonrpc": "2.0", "method": "notifications/initialized"},
            {"jsonrpc": "2.0", "id": 401, "method": "tools/list"},
            5,
            {"jsonrpc": "2.0", "id": 402, "method": "nope"}
        ])
        .to_string()
        .into_bytes(),
    ));
    s.push(step(br#"[{"jsonrpc":"2.0","method":"notifications/initialized"}]"#.to_vec()));
    // CRLF line ending, leading and trailing whitespace.
    s.push(step(b"  {\"jsonrpc\":\"2.0\",\"id\":500,\"method\":\"ping\"}\r".to_vec()));
    s
}

/// The documented differences, removed from a reply before comparing. Anything else must match.
fn normalise(mut v: Value, free_text: bool) -> Value {
    if let Value::Array(items) = v {
        return Value::Array(items.into_iter().map(|i| normalise(i, free_text)).collect());
    }
    // 1. `capabilities` gains `listChanged: false` on tools and resources.
    if let Some(c) = v.pointer_mut("/result/capabilities").and_then(Value::as_object_mut) {
        for cap in c.values_mut() {
            if let Some(o) = cap.as_object_mut() {
                o.remove("listChanged");
            }
        }
    }
    // 2. Error texts are reworded (too long/large, non-UTF-8, unknown method, unknown resource);
    //    the codes are compared.
    if let Some(e) = v.get_mut("error").and_then(Value::as_object_mut) {
        let code = e.get("code").and_then(Value::as_i64).unwrap_or(0);
        if [-32601, -32002].contains(&code) || e.get("message").and_then(Value::as_str).is_some_and(|m| m.contains("too ") || m.contains("UTF-8")) {
            e.insert("message".into(), json!("<reworded>"));
        }
    }
    // 3. Document ids come from a process-wide counter, so two sessions in one process differ.
    if let Some(text) = v.pointer_mut("/result/content/0/text")
        && let Some(Ok(mut j)) = text.as_str().map(serde_json::from_str::<Value>)
    {
        zero_ids(&mut j);
        *text = json!(j.to_string());
    }
    // 4. A tool call refused before the tool runs: the validator's own words.
    if free_text && v.pointer("/result/isError") == Some(&json!(true)) {
        v["result"]["content"] = json!("<validator text>");
    }
    v
}

fn zero_ids(v: &mut Value) {
    match v {
        Value::Object(o) => {
            for (k, x) in o.iter_mut() {
                if k == "id" && x.is_number() {
                    *x = json!(0);
                } else {
                    zero_ids(x);
                }
            }
        }
        Value::Array(a) => a.iter_mut().for_each(zero_ids),
        _ => {}
    }
}

fn run_oracle(input: &[u8]) -> Vec<Value> {
    let mut server = oracle::Server::new(Box::new(Headless::default()));
    let mut out = Vec::new();
    server.serve(std::io::Cursor::new(input.to_vec()), &mut out).unwrap();
    lines(out)
}

fn run_shared(input: &[u8]) -> Vec<Value> {
    let mut server = Server::new(Box::new(Headless::default()));
    let mut out = Vec::new();
    server.serve(std::io::Cursor::new(input.to_vec()), &mut out).unwrap();
    lines(out)
}

fn lines(out: Vec<u8>) -> Vec<Value> {
    String::from_utf8(out).unwrap().lines().map(|l| serde_json::from_str(l).unwrap_or_else(|e| panic!("{e}: {l}"))).collect()
}

/// Both servers answer a line at a time, so each step is compared on its own reply (or lack of one).
fn compare(steps: &[Step]) {
    let mut old = oracle::Server::new(Box::new(Headless::default()));
    let mut new = Server::new(Box::new(Headless::default()));
    for (i, s) in steps.iter().enumerate() {
        let text = String::from_utf8_lossy(&s.line).into_owned();
        let (a, b) = (old.handle_line(&text), new.handle_line(&text));
        let parse = |r: Option<String>| r.map(|r| normalise(serde_json::from_str::<Value>(&r).unwrap(), s.free_text));
        assert_eq!(parse(b), parse(a), "step {i}: {text}");
    }
    assert_eq!(old.is_initialized(), new.is_initialized());
}

#[test]
fn shared_server_answers_the_scripted_session_like_the_old_one() {
    compare(&script());
}

/// The same session as a byte stream through `serve`, with an oversize line and a line that is
/// not UTF-8 in the middle: same replies, same order.
#[test]
fn serve_matches_the_old_server_with_an_oversize_and_a_non_utf8_line() {
    let mut input = Vec::new();
    for (i, s) in script().iter().enumerate() {
        if i == 20 {
            input.extend(vec![b'a'; (1 << 20) + 1]);
            input.push(b'\n');
            input.extend_from_slice(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"ping\",\"x\":\"\xff\xfe\"}\n");
            // Exactly at the limit is accepted (and is a parse error); one over is refused.
            input.extend(vec![b' '; 1 << 20]);
            input.push(b'\n');
            input.extend(vec![b'{'; 1 << 20]);
            input.push(b'\n');
        }
        input.extend_from_slice(&s.line);
        input.push(b'\n');
    }
    // An unterminated last line still counts.
    input.extend_from_slice(br#"{"jsonrpc":"2.0","id":9,"method":"ping"}"#);
    let (old, new) = (run_oracle(&input), run_shared(&input));
    let norm = |v: Vec<Value>| v.into_iter().map(|r| normalise(r, false)).collect::<Vec<_>>();
    // The script's free-text failures are only compared in `compare`; drop their texts here.
    let strip = |v: Vec<Value>| -> Vec<Value> {
        v.into_iter()
            .map(|mut r| {
                if r.pointer("/result/isError") == Some(&json!(true))
                    && r["result"]["content"][0]["text"].as_str().is_some_and(|t| !t.starts_with('{') && !t.starts_with('['))
                {
                    r["result"]["content"] = json!("<text>");
                }
                r
            })
            .collect()
    };
    let (old, new) = (strip(norm(old)), strip(norm(new)));
    assert_eq!(new.len(), old.len());
    for (i, (n, o)) in new.iter().zip(&old).enumerate() {
        assert_eq!(n, o, "reply {i}");
    }
}

/// Difference 1 (accepted): the oversize and non-UTF-8 replies keep their codes, the words change.
#[test]
fn refused_lines_keep_their_codes_and_say_why() {
    let mut input = vec![b'a'; (1 << 20) + 1];
    input.extend_from_slice(b"\n\xff\n");
    let old = run_oracle(&input);
    let new = run_shared(&input);
    assert_eq!(old[0]["error"]["code"], -32600);
    assert_eq!(new[0]["error"]["code"], -32600);
    assert_eq!(old[0]["error"]["message"], "request line too long");
    assert_eq!(new[0]["error"]["message"], "request too large");
    assert_eq!(old[1]["error"]["code"], -32700);
    assert_eq!(new[1]["error"]["code"], -32700);
    assert!(new[1]["error"]["message"].as_str().unwrap().contains("UTF-8"));
    assert_eq!(new[0]["id"], Value::Null);
}

/// Difference 2 (accepted): a request with no `uri` is "unknown resource" -32002, not -32602
/// (the shared core hands the tools an empty uri); an unknown uri keeps -32002.
#[test]
fn resources_read_without_a_uri_is_not_found() {
    let mut old = oracle::Server::new(Box::new(Headless::default()));
    let mut new = Server::new(Box::new(Headless::default()));
    let line = req(json!(1), "resources/read", json!({}));
    let line = String::from_utf8(line).unwrap();
    let o: Value = serde_json::from_str(&old.handle_line(&line).unwrap()).unwrap();
    let n: Value = serde_json::from_str(&new.handle_line(&line).unwrap()).unwrap();
    assert_eq!(o["error"]["code"], -32602);
    assert_eq!(n["error"]["code"], -32002);
}

/// Difference 3 (accepted): the shared core serves `prompts/list` (empty) where the old server
/// said "method not found"; WordCraft has no prompts, and `prompts/get` stays not found.
#[test]
fn prompts_list_is_empty_and_prompts_get_is_not_found() {
    let mut new = Server::new(Box::new(Headless::default()));
    let r = new.handle(json!({"jsonrpc": "2.0", "id": 1, "method": "prompts/list"})).unwrap();
    assert_eq!(r["result"], json!({"prompts": []}));
    let r = new.handle(json!({"jsonrpc": "2.0", "id": 2, "method": "prompts/get", "params": {"name": "x"}})).unwrap();
    assert_eq!(r["error"]["code"], -32601);
    // And capabilities do not advertise prompts or completions.
    let r = new.handle(json!({"jsonrpc": "2.0", "id": 3, "method": "initialize", "params": {}})).unwrap();
    assert_eq!(r["result"]["capabilities"], json!({"tools": {"listChanged": false}, "resources": {"listChanged": false}}));
}

/// Difference 4 (accepted hardening): a batch of more than 1000 messages is refused whole; the old
/// server answered every member.
#[test]
fn batches_are_capped_at_a_thousand() {
    let batch = |n: usize| Value::Array((0..n).map(|i| json!({"jsonrpc": "2.0", "id": i, "method": "ping"})).collect()).to_string();
    let mut old = oracle::Server::new(Box::new(Headless::default()));
    let mut new = Server::new(Box::new(Headless::default()));
    let (a, b) = (old.handle_line(&batch(1000)).unwrap(), new.handle_line(&batch(1000)).unwrap());
    assert_eq!(a, b);
    let a: Value = serde_json::from_str(&old.handle_line(&batch(1001)).unwrap()).unwrap();
    let b: Value = serde_json::from_str(&new.handle_line(&batch(1001)).unwrap()).unwrap();
    assert_eq!(a.as_array().map(Vec::len), Some(1001));
    assert_eq!(b["error"]["code"], -32600);
}

/// Difference 5 (accepted, stricter): arguments the tool's schema does not list, or of another
/// type, are a tool error before the tool runs. The old server passed them on and ignored them.
#[test]
fn unknown_and_mistyped_arguments_are_tool_errors() {
    let mut new = Server::new(Box::new(Headless::default()));
    for (name, args, needle) in [
        ("type_text", json!({"text": "x", "bogus": 1}), "unknown argument `bogus`"),
        ("get_text", json!({"x": 1}), "unknown argument `x`"),
        ("type_text", json!({"text": 5}), "must be of type"),
        ("select_text", json!({"text": "x", "occurrence": "1"}), "must be of type"),
    ] {
        let r = new.handle(json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": name, "arguments": args}})).unwrap();
        assert_eq!(r["result"]["isError"], true, "{r}");
        assert!(r["result"]["content"][0]["text"].as_str().unwrap().contains(needle), "{r}");
        assert!(r.get("error").is_none(), "a tool failure is a result, not a JSON-RPC error: {r}");
    }
}

/// Difference 6 (accepted, stricter): a required argument that is missing is refused before the
/// tool runs, where `type_text` and `select_text` used to treat a missing `text` as "". The
/// schemas have always listed `text` as required, so the call was never valid.
#[test]
fn a_missing_required_argument_is_refused_where_the_tool_defaulted_it() {
    let mut old = oracle::Server::new(Box::new(Headless::default()));
    let mut new = Server::new(Box::new(Headless::default()));
    for name in ["type_text", "select_text"] {
        let line = String::from_utf8(req(json!(1), "tools/call", json!({"name": name, "arguments": {}}))).unwrap();
        let o: Value = serde_json::from_str(&old.handle_line(&line).unwrap()).unwrap();
        let n: Value = serde_json::from_str(&new.handle_line(&line).unwrap()).unwrap();
        // type_text {} used to type nothing and succeed; select_text {} failed to find "".
        assert_eq!(o["result"]["isError"], name == "select_text", "{name}: {o}");
        assert_eq!(n["result"]["isError"], true, "{name}: {n}");
        assert!(n["result"]["content"][0]["text"].as_str().unwrap().contains("missing required argument `text`"), "{n}");
    }
}

/// Every argument the tools read, and every one the documentation shows, passes the check: the
/// schemas list what the tools accept, so no call that worked now fails.
#[test]
fn every_documented_call_passes_the_argument_check() {
    use craft_mcp_server::check_arguments;
    let defs = crate::tools::tool_definitions();
    let defs = defs.as_array().unwrap();
    for (name, args) in [
        ("list_commands", json!({"query": "x"})),
        ("execute", json!({"command": "insert.table", "params": {"rows": 3, "cols": 4}, "doc": 3})),
        ("insert_table", json!({"values": [["Month", "Cases"], ["January", 12]], "header": true, "style": "TableGrid", "doc": 1})),
        ("batch", json!({"commands": [{"command": "a"}], "keepGoing": true, "doc": 0})),
        ("new_document", json!({"template": "letter"})),
        ("open_document", json!({"path": "a.docx", "password": "p"})),
        ("save_document", json!({"path": "a.pdf", "doc": 2})),
        ("type_text", json!({"text": "x", "paragraphs": true, "doc": 2})),
        ("select_text", json!({"text": "x", "occurrence": 2, "doc": 2})),
        ("get_text", json!({"doc": 2})),
        ("inspect_document", json!({"doc": 2})),
        ("render_page", json!({"page": 1, "scale": 0.5, "doc": 2})),
        ("list_documents", json!({})),
        ("parity", json!({})),
        ("screenshot", json!({})),
        ("click", json!({"x": 1, "y": 2, "count": 2, "button": "right", "shift": true, "alt": true, "cmd": true, "ctrl": true, "command": true})),
        ("key", json!({"key": "B", "shift": true, "alt": true, "cmd": true, "ctrl": true, "command": true})),
        ("ui_inspect", json!({})),
    ] {
        assert_eq!(check_arguments(defs, name, &args), Ok(()), "{name}");
    }
    // Every tool is covered above.
    assert_eq!(defs.len(), 18);
}

/// Hostile and random input never panics or breaks the stream: every reply is one JSON line.
#[test]
fn random_lines_never_panic() {
    let mut x: u64 = 0x9e37_79b9_7f4a_7c15;
    let mut next = move || {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        x
    };
    let valid: Vec<Vec<u8>> = script().into_iter().map(|s| s.line).filter(|l| !l.is_empty()).collect();
    let toks: [&[u8]; 14] =
        [b"{", b"}", b"[", b"]", b"\"", b":", b",", b"null", b"\"method\"", b"\"id\"", b"\"params\"", b"\"tools/call\"", b"\"arguments\"", b"\xff"];
    let mut input = Vec::new();
    for _ in 0..1500 {
        let mut line = match next() % 3 {
            // Random bytes.
            0 => (0..next() % 64).map(|_| (next() & 0xff) as u8).collect(),
            // JSON-ish tokens.
            1 => (0..next() % 20).flat_map(|_| toks[(next() % 14) as usize].to_vec()).collect(),
            // A valid request, mutated: bytes flipped, cut, or doubled.
            _ => {
                let mut l = valid[(next() % valid.len() as u64) as usize].clone();
                for _ in 0..next() % 4 {
                    if l.is_empty() {
                        break;
                    }
                    let at = (next() % l.len() as u64) as usize;
                    match next() % 3 {
                        0 => l[at] = (next() & 0xff) as u8,
                        1 => l.truncate(at),
                        _ => l.insert(at, toks[(next() % 14) as usize][0]),
                    }
                }
                l
            }
        };
        line.retain(|&b| b != b'\n');
        // Keep random lines from naming tools that touch the disk.
        if String::from_utf8_lossy(&line).contains("save_document") || String::from_utf8_lossy(&line).contains("open_document") {
            continue;
        }
        input.extend(line);
        input.push(b'\n');
    }
    let replies = run_shared(&input);
    for r in replies {
        assert!(r.is_object() || r.is_array(), "{r}");
    }
}
