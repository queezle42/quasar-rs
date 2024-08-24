{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

    flake-utils.url = "github:numtide/flake-utils";

    crane = {
      url = "github:ipetkov/crane";
      inputs.nixpkgs.follows = "nixpkgs";
    };

    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = {
    self,
    nixpkgs,
    flake-utils,
    crane,
    rust-overlay,
  }:
    flake-utils.lib.eachDefaultSystem (system: let
      pkgs = import nixpkgs {
        inherit system;
        overlays = [(import rust-overlay)];
      };

      craneLib = (crane.mkLib pkgs).overrideToolchain (p: p.rust-bin.nightly.latest.default.override {
        extensions = [ "rust-analyzer" "rust-src" ];
      });

      src = craneLib.cleanCargoSource ./.;

      commonArgs = {
        inherit src;
        strictDeps = true;
      };

      # Build *just* the cargo dependencies so they can be cached.
      # Check again later if we might want to use cargo-hakari to ensure
      # dependencies are compiled with the correct configuration.
      cargoArtifacts = craneLib.buildDepsOnly commonArgs;

      crateArgs =
        commonArgs
        // {
          inherit cargoArtifacts;
        };

      singlePackage = crateSrc:
        craneLib.buildPackage (crateArgs
          // rec {
            inherit (craneLib.crateNameFromCargoToml {cargoToml = "${crateSrc}/Cargo.toml";}) pname;
            cargoExtraArgs = "--package ${pname}";
          });

      watch = pkgs.writeScriptBin "watch" ''
        cargo watch --clear --delay .1 -x 'clippy --workspace --all-targets' -x 'nextest run --workspace --all-targets' -x 'test --doc --workspace'
      '';
    in {
      packages = {
        # Default builds the whole workspace and adds all binaries to the result
        default = craneLib.buildPackage crateArgs;

        # Partial builds
        quasar-observable = singlePackage ./quasar-observable;

        checkFormat = craneLib.cargoFmt {inherit src;};

        doc = craneLib.cargoDoc crateArgs;

        examples = craneLib.buildPackage (crateArgs
          // {
            pname = "quasar-examples";
            cargoExtraArgs = "--examples";
          });
      };

      checks = {
        build = self.packages.${system}.default;
        examples = self.packages.${system}.examples;
        format = self.packages.${system}.checkFormat;
      };

      devShells.default = craneLib.devShell {
        # Inherit inputs from checks.
        checks = self.checks.${system};

        # Additional dev-shell environment variables can be set directly
        # MY_CUSTOM_DEVELOPMENT_VAR = "something else";

        # Extra inputs can be added here; cargo and rustc are provided by default.
        packages = [
          pkgs.cargo-watch
          pkgs.cargo-nextest
          watch
        ];
      };
    });
}
