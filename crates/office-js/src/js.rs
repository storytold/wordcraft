//! The JavaScript side: the `Office`, `Word` and `OfficeExtension` globals, the proxy classes,
//! and the transport to the host.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use js_sys::{Array, Function, Object, Promise, Reflect};
use serde_json::{Map, Value as Json, json};
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::{JsFuture, future_to_promise, spawn_local};

use crate::core::{Ctx, Named, OfficeError, Path, as_item_props, child, enum_value, id_string, is_set_supported, load_props, method, prop};

/// A synchronous transport (tests answer in-process).
type SyncTransport = Rc<dyn Fn(String) -> String>;

thread_local! {
    static TRANSPORT: RefCell<Option<SyncTransport>> = const { RefCell::new(None) };
    static WAITING: RefCell<HashMap<u64, Function>> = RefCell::new(HashMap::new());
    static NEXT_ID: Cell<u64> = const { Cell::new(1) };
    static SETTINGS: RefCell<Map<String, Json>> = RefCell::new(Map::new());
    static INSTALLED: Cell<bool> = const { Cell::new(false) };
    static READY: RefCell<Option<Promise>> = const { RefCell::new(None) };
}

// ---------------------------------------------------------------- values and errors

pub(crate) fn to_js(v: &Json) -> JsValue {
    match v {
        Json::Null => JsValue::NULL,
        _ => js_sys::JSON::parse(&v.to_string()).unwrap_or(JsValue::NULL),
    }
}

pub(crate) fn from_js(v: &JsValue) -> Json {
    if v.is_undefined() || v.is_null() {
        return Json::Null;
    }
    js_sys::JSON::stringify(v).ok().and_then(|s| s.as_string()).and_then(|s| serde_json::from_str(&s).ok()).unwrap_or(Json::Null)
}

/// An `OfficeExtension.Error`: an `Error` named `RichApi.Error` with `code` and `debugInfo`.
fn js_error(e: &OfficeError) -> JsValue {
    let err = js_sys::Error::new(&e.message);
    err.set_name("RichApi.Error");
    let _ = Reflect::set(&err, &"code".into(), &e.code.as_str().into());
    let debug = to_js(&json!({"code": e.code, "message": e.message, "errorLocation": e.action}));
    let _ = Reflect::set(&err, &"debugInfo".into(), &debug);
    err.into()
}

fn not_implemented(what: &str) -> JsValue {
    js_error(&OfficeError::not_implemented(what))
}

// ---------------------------------------------------------------- the transport

/// Sends one batch of actions and waits for the host's answer (`{"results"}` or `{"error"}`).
async fn send(actions: Vec<Json>) -> Result<Json, OfficeError> {
    let id = NEXT_ID.with(|n| {
        let id = n.get();
        n.set(id.wrapping_add(1));
        id
    });
    let request = json!({"id": id, "actions": actions}).to_string();
    if let Some(t) = TRANSPORT.with(|t| t.borrow().clone()) {
        return serde_json::from_str(&t(request)).map_err(|e| OfficeError::new("GeneralException", format!("the host's answer isn't JSON: {e}")));
    }
    let global = js_sys::global();
    let host = Reflect::get(&global, &"__craftOffice".into()).unwrap_or(JsValue::UNDEFINED);
    let post = Reflect::get(&host, &"post".into()).ok().and_then(|f| f.dyn_into::<Function>().ok());
    let Some(post) = post else { return Err(OfficeError::new("GeneralException", "this page isn't running in WordCraft")) };
    let answer = Promise::new(&mut |resolve, _reject| {
        WAITING.with(|w| w.borrow_mut().insert(id, resolve));
    });
    if post.call1(&host, &request.into()).is_err() {
        WAITING.with(|w| w.borrow_mut().remove(&id));
        return Err(OfficeError::new("GeneralException", "WordCraft didn't take the request"));
    }
    let reply = JsFuture::from(answer).await.map_err(|_| OfficeError::new("GeneralException", "the request was dropped"))?;
    let text = reply.as_string().unwrap_or_default();
    serde_json::from_str(&text).map_err(|e| OfficeError::new("GeneralException", format!("the host's answer isn't JSON: {e}")))
}

/// `globalThis.__craftOfficeReply(json)`: the host's answer to a request, by its `id`.
fn reply(text: String) {
    let id = serde_json::from_str::<Json>(&text).ok().and_then(|v| v.get("id").and_then(Json::as_u64));
    if let Some(id) = id
        && let Some(resolve) = WAITING.with(|w| w.borrow_mut().remove(&id))
    {
        let _ = resolve.call1(&JsValue::UNDEFINED, &text.into());
    }
}

// ---------------------------------------------------------------- object kinds

/// What the library knows of a class: the properties `load()` with no argument asks for, the
/// objects it leads to by property (`load("font")` loads the font's properties), and
/// (collections) the kind of the items and how the host names them. A listed collection
/// (`search`'s answer) has no path the host knows: its items come with the call's answer.
struct Kind {
    defaults: &'static [&'static str],
    children: &'static [(&'static str, &'static Kind)],
    item: Option<(&'static Kind, Named)>,
    listed: bool,
}

