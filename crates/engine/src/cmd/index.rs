//! References › Index: Mark Entry (XE fields), Insert Index and Update Index (the INDEX field).
//! Field switches follow ECMA-376 §17.16.5.32 (INDEX) and §17.16.5.79 (XE).

use std::collections::BTreeMap;

use serde_json::{Value, json};
use wordcraft_doc::para::InlineObject;
use wordcraft_doc::props::{CharProps, TabAlign, TabLeader, TabStop};
use wordcraft_doc::section::SectionStart;
use wordcraft_doc::{Block, Paragraph, Path, Pos, StoryRef};

use super::references::{FieldCode, leader};
use super::sel_result;
use crate::{CmdError, CmdResult, CommandSpec, Session, p};

/// The paragraph style generated lists use for their entries (see `citations.rs`).
const ENTRY_STYLE: &str = "ListEntry";

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("references.markEntry", "Mark Entry", "References › Index", mark_entry).params(
            r#"{"entry"?: string (the selection), "subentry"?: string, "crossReference"?: "See …", "bookmark"?: page-range bookmark, "bold"?: bool, "italic"?: bool, "all"?: bool (Mark All: every paragraph's first match of "text" or the selection)}"#,
        ),
        CommandSpec::new("references.index", "Insert Index", "References › Index", insert_index).params(
            r#"{"type"?: "indented|runIn", "columns"?: 1-4, "rightAlign"?: bool, "tabLeader"?: "dot|hyphen|underscore|none", "headings"?: bool (letter headings)} (replaces an existing index's settings)"#,
        ),
        CommandSpec::new("references.updateIndex", "Update Index", "References › Index", |s, _| {
            super::citations::update_generated(s)?;
            sel_result(s)
        }),
    ]
}

fn clean(t: &str, max: usize) -> String {
    t.chars().filter(|c| *c != '"' && !c.is_control()).take(max).collect::<String>().trim().to_string()
}

/// The XE field for `references.markEntry`'s parameters: `XE "Main:Sub" \t "See X"` or
/// `\r bookmark`, then `\b` and `\i` for a bold or italic page number.
fn xe_instr(s: &Session, v: &Value, main: &str) -> Result<String, CmdError> {
    let sub = p::str(v, "subentry").map(|t| clean(t, 200)).unwrap_or_default();
    let mut instr = if sub.is_empty() { format!("XE \"{main}\"") } else { format!("XE \"{main}:{sub}\"") };
    if let Some(see) = p::str(v, "crossReference").map(|t| clean(t, 200)).filter(|t| !t.is_empty()) {
        instr.push_str(&format!(" \\t \"{see}\""));
    } else if let Some(bm) = p::str(v, "bookmark").filter(|b| !b.trim().is_empty()) {
        if !s.doc.bookmarks().iter().any(|(n, _)| n == bm) {
            return Err(CmdError::Params(format!("no bookmark `{bm}`")));
        }
        instr.push_str(&format!(" \\r {}", bm.trim()));
    }
    if p::bool(v, "bold").unwrap_or(false) {
        instr.push_str(" \\b");
    }
    if p::bool(v, "italic").unwrap_or(false) {
        instr.push_str(" \\i");
    }
    Ok(instr)
}

fn mark_entry(s: &mut Session, v: &Value) -> CmdResult {
    let main = clean(&p::str(v, "entry").map(str::to_string).unwrap_or_else(|| s.selected_text()), 200);
    if main.is_empty() {
        return Err(CmdError::Params("select a word or give `entry`".into()));
    }
    let instr = xe_instr(s, v, &main)?;
    let hidden = CharProps { hidden: Some(true), ..Default::default() };
    let field = || InlineObject::Field { instr: instr.clone(), result: String::new(), locked: false };
    if !p::bool(v, "all").unwrap_or(false) {
        let (_, b) = s.sel.ordered();
        s.doc.insert_object(&b, field(), &hidden)?;
        return Ok(json!({"entry": main, "marked": 1}));
    }
    // Mark All: the first whole-word, same-case match in every paragraph that hasn't this entry.
    let text = p::str(v, "text").map(str::to_string).unwrap_or_else(|| {
        let t = s.selected_text();
        if t.trim().is_empty() { main.clone() } else { t.trim().to_string() }
    });
    if text.is_empty() {
        return Err(CmdError::Params("nothing to look for".into()));
    }
    let mut marked = 0u32;
    for path in s.doc.para_paths(StoryRef::Body) {
        let Some(para) = s.doc.para(StoryRef::Body, &path) else { continue };
        if para.props.style.as_deref().is_some_and(|st| st == ENTRY_STYLE || st.starts_with("TOC")) {
            continue;
        }
        if para.objects.iter().any(|o| matches!(o, InlineObject::Field { instr: i, .. } if *i == instr)) {
            continue;
        }
        let whole = |a: usize, b: usize| {
            let before = para.text.get(..a).and_then(|t| t.chars().next_back()).is_none_or(|c| !c.is_alphanumeric());
            let after = para.text.get(b..).and_then(|t| t.chars().next()).is_none_or(|c| !c.is_alphanumeric());
            before && after
        };
        let Some(end) = para.text.match_indices(&text).map(|(a, m)| (a, a + m.len())).find(|(a, b)| whole(*a, *b)).map(|(_, b)| b) else {
            continue;
        };
        s.doc.insert_object(&Pos { story: StoryRef::Body, path, off: end }, field(), &hidden)?;
        marked += 1;
    }
    s.clamp_selection();
    Ok(json!({"entry": main, "marked": marked}))
}

