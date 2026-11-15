use super::*;

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
