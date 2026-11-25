//! Question bindings for Cornucopia & Guilds (see `semantics::SETS`).
use super::{
    Binding, Choice, ProjectionError, SetExtras, WireCatalog, card_names, cost_bound, gap,
};
use dominion_engine::{Card, Decision, DecisionKind, Occasion, Opt, Purpose, Trigger, View};
use dominion_online::questions::LogArgument;

pub(super) const EXTRAS: SetExtras = SetExtras {
    choice,
    ability_arguments,
    decline,
};

pub(super) fn binding(
    view: &View<'_>,
    d: &Decision,
    c: &WireCatalog,
) -> Option<Result<Binding, ProjectionError>> {
    use Card::*;
    use Purpose as P;
    let ordered = matches!(d.kind, DecisionKind::Cards { ordered: true, .. });
    let step = view.pending_step();
    let second = view.pending_counter() == Some(2);
    let (kind, story) = match (d.source?, d.purpose) {
        (Doctor, P::Overpay) => ("OVERPAY", "DOCTOR_OVERPAY"),
        (Farrier, P::Overpay) => ("OVERPAY", "FARRIER_OVERPAY"),
        (Herald, P::Overpay) => ("OVERPAY", "HERALD_OVERPAY"),
        (Infirmary, P::Overpay) => ("OVERPAY", "INFIRMARY_OVERPAY"),
        (Masterpiece, P::Overpay) => ("OVERPAY", "MASTERPIECE"),
        (Stonemason, P::Overpay) => ("OVERPAY", "STONEMASON_OVERPAY_AMOUNT"),
        (Advisor, P::Discard) => ("DISCARD", "ADVISOR"),
        (Butcher, P::Trash) => ("TRASH", "BUTCHER_TRASH"),
        (Butcher, P::Gain) => ("GAIN", "BUTCHER_GAIN"),
        (Coronet, P::Play) if step == Some(1) => ("PLAY", "CORONET_ACTION"),
        (Coronet, P::Play) => ("PLAY", "CORONET_TREASURE"),
        (Courser, P::Mode) => ("CHOOSE_MODE", "COURSER"),
        (TrustySteed, P::Mode) => ("CHOOSE_MODE", "TRUSTY_STEED"),
        (Doctor, P::Name) => ("WISH", "DOCTOR_PLAY"),
        (Doctor, P::TopDeck) if ordered => ("ORDER_CARDS", "DOCTOR_TOPDECK"),
        (Doctor, P::Either) => ("CHOOSE_MODE", "DOCTOR_MODE"),
        (Ferryman, P::Discard) => ("DISCARD", "FERRYMAN"),
        (Followers, P::Discard) => ("DISCARD", "FOLLOWERS"),
        (Footpad, P::Discard) => ("DISCARD", "FOOTPAD"),
        (Hamlet, P::Discard) if step == Some(1) => ("DISCARD", "HAMLET_ACTION"),
        (Hamlet, P::Discard) => ("DISCARD", "HAMLET_BUY"),
        (Herald, P::TopDeck) => (
            if ordered { "ORDER_CARDS" } else { "TOPDECK" },
            "HERALD_CHOOSE",
        ),
        (HornOfPlenty, P::Gain) => ("GAIN", "HORN_OF_PLENTY"),
        (HorseTraders, P::Discard) => ("DISCARD", "HORSE_TRADERS"),
        (Infirmary, P::Trash) => ("TRASH", "INFIRMARY_TRASH"),
        (Jester, P::Gain) => ("CHOOSE_MODE", "JESTER"),
        (Journeyman, P::Name) => ("WISH", "JOURNEYMAN"),
        (Joust, P::SetAside) => ("SET_ASIDE", "JOUST_SET_ASIDE"),
        (Joust, P::Gain) => ("GAIN", "JOUST_GAIN_REWARD"),
        (Plaza, P::Discard) => ("DISCARD", "PLAZA_DISCARD"),
        (Remake, P::Trash) if second => ("TRASH", "REMAKE_TRASH_SECOND"),
        (Remake, P::Trash) => ("TRASH", "REMAKE_TRASH_FIRST"),
        (Remake, P::Gain) if second => ("GAIN", "REMAKE_GAIN_SECOND"),
        (Remake, P::Gain) => ("GAIN", "REMAKE_GAIN_FIRST"),
        (Shop, P::Play) => ("PLAY", "SHOP"),
        (Stonemason, P::Trash) => ("TRASH", "STONEMASON_TRASH"),
        // The overpay's gains ask from its ability, with no trashed card.
        (Stonemason, P::Gain) if view.pending_reference().is_none() => {
            ("GAIN", "STONEMASON_GAIN_EQUAL")
        }
        (Stonemason, P::Gain) if second => ("GAIN", "STONEMASON_GAIN2"),
        (Stonemason, P::Gain) => ("GAIN", "STONEMASON_GAIN1"),
        (Taxman, P::Trash) => ("TRASH", "TAXMAN_TRASH"),
        (Taxman, P::Gain) => ("GAIN", "TAXMAN_GAIN"),
        (Tournament, P::Reveal) => ("REVEAL", "TOURNAMENT_REVEAL"),
        (Tournament, P::Gain) => ("GAIN_PRIZE", "TOURNAMENT_GAIN_PRIZE_OR_DUCHY"),
        (YoungWitch, P::Discard) => ("DISCARD", "YOUNG_WITCH"),
        (YoungWitch, P::Reveal) => ("REVEAL", "YOUNG_WITCH_BANE"),
        _ => return None,
    };
    Some(arguments(view, d, c, story).map(|args| (kind, story, args)))
}

