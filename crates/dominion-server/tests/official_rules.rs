//! Offline admission/capability contract; deterministic effects live in projection::rules.
use dominion_engine::{Card, GameConfig, Set};
use dominion_online::catalog::Catalog;
use dominion_server::{hub::Meta, official_host::Host};
use std::sync::Arc;

fn host() -> Arc<Host> {
    Host::new(Arc::new(
        serde_json::from_str::<Catalog>(include_str!("../src/official_host/catalog/pinned.json"))
            .unwrap(),
    ))
    .unwrap()
}
#[test]
fn every_current_base_intrigue_card_is_admitted_by_declared_capabilities() {
    let host = host();
    let mut n = 0;
    for &card in Card::ALL.iter().filter(|c| {
        matches!(c.set(), Set::Base | Set::Intrigue) && c.def().kingdom && !c.def().removed
    }) {
        let mut cfg = GameConfig::new(3, vec![card], 41);
        cfg.colonies = Some(false);
        cfg.shelters = Some(false);
        let report = host.compatibility_report(&cfg, &Meta::default());
        assert_eq!(report["supported"], true, "{}: {report:#}", card.name());
        n += 1;
    }
    assert_eq!(n, 52);
}
#[test]
fn cross_set_requirements_and_every_engine_card_have_machine_readable_inventory() {
    let host = host();
    for card in [
        Card::Haven,
        Card::Caravan,
        Card::Lighthouse,
        Card::NativeVillage,
        Card::Outpost,
        Card::Island,
        Card::Baker,
        Card::Recruiter,
        Card::Engineer,
        Card::Alchemist,
        Card::Settlers,
        Card::Bazaar,
    ] {
        let mut cfg = GameConfig::new(2, vec![card], 3);
        cfg.shelters = Some(false);
        cfg.colonies = Some(false);
        let report = host.compatibility_report(&cfg, &Meta::default());
        assert_eq!(report["supported"], true, "{}: {report:#}", card.name());
    }
    let report = host.coverage_report();
    let cards = report["cards"].as_array().unwrap();
    assert_eq!(cards.len(), Card::ALL.len());
    assert!(cards.iter().any(|c| c["catalog_present"] == false));
    assert!(cards.iter().any(|c| c["semantic_mapped"] == false));
    assert!(
        cards
            .iter()
            .any(|c| c["semantic_mapped"] == true && c["tested"] == false)
    );
    assert_eq!(report["supported"], false);
    let possession = cards.iter().find(|c| c["card"] == "Possession").unwrap();
    assert!(
        possession["gaps"]
            .as_array()
            .unwrap()
            .iter()
            .any(|g| g["feature"] == "engine-unimplemented")
    );
    if let Ok(path) = std::env::var("DOMINION_COVERAGE_REPORT") {
        std::fs::write(path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    }
}
#[test]
fn missing_semantic_symbol_blocks_setup_before_an_unreachable_branch() {
    let mut catalog: Catalog =
        serde_json::from_str(include_str!("../src/official_host/catalog/pinned.json")).unwrap();
    let modes = catalog.enums.get_mut("CardModes").unwrap();
    let i = modes.iter().position(|s| s == "PAWN_COIN").unwrap();
    modes[i] = "MISSING".into();
    let host = Host::new(Arc::new(catalog)).unwrap();
    let report =
        host.compatibility_report(&GameConfig::new(2, vec![Card::Pawn], 3), &Meta::default());
    assert_eq!(report["supported"], false);
    assert!(
        report["gaps"]
            .as_array()
            .unwrap()
            .iter()
            .any(|g| g["detail"] == "CardModes.PAWN_COIN")
    );
}
