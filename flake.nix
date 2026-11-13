{
  description = "Headless Rust rules engine for the Dominion deck-building game";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    fenix = {
      url = "github:nix-community/fenix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = {
    self,
    nixpkgs,
    fenix,
    flake-utils,
  }:
    {
      nixosModules.default = import ./nix/module.nix self;
    }
    // flake-utils.lib.eachDefaultSystem (system: let
      pkgs = nixpkgs.legacyPackages.${system};
      fenixPkgs = fenix.packages.${system};
      toolchain = fenixPkgs.combine [
        (fenixPkgs.stable.withComponents [
          "cargo"
          "clippy"
          "rust-src"
          "rustc"
          "rustfmt"
        ])
        fenixPkgs.targets.wasm32-unknown-unknown.stable.rust-std
      ];
      rustPlatform = pkgs.makeRustPlatform {
        cargo = toolchain;
        rustc = toolchain;
      };
      workspace = builtins.fromTOML (builtins.readFile ./Cargo.toml);
      src = pkgs.lib.cleanSourceWith {
        src = ./.;
        filter = path: type: let
          base = baseNameOf path;
        in
          !(builtins.elem base [".git" ".github" ".direnv" ".worktrees" "target"]);
      };
      dominion = rustPlatform.buildRustPackage {
        pname = "dominion-engine";
        version = workspace.workspace.package.version;
        inherit src;
        cargoLock.lockFile = ./Cargo.lock;
        meta.mainProgram = "dominion";
      };
      # The web client, as `just web-build` builds it: dx drives cargo (offline, from the
      # vendored lockfile), wasm-bindgen and wasm-opt, all from nixpkgs.
      web = rustPlatform.buildRustPackage {
        pname = "dominion-web";
        version = workspace.workspace.package.version;
        inherit src;
        cargoLock.lockFile = ./Cargo.lock;
        nativeBuildInputs = [
          pkgs.dioxus-cli
          pkgs.wasm-bindgen-cli_0_2_130
          pkgs.binaryen
        ];
        buildPhase = ''
          runHook preBuild
          export HOME=$TMPDIR
          (cd crates/dominion-web && dx build --release --platform web --package dominion-web --features web)
          runHook postBuild
        '';
        installPhase = ''
          runHook preInstall
          cp -r target/dx/dominion-web/release/web/public $out
          runHook postInstall
        '';
        doCheck = false;
      };
      # `dominion` serving the packaged client when no web_dir is configured.
      dominion-with-web = pkgs.symlinkJoin {
        name = "dominion-with-web-${dominion.version}";
        paths = [dominion];
        nativeBuildInputs = [pkgs.makeWrapper];
        postBuild = ''
          wrapProgram $out/bin/dominion --set-default DOMINION_WEB_DIR ${web}
        '';
        meta.mainProgram = "dominion";
      };
    in {
      packages = {
        default = dominion;
        inherit web dominion-with-web;
      };

      checks = {
        default = dominion;
        inherit web;
      };

      devShells.default = pkgs.mkShell {
        packages = [
          toolchain
          pkgs.just
          pkgs.alejandra
          # The web client: dx drives the wasm build, with the wasm-bindgen matching
          # crates/dominion-web's pin and wasm-opt for release builds.
          pkgs.dioxus-cli
          pkgs.wasm-bindgen-cli_0_2_130
          pkgs.binaryen
        ];
        RUST_BACKTRACE = 1;
      };

      formatter = pkgs.alejandra;
    });
}
