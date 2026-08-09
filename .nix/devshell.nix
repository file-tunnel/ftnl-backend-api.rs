{ pkgs, agentCheck }:
pkgs.mkShell {
  packages = [
    agentCheck
  ]
  ++ (with pkgs; [
    actionlint
    age
    cargo
    clippy
    git
    just
    jq
    jdk21_headless
    nixfmt
    nodejs_22
    openssl
    pkg-config
    pkgs.ores-sops
    ripgrep
    rust-analyzer
    rustc
    rustfmt
    shellcheck
    shfmt
    sops
  ]);

  LANG = if pkgs.stdenv.hostPlatform.isDarwin then "en_US.UTF-8" else "C.UTF-8";
  LC_ALL = if pkgs.stdenv.hostPlatform.isDarwin then "en_US.UTF-8" else "C.UTF-8";

  shellHook = ''
    export FTNL_DEV_SHELL="backend-api"
    export XDG_CACHE_HOME="''${XDG_CACHE_HOME:-$PWD/.cache/nix-agent}"
  '';
}
