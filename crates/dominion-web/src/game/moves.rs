//! Which cards moved where between two boards, for the table to show them moving.
//!
//! Zones hold counts, not cards, so moves are inferred: the new log entries are played
//! over the old board (a play takes the card from the hand to play, a draw from the deck
//! to the hand, Clean-up discards what isn't kept in play, ...), cards the log doesn't
//! name are named by where they end up, and what the log leaves out (Clean-up, cards
//! put into a hand) is filled in from what still differs from the new board. When that
//! doesn't arrive exactly at the new board the inference gives up: no movement is
//! better than wrong movement.

use super::board::{Board, PileAt, PileEntry, PlayerEntry, Zone};
use super::entry::{self, Entry, GainTo, Moved, Place};
use dominion_protocol::LogEntry;
use std::collections::{BTreeMap, BTreeSet};

/// A place cards are in.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Loc {
    Hand(u8),
    Deck(u8),
    Discard(u8),
    InPlay(u8),
    /// A mat or set-aside cards, by the view's key (`tavern`, `setAside`, ...).
    Mat(u8, String),
    Trash,
    Pile(PileAt),
}

impl Loc {
    /// The table element standing for it (`data-anchor`).
    pub fn anchor(&self) -> String {
        match self {
            Loc::Hand(s) => format!("hand:{s}"),
            Loc::Deck(s) => format!("deck:{s}"),
            Loc::Discard(s) => format!("discard:{s}"),
            Loc::InPlay(s) => format!("inplay:{s}"),
            Loc::Mat(s, k) => format!("zone:{s}:{k}"),
            Loc::Trash => "trash".into(),
            Loc::Pile(PileAt::Supply(i)) => format!("pile:s{i}"),
            Loc::Pile(PileAt::NonSupply(i)) => format!("pile:n{i}"),
        }
    }

    pub fn owner(&self) -> Option<u8> {
        match self {
            Loc::Hand(s) | Loc::Deck(s) | Loc::Discard(s) | Loc::InPlay(s) | Loc::Mat(s, _) => {
                Some(*s)
            }
            Loc::Trash | Loc::Pile(_) => None,
        }
    }
}

/// One card going from one place to another. `card` is what the seat may see of it:
/// `None` for a card it can't tell, drawn as a back.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Move {
    pub card: Option<String>,
    pub from: Loc,
    pub to: Loc,
    /// Part of a shuffle, which dominion.games doesn't show moving.
    pub shuffle: bool,
    /// Whose turn it was.
    pub turn: Option<u8>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Inference {
    pub moves: Vec<Move>,
    /// Whether the moves take the old board exactly to the new one. When they don't,
    /// `moves` is empty.
    pub complete: bool,
}

/// What a zone holds as far as the seat knows: cards by name and how many unknown.
/// Piles are all `unknown`: any card may be taken from them by name.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Held {
    known: BTreeMap<String, u32>,
    unknown: u32,
}

impl Held {
    fn count(&self) -> u32 {
        self.known.values().sum::<u32>() + self.unknown
    }

    fn of(&self, c: &str) -> u32 {
        self.known.get(c).copied().unwrap_or(0)
    }

    fn from_zone(z: &Zone) -> Held {
        Held {
            known: z
                .known
                .iter()
                .filter(|s| s.count > 0)
                .map(|s| (s.card.clone(), s.count))
                .collect(),
            unknown: z.unknown,
        }
    }

    fn add(&mut self, c: Option<&str>) {
        match c {
            Some(c) => *self.known.entry(c.to_string()).or_default() += 1,
            None => self.unknown += 1,
        }
    }

    /// Takes a card named `c` (from the unknown ones if none is known as `c`), or an
    /// unknown one. Returns whether it was known by that name.
    fn take(&mut self, c: Option<&str>) -> Option<bool> {
        if let Some(c) = c
            && let Some(n) = self.known.get_mut(c)
            && *n > 0
        {
            *n -= 1;
            if *n == 0 {
                self.known.remove(c);
            }
            return Some(true);
        }
        if self.unknown > 0 {
            self.unknown -= 1;
            return Some(false);
        }
        None
    }

    /// Whether `self` and `other` can be the same cards, known differently: the same
    /// number, and no more of a name in one than the other's known and unknown allow.
    fn agrees(&self, other: &Held) -> bool {
        if self.count() != other.count() {
            return false;
        }
        let over = |a: &Held, b: &Held| -> u32 {
            a.known
                .iter()
                .map(|(c, &n)| n.saturating_sub(b.of(c)))
                .sum()
        };
        over(self, other) <= other.unknown && over(other, self) <= self.unknown
    }
}

type Zones = BTreeMap<Loc, Held>;

/// Where a player's cards taken off their deck to look at (or revealed) are held.
fn looking(p: u8) -> Loc {
    Loc::Mat(p, "looking".into())
}

