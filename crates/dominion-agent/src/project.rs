//! Projection of an engine game, as one seat knows it, onto agent-view@2, and of answers
//! back onto engine responses.

use crate::types::{self, *};
use dominion_engine::{
    Card, CardId, Decision, DecisionKind, EndReason, Event, Game, Kind, KnownCards, Mat, Move, Opt,
    Phase, PileId, PlayerId, Purpose, Response, Token, Until, View, Zone,
};
use std::collections::{BTreeMap, BTreeSet};

/// Kebab-case of a CamelCase name.
pub(crate) fn kebab(name: &str) -> String {
    let mut out = String::new();
    for (i, ch) in name.chars().enumerate() {
        if ch.is_ascii_uppercase() {
            if i > 0 {
                out.push('-');
            }
            out.push(ch.to_ascii_lowercase());
        } else {
            out.push(ch);
        }
    }
    out
}

pub(crate) fn purpose_name(p: Purpose) -> String {
    match p {
        // agent-view@2 has no purpose of its own for it.
        Purpose::Way => "mode".into(),
        p => kebab(&format!("{p:?}")),
    }
}

pub(crate) fn phase_name(p: Phase) -> String {
    kebab(&format!("{p:?}"))
}

fn mat_name(mat: Mat) -> &'static str {
    match mat {
        Mat::Tavern => "tavern",
        Mat::Island => "island",
        Mat::NativeVillage => "nativeVillage",
        Mat::Exile => "exile",
    }
}

/// A zone's name as seen from its owner: `hand`, `deck`, `tavern`, ...
pub(crate) fn zone_name(zone: Zone) -> String {
    match zone {
        Zone::Deck(_) => "deck".into(),
        Zone::Hand(_) => "hand".into(),
        Zone::Discard(_) => "discard".into(),
        Zone::InPlay(_) => "inPlay".into(),
        Zone::SetAside(_) => "setAside".into(),
        Zone::Mat(_, mat) => mat_name(mat).into(),
        Zone::Pile(_) => "pile".into(),
        Zone::Trash => "trash".into(),
    }
}

fn token_name(t: Token) -> &'static str {
    match t {
        Token::PlusCard => "+card",
        Token::PlusAction => "+action",
        Token::PlusBuy => "+buy",
        Token::PlusCoin => "+coin",
        Token::MinusCost => "-cost",
        Token::Trashing => "trashing",
        Token::Estate => "estate",
    }
}

pub(crate) fn cost(c: dominion_engine::Cost) -> types::Cost {
    types::Cost {
        coins: c.coins.into(),
        potions: c.potions.into(),
        debt: c.debt.into(),
    }
}

pub(crate) fn cost_text(c: &types::Cost) -> String {
    dominion_engine::Cost {
        coins: c.coins as u8,
        potions: c.potions as u8,
        debt: c.debt as u8,
    }
    .to_string()
}

fn names(cards: &[Card]) -> Vec<String> {
    cards.iter().map(|c| c.name().to_string()).collect()
}

fn count(known: &dominion_engine::CardCount) -> CardCount {
    known
        .iter()
        .map(|(c, &n)| (c.name().to_string(), n))
        .collect()
}

fn zone_view(k: &KnownCards, zone: Zone) -> ZoneView {
    let z = k.zone(zone);
    ZoneView {
        count: z.count as u32,
        known: count(&z.known),
        unknown: z.unknown as u32,
    }
}

/// Seat names and ids a table supplies.
pub struct Context<'a> {
    pub game_id: Option<&'a str>,
    pub names: &'a [String],
    pub decision_id: Option<&'a str>,
    /// Log entries from this index on are shown as `sinceLastDecision`.
    pub log_from: usize,
}

impl Context<'_> {
    fn name(&self, p: PlayerId) -> String {
        self.names
            .get(p.index())
            .cloned()
            .unwrap_or_else(|| p.to_string())
    }
}

