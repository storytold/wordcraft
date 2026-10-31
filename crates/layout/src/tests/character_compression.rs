//! Word-measured character compression at the end of a justified line.

use super::*;
use wordcraft_doc::{CharacterSpacingControl, para_block};

fn first_paragraph(size: f32, width_em: f32, align: Align, policy: CharacterSpacingControl, punctuation: char, hang: bool) -> Arc<ParaLayout> {
    let mut doc = Document::new();
    doc.settings.character_spacing_control = policy;
    let text = format!("漢漢漢漢漢{punctuation}漢漢字終");
    let props = CharProps {
        font: Some("Hiragino Kaku Gothic ProN".into()),
        font_ea: Some("Hiragino Kaku Gothic ProN".into()),
        size: Some(size),
        spacing: Some(0.0),
        ..Default::default()
    };
    let mut p = Paragraph::with_text(&text, props);
    p.props.align = Some(align);
    p.props.indent_right = Some(468.0 - width_em * size);
    p.props.overflow_punct = Some(hang);
    p.props.auto_space_de = Some(false);
    p.props.auto_space_dn = Some(false);
    doc.body = vec![para_block(p)];
    lay(&doc)
        .pages
        .iter()
        .flat_map(|page| &page.items)
        .find_map(|item| match item {
            Placed::Lines { para, l0: 0, story: StoryRef::Body, .. } => Some(para.clone()),
            _ => None,
        })
        .expect("body paragraph")
}

#[test]
fn justified_comma_can_fit_by_at_most_half_an_em_when_enabled() {
    let face = wordcraft_fonts::word::resolve("Hiragino Kaku Gothic ProN", false, false);
    if face.substituted {
        eprintln!("skipped: needs installed Hiragino Kaku Gothic ProN");
        return;
    }
    for size in [9.0, 12.0, 18.0] {
        for policy in [CharacterSpacingControl::CompressPunctuation, CharacterSpacingControl::CompressPunctuationAndJapaneseKana] {
            let fitted = first_paragraph(size, 5.5, Align::Justify, policy.clone(), '、', false);
            assert_eq!(fitted.lines[0].stop, "漢漢漢漢漢、".len(), "{size}pt must include the comma");
            let comma = fitted.clusters.iter().find(|c| c.start == "漢漢漢漢漢".len()).expect("comma cluster");
            assert!((comma.adv - size * 0.5).abs() < 0.01, "{size}pt comma advance {}", comma.adv);
            let too_short = first_paragraph(size, 5.45, Align::Justify, policy, '、', false);
            assert!(too_short.lines[0].stop < "漢漢漢漢漢、".len(), "{size}pt must not fit beyond the measured limit");
        }
        let plain = first_paragraph(size, 5.5, Align::Justify, CharacterSpacingControl::DoNotCompress, '、', false);
        let left = first_paragraph(size, 5.5, Align::Left, CharacterSpacingControl::CompressPunctuation, '、', false);
        assert!(plain.lines[0].stop < "漢漢漢漢漢、".len());
        assert!(left.lines[0].stop < "漢漢漢漢漢、".len());
        let latin_comma = first_paragraph(size, 5.5, Align::Justify, CharacterSpacingControl::CompressPunctuation, ',', false);
        let latin_comma_plain = first_paragraph(size, 5.5, Align::Justify, CharacterSpacingControl::DoNotCompress, ',', false);
        assert_eq!(latin_comma.lines[0].stop, latin_comma_plain.lines[0].stop, "halfwidth punctuation stays unchanged");
    }
}

#[test]
fn compressed_comma_stays_compact_without_hanging_but_justifies_with_hanging() {
    let face = wordcraft_fonts::word::resolve("Hiragino Kaku Gothic ProN", false, false);
    if face.substituted {
        eprintln!("skipped: needs installed Hiragino Kaku Gothic ProN");
        return;
    }
    for size in [9.0, 12.0, 18.0] {
        let comma_offset = "漢漢漢漢漢".len();
        let at = |width, hang| {
            let p = first_paragraph(size, width, Align::Justify, CharacterSpacingControl::CompressPunctuation, '、', hang);
            assert_eq!(p.lines[0].stop, "漢漢漢漢漢、".len());
            p.x_of(0, comma_offset).expect("comma origin")
        };
        let compact = (at(5.5, false), at(5.7, false));
        assert!((compact.0 - compact.1).abs() < 0.01, "{size}pt without hanging: {compact:?}");
        let hanging = (at(5.5, true), at(5.7, true));
        assert!(hanging.1 - hanging.0 > size * 0.1, "{size}pt with hanging: {hanging:?}");
    }
}

