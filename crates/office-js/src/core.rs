//! What a `RequestContext` keeps between syncs, independent of JavaScript: the queued actions,
//! where each load's result goes, the objects named in the batch, and the properties known so
//! far, keyed by object path. Proxies are a context and a path, so `paragraph.font` reached
//! twice is the same object.

use std::collections::HashMap;

use serde_json::{Map, Value as Json, json};

/// A path from a root (`document`, `settings`) to an object: steps as JSON (`{"p": "body"}`,
/// `{"m": "getRange", "a": ["End"]}`).
pub type Path = Vec<Json>;

/// A property step.
pub fn prop(name: &str) -> Json {
    json!({"p": name})
}

/// A method step.
pub fn method(name: &str, args: Vec<Json>) -> Json {
    json!({"m": name, "a": args})
}

/// `path` with `step` after it.
pub fn child(path: &[Json], step: Json) -> Path {
    let mut p = path.to_vec();
    p.push(step);
    p
}

fn key(path: &[Json]) -> String {
    serde_json::to_string(path).unwrap_or_default()
}

/// An id as the host answers it (`"p:3"`, or a number), as a string.
pub fn id_string(v: &Json) -> Option<String> {
    match v {
        Json::String(s) => Some(s.clone()),
        Json::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

/// The kinds of object the host names by id, and how: a paragraph by
/// `document.paragraphs.getById(id)`, a range by `document.getRangeById(id)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Named {
    Paragraph,
    Range,
}

impl Named {
    fn base(self) -> Path {
        match self {
            Named::Paragraph => vec![prop("document"), prop("paragraphs")],
            Named::Range => vec![prop("document")],
        }
    }
    fn method(self) -> &'static str {
        match self {
            Named::Paragraph => "getById",
            Named::Range => "getRangeById",
        }
    }
    /// The path naming the object `id` (an id, or a reference the context replaces on the wire).
    pub fn path(self, id: Json) -> Path {
        child(&self.base(), method(self.method(), vec![id]))
    }
}

/// An Office.js error (`OfficeExtension.Error`): its code, message and which action failed.
#[derive(Clone, Debug, PartialEq)]
pub struct OfficeError {
    pub code: String,
    pub message: String,
    pub action: Option<u64>,
}

impl OfficeError {
    pub fn new(code: &str, message: impl Into<String>) -> OfficeError {
        OfficeError { code: code.into(), message: message.into(), action: None }
    }
    pub fn not_loaded(prop: &str) -> OfficeError {
        OfficeError::new(
            "PropertyNotLoaded",
            format!(
                "The property '{prop}' is not available. Before reading the property's value, call the load method on the containing object and call \"context.sync()\" on the associated request context."
            ),
        )
    }
    pub fn not_implemented(what: &str) -> OfficeError {
        OfficeError::new("NotImplemented", format!("{what} isn't supported by WordCraft yet"))
    }
    pub fn invalid(message: impl Into<String>) -> OfficeError {
        OfficeError::new("InvalidArgument", message)
    }
}

/// An object named by reference (`{"ref": r}` in a path step): one an `insert*` call creates
/// (`nav` is `None`), or one a navigation method reaches (`nav`: the steps that reach it). While
/// a queued action answers its id (`action`), the wire names it `{"result": action}`; once a sync
/// answered, by `id`.
struct Ref {
    named: Named,
    action: Option<usize>,
    id: Option<String>,
    nav: Option<Path>,
}

impl Ref {
    fn unpinned(&self) -> bool {
        self.action.is_none() && self.id.is_none() && self.nav.is_some()
    }
}

/// What a sync waits for: loads (the action's index, the object's path, how its items are
/// named), references (the reference, the action answering its id) and listings (the action, the
/// key its items are kept under, how they're named).
pub struct Pending {
    loads: Vec<(usize, Path, Option<Named>)>,
    refs: Vec<(usize, usize)>,
    listings: Vec<(usize, Path, Named)>,
}

