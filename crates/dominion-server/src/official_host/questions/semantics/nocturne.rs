//! Question bindings for Nocturne (see `semantics::SETS`).
use super::{Binding, Choice, ProjectionError, SetExtras, WireCatalog, gap};
use dominion_engine::{Card, CardId, Decision, DecisionKind, Opt, Purpose, Trigger, View};
use dominion_online::questions::{CardFrequency, LogArgument};

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
    let yes_no = matches!(&d.kind, DecisionKind::Choose { options, .. }
        if options.as_slice() == [Opt::Yes, Opt::No]);
    let (kind, story) = match (d.source?, d.purpose) {
        (Bat, P::Trash) => ("TRASH", "BAT"),
        (BlessedVillage, P::Either) => ("CHOOSE_MODE", "BLESSED_VILLAGE_MODE"),
        (Cemetery, P::Trash) => ("TRASH", "CEMETERY"),
        (Changeling, P::Gain) => ("GAIN", "CHANGELING_GAIN_COPY"),
        (Cobbler, P::Gain) => ("GAIN", "COBBLER"),
        (Conclave, P::Play) => ("PLAY", "CONCLAVE"),
        (Crypt, P::SetAside) => ("SET_ASIDE", "CRYPT_SET_ASIDE"),
        (Crypt, P::Move) => ("PUT_IN_HAND", "CRYPT_RETURN_TO_HAND"),
        (DevilsWorkshop, P::Gain) => ("GAIN", "DEVILS_WORKSHOP"),
        (Druid, P::Either) => ("RECEIVE", "DRUID"),
        (Exorcist, P::Trash) => ("TRASH", "EXORCIST_TRASH"),
        (Exorcist, P::Gain) => ("GAIN", "EXORCIST_GAIN"),
        (Fool, P::Either) => ("RECEIVE", "FOOL"),
        (Goat, P::Trash) => ("TRASH", "GOAT"),
        (HauntedMirror, P::Discard) => ("DISCARD", "HAUNTED_MIRROR"),
        (Imp, P::Play) => ("PLAY", "IMP"),
        (Monastery, P::Trash) => ("TRASH", "MONASTERY"),
        (Necromancer, P::Play) => ("PLAY", "NECROMANCER"),
        (NightWatchman, P::Discard) => ("DISCARD", "NIGHT_WATCHMAN_DISCARD"),
        (NightWatchman, P::TopDeck) if ordered => ("ORDER_CARDS", "NIGHT_WATCHMAN_TOPDECK"),
        (Pixie, P::Trash) => ("CHOOSE_MODE", "PIXIE_MODE"),
        (Pooka, P::Trash) => ("TRASH", "POOKA"),
        (Raider, P::Discard) => ("DISCARD", "RAIDER"),
        (SacredGrove, P::Other) => ("CHOOSE_MODE", "SACRED_GROVE_MODE"),
        (SecretCave, P::Discard) if yes_no => ("COMPLEX_OR", "SECRET_CAVE"),
        (SecretCave, P::Discard) => ("DISCARD", "SECRET_CAVE"),
        (Shepherd, P::Discard) => ("DISCARD", "SHEPHERD"),
        (TragicHero, P::Gain) => ("GAIN", "TRAGIC_HERO"),
        (Vampire, P::Gain) => ("GAIN", "VAMPIRE"),
        (Wish, P::Gain) => ("GAIN", "WISH"),
        (ZombieApprentice, P::Trash) => ("TRASH", "ZOMBIE_APPRENTICE"),
        (ZombieMason, P::Gain) => ("GAIN", "ZOMBIE_MASON"),
        (ZombieSpy, P::Discard) => ("CHOOSE_MODE", "ZOMBIE_SPY"),
        // Boons and Hexes.
        (TheEarthsGift, P::Discard) => ("DISCARD", "THE_EARTHS_GIFT_DISCARD"),
        (TheEarthsGift, P::Gain) => ("GAIN", "THE_EARTHS_GIFT_GAIN"),
        (TheFlamesGift, P::Trash) => ("TRASH", "THE_FLAMES_GIFT"),
        (TheMoonsGift, P::TopDeck) => ("TOPDECK", "THE_MOONS_GIFT"),
        (TheSkysGift, P::Discard) if yes_no => ("COMPLEX_OR", "THE_SKYS_GIFT"),
        (TheSkysGift, P::Discard) => ("DISCARD", "THE_SKYS_GIFT"),
        (TheSunsGift, P::Discard) => ("DISCARD", "THE_SUNS_GIFT_DISCARD"),
        (TheSunsGift, P::TopDeck) if ordered => ("ORDER_CARDS", "THE_SUNS_GIFT_TOPDECK"),
        (TheWindsGift, P::Discard) => ("DISCARD", "THE_WINDS_GIFT"),
        (Fear, P::Discard) => ("DISCARD", "FEAR_DISCARD"),
        (Haunting, P::TopDeck) => ("TOPDECK", "HAUNTING"),
        (Locusts, P::Gain) => ("GAIN", "LOCUSTS"),
        (Poverty, P::Discard) => ("DISCARD", "POVERTY"),
        (LostInTheWoods, P::Discard) => ("DISCARD", "LOST_IN_THE_WOODS"),
        _ => return None,
    };
    Some(arguments(view, d, c, story).map(|args| (kind, story, args)))
}

