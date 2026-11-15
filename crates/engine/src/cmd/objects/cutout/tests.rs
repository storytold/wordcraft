use super::*;
use serde_json::json;

/// A repeatable noise value in -0.5..0.5 for a pixel.
fn noise(x: u32, y: u32, seed: u32) -> f32 {
    let mut v = x.wrapping_mul(374_761_393) ^ y.wrapping_mul(668_265_263) ^ seed.wrapping_mul(2_246_822_519);
    v = (v ^ (v >> 13)).wrapping_mul(1_274_126_177);
    ((v ^ (v >> 16)) & 0xffff) as f32 / 65535.0 - 0.5
}

/// A photo-like picture: a noisy sky-to-haze gradient behind a noisy warm subject (with a darker
/// patch) wherever `inside` says, from fractions across and down.
fn photo(w: u32, h: u32, inside: impl Fn(f32, f32) -> bool) -> RgbaImage {
    RgbaImage::from_fn(w, h, |x, y| {
        let (fx, fy) = ((x as f32 + 0.5) / w as f32, (y as f32 + 0.5) / h as f32);
        let px = |c: [f32; 3], amp: f32| {
            let n = noise(x, y, 7) * amp;
            image::Rgba([(c[0] + n).clamp(0.0, 255.0) as u8, (c[1] + n * 0.8).clamp(0.0, 255.0) as u8, (c[2] + n * 0.6).clamp(0.0, 255.0) as u8, 255])
        };
        if inside(fx, fy) {
            if fy > 0.6 { px([120.0, 52.0, 40.0], 30.0) } else { px([205.0, 110.0, 70.0], 30.0) }
        } else {
            px([110.0 + 70.0 * fy, 160.0 + 50.0 * fy, 225.0 + 15.0 * fy], 24.0)
        }
    })
}

fn ellipse(cx: f32, cy: f32, rx: f32, ry: f32) -> impl Fn(f32, f32) -> bool {
    move |x, y| ((x - cx) / rx).powi(2) + ((y - cy) / ry).powi(2) <= 1.0
}

/// Intersection over union of the kept pixels (alpha over half) and the true subject.
fn iou(mask: &GrayImage, inside: impl Fn(f32, f32) -> bool) -> f32 {
    let (w, h) = mask.dimensions();
    let (mut both, mut either) = (0u32, 0u32);
    for (x, y, p) in mask.enumerate_pixels() {
        let truth = inside((x as f32 + 0.5) / w as f32, (y as f32 + 0.5) / h as f32);
        let kept = p.0[0] > 127;
        both += u32::from(truth && kept);
        either += u32::from(truth || kept);
    }
    both as f32 / either.max(1) as f32
}

fn alpha(mask: &GrayImage, fx: f32, fy: f32) -> u8 {
    let (w, h) = mask.dimensions();
    mask.get_pixel(((fx * w as f32) as u32).min(w - 1), ((fy * h as f32) as u32).min(h - 1)).0[0]
}

#[test]
fn a_subject_in_the_middle_of_a_photo_is_kept_and_its_background_removed() {
    let subject = ellipse(0.5, 0.52, 0.27, 0.36);
    let img = photo(300, 220, &subject);
    let mask = foreground(&img, &[]);
    assert_eq!(mask.dimensions(), (300, 220));
    let score = iou(&mask, &subject);
    assert!(score > 0.9, "IoU {score}");
    assert_eq!(alpha(&mask, 0.5, 0.5), 255, "the subject");
    assert_eq!(alpha(&mask, 0.5, 0.8), 255, "the subject's darker part");
    for (x, y) in [(0.02, 0.02), (0.98, 0.98), (0.1, 0.5), (0.9, 0.3)] {
        assert_eq!(alpha(&mask, x, y), 0, "background at {x} {y}");
    }
}

