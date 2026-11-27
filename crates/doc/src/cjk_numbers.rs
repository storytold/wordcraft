//! East Asian list and page number formats (ECMA-376 §17.18.59 ST_NumberFormat): Chinese,
//! Japanese and Korean counting systems (with units 十/百/千 and myriads 万/億), legal (financial)
//! numerals, digit-by-digit ideographs, the heavenly stems and earthly branches, kana and hangul
//! sequences and enclosed numbers. Written from the number systems' rules; a value outside a
//! sequence's range falls back to plain decimal.

/// Whether a counting system writes the "one" before a unit.
#[derive(Clone, Copy, PartialEq, Eq)]
enum OmitOne {
    /// Always written: legal numerals (壹拾, 壱百).
    Never,
    /// Dropped only before a leading ten: 十, 十一, 十万 but 一百一十 (Chinese).
    LeadingTen,
    /// Dropped before 十, 百 and 千 inside every group of four: 十, 百, 千 but 一万 (Japanese,
    /// Korean).
    Units,
}

/// A myriad-based counting system (groups of four digits, like 1|2345|6789).
struct Counting {
    /// The digits 1 to 9 (index 0 unused).
    digits: [char; 10],
    /// The units ten, hundred, thousand.
    units: [char; 3],
    /// The myriads: ten thousand, hundred million.
    myriads: [char; 2],
    /// The character written once for a run of zeros between two digits (Chinese 零); `None`
    /// skips zeros (Japanese, Korean).
    zero: Option<char>,
    omit_one: OmitOne,
}

impl Counting {
    fn format(&self, n: u32) -> String {
        if n == 0 {
            return self.zero.unwrap_or('〇').to_string();
        }
        let groups = [n / 100_000_000, (n / 10_000) % 10_000, n % 10_000];
        let mut out = String::new();
        let mut zero_pending = false;
        for (gi, &g) in groups.iter().enumerate() {
            if g == 0 {
                zero_pending |= !out.is_empty();
                continue;
            }
            let ds = [g / 1000, (g / 100) % 10, (g / 10) % 10, g % 10];
            for (pos, &d) in ds.iter().enumerate() {
                if d == 0 {
                    zero_pending |= !out.is_empty();
                    continue;
                }
                if zero_pending && let Some(z) = self.zero {
                    out.push(z);
                }
                zero_pending = false;
                // pos 0 = thousands, 1 = hundreds, 2 = tens, 3 = ones.
                let unit = match pos {
                    0 => self.units.get(2),
                    1 => self.units.get(1),
                    2 => self.units.first(),
                    _ => None,
                };
                let omit = d == 1
                    && unit.is_some()
                    && match self.omit_one {
                        OmitOne::Never => false,
                        OmitOne::LeadingTen => pos == 2 && out.is_empty(),
                        OmitOne::Units => true,
                    };
                if !omit {
                    out.push(self.digits.get(d as usize).copied().unwrap_or('?'));
                }
                if let Some(&u) = unit {
                    out.push(u);
                }
            }
            // Trailing zeros of a group are not a gap: the next group starts a new place.
            zero_pending = false;
            match gi {
                0 => out.extend(self.myriads.get(1)),
                1 => out.extend(self.myriads.first()),
                _ => {}
            }
        }
        out
    }
}

const CN_DIGITS: [char; 10] = ['〇', '一', '二', '三', '四', '五', '六', '七', '八', '九'];

const CHINESE: Counting =
    Counting { digits: CN_DIGITS, units: ['十', '百', '千'], myriads: ['万', '亿'], zero: Some('零'), omit_one: OmitOne::LeadingTen };
const TAIWANESE: Counting =
    Counting { digits: CN_DIGITS, units: ['十', '百', '千'], myriads: ['萬', '億'], zero: Some('零'), omit_one: OmitOne::LeadingTen };
