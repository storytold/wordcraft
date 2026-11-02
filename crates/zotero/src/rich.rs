//! Zotero's citation text into document content. Rich text arrives as RTF (`{\rtf …}`) or HTML;
//! only character formatting that belongs to a citation style is kept (italic, bold, small
//! caps, super/subscript, underline, links), on top of the formatting where the field sits.

use wordcraft_doc::edit::Fragment;
use wordcraft_doc::para::OBJ;
use wordcraft_doc::{Block, CharProps, Document, Paragraph, Run};

/// Most paragraphs taken from one text (a bibliography of thousands of entries is fine).
const MAX_PARAS: usize = 100_000;
/// Most formatting runs taken from one text; past it, text keeps the formatting before it.
pub(crate) const MAX_RUNS: usize = 1_000_000;

fn style_props(base: &CharProps, src: &CharProps) -> CharProps {
    let mut p = base.clone();
    if src.bold.is_some() {
        p.bold = src.bold;
    }
    if src.italic.is_some() {
        p.italic = src.italic;
    }
    if src.underline.is_some() {
        p.underline = src.underline;
    }
    if src.strike.is_some() {
        p.strike = src.strike;
    }
    if src.small_caps.is_some() {
        p.small_caps = src.small_caps;
    }
    if src.caps.is_some() {
        p.caps = src.caps;
    }
    if src.vert_align.is_some() {
        p.vert_align = src.vert_align;
    }
    if src.link.is_some() {
        p.link = src.link.clone();
    }
    p
}

/// The body paragraphs of `d` (Zotero's RTF or HTML, read), at most [`MAX_PARAS`], formatted on
/// top of `base` with the run styling they carry (past [`MAX_RUNS`] runs, text keeps the
/// formatting before it; each paragraph still needs its first run, so at most [`MAX_PARAS`] more). Objects are dropped.
fn from_doc(d: &Document, base: &CharProps) -> Vec<Paragraph> {
    let mut out = Vec::new();
    let mut runs = 0usize;
    for b in d.body.iter().take(MAX_PARAS) {
        let Some(src) = b.as_para() else { continue };
        let mut p = Paragraph::new();
        p.mark = base.clone();
        // Appended run by run, merging only with the run before; normalized once at the end.
        for (range, props) in src.run_ranges() {
            let Some(t) = src.text.get(range) else { continue };
            let t: String = t.chars().filter(|c| *c != OBJ).collect();
            if t.is_empty() {
                continue;
            }
            let props = style_props(base, props);
            p.text.push_str(&t);
            match p.runs.last_mut() {
                Some(last) if last.props == props || runs >= MAX_RUNS => last.len += t.len(),
                _ => {
                    p.runs.push(Run { len: t.len(), props });
                    runs += 1;
                }
            }
        }
        p.normalize();
        p.touch();
        out.push(p);
    }
    out
}

fn plain(text: &str, base: &CharProps) -> Vec<Paragraph> {
    text.split('\n')
        .take(MAX_PARAS)
        .map(|l| {
            let mut p = Paragraph::with_text(l.trim_end_matches('\r'), base.clone());
            p.mark = base.clone();
            p
        })
        .collect()
}

/// `text` as paragraphs formatted on top of `base`. Never empty.
pub fn fragment(text: &str, rich: bool, base: &CharProps) -> Fragment {
    let t = text.trim_start();
    let mut paras = if rich && t.starts_with("{\\rtf") {
        match wordcraft_formats::rtf::import(t.as_bytes()) {
            Ok(d) => from_doc(&d, base),
            Err(e) => {
                log::warn!("Zotero sent RTF we could not read ({e}); inserting it as text");
                plain(text, base)
            }
        }
    } else if rich && t.contains('<') {
        from_doc(&wordcraft_formats::html::import(text), base)
    } else {
        plain(text, base)
    };
    while paras.len() > 1 && paras.last().is_some_and(|p| p.text.trim().is_empty()) {
        paras.pop();
    }
    if paras.is_empty() {
        paras.push(Paragraph::new());
    }
    Fragment { blocks: paras.into_iter().map(Block::Para).collect(), ..Default::default() }
}
