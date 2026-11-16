//! Static extraction from the exact supported public bundle. No upstream code executes.
use std::collections::BTreeMap;

use dominion_protocol::Cost;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{Error, Result};

pub const VERSION: &str = "2.3.7";
pub const BODY_URL: &str = "https://dominion.games/js/dominion-webclient-body-2.3.7.min.js";
pub const BODY_SHA256: &str = "53f5163219f75d782e971d9dd96337a016c1f4d2c23a51476f50700ec6a816dc";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CardInfo {
    pub key: String,
    pub name: String,
    pub cost: Cost,
    pub types: Vec<String>,
    pub kingdom: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Catalog {
    pub cards: Vec<CardInfo>,
    pub enums: BTreeMap<String, Vec<String>>,
    pub labels: BTreeMap<String, Vec<Option<String>>>,
}

impl Catalog {
    pub fn from_bundle(bytes: &[u8]) -> Result<Self> {
        let hash = format!("{:x}", Sha256::digest(bytes));
        if hash != BODY_SHA256 {
            return Err(Error::Unsupported(format!(
                "client bundle hash {hash}; expected {VERSION} / {BODY_SHA256}"
            )));
        }
        let source = std::str::from_utf8(bytes)
            .map_err(|_| Error::Protocol("bundle is not UTF-8".into()))?;
        Self::extract(source)
    }

    fn extract(source: &str) -> Result<Self> {
        let mut enums = BTreeMap::new();
        for name in [
            "Types",
            "CounterNames",
            "TokenNames",
            "AbilityNames",
            "CardModes",
            "GameButtons",
            "QuestionTypes",
            "QuestionIds",
            "CleanupActions",
            "ExtraTurnTypes",
            "LogEntryNames",
            "DeclineButtonIds",
            "MovementTypes",
            "ResignationTypes",
            "TimerPresetOptions",
            "TimeSettingsParameters",
            "TableRuleIds",
            "Expansions",
            "UpdateTypes",
            "UserPrefIds",
            "UserPrefTypes",
            "ActivityStati",
            "TableStati",
            "FailureReasons",
            "SuccessDescriptions",
        ] {
            let values: Vec<String> = object(source, name)?
                .into_iter()
                .map(|(k, _)| k.to_string())
                .collect();
            enums.insert(name.to_string(), values);
        }
        let zones = list(assignment(source, "ZoneNames").ok_or_else(|| malformed("ZoneNames"))?)?;
        if zones.len() != 35
            || zones
                .iter()
                .any(|name| !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_'))
        {
            return Err(malformed("ZoneNames constructors"));
        }
        enums.insert(
            "ZoneNames".into(),
            zones.into_iter().map(str::to_owned).collect(),
        );
        let types: Vec<_> = object(source, "Types")?
            .into_iter()
            .map(|(_, v)| {
                string(
                    args(v)?
                        .first()
                        .copied()
                        .ok_or_else(|| malformed("type name"))?,
                )
            })
            .collect::<Result<_>>()?;
        let mut cards = Vec::new();
        for (key, value) in object(source, "CardNames")? {
            let values = args(value)?;
            if values.len() < 5 {
                return Err(malformed("CardName fields"));
            }
            let mut name = string(values[0])?;
            if let Some(english) = assignment(source, &format!("EnglishCardNames[CardNames.{key}]"))
                && let Ok(parts) = args(english)
                && let Some(first) = parts.first()
                && let Ok(s) = string(first)
            {
                name = s;
            }
            let cost = args(values[2])?;
            let number = |i: usize| -> Result<u32> {
                cost.get(i)
                    .map_or(Ok(0), |v| v.parse().map_err(|_| malformed("card cost")))
            };
            let card_types = list(values[3])?
                .into_iter()
                .map(|v| {
                    let key = v
                        .strip_prefix("Types.")
                        .ok_or_else(|| malformed("card type"))?;
                    let index = enums["Types"]
                        .iter()
                        .position(|k| k == key)
                        .ok_or_else(|| malformed("type reference"))?;
                    Ok(types[index].clone())
                })
                .collect::<Result<_>>()?;
            cards.push(CardInfo {
                key: key.into(),
                name,
                cost: Cost {
                    coins: number(0)?,
                    potions: number(1)?,
                    debt: number(2)?,
                },
                types: card_types,
                kingdom: matches!(values[4], "!0" | "true"),
            });
        }
        if cards.len() != 881 || cards[2].name != "Copper" || cards[31].name != "Village" {
            return Err(malformed("catalog ordinals"));
        }
        let mut labels = BTreeMap::new();
        for (en, english) in [
            ("AbilityNames", "EnglishAbilityNames"),
            ("CardModes", "EnglishCardModes"),
            ("GameButtons", "EnglishGameButtons"),
            ("QuestionIds", "EnglishQuestionDescriptions"),
            ("CleanupActions", "EnglishCleanupActions"),
            ("LogEntryNames", "EnglishLogEntries"),
            ("DeclineButtonIds", "EnglishDeclineButtons"),
        ] {
            let entries = enums[en]
                .iter()
                .map(|key| {
                    let value = assignment(source, &format!("{english}[{en}.{key}]"))?;
                    if value.starts_with('[') {
                        list(value).ok()?.last().and_then(|v| string(v).ok())
                    } else {
                        string(value).ok()
                    }
                })
                .collect();
            labels.insert(en.into(), entries);
        }
        labels.insert(
            "AbilityDescriptions".into(),
            enums["AbilityNames"]
                .iter()
                .map(|key| {
                    assignment(
                        source,
                        &format!("EnglishAbilityDescriptions[AbilityNames.{key}]"),
                    )
                    .and_then(|v| string(v).ok())
                })
                .collect(),
        );
        Ok(Self {
            cards,
            enums,
            labels,
        })
    }

    pub fn card(&self, ordinal: i32) -> Result<&CardInfo> {
        usize::try_from(ordinal)
            .ok()
            .and_then(|i| self.cards.get(i))
            .ok_or_else(|| malformed("card ordinal"))
    }

    pub fn ordinal(&self, name: &str) -> Option<i32> {
        let key = normalize(name);
        self.cards
            .iter()
            .position(|c| normalize(&c.name) == key)
            .map(|i| i as i32)
    }

    pub fn label(&self, table: &str, ordinal: i32) -> Option<String> {
        let i = usize::try_from(ordinal).ok()?;
        if let Some(Some(label)) = self.labels.get(table).and_then(|v| v.get(i)) {
            return Some(label.clone());
        }
        self.enums
            .get(table)?
            .get(i)
            .map(|key| key.replace('_', " ").to_lowercase())
    }
}

pub fn normalize(name: &str) -> String {
    name.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn malformed(field: &str) -> Error {
    Error::Protocol(format!("invalid pinned catalog: {field}"))
}

fn assignment<'a>(source: &'a str, name: &str) -> Option<&'a str> {
    let needle = format!("{name}=");
    source.match_indices(&needle).find_map(|(i, _)| {
        if i > 0
            && (source.as_bytes()[i - 1].is_ascii_alphanumeric()
                || b"_$.".contains(&source.as_bytes()[i - 1]))
        {
            return None;
        }
        let rest = &source[i + needle.len()..];
        let end = boundary(rest, true).ok()?;
        Some(&rest[..end])
    })
}

fn object<'a>(source: &'a str, name: &str) -> Result<Vec<(&'a str, &'a str)>> {
    let value = assignment(source, name).ok_or_else(|| malformed(name))?;
    let inner = value
        .strip_prefix('{')
        .and_then(|s| s.strip_suffix('}'))
        .ok_or_else(|| malformed(name))?;
    split(inner)?
        .into_iter()
        .map(|entry| {
            entry
                .split_once(':')
                .map(|(k, v)| (k.trim(), v.trim()))
                .ok_or_else(|| malformed(name))
        })
        .collect()
}

