use clap::{ArgGroup, Args, Parser, Subcommand};
use dominion_server::Options;
use dominion_server::config::Config;
use dominion_server::hub::{Hub, NewGame};
use dominion_server::solo::{self, Solo};
use std::collections::BTreeMap;
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

/// Play Dominion through dominion/play@1 (agent-view@2).
#[derive(Parser)]
#[command(name = "dominion", version)]
struct Cli {
    /// Settings file (default $XDG_CONFIG_HOME/dominion/config.toml).
    #[arg(long, global = true)]
    config: Option<PathBuf>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Browser-free official dominion.games backend (separate from hosted Arena games).
    Online {
        #[command(subcommand)]
        command: OnlineCommand,
    },
    /// Host games as an MCP server (streamable HTTP at /mcp), with an HTTP API at /api
    /// for the web client.
    Serve {
        #[arg(long)]
        listen: Option<IpAddr>,
        #[arg(long)]
        port: Option<u16>,
        /// Directory to save games in and resume them from.
        #[arg(long, conflicts_with = "ephemeral")]
        state_dir: Option<PathBuf>,
        /// Keep games in memory only.
        #[arg(long)]
        ephemeral: bool,
        /// Directory of the asset cache, served at /assets.
        #[arg(long)]
        assets_dir: Option<PathBuf>,
        /// Directory of the built web client, served at / (default: web_dir, then $DOMINION_WEB_DIR).
        #[arg(long)]
        web_dir: Option<PathBuf>,
        /// What new games' recorded event streams are written in: jsonl or cbor
        /// (default: events_encoding, then jsonl).
        #[arg(long)]
        events_encoding: Option<dominion_agent::recording::Encoding>,
        /// Whether new games are recorded as event streams: true or false
        /// (default: record_events, then true).
        #[arg(long)]
        record_events: Option<bool>,
    },
    /// Play one seat from the terminal against a server.
    Play(PlayArgs),
    /// Print cards' official text.
    Card {
        #[arg(required = true)]
        names: Vec<String>,
    },
    /// Play the server's own agents on this machine: connect to its `agent_socket` and
    /// run Claude Code, Codex or T3 Code for every seat they hold.
    Agents,
    /// List or retire the games a running `dominion serve` hosts, or void finished games'
    /// ratings, through its `control_socket`.
    Games {
        /// The server's control socket (default from the config file).
        #[arg(long)]
        socket: Option<PathBuf>,
        #[command(subcommand)]
        command: GamesCommand,
    },
    /// Authorize the runner with T3 Code; print the access token to stdout for a credential manager.
    T3Login,
    /// Inside an agent's sandbox: serve each `PORT=SOCKET` on loopback, passing connections
    /// to the unix socket, and run the command. The runner starts this itself.
    #[command(hide = true)]
    SandboxForward {
        #[arg(long = "port", value_name = "PORT=SOCKET")]
        ports: Vec<String>,
        #[arg(last = true, required = true)]
        command: Vec<String>,
    },
    /// Manage the local cache of dominion.games' images, fonts and sounds.
    Assets {
        #[command(subcommand)]
        command: AssetsCommand,
    },
    /// Play a deck out alone, in a game whose other seats sit idle.
    Solo(SoloArgs),
}

#[derive(Subcommand)]
enum GamesCommand {
    /// Every hosted game: id, finished or not, idle time, where it came from, players.
    List,
    /// Stop hosting these games now, moving their files to `expired/` as idle expiry
    /// does. Unfinished games of a lobby table or the server's agents are refused.
    Retire {
        #[arg(required = true)]
        games: Vec<String>,
    },
    /// Void finished rated games: take them out of the record (they stay in the history,
    /// marked voided) and off their players' tallies, and make every rating what the other rated games make it,
    /// played again in the order they ended. Refused unless the records give exactly
    /// the ratings kept now.
    Void {
        #[arg(required = true)]
        games: Vec<String>,
        /// Say what would change, and change nothing.
        #[arg(long)]
        dry_run: bool,
        /// Only mark games voided that an earlier void made unrated before voids were
        /// kept in the record, and take them off their players' tallies; no rating changes.
        #[arg(long)]
        record_only: bool,
    },
    /// Drain official accounts before a restart: they start no new games, and their
    /// running games play to the end (a restart mid-game costs the seat its clock).
    /// Lists the games still running; with --wait, returns once there are none. The
    /// restarted server starts undrained.
    Drain {
        /// Wait until no official game is running.
        #[arg(long)]
        wait: bool,
        /// Stop draining.
        #[arg(long, conflicts_with = "wait")]
        cancel: bool,
    },
    /// Write dominion/events@1 streams for games saved before the server recorded them,
    /// here and in `expired/`, checking each against its snapshot first. Run it with the
    /// server stopped: it takes the state directory's lock (a dry run doesn't).
    Backfill {
        /// Convert and check every game, report, and write nothing.
        #[arg(long)]
        dry_run: bool,
        /// The server's state directory (default: state_dir).
        #[arg(long)]
        state_dir: Option<PathBuf>,
        /// What streams are written in: jsonl or cbor (default: events_encoding, then
        /// jsonl).
        #[arg(long)]
        events_encoding: Option<dominion_agent::recording::Encoding>,
    },
}