/// The INDEX field for `references.index`'s parameters: `\h "A"` letter headings, `\c "n"`
/// columns, `\e` with a tab for right-aligned page numbers, `\r` run-in subentries.
pub fn index_instr(v: &Value) -> String {
    let mut s = String::from("INDEX");
    if p::bool(v, "headings").unwrap_or(true) {
        s.push_str(" \\h \"A\"");
    }
    let cols = p::u64(v, "columns").unwrap_or(1).clamp(1, 4);
    if cols > 1 {
        s.push_str(&format!(" \\c \"{cols}\""));
    }
    if p::bool(v, "rightAlign").unwrap_or(false) {
        s.push_str(" \\e \"\t\"");
    }
    if p::str(v, "type") == Some("runIn") {
        s.push_str(" \\r");
    }
    s
}

/// The body block holding the INDEX field, and its instruction.
fn index_field(s: &Session) -> Option<(usize, String)> {
    s.doc.body.iter().enumerate().find_map(|(i, b)| {
        b.as_para()?.objects.iter().find_map(|o| match o {
            InlineObject::Field { instr, .. } if FieldCode::parse(instr).name == "INDEX" => Some((i, instr.clone())),
            _ => None,
        })
    })
}

fn insert_index(s: &mut Session, v: &Value) -> CmdResult {
    let instr = index_instr(v);
    match index_field(s) {
        Some((i, _)) => {
            // Settings for the index that is there: a new field code, then rebuild it.
            let para = s.doc.para_mut(StoryRef::Body, &Path::top(i))?;
            for o in para.objects.iter_mut() {
                if let InlineObject::Field { instr: old, .. } = o
                    && FieldCode::parse(old).name == "INDEX"
                {
                    *old = instr.clone();
                }
            }
            para.touch();
        }
        None => super::citations::generated_list(s, &instr, "Index")?,
    }
    if let Some((i, _)) = index_field(s) {
        rebuild(s, i, p::str(v, "tabLeader").map(leader))?;
    }
    s.clamp_selection();
    sel_result(s)
}

/// One page reference of an entry.
#[derive(Clone, Debug, PartialEq)]
struct PageRef {
    page: String,
    bold: bool,
    italic: bool,
}

#[derive(Default)]
struct Node {
    /// The entry's levels as written (the key is their lower-case form).
    levels: Vec<String>,
    pages: Vec<PageRef>,
    see: Vec<String>,
}