fn arguments(
    view: &View<'_>,
    d: &Decision,
    c: &WireCatalog,
    story: &str,
) -> Result<Vec<LogArgument>, ProjectionError> {
    let reference = || {
        view.pending_reference()
            .ok_or_else(|| gap(d, "missing visible referenced card"))
    };
    Ok(match story {
        "BUTCHER_GAIN"
        | "HORN_OF_PLENTY"
        | "STONEMASON_GAIN1"
        | "STONEMASON_GAIN2"
        | "STONEMASON_GAIN_EQUAL"
        | "TAXMAN_GAIN"
        | "REMAKE_GAIN_FIRST"
        | "REMAKE_GAIN_SECOND" => {
            vec![cost_bound(view, d)?]
        }
        "STONEMASON_OVERPAY_AMOUNT" => {
            let DecisionKind::Number { max, .. } = d.kind else {
                return Err(gap(d, "overpay without an amount"));
            };
            vec![LogArgument::Amount {
                coins: max,
                potions: 0,
                debt: 0,
            }]
        }
        "ADVISOR" => vec![super::subject(view, d)?],
        "DOCTOR_MODE" => vec![card_names(view, d, c, d.about.iter().copied())?],
        "YOUNG_WITCH_BANE" => vec![card_names(view, d, c, [reference()?])?],
        "JESTER" => {
            let DecisionKind::Choose { options, .. } = &d.kind else {
                return Err(gap(d, "Jester without players"));
            };
            let other = options
                .iter()
                .find_map(|o| match o {
                    Opt::Player(p) if *p != d.player => Some(*p),
                    _ => None,
                })
                .ok_or_else(|| gap(d, "Jester without its victim"))?;
            vec![
                card_names(view, d, c, [reference()?])?,
                LogArgument::Player(other.0.into()),
            ]
        }
        _ => vec![],
    })
}

fn choice(view: &View<'_>, d: &Decision) -> Option<Choice> {
    let DecisionKind::Choose { options, .. } = &d.kind else {
        return None;
    };
    match d.source? {
        Card::Jester => options
            .iter()
            .map(|o| match o {
                Opt::Player(p) if *p == d.player => Some("JESTER_YOU_GAIN"),
                Opt::Player(_) => Some("JESTER_THEY_GAIN"),
                _ => None,
            })
            .collect::<Option<Vec<_>>>()
            .map(Choice::Modes),
        Card::YoungWitch if options.as_slice() == [Opt::Yes, Opt::No] => {
            view.pending_reference().map(Choice::Card)
        }
        Card::Tournament if options.as_slice() == [Opt::Yes, Opt::No] => {
            let me = view.me()?;
            view.player(me)
                .hand()?
                .iter()
                .copied()
                .find(|&id| view.card(id) == Some(Card::Province))
                .map(Choice::Card)
        }
        _ => None,
    }
}

