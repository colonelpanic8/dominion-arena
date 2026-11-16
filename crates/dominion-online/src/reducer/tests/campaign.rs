use super::*;

struct Random(u64);
impl Random {
    fn pick(&mut self, count: usize) -> usize {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1);
        ((self.0 >> 32) as usize) % count
    }
}

fn observe(zone: i32, card: i32) -> model::Event {
    model::Event::Inspection(model::Inspection {
        player: 0,
        cards: vec![card],
        from_zone: zone,
        to_zone: -1,
        is_public: true,
        linked_to_question: false,
    })
}

#[test]
fn inspection_then_shuffle_revokes_copy_identity() {
    let mut r = reducer();
    r.apply(observe(1, 2)).unwrap();
    assert_eq!(QuestionContext::card_name(&r, 2), Some("Gold".into()));
    r.apply(model::Event::Shuffle(model::Shuffle {
        owner: 0,
        include_discard: false,
    }))
    .unwrap();
    assert_eq!(QuestionContext::card_name(&r, 2), None);
    assert_eq!(
        r.view().unwrap().you.unwrap().deck.known.get("Gold"),
        Some(&1)
    );
}

#[test]
fn disjoint_repeated_inspections_count_copies_once() {
    let mut r = reducer();
    r.apply(movement(20, 1, vec![7], vec![7])).unwrap();
    r.apply(observe(1, 1)).unwrap();
    r.apply(observe(1, 1)).unwrap();
    r.apply(observe(1, 7)).unwrap();
    assert_eq!(
        r.view().unwrap().you.unwrap().deck.known.get("Village"),
        Some(&2)
    );
    r.apply(movement(1, 0, vec![7], vec![7])).unwrap();
    assert_eq!(
        r.view().unwrap().you.unwrap().deck.known.get("Village"),
        Some(&1)
    );
    assert_eq!(QuestionContext::card_name(&r, 1), Some("Village".into()));
}

#[test]
fn turn_change_retires_question_only_visibility() {
    let mut r = reducer();
    let mut q = question();
    q.kind = QuestionKind::Choice {
        min: 1,
        max: 1,
        elements: crate::questions::QuestionElements::Cards(vec![2]),
        decline: -1,
        defaults: vec![],
        affected: vec![],
    };
    r.ask(q).unwrap();
    let id = decision_id(&r);
    assert_eq!(r.card_name(2), Some("Gold".into()));
    r.apply(model::Event::TurnDescription(model::TurnDescription {
        owner_id: 1,
        turn_number: 2,
        turn_type: 0,
        controller_id: 1,
    }))
    .unwrap();
    assert!(r.view().unwrap().decision.is_none());
    assert_eq!(r.card_name(2), None);
    assert!(r.answer(&view::Answer::indices(id, vec![0]), None).is_err());
}

#[test]
fn anonymous_departure_revokes_unlocated_copy_references() {
    let mut r = reducer();
    r.apply(observe(1, 1)).unwrap();
    r.apply(observe(1, 2)).unwrap();
    r.apply(movement(1, 0, vec![-1], vec![-1])).unwrap();
    assert_eq!(QuestionContext::card_name(&r, 1), None);
    assert_eq!(QuestionContext::card_name(&r, 2), None);
}

fn assert_truth(r: &Reducer, truth: &BTreeMap<i32, Vec<i32>>, seed: u64, step: usize) {
    let context = || format!("seed {seed}, step {step}");
    for (zone, cards) in truth {
        let k = &r.zones[zone];
        assert_eq!(k.count as usize, cards.len(), "{} zone {zone}", context());
        let mut actual = BTreeMap::<i32, u32>::new();
        for id in cards {
            *actual.entry(r.raw_name(*id).unwrap()).or_default() += 1;
        }
        for (name, count) in &k.known {
            assert!(
                *count <= *actual.get(name).unwrap_or(&0),
                "{} zone {zone}: claimed {name} x{count}, actual {actual:?}",
                context()
            );
        }
        for (id, name) in &k.exact {
            assert!(
                cards.contains(id),
                "{}: exact card {id} is not in zone {zone}",
                context()
            );
            assert_eq!(*name, r.raw_name(*id).unwrap(), "{}", context());
        }
    }
    for id in &r.visible_ids {
        assert!(
            r.zones.values().any(|k| k.exact.contains_key(id)),
            "{}: unlocated visible copy {id}",
            context()
        );
    }
    for (owner, known) in &r.owned {
        let mut actual = BTreeMap::<i32, u32>::new();
        for zone in r
            .state()
            .unwrap()
            .zones
            .iter()
            .filter(|z| z.owner == *owner)
        {
            for id in &truth[&zone.index] {
                *actual.entry(r.raw_name(*id).unwrap()).or_default() += 1;
            }
        }
        for (name, count) in known {
            assert!(
                *count <= *actual.get(name).unwrap_or(&0),
                "{} owner {owner}: claimed {name} x{count}, actual {actual:?}",
                context()
            );
        }
    }
}

