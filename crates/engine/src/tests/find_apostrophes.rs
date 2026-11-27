use super::*;

/// Keyboard smart quotes stay searchable, and replacement keeps their original spelling.
#[test]
fn keyboard_apostrophe_finds_smart_quoted_typing() {
    let mut s = s();
    for ch in "Tau'va".chars() {
        run(&mut s, "text.insert", json!({"text": ch.to_string()}));
    }
    assert_eq!(text(&s), "Tau’va");
    for query in ["Tau'", "Tau'v", "Tau'va", "tau'VA"] {
        assert_eq!(run(&mut s, "edit.find", json!({"text": query}))["count"], 1, "{query}");
    }
    assert_eq!(run(&mut s, "edit.find", json!({"text": "tau'va", "matchCase": true}))["count"], 0);
    assert_eq!(run(&mut s, "edit.replaceAll", json!({"text": "Tau'va", "with": "[^&]", "matchCase": false}))["replaced"], 1);
    assert_eq!(text(&s), "[Tau’va]");
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(text(&s), "Tau’va");
    run(&mut s, "edit.redo", json!({}));
    assert_eq!(text(&s), "[Tau’va]");
}

/// Normal search uses raw UTF-8 ranges while explicit syntax and modifier letters stay exact.
#[test]
fn normal_apostrophe_search_keeps_exact_modes_and_original_byte_ranges() {
    let mut s = s();
    let original = "🙂 Tau'va Tau’va Tau‘va Tauʼva";
    run(&mut s, "document.setText", json!({"text": original}));
    let found = run(&mut s, "edit.find", json!({"text": "Tau'va", "wholeWord": true}));
    assert_eq!(found["count"], 3);
    let ranges: Vec<_> =
        found["matches"].as_array().unwrap().iter().map(|m| (m["start"]["off"].as_u64().unwrap(), m["end"]["off"].as_u64().unwrap())).collect();
    assert_eq!(ranges, [(5, 11), (12, 20), (21, 29)]);
    for options in [json!({"special": false}), json!({"wildcards": true}), json!({"regex": true})] {
        let mut exact = super::s();
        run(&mut exact, "document.setText", json!({"text": original}));
        let mut params = options;
        params["text"] = json!("Tau'va");
        assert_eq!(run(&mut exact, "edit.find", params)["count"], 1);
    }
    for query in ["Tau^u39va", "Tau^u8217va", "Tau’va", "Tau‘va", "Tauʼva"] {
        assert_eq!(run(&mut s, "edit.find", json!({"text": query}))["count"], 1, "{query}");
    }
    assert_eq!(run(&mut s, "edit.replaceAll", json!({"text": "Tau'va", "with": "<^&>"}))["replaced"], 3);
    assert_eq!(text(&s), "🙂 <Tau'va> <Tau’va> <Tau‘va> Tauʼva");
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(text(&s), original);
}

/// A keyboard double quote finds the curly ones smart quotes type, as a keyboard apostrophe does.
#[test]
fn keyboard_double_quote_finds_smart_quoted_typing() {
    let mut s = s();
    for ch in "\"hi\"".chars() {
        run(&mut s, "text.insert", json!({"text": ch.to_string()}));
    }
    assert_eq!(text(&s), "“hi”");
    for (query, count) in [("\"", 2), ("\"hi", 1), ("hi\"", 1), ("\"hi\"", 1), ("\"HI\"", 1)] {
        assert_eq!(run(&mut s, "edit.find", json!({"text": query}))["count"], count, "{query}");
    }
    assert_eq!(run(&mut s, "edit.find", json!({"text": "\"HI\"", "matchCase": true}))["count"], 0);
    assert_eq!(run(&mut s, "edit.replaceAll", json!({"text": "\"hi\"", "with": "[^&]"}))["replaced"], 1);
    assert_eq!(text(&s), "[“hi”]");
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(text(&s), "“hi”");
}

/// Straight and curly double quotes: only a typed `"` in a normal search reads as all three.
#[test]
fn normal_double_quote_search_keeps_exact_modes_and_original_byte_ranges() {
    let mut s = s();
    let original = "🙂 \"hi\" “hi” ”hi“ 'hi' ″hi″";
    run(&mut s, "document.setText", json!({"text": original}));
    let found = run(&mut s, "edit.find", json!({"text": "\"hi\""}));
    assert_eq!(found["count"], 3);
    let ranges: Vec<_> =
        found["matches"].as_array().unwrap().iter().map(|m| (m["start"]["off"].as_u64().unwrap(), m["end"]["off"].as_u64().unwrap())).collect();
    assert_eq!(ranges, [(5, 9), (10, 18), (19, 27)]);
    assert_eq!(run(&mut s, "edit.find", json!({"text": "\""}))["count"], 6);
    for options in [json!({"special": false}), json!({"wildcards": true}), json!({"regex": true})] {
        let mut exact = super::s();
        run(&mut exact, "document.setText", json!({"text": original}));
        let mut params = options;
        params["text"] = json!("\"hi\"");
        assert_eq!(run(&mut exact, "edit.find", params)["count"], 1);
    }
    for query in ["^034hi^034", "^u8220hi^u8221", "“hi”", "”hi“"] {
        assert_eq!(run(&mut s, "edit.find", json!({"text": query}))["count"], 1, "{query}");
    }
    assert_eq!(run(&mut s, "edit.replaceAll", json!({"text": "\"hi\"", "with": "<^&>"}))["replaced"], 3);
    assert_eq!(text(&s), "🙂 <\"hi\"> <“hi”> <”hi“> 'hi' ″hi″");
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(text(&s), original);
}