fn args(value: &str) -> Result<Vec<&str>> {
    let start = value.find('(').ok_or_else(|| malformed("constructor"))?;
    split(
        value[start + 1..]
            .strip_suffix(')')
            .ok_or_else(|| malformed("constructor close"))?,
    )
}

fn list(value: &str) -> Result<Vec<&str>> {
    split(
        value
            .strip_prefix('[')
            .and_then(|s| s.strip_suffix(']'))
            .ok_or_else(|| malformed("array"))?,
    )
}

fn split(mut source: &str) -> Result<Vec<&str>> {
    let mut out = Vec::new();
    while !source.trim().is_empty() {
        let at = boundary(source, false)?;
        out.push(source[..at].trim());
        source = &source[at..];
        if source.starts_with(',') {
            source = &source[1..];
        } else {
            break;
        }
    }
    Ok(out)
}

fn boundary(source: &str, semicolon: bool) -> Result<usize> {
    let mut depth = 0usize;
    let mut quote = None;
    let mut escape = false;
    for (i, b) in source.bytes().enumerate() {
        if let Some(q) = quote {
            if escape {
                escape = false;
            } else if b == b'\\' {
                escape = true;
            } else if b == q {
                quote = None;
            }
            continue;
        }
        match b {
            b'"' | b'\'' | b'`' => quote = Some(b),
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => {
                depth = depth
                    .checked_sub(1)
                    .ok_or_else(|| malformed("unbalanced literal"))?
            }
            b',' if depth == 0 => return Ok(i),
            b';' if depth == 0 && semicolon => return Ok(i),
            _ => {}
        }
    }
    if depth != 0 || quote.is_some() {
        return Err(malformed("unterminated literal"));
    }
    Ok(source.len())
}