pub fn view(game: &Game, known: &KnownCards, seat: PlayerId, cx: &Context<'_>) -> AgentView {
    let v = game.view(seat);
    let n = game.num_players();
    let player = |p: PlayerId| player_view(&v, known, p, cx);
    let opponents = (1..n)
        .map(|i| player(PlayerId(((seat.index() + i) % n) as u8)))
        .collect();
    let notes = v.notes();
    let pile_card = |pile: Option<PileId>| pile.map(|p| v.pile(p).card().name().to_string());
    let mut supply = vec![];
    let mut non_supply = vec![];
    for pile in v.piles().filter(|p| !p.removed()) {
        let top = pile.top().and_then(|c| v.card(c));
        let shown = top.unwrap_or(pile.card());
        let types: Vec<String> = match pile.top() {
            Some(c) => v.types(c).map(|t| t.names().map(String::from).collect()),
            None if pile.hidden() => Some(vec![]),
            None => None,
        }
        .unwrap_or_else(|| shown.type_names().into_iter().map(String::from).collect());
        let pile_cost = match pile.top() {
            Some(c) => v.cost_of(c),
            None if pile.hidden() => None,
            None => shown.cost().map(|_| v.cost(shown)),
        };
        let pv = PileView {
            card: pile.card().name().to_string(),
            count: pile.size() as u32,
            top: top
                .filter(|&t| t != pile.card())
                .map(|t| t.name().to_string()),
            cost: pile_cost.map(cost),
            types,
            kingdom: pile.kingdom(),
            hidden: pile.hidden(),
            tokens: PileTokens {
                embargo: pile.embargo(),
                trade_route: pile.trade_route(),
                vp: pile.vp_tokens(),
                debt: pile.debt_tokens(),
                players: pile
                    .tokens()
                    .iter()
                    .map(|&(p, t)| PlayerToken {
                        player: p.0,
                        token: token_name(t).into(),
                    })
                    .collect(),
            },
            traits: names(pile.traits()),
        };
        if pile.in_supply() {
            supply.push(pv);
        } else {
            non_supply.push(pv);
        }
    }
    let prophecy = v.prophecy();
    let landscapes = v
        .landscapes()
        .iter()
        .map(|&card| {
            let suns = prophecy.filter(|(p, _)| *p == card).map(|(_, n)| n);
            let vp = notes
                .iter()
                .find(|n| n.source == card && n.player.is_none() && n.pile.is_none())
                .map(|n| n.count)
                .filter(|_| card.kind() == Kind::Landmark);
            LandscapeView {
                card: card.name().to_string(),
                kind: match card.kind() {
                    k @ (Kind::Event
                    | Kind::Landmark
                    | Kind::Project
                    | Kind::Way
                    | Kind::Ally
                    | Kind::Trait
                    | Kind::Prophecy
                    | Kind::Study) => kebab(&format!("{k:?}")),
                    _ => "other".into(),
                },
                cost: card.cost().map(|_| cost(v.cost(card))),
                types: card.type_names().into_iter().map(String::from).collect(),
                vp,
                suns,
                active: suns.map(|n| n == 0),
                owners: v
                    .players()
                    .filter(|p| p.projects().contains(&card))
                    .map(|p| p.id().0)
                    .collect(),
            }
        })
        .collect();
    let card_deck = |d: dominion_engine::CardDeckView<'_>| {
        (d.draw_size > 0 || !d.discard.is_empty()).then(|| CardDeckView {
            count: d.draw_size as u32,
            discard: names(d.discard),
        })
    };
    let lasting = v
        .lasting()
        .into_iter()
        .map(|l| {
            let mut cards: Vec<CardId> = l.keeps.clone();
            cards.extend(l.data.cards.iter().copied());
            cards.extend(l.data.pick);
            cards.sort();
            cards.dedup();
            let (until, until_player) = match l.until {
                _ if l.ending => ("end-of-turn", None),
                Until::EndOfTurn => ("end-of-turn", None),
                Until::StartOfTurn(p) => ("start-of-turn", Some(p.0)),
                Until::Removed => ("removed", None),
            };
            LastingView {
                card: l.card.name().to_string(),
                owner: l.owner.0,
                until: until.into(),
                until_player,
                cards: cards
                    .into_iter()
                    .filter_map(|c| v.card(c))
                    .map(|c| c.name().to_string())
                    .collect(),
                count: (l.data.n != 0).then_some(l.data.n),
            }
        })
        .collect();
    let mut uncertain = vec![];
    for u in &known.uncertain {
        let owner = u.zones[0].0.owner();
        if owner.is_some() && u.zones.iter().all(|(z, _)| z.owner() == owner) {
            continue;
        }
        uncertain.push(SharedUncertain {
            zones: u
                .zones
                .iter()
                .map(|&(z, n)| ZoneRef {
                    player: z.owner().map(|p| p.0),
                    pile: match z {
                        Zone::Pile(p) => pile_card(Some(p)),
                        _ => None,
                    },
                    zone: zone_name(z),
                    count: n as u32,
                })
                .collect(),
            count: u.zones.iter().map(|(_, n)| *n as u32).sum(),
            candidates: count(&u.candidates),
        });
    }
    AgentView {
        schema: SCHEMA.into(),
        game: game_info(game, cx),
        seat: Some(seat.0),
        spectating: false,
        turn: turn_view(game, seat, cx),
        you: Some(player(seat)),
        opponents,
        supply,
        non_supply,
        landscapes,
        trash: zone_view(known, Zone::Trash),
        uncertain,
        boons: card_deck(v.boons()),
        hexes: card_deck(v.hexes()),
        artifacts: v
            .artifacts()
            .iter()
            .map(|&(card, holder)| ArtifactView {
                card: card.name().to_string(),
                holder: holder.map(|p| p.0),
            })
            .collect(),
        lasting,
        extra_turns: v
            .extra_turns()
            .iter()
            .map(|e| ExtraTurnView {
                player: e.player.0,
                source: e.source.name().to_string(),
            })
            .collect(),
        notes: notes
            .iter()
            .map(|n| NoteView {
                source: n.source.name().to_string(),
                player: n.player.map(|p| p.0),
                pile: pile_card(n.pile),
                card: n.card.map(|c| c.name().to_string()),
                count: n.count,
            })
            .collect(),
        counters: BTreeMap::new(),
        decision: v
            .pending()
            .map(|d| decision(game, known, d, cx.decision_id.unwrap_or("?")).0),
        since_last_decision: log(game, cx),
    }
}

