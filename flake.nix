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
    flake-utils.lib.eachDefaultSystem (system: let
      pkgs = nixpkgs.legacyPackages.${system};
      toolchain = fenix.packages.${system}.stable.withComponents [
        "cargo"
        "clippy"
        "rust-src"
        "rustc"
        "rustfmt"
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
      };
    in {
      packages.default = dominion;

      checks.default = dominion;

      devShells.default = pkgs.mkShell {
        packages = [
          toolchain
          pkgs.just
          pkgs.alejandra
        ];
        RUST_BACKTRACE = 1;
      };

      formatter = pkgs.alejandra;
    });
}
