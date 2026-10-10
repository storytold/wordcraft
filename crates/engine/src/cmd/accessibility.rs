//! The accessibility checker (Review › Check Accessibility): what makes the body hard to use with
//! a screen reader or hard to read, as `{"kind": "error|warning|tip", "rule", "issue", "where",
//! "pos"?, "select"?, "fix"?, "action"?}` items.
//!
//! `where` is the paragraph (or table) path; `pos` a place to put the caret there; `select` the
//! range that selects the object an issue is about; `action` a command (`{"label", "command",
//! "params"}`) that fixes it, run with the caret at `pos` (or the object selected). The rules and
//! their wording are our own; the contrast rule uses the WCAG 2 contrast ratio.

use serde_json::{Value, json};
use wordcraft_doc::para::{InlineObject, OBJ};
use wordcraft_doc::props::{Rgb, TextColor, VMerge};
use wordcraft_doc::table::Table;
use wordcraft_doc::{Block, Document, Paragraph, Path, Pos, StoryRef};

/// Most issues reported (a hostile document can't make the report huge).
const MAX_ISSUES: usize = 500;
/// This many empty paragraphs in a row look like spacing made with Enter.
const BLANK_RUN: usize = 3;

/// The checker's findings for `doc`, errors first, then warnings, then tips (each in document order).
pub fn check(doc: &Document) -> Vec<Value> {
    let mut c = Checker { doc, issues: Vec::new() };
    c.run();
    let rank = |v: &Value| match v["kind"].as_str() {
        Some("error") => 0,
        Some("warning") => 1,
        _ => 2,
    };
    c.issues.sort_by_key(rank);
    c.issues.truncate(MAX_ISSUES);
    c.issues
}

struct Checker<'a> {
    doc: &'a Document,
    issues: Vec<Value>,
}

fn at(path: &Path, off: usize) -> Pos {
    Pos { story: StoryRef::Body, path: path.clone(), off }
}

