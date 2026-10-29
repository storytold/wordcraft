//! Our supplement of Russian words older Hunspell dictionaries lack (`маркетплейс`, `промокод`,
//! `интеграция`, `бонусный`…), from `assets/spelling/ru-extra.txt`: each base form with its
//! declension, expanded here into every form.

use std::collections::HashSet;
use std::sync::OnceLock;

const EXTRA_RU: &str = include_str!("../../../../assets/spelling/ru-extra.txt");

/// Every form of the supplement: common words (lower case) and names (lower case too, but they
/// only match a word written with a capital).
struct Extra {
    words: HashSet<String>,
    names: HashSet<String>,
}

fn extra() -> &'static Extra {
    static E: OnceLock<Extra> = OnceLock::new();
    E.get_or_init(|| {
        let mut e = Extra { words: HashSet::new(), names: HashSet::new() };
        for line in EXTRA_RU.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')) {
            let (base, class) = line.split_once('\t').map_or((line, ""), |(b, c)| (b.trim(), c.trim()));
            let name = base.chars().next().is_some_and(char::is_uppercase);
            let set = if name { &mut e.names } else { &mut e.words };
            for f in forms(base, class) {
                set.insert(f.to_lowercase());
            }
        }
        e
    })
}

/// Is `w` a form of a supplement word (a name only when written with a capital)?
pub(super) fn contains(w: &str) -> bool {
    let e = extra();
    let lower = w.to_lowercase();
    e.words.contains(&lower) || (w.chars().next().is_some_and(char::is_uppercase) && e.names.contains(&lower))
}

fn is_velar(c: char) -> bool {
    matches!(c, 'г' | 'к' | 'х')
}

fn is_hushing(c: char) -> bool {
    matches!(c, 'ж' | 'ш' | 'ч' | 'щ')
}