/// A larger picture is cut at the working size, then its edge refined at full size: the edge
/// follows the subject to within a few pixels, not in blocks of the working size.
#[test]
fn a_large_picture_keeps_a_fine_edge() {
    let subject = ellipse(0.5, 0.5, 0.3, 0.3);
    let img = photo(900, 900, &subject);
    let mask = foreground(&img, &[]);
    assert_eq!(mask.dimensions(), (900, 900));
    let score = iou(&mask, &subject);
    assert!(score > 0.93, "IoU {score}");
    // Across the edge on the middle row: the mask goes from kept to cleared within a few pixels
    // of the true edge (at 0.8 × 900 = 720).
    let row: Vec<u8> = (690..750).map(|x| mask.get_pixel(x, 450).0[0]).collect();
    let last_kept = row.iter().rposition(|&a| a > 200).map(|i| i + 690).unwrap_or(0);
    let first_clear = row.iter().position(|&a| a < 50).map(|i| i + 690).unwrap_or(0);
    assert!((710..=730).contains(&last_kept) && (712..=735).contains(&first_clear), "{last_kept} {first_clear}: {row:?}");
}

/// A portrait's shoulders run off the bottom of the picture: they're kept there too, not cut
/// off by the band the background starts from.
#[test]
fn a_subject_running_off_the_edge_is_kept_at_the_edge() {
    let bust = |x: f32, y: f32| ellipse(0.5, 0.35, 0.16, 0.22)(x, y) || ((0.22..=0.78).contains(&x) && y >= 0.55);
    let img = photo(240, 300, bust);
    let mask = foreground(&img, &[]);
    let score = iou(&mask, bust);
    assert!(score > 0.88, "IoU {score}");
    assert!(alpha(&mask, 0.5, 0.995) > 200, "the shoulders at the bottom edge: {}", alpha(&mask, 0.5, 0.995));
    assert_eq!(alpha(&mask, 0.05, 0.99), 0, "beside them");
    assert_eq!(alpha(&mask, 0.5, 0.03), 0, "above the head");
}

#[test]
fn marks_keep_and_remove_what_they_cover() {
    let subject = ellipse(0.5, 0.5, 0.3, 0.3);
    let img = photo(200, 200, &subject);
    // Remove the subject's top, keep a patch of sky.
    let marks =
        vec![Mark { keep: false, points: vec![[0.3, 0.3], [0.7, 0.3]], width: 0.05 }, Mark { keep: true, points: vec![[0.1, 0.85]], width: 0.05 }];
    let mask = foreground(&img, &marks);
    assert_eq!(alpha(&mask, 0.5, 0.3), 0, "under the remove mark");
    assert_eq!(alpha(&mask, 0.1, 0.85), 255, "under the keep mark");
    assert_eq!(alpha(&mask, 0.5, 0.6), 255, "the rest of the subject");
    // A later mark wins where two cross.
    let both = vec![Mark { keep: false, points: vec![[0.5, 0.5]], width: 0.05 }, Mark { keep: true, points: vec![[0.5, 0.5]], width: 0.05 }];
    assert_eq!(alpha(&foreground(&img, &both), 0.5, 0.5), 255);
}

#[test]
fn a_single_marked_pixel_survives_refinement_among_opposite_marks() {
    let img = RgbaImage::from_pixel(100, 100, image::Rgba([128, 128, 128, 255]));
    for keep in [true, false] {
        let marks = [Mark { keep: !keep, points: vec![[0.505, 0.505]], width: 0.05 }, Mark { keep, points: vec![[0.505, 0.505]], width: 0.002 }];
        let mask = foreground(&img, &marks);
        assert_eq!(mask.get_pixel(50, 50).0[0], if keep { 255 } else { 0 }, "the later mark wins, keep={keep}");
        assert_eq!(mask.get_pixel(51, 50).0[0], if keep { 0 } else { 255 }, "the surrounding mark still applies");
    }
}

#[test]
fn a_transparent_background_stays_transparent_and_marks_cannot_bring_it_back() {
    let subject = ellipse(0.5, 0.5, 0.25, 0.25);
    let mut img = photo(160, 160, &subject);
    for (x, y, p) in img.enumerate_pixels_mut() {
        if !subject((x as f32 + 0.5) / 160.0, (y as f32 + 0.5) / 160.0) {
            p.0[3] = 0;
        }
    }
    let keep_all = vec![Mark { keep: true, points: vec![[0.05, 0.05], [0.95, 0.95]], width: 0.05 }];
    for marks in [vec![], keep_all] {
        let mask = foreground(&img, &marks);
        assert_eq!(alpha(&mask, 0.05, 0.05), 0);
        assert_eq!(alpha(&mask, 0.5, 0.5), 255);
    }
}