const CHINESE_LEGAL: Counting = Counting {
    digits: ['零', '壹', '贰', '叁', '肆', '伍', '陆', '柒', '捌', '玖'],
    units: ['拾', '佰', '仟'],
    myriads: ['万', '亿'],
    zero: Some('零'),
    omit_one: OmitOne::Never,
};
const TRADITIONAL_LEGAL: Counting = Counting {
    digits: ['零', '壹', '貳', '參', '肆', '伍', '陸', '柒', '捌', '玖'],
    units: ['拾', '佰', '仟'],
    myriads: ['萬', '億'],
    zero: Some('零'),
    omit_one: OmitOne::Never,
};
const JAPANESE: Counting =
    Counting { digits: CN_DIGITS, units: ['十', '百', '千'], myriads: ['万', '億'], zero: None, omit_one: OmitOne::Units };
const JAPANESE_LEGAL: Counting = Counting {
    digits: ['〇', '壱', '弐', '参', '四', '伍', '六', '七', '八', '九'],
    units: ['拾', '百', '阡'],
    myriads: ['萬', '億'],
    zero: None,
    omit_one: OmitOne::Never,
};
const KOREAN: Counting = Counting {
    digits: ['영', '일', '이', '삼', '사', '오', '육', '칠', '팔', '구'],
    units: ['십', '백', '천'],
    myriads: ['만', '억'],
    zero: None,
    omit_one: OmitOne::Units,
};

/// 一, 二 … 十, 十一 … 九十九 (Chinese counting); from 100 on, digit by digit (一〇〇).
pub(crate) fn chinese_counting(n: u32) -> String {
    if n < 100 { CHINESE.format(n) } else { digits(n, &CN_DIGITS) }
}

/// Chinese counting with every unit: 一百零一, 一千, 一万.
pub(crate) fn chinese_counting_thousand(n: u32) -> String {
    CHINESE.format(n)
}

/// Simplified Chinese legal (financial) numerals: 壹, 贰, 叁 … 壹拾.
pub(crate) fn chinese_legal_simplified(n: u32) -> String {
    CHINESE_LEGAL.format(n)
}

/// Traditional Chinese legal numerals: 壹, 貳, 參 … 壹拾.
pub(crate) fn ideograph_legal_traditional(n: u32) -> String {
    TRADITIONAL_LEGAL.format(n)
}

/// Taiwanese counting: like Chinese counting below 100, digit by digit above.
pub(crate) fn taiwanese_counting(n: u32) -> String {
    if n < 100 { TAIWANESE.format(n) } else { digits(n, &CN_DIGITS) }
}

/// Taiwanese counting with every unit: 一百零一, 一千, 一萬.
pub(crate) fn taiwanese_counting_thousand(n: u32) -> String {
    TAIWANESE.format(n)
}

/// Japanese counting: 一 … 十, 十一, 百, 千, 一万.
pub(crate) fn japanese_counting(n: u32) -> String {
    JAPANESE.format(n)
}

/// Japanese legal numerals: 壱, 弐, 参 … 壱拾.
pub(crate) fn japanese_legal(n: u32) -> String {
    JAPANESE_LEGAL.format(n)
}

/// Japanese digits with 万/億 marking each group of four: 一二, 一万二三四五.
pub(crate) fn japanese_digital_ten_thousand(n: u32) -> String {
    let groups = [(n / 100_000_000, Some('億')), ((n / 10_000) % 10_000, Some('万')), (n % 10_000, None)];
    let mut out = String::new();
    for (g, mark) in groups {
        let s = digits(g, &CN_DIGITS);
        if out.is_empty() {
            if g == 0 && mark.is_some() {
                continue;
            }
        } else {
            // Inner groups keep their four places: 一万〇〇〇一.
            out.extend(std::iter::repeat_n('〇', 4usize.saturating_sub(s.chars().count())));
        }
        out.push_str(&s);
        out.extend(mark);
    }
    out
}

