use dominion_engine::Card;
use dominion_online::catalog::{CardInfo, Catalog};
use dominion_protocol::Cost;
use std::{collections::BTreeMap, sync::Arc};
pub const KINGDOM: [Card; 10] = [
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
pub fn catalog() -> Arc<Catalog> {
    let mut cards = vec![CardInfo {
        key: "BACK".into(),
        name: "Back".into(),
        cost: Cost::default(),
        types: vec![],
        kingdom: false,
    }];
    for card in [
        Card::Copper,
        Card::Silver,
        Card::Gold,
        Card::Estate,
        Card::Duchy,
        Card::Province,
        Card::Curse,
    ]
    .into_iter()
    .chain(KINGDOM)
    {
        let cost = card.cost().unwrap();
        cards.push(CardInfo {
            key: card.name().replace(' ', "").to_uppercase(),
            name: card.name().into(),
            cost: Cost {
                coins: cost.coins.into(),
                potions: cost.potions.into(),
                debt: cost.debt.into(),
            },
            types: card.types().names().map(String::from).collect(),
            kingdom: KINGDOM.contains(&card),
        });
    }
    let enums = [
        ("QuestionTypes", "PLAY GAIN DISCARD TRASH TRASH_UNIQUE SELECT_ZONE SELECT_TURN RESOLVE_ABILITY ORDER_CARDS BUY CHOOSE_MODE HOW_MUCH_REPAY HOW_MANY_SPEND TOPDECK BOTTOMDECK SET_ASIDE REVEAL PASS COMPLEX_OR COMPLEX_AND"),
        ("QuestionIds", "GAME_ACTION_PHASE GAME_MAY_PLAY_ACTION GAME_BUY_PHASE GAME_MAY_PLAY_TREASURES GAME_MAY_BUY GAME_MAY_START_BUYING CELLAR CHAPEL WORKSHOP"),
        ("DeclineButtonIds", "END_ACTION_PHASE END_BUY_PHASE END_NIGHT_PHASE END_TURN DONT_DISCARD DONT_TRASH DONT_GAIN"),
        ("CounterNames", "ACTIONS COINS POTIONS BUYS POINTS COFFERS DEBT VP_TOKENS PIRATE_STRIKES EMBARGO_TOKENS TRADE_ROUTE_COINS TRADE_ROUTE_MAT VILLAGERS SINISTER_PLOT BLOCKADE_TOKENS FAVORS CARD_TOKENS REMAINING_HAND_PLAYS SUN_TOKENS"),
        ("TokenNames", "PLUS_ACTION PLUS_BUY PLUS_COIN PLUS_CARD MINUS_COST TRASH JOURNEY MINUS_CARD MINUS_COIN"),
        ("ExtraTurnTypes", "REGULAR BETWEEN POSSESSION MISSION OUTPOST FLEET SEIZE_THE_DAY ISLAND_FOLK VOYAGE JOURNEY"),
        ("Types", "Action Treasure Victory Curse"),
        ("ZoneNames", "HandZone DrawZone DiscardZone TrashZone SupplyCardZone BasicTreasureZone BasicVictoryZone SingleLandscapeZone InPlayZone HiddenWindowZone TavernZone NVZone IslandZone NonSupplyZone HiddenWindowZone HiddenWindowZone HiddenWindowZone HiddenWindowZone HiddenWindowZone HiddenWindowZone DruidBoonZone ExileZone MouseZone RevealZone SetAsideZone SkippableRevealZone SetAsideZone SetAsideZone SetAsideZone SkippableRevealZone TheBox SetAsideZone InteractiveNonSupplyZone HiddenWindowZone ReducedLandscapeZone"),
        ("TimerPresetOptions", "RECKLESS HASTY FRIENDLY PATIENT OFF CUSTOM LEAGUE DOC_CHAMPIONSHIP"),
        ("TimeSettingsParameters", "INITIAL_TIMEOUT FREE_DECISION_TIME MAXIMUM_TIME START_OF_GAME_ADDED_TIME START_OF_GAME_MINIMUM_TIME START_OF_MY_TURN_ADDED_TIME START_OF_MY_TURN_MINIMUM_TIME START_OF_BUY_ADDED_TIME START_OF_BUY_MINIMUM_TIME NOT_MY_TURN_ADDED_TIME NOT_MY_TURN_MINIMUM_TIME BUY_DECISION_ADDED_TIME BUY_DECISION_MINIMUM_TIME DEFAULT_DECISION_ADDED_TIME DEFAULT_DECISION_MINIMUM_TIME START_OF_OTHER_TURN_ADDED_TIME START_OF_OTHER_TURN_MINIMUM_TIME LOW_THRESHOLD VERY_LOW_THRESHOLD"),
        ("ResignationTypes", "MANUAL MANUAL_FROM_RECONNECT FORCE_RESIGNED TIMED_OUT"),
        ("LogEntryNames", "EXPLAIN_POINTS_DUH EXPLAIN_POINTS_VP_TOKENS"),
    ].into_iter().map(|(name, entries)| (name.into(), entries.split_whitespace().map(String::from).collect())).collect();
    Arc::new(Catalog {
        cards,
        enums,
        labels: BTreeMap::new(),
        texts: BTreeMap::new(),
        productions: BTreeMap::new(),
    })
}