fn game_info(game: &Game, cx: &Context<'_>) -> GameInfo {
    let result = game.outcome().map(|o| {
        let n = o.scores.len();
        let key = |p: usize| (std::cmp::Reverse(o.scores[p]), o.turns[p]);
        GameResult {
            reason: Some(
                match o.reason {
                    EndReason::ProvincesGone => "provinces-gone",
                    EndReason::ColoniesGone => "colonies-gone",
                    EndReason::PilesEmpty => "piles-empty",
                    EndReason::TurnLimit => "turn-limit",
                }
                .into(),
            ),
            players: (0..n)
                .map(|p| ResultPlayer {
                    index: p as u8,
                    name: cx.name(PlayerId(p as u8)),
                    vp: o.scores[p],
                    rank: 1 + (0..n).filter(|&q| key(q) < key(p)).count() as u32,
                    turns: o.turns[p],
                    resigned: false,
                })
                .collect(),
        }
    });
    GameInfo {
        id: cx.game_id.map(String::from),
        running: game.outcome().is_none(),
        result,
        kingdom: game
            .piles()
            .iter()
            .filter(|p| p.is_kingdom())
            .map(|p| p.card.name().to_string())
            .collect(),
        landscapes: names(game.landscapes()),
    }
}

fn turn_view(game: &Game, seat: PlayerId, cx: &Context<'_>) -> TurnView {
    if game.outcome().is_some() {
        return TurnView::default();
    }
    let t = game.turn();
    let v = game.view(seat);
    TurnView {
        number: Some(v.player(t.player).turns_taken()),
        player: Some(t.player.0),
        active_player: Some(cx.name(t.player)),
        your_turn: t.player == seat,
        phase: Some(phase_name(t.phase)),
        extra: t.extra.map(|c| c.name().to_string()),
        actions: Some(t.actions),
        buys: Some(t.buys),
        coins: Some(t.coins),
        potions: Some(t.potions),
        debt: Some(v.player(t.player).debt()).filter(|&d| d > 0),
    }
}

fn player_view(v: &View<'_>, known: &KnownCards, p: PlayerId, cx: &Context<'_>) -> PlayerView {
    let pv = v.player(p);
    let uses_journey = [Card::Ranger, Card::Giant, Card::Pilgrimage]
        .iter()
        .any(|&c| v.supply_pile(c).is_some() || v.landscapes().contains(&c));
    let mut other_zones = BTreeMap::new();
    let mut zones: Vec<Zone> = vec![Zone::SetAside(p)];
    zones.extend(
        [Mat::Tavern, Mat::Island, Mat::NativeVillage, Mat::Exile].map(|m| Zone::Mat(p, m)),
    );
    for zone in zones {
        let z = zone_view(known, zone);
        if z.count > 0 {
            other_zones.insert(zone_name(zone), z);
        }
    }
    let deck = zone_view(known, Zone::Deck(p));
    let (owned_known, owned_unknown) = &known.owned[p.index()];
    let uncertain = known
        .uncertain
        .iter()
        .filter(|u| u.zones.iter().all(|(z, _)| z.owner() == Some(p)))
        .map(|u| Uncertain {
            zones: u.zones.iter().map(|&(z, _)| zone_name(z)).collect(),
            count: u.zones.iter().map(|(_, n)| *n as u32).sum(),
            candidates: count(&u.candidates),
            outside: 0,
        })
        .collect();
    let mut counters = BTreeMap::new();
    for (name, value) in [
        ("extraTurnsTaken", pv.extra_turns_taken()),
        ("skipTurns", pv.skip_turns()),
        ("turnsSkipped", pv.turns_skipped()),
    ] {
        if value > 0 {
            counters.insert(name.to_string(), value.into());
        }
    }
    PlayerView {
        index: p.0,
        name: cx.name(p),
        vp: Some(pv.score()),
        hand: zone_view(known, Zone::Hand(p)),
        in_play: zone_view(known, Zone::InPlay(p)),
        deck: DeckView {
            count: deck.count,
            known: deck.known,
            unknown: deck.unknown,
            order_known: false,
        },
        discard: zone_view(known, Zone::Discard(p)),
        other_zones,
        owned: Owned {
            known: count(owned_known),
            unknown: *owned_unknown as u32,
        },
        uncertain,
        unlocated: CardCount::new(),
        tokens: Tokens {
            coffers: pv.coffers(),
            villagers: pv.villagers(),
            favors: pv.favors(),
            debt: pv.debt(),
            vp_tokens: pv.vp_tokens(),
            pirate_ship: pv.pirate_ship(),
            minus_card: pv.minus_card(),
            minus_coin: pv.minus_coin(),
            journey_face_up: uses_journey.then(|| pv.journey_face_up()),
        },
        adventure_tokens: pv
            .tokens()
            .iter()
            .map(|(&t, &pile)| {
                (
                    token_name(t).to_string(),
                    pile.map(|x| v.pile(x).card().name().to_string()),
                )
            })
            .collect(),
        projects: names(pv.projects()),
        states: names(pv.states()),
        boons: names(pv.boons_kept()),
        artifacts: v
            .artifacts()
            .iter()
            .filter(|(_, h)| *h == Some(p))
            .map(|(a, _)| a.name().to_string())
            .collect(),
        turns_taken: Some(pv.turns_taken()),
        counters,
    }
}

/// What an option of a decision answers with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Target {
    Move(Move),
    Card(CardId),
    Pile(Option<PileId>),
    Opt(Opt),
    Name(Card),
    Number(i32),
}