fn player_zones(p: &PlayerEntry, out: &mut Zones) {
    let s = p.seat;
    out.insert(Loc::Hand(s), Held::from_zone(&p.hand));
    out.insert(Loc::Deck(s), Held::from_zone(&p.deck));
    out.insert(Loc::Discard(s), Held::from_zone(&p.discard));
    out.insert(Loc::InPlay(s), Held::from_zone(&p.in_play));
    for z in &p.zones {
        out.insert(Loc::Mat(s, z.key.clone()), Held::from_zone(&z.zone));
    }
}

fn piles(b: &Board) -> impl Iterator<Item = &PileEntry> {
    b.supply
        .basic
        .iter()
        .chain(b.supply.kingdom.iter().flatten())
        .chain(&b.non_supply)
}

fn zones(b: &Board) -> Zones {
    let mut out = Zones::new();
    for p in b.you.iter().chain(&b.opponents) {
        player_zones(p, &mut out);
    }
    out.insert(Loc::Trash, Held::from_zone(&b.trash));
    for p in piles(b) {
        out.insert(
            Loc::Pile(p.at),
            Held {
                known: BTreeMap::new(),
                unknown: p.count,
            },
        );
    }
    out
}

/// What the log says happened, for the cards it moved.
#[derive(Clone, Debug, PartialEq)]
enum Step {
    Play(u8, String),
    Call(u8, String),
    Gain(u8, String, Loc),
    Trash(u8, String),
    Discard(u8, String),
    /// A card back to its pile, and another from its pile to the discard pile.
    Exchange(u8, String, String),
    TopDeck(u8, Option<String>),
    SetAside(u8, String),
    Draw(u8, u32),
    Shuffle(u8),
    /// Cards taken off the deck to look at.
    Look(u8, u32),
    /// A card revealed: from the hand, or taken off the deck.
    Reveal(u8, String),
    Moved(u8, Moved),
    /// Clean-up: what isn't kept in play and the hand go to the discard pile. When
    /// tentative (the new board is in the middle of Clean-up), only if cards in play
    /// still have to go.
    Cleanup(u8, bool),
    /// A turn begins: whose.
    TurnStart(Option<u8>),
}

fn step(e: &LogEntry, names: &[String]) -> Option<Step> {
    let p = e.player?;
    Some(match entry::parse(e, names)? {
        Entry::Played(c) => Step::Play(p, c),
        Entry::Called(c) => Step::Call(p, c),
        Entry::Gained(c, to) => {
            let to = match to {
                GainTo::Hand => Loc::Hand(p),
                GainTo::Deck => Loc::Deck(p),
                GainTo::SetAside => Loc::Mat(p, "setAside".into()),
                GainTo::Discard => Loc::Discard(p),
            };
            Step::Gain(p, c, to)
        }
        Entry::Trashed(c) => Step::Trash(p, c),
        Entry::Discarded(c) => Step::Discard(p, c),
        Entry::Exchanged { card, into } => Step::Exchange(p, card, into),
        Entry::TopDecked(c) => Step::TopDeck(p, c),
        Entry::SetAside(c) => Step::SetAside(p, c),
        Entry::Drew(n) => Step::Draw(p, n),
        Entry::Shuffled => Step::Shuffle(p),
        Entry::Looked(n) => Step::Look(p, n),
        Entry::Revealed(c) => Step::Reveal(p, c),
        Entry::Moved(m) => Step::Moved(p, m),
        _ => return None,
    })
}

/// The log as steps, with Clean-up put where each turn's ended: before its last draws
/// and shuffles, ahead of the next turn (or of the end of the game, or of the end of the
/// log when the new board is in the middle of Clean-up).
fn steps(
    log: &[LogEntry],
    names: &[String],
    turn_player: Option<u8>,
    end: Option<bool>,
) -> Vec<Step> {
    let mut out: Vec<Step> = vec![];
    let mut cur = turn_player;
    let cleanup = |out: &mut Vec<Step>, p: u8, tentative: bool| {
        let mut at = out.len();
        while at > 0 && matches!(out[at - 1], Step::Draw(q, _) | Step::Shuffle(q) if q == p) {
            at -= 1;
        }
        out.insert(at, Step::Cleanup(p, tentative));
    };
    for e in log {
        if e.kind == "turn-start" {
            if let Some(p) = cur {
                cleanup(&mut out, p, false);
            }
            cur = e.player;
            out.push(Step::TurnStart(cur));
            continue;
        }
        if let Some(s) = step(e, names) {
            out.push(s);
        }
    }
    if let (Some(tentative), Some(p)) = (end, cur) {
        cleanup(&mut out, p, tentative);
    }
    out
}

