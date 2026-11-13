//! The server's own agents. A runner on the server's machine (`dominion agents`, over
//! the `agent_socket`) offers them and starts one for every seat they hold in a running
//! game; the server tells it which those are ([`ToRunner::Jobs`]) whenever that changes.
//! Each is an account of its own (`Login::House`), so it has a rating and records.
//! Registered players fill seats with them, a few a day each, as many at once as the
//! runner allows.

use super::{Arena, Error, Login, Result, State, User, conflict};
use crate::house::{Job, Offer, ToRunner};
use crate::hub::{lock, now_ms};
use dominion_protocol::lobby::{self, HouseAgent, UserKind};
use std::collections::{HashMap, VecDeque};
use std::time::Duration;
use tokio::sync::mpsc::UnboundedSender;

/// Which runner connection is the current one.
pub type RunnerId = u64;

const DAY_MS: u64 = 86_400_000;

#[derive(Default)]
pub(super) struct House {
    runner: Option<Runner>,
    next_runner: RunnerId,
    /// Agent seats of games being set up, not yet among the jobs.
    reserved: u32,
    /// When each account filled its agent seats, over the last day (ms).
    filled: HashMap<String, VecDeque<u64>>,
}

struct Runner {
    id: RunnerId,
    capacity: u32,
    agents: Vec<Agent>,
    send: UnboundedSender<ToRunner>,
    /// The jobs it was told of last.
    told: Option<Vec<Job>>,
}

#[derive(Clone)]
pub(super) struct Agent {
    /// The account's id.
    pub user: String,
    pub name: String,
    pub about: String,
}

impl House {
    pub(super) fn agents(&self) -> &[Agent] {
        self.runner.as_ref().map_or(&[], |r| &r.agents)
    }

    pub(super) fn agent_named(&self, name: &str) -> Option<&Agent> {
        let key = name.trim().to_lowercase();
        self.agents().iter().find(|a| a.name.to_lowercase() == key)
    }

    pub(super) fn offers(&self, user: &str) -> bool {
        self.agents().iter().any(|a| a.user == user)
    }

    fn filled_today(&mut self, user: &str) -> u32 {
        let since = now_ms().saturating_sub(DAY_MS);
        let Some(times) = self.filled.get_mut(user) else {
            return 0;
        };
        while times.front().is_some_and(|&t| t < since) {
            times.pop_front();
        }
        times.len() as u32
    }
}

/// Seats filled with the server's agents for a game being set up, released when dropped.
pub(super) struct Reservation<'a> {
    arena: &'a Arena,
    host: String,
    seats: u32,
    kept: bool,
}

impl Reservation<'_> {
    /// The game was set up: the seats count against the host's day.
    pub(super) fn keep(mut self) {
        self.kept = true;
    }
}

impl Drop for Reservation<'_> {
    fn drop(&mut self) {
        if self.seats == 0 {
            return;
        }
        {
            let mut state = lock(&self.arena.state);
            let house = &mut state.house;
            house.reserved = house.reserved.saturating_sub(self.seats);
            if self.kept {
                let times = house.filled.entry(self.host.clone()).or_default();
                times.extend(std::iter::repeat_n(now_ms(), self.seats as usize));
            }
        }
        if self.kept {
            self.arena.dispatch();
        }
        self.arena.bump();
    }
}

impl Arena {
    /// The jobs: every seat the runner's agents hold in a running game.
    fn jobs(&self, agents: &[Agent]) -> Vec<Job> {
        let mut jobs: Vec<Job> = (agents.iter())
            .flat_map(|agent| {
                (self.hub.seats_of(&agent.user, Duration::ZERO).into_iter())
                    .filter(|g| g.running)
                    .map(|g| Job {
                        agent: agent.name.clone(),
                        game_id: g.game_id,
                        seat: g.seat,
                        name: g.name,
                        token: g.token,
                    })
            })
            .collect();
        jobs.sort_by(|a, b| (&a.game_id, a.seat).cmp(&(&b.game_id, b.seat)));
        jobs
    }

    /// Tells the runner its jobs, if they changed since it was told last.
    pub(super) fn dispatch(&self) {
        let mut state = lock(&self.state);
        let Some(runner) = &mut state.house.runner else {
            return;
        };
        let jobs = self.jobs(&runner.agents);
        if runner.told.as_ref() != Some(&jobs) {
            let _ = runner.send.send(ToRunner::Jobs { jobs: jobs.clone() });
            runner.told = Some(jobs);
        }
    }