#[derive(Default)]
pub struct Ctx {
    queue: Vec<Json>,
    loads: Vec<(usize, Path, Option<Named>)>,
    listings: Vec<(usize, Path, Named)>,
    refs: Vec<Ref>,
    listed: usize,
    cache: HashMap<String, Map<String, Json>>,
}

/// The references (`{"ref": r}` arguments) in a path, in order.
fn refs_in(path: &[Json]) -> impl Iterator<Item = usize> + '_ {
    path.iter()
        .filter_map(|step| step.get("a").and_then(Json::as_array))
        .flatten()
        .filter_map(|a| a.get("ref").and_then(Json::as_u64).and_then(|r| usize::try_from(r).ok()))
}

impl Ctx {
    pub fn new() -> Ctx {
        Ctx::default()
    }

    /// Whether actions are waiting for a sync.
    pub fn has_queued(&self) -> bool {
        !self.queue.is_empty()
    }

    fn first_unpinned(&self, path: &[Json]) -> Option<usize> {
        refs_in(path).find(|r| self.refs.get(*r).is_some_and(Ref::unpinned))
    }

    /// The unpinned reference to pin first for `path`: one whose own steps name no unpinned
    /// reference (a navigation from an object that isn't pinned yet pins that object first).
    fn next_to_pin(&self, path: &[Json]) -> Option<usize> {
        let mut r = self.first_unpinned(path)?;
        // References only name earlier ones, so this ends; the bound is a guard.
        for _ in 0..=self.refs.len() {
            match self.refs.get(r).and_then(|x| x.nav.as_deref()).and_then(|nav| self.first_unpinned(nav)) {
                Some(inner) if inner != r => r = inner,
                _ => return Some(r),
            }
        }
        Some(r)
    }

    /// Pins every unpinned reference `path` names: queues a load of its id along its steps.
    fn pin(&mut self, path: &[Json]) {
        for _ in 0..=self.refs.len() {
            let Some(r) = self.next_to_pin(path) else { return };
            let Some(nav) = self.refs.get(r).and_then(|x| x.nav.clone()) else { return };
            let wired = self.wire(&nav);
            self.queue.push(json!({"op": "load", "path": wired, "props": ["id"]}));
            let i = self.queue.len() - 1;
            if let Some(x) = self.refs.get_mut(r) {
                x.action = Some(i);
            }
        }
    }

    /// `path` as the host reads it: references replaced by the objects' ids, by `{"result": i}`
    /// (the id action `i` of the same sync answers), or (an object whose sync failed) by `""`,
    /// which no object has.
    pub fn wire(&self, path: &[Json]) -> Path {
        path.iter()
            .map(|step| {
                let Some(args) = step.get("a").and_then(Json::as_array) else { return step.clone() };
                if !args.iter().any(|a| a.get("ref").is_some()) {
                    return step.clone();
                }
                let args: Vec<Json> = args
                    .iter()
                    .map(|a| match a.get("ref").and_then(Json::as_u64).and_then(|r| usize::try_from(r).ok()) {
                        Some(r) => match self.refs.get(r) {
                            Some(Ref { id: Some(id), .. }) => json!(id),
                            Some(Ref { action: Some(i), .. }) => json!({"result": i}),
                            _ => json!(""),
                        },
                        None => a.clone(),
                    })
                    .collect();
                let mut step = step.clone();
                step["a"] = Json::Array(args);
                step
            })
            .collect()
    }

    fn push(&mut self, mut action: Json) -> usize {
        if let Some(path) = action.get("path").and_then(Json::as_array).cloned() {
            self.pin(&path);
            action["path"] = Json::Array(self.wire(&path));
        }
        self.queue.push(action);
        self.queue.len() - 1
    }

    fn push_load(&mut self, path: &[Json], props: Vec<String>, items: Option<Named>) {
        let i = self.push(json!({"op": "load", "path": path, "props": props}));
        self.loads.push((i, path.to_vec(), items));
    }

