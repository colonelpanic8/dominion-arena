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

/// Play Dominion through the agent-view@2 protocol.
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
    /// Host games as an MCP server (streamable HTTP at /mcp).
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
    },
    /// Play one seat from the terminal against a server.
    Play(PlayArgs),
    /// Print cards' official text.
    Card {
        #[arg(required = true)]
        names: Vec<String>,
    },
    /// Play a deck out alone, in a game whose other seats sit idle.
    Solo(SoloArgs),
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
            eprintln!(
                "dominion: {} saved games in {} ({} finished games expired)",
                loaded.games,
                dir.display(),
                loaded.expired
            );
            hub
        }
    };
    eprintln!("dominion: MCP endpoint http://{addr}/mcp");
    let stop = CancellationToken::new();
    let signal = stop.clone();
    tokio::spawn(async move {
        let _ = tokio::signal::ctrl_c().await;
        signal.cancel();
    });
    let options = Options {
        allowed_hosts: hosts,
        session_idle: config.session_idle(),
    };
    dominion_server::serve(listener, Arc::new(hub), options, stop)
        .await
        .map_err(|e| e.to_string())
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    let mut config = match Config::load(cli.config.as_deref()) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("dominion: {e}");
            return ExitCode::FAILURE;
        }
    };
    let result = match cli.command {
        Command::Serve {
            listen,
            port,
            state_dir,
            ephemeral,
        } => {
            config.listen = listen.or(config.listen);
            config.port = port.or(config.port);
            config.state_dir = state_dir.or(config.state_dir);
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
        Command::Solo(args) => solo(args, &config),
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