fn option(index: usize, action: &str, label: String) -> AgentOption {
    AgentOption {
        index: index as u32,
        group: 0,
        action: action.into(),
        label,
        card: None,
        zone: None,
        cost: None,
        value: None,
        player: None,
    }
}

/// What choosing cards or a pile for `p` does to them, if the purpose says.
fn verb(p: Purpose) -> Option<&'static str> {
    Some(match p {
        Purpose::Discard => "discard",
        Purpose::Trash => "trash",
        Purpose::Gain => "gain",
        Purpose::TopDeck => "put onto your deck",
        Purpose::Move => "move",
        Purpose::Play => "play",
        Purpose::SetAside => "set aside",
        Purpose::Reveal => "reveal",
        Purpose::Reaction => "react with",
        Purpose::Exile => "exile",
        Purpose::Return => "return",
        Purpose::Pass => "pass",
        Purpose::Trigger
        | Purpose::Mode
        | Purpose::Way
        | Purpose::Either
        | Purpose::Overpay
        | Purpose::Name
        | Purpose::Turn
        | Purpose::Other => return None,
    })
}

fn bounds(min: usize, max: usize) -> String {
    if min == max {
        format!("{min}")
    } else {
        format!("{min}-{max}")
    }
}

fn lower_first(s: &str) -> String {
    let mut chars = s.chars();
    chars
        .next()
        .map(|c| c.to_lowercase().chain(chars).collect())
        .unwrap_or_default()
}

/// "a card", "2 cards", "up to 3 cards", "1 to 3 cards".
fn how_many(min: usize, max: usize, one: &str, many: &str) -> String {
    let noun = |n: usize| if n == 1 { one } else { many };
    match (min, max) {
        (1, 1) if one.starts_with(['a', 'e', 'i', 'o', 'u']) => format!("an {one}"),
        (1, 1) => format!("a {one}"),
        (a, b) if a == b => format!("{a} {}", noun(a)),
        (0, b) => format!("up to {b} {}", noun(b)),
        (a, b) => format!("{a} to {b} {many}"),
    }
}

/// The cards an agent may expect to name: those of the game's visible piles and every
/// card the seat knows of.
fn nameable(game: &Game, known: &KnownCards, seat: PlayerId) -> Vec<Card> {
    let v = game.view(seat);
    let mut cards = BTreeSet::new();
    for pile in v.piles().filter(|p| !p.removed() && !p.hidden()) {
        cards.extend(pile.card().pile_members());
    }
    for z in known.zones.values() {
        cards.extend(z.known.keys().copied());
    }
    cards.retain(|c: &Card| c.kind() == Kind::Card);
    let mut cards: Vec<Card> = cards.into_iter().collect();
    cards.sort_by_key(|c| c.name());
    cards
}