fn decline(d: &Decision) -> Option<&'static str> {
    Some(match d.source? {
        Card::Hamlet => "HAMLET_DONT_DISCARD",
        Card::Plaza => "PLAZA_NO_DISCARD",
        Card::Tournament => "TOURNAMENT_DONT_REVEAL",
        Card::YoungWitch => "YOUNG_WITCH_DONT_REVEAL_BANE",
        _ => return None,
    })
}

pub(super) fn mode(card: Card, mode: u8) -> Option<&'static str> {
    use Card::*;
    let choices: &[&str] = match card {
        Courser => &[
            "COURSER_CARDS",
            "COURSER_ACTIONS",
            "COURSER_COINS",
            "COURSER_SILVERS",
        ],
        TrustySteed => &[
            "TRUSTY_STEED_CARDS",
            "TRUSTY_STEED_ACTIONS",
            "TRUSTY_STEED_COINS",
            "TRUSTY_STEED_SILVERS",
        ],
        Doctor => &["DOCTOR_TRASH", "DOCTOR_DISCARD", "DOCTOR_TOPDECK"],
        _ => return None,
    };
    choices.get(usize::from(mode)).copied()
}

pub(super) fn ability(card: Card, ability: u8) -> Option<&'static str> {
    use Card::*;
    Some(match (card, ability) {
        (Doctor, 0) => "RESOLVE_DOCTOR_OVERPAY",
        (Herald, 0) => "RESOLVE_HERALD_OVERPAY",
        (Masterpiece, 0) => "RESOLVE_MASTERPIECE_OVERPAY",
        (Stonemason, 0) => "RESOLVE_STONEMASON_OVERPAY",
        (Farmhands, 2) => "DELAY_PLAY",
        (Footpad, 0) => "DRAW_INSTRUCTIONS",
        (MerchantGuild, 0) => "MERCHANT_GUILD",
        (HorseTraders, 0) => "HORSE_TRADERS_REACTION",
        (HorseTraders, 1) => "HORSE_TRADERS_RETURN_ABILITY",
        _ => return None,
    })
}

fn ability_arguments(
    view: &View<'_>,
    t: &Trigger,
    _occasion: Option<&Occasion>,
    c: &WireCatalog,
) -> Option<Result<Vec<LogArgument>, ProjectionError>> {
    match (t.card, t.ability) {
        (Card::Footpad, 0) => Some(Ok(vec![LogArgument::Number(1)])),
        (Card::Farmhands, 2) => {
            let card = view
                .lasting()
                .into_iter()
                .find(|l| Some(l.id) == t.lasting)
                .and_then(|l| l.data.pick)
                .and_then(|id| view.card(id));
            Some(match card {
                Some(card) => c.card(card).map(|name| {
                    vec![LogArgument::CardNames(vec![
                        dominion_online::questions::CardFrequency { name, frequency: 1 },
                    ])]
                }),
                None => Err(ProjectionError::Inconsistent("Farmhands card unseen")),
            })
        }
        _ => None,
    }
}

macro_rules! symbols {
    ($($table:ident . $symbol:ident),* $(,)?) => {
        &[$((stringify!($table), stringify!($symbol))),*]
    };
}