fn named(c: &WireCatalog, card: Card) -> Result<LogArgument, ProjectionError> {
    Ok(LogArgument::CardNames(vec![CardFrequency {
        name: c.card(card)?,
        frequency: 1,
    }]))
}

fn arguments(
    view: &View<'_>,
    d: &Decision,
    c: &WireCatalog,
    story: &str,
) -> Result<Vec<LogArgument>, ProjectionError> {
    Ok(match story {
        "BLESSED_VILLAGE_MODE" | "SACRED_GROVE_MODE" => {
            let boon = view.pending_named().ok_or_else(|| gap(d, "missing Boon"))?;
            vec![named(c, boon)?]
        }
        "MONASTERY" => {
            let left = view
                .pending_counter()
                .ok_or_else(|| gap(d, "missing Monastery count"))?;
            vec![LogArgument::Number(left)]
        }
        _ => vec![],
    })
}

fn choice(_view: &View<'_>, d: &Decision) -> Option<Choice> {
    let DecisionKind::Choose { options, .. } = &d.kind else {
        return None;
    };
    let yes_no = options.as_slice() == [Opt::Yes, Opt::No];
    match d.source? {
        Card::Druid | Card::Fool => options
            .iter()
            .map(|o| match o {
                Opt::Card(boon) => Some(*boon),
                _ => None,
            })
            .collect::<Option<Vec<_>>>()
            .map(Choice::Names),
        Card::Pixie if yes_no => Some(Choice::Modes(vec!["PIXIE_TRASH", "PIXIE_KEEP"])),
        Card::SacredGrove if yes_no => Some(Choice::Modes(vec![
            "SACRED_GROVE_RECEIVE",
            "SACRED_GROVE_NO",
        ])),
        Card::ZombieSpy if yes_no => Some(Choice::Modes(vec![
            "ZOMBIE_SPY_DISCARD",
            "ZOMBIE_SPY_TOPDECK",
        ])),
        Card::SecretCave | Card::TheSkysGift if yes_no => {
            Some(Choice::Buttons(vec!["CONTINUE", "DONT_DISCARD"]))
        }
        _ => None,
    }
}

fn decline(d: &Decision) -> Option<&'static str> {
    Some(match d.source? {
        Card::Conclave => "CONCLAVE_DONT",
        Card::Imp => "IMP_DONT",
        Card::Necromancer => "NECROMANCER_DONT",
        _ => return None,
    })
}

pub(super) fn mode(card: Card, mode: u8) -> Option<&'static str> {
    let choices: &[&str] = match card {
        Card::BlessedVillage => &["BLESSED_VILLAGE_NOW", "BLESSED_VILLAGE_LATER"],
        _ => return None,
    };
    choices.get(usize::from(mode)).copied()
}

pub(super) fn ability(card: Card, ability: u8) -> Option<&'static str> {
    use Card::*;
    Some(match (ability, card) {
        (0, Ghost) => "GHOST",
        (0, Cobbler) => "COBBLER_ABILITY",
        (0, Crypt) => "CRYPT_ABILITY",
        (0, DenOfSin) => "DEN_OF_SIN_ABILITY",
        (0, GhostTown) => "GHOST_TOWN_ABILITY",
        (0, Guardian) => "GUARDIAN_COIN",
        (0, Raider) => "RAIDER_ABILITY",
        (0, SecretCave) => "SECRET_CAVE",
        (0, BlessedVillage) => "BLESSED_VILLAGE",
        (1, BlessedVillage) => "RECEIVE_BOON",
        (0, Cemetery) => "CEMETERY",
        (0, CursedVillage) => "RECEIVE_HEX",
        (0, Changeling) => "CHANGELING_MAY_EXCHANGE",
        (0, Tracker) => "TOPDECK_REACTION",
        (0, FaithfulHound) => "FAITHFUL_HOUND",
        (1, FaithfulHound) => "FAITHFUL_HOUND_RETURN",
        (0, HauntedMirror) => "HAUNTED_MIRROR",
        (0, Necromancer) => "REMOVE_FLIPPED_STATUS",
        (0, TheRiversGift) => "DRAW_INSTRUCTIONS",
        (0, LostInTheWoods) => "LOST_IN_THE_WOODS",
        (0, Deluded) => "DELUDED_RETURNED",
        (0, Envious) => "ENVIOUS_RETURNED",
        _ => return None,
    })
}

