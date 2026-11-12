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
    /// face down together. It lasts while the card stays there unrevealed.
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
    /// Cards shown to this viewer since the last decision that haven't moved since: they
    /// can follow them.
    recent: HashSet<CardId>,
    #[serde(default)]
    leaving: Option<Leaving>,
}

/// Cards a viewer watched leave a held set for one place without seeing which they were.
/// Until the viewer knows where the rest of the set goes, the groups stay as they were.
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Leaving {
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
}

/// One group of indistinguishable cards, as one viewer knows it.
#[derive(Clone, Copy, Debug)]
pub struct GroupView<'a> {
    pub cards: &'a [CardId],
    /// Their identities as a multiset, if the viewer knows it.
    pub ids: Option<&'a CardCount>,
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
        self.groups.push(Group { cards, ids });
    }

    fn add_to(&mut self, g: usize, card: CardId, is: Card) {
        self.ensure(card);
        self.group_of[card.index()] = g;
        let group = &mut self.groups[g];
        group.cards.push(card);
        if let Some(ids) = &mut group.ids {
            add(ids, is, 1);
        }
    }

    /// The viewer sees `card`, which is `is`.
    fn identify(&mut self, card: CardId, is: Card) {
        let g = self.group_of[card.index()];
        let group = &mut self.groups[g];
        if group.cards.len() == 1 {
            group.ids = Some(CardCount::from([(is, 1)]));
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

    /// The viewer sees that one of `among` is an `is`: `card`, but any of them that may be
    /// an `is` could be the one.
    fn identify_among(&mut self, card: CardId, is: Card, among: &[CardId]) {
        let may_be = |c: &CardId| {
            let ids = &self.groups[self.group_of[c.index()]].ids;
            ids.as_ref().is_none_or(|ids| ids.contains_key(&is))
        };
        let same: Vec<CardId> = among.iter().copied().filter(may_be).chain([card]).collect();
        self.merge(same);
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
        let Some(held) = held else {
            self.disturb(card, from);
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

    /// The cards that left a held set unseen could be any of it, unless all of it left.
    fn settle(&mut self) {
        if let Some(l) = self.leaving.take()
            && !l.rest.is_empty()
        {
            self.merge(l.left.into_iter().chain(l.rest));
        }
    }

    /// The viewer can no longer tell `cards` apart.
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
        for &g in rest {
            let taken = std::mem::take(&mut self.groups[g]);
            for &c in &taken.cards {
                self.group_of[c.index()] = into;
            }
            let group = &mut self.groups[into];
            group.cards.extend(taken.cards);
            group.ids = match (group.ids.take(), taken.ids) {
                (Some(mut a), Some(b)) => {
                    for (card, n) in b {
                        add(&mut a, card, n);
                    }
                    Some(a)
                }
                _ => None,
            };
        }
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
                self.face_up.retain(|c| !cards.contains(c));
                let mixed = cards
                    .iter()
                    .copied()
                    .filter(|c| !marked(self.is[c.index()]));
                for mind in &mut self.minds {
                    mind.merge(mixed.clone());
                    mixed.clone().for_each(|c| {
                        mind.recent.remove(&c);
                    });
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
        if matches!(to, Zone::SetAside(_)) && !face_up && !marked(is) {
            self.hold(card, to, joining);
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
        for group in self.groups(viewer) {
            let mut zones: Vec<(Zone, usize)> = vec![];
            for &c in group.cards {
                let zone = game.zone(c);
                match zones.iter_mut().find(|(z, _)| *z == zone) {
                    Some((_, n)) => *n += 1,
                    None => zones.push((zone, 1)),
                }
            }
            // Not in the cards' order, which says which card is where.
            zones.sort();
            let owner = zones[0].0.owner();
            let one_owner = zones.iter().all(|(z, _)| z.owner() == owner);
            // A card is owned as what it is printed as; how many cards each player owns
            // is public, whoever's they can't be told apart from.
            match (owner.filter(|_| one_owner), group.ids) {
                (Some(p), Some(ids)) => {
                    let owned = &mut out.owned[p.index()].0;
                    ids.iter().for_each(|(&c, &n)| add(owned, c, n));
                }
                _ => {
                    for &(zone, n) in &zones {
                        if let Some(p) = zone.owner() {
                            out.owned[p.index()].1 += n;
                        }
                    }
                }
            }
            let ids = match (group.cards, group.ids) {
                ([c], Some(_)) if view.card(*c).is_some() => {
                    view.card(*c).map(|card| CardCount::from([(card, 1)]))
                }
                (_, ids) => ids.cloned(),
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
                    (Some(ids), _) if zones.len() == 1 => {
                        ids.iter().for_each(|(&c, &n)| add(&mut z.known, c, n))
                    }
                    _ => z.unknown += n,
                }
            }
            if zones.len() > 1
                && only.is_none()
                && let Some(candidates) = ids
            {
                out.uncertain.push(UncertainCards { zones, candidates });
            }
        }
        out.uncertain
            .sort_by(|a, b| (&a.zones, &a.candidates).cmp(&(&b.zones, &b.candidates)));
        out
    }
}