/// Every XE entry in the body with its page (or page range), grouped by entry.
fn collect(s: &mut Session) -> BTreeMap<Vec<String>, Node> {
    let mut marks: Vec<(FieldCode, Pos)> = Vec::new();
    for path in s.doc.para_paths(StoryRef::Body) {
        let Some(p) = s.doc.para(StoryRef::Body, &path) else { continue };
        for off in p.object_offsets() {
            if let Some(InlineObject::Field { instr, .. }) = p.object_at(off) {
                let f = FieldCode::parse(instr);
                if f.name == "XE" {
                    marks.push((f, Pos { story: StoryRef::Body, path: path.clone(), off }));
                }
            }
        }
    }
    let l = s.layout();
    let page = |pos: &Pos| l.caret(pos).and_then(|c| l.pages.get(c.page)).map(|p| p.number).unwrap_or(1);
    let bookmark_end = |name: &str| {
        s.doc.para_paths(StoryRef::Body).into_iter().find_map(|path| {
            let p = s.doc.para(StoryRef::Body, &path)?;
            let off = p.object_offsets().into_iter().find(|o| matches!(p.object_at(*o), Some(InlineObject::BookmarkEnd { name: n }) if n == name))?;
            Some(Pos { story: StoryRef::Body, path, off })
        })
    };
    let starts = s.doc.bookmarks();
    let mut tree: BTreeMap<Vec<String>, Node> = BTreeMap::new();
    for (f, pos) in marks {
        let text = f.args.first().cloned().unwrap_or_default();
        let levels: Vec<String> = text.split(':').map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).take(9).collect();
        if levels.is_empty() {
            continue;
        }
        // Every level above gets a line of its own, even without pages.
        for k in 1..levels.len() {
            let head: Vec<String> = levels.iter().take(k).cloned().collect();
            tree.entry(head.iter().map(|x| x.to_lowercase()).collect()).or_insert_with(|| Node { levels: head, ..Default::default() });
        }
        let node =
            tree.entry(levels.iter().map(|x| x.to_lowercase()).collect()).or_insert_with(|| Node { levels: levels.clone(), ..Default::default() });
        if let Some(see) = f.arg('t') {
            if !node.see.iter().any(|x| x == see) {
                node.see.push(see.to_string());
            }
            continue;
        }
        let range = f.arg('r').and_then(|bm| {
            let a = starts.iter().find(|(n, _)| n == bm).map(|(_, p)| page(p))?;
            let b = bookmark_end(bm).map(|p| page(&p)).unwrap_or(a);
            Some(if b > a { format!("{a}\u{2013}{b}") } else { a.to_string() })
        });
        let r = PageRef { page: range.unwrap_or_else(|| page(&pos).to_string()), bold: f.has('b'), italic: f.has('i') };
        match node.pages.iter_mut().find(|x| x.page == r.page) {
            Some(x) => {
                x.bold |= r.bold;
                x.italic |= r.italic;
            }
            None => node.pages.push(r),
        }
    }
    tree
}

/// Text runs of one index line.
type Line = Vec<(String, CharProps)>;

fn refs(line: &mut Line, node: &Node, sep: &str) {
    for (k, r) in node.pages.iter().enumerate() {
        line.push((if k == 0 { sep.to_string() } else { ", ".into() }, CharProps::default()));
        line.push((r.page.clone(), CharProps { bold: r.bold.then_some(true), italic: r.italic.then_some(true), ..Default::default() }));
    }
    for (k, see) in node.see.iter().enumerate() {
        let lead = if k > 0 || !node.pages.is_empty() { "; " } else { ". " };
        line.push((lead.into(), CharProps::default()));
        line.push((see.clone(), CharProps { italic: Some(true), ..Default::default() }));
    }
}