/// The forms of `base` declined as `class` (see the supplement file's header).
fn forms(base: &str, class: &str) -> Vec<String> {
    let with = |stem: &str, ends: &[&str]| ends.iter().map(|e| format!("{stem}{e}")).collect::<Vec<_>>();
    let last = |s: &str| s.chars().last().unwrap_or(' ');
    if let Some(list) = class.strip_prefix('=') {
        let mut v: Vec<String> = list.split_whitespace().map(str::to_string).collect();
        v.push(base.to_string());
        return v;
    }
    match class {
        "m" => {
            let c = last(base);
            let plural = if is_velar(c) || is_hushing(c) { "и" } else { "ы" };
            let gen_pl = if is_hushing(c) { "ей" } else { "ов" };
            let mut v = with(base, &["", "а", "у", "ом", "е", plural, gen_pl, "ам", "ами", "ах"]);
            if is_hushing(c) || c == 'ц' {
                v.push(format!("{base}ем"));
            }
            v
        }
        "f" => {
            let Some(stem) = base.strip_suffix('а') else { return vec![base.to_string()] };
            let c = last(stem);
            let plural = if is_velar(c) || is_hushing(c) { "и" } else { "ы" };
            let mut v = with(stem, &["а", plural, "е", "у", "ой", "ою", "", "ам", "ами", "ах"]);
            if is_hushing(c) || c == 'ц' {
                v.extend(with(stem, &["ей", "ею"]));
            }
            v
        }
        "fia" => {
            base.strip_suffix("ия").map_or_else(|| vec![base.to_string()], |s| with(s, &["ия", "ии", "ию", "ией", "иею", "ий", "иям", "иями", "иях"]))
        }
        "fs" => base.strip_suffix('ь').map_or_else(|| vec![base.to_string()], |s| with(s, &["ь", "и", "ью", "ей", "ям", "ями", "ях"])),
        "no" => base.strip_suffix('о').map_or_else(|| vec![base.to_string()], |s| with(s, &["о", "а", "у", "ом", "е", "", "ам", "ами", "ах"])),
        "ovat" => {
            // Verbs in -овать (`колеровать`, `администрировать`): the conjugation, the past, the
            // imperative and the participles.
            let Some(stem) = base.strip_suffix("овать") else { return vec![base.to_string()] };
            let mut v = with(
                stem,
                &[
                    "овать",
                    "ую",
                    "уешь",
                    "ует",
                    "уем",
                    "уете",
                    "уют",
                    "овал",
                    "овала",
                    "овало",
                    "овали",
                    "уя",
                    "уй",
                    "уйте",
                    "уется",
                    "уются",
                    "уюсь",
                    "уешься",
                    "уемся",
                    "уетесь",
                    "овался",
                    "овалась",
                    "овалось",
                    "овались",
                    "оваться",
                    "овав",
                    "овавшись",
                ],
            );
            for participle in ["уемый", "ованный", "ующий", "овавший", "ующийся", "овавшийся"] {
                let adj = format!("{stem}{participle}");
                if let Some(reflexive) = adj.strip_suffix("ся") {
                    v.extend(forms(reflexive, "adj").into_iter().map(|f| format!("{f}ся")));
                } else {
                    v.extend(forms(&adj, "adj"));
                }
            }
            v.push(format!("{stem}ован"));
            v.extend(with(stem, &["ована", "овано", "ованы"]));
            v
        }
        "nie" => {
            base.strip_suffix("ие").map_or_else(|| vec![base.to_string()], |s| with(s, &["ие", "ия", "ию", "ием", "ии", "ий", "иям", "иями", "иях"]))
        }
        "adj" => {
            if let Some(stem) = base.strip_suffix("ый").or_else(|| base.strip_suffix("ой")) {
                let mut v = with(stem, &["ого", "ому", "ым", "ом", "ая", "ой", "ую", "ое", "ые", "ых", "ыми"]);
                v.push(base.to_string());
                return v;
            }
            let Some(stem) = base.strip_suffix("ий") else { return vec![base.to_string()] };
            let c = last(stem);
            let ends: &[&str] = if is_velar(c) {
                &["ий", "ого", "ому", "им", "ом", "ая", "ой", "ую", "ое", "ие", "их", "ими"]
            } else if is_hushing(c) {
                &["ий", "его", "ему", "им", "ем", "ая", "ей", "ую", "ее", "ие", "их", "ими"]
            } else {
                &["ий", "его", "ему", "им", "ем", "яя", "ей", "юю", "ее", "ие", "их", "ими"]
            };
            with(stem, ends)
        }
        _ => vec![base.to_string()],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supplement_words_decline() {
        for w in [
            "маркетплейс",
            "маркетплейсов",
            "маркетплейсах",
            "Маркетплейсами",
            "чекбоксы",
            "промокода",
            "кешбэки",
            "инфографики",
            "оплат",
            "отгрузок",
            "интеграций",
            "соцсетей",
            "согласий",
            "бонусных",
            "газоиспользующего",
            "колеруется",
            "заколерованный",
            "онлайн",
            "чат-ботом",
            "Яндекса",
            "Битриксе",
        ] {
            assert!(contains(w), "{w}");
        }
        // Names need their capital; nonsense isn't in.
        assert!(!contains("яндекс"));
        assert!(!contains("маркетплейсыы"));
    }

    #[test]
    fn declension_classes() {
        assert!(forms("бренд", "m").contains(&"брендов".to_string()));
        assert!(forms("блог", "m").contains(&"блоги".to_string()));
        assert!(forms("мерч", "m").contains(&"мерчей".to_string()));
        assert!(forms("книга", "f").contains(&"книги".to_string()));
        assert!(forms("синий", "adj").contains(&"синяя".to_string()));
        assert!(forms("русский", "adj").contains(&"русская".to_string()));
        assert!(forms("свежий", "adj").contains(&"свежая".to_string()));
        assert_eq!(forms("медиа", ""), ["медиа"]);
        let ovat = forms("модерировать", "ovat");
        for f in [
            "модерирую",
            "модерирует",
            "модерировали",
            "модерируется",
            "модерируемого",
            "модерированной",
            "модерирующая",
            "модерирующемуся",
            "модерирован",
        ] {
            assert!(ovat.contains(&f.to_string()), "{f}");
        }
        assert!(forms("юрлицо", "no").contains(&"юрлиц".to_string()));
    }
}
