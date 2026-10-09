//! Chat add-in sweep. Every member-allowed command (also those marked as not
//! editing: the gate checks them too) runs as a member,
//! with default params and with a representative params set, in several places of a sample
//! document (owner text, a table, a header, an owner's tracked insertion, an owner's comment):
//!
//! - through the gate (`run_as_member`) it is either refused, with the document unchanged, or
//!   it leaves only changes tracked under the member's name plus allowed formatting (announced
//!   once in the chat); accept/reject only resolve tracked changes;
//! - its raw effect (the same command without the guard) is pinned in `UNTRACKED`: an upstream
//!   merge that makes an allowed command change the document without tracking fails this test
//!   until someone reviews the command (deny it, or accept the new entry).
//!
//! The checker below is written independently of `chat_guard` (per-character, JSON-based).

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Value, json};
use wordcraft_doc::para::{InlineObject, OBJ, Paragraph};
use wordcraft_doc::{Block, Blocks, CharProps, Document};
use wordcraft_engine::Session;
use wordcraft_ui_egui::WordApp;
use wordcraft_ui_egui::chat_gate::{member_allowed, run_as_member};

const ME: &str = "@claude";
const OWNER: &str = "Owner";

/// Allowed mutating commands whose raw effect (without the guard) is an untracked change in at
/// least one place of the sweep. The guard refuses them there. Review every change to this list.
const UNTRACKED: &[&str] = &[
    "design.effects",
    "design.pageBorders",
    "design.pageColor",
    "design.paragraphSpacing",
    "design.styleSet",
    "design.themeFonts",
    "design.watermark",
    "format.allCaps",
    "format.charStyle",
    "format.emboss",
    "format.engrave",
    "format.outline",
    "format.position",
    "format.scale",
    "format.set",
    "format.shading",
    "format.shadow",
    "format.smallCaps",
    "format.spacing",
    "hf.position",
    "insert.blankPage",
    "insert.bookmark",
    "insert.docProperty",
    "insert.dropCap",
    "insert.editFooter",
    "insert.equation",
    "insert.field",
    "insert.horizontalLine",
    "insert.link",
    "insert.pageNumber",
    "insert.shape",
    "insert.signatureLine",
    "insert.spreadsheet",
    "insert.table",
    "insert.textBox",
    "layout.break",
    "layout.columns",
    "layout.differentFirstPage",
    "layout.differentOddEven",
    "layout.hyphenation",
    "layout.lineNumbers",
    "layout.margins",
    "layout.orientation",
    "layout.pageNumberFormat",
    "layout.size",
    "para.borders",
    "para.bullets",
    "para.keepLines",
    "para.keepNext",
    "para.multilevel",
    "para.numbering",
    "para.pageBreakBefore",
    "para.rtl",
    "para.set",
    "para.shading",
    "para.tabs",
    "para.widowControl",
    "references.addText",
    "references.bibliography",
    "references.citation",
    "references.endnote",
    "references.footnote",
    "references.index",
    "references.markCitation",
    "references.markEntry",
    "references.noteOptions",
    "references.sources",
    "references.tableOfAuthorities",
    "references.tableOfFigures",
    "references.toc",
    "references.updateFields",
    "references.updateFigures",
    "references.updateIndex",
    "review.deleteComment",
    "review.language",
    "review.rejectAll",
    "styles.create",
    "styles.modify",
    "table.autofit",
    "table.borderPainter",
    "table.borders",
    "table.cellAlign",
    "table.cellMargins",
    "table.columnWidth",
    "table.distributeRows",
    "table.look",
    "table.properties",
    "table.quick",
    "table.repeatHeader",
    "table.rowHeight",
    "table.shading",
    "table.textDirection",
];

fn owner(a: &mut WordApp, id: &str, p: Value) {
    if let Err(e) = a.run(id, p) {
        panic!("sample {id}: {e}");
    }
}

