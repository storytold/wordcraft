use super::*;

fn texts(v: &[Utterance]) -> Vec<&str> {
    v.iter().map(|u| u.text.as_str()).collect()
}

#[test]
fn powershell_is_launched_from_the_system_folder() {
    let tail = std::path::Path::new("System32").join("WindowsPowerShell").join("v1.0").join("powershell.exe");
    let p = windows_powershell(Some("D:\\Win".into()));
    assert!(p.starts_with("D:\\Win") && p.ends_with(&tail), "{}", p.display());
    // No SystemRoot, or an empty one: the default Windows folder.
    for root in [None, Some("".into())] {
        let p = windows_powershell(root);
        assert!(p.starts_with("C:\\Windows") && p.ends_with(&tail), "{}", p.display());
    }
}

#[test]
fn powershell_starts_without_a_console_window() {
    // The flags `speak` passes to `creation_flags` (std can't read them back from the
    // `Command`): Win32's CREATE_NO_WINDOW (0x0800_0000), or every sentence flashes a
    // console (#1122).
    let w = windows_speech(Some("D:\\Win".into()), 1.0, None);
    assert_eq!(w.creation_flags, 0x0800_0000);
    assert_eq!(w.program, windows_powershell(Some("D:\\Win".into())));
    assert_eq!(w.args[..3], ["-NoProfile", "-NonInteractive", "-Command"]);
    assert!(w.args[3].contains("$s.Rate = 0;"), "{}", w.args[3]);
    assert!(windows_speech(None, 3.0, None).args[3].contains("$s.Rate = 10;"));
}

#[test]
fn splits_sentences_but_not_abbreviations() {
    let t = "Smith et al. found 3.5 times more (see Fig. 2). J. Smith agreed! Did they? Yes… Pick one, e.g. this one.";
    let spans = split_sentences(t);
    let got: Vec<String> = spans.iter().map(|(a, b)| t.chars().skip(*a).take(b - a).collect()).collect();
    assert_eq!(got, ["Smith et al. found 3.5 times more (see Fig. 2).", "J. Smith agreed!", "Did they?", "Yes…", "Pick one, e.g. this one."]);
}

#[test]
fn long_sentences_are_split() {
    let t = "word ".repeat(400);
    let spans = split_sentences(&t);
    assert!(spans.len() >= 3);
    assert!(spans.iter().all(|(a, b)| b - a <= MAX_SENTENCE + 5));
}

/// A word with no space to split at, or a run of punctuation, is still cut into utterances of
/// at most `MAX_UTTERANCE` characters, covering all of it (#3680).
#[test]
fn sentences_without_spaces_are_split() {
    for t in ["x".repeat(10_000), "!".repeat(10_000), format!("Go {} now.", "a.".repeat(5_000))] {
        let spans = split_sentences(&t);
        assert!(spans.len() >= 8, "{} spans", spans.len());
        assert!(spans.iter().all(|(a, b)| a < b && b - a <= MAX_UTTERANCE));
        assert_eq!(spans.first().map(|s| s.0), Some(0));
        assert_eq!(spans.last().map(|s| s.1), Some(t.chars().count()));
        assert!(spans.windows(2).all(|w| w[0].1 <= w[1].0));
    }
}

#[test]
fn tidy_removes_citation_leftovers() {
    assert_eq!(tidy("as shown  ( ) ."), "as shown.");
    assert_eq!(tidy("as shown [] , and"), "as shown, and");
    assert_eq!(tidy(" a ( ( [ ] ) ) b ( x ) [ ) ] ; "), "a b ( x ) [ ) ];");
}

/// The tidy that ran replacements until nothing changed, kept to check the one-pass tidy gives
/// the same text.
fn tidy_by_replacing(s: &str) -> String {
    let mut t: String = s.split_whitespace().collect::<Vec<_>>().join(" ");
    loop {
        let before = t.len();
        for (from, to) in [("()", ""), ("[]", ""), ("( )", ""), ("[ ]", ""), (" .", "."), (" ,", ","), (" ;", ";"), (" :", ":"), ("  ", " ")] {
            t = t.replace(from, to);
        }
        if t.len() == before {
            break;
        }
    }
    t.trim().to_string()
}

#[test]
fn tidy_matches_replacing_until_nothing_changes() {
    let alphabet = ['(', ')', '[', ']', ' ', ' ', '\t', '\u{a0}', '.', ',', ';', ':', 'a', 'é'];
    let mut seed: u64 = 0x3680;
    for _ in 0..20_000 {
        seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
        let len = (seed >> 59) as usize + 1;
        let s: String = (0..len)
            .map(|i| {
                let r = seed.rotate_left(5 * i as u32 + 1) ^ (i as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15);
                alphabet[(r % alphabet.len() as u64) as usize]
            })
            .collect();
        assert_eq!(tidy(&s), tidy_by_replacing(&s), "{s:?}");
    }
}

