//! An opt-in, ordered record of everything that changes on the table: what the
//! `dominion/events@1` stream is written from. [`Game::new_journaled`] turns it on.
//!
//! It holds the [`Trace`] entries (every card created, moved, mixed and shown) with the
//! rules' word for each move, the log entries in their place among them, and every
//! counter as it changes: counters are read whenever something is recorded, so a change
//! lies between the same two card moves it happened between.

use crate::game::{Phase, Pos, Token};
use crate::{
    Card, CardId, Cost, Game, Kind, Note, PileId, PileStructure, PlayerId, SetupRole, Trace, Types,
    Zone,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// What kind of move a card made: the rules' word for it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Verb {
    Play,
    Draw,
    Discard,
    Gain,
    Trash,
    Topdeck,
    Bottomdeck,
    /// Into a deck under some of its cards.
    Insert,
    Reveal,
    Look,
    SetAside,
    Exile,
    /// Any other placement: into a hand, onto a mat, a deck into its discard pile.
    Put,
    /// To its pile.
    Return,
    /// One of an exchange's two moves.
    Exchange,
    Shuffle,
    /// From the top of a pile to its bottom.
    Rotate,
}

/// A counter the journal follows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Gauge {
    Actions,
    Buys,
    Coins,
    Potions,
    Coffers(PlayerId),
    Villagers(PlayerId),
    Favors(PlayerId),
    Debt(PlayerId),
    VpTokens(PlayerId),
    PirateShip(PlayerId),
    PileVp(PileId),
    PileDebt(PileId),
    Embargo(PileId),
    TradeRoute(PileId),
    TradeRouteMat,
    Sun,
    /// VP tokens on a Landmark.
    LandscapeVp(Card),
    /// The player's tokens on Sinister Plot.
    SinisterPlot(PlayerId),
    /// Family of Inventors' Favor tokens on a pile.
    PileFavors(PileId),
}

/// Where a Boon or Hex is: its deck, its discard pile, set aside for Druid, or with a
/// player who took it, is receiving it or keeps it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Spot {
    Deck,
    Discard,
    Druid,
    Player(PlayerId),
}

/// A player's token: an Adventures token, or the -1 Card, -$1 or Journey token.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum TokenKind {
    Adventures(Token),
    MinusCard,
    MinusCoin,
    Journey,
}

/// Where a token that is out is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TokenAt {
    Pile(PileId),
    /// With its player, on no pile.
    Player,
    /// The Journey token, face down.
    FaceDown,
}

/// A pile's properties.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PileProps {
    pub in_supply: bool,
    pub hidden: bool,
    /// Removed from the game by Divine Wind; never set for a holder, which is no pile.
    pub removed: bool,
    pub roles: Vec<(Card, SetupRole)>,
    pub traits: Vec<Card>,
    pub structure: Option<PileStructure>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Step {
    /// A trace entry; `verb` is how its card moved, for a [`Trace::Moved`].
    Trace {
        entry: Trace,
        verb: Option<Verb>,
    },
    /// `player` plays `card`: its instructions are about to be followed. The move right
    /// before it, if its verb is [`Verb::Play`], put it into play.
    Played {
        player: PlayerId,
        card: CardId,
    },
    /// [`Game::log`]'s entry at `index` was logged here.
    Logged {
        index: usize,
    },
    /// A turn began: `player`'s `number`th, an extra one from `extra`. Its counters are 1
    /// Action, 1 Buy, $0 and no Potions, and its phase [`Phase::Start`].
    Turn {
        player: PlayerId,
        number: u32,
        extra: Option<Card>,
    },
    Phase(Phase),
    /// `gauge` changed from `was` to `now`.
    Gauge {
        gauge: Gauge,
        was: i64,
        now: i64,
    },
    /// A Boon or Hex went to `to`.
    Fate {
        card: Card,
        to: Spot,
    },
    /// The Boons' (or Hexes') discard pile was shuffled into their empty deck: the deck
    /// now, top first.
    FateShuffled {
        hexes: bool,
        cards: Vec<Card>,
    },
    /// Boons or Hexes came into the game at `to`, each on top of the one before.
    FateAdded {
        cards: Vec<Card>,
        to: Spot,
    },
    /// `player`'s `token` is now `at`, or no longer out.
    Token {
        player: PlayerId,
        token: TokenKind,
        at: Option<TokenAt>,
    },
    /// A Project cube, Artifact or State changed hands.
    Holding {
        card: Card,
        from: Option<PlayerId>,
        to: Option<PlayerId>,
    },
    /// A pile's properties changed, or it joined the game.
    Pile {
        pile: PileId,
        props: PileProps,
    },
    /// A landscape or Artifact joined the game, or became its Prophecy.
    Landscape {
        card: Card,
        prophecy: bool,
    },
    /// The public notes the table keeps (see [`table_note`]) are now these.
    Notes(Vec<Note>),
    /// Derived values changed: what cards cost, and their types (`None`: the printed ones
    /// again).
    Derived {
        costs: Vec<(Card, Cost)>,
        types: Vec<(Card, Option<Types>)>,
    },
}