#[test]
fn odd_sizes_and_flat_pictures_never_panic() {
    for (w, h) in [(1, 1), (2, 1), (3, 2), (1, 5000), (5000, 1), (7, 7), (40, 3)] {
        let img = RgbaImage::from_fn(w, h, |x, y| image::Rgba([(x * 7 % 256) as u8, (y * 3 % 256) as u8, 90, 255]));
        let marks = vec![Mark { keep: false, points: vec![[0.5, 0.5]], width: MAX_PEN }];
        assert_eq!(foreground(&img, &[]).dimensions(), (w, h));
        assert_eq!(foreground(&img, &marks).dimensions(), (w, h));
    }
    // Too small for a band and a middle: everything is kept.
    let tiny = RgbaImage::from_pixel(2, 2, image::Rgba([10, 200, 10, 255]));
    assert!(foreground(&tiny, &[]).pixels().all(|p| p.0[0] == 255));
    // One colour throughout: nothing tells the sides apart; whatever is chosen, it is quick.
    let flat = RgbaImage::from_pixel(300, 300, image::Rgba([250, 250, 250, 255]));
    assert_eq!(foreground(&flat, &[]).dimensions(), (300, 300));
    assert_eq!(GrayImage::new(0, 0), foreground(&RgbaImage::new(0, 0), &[]));
}

#[test]
fn working_sizes_keep_the_proportions_and_the_budget() {
    assert_eq!(fit(100, 50, 10_000), (100, 50));
    assert_eq!(fit(400, 200, 20_000), (200, 100));
    for (w, h) in [(1, 2_000_000), (2_000_000, 1), (3, 1_000_000), (u32::MAX, u32::MAX)] {
        let (a, b) = fit(w, h, WORK_PIXELS);
        assert!(a >= 1 && b >= 1 && u64::from(a) * u64::from(b) <= WORK_PIXELS, "{w}×{h} → {a}×{b}");
    }
}

#[test]
fn marks_are_read_from_params_and_junk_is_refused() {
    let ok = marks_from(&json!({"marks": [{"keep": true, "points": [[0.2, 0.3], [0.4, 0.5]]}, {"keep": false, "points": [[9e99, -3]], "width": 7}]}))
        .unwrap();
    assert_eq!(ok.len(), 2);
    assert_eq!(ok[0], Mark { keep: true, points: vec![[0.2, 0.3], [0.4, 0.5]], width: DEFAULT_PEN });
    assert_eq!(ok[1], Mark { keep: false, points: vec![[2.0, -1.0]], width: MAX_PEN }, "clamped");
    assert!(marks_from(&json!({})).unwrap().is_empty());
    assert!(marks_from(&json!({"marks": null})).unwrap().is_empty());
    for junk in [
        json!({"marks": 3}),
        json!({"marks": [{"points": [[0, 0]]}]}),
        json!({"marks": [{"keep": true}]}),
        json!({"marks": [{"keep": true, "points": []}]}),
        json!({"marks": [{"keep": true, "points": [[0]]}]}),
        json!({"marks": [{"keep": true, "points": [["a", 1]]}]}),
        json!({"marks": [{"keep": true, "points": [[0, 0, 0]]}]}),
        json!({"marks": [{"keep": "yes", "points": [[0, 0]]}]}),
        json!({"marks": [{"keep": true, "points": vec![[0.5, 0.5]; MAX_MARK_POINTS + 1]}]}),
        json!({"marks": vec![json!({"keep": true, "points": [[0, 0]]}); MAX_MARKS + 1]}),
    ] {
        assert!(matches!(marks_from(&junk), Err(CmdError::Params(_))), "{junk}");
    }
}

/// The most points, each stroke corner to corner at the widest pen, still finish.
#[test]
fn the_widest_longest_marks_are_bounded() {
    let img = photo(400, 400, ellipse(0.5, 0.5, 0.3, 0.3));
    let zigzag: Vec<[f32; 2]> = (0..MAX_MARK_POINTS).map(|i| if i % 2 == 0 { [-1.0, -1.0] } else { [2.0, 2.0] }).collect();
    let marks = vec![Mark { keep: false, points: zigzag, width: MAX_PEN }];
    let mask = foreground(&img, &marks);
    assert_eq!(alpha(&mask, 0.5, 0.5), 0, "the diagonal stroke crosses the middle");
}

