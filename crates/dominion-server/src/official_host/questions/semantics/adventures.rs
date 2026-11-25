//! Question bindings for Adventures (see `semantics::SETS`).
use super::{Binding, ChoiceShape, ProjectionError, WireCatalog, cost_bound, gap};
use dominion_engine::{
    Card, CardId, Decision, DecisionKind, Mat, Opt, Purpose, Token, Trigger, View,
};
use dominion_online::questions::LogArgument;

/// Whether the deciding player's `token` is on a pile already.
fn placed(view: &View<'_>, d: &Decision, token: Token) -> bool {
    matches!(view.player(d.player).tokens().get(&token), Some(Some(_)))
}

/// The token Teacher is moving (its counter holds the token).
fn teacher_token(view: &View<'_>) -> Option<Token> {
    let n = view.pending_counter()?;
    [
        Token::PlusCard,
        Token::PlusAction,
        Token::PlusBuy,
        Token::PlusCoin,
    ]
    .into_iter()
    .find(|&t| t as i32 == n)
}

pub(super) fn binding(
    view: &View<'_>,
    d: &Decision,
    _c: &WireCatalog,
) -> Option<Result<Binding, ProjectionError>> {
    use Card::*;
    use Purpose as P;
    let ordered = matches!(d.kind, DecisionKind::Cards { ordered: true, .. });
    let counter = view.pending_counter();
    let token_story = |token: Token, new: &'static str, moved: &'static str| {
        if placed(view, d, token) { moved } else { new }
    };
    let (kind, story) = match (d.source?, d.purpose) {
        (Alms, P::Gain) => ("GAIN", "ALMS"),
        (Amulet, P::Mode) => ("CHOOSE_MODE", "AMULET_MODE"),
        (Amulet, P::Trash) => ("TRASH", "AMULET_TRASH"),
        (Artificer, P::Discard) => ("DISCARD", "ARTIFICER_DISCARD"),
        (Artificer, P::Gain) => ("GAIN", "ARTIFICER_GAIN"),
        (Ball, P::Gain) if counter == Some(1) => ("GAIN", "BALL_FIRST"),
        (Ball, P::Gain) => ("GAIN", "BALL_SECOND"),
        (Bonfire, P::Trash) => ("TRASH", "BONFIRE"),
        (Disciple, P::Play) => ("PLAY", "DISCIPLE"),
        (Dungeon, P::Discard) => ("DISCARD", "DUNGEON"),
        (Ferry, P::Other) => ("SELECT_ZONE", "FERRY"),
        (Fugitive, P::Discard) => ("DISCARD", "FUGITIVE"),
        (Gear, P::SetAside) => ("SET_ASIDE", "GEAR"),
        (HauntedWoods, P::TopDeck) if ordered => ("ORDER_CARDS", "HAUNTED_WOODS"),
        (Hero, P::Gain) => ("GAIN", "HERO"),
        (Inheritance, P::SetAside) => ("SET_ASIDE", "INHERITANCE"),
        (LostArts, P::Other) => (
            "SELECT_ZONE",
            token_story(Token::PlusAction, "LOST_ARTS_NEW", "LOST_ARTS_MOVE"),
        ),
        (Messenger, P::Discard) => ("CHOOSE_MODE", "MESSENGER_DISCARD"),
        (Messenger, P::Gain) => ("GAIN", "MESSENGER_GAIN"),
        (Miser, P::Mode) => ("CHOOSE_MODE", "MISER"),
        (Pathfinding, P::Other) => (
            "SELECT_ZONE",
            token_story(Token::PlusCard, "PATHFINDING_NEW", "PATHFINDING_MOVE"),
        ),
        (Pilgrimage, P::Gain) => ("GAIN", "PILGRIMAGE_FIRST"),
        (Plan, P::Other) => (
            "SELECT_ZONE",
            token_story(Token::Trashing, "PLAN_NEW", "PLAN_MOVE"),
        ),
        (Plan, P::Trash) => ("TRASH", "TRASHING_TOKEN"),
        (Quest, P::Either) => ("CHOOSE_MODE", "QUEST_MODE"),
        (Quest, P::Discard) => match d.kind {
            DecisionKind::Cards { min: 1, .. } => ("DISCARD", "QUEST_ATTACK"),
            DecisionKind::Cards { min: 6, .. } => ("DISCARD", "QUEST_6CARDS"),
            // No story says "discard two Curses".
            _ => return None,
        },
        (Ratcatcher, P::Trash) => ("TRASH", "RATCATCHER"),
        (Raze, P::Trash) => ("TRASH", raze_story(view, d)),
        (Raze, P::Move) => ("PUT_IN_HAND", "RAZE_CHOOSE_CARD"),
        (Save, P::SetAside) => ("SET_ASIDE", "SAVE"),
        (ScoutingParty, P::Discard) => ("DISCARD", "SCOUTING_PARTY_DISCARD"),
        (ScoutingParty, P::TopDeck) if ordered => ("ORDER_CARDS", "SCOUTING_PARTY_TOPDECK"),
        (Seaway, P::Gain) => ("GAIN", "SEAWAY"),
        (Soldier, P::Discard) => ("DISCARD", "SOLDIER"),
        (Storyteller, P::Play) => (
            "PLAY",
            match counter {
                Some(0) => "STORYTELLER_FIRST",
                Some(1) => "STORYTELLER_SECOND",
                _ => "STORYTELLER_THIRD",
            },
        ),
        (Teacher, P::Either) => ("CHOOSE_MODE", "TEACHER_CHOOSE_TOKEN"),
        // Their "_MOVE" prompts say "Put your ... token" and their "_NEW" ones "Move".
        (Teacher, P::Other) => {
            let token = teacher_token(view)?;
            let (put, moved) = match token {
                Token::PlusCard => ("TEACHER_PLUS_CARD_MOVE", "TEACHER_PLUS_CARD_NEW"),
                Token::PlusAction => ("TEACHER_PLUS_ACTION_MOVE", "TEACHER_PLUS_ACTION_NEW"),
                Token::PlusBuy => ("TEACHER_PLUS_BUY_MOVE", "TEACHER_PLUS_BUY_NEW"),
                _ => ("TEACHER_PLUS_COIN_MOVE", "TEACHER_PLUS_COIN_NEW"),
            };
            ("SELECT_ZONE", token_story(token, put, moved))
        }
        (Trade, P::Trash) => ("TRASH", "TRADE"),
        (Training, P::Other) => (
            "SELECT_ZONE",
            token_story(Token::PlusCoin, "TRAINING_NEW", "TRAINING_MOVE"),
        ),
        (Transmogrify, P::Trash) => ("TRASH", "TRANSMOGRIFY_TRASH"),
        (Transmogrify, P::Gain) => ("GAIN", "TRANSMOGRIFY_GAIN"),
        _ => return None,
    };
    Some(arguments(view, d, story).map(|args| (kind, story, args)))
}

