//! What each player can know about every physical card, with perfect recall.
//!
//! For each viewer, every card belongs to a group of cards the viewer can't tell apart;
//! the viewer knows each group's cards (where each one is: card movements are public)
//! and, unless it is unknown, the multiset of their identities. A card the viewer sees
//! becomes a group of its own. Deck order is never tracked.
//!
//! Groups merge when the viewer loses track of which card is which: in a shuffle, when
//! cards are put back in an order someone else chose, and when a card leaves a set of
//! cards its owner holds out of the viewer's sight. Such a set is a hand, a Native Village
//! mat, the cards taken off a deck to be looked at, cards set aside face down together, or
//! the cards of a deck or discard pile their owner last looked through. A card leaving it unseen could be any of the set, unless
//! the whole set goes the same way; a card seen leaving it (or revealed from it) tells the
//! viewer its name and no more, so it could be any card of the set that may have that
//! name. Everything else can be followed: cards are taken from known places in a deck,
//! and a card everyone saw reach a discard pile is still that card when an effect moves
//! it on. Stash and the Shadow cards have their own backs, so everyone always knows which
//! card each of them is.
//!
//! When a card leaves such a set unseen, only the set's cards become one group: a group
//! some of whose cards were in the set splits, each part being some of what the group
//! was. The parts of a group are its family, which together are what it was, and become
//! one again when all of them are mixed.
//!
//! [`Knowledge`] consumes a traced game's [`Trace`]; build it before the game's first
//! [`Game::take_trace`] and call [`Knowledge::update`] after every response.

use crate::game::{Mat, Pile, Zone};
use crate::{Card, CardId, Game, PlayerId, Trace, Types};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};

/// Cards of some identities, by identity.
pub type CardCount = BTreeMap<Card, u32>;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Knowledge {
    /// Each card's zone and printed identity, as of the trace consumed so far.
    zone: Vec<Zone>,
    is: Vec<Card>,
    #[serde(with = "zone_map")]
    members: HashMap<Zone, Vec<CardId>>,
    minds: Vec<Mind>,
    /// Where the last `Created` card went, and that card, to group cards created together.
    created_in: Option<(Container, CardId)>,
    /// Per card, the set it is held in out of the other players' sight (0: none): the
    /// cards of a deck or discard pile their owner looked at together, or cards set aside
    /// face down together. It lasts while the card stays there unrevealed and is not
    /// shuffled with other cards.
    #[serde(default)]
    held: Vec<u32>,
    #[serde(default)]
    sets: u32,
    /// The set the last entry put a card in, and where: a card the next entry looks at
    /// or sets aside there joins it.
    #[serde(default)]
    joining: Option<(Zone, u32)>,
    /// The card the last entry showed to everyone.
    #[serde(default)]
    shown: Option<CardId>,
    #[serde(default)]
    face_up: HashSet<CardId>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
enum Container {
    Player(PlayerId),
    Zone(Zone),
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct Mind {
    group_of: Vec<usize>,
    groups: Vec<Group>,
    #[serde(default)]
    families: Vec<Family>,
    /// Cards shown to this viewer since the last decision that haven't moved since: they
    /// can follow them.
    recent: HashSet<CardId>,
    #[serde(default)]
    leaving: Option<Leaving>,
    /// Hands discarded whole unseen: their identities arrived together, their positions
    /// within the discard pile are unknown.
    #[serde(default)]
    arrived: Vec<Vec<CardId>>,
}

/// Cards a viewer watched leave a held set for one place without seeing which they were.
/// Until the viewer knows where the rest of the set goes, the groups stay as they were.
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Leaving {
    from: Zone,
    to: Zone,
    left: Vec<CardId>,
    /// The cards still in the set.
    rest: Vec<CardId>,
    /// Cards of the set the viewer has seen since.
    seen: Vec<CardId>,
}

/// Zones aren't valid JSON object keys, so the map is stored as pairs.
mod zone_map {
    use super::{CardId, HashMap, Zone};
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(
        map: &HashMap<Zone, Vec<CardId>>,
        s: S,
    ) -> Result<S::Ok, S::Error> {
        s.collect_seq(map.iter())
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        d: D,
    ) -> Result<HashMap<Zone, Vec<CardId>>, D::Error> {
        Ok(Vec::<(Zone, Vec<CardId>)>::deserialize(d)?
            .into_iter()
            .collect())
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct Group {
    cards: Vec<CardId>,
    /// `None` when the viewer doesn't know which cards these are.
    ids: Option<CardCount>,
    /// How many of `ids` are not these cards: the cards are some of `ids`, and the others
    /// are in the other groups of its family.
    #[serde(default)]
    extra: u32,
    #[serde(default)]
    family: Option<usize>,
}

/// Groups split from one the viewer knew the cards of, when only some of its cards
/// became indistinguishable from others (some left a set held out of sight): together
/// they are exactly `ids`, each of them some of its own `ids`.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct Family {
    ids: Option<CardCount>,
    groups: Vec<usize>,
}

/// One group of indistinguishable cards, as one viewer knows it.
#[derive(Clone, Copy, Debug)]
pub struct GroupView<'a> {
    pub cards: &'a [CardId],
    /// Their identities as a multiset, if the viewer knows it: with `extra` more than
    /// there are cards, which of them the cards are.
    pub ids: Option<&'a CardCount>,
    pub extra: u32,
}

/// What a viewer knows about the cards in one zone.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ZoneKnowledge {
    pub count: usize,
    pub known: CardCount,
    pub unknown: usize,
}

/// Cards of a group spread over several zones: together they are `candidates`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UncertainCards {
    /// Each zone and how many of the cards are there.
    pub zones: Vec<(Zone, usize)>,
    pub candidates: CardCount,
    /// How many of the candidates are not these cards but are elsewhere.
    pub outside: usize,
}