/// The decision as agent-view@2 options, with what each option answers with.
pub(crate) fn decision(
    game: &Game,
    known: &KnownCards,
    d: &Decision,
    id: &str,
) -> (AgentDecision, Vec<Target>) {
    let v = game.view(d.player);
    let source = d.source.map(|c| c.name().to_string());
    let about: Vec<&str> = d
        .about
        .iter()
        .filter_map(|&c| v.card(c))
        .map(|c| c.name())
        .collect();
    let prefix = match (d.source, about.is_empty()) {
        (Some(c), true) => format!("{c}: "),
        (Some(c), false) => format!("{c} (looking at {}): ", about.join(", ")),
        (None, false) => format!("Looking at {}: ", about.join(", ")),
        (None, true) => String::new(),
    };
    let mut items: Vec<(AgentOption, Target)> = vec![];
    let card_option = |i: usize, c: CardId, action: &str| {
        let name = v
            .card(c)
            .map_or("a card".to_string(), |c| c.name().to_string());
        let mut o = option(i, action, name.clone());
        o.card = v.card(c).map(|c| c.name().to_string());
        o.zone = v.zone(c).map(zone_name);
        o
    };
    let (kind, prompt, min, max, ordered, phase) = match &d.kind {
        DecisionKind::Turn { phase, moves } => {
            let phase_label = format!("{phase:?}");
            for &m in moves {
                let i = items.len();
                let o = match m {
                    Move::Play(c) => {
                        let mut o = card_option(i, c, "play");
                        o.label = format!("Play {}", o.label);
                        o
                    }
                    Move::Buy(pile) => {
                        let top = v.pile(pile).top();
                        let card = top.and_then(|c| v.card(c)).unwrap_or(v.pile(pile).card());
                        let c = top
                            .and_then(|t| v.cost_of(t))
                            .unwrap_or_else(|| v.cost(card));
                        let mut o = option(i, "buy", format!("Buy {card} ({c})"));
                        o.card = Some(card.name().to_string());
                        o.cost = Some(cost(c));
                        o
                    }
                    Move::BuyLandscape(card) => {
                        let c = v.cost(card);
                        let kind = format!("{:?}", card.kind());
                        let mut o = option(i, "buy-landscape", format!("Buy {kind} {card} ({c})"));
                        o.card = Some(card.name().to_string());
                        o.cost = Some(cost(c));
                        o
                    }
                    Move::Coffers => option(i, "spend-coffers", "Spend a Coffers (+$1)".into()),
                    Move::Villager => {
                        option(i, "spend-villager", "Spend a Villager (+1 Action)".into())
                    }
                    Move::PayDebt => option(i, "pay-debt", "Pay off 1 Debt with $1".into()),
                    Move::EndPhase => {
                        option(i, "end-phase", format!("End your {} phase", phase_label))
                    }
                };
                items.push((o, Target::Move(m)));
            }
            let prompt = format!("{phase_label} phase: choose a move");
            ("turn", prompt, 1, 1, false, Some(phase_name(*phase)))
        }
        DecisionKind::Cards {
            candidates,
            min,
            max,
            ordered,
        } => {
            for &c in candidates {
                let i = items.len();
                items.push((card_option(i, c, "select"), Target::Card(c)));
            }
            let order = if *ordered {
                ", in order (first listed ends on top)"
            } else {
                ""
            };
            let what = how_many(*min, *max, "card", "cards");
            let ask = match (&d.question, verb(d.purpose)) {
                (Some(q), _) => format!("choose {what} to {q}"),
                (None, Some(verb)) => format!("choose {what} to {verb}"),
                (None, None) => format!("choose {what}"),
            };
            let prompt = format!("{prefix}{ask}{order}");
            ("cards", prompt, *min, *max, *ordered, None)
        }
        DecisionKind::Pile { piles, optional } => {
            for &pile in piles {
                let i = items.len();
                let pv = v.pile(pile);
                let card = pv.top().and_then(|c| v.card(c)).unwrap_or(pv.card());
                let c = pv.top().and_then(|t| v.cost_of(t)).unwrap_or(v.cost(card));
                let mut o = option(i, "pile", format!("{card} ({c}, {} left)", pv.size()));
                o.card = Some(card.name().to_string());
                o.cost = Some(cost(c));
                items.push((o, Target::Pile(Some(pile))));
            }
            if *optional {
                let i = items.len();
                items.push((option(i, "decline", "None".into()), Target::Pile(None)));
            }
            let prompt = match (&d.question, verb(d.purpose)) {
                (Some(q), _) => format!("{prefix}{q}"),
                (None, Some(verb)) => format!("{prefix}choose a pile to {verb} from"),
                (None, None) => format!("{prefix}choose a pile"),
            };
            ("pile", prompt, 1, 1, false, None)
        }
        DecisionKind::Choose {
            options: opts,
            labels,
            min,
            max,
        } => {
            let triggers = v.pending_triggers();
            for (j, &opt) in opts.iter().enumerate() {
                let i = items.len();
                let label = labels.get(j).cloned().unwrap_or_default();
                let mut o = match opt {
                    Opt::Yes => option(i, "yes", label),
                    Opt::No => option(i, "no", label),
                    Opt::Mode(m) => {
                        let mut o = option(i, "mode", label);
                        o.value = Some(m.into());
                        o
                    }
                    Opt::Card(card) => {
                        let mut o = option(i, "select", label);
                        o.card = Some(card.name().to_string());
                        o
                    }
                    Opt::Player(p) => {
                        let mut o = option(i, "player", label);
                        o.player = Some(p.0);
                        o
                    }
                    Opt::Trigger(t) => {
                        let mut o = option(i, "trigger", label);
                        o.card = triggers
                            .get(t as usize)
                            .map(|tr| tr.card.name().to_string());
                        o
                    }
                };
                if o.label.is_empty() {
                    o.label = o.card.clone().unwrap_or_else(|| format!("{opt:?}"));
                }
                items.push((o, Target::Opt(opt)));
            }
            let prompt = match (&d.question, opts.as_slice()) {
                (Some(q), _) => format!("{prefix}{q}"),
                (None, [Opt::Yes, Opt::No]) => {
                    format!("{prefix}{}?", lower_first(&items[0].0.label))
                }
                (None, _) => format!(
                    "{prefix}choose {}",
                    how_many(*min, *max, "option", "options")
                ),
            };
            ("choose", prompt, *min, *max, false, None)
        }
        DecisionKind::NameCard => {
            for card in nameable(game, known, d.player) {
                let i = items.len();
                let mut o = option(i, "name", card.name().to_string());
                o.card = Some(card.name().to_string());
                items.push((o, Target::Name(card)));
            }
            let prompt = format!("{prefix}name a card (any card may be named)");
            ("name", prompt, 1, 1, false, None)
        }
        DecisionKind::Number { min, max } => {
            for n in *min..=*max {
                let i = items.len();
                let mut o = option(i, "number", n.to_string());
                o.value = Some(n);
                items.push((o, Target::Number(n)));
            }
            let prompt = match (&d.question, d.purpose) {
                (Some(q), _) => format!("{prefix}{q} ({min} to {max})"),
                (None, Purpose::Overpay) => {
                    format!("{prefix}choose how much to overpay (${min} to ${max})")
                }
                (None, _) => format!("{prefix}choose a number from {min} to {max}"),
            };
            ("number", prompt, 1, 1, false, None)
        }
    };
    let (options, targets): (Vec<AgentOption>, Vec<Target>) = items.into_iter().unzip();
    let decision = AgentDecision {
        id: id.to_string(),
        player: Some(d.player.0),
        prompt: prompt.clone(),
        kind: kind.into(),
        purpose: purpose_name(d.purpose),
        source,
        phase,
        ordered,
        name_any: matches!(d.kind, DecisionKind::NameCard),
        min: Some(min as u32),
        max: Some(max as u32),
        groups: vec![DecisionGroup {
            group: 0,
            kind: kind.into(),
            prompt,
            min: min as u32,
            max: max as u32,
        }],
        options,
    };
    (decision, targets)
}

