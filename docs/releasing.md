# Releasing WordCraft

The canonical recipe is [craftrules `release/playbook.md`](https://github.com/storytold/craftrules/blob/main/release/playbook.md);
this file has the WordCraft specifics.

1. Bump the version on `main`: `cargo xtask version set 0.2.0` (the one source of truth is
   `[workspace.package] version`; the app shows it in `--version` and About, with the commit from
   `WORDCRAFT_BUILD_SHA`).
2. Merge `main` into `release` and push. `.github/workflows/release.yml` builds:

| Platform | Artifacts | Signing |
|---|---|---|
| macOS | universal `WordCraft.app` in a `.dmg`, universal CLI zip | Developer ID + notarization (`APPLE_*` secrets) |
| Windows | x64, x86 and arm64 `.msi` + portable `.zip` | Azure Trusted Signing (`AZURE_*` secrets) |
| Linux | x86_64 and aarch64 `.AppImage` (+ `.zsync` for AppImageUpdate), `.flatpak` bundle, `.deb`, `.rpm`, `.tar.gz`; Flathub manifest | checksums |
| FreeBSD | x86_64 `.tar.gz` | checksums |
| Web | `wordcraft-web-<version>.zip` (static site: `index.html`, wasm, glue) | none |

3. The run drafts a GitHub Release `WordCraft v<version>` with `SHA256SUMS.txt`; review and publish it.

Signing secrets live in the repository's `release` environment (deployment branch: `release` only),
synced from the release manager's machine (`craftrules/release/signing-setup.md`). Missing secrets
produce unsigned builds with a warning, so forks still build.

Local packaging: `packaging/macos/package.sh --arch universal`, `packaging/windows/package.ps1`,
`packaging/linux/package.sh` (then `packaging/linux/flatpak-bundle.sh` for the Flatpak), `packaging/web/package.sh`.

Dry run: `gh workflow run release.yml --ref <branch>` builds the unsigned jobs (version, Linux,
Flatpak, FreeBSD, web) on any branch; the signing jobs are refused there by the `release`
environment's branch rule, so no draft release is made.
