//! Build `assets/proofing/pl.dic` (WordCraft's compact affix format) from the SJP.PL Polish
//! Hunspell dictionary.
//!
//! 1. Download `sjp-hunspell-pl-YYYYMMDD.zip` from <https://sjp.pl/sl/en/> (licensed under the
//!    user's choice of GPL 2, LGPL 2.1, MPL 1.1, Apache 2.0 or CC BY 4.0; WordCraft uses it under
//!    Apache 2.0, see `ATTRIBUTION.md`). Unzip it, then the `pl_PL.zip` inside.
//! 2. `cargo run -p wordcraft-proof --example pl_dictionary -- pl_PL.aff pl_PL.dic assets/proofing/pl.dic 2026-09-01`
//!
//! The last argument is the dictionary's version date (from `README_pl_PL.txt`), recorded in
//! the file. The `.aff`/`.dic` pair is decoded from the encoding its `SET` line names
//! (ISO8859-2 or UTF-8); `--utf8` before the paths forces UTF-8 (for files already converted).

use std::process::ExitCode;

use wordcraft_proof::affix::AffixDict;

/// ISO 8859-2 code points 0xA0..=0xFF (0x00..=0x9F map to themselves).
const ISO_8859_2_HIGH: &str = "\u{a0}Ą˘Ł¤ĽŚ§¨ŠŞŤŹ\u{ad}ŽŻ°ą˛ł´ľśˇ¸šşťź˝žżŔÁÂĂÄĹĆÇČÉĘËĚÍÎĎĐŃŇÓÔŐÖ×ŘŮÚŰÜÝŢßŕáâăäĺćçčéęëěíîďđńňóôőö÷řůúűüýţ˙";

fn decode_latin2(bytes: &[u8]) -> String {
    let high: Vec<char> = ISO_8859_2_HIGH.chars().collect();
    bytes.iter().map(|&b| if b < 0xa0 { char::from(b) } else { high.get(usize::from(b - 0xa0)).copied().unwrap_or('\u{fffd}') }).collect()
}

fn decode(bytes: &[u8], force_utf8: bool) -> Result<String, String> {
    let head = String::from_utf8_lossy(bytes.get(..bytes.len().min(4096)).unwrap_or_default()).to_string();
    let set = head.lines().find_map(|l| l.strip_prefix("SET ")).map(str::trim).unwrap_or("UTF-8").to_ascii_uppercase();
    if force_utf8 || set == "UTF-8" {
        String::from_utf8(bytes.to_vec()).map_err(|e| format!("not UTF-8: {e}"))
    } else if set == "ISO8859-2" || set == "ISO-8859-2" {
        Ok(decode_latin2(bytes))
    } else {
        Err(format!("unsupported encoding {set}"))
    }
}

fn run() -> Result<(), String> {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let force_utf8 = args.first().is_some_and(|a| a == "--utf8");
    if force_utf8 {
        args.remove(0);
    }
    let [aff_path, dic_path, out_path, version] = args.as_slice() else {
        return Err("usage: pl_dictionary [--utf8] <pl_PL.aff> <pl_PL.dic> <out.dic> <version date>".into());
    };
    let aff_bytes = std::fs::read(aff_path).map_err(|e| format!("{aff_path}: {e}"))?;
    let aff = decode(&aff_bytes, force_utf8)?;
    let dic_bytes = std::fs::read(dic_path).map_err(|e| format!("{dic_path}: {e}"))?;
    // The .dic has no SET line of its own: it is in the .aff's encoding.
    let dic = if force_utf8 || !aff.contains("SET ISO8859-2") {
        String::from_utf8(dic_bytes).map_err(|e| format!("{dic_path}: not UTF-8: {e}"))?
    } else {
        decode_latin2(&dic_bytes)
    };
    let source = format!("SJP.PL Polish spelling dictionary for Hunspell, version {version}, https://sjp.pl/sl/en/");
    let comments = [
        source.as_str(),
        "Copyright the SJP.PL contributors (https://sjp.pl, sjpslownik@gmail.com).",
        "Offered under GPL 2, LGPL 2.1, MPL 1.1, Apache 2.0 or CC BY 4.0; WordCraft uses it under the Apache License 2.0.",
        "Converted unchanged (stems, flags, affix rules, TRY, MAP and REP) to WordCraft's affix format by crates/proof/examples/pl_dictionary.rs.",
    ];
    let d = AffixDict::from_hunspell(&aff, &dic, &comments)?;
    let bytes = d.to_bytes();
    let back = AffixDict::from_bytes(&bytes)?;
    if back.len() != d.len() || back.rule_count() != d.rule_count() {
        return Err("round trip changed the dictionary".into());
    }
    for w in ["źdźbło", "gżegżółka", "chrząszcz", "pięćdziesięciu", "rzeka", "którzy", "Warszawie", "niebieskiego"] {
        if !back.check(w) {
            return Err(format!("`{w}` is not accepted: conversion problem"));
        }
    }
    std::fs::write(out_path, &bytes).map_err(|e| format!("{out_path}: {e}"))?;
    println!(
        "{out_path}: {} stems, {} affix rules, {} REP, {} MAP groups, {} bytes",
        d.len(),
        d.rule_count(),
        d.rep().len(),
        d.map().len(),
        bytes.len()
    );
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}
