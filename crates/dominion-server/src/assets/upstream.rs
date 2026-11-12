//! What dominion.games' client says about its cards and static files, read out of its
//! (minified) page and bundles as data. Nothing here runs upstream's code.

use std::collections::BTreeSet;

/// The client's versioned files, from its `index.html`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Client {
    pub version: String,
    /// The bundle holding the card tables.
    pub body_js: String,
    /// The other bundles, stylesheets and page templates, which name static files.
    pub documents: Vec<String>,
}

pub fn parse_index(html: &str) -> Result<Client, String> {
    let mut files: Vec<String> = Vec::new();
    for dir in ["js/", "css/", "html/"] {
        for (start, _) in html.match_indices(dir) {
            if start > 0 && is_path_char(html.as_bytes()[start - 1]) {
                continue;
            }
            let path = take_path(&html[start..]);
            if [".js", ".css", ".html"]
                .iter()
                .any(|ext| path.ends_with(ext))
                && !files.iter().any(|f| f == path)
            {
                files.push(path.to_string());
            }
        }
    }
    let body = files
        .iter()
        .position(|f| f.starts_with("js/dominion-webclient-body-") && f.ends_with(".js"))
        .ok_or("dominion.games' index.html no longer loads js/dominion-webclient-body-*.js")?;
    let body_js = files.remove(body);
    let version = body_js["js/dominion-webclient-body-".len()..]
        .trim_end_matches(".js")
        .trim_end_matches(".min")
        .to_string();
    Ok(Client {
        version,
        body_js,
        documents: files,
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Expansion {
    pub name: String,
    /// The art directory; `None` for expansions with no cards of their own.
    pub file_name: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UpCard {
    /// The `CardNames` key (`KINGS_COURT`).
    pub key: String,
    /// Upstream's name, without punctuation (`Kings Court`).
    pub name: String,
    pub expansion: Option<Expansion>,
    /// Type names (`Action`, `Attack`).
    pub types: Vec<String>,
    /// Can be a Kingdom pile (the card or its pile's randomizer).
    pub kingdom_pile: bool,
}

/// Upstream's card and type tables.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tables {
    /// Type names in declaration order, which is the order type strings use.
    pub types: Vec<String>,
    /// Types that don't change a full template.
    pub no_template: Vec<String>,
    /// Types that also don't change a mini template.
    pub no_mini_template: Vec<String>,
    /// (type, word) in the order micro templates name them.
    pub micro: Vec<(String, String)>,
    pub cards: Vec<UpCard>,
    /// Keys of the basic Victory, Curse and Treasure cards, which have separate mini art.
    pub base_cards: BTreeSet<String>,
    /// Keys of cards that come with a Heirloom, whose frame says so.
    pub heirloom_cards: BTreeSet<String>,
}

impl Tables {
    pub fn parse(js: &str) -> Result<Tables, String> {
        let missing = |what: &str| format!("couldn't find {what} in dominion.games' client");
        let expansions: Vec<(String, Expansion)> = object(js, "Expansions", "new Expansion(")
            .ok_or_else(|| missing("the expansion table (Expansions)"))?
            .into_iter()
            .filter_map(|(key, args)| {
                Some((
                    key,
                    Expansion {
                        name: string(args.first()?)?,
                        file_name: args.get(1).and_then(|a| string(a)),
                    },
                ))
            })
            .collect();
        let types: Vec<(String, String)> = object(js, "Types", "new Type(")
            .ok_or_else(|| missing("the type table (Types)"))?
            .into_iter()
            .filter_map(|(key, args)| Some((key, string(args.first()?)?)))
            .collect();
        let type_name = |r: &str| {
            let key = r.strip_prefix("Types.")?;
            types.iter().find(|(k, _)| k == key).map(|(_, n)| n.clone())
        };
        let type_list = |name: &str| -> Result<Vec<String>, String> {
            let list = array(js, name).ok_or_else(|| missing(name))?;
            Ok(list.iter().filter_map(|r| type_name(r)).collect())
        };
        let no_template = type_list("NO_TEMPLATE_TYPES")?;
        let no_mini_template = type_list("NO_MINI_TEMPLATE_TYPES")?;
        let micro = micro_words(js)
            .ok_or_else(|| missing("the micro template rule (getMicroTemplateURL)"))?
            .into_iter()
            .filter_map(|(key, word)| Some((type_name(&format!("Types.{key}"))?, word)))
            .collect::<Vec<_>>();
        if micro.is_empty() {
            return Err(missing("the micro template rule (getMicroTemplateURL)"));
        }
        let cards: Vec<UpCard> = object(js, "CardNames", "new CardName(")
            .ok_or_else(|| missing("the card table (CardNames)"))?
            .into_iter()
            .filter_map(|(key, args)| {
                let expansion = args.get(1)?.strip_prefix("Expansions.").and_then(|e| {
                    expansions
                        .iter()
                        .find(|(k, _)| k == e)
                        .map(|(_, x)| x.clone())
                });
                let types = args.get(3)?;
                let types = types.strip_prefix('[')?.strip_suffix(']')?;
                Some(UpCard {
                    key,
                    name: string(args.first()?)?,
                    expansion,
                    types: split_top(types)
                        .iter()
                        .filter_map(|t| type_name(t))
                        .collect(),
                    kingdom_pile: args.get(4).is_some_and(|b| boolean(b)),
                })
            })
            .collect();
        if cards.is_empty() {
            return Err(missing("the card table (CardNames)"));
        }
        let card_list = |name: &str| -> Result<BTreeSet<String>, String> {
            let list = array(js, name).ok_or_else(|| missing(name))?;
            Ok(list
                .iter()
                .filter_map(|r| r.strip_prefix("CardNames.").map(String::from))
                .collect())
        };
        let mut base_cards = card_list("basicVictoryCards")?;
        base_cards.extend(card_list("basicTreasureCards")?);
        Ok(Tables {
            types: types.into_iter().map(|(_, n)| n).collect(),
            no_template,
            no_mini_template,
            micro,
            cards,
            base_cards,
            heirloom_cards: card_list("HEIRLOOM_CARDS")?,
        })
    }
}

/// Static files a document names in full (`"images/elements/coin.png"`,
/// `url(../fonts/x.ttf)`), relative to the site root. `dir` is the document's directory
/// (`css` for a stylesheet), against which `../` resolves.
pub fn literal_paths(doc: &str, dir: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for prefix in ["images/", "fonts/", "sounds/"] {
        for (start, _) in doc.match_indices(prefix) {
            let mut before = &doc[..start];
            let mut base: Vec<&str> = dir.split('/').filter(|s| !s.is_empty()).collect();
            while let Some(rest) = before.strip_suffix("../") {
                before = rest;
                if base.pop().is_none() {
                    break;
                }
            }
            if !base.is_empty() || before.bytes().last().is_some_and(is_path_char) {
                continue;
            }
            let path = take_path(&doc[start..]);
            if has_asset_extension(path) {
                out.insert(path.to_string());
            }
        }
    }
    out
}

/// Static files the client builds names for from its tables: tokens, counters, buttons,
/// player cubes, sounds, stock avatars, expansion symbols, box art, landscape selectors.
pub fn constructed_paths(js: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let arg = |args: &Vec<String>, i: usize| args.get(i).and_then(|a| string(a));
    for args in calls(js, "TokenName") {
        for name in [arg(&args, 1), arg(&args, 2)].into_iter().flatten() {
            out.insert(format!("images/tokens/{name}.png"));
        }
    }
    for args in calls(js, "CounterName") {
        if let Some(name) = arg(&args, 1) {
            out.insert(format!("images/elements/{name}.png"));
        }
    }
    let buttons = calls(js, "CardMode")
        .iter()
        .filter_map(|a| arg(a, 1))
        .chain(calls(js, "AbilityName").iter().filter_map(|a| arg(a, 3)))
        .chain(calls(js, "GameButton").iter().filter_map(|a| arg(a, 0)))
        .collect::<Vec<_>>();
    for name in buttons {
        out.insert(format!("images/buttons/{name}.png"));
    }
    for args in calls(js, "PlayerColor") {
        if let Some(name) = arg(&args, 1).or_else(|| arg(&args, 0)) {
            out.insert(format!("images/cubes/cube_{name}.png"));
        }
    }
    for args in calls(js, "Sound") {
        if let Some(name) = arg(&args, 0) {
            for ext in ["mp3", "ogg"] {
                out.insert(format!("sounds/{name}.{ext}"));
            }
        }
    }
    for name in array(js, "StockAvatars").unwrap_or_default() {
        if let Some(name) = string(&name) {
            out.insert(format!("images/avatars/{}.webp", name.to_lowercase()));
        }
    }
    for (_, args) in object(js, "Expansions", "new Expansion(").unwrap_or_default() {
        let Some(name) = arg(&args, 0) else { continue };
        let Some(file) = arg(&args, 1) else { continue };
        out.insert(format!("images/elements/{file}-small.png"));
        out.insert(format!("images/elements/{file}-small-white.png"));
        let lower = name.to_lowercase();
        let dashed = lower.split_whitespace().collect::<Vec<_>>();
        out.insert(format!(
            "images/elements/{}-small-white.png",
            dashed.join("-")
        ));
        out.insert(format!("images/box_art/{}.jpg", dashed.concat()));
    }
    if let Some(at) = js.find("getBackground=function") {
        let body = &js[at..];
        let body = &body[..body.find("}}").unwrap_or(body.len())];
        for (i, _) in body.match_indices("return") {
            if let Some(name) = string(body[i + "return".len()..].trim_start()) {
                out.insert(format!("images/cards/templates/{name}-selector.webp"));
            }
        }
    }
    out.retain(|p| is_safe_path(p));
    out
}

/// Whether `path` is a plain relative path that stays inside the cache. Upstream keeps
/// one apostrophe (`king's-cache.webp`).
pub fn is_safe_path(path: &str) -> bool {
    !path.is_empty()
        && path.bytes().all(|b| is_path_char(b) || b == b'\'')
        && path
            .split('/')
            .all(|seg| !seg.is_empty() && seg != "." && seg != "..")
}

fn has_asset_extension(path: &str) -> bool {
    const EXTENSIONS: &[&str] = &[
        "png", "jpg", "jpeg", "webp", "gif", "svg", "ttf", "otf", "woff", "woff2", "eot", "mp3",
        "ogg", "wav",
    ];
    path.rsplit_once('.')
        .is_some_and(|(stem, ext)| !stem.ends_with('/') && EXTENSIONS.contains(&ext))
}

fn is_path_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.' | b'/')
}

fn take_path(s: &str) -> &str {
    let end = s.bytes().position(|b| !is_path_char(b)).unwrap_or(s.len());
    &s[..end]
}

fn is_ident_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'$'
}

/// The index of the bracket closing the one at `open`, skipping string literals.
fn closing(s: &str, open: usize) -> Option<usize> {
    let bytes = s.as_bytes();
    let mut depth = 0usize;
    let mut quote = None;
    let mut i = open;
    while i < bytes.len() {
        let b = bytes[i];
        match quote {
            Some(q) => {
                if b == b'\\' {
                    i += 1;
                } else if b == q {
                    quote = None;
                }
            }
            None => match b {
                b'"' | b'\'' | b'`' => quote = Some(b),
                b'(' | b'[' | b'{' => depth += 1,
                b')' | b']' | b'}' => {
                    depth = depth.checked_sub(1)?;
                    if depth == 0 {
                        return Some(i);
                    }
                }
                _ => {}
            },
        }
        i += 1;
    }
    None
}

/// Splits on commas outside brackets and strings.
fn split_top(s: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut start = 0;
    let mut i = 0;
    let bytes = s.as_bytes();
    while i < bytes.len() {
        match bytes[i] {
            b'(' | b'[' | b'{' | b'"' | b'\'' | b'`' => {
                let end = if matches!(bytes[i], b'"' | b'\'' | b'`') {
                    string_end(s, i)
                } else {
                    closing(s, i)
                };
                match end {
                    Some(end) => i = end,
                    None => break,
                }
            }
            b',' => {
                parts.push(s[start..i].trim().to_string());
                start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    let last = s[start..].trim();
    if !last.is_empty() {
        parts.push(last.to_string());
    }
    parts
}

fn string_end(s: &str, open: usize) -> Option<usize> {
    let bytes = s.as_bytes();
    let q = bytes[open];
    let mut i = open + 1;
    while i < bytes.len() {
        if bytes[i] == b'\\' {
            i += 1;
        } else if bytes[i] == q {
            return Some(i);
        }
        i += 1;
    }
    None
}

/// The value of a string literal that `s` starts with.
fn string(s: &str) -> Option<String> {
    let q = *s.as_bytes().first()?;
    if !matches!(q, b'"' | b'\'') {
        return None;
    }
    let end = string_end(s, 0)?;
    let mut out = String::new();
    let mut chars = s[1..end].chars();
    while let Some(c) = chars.next() {
        out.push(if c == '\\' { chars.next()? } else { c });
    }
    Some(out)
}

fn boolean(s: &str) -> bool {
    matches!(s, "!0" | "true")
}

/// The contents of the brackets after each `name=` (as a whole identifier).
fn assignments<'a>(js: &'a str, name: &str, open: u8) -> Vec<&'a str> {
    let needle = format!("{name}={}", open as char);
    js.match_indices(&needle)
        .filter_map(|(at, m)| {
            if at > 0 && (is_ident_char(js.as_bytes()[at - 1]) || js.as_bytes()[at - 1] == b'.') {
                return None;
            }
            let open = at + m.len() - 1;
            Some(&js[open + 1..closing(js, open)?])
        })
        .collect()
}

/// `name={KEY:new Ctor(args),...}`: the first such object holding `ctor` calls, as
/// (key, args) pairs.
fn object(js: &str, name: &str, ctor: &str) -> Option<Vec<(String, Vec<String>)>> {
    let body = assignments(js, name, b'{')
        .into_iter()
        .find(|b| b.contains(ctor))?;
    Some(
        split_top(body)
            .into_iter()
            .filter_map(|entry| {
                let (key, value) = entry.split_once(':')?;
                let args = value.trim().strip_prefix(ctor)?.strip_suffix(')')?;
                Some((key.trim().to_string(), split_top(args)))
            })
            .collect(),
    )
}

/// `name=[a,b,...]`: the items of the first such array.
fn array(js: &str, name: &str) -> Option<Vec<String>> {
    assignments(js, name, b'[').first().map(|b| split_top(b))
}

/// The arguments of every `new Ctor(...)`.
fn calls(js: &str, ctor: &str) -> Vec<Vec<String>> {
    let needle = format!("new {ctor}(");
    js.match_indices(&needle)
        .filter_map(|(at, m)| {
            let open = at + m.len() - 1;
            Some(split_top(&js[open + 1..closing(js, open)?]))
        })
        .collect()
}

/// (type key, word) pairs of the micro template rule, in its order.
fn micro_words(js: &str) -> Option<Vec<(String, String)>> {
    let at = js.find("function getMicroTemplateURL(")?;
    let open = at + js[at..].find('{')?;
    let body = &js[open..=closing(js, open)?];
    let mut out = Vec::new();
    let mut rest = body;
    while let Some(i) = rest.find("Types.") {
        rest = &rest[i + "Types.".len()..];
        let key_len = rest
            .bytes()
            .position(|b| !is_ident_char(b))
            .unwrap_or(rest.len());
        let key = &rest[..key_len];
        let next_type = rest.find("Types.").unwrap_or(rest.len());
        if let Some(q) = rest[..next_type].find('"')
            && let Some(word) = string(&rest[q..])
        {
            out.push((key.to_string(), word));
        }
    }
    Some(out)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) const BODY: &str = include_str!("fixtures/body.js");

    pub(crate) fn tables() -> Tables {
        Tables::parse(BODY).unwrap()
    }

    #[test]
    fn finds_the_bundles_in_index_html() {
        let client = parse_index(include_str!("fixtures/index.html")).unwrap();
        assert_eq!(client.version, "2.3.7");
        assert_eq!(client.body_js, "js/dominion-webclient-body-2.3.7.min.js");
        assert_eq!(
            client.documents,
            [
                "js/dominion-webclient-head-2.3.7.min.js",
                "css/dominion-webclient-head-2.3.7.min.css",
                "css/dominion-webclient-body-2.3.7.min.css",
                "html/login-page-2.3.7.html",
            ]
        );
        assert!(parse_index("<html></html>").is_err());
    }

    #[test]
    fn reads_the_tables() {
        let t = tables();
        assert_eq!(&t.types[..3], ["Action", "Night", "Treasure"]);
        assert_eq!(t.no_mini_template, ["Traveller", "Randomizer"]);
        assert!(t.no_template.contains(&"Heirloom".to_string()));
        assert_eq!(t.micro[0], ("Action".into(), "action".into()));
        assert_eq!(t.micro.last().unwrap(), &("Trait".into(), "trait".into()));
        let court = t.cards.iter().find(|c| c.key == "KINGS_COURT").unwrap();
        assert_eq!(court.name, "Kings Court");
        assert_eq!(
            court.expansion.as_ref().unwrap().file_name.as_deref(),
            Some("prosperity")
        );
        assert!(court.kingdom_pile);
        let unused = t.cards.iter().find(|c| c.key == "UNUSED_SLOT_27").unwrap();
        assert_eq!(unused.expansion, None);
        let lands = t.cards.iter().find(|c| c.key == "DISTANT_LANDS").unwrap();
        assert_eq!(lands.types, ["Action", "Reserve", "Victory"]);
        assert!(t.base_cards.contains("PLATINUM") && t.base_cards.contains("CURSE"));
        assert!(t.heirloom_cards.contains("PIXIE"));
    }

    #[test]
    fn says_which_table_is_gone() {
        let without_cards = BODY.replace("CardNames={", "Cards={");
        let err = Tables::parse(&without_cards).unwrap_err();
        assert!(err.contains("CardNames"), "{err}");
        let err = Tables::parse(&BODY.replace("NO_MINI_TEMPLATE_TYPES", "X")).unwrap_err();
        assert!(err.contains("NO_MINI_TEMPLATE_TYPES"), "{err}");
    }

    #[test]
    fn finds_static_paths_named_in_full() {
        let js = literal_paths(BODY, "");
        assert!(js.contains("images/cards/templates/back.jpg"));
        assert!(js.contains("images/cards/templates/back-mini.jpg"));
        assert!(!js.iter().any(|p| !has_asset_extension(p)));
        let css = literal_paths(include_str!("fixtures/body.css"), "css");
        assert_eq!(
            css.into_iter().collect::<Vec<_>>(),
            [
                "fonts/TrajanPro3SemiBold.ttf",
                "images/cards/templates/randomizer-back-landscape.jpg",
                "images/elements/coin.png",
            ]
        );
        let html = literal_paths(include_str!("fixtures/index.html"), "");
        assert_eq!(
            html.into_iter().collect::<Vec<_>>(),
            ["images/dom-icon-black.png"]
        );
    }

    #[test]
    fn builds_the_constructed_paths() {
        let paths = constructed_paths(BODY);
        for p in [
            "images/tokens/action.png",
            "images/tokens/journey-token-back.png",
            "images/elements/potion-square.png",
            "images/buttons/1-card.png",
            "images/buttons/topdeck.png",
            "images/buttons/autoplay-treasures.png",
            "images/cubes/cube_red.png",
            "sounds/your-turn.mp3",
            "sounds/your-turn.ogg",
            "images/avatars/festival.webp",
            "images/elements/dark-ages-small.png",
            "images/box_art/darkages.jpg",
            "images/cards/templates/way-selector.webp",
        ] {
            assert!(paths.contains(p), "{p}");
        }
        assert!(!paths.iter().any(|p| p.contains("null")));
        assert!(paths.iter().all(|p| is_safe_path(p)));
    }

    #[test]
    fn rejects_paths_that_leave_the_cache() {
        assert!(is_safe_path("images/cards/art/base/village.webp"));
        assert!(is_safe_path("images/cards/art/plunder/king's-cache.webp"));
        for p in [
            "",
            "/etc/passwd",
            "images/../../x.png",
            "images//x.png",
            "a b.png",
        ] {
            assert!(!is_safe_path(p), "{p}");
        }
    }
}
