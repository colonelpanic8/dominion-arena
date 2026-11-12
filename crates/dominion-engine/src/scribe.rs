//! Keeps the public log complete. What changes without an event of its own (counters and
//! tokens, cards moved by an effect) is logged before the next event or decision, after
//! the ability it happened in (see [`Event::Resolved`]).

use crate::event::{Counter, Event, Marker};
use crate::game::{Mat, Pos, Token, Zone};
use crate::{Card, CardId, Game, PileId, PlayerId};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub(crate) struct Scribe {
    /// Counters and tokens as the log last showed them; `None` until the game starts.
    tally: Option<Tally>,
    /// Card moves no event has accounted for yet.
    moves: Vec<Unlogged>,
    /// Cards shown to everyone since they last moved.
    shown: Vec<CardId>,
    /// Abilities resolving, innermost last.
    resolving: Vec<Resolving>,
    #[serde(default)]
    searched_discards: Vec<(PlayerId, usize)>,
    #[serde(default)]
    actor: Option<PlayerId>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Unlogged {
    card: CardId,
    #[serde(default)]
    actor: Option<PlayerId>,
    from: Zone,
    to: Zone,
    bottom: bool,
    #[serde(default)]
    under: usize,
    /// Everyone knew which card it was.
    public: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Resolving {
    /// How many plays were in progress when it began: +$ and the like are its own only
    /// while no other play has begun.
    plays: usize,
    /// Logged before the first thing it does.
    #[serde(default)]
    header: Option<Event>,
    /// What the play in progress did instead of its own effect when the ability began.
    #[serde(default)]
    play: (Option<crate::Plus>, bool),
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
struct Tally {
    players: Vec<PlayerTally>,
    piles: Vec<[i32; 4]>,
    trade_route_mat: u32,
    suns: u32,
    #[serde(default)]
    plot: Vec<u32>,
    #[serde(default)]
    inventors: BTreeMap<PileId, u32>,
    /// Tokens on the cards of lasting effects (Garrison's), by the effect's id.
    #[serde(default)]
    lasting_tokens: BTreeMap<u32, (PlayerId, Card, u32)>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
struct PlayerTally {
    counters: [i32; 6],
    tokens: BTreeMap<Token, Option<PileId>>,
    markers: [bool; 3],
    states: Vec<Card>,
}

const PLAYER_COUNTERS: [Counter; 6] = [
    Counter::Coffers,
    Counter::Villagers,
    Counter::Favors,
    Counter::Debt,
    Counter::VpTokens,
    Counter::PirateShip,
];
const PILE_COUNTERS: [Counter; 4] = [
    Counter::VpTokens,
    Counter::Debt,
    Counter::Embargo,
    Counter::TradeRoute,
];
const MARKERS: [Marker; 3] = [Marker::MinusCard, Marker::MinusCoin, Marker::Journey];

impl Game {
    /// Starts logging changes: what setup did is not part of the story.
    pub(crate) fn start_log(&mut self) {
        self.scribe.moves.clear();
        self.scribe.tally = Some(self.tally());
    }

    fn tally(&self) -> Tally {
        Tally {
            players: self
                .players
                .iter()
                .map(|p| PlayerTally {
                    counters: [
                        p.coffers as i32,
                        p.villagers as i32,
                        p.favors as i32,
                        p.debt as i32,
                        p.vp_tokens,
                        p.pirate_ship as i32,
                    ],
                    tokens: p.tokens.clone(),
                    markers: [p.minus_card, p.minus_coin, p.journey_face_up],
                    states: p.states.clone(),
                })
                .collect(),
            piles: self
                .piles
                .iter()
                .map(|p| {
                    [
                        p.vp_tokens,
                        p.debt_tokens,
                        p.embargo as i32,
                        p.trade_route as i32,
                    ]
                })
                .collect(),
            trade_route_mat: self.trade_route_mat,
            suns: self.prophecy.map_or(0, |(_, n)| n),
            plot: self.ext.renaissance.plot.clone(),
            inventors: self.ext.allies.inventors.iter().copied().collect(),
            lasting_tokens: self
                .lasting
                .iter()
                .filter_map(|l| {
                    let n = crate::cards::lasting_tokens(l).filter(|&n| n > 0)?;
                    Some((l.id, (l.owner, l.card, n)))
                })
                .collect(),
        }
    }

    /// Whether the log is running: the game has started.
    pub(crate) fn logging(&self) -> bool {
        self.scribe.tally.is_some()
    }

    /// Whether everyone knows which card `id` is, where it is now.
    pub(crate) fn is_public(&self, id: CardId) -> bool {
        if self.scribe.shown.contains(&id) {
            return true;
        }
        match self.zone(id) {
            Zone::InPlay(_) | Zone::Trash => true,
            Zone::Pile(p) => !self.piles[p.index()].hidden,
            Zone::Mat(_, mat) => mat != Mat::NativeVillage,
            Zone::SetAside(p) => self.players[p.index()].face_up.contains(&id),
            // Revealed ones are shown, until put back in an order no one else follows.
            Zone::Looking(_) => false,
            // The top card, unless the pile is being gone through: looked through, or
            // with cards already taken from it (the next one was never on top).
            Zone::Discard(p) => {
                self.players[p.index()].discard.last() == Some(&id)
                    && !self.scribe.searched_discards.iter().any(|&(q, _)| p == q)
                    && !self.scribe.moves.iter().any(|m| m.from == Zone::Discard(p))
            }
            Zone::Hand(_) | Zone::Deck(_) => false,
        }
    }

    pub(crate) fn note_discard_search(&mut self, player: PlayerId) {
        let search = (player, self.stack.len());
        if !self.scribe.searched_discards.contains(&search) {
            self.scribe.searched_discards.push(search);
        }
    }

    pub(crate) fn end_discard_searches(&mut self) {
        self.scribe
            .searched_discards
            .retain(|&(_, depth)| depth < self.stack.len());
    }

    pub(crate) fn log_actor(&mut self, actor: Option<PlayerId>) {
        self.scribe.actor = actor;
    }

    /// Records that `id` is moving from where it is to `to`, for the log.
    pub(crate) fn note_move(&mut self, id: CardId, to: Zone, pos: Pos) {
        if self.scribe.tally.is_none() {
            return;
        }
        let from = self.zone(id);
        let public = self.is_public(id)
            || match to {
                Zone::InPlay(_) | Zone::Trash => true,
                Zone::Pile(p) => !self.piles[p.index()].hidden,
                Zone::Mat(_, mat) => mat != Mat::NativeVillage,
                _ => false,
            };
        self.scribe.shown.retain(|&c| c != id);
        self.scribe.moves.push(Unlogged {
            card: id,
            actor: self.scribe.actor,
            from,
            to,
            bottom: pos == Pos::Bottom,
            under: match pos {
                Pos::Under(n) => n,
                _ => 0,
            },
            public,
        });
    }

    /// Moves `id` without a line in the log (Clean-up's discards, an opening hand dealt again).
    pub(crate) fn move_unlogged(&mut self, id: CardId, to: Zone) {
        self.move_card(id, to, crate::game::Pos::Top);
        self.scribe.moves.retain(|m| m.card != id);
    }

    /// Everyone saw which card `id` is.
    pub(crate) fn note_shown(&mut self, id: CardId) {
        if !self.scribe.shown.contains(&id) {
            self.scribe.shown.push(id);
        }
    }

    /// At a decision: a card shown in a hand (or set aside face down) can't be told from
    /// the others there any more once its owner may have added or chosen cards since.
    pub(crate) fn forget_hidden_shows(&mut self) {
        let shown = std::mem::take(&mut self.scribe.shown);
        self.scribe.shown = shown
            .into_iter()
            .filter(|&c| match self.zone(c) {
                Zone::Hand(_) | Zone::Mat(_, Mat::NativeVillage) => false,
                Zone::SetAside(p) | Zone::Looking(p) => {
                    self.players[p.index()].face_up.contains(&c)
                }
                _ => true,
            })
            .collect();
    }

    /// No one can follow `cards` any more (shuffled, or put back in a private order).
    pub(crate) fn note_mixed(&mut self, cards: &[CardId]) {
        self.scribe.shown.retain(|c| !cards.contains(c));
    }

    /// An ability begins to resolve; `header`, if any, is logged before the first thing it
    /// does.
    pub(crate) fn begin_resolving(&mut self, header: Option<Event>) {
        self.scribe.resolving.push(Resolving {
            plays: self.plays.len(),
            header,
            play: self.play_changes(),
        });
    }

    fn play_changes(&self) -> (Option<crate::Plus>, bool) {
        self.plays
            .last()
            .map_or((None, false), |st| (st.replaced, st.blocked))
    }

    pub(crate) fn end_resolving(&mut self) {
        self.sync_log(None);
        let changes = self.play_changes();
        // An ability that only changed what the play in progress does (Enchantress,
        // Highwayman) has no other line.
        if let Some(r) = self.scribe.resolving.pop()
            && let Some(header) = r.header
            && r.plays == self.plays.len()
            && r.play != changes
        {
            self.log.push(header);
        }
    }

    pub(crate) fn log_choice(&mut self, decision: &crate::Decision, response: &crate::Response) {
        use crate::{DecisionKind, Response};
        if !decision.public_choice {
            return;
        }
        let choices = match (&decision.kind, response) {
            (
                DecisionKind::Choose {
                    options, labels, ..
                },
                Response::Options(chosen),
            ) => {
                if decision.about.iter().any(|&c| !self.is_public(c)) {
                    return;
                }
                options
                    .iter()
                    .zip(labels)
                    .filter(|(o, _)| chosen.contains(o))
                    .map(|(_, label)| label.clone())
                    .collect()
            }
            (DecisionKind::Number { .. }, Response::Number(n)) => {
                vec![match &decision.question {
                    Some(question) => format!("{n} ({question})"),
                    None => n.to_string(),
                }]
            }
            _ => return,
        };
        self.emit(Event::Chosen {
            player: decision.player,
            source: decision.source,
            choices,
        });
    }

    /// Logs +Actions, +Buys and +$ given outside a card's own play.
    pub(crate) fn note_bonus(&mut self, player: PlayerId, actions: u32, buys: u32, coins: u32) {
        let own = self
            .scribe
            .resolving
            .last()
            .is_some_and(|r| r.plays == self.plays.len());
        if own && actions + buys + coins > 0 {
            let e = Event::Bonus {
                player,
                actions,
                buys,
                coins,
            };
            self.sync_log(Some(&e));
            self.log.push(e);
        }
    }

    /// Logs what changed without an event since the log was last brought up to date, the
    /// moves `event` (about to be logged) accounts for aside.
    pub(crate) fn sync_log(&mut self, event: Option<&Event>) {
        // A game saved before the log kept track starts keeping track now.
        let Some(old) = self.scribe.tally.take() else {
            self.scribe.tally = Some(self.tally());
            return;
        };
        if let Some(e) = event {
            self.explain(e);
        }
        let mut lines = self.unlogged_moves();
        let new = self.tally();
        if new != old {
            counter_changes(&old, &new, &mut lines);
        }
        self.scribe.tally = Some(new);
        if lines.is_empty() && event.is_none() {
            return;
        }
        for r in &mut self.scribe.resolving {
            if let Some(h) = r.header.take() {
                self.log.push(h);
            }
        }
        self.log.extend(lines);
    }

    /// Drops the pending moves `e` reports.
    fn explain(&mut self, e: &Event) {
        let (to, count, card) = match *e {
            Event::Drew { player, count } => (Zone::Hand(player), count, None),
            Event::Discarded { player, card } => (Zone::Discard(player), 1, Some(card)),
            Event::Trashed { card, .. } => (Zone::Trash, 1, Some(card)),
            Event::Gained { player, card, to } => {
                let zone = match to {
                    crate::Dest::Discard => Zone::Discard(player),
                    crate::Dest::Hand => Zone::Hand(player),
                    crate::Dest::DeckTop => Zone::Deck(player),
                    crate::Dest::SetAside => Zone::SetAside(player),
                };
                (zone, 1, Some(card))
            }
            Event::TopDecked { player, card } => (Zone::Deck(player), 1, card),
            Event::SetAside { player, card } => (Zone::SetAside(player), 1, Some(card)),
            Event::Played { player, card } | Event::Called { player, card } => {
                (Zone::InPlay(player), 1, Some(card))
            }
            Event::Exchanged { player, card, into } => {
                self.account(|z| matches!(z, Zone::Pile(_)), 1, Some(card));
                (Zone::Discard(player), 1, Some(into))
            }
            _ => return,
        };
        self.account(|z| z == to, count, card);
    }

    /// Drops the last `count` pending moves of `card` (any card if `None`) to a zone `to`
    /// accepts.
    fn account(&mut self, to: impl Fn(Zone) -> bool, count: usize, card: Option<Card>) {
        let mut left = count;
        let mut i = self.scribe.moves.len();
        while left > 0 && i > 0 {
            i -= 1;
            let m = &self.scribe.moves[i];
            let id = m.card;
            if to(m.to) && card.is_none_or(|c| self.card(id) == c || self.printed(id) == c) {
                self.scribe.moves.remove(i);
                left -= 1;
            }
        }
    }

    /// The pending moves as log lines: consecutive moves between the same zones together.
    fn unlogged_moves(&mut self) -> Vec<Event> {
        let moves = std::mem::take(&mut self.scribe.moves);
        let mut lines: Vec<Event> = vec![];
        for m in moves {
            let card = m.public.then(|| self.card(m.card));
            let player = m
                .from
                .owner()
                .or(m.to.owner())
                .or(m.actor)
                .unwrap_or(self.turn.player);
            if let Some(Event::Moved {
                from,
                to,
                cards,
                count,
                bottom,
                under,
                player: previous,
            }) = lines.last_mut()
                && (*from, *to, *bottom, *under, *previous)
                    == (Some(m.from), m.to, m.bottom, m.under, player)
            {
                cards.extend(card);
                *count += 1;
                continue;
            }
            lines.push(Event::Moved {
                player,
                from: Some(m.from),
                to: m.to,
                cards: card.into_iter().collect(),
                count: 1,
                bottom: m.bottom,
                under: m.under,
            });
        }
        lines
    }
}

fn counter_changes(old: &Tally, new: &Tally, out: &mut Vec<Event>) {
    for (i, now) in new.players.iter().enumerate() {
        let player = PlayerId(i as u8);
        let was = old.players.get(i).cloned().unwrap_or_default();
        for (j, &counter) in PLAYER_COUNTERS.iter().enumerate() {
            let delta = now.counters[j] - was.counters[j];
            if delta != 0 {
                out.push(Event::Counter {
                    player: Some(player),
                    pile: None,
                    counter,
                    delta,
                });
            }
        }
        for (&token, &pile) in &now.tokens {
            if was.tokens.get(&token) != Some(&pile) {
                out.push(Event::TokenMoved {
                    player,
                    token,
                    pile,
                });
            }
        }
        for &token in was.tokens.keys() {
            if !now.tokens.contains_key(&token) {
                out.push(Event::TokenMoved {
                    player,
                    token,
                    pile: None,
                });
            }
        }
        for (j, &marker) in MARKERS.iter().enumerate() {
            if now.markers[j] != was.markers[j] {
                out.push(Event::Marker {
                    player,
                    marker,
                    on: now.markers[j],
                });
            }
        }
        for &card in &was.states {
            if !now.states.contains(&card) {
                out.push(Event::Returned { player, card });
            }
        }
    }
    for (i, now) in new.piles.iter().enumerate() {
        let was = old.piles.get(i).copied().unwrap_or_default();
        for (j, &counter) in PILE_COUNTERS.iter().enumerate() {
            let delta = now[j] - was[j];
            if delta != 0 {
                out.push(Event::Counter {
                    player: None,
                    pile: Some(PileId(i as u8)),
                    counter,
                    delta,
                });
            }
        }
    }
    for (i, &now) in new.plot.iter().enumerate() {
        let was = old.plot.get(i).copied().unwrap_or(0);
        if now != was {
            out.push(Event::CardCounter {
                player: Some(PlayerId(i as u8)),
                card: Card::SinisterPlot,
                pile: None,
                delta: now as i32 - was as i32,
            });
        }
    }
    let piles: std::collections::BTreeSet<_> = old
        .inventors
        .keys()
        .chain(new.inventors.keys())
        .copied()
        .collect();
    for pile in piles {
        let delta = new.inventors.get(&pile).copied().unwrap_or(0) as i32
            - old.inventors.get(&pile).copied().unwrap_or(0) as i32;
        if delta != 0 {
            out.push(Event::CardCounter {
                player: None,
                card: Card::FamilyOfInventors,
                pile: Some(pile),
                delta,
            });
        }
    }
    let ids: std::collections::BTreeSet<_> = old
        .lasting_tokens
        .keys()
        .chain(new.lasting_tokens.keys())
        .copied()
        .collect();
    for id in ids {
        let (was, now) = (old.lasting_tokens.get(&id), new.lasting_tokens.get(&id));
        let Some(&(player, card, _)) = now.or(was) else {
            continue;
        };
        let delta = now.map_or(0, |t| t.2 as i32) - was.map_or(0, |t| t.2 as i32);
        if delta != 0 {
            out.push(Event::CardCounter {
                player: Some(player),
                card,
                pile: None,
                delta,
            });
        }
    }
    for (counter, now, was) in [
        (
            Counter::TradeRoute,
            new.trade_route_mat,
            old.trade_route_mat,
        ),
        (Counter::Sun, new.suns, old.suns),
    ] {
        if now != was {
            out.push(Event::Counter {
                player: None,
                pile: None,
                counter,
                delta: now as i32 - was as i32,
            });
        }
    }
}
