//! The command that starts this installation's MCP server, for File › Options › Agents: an AI app
//! on the same computer (Claude Code, Codex, Claude Desktop, Cursor…) runs it as a child process
//! and talks to it over standard input and output.
//! - **AppImage**: the image itself with `--cli mcp`; its `AppRun` (`packaging/linux/AppRun`)
//!   hands that to the bundled wordcraft-cli, which lives in a mount that changes on every start.
//!   Run through `/usr/bin/env APPIMAGE_EXTRACT_AND_RUN=1`, so the runtime unpacks into /tmp
//!   instead of mounting with FUSE: started from VS Code's snap (Ubuntu 26.04) the mount fails and
//!   the server closes at once. The first start unpacks 115 MB in about 0.25 s; later ones reuse it.
//! - **Flatpak**: `flatpak run --command=wordcraft-cli <app id> mcp`, from the host.
//! - **Everything else** (deb, rpm, tar.gz, the Windows installer and zip): the wordcraft-cli next
//!   to this program. The macOS app has none (the CLI is a separate download), so `None`.

use std::ffi::OsString;
use std::path::Path;

/// This installation's command: the program, then its arguments.
pub fn command() -> Option<Vec<String>> {
    let exe = std::env::current_exe().ok()?;
    let env = |name: &str| std::env::var_os(name).filter(|v| !v.is_empty());
    resolve(&exe, env("APPIMAGE"), env("APPDIR"), env("FLATPAK_ID"), Path::is_file)
}

/// [`command`] for the program at `exe`, given the AppImage runtime's `APPIMAGE` and `APPDIR` and
/// Flatpak's `FLATPAK_ID`. Those are inherited by child processes, so they count only when `exe`
/// is inside them: WordCraft started from a terminal in another AppImage (Cursor's, say) is not
/// that AppImage. `None` also for a path that isn't valid Unicode, which a config file can't name.
fn resolve(
    exe: &Path,
    appimage: Option<OsString>,
    appdir: Option<OsString>,
    flatpak_id: Option<OsString>,
    exists: impl Fn(&Path) -> bool,
) -> Option<Vec<String>> {
    if let (Some(image), Some(dir)) = (appimage, appdir)
        && exe.starts_with(&dir)
    {
        return Some(vec!["/usr/bin/env".into(), "APPIMAGE_EXTRACT_AND_RUN=1".into(), image.into_string().ok()?, "--cli".into(), "mcp".into()]);
    }
    if let Some(id) = flatpak_id
        && exe.starts_with("/app")
    {
        return Some(vec!["flatpak".into(), "run".into(), "--command=wordcraft-cli".into(), id.into_string().ok()?, "mcp".into()]);
    }
    let cli = exe.with_file_name(if cfg!(windows) { "wordcraft-cli.exe" } else { "wordcraft-cli" });
    if !exists(&cli) {
        return None;
    }
    Some(vec![cli.into_os_string().into_string().ok()?, "mcp".into()])
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn os(s: &str) -> Option<OsString> {
        Some(s.into())
    }

    #[test]
    fn a_package_install_runs_the_cli_beside_the_app() {
        let cli = if cfg!(windows) { "wordcraft-cli.exe" } else { "wordcraft-cli" };
        let exe = PathBuf::from("/usr/bin").join(if cfg!(windows) { "wordcraft.exe" } else { "wordcraft" });
        let want = PathBuf::from("/usr/bin").join(cli).to_string_lossy().into_owned();
        assert_eq!(resolve(&exe, None, None, None, |p| p.ends_with(cli)), Some(vec![want, "mcp".to_string()]));
        // The macOS app bundle has no CLI next to it.
        assert_eq!(resolve(Path::new("/Applications/WordCraft.app/Contents/MacOS/WordCraft"), None, None, None, |_| false), None);
    }

    #[test]
    fn an_appimage_runs_itself_with_cli() {
        let exe = Path::new("/tmp/.mount_WordCrA1b2C3/usr/bin/wordcraft");
        let got = resolve(exe, os("/home/u/Apps/WordCraft.AppImage"), os("/tmp/.mount_WordCrA1b2C3"), None, |_| true);
        let want = ["/usr/bin/env", "APPIMAGE_EXTRACT_AND_RUN=1", "/home/u/Apps/WordCraft.AppImage", "--cli", "mcp"];
        assert_eq!(got, Some(want.iter().map(|s| s.to_string()).collect()));
    }

    /// A deb install started from a terminal inside another AppImage or Flatpak app inherits its
    /// variables; it still runs its own CLI.
    #[test]
    fn variables_inherited_from_another_app_are_ignored() {
        let exe = Path::new("/usr/bin/wordcraft");
        let own = resolve(exe, None, None, None, |_| true);
        assert!(own.is_some());
        assert_eq!(resolve(exe, os("/home/u/Cursor.AppImage"), os("/tmp/.mount_CursorX"), None, |_| true), own);
        assert_eq!(resolve(exe, None, None, os("com.visualstudio.code"), |_| true), own);
        assert_eq!(resolve(exe, os("/home/u/Cursor.AppImage"), None, None, |_| false), None, "APPIMAGE without APPDIR");
    }

    #[test]
    fn a_flatpak_runs_its_cli_through_flatpak_run() {
        let got = resolve(Path::new("/app/bin/wordcraft"), None, None, os("ai.storyteller.wordcraft"), |_| true);
        let want = ["flatpak", "run", "--command=wordcraft-cli", "ai.storyteller.wordcraft", "mcp"];
        assert_eq!(got, Some(want.iter().map(|s| s.to_string()).collect()));
        // `/application` is not `/app`.
        assert_eq!(resolve(Path::new("/application/wordcraft"), None, None, os("ai.storyteller.wordcraft"), |_| false), None);
    }

    /// The `--cli` the Options page offers is what the AppImage's entry point passes on.
    #[test]
    fn the_appimage_entry_point_hands_cli_to_wordcraft_cli() {
        let app_run = include_str!("../../../packaging/linux/AppRun");
        assert!(app_run.contains(r#"if [ "${1-}" = "--cli" ]; then"#), "{app_run}");
        assert!(app_run.contains(r#"exec "$HERE/usr/bin/wordcraft-cli" "$@""#), "{app_run}");
        let package = include_str!("../../../packaging/linux/package.sh");
        assert!(package.contains(r#"install -m755 "$HERE/AppRun" "$APPDIR/AppRun""#), "package.sh installs AppRun");
    }
}