/// Sino-Korean numbers in hangul: 일, 이, 삼 … 십, 십일, 백.
pub(crate) fn korean_counting(n: u32) -> String {
    KOREAN.format(n)
}

/// Sino-Korean digits in hangul, one per digit: 일, 일영, 일일.
pub(crate) fn korean_digital(n: u32) -> String {
    digits(n, &KOREAN.digits)
}

const KO_ONES: [&str; 10] = ["", "하나", "둘", "셋", "넷", "다섯", "여섯", "일곱", "여덟", "아홉"];
const KO_TENS: [&str; 10] = ["", "열", "스물", "서른", "마흔", "쉰", "예순", "일흔", "여든", "아흔"];

/// Native Korean numbers: 하나, 둘, 셋 … 열, 열하나 … 아흔아홉 (decimal from 100 on).
pub(crate) fn korean_legal(n: u32) -> String {
    if n == 0 || n > 99 {
        return n.to_string();
    }
    let (t, o) = ((n / 10) as usize, (n % 10) as usize);
    format!("{}{}", KO_TENS.get(t).copied().unwrap_or(""), KO_ONES.get(o).copied().unwrap_or(""))
}

/// One ideograph per decimal digit: 一, 一〇, 一〇一.
pub(crate) fn ideograph_digital(n: u32) -> String {
    digits(n, &CN_DIGITS)
}

fn digits(n: u32, set: &[char; 10]) -> String {
    n.to_string().bytes().map(|b| set.get(b.saturating_sub(b'0') as usize).copied().unwrap_or('?')).collect()
}

const STEMS: [char; 10] = ['甲', '乙', '丙', '丁', '戊', '己', '庚', '辛', '壬', '癸'];
const BRANCHES: [char; 12] = ['子', '丑', '寅', '卯', '辰', '巳', '午', '未', '申', '酉', '戌', '亥'];

/// The ten heavenly stems: 甲, 乙, 丙 … 癸.
pub(crate) fn ideograph_traditional(n: u32) -> String {
    nth(&STEMS, n)
}

/// The twelve earthly branches: 子, 丑, 寅 … 亥.
pub(crate) fn ideograph_zodiac(n: u32) -> String {
    nth(&BRANCHES, n)
}

/// The sixty-year cycle: 甲子, 乙丑, 丙寅 … 癸亥.
pub(crate) fn ideograph_zodiac_traditional(n: u32) -> String {
    if !(1..=60).contains(&n) {
        return n.to_string();
    }
    let i = (n - 1) as usize;
    [STEMS.get(i % 10), BRANCHES.get(i % 12)].into_iter().flatten().collect()
}

/// The `n`th (1-based) item of a sequence, or decimal past its end.
fn nth(set: &[char], n: u32) -> String {
    n.checked_sub(1).and_then(|i| set.get(i as usize)).map(ToString::to_string).unwrap_or_else(|| n.to_string())
}

/// Code points `first..` enclosing 1 to `count`, decimal beyond.
fn enclosed(n: u32, first: u32, count: u32) -> String {
    if (1..=count).contains(&n)
        && let Some(c) = char::from_u32(first + n - 1)
    {
        return c.to_string();
    }
    n.to_string()
}

/// ① ② ③ … ⑳.
pub(crate) fn decimal_enclosed_circle(n: u32) -> String {
    enclosed(n, 0x2460, 20)
}

/// ① … ⑩ (the Chinese set stops at ten).
pub(crate) fn decimal_enclosed_circle_chinese(n: u32) -> String {
    enclosed(n, 0x2460, 10)
}

/// ⒈ ⒉ ⒊ … ⒛.
pub(crate) fn decimal_enclosed_fullstop(n: u32) -> String {
    enclosed(n, 0x2488, 20)
}

/// ⑴ ⑵ ⑶ … ⒇.
pub(crate) fn decimal_enclosed_paren(n: u32) -> String {
    enclosed(n, 0x2474, 20)
}

