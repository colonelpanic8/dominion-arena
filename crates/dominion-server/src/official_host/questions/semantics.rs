use super::{ProjectionError, WireCatalog};
use dominion_engine::{Card, Decision, DecisionKind, Purpose, Trigger, View};
use dominion_online::questions::{CardFrequency, LogArgument};

#[path = "semantics/adventures.rs"]
mod adventures;
#[path = "semantics/alchemy.rs"]
mod alchemy;
#[path = "semantics/allies.rs"]
mod allies;
#[path = "semantics/cornucopia_guilds.rs"]
mod cornucopia_guilds;
#[path = "semantics/dark_ages.rs"]
mod dark_ages;
#[path = "semantics/empires.rs"]
mod empires;
#[path = "semantics/hinterlands.rs"]
mod hinterlands;
#[path = "semantics/menagerie.rs"]
mod menagerie;
#[path = "semantics/nocturne.rs"]
mod nocturne;
#[path = "semantics/plunder.rs"]
mod plunder;
#[path = "semantics/promos.rs"]
mod promos;
#[path = "semantics/prosperity.rs"]
mod prosperity;
#[path = "semantics/renaissance.rs"]
mod renaissance;
#[path = "semantics/rising_sun.rs"]
mod rising_sun;
#[path = "semantics/seaside.rs"]
mod seaside;

pub(super) type Binding = (&'static str, &'static str, Vec<LogArgument>);

/// Each set's own bindings, tried after the shared ones below.
struct SetBindings {
    binding: fn(&View<'_>, &Decision, &WireCatalog) -> Option<Result<Binding, ProjectionError>>,
    mode: fn(Card, u8) -> Option<&'static str>,
    ability: fn(Card, u8) -> Option<&'static str>,
    symbols: fn(Card) -> &'static [(&'static str, &'static str)],
    requirements: fn(Card) -> Option<&'static [&'static str]>,
}

macro_rules! sets {
    ($($m:ident),*) => {
        const SETS: &[SetBindings] = &[$(SetBindings {
            binding: $m::binding,
            mode: $m::mode,
            ability: $m::ability,
            symbols: $m::symbols,
            requirements: $m::requirements,
        }),*];
    };
}
sets!(
    seaside,
    alchemy,
    prosperity,
    hinterlands,
    promos,
    dark_ages,
    cornucopia_guilds,
    adventures,
    nocturne,
    empires,
    renaissance,
    menagerie,
    allies,
    plunder,
    rising_sun
);

