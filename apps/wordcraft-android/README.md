# WordCraft para Android ARM64 (experimental)

Este é o alvo Android nativo do WordCraft. Ele reutiliza o motor e a interface egui do workspace completo, sem WebView. O APK é compilado para `arm64-v8a` (`aarch64-linux-android`) e requer Android 8.0/API 26 ou posterior.

## Compilar do zero no Linux ou WSL 2

### 1. Instale as ferramentas

Instale Rust pelo [rustup](https://rustup.rs/), o **JDK 21 completo** (precisa incluir `java`, `javac` e `keytool`) e as Android Command-line Tools. Com `ANDROID_HOME` apontando para o SDK, aceite a licença Android interativamente e instale as versões usadas por este alvo:

```sh
export ANDROID_HOME="$HOME/Android/Sdk"
export ANDROID_SDK_ROOT="$ANDROID_HOME"
"$ANDROID_HOME/cmdline-tools/latest/bin/sdkmanager" --licenses
"$ANDROID_HOME/cmdline-tools/latest/bin/sdkmanager" --install \
  "platform-tools" \
  "platforms;android-35" \
  "build-tools;35.0.0" \
  "ndk;30.0.16248370"
export ANDROID_NDK_HOME="$ANDROID_HOME/ndk/30.0.16248370"
```

O projeto fixa o API de compilação em 35, o Build Tools em 35.0.0 e o NDK em 30.0.16248370. As licenças precisam ser aceitas no computador antes do primeiro build.

### 2. Clone a branch Android

```sh
git clone --branch android-native-arm64 https://github.com/anonymous3210/wordcraft.git
cd wordcraft
```

Se o repositório já estiver clonado, entre na pasta e faça checkout da branch `android-native-arm64`.

### 3. Gere o APK

```sh
./apps/wordcraft-android/build.sh
```

O script verifica as ferramentas, instala `cargo-apk2` 1.5.0 se estiver faltando, adiciona o target Rust ARM64 e compila usando o `Cargo.lock` do repositório. O arquivo de saída é:

```text
target/debug/apk/wordcraft.apk
```

Para instalar num aparelho ARM64 conectado com depuração USB:

```sh
adb install -r target/debug/apk/wordcraft.apk
```

Também é possível executar manualmente, sem o script, depois de preparar as ferramentas:

```sh
rustup target add aarch64-linux-android
cargo install cargo-apk2 --version 1.5.0 --locked
cargo apk2 build --manifest-path apps/wordcraft-android/Cargo.toml
```

## Build release

Uma build release otimizada exige uma chave de assinatura própria. Guarde o arquivo e a senha fora do repositório e não reutilize a chave de depuração:

```sh
export CARGO_APK_RELEASE_KEYSTORE="$HOME/.android/wordcraft-release.jks"
read -rsp "Senha do keystore: " CARGO_APK_RELEASE_KEYSTORE_PASSWORD; echo
export CARGO_APK_RELEASE_KEYSTORE_PASSWORD
cargo apk2 build --release --manifest-path apps/wordcraft-android/Cargo.toml
unset CARGO_APK_RELEASE_KEYSTORE CARGO_APK_RELEASE_KEYSTORE_PASSWORD
```

O APK otimizado sai em `target/release/apk/wordcraft.apk`. O APK gerado pelo script padrão é de depuração e serve para instalação/teste; não o publique numa loja.

## Escopo atual

A inicialização, o renderizador e a interface são nativos. Ainda não estão conectados o seletor Android de documentos nem os serviços de impressão e compartilhamento. A interface existente foi desenhada para ribbon de desktop, portanto a ergonomia precisa ser validada num aparelho. Esta integração compila e empacota, mas não foi testada em dispositivo físico.
