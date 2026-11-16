use super::*;

/// Synthetic data at the independently checked 2.3.7 ordinals; no bundle needed.
pub(crate) fn fixture() -> Arc<Catalog> {
    Arc::new(serde_json::from_str(include_str!("pinned.json")).unwrap())
}

#[test]
fn missing_semantic_symbol_or_card_fails_at_startup() {
    let mut catalog = (*fixture()).clone();
    catalog.enums.get_mut("QuestionIds").unwrap()[24] = "UNRELATED".into();
    assert!(matches!(
        WireCatalog::new(Arc::new(catalog)),
        Err(ProjectionError::MissingSymbol(_))
    ));
    let mut catalog = (*fixture()).clone();
    catalog.cards[31].name = "Unrelated".into();
    let wire = WireCatalog::new(Arc::new(catalog)).unwrap();
    assert!(wire.card(Card::Village).is_err());
}

#[test]
#[ignore = "offline proof requires DOMINION_ONLINE_TEST_BUNDLE"]
fn pinned_bundle_proves_fixture_semantics_and_layout_ordinals() {
    let bytes = std::fs::read(std::env::var("DOMINION_ONLINE_TEST_BUNDLE").unwrap()).unwrap();
    let actual = Arc::new(Catalog::from_bundle(&bytes).unwrap());
    let real = WireCatalog::new(actual).unwrap();
    let synthetic = WireCatalog::new(fixture()).unwrap();
    for &card in Card::ALL {
        assert_eq!(
            real.card(card).ok(),
            synthetic.card(card).ok(),
            "{}",
            card.name()
        );
    }
    for (table, symbols) in REQUIRED_SYMBOLS {
        for symbol in *symbols {
            assert_eq!(
                real.symbol(table, symbol).unwrap(),
                synthetic.symbol(table, symbol).unwrap(),
                "{table}.{symbol}"
            );
        }
    }
    let source = std::str::from_utf8(&bytes).unwrap();
    let names = source
        .split_once(",ZoneNames=[")
        .unwrap()
        .1
        .split_once(']')
        .unwrap()
        .0
        .split(',')
        .collect::<Vec<_>>();
    for (index, name) in [
        (zone::HAND, "HandZone"),
        (zone::DRAW, "DrawZone"),
        (zone::DISCARD, "DiscardZone"),
        (zone::TRASH, "TrashZone"),
        (zone::SUPPLY, "SupplyCardZone"),
        (zone::TREASURE, "BasicTreasureZone"),
        (zone::VICTORY, "BasicVictoryZone"),
        (zone::PLAY, "InPlayZone"),
    ] {
        assert_eq!(names[index as usize], name);
    }
    assert_eq!(real.symbol("TimerPresetOptions", "OFF").unwrap(), TIMER_OFF);
    for (index, name) in [
        (MANUAL_RESIGNATION, "MANUAL"),
        (FORCED_RESIGNATION, "FORCE_RESIGNED"),
    ] {
        assert_eq!(real.symbol("ResignationTypes", name).unwrap(), index);
    }
}

#[test]
#[ignore = "explicit local fixture generation"]
fn export_pinned_catalog() {
    let bytes = std::fs::read(std::env::var("DOMINION_ONLINE_TEST_BUNDLE").unwrap()).unwrap();
    let catalog = Catalog::from_bundle(&bytes).unwrap();
    std::fs::write(
        std::env::var("DOMINION_CATALOG_FIXTURE").unwrap(),
        serde_json::to_vec(&catalog).unwrap(),
    )
    .unwrap();
}