/// A viewer's knowledge of every zone, as zone summaries.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct KnownCards {
    pub zones: HashMap<Zone, ZoneKnowledge>,
    pub uncertain: Vec<UncertainCards>,
    /// Per player: the identities of cards they own that the viewer knows, wherever they
    /// are among that player's zones, and how many owned cards are unknown.
    pub owned: Vec<(CardCount, usize)>,
}

impl KnownCards {
    pub fn zone(&self, zone: Zone) -> ZoneKnowledge {
        self.zones.get(&zone).cloned().unwrap_or_default()
    }
}

fn add(count: &mut CardCount, card: Card, n: u32) {
    *count.entry(card).or_default() += n;
}

fn add_all(to: &mut Option<CardCount>, from: Option<&CardCount>) {
    match (to.as_mut(), from) {
        (Some(a), Some(b)) => b.iter().for_each(|(&c, &n)| add(a, c, n)),
        _ => *to = None,
    }
}

fn remove(count: &mut CardCount, card: Card) -> bool {
    match count.get_mut(&card) {
        Some(n) if *n > 1 => *n -= 1,
        Some(_) => {
            count.remove(&card);
        }
        None => return false,
    }
    true
}

impl Mind {
    fn ensure(&mut self, card: CardId) {
        if self.group_of.len() <= card.index() {
            self.group_of.resize(card.index() + 1, usize::MAX);
        }
    }

    fn new_group(&mut self, cards: Vec<CardId>, ids: Option<CardCount>) {
        let g = self.groups.len();
        for &c in &cards {
            self.ensure(c);
            self.group_of[c.index()] = g;
        }
        self.groups.push(Group {
            cards,
            ids,
            ..Group::default()
        });
    }

    fn add_to(&mut self, g: usize, card: CardId, is: Card) {
        self.ensure(card);
        self.group_of[card.index()] = g;
        let group = &mut self.groups[g];
        group.cards.push(card);
        if let Some(ids) = &mut group.ids {
            add(ids, is, 1);
        }
        if let Some(f) = group.family
            && let Some(ids) = &mut self.families[f].ids
        {
            add(ids, is, 1);
        }
    }

    /// The viewer sees `card`, which is `is`.
    fn identify(&mut self, card: CardId, is: Card) {
        if let Some(i) = self.arrived.iter().position(|set| set.contains(&card)) {
            let among = self.arrived[i].clone();
            self.arrived[i].retain(|&c| c != card);
            if self.arrived[i].len() < 2 {
                self.arrived.swap_remove(i);
            }
            self.identify_among(card, is, &among);
            return;
        }
        self.identify_exact(card, is);
    }

