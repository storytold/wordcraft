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
fn every_reading_counts_as_a_new_one() {
    // The worker forgets what it learnt about the speech programs when a reading starts, even
    // when the same worker thread carries on (a pause, then Read Aloud again).
    let mut r = ReadAloud::default();
    r.backend = Backend::Hold;
    let readings = |r: &ReadAloud| lock(&r.shared.0).readings;
    let doc = Document::from_text("One. Two.");
    r.start(sentences(&doc, &Pos::body(0, 0), None, true));
    let first = readings(&r);
    r.play_pause();
    r.start(sentences(&doc, &Pos::body(0, 0), None, true));
    assert_eq!(readings(&r), first + 1);
    r.play_pause();
    r.play_pause();
    assert_eq!(readings(&r), first + 1, "pause and play are the same reading");
    r.stop();
}

#[cfg(all(unix, not(target_os = "macos")))]
#[test]
fn a_sentence_that_never_began_says_so_not_another_program() {
    // No begin event within the queue bound: no other program need be involved, so the
    // message names the sentence not starting, not another program stopping it.
    let halted = |o| match dispatcher_spoken(Ok(o)) {
        Spoken::Halted(m) => m,
        _ => String::new(),
    };
    assert_eq!(halted(ssip::Outcome::NotStarted), NOT_STARTED);
    assert!(!NOT_STARTED.contains("another program"));
    assert_eq!(halted(ssip::Outcome::Halted), HALTED_BY_OTHER);
}

#[test]
fn play_after_a_failed_sentence_plays_it_again() {
    // A sentence whose end never came stops reading with an error (as the worker does on a
    // timeout); Play starts it again and the message goes.
    let mut r = ReadAloud::default();
    r.backend = Backend::Hold;
    r.start(sentences(&Document::from_text("One. Two."), &Pos::body(0, 0), None, true));
    r.skip(1);
    {
        let mut s = lock(&r.shared.0);
        s.state = State::Stopped;
        s.error = Some("Read Aloud couldn't use Speech Dispatcher: the sentence didn't end".into());
    }
    r.play_pause();
    assert_eq!((r.state(), r.progress().0, r.error()), (State::Playing, 1, None));
    r.stop();
}

#[test]
fn play_after_a_halt_carries_on_and_clears_the_message() {
    // Another program stopped the sentence: reading paused on it with a message, and Play
    // goes on from the same sentence.
    let mut r = ReadAloud::default();
    r.backend = Backend::Hold;
    r.start(sentences(&Document::from_text("One. Two."), &Pos::body(0, 0), None, true));
    {
        let mut s = lock(&r.shared.0);
        s.state = State::Paused;
        s.error = Some("stopped by another program".into());
    }
    r.play_pause();
    assert_eq!(r.state(), State::Playing);
    assert_eq!(r.progress().0, 0);
    assert_eq!(r.error(), None);
    r.stop();
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
    assert_eq!(espeak_args("180", Some("hu-HU")), ["-s", "180", "-v", "hu", "--stdin"]);
    // A chosen voice goes the same way, never as an option of its own.
    assert_eq!(espeak_voice_args("180", Some("urj/hu")), ["-s", "180", "-v", "urj/hu", "--stdin"]);
    assert_eq!(espeak_voice_args("180", Some("--version")), ["-s", "180", "--stdin"]);
    assert_eq!(espeak_voice_args("180", Some("a b")), ["-s", "180", "--stdin"]);
}

