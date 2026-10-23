self:
{ config, lib, pkgs, ... }:

let
  cfg = config.services.nebula-client;
in
{
  options.services.nebula-client = {
    enable = lib.mkEnableOption "Nebula Client, a service that keeps this machine in Nebula networks";

    package = lib.mkOption {
      type = lib.types.package;
      default = self.packages.${pkgs.stdenv.hostPlatform.system}.default;
      defaultText = lib.literalExpression "nebula-client.packages.\${system}.default";
      description = "Nebula Client package.";
    };

    nebulaPackage = lib.mkOption {
      type = lib.types.package;
      default = pkgs.nebula;
      defaultText = lib.literalExpression "pkgs.nebula";
      description = "Nebula used by the service (taken from PATH instead of downloading).";
    };

    users = lib.mkOption {
      type = lib.types.listOf lib.types.str;
      default = [ ];
      example = [ "alice" ];
      description = "Users allowed to control the service from the window and the CLI (members of the nebula-client group).";
    };

    trustedInterfaces = lib.mkOption {
      type = lib.types.listOf lib.types.str;
      default = [ ];
      example = [ "nebula1" "nb-friends" ];
      description = ''
        Nebula adapters to add to networking.firewall.trustedInterfaces. Nebula has its own
        firewall from the panel; without this the NixOS firewall also filters incoming traffic
        from peers. Adapter names are shown by `nebula-client-service status`.
      '';
    };
  };

  config = lib.mkIf cfg.enable {
    environment.systemPackages = [ cfg.package ];
    users.groups.nebula-client.members = cfg.users;
    networking.firewall.trustedInterfaces = cfg.trustedInterfaces;

    systemd.services.nebula-client = {
      description = "Nebula Client";
      wantedBy = [ "multi-user.target" ];
      wants = [ "network-online.target" ];
      after = [ "network-online.target" ];
      path = [ cfg.nebulaPackage ];
      serviceConfig = {
        ExecStart = "${cfg.package}/bin/nebula-client-service run";
        Restart = "always";
        RestartSec = 5;
        StateDirectory = "nebula-client";
        StateDirectoryMode = "0700";
        RuntimeDirectory = "nebula-client";
        RuntimeDirectoryMode = "0755";
        KillMode = "mixed";
        TimeoutStopSec = 15;
      };
    };
  };
}