fn ability_arguments(
    view: &View<'_>,
    t: &Trigger,
    index: usize,
    c: &WireCatalog,
) -> Option<Result<Vec<LogArgument>, ProjectionError>> {
    let lasting = || {
        view.lasting()
            .into_iter()
            .find(|l| Some(l.id) == t.lasting)
            .map(|l| (l.data.cards.clone(), l.data.pick, l.data.card))
    };
    let cards = |ids: Vec<CardId>| -> Result<Vec<LogArgument>, ProjectionError> {
        let mut frequencies = std::collections::BTreeMap::new();
        for id in ids {
            let card = view
                .card(id)
                .ok_or(ProjectionError::Inconsistent("ability card unseen"))?;
            *frequencies.entry(c.card(card)?).or_insert(0) += 1;
        }
        Ok(vec![LogArgument::CardNames(
            frequencies
                .into_iter()
                .map(|(name, frequency)| CardFrequency { name, frequency })
                .collect(),
        )])
    };
    match (t.card, t.ability) {
        (Card::Changeling, 0) | (Card::Tracker, 0) => {
            Some(match view.pending_occasion_card(index) {
                Some(card) => named(c, card).map(|a| vec![a]),
                None => Err(ProjectionError::Inconsistent("ability card unseen")),
            })
        }
        (Card::Ghost, 0) | (Card::Crypt, 0) => {
            let (mut held, pick, _) = lasting().unwrap_or_default();
            held.extend(pick);
            Some(cards(held))
        }
        (Card::BlessedVillage, 1) => Some(match lasting().and_then(|l| l.2) {
            Some(boon) => named(c, boon).map(|a| vec![a]),
            None => Err(ProjectionError::Inconsistent(
                "Blessed Village without its Boon",
            )),
        }),
        (Card::TheRiversGift, 0) => Some(Ok(vec![LogArgument::Number(1)])),
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
        Bat => symbols![QuestionIds.BAT, QuestionTypes.TRASH],
        BlessedVillage => symbols![
            QuestionIds.BLESSED_VILLAGE_MODE,
            QuestionTypes.CHOOSE_MODE,
            CardModes.BLESSED_VILLAGE_NOW,
            CardModes.BLESSED_VILLAGE_LATER,
            AbilityNames.BLESSED_VILLAGE,
            AbilityNames.RECEIVE_BOON,
        ],
        Cemetery => symbols![
            QuestionIds.CEMETERY,
            QuestionTypes.TRASH,
            AbilityNames.CEMETERY,
        ],
        Changeling => symbols![
            QuestionIds.CHANGELING_GAIN_COPY,
            QuestionTypes.GAIN,
            AbilityNames.CHANGELING_MAY_EXCHANGE,
        ],
        Cobbler => symbols![
            QuestionIds.COBBLER,
            QuestionTypes.GAIN,
            AbilityNames.COBBLER_ABILITY,
        ],
        Conclave => symbols![
            QuestionIds.CONCLAVE,
            QuestionTypes.PLAY,
            DeclineButtonIds.CONCLAVE_DONT,
        ],
        Crypt => symbols![
            QuestionIds.CRYPT_SET_ASIDE,
            QuestionIds.CRYPT_RETURN_TO_HAND,
            QuestionTypes.SET_ASIDE,
            QuestionTypes.PUT_IN_HAND,
            AbilityNames.CRYPT_ABILITY,
        ],
        CursedVillage => symbols![AbilityNames.RECEIVE_HEX],
        DenOfSin => symbols![AbilityNames.DEN_OF_SIN_ABILITY],
        DevilsWorkshop => symbols![QuestionIds.DEVILS_WORKSHOP, QuestionTypes.GAIN],
        Druid => symbols![QuestionIds.DRUID, QuestionTypes.RECEIVE],
        Exorcist => symbols![
            QuestionIds.EXORCIST_TRASH,
            QuestionIds.EXORCIST_GAIN,
            QuestionTypes.TRASH,
            QuestionTypes.GAIN,
        ],
        FaithfulHound => symbols![
            AbilityNames.FAITHFUL_HOUND,
            AbilityNames.FAITHFUL_HOUND_RETURN,
        ],
        Fool => symbols![QuestionIds.FOOL, QuestionTypes.RECEIVE],
        Ghost => symbols![AbilityNames.GHOST],
        GhostTown => symbols![AbilityNames.GHOST_TOWN_ABILITY],
        Goat => symbols![QuestionIds.GOAT, QuestionTypes.TRASH],
        Guardian => symbols![AbilityNames.GUARDIAN_COIN],
        HauntedMirror => symbols![
            QuestionIds.HAUNTED_MIRROR,
            QuestionTypes.DISCARD,
            AbilityNames.HAUNTED_MIRROR,
        ],
        Imp => symbols![
            QuestionIds.IMP,
            QuestionTypes.PLAY,
            DeclineButtonIds.IMP_DONT
        ],
        Monastery => symbols![QuestionIds.MONASTERY, QuestionTypes.TRASH],
        Necromancer => symbols![
            QuestionIds.NECROMANCER,
            QuestionTypes.PLAY,
            DeclineButtonIds.NECROMANCER_DONT,
            AbilityNames.REMOVE_FLIPPED_STATUS,
        ],
        NightWatchman => symbols![
            QuestionIds.NIGHT_WATCHMAN_DISCARD,
            QuestionIds.NIGHT_WATCHMAN_TOPDECK,
            QuestionTypes.DISCARD,
            QuestionTypes.ORDER_CARDS,
        ],
        Pixie => symbols![
            QuestionIds.PIXIE_MODE,
            QuestionTypes.CHOOSE_MODE,
            CardModes.PIXIE_TRASH,
            CardModes.PIXIE_KEEP,
        ],
        Pooka => symbols![QuestionIds.POOKA, QuestionTypes.TRASH],
        Raider => symbols![
            QuestionIds.RAIDER,
            QuestionTypes.DISCARD,
            AbilityNames.RAIDER_ABILITY,
        ],
        SacredGrove => symbols![
            QuestionIds.SACRED_GROVE_MODE,
            QuestionTypes.CHOOSE_MODE,
            CardModes.SACRED_GROVE_RECEIVE,
            CardModes.SACRED_GROVE_NO,
        ],
        SecretCave => symbols![
            QuestionIds.SECRET_CAVE,
            QuestionTypes.COMPLEX_OR,
            QuestionTypes.BUTTON,
            QuestionTypes.DISCARD,
            DeclineButtonIds.CONTINUE,
            DeclineButtonIds.DONT_DISCARD,
            AbilityNames.SECRET_CAVE,
        ],
        Shepherd => symbols![QuestionIds.SHEPHERD, QuestionTypes.DISCARD],
        Tracker => symbols![AbilityNames.TOPDECK_REACTION],
        TragicHero => symbols![QuestionIds.TRAGIC_HERO, QuestionTypes.GAIN],
        Vampire => symbols![QuestionIds.VAMPIRE, QuestionTypes.GAIN],
        Wish => symbols![QuestionIds.WISH, QuestionTypes.GAIN],
        ZombieApprentice => symbols![QuestionIds.ZOMBIE_APPRENTICE, QuestionTypes.TRASH],
        ZombieMason => symbols![QuestionIds.ZOMBIE_MASON, QuestionTypes.GAIN],
        ZombieSpy => symbols![
            QuestionIds.ZOMBIE_SPY,
            QuestionTypes.CHOOSE_MODE,
            CardModes.ZOMBIE_SPY_DISCARD,
            CardModes.ZOMBIE_SPY_TOPDECK,
        ],
        TheEarthsGift => symbols![
            QuestionIds.THE_EARTHS_GIFT_DISCARD,
            QuestionIds.THE_EARTHS_GIFT_GAIN,
            QuestionTypes.DISCARD,
            QuestionTypes.GAIN,
        ],
        TheFlamesGift => symbols![QuestionIds.THE_FLAMES_GIFT, QuestionTypes.TRASH],
        TheMoonsGift => symbols![QuestionIds.THE_MOONS_GIFT, QuestionTypes.TOPDECK],
        TheRiversGift => symbols![AbilityNames.DRAW_INSTRUCTIONS],
        TheSkysGift => symbols![
            QuestionIds.THE_SKYS_GIFT,
            QuestionTypes.COMPLEX_OR,
            QuestionTypes.BUTTON,
            QuestionTypes.DISCARD,
            DeclineButtonIds.CONTINUE,
            DeclineButtonIds.DONT_DISCARD,
        ],
        TheSunsGift => symbols![
            QuestionIds.THE_SUNS_GIFT_DISCARD,
            QuestionIds.THE_SUNS_GIFT_TOPDECK,
            QuestionTypes.DISCARD,
            QuestionTypes.ORDER_CARDS,
        ],
        TheWindsGift => symbols![QuestionIds.THE_WINDS_GIFT, QuestionTypes.DISCARD],
        Fear => symbols![QuestionIds.FEAR_DISCARD, QuestionTypes.DISCARD],
        Haunting => symbols![QuestionIds.HAUNTING, QuestionTypes.TOPDECK],
        Locusts => symbols![QuestionIds.LOCUSTS, QuestionTypes.GAIN],
        Poverty => symbols![QuestionIds.POVERTY, QuestionTypes.DISCARD],
        LostInTheWoods => symbols![
            QuestionIds.LOST_IN_THE_WOODS,
            QuestionTypes.DISCARD,
            AbilityNames.LOST_IN_THE_WOODS,
        ],
        Deluded => symbols![AbilityNames.DELUDED_RETURNED],
        Envious => symbols![AbilityNames.ENVIOUS_RETURNED],
        _ => &[],
    }
}

