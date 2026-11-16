//! Every card and landscape a game can deal has a rating in the playbook the agents read,
//! so their ratings lookup never comes back empty for a card on their board. Runs when
//! `DOMINION_PLAYBOOK` names a dominion-playbook checkout (the dev shell and the Nix build
//! set it to the flake's input).

use serde_json::Value;
use std::path::Path;

const LANDSCAPES: [&str; 7] = [
    "Event", "Project", "Way", "Landmark", "Ally", "Trait", "Prophecy",
];

#[test]
fn every_dealable_card_has_a_rating() {
    let Some(playbook) = std::env::var_os("DOMINION_PLAYBOOK") else {
        eprintln!("DOMINION_PLAYBOOK is unset: skipped");
        return;
    };
    let cards = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/cards");
    let mut names = vec![];
    for entry in std::fs::read_dir(&cards).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|e| e != "json") {
            continue;
        }
        let set: Vec<Value> = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        for card in set {
            let landscape = (card["types"].as_array().into_iter().flatten())
                .any(|t| LANDSCAPES.contains(&t.as_str().unwrap_or("")));
            // Arcana is unreleased and unrated; removed cards aren't dealt at random, and
            // the sheet rates only some of them.
            let dealt = (card["kingdom"] == true || landscape)
                && card["unimplemented"] != true
                && card["removed"] != true
                && card["set"] != "arcana";
            if dealt {
                names.push(card["name"].as_str().unwrap().to_string());
            }
        }
    }
    assert!(names.len() > 600, "only {} cards found", names.len());
    let out = std::process::Command::new("python3")
        .arg("data/card-ratings.py")
        .args(&names)
        .current_dir(&playbook)
        .output()
        .expect("python3 runs the playbook's ratings lookup");
    let text = String::from_utf8_lossy(&out.stdout);
    let unrated: Vec<&str> = text
        .lines()
        .filter(|l| l.starts_with("Not rated:"))
        .collect();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(unrated.is_empty(), "{}", unrated.join("\n"));
}
