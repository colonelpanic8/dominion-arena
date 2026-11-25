//! Question bindings for this set's cards (see `semantics::SETS`).
use super::{
    AbilityArguments, Binding, ChoiceShape, ProjectionError, WireCatalog, ability_card, cost_limit,
    gap, names_of, occasion_card,
};
use dominion_engine::{Card, Decision, DecisionKind, Opt, Purpose, Trigger, View};
use dominion_online::questions::LogArgument;

pub(super) fn binding(
    view: &View<'_>,
    d: &Decision,
    c: &WireCatalog,
) -> Option<Result<Binding, ProjectionError>> {
    use Card::*;
    use Purpose as P;
    let ordered = matches!(d.kind, DecisionKind::Cards { ordered: true, .. });
    let choosing = matches!(d.kind, DecisionKind::Choose { .. });
    let own_turn = view.turn().player == d.player;
    let (kind, story) = match (d.source?, d.purpose) {
        (Anvil, P::Discard) => ("DISCARD", "ANVIL_DISCARD"),
        (Anvil, P::Gain) => ("GAIN", "ANVIL_GAIN"),
        (Bishop, P::Trash) if own_turn => ("TRASH", "BISHOP_SELF"),
        (Bishop, P::Trash) => ("TRASH", "BISHOP_OTHER"),
        (Clerk, P::TopDeck) => ("TOPDECK", "CLERK"),
        (Contraband, P::Name) => {
            let buyer = LogArgument::Player(view.turn().player.0.into());
            return Some(Ok(("WISH", "CONTRABAND", vec![buyer])));
        }
        (CountingHouse, P::Move) => ("PUT_IN_HAND", "COUNTING_HOUSE"),
        (CrystalBall, P::Either) => {
            return Some(about(view, d, c).map(|arg| ("CHOOSE_MODE", "CRYSTAL_BALL", vec![arg])));
        }
        (Expand, P::Trash) => ("TRASH", "EXPAND_TRASH"),
        (Expand, P::Gain) => {
            return Some(cost_limit(view, d).map(|a| ("GAIN", "EXPAND_GAIN", vec![a])));
        }
        (Forge, P::Trash) => ("TRASH", "FORGE_TRASH"),
        (Forge, P::Gain) => {
            return Some(cost_limit(view, d).map(|a| ("GAIN", "FORGE_GAIN", vec![a])));
        }
        (Goons, P::Discard) => ("DISCARD", "GOONS"),
        (Investment, P::Mode) => ("CHOOSE_MODE", "INVESTMENT_MODE"),
        (Investment, P::Trash) => ("TRASH", "INVESTMENT_TRASH"),
        (KingsCourt, P::Play) => ("PLAY", "KINGS_COURT"),
        (Loan, P::Either) => {
            let card = view
                .pending_reference()
                .and_then(|id| view.card(id))
                .ok_or_else(|| gap(d, "missing revealed Treasure"));
            return Some(
                card.and_then(|card| names_of(c, [card]))
                    .map(|arg| ("CHOOSE_MODE", "LOAN", vec![arg])),
            );
        }
        (Mint, P::Reveal) => ("REVEAL", "MINT"),
        (Mountebank, P::Discard) if choosing => ("DISCARD", "MOUNTEBANK"),
        (Rabble, P::TopDeck) if ordered => ("ORDER_CARDS", "RABBLE"),
        (Tiara, P::Play) => ("PLAY", "TIARA_PLAY"),
        (TradeRoute, P::Trash) => ("TRASH", "TRADE_ROUTE"),
        (Vault, P::Discard) if choosing => ("COMPLEX_OR", "VAULT_OTHER"),
        (Vault, P::Discard) if own_turn => ("DISCARD", "VAULT_SELF"),
        (Vault, P::Discard) => ("DISCARD", "VAULT_OTHER"),
        (WarChest, P::Name) => {
            let player = LogArgument::Player(view.turn().player.0.into());
            return Some(Ok(("WISH", "WAR_CHEST_PROHIBIT", vec![player])));
        }
        (WarChest, P::Gain) => ("GAIN", "WAR_CHEST_GAIN"),
        _ => return None,
    };
    Some(Ok((kind, story, vec![])))
}

/// The one card the question is about, as CARDS0.
fn about(view: &View<'_>, d: &Decision, c: &WireCatalog) -> Result<LogArgument, ProjectionError> {
    let card = d
        .about
        .first()
        .and_then(|&id| view.card(id))
        .ok_or_else(|| gap(d, "missing card the question is about"))?;
    names_of(c, [card])
}

