//! Declarative capability inventory. Catalog presence alone never admits rules.
use super::catalog::WireCatalog;
use crate::hub;
use dominion_engine::{Card, GameConfig};
use serde::Serialize;

/// Historical fixture kingdom; it is not used for admission.
#[cfg(test)]
pub(crate) const KINGDOM: &[Card] = &[
    Card::Village,
    Card::Smithy,
    Card::Market,
    Card::Festival,
    Card::Laboratory,
    Card::Merchant,
    Card::CouncilRoom,
    Card::Cellar,
    Card::Chapel,
    Card::Workshop,
];
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error, Serialize)]
#[error("unsupported official transport feature: {feature}")]
pub(crate) struct UnsupportedFeature {
    pub(crate) feature: String,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct CompatibilityGap {
    pub feature: String,
    pub card: Option<String>,
    pub detail: String,
}
impl std::fmt::Display for CompatibilityGap {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(card) = &self.card {
            write!(f, "{card}: ")?;
        }
        write!(f, "{}: {}", self.feature, self.detail)
    }
}
impl std::error::Error for CompatibilityGap {}
#[derive(Debug, Clone, Serialize)]
pub(crate) struct CardCoverage {
    pub card: String,
    pub set: String,
    pub kind: String,
    pub kingdom: bool,
    pub engine_unimplemented: bool,
    pub catalog_present: bool,
    pub semantic_mapped: bool,
    pub tested: bool,
    pub original_consumer_tested: bool,
    pub consumer_scenarios: Vec<String>,
    pub removed: bool,
    pub requirements: Vec<String>,
    pub evidence: Vec<String>,
    pub gaps: Vec<CompatibilityGap>,
}
#[derive(Debug, Clone, Serialize)]
pub(crate) struct CompatibilityReport {
    pub protocol: &'static str,
    pub supported: bool,
    pub cards: Vec<CardCoverage>,
    pub gaps: Vec<CompatibilityGap>,
    pub limitations: Vec<&'static str>,
}

fn requirements(card: Card) -> Option<&'static [&'static str]> {
    use Card::*;
    Some(match card {
        Copper | Silver | Gold | Potion | Estate | Duchy | Province | Curse | Platinum | Colony
        | Gardens | Duke | Farm => &[],
        Cellar | Chapel | Moneylender | Poacher | Baron | Courtyard | Diplomat | Masquerade
        | TradingPost => &["card-selection"],
        Recruiter => &["card-selection", "villagers"],
        Harbinger => &["discard-inspection", "card-selection"],
        Vassal => &["optional-play", "visible-reference"],
        Workshop | Ironworks => &["pile-selection"],
        Bureaucrat | Militia | Bandit | Torturer | Swindler => {
            &["off-turn-attack", "card-selection", "pile-selection"]
        }
        Remodel | Mine | Replace | Upgrade => {
            &["card-selection", "pile-selection", "explicit-cost-bound"]
        }
        ThroneRoom => &["repeated-play", "card-selection"],
        Library => &["optional-mode", "set-aside", "visible-reference"],
        Sentry | Patrol => &["looking", "card-selection", "ordered-cards"],
        Artisan => &["pile-selection", "card-selection"],
        Courtier | Lurker | Minion | Nobles | Pawn | Steward => {
            &["semantic-modes", "card-selection", "pile-selection"]
        }
        Mill | MiningVillage | Engineer => &["optional-effect", "card-selection", "pile-selection"],
        SecretPassage => &["card-selection", "location-number"],
        WishingWell => &["name-card"],
        Merchant | Bridge => &["passive-lasting", "dynamic-cost"],
        Moat => &["reaction-ability"],
        CouncilRoom | Witch | ShantyTown | Conspirator => &["public-effects"],
        Haven => &["set-aside", "duration-reminder", "duration-ability"],
        Caravan | Lighthouse => &["duration-reminder", "duration-ability"],
        NativeVillage => &["hidden-mat", "semantic-modes"],
        Island => &["island-mat", "card-selection"],
        Outpost => &["extra-turn", "duration-reminder"],
        Baker => &["coffers"],
        Alchemist => &["potion-cost", "optional-ability"],
        Expedition => &["event", "duration-reminder"],
        Cathedral => &["project", "duration-ability", "card-selection"],
        Settlers | BustlingVillage => &["split-pile", "discard-inspection", "card-selection"],
        _ if vanilla(card) && card.def().plus.coffers > 0 && card.def().plus.villagers > 0 => {
            &["coffers", "villagers"]
        }
        _ if vanilla(card) && card.def().plus.coffers > 0 => &["coffers"],
        _ if vanilla(card) && card.def().plus.villagers > 0 => &["villagers"],
        _ if vanilla(card) => &[],
        _ => return super::questions::semantics::set_requirements(card),
    })
}