fn sample() -> WordApp {
    let mut a = WordApp::new(Session::new(Document::new()), Default::default());
    a.session.author = OWNER.into();
    owner(&mut a, "document.setText", json!({"text": "Alfa beta gama.\nDelta epsilon.\nZeta eta."}));
    owner(&mut a, "insert.editHeader", json!({}));
    owner(&mut a, "text.insert", json!({"text": "Cabeçalho do dono"}));
    owner(&mut a, "insert.closeHeader", json!({}));
    owner(&mut a, "caret.docEnd", json!({}));
    owner(&mut a, "insert.table", json!({"rows": 2, "cols": 2}));
    owner(&mut a, "text.insert", json!({"text": "Celula"}));
    owner(&mut a, "review.trackChanges", json!({"value": true}));
    owner(&mut a, "select.text", json!({"text": "Delta"}));
    owner(&mut a, "caret.right", json!({}));
    owner(&mut a, "text.insert", json!({"text": " nova"}));
    owner(&mut a, "review.trackChanges", json!({"value": false}));
    owner(&mut a, "select.text", json!({"text": "Zeta"}));
    owner(&mut a, "review.newComment", json!({"text": "comentário do dono"}));
    owner(&mut a, "caret.docStart", json!({}));
    a.chat = Some(wordcraft_chat::Hub::new("k".into()));
    a
}

/// Where the member's selection is before the command.
fn places() -> Vec<(&'static str, Vec<(&'static str, Value)>)> {
    vec![
        ("owner text", vec![("select.text", json!({"text": "beta gama"}))]),
        ("table cell", vec![("select.text", json!({"text": "Celula"}))]),
        ("owner insertion", vec![("select.text", json!({"text": "epsilon"})), ("select.paragraph", json!({}))]),
        ("comment anchor", vec![("select.text", json!({"text": "eta."})), ("select.paragraph", json!({}))]),
        ("everything", vec![("select.all", json!({}))]),
        ("header", vec![("insert.editHeader", json!({})), ("select.all", json!({}))]),
        (
            "owner paragraph split by a member Enter",
            vec![("select.text", json!({"text": "beta"})), ("select.collapse", json!({})), ("text.newParagraph", json!({})), ("select.text", json!({"text": "beta gama."}))],
        ),
    ]
}

