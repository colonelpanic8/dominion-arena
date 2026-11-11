//! Generates the card table from `data/cards/*.json`.

use serde_json::Value;
use std::collections::BTreeSet;
use std::fmt::Write;
use std::path::PathBuf;

/// Set order: (file stem, `Set` variant, display name).
const SETS: &[(&str, &str, &str)] = &[
    ("base", "Base", "Base"),
    ("intrigue", "Intrigue", "Intrigue"),
    ("seaside", "Seaside", "Seaside"),
    ("alchemy", "Alchemy", "Alchemy"),
    ("prosperity", "Prosperity", "Prosperity"),
    (
        "cornucopia_guilds",
        "CornucopiaGuilds",
        "Cornucopia & Guilds",
    ),
    ("hinterlands", "Hinterlands", "Hinterlands"),
    ("dark_ages", "DarkAges", "Dark Ages"),
    ("adventures", "Adventures", "Adventures"),
    ("empires", "Empires", "Empires"),
    ("nocturne", "Nocturne", "Nocturne"),
    ("renaissance", "Renaissance", "Renaissance"),
    ("menagerie", "Menagerie", "Menagerie"),
    ("allies", "Allies", "Allies"),
    ("plunder", "Plunder", "Plunder"),
    ("rising_sun", "RisingSun", "Rising Sun"),
    ("arcana", "Arcana", "Arcana"),
    ("promos", "Promos", "Promos"),
];

/// Types listed first, in this order; any others follow alphabetically.
const TYPE_ORDER: &[&str] = &[
    "Action",
    "Treasure",
    "Victory",
    "Curse",
    "Attack",
    "Reaction",
    "Duration",
    "Night",
    "Command",
    "Reserve",
    "Event",
    "Landmark",
    "Project",
    "Way",
    "Ally",
    "Trait",
    "Prophecy",
    "Boon",
    "Hex",
    "State",
    "Artifact",
    "Randomizer",
];

fn ident(name: &str) -> String {
    let mut out = String::new();
    let mut upper = true;
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            if upper {
                out.extend(c.to_uppercase());
            } else {
                out.push(c);
            }
            upper = false;
        } else if c == ' ' || c == '-' || c == '/' {
            upper = true;
        }
    }
    out
}

fn type_const(name: &str) -> String {
    name.to_uppercase().replace(' ', "_")
}

fn num(v: &Value, key: &str) -> i64 {
    v.get(key).and_then(Value::as_i64).unwrap_or(0)
}

fn flag(v: &Value, key: &str) -> bool {
    v.get(key).and_then(Value::as_bool).unwrap_or(false)
}