/// What the journal reads of the table besides counters, each time it records.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
struct Holdings {
    tokens: BTreeMap<(PlayerId, TokenKind), TokenAt>,
    held: BTreeMap<Card, Vec<PlayerId>>,
    piles: Vec<PileProps>,
    landscapes: Vec<Card>,
    prophecy: Option<Card>,
    notes: Vec<Note>,
    costs: BTreeMap<Card, Cost>,
    types: BTreeMap<Card, Types>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Journal {
    steps: Vec<Step>,
    /// What the gauges read when last recorded, in [`Game::gauges`]' order.
    readings: Vec<(Gauge, i64)>,
    phase: Phase,
    #[serde(default)]
    holdings: Holdings,
}

impl Game {
    /// Starts a game that records a journal (and a [`Trace`]), beginning with every card
    /// in it.
    pub fn new_journaled(config: crate::GameConfig) -> Result<Game, crate::SetupError> {
        let mut game = crate::setup::build(config)?;
        game.enable_trace();
        let steps = (game.trace().unwrap_or_default().iter().cloned())
            .map(|entry| Step::Trace { entry, verb: None })
            .collect();
        // What setup gave (starting Favors, tokens on piles) is journaled after the cards.
        let turn = |g: &Gauge| {
            matches!(
                g,
                Gauge::Actions | Gauge::Buys | Gauge::Coins | Gauge::Potions
            )
        };
        game.journal = Some(Journal {
            steps,
            readings: (game.gauges().into_iter())
                .filter(|(g, _)| turn(g))
                .collect(),
            phase: game.turn.phase,
            holdings: Holdings::default(),
        });
        let druid = game.ext.nocturne.druid.clone();
        for (cards, to) in [
            (game.boons.draw.clone(), Spot::Deck),
            (game.boons.discard.clone(), Spot::Discard),
            (druid, Spot::Druid),
            (game.hexes.draw.clone(), Spot::Deck),
            (game.hexes.discard.clone(), Spot::Discard),
        ] {
            if !cards.is_empty() {
                game.journal_push(Step::FateAdded { cards, to });
            }
        }
        game.sync_journal();
        game.start();
        Ok(game)
    }

    /// Takes the journal recorded so far, leaving it empty (and still recording).
    pub fn take_journal(&mut self) -> Vec<Step> {
        self.sync_journal();
        (self.journal.as_mut())
            .map(|j| std::mem::take(&mut j.steps))
            .unwrap_or_default()
    }

    /// Moves `id` as the rules' `verb`.
    pub(crate) fn move_as(&mut self, verb: Verb, id: CardId, to: Zone, pos: Pos) {
        self.verb = Some(verb);
        self.move_card(id, to, pos);
        self.verb = None;
    }

    pub(crate) fn journal_trace(&mut self, entry: &Trace) {
        if self.journal.is_none() {
            return;
        }
        self.sync_journal();
        let verb = match *entry {
            Trace::Moved {
                to, bottom, under, ..
            } => Some(self.verb.unwrap_or(match to {
                Zone::Deck(_) if bottom => Verb::Bottomdeck,
                Zone::Deck(_) if under > 0 => Verb::Insert,
                Zone::Deck(_) => Verb::Topdeck,
                Zone::Mat(_, crate::Mat::Exile) => Verb::Exile,
                Zone::SetAside(_) => Verb::SetAside,
                Zone::Looking(_) => Verb::Look,
                Zone::Pile(_) => Verb::Return,
                _ => Verb::Put,
            })),
            _ => None,
        };
        self.journal_push(Step::Trace {
            entry: entry.clone(),
            verb,
        });
    }