#[test]
fn installed_voices_are_read_from_each_listing() {
    let espeak = " Pty Language       Age/Gender VoiceName          File                 Other Languages\n 5  hu              --/M      Hungarian          urj/hu               \n 2  en-gb           --/M      English_(Great_Britain) gmw/en               (en 2)\n 7  hu              --/F      hungarian-mbrola-1 mb/mb-hu1            \n\n";
    let v = espeak_voices(espeak, &|_| false);
    assert_eq!(v.len(), 2, "MBROLA voices need mbrola and their database: {v:?}");
    assert_eq!(v[0], Voice { id: "urj/hu".into(), name: "Hungarian".into(), lang: "hu".into(), kind: VoiceKind::Espeak });
    assert_eq!(v[1].name, "English (Great Britain)");
    assert_eq!(espeak_voices(espeak, &|db| db == "hu1").len(), 3);
    assert_eq!(espeak_voices(espeak, &|db| db == "de4").len(), 2, "only the voices whose database is there");
    let win = "Microsoft Szabolcs\thu-HU\r\nMicrosoft Zira Desktop\ten-US\r\n\tbroken\r\n";
    let v = windows_voices(win);
    assert_eq!(v.len(), 2);
    assert_eq!((v[0].id.as_str(), v[0].lang.as_str()), ("Microsoft Szabolcs", "hu-HU"));
    assert!(WINDOWS_VOICES.contains("GetInstalledVoices"));
    // Voice names arrive in UTF-8 without a byte order mark; a mark that comes anyway is dropped.
    assert!(WINDOWS_VOICES.starts_with("[Console]::OutputEncoding = New-Object System.Text.UTF8Encoding $false;"), "{WINDOWS_VOICES}");
    let v = windows_voices("\u{FEFF}Microsoft Szabolcs\thu-HU\r\n  Padded  \ten-US\r\n");
    assert_eq!(v.iter().map(|v| v.id.as_str()).collect::<Vec<_>>(), ["Microsoft Szabolcs", "Padded"]);
    assert!(espeak_voices("", &|_| true).is_empty() && windows_voices("").is_empty());
}

#[test]
fn a_good_voice_list_survives_an_empty_answer() {
    let hu = Voice { id: "urj/hu".into(), name: "Hungarian".into(), lang: "hu".into(), kind: VoiceKind::Espeak };
    let old: Arc<[Voice]> = vec![hu.clone()].into();
    // Speech Dispatcher exited after its idle timeout: the refresh found nothing.
    assert_eq!(&*keep_voices(Some(Arc::clone(&old)), Vec::new()), &[hu.clone()][..]);
    assert!(keep_voices(None, Vec::new()).is_empty());
    assert!(keep_voices(Some(Arc::new([])), Vec::new()).is_empty());
    let gb = Voice { id: "gmw/en".into(), name: "English".into(), lang: "en-gb".into(), kind: VoiceKind::Espeak };
    assert_eq!(&*keep_voices(Some(old), vec![gb.clone()]), &[gb][..], "a new list replaces it");
}

#[test]
fn the_voice_picker_may_start_the_server_once() {
    use std::time::Duration;
    let s = Duration::from_secs;
    // An empty list asked for a moment ago: not again on every repaint…
    assert!(!ask_voices_now(true, false, Some(s(1)), false));
    // …but at once when the picker opens and the server may be started.
    assert!(ask_voices_now(true, false, Some(s(1)), true));
    assert!(!ask_voices_now(true, true, Some(s(1)), true), "not while asking");
    assert!(!ask_voices_now(false, false, Some(s(1)), true), "a list is there");
    assert!(ask_voices_now(true, false, Some(VOICES_RETRY), false));
    assert!(ask_voices_now(false, false, Some(VOICES_REFRESH), false));
    assert!(ask_voices_now(true, false, None, false), "never asked");
    // A request that arrives while a query without it runs (the saved voice restored at
    // start-up, the picker opened during the first frame's query) isn't lost: an empty
    // answer is followed at once by a query that may start the server.
    assert_eq!(after_voice_query(true, false, true), (true, true));
    // A query that ran with it settles it, whatever it found; so does a list that came back,
    // so no later refresh starts a server.
    assert_eq!(after_voice_query(true, true, false), (false, false));
    assert_eq!(after_voice_query(false, false, true), (true, false));
    // No request: nothing more.
    assert_eq!(after_voice_query(true, false, false), (false, false));
    // A silent backend never asks the system, picker or not.
    let r = ReadAloud::default();
    r.ask_voices_starting_server();
    assert_eq!(r.voices().as_deref(), Some(&[][..]));
}