#[test]
fn backtracking_before_a_compressed_comma_keeps_its_next_line_advance() {
    let face = wordcraft_fonts::word::resolve("Hiragino Kaku Gothic ProN", false, false);
    if face.substituted {
        eprintln!("skipped: needs installed Hiragino Kaku Gothic ProN");
        return;
    }
    let mut doc = Document::new();
    doc.settings.character_spacing_control = CharacterSpacingControl::CompressPunctuation;
    let text = "漢漢漢漢漢、。漢";
    let props = CharProps {
        font: Some("Hiragino Kaku Gothic ProN".into()),
        font_ea: Some("Hiragino Kaku Gothic ProN".into()),
        size: Some(9.0),
        kern: Some(0.0),
        ..Default::default()
    };
    let mut p = Paragraph::with_text(text, props);
    p.props.align = Some(Align::Justify);
    p.props.indent_right = Some(468.0 - 9.0 * 5.5);
    p.props.overflow_punct = Some(false);
    p.props.auto_space_de = Some(false);
    p.props.auto_space_dn = Some(false);
    doc.body = vec![para_block(p)];
    let layout = lay(&doc);
    let para = layout
        .pages
        .iter()
        .flat_map(|page| &page.items)
        .find_map(|item| match item {
            Placed::Lines { para, l0: 0, story: StoryRef::Body, .. } => Some(para),
            _ => None,
        })
        .expect("body paragraph");
    let comma = para.clusters.iter().find(|c| c.start == "漢漢漢漢漢".len()).expect("comma cluster");
    assert!(para.lines[0].stop <= comma.start, "the following closing mark backtracks before the comma");
    assert!((comma.adv - 9.0).abs() < 0.01, "a discarded compression must not survive on the next line: {}", comma.adv);
}

#[test]
fn punctuation_after_default_or_left_tab_fits_without_moving_the_stop() {
    use wordcraft_doc::props::{TabAlign, TabLeader, TabStop};
    let face = wordcraft_fonts::word::resolve("Hiragino Kaku Gothic ProN", false, false);
    if face.substituted {
        eprintln!("skipped: needs installed Hiragino Kaku Gothic ProN");
        return;
    }
    let text = "漢、\t漢ぁ漢漢、漢終";
    for size in [5.0, 5.5, 6.5, 7.5, 8.0, 9.0, 10.0, 10.5, 11.0, 12.0, 14.0, 16.0, 18.0, 20.0, 22.0, 24.0, 26.0, 28.0, 36.0, 48.0, 72.0] {
        for explicit in [false, true] {
            for policy in [CharacterSpacingControl::DoNotCompress, CharacterSpacingControl::CompressPunctuation] {
                let mut doc = Document::new();
                doc.settings.default_tab = size * 3.0;
                doc.settings.character_spacing_control = policy.clone();
                doc.last_section.page_w = 1000.0;
                let mut p = Paragraph::with_text(
                    text,
                    CharProps {
                        font: Some("Hiragino Kaku Gothic ProN".into()),
                        font_ea: Some("Hiragino Kaku Gothic ProN".into()),
                        size: Some(size),
                        spacing: Some(0.0),
                        kern: Some(0.0),
                        ..Default::default()
                    },
                );
                p.props.align = Some(Align::Justify);
                p.props.indent_right = Some(856.0 - size * 7.55);
                p.props.overflow_punct = Some(false);
                p.props.auto_space_de = Some(false);
                p.props.auto_space_dn = Some(false);
                if explicit {
                    p.props.tabs = Some(vec![TabStop { pos: size * 3.0, align: TabAlign::Left, leader: TabLeader::None }]);
                }
                doc.body = vec![para_block(p)];
                let layout = lay(&doc);
                let para = layout
                    .pages
                    .iter()
                    .flat_map(|page| &page.items)
                    .find_map(|item| match item {
                        Placed::Lines { para, l0: 0, story: StoryRef::Body, .. } => Some(para),
                        _ => None,
                    })
                    .expect("body paragraph");
                let x = para.x_of(0, "漢、\t".len()).expect("tab stop");
                assert!((x - size * 3.0).abs() < 0.01, "{size}pt explicit={explicit} {policy:?}: tab moved to {x}");
                let expected = if policy == CharacterSpacingControl::CompressPunctuation { "漢、\t漢ぁ漢漢、" } else { "漢、\t漢ぁ漢" };
                assert_eq!(para.lines[0].stop, expected.len(), "{size}pt explicit={explicit} {policy:?}");
            }
        }
    }
}

