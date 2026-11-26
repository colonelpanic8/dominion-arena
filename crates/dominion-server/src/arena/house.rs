//! The server's own agents. A runner on the server's machine (`dominion agents`, over
//! the `agent_socket`) offers them and starts one for every seat they hold in a running
//! game; the server tells it which those are ([`ToRunner::Jobs`]) whenever that changes.
//! Each is an account of its own (`Login::House`), so it has a rating and records: made
//! when the runner offers it, or for an agent offered `on_demand` (one for each model and
//! effort the runner finds), when it is first seated. Registered players fill seats with
//! them, a few a day each, as many at once as the runner allows.

use super::{Arena, Error, Login, Result, State, User, conflict};
use crate::house::{Job, Offer, ToRunner};
use crate::hub::{lock, now_ms};
use dominion_protocol::lobby::{self, AgentSetup, HouseAgent, HouseChoice, HouseModel, UserKind};
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::time::Duration;
use tokio::sync::mpsc::UnboundedSender;

/// Which runner connection is the current one.
pub type RunnerId = u64;

const DAY_MS: u64 = 86_400_000;

/// What an agent attached to a person's seat is told before anything else.
const ATTACHED_BRIEF: &str = "You are attached to {player}'s seat. {player} is a person and \
plays it: you see exactly what they see, and you do not answer its decisions. Follow what each \
result tells you. While it says ADVISING, call wait (timeout_s 300) until it says ADVICE \
WANTED; then call advise once with a short note for {player} and go back to wait. A note says \
what matters at the decision shown: interactions between cards in their hand, in play and in \
the Kingdom that are easy to miss, what the opponents threaten, how close the game is to \
ending, and when asked or at a buy, what you would do and why in a sentence. Lead with the \
point, name cards exactly, stay under 80 words, and never restate the hand or the rules. \
Answer a question from {player} directly. If a result shows a DECISION without ADVISING, \
{player} has handed you the seat: play it with step as your own until a result says ADVISING \
again. Stop at GAME OVER.";

#[derive(Default)]
pub(super) struct House {
    runner: Option<Runner>,
    next_runner: RunnerId,
    /// Agent seats of games being set up, not yet among the jobs.
    reserved: u32,
    /// The official-server job of each official account, by its id.
    official: BTreeMap<String, Job>,
    /// When each account filled its agent seats, over the last day (ms).
    filled: HashMap<String, VecDeque<u64>>,
    /// What each agent was when last offered, by its name in lower case: kept after it
    /// stops being offered, for the games it plays out.
    setups: HashMap<String, AgentSetup>,
}

/// What the runner offers as an agent, by its name in lower case.
pub(super) type Setups = HashMap<String, AgentSetup>;

struct Runner {
    id: RunnerId,
    capacity: u32,
    /// What it offers.
    offers: Vec<Offer>,
    /// The offers with an account.
    agents: Vec<Agent>,
    /// Agents no longer offered, still playing the games they had when they were.
    retired: Vec<Agent>,
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

impl Runner {
    /// Every agent with seats it may be playing: those offered and those retired.
    fn playing(&self) -> Vec<Agent> {
        self.agents.iter().chain(&self.retired).cloned().collect()
    }
}

impl House {
    pub(super) fn agents(&self) -> &[Agent] {
        self.runner.as_ref().map_or(&[], |r| &r.agents)
    }

    pub(super) fn agent_named(&self, name: &str) -> Option<&Agent> {
        let key = name.trim().to_lowercase();
        self.agents().iter().find(|a| a.name.to_lowercase() == key)
    }

    pub(super) fn official_job(&self, account: &str) -> Option<Job> {
        self.official.get(account).cloned()
    }

    /// Every agent's setup as last offered.
    pub(super) fn setups(&self) -> Setups {
        self.setups.clone()
    }

    fn remember(&mut self, offers: &[Offer]) {
        for offer in offers {
            if let Some(m) = &offer.model {
                let setup = AgentSetup {
                    agent: offer.name.clone(),
                    speed: m.fast_mode.then_some(lobby::InferenceSpeed::Fast),
                    program: m.program.clone(),
                    model: Some(m.id.clone()),
                    model_name: Some(m.name.clone()),
                    effort: m.effort.clone(),
                    ..AgentSetup::default()
                };
                self.setups.insert(offer.name.trim().to_lowercase(), setup);
            }
        }
    }