#[derive(Subcommand)]
enum OnlineCommand {
    /// Download and verify the supported public client catalog.
    Sync,
    /// Query the official server states without authentication.
    Probe,
    /// List the configured official accounts without connecting.
    Accounts,
    /// Check password login and 31 seconds of application heartbeat, then disconnect.
    Login {
        /// The account's id (default: the `online_*` one, else the only one).
        #[arg(long)]
        account: Option<String>,
    },
    /// Start a private unrated game between two configured accounts, report the table,
    /// timer and both clocks while nobody moves, then end it and disconnect both. Both
    /// accounts must be idle and not kept by a running server.
    PairCheck {
        /// The account that opens the table (its id).
        #[arg(long)]
        host: String,
        /// The account that joins it (its id).
        #[arg(long)]
        guest: String,
        /// Timer preset: 0 Reckless, 1 Hasty, 2 Friendly, 3 Patient, 4 Off.
        #[arg(long, default_value_t = 0)]
        timer: u8,
        /// Required Kingdom piles, comma-separated.
        #[arg(long, value_delimiter = ',')]
        kingdom: Vec<String>,
        /// How long to watch before the host resigns, in seconds.
        #[arg(long, default_value_t = 20)]
        watch: u64,
        /// Leave the game running instead of resigning: watch it until dominion.games
        /// ends it (a clock running out) or `--watch` runs out.
        #[arg(long)]
        idle: bool,
    },
    /// Serve dominion/play@1 over stdio for the configured exclusive account.
    Mcp {
        /// The account's id (default: the `online_*` one, else the only one).
        #[arg(long)]
        account: Option<String>,
    },
}

#[derive(Subcommand)]
enum AssetsCommand {
    /// Download the assets and write the manifest mapping cards to them.
    Sync {
        /// Cache directory (default from the config file).
        #[arg(long)]
        dir: Option<PathBuf>,
        /// Download files already in the cache again.
        #[arg(long)]
        force: bool,
        /// Report what would be downloaded, fetching only the client's bundles.
        #[arg(long)]
        dry_run: bool,
    },
    /// Print the cache directory.
    Path,
}

#[derive(Args)]
struct SoloArgs {
    /// Which solo game; each name is its own game.
    #[arg(long, short, global = true, default_value = "solo")]
    game: String,
    /// Directory the games are kept in (default from the config file, else
    /// $XDG_STATE_HOME/dominion/solo).
    #[arg(long, global = true)]
    dir: Option<PathBuf>,
    #[command(subcommand)]
    command: SoloCommand,
}

