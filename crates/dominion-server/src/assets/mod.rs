//! `dominion assets sync`: a local cache of dominion.games' card art, frame templates,
//! icons, fonts and sounds, and the manifest that maps engine cards to them. The assets
//! belong to their owners; they are only ever cached locally.

mod fetch;
pub mod manifest;
mod rules;
mod upstream;

use dominion_engine::{Card, Set};
use fetch::{Fetched, Fetcher};
use futures_util::StreamExt;
use manifest::{CardAssets, Manifest, Missing};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

pub const SOURCE: &str = "https://dominion.games";

#[derive(Clone, Debug)]
pub struct SyncOptions {
    pub dir: PathBuf,
    /// Download files even if they are already in the cache.
    pub force: bool,
    /// Fetch only the client's page and bundles, and report what would be downloaded.
    pub dry_run: bool,
}

/// What to fetch: every engine card's files and the other static files.
struct Plan {
    version: String,
    /// By engine card name; `None` if upstream doesn't have the card.
    cards: BTreeMap<&'static str, Option<CardAssets>>,
    files: BTreeSet<String>,
    /// Unreleased cards upstream has nothing for.
    skipped: usize,
}

impl Plan {
    fn new(version: String, tables: &upstream::Tables, mut files: BTreeSet<String>) -> Plan {
        let join = rules::Join::new(tables);
        let mut cards = BTreeMap::new();
        let mut skipped = 0;
        for &card in Card::ALL {
            match join.find(card.name()) {
                Some(up) => {
                    let mut assets = tables.card_files(up, card);
                    assets.retain(upstream::is_safe_path);
                    files.extend(
                        tables
                            .extra_files(up, card)
                            .into_iter()
                            .filter(|p| upstream::is_safe_path(p)),
                    );
                    cards.insert(card.name(), Some(assets));
                }
                None if card.set() == Set::Arcana => skipped += 1,
                None => {
                    cards.insert(card.name(), None);
                }
            }
        }
        Plan {
            version,
            cards,
            files,
            skipped,
        }
    }