/// How a set presents a choice of options (yes/no, alternatives) `prepare` has no
/// generic shape for.
pub(super) enum ChoiceShape {
    /// Selecting the card answers `yes`; the decline button answers `no`.
    Target {
        card: dominion_engine::CardId,
        yes: dominion_engine::Opt,
        no: dominion_engine::Opt,
        decline: &'static str,
    },
    /// A CardModes symbol per option; an empty selection, if declinable, is `decline`.
    Modes {
        options: Vec<(dominion_engine::Opt, &'static str)>,
        decline: Option<(dominion_engine::Opt, &'static str)>,
    },
    /// A DeclineButtonIds button per option.
    Buttons(Vec<(dominion_engine::Opt, &'static str)>),
    /// A disclosed card per option (one of each name offered).
    Cards(Vec<(dominion_engine::Opt, dominion_engine::CardId)>),
    /// A card name per option.
    Names(Vec<(dominion_engine::Opt, Card)>),
}

pub(super) fn choice_shape(
    view: &View<'_>,
    d: &Decision,
) -> Option<Result<ChoiceShape, ProjectionError>> {
    allies::choice_shape(view, d)
        .or_else(|| plunder::choice_shape(view, d))
        .or_else(|| rising_sun::choice_shape(view, d))
}

/// A card's requirements as a set's module declares them.
pub(crate) fn set_requirements(card: Card) -> Option<&'static [&'static str]> {
    SETS.iter().find_map(|s| (s.requirements)(card))
}

pub(super) fn gap(d: &Decision, feature: &str) -> ProjectionError {
    ProjectionError::Compatibility(super::super::profile::CompatibilityGap {
        feature: "decision-semantic".into(),
        card: d.source.map(|c| c.name().into()),
        detail: format!("{feature} ({:?})", d.purpose),
    })
}

/// Stories are selected by effect purpose and response shape, never display labels.
pub(super) fn binding(
    view: &View<'_>,
    d: &Decision,
    c: &WireCatalog,
) -> Result<(&'static str, &'static str, Vec<LogArgument>), ProjectionError> {
    use Card::*;
    use Purpose as P;
    let ordered = matches!(d.kind, DecisionKind::Cards { ordered: true, .. });
    let choosing = matches!(d.kind, DecisionKind::Choose { .. });
    let (kind, story) = match (d.source, d.purpose) {
        (Some(Cellar), P::Discard) => ("DISCARD", "CELLAR"),
        (Some(Chapel), P::Trash) => ("TRASH", "CHAPEL"),
        (Some(Harbinger), P::TopDeck) => ("TOPDECK", "HARBINGER_TOPDECK"),
        (Some(Vassal), P::Play) => ("PLAY", "VASSAL"),
        (Some(Workshop), P::Gain) => ("GAIN", "WORKSHOP"),
        (Some(Bureaucrat), P::TopDeck) => ("TOPDECK", "BUREAUCRAT"),
        (Some(Militia), P::Discard) => ("DISCARD", "MILITIA"),
        (Some(Moneylender), P::Trash) => ("TRASH", "MONEYLENDER"),
        (Some(Poacher), P::Discard) => ("DISCARD", "POACHER"),
        (Some(Remodel), P::Trash) => ("TRASH", "REMODEL_TRASH"),
        (Some(Remodel), P::Gain) => ("GAIN", "REMODEL_GAIN"),
        (Some(Mine), P::Trash) => ("TRASH", "MINE_TRASH"),
        (Some(Mine), P::Gain) => ("GAIN", "MINE_GAIN"),
        (Some(ThroneRoom), P::Play) => ("PLAY", "THRONE_ROOM"),
        (Some(Bandit), P::Trash) => ("TRASH", "BANDIT"),
        (Some(Library), P::SetAside) => ("CHOOSE_MODE", "LIBRARY"),
        (Some(Sentry), P::Trash) => ("TRASH", "SENTRY_TRASH"),
        (Some(Sentry), P::Discard) => ("DISCARD", "SENTRY_DISCARD"),
        (Some(Sentry), P::TopDeck) if ordered => ("ORDER_CARDS", "SENTRY_TOPDECK"),
        (Some(Artisan), P::Gain) => ("GAIN", "ARTISAN_GAIN"),
        (Some(Artisan), P::TopDeck) => ("TOPDECK", "ARTISAN_TOPDECK"),
        (Some(Baron), P::Discard) => ("DISCARD", "BARON"),
        (Some(Courtier), P::Reveal) => ("REVEAL", "COURTIER_REVEAL"),
        (Some(Courtier), P::Mode) => ("CHOOSE_MODE", "COURTIER_MODE"),
        (Some(Courtyard), P::TopDeck) => ("TOPDECK", "COURTYARD"),
        (Some(Diplomat), P::Discard) => ("DISCARD", "DIPLOMAT"),
        (Some(Ironworks), P::Gain) => ("GAIN", "IRONWORKS"),
        (Some(Lurker), P::Mode | P::Either) => ("CHOOSE_MODE", "LURKER_COMPLEX"),
        (Some(Lurker), P::Trash) => ("TRASH", "LURKER_TRASH"),
        (Some(Lurker), P::Gain) => ("GAIN", "LURKER_GAIN"),
        (Some(Masquerade), P::Pass) => ("PASS", "MASQUERADE_PASS"),
        (Some(Masquerade), P::Trash) => ("TRASH", "MASQUERADE_TRASH"),
        (Some(Mill), P::Discard) => ("DISCARD", "MILL"),
        (Some(MiningVillage), P::Trash) => ("TRASH", "MINING_VILLAGE"),
        (Some(Minion), P::Mode) => ("CHOOSE_MODE", "MINION"),
        (Some(Nobles), P::Mode) => ("CHOOSE_MODE", "NOBLES"),
        (Some(Patrol), P::TopDeck) if ordered => ("ORDER_CARDS", "PATROL"),
        (Some(Pawn), P::Mode) => ("CHOOSE_MODE", "PAWN"),
        (Some(Replace), P::Trash) => ("TRASH", "REPLACE_TRASH"),
        (Some(Replace), P::Gain) => ("GAIN", "REPLACE_GAIN"),
        (Some(SecretPassage), P::Move) if matches!(d.kind, DecisionKind::Number { .. }) => {
            ("LOCATION", "SECRET_PASSAGE_INSERT")
        }
        (Some(SecretPassage), P::Move) => ("INSERT_INTO_DECK", "SECRET_PASSAGE_INSERT"),
        (Some(Steward), P::Mode) => ("CHOOSE_MODE", "STEWARD_MODE"),
        (Some(Steward), P::Trash) => ("TRASH", "STEWARD_TRASH"),
        (Some(Swindler), P::Gain) => ("GAIN", "SWINDLER"),
        (Some(Torturer), P::Either) => ("COMPLEX_OR", "TORTURER_DISCARD_OR_CURSE"),
        (Some(Torturer), P::Discard) => ("DISCARD", "TORTURER_MAY_DISCARD"),
        (Some(TradingPost), P::Trash) => ("TRASH", "TRADING_POST"),
        (Some(Upgrade), P::Trash) => ("TRASH", "UPGRADE_TRASH"),
        (Some(Upgrade), P::Gain) => ("GAIN", "UPGRADE_GAIN"),
        (Some(WishingWell), P::Name) => ("WISH", "WISHING_WELL"),
        (Some(Haven), P::SetAside) => ("SET_ASIDE", "HAVEN"),
        (Some(Island), P::Move) => ("SET_ASIDE", "ISLAND"),
        (Some(NativeVillage), P::Mode) => ("CHOOSE_MODE", "NATIVE_VILLAGE_BOTH"),
        (Some(Recruiter), P::Trash) => ("TRASH", "RECRUITER"),
        (Some(Engineer), P::Gain) => ("GAIN", "ENGINEER_GAIN"),
        (Some(Engineer), P::Trash) if choosing => ("TRASH", "ENGINEER_TRASH"),
        (Some(Cathedral), P::Trash) => ("TRASH", "CATHEDRAL"),
        (Some(Settlers), P::Move) => ("PUT_IN_HAND", "SETTLERS"),
        (Some(BustlingVillage), P::Move) => ("PUT_IN_HAND", "BUSTLING_VILLAGE"),
        _ => {
            return SETS
                .iter()
                .find_map(|s| (s.binding)(view, d, c))
                .unwrap_or_else(|| Err(gap(d, "missing story/shape binding")));
        }
    };
    let mut args = vec![];
    if matches!(
        story,
        "MINE_GAIN" | "REMODEL_GAIN" | "REPLACE_GAIN" | "UPGRADE_GAIN"
    ) {
        let limit = view
            .pending_cost_limit()
            .ok_or_else(|| gap(d, "missing explicit cost bound (old save)"))?;
        args.push(LogArgument::Amount {
            coins: limit.coins.into(),
            potions: limit.potions.into(),
            debt: limit.debt.into(),
        });
    }
    if story == "SWINDLER" {
        let subject = view
            .pending_subject()
            .ok_or_else(|| gap(d, "missing affected player"))?;
        args.push(LogArgument::Player(subject.0.into()));
    }
    if matches!(story, "VASSAL" | "LIBRARY") {
        let card = view
            .pending_reference()
            .and_then(|id| view.card(id))
            .ok_or_else(|| gap(d, "missing visible referenced card"))?;
        args.push(LogArgument::CardNames(vec![CardFrequency {
            name: c.card(card)?,
            frequency: 1,
        }]));
    }
    if story == "NATIVE_VILLAGE_BOTH" {
        let mut frequencies = std::collections::BTreeMap::new();
        for &id in view
            .player(d.player)
            .mat(dominion_engine::Mat::NativeVillage)
            .unwrap_or(&[])
        {
            let card = view
                .card(id)
                .ok_or_else(|| gap(d, "mat identity unavailable"))?;
            *frequencies.entry(c.card(card)?).or_insert(0) += 1;
        }
        args.push(LogArgument::CardNames(
            frequencies
                .into_iter()
                .map(|(name, frequency)| CardFrequency { name, frequency })
                .collect(),
        ));
    }
    Ok((kind, story, args))
}

pub(super) fn mode(card: Card, mode: u8) -> Option<&'static str> {
    use Card::*;
    let choices: &[&str] = match card {
        Pawn => &["PAWN_CARD", "PAWN_ACTION", "PAWN_BUY", "PAWN_COIN"],
        Steward => &["STEWARD_CARDS", "STEWARD_COINS", "STEWARD_TRASH"],
        Minion => &["MINION_COINS", "MINION_NEWHAND"],
        Nobles => &["NOBLES_CARDS", "NOBLES_ACTIONS"],
        Courtier => &[
            "COURTIER_ACTION",
            "COURTIER_BUY",
            "COURTIER_COINS",
            "COURTIER_GOLD",
        ],
        Lurker => &["LURKER_TRASH", "LURKER_GAIN"],
        NativeVillage => &["NATIVE_VILLAGE_SET_ASIDE", "NATIVE_VILLAGE_RETURN"],
        _ => return SETS.iter().find_map(|s| (s.mode)(card, mode)),
    };
    choices.get(usize::from(mode)).copied()
}

pub(super) fn ability(card: Card, ability: u8) -> Option<&'static str> {
    use Card::*;
    if let Some(symbol) = SETS.iter().find_map(|s| (s.ability)(card, ability)) {
        return Some(symbol);
    }
    if ability != 0 {
        return None;
    }
    Some(match card {
        Moat => "MOAT_REACTION",
        Diplomat => "DIPLOMAT_REACTION",
        Haven => "HAVEN_ABILITY",
        Caravan => "CARAVAN_ABILITY",
        Lighthouse => "LIGHTHOUSE_COIN",
        Outpost => "OUTPOST_ABILITY",
        Alchemist => "ALCHEMIST_TOPDECK",
        Cathedral => "CATHEDRAL",
        _ => return None,
    })
}

/// How a set shows a `Choose` decision that isn't all CardModes.
pub(super) fn option_shape(
    view: &View<'_>,
    d: &Decision,
    c: &WireCatalog,
) -> Option<Result<super::OptionShape, ProjectionError>> {
    empires::option_shape(view, d, c)
        .or_else(|| renaissance::option_shape(view, d, c))
        .or_else(|| menagerie::option_shape(view, d, c))
}

/// A set's own decline button for an optional choice ("Don't Crown").
pub(super) fn decline_button(d: &Decision) -> Option<&'static str> {
    empires::decline_button(d)
        .or_else(|| renaissance::decline_button(d))
        .or_else(|| menagerie::decline_button(d))
}

