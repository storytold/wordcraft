//! Large-document benchmark (#554): a synthetic book of about N pages built in code (long
//! paragraphs, headings, pictures, tables, footnotes, fields) timed through the engine the way
//! the app uses it: open, first layout, a keystroke at the start / middle / end (command plus
//! incremental re-layout), rendering visible pages (scrolling), save and PDF export.
//!
//! `cargo xtask bench [--pages N] [--keys K]` runs it in release mode at low priority, or
//! directly: `cargo run --release -p wordcraft-engine --example large_doc_bench -- --pages 500`.
//! Times are wall clock in milliseconds (medians where a step repeats); the counters (paragraphs
//! laid out, pages) are what the regression tests pin.

use std::time::Instant;

use serde_json::json;
use wordcraft_engine::Session;
use wordcraft_engine::doc::{Block, Path, Pos, StoryRef};

fn arg(name: &str, default: usize) -> usize {
    let args: Vec<String> = std::env::args().collect();
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).and_then(|v| v.parse().ok()).unwrap_or(default)
}

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1000.0
}

fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(|a, b| a.total_cmp(b));
    v.get(v.len() / 2).copied().unwrap_or(0.0)
}

fn row(what: &str, v: f64, note: &str) {
    println!("| {what:<44} | {v:>10.1} | {note}");
}

/// The top-level body paragraph nearest `block` (searching forward, then back).
fn para_near(s: &Session, block: usize) -> Option<usize> {
    let n = s.doc.body.len();
    let is_text = |i: usize| matches!(s.doc.body.get(i).map(|b| &**b), Some(Block::Para(p)) if p.len() > 40 && p.objects.is_empty());
    (block..n).find(|&i| is_text(i)).or_else(|| (0..block.min(n)).rev().find(|&i| is_text(i)))
}

