use super::*;

mod compare_stage0;

#[test]
fn compare_marks_changes() {
    let mut s = Session::new(Document::from_text("The cat sat.\nKeep this.\nOld line."));
    let r = s.run("review.compare", &json!({"text": "The dog sat.\nKeep this.\nNew line here."})).unwrap();
    assert!(r["changes"].as_u64().unwrap() >= 2);
    s.run("review.acceptAll", &json!({})).unwrap();
    assert_eq!(s.doc.plain_text(StoryRef::Body), "The dog sat.\nKeep this.\nNew line here.");
}

#[test]
fn compare_options_ignore_case_and_white_space() {
    let original = "The Cat sat.\nTwo  spaces here.";
    let revised = "The cat sat.\nTwo spaces here.";
    // By default both differences are changes.
    let mut s = Session::new(Document::from_text(original));
    let r = s.run("review.compare", &json!({"text": revised, "author": "Reviewer"})).unwrap();
    assert_eq!(r["changes"], 2);
    assert!(s.doc.revisions.iter().any(|x| x.author == "Reviewer"));
    // Ignoring case and white space: no changes, and the result reads as the revised text.
    let mut s = Session::new(Document::from_text(original));
    let r = s.run("review.compare", &json!({"text": revised, "caseChanges": false, "whiteSpace": false})).unwrap();
    assert_eq!(r["changes"], 0);
    assert_eq!(s.doc.plain_text(StoryRef::Body), revised);
    // Character level marks only the changed letters.
    let mut s = Session::new(Document::from_text("colour"));
    s.run("review.compare", &json!({"text": "color", "level": "character"})).unwrap();
    let p = s.doc.body[0].as_para().unwrap();
    let deleted: String = p.run_ranges().filter(|(_, c)| c.del.is_some()).map(|(r, _)| p.text[r].to_string()).collect();
    assert_eq!(deleted, "u");
    // Two dates that happen to share digits in a different order ("01.09.2026" vs
    // "19.10.2026") must come out as one clean replacement, not a scrambled chain of
    // single-character deletions and insertions wherever a digit coincidentally recurs.
    let mut s = Session::new(Document::from_text("zawarta w dniu 01.09.2026 pomiędzy:"));
    s.run("review.compare", &json!({"text": "zawarta w dniu 19.10.2026 pomiędzy:", "level": "character"})).unwrap();
    let p = s.doc.body[0].as_para().unwrap();
    let deleted: String = p.run_ranges().filter(|(_, c)| c.del.is_some()).map(|(r, _)| p.text[r].to_string()).collect();
    let inserted: String = p.run_ranges().filter(|(_, c)| c.ins.is_some()).map(|(r, _)| p.text[r].to_string()).collect();
    assert_eq!(deleted, "01.09");
    assert_eq!(inserted, "19.10");
    assert_eq!(p.run_ranges().filter(|(_, c)| c.del.is_some()).count(), 1, "one clean deleted run, not several scattered ones");
    assert_eq!(p.run_ranges().filter(|(_, c)| c.ins.is_some()).count(), 1, "one clean inserted run, not several scattered ones");
    // Formatting differences become formatting changes.
    let mut s = Session::new(Document::from_text("Same words."));
    let mut rev = Document::from_text("Same words.");
    rev.format_range(&rev.start_of(StoryRef::Body), &rev.end_of(StoryRef::Body), &|c| c.bold = Some(true)).unwrap();
    let path = std::env::temp_dir().join(format!("wc-compare-{}.docx", std::process::id()));
    crate::io::save_path(&path, &rev).unwrap();
    let r = s.run("review.compare", &json!({"path": path.to_string_lossy()})).unwrap();
    let _ = std::fs::remove_file(&path);
    assert_eq!(r["changes"], 1);
    assert!(s.doc.revisions.iter().any(|x| x.kind == RevisionKind::Format));
}

#[test]
fn collapse_noise_matches_merges_short_interior_runs_but_keeps_prefix_suffix() {
    // "01.09" vs "19.10": a pure LCS finds a scrambled chain of single-character matches
    // ('1', '.', '0') threaded through what a reader expects to be one clean replacement.
    let ops = lcs_ops(&['0', '1', '.', '0', '9'], &['1', '9', '.', '1', '0'], &mut { u64::MAX });
    assert!(ops.contains(&Op::Same), "the raw LCS does find spurious matches here, or this test proves nothing");
    assert_eq!(collapse_noise_matches(&ops), vec![Op::Del, Op::Del, Op::Del, Op::Del, Op::Del, Op::Ins, Op::Ins, Op::Ins, Op::Ins, Op::Ins]);
    // A plain single-character replacement flanked by a real common prefix and suffix has no
    // interior match to begin with (the DP's middle is pure delete/insert), so it's untouched.
    let ops = lcs_ops(&['a', 'b', 'c', 'x', 'd', 'e', 'f'], &['a', 'b', 'c', 'y', 'd', 'e', 'f'], &mut { u64::MAX });
    assert_eq!(collapse_noise_matches(&ops), ops);
}

/// Verifies that `lcs_ops` chunks sequence alignment across multiple `MAX_LCS` windows,
/// replaying operations to prove exact reconstruction and asserting specific edit indices.
#[test]
fn lcs_ops_chunks_across_multiple_windows() {
    // Two sequences of 9,000 elements (spanning three 4,000-element MAX_LCS windows).
    // Element at 50 moves to 8,500. Substitutions at 0 and 8,999 zero the outer prefix/suffix.
    let total = 9000;
    let mut a: Vec<usize> = (0..total).collect();
    let mut b = a.clone();
    a[0] = 99999;
    b[0] = 88888;
    a[total - 1] = 77777;
    b[total - 1] = 66666;
    let moved = a[50];
    b.remove(50);
    b.insert(8500, moved);

    let mut budget = u64::MAX;
    let ops = lcs_ops(&a, &b, &mut budget);

    // Replay ops against `a` and `b` to verify exact sequence reconstruction and order.
    let mut ia = 0;
    let mut ib = 0;
    let mut dels_seen = Vec::new();
    let mut inss_seen = Vec::new();
    let mut reconstructed = Vec::new();

    for op in &ops {
        match op {
            Op::Same => {
                assert_eq!(a.get(ia), b.get(ib), "Same op must match elements at ia={ia}, ib={ib}");
                if let Some(&val) = a.get(ia) {
                    reconstructed.push(val);
                }
                ia += 1;
                ib += 1;
            }
            Op::Del => {
                if let Some(&val) = a.get(ia) {
                    dels_seen.push((ia, val));
                }
                ia += 1;
            }
            Op::Ins => {
                if let Some(&val) = b.get(ib) {
                    inss_seen.push((ib, val));
                    reconstructed.push(val);
                }
                ib += 1;
            }
        }
    }

    assert_eq!(ia, a.len());
    assert_eq!(ib, b.len());
    assert_eq!(reconstructed, b);
    // Truth: 2 substitutions (indices 0 and 8999), 1 deletion at 50, 1 insertion at 8500.
    assert_eq!(dels_seen, vec![(0, 99999), (50, 50), (8999, 77777)]);
    assert_eq!(inss_seen, vec![(0, 88888), (8500, 50), (8999, 66666)]);
}

/// Verifies that `lcs_ops` forces successive full 4,000-element windows without
/// intermediate prefix trimming collapsing them, asserting exact budget exhaustion.
#[test]
fn lcs_ops_forces_successive_full_windows_without_intermediate_prefix_trim() {
    // Force successive full 4,000-element windows without intermediate prefix trimming
    // collapsing the middle into fewer tables.
    // Length: 9,000 elements across 3 windows (4,000 + 4,000 + 1,000).
    // Boundary substitutions at indices 0, 4000, 8000, and 8999 guarantee rem_a[0] != rem_b[0]
    // at the beginning of each window, forcing intermediate prefix trim to find p == 0 so
    // all three full DP tables must execute.
    let total = 9000;
    let mut a: Vec<usize> = (0..total).collect();
    let mut b = a.clone();
    a[0] = 11111;
    b[0] = 22222;
    a[4000] = 33333;
    b[4000] = 44444;
    a[8000] = 55555;
    b[8000] = 66666;
    a[total - 1] = 77777;
    b[total - 1] = 88888;

    // Budget for 3 full windows:
    // Window 1: 4,000 * 4,000 = 16,000,000 cells
    // Window 2: 4,000 * 4,000 = 16,000,000 cells
    // Window 3: 1,000 * 1,000 = 1,000,000 cells
    // Total cells = 33,000,000.
    let mut budget = 33_000_000u64;
    let ops = lcs_ops(&a, &b, &mut budget);

    // Exact budget is fully exhausted (budget == 0), proving all 3 full DP tables were built.
    assert_eq!(budget, 0, "all 3 successive windows must execute their DP tables without collapsing");

    let mut ia = 0;
    let mut ib = 0;
    let mut reconstructed = Vec::new();
    let mut dels_seen = Vec::new();
    let mut inss_seen = Vec::new();

    for op in &ops {
        match op {
            Op::Same => {
                assert_eq!(a.get(ia), b.get(ib));
                if let Some(&val) = a.get(ia) {
                    reconstructed.push(val);
                }
                ia += 1;
                ib += 1;
            }
            Op::Del => {
                if let Some(&val) = a.get(ia) {
                    dels_seen.push((ia, val));
                }
                ia += 1;
            }
            Op::Ins => {
                if let Some(&val) = b.get(ib) {
                    inss_seen.push((ib, val));
                    reconstructed.push(val);
                }
                ib += 1;
            }
        }
    }

    assert_eq!(ia, a.len());
    assert_eq!(ib, b.len());
    assert_eq!(reconstructed, b);
    assert_eq!(dels_seen, vec![(0, 11111), (4000, 33333), (8000, 55555), (8999, 77777)]);
    assert_eq!(inss_seen, vec![(0, 22222), (4000, 44444), (8000, 66666), (8999, 88888)]);

    // Also verify completely disjoint sequences across multiple windows with zero common items.
    // Window 1: 4,000 * 4,000 = 16,000,000 cells (deletes 4,000 of a_diff)
    // Window 2: 4,000 * 4,000 = 16,000,000 cells (deletes next 4,000 of a_diff)
    // Window 3: 1,000 of a_diff * 4,000 of b_diff = 4,000,000 cells
    // Total cells = 36,000,000.
    let a_diff: Vec<usize> = (0..9000).collect();
    let b_diff: Vec<usize> = (10_000..19_000).collect();
    let mut budget_diff = 36_000_000u64;
    let ops_diff = lcs_ops(&a_diff, &b_diff, &mut budget_diff);
    assert_eq!(budget_diff, 0, "all 3 windows in disjoint comparison must exhaust exact 36,000,000 budget");
    assert_eq!(ops_diff.iter().filter(|o| **o == Op::Del).count(), 9000);
    assert_eq!(ops_diff.iter().filter(|o| **o == Op::Ins).count(), 9000);
    assert_eq!(ops_diff.iter().filter(|o| **o == Op::Same).count(), 0);
}

/// Tests `lcs_ops` budget behavior across edge cases: zero initial budget, exact-fit allocation,
/// one-short exhaustion, and multi-window exhaustion after the first chunk.
#[test]
fn lcs_ops_exhausted_budget_falls_back_whole() {
    // 1. Budget 0 before any table: middle degrades whole, budget remains 0
    let a = vec![1, 2, 3, 4, 5];
    let b = vec![1, 8, 9, 5];
    let mut budget = 0;
    let ops = lcs_ops(&a, &b, &mut budget);
    assert_eq!(ops, vec![Op::Same, Op::Del, Op::Del, Op::Del, Op::Ins, Op::Ins, Op::Same]);
    assert_eq!(budget, 0, "budget must remain 0");

    // 2. Exact-fit: cells == budget
    // Middle has 3 elements in a (2, 3, 4) and 2 in b (8, 9) -> 3 * 2 = 6 cells.
    let mut budget_exact = 6;
    let ops_exact = lcs_ops(&a, &b, &mut budget_exact);
    assert_eq!(budget_exact, 0, "budget exactly exhausted to 0");
    assert_eq!(ops_exact, vec![Op::Same, Op::Del, Op::Del, Op::Del, Op::Ins, Op::Ins, Op::Same]);

    // Exact-fit with shared elements in middle
    let a_mid = vec![1, 2, 3, 5];
    let b_mid = vec![1, 2, 9, 5];
    // Middle is (3) vs (9): 1 * 1 = 1 cell
    let mut budget_mid = 1;
    let ops_mid = lcs_ops(&a_mid, &b_mid, &mut budget_mid);
    assert_eq!(budget_mid, 0);
    assert_eq!(ops_mid, vec![Op::Same, Op::Same, Op::Del, Op::Ins, Op::Same]);

    // 3. One-short: budget == cells - 1
    let mut budget_short = 5;
    let ops_short = lcs_ops(&a, &b, &mut budget_short);
    assert_eq!(budget_short, 5, "one-short budget must not be charged");
    assert_eq!(ops_short, vec![Op::Same, Op::Del, Op::Del, Op::Del, Op::Ins, Op::Ins, Op::Same]);

    // 4. Multi-window: Exhaustion after the first window
    // Two windows: window 1 has 4,000 elements (16,000,000 cells), window 2 has 500 elements (250,000 cells).
    let mut a_multi: Vec<usize> = (0..4500).collect();
    let mut b_multi = a_multi.clone();
    a_multi[0] = 99999;
    b_multi[0] = 88888;
    a_multi[4000] = 77777;
    b_multi[4000] = 66666;
    a_multi[4499] = 55555;
    b_multi[4499] = 44444;

    // Case 4A: budget fits window 1 exactly (16,000,000), exhausts for window 2
    let mut budget_w1_only = 16_000_000u64;
    let ops_w1_only = lcs_ops(&a_multi, &b_multi, &mut budget_w1_only);
    assert_eq!(budget_w1_only, 0, "budget spent on window 1, 0 remaining for window 2");
    // Window 1 aligned via DP table, window 2 fell back to 500 Dels and 500 Inss
    let tail_dels = ops_w1_only.iter().rev().skip(500).take(500).filter(|o| **o == Op::Del).count();
    let tail_inss = ops_w1_only.iter().rev().take(500).filter(|o| **o == Op::Ins).count();
    assert_eq!(tail_dels, 500);
    assert_eq!(tail_inss, 500);

    // Case 4B: budget fits both windows exactly: 16,250,000
    let mut budget_both = 16_250_000u64;
    let ops_both = lcs_ops(&a_multi, &b_multi, &mut budget_both);
    assert_eq!(budget_both, 0, "both windows charged exactly to 0");
    assert_eq!(ops_both.iter().filter(|o| **o == Op::Del).count(), 3);
    assert_eq!(ops_both.iter().filter(|o| **o == Op::Ins).count(), 3);

    // Case 4C: one short on window 2: 16_249_999
    let mut budget_both_short = 16_249_999u64;
    let _ = lcs_ops(&a_multi, &b_multi, &mut budget_both_short);
    assert_eq!(budget_both_short, 249_999, "remaining budget must reflect window 1 spent and window 2 unspent");
}

/// Tests boundary conditions for `lcs_ops`: empty slices, one-sided empty, exact `MAX_LCS`,
/// `MAX_LCS + 1`, `2 * MAX_LCS`, and prefix-trim loop break paths.
#[test]
fn lcs_ops_boundary_cases_and_break_paths() {
    // 1. Empty sequences
    let mut b = u64::MAX;
    assert_eq!(lcs_ops::<i32>(&[], &[], &mut b), Vec::<Op>::new());

    // 2. One-sided empty
    let items = [1, 2, 3];
    assert_eq!(lcs_ops(&[], &items, &mut b), vec![Op::Ins, Op::Ins, Op::Ins]);
    assert_eq!(lcs_ops(&items, &[], &mut b), vec![Op::Del, Op::Del, Op::Del]);

    // 3. Exact MAX_LCS (4,000)
    let len_max = 4000;
    let mut a_max: Vec<usize> = (0..len_max).collect();
    let mut b_max = a_max.clone();
    a_max[0] = 99999;
    b_max[0] = 88888;
    a_max[len_max - 1] = 77777;
    b_max[len_max - 1] = 66666;
    let mut b_budget = u64::MAX;
    let ops_max = lcs_ops(&a_max, &b_max, &mut b_budget);
    assert_eq!(ops_max.len(), len_max + 2);
    assert_eq!(ops_max.iter().filter(|o| **o == Op::Same).count(), len_max - 2);

    // 4. Exact MAX_LCS + 1 (4,001)
    let len_plus1 = 4001;
    let mut a_p1: Vec<usize> = (0..len_plus1).collect();
    let mut b_p1 = a_p1.clone();
    a_p1[0] = 99999;
    b_p1[0] = 88888;
    a_p1[len_plus1 - 1] = 77777;
    b_p1[len_plus1 - 1] = 66666;
    let mut b_budget2 = u64::MAX;
    let ops_p1 = lcs_ops(&a_p1, &b_p1, &mut b_budget2);
    assert_eq!(ops_p1.len(), len_plus1 + 2);
    assert_eq!(ops_p1.iter().filter(|o| **o == Op::Same).count(), len_plus1 - 2);

    // 5. Exact 2 * MAX_LCS (8,000)
    let len_2m = 8000;
    let mut a_2m: Vec<usize> = (0..len_2m).collect();
    let mut b_2m = a_2m.clone();
    a_2m[0] = 11111;
    b_2m[0] = 22222;
    a_2m[4000] = 33333;
    b_2m[4000] = 44444;
    a_2m[len_2m - 1] = 55555;
    b_2m[len_2m - 1] = 66666;
    let mut b_budget3 = u64::MAX;
    let ops_2m = lcs_ops(&a_2m, &b_2m, &mut b_budget3);
    assert_eq!(ops_2m.iter().filter(|o| **o == Op::Del).count(), 3);
    assert_eq!(ops_2m.iter().filter(|o| **o == Op::Ins).count(), 3);
    assert_eq!(ops_2m.iter().filter(|o| **o == Op::Same).count(), len_2m - 3);

    // 6. Prefix-trim break paths inside the while loop:
    // Case 6A: rem_a becomes empty after matching p elements
    let mut a_break_a: Vec<usize> = (0..4005).collect();
    let mut b_break_a: Vec<usize> = (0..4008).collect();
    a_break_a[0] = 99999;
    b_break_a[0] = 88888;
    let mut b_break1 = u64::MAX;
    let ops_break_a = lcs_ops(&a_break_a, &b_break_a, &mut b_break1);
    assert_eq!(ops_break_a.iter().filter(|o| **o == Op::Del).count(), 1);
    assert_eq!(ops_break_a.iter().filter(|o| **o == Op::Ins).count(), 4);

    // Case 6B: rem_b becomes empty after matching p elements
    let mut a_break_b: Vec<usize> = (0..4008).collect();
    let mut b_break_b: Vec<usize> = (0..4005).collect();
    a_break_b[0] = 99999;
    b_break_b[0] = 88888;
    let mut b_break2 = u64::MAX;
    let ops_break_b = lcs_ops(&a_break_b, &b_break_b, &mut b_break2);
    assert_eq!(ops_break_b.iter().filter(|o| **o == Op::Del).count(), 4);
    assert_eq!(ops_break_b.iter().filter(|o| **o == Op::Ins).count(), 1);

    // Case 6C: both become empty simultaneously after matching p elements
    let mut a_break_both: Vec<usize> = (0..4005).collect();
    let mut b_break_both: Vec<usize> = (0..4005).collect();
    a_break_both[0] = 99999;
    b_break_both[0] = 88888;
    let mut b_break3 = u64::MAX;
    let ops_break_both = lcs_ops(&a_break_both, &b_break_both, &mut b_break3);
    assert_eq!(ops_break_both.iter().filter(|o| **o == Op::Del).count(), 1);
    assert_eq!(ops_break_both.iter().filter(|o| **o == Op::Ins).count(), 1);
    assert_eq!(ops_break_both.iter().filter(|o| **o == Op::Same).count(), 4004);
}

#[test]
fn compare_at_word_level_still_matches_a_whole_unchanged_word() {
    // `collapse_noise_matches` only runs for character-level compares (wired up in
    // `word_diff`): a single matched *word* between two edits is a real, meaningful match
    // (unlike a single matched character), and must never be folded into a replacement.
    let mut s = Session::new(Document::from_text("the cat sat"));
    s.run("review.compare", &json!({"text": "a cat ran"})).unwrap();
    let p = s.doc.body[0].as_para().unwrap();
    assert!(
        p.run_ranges().any(|(r, c)| c.del.is_none() && c.ins.is_none() && p.text[r].trim() == "cat"),
        "the unchanged word must survive as plain text"
    );
}

#[test]
fn compare_character_level_surfaces_fallback_on_long_paragraphs() {
    // Under MAX_CHAR_DIFF (2,000 characters), character-level diff engages normally:
    // sub-word fragment is marked, characterFallback is 0, and notHonoured does not contain "level".
    let original_short = "The quick brown fox jumps over the lazy dog. ".repeat(10);
    let revised_short = original_short.replacen("jumps", "jumped", 1);
    assert!(original_short.chars().count() < 2000);
    let mut s = Session::new(Document::from_text(&original_short));
    let r = s.run("review.compare", &json!({"text": revised_short, "level": "character"})).unwrap();
    assert_eq!(r["characterFallback"].as_u64(), Some(0));
    let not_honoured = r["notHonoured"].as_array().expect("notHonoured array");
    assert!(!not_honoured.iter().any(|v| v == "level"), "level should be honoured for short paragraphs");
    let p = s.doc.body[0].as_para().expect("body paragraph");
    let deleted: String = p.run_ranges().filter(|(_, c)| c.del.is_some()).map(|(r, _)| p.text[r].to_string()).collect();
    let inserted: String = p.run_ranges().filter(|(_, c)| c.ins.is_some()).map(|(r, _)| p.text[r].to_string()).collect();
    assert_eq!(deleted, "s", "character level marks only the sub-word suffix for short paragraphs");
    assert_eq!(inserted, "ed", "character level marks only the sub-word suffix for short paragraphs");

    // Above MAX_CHAR_DIFF (2,000 characters), character-level diff falls back to word-level diff.
    // The fallback is surfaced in result metadata: characterFallback is incremented and
    // notHonoured reports "level" rather than silently degrading without signal.
    // In the output document, the fallback marks the whole word rather than sub-word fragments.
    let original_long = "The quick brown fox jumps over the lazy dog. ".repeat(50);
    let revised_long = original_long.replacen("jumps", "jumped", 1);
    assert!(original_long.chars().count() > 2000);
    let mut s = Session::new(Document::from_text(&original_long));
    let r = s.run("review.compare", &json!({"text": revised_long, "level": "character"})).unwrap();
    assert_eq!(r["characterFallback"].as_u64(), Some(1));
    let not_honoured = r["notHonoured"].as_array().expect("notHonoured array");
    assert!(not_honoured.iter().any(|v| v == "level"), "level should be listed in notHonoured when fallback occurs");
    let p = s.doc.body[0].as_para().expect("body paragraph");
    let deleted: String = p.run_ranges().filter(|(_, c)| c.del.is_some()).map(|(r, _)| p.text[r].to_string()).collect();
    let inserted: String = p.run_ranges().filter(|(_, c)| c.ins.is_some()).map(|(r, _)| p.text[r].to_string()).collect();
    assert_eq!(deleted, "jumps ", "long paragraph fallback marks the whole word");
    assert_eq!(inserted, "jumped ", "long paragraph fallback marks the whole word");
}