impl Kind {
    const fn object(defaults: &'static [&'static str], children: &'static [(&'static str, &'static Kind)]) -> Kind {
        Kind { defaults, children, item: None, listed: false }
    }
    fn child(&self, name: &str) -> Option<&'static Kind> {
        self.children.iter().find(|(n, _)| *n == name).map(|(_, k)| *k)
    }
    /// `props` with each one naming an object (`font`, `font` of `items`) replaced by that
    /// object's default properties (`font/bold`, `font/italic`, …).
    fn expand(&'static self, props: Vec<String>) -> Vec<String> {
        let mut out = Vec::new();
        for p in props {
            let mut kind = self;
            let mut leads_to_object = true;
            for part in p.split('/').map(str::trim) {
                match kind.child(part) {
                    Some(k) => kind = k,
                    None => {
                        leads_to_object = false;
                        break;
                    }
                }
            }
            if leads_to_object && !std::ptr::eq(kind, self) {
                out.extend(kind.defaults.iter().map(|d| format!("{p}/{d}")));
            } else {
                out.push(p);
            }
        }
        out
    }
}

static DOCUMENT: Kind = Kind::object(&[], &[]);
static PROPERTIES: Kind = Kind::object(&["title", "subject", "author", "keywords", "comments"], &[]);
static FONT: Kind = Kind::object(&["bold", "italic", "underline", "size", "name", "color", "highlightColor"], &[]);
static BODY: Kind = Kind::object(&["text", "style"], &[("font", &FONT)]);
static PARAGRAPH: Kind = Kind::object(
    &[
        "text",
        "style",
        "styleBuiltIn",
        "alignment",
        "firstLineIndent",
        "leftIndent",
        "rightIndent",
        "lineSpacing",
        "spaceAfter",
        "spaceBefore",
        "isListItem",
    ],
    &[("font", &FONT)],
);
static PARAGRAPHS: Kind = Kind {
    defaults: &[
        "items/text",
        "items/style",
        "items/styleBuiltIn",
        "items/alignment",
        "items/firstLineIndent",
        "items/leftIndent",
        "items/rightIndent",
        "items/lineSpacing",
        "items/spaceAfter",
        "items/spaceBefore",
        "items/isListItem",
    ],
    children: &[],
    item: Some((&PARAGRAPH, Named::Paragraph)),
    listed: false,
};
static RANGE: Kind = Kind::object(&["text", "isEmpty", "style"], &[("font", &FONT)]);
static FOUND: Kind = Kind { defaults: &["items/text"], children: &[], item: Some((&RANGE, Named::Range)), listed: true };

// ---------------------------------------------------------------- proxies

/// A context, a path and a kind: what every proxy object is.
#[derive(Clone)]
struct P {
    ctx: Rc<RefCell<Ctx>>,
    path: Path,
    kind: &'static Kind,
}

impl P {
    fn to(&self, step: Json, kind: &'static Kind) -> P {
        P { ctx: self.ctx.clone(), path: child(&self.path, step), kind }
    }
    fn at(&self, path: Path, kind: &'static Kind) -> P {
        P { ctx: self.ctx.clone(), path, kind }
    }
    fn get(&self, name: &str) -> Result<JsValue, JsValue> {
        self.ctx.borrow().get(&self.path, name).map(to_js).ok_or_else(|| js_error(&OfficeError::not_loaded(name)))
    }
    fn set(&self, name: &str, v: &JsValue) {
        self.ctx.borrow_mut().set(&self.path, name, from_js(v));
    }
    fn load(&self, arg: &JsValue) -> Result<(), JsValue> {
        let props: Vec<String> =
            if arg.is_undefined() || arg.is_null() { self.kind.defaults.iter().map(|s| s.to_string()).collect() } else { load_props(&from_js(arg)) };
        let Some((item_kind, named)) = self.kind.item else {
            return self.ctx.borrow_mut().load(&self.path, &self.kind.expand(props), None).map_err(|e| js_error(&e));
        };
        let props = as_item_props(&props);
        // `items/font` loads the items' fonts: not one load, so not supported (nor is `items/font/bold`).
        let props: Vec<String> = props
            .iter()
            .map(|p| {
                p.strip_prefix("items/").map(|q| item_kind.expand(vec![q.to_string()])).map(|v| v.iter().map(|q| format!("items/{q}")).collect())
            })
            .flat_map(|v: Option<Vec<String>>| v.unwrap_or_else(|| vec!["items".to_string()]))
            .collect();
        if self.kind.listed {
            return self.load_listed(&props);
        }
        self.ctx.borrow_mut().load(&self.path, &props, Some(named)).map_err(|e| js_error(&e))
    }
    /// `load()` of a listed collection (`search`'s answer): its items come with their `text`;
    /// other properties are loaded item by item once the items are known.
    fn load_listed(&self, props: &[String]) -> Result<(), JsValue> {
        let wanted: Vec<String> = props.iter().filter_map(|p| p.strip_prefix("items/")).map(str::to_string).collect();
        let extra = wanted.iter().any(|p| p != "text");
        if !extra {
            return Ok(());
        }
        let Ok(ids) = self.item_ids() else {
            return Err(not_implemented(
                "loading the found ranges' properties (other than text) in the sync that runs the search: load them after it",
            ));
        };
        let mut ctx = self.ctx.borrow_mut();
        for id in ids {
            ctx.load(&Named::Range.path(json!(id)), &wanted, None).map_err(|e| js_error(&e))?;
        }
        Ok(())
    }
    fn call(&self, name: &str, args: Vec<Json>) {
        self.ctx.borrow_mut().call(&self.path, name, args);
    }
    /// Queues a call that creates an object (`insertParagraph`); the proxy of the new object.
    fn create(&self, name: &str, args: Vec<Json>, named: Named, kind: &'static Kind) -> P {
        let path = self.ctx.borrow_mut().call_creating(&self.path, name, args, named);
        self.at(path, kind)
    }
    /// The object a navigation method (`getNext()`) reaches, pinned by its first use.
    fn nav(&self, step: Json, named: Named, kind: &'static Kind) -> P {
        let path = self.ctx.borrow_mut().navigate(child(&self.path, step), named);
        self.at(path, kind)
    }
    /// The loaded items' ids.
    fn item_ids(&self) -> Result<Vec<String>, JsValue> {
        let items = self.ctx.borrow().get(&self.path, "items").cloned().ok_or_else(|| js_error(&OfficeError::not_loaded("items")))?;
        Ok(items.as_array().into_iter().flatten().filter_map(|i| i.get("id").and_then(id_string)).collect())
    }
    fn items_as<T: From<P> + Into<JsValue>>(&self) -> Result<Array, JsValue> {
        let out = Array::new();
        if let Some((kind, named)) = self.kind.item {
            for id in self.item_ids()? {
                out.push(&T::from(self.at(named.path(json!(id)), kind)).into());
            }
        }
        Ok(out)
    }
    /// `toJSON()`: the properties loaded so far (a collection: `{items: [...]}`), without the
    /// host's ids, which aren't part of Word's API.
    fn to_json(&self) -> JsValue {
        let ctx = self.ctx.borrow();
        let own = |path: &Path| -> Json {
            let props = ctx.object(path).cloned().unwrap_or_default();
            Json::Object(props.into_iter().filter(|(k, _)| k != "id").collect())
        };
        if let Some((_, named)) = self.kind.item {
            let ids: Vec<String> =
                ctx.get(&self.path, "items").and_then(Json::as_array).into_iter().flatten().filter_map(|i| i.get("id").and_then(id_string)).collect();
            let items: Vec<Json> = ids.iter().map(|id| own(&named.path(json!(id)))).collect();
            return to_js(&json!({"items": items}));
        }
        to_js(&own(&self.path))
    }
}

