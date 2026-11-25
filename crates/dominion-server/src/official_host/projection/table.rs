//! What dominion.games shows of the table besides the players' cards and the Supply: the
//! Boons' and Hexes' decks, cards setup sets aside, the reminders of what a player has
//! (States, kept Boons, Artifacts), tokens, and the counts notes keep.
use super::*;
use dominion_engine::{Kind, Note, PileId, PileView, Set, SetupRole, Token};

/// Refuses what the transport can't show yet.
pub(super) fn validate(view: &View<'_>) -> Result<(), ProjectionError> {
    let gap = |feature: String| Err(ProjectionError::UnsupportedState(feature));
    if view.me().is_none() {
        return gap("spectator visibility policy".into());
    }
    if let Some(note) = view.notes().iter().find(|n| !shown(view, n)) {
        return gap(format!("{} note", note.source.name()));
    }
    if view
        .players()
        .any(|p| p.tokens().contains_key(&Token::Estate))
    {
        return gap("Inheritance's Estate token".into());
    }
    for p in view.piles() {
        if p.traits().len() > 1 {
            return gap("more than one Trait on a pile".into());
        }
        if p.hidden() && p.card() == Card::BlackMarket {
            return gap("Black Market deck".into());
        }
    }
    Ok(())
}

/// Whether the table shows `note`: as a pile marker, set-aside cards, a counter, a card
/// already on a mat, or through what it is about (Fleet's round is the turn's type; a
/// Trait on a pile out of the game marks nothing on the table). Contraband's and War
/// Chest's named cards are in the log alone, as on dominion.games.
fn shown(view: &View<'_>, note: &Note) -> bool {
    if setup_marker(note).is_some() {
        return true;
    }
    match (note.source, note.setup) {
        (Card::Druid | Card::WayOfTheMouse | Card::Riverboat, Some(SetupRole::SetAside)) => true,
        (
            Card::TradeRoute
            | Card::SinisterPlot
            | Card::Fleet
            | Card::Voyage
            | Card::FamilyOfInventors
            | Card::Invest
            | Card::Contraband
            | Card::WarChest,
            None,
        ) => true,
        (c, None) if c.kind() == Kind::Landmark => note.pile.is_none(),
        (c, None) if c.kind() == Kind::Trait => note.pile.is_some_and(|p| view.pile(p).removed()),
        _ => false,
    }
}

/// The zone a pile out of the game holds its card in, where it holds one: Way of the
/// Mouse's card, or Riverboat's in the Box. Divine Wind's piles are no zone at all.
pub(super) fn holder(view: &View<'_>, pile: &PileView<'_>) -> Option<i32> {
    if !pile.removed() {
        return None;
    }
    view.notes().iter().find_map(|n| {
        let kind = match n.source {
            Card::WayOfTheMouse => zone::MOUSE,
            Card::Riverboat => zone::THE_BOX,
            _ => return None,
        };
        (n.setup == Some(SetupRole::SetAside) && n.card == Some(pile.card())).then_some(kind)
    })
}

/// The Boons' and Hexes' decks (a non-Supply pile face down, and a discard pile in a
/// hidden window) and Druid's Boons beside its pile.
pub(super) fn fate_zones(
    view: &View<'_>,
    catalog: &WireCatalog,
) -> Result<Vec<VisibleZone>, ProjectionError> {
    let kept = view.players().any(|p| !p.boons_kept().is_empty());
    let mut zones = vec![];
    for (deck, kept, (draw, discard, kind)) in [
        (
            view.boons(),
            kept,
            ("Boon Drawpile", "Boon Discardpile", zone::BOON_DISCARD),
        ),
        (
            view.hexes(),
            false,
            ("Hex Drawpile", "Hex Discardpile", zone::HEX_DISCARD),
        ),
    ] {
        if deck.draw_size + deck.discard.len() == 0 && !kept {
            continue;
        }
        zones.push(VisibleZone::new(
            zone::NON_SUPPLY,
            -1,
            catalog.named(draw)?,
            vec![None; deck.draw_size],
        ));
        zones.push(VisibleZone::named(
            kind,
            catalog.named(discard)?,
            deck.discard,
        ));
    }
    let druid: Vec<Card> = druid_boons(view).collect();
    if !druid.is_empty() {
        zones.push(VisibleZone::named(
            zone::DRUID_BOONS,
            catalog.named("Druid Boons")?,
            &druid,
        ));
    }
    Ok(zones)
}