fn main() {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let data = manifest.join("../../data/cards");
    println!("cargo:rerun-if-changed={}", data.display());

    let mut cards: Vec<(String, Value)> = vec![];
    for (stem, set, _) in SETS {
        let path = data.join(format!("{stem}.json"));
        println!("cargo:rerun-if-changed={}", path.display());
        let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path:?}: {e}"));
        let list: Vec<Value> =
            serde_json::from_str(&text).unwrap_or_else(|e| panic!("{path:?}: {e}"));
        for card in list {
            cards.push((set.to_string(), card));
        }
    }

    let mut all_types = BTreeSet::new();
    for (_, c) in &cards {
        for t in c["types"].as_array().unwrap() {
            all_types.insert(t.as_str().unwrap().to_string());
        }
    }
    let mut types: Vec<String> = TYPE_ORDER
        .iter()
        .map(|s| s.to_string())
        .filter(|t| all_types.contains(t))
        .collect();
    types.extend(
        all_types
            .into_iter()
            .filter(|t| !TYPE_ORDER.contains(&t.as_str())),
    );
    assert!(types.len() <= 64, "too many card types");

    let mut out = String::new();
    out.push_str("bitflags::bitflags! {\n    /// Card types.\n    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]\n    pub struct Types: u64 {\n");
    for (i, t) in types.iter().enumerate() {
        writeln!(out, "        const {} = 1 << {i};", type_const(t)).unwrap();
    }
    out.push_str("    }\n}\n\n/// Each type with its printed name, in canonical order.\npub const TYPE_NAMES: &[(Types, &str)] = &[\n");
    for t in &types {
        writeln!(out, "    (Types::{}, {t:?}),", type_const(t)).unwrap();
    }
    out.push_str("];\n\n");

    out.push_str("/// The expansions, in release order.\n#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]\npub enum Set {\n");
    for (_, set, _) in SETS {
        writeln!(out, "    {set},").unwrap();
    }
    out.push_str("}\n\nimpl Set {\n    pub const ALL: &[Set] = &[");
    for (_, set, _) in SETS {
        write!(out, "Set::{set}, ").unwrap();
    }
    out.push_str("];\n\n    pub fn name(self) -> &'static str {\n        match self {\n");
    for (_, set, name) in SETS {
        writeln!(out, "            Set::{set} => {name:?},").unwrap();
    }
    out.push_str("        }\n    }\n}\n\n");

    let mut idents = BTreeSet::new();
    out.push_str("cards! {\n");
    for (_, c) in &cards {
        let name = c["name"].as_str().unwrap();
        let id = ident(name);
        assert!(idents.insert(id.clone()), "duplicate card identifier {id}");
        let text = c["text"].as_str().unwrap_or("");
        writeln!(out, "    /// **{name}**").unwrap();
        if !text.is_empty() {
            out.push_str("    ///\n");
            for line in text.lines() {
                writeln!(
                    out,
                    "    /// {}",
                    line.replace('[', "\\[").replace(']', "\\]")
                )
                .unwrap();
                out.push_str("    ///\n");
            }
        }
        writeln!(out, "    {id},").unwrap();
    }
    out.push_str("}\n\n");

    writeln!(
        out,
        "pub(crate) static DEFS: [CardDef; {}] = [",
        cards.len()
    )
    .unwrap();
    for (set, c) in &cards {
        let name = c["name"].as_str().unwrap();
        let cost = match c.get("cost") {
            Some(Value::Object(_)) => {
                let cost = &c["cost"];
                format!(
                    "Some(Cost {{ coins: {}, potions: {}, debt: {} }})",
                    num(cost, "coins"),
                    num(cost, "potions"),
                    num(cost, "debt")
                )
            }
            _ => "None".to_string(),
        };
        let cost_flags = c.get("cost").cloned().unwrap_or(Value::Null);
        let types_expr = {
            let ts: Vec<String> = c["types"]
                .as_array()
                .unwrap()
                .iter()
                .map(|t| format!("Types::{}.bits()", type_const(t.as_str().unwrap())))
                .collect();
            if ts.is_empty() {
                "Types::empty()".to_string()
            } else {
                format!("Types::from_bits_retain({})", ts.join(" | "))
            }
        };
        let plus = c.get("plus").cloned().unwrap_or(Value::Null);
        let pile = match c.get("pile").and_then(Value::as_str) {
            Some(p) => format!("Some(Card::{})", ident(p)),
            None => "None".to_string(),
        };
        writeln!(
            out,
            "    CardDef {{ name: {name:?}, set: Set::{set}, cost: {cost}, variable_cost: {}, overpay: {}, types: {types_expr}, \
             plus: Plus {{ cards: {}, actions: {}, buys: {}, coins: {}, villagers: {}, coffers: {}, favors: {} }}, \
             coins: {}, potions: {}, vp: {}, kingdom: {}, basic: {}, removed: {}, pile: {pile}, text: {:?} }},",
            flag(&cost_flags, "variable"),
            flag(&cost_flags, "overpay"),
            num(&plus, "cards"),
            num(&plus, "actions"),
            num(&plus, "buys"),
            num(&plus, "coins"),
            num(&plus, "villagers"),
            num(&plus, "coffers"),
            num(&plus, "favors"),
            num(c, "coins"),
            num(c, "potions"),
            num(c, "vp"),
            flag(c, "kingdom"),
            flag(c, "basic"),
            flag(c, "removed"),
            c["text"].as_str().unwrap_or(""),
        )
        .unwrap();
    }
    out.push_str("];\n");

    let out_path = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("cards_data.rs");
    std::fs::write(out_path, out).unwrap();
}