/// Conserves physical cards across observational transitions, without simulating
/// Dominion outcomes or using the adapter's knowledge model as its oracle.
#[test]
fn seeded_observation_campaign() {
    let start = std::env::var("PROTOCOL_REDUCER_SEED")
        .map(|s| s.parse::<u64>().unwrap())
        .unwrap_or(1);
    let seeds = std::env::var("PROTOCOL_REDUCER_SEEDS")
        .map(|s| s.parse::<u64>().unwrap())
        .unwrap_or(64);
    let schema: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../dominion-agent/protocol/agent-view.schema.json"
    ))
    .unwrap();
    let validator = jsonschema::validator_for(&schema).unwrap();
    for seed in start..start + seeds {
        let mut random = Random(seed);
        let initial = snapshot();
        let mut truth: BTreeMap<_, _> = initial
            .state
            .zones
            .iter()
            .map(|z| (z.index, z.cards.clone()))
            .collect();
        let zone_ids: Vec<_> = truth.keys().copied().collect();
        let mut r = reducer();
        let mut replay = reducer();
        for step in 0..96 {
            let occupied: Vec<_> = truth
                .iter()
                .filter(|(_, v)| !v.is_empty())
                .map(|(k, _)| *k)
                .collect();
            let source = occupied[random.pick(occupied.len())];
            let position = random.pick(truth[&source].len());
            let card = truth[&source][position];
            let event = match random.pick(8) {
                0 | 1 => observe(source, card),
                2 => {
                    let owner = random.pick(2) as i32;
                    let (deck, discard) = if owner == 0 { (1, 2) } else { (11, 12) };
                    let include_discard = random.pick(2) == 0;
                    if include_discard {
                        let old = std::mem::take(truth.get_mut(&discard).unwrap());
                        truth.get_mut(&deck).unwrap().extend(old);
                    }
                    model::Event::Shuffle(model::Shuffle {
                        owner,
                        include_discard,
                    })
                }
                3 => {
                    let ids = truth[&source].clone();
                    let mut wire = ids;
                    if matches!(r.zone(source).unwrap().kind, 1) {
                        wire.fill(-1);
                    }
                    model::Event::PileReorder(model::PileReorder {
                        pile: source,
                        cards: wire,
                    })
                }
                _ => {
                    let target = zone_ids[random.pick(zone_ids.len())];
                    let before = if random.pick(2) == 0 { card } else { -1 };
                    let after = if random.pick(2) == 0 { card } else { -1 };
                    truth.get_mut(&source).unwrap().remove(position);
                    truth.get_mut(&target).unwrap().push(card);
                    movement(source, target, vec![before], vec![after])
                }
            };
            r.apply(event.clone())
                .unwrap_or_else(|e| panic!("seed {seed}, step {step}, {event:?}: {e}"));
            replay.apply(event).unwrap();
            assert_truth(&r, &truth, seed, step);
            let view = serde_json::to_value(r.view().unwrap()).unwrap();
            let mut redacted = r.clone();
            for card in &mut redacted.state.as_mut().unwrap().cards {
                if !r.visible_ids.contains(&card.id) && !r.question_visible.contains(&card.id) {
                    card.card_name = 1 + (card.card_name % 3);
                }
            }
            assert_eq!(
                view,
                serde_json::to_value(redacted.view().unwrap()).unwrap(),
                "seed {seed}, step {step}: hidden allocations changed the view"
            );
            assert!(
                validator.is_valid(&view),
                "seed {seed}, step {step}: invalid schema"
            );
            assert_eq!(
                view,
                serde_json::to_value(replay.view().unwrap()).unwrap(),
                "seed {seed}, step {step}: nondeterministic replay"
            );
            if step % 16 == 0 {
                r.ask(question()).unwrap();
                replay.ask(question()).unwrap();
                let id = decision_id(&r);
                let stale = view::Answer::indices("stale", vec![0]);
                assert!(r.answer(&stale, None).is_err());
                assert_eq!(id, decision_id(&r));
                let answer = view::Answer::indices(id, vec![0]);
                assert_eq!(
                    r.answer(&answer, None).unwrap(),
                    replay.answer(&answer, None).unwrap()
                );
                assert!(r.answer(&answer, None).is_err());
            }
            if step % 24 == 23 {
                let mut replacement = snapshot();
                for zone in &mut replacement.state.zones {
                    zone.cards = truth[&zone.index].clone();
                }
                r.replace(replacement.clone()).unwrap();
                replay.replace(replacement).unwrap();
                assert_truth(&r, &truth, seed, step);
            }
        }
    }
}

#[test]
fn three_player_passing_and_controlled_private_inspection() {
    let mut s = snapshot();
    s.state.players.push(model::Player {
        player_id: 1234,
        card_frequencies: vec![],
        reminders: vec![],
        bought_projects: vec![],
    });
    s.state
        .zones
        .iter_mut()
        .find(|z| z.index == 20)
        .unwrap()
        .cards
        .clear();
    let mut third = s.state.zones.iter().find(|z| z.index == 0).unwrap().clone();
    third.index = 40;
    third.owner = 2;
    third.cards = vec![7];
    s.state.zones.push(third);
    let mut r = Reducer::new(catalog(), 42);
    r.replace(s).unwrap();
    r.apply(movement(0, 10, vec![0], vec![-1])).unwrap();
    r.apply(movement(10, 40, vec![-1], vec![-1])).unwrap();
    r.apply(movement(40, 0, vec![-1], vec![7])).unwrap();
    let v = r.view().unwrap();
    assert_eq!(
        v.you.unwrap().hand.known,
        BTreeMap::from([("Village".into(), 1)])
    );
    assert_eq!(v.opponents.len(), 2);
    assert!(
        !serde_json::to_string(&v.opponents)
            .unwrap()
            .contains("Gold")
    );
    r.apply(model::Event::TurnDescription(model::TurnDescription {
        owner_id: 2,
        turn_number: 2,
        turn_type: 0,
        controller_id: 0,
    }))
    .unwrap();
    r.apply(model::Event::Inspection(model::Inspection {
        player: 2,
        cards: vec![4],
        from_zone: 40,
        to_zone: -1,
        is_public: false,
        linked_to_question: false,
    }))
    .unwrap();
    assert_eq!(
        r.view().unwrap().opponents[1].hand.known.get("Gold"),
        Some(&1)
    );
    assert_eq!(r.card_name(5), None);
}
