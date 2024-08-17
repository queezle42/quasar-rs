{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

    crane = {
      url = "github:ipetkov/crane";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = { self, nixpkgs, crane }:
  let
    lib = nixpkgs.lib;
    systems = lib.platforms.unix;
    forAllSystems = lib.genAttrs systems;
  in {
    packages = forAllSystems (system:
      let
        pkgs = import nixpkgs { inherit system; };

        craneLib = crane.mkLib pkgs;

        xmlFilter = path: _type: builtins.match ".*\.xml" path != null;

        src = craneLib.cleanCargoSource ./.;

        commonArgs = {
          inherit src;
          strictDeps = true;
        };

        # Build *just* the cargo dependencies so they can be cached.
        # Check again later if we might want to use cargo-hakari to ensure
        # dependencies are compiled with the correct configuration.
        cargoArtifacts = craneLib.buildDepsOnly commonArgs;

        crateArgs = commonArgs // {
          inherit cargoArtifacts;
        };

        singlePackage = crateSrc: craneLib.buildPackage (crateArgs // rec {
          inherit (craneLib.crateNameFromCargoToml { cargoToml = "${crateSrc}/Cargo.toml"; }) pname;
          cargoExtraArgs = "--package ${pname}";
        });

      in {
        # Default builds the whole workspace and adds all binaries to the result
        default = craneLib.buildPackage crateArgs;

        # Partial builds
        quasar-observable = singlePackage ./quasar-observable;

        checkFormat = craneLib.cargoFmt { inherit src; };

        doc = craneLib.cargoDoc crateArgs;

        examples = craneLib.buildPackage (crateArgs // {
          pname = "quasar-examples";
          cargoExtraArgs = "--examples";
        });
      }
    );

    checks = forAllSystems (system: {
      build = self.packages.${system}.default;
      examples = self.packages.${system}.examples;
      format = self.packages.${system}.checkFormat;
    });

    devShells = forAllSystems (system:
      let
        pkgs = import nixpkgs { inherit system; };
      in {
        default = pkgs.mkShell {
          packages = [
            pkgs.cargo
            pkgs.cargo-watch
            pkgs.rust-analyzer
            pkgs.rustfmt
          ];
        };
      }
    );
  };
}
