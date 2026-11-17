//! Typing timing: opens a document, types at the middle of it, and prints the layout time of
//! each keystroke (the pagination every edit pays for).
//!
//! `cargo run --release -p wordcraft-engine --example typing_bench -- doc.docx [keys]`

use serde_json::json;
use wordcraft_engine::Session;

fn main() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let path = args.first().ok_or("usage: typing_bench doc.docx [keys]")?;
    let keys: usize = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(40);
    let mut s = Session::new(wordcraft_engine::doc::Document::new());
    s.run("file.open", &json!({"path": path})).map_err(|e| e.to_string())?;
    let first = s.layout().ms;
    let mid = s.doc.body.len() / 2;
    let block = (mid..s.doc.body.len()).chain((0..mid).rev()).find(|&i| s.doc.body.get(i).and_then(|b| b.as_para()).is_some()).unwrap_or(mid);
    s.run("caret.set", &json!({"pos": {"story": "body", "path": [block], "off": 0}})).map_err(|e| e.to_string())?;
    let mut ms = Vec::with_capacity(keys);
    for _ in 0..keys {
        s.run("text.insert", &json!({"text": "a"})).map_err(|e| e.to_string())?;
        ms.push(s.layout().ms);
    }
    ms.sort_by(f64::total_cmp);
    let med = ms.get(ms.len() / 2).copied().unwrap_or(0.0);
    println!(
        "blocks {}, first layout {first:.1} ms, per key: median {med:.1} ms, min {:.1} ms",
        s.doc.body.len(),
        ms.first().copied().unwrap_or(0.0)
    );
    Ok(())
}
