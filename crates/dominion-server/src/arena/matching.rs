//! Automatch: players search for a game, and the server pairs those whose searches fit
//! each other, closest in rating first, into a game with a random kingdom.

use super::{Arena, Ident, Result, State, bad, conflict, limited};
use crate::hub::{self, Meta, lock, now_ms};
use dominion_protocol::api::DealRequest;
use dominion_protocol::lobby::{
    MatchRequest, Opponents, QueueState, QueueStatus, Source, UserKind,
};
use rand::seq::SliceRandom;
use std::time::{Duration, Instant};

pub(super) struct Entry {
    pub user: String,
    /// The bot's owner, or the account itself: no game pairs two of one owner's.
    owner: String,
    pub agent: bool,
    guest: bool,
    rating: f64,
    request: MatchRequest,
    /// Milliseconds since the Unix epoch.
    since: u64,
    banned: Vec<String>,
}

pub(super) struct Matched {
    game: String,
    request: MatchRequest,
    since: u64,
    at: Instant,
}

/// How long a found game is shown as the search's result.
const SHOWN: Duration = Duration::from_secs(120);

fn fits(a: &Entry, b: &Entry) -> bool {
    let wants = |x: &Entry, y: &Entry| match x.request.opponents {
        Opponents::Any => true,
        Opponents::Humans => !y.agent,
        Opponents::Agents => y.agent,
    };
    a.owner != b.owner && wants(a, b) && wants(b, a)
}

impl State {
    pub(super) fn queue_status(&self, user: &str) -> Option<QueueStatus> {
        if let Some(e) = self.queue.iter().find(|e| e.user == user) {
            return Some(QueueStatus {
                state: QueueState::Searching,
                since: e.since,
                request: e.request.clone(),
                game_id: None,
            });
        }
        self.matched.get(user).map(|m| QueueStatus {
            state: QueueState::Matched,
            since: m.since,
            request: m.request.clone(),
            game_id: Some(m.game.clone()),
        })
    }

    /// Forgets the found games shown long enough. Whether any were.
    pub(super) fn forget_matches(&mut self) -> bool {
        let before = self.matched.len();
        self.matched.retain(|_, m| m.at.elapsed() < SHOWN);
        self.matched.len() != before
    }

    /// Takes a group that can play together out of the queue, the longest waiting first.
    fn take_group(&mut self) -> Option<Vec<Entry>> {
        for first in 0..self.queue.len() {
            let mut sizes = self.queue[first].request.players.clone();
            sizes.sort_unstable();
            for size in sizes {
                let mut group = vec![first];
                let mut others: Vec<usize> = (0..self.queue.len())
                    .filter(|&i| i != first && self.queue[i].request.players.contains(&size))
                    .collect();
                let rating = self.queue[first].rating;
                others.sort_by(|&a, &b| {
                    let far = |i: usize| (self.queue[i].rating - rating).abs();
                    far(a).total_cmp(&far(b))
                });
                for other in others {
                    if group.len() == usize::from(size) {
                        break;
                    }
                    if group
                        .iter()
                        .all(|&g| fits(&self.queue[g], &self.queue[other]))
                    {
                        group.push(other);
                    }
                }
                if group.len() == usize::from(size) {
                    group.sort_unstable();
                    return Some(group.iter().rev().map(|&i| self.queue.remove(i)).collect());
                }
            }
        }
        None
    }
}

impl Arena {
    /// Starts `ident`'s search, in place of any they had. They leave their table.
    pub fn search(&self, ident: &Ident, request: &MatchRequest) -> Result<QueueStatus> {
        limited(&self.limits.lobby, ident.user.clone(), "searches")?;
        let mut request = request.clone();
        request.players.sort_unstable();
        request.players.dedup();
        if request.players.is_empty() || request.players.iter().any(|n| !(2..=6).contains(n)) {
            return bad("`players` lists the player counts you accept, each 2 to 6");
        }
        if self.hub.in_timed_game(&ident.user) {
            return conflict("finish or resign the game you are in first");
        }
        let mut state = lock(&self.state);
        let user = state.user(&ident.user)?.clone();
        state.leave_all(&user.id);
        state.touch(&user.id);
        let entry = Entry {
            owner: user.owner().unwrap_or(&user.id).to_string(),
            agent: ident.agent || user.kind() == UserKind::Bot,
            guest: user.kind() == UserKind::Guest,
            rating: user.rating.rating,
            banned: (user.banned_cards.clone())
                .unwrap_or_else(|| self.settings.banned_cards.clone()),
            user: user.id.clone(),
            request,
            since: now_ms(),
        };
        state.queue.push(entry);
        drop(state);
        self.run_matching();
        self.bump();
        let status = lock(&self.state).queue_status(&ident.user);
        status.ok_or_else(|| super::Error::Conflict("the search ended".into()))
    }

    /// Ends `ident`'s search, or stops showing the game it found.
    pub fn cancel_search(&self, ident: &Ident) {
        let mut state = lock(&self.state);
        state.queue.retain(|e| e.user != ident.user);
        state.matched.remove(&ident.user);
        drop(state);
        self.bump();
    }

    pub fn queue_status(&self, ident: &Ident) -> Option<QueueStatus> {
        let mut state = lock(&self.state);
        state.touch(&ident.user);
        state.queue_status(&ident.user)
    }

    /// Starts a game for every group in the queue that can play together.
    pub(super) fn run_matching(&self) {
        loop {
            let Some(mut group) = lock(&self.state).take_group() else {
                return;
            };
            group.shuffle(&mut rand::rng());
            match self.start_match(&group) {
                Ok(game) => {
                    let mut state = lock(&self.state);
                    for e in group {
                        let matched = Matched {
                            game: game.clone(),
                            request: e.request,
                            since: e.since,
                            at: Instant::now(),
                        };
                        state.matched.insert(e.user, matched);
                    }
                    drop(state);
                    self.bump();
                }
                Err(e) => {
                    eprintln!("dominion: starting a matched game: {e}");
                    lock(&self.state).queue.extend(group);
                    return;
                }
            }
        }
    }

    fn start_match(&self, group: &[Entry]) -> Result<String> {
        let mut banned: Vec<String> = vec![];
        for card in group.iter().flat_map(|e| &e.banned) {
            if !banned.contains(card) {
                banned.push(card.clone());
            }
        }
        let request = DealRequest {
            players: group.len() as u8,
            banned,
            ..DealRequest::default()
        };
        let (config, _) = hub::deal_config(&request)?;
        let names: Vec<String> = {
            let state = lock(&self.state);
            (group.iter())
                .map(|e| state.user(&e.user).map(|u| u.name.clone()))
                .collect::<Result<_>>()?
        };
        let meta = Meta {
            source: Source::Automatch,
            owners: group.iter().map(|e| Some(e.user.clone())).collect(),
            rated: group.iter().all(|e| e.request.rated && !e.guest),
            watchable: true,
            timed: true,
            no_undo: true,
            ..Meta::default()
        };
        Ok(self.hub.host(config, names, meta)?.game_id)
    }
}
