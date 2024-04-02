{
  inputs = {
    cargo2nix.url = "github:cargo2nix/cargo2nix/release-0.11.0";
    flake-utils.follows = "cargo2nix/flake-utils";
    nixpkgs.follows = "cargo2nix/nixpkgs";
  };

  outputs = inputs: with inputs;
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs {
          inherit system;
          overlays = [cargo2nix.overlays.default];
        };

        rustPkgs = pkgs.rustBuilder.makePackageSet {
          rustVersion = "1.75.0";
          packageFun = import ./Cargo.nix;
        };

        nonRustDeps = [
            pkgs.libiconv
          ];
        rust-toolchain = pkgs.symlinkJoin {
            name = "rust-toolchain";
            paths = [ pkgs.rustfmt pkgs.rustc pkgs.cargo pkgs.cargo-watch pkgs.rust-analyzer pkgs.rustPlatform.rustcSrc ];
        };

      in rec {
        # defaultPackage = rustPkgs.workspace.wwfv {};
        devShells.default = pkgs.mkShell {
            shellHook = ''
              # For rust-analyzer 'hover' tooltips to work.
              export RUST_SRC_PATH=${pkgs.rustPlatform.rustLibSrc}
            '';
            # LD_LIBRARY_PATH = "${pkgs.stdenv.cc.cc.lib}/lib";
            buildInputs = nonRustDeps;
            nativeBuildInputs = with pkgs; [
              rust-toolchain
              netcat
            ];
            RUST_BACKTRACE = 1;
        };
            
      }
    );
}