    fn identify_exact(&mut self, card: CardId, is: Card) {
        let g = self.group_of[card.index()];
        let family = self.groups[g].family;
        if let Some(f) = family
            && let Some(ids) = &mut self.families[f].ids
            && !remove(ids, is)
        {
            debug_assert!(
                false,
                "{card:?} ({is}) missing from its family's identities"
            );
            self.families[f].ids = None;
        }
        let group = &mut self.groups[g];
        if group.cards.len() == 1 {
            group.ids = Some(CardCount::from([(is, 1)]));
            group.extra = 0;
            if let Some(f) = group.family.take() {
                self.families[f].groups.retain(|&x| x != g);
                self.tidy(f);
            }
            return;
        }
        group.cards.retain(|&c| c != card);
        if let Some(ids) = &mut group.ids
            && !remove(ids, is)
        {
            debug_assert!(false, "{card:?} ({is}) missing from its group's identities");
            group.ids = None;
        }
        self.new_group(vec![card], Some(CardCount::from([(is, 1)])));
    }

    /// Cards with only one possible identity no longer constrain their families.
    fn reduce(&mut self) {
        loop {
            let known: Vec<_> = self
                .groups
                .iter()
                .filter_map(|g| {
                    let ids = g.ids.as_ref()?;
                    (ids.len() == 1 && !g.cards.is_empty() && g.family.is_some())
                        .then(|| (g.cards.clone(), *ids.keys().next().unwrap()))
                })
                .collect();
            if known.is_empty() {
                break;
            }
            for (cards, is) in known {
                for c in cards {
                    self.identify_exact(c, is);
                }
            }
        }
    }

    /// A family down to one group: that group is exactly the family's cards.
    fn tidy(&mut self, f: usize) {
        if let [g] = self.families[f].groups[..] {
            let family = std::mem::take(&mut self.families[f]);
            let group = &mut self.groups[g];
            group.ids = family.ids;
            group.extra = 0;
            group.family = None;
        }
    }

    /// Splits the groups `cards` are in that also hold other cards, so that `cards` are
    /// groups of their own: each part is some of what the group was.
    fn split_off(&mut self, cards: &[CardId]) {
        let inside: HashSet<CardId> = cards.iter().copied().collect();
        let mut groups: Vec<usize> = cards.iter().map(|c| self.group_of[c.index()]).collect();
        groups.sort_unstable();
        groups.dedup();
        for g in groups {
            let (inn, out): (Vec<CardId>, Vec<CardId>) = self.groups[g]
                .cards
                .iter()
                .partition(|c| inside.contains(c));
            if out.is_empty() {
                continue;
            }
            let group = &self.groups[g];
            let (ids, extra) = (group.ids.clone(), group.extra);
            let family = match group.family {
                Some(f) => Some(f),
                None if ids.is_some() => {
                    self.families.push(Family {
                        ids: ids.clone(),
                        groups: vec![g],
                    });
                    Some(self.families.len() - 1)
                }
                None => None,
            };
            let new = self.groups.len();
            for &c in &inn {
                self.group_of[c.index()] = new;
            }
            let group = &mut self.groups[g];
            group.extra = extra + inn.len() as u32;
            group.family = family;
            self.groups.push(Group {
                ids,
                extra: extra + out.len() as u32,
                family,
                cards: inn,
            });
            self.groups[g].cards = out;
            if let Some(f) = family {
                self.families[f].groups.push(new);
            }
        }
    }

    /// The viewer sees that one of `among` is an `is`: `card`, but any of them that may be
    /// an `is` could be the one.
    fn identify_among(&mut self, card: CardId, is: Card, among: &[CardId]) {
        let may_be = |c: &CardId| {
            let ids = &self.groups[self.group_of[c.index()]].ids;
            ids.as_ref().is_none_or(|ids| ids.contains_key(&is))
        };
        let same: Vec<CardId> = among
            .iter()
            .copied()
            .filter(|c| !self.recent.contains(c))
            .filter(may_be)
            .chain([card])
            .collect();
        // This may be a copy the viewer already counted in the held set.
        let known = std::iter::once(card).chain(same.iter().copied()).find(|c| {
            self.groups[self.group_of[c.index()]]
                .ids
                .as_ref()
                .is_some_and(|ids| ids.len() == 1 && ids.contains_key(&is))
        });
        if let Some(other) = known {
            let (a, b) = (self.group_of[card.index()], self.group_of[other.index()]);
            if a != b {
                for c in &mut self.groups[a].cards {
                    if *c == card {
                        *c = other;
                    }
                }
                for c in &mut self.groups[b].cards {
                    if *c == other {
                        *c = card;
                    }
                }
                self.group_of[card.index()] = b;
                self.group_of[other.index()] = a;
            }
        } else {
            self.merge(same);
        }
        self.identify(card, is);
    }

