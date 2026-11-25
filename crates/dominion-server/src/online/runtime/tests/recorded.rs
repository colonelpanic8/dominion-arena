//! Public question trees and the card instances they reference, extracted from the
//! named dominion.games recordings. Only Knights also needs its recorded pile zone.
use super::*;
use crate::mcp::play::{Play, Presentation};
use dominion_online::catalog::CardInfo;
use serde::Deserialize;
use std::sync::Mutex;

#[derive(Deserialize)]
struct Case {
    cards: Vec<model::Card>,
    zones: Vec<model::Zone>,
    question: Question,
}

#[derive(Deserialize)]
struct CatalogExcerpt {
    cards: BTreeMap<usize, CardInfo>,
    labels: BTreeMap<String, BTreeMap<usize, String>>,
}

fn recorded(source: &str) -> Reducer {
    let case: Case = serde_json::from_str(source).unwrap();
    let excerpt: CatalogExcerpt =
        serde_json::from_str(include_str!("recorded/catalog.json")).unwrap();
    let mut cards = vec![excerpt.cards[&0].clone(); *excerpt.cards.keys().last().unwrap() + 1];
    for (id, info) in excerpt.cards {
        cards[id] = info;
    }
    let labels = excerpt
        .labels
        .into_iter()
        .map(|(key, entries)| {
            let mut labels = vec![None; *entries.keys().last().unwrap() + 1];
            for (id, text) in entries {
                labels[id] = Some(text);
            }
            (key, labels)
        })
        .collect();
    let mut snap = snapshot();
    snap.state.zones = case.zones;
    for c in case.cards {
        let id = c.id as usize;
        if id >= snap.state.cards.len() {
            snap.state.cards.resize_with(id + 1, || model::Card {
                id: -1,
                card_name: 0,
                pile_name: 0,
            });
        }
        snap.state.cards[id] = c;
    }
    let mut r = Reducer::new(
        Arc::new(Catalog {
            cards,
            enums: Default::default(),
            labels,
            texts: Default::default(),
            productions: Default::default(),
        }),
        42,
    );
    r.replace(snap).unwrap();
    r.ask(case.question).unwrap();
    r
}

fn observation(r: &Reducer) -> rmcp::model::CallToolResult {
    let (_dir, mut rt) = runtime();
    rt.update.reducer = r.clone();
    let (_, updates) = watch::channel(rt.update);
    let (tx, _) = mpsc::channel(1);
    let seat = crate::online::Session::new(Client { tx, updates });
    Play {
        seat: &seat,
        shown: &Mutex::new(Presentation::default()),
        clock_cues: false,
    }
    .observation(true)
}

fn submit(r: &mut Reducer, labels: &[&str]) -> dominion_online::Result<(i32, Vec<i32>)> {
    let d = r.view().unwrap().decision.unwrap();
    let indices = labels
        .iter()
        .map(|label| d.options.iter().find(|o| o.label == *label).unwrap().index)
        .collect();
    r.answer(&Answer::indices(d.id, indices), None)
}

#[test]
fn peasant_exchange_sends_the_official_default() {
    let mut r = recorded(include_str!("recorded/186282628.json"));
    let d = r.view().unwrap().decision.unwrap();
    let exchange = "Exchange [Peasant] for a [Soldier]";
    assert_eq!(
        d.options
            .iter()
            .map(|o| o.label.as_str())
            .collect::<Vec<_>>(),
        [exchange, "Continue"]
    );
    let state = observation(&r);
    assert!(state.content[0].as_text().unwrap().text.contains(exchange));
    assert!(
        state
            .structured_content
            .unwrap()
            .get("decisionConsequence")
            .is_none()
    );
    assert_eq!(
        submit(&mut r, &[exchange]).unwrap(),
        (12, vec![1, 0, 1, 0, 0])
    );
}

#[test]
fn way_of_the_owl_requires_the_vassal_to_play() {
    let mut r = recorded(include_str!("recorded/186284041.json"));
    let state = observation(&r);
    let text = &state.content[0].as_text().unwrap().text;
    assert!(
        text.contains("a Way to play the card you choose with"),
        "{text}"
    );
    assert!(
        submit(&mut r, &["Way of the Owl"])
            .unwrap_err()
            .to_string()
            .contains("Way of the Owl is only the Way")
    );
    assert_eq!(
        submit(&mut r, &["Play Vassal", "Way of the Owl"]).unwrap(),
        (22, vec![0, 1, 0, 1, 0])
    );
}

#[test]
fn scouting_party_buy_exposes_the_five_unplayed_coppers() {
    let mut r = recorded(include_str!("recorded/186357877.json"));
    let state = observation(&r);
    let body = state.structured_content.as_ref().unwrap();
    let d = r.view().unwrap().decision.unwrap();
    let note = &body["decisionConsequence"];
    let text = &state.content[0].as_text().unwrap().text;
    assert_eq!(body["view"]["decision"], serde_json::json!(d));
    assert!(
        text.contains("5 Copper ($5)") && text.contains(note["text"].as_str().unwrap()),
        "{text}"
    );
    let indices = |action: &str| {
        d.options
            .iter()
            .filter(|o| o.action == action)
            .map(|o| o.index)
            .collect::<Vec<_>>()
    };
    assert_eq!(note["forgoneOptions"], serde_json::json!(indices("play")));
    assert_eq!(indices("play").len(), 5);
    let mut closing = indices("buy");
    closing.extend(indices("end-phase"));
    assert_eq!(note["closingOptions"], serde_json::json!(closing));
    assert_eq!(
        submit(&mut r.clone(), &["Buy Scouting Party"]).unwrap(),
        (22, vec![0, 16])
    );
    assert_eq!(
        submit(&mut r, &["Autoplay Treasures"]).unwrap(),
        (22, vec![2, 0])
    );
}

#[test]
fn knights_show_the_same_top_the_server_offers_to_sell() {
    let mut r = recorded(include_str!("recorded/186315373.json"));
    let state = observation(&r);
    let text = &state.content[0].as_text().unwrap().text;
    assert!(
        text.contains("Knights (top Dame Sylvia)") && text.contains("Buy Dame Sylvia"),
        "{text}"
    );
    assert_eq!(
        submit(&mut r, &["Buy Dame Sylvia"]).unwrap(),
        (8, vec![0, 8])
    );
}
