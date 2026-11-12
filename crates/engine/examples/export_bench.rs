//! Export timing: saves a long generated document (the built-in sample document repeated) through
//! the real save path, the way `file.save` / `file.export` write it, and prints wall times.
//!
//! `cargo run --release -p wordcraft-engine --example export_bench -- [ext] [copies] [runs] [out]`
//!
//! Defaults: `pdf`, 40 copies (about 100 pages), 7 runs. With `out` the last run's bytes are
//! written there, to compare the output of two builds.

use std::sync::Arc;
use std::time::Instant;

use wordcraft_engine::doc::props::CharProps;
use wordcraft_engine::doc::{Block, Document};
use wordcraft_engine::{io, sample};

/// The sample document's body `copies` times, each paragraph ending in its copy number so no two
/// copies share text (as the chapters of a real long document don't).
fn long_document(copies: usize) -> Document {
    let mut d = sample::sample_document();
    // A fixed creation date: the PDF's dates (and the ID derived from them) stay the same from run to run.
    d.core.created = "2026-01-01T00:00:00Z".into();
    let one: Vec<Block> = d.body.iter().map(|b| (**b).clone()).collect();
    d.body.clear();
    for i in 0..copies {
        for b in &one {
            let mut b = b.clone();
            if let Block::Para(p) = &mut b {
                let at = p.len();
                let _ = p.insert_text(at, &format!(" {i}"), &CharProps::default());
            }
            d.body.push(Arc::new(b));
        }
    }
    d
}

fn main() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let ext = args.first().map_or("pdf", String::as_str);
    let copies = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(40);
    let runs = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(7usize).max(1);
    let doc = long_document(copies);
    let name = format!("bench.{ext}");
    let mut ms = Vec::with_capacity(runs);
    let mut bytes = Vec::new();
    for _ in 0..runs {
        let t = Instant::now();
        bytes = io::save_bytes(&name, &doc)?;
        ms.push(t.elapsed().as_secs_f64() * 1000.0);
    }
    let mut sorted = ms.clone();
    sorted.sort_by(f64::total_cmp);
    let median = sorted.get(sorted.len() / 2).copied().unwrap_or_default();
    let min = sorted.first().copied().unwrap_or_default();
    println!(
        "{ext}: {copies} copies, {} bytes, runs {:?} ms, median {median:.1} ms, min {min:.1} ms",
        bytes.len(),
        ms.iter().map(|m| m.round()).collect::<Vec<_>>()
    );
    if let Some(out) = args.get(3) {
        std::fs::write(out, &bytes).map_err(|e| e.to_string())?;
    }
    Ok(())
}