    /// The viewer sees `card`, an `is`, in or leaving `zone`. `held` is the set it is
    /// held in out of their sight, if any.
    fn see(&mut self, card: CardId, is: Card, zone: Zone, held: Option<&[CardId]>) {
        let Some(held) = held else {
            self.disturb(card, zone);
            self.identify(card, is);
            return;
        };
        let mut among = held.to_vec();
        match &mut self.leaving {
            // Cards of this set left unseen: this one could as well be one of those.
            Some(l) if l.rest.contains(&card) && among.iter().all(|c| l.rest.contains(c)) => {
                l.rest.retain(|&c| c != card);
                l.seen.push(card);
                among.extend(&l.left);
            }
            _ => self.settle(),
        }
        self.identify_among(card, is, &among);
    }

    /// `card` leaves `from` for `to` and the viewer doesn't see what it is. `held` is the
    /// set it was held in out of their sight, if any.
    fn lose(&mut self, card: CardId, from: Zone, to: Zone, held: Option<&[CardId]>) {
        self.disturb(card, from);
        let arrived = self
            .arrived
            .iter()
            .position(|set| set.contains(&card))
            .map(|i| self.arrived.swap_remove(i));
        let continuing = self
            .leaving
            .as_ref()
            .filter(|l| l.from == from && l.to == to && l.rest.contains(&card))
            .map(|l| l.rest.clone());
        let held = held.or(arrived.as_deref()).or(continuing.as_deref());
        let Some(held) = held else {
            return;
        };
        let rest: Vec<CardId> = held.iter().copied().filter(|&c| c != card).collect();
        match &mut self.leaving {
            Some(l)
                if l.to == to
                    && l.rest.contains(&card)
                    && rest.iter().all(|c| l.rest.contains(c)) =>
            {
                l.rest.retain(|&c| c != card);
                l.left.push(card);
            }
            _ => {
                self.settle();
                self.leaving = Some(Leaving {
                    from,
                    to,
                    left: vec![card],
                    rest,
                    seen: vec![],
                });
            }
        }
    }

    /// A card the viewer can follow is in `zone`, or leaving it: if that touches the
    /// cards that just left a held set, the viewer stops waiting for the rest of it.
    fn disturb(&mut self, card: CardId, zone: Zone) {
        let touched =
            |l: &Leaving| !l.seen.contains(&card) && (l.to == zone || l.left.contains(&card));
        if self.leaving.as_ref().is_some_and(touched) {
            self.settle();
        }
    }

    /// The cards that left a held set unseen could be any of it.
    fn settle(&mut self) {
        if let Some(l) = self.leaving.take() {
            if l.rest.is_empty() {
                if l.left.len() > 1
                    && matches!(l.from, Zone::Hand(_))
                    && matches!(l.to, Zone::Discard(_))
                    && l.left
                        .iter()
                        .any(|c| self.group_of[c.index()] != self.group_of[l.left[0].index()])
                {
                    self.arrived.push(l.left);
                }
                return;
            }
            // Only the set's cards: the rest of their groups didn't go anywhere.
            let set: Vec<CardId> = l.left.into_iter().chain(l.rest).collect();
            self.split_off(&set);
            self.merge(set);
        }
    }

