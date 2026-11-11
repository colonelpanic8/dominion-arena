//! The `dominion` binary: `serve` and the terminal client.

use dominion_agent::Answer;
use dominion_server::Options;
use dominion_server::hub::{Created, Hub, NewGame, Seat};
use std::io::{BufRead, BufReader, ErrorKind, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant};
use tempfile::TempDir;
use tokio::runtime::Runtime;
use tokio_util::sync::CancellationToken;

const PATIENCE: Duration = Duration::from_secs(30);

/// The binary, kept away from this machine's settings and saved games.
fn dominion(home: &TempDir) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_dominion"));
    command
        .env("XDG_CONFIG_HOME", home.path().join("config"))
        .env("XDG_STATE_HOME", home.path().join("state"));
    command
}

/// A server in this process, so a test can reach into its games.
struct Server {
    addr: SocketAddr,
    stop: CancellationToken,
    runtime: Runtime,
}

impl Server {
    fn start(addr: &str, hub: Arc<Hub>) -> Server {
        let runtime = Runtime::new().unwrap();
        let listener = runtime
            .block_on(tokio::net::TcpListener::bind(addr))
            .unwrap();
        let addr = listener.local_addr().unwrap();
        let stop = CancellationToken::new();
        runtime.spawn(dominion_server::serve(
            listener,
            hub,
            Options::default(),
            stop.clone(),
        ));
        Server {
            addr,
            stop,
            runtime,
        }
    }

    fn url(&self) -> String {
        format!("http://{}/mcp", self.addr)
    }

    /// Stops serving and lets go of the hub.
    fn stop(self) {
        self.stop.cancel();
        self.runtime.shutdown_timeout(Duration::from_secs(5));
    }
}

/// `dominion play` with its output collected as it comes.
struct Play {
    child: Child,
    stdin: ChildStdin,
    printed: Receiver<String>,
    unread: String,
    _home: TempDir,
}

impl Play {
    fn start(server: &Server, game: &Created, seat: usize, more: &[&str]) -> Play {
        let home = tempfile::tempdir().unwrap();
        let mut child = dominion(&home)
            .args(["play", "--server", &server.url()])
            .args(["--game", &game.game_id])
            .args(["--token", &game.seats[seat].token])
            .args(more)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let mut stdout = child.stdout.take().unwrap();
        let (tx, printed) = channel();
        std::thread::spawn(move || {
            let mut chunk = [0; 4096];
            while let Ok(n @ 1..) = stdout.read(&mut chunk) {
                if tx
                    .send(String::from_utf8_lossy(&chunk[..n]).into_owned())
                    .is_err()
                {
                    break;
                }
            }
        });
        Play {
            stdin: child.stdin.take().unwrap(),
            child,
            printed,
            unread: String::new(),
            _home: home,
        }
    }

    /// What it prints from here up to and including `text`.
    fn until(&mut self, text: &str) -> String {
        let deadline = Instant::now() + PATIENCE;
        loop {
            if let Some(at) = self.unread.find(text) {
                let rest = self.unread.split_off(at + text.len());
                return std::mem::replace(&mut self.unread, rest);
            }
            let left = deadline.saturating_duration_since(Instant::now());
            match self.printed.recv_timeout(left) {
                Ok(chunk) => self.unread.push_str(&chunk),
                Err(_) => panic!("the client printed no {text:?}:\n{}", self.unread),
            }
        }
    }

    fn enter(&mut self, line: &str) {
        writeln!(self.stdin, "{line}").unwrap();
    }

    /// Whether it succeeded and what it wrote to stderr, once it exits by itself.
    fn exit(mut self) -> (bool, String) {
        let deadline = Instant::now() + PATIENCE;
        let status = loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                break status;
            }
            assert!(Instant::now() < deadline, "the client is still running");
            std::thread::sleep(Duration::from_millis(50));
        };
        let mut error = String::new();
        let mut stderr = self.child.stderr.take().unwrap();
        stderr.read_to_string(&mut error).unwrap();
        (status.success(), error)
    }
}