#[test]
fn compare_character_level_exact_boundaries_at_limit() {
    let prefix = "The quick brown fox jumps over the lazy dog. ".repeat(44); // 1980 chars

    // Exactly MAX_CHAR_DIFF (2000) characters: character-level diff engages without fallback.
    let original_2000 = format!("{prefix}1234567890123456789x");
    let revised_2000 = format!("{prefix}1234567890123456789y");
    assert_eq!(original_2000.chars().count(), 2000);
    assert_eq!(revised_2000.chars().count(), 2000);
    let mut s = Session::new(Document::from_text(&original_2000));
    let r = s.run("review.compare", &json!({"text": revised_2000, "level": "character"})).unwrap();
    assert_eq!(r["characterFallback"].as_u64(), Some(0));
    let not_honoured = r["notHonoured"].as_array().expect("notHonoured array");
    assert!(!not_honoured.iter().any(|v| v == "level"), "2000 chars should not fall back");
    let p = s.doc.body[0].as_para().expect("body paragraph");
    let deleted: String = p.run_ranges().filter(|(_, c)| c.del.is_some()).map(|(r, _)| p.text[r].to_string()).collect();
    let inserted: String = p.run_ranges().filter(|(_, c)| c.ins.is_some()).map(|(r, _)| p.text[r].to_string()).collect();
    assert_eq!(deleted, "x", "character level marks single changed character at exactly 2000 chars");
    assert_eq!(inserted, "y", "character level marks single changed character at exactly 2000 chars");

    // Exactly 2001 characters (MAX_CHAR_DIFF + 1): triggers fallback to word-level diff.
    let original_2001 = format!("{prefix}12345678901234567890x");
    let revised_2001 = format!("{prefix}12345678901234567890y");
    assert_eq!(original_2001.chars().count(), 2001);
    assert_eq!(revised_2001.chars().count(), 2001);
    let mut s = Session::new(Document::from_text(&original_2001));
    let r = s.run("review.compare", &json!({"text": revised_2001, "level": "character"})).unwrap();
    assert_eq!(r["characterFallback"].as_u64(), Some(1));
    let not_honoured = r["notHonoured"].as_array().expect("notHonoured array");
    assert!(not_honoured.iter().any(|v| v == "level"), "2001 chars must trigger fallback");
    let p = s.doc.body[0].as_para().expect("body paragraph");
    let deleted: String = p.run_ranges().filter(|(_, c)| c.del.is_some()).map(|(r, _)| p.text[r].to_string()).collect();
    let inserted: String = p.run_ranges().filter(|(_, c)| c.ins.is_some()).map(|(r, _)| p.text[r].to_string()).collect();
    assert_eq!(deleted, "12345678901234567890x", "fallback marks whole word at 2001 chars");
    assert_eq!(inserted, "12345678901234567890y", "fallback marks whole word at 2001 chars");
}

#[test]
fn compare_character_level_multiple_paragraphs_and_asymmetric_lengths() {
    let short_a = "Short original text here.";
    let short_b = "Short revised text here.";
    let long_a = "The quick brown fox jumps over the lazy dog. ".repeat(50);
    let long_b = long_a.replacen("jumps", "jumped", 1);
    let long_c = "Another long paragraph that exceeds two thousand chars. ".repeat(40);
    let long_d = long_c.replacen("Another", "Different", 1);

    // Document with 1 short modified paragraph and 1 long modified paragraph: fallback count is 1.
    let doc_mixed_a = format!("{short_a}\n{long_a}");
    let doc_mixed_b = format!("{short_b}\n{long_b}");
    let mut s = Session::new(Document::from_text(&doc_mixed_a));
    let r = s.run("review.compare", &json!({"text": doc_mixed_b, "level": "character"})).unwrap();
    assert_eq!(r["characterFallback"].as_u64(), Some(1));

    // Document with 2 long modified paragraphs: fallback count is 2.
    let doc_2long_a = format!("{long_a}\n{long_c}");
    let doc_2long_b = format!("{long_b}\n{long_d}");
    let mut s = Session::new(Document::from_text(&doc_2long_a));
    let r = s.run("review.compare", &json!({"text": doc_2long_b, "level": "character"})).unwrap();
    assert_eq!(r["characterFallback"].as_u64(), Some(2));

    // Asymmetric lengths: original short, revised long (> 2000 chars) -> fallback count is 1.
    let mut s = Session::new(Document::from_text(short_a));
    let r = s.run("review.compare", &json!({"text": long_b, "level": "character"})).unwrap();
    assert_eq!(r["characterFallback"].as_u64(), Some(1));

    // Asymmetric lengths: original long (> 2000 chars), revised short -> fallback count is 1.
    let mut s = Session::new(Document::from_text(&long_a));
    let r = s.run("review.compare", &json!({"text": short_b, "level": "character"})).unwrap();
    assert_eq!(r["characterFallback"].as_u64(), Some(1));
}

#[test]
fn compare_word_level_default_and_explicit_never_flag_fallback() {
    let original_long = "The quick brown fox jumps over the lazy dog. ".repeat(50);
    let revised_long = original_long.replacen("jumps", "jumped", 1);
    assert!(original_long.chars().count() > 2000);

    // Explicit "level": "word" on long paragraph: characterFallback is 0 and "level" not in notHonoured.
    let mut s = Session::new(Document::from_text(&original_long));
    let r = s.run("review.compare", &json!({"text": revised_long, "level": "word"})).unwrap();
    assert_eq!(r["characterFallback"].as_u64(), Some(0));
    let not_honoured = r["notHonoured"].as_array().expect("notHonoured array");
    assert!(!not_honoured.iter().any(|v| v == "level"));
    let p = s.doc.body[0].as_para().expect("body paragraph");
    let deleted: String = p.run_ranges().filter(|(_, c)| c.del.is_some()).map(|(r, _)| p.text[r].to_string()).collect();
    let inserted: String = p.run_ranges().filter(|(_, c)| c.ins.is_some()).map(|(r, _)| p.text[r].to_string()).collect();
    assert_eq!(deleted, "jumps ");
    assert_eq!(inserted, "jumped ");

    // Default (no "level" param provided): behaves as word level, characterFallback is 0 and "level" not in notHonoured.
    let mut s = Session::new(Document::from_text(&original_long));
    let r = s.run("review.compare", &json!({"text": revised_long})).unwrap();
    assert_eq!(r["characterFallback"].as_u64(), Some(0));
    let not_honoured = r["notHonoured"].as_array().expect("notHonoured array");
    assert!(!not_honoured.iter().any(|v| v == "level"));
    let p = s.doc.body[0].as_para().expect("body paragraph");
    let deleted: String = p.run_ranges().filter(|(_, c)| c.del.is_some()).map(|(r, _)| p.text[r].to_string()).collect();
    let inserted: String = p.run_ranges().filter(|(_, c)| c.ins.is_some()).map(|(r, _)| p.text[r].to_string()).collect();
    assert_eq!(deleted, "jumps ");
    assert_eq!(inserted, "jumped ");
}

#[test]
fn compare_keeps_text_boxes_of_the_original() {
    // Text boxes are anchored in body paragraphs: one after changed text, one alone in an
    // otherwise empty paragraph, one after text the revised document keeps. Their anchors must
    // survive the compare (marked deleted, since the revised text has no boxes), so no text is lost.
    for show_in in ["original", "revised", "new"] {
        let mut s = Session::new(Document::from_text("Body one.\n\nKeep."));
        let mut boxes = Vec::new();
        for (para, off, text) in [(0usize, "Body one.".len(), "Alpha boxword gamma."), (1, 0, "Vml onlyword here."), (2, "Keep.".len(), "Kept box.")]
        {
            s.run("caret.set", &json!({"pos": Pos::body(para, off)})).unwrap();
            let id = s.run("insert.textBox", &json!({"text": text})).unwrap()["story"].as_u64().unwrap() as u32;
            boxes.push((id, text));
        }
        s.run("caret.set", &json!({"pos": Pos::body(0, 0)})).unwrap();
        s.run("review.compare", &json!({"text": "Totally new first.\nSecond new line.\nKeep.", "showIn": show_in})).unwrap();
        for (id, text) in &boxes {
            let anchor = s.doc.text_box_anchor(*id).unwrap_or_else(|| panic!("{show_in}: the box \"{text}\" lost its anchor"));
            assert_eq!(s.doc.plain_text(StoryRef::Part(*id)), *text);
            let p = s.doc.para(anchor.story, &anchor.path).unwrap();
            assert!(
                p.props_of_char(anchor.off - wordcraft_doc::para::OBJ.len_utf8()).del.is_some(),
                "{show_in}: the box \"{text}\" is not marked deleted"
            );
        }
        s.run("review.rejectAll", &json!({})).unwrap();
        assert_eq!(s.doc.plain_text(StoryRef::Body), "Body one.\n\nKeep.");
        for (id, text) in &boxes {
            assert!(s.doc.text_box_anchor(*id).is_some(), "{show_in}: rejecting lost the box \"{text}\"");
            assert_eq!(s.doc.plain_text(StoryRef::Part(*id)), *text);
        }
    }
}

struct TempTestDir(std::path::PathBuf);

impl TempTestDir {
    fn new(prefix: &str) -> Self {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let id = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("{prefix}-{}-{id}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create temp test dir");
        Self(dir)
    }

    fn path(&self, name: &str) -> std::path::PathBuf {
        self.0.join(name)
    }
}

impl Drop for TempTestDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// #6086: When base and revised have an embedded image at identical size and extents,
/// but the media bytes were replaced, Compare must emit a tracked deletion of the base
/// image and tracked insertion of the revised image, and keep both media parts in the
/// result package.
#[test]
fn compare_detects_replaced_image_bytes_with_unchanged_extents() {
    let tmp = TempTestDir::new("wc-compare-img-replace");
    let make_doc = |bytes: Vec<u8>| -> Document {
        let mut d = Document::new();
        let key = d.add_media(bytes, "png");
        let mut p = Paragraph::new();
        let img = InlineObject::Image {
            media: key,
            w: 120.0,
            h: 80.0,
            alt: "photo".into(),
            float: Default::default(),
            crop: [0.0; 4],
            fx: Default::default(),
            ole: None,
        };
        p.insert_object(0, img, &CharProps::default()).unwrap();
        d.body = vec![Arc::new(Block::Para(p))];
        d
    };
    let base_png = vec![0x89, b'P', b'N', b'G', 1, 1, 1, 1];
    let rev_png = vec![0x89, b'P', b'N', b'G', 2, 2, 2, 2];
    let base_doc = make_doc(base_png.clone());
    let rev_doc = make_doc(rev_png.clone());

    let path = tmp.path("revised.docx");
    crate::io::save_path(&path, &rev_doc).unwrap();

    let mut s = Session::new(base_doc);
    let r = s.run("review.compare", &json!({"path": path.to_string_lossy(), "showIn": "new"})).unwrap();

    assert!(r["changes"].as_u64().unwrap() >= 1, "an image byte change must be detected as a change");
    let p = s.doc.body[0].as_para().expect("body has paragraph");
    assert_eq!(p.objects.len(), 2, "paragraph must have both deleted and inserted images");
    let objs: Vec<(usize, &InlineObject)> = p.objects_with_offsets().collect();
    assert_eq!(objs.len(), 2, "paragraph must have two objects with offsets");
    let (off_del, del_img) = objs[0];
    let (off_ins, ins_img) = objs[1];
    let (InlineObject::Image { media: m_del, .. }, InlineObject::Image { media: m_ins, .. }) = (del_img, ins_img) else {
        panic!("both objects must be images");
    };
    assert_ne!(m_del, m_ins, "deleted and inserted images must have distinct media keys");
    assert!(p.props_of_char(off_del).del.is_some(), "base image must have del revision mark");
    assert!(p.props_of_char(off_ins).ins.is_some(), "revised image must have ins revision mark");
    assert_eq!(s.doc.media.get(m_del).map(|b| &**b), Some(&base_png));
    assert_eq!(s.doc.media.get(m_ins).map(|b| &**b), Some(&rev_png));

    // And saving to DOCX and reading back preserves both media files in the package.
    let out_path = tmp.path("out.docx");
    crate::io::save_path(&out_path, &s.doc).unwrap();
    let back = crate::io::open_path(&out_path).unwrap();
    assert!(back.media.values().any(|b| **b == base_png), "result package must contain base image bytes");
    assert!(back.media.values().any(|b| **b == rev_png), "result package must contain revised image bytes");

    // Identical image bytes at identical size: no changes emitted.
    let base_doc_same = make_doc(base_png.clone());
    let same_path = tmp.path("same.docx");
    crate::io::save_path(&same_path, &make_doc(base_png.clone())).unwrap();
    let mut s_same = Session::new(base_doc_same);
    let r_same = s_same.run("review.compare", &json!({"path": same_path.to_string_lossy(), "showIn": "new"})).unwrap();
    assert_eq!(r_same["changes"], 0, "comparing identical image bytes must emit no changes");

    // Extensionless media in revised document adopts with .bin extension fallback and preserves payload.
    let extless_bytes = vec![0xFF, 0xD8, 0xFF, 0xE0, 1, 2, 3, 4];
    let mut rev_doc_extless = Document::new();
    rev_doc_extless.media.insert("rawimage".into(), Arc::new(extless_bytes.clone()));
    let mut p_extless = Paragraph::new();
    p_extless
        .insert_object(
            0,
            InlineObject::Image {
                media: "rawimage".into(),
                w: 120.0,
                h: 80.0,
                alt: "photo".into(),
                float: Default::default(),
                crop: [0.0; 4],
                fx: Default::default(),
                ole: None,
            },
            &CharProps::default(),
        )
        .unwrap();
    rev_doc_extless.body = vec![Arc::new(Block::Para(p_extless))];
    let extless_path = tmp.path("extless.json");
    crate::io::save_path(&extless_path, &rev_doc_extless).unwrap();
    let mut s_extless = Session::new(make_doc(base_png));
    let _ = s_extless.run("review.compare", &json!({"path": extless_path.to_string_lossy(), "showIn": "new"})).unwrap();
    let bin_entry = s_extless.doc.media.iter().find(|(k, _)| k.ends_with(".bin"));
    assert!(bin_entry.is_some(), "extensionless media falls back to bin extension");
    assert_eq!(bin_entry.map(|(_, b)| &***b), Some(&extless_bytes[..]));

    // Media inside deeply nested groups is remapped iteratively without recursion limits.
    let mut deep_obj = InlineObject::Image {
        media: "deep_img.png".into(),
        w: 50.0,
        h: 50.0,
        alt: "".into(),
        float: Default::default(),
        crop: [0.0; 4],
        fx: Default::default(),
        ole: None,
    };
    for _ in 0..20 {
        deep_obj = InlineObject::Group {
            w: 50.0,
            h: 50.0,
            float: Default::default(),
            ch_w: 50.0,
            ch_h: 50.0,
            children: vec![wordcraft_doc::para::GroupChild { x: 0.0, y: 0.0, obj: deep_obj }],
            canvas: None,
        };
    }
    let mut map: std::collections::BTreeMap<String, String> = std::collections::BTreeMap::new();
    map.insert("deep_img.png".into(), "image99.png".into());
    super::compare::remap_object_media(&mut deep_obj, &mut |k| map.get(k).cloned());
    let mut cur = &deep_obj;
    while let InlineObject::Group { children, .. } = cur {
        cur = &children[0].obj;
    }
    if let InlineObject::Image { media, .. } = cur {
        assert_eq!(media, "image99.png", "media inside 20-level group must be remapped");
    } else {
        panic!("innermost object must be Image");
    }
}

/// #6086: When an image replacement in a same-text paragraph triggers `word_diff`, matched text,
/// paragraph mark, and paragraph formatting changes must also record `fmt_change` revisions when
/// `formatting` is enabled so Reject All restores both the original formatting and the original
/// image, while Accept All keeps the revised formatting and revised image.
#[test]
fn compare_replaced_image_in_paragraph_tracks_matched_formatting_for_reject() {
    let tmp = TempTestDir::new("wc-compare-img-para-fmt");
    let base_png = vec![0x89, b'P', b'N', b'G', 7, 7, 7, 7];
    let rev_png = vec![0x89, b'P', b'N', b'G', 8, 8, 8, 8];

    let make_para_doc = |bytes: Vec<u8>, bold: Option<bool>, align: Option<wordcraft_doc::props::Align>, mark_italic: Option<bool>| -> Document {
        let mut d = Document::new();
        let key = d.add_media(bytes, "png");
        let mut p = Paragraph::with_text("Logo", CharProps { bold, ..Default::default() });
        p.props.align = align;
        p.mark = CharProps { italic: mark_italic, ..Default::default() };
        let at = p.len();
        p.insert_object(
            at,
            InlineObject::Image {
                media: key,
                w: 120.0,
                h: 80.0,
                alt: "logo".into(),
                float: Default::default(),
                crop: [0.0; 4],
                fx: Default::default(),
                ole: None,
            },
            &CharProps::default(),
        )
        .unwrap();
        d.body = vec![Arc::new(Block::Para(p))];
        d
    };

    let base_doc = make_para_doc(base_png.clone(), None, None, None);
    let rev_doc = make_para_doc(rev_png.clone(), Some(true), Some(wordcraft_doc::props::Align::Center), Some(true));
    let rev_path = tmp.path("rev.docx");
    crate::io::save_path(&rev_path, &rev_doc).unwrap();

    let mut s = Session::new(base_doc.clone());
    let r = s.run("review.compare", &json!({"path": rev_path.to_string_lossy(), "showIn": "new"})).unwrap();
    assert_eq!(r["changes"], 1);

    let p = s.doc.body[0].as_para().expect("compared paragraph");
    assert_eq!(p.props_of_char(0).bold, Some(true), "matched text carries revised bold");
    assert!(p.props_of_char(0).fmt_change.is_some(), "matched text carries formatting revision");
    assert_eq!(p.props.align, Some(wordcraft_doc::props::Align::Center));
    assert!(p.props.fmt_change.is_some(), "paragraph properties carry formatting revision");
    assert_eq!(p.mark.italic, Some(true));
    assert!(p.mark.fmt_change.is_some(), "paragraph mark carries formatting revision");
    let objs: Vec<(usize, &InlineObject)> = p.objects_with_offsets().collect();
    assert_eq!(objs.len(), 2);
    assert!(p.props_of_char(objs[0].0).del.is_some());
    assert!(p.props_of_char(objs[1].0).ins.is_some());

    // Round-trip through DOCX before resolving both ways.
    let cmp_path = tmp.path("compared.docx");
    crate::io::save_path(&cmp_path, &s.doc).unwrap();
    let reopened = crate::io::open_path(&cmp_path).unwrap();

    let mut accepted = Session::new(reopened.clone());
    accepted.run("review.acceptAll", &json!({})).unwrap();
    let p_acc = accepted.doc.body[0].as_para().unwrap();
    assert_eq!(p_acc.props_of_char(0).bold, Some(true));
    assert!(p_acc.props_of_char(0).fmt_change.is_none());
    assert_eq!(p_acc.props.align, Some(wordcraft_doc::props::Align::Center));
    assert_eq!(p_acc.mark.italic, Some(true));
    let objs_acc: Vec<(usize, &InlineObject)> = p_acc.objects_with_offsets().collect();
    assert_eq!(objs_acc.len(), 1);
    let InlineObject::Image { media: m_acc, .. } = objs_acc[0].1 else { panic!("expected image") };
    assert_eq!(accepted.doc.media.get(m_acc).map(|b| &**b), Some(&rev_png));

    let mut rejected = Session::new(reopened);
    rejected.run("review.rejectAll", &json!({})).unwrap();
    let p_rej = rejected.doc.body[0].as_para().unwrap();
    assert_ne!(p_rej.props_of_char(0).bold, Some(true), "Reject All must restore original non-bold text");
    assert!(p_rej.props_of_char(0).fmt_change.is_none());
    assert_eq!(p_rej.props.align, None, "Reject All must restore original paragraph alignment");
    assert_ne!(p_rej.mark.italic, Some(true), "Reject All must restore original paragraph mark");
    let objs_rej: Vec<(usize, &InlineObject)> = p_rej.objects_with_offsets().collect();
    assert_eq!(objs_rej.len(), 1);
    let InlineObject::Image { media: m_rej, .. } = objs_rej[0].1 else { panic!("expected image") };
    assert_eq!(rejected.doc.media.get(m_rej).map(|b| &**b), Some(&base_png), "Reject All must restore original image");

    // With formatting disabled, image del/ins marks remain while formatting revisions are omitted.
    let mut s_no_fmt = Session::new(base_doc);
    let r_no_fmt = s_no_fmt.run("review.compare", &json!({"path": rev_path.to_string_lossy(), "showIn": "new", "formatting": false})).unwrap();
    assert_eq!(r_no_fmt["changes"], 1);
    let p_nf = s_no_fmt.doc.body[0].as_para().unwrap();
    assert!(p_nf.props_of_char(0).fmt_change.is_none());
    assert!(p_nf.props.fmt_change.is_none());
    assert!(p_nf.mark.fmt_change.is_none());
    let objs_nf: Vec<(usize, &InlineObject)> = p_nf.objects_with_offsets().collect();
    assert_eq!(objs_nf.len(), 2);
    assert!(p_nf.props_of_char(objs_nf[0].0).del.is_some());
    assert!(p_nf.props_of_char(objs_nf[1].0).ins.is_some());
}