/// Representative params (besides `{}`), per command.
fn representative(id: &str) -> Value {
    match id {
        "design.pageBorders" => json!({"kind": "box", "width": 2, "color": "FF0000"}),
        "design.pageColor" => json!({"color": "FFEEDD"}),
        "design.paragraphSpacing" => json!({"value": "double"}),
        "design.styleSet" => json!({"name": "lines"}),
        "design.theme" => json!({"name": "Office"}),
        "design.themeFonts" => json!({"heading": "Arial", "body": "Arial"}),
        "design.watermark" => json!({"text": "CANCELADO"}),
        "format.bold" | "format.italic" | "format.strikethrough" => json!({"value": true}),
        "format.charStyle" => json!({"style": "Strong"}),
        "format.color" => json!({"color": "FF0000"}),
        "format.font" => json!({"name": "Arial"}),
        "format.highlight" => json!({"color": "yellow"}),
        "format.position" => json!({"points": 3}),
        "format.scale" => json!({"percent": 150}),
        "format.set" => json!({"props": {"bold": true, "size": 14, "link": "https://example.com"}}),
        "format.shading" => json!({"color": "FFFF00"}),
        "format.size" => json!({"size": 14}),
        "format.spacing" => json!({"points": 2}),
        "format.underline" => json!({"value": true, "style": "double"}),
        "hf.position" => json!({"header": 20, "footer": 20}),
        "insert.bookmark" => json!({"name": "marca"}),
        "insert.crossReference" => json!({"to": "bookmark", "target": "marca"}),
        "insert.dateTime" => json!({"format": "d/M/yyyy"}),
        "insert.docProperty" => json!({"name": "Title"}),
        "insert.dropCap" => json!({"lines": 3}),
        "insert.equation" => json!({"linear": "x=1"}),
        "insert.field" => json!({"instr": "PAGE", "result": "1"}),
        "insert.link" => json!({"url": "https://example.com", "text": "ligação"}),
        "insert.pageNumber" => json!({"position": "current"}),
        "insert.shape" => json!({"kind": "rectangle"}),
        "insert.signatureLine" => json!({"signer": "Ana"}),
        "insert.spreadsheet" => json!({"csv": "a,b\n1,2"}),
        "insert.symbol" => json!({"char": "§"}),
        "insert.table" => json!({"rows": 2, "cols": 2}),
        "insert.textBox" => json!({"text": "caixa"}),
        "layout.break" => json!({"kind": "nextPage"}),
        "layout.columns" => json!({"count": 2}),
        "layout.lineNumbers" => json!({"value": "continuous"}),
        "layout.margins" => json!({"preset": "narrow"}),
        "layout.orientation" => json!({"value": "landscape"}),
        "layout.pageNumberFormat" => json!({"format": "lowerRoman", "start": 3}),
        "layout.size" => json!({"name": "A4"}),
        "para.align" => json!({"value": "center"}),
        "para.borders" => json!({"kind": "all"}),
        "para.bullets" => json!({"kind": "bullet"}),
        "para.defineBullet" => json!({"char": "»"}),
        "para.defineNumber" => json!({"format": "upperRoman", "start": 4}),
        "para.indents" => json!({"left": 36, "firstLine": -18}),
        "para.lineSpacing" => json!({"value": 2}),
        "para.listLevel" => json!({"level": 2}),
        "para.multilevel" => json!({"kind": "legal"}),
        "para.numbering" => json!({"kind": "numbered"}),
        "para.set" => json!({"props": {"align": "right", "keepNext": true}}),
        "para.setNumberingValue" => json!({"value": 7}),
        "para.shading" => json!({"color": "EEEEEE"}),
        "para.spacing" => json!({"before": 12, "after": 6}),
        "para.style" => json!({"style": "Heading1"}),
        "para.tabs" => json!({"tabs": [{"pos": 72, "align": "left", "leader": "dot"}]}),
        "references.addText" => json!({"level": 1}),
        "references.bibliography" => json!({"title": "Fontes"}),
        "references.citation" => json!({"source": {"tag": "X1", "kind": "book", "author": "Silva, Ana", "title": "T", "year": "2020"}}),
        "references.endnote" | "references.footnote" => json!({"text": "nota"}),
        "references.markCitation" | "references.markEntry" => json!({"entry": "entrada"}),
        "references.noteOptions" => json!({"footnoteFormat": "lowerRoman"}),
        "references.sources" => json!({"add": {"tag": "X2", "kind": "book", "author": "Sousa, Rui", "title": "U", "year": "2021"}}),
        "references.tableOfFigures" => json!({"label": "Table"}),
        "references.toc" => json!({"levels": 2, "title": "Índice"}),
        "review.applySuggestion" => json!({"text": "Gama"}),
        "review.deleteComment" => json!({"id": 0}),
        "review.language" => json!({"lang": "pt-PT", "noProof": true}),
        "review.newComment" => json!({"text": "do agente"}),
        "review.reply" => json!({"id": 0, "text": "resposta do agente"}),
        "review.resolveComment" => json!({"id": 0}),
        "styles.create" => json!({"name": "Agente", "fromSelection": true}),
        "styles.modify" => json!({"style": "Normal", "chr": {"bold": true}, "para": {"align": "center"}}),
        "styles.updateToMatch" => json!({"style": "Normal"}),
        "table.autofit" => json!({"mode": "window"}),
        "table.borders" => json!({"kind": "none"}),
        "table.cellAlign" => json!({"value": "bottomRight"}),
        "table.cellMargins" => json!({"top": 4, "left": 4}),
        "table.columnWidth" => json!({"width": 100}),
        "table.look" => json!({"headerRow": false, "bandedRows": true}),
        "table.properties" => json!({"align": "center"}),
        "table.quick" => json!({"kind": "matrix"}),
        "table.rowHeight" => json!({"height": 40}),
        "table.shading" => json!({"color": "CCCCCC"}),
        "table.style" => json!({"style": "TableGrid"}),
        "text.insert" => json!({"text": "novo texto"}),
        _ => json!({"value": true}),
    }
}

