//! Font size labels in the size boxes (Home › Font, the mini toolbar, the Font dialog).
//!
//! Chinese interfaces name the traditional type sizes (号数): 小四 is 12 pt, 五号 10.5 pt
//! (#5122). The list shows the named sizes, largest first, then the point values, and the box
//! shows the current size's name when it has one. Only the labels change: the value stored and
//! sent to `format.size` is always points, so the document, the command layer, the CLI and MCP
//! stay locale-neutral. The names aren't translations of English strings (no other language has
//! them), so they live in this table rather than in the interface catalogs.

use crate::i18n::Lang;

/// One named Chinese size: Simplified name, Traditional name, points.
struct Named {
    hans: &'static str,
    hant: &'static str,
    pt: f64,
}

/// The named sizes, largest first, as the list shows them.
const NAMED: [Named; 16] = [
    Named { hans: "初号", hant: "初號", pt: 42.0 },
    Named { hans: "小初", hant: "小初", pt: 36.0 },
    Named { hans: "一号", hant: "一號", pt: 26.0 },
    Named { hans: "小一", hant: "小一", pt: 24.0 },
    Named { hans: "二号", hant: "二號", pt: 22.0 },
    Named { hans: "小二", hant: "小二", pt: 18.0 },
    Named { hans: "三号", hant: "三號", pt: 16.0 },
    Named { hans: "小三", hant: "小三", pt: 15.0 },
    Named { hans: "四号", hant: "四號", pt: 14.0 },
    Named { hans: "小四", hant: "小四", pt: 12.0 },
    Named { hans: "五号", hant: "五號", pt: 10.5 },
    Named { hans: "小五", hant: "小五", pt: 9.0 },
    Named { hans: "六号", hant: "六號", pt: 7.5 },
    Named { hans: "小六", hant: "小六", pt: 6.5 },
    Named { hans: "七号", hant: "七號", pt: 5.5 },
    Named { hans: "八号", hant: "八號", pt: 5.0 },
];

/// The point values a Chinese list shows after the names: the usual list plus the small sizes
/// only the Chinese ladder has.
const CHINESE_POINTS: [f64; 4] = [5.0, 5.5, 6.5, 7.5];

/// Which naming an interface uses.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Naming {
    Points,
    Hans,
    Hant,
}

fn naming(lang: Lang) -> Naming {
    match lang.code() {
        "zh-hans" => Naming::Hans,
        "zh-hant" => Naming::Hant,
        _ => Naming::Points,
    }
}

/// Two sizes are the same when they agree to a hundredth of a point (sizes are stored in
/// half-points, so this only absorbs float noise).
fn same(a: f64, b: f64) -> bool {
    (a - b).abs() < 0.005
}

/// A point value as the box shows it: `12`, `10.5`.
fn points(pt: f64) -> String {
    if pt.fract() == 0.0 { format!("{pt:.0}") } else { pt.to_string() }
}

/// The label for `pt` in `lang`'s interface: its Chinese name when it has one there, else the
/// number of points.
pub fn label(pt: f64, lang: Lang) -> String {
    let named = |pick: fn(&Named) -> &'static str| NAMED.iter().find(|n| same(n.pt, pt)).map(|n| pick(n).to_string());
    match naming(lang) {
        Naming::Hans => named(|n| n.hans),
        Naming::Hant => named(|n| n.hant),
        Naming::Points => None,
    }
    .unwrap_or_else(|| points(pt))
}

/// The size list for `lang`'s interface.
pub fn items(lang: Lang) -> Vec<String> {
    let usual = wordcraft_engine::cmd::format::SIZES.iter().map(|&s| f64::from(s));
    let pick: fn(&Named) -> &'static str = match naming(lang) {
        Naming::Points => return usual.map(points).collect(),
        Naming::Hans => |n| n.hans,
        Naming::Hant => |n| n.hant,
    };
    let mut nums: Vec<f64> = CHINESE_POINTS.iter().copied().chain(usual).collect();
    nums.sort_by(f64::total_cmp);
    nums.dedup_by(|a, b| same(*a, *b));
    NAMED.iter().map(|n| pick(n).to_string()).chain(nums.into_iter().map(points)).collect()
}