type ArgumentsHook<'a> = dyn Fn() -> Option<Result<Vec<LogArgument>, ProjectionError>> + 'a;

/// The log arguments an ability's description names (its Argument0, ...), by symbol.
pub(super) fn ability_arguments(
    view: &View<'_>,
    d: &Decision,
    t: &Trigger,
    symbol: &str,
    c: &WireCatalog,
) -> Result<Vec<LogArgument>, ProjectionError> {
    use dominion_engine::Set;
    // Sets share ability names (TOPDECK_REACTION): the trigger's own set reads it first.
    let hooks: [(Set, &ArgumentsHook); 6] = [
        (Set::Empires, &|| {
            empires::ability_arguments(view, d, t, symbol, c)
        }),
        (Set::Renaissance, &|| {
            renaissance::ability_arguments(view, d, t, symbol, c)
        }),
        (Set::Menagerie, &|| {
            menagerie::ability_arguments(view, d, t, symbol, c)
        }),
        (Set::Allies, &|| allies::ability_arguments(view, t, c)),
        (Set::Plunder, &|| plunder::ability_arguments(view, t, c)),
        (Set::RisingSun, &|| {
            rising_sun::ability_arguments(view, t, c)
        }),
    ];
    let set = t.card.set();
    hooks
        .iter()
        .filter(|(s, _)| *s == set)
        .chain(hooks.iter().filter(|(s, _)| *s != set))
        .find_map(|(_, hook)| hook())
        .unwrap_or(Ok(vec![]))
}