/// ㊀ ㊁ ㊂ … ㊉ (circled ideographs).
pub(crate) fn ideograph_enclosed_circle(n: u32) -> String {
    enclosed(n, 0x3280, 10)
}

/// Full-width digits: １, ２ … １０.
pub(crate) fn decimal_full_width(n: u32) -> String {
    n.to_string().chars().map(|c| c.to_digit(10).and_then(|d| char::from_u32(0xFF10 + d)).unwrap_or(c)).collect()
}

const AIUEO: &str = "アイウエオカキクケコサシスセソタチツテトナニヌネノハヒフヘホマミムメモヤユヨラリルレロワヲン";
const AIUEO_HALF: &str = "ｱｲｳｴｵｶｷｸｹｺｻｼｽｾｿﾀﾁﾂﾃﾄﾅﾆﾇﾈﾉﾊﾋﾌﾍﾎﾏﾐﾑﾒﾓﾔﾕﾖﾗﾘﾙﾚﾛﾜｦﾝ";
const IROHA: &str = "イロハニホヘトチリヌルヲワカヨタレソツネナラムウヰノオクヤマケフコエテアサキユメミシヱヒモセス";
const IROHA_HALF: &str = "ｲﾛﾊﾆﾎﾍﾄﾁﾘﾇﾙｦﾜｶﾖﾀﾚｿﾂﾈﾅﾗﾑｳヰﾉｵｸﾔﾏｹﾌｺｴﾃｱｻｷﾕﾒﾐｼヱﾋﾓｾｽ";
const GANADA: &str = "가나다라마바사아자차카타파하";
const CHOSUNG: &str = "ㄱㄴㄷㄹㅁㅂㅅㅇㅈㅊㅋㅌㅍㅎ";

fn nth_of(set: &str, n: u32) -> String {
    n.checked_sub(1).and_then(|i| set.chars().nth(i as usize)).map(|c| c.to_string()).unwrap_or_else(|| n.to_string())
}

/// Half-width katakana in gojūon order: ｱ, ｲ, ｳ …
pub(crate) fn aiueo(n: u32) -> String {
    nth_of(AIUEO_HALF, n)
}

/// Full-width katakana in gojūon order: ア, イ, ウ …
pub(crate) fn aiueo_full_width(n: u32) -> String {
    nth_of(AIUEO, n)
}

/// Half-width katakana in iroha order: ｲ, ﾛ, ﾊ …
pub(crate) fn iroha(n: u32) -> String {
    nth_of(IROHA_HALF, n)
}

/// Full-width katakana in iroha order: イ, ロ, ハ …
pub(crate) fn iroha_full_width(n: u32) -> String {
    nth_of(IROHA, n)
}

/// Hangul syllables: 가, 나, 다 …
pub(crate) fn ganada(n: u32) -> String {
    nth_of(GANADA, n)
}