    pub(super) fn offers(&self, user: &str) -> bool {
        self.agents().iter().any(|a| a.user == user)
    }

    /// What the runner offers as `name`, account or not.
    fn offer_named(&self, name: &str) -> Option<&Offer> {
        let key = name.trim().to_lowercase();
        (self.runner.as_ref()?.offers.iter()).find(|o| o.name.to_lowercase() == key)
    }

    pub(super) fn check_options(&self, agent: &str, options: lobby::LaunchOptions) -> Result<()> {
        let model = self
            .offer_named(agent)
            .and_then(|offer| offer.model.as_ref());
        if options.speed.is_some() && !model.is_some_and(|m| m.supports_speed) {
            return super::bad(format!("{agent} does not offer selectable inference speed"));
        }
        if options.strategy.is_classic() || model.is_some_and(|m| m.supports_clock_aware) {
            Ok(())
        } else {
            super::bad(format!("{agent} does not offer the clock-aware strategy"))
        }
    }

    /// The agent seated for a model alone: the first model's default effort.
    pub(super) fn default_agent(&self) -> Option<String> {
        let models = self.models();
        Some(models.first()?.default_choice()?.agent.clone())
    }

    /// Every offer, by model, in the order offered.
    fn models(&self) -> Vec<HouseModel> {
        let mut models: Vec<HouseModel> = vec![];
        for offer in self.runner.iter().flat_map(|r| &r.offers) {
            let choice = HouseChoice {
                supports_clock_aware: offer.model.as_ref().is_some_and(|m| m.supports_clock_aware),
                supports_speed: offer.model.as_ref().is_some_and(|m| m.supports_speed),
                fast_mode: offer.model.as_ref().is_some_and(|m| m.fast_mode),
                effort: offer.model.as_ref().and_then(|m| m.effort.clone()),
                agent: offer.name.clone(),
            };
            let Some(m) = &offer.model else {
                models.push(HouseModel {
                    program: offer.about.clone(),
                    model: offer.name.clone(),
                    name: offer.name.clone(),
                    choices: vec![choice],
                });
                continue;
            };
            match (models.iter_mut()).find(|h| h.program == m.program && h.model == m.id) {
                Some(model) => model.choices.push(choice),
                None => models.push(HouseModel {
                    program: m.program.clone(),
                    model: m.id.clone(),
                    name: m.name.clone(),
                    choices: vec![choice],
                }),
            }
        }
        models
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

pub(super) fn no_agents() -> Error {
    Error::Forbidden("this account may not seat the server's agents".into())
}

/// Seats filled with the server's agents for a game being set up, released when dropped.
pub(crate) struct Reservation<'a> {
    arena: &'a Arena,
    host: String,
    seats: u32,
    kept: bool,
}

impl Reservation<'_> {
    /// The game was set up: the seats count against the host's day.
    pub(crate) fn keep(mut self) {
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
    /// Older lobby snapshots always seat the host, but do not identify them if several
    /// accounts played. Older Bot Games also omit the house seat flag.
    pub(super) fn restore_model_hosts(&self) -> Result<()> {
        use lobby::Source;
        for game in self
            .hub
            .hosting()
            .into_iter()
            .filter(|g| g.running && matches!(g.source, Source::Table | Source::Automatch))
        {
            let meta = self.hub.meta(&game.id)?;
            if meta.host.is_some() {
                continue;
            }
            let mut agents = meta.agents;
            if game.source == Source::Automatch && !agents.contains(&true) && game.names.len() == 2
            {
                agents.clear();
                for owner in &meta.owners {
                    let user = owner
                        .as_ref()
                        .map(|id| self.db.user(id))
                        .transpose()?
                        .flatten();
                    agents.push(user.is_some_and(|u| matches!(u.login, Login::House)));
                }
            }
            if !agents.contains(&true) {
                continue;
            }
            let mut people = meta
                .owners
                .iter()
                .enumerate()
                .filter(|(i, _)| agents.get(*i) == Some(&false))
                .filter_map(|(_, user)| user.as_ref());
            if let Some(host) = people.next()
                && people.next().is_none()
            {
                self.hub
                    .restore_model_host(&game.id, agents, host.clone())?;
            }
        }
        Ok(())
    }

    /// Creates an ordinary game, optionally seating the server's agents directly.
    pub fn create_game(
        &self,
        who: Option<&super::Ident>,
        req: &crate::hub::NewGame,
    ) -> Result<crate::hub::Created> {
        if !req.agents.is_empty() && req.agents.len() != usize::from(req.players) {
            return super::bad("agents must have one entry per player (null for a token seat)");
        }
        let count = req.agents.iter().filter(|a| a.is_some()).count();
        let rated = req.rated.unwrap_or(false);
        if rated && count != usize::from(req.players) {
            return super::bad("a rated game has one of the server's agents in every seat");
        }
        if count == 0 {
            let mut req = req.clone();
            req.agents.clear();
            let config = crate::hub::config(&req)?;
            let names = crate::hub::names(&req)?;
            let mut meta = crate::hub::Meta {
                watchable: req.watchable.unwrap_or(true),
                clock: req.clock.map(crate::hub::ClockState::new),
                host: who.map(|w| w.user.clone()),
                ..Default::default()
            };
            crate::hub::brief(&req, &mut meta)?;
            return Ok(self.hub.host(config, names, meta)?);
        }
        let who =
            who.ok_or_else(|| Error::Unauthorized("sign in to seat the server's agents".into()))?;
        if self.may_seat_agents(&self.user(&who.user)?) {
            for selection in req.agents.iter().flatten() {
                self.summon(selection.name())?;
            }
        }
        let agents = {
            let state = lock(&self.state);
            req.agents
                .iter()
                .map(|name| {
                    name.as_ref()
                        .map(|selection| {
                            let name = selection.name();
                            state.house.check_options(name, selection.options())?;
                            state.house.agent_named(name).cloned().ok_or_else(|| {
                                Error::BadRequest(format!(
                                    "the server's agent {name:?} is unavailable"
                                ))
                            })
                        })
                        .transpose()
                })
                .collect::<Result<Vec<_>>>()?
        };
        let users: std::collections::HashSet<&str> =
            agents.iter().flatten().map(|a| a.user.as_str()).collect();
        if rated && users.len() != count {
            return super::bad("a rated game has a different agent in every seat");
        }
        let mut req = req.clone();
        if req.names.len() > usize::from(req.players) {
            return super::bad("more names than players");
        }
        for (i, agent) in agents.iter().enumerate().skip(req.names.len()) {
            req.names.push(agent.as_ref().map_or_else(
                || format!("P{}", i + 1),
                |a| {
                    if agents.iter().flatten().filter(|b| b.user == a.user).count() > 1 {
                        let suffix = format!(" ({})", i + 1);
                        let room = crate::hub::MAX_NAME_CHARS - suffix.len();
                        let name: String = a.name.chars().take(room).collect();
                        format!("{}{suffix}", name.trim_end())
                    } else {
                        a.name.clone()
                    }
                },
            ));
        }
        let config = crate::hub::config(&req)?;
        let names = crate::hub::names(&req)?;
        let owners: Vec<_> = agents
            .iter()
            .map(|a| a.as_ref().map(|a| a.user.clone()))
            .collect();
        let mut meta = crate::hub::Meta {
            host: Some(who.user.clone()),
            owners: owners.clone(),
            watchable: req.watchable.unwrap_or(true),
            timed: true,
            clock: req.clock.map(crate::hub::ClockState::new),
            rated,
            undo: match rated {
                true => lobby::UndoRule::Off,
                false => lobby::UndoRule::Ask,
            },
            agents: agents.iter().map(Option::is_some).collect(),
            launch_options: req
                .agents
                .iter()
                .map(|a| a.as_ref().map(|s| s.options()).unwrap_or_default())
                .collect(),
            ..Default::default()
        };
        crate::hub::brief(&req, &mut meta)?;
        let reservation = self.reserve(&who.user, count as u32)?;
        let mut created = self.hub.host(config, names, meta)?;
        // Agent credentials go only to the runner; the creator gets token seats.
        created
            .seats
            .retain(|s| owners[usize::from(s.seat)].is_none());
        reservation.keep();
        Ok(created)
    }

    /// Attaches the server's agent `agent` to `who`'s own seat `seat` of game `id`: they
    /// go on playing it, and it counts as one of their agent seats for the day.
    pub fn attach_model(
        &self,
        who: &super::Ident,
        id: &str,
        seat: u8,
        agent: &str,
        options: lobby::LaunchOptions,
    ) -> Result<()> {
        if who.agent {
            return Err(Error::Forbidden("a bot plays its seat itself".into()));
        }
        if self.may_seat_agents(&self.user(&who.user)?) {
            self.summon(agent)?;
        }
        let found = {
            let state = lock(&self.state);
            state.house.check_options(agent, options)?;
            let program = (state.house.offer_named(agent))
                .and_then(|o| Some(o.model.as_ref()?.program.as_str()));
            // Its conversation answers decisions only: it has no way to write a note.
            if program == Some(crate::runner::Program::T3Code.label()) {
                return super::bad("a T3 Code agent can't be attached to a seat");
            }
            state.house.agent_named(agent).cloned().ok_or_else(|| {
                Error::BadRequest(format!("the server's agent {agent:?} is unavailable"))
            })?
        };
        let reservation = self.reserve(&who.user, 1)?;
        (self.hub).attach_model(id, seat, &who.user, (found.user, found.name), options)?;
        reservation.keep();
        Ok(())
    }

    /// Sends away the model attached to `who`'s seat `seat` of game `id`.
    pub fn detach_model(&self, who: &super::Ident, id: &str, seat: u8) -> Result<()> {
        self.hub.detach_model(id, seat, &who.user)?;
        self.dispatch();
        Ok(())
    }

    /// The jobs: every seat the runner's agents hold in a running game, or in a finished
    /// one whose debrief they still owe, and every person's seat they are attached to.
    fn jobs(&self, agents: &[Agent], official: &BTreeMap<String, Job>) -> Vec<Job> {
        let mut jobs: Vec<Job> = (agents.iter())
            .flat_map(|agent| {
                (self
                    .hub
                    .seats_of(&agent.user, crate::hub::DEBRIEF_WAIT)
                    .into_iter())
                .filter(|g| g.running || self.hub.debrief_pending(&g.game_id, g.seat))
                .map(|g| {
                    let meta = self.hub.meta(&g.game_id).unwrap_or_default();
                    Job {
                        options: meta
                            .launch_options
                            .get(usize::from(g.seat))
                            .copied()
                            .unwrap_or_default(),
                        official: false,
                        official_account: None,
                        transcript: true,
                        configuration: self.hub.configuration(&g.game_id),
                        agent: agent.name.clone(),
                        brief: meta.briefs.get(usize::from(g.seat)).cloned().flatten(),
                        debrief: meta.debrief,
                        game_id: g.game_id,
                        seat: g.seat,
                        name: g.name,
                        token: g.token,
                        attached: false,
                    }
                })
            })
            .collect();
        jobs.extend(self.hub.attached().into_iter().filter_map(|(g, model)| {
            let agent = agents.iter().find(|a| a.user == model.user)?;
            Some(Job {
                options: model.options,
                official: false,
                official_account: None,
                // It says what its player holds: nobody else reads it.
                transcript: false,
                configuration: self.hub.configuration(&g.game_id),
                agent: agent.name.clone(),
                brief: Some(ATTACHED_BRIEF.replace("{player}", &g.name)),
                debrief: None,
                game_id: g.game_id,
                seat: g.seat,
                name: g.name,
                token: g.token,
                attached: true,
            })
        }));
        jobs.extend(official.values().cloned());
        jobs.sort_by(|a, b| (&a.game_id, a.seat).cmp(&(&b.game_id, b.seat)));
        jobs
    }

    /// Tells the runner its jobs, if they changed since it was told last.
    pub(crate) fn dispatch(&self) {
        let mut state = lock(&self.state);
        let official = state.house.official.clone();
        let Some(runner) = &mut state.house.runner else {
            return;
        };
        let jobs: Vec<_> = self
            .jobs(&runner.playing(), &official)
            .into_iter()
            .filter(|j| {
                j.options == lobby::LaunchOptions::default()
                    || runner.told.as_ref().is_some_and(|jobs| jobs.contains(j))
                    || runner.offers.iter().any(|o| {
                        o.name.eq_ignore_ascii_case(&j.agent)
                            && o.model.as_ref().is_some_and(|m| {
                                (j.options.strategy.is_classic() || m.supports_clock_aware)
                                    && (j.options.speed.is_none() || m.supports_speed)
                            })
                    })
            })
            .filter(|j| !j.official || !j.game_id.is_empty())
            .collect();
        if runner.told.as_ref() != Some(&jobs) {
            let _ = runner.send.send(ToRunner::Jobs { jobs: jobs.clone() });
            runner.told = Some(jobs);
        }
    }

    /// A runner connected, offering `offers`, `capacity` at once; it replaces any other.
    pub fn runner_connected(
        &self,
        offers: Vec<Offer>,
        capacity: u32,
        send: UnboundedSender<ToRunner>,
    ) -> RunnerId {
        let (offers, agents) = self.admit(offers, &send);
        let id = {
            let mut state = lock(&self.state);
            let house = &mut state.house;
            house.remember(&offers);
            house.next_runner += 1;
            house.runner = Some(Runner {
                id: house.next_runner,
                capacity,
                offers,
                agents,
                retired: vec![],
                send,
                told: None,
            });
            house.next_runner
        };
        self.dispatch();
        self.bump();
        id
    }

    /// Runner `id` offers `offers` now.
    pub fn offers_changed(&self, id: RunnerId, offers: Vec<Offer>) {
        let send = {
            let state = lock(&self.state);
            match &state.house.runner {
                Some(r) if r.id == id => r.send.clone(),
                _ => return,
            }
        };
        let (offers, mut agents) = self.admit(offers, &send);
        {
            let mut state = lock(&self.state);
            if state.house.runner.as_ref().is_none_or(|r| r.id != id) {
                return;
            }
            state.house.remember(&offers);
            let Some(runner) = state.house.runner.as_mut() else {
                return;
            };
            // Agents summoned meanwhile stay; those no longer offered go on with their games.
            let mut retired = vec![];
            for agent in runner.agents.drain(..).chain(runner.retired.drain(..)) {
                let key = agent.name.to_lowercase();
                if offers.iter().any(|o| o.name.to_lowercase() == key) {
                    if agents.iter().all(|a| a.user != agent.user) {
                        agents.push(agent);
                    }
                } else if (self.hub.seats_of(&agent.user, Duration::ZERO).iter()).any(|g| g.running)
                {
                    retired.push(agent);
                }
            }
            runner.offers = offers;
            runner.agents = agents;
            runner.retired = retired;
        }
        self.dispatch();
        self.bump();
    }

    /// The offers that may be seated, and those with an account. Each agent plays as the
    /// account of its name, made now unless it is offered on demand; a name taken by
    /// someone else's account is refused.
    fn admit(
        &self,
        offers: Vec<Offer>,
        send: &UnboundedSender<ToRunner>,
    ) -> (Vec<Offer>, Vec<Agent>) {
        let mut admitted: Vec<Offer> = vec![];
        let mut agents: Vec<Agent> = vec![];
        for offer in offers {
            let key = offer.name.trim().to_lowercase();
            if admitted.iter().any(|o| o.name.to_lowercase() == key) {
                continue;
            }
            let account = match offer.on_demand {
                false => self.house_account(&offer.name).map(Some),
                true => self.existing_house_account(&offer.name),
            };
            match account {
                Ok(account) => {
                    if let Some(user) = account {
                        agents.push(Agent {
                            user: user.id,
                            name: user.name,
                            about: offer.about.clone(),
                        });
                    }
                    admitted.push(offer);
                }
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
        (admitted, agents)
    }

    /// Makes the account of the agent offered on demand as `name`, if it has none: before
    /// it is seated. An agent that isn't offered is left to the caller to refuse.
    pub(super) fn summon(&self, name: &str) -> Result<()> {
        let mut state = lock(&self.state);
        if state.house.agent_named(name).is_some() {
            return Ok(());
        }
        let Some(offer) = state.house.offer_named(name).cloned() else {
            return Ok(());
        };
        let user = self.house_account(&offer.name)?;
        if let Some(runner) = state.house.runner.as_mut() {
            runner.retired.retain(|a| a.user != user.id);
            runner.agents.push(Agent {
                user: user.id,
                name: user.name,
                about: offer.about,
            });
        }
        Ok(())
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
        if let Some(user) = self.existing_house_account(name)? {
            return Ok(user);
        }
        // Another may have made it meanwhile.
        (self
            .db
            .add_user(name.trim(), &Login::House, now_ms() / 1000))
        .or_else(|e| self.existing_house_account(name)?.ok_or(e))
    }

    fn existing_house_account(&self, name: &str) -> Result<Option<User>> {
        let name = super::accounts::house_name(name)?;
        match self.db.user_by_name(&name)? {
            Some(user) if matches!(user.login, Login::House) => Ok(Some(user)),
            Some(_) => conflict(format!("the name {name} is someone else's")),
            None => Ok(None),
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
        if !self.may_seat_agents(&self.user(host)?) {
            return Err(no_agents());
        }
        let per_day = self.settings.agent_seats_per_day;
        let mut state = lock(&self.state);
        let Some(runner) = &state.house.runner else {
            return conflict("the server's agents aren't available now");
        };
        let busy =
            self.jobs(&runner.playing(), &state.house.official).len() as u32 + state.house.reserved;
        let capacity = runner.capacity;
        if busy + seats > capacity {
            return conflict(format!(
                "the server's agents are busy ({busy} of {capacity} playing); try again when a game ends"
            ));
        }
        let filled = state.house.filled_today(host);
        if per_day != 0 && filled + seats > per_day {
            return Err(Error::TooMany(
                format!("you have filled {filled} of your {per_day} agent seats for today"),
                Duration::from_secs(3600),
            ));
        }
        state.house.reserved += seats;
        Ok(reservation(seats))
    }

    /// Whether `user` may seat the server's agents: an account `agent_players` names, or
    /// without that list, any registered player.
    pub(super) fn may_seat_agents(&self, user: &User) -> bool {
        match (&self.settings.agent_players, &user.login) {
            (_, Login::Guest | Login::House) => false,
            (None, _) => user.kind() == UserKind::Registered,
            (Some(allowed), _) => {
                (allowed.iter()).any(|name| name.trim().eq_ignore_ascii_case(&user.name))
            }
        }
    }

    /// What the lobby says about the server's agents, for `you`.
    pub(super) fn house_info(&self, state: &mut State, you: Option<&User>) -> lobby::House {
        let per_day = self.settings.agent_seats_per_day;
        let left = you
            .filter(|u| self.may_seat_agents(u))
            .map(|u| match per_day {
                0 => lobby::House::UNLIMITED,
                _ => per_day.saturating_sub(state.house.filled_today(&u.id)),
            });
        let Some(runner) = &state.house.runner else {
            return lobby::House {
                left,
                ..lobby::House::default()
            };
        };
        let jobs = self.jobs(&runner.playing(), &state.house.official);
        let busy = jobs.len() as u32 + state.house.reserved;
        lobby::House {
            models: state.house.models(),
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

impl Arena {
    /// The agents an official account may be sent: every one the runner offers, by name
    /// and by model.
    pub(crate) fn official_agents(&self) -> (Vec<String>, Vec<HouseModel>) {
        let state = lock(&self.state);
        let names = (state.house.runner.iter())
            .flat_map(|r| &r.offers)
            .map(|o| o.name.clone())
            .collect();
        (names, state.house.models())
    }
    /// Keeps a runner slot for official account `account`'s next game.
    pub(crate) fn reserve_official(
        &self,
        owner: &str,
        account: &str,
        agent: &str,
        options: lobby::LaunchOptions,
    ) -> Result<Reservation<'_>> {
        self.summon(agent)?;
        let mut state = lock(&self.state);
        let Some(runner) = &state.house.runner else {
            return conflict("the agent runner is disconnected");
        };
        if state.house.agent_named(agent).is_none() {
            return conflict("selected agent is unavailable");
        }
        state.house.check_options(agent, options)?;
        if state.house.official.contains_key(account) {
            return conflict("an agent is already assigned to this official account");
        }
        let busy =
            self.jobs(&runner.playing(), &state.house.official).len() as u32 + state.house.reserved;
        if busy >= runner.capacity {
            return conflict("the agent runner is at capacity");
        }
        state.house.reserved += 1;
        Ok(Reservation {
            arena: self,
            host: owner.into(),
            seats: 1,
            kept: false,
        })
    }
    pub(crate) fn official_job(&self, account: &str) -> Option<Job> {
        lock(&self.state).house.official.get(account).cloned()
    }
    /// Sets official account `account`'s job; the job is marked as that account's.
    pub(crate) fn set_official_job(&self, account: &str, job: Option<Job>) {
        {
            let state = &mut *lock(&self.state);
            match job {
                Some(mut job) => {
                    job.official = true;
                    job.official_account = Some(account.into());
                    super::official::played_by(state, account, &job);
                    state.house.official.insert(account.into(), job);
                }
                None => {
                    state.house.official.remove(account);
                }
            }
        }
        self.dispatch();
        self.bump();
    }
}