struct Sim<'a> {
    zones: Zones,
    goal: &'a Zones,
    /// What the steps still to come will add to (or take from) each zone's count.
    pending: BTreeMap<Loc, i64>,
    /// Cards drawn and discarded again within the log can't be named from the new
    /// board: then only counts must agree, and only cards the log names are shown.
    loose: bool,
    /// The step being run names its card.
    naming: bool,
    /// How many times the steps still to come move each player's card of a name out of
    /// their play area.
    leaving_play: BTreeMap<(u8, String), u32>,
    /// What the Clean-up being run keeps in play.
    keep: Option<Held>,
    /// Players whose Clean-up is among the steps still to come.
    cleanup_ahead: BTreeSet<u8>,
    /// The step being run is a shuffle.
    shuffling: bool,
    /// Whose turn the step being run is in.
    turn: Option<u8>,
    old: &'a Board,
    new: &'a Board,
    moves: Vec<Done>,
}

/// A move made, with what was taken from the source (known by name or not) so that it
/// can be undone.
#[derive(Clone, Debug)]
struct Done {
    face: Option<String>,
    from: Loc,
    to: Loc,
    taken: Option<String>,
    put: Option<String>,
    shuffle: bool,
    turn: Option<u8>,
}

impl Sim<'_> {
    fn held(&self, l: &Loc) -> Held {
        self.zones.get(l).cloned().unwrap_or_default()
    }

    fn goal(&self, l: &Loc) -> Held {
        self.goal.get(l).cloned().unwrap_or_default()
    }

    /// The zone holds more than it will, after the steps still to come.
    fn surplus(&self, l: &Loc) -> bool {
        let ahead = i64::from(self.held(l).count()) + self.pending.get(l).copied().unwrap_or(0);
        ahead > i64::from(self.goal(l).count())
    }

    /// Counts what the steps after the current one will move for certain: draws, plays,
    /// gains, Clean-up and shuffles.
    fn look_ahead(&mut self, rest: &[Step]) {
        let mut pending: BTreeMap<Loc, i64> = BTreeMap::new();
        let mut leaving: BTreeMap<(u8, String), u32> = BTreeMap::new();
        self.cleanup_ahead.clear();
        let mut add = |l: Loc, n: i64| *pending.entry(l).or_default() += n;
        for (i, s) in rest.iter().enumerate() {
            match s {
                Step::Moved(p, m) => {
                    let n = i64::from(m.count());
                    let first = m.cards.first().map(String::as_str);
                    let from = match (&m.from, m.back) {
                        (Some(f), _) => self.source_loc(f, first),
                        (None, true) => Some(looking(*p)),
                        (None, false) => None,
                    };
                    if let Some(f) = from {
                        if let Loc::InPlay(q) = f {
                            for c in &m.cards {
                                *leaving.entry((q, c.clone())).or_default() += 1;
                            }
                        }
                        add(f, -n);
                    }
                    if let Some(to) = self.dest_loc(&m.to, first) {
                        add(to, n);
                    }
                }
                Step::Look(p, n) => {
                    add(Loc::Deck(*p), -i64::from(*n));
                    add(looking(*p), i64::from(*n));
                }
                Step::Call(p, _) => {
                    add(Loc::Mat(*p, "tavern".into()), -1);
                    add(Loc::InPlay(*p), 1);
                }
                Step::Draw(p, n) => {
                    add(Loc::Deck(*p), -i64::from(*n));
                    add(Loc::Hand(*p), i64::from(*n));
                }
                Step::Play(p, _) => {
                    add(Loc::Hand(*p), -1);
                    add(Loc::InPlay(*p), 1);
                }
                Step::Gain(_, _, to) => add(to.clone(), 1),
                Step::Shuffle(p) => {
                    let n = i64::from(self.held(&Loc::Discard(*p)).count());
                    add(Loc::Discard(*p), -n);
                    add(Loc::Deck(*p), n);
                }
                Step::Cleanup(p, _) => {
                    self.cleanup_ahead.insert(*p);
                    let hand = i64::from(self.held(&Loc::Hand(*p)).count());
                    let play = i64::from(self.held(&Loc::InPlay(*p)).count())
                        - i64::from(self.kept(*p, &rest[i + 1..]).count());
                    add(Loc::Hand(*p), -hand);
                    add(Loc::InPlay(*p), -play.max(0));
                    add(Loc::Discard(*p), hand + play.max(0));
                }
                _ => {}
            }
        }
        self.pending = pending;
        self.leaving_play = leaving;
    }

    /// What a player's Clean-up keeps in play: what the new board has there, less what
    /// the steps after it play.
    fn kept(&self, p: u8, after: &[Step]) -> Held {
        let mut keep = self.goal(&Loc::InPlay(p));
        for s in after {
            if let Step::Play(q, c) | Step::Call(q, c) = s
                && *q == p
            {
                keep.take(Some(c));
            }
        }
        keep
    }

    fn has(&self, l: &Loc, c: Option<&str>) -> bool {
        let h = self.held(l);
        h.unknown > 0 || c.is_some_and(|c| h.of(c) > 0) || (c.is_none() && h.count() > 0)
    }

    /// Whether the seat sees the cards of `l` (a pile's or the trash's, or its own).
    fn public(&self, l: &Loc) -> bool {
        match l {
            Loc::Trash | Loc::Pile(_) | Loc::InPlay(_) | Loc::Discard(_) => true,
            Loc::Hand(s) | Loc::Deck(s) | Loc::Mat(s, _) => {
                self.new.you.as_ref().is_some_and(|y| y.seat == *s)
            }
        }
    }

    /// Moves a card. `c` is its name when the log or the boards tell it.
    fn apply(&mut self, c: Option<&str>, from: &Loc, to: &Loc) -> bool {
        let Some(src) = self.zones.get_mut(from) else {
            return false;
        };
        let taken = match src.take(c) {
            Some(true) => c.map(str::to_string),
            Some(false) => None,
            None => return false,
        };
        let goal = self.goal(to);
        let dest = self.zones.entry(to.clone()).or_default();
        // A card arrives known when the seat knows it where it ends up.
        let put = c.filter(|c| goal.of(c) > dest.of(c) || goal.unknown == 0);
        dest.add(put);
        let face = c
            .filter(|_| !self.loose || self.naming)
            .filter(|_| taken.is_some() || put.is_some() || self.public(to) || self.public(from))
            .map(str::to_string);
        self.moves.push(Done {
            face,
            from: from.clone(),
            to: to.clone(),
            taken,
            put: put.map(str::to_string),
            shuffle: self.shuffling,
            turn: self.turn,
        });
        true
    }

    /// Moves an unnamed card (a draw, a card put onto a deck), naming it after what the
    /// destination will hold that the source does.
    fn apply_unnamed(&mut self, from: &Loc, to: &Loc) -> bool {
        let src = self.held(from);
        let (dest, goal) = (self.held(to), self.goal(to));
        // A hand a Clean-up will empty before the new board isn't the new board's.
        let passing = matches!(to, Loc::Hand(p) if self.cleanup_ahead.contains(p));
        let needed = goal
            .known
            .iter()
            .filter(|&(c, &n)| n > dest.of(c) && !passing)
            .map(|(c, _)| c.clone());
        let mut needed_here = None;
        let mut needed_any = None;
        for c in needed {
            if src.of(&c) > 0 && needed_here.is_none() {
                needed_here = Some(c.clone());
            }
            if needed_any.is_none() {
                needed_any = Some(c);
            }
        }
        if let Some(c) = needed_here.or(needed_any.filter(|_| src.unknown > 0)) {
            return self.apply(Some(&c), from, to);
        }
        if src.unknown > 0 {
            return self.apply(None, from, to);
        }
        // A card of a known mix leaving it unseen: take one the source must lose, and
        // let it arrive unknown.
        let goal_src = self.goal(from);
        let leaving = src
            .known
            .iter()
            .find(|&(c, &n)| n > goal_src.of(c))
            .or_else(|| src.known.iter().next())
            .map(|(c, _)| c.clone());
        let Some(c) = leaving else { return false };
        let Some(z) = self.zones.get_mut(from) else {
            return false;
        };
        z.take(Some(&c));
        self.zones.entry(to.clone()).or_default().add(None);
        self.moves.push(Done {
            face: None,
            from: from.clone(),
            to: to.clone(),
            taken: Some(c),
            put: None,
            shuffle: self.shuffling,
            turn: self.turn,
        });
        true
    }

    fn knows(&self, l: &Loc, c: Option<&str>) -> bool {
        c.is_some_and(|c| self.held(l).of(c) > 0)
    }

    /// Where a card comes from, of `from` in order of likelihood: a place that must
    /// lose cards and is known to hold it, then one that must lose cards and may hold
    /// it, then the first known to hold it, then the first that may.
    fn source(&self, c: Option<&str>, from: &[Loc]) -> Option<Loc> {
        from.iter()
            .find(|l| self.surplus(l) && self.knows(l, c))
            .or_else(|| from.iter().find(|l| self.surplus(l) && self.has(l, c)))
            .or_else(|| from.iter().find(|l| self.knows(l, c)))
            .or_else(|| from.iter().find(|l| self.has(l, c)))
            .cloned()
    }

    /// A player's places, cards they are looking at first: what an effect does next is
    /// most likely done to them.
    fn player_locs(&self, p: u8) -> Vec<Loc> {
        let look = looking(p);
        let mut out = vec![];
        if self.held(&look).count() > 0 {
            out.push(look.clone());
        }
        out.extend([Loc::Hand(p), Loc::InPlay(p), Loc::Deck(p), Loc::Discard(p)]);
        out.extend(
            self.zones
                .keys()
                .filter(|l| matches!(l, Loc::Mat(s, _) if *s == p) && **l != look)
                .cloned(),
        );
        out
    }

    /// Piles a card named `c` comes from or goes back to: by name or top card.
    fn piles_of(&self, c: &str) -> Vec<Loc> {
        let mut out: Vec<Loc> = vec![];
        for b in [self.old, self.new] {
            for p in piles(b) {
                let l = Loc::Pile(p.at);
                if (p.name == c || p.card == c) && !out.contains(&l) {
                    out.push(l);
                }
            }
        }
        out
    }

    fn loc(&self, place: &Place, pile: impl Fn(&[Loc]) -> Option<Loc>) -> Option<Loc> {
        Some(match place {
            Place::Hand(p) => Loc::Hand(*p),
            Place::Deck(p) => Loc::Deck(*p),
            Place::Discard(p) => Loc::Discard(*p),
            Place::InPlay(p) => Loc::InPlay(*p),
            Place::Zone(p, k) => Loc::Mat(*p, (*k).into()),
            Place::Trash => Loc::Trash,
            Place::Pile(name) => return pile(&self.piles_of(name)),
        })
    }

    /// Where cards named `card` (if the log names them) come from: a pile by that name
    /// that must lose cards, else one that has some.
    fn source_loc(&self, place: &Place, card: Option<&str>) -> Option<Loc> {
        self.loc(place, |piles| {
            let mut piles = piles.to_vec();
            if let Some(c) = card {
                piles.extend(self.piles_of(c));
            }
            piles
                .iter()
                .find(|l| self.surplus(l))
                .or_else(|| piles.iter().find(|l| self.has(l, None)))
                .cloned()
        })
    }

    /// Where cards go to: a pile by that name that must gain cards, else the first.
    fn dest_loc(&self, place: &Place, card: Option<&str>) -> Option<Loc> {
        self.loc(place, |piles| {
            let mut piles = piles.to_vec();
            if let Some(c) = card {
                piles.extend(self.piles_of(c));
            }
            let wanting = |l: &&Loc| self.held(l).count() < self.goal(l).count();
            piles.iter().find(wanting).or(piles.first()).cloned()
        })
    }

    /// Moves one card the log moved from `from` (if it says, and the card is there) or
    /// else from where it most likely was.
    fn move_one(&mut self, p: u8, c: Option<&str>, from: Option<&Loc>, to: &Loc) {
        let f = match from.filter(|f| self.has(f, c)) {
            Some(f) => f.clone(),
            None => {
                let mut from: Vec<Loc> = self
                    .player_locs(p)
                    .into_iter()
                    .filter(|l| l != to)
                    .collect();
                if let Some(c) = c {
                    from.push(Loc::Trash);
                    from.extend(self.piles_of(c).into_iter().filter(|l| l != to));
                }
                match self.source(c, &from) {
                    Some(f) => f,
                    None => return,
                }
            }
        };
        match c {
            Some(c) => self.apply(Some(c), &f, to),
            None => self.apply_unnamed(&f, to),
        };
    }

    fn run(&mut self, s: &Step) {
        match s {
            Step::Play(p, c) => {
                let to = Loc::InPlay(*p);
                // A card played again (Throne Room) is already in play. Before a Clean-up
                // the new board can't tell, so then it is one the hand is known not to
                // hold.
                let replay = if self.cleanup_ahead.contains(p) {
                    let hand = self.held(&Loc::Hand(*p));
                    self.held(&to).of(c) > 0 && hand.of(c) == 0 && hand.unknown == 0
                } else {
                    let leaving = self.leaving_play.get(&(*p, c.clone())).copied();
                    self.held(&to).of(c) >= self.goal(&to).of(c) + leaving.unwrap_or(0)
                };
                if replay {
                    return;
                }
                let hand = Loc::Hand(*p);
                // What a Clean-up still to come will do leaves no place short or over, so
                // then the hand is where a card is played from.
                let f = if self.cleanup_ahead.contains(p) && self.has(&hand, Some(c)) {
                    Some(hand)
                } else {
                    let mut from = vec![hand];
                    from.extend(self.player_locs(*p).into_iter().filter(|l| *l != to));
                    self.source(Some(c), &from)
                };
                if let Some(f) = f {
                    self.apply(Some(c), &f, &to);
                }
            }
            Step::Gain(_, c, to) => {
                let trash = Loc::Trash;
                let from = if self.held(&trash).of(c) > self.goal(&trash).of(c) {
                    Some(trash)
                } else {
                    let piles = self.piles_of(c);
                    piles
                        .iter()
                        .find(|l| self.surplus(l))
                        .or_else(|| piles.iter().find(|l| self.has(l, None)))
                        .cloned()
                        .or_else(|| {
                            self.zones
                                .keys()
                                .find(|l| matches!(l, Loc::Pile(_)) && self.surplus(l))
                                .cloned()
                        })
                };
                if let Some(f) = from {
                    self.apply(Some(c), &f, to);
                }
            }
            Step::Trash(p, c) => {
                let mut from = self.player_locs(*p);
                from.extend(self.piles_of(c));
                let others: Vec<Loc> = self
                    .zones
                    .keys()
                    .filter(|l| l.owner().is_some_and(|o| o != *p))
                    .cloned()
                    .collect();
                from.extend(others);
                if let Some(f) = self.source(Some(c), &from) {
                    self.apply(Some(c), &f, &Loc::Trash);
                }
            }
            Step::Discard(p, c) | Step::SetAside(p, c) => {
                let to = match s {
                    Step::Discard(..) => Loc::Discard(*p),
                    _ => Loc::Mat(*p, "setAside".into()),
                };
                let from: Vec<Loc> = self
                    .player_locs(*p)
                    .into_iter()
                    .filter(|l| *l != to)
                    .collect();
                if let Some(f) = self.source(Some(c), &from) {
                    self.apply(Some(c), &f, &to);
                }
            }
            Step::TopDeck(p, c) => {
                let to = Loc::Deck(*p);
                let from: Vec<Loc> = self
                    .player_locs(*p)
                    .into_iter()
                    .filter(|l| *l != to)
                    .collect();
                if let Some(f) = self.source(c.as_deref(), &from) {
                    match c {
                        Some(c) => self.apply(Some(c), &f, &to),
                        None => self.apply_unnamed(&f, &to),
                    };
                }
            }
            Step::Exchange(p, card, into) => {
                // From wherever it was: a place holding more of it than it will.
                let from = self
                    .player_locs(*p)
                    .into_iter()
                    .find(|l| self.held(l).of(card) > self.goal(l).of(card));
                if let Some(pile) = self.dest_loc(&Place::Pile(card.clone()), Some(card)) {
                    self.move_one(*p, Some(card), from.as_ref(), &pile);
                }
                let into_pile = self.source_loc(&Place::Pile(into.clone()), Some(into));
                self.move_one(*p, Some(into), into_pile.as_ref(), &Loc::Discard(*p));
            }
            Step::Call(p, c) => {
                let tavern = Loc::Mat(*p, "tavern".into());
                self.move_one(*p, Some(c), Some(&tavern), &Loc::InPlay(*p));
            }
            Step::Look(p, n) => {
                for _ in 0..*n {
                    if !self.apply_unnamed(&Loc::Deck(*p), &looking(*p)) {
                        break;
                    }
                }
            }
            Step::Reveal(p, c) => {
                let look = looking(*p);
                let ahead = i64::from(self.held(&look).count())
                    + self.pending.get(&look).copied().unwrap_or(0);
                if ahead < i64::from(self.goal(&look).count()) {
                    self.apply(Some(c), &Loc::Deck(*p), &look);
                }
            }
            Step::Draw(p, n) => {
                for _ in 0..*n {
                    if !self.apply_unnamed(&Loc::Deck(*p), &Loc::Hand(*p)) {
                        break;
                    }
                }
            }
            Step::Shuffle(p) => {
                let from = Loc::Discard(*p);
                let to = Loc::Deck(*p);
                let h = self.held(&from);
                for (c, n) in h.known {
                    for _ in 0..n {
                        self.apply(Some(&c), &from, &to);
                    }
                }
                for _ in 0..h.unknown {
                    self.apply(None, &from, &to);
                }
            }
            Step::Moved(p, m) => {
                let first = m.cards.first().map(String::as_str);
                let Some(to) = self.dest_loc(&m.to, first) else {
                    return;
                };
                let from = match (&m.from, m.back) {
                    (Some(f), _) => self.source_loc(f, first),
                    // Off the deck to look at, or the deck onto itself.
                    (None, true) if self.held(&looking(*p)).count() > 0 => Some(looking(*p)),
                    (None, true) => return,
                    (None, false) => None,
                };
                for c in &m.cards {
                    self.move_one(*p, Some(c), from.as_ref(), &to);
                }
                for _ in 0..m.hidden {
                    self.move_one(*p, None, from.as_ref(), &to);
                }
            }
            Step::TurnStart(_) => {}
            Step::Cleanup(p, tentative) => {
                let play = Loc::InPlay(*p);
                let discard = Loc::Discard(*p);
                let h = self.held(&play);
                let keep = self.keep.clone().unwrap_or_default();
                if *tentative && h.count() <= keep.count() {
                    return;
                }
                for (c, n) in h.known {
                    for _ in keep.of(&c).min(n)..n {
                        self.apply(Some(&c), &play, &discard);
                    }
                }
                for _ in keep.unknown.min(h.unknown)..h.unknown {
                    self.apply(None, &play, &discard);
                }
                let hand = Loc::Hand(*p);
                let h = self.held(&hand);
                for (c, n) in h.known {
                    for _ in 0..n {
                        self.apply(Some(&c), &hand, &discard);
                    }
                }
                for _ in 0..h.unknown {
                    self.apply(None, &hand, &discard);
                }
            }
        }
    }

    /// Takes back moves that left their source short and their destination over.
    fn undo_overshoot(&mut self) {
        let mut i = self.moves.len();
        while i > 0 {
            i -= 1;
            let m = self.moves[i].clone();
            let over = self.held(&m.to).count() > self.goal(&m.to).count();
            let short = self.held(&m.from).count() < self.goal(&m.from).count();
            if !(over && short) {
                continue;
            }
            let Some(dest) = self.zones.get_mut(&m.to) else {
                continue;
            };
            if dest.take(m.put.as_deref()) != Some(m.put.is_some()) {
                continue;
            }
            self.zones
                .entry(m.from.clone())
                .or_default()
                .add(m.taken.as_deref());
            self.moves.remove(i);
        }
    }

    /// Moves the log leaves out, between one player's zones, where one holds too many
    /// and another too few.
    fn fill_gaps(&mut self) {
        let seats: Vec<u8> = self
            .zones
            .keys()
            .filter_map(Loc::owner)
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect();
        for p in seats {
            let mats: Vec<Loc> = self
                .zones
                .keys()
                .filter(|l| matches!(l, Loc::Mat(s, _) if *s == p))
                .cloned()
                .collect();
            let (hand, deck, discard, play) =
                (Loc::Hand(p), Loc::Deck(p), Loc::Discard(p), Loc::InPlay(p));
            let mut pairs = vec![
                (play.clone(), discard.clone()),
                (hand.clone(), discard.clone()),
            ];
            pairs.extend(mats.iter().map(|m| (m.clone(), hand.clone())));
            pairs.extend([
                (deck.clone(), hand.clone()),
                (hand.clone(), deck.clone()),
                (discard.clone(), deck.clone()),
                (play.clone(), hand.clone()),
                (discard.clone(), hand.clone()),
            ]);
            pairs.extend(mats.iter().map(|m| (m.clone(), discard.clone())));
            pairs.extend(mats.iter().map(|m| (play.clone(), m.clone())));
            for (a, b) in pairs {
                while self.surplus(&a) && self.held(&b).count() < self.goal(&b).count() {
                    if !self.fill_one(&a, &b) {
                        break;
                    }
                }
            }
        }
    }

    fn fill_one(&mut self, a: &Loc, b: &Loc) -> bool {
        let (ha, ga, hb, gb) = (self.held(a), self.goal(a), self.held(b), self.goal(b));
        let extra = |h: &Held, g: &Held| -> Vec<String> {
            h.known
                .iter()
                .filter(|&(c, &n)| n > g.of(c))
                .map(|(c, _)| c.clone())
                .collect()
        };
        let a_extra = extra(&ha, &ga);
        let b_wants = extra(&gb, &hb);
        if let Some(c) = a_extra.iter().find(|c| b_wants.contains(c)) {
            return self.apply(Some(c), a, b);
        }
        if let Some(c) = a_extra.first().filter(|_| gb.unknown > hb.unknown) {
            return self.apply(Some(c), a, b);
        }
        if let Some(c) = b_wants.first().filter(|_| ha.unknown > ga.unknown) {
            return self.apply(Some(c), a, b);
        }
        if ha.unknown > ga.unknown && gb.unknown > hb.unknown {
            return self.apply(None, a, b);
        }
        false
    }

    fn agrees(&self) -> bool {
        let keys: std::collections::BTreeSet<&Loc> =
            self.zones.keys().chain(self.goal.keys()).collect();
        keys.into_iter().all(|l| match l {
            Loc::Pile(_) => self.held(l).count() == self.goal(l).count(),
            _ if self.loose => self.held(l).count() == self.goal(l).count(),
            _ => self.held(l).agrees(&self.goal(l)),
        })
    }
}

