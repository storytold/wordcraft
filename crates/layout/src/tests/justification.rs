//! Justification uses legal East Asian boundaries as well as word spaces, independent of
//! the requested font's metrics. Compare against left alignment to isolate inserted room.

use super::*;
use wordcraft_doc::props::{CharProps, LineSpacing};

fn paragraph(text: &str, family: &str, size: f32, width: f32, align: Align, no_expand: bool) -> Arc<ParaLayout> {
    let chr = CharProps { font: Some(family.into()), font_ea: Some(family.into()), size: Some(size), ..Default::default() };
    let mut para = Paragraph::with_text(text, chr);
    para.props = ParaProps {
        align: Some(align),
        snap_to_grid: Some(false),
        overflow_punct: Some(false),
        auto_space_de: Some(false),
        auto_space_dn: Some(false),
        line_spacing: Some(LineSpacing::Exactly(size * 1.5)),
        ..Default::default()
    };
    let mut doc = Document::new();
    doc.settings.do_not_expand_shift_return = no_expand;
    doc.last_section.page_w = width + 40.0;
    doc.last_section.margin_left = 20.0;
    doc.last_section.margin_right = 20.0;
    doc.body = vec![wordcraft_doc::para_block(para)];
    let layout = lay(&doc);
    layout.pages.iter().flat_map(|p| &p.items).find_map(|item| if let Placed::Lines { para, .. } = item { Some(para.clone()) } else { None }).unwrap()
}

fn x_at(para: &ParaLayout, line: usize, byte: usize) -> f32 {
    let row = &para.lines[line];
    let cluster = para.clusters.iter().position(|c| c.start == byte).unwrap();
    row.xs[cluster - row.c0]
}

#[test]
fn wrapped_east_asian_text_reaches_the_margin_and_the_final_line_stays_natural() {
    let text = "東京都教育委員会事務局申請書記入方法".repeat(5);
    for family in ["Hiragino Mincho ProN", "Yu Mincho", "Noto Sans JP"] {
        for size in [9.0, 10.5, 12.0, 24.0] {
            for cells in [7.35, 13.65, 21.4] {
                let width = size * cells;
                let on = paragraph(&text, family, size, width, Align::Justify, false);
                let off = paragraph(&text, family, size, width, Align::Left, false);
                assert!(on.lines.len() > 1, "the fixture wraps: {family}, {size}, {width}");
                assert_eq!(on.lines.len(), off.lines.len(), "justification must not change break positions");
                for (i, (a, b)) in on.lines.iter().zip(&off.lines).enumerate() {
                    assert_eq!((a.c0, a.c1), (b.c0, b.c1));
                    if i + 1 == on.lines.len() {
                        assert_eq!(a.xs, b.xs, "the final line remains at its natural advances");
                    } else {
                        assert_eq!(a.end, LineEnd::Wrap);
                        assert!((a.xs.last().unwrap() - a.right).abs() < 0.01, "CJK line must reach its margin: {family}, {size}, {width}: {a:?}");
                        assert!(a.xs.iter().zip(&b.xs).any(|(x, y)| *x > *y + 0.01), "the line has room to justify");
                    }
                }
            }
        }
    }
}

#[test]
fn mixed_cjk_spaces_and_punctuation_share_room_without_splitting_latin_words() {
    let text = "甲乙　丙丁（戊己）、ABC DEF庚辛\n終";
    for family in ["Hiragino Mincho ProN", "Yu Mincho", "Noto Sans JP"] {
        for size in [10.5, 12.0, 24.0] {
            let on = paragraph(text, family, size, size * 30.0, Align::Justify, false);
            let off = paragraph(text, family, size, size * 30.0, Align::Left, false);
            assert_eq!(on.lines.len(), 2);
            assert_eq!(off.lines.len(), 2);
            let delta = |needle: &str| {
                let byte = text.find(needle).unwrap();
                x_at(&on, 0, byte) - x_at(&off, 0, byte)
            };
            assert!(delta("乙") > delta("甲") + 0.01, "CJK boundaries share room even when the line also has spaces");
            assert!(delta("丙") > delta("　") + 0.01, "the internal ideographic space shares room");
            assert!((delta("戊") - delta("（")).abs() < 0.01, "no extra room after an opening bracket");
            assert!((delta("）") - delta("己")).abs() < 0.01, "no extra room before a closing bracket");
            assert!((delta("、") - delta("）")).abs() < 0.01, "adjacent closing punctuation stays together");
            for word in ["ABC", "DEF"] {
                let first = word.get(..1).unwrap();
                for ch in word.chars() {
                    assert!((delta(&ch.to_string()) - delta(first)).abs() < 0.01, "Latin words retain their internal advances");
                }
            }
            assert!((on.lines[0].xs.last().unwrap() - on.lines[0].right).abs() < 0.01);
            assert_eq!(on.lines[1].xs, off.lines[1].xs);
        }
    }
}

#[test]
fn latin_only_words_and_disabled_manual_break_expansion_keep_their_advances() {
    for text in ["AlphaBeta\nlast", "Alpha Beta Gamma\nlast", "甲乙丙丁\n終"] {
        for size in [10.5, 18.0] {
            let off = paragraph(text, "Source Sans 3", size, size * 24.0, Align::Left, false);
            let disabled = paragraph(text, "Source Sans 3", size, size * 24.0, Align::Justify, true);
            assert_eq!(off.lines.len(), 2);
            assert_eq!(disabled.lines.len(), 2);
            for (a, b) in off.lines.iter().zip(&disabled.lines) {
                assert_eq!(a.xs, b.xs, "doNotExpandShiftReturn keeps both lines natural");
            }
            if text.starts_with("AlphaBeta") {
                let on = paragraph(text, "Source Sans 3", size, size * 24.0, Align::Justify, false);
                assert_eq!(on.lines[0].xs, off.lines[0].xs, "a Latin word gains no character spacing under Justify");
            }
            if text.starts_with("Alpha Beta") {
                let on = paragraph(text, "Source Sans 3", size, size * 24.0, Align::Justify, false);
                assert!((on.lines[0].xs.last().unwrap() - on.lines[0].right).abs() < 0.01, "Latin spaces still justify");
                let gain = |byte| x_at(&on, 0, byte) - x_at(&off, 0, byte);
                assert!((gain(0) - gain(4)).abs() < 0.01, "letters inside Alpha do not separate");
                assert!(gain(6) > gain(4), "the interword space expands");
            }
        }
    }
}