// ---------------------------------------------------------------- independent checker

const CHAR_ALLOWED: [&str; 11] = ["bold", "italic", "underline", "underlineColor", "strike", "doubleStrike", "font", "size", "color", "highlight", "vertAlign"];
/// Paragraph properties a member may change on the owner's paragraphs. Lists (`numbering`, and
/// the list a style puts the paragraph in) are not among them.
const PARA_ALLOWED: [&str; 9] = ["style", "align", "indentLeft", "indentRight", "indentFirst", "spaceBefore", "spaceAfter", "lineSpacing", "contextualSpacing"];

#[derive(Clone, Copy, PartialEq, Debug)]
enum Lens {
    Member,
    Accepted,
    Rejected,
}

#[derive(Clone, Copy, PartialEq)]
enum Level {
    Text,
    Allowed,
    Strict,
}

#[derive(Debug, PartialEq)]
enum Class {
    Clean,
    Format,
    Dirty(&'static str),
}

struct Ctx<'a> {
    d: &'a Document,
    lens: Lens,
    level: Level,
    own: &'a BTreeSet<u32>,
}

fn json_of<T: serde::Serialize>(t: &T) -> Value {
    serde_json::to_value(t).unwrap_or(Value::Null)
}

fn without(v: Value, keys: &[&str]) -> Value {
    match v {
        Value::Object(mut m) => {
            for k in keys {
                m.remove(*k);
            }
            Value::Object(m)
        }
        v => v,
    }
}

impl Ctx<'_> {
    fn who(&self, r: Option<u32>) -> Option<String> {
        r.and_then(|i| self.d.revisions.get(i as usize)).map(|x| format!("{:?}/{}/{}", x.kind, x.author, x.date))
    }
    fn mine(&self, r: Option<u32>) -> bool {
        self.lens == Lens::Member && r.and_then(|i| self.d.revisions.get(i as usize)).is_some_and(|x| x.author == ME)
    }
    fn gone(&self, c: &CharProps) -> bool {
        match self.lens {
            Lens::Member => self.mine(c.ins),
            Lens::Accepted => c.del.is_some(),
            Lens::Rejected => c.ins.is_some(),
        }
    }
    fn joins(&self, mark: &CharProps) -> bool {
        match self.lens {
            Lens::Member => self.mine(mark.ins),
            Lens::Accepted => false,
            Lens::Rejected => mark.ins.is_some(),
        }
    }
    fn revs(&self, c: &CharProps) -> String {
        let keep = self.lens == Lens::Member;
        let ins = if keep && !self.mine(c.ins) { self.who(c.ins) } else { None };
        let del = if keep && !self.mine(c.del) { self.who(c.del) } else { None };
        format!("ins={ins:?} del={del:?}")
    }
    fn look(&self, style: Option<&str>, c: &CharProps) -> String {
        let mut p = c.clone();
        p.ins = None;
        p.del = None;
        match self.level {
            Level::Text => String::new(),
            Level::Allowed => format!("hidden={} {}", self.d.styles.resolve_char(style, &p).hidden, without(json_of(&p), &CHAR_ALLOWED)),
            Level::Strict => format!("hidden={} {}", self.d.styles.resolve_char(style, &p).hidden, json_of(&p)),
        }
    }
    fn para_look(&self, p: &Paragraph) -> String {
        let list = self.d.styles.resolve_para(&p.props).numbering.filter(|n| n.num != 0).map(|n| (n.num, n.level));
        match self.level {
            Level::Text => String::new(),
            Level::Allowed => format!("{} list={list:?}", without(json_of(&p.props), &PARA_ALLOWED)),
            Level::Strict => format!("{} list={list:?}", json_of(&p.props)),
        }
    }
    fn section(&self, p: &Paragraph) -> String {
        if self.level == Level::Text { String::new() } else { json_of(&p.section).to_string() }
    }
}

/// One output line per paragraph as seen through the lens (joined paragraphs are one line).
struct Line {
    head: String,
    section: String,
    segs: Vec<(String, String)>,
    mark: String,
    mark_revs: String,
}