#[derive(Subcommand)]
enum SoloCommand {
    /// Start the game, replacing one of the same name.
    New {
        /// Kingdom piles, comma separated.
        #[arg(long, value_delimiter = ',', required = true)]
        kingdom: Vec<String>,
        /// Events, Projects, Ways, Landmarks, Traits, an Ally or Prophecy, comma separated.
        #[arg(long, value_delimiter = ',')]
        landscapes: Vec<String>,
        /// A setup choice as CARD=CHOICE[,CHOICE...]: "Young Witch=Cellar" (its Bane),
        /// "Reckless=Smithy" (a Trait's pile), "Way of the Mouse=Crossroads". Repeatable.
        #[arg(long, value_name = "CARD=CHOICE")]
        setup: Vec<String>,
        /// Use Platinum and Colony.
        #[arg(long)]
        colonies: bool,
        /// Start with Shelters.
        #[arg(long)]
        shelters: bool,
        /// Seats in all; every seat but yours sits idle (default 2).
        #[arg(long, default_value_t = 2)]
        players: u8,
        /// The shuffle seed (default: random, and printed).
        #[arg(long)]
        seed: Option<u64>,
    },
    /// Do steps in order, separated by `;`, stopping at the first that can't be done:
    /// `play A, B` · `treasures` (play every Treasure in hand) · `buy A, B` · `pay` (off
    /// Debt) · `coffers N` · `villagers N` · `end` (the phase) · `next` (end the turn) ·
    /// `pick A, B x2` or `pick none` (answer any question by card, option text or option
    /// number) · `name CARD`.
    Do {
        #[arg(required = true)]
        steps: Vec<String>,
        /// Also print the Supply, the landscapes and the idle seats.
        #[arg(long)]
        full: bool,
    },
    /// Show the game as it stands.
    Show {
        /// Print the whole log first.
        #[arg(long)]
        log: bool,
    },
    /// Take back the last `do`.
    Undo,
}

fn solo(args: SoloArgs, config: &Config) -> Result<(), String> {
    let dir = args
        .dir
        .or_else(|| config.solo_dir())
        .ok_or("no place to keep solo games: pass --dir, or set solo_dir or XDG_STATE_HOME")?;
    let path = solo::path(&dir, &args.game)?;
    let show = |solo: &Solo, log: &[String], brief: bool| {
        if !log.is_empty() {
            println!("LOG");
            log.iter().for_each(|line| println!("  {line}"));
        }
        println!("{}", solo.text(brief));
    };
    match args.command {
        SoloCommand::New {
            kingdom,
            landscapes,
            setup,
            colonies,
            shelters,
            players,
            seed,
        } => {
            let setup = setup
                .iter()
                .map(|s| {
                    let (by, chosen) = s
                        .split_once('=')
                        .ok_or_else(|| format!("--setup {s:?} is not CARD=CHOICE"))?;
                    let chosen = chosen.split(',').map(|c| c.trim().to_string()).collect();
                    Ok((by.trim().to_string(), chosen))
                })
                .collect::<Result<BTreeMap<String, Vec<String>>, String>>()?;
            let game = Solo::new(
                &args.game,
                &NewGame {
                    players,
                    kingdom,
                    landscapes,
                    seed,
                    colonies: Some(colonies),
                    shelters: Some(shelters),
                    setup,
                    ..NewGame::default()
                },
            )?;
            solo::save(&path, &game)?;
            println!("solo game {:?}, seed {}", args.game, game.seed());
            show(&game, game.log(), false);
            Ok(())
        }
        SoloCommand::Do { steps, full } => {
            let mut game = solo::load(&path)?;
            let outcome = game.run(&steps.join(" "))?;
            solo::save(&path, &game)?;
            show(&game, &outcome.log, !full);
            match outcome.stopped {
                None => Ok(()),
                Some((step, why)) => Err(format!(
                    "stopped at {step:?}: {why}. The steps before it stand; the state above \
                     is current"
                )),
            }
        }
        SoloCommand::Show { log } => {
            let game = solo::load(&path)?;
            show(&game, if log { game.log() } else { &[] }, false);
            Ok(())
        }
        SoloCommand::Undo => {
            let mut game = solo::load(&path)?;
            game.undo()?;
            solo::save(&path, &game)?;
            show(&game, &[], true);
            Ok(())
        }
    }
}

