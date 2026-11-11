//! The games a server hosts, and the seat tokens that let a client act for a seat.

use dominion_agent::{AgentView, Answer, AnswerError, Table};
use dominion_engine::{Card, GameConfig, PlayerId, Set};
use rand::Rng;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::watch;

/// What a client asks for when creating a game.
#[derive(Clone, Debug, Default, Deserialize, Serialize, schemars::JsonSchema)]
pub struct NewGame {
    /// Number of players, 2 to 6.
    pub players: u8,
    /// Seat names, in turn order (default P1, P2, ...).
    #[serde(default)]
    pub names: Vec<String>,
    /// Kingdom piles by name. Without it, a random kingdom is dealt from `sets`.
    #[serde(default)]
    pub kingdom: Vec<String>,
    /// Events, Landmarks, Projects, Ways, ... by name (with `kingdom`).
    #[serde(default)]
    pub landscapes: Vec<String>,
    /// Sets to deal a random kingdom from (default: every released set).
    #[serde(default)]
    pub sets: Vec<String>,
    pub seed: Option<u64>,
    /// End the game after this many turns in total.
    pub turn_limit: Option<u32>,
    pub colonies: Option<bool>,
    pub shelters: Option<bool>,
}

#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct SeatToken {
    pub seat: u8,
    pub name: String,
    pub token: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Created {
    pub game_id: String,
    pub seats: Vec<SeatToken>,
}

#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct GameSummary {
    pub game_id: String,
    pub names: Vec<String>,
    pub running: bool,
    /// The seat that must decide now.
    pub deciding: Option<u8>,
    pub kingdom: Vec<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum HubError {
    #[error("{0}")]
    BadRequest(String),
    #[error("unknown game {0}")]
    UnknownGame(String),
    #[error("that token is not a seat token of game {0}")]
    BadToken(String),
    #[error(transparent)]
    Answer(#[from] AnswerError),
}

struct Hosted {
    table: Table,
    tokens: Vec<String>,
    /// Bumped after every answer, for waiters.
    version: watch::Sender<u64>,
}

/// A seat a token claims.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Seat {
    pub game: String,
    pub seat: PlayerId,
}

#[derive(Default)]
pub struct Hub {
    games: Mutex<BTreeMap<String, Arc<Mutex<Hosted>>>>,
    next: Mutex<u64>,
}

fn card(name: &str, want: impl Fn(Card) -> bool) -> Result<Card, HubError> {
    Card::from_name(name)
        .filter(|&c| want(c))
        .ok_or_else(|| HubError::BadRequest(format!("{name:?} is not a known card here")))
}

fn released_sets() -> Vec<Set> {
    Set::ALL
        .iter()
        .copied()
        .filter(|&s| s != Set::Arcana)
        .collect()
}

pub fn config(req: &NewGame) -> Result<GameConfig, HubError> {
    let seed = req.seed.unwrap_or_else(|| rand::rng().random());
    let mut config = if req.kingdom.is_empty() {
        let sets = if req.sets.is_empty() {
            released_sets()
        } else {
            req.sets
                .iter()
                .map(|name| {
                    let key = |s: &str| s.to_lowercase().replace(['&', ' ', '-', '_'], "");
                    Set::ALL
                        .iter()
                        .copied()
                        .find(|s| key(s.name()) == key(name))
                        .ok_or_else(|| HubError::BadRequest(format!("unknown set {name:?}")))
                })
                .collect::<Result<_, _>>()?
        };
        GameConfig::random(req.players, &sets, seed)
    } else {
        let kingdom = req
            .kingdom
            .iter()
            .map(|n| card(n, |c| c.def().kingdom))
            .collect::<Result<_, _>>()?;
        let mut config = GameConfig::new(req.players, kingdom, seed);
        config.landscapes = req
            .landscapes
            .iter()
            .map(|n| card(n, |c| c.kind().is_landscape()))
            .collect::<Result<_, _>>()?;
        config
    };
    config.turn_limit = req.turn_limit;
    config.colonies = req.colonies;
    config.shelters = req.shelters;
    Ok(config)
}

fn token() -> String {
    format!("{:032x}", rand::rng().random::<u128>())
}