    /// The viewer can no longer tell `cards` apart: their groups become one. A family all
    /// of whose groups are among them is whole again.
    fn merge(&mut self, cards: impl IntoIterator<Item = CardId>) {
        let mut groups: Vec<usize> = cards
            .into_iter()
            .map(|c| self.group_of[c.index()])
            .collect();
        groups.sort_unstable();
        groups.dedup();
        let Some((&into, rest)) = groups.split_first() else {
            return;
        };
        if rest.is_empty() {
            return;
        }
        let mut families: Vec<usize> = groups
            .iter()
            .filter_map(|&g| self.groups[g].family)
            .collect();
        families.sort_unstable();
        families.dedup();
        let (whole, partial): (Vec<usize>, Vec<usize>) = families
            .into_iter()
            .partition(|&f| self.families[f].groups.iter().all(|g| groups.contains(g)));
        // What the cards outside partial families are, and what the others may be.
        let mut exact = Some(CardCount::new());
        let mut pool = Some(CardCount::new());
        let mut extra = 0;
        for &g in &groups {
            let group = &self.groups[g];
            match group.family {
                Some(f) if partial.contains(&f) => {
                    add_all(&mut pool, group.ids.as_ref());
                    extra += group.extra;
                }
                Some(_) => {}
                None => add_all(&mut exact, group.ids.as_ref()),
            }
        }
        for &f in &whole {
            let family = std::mem::take(&mut self.families[f]);
            add_all(&mut exact, family.ids.as_ref());
        }
        for &g in rest {
            let taken = std::mem::take(&mut self.groups[g]);
            for &c in &taken.cards {
                self.group_of[c.index()] = into;
            }
            self.groups[into].cards.extend(taken.cards);
        }
        let Some((&f, others)) = partial.split_first() else {
            let group = &mut self.groups[into];
            (group.ids, group.extra, group.family) = (exact, 0, None);
            return;
        };
        // The partial families become one, which the cards that joined them are part of.
        for &o in others {
            let family = std::mem::take(&mut self.families[o]);
            for &g in &family.groups {
                self.groups[g].family = Some(f);
            }
            add_all(&mut self.families[f].ids, family.ids.as_ref());
            self.families[f].groups.extend(family.groups);
        }
        add_all(&mut self.families[f].ids, exact.as_ref());
        self.families[f].groups.retain(|g| !groups.contains(g));
        self.families[f].groups.push(into);
        add_all(&mut pool, exact.as_ref());
        // No more of a card than the whole family has.
        if let (Some(pool), Some(all)) = (&mut pool, &self.families[f].ids) {
            for (card, n) in pool.iter_mut() {
                let most = all.get(card).copied().unwrap_or(0);
                extra -= n.saturating_sub(most);
                *n = (*n).min(most);
            }
            pool.retain(|_, n| *n > 0);
        }
        let group = &mut self.groups[into];
        (group.ids, group.extra, group.family) = (pool, extra, Some(f));
        self.tidy(f);
    }
}

/// Whether everyone can tell this card from its back: Stash, and the Shadow cards, each
/// with a back of its own.
fn marked(card: Card) -> bool {
    card == Card::Stash || card.is(Types::SHADOW)
}

/// Whether nobody saw which cards went into `pile`: the Ruins are dealt out face down,
/// unless all of them are used.
fn secret(game: &Game, pile: &Pile) -> bool {
    pile.card == Card::Ruins && game.num_players() < 6
}

fn public(game: &Game, zone: Zone) -> bool {
    match zone {
        Zone::InPlay(_) | Zone::Trash => true,
        Zone::Mat(_, mat) => mat != Mat::NativeVillage,
        Zone::Pile(pile) => !game.pile(pile).hidden,
        Zone::Deck(_) | Zone::Hand(_) | Zone::Discard(_) | Zone::SetAside(_) | Zone::Looking(_) => {
            false
        }
    }
}

/// Whether the owner's view of `zone` lets `v` follow cards in and out of it.
fn private_to(zone: Zone, v: PlayerId) -> bool {
    match zone {
        Zone::Hand(p) | Zone::SetAside(p) | Zone::Looking(p) | Zone::Mat(p, Mat::NativeVillage) => {
            v == p
        }
        _ => false,
    }
}

impl Knowledge {
    pub fn new(players: usize) -> Knowledge {
        Knowledge {
            zone: vec![],
            is: vec![],
            members: HashMap::new(),
            minds: vec![Mind::default(); players],
            created_in: None,
            held: vec![],
            sets: 0,
            joining: None,
            shown: None,
            face_up: HashSet::new(),
        }
    }

    /// Consumes the game's trace so far, then records what each player sees now.
    pub fn update(&mut self, game: &mut Game) {
        for entry in game.take_trace() {
            self.apply(game, entry);
        }
        self.settle();
        self.observe(game);
        self.minds.iter_mut().for_each(Mind::reduce);
    }

    fn settle(&mut self) {
        self.minds.iter_mut().for_each(Mind::settle);
    }