impl Checker<'_> {
    fn push(&mut self, v: Value) {
        if self.issues.len() < MAX_ISSUES * 2 {
            self.issues.push(v);
        }
    }

    fn run(&mut self) {
        let doc = self.doc;
        let mut last_level: Option<u8> = None;
        let mut tables: Vec<Path> = Vec::new();
        let mut blank: Vec<Path> = Vec::new();
        for path in doc.para_paths(StoryRef::Body) {
            let Some(p) = doc.para(StoryRef::Body, &path) else { continue };
            if let Some((tp, _, _)) = path.cell()
                && !tables.contains(&tp)
            {
                tables.push(tp);
            }
            self.objects(&path, p);
            // Empty paragraphs in a row (top level, no section break): spacing made with Enter.
            let consecutive = blank.last().is_none_or(|b| b.0.len() == 1 && path.0.len() == 1 && b.last() + 1 == path.last());
            if path.0.len() == 1 && p.text.trim().is_empty() && p.section.is_none() && consecutive {
                blank.push(path.clone());
            } else {
                self.blank_run(&blank);
                blank.clear();
                if path.0.len() == 1 && p.text.trim().is_empty() && p.section.is_none() {
                    blank.push(path.clone());
                }
            }
            let rp = doc.styles.resolve_para(&p.props);
            if let Some(l) = rp.outline_level.filter(|_| !p.plain_text().trim().is_empty()) {
                if let Some(prev) = last_level
                    && l > prev + 1
                {
                    self.push(json!({"kind": "warning", "rule": "headingOrder", "issue": format!("Heading level skipped (Heading {} after Heading {})", l + 1, prev + 1), "where": path.0, "pos": at(&path, 0), "fix": "Use the next heading level down, so the outline has no gaps"}));
                }
                last_level = Some(l);
            }
            if p.runs.iter().any(|r| r.props.link.is_some()) {
                let txt = p.plain_text().to_lowercase();
                if txt.contains("click here") {
                    self.push(json!({"kind": "tip", "rule": "linkText", "issue": "Link text \"click here\" isn't descriptive", "where": path.0, "pos": at(&path, 0)}));
                }
            }
            self.contrast(&path, p, rp.shading);
        }
        self.blank_run(&blank);
        for tp in tables {
            if let Some(t) = doc.table(StoryRef::Body, &tp) {
                self.table(&tp, t);
            }
        }
        if doc.core.title.trim().is_empty() {
            self.push(json!({"kind": "tip", "rule": "docTitle", "issue": "Document has no title", "fix": "file.properties {title}"}));
        }
    }

    fn blank_run(&mut self, blank: &[Path]) {
        if blank.len() >= BLANK_RUN
            && let Some(first) = blank.first()
        {
            self.push(json!({"kind": "tip", "rule": "blankParagraphs", "issue": format!("{} empty paragraphs in a row", blank.len()), "where": first.0, "pos": at(first, 0), "fix": "Use Space Before/After (Layout › Paragraph) or a page break instead of empty paragraphs"}));
        }
    }

    /// Alt text and reading order of the drawings in paragraph `p`.
    fn objects(&mut self, path: &Path, p: &Paragraph) {
        for off in p.object_offsets() {
            let Some(o) = p.object_at(off).filter(|o| o.is_drawing()) else { continue };
            let pos = at(path, off);
            let select = json!({"anchor": pos, "focus": at(path, off + OBJ.len_utf8())});
            // Ink: every stroke is its own object, so asking about each would bury the report.
            if o.is_decorative() || o.ink().is_some() {
                continue;
            }
            let titled = o.frame().is_some_and(|(_, _, f)| !f.alt.title.trim().is_empty());
            let described = !o.alt_text().trim().is_empty() || titled;
            if !described && !self.speaks_for_itself(o) {
                let (rule, issue) = match o {
                    InlineObject::Graphic { .. } => ("altText", "Chart or diagram has no alternative text"),
                    _ => ("altText", "Missing alternative text"),
                };
                self.push(json!({
                    "kind": "error", "rule": rule, "issue": issue, "what": kind_of(o), "where": path.0, "pos": pos, "select": select,
                    "fix": "Describe it in the Alt Text pane (object.altText), or mark it decorative",
                    "action": {"label": "Mark as decorative", "command": "object.altText", "params": {"decorative": true}},
                }));
            }
            if o.is_floating() {
                let mut v = json!({"kind": "tip", "rule": "floating", "issue": "Object isn't in line with text, so its reading order is unclear", "what": kind_of(o), "where": path.0, "pos": pos, "select": select, "fix": "Place it in line with text (Layout › Wrap Text)"});
                // Charts and diagrams keep their imported placement (see objects.rs).
                if !matches!(o, InlineObject::Graphic { .. }) {
                    v["action"] = json!({"label": "In Line with Text", "command": "arrange.wrap", "params": {"wrap": "inline"}});
                }
                self.push(v);
            }
        }
    }

    /// A text box with text in it is read out as that text.
    fn speaks_for_itself(&self, o: &InlineObject) -> bool {
        o.text_box().and_then(|id| self.doc.parts.get(&id)).is_some_and(|part| {
            part.blocks.iter().any(|b| match &**b {
                Block::Para(p) => !p.plain_text().trim().is_empty(),
                _ => true,
            })
        })
    }

    /// Text whose colour is too close to what it's drawn on (WCAG 2: 4.5:1, 3:1 for large text).
    fn contrast(&mut self, path: &Path, p: &Paragraph, para_shading: Option<Rgb>) {
        let doc = self.doc;
        let cell = path.cell().and_then(|(tp, r, c)| {
            let t = doc.table(StoryRef::Body, &tp)?;
            t.rows.get(r).and_then(|row| row.cells.get(c)).and_then(|c| c.props.shading).or(t.props.shading)
        });
        let page = doc.settings.page_color.unwrap_or(Rgb::WHITE);
        let mut start = 0usize;
        for r in &p.runs {
            let text = p.text.get(start..start.saturating_add(r.len)).unwrap_or("");
            start = start.saturating_add(r.len);
            if text.trim().is_empty() || r.props.hidden == Some(true) {
                continue;
            }
            let rc = doc.styles.resolve_char(p.props.style.as_deref(), &r.props);
            let TextColor::Rgb(fg) = rc.color else { continue };
            let bg = rc.highlight.or(rc.shading).or(para_shading).or(cell).unwrap_or(page);
            let ratio = contrast_ratio(fg, bg);
            let large = rc.size >= 18.0 || (rc.bold && rc.size >= 14.0);
            let need = if large { 3.0 } else { 4.5 };
            if ratio < need {
                let kind = if ratio < 3.0 { "warning" } else { "tip" };
                self.push(json!({"kind": kind, "rule": "contrast", "issue": format!("Text is hard to read: contrast {:.1}:1, {need}:1 needed", ratio), "where": path.0, "pos": at(path, 0), "fix": "Use a darker or lighter text colour, or change the shading behind it"}));
                return;
            }
        }
    }

    fn table(&mut self, tp: &Path, t: &Table) {
        let cell_pos = |r: usize, c: usize| {
            let mut v = tp.0.clone();
            v.extend([r as u32, c as u32, 0]);
            at(&Path(v), 0)
        };
        if t.props.caption.as_deref().is_none_or(|s| s.trim().is_empty()) && t.props.description.as_deref().is_none_or(|s| s.trim().is_empty()) {
            self.push(json!({"kind": "warning", "rule": "tableAltText", "issue": "Table has no alternative text", "where": tp.0, "pos": cell_pos(0, 0), "fix": "Give it a title or description (table.altText)"}));
        }
        if t.rows.len() >= 2 && !t.rows.first().is_some_and(|r| r.props.header) {
            self.push(json!({
                "kind": "warning", "rule": "tableHeader", "issue": "Table has no header row", "where": tp.0, "pos": cell_pos(0, 0),
                "fix": "Repeat the first row as a header row (table.repeatHeader)",
                "action": {"label": "Repeat Header Rows", "command": "table.repeatHeader", "params": {"value": true}},
            }));
        }
        let empty = |c: &wordcraft_doc::table::Cell| {
            c.blocks.iter().all(|b| match &**b {
                Block::Para(p) => p.text.trim().is_empty(),
                _ => false,
            })
        };
        // A table with nothing in it yet is being filled in, not used for spacing.
        let filled = !t.rows.iter().flat_map(|r| &r.cells).all(empty);
        if t.rows.len() >= 2 && filled {
            for (r, row) in t.rows.iter().enumerate() {
                if !row.cells.is_empty() && row.cells.iter().all(empty) {
                    self.push(json!({"kind": "warning", "rule": "blankRow", "issue": format!("Row {} of a table is empty: it looks like spacing", r + 1), "where": tp.0, "pos": cell_pos(r, 0), "fix": "Delete the row and use cell margins or spacing instead"}));
                }
            }
        }
        let cols = t.rows.iter().map(|r| r.cells.len()).max().unwrap_or(0);
        if cols >= 2 && t.rows.len() >= 2 && filled {
            for c in 0..cols.min(63) {
                if t.rows.iter().all(|row| row.cells.get(c).is_none_or(empty)) {
                    self.push(json!({"kind": "warning", "rule": "blankColumn", "issue": format!("Column {} of a table is empty: it looks like spacing", c + 1), "where": tp.0, "pos": cell_pos(0, c), "fix": "Delete the column and use cell margins instead"}));
                }
            }
        }
        if t.rows.iter().flat_map(|r| &r.cells).any(|c| c.span() > 1 || c.props.vmerge != VMerge::None) {
            self.push(json!({"kind": "tip", "rule": "mergedCells", "issue": "Table has merged cells, which screen readers can lose track of", "where": tp.0, "pos": cell_pos(0, 0), "fix": "Split the merged cells (Table Layout › Merge)"}));
        }
    }
}