impl Hub {
    pub fn create(&self, req: &NewGame) -> Result<Created, HubError> {
        let config = config(req)?;
        let id = {
            let mut next = self.next.lock().unwrap();
            *next += 1;
            format!("g{next}")
        };
        let table = Table::new(&id, config, req.names.clone())
            .map_err(|e| HubError::BadRequest(e.to_string()))?;
        let tokens: Vec<String> = (0..table.names().len()).map(|_| token()).collect();
        let seats = tokens
            .iter()
            .enumerate()
            .map(|(i, t)| SeatToken {
                seat: i as u8,
                name: table.names()[i].clone(),
                token: t.clone(),
            })
            .collect();
        let hosted = Hosted {
            table,
            tokens,
            version: watch::channel(0).0,
        };
        self.games
            .lock()
            .unwrap()
            .insert(id.clone(), Arc::new(Mutex::new(hosted)));
        Ok(Created { game_id: id, seats })
    }

    fn game(&self, id: &str) -> Result<Arc<Mutex<Hosted>>, HubError> {
        self.games
            .lock()
            .unwrap()
            .get(id)
            .cloned()
            .ok_or_else(|| HubError::UnknownGame(id.into()))
    }

    /// The seat `token` claims in game `id`.
    pub fn claim(&self, id: &str, token: &str) -> Result<Seat, HubError> {
        let game = self.game(id)?;
        let hosted = game.lock().unwrap();
        let seat = hosted
            .tokens
            .iter()
            .position(|t| t == token)
            .ok_or_else(|| HubError::BadToken(id.into()))?;
        Ok(Seat {
            game: id.into(),
            seat: PlayerId(seat as u8),
        })
    }

    /// The seat `token` claims in whichever game it belongs to.
    pub fn find(&self, token: &str) -> Result<Seat, HubError> {
        let games: Vec<(String, Arc<Mutex<Hosted>>)> = self
            .games
            .lock()
            .unwrap()
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        for (id, game) in games {
            let hosted = game.lock().unwrap();
            if let Some(seat) = hosted.tokens.iter().position(|t| t == token) {
                return Ok(Seat {
                    game: id,
                    seat: PlayerId(seat as u8),
                });
            }
        }
        Err(HubError::BadRequest("unknown seat token".into()))
    }

    pub fn list(&self) -> Vec<GameSummary> {
        let games: Vec<Arc<Mutex<Hosted>>> = self.games.lock().unwrap().values().cloned().collect();
        games
            .iter()
            .map(|g| {
                let h = g.lock().unwrap();
                let t = &h.table;
                GameSummary {
                    game_id: t.id().into(),
                    names: t.names().to_vec(),
                    running: !t.is_over(),
                    deciding: t.deciding().map(|p| p.0),
                    kingdom: t
                        .game()
                        .config()
                        .kingdom
                        .iter()
                        .map(|c| c.name().to_string())
                        .collect(),
                }
            })
            .collect()
    }

    pub fn view(&self, seat: &Seat) -> Result<(AgentView, String), HubError> {
        let game = self.game(&seat.game)?;
        let h = game.lock().unwrap();
        let view = h.table.view(seat.seat);
        let text = dominion_agent::text::render(&view);
        Ok((view, text))
    }

    pub fn answer(&self, seat: &Seat, answer: &Answer) -> Result<(), HubError> {
        let game = self.game(&seat.game)?;
        let mut h = game.lock().unwrap();
        h.table.answer(seat.seat, answer)?;
        h.version.send_modify(|v| *v += 1);
        Ok(())
    }

    /// Waits until `seat` must decide or the game is over, for at most `timeout`. Returns
    /// whether it is so.
    pub async fn wait(&self, seat: &Seat, timeout: Duration) -> Result<bool, HubError> {
        let game = self.game(&seat.game)?;
        let mut rx = game.lock().unwrap().version.subscribe();
        let ready = |game: &Arc<Mutex<Hosted>>| {
            let h = game.lock().unwrap();
            h.table.is_over() || h.table.deciding() == Some(seat.seat)
        };
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            if ready(&game) {
                return Ok(true);
            }
            match tokio::time::timeout_at(deadline, rx.changed()).await {
                Ok(Ok(())) => continue,
                _ => return Ok(ready(&game)),
            }
        }
    }
}