pub(super) fn symbols(card: Card) -> &'static [(&'static str, &'static str)] {
    use Card::*;
    match card {
        Advisor => symbols![QuestionIds.ADVISOR, QuestionTypes.DISCARD],
        Butcher => symbols![
            QuestionIds.BUTCHER_TRASH,
            QuestionIds.BUTCHER_GAIN,
            QuestionTypes.TRASH,
            QuestionTypes.GAIN,
        ],
        Coronet => symbols![
            QuestionIds.CORONET_ACTION,
            QuestionIds.CORONET_TREASURE,
            QuestionTypes.PLAY,
        ],
        Courser => symbols![
            QuestionIds.COURSER,
            QuestionTypes.CHOOSE_MODE,
            CardModes.COURSER_CARDS,
            CardModes.COURSER_ACTIONS,
            CardModes.COURSER_COINS,
            CardModes.COURSER_SILVERS,
        ],
        TrustySteed => symbols![
            QuestionIds.TRUSTY_STEED,
            QuestionTypes.CHOOSE_MODE,
            CardModes.TRUSTY_STEED_CARDS,
            CardModes.TRUSTY_STEED_ACTIONS,
            CardModes.TRUSTY_STEED_COINS,
            CardModes.TRUSTY_STEED_SILVERS,
        ],
        Doctor => symbols![
            QuestionIds.DOCTOR_OVERPAY,
            QuestionIds.DOCTOR_PLAY,
            QuestionIds.DOCTOR_TOPDECK,
            QuestionIds.DOCTOR_MODE,
            QuestionTypes.OVERPAY,
            QuestionTypes.WISH,
            QuestionTypes.ORDER_CARDS,
            QuestionTypes.CHOOSE_MODE,
            CardModes.DOCTOR_TRASH,
            CardModes.DOCTOR_DISCARD,
            CardModes.DOCTOR_TOPDECK,
            AbilityNames.RESOLVE_DOCTOR_OVERPAY,
        ],
        Farrier => symbols![QuestionIds.FARRIER_OVERPAY, QuestionTypes.OVERPAY],
        Farmhands => symbols![AbilityNames.DELAY_PLAY],
        Ferryman => symbols![QuestionIds.FERRYMAN, QuestionTypes.DISCARD],
        Followers => symbols![QuestionIds.FOLLOWERS, QuestionTypes.DISCARD],
        Footpad => symbols![
            QuestionIds.FOOTPAD,
            QuestionTypes.DISCARD,
            AbilityNames.DRAW_INSTRUCTIONS,
        ],
        Hamlet => symbols![
            QuestionIds.HAMLET_ACTION,
            QuestionIds.HAMLET_BUY,
            QuestionTypes.DISCARD,
            DeclineButtonIds.HAMLET_DONT_DISCARD,
        ],
        Herald => symbols![
            QuestionIds.HERALD_OVERPAY,
            QuestionIds.HERALD_CHOOSE,
            QuestionTypes.OVERPAY,
            QuestionTypes.TOPDECK,
            QuestionTypes.ORDER_CARDS,
            AbilityNames.RESOLVE_HERALD_OVERPAY,
        ],
        HornOfPlenty => symbols![QuestionIds.HORN_OF_PLENTY, QuestionTypes.GAIN],
        HorseTraders => symbols![
            QuestionIds.HORSE_TRADERS,
            QuestionTypes.DISCARD,
            AbilityNames.HORSE_TRADERS_REACTION,
            AbilityNames.HORSE_TRADERS_RETURN_ABILITY,
        ],
        Infirmary => symbols![
            QuestionIds.INFIRMARY_OVERPAY,
            QuestionIds.INFIRMARY_TRASH,
            QuestionTypes.OVERPAY,
            QuestionTypes.TRASH,
        ],
        Jester => symbols![
            QuestionIds.JESTER,
            QuestionTypes.CHOOSE_MODE,
            CardModes.JESTER_THEY_GAIN,
            CardModes.JESTER_YOU_GAIN,
        ],
        Journeyman => symbols![QuestionIds.JOURNEYMAN, QuestionTypes.WISH],
        Joust => symbols![
            QuestionIds.JOUST_SET_ASIDE,
            QuestionIds.JOUST_GAIN_REWARD,
            QuestionTypes.SET_ASIDE,
            QuestionTypes.GAIN,
        ],
        Masterpiece => symbols![
            QuestionIds.MASTERPIECE,
            QuestionTypes.OVERPAY,
            AbilityNames.RESOLVE_MASTERPIECE_OVERPAY,
        ],
        MerchantGuild => symbols![AbilityNames.MERCHANT_GUILD],
        Plaza => symbols![
            QuestionIds.PLAZA_DISCARD,
            QuestionTypes.DISCARD,
            DeclineButtonIds.PLAZA_NO_DISCARD,
        ],
        Remake => symbols![
            QuestionIds.REMAKE_TRASH_FIRST,
            QuestionIds.REMAKE_TRASH_SECOND,
            QuestionIds.REMAKE_GAIN_FIRST,
            QuestionIds.REMAKE_GAIN_SECOND,
            QuestionTypes.TRASH,
            QuestionTypes.GAIN,
        ],
        Shop => symbols![QuestionIds.SHOP, QuestionTypes.PLAY],
        Stonemason => symbols![
            QuestionIds.STONEMASON_OVERPAY_AMOUNT,
            QuestionIds.STONEMASON_TRASH,
            QuestionIds.STONEMASON_GAIN1,
            QuestionIds.STONEMASON_GAIN2,
            QuestionIds.STONEMASON_GAIN_EQUAL,
            QuestionTypes.OVERPAY,
            QuestionTypes.TRASH,
            QuestionTypes.GAIN,
            AbilityNames.RESOLVE_STONEMASON_OVERPAY,
        ],
        Taxman => symbols![
            QuestionIds.TAXMAN_TRASH,
            QuestionIds.TAXMAN_GAIN,
            QuestionTypes.TRASH,
            QuestionTypes.GAIN,
        ],
        Tournament => symbols![
            QuestionIds.TOURNAMENT_REVEAL,
            QuestionIds.TOURNAMENT_GAIN_PRIZE_OR_DUCHY,
            QuestionTypes.REVEAL,
            QuestionTypes.GAIN_PRIZE,
            DeclineButtonIds.TOURNAMENT_DONT_REVEAL,
        ],
        YoungWitch => symbols![
            QuestionIds.YOUNG_WITCH,
            QuestionIds.YOUNG_WITCH_BANE,
            QuestionTypes.DISCARD,
            QuestionTypes.REVEAL,
            DeclineButtonIds.YOUNG_WITCH_DONT_REVEAL_BANE,
        ],
        _ => &[],
    }
}

