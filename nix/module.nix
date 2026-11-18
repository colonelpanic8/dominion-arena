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
  credentials = "/run/credentials/dominion.service";
  accountCredentials = encrypted:
    lib.mapAttrsToList (id: a: "online-password-${id}:${
      if encrypted
      then a.encryptedCredential
      else a.passwordFile
    }")
    (lib.filterAttrs (_: a:
      (
        if encrypted
        then a.encryptedCredential
        else a.passwordFile
      )
      != null)
    cfg.onlineAccounts);
  clis =
    cfg.agents.path
    ++ lib.filter (p: p != null) [cfg.agents.claude cfg.agents.codex cfg.agents.antigravity];
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
    onlinePasswordCredential = lib.mkOption {
      type = lib.types.nullOr lib.types.str;
      default = null;
      description = "Path to a systemd-creds encrypted password on the host, loaded only at runtime.";
    };
    onlineAccounts = lib.mkOption {
      default = {};
      example = lib.literalExpression ''
        {
          sol = {
            account = "SolDominion";
            owner = "Arena Matches";
            exclusive = true;
            passwordFile = config.age.secrets.dominion-online-sol.path;
            settings.resume = true;
          };
        }
      '';
      description = ''
        More official dominion.games accounts kept by the server, by id (lowercase letters,
        digits and `-`; routes `/online/<id>`, `/api/online/<id>/...`, `/mcp/online/<id>`).
        Each becomes an `[[online_accounts]]` table whose password is the service credential
        `online-password-<id>`, loaded from `passwordFile` (a root-readable runtime file,
        such as an agenix secret) or `encryptedCredential` (systemd-creds, whose embedded
        name must be `online-password-<id>`). Neither enters the Nix store.
      '';
      type = lib.types.attrsOf (lib.types.submodule {
        options = {
          account = lib.mkOption {
            type = lib.types.str;
            description = "The official-server account name.";
          };
          owner = lib.mkOption {
            type = lib.types.str;
            description = "The registered Arena account that manages it.";
          };
          exclusive = lib.mkOption {
            type = lib.types.bool;
            description = "Your assertion that the account is idle and used only by this server; it must be true.";
          };
          passwordFile = lib.mkOption {
            type = lib.types.nullOr lib.types.str;
            default = null;
            description = "Runtime file whose first line is the password (loaded with `LoadCredential`).";
          };
          encryptedCredential = lib.mkOption {
            type = lib.types.nullOr lib.types.str;
            default = null;
            description = "systemd-creds encrypted password (loaded with `LoadCredentialEncrypted`).";
          };
          settings = lib.mkOption {
            type = toml.type;
            default = {};
            example = {resume = true;};
            description = "Other keys of its `[[online_accounts]]` table: `resume`, `endpoint`, `state_dir`, `enabled`.";
          };
        };
      });
    };
    agents = {
      enable = lib.mkEnableOption ''
        the server's own agents: `dominion agents` runs as `user`, with that account's
        Claude Code, Codex, and Antigravity CLI logins, and plays a seat for each agent registered players (`agent_players`, and their bots)
        seat (`agent_seats_per_day` each a day, 0 for no limit; `agents_at_once` at once). They use that
        account's subscriptions'';
      user = lib.mkOption {
        type = lib.types.str;
        example = "alice";
        description = "The account the agents run as: its native CLI logins play.";
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
        description = "Packages put on the runner's PATH ahead of `user`'s profile, where native CLIs are looked for.";
      };
      claude = lib.mkOption {
        type = lib.types.nullOr lib.types.package;
        default = null;
        example = lib.literalExpression "pkgs.claude-code";
        description = "Claude Code, for `claude` agents (default: `claude` on `user`'s PATH).";
      };
      codex = lib.mkOption {
        type = lib.types.nullOr lib.types.package;
        default = null;
        example = lib.literalExpression "pkgs.codex";
        description = "Codex, for `codex` agents (default: `codex` on `user`'s PATH). `openai` agents need no CLI: they use Codex's ChatGPT login in `~/.codex`.";
      };
      antigravity = lib.mkOption {
        type = lib.types.nullOr lib.types.package;
        default = null;
        example = lib.literalExpression "pkgs.antigravity-cli";
        description = "The Antigravity CLI (`agy`), for `antigravity` agents (default: `agy` on `user`'s PATH).";
      };
      playbook = lib.mkOption {
        type = lib.types.nullOr lib.types.path;
        default = self.inputs.dominion-playbook;
        defaultText = lib.literalExpression "dominion-arena.inputs.dominion-playbook";
        description = "The dominion-playbook the agents start in (`playbook_dir`); null sends them none.";
      };
      profiles = lib.mkOption {
        type = lib.types.listOf toml.type;
        default = import ./agents.nix;
        defaultText = lib.literalExpression "import ./nix/agents.nix";
        description = "The agent profiles offered to players: the runner's `[[agents]]`.";
      };
      catalogs = lib.mkOption {
        type = lib.types.listOf toml.type;
        default = import ./catalogs.nix;
        defaultText = lib.literalExpression "import ./nix/catalogs.nix";
        description = ''
          Providers whose models are offered to players, each at every effort allowed: the
          runner's `[[catalogs]]`. New models appear as the providers list them; a profile
          of the same program, model and effort is offered in its place.
        '';
      };
      atOnce = lib.mkOption {
        type = lib.types.ints.positive;
        default = 2;
        description = "Agent seats played at once (`agents_at_once`).";
      };
      anthropic = {
        apiKeyCredential = lib.mkOption {
          type = lib.types.nullOr lib.types.str;
          default = null;
          example = "/var/lib/private/dominion-anthropic-api-key.cred";
          description = ''
            systemd-creds encrypted Anthropic API key (credential name `anthropic-api-key`),
            for `anthropic` agents: the runner gets it as `ANTHROPIC_API_KEY`. The CLI
            seats' environment is cleared, so it never reaches them.
          '';
        };
        workspace = lib.mkOption {
          type = lib.types.nullOr lib.types.str;
          default = null;
          example = "wrkspc_01...";
          description = "`ANTHROPIC_WORKSPACE_ID`: the workspace a key not scoped to one bills to.";
        };
      };
    };
    syncAssets = lib.mkEnableOption ''
      downloading dominion.games' card art, fonts and sounds into `assets_dir` before the
      server starts (`dominion assets sync`). They belong to their owners: this is for a
      server only you and yours reach'';
  };

  config = lib.mkIf cfg.enable {
    assertions = lib.flatten (lib.mapAttrsToList (id: a: [
        {
          assertion = builtins.match "[a-z0-9-]+" id != null && id != "default";
          message = "services.dominion.onlineAccounts.${id}: use lowercase letters, digits and -, not default";
        }
        {
          assertion = (a.passwordFile == null) != (a.encryptedCredential == null);
          message = "services.dominion.onlineAccounts.${id}: set exactly one of passwordFile or encryptedCredential";
        }
      ])
      cfg.onlineAccounts);
    services.dominion.settings = {
      state_dir = lib.mkDefault "/var/lib/dominion/games";
      assets_dir = lib.mkDefault "/var/cache/dominion/assets";
      agent_socket = lib.mkIf cfg.agents.enable (lib.mkDefault "/run/dominion/agents.sock");
      control_socket = lib.mkDefault "/run/dominion/control.sock";
      online_state_dir = lib.mkDefault "/var/lib/dominion/online";
      online_cache_dir = lib.mkDefault "/var/cache/dominion/online";
      online_password_file =
        lib.mkIf (cfg.onlinePasswordCredential != null)
        (lib.mkDefault "${credentials}/online-password");
      online_accounts = lib.mkIf (cfg.onlineAccounts != {}) (lib.mapAttrsToList (id: a:
        {
          inherit id;
          inherit (a) account owner exclusive;
          password_file = "${credentials}/online-password-${id}";
        }
        // a.settings)
      cfg.onlineAccounts);
    };
    services.dominion.agents.settings = lib.mkIf cfg.agents.enable ({
        agent_socket = lib.mkDefault cfg.settings.agent_socket;
        server = lib.mkDefault "http://127.0.0.1:${toString (cfg.settings.port or 7317)}/mcp";
        agents = lib.mkDefault cfg.agents.profiles;
        catalogs = lib.mkDefault cfg.agents.catalogs;
        agents_at_once = lib.mkDefault cfg.agents.atOnce;
        # What the agents' sandbox has on PATH: a shell for their lookups, and python3
        # for the playbook's ratings script.
        sandbox_path = lib.mkDefault (map (p: "${lib.getBin p}/bin") (with pkgs; [
          bashNonInteractive
          coreutils
          findutils
          gnugrep
          gnused
          python3
        ]));
      }
      // lib.optionalAttrs (cfg.agents.playbook != null) {
        playbook_dir = lib.mkDefault "${cfg.agents.playbook}";
      });

    users.users.dominion = {
      isSystemUser = true;
      group = "dominion";
    };
    users.groups.dominion = {};

    # `sudo dominion-games list` / `retire ID...`: the control socket is the server user's.
    environment.systemPackages = [
      (pkgs.writeShellScriptBin "dominion-games" ''
        exec ${pkgs.util-linux}/bin/runuser -u dominion -- ${dominion} games "$@"
      '')
    ];

    systemd.services.dominion = {
      description = "Dominion game server";
      wantedBy = ["multi-user.target"];
      after = ["network.target"];
      serviceConfig = {
        ExecStart = "${dominion} serve";
        ExecStartPre =
          lib.optional ((cfg.settings.online_enabled or false) || (cfg.settings.online_accounts or []) != [])
          "${dominion} online sync";
        LoadCredentialEncrypted =
          lib.optional (cfg.onlinePasswordCredential != null)
          "online-password:${cfg.onlinePasswordCredential}"
          ++ accountCredentials true;
        LoadCredential = accountCredentials false;
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
      environment.ANTHROPIC_WORKSPACE_ID = lib.mkIf (cfg.agents.anthropic.workspace != null) cfg.agents.anthropic.workspace;
      environment.PATH = lib.mkForce (lib.concatStringsSep ":" (
        lib.optional (clis != []) (lib.makeBinPath clis)
        ++ [
          (lib.makeBinPath [pkgs.bubblewrap])
          "/etc/profiles/per-user/${cfg.agents.user}/bin"
          "/run/current-system/sw/bin"
        ]
      ));
      serviceConfig = {
        ExecStart =
          if cfg.agents.anthropic.apiKeyCredential == null
          then "${lib.getExe cfg.package} --config ${agentsFile} agents"
          else
            pkgs.writeShellScript "dominion-agents" ''
              ANTHROPIC_API_KEY="$(< "$CREDENTIALS_DIRECTORY/anthropic-api-key")"
              export ANTHROPIC_API_KEY
              exec ${lib.getExe cfg.package} --config ${agentsFile} agents
            '';
        LoadCredentialEncrypted =
          lib.optional (cfg.agents.anthropic.apiKeyCredential != null)
          "anthropic-api-key:${cfg.agents.anthropic.apiKeyCredential}";
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
