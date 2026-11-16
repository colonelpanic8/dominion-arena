use dominion_agent::{AgentView, NoteView, SetupRole, Table, glossary_entry, glossary_for, text};
use dominion_engine::{Card, GameConfig, Kind, PlayerId};

fn view(config: GameConfig) -> AgentView {
    Table::new("reference", config, vec![])
        .unwrap()
        .view(PlayerId(0))
}

fn reference(view: &AgentView) -> String {
    let entries = glossary_for(view);
    let rendered = text::reference(view, &entries);
    for entry in entries {
        let prefix = format!("{} |", entry.name);
        assert_eq!(
            rendered
                .lines()
                .filter(|line| line.trim_start().starts_with(&prefix))
                .count(),
            1,
            "{} must be defined exactly once: {rendered}",
            entry.name,
        );
        for line in entry.text.lines() {
            assert!(
                rendered.lines().any(|actual| actual.trim_start() == line),
                "missing official text for {}: {line}",
                entry.name
            );
        }
    }
    rendered
}

fn section<'a>(text: &'a str, start: &str, end: &str) -> &'a str {
    text.split_once(start).unwrap().1.split_once(end).unwrap().0
}

#[test]
fn exclusive_companions_and_pile_members_stay_with_their_source() {
    let v = view(GameConfig::new(
        2,
        vec![
            Card::Page,
            Card::Tournament,
            Card::Joust,
            Card::Augurs,
            Card::Encampment,
            Card::Cemetery,
            Card::Necromancer,
            Card::BorderGuard,
            Card::Treasurer,
            Card::Festival,
        ],
        11,
    ));
    let r = reference(&v);
    let page = section(&r, "--- Page Pile", "--- Tournament Pile");
    for name in ["Page", "Treasure Hunter", "Warrior", "Hero", "Champion"] {
        assert!(page.contains(&format!("{name} |")));
    }
    let tournament = section(&r, "--- Tournament Pile", "--- Joust Pile");
    let joust = section(&r, "--- Joust Pile", "--- Augurs Pile");
    for &card in Card::ALL {
        let group = if card.is(dominion_engine::Types::PRIZE) {
            Some(tournament)
        } else if card.is(dominion_engine::Types::REWARD) {
            Some(joust)
        } else {
            None
        };
        if let Some(group) = group {
            assert!(
                group.contains(&format!("{} |", card.name())),
                "missing {card}"
            );
        }
    }
    let augurs = section(&r, "--- Augurs Pile", "--- Encampment Pile");
    assert!(augurs.contains(
        "rotating; initially top to bottom: 4 Herb Gatherer, 4 Acolyte, 4 Sorceress, 4 Sibyl"
    ));
    for card in Card::Augurs.pile_members() {
        assert!(augurs.contains(&format!("{} |", card.name())));
    }
    let cemetery = section(&r, "--- Cemetery Pile", "--- Necromancer Pile");
    assert!(cemetery.contains("Haunted Mirror |"));
    let necromancer = section(&r, "--- Necromancer Pile", "--- Border Guard Pile");
    for name in ["Zombie Apprentice", "Zombie Mason", "Zombie Spy"] {
        assert!(necromancer.contains(&format!("{name} |")));
    }
    assert!(necromancer.contains("into the trash at setup"));
    let border_guard = section(&r, "--- Border Guard Pile", "--- Treasurer Pile");
    assert!(border_guard.contains("Lantern |"));
    assert!(border_guard.contains("Horn |"));
}

#[test]
fn extra_piles_with_one_source_stay_with_that_source() {
    let v = view(GameConfig::new(
        2,
        vec![Card::Hermit, Card::Vampire, Card::Village],
        11,
    ));
    let r = reference(&v);
    let hermit = section(&r, "--- Hermit Pile", "--- Vampire Pile");
    assert!(hermit.contains("Madman | Action"));
    let vampire = section(&r, "--- Vampire Pile", "Village |");
    assert!(vampire.contains("Bat | Night"));
}

#[test]
fn prize_and_reward_summaries_use_initial_counts_after_piles_empty() {
    for players in [2, 3] {
        let mut v = view(GameConfig::new(
            players,
            vec![Card::Tournament, Card::Joust],
            11,
        ));
        for pile in &mut v.non_supply {
            pile.count = 0;
        }
        let r = reference(&v);
        assert!(r.contains("5 non-Supply Prize piles (1 copy each)"));
        let copies = if players == 2 { "1 copy" } else { "2 copies" };
        assert!(r.contains(&format!("6 non-Supply Reward piles ({copies} each)")));
    }
}