/// The features a card needs (see `profile::requirements`); `None` until reviewed.
pub(super) fn requirements(card: Card) -> Option<&'static [&'static str]> {
    use Card::*;
    Some(match card {
        // Its on-gain "set aside a card to play next turn" has no ability name to offer.
        Farmhands => return None,
        Carnival | Fairgrounds | FarmingVillage | FortuneTeller | Harvest | HuntingParty
        | Menagerie | Soothsayer | BagOfGold | Diadem | Demesne | Housecarl => &[],
        Advisor => &["looking", "card-selection"],
        Butcher => &[
            "card-selection",
            "pile-selection",
            "explicit-cost-bound",
            "coffers",
        ],
        Coronet => &["optional-play", "repeated-play", "card-selection"],
        Courser | TrustySteed => &["semantic-modes"],
        Doctor => &[
            "overpay",
            "name-card",
            "looking",
            "ordered-cards",
            "semantic-modes",
        ],
        Farrier | Masterpiece => &["overpay"],
        Ferryman | Hamlet => &["card-selection"],
        Followers => &["off-turn-attack", "card-selection"],
        Footpad => &["off-turn-attack", "card-selection", "coffers"],
        Herald => &[
            "overpay",
            "discard-inspection",
            "card-selection",
            "ordered-cards",
        ],
        HornOfPlenty => &["pile-selection", "explicit-cost-bound"],
        HorseTraders => &["card-selection", "reaction-ability", "duration-ability"],
        HugeTurnip | MerchantGuild => &["coffers"],
        Infirmary => &["overpay", "card-selection", "repeated-play"],
        Jester => &["off-turn-attack", "semantic-modes", "visible-reference"],
        Journeyman => &["name-card"],
        Joust => &["set-aside", "card-selection", "pile-selection"],
        Plaza => &["card-selection", "coffers"],
        Princess | Renown => &["passive-lasting", "dynamic-cost"],
        Remake => &["card-selection", "pile-selection", "explicit-cost-bound"],
        Shop => &["optional-play", "card-selection"],
        Stonemason => &[
            "overpay",
            "card-selection",
            "pile-selection",
            "explicit-cost-bound",
        ],
        Taxman => &[
            "off-turn-attack",
            "card-selection",
            "pile-selection",
            "explicit-cost-bound",
        ],
        Tournament => &["optional-effect", "pile-selection"],
        YoungWitch => &["off-turn-attack", "card-selection", "optional-effect"],
        _ => return None,
    })
}