fn raze_story(view: &View<'_>, d: &Decision) -> &'static str {
    let DecisionKind::Cards { candidates, .. } = &d.kind else {
        return "RAZE_ALL";
    };
    let this = view.pending_source();
    let from_play = this.is_some_and(|id| candidates.contains(&id));
    let from_hand = candidates.iter().any(|&id| Some(id) != this);
    match (from_play, from_hand) {
        (true, true) => "RAZE_ALL",
        (true, false) => "RAZE_CAN_TRASH_FROM_PLAY",
        _ => "RAZE_CAN_TRASH_FROM_HAND",
    }
}

fn arguments(
    view: &View<'_>,
    d: &Decision,
    story: &str,
) -> Result<Vec<LogArgument>, ProjectionError> {
    Ok(match story {
        "ARTIFICER_GAIN" | "TRANSMOGRIFY_GAIN" => vec![cost_bound(view, d)?],
        "MISER" => {
            let coppers = view
                .player(d.player)
                .mat(Mat::Tavern)
                .unwrap_or(&[])
                .iter()
                .filter(|&&id| view.card(id) == Some(Card::Copper))
                .count();
            vec![LogArgument::Amount {
                coins: coppers as i32,
                potions: 0,
                debt: 0,
            }]
        }
        "SCOUTING_PARTY_DISCARD" => {
            let DecisionKind::Cards { min, .. } = d.kind else {
                return Err(gap(d, "Scouting Party without a count"));
            };
            vec![LogArgument::Number(min as i32)]
        }
        _ => vec![],
    })
}

pub(super) fn choice_shape(
    _view: &View<'_>,
    d: &Decision,
) -> Option<Result<ChoiceShape, ProjectionError>> {
    let DecisionKind::Choose { options, .. } = &d.kind else {
        return None;
    };
    match d.source? {
        Card::Messenger if options.as_slice() == [Opt::Yes, Opt::No] => {
            Some(Ok(ChoiceShape::Modes {
                options: vec![(Opt::Yes, "MESSENGER_DISCARD"), (Opt::No, "MESSENGER_KEEP")],
                decline: None,
            }))
        }
        _ => None,
    }
}

pub(super) fn decline_button(d: &Decision) -> Option<&'static str> {
    Some(match d.source? {
        Card::Artificer if d.purpose == Purpose::Discard => "ARTIFICER_DONT",
        _ => return None,
    })
}