impl Line {
    fn push(&mut self, attrs: String, text: &str) {
        if text.is_empty() {
            return;
        }
        match self.segs.last_mut() {
            Some((a, t)) if *a == attrs => t.push_str(text),
            _ => self.segs.push((attrs, text.to_string())),
        }
    }
    fn render(&self) -> String {
        let body: String = self.segs.iter().map(|(a, t)| format!("<{a}>{t}")).collect();
        format!("P[{} {}] {body} M[{} {}]", self.head, self.section, self.mark, self.mark_revs)
    }
}

fn line_of(c: &Ctx, p: &Paragraph) -> Line {
    let style = p.props.style.as_deref();
    let mut l = Line { head: c.para_look(p), section: c.section(p), segs: Vec::new(), mark: c.look(style, &p.mark), mark_revs: c.revs(&p.mark) };
    let mut objs = p.objects.iter();
    let mut start = 0;
    for r in &p.runs {
        let seg = p.text.get(start..start + r.len).unwrap_or("");
        start += r.len;
        let gone = c.gone(&r.props);
        let attrs = format!("{} {}", c.revs(&r.props), c.look(style, &r.props));
        for ch in seg.chars() {
            if ch == OBJ {
                let o = objs.next();
                if gone {
                    continue;
                }
                let own_anchor =
                    c.lens == Lens::Member && matches!(o, Some(InlineObject::CommentStart { id } | InlineObject::CommentEnd { id }) if c.own.contains(id));
                if !own_anchor {
                    l.push(attrs.clone(), &format!("[{}]", json_of(&o)));
                }
            } else if !gone {
                l.push(attrs.clone(), ch.encode_utf8(&mut [0u8; 4]));
            }
        }
    }
    if start < p.text.len() {
        l.push("uncovered".into(), p.text.get(start..).unwrap_or(""));
    }
    l
}

/// The paragraph look of a joined line: every piece that holds text in the view (the last piece
/// when none does), repeated looks once.
fn heads(pieces: &mut Vec<(String, bool)>) -> String {
    let any = pieces.iter().any(|(_, t)| *t);
    let n = pieces.len();
    let mut keep: Vec<String> = Vec::new();
    for (i, (h, t)) in pieces.drain(..).enumerate() {
        if (t || (!any && i + 1 == n)) && keep.last() != Some(&h) {
            keep.push(h);
        }
    }
    keep.join(" | ")
}

fn flush(cur: &mut Option<Line>, pieces: &mut Vec<(String, bool)>, out: &mut Vec<String>) {
    if let Some(mut done) = cur.take() {
        done.head = heads(pieces);
        out.push(done.render());
    }
    pieces.clear();
}

fn walk(c: &Ctx, bl: &Blocks, out: &mut Vec<String>, depth: usize) {
    if depth > 16 {
        return;
    }
    let mut cur: Option<Line> = None;
    let mut pieces: Vec<(String, bool)> = Vec::new();
    let mut join = false;
    for b in bl {
        match &**b {
            Block::Para(p) => {
                let l = line_of(c, p);
                let piece = (l.head.clone(), !l.segs.is_empty());
                match cur.as_mut() {
                    Some(prev) if join => {
                        for (a, t) in l.segs {
                            prev.push(a, &t);
                        }
                        // A split gives the original mark's revisions and section to the
                        // second paragraph; the paragraph look of EVERY piece that still holds
                        // text is kept (`heads`).
                        prev.mark_revs = l.mark_revs;
                        prev.section = l.section;
                    }
                    _ => {
                        flush(&mut cur, &mut pieces, out);
                        cur = Some(l);
                    }
                }
                pieces.push(piece);
                join = c.joins(&p.mark);
            }
            Block::Table(t) => {
                flush(&mut cur, &mut pieces, out);
                join = false;
                let mut shell = t.clone();
                for r in &mut shell.rows {
                    for cell in &mut r.cells {
                        cell.blocks.clear();
                    }
                }
                out.push(if c.level == Level::Text { "TABLE".into() } else { format!("TABLE {}", json_of(&shell)) });
                for r in &t.rows {
                    out.push("ROW".into());
                    for cell in &r.cells {
                        out.push("CELL".into());
                        walk(c, &cell.blocks, out, depth + 1);
                    }
                }
                out.push("END".into());
            }
        }
    }
    flush(&mut cur, &mut pieces, out);
}

