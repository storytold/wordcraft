//! The window's chat (Review › Chat): open the pane, start and stop the chat, invite and remove
//! agents, post as the owner, list the members. The desktop app installs the chat
//! (`Session::chat`); without one (headless, web) every command is disabled with "no chat in
//! this window". None of them changes the document.

use std::sync::Arc;

use serde_json::json;
use wordcraft_chat::Chat;

use crate::{CmdError, CommandSpec, Session, p};

const LOC: &str = "Review › Chat";
const NO_CHAT: &str = "no chat in this window";

fn has_chat(s: &Session) -> Option<&'static str> {
    if s.chat.is_some() { None } else { Some(NO_CHAT) }
}

fn chat(s: &Session) -> Result<Arc<Chat>, CmdError> {
    s.chat.clone().ok_or_else(|| CmdError::Disabled(NO_CHAT.into()))
}

fn failed(e: impl ToString) -> CmdError {
    CmdError::Failed(e.to_string())
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("chat.open", "Chat", LOC, |s, v| {
            chat(s)?;
            let open = p::bool(v, "value").unwrap_or(true);
            s.ui_requests.push(json!({"chatPane": open}));
            Ok(json!({"open": open}))
        })
        .params(r#"{"value"?: bool}"#)
        .pure()
        .when(has_chat),
        CommandSpec::new("chat.start", "Start Chat", LOC, |s, _| {
            let addr = chat(s)?.start().map_err(failed)?;
            Ok(json!({"address": addr}))
        })
        .pure()
        .when(has_chat),
        CommandSpec::new("chat.stop", "Stop Chat", LOC, |s, _| {
            chat(s)?.stop();
            Ok(json!({}))
        })
        .pure()
        .when(has_chat),
        CommandSpec::new("chat.invite", "Invite Agent", LOC, |s, v| {
            let name = p::req_str(v, "name")?;
            let inv = chat(s)?.invite(name).map_err(failed)?;
            Ok(json!({"handle": inv.handle, "code": inv.code, "line": inv.line}))
        })
        .params(r#"{"name": string} -> {"handle", "code", "line"}"#)
        .pure()
        .when(has_chat),
        CommandSpec::new("chat.remove", "Remove Agent", LOC, |s, v| {
            let h = wordcraft_chat::rules::normalize_handle(p::req_str(v, "name")?);
            if chat(s)?.hub().remove(&h) { Ok(json!({"removed": h})) } else { Err(failed(format!("{h} is not in the chat"))) }
        })
        .params(r#"{"name": string}"#)
        .pure()
        .when(has_chat),
        CommandSpec::new("chat.post", "Send Chat Message", LOC, |s, v| {
            let text = p::req_str(v, "text")?;
            let c = chat(s)?;
            if !c.running() {
                return Err(failed(wordcraft_chat::NOT_STARTED));
            }
            let m = c.hub().post_owner(text).map_err(failed)?;
            serde_json::to_value(&m).map_err(failed)
        })
        .params(r#"{"text": string}"#)
        .pure()
        .when(has_chat),
        CommandSpec::new("chat.members", "Chat Members", LOC, |s, _| serde_json::to_value(chat(s)?.hub().members()).map_err(failed))
            .pure()
            .when(has_chat),
    ]
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use serde_json::json;
    use wordcraft_chat::testing::{FakePort, TestEnv};
    use wordcraft_chat::{Chat, Hub};

    use crate::Session;

    const IDS: [&str; 7] = ["chat.open", "chat.start", "chat.stop", "chat.invite", "chat.remove", "chat.post", "chat.members"];

    fn with_chat() -> Session {
        let mut s = Session::new(wordcraft_doc::Document::new());
        s.chat = Some(Arc::new(Chat::new(Hub::new(TestEnv::at(1_000)), FakePort::closed())));
        s
    }

    #[test]
    fn without_a_chat_every_chat_command_says_so() {
        let mut s = Session::new(wordcraft_doc::Document::new());
        for id in IDS {
            let e = s.run(id, &json!({"name": "@claude", "text": "hi"})).unwrap_err();
            assert!(e.to_string().contains("no chat in this window"), "{id}: {e}");
        }
    }

    #[test]
    fn invite_post_remove_with_a_chat() {
        let mut s = with_chat();
        let e = s.run("chat.invite", &json!({"name": "@claude"})).unwrap_err();
        assert!(e.to_string().contains("not started"), "{e}");
        assert_eq!(s.run("chat.start", &json!({})).unwrap()["address"], "127.0.0.1:7981");
        let inv = s.run("chat.invite", &json!({"name": "Claude"})).unwrap();
        assert_eq!(inv["handle"], "@claude");
        let code = inv["code"].as_str().unwrap().to_string();
        assert_eq!(inv["line"], format!("wordcraft-cli chat join 127.0.0.1:7981 {code} --as @claude"));
        s.chat.as_ref().unwrap().hub().join(&code).unwrap();
        assert_eq!(s.run("chat.members", &json!({})).unwrap()[0]["handle"], "@claude");
        let m = s.run("chat.post", &json!({"text": "please check clause 3"})).unwrap();
        assert_eq!((m["role"].clone(), m["from"].clone(), m["mentions"].clone()), (json!("owner"), json!("OWNER"), json!(["@claude"])));
        assert_eq!(s.run("chat.remove", &json!({"name": "claude"})).unwrap()["removed"], "@claude");
        assert!(s.run("chat.remove", &json!({"name": "@claude"})).is_err(), "not a member any more");
        assert_eq!(s.run("chat.members", &json!({})).unwrap(), json!([]));
        s.run("chat.stop", &json!({})).unwrap();
        assert!(s.run("chat.post", &json!({"text": "x"})).unwrap_err().to_string().contains("not started"));
        assert!(!s.dirty, "chat commands never touch the document");
        assert_eq!(s.undo_depth(), 0);
    }

    #[test]
    fn open_asks_the_ui_for_the_pane() {
        let mut s = with_chat();
        s.run("chat.open", &json!({})).unwrap();
        s.run("chat.open", &json!({"value": false})).unwrap();
        assert_eq!(s.ui_requests, vec![json!({"chatPane": true}), json!({"chatPane": false})]);
    }

    #[test]
    fn junk_params_never_panic_with_a_chat() {
        let junk = [
            json!(null),
            json!({}),
            json!({"name": 5, "text": [], "value": "x"}),
            json!({"name": "@\u{202e}x".repeat(1000), "text": "\n".repeat(200_000), "value": 1e308}),
        ];
        for id in IDS {
            for j in &junk {
                let mut s = with_chat();
                let _ = s.run("chat.start", &json!({}));
                let _ = s.run(id, j);
            }
        }
    }
}