    /// Queues loading `props` of the object at `path`. A property path (`font/bold`) loads that
    /// property of the object it leads to; `items` and `items/text` load a collection's items
    /// (with `text` of each), named as `items` says; no properties loads the host's defaults.
    pub fn load(&mut self, path: &[Json], props: &[String], items_named: Option<Named>) -> Result<(), OfficeError> {
        let mut own = Vec::new();
        let mut items: Option<Vec<String>> = None;
        let mut nested: Vec<(Path, Vec<String>)> = Vec::new();
        for p in props.iter().map(|p| p.trim()).filter(|p| !p.is_empty()) {
            let parts: Vec<&str> = p.split('/').map(str::trim).collect();
            if parts.first() == Some(&"items") {
                let list = items.get_or_insert_with(Vec::new);
                match parts.as_slice() {
                    [_] => {}
                    [_, sub] => {
                        if !list.iter().any(|q| q == sub) {
                            list.push(sub.to_string());
                        }
                    }
                    _ => return Err(OfficeError::not_implemented(&format!("loading '{p}' (a property of the items' objects)"))),
                }
                continue;
            }
            match parts.split_last() {
                Some((last, [])) => own.push(last.to_string()),
                Some((last, steps)) => {
                    let mut sub = path.to_vec();
                    sub.extend(steps.iter().map(|s| prop(s)));
                    match nested.iter_mut().find(|(q, _)| *q == sub) {
                        Some((_, list)) => list.push(last.to_string()),
                        None => nested.push((sub, vec![last.to_string()])),
                    }
                }
                None => {}
            }
        }
        let had_items = items.is_some();
        if let Some(list) = items {
            let mut props = vec!["items".to_string()];
            props.extend(list);
            self.push_load(path, props, items_named);
        }
        if !own.is_empty() || (nested.is_empty() && !had_items) {
            self.push_load(path, own, None);
        }
        for (sub, list) in nested {
            self.push_load(&sub, list, None);
        }
        Ok(())
    }

    /// Queues setting a property; the new value is known at once.
    pub fn set(&mut self, path: &[Json], prop: &str, value: Json) {
        self.push(json!({"op": "set", "path": path, "prop": prop, "value": value.clone()}));
        self.cache.entry(key(path)).or_default().insert(prop.to_string(), value);
    }

    /// Queues a method called for its effect.
    pub fn call(&mut self, path: &[Json], method: &str, args: Vec<Json>) {
        self.push(json!({"op": "call", "path": path, "method": method, "args": args}));
    }

    /// Queues a method that creates an object and answers its id (`insertParagraph`,
    /// `insertText`); the path naming the new object, before and after the sync.
    pub fn call_creating(&mut self, path: &[Json], method: &str, args: Vec<Json>, named: Named) -> Path {
        let i = self.push(json!({"op": "call", "path": path, "method": method, "args": args}));
        self.refs.push(Ref { named, action: Some(i), id: None, nav: None });
        named.path(json!({"ref": self.refs.len() - 1}))
    }

    /// The path naming the object the steps `nav` reach (`getSelection()`, `getNext()`), pinned
    /// by the first action that uses it.
    pub fn navigate(&mut self, nav: Path, named: Named) -> Path {
        self.refs.push(Ref { named, action: None, id: None, nav: Some(nav) });
        named.path(json!({"ref": self.refs.len() - 1}))
    }

    /// Queues a method that answers a list of objects (`search` → `{"items": [{id, …}]}`); the
    /// key the list is kept under (a path the host never sees).
    pub fn call_listing(&mut self, path: &[Json], method: &str, args: Vec<Json>, named: Named) -> Path {
        let i = self.push(json!({"op": "call", "path": path, "method": method, "args": args}));
        self.listed += 1;
        let at = child(path, json!({"listing": self.listed, "m": method}));
        self.listings.push((i, at.clone(), named));
        at
    }

    /// A property known so far (loaded, or set since).
    pub fn get(&self, path: &[Json], prop: &str) -> Option<&Json> {
        self.cache.get(&key(path)).and_then(|m| m.get(prop))
    }

    /// Every property known so far of the object at `path`.
    pub fn object(&self, path: &[Json]) -> Option<&Map<String, Json>> {
        self.cache.get(&key(path))
    }

