# shellcheck shell=bash
# The X11 keyboard bridge is opened with dlopen by winit, not linked into the executable.
# AppImage packaging therefore has to copy it explicitly, together with libxcb-xkb (#5363).

bundle_x11_keyboard() {
  local appdir="$1" compiler="${CROSS_COMPILE:-}gcc" library source notice package machine sysroot
  machine="$("$compiler" -dumpmachine)"
  case "$ARCH:$machine" in
    x86_64:x86_64*-linux-* | aarch64:aarch64*-linux-*) ;;
    *) echo "error: AppImage architecture $ARCH does not match the library compiler $machine" >&2; return 1 ;;
  esac
  sysroot="$("$compiler" -print-sysroot)"
  for library in libxkbcommon-x11.so.0 libxcb-xkb.so.1; do
    source="$("$compiler" -print-file-name="$library")"
    if [[ "$source" = "$library" || ! -f "$source" ]]; then
      echo "error: AppImage needs target $library; install libxkbcommon-x11-0 and libxcb-xkb1 on the build machine" >&2
      return 1
    fi
    case "$library" in
      libxkbcommon-x11.so.0) package=libxkbcommon-x11-0 ;;
      libxcb-xkb.so.1) package=libxcb-xkb1 ;;
    esac
    # Keep the target packages' notices as well as their libraries.
    notice="$sysroot/usr/share/doc/$package/copyright"
    if [[ ! -f "$notice" ]]; then
      echo "error: missing copyright notice for $package: $notice" >&2
      return 1
    fi
    install -Dm644 "$source" "$appdir/usr/lib/$library"
    install -Dm644 "$notice" "$appdir/usr/share/doc/wordcraft/$package-copyright"
  done
  install -Dm755 "$HERE/AppRun" "$appdir/AppRun"
}
