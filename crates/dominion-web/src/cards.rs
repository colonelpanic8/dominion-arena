//! Card data from `GET /api/cards`, looked up by name.

use dominion_protocol::api::CardInfo;
use std::collections::{BTreeMap, BTreeSet};

const LANDSCAPE_KINDS: [&str; 7] = [
    "event", "landmark", "project", "way", "ally", "trait", "prophecy",
];

#[derive(Clone, Debug, Default, PartialEq)]
pub struct CardDb {
    cards: Vec<CardInfo>,
    by_key: BTreeMap<String, usize>,
}

fn key(name: &str) -> String {
    name.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

impl CardDb {
    pub fn new(cards: Vec<CardInfo>) -> CardDb {
        let by_key = cards
            .iter()
            .enumerate()
            .map(|(i, c)| (key(&c.name), i))
            .collect();
        CardDb { cards, by_key }
    }

    pub fn is_empty(&self) -> bool {
        self.cards.is_empty()
    }

    pub fn all(&self) -> &[CardInfo] {
        &self.cards
    }

    /// Looks a card up by name, ignoring case, spaces and punctuation.
    pub fn get(&self, name: &str) -> Option<&CardInfo> {
        self.by_key.get(&key(name)).map(|&i| &self.cards[i])
    }

    /// The card's text as a tooltip: name, types, cost and official text.
    pub fn tooltip(&self, name: &str) -> String {
        let Some(c) = self.get(name) else {
            return String::new();
        };
        let mut head = format!("{} — {}", c.name, c.types.join(" - "));
        if let Some(cost) = &c.cost {
            let mut parts = vec![];
            if cost.coins > 0 || (cost.potions == 0 && cost.debt == 0) {
                parts.push(format!("${}", cost.coins));
            }
            if cost.potions > 0 {
                parts.push(format!("{}P", cost.potions));
            }
            if cost.debt > 0 {
                parts.push(format!("{}D", cost.debt));
            }
            let mut cost = parts.join(" ");
            if c.variable_cost {
                cost.push('*');
            }
            if c.overpay {
                cost.push('+');
            }
            head.push_str(&format!(" ({cost})"));
        }
        let text = c.text.replace("\n---\n", "\n—————\n");
        format!("{head}\n{text}")
    }

    fn names(&self, keep: impl Fn(&CardInfo) -> bool) -> Vec<String> {
        let mut names: Vec<String> = self
            .cards
            .iter()
            .filter(|c| keep(c))
            .map(|c| c.name.clone())
            .collect();
        names.sort();
        names
    }

    /// Kingdom cards a game can be set up with.
    pub fn kingdom_names(&self) -> Vec<String> {
        self.names(|c| c.kingdom && !c.removed)
    }

    /// Events, Landmarks, Projects, Ways, Allies, Traits and Prophecies.
    pub fn landscape_names(&self) -> Vec<String> {
        self.names(|c| LANDSCAPE_KINDS.contains(&c.kind.as_str()) && !c.removed)
    }

    /// Everything a "name a card" decision accepts.
    pub fn nameable(&self) -> Vec<String> {
        self.names(|c| c.kind == "card")
    }

    /// The sets a random kingdom can be dealt from.
    pub fn sets(&self) -> Vec<String> {
        self.cards
            .iter()
            .filter(|c| c.kingdom && !c.removed)
            .map(|c| c.set.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect()
    }

    /// The card a "name a card" answer names, if `input` is one.
    pub fn card_name(&self, input: &str) -> Option<&str> {
        self.get(input)
            .filter(|c| c.kind == "card")
            .map(|c| c.name.as_str())
    }
}