#[test]
fn setup_deck_contents_are_grouped_without_exposing_order() {
    let mut config = GameConfig::new(2, vec![Card::BlackMarket], 11);
    config
        .setup_choices
        .insert(Card::BlackMarket, vec![Card::Village, Card::Moat]);
    let v = view(config);
    let contents: Vec<_> = v
        .notes
        .iter()
        .filter(|note| note.source == "Black Market")
        .collect();
    assert_eq!(contents.len(), 2);
    assert!(
        contents
            .iter()
            .all(|note| note.setup == Some(SetupRole::Deck))
    );
    let r = reference(&v);
    let deck = r
        .split_once("--- Black Market Deck Cards (unordered) ---")
        .unwrap()
        .1;
    assert!(deck.contains("Village |"));
    assert!(deck.contains("Moat |"));
    assert!(deck.find("Moat |").unwrap() < deck.find("Village |").unwrap());
}

#[test]
fn reference_definitions_do_not_introduce_component_sources() {
    let v = view(GameConfig::new(2, vec![Card::Tormentor], 11));
    let mut entries = glossary_for(&v);
    entries.push(glossary_entry(Card::Exorcist));
    let r = text::reference(&v, &entries);
    assert!(r.contains("--- Imp (used by Tormentor) ---"));
    assert!(!r.contains("used by Exorcist"));
}

#[test]
fn only_setup_notes_attach_cards_to_their_source() {
    let mut v = view(GameConfig::new(2, vec![Card::Village, Card::Peasant], 11));
    v.notes.push(NoteView {
        source: "Village".into(),
        player: Some(0),
        card: Some("Peasant".into()),
        pile: None,
        count: 0,
        setup: None,
    });
    let r = reference(&v);
    assert!(!r.contains("--- Village Pile"));
    v.notes.last_mut().unwrap().setup = Some(SetupRole::SetAside);
    let r = reference(&v);
    let village = section(&r, "--- Village Pile", "--- Peasant Pile");
    for name in ["Peasant", "Soldier", "Fugitive", "Disciple", "Teacher"] {
        assert!(village.contains(&format!("{name} |")), "{name}: {village}");
    }
}

#[test]
fn shared_components_list_all_sources_and_are_defined_once() {
    let mut config = GameConfig::new(
        2,
        vec![
            Card::Sleigh,
            Card::Paddock,
            Card::Pickaxe,
            Card::Cutthroat,
            Card::Exorcist,
            Card::Tormentor,
            Card::BlessedVillage,
        ],
        11,
    );
    config.landscapes = vec![Card::Cursed];
    let v = view(config);
    let r = reference(&v);
    for (pile, sources) in [
        ("Horse", &["Paddock", "Sleigh"][..]),
        ("Loot", &["Cursed", "Cutthroat", "Pickaxe"][..]),
        ("Imp", &["Exorcist", "Tormentor"][..]),
    ] {
        let header = r
            .lines()
            .find(|line| line.starts_with(&format!("--- {pile} (used by")))
            .unwrap();
        for source in sources {
            assert!(header.contains(source), "{source}: {header}");
        }
    }
    let entries = glossary_for(&v);
    for kind in [Kind::Boon, Kind::Hex] {
        assert_eq!(
            entries
                .iter()
                .filter(|e| Card::from_name(&e.name).unwrap().kind() == kind)
                .count(),
            12
        );
    }
    for name in ["Deluded", "Envious", "Miserable", "Twice Miserable"] {
        assert!(entries.iter().any(|entry| entry.name == name));
    }
    assert!(
        !entries
            .iter()
            .any(|entry| entry.name == "Lost in the Woods")
    );
}

#[test]
fn shared_pile_sources_follow_dependencies_instead_of_mentions_in_text() {
    let v = view(GameConfig::new(
        2,
        vec![
            Card::HorseTraders,
            Card::Sleigh,
            Card::SecretCave,
            Card::Cemetery,
        ],
        11,
    ));
    let r = reference(&v);
    let horse = r
        .lines()
        .find(|line| line.starts_with("--- Horse (used by"))
        .unwrap();
    assert!(horse.contains("Sleigh"));
    assert!(!horse.contains("Horse Traders"));
    for (pile, sources) in [
        ("Wish", ["Secret Cave", "Magic Lamp"]),
        ("Ghost", ["Cemetery", "Haunted Mirror"]),
    ] {
        let header = r
            .lines()
            .find(|line| line.starts_with(&format!("--- {pile} (used by")))
            .unwrap();
        for source in sources {
            assert!(header.contains(source), "{header}");
        }
    }
}