/// The moves taking `old` to `new`, given the log entries between them.
pub fn infer(old: &Board, new: &Board, log: &[LogEntry]) -> Inference {
    let start = zones(old);
    let goal = zones(new);
    let mut names: Vec<String> = vec![];
    for p in old.you.iter().chain(&old.opponents) {
        let i = usize::from(p.seat);
        if names.len() <= i {
            names.resize_with(i + 1, String::new);
        }
        names[i] = p.name.clone();
    }
    for (i, n) in names.iter_mut().enumerate() {
        if n.is_empty() {
            *n = entry::seat_name(&[], i as u8);
        }
    }
    let first_turn = old.status.player.as_ref().map(|w| w.seat);
    let mut sim = Sim {
        zones: start,
        goal: &goal,
        old,
        new,
        moves: vec![],
        pending: BTreeMap::new(),
        leaving_play: BTreeMap::new(),
        cleanup_ahead: BTreeSet::new(),
        keep: None,
        shuffling: false,
        turn: first_turn,
        loose: false,
        naming: false,
    };
    let end = if old.result.is_none() && new.result.is_some() {
        Some(false)
    } else if new.status.phase.as_deref() == Some("cleanup") {
        Some(true)
    } else {
        None
    };
    let steps = steps(log, &names, first_turn, end);
    sim.loose = steps.iter().enumerate().any(|(i, s)| {
        matches!(s, Step::Draw(p, _)
            if steps[i + 1..].iter().any(|t| matches!(t, Step::Cleanup(q, _) if q == p)))
    });
    for (i, s) in steps.iter().enumerate() {
        sim.look_ahead(&steps[i + 1..]);
        sim.shuffling = matches!(s, Step::Shuffle(_));
        sim.keep = match s {
            Step::Cleanup(p, _) => Some(sim.kept(*p, &steps[i + 1..])),
            _ => None,
        };
        if let Step::TurnStart(p) = s {
            sim.turn = *p;
        }
        sim.naming = matches!(
            s,
            Step::Play(..)
                | Step::Gain(..)
                | Step::Trash(..)
                | Step::Discard(..)
                | Step::SetAside(..)
                | Step::Call(..)
                | Step::Exchange(..)
                | Step::Reveal(..)
                | Step::TopDeck(_, Some(_))
        ) || matches!(s, Step::Moved(_, m) if !m.cards.is_empty());
        sim.run(s);
    }
    sim.naming = false;
    sim.shuffling = false;
    sim.turn = new.status.player.as_ref().map(|w| w.seat);
    sim.pending.clear();
    sim.cleanup_ahead.clear();
    sim.undo_overshoot();
    sim.fill_gaps();
    if !sim.agrees() {
        return Inference::default();
    }
    Inference {
        moves: sim
            .moves
            .into_iter()
            .map(|d| Move {
                card: d.face,
                from: d.from,
                to: d.to,
                shuffle: d.shuffle,
                turn: d.turn,
            })
            .collect(),
        complete: true,
    }
}

