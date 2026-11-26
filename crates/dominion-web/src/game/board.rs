//! A view arranged for drawing: piles in their places, players with their tokens as
//! badges, cards stacked in a stable order, and every other part of the view in a form
//! ready to show, so that nothing a view holds is lost on the table.

use super::meta::{self, SeatAccount};
use crate::cards::CardDb;
use crate::rating::SeatRating;
use dominion_protocol::api::{InPlay, TableExtras};
use dominion_protocol::lobby::GameMeta;
use dominion_protocol::{
    AgentView, CardCount, Cost, DeckView, PileMarker, PileView, PlayerView, ResultPlayer,
    SetupRole, ZoneView,
};
use std::cmp::Ordering;
use std::collections::{BTreeMap, HashMap};
use std::sync::LazyLock;

#[derive(Clone, Debug, PartialEq)]
pub struct Board {
    pub status: Status,
    pub supply: Supply,
    pub non_supply: Vec<PileEntry>,
    pub landscapes: Vec<LandscapeEntry>,
    /// The seat's own player; none for someone watching.
    pub you: Option<PlayerEntry>,
    /// The other players, in turn order from the seat; for someone watching, every
    /// player from the first seat.
    pub opponents: Vec<PlayerEntry>,
    /// The view of someone watching: no seat's.
    pub watching: bool,
    pub play_area: PlayArea,
    pub trash: Zone,
    pub boons: Option<CardDeck>,
    pub hexes: Option<CardDeck>,
    pub set_asides: Vec<SetupSetAside>,
    pub artifacts: Vec<Holding>,
    pub lasting: Vec<Lasting>,
    pub extra_turns: Vec<ExtraTurn>,
    pub notes: Vec<Note>,
    pub counters: Vec<(String, i64)>,
    /// Cards whose whereabouts are known only to be among several players' zones.
    pub uncertain: Vec<Uncertain>,
    pub result: Option<Vec<ResultPlayer>>,
    pub end_reason: Option<String>,
    /// Why the game ended early, when it did: `resigned` or `abandoned`.
    pub ended: Option<String>,
    pub rated: bool,
    /// It was to be rated, and was taken out of the rated record.
    pub voided: bool,
    pub timed: bool,
    /// People watching now.
    pub watchers: u32,
}

impl Board {
    /// What a card costs now where that differs from its printed cost, as the pile it is
    /// on top of says: dominion.games draws cards in hand and play at their current cost.
    pub fn changed_cost(&self, name: &str, cards: &CardDb) -> Option<Cost> {
        let piles = (self.supply.basic.iter())
            .chain(self.supply.kingdom.iter().flatten())
            .chain(&self.non_supply);
        let now = piles
            .filter(|p| p.card == name)
            .find_map(|p| p.cost.clone())?;
        let printed = cards.get(name).and_then(|c| c.cost)?;
        ((printed.coins, printed.potions, printed.debt) != (now.coins, now.potions, now.debt))
            .then_some(now)
    }

    /// What a card costs now, by name, for each card a pile shows: dominion.games prints
    /// that on every copy, wherever it is (Bridge, Highway, Canal, ...).
    pub fn current_costs(&self) -> HashMap<String, Cost> {
        self.supply
            .basic
            .iter()
            .chain(self.supply.kingdom.iter().flatten())
            .chain(&self.non_supply)
            .filter(|p| !p.hidden)
            .filter_map(|p| Some((p.card.clone(), p.cost.clone()?)))
            .collect()
    }

    /// The player drawn where the seat's own is: for someone watching, the first.
    pub fn front(&self) -> Option<&PlayerEntry> {
        self.you
            .as_ref()
            .or_else(|| self.opponents.first().filter(|_| self.watching))
    }

    /// The Project cubes `player`'s bar shows: dominion.games gives each player one per
    /// Project in the game, at most two, and takes one off for each Project they bought.
    pub fn project_cubes(&self, player: &PlayerEntry) -> usize {
        (self.landscapes.iter())
            .filter(|l| l.kind == "project")
            .count()
            .min(2)
            .saturating_sub(player.projects.len())
    }

    /// The players drawn as opponents: everyone but [`Board::front`].
    pub fn others(&self) -> &[PlayerEntry] {
        match (&self.you, self.watching) {
            (None, true) => self.opponents.get(1..).unwrap_or_default(),
            _ => &self.opponents,
        }
    }

    /// Every player.
    pub fn players(&self) -> impl Iterator<Item = &PlayerEntry> {
        self.you.iter().chain(&self.opponents)
    }

    /// Whether it is the turn of the player at the front.
    pub fn front_turn(&self) -> bool {
        let turn = self.status.player.as_ref().map(|w| w.seat);
        turn.is_some() && turn == self.front().map(|p| p.seat)
    }