/// Rebuild the index whose field is in body block `i`: entries sorted and grouped, indented or
/// run-in (`\r`), letter headings (`\h`), the page separator (`\e`, a tab right-aligns the page
/// numbers with `leader` or the leader the entries had), and `\c` columns as a continuous
/// section of their own.
pub fn rebuild(s: &mut Session, i: usize, leader: Option<TabLeader>) -> Result<(), CmdError> {
    let Some(instr) = s.doc.body.get(i).and_then(|b| b.as_para()).and_then(|p| {
        p.objects.iter().find_map(|o| match o {
            InlineObject::Field { instr, .. } if FieldCode::parse(instr).name == "INDEX" => Some(instr.clone()),
            _ => None,
        })
    }) else {
        return Ok(());
    };
    let f = FieldCode::parse(&instr);
    // Old entries go; keep their leader and the section their columns had.
    let mut old_leader = None;
    let mut old_sect = None;
    while let Some(p) = s.doc.body.get(i + 1).and_then(|b| b.as_para()).filter(|p| p.props.style.as_deref() == Some(ENTRY_STYLE)) {
        old_leader = old_leader.or_else(|| p.props.tabs.as_ref().and_then(|t| t.last()).map(|t| t.leader));
        if p.section.is_some() {
            old_sect = p.section.clone();
        }
        s.doc.remove_block(StoryRef::Body, &Path::top(i + 1))?;
    }
    let leader = leader.or(old_leader).unwrap_or(TabLeader::Dot);
    let cols = f.arg('c').and_then(|c| c.trim().parse::<u32>().ok()).unwrap_or(1).clamp(1, 4);
    let sep = f.arg('e').map(str::to_string).unwrap_or_else(|| ", ".into());
    let run_in = f.has('r');
    let headings = f.switches.iter().find(|(c, _)| *c == 'h').map(|(_, a)| a.clone().unwrap_or_default());
    let tree = collect(s);
    let mut lines: Vec<(usize, Line)> = Vec::new();
    let mut letter: Option<String> = None;
    let mut keys = tree.keys().peekable();
    while let Some(key) = keys.next() {
        let Some(node) = tree.get(key) else { continue };
        let Some(main) = node.levels.first() else { continue };
        if let Some(h) = &headings {
            let first = main.chars().next().map(|c| c.to_uppercase().to_string());
            if first != letter {
                if letter.is_some() || !h.is_empty() {
                    lines.push((0, vec![(if h.is_empty() { String::new() } else { first.clone().unwrap_or_default() }, CharProps::default())]));
                }
                letter = first;
            }
        }
        if run_in {
            // Subentries follow their main entry on its line: `Main, 3: sub, 4; other, 5`.
            if node.levels.len() > 1 {
                continue;
            }
            let mut line: Line = vec![(main.clone(), CharProps::default())];
            refs(&mut line, node, &sep);
            let mut first = true;
            while let Some(sub) = keys.peek().filter(|k| k.len() > 1 && k.first() == key.first()).and_then(|k| tree.get(*k)) {
                keys.next();
                line.push((if first { ": " } else { "; " }.into(), CharProps::default()));
                first = false;
                line.push((sub.levels.iter().skip(1).cloned().collect::<Vec<_>>().join(", "), CharProps::default()));
                refs(&mut line, sub, ", ");
            }
            lines.push((0, line));
        } else {
            let mut line: Line = vec![(node.levels.last().cloned().unwrap_or_default(), CharProps::default())];
            refs(&mut line, node, &sep);
            lines.push((node.levels.len().saturating_sub(1), line));
        }
    }
    // Entries are laid out in the index's columns.
    let sects = s.doc.sections();
    let sect = sects.get(s.doc.section_index_of(i + 1)).map(|(_, x)| (*x).clone()).unwrap_or_default();
    let space = old_sect.as_ref().map_or(sect.columns.space, |x| x.columns.space);
    let col_w = (sect.text_width() - space * (cols as f32 - 1.0)) / cols as f32;
    let n = lines.len();
    for (k, (level, runs)) in lines.into_iter().enumerate() {
        let mut para = Paragraph::new().styled(ENTRY_STYLE);
        for (t, props) in runs {
            let at = para.len();
            para.insert_text(at, &t, &props)?;
        }
        para.props.indent_left = Some(18.0 * (level as f32 + 1.0));
        para.props.indent_first = Some(-18.0);
        if para.text.contains('\t') {
            para.props.tabs = Some(vec![TabStop { pos: (col_w - 0.5).max(18.0), align: TabAlign::Right, leader }]);
        }
        s.doc.insert_block(StoryRef::Body, &Path::top(i + 1 + k), Block::Para(para))?;
    }
    columns(s, i, n, cols, old_sect.map(|b| *b))?;
    s.touch();
    Ok(())
}