/// One card (or a run of cards between the same two places) to draw moving.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ghost {
    pub card: Option<String>,
    pub from: Loc,
    pub to: Loc,
    /// How many cards it stands for.
    pub count: u32,
    pub turn: Option<u8>,
}

/// The ghosts to draw for `moves`, leaving out shuffles (dominion.games shows none): one
/// per move while there are at most `max`, else one per pair of places (in the order
/// first seen), at most `max` of them, so that a long run of moves is summarised rather
/// than replayed.
pub fn ghosts(moves: &[Move], max: usize) -> Vec<Ghost> {
    let moves: Vec<&Move> = moves.iter().filter(|m| !m.shuffle).collect();
    let ghost = |m: &Move| Ghost {
        card: m.card.clone(),
        from: m.from.clone(),
        to: m.to.clone(),
        count: 1,
        turn: m.turn,
    };
    if moves.len() <= max {
        return moves.into_iter().map(ghost).collect();
    }
    let mut out: Vec<Ghost> = vec![];
    for m in moves {
        match out
            .iter_mut()
            .find(|g| g.from == m.from && g.to == m.to && g.turn == m.turn)
        {
            Some(g) => {
                g.count += 1;
                if g.card != m.card {
                    g.card = None;
                }
            }
            None => out.push(ghost(m)),
        }
    }
    out.truncate(max);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mv(card: Option<&str>, from: Loc, to: Loc) -> Move {
        Move {
            card: card.map(String::from),
            from,
            to,
            shuffle: false,
            turn: Some(0),
        }
    }

    fn ghost(card: Option<&str>, from: Loc, to: Loc, count: u32) -> Ghost {
        Ghost {
            card: card.map(String::from),
            from,
            to,
            count,
            turn: Some(0),
        }
    }

    #[test]
    fn a_long_run_of_moves_is_summarised_by_where_they_go() {
        let mut moves = vec![mv(Some("Copper"), Loc::Hand(0), Loc::InPlay(0)); 3];
        moves.extend(vec![mv(None, Loc::Deck(0), Loc::Hand(0)); 12]);
        moves.push(mv(Some("Gold"), Loc::Hand(0), Loc::InPlay(0)));
        assert_eq!(ghosts(&moves[..3], 12).len(), 3);
        assert_eq!(
            ghosts(&moves, 12),
            [
                ghost(None, Loc::Hand(0), Loc::InPlay(0), 4),
                ghost(None, Loc::Deck(0), Loc::Hand(0), 12),
            ]
        );
    }

    #[test]
    fn shuffles_have_no_ghosts() {
        let mut moves = vec![
            Move {
                shuffle: true,
                ..mv(None, Loc::Discard(0), Loc::Deck(0))
            };
            20
        ];
        moves.push(mv(Some("Gold"), Loc::Deck(0), Loc::Hand(0)));
        assert_eq!(
            ghosts(&moves, 12),
            [ghost(Some("Gold"), Loc::Deck(0), Loc::Hand(0), 1)]
        );
    }
}