pub(super) fn mode(card: Card, mode: u8) -> Option<&'static str> {
    use Card::*;
    let choices: &[&str] = match card {
        Amulet => &["AMULET_COIN", "AMULET_TRASH", "AMULET_SILVER"],
        Miser => &["MISER_COPPER_TO_MAT", "MISER_PROFIT"],
        Quest => &["QUEST_ATTACK", "QUEST_CURSES", "QUEST_CARDS"],
        Teacher => &[
            "TEACHER_CARD",
            "TEACHER_ACTION",
            "TEACHER_BUY",
            "TEACHER_COIN",
        ],
        _ => return None,
    };
    choices.get(usize::from(mode)).copied()
}

pub(super) fn ability(card: Card, ability: u8) -> Option<&'static str> {
    use Card::*;
    Some(match (ability, card) {
        (0, Amulet) => "AMULET",
        (0, CaravanGuard) => "CARAVAN_GUARD_NEXT_TURN",
        (0, Dungeon) => "DUNGEON_ABILITY",
        (0, Gear) => "GEAR_ABILITY",
        (0, HauntedWoods) => "HAUNTED_WOODS_DURATION",
        (0, SwampHag) => "SWAMP_HAG_DURATION",
        (0, BridgeTroll) => "BRIDGE_TROLL_ABILITY",
        (0, Hireling) => "HIRELING",
        (1, Ratcatcher) => "CALL_RATCATCHER",
        (1, Guide) => "CALL_GUIDE",
        (1, Transmogrify) => "CALL_TRANSMOGRIFY",
        (1, Teacher) => "CALL_TEACHER",
        (1, CoinOfTheRealm) => "CALL_COIN",
        (1, RoyalCarriage) => "CALL_ROYAL_CARRIAGE",
        (1, Duplicate) => "CALL_DUPLICATE",
        (2, CaravanGuard) => "CARAVAN_GUARD_REACTION",
        (4, Pathfinding) => "PLUS_CARD_TOKEN",
        (4, LostArts) => "PLUS_ACTION_TOKEN",
        (4, Seaway) => "PLUS_BUY_TOKEN",
        (4, Training) => "PLUS_COIN_TOKEN",
        (4, Champion) => "GIVE_PLUS_ACTIONS",
        (5, LostCity) => "LOST_CITY",
        (5, Messenger) => "MESSENGER_GAIN",
        (6, Page) => "PAGE_EXCHANGE",
        (6, TreasureHunter) => "TREASURE_HUNTER_EXCHANGE",
        (6, Warrior) => "WARRIOR_EXCHANGE",
        (6, Hero) => "HERO_EXCHANGE",
        (6, Peasant) => "PEASANT_EXCHANGE",
        (6, Soldier) => "SOLDIER_EXCHANGE",
        (6, Fugitive) => "FUGITIVE_EXCHANGE",
        (6, Disciple) => "DISCIPLE_EXCHANGE",
        (7, HauntedWoods) => "HAUNTED_WOODS_TOPDECK",
        (8, Plan) => "TRASH_TOKEN_ABILITY",
        (9, TravellingFair) => "TRAVELLING_FAIR_REACTION",
        (10, Save) => "SAVE_RETURN_TO_HAND",
        (11, WineMerchant) => "WINE_MERCHANT_DISCARD",
        _ => return None,
    })
}

