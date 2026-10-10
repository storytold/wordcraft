//! Loopback JSON-lines control server: one request per line, one reply per line.
//! This is the transport the MCP server (`wordcraft mcp --connect`) wraps.

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Duration;

use serde_json::{Value, json};
use wordcraft_ui_egui::ControlRequest;

pub fn start(port: u16, ctx: egui::Context) -> Receiver<ControlRequest> {
    let (tx, rx) = channel::<ControlRequest>();
    let listener = match TcpListener::bind(("127.0.0.1", port)) {
        Ok(l) => l,
        Err(e) => {
            log::error!("control server failed to bind 127.0.0.1:{port}: {e}");
            return rx;
        }
    };
    log::info!("control server listening on 127.0.0.1:{port}");
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let tx = tx.clone();
            let ctx = ctx.clone();
            std::thread::spawn(move || serve(stream, tx, ctx));
        }
    });
    rx
}

fn serve(stream: TcpStream, tx: Sender<ControlRequest>, ctx: egui::Context) {
    let Ok(read) = stream.try_clone() else { return };
    serve_lines(BufReader::new(read), stream, &tx, &ctx);
}

/// Answer requests until the peer hangs up or sends a line that isn't JSON. Closing on bad
/// input matters: an HTTP request (say, a web page's `fetch` to this port) starts with a
/// request line that never parses, so it can't carry a command in its body.
fn serve_lines(input: impl BufRead, mut out: impl Write, tx: &Sender<ControlRequest>, ctx: &egui::Context) {
    for line in input.lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let msg = match serde_json::from_str::<Value>(&line) {
            Ok(msg) => msg,
            Err(e) => {
                let _ = writeln!(out, "{}", json!({"ok": false, "error": format!("bad JSON: {e}; closing the connection")}));
                break;
            }
        };
        let id = msg.get("id").cloned().unwrap_or(Value::Null);
        let method = msg.get("method").and_then(Value::as_str).unwrap_or("").to_string();
        let params = msg.get("params").cloned().unwrap_or(json!({}));
        let (req, rrx) = ControlRequest::new(method, params);
        if tx.send(req).is_err() {
            break;
        }
        ctx.request_repaint();
        let mut reply = rrx.recv_timeout(Duration::from_secs(60)).unwrap_or_else(|_| json!({"ok": false, "error": "timeout"}));
        if let Some(o) = reply.as_object_mut() {
            o.insert("id".into(), id);
        }
        if writeln!(out, "{reply}").is_err() {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Run `input` through the server with an app that answers every request with its method
    /// name; returns the methods the app received and the reply lines.
    fn run(input: &str) -> (Vec<String>, Vec<Value>) {
        let (tx, rx) = channel::<ControlRequest>();
        let app = std::thread::spawn(move || {
            let mut seen = Vec::new();
            for req in rx {
                let _ = req.reply.send(json!({"ok": true, "result": req.method}));
                seen.push(req.method);
            }
            seen
        });
        let mut out = Vec::new();
        serve_lines(input.as_bytes(), &mut out, &tx, &egui::Context::default());
        drop(tx);
        let seen = app.join().unwrap();
        let replies = String::from_utf8(out).unwrap().lines().map(|l| serde_json::from_str(l).unwrap()).collect();
        (seen, replies)
    }

    #[test]
    fn json_lines_get_replies() {
        let (seen, replies) = run("{\"id\":1,\"method\":\"format.bold\"}\n\n{\"id\":2,\"method\":\"ui.inspect\"}\n");
        assert_eq!(seen, ["format.bold", "ui.inspect"]);
        assert_eq!(replies.len(), 2);
        assert_eq!(replies[1]["id"], 2);
        assert_eq!(replies[1]["result"], "ui.inspect");
    }

    #[test]
    fn bad_json_closes_the_connection() {
        let (seen, replies) = run("not json\n{\"id\":1,\"method\":\"format.bold\"}\n");
        assert!(seen.is_empty());
        assert_eq!(replies.len(), 1);
        assert_eq!(replies[0]["ok"], false);
    }

    #[test]
    fn http_request_cannot_smuggle_a_command() {
        // What a web page's `fetch("http://127.0.0.1:<port>/", {method: "POST", body})` sends.
        let body = "\n{\"method\":\"file.saveAs\",\"params\":{\"path\":\"/tmp/x.docx\"}}\n";
        let req = format!("POST / HTTP/1.1\r\nHost: 127.0.0.1:7981\r\nContent-Type: text/plain\r\nContent-Length: {}\r\n\r\n{body}", body.len());
        let (seen, _) = run(&req);
        assert!(seen.is_empty(), "{seen:?}");
    }
}
