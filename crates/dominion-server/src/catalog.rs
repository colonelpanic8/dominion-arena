//! The models the runner's agents may play with, found in each provider's own catalog
//! (`[[catalogs]]`): every model a pattern allows, at every effort allowed, is an agent
//! of its own, named after them ("Opus 5.5 High"), and has its account made when it is
//! first seated. An `[[agents]]` profile of the same program, model and effort takes
//! its place under the profile's name.

use crate::house::{Offer, OfferedModel};
use crate::runner::{AgentSpec, Program};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CatalogSpec {
    /// `claude`, `codex`, `antigravity` or `openai`.
    pub program: Program,
    /// Model ids to offer, `*` matching anything (default: every model it lists).
    #[serde(default)]
    pub models: Vec<String>,
    /// Model ids not to offer, `*` matching anything.
    #[serde(default)]
    pub exclude: Vec<String>,
    /// Efforts to offer each model at, where it has them (default: all it has).
    pub efforts: Option<Vec<String>>,
    /// Ends its agents' names, telling them from another program's agents of the same
    /// model ("GPT-6.2-Sol High Codex").
    pub tag: Option<String>,
    pub command: Option<String>,
    pub auth_dir: Option<PathBuf>,
    pub instructions: Option<String>,
    pub response_seconds: Option<u32>,
    pub api_key_env: Option<String>,
    pub context_results: Option<usize>,
}

/// A model a provider lists.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Found {
    pub id: String,
    pub name: String,
    /// The efforts it may be run at.
    pub efforts: Vec<String>,
    /// The model of each effort whose id names it (Antigravity's "gemini-4-argon-high"),
    /// run with no effort of its own.
    pub ids: Vec<(String, String)>,
}

impl Found {
    /// The model to run at `effort`, and the effort to give it.
    fn variant(&self, effort: Option<&str>) -> (String, Option<String>) {
        match effort.and_then(|e| self.ids.iter().find(|(x, _)| x == e)) {
            Some((_, id)) => (id.clone(), None),
            None => (self.id.clone(), effort.map(String::from)),
        }
    }

    fn runs(&self, id: &str) -> bool {
        self.id == id || self.ids.iter().any(|(_, i)| i == id)
    }
}

impl Program {
    /// What runs it, for people.
    pub fn label(self) -> &'static str {
        match self {
            Program::Claude => "Claude Code",
            Program::Codex => "Codex",
            Program::Antigravity => "Antigravity",
            Program::T3Code => "T3 Code",
            Program::OpenAi => "OpenAI",
        }
    }

    /// The efforts its agents may be given.
    pub fn efforts(self) -> &'static [&'static str] {
        match self {
            Program::Claude => &["low", "medium", "high", "xhigh", "max"],
            Program::Codex => &["minimal", "low", "medium", "high", "xhigh", "max", "ultra"],
            Program::Antigravity => &["low", "medium", "high", "xhigh", "max"],
            Program::OpenAi => &["minimal", "low", "medium", "high", "xhigh"],
            Program::T3Code => &[],
        }
    }

    /// Its native login and configuration directory.
    pub fn native_dir(self) -> Option<PathBuf> {
        let (var, dir) = match self {
            Program::Claude => ("CLAUDE_CONFIG_DIR", ".claude"),
            Program::Codex | Program::OpenAi => ("CODEX_HOME", ".codex"),
            Program::Antigravity => ("GEMINI_HOME", ".gemini"),
            Program::T3Code => return None,
        };
        std::env::var_os(var)
            .map(PathBuf::from)
            .or_else(|| Some(PathBuf::from(std::env::var_os("HOME")?).join(dir)))
    }
}

impl CatalogSpec {
    pub fn validate(&self) -> Result<(), String> {
        if self.program == Program::T3Code {
            return Err(
                "t3code models aren't listed in a catalog: offer them as [[agents]]".into(),
            );
        }
        for effort in self.efforts.iter().flatten() {
            if !self.program.efforts().contains(&effort.as_str()) {
                return Err(format!("unsupported {:?} effort {effort:?}", self.program));
            }
        }
        if self.program != Program::OpenAi
            && (self.api_key_env.is_some() || self.context_results.is_some())
        {
            return Err("api_key_env and context_results require program = \"openai\"".into());
        }
        Ok(())
    }

