{ pkgs ? import <nixpkgs> {
    builtins = [(import (fetchTarball {
      url    = "https://github.com/NixOS/nixpkgs/archive/e06c5e01088672bc460b2bc6b61d88e95190a492.tar.gz";
      sha256 = "sha256:e7d37547638aeb6b70a9dbf6dcc5970529edef39b46760a1c9689ac7f066ed58";
    }))];
    overlays = [
      (import (fetchGit {
        url = "https://github.com/oxalica/rust-overlay.git";
        rev = "8b81b8ed00b20fd57b24adcb390bd96ea81ecd90";
      }))
    ];
   }
}:

pkgs.mkShell {
  name = "nomos-research-build-shell";

  buildInputs = with pkgs; [
    pkg-config
    rust-bin.stable."1.85.1".default
    clang_14
    llvmPackages_14.libclang
    openssl.dev
  ];
  shellHook = ''
    export LIBCLANG_PATH="${pkgs.llvmPackages_14.libclang.lib}/lib";
  '';
}