    /// The queued actions, taken for one sync, and what waits for its answer.
    pub fn take(&mut self) -> (Vec<Json>, Pending) {
        let mut refs = Vec::new();
        for (r, x) in self.refs.iter_mut().enumerate() {
            if let Some(i) = x.action.take() {
                refs.push((r, i));
            }
        }
        let pending = Pending { loads: std::mem::take(&mut self.loads), refs, listings: std::mem::take(&mut self.listings) };
        (std::mem::take(&mut self.queue), pending)
    }

    fn keep_items(&mut self, path: &[Json], props: &Map<String, Json>, named: Option<Named>) {
        // A collection's items become objects of their own: `paragraphs.items[0].text`.
        if let (Some(named), Some(Json::Array(items))) = (named, props.get("items")) {
            for item in items {
                if let Some(id) = item.get("id").and_then(id_string)
                    && let Json::Object(fields) = item
                {
                    self.cache.entry(key(&named.path(json!(id)))).or_default().extend(fields.clone());
                }
            }
        }
        self.cache.entry(key(path)).or_default().extend(props.clone());
    }

    /// Reads a sync's answer: `{"results": [...]}` fills the loaded objects, the listings and the
    /// ids of the named ones; `{"error": …}` is the error the sync rejects with.
    pub fn apply(&mut self, pending: Pending, reply: &Json) -> Result<Json, OfficeError> {
        let results = results(reply)?;
        for (r, i) in pending.refs {
            let Some(id) = results.get(i).and_then(|v| v.get("id")).and_then(id_string) else { continue };
            if let Some(x) = self.refs.get_mut(r) {
                x.id = Some(id.clone());
                let at = x.named.path(json!({"ref": r}));
                self.cache.entry(key(&at)).or_default().insert("id".into(), json!(id));
            }
        }
        for (i, path, named) in pending.loads {
            if let Some(Json::Object(props)) = results.get(i) {
                self.keep_items(&path, props, named);
            }
        }
        for (i, path, named) in pending.listings {
            if let Some(Json::Object(props)) = results.get(i) {
                self.keep_items(&path, props, Some(named));
            }
        }
        Ok(Json::Array(results.to_vec()))
    }
}

/// A sync's answer: the results, one per action, or the error it carries.
pub fn results(reply: &Json) -> Result<&[Json], OfficeError> {
    if let Some(e) = reply.get("error") {
        return Err(OfficeError {
            code: e.get("code").and_then(Json::as_str).unwrap_or("GeneralException").to_string(),
            message: e.get("message").and_then(Json::as_str).unwrap_or("The request failed").to_string(),
            action: e.get("action").and_then(Json::as_u64),
        });
    }
    reply
        .get("results")
        .and_then(Json::as_array)
        .map(Vec::as_slice)
        .ok_or_else(|| OfficeError::new("GeneralException", "the host's answer has no results"))
}

/// The property names a `load()` argument names: `"a, b"`, `["a", "b"]`, `{select: "a,b"}` or
/// `{a: true, b: true}` (already turned into JSON).
pub fn load_props(arg: &Json) -> Vec<String> {
    match arg {
        Json::String(s) => s.split(',').map(|p| p.trim().to_string()).filter(|p| !p.is_empty()).collect(),
        Json::Array(list) => list.iter().filter_map(Json::as_str).map(str::to_string).collect(),
        Json::Object(o) => match o.get("select") {
            Some(sel) => load_props(sel),
            None => o.iter().filter(|(_, v)| v.as_bool() == Some(true)).map(|(k, _)| k.clone()).collect(),
        },
        _ => Vec::new(),
    }
}

/// A collection's `load()` properties as its items' (Word's collections have no scalar
/// properties of their own: `paragraphs.load("text")` is `items/text`).
pub fn as_item_props(props: &[String]) -> Vec<String> {
    props
        .iter()
        .map(|p| p.trim())
        .filter(|p| !p.is_empty())
        .map(|p| if p == "items" || p.starts_with("items/") { p.to_string() } else { format!("items/{p}") })
        .collect()
}