/// A location argument (`"End"`, `Word.InsertLocation.end`), or `default` when it's left out.
fn location(v: &JsValue, default: Option<&str>) -> Result<Json, JsValue> {
    match (v.as_string(), default) {
        (Some(s), _) => Ok(json!(s)),
        (None, Some(d)) if v.is_undefined() || v.is_null() => Ok(json!(d)),
        _ => Err(js_error(&OfficeError::invalid("a location (Word.InsertLocation or Word.RangeLocation) is required"))),
    }
}

/// The fields of an options object a call takes (`SearchOptions`).
fn options(v: &JsValue, keys: &[&str]) -> Json {
    let all = from_js(v);
    let picked: Map<String, Json> = keys.iter().filter_map(|k| all.get(*k).filter(|v| !v.is_null()).map(|v| (k.to_string(), v.clone()))).collect();
    Json::Object(picked)
}

/// Each proxy class: a [`P`], `load()` (returns the object, as Office.js does), `toJSON()`,
/// `track()`/`untrack()` (objects are named by id, so there's nothing to track) and `context`.
macro_rules! class {
    ($t:ident) => {
        #[wasm_bindgen]
        pub struct $t {
            p: P,
        }
        impl From<P> for $t {
            fn from(p: P) -> $t {
                $t { p }
            }
        }
        #[wasm_bindgen]
        impl $t {
            pub fn load(&self, props: JsValue) -> Result<$t, JsValue> {
                self.p.load(&props)?;
                Ok($t { p: self.p.clone() })
            }
            #[wasm_bindgen(js_name = toJSON)]
            pub fn to_json(&self) -> JsValue {
                self.p.to_json()
            }
            pub fn track(&self) -> $t {
                $t { p: self.p.clone() }
            }
            pub fn untrack(&self) -> $t {
                $t { p: self.p.clone() }
            }
            #[wasm_bindgen(getter)]
            pub fn context(&self) -> RequestContext {
                RequestContext { p: P { ctx: self.p.ctx.clone(), path: Vec::new(), kind: &DOCUMENT } }
            }
        }
    };
}

/// A read-only property.
macro_rules! ro {
    ($t:ident, $f:ident, $js:literal) => {
        #[wasm_bindgen]
        impl $t {
            #[wasm_bindgen(getter = $js)]
            pub fn $f(&self) -> Result<JsValue, JsValue> {
                self.p.get($js)
            }
        }
    };
}

/// A property that can be set.
macro_rules! rw {
    ($t:ident, $f:ident, $set:ident, $js:literal) => {
        ro!($t, $f, $js);
        #[wasm_bindgen]
        impl $t {
            #[wasm_bindgen(setter = $js)]
            pub fn $set(&self, v: JsValue) {
                self.p.set($js, &v);
            }
        }
    };
}

/// A collection's `items`, once `load("items")` (or a listing) and a sync have run.
macro_rules! collection {
    ($t:ident, $item:ident) => {
        class!($t);
        #[wasm_bindgen]
        impl $t {
            #[wasm_bindgen(getter)]
            pub fn items(&self) -> Result<Array, JsValue> {
                self.p.items_as::<$item>()
            }
        }
    };
}

/// `Word.RequestContext`: `document`, `sync()`, `trackedObjects`.
#[wasm_bindgen]
#[derive(Clone)]
pub struct RequestContext {
    p: P,
}