pub(super) fn mode(card: Card, mode: u8) -> Option<&'static str> {
    let choices: &[&str] = match card {
        Card::CrystalBall => &[
            "CRYSTAL_BALL_TRASH",
            "CRYSTAL_BALL_DISCARD",
            "CRYSTAL_BALL_PLAY",
            "CRYSTAL_BALL_TOPDECK",
        ],
        Card::Investment => &["INVESTMENT_COIN", "INVESTMENT_VP"],
        Card::Loan => &["LOAN_DISCARD", "LOAN_TRASH"],
        _ => return None,
    };
    choices.get(usize::from(mode)).copied()
}

pub(super) fn ability(card: Card, ability: u8) -> Option<&'static str> {
    Some(match (card, ability) {
        (Card::Clerk, 0) => "CLERK_ABILITY",
        (Card::Hoard, 0) => "TREASURE_CHEST",
        (Card::Talisman, 0) => "GAIN_BY_NAME",
        (Card::Mint, 0) => "MINT",
        (Card::RoyalSeal | Card::Tiara, 0) => "TOPDECK_REACTION",
        _ => return None,
    })
}

pub(super) fn choice_shape(
    view: &View<'_>,
    d: &Decision,
) -> Option<Result<ChoiceShape, ProjectionError>> {
    if !matches!(d.kind, DecisionKind::Choose { .. }) {
        return None;
    }
    Some(Ok(match d.source? {
        Card::Vault => {
            ChoiceShape::Buttons(vec![(Opt::Yes, "CONTINUE"), (Opt::No, "VAULT_NO_DISCARD")])
        }
        Card::Mountebank => {
            let curse = view
                .hand()
                .iter()
                .copied()
                .filter(|&id| view.card(id) == Some(Card::Curse))
                .min();
            match curse {
                Some(card) => ChoiceShape::Target {
                    card,
                    yes: Opt::Yes,
                    no: Opt::No,
                    decline: "DONT_DISCARD",
                },
                None => return Some(Err(gap(d, "Mountebank without a Curse in hand"))),
            }
        }
        _ => return None,
    }))
}

pub(super) fn ability_arguments(view: &View<'_>, t: &Trigger, c: &WireCatalog) -> AbilityArguments {
    match (t.card, t.ability) {
        (Card::RoyalSeal | Card::Tiara, 0) => Some(ability_card(t, occasion_card(view, t), c)),
        // "gain Smithy (Talisman)"
        (Card::Talisman, 0) => Some(ability_card(t, occasion_card(view, t), c).and_then(
            |mut args| {
                args.push(names_of(c, [Card::Talisman])?);
                Ok(args)
            },
        )),
        _ => None,
    }
}

