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

## Shared workflow trust

`release.yml`, `freebsd.yml`, `packaging-lint.yml` and `windows-arm64.yml` call
`storytold/craft-common-actions` at `@main`. That is deliberate: a fix to the shared pipeline
reaches every craft app without a pull request in each repository. The decision is recorded in
[craft-common-actions#6](https://github.com/storytold/craft-common-actions/issues/6); the remaining
hardening is [WordCraft #1358](https://github.com/storytold/wordcraft/issues/1358). The trade-off is
that the code which builds and signs a release can change outside this repository's history, so a
release cannot be reproduced from its WordCraft commit alone.

What limits the risk:

- WordCraft passes only the secrets it names explicitly (never `secrets: inherit`), so the shared
  workflow cannot read unrelated repository secrets.
- The signing and publishing jobs run in the `release` environment, whose deployment branch rule
  allows only the `release` branch. The secrets are passed to the called workflow as a whole, so
  this is not a boundary around them: the review controls on the shared repository are.
- Every third-party action in these workflows is pinned to a full commit SHA. The shared workflow's
  third-party actions are pinned the same way; its own composite actions are still called at
  `@main`, so pinning a caller alone would not freeze all of the code.
- `craft-fonts-ref` is a full commit SHA, so the fonts embedded in a release are pinned even
  though the workflow that checks them out is not.
- `cargo xtask workflows` (part of `cargo xtask ci`) rejects any `uses:` that is neither a local
  path nor a full 40-character commit SHA (or an immutable Docker digest). The shared workflow is
  the one allowlisted exception, with its reason, in `xtask/src/workflows.rs`.
- `storytold/craft-common-actions` must require a pull request and review on `main`, and run CI on
  every change. Those are repository settings, not files here. Until #1358 is closed, check them
  before a release, and review
  [the shared workflow's history](https://github.com/storytold/craft-common-actions/commits/main)
  when it changes.

Local packaging: `packaging/macos/package.sh --arch universal`, `packaging/windows/package.ps1`,
`packaging/linux/package.sh` (then `packaging/linux/flatpak-bundle.sh` for the Flatpak), `packaging/web/package.sh`.

Dry run: `gh workflow run release.yml --ref <branch>` builds the unsigned jobs (version, Linux,
Linux riscv64, Flatpak, FreeBSD, web) on any branch; the signing jobs are refused there by the `release`
environment's branch rule, so no draft release is made.
