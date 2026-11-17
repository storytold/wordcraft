# Releasing WordCraft

1. On a branch, bump the version with `cargo xtask version set 0.2.0`, then open and
   merge a pull request into `main` (the one source of truth is
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
synced from the release manager's machine. On the release branch, the shared workflow fails the
macOS and Windows jobs if their signing credentials are missing. Local packaging can still
produce unsigned builds with a warning.

Local packaging: `packaging/macos/package.sh --arch universal`, `packaging/windows/package.ps1`,
`packaging/linux/package.sh` (then `packaging/linux/flatpak-bundle.sh` for the Flatpak), `packaging/web/package.sh`.

A scheduled dry run runs on `main` nightly at 03:00 UTC. It builds every release target except
macOS, without signing, entering the `release` environment, pushing Flatpak images or making a
draft release. Manual dispatch uses the normal release path (`dry-run` is false).