#[wasm_bindgen]
impl RequestContext {
    #[wasm_bindgen(constructor)]
    pub fn new() -> RequestContext {
        RequestContext { p: P { ctx: Rc::new(RefCell::new(Ctx::new())), path: Vec::new(), kind: &DOCUMENT } }
    }
    #[wasm_bindgen(getter)]
    pub fn document(&self) -> Document {
        Document { p: self.p.at(vec![prop("document")], &DOCUMENT) }
    }
    /// Sends the queued actions; resolves once loaded properties are filled.
    pub fn sync(&self) -> Promise {
        let ctx = self.p.ctx.clone();
        future_to_promise(async move { sync(ctx).await.map(|_| JsValue::UNDEFINED) })
    }
    /// `context.trackedObjects.add/remove`: objects are named by id, so there's nothing to track.
    #[wasm_bindgen(getter, js_name = trackedObjects)]
    pub fn tracked_objects(&self) -> JsValue {
        let noop = Closure::<dyn Fn(JsValue)>::new(|_| {}).into_js_value();
        obj(&[("add", noop.clone()), ("remove", noop)]).into()
    }
}

impl Default for RequestContext {
    fn default() -> Self {
        RequestContext::new()
    }
}

async fn sync(ctx: Rc<RefCell<Ctx>>) -> Result<(), JsValue> {
    let (actions, pending) = ctx.borrow_mut().take();
    if actions.is_empty() {
        return Ok(());
    }
    let answer = send(actions).await.map_err(|e| js_error(&e))?;
    ctx.borrow_mut().apply(pending, &answer).map(|_| ()).map_err(|e| js_error(&e))
}

// ---------------------------------------------------------------- document

class!(Document);

#[wasm_bindgen]
impl Document {
    #[wasm_bindgen(getter)]
    pub fn body(&self) -> Body {
        Body { p: self.p.to(prop("body"), &BODY) }
    }
    /// `properties` (WordApi 1.3): title, subject, author, keywords, comments.
    #[wasm_bindgen(getter)]
    pub fn properties(&self) -> DocumentProperties {
        DocumentProperties { p: self.p.to(prop("properties"), &PROPERTIES) }
    }
    /// `getSelection()`: the selection (or the caret, an empty range) as it is when first used.
    #[wasm_bindgen(js_name = getSelection)]
    pub fn get_selection(&self) -> Range {
        Range { p: self.p.nav(method("getSelection", vec![]), Named::Range, &RANGE) }
    }
    pub fn save(&self) {
        self.p.call("save", vec![]);
    }
}

class!(DocumentProperties);
rw!(DocumentProperties, title, set_title, "title");
rw!(DocumentProperties, subject, set_subject, "subject");
rw!(DocumentProperties, author, set_author, "author");
rw!(DocumentProperties, keywords, set_keywords, "keywords");
rw!(DocumentProperties, comments, set_comments, "comments");

// ---------------------------------------------------------------- body

class!(Body);
ro!(Body, text, "text");
rw!(Body, style, set_style, "style");

#[wasm_bindgen]
impl Body {
    #[wasm_bindgen(getter)]
    pub fn paragraphs(&self) -> ParagraphCollection {
        ParagraphCollection { p: self.p.to(prop("paragraphs"), &PARAGRAPHS) }
    }
    #[wasm_bindgen(getter)]
    pub fn font(&self) -> Font {
        Font { p: self.p.to(prop("font"), &FONT) }
    }
    /// `getRange(rangeLocation?)`: `Whole` (the default), `Start`, `End`, `Content`.
    #[wasm_bindgen(js_name = getRange)]
    pub fn get_range(&self, location_: JsValue) -> Result<Range, JsValue> {
        Ok(Range { p: self.p.nav(method("getRange", vec![location(&location_, Some("Whole"))?]), Named::Range, &RANGE) })
    }
    /// `insertText(text, insertLocation)`: `Start`, `End`, `Replace`.
    #[wasm_bindgen(js_name = insertText)]
    pub fn insert_text(&self, text: String, location_: JsValue) -> Result<Range, JsValue> {
        Ok(Range { p: self.p.create("insertText", vec![json!(text), location(&location_, None)?], Named::Range, &RANGE) })
    }
    /// `insertParagraph(paragraphText, insertLocation)`: `Start`, `End`.
    #[wasm_bindgen(js_name = insertParagraph)]
    pub fn insert_paragraph(&self, text: String, location_: JsValue) -> Result<Paragraph, JsValue> {
        Ok(Paragraph { p: self.p.create("insertParagraph", vec![json!(text), location(&location_, None)?], Named::Paragraph, &PARAGRAPH) })
    }
    pub fn clear(&self) {
        self.p.call("clear", vec![]);
    }
    /// `search(searchText, searchOptions?)`: `matchCase`, `matchWholeWord`.
    pub fn search(&self, text: String, opts: JsValue) -> RangeCollection {
        let at = self.p.ctx.borrow_mut().call_listing(&self.p.path, "search", vec![json!(text), options(&opts, SEARCH_OPTIONS)], Named::Range);
        RangeCollection { p: self.p.at(at, &FOUND) }
    }
}

const SEARCH_OPTIONS: &[&str] = &["matchCase", "matchWholeWord"];

// ---------------------------------------------------------------- paragraphs

collection!(ParagraphCollection, Paragraph);

#[wasm_bindgen]
impl ParagraphCollection {
    #[wasm_bindgen(js_name = getFirst)]
    pub fn get_first(&self) -> Paragraph {
        Paragraph { p: self.p.nav(method("getFirst", vec![]), Named::Paragraph, &PARAGRAPH) }
    }
    #[wasm_bindgen(js_name = getLast)]
    pub fn get_last(&self) -> Paragraph {
        Paragraph { p: self.p.nav(method("getLast", vec![]), Named::Paragraph, &PARAGRAPH) }
    }
}