pub(super) fn symbols(card: Card) -> &'static [(&'static str, &'static str)] {
    use Card::*;
    match card {
        Anvil => &[
            ("QuestionIds", "ANVIL_DISCARD"),
            ("QuestionIds", "ANVIL_GAIN"),
            ("QuestionTypes", "DISCARD"),
            ("QuestionTypes", "GAIN"),
        ],
        Bishop => &[
            ("QuestionIds", "BISHOP_OTHER"),
            ("QuestionIds", "BISHOP_SELF"),
            ("QuestionTypes", "TRASH"),
        ],
        Clerk => &[
            ("AbilityNames", "CLERK_ABILITY"),
            ("QuestionIds", "CLERK"),
            ("QuestionTypes", "TOPDECK"),
        ],
        Contraband => &[("QuestionIds", "CONTRABAND"), ("QuestionTypes", "WISH")],
        CountingHouse => &[
            ("QuestionIds", "COUNTING_HOUSE"),
            ("QuestionTypes", "PUT_IN_HAND"),
        ],
        CrystalBall => &[
            ("CardModes", "CRYSTAL_BALL_DISCARD"),
            ("CardModes", "CRYSTAL_BALL_PLAY"),
            ("CardModes", "CRYSTAL_BALL_TOPDECK"),
            ("CardModes", "CRYSTAL_BALL_TRASH"),
            ("QuestionIds", "CRYSTAL_BALL"),
            ("QuestionTypes", "CHOOSE_MODE"),
        ],
        Expand => &[
            ("QuestionIds", "EXPAND_GAIN"),
            ("QuestionIds", "EXPAND_TRASH"),
            ("QuestionTypes", "GAIN"),
            ("QuestionTypes", "TRASH"),
        ],
        Forge => &[
            ("QuestionIds", "FORGE_GAIN"),
            ("QuestionIds", "FORGE_TRASH"),
            ("QuestionTypes", "GAIN"),
            ("QuestionTypes", "TRASH"),
        ],
        Goons => &[("QuestionIds", "GOONS"), ("QuestionTypes", "DISCARD")],
        Hoard => &[("AbilityNames", "TREASURE_CHEST")],
        Investment => &[
            ("CardModes", "INVESTMENT_COIN"),
            ("CardModes", "INVESTMENT_VP"),
            ("QuestionIds", "INVESTMENT_MODE"),
            ("QuestionIds", "INVESTMENT_TRASH"),
            ("QuestionTypes", "CHOOSE_MODE"),
            ("QuestionTypes", "TRASH"),
        ],
        KingsCourt => &[("QuestionIds", "KINGS_COURT"), ("QuestionTypes", "PLAY")],
        Loan => &[
            ("CardModes", "LOAN_DISCARD"),
            ("CardModes", "LOAN_TRASH"),
            ("QuestionIds", "LOAN"),
            ("QuestionTypes", "CHOOSE_MODE"),
        ],
        Mint => &[
            ("AbilityNames", "MINT"),
            ("QuestionIds", "MINT"),
            ("QuestionTypes", "REVEAL"),
        ],
        Mountebank => &[("QuestionIds", "MOUNTEBANK"), ("QuestionTypes", "DISCARD")],
        Rabble => &[("QuestionIds", "RABBLE"), ("QuestionTypes", "ORDER_CARDS")],
        RoyalSeal => &[("AbilityNames", "TOPDECK_REACTION")],
        Talisman => &[("AbilityNames", "GAIN_BY_NAME")],
        Tiara => &[
            ("AbilityNames", "TOPDECK_REACTION"),
            ("QuestionIds", "TIARA_PLAY"),
            ("QuestionTypes", "PLAY"),
        ],
        TradeRoute => &[("QuestionIds", "TRADE_ROUTE"), ("QuestionTypes", "TRASH")],
        Vault => &[
            ("DeclineButtonIds", "CONTINUE"),
            ("DeclineButtonIds", "VAULT_NO_DISCARD"),
            ("QuestionIds", "VAULT_OTHER"),
            ("QuestionIds", "VAULT_SELF"),
            ("QuestionTypes", "BUTTON"),
            ("QuestionTypes", "COMPLEX_OR"),
            ("QuestionTypes", "DISCARD"),
        ],
        WarChest => &[
            ("QuestionIds", "WAR_CHEST_GAIN"),
            ("QuestionIds", "WAR_CHEST_PROHIBIT"),
            ("QuestionTypes", "GAIN"),
            ("QuestionTypes", "WISH"),
        ],
        _ => &[],
    }
}

/// The features a card needs (see `profile::requirements`); `None` until reviewed.
pub(super) fn requirements(card: Card) -> Option<&'static [&'static str]> {
    use Card::*;
    Some(match card {
        Anvil => &["card-selection", "pile-selection"],
        Bank | City | GrandMarket | Magnate | Monument | Venture | WorkersVillage => &[],
        Bishop => &["card-selection"],
        Charlatan => &["off-turn-attack"],
        Clerk => &["off-turn-attack", "card-selection", "reaction-ability"],
        Collection | Hoard | Talisman => &["passive-lasting"],
        Contraband | WarChest => &["name-card", "pile-selection", "public-effects"],
        CountingHouse => &["discard-inspection", "card-selection"],
        CrystalBall => &["looking", "semantic-modes"],
        Expand | Forge => &["card-selection", "pile-selection", "explicit-cost-bound"],
        Goons => &["off-turn-attack", "card-selection"],
        Investment => &["semantic-modes", "card-selection"],
        KingsCourt => &["repeated-play", "card-selection"],
        Loan => &["semantic-modes", "visible-reference"],
        Mint => &["card-selection", "pile-selection", "optional-ability"],
        Mountebank => &["off-turn-attack", "optional-effect"],
        Peddler | Quarry => &["dynamic-cost"],
        Rabble => &["off-turn-attack", "looking", "ordered-cards"],
        RoyalSeal => &["optional-ability"],
        Tiara => &["repeated-play", "card-selection", "optional-ability"],
        TradeRoute => &["card-selection", "pile-tokens"],
        Vault => &["card-selection", "optional-effect"],
        _ => return None,
    })
}