fn main() -> Result<(), String> {
    let pages = arg("--pages", 500);
    let keys = arg("--keys", 5).max(1);
    println!("WordCraft large-document benchmark: target {pages} pages, {keys} keystrokes per position");
    let t = Instant::now();
    let doc = wordcraft_engine::sample::large_document(pages);
    let built = ms(t);
    let pictures = doc.media.len();
    let tables = doc.body.iter().filter(|b| matches!(***b, Block::Table(_))).count();
    let notes = doc.parts.values().filter(|p| p.kind == wordcraft_engine::doc::PartKind::Footnote).count();
    println!("| step                                         |         ms | notes");
    println!("|----------------------------------------------|-----------:|------");
    row("build document in code", built, &format!("{} body blocks, {pictures} pictures, {tables} tables, {notes} footnotes", doc.body.len()));

    let t = Instant::now();
    let bytes = wordcraft_engine::io::save_bytes("bench.docx", &doc)?;
    row("save .docx", ms(t), &format!("{:.1} MB", bytes.len() as f64 / 1e6));
    let t = Instant::now();
    let opened = wordcraft_engine::io::open_bytes("bench.docx", &bytes)?;
    row("open .docx", ms(t), "");

    let mut s = Session::new(opened);
    let t = Instant::now();
    let lay = s.layout();
    let first = ms(t);
    let n_pages = lay.pages.len();
    row("first layout", first, &format!("{n_pages} pages"));
    let t = Instant::now();
    let _ = s.layout();
    row("layout again (unchanged)", ms(t), "");

    // Keystrokes: type `keys` letters at the start, middle and end of the body; each is the
    // command plus the re-layout the next frame asks for.
    for (label, at_page) in [("start", 0usize), ("middle", n_pages / 2), ("end", n_pages.saturating_sub(1))] {
        let lay = s.layout();
        let block = lay.pages.get(at_page).map(|p| p.first_block).unwrap_or(0);
        let Some(i) = para_near(&s, block) else { continue };
        s.sel = wordcraft_engine::Selection::caret(Pos { story: StoryRef::Body, path: Path::top(i), off: 10 });
        let (mut cmd, mut relay, mut misses, mut placed) = (Vec::new(), Vec::new(), 0u64, 0usize);
        for _ in 0..keys {
            let t = Instant::now();
            s.run("text.insert", &json!({"text": "x"})).map_err(|e| e.to_string())?;
            cmd.push(ms(t));
            let before = s.layout_cache_stats().1;
            let t = Instant::now();
            placed = s.layout().blocks_placed;
            relay.push(ms(t));
            misses = s.layout_cache_stats().1.saturating_sub(before);
        }
        let total: Vec<f64> = cmd.iter().zip(&relay).map(|(a, b)| a + b).collect();
        row(&format!("keystroke at {label} (command)"), median(cmd), "");
        row(
            &format!("keystroke at {label} (re-layout)"),
            median(relay),
            &format!("{misses} paragraphs laid out, {placed} of {} blocks placed", s.doc.body.len()),
        );
        row(&format!("keystroke at {label} (total)"), median(total), &format!("page {}", at_page + 1));
    }

    // For comparison: a pass that places every block again (what each keystroke cost before
    // incremental pagination), its paragraphs all from the cache.
    {
        let opts = wordcraft_engine::layout::LayoutOptions { proofing: true, ..Default::default() };
        let mut cache = wordcraft_engine::layout::LayoutCache::new();
        let _ = wordcraft_engine::layout::layout(&s.doc, &mut cache, &opts);
        let mut times = Vec::new();
        for _ in 0..3 {
            let mut d = s.doc.clone();
            d.body = d.body.iter().map(|b| std::sync::Arc::new((**b).clone())).collect();
            let t = Instant::now();
            let l = wordcraft_engine::layout::layout(&d, &mut cache, &opts);
            times.push(ms(t));
            let _ = l.blocks_placed;
        }
        row("re-layout placing every block (warm cache)", median(times), "the pre-#554 keystroke path, for comparison");
    }

    // Caret and selection geometry the frame needs after a keystroke.
    let lay = s.layout();
    let t = Instant::now();
    let caret = lay.caret_on(&s.sel.focus, s.page_hint);
    row("caret geometry at the end", ms(t), if caret.is_some() { "" } else { "no caret" });

    // Rendering the visible pages: three pages at the middle (first time decodes pictures),
    // the same again (cached), then scrolling through ten pages.
    let opts = wordcraft_engine::render::RenderOptions::default();
    let scale = 1.5;
    let mid = n_pages / 2;
    let span: Vec<usize> = (mid..(mid + 3).min(n_pages)).collect();
    for label in ["render 3 visible pages (cold)", "render 3 visible pages (warm)"] {
        let t = Instant::now();
        for &p in &span {
            if let Some(page) = lay.pages.get(p) {
                let _ = wordcraft_engine::render::render_page(&s.doc, page, scale, &opts);
            }
        }
        row(label, ms(t), &format!("scale {scale}"));
    }
    let scroll: Vec<usize> = (n_pages / 4..(n_pages / 4 + 10).min(n_pages)).collect();
    let mut per = Vec::new();
    for &p in &scroll {
        if let Some(page) = lay.pages.get(p) {
            let t = Instant::now();
            let _ = wordcraft_engine::render::render_page(&s.doc, page, scale, &opts);
            per.push(ms(t));
        }
    }
    row("scroll: render one page (median of 10)", median(per), "");

    let t = Instant::now();
    let bytes = wordcraft_engine::io::save_bytes("bench.docx", &s.doc)?;
    row("save .docx after edits", ms(t), &format!("{:.1} MB", bytes.len() as f64 / 1e6));
    let t = Instant::now();
    let pdf = wordcraft_engine::io::save_bytes("bench.pdf", &s.doc)?;
    row("export PDF", ms(t), &format!("{:.1} MB", pdf.len() as f64 / 1e6));
    Ok(())
}
