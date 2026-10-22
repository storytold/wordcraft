//! Lists, as the exchange-flow writer wrote them: each run of numbered paragraphs is a
//! `text:list` of its own list style, whose levels are bullets or numbers (`1.`, `a.`, `i.` in
//! turn) as the paragraphs at each level are, with indents of half an inch a level. A list cut by
//! a section boundary continues the one before it (`text:continue-list`).
//!
//! The numbers are Word's: the model's counters run over every list item in document order, and
//! an item whose number is not the one ODF would count (the level's start, or the previous item's
//! number plus one, deeper levels starting again after a higher item) says its number in
//! `text:start-value`. That keeps a list continued after other paragraphs, a list restarted on
//! the same definition, a second list right after the first, and start overrides.
//!
//! todo: the numbering definitions themselves (formats, label templates, indents).

use std::sync::Arc;

use wordcraft_doc::{Block, Document, Paragraph};

use super::{Ctx, pt, text};
use crate::model::ListInfo;

#[derive(Default)]
pub(in crate::odt) struct State {
    /// The next list continues the last one of the body (set at a section boundary).
    pub continue_next: bool,
    /// `xml:id` of the last list of the body.
    last_id: Option<String>,
}

/// Whether `p` is numbered or bulleted, and at what level.
pub(super) fn info(doc: &Document, p: &Paragraph) -> Option<ListInfo> {
    crate::model::para_kind(doc, p).1
}

/// Word's number of list item `p` at its level, advancing the counters.
fn number(ctx: &mut Ctx, p: &Paragraph) -> Option<u32> {
    let doc = ctx.doc;
    let n = doc.styles.resolve_para(&p.props).numbering?;
    ctx.counters.next_label(&doc.numbering, n.num, n.level)?;
    ctx.counters.values(&doc.numbering, n.num)?.get(usize::from(n.level.min(8))).copied()
}

/// `text:list-style` with the levels `ordered` says.
fn list_style(name: &str, ordered: &[bool; 9], starts: &[u32; 9]) -> String {
    let mut s = format!("<text:list-style style:name=\"{name}\">");
    for (l, (ordered, start)) in ordered.iter().zip(starts).enumerate() {
        let lv = l + 1;
        let props = format!(
            "<style:list-level-properties text:list-level-position-and-space-mode=\"label-alignment\"><style:list-level-label-alignment text:label-followed-by=\"listtab\" text:list-tab-stop-position=\"{0}\" fo:text-indent=\"-18pt\" fo:margin-left=\"{0}\"/></style:list-level-properties>",
            pt(36.0 * lv as f32)
        );
        if *ordered {
            let fmt = ["1", "a", "i"].get(l % 3).copied().unwrap_or("1");
            let from = if *start == 1 { String::new() } else { format!(" text:start-value=\"{start}\"") };
            s.push_str(&format!("<text:list-level-style-number text:level=\"{lv}\" style:num-suffix=\".\" style:num-format=\"{fmt}\"{from}>{props}</text:list-level-style-number>"));
        } else {
            let ch = ["•", "◦", "▪"].get(l % 3).copied().unwrap_or("•");
            s.push_str(&format!(
                "<text:list-level-style-bullet text:level=\"{lv}\" text:bullet-char=\"{ch}\">{props}</text:list-level-style-bullet>"
            ));
        }
    }
    s.push_str("</text:list-style>");
    s
}

/// The list starting at `bl[0]`: the numbered paragraphs that follow, nested by level, up to a
/// top-level item of the other kind. How many blocks it took.
pub(super) fn list(ctx: &mut Ctx, bl: &[Arc<Block>], out: &mut String) -> usize {
    let doc = ctx.doc;
    let mut items: Vec<(&Paragraph, usize)> = Vec::new();
    let mut ordered = [false; 9];
    let mut seen = [false; 9];
    let mut starts = [1u32; 9];
    for b in bl {
        let Block::Para(p) = &**b else { break };
        let Some(li) = info(doc, p) else { break };
        let l = usize::from(li.level.min(8));
        if l == 0 && seen[0] && ordered[0] != li.ordered {
            break;
        }
        if let (Some(s), Some(o)) = (seen.get_mut(l), ordered.get_mut(l))
            && !*s
        {
            *s = true;
            *o = li.ordered;
            starts[l] = li.start;
        }
        items.push((p, l));
    }
    // Every list has a style of its own, so that none continues another by sharing it.
    let id = ctx.name("lst");
    let style = ctx.auto("L", &id, |name| list_style(name, &ordered, &starts));
    let body = ctx.depth == 0;
    let cont = match &ctx.lists.last_id {
        Some(prev) if body && std::mem::take(&mut ctx.lists.continue_next) => format!(" text:continue-list=\"{prev}\""),
        _ => String::new(),
    };
    if body {
        ctx.lists.last_id = Some(id.clone());
    }
    // Open lists: per level, whether a list item is open.
    let mut stack: Vec<bool> = Vec::new();
    // What ODF has counted at each level so far.
    let mut counted: [Option<u32>; 9] = [None; 9];
    for (p, lv) in &items {
        while stack.len() > lv + 1 {
            if stack.pop() == Some(true) {
                out.push_str("</text:list-item>");
            }
            out.push_str("</text:list>");
        }
        while stack.len() < lv + 1 {
            if let Some(top) = stack.last_mut()
                && !*top
            {
                out.push_str("<text:list-item>");
                *top = true;
            }
            if stack.is_empty() {
                out.push_str(&format!("<text:list xml:id=\"{id}\" text:style-name=\"{style}\"{cont}>"));
            } else {
                out.push_str("<text:list>");
            }
            stack.push(false);
        }
        if let Some(top) = stack.last_mut() {
            if *top {
                out.push_str("</text:list-item>");
            }
            *top = true;
        }
        let natural = counted.get(*lv).copied().flatten().map_or(starts.get(*lv).copied().unwrap_or(1), |v| v.saturating_add(1));
        let word = number(ctx, p).unwrap_or(natural);
        let from = if ordered.get(*lv) == Some(&true) && word != natural { format!(" text:start-value=\"{word}\"") } else { String::new() };
        for (l, c) in counted.iter_mut().enumerate().skip(*lv) {
            *c = (l == *lv).then_some(word);
        }
        out.push_str(&format!("<text:list-item{from}>"));
        text::para(ctx, p, out);
    }
    while let Some(open) = stack.pop() {
        if open {
            out.push_str("</text:list-item>");
        }
        out.push_str("</text:list>");
    }
    items.len().max(1)
}