class!(Paragraph);
ro!(Paragraph, text, "text");
rw!(Paragraph, style, set_style, "style");
rw!(Paragraph, style_built_in, set_style_built_in, "styleBuiltIn");
rw!(Paragraph, alignment, set_alignment, "alignment");
rw!(Paragraph, first_line_indent, set_first_line_indent, "firstLineIndent");
rw!(Paragraph, left_indent, set_left_indent, "leftIndent");
rw!(Paragraph, right_indent, set_right_indent, "rightIndent");
rw!(Paragraph, line_spacing, set_line_spacing, "lineSpacing");
rw!(Paragraph, space_after, set_space_after, "spaceAfter");
rw!(Paragraph, space_before, set_space_before, "spaceBefore");
ro!(Paragraph, is_list_item, "isListItem");

#[wasm_bindgen]
impl Paragraph {
    #[wasm_bindgen(getter)]
    pub fn font(&self) -> Font {
        Font { p: self.p.to(prop("font"), &FONT) }
    }
    /// `getRange(rangeLocation?)`: `Whole` (the default), `Start`, `End`, `Content`.
    #[wasm_bindgen(js_name = getRange)]
    pub fn get_range(&self, location_: JsValue) -> Result<Range, JsValue> {
        Ok(Range { p: self.p.nav(method("getRange", vec![location(&location_, Some("Whole"))?]), Named::Range, &RANGE) })
    }
    /// `getNext()` (WordApi 1.3): fails the sync with `ItemNotFound` after the last paragraph.
    #[wasm_bindgen(js_name = getNext)]
    pub fn get_next(&self) -> Paragraph {
        Paragraph { p: self.p.nav(method("getNext", vec![]), Named::Paragraph, &PARAGRAPH) }
    }
    /// `getPrevious()` (WordApi 1.3).
    #[wasm_bindgen(js_name = getPrevious)]
    pub fn get_previous(&self) -> Paragraph {
        Paragraph { p: self.p.nav(method("getPrevious", vec![]), Named::Paragraph, &PARAGRAPH) }
    }
    /// `insertText(text, insertLocation)`: `Start`, `End`, `Replace`.
    #[wasm_bindgen(js_name = insertText)]
    pub fn insert_text(&self, text: String, location_: JsValue) -> Result<Range, JsValue> {
        Ok(Range { p: self.p.create("insertText", vec![json!(text), location(&location_, None)?], Named::Range, &RANGE) })
    }
    /// `insertParagraph(paragraphText, insertLocation)`: `Before`, `After`.
    #[wasm_bindgen(js_name = insertParagraph)]
    pub fn insert_paragraph(&self, text: String, location_: JsValue) -> Result<Paragraph, JsValue> {
        Ok(Paragraph { p: self.p.create("insertParagraph", vec![json!(text), location(&location_, None)?], Named::Paragraph, &PARAGRAPH) })
    }
    pub fn delete(&self) {
        self.p.call("delete", vec![]);
    }
    /// `select(selectionMode?)`: `Select` (the default), `Start`, `End`.
    pub fn select(&self, mode: JsValue) {
        self.p.call("select", select_args(&mode));
    }
}

fn select_args(mode: &JsValue) -> Vec<Json> {
    match mode.as_string() {
        Some(m) if !m.eq_ignore_ascii_case("select") => vec![json!(m)],
        _ => Vec::new(),
    }
}

// ---------------------------------------------------------------- ranges

class!(Range);
ro!(Range, text, "text");
ro!(Range, is_empty, "isEmpty");
rw!(Range, style, set_style, "style");

#[wasm_bindgen]
impl Range {
    #[wasm_bindgen(getter)]
    pub fn font(&self) -> Font {
        Font { p: self.p.to(prop("font"), &FONT) }
    }
    #[wasm_bindgen(getter)]
    pub fn paragraphs(&self) -> ParagraphCollection {
        ParagraphCollection { p: self.p.to(prop("paragraphs"), &PARAGRAPHS) }
    }
    /// `insertText(text, insertLocation)`: `Start`, `End`, `Before`, `After`, `Replace`.
    #[wasm_bindgen(js_name = insertText)]
    pub fn insert_text(&self, text: String, location_: JsValue) -> Result<Range, JsValue> {
        Ok(Range { p: self.p.create("insertText", vec![json!(text), location(&location_, None)?], Named::Range, &RANGE) })
    }
    /// `insertParagraph(paragraphText, insertLocation)`: `Before`, `After`.
    #[wasm_bindgen(js_name = insertParagraph)]
    pub fn insert_paragraph(&self, text: String, location_: JsValue) -> Result<Paragraph, JsValue> {
        Ok(Paragraph { p: self.p.create("insertParagraph", vec![json!(text), location(&location_, None)?], Named::Paragraph, &PARAGRAPH) })
    }
    pub fn delete(&self) {
        self.p.call("delete", vec![]);
    }
    /// `select(selectionMode?)`: `Select` (the default), `Start`, `End`.
    pub fn select(&self, mode: JsValue) {
        self.p.call("select", select_args(&mode));
    }
}

collection!(RangeCollection, Range);

class!(Font);
rw!(Font, bold, set_bold, "bold");
rw!(Font, italic, set_italic, "italic");
rw!(Font, underline, set_underline, "underline");
rw!(Font, size, set_size, "size");
rw!(Font, name, set_name, "name");
rw!(Font, color, set_color, "color");
rw!(Font, highlight_color, set_highlight_color, "highlightColor");

// ---------------------------------------------------------------- Word.run

