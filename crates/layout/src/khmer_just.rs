//! How the spare room of a wrapped line is shared out by Thai Distributed and Khmer Justify.
//!
//! Neither script puts spaces between words, so Justify has nothing to stretch. The gaps between
//! clusters are classified by `wordcraft_doc::khmer::Gap`; this module only decides how much
//! each gap takes. It is pure: the paragraph layout supplies the gaps and applies the result.

use wordcraft_doc::khmer::Gap;

/// How to share the room.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Word's Thai Distributed: every gap that may grow gets the same.
    Even,
    /// Khmer Justify: phrase spaces first, then word boundaries, then syllables, each up to a cap.
    Tiered,
}

/// The most a gap of this kind takes, in ems, before the next kind gets a share. A phrase space
/// is where Khmer typesetting opens a line up; syllable gaps are kept tight so a word stays
/// readable as one word.
fn cap_em(g: Gap) -> f32 {
    match g {
        Gap::Space => 0.5,
        Gap::Word => 0.2,
        Gap::Syllable => 0.06,
        Gap::None => 0.0,
    }
}

/// The extra width for each gap (`(kind, font size)`, in order), summing to `slack` when any gap
/// may grow, and all zero otherwise. Never negative.
pub fn distribute(mode: Mode, slack: f32, gaps: &[(Gap, f32)]) -> Vec<f32> {
    let mut out = vec![0.0f32; gaps.len()];
    if slack <= 0.0 || !slack.is_finite() {
        return out;
    }
    let live = gaps.iter().filter(|g| g.0 != Gap::None).count();
    if live == 0 {
        return out;
    }
    let mut left = slack;
    if mode == Mode::Tiered {
        for tier in [Gap::Space, Gap::Word, Gap::Syllable] {
            let idx: Vec<usize> = gaps.iter().enumerate().filter(|(_, g)| g.0 == tier).map(|(i, _)| i).collect();
            if idx.is_empty() || left <= 0.0 {
                continue;
            }
            // Water-filling: gaps of different font sizes have different caps.
            let mut open = idx;
            while !open.is_empty() && left > 1e-4 {
                let share = left / open.len() as f32;
                let mut spent = 0.0;
                open.retain(|&i| {
                    let room = (gaps.get(i).map_or(0.0, |g| cap_em(g.0) * g.1) - out.get(i).copied().unwrap_or(0.0)).max(0.0);
                    let add = share.min(room);
                    if let Some(o) = out.get_mut(i) {
                        *o += add;
                    }
                    spent += add;
                    room > share + 1e-4
                });
                left -= spent;
                if spent <= 1e-6 {
                    break;
                }
            }
        }
    }
    if left > 1e-4 {
        // Whatever the caps held back, or all of it for Even: the same on every gap.
        let per = left / live as f32;
        for (o, g) in out.iter_mut().zip(gaps) {
            if g.0 != Gap::None {
                *o += per;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sum(v: &[f32]) -> f32 {
        v.iter().sum()
    }

    #[test]
    fn the_room_is_always_used_up() {
        let gaps = [(Gap::Syllable, 12.0), (Gap::Space, 12.0), (Gap::Syllable, 12.0), (Gap::None, 12.0), (Gap::Word, 12.0)];
        for mode in [Mode::Even, Mode::Tiered] {
            for slack in [0.5, 3.0, 40.0, 400.0] {
                let d = distribute(mode, slack, &gaps);
                assert!((sum(&d) - slack).abs() < 1e-3, "{mode:?} {slack}: {d:?}");
                assert_eq!(d[3], 0.0, "a gap inside a syllable never grows");
                assert!(d.iter().all(|x| *x >= 0.0));
            }
        }
    }

    #[test]
    fn khmer_justify_prefers_spaces_then_words_then_syllables() {
        let gaps = [(Gap::Syllable, 10.0), (Gap::Space, 10.0), (Gap::Word, 10.0), (Gap::Syllable, 10.0)];
        // A little room goes to the space alone.
        assert_eq!(distribute(Mode::Tiered, 3.0, &gaps), vec![0.0, 3.0, 0.0, 0.0]);
        // Past the space's cap (0.5 em = 5), words take theirs (0.2 em = 2).
        let d = distribute(Mode::Tiered, 6.0, &gaps);
        assert_eq!(d, vec![0.0, 5.0, 1.0, 0.0]);
        // Then the syllables (0.06 em = 0.6 each).
        let d = distribute(Mode::Tiered, 8.2, &gaps);
        assert!((d[0] - 0.6).abs() < 1e-4 && (d[3] - 0.6).abs() < 1e-4 && (d[1] - 5.0).abs() < 1e-4 && (d[2] - 2.0).abs() < 1e-4, "{d:?}");
    }

    #[test]
    fn thai_distributed_is_even() {
        let gaps = [(Gap::Syllable, 12.0), (Gap::None, 12.0), (Gap::Space, 12.0), (Gap::Syllable, 12.0)];
        assert_eq!(distribute(Mode::Even, 6.0, &gaps), vec![2.0, 0.0, 2.0, 2.0]);
    }

    #[test]
    fn nothing_to_stretch_or_nothing_to_give() {
        assert_eq!(distribute(Mode::Tiered, 5.0, &[(Gap::None, 12.0)]), vec![0.0]);
        assert_eq!(distribute(Mode::Tiered, 5.0, &[]), Vec::<f32>::new());
        assert_eq!(distribute(Mode::Tiered, -1.0, &[(Gap::Space, 12.0)]), vec![0.0]);
        assert_eq!(distribute(Mode::Tiered, f32::NAN, &[(Gap::Space, 12.0)]), vec![0.0]);
        assert_eq!(distribute(Mode::Even, f32::INFINITY, &[(Gap::Space, 12.0)]), vec![0.0]);
    }

    #[test]
    fn caps_follow_each_gaps_own_font_size() {
        // A big-type space holds more than a small-type one before the overflow shares the rest.
        let gaps = [(Gap::Space, 20.0), (Gap::Space, 10.0)];
        let d = distribute(Mode::Tiered, 12.0, &gaps);
        assert!((d[0] - 7.0).abs() < 1e-4 && (d[1] - 5.0).abs() < 1e-4, "{d:?}");
    }
}