    /// A runner connected, offering `offers`, `capacity` at once; it replaces any other.
    /// Each agent plays as the account of its name, made if there is none; a name taken
    /// by someone else's account is refused.
    pub fn runner_connected(
        &self,
        offers: Vec<Offer>,
        capacity: u32,
        send: UnboundedSender<ToRunner>,
    ) -> RunnerId {
        let mut agents: Vec<Agent> = vec![];
        for offer in offers {
            match self.house_account(&offer.name) {
                Ok(user) if agents.iter().all(|a| a.user != user.id) => agents.push(Agent {
                    user: user.id,
                    name: user.name,
                    about: offer.about,
                }),
                Ok(_) => {}
                Err(e) => {
                    let reason = e.to_string();
                    eprintln!("dominion: the agent {}: {reason}", offer.name);
                    let _ = send.send(ToRunner::Refused {
                        agent: offer.name,
                        reason,
                    });
                }
            }
        }
        let id = {
            let mut state = lock(&self.state);
            let house = &mut state.house;
            house.next_runner += 1;
            house.runner = Some(Runner {
                id: house.next_runner,
                capacity,
                agents,
                send,
                told: None,
            });
            house.next_runner
        };
        self.dispatch();
        self.bump();
        id
    }

    /// The runner `id` went away. Its agents can't be seated until one is back; their
    /// games wait for them (and a timed one forfeits them after a while).
    pub fn runner_gone(&self, id: RunnerId) {
        let mut state = lock(&self.state);
        if state.house.runner.as_ref().is_some_and(|r| r.id == id) {
            state.house.runner = None;
            drop(state);
            self.bump();
        }
    }

    fn house_account(&self, name: &str) -> Result<User> {
        let name = super::accounts::account_name(name)?;
        match self.db.user_by_name(&name)? {
            Some(user) if matches!(user.login, Login::House) => Ok(user),
            Some(_) => conflict(format!("the name {name} is someone else's")),
            None => self.db.add_user(&name, &Login::House, now_ms() / 1000),
        }
    }

    /// Keeps `seats` agent seats for a game `host` is setting up, if they may fill that
    /// many today and the runner has room for them.
    pub(super) fn reserve(&self, host: &str, seats: u32) -> Result<Reservation<'_>> {
        let reservation = |seats| Reservation {
            arena: self,
            host: host.to_string(),
            seats,
            kept: false,
        };
        if seats == 0 {
            return Ok(reservation(0));
        }
        if self.user(host)?.kind() != UserKind::Registered {
            return Err(Error::Forbidden(
                "only registered players seat the server's agents".into(),
            ));
        }
        let per_day = self.settings.agent_seats_per_day;
        let mut state = lock(&self.state);
        let Some(runner) = &state.house.runner else {
            return conflict("the server's agents aren't available now");
        };
        let busy = self.jobs(&runner.agents).len() as u32 + state.house.reserved;
        let capacity = runner.capacity;
        if busy + seats > capacity {
            return conflict(format!(
                "the server's agents are busy ({busy} of {capacity} playing); try again when a game ends"
            ));
        }
        let filled = state.house.filled_today(host);
        if filled + seats > per_day {
            return Err(Error::TooMany(
                format!("you have filled {filled} of your {per_day} agent seats for today"),
                Duration::from_secs(3600),
            ));
        }
        state.house.reserved += seats;
        Ok(reservation(seats))
    }

    /// What the lobby says about the server's agents, for `you`.
    pub(super) fn house_info(&self, state: &mut State, you: Option<&User>) -> lobby::House {
        let per_day = self.settings.agent_seats_per_day;
        let left = you
            .filter(|u| u.kind() == UserKind::Registered)
            .map(|u| per_day.saturating_sub(state.house.filled_today(&u.id)));
        let Some(runner) = &state.house.runner else {
            return lobby::House {
                left,
                ..lobby::House::default()
            };
        };
        let jobs = self.jobs(&runner.agents);
        let busy = jobs.len() as u32 + state.house.reserved;
        lobby::House {
            agents: (runner.agents.iter())
                .filter_map(|a| {
                    Some(HouseAgent {
                        user: self.db.user_ref(&a.user)?,
                        about: a.about.clone(),
                        playing: jobs.iter().filter(|j| j.agent == a.name).count() as u32,
                    })
                })
                .collect(),
            free: runner.capacity.saturating_sub(busy),
            left,
        }
    }
}