/// #6086: Replaced image bytes inside a table whose text is unchanged must be tracked in the
/// cell paragraph (not silently applied unmarked), survive DOCX save/reopen, and resolve to the
/// revised or original image under Accept All and Reject All.
#[test]
fn compare_detects_replaced_image_bytes_inside_unchanged_text_table() {
    let tmp = TempTestDir::new("wc-compare-tbl-img-replace");
    let make_table_doc = |bytes: Vec<u8>| -> Document {
        let mut d = Document::new();
        let key = d.add_media(bytes, "png");
        let mut p = Paragraph::with_text("Logo", CharProps::default());
        let at = p.len();
        p.insert_object(
            at,
            InlineObject::Image {
                media: key,
                w: 120.0,
                h: 80.0,
                alt: "logo".into(),
                float: Default::default(),
                crop: [0.0; 4],
                fx: Default::default(),
                ole: None,
            },
            &CharProps::default(),
        )
        .unwrap();
        let mut t = wordcraft_doc::table::Table::new(1, 1, 200.0);
        t.rows[0].cells[0].blocks = vec![Arc::new(Block::Para(p))];
        d.body = vec![Arc::new(Block::Table(t)), Arc::new(Block::Para(Paragraph::new()))];
        d
    };

    fn cell_para(doc: &Document) -> &Paragraph {
        let t = doc.body[0].as_table().expect("body holds 1x1 table");
        assert_eq!(doc.body.iter().filter(|b| b.as_table().is_some()).count(), 1, "body keeps one table");
        assert_eq!(t.rows.len(), 1);
        assert_eq!(t.rows[0].cells.len(), 1);
        t.rows[0].cells[0].blocks[0].as_para().expect("cell holds paragraph")
    }

    let base_png = vec![0x89, b'P', b'N', b'G', 10, 20, 30, 40];
    let rev_png = vec![0x89, b'P', b'N', b'G', 50, 60, 70, 80];
    let orphan_hdr_png = vec![0x89, b'P', b'N', b'G', 99, 98, 97, 96];

    let base_doc = make_table_doc(base_png.clone());
    let mut rev_doc = make_table_doc(rev_png.clone());
    // Attach an uncompared header part referencing media that nothing in the body uses.
    let hdr_key = rev_doc.add_media(orphan_hdr_png.clone(), "png");
    let mut hdr_para = Paragraph::new();
    hdr_para
        .insert_object(
            0,
            InlineObject::Image {
                media: hdr_key,
                w: 40.0,
                h: 40.0,
                alt: "hdr".into(),
                float: Default::default(),
                crop: [0.0; 4],
                fx: Default::default(),
                ole: None,
            },
            &CharProps::default(),
        )
        .unwrap();
    let hdr_part = rev_doc.add_part(wordcraft_doc::PartKind::Header, vec![Arc::new(Block::Para(hdr_para))]);
    rev_doc.last_section.headers.default = Some(hdr_part);

    let rev_path = tmp.path("tbl-rev.docx");
    crate::io::save_path(&rev_path, &rev_doc).unwrap();

    let mut s = Session::new(base_doc);
    let r = s.run("review.compare", &json!({"path": rev_path.to_string_lossy(), "showIn": "new"})).unwrap();

    assert_eq!(r["changes"], 1, "replaced image inside unchanged-text table must count as 1 change");
    assert!(
        !s.doc.media.values().any(|b| **b == orphan_hdr_png),
        "unreferenced media from uncompared revised header parts must not stay in result media"
    );

    let check_tracked_cell = |doc: &Document| {
        let p = cell_para(doc);
        assert_eq!(p.plain_text(), "Logo");
        let objs: Vec<(usize, &InlineObject)> = p.objects_with_offsets().collect();
        assert_eq!(objs.len(), 2, "table cell must hold deleted base image and inserted revised image");
        let (off_del, InlineObject::Image { media: m_del, .. }) = objs[0] else { panic!("expected base image") };
        let (off_ins, InlineObject::Image { media: m_ins, .. }) = objs[1] else { panic!("expected revised image") };
        assert_ne!(m_del, m_ins);
        assert!(p.props_of_char(0).del.is_none() && p.props_of_char(0).ins.is_none(), "`Logo` text stays unmarked");
        assert!(p.props_of_char(off_del).del.is_some(), "base table image must carry del mark");
        assert!(p.props_of_char(off_ins).ins.is_some(), "revised table image must carry ins mark");
        assert_eq!(doc.media.get(m_del).map(|b| &**b), Some(&base_png));
        assert_eq!(doc.media.get(m_ins).map(|b| &**b), Some(&rev_png));
    };
    check_tracked_cell(&s.doc);

    // Save compared result to DOCX and reopen: change marks and both payloads survive.
    let cmp_path = tmp.path("tbl-cmp.docx");
    crate::io::save_path(&cmp_path, &s.doc).unwrap();
    let reopened = crate::io::open_path(&cmp_path).unwrap();
    check_tracked_cell(&reopened);

    // Accept All on reopened DOCX -> saves and reopens with only the revised image unmarked.
    let mut accepted = Session::new(reopened.clone());
    accepted.run("review.acceptAll", &json!({})).unwrap();
    let acc_path = tmp.path("tbl-acc.docx");
    crate::io::save_path(&acc_path, &accepted.doc).unwrap();
    let acc_back = crate::io::open_path(&acc_path).unwrap();
    let p_acc = cell_para(&acc_back);
    assert_eq!(p_acc.plain_text(), "Logo");
    let objs_acc: Vec<(usize, &InlineObject)> = p_acc.objects_with_offsets().collect();
    assert_eq!(objs_acc.len(), 1, "Accept All must leave one image in the table cell");
    let (off_acc, InlineObject::Image { media: m_acc, .. }) = objs_acc[0] else { panic!("expected image") };
    assert!(p_acc.props_of_char(off_acc).ins.is_none() && p_acc.props_of_char(off_acc).del.is_none());
    assert_eq!(acc_back.media.get(m_acc).map(|b| &**b), Some(&rev_png), "Accept All must keep revised image bytes");

    // Reject All on reopened DOCX -> saves and reopens with only the original image unmarked.
    let mut rejected = Session::new(reopened);
    rejected.run("review.rejectAll", &json!({})).unwrap();
    let rej_path = tmp.path("tbl-rej.docx");
    crate::io::save_path(&rej_path, &rejected.doc).unwrap();
    let rej_back = crate::io::open_path(&rej_path).unwrap();
    let p_rej = cell_para(&rej_back);
    assert_eq!(p_rej.plain_text(), "Logo");
    let objs_rej: Vec<(usize, &InlineObject)> = p_rej.objects_with_offsets().collect();
    assert_eq!(objs_rej.len(), 1, "Reject All must restore one image in the table cell");
    let (off_rej, InlineObject::Image { media: m_rej, .. }) = objs_rej[0] else { panic!("expected image") };
    assert!(p_rej.props_of_char(off_rej).ins.is_none() && p_rej.props_of_char(off_rej).del.is_none());
    assert_eq!(rej_back.media.get(m_rej).map(|b| &**b), Some(&base_png), "Reject All must recover base image bytes");

    // Identical image bytes inside an unchanged-text table: 0 changes and 1 unmarked image.
    let same_tbl_path = tmp.path("tbl-same.docx");
    crate::io::save_path(&same_tbl_path, &make_table_doc(base_png.clone())).unwrap();
    let mut s_same = Session::new(make_table_doc(base_png));
    let r_same = s_same.run("review.compare", &json!({"path": same_tbl_path.to_string_lossy(), "showIn": "new"})).unwrap();
    assert_eq!(r_same["changes"], 0, "identical table image bytes must emit 0 changes");
    assert_eq!(cell_para(&s_same.doc).objects.len(), 1);
}

/// #6086: Same-text tables up to `MAX_TABLE_DEPTH` recurse cell by cell for replaced media;
/// layout-only matches keep a single table as on `main`, while incompatible layouts or nesting
/// beyond `MAX_TABLE_DEPTH` with replaced media defer structural table changes and report `"tables"`
/// in `notHonoured` without leaving duplicate table shells.
#[test]
fn compare_table_image_replacement_handles_layout_fallbacks_and_max_depth() {
    let tmp = TempTestDir::new("wc-compare-tbl-depth");
    let base_png = vec![0x89, b'P', b'N', b'G', 10, 20, 30, 40];
    let rev_png = vec![0x89, b'P', b'N', b'G', 50, 60, 70, 80];

    let assert_single_table = |doc: &Document, want_rows: usize, want_cols: usize, label: &str| {
        let tables: Vec<&wordcraft_doc::table::Table> = doc.body.iter().filter_map(|b| b.as_table()).collect();
        assert_eq!(tables.len(), 1, "{label}: must keep a single table");
        assert_eq!(tables[0].rows.len(), want_rows, "{label}: row count");
        assert_eq!(tables[0].rows[0].cells.len(), want_cols, "{label}: col count");
    };

    // Reshaped tables (1x2 vs 2x1) with identical joined cell text and replaced image bytes:
    // cannot diff 1-to-1 by cell, so structural table replacement is deferred with a single table
    // and `"tables"` is reported in `notHonoured`.
    let mut base_1x2 = Document::new();
    let k1 = base_1x2.add_media(base_png.clone(), "png");
    let mut p_c0 = Paragraph::with_text("Logo", CharProps::default());
    let at_c0 = p_c0.len();
    p_c0.insert_object(
        at_c0,
        InlineObject::Image {
            media: k1,
            w: 120.0,
            h: 80.0,
            alt: "logo".into(),
            float: Default::default(),
            crop: [0.0; 4],
            fx: Default::default(),
            ole: None,
        },
        &CharProps::default(),
    )
    .unwrap();
    let mut t_1x2 = wordcraft_doc::table::Table::new(1, 2, 200.0);
    t_1x2.rows[0].cells[0].blocks = vec![Arc::new(Block::Para(p_c0))];
    t_1x2.rows[0].cells[1].blocks = vec![Arc::new(Block::Para(Paragraph::with_text("Caption", CharProps::default())))];
    base_1x2.body = vec![Arc::new(Block::Table(t_1x2)), Arc::new(Block::Para(Paragraph::new()))];

    let mut rev_2x1 = Document::new();
    let k2 = rev_2x1.add_media(rev_png.clone(), "png");
    let mut p_r0 = Paragraph::with_text("Logo", CharProps::default());
    let at_r0 = p_r0.len();
    p_r0.insert_object(
        at_r0,
        InlineObject::Image {
            media: k2,
            w: 120.0,
            h: 80.0,
            alt: "logo".into(),
            float: Default::default(),
            crop: [0.0; 4],
            fx: Default::default(),
            ole: None,
        },
        &CharProps::default(),
    )
    .unwrap();
    let mut t_2x1 = wordcraft_doc::table::Table::new(2, 1, 200.0);
    t_2x1.rows[0].cells[0].blocks = vec![Arc::new(Block::Para(p_r0))];
    t_2x1.rows[1].cells[0].blocks = vec![Arc::new(Block::Para(Paragraph::with_text("Caption", CharProps::default())))];
    rev_2x1.body = vec![Arc::new(Block::Table(t_2x1)), Arc::new(Block::Para(Paragraph::new()))];

    let reshaped_path = tmp.path("tbl-reshaped.docx");
    crate::io::save_path(&reshaped_path, &rev_2x1).unwrap();
    let mut s_reshaped = Session::new(base_1x2);
    let r_reshaped = s_reshaped.run("review.compare", &json!({"path": reshaped_path.to_string_lossy(), "showIn": "new"})).unwrap();
    assert_eq!(r_reshaped["changes"], 0);
    assert!(
        r_reshaped["notHonoured"].as_array().unwrap().iter().any(|v| v == "tables"),
        "incompatible reshaped table with replaced media must report `tables` in notHonoured"
    );
    let assert_table_image = |doc: &Document, expected: &[u8], label: &str| {
        let t = doc.body.iter().find_map(|b| b.as_table()).expect("table");
        let p = t.rows[0].cells[0].blocks[0].as_para().expect("cell para");
        let objs: Vec<(usize, &InlineObject)> = p.objects_with_offsets().collect();
        assert_eq!(objs.len(), 1, "{label}: expected single image");
        let InlineObject::Image { media, .. } = objs[0].1 else { panic!("{label}: expected image") };
        assert_eq!(doc.media.get(media).map(|b| b.as_slice()), Some(expected), "{label}: image payload");
    };
    assert_single_table(&s_reshaped.doc, 1, 2, "reshaped table compare");
    assert_table_image(&s_reshaped.doc, &base_png, "reshaped table compare");
    for cmd in ["review.acceptAll", "review.rejectAll"] {
        let mut s_res = Session::new(s_reshaped.doc.clone());
        s_res.run(cmd, &json!({})).unwrap();
        assert_single_table(&s_res.doc, 1, 2, cmd);
        assert_table_image(&s_res.doc, &base_png, cmd);
    }

    // Equal cell count but changed spans ([1, 2] vs [2, 1]):
    // - with identical image (layout-only match): keeps single table with 0 changes and no fallback flag;
    // - with replaced image: defers structural replacement and reports `tables` in notHonoured.
    let make_span_doc = |spans: [u32; 2], bytes: Vec<u8>| -> Document {
        let mut d = Document::new();
        let k = d.add_media(bytes, "png");
        let mut p = Paragraph::with_text("Logo", CharProps::default());
        let at = p.len();
        p.insert_object(
            at,
            InlineObject::Image {
                media: k,
                w: 120.0,
                h: 80.0,
                alt: "logo".into(),
                float: Default::default(),
                crop: [0.0; 4],
                fx: Default::default(),
                ole: None,
            },
            &CharProps::default(),
        )
        .unwrap();
        let mut t = wordcraft_doc::table::Table::new(1, 3, 300.0);
        t.rows[0].cells.truncate(2);
        t.rows[0].cells[0].props.span = spans[0];
        t.rows[0].cells[0].blocks = vec![Arc::new(Block::Para(p))];
        t.rows[0].cells[1].props.span = spans[1];
        t.rows[0].cells[1].blocks = vec![Arc::new(Block::Para(Paragraph::with_text("Caption", CharProps::default())))];
        d.body = vec![Arc::new(Block::Table(t)), Arc::new(Block::Para(Paragraph::new()))];
        d
    };
    let span_same_path = tmp.path("tbl-span-same.docx");
    crate::io::save_path(&span_same_path, &make_span_doc([2, 1], base_png.clone())).unwrap();
    let mut s_span_same = Session::new(make_span_doc([1, 2], base_png.clone()));
    let r_span_same = s_span_same.run("review.compare", &json!({"path": span_same_path.to_string_lossy(), "showIn": "new"})).unwrap();
    assert_eq!(r_span_same["changes"], 0, "layout-only span match keeps old single-table behavior");
    assert!(!r_span_same["notHonoured"].as_array().unwrap().iter().any(|v| v == "tables"));
    assert_single_table(&s_span_same.doc, 1, 2, "layout-only span match");

    let span_diff_path = tmp.path("tbl-span-diff.docx");
    crate::io::save_path(&span_diff_path, &make_span_doc([2, 1], rev_png.clone())).unwrap();
    let mut s_span_diff = Session::new(make_span_doc([1, 2], base_png.clone()));
    let r_span_diff = s_span_diff.run("review.compare", &json!({"path": span_diff_path.to_string_lossy(), "showIn": "new"})).unwrap();
    assert_eq!(r_span_diff["changes"], 0);
    assert!(r_span_diff["notHonoured"].as_array().unwrap().iter().any(|v| v == "tables"));
    assert_single_table(&s_span_diff.doc, 1, 2, "incompatible span with replaced media");
    assert_table_image(&s_span_diff.doc, &base_png, "incompatible span with replaced media");
    let t_span = s_span_diff.doc.body[0].as_table().unwrap();
    assert_eq!([t_span.rows[0].cells[0].span(), t_span.rows[0].cells[1].span()], [1, 2]);

    // Nested tables up to MAX_TABLE_DEPTH (18 levels, leaf at depth 17) recursively retain one
    // table per nesting level and track replaced image bytes at the leaf.
    let make_nested_table_doc = |levels: usize, bytes: Vec<u8>| -> Document {
        let mut d = Document::new();
        let k = d.add_media(bytes, "png");
        let mut p = Paragraph::with_text("Deep", CharProps::default());
        let at = p.len();
        p.insert_object(
            at,
            InlineObject::Image {
                media: k,
                w: 50.0,
                h: 50.0,
                alt: "deep".into(),
                float: Default::default(),
                crop: [0.0; 4],
                fx: Default::default(),
                ole: None,
            },
            &CharProps::default(),
        )
        .unwrap();
        let mut blk = Block::Para(p);
        for _ in 0..levels {
            let mut t = wordcraft_doc::table::Table::new(1, 1, 200.0);
            // Every OOXML table cell containing a nested table ends with a trailing paragraph.
            t.rows[0].cells[0].blocks = vec![Arc::new(blk), Arc::new(Block::Para(Paragraph::new()))];
            blk = Block::Table(t);
        }
        d.body = vec![Arc::new(blk), Arc::new(Block::Para(Paragraph::new()))];
        d
    };
    let deep_path = tmp.path("tbl-deep.docx");
    crate::io::save_path(&deep_path, &make_nested_table_doc(wordcraft_doc::table::MAX_TABLE_DEPTH, rev_png.clone())).unwrap();
    let mut s_deep = Session::new(make_nested_table_doc(wordcraft_doc::table::MAX_TABLE_DEPTH, base_png.clone()));
    let r_deep = s_deep.run("review.compare", &json!({"path": deep_path.to_string_lossy(), "showIn": "new"})).unwrap();
    assert_eq!(r_deep["changes"], 1, "replaced image at MAX_TABLE_DEPTH must still be tracked");
    assert!(!r_deep["notHonoured"].as_array().unwrap().iter().any(|v| v == "tables"));
    let mut cur_blocks = &s_deep.doc.body;
    for level in 0..wordcraft_doc::table::MAX_TABLE_DEPTH {
        let lvl_tables: Vec<&wordcraft_doc::table::Table> = cur_blocks.iter().filter_map(|b| b.as_table()).collect();
        assert_eq!(lvl_tables.len(), 1, "level {level} must retain one recursively diffed table");
        cur_blocks = &lvl_tables[0].rows[0].cells[0].blocks;
    }
    let leaf_p = cur_blocks[0].as_para().expect("innermost cell holds leaf paragraph");
    assert_eq!(leaf_p.plain_text(), "Deep");
    let leaf_objs: Vec<(usize, &InlineObject)> = leaf_p.objects_with_offsets().collect();
    assert_eq!(leaf_objs.len(), 2, "leaf at MAX_TABLE_DEPTH holds both deleted and inserted images");
    let (off_d, InlineObject::Image { media: md, .. }) = leaf_objs[0] else { panic!("expected base leaf image") };
    let (off_i, InlineObject::Image { media: mi, .. }) = leaf_objs[1] else { panic!("expected revised leaf image") };
    assert!(leaf_p.props_of_char(off_d).del.is_some() && leaf_p.props_of_char(off_d).ins.is_none());
    assert!(leaf_p.props_of_char(off_i).ins.is_some() && leaf_p.props_of_char(off_i).del.is_none());
    assert_eq!(s_deep.doc.media.get(md).map(|b| &**b), Some(&base_png));
    assert_eq!(s_deep.doc.media.get(mi).map(|b| &**b), Some(&rev_png));

    // Beyond MAX_TABLE_DEPTH (19 nested tables, reaching depth == MAX_TABLE_DEPTH): defers structural
    // replacement (single table, no duplicate empty shell) and reports `tables` in notHonoured.
    let over_depth_path = tmp.path("tbl-over-depth.docx");
    crate::io::save_path(&over_depth_path, &make_nested_table_doc(wordcraft_doc::table::MAX_TABLE_DEPTH + 1, rev_png.clone())).unwrap();
    let mut s_over = Session::new(make_nested_table_doc(wordcraft_doc::table::MAX_TABLE_DEPTH + 1, base_png.clone()));
    let r_over = s_over.run("review.compare", &json!({"path": over_depth_path.to_string_lossy(), "showIn": "new"})).unwrap();
    assert_eq!(r_over["changes"], 0);
    assert!(
        r_over["notHonoured"].as_array().unwrap().iter().any(|v| v == "tables"),
        "tables beyond MAX_TABLE_DEPTH with replaced media must report `tables` in notHonoured"
    );
    let mut over_blocks = &s_over.doc.body;
    for _ in 0..wordcraft_doc::table::MAX_TABLE_DEPTH {
        let lvl_tables: Vec<&wordcraft_doc::table::Table> = over_blocks.iter().filter_map(|b| b.as_table()).collect();
        assert_eq!(lvl_tables.len(), 1);
        over_blocks = &lvl_tables[0].rows[0].cells[0].blocks;
    }
    let cap_tables: Vec<&wordcraft_doc::table::Table> = over_blocks.iter().filter_map(|b| b.as_table()).collect();
    assert_eq!(cap_tables.len(), 1, "at depth == MAX_TABLE_DEPTH, keeps single table without duplicate shell");
    let over_leaf_p = cap_tables[0].rows[0].cells[0].blocks[0].as_para().expect("over-depth leaf para");
    let over_leaf_objs: Vec<(usize, &InlineObject)> = over_leaf_p.objects_with_offsets().collect();
    assert_eq!(over_leaf_objs.len(), 1, "over-depth table preserves single original image");
    let InlineObject::Image { media: m_over, .. } = over_leaf_objs[0].1 else { panic!("expected over-depth image") };
    assert_eq!(s_over.doc.media.get(m_over).map(|b| &**b), Some(&base_png));
}

