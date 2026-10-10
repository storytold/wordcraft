//! Builds a spelling word list (`assets/spelling/*.words`) from plain text files.
//!
//! `cargo run -p wordcraft-proof --example wordlist -- forms.tsv nouns.txt assets/spelling/de-extra.txt assets/spelling/de.words`
//!
//! Every input but the last argument (the output) has one word per line, optionally followed by
//! a tab and anything else (a part-of-speech tag, as in a Morfologik export). Kept are single
//! words made of letters, hyphens, dots and apostrophes; lines starting with `#` (comments, or
//! entries the source disabled), multi-word entries and words starting with a digit are
//! dropped. See ATTRIBUTION.md for how `de.words` is made.

use wordcraft_proof::wordlist::WordList;

fn keep(w: &str) -> bool {
    let n = w.chars().count();
    (1..=64).contains(&n)
        && w.chars().next().is_some_and(char::is_alphabetic)
        && w.chars().all(|c| c.is_alphabetic() || matches!(c, '-' | '.' | '\'' | '’'))
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some((output, inputs)) = args.split_last().filter(|(_, i)| !i.is_empty()) else {
        return Err("usage: wordlist <input.txt|tsv>... <output.words>".into());
    };
    let mut words = Vec::new();
    for input in inputs {
        let text = std::fs::read_to_string(input)?;
        words.extend(
            text.lines().filter(|l| !l.starts_with('#')).filter_map(|l| l.split('\t').next()).map(str::trim).filter(|w| keep(w)).map(str::to_string),
        );
    }
    let list = WordList::from_words(words);
    let bytes = list.to_bytes();
    std::fs::write(output, &bytes)?;
    println!("{} words, {} bytes → {output}", list.len(), bytes.len());
    Ok(())
}