async fn games(
    socket: Option<PathBuf>,
    command: GamesCommand,
    config: &Config,
) -> Result<(), String> {
    use dominion_server::control::{Reply, Request, request};
    let socket = (socket.or_else(|| config.control_socket.clone()))
        .ok_or("no control socket: pass --socket, or set control_socket")?;
    let asked = match command {
        GamesCommand::List => Request::List,
        GamesCommand::Retire { games } => Request::Retire { games },
        GamesCommand::Void {
            games,
            dry_run,
            record_only,
        } => Request::Void {
            games,
            dry_run,
            record_only,
        },
        GamesCommand::Drain { wait, cancel } => loop {
            let reply = request(&socket, &Request::Drain { cancel }).await?;
            let Reply::Draining { draining, running } = reply else {
                return Err(format!("unexpected reply {reply:?}"));
            };
            if !draining {
                println!("not draining");
                return Ok(());
            }
            for (account, game) in &running {
                println!("{account}: game {game} running");
            }
            if running.is_empty() {
                println!("drained: no official game running");
                return Ok(());
            }
            if !wait {
                return Ok(());
            }
            tokio::time::sleep(std::time::Duration::from_secs(10)).await;
        },
        GamesCommand::Backfill { .. } => unreachable!("run without the server"),
    };
    match request(&socket, &asked).await? {
        Reply::Games { games } => {
            for g in games {
                let mut notes = vec![];
                notes.extend(g.table.map(|t| format!("table {t}")));
                if g.agents {
                    notes.push("agents".into());
                }
                if g.connected {
                    notes.push("connected".into());
                }
                println!(
                    "{:<6} {:<8} idle {:>7}  {:<7} {}{}",
                    g.id,
                    if g.running { "running" } else { "over" },
                    idle(g.idle_seconds),
                    format!("{:?}", g.source).to_lowercase(),
                    g.names.join(" vs "),
                    if notes.is_empty() {
                        String::new()
                    } else {
                        format!("  [{}]", notes.join(", "))
                    },
                );
            }
            Ok(())
        }
        Reply::Retired { retired, refused } => {
            if !retired.is_empty() {
                println!("retired {}", retired.join(" "));
            }
            for (id, why) in &refused {
                eprintln!("{id}: {why}");
            }
            if refused.is_empty() {
                Ok(())
            } else {
                Err(format!("{} not retired", refused.len()))
            }
        }
        Reply::Voided { voided, dry_run } => {
            let verb = if dry_run { "would void" } else { "voided" };
            println!("{verb} {}", voided.games.join(" "));
            for (name, before, after) in &voided.ratings {
                println!("  {name:<24} {before:>8.1} -> {after:>8.1}");
            }
            for (name, games, wins) in &voided.tallies {
                println!("  {name:<24} games -{games}, wins -{wins}");
            }
            Ok(())
        }
        Reply::Draining { .. } => unreachable!("drained above"),
        Reply::Error { message } => Err(message),
    }
}

fn idle(seconds: u64) -> String {
    match seconds {
        s if s < 3600 => format!("{}m", s / 60),
        s if s < 86_400 => format!("{}h{:02}m", s / 3600, s % 3600 / 60),
        s => format!("{}d{:02}h", s / 86_400, s % 86_400 / 3600),
    }
}

#[derive(Args)]
#[command(group(ArgGroup::new("seat").required(true).args(["new", "game"])))]
struct PlayArgs {
    /// The server's MCP endpoint (default from the config file, else the local server).
    #[arg(long)]
    server: Option<String>,
    /// Seconds to keep trying to reach a server that went away (default 30).
    #[arg(long, value_name = "SECONDS")]
    reconnect: Option<u32>,
    /// Create a new game and take its first seat.
    #[arg(long, conflicts_with_all = ["game", "token"])]
    new: bool,
    /// Join this game (with --token).
    #[arg(long, requires = "token")]
    game: Option<String>,
    /// The seat token for --game.
    #[arg(long, requires = "game")]
    token: Option<String>,
    /// Number of players in the new game (default 2).
    #[arg(long, conflicts_with = "game")]
    players: Option<u8>,
    /// Kingdom cards of the new game, comma separated (default: a random kingdom).
    #[arg(long, value_delimiter = ',', conflicts_with = "game")]
    kingdom: Vec<String>,
    /// Sets to deal the new game's random kingdom from, comma separated.
    #[arg(long, value_delimiter = ',', conflicts_with_all = ["game", "kingdom"])]
    sets: Vec<String>,
    #[arg(long, conflicts_with = "game")]
    seed: Option<u64>,
    #[arg(long, conflicts_with = "game")]
    turn_limit: Option<u32>,
    /// Seat names of the new game, comma separated.
    #[arg(long, value_delimiter = ',', conflicts_with = "game")]
    names: Vec<String>,
}