/// Turns an answer's indices or name into the engine response, checking what the
/// engine's own validation can't explain in agent terms.
pub(crate) fn response(
    d: &Decision,
    agent: &AgentDecision,
    targets: &[Target],
    answer: &Answer,
) -> Result<Response, String> {
    if let Some(name) = &answer.name {
        if answer.indices.is_some() {
            return Err("give either indices or name, not both".into());
        }
        if !agent.name_any {
            return Err(format!(
                "decision {} doesn't take a name; answer with option indices",
                agent.id
            ));
        }
        let card = Card::from_name(name)
            .filter(|c| c.kind() == Kind::Card)
            .ok_or_else(|| format!("{name:?} is not the name of a card"))?;
        return Ok(Response::Name(card));
    }
    let indices = answer
        .indices
        .as_ref()
        .ok_or("an answer needs indices (or name, where allowed)")?;
    let mut seen = BTreeSet::new();
    let mut chosen = vec![];
    for &i in indices {
        let t = targets.get(i as usize).ok_or_else(|| {
            format!(
                "option {i} does not exist (decision has {} options)",
                targets.len()
            )
        })?;
        if !seen.insert(i) {
            return Err(format!("option {i} is chosen more than once"));
        }
        chosen.push(*t);
    }
    let (min, max) = (agent.min.unwrap_or(1), agent.max.unwrap_or(1));
    if !(min..=max).contains(&(chosen.len() as u32)) {
        return Err(format!(
            "choose {} option(s), got {}",
            bounds(min as usize, max as usize),
            chosen.len()
        ));
    }
    let one = || chosen.first().copied();
    Ok(match &d.kind {
        DecisionKind::Turn { .. } => match one() {
            Some(Target::Move(m)) => Response::Move(m),
            _ => unreachable!("turn options are moves"),
        },
        DecisionKind::Cards { .. } => Response::Cards(
            chosen
                .iter()
                .map(|t| match t {
                    Target::Card(c) => *c,
                    _ => unreachable!("card options are cards"),
                })
                .collect(),
        ),
        DecisionKind::Pile { .. } => match one() {
            Some(Target::Pile(p)) => Response::Pile(p),
            _ => unreachable!("pile options are piles"),
        },
        DecisionKind::Choose { .. } => Response::Options(
            chosen
                .iter()
                .map(|t| match t {
                    Target::Opt(o) => *o,
                    _ => unreachable!("choose options are options"),
                })
                .collect(),
        ),
        DecisionKind::NameCard => match one() {
            Some(Target::Name(c)) => Response::Name(c),
            _ => unreachable!("name options are names"),
        },
        DecisionKind::Number { .. } => match one() {
            Some(Target::Number(n)) => Response::Number(n),
            _ => unreachable!("number options are numbers"),
        },
    })
}

/// `n` of a thing, in words: "a card", "3 cards".
fn several(n: usize, one: &str, many: &str) -> String {
    match n {
        1 => format!("a {one}"),
        n => format!("{n} {many}"),
    }
}

fn amount(n: i32, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// A token's printed name.
fn token_text(t: Token) -> &'static str {
    match t {
        Token::PlusCard => "+1 Card",
        Token::PlusAction => "+1 Action",
        Token::PlusBuy => "+1 Buy",
        Token::PlusCoin => "+$1",
        Token::MinusCost => "-$2 cost",
        Token::Trashing => "Trashing",
        Token::Estate => "Estate",
    }
}

/// Where `zone` is, from `player`'s side of the table.
fn place(game: &Game, zone: Zone, player: PlayerId, cx: &Context<'_>) -> String {
    let whose = |p: PlayerId| {
        if p == player {
            "their".to_string()
        } else {
            format!("{}'s", cx.name(p))
        }
    };
    match zone {
        Zone::Hand(p) => format!("{} hand", whose(p)),
        Zone::Deck(p) => format!("{} deck", whose(p)),
        Zone::Discard(p) => format!("{} discard pile", whose(p)),
        Zone::InPlay(p) if p == player => "play".to_string(),
        Zone::InPlay(p) => format!("{} play area", whose(p)),
        Zone::SetAside(p) if p == player => "set aside".to_string(),
        Zone::SetAside(p) => format!("{} set-aside cards", whose(p)),
        Zone::Mat(p, mat) => {
            let mat = match mat {
                Mat::Tavern => "Tavern",
                Mat::Island => "Island",
                Mat::NativeVillage => "Native Village",
                Mat::Exile => "Exile",
            };
            format!("{} {mat} mat", whose(p))
        }
        Zone::Pile(id) => format!("the {} pile", game.pile(id).card),
        Zone::Trash => "the trash".to_string(),
    }
}