/// #7307: with smart quotes on, a quote typed in Replace with goes in curly by the character
/// before the match, as typing curls it, so finding a straight quote and replacing it with a
/// typed one curls a manuscript's stray quotes in bulk.
#[test]
fn a_quote_typed_in_replace_with_is_curled_by_smart_quotes() {
    let mut s = s();
    run(&mut s, "document.setText", json!({"text": "He said \"hi\" (\"no\") and 'bye' -- don't"}));
    assert_eq!(run(&mut s, "edit.replaceAll", json!({"text": "^034", "with": "\""}))["replaced"], 4);
    assert_eq!(text(&s), "He said “hi” (“no”) and 'bye' -- don't");
    assert_eq!(run(&mut s, "edit.replaceAll", json!({"text": "^039", "with": "'"}))["replaced"], 3);
    assert_eq!(text(&s), "He said “hi” (“no”) and ‘bye’ -- don’t");
    // One undo step each.
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(text(&s), "He said “hi” (“no”) and 'bye' -- don't");

    // A match touching the one before it curls by that one's replacement: after an opening
    // double quote a single quote opens too.
    let mut s = super::s();
    run(&mut s, "document.setText", json!({"text": "xx"}));
    assert_eq!(run(&mut s, "edit.replaceAll", json!({"text": "x", "with": "'\""}))["replaced"], 2);
    assert_eq!(text(&s), "‘“‘“");
}

/// #7307: a typed `'` in Find what also finds curly apostrophes. A match that already is a curly
/// quote of that kind is left as it is and not counted, so an apostrophe set on purpose (’90s)
/// isn't turned into an opening quote.
#[test]
fn replace_with_a_typed_quote_leaves_curly_ones_and_does_not_count_them() {
    let mut s = s();
    run(&mut s, "document.setText", json!({"text": "the ’90s, it’s 'x' and “y”"}));
    assert_eq!(run(&mut s, "edit.replaceAll", json!({"text": "'", "with": "'"}))["replaced"], 2);
    assert_eq!(text(&s), "the ’90s, it’s ‘x’ and “y”");
    assert_eq!(run(&mut s, "edit.replaceAll", json!({"text": "\"", "with": "\""}))["replaced"], 0);
    assert_eq!(text(&s), "the ’90s, it’s ‘x’ and “y”");
    // A whole word with its apostrophe: already curled, left alone; straight, curled.
    run(&mut s, "document.setText", json!({"text": "don’t don't"}));
    assert_eq!(run(&mut s, "edit.replaceAll", json!({"text": "don't", "with": "don't"}))["replaced"], 1);
    assert_eq!(text(&s), "don’t don’t");

    // Replace (one at a time) skips such a match too, and moves on to the next.
    let mut s = super::s();
    run(&mut s, "document.setText", json!({"text": "it’s 'x"}));
    let first = run(&mut s, "edit.replace", json!({"text": "'", "with": "'"}));
    assert_eq!(first["replaced"], 0, "the first call selects the first match");
    let kept = run(&mut s, "edit.replace", json!({}));
    assert_eq!((kept["replaced"].clone(), text(&s)), (json!(0), "it’s 'x".to_string()), "the curly apostrophe stays");
    assert_eq!(run(&mut s, "edit.replace", json!({}))["replaced"], 1);
    assert_eq!(text(&s), "it’s ‘x");
}

/// #7307: a quote given by its code stays straight, so curly quotes can still be made straight on
/// purpose; with smart quotes off, or with a regular expression, a typed quote stays straight too.
#[test]
fn coded_quotes_and_smart_quotes_off_replace_with_straight_quotes() {
    let mut s = s();
    run(&mut s, "document.setText", json!({"text": "“hi” it’s"}));
    assert_eq!(run(&mut s, "edit.replaceAll", json!({"text": "\"", "with": "^034"}))["replaced"], 2);
    assert_eq!(run(&mut s, "edit.replaceAll", json!({"text": "'", "with": "^039"}))["replaced"], 1);
    assert_eq!(text(&s), "\"hi\" it's");

    let mut s = super::s();
    run(&mut s, "document.setText", json!({"text": "“hi”"}));
    assert_eq!(run(&mut s, "edit.replaceAll", json!({"text": "\"", "with": "\"", "regex": false, "wildcards": false}))["replaced"], 0);
    assert_eq!(run(&mut s, "edit.replaceAll", json!({"text": "[“”]", "with": "\"", "regex": true}))["replaced"], 2);
    assert_eq!(text(&s), "\"hi\"");

    let mut s = super::s();
    s.prefs.autocorrect.smart_quotes = false;
    run(&mut s, "document.setText", json!({"text": "“hi”"}));
    assert_eq!(run(&mut s, "edit.replaceAll", json!({"text": "\"", "with": "\"", "regex": false}))["replaced"], 2);
    assert_eq!(text(&s), "\"hi\"");
}