/// `Word.run(batch)` (or `(object, batch)`): runs `batch(context)`, then sends what it left
/// queued; resolves with the batch's value.
fn word_run(a: JsValue, b: JsValue) -> Promise {
    let batch = a.clone().dyn_into::<Function>().or_else(|_| b.dyn_into::<Function>());
    future_to_promise(async move {
        let batch = batch.map_err(|_| js_error(&OfficeError::invalid("Word.run needs a function")))?;
        let rc = RequestContext::new();
        let ctx = rc.p.ctx.clone();
        let mut value = batch.call1(&JsValue::UNDEFINED, &JsValue::from(rc))?;
        if let Ok(p) = value.clone().dyn_into::<Promise>() {
            value = JsFuture::from(p).await?;
        }
        if ctx.borrow().has_queued() {
            sync(ctx).await?;
        }
        Ok(value)
    })
}

// ---------------------------------------------------------------- Office (common API)

fn platform() -> String {
    let host = Reflect::get(&js_sys::global(), &"__craftOffice".into()).unwrap_or(JsValue::UNDEFINED);
    Reflect::get(&host, &"platform".into()).ok().and_then(|p| p.as_string()).unwrap_or_else(|| "PC".into())
}

fn obj(pairs: &[(&str, JsValue)]) -> Object {
    let o = Object::new();
    for (k, v) in pairs {
        let _ = Reflect::set(&o, &(*k).into(), v);
    }
    o
}

/// `{status, value, error, asyncContext}` for a callback.
fn async_result(r: Result<JsValue, OfficeError>, async_context: JsValue) -> JsValue {
    let o = match r {
        Ok(v) => obj(&[("status", "succeeded".into()), ("value", v)]),
        Err(e) => {
            let err = obj(&[("code", e.code.as_str().into()), ("name", e.code.as_str().into()), ("message", e.message.as_str().into())]);
            obj(&[("status", "failed".into()), ("value", JsValue::UNDEFINED), ("error", err.into())])
        }
    };
    let _ = Reflect::set(&o, &"asyncContext".into(), &async_context);
    o.into()
}

/// `(options?, callback?)` as Office.js's `*Async` methods take them.
fn options_and_callback(a: JsValue, b: JsValue) -> (JsValue, Option<Function>) {
    match a.clone().dyn_into::<Function>() {
        Ok(f) => (JsValue::UNDEFINED, Some(f)),
        Err(_) => (a, b.dyn_into::<Function>().ok()),
    }
}

fn finish(callback: Option<Function>, options: JsValue, r: Result<JsValue, OfficeError>) {
    if let Some(cb) = callback {
        let ctx = Reflect::get(&options, &"asyncContext".into()).unwrap_or(JsValue::UNDEFINED);
        let _ = cb.call1(&JsValue::UNDEFINED, &async_result(r, ctx));
    }
}

/// A common-API call, answered on the `document` root (`getSelectedData`, `setSelectedData`).
async fn document_call(method_name: &str, args: Vec<Json>) -> Result<Json, OfficeError> {
    let answer = send(vec![json!({"op": "call", "path": [prop("document")], "method": method_name, "args": args})]).await?;
    let results = crate::core::results(&answer)?;
    Ok(results.first().cloned().unwrap_or(Json::Null))
}

fn document() -> Object {
    let get_selected = Closure::<dyn Fn(JsValue, JsValue, JsValue)>::new(|coercion: JsValue, a: JsValue, b: JsValue| {
        let (options, cb) = options_and_callback(a, b);
        let kind = coercion.as_string().unwrap_or_else(|| "text".into());
        spawn_local(async move {
            let r = match kind.as_str() {
                "text" => document_call("getSelectedData", vec![json!(kind)]).await.map(|v| to_js(&v)),
                other => Err(OfficeError::not_implemented(&format!("coercion type {other}"))),
            };
            finish(cb, options, r);
        });
    });
    let set_selected = Closure::<dyn Fn(JsValue, JsValue, JsValue)>::new(|data: JsValue, a: JsValue, b: JsValue| {
        let (options, cb) = options_and_callback(a, b);
        let data = from_js(&data);
        let coercion = Reflect::get(&options, &"coercionType".into()).ok().and_then(|c| c.as_string());
        spawn_local(async move {
            let r = match coercion.as_deref() {
                None | Some("text") if data.is_string() => document_call("setSelectedData", vec![data]).await.map(|_| JsValue::UNDEFINED),
                Some(other) if other != "text" => Err(OfficeError::not_implemented(&format!("coercion type {other}"))),
                _ => Err(OfficeError::not_implemented("setting the selection to anything but text")),
            };
            finish(cb, options, r);
        });
    });
    let doc = obj(&[("getSelectedDataAsync", get_selected.into_js_value()), ("setSelectedDataAsync", set_selected.into_js_value())]);
    let _ = Reflect::set(&doc, &"settings".into(), &settings().into());
    let _ = Reflect::set(&doc, &"mode".into(), &"readWrite".into());
    doc
}

