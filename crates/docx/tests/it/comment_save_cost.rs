//! Saving large comment threads must not search all comments for each emitted ID.

use wordcraft_doc::{Comment, Document, Paragraph, Part, PartKind, para_block};

fn comments(n: u32, imported: bool) -> Document {
    let mut d = Document::from_text("Original comment save fixture.");
    for i in 0..n {
        // Sparse IDs and replies to a later comment also exercise paragraph-ID lookup order.
        let id = i * 3 + 7;
        d.parts.insert(id, Part { kind: PartKind::Comment, blocks: vec![para_block(Paragraph::new())], ..Default::default() });
        d.comments.insert(
            id,
            Comment { part: id, parent: (i + 1 < n).then_some(id + 3), durable_id: imported.then(|| format!("{:08X}", i + 1)), ..Default::default() },
        );
    }
    d
}

fn cpu_cost(mut f: impl FnMut()) -> f64 {
    let ran = || std::fs::read_to_string("/proc/thread-self/schedstat").ok()?.split_whitespace().next()?.parse::<u64>().ok();
    let (wall, before) = (std::time::Instant::now(), ran());
    f();
    match (before, ran()) {
        (Some(a), Some(b)) if b > a => (b - a) as f64 * 1e-9,
        _ => wall.elapsed().as_secs_f64(),
    }
}

#[test]
fn many_comments_save_without_quadratic_id_searches() {
    for imported in [true, false] {
        let small = comments(4_000, imported);
        let big = comments(32_000, imported);
        let save = |d: &Document| drop(std::hint::black_box(wordcraft_docx::write(d).unwrap()));
        save(&small);
        save(&big);
        let mut ratios = Vec::new();
        // Alternating inputs and thread CPU time leave scheduler delays out on Linux. Elsewhere
        // use elapsed time; retry ratios far from linear before declaring a regression.
        for _ in 0..3 {
            let (mut s, mut b) = (f64::INFINITY, f64::INFINITY);
            for _ in 0..3 {
                s = s.min(cpu_cost(|| (0..8).for_each(|_| save(&small))) / 8.0);
                b = b.min(cpu_cost(|| save(&big)));
            }
            let ratio = b / s.max(1e-9);
            eprintln!("comments imported={imported}: {s:.4}s -> {b:.4}s, ratio {ratio:.2}");
            if ratio < 24.0 {
                break;
            }
            ratios.push(ratio);
        }
        assert!(ratios.len() < 3, "8x comments took {ratios:?}x CPU: repeated ID searches");
    }
}
