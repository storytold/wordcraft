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

## The Nightly channel

A separate **Nightly** channel exists beside the release above. Its producer and signer are
**enabled for supervised manual runs only** (no schedule, no automatic dispatch) and its signing
policy is installed; it is not the release dry run above.

What the channel is:

- A build channel: `WORDCRAFT_BUILD_CHANNEL=nightly-v1` (with `WORDCRAFT_VERSION`, `WORDCRAFT_BUILD_SHA`
  and `WORDCRAFT_BUILD_DATE`) compiles a `WordCraft Nightly` build with its own version line,
  Windows identity and icon. Unset or `stable` stays the ordinary build.
- The Launcher protocol: `--launch-profile nightly-v1 --profile-root <abs> -- files`, with
  `--version` answered in both plain and profile form before any state is read. A Nightly build
  without a profile is refused (it never falls back to the stable data).
- Its own data: preferences, window place, logs, recovery, fonts, templates, the dictionary, the
  suite font store and any control tokens live under that profile, which may not overlap stable
  data, the shared font store or an installation.
- Packaging: `packaging/linux/nightly.sh --arch x86_64` (plus the Arch handoff in
  `packaging/linux/nightly-arch-tar.sh`), `packaging/windows/package.ps1 -Arch x64 -Nightly
  -BuildOnly`, `packaging/macos/nightly.sh --arch aarch64` with `nightly-sign.sh` and
  `nightly-verify.sh` for the trusted step, and `packaging/nightly-icons.py` for its icons.
- Qualification: `.github/workflows/nightly-build.yml` builds and checks all three unsigned
  archives on the `full-ci` label or on dispatch, with no secrets;
  `packaging/nightly-tests.py` runs the synthetic suites in Packaging lint.

Every packaged Linux format ships one launcher (`packaging/linux/nightly-launcher.sh`) that starts
the binary the channel's way, because a Nightly build refuses to run without a profile:
`wordcraft --launch-profile nightly-v1 --profile-root "${XDG_DATA_HOME:-$HOME/.local/share}/wordcraft-nightly" -- "$@"`.
That launcher is the deb/rpm `/usr/bin/wordcraft-nightly` wrapper (real binary in
`/usr/lib/wordcraft-nightly/`), the AppImage `AppRun` (a script, not a symlink, calling
`usr/bin/wordcraft`), the Arch `bin/wordcraft-nightly` launcher (real binary in
`bin/.wordcraft-nightly/`), the plain tar's `bin/wordcraft` (real binary in `lib/wordcraft-nightly/`), which the Flatpak
bundle also installs. The app-owned Nightly tar's `wordcraft` is craft-common-actions' shared
template instead (`packaging/linux/nightly-tar-launch.sh`, #363 rendered for this app), starting
`wordcraft-bin` beside it with
`--profile-root ${XDG_DATA_HOME:-$HOME/.local/share}/ai.storyteller.wordcraft.nightly`, and asking a
bare `--version`/`-V` in the Launcher's form. `XDG_DATA_HOME` keeps the profile on the app's own data directory inside the
Flatpak sandbox (`~/.var/app/ai.storyteller.wordcraft.nightly/data/wordcraft-nightly`), which
persists; the sandbox's home is discarded. Redirecting `XDG_*` alone is refused: the build's root
validator derives the stable boundary from it, so the profile would sit inside it. The launcher
passes an already-supplied Launcher form (`--launch-profile`, and `--version`/`-V`) straight
through, so the ArtCraft Launcher's own root is used and its version check opens no window. The
remaining activation steps are described in
[nightly-release-preparation.md](nightly-release-preparation.md).