#[test]
fn measured_fullwidth_marks_fit_without_changing_halfwidth_brackets() {
    let face = wordcraft_fonts::word::resolve("Hiragino Kaku Gothic ProN", false, false);
    if face.substituted {
        eprintln!("skipped: needs installed Hiragino Kaku Gothic ProN");
        return;
    }
    for size in [5.0, 5.5, 6.5, 7.5, 8.0, 9.0, 10.0, 10.5, 11.0, 12.0, 14.0, 16.0, 18.0, 20.0, 22.0, 24.0, 26.0, 28.0, 36.0, 48.0, 72.0] {
        for ch in ['、', '。', '，', '．', '」', '』', '）'] {
            let p = first_paragraph(size, 5.5, Align::Justify, CharacterSpacingControl::CompressPunctuation, ch, false);
            assert_eq!(p.lines[0].stop, format!("漢漢漢漢漢{ch}").len(), "{size}pt {ch}");
        }
        for ch in ['「', '『', '（'] {
            let fitted = first_paragraph(size, 6.5, Align::Justify, CharacterSpacingControl::CompressPunctuation, ch, false);
            let plain = first_paragraph(size, 6.5, Align::Justify, CharacterSpacingControl::DoNotCompress, ch, false);
            assert_eq!(fitted.lines[0].stop, format!("漢漢漢漢漢{ch}漢").len(), "{size}pt {ch}");
            assert!(plain.lines[0].stop < fitted.lines[0].stop, "{size}pt {ch} without compression");
            let offset = "漢漢漢漢漢".len();
            let glyph_dx = |p: &ParaLayout| {
                let c = p.clusters.iter().find(|c| c.start == offset).expect("opening mark");
                p.glyphs.get(c.g0 as usize).expect("opening glyph").dx
            };
            assert!((glyph_dx(&fitted) - glyph_dx(&plain) + 0.5 * size).abs() < 0.01, "{size}pt {ch} ink position");
        }
    }
}

#[test]
fn japanese_small_kana_breaks_follow_strict_kinsoku() {
    for size in [5.0, 5.5, 6.5, 7.5, 8.0, 9.0, 10.0, 10.5, 11.0, 12.0, 14.0, 16.0, 18.0, 20.0, 22.0, 24.0, 26.0, 28.0, 36.0, 48.0, 72.0] {
        for (language, strict, kinsoku, expected) in [
            (None, false, true, true),
            (None, true, true, false),
            (None, true, false, true),
            (Some("ja-JP"), false, true, true),
            (Some("ja-JP"), true, true, false),
            (Some("ja-JP"), true, false, true),
            (Some("zh-CN"), false, true, false),
            (Some("ko-KR"), false, true, false),
            (Some("en-US"), false, true, false),
        ] {
            for small in ['ぁ', 'ァ'] {
                let mut doc = Document::new();
                doc.settings.strict_first_and_last_chars = strict;
                doc.styles.default_chr.lang = None;
                doc.styles.default_chr.lang_ea = None;
                let text = format!("漢{small}漢");
                let props = CharProps { size: Some(size), lang_ea: language.map(str::to_string), ..Default::default() };
                let mut p = Paragraph::with_text(&text, props);
                p.props.kinsoku = Some(kinsoku);
                doc.body = vec![para_block(p)];
                let layout = lay(&doc);
                let para = layout
                    .pages
                    .iter()
                    .flat_map(|page| &page.items)
                    .find_map(|item| match item {
                        Placed::Lines { para, l0: 0, story: StoryRef::Body, .. } => Some(para),
                        _ => None,
                    })
                    .expect("body paragraph");
                let first = para.clusters.iter().find(|c| c.start == 0).expect("first character");
                assert_eq!(first.break_after, expected, "{size}pt {language:?} strict={strict} kinsoku={kinsoku} {small}");
            }
        }
    }
}

