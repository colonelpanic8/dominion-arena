//! What each player can know about every physical card, with perfect recall.
//!
//! For each viewer, every card belongs to a group of cards the viewer can't tell apart;
//! the viewer knows each group's cards (where each one is: card movements are public)
//! and, unless it is unknown, the multiset of their identities. A card the viewer sees
//! becomes a group of its own. Shuffles and other rearrangements no one can follow merge
//! groups, and so does a card leaving a place where the viewer couldn't tell which card
//! it was (an opponent's hand, the middle of a deck or discard pile). Deck order is never
//! tracked.
//!
//! [`Knowledge`] consumes a traced game's [`Trace`]; build it before the game's first
//! [`Game::take_trace`] and call [`Knowledge::update`] after every response.

use crate::game::{Mat, Zone};
use crate::{Card, CardId, Game, PlayerId, Trace};
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
    /// The turn's player and who controls it, as of the trace consumed so far.
    turn: (PlayerId, PlayerId),
    /// Where the last `Created` card went, and that card, to group cards created together.
    created_in: Option<(Container, CardId)>,
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
    /// Cards this viewer identified since the last decision: they can follow them.
    recent: HashSet<CardId>,
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

/// Whether `v` sees what `p` sees: their own cards, or those of a player whose turn they
/// control.
fn sees_for(turn: (PlayerId, PlayerId), v: PlayerId, p: PlayerId) -> bool {
    v == p || turn == (p, v)
}

fn public(game: &Game, zone: Zone) -> bool {
    match zone {
        Zone::InPlay(_) | Zone::Trash => true,
        Zone::Mat(_, mat) => mat != Mat::NativeVillage,
        Zone::Pile(pile) => !game.pile(pile).hidden,
        Zone::Deck(_) | Zone::Hand(_) | Zone::Discard(_) | Zone::SetAside(_) => false,
    }
}

/// Whether the owner's view of `zone` lets `v` follow cards in and out of it.
fn private_to(turn: (PlayerId, PlayerId), zone: Zone, v: PlayerId) -> bool {
    match zone {
        Zone::Hand(p) | Zone::SetAside(p) | Zone::Mat(p, Mat::NativeVillage) => {
            sees_for(turn, v, p)
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
            turn: (PlayerId(0), PlayerId(0)),
            created_in: None,
        }
    }

    /// Consumes the game's trace so far, then records what each player sees now.
    pub fn update(&mut self, game: &mut Game) {
        for entry in game.take_trace() {
            self.apply(game, entry);
        }
        self.observe(game);
    }

    fn apply(&mut self, game: &Game, entry: Trace) {
        match entry {
            Trace::Created { card, is, zone } => self.created(game, card, is, zone),
            Trace::Moved {
                card,
                from,
                to,
                top,
            } => {
                self.created_in = None;
                self.moved(game, card, from, to, top);
            }
            Trace::Mixed { cards } => {
                self.created_in = None;
                for mind in &mut self.minds {
                    mind.merge(cards.iter().copied());
                }
            }
            Trace::Shown { card, to } => {
                self.created_in = None;
                let is = self.is[card.index()];
                for (v, mind) in self.minds.iter_mut().enumerate() {
                    if to.is_none_or(|p| sees_for(self.turn, PlayerId(v as u8), p)) {
                        mind.identify(card, is);
                        mind.recent.insert(card);
                    }
                }
            }
            Trace::Turn { player, controller } => self.turn = (player, controller),
            Trace::Changed { card, from, to } => {
                self.created_in = None;
                self.is[card.index()] = to;
                for mind in &mut self.minds {
                    let group = &mut mind.groups[mind.group_of[card.index()]];
                    if let Some(ids) = &mut group.ids {
                        remove(ids, from);
                        add(ids, to, 1);
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
        self.zone[card.index()] = zone;
        self.is[card.index()] = is;
        self.members.entry(zone).or_default().push(card);
        let container = match zone.owner() {
            Some(p) => Container::Player(p),
            None => Container::Zone(zone),
        };
        let pile = match zone {
            Zone::Pile(pile) => Some(game.pile(pile)),
            _ => None,
        };
        let joins = self.created_in.filter(|&(c, _)| c == container);
        for mind in &mut self.minds {
            match joins {
                Some((_, last)) => {
                    // A pile's identities are public only if all its cards are the same.
                    let g = mind.group_of[last.index()];
                    let group = &mut mind.groups[g];
                    if pile.is_some()
                        && group.ids.as_ref().is_some_and(|ids| !ids.contains_key(&is))
                    {
                        group.ids = None;
                    }
                    mind.add_to(g, card, is);
                }
                None => {
                    let known = !pile.is_some_and(|p| p.hidden);
                    mind.new_group(vec![card], known.then(|| CardCount::from([(is, 1)])));
                }
            }
        }
        self.created_in = Some((container, card));
    }

    fn moved(&mut self, game: &Game, card: CardId, from: Zone, to: Zone, top: bool) {
        self.zone[card.index()] = to;
        if let Some(cards) = self.members.get_mut(&from) {
            cards.retain(|&c| c != card);
        }
        self.members.entry(to).or_default().push(card);
        let is = self.is[card.index()];
        let shown = public(game, from) || public(game, to);
        for (v, mind) in self.minds.iter_mut().enumerate() {
            let v = PlayerId(v as u8);
            if shown || private_to(self.turn, from, v) || private_to(self.turn, to, v) {
                mind.identify(card, is);
                mind.recent.insert(card);
                continue;
            }
            if mind.recent.contains(&card) {
                continue;
            }
            let lost = match from {
                Zone::Hand(_) | Zone::Mat(_, Mat::NativeVillage) => true,
                Zone::Deck(_) | Zone::Discard(_) => !top,
                _ => false,
            };
            if lost && let Some(rest) = self.members.get(&from) {
                mind.merge(rest.iter().copied().chain([card]));
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
            let owner = zones[0].0.owner();
            let one_owner = zones.iter().all(|(z, _)| z.owner() == owner);
            let ids = match (group.cards, group.ids) {
                ([c], Some(_)) if view.card(*c).is_some() => {
                    view.card(*c).map(|card| CardCount::from([(card, 1)]))
                }
                (_, ids) => ids.cloned(),
            };
            if let Some(p) = owner.filter(|_| one_owner) {
                let owned = &mut out.owned[p.index()];
                match &ids {
                    Some(ids) => ids.iter().for_each(|(&c, &n)| add(&mut owned.0, c, n)),
                    None => owned.1 += group.cards.len(),
                }
            }
            for &(zone, n) in &zones {
                let z = out.zones.entry(zone).or_default();
                z.count += n;
                match &ids {
                    Some(ids) if zones.len() == 1 => {
                        ids.iter().for_each(|(&c, &n)| add(&mut z.known, c, n))
                    }
                    _ => z.unknown += n,
                }
            }
            if zones.len() > 1
                && let Some(candidates) = ids
            {
                out.uncertain.push(UncertainCards { zones, candidates });
            }
        }
        out
    }
}
