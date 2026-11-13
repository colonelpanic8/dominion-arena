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
  agentsFile = toml.generate "dominion-agents.toml" cfg.agents.settings;
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
    agents = {
      enable = lib.mkEnableOption ''
        the server's own agents: `dominion agents` runs as `user`, with that account's
        Claude Code and Codex logins, and plays a seat for each agent registered players
        seat (`agent_seats_per_day` each a day, `agents_at_once` at once). They use that
        account's subscriptions'';
      user = lib.mkOption {
        type = lib.types.str;
        example = "alice";
        description = "The account the agents run as: its `claude` and `codex` logins play.";
      };
      settings = lib.mkOption {
        type = toml.type;
        default = {};
        example = {
          agents_at_once = 1;
          agents = [
            {
              name = "Claude Opus";
              program = "claude";
              model = "opus";
            }
          ];
        };
        description = ''
          The runner's settings file (`agents`, `agents_at_once`, `agents_dir`). It
          connects to the server's `agent_socket`, and its agents to the server's MCP
          endpoint on loopback.
        '';
      };
      path = lib.mkOption {
        type = lib.types.listOf lib.types.package;
        default = [];
        description = "Packages put on the runner's PATH ahead of `user`'s profile, where `claude` and `codex` are looked for.";
      };
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
      agent_socket = lib.mkIf cfg.agents.enable (lib.mkDefault "/run/dominion/agents.sock");
    };
    services.dominion.agents.settings = lib.mkIf cfg.agents.enable {
      agent_socket = lib.mkDefault cfg.settings.agent_socket;
      server = lib.mkDefault "http://127.0.0.1:${toString (cfg.settings.port or 7317)}/mcp";
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
        # The agent socket's: the group may reach it.
        RuntimeDirectory = "dominion";
        RuntimeDirectoryMode = "0750";
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

    # Unhardened: Claude Code and Codex need the user's home, and Node a JIT. Stopping it
    # stops the agents; when it starts again it is told their seats and starts them again.
    systemd.services.dominion-agents = lib.mkIf cfg.agents.enable {
      description = "Dominion server's agents";
      wantedBy = ["multi-user.target"];
      after = ["dominion.service" "network-online.target"];
      wants = ["network-online.target"];
      environment.PATH = lib.mkForce (lib.concatStringsSep ":" (
        lib.optional (cfg.agents.path != []) (lib.makeBinPath cfg.agents.path)
        ++ [
          "/etc/profiles/per-user/${cfg.agents.user}/bin"
          "/run/current-system/sw/bin"
        ]
      ));
      serviceConfig = {
        ExecStart = "${lib.getExe cfg.package} --config ${agentsFile} agents";
        User = cfg.agents.user;
        SupplementaryGroups = ["dominion"];
        Restart = "always";
        RestartSec = 10;
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