struct Seen {
    stories: BTreeMap<String, Vec<String>>,
    comments: BTreeMap<u32, String>,
}

fn seen(d: &Document, lens: Lens, level: Level, own: &BTreeSet<u32>) -> Seen {
    let c = Ctx { d, lens, level, own };
    let own_parts: BTreeSet<u32> = d.comments.iter().filter(|(k, _)| own.contains(k)).map(|(_, x)| x.part).collect();
    let mut stories = BTreeMap::new();
    let mut body = Vec::new();
    walk(&c, &d.body, &mut body, 0);
    stories.insert("body".to_string(), body);
    for (id, part) in &d.parts {
        if own_parts.contains(id) {
            continue;
        }
        let mut lines = Vec::new();
        walk(&c, &part.blocks, &mut lines, 0);
        stories.insert(format!("part {id} {:?}", part.kind), lines);
    }
    let comments = d
        .comments
        .iter()
        .filter(|(k, _)| !own.contains(k))
        .map(|(k, x)| (*k, format!("{}|{}|{}|{:?}|{}", x.author, x.initials, x.date, x.parent, x.part)))
        .collect();
    Seen { stories, comments }
}

/// Every story of `b` unchanged in `a` (new stories are only seen through a reference).
fn same(b: &Seen, a: &Seen) -> bool {
    b.comments == a.comments && b.stories.iter().all(|(k, v)| a.stories.get(k) == Some(v))
}

/// Everything outside the stories, comments, revisions and numbering.
fn rest(d: &Document) -> Value {
    let mut v = without(json_of(d), &["body", "parts", "comments", "revisions", "numbering"]);
    if let Some(s) = v.get_mut("settings").and_then(Value::as_object_mut) {
        s.remove("trackChanges");
    }
    json!([v, d.media.keys().collect::<Vec<_>>(), d.passthrough.keys().collect::<Vec<_>>()])
}

fn classify(b: &Document, a: &Document, lens: Lens) -> Class {
    let own: BTreeSet<u32> = if lens == Lens::Member {
        b.comments.iter().chain(a.comments.iter()).filter(|(_, c)| c.author == ME).map(|(k, _)| *k).collect()
    } else {
        BTreeSet::new()
    };
    let at = |lvl| (seen(b, lens, lvl, &own), seen(a, lens, lvl, &own));
    let (tb, ta) = at(Level::Text);
    if !same(&tb, &ta) {
        return Class::Dirty("text");
    }
    let (fb, fa) = at(Level::Allowed);
    if !same(&fb, &fa) {
        return Class::Dirty("format");
    }
    if rest(b) != rest(a) {
        return Class::Dirty("document settings");
    }
    let (sb, sa) = at(Level::Strict);
    let strict = same(&sb, &sa) && json_of(&b.numbering) == json_of(&a.numbering);
    match (strict, lens) {
        (true, _) => Class::Clean,
        (false, Lens::Member) => Class::Format,
        (false, _) => Class::Dirty("judge"),
    }
}

fn lens_for(id: &str) -> Lens {
    if id.starts_with("review.accept") {
        Lens::Accepted
    } else if id.starts_with("review.reject") {
        Lens::Rejected
    } else {
        Lens::Member
    }
}

fn formatou(a: &WordApp) -> usize {
    a.chat.as_ref().map(|h| h.messages()).unwrap_or_default().iter().filter(|m| m.text.contains(" formatted: ")).count()
}

/// The sample with the member's selection placed; `None` when the place cannot be reached.
fn placed(place: &[(&str, Value)]) -> Option<WordApp> {
    let mut a = sample();
    for (c, p) in place {
        run_as_member(&mut a, ME, c, p.clone()).ok()?;
    }
    Some(a)
}

