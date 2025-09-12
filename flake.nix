{
  description = "Development environment for Nomos.";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/02c80fc5421018016669d79765b40a18aaf3bd8d";
    rust-overlay = {
      url = "github:oxalica/rust-overlay/c448ab42002ac39d3337da10420c414fccfb1088";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = { self, nixpkgs, rust-overlay, ... }:
    let
      systems = [ "x86_64-linux" "aarch64-darwin" "x86_64-windows" ];
      forAll = fn: builtins.listToAttrs (map (system: { name = system; value = fn system; }) systems);

    in
    {
      devShells = forAll (system:
        let
          pkgs = import nixpkgs {
            inherit system;
            overlays = [ rust-overlay.overlays.default ];
          };

        in
        {
          default = self.devShells.${system}.research;
          research = pkgs.mkShell {
            name = "research";
            buildInputs = with pkgs; [
              pkg-config
              # Updating the version here requires also updating the `rev` version in the `overlays` section above
              # with a commit that contains the new version in its manifest
              rust-bin.stable."1.89.0".default
              clang_14
              llvmPackages_14.libclang
              openssl.dev
            ];
            shellHook = ''
              export LIBCLANG_PATH="${pkgs.llvmPackages_14.libclang.lib}/lib";
            '';
          };
          ci = pkgs.mkShell {
            name = "ci";
            buildInputs = with pkgs; [
              pkg-config
              rust-bin.stable."1.88.0".default
              openssl
              clang_14
              gmp                  # provides libgmp.so.10
              stdenv.cc.cc.lib     # provides libstdc++.so.6
              patchelf
            ];
            shellHook = ''
              # Tooling hints for clang/openssl
              export LIBCLANG_PATH="${pkgs.llvmPackages_14.libclang.lib}/lib"
              export PKG_CONFIG_PATH="${pkgs.openssl.dev}/lib/pkgconfig"

              # Cache locations (respect existing values)
              export CARGO_HOME="''${CARGO_HOME:-$HOME/.cargo}"
              export RUSTUP_HOME="''${RUSTUP_HOME:-$HOME/.rustup}"
              export RISC0_HOME="''${RISC0_HOME:-$HOME/.risc0}"
              mkdir -p "$CARGO_HOME" "$RUSTUP_HOME" "$RISC0_HOME"

              # Sanitize env that might leak from outer shells/CI
              unset CARGO_TARGET_DIR CARGO_BUILD_TARGET RUSTC_WRAPPER RUSTFLAGS RUSTC RUSTC_WORKSPACE_WRAPPER

              # RISC Zero knobs: build natively inside the devShell
              export RISC0_USE_DOCKER=0
              unset RISC0_SKIP_BUILD

              # Useful defaults
              export RUST_BACKTRACE="''${RUST_BACKTRACE:-1}"
              export CARGO_TERM_COLOR="''${CARGO_TERM_COLOR:-always}"

              # Keep only nix store entries on PATH, then prepend our cargo bin
              PATH="$(printf %s "$PATH" | tr ':' '\n' | grep '^/nix/store/' | paste -sd: - || true)"
              export PATH="$CARGO_HOME/bin:$PATH"

              # Debug (opt-in): set DEV_SHELL_DEBUG=1 to see details
              if [ -n "''${DEV_SHELL_DEBUG:-}" ]; then
                echo "=== devShell ==="

                echo "HOME=$HOME"
                echo "CARGO_HOME=$CARGO_HOME"
                echo "RUSTUP_HOME=$RUSTUP_HOME"
                echo "RISC0_HOME=$RISC0_HOME"

                echo "cargo: $(command -v cargo)";  command -v cargo  && cargo --version  || true
                echo "rustc: $(command -v rustc)";  command -v rustc  && rustc --version  || true
                echo "rustup: $(command -v rustup)"; command -v rustup && rustup --version || true
                echo "rzup: $(command -v rzup)";     command -v rzup   && rzup --version   || true

                IFS=: read -r -a PATH_PARTS <<< "$PATH"
                echo "PATH entries (''${#PATH_PARTS[@]}):"
                for i in "''${!PATH_PARTS[@]}"; do
                  printf '  %2d. %s\n' "$i" "''${PATH_PARTS[$i]}"
                done

                echo "=== /devShell ==="
              fi
            '';
          };
        }
      );
    };
}
