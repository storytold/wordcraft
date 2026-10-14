//! The library as an add-in sees it — `Word.run`, proxies, `context.sync()`, the common API —
//! against a fake host that answers `office.sync` per the bridge contract (an in-memory
//! document, all or nothing per sync). Run with `wasm-bindgen-test-runner` (Node).

use std::cell::RefCell;

use js_sys::{Array, Function, Promise, Reflect};
use serde_json::{Map, Value as Json, json};
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::{JsFuture, future_to_promise, spawn_local};
use wasm_bindgen_test::wasm_bindgen_test;

use super::{from_js, install, install_with_transport, to_js};

// ---------------------------------------------------------------- the fake host

const PARA_PROPS: &[&str] =
    &["style", "styleBuiltIn", "alignment", "firstLineIndent", "leftIndent", "rightIndent", "lineSpacing", "spaceAfter", "spaceBefore"];
const FONT_PROPS: &[&str] = &["bold", "italic", "underline", "size", "name", "color", "highlightColor"];

#[derive(Clone)]
struct Para {
    text: String,
    props: Map<String, Json>,
    font: Map<String, Json>,
    list: bool,
}

impl Para {
    fn new(text: &str) -> Para {
        let props = obj_of(&[
            ("style", json!("Normal")),
            ("styleBuiltIn", json!("Normal")),
            ("alignment", json!("Left")),
            ("firstLineIndent", json!(0)),
            ("leftIndent", json!(0)),
            ("rightIndent", json!(0)),
            ("lineSpacing", json!(12)),
            ("spaceAfter", json!(8)),
            ("spaceBefore", json!(0)),
        ]);
        let font = obj_of(&[
            ("bold", json!(false)),
            ("italic", json!(false)),
            ("underline", json!("None")),
            ("size", json!(11)),
            ("name", json!("Calibri")),
            ("color", json!("#000000")),
            ("highlightColor", Json::Null),
        ]);
        Para { text: text.into(), props, font, list: false }
    }
}

/// A range: paragraph and byte offset of its start and of its end.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Rng {
    p0: usize,
    from: usize,
    p1: usize,
    to: usize,
}

impl Rng {
    fn id(&self) -> String {
        format!("r:p:{}:{}-p:{}:{}", self.p0, self.from, self.p1, self.to)
    }
    fn caret(p: usize, at: usize) -> Rng {
        Rng { p0: p, from: at, p1: p, to: at }
    }
}

#[derive(Clone)]
struct Doc {
    paras: Vec<Para>,
    sel: Rng,
    props: Map<String, Json>,
    settings: Json,
}

impl Doc {
    fn new() -> Doc {
        let mut second = Para::new("The quick brown fox jumps over the lazy dog. A fox again.");
        second.list = true;
        Doc {
            paras: vec![Para::new("Hello world"), second],
            sel: Rng { p0: 0, from: 0, p1: 0, to: 5 },
            props: obj_of(&[
                ("title", json!("Draft")),
                ("subject", json!("")),
                ("author", json!("Ada")),
                ("keywords", json!("")),
                ("comments", json!("")),
            ]),
            settings: json!({}),
        }
    }
    fn whole(&self) -> Rng {
        let last = self.paras.len().saturating_sub(1);
        Rng { p0: 0, from: 0, p1: last, to: self.paras.get(last).map_or(0, |p| p.text.len()) }
    }
    fn para_range(&self, i: usize) -> Rng {
        Rng { p0: i, from: 0, p1: i, to: self.paras[i].text.len() }
    }
    fn text_of(&self, r: Rng) -> String {
        if r.p0 == r.p1 {
            return self.paras[r.p0].text[r.from..r.to].to_string();
        }
        let mut parts = vec![self.paras[r.p0].text[r.from..].to_string()];
        parts.extend((r.p0 + 1..r.p1).map(|i| self.paras[i].text.clone()));
        parts.push(self.paras[r.p1].text[..r.to].to_string());
        parts.join("\r")
    }
    fn valid(&self, r: Rng) -> bool {
        r.p0 <= r.p1
            && r.p1 < self.paras.len()
            && r.from <= self.paras[r.p0].text.len()
            && r.to <= self.paras[r.p1].text.len()
            && (r.p0 < r.p1 || r.from <= r.to)
    }
    /// Inserts `text` at `(p, at)`; the range it fills.
    fn insert_at(&mut self, p: usize, at: usize, text: &str) -> Rng {
        self.paras[p].text.insert_str(at, text);
        Rng { p0: p, from: at, p1: p, to: at + text.len() }
    }
    /// Replaces the text of a range in one paragraph.
    fn replace(&mut self, r: Rng, text: &str) -> Result<Rng, Fail> {
        if r.p0 != r.p1 {
            return Err(("NotImplemented", "the fake host replaces text in one paragraph".into()));
        }
        self.paras[r.p0].text.replace_range(r.from..r.to, text);
        Ok(Rng { p0: r.p0, from: r.from, p1: r.p0, to: r.from + text.len() })
    }
}

thread_local! {
    static DOC: RefCell<Doc> = RefCell::new(Doc::new());
    static LOG: RefCell<Vec<Vec<Json>>> = const { RefCell::new(Vec::new()) };
}

#[derive(Clone, Copy, Debug)]
enum Of {
    Body,
    Para(usize),
    Range(Rng),
}

#[derive(Clone, Copy, Debug)]
enum T {
    Doc,
    Body,
    Props,
    Paras(Option<Rng>),
    Para(usize),
    Range(Rng),
    Font(Of),
    Settings,
}