    /// Journals cards rearranged within `zone` with no trace entry of their own: each of
    /// `cards` in turn left it (from its top, when `top`) and came back at `pos`.
    pub(crate) fn journal_within(
        &mut self,
        verb: Verb,
        zone: Zone,
        cards: &[CardId],
        top: bool,
        pos: Pos,
    ) {
        if self.journal.is_none() {
            return;
        }
        self.sync_journal();
        for &card in cards {
            self.journal_push(Step::Trace {
                entry: Trace::Moved {
                    card,
                    from: zone,
                    to: zone,
                    top,
                    bottom: pos == Pos::Bottom,
                    under: match pos {
                        Pos::Under(n) => n,
                        _ => 0,
                    },
                },
                verb: Some(verb),
            });
        }
    }

    pub(crate) fn journal_played(&mut self, player: PlayerId, card: CardId) {
        if self.journal.is_some() {
            self.journal_push(Step::Played { player, card });
        }
    }

    pub(crate) fn journal_logged(&mut self, index: usize) {
        if self.journal.is_some() {
            self.sync_journal();
            self.journal_push(Step::Logged { index });
        }
    }

    /// Records the turn [`Game::turn`] was just made.
    pub(crate) fn journal_turn(&mut self) {
        let Some(journal) = &mut self.journal else {
            return;
        };
        let p = self.turn.player;
        journal.steps.push(Step::Turn {
            player: p,
            number: self.players[p.index()].turns_taken + 1,
            extra: (self.turn.extra).or(self.ending.is_some().then_some(Card::Fleet)),
        });
        journal.phase = Phase::Start;
        for (gauge, value) in &mut journal.readings {
            match gauge {
                Gauge::Actions | Gauge::Buys => *value = 1,
                Gauge::Coins | Gauge::Potions => *value = 0,
                _ => {}
            }
        }
    }

    /// Records that the Boon or Hex `card` went to `to`.
    pub(crate) fn journal_fate(&mut self, card: Card, to: Spot) {
        if self.journal.is_some() {
            self.sync_journal();
            self.journal_push(Step::Fate { card, to });
        }
    }

    /// Records the Boons or Hexes deck just shuffled from its discard pile.
    pub(crate) fn journal_fate_shuffled(&mut self, hexes: bool) {
        if self.journal.is_some() {
            self.sync_journal();
            let deck = if hexes { &self.hexes } else { &self.boons };
            let cards = deck.draw.iter().rev().copied().collect();
            self.journal_push(Step::FateShuffled { hexes, cards });
        }
    }

    /// Records Boons or Hexes that came into the game at `to`, bottom first.
    pub(crate) fn journal_fate_added(&mut self, cards: Vec<Card>, to: Spot) {
        if self.journal.is_some() && !cards.is_empty() {
            self.sync_journal();
            self.journal_push(Step::FateAdded { cards, to });
        }
    }

    /// Records the gauges, the phase and the rest of the table that changed since they
    /// were last recorded.
    pub(crate) fn sync_journal(&mut self) {
        if self.journal.is_none() {
            return;
        }
        let holdings = self.holdings();
        let now = self.gauges();
        let phase = self.turn.phase;
        let journal = self.journal.as_mut().expect("checked above");
        if journal.phase != phase {
            journal.phase = phase;
            journal.steps.push(Step::Phase(phase));
        }
        for &(gauge, value) in &now {
            let was = (journal.readings.iter())
                .find(|(g, _)| *g == gauge)
                .map_or(0, |&(_, v)| v);
            if was != value {
                journal.steps.push(Step::Gauge {
                    gauge,
                    was,
                    now: value,
                });
            }
        }
        // A gauge that is no longer read (Favors on a pile Divine Wind removed) is 0.
        for &(gauge, was) in &journal.readings {
            if was != 0 && !now.iter().any(|(g, _)| *g == gauge) {
                journal.steps.push(Step::Gauge { gauge, was, now: 0 });
            }
        }
        journal.readings = now;
        let was = std::mem::replace(&mut journal.holdings, holdings);
        holding_changes(&was, &journal.holdings, &mut journal.steps);
    }