/// Hangul initial consonants: ㄱ, ㄴ, ㄷ …
pub(crate) fn chosung(n: u32) -> String {
    nth_of(CHOSUNG, n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chinese_no_zero_after_group_trailing_zeros() {
        assert_eq!(chinese_counting_thousand(201_000), "二十万一千");
        assert_eq!(chinese_counting_thousand(120_000), "十二万");
        assert_eq!(chinese_counting_thousand(100_100), "十万零一百");
        assert_eq!(taiwanese_counting_thousand(201_000), "二十萬一千");
        assert_eq!(chinese_counting_thousand(100_000_001), "一亿零一");
        assert_eq!(chinese_counting_thousand(100_010_000), "一亿零一万");
        assert_eq!(chinese_counting_thousand(120_000_001), "一亿二千万零一");
    }

    #[test]
    fn chinese_counting_systems() {
        let c = |n| chinese_counting(n);
        assert_eq!([c(1), c(2), c(3), c(10), c(11), c(20), c(21), c(99)], ["一", "二", "三", "十", "十一", "二十", "二十一", "九十九"]);
        assert_eq!([c(100), c(101), c(1000)], ["一〇〇", "一〇一", "一〇〇〇"]);
        let t = |n| chinese_counting_thousand(n);
        assert_eq!(
            [t(10), t(11), t(20), t(101), t(110), t(111), t(1000), t(1010)],
            ["十", "十一", "二十", "一百零一", "一百一十", "一百一十一", "一千", "一千零一十"]
        );
        assert_eq!([t(10_000), t(10_001), t(100_100), t(1_000_000), t(100_000_001)], ["一万", "一万零一", "十万零一百", "一百万", "一亿零一"]);
        assert_eq!(t(u32::MAX), "四十二亿九千四百九十六万七千二百九十五");
        let l = |n| chinese_legal_simplified(n);
        assert_eq!([l(1), l(3), l(10), l(11), l(101), l(1000)], ["壹", "叁", "壹拾", "壹拾壹", "壹佰零壹", "壹仟"]);
        assert_eq!([taiwanese_counting_thousand(10_001), ideograph_legal_traditional(20)], ["一萬零一", "貳拾"]);
        assert_eq!([ideograph_digital(10), ideograph_digital(101)], ["一〇", "一〇一"]);
    }

    #[test]
    fn japanese_and_korean() {
        let j = |n| japanese_counting(n);
        assert_eq!([j(10), j(11), j(20), j(101), j(1000), j(10_000), j(10_001)], ["十", "十一", "二十", "百一", "千", "一万", "一万一"]);
        assert_eq!([japanese_legal(10), japanese_legal(1000)], ["壱拾", "壱阡"]);
        let d = |n| japanese_digital_ten_thousand(n);
        assert_eq!([d(12), d(10_000), d(12_345), d(100_000_000)], ["一二", "一万〇〇〇〇", "一万二三四五", "一億〇〇〇〇万〇〇〇〇"]);
        assert_eq!([korean_counting(11), korean_counting(20), korean_counting(101)], ["십일", "이십", "백일"]);
        assert_eq!(
            [korean_digital(10), korean_legal(1), korean_legal(11), korean_legal(20), korean_legal(100)],
            ["일영", "하나", "열하나", "스물", "100"]
        );
    }

    #[test]
    fn sequences_and_enclosed_numbers() {
        assert_eq!([ideograph_traditional(1), ideograph_traditional(10), ideograph_traditional(11)], ["甲", "癸", "11"]);
        assert_eq!([ideograph_zodiac(1), ideograph_zodiac(12), ideograph_zodiac(13)], ["子", "亥", "13"]);
        assert_eq!([ideograph_zodiac_traditional(1), ideograph_zodiac_traditional(11), ideograph_zodiac_traditional(60)], ["甲子", "甲戌", "癸亥"]);
        assert_eq!([decimal_enclosed_circle(1), decimal_enclosed_circle(20), decimal_enclosed_circle(21)], ["①", "⑳", "21"]);
        assert_eq!([decimal_enclosed_circle_chinese(10), decimal_enclosed_circle_chinese(11)], ["⑩", "11"]);
        assert_eq!([decimal_enclosed_fullstop(1), decimal_enclosed_paren(20), ideograph_enclosed_circle(1)], ["⒈", "⒇", "㊀"]);
        assert_eq!(decimal_full_width(10), "１０");
        assert_eq!([aiueo_full_width(1), aiueo_full_width(46), aiueo_full_width(47), aiueo(2)], ["ア", "ン", "47", "ｲ"]);
        assert_eq!([iroha_full_width(1), iroha_full_width(47), iroha(3)], ["イ", "ス", "ﾊ"]);
        assert_eq!([ganada(1), chosung(14), ganada(0)], ["가", "ㅎ", "0"]);
        assert_eq!([AIUEO.chars().count(), AIUEO_HALF.chars().count(), IROHA.chars().count(), IROHA_HALF.chars().count()], [46, 46, 47, 47]);
    }
}