/// "puts Copper from their hand onto their deck" and the like.
fn put_text(
    game: &Game,
    player: PlayerId,
    (from, to, bottom): (Option<Zone>, Zone, bool),
    cards: &[Card],
    count: usize,
    cx: &Context<'_>,
) -> String {
    let mut what: Vec<String> = cards.iter().map(|c| c.to_string()).collect();
    let hidden = count.saturating_sub(cards.len());
    if hidden > 0 {
        what.push(if cards.is_empty() {
            several(hidden, "card", "cards")
        } else {
            several(hidden, "other card", "other cards")
        });
    }
    let what = what.join(", ");
    let mut to_text = place(game, to, player, cx);
    if bottom {
        to_text = format!("the bottom of {to_text}");
    }
    if from == Some(to) && matches!(to, Zone::Deck(_)) {
        return format!("puts {what} back onto {to_text}");
    }
    let from = match from {
        Some(Zone::SetAside(p)) if p == player => " they had set aside".to_string(),
        Some(Zone::Pile(_)) if matches!(to, Zone::Pile(_)) => String::new(),
        Some(z) => format!(" from {}", place(game, z, player, cx)),
        None => String::new(),
    };
    match to {
        Zone::SetAside(p) if p == player => format!("sets aside {what}{from}"),
        Zone::Pile(_) => format!("returns {what}{from} to {to_text}"),
        Zone::Deck(_) => format!("puts {what}{from} onto {to_text}"),
        Zone::Mat(..) => format!("puts {what}{from} on {to_text}"),
        _ => format!("puts {what}{from} into {to_text}"),
    }
}

fn counter_text(
    game: &Game,
    (player, pile): (Option<PlayerId>, Option<PileId>),
    counter: dominion_engine::Counter,
    delta: i32,
    cx: &Context<'_>,
) -> String {
    use dominion_engine::Counter as C;
    let n = delta.abs();
    let up = delta > 0;
    if let Some(p) = player {
        let (verb, what) = match counter {
            C::Coffers => (if up { "takes" } else { "spends" }, format!("{n} Coffers")),
            C::Villagers => (
                if up { "takes" } else { "spends" },
                amount(n, "Villager", "Villagers"),
            ),
            C::Favors => (
                if up { "takes" } else { "spends" },
                amount(n, "Favor", "Favors"),
            ),
            C::Debt => (if up { "takes" } else { "pays off" }, format!("{n} Debt")),
            C::VpTokens => (
                if up { "takes" } else { "loses" },
                amount(n, "VP token", "VP tokens"),
            ),
            C::PirateShip => {
                let what = amount(n, "coin token", "coin tokens");
                return if up {
                    format!("{} puts {what} on their Pirate Ship mat", cx.name(p))
                } else {
                    format!("{} removes {what} from their Pirate Ship mat", cx.name(p))
                };
            }
            _ => ("gets", format!("{delta:+} {counter:?}")),
        };
        return format!("{} {verb} {what}", cx.name(p));
    }
    let what = match counter {
        C::VpTokens => amount(n, "VP token", "VP tokens"),
        C::Debt => amount(n, "Debt token", "Debt tokens"),
        C::Embargo => amount(n, "Embargo token", "Embargo tokens"),
        C::TradeRoute if pile.is_some() => amount(n, "Trade Route token", "Trade Route tokens"),
        C::TradeRoute => amount(n, "coin token", "coin tokens"),
        C::Sun => amount(n, "Sun token", "Sun tokens"),
        _ => amount(n, "token", "tokens"),
    };
    let place = match (pile, counter) {
        (Some(id), _) => format!("the {} pile", game.pile(id).card),
        (None, C::TradeRoute) => "the Trade Route mat".to_string(),
        (None, _) => game
            .prophecy()
            .map_or("the Prophecy".to_string(), |(c, _)| c.to_string()),
    };
    if up {
        format!("{what} put on {place}")
    } else {
        format!("{what} removed from {place}")
    }
}