    fn holdings(&self) -> Holdings {
        let mut tokens = BTreeMap::new();
        let mut held = BTreeMap::<Card, Vec<PlayerId>>::new();
        for (i, pl) in self.players.iter().enumerate() {
            let p = PlayerId(i as u8);
            for (&token, &pile) in &pl.tokens {
                let at = pile.map_or(TokenAt::Player, TokenAt::Pile);
                tokens.insert((p, TokenKind::Adventures(token)), at);
            }
            for (on, token, at) in [
                (pl.minus_card, TokenKind::MinusCard, TokenAt::Player),
                (pl.minus_coin, TokenKind::MinusCoin, TokenAt::Player),
                (!pl.journey_face_up, TokenKind::Journey, TokenAt::FaceDown),
            ] {
                if on {
                    tokens.insert((p, token), at);
                }
            }
            for &card in pl.projects.iter().chain(&pl.states) {
                held.entry(card).or_default().push(p);
            }
        }
        for &(card, holder) in &self.artifacts {
            if let Some(p) = holder {
                held.entry(card).or_default().push(p);
            }
        }
        for holders in held.values_mut() {
            holders.sort();
        }
        let notes = crate::cards::notes(self);
        let piles = (self.piles.iter().enumerate())
            .map(|(i, pile)| {
                let id = PileId(i as u8);
                let roles = (notes.iter())
                    .filter_map(|n| Some((n.source, n.setup?, n.pile.or_else(|| self.held_by(n)))))
                    .filter(|&(_, _, pile)| pile == Some(id))
                    .map(|(source, role, _)| (source, role));
                let mut roles: Vec<(Card, SetupRole)> = roles.collect();
                roles.dedup();
                PileProps {
                    in_supply: pile.in_supply,
                    hidden: pile.hidden,
                    removed: pile.removed && !pile.holder,
                    roles,
                    traits: pile.traits.clone(),
                    structure: pile.structure.clone(),
                }
            })
            .collect();
        let landscapes = (self.landscapes.iter())
            .chain(self.artifacts.iter().map(|(card, _)| card))
            .copied()
            .collect();
        let mut names: Vec<Card> = self.cards.clone();
        names.sort();
        names.dedup();
        let costs = names.iter().map(|&c| (c, self.cost(c))).collect();
        let types = (names.iter())
            .map(|&c| (c, self.card_types(c)))
            .filter(|&(c, t)| t != c.types())
            .collect();
        Holdings {
            tokens,
            held,
            piles,
            landscapes,
            prophecy: self.prophecy.map(|(card, _)| card),
            notes: notes.into_iter().filter(table_note).collect(),
            costs,
            types,
        }
    }

    /// The pile holding the card a [`SetupRole::SetAside`] note names (Riverboat's).
    fn held_by(&self, note: &Note) -> Option<PileId> {
        let card = note
            .card
            .filter(|_| note.setup == Some(SetupRole::SetAside))?;
        (self.piles.iter())
            .position(|p| p.holder && p.card == card)
            .map(|i| PileId(i as u8))
    }

    fn journal_push(&mut self, step: Step) {
        if let Some(journal) = &mut self.journal {
            journal.steps.push(step);
        }
    }

