//! The clap definition is consistent, and every shell's completions and every man page
//! (`examples/man-and-completions.rs`) can be generated from it.

use clap::ValueEnum;
use clap_complete::Shell;

// The example itself, so the test writes the files exactly as the packages get them.
#[allow(dead_code)]
#[path = "../../examples/man-and-completions.rs"]
mod example;

#[test]
fn command_is_consistent() {
    wordcraft_cli::command().debug_assert();
}

#[test]
fn completions_generate_for_every_shell() {
    for &shell in Shell::value_variants() {
        let mut out = Vec::new();
        clap_complete::generate(shell, &mut wordcraft_cli::command(), "wordcraft-cli", &mut out);
        assert!(!out.is_empty(), "{shell}");
    }
}

#[test]
fn man_pages_render_for_the_command_and_every_subcommand() {
    let cmd = wordcraft_cli::command();
    for page in std::iter::once(&cmd).chain(cmd.get_subcommands()) {
        let mut out = Vec::new();
        clap_mangen::Man::new(page.clone()).render(&mut out).unwrap();
        assert!(!out.is_empty(), "{}", page.get_name());
    }
}

/// The example writes the five completion files under the names `packaging/linux/package.sh` and
/// `packaging/freebsd/package.sh` install, `wordcraft-cli.1` and a page per subcommand, and no
/// pages for the `help` subcommand.
#[test]
fn the_example_writes_the_files_the_packages_install() {
    let dir = std::env::temp_dir().join(format!("wc-cli-{}-man-and-completions", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    example::generate(&dir).unwrap();

    let mut written: Vec<String> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name().into_string().unwrap()).collect();
    written.sort();
    let completions = ["wordcraft-cli.bash", "_wordcraft-cli", "wordcraft-cli.fish", "wordcraft-cli.elv", "_wordcraft-cli.ps1"];
    let subcommands = ["commands", "convert", "info", "inspect", "mcp", "parity", "render", "run", "text", "validate", "zotero"];
    let mut expected: Vec<String> = completions.map(String::from).into();
    expected.push("wordcraft-cli.1".into());
    expected.extend(subcommands.iter().map(|s| format!("wordcraft-cli-{s}.1")));
    expected.sort();
    assert_eq!(written, expected);
    std::fs::remove_dir_all(&dir).unwrap();
}