/// Declared symbol dependencies for admission; evaluated even before the first play.
pub(crate) fn symbols(card: Card) -> &'static [(&'static str, &'static str)] {
    match card {
        Card::Alchemist => &[("AbilityNames", "ALCHEMIST_TOPDECK")],
        Card::Artisan => &[
            ("QuestionIds", "ARTISAN_GAIN"),
            ("QuestionIds", "ARTISAN_TOPDECK"),
            ("QuestionTypes", "GAIN"),
            ("QuestionTypes", "TOPDECK"),
        ],
        Card::Bandit => &[("QuestionIds", "BANDIT"), ("QuestionTypes", "TRASH")],
        Card::Baron => &[("QuestionIds", "BARON"), ("QuestionTypes", "DISCARD")],
        Card::Bureaucrat => &[("QuestionIds", "BUREAUCRAT"), ("QuestionTypes", "TOPDECK")],
        Card::BustlingVillage => &[
            ("QuestionIds", "BUSTLING_VILLAGE"),
            ("QuestionTypes", "PUT_IN_HAND"),
        ],
        Card::Caravan => &[("AbilityNames", "CARAVAN_ABILITY")],
        Card::Cathedral => &[
            ("AbilityNames", "CATHEDRAL"),
            ("QuestionIds", "CATHEDRAL"),
            ("QuestionTypes", "TRASH"),
        ],
        Card::Cellar => &[("QuestionIds", "CELLAR"), ("QuestionTypes", "DISCARD")],
        Card::Chapel => &[("QuestionIds", "CHAPEL"), ("QuestionTypes", "TRASH")],
        Card::Courtier => &[
            ("CardModes", "COURTIER_ACTION"),
            ("CardModes", "COURTIER_BUY"),
            ("CardModes", "COURTIER_COINS"),
            ("CardModes", "COURTIER_GOLD"),
            ("QuestionIds", "COURTIER_MODE"),
            ("QuestionIds", "COURTIER_REVEAL"),
            ("QuestionTypes", "CHOOSE_MODE"),
            ("QuestionTypes", "REVEAL"),
        ],
        Card::Courtyard => &[("QuestionIds", "COURTYARD"), ("QuestionTypes", "TOPDECK")],
        Card::Diplomat => &[
            ("AbilityNames", "DIPLOMAT_REACTION"),
            ("QuestionIds", "DIPLOMAT"),
            ("QuestionTypes", "DISCARD"),
        ],
        Card::Engineer => &[
            ("QuestionIds", "ENGINEER_GAIN"),
            ("QuestionIds", "ENGINEER_TRASH"),
            ("QuestionTypes", "GAIN"),
            ("QuestionTypes", "TRASH"),
        ],
        Card::Harbinger => &[
            ("QuestionIds", "HARBINGER_TOPDECK"),
            ("QuestionTypes", "TOPDECK"),
        ],
        Card::Haven => &[
            ("AbilityNames", "HAVEN_ABILITY"),
            ("QuestionIds", "HAVEN"),
            ("QuestionTypes", "SET_ASIDE"),
        ],
        Card::Ironworks => &[("QuestionIds", "IRONWORKS"), ("QuestionTypes", "GAIN")],
        Card::Island => &[("QuestionIds", "ISLAND"), ("QuestionTypes", "SET_ASIDE")],
        Card::Library => &[("QuestionIds", "LIBRARY"), ("QuestionTypes", "CHOOSE_MODE")],
        Card::Lighthouse => &[("AbilityNames", "LIGHTHOUSE_COIN")],
        Card::Lurker => &[
            ("CardModes", "LURKER_GAIN"),
            ("CardModes", "LURKER_TRASH"),
            ("QuestionIds", "LURKER_COMPLEX"),
            ("QuestionIds", "LURKER_GAIN"),
            ("QuestionIds", "LURKER_TRASH"),
            ("QuestionTypes", "CHOOSE_MODE"),
            ("QuestionTypes", "GAIN"),
            ("QuestionTypes", "TRASH"),
        ],
        Card::Masquerade => &[
            ("QuestionIds", "MASQUERADE_PASS"),
            ("QuestionIds", "MASQUERADE_TRASH"),
            ("QuestionTypes", "PASS"),
            ("QuestionTypes", "TRASH"),
        ],
        Card::Militia => &[("QuestionIds", "MILITIA"), ("QuestionTypes", "DISCARD")],
        Card::Mill => &[("QuestionIds", "MILL"), ("QuestionTypes", "DISCARD")],
        Card::Mine => &[
            ("QuestionIds", "MINE_GAIN"),
            ("QuestionIds", "MINE_TRASH"),
            ("QuestionTypes", "GAIN"),
            ("QuestionTypes", "TRASH"),
        ],
        Card::MiningVillage => &[
            ("QuestionIds", "MINING_VILLAGE"),
            ("QuestionTypes", "TRASH"),
        ],
        Card::Minion => &[
            ("CardModes", "MINION_COINS"),
            ("CardModes", "MINION_NEWHAND"),
            ("QuestionIds", "MINION"),
            ("QuestionTypes", "CHOOSE_MODE"),
        ],
        Card::Moat => &[("AbilityNames", "MOAT_REACTION")],
        Card::Moneylender => &[("QuestionIds", "MONEYLENDER"), ("QuestionTypes", "TRASH")],
        Card::NativeVillage => &[
            ("CardModes", "NATIVE_VILLAGE_RETURN"),
            ("CardModes", "NATIVE_VILLAGE_SET_ASIDE"),
            ("QuestionIds", "NATIVE_VILLAGE_BOTH"),
            ("QuestionTypes", "CHOOSE_MODE"),
        ],
        Card::Nobles => &[
            ("CardModes", "NOBLES_ACTIONS"),
            ("CardModes", "NOBLES_CARDS"),
            ("QuestionIds", "NOBLES"),
            ("QuestionTypes", "CHOOSE_MODE"),
        ],
        Card::Outpost => &[("AbilityNames", "OUTPOST_ABILITY")],
        Card::Patrol => &[("QuestionIds", "PATROL"), ("QuestionTypes", "ORDER_CARDS")],
        Card::Pawn => &[
            ("CardModes", "PAWN_ACTION"),
            ("CardModes", "PAWN_BUY"),
            ("CardModes", "PAWN_CARD"),
            ("CardModes", "PAWN_COIN"),
            ("QuestionIds", "PAWN"),
            ("QuestionTypes", "CHOOSE_MODE"),
        ],
        Card::Poacher => &[("QuestionIds", "POACHER"), ("QuestionTypes", "DISCARD")],
        Card::Recruiter => &[("QuestionIds", "RECRUITER"), ("QuestionTypes", "TRASH")],
        Card::Remodel => &[
            ("QuestionIds", "REMODEL_GAIN"),
            ("QuestionIds", "REMODEL_TRASH"),
            ("QuestionTypes", "GAIN"),
            ("QuestionTypes", "TRASH"),
        ],
        Card::Replace => &[
            ("QuestionIds", "REPLACE_GAIN"),
            ("QuestionIds", "REPLACE_TRASH"),
            ("QuestionTypes", "GAIN"),
            ("QuestionTypes", "TRASH"),
        ],
        Card::SecretPassage => &[
            ("QuestionIds", "SECRET_PASSAGE_INSERT"),
            ("QuestionTypes", "INSERT_INTO_DECK"),
            ("QuestionTypes", "LOCATION"),
        ],
        Card::Sentry => &[
            ("QuestionIds", "SENTRY_DISCARD"),
            ("QuestionIds", "SENTRY_TOPDECK"),
            ("QuestionIds", "SENTRY_TRASH"),
            ("QuestionTypes", "DISCARD"),
            ("QuestionTypes", "ORDER_CARDS"),
            ("QuestionTypes", "TRASH"),
        ],
        Card::Settlers => &[
            ("QuestionIds", "SETTLERS"),
            ("QuestionTypes", "PUT_IN_HAND"),
        ],
        Card::Steward => &[
            ("CardModes", "STEWARD_CARDS"),
            ("CardModes", "STEWARD_COINS"),
            ("CardModes", "STEWARD_TRASH"),
            ("QuestionIds", "STEWARD_MODE"),
            ("QuestionIds", "STEWARD_TRASH"),
            ("QuestionTypes", "CHOOSE_MODE"),
            ("QuestionTypes", "TRASH"),
        ],
        Card::Swindler => &[("QuestionIds", "SWINDLER"), ("QuestionTypes", "GAIN")],
        Card::ThroneRoom => &[("QuestionIds", "THRONE_ROOM"), ("QuestionTypes", "PLAY")],
        Card::Torturer => &[
            ("QuestionIds", "TORTURER_DISCARD_OR_CURSE"),
            ("QuestionIds", "TORTURER_MAY_DISCARD"),
            ("QuestionTypes", "COMPLEX_OR"),
            ("QuestionTypes", "DISCARD"),
        ],
        Card::TradingPost => &[("QuestionIds", "TRADING_POST"), ("QuestionTypes", "TRASH")],
        Card::Upgrade => &[
            ("QuestionIds", "UPGRADE_GAIN"),
            ("QuestionIds", "UPGRADE_TRASH"),
            ("QuestionTypes", "GAIN"),
            ("QuestionTypes", "TRASH"),
        ],
        Card::Vassal => &[("QuestionIds", "VASSAL"), ("QuestionTypes", "PLAY")],
        Card::WishingWell => &[("QuestionIds", "WISHING_WELL"), ("QuestionTypes", "WISH")],
        Card::Workshop => &[("QuestionIds", "WORKSHOP"), ("QuestionTypes", "GAIN")],
        _ => SETS
            .iter()
            .map(|s| (s.symbols)(card))
            .find(|s| !s.is_empty())
            .unwrap_or(&[]),
    }
}