/// #6086: Compatible tables with replaced media track table, row, and cell property differences
/// via undoable `fmt_change` revisions; image-free layout-only grid differences keep the old
/// single-table behavior without emitting two tables, and changed grid widths combined with
/// replaced media defer structural replacement when `formatting` is enabled (reporting `"tables"`
/// in `notHonoured`) while tracking the replaced media when `formatting` is false.
#[test]
fn compare_unchanged_text_table_tracks_table_row_cell_props_and_grid_widths() {
    let tmp = TempTestDir::new("wc-compare-tbl-props");
    let base_png = vec![0x89, b'P', b'N', b'G', 11, 22, 33, 44];
    let rev_png = vec![0x89, b'P', b'N', b'G', 55, 66, 77, 88];

    let make_styled_table_doc = |bytes: Vec<u8>, styled: bool, width: f32| -> Document {
        let mut d = Document::new();
        let k = d.add_media(bytes, "png");
        let mut p = Paragraph::with_text("Logo", CharProps::default());
        let at = p.len();
        p.insert_object(
            at,
            InlineObject::Image {
                media: k,
                w: 100.0,
                h: 60.0,
                alt: "logo".into(),
                float: Default::default(),
                crop: [0.0; 4],
                fx: Default::default(),
                ole: None,
            },
            &CharProps::default(),
        )
        .unwrap();
        let mut t = wordcraft_doc::table::Table::new(1, 1, width);
        if styled {
            t.props.align = Some(wordcraft_doc::props::Align::Center);
            t.rows[0].props.cant_split = true;
            t.rows[0].cells[0].props.shading = Some(wordcraft_doc::props::Rgb(0xEE, 0xFF, 0xDD));
        }
        t.rows[0].cells[0].blocks = vec![Arc::new(Block::Para(p))];
        d.body = vec![Arc::new(Block::Table(t)), Arc::new(Block::Para(Paragraph::new()))];
        d
    };

    let rev_styled_path = tmp.path("rev-styled.docx");
    crate::io::save_path(&rev_styled_path, &make_styled_table_doc(rev_png.clone(), true, 200.0)).unwrap();

    let mut s = Session::new(make_styled_table_doc(base_png.clone(), false, 200.0));
    let r = s.run("review.compare", &json!({"path": rev_styled_path.to_string_lossy(), "showIn": "new"})).unwrap();
    assert_eq!(r["changes"], 4, "table, row, cell property changes + replaced cell image count as 4 changes");
    assert!(!r["notHonoured"].as_array().unwrap().iter().any(|v| v == "tables"));
    let t = s.doc.body[0].as_table().unwrap();
    assert!(t.props.fmt_change.is_some(), "table property change recorded");
    assert!(t.rows[0].props.fmt_change.is_some(), "row property change recorded");
    assert!(t.rows[0].cells[0].props.fmt_change.is_some(), "cell property change recorded");

    let cmp_styled_path = tmp.path("cmp-styled.docx");
    crate::io::save_path(&cmp_styled_path, &s.doc).unwrap();
    let reopened = crate::io::open_path(&cmp_styled_path).unwrap();

    let mut accepted = Session::new(reopened.clone());
    accepted.run("review.acceptAll", &json!({})).unwrap();
    let t_acc = accepted.doc.body[0].as_table().unwrap();
    assert_eq!(t_acc.props.align, Some(wordcraft_doc::props::Align::Center));
    assert!(t_acc.rows[0].props.cant_split);
    assert_eq!(t_acc.rows[0].cells[0].props.shading, Some(wordcraft_doc::props::Rgb(0xEE, 0xFF, 0xDD)));

    let mut rejected = Session::new(reopened);
    rejected.run("review.rejectAll", &json!({})).unwrap();
    let t_rej = rejected.doc.body[0].as_table().unwrap();
    assert_eq!(t_rej.props.align, None, "Reject All restores original table alignment");
    assert!(!t_rej.rows[0].props.cant_split, "Reject All restores original row cant_split");
    assert_eq!(t_rej.rows[0].cells[0].props.shading, None, "Reject All restores original cell shading");
    let p_rej = t_rej.rows[0].cells[0].blocks[0].as_para().unwrap();
    let objs_rej: Vec<(usize, &InlineObject)> = p_rej.objects_with_offsets().collect();
    assert_eq!(objs_rej.len(), 1);
    let InlineObject::Image { media: m_rej, .. } = objs_rej[0].1 else { panic!("expected image") };
    assert_eq!(rejected.doc.media.get(m_rej).map(|b| &**b), Some(&base_png));

    // Layout/formatting-only differences with identical media keep single-table behavior.
    let rev_fmt_only_path = tmp.path("rev-fmt-only.docx");
    crate::io::save_path(&rev_fmt_only_path, &make_styled_table_doc(base_png.clone(), true, 200.0)).unwrap();
    let mut s_no_fmt = Session::new(make_styled_table_doc(base_png.clone(), false, 200.0));
    let r_no_fmt =
        s_no_fmt.run("review.compare", &json!({"path": rev_fmt_only_path.to_string_lossy(), "showIn": "new", "formatting": false})).unwrap();
    assert_eq!(r_no_fmt["changes"], 0);
    assert_eq!(s_no_fmt.doc.body.iter().filter(|b| b.as_table().is_some()).count(), 1);

    // Image-free table with identical cell text and only a changed column width:
    // keeps the old single-table behavior without emitting two tables or empty shells.
    let make_grid_doc = |width: f32| -> Document {
        let mut d = Document::new();
        let mut t = wordcraft_doc::table::Table::new(1, 1, width);
        t.rows[0].cells[0].blocks = vec![Arc::new(Block::Para(Paragraph::with_text("Cell", CharProps::default())))];
        d.body = vec![Arc::new(Block::Table(t)), Arc::new(Block::Para(Paragraph::new()))];
        d
    };
    let grid_path = tmp.path("rev-grid.docx");
    crate::io::save_path(&grid_path, &make_grid_doc(260.0)).unwrap();
    let mut s_grid = Session::new(make_grid_doc(200.0));
    let r_grid = s_grid.run("review.compare", &json!({"path": grid_path.to_string_lossy(), "showIn": "new"})).unwrap();
    assert_eq!(r_grid["changes"], 0, "image-free grid width match keeps old single-table behavior");
    assert!(!r_grid["notHonoured"].as_array().unwrap().iter().any(|v| v == "tables"));

    let assert_single_grid_table = |doc: &Document| {
        let tables: Vec<&wordcraft_doc::table::Table> = doc.body.iter().filter_map(|b| b.as_table()).collect();
        assert_eq!(tables.len(), 1, "must keep a single table with no duplicate empty shell");
        assert_eq!(tables[0].grid, vec![260.0]);
        let p = tables[0].rows[0].cells[0].blocks[0].as_para().unwrap();
        assert_eq!(p.plain_text(), "Cell");
        assert!(p.runs.iter().all(|r| r.props.ins.is_none() && r.props.del.is_none()));
    };
    assert_single_grid_table(&s_grid.doc);

    for cmd in ["review.acceptAll", "review.rejectAll"] {
        let mut s_res = Session::new(s_grid.doc.clone());
        s_res.run(cmd, &json!({})).unwrap();
        assert_single_grid_table(&s_res.doc);
    }

    let cmp_grid_path = tmp.path("cmp-grid.docx");
    crate::io::save_path(&cmp_grid_path, &s_grid.doc).unwrap();
    let reopened_grid = crate::io::open_path(&cmp_grid_path).unwrap();
    for cmd in ["review.acceptAll", "review.rejectAll"] {
        let mut s_res = Session::new(reopened_grid.clone());
        s_res.run(cmd, &json!({})).unwrap();
        assert_single_grid_table(&s_res.doc);
    }

    // Replaced image + changed column grid width:
    // - with formatting enabled: incompatible grid width defers structural change and reports `tables`;
    // - with formatting disabled: tracks the replaced image in the single table.
    let grid_img_path = tmp.path("rev-grid-img.docx");
    crate::io::save_path(&grid_img_path, &make_styled_table_doc(rev_png.clone(), false, 260.0)).unwrap();
    let mut s_grid_img = Session::new(make_styled_table_doc(base_png.clone(), false, 200.0));
    let r_grid_img = s_grid_img.run("review.compare", &json!({"path": grid_img_path.to_string_lossy(), "showIn": "new"})).unwrap();
    assert_eq!(r_grid_img["changes"], 0);
    assert!(r_grid_img["notHonoured"].as_array().unwrap().iter().any(|v| v == "tables"));
    assert_eq!(s_grid_img.doc.body.iter().filter(|b| b.as_table().is_some()).count(), 1);
    let t_grid_img = s_grid_img.doc.body[0].as_table().unwrap();
    assert_eq!(t_grid_img.grid, vec![200.0], "unsupported grid+media fallback preserves original table grid");
    let assert_grid_base_img = |doc: &Document, label: &str| {
        let p = doc.body[0].as_table().unwrap().rows[0].cells[0].blocks[0].as_para().unwrap();
        let objs: Vec<(usize, &InlineObject)> = p.objects_with_offsets().collect();
        assert_eq!(objs.len(), 1, "{label}");
        let InlineObject::Image { media, .. } = objs[0].1 else { panic!("expected image") };
        assert_eq!(doc.media.get(media).map(|b| &**b), Some(&base_png), "{label}");
    };
    assert_grid_base_img(&s_grid_img.doc, "unsupported grid+media fallback preserves original image");
    let mut s_grid_img_kept = Session::new(s_grid_img.doc.clone());
    s_grid_img_kept.run("review.rejectAll", &json!({})).unwrap();
    assert_grid_base_img(&s_grid_img_kept.doc, "unsupported grid+media rejectAll keeps original image");

    let mut s_grid_img_nf = Session::new(make_styled_table_doc(base_png.clone(), false, 200.0));
    let r_grid_img_nf =
        s_grid_img_nf.run("review.compare", &json!({"path": grid_img_path.to_string_lossy(), "showIn": "new", "formatting": false})).unwrap();
    assert_eq!(r_grid_img_nf["changes"], 1, "with formatting false, replaced image inside grid-changed table is tracked");
    assert!(!r_grid_img_nf["notHonoured"].as_array().unwrap().iter().any(|v| v == "tables"));
    assert_eq!(s_grid_img_nf.doc.body.iter().filter(|b| b.as_table().is_some()).count(), 1);
    let mut s_grid_img_rej = Session::new(s_grid_img_nf.doc.clone());
    s_grid_img_rej.run("review.rejectAll", &json!({})).unwrap();
    let p_grid_rej = s_grid_img_rej.doc.body[0].as_table().unwrap().rows[0].cells[0].blocks[0].as_para().unwrap();
    let objs_grid_rej: Vec<(usize, &InlineObject)> = p_grid_rej.objects_with_offsets().collect();
    assert_eq!(objs_grid_rej.len(), 1);
    let InlineObject::Image { media: m_grid_rej, .. } = objs_grid_rej[0].1 else { panic!("expected image") };
    assert_eq!(s_grid_img_rej.doc.media.get(m_grid_rej).map(|b| &**b), Some(&base_png));
}

#[test]
fn compare_with_a_revised_file_keeps_its_text_boxes_backed() {
    // Text boxes in both documents, the revised one from a file: every anchor in the result
    // shows a text box story of the result, and both documents' boxes are there, marked.
    fn with_boxes(text: &str, boxes: &[(usize, usize, &str)]) -> Session {
        let mut s = Session::new(Document::from_text(text));
        for (para, off, text) in boxes {
            s.run("caret.set", &json!({"pos": Pos::body(*para, *off)})).unwrap();
            s.run("insert.textBox", &json!({"text": text})).unwrap();
        }
        s
    }
    /// Each text box anchored in the body: its story's text, and whether its anchor is deleted
    /// or inserted.
    fn boxes(d: &Document) -> Vec<(String, bool, bool)> {
        let mut out = Vec::new();
        for path in d.para_paths(StoryRef::Body) {
            let p = d.para(StoryRef::Body, &path).unwrap();
            for (off, o) in p.objects_with_offsets() {
                for id in o.text_boxes() {
                    let part = d.parts.get(&id).unwrap_or_else(|| panic!("an anchor shows story {id}, which the result lacks"));
                    assert_eq!(part.kind, wordcraft_doc::PartKind::TextBox, "story {id}");
                    let c = p.props_of_char(off);
                    out.push((d.plain_text(StoryRef::Part(id)), c.del.is_some(), c.ins.is_some()));
                }
            }
        }
        out.sort();
        out
    }
    let rev = with_boxes("Body one.\nNew middle.\nKeep.", &[(1, "New middle.".len(), "Revised box."), (2, "Keep.".len(), "Kept box.")]);
    let path = std::env::temp_dir().join(format!("wc-compare-boxes-{}.docx", std::process::id()));
    crate::io::save_path(&path, &rev.doc).unwrap();
    let t = |t: &str, del, ins| (t.to_string(), del, ins);
    for show_in in ["original", "revised", "new"] {
        for (then, left) in [("review.acceptAll", "Revised box."), ("review.rejectAll", "Original box.")] {
            let mut s = with_boxes("Body one.\nOld middle.\nKeep.", &[(1, "Old middle.".len(), "Original box."), (2, "Keep.".len(), "Kept box.")]);
            s.run("review.compare", &json!({"path": path.to_string_lossy(), "showIn": show_in})).unwrap();
            assert_eq!(boxes(&s.doc), [t("Kept box.", false, false), t("Original box.", true, false), t("Revised box.", false, true)], "{show_in}");
            s.run(then, &json!({})).unwrap();
            assert_eq!(boxes(&s.doc), [t("Kept box.", false, false), t(left, false, false)], "{show_in} {then}");
        }
    }
    let _ = std::fs::remove_file(&path);
}