pub(crate) fn log_entry(seq: usize, event: &Event, game: &Game, cx: &Context<'_>) -> LogEntry {
    let name = |p: PlayerId| cx.name(p);
    let (player, card, text) = match event {
        Event::TurnStart { player, turn } => (
            Some(*player),
            None,
            format!("{} starts turn {turn}", name(*player)),
        ),
        Event::Played { player, card } => (
            Some(*player),
            Some(*card),
            format!("{} plays {card}", name(*player)),
        ),
        Event::UsedWay { player, card, way } => (
            Some(*player),
            Some(*way),
            format!("{} uses {way} for {card}", name(*player)),
        ),
        Event::Named { player, card } => (
            Some(*player),
            Some(*card),
            format!("{} names {card}", name(*player)),
        ),
        Event::Bought { player, card } | Event::BoughtLandscape { player, card } => (
            Some(*player),
            Some(*card),
            format!("{} buys {card}", name(*player)),
        ),
        Event::Overpaid {
            player,
            amount,
            potions,
        } => {
            let p = if *potions > 0 {
                format!(" and {potions} Potion(s)")
            } else {
                String::new()
            };
            (
                Some(*player),
                None,
                format!("{} overpays ${amount}{p}", name(*player)),
            )
        }
        Event::Gained { player, card, to } => {
            let to = match to {
                dominion_engine::Dest::Discard => "",
                dominion_engine::Dest::Hand => " to their hand",
                dominion_engine::Dest::DeckTop => " onto their deck",
                dominion_engine::Dest::SetAside => ", setting it aside",
            };
            (
                Some(*player),
                Some(*card),
                format!("{} gains {card}{to}", name(*player)),
            )
        }
        Event::Trashed { player, card } => (
            Some(*player),
            Some(*card),
            format!("{} trashes {card}", name(*player)),
        ),
        Event::Discarded { player, card } => (
            Some(*player),
            Some(*card),
            format!("{} discards {card}", name(*player)),
        ),
        Event::TopDecked { player, card } => (
            Some(*player),
            *card,
            format!(
                "{} puts {} onto their deck",
                name(*player),
                card.map_or("a card".to_string(), |c| c.to_string())
            ),
        ),
        Event::SetAside { player, card } => (
            Some(*player),
            Some(*card),
            format!("{} sets aside {card}", name(*player)),
        ),
        Event::Revealed { player, card } => (
            Some(*player),
            Some(*card),
            format!("{} reveals {card}", name(*player)),
        ),
        Event::RevealedHand { player, cards } => {
            let list: Vec<String> = cards.iter().map(|c| c.to_string()).collect();
            let list = if list.is_empty() {
                "nothing".to_string()
            } else {
                list.join(", ")
            };
            (
                Some(*player),
                None,
                format!("{} reveals their hand: {list}", name(*player)),
            )
        }
        Event::Drew { player, count } => (
            Some(*player),
            None,
            format!("{} draws {count} card(s)", name(*player)),
        ),
        Event::Shuffled { player } => (Some(*player), None, format!("{} shuffles", name(*player))),
        Event::Received { player, card } => (
            Some(*player),
            Some(*card),
            format!("{} receives {card}", name(*player)),
        ),
        Event::Rotated { player, pile } => (
            Some(*player),
            None,
            format!(
                "{} rotates the {} pile",
                name(*player),
                game.pile(*pile).card
            ),
        ),
        Event::Moved {
            player,
            from,
            to,
            cards,
            count,
            bottom,
        } => {
            let text = put_text(game, *player, (*from, *to, *bottom), cards, *count, cx);
            let card = (*count == 1).then(|| cards.first().copied()).flatten();
            (Some(*player), card, format!("{} {text}", name(*player)))
        }
        Event::Looked { player, count } => {
            let top = match count {
                1 => "the top card".to_string(),
                n => format!("the top {n} cards"),
            };
            (
                Some(*player),
                None,
                format!("{} looks at {top} of their deck", name(*player)),
            )
        }
        Event::Counter {
            player,
            pile,
            counter,
            delta,
        } => (
            *player,
            None,
            counter_text(game, (*player, *pile), *counter, *delta, cx),
        ),
        Event::TokenMoved {
            player,
            token,
            pile,
        } => {
            let token = token_text(*token);
            let text = match pile {
                Some(id) => format!(
                    "{} puts their {token} token on the {} pile",
                    name(*player),
                    game.pile(*id).card
                ),
                None => format!("{} takes their {token} token", name(*player)),
            };
            (Some(*player), None, text)
        }
        Event::Marker { player, marker, on } => {
            use dominion_engine::Marker as M;
            let text = match (marker, on) {
                (M::MinusCard, true) => "takes the -1 Card token",
                (M::MinusCard, false) => "removes their -1 Card token",
                (M::MinusCoin, true) => "takes the -$1 token",
                (M::MinusCoin, false) => "removes their -$1 token",
                (M::Journey, true) => "turns their Journey token face up",
                (M::Journey, false) => "turns their Journey token face down",
            };
            (Some(*player), None, format!("{} {text}", name(*player)))
        }
        Event::Returned { player, card } => (
            Some(*player),
            Some(*card),
            format!("{} returns {card}", name(*player)),
        ),
        Event::Resolved { player, card } => (
            Some(*player),
            Some(*card),
            format!("{}'s {card} resolves", name(*player)),
        ),
        Event::Bonus {
            player,
            actions,
            buys,
            coins,
        } => {
            let mut parts = vec![];
            if *actions > 0 {
                parts.push(format!("+{}", amount(*actions as i32, "Action", "Actions")));
            }
            if *buys > 0 {
                parts.push(format!("+{}", amount(*buys as i32, "Buy", "Buys")));
            }
            if *coins > 0 {
                parts.push(format!("+${coins}"));
            }
            (
                Some(*player),
                None,
                format!("{} gets {}", name(*player), parts.join(", ")),
            )
        }
        Event::Unaffected { player, attack, by } => (
            Some(*player),
            Some(*by),
            format!("{} is unaffected by {attack} ({by})", name(*player)),
        ),
        Event::ReturnedToAction { player } => (
            Some(*player),
            None,
            format!("{} returns to their Action phase", name(*player)),
        ),
        Event::Called { player, card } => (
            Some(*player),
            Some(*card),
            format!("{} calls {card}", name(*player)),
        ),
    };
    let kind = kebab(
        format!("{event:?}")
            .split([' ', '{', '('])
            .next()
            .unwrap_or("event"),
    );
    LogEntry {
        seq,
        kind,
        player: player.map(|p| p.0),
        card: card.map(|c| c.name().to_string()),
        text,
    }
}

fn log(game: &Game, cx: &Context<'_>) -> Vec<LogEntry> {
    game.log()
        .iter()
        .enumerate()
        .skip(cx.log_from)
        .map(|(i, e)| log_entry(i, e, game, cx))
        .collect()
}