/// Only instructions the engine applies directly as printed vanilla bonuses.
fn vanilla(card: Card) -> bool {
    if card.kind() != dominion_engine::Kind::Card || card.def().unimplemented {
        return false;
    }
    if card.types().intersects(
        dominion_engine::Types::LIAISON
            | dominion_engine::Types::OMEN
            | dominion_engine::Types::FATE
            | dominion_engine::Types::DOOM
            | dominion_engine::Types::LOOTER,
    ) {
        return false;
    }
    let plus = card.def().plus;
    let lines = [
        (plus.cards, "Card", "Cards"),
        (plus.actions, "Action", "Actions"),
        (plus.buys, "Buy", "Buys"),
        (plus.villagers, "Villager", "Villagers"),
        (plus.coffers, "Coffers", "Coffers"),
        (plus.favors, "Favor", "Favors"),
    ];
    let mut expected: Vec<String> = lines
        .into_iter()
        .filter(|(n, _, _)| *n > 0)
        .map(|(n, s, p)| format!("+{n} {}", if n == 1 { s } else { p }))
        .collect();
    if plus.coins > 0 {
        expected.push(format!("+${}", plus.coins));
    }
    let actual: Vec<_> = card
        .def()
        .text
        .lines()
        .filter(|l| !l.trim().is_empty())
        .collect();
    !actual.is_empty()
        && actual.len() == expected.len()
        && actual.iter().all(|line| expected.iter().any(|e| e == line))
}