    /// The board with what a response says beside its view: the accounts playing, who is
    /// connected, what kind of game it is and how it ended.
    pub fn with_meta(mut self, m: &GameMeta) -> Board {
        for p in self.you.iter_mut().chain(&mut self.opponents) {
            p.account = meta::account(m, p.seat);
            p.model = m.models.get(usize::from(p.seat)).cloned().flatten();
            p.level = meta::level(m, p.seat);
            p.rating = meta::rating(m, p.seat);
        }
        self.rated = m.rated;
        self.voided = m.voided;
        self.timed = m.timed || m.clock.is_some();
        self.watchers = m.watchers;
        self.ended = m.ended.clone();
        if let (Some(players), Some(ended)) = (&self.result, &self.ended)
            && players.iter().any(|p| p.resigned)
        {
            self.end_reason = meta::end_reason(Some("resigned"), players, Some(ended));
        }
        self
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Status {
    pub running: bool,
    pub phase: Option<String>,
    pub actions: Option<u32>,
    pub buys: Option<u32>,
    pub coins: Option<u32>,
    pub potions: Option<u32>,
    pub debt: Option<u32>,
    pub turn: Option<u32>,
    pub player: Option<Who>,
    pub your_turn: bool,
    /// What gave this turn, if it is an extra turn.
    pub extra: Option<String>,
    pub deciding: Option<Who>,
    pub you_decide: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Who {
    pub seat: u8,
    pub name: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Supply {
    /// Treasures, Victory cards and Curses in the usual order, then the other non-Kingdom
    /// Supply piles (Ruins, ...).
    pub basic: Vec<PileEntry>,
    /// Kingdom piles in rows of at most five, the most expensive row first, each row by
    /// cost then name.
    pub kingdom: Vec<Vec<PileEntry>>,
}

/// Where a pile is in the view: names aren't unique (the Black Market deck is named
/// after its Kingdom card).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PileAt {
    Supply(usize),
    NonSupply(usize),
}

impl PileAt {
    pub fn pile(self, view: &AgentView) -> Option<&PileView> {
        match self {
            PileAt::Supply(i) => view.supply.get(i),
            PileAt::NonSupply(i) => view.non_supply.get(i),
        }
    }

    /// Every pile of the view.
    pub fn all(view: &AgentView) -> impl Iterator<Item = (PileAt, &PileView)> {
        let supply = view
            .supply
            .iter()
            .enumerate()
            .map(|(i, p)| (PileAt::Supply(i), p));
        let non = view
            .non_supply
            .iter()
            .enumerate()
            .map(|(i, p)| (PileAt::NonSupply(i), p));
        supply.chain(non)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct PileEntry {
    pub at: PileAt,
    pub name: String,
    /// The card to draw: the top card, else the pile's own.
    pub card: String,
    pub count: u32,
    pub cost: Option<Cost>,
    /// The printed cost of the pile's own card, which places it among the piles
    /// whatever its top card costs now.
    pub printed: (u32, u32, u32),
    pub empty: bool,
    pub kingdom: bool,
    pub hidden: bool,
    pub types: Vec<String>,
    pub badges: Vec<Badge>,
    /// What dominion.games prints on the pile's card (Bane, Split, ...), in its order.
    pub markers: Vec<PileMarker>,
    /// The Trait on it, drawn across its top.
    pub attached: Option<String>,
    /// Holding different cards one can look through: a back shows under it.
    pub shadowed: bool,
}

/// A token, Trait or counter shown on a pile, player or landscape.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Badge {
    /// A stable key for its icon: `coffers`, `embargo`, `trait`, `+card`, ...
    pub kind: String,
    /// What it says in words.
    pub text: String,
    pub count: Option<i32>,
    /// The token's owner, for player tokens on piles.
    pub player: Option<u8>,
    /// A Trait's card, a token's pile.
    pub card: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LandscapeEntry {
    pub card: String,
    pub kind: String,
    pub cost: Option<Cost>,
    pub types: Vec<String>,
    pub badges: Vec<Badge>,
    pub owners: Vec<Who>,
    /// The pile it names (a Trait's, Obelisk's), whose card is drawn on it.
    pub pile: Option<Box<PileEntry>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stack {
    pub card: String,
    pub count: u32,
}

/// A zone as far as the seat knows it: its size, the cards known in it, and how many
/// are not.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Zone {
    pub count: u32,
    pub known: Vec<Stack>,
    pub unknown: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NamedZone {
    /// The view's key: `setAside`, `looking`, `tavern`, `island`, `nativeVillage`, `exile`.
    pub key: String,
    pub label: String,
    pub zone: Zone,
    /// Its cards in the order they lie, when the seat sees them all there.
    pub order: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PlayerEntry {
    pub seat: u8,
    pub name: String,
    pub vp: Option<i32>,
    pub turns: Option<u32>,
    pub is_turn: bool,
    pub deciding: bool,
    pub hand: Zone,
    pub in_play: Zone,
    pub deck: Zone,
    pub deck_order_known: bool,
    pub discard: Zone,
    /// The face-up card on top of the discard pile.
    pub discard_top: Option<String>,
    /// Cards in play in the order they came into play, stacked by [`runs`].
    pub play_order: Vec<Stack>,
    /// Mats and set-aside cards.
    pub zones: Vec<NamedZone>,
    pub badges: Vec<Badge>,
    pub projects: Vec<String>,
    pub states: Vec<String>,
    pub boons: Vec<String>,
    pub artifacts: Vec<String>,
    /// What dominion.games reminds the player of beside their name, by card with how
    /// many each has, in card order: lasting effects, kept Boons, States and Artifacts.
    pub reminders: Vec<(String, u32)>,
    /// Every card the player owns, as far as known.
    pub owned: Zone,
    pub unlocated: Vec<Stack>,
    pub uncertain: Vec<Uncertain>,
    pub counters: Vec<(String, i64)>,
    /// The account playing the seat ([`Board::with_meta`]).
    pub account: Option<SeatAccount>,
    /// The model attached to the player's seat, which everyone at the table is told of.
    pub model: Option<dominion_protocol::lobby::SeatModel>,
    /// In a dominion.games game: the player's level there ([`Board::with_meta`]).
    pub level: Option<f64>,
    /// The player's rating in full, where the game says it ([`Board::with_meta`]).
    pub rating: Option<SeatRating>,
}

/// The tokens dominion.games draws on a player's deck rather than their bar.
pub const ON_DECK: [&str; 2] = ["-card", "-coin"];

impl PlayerEntry {
    /// The tokens drawn on the player's deck.
    pub fn deck_tokens(&self) -> Vec<Badge> {
        (self.badges.iter())
            .filter(|b| ON_DECK.contains(&b.kind.as_str()))
            .cloned()
            .collect()
    }

    /// The tokens drawn on the player's bar.
    pub fn bar_tokens(&self) -> Vec<Badge> {
        (self.badges.iter())
            .filter(|b| !ON_DECK.contains(&b.kind.as_str()))
            .cloned()
            .collect()
    }

    /// The zones the player's cards are in, by the view's key and a label: the draw
    /// pile, discard pile, hand, cards in play, and each mat or set-aside zone holding
    /// cards.
    fn parts(&self) -> impl Iterator<Item = (&str, &str, &Zone)> {
        [
            ("deck", "Draw pile", &self.deck),
            ("discard", "Discard pile", &self.discard),
            ("hand", "Hand", &self.hand),
            ("inPlay", "In play", &self.in_play),
        ]
        .into_iter()
        .chain(
            self.zones
                .iter()
                .filter(|z| z.zone.count > 0)
                .map(|z| (z.key.as_str(), z.label.as_str(), &z.zone)),
        )
    }

    /// The deck breakdown from the viewer's knowledge, without assigning uncertain
    /// cards to a particular location: every card owned, then each zone of them.
    pub fn composition(&self) -> Vec<NamedZone> {
        [("owned", "Entire deck", &self.owned)]
            .into_iter()
            .chain(self.parts())
            .map(|(key, label, zone)| NamedZone {
                key: key.into(),
                label: label.into(),
                zone: zone.clone(),
                order: vec![],
            })
            .collect()
    }

    /// The zones holding cards the viewer can't name, as one: the cards known to be
    /// owned and seen nowhere else are among them. `None` unless that says more than
    /// the zones do apart.
    pub fn rest(&self) -> Option<NamedZone> {
        if self.parts().map(|(_, _, z)| z.count).sum::<u32>() != self.owned.count {
            return None;
        }
        let mut left: BTreeMap<&str, u32> = BTreeMap::new();
        for s in &self.owned.known {
            *left.entry(&s.card).or_default() += s.count;
        }
        for s in self.parts().flat_map(|(_, _, z)| &z.known) {
            let n = left.get_mut(s.card.as_str())?;
            *n = n.checked_sub(s.count)?;
        }
        left.retain(|_, n| *n > 0);
        let hidden: Vec<_> = self.parts().filter(|(_, _, z)| z.unknown > 0).collect();
        if hidden.len() < 2 || left.is_empty() {
            return None;
        }
        for s in hidden.iter().flat_map(|(_, _, z)| &z.known) {
            *left.entry(&s.card).or_default() += s.count;
        }
        let labels: Vec<&str> = hidden.iter().map(|&(_, label, _)| label).collect();
        Some(NamedZone {
            key: "rest".into(),
            label: labels.join(" + "),
            zone: Zone {
                count: hidden.iter().map(|(_, _, z)| z.count).sum(),
                known: left
                    .into_iter()
                    .map(|(card, count)| Stack {
                        card: card.into(),
                        count,
                    })
                    .collect(),
                unknown: self.owned.unknown,
            },
            order: vec![],
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlayArea {
    pub player: Option<Who>,
    pub cards: Zone,
    /// The cards in the order they came into play, stacked by [`runs`].
    pub order: Vec<Stack>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CardDeck {
    pub count: u32,
    /// Its discard pile, the last one on top.
    pub discard: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SetupSetAside {
    pub source: String,
    pub card: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Holding {
    pub card: String,
    pub holder: Option<Who>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Lasting {
    pub card: String,
    pub owner: Who,
    /// When it ends, in words.
    pub until: String,
    pub cards: Vec<String>,
    pub count: Option<i32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExtraTurn {
    pub player: Who,
    pub source: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Note {
    pub source: String,
    pub player: Option<Who>,
    pub pile: Option<String>,
    pub card: Option<String>,
    pub count: i32,
    /// The note in words.
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Uncertain {
    pub count: u32,
    pub candidates: Vec<Stack>,
    /// Further candidates the viewer can't name.
    pub unknown: u32,
    /// Where they are, in words.
    pub text: String,
}

const BASIC_ORDER: [&str; 10] = [
    "Copper", "Silver", "Gold", "Platinum", "Potion", "Estate", "Duchy", "Province", "Colony",
    "Curse",
];

pub fn cost_key(c: Option<&Cost>) -> (u32, u32, u32) {
    c.map_or((0, 0, 0), |c| (c.coins, c.potions, c.debt))
}

/// The `+N Things` a card's text gives, read from its first lines.
fn plus(text: &str, thing: &str) -> u32 {
    text.lines()
        .take_while(|l| l.trim_start().starts_with('+'))
        .find_map(|l| {
            let l = l.trim_start().trim_start_matches('+');
            let (n, rest) = l.split_once(' ')?;
            rest.starts_with(thing).then(|| n.parse().ok()).flatten()
        })
        .unwrap_or(0)
}

/// Where each card it has comes in dominion.games' list of cards (client 2.3.7), by name.
static CARD_ORDER: LazyLock<HashMap<&'static str, u16>> = LazyLock::new(|| {
    include_str!("card_order.tsv")
        .lines()
        .filter_map(|l| {
            let (name, at) = l.split_once('\t')?;
            Some((name, at.parse().ok()?))
        })
        .collect()
});

/// Where `card` comes in dominion.games' list of cards, which orders its card ids:
/// cards it lacks come last.
pub fn id_order(card: &str) -> u16 {
    CARD_ORDER.get(card).copied().unwrap_or(u16::MAX)
}

/// dominion.games' sorting group of each card it has (client 2.3.7), by name.
static SORT_GROUPS: LazyLock<HashMap<&'static str, u8>> = LazyLock::new(|| {
    include_str!("sort_groups.tsv")
        .lines()
        .filter_map(|l| {
            let (name, group) = l.split_once('\t')?;
            Some((name, group.parse().ok()?))
        })
        .collect()
});

/// Where a card goes in a hand, as dominion.games sorts it: cards that play others,
/// villages, draw, cantrips, other Actions, plain Treasures, other Treasures, Night
/// cards, Victory cards, then Curses and the rest. Its own group for a card it has,
/// else one judged from the card's text.
pub fn sort_group(cards: &CardDb, card: &str) -> u8 {
    if let Some(&group) = SORT_GROUPS.get(card) {
        return group;
    }
    let Some(c) = cards.get(card) else { return 9 };
    let is = |t: &str| c.types.iter().any(|x| x == t);
    let text = c.text.split("\n---").next().unwrap_or("");
    if is("Action") {
        let lower = text.to_lowercase();
        if lower.contains("play")
            && ["twice", "three times", "again"]
                .iter()
                .any(|w| lower.contains(w))
        {
            0
        } else if plus(text, "Action") >= 2 {
            1
        } else if plus(text, "Card") >= 2 {
            2
        } else if plus(text, "Action") == 1 {
            3
        } else {
            4
        }
    } else if is("Treasure") {
        if c.basic { 5 } else { 6 }
    } else if is("Night") {
        7
    } else if is("Victory") {
        8
    } else {
        9
    }
}

/// The order cards are stacked in: by [`sort_group`], then printed cost, then name.
pub fn card_order(cards: &CardDb, a: &str, b: &str) -> Ordering {
    let key = |n: &str| {
        let cost = cards.get(n).and_then(|c| c.cost).unwrap_or_default();
        (sort_group(cards, n), cost.coins, cost.potions, cost.debt)
    };
    key(a).cmp(&key(b)).then_with(|| a.cmp(b))
}

/// Every card in any of `owned` (a player's cards by name each), in [`card_order`], with
/// how many of it each has.
pub fn side_by_side(owned: &[Vec<(&str, u32)>], cards: &CardDb) -> Vec<(String, Vec<u32>)> {
    let mut rows: BTreeMap<&str, Vec<u32>> = BTreeMap::new();
    for (i, p) in owned.iter().enumerate() {
        for &(card, n) in p {
            rows.entry(card).or_insert_with(|| vec![0; owned.len()])[i] += n;
        }
    }
    let mut out: Vec<_> = rows.into_iter().map(|(c, n)| (c.to_string(), n)).collect();
    out.sort_by(|a, b| card_order(cards, &a.0, &b.0));
    out
}

/// A column the card counts can be sorted by.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CountsKey {
    /// A player's counts, by their place among the columns.
    Player(usize),
    /// The first player's count less the second's.
    Delta,
}

/// Orders rows of `owned_side_by_side` by a column, largest first unless `ascending`;
/// rows that tie keep their order.
pub fn sort_counts(rows: &mut [(String, Vec<u32>)], key: CountsKey, ascending: bool) {
    let at = |n: &[u32], i: usize| i64::from(n.get(i).copied().unwrap_or(0));
    let value = |n: &[u32]| match key {
        CountsKey::Player(i) => at(n, i),
        CountsKey::Delta => at(n, 0) - at(n, 1),
    };
    rows.sort_by(|a, b| {
        let o = value(&a.1).cmp(&value(&b.1));
        if ascending { o } else { o.reverse() }
    });
}

pub fn stacks(known: &CardCount, cards: &CardDb) -> Vec<Stack> {
    let mut out: Vec<Stack> = known
        .iter()
        .filter(|&(_, &n)| n > 0)
        .map(|(c, &n)| Stack {
            card: c.clone(),
            count: n,
        })
        .collect();
    out.sort_by(|a, b| card_order(cards, &a.card, &b.card));
    out
}

fn zone(z: &ZoneView, cards: &CardDb) -> Zone {
    Zone {
        count: z.count,
        known: stacks(&z.known, cards),
        unknown: z.unknown,
    }
}

fn deck(z: &DeckView, cards: &CardDb) -> Zone {
    Zone {
        count: z.count,
        known: stacks(&z.known, cards),
        unknown: z.unknown,
    }
}

fn badge(kind: &str, text: String, count: Option<i32>) -> Badge {
    Badge {
        kind: kind.into(),
        text,
        count,
        player: None,
        card: None,
    }
}

fn zone_label(key: &str) -> String {
    match key {
        "setAside" => "Set aside".into(),
        "looking" => "Looking at".into(),
        "tavern" => "Tavern mat".into(),
        "island" => "Island mat".into(),
        "nativeVillage" => "Native Village mat".into(),
        "exile" => "Exile".into(),
        "deck" => "deck".into(),
        "hand" => "hand".into(),
        "discard" => "discard pile".into(),
        "inPlay" => "play".into(),
        "trash" => "trash".into(),
        "pile" => "a pile".into(),
        other => other.into(),
    }
}

struct Names<'a>(&'a AgentView);

impl Names<'_> {
    fn name(&self, seat: u8) -> String {
        self.0
            .you
            .iter()
            .chain(&self.0.opponents)
            .find(|p| p.index == seat)
            .map(|p| p.name.clone())
            .or_else(|| {
                let r = self.0.game.result.as_ref()?;
                r.players
                    .iter()
                    .find(|p| p.index == seat)
                    .map(|p| p.name.clone())
            })
            .unwrap_or_else(|| format!("P{}", seat + 1))
    }

    fn who(&self, seat: u8) -> Who {
        Who {
            seat,
            name: self.name(seat),
        }
    }
}

fn pile_entry(at: PileAt, p: &PileView, names: &Names<'_>, cards: &CardDb) -> PileEntry {
    let t = &p.tokens;
    let mut badges = vec![];
    if t.embargo > 0 {
        badges.push(badge(
            "embargo",
            format!("{} Embargo token(s)", t.embargo),
            Some(t.embargo as i32),
        ));
    }
    if t.blockade > 0 {
        badges.push(badge(
            "blockade",
            format!("{} Blockade token(s)", t.blockade),
            Some(t.blockade as i32),
        ));
    }
    if t.trade_route {
        badges.push(badge("trade-route", "Trade Route token".into(), None));
    }
    if t.vp != 0 {
        badges.push(badge("vp", format!("{} VP token(s)", t.vp), Some(t.vp)));
    }
    if t.debt != 0 {
        badges.push(badge(
            "debt",
            format!("{} Debt token(s)", t.debt),
            Some(t.debt),
        ));
    }
    for pt in &t.players {
        badges.push(Badge {
            kind: pt.token.clone(),
            text: format!("{}'s {} token", names.name(pt.player), pt.token),
            count: None,
            player: Some(pt.player),
            card: None,
        });
    }
    PileEntry {
        at,
        name: p.card.clone(),
        card: p.top.clone().unwrap_or_else(|| p.card.clone()),
        count: p.count,
        cost: p.cost.clone(),
        printed: match cards.get(&p.card).and_then(|c| c.cost) {
            Some(c) => (c.coins, c.potions, c.debt),
            None => cost_key(p.cost.as_ref()),
        },
        empty: p.count == 0,
        kingdom: p.kingdom,
        hidden: p.hidden,
        types: p.types.clone(),
        badges,
        markers: p
            .structure
            .as_ref()
            .and_then(|s| s.marker())
            .into_iter()
            .collect(),
        attached: p.traits.first().cloned(),
        shadowed: p.structure.as_ref().is_some_and(|s| s.shadowed(p.count)),
    }
}

fn player_badges(p: &PlayerView) -> Vec<Badge> {
    let t = &p.tokens;
    let mut out = vec![];
    let mut n = |kind: &str, label: &str, v: i64| {
        if v != 0 {
            out.push(badge(kind, format!("{v} {label}"), Some(v as i32)));
        }
    };
    // dominion.games' order on a player's bar.
    n("vp", "VP token(s)", t.vp_tokens.into());
    n("debt", "Debt", t.debt.into());
    n("pirate-ship", "Pirate Ship token(s)", t.pirate_ship.into());
    n("villagers", "Villager(s)", t.villagers.into());
    n("coffers", "Coffers", t.coffers.into());
    n("favors", "Favor(s)", t.favors.into());
    for (on, kind, text) in [
        (t.minus_card, "-card", "-1 Card token"),
        (t.minus_coin, "-coin", "-$1 token"),
    ] {
        if on {
            out.push(Badge {
                player: Some(p.index),
                ..badge(kind, text.into(), None)
            });
        }
    }
    if let Some(up) = t.journey_face_up {
        let (kind, side) = if up {
            ("journey", "up")
        } else {
            ("journey-down", "down")
        };
        out.push(badge(kind, format!("Journey token face {side}"), None));
    }
    // Adventures tokens are drawn on their piles only, as dominion.games draws them.
    out
}

fn counters(c: &std::collections::BTreeMap<String, i64>) -> Vec<(String, i64)> {
    c.iter().map(|(k, &v)| (k.clone(), v)).collect()
}

/// Cards in play stacked as dominion.games stacks them: a card joins the stack before it
/// when it has that card's name and, like it, stays in play or doesn't; one with cards
/// set aside or tokens of its own is always drawn alone.
pub fn runs(cards: &[InPlay]) -> Vec<Stack> {
    let mut out: Vec<Stack> = vec![];
    let mut last: Option<&InPlay> = None;
    for c in cards {
        match (out.last_mut(), last) {
            (Some(s), Some(l))
                if l.card == c.card && l.stays == c.stays && !l.holds && !c.holds =>
            {
                s.count += 1
            }
            _ => out.push(Stack {
                card: c.card.clone(),
                count: 1,
            }),
        }
        last = Some(c);
    }
    out
}

/// A player's reminders by card, with how many each has, in card order: their lasting
/// effects, the Boons they keep (one reminder with the effect a Boon may have made), their
/// States and the Artifacts they hold.
fn reminders(view: &AgentView, p: &PlayerView, cards: &CardDb) -> Vec<(String, u32)> {
    let mut out: Vec<(String, u32)> = vec![];
    for l in view.lasting.iter().filter(|l| l.owner == p.index) {
        match out.iter_mut().find(|(c, _)| *c == l.card) {
            Some((_, n)) => *n += 1,
            None => out.push((l.card.clone(), 1)),
        }
    }
    for c in p.boons.iter().chain(&p.states).chain(&p.artifacts) {
        if !out.iter().any(|(o, _)| o == c) {
            out.push((c.clone(), 1));
        }
    }
    out.sort_by_key(|(c, _)| cards.order(c));
    out
}

fn player_entry(
    p: &PlayerView,
    view: &AgentView,
    deciding: Option<u8>,
    extras: &TableExtras,
    cards: &CardDb,
) -> PlayerEntry {
    let extra = extras.players.iter().find(|t| t.seat == p.index);
    let in_play = zone(&p.in_play, cards);
    let play_order = match extra {
        Some(t) if t.in_play.len() as u32 == p.in_play.count => runs(&t.in_play),
        _ => in_play.known.clone(),
    };
    let discard_top = extra.and_then(|t| t.discard_top.clone()).or_else(|| {
        match (p.discard.known.len(), p.discard.unknown) {
            (1, 0) => p.discard.known.keys().next().cloned(),
            _ => None,
        }
    });
    PlayerEntry {
        seat: p.index,
        name: p.name.clone(),
        vp: p.vp,
        turns: p.turns_taken,
        is_turn: view.turn.player == Some(p.index),
        deciding: deciding == Some(p.index),
        hand: zone(&p.hand, cards),
        in_play,
        play_order,
        discard_top,
        deck: deck(&p.deck, cards),
        deck_order_known: p.deck.order_known,
        discard: zone(&p.discard, cards),
        zones: p
            .other_zones
            .iter()
            .map(|(k, z)| NamedZone {
                key: k.clone(),
                label: zone_label(k),
                zone: zone(z, cards),
                order: z.order.clone(),
            })
            .collect(),
        badges: player_badges(p),
        projects: p.projects.clone(),
        states: p.states.clone(),
        boons: p.boons.clone(),
        artifacts: p.artifacts.clone(),
        reminders: reminders(view, p, cards),
        owned: Zone {
            count: p.owned.known.values().sum::<u32>() + p.owned.unknown,
            known: stacks(&p.owned.known, cards),
            unknown: p.owned.unknown,
        },
        unlocated: stacks(&p.unlocated, cards),
        uncertain: p
            .uncertain
            .iter()
            .map(|u| {
                let zones: Vec<String> = u.zones.iter().map(|z| zone_label(z)).collect();
                Uncertain {
                    count: u.count,
                    candidates: stacks(&u.candidates, cards),
                    unknown: u.unknown,
                    text: format!(
                        "{} {} in {}",
                        u.count,
                        if u.count == 1 { "card" } else { "cards" },
                        zones.join(" or ")
                    ),
                }
            })
            .collect(),
        counters: counters(&p.counters),
        account: None,
        model: None,
        level: None,
        rating: None,
    }
}

fn kingdom_rows(mut piles: Vec<PileEntry>) -> Vec<Vec<PileEntry>> {
    piles.sort_by(|a, b| {
        cost_key(a.cost.as_ref())
            .cmp(&cost_key(b.cost.as_ref()))
            .then_with(|| a.name.cmp(&b.name))
    });
    let mut rows = vec![];
    while !piles.is_empty() {
        let at = piles.len().saturating_sub(5);
        rows.push(piles.split_off(at));
    }
    rows
}

/// Puts a note where it belongs: tokens it counts on the pile or landscape they sit on
/// (Family of Inventors' Favors, Sinister Plot's), or the pile it names (the Bane,
/// Obelisk's pile). Every note is also listed with the other effects.
fn note_badge(
    n: &dominion_protocol::NoteView,
    names: &Names<'_>,
    basic: &mut [PileEntry],
    kingdom: &mut [PileEntry],
    non_supply: &mut [PileEntry],
    landscapes: &mut [LandscapeEntry],
) {
    let Some((on, b)) = note_tokens(n, names).or_else(|| note_card_badge(n)) else {
        return;
    };
    if let Some(p) = basic
        .iter_mut()
        .chain(kingdom.iter_mut())
        .chain(non_supply.iter_mut())
        .find(|p| p.name == on)
    {
        let marker = match (n.setup, n.source.as_str()) {
            (Some(SetupRole::Bane), _) => Some(PileMarker::Bane),
            (_, "Obelisk") if n.pile.is_some() => Some(PileMarker::Obelisk),
            _ => None,
        };
        match marker {
            // The marker says it, and Obelisk's card shows the pile.
            Some(m) => {
                if !p.markers.contains(&m) {
                    p.markers.push(m);
                    p.markers.sort();
                }
                if m == PileMarker::Obelisk
                    && let Some(l) = landscapes.iter_mut().find(|l| l.card == n.source)
                {
                    l.pile = Some(Box::new(p.clone()));
                }
            }
            None => p.badges.push(b),
        }
    } else if let Some(l) = landscapes.iter_mut().find(|l| l.card == on) {
        l.badges.push(b);
    }
}

/// A note that counts tokens on a pile or landscape, as those tokens.
fn note_tokens<'a>(
    n: &'a dominion_protocol::NoteView,
    names: &Names<'_>,
) -> Option<(&'a str, Badge)> {
    match (n.source.as_str(), &n.pile, n.player) {
        ("Family of Inventors", Some(pile), None) => Some((
            pile.as_str(),
            badge(
                "favors",
                format!("{} Favor token(s) from Family of Inventors", n.count),
                Some(n.count),
            ),
        )),
        ("Sinister Plot", None, Some(p)) => Some((
            n.source.as_str(),
            Badge {
                kind: "sinister-plot".into(),
                text: format!(
                    "{}'s Sinister Plot: {} {}",
                    names.name(p),
                    n.count,
                    if n.count == 1 { "token" } else { "tokens" }
                ),
                count: Some(n.count),
                player: Some(p),
                card: None,
            },
        )),
        _ => None,
    }
}

/// The public set-asides shown as cards beside the kingdom.
pub fn setup_set_asides(view: &AgentView) -> impl Iterator<Item = (&str, &str)> {
    view.notes
        .iter()
        .filter(|n| n.setup == Some(SetupRole::SetAside) && n.player.is_none())
        .filter_map(|n| Some((n.source.as_str(), n.card.as_deref()?)))
}

/// A note shown as the card it names, and where it goes.
fn note_card_badge(n: &dominion_protocol::NoteView) -> Option<(&str, Badge)> {
    if n.player.is_some()
        || n.pile.as_ref() == Some(&n.source)
        || n.setup == Some(SetupRole::SetAside)
    {
        return None;
    }
    let (on, kind, card) = match (&n.pile, &n.card) {
        (Some(pile), None) => (
            pile.as_str(),
            if n.setup == Some(SetupRole::Bane) {
                "bane"
            } else {
                "note"
            },
            n.source.clone(),
        ),
        (None, Some(card)) => (n.source.as_str(), "note", card.clone()),
        _ => return None,
    };
    let b = Badge {
        kind: kind.into(),
        text: match &n.pile {
            Some(_) => format!("{} ({})", n.source, n.count),
            None => format!("{}: {card}", n.source),
        },
        count: (n.count != 0).then_some(n.count),
        player: None,
        card: Some(card),
    };
    Some((on, b))
}

fn until_text(l: &dominion_protocol::LastingView, names: &Names<'_>) -> String {
    match (l.until.as_str(), l.until_player) {
        ("end-of-turn", _) => "this turn".into(),
        ("start-of-turn", Some(p)) if p == l.owner => "until their next turn".into(),
        ("start-of-turn", Some(p)) => format!("until {}'s next turn", names.name(p)),
        ("removed", _) => "while it lasts".into(),
        (other, _) => other.into(),
    }
}

pub fn board(
    view: &AgentView,
    deciding: Option<u8>,
    extras: &TableExtras,
    cards: &CardDb,
) -> Board {
    let names = Names(view);
    let t = &view.turn;
    let running = view.game.running;
    let status = Status {
        running,
        phase: t.phase.clone(),
        actions: t.actions,
        buys: t.buys,
        coins: t.coins,
        potions: t.potions,
        debt: t.debt,
        turn: t.number,
        player: t.player.map(|p| names.who(p)),
        your_turn: t.your_turn,
        extra: t.extra.clone(),
        deciding: deciding.filter(|_| running).map(|p| names.who(p)),
        you_decide: running && !view.spectating && deciding.is_some() && deciding == view.seat,
    };

    let mut basic: Vec<PileEntry> = vec![];
    let mut others = vec![];
    let mut kingdom = vec![];
    for (i, p) in view.supply.iter().enumerate() {
        let e = pile_entry(PileAt::Supply(i), p, &names, cards);
        if p.kingdom {
            kingdom.push(e);
        } else if BASIC_ORDER.contains(&p.card.as_str()) {
            basic.push(e);
        } else {
            others.push(e);
        }
    }
    basic.sort_by_key(|e| BASIC_ORDER.iter().position(|&n| n == e.name));
    basic.extend(others);

    let landscapes: Vec<LandscapeEntry> = view
        .landscapes
        .iter()
        .map(|l| {
            let mut badges = vec![];
            // dominion.games draws a landscape's VP only while it has some.
            if let Some(vp) = l.vp.filter(|&vp| vp != 0) {
                badges.push(badge("vp", format!("{vp} VP"), Some(vp)));
            }
            // A Prophecy in effect has no Sun tokens left, and dominion.games draws none.
            if let Some(s) = l.suns.filter(|&s| s > 0) {
                badges.push(badge("sun", format!("{s} Sun(s)"), Some(s as i32)));
            }
            LandscapeEntry {
                card: l.card.clone(),
                kind: l.kind.clone(),
                cost: l.cost.clone(),
                types: l.types.clone(),
                badges,
                owners: l.owners.iter().map(|&p| names.who(p)).collect(),
                pile: None,
            }
        })
        .collect();

    let you = view
        .you
        .as_ref()
        .map(|p| player_entry(p, view, deciding, extras, cards));
    let opponents: Vec<PlayerEntry> = view
        .opponents
        .iter()
        .map(|p| player_entry(p, view, deciding, extras, cards))
        .collect();
    let play_area = {
        let player = t.player;
        let them = you
            .iter()
            .chain(&opponents)
            .find(|p| Some(p.seat) == player);
        PlayArea {
            player: player.map(|p| names.who(p)),
            cards: them.map(|p| p.in_play.clone()).unwrap_or_default(),
            order: them.map(|p| p.play_order.clone()).unwrap_or_default(),
        }
    };
    let card_deck = |d: &dominion_protocol::CardDeckView| CardDeck {
        count: d.count,
        discard: d.discard.clone(),
    };
    let mut non_supply: Vec<PileEntry> = view
        .non_supply
        .iter()
        .enumerate()
        .map(|(i, p)| pile_entry(PileAt::NonSupply(i), p, &names, cards))
        .collect();
    let mut landscapes: Vec<LandscapeEntry> = landscapes;
    for p in basic.iter().chain(&kingdom).chain(&non_supply) {
        if let Some(l) =
            (p.attached.as_ref()).and_then(|t| landscapes.iter_mut().find(|l| &l.card == t))
        {
            l.pile = Some(Box::new(p.clone()));
        }
    }
    for n in &view.notes {
        note_badge(
            n,
            &names,
            &mut basic,
            &mut kingdom,
            &mut non_supply,
            &mut landscapes,
        );
    }
    Board {
        status,
        supply: Supply {
            basic,
            kingdom: kingdom_rows(kingdom),
        },
        non_supply,
        landscapes,
        you,
        opponents,
        watching: view.seat.is_none(),
        play_area,
        trash: zone(&view.trash, cards),
        boons: view.boons.as_ref().map(card_deck),
        hexes: view.hexes.as_ref().map(card_deck),
        set_asides: setup_set_asides(view)
            .map(|(source, card)| SetupSetAside {
                source: source.into(),
                card: card.into(),
            })
            .collect(),
        artifacts: view
            .artifacts
            .iter()
            .map(|a| Holding {
                card: a.card.clone(),
                holder: a.holder.map(|p| names.who(p)),
            })
            .collect(),
        lasting: view
            .lasting
            .iter()
            .map(|l| Lasting {
                card: l.card.clone(),
                owner: names.who(l.owner),
                until: until_text(l, &names),
                cards: l.cards.clone(),
                count: l.count,
            })
            .collect(),
        extra_turns: view
            .extra_turns
            .iter()
            .map(|e| ExtraTurn {
                player: names.who(e.player),
                source: e.source.clone(),
            })
            .collect(),
        notes: view
            .notes
            .iter()
            .map(|n| {
                let mut text = format!("{}: {}", n.source, n.count);
                for (what, v) in [
                    ("player", n.player.map(|p| names.name(p))),
                    ("pile", n.pile.clone()),
                    ("card", n.card.clone()),
                ] {
                    if let Some(v) = v {
                        text.push_str(&format!(", {what} {v}"));
                    }
                }
                Note {
                    source: n.source.clone(),
                    player: n.player.map(|p| names.who(p)),
                    pile: n.pile.clone(),
                    card: n.card.clone(),
                    count: n.count,
                    text,
                }
            })
            .collect(),
        counters: counters(&view.counters),
        uncertain: view
            .uncertain
            .iter()
            .map(|u| {
                let zones: Vec<String> = u
                    .zones
                    .iter()
                    .map(|z| {
                        let place = match (&z.pile, z.player) {
                            (Some(pile), _) => format!("the {pile} pile"),
                            (None, Some(p)) => {
                                format!("{}'s {}", names.name(p), zone_label(&z.zone))
                            }
                            (None, None) => format!("the {}", zone_label(&z.zone)),
                        };
                        format!("{} in {place}", z.count)
                    })
                    .collect();
                Uncertain {
                    count: u.count,
                    candidates: stacks(&u.candidates, cards),
                    unknown: u.unknown,
                    text: zones.join(", "),
                }
            })
            .collect(),
        result: view.game.result.as_ref().map(|r| r.players.clone()),
        end_reason: view
            .game
            .result
            .as_ref()
            .and_then(|r| meta::end_reason(r.reason.as_deref(), &r.players, None)),
        ended: None,
        rated: false,
        voided: false,
        timed: false,
        watchers: 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn in_play(card: &str, stays: bool, holds: bool) -> InPlay {
        InPlay {
            card: card.into(),
            stays,
            holds,
        }
    }

    #[test]
    fn counts_sort_by_a_column_keeping_ties_in_order() {
        let rows = |r: &[(&str, [u32; 2])]| -> Vec<(String, Vec<u32>)> {
            r.iter().map(|(c, n)| (c.to_string(), n.to_vec())).collect()
        };
        let names = |r: &[(String, Vec<u32>)]| r.iter().map(|(c, _)| c.clone()).collect::<Vec<_>>();
        let start = rows(&[
            ("Witch", [1, 0]),
            ("Copper", [7, 7]),
            ("Silver", [3, 2]),
            ("Estate", [3, 3]),
        ]);
        let mut r = start.clone();
        sort_counts(&mut r, CountsKey::Player(0), false);
        assert_eq!(names(&r), ["Copper", "Silver", "Estate", "Witch"]);
        let mut r = start.clone();
        sort_counts(&mut r, CountsKey::Delta, false);
        assert_eq!(names(&r), ["Witch", "Silver", "Copper", "Estate"]);
        let mut r = start;
        sort_counts(&mut r, CountsKey::Player(1), true);
        assert_eq!(names(&r), ["Witch", "Silver", "Estate", "Copper"]);
    }

    #[test]
    fn cards_come_in_dominion_games_order() {
        let lines = include_str!("card_order.tsv").lines().count();
        assert_eq!(CARD_ORDER.len(), lines);
        assert!(id_order("Copper") < id_order("Caravan"));
        assert!(id_order("Caravan") < id_order("Fishing Village"));
        assert!(id_order("Seize the Day") < id_order("No such card"));
    }

    #[test]
    fn cards_sort_into_dominion_games_groups() {
        let lines = include_str!("sort_groups.tsv").lines().count();
        assert_eq!(SORT_GROUPS.len(), lines);
        let db = CardDb::default();
        let groups = ["Throne Room", "Hamlet", "Wharf", "Royal Seal", "Province"]
            .map(|card| sort_group(&db, card));
        assert_eq!(groups, [0, 1, 2, 5, 8]);
    }

    #[test]
    fn cards_in_play_stack_only_with_a_like_card_before_them() {
        let cards = [
            in_play("Throne Room", false, false),
            in_play("Throne Room", false, false),
            in_play("Wharf", true, false),
            in_play("Wharf", true, false),
            in_play("Wharf", false, false),
            in_play("Haven", true, true),
            in_play("Haven", true, true),
            in_play("Copper", false, false),
            in_play("Wharf", false, false),
        ];
        let stacked: Vec<_> = runs(&cards)
            .into_iter()
            .map(|s| (s.card, s.count))
            .collect();
        let expected = [
            ("Throne Room", 2),
            ("Wharf", 2),
            ("Wharf", 1),
            ("Haven", 1),
            ("Haven", 1),
            ("Copper", 1),
            ("Wharf", 1),
        ];
        assert_eq!(
            stacked,
            expected.map(|(card, count)| (card.to_string(), count))
        );
    }
}