/// Word measured kana-on at 5.94em across the size matrix and 72pt around 1/16em.
#[test]
fn measured_kana_only_fit_with_kana_compression_policy() {
    if wordcraft_fonts::word::resolve("Hiragino Kaku Gothic ProN", false, false).substituted {
        return;
    }
    for size in [5.0, 5.5, 6.5, 7.5, 8.0, 9.0, 10.0, 10.5, 11.0, 12.0, 14.0, 16.0, 18.0, 20.0, 22.0, 24.0, 26.0, 28.0, 36.0, 48.0, 72.0] {
        for kana in ['あ', 'ア', 'ぁ', 'ァ', 'い', 'え', 'お', 'カ', 'キ', 'ク', 'ゃ', 'ャ', 'っ', 'ッ', 'ヴ'] {
            let expected = format!("漢漢漢漢漢{kana}").len();
            let fit = first_paragraph(size, 5.94, Align::Justify, CharacterSpacingControl::CompressPunctuationAndJapaneseKana, kana, false);
            assert_eq!(fit.lines[0].stop, expected, "{size}pt {kana}");
            for policy in [CharacterSpacingControl::DoNotCompress, CharacterSpacingControl::CompressPunctuation] {
                let plain = first_paragraph(size, 5.94, Align::Justify, policy, kana, false);
                assert!(plain.lines[0].stop < expected, "{size}pt {kana}");
            }
            let short = first_paragraph(size, 5.92, Align::Justify, CharacterSpacingControl::CompressPunctuationAndJapaneseKana, kana, false);
            assert!(short.lines[0].stop < expected, "{size}pt {kana} exceeds measured room");
        }
    }
}

#[test]
fn kana_compression_does_not_change_halfwidth_or_combining_clusters() {
    for text in ["漢漢漢漢漢ｱ漢漢", "漢漢漢漢漢う\u{3099}漢漢"] {
        let make = |policy| {
            let mut doc = Document::new();
            doc.settings.character_spacing_control = policy;
            let props = CharProps {
                font: Some("Hiragino Kaku Gothic ProN".into()),
                font_ea: Some("Hiragino Kaku Gothic ProN".into()),
                size: Some(18.0),
                spacing: Some(0.0),
                ..Default::default()
            };
            let mut p = Paragraph::with_text(text, props);
            p.props.align = Some(Align::Justify);
            p.props.indent_right = Some(468.0 - 5.94 * 18.0);
            p.props.overflow_punct = Some(false);
            doc.body = vec![para_block(p)];
            lay(&doc)
                .pages
                .iter()
                .flat_map(|page| &page.items)
                .find_map(|item| match item {
                    Placed::Lines { para, l0: 0, story: StoryRef::Body, .. } => Some(para.clone()),
                    _ => None,
                })
                .expect("body paragraph")
        };
        let plain = make(CharacterSpacingControl::DoNotCompress);
        let kana = make(CharacterSpacingControl::CompressPunctuationAndJapaneseKana);
        assert_eq!(plain.lines.iter().map(|l| l.stop).collect::<Vec<_>>(), kana.lines.iter().map(|l| l.stop).collect::<Vec<_>>(), "{text}");
        assert_eq!(plain.clusters.iter().map(|c| c.adv).collect::<Vec<_>>(), kana.clusters.iter().map(|c| c.adv).collect::<Vec<_>>(), "{text}");
    }
}

/// Word's twip-rounded widths put 5.5pt just beyond the measured 1/16em limit.
#[test]
fn kana_compression_does_not_round_up_past_the_limit() {
    if wordcraft_fonts::word::resolve("Hiragino Kaku Gothic ProN", false, false).substituted {
        return;
    }
    let policy = CharacterSpacingControl::CompressPunctuationAndJapaneseKana;
    let beyond = first_paragraph(5.5, 32.65 / 5.5, Align::Justify, policy.clone(), 'あ', false);
    assert!(beyond.lines[0].stop < "漢漢漢漢漢あ".len());
    let exact = first_paragraph(8.0, 5.9375, Align::Justify, policy, 'あ', false);
    assert_eq!(exact.lines[0].stop, "漢漢漢漢漢あ".len());
}
