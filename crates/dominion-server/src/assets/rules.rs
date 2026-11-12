//! How dominion.games names a card's files: art by expansion and name, frame templates
//! by type string, and the join from engine cards to upstream's card table.

use super::manifest::CardAssets;
use super::upstream::{Tables, UpCard};
use dominion_engine::Card;
use std::collections::HashMap;

/// Types upstream draws as a landscape (wide card with a cost corner).
const LANDSCAPE_TYPES: &[&str] = &[
    "Event", "Landmark", "Project", "Way", "Ally", "Trait", "Prophecy", "Boon", "Hex", "State",
    "Artifact",
];

/// Types whose cards have a reminder icon (`images/reminders/<name>.png`).
const REMINDER_TYPES: &[&str] = &["State", "Artifact"];

/// Lowercase alphanumerics only: upstream's names drop punctuation.
pub fn normalize(name: &str) -> String {
    name.chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

fn dashed(name: &str) -> String {
    name.to_lowercase().replace(' ', "-")
}

/// Upstream's cards by normalized name.
pub struct Join<'a> {
    by_key: HashMap<String, &'a UpCard>,
}

impl<'a> Join<'a> {
    pub fn new(tables: &'a Tables) -> Join<'a> {
        let mut by_key: HashMap<String, &UpCard> = HashMap::new();
        for card in &tables.cards {
            if card
                .expansion
                .as_ref()
                .is_none_or(|e| e.file_name.is_none())
            {
                continue;
            }
            let deprecated = |c: &UpCard| {
                c.expansion
                    .as_ref()
                    .is_some_and(|e| e.file_name.as_deref() == Some("deprecated"))
            };
            by_key
                .entry(normalize(&card.name))
                .and_modify(|old| {
                    if deprecated(old) && !deprecated(card) {
                        *old = card;
                    }
                })
                .or_insert(card);
        }
        Join { by_key }
    }

    /// The upstream card for an engine card name. Split and mixed pile randomizers are
    /// "<singular> Pile" upstream (Augurs, Ruins, Loot).
    pub fn find(&self, name: &str) -> Option<&'a UpCard> {
        let key = normalize(name);
        let mut keys = vec![key.clone(), format!("{key}pile")];
        for suffix in ["es", "s"] {
            if let Some(stem) = key.strip_suffix(suffix) {
                keys.push(format!("{stem}pile"));
            }
        }
        keys.iter().find_map(|k| self.by_key.get(k).copied())
    }
}

impl Tables {
    /// The card's type names in table order, lowercase, joined by `-`.
    fn type_string(&self, types: &[String]) -> String {
        self.types
            .iter()
            .filter(|t| types.contains(t))
            .map(|t| t.to_lowercase().replace(' ', ""))
            .collect::<Vec<_>>()
            .join("-")
    }

    /// Removes the first `-<type>` of each listed type, as upstream does.
    fn without(mut s: String, types: &[String]) -> String {
        for t in types {
            let pat = format!("-{}", t.to_lowercase());
            if let Some(i) = s.find(&pat) {
                s.replace_range(i..i + pat.len(), "");
            }
        }
        s
    }

    pub fn full_template(
        &self,
        types: &[String],
        basic: bool,
        heirloom: bool,
        landscape: bool,
    ) -> Option<String> {
        let s = Tables::without(self.type_string(types), &self.no_template);
        (!s.is_empty()).then(|| {
            format!(
                "images/cards/templates/{s}{}{}{}.webp",
                if basic { "-basic" } else { "" },
                if heirloom { "-heirloom" } else { "" },
                if landscape { "-with-corner" } else { "" },
            )
        })
    }

    /// A landscape's mini form is its full template without the cost corner.
    pub fn mini_template(&self, types: &[String], landscape: bool, trait_: bool) -> Option<String> {
        if landscape {
            return self.full_template(types, false, false, false);
        }
        let s = Tables::without(self.type_string(types), &self.no_template);
        let s = Tables::without(s, &self.no_mini_template);
        (!s.is_empty()).then(|| {
            format!(
                "images/cards/templates/mini-{s}{}.webp",
                if trait_ { "-trait" } else { "" }
            )
        })
    }

    pub fn micro_template(&self, types: &[String]) -> Option<String> {
        let words: Vec<&str> = self
            .micro
            .iter()
            .filter(|(t, _)| types.contains(t))
            .map(|(_, w)| w.as_str())
            .collect();
        (!words.is_empty())
            .then(|| format!("images/cards/templates/micro-{}.webp", words.join("-")))
    }

    /// The types upstream renders `card` with: its own, or the engine's when upstream
    /// lists none (a pile placeholder such as Loot Pile).
    fn render_types(&self, card: &UpCard, engine: Card) -> Vec<String> {
        if !card.types.is_empty() {
            return card.types.clone();
        }
        engine
            .type_names()
            .into_iter()
            .filter(|t| self.types.iter().any(|u| u == t))
            .map(String::from)
            .collect()
    }