impl Drop for Play {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn two_players() -> NewGame {
    NewGame {
        players: 2,
        kingdom: vec!["Village".into()],
        seed: Some(1),
        ..NewGame::default()
    }
}

fn seat(game: &Created, seat: usize) -> Seat {
    Seat {
        game: game.game_id.clone(),
        seat: dominion_engine::PlayerId(seat as u8),
    }
}

/// Answers `seat`'s open decision with its first option, or the one labelled `label`.
fn answer(hub: &Hub, seat: &Seat, label: Option<&str>) {
    let decision = hub.view(seat).unwrap().0.decision.unwrap();
    let option = decision
        .options
        .iter()
        .find(|o| label.is_none_or(|l| o.label.contains(l)))
        .unwrap();
    let answer = Answer::indices(&decision.id, vec![option.index]);
    hub.answer(seat, &answer).unwrap();
}

#[test]
fn serve_announces_the_port_it_got() {
    let home = tempfile::tempdir().unwrap();
    let mut child = dominion(&home)
        .args(["serve", "--ephemeral", "--listen", "127.0.0.1"])
        .args(["--port", "0"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut line = String::new();
    BufReader::new(child.stderr.take().unwrap())
        .read_line(&mut line)
        .unwrap();
    let addr = line
        .trim()
        .strip_prefix("dominion: MCP endpoint http://")
        .and_then(|rest| rest.strip_suffix("/mcp"))
        .and_then(|addr| addr.parse::<SocketAddr>().ok());
    let reached = addr.map(TcpStream::connect);
    child.kill().unwrap();
    child.wait().unwrap();
    assert!(matches!(reached, Some(Ok(_))), "{line}");
}

#[test]
fn serve_does_not_drop_persistence_quietly() {
    let home = tempfile::tempdir().unwrap();
    let run = dominion(&home)
        .args(["serve", "--listen", "127.0.0.1", "--port", "0"])
        .env_remove("XDG_STATE_HOME")
        .env_remove("HOME")
        .output()
        .unwrap();
    let error = String::from_utf8_lossy(&run.stderr);
    assert!(!run.status.success());
    assert!(error.contains("--ephemeral"), "{error}");
}

#[test]
fn play_shows_the_game_whenever_it_has_to_wait() {
    let hub = Arc::new(Hub::default());
    let server = Server::start("127.0.0.1:0", hub.clone());
    let game = hub.create(&two_players()).unwrap();
    let mut play = Play::start(&server, &game, 1, &[]);
    let joined = play.until("Waiting for the other players...");
    assert!(joined.contains("YOU: P2"), "{joined}");

    answer(&hub, &seat(&game, 0), Some("End your Buy phase"));
    let turn = play.until("> ");
    assert!(turn.contains("DECISION"), "{turn}");
    let end = hub.view(&seat(&game, 1)).unwrap().0.decision.unwrap();
    let end = end
        .options
        .iter()
        .find(|o| o.label.contains("End your Buy phase"))
        .unwrap();
    play.enter(&end.index.to_string());
    let passed = play.until("Waiting for the other players...");
    assert!(passed.contains("P1's turn"), "{passed}");
    server.stop();
}

#[test]
fn play_takes_up_a_decision_that_changed_under_it() {
    let hub = Arc::new(Hub::default());
    let server = Server::start("127.0.0.1:0", hub.clone());
    let game = hub.create(&two_players()).unwrap();
    let mut play = Play::start(&server, &game, 0, &[]);
    play.until("DECISION d1");
    play.until("> ");

    // The same seat answers from somewhere else.
    answer(&hub, &seat(&game, 0), None);
    play.enter("0");
    let stale = play.until("> ");
    assert!(
        stale.contains("rejected: decision d1 is not open"),
        "{stale}"
    );
    assert!(stale.contains("DECISION d2"), "{stale}");
    play.enter("0");
    let next = play.until("> ");
    assert!(
        next.contains("DECISION d3") && !next.contains("rejected"),
        "{next}"
    );
    server.stop();
}

#[test]
fn play_rides_out_a_server_restart() {
    let home = tempfile::tempdir().unwrap();
    let keep = Duration::from_secs(3600);
    let (hub, _) = Hub::open(home.path(), keep).unwrap();
    let hub = Arc::new(hub);
    let server = Server::start("127.0.0.1:0", hub.clone());
    let addr = server.addr;
    let game = hub.create(&two_players()).unwrap();
    let mut play = Play::start(&server, &game, 1, &[]);
    play.until("Waiting for the other players...");

    server.stop();
    drop(hub);
    // The stopped server's tasks let go of the hub, and so of its directory, in their
    // own time.
    let deadline = Instant::now() + PATIENCE;
    let (hub, loaded) = loop {
        match Hub::open(home.path(), keep) {
            Ok(opened) => break opened,
            Err(e) if e.kind() == ErrorKind::ResourceBusy && Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(e) => panic!("{e}"),
        }
    };
    assert_eq!(loaded.games, 1);
    let hub = Arc::new(hub);
    let server = Server::start(&addr.to_string(), hub.clone());
    play.until("Connected again.");
    answer(&hub, &seat(&game, 0), Some("End your Buy phase"));
    let turn = play.until("> ");
    assert!(turn.contains("DECISION"), "{turn}");
    server.stop();
}

#[test]
fn play_gives_up_on_a_server_that_stays_away() {
    let hub = Arc::new(Hub::default());
    let server = Server::start("127.0.0.1:0", hub.clone());
    let game = hub.create(&two_players()).unwrap();
    let mut play = Play::start(&server, &game, 1, &["--reconnect", "1"]);
    play.until("Waiting for the other players...");
    server.stop();
    play.until("Lost the server");
    let (succeeded, error) = play.exit();
    assert!(!succeeded);
    assert!(error.contains("dominion: lost the server"), "{error}");
}

#[test]
fn play_checks_its_flags_before_it_connects() {
    let home = tempfile::tempdir().unwrap();
    // Nothing listens here, so a client that connected first would say so instead.
    let nowhere = ["play", "--server", "http://127.0.0.1:1/mcp"];
    for (flags, complaint) in [
        (&[][..], "--new"),
        (&["--game", "g1"], "--token"),
        (
            &["--game", "g1", "--token", "t", "--players", "3"],
            "--players",
        ),
        (
            &["--game", "g1", "--token", "t", "--kingdom", "Moat"],
            "--kingdom",
        ),
        (&["--new", "--game", "g1", "--token", "t"], "--new"),
    ] {
        let run = dominion(&home).args(nowhere).args(flags).output().unwrap();
        let error = String::from_utf8_lossy(&run.stderr);
        assert!(!run.status.success(), "{flags:?}");
        assert!(
            error.contains(complaint) && !error.contains("connecting"),
            "{flags:?}: {error}"
        );
    }
    let run = dominion(&home)
        .args(nowhere)
        .args(["--new", "--players", "3"])
        .output()
        .unwrap();
    let error = String::from_utf8_lossy(&run.stderr);
    assert!(error.contains("connecting"), "{error}");
}