fn druid_boons<'a>(view: &'a View<'_>) -> impl Iterator<Item = Card> + 'a {
    view.notes()
        .into_iter()
        .filter(|n| n.source == Card::Druid && n.setup == Some(SetupRole::SetAside))
        .filter_map(|n| n.card)
}

/// The cards in the game that lie in no zone, which their client lists among the game's
/// cards: every Boon and Hex once their deck is in it, and the States and Artifacts the
/// game's cards use or a player has. All of them are public.
pub(super) fn offstage(view: &View<'_>) -> BTreeSet<Card> {
    let mut out = BTreeSet::new();
    let boons = view.boons();
    let hexes = view.hexes();
    let has_boons = boons.draw_size + boons.discard.len() > 0
        || view.players().any(|p| !p.boons_kept().is_empty())
        || druid_boons(view).next().is_some();
    let has_hexes = hexes.draw_size + hexes.discard.len() > 0;
    for (present, kind) in [(has_boons, Kind::Boon), (has_hexes, Kind::Hex)] {
        if present {
            out.extend(Card::in_set(Set::Nocturne).filter(|c| c.kind() == kind));
        }
    }
    let in_game = view
        .piles()
        .filter(|p| !p.removed())
        .flat_map(|p| p.names().unwrap_or_default().into_iter().chain([p.card()]))
        .chain(view.landscapes().iter().copied())
        .collect::<BTreeSet<_>>();
    for card in in_game.into_iter().chain(out.clone()) {
        let related = card.related();
        out.extend(related.states.iter().copied());
        out.extend(related.artifacts.iter().copied());
    }
    out.extend(view.artifacts().iter().map(|(a, _)| *a));
    for p in view.players() {
        out.extend(p.states().iter().copied());
    }
    out
}

/// What a player is reminded they have: States, kept Boons, the Artifacts they hold, and
/// lasting effects of those kinds. dominion.games has reminder art for no other card (a
/// Duration's effect has none), so it reminds of nothing else.
pub(super) fn reminders(
    view: &View<'_>,
    catalog: &WireCatalog,
    seat: usize,
) -> Result<Vec<model::Reminder>, ProjectionError> {
    let p = view.player(PlayerId(seat as u8));
    let held = view
        .artifacts()
        .iter()
        .filter(|(_, holder)| *holder == Some(p.id()))
        .map(|(a, _)| *a);
    let mut counts: BTreeMap<i32, i32> = BTreeMap::new();
    for card in view
        .lasting()
        .iter()
        .filter(|l| l.owner == p.id())
        .map(|l| l.card)
        .filter(|c| c.is(Types::STATE | Types::BOON | Types::ARTIFACT))
        .chain(p.states().iter().copied())
        .chain(p.boons_kept().iter().copied())
        .chain(held)
    {
        *counts.entry(catalog.card(card)?).or_insert(0) += 1;
    }
    Ok(counts
        .into_iter()
        .map(|(card_name, frequency)| model::Reminder {
            card_name,
            frequency,
            blinking: false,
        })
        .collect())
}