    /// The files upstream draws `card` (the engine's `engine`) with.
    pub fn card_files(&self, card: &UpCard, engine: Card) -> CardAssets {
        let types = self.render_types(card, engine);
        let landscape = types.iter().any(|t| LANDSCAPE_TYPES.contains(&t.as_str()));
        let basic = self.base_cards.contains(&card.key);
        let expansion = card.expansion.as_ref();
        let art = expansion.and_then(|e| {
            let dir = e.file_name.as_ref()?;
            Some(format!(
                "images/cards/art/{dir}/{}.webp",
                dashed(&card.name)
            ))
        });
        let mini_art = if basic {
            expansion.map(|e| {
                format!(
                    "images/cards/art/{}/mini-{}.webp",
                    dashed(&e.name),
                    dashed(&card.name)
                )
            })
        } else {
            art.clone()
        };
        CardAssets {
            art,
            mini_art,
            template: self.full_template(
                &types,
                basic,
                self.heirloom_cards.contains(&card.key),
                landscape,
            ),
            mini_template: self.mini_template(&types, landscape, false),
            micro_template: self.micro_template(&types),
        }
    }

    /// Variants of `card`'s files that aren't its own: the mini frame with a Trait on
    /// its pile, and a State's or Artifact's reminder icon.
    pub fn extra_files(&self, card: &UpCard, engine: Card) -> Vec<String> {
        let types = self.render_types(card, engine);
        let landscape = types.iter().any(|t| LANDSCAPE_TYPES.contains(&t.as_str()));
        let mut out = Vec::new();
        if card.kingdom_pile && !landscape {
            out.extend(self.mini_template(&types, false, true));
        }
        if types.iter().any(|t| REMINDER_TYPES.contains(&t.as_str())) {
            out.push(format!("images/reminders/{}.png", dashed(&card.name)));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assets::upstream::tests::tables;

    fn files(name: &str) -> CardAssets {
        let t = tables();
        let join = Join::new(&t);
        let engine = Card::from_name(name).unwrap();
        let card = join.find(engine.name()).unwrap_or_else(|| panic!("{name}"));
        t.card_files(card, engine)
    }

    fn short(p: Option<String>) -> String {
        p.map_or("-".into(), |p| {
            p.trim_start_matches("images/cards/templates/")
                .trim_end_matches(".webp")
                .to_string()
        })
    }

    /// (template, mini template, micro template) without directory and extension.
    fn frames(name: &str) -> [String; 3] {
        let f = files(name);
        [
            short(f.template),
            short(f.mini_template),
            short(f.micro_template),
        ]
    }

    #[test]
    fn joins_engine_names_to_upstream_names() {
        let t = tables();
        let join = Join::new(&t);
        for (engine, upstream) in [
            ("King's Court", "KINGS_COURT"),
            ("Will-o'-Wisp", "WILL_O_WISP"),
            ("Jack of All Trades", "JACK_OF_ALL_TRADES"),
            ("The Sea's Gift", "THE_SEAS_GIFT"),
            ("Architects' Guild", "ARCHITECTS_GUILD"),
            ("Augurs", "AUGUR_PILE"),
            ("Ruins", "RUIN_PILE"),
            ("Loot", "LOOT_PILE"),
            ("Knights", "KNIGHTS"),
        ] {
            assert_eq!(join.find(engine).map(|c| c.key.as_str()), Some(upstream));
        }
        assert!(join.find("Unused").is_none());
        assert!(join.find("Adventurer").is_none());
    }

    #[test]
    fn art_comes_from_the_expansion_directory() {
        let court = files("King's Court");
        assert_eq!(
            court.art.as_deref(),
            Some("images/cards/art/prosperity/kings-court.webp")
        );
        assert_eq!(court.mini_art, court.art);
        assert_eq!(
            files("Will-o'-Wisp").art.as_deref(),
            Some("images/cards/art/nocturne/will-o-wisp.webp")
        );
        assert_eq!(
            files("Venture").art.as_deref(),
            Some("images/cards/art/deprecated/venture.webp")
        );
        assert_eq!(
            files("Augurs").art.as_deref(),
            Some("images/cards/art/allies/augur-pile.webp")
        );
    }

    #[test]
    fn basic_cards_have_their_own_mini_art() {
        for (card, mini) in [
            ("Copper", "base/mini-copper"),
            ("Curse", "base/mini-curse"),
            ("Estate", "base/mini-estate"),
            ("Platinum", "prosperity/mini-platinum"),
            ("Potion", "alchemy/mini-potion"),
        ] {
            assert_eq!(
                files(card).mini_art,
                Some(format!("images/cards/art/{mini}.webp")),
                "{card}"
            );
        }
    }

    #[test]
    fn picks_the_frame_upstream_picks() {
        for (card, expected) in [
            ("Village", ["action", "mini-action", "micro-action"]),
            ("Militia", ["action", "mini-action", "micro-action"]),
            (
                "Moat",
                [
                    "action-reaction",
                    "mini-action-reaction",
                    "micro-action-reaction",
                ],
            ),
            ("Venture", ["treasure", "mini-treasure", "micro-treasure"]),
            (
                "Copper",
                ["treasure-basic", "mini-treasure", "micro-treasure"],
            ),
            ("Curse", ["curse-basic", "mini-curse", "micro-curse"]),
            ("Gardens", ["victory", "mini-victory", "micro-victory"]),
            (
                "Mill",
                [
                    "action-victory",
                    "mini-action-victory",
                    "micro-action-victory",
                ],
            ),
            (
                "Farm",
                [
                    "treasure-victory",
                    "mini-treasure-victory",
                    "micro-treasure-victory",
                ],
            ),
            (
                "Distant Lands",
                [
                    "action-reserve-victory",
                    "mini-action-reserve-victory",
                    "micro-action-reserve-victory",
                ],
            ),
            (
                "Caravan",
                [
                    "action-duration",
                    "mini-action-duration",
                    "micro-action-duration",
                ],
            ),
            (
                "Ratcatcher",
                [
                    "action-reserve",
                    "mini-action-reserve",
                    "micro-action-reserve",
                ],
            ),
            ("Monastery", ["night", "mini-night", "micro-night"]),
            (
                "Ghost",
                [
                    "night-duration",
                    "mini-night-duration",
                    "micro-night-duration",
                ],
            ),
            ("Vampire", ["night", "mini-night", "micro-night"]),
            ("Pixie", ["action-heirloom", "mini-action", "micro-action"]),
            (
                "Haunted Mirror",
                ["treasure", "mini-treasure", "micro-treasure"],
            ),
            (
                "Necropolis",
                ["action-shelter", "mini-action-shelter", "micro-action"],
            ),
            (
                "Hovel",
                [
                    "reaction-shelter",
                    "mini-reaction-shelter",
                    "micro-reaction",
                ],
            ),
            (
                "Abandoned Mine",
                ["action-ruins", "mini-action-ruins", "micro-action-ruins"],
            ),
            (
                "Ruins",
                ["action-ruins", "mini-action-ruins", "micro-action-ruins"],
            ),
            ("Dame Anna", ["action", "mini-action", "micro-action"]),
            ("Knights", ["action", "mini-action", "micro-action"]),
            (
                "Small Castle",
                [
                    "action-victory",
                    "mini-action-victory",
                    "micro-action-victory",
                ],
            ),
            (
                "Humble Castle",
                [
                    "treasure-victory",
                    "mini-treasure-victory",
                    "micro-treasure-victory",
                ],
            ),
            ("Page", ["action-traveller", "mini-action", "micro-action"]),
            ("Alms", ["event-with-corner", "event", "-"]),
            (
                "Aqueduct",
                ["landmark-with-corner", "landmark", "micro-landmark"],
            ),
            ("Academy", ["project-with-corner", "project", "-"]),
            ("Way of the Mouse", ["way-with-corner", "way", "-"]),
            (
                "Architects' Guild",
                ["ally-with-corner", "ally", "micro-ally"],
            ),
            ("Cheap", ["trait-with-corner", "trait", "micro-trait"]),
            (
                "Approaching Army",
                ["prophecy-with-corner", "prophecy", "-"],
            ),
            ("The Sea's Gift", ["boon-with-corner", "boon", "micro-boon"]),
            ("Bad Omens", ["hex-with-corner", "hex", "micro-hex"]),
            ("Deluded", ["state-with-corner", "state", "micro-state"]),
            ("Flag", ["artifact-with-corner", "artifact", "-"]),
            ("Settlers", ["action", "mini-action", "micro-action"]),
            (
                "Bustling Village",
                ["action", "mini-action", "micro-action"],
            ),
            ("Augurs", ["action", "mini-action", "micro-action"]),
            ("Herb Gatherer", ["action", "mini-action", "micro-action"]),
            ("Loot", ["treasure", "mini-treasure", "micro-treasure"]),
            (
                "Amphora",
                [
                    "treasure-duration",
                    "mini-treasure-duration",
                    "micro-treasure-duration",
                ],
            ),
        ] {
            assert_eq!(frames(card), expected.map(String::from), "{card}");
        }
    }

    #[test]
    fn extra_files_are_trait_minis_and_reminders() {
        let t = tables();
        let join = Join::new(&t);
        let extra = |name: &str| {
            let engine = Card::from_name(name).unwrap();
            t.extra_files(join.find(name).unwrap(), engine)
        };
        assert_eq!(
            extra("Village"),
            ["images/cards/templates/mini-action-trait.webp"]
        );
        assert_eq!(extra("Deluded"), ["images/reminders/deluded.png"]);
        assert!(extra("Copper").is_empty());
        assert!(extra("Alms").is_empty());
    }
}
