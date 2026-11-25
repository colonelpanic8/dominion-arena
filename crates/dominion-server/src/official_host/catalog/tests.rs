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
    for (name, index) in ZONE_SYMBOLS {
        assert_eq!(names[*index as usize], *name);
    }
    assert_eq!(real.symbol("TimerPresetOptions", "OFF").unwrap(), TIMER_OFF);
    for (index, name) in [
        (MANUAL_RESIGNATION, "MANUAL"),
        (FORCED_RESIGNATION, "FORCE_RESIGNED"),
        (TIMED_OUT_RESIGNATION, "TIMED_OUT"),
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

#[test]
fn dominion_games_timers_map_to_arena_clocks_and_back() {
    let catalog = WireCatalog::new(fixture()).unwrap();
    let off = catalog.timer(None).unwrap();
    assert_eq!(off.preset, TIMER_OFF);
    assert_eq!(catalog.clock(&off), Ok(None));
    for (name, clock) in Clock::PRESETS {
        let timer = catalog.timer(Some(clock)).unwrap();
        let preset = catalog.symbol("TimerPresetOptions", &name.to_uppercase());
        assert_eq!(timer.preset, preset.unwrap());
        assert_eq!(catalog.clock(&timer), Ok(Some(clock)));
    }
    // A clock that is no preset is a Custom timer of the same times.
    let custom = Clock::new(100, 7);
    let timer = catalog.timer(Some(custom)).unwrap();
    assert_eq!(
        timer.preset,
        catalog.symbol("TimerPresetOptions", "CUSTOM").unwrap()
    );
    assert!(timer.parameters.contains(&(3, 100_000)));
    assert!(timer.parameters.contains(&(5, 7_000)));
    assert_eq!(catalog.clock(&timer), Ok(Some(custom)));
    // Free decision time has no Arena equivalent.
    let mut free = timer.clone();
    free.parameters.push((1, 5_000));
    assert!(catalog.clock(&free).is_err());
    let league = TimerSettings {
        preset: catalog.symbol("TimerPresetOptions", "LEAGUE").unwrap(),
        ..timer
    };
    assert!(catalog.clock(&league).is_err());
}