fn feature_symbols(feature: &str) -> &'static [(&'static str, &'static str)] {
    match feature {
        "coffers" => &[
            ("CounterNames", "COFFERS"),
            ("GameButtons", "USE_COFFERS"),
            ("QuestionIds", "GAME_USE_COFFERS"),
            ("QuestionTypes", "BUTTON"),
        ],
        "villagers" => &[
            ("CounterNames", "VILLAGERS"),
            ("GameButtons", "USE_VILLAGER"),
            ("QuestionIds", "GAME_USE_VILLAGER"),
            ("QuestionTypes", "BUTTON"),
        ],
        "reaction-ability" | "optional-ability" => &[
            ("QuestionTypes", "RESOLVE_ABILITY"),
            ("QuestionIds", "GAME_MAY_REACT_WITH"),
            ("DeclineButtonIds", "DONE_REACTING"),
        ],
        "duration-ability" => &[
            ("QuestionTypes", "RESOLVE_ABILITY"),
            ("QuestionIds", "GAME_RESOLVE_ABILITY"),
        ],
        "optional-play" => &[("DeclineButtonIds", "DONT_PLAY")],
        "optional-mode" => &[
            ("CardModes", "LIBRARY_SKIP_IT"),
            ("CardModes", "LIBRARY_DRAW_IT"),
        ],
        "optional-effect" => &[
            ("DeclineButtonIds", "DONT_TRASH"),
            ("DeclineButtonIds", "CONTINUE"),
            ("DeclineButtonIds", "DONT_DISCARD"),
        ],
        "location-number" => &[("QuestionTypes", "LOCATION")],
        "ordered-cards" => &[("QuestionTypes", "ORDER_CARDS")],
        "set-aside" => &[("ZoneNames", "SetAsideZone")],
        "hidden-mat" => &[("ZoneNames", "NVZone")],
        "island-mat" => &[("ZoneNames", "IslandZone")],
        "event" | "project" => &[
            ("ZoneNames", "SingleLandscapeZone"),
            ("QuestionIds", "GAME_MAY_BUY_NONCARDS"),
        ],
        "extra-turn" => &[("ExtraTurnTypes", "OUTPOST")],
        _ => &[],
    }
}
fn verified_consumer_scenarios(card: Card) -> Vec<String> {
    static EVIDENCE: std::sync::OnceLock<serde_json::Value> = std::sync::OnceLock::new();
    let evidence = EVIDENCE.get_or_init(|| {
        serde_json::from_str(include_str!("profile/consumer-evidence.json"))
            .expect("checked consumer evidence manifest")
    });
    if evidence["verified"] != true
        || evidence["bundle_sha256"] != dominion_online::catalog::BODY_SHA256
    {
        return vec![];
    }
    evidence["cards"][card.name()]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|s| s.as_str().map(String::from))
        .collect()
}
fn card_coverage(card: Card, c: &WireCatalog) -> CardCoverage {
    let present = c.card(card).is_ok();
    let declared = requirements(card);
    let mut gaps = vec![];
    if !present {
        gaps.push(CompatibilityGap {
            feature: "catalog-symbol".into(),
            card: Some(card.name().into()),
            detail: "absent from pinned client (version/edition difference)".into(),
        });
    }
    if card.def().unimplemented {
        gaps.push(CompatibilityGap {
            feature: "engine-unimplemented".into(),
            card: Some(card.name().into()),
            detail: "deliberately excluded by engine".into(),
        });
    }
    if declared.is_none() {
        gaps.push(CompatibilityGap {
            feature: "semantic-inventory".into(),
            card: Some(card.name().into()),
            detail: "effect/setup closure not reviewed; not advertised as supported".into(),
        });
    }
    for &(table, symbol) in super::questions::semantics::symbols(card)
        .iter()
        .chain(
            declared
                .unwrap_or(&[])
                .iter()
                .flat_map(|feature| feature_symbols(feature).iter()),
        )
        .chain(match card {
            Card::Recruiter => [
                ("CounterNames", "VILLAGERS"),
                ("GameButtons", "USE_VILLAGER"),
                ("QuestionIds", "GAME_USE_VILLAGER"),
                ("QuestionTypes", "BUTTON"),
            ]
            .as_slice(),
            Card::Engineer => [
                ("CounterNames", "DEBT"),
                ("GameButtons", "PAY_OFF_DEBT"),
                ("QuestionIds", "GAME_PAY_OFF_DEBT"),
                ("QuestionTypes", "BUTTON"),
            ]
            .as_slice(),
            Card::Torturer => [
                ("QuestionIds", "TORTURER_MAY_CURSE"),
                ("QuestionIds", "TORTURER_MAY_DISCARD"),
                ("DeclineButtonIds", "CONTINUE"),
                ("DeclineButtonIds", "TORTURER_GAIN_CURSE"),
            ]
            .as_slice(),
            _ => &[],
        })
    {
        if c.symbol(table, symbol).is_err() {
            gaps.push(CompatibilityGap {
                feature: "semantic-symbol".into(),
                card: Some(card.name().into()),
                detail: format!("{table}.{symbol}"),
            });
        }
    }
    let tested = matches!(
        card.set(),
        dominion_engine::Set::Base | dominion_engine::Set::Intrigue
    ) && !card.def().removed
        || matches!(
            card,
            Card::Haven
                | Card::Caravan
                | Card::Lighthouse
                | Card::NativeVillage
                | Card::Outpost
                | Card::Island
                | Card::Baker
                | Card::Recruiter
                | Card::Engineer
                | Card::Alchemist
                | Card::Expedition
                | Card::Cathedral
                | Card::Settlers
                | Card::BustlingVillage
        );
    let consumer_scenarios = verified_consumer_scenarios(card);
    let original_consumer_tested = !consumer_scenarios.is_empty();
    CardCoverage {
        original_consumer_tested,
        consumer_scenarios,
        card: card.name().into(),
        set: format!("{:?}", card.set()),
        kind: format!("{:?}", card.kind()),
        kingdom: card.def().kingdom,
        engine_unimplemented: card.def().unimplemented,
        catalog_present: present,
        semantic_mapped: declared.is_some(),
        tested,
        removed: card.def().removed,
        requirements: declared
            .unwrap_or(&[])
            .iter()
            .map(|s| s.to_string())
            .collect(),
        evidence: if original_consumer_tested {
            vec!["projection::rules deterministic effect scenarios".into(),"hash-pinned original-service/model/answer scenarios recorded in profile/consumer-evidence.json; selected branches, not all interactions".into()]
        } else if declared.is_some() && tested {
            vec![
                "projection::rules deterministic effect scenarios (not exhaustive interactions)"
                    .into(),
            ]
        } else {
            vec![]
        },
        gaps,
    }
}
fn report(cards: Vec<CardCoverage>, mut gaps: Vec<CompatibilityGap>) -> CompatibilityReport {
    gaps.extend(cards.iter().flat_map(|c| c.gaps.clone()));
    CompatibilityReport {
        protocol: "2.3.7",
        supported: gaps.is_empty(),
        cards,
        gaps,
        limitations: vec![
            "replacement snapshots; historical logs and smooth animations not implemented",
            "catalog/semantic/test evidence are distinct; seeded games do not prove all interactions",
            "unreviewed ancillary setup and expansion state block admission",
        ],
    }
}
pub(crate) fn coverage_report(c: &WireCatalog) -> CompatibilityReport {
    report(
        Card::ALL
            .iter()
            .map(|&card| card_coverage(card, c))
            .collect(),
        vec![],
    )
}
pub(crate) fn assess_setup(
    config: &GameConfig,
    meta: &hub::Meta,
    c: &WireCatalog,
) -> CompatibilityReport {
    let mut gaps = vec![];
    let mut add = |feature: &str| {
        gaps.push(CompatibilityGap {
            feature: feature.into(),
            card: None,
            detail: "not represented by current official transport".into(),
        })
    };
    if !(2..=6).contains(&config.players) {
        add("player-count");
    }
    if meta.rated {
        add("rated-game");
    }
    if meta.timed {
        add("timed-game");
    }
    if config.turn_limit.is_some() {
        add("nonstandard-turn-limit");
    }
    if config.shelters == Some(true) {
        add("shelter-setup");
    }
    if !config.setup_choices.is_empty() {
        add("ancillary-setup-choice-closure");
    }
    let mut cards = config.kingdom.clone();
    cards.extend(&config.landscapes);
    cards.extend([
        Card::Copper,
        Card::Silver,
        Card::Gold,
        Card::Estate,
        Card::Duchy,
        Card::Province,
        Card::Curse,
    ]);
    if config.colonies == Some(true) {
        cards.extend([Card::Platinum, Card::Colony]);
    }
    if config.kingdom.contains(&Card::Alchemist) {
        cards.push(Card::Potion);
    }
    if config.kingdom.contains(&Card::Settlers) {
        cards.push(Card::BustlingVillage);
    }
    for (i, card) in config.kingdom.iter().enumerate() {
        if config.kingdom[..i].contains(card) {
            add("duplicate-kingdom-pile");
        }
    }
    let mut i = 0;
    while i < cards.len() {
        let card = cards[i];
        i += 1;
        let related = card.related();
        let closure = related
            .piles
            .iter()
            .chain(related.artifacts)
            .chain(related.trash)
            .chain(related.states)
            .copied()
            .chain(related.heirloom)
            .chain(card.def().pile);
        for dependency in closure {
            if !cards.contains(&dependency) {
                cards.push(dependency);
            }
        }
        for &dependency in Card::ALL {
            if dependency.def().pile == Some(card) && !cards.contains(&dependency) {
                cards.push(dependency);
            }
        }
        if card.uses_pile(Card::Potion) && !cards.contains(&Card::Potion) {
            cards.push(Card::Potion);
        }
    }
    cards.sort();
    cards.dedup();
    report(
        cards
            .into_iter()
            .map(|card| card_coverage(card, c))
            .collect(),
        gaps,
    )
}
/// `unreviewed` admits cards whose rules no one has mapped onto the transport yet.
pub(crate) fn check_supported(
    config: &GameConfig,
    meta: &hub::Meta,
    c: &WireCatalog,
    unreviewed: bool,
) -> Result<(), UnsupportedFeature> {
    let report = assess_setup(config, meta, c);
    let mut gaps = report
        .gaps
        .iter()
        .filter(|g| !(unreviewed && g.feature == "semantic-inventory"));
    match gaps.next() {
        Some(gap) => Err(UnsupportedFeature {
            feature: format!(
                "{}{}: {}",
                gap.card
                    .as_ref()
                    .map(|c| format!("{c}: "))
                    .unwrap_or_default(),
                gap.feature,
                gap.detail
            ),
        }),
        None => Ok(()),
    }
}
