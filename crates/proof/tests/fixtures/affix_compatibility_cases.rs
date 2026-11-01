// Original, deliberately small fixtures for behavior supported before French proofing.
// These are not extracts from installed dictionaries and do not claim full language coverage.
pub struct Case {
    pub name: &'static str,
    pub aff: String,
    pub dic: String,
    pub probes: Vec<&'static str>,
}

pub fn cases() -> Vec<Case> {
    let mut out = vec![
        Case {
            name: "pl: conditional strip, prefix cross products, diacritics",
            aff: "SET UTF-8\nTRY aeoiułkr\nPFX b Y 1\nPFX b 0 nie .\nSFX a Y 3\nSFX a 0 y [^y]\nSFX a ek ka ek\nSFX a 0 em .\nSFX c N 1\nSFX c a ą a\nMAP 2\nMAP aą\nMAP oóu\nREP 2\nREP ż rz\nREP u ó\n".into(),
            dic: "4\nkot/ab\npiesek/a\nkrowa/c\nWarszawa/c\n".into(),
            probes: vec!["kot", "koty", "kotem", "niekoty", "pieska", "pieseka", "niepieska", "krową", "krowy", "Warszawą", "warszawą", "kotą", "nie", ""],
        },
        Case {
            name: "pt: UTF-8 flags, forbidden words, two suffixes",
            aff: "SET UTF-8\nFLAG UTF-8\nFORBIDDENWORD ý\nTRY aeiouçãõ\nSFX A Y 1\nSFX A o a/B o\nSFX B Y 1\nSFX B 0 s .\nPFX P Y 1\nPFX P 0 re .\n".into(),
            dic: "4\nbonito/AB\ncasa/B\nfazer/P\nimundo/ýB\n".into(),
            probes: vec!["bonito", "bonita", "bonitos", "bonitas", "bonitass", "casa", "casas", "casaa", "refazer", "recasas", "imundo", "imundos", "Imundo", "IMUNDO"],
        },
        Case {
            name: "it: stripping, non-crossing suffix, proper-name casing",
            aff: "SET UTF-8\nSFX A Y 1\nSFX A o i o\nSFX B N 1\nSFX B a e a\nPFX P Y 1\nPFX P 0 ri .\n".into(),
            dic: "4\nlibro/A\ncasa/BP\nfare/P\nRoma\n".into(),
            probes: vec!["libri", "libros", "case", "ricas a", "ricasa", "ricase", "rifare", "r ilibri", "Roma", "roma", "ROMA", ""],
        },
        Case {
            name: "cs: negated conditions and shared Unicode stems",
            aff: "SET UTF-8\nPFX N Y 1\nPFX N 0 ne .\nSFX S Y 1\nSFX S 0 y [^y]\nSFX E N 1\nSFX E a ě a\n".into(),
            dic: "4\nstrom/NS\nžena/E\nmladý/N\ny/S\n".into(),
            probes: vec!["stromy", "nestromy", "nestrom", "ženě", "ženy", "neženě", "nemladý", "mladýy", "yy", "Ženě", "STROMY"],
        },
        Case {
            name: "ru: Cyrillic flags and suggestions exclusion",
            aff: "SET UTF-8\nFLAG UTF-8\nNOSUGGEST Н\nFORBIDDENWORD Ф\nSFX Я Y 1\nSFX Я 0 ы .\nPFX П Y 1\nPFX П 0 не .\nMAP 1\nMAP её\n".into(),
            dic: "4\nкот/ЯП\nдом/Я\nтайна/НЯ\nзапрет/ФЯ\n".into(),
            probes: vec!["кот", "коты", "некоты", "домы", "недомы", "тайна", "тайнаы", "ТАЙНАЫ", "запрет", "запреты", "КОТЫ", "кОт"],
        },
        Case {
            name: "id: continuation and NOSUGGEST through derivations",
            aff: "SET UTF-8\nNOSUGGEST N\nKEY qwerty|asdf\nTRY abcdeikn\nREP 1\nREP dj j\nPFX P Y 1\nPFX P 0 ber .\nSFX A Y 1\nSFX A 0 kan/B .\nSFX B Y 1\nSFX B 0 lah .\n".into(),
            dic: "3\nmain/PA\nbaca/A\nrahasia/NA\n".into(),
            probes: vec!["main", "bermain", "mainkan", "mainkanlah", "bermainkanlah", "mainlah", "mainkanlahlah", "bacakanlah", "berbaca", "rahasia", "rahasiakan", "RAHASIAKANLAH"],
        },
    ];
    let flags: Vec<char> = ('a'..='z').chain('A'..='Z').chain('0'..='9').chain(['á', 'é', 'í', 'ó', 'ú', 'ç', 'ã', 'õ']).collect();
    let mut aff = String::from("SET UTF-8\nFLAG UTF-8\n");
    for flag in &flags {
        aff.push_str(&format!("SFX {flag} N 1\nSFX {flag} 0 {flag} .\n"));
    }
    out.push(Case {
        name: "pt-sized: 70 distinct flags across the old u128 boundary",
        aff,
        dic: format!("2\nverbo/{}\noutro/õ\n", flags.iter().collect::<String>()),
        probes: vec!["verboa", "verboz", "verboA", "verboZ", "verbo0", "verbo9", "verboç", "verboõ", "verboß", "outroõ", "outroa"],
    });
    out
}