/// Nested empty brackets are removed in one pass: 40,000 pairs took seconds when each pass
/// removed one level (#3680).
#[test]
fn tidy_is_linear_on_nested_brackets() {
    let n = 40_000;
    let started = std::time::Instant::now();
    assert_eq!(tidy(&format!("a {}{} b", "( [".repeat(n), "] )".repeat(n))), "a b");
    assert_eq!(tidy(&format!("{}x{}", "(".repeat(n), ")".repeat(n))).len(), 2 * n + 1);
    // Milliseconds here; `tidy_by_replacing` (the old tidy) takes about 5 minutes on this input.
    assert!(started.elapsed() < std::time::Duration::from_secs(10), "{:?}", started.elapsed());
}

#[test]
fn citation_codes() {
    assert!(is_citation_code(" ADDIN ZOTERO_ITEM CSL_CITATION {}"));
    assert!(is_citation_code("ADDIN ZOTERO_BIBL {\"uncited\":[]} CSL_BIBLIOGRAPHY"));
    assert!(is_citation_code("ADDIN EN.CITE <EndNote>"));
    assert!(is_citation_code("ADDIN CSL_CITATION {}"));
    assert!(is_citation_code("CITATION Smi20 \\l 1033"));
    assert!(is_citation_code("BIBLIOGRAPHY"));
    assert!(!is_citation_code("ADDIN ZOTERO_PREF_1"));
    assert!(!is_citation_code("PAGE"));
    assert!(!is_citation_code(""));
    assert!(!is_citation_code("ADD"));
}

#[test]
fn plain_document_sentences() {
    let doc = Document::from_text("First one. Second one.\nThird paragraph");
    let s = sentences(&doc, &Pos::body(0, 0), None, true);
    assert_eq!(texts(&s), ["First one.", "Second one.", "Third paragraph"]);
    assert_eq!(s[1].start, Pos::body(0, 11));
    assert_eq!(s[1].end, Pos::body(0, 22));
    // From the caret, and only up to the end of a selection.
    let s = sentences(&doc, &Pos::body(0, 11), Some(&Pos::body(1, 5)), true);
    assert_eq!(texts(&s), ["Second one.", "Third"]);
}

#[test]
fn player_moves_through_sentences() {
    let doc = Document::from_text("One. Two. Three.");
    let mut r = ReadAloud::default();
    r.backend = Backend::Hold;
    r.start(sentences(&doc, &Pos::body(0, 0), None, true));
    assert_eq!(r.state(), State::Playing);
    assert_eq!(r.current().map(|u| u.text), Some("One.".into()));
    r.skip(1);
    r.skip(5);
    assert_eq!(r.progress(), (2, 3));
    r.skip(-10);
    assert_eq!(r.progress(), (0, 3));
    r.play_pause();
    assert_eq!(r.state(), State::Paused);
    r.set_rate(99.0);
    assert_eq!(r.rate(), MAX_RATE);
    r.set_rate(f32::NAN);
    assert_eq!(r.rate(), 1.0);
    r.play_pause();
    assert_eq!(r.state(), State::Playing);
    r.stop();
    assert_eq!(r.state(), State::Stopped);
    assert!(!r.open);
    assert!(r.current().is_none());
}