fn string(value: &str) -> Result<String> {
    if value.starts_with('"') {
        return serde_json::from_str(value).map_err(|_| malformed("string literal"));
    }
    Err(malformed("string literal"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn drift_is_rejected_before_extraction() {
        assert!(matches!(
            Catalog::from_bundle(b"CardNames={}"),
            Err(Error::Unsupported(_))
        ));
    }
    #[test]
    fn literal_boundaries_do_not_split_strings_or_nested_data() {
        assert_eq!(
            object("A={X:new K(\"a,b\",[1,2]),Y:new K(3)};", "A")
                .unwrap()
                .len(),
            2
        );
        assert_eq!(assignment("LongA={};A=\"hello\";", "A"), Some("\"hello\""));
        assert!(split("[1,2").is_err());
    }
    #[test]
    #[ignore = "requires downloaded public pinned bundle; no network in tests"]
    fn extracts_verified_bundle() {
        let path = std::env::var("DOMINION_ONLINE_TEST_BUNDLE").expect("test bundle path");
        let catalog = Catalog::from_bundle(&std::fs::read(path).unwrap()).unwrap();
        assert_eq!(catalog.cards.len(), 881);
        assert_eq!(catalog.enums["ZoneNames"].len(), 35);
        for (index, name) in [
            (0, "HandZone"),
            (1, "DrawZone"),
            (2, "DiscardZone"),
            (3, "TrashZone"),
            (4, "SupplyCardZone"),
            (5, "BasicTreasureZone"),
            (6, "BasicVictoryZone"),
            (8, "InPlayZone"),
        ] {
            assert_eq!(catalog.enums["ZoneNames"][index], name);
        }
        assert_eq!(catalog.enums["ResignationTypes"][0], "MANUAL");
        assert_eq!(catalog.enums["TimerPresetOptions"][4], "OFF");
        assert_eq!(catalog.enums["ExtraTurnTypes"][0], "REGULAR");
        assert_eq!(
            catalog.enums["UpdateTypes"],
            ["NONE", "AUTOMATCH", "FRIEND_LIST", "TABLES"]
        );
        assert_eq!(catalog.enums["Expansions"].len(), 20);
        assert_eq!(catalog.enums["TableRuleIds"].len(), 25);
        assert_eq!(catalog.enums["UserPrefIds"].len(), 52);
        assert_eq!(catalog.enums["ActivityStati"].len(), 6);
        assert_eq!(catalog.enums["TableStati"].len(), 5);
        assert_eq!(catalog.enums["SuccessDescriptions"].len(), 11);
        assert_eq!(catalog.enums["FailureReasons"], crate::transport::FAILURES);
        assert_eq!(catalog.enums["CardModes"].len(), 221);
        assert_eq!(catalog.enums["GameButtons"].len(), 8);
        assert_eq!(catalog.enums["DeclineButtonIds"].len(), 71);
        assert_eq!(
            catalog.label("DeclineButtonIds", 0).as_deref(),
            Some("End Actions")
        );
        assert_eq!(
            catalog
                .card(catalog.ordinal("King's Court").unwrap())
                .unwrap()
                .name,
            "King's Court"
        );
    }
}