/// `Office.context.document.settings`: kept here, saved to the host with `saveAsync`.
fn settings() -> Object {
    let get = Closure::<dyn Fn(String) -> JsValue>::new(|name: String| SETTINGS.with(|s| s.borrow().get(&name).map(to_js).unwrap_or(JsValue::NULL)));
    let set = Closure::<dyn Fn(String, JsValue)>::new(|name: String, v: JsValue| {
        let v = from_js(&v);
        SETTINGS.with(|s| s.borrow_mut().insert(name, v));
    });
    let remove = Closure::<dyn Fn(String)>::new(|name: String| {
        SETTINGS.with(|s| s.borrow_mut().remove(&name));
    });
    let save = Closure::<dyn Fn(JsValue, JsValue)>::new(|a: JsValue, b: JsValue| {
        let (options, cb) = options_and_callback(a, b);
        let all = SETTINGS.with(|s| Json::Object(s.borrow().clone()));
        spawn_local(async move {
            let r = match send(vec![json!({"op": "call", "path": [prop("settings")], "method": "save", "args": [all]})]).await {
                Ok(answer) => crate::core::results(&answer).map(|_| JsValue::UNDEFINED),
                Err(e) => Err(e),
            };
            finish(cb, options, r);
        });
    });
    obj(&[("get", get.into_js_value()), ("set", set.into_js_value()), ("remove", remove.into_js_value()), ("saveAsync", save.into_js_value())])
}

/// Reads the add-in's saved settings from the host (at start).
async fn load_settings() {
    if let Ok(answer) = send(vec![json!({"op": "load", "path": [prop("settings")], "props": []})]).await
        && let Some(Json::Object(saved)) = answer.get("results").and_then(|r| r.get(0)).cloned()
    {
        SETTINGS.with(|s| *s.borrow_mut() = saved);
    }
}

/// An enum object: `{Key: "Value"}` for each value.
fn enums(pairs: &[(&str, &str)]) -> JsValue {
    let o = Object::new();
    for (k, v) in pairs {
        let _ = Reflect::set(&o, &(*k).into(), &(*v).into());
    }
    o.into()
}

/// A Word enum: camelCase keys, PascalCase values (`end: "End"`).
fn word_enum<S: AsRef<str>>(keys: &[S]) -> JsValue {
    let o = Object::new();
    for k in keys {
        let k = k.as_ref();
        let _ = Reflect::set(&o, &k.into(), &enum_value(k).into());
    }
    o.into()
}

fn office() -> Object {
    let platform = platform();
    let info = obj(&[("host", "Word".into()), ("platform", platform.as_str().into())]);
    let on_ready = {
        let info = info.clone();
        Closure::<dyn Fn(JsValue) -> Promise>::new(move |cb: JsValue| {
            let ready = READY.with(|r| r.borrow().clone()).unwrap_or_else(|| Promise::resolve(&JsValue::UNDEFINED));
            let info = info.clone();
            future_to_promise(async move {
                let _ = JsFuture::from(ready).await;
                if let Ok(f) = cb.dyn_into::<Function>() {
                    let _ = f.call1(&JsValue::UNDEFINED, &info);
                }
                Ok(info.into())
            })
        })
    };
    let is_set = Closure::<dyn Fn(String, JsValue) -> bool>::new(|name: String, v: JsValue| {
        let v = v.as_string().or_else(|| v.as_f64().map(|f| format!("{f}")));
        is_set_supported(&name, v.as_deref())
    });
    let dialog = Closure::<dyn Fn(JsValue, JsValue, JsValue)>::new(|_url: JsValue, a: JsValue, b: JsValue| {
        let (options, cb) = options_and_callback(a, b);
        finish(cb, options, Err(OfficeError::not_implemented("displayDialogAsync")));
    });
    let context = obj(&[
        ("diagnostics", obj(&[("host", "Word".into()), ("platform", platform.as_str().into()), ("version", "16.0 (WordCraft)".into())]).into()),
        ("requirements", obj(&[("isSetSupported", is_set.into_js_value())]).into()),
        ("document", document().into()),
        ("ui", obj(&[("displayDialogAsync", dialog.into_js_value())]).into()),
        ("displayLanguage", "en-US".into()),
        ("contentLanguage", "en-US".into()),
        ("host", "Word".into()),
        ("platform", platform.as_str().into()),
    ]);
    obj(&[
        ("onReady", on_ready.into_js_value()),
        ("context", context.into()),
        (
            "CoercionType",
            enums(&[
                ("Text", "text"),
                ("Matrix", "matrix"),
                ("Table", "table"),
                ("Html", "html"),
                ("Ooxml", "ooxml"),
                ("SlideRange", "slideRange"),
                ("Image", "image"),
            ]),
        ),
        ("AsyncResultStatus", enums(&[("Succeeded", "succeeded"), ("Failed", "failed")])),
        ("HostType", enums(&[("Excel", "Excel"), ("Word", "Word"), ("PowerPoint", "PowerPoint"), ("Outlook", "Outlook")])),
        ("PlatformType", enums(&[("PC", "PC"), ("Mac", "Mac"), ("OfficeOnline", "OfficeOnline"), ("iOS", "iOS"), ("Android", "Android")])),
    ])
}

/// `Word.Style` / `Word.BuiltInStyleName`'s keys (WordApi 1.3), from the public API reference.
fn built_in_styles() -> Vec<String> {
    let mut keys: Vec<String> = vec!["other".into(), "normal".into()];
    keys.extend((1..=9).map(|i| format!("heading{i}")));
    keys.extend((1..=9).map(|i| format!("toc{i}")));
    keys.extend(
        [
            "footnoteText",
            "header",
            "footer",
            "caption",
            "footnoteReference",
            "endnoteReference",
            "endnoteText",
            "title",
            "subtitle",
            "hyperlink",
            "strong",
            "emphasis",
            "noSpacing",
            "listParagraph",
            "quote",
            "intenseQuote",
            "subtleEmphasis",
            "intenseEmphasis",
            "subtleReference",
            "intenseReference",
            "bookTitle",
            "bibliography",
            "tocHeading",
            "tableGrid",
        ]
        .iter()
        .map(|s| s.to_string()),
    );
    keys.extend((1..=5).map(|i| format!("plainTable{i}")));
    keys.push("tableGridLight".into());
    for family in ["gridTable", "listTable"] {
        for variant in ["1Light", "2", "3", "4", "5Dark", "6Colorful", "7Colorful"] {
            keys.push(format!("{family}{variant}"));
            keys.extend((1..=6).map(|a| format!("{family}{variant}_Accent{a}")));
        }
    }
    keys
}