/// Put the `n` entries after the index field's paragraph `i` in a continuous section with `cols`
/// columns (the field's paragraph ends the section before), or merge that section back when the
/// index has one column again.
fn columns(s: &mut Session, i: usize, n: usize, cols: u32, old: Option<wordcraft_doc::section::SectionProps>) -> Result<(), CmdError> {
    let last = i + n;
    if cols > 1 && n > 0 {
        // A paragraph after the index keeps the following section from being empty.
        if last + 1 >= s.doc.body.len() {
            s.doc.insert_block(StoryRef::Body, &Path::top(last + 1), Block::Para(Paragraph::new()))?;
        }
        let after = s.doc.sections().get(s.doc.section_index_of(last + 1)).map(|(_, x)| (*x).clone()).unwrap_or_default();
        let head = s.doc.para_mut(StoryRef::Body, &Path::top(i))?;
        let new = head.section.is_none();
        if new {
            head.section = Some(Box::new(after.clone()));
        }
        let mut sect = old.unwrap_or_else(|| after.clone());
        sect.columns.count = cols;
        sect.columns.widths.clear();
        sect.start = SectionStart::Continuous;
        sect.page_num_start = None;
        let end = s.doc.para_mut(StoryRef::Body, &Path::top(last))?;
        end.section = Some(Box::new(sect));
        if new {
            let rest = s.doc.section_mut(last + 1);
            rest.start = SectionStart::Continuous;
            rest.page_num_start = None;
        }
    } else if old.is_some() {
        // Back to one column: the sections around the index join again, with the first one's start.
        let head = s.doc.para_mut(StoryRef::Body, &Path::top(i))?;
        if let Some(before) = head.section.take() {
            head.touch();
            let rest = s.doc.section_mut(i);
            rest.start = before.start;
            rest.page_num_start = before.page_num_start;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn index_lines(s: &Session) -> Vec<String> {
        s.doc.body.iter().filter_map(|b| b.as_para()).filter(|p| p.props.style.as_deref() == Some(ENTRY_STYLE)).map(|p| p.plain_text()).collect()
    }

    /// #402: Mark Entry writes XE switches (subentry, bold page, cross-reference, Mark All) and
    /// the INDEX field's switches change the index: run-in subentries, letter headings, a tab
    /// before right-aligned page numbers, and two columns in a continuous section.
    #[test]
    fn index_switches_shape_the_index() {
        let mut s = Session::new(wordcraft_doc::Document::from_text("Kilns are hot.\nGlaze the kilns.\nKilns again.\nPresses."));
        s.run("select.text", &json!({"text": "Kilns"})).unwrap();
        s.run("references.markEntry", &json!({"subentry": "firing", "bold": true})).unwrap();
        s.run("select.text", &json!({"text": "Presses"})).unwrap();
        s.run("references.markEntry", &json!({"crossReference": "See Kilns"})).unwrap();
        let all = s.run("references.markEntry", &json!({"entry": "Kilns", "text": "Kilns", "all": true})).unwrap();
        assert_eq!(all["marked"], 2, "the paragraphs with a whole-word, same-case match");
        let instrs: Vec<String> = s
            .doc
            .body
            .iter()
            .filter_map(|b| b.as_para())
            .flat_map(|p| p.objects.iter())
            .filter_map(|o| match o {
                InlineObject::Field { instr, .. } => Some(instr.clone()),
                _ => None,
            })
            .collect();
        assert!(
            instrs.contains(&"XE \"Kilns:firing\" \\b".to_string()) && instrs.contains(&"XE \"Presses\" \\t \"See Kilns\"".to_string()),
            "{instrs:?}"
        );
        s.run("caret.docEnd", &json!({})).unwrap();
        s.run("text.newParagraph", &json!({})).unwrap();
        s.run("references.index", &json!({"type": "runIn", "rightAlign": true, "columns": 2, "tabLeader": "hyphen"})).unwrap();
        let (i, instr) = index_field(&s).unwrap();
        assert_eq!(instr, "INDEX \\h \"A\" \\c \"2\" \\e \"\t\" \\r");
        assert_eq!(index_lines(&s), vec!["K", "Kilns\t1: firing, 1", "P", "Presses. See Kilns"]);
        let entry = s.doc.body[i + 2].as_para().unwrap();
        assert_eq!(entry.props.tabs.as_ref().unwrap()[0].leader, TabLeader::Hyphen);
        assert!(entry.runs.iter().any(|r| r.props.bold == Some(true)), "the \\b page number is bold");
        // The entries sit in a two-column continuous section.
        let last = s.doc.body[i + 4].as_para().unwrap();
        let sect = last.section.as_ref().expect("index section");
        assert_eq!((sect.columns.count, sect.start), (2, SectionStart::Continuous));
        // Indented again: subentries get lines of their own; the leader stays on update.
        s.run("references.index", &json!({"columns": 1, "rightAlign": true})).unwrap();
        assert_eq!(index_lines(&s), vec!["K", "Kilns\t1", "firing\t1", "P", "Presses. See Kilns"]);
        assert!(s.doc.body.iter().filter_map(|b| b.as_para()).all(|p| p.section.is_none()), "one section again");
        s.run("references.updateIndex", &json!({})).unwrap();
        let (i, _) = index_field(&s).unwrap();
        assert_eq!(s.doc.body[i + 2].as_para().unwrap().props.tabs.as_ref().unwrap()[0].leader, TabLeader::Hyphen);
    }
}
