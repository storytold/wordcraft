//! Write the edits of a DOCX (opened from an HWPX or HWP) back into a copy of that file:
//! `cargo run -p wordcraft-hwp --example write_back -- original.hwpx edited.docx out.hwpx`
//! (or `original.hwp ... out.hwp`).
//!
//! Text edits only: formatting, table and picture edits need the documents before and after
//! (as WordCraft's own save passes them), so this example leaves them out without listing them.
//!
//! 본 제품은 한글과컴퓨터의 한글 문서 파일(.hwp) 공개 문서를 참고하여 개발하였습니다.

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [original, edited, output] = args.as_slice() else {
        eprintln!("usage: write_back <original.hwpx|.hwp> <edited.docx> <out.hwpx|.hwp>");
        return ExitCode::FAILURE;
    };
    let read = |p: &str| std::fs::read(p).map_err(|e| format!("{p}: {e}"));
    let write = |o: &[u8], e: &[u8]| {
        let r =
            if original.to_ascii_lowercase().ends_with(".hwp") { wordcraft_hwp::write_back_hwp(o, e, None) } else { wordcraft_hwp::write_back(o, e) };
        r.map_err(|e| e.to_string())
    };
    eprintln!("note: text edits only; formatting, table and picture edits are not written by this example");
    let result = read(original).and_then(|o| read(edited).and_then(|e| write(&o, &e)));
    match result {
        Ok((bytes, report)) => {
            if let Err(e) = std::fs::write(output, bytes) {
                eprintln!("{output}: {e}");
                return ExitCode::FAILURE;
            }
            println!(
                "{{\"modified\": {}, \"inserted\": {}, \"deleted\": {}, \"unchanged\": {}, \"skipped\": {:?}}}",
                report.modified, report.inserted, report.deleted, report.unchanged, report.skipped
            );
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}
