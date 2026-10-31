use super::*;

fn type_keys(a: &mut WordApp, ctx: &egui::Context, keys: &str) {
        for ch in keys.chars() {
            ctx.input_mut(|i| {
                i.events.clear();
                i.events.push(egui::Event::Text(ch.to_string()));
            });
            crate::keys::canvas_events(a, ctx);
            ctx.input_mut(|i| i.events.clear());
        }
    }

    #[test]
    fn bangla_jatiya_typing_independent_vowels_and_joint_words() {
        let mut a = app();
        a.session.bangla_mode = wordcraft_engine::cmd::bangla::BanglaMode::Jatiya;
        let ctx = egui::Context::default();

        // 1. Independent vowel: g + f -> আ
        type_keys(&mut a, &ctx, "gf");
        assert_eq!(body_text(&a).trim(), "আ");

        // 2. Clear document
        a.session.doc = wordcraft_doc::Document::new();
        a.session.sel = wordcraft_engine::Selection::caret(wordcraft_doc::Pos {
            story: wordcraft_doc::StoryRef::Body,
            path: wordcraft_doc::Path(vec![0]),
            off: 0,
        });

        // 3. Joint words:
        // j + g + N -> ক্ষ
        type_keys(&mut a, &ctx, "jgN");
        assert_eq!(body_text(&a).trim(), "ক্ষ");

        // Clear doc
        a.session.doc = wordcraft_doc::Document::new();
        a.session.sel = wordcraft_engine::Selection::caret(wordcraft_doc::Pos {
            story: wordcraft_doc::StoryRef::Body,
            path: wordcraft_doc::Path(vec![0]),
            off: 0,
        });

        // k + g + k -> ত্ত
        type_keys(&mut a, &ctx, "kgk");
        assert_eq!(body_text(&a).trim(), "ত্ত");

        // Clear doc
        a.session.doc = wordcraft_doc::Document::new();
        a.session.sel = wordcraft_engine::Selection::caret(wordcraft_doc::Pos {
            story: wordcraft_doc::StoryRef::Body,
            path: wordcraft_doc::Path(vec![0]),
            off: 0,
        });

        // 4. Pre-kar + conjunct:
        // d + j + g + N -> ক্ষি
        type_keys(&mut a, &ctx, "djgN");
        assert_eq!(body_text(&a).trim(), "ক্ষি");

        // Clear doc
        a.session.doc = wordcraft_doc::Document::new();
        a.session.sel = wordcraft_engine::Selection::caret(wordcraft_doc::Pos {
            story: wordcraft_doc::StoryRef::Body,
            path: wordcraft_doc::Path(vec![0]),
            off: 0,
        });

        // 5. Pre-kar + consonant + aa-kar -> O-kar:
        // c + n + f -> সো
        type_keys(&mut a, &ctx, "cnf");
        assert_eq!(body_text(&a).trim(), "সো");

        // Clear doc
        a.session.doc = wordcraft_doc::Document::new();
        a.session.sel = wordcraft_engine::Selection::caret(wordcraft_doc::Pos {
            story: wordcraft_doc::StoryRef::Body,
            path: wordcraft_doc::Path(vec![0]),
            off: 0,
        });

        // 6. Complete word: আমার (gf + m + f + v)
        type_keys(&mut a, &ctx, "gfmfv");
        assert_eq!(body_text(&a).trim(), "আমার");

        // Clear doc
        a.session.doc = wordcraft_doc::Document::new();
        a.session.sel = wordcraft_engine::Selection::caret(wordcraft_doc::Pos {
            story: wordcraft_doc::StoryRef::Body,
            path: wordcraft_doc::Path(vec![0]),
            off: 0,
        });

        // 7. Joint word with ri-kar and pre-kar: বৃষ্টি (h + a + d + N + g + t)
        type_keys(&mut a, &ctx, "hadNgt");
        assert_eq!(body_text(&a).trim(), "বৃষ্টি");

        // Clear doc
        a.session.doc = wordcraft_doc::Document::new();
        a.session.sel = wordcraft_engine::Selection::caret(wordcraft_doc::Pos {
            story: wordcraft_doc::StoryRef::Body,
            path: wordcraft_doc::Path(vec![0]),
            off: 0,
        });

        // 8. Reph: কর্ম (j + A + m)
        type_keys(&mut a, &ctx, "jAm");
        assert_eq!(body_text(&a).trim(), "কর্ম");

        // Clear doc
        a.session.doc = wordcraft_doc::Document::new();
        a.session.sel = wordcraft_engine::Selection::caret(wordcraft_doc::Pos {
            story: wordcraft_doc::StoryRef::Body,
            path: wordcraft_doc::Path(vec![0]),
            off: 0,
        });

        // 9. Ra-fala and Ya-fala: ছাত্র (y + f + k + z), সত্য (n + k + Z)
        type_keys(&mut a, &ctx, "yfzk");
        // Clear doc
        a.session.doc = wordcraft_doc::Document::new();
        a.session.sel = wordcraft_engine::Selection::caret(wordcraft_doc::Pos {
            story: wordcraft_doc::StoryRef::Body,
            path: wordcraft_doc::Path(vec![0]),
            off: 0,
        });
        type_keys(&mut a, &ctx, "kz");
        assert_eq!(body_text(&a).trim(), "ত্র");

        a.session.doc = wordcraft_doc::Document::new();
        a.session.sel = wordcraft_engine::Selection::caret(wordcraft_doc::Pos {
            story: wordcraft_doc::StoryRef::Body,
            path: wordcraft_doc::Path(vec![0]),
            off: 0,
        });
        type_keys(&mut a, &ctx, "kZ");
        assert_eq!(body_text(&a).trim(), "ত্য");

        // 10. Independent vowels in the middle of words (শব্দের মাঝে 'আ' / 'ই'):
        a.session.doc = wordcraft_doc::Document::new();
        a.session.sel = wordcraft_engine::Selection::caret(wordcraft_doc::Pos {
            story: wordcraft_doc::StoryRef::Body,
            path: wordcraft_doc::Path(vec![0]),
            off: 0,
        });
        type_keys(&mut a, &ctx, "jgf");
        assert_eq!(body_text(&a).trim(), "কআ");

        a.session.doc = wordcraft_doc::Document::new();
        a.session.sel = wordcraft_engine::Selection::caret(wordcraft_doc::Pos {
            story: wordcraft_doc::StoryRef::Body,
            path: wordcraft_doc::Path(vec![0]),
            off: 0,
        });
        type_keys(&mut a, &ctx, "jfgf");
        assert_eq!(body_text(&a).trim(), "কাআ");

        a.session.doc = wordcraft_doc::Document::new();
        a.session.sel = wordcraft_engine::Selection::caret(wordcraft_doc::Pos {
            story: wordcraft_doc::StoryRef::Body,
            path: wordcraft_doc::Path(vec![0]),
            off: 0,
        });
        type_keys(&mut a, &ctx, "hgd");
        assert_eq!(body_text(&a).trim(), "বই");

        // 11. Stop mark / Dari ('.' and 'G'):
        a.session.doc = wordcraft_doc::Document::new();
        a.session.sel = wordcraft_engine::Selection::caret(wordcraft_doc::Pos {
            story: wordcraft_doc::StoryRef::Body,
            path: wordcraft_doc::Path(vec![0]),
            off: 0,
        });
        type_keys(&mut a, &ctx, "j.");
        assert_eq!(body_text(&a).trim(), "ক।");

        a.session.doc = wordcraft_doc::Document::new();
        a.session.sel = wordcraft_engine::Selection::caret(wordcraft_doc::Pos {
            story: wordcraft_doc::StoryRef::Body,
            path: wordcraft_doc::Path(vec![0]),
            off: 0,
        });
        type_keys(&mut a, &ctx, "jG");
        assert_eq!(body_text(&a).trim(), "ক।");

        // 12. Bisarga ('|', shift of \ / ৎ) and Colon (':'):
        a.session.doc = wordcraft_doc::Document::new();
        a.session.sel = wordcraft_engine::Selection::caret(wordcraft_doc::Pos {
            story: wordcraft_doc::StoryRef::Body,
            path: wordcraft_doc::Path(vec![0]),
            off: 0,
        });
        type_keys(&mut a, &ctx, "|");
        assert_eq!(body_text(&a).trim(), "ঃ");

        a.session.doc = wordcraft_doc::Document::new();
        a.session.sel = wordcraft_engine::Selection::caret(wordcraft_doc::Pos {
            story: wordcraft_doc::StoryRef::Body,
            path: wordcraft_doc::Path(vec![0]),
            off: 0,
        });
        type_keys(&mut a, &ctx, "j|");
        assert_eq!(body_text(&a).trim(), "কঃ");

        a.session.doc = wordcraft_doc::Document::new();
        a.session.sel = wordcraft_engine::Selection::caret(wordcraft_doc::Pos {
            story: wordcraft_doc::StoryRef::Body,
            path: wordcraft_doc::Path(vec![0]),
            off: 0,
        });
        type_keys(&mut a, &ctx, "j:");
        assert_eq!(body_text(&a).trim(), "ক:");

        // 13. Pre-kar with ড়/ঢ়/য়/ৎ and pre-kar with conjuncts:
        a.session.doc = wordcraft_doc::Document::new();
        a.session.sel = wordcraft_engine::Selection::caret(wordcraft_doc::Pos {
            story: wordcraft_doc::StoryRef::Body,
            path: wordcraft_doc::Path(vec![0]),
            off: 0,
        });
        // বাড়ি: h (ব), f (া), d (ি), p (ড়)
        type_keys(&mut a, &ctx, "hfdp");
        assert_eq!(body_text(&a).trim(), "বাড়ি");

        // ক্লে: c (ে), j (ক), g (্), V (ল)
        a.session.doc = wordcraft_doc::Document::new();
        a.session.sel = wordcraft_engine::Selection::caret(wordcraft_doc::Pos {
            story: wordcraft_doc::StoryRef::Body,
            path: wordcraft_doc::Path(vec![0]),
            off: 0,
        });
        type_keys(&mut a, &ctx, "cjgV");
        assert_eq!(body_text(&a).trim(), "ক্লে");

        // ক্লো: c (ে), j (ক), g (্), V (ল), f (া)
        a.session.doc = wordcraft_doc::Document::new();
        a.session.sel = wordcraft_engine::Selection::caret(wordcraft_doc::Pos {
            story: wordcraft_doc::StoryRef::Body,
            path: wordcraft_doc::Path(vec![0]),
            off: 0,
        });
        type_keys(&mut a, &ctx, "cjgVf");
        assert_eq!(body_text(&a).trim(), "ক্লো");

        // 14. Pre-kar after consonant in words:
        // বসে: h (ব), c (ে), n (স) -> বসে (not বেস)
        a.session.doc = wordcraft_doc::Document::new();
        a.session.sel = wordcraft_engine::Selection::caret(wordcraft_doc::Pos {
            story: wordcraft_doc::StoryRef::Body,
            path: wordcraft_doc::Path(vec![0]),
            off: 0,
        });
        type_keys(&mut a, &ctx, "hcn");
        assert_eq!(body_text(&a).trim(), "বসে");

        // করে: j (ক), c (ে), v (র) -> করে (not কের)
        a.session.doc = wordcraft_doc::Document::new();
        a.session.sel = wordcraft_engine::Selection::caret(wordcraft_doc::Pos {
            story: wordcraft_doc::StoryRef::Body,
            path: wordcraft_doc::Path(vec![0]),
            off: 0,
        });
        type_keys(&mut a, &ctx, "jcv");
        assert_eq!(body_text(&a).trim(), "করে");

        // কেউ: c (ে), j (ক), g (linker), s (উ) -> কেউ (not কউে)
        a.session.doc = wordcraft_doc::Document::new();
        a.session.sel = wordcraft_engine::Selection::caret(wordcraft_doc::Pos {
            story: wordcraft_doc::StoryRef::Body,
            path: wordcraft_doc::Path(vec![0]),
            off: 0,
        });
        type_keys(&mut a, &ctx, "cjgs");
        assert_eq!(body_text(&a).trim(), "কেউ");

        // আমারে: gf (আ), m (ম), f (া), c (ে), v (র) -> আমারে
        a.session.doc = wordcraft_doc::Document::new();
        a.session.sel = wordcraft_engine::Selection::caret(wordcraft_doc::Pos {
            story: wordcraft_doc::StoryRef::Body,
            path: wordcraft_doc::Path(vec![0]),
            off: 0,
        });
        type_keys(&mut a, &ctx, "gfmfcv");
        assert_eq!(body_text(&a).trim(), "আমারে");

        // 15. Pre-kar + conjunct + Ra-fala ('z') / Ya-fala ('Z') in Jatiya:
        // যান্ত্রিক: w (য), f (া), d (ি), b (ন), g (্), k (ত), z (্র), j (ক) -> যান্ত্রিক (not যান্তি্রক)
        a.session.doc = wordcraft_doc::Document::new();
        a.session.sel = wordcraft_engine::Selection::caret(wordcraft_doc::Pos {
            story: wordcraft_doc::StoryRef::Body,
            path: wordcraft_doc::Path(vec![0]),
            off: 0,
        });
        type_keys(&mut a, &ctx, "wfdbgkzj");
        assert_eq!(body_text(&a).trim(), "যান্ত্রিক");

        // যান্ত্রিক with explicit 'g' before 'z': wfdbgk g z j -> যান্ত্রিক
        a.session.doc = wordcraft_doc::Document::new();
        a.session.sel = wordcraft_engine::Selection::caret(wordcraft_doc::Pos {
            story: wordcraft_doc::StoryRef::Body,
            path: wordcraft_doc::Path(vec![0]),
            off: 0,
        });
        type_keys(&mut a, &ctx, "wfdbgkgzj");
        assert_eq!(body_text(&a).trim(), "যান্ত্রিক");

        // প্রিয়: d (ি), r (প), z (্র), W (য়) -> প্রিয়
        a.session.doc = wordcraft_doc::Document::new();
        a.session.sel = wordcraft_engine::Selection::caret(wordcraft_doc::Pos {
            story: wordcraft_doc::StoryRef::Body,
            path: wordcraft_doc::Path(vec![0]),
            off: 0,
        });
        type_keys(&mut a, &ctx, "drzW");
        assert_eq!(body_text(&a).trim(), "প্রিয়");

        // প্রেম: c (ে), r (প), z (্র), m (ম) -> প্রেম
        a.session.doc = wordcraft_doc::Document::new();
        a.session.sel = wordcraft_engine::Selection::caret(wordcraft_doc::Pos {
            story: wordcraft_doc::StoryRef::Body,
            path: wordcraft_doc::Path(vec![0]),
            off: 0,
        });
        type_keys(&mut a, &ctx, "crzm");
        assert_eq!(body_text(&a).trim(), "প্রেম");

        // জ্যোতি: c (ে), u (জ), Z (্য), f (া), d (ি), k (ত) -> জ্যোতি
        a.session.doc = wordcraft_doc::Document::new();
        a.session.sel = wordcraft_engine::Selection::caret(wordcraft_doc::Pos {
            story: wordcraft_doc::StoryRef::Body,
            path: wordcraft_doc::Path(vec![0]),
            off: 0,
        });
        type_keys(&mut a, &ctx, "cuZfdk");
        assert_eq!(body_text(&a).trim(), "জ্যোতি");
    }

    #[test]
    fn bangla_ansi_typing_joint_words_and_vowels() {
        let mut a = app();
        a.session.bangla_mode = wordcraft_engine::cmd::bangla::BanglaMode::Ansi;
        let ctx = egui::Context::default();

        // 1. Joint word in ANSI: j + g + N -> ¶ (ক্ষ)
        type_keys(&mut a, &ctx, "jgN");
        assert_eq!(body_text(&a).trim(), "¶");

        // Clear doc
        a.session.doc = wordcraft_doc::Document::new();
        a.session.sel = wordcraft_engine::Selection::caret(wordcraft_doc::Pos {
            story: wordcraft_doc::StoryRef::Body,
            path: wordcraft_doc::Path(vec![0]),
            off: 0,
        });

        // 2. Joint word: k + g + k -> Ë (ত্ত)
        type_keys(&mut a, &ctx, "kgk");
        assert_eq!(body_text(&a).trim(), "Ë");

        // Clear doc
        a.session.doc = wordcraft_doc::Document::new();
        a.session.sel = wordcraft_engine::Selection::caret(wordcraft_doc::Pos {
            story: wordcraft_doc::StoryRef::Body,
            path: wordcraft_doc::Path(vec![0]),
            off: 0,
        });

        // 3. Joint word: b + g + k -> šÍ (ন্ত)
        type_keys(&mut a, &ctx, "bgk");
        assert_eq!(body_text(&a).trim(), "šÍ");

        // Clear doc
        a.session.doc = wordcraft_doc::Document::new();
        a.session.sel = wordcraft_engine::Selection::caret(wordcraft_doc::Pos {
            story: wordcraft_doc::StoryRef::Body,
            path: wordcraft_doc::Path(vec![0]),
            off: 0,
        });

        // 4. Independent vowel: g + f -> Av (আ)
        type_keys(&mut a, &ctx, "gf");
        assert_eq!(body_text(&a).trim(), "Av");

        // 5. Dari in ANSI ('.' and 'G'):
        a.session.doc = wordcraft_doc::Document::new();
        a.session.sel = wordcraft_engine::Selection::caret(wordcraft_doc::Pos {
            story: wordcraft_doc::StoryRef::Body,
            path: wordcraft_doc::Path(vec![0]),
            off: 0,
        });
        type_keys(&mut a, &ctx, "j.");
        assert_eq!(body_text(&a).trim(), "K.");

        a.session.doc = wordcraft_doc::Document::new();
        a.session.sel = wordcraft_engine::Selection::caret(wordcraft_doc::Pos {
            story: wordcraft_doc::StoryRef::Body,
            path: wordcraft_doc::Path(vec![0]),
            off: 0,
        });
        type_keys(&mut a, &ctx, "jG");
        assert_eq!(body_text(&a).trim(), "K।");

        // 6. Bisarga in ANSI ('|') and Colon (':'):
        a.session.doc = wordcraft_doc::Document::new();
        a.session.sel = wordcraft_engine::Selection::caret(wordcraft_doc::Pos {
            story: wordcraft_doc::StoryRef::Body,
            path: wordcraft_doc::Path(vec![0]),
            off: 0,
        });
        type_keys(&mut a, &ctx, "|");
        assert_eq!(body_text(&a).trim(), "t");

        a.session.doc = wordcraft_doc::Document::new();
        a.session.sel = wordcraft_engine::Selection::caret(wordcraft_doc::Pos {
            story: wordcraft_doc::StoryRef::Body,
            path: wordcraft_doc::Path(vec![0]),
            off: 0,
        });
        type_keys(&mut a, &ctx, ":");
        assert_eq!(body_text(&a).trim(), ":");

        // 7. Reph in ANSI ('A'):
        a.session.doc = wordcraft_doc::Document::new();
        a.session.sel = wordcraft_engine::Selection::caret(wordcraft_doc::Pos {
            story: wordcraft_doc::StoryRef::Body,
            path: wordcraft_doc::Path(vec![0]),
            off: 0,
        });
        type_keys(&mut a, &ctx, "jA");
        assert_eq!(body_text(&a).trim(), "K©");

        a.session.doc = wordcraft_doc::Document::new();
        a.session.sel = wordcraft_engine::Selection::caret(wordcraft_doc::Pos {
            story: wordcraft_doc::StoryRef::Body,
            path: wordcraft_doc::Path(vec![0]),
            off: 0,
        });
        type_keys(&mut a, &ctx, "Aj");
        assert_eq!(body_text(&a).trim(), "K©");

        // Pre-kar + Reph in ANSI: d (w / ি) + A (reph) + j (K / ক) -> wK© (র্কি)
        a.session.doc = wordcraft_doc::Document::new();
        a.session.sel = wordcraft_engine::Selection::caret(wordcraft_doc::Pos {
            story: wordcraft_doc::StoryRef::Body,
            path: wordcraft_doc::Path(vec![0]),
            off: 0,
        });
        type_keys(&mut a, &ctx, "dAj");
        assert_eq!(body_text(&a).trim(), "wK©");

        // 8. Ra-fala in ANSI ('z'):
        // k (ত) + z (্র) -> Î (ত্র)
        a.session.doc = wordcraft_doc::Document::new();
        a.session.sel = wordcraft_engine::Selection::caret(wordcraft_doc::Pos {
            story: wordcraft_doc::StoryRef::Body,
            path: wordcraft_doc::Path(vec![0]),
            off: 0,
        });
        type_keys(&mut a, &ctx, "kz");
        assert_eq!(body_text(&a).trim(), "Î");

        // j (ক) + z (্র) -> µ (ক্র)
        a.session.doc = wordcraft_doc::Document::new();
        a.session.sel = wordcraft_engine::Selection::caret(wordcraft_doc::Pos {
            story: wordcraft_doc::StoryRef::Body,
            path: wordcraft_doc::Path(vec![0]),
            off: 0,
        });
        type_keys(&mut a, &ctx, "jz");
        assert_eq!(body_text(&a).trim(), "µ");

        // 9. Conjunct V + g + o -> î (ল্গ):
        a.session.doc = wordcraft_doc::Document::new();
        a.session.sel = wordcraft_engine::Selection::caret(wordcraft_doc::Pos {
            story: wordcraft_doc::StoryRef::Body,
            path: wordcraft_doc::Path(vec![0]),
            off: 0,
        });
        type_keys(&mut a, &ctx, "Vgo");
        assert_eq!(body_text(&a).trim(), "î");

        // 10. Initial vs Medial E-kar in ANSI typing:
        // Initial E-kar in "তোমায়" (c + k + f + m + f + W) -> "†Zvgvq" (starts with †)
        a.session.doc = wordcraft_doc::Document::new();
        a.session.sel = wordcraft_engine::Selection::caret(wordcraft_doc::Pos {
            story: wordcraft_doc::StoryRef::Body,
            path: wordcraft_doc::Path(vec![0]),
            off: 0,
        });
        type_keys(&mut a, &ctx, "ckfmfW");
        assert_eq!(body_text(&a).trim(), "†Zvgvq");

        // Medial E-kar in "ভালোবাসি" (H + f + c + V + f + h + f + d + n) -> "fv‡jvevwm" (uses ‡ without gap before লো)
        a.session.doc = wordcraft_doc::Document::new();
        a.session.sel = wordcraft_engine::Selection::caret(wordcraft_doc::Pos {
            story: wordcraft_doc::StoryRef::Body,
            path: wordcraft_doc::Path(vec![0]),
            off: 0,
        });
        type_keys(&mut a, &ctx, "HfcVfhfdn");
        assert_eq!(body_text(&a).trim(), "fv‡jvevwm");
    }

    #[test]
    fn bangla_phonetic_typing_preserves_earlier_words_on_space() {
        let mut a = app();
        a.session.bangla_mode = wordcraft_engine::cmd::bangla::BanglaMode::Phonetic;
        let ctx = egui::Context::default();

        // Type "amar " then "shonar " then "bangla"
        type_keys(&mut a, &ctx, "amar ");
        assert_eq!(body_text(&a).trim(), "আমার");

        type_keys(&mut a, &ctx, "shonar ");
        assert_eq!(body_text(&a).trim(), "আমার সোনার");

        type_keys(&mut a, &ctx, "bangla");
        assert_eq!(body_text(&a).trim(), "আমার সোনার বাংলা");

        // Dari in Phonetic ('.' and '|'):
        type_keys(&mut a, &ctx, ".");
        assert_eq!(body_text(&a).trim(), "আমার সোনার বাংলা।");

        // Bisarga in Phonetic ('|', shift of \ / ৎ) and Colon (':'):
        a.session.doc = wordcraft_doc::Document::new();
        a.session.sel = wordcraft_engine::Selection::caret(wordcraft_doc::Pos {
            story: wordcraft_doc::StoryRef::Body,
            path: wordcraft_doc::Path(vec![0]),
            off: 0,
        });
        type_keys(&mut a, &ctx, ":");
        assert_eq!(body_text(&a).trim(), ":");

        // Dari in Phonetic ('|', Shift + \):
        a.session.doc = wordcraft_doc::Document::new();
        a.session.sel = wordcraft_engine::Selection::caret(wordcraft_doc::Pos {
            story: wordcraft_doc::StoryRef::Body,
            path: wordcraft_doc::Path(vec![0]),
            off: 0,
        });
        type_keys(&mut a, &ctx, "|");
        assert_eq!(body_text(&a).trim(), "।");

        // User requested sentence typing verification:
        a.session.doc = wordcraft_doc::Document::new();
        a.session.sel = wordcraft_engine::Selection::caret(wordcraft_doc::Pos {
            story: wordcraft_doc::StoryRef::Body,
            path: wordcraft_doc::Path(vec![0]),
            off: 0,
        });
        type_keys(&mut a, &ctx, "ami bangladesh krishi banke chakri kori|");
        assert_eq!(body_text(&a).trim(), "আমি বাংলাদেশ কৃষি ব্যাংকে চাকরি করি।");

        a.session.doc = wordcraft_doc::Document::new();
        a.session.sel = wordcraft_engine::Selection::caret(wordcraft_doc::Pos {
            story: wordcraft_doc::StoryRef::Body,
            path: wordcraft_doc::Path(vec![0]),
            off: 0,
        });
        type_keys(&mut a, &ctx, "dukho ");
        assert_eq!(body_text(&a).trim(), "দুঃখ");

        // Middle-of-word independent 'আ':
        a.session.doc = wordcraft_doc::Document::new();
        a.session.sel = wordcraft_engine::Selection::caret(wordcraft_doc::Pos {
            story: wordcraft_doc::StoryRef::Body,
            path: wordcraft_doc::Path(vec![0]),
            off: 0,
        });
        type_keys(&mut a, &ctx, "naA ");
        assert_eq!(body_text(&a).trim(), "নাআ");

        a.session.doc = wordcraft_doc::Document::new();
        a.session.sel = wordcraft_engine::Selection::caret(wordcraft_doc::Pos {
            story: wordcraft_doc::StoryRef::Body,
            path: wordcraft_doc::Path(vec![0]),
            off: 0,
        });
        type_keys(&mut a, &ctx, "k'a ");
        assert_eq!(body_text(&a).trim(), "কআ");

        // Chandrabindu '^':
        a.session.doc = wordcraft_doc::Document::new();
        a.session.sel = wordcraft_engine::Selection::caret(wordcraft_doc::Pos {
            story: wordcraft_doc::StoryRef::Body,
            path: wordcraft_doc::Path(vec![0]),
            off: 0,
        });
        type_keys(&mut a, &ctx, "c^ad ");
        assert_eq!(body_text(&a).trim(), "চাঁদ");

        // Khanda Ta 't`':
        a.session.doc = wordcraft_doc::Document::new();
        a.session.sel = wordcraft_engine::Selection::caret(wordcraft_doc::Pos {
            story: wordcraft_doc::StoryRef::Body,
            path: wordcraft_doc::Path(vec![0]),
            off: 0,
        });
        type_keys(&mut a, &ctx, "hot`hat ");
        assert_eq!(body_text(&a).trim(), "হঠাৎ");

        // Classic Bijoy shortcuts: Ctrl+Alt+B and Ctrl+Alt+V
        ctx.input_mut(|i| {
            i.events.clear();
            i.events.push(egui::Event::Key {
                key: egui::Key::B,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers { alt: true, ctrl: true, shift: false, command: false, mac_cmd: false },
            });
        });
        crate::keys::canvas_events(&mut a, &ctx);
        assert_eq!(a.session.bangla_mode, wordcraft_engine::cmd::bangla::BanglaMode::Ansi);

        ctx.input_mut(|i| {
            i.events.clear();
            i.events.push(egui::Event::Key {
                key: egui::Key::V,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers { alt: true, ctrl: true, shift: false, command: false, mac_cmd: false },
            });
        });
        crate::keys::canvas_events(&mut a, &ctx);
        assert_eq!(a.session.bangla_mode, wordcraft_engine::cmd::bangla::BanglaMode::Jatiya);
    }

    #[test]
    fn review_phonetic_composition_must_not_erase_text_after_caret_relocation() {
        let mut a = app();
        a.session.doc = wordcraft_doc::Document::from_text("KEEP ");
        a.session.bangla_mode = wordcraft_engine::cmd::bangla::BanglaMode::Phonetic;
        let ctx = egui::Context::default();

        // Place caret at offset 5 (end of "KEEP ")
        a.session.sel = wordcraft_engine::Selection::caret(wordcraft_doc::Pos {
            story: wordcraft_doc::StoryRef::Body,
            path: wordcraft_doc::Path(vec![0]),
            off: 5,
        });

        // Type phonetic "ami"
        type_keys(&mut a, &ctx, "ami");
        assert_eq!(body_text(&a), "KEEP আমি");

        // Relocate caret to offset 4 (after 'P', before ' ') without navigation keys (e.g. canvas click)
        a.session.sel = wordcraft_engine::Selection::caret(wordcraft_doc::Pos {
            story: wordcraft_doc::StoryRef::Body,
            path: wordcraft_doc::Path(vec![0]),
            off: 4,
        });

        // Type 'b' at the new caret position
        type_keys(&mut a, &ctx, "b");
        let text = body_text(&a);
        assert!(!text.contains("Kআমিব আমি"), "EEP must not be erased: {text}");
        assert_eq!(text, "KEEPব আমি");
    }

    