    fn apply(&mut self, game: &Game, entry: Trace) {
        let shown = self.shown.take();
        let joining = self.joining.take();
        if !matches!(entry, Trace::Created { .. }) {
            self.created_in = None;
        }
        match entry {
            Trace::Created { card, is, zone } => {
                self.settle();
                self.created(game, card, is, zone);
            }
            // Cards rearranged in place: `Mixed` says who can't follow that.
            Trace::Moved { from, to, .. } if from == to => {}
            Trace::Moved { card, from, to, .. } => {
                let face_up = shown == Some(card);
                self.moved(game, card, from, to, face_up, joining);
            }
            Trace::Mixed { cards } => {
                self.settle();
                self.minds.iter_mut().for_each(Mind::reduce);
                let mut held: Vec<_> = cards.iter().filter_map(|&c| self.held_with(c)).collect();
                held.sort_unstable();
                held.dedup();
                self.face_up.retain(|c| !cards.contains(c));
                // Mixed with other cards (a whole deck shuffled), a held set's cards are
                // no longer the ones looked at together.
                let set = |c: &CardId| self.held[c.index()];
                if cards.iter().any(|c| set(c) != set(&cards[0])) {
                    cards.iter().for_each(|c| self.held[c.index()] = 0);
                }
                let mixed: Vec<_> = cards
                    .iter()
                    .copied()
                    .filter(|c| !marked(self.is[c.index()]))
                    .collect();
                for (v, mind) in self.minds.iter_mut().enumerate() {
                    let arrivals = std::mem::take(&mut mind.arrived);
                    for set in arrivals {
                        if set.iter().any(|c| cards.contains(c)) {
                            if set.iter().any(|c| !cards.contains(c)) {
                                mind.split_off(&set);
                                mind.merge(set);
                            }
                        } else {
                            mind.arrived.push(set);
                        }
                    }
                    for set in &held {
                        if set
                            .iter()
                            .any(|c| self.zone[c.index()].owner() != Some(PlayerId(v as u8)))
                        {
                            mind.split_off(set);
                            mind.merge(set.iter().copied());
                        }
                    }
                    mind.split_off(&mixed);
                    mind.merge(mixed.iter().copied());
                    for c in &mixed {
                        mind.recent.remove(c);
                    }
                }
            }
            Trace::Shown { card, to } => {
                let is = self.is[card.index()];
                let zone = self.zone[card.index()];
                let held = self.held_with(card);
                for (v, mind) in self.minds.iter_mut().enumerate() {
                    let v = PlayerId(v as u8);
                    if to.is_none_or(|p| p == v) {
                        let hidden = zone.owner().is_some_and(|p| p != v);
                        let held = held.as_deref().filter(|_| hidden);
                        mind.see(
                            card,
                            is,
                            zone,
                            held.filter(|_| !mind.recent.contains(&card)),
                        );
                        mind.recent.insert(card);
                    }
                }
                match to {
                    None => {
                        self.held[card.index()] = 0;
                        if matches!(zone, Zone::Looking(_)) {
                            self.face_up.insert(card);
                        }
                        self.shown = Some(card);
                    }
                    Some(p) => {
                        if matches!(zone, Zone::Deck(_) | Zone::Discard(_))
                            && zone.owner() == Some(p)
                        {
                            self.hold(card, zone, joining);
                        }
                    }
                }
            }
            Trace::LookedOver { player, cards } => {
                let mind = &mut self.minds[player.index()];
                for card in cards {
                    mind.see(card, self.is[card.index()], self.zone[card.index()], None);
                    mind.recent.insert(card);
                }
            }
            Trace::Turn { .. } => self.settle(),
            Trace::Changed { card, from, to } => {
                self.settle();
                self.is[card.index()] = to;
                for mind in &mut self.minds {
                    let group = &mut mind.groups[mind.group_of[card.index()]];
                    if let Some(ids) = &mut group.ids {
                        remove(ids, from);
                        add(ids, to, 1);
                    }
                    if let Some(f) = group.family
                        && let Some(ids) = &mut mind.families[f].ids
                    {
                        remove(ids, from);
                        add(ids, to, 1);
                    }
                    if marked(to) {
                        mind.identify(card, to);
                    }
                }
            }
        }
    }

    fn created(&mut self, game: &Game, card: CardId, is: Card, zone: Zone) {
        if self.zone.len() <= card.index() {
            self.zone.resize(card.index() + 1, zone);
            self.is.resize(card.index() + 1, is);
        }
        if self.held.len() < self.zone.len() {
            self.held.resize(self.zone.len(), 0);
        }
        self.zone[card.index()] = zone;
        self.is[card.index()] = is;
        self.members.entry(zone).or_default().push(card);
        let container = match zone.owner() {
            Some(p) => Container::Player(p),
            None => Container::Zone(zone),
        };
        if marked(is) {
            for mind in &mut self.minds {
                mind.new_group(vec![card], Some(CardCount::from([(is, 1)])));
            }
            return;
        }
        let joins = self.created_in.filter(|&(c, _)| c == container);
        // Which cards a pile is made of is public, even when its order isn't.
        let known = match zone {
            Zone::Pile(pile) => !secret(game, game.pile(pile)),
            _ => true,
        };
        for mind in &mut self.minds {
            match joins {
                Some((_, last)) => mind.add_to(mind.group_of[last.index()], card, is),
                None => mind.new_group(vec![card], known.then(|| CardCount::from([(is, 1)]))),
            }
        }
        self.created_in = Some((container, card));
    }