/// The points a typed or chosen size stands for in `lang`'s interface: a finite, positive number
/// of points, or, in a Chinese interface, a size name in either script (`小四`, `初號`). Other
/// interfaces don't take the names: Japan's own 号 sizes differ (its 一号 is 27.5 pt, not 26).
/// Anything else is `None`.
pub fn parse(text: &str, lang: Lang) -> Option<f64> {
    let t = text.trim();
    if naming(lang) != Naming::Points
        && let Some(n) = NAMED.iter().find(|n| n.hans == t || n.hant == t)
    {
        return Some(n.pt);
    }
    t.parse::<f64>().ok().filter(|pt| pt.is_finite() && *pt > 0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The interface languages, from inline fixtures so the test needn't embed the catalogs.
    fn lang(code: &'static str) -> Lang {
        crate::i18n::fixture_lang(code, "")
    }

    #[test]
    fn chinese_names_label_their_sizes() {
        // The table itself, whatever catalogs this build embeds.
        let hans = |pt: f64| NAMED.iter().find(|n| same(n.pt, pt)).map(|n| n.hans);
        assert_eq!(hans(12.0), Some("小四"));
        assert_eq!(hans(10.5), Some("五号"));
        assert_eq!(hans(42.0), Some("初号"));
        assert_eq!(hans(5.0), Some("八号"));
        assert_eq!(hans(13.0), None);
        // Every size has one name, and the list runs largest first.
        assert!(NAMED.windows(2).all(|w| w[0].pt > w[1].pt));
        assert!(NAMED.iter().all(|n| n.hans.chars().count() == 2 && n.hant.chars().count() == 2));
        assert!(NAMED.iter().all(|n| n.hant.replace('號', "号") == n.hans));
    }

    #[test]
    fn labels_follow_the_interface() {
        let en = lang("en");
        assert_eq!(label(12.0, en), "12");
        assert_eq!(label(10.5, en), "10.5");
        assert_eq!(items(en).first().map(String::as_str), Some("8"));
        assert_eq!(items(en).len(), wordcraft_engine::cmd::format::SIZES.len());
        let hans = lang("zh-hans");
        assert_eq!(label(12.0, hans), "小四");
        assert_eq!(label(10.5, hans), "五号");
        assert_eq!(label(13.0, hans), "13");
        assert_eq!(label(11.0, hans), "11");
        let list = items(hans);
        assert_eq!(list.first().map(String::as_str), Some("初号"));
        assert_eq!(list.get(15).map(String::as_str), Some("八号"));
        // Then the points, smallest first, with the Chinese ladder's small sizes.
        assert_eq!(list.get(16..20), Some(&["5".to_string(), "5.5".into(), "6.5".into(), "7.5".into()][..]));
        assert_eq!(list.last().map(String::as_str), Some("72"));
        assert_eq!(list.len(), 16 + 4 + wordcraft_engine::cmd::format::SIZES.len());
        let hant = lang("zh-hant");
        assert_eq!(label(12.0, hant), "小四");
        assert_eq!(label(42.0, hant), "初號");
        assert_eq!(items(hant).first().map(String::as_str), Some("初號"));
        // Japanese and Korean Word list numbers, as do the other interfaces.
        assert_eq!(label(12.0, lang("ja")), "12");
        assert_eq!(items(lang("ko")), items(en));
    }

    /// Every named size, both ways, in both scripts: the name shows for its points and parses
    /// back to them. The table: Word's Chinese size list (a native speaker should confirm it).
    #[test]
    fn every_named_size_round_trips_in_both_scripts() {
        let expected = [
            ("初号", "初號", 42.0),
            ("小初", "小初", 36.0),
            ("一号", "一號", 26.0),
            ("小一", "小一", 24.0),
            ("二号", "二號", 22.0),
            ("小二", "小二", 18.0),
            ("三号", "三號", 16.0),
            ("小三", "小三", 15.0),
            ("四号", "四號", 14.0),
            ("小四", "小四", 12.0),
            ("五号", "五號", 10.5),
            ("小五", "小五", 9.0),
            ("六号", "六號", 7.5),
            ("小六", "小六", 6.5),
            ("七号", "七號", 5.5),
            ("八号", "八號", 5.0),
        ];
        assert_eq!(expected.len(), NAMED.len());
        let (hans, hant, en) = (lang("zh-hans"), lang("zh-hant"), lang("en"));
        for (s, t, pt) in expected {
            assert_eq!(label(pt, hans), s, "{pt} pt, Simplified");
            assert_eq!(label(pt, hant), t, "{pt} pt, Traditional");
            for l in [hans, hant] {
                assert_eq!(parse(s, l), Some(pt), "{s}");
                assert_eq!(parse(t, l), Some(pt), "{t}");
            }
            // English shows the number and takes only the number.
            assert_eq!(parse(&label(pt, en), en), Some(pt), "{pt} pt, English");
            assert!(label(pt, en).chars().all(|c| c.is_ascii_digit() || c == '.'), "{pt} pt, English: {}", label(pt, en));
            assert_eq!(parse(s, en), None, "{s} in English");
        }
        // A size with no name shows as its number everywhere, and parses back.
        for l in [hans, hant, en] {
            assert_eq!(label(13.5, l), "13.5");
            assert_eq!(parse("13.5", l), Some(13.5));
            assert_eq!(label(11.0, l), "11");
        }
        // English lists numbers only, the usual sizes.
        assert!(items(en).iter().all(|i| i.parse::<f64>().is_ok()), "{:?}", items(en));
    }

    #[test]
    fn typed_sizes_parse_to_points() {
        // A Chinese interface takes the names in either script.
        for code in ["zh-hans", "zh-hant"] {
            let l = lang(code);
            assert_eq!(parse("小四", l), Some(12.0));
            assert_eq!(parse(" 四号 ", l), Some(14.0));
            assert_eq!(parse("初號", l), Some(42.0));
            assert_eq!(parse("五號", l), Some(10.5));
        }
        // Every interface takes numbers; only Chinese ones take the names (Japan's 一号 differs).
        for code in ["en", "ja", "ko", "zh-hans"] {
            let l = lang(code);
            assert_eq!(parse("12", l), Some(12.0));
            assert_eq!(parse("10.5", l), Some(10.5));
            for junk in ["", "abc", "小", "NaN", "inf", "-3", "0"] {
                assert_eq!(parse(junk, l), None, "{code} {junk:?}");
            }
        }
        for code in ["en", "ja", "ko"] {
            assert_eq!(parse("小四", lang(code)), None, "{code}");
            assert_eq!(parse("一号", lang(code)), None, "{code}");
        }
        // Every list entry parses, and its size's label (a name, where the size has one: `5`
        // shows as 八号) parses back to the same size.
        for code in ["en", "zh-hans", "zh-hant"] {
            let l = lang(code);
            for item in items(l) {
                let pt = parse(&item, l).unwrap();
                assert_eq!(parse(&label(pt, l), l), Some(pt), "{code} {item}");
            }
        }
        assert_eq!(label(5.0, lang("zh-hans")), "八号");
    }
}