    /// Whether the patterns allow `model`, by its id or that of one of its efforts.
    fn allows(&self, model: &Found) -> bool {
        let mut ids = std::iter::once(&model.id).chain(model.ids.iter().map(|(_, id)| id));
        ids.any(|id| {
            (self.models.is_empty() || self.models.iter().any(|p| glob(p, id)))
                && !self.exclude.iter().any(|p| glob(p, id))
        })
    }

    /// The models the provider lists now. Antigravity's are asked of its CLI, so this
    /// blocks.
    pub fn discover(&self) -> Result<Vec<Found>, String> {
        let dir = (self.auth_dir.clone())
            .or_else(|| self.program.native_dir())
            .ok_or("no provider directory: set auth_dir (or HOME)")?;
        let found = match self.program {
            Program::Claude => claude_catalog(&dir)?,
            Program::Codex => codex_catalog(&read(&dir.join("models_cache.json"))?, false)?,
            Program::OpenAi => codex_catalog(&read(&dir.join("models_cache.json"))?, true)?,
            Program::Antigravity => {
                let mut cmd = std::process::Command::new(self.command.as_deref().unwrap_or("agy"));
                cmd.arg("models").stdin(std::process::Stdio::null());
                if self.auth_dir.is_some() {
                    cmd.env("GEMINI_HOME", &dir);
                }
                let out = cmd.output().map_err(|e| format!("agy models: {e}"))?;
                if !out.status.success() {
                    return Err(format!(
                        "agy models: {}",
                        String::from_utf8_lossy(&out.stderr).trim()
                    ));
                }
                antigravity_models(&String::from_utf8_lossy(&out.stdout))
            }
            Program::T3Code => vec![],
        };
        Ok(found.into_iter().filter(|m| self.allows(m)).collect())
    }

    /// The agent playing `model` at `effort`.
    fn agent(&self, model: &Found, effort: Option<&str>) -> AgentSpec {
        let (id, own_effort) = model.variant(effort);
        AgentSpec {
            name: agent_name(&model.name, effort, self.tag.as_deref()),
            program: self.program,
            model: Some(id),
            command: self.command.clone(),
            about: None,
            instructions: self.instructions.clone(),
            effort: own_effort,
            response_seconds: self.response_seconds,
            history_turns: None,
            auth_dir: self.auth_dir.clone(),
            provider_instance: None,
            options: BTreeMap::new(),
            api_key_env: self.api_key_env.clone(),
            context_results: self.context_results,
        }
    }
}

fn read(path: &Path) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|e| {
        format!(
            "{}: {e}; start the provider's CLI once to fill its model catalog",
            path.display()
        )
    })
}

/// `pattern` with `*` matching any run of characters.
fn glob(pattern: &str, text: &str) -> bool {
    let mut parts = pattern.split('*');
    let first = parts.next().unwrap_or_default();
    let Some(mut rest) = text.strip_prefix(first) else {
        return false;
    };
    let parts: Vec<&str> = parts.collect();
    let Some((last, middle)) = parts.split_last() else {
        return rest.is_empty();
    };
    for part in middle {
        match rest.find(part) {
            Some(i) => rest = &rest[i + part.len()..],
            None => return false,
        }
    }
    rest.ends_with(last)
}

/// Claude Code's catalog (`cache/model-catalog/*-cc.json`), the most recently fetched.
fn claude_catalog(dir: &Path) -> Result<Vec<Found>, String> {
    let folder = dir.join("cache/model-catalog");
    let entries = std::fs::read_dir(&folder).map_err(|e| {
        format!(
            "{}: {e}; start Claude Code once to fill its model catalog",
            folder.display()
        )
    })?;
    let newest = entries
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().ends_with("-cc.json"))
        .filter_map(|e| serde_json::from_str::<Value>(&read(&e.path()).ok()?).ok())
        .max_by_key(|c| c["fetchedAt"].as_u64().unwrap_or(0))
        .ok_or_else(|| format!("{}: no Claude Code catalog", folder.display()))?;
    claude_models(&newest)
}