    fn paths(&self) -> BTreeSet<&str> {
        let card_paths = self
            .cards
            .values()
            .flatten()
            .flat_map(|a| a.fields().into_iter().filter_map(|(_, p)| p));
        self.files
            .iter()
            .map(String::as_str)
            .chain(card_paths)
            .collect()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Outcome {
    Present,
    Downloaded,
    Missing(String),
    /// Would be downloaded (a dry run).
    Planned,
}

pub async fn sync(options: &SyncOptions) -> Result<(), String> {
    let fetcher = Fetcher::new(SOURCE);
    let index = fetcher.text("").await?;
    let client = upstream::parse_index(&index)?;
    let body = fetcher.text(&client.body_js).await?;
    let tables = upstream::Tables::parse(&body)
        .map_err(|e| format!("{e} {} (it may have changed)", client.version))?;
    let mut files = upstream::literal_paths(&index, "");
    files.extend(upstream::literal_paths(&body, ""));
    files.extend(upstream::constructed_paths(&body));
    for doc in &client.documents {
        let text = fetcher.text(doc).await?;
        let dir = doc.rsplit_once('/').map_or("", |(d, _)| d);
        // Stylesheets resolve `url()` against themselves; scripts and templates against
        // the page.
        let dir = if doc.ends_with(".css") { dir } else { "" };
        files.extend(upstream::literal_paths(&text, dir));
    }
    files.retain(|p| upstream::is_safe_path(p));
    let plan = Plan::new(client.version, &tables, files);

    std::fs::create_dir_all(&options.dir)
        .map_err(|e| format!("creating {}: {e}", options.dir.display()))?;
    let mut outcomes: BTreeMap<String, Outcome> = BTreeMap::new();
    let mut todo = Vec::new();
    for path in plan.paths() {
        if !options.force && is_cached(&options.dir.join(path)) {
            outcomes.insert(path.to_string(), Outcome::Present);
        } else if options.dry_run {
            outcomes.insert(path.to_string(), Outcome::Planned);
        } else {
            todo.push(path.to_string());
        }
    }
    let total = todo.len();
    if total > 0 {
        eprintln!("dominion: requesting {total} files from {SOURCE}");
    }
    let mut results = futures_util::stream::iter(todo)
        .map(|path| {
            let fetcher = &fetcher;
            async move {
                let got = fetcher.get(&path).await;
                (path, got)
            }
        })
        .buffer_unordered(fetch::IN_FLIGHT);
    let mut done = 0;
    while let Some((path, got)) = results.next().await {
        let outcome = match got? {
            Fetched::Body(bytes) => {
                let file = options.dir.join(&path);
                fetch::write_atomic(&file, &bytes)
                    .map_err(|e| format!("writing {}: {e}", file.display()))?;
                Outcome::Downloaded
            }
            Fetched::Missing(reason) => Outcome::Missing(reason),
        };
        outcomes.insert(path, outcome);
        done += 1;
        if done % 250 == 0 {
            eprintln!("dominion: {done} of {total}");
        }
    }
    drop(results);

    let manifest = manifest_of(&plan, &outcomes);
    if !options.dry_run {
        let json = serde_json::to_vec_pretty(&manifest).map_err(|e| e.to_string())?;
        let file = options.dir.join(manifest::FILE_NAME);
        fetch::write_atomic(&file, &json)
            .map_err(|e| format!("writing {}: {e}", file.display()))?;
    }
    print!("{}", summary(&plan, &outcomes, &manifest, options));
    Ok(())
}

fn is_cached(path: &Path) -> bool {
    std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.len() > 0)
}

fn manifest_of(plan: &Plan, outcomes: &BTreeMap<String, Outcome>) -> Manifest {
    let have = |p: &str| {
        matches!(
            outcomes.get(p),
            Some(Outcome::Present | Outcome::Downloaded | Outcome::Planned)
        )
    };
    let mut cards = BTreeMap::new();
    let mut missing = Vec::new();
    let mut card_art = BTreeSet::new();
    for (&name, planned) in &plan.cards {
        let Some(planned) = planned else {
            missing.push(Missing {
                card: Some(name.into()),
                what: "card".into(),
                url: None,
                reason: "not in upstream's card table".into(),
            });
            cards.insert(name.to_string(), CardAssets::default());
            continue;
        };
        for (what, path) in planned.fields() {
            let Some(path) = path else { continue };
            if let Some(Outcome::Missing(reason)) = outcomes.get(path) {
                missing.push(Missing {
                    card: Some(name.into()),
                    what: what.into(),
                    url: Some(fetch::url(SOURCE, path)),
                    reason: reason.clone(),
                });
            }
        }
        let mut assets = planned.clone();
        assets.retain(have);
        card_art.extend(assets.art.clone());
        card_art.extend(assets.mini_art.clone());
        cards.insert(name.to_string(), assets);
    }
    let mut files = Vec::new();
    for (path, outcome) in outcomes {
        if card_art.contains(path) {
            continue;
        }
        match outcome {
            Outcome::Missing(reason) => {
                if plan.files.contains(path) {
                    missing.push(Missing {
                        card: None,
                        what: "file".into(),
                        url: Some(fetch::url(SOURCE, path)),
                        reason: reason.clone(),
                    });
                }
            }
            _ => files.push(path.clone()),
        }
    }
    Manifest {
        schema: manifest::SCHEMA.into(),
        source: SOURCE.into(),
        client_version: plan.version.clone(),
        cards,
        files,
        missing,
    }
}

/// Whether a card has its art and every frame that applies to it.
fn covered(planned: &CardAssets, got: &CardAssets) -> bool {
    planned.art.is_some()
        && planned.template.is_some()
        && planned.mini_template.is_some()
        && planned.fields() == got.fields()
}

/// The directory a file is counted under: two levels deep, three under `images/cards`.
fn category(path: &str) -> &str {
    let depth = if path.starts_with("images/cards/") {
        3
    } else {
        2
    };
    let end = path
        .match_indices('/')
        .take(depth)
        .last()
        .map_or(0, |(i, _)| i);
    &path[..end]
}

fn summary(
    plan: &Plan,
    outcomes: &BTreeMap<String, Outcome>,
    manifest: &Manifest,
    options: &SyncOptions,
) -> String {
    use std::fmt::Write;
    let count = |f: fn(&Outcome) -> bool| outcomes.values().filter(|o| f(o)).count();
    let mut out = String::new();
    let _ = writeln!(
        out,
        "dominion.games client {} -> {}",
        plan.version,
        options.dir.display()
    );
    if options.dry_run {
        let _ = writeln!(
            out,
            "files: {} to download, {} already present (dry run: nothing downloaded)",
            count(|o| *o == Outcome::Planned),
            count(|o| *o == Outcome::Present),
        );
    } else {
        let _ = writeln!(
            out,
            "files: {} downloaded, {} already present, {} missing",
            count(|o| *o == Outcome::Downloaded),
            count(|o| *o == Outcome::Present),
            count(|o| matches!(o, Outcome::Missing(_))),
        );
    }
    let mut by_category: BTreeMap<&str, usize> = BTreeMap::new();
    for (path, outcome) in outcomes {
        if !matches!(outcome, Outcome::Missing(_)) {
            *by_category.entry(category(path)).or_default() += 1;
        }
    }
    let categories: Vec<String> = by_category
        .iter()
        .map(|(c, n)| format!("{c} {n}"))
        .collect();
    let _ = writeln!(out, "by directory: {}", categories.join(", "));
    let mut uncovered = Vec::new();
    for (&name, planned) in &plan.cards {
        let reason = match planned {
            None => "not in upstream's card table".to_string(),
            Some(planned) => {
                let got = &manifest.cards[name];
                if covered(planned, got) {
                    continue;
                }
                let lacking: Vec<&str> = planned
                    .fields()
                    .iter()
                    .zip(got.fields())
                    .filter(|((_, p), (_, g))| p.is_none() || g.is_none())
                    .map(|((what, _), _)| *what)
                    .filter(|w| *w != "microTemplate" || planned.micro_template.is_some())
                    .collect();
                format!("no {}", lacking.join(", "))
            }
        };
        uncovered.push(format!("  {name}: {reason}"));
    }
    let _ = writeln!(
        out,
        "cards: {} of {} fully covered{}",
        plan.cards.len() - uncovered.len(),
        plan.cards.len(),
        if plan.skipped > 0 {
            format!(" ({} unreleased cards skipped)", plan.skipped)
        } else {
            String::new()
        }
    );
    if !uncovered.is_empty() {
        let _ = writeln!(out, "not covered:");
        for line in uncovered {
            let _ = writeln!(out, "{line}");
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_manifest_lists_only_what_was_obtained() {
        let tables = upstream::tests::tables();
        let files = ["fonts/a.ttf", "images/x.png"].map(String::from).into();
        let plan = Plan::new("2.3.7".into(), &tables, files);
        let village_art = "images/cards/art/base/village.webp";
        let mut outcomes: BTreeMap<String, Outcome> = plan
            .paths()
            .into_iter()
            .map(|p| (p.to_string(), Outcome::Downloaded))
            .collect();
        outcomes.insert(village_art.into(), Outcome::Missing("404".into()));
        outcomes.insert("images/x.png".into(), Outcome::Missing("404".into()));
        let m = manifest_of(&plan, &outcomes);

        let village = serde_json::to_value(&m.cards["Village"]).unwrap();
        assert_eq!(
            village,
            serde_json::json!({
                "template": "images/cards/templates/action.webp",
                "miniTemplate": "images/cards/templates/mini-action.webp",
                "microTemplate": "images/cards/templates/micro-action.webp",
            })
        );
        let gone = |card: Option<&str>, what: &str| {
            m.missing
                .iter()
                .find(|x| x.card.as_deref() == card && x.what == what)
                .cloned()
        };
        let art = gone(Some("Village"), "art").unwrap();
        assert_eq!(
            art.url.as_deref(),
            Some(&*format!("{SOURCE}/{village_art}"))
        );
        assert_eq!(art.reason, "404");
        assert_eq!(
            gone(None, "file").and_then(|x| x.url),
            Some(format!("{SOURCE}/images/x.png"))
        );
        assert_eq!(m.cards["Adventurer"], CardAssets::default());
        assert!(gone(Some("Adventurer"), "card").is_some());
        assert!(!m.cards.contains_key("Ambrosia"));

        assert!(m.files.contains(&"fonts/a.ttf".to_string()));
        assert!(
            m.files
                .contains(&"images/cards/templates/action.webp".to_string())
        );
        assert!(!m.files.iter().any(|f| f.starts_with("images/cards/art/")));
        assert!(m.files.is_sorted());
    }

    #[test]
    fn categories_group_by_directory() {
        assert_eq!(
            category("images/cards/art/base/village.webp"),
            "images/cards/art"
        );
        assert_eq!(
            category("images/cards/templates/action.webp"),
            "images/cards/templates"
        );
        assert_eq!(category("images/elements/coin.png"), "images/elements");
        assert_eq!(category("images/dom-icon-black.png"), "images");
        assert_eq!(category("fonts/TrajanPro3SemiBold.ttf"), "fonts");
        assert_eq!(category("sounds/ping.mp3"), "sounds");
    }
}