#[cfg(unix)]
#[test]
fn a_voice_listing_program_that_hangs_is_stopped() {
    assert_eq!(run_listing("sh".as_ref(), &["-c", "echo hu"]).as_deref(), Some("hu\n"));
    assert_eq!(run_listing("sh".as_ref(), &["-c", "exit 3"]), None, "a failure lists nothing");
    let start = std::time::Instant::now();
    assert_eq!(run_listing("sh".as_ref(), &["-c", "echo partial; sleep 30"]), None);
    assert!(start.elapsed() < LIST_TIMEOUT * 4, "{:?}", start.elapsed());
    assert_eq!(run_listing("no-such-program-for-wordcraft".as_ref(), &[]), None);
    // The program exits but a child keeps its output open for a while: the listing still
    // arrives, within the overall bound.
    let script = "echo hu; (sleep 0.5; echo late) & exit 0";
    assert_eq!(run_listing("sh".as_ref(), &["-c", script]).as_deref(), Some("hu\nlate\n"));
}

#[test]
fn an_mbrola_voice_is_listed_only_with_its_database() {
    // espeak-ng's own places: `mbrola/<db>`, `mbrola/<db>/<db>` and `mbrola/voices/<db>`.
    let root = std::env::temp_dir().join(format!("wordcraft-mbrola-{}", std::process::id()));
    let (a, b) = (root.join("a"), root.join("b"));
    std::fs::create_dir_all(a.join("mbrola").join("hu1")).unwrap();
    std::fs::create_dir_all(b.join("mbrola").join("voices")).unwrap();
    std::fs::create_dir_all(b.join("mbrola").join("en1")).unwrap();
    std::fs::write(a.join("mbrola").join("hu1").join("hu1"), b"").unwrap();
    std::fs::write(b.join("mbrola").join("voices").join("de4"), b"").unwrap();
    let dirs = std::env::join_paths([&a, &b]).unwrap();
    assert!(mbrola_database("hu1", Some(&dirs)));
    assert!(mbrola_database("de4", Some(&dirs)));
    assert!(!mbrola_database("en1", Some(&dirs)), "a folder without the database");
    assert!(!mbrola_database("fr1", Some(&dirs)));
    let listing = " Pty Language Age/Gender VoiceName File Other\n 7 hu --/F hungarian-mbrola-1 mb/mb-hu1\n 6 en --/M german-mbrola-4-en mb/mb-de4-en\n 5 en --/M english-mb-en1 mb/mb-en1\n 5 xx --/M odd mb/../../x\n";
    let ids: Vec<String> = espeak_voices(listing, &|db| mbrola_database(db, Some(&dirs))).into_iter().map(|v| v.id).collect();
    assert_eq!(ids, ["mb/mb-hu1", "mb/mb-de4-en"]);
    // espeak-ng 1.52's `--voices` has no MBROLA lines at all; `--voices=mb` lists them.
    let plain = " Pty Language Age/Gender VoiceName File Other\n 5 hu --/M Hungarian urj/hu\n";
    let mb = " Pty Language Age/Gender VoiceName File Other\n 7 hu --/F hungarian-mbrola-1 mb/mb-hu1\n 6 en --/M english-mb-en1 mb/mb-en1\n";
    let ids: Vec<String> = espeak_all_voices(plain, Some(mb), &|db| mbrola_database(db, Some(&dirs))).into_iter().map(|v| v.id).collect();
    assert_eq!(ids, ["urj/hu", "mb/mb-hu1"]);
    assert_eq!(espeak_all_voices(plain, None, &|_| true).len(), 1, "no mbrola program: no listing asked for");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_chosen_voice_reads_its_own_language() {
    let hu = Voice { id: "urj/hu".into(), name: "Hungarian".into(), lang: "hu".into(), kind: VoiceKind::Espeak };
    assert_eq!(voice_for(Some(&hu), Some("hu-HU"), "Jó reggelt"), Some(&hu));
    assert_eq!(voice_for(Some(&hu), None, "Jó reggelt"), Some(&hu), "no language: the chosen voice");
    assert_eq!(voice_for(Some(&hu), Some("ru-RU"), "Привет"), None, "a Russian quotation keeps the Russian voice");
    assert_eq!(voice_for(Some(&hu), Some("x-none"), "x"), Some(&hu));
    let gb = Voice { id: "gmw/en".into(), name: "English".into(), lang: "en-gb".into(), kind: VoiceKind::Espeak };
    assert_eq!(voice_for(Some(&gb), Some("en-US"), "Hello"), Some(&gb));
    assert_eq!(voice_for(None, Some("hu"), "Szia"), None);
}

#[test]
fn text_in_another_script_keeps_its_own_voice() {
    let hu = Voice { id: "urj/hu".into(), name: "Hungarian".into(), lang: "hu".into(), kind: VoiceKind::Espeak };
    // Text in another script than the voice's keeps its language's voice, either way round.
    let ru = Voice { id: "Milena".into(), name: "Milena".into(), lang: "ru_RU".into(), kind: VoiceKind::Mac };
    assert_eq!(voice_for(Some(&ru), Some("ru"), "Привет"), Some(&ru));
    assert_eq!(voice_for(Some(&ru), Some("uk-UA"), "Привіт"), Some(&ru), "the same script");
    assert_eq!(voice_for(Some(&ru), Some("en-US"), "Hello"), None, "English text keeps an English voice");
    let cmn = Voice { id: "sit/cmn".into(), name: "Chinese (Mandarin)".into(), lang: "cmn".into(), kind: VoiceKind::Espeak };
    assert_eq!(voice_for(Some(&cmn), Some("zh"), "你好"), Some(&cmn));
    assert_eq!(voice_for(Some(&cmn), Some("en-US"), "Hello"), None);
    // Scripts the detection doesn't know aren't taken for Latin, either way round.
    assert_eq!(voice_for(Some(&hu), Some("hi"), "नमस्ते दुनिया"), None, "Hindi keeps the Hindi voice");
    assert_eq!(voice_for(Some(&hu), Some("en-US"), "नमस्ते दुनिया"), None, "Devanagari tagged en-US too");
    let hi = Voice { id: "inc/hi".into(), name: "Hindi".into(), lang: "hi".into(), kind: VoiceKind::Espeak };
    assert_eq!(voice_for(Some(&hi), Some("hi-IN"), "नमस्ते"), Some(&hi));
    assert_eq!(voice_for(Some(&hi), Some("en-US"), "Hello"), None, "a Hindi voice doesn't read English");
    let ka = Voice { id: "ccs/ka".into(), name: "Georgian".into(), lang: "ka".into(), kind: VoiceKind::Espeak };
    assert_eq!(voice_for(Some(&ka), Some("en-US"), "Hello"), None);
}

#[test]
fn latin_text_is_told_by_its_letters() {
    assert!(is_latin_text("Árvíztűrő tükörfúrógép, Ñandú, Łódź 123"));
    assert!(!is_latin_text("Hello мир"));
    // Symbols, ligatures, modifier letters and a stray Greek letter don't make text foreign.
    assert!(is_latin_text("5 µm vastag réteg"));
    assert!(is_latin_text("A ﬁnom ﬁlm ﬂottul fut."));
    assert!(is_latin_text("Hawaiʻi és Oʼahu szigete"));
    assert!(is_latin_text("Az α-részecskék pályáját mérték a kísérletben."));
    assert!(!is_latin_text("123 – !"), "no letters: not Latin text");
}

#[test]
fn numbers_and_latin_text_keep_the_chosen_voice_where_their_tag_allows() {
    let hu = Voice { id: "urj/hu".into(), name: "Hungarian".into(), lang: "hu".into(), kind: VoiceKind::Espeak };
    let gb = Voice { id: "gmw/en".into(), name: "English".into(), lang: "en-gb".into(), kind: VoiceKind::Espeak };
    // A number in a Japanese, Russian or Hindi document keeps the voice of its tag; in a
    // document tagged with a Latin-script language it keeps the chosen voice.
    assert_eq!(voice_for(Some(&gb), Some("ja"), "2026"), None);
    assert_eq!(voice_for(Some(&gb), Some("ru-RU"), "3.14"), None);
    assert_eq!(voice_for(Some(&hu), Some("ja"), "2026"), None);
    assert_eq!(voice_for(Some(&hu), Some("hi-IN"), "2026"), None);
    assert_eq!(voice_for(Some(&hu), Some("en-US"), "2026"), Some(&hu), "a Hungarian document tagged en-US");
    assert_eq!(voice_for(Some(&hu), Some("de"), "3.14 – 42"), Some(&hu));
    assert_eq!(voice_for(Some(&hu), Some("en-US"), "5 µm vastag réteg"), Some(&hu));
    assert_eq!(voice_for(Some(&hu), Some("en-US"), "A ﬁnom ﬁlm ﬂottul fut."), Some(&hu));
    // A Latin-script voice reads Latin text whatever the tag: an English sentence in a Russian
    // document (every run `ru-RU`), or a Greek-tagged Latin sentence.
    assert_eq!(voice_for(Some(&gb), Some("ru-RU"), "This sentence is English."), Some(&gb));
    assert_eq!(voice_for(Some(&gb), Some("el"), "Plain Latin text."), Some(&gb));
    assert_eq!(voice_for(Some(&gb), Some("ru-RU"), "Это русский текст."), None, "Cyrillic keeps the Russian voice");
    // A voice whose language the program doesn't name reads Latin text, as before.
    for lang in ["", "none", "x"] {
        let unnamed = Voice { id: "v".into(), name: "Voice".into(), lang: lang.into(), kind: VoiceKind::Mac };
        assert_eq!(voice_for(Some(&unnamed), Some("en-US"), "Hello"), Some(&unnamed), "{lang:?}");
        assert_eq!(voice_for(Some(&unnamed), Some("ru"), "Привет"), None, "{lang:?}");
    }
}

#[test]
fn a_chosen_voice_reads_text_tagged_with_the_default_language() {
    // A new document (and almost every Word file) tags every run `en-US`, Hungarian text too:
    // the chosen Hungarian voice reads it, and only the Russian sentence gets its own voice.
    let doc = Document::from_text("Jó reggelt kívánok. Привет, как дела?");
    let s = sentences(&doc, &Pos::body(0, 0), None, true);
    assert_eq!(s.iter().map(|u| u.lang.as_deref()).collect::<Vec<_>>(), [Some("en-US"), Some("ru")]);
    let hu = Voice { id: "urj/hu".into(), name: "Hungarian".into(), lang: "hu".into(), kind: VoiceKind::Espeak };
    assert_eq!(voice_for(Some(&hu), s[0].lang.as_deref(), &s[0].text), Some(&hu));
    assert_eq!(voice_for(Some(&hu), s[1].lang.as_deref(), &s[1].text), None);
}

#[test]
fn the_chosen_voice_is_kept_and_reported() {
    let mut r = ReadAloud::default();
    assert_eq!(r.status()["voice"], Value::Null);
    // A silent backend never asks the system (no `espeak-ng --voices`, no server started).
    assert_ne!(r.backend, Backend::System);
    assert_eq!(r.voices().as_deref(), Some(&[][..]));
    r.set_voice(Some(" urj/hu "));
    assert_eq!(r.voice().as_deref(), Some("urj/hu"));
    assert_eq!(r.status()["voice"], "urj/hu");
    r.set_voice(Some(""));
    assert_eq!(r.voice(), None);
    r.set_voice(Some(&"x".repeat(MAX_VOICE_ID + 1)));
    assert_eq!(r.voice(), None, "an overlong id is dropped");
    r.set_voice(None);
    assert_eq!(r.voice(), None);
    // During reading a new voice restarts the sentence; the same voice again doesn't.
    r.backend = Backend::Hold;
    r.start(sentences(&Document::from_text("One. Two."), &Pos::body(0, 0), None, true));
    let generation = |r: &ReadAloud| lock(&r.shared.0).generation;
    let before = generation(&r);
    r.set_voice(Some("urj/hu"));
    assert_eq!(generation(&r), before + 1);
    r.set_voice(Some(" urj/hu "));
    assert_eq!(generation(&r), before + 1, "the same voice: the sentence goes on");
    r.stop();
}

#[test]
fn windows_speaks_with_the_chosen_voice() {
    let w = windows_speech_with(None, 1.0, Some("ru"), Some("Microsoft Szabolcs"));
    assert!(w.args[3].contains("$s.SelectVoice('Microsoft Szabolcs');"), "{}", w.args[3]);
    assert!(!w.args[3].contains("GetInstalledVoices"), "the chosen voice replaces the choice by language");
    let evil = windows_speech_with(None, 1.0, None, Some("x'); Remove-Item y; ('"));
    assert!(evil.args[3].contains("SelectVoice('x''); Remove-Item y; (''');"), "{}", evil.args[3]);
    // PowerShell ends a single-quoted string at a typographic quote too.
    let curly = windows_speech_with(None, 1.0, None, Some("x\u{2019}); Remove-Item y; (\u{2018}"));
    assert!(curly.args[3].contains("SelectVoice('x\u{2019}\u{2019}); Remove-Item y; (\u{2018}\u{2018}');"), "{}", curly.args[3]);
    assert_eq!(powershell_literal("a\u{201A}b\u{201B}c\nd"), "a\u{201A}\u{201A}b\u{201B}\u{201B}cd");
}

#[test]
fn speech_dispatcher_rate_follows_the_speed() {
    assert_eq!(spd_rate(1.0), 0);
    assert_eq!(spd_rate(MIN_RATE), -50);
    assert_eq!(spd_rate(MAX_RATE), 100);
    assert_eq!(spd_rate(2.0), 50);
    assert_eq!(spd_rate(10.0), 100);
    assert_eq!(spd_rate(0.0), -50);
    assert_eq!(spd_rate(f32::NAN), 0);
    assert_eq!(spd_rate(1.5), 25);
}

#[test]
fn speech_dispatcher_keeps_the_region() {
    let tag = |t| dispatcher_language(Some(t));
    assert_eq!(tag("en-GB").as_deref(), Some("en-GB"));
    assert_eq!(tag(" pt-BR ").as_deref(), Some("pt-BR"));
    assert_eq!(tag("zh-TW").as_deref(), Some("zh-TW"));
    assert_eq!(tag("hu").as_deref(), Some("hu"));
    assert_eq!(tag("HU-hu").as_deref(), Some("hu-HU"));
    // SSIP takes language and region: script and variant subtags are left out.
    assert_eq!(tag("sr-Latn-RS").as_deref(), Some("sr-RS"));
    assert_eq!(tag("zh-Hant-TW").as_deref(), Some("zh-TW"));
    assert_eq!(tag("es-419").as_deref(), Some("es-419"));
    assert_eq!(tag("de-CH-1996").as_deref(), Some("de-CH"));
    assert_eq!(tag("sr-Cyrl").as_deref(), Some("sr"));
    assert_eq!(dispatcher_language(Some("x-none")), None);
    assert_eq!(dispatcher_language(Some("hu\r\nQUIT")), None);
    assert_eq!(dispatcher_language(None), None);
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
