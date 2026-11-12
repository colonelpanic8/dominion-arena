//! `manifest.json` of the asset cache (`dominion/assets@1`). Depends on serde only.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const SCHEMA: &str = "dominion/assets@1";
pub const FILE_NAME: &str = "manifest.json";

/// Maps engine cards to their cached files. Paths are relative to the cache root and
/// listed only if the file is in the cache.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    pub schema: String,
    pub source: String,
    pub client_version: String,
    /// By engine card name.
    pub cards: BTreeMap<String, CardAssets>,
    /// Every other file in the cache (templates, icons, tokens, fonts, sounds, ...),
    /// sorted.
    pub files: Vec<String>,
    pub missing: Vec<Missing>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CardAssets {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub art: Option<String>,
    /// The art at mini size (a supply pile).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mini_art: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub template: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mini_template: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub micro_template: Option<String>,
}

impl CardAssets {
    /// Each field by its JSON name.
    pub fn fields(&self) -> [(&'static str, Option<&str>); 5] {
        [
            ("art", self.art.as_deref()),
            ("miniArt", self.mini_art.as_deref()),
            ("template", self.template.as_deref()),
            ("miniTemplate", self.mini_template.as_deref()),
            ("microTemplate", self.micro_template.as_deref()),
        ]
    }

    /// Keeps only the fields whose path `keep` accepts.
    pub fn retain(&mut self, mut keep: impl FnMut(&str) -> bool) {
        for field in [
            &mut self.art,
            &mut self.mini_art,
            &mut self.template,
            &mut self.mini_template,
            &mut self.micro_template,
        ] {
            if field.as_deref().is_some_and(|p| !keep(p)) {
                *field = None;
            }
        }
    }
}

/// Something looked for and not obtained.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Missing {
    /// The engine card it was for; absent for other files.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub card: Option<String>,
    /// A [`CardAssets`] field name, `card` for a card upstream doesn't have, or `file`.
    pub what: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    pub reason: String,
}
