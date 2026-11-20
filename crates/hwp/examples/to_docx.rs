//! Convert an HWPX or HWP file to DOCX: `cargo run -p wordcraft-hwp --example to_docx -- in.hwpx out.docx`.
//!
//! 본 제품은 한글과컴퓨터의 한글 문서 파일(.hwp) 공개 문서를 참고하여 개발하였습니다.

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [input, output] = args.as_slice() else {
        eprintln!("usage: to_docx <in.hwpx|in.hwp> <out.docx>");
        return ExitCode::FAILURE;
    };
    let result = std::fs::read(input).map_err(|e| e.to_string()).and_then(|b| wordcraft_hwp::to_docx(&b).map_err(|e| e.to_string()));
    match result {
        Ok((docx, report)) => {
            if let Err(e) = std::fs::write(output, docx) {
                eprintln!("{output}: {e}");
                return ExitCode::FAILURE;
            }
            println!("{:?}", report.stats);
            if !report.unsupported.is_empty() {
                println!("unsupported: {:?}", report.unsupported);
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("{input}: {e}");
            ExitCode::FAILURE
        }
    }
}