    fn gauges(&self) -> Vec<(Gauge, i64)> {
        let turn = &self.turn;
        let mut out = vec![
            (Gauge::Actions, i64::from(turn.actions)),
            (Gauge::Buys, i64::from(turn.buys)),
            (Gauge::Coins, i64::from(turn.coins)),
            (Gauge::Potions, i64::from(turn.potions)),
            (Gauge::TradeRouteMat, i64::from(self.trade_route_mat)),
            (
                Gauge::Sun,
                self.prophecy.map_or(0, |(_, suns)| i64::from(suns)),
            ),
        ];
        for (i, p) in self.players.iter().enumerate() {
            let id = PlayerId(i as u8);
            out.extend([
                (Gauge::Coffers(id), i64::from(p.coffers)),
                (Gauge::Villagers(id), i64::from(p.villagers)),
                (Gauge::Favors(id), i64::from(p.favors)),
                (Gauge::Debt(id), i64::from(p.debt)),
                (Gauge::VpTokens(id), i64::from(p.vp_tokens)),
                (Gauge::PirateShip(id), i64::from(p.pirate_ship)),
            ]);
        }
        for (i, pile) in self.piles.iter().enumerate() {
            let id = PileId(i as u8);
            out.extend([
                (Gauge::PileVp(id), i64::from(pile.vp_tokens)),
                (Gauge::PileDebt(id), i64::from(pile.debt_tokens)),
                (Gauge::Embargo(id), i64::from(pile.embargo)),
                (Gauge::TradeRoute(id), i64::from(pile.trade_route)),
            ]);
        }
        for &(card, vp) in &self.ext.empires.landmark_vp {
            out.push((Gauge::LandscapeVp(card), i64::from(vp)));
        }
        for (i, &n) in self.ext.renaissance.plot.iter().enumerate() {
            out.push((Gauge::SinisterPlot(PlayerId(i as u8)), i64::from(n)));
        }
        for &(pile, n) in &self.ext.allies.inventors {
            out.push((Gauge::PileFavors(pile), i64::from(n)));
        }
        out
    }
}

/// Whether the table keeps `note` among its notes: what it has no other place for. Pile
/// roles, counters on landscapes, Druid's Boons and Traits are elsewhere; lasting effects,
/// turns and the game's end are not notes.
pub fn table_note(note: &Note) -> bool {
    note.setup.is_none()
        && !matches!(
            note.source,
            Card::SinisterPlot
                | Card::FamilyOfInventors
                | Card::TradeRoute
                | Card::Fleet
                | Card::Voyage
                | Card::Contraband
                | Card::WarChest
        )
        && !matches!(note.source.kind(), Kind::Landmark | Kind::Trait)
}

fn holding_changes(was: &Holdings, now: &Holdings, out: &mut Vec<Step>) {
    for (i, props) in now.piles.iter().enumerate() {
        if was.piles.get(i) != Some(props) {
            out.push(Step::Pile {
                pile: PileId(i as u8),
                props: props.clone(),
            });
        }
    }
    for &card in &now.landscapes {
        let prophecy = now.prophecy == Some(card);
        if !was.landscapes.contains(&card) || (prophecy && was.prophecy != Some(card)) {
            out.push(Step::Landscape { card, prophecy });
        }
    }
    let keys: std::collections::BTreeSet<_> = was.tokens.keys().chain(now.tokens.keys()).collect();
    for &(player, token) in keys {
        let at = now.tokens.get(&(player, token)).copied();
        if was.tokens.get(&(player, token)).copied() != at {
            out.push(Step::Token { player, token, at });
        }
    }
    let cards: std::collections::BTreeSet<_> = was.held.keys().chain(now.held.keys()).collect();
    for &card in cards {
        let (before, after) = (was.held.get(&card), now.held.get(&card));
        let (before, after) = (before.map_or(&[][..], |v| v), after.map_or(&[][..], |v| v));
        let gone: Vec<_> = before
            .iter()
            .filter(|p| !after.contains(p))
            .copied()
            .collect();
        let came: Vec<_> = after
            .iter()
            .filter(|p| !before.contains(p))
            .copied()
            .collect();
        // Only an Artifact passes from one player to another.
        if card.kind() == Kind::Artifact {
            for i in 0..gone.len().max(came.len()) {
                let (from, to) = (gone.get(i).copied(), came.get(i).copied());
                out.push(Step::Holding { card, from, to });
            }
        } else {
            for &from in &gone {
                out.push(Step::Holding {
                    card,
                    from: Some(from),
                    to: None,
                });
            }
            for &to in &came {
                out.push(Step::Holding {
                    card,
                    from: None,
                    to: Some(to),
                });
            }
        }
    }
    if was.notes != now.notes {
        out.push(Step::Notes(now.notes.clone()));
    }
    let costs: Vec<_> = (now.costs.iter())
        .filter(|(c, cost)| was.costs.get(c) != Some(cost))
        .map(|(&c, &cost)| (c, cost))
        .collect();
    let names: std::collections::BTreeSet<_> = was.types.keys().chain(now.types.keys()).collect();
    let types: Vec<_> = (names.into_iter())
        .filter(|c| was.types.get(c) != now.types.get(c))
        .map(|&c| (c, now.types.get(&c).copied()))
        .collect();
    if !costs.is_empty() || !types.is_empty() {
        out.push(Step::Derived { costs, types });
    }
}
