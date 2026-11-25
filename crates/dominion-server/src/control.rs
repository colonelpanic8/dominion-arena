//! The operator's socket: `dominion games` lists the games a running `dominion serve`
//! hosts, retires them, voids finished rated games, and drains official accounts before
//! a restart. A connection sends one request
//! line and reads one reply line, both JSON. Only the socket's owner may connect.

use crate::arena::Arena;
use crate::hub::Hosting;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio_util::sync::CancellationToken;

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "kebab-case")]
pub enum Request {
    List,
    Retire {
        games: Vec<String>,
    },
    Void {
        games: Vec<String>,
        dry_run: bool,
        #[serde(default)]
        record_only: bool,
    },
    /// Start draining official accounts for a restart (or stop, with `cancel`): no new
    /// games start, and the reply lists those still running.
    Drain {
        #[serde(default)]
        cancel: bool,
    },
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "reply", rename_all = "kebab-case")]
pub enum Reply {
    Games {
        games: Vec<Hosting>,
    },
    Retired {
        retired: Vec<String>,
        /// Each game not retired, with why.
        refused: Vec<(String, String)>,
    },
    Voided {
        voided: crate::arena::Voided,
        dry_run: bool,
    },
    Draining {
        draining: bool,
        /// The official games still running, as `(account, game)`.
        running: Vec<(String, String)>,
    },
    Error {
        message: String,
    },
}

/// Answers `dominion games` on `path` until `stop`.
pub async fn listen(
    arena: Arc<Arena>,
    path: &Path,
    stop: CancellationToken,
) -> std::io::Result<()> {
    if std::fs::symlink_metadata(path).is_ok() {
        std::fs::remove_file(path)?;
    }
    let listener = UnixListener::bind(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    loop {
        let stream = tokio::select! {
            _ = stop.cancelled() => return Ok(()),
            accepted = listener.accept() => match accepted {
                Ok((stream, _)) => stream,
                Err(e) => {
                    eprintln!("dominion: control socket: {e}");
                    continue;
                }
            },
        };
        tokio::spawn(answer(arena.clone(), stream));
    }
}

async fn answer(arena: Arc<Arena>, stream: UnixStream) {
    let (read, mut write) = stream.into_split();
    let Ok(Some(line)) = BufReader::new(read).lines().next_line().await else {
        return;
    };
    let reply = match serde_json::from_str::<Request>(&line) {
        Err(e) => Reply::Error {
            message: format!("bad request: {e}"),
        },
        Ok(request) => tokio::task::spawn_blocking(move || handle(&arena, request))
            .await
            .unwrap_or_else(|e| Reply::Error {
                message: e.to_string(),
            }),
    };
    let mut line = serde_json::to_string(&reply).expect("replies serialize");
    line.push('\n');
    let _ = write.write_all(line.as_bytes()).await;
}

fn handle(arena: &Arena, request: Request) -> Reply {
    match request {
        Request::List => Reply::Games {
            games: arena.hub.hosting(),
        },
        Request::Retire { games } => {
            let (mut retired, mut refused) = (vec![], vec![]);
            for (id, done) in arena.retire(&games) {
                match done {
                    Ok(()) => retired.push(id),
                    Err(e) => refused.push((id, e)),
                }
            }
            Reply::Retired { retired, refused }
        }
        Request::Drain { cancel } => Reply::Draining {
            draining: !cancel,
            running: arena.drain(!cancel),
        },
        Request::Void {
            games,
            dry_run,
            record_only,
        } => {
            let how = crate::arena::VoidHow {
                dry_run,
                record_only,
            };
            match arena.void_ratings(&games, how) {
                Ok(voided) => Reply::Voided { voided, dry_run },
                Err(e) => Reply::Error {
                    message: e.to_string(),
                },
            }
        }
    }
}

/// Sends `request` to the server at `path`.
pub async fn request(path: &Path, request: &Request) -> Result<Reply, String> {
    let stream = UnixStream::connect(path)
        .await
        .map_err(|e| format!("{}: {e}", path.display()))?;
    let (read, mut write) = stream.into_split();
    let mut line = serde_json::to_string(request).map_err(|e| e.to_string())?;
    line.push('\n');
    write
        .write_all(line.as_bytes())
        .await
        .map_err(|e| e.to_string())?;
    let reply = BufReader::new(read)
        .lines()
        .next_line()
        .await
        .map_err(|e| e.to_string())?
        .ok_or("the server closed the connection without a reply")?;
    serde_json::from_str(&reply).map_err(|e| format!("bad reply: {e}"))
}