const UNDERLINE_TYPES: &[&str] = &[
    "mixed",
    "none",
    "hidden",
    "dotLine",
    "single",
    "word",
    "double",
    "thick",
    "dotted",
    "dottedHeavy",
    "dashLine",
    "dashLineHeavy",
    "dashLineLong",
    "dashLineLongHeavy",
    "dotDashLine",
    "dotDashLineHeavy",
    "twoDotDashLine",
    "twoDotDashLineHeavy",
    "wave",
    "waveHeavy",
    "waveDouble",
];

const ERROR_CODES: &[&str] =
    &["AccessDenied", "GeneralException", "InvalidArgument", "ItemNotFound", "NotImplemented", "PropertyNotLoaded", "ValueNotLoaded"];

fn word() -> Object {
    let run = Closure::<dyn Fn(JsValue, JsValue) -> Promise>::new(word_run);
    let o = obj(&[("run", run.into_js_value())]);
    // `new Word.RequestContext()`: the exported class, reached through an instance of it.
    let ctor = Reflect::get(&JsValue::from(RequestContext::new()), &"constructor".into()).unwrap_or(JsValue::UNDEFINED);
    let _ = Reflect::set(&o, &"RequestContext".into(), &ctor);
    let styles = built_in_styles();
    let codes = Object::new();
    for c in ERROR_CODES {
        let mut chars = c.chars();
        let key: String = chars.next().map(|f| f.to_lowercase().chain(chars).collect()).unwrap_or_default();
        let _ = Reflect::set(&codes, &key.into(), &(*c).into());
    }
    let tables: [(&str, JsValue); 8] = [
        ("InsertLocation", word_enum(&["before", "after", "start", "end", "replace"])),
        ("RangeLocation", word_enum(&["whole", "start", "end", "before", "after", "content"])),
        ("Alignment", word_enum(&["mixed", "unknown", "left", "centered", "right", "justified"])),
        ("UnderlineType", word_enum(UNDERLINE_TYPES)),
        ("SelectionMode", word_enum(&["select", "start", "end"])),
        ("Style", word_enum(&styles)),
        ("BuiltInStyleName", word_enum(&styles)),
        // `Word.ErrorCodes`: camelCase keys, PascalCase values (`itemNotFound: "ItemNotFound"`).
        ("ErrorCodes", codes.into()),
    ];
    for (name, value) in tables {
        let _ = Reflect::set(&o, &name.into(), &value);
    }
    o
}

fn office_extension() -> Object {
    let codes = Object::new();
    for c in ERROR_CODES {
        let _ = Reflect::set(&codes, &(*c).into(), &(*c).into());
    }
    // `OfficeExtension.Error` is `Error`: every error this library throws is one.
    let error = Reflect::get(&js_sys::global(), &"Error".into()).unwrap_or(JsValue::UNDEFINED);
    obj(&[("Error", error), ("ErrorCodes", codes.into())])
}

fn define(name: &str, value: &JsValue) {
    let desc = obj(&[("value", value.clone()), ("writable", false.into()), ("configurable", false.into()), ("enumerable", true.into())]);
    let global: Object = js_sys::global();
    let _ = Reflect::define_property(&global, &name.into(), &desc);
}

/// Defines `Office`, `Word` and `OfficeExtension` (not writable: a later `office.js` can't
/// replace them), listens for the host's answers, and loads the add-in's settings; `onReady`
/// resolves after that. Called again, it only starts over the settings and `onReady`.
fn install_globals() {
    if !INSTALLED.with(|i| i.replace(true)) {
        define("Office", &office().into());
        define("Word", &word().into());
        define("OfficeExtension", &office_extension().into());
        let reply = Closure::<dyn Fn(String)>::new(reply);
        let _ = Reflect::set(&js_sys::global(), &"__craftOfficeReply".into(), &reply.into_js_value());
    }
    SETTINGS.with(|s| s.borrow_mut().clear());
    let ready = future_to_promise(async {
        load_settings().await;
        // A page that set `Office.initialize` gets it called, as Office does.
        let office = Reflect::get(&js_sys::global(), &"Office".into()).unwrap_or(JsValue::UNDEFINED);
        if let Ok(init) = Reflect::get(&office, &"initialize".into()).and_then(|f| f.dyn_into::<Function>()) {
            let _ = init.call1(&JsValue::UNDEFINED, &"inserted".into());
        }
        Ok(JsValue::UNDEFINED)
    });
    READY.with(|r| *r.borrow_mut() = Some(ready));
}

/// Installs the library in a task pane that talks to WordCraft through
/// `globalThis.__craftOffice.post`.
#[cfg_attr(not(test), wasm_bindgen(start))]
pub fn install() {
    TRANSPORT.with(|t| *t.borrow_mut() = None);
    install_globals();
}

/// Installs the library with a synchronous transport (tests: a host in the same process).
pub fn install_with_transport(t: impl Fn(String) -> String + 'static) {
    TRANSPORT.with(|tr| *tr.borrow_mut() = Some(Rc::new(t)));
    install_globals();
}

#[cfg(test)]
mod tests;