pub(super) fn ability_arguments(
    view: &View<'_>,
    t: &Trigger,
    c: &WireCatalog,
) -> Option<Result<Vec<LogArgument>, ProjectionError>> {
    let names = |cards: &[CardId]| -> Result<Vec<LogArgument>, ProjectionError> {
        let mut frequencies = std::collections::BTreeMap::new();
        for &id in cards {
            let card = view
                .card(id)
                .ok_or(ProjectionError::Inconsistent("ability card unseen"))?;
            *frequencies.entry(c.card(card)?).or_insert(0) += 1;
        }
        Ok(vec![LogArgument::CardNames(
            frequencies
                .into_iter()
                .map(
                    |(name, frequency)| dominion_online::questions::CardFrequency {
                        name,
                        frequency,
                    },
                )
                .collect(),
        )])
    };
    let lasting_cards = || {
        view.lasting()
            .into_iter()
            .find(|l| Some(l.id) == t.lasting)
            .map(|l| l.data.cards.clone())
            .unwrap_or_default()
    };
    match (t.card, t.ability) {
        (Card::Duplicate, 1) | (Card::TravellingFair, 9) | (Card::RoyalCarriage, 1) => {
            let index = super::trigger_index(view, t)?;
            Some(match view.pending_occasion_card(index) {
                Some(card) => c.card(card).map(|name| {
                    vec![LogArgument::CardNames(vec![
                        dominion_online::questions::CardFrequency { name, frequency: 1 },
                    ])]
                }),
                None => Err(ProjectionError::Inconsistent("ability card unseen")),
            })
        }
        (Card::Gear, 0) | (Card::Save, 10) => Some(names(&lasting_cards())),
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
        Alms => symbols![QuestionIds.ALMS, QuestionTypes.GAIN],
        Amulet => symbols![
            QuestionIds.AMULET_MODE,
            QuestionIds.AMULET_TRASH,
            QuestionTypes.CHOOSE_MODE,
            QuestionTypes.TRASH,
            CardModes.AMULET_COIN,
            CardModes.AMULET_TRASH,
            CardModes.AMULET_SILVER,
            AbilityNames.AMULET,
        ],
        Artificer => symbols![
            QuestionIds.ARTIFICER_DISCARD,
            QuestionIds.ARTIFICER_GAIN,
            QuestionTypes.DISCARD,
            QuestionTypes.GAIN,
            DeclineButtonIds.ARTIFICER_DONT,
        ],
        Ball => symbols![
            QuestionIds.BALL_FIRST,
            QuestionIds.BALL_SECOND,
            QuestionTypes.GAIN,
        ],
        Bonfire => symbols![QuestionIds.BONFIRE, QuestionTypes.TRASH],
        BridgeTroll => symbols![AbilityNames.BRIDGE_TROLL_ABILITY],
        CaravanGuard => symbols![
            AbilityNames.CARAVAN_GUARD_NEXT_TURN,
            AbilityNames.CARAVAN_GUARD_REACTION,
        ],
        Champion => symbols![AbilityNames.GIVE_PLUS_ACTIONS],
        CoinOfTheRealm => symbols![AbilityNames.CALL_COIN],
        Disciple => symbols![
            QuestionIds.DISCIPLE,
            QuestionTypes.PLAY,
            AbilityNames.DISCIPLE_EXCHANGE,
        ],
        Dungeon => symbols![
            QuestionIds.DUNGEON,
            QuestionTypes.DISCARD,
            AbilityNames.DUNGEON_ABILITY,
        ],
        Duplicate => symbols![AbilityNames.CALL_DUPLICATE],
        Ferry => symbols![QuestionIds.FERRY, QuestionTypes.SELECT_ZONE],
        Fugitive => symbols![
            QuestionIds.FUGITIVE,
            QuestionTypes.DISCARD,
            AbilityNames.FUGITIVE_EXCHANGE,
        ],
        Gear => symbols![
            QuestionIds.GEAR,
            QuestionTypes.SET_ASIDE,
            AbilityNames.GEAR_ABILITY,
        ],
        Guide => symbols![AbilityNames.CALL_GUIDE],
        HauntedWoods => symbols![
            QuestionIds.HAUNTED_WOODS,
            QuestionTypes.ORDER_CARDS,
            AbilityNames.HAUNTED_WOODS_DURATION,
            AbilityNames.HAUNTED_WOODS_TOPDECK,
        ],
        Hero => symbols![
            QuestionIds.HERO,
            QuestionTypes.GAIN,
            AbilityNames.HERO_EXCHANGE,
        ],
        Hireling => symbols![AbilityNames.HIRELING],
        Inheritance => symbols![QuestionIds.INHERITANCE, QuestionTypes.SET_ASIDE],
        LostArts => symbols![
            QuestionIds.LOST_ARTS_NEW,
            QuestionIds.LOST_ARTS_MOVE,
            QuestionTypes.SELECT_ZONE,
            AbilityNames.PLUS_ACTION_TOKEN,
        ],
        LostCity => symbols![AbilityNames.LOST_CITY],
        Messenger => symbols![
            QuestionIds.MESSENGER_DISCARD,
            QuestionIds.MESSENGER_GAIN,
            QuestionTypes.CHOOSE_MODE,
            QuestionTypes.GAIN,
            CardModes.MESSENGER_DISCARD,
            CardModes.MESSENGER_KEEP,
            AbilityNames.MESSENGER_GAIN,
        ],
        Miser => symbols![
            QuestionIds.MISER,
            QuestionTypes.CHOOSE_MODE,
            CardModes.MISER_COPPER_TO_MAT,
            CardModes.MISER_PROFIT,
        ],
        Page => symbols![AbilityNames.PAGE_EXCHANGE],
        Pathfinding => symbols![
            QuestionIds.PATHFINDING_NEW,
            QuestionIds.PATHFINDING_MOVE,
            QuestionTypes.SELECT_ZONE,
            AbilityNames.PLUS_CARD_TOKEN,
        ],
        Peasant => symbols![AbilityNames.PEASANT_EXCHANGE],
        Pilgrimage => symbols![QuestionIds.PILGRIMAGE_FIRST, QuestionTypes.GAIN],
        Plan => symbols![
            QuestionIds.PLAN_NEW,
            QuestionIds.PLAN_MOVE,
            QuestionIds.TRASHING_TOKEN,
            QuestionTypes.SELECT_ZONE,
            QuestionTypes.TRASH,
            AbilityNames.TRASH_TOKEN_ABILITY,
        ],
        Quest => symbols![
            QuestionIds.QUEST_MODE,
            QuestionIds.QUEST_ATTACK,
            QuestionIds.QUEST_6CARDS,
            QuestionTypes.CHOOSE_MODE,
            QuestionTypes.DISCARD,
            CardModes.QUEST_ATTACK,
            CardModes.QUEST_CURSES,
            CardModes.QUEST_CARDS,
        ],
        Ratcatcher => symbols![
            QuestionIds.RATCATCHER,
            QuestionTypes.TRASH,
            AbilityNames.CALL_RATCATCHER,
        ],
        Raze => symbols![
            QuestionIds.RAZE_ALL,
            QuestionIds.RAZE_CAN_TRASH_FROM_PLAY,
            QuestionIds.RAZE_CAN_TRASH_FROM_HAND,
            QuestionIds.RAZE_CHOOSE_CARD,
            QuestionTypes.TRASH,
            QuestionTypes.PUT_IN_HAND,
        ],
        RoyalCarriage => symbols![AbilityNames.CALL_ROYAL_CARRIAGE],
        Save => symbols![
            QuestionIds.SAVE,
            QuestionTypes.SET_ASIDE,
            AbilityNames.SAVE_RETURN_TO_HAND,
        ],
        ScoutingParty => symbols![
            QuestionIds.SCOUTING_PARTY_DISCARD,
            QuestionIds.SCOUTING_PARTY_TOPDECK,
            QuestionTypes.DISCARD,
            QuestionTypes.ORDER_CARDS,
        ],
        Seaway => symbols![
            QuestionIds.SEAWAY,
            QuestionTypes.GAIN,
            AbilityNames.PLUS_BUY_TOKEN,
        ],
        Soldier => symbols![
            QuestionIds.SOLDIER,
            QuestionTypes.DISCARD,
            AbilityNames.SOLDIER_EXCHANGE,
        ],
        Storyteller => symbols![
            QuestionIds.STORYTELLER_FIRST,
            QuestionIds.STORYTELLER_SECOND,
            QuestionIds.STORYTELLER_THIRD,
            QuestionTypes.PLAY,
        ],
        SwampHag => symbols![AbilityNames.SWAMP_HAG_DURATION],
        Teacher => symbols![
            QuestionIds.TEACHER_CHOOSE_TOKEN,
            QuestionIds.TEACHER_PLUS_CARD_MOVE,
            QuestionIds.TEACHER_PLUS_CARD_NEW,
            QuestionIds.TEACHER_PLUS_ACTION_MOVE,
            QuestionIds.TEACHER_PLUS_ACTION_NEW,
            QuestionIds.TEACHER_PLUS_BUY_MOVE,
            QuestionIds.TEACHER_PLUS_BUY_NEW,
            QuestionIds.TEACHER_PLUS_COIN_MOVE,
            QuestionIds.TEACHER_PLUS_COIN_NEW,
            QuestionTypes.CHOOSE_MODE,
            QuestionTypes.SELECT_ZONE,
            CardModes.TEACHER_CARD,
            CardModes.TEACHER_ACTION,
            CardModes.TEACHER_BUY,
            CardModes.TEACHER_COIN,
            AbilityNames.CALL_TEACHER,
        ],
        Trade => symbols![QuestionIds.TRADE, QuestionTypes.TRASH],
        Training => symbols![
            QuestionIds.TRAINING_NEW,
            QuestionIds.TRAINING_MOVE,
            QuestionTypes.SELECT_ZONE,
            AbilityNames.PLUS_COIN_TOKEN,
        ],
        Transmogrify => symbols![
            QuestionIds.TRANSMOGRIFY_TRASH,
            QuestionIds.TRANSMOGRIFY_GAIN,
            QuestionTypes.TRASH,
            QuestionTypes.GAIN,
            AbilityNames.CALL_TRANSMOGRIFY,
        ],
        TravellingFair => symbols![AbilityNames.TRAVELLING_FAIR_REACTION],
        TreasureHunter => symbols![AbilityNames.TREASURE_HUNTER_EXCHANGE],
        Warrior => symbols![AbilityNames.WARRIOR_EXCHANGE],
        WineMerchant => symbols![AbilityNames.WINE_MERCHANT_DISCARD],
        _ => &[],
    }
}

