# wordcraft-bin (AUR)

PKGBUILD for the [AUR](https://aur.archlinux.org) package `wordcraft-bin`. It installs the
prebuilt Linux release tarball (`wordcraft-<ver>-linux-<arch>.tar.gz`) from GitHub Releases —
same binaries as the `.deb`/`.rpm` built by `.github/workflows/release.yml`, repackaged for
pacman. No compilation; `x86_64` and `aarch64`.

## Publishing a new version

1. Bump `pkgver` and update both `sha256sums_x86_64`/`sha256sums_aarch64` from that release's
   `SHA256SUMS.txt`.
2. `makepkg --printsrcinfo > .SRCINFO`
3. `makepkg -f` to confirm it builds, then push `PKGBUILD` + `.SRCINFO` to the AUR git repo
   (`ssh://aur@aur.archlinux.org/wordcraft-bin.git`) — a separate remote from this repo, requires
   an AUR account and SSH key.

This directory is the source of truth; the AUR repo is a mirror of just these two files.