/// Players' tokens: Adventures tokens on piles, and a -1 Card or -$1 token on its
/// owner's deck while it is out (flipped, as their client reads it). Every token lies in
/// a zone: their client attaches it there.
pub(super) fn tokens(
    view: &View<'_>,
    catalog: &WireCatalog,
    piles: &BTreeMap<PileId, i32>,
    zones: &[model::Zone],
) -> Result<Vec<model::Token>, ProjectionError> {
    let mut tokens = vec![];
    for p in view.players() {
        let owner = i32::from(p.id().0);
        for (&token, pile) in p.tokens() {
            let Some(&zone) = pile.and_then(|pile| piles.get(&pile)) else {
                continue;
            };
            let kind = match token {
                Token::PlusAction => "PLUS_ACTION",
                Token::PlusBuy => "PLUS_BUY",
                Token::PlusCoin => "PLUS_COIN",
                Token::PlusCard => "PLUS_CARD",
                Token::MinusCost => "MINUS_COST",
                Token::Trashing => "TRASH",
                Token::Estate => {
                    return Err(ProjectionError::UnsupportedState(
                        "Inheritance's Estate token".into(),
                    ));
                }
            };
            tokens.push(model::Token {
                kind: catalog.symbol("TokenNames", kind)?,
                owner,
                zone,
                is_flipped: false,
            });
        }
        if !(p.minus_card() || p.minus_coin()) {
            continue;
        }
        let deck = zones
            .iter()
            .find(|z| z.kind == zone::DRAW && z.owner == owner)
            .map(|z| z.index)
            .ok_or(ProjectionError::Inconsistent("deck zone"))?;
        for (out, kind) in [
            (p.minus_card(), "MINUS_CARD"),
            (p.minus_coin(), "MINUS_COIN"),
        ] {
            if out {
                tokens.push(model::Token {
                    kind: catalog.symbol("TokenNames", kind)?,
                    owner,
                    zone: deck,
                    is_flipped: true,
                });
            }
        }
    }
    Ok(tokens)
}

/// A counter of nobody's (-1), or a player's, on a zone or on the table (-1).
pub(super) struct Count {
    pub kind: &'static str,
    pub owner: i32,
    pub zone: i32,
    pub value: i32,
}

/// The tokens on piles and landscapes, and the counts notes keep.
pub(super) fn counts(
    view: &View<'_>,
    piles: &BTreeMap<PileId, i32>,
    landscapes: &BTreeMap<Card, i32>,
) -> Vec<Count> {
    let mut out = vec![];
    let mut add = |kind, owner, zone, value: i32| {
        if value != 0 {
            out.push(Count {
                kind,
                owner,
                zone,
                value,
            });
        }
    };
    for (&pile, &zone) in piles {
        let p = view.pile(pile);
        add("VP_TOKENS", -1, zone, p.vp_tokens());
        add("DEBT", -1, zone, p.debt_tokens());
        add("EMBARGO_TOKENS", -1, zone, p.embargo() as i32);
        add("BLOCKADE_TOKENS", -1, zone, p.blockades() as i32);
        add("TRADE_ROUTE_COINS", -1, zone, i32::from(p.trade_route()));
    }
    for note in view.notes() {
        let on = |card: Card| landscapes.get(&card).copied();
        let owner = note.player.map_or(-1, |p| i32::from(p.0));
        match note.source {
            Card::TradeRoute => add("TRADE_ROUTE_MAT", -1, -1, note.count),
            Card::Voyage => add("REMAINING_HAND_PLAYS", owner, -1, note.count),
            Card::FamilyOfInventors => {
                if let Some(&zone) = note.pile.and_then(|p| piles.get(&p)) {
                    add("FAVORS", -1, zone, note.count);
                }
            }
            Card::SinisterPlot => {
                if let Some(zone) = on(Card::SinisterPlot) {
                    add("SINISTER_PLOT", owner, zone, note.count);
                }
            }
            c if c.kind() == Kind::Landmark && note.pile.is_none() => {
                if let Some(zone) = on(c) {
                    add("VP_TOKENS", -1, zone, note.count);
                }
            }
            _ => {}
        }
    }
    if let Some((prophecy, suns)) = view.prophecy()
        && let Some(zone) = landscapes.get(&prophecy).copied()
    {
        add("SUN_TOKENS", -1, zone, suns as i32);
    }
    out
}

/// The type of the turn being taken: what granted it, or Fleet's round after the game's
/// end.
pub(super) fn turn_type(view: &View<'_>) -> Result<&'static str, ProjectionError> {
    Ok(match view.turn().extra {
        None if view.notes().iter().any(|n| n.source == Card::Fleet) => "FLEET",
        None => "REGULAR",
        Some(Card::Outpost) => "OUTPOST",
        Some(Card::Voyage) => "VOYAGE",
        Some(Card::Mission) => "MISSION",
        Some(Card::SeizeTheDay) => "SEIZE_THE_DAY",
        Some(Card::IslandFolk) => "ISLAND_FOLK",
        Some(Card::Journey) => "JOURNEY",
        Some(c) => {
            return Err(ProjectionError::UnsupportedState(format!(
                "{} extra turn",
                c.name()
            )));
        }
    })
}
