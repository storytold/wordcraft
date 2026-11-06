{
  description = "WordCraft: a word processor that opens and saves Word files";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs = { self, nixpkgs }:
    let
      systems = [ "x86_64-linux" "aarch64-linux" ];
      forAllSystems = f: nixpkgs.lib.genAttrs systems (system: f nixpkgs.legacyPackages.${system});

      appId = "ai.storyteller.wordcraft";

      # Libraries the app loads with dlopen at runtime (winit, wgpu, wayland-client,
      # xkbcommon). They are not linked, so Nix cannot detect them: we put them on
      # LD_LIBRARY_PATH (wrapper for the package, variable for the dev shell).
      runtimeLibs = pkgs: with pkgs; [
        vulkan-loader
        libGL
        libxkbcommon
        wayland
        libx11
        libxcursor
        libxi
        libxrandr
        libxcb
      ];

      # Same craft-fonts revision as the release workflow (.github/workflows/release.yml):
      # bump both together. Official releases embed these fonts (Japanese and Chinese UI).
      craftFonts = pkgs: pkgs.fetchFromGitHub {
        owner = "storytold";
        repo = "craft-fonts";
        rev = "b076417b5880bd4154bd22f00695b1d28c6bea83";
        hash = "sha256-jZ3VBAJDofB6RvsS7GGV5lLT4xiShIjpz5XHBCD2oUc=";
      };

      # Release date of the AppStream metadata: the commit date of the flake source, which is
      # reproducible, unlike the current time.
      buildDate =
        let d = self.lastModifiedDate or "";
        in
        if builtins.stringLength d >= 8
        then "${builtins.substring 0 4 d}-${builtins.substring 4 2 d}-${builtins.substring 6 2 d}"
        else "1970-01-01";

      mkWordcraft = pkgs:
        let
          inherit (pkgs) lib;
          libs = runtimeLibs pkgs;
          fonts = craftFonts pkgs;
          version = (builtins.fromTOML (builtins.readFile ./Cargo.toml)).workspace.package.version;
        in
        pkgs.rustPlatform.buildRustPackage {
          pname = "wordcraft";
          inherit version;

          src = lib.cleanSourceWith {
            src = ./.;
            filter = path: _type:
              !(builtins.elem (baseNameOf path) [ "target" ".git" ".github" "result" "flake.nix" "flake.lock" ]);
          };

          cargoLock = {
            lockFile = ./Cargo.lock;
            # Git dependencies, if any, are fetched from the revisions pinned in
            # Cargo.lock, so no hash has to be written here.
            allowBuiltinFetchGit = true;
          };

          # Only the GUI and the CLI (apps/wordcraft-web targets wasm).
          cargoBuildFlags = [ "-p" "wordcraft" "-p" "wordcraft-cli" ];

          # The tests open windows and need a GPU: they cannot run in the Nix sandbox.
          doCheck = false;

          nativeBuildInputs = [ pkgs.pkg-config pkgs.makeWrapper ];
          buildInputs = libs;

          env = {
            CRAFT_FONTS_DIR = "${fonts}";
            CRAFT_FONTS_REQUIRED = "1";
          };

          postInstall = ''
            install -Dm644 packaging/linux/${appId}.desktop \
              $out/share/applications/${appId}.desktop
            install -Dm644 packaging/linux/${appId}.mime.xml \
              $out/share/mime/packages/${appId}.xml
            mkdir -p $out/share/metainfo
            substitute packaging/linux/${appId}.metainfo.xml.in \
              $out/share/metainfo/${appId}.metainfo.xml \
              --subst-var-by VERSION ${version} \
              --subst-var-by DATE ${buildDate}
            mkdir -p $out/share/icons
            cp -R assets/app-icon/hicolor $out/share/icons/

            doc=$out/share/doc/wordcraft
            mkdir -p $doc
            for f in README.md LICENSE LICENSE-MIT LICENSE-APACHE COPYRIGHT NOTICE ATTRIBUTION.md; do
              if [ -f "$f" ]; then install -Dm644 "$f" "$doc/$f"; fi
            done
            for f in ${fonts}/fonts/*/OFL.txt; do
              if [ -f "$f" ]; then install -Dm644 "$f" "$doc/OFL-$(basename "$(dirname "$f")").txt"; fi
            done
          '';

          postFixup = ''
            wrapProgram $out/bin/wordcraft \
              --prefix LD_LIBRARY_PATH : ${lib.makeLibraryPath libs}
          '';

          meta = {
            description = "Word processor that opens and saves Word files";
            homepage = "https://github.com/storytold/wordcraft";
            license = with lib.licenses; [ mit asl20 ];
            mainProgram = "wordcraft";
            platforms = systems;
          };
        };
    in
    {
      packages = forAllSystems (pkgs:
        let wordcraft = mkWordcraft pkgs;
        in
        {
          inherit wordcraft;
          default = wordcraft;
        });

      overlays.default = final: _prev: {
        wordcraft = mkWordcraft final;
      };

      devShells = forAllSystems (pkgs: {
        default = pkgs.mkShell {
          inputsFrom = [ self.packages.${pkgs.stdenv.hostPlatform.system}.wordcraft ];
          packages = with pkgs; [ cargo rustc clippy rustfmt rust-analyzer ];
          LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath (runtimeLibs pkgs);
          CRAFT_FONTS_DIR = "${craftFonts pkgs}";
          RUST_SRC_PATH = "${pkgs.rustPlatform.rustLibSrc}";
        };
      });

      checks = forAllSystems (pkgs: {
        wordcraft = self.packages.${pkgs.stdenv.hostPlatform.system}.wordcraft;
      });

      formatter = forAllSystems (pkgs: pkgs.nixpkgs-fmt);
    };
}
