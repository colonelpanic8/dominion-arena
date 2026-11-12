//! A view arranged for drawing: piles in their places, players with their tokens as
//! badges, cards stacked in a stable order, and every other part of the view in a form
//! ready to show, so that nothing a view holds is lost on the table.

use crate::cards::CardDb;
use dominion_protocol::api::TableExtras;
use dominion_protocol::{
    AgentView, CardCount, Cost, DeckView, PileView, PlayerView, ResultPlayer, ZoneView,
};
use std::cmp::Ordering;

#[derive(Clone, Debug, PartialEq)]
pub struct Board {
    pub status: Status,
    pub supply: Supply,
    pub non_supply: Vec<PileEntry>,
    pub landscapes: Vec<LandscapeEntry>,
    /// The seat's own player, if it has one.
    pub you: Option<PlayerEntry>,
    /// The other players, in turn order from the seat.
    pub opponents: Vec<PlayerEntry>,
    pub play_area: PlayArea,
    pub trash: Zone,
    pub boons: Option<CardDeck>,
    pub hexes: Option<CardDeck>,
    pub artifacts: Vec<Holding>,
    pub lasting: Vec<Lasting>,
    pub extra_turns: Vec<ExtraTurn>,
    pub notes: Vec<Note>,
    pub counters: Vec<(String, i64)>,
    /// Cards whose whereabouts are known only to be among several players' zones.
    pub uncertain: Vec<Uncertain>,
    pub result: Option<Vec<ResultPlayer>>,
    pub end_reason: Option<String>,
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
    /// Who plays this turn for its player (Possession).
    pub controller: Option<Who>,
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
    pub empty: bool,
    pub kingdom: bool,
    pub hidden: bool,
    pub types: Vec<String>,
    pub badges: Vec<Badge>,
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
    /// The view's key: `setAside`, `tavern`, `island`, `nativeVillage`, `exile`.
    pub key: String,
    pub label: String,
    pub zone: Zone,
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
    /// Cards in play in the order they came into play, runs of one card stacked.
    pub play_order: Vec<Stack>,
    /// Mats and set-aside cards.
    pub zones: Vec<NamedZone>,
    pub badges: Vec<Badge>,
    pub projects: Vec<String>,
    pub states: Vec<String>,
    pub boons: Vec<String>,
    pub artifacts: Vec<String>,
    /// Every card the player owns, as far as known.
    pub owned: Zone,
    pub unlocated: Vec<Stack>,
    pub uncertain: Vec<Uncertain>,
    pub counters: Vec<(String, i64)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlayArea {
    pub player: Option<Who>,
    pub cards: Zone,
    /// The cards in the order they came into play, runs of one card stacked.
    pub order: Vec<Stack>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CardDeck {
    pub count: u32,
    /// Its discard pile, the last one on top.
    pub discard: Vec<String>,
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

/// Where a card goes in a hand, as dominion.games sorts it: cards that play others,
/// villages, draw, cantrips, other Actions, plain Treasures, other Treasures, Night
/// cards, Victory cards, then Curses and the rest.
pub fn sort_group(cards: &CardDb, card: &str) -> u8 {
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

fn pile_entry(at: PileAt, p: &PileView, names: &Names<'_>) -> PileEntry {
    let t = &p.tokens;
    let mut badges = vec![];
    if t.embargo > 0 {
        badges.push(badge(
            "embargo",
            format!("{} Embargo token(s)", t.embargo),
            Some(t.embargo as i32),
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
    for tr in &p.traits {
        badges.push(Badge {
            kind: "trait".into(),
            text: format!("Trait: {tr}"),
            count: None,
            player: None,
            card: Some(tr.clone()),
        });
    }
    PileEntry {
        at,
        name: p.card.clone(),
        card: p.top.clone().unwrap_or_else(|| p.card.clone()),
        count: p.count,
        cost: p.cost.clone(),
        empty: p.count == 0,
        kingdom: p.kingdom,
        hidden: p.hidden,
        types: p.types.clone(),
        badges,
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
    n("coffers", "Coffers", t.coffers.into());
    n("villagers", "Villager(s)", t.villagers.into());
    n("favors", "Favor(s)", t.favors.into());
    n("debt", "Debt", t.debt.into());
    n("vp", "VP token(s)", t.vp_tokens.into());
    n("pirate-ship", "Pirate Ship token(s)", t.pirate_ship.into());
    if t.minus_card {
        out.push(badge("-card", "-1 Card token".into(), None));
    }
    if t.minus_coin {
        out.push(badge("-coin", "-$1 token".into(), None));
    }
    if t.journey_face_up {
        out.push(badge("journey", "Journey token face up".into(), None));
    }
    for (token, pile) in &p.adventure_tokens {
        out.push(Badge {
            kind: token.clone(),
            text: match pile {
                Some(pile) => format!("{token} token on {pile}"),
                None => format!("{token} token"),
            },
            count: None,
            player: Some(p.index),
            card: pile.clone(),
        });
    }
    out
}

fn counters(c: &std::collections::BTreeMap<String, i64>) -> Vec<(String, i64)> {
    c.iter().map(|(k, &v)| (k.clone(), v)).collect()
}

/// Runs of one card in a row, stacked.
pub fn runs(names: &[String]) -> Vec<Stack> {
    let mut out: Vec<Stack> = vec![];
    for n in names {
        match out.last_mut() {
            Some(s) if &s.card == n => s.count += 1,
            _ => out.push(Stack {
                card: n.clone(),
                count: 1,
            }),
        }
    }
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
            })
            .collect(),
        badges: player_badges(p),
        projects: p.projects.clone(),
        states: p.states.clone(),
        boons: p.boons.clone(),
        artifacts: p.artifacts.clone(),
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
                    text: format!("{} card(s) in {}", u.count, zones.join(" or ")),
                }
            })
            .collect(),
        counters: counters(&p.counters),
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

/// Puts a note where it belongs: on the pile it names (the Bane, Obelisk's pile), or a
/// card it names on the pile or landscape it comes from (Druid's Boons, Way of the
/// Mouse's card). Every note is also listed with the other effects.
fn note_badge(
    n: &dominion_protocol::NoteView,
    basic: &mut [PileEntry],
    kingdom: &mut [PileEntry],
    non_supply: &mut [PileEntry],
    landscapes: &mut [LandscapeEntry],
) {
    if n.player.is_some() || n.pile.as_ref() == Some(&n.source) {
        return;
    }
    let (on, kind, card) = match (&n.pile, &n.card) {
        (Some(pile), None) => (
            pile.as_str(),
            if n.source == "Young Witch" {
                "bane"
            } else {
                "note"
            },
            n.source.clone(),
        ),
        (None, Some(card)) => (n.source.as_str(), "note", card.clone()),
        _ => return,
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
    if let Some(p) = basic
        .iter_mut()
        .chain(kingdom.iter_mut())
        .chain(non_supply.iter_mut())
        .find(|p| p.name == on)
    {
        p.badges.push(b);
    } else if let Some(l) = landscapes.iter_mut().find(|l| l.card == on) {
        l.badges.push(b);
    }
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
        controller: t.controller.map(|p| names.who(p)),
        deciding: deciding.filter(|_| running).map(|p| names.who(p)),
        you_decide: running && deciding.is_some() && deciding == view.seat,
    };

    let mut basic: Vec<PileEntry> = vec![];
    let mut others = vec![];
    let mut kingdom = vec![];
    for (i, p) in view.supply.iter().enumerate() {
        let e = pile_entry(PileAt::Supply(i), p, &names);
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
            if let Some(vp) = l.vp {
                badges.push(badge("vp", format!("{vp} VP"), Some(vp)));
            }
            if let Some(s) = l.suns {
                badges.push(badge("sun", format!("{s} Sun(s)"), Some(s as i32)));
            }
            if l.active == Some(true) {
                badges.push(badge("active", "In effect".into(), None));
            }
            LandscapeEntry {
                card: l.card.clone(),
                kind: l.kind.clone(),
                cost: l.cost.clone(),
                types: l.types.clone(),
                badges,
                owners: l.owners.iter().map(|&p| names.who(p)).collect(),
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
        .map(|(i, p)| pile_entry(PileAt::NonSupply(i), p, &names))
        .collect();
    let mut landscapes: Vec<LandscapeEntry> = landscapes;
    for n in &view.notes {
        note_badge(
            n,
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
        play_area,
        trash: zone(&view.trash, cards),
        boons: view.boons.as_ref().map(card_deck),
        hexes: view.hexes.as_ref().map(card_deck),
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
                    text: zones.join(", "),
                }
            })
            .collect(),
        result: view.game.result.as_ref().map(|r| r.players.clone()),
        end_reason: view.game.result.as_ref().and_then(|r| r.reason.clone()),
    }
}