    /// The cards `card` is held with out of the other players' sight, itself included.
    fn held_with(&self, card: CardId) -> Option<Vec<CardId>> {
        if marked(self.is[card.index()]) || self.face_up.contains(&card) {
            return None;
        }
        let zone = self.zone[card.index()];
        let set = match zone {
            Zone::Hand(_) | Zone::Looking(_) | Zone::Mat(_, Mat::NativeVillage) => None,
            Zone::Deck(_) | Zone::Discard(_) | Zone::SetAside(_)
                if self.held[card.index()] != 0 =>
            {
                Some(self.held[card.index()])
            }
            _ => return None,
        };
        let with = |c: &CardId| {
            !marked(self.is[c.index()])
                && !self.face_up.contains(c)
                && set.is_none_or(|set| self.held[c.index()] == set)
        };
        Some(self.members[&zone].iter().copied().filter(with).collect())
    }

    /// Puts `card`, in `zone`, in the held set the last entry added to there, or a new one.
    fn hold(&mut self, card: CardId, zone: Zone, joining: Option<(Zone, u32)>) {
        let set = match joining {
            Some((z, set)) if z == zone => set,
            _ => {
                self.sets += 1;
                self.sets
            }
        };
        self.held[card.index()] = set;
        self.joining = Some((zone, set));
    }

    /// `face_up` says the card was shown to everyone just before.
    fn moved(
        &mut self,
        game: &Game,
        card: CardId,
        from: Zone,
        to: Zone,
        face_up: bool,
        joining: Option<(Zone, u32)>,
    ) {
        let is = self.is[card.index()];
        let held = self.held_with(card);
        self.face_up.remove(&card);
        if matches!(to, Zone::Looking(_)) && face_up {
            self.face_up.insert(card);
        }
        self.zone[card.index()] = to;
        if let Some(cards) = self.members.get_mut(&from) {
            cards.retain(|&c| c != card);
        }
        self.members.entry(to).or_default().push(card);
        self.held[card.index()] = 0;
        // Set aside face down: with the cards set aside along with it.
        if matches!(to, Zone::SetAside(_)) && !face_up {
            if marked(is) {
                // Its back is known, not which of the cards around it is which.
                self.joining = joining;
            } else {
                self.hold(card, to, joining);
            }
        }
        let shown = public(game, from) || public(game, to) || marked(is);
        for (v, mind) in self.minds.iter_mut().enumerate() {
            let v = PlayerId(v as u8);
            let hidden = from.owner().is_some_and(|p| p != v);
            let followed = mind.recent.remove(&card);
            let held = held.as_deref().filter(|_| hidden && !followed);
            if shown || private_to(from, v) || private_to(to, v) {
                mind.see(card, is, from, held);
            } else {
                mind.lose(card, from, to, held);
            }
        }
    }

    /// Every card a player sees now (their hand, public cards, cards offered to them)
    /// becomes known to them.
    fn observe(&mut self, game: &Game) {
        for (v, mind) in self.minds.iter_mut().enumerate() {
            let view = game.view(PlayerId(v as u8));
            for (i, &is) in self.is.iter().enumerate() {
                let id = CardId(i as u32);
                if view.card(id).is_some() {
                    mind.identify(id, is);
                }
            }
            mind.recent.clear();
        }
    }

