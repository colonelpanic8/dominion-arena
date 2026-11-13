# `services.dominion`: `dominion serve` as a system service, its settings file made from
# `settings`.
self: {
  config,
  lib,
  pkgs,
  ...
}: let
  cfg = config.services.dominion;
  toml = pkgs.formats.toml {};
  configFile = toml.generate "dominion-config.toml" cfg.settings;
  dominion = "${lib.getExe cfg.package} --config ${configFile}";
in {
  options.services.dominion = {
    enable = lib.mkEnableOption "the Dominion game server (`dominion serve`)";
    package = lib.mkOption {
      type = lib.types.package;
      default = self.packages.${pkgs.stdenv.hostPlatform.system}.dominion-with-web;
      defaultText = lib.literalExpression "dominion-arena.packages.\${system}.dominion-with-web";
      description = "The `dominion` to run; the default serves the web client too.";
    };
    settings = lib.mkOption {
      type = toml.type;
      default = {};
      example = {
        port = 7317;
        public_url = "https://dominion.example";
        trust_proxy = true;
      };
      description = ''
        The settings file, key by key (the README lists them). `state_dir` and
        `assets_dir` default to the service's own directories under `/var/lib` and
        `/var/cache`.
      '';
    };
    syncAssets = lib.mkEnableOption ''
      downloading dominion.games' card art, fonts and sounds into `assets_dir` before the
      server starts (`dominion assets sync`). They belong to their owners: this is for a
      server only you and yours reach'';
  };

  config = lib.mkIf cfg.enable {
    services.dominion.settings = {
      state_dir = lib.mkDefault "/var/lib/dominion/games";
      assets_dir = lib.mkDefault "/var/cache/dominion/assets";
    };

    users.users.dominion = {
      isSystemUser = true;
      group = "dominion";
    };
    users.groups.dominion = {};

    systemd.services.dominion = {
      description = "Dominion game server";
      wantedBy = ["multi-user.target"];
      after = ["network.target"];
      serviceConfig = {
        ExecStart = "${dominion} serve";
        User = "dominion";
        Group = "dominion";
        StateDirectory = "dominion";
        StateDirectoryMode = "0700";
        CacheDirectory = "dominion";
        Restart = "on-failure";
        RestartSec = 5;
        NoNewPrivileges = true;
        ProtectSystem = "strict";
        ProtectHome = true;
        PrivateTmp = true;
        PrivateDevices = true;
        ProtectKernelTunables = true;
        ProtectKernelModules = true;
        ProtectControlGroups = true;
        RestrictAddressFamilies = ["AF_INET" "AF_INET6" "AF_UNIX"];
        RestrictNamespaces = true;
        LockPersonality = true;
        MemoryDenyWriteExecute = true;
        SystemCallArchitectures = "native";
      };
    };

    # Nothing waits for it: the server serves each file once it is there. It runs at
    # every boot and activation, fetching only what the cache lacks.
    systemd.services.dominion-assets = lib.mkIf cfg.syncAssets {
      description = "Sync the Dominion server's asset cache";
      wantedBy = ["multi-user.target"];
      after = ["network-online.target"];
      wants = ["network-online.target"];
      serviceConfig = {
        ExecStart = "${dominion} assets sync";
        User = "dominion";
        Group = "dominion";
        CacheDirectory = "dominion";
        NoNewPrivileges = true;
        ProtectSystem = "strict";
        ProtectHome = true;
        PrivateTmp = true;
      };
    };
  };
}