/// The features a card needs (see `profile::requirements`); `None` until reviewed.
pub(super) fn requirements(card: Card) -> Option<&'static [&'static str]> {
    use Card::*;
    Some(match card {
        // Discarded among other cards it isn't on the table to point its ability at.
        FaithfulHound => return None,
        WillOWisp | Pasture | Pouch | CursedGold | LuckyCoin | MagicLamp | TheFieldsGift
        | TheForestsGift | TheMountainsGift | TheRiversGift | TheSeasGift | TheSwampsGift
        | BadOmens | Delusion | Envy | Famine | Greed | Misery | Plague | War | Deluded
        | Envious | Miserable | TwiceMiserable => &[],
        Bard | Idol => &["boons"],
        Leprechaun | Skulk | Tormentor | Werewolf | CursedVillage => &["hexes", "optional-play"],
        Vampire => &["hexes", "pile-selection"],
        BlessedVillage | Pixie | SacredGrove => &["boons", "semantic-modes"],
        Druid | Fool => &["boons", "set-aside", "optional-ability", "card-selection"],
        Tracker => &["boons", "optional-ability"],
        Cemetery | Monastery | Pooka | Shepherd | Bat | Goat | ZombieApprentice => {
            &["card-selection"]
        }
        HauntedMirror | Changeling => &["optional-ability", "card-selection"],
        Cobbler => &["pile-selection", "duration-reminder", "duration-ability"],
        Conclave | Imp => &["optional-play", "card-selection"],
        Crypt => &[
            "set-aside",
            "card-selection",
            "duration-reminder",
            "duration-ability",
        ],
        DenOfSin | GhostTown | Guardian => &["duration-reminder", "duration-ability"],
        DevilsWorkshop | TragicHero | Wish | ZombieMason | Locusts => &["pile-selection"],
        Exorcist => &["card-selection", "pile-selection"],
        Ghost => &["set-aside", "duration-ability", "repeated-play"],
        Necromancer => &["card-selection", "optional-play"],
        NightWatchman | TheSunsGift => &["looking", "card-selection", "ordered-cards"],
        Raider => &[
            "off-turn-attack",
            "card-selection",
            "duration-reminder",
            "duration-ability",
        ],
        SecretCave => &[
            "optional-effect",
            "card-selection",
            "duration-reminder",
            "duration-ability",
        ],
        ZombieSpy => &["semantic-modes", "looking"],
        TheEarthsGift => &["card-selection", "pile-selection"],
        TheFlamesGift | TheWindsGift | Fear | Haunting | Poverty => &["card-selection"],
        TheMoonsGift => &["discard-inspection", "card-selection"],
        TheSkysGift => &["optional-effect", "card-selection"],
        LostInTheWoods => &["optional-ability", "card-selection"],
        _ => return None,
    })
}