    /// `viewer`'s groups, singletons included.
    pub fn groups(&self, viewer: PlayerId) -> impl Iterator<Item = GroupView<'_>> {
        self.minds[viewer.index()]
            .groups
            .iter()
            .filter(|g| !g.cards.is_empty())
            .map(|g| GroupView {
                cards: &g.cards,
                ids: g.ids.as_ref(),
                extra: g.extra,
            })
    }

    /// What `viewer` knows about every zone of `game`, which must be the game this
    /// knowledge has been updated with. Cards the viewer sees count as what they
    /// currently are (Band of Misfits in play); others as printed.
    pub fn known(&self, game: &Game, viewer: PlayerId) -> KnownCards {
        let view = game.view(viewer);
        let mut out = KnownCards {
            owned: vec![(CardCount::new(), 0); game.num_players()],
            ..KnownCards::default()
        };
        let zones_of = |cards: &mut dyn Iterator<Item = CardId>| {
            let mut zones: Vec<(Zone, usize)> = vec![];
            for c in cards {
                let zone = game.zone(c);
                match zones.iter_mut().find(|(z, _)| *z == zone) {
                    Some((_, n)) => *n += 1,
                    None => zones.push((zone, 1)),
                }
            }
            // Not in the cards' order, which says which card is where.
            zones.sort();
            zones
        };
        // A card is owned as what it is printed as; how many cards each player owns is
        // public, whoever's they can't be told apart from.
        let mut own = |zones: &[(Zone, usize)], ids: Option<&CardCount>| {
            let owner = zones[0].0.owner();
            let one_owner = zones.iter().all(|(z, _)| z.owner() == owner);
            match (owner.filter(|_| one_owner), ids) {
                (Some(p), Some(ids)) => {
                    let owned = &mut out.owned[p.index()].0;
                    ids.iter().for_each(|(&c, &n)| add(owned, c, n));
                }
                _ => {
                    for &(zone, n) in zones {
                        if let Some(p) = zone.owner() {
                            out.owned[p.index()].1 += n;
                        }
                    }
                }
            }
        };
        let mind = &self.minds[viewer.index()];
        let mut whole = HashSet::new();
        let mut whole_zones = vec![];
        for (i, family) in mind
            .families
            .iter()
            .enumerate()
            .filter(|(_, f)| !f.groups.is_empty())
        {
            let mut cards = family
                .groups
                .iter()
                .flat_map(|&g| mind.groups[g].cards.iter().copied());
            let zones = zones_of(&mut cards);
            own(&zones, family.ids.as_ref());
            if let ([(zone, n)], Some(ids)) = (&zones[..], &family.ids) {
                whole.insert(i);
                whole_zones.push((*zone, *n, ids));
            }
        }
        let groups = mind.groups.iter().filter(|g| !g.cards.is_empty());
        for (group, zones) in groups.map(|g| (g, zones_of(&mut g.cards.iter().copied()))) {
            if group.family.is_none() {
                own(&zones, group.ids.as_ref());
            }
        }
        for (zone, n, ids) in whole_zones {
            let z = out.zones.entry(zone).or_default();
            z.count += n;
            for (&c, &k) in ids {
                add(&mut z.known, c, k);
            }
        }
        for group in mind.groups.iter().filter(|g| !g.cards.is_empty()) {
            if group.family.is_some_and(|f| whole.contains(&f)) {
                continue;
            }
            let zones = zones_of(&mut group.cards.iter().copied());
            let mut extra = group.extra;
            let ids = match (&group.cards[..], &group.ids) {
                ([c], Some(_)) if view.card(*c).is_some() => {
                    view.card(*c).map(|card| CardCount::from([(card, 1)]))
                }
                (_, Some(ids)) => {
                    // No more of a card than its whole family has.
                    let all = group.family.and_then(|f| mind.families[f].ids.as_ref());
                    let mut ids = ids.clone();
                    if let Some(all) = all {
                        for (card, n) in ids.iter_mut() {
                            let most = all.get(card).copied().unwrap_or(0);
                            extra -= n.saturating_sub(most);
                            *n = (*n).min(most);
                        }
                        ids.retain(|_, n| *n > 0);
                    }
                    Some(ids)
                }
                (_, None) => None,
            };
            // Cards that can only be one card are known wherever they are.
            let only = match &ids {
                Some(ids) if ids.len() == 1 => ids.keys().next().copied(),
                _ => None,
            };
            for &(zone, n) in &zones {
                let z = out.zones.entry(zone).or_default();
                z.count += n;
                match (&ids, only) {
                    (_, Some(card)) => add(&mut z.known, card, n as u32),
                    // Some of the candidates: each is there as often as it is a candidate
                    // more times than there are candidates elsewhere.
                    (Some(ids), _) if zones.len() == 1 => {
                        let mut known = 0;
                        for (&c, &k) in ids {
                            let at_least = k.saturating_sub(extra);
                            if at_least > 0 {
                                add(&mut z.known, c, at_least);
                                known += at_least as usize;
                            }
                        }
                        z.unknown += n - known;
                    }
                    _ => z.unknown += n,
                }
            }
            if zones.len() > 1
                && only.is_none()
                && let Some(candidates) = ids
            {
                out.uncertain.push(UncertainCards {
                    zones,
                    candidates,
                    outside: extra as usize,
                });
            }
        }
        out.uncertain
            .sort_by(|a, b| (&a.zones, &a.candidates).cmp(&(&b.zones, &b.candidates)));
        out
    }
}