fn claude_models(catalog: &Value) -> Result<Vec<Found>, String> {
    let models = catalog["catalog"]["config"]["models"]
        .as_array()
        .ok_or("Claude Code's catalog has no models")?;
    Ok(models
        .iter()
        .filter_map(|m| {
            let levels = match m["runtime"]["effort_levels"].as_array() {
                Some(levels) => levels.iter().filter_map(|l| l.as_str()).collect(),
                None => (m["thinking"]["effort_options"].as_array().into_iter())
                    .flatten()
                    .filter_map(|o| o["id"].as_str())
                    .collect::<Vec<_>>(),
            };
            Some(Found {
                id: m["id"].as_str()?.into(),
                name: m["name"].as_str().or(m["id"].as_str())?.into(),
                efforts: levels.into_iter().map(String::from).collect(),
                ids: vec![],
            })
        })
        .collect())
}

/// Codex's `models_cache.json`: the models it lists, those of its API with `api`.
fn codex_catalog(text: &str, api: bool) -> Result<Vec<Found>, String> {
    let catalog: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    let models = catalog["models"]
        .as_array()
        .ok_or("Codex model catalog has no models")?;
    let mut listed: Vec<&Value> = (models.iter())
        .filter(|m| m["visibility"] == "list" && (!api || m["supported_in_api"] == true))
        .collect();
    listed.sort_by_key(|m| m["priority"].as_i64().unwrap_or(i64::MAX));
    Ok(listed
        .into_iter()
        .filter_map(|m| {
            Some(Found {
                id: m["slug"].as_str()?.into(),
                name: m["display_name"].as_str().or(m["slug"].as_str())?.into(),
                efforts: (m["supported_reasoning_levels"].as_array().into_iter())
                    .flatten()
                    .filter_map(|l| l["effort"].as_str().map(String::from))
                    .collect(),
                ids: vec![],
            })
        })
        .collect())
}

/// `agy models`: an id and a name a line, the effort in both.
/// Models whose id and name end in an effort ("gemini-4-argon-high", "Gemini 4 Argon
/// (High)") are one model at each of them.
fn antigravity_models(out: &str) -> Vec<Found> {
    let order = Program::Antigravity.efforts();
    let mut models: Vec<Found> = vec![];
    for line in out.lines() {
        let Some((id, name)) = line.split_once('\t') else {
            continue;
        };
        let (id, name) = (id.trim(), name.trim());
        if id.is_empty() {
            continue;
        }
        let name = if name.is_empty() { id } else { name };
        let split = order.iter().find_map(|e| {
            let base_id = id.strip_suffix(&format!("-{e}"))?;
            let label = format!("({})", effort_label(e));
            let base_name = name.strip_suffix(&label)?.trim_end();
            Some((*e, base_id, base_name))
        });
        let Some((effort, base_id, base_name)) = split else {
            models.push(Found {
                id: id.into(),
                name: name.into(),
                efforts: vec![],
                ids: vec![],
            });
            continue;
        };
        let at = match models
            .iter()
            .position(|m| m.id == base_id && !m.ids.is_empty())
        {
            Some(at) => at,
            None => {
                models.push(Found {
                    id: base_id.into(),
                    name: base_name.into(),
                    efforts: vec![],
                    ids: vec![],
                });
                models.len() - 1
            }
        };
        let model = &mut models[at];
        model.ids.push((effort.into(), id.into()));
        model.efforts.push(effort.into());
        model
            .efforts
            .sort_by_key(|e| order.iter().position(|o| o == e));
    }
    models
}