/// The features a card needs (see `profile::requirements`); `None` until reviewed.
pub(super) fn requirements(card: Card) -> Option<&'static [&'static str]> {
    use Card::*;
    Some(match card {
        // Haunted Woods: the projection can't order a hand onto the deck yet. Inheritance:
        // its note isn't projected. Quest: no story says "discard two Curses".
        HauntedWoods | Inheritance | Quest => return None,
        Giant | LostCity | Magpie | Port | Ranger | Relic | TreasureTrove | DistantLands | Page
        | TreasureHunter | Warrior | Peasant => &["optional-ability"],
        Hero | Fugitive | Soldier => &["optional-ability", "card-selection", "pile-selection"],
        Champion => &["optional-ability", "duration-ability"],
        Disciple => &["optional-ability", "optional-play", "repeated-play"],
        Teacher => &["optional-ability", "semantic-modes", "pile-selection"],
        Amulet => &[
            "semantic-modes",
            "card-selection",
            "duration-reminder",
            "duration-ability",
        ],
        Artificer => &["card-selection", "pile-selection", "explicit-cost-bound"],
        BridgeTroll | Hireling | SwampHag => &["duration-reminder", "duration-ability"],
        CaravanGuard => &["reaction-ability", "duration-reminder", "duration-ability"],
        CoinOfTheRealm | Duplicate | Guide | WineMerchant => &["optional-ability"],
        Dungeon => &["card-selection", "duration-reminder", "duration-ability"],
        Gear => &[
            "set-aside",
            "card-selection",
            "duration-reminder",
            "duration-ability",
        ],
        Messenger => &["semantic-modes", "pile-selection", "optional-ability"],
        Miser => &["semantic-modes"],
        Ratcatcher => &["optional-ability", "card-selection"],
        Raze => &["looking", "card-selection"],
        RoyalCarriage => &["optional-ability", "repeated-play"],
        Storyteller => &["optional-play", "card-selection"],
        Transmogrify => &[
            "optional-ability",
            "card-selection",
            "pile-selection",
            "explicit-cost-bound",
        ],
        Alms | Ball | Ferry | LostArts | Pathfinding | Seaway | Training => {
            &["event", "pile-selection"]
        }
        Bonfire | Pilgrimage | Plan | Trade => &["event", "card-selection"],
        Borrow | Mission | Raid => &["event"],
        Save => &["event", "set-aside", "card-selection"],
        ScoutingParty => &["event", "looking", "card-selection", "ordered-cards"],
        TravellingFair => &["event", "optional-ability"],
        _ => return None,
    })
}