#[test]
fn compare_bounds_total_diff_work() {
    // Many long, slightly different paragraphs: each pair alone fits one comparison table,
    // but the work over all pairs must stay bounded. Pairs past the budget are compared
    // whole (deleted and inserted), and every pair still counts as a change.
    let words = 3000;
    let paras = 12;
    let doc = |rev: bool| -> String {
        (0..paras)
            .map(|p| (0..words).map(|w| if w % 10 == 0 && rev { format!("x{p}w{w}") } else { format!("p{p}w{w}") }).collect::<Vec<_>>().join(" "))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let (original, revised) = (doc(false), doc(true));
    let mut s = Session::new(Document::from_text(&original));
    let r = s.run("review.compare", &json!({"text": revised})).unwrap();
    assert_eq!(r["changes"], paras);
    // A pair diffed word by word keeps most of its text unmarked; one replaced whole keeps
    // only its common start and end.
    let word_diffed = |b: &Arc<Block>| {
        let p = b.as_para().unwrap();
        let same: usize = p.run_ranges().filter(|(_, c)| c.ins.is_none() && c.del.is_none()).map(|(r, _)| r.len()).sum();
        same * 2 > p.text.len()
    };
    assert!(word_diffed(&s.doc.body[0]));
    let diffed = s.doc.body.iter().filter(|b| word_diffed(b)).count() as u64;
    let cells = (words as u64) * (words as u64);
    assert!(diffed * cells <= 50_000_000 + cells, "{diffed} pairs diffed word by word");
    assert!(!word_diffed(s.doc.body.last().unwrap()));
    s.run("review.acceptAll", &json!({})).unwrap();
    assert_eq!(s.doc.plain_text(StoryRef::Body), revised);
}

/// Verifies that moving a paragraph past `MAX_LCS` in a long document does not cascade into false
/// revisions in the tail, asserting exact changed block indices, text, and edit types.
#[test]
fn compare_lcs_window_move_past_cap_does_not_cascade_false_revisions() {
    // Issue #6110: 4,600 blocks where prefix/suffix trim is zeroed (substitutions at blocks 0
    // and 4599) and a record moves from index 100 to index 4400 (past MAX_LCS = 4000).
    // Without chunking/continuation past the window, the tail misaligns and positional pairing
    // marks ~202 paragraphs instead of the true 4 changes (substitutions at 0 and 4599,
    // deletion at 100, insertion at 4400).
    let total = 4600;
    let mut orig_paras: Vec<String> = (0..total).map(|i| format!("Record {i} has alpha beta gamma payload.")).collect();
    let mut rev_paras = orig_paras.clone();
    // Zero out prefix trim: substitution at 0
    orig_paras[0] = "Record 0 has alpha beta gamma original.".into();
    rev_paras[0] = "Record 0 has alpha beta gamma revised.".into();
    // Zero out suffix trim: substitution at 4599
    orig_paras[total - 1] = format!("Record {} has alpha beta gamma original.", total - 1);
    rev_paras[total - 1] = format!("Record {} has alpha beta gamma revised.", total - 1);
    // Move record 100 to 4400
    let moved = orig_paras[100].clone();
    rev_paras.remove(100);
    rev_paras.insert(4400, moved);

    let original = orig_paras.join("\n");
    let revised = rev_paras.join("\n");

    let mut s = Session::new(Document::from_text(&original));
    let r = s.run("review.compare", &json!({"text": revised})).unwrap();

    // The true number of changed paragraphs is 4 (the two substitutions, the move's deletion,
    // and the move's insertion). On unpatched code, window truncation misaligned the tail
    // and marked ~202 paragraphs.
    assert_eq!(r["changes"], 4, "move past MAX_LCS cap must not cascade into false revisions in the tail");

    // In s.doc.body (total 4601 blocks: 4598 unchanged, 1 del at 100, 1 ins at 4401, 2 substituted at 0 and 4600):
    let changed_blocks: Vec<(usize, String)> = s
        .doc
        .body
        .iter()
        .enumerate()
        .filter_map(|(idx, b)| {
            let p = b.as_para()?;
            let has_change = p.mark.del.is_some() || p.mark.ins.is_some() || p.runs.iter().any(|r| r.props.del.is_some() || r.props.ins.is_some());
            if has_change { Some((idx, p.plain_text())) } else { None }
        })
        .collect();

    assert_eq!(changed_blocks.len(), 4);
    assert_eq!(changed_blocks[0], (0, "Record 0 has alpha beta gamma original.revised.".into()));
    assert_eq!(changed_blocks[1], (100, "Record 100 has alpha beta gamma payload.".into()));
    assert_eq!(changed_blocks[2], (4401, "Record 100 has alpha beta gamma payload.".into()));
    assert_eq!(changed_blocks[3], (4600, "Record 4599 has alpha beta gamma original.revised.".into()));

    // Verify exact change types on those blocks:
    let b0 = s.doc.body[0].as_para().unwrap();
    assert!(b0.runs.iter().any(|r| r.props.del.is_some()) && b0.runs.iter().any(|r| r.props.ins.is_some()), "block 0 is text replacement");
    let b100 = s.doc.body[100].as_para().unwrap();
    assert!(b100.runs.iter().all(|r| r.props.del.is_some()) && b100.mark.del.is_some(), "block 100 is deleted whole");
    let b4401 = s.doc.body[4401].as_para().unwrap();
    assert!(b4401.runs.iter().all(|r| r.props.ins.is_some()) && b4401.mark.ins.is_some(), "block 4401 is inserted whole");
    let b4600 = s.doc.body[4600].as_para().unwrap();
    assert!(b4600.runs.iter().any(|r| r.props.del.is_some()) && b4600.runs.iter().any(|r| r.props.ins.is_some()), "block 4600 is text replacement");

    s.run("review.acceptAll", &json!({})).unwrap();
    assert_eq!(s.doc.plain_text(StoryRef::Body), revised);
}

/// Verifies that `mark_block` records paragraph mark revisions on whole-block deletions,
/// whole-block insertions, empty paragraphs, and ensures clean resolution via `review.acceptAll`.
#[test]
fn compare_mark_block_records_paragraph_mark_revisions_on_whole_block_changes() {
    // Whole block delete/insert: test with differing paragraph count so whole-block paths trigger
    let orig_multi = "Keep start.\nDelete me.\nKeep end.";
    let rev_multi = "Keep start.\nKeep end.";
    let mut s2 = Session::new(Document::from_text(orig_multi));
    if let Some(Block::Para(p)) = s2.doc.body.get_mut(1).map(Arc::make_mut) {
        p.mark.bold = Some(true);
    }
    s2.run("review.compare", &json!({"text": rev_multi})).unwrap();

    // In s2.doc.body, block 1 was deleted whole.
    // Verify that p.mark.del is recorded by mark_block and formatting is preserved.
    let del_para = s2.doc.body[1].as_para().expect("block 1 must be paragraph");
    assert!(del_para.mark.del.is_some(), "whole-block delete must record revision on paragraph mark");
    assert_eq!(del_para.mark.bold, Some(true), "paragraph mark formatting must be preserved on deleted block");

    // Accept all changes: deleted paragraph is removed
    s2.run("review.acceptAll", &json!({})).unwrap();
    assert_eq!(s2.doc.plain_text(StoryRef::Body), rev_multi);

    // Test whole-block insertion
    let mut s3 = Session::new(Document::from_text(rev_multi));
    s3.run("review.compare", &json!({"text": orig_multi})).unwrap();
    let ins_para = s3.doc.body[1].as_para().expect("block 1 must be paragraph");
    assert!(ins_para.mark.ins.is_some(), "whole-block insert must record revision on paragraph mark");
    s3.run("review.acceptAll", &json!({})).unwrap();
    assert_eq!(s3.doc.plain_text(StoryRef::Body), orig_multi);

    // Test empty paragraph deletion and insertion (len == 0, where only mark carries the revision)
    let orig_empty = "Before.\n\nAfter.";
    let rev_empty = "Before.\nAfter.";
    let mut s4 = Session::new(Document::from_text(orig_empty));
    s4.run("review.compare", &json!({"text": rev_empty})).unwrap();
    let empty_del = s4.doc.body[1].as_para().expect("empty paragraph block");
    assert_eq!(empty_del.len(), 0);
    assert!(empty_del.mark.del.is_some(), "empty paragraph deletion must set mark.del");
    s4.run("review.acceptAll", &json!({})).unwrap();
    assert_eq!(s4.doc.plain_text(StoryRef::Body), rev_empty);

    let mut s5 = Session::new(Document::from_text(rev_empty));
    s5.run("review.compare", &json!({"text": orig_empty})).unwrap();
    let empty_ins = s5.doc.body[1].as_para().expect("empty paragraph block");
    assert_eq!(empty_ins.len(), 0);
    assert!(empty_ins.mark.ins.is_some(), "empty paragraph insertion must set mark.ins");
    s5.run("review.acceptAll", &json!({})).unwrap();
    assert_eq!(s5.doc.plain_text(StoryRef::Body), orig_empty);
}

#[test]
fn macros_record_and_run() {
    let mut s = Session::new(Document::new());
    s.run("tools.recordMacro", &json!({"name": "hi"})).unwrap();
    s.run("text.insert", &json!({"text": "Hi "})).unwrap();
    s.run("format.bold", &json!({})).unwrap();
    s.run("tools.recordMacro", &json!({})).unwrap();
    s.run("tools.macros", &json!({"run": "hi"})).unwrap();
    assert_eq!(s.doc.plain_text(StoryRef::Body), "Hi Hi ");
    s.run("edit.repeat", &json!({})).unwrap();
}

#[test]
fn autocorrect_on_space() {
    let mut s = Session::new(Document::new());
    for t in ["teh", " ", "Teh", " ", "(c)", " "] {
        s.run("text.insert", &json!({"text": t})).unwrap();
    }
    // The first word of a sentence is capitalised too.
    assert_eq!(s.doc.plain_text(StoryRef::Body), "The The © ");
}

#[test]
fn protection_blocks_edits() {
    let mut s = Session::new(Document::from_text("locked"));
    s.run("review.restrict", &json!({"mode": "readOnly"})).unwrap();
    assert!(s.run("text.insert", &json!({"text": "x"})).is_err());
    // The "none" mode is the shorthand for stopping the protection, and it doesn't stop a
    // protected document: only the explicit stop does (#1327 follow-up).
    assert!(s.run("review.restrict", &json!({"mode": "none"})).is_err());
    assert!(s.run("text.insert", &json!({"text": "x"})).is_err());
    s.run("review.restrict", &json!({"stop": true})).unwrap();
    assert!(s.run("text.insert", &json!({"text": "x"})).is_ok());
}

#[test]
fn accessibility_and_quick_parts() {
    let mut s = Session::new(crate::sample::sample_document());
    let r = s.run("file.accessibility", &json!({})).unwrap();
    assert!(r["issues"].is_array());
    s.run("select.text", &json!({"text": "Membership"})).unwrap();
    s.run("insert.quickParts", &json!({"save": "m"})).unwrap();
    s.run("caret.docEnd", &json!({})).unwrap();
    s.run("insert.quickParts", &json!({"insert": "m"})).unwrap();
    assert!(s.doc.plain_text(StoryRef::Body).ends_with("Membership"));
}

/// #407 Language: the selection's language and "do not check", the default language and
/// the detect preference; a bad tag is refused.
#[test]
fn proofing_language_options() {
    let mut s = Session::new(Document::from_text("Bonjour"));
    s.run("select.all", &json!({})).unwrap();
    s.run("review.language", &json!({"lang": "fr-FR", "noProof": true, "detect": false})).unwrap();
    let rc = s.doc.para_at(&Pos::body(0, 1)).map(|p| p.props_at(1).clone()).unwrap();
    assert_eq!((rc.lang.as_deref(), rc.no_proof), (Some("fr-FR"), Some(true)));
    assert!(!s.prefs.detect_language);
    s.run("review.language", &json!({"lang": "de-DE", "default": true})).unwrap();
    assert_eq!(s.doc.styles.default_chr.lang.as_deref(), Some("de-DE"));
    assert!(s.run("review.language", &json!({"lang": "<script>"})).is_err());
}

/// #2242: `insert.spreadsheet` fills a table from `values`, with a repeating header and a style.
#[test]
fn spreadsheet_takes_values_header_and_style() {
    let mut s = Session::new(Document::from_text(""));
    s.run("insert.spreadsheet", &json!({"values": [["Month", "Cases"], ["Jan", 12], ["Feb", null]], "header": true, "style": "TableGrid"})).unwrap();
    let Some(Block::Table(t)) = s.doc.body.iter().map(|b| &**b).find(|b| matches!(b, Block::Table(_))) else { panic!("no table") };
    assert_eq!((t.rows.len(), t.rows[0].cells.len()), (3, 2));
    assert!(t.rows[0].props.header && !t.rows[1].props.header);
    assert_eq!(t.rows[1].cells[1].blocks[0].as_para().unwrap().text, "12");
    assert_eq!(t.props.style.as_deref(), Some("TableGrid"));
}

/// An explicit `values: null` is a parameter error, not the legacy CSV sample table: the document
/// and the undo history stay as they were. Only an absent `values` falls back to `csv`.
#[test]
fn spreadsheet_rejects_an_explicit_null_values_without_changing_anything() {
    let mut s = Session::new(Document::from_text("keep"));
    let depth = s.undo_depth();
    for args in [json!({"values": null}), json!({"values": null, "csv": "a,b\n1,2"})] {
        assert!(s.run("insert.spreadsheet", &args).is_err(), "{args}");
        assert_eq!(s.doc.plain_text(StoryRef::Body), "keep");
        assert!(!s.doc.body.iter().any(|b| matches!(&**b, Block::Table(_))));
        assert_eq!(s.undo_depth(), depth, "{args}: no undo step");
    }
    s.run("insert.spreadsheet", &json!({"csv": "a,b\n1,2"})).unwrap();
    let Some(Block::Table(t)) = s.doc.body.iter().map(|b| &**b).find(|b| matches!(b, Block::Table(_))) else { panic!("no table") };
    assert_eq!((t.rows.len(), t.rows[0].cells.len()), (2, 2), "an absent `values` still takes the csv");
}

/// #2242: malformed `values` is an error, never a panic, and leaves the document alone.
#[test]
fn spreadsheet_rejects_hostile_values() {
    let wide: Vec<Value> = (0..64).map(|i| json!(i)).collect();
    let many: Vec<Value> = (0..1001).map(|_| json!(["x"])).collect();
    for bad in [
        json!("x"),
        json!(5),
        json!([]),
        json!(["a"]),
        json!([["a", "b"], ["c"]]),
        json!([wide]),
        json!(many),
        json!([[{"a": 1}]]),
        json!([[[1]]]),
        json!([[]]),
    ] {
        let mut s = Session::new(Document::from_text("keep"));
        assert!(s.run("insert.spreadsheet", &json!({"values": bad})).is_err(), "{bad}");
        assert_eq!(s.doc.plain_text(StoryRef::Body), "keep");
        assert!(!s.doc.body.iter().any(|b| matches!(&**b, Block::Table(_))));
    }
}

/// The inspector's removals delete straight from the document, so they aren't tracked: on a
/// document that tracks changes the text would go with no revision to show for it, and the
/// revision trail would be broken behind the user's back. The removal is refused instead,
/// and the document is left as it was.
#[test]
fn inspect_removals_are_refused_while_the_document_tracks_changes() {
    let mut s = Session::new(Document::from_text("Visible text.\nHidden text."));
    s.run("select.text", &json!({"text": "Hidden"})).unwrap();
    s.run("format.hidden", &json!({"value": true})).unwrap();
    // Tracking without Restrict Editing isolates the Inspector guard from the protected
    // document check added separately in #3285.
    s.run("review.trackChanges", &json!({"value": true})).unwrap();
    assert!(s.doc.settings.track_changes, "the document doesn't track changes");
    let before = s.doc.clone();
    let err = s.run("file.inspect", &json!({"remove": ["hidden"]})).unwrap_err();
    assert!(matches!(err, CmdError::Disabled(_)), "{err}");
    assert_eq!(s.doc, before, "the document changed despite the refusal");
    // Reading the document stays allowed.
    let found = s.run("file.inspect", &json!({})).unwrap();
    assert_eq!(found["found"]["hiddenText"], true, "{found}");
}

/// A document that doesn't track changes is inspected as before: the hidden text goes.
#[test]
fn inspect_removals_work_when_the_document_does_not_track_changes() {
    let mut s = Session::new(Document::from_text("Visible text.\nHidden text."));
    s.run("select.text", &json!({"text": "Hidden"})).unwrap();
    s.run("format.hidden", &json!({"value": true})).unwrap();
    assert!(!s.doc.settings.track_changes);
    let out = s.run("file.inspect", &json!({"remove": ["hidden"]})).unwrap();
    assert_eq!(out["residual"], json!([]), "{out}");
    let text = s.doc.plain_text(StoryRef::Body);
    assert!(!text.contains("Hidden"), "the hidden text is still there: {text}");
    assert!(text.contains("Visible text."), "{text}");
}

/// Helper that creates a session with a body paragraph and an attached footnote or endnote.
fn session_with_note(body: &str, note_text: &str, endnote: bool) -> Session {
    let mut s = Session::new(Document::from_text(body));
    s.run("select.all", &json!({})).unwrap();
    s.run("caret.end", &json!({})).unwrap();
    let cmd = if endnote { "references.endnote" } else { "references.footnote" };
    s.run(cmd, &json!({"text": note_text})).unwrap();
    s
}

/// #5842: Compare diffs footnote content between the original and revised documents,
/// tracking word deletions and insertions inside the footnote story without marking
/// the unchanged body reference as replaced.
#[test]
fn compare_diffs_footnote_text_changes() {
    let orig = session_with_note("Body paragraph.", "original note text", false);
    let rev = session_with_note("Body paragraph.", "revised note text", false);
    let rev_path = std::env::temp_dir().join(format!("wc-rev-fn-{}.docx", std::process::id()));
    crate::io::save_path(&rev_path, &rev.doc).unwrap();

    let mut s = orig;
    let r = s.run("review.compare", &json!({"path": rev_path.to_string_lossy()})).unwrap();
    let _ = std::fs::remove_file(&rev_path);

    assert!(r["changes"].as_u64().unwrap() >= 1, "footnote changes count: {r}");
    // The body reference is unchanged.
    let p = s.doc.body[0].as_para().unwrap();
    assert!(p.runs.iter().all(|r| r.props.del.is_none() && r.props.ins.is_none()));

    // The footnote story holds the tracked changes.
    let fn_part = s.doc.parts.values().find(|p| p.kind == wordcraft_doc::PartKind::Footnote).unwrap();
    let fn_para = fn_part.blocks[0].as_para().unwrap();
    let deleted: Vec<String> = fn_para.run_ranges().filter(|(_, c)| c.del.is_some()).map(|(r, _)| fn_para.text[r].to_string()).collect();
    let inserted: Vec<String> = fn_para.run_ranges().filter(|(_, c)| c.ins.is_some()).map(|(r, _)| fn_para.text[r].to_string()).collect();
    assert!(deleted.iter().any(|s| s.contains("original")), "deleted text in note: {deleted:?}");
    assert!(inserted.iter().any(|s| s.contains("revised")), "inserted text in note: {inserted:?}");

    // Accepting all changes applies the revised footnote text.
    s.run("review.acceptAll", &json!({})).unwrap();
    let fn_part = s.doc.parts.values().find(|p| p.kind == wordcraft_doc::PartKind::Footnote).unwrap();
    assert!(fn_part.blocks[0].as_para().unwrap().plain_text().contains("revised note text"));

    // Rejecting all changes restores the original footnote text.
    let orig2 = session_with_note("Body paragraph.", "original note text", false);
    let rev2 = session_with_note("Body paragraph.", "revised note text", false);
    let rev_path2 = std::env::temp_dir().join(format!("wc-rev-fn2-{}.docx", std::process::id()));
    crate::io::save_path(&rev_path2, &rev2.doc).unwrap();
    let mut s2 = orig2;
    s2.run("review.compare", &json!({"path": rev_path2.to_string_lossy()})).unwrap();
    let _ = std::fs::remove_file(&rev_path2);
    s2.run("review.rejectAll", &json!({})).unwrap();
    let fn_part2 = s2.doc.parts.values().find(|p| p.kind == wordcraft_doc::PartKind::Footnote).unwrap();
    assert!(fn_part2.blocks[0].as_para().unwrap().plain_text().contains("original note text"));
}

/// #5842: Compare records paragraph formatting changes within footnotes as `w:pPrChange`.
#[test]
fn compare_diffs_footnote_formatting_changes() {
    use wordcraft_doc::props::Align;
    let orig = session_with_note("Body paragraph.", "formatted note text", false);
    let mut rev = session_with_note("Body paragraph.", "formatted note text", false);
    // Change paragraph alignment in the revised footnote.
    let fn_id = rev.doc.parts.iter().find(|(_, p)| p.kind == wordcraft_doc::PartKind::Footnote).map(|(k, _)| *k).unwrap();
    let p = rev.doc.para_mut(StoryRef::Part(fn_id), &wordcraft_doc::Path::top(0)).unwrap();
    p.props.align = Some(Align::Center);
    let rev_path = std::env::temp_dir().join(format!("wc-rev-fnfmt-{}.docx", std::process::id()));
    crate::io::save_path(&rev_path, &rev.doc).unwrap();

    let mut s = orig;
    let r = s.run("review.compare", &json!({"path": rev_path.to_string_lossy()})).unwrap();
    let _ = std::fs::remove_file(&rev_path);

    assert_eq!(r["changes"], 1, "formatting change counted: {r}");
    let fn_part = s.doc.parts.get(&fn_id).unwrap();
    let fn_para = fn_part.blocks[0].as_para().unwrap();
    assert!(fn_para.props.fmt_change.is_some(), "footnote paragraph has pPrChange");
    assert_eq!(fn_para.props.align, Some(Align::Center));

    // Rejecting restores original paragraph alignment.
    s.run("review.rejectAll", &json!({})).unwrap();
    let fn_part = s.doc.parts.get(&fn_id).unwrap();
    let fn_para = fn_part.blocks[0].as_para().unwrap();
    assert!(fn_para.props.fmt_change.is_none());
    assert_eq!(fn_para.props.align, None);
}

/// #5842: Compare diffs endnote stories identically to footnotes, tracking edits in endnotes.
#[test]
fn compare_diffs_endnotes() {
    let orig = session_with_note("Body paragraph.", "original endnote", true);
    let rev = session_with_note("Body paragraph.", "revised endnote", true);
    let rev_path = std::env::temp_dir().join(format!("wc-rev-en-{}.docx", std::process::id()));
    crate::io::save_path(&rev_path, &rev.doc).unwrap();

    let mut s = orig;
    let r = s.run("review.compare", &json!({"path": rev_path.to_string_lossy()})).unwrap();
    let _ = std::fs::remove_file(&rev_path);

    assert!(r["changes"].as_u64().unwrap() >= 1, "endnote changes counted: {r}");
    let en_part = s.doc.parts.values().find(|p| p.kind == wordcraft_doc::PartKind::Endnote).unwrap();
    let en_para = en_part.blocks[0].as_para().unwrap();
    assert!(en_para.runs.iter().any(|r| r.props.del.is_some() || r.props.ins.is_some()));

    s.run("review.acceptAll", &json!({})).unwrap();
    let en_part = s.doc.parts.values().find(|p| p.kind == wordcraft_doc::PartKind::Endnote).unwrap();
    assert!(en_part.blocks[0].as_para().unwrap().plain_text().contains("revised endnote"));
}

/// #5842: When `footnotes` is explicitly false, footnote diffing is skipped and footnotes are
/// carried over verbatim from the original document, and `footnotes` is not in `notHonoured`.
#[test]
fn compare_honours_footnotes_option_disabled() {
    let orig = session_with_note("Body paragraph.", "original note", false);
    let rev = session_with_note("Body paragraph.", "revised note", false);
    let rev_path = std::env::temp_dir().join(format!("wc-rev-fndis-{}.docx", std::process::id()));
    crate::io::save_path(&rev_path, &rev.doc).unwrap();

    let mut s = orig;
    let r = s.run("review.compare", &json!({"path": rev_path.to_string_lossy(), "footnotes": false})).unwrap();
    let _ = std::fs::remove_file(&rev_path);

    assert_eq!(r["changes"], 0, "no changes when footnotes option disabled");
    let not_honoured = r["notHonoured"].as_array().unwrap();
    assert!(!not_honoured.iter().any(|v| v == "footnotes"), "footnotes option is honoured: {not_honoured:?}");

    let fn_part = s.doc.parts.values().find(|p| p.kind == wordcraft_doc::PartKind::Footnote).unwrap();
    let fn_para = fn_part.blocks[0].as_para().unwrap();
    assert!(fn_para.plain_text().contains("original note"));
    assert!(fn_para.runs.iter().all(|r| r.props.del.is_none() && r.props.ins.is_none()));
}

/// #5842: Compare correctly handles inserted and deleted footnotes across documents,
/// tracking the deletion of the body reference and adopting newly added notes.
#[test]
fn compare_handles_inserted_and_deleted_footnotes() {
    // Original has 2 footnotes.
    let mut orig = Session::new(Document::from_text("Para one.\nPara two."));
    orig.run("caret.set", &json!({"pos": Pos::body(0, "Para one.".len())})).unwrap();
    orig.run("references.footnote", &json!({"text": "First note."})).unwrap();
    orig.run("caret.set", &json!({"pos": Pos::body(1, "Para two.".len())})).unwrap();
    orig.run("references.footnote", &json!({"text": "Second note."})).unwrap();

    // Revised deletes the first footnote, keeps the second, and inserts a third.
    let mut rev = Session::new(Document::from_text("Para one.\nPara two.\nPara three."));
    rev.run("caret.set", &json!({"pos": Pos::body(1, "Para two.".len())})).unwrap();
    rev.run("references.footnote", &json!({"text": "Second note."})).unwrap();
    rev.run("caret.set", &json!({"pos": Pos::body(2, "Para three.".len())})).unwrap();
    rev.run("references.footnote", &json!({"text": "Third note."})).unwrap();

    let fn1_id = orig.doc.body[0]
        .as_para()
        .unwrap()
        .objects
        .iter()
        .find_map(|o| if let InlineObject::NoteRef { id, .. } = o { Some(*id) } else { None })
        .unwrap();
    let fn2_id = orig.doc.body[1]
        .as_para()
        .unwrap()
        .objects
        .iter()
        .find_map(|o| if let InlineObject::NoteRef { id, .. } = o { Some(*id) } else { None })
        .unwrap();

    let rev_path = std::env::temp_dir().join(format!("wc-rev-fndel-{}.docx", std::process::id()));
    crate::io::save_path(&rev_path, &rev.doc).unwrap();

    let mut s = orig;
    let r = s.run("review.compare", &json!({"path": rev_path.to_string_lossy()})).unwrap();
    let _ = std::fs::remove_file(&rev_path);

    assert!(r["changes"].as_u64().unwrap() >= 1);
    // Para one has deleted footnote reference.
    let p1 = s.doc.body[0].as_para().unwrap();
    let deleted_ref = p1.runs.iter().any(|r| r.props.del.is_some());
    assert!(deleted_ref, "deleted footnote reference mark is tracked as del");

    // Para two has unchanged footnote reference.
    let p2 = s.doc.body[1].as_para().unwrap();
    let unchanged_ref = p2.runs.iter().all(|r| r.props.del.is_none() && r.props.ins.is_none());
    assert!(unchanged_ref, "unchanged footnote reference is retained");

    // Deleted footnote part content is marked with del.
    let fn1_part = s.doc.parts.get(&fn1_id).unwrap();
    assert!(fn1_part.blocks[0].as_para().unwrap().runs.iter().all(|r| r.props.del.is_some()), "deleted note content marked with del");

    // Unchanged footnote part content has no del or ins.
    let fn2_part = s.doc.parts.get(&fn2_id).unwrap();
    assert!(
        fn2_part.blocks[0].as_para().unwrap().runs.iter().all(|r| r.props.del.is_none() && r.props.ins.is_none()),
        "unchanged note content has no tracked marks"
    );

    // Para three has inserted footnote reference, and inserted footnote part content is marked with ins.
    let p3 = s.doc.body.iter().filter_map(|b| b.as_para()).find(|p| p.plain_text().contains("Para three")).unwrap();
    let fn3_id = p3.objects.iter().find_map(|o| if let InlineObject::NoteRef { id, .. } = o { Some(*id) } else { None }).unwrap();
    let fn3_part = s.doc.parts.get(&fn3_id).unwrap();
    assert!(fn3_part.blocks[0].as_para().unwrap().runs.iter().all(|r| r.props.ins.is_some()), "inserted note content marked with ins");
}

/// #5842: When original has two footnotes (note 1 and note 2), and revised deletes note 1
/// and lightly edits note 2, note 2 must be paired with note 2 (not note 1).
/// Paragraph 2 retains its unchanged footnote reference, and note 2 content diffs cleanly.
#[test]
fn compare_pairs_notes_across_deletion_and_edit() {
    let mut orig = Session::new(Document::from_text("Para one.\nPara two."));
    orig.run("caret.set", &json!({"pos": Pos::body(0, "Para one.".len())})).unwrap();
    orig.run("references.footnote", &json!({"text": "First footnote content."})).unwrap();
    orig.run("caret.set", &json!({"pos": Pos::body(1, "Para two.".len())})).unwrap();
    orig.run("references.footnote", &json!({"text": "Second footnote content."})).unwrap();

    // Revised deletes footnote 1 from Para one and lightly edits footnote 2 in Para two.
    let mut rev = Session::new(Document::from_text("Para one.\nPara two."));
    rev.run("caret.set", &json!({"pos": Pos::body(1, "Para two.".len())})).unwrap();
    rev.run("references.footnote", &json!({"text": "Second footnote content revised."})).unwrap();

    let rev_path = std::env::temp_dir().join(format!("wc-rev-fnpair-{}.docx", std::process::id()));
    crate::io::save_path(&rev_path, &rev.doc).unwrap();

    let mut s = orig;
    let r = s.run("review.compare", &json!({"path": rev_path.to_string_lossy()})).unwrap();
    let _ = std::fs::remove_file(&rev_path);

    assert!(r["changes"].as_u64().unwrap() >= 1);

    // Para one: footnote reference was deleted.
    let p1 = s.doc.body[0].as_para().unwrap();
    let p1_has_del_ref = p1.runs.iter().any(|r| r.props.del.is_some());
    assert!(p1_has_del_ref, "Para one footnote reference is tracked as deleted");

    // Para two: footnote reference to Note 2 must be retained without deletion/insertion markup!
    let p2 = s.doc.body[1].as_para().unwrap();
    let p2_ref_clean = p2.runs.iter().all(|r| r.props.del.is_none() && r.props.ins.is_none());
    assert!(p2_ref_clean, "Para two retains footnote reference cleanly without del/ins markup");

    // Note 2 part: must contain the edit ("revised" inserted), rather than note 1 being rewritten.
    let fn2_id = p2.objects.iter().find_map(|o| if let InlineObject::NoteRef { id, .. } = o { Some(*id) } else { None }).unwrap();
    let fn2_part = s.doc.parts.get(&fn2_id).unwrap();
    let fn2_text = fn2_part.blocks[0].as_para().unwrap().plain_text();
    assert!(fn2_text.contains("Second footnote content"), "Note 2 content retained: {fn2_text}");
    let has_ins = fn2_part.blocks[0].as_para().unwrap().runs.iter().any(|r| r.props.ins.is_some());
    assert!(has_ins, "Note 2 tracks insertion of revised text");
}

/// #5842: DOCX roundtrip preserves footnote tracked revisions (`w:ins`, `w:del`) produced by compare.
#[test]
fn compare_footnote_docx_roundtrip() {
    let orig = session_with_note("Body sentence.", "alpha note content", false);
    let rev = session_with_note("Body sentence.", "beta note content", false);
    let orig_path = std::env::temp_dir().join(format!("wc-orig-rt-{}.docx", std::process::id()));
    let rev_path = std::env::temp_dir().join(format!("wc-rev-rt-{}.docx", std::process::id()));
    let out_path = std::env::temp_dir().join(format!("wc-out-rt-{}.docx", std::process::id()));

    crate::io::save_path(&orig_path, &orig.doc).unwrap();
    crate::io::save_path(&rev_path, &rev.doc).unwrap();

    let mut s = Session::new(Document::new());
    s.run(
        "review.compare",
        &json!({
            "original": orig_path.to_string_lossy(),
            "path": rev_path.to_string_lossy()
        }),
    )
    .unwrap();

    crate::io::save_path(&out_path, &s.doc).unwrap();
    let loaded = crate::io::open_path(&out_path).unwrap();

    let _ = std::fs::remove_file(&orig_path);
    let _ = std::fs::remove_file(&rev_path);
    let _ = std::fs::remove_file(&out_path);

    let fn_part = loaded.parts.values().find(|p| p.kind == wordcraft_doc::PartKind::Footnote).unwrap();
    let fn_para = fn_part.blocks[0].as_para().unwrap();
    assert!(fn_para.runs.iter().any(|r| r.props.del.is_some()), "del preserved in docx footnotes.xml");
    assert!(fn_para.runs.iter().any(|r| r.props.ins.is_some()), "ins preserved in docx footnotes.xml");
}

/// #5842: Verifies that `lcs_len` returns genuine in-order common subsequences even when
/// the comparison cell budget is exhausted (`budget = 0`), without allocating DP tables.
#[test]
fn lcs_len_exhausted_budget_bounded_fallback() {
    let mut zero_budget = 0u64;

    // 1. Identical slices: matches completely via prefix/suffix.
    let identical_a = vec!["alpha", "beta", "gamma", "delta"];
    let identical_b = vec!["alpha", "beta", "gamma", "delta"];
    assert_eq!(super::lcs_len(&identical_a, &identical_b, &mut zero_budget), 4);
    assert_eq!(zero_budget, 0);

    // 2. Single word substitution in middle: prefix and suffix match surrounding tokens.
    let mid_a = vec!["start", "word_a", "middle", "end"];
    let mid_b = vec!["start", "word_b", "middle", "end"];
    assert_eq!(super::lcs_len(&mid_a, &mid_b, &mut zero_budget), 3);

    // 3. Edits at both ends: greedy scan matches unchanged core tokens in order.
    let core_a = vec!["head1", "core_x", "core_y", "tail1"];
    let core_b = vec!["head2", "core_x", "core_y", "tail2"];
    assert_eq!(super::lcs_len(&core_a, &core_b, &mut zero_budget), 2);

    // 4. Unrelated slices: returns 0 matches without false positives.
    let unrelated_a = vec!["apple", "banana", "cherry"];
    let unrelated_b = vec!["dog", "elephant", "fox"];
    assert_eq!(super::lcs_len(&unrelated_a, &unrelated_b, &mut zero_budget), 0);

    // 5. Normal budget debits only core DP cells, not whole sequence length.
    let mut ample_budget = 100u64;
    // Prefix 1 ("p"), suffix 1 ("s"), core 1x1 ("a" vs "b"). Cells = 1.
    let budgeted_a = vec!["p", "a", "s"];
    let budgeted_b = vec!["p", "b", "s"];
    assert_eq!(super::lcs_len(&budgeted_a, &budgeted_b, &mut ample_budget), 2);
    assert_eq!(ample_budget, 99);
}

/// #5842: When a document has many footnotes (30+) and both reference paragraphs and note
/// contents are edited, note similarity scoring falls back to bounded greedy alignment after
/// `MAX_NOTE_ALIGN_BUDGET` is exhausted, pairing all 30 corresponding notes rather than
/// treating them as whole deletions and insertions.
#[test]
fn compare_pairs_many_notes_when_budget_exhausted() {
    let note_count = 30usize;
    let mut orig_text = String::new();
    let mut rev_text = String::new();

    for i in 0..note_count {
        if i > 0 {
            orig_text.push('\n');
            rev_text.push('\n');
        }
        orig_text.push_str(&format!("Body paragraph {i} discussing original thesis topic."));
        rev_text.push_str(&format!("Body paragraph {i} discussing revised thesis subject."));
    }

    let mut orig = Session::new(Document::from_text(&orig_text));
    let mut rev = Session::new(Document::from_text(&rev_text));

    let padding = "extra reference background context terminology narrative citations archive analysis documentation ";

    for i in 0..note_count {
        let orig_para_len = orig.doc.body[i].as_para().unwrap().text.len();
        orig.run("caret.set", &json!({"pos": Pos::body(i, orig_para_len)})).unwrap();
        orig.run(
            "references.footnote",
            &json!({"text": format!("Footnote entry {i} citing detailed historical observations and background data. {padding}{padding}{padding}{padding}")}),
        )
        .unwrap();

        let rev_para_len = rev.doc.body[i].as_para().unwrap().text.len();
        rev.run("caret.set", &json!({"pos": Pos::body(i, rev_para_len)})).unwrap();
        rev.run(
            "references.footnote",
            &json!({"text": format!("Footnote entry {i} citing detailed historical commentary and background data. {padding}{padding}{padding}{padding}")}),
        )
        .unwrap();
    }

    let rev_path = std::env::temp_dir().join(format!("wc-rev-manyfn-{}.docx", std::process::id()));
    crate::io::save_path(&rev_path, &rev.doc).unwrap();

    let mut s = orig;
    let r = s.run("review.compare", &json!({"path": rev_path.to_string_lossy()})).unwrap();
    let _ = std::fs::remove_file(&rev_path);

    assert!(r["changes"].as_u64().unwrap() >= note_count as u64);

    // Ensure all 30 footnotes survived as paired footnotes in the result document.
    let footnotes: Vec<_> = s.doc.parts.values().filter(|p| p.kind == wordcraft_doc::PartKind::Footnote).collect();
    assert_eq!(footnotes.len(), note_count, "All 30 footnotes must be paired rather than deleted and re-inserted");

    // Verify every note paired with its counterpart and tracked the internal word substitution.
    for part in &footnotes {
        let para = part.blocks[0].as_para().unwrap();
        let has_ins = para.run_ranges().any(|(r, c)| c.ins.is_some() && para.text[r].contains("commentary"));
        let has_del = para.run_ranges().any(|(r, c)| c.del.is_some() && para.text[r].contains("observations"));
        assert!(has_ins, "Note tracks insertion of 'commentary': {}", para.plain_text());
        assert!(has_del, "Note tracks deletion of 'observations': {}", para.plain_text());
    }
}

/// #5842: Past `MAX_NOTE_ALIGN_RUN` an edited run of notes must still pair by identity, not by
/// position. The revised document deletes note 0 and edits every remaining note, so no note text
/// anchors a pairing and the run is too large for the similarity matrix. Every remaining note
/// must keep its own original counterpart: its body reference stays clean, its part diffs its own
/// text, and rejecting one note's revision restores that note alone.
#[test]
fn compare_pairs_notes_above_alignment_run_boundary() {
    let note_count = super::compare::MAX_NOTE_ALIGN_RUN + 1;
    let mut orig_text = String::new();
    let mut rev_text = String::new();
    for i in 0..note_count {
        if i > 0 {
            orig_text.push('\n');
            rev_text.push('\n');
        }
        orig_text.push_str(&format!("Paragraph {i} anchors a note."));
        rev_text.push_str(&format!("Paragraph {i} anchors a note."));
    }
    let mut orig = Session::new(Document::from_text(&orig_text));
    let mut rev = Session::new(Document::from_text(&rev_text));

    let mut orig_note_ids = Vec::with_capacity(note_count);
    for i in 0..note_count {
        let len = orig.doc.body[i].as_para().unwrap().text.len();
        orig.run("caret.set", &json!({"pos": Pos::body(i, len)})).unwrap();
        orig.run("references.footnote", &json!({"text": format!("note[{i}] original observation.")})).unwrap();
        let id = orig.doc.body[i]
            .as_para()
            .and_then(|p| p.objects.iter().find_map(|o| if let InlineObject::NoteRef { id, .. } = o { Some(*id) } else { None }))
            .unwrap();
        orig_note_ids.push(id);

        // The revised document keeps every paragraph but deletes note 0 and edits all others.
        if i > 0 {
            let len = rev.doc.body[i].as_para().unwrap().text.len();
            rev.run("caret.set", &json!({"pos": Pos::body(i, len)})).unwrap();
            rev.run("references.footnote", &json!({"text": format!("note[{i}] revised commentary.")})).unwrap();
        }
    }

    let rev_path = std::env::temp_dir().join(format!("wc-rev-manyfn-boundary-{}.docx", std::process::id()));
    crate::io::save_path(&rev_path, &rev.doc).unwrap();

    let mut s = orig;
    let r = s.run("review.compare", &json!({"path": rev_path.to_string_lossy()})).unwrap();
    let _ = std::fs::remove_file(&rev_path);
    assert!(r["changes"].as_u64().unwrap() >= note_count as u64);

    // Note 0's reference is deleted and its story is fully deleted.
    let p0 = s.doc.body[0].as_para().unwrap();
    assert!(p0.runs.iter().any(|run| run.props.del.is_some()), "paragraph 0 note reference is deleted");
    let gone = s.doc.parts.get(&orig_note_ids[0]).unwrap();
    assert!(gone.blocks[0].as_para().unwrap().runs.iter().all(|run| run.props.del.is_some()), "note 0 story is fully marked deleted");

    // Every remaining paragraph keeps its own reference, and every remaining note diffs its own text.
    for (i, &note_id) in orig_note_ids.iter().enumerate().skip(1) {
        let para = s.doc.body.get(i).and_then(|b| b.as_para()).unwrap();
        assert!(para.runs.iter().all(|run| run.props.del.is_none() && run.props.ins.is_none()), "paragraph {i} note reference is unchanged");
        let part = s.doc.parts.get(&note_id).unwrap();
        let text = part.blocks[0].as_para().unwrap().plain_text();
        assert!(text.contains(&format!("note[{i}] original observation")), "note {i} keeps its own original text: {text}");
        assert!(text.contains("revised commentary"), "note {i} tracks its revised text: {text}");
        for other in [i.saturating_sub(1), i.saturating_add(1)] {
            if other != i && other < note_count {
                assert!(!text.contains(&format!("note[{other}]")), "note {i} must not carry note {other}'s text: {text}");
            }
        }
    }

    // Rejecting one note's revision restores that note and leaves its neighbours revised.
    let victim = 150usize;
    let victim_id = orig_note_ids[victim];
    let end = s.doc.parts.get(&victim_id).and_then(|p| p.blocks.first()).and_then(|b| b.as_para()).map(Paragraph::len).unwrap();
    let story = StoryRef::Part(victim_id);
    s.run(
        "select.range",
        &json!({
            "anchor": Pos::new(story, wordcraft_doc::Path::top(0), 0),
            "focus": Pos::new(story, wordcraft_doc::Path::top(0), end),
        }),
    )
    .unwrap();
    s.run("review.reject", &json!({})).unwrap();

    assert_eq!(
        s.doc.plain_text(StoryRef::Part(victim_id)).trim(),
        format!("note[{victim}] original observation."),
        "rejecting note {victim}'s revision restores its original text"
    );
    let restored = s.doc.parts.get(&victim_id).unwrap();
    assert!(
        restored.blocks[0].as_para().unwrap().runs.iter().all(|run| run.props.ins.is_none() && run.props.del.is_none()),
        "note {victim} has no tracked revisions left"
    );
    let neighbour = s.doc.parts.get(&orig_note_ids[victim + 1]).unwrap();
    let neighbour_para = neighbour.blocks[0].as_para().unwrap();
    assert!(
        neighbour_para.runs.iter().any(|run| run.props.ins.is_some()),
        "note {} keeps its own tracked revision: {}",
        victim + 1,
        neighbour_para.plain_text()
    );
    assert!(neighbour_para.plain_text().contains(&format!("note[{}]", victim + 1)));
}

/// #5842: When a paired footnote's text is edited, the note's run formatting is preserved:
/// the footnote reference mark retains `FootnoteReference`, unchanged text retains its own
/// font/style properties rather than inheriting the first run's props, and inserted words
/// take the revised run props.
#[test]
fn compare_preserves_note_run_formatting() {
    let mut orig = Session::new(Document::from_text("Body sentence."));
    orig.run("caret.set", &json!({"pos": Pos::body(0, "Body sentence.".len())})).unwrap();
    orig.run("references.footnote", &json!({"text": "Original note content."})).unwrap();

    // Style a specific word in the original footnote as bold.
    let fn_id = orig.doc.parts.iter().find(|(_, p)| p.kind == wordcraft_doc::PartKind::Footnote).map(|(&id, _)| id).unwrap();
    if let Some(fn_part) = orig.doc.parts.get_mut(&fn_id)
        && let Some(Block::Para(p)) = fn_part.blocks.get_mut(0).map(Arc::make_mut)
        && let Some(idx) = p.text.find("Original")
    {
        let _ = p.format(idx, idx + "Original".len(), &|c| c.bold = Some(true));
    }

    let mut rev = Session::new(Document::from_text("Body sentence."));
    rev.run("caret.set", &json!({"pos": Pos::body(0, "Body sentence.".len())})).unwrap();
    rev.run("references.footnote", &json!({"text": "Original note updated."})).unwrap();
    let rev_fn_id = rev.doc.parts.iter().find(|(_, p)| p.kind == wordcraft_doc::PartKind::Footnote).map(|(&id, _)| id).unwrap();
    if let Some(fn_part) = rev.doc.parts.get_mut(&rev_fn_id)
        && let Some(Block::Para(p)) = fn_part.blocks.get_mut(0).map(Arc::make_mut)
    {
        if let Some(idx) = p.text.find("Original") {
            let _ = p.format(idx, idx + "Original".len(), &|c| c.bold = Some(true));
        }
        if let Some(idx) = p.text.find("updated") {
            let _ = p.format(idx, idx + "updated".len(), &|c| c.italic = Some(true));
        }
    }

    let rev_path = std::env::temp_dir().join(format!("wc-rev-fnprops-{}.docx", std::process::id()));
    crate::io::save_path(&rev_path, &rev.doc).unwrap();

    let mut s = orig;
    let r = s.run("review.compare", &json!({"path": rev_path.to_string_lossy()})).unwrap();
    let _ = std::fs::remove_file(&rev_path);

    assert!(r["changes"].as_u64().unwrap() >= 1);

    let fn_part = s.doc.parts.get(&fn_id).unwrap();
    let p = fn_part.blocks[0].as_para().unwrap();

    // Verify the footnote reference run style is still FootnoteReference.
    let ref_run = p.runs.first().unwrap();
    assert_eq!(ref_run.props.style.as_deref(), Some("FootnoteReference"));

    // Verify "Original" retained bold: true and is NOT FootnoteReference.
    let orig_idx = p.text.find("Original").unwrap();
    let orig_props = p.props_of_char(orig_idx);
    assert_eq!(orig_props.bold, Some(true), "Original retained bold");
    assert_ne!(orig_props.style.as_deref(), Some("FootnoteReference"), "Text does not inherit FootnoteReference");

    // Verify "updated" is marked ins and has italic: true from revised run.
    let upd_idx = p.text.find("updated").unwrap();
    let upd_props = p.props_of_char(upd_idx);
    assert!(upd_props.ins.is_some(), "'updated' is tracked as ins");
    assert_eq!(upd_props.italic, Some(true), "'updated' has italic: true from revised run");
}

/// #5842: When `footnotes` is false, note references in the revised body are still aligned and
/// remapped to their matching original notes, while revised-only notes are adopted without diffing
/// so every reference resolves to a valid note part for layout and DOCX save.
#[test]
fn compare_remaps_revised_note_refs_when_footnotes_false() {
    let mut orig = Session::new(Document::from_text("Para one.\nPara two."));
    orig.run("caret.set", &json!({"pos": Pos::body(0, "Para one.".len())})).unwrap();
    orig.run("references.footnote", &json!({"text": "First footnote content."})).unwrap();
    orig.run("caret.set", &json!({"pos": Pos::body(1, "Para two.".len())})).unwrap();
    orig.run("references.footnote", &json!({"text": "Second footnote content."})).unwrap();

    let fn_ids: Vec<u32> = orig.doc.parts.iter().filter(|(_, p)| p.kind == wordcraft_doc::PartKind::Footnote).map(|(&id, _)| id).collect();
    assert_eq!(fn_ids.len(), 2);
    let orig_fn1_id = fn_ids[0];
    let orig_fn2_id = fn_ids[1];

    let mut rev = Session::new(Document::from_text("Para two.\nPara three."));
    rev.run("caret.set", &json!({"pos": Pos::body(0, "Para two.".len())})).unwrap();
    rev.run("references.footnote", &json!({"text": "Second footnote content."})).unwrap();
    rev.run("caret.set", &json!({"pos": Pos::body(1, "Para three.".len())})).unwrap();
    rev.run("references.footnote", &json!({"text": "Third footnote content."})).unwrap();

    let rev_path = std::env::temp_dir().join(format!("wc-rev-fnremapdis-{}.docx", std::process::id()));
    let out_path = std::env::temp_dir().join(format!("wc-out-fnremapdis-{}.docx", std::process::id()));
    crate::io::save_path(&rev_path, &rev.doc).unwrap();

    let mut s = orig;
    let _ = s.run("review.compare", &json!({"path": rev_path.to_string_lossy(), "footnotes": false})).unwrap();
    let _ = std::fs::remove_file(&rev_path);

    // The surviving paragraph's footnote reference must resolve to orig_fn2_id, not note 1.
    let p2 = s.doc.body.iter().filter_map(|b| b.as_para()).find(|p| p.plain_text().contains("Para two.")).unwrap();
    let p2_refs: Vec<u32> = p2.objects.iter().filter_map(|o| if let InlineObject::NoteRef { id, .. } = o { Some(*id) } else { None }).collect();
    assert_eq!(p2_refs, vec![orig_fn2_id], "Surviving note reference was remapped to original note 2");

    // The revised-only note reference must resolve to an adopted footnote part without diff marks.
    let p3 = s.doc.body.iter().filter_map(|b| b.as_para()).find(|p| p.plain_text().contains("Para three.")).unwrap();
    let fn3_id = p3.objects.iter().find_map(|o| if let InlineObject::NoteRef { id, .. } = o { Some(*id) } else { None }).unwrap();
    assert_ne!(fn3_id, orig_fn1_id);
    assert_ne!(fn3_id, orig_fn2_id);
    let fn3_part = s.doc.parts.get(&fn3_id).expect("revised-only footnote was adopted into s.doc.parts");
    assert_eq!(fn3_part.kind, wordcraft_doc::PartKind::Footnote);
    let fn3_para = fn3_part.blocks[0].as_para().unwrap();
    assert!(fn3_para.plain_text().contains("Third footnote content."));
    assert!(
        fn3_para.runs.iter().all(|r| r.props.ins.is_none() && r.props.del.is_none()),
        "adopted note has no tracked ins/del marks when footnotes=false"
    );

    // DOCX roundtrip preserves both surviving and adopted notes and their reference targets.
    crate::io::save_path(&out_path, &s.doc).unwrap();
    let loaded = crate::io::open_path(&out_path).unwrap();
    let _ = std::fs::remove_file(&out_path);
    let loaded_note_text = |para_prefix: &str| -> String {
        let p = loaded.body.iter().filter_map(|b| b.as_para()).find(|p| p.plain_text().contains(para_prefix)).unwrap();
        let ref_id = p.objects.iter().find_map(|o| if let InlineObject::NoteRef { id, .. } = o { Some(*id) } else { None }).unwrap();
        let part = loaded.parts.get(&ref_id).expect("reloaded footnote reference resolves to a footnote part");
        assert_eq!(part.kind, wordcraft_doc::PartKind::Footnote);
        part.blocks.iter().filter_map(|b| b.as_para()).map(Paragraph::plain_text).collect::<Vec<_>>().join("\n")
    };
    assert!(loaded_note_text("Para two.").contains("Second footnote content."), "saved DOCX keeps Para two. -> second footnote");
    assert!(loaded_note_text("Para three.").contains("Third footnote content."), "saved DOCX keeps Para three. -> third footnote");
}

/// #5842: Paired notes adopt revised text box stories before comparing blocks, so text box stories
/// exist in `s.doc.parts` under independent IDs and comparison/output use the adopted slot IDs.
#[test]
fn compare_paired_note_adopts_text_boxes_before_comparing() {
    let orig = session_with_note("Body paragraph.", "Footnote content.", false);
    let orig_fn_id = orig.doc.parts.iter().find(|(_, p)| p.kind == wordcraft_doc::PartKind::Footnote).map(|(&id, _)| id).unwrap();

    let mut rev = session_with_note("Body paragraph.", "Footnote content with text box.", false);
    let rev_fn_id = rev.doc.parts.iter().find(|(_, p)| p.kind == wordcraft_doc::PartKind::Footnote).map(|(&id, _)| id).unwrap();

    let tb_story_id =
        rev.doc.add_part(wordcraft_doc::PartKind::TextBox, vec![para_block(Paragraph::with_text("Box inside paired note.", CharProps::default()))]);
    let shape_obj = InlineObject::Shape {
        kind: wordcraft_doc::para::ShapeKind::TextBox,
        w: 50.0,
        h: 50.0,
        fill: None,
        stroke: None,
        stroke_width: 1.0,
        float: Default::default(),
        story: Some(tb_story_id),
        freeform: None,
        effects: Default::default(),
        extra: Default::default(),
    };
    if let Some(fn_part) = rev.doc.parts.get_mut(&rev_fn_id)
        && let Some(Block::Para(p)) = fn_part.blocks.get_mut(0).map(Arc::make_mut)
    {
        let at = p.len();
        let _ = p.insert_object(at, shape_obj, &CharProps::default());
    }

    let rev_path = std::env::temp_dir().join(format!("wc-rev-paired-tb-{}.docx", std::process::id()));
    crate::io::save_path(&rev_path, &rev.doc).unwrap();

    let mut s = orig;
    let r = s.run("review.compare", &json!({"path": rev_path.to_string_lossy()})).unwrap();
    let _ = std::fs::remove_file(&rev_path);

    assert!(r["changes"].as_u64().unwrap() >= 1);

    // Verify text box part from the paired note was adopted into s.doc:
    let adopted_tb = s.doc.parts.values().find(|p| p.kind == wordcraft_doc::PartKind::TextBox);
    assert!(adopted_tb.is_some(), "Text box from paired note was adopted into s.doc");
    let adopted_text = adopted_tb.unwrap().blocks[0].as_para().unwrap().plain_text();
    assert_eq!(adopted_text, "Box inside paired note.");

    // Verify footnote blocks reference the adopted ID:
    let adopted_id = s.doc.parts.iter().find(|(_, p)| p.kind == wordcraft_doc::PartKind::TextBox).map(|(&id, _)| id).unwrap();
    let fn_part = s.doc.parts.get(&orig_fn_id).expect("original footnote part");
    let mut found_slot = false;
    for b in &fn_part.blocks {
        if let Block::Para(p) = &**b {
            for o in &p.objects {
                if let InlineObject::Shape { story: Some(id), .. } = o
                    && *id == adopted_id
                {
                    found_slot = true;
                }
            }
        }
    }
    assert!(found_slot, "Paired footnote references adopted text box ID {adopted_id}");
}

/// #5842: Notes whose text repeats must pair by reference location: deleting the first duplicate
/// leaves the surviving note with its own original identity, not the first one's.
#[test]
fn compare_pairs_duplicate_note_text_by_reference() {
    let mut orig = Session::new(Document::from_text("Para one.\nPara two."));
    orig.run("caret.set", &json!({"pos": Pos::body(0, "Para one.".len())})).unwrap();
    orig.run("references.footnote", &json!({"text": "Identical note text."})).unwrap();
    orig.run("caret.set", &json!({"pos": Pos::body(1, "Para two.".len())})).unwrap();
    orig.run("references.footnote", &json!({"text": "Identical note text."})).unwrap();
    let note_ids: Vec<u32> = orig
        .doc
        .body
        .iter()
        .filter_map(|b| b.as_para())
        .flat_map(|p| p.objects.iter())
        .filter_map(|o| if let InlineObject::NoteRef { id, .. } = o { Some(*id) } else { None })
        .collect();
    assert_eq!(note_ids.len(), 2);

    // The revised document keeps only the second paragraph and its note, word for word.
    let mut rev = Session::new(Document::from_text("Para two."));
    rev.run("caret.set", &json!({"pos": Pos::body(0, "Para two.".len())})).unwrap();
    rev.run("references.footnote", &json!({"text": "Identical note text."})).unwrap();

    let rev_path = std::env::temp_dir().join(format!("wc-rev-dupnotes-{}.docx", std::process::id()));
    crate::io::save_path(&rev_path, &rev.doc).unwrap();

    let mut s = orig;
    let _ = s.run("review.compare", &json!({"path": rev_path.to_string_lossy()})).unwrap();
    let _ = std::fs::remove_file(&rev_path);

    let ref_ids: Vec<u32> = s
        .doc
        .body
        .iter()
        .filter_map(|b| b.as_para())
        .flat_map(|p| p.objects.iter())
        .filter_map(|o| if let InlineObject::NoteRef { id, .. } = o { Some(*id) } else { None })
        .collect();
    assert!(ref_ids.contains(&note_ids[1]), "the surviving reference resolves to the second note: {ref_ids:?}");
    assert_eq!(ref_ids.iter().filter(|&&id| id == note_ids[0]).count(), 1, "the first note keeps only its deleted reference");
    let kept = s.doc.parts.get(&note_ids[1]).unwrap();
    assert!(
        kept.blocks[0].as_para().unwrap().runs.iter().all(|run| run.props.ins.is_none() && run.props.del.is_none()),
        "the surviving note is unchanged"
    );
    let gone = s.doc.parts.get(&note_ids[0]).unwrap();
    assert!(gone.blocks[0].as_para().unwrap().runs.iter().all(|run| run.props.del.is_some()), "the first duplicate note is fully deleted");
}

/// #5842: Prepaired duplicate-text note anchors must bound the remaining note alignment so text
/// LCS cannot match a note across an anchor (A–E five-paragraph regression).
#[test]
fn compare_keeps_prepaired_note_anchors_as_segment_boundaries() {
    let body = "Paragraph A.\nParagraph B.\nParagraph C.\nParagraph D.\nParagraph E.";
    let mut orig = Session::new(Document::from_text(body));
    let mut rev = Session::new(Document::from_text(body));

    let orig_notes = ["Shared anchor note.", "old beta", "Shared anchor note.", "Shared anchor note.", "old epsilon"];
    let rev_notes = ["Shared anchor note.", "new beta", "Shared anchor note.", "Shared anchor note.", "old beta"];
    let mut orig_ids = Vec::with_capacity(orig_notes.len());

    for (i, (&o_text, &r_text)) in orig_notes.iter().zip(&rev_notes).enumerate() {
        let o_len = orig.doc.body[i].as_para().unwrap().text.len();
        orig.run("caret.set", &json!({"pos": Pos::body(i, o_len)})).unwrap();
        orig.run("references.footnote", &json!({"text": o_text})).unwrap();
        let o_id = orig.doc.body[i]
            .as_para()
            .and_then(|p| p.objects.iter().find_map(|o| if let InlineObject::NoteRef { id, .. } = o { Some(*id) } else { None }))
            .unwrap();
        orig_ids.push(o_id);

        let r_len = rev.doc.body[i].as_para().unwrap().text.len();
        rev.run("caret.set", &json!({"pos": Pos::body(i, r_len)})).unwrap();
        rev.run("references.footnote", &json!({"text": r_text})).unwrap();
    }

    let rev_path = std::env::temp_dir().join(format!("wc-rev-anchorseg-{}.docx", std::process::id()));
    crate::io::save_path(&rev_path, &rev.doc).unwrap();

    let mut s = orig;
    let _ = s.run("review.compare", &json!({"path": rev_path.to_string_lossy()})).unwrap();
    let _ = std::fs::remove_file(&rev_path);

    let footnotes: Vec<_> = s.doc.parts.values().filter(|p| p.kind == wordcraft_doc::PartKind::Footnote).collect();
    assert_eq!(footnotes.len(), 5, "all five footnotes stay paired within their segments");

    for (i, &expected_id) in orig_ids.iter().enumerate() {
        let para = s.doc.body[i].as_para().unwrap();
        assert!(para.runs.iter().all(|r| r.props.del.is_none() && r.props.ins.is_none()), "paragraph {i} keeps its note reference cleanly");
        let ref_ids: Vec<u32> = para.objects.iter().filter_map(|o| if let InlineObject::NoteRef { id, .. } = o { Some(*id) } else { None }).collect();
        assert_eq!(ref_ids, vec![expected_id], "paragraph {i} keeps its own note id");
    }

    let note_b = s.doc.parts.get(&orig_ids[1]).unwrap().blocks[0].as_para().unwrap();
    assert!(note_b.run_ranges().any(|(r, c)| c.del.is_some() && note_b.text[r].contains("old")), "note B deletes 'old': {}", note_b.plain_text());
    assert!(note_b.run_ranges().any(|(r, c)| c.ins.is_some() && note_b.text[r].contains("new")), "note B inserts 'new': {}", note_b.plain_text());

    let note_e = s.doc.parts.get(&orig_ids[4]).unwrap().blocks[0].as_para().unwrap();
    assert!(
        note_e.run_ranges().any(|(r, c)| c.del.is_some() && note_e.text[r].contains("epsilon")),
        "note E deletes 'epsilon': {}",
        note_e.plain_text()
    );
    assert!(note_e.run_ranges().any(|(r, c)| c.ins.is_some() && note_e.text[r].contains("beta")), "note E inserts 'beta': {}", note_e.plain_text());
}

/// #5842: Duplicate-text notes referenced in the same paragraph must use inline reference context
/// so deleting the first reference preserves the surviving second note's identity (both when the
/// surrounding text stays and when the first clause is also deleted).
#[test]
fn compare_pairs_duplicate_note_text_in_same_paragraph_by_inline_context() {
    for (label, rev_body, rev_caret) in [
        ("ref-only-deleted", "First claim. Second claim.", "First claim. Second claim".len()),
        ("clause-and-ref-deleted", "Second claim.", "Second claim".len()),
    ] {
        let mut orig = Session::new(Document::from_text("First claim. Second claim."));
        orig.run("caret.set", &json!({"pos": Pos::body(0, "First claim".len())})).unwrap();
        orig.run("references.footnote", &json!({"text": "Identical note text."})).unwrap();
        let second_pos = "First claim".len() + wordcraft_doc::para::OBJ.len_utf8() + ". Second claim".len();
        orig.run("caret.set", &json!({"pos": Pos::body(0, second_pos)})).unwrap();
        orig.run("references.footnote", &json!({"text": "Identical note text."})).unwrap();

        let note_ids: Vec<u32> = orig.doc.body[0]
            .as_para()
            .unwrap()
            .objects
            .iter()
            .filter_map(|o| if let InlineObject::NoteRef { id, .. } = o { Some(*id) } else { None })
            .collect();
        assert_eq!(note_ids.len(), 2, "{label}: expected 2 original notes");

        // Make the first original note bold so a wrong pairing also leaks a formatting revision.
        if let Some(part) = orig.doc.parts.get_mut(&note_ids[0])
            && let Some(Block::Para(p)) = part.blocks.first_mut().map(Arc::make_mut)
        {
            let len = p.len();
            let _ = p.format(0, len, &|c| c.bold = Some(true));
        }

        let mut rev = Session::new(Document::from_text(rev_body));
        rev.run("caret.set", &json!({"pos": Pos::body(0, rev_caret)})).unwrap();
        rev.run("references.footnote", &json!({"text": "Identical note text."})).unwrap();

        let rev_path = std::env::temp_dir().join(format!("wc-rev-samepara-{label}-{}.docx", std::process::id()));
        crate::io::save_path(&rev_path, &rev.doc).unwrap();

        let mut s = orig;
        let _ = s.run("review.compare", &json!({"path": rev_path.to_string_lossy()})).unwrap();
        let _ = std::fs::remove_file(&rev_path);

        let p = s.doc.body[0].as_para().unwrap();
        let surviving_refs: Vec<u32> =
            p.objects.iter().filter_map(|o| if let InlineObject::NoteRef { id, .. } = o { Some(*id) } else { None }).collect();
        assert_eq!(surviving_refs, vec![note_ids[0], note_ids[1]], "{label}: both original note refs stay in order");

        let ref_statuses: Vec<(u32, bool, bool)> = p
            .run_ranges()
            .filter_map(|(r, c)| {
                let slice = &p.text[r.clone()];
                if !slice.contains(wordcraft_doc::para::OBJ) {
                    return None;
                }
                let mut obj_k = 0usize;
                for (off, ch) in p.text.char_indices() {
                    if ch == wordcraft_doc::para::OBJ {
                        if r.contains(&off)
                            && let Some(InlineObject::NoteRef { id, .. }) = p.objects.get(obj_k)
                        {
                            return Some((*id, c.del.is_some(), c.ins.is_some()));
                        }
                        obj_k += 1;
                    }
                }
                None
            })
            .collect();
        assert_eq!(
            ref_statuses,
            vec![(note_ids[0], true, false), (note_ids[1], false, false)],
            "{label}: first note ref is deleted and second note ref stays clean"
        );

        let kept = s.doc.parts.get(&note_ids[1]).unwrap().blocks[0].as_para().unwrap();
        assert!(
            kept.runs.iter().all(|run| run.props.ins.is_none() && run.props.del.is_none() && run.props.fmt_change.is_none()),
            "{label}: surviving second note is unchanged"
        );
        let gone = s.doc.parts.get(&note_ids[0]).unwrap().blocks[0].as_para().unwrap();
        assert!(gone.runs.iter().all(|run| run.props.del.is_some()), "{label}: deleted first note is marked deleted");
    }
}

/// #5842: Moving a footnote reference within a paragraph whose plain text did not change must
/// still route through tracked diffing so Reject All restores the original reference offset.
#[test]
fn compare_tracks_moved_note_reference_in_unchanged_paragraph() {
    let ref_prefix = |doc: &Document| -> String {
        let p = doc.body[0].as_para().unwrap();
        let mut out = String::new();
        let mut obj_k = 0usize;
        for c in p.text.chars() {
            if c == wordcraft_doc::para::OBJ {
                if let Some(InlineObject::NoteRef { .. }) = p.objects.get(obj_k) {
                    return out;
                }
                obj_k += 1;
            } else {
                out.push(c);
            }
        }
        panic!("missing NoteRef in paragraph: {:?}", p.text);
    };

    let mut orig = Session::new(Document::from_text("Alpha. Beta."));
    orig.run("caret.set", &json!({"pos": Pos::body(0, "Alpha".len())})).unwrap();
    orig.run("references.footnote", &json!({"text": "Shared note."})).unwrap();

    let mut rev = Session::new(Document::from_text("Alpha. Beta."));
    rev.run("caret.set", &json!({"pos": Pos::body(0, "Alpha. Beta".len())})).unwrap();
    rev.run("references.footnote", &json!({"text": "Shared note."})).unwrap();

    let rev_path = std::env::temp_dir().join(format!("wc-rev-moveref-{}.docx", std::process::id()));
    crate::io::save_path(&rev_path, &rev.doc).unwrap();

    let mut s = orig;
    let r = s.run("review.compare", &json!({"path": rev_path.to_string_lossy()})).unwrap();
    assert!(r["changes"].as_u64().unwrap() >= 1, "moved note ref counts as a change: {r}");
    let p = s.doc.body[0].as_para().unwrap();
    assert!(p.runs.iter().any(|run| run.props.del.is_some()), "original reference position is tracked as deleted");
    assert!(p.runs.iter().any(|run| run.props.ins.is_some()), "revised reference position is tracked as inserted");

    s.run("review.rejectAll", &json!({})).unwrap();
    assert_eq!(ref_prefix(&s.doc), "Alpha", "Reject All restores original note reference position");
    assert_eq!(s.doc.plain_text(StoryRef::Body), "Alpha. Beta.");

    let mut orig2 = Session::new(Document::from_text("Alpha. Beta."));
    orig2.run("caret.set", &json!({"pos": Pos::body(0, "Alpha".len())})).unwrap();
    orig2.run("references.footnote", &json!({"text": "Shared note."})).unwrap();
    orig2.run("review.compare", &json!({"path": rev_path.to_string_lossy()})).unwrap();
    let _ = std::fs::remove_file(&rev_path);
    orig2.run("review.acceptAll", &json!({})).unwrap();
    assert_eq!(ref_prefix(&orig2.doc), "Alpha. Beta", "Accept All keeps revised note reference position");
    assert_eq!(orig2.doc.plain_text(StoryRef::Body), "Alpha. Beta.");
}

/// #5842: Revised-only note references nested at `MAX_TABLE_DEPTH` must still be remapped when
/// their source ID collides with an original note ID.
#[test]
fn compare_remaps_note_reference_at_max_table_depth() {
    fn find_deep_para(blocks: &[Arc<Block>], depth: usize, target_depth: usize) -> Option<&Paragraph> {
        for b in blocks {
            match &**b {
                Block::Para(p) if depth == target_depth => return Some(p),
                Block::Para(_) => {}
                Block::Table(t) => {
                    for c in t.rows.iter().flat_map(|r| r.cells.iter()) {
                        if let Some(p) = find_deep_para(&c.blocks, depth + 1, target_depth) {
                            return Some(p);
                        }
                    }
                }
            }
        }
        None
    }

    let mut orig = Session::new(Document::from_text("Orig."));
    orig.run("caret.set", &json!({"pos": Pos::body(0, "Orig.".len())})).unwrap();
    orig.run("references.footnote", &json!({"text": "Alpha footnote."})).unwrap();
    let orig_fn_id = orig.doc.parts.iter().find(|(_, p)| p.kind == wordcraft_doc::PartKind::Footnote).map(|(&id, _)| id).unwrap();

    let mut rev = Session::new(Document::from_text("Orig."));
    rev.run("caret.set", &json!({"pos": Pos::body(0, "Orig.".len())})).unwrap();
    for _ in 0..wordcraft_doc::table::MAX_TABLE_DEPTH {
        rev.run("insert.table", &json!({"rows": 1, "cols": 1})).unwrap();
    }
    assert_eq!(rev.sel.focus.path.depth(), wordcraft_doc::table::MAX_TABLE_DEPTH);
    rev.run("text.insert", &json!({"text": "Deep cell."})).unwrap();
    rev.run("references.footnote", &json!({"text": "Gamma entry inside deepest cell."})).unwrap();

    let rev_path = std::env::temp_dir().join(format!("wc-rev-deeptbl-{}.docx", std::process::id()));
    crate::io::save_path(&rev_path, &rev.doc).unwrap();

    let mut s = orig;
    let _ = s.run("review.compare", &json!({"path": rev_path.to_string_lossy()})).unwrap();
    let _ = std::fs::remove_file(&rev_path);

    let deep_para = find_deep_para(&s.doc.body, 0, wordcraft_doc::table::MAX_TABLE_DEPTH).expect("deepest cell paragraph exists");
    assert!(deep_para.plain_text().contains("Deep cell."));
    let remapped_id = deep_para.objects.iter().find_map(|o| if let InlineObject::NoteRef { id, .. } = o { Some(*id) } else { None }).unwrap();
    assert_ne!(remapped_id, orig_fn_id, "deepest table cell note reference must be remapped off original note ID");
    let adopted_part = s.doc.parts.get(&remapped_id).expect("remapped note ID resolves to adopted note part");
    assert!(adopted_part.blocks[0].as_para().unwrap().plain_text().contains("Gamma entry inside deepest cell."));
    assert!(s.doc.parts.get(&orig_fn_id).unwrap().blocks[0].as_para().unwrap().plain_text().contains("Alpha footnote."));
}

/// #5842: Multiple edited notes in the same unchanged paragraph use inline reference context to
/// break ties rather than pairing with a later added note.
#[test]
fn compare_breaks_same_paragraph_note_tie_by_inline_context() {
    let mut orig = Session::new(Document::from_text("Alpha. Beta."));
    orig.run("caret.set", &json!({"pos": Pos::body(0, "Alpha".len())})).unwrap();
    orig.run("references.footnote", &json!({"text": "Mercury note."})).unwrap();
    let orig_fn_id = orig.doc.parts.iter().find(|(_, p)| p.kind == wordcraft_doc::PartKind::Footnote).map(|(&id, _)| id).unwrap();

    let mut rev = Session::new(Document::from_text("Alpha. Beta."));
    rev.run("caret.set", &json!({"pos": Pos::body(0, "Alpha".len())})).unwrap();
    rev.run("references.footnote", &json!({"text": "Venus note."})).unwrap();
    let second_pos = "Alpha".len() + wordcraft_doc::para::OBJ.len_utf8() + ". Beta".len();
    rev.run("caret.set", &json!({"pos": Pos::body(0, second_pos)})).unwrap();
    rev.run("references.footnote", &json!({"text": "Saturn note."})).unwrap();

    let rev_path = std::env::temp_dir().join(format!("wc-rev-sametie-{}.docx", std::process::id()));
    crate::io::save_path(&rev_path, &rev.doc).unwrap();

    let mut s = orig;
    let _ = s.run("review.compare", &json!({"path": rev_path.to_string_lossy()})).unwrap();
    let _ = std::fs::remove_file(&rev_path);

    let paired_text = s.doc.parts.get(&orig_fn_id).unwrap().blocks[0].as_para().unwrap().plain_text();
    assert!(paired_text.contains("Venus"), "original note pairs with Venus at Alpha: {paired_text}");
    assert!(!paired_text.contains("Saturn"), "original note must not pair with Saturn at Beta: {paired_text}");

    let footnotes: Vec<_> = s.doc.parts.iter().filter(|(_, p)| p.kind == wordcraft_doc::PartKind::Footnote).collect();
    assert_eq!(footnotes.len(), 2);
    let inserted = footnotes.into_iter().find(|&(&id, _)| id != orig_fn_id).unwrap().1;
    let inserted_para = inserted.blocks[0].as_para().unwrap();
    assert!(inserted_para.plain_text().contains("Saturn note."));
    assert!(inserted_para.runs.iter().all(|r| r.props.ins.is_some()));
}

/// #5842: Past `MAX_NOTE_ALIGN_RUN`, unique unchanged reference paragraphs anchor edited notes even
/// when an inserted leading paragraph shifts every block index.
#[test]
fn compare_anchors_unique_note_paragraphs_past_run_limit_when_shifted() {
    let note_count = super::compare::MAX_NOTE_ALIGN_RUN + 1;
    let mut orig_text = String::new();
    let mut rev_text = String::from("Inserted intro paragraph.");
    for i in 0..note_count {
        if i > 0 {
            orig_text.push('\n');
        }
        orig_text.push_str(&format!("Unique anchor {i}."));
        rev_text.push_str(&format!("\nUnique anchor {i}."));
    }
    let mut orig = Session::new(Document::from_text(&orig_text));
    let mut rev = Session::new(Document::from_text(&rev_text));

    let mut orig_note_ids = Vec::with_capacity(note_count);
    for i in 0..note_count {
        let o_len = orig.doc.body[i].as_para().unwrap().text.len();
        orig.run("caret.set", &json!({"pos": Pos::body(i, o_len)})).unwrap();
        orig.run("references.footnote", &json!({"text": format!("note[{i}] original text.")})).unwrap();
        let id = orig.doc.body[i]
            .as_para()
            .and_then(|p| p.objects.iter().find_map(|o| if let InlineObject::NoteRef { id, .. } = o { Some(*id) } else { None }))
            .unwrap();
        orig_note_ids.push(id);

        let r_len = rev.doc.body[i + 1].as_para().unwrap().text.len();
        rev.run("caret.set", &json!({"pos": Pos::body(i + 1, r_len)})).unwrap();
        rev.run("references.footnote", &json!({"text": format!("note[{i}] revised text.")})).unwrap();
    }

    let rev_path = std::env::temp_dir().join(format!("wc-rev-shiftedanchor-{}.docx", std::process::id()));
    crate::io::save_path(&rev_path, &rev.doc).unwrap();

    let mut s = orig;
    let _ = s.run("review.compare", &json!({"path": rev_path.to_string_lossy()})).unwrap();
    let _ = std::fs::remove_file(&rev_path);

    let footnotes: Vec<_> = s.doc.parts.values().filter(|p| p.kind == wordcraft_doc::PartKind::Footnote).collect();
    assert_eq!(footnotes.len(), note_count, "all {note_count} shifted notes remain paired");
    for (i, &note_id) in orig_note_ids.iter().enumerate() {
        let para = s.doc.parts.get(&note_id).unwrap().blocks[0].as_para().unwrap();
        assert!(para.run_ranges().any(|(r, c)| c.del.is_some() && para.text[r].contains("original")), "note {i} tracks del");
        assert!(para.run_ranges().any(|(r, c)| c.ins.is_some() && para.text[r].contains("revised")), "note {i} tracks ins");
    }
}

/// #5842: Source revision indices on revised note blocks are stripped before adoption or comparison
/// so foreign revision IDs never alias Compare's own revision table.
#[test]
fn compare_strips_source_revisions_from_adopted_notes() {
    let mut orig = Session::new(Document::from_text("Para one.\nPara two."));
    orig.run("caret.set", &json!({"pos": Pos::body(0, "Para one.".len())})).unwrap();
    orig.run("references.footnote", &json!({"text": "Unchanged note."})).unwrap();
    let orig_fn_id = orig.doc.parts.iter().find(|(_, p)| p.kind == wordcraft_doc::PartKind::Footnote).map(|(&id, _)| id).unwrap();

    let mut rev = Session::new(Document::from_text("Para one.\nPara two."));
    rev.run("caret.set", &json!({"pos": Pos::body(0, "Para one.".len())})).unwrap();
    rev.run("references.footnote", &json!({"text": "Unchanged note."})).unwrap();
    rev.run("caret.set", &json!({"pos": Pos::body(1, "Para two.".len())})).unwrap();
    rev.run("references.footnote", &json!({"text": "Inserted note."})).unwrap();

    rev.doc.revisions.push(wordcraft_doc::Revision {
        kind: RevisionKind::Delete,
        author: "Other".into(),
        date: "2026-01-01T00:00:00Z".into(),
        move_name: None,
    });
    for part in rev.doc.parts.values_mut() {
        if part.kind == wordcraft_doc::PartKind::Footnote
            && let Some(Block::Para(p)) = part.blocks.first_mut().map(Arc::make_mut)
            && let Some(r) = p.runs.last_mut()
        {
            r.props.del = Some(0);
        }
    }

    let rev_path = std::env::temp_dir().join(format!("wc-rev-srcrevs-{}.docx", std::process::id()));
    crate::io::save_path(&rev_path, &rev.doc).unwrap();

    let mut s = orig;
    let _ = s.run("review.compare", &json!({"path": rev_path.to_string_lossy()})).unwrap();
    let _ = std::fs::remove_file(&rev_path);

    let paired_para = s.doc.parts.get(&orig_fn_id).unwrap().blocks[0].as_para().unwrap();
    assert!(
        paired_para.runs.iter().all(|r| r.props.del.is_none() && r.props.ins.is_none() && r.props.fmt_change.is_none()),
        "paired unchanged note does not inherit revised source revision indices"
    );

    let inserted_part = s.doc.parts.iter().find(|&(&id, p)| id != orig_fn_id && p.kind == wordcraft_doc::PartKind::Footnote).unwrap().1;
    let inserted_para = inserted_part.blocks[0].as_para().unwrap();
    assert!(inserted_para.runs.iter().all(|r| r.props.del.is_none()), "adopted inserted note has no leaked del index");
    assert!(inserted_para.runs.iter().all(|r| r.props.ins.is_some()), "adopted inserted note carries Compare's ins mark");
}

/// #5842: Adopting or comparing revised note blocks copies and remaps media referenced directly
/// and inside nested text-box stories, preserving distinct images under colliding source keys
/// across DOCX save and reopen.
#[test]
fn compare_adopts_and_remaps_note_media_across_docx_roundtrip() {
    let solid_png = |rgba: [u8; 4]| {
        let img = image::RgbaImage::from_pixel(4, 4, image::Rgba(rgba));
        wordcraft_geom::picture::encode_format(&image::DynamicImage::ImageRgba8(img), image::ImageFormat::Png).unwrap()
    };
    let red_png = solid_png([220, 20, 20, 255]);
    let blue_png = solid_png([20, 40, 220, 255]);
    let green_png = solid_png([20, 180, 40, 255]);

    let image_obj = |key: &str| InlineObject::Image {
        media: key.into(),
        w: 24.0,
        h: 24.0,
        alt: String::new(),
        float: Default::default(),
        crop: [0.0; 4],
        fx: Default::default(),
        ole: None,
    };
    let tb_obj = |story_id: u32| InlineObject::Shape {
        kind: wordcraft_doc::para::ShapeKind::TextBox,
        w: 50.0,
        h: 50.0,
        fill: None,
        stroke: None,
        stroke_width: 1.0,
        float: Default::default(),
        story: Some(story_id),
        freeform: None,
        effects: Default::default(),
        extra: Default::default(),
    };
    let append_obj = |doc: &mut Document, part_id: u32, obj: InlineObject| {
        if let Some(part) = doc.parts.get_mut(&part_id)
            && let Some(Block::Para(p)) = part.blocks.first_mut().map(Arc::make_mut)
        {
            let at = p.len();
            let _ = p.insert_object(at, obj, &CharProps::default());
        }
    };

    // Original: body + paired footnote 1 with red image under key "image1.png".
    let mut orig = Session::new(Document::from_text("Para one.\nPara two."));
    orig.run("caret.set", &json!({"pos": Pos::body(0, "Para one.".len())})).unwrap();
    orig.run("references.footnote", &json!({"text": "Shared note "})).unwrap();
    let orig_fn1_id = orig.doc.parts.iter().find(|(_, p)| p.kind == wordcraft_doc::PartKind::Footnote).map(|(&id, _)| id).unwrap();
    orig.doc.media.insert("image1.png".into(), Arc::new(red_png.clone()));
    append_obj(&mut orig.doc, orig_fn1_id, image_obj("image1.png"));

    // Revised: paired footnote 1 has blue image under the SAME key "image1.png";
    // inserted footnote 2 has a direct image ("image1.png" = blue) and a nested text box
    // ("image2.png" = green, absent from original).
    let mut rev = Session::new(Document::from_text("Para one.\nPara two."));
    rev.run("caret.set", &json!({"pos": Pos::body(0, "Para one.".len())})).unwrap();
    rev.run("references.footnote", &json!({"text": "Shared note "})).unwrap();
    rev.run("caret.set", &json!({"pos": Pos::body(1, "Para two.".len())})).unwrap();
    rev.run("references.footnote", &json!({"text": "Adopted note "})).unwrap();

    let rev_fn_ids: Vec<u32> = rev.doc.parts.iter().filter(|(_, p)| p.kind == wordcraft_doc::PartKind::Footnote).map(|(&id, _)| id).collect();
    assert_eq!(rev_fn_ids.len(), 2);
    let rev_fn1_id = rev_fn_ids[0];
    let rev_fn2_id = rev_fn_ids[1];

    rev.doc.media.insert("image1.png".into(), Arc::new(blue_png.clone()));
    rev.doc.media.insert("image2.png".into(), Arc::new(green_png.clone()));
    append_obj(&mut rev.doc, rev_fn1_id, image_obj("image1.png"));

    let nested_tb_id = rev.doc.add_part(wordcraft_doc::PartKind::TextBox, vec![para_block(Paragraph::with_text("Nested ", CharProps::default()))]);
    append_obj(&mut rev.doc, nested_tb_id, image_obj("image2.png"));
    let outer_tb_id = rev.doc.add_part(wordcraft_doc::PartKind::TextBox, vec![para_block(Paragraph::with_text("Outer ", CharProps::default()))]);
    append_obj(&mut rev.doc, outer_tb_id, tb_obj(nested_tb_id));
    append_obj(&mut rev.doc, rev_fn2_id, image_obj("image1.png"));
    append_obj(&mut rev.doc, rev_fn2_id, tb_obj(outer_tb_id));

    let pid = std::process::id();
    let rev_path = std::env::temp_dir().join(format!("wc-rev-note-media-{pid}.docx"));
    let out_path = std::env::temp_dir().join(format!("wc-out-note-media-{pid}.docx"));
    crate::io::save_path(&rev_path, &rev.doc).unwrap();
    let reopened_rev = crate::io::open_path(&rev_path).unwrap();
    assert!(reopened_rev.media.contains_key("image1.png"), "reopened revised DOCX uses colliding source key image1.png");

    let mut s = orig;
    let r = s.run("review.compare", &json!({"path": rev_path.to_string_lossy()})).unwrap();
    let _ = std::fs::remove_file(&rev_path);
    assert!(r["changes"].as_u64().unwrap() >= 1);

    // Save comparison result to DOCX and reopen to verify media keys and bytes survive roundtrip.
    crate::io::save_path(&out_path, &s.doc).unwrap();
    let loaded = crate::io::open_path(&out_path).unwrap();
    let _ = std::fs::remove_file(&out_path);

    let mut loaded_fns: Vec<&wordcraft_doc::Part> = loaded.parts.values().filter(|p| p.kind == wordcraft_doc::PartKind::Footnote).collect();
    assert_eq!(loaded_fns.len(), 2);
    loaded_fns.sort_by_key(|p| p.blocks[0].as_para().map(Paragraph::plain_text).unwrap_or_default());
    let (adopted_fn, paired_fn) = if loaded_fns[0].blocks[0].as_para().unwrap().plain_text().contains("Adopted note") {
        (loaded_fns[0], loaded_fns[1])
    } else {
        (loaded_fns[1], loaded_fns[0])
    };

    // Paired note keeps original red image (deleted) and remapped blue image (inserted).
    let paired_imgs: Vec<&str> = paired_fn.blocks[0]
        .as_para()
        .unwrap()
        .objects
        .iter()
        .filter_map(|o| if let InlineObject::Image { media, .. } = o { Some(media.as_str()) } else { None })
        .collect();
    assert_eq!(paired_imgs.len(), 2, "paired note retains original and revised images: {paired_imgs:?}");
    let paired_bytes: Vec<&[u8]> = paired_imgs.iter().map(|k| loaded.media.get(*k).expect("paired image bytes present").as_slice()).collect();
    assert!(paired_bytes.contains(&red_png.as_slice()), "paired note kept original red image");
    assert!(paired_bytes.contains(&blue_png.as_slice()), "paired note kept revised blue image");

    // Adopted note keeps its blue image and its nested text box's green image.
    let adopted_para = adopted_fn.blocks[0].as_para().unwrap();
    let adopted_img_key =
        adopted_para.objects.iter().find_map(|o| if let InlineObject::Image { media, .. } = o { Some(media.as_str()) } else { None }).unwrap();
    assert_eq!(loaded.media.get(adopted_img_key).expect("adopted note image").as_slice(), blue_png.as_slice());

    let loaded_outer_tb_id = adopted_para.objects.iter().find_map(InlineObject::text_box).expect("adopted outer text box");
    let loaded_nested_tb_id = loaded.parts[&loaded_outer_tb_id].blocks[0]
        .as_para()
        .unwrap()
        .objects
        .iter()
        .find_map(InlineObject::text_box)
        .expect("adopted nested text box");
    let nested_img_key = loaded.parts[&loaded_nested_tb_id].blocks[0]
        .as_para()
        .unwrap()
        .objects
        .iter()
        .find_map(|o| if let InlineObject::Image { media, .. } = o { Some(media.as_str()) } else { None })
        .unwrap();
    assert_eq!(loaded.media.get(nested_img_key).expect("nested text box image").as_slice(), green_png.as_slice());
}

/// #3518: every run and paragraph mark of a story, as (story, path, lang, noProof) rows.
fn proofing_rows(s: &Session) -> Vec<(StoryRef, Option<String>, Option<bool>)> {
    let stories = std::iter::once(StoryRef::Body).chain(s.doc.parts.keys().map(|k| StoryRef::Part(*k)));
    let mut out = Vec::new();
    for st in stories {
        for path in s.doc.para_paths(st) {
            let Some(p) = s.doc.para(st, &path) else { continue };
            out.push((st, p.mark.lang.clone(), p.mark.no_proof));
            out.extend(p.runs.iter().map(|r| (st, r.props.lang.clone(), r.props.no_proof)));
        }
    }
    out
}

/// A document with its text in German in the body, a table cell, a header, a footnote, a comment
/// and a text box, as a Word DOCX keeps a `w:lang` on each run, plus a glossary entry.
fn multi_story_doc() -> Session {
    use wordcraft_doc::{Block, BuildingBlock, CharProps, Paragraph, PartKind, Table};
    let de = || CharProps { lang: Some("de-DE".into()), ..Default::default() };
    let blk = |t: &str| Arc::new(Block::Para(Paragraph::with_text(t, de())));
    let mut d = Document::from_text("Guten Tag");
    d.body = vec![blk("Guten Tag")];
    let mut t = Table::new(1, 2, 200.0);
    if let Some(c) = t.rows.first_mut().and_then(|r| r.cells.first_mut()) {
        c.blocks = vec![blk("Zelle")];
    }
    d.body.push(Arc::new(Block::Table(t)));
    d.body.push(blk("Ende"));
    for kind in [PartKind::Header, PartKind::Footnote, PartKind::Comment, PartKind::TextBox] {
        d.add_part(kind, vec![blk("Teil")]);
    }
    d.glossary.push(BuildingBlock { name: "g".into(), blocks: vec![blk("Baustein")], ..Default::default() });
    Session::new(d)
}

#[test]
fn language_scope_document_sets_every_story() {
    let mut s = multi_story_doc();
    s.run("select.text", &json!({"text": "Guten"})).unwrap();
    let (sel, before) = (s.sel.clone(), s.doc.clone());
    let rows = proofing_rows(&s);
    assert!(rows.len() >= 10 && rows.iter().any(|r| r.1.as_deref() == Some("de-DE")));
    s.run("review.language", &json!({"lang": "fr-FR", "scope": "document"})).unwrap();
    // Runs that had their own language, the paragraph marks and the empty cell: all French.
    let rows = proofing_rows(&s);
    assert!(rows.iter().all(|r| r.1.as_deref() == Some("fr-FR")), "{rows:?}");
    for kind_story in s.doc.parts.keys() {
        assert!(rows.iter().any(|r| r.0 == StoryRef::Part(*kind_story)));
    }
    assert!(s.doc.plain_text(StoryRef::Body).contains("Zelle"));
    let g = &s.doc.glossary[0].blocks;
    assert_eq!(g[0].as_para().map(|p| p.runs[0].props.lang.clone()), Some(Some("fr-FR".into())));
    assert_eq!(g[0].as_para().map(|p| p.mark.lang.clone()), Some(Some("fr-FR".into())));
    // The selection stays; the document is modified.
    assert_eq!(s.sel, sel);
    assert!(s.doc != before);
    // One undo restores everything.
    s.run("edit.undo", &json!({})).unwrap();
    assert!(s.doc == before);
    assert_eq!(s.sel, sel);
}

#[test]
fn language_scope_document_no_proof_and_pending() {
    let mut s = multi_story_doc();
    s.run("review.language", &json!({"noProof": true, "scope": "document"})).unwrap();
    assert!(proofing_rows(&s).iter().all(|r| r.2 == Some(true)));
    // The language itself wasn't asked for and stays.
    assert!(proofing_rows(&s).iter().any(|r| r.1.as_deref() == Some("de-DE")));
    // Formatting waiting for typing follows.
    s.run("format.bold", &json!({})).unwrap();
    s.run("review.language", &json!({"lang": "pl-PL", "scope": "document"})).unwrap();
    assert_eq!(s.pending.as_ref().and_then(|p| p.lang.as_deref()), Some("pl-PL"));
}

/// Document scope returns the caret/selection like the selection path, plus the scope.
#[test]
fn language_scope_document_returns_the_selection_result() {
    let mut s = multi_story_doc();
    s.run("select.text", &json!({"text": "Guten"})).unwrap();
    let out = s.run("review.language", &json!({"lang": "fr-FR", "scope": "document"})).unwrap();
    assert_eq!(out["scope"], "document");
    assert_eq!(out["anchor"], crate::cmd::pos_json(&s.sel.anchor), "{out}");
    assert_eq!(out["focus"], crate::cmd::pos_json(&s.sel.focus), "{out}");
}

/// Tracked formatting records only the body and parts, so while it is on the glossary (outside
/// any story) is left alone instead of changing untracked; with tracking off it changes.
#[test]
fn language_scope_document_skips_glossary_while_tracking_formatting() {
    let gloss = |s: &Session| s.doc.glossary[0].blocks[0].as_para().map(|p| (p.runs[0].props.lang.clone(), p.mark.lang.clone()));
    let mut s = multi_story_doc();
    s.run("review.trackChanges", &json!({"value": true})).unwrap();
    s.run("review.trackingOptions", &json!({"trackFormatting": true})).unwrap();
    let out = s.run("review.language", &json!({"lang": "fr-FR", "scope": "document"})).unwrap();
    assert_eq!(out["glossarySkipped"], true, "{out}");
    assert_eq!(gloss(&s), Some((Some("de-DE".into()), Some("de-DE".into()))));
    assert!(proofing_rows(&s).iter().all(|r| r.1.as_deref() == Some("fr-FR")));
    assert!(s.doc.revisions.iter().any(|r| r.kind == wordcraft_doc::RevisionKind::Format), "the body change is tracked");
    let mut s = multi_story_doc();
    s.run("review.trackChanges", &json!({"value": true})).unwrap();
    s.run("review.trackingOptions", &json!({"trackFormatting": false})).unwrap();
    let out = s.run("review.language", &json!({"lang": "fr-FR", "scope": "document"})).unwrap();
    assert!(out.get("glossarySkipped").is_none(), "{out}");
    assert_eq!(gloss(&s), Some((Some("fr-FR".into()), Some("fr-FR".into()))));
}

#[test]
fn language_default_scope_is_selection_only() {
    let mut s = multi_story_doc();
    s.run("select.text", &json!({"text": "Guten"})).unwrap();
    s.run("review.language", &json!({"lang": "fr-FR"})).unwrap();
    let rows = proofing_rows(&s);
    assert!(rows.iter().any(|r| r.1.as_deref() == Some("fr-FR")));
    assert!(rows.iter().filter(|r| r.1.as_deref() == Some("fr-FR")).count() < rows.len() / 2);
    let mut s = multi_story_doc();
    s.run("select.text", &json!({"text": "Guten"})).unwrap();
    s.run("review.language", &json!({"lang": "fr-FR", "scope": "selection"})).unwrap();
    assert_eq!(proofing_rows(&s), rows);
}

#[test]
fn language_bad_scope_is_a_params_error() {
    let mut s = multi_story_doc();
    let before = s.doc.clone();
    for bad in [json!("everything"), json!(1), json!(true), json!(["document"])] {
        let e = s.run("review.language", &json!({"lang": "fr-FR", "detect": false, "scope": bad})).unwrap_err();
        assert!(matches!(e, CmdError::Params(_)), "{e:?}");
    }
    assert!(s.doc == before);
    assert!(s.prefs.detect_language);
}

/// Document scope is refused under editing protection, before anything changes.
#[test]
fn language_document_scope_is_refused_under_protection() {
    for mode in ["readOnly", "comments", "forms"] {
        let mut s = multi_story_doc();
        s.doc.settings.protection = Some(mode.into());
        let before = s.doc.clone();
        let e = s.run("review.language", &json!({"lang": "fr-FR", "detect": false, "scope": "document"})).unwrap_err();
        assert!(matches!(e, CmdError::Disabled(_)), "{mode}: {e:?}");
        assert!(s.doc == before, "{mode}: nothing changed");
        assert!(s.prefs.detect_language, "{mode}: the detect preference is untouched");
    }
}
