{
  description = "WordCraft: a word processor that opens and saves Word files";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs = { self, nixpkgs }:
    let
      systems = [ "x86_64-linux" "aarch64-linux" ];
      forAllSystems = f: nixpkgs.lib.genAttrs systems (system: f nixpkgs.legacyPackages.${system});

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
    in
    {
      packages = forAllSystems (pkgs:
        let
          inherit (pkgs) lib;
          libs = runtimeLibs pkgs;

          wordcraft = pkgs.rustPlatform.buildRustPackage {
            pname = "wordcraft";
            version = (builtins.fromTOML (builtins.readFile ./Cargo.toml)).workspace.package.version;

            src = lib.cleanSourceWith {
              src = ./.;
              filter = path: _type: !(builtins.elem (baseNameOf path) [ "target" ".git" ".github" "result" ]);
            };

            cargoLock = {
              lockFile = ./Cargo.lock;
              # Fetches the git dependencies (craft-libs) straight from the revisions
              # pinned in Cargo.lock, so no hash has to be written here.
              allowBuiltinFetchGit = true;
            };

            # Only the GUI and the CLI (apps/wordcraft-web targets wasm).
            cargoBuildFlags = [ "-p" "wordcraft" "-p" "wordcraft-cli" ];

            # The tests open windows and need a GPU: they cannot run in the Nix sandbox.
            doCheck = false;

            nativeBuildInputs = [ pkgs.pkg-config pkgs.makeWrapper ];
            buildInputs = libs;

            postInstall = ''
              install -Dm644 packaging/linux/ai.storyteller.wordcraft.desktop \
                $out/share/applications/ai.storyteller.wordcraft.desktop
              install -Dm644 packaging/linux/ai.storyteller.wordcraft.mime.xml \
                $out/share/mime/packages/ai.storyteller.wordcraft.xml
              mkdir -p $out/share/icons
              cp -R assets/app-icon/hicolor $out/share/icons/
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
          inherit wordcraft;
          default = wordcraft;
        });

      devShells = forAllSystems (pkgs: {
        default = pkgs.mkShell {
          inputsFrom = [ self.packages.${pkgs.stdenv.hostPlatform.system}.wordcraft ];
          packages = with pkgs; [ cargo rustc clippy rustfmt rust-analyzer ];
          LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath (runtimeLibs pkgs);
          RUST_SRC_PATH = "${pkgs.rustPlatform.rustLibSrc}";
        };
      });

      formatter = forAllSystems (pkgs: pkgs.nixpkgs-fmt);
    };
}
