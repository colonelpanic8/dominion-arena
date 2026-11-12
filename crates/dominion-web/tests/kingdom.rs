//! The kingdom builder's randomizer and the request it builds, against the engine's card
//! table and setup.

mod common;

use common::card_db;
use dominion_engine::{Card, GameConfig};
use dominion_web::kingdom::{self, Deal, KINGDOM_SIZE, Options, Rng, Slot, Tray};
use std::collections::BTreeSet;

fn set_of(names: &[&str]) -> BTreeSet<String> {
    names.iter().map(|s| s.to_string()).collect()
}

fn opts() -> Options {
    Options {
        players: 2,
        ..Options::default()
    }
}

#[test]
fn randomizing_keeps_locked_piles_and_deals_distinct_ones_from_the_chosen_sets() {
    let db = card_db();
    let sets = set_of(&["Base", "Seaside"]);
    let pool: Vec<String> = kingdom::kingdom_piles(&db, &sets, false)
        .into_iter()
        .map(|c| c.name.clone())
        .collect();
    let mut rng = Rng::new(7);
    for round in 0..200 {
        let mut tray = Tray::default();
        tray.toggle_pile("Village");
        tray.toggle_pile("Smithy");
        tray.toggle_pile("Haven");
        tray.toggle_pile("Witch");
        tray.toggle_lock(0);
        tray.toggle_lock(2);
        if round % 2 == 0 {
            tray.toggle_lock(3);
        }
        let locked: Vec<(usize, Slot)> = tray
            .kingdom
            .iter()
            .cloned()
            .enumerate()
            .filter(|(_, s)| s.locked)
            .collect();
        tray.randomize(&pool, &mut rng);
        tray.randomize(&pool, &mut rng);
        assert_eq!(tray.kingdom.len(), KINGDOM_SIZE);
        for (i, s) in &locked {
            assert_eq!(&tray.kingdom[*i], s);
        }
        let names: BTreeSet<&str> = tray.kingdom.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names.len(), KINGDOM_SIZE, "a pile was dealt twice");
        for s in tray.kingdom.iter().filter(|s| !s.locked) {
            let card = db.get(&s.name).unwrap();
            assert!(sets.contains(&card.set) && card.kingdom && !card.removed);
        }
    }
}

#[test]
fn randomizing_stops_when_the_sets_run_out() {
    let db = card_db();
    let pool: Vec<String> = kingdom::kingdom_piles(&db, &set_of(&["Promos"]), false)
        .into_iter()
        .map(|c| c.name.clone())
        .take(4)
        .collect();
    let mut tray = Tray::default();
    tray.toggle_pile(&pool[0]);
    tray.toggle_lock(0);
    tray.randomize(&pool, &mut Rng::new(1));
    let names: BTreeSet<&String> = tray.kingdom.iter().map(|s| &s.name).collect();
    assert_eq!(names, pool.iter().collect());
}

#[test]
fn a_tray_deal_never_sends_sets_and_a_server_deal_never_sends_a_kingdom() {
    let mut tray = Tray::default();
    tray.toggle_pile("Village");
    let sets = vec!["Base".to_string(), "Intrigue".to_string()];

    let chosen = kingdom::request(Deal::Tray, &tray, &sets, &opts()).unwrap();
    assert_eq!(chosen.kingdom, ["Village"]);
    assert!(chosen.sets.is_empty());

    let dealt = kingdom::request(Deal::Server, &tray, &sets, &opts()).unwrap();
    assert!(dealt.kingdom.is_empty());
    assert_eq!(dealt.sets, sets);

    let empty = kingdom::request(Deal::Tray, &Tray::default(), &sets, &opts()).unwrap();
    assert!(empty.kingdom.is_empty());
    assert_eq!(empty.sets, sets);

    assert!(kingdom::request(Deal::Server, &tray, &[], &opts()).is_err());
    assert_eq!(
        kingdom::request(Deal::Tray, &tray, &[], &opts())
            .unwrap()
            .names,
        ["P1", "P2"]
    );
}

#[test]
fn a_game_has_one_ally_and_one_prophecy_at_most() {
    let allies: Vec<&str> = Card::ALL
        .iter()
        .filter(|c| c.kind() == dominion_engine::Kind::Ally)
        .take(2)
        .map(|c| c.name())
        .collect();
    let mut tray = Tray::default();
    let db = with_kinds(card_db());
    tray.toggle_landscape(&db, allies[0]);
    tray.toggle_landscape(&db, "Alms");
    tray.toggle_landscape(&db, allies[1]);
    assert_eq!(tray.landscapes, ["Alms", allies[1]]);
    let mut config = GameConfig::new(2, vec![Card::Village], 1);
    config.landscapes = tray
        .landscapes
        .iter()
        .map(|n| Card::from_name(n).unwrap())
        .collect();
    assert!(dominion_engine::Game::new(config).is_ok());
}

/// The shared card table with each landscape's real kind.
fn with_kinds(db: dominion_web::cards::CardDb) -> dominion_web::cards::CardDb {
    let cards = db
        .all()
        .iter()
        .cloned()
        .map(|mut c| {
            if let Some(card) = Card::from_name(&c.name) {
                c.kind = format!("{:?}", card.kind()).to_lowercase();
            }
            c
        })
        .collect();
    dominion_web::cards::CardDb::new(cards)
}