#[test]
fn every_allowed_command_is_refused_tracked_or_allowed_formatting() {
    let reg = Session::new(Document::new()).registry.clone();
    let ids: Vec<&str> = reg.all().iter().filter(|c| member_allowed("engine.execute", Some(c.id))).map(|c| c.id).collect();
    assert!(ids.len() > 100, "{} commands", ids.len());
    let mut untracked: BTreeSet<&str> = BTreeSet::new();
    let mut failures: Vec<String> = Vec::new();
    let mut runs = 0;
    let (mut refused, mut formatted) = (0, 0);
    for (place_name, place) in places() {
        if placed(&place).is_none() {
            failures.push(format!("place `{place_name}` cannot be reached"));
            continue;
        }
        for id in &ids {
            for params in [json!({}), representative(id)] {
                runs += 1;
                let what = format!("{id} {params} at {place_name}");
                // Raw effect: the command as the member, tracking on, without the guard.
                let raw_dirty = {
                    let Some(mut raw) = placed(&place) else { continue };
                    if let Some(ms) = raw.member_sel.get(ME).cloned() {
                        raw.session.sel = ms.sel;
                    }
                    raw.session.author = ME.into();
                    if lens_for(id) == Lens::Member {
                        raw.session.doc.settings.track_changes = true;
                    }
                    let before = raw.session.doc.clone();
                    let ok = raw.session.run(id, &params).is_ok();
                    ok && matches!(classify(&before, &raw.session.doc, lens_for(id)), Class::Dirty(_))
                };
                if raw_dirty {
                    untracked.insert(id);
                }
                // Through the gate.
                let Some(mut g) = placed(&place) else { continue };
                let before = g.session.doc.clone();
                let said = formatou(&g);
                match run_as_member(&mut g, ME, id, params.clone()) {
                    Err(e) => {
                        refused += 1;
                        if g.session.doc != before {
                            failures.push(format!("{what}: refused ({e}) but the document changed"));
                        }
                        if formatou(&g) != said {
                            failures.push(format!("{what}: refused but announced"));
                        }
                    }
                    Ok(_) => {
                        let class = classify(&before, &g.session.doc, lens_for(id));
                        let announced = formatou(&g) - said;
                        formatted += announced;
                        if raw_dirty {
                            failures.push(format!("{what}: untracked without the guard, accepted through it ({class:?})"));
                        }
                        match class {
                            Class::Dirty(why) => failures.push(format!("{what}: accepted with an untracked change ({why})")),
                            Class::Format if announced != 1 => failures.push(format!("{what}: formatting announced {announced} times")),
                            Class::Clean if announced != 0 => failures.push(format!("{what}: no formatting but announced")),
                            _ => {}
                        }
                    }
                }
            }
        }
    }
    assert!(runs > 1000, "{runs} runs");
    eprintln!("sweep: {} commands, {runs} runs, {refused} refused, {formatted} announced as formatting", ids.len());
    assert!(failures.is_empty(), "{} failures:\n{}", failures.len(), failures.join("\n"));
    let got: Vec<&str> = untracked.into_iter().collect();
    if got != UNTRACKED {
        panic!("UNTRACKED changed (review each command, then update the list):\n{}", got.iter().map(|i| format!("    \"{i}\",")).collect::<Vec<_>>().join("\n"));
    }
}

#[test]
fn core_text_commands_stay_tracked() {
    // If an upstream change made these untracked, members could not edit at all.
    for (place, steps) in [
        ("owner text", vec![("text.insert", json!({"text": "X"})), ("text.delete", json!({})), ("text.backspace", json!({}))]),
        ("owner text", vec![("text.newParagraph", json!({})), ("text.insert", json!({"text": "Nova"}))]),
        ("owner text", vec![("review.newComment", json!({"text": "nota"}))]),
    ] {
        let mut a = sample();
        let _ = run_as_member(&mut a, ME, "select.text", json!({"text": "beta gama"}));
        for (id, p) in steps {
            let r = run_as_member(&mut a, ME, id, p.clone());
            assert!(r.is_ok(), "{place}: {id} {p}: {r:?}");
        }
    }
}