#[test]
fn randomized_setup_cards_and_landmark_state_stay_with_their_rules() {
    let mut config = GameConfig::new(
        2,
        vec![
            Card::Druid,
            Card::Riverboat,
            Card::YoungWitch,
            Card::Ferryman,
            Card::Augurs,
        ],
        11,
    );
    config.landscapes = vec![
        Card::WayOfTheMouse,
        Card::Labyrinth,
        Card::Obelisk,
        Card::Wall,
    ];
    config.setup_choices.extend([
        (
            Card::Druid,
            vec![Card::TheSeasGift, Card::TheWindsGift, Card::TheFieldsGift],
        ),
        (Card::Riverboat, vec![Card::Market]),
        (Card::YoungWitch, vec![Card::Moat]),
        (Card::Ferryman, vec![Card::Catapult]),
        (Card::WayOfTheMouse, vec![Card::Village]),
        (Card::Obelisk, vec![Card::Augurs]),
    ]);
    let v = view(config);
    for (source, role) in [
        ("Druid", SetupRole::SetAside),
        ("Riverboat", SetupRole::SetAside),
        ("Way of the Mouse", SetupRole::SetAside),
        ("Young Witch", SetupRole::Bane),
        ("Ferryman", SetupRole::ExtraPile),
        ("Obelisk", SetupRole::ChosenPile),
    ] {
        let note = v.notes.iter().find(|note| note.source == source).unwrap();
        assert_eq!(note.setup, Some(role), "{source}");
        let mut legacy = serde_json::to_value(note).unwrap();
        legacy.as_object_mut().unwrap().remove("setup");
        assert_eq!(
            serde_json::from_value::<NoteView>(legacy).unwrap().setup,
            None
        );
    }
    let r = reference(&v);
    let druid = section(&r, "--- Druid Pile", "--- Riverboat Pile");
    for boon in [Card::TheSeasGift, Card::TheWindsGift, Card::TheFieldsGift] {
        assert!(druid.contains(&format!("{} |", boon.name())));
    }
    assert_eq!(
        glossary_for(&v)
            .iter()
            .filter(|entry| Card::from_name(&entry.name).unwrap().kind() == Kind::Boon)
            .count(),
        3
    );
    assert!(!r.contains("=== BOONS"));
    assert!(r.contains("--- Will-o'-Wisp (used by Druid) ---"));
    let riverboat = section(&r, "--- Riverboat Pile", "--- Young Witch Pile");
    assert!(riverboat.contains("set-aside cards: Market"));
    assert!(riverboat.contains("Market |"));
    let ferryman = section(&r, "--- Ferryman Pile", "--- Augurs Pile");
    assert!(ferryman.contains("Catapult: stacked; initially top to bottom: 5 Catapult, 5 Rocks."));
    assert!(ferryman.contains("Catapult |"));
    assert!(ferryman.contains("Rocks |"));
    let mouse = section(&r, "Way of the Mouse |", "Labyrinth |");
    assert!(mouse.contains("set-aside cards: Village"));
    assert!(mouse.contains("Village |"));
    let labyrinth = section(&r, "Labyrinth |", "Obelisk |");
    assert!(
        labyrinth
            .contains("When you gain a second card in one of your turns, take 2 VP from here.")
    );
    assert!(labyrinth.contains("Current: 12 VP remaining"));
    assert!(r.contains("chosen pile: Augurs"));
    let live = text::render(&v);
    assert!(live.contains("\nSUPPLY\n  Copper"));
    assert!(live.contains("\nLANDSCAPES\n  Way of the Mouse"));
    assert!(live.contains("Labyrinth (landmark) 12 VP"));
    assert_eq!(v.you.unwrap().vp, Some(3));
}

#[test]
fn a_traveller_chosen_for_a_way_keeps_its_companions_with_it() {
    let mut config = GameConfig::new(2, vec![Card::Village], 11);
    config.landscapes = vec![Card::WayOfTheMouse];
    config
        .setup_choices
        .insert(Card::WayOfTheMouse, vec![Card::Peasant]);
    let v = view(config);
    let r = reference(&v);
    let mouse = r.split_once("Way of the Mouse |").unwrap().1;
    for name in ["Peasant", "Soldier", "Fugitive", "Disciple", "Teacher"] {
        assert!(mouse.contains(&format!("{name} |")), "{name}: {r}");
    }
}
