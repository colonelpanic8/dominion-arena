//! The stylesheets take the palette from tokens.css: none writes out a token's colour, or
//! one of the near-duplicates the tokens replaced, in place of the token. The game table
//! and the cards keep dominion.games' own colours, and are not held to it.

use std::fs;
use std::path::Path;

/// Stylesheets drawn in dominion.games' colours rather than ours.
const OWN_COLOURS: &[&str] = &["tokens.css", "table.css", "cards.css"];

/// Colours a token replaced, each with the token to use instead.
const RETIRED: &[(&str, &str)] = &[
    ("#b08a3e", "gold-deep"),
    ("#c79a3a", "gold-deep"),
    ("#ffef64", "gold-glow"),
    ("#e79535", "ember"),
    ("#93431f", "copper"),
    ("#dddddd", "text"),
    ("#eeeeee", "text"),
    ("#e9e3d6", "text"),
    ("#e8e1d0", "text"),
    ("#c8c0ae", "text-soft"),
    ("#cccccc", "text-soft"),
    ("#bbbbbb", "text-soft"),
    ("#c8c8c8", "text-soft"),
    ("#b9b2a3", "text-muted"),
    ("#aaaaaa", "text-muted"),
    ("#a29e8b", "text-muted"),
    ("#888888", "text-faint"),
    ("#777777", "text-faint"),
    ("#77705f", "text-faint"),
    ("#e0705a", "bad"),
];

/// `#abc` and `#aabbcc` as `#aabbcc`; None for anything else.
fn hex(s: &str) -> Option<String> {
    let digits = s.strip_prefix('#')?;
    if !digits.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let d = digits.to_ascii_lowercase();
    match d.len() {
        3 => Some(d.chars().flat_map(|c| [c, c]).collect()),
        6 => Some(d),
        _ => None,
    }
    .map(|d| format!("#{d}"))
}

/// Every colour a line writes out, as `#rrggbb`: hex literals, and `rgb()` / `rgba()`.
fn colours(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let bytes = line.as_bytes();
    for (i, _) in line.match_indices('#') {
        let end = (i + 1..bytes.len())
            .find(|&j| !bytes[j].is_ascii_alphanumeric())
            .unwrap_or(bytes.len());
        out.extend(hex(&line[i..end]));
    }
    for (i, _) in line.match_indices("rgb") {
        let Some(open) = line[i..].find('(') else {
            continue;
        };
        let Some(close) = line[i + open..].find(')') else {
            continue;
        };
        let parts: Vec<u8> = line[i + open + 1..i + open + close]
            .split(',')
            .take(3)
            .filter_map(|p| p.trim().parse().ok())
            .collect();
        if let [r, g, b] = parts[..] {
            out.push(format!("#{r:02x}{g:02x}{b:02x}"));
        }
    }
    out
}

#[test]
fn stylesheets_use_the_tokens() {
    let assets = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets");
    let tokens = fs::read_to_string(assets.join("tokens.css")).unwrap();
    let mut refused: Vec<(String, String)> = RETIRED
        .iter()
        .map(|(c, t)| (c.to_string(), t.to_string()))
        .collect();
    for line in tokens.lines() {
        let Some((name, value)) = line.trim().split_once(':') else {
            continue;
        };
        let (Some(name), Some(value)) = (
            name.strip_prefix("--"),
            hex(value.trim().trim_end_matches(';')),
        ) else {
            continue;
        };
        refused.push((value, name.to_string()));
    }
    assert!(
        refused.len() > RETIRED.len(),
        "tokens.css defines no colours"
    );

    let mut found = Vec::new();
    let mut sheets: Vec<_> = fs::read_dir(&assets)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "css"))
        .filter(|p| !OWN_COLOURS.iter().any(|own| p.ends_with(own)))
        .collect();
    sheets.sort();
    for sheet in sheets {
        let name = sheet.file_name().unwrap().to_string_lossy().into_owned();
        for (n, line) in fs::read_to_string(&sheet).unwrap().lines().enumerate() {
            for colour in colours(line) {
                if let Some((_, token)) = refused.iter().find(|(c, _)| *c == colour) {
                    found.push(format!("{name}:{}: {colour} is var(--{token})", n + 1));
                }
            }
        }
    }
    assert!(
        found.is_empty(),
        "colours written out in place of a token:\n{}",
        found.join("\n")
    );
}