/// An enum's value as the API spells it: `end` → `"End"`, `gridTable1Light_Accent1` →
/// `"GridTable1Light_Accent1"`.
pub fn enum_value(key: &str) -> String {
    let mut chars = key.chars();
    match chars.next() {
        Some(c) => c.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// The requirement sets reported as supported, with the highest version.
/// WordApi is answered as 1.3 although only its text, paragraph, range, font and properties
/// objects are there (no content controls, pictures, tables or HTML/OOXML): add-ins check the set
/// before they start, and the other craft apps report their partial ExcelApi and PowerPointApi
/// sets the same way. What's missing is listed in docs/officejs.md.
pub const REQUIREMENT_SETS: &[(&str, (u32, u32))] = &[("WordApi", (1, 3)), ("Settings", (1, 1)), ("Selection", (1, 1)), ("TextCoercion", (1, 1))];

/// `Office.context.requirements.isSetSupported(name, minVersion)`.
pub fn is_set_supported(name: &str, min: Option<&str>) -> bool {
    let Some((_, have)) = REQUIREMENT_SETS.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)) else { return false };
    let want = min.map(version).unwrap_or((1, 1));
    want <= *have
}

fn version(v: &str) -> (u32, u32) {
    let mut parts = v.trim().split('.').map(|p| p.parse::<u32>().unwrap_or(0));
    (parts.next().unwrap_or(1), parts.next().unwrap_or(0))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body() -> Path {
        vec![prop("document"), prop("body")]
    }

    #[test]
    fn loads_fill_the_objects_their_paths_name() {
        let mut c = Ctx::new();
        let para = Named::Paragraph.path(json!("p:2"));
        assert!(c.get(&para, "text").is_none());
        c.load(&para, &["text".into(), "font/bold".into(), "style".into()], None).unwrap();
        let (actions, pending) = c.take();
        assert_eq!(actions.len(), 2);
        assert_eq!(
            actions[0],
            json!({"op": "load", "path": [{"p": "document"}, {"p": "paragraphs"}, {"m": "getById", "a": ["p:2"]}], "props": ["text", "style"]})
        );
        assert_eq!(actions[1]["path"][3], json!({"p": "font"}));
        assert!(!c.has_queued());
        c.apply(pending, &json!({"results": [{"text": "Hello", "style": "Normal"}, {"bold": null}]})).unwrap();
        assert_eq!(c.get(&para, "text"), Some(&json!("Hello")));
        assert_eq!(c.get(&child(&para, prop("font")), "bold"), Some(&Json::Null), "a mixed value is known, as null");
    }

    #[test]
    fn empty_load_asks_for_the_defaults_once() {
        let mut c = Ctx::new();
        c.load(&body(), &[], None).unwrap();
        assert_eq!(c.take().0, vec![json!({"op": "load", "path": body(), "props": []})]);
    }

    #[test]
    fn sets_are_known_at_once_and_errors_carry_their_code() {
        let mut c = Ctx::new();
        let font = child(&body(), prop("font"));
        c.set(&font, "bold", json!(true));
        assert_eq!(c.get(&font, "bold"), Some(&json!(true)));
        c.call(&body(), "clear", vec![]);
        let (actions, pending) = c.take();
        assert_eq!(actions[1]["method"], "clear");
        let e = c.apply(pending, &json!({"error": {"code": "ItemNotFound", "message": "no paragraph", "action": 1}})).unwrap_err();
        assert_eq!((e.code.as_str(), e.action), ("ItemNotFound", Some(1)));
        assert_eq!(results(&json!({})).unwrap_err().code, "GeneralException");
    }

    #[test]
    fn collection_items_become_objects_by_id() {
        let mut c = Ctx::new();
        let paras = child(&body(), prop("paragraphs"));
        let props = as_item_props(&["text".into(), "items/style".into(), "items".into()]);
        assert_eq!(props, ["items/text", "items/style", "items"]);
        c.load(&paras, &props, Some(Named::Paragraph)).unwrap();
        let (actions, pending) = c.take();
        assert_eq!(actions, vec![json!({"op": "load", "path": paras, "props": ["items", "text", "style"]})]);
        c.apply(pending, &json!({"results": [{"items": [{"id": "p:0", "text": "A", "style": "Normal"}, {"id": "p:1", "text": "B"}]}]})).unwrap();
        assert_eq!(c.get(&Named::Paragraph.path(json!("p:1")), "text"), Some(&json!("B")));
        assert_eq!(c.get(&paras, "items").and_then(Json::as_array).map(Vec::len), Some(2));
        assert!(c.load(&paras, &["items/font/bold".into()], Some(Named::Paragraph)).is_err());
    }

    #[test]
    fn created_objects_are_named_by_result_then_by_id() {
        let mut c = Ctx::new();
        c.set(&child(&body(), prop("font")), "italic", json!(false));
        let p = c.call_creating(&body(), "insertParagraph", vec![json!("Hi"), json!("End")], Named::Paragraph);
        c.set(&p, "styleBuiltIn", json!("Heading1"));
        let r = c.call_creating(&p, "insertText", vec![json!("!"), json!("End")], Named::Range);
        c.load(&child(&r, prop("font")), &["bold".into()], None).unwrap();
        let (actions, pending) = c.take();
        assert_eq!(actions[2]["path"], json!([{"p": "document"}, {"p": "paragraphs"}, {"m": "getById", "a": [{"result": 1}]}]));
        assert_eq!(actions[3]["path"][2], json!({"m": "getById", "a": [{"result": 1}]}));
        assert_eq!(actions[4]["path"], json!([{"p": "document"}, {"m": "getRangeById", "a": [{"result": 3}]}, {"p": "font"}]));
        c.apply(pending, &json!({"results": [null, {"id": "p:4"}, null, {"id": "r:p:4:2-p:4:3"}, {"bold": false}]})).unwrap();
        assert_eq!(c.get(&p, "id"), Some(&json!("p:4")));
        assert_eq!(c.get(&child(&r, prop("font")), "bold"), Some(&json!(false)));
        c.call(&p, "delete", vec![]);
        c.call(&r, "select", vec![]);
        let (actions, _) = c.take();
        assert_eq!(actions[0]["path"][2], json!({"m": "getById", "a": ["p:4"]}));
        assert_eq!(actions[1]["path"][1], json!({"m": "getRangeById", "a": ["r:p:4:2-p:4:3"]}));
        // An object whose sync failed names no object.
        let gone = c.call_creating(&body(), "insertParagraph", vec![json!(""), json!("Start")], Named::Paragraph);
        let (_, pending) = c.take();
        assert!(c.apply(pending, &json!({"error": {"code": "InvalidArgument"}})).is_err());
        c.call(&gone, "delete", vec![]);
        assert_eq!(c.take().0[0]["path"][2], json!({"m": "getById", "a": [""]}));
    }

    #[test]
    fn navigated_objects_are_pinned_by_their_first_use() {
        let mut c = Ctx::new();
        let paras = child(&body(), prop("paragraphs"));
        let first = c.navigate(child(&paras, method("getFirst", vec![])), Named::Paragraph);
        let next = c.navigate(child(&first, method("getNext", vec![])), Named::Paragraph);
        let unused = c.navigate(child(&next, method("getNext", vec![])), Named::Paragraph);
        assert!(!c.has_queued(), "an object that's only reached sends nothing");
        // Using `next` pins `first` (which it's reached from), then `next`.
        c.set(&next, "alignment", json!("Centered"));
        c.call(&first, "select", vec![]);
        let (actions, pending) = c.take();
        assert_eq!(
            actions,
            vec![
                json!({"op": "load", "path": [{"p": "document"}, {"p": "body"}, {"p": "paragraphs"}, {"m": "getFirst", "a": []}], "props": ["id"]}),
                json!({"op": "load", "path": [{"p": "document"}, {"p": "paragraphs"}, {"m": "getById", "a": [{"result": 0}]}, {"m": "getNext", "a": []}], "props": ["id"]}),
                json!({"op": "set", "path": [{"p": "document"}, {"p": "paragraphs"}, {"m": "getById", "a": [{"result": 1}]}], "prop": "alignment", "value": "Centered"}),
                json!({"op": "call", "path": [{"p": "document"}, {"p": "paragraphs"}, {"m": "getById", "a": [{"result": 0}]}], "method": "select", "args": []}),
            ]
        );
        c.apply(pending, &json!({"results": [{"id": "p:0"}, {"id": "p:1"}, null, null]})).unwrap();
        assert_eq!(c.get(&next, "alignment"), Some(&json!("Centered")));
        c.load(&unused, &["text".into()], None).unwrap();
        let (actions, pending) = c.take();
        assert_eq!(actions[0]["path"], json!([{"p": "document"}, {"p": "paragraphs"}, {"m": "getById", "a": ["p:1"]}, {"m": "getNext", "a": []}]));
        assert_eq!(actions[1]["path"][2], json!({"m": "getById", "a": [{"result": 0}]}));
        // A failed pin is tried again by the next use.
        assert!(c.apply(pending, &json!({"error": {"code": "ItemNotFound", "action": 0}})).is_err());
        c.call(&unused, "select", vec![]);
        let (actions, _) = c.take();
        assert_eq!(actions[0]["props"], json!(["id"]));
        assert_eq!(actions[1]["path"][2], json!({"m": "getById", "a": [{"result": 0}]}));
    }

    #[test]
    fn a_long_navigation_chain_is_pinned_without_recursion() {
        let mut c = Ctx::new();
        let mut p = c.navigate(vec![prop("document"), prop("body"), prop("paragraphs"), method("getFirst", vec![])], Named::Paragraph);
        for _ in 0..2000 {
            p = c.navigate(child(&p, method("getNext", vec![])), Named::Paragraph);
        }
        c.call(&p, "select", vec![]);
        let (actions, _) = c.take();
        assert_eq!(actions.len(), 2002);
        assert_eq!(actions[2001]["path"][2], json!({"m": "getById", "a": [{"result": 2000}]}));
    }

    #[test]
    fn listings_keep_their_items() {
        let mut c = Ctx::new();
        let at = c.call_listing(&body(), "search", vec![json!("fox"), json!({"matchCase": true})], Named::Range);
        let at2 = c.call_listing(&body(), "search", vec![json!("fox"), json!({})], Named::Range);
        assert_ne!(at, at2);
        let (actions, pending) = c.take();
        assert_eq!(actions[0], json!({"op": "call", "path": body(), "method": "search", "args": ["fox", {"matchCase": true}]}));
        c.apply(pending, &json!({"results": [{"items": [{"id": "r:p:0:4-p:0:7", "text": "fox"}]}, {"items": []}]})).unwrap();
        assert_eq!(c.get(&at, "items").and_then(Json::as_array).map(Vec::len), Some(1));
        assert_eq!(c.get(&Named::Range.path(json!("r:p:0:4-p:0:7")), "text"), Some(&json!("fox")));
    }

    #[test]
    fn enums_and_load_arguments_and_requirement_sets() {
        assert_eq!(enum_value("end"), "End");
        assert_eq!(enum_value("gridTable1Light_Accent1"), "GridTable1Light_Accent1");
        assert_eq!(enum_value(""), "");
        assert_eq!(load_props(&json!("text, style")), ["text", "style"]);
        assert_eq!(load_props(&json!(["text"])), ["text"]);
        assert_eq!(load_props(&json!({"select": "a,b"})), ["a", "b"]);
        assert_eq!(load_props(&json!({"text": true, "style": false})), ["text"]);
        assert!(is_set_supported("Settings", Some("1.1")) && is_set_supported("textcoercion", None));
        assert!(is_set_supported("WordApi", Some("1.3")) && is_set_supported("WordApi", None));
        assert!(!is_set_supported("WordApi", Some("1.4")));
        assert!(!is_set_supported("Settings", Some("1.2")) && !is_set_supported("ExcelApi", Some("1.1")));
        assert_eq!(id_string(&json!(3)), Some("3".into()));
    }
}
