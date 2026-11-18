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
//!
//! [`Knowledge::with_watcher`] also keeps what someone watching without a seat knows: a
//! viewer who is no player, so they follow exactly what is public.

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
    /// What someone watching knows, if kept.
    #[serde(default)]
    watcher: Option<Mind>,
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
    /// Cards put in a private order while in a public zone: everyone follows them out of
    /// it, so they are mixed once they have left it.
    #[serde(default)]
    ordering: Vec<CardId>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
enum Container {
    Player(PlayerId),
    Zone(Zone),
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

type Mind = dominion_knowledge::Mind<Card, CardId, Zone>;
pub type GroupView<'a> = dominion_knowledge::GroupView<'a, Card, CardId>;
pub type ZoneKnowledge = dominion_knowledge::ZoneKnowledge<Card>;
pub type UncertainCards = dominion_knowledge::UncertainCards<Card, Zone>;
pub type KnownCards = dominion_knowledge::KnownCards<Card, Zone>;

impl dominion_knowledge::Instance for CardId {
    fn index(self) -> usize {
        self.index()
    }
}
impl dominion_knowledge::Location for Zone {
    fn is_hand(self) -> bool {
        matches!(self, Zone::Hand(_))
    }
    fn is_discard(self) -> bool {
        matches!(self, Zone::Discard(_))
    }
}

/// Whether everyone can tell this card from its back: Stash, and the Shadow cards, each
/// with a back of its own.
pub(crate) fn marked(card: Card) -> bool {
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

/// A viewer: a player, or `None` for someone watching, who is no player.
type Viewer = Option<PlayerId>;

/// Whether the owner's view of `zone` lets `v` follow cards in and out of it.
fn private_to(zone: Zone, v: Viewer) -> bool {
    match zone {
        Zone::Hand(p) | Zone::SetAside(p) | Zone::Looking(p) | Zone::Mat(p, Mat::NativeVillage) => {
            v == Some(p)
        }
        _ => false,
    }
}

fn view_of(game: &Game, v: Viewer) -> crate::View<'_> {
    match v {
        Some(p) => game.view(p),
        None => game.watch(),
    }
}

/// Every viewer's mind: the players', then the watcher's. Takes the two fields, so that
/// the rest of the knowledge can be read meanwhile.
fn minds<'a>(
    players: &'a mut [Mind],
    watcher: &'a mut Option<Mind>,
) -> impl Iterator<Item = (Viewer, &'a mut Mind)> {
    let players = players.iter_mut().enumerate();
    players
        .map(|(v, mind)| (Some(PlayerId(v as u8)), mind))
        .chain(watcher.iter_mut().map(|mind| (None, mind)))
}

impl Knowledge {
    pub fn new(players: usize) -> Knowledge {
        Knowledge {
            zone: vec![],
            is: vec![],
            members: HashMap::new(),
            minds: vec![Mind::default(); players],
            watcher: None,
            created_in: None,
            held: vec![],
            sets: 0,
            joining: None,
            shown: None,
            face_up: HashSet::new(),
            ordering: vec![],
        }
    }

    /// Also keeps what someone watching without a seat knows.
    pub fn with_watcher(players: usize) -> Knowledge {
        Knowledge {
            watcher: Some(Mind::default()),
            ..Knowledge::new(players)
        }
    }

    /// Whether it keeps what someone watching knows.
    pub fn watches(&self) -> bool {
        self.watcher.is_some()
    }

    /// Every viewer's mind: the players', then the watcher's.
    fn minds_mut(&mut self) -> impl Iterator<Item = (Viewer, &mut Mind)> {
        minds(&mut self.minds, &mut self.watcher)
    }

    fn mind(&self, v: Viewer) -> Option<&Mind> {
        match v {
            Some(p) => self.minds.get(p.index()),
            None => self.watcher.as_ref(),
        }
    }

    /// Consumes the game's trace so far, then records what each viewer sees now.
    pub fn update(&mut self, game: &mut Game) {
        for entry in game.take_trace() {
            self.apply(game, entry);
        }
        self.end_ordering(game);
        self.settle();
        self.observe(game);
        self.minds_mut().for_each(|(_, mind)| mind.reduce());
    }

    fn settle(&mut self) {
        self.minds_mut().for_each(|(_, mind)| mind.settle());
    }