#[test]
fn the_minimum_cut_takes_the_cheapest_labelling() {
    let cut = |pulls: [i64; 2], link: i64| {
        let mut g = Graph::new(2, 1);
        g.terminal(0, pulls[0]);
        g.terminal(1, pulls[1]);
        g.edge(0, 1, link, link);
        g.min_cut(&mut FLOW_BUDGET.clone())
    };
    assert_eq!(cut([10, -10], 3), vec![true, false], "a weak link is cut");
    assert_eq!(cut([10, -5], 30), vec![true, true], "a strong link pulls the weaker one over");
    assert_eq!(cut([5, -10], 30), vec![false, false]);
    // Out of budget: still one side for each pixel.
    let mut g = Graph::new(2, 1);
    g.terminal(0, 10);
    g.terminal(1, -10);
    g.edge(0, 1, 30, 30);
    assert_eq!(g.min_cut(&mut 0).len(), 2);
}

#[test]
fn the_cut_is_a_minimum_cut_on_random_grids() {
    let mut seed = 0x2545_f491u32;
    let mut rnd = |m: u32| {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        seed % m
    };
    for case in 0..400 {
        let (w, h) = (1 + rnd(7) as usize, 1 + rnd(7) as usize);
        let mut g = Graph::new(w * h, 0);
        for i in 0..w * h {
            g.terminal(i, rnd(41) as i64 - 20);
        }
        for y in 0..h {
            for x in 0..w {
                let i = y * w + x;
                let mut link = |j: usize| {
                    let c = rnd(25) as i64;
                    let d = if case % 3 == 0 { rnd(25) as i64 } else { c };
                    g.edge(i, j, c, d);
                };
                if x + 1 < w {
                    link(i + 1);
                }
                if y + 1 < h {
                    link(i + w);
                    if x + 1 < w {
                        link(i + w + 1);
                    }
                    if x > 0 {
                        link(i + w - 1);
                    }
                }
            }
        }
        let (tr, edges) = (g.tr.clone(), g.edges.clone());
        let side = g.min_cut(&mut FLOW_BUDGET.clone());
        assert_eq!(cut_cost(&tr, &edges, &side), reference_max_flow(&tr, &edges), "case {case}: {w}×{h}");
    }
}

/// Cost of a labelling in the original graph, before the production solver changes capacities.
fn cut_cost(tr: &[i64], edges: &[(u32, u32, i64, i64)], side: &[bool]) -> i64 {
    let terminals: i64 = tr.iter().zip(side).map(|(&pull, &source)| if source { (-pull).max(0) } else { pull.max(0) }).sum();
    terminals
        + edges
            .iter()
            .map(|&(a, b, ab, ba)| match (side[a as usize], side[b as usize]) {
                (true, false) => ab,
                (false, true) => ba,
                _ => 0,
            })
            .sum::<i64>()
}

/// Independent Edmonds–Karp reference for the small random graphs above. A dense residual
/// matrix and breadth-first augmenting paths avoid sharing the production solver's tree logic.
fn reference_max_flow(tr: &[i64], edges: &[(u32, u32, i64, i64)]) -> i64 {
    let (source, sink) = (tr.len(), tr.len() + 1);
    let nodes = tr.len() + 2;
    let mut residual = vec![vec![0i64; nodes]; nodes];
    for (i, &pull) in tr.iter().enumerate() {
        residual[source][i] = pull.max(0);
        residual[i][sink] = (-pull).max(0);
    }
    for &(a, b, ab, ba) in edges {
        residual[a as usize][b as usize] += ab;
        residual[b as usize][a as usize] += ba;
    }
    let mut flow = 0;
    loop {
        let mut parent = vec![usize::MAX; nodes];
        parent[source] = source;
        let mut queue = std::collections::VecDeque::from([source]);
        while let Some(u) = queue.pop_front() {
            for (v, &capacity) in residual[u].iter().enumerate() {
                if capacity > 0 && parent[v] == usize::MAX {
                    parent[v] = u;
                    queue.push_back(v);
                }
            }
        }
        if parent[sink] == usize::MAX {
            return flow;
        }
        let mut amount = i64::MAX;
        let mut v = sink;
        while v != source {
            let u = parent[v];
            amount = amount.min(residual[u][v]);
            v = u;
        }
        v = sink;
        while v != source {
            let u = parent[v];
            residual[u][v] -= amount;
            residual[v][u] += amount;
            v = u;
        }
        flow += amount;
    }
}
