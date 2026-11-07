use serde_json::{Value, json};
use wordcraft_mcp::{Backend, tools::call_tool};

#[derive(Default)]
struct Recording {
    seen: Option<(String, Value)>,
}
impl Backend for Recording {
    fn call(&mut self, method: &str, params: Value) -> Result<Value, String> {
        self.seen = Some((method.into(), params));
        Ok(json!({"none":true}))
    }
    fn has_ui(&self) -> bool {
        true
    }
    fn describe(&self) -> String {
        "test window".into()
    }
}

#[test]
fn omitted_wait_arguments_use_the_declared_defaults() {
    let mut backend = Recording::default();
    let result = call_tool(&mut backend, "wait_for_request", &Value::Null);
    assert!(!result.is_error, "arguments are optional: {result:?}");
    assert_eq!(backend.seen, Some(("agent.listen".into(), json!({"timeoutMs":30000}))));
}

#[test]
fn proposals_and_comments_are_forwarded_without_execution_or_truncation() {
    let mut backend = Recording::default();
    let args = json!({"id":7,"proposal":"{\"command\":\"file.close\"}","format":"json","comment":"quote \" and newline\n"});
    let result = call_tool(&mut backend, "reply_to_request", &args);
    assert!(!result.is_error, "{result:?}");
    let (method, params) = backend.seen.expect("one forwarding call");
    assert_eq!(method, "agent.reply");
    assert_eq!(params["proposal"], args["proposal"]);
    assert_eq!(params["comment"], args["comment"]);
    assert_eq!(params["format"], args["format"]);
    assert_eq!(params["done"], true);
}

#[test]
fn invalid_done_type_is_refused_before_forwarding() {
    let mut backend = Recording::default();
    let result = call_tool(&mut backend, "reply_to_request", &json!({"id":7,"text":"done","done":"yes"}));
    assert!(result.is_error);
    assert!(backend.seen.is_none());
}