async fn serve(config: Config) -> Result<(), String> {
    let addr = SocketAddr::new(config.listen(), config.port());
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|e| format!("listening on {addr}: {e}"))?;
    // The port asked for may be 0: any free one.
    let addr = listener.local_addr().map_err(|e| e.to_string())?;
    let mut hosts = config.allowed_hosts.clone();
    if !config.listen().is_loopback() && !config.listen().is_unspecified() {
        hosts.push(config.listen().to_string());
    }
    hosts.extend(config.public_host());
    let hub = match config.state_dir() {
        None if config.persist == Some(false) => Hub::default(),
        None => {
            return Err(
                "no place to save games: set state_dir (or XDG_STATE_HOME or HOME), \
                 or pass --ephemeral"
                    .into(),
            );
        }
        Some(dir) => {
            let (hub, loaded) = Hub::open(&dir, config.keep_finished())
                .map_err(|e| format!("opening {}: {e}", dir.display()))?;
            for (path, e) in &loaded.unreadable {
                eprintln!("dominion: skipping {}: {e}", path.display());
            }
            for (path, e) in &loaded.unrecorded {
                eprintln!(
                    "dominion: {} records no more events, its stream incomplete: {e}",
                    path.display()
                );
            }
            if loaded.never_recorded > 0 {
                eprintln!(
                    "dominion: {} saved games have no event stream (saved before games were \
                     recorded, or with record_events off)",
                    loaded.never_recorded
                );
            }
            for (path, e) in &loaded.stale {
                eprintln!("dominion: {}: {e}", path.display());
            }
            if loaded.reconstructed > 0 {
                eprintln!(
                    "dominion: {} saved games have a stream rebuilt by `dominion games backfill`",
                    loaded.reconstructed
                );
            }
            if loaded.reconstructed_unserved > 0 {
                eprintln!(
                    "dominion: {} saved games that can still be undone have a stream rebuilt by \
                     `dominion games backfill`, served once they can't change",
                    loaded.reconstructed_unserved
                );
            }
            hub.set_events_encoding(config.events_encoding.unwrap_or_default());
            hub.set_record_events(config.record_events.unwrap_or(true));
            eprintln!(
                "dominion: {} saved games in {} ({} finished games moved to expired/)",
                loaded.games,
                dir.display(),
                loaded.expired
            );
            hub
        }
    };
    eprintln!("dominion: MCP endpoint http://{addr}/mcp");
    let web_dir = config.web_dir();
    if web_dir.is_some() {
        eprintln!("dominion: web client http://{addr}/");
    }
    let stop = CancellationToken::new();
    let signal = stop.clone();
    tokio::spawn(async move {
        let _ = tokio::signal::ctrl_c().await;
        signal.cancel();
    });
    let options = Options {
        allowed_hosts: hosts,
        session_idle: config.session_idle(),
        assets_dir: config.assets_dir(),
        web_dir,
        web: config.web(),
        arena: config.arena(),
        public_url: config.public_url.clone(),
        trust_proxy: config.trust_proxy.unwrap_or(false),
        agent_socket: config.agent_socket.clone(),
        control_socket: config.control_socket.clone(),
        official_server: dominion_server::official_host::Host::from_config(&config)?,
        online: dominion_server::online::Hosted::start_all(&config).await?,
    };
    dominion_server::serve(listener, Arc::new(hub), options, stop)
        .await
        .map_err(|e| e.to_string())
}

async fn assets(command: AssetsCommand, mut config: Config) -> Result<(), String> {
    let no_dir = || "no asset cache directory: set assets_dir (or XDG_CACHE_HOME or HOME)";
    match command {
        AssetsCommand::Sync {
            dir,
            force,
            dry_run,
        } => {
            config.assets_dir = dir.or(config.assets_dir);
            let dir = config.assets_dir().ok_or_else(no_dir)?;
            let options = dominion_server::assets::SyncOptions {
                dir,
                force,
                dry_run,
            };
            dominion_server::assets::sync(&options).await
        }
        AssetsCommand::Path => {
            println!("{}", config.assets_dir().ok_or_else(no_dir)?.display());
            Ok(())
        }
    }
}