/// An agent's name from its model's: "Opus 5.5 High", within the longest seat name.
pub fn agent_name(model: &str, effort: Option<&str>, tag: Option<&str>) -> String {
    let clean = |s: &str| {
        let s: String = (s.chars())
            .map(|c| match c {
                c if c.is_ascii_alphanumeric() || "_-.()".contains(c) => c,
                _ => ' ',
            })
            .collect();
        s.split_whitespace().collect::<Vec<_>>().join(" ")
    };
    let suffix: Vec<String> = [effort.map(effort_label), tag.map(clean)]
        .into_iter()
        .flatten()
        .filter(|s| !s.is_empty())
        .collect();
    let suffix = suffix.join(" ");
    let room = crate::hub::MAX_NAME_CHARS - suffix.len().min(crate::hub::MAX_NAME_CHARS - 2) - 1;
    let model: String = clean(model)
        .trim_start_matches(|c: char| !c.is_ascii_alphanumeric())
        .chars()
        .take(room)
        .collect();
    let model = model.trim_end();
    match suffix.is_empty() {
        true => model.to_string(),
        false => format!("{model} {suffix}"),
    }
}

fn effort_label(effort: &str) -> String {
    match effort {
        "xhigh" => "XHigh".into(),
        e => {
            let mut chars = e.chars();
            (chars.next().into_iter())
                .flat_map(char::to_uppercase)
                .chain(chars)
                .collect()
        }
    }
}

