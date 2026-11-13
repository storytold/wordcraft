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