async fn sandbox_forward(ports: Vec<String>, command: Vec<String>) -> ExitCode {
    let ports: Result<Vec<(u16, PathBuf)>, String> = (ports.iter())
        .map(|p| {
            let (port, socket) = p.split_once('=').ok_or(format!("{p}: want PORT=SOCKET"))?;
            let port = port.parse().map_err(|e| format!("{p}: {e}"))?;
            Ok((port, PathBuf::from(socket)))
        })
        .collect();
    match ports.map(|ports| dominion_server::harness::sandbox::forward(ports, command)) {
        Ok(run) => match run.await {
            Ok(code) => ExitCode::from(u8::try_from(code).unwrap_or(1)),
            Err(e) => {
                eprintln!("dominion sandbox-forward: {e}");
                ExitCode::FAILURE
            }
        },
        Err(e) => {
            eprintln!("dominion sandbox-forward: {e}");
            ExitCode::FAILURE
        }
    }
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    if let Command::SandboxForward { ports, command } = cli.command {
        return sandbox_forward(ports, command).await;
    }
    let mut config = match Config::load(cli.config.as_deref()) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("dominion: {e}");
            return ExitCode::FAILURE;
        }
    };
    let result = match cli.command {
        Command::Online { command } => match command {
            OnlineCommand::Sync => dominion_server::online::sync(&config).await,
            OnlineCommand::Probe => dominion_server::online::probe().await,
            OnlineCommand::Accounts => dominion_server::online::list(&config),
            OnlineCommand::Login { account } => {
                dominion_server::online::login_check(&config, account.as_deref()).await
            }
            OnlineCommand::PairCheck {
                host,
                guest,
                timer,
                kingdom,
                watch,
                idle,
            } => {
                let options = dominion_server::online::PairTable {
                    kingdom,
                    timer,
                    ..Default::default()
                };
                let end = if idle {
                    dominion_server::online::PairCheckEnd::Idle
                } else {
                    dominion_server::online::PairCheckEnd::Resign
                };
                let watch = std::time::Duration::from_secs(watch);
                dominion_server::online::pair_check(&config, &host, &guest, options, watch, end)
                    .await
            }
            OnlineCommand::Mcp { account } => {
                dominion_server::online::serve(&config, account.as_deref()).await
            }
        },
        Command::Serve {
            listen,
            port,
            state_dir,
            ephemeral,
            assets_dir,
            web_dir,
            events_encoding,
            record_events,
        } => {
            config.assets_dir = assets_dir.or(config.assets_dir);
            config.web_dir = web_dir.or(config.web_dir);
            config.listen = listen.or(config.listen);
            config.port = port.or(config.port);
            config.state_dir = state_dir.or(config.state_dir);
            config.events_encoding = events_encoding.or(config.events_encoding);
            config.record_events = record_events.or(config.record_events);
            if ephemeral {
                config.persist = Some(false);
            }
            serve(config).await
        }
        Command::Play(args) => {
            let url = args.server.clone().unwrap_or_else(|| config.server());
            let reconnect = args
                .reconnect
                .map_or(config.reconnect(), |s| Duration::from_secs(s.into()));
            let new = args.new.then(|| NewGame {
                players: args.players.unwrap_or(2),
                names: args.names,
                kingdom: args.kingdom,
                sets: args.sets,
                seed: args.seed,
                turn_limit: args.turn_limit,
                ..NewGame::default()
            });
            let seat = args.game.zip(args.token);
            dominion_server::client::play(&url, new, seat, reconnect).await
        }
        Command::Assets { command } => assets(command, config).await,
        Command::Agents => match config.runner() {
            Ok(settings) => dominion_server::runner::run(settings).await,
            Err(e) => Err(e),
        },
        Command::Games {
            command:
                GamesCommand::Backfill {
                    dry_run,
                    state_dir,
                    events_encoding,
                },
            ..
        } => match state_dir.or_else(|| config.state_dir()) {
            None => Err("no state directory: pass --state-dir, or set state_dir".into()),
            Some(dir) => {
                let encoding = events_encoding.or(config.events_encoding);
                let options =
                    dominion_server::backfill::Options::now(dry_run, encoding.unwrap_or_default());
                dominion_server::backfill::cli(&dir, &options)
            }
        },
        Command::Games { socket, command } => games(socket, command, &config).await,
        Command::Solo(args) => solo(args, &config),
        Command::SandboxForward { .. } => unreachable!("handled before the settings load"),
        Command::T3Login => match &config.t3code {
            None => Err("add [t3code] to the settings file".into()),
            Some(settings) => dominion_server::t3::login(&settings.url, |url| {
                eprintln!("Open this URL and approve Full access for the Dominion runner:\n{url}");
            })
            .await
            .map(|token| println!("{token}")),
        },
        Command::Card { names } => names.iter().try_for_each(|name| {
            dominion_agent::glossary(name)
                .map(|e| println!("{}\n", dominion_server::card_text(&e)))
                .ok_or_else(|| format!("no card is named {name:?}"))
        }),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("dominion: {e}");
            ExitCode::FAILURE
        }
    }
}