/// What the runner offers: every catalog's models at each effort they and it allow,
/// a profile in place of the one of the same program, model and effort, then the other
/// profiles. Names repeated (ignoring case) are offered once, first come.
pub fn offers(
    profiles: &[AgentSpec],
    catalogs: &[(CatalogSpec, Vec<Found>)],
) -> Vec<(AgentSpec, Offer)> {
    let same = |p: &AgentSpec, s: &AgentSpec| {
        (p.program, &p.model, &p.effort) == (s.program, &s.model, &s.effort)
    };
    let mut used = vec![false; profiles.len()];
    let mut out: Vec<(AgentSpec, Offer)> = vec![];
    let mut push = |spec: AgentSpec, model: OfferedModel, on_demand: bool| {
        let key = spec.name.to_lowercase();
        if out.iter().any(|(s, _)| s.name.to_lowercase() == key) {
            eprintln!("dominion agents: {} is offered once", spec.name);
            return;
        }
        let offer = Offer {
            name: spec.name.clone(),
            about: spec.about(),
            model: Some(model),
            on_demand,
        };
        out.push((spec, offer));
    };
    let offered = |program: Program, model: &Found, effort: Option<&str>| OfferedModel {
        program: program.label().into(),
        id: model.id.clone(),
        name: model.name.clone(),
        effort: effort.map(String::from),
    };
    for (catalog, found) in catalogs {
        for model in found {
            let efforts: Vec<Option<&str>> = match model.efforts.is_empty() {
                true => vec![None],
                false => (model.efforts.iter())
                    .map(String::as_str)
                    .filter(|e| catalog.program.efforts().contains(e))
                    .filter(|e| {
                        catalog
                            .efforts
                            .as_ref()
                            .is_none_or(|a| a.iter().any(|a| a == e))
                    })
                    .map(Some)
                    .collect(),
            };
            for effort in efforts {
                let spec = catalog.agent(model, effort);
                let group = offered(catalog.program, model, effort);
                match (profiles.iter().enumerate()).find(|(_, p)| same(p, &spec)) {
                    Some((i, profile)) => {
                        used[i] = true;
                        push(profile.clone(), group, false);
                    }
                    None => push(spec, group, true),
                }
            }
        }
    }
    for (profile, _) in profiles.iter().zip(&used).filter(|(_, used)| !**used) {
        let id = profile.model.as_deref();
        let found = (catalogs.iter())
            .filter(|(c, _)| c.program == profile.program)
            .flat_map(|(_, found)| found)
            .find(|m| id.is_some_and(|id| m.runs(id)));
        let group = match found {
            Some(m) => {
                let effort = (m.ids.iter())
                    .find(|(_, i)| Some(i.as_str()) == id)
                    .map(|(e, _)| e.as_str())
                    .or(profile.effort.as_deref());
                offered(profile.program, m, effort)
            }
            None => OfferedModel {
                program: profile.program.label().into(),
                id: id.unwrap_or_default().into(),
                name: id.unwrap_or(&profile.name).into(),
                effort: profile.effort.clone(),
            },
        };
        push(profile.clone(), group, false);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn catalog(program: Program) -> CatalogSpec {
        CatalogSpec {
            program,
            models: vec![],
            exclude: vec![],
            efforts: None,
            tag: None,
            command: None,
            auth_dir: None,
            instructions: None,
            response_seconds: None,
            api_key_env: None,
            context_results: None,
        }
    }

    fn found(id: &str, name: &str, efforts: &[&str]) -> Found {
        Found {
            id: id.into(),
            name: name.into(),
            efforts: efforts.iter().map(|e| e.to_string()).collect(),
            ids: vec![],
        }
    }

    #[test]
    fn globs() {
        assert!(glob("claude-*", "claude-opus-5-5"));
        assert!(glob("*-sol", "gpt-6.2-sol"));
        assert!(glob("gpt-*-sol*", "gpt-6.2-sol"));
        assert!(glob("gpt-6", "gpt-6"));
        assert!(!glob("gpt-6", "gpt-6.1"));
        assert!(!glob("claude-*", "gpt-6"));
        assert!(!glob("a*b*c", "acb"));
    }

    #[test]
    fn names_fit_a_seat() {
        assert_eq!(agent_name("Opus 5.5", Some("high"), None), "Opus 5.5 High");
        assert_eq!(
            agent_name("GPT-6.2-Sol", Some("xhigh"), Some("Codex")),
            "GPT-6.2-Sol XHigh Codex"
        );
        assert_eq!(
            agent_name("Gemini 4 Argon (High)", None, None),
            "Gemini 4 Argon (High)"
        );
        assert_eq!(
            agent_name("GPT-6.1 Terra/β", Some("low"), None),
            "GPT-6.1 Terra Low"
        );
        let long = agent_name(
            "An Extraordinarily Long Model Name Indeed",
            Some("medium"),
            None,
        );
        assert_eq!(long, "An Extraordinarily Long M Medium");
        assert_eq!(long.len(), crate::hub::MAX_NAME_CHARS);
    }

    #[test]
    fn reads_claude_codes_catalog() {
        let cache = json!({"fetchedAt": 1, "catalog": {"config": {"models": [
            {"id": "claude-opus-5-5", "name": "Opus 5.5",
             "runtime": {"effort_levels": ["low", "high", "max"]}},
            {"id": "claude-haiku-5-5", "name": "Haiku 5.5",
             "thinking": {"effort_options": [{"id": "low"}, {"id": "medium"}]}},
        ]}}});
        assert_eq!(
            claude_models(&cache).unwrap(),
            [
                found("claude-opus-5-5", "Opus 5.5", &["low", "high", "max"]),
                found("claude-haiku-5-5", "Haiku 5.5", &["low", "medium"]),
            ]
        );
    }

    #[test]
    fn reads_codexs_catalog() {
        let cache = json!({"models": [
            {"slug": "gpt-b", "display_name": "B", "visibility": "list", "priority": 2,
             "supported_in_api": false,
             "supported_reasoning_levels": [{"effort": "high"}]},
            {"slug": "gpt-hidden", "visibility": "hide", "priority": 0},
            {"slug": "gpt-a", "display_name": "A", "visibility": "list", "priority": 1,
             "supported_in_api": true,
             "supported_reasoning_levels": [{"effort": "low"}, {"effort": "ultra"}]},
        ]})
        .to_string();
        let ids = |api| -> Vec<String> {
            (codex_catalog(&cache, api).unwrap().into_iter())
                .map(|m| m.id)
                .collect()
        };
        assert_eq!(ids(false), ["gpt-a", "gpt-b"]);
        assert_eq!(ids(true), ["gpt-a"]);
    }

    #[test]
    fn reads_agy_models_with_their_efforts() {
        let out = "Fetching available models...\n\
            gemini-4-argon-high\tGemini 4 Argon (High)\n\
            gemini-pro-orchestrator\tOrchestrator (Argon)\n\
            gemini-4-argon-low\tGemini 4 Argon (Low)\n";
        let argon = Found {
            ids: vec![
                ("high".into(), "gemini-4-argon-high".into()),
                ("low".into(), "gemini-4-argon-low".into()),
            ],
            ..found("gemini-4-argon", "Gemini 4 Argon", &["low", "high"])
        };
        let models = antigravity_models(out);
        assert_eq!(
            models,
            [
                argon.clone(),
                found("gemini-pro-orchestrator", "Orchestrator (Argon)", &[]),
            ]
        );

        // A profile of one of its models takes that effort's place.
        let profile = AgentSpec {
            name: "Argon High".into(),
            model: Some("gemini-4-argon-high".into()),
            ..catalog(Program::Antigravity).agent(&argon, None)
        };
        let offered = offers(&[profile], &[(catalog(Program::Antigravity), models)]);
        let names: Vec<(&str, Option<&str>, Option<&str>)> = (offered.iter())
            .map(|(s, o)| {
                let m = o.model.as_ref().unwrap();
                (s.name.as_str(), s.model.as_deref(), m.effort.as_deref())
            })
            .collect();
        assert_eq!(
            names,
            [
                (
                    "Gemini 4 Argon Low",
                    Some("gemini-4-argon-low"),
                    Some("low")
                ),
                ("Argon High", Some("gemini-4-argon-high"), Some("high")),
                (
                    "Orchestrator (Argon)",
                    Some("gemini-pro-orchestrator"),
                    None
                ),
            ]
        );
        assert!(offered.iter().all(|(s, _)| s.effort.is_none()));
        assert_eq!(offered[1].1.model.as_ref().unwrap().id, "gemini-4-argon");
    }

    #[test]
    fn offers_each_allowed_effort_with_profiles_in_place() {
        let mut claude = catalog(Program::Claude);
        claude.efforts = Some(vec!["high".into(), "max".into()]);
        let mut gpt = catalog(Program::OpenAi);
        gpt.models = vec!["gpt-6*".into()];
        let profile = AgentSpec {
            effort: Some("high".into()),
            ..catalog(Program::Claude).agent(&found("claude-opus-5-5", "", &[]), None)
        };
        let profile = AgentSpec {
            name: "Opus High".into(),
            ..profile
        };
        let codex_profile = AgentSpec {
            name: "Codex".into(),
            program: Program::Codex,
            model: None,
            ..profile.clone()
        };
        let catalogs = [
            (
                claude.clone(),
                vec![found(
                    "claude-opus-5-5",
                    "Opus 5.5",
                    &["low", "high", "max"],
                )],
            ),
            // OpenAI's API takes no max effort.
            (
                gpt.clone(),
                vec![found("gpt-6-sol", "GPT-6-Sol", &["high", "max"])],
            ),
        ];
        let offered = offers(&[codex_profile, profile], &catalogs);
        let names: Vec<(&str, bool, Option<&str>)> = (offered.iter())
            .map(|(s, o)| (s.name.as_str(), o.on_demand, s.effort.as_deref()))
            .collect();
        assert_eq!(
            names,
            [
                ("Opus High", false, Some("high")),
                ("Opus 5.5 Max", true, Some("max")),
                ("GPT-6-Sol High", true, Some("high")),
                ("Codex", false, Some("high")),
            ]
        );
        let opus = offered[0].1.model.clone().unwrap();
        assert_eq!(
            (opus.program.as_str(), opus.name.as_str()),
            ("Claude Code", "Opus 5.5")
        );
        assert_eq!(offered[1].0.model.as_deref(), Some("claude-opus-5-5"));
    }
}
