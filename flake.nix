{
  description = "Reproducible development environment for the File Tunnel backend API";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
  inputs.ores-sops.url = "github:ORESoftware/ores-sops";

  outputs =
    { nixpkgs, ores-sops, ... }:
    let
      systems = [
        "aarch64-darwin"
        "aarch64-linux"
        "x86_64-darwin"
        "x86_64-linux"
      ];
      forAllSystems = nixpkgs.lib.genAttrs systems;
      pkgsFor =
        system:
        import nixpkgs {
          inherit system;
          overlays = [ ores-sops.overlays.default ];
        };
      mkAgentCheck =
        pkgs:
        pkgs.writeShellApplication {
          name = "agent-check";
          runtimeInputs = [
            pkgs.bash
            pkgs.coreutils
            pkgs.git
          ];
          text = builtins.readFile ./.nix/agent-check.sh;
        };
    in
    {
      formatter = forAllSystems (system: (pkgsFor system).nixfmt);

      packages = forAllSystems (
        system:
        let
          agentCheck = mkAgentCheck (pkgsFor system);
        in
        {
          agent-check = agentCheck;
          default = agentCheck;
        }
      );

      apps = forAllSystems (system: {
        agent-check = {
          type = "app";
          program = nixpkgs.lib.getExe (mkAgentCheck nixpkgs.legacyPackages.${system});
        };
      });

      checks = forAllSystems (system: {
        agent-check = mkAgentCheck nixpkgs.legacyPackages.${system};
      });

      devShells = forAllSystems (
        system:
        let
          pkgs = pkgsFor system;
        in
        {
          default = import ./.nix/devshell.nix {
            inherit pkgs;
            agentCheck = mkAgentCheck pkgs;
          };
        }
      );
    };
}