fn kind_of(o: &InlineObject) -> &'static str {
    match o {
        InlineObject::Image { ole: Some(_), .. } => "object",
        InlineObject::Image { .. } => "picture",
        InlineObject::Graphic { .. } => "chart",
        InlineObject::Shape { story: Some(_), .. } => "textBox",
        InlineObject::Shape { .. } => "shape",
        InlineObject::Group { .. } => "group",
        _ => "object",
    }
}

/// WCAG 2 relative luminance of an sRGB colour.
fn luminance(c: Rgb) -> f64 {
    let ch = |v: u8| {
        let s = v as f64 / 255.0;
        if s <= 0.04045 { s / 12.92 } else { ((s + 0.055) / 1.055).powf(2.4) }
    };
    0.2126 * ch(c.0) + 0.7152 * ch(c.1) + 0.0722 * ch(c.2)
}

/// WCAG 2 contrast ratio of two colours, 1 to 21.
pub fn contrast_ratio(a: Rgb, b: Rgb) -> f64 {
    let (la, lb) = (luminance(a), luminance(b));
    let (hi, lo) = if la >= lb { (la, lb) } else { (lb, la) };
    (hi + 0.05) / (lo + 0.05)
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use wordcraft_doc::Document;
    use wordcraft_doc::props::Rgb;

    use crate::Session;

    fn rules(s: &mut Session) -> Vec<(String, String)> {
        let v = s.run("file.accessibility", &json!({})).unwrap();
        v["issues"]
            .as_array()
            .unwrap()
            .iter()
            .map(|i| (i["kind"].as_str().unwrap().to_string(), i["rule"].as_str().unwrap_or("").to_string()))
            .collect()
    }

    #[test]
    fn contrast_ratio_matches_wcag() {
        assert!((super::contrast_ratio(Rgb::BLACK, Rgb::WHITE) - 21.0).abs() < 0.01);
        assert!((super::contrast_ratio(Rgb(0x77, 0x77, 0x77), Rgb::WHITE) - 4.48).abs() < 0.01);
    }

    #[test]
    fn shapes_need_alt_text_unless_decorative_and_the_fix_marks_them() {
        let mut s = Session::new(Document::from_text("Title"));
        s.doc.core.title = "T".into();
        s.run("insert.shape", &json!({"kind": "ellipse"})).unwrap();
        assert!(rules(&mut s).contains(&("error".into(), "altText".into())));
        // The issue says how to fix it: select the object, run the action.
        let v = s.run("file.accessibility", &json!({})).unwrap();
        let i = v["issues"].as_array().unwrap().iter().find(|i| i["rule"] == "altText").unwrap().clone();
        s.run("select.range", &i["select"]).unwrap();
        s.run(i["action"]["command"].as_str().unwrap(), &i["action"]["params"]).unwrap();
        assert!(!rules(&mut s).iter().any(|(_, r)| r == "altText"));
    }

    #[test]
    fn tables_blank_paragraphs_and_contrast_are_reported() {
        let mut s = Session::new(Document::from_text("Intro"));
        s.doc.core.title = "T".into();
        // Light grey text on white.
        s.run("select.all", &json!({})).unwrap();
        s.run("format.color", &json!({"color": "BBBBBB"})).unwrap();
        s.run("caret.docEnd", &json!({})).unwrap();
        for _ in 0..4 {
            s.run("text.newParagraph", &json!({})).unwrap();
        }
        s.run("insert.table", &json!({"rows": 3, "cols": 2})).unwrap();
        s.run("text.insert", &json!({"text": "Year"})).unwrap();
        let r = rules(&mut s);
        for want in
            [("warning", "contrast"), ("tip", "blankParagraphs"), ("warning", "tableHeader"), ("warning", "tableAltText"), ("warning", "blankRow")]
        {
            assert!(r.contains(&(want.0.into(), want.1.into())), "{want:?} in {r:?}");
        }
        // Fix the header row from the issue's action, with the caret where it says.
        let v = s.run("file.accessibility", &json!({})).unwrap();
        let i = v["issues"].as_array().unwrap().iter().find(|i| i["rule"] == "tableHeader").unwrap().clone();
        s.run("caret.set", &json!({"pos": i["pos"]})).unwrap();
        s.run(i["action"]["command"].as_str().unwrap(), &i["action"]["params"]).unwrap();
        assert!(!rules(&mut s).iter().any(|(_, r)| r == "tableHeader"));
    }
}