type Fail = (&'static str, String);

fn not_found(what: impl std::fmt::Display) -> Fail {
    ("ItemNotFound", format!("{what} doesn't exist"))
}

fn obj_of(pairs: &[(&str, Json)]) -> Map<String, Json> {
    pairs.iter().map(|(k, v)| (k.to_string(), v.clone())).collect()
}

/// An argument, with `{"result": i}` replaced by the id action `i` answered.
fn arg(a: &Json, results: &[Json]) -> Json {
    match a.get("result").and_then(Json::as_u64) {
        Some(i) => results.get(i as usize).and_then(|r| r.get("id")).cloned().unwrap_or(Json::Null),
        None => a.clone(),
    }
}

fn parse_range(id: &str) -> Option<Rng> {
    let rest = id.strip_prefix("r:p:")?;
    let (a, b) = rest.split_once("-p:")?;
    let (p0, from) = a.split_once(':')?;
    let (p1, to) = b.split_once(':')?;
    Some(Rng { p0: p0.parse().ok()?, from: from.parse().ok()?, p1: p1.parse().ok()?, to: to.parse().ok()? })
}

/// The paragraphs a range or the body covers.
fn covered(d: &Doc, of: Of) -> Vec<usize> {
    match of {
        Of::Body => (0..d.paras.len()).collect(),
        Of::Para(i) => vec![i],
        Of::Range(r) => (r.p0..=r.p1).collect(),
    }
}

fn location_range(whole: Rng, loc: &str) -> Result<Rng, Fail> {
    match loc.to_ascii_lowercase().as_str() {
        "whole" | "content" => Ok(whole),
        "start" => Ok(Rng::caret(whole.p0, whole.from)),
        "end" => Ok(Rng::caret(whole.p1, whole.to)),
        other => Err(("InvalidArgument", format!("location {other}"))),
    }
}

fn resolve(d: &Doc, path: &[Json], results: &[Json]) -> Result<T, Fail> {
    let mut t: Option<T> = None;
    for step in path {
        let p = step.get("p").and_then(Json::as_str);
        let m = step.get("m").and_then(Json::as_str);
        let a: Vec<Json> = step.get("a").and_then(Json::as_array).map(|v| v.iter().map(|x| arg(x, results)).collect()).unwrap_or_default();
        let first = a.first().and_then(Json::as_str).unwrap_or_default().to_string();
        let in_scope = |scope: Option<Rng>| -> Vec<usize> { scope.map_or_else(|| (0..d.paras.len()).collect(), |r| (r.p0..=r.p1).collect()) };
        t = Some(match (t, p, m) {
            (None, Some("document"), _) => T::Doc,
            (None, Some("settings"), _) => T::Settings,
            (Some(T::Doc), Some("body"), _) => T::Body,
            (Some(T::Doc), Some("properties"), _) => T::Props,
            (Some(T::Doc), Some("paragraphs"), _) | (Some(T::Body), Some("paragraphs"), _) => T::Paras(None),
            (Some(T::Doc), _, Some("getSelection")) => T::Range(d.sel),
            (Some(T::Doc), _, Some("getRangeById")) => {
                T::Range(parse_range(&first).filter(|r| d.valid(*r)).ok_or_else(|| not_found(format!("range {first:?}")))?)
            }
            (Some(T::Body), Some("font"), _) => T::Font(Of::Body),
            (Some(T::Body), _, Some("getRange")) => T::Range(location_range(d.whole(), &first)?),
            (Some(T::Paras(scope)), _, Some("getFirst")) => T::Para(*in_scope(scope).first().ok_or_else(|| not_found("a first paragraph"))?),
            (Some(T::Paras(scope)), _, Some("getLast")) => T::Para(*in_scope(scope).last().ok_or_else(|| not_found("a last paragraph"))?),
            (Some(T::Paras(None)), _, Some("getById")) => {
                let i = first.strip_prefix("p:").and_then(|i| i.parse::<usize>().ok()).filter(|i| *i < d.paras.len());
                T::Para(i.ok_or_else(|| not_found(format!("paragraph {first:?}")))?)
            }
            (Some(T::Para(i)), Some("font"), _) => T::Font(Of::Para(i)),
            (Some(T::Para(i)), _, Some("getNext")) => {
                if i + 1 >= d.paras.len() {
                    return Err(not_found("the paragraph after the last one"));
                }
                T::Para(i + 1)
            }
            (Some(T::Para(i)), _, Some("getPrevious")) => T::Para(i.checked_sub(1).ok_or_else(|| not_found("the paragraph before the first one"))?),
            (Some(T::Para(i)), _, Some("getRange")) => T::Range(location_range(d.para_range(i), &first)?),
            (Some(T::Range(r)), Some("font"), _) => T::Font(Of::Range(r)),
            (Some(T::Range(r)), Some("paragraphs"), _) => T::Paras(Some(r)),
            (t, _, _) => return Err(("InvalidArgument", format!("no step {step} from {t:?}"))),
        });
    }
    t.ok_or_else(|| ("InvalidArgument", "empty path".into()))
}

fn pick(all: &Map<String, Json>, props: &[String]) -> Json {
    Json::Object(props.iter().filter_map(|p| all.get(p).map(|v| (p.clone(), v.clone()))).collect())
}

fn para_props(d: &Doc, i: usize) -> Map<String, Json> {
    let p = &d.paras[i];
    let mut m = p.props.clone();
    m.insert("id".into(), json!(format!("p:{i}")));
    m.insert("text".into(), json!(p.text));
    m.insert("isListItem".into(), json!(p.list));
    m
}

/// One value for many paragraphs: theirs if they agree, else `null` (mixed).
fn merged(values: impl Iterator<Item = Json>) -> Json {
    let mut out: Option<Json> = None;
    for v in values {
        match &out {
            None => out = Some(v),
            Some(o) if *o != v => return Json::Null,
            _ => {}
        }
    }
    out.unwrap_or(Json::Null)
}

fn load(d: &Doc, t: T, props: &[String]) -> Result<Json, Fail> {
    let rest: Vec<String> = props.iter().filter(|p| *p != "items").cloned().collect();
    if let T::Paras(scope) = t {
        let list: Vec<usize> = scope.map_or_else(|| (0..d.paras.len()).collect(), |r| (r.p0..=r.p1).collect());
        let mut want = rest.clone();
        want.insert(0, "id".into());
        return Ok(json!({"items": list.iter().map(|i| pick(&para_props(d, *i), &want)).collect::<Vec<_>>()}));
    }
    let all: Map<String, Json> = match t {
        T::Body => obj_of(&[
            ("text", json!(d.paras.iter().map(|p| p.text.clone()).collect::<Vec<_>>().join("\n"))),
            ("style", merged(d.paras.iter().map(|p| p.props["style"].clone()))),
        ]),
        T::Props => d.props.clone(),
        T::Para(i) => para_props(d, i),
        T::Range(r) => obj_of(&[
            ("id", json!(r.id())),
            ("text", json!(d.text_of(r))),
            ("isEmpty", json!(r.p0 == r.p1 && r.from == r.to)),
            ("style", merged(covered(d, Of::Range(r)).into_iter().map(|i| d.paras[i].props["style"].clone()))),
        ]),
        T::Font(of) => {
            let paras = covered(d, of);
            FONT_PROPS.iter().map(|k| (k.to_string(), merged(paras.iter().map(|i| d.paras[*i].font[*k].clone())))).collect()
        }
        T::Settings => return Ok(d.settings.clone()),
        _ => return Err(("InvalidArgument", format!("can't load {t:?}"))),
    };
    if rest.iter().any(|p| !all.contains_key(p)) {
        return Err(("InvalidArgument", format!("unknown property in {rest:?} of {t:?}")));
    }
    Ok(if rest.is_empty() { Json::Object(all) } else { pick(&all, &rest) })
}

fn set(d: &mut Doc, t: T, name: &str, v: Json) -> Result<(), Fail> {
    match t {
        T::Para(i) if PARA_PROPS.contains(&name) => {
            if name == "styleBuiltIn" {
                d.paras[i].props.insert("style".into(), v.clone());
            }
            d.paras[i].props.insert(name.into(), v);
        }
        T::Body if name == "style" => d.paras.iter_mut().for_each(|p| {
            p.props.insert("style".into(), v.clone());
        }),
        T::Range(r) if name == "style" => (r.p0..=r.p1).for_each(|i| {
            d.paras[i].props.insert("style".into(), v.clone());
        }),
        T::Font(of) if FONT_PROPS.contains(&name) => {
            for i in covered(d, of) {
                d.paras[i].font.insert(name.into(), v.clone());
            }
        }
        T::Props if d.props.contains_key(name) => {
            d.props.insert(name.into(), v);
        }
        _ => return Err(("InvalidArgument", format!("can't set {name} of {t:?}"))),
    }
    Ok(())
}

fn insert_paragraph(d: &mut Doc, at: usize, text: &str) -> Json {
    let at = at.min(d.paras.len());
    let style = d.paras.get(at.saturating_sub(1)).map(|p| p.props.clone());
    let mut p = Para::new(text);
    if let Some(style) = style {
        p.props = style;
    }
    d.paras.insert(at, p);
    json!({"id": format!("p:{at}")})
}

fn range_id(r: Rng) -> Result<Json, Fail> {
    Ok(json!({"id": r.id()}))
}

fn call(d: &mut Doc, t: T, name: &str, args: &[Json]) -> Result<Json, Fail> {
    let text = args.first().and_then(Json::as_str).unwrap_or_default().to_string();
    let loc = args.get(1).and_then(Json::as_str).unwrap_or_default().to_ascii_lowercase();
    match (t, name, loc.as_str()) {
        (T::Body, "insertParagraph", "start") => Ok(insert_paragraph(d, 0, &text)),
        (T::Body, "insertParagraph", "end") => Ok(insert_paragraph(d, d.paras.len(), &text)),
        (T::Para(i), "insertParagraph", "before") => Ok(insert_paragraph(d, i, &text)),
        (T::Para(i), "insertParagraph", "after") => Ok(insert_paragraph(d, i + 1, &text)),
        (T::Range(r), "insertParagraph", "before") => Ok(insert_paragraph(d, r.p0, &text)),
        (T::Range(r), "insertParagraph", "after") => Ok(insert_paragraph(d, r.p1 + 1, &text)),
        (T::Body, "insertText", "start") => range_id(d.insert_at(0, 0, &text)),
        (T::Body, "insertText", "end") => {
            let w = d.whole();
            range_id(d.insert_at(w.p1, w.to, &text))
        }
        (T::Para(i), "insertText", "start") => range_id(d.insert_at(i, 0, &text)),
        (T::Para(i), "insertText", "end") => {
            let end = d.paras[i].text.len();
            range_id(d.insert_at(i, end, &text))
        }
        (T::Para(i), "insertText", "replace") => {
            let r = d.para_range(i);
            range_id(d.replace(r, &text)?)
        }
        (T::Range(r), "insertText", "start" | "before") => range_id(d.insert_at(r.p0, r.from, &text)),
        (T::Range(r), "insertText", "end" | "after") => range_id(d.insert_at(r.p1, r.to, &text)),
        (T::Range(r), "insertText", "replace") => range_id(d.replace(r, &text)?),
        (_, "insertText" | "insertParagraph", other) => Err(("InvalidArgument", format!("location {other:?} for {name} on {t:?}"))),
        (T::Para(i), "delete", _) => {
            d.paras.remove(i);
            Ok(Json::Null)
        }
        (T::Range(r), "delete", _) => {
            d.replace(r, "")?;
            Ok(Json::Null)
        }
        (T::Para(i), "select", _) => {
            d.sel = d.para_range(i);
            Ok(Json::Null)
        }
        (T::Range(r), "select", _) => {
            d.sel = match args.first().and_then(Json::as_str) {
                Some("Start") => Rng::caret(r.p0, r.from),
                Some("End") => Rng::caret(r.p1, r.to),
                _ => r,
            };
            Ok(Json::Null)
        }
        (T::Body, "clear", _) => {
            d.paras = vec![Para::new("")];
            d.sel = Rng::caret(0, 0);
            Ok(Json::Null)
        }
        (T::Body, "search", _) => {
            let case = args.get(1).and_then(|o| o.get("matchCase")).and_then(Json::as_bool).unwrap_or(false);
            let fold = |s: &str| if case { s.to_string() } else { s.to_lowercase() };
            let mut items = Vec::new();
            for (i, p) in d.paras.iter().enumerate() {
                let hay = fold(&p.text);
                let needle = fold(&text);
                for (at, _) in hay.match_indices(&needle) {
                    let r = Rng { p0: i, from: at, p1: i, to: at + needle.len() };
                    items.push(json!({"id": r.id(), "text": d.text_of(r)}));
                }
            }
            Ok(json!({"items": items}))
        }
        (T::Doc, "getSelectedData", _) => Ok(json!(d.text_of(d.sel))),
        (T::Doc, "setSelectedData", _) => {
            let sel = d.sel;
            d.sel = d.replace(sel, &text)?;
            Ok(Json::Null)
        }
        (T::Doc, "save", _) => Ok(Json::Null),
        (T::Settings, "save", _) => {
            d.settings = args.first().cloned().unwrap_or(json!({}));
            Ok(Json::Null)
        }
        _ => Err(("InvalidArgument", format!("no method {name} on {t:?}"))),
    }
}

/// `office.sync`: every action in order, all or nothing.
fn office_sync(request: &str) -> String {
    let req: Json = serde_json::from_str(request).unwrap();
    let actions = req["actions"].as_array().cloned().unwrap_or_default();
    LOG.with(|l| l.borrow_mut().push(actions.clone()));
    let mut out = DOC.with(|cell| {
        let mut doc = cell.borrow().clone();
        let mut results: Vec<Json> = Vec::new();
        for (i, a) in actions.iter().enumerate() {
            let path = a["path"].as_array().cloned().unwrap_or_default();
            let r = resolve(&doc, &path, &results).and_then(|t| match a["op"].as_str() {
                Some("load") => {
                    let props: Vec<String> = a["props"].as_array().into_iter().flatten().filter_map(|p| p.as_str().map(String::from)).collect();
                    load(&doc, t, &props)
                }
                Some("set") => set(&mut doc, t, a["prop"].as_str().unwrap_or_default(), a["value"].clone()).map(|_| Json::Null),
                Some("call") => call(&mut doc, t, a["method"].as_str().unwrap_or_default(), a["args"].as_array().map(Vec::as_slice).unwrap_or(&[])),
                _ => Err(("InvalidArgument", "bad op".into())),
            });
            match r {
                Ok(v) => results.push(v),
                Err((code, message)) => return json!({"error": {"code": code, "message": message, "action": i}}),
            }
        }
        *cell.borrow_mut() = doc;
        json!({"results": results})
    });
    out["id"] = req["id"].clone();
    out.to_string()
}

/// A fresh document and the library installed with the fake host as its transport.
fn setup() {
    DOC.with(|d| *d.borrow_mut() = Doc::new());
    LOG.with(|l| l.borrow_mut().clear());
    install_with_transport(|r| office_sync(&r));
}

fn doc<R>(f: impl FnOnce(&Doc) -> R) -> R {
    DOC.with(|d| f(&d.borrow()))
}

/// The syncs sent so far, each a list of actions, without the settings' (the library's own).
fn syncs() -> Vec<Vec<Json>> {
    LOG.with(|l| l.borrow().iter().filter(|s| s.first().is_none_or(|a| a["path"] != json!([{"p": "settings"}]))).cloned().collect())
}

// ---------------------------------------------------------------- helpers

fn global(name: &str) -> JsValue {
    Reflect::get(&js_sys::global(), &name.into()).unwrap()
}

fn g(o: &JsValue, k: &str) -> JsValue {
    Reflect::get(o, &k.into()).unwrap()
}

fn path(o: &JsValue, keys: &[&str]) -> JsValue {
    keys.iter().fold(o.clone(), |o, k| g(&o, k))
}

fn set_js(o: &JsValue, k: &str, v: Json) {
    assert!(Reflect::set(o, &k.into(), &to_js(&v)).unwrap(), "{k} is settable");
}

fn call_js(o: &JsValue, name: &str, args: &[JsValue]) -> JsValue {
    try_call(o, name, args).unwrap()
}

fn try_call(o: &JsValue, name: &str, args: &[JsValue]) -> Result<JsValue, JsValue> {
    let f: Function = g(o, name).dyn_into().unwrap_or_else(|_| panic!("{name} is a function"));
    f.apply(o, &args.iter().cloned().collect::<Array>())
}

fn s(v: &str) -> JsValue {
    v.into()
}

fn code(e: &JsValue) -> String {
    g(e, "code").as_string().unwrap_or_default()
}

fn word(keys: &[&str]) -> JsValue {
    path(&global("Word"), keys)
}

/// `Word.run(batch)` with an async Rust batch; resolves with what the batch returns.
async fn word_run<F>(batch: impl FnOnce(JsValue) -> F + 'static) -> Result<JsValue, JsValue>
where
    F: std::future::Future<Output = Result<JsValue, JsValue>> + 'static,
{
    let batch = Closure::once_into_js(move |ctx: JsValue| -> Promise { future_to_promise(batch(ctx)) });
    let run = g(&global("Word"), "run");
    let p: Promise = run.dyn_into::<Function>().unwrap().call1(&JsValue::UNDEFINED, &batch).unwrap().dyn_into().unwrap();
    JsFuture::from(p).await
}

async fn sync(ctx: &JsValue) -> Result<JsValue, JsValue> {
    JsFuture::from(call_js(ctx, "sync", &[]).dyn_into::<Promise>().unwrap()).await
}

fn body(ctx: &JsValue) -> JsValue {
    path(ctx, &["document", "body"])
}

// ---------------------------------------------------------------- tests

#[wasm_bindgen_test]
async fn a_paragraph_is_inserted_at_the_end_styled_and_bolded() {
    setup();
    let out = word_run(|ctx| async move {
        let end = word(&["InsertLocation", "end"]);
        assert_eq!(end.as_string().as_deref(), Some("End"));
        let p = call_js(&body(&ctx), "insertParagraph", &[s("Closing words"), end]);
        set_js(&p, "styleBuiltIn", from_js(&word(&["Style", "heading2"])));
        set_js(&g(&p, "font"), "bold", json!(true));
        set_js(&p, "alignment", from_js(&word(&["Alignment", "centered"])));
        call_js(&p, "load", &[s("text, style, spaceAfter")]);
        // Not loaded until the sync: reading it throws PropertyNotLoaded.
        assert_eq!(code(&Reflect::get(&p, &"text".into()).unwrap_err()), "PropertyNotLoaded");
        assert_eq!(from_js(&g(&p, "styleBuiltIn")), json!("Heading2"), "a value just set is known at once");
        sync(&ctx).await?;
        let first = json!({"text": from_js(&g(&p, "text")), "style": from_js(&g(&p, "style")), "spaceAfter": from_js(&g(&p, "spaceAfter"))});
        // The same proxy, in a later sync, is named by the id the host answered.
        let r = call_js(&p, "insertText", &[s("."), s("End")]);
        set_js(&g(&r, "font"), "italic", json!(true));
        call_js(&p, "load", &[s("font")]);
        sync(&ctx).await?;
        Ok(to_js(&json!({"first": first, "json": from_js(&call_js(&p, "toJSON", &[])), "font": from_js(&call_js(&g(&p, "font"), "toJSON", &[]))})))
    })
    .await
    .unwrap();
    let out = from_js(&out);
    assert_eq!(out["first"], json!({"text": "Closing words", "style": "Heading2", "spaceAfter": 8}));
    assert_eq!(out["json"]["styleBuiltIn"], "Heading2");
    assert!(out["json"].get("id").is_none(), "the host's ids aren't Word API properties");
    assert_eq!((out["font"]["bold"].as_bool(), out["font"]["italic"].as_bool()), (Some(true), Some(true)));
    assert_eq!(
        doc(|d| d.paras.iter().map(|p| p.text.clone()).collect::<Vec<_>>()),
        ["Hello world", "The quick brown fox jumps over the lazy dog. A fox again.", "Closing words."]
    );
    assert_eq!(doc(|d| (d.paras[2].props["alignment"].clone(), d.paras[2].font["bold"].clone())), (json!("Centered"), json!(true)));
    let wire = syncs();
    assert_eq!(
        wire[0][0],
        json!({"op": "call", "path": [{"p": "document"}, {"p": "body"}], "method": "insertParagraph", "args": ["Closing words", "End"]})
    );
    assert_eq!(
        wire[0][1],
        json!({"op": "set", "path": [{"p": "document"}, {"p": "paragraphs"}, {"m": "getById", "a": [{"result": 0}]}], "prop": "styleBuiltIn", "value": "Heading2"})
    );
    assert_eq!(wire[0][2]["path"][2], json!({"m": "getById", "a": [{"result": 0}]}));
    assert_eq!(wire[0][2]["path"][3], json!({"p": "font"}));
    assert_eq!(wire[1][0]["path"], json!([{"p": "document"}, {"p": "paragraphs"}, {"m": "getById", "a": ["p:2"]}]));
    assert_eq!(wire[1][1]["path"], json!([{"p": "document"}, {"m": "getRangeById", "a": [{"result": 0}]}, {"p": "font"}]));
    assert_eq!(wire[1][2]["props"], json!(["bold", "italic", "underline", "size", "name", "color", "highlightColor"]), "load('font') loads the font");
}

#[wasm_bindgen_test]
async fn body_paragraphs_load_with_their_text() {
    setup();
    let out = word_run(|ctx| async move {
        let paras = g(&body(&ctx), "paragraphs");
        call_js(&paras, "load", &[s("items/text, style")]);
        assert_eq!(code(&Reflect::get(&paras, &"items".into()).unwrap_err()), "PropertyNotLoaded");
        sync(&ctx).await?;
        let items: Array = g(&paras, "items").dyn_into()?;
        let texts: Vec<Json> = items.iter().map(|i| from_js(&g(&i, "text"))).collect();
        set_js(&g(&items.get(1), "font"), "underline", from_js(&word(&["UnderlineType", "double"])));
        // Navigation: the last paragraph, a paragraph after it, the one before the last.
        let last = call_js(&paras, "getLast", &[]);
        let added = call_js(&last, "insertParagraph", &[s("After last"), s("After")]);
        let prev = call_js(&last, "getPrevious", &[]);
        call_js(&prev, "load", &[s("text, isListItem")]);
        call_js(&added, "load", &[s("text")]);
        let json = from_js(&call_js(&paras, "toJSON", &[]));
        sync(&ctx).await?;
        Ok(to_js(&json!({
            "texts": texts, "json": json, "count": items.length(),
            "prev": from_js(&call_js(&prev, "toJSON", &[])), "added": from_js(&g(&added, "text")),
        })))
    })
    .await
    .unwrap();
    let out = from_js(&out);
    assert_eq!(out["texts"], json!(["Hello world", "The quick brown fox jumps over the lazy dog. A fox again."]));
    assert_eq!(out["count"], 2, "Word has no getCount: items.length is the count");
    assert_eq!(out["json"]["items"][0], json!({"text": "Hello world", "style": "Normal"}));
    assert_eq!(out["prev"], json!({"text": "Hello world", "isListItem": false}));
    assert_eq!(out["added"], "After last");
    assert_eq!(doc(|d| d.paras[1].font["underline"].clone()), json!("Double"));
    let wire = syncs();
    assert_eq!(
        wire[0],
        vec![json!({"op": "load", "path": [{"p": "document"}, {"p": "body"}, {"p": "paragraphs"}], "props": ["items", "text", "style"]})]
    );
    assert_eq!(wire[1][0]["path"], json!([{"p": "document"}, {"p": "paragraphs"}, {"m": "getById", "a": ["p:1"]}, {"p": "font"}]));
    // `getLast()` is pinned by its first use (a load of its id), then named by that result.
    assert_eq!(
        wire[1][1],
        json!({"op": "load", "path": [{"p": "document"}, {"p": "body"}, {"p": "paragraphs"}, {"m": "getLast", "a": []}], "props": ["id"]})
    );
    assert_eq!(wire[1][2]["path"], json!([{"p": "document"}, {"p": "paragraphs"}, {"m": "getById", "a": [{"result": 1}]}]));
    assert_eq!(wire[1][2]["method"], "insertParagraph");
    assert_eq!(wire[1][3]["path"][2], json!({"m": "getById", "a": [{"result": 1}]}));
    assert_eq!(wire[1][3]["path"][3], json!({"m": "getPrevious", "a": []}));
    assert_eq!(wire[1][5]["path"][2], json!({"m": "getById", "a": [{"result": 2}]}));
}

#[wasm_bindgen_test]
async fn text_is_inserted_into_the_selection() {
    setup();
    let out = word_run(|ctx| async move {
        let sel = call_js(&g(&ctx, "document"), "getSelection", &[]);
        call_js(&sel, "load", &[s("text, isEmpty")]);
        sync(&ctx).await?;
        let before = json!({"text": from_js(&g(&sel, "text")), "isEmpty": from_js(&g(&sel, "isEmpty"))});
        let r = call_js(&sel, "insertText", &[s("Howdy"), s("Replace")]);
        set_js(&g(&r, "font"), "highlightColor", json!("Yellow"));
        call_js(&r, "select", &[word(&["SelectionMode", "end"])]);
        let caret = call_js(&g(&ctx, "document"), "getSelection", &[]);
        call_js(&caret, "load", &[s("isEmpty")]);
        call_js(&r, "load", &[s("text")]);
        let bad = try_call(&sel, "insertText", &[s("x")]).unwrap_err();
        assert_eq!(code(&bad), "InvalidArgument", "a location is required");
        sync(&ctx).await?;
        Ok(to_js(&json!({"before": before, "inserted": from_js(&g(&r, "text")), "caretEmpty": from_js(&g(&caret, "isEmpty"))})))
    })
    .await
    .unwrap();
    let out = from_js(&out);
    assert_eq!(out["before"], json!({"text": "Hello", "isEmpty": false}));
    assert_eq!((out["inserted"].as_str(), out["caretEmpty"].as_bool()), (Some("Howdy"), Some(true)));
    assert_eq!(
        doc(|d| (d.paras[0].text.clone(), d.sel, d.paras[0].font["highlightColor"].clone())),
        ("Howdy world".to_string(), Rng::caret(0, 5), json!("Yellow"))
    );
    let wire = syncs();
    assert_eq!(wire[0][0], json!({"op": "load", "path": [{"p": "document"}, {"m": "getSelection", "a": []}], "props": ["id"]}));
    assert_eq!(wire[0][1]["path"], json!([{"p": "document"}, {"m": "getRangeById", "a": [{"result": 0}]}]));
    assert_eq!(wire[1][0]["path"], json!([{"p": "document"}, {"m": "getRangeById", "a": ["r:p:0:0-p:0:5"]}]));
    assert_eq!(wire[1][0]["args"], json!(["Howdy", "Replace"]));
    assert_eq!(wire[1][1]["path"], json!([{"p": "document"}, {"m": "getRangeById", "a": [{"result": 0}]}, {"p": "font"}]));
    assert_eq!(
        wire[1][2],
        json!({"op": "call", "path": [{"p": "document"}, {"m": "getRangeById", "a": [{"result": 0}]}], "method": "select", "args": ["End"]})
    );
    // The second `getSelection()` is a new object, pinned after the select moved the selection.
    assert_eq!(wire[1][3]["props"], json!(["id"]));
    assert_eq!(wire[1][4]["path"][1], json!({"m": "getRangeById", "a": [{"result": 3}]}));
}

#[wasm_bindgen_test]
async fn search_finds_ranges_to_format() {
    setup();
    let out = word_run(|ctx| async move {
        let found = call_js(&body(&ctx), "search", &[s("fox"), to_js(&json!({"matchCase": true, "matchPrefix": true}))]);
        call_js(&found, "load", &[s("items")]);
        let early = try_call(&found, "load", &[s("items/style")]).unwrap_err();
        assert_eq!(code(&early), "NotImplemented");
        sync(&ctx).await?;
        let items: Array = g(&found, "items").dyn_into()?;
        let texts: Vec<Json> = items.iter().map(|i| from_js(&g(&i, "text"))).collect();
        for i in items.iter() {
            set_js(&g(&i, "font"), "highlightColor", json!("#FFFF00"));
        }
        call_js(&found, "load", &[s("style")]);
        let none = call_js(&body(&ctx), "search", &[s("FOX"), to_js(&json!({"matchCase": true}))]);
        call_js(&none, "load", &[s("items/text")]);
        sync(&ctx).await?;
        let none_items: Array = g(&none, "items").dyn_into()?;
        Ok(to_js(&json!({"texts": texts, "json": from_js(&call_js(&found, "toJSON", &[])), "none": none_items.length()})))
    })
    .await
    .unwrap();
    let out = from_js(&out);
    assert_eq!(out["texts"], json!(["fox", "fox"]));
    assert_eq!(out["json"], json!({"items": [{"text": "fox", "style": "Normal"}, {"text": "fox", "style": "Normal"}]}));
    assert_eq!(out["none"], 0);
    let wire = syncs();
    assert_eq!(
        wire[0],
        vec![json!({"op": "call", "path": [{"p": "document"}, {"p": "body"}], "method": "search", "args": ["fox", {"matchCase": true}]})]
    );
    assert_eq!(wire[1][0]["path"], json!([{"p": "document"}, {"m": "getRangeById", "a": ["r:p:1:16-p:1:19"]}, {"p": "font"}]));
    assert_eq!(wire[1][2], json!({"op": "load", "path": [{"p": "document"}, {"m": "getRangeById", "a": ["r:p:1:16-p:1:19"]}], "props": ["style"]}));
    assert_eq!(doc(|d| d.paras[1].font["highlightColor"].clone()), json!("#FFFF00"));
}

#[wasm_bindgen_test]
async fn fonts_read_null_where_the_text_is_mixed() {
    setup();
    DOC.with(|d| d.borrow_mut().paras[1].font.insert("bold".into(), json!(true)));
    let out = word_run(|ctx| async move {
        let font = g(&body(&ctx), "font");
        call_js(&font, "load", &[]);
        let whole = call_js(&body(&ctx), "getRange", &[]);
        call_js(&whole, "load", &[s("font/bold, font/size, text")]);
        let first = call_js(&g(&body(&ctx), "paragraphs"), "getFirst", &[]);
        call_js(&first, "load", &[s("font/bold")]);
        sync(&ctx).await?;
        Ok(to_js(&json!({
            "body": from_js(&call_js(&font, "toJSON", &[])),
            "wholeBold": from_js(&path(&whole, &["font", "bold"])), "wholeSize": from_js(&path(&whole, &["font", "size"])),
            "firstBold": from_js(&path(&first, &["font", "bold"])), "wholeText": from_js(&g(&whole, "text")),
        })))
    })
    .await
    .unwrap();
    let out = from_js(&out);
    assert_eq!(
        out["body"],
        json!({"bold": null, "italic": false, "underline": "None", "size": 11, "name": "Calibri", "color": "#000000", "highlightColor": null})
    );
    assert_eq!((out["wholeBold"].clone(), out["wholeSize"].clone(), out["firstBold"].clone()), (Json::Null, json!(11), json!(false)));
    assert_eq!(out["wholeText"], "Hello world\rThe quick brown fox jumps over the lazy dog. A fox again.");
    assert_eq!(syncs()[0][1]["path"], json!([{"p": "document"}, {"p": "body"}, {"m": "getRange", "a": ["Whole"]}]), "getRange() is Whole");
}

#[wasm_bindgen_test]
async fn a_failing_sync_rejects_with_the_code_and_changes_nothing() {
    setup();
    let e = word_run(|ctx| async move {
        let p = call_js(&body(&ctx), "insertParagraph", &[s("Doomed"), s("End")]);
        set_js(&p, "style", json!("Quote"));
        // The paragraph after the last one: ItemNotFound, so the insert is taken back too.
        let after = call_js(&call_js(&g(&body(&ctx), "paragraphs"), "getLast", &[]), "getNext", &[]);
        call_js(&after, "insertText", &[s("x"), s("Start")]);
        sync(&ctx).await
    })
    .await
    .unwrap_err();
    assert_eq!(code(&e), "ItemNotFound");
    assert_eq!(g(&e, "name").as_string().as_deref(), Some("RichApi.Error"));
    assert!(e.is_instance_of::<js_sys::Error>(), "OfficeExtension.Error is an Error");
    assert_eq!(from_js(&g(&e, "debugInfo"))["errorLocation"], 3);
    assert_eq!(doc(|d| d.paras.len()), 2, "the insert in the same sync was taken back");
    // An object reached but never used sends nothing (Word sends only what actions use).
    let r = word_run(|ctx| async move {
        let _unused = call_js(&call_js(&g(&body(&ctx), "paragraphs"), "getLast", &[]), "getNext", &[]);
        call_js(&body(&ctx), "insertParagraph", &[s("Kept"), s("Start")]);
        Ok(JsValue::from(1))
    })
    .await
    .unwrap();
    assert_eq!(r.as_f64(), Some(1.0), "Word.run resolves with the batch's value, after sending what was queued");
    assert_eq!(doc(|d| d.paras[0].text.clone()), "Kept");
    // A paragraph created in a failed sync names nothing later.
    let e = word_run(|ctx| async move {
        let p = call_js(&body(&ctx), "insertParagraph", &[s("Gone"), s("Middle")]);
        let _ = sync(&ctx).await;
        call_js(&p, "delete", &[]);
        sync(&ctx).await
    })
    .await
    .unwrap_err();
    assert_eq!(code(&e), "ItemNotFound");
    assert_eq!(syncs().last().unwrap()[0]["path"][2], json!({"m": "getById", "a": [""]}));
    let not_a_batch = call_js(&global("Word"), "run", &[JsValue::from(3)]).dyn_into::<Promise>().unwrap();
    assert_eq!(code(&JsFuture::from(not_a_batch).await.unwrap_err()), "InvalidArgument");
}

#[wasm_bindgen_test]
async fn document_properties_and_the_common_api() {
    setup();
    let out = word_run(|ctx| async move {
        let props = path(&ctx, &["document", "properties"]);
        call_js(&props, "load", &[]);
        sync(&ctx).await?;
        let before = from_js(&call_js(&props, "toJSON", &[]));
        set_js(&props, "title", json!("Final"));
        set_js(&props, "keywords", json!("report, q3"));
        call_js(&g(&ctx, "document"), "save", &[]);
        sync(&ctx).await?;
        Ok(to_js(&before))
    })
    .await
    .unwrap();
    assert_eq!(from_js(&out), json!({"title": "Draft", "subject": "", "author": "Ada", "keywords": "", "comments": ""}));
    assert_eq!(doc(|d| (d.props["title"].clone(), d.props["keywords"].clone())), (json!("Final"), json!("report, q3")));
    // The common API reads and writes the selected text, on the `document` root.
    let text = document_async("getSelectedDataAsync", s("text")).await;
    assert_eq!(g(&text, "status").as_string().as_deref(), Some("succeeded"));
    assert_eq!(g(&text, "value").as_string().as_deref(), Some("Hello"));
    let set_r = document_async("setSelectedDataAsync", s("Goodbye")).await;
    assert_eq!(g(&set_r, "status").as_string().as_deref(), Some("succeeded"));
    assert_eq!(doc(|d| d.paras[0].text.clone()), "Goodbye world");
    let html = document_async("getSelectedDataAsync", s("html")).await;
    assert_eq!(path(&html, &["error", "code"]).as_string().as_deref(), Some("NotImplemented"));
    let wire = syncs();
    assert_eq!(wire[wire.len() - 1][0], json!({"op": "call", "path": [{"p": "document"}], "method": "setSelectedData", "args": ["Goodbye"]}));
    assert_eq!(wire[wire.len() - 2][0], json!({"op": "call", "path": [{"p": "document"}], "method": "getSelectedData", "args": ["text"]}));
}

/// Calls `Office.context.document.<name>(first, callback)` and waits for the callback's result.
async fn document_async(name: &str, first: JsValue) -> JsValue {
    let doc = path(&global("Office"), &["context", "document"]);
    let name = name.to_string();
    let p = Promise::new(&mut |resolve, _| {
        let _ = call_js(&doc, &name, &[first.clone(), resolve.into()]);
    });
    JsFuture::from(p).await.unwrap()
}

async fn callback_only(o: &JsValue, name: &str) -> JsValue {
    let o = o.clone();
    let name = name.to_string();
    let p = Promise::new(&mut |resolve, _| {
        let _ = call_js(&o, &name, &[resolve.into()]);
    });
    JsFuture::from(p).await.unwrap()
}

#[wasm_bindgen_test]
async fn settings_requirements_and_the_globals() {
    setup();
    let office = global("Office");
    let info = JsFuture::from(call_js(&office, "onReady", &[]).dyn_into::<Promise>().unwrap()).await.unwrap();
    assert_eq!(g(&info, "host").as_string().as_deref(), Some("Word"));
    assert_eq!(path(&office, &["HostType", "Word"]).as_string().as_deref(), Some("Word"));
    assert_eq!(path(&office, &["context", "host"]).as_string().as_deref(), Some("Word"));
    let settings = path(&office, &["context", "document", "settings"]);
    call_js(&settings, "set", &[s("theme"), s("dark")]);
    assert_eq!(call_js(&settings, "get", &[s("theme")]).as_string().as_deref(), Some("dark"));
    let saved = callback_only(&settings, "saveAsync").await;
    assert_eq!(g(&saved, "status").as_string().as_deref(), Some("succeeded"));
    // Installed again (the pane reloaded), the settings come back from the host.
    install_with_transport(|r| office_sync(&r));
    let _ = JsFuture::from(call_js(&office, "onReady", &[]).dyn_into::<Promise>().unwrap()).await.unwrap();
    assert_eq!(call_js(&settings, "get", &[s("theme")]).as_string().as_deref(), Some("dark"));
    assert!(call_js(&settings, "get", &[s("missing")]).is_null());
    let req = path(&office, &["context", "requirements"]);
    let supported = |name: &str, v: &str| call_js(&req, "isSetSupported", &[s(name), s(v)]).as_bool();
    assert_eq!(supported("Settings", "1.1"), Some(true));
    assert_eq!(supported("TextCoercion", "1.1"), Some(true));
    assert_eq!(supported("WordApi", "1.3"), Some(true), "reported as the other craft apps report theirs");
    assert_eq!(supported("WordApi", "1.4"), Some(false));
    assert_eq!(supported("PowerPointApi", "1.1"), Some(false));
    // The globals can't be replaced or deleted by a later script (Microsoft's office.js, say).
    for name in ["Word", "Office", "OfficeExtension"] {
        let _ = Reflect::set(&js_sys::global(), &name.into(), &JsValue::from(1));
        let _ = Reflect::delete_property(&js_sys::global(), &name.into());
        assert!(global(name).is_object(), "{name} stays");
    }
    assert!(global("Excel").is_undefined() && global("PowerPoint").is_undefined());
    assert_eq!(word(&["ErrorCodes", "itemNotFound"]).as_string().as_deref(), Some("ItemNotFound"));
    assert_eq!(word(&["RangeLocation", "content"]).as_string().as_deref(), Some("Content"));
    assert_eq!(word(&["BuiltInStyleName", "gridTable4_Accent1"]).as_string().as_deref(), Some("GridTable4_Accent1"));
    assert!(word(&["Style", "toc9"]).is_string() && word(&["UnderlineType", "waveDouble"]).is_string());
    // `new Word.RequestContext()` works as the old samples use it.
    let ctor: Function = word(&["RequestContext"]).dyn_into().unwrap();
    let ctx = Reflect::construct(&ctor, &Array::new()).unwrap();
    let paras = path(&ctx, &["document", "body", "paragraphs"]);
    call_js(&paras, "load", &[s("text")]);
    sync(&ctx).await.unwrap();
    assert_eq!(Array::from(&g(&paras, "items")).length(), 2);
    assert!(g(&paras, "context").is_object());
}

#[wasm_bindgen_test]
async fn the_page_transport_posts_and_takes_replies() {
    DOC.with(|d| *d.borrow_mut() = Doc::new());
    // What the desktop web view provides: `__craftOffice.post`, answered later through
    // `__craftOfficeReply`.
    let post = Closure::<dyn Fn(String)>::new(|request: String| {
        spawn_local(async move {
            let answer = office_sync(&request);
            let reply: Function = global("__craftOfficeReply").dyn_into().unwrap();
            reply.call1(&JsValue::UNDEFINED, &answer.into()).unwrap();
        });
    });
    let host = js_sys::Object::new();
    Reflect::set(&host, &"post".into(), &post.into_js_value()).unwrap();
    Reflect::set(&js_sys::global(), &"__craftOffice".into(), &host).unwrap();
    install();
    let text = word_run(|ctx| async move {
        let b = body(&ctx);
        call_js(&b, "insertParagraph", &[s("Posted"), s("Start")]);
        call_js(&b, "load", &[s("text")]);
        sync(&ctx).await?;
        Ok(g(&b, "text"))
    })
    .await
    .unwrap();
    assert_eq!(text.as_string().as_deref(), Some("Posted\nHello world\nThe quick brown fox jumps over the lazy dog. A fox again."));
    let _ = Reflect::delete_property(&js_sys::global(), &"__craftOffice".into());
}
