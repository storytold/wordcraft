#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

fail() {
  printf 'ERRO: %s\n' "$1" >&2
  exit 1
}

[[ "$(uname -s)" == "Linux" ]] || fail "Este script suporta Linux e WSL. No Windows, use WSL 2 com a toolchain Android Linux."
command -v cargo >/dev/null 2>&1 || fail "Rust/Cargo não encontrado. Instale Rust pelo rustup: https://rustup.rs/"
command -v rustup >/dev/null 2>&1 || fail "rustup não encontrado. Instale Rust pelo rustup: https://rustup.rs/"
command -v java >/dev/null 2>&1 || fail "Java não encontrado. Instale JDK 21 (incluindo java, javac e keytool)."
command -v javac >/dev/null 2>&1 || fail "javac não encontrado. Instale o JDK completo, não apenas o Java Runtime."
command -v keytool >/dev/null 2>&1 || fail "keytool não encontrado. Instale o JDK completo."

export ANDROID_HOME="${ANDROID_HOME:-${ANDROID_SDK_ROOT:-$HOME/Android/Sdk}}"
export ANDROID_SDK_ROOT="$ANDROID_HOME"
export ANDROID_NDK_HOME="${ANDROID_NDK_HOME:-$ANDROID_HOME/ndk/30.0.16248370}"
export NDK_HOME="$ANDROID_NDK_HOME"

[[ -f "$ANDROID_HOME/platforms/android-35/android.jar" ]] || fail "Android SDK API 35 ausente. Instale: platforms;android-35"
[[ -x "$ANDROID_HOME/build-tools/35.0.0/aapt" ]] || fail "Android Build Tools 35.0.0 ausentes. Instale: build-tools;35.0.0"
[[ -x "$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/linux-x86_64/bin/aarch64-linux-android26-clang" ]] || fail "NDK 30.0.16248370 ausente ou inválido em $ANDROID_NDK_HOME"

if ! command -v cargo-apk2 >/dev/null 2>&1; then
  printf 'Instalando cargo-apk2 1.5.0...\n'
  cargo install cargo-apk2 --version 1.5.0 --locked
fi

rustup target add aarch64-linux-android
cargo apk2 build --manifest-path "$ROOT/apps/wordcraft-android/Cargo.toml"

APK="$ROOT/target/debug/apk/wordcraft.apk"
[[ -s "$APK" ]] || fail "O build terminou sem produzir $APK"
printf '\nAPK criado com sucesso:\n%s\n' "$APK"