#[test]
fn player_finishes_on_its_own() {
    let doc = Document::from_text("One. Two. Three.");
    let mut r = ReadAloud::default();
    r.backend = Backend::Instant;
    r.start(sentences(&doc, &Pos::body(0, 0), None, true));
    for _ in 0..200 {
        if r.state() == State::Stopped {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert_eq!(r.state(), State::Stopped);
    assert_eq!(r.progress(), (3, 3));
    assert!(r.open, "the player stays open after the last sentence");
    // Play again starts from the top.
    r.backend = Backend::Hold;
    r.play_pause();
    assert_eq!(r.progress().0, 0);
    assert_eq!(r.state(), State::Playing);
}

#[test]
fn empty_queue_does_not_play() {
    let mut r = ReadAloud::default();
    r.start(Vec::new());
    assert_eq!(r.state(), State::Stopped);
    r.skip(1);
    r.skip(-1);
    r.play_pause();
    assert_eq!(r.state(), State::Stopped);
}
#[test]
fn languages_are_detected_from_the_script() {
    for (text, tag) in [
        ("Привет как твои дела?", "ru"),
        ("Привіт, як справи? Їжак і ґанок", "uk"),
        ("Γειά σου κόσμε", "el"),
        ("مرحبا بالعالم", "ar"),
        ("שלום עולם", "he"),
        ("こんにちは、世界", "ja"),
        ("你好，世界", "zh"),
        ("안녕하세요 세계", "ko"),
        ("สวัสดีชาวโลก", "th"),
        ("Hello Привет мир как дела", "ru"),
        ("Hello world and Привет", "ru"),
    ] {
        assert_eq!(detect_language(text), Some(tag), "{text}");
    }
    // Latin text, nothing to read, or only punctuation: no guess.
    for text in ["Hello, world", "", "  ", "?!…", "123"] {
        assert_eq!(detect_language(text), None, "{text:?}");
    }
}

#[test]
fn an_explicit_language_wins_over_detection() {
    // The document says so (a Russian run, even of Latin letters).
    assert_eq!(speech_language("Privet", Some("ru-RU"), None).as_deref(), Some("ru-RU"));
    assert_eq!(speech_language("Привет", Some("uk-UA"), None).as_deref(), Some("uk-UA"));
    assert_eq!(speech_language("Hallo Welt", Some("de-DE"), None).as_deref(), Some("de-DE"));
    // The default English tag on Cyrillic text (Word writes it on every run): the script wins.
    assert_eq!(speech_language("Привет", Some("en-US"), None).as_deref(), Some("ru"));
    // Nothing to go on: the default voice.
    assert_eq!(speech_language("Hello", None, None), None);
    assert_eq!(speech_language("Привет", None, None).as_deref(), Some("ru"));
    // A hostile tag is never passed on.
    assert_eq!(speech_language("Hello", Some("en'; calc; '"), None), None);
}

#[test]
fn sentences_carry_the_run_language() {
    let mut doc = Document::from_text("Привет как твои дела?\nPrivet.");
    let s = sentences(&doc, &Pos::body(0, 0), None, true);
    assert_eq!(s.iter().map(|u| u.lang.as_deref()).collect::<Vec<_>>(), [Some("ru"), Some("en-US")]);
    if let wordcraft_doc::Block::Para(p) = Arc::make_mut(&mut doc.body[1]) {
        for r in &mut p.runs {
            r.props.lang = Some("ru-RU".into());
        }
    }
    let s = sentences(&doc, &Pos::body(0, 0), None, true);
    assert_eq!(s.iter().map(|u| u.lang.as_deref()).collect::<Vec<_>>(), [Some("ru"), Some("ru-RU")]);
}

#[test]
fn windows_script_reads_utf8_and_picks_the_voice() {
    let w = windows_speech(None, 1.0, Some("ru"));
    let script = &w.args[3];
    assert!(script.contains("UTF8"), "{script}");
    assert!(script.contains("GetInstalledVoices") && script.contains("SelectVoice"), "{script}");
    assert!(script.contains("'ru'"), "{script}");
    // The default voice when the language is unknown, and no voice code at all.
    let d = windows_speech(None, 1.0, None);
    assert!(d.args[3].contains("UTF8") && !d.args[3].contains("SelectVoice"), "{}", d.args[3]);
    // The sentence is never in the script: it arrives on stdin.
    assert!(!windows_speech(None, 1.0, Some("ru-RU")).args[3].contains("Привет"));
    // A tag that isn't a plain language tag is dropped, not put in the script.
    let h = windows_speech(None, 1.0, Some("ru'; Remove-Item x; '"));
    assert!(!h.args[3].contains("Remove-Item"), "{}", h.args[3]);
}

#[test]
fn espeak_gets_the_language() {
    assert_eq!(espeak_args("180", Some("ru-RU")), ["-s", "180", "-v", "ru", "--stdin"]);
    assert_eq!(espeak_args("180", Some("uk")), ["-s", "180", "-v", "uk", "--stdin"]);
    assert_eq!(espeak_args("180", None), ["-s", "180", "--stdin"]);
    assert_eq!(espeak_args("180", Some("--evil")), ["-s", "180", "--stdin"]);
}

#[test]
fn english_text_keeps_the_default_voice() {
    // Word writes `en-US` on almost every run: espeak's default voice is English already, so
    // no `-v` (as before #1843), and Word's language-less `x-none` is no voice either.
    assert_eq!(espeak_args("180", Some("en-US")), ["-s", "180", "--stdin"]);
    assert_eq!(espeak_args("180", Some("en-GB")), ["-s", "180", "--stdin"]);
    assert_eq!(espeak_args("180", Some("x-none")), ["-s", "180", "--stdin"]);
    // Windows: the user's default voice stays when it already speaks the language, any region.
    let script = &windows_speech(None, 1.0, Some("en-US")).args[3];
    let guard = script.find("if ($s.Voice.Culture.TwoLetterISOLanguageName -ne 'en')");
    let pick = script.find("GetInstalledVoices");
    assert!(guard.is_some() && pick.is_some() && guard < pick, "{script}");
}

#[test]
fn macos_keeps_the_system_voice_for_its_language() {
    // Current macOS lists its novelty voices first, in alphabetical order.
    let list = "Albert              en_US    # Hello! My name is Albert.\n\
                    Bad News            en_US    # The light you see at the end of the tunnel.\n\
                    Bahh                en_US    # Do not pull the wool over my eyes.\n\
                    Samantha            en_US    # Hello! My name is Samantha.\n\
                    Milena              ru_RU    # Здравствуйте!\n";
    // English text with an English system voice: no `-v`, so `say` keeps the user's voice.
    assert_eq!(mac_voice("en-US", list, "en"), None);
    assert_eq!(mac_voice("en", list, "en"), None);
    // English text on a Russian system: an English voice, never a novelty one.
    assert_eq!(mac_voice("en-US", list, "ru").as_deref(), Some("Samantha"));
    assert_eq!(mac_voice("en-US", "Albert en_US # Hi\nBahh en_US # Hi\n", "ru"), None);
    // The system voice's language: the chosen voice's, else the locale's, else English.
    assert_eq!(mac_default_language(Some("Milena"), list, Some("en_US")), "ru");
    assert_eq!(mac_default_language(Some("Gone"), list, Some("de_DE")), "de");
    assert_eq!(mac_default_language(None, list, Some("ru_RU.UTF-8")), "ru");
    assert_eq!(mac_default_language(None, list, Some("C")), "en");
    assert_eq!(mac_default_language(None, "", None), "en");
}

#[test]
fn a_voice_the_program_lacks_falls_back_to_the_default() {
    // espeak-ng exits with a failure, saying nothing, for a `-v` it doesn't know.
    assert!(retry_without_voice(true, Some(false)));
    // It spoke, it was given no voice, or it was stopped (or never exited on its own).
    assert!(!retry_without_voice(true, Some(true)));
    assert!(!retry_without_voice(false, Some(false)));
    assert!(!retry_without_voice(true, None));
}

#[test]
fn right_to_left_text_uses_the_complex_script_tag() {
    // Persian tagged `w:bidi="fa-IR"` on a `w:val="en-US"` run: Persian, not Arabic.
    assert_eq!(speech_language("سلام دنیا", Some("en-US"), Some("fa-IR")).as_deref(), Some("fa-IR"));
    assert_eq!(speech_language("שלום עולם", Some("en-US"), Some("he-IL")).as_deref(), Some("he-IL"));
    // The complex-script tag is for complex-script text only.
    assert_eq!(speech_language("Hello", Some("en-US"), Some("fa-IR")).as_deref(), Some("en-US"));
    assert_eq!(speech_language("سلام", Some("en-US"), Some("he-IL")).as_deref(), Some("ar"));
}

#[test]
fn mixed_sentences_follow_the_other_script() {
    // More English letters than Cyrillic ones: the English voice would spell the Russian out.
    assert_eq!(detect_language("Я открыл Microsoft Word and PowerPoint"), Some("ru"));
    // A single foreign word in an English sentence isn't a real share: the default voice.
    assert_eq!(detect_language("The word дом means house"), None);
    // Ties between scripts always go the same way, the first script in the list.
    assert_eq!(detect_language("αβ дг"), Some("ru"));
    assert_eq!(detect_language("дг αβ"), Some("ru"));
}

#[test]
fn han_letters_keep_a_japanese_or_korean_tag() {
    assert_eq!(speech_language("東京都。", Some("ja-JP"), None).as_deref(), Some("ja-JP"));
    assert_eq!(speech_language("東京都。", Some("ko-KR"), None).as_deref(), Some("ko-KR"));
    assert_eq!(speech_language("東京都。", Some("en-US"), None).as_deref(), Some("zh"));
}

#[test]
fn macos_voice_matches_the_language() {
    let list = "Alex                en_US    # Most people recognize me by my voice.\n\
                    Eddy (Russian (Russia)) ru_RU    # Привет!\n\
                    Milena              ru_RU    # Здравствуйте!\n\
                    Samantha            en_US    # Hello!\n\
                    Lesya               uk_UA    # Привіт!\n\
                    Daniel              en_GB    # Hello!\n";
    assert_eq!(mac_voice("ru", list, "en").as_deref(), Some("Eddy (Russian (Russia))"));
    assert_eq!(mac_voice("ru-RU", list, "en").as_deref(), Some("Eddy (Russian (Russia))"));
    assert_eq!(mac_voice("uk", list, "en").as_deref(), Some("Lesya"));
    assert_eq!(mac_voice("en-GB", list, "ru").as_deref(), Some("Daniel"));
    // No voice for the language, or nothing listed: the default voice.
    assert_eq!(mac_voice("el", list, "en"), None);
    assert_eq!(mac_voice("ru", "", "en"), None);
    assert_eq!(mac_voice("ru", "garbage line\n\n  \n", "en"), None);
}
