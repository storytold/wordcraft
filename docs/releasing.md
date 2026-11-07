# Releasing WordCraft

1. Bump the version on `main`: `cargo xtask version set 0.2.0` (the one source of truth is
   `[workspace.package] version`; the app shows it in `--version` and About, with the commit from
   `WORDCRAFT_BUILD_SHA`).
2. Merge `main` into `release` and push. `.github/workflows/release.yml` builds:

| Platform | Artifacts | Signing |
|---|---|---|
| macOS | universal `WordCraft.app` in a `.dmg`, universal CLI zip | Developer ID + notarization (`APPLE_*` secrets) |
| Windows | x64, x86 and arm64 `.msi` (one dual-scope package: per-user by default, needs no admin rights; "Everyone on this computer" installs per machine) + portable `.zip` | Azure Trusted Signing (`AZURE_*` secrets) |
| Linux | x86_64 and aarch64 `.AppImage` (+ `.zsync` for AppImageUpdate), `.flatpak` bundle, `.deb`, `.rpm`, `.tar.gz`; Flathub manifest | checksums |
| Linux riscv64 | `.tar.gz` (cross-compiled on `ubuntu-24.04`, glibc >= 2.39; CLI smoke-tested under QEMU) | checksums |
| FreeBSD | x86_64 `.tar.gz` | checksums |
| Web | `wordcraft-web-<version>.zip` (static site: `index.html`, wasm, glue) | none |

3. The run drafts a GitHub Release `WordCraft v<version>` with `SHA256SUMS.txt`; review and publish it.

Signing secrets live in the repository's `release` environment (deployment branch: `release` only),
synced from the release manager's machine. Missing secrets
produce unsigned builds with a warning, so forks still build.

Local packaging: `packaging/macos/package.sh --arch universal`, `packaging/windows/package.ps1`,
`packaging/linux/package.sh` (then `packaging/linux/flatpak-bundle.sh` for the Flatpak), `packaging/web/package.sh`.

Dry run: `gh workflow run release.yml --ref <branch>` builds the unsigned jobs (version, Linux,
Linux riscv64, Flatpak, FreeBSD, web) on any branch; the signing jobs are refused there by the `release`
environment's branch rule, so no draft release is made.

Local macOS packaging prefers `xcrun stapler` (staple and validate). Without it, `rcodesign staple` uses the same bounded retries. If neither tool is available, packaging warns that Gatekeeper will fetch the ticket online and still runs the final Gatekeeper assessments.

On GitHub Actions, notarized artifacts still require the native `stapler` to staple and validate the ticket; absence is fatal even if `rcodesign` is installed. The `rcodesign` fallback and missing-tool warning are for local packaging only. A successful online Gatekeeper assessment is not evidence that a ticket was stapled. These paths are covered with synthetic shell mocks, not a new Apple notarization run.

## Revision history

| Date | Change |
|---|---|

| 2026-11-05 | trivial | PR #5268: local macOS packaging may use rcodesign or warn when stapling tools are absent; GitHub Actions releases still require native stapling and ticket validation. Synthetic app/DMG and failure controls; no new Apple notarization run. |