    fn apply(&mut self, game: &Game, entry: Trace) {
        let moving = |card: &CardId| self.ordering.contains(card);
        if !matches!(&entry, Trace::Moved { card, .. } if moving(card)) {
            self.end_ordering(game);
        }
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
            Trace::Positioned { .. } => {}
            Trace::Moved { card, from, to, .. } => {
                let face_up = shown == Some(card);
                self.moved(game, card, from, to, face_up, joining);
                if self
                    .ordering
                    .iter()
                    .all(|c| !public(game, self.zone[c.index()]))
                {
                    self.end_ordering(game);
                }
            }
            Trace::Mixed { cards } => {
                let (ordering, cards): (Vec<CardId>, Vec<CardId>) = cards
                    .into_iter()
                    .partition(|c| public(game, self.zone[c.index()]));
                self.ordering = ordering;
                self.mix(&cards);
            }
            Trace::Shown { card, to } => {
                let is = self.is[card.index()];
                let zone = self.zone[card.index()];
                let held = self.held_with(card);
                for (v, mind) in self.minds_mut() {
                    if to.is_none_or(|p| Some(p) == v) {
                        let hidden = zone.owner().is_some_and(|p| Some(p) != v);
                        let held = held.as_deref().filter(|_| hidden);
                        mind.see(card, is, zone, held.filter(|_| !mind.is_recent(card)));
                        mind.remember(card);
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
                    mind.remember(card);
                }
            }
            Trace::Turn { .. } => self.settle(),
            Trace::Changed { card, from, to } => {
                self.settle();
                self.is[card.index()] = to;
                for (_, mind) in self.minds_mut() {
                    mind.change(card, from, to);
                    if marked(to) {
                        mind.identify(card, to);
                    }
                }
            }
        }
    }

    /// The cards an `ordering` put out of public view, mixed where they went.
    fn end_ordering(&mut self, game: &Game) {
        let cards = std::mem::take(&mut self.ordering);
        let gone: Vec<CardId> = cards
            .into_iter()
            .filter(|c| !public(game, self.zone[c.index()]))
            .collect();
        if gone.len() > 1 {
            self.mix(&gone);
        }
    }

    /// Nobody can follow `cards` any more.
    fn mix(&mut self, cards: &[CardId]) {
        self.settle();
        self.minds_mut().for_each(|(_, mind)| mind.reduce());
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
        for (v, mind) in minds(&mut self.minds, &mut self.watcher) {
            let arrivals = mind.take_arrivals();
            for set in arrivals {
                if set.iter().any(|c| cards.contains(c)) {
                    if set.iter().any(|c| !cards.contains(c)) {
                        mind.mix(&set);
                    }
                } else {
                    mind.keep_arrival(set);
                }
            }
            for set in &held {
                // Not all the viewer's own, which someone watching has none of.
                let theirs = |c: &CardId| {
                    let owner = self.zone[c.index()].owner();
                    owner.is_some_and(|o| Some(o) == v)
                };
                if !set.iter().all(theirs) {
                    mind.mix(set);
                }
            }
            mind.mix(&mixed);
            for c in &mixed {
                mind.forget_recent(*c);
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
            for (_, mind) in self.minds_mut() {
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
        for (_, mind) in self.minds_mut() {
            match joins {
                Some((_, last)) => mind.add_to(mind.group_of(last), card, is),
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
        for (v, mind) in self.minds_mut() {
            let hidden = from.owner().is_some_and(|p| Some(p) != v);
            let followed = mind.forget_recent(card);
            let held = held.as_deref().filter(|_| hidden && !followed);
            if shown || private_to(from, v) || private_to(to, v) {
                mind.see(card, is, from, held);
            } else {
                mind.lose(card, from, to, held);
            }
        }
    }

    /// Every card a viewer sees now (their hand, public cards, cards offered to them)
    /// becomes known to them.
    fn observe(&mut self, game: &Game) {
        let ids = std::mem::take(&mut self.is);
        for (v, mind) in self.minds_mut() {
            let view = view_of(game, v);
            for (i, &is) in ids.iter().enumerate() {
                let id = CardId(i as u32);
                if view.card(id).is_some() {
                    mind.identify(id, is);
                }
            }
            mind.clear_recent();
        }
        self.is = ids;
    }

    /// `viewer`'s groups, singletons included.
    pub fn groups(&self, viewer: PlayerId) -> impl Iterator<Item = GroupView<'_>> {
        self.minds[viewer.index()].views()
    }

    /// The groups of someone watching, if their knowledge is kept.
    pub fn watched_groups(&self) -> Option<impl Iterator<Item = GroupView<'_>>> {
        self.watcher.as_ref().map(Mind::views)
    }

    /// What `viewer` knows about every zone of `game`, which must be the game this
    /// knowledge has been updated with. Cards the viewer sees count as what they
    /// currently are (Band of Misfits in play); others as printed.
    pub fn known(&self, game: &Game, viewer: PlayerId) -> KnownCards {
        self.known_to(game, Some(viewer))
            .expect("the viewer is a player of the game")
    }

    /// [`Knowledge::known`] for someone watching, if their knowledge is kept: what every
    /// player may know.
    pub fn watched(&self, game: &Game) -> Option<KnownCards> {
        self.known_to(game, None)
    }

    fn known_to(&self, game: &Game, viewer: Viewer) -> Option<KnownCards> {
        let mind = self.mind(viewer)?;
        let view = view_of(game, viewer);
        Some(mind.summarize(
            game.num_players(),
            |c| game.zone(c),
            |z| z.owner().map(|p| p.index()),
            |c| view.card(c),
        ))
    }
}
