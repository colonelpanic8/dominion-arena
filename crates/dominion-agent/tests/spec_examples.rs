//! The `dominion/events@1` spec (`protocol/events.md`) held to the code: its example
//! streams are what the generator below writes now, each folds and replays to itself,
//! every event it quotes is a line of one of them, and it names every type and field of
//! `dominion_protocol::events`.
//!
//! `just events-examples` writes the examples again (`EVENTS_EXAMPLES=write`).

use dominion_agent::events::Recorder;
use dominion_agent::recording::{self, Encoding};
use dominion_engine::{Card, Game, GameConfig, Response};
use dominion_protocol::events::{Event, Header, SCHEMA, Table, Viewer};
use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

mod support;
use support::responders::{eager_response, random_response};

const REGENERATE: &str = "run `just events-examples` to write them again, then check every \
    excerpt protocol/events.md quotes from them (`just check` holds the excerpts to the files)";

type Responder = fn(&Game, &mut ChaCha8Rng) -> Response;

/// A two-player game, Ann against Bo, whose seats answer with `respond` seeded by `seed`.
struct Example {
    name: &'static str,
    /// The header's game id.
    game: Option<&'static str>,
    kingdom: &'static [Card],
    landscapes: &'static [Card],
    seed: u64,
    turn_limit: u32,
    respond: Responder,
    /// Also written as CBOR.
    cbor: bool,
}

const MOAT_CHAPEL_KINGDOM: &[Card] = {
    use Card::*;
    &[
        ThroneRoom, Wharf, Moat, Militia, Chapel, Fortress, Haven, Village, Smithy, Market,
    ]
};

const EXAMPLES: &[Example] = {
    use Card::*;
    &[
        Example {
            name: "moat-chapel",
            game: Some("g12"),
            kingdom: MOAT_CHAPEL_KINGDOM,
            landscapes: &[],
            seed: 189,
            turn_limit: 40,
            respond: eager_response,
            cbor: true,
        },
        Example {
            name: "throne-room-wharf",
            game: None,
            kingdom: MOAT_CHAPEL_KINGDOM,
            landscapes: &[],
            seed: 15,
            turn_limit: 40,
            respond: eager_response,
            cbor: false,
        },
        Example {
            name: "outpost-fleet",
            game: None,
            kingdom: &[
                Outpost,
                Haven,
                Quartermaster,
                Grotto,
                Library,
                Village,
                Smithy,
                Market,
                Laboratory,
                Festival,
            ],
            landscapes: &[Fleet],
            seed: 12,
            turn_limit: 150,
            respond: eager_response,
            cbor: false,
        },
        Example {
            name: "boons-hexes",
            game: None,
            kingdom: &[
                Bard,
                BlessedVillage,
                Skulk,
                Druid,
                Bridge,
                Ranger,
                FlagBearer,
                Village,
                Smithy,
                Market,
            ],
            landscapes: &[Pageant],
            seed: 4,
            turn_limit: 60,
            respond: eager_response,
            cbor: false,
        },
        Example {
            name: "inherited",
            game: None,
            kingdom: &[
                Hermit,
                Rats,
                Marauder,
                Fortress,
                Procession,
                Quartermaster,
                Grotto,
                Village,
                Smithy,
                Market,
            ],
            landscapes: &[Inherited],
            seed: 1,
            turn_limit: 20,
            respond: random_response,
            cbor: false,
        },
        Example {
            name: "divine-wind",
            game: None,
            kingdom: &[
                YoungWitch,
                RusticVillage,
                Poet,
                TeaHouse,
                Kitsune,
                MountainShrine,
                RiverShrine,
                Village,
                Smithy,
                Market,
            ],
            landscapes: &[DivineWind],
            seed: 7,
            turn_limit: 16,
            respond: eager_response,
            cbor: false,
        },
        Example {
            name: "necromancer-star-chart",
            game: None,
            kingdom: &[
                Necromancer,
                Lurker,
                Stash,
                Wharf,
                Rats,
                Village,
                Smithy,
                Market,
                Masquerade,
                Bureaucrat,
            ],
            landscapes: &[StarChart, Citadel],
            seed: 89,
            turn_limit: 30,
            respond: eager_response,
            cbor: false,
        },
    ]
};

fn protocol_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../protocol")
}

fn examples_dir() -> PathBuf {
    protocol_dir().join("examples/events")
}

/// The example's events, played from its config.
fn play(example: &Example) -> Vec<Event> {
    let mut config = GameConfig::new(2, example.kingdom.to_vec(), example.seed);
    config.landscapes = example.landscapes.to_vec();
    config.turn_limit = Some(example.turn_limit);
    let mut game = Game::new_journaled(config).expect("the example sets up");
    let seats = ["Ann".to_string(), "Bo".to_string()];
    let mut recorder = Recorder::new();
    let mut rng = ChaCha8Rng::seed_from_u64(example.seed);
    let mut events = vec![];
    loop {
        events.extend(recorder.record(&mut game, &seats).expect("recorded"));
        if game.outcome().is_some() {
            return events;
        }
        let response = (example.respond)(&game, &mut rng);
        game.respond(response).expect("a legal answer");
    }
}

/// The example's files: name and bytes.
fn files(example: &Example) -> Vec<(String, Vec<u8>)> {
    let header = Header {
        schema: SCHEMA.into(),
        viewer: Viewer::Full,
        game: example.game.map(str::to_string),
        reconstructed: None,
    };
    let events = play(example);
    let mut encodings = vec![Encoding::JsonLines];
    if example.cbor {
        encodings.push(Encoding::Cbor);
    }
    (encodings.into_iter())
        .map(|encoding| {
            let mut bytes = vec![];
            encoding.write(&mut bytes, &header).unwrap();
            for event in &events {
                encoding.write(&mut bytes, event).unwrap();
            }
            (format!("{}{}", example.name, encoding.suffix()), bytes)
        })
        .collect()
}

/// The events of the example file `name`.
fn read(name: &str, bytes: &[u8]) -> Vec<Event> {
    let encoding = Encoding::of_file(name).expect("a stream file");
    let recording = recording::read(encoding, bytes, None)
        .unwrap_or_else(|e| panic!("{name} doesn't read: {e}"));
    assert_eq!(
        recording.len_of(recording.events.len()),
        Some(bytes.len()),
        "{name}: something follows its last event"
    );
    recording.events
}

#[test]
fn the_examples_are_what_the_generator_writes() {
    let dir = examples_dir();
    let write = std::env::var("EVENTS_EXAMPLES").is_ok_and(|v| v == "write");
    let mut expected = BTreeSet::new();
    for example in EXAMPLES {
        for (name, bytes) in files(example) {
            let path = dir.join(&name);
            if write {
                std::fs::create_dir_all(&dir).unwrap();
                std::fs::write(&path, &bytes).unwrap();
            } else {
                let there = std::fs::read(&path)
                    .unwrap_or_else(|e| panic!("{}: {e}; {REGENERATE}", path.display()));
                assert!(
                    there == bytes,
                    "{name} is not what the code writes now; {REGENERATE}"
                );
            }
            expected.insert(name);
        }
    }
    for entry in std::fs::read_dir(&dir).unwrap() {
        let name = entry.unwrap().file_name().into_string().unwrap();
        if !expected.contains(&name) {
            match write {
                true => std::fs::remove_file(dir.join(&name)).unwrap(),
                false => panic!("{name} is no example; {REGENERATE}"),
            }
        }
    }
}

#[test]
fn each_example_folds_and_replays_to_itself() {
    let dir = examples_dir();
    for example in EXAMPLES {
        let mut first: Option<Vec<Event>> = None;
        for (name, _) in files(example) {
            let bytes = std::fs::read(dir.join(&name)).unwrap();
            let events = read(&name, &bytes);
            let mut table = Table::default();
            for event in &events {
                table
                    .apply(event)
                    .unwrap_or_else(|e| panic!("{name} doesn't fold: {e}"));
            }
            assert!(table.ended.is_some(), "{name}: the game doesn't end");
            let replay = dominion_agent::replay::replay(&events)
                .unwrap_or_else(|d| panic!("{name} doesn't replay: {d}"));
            assert_eq!(replay.recorder.written(), events.len() as u64, "{name}");
            match &first {
                Some(other) => assert!(other == &events, "{name}: another encoding differs"),
                None => first = Some(events),
            }
        }
    }
}

/// The spec's fenced blocks: their info string, first line number and lines.
fn fenced_blocks(spec: &str) -> Vec<(String, usize, Vec<&str>)> {
    let mut blocks = vec![];
    let mut open: Option<(String, usize, Vec<&str>)> = None;
    for (i, line) in spec.lines().enumerate() {
        match (&mut open, line.strip_prefix("```")) {
            (None, Some(info)) => open = Some((info.trim().to_string(), i + 2, vec![])),
            (Some(_), Some("")) => blocks.push(open.take().unwrap()),
            (Some((_, _, lines)), _) => lines.push(line),
            (None, None) => {}
        }
    }
    assert!(open.is_none(), "a fenced block isn't closed");
    blocks
}

/// Whether `quoted`, where each `…` stands for any text, is the whole of `line`.
fn matches(quoted: &str, line: &str) -> bool {
    let mut parts = quoted.split('…');
    let first = parts.next().unwrap();
    let Some(mut rest) = line.strip_prefix(first) else {
        return false;
    };
    let parts: Vec<&str> = parts.collect();
    for (i, part) in parts.iter().enumerate() {
        if i == parts.len() - 1 {
            return rest.ends_with(part) && rest.len() >= part.len();
        }
        match rest.find(part) {
            Some(at) => rest = &rest[at + part.len()..],
            None => return false,
        }
    }
    rest.is_empty()
}

#[test]
fn every_event_the_spec_quotes_is_a_line_of_an_example() {
    let spec = std::fs::read_to_string(protocol_dir().join("events.md")).unwrap();
    let examples: Vec<(String, Vec<String>)> = (EXAMPLES.iter())
        .map(|e| {
            let name = format!("{}{}", e.name, Encoding::JsonLines.suffix());
            let text = std::fs::read_to_string(examples_dir().join(&name)).unwrap();
            (name, text.lines().map(str::to_string).collect())
        })
        .collect();
    let mut wrong = vec![];
    for (info, start, lines) in fenced_blocks(&spec) {
        let mut words = info.split_whitespace();
        let (Some(lang @ ("json" | "jsonl")), rest) = (words.next(), words.next()) else {
            continue;
        };
        match rest {
            Some("illustration") => continue,
            Some(other) => panic!("events.md:{start}: a {lang} block marked {other:?}"),
            None => {}
        }
        let quoted: Vec<(usize, &str)> = (lines.iter().enumerate())
            .map(|(i, l)| (start + i, *l))
            .filter(|(_, l)| *l != "…")
            .collect();
        for (n, line) in &quoted {
            if !(line.starts_with("{\"seq\":") || line.starts_with("{\"schema\":")) {
                wrong.push(format!(
                    "events.md:{n}: neither an event nor a header (mark the block \
                     `{lang} illustration` if it only shows a shape): {line}"
                ));
            }
        }
        let found = examples.iter().find(|(_, file)| {
            let mut from = 0;
            quoted.iter().all(
                |(_, q)| match (file[from..].iter()).position(|l| matches(q, l)) {
                    Some(i) => {
                        from += i + 1;
                        true
                    }
                    None => false,
                },
            )
        });
        if found.is_none() {
            for (n, q) in &quoted {
                if !examples
                    .iter()
                    .any(|(_, f)| f.iter().any(|l| matches(q, l)))
                {
                    wrong.push(format!("events.md:{n}: in no example: {q}"));
                }
            }
            wrong.push(format!(
                "events.md:{start}: the block's lines are not lines of one example, in order"
            ));
        }
    }
    assert!(
        wrong.is_empty(),
        "{}\nQuote the examples as they are now (if the format changed: {REGENERATE}).",
        wrong.join("\n")
    );
}

/// `snake_case` as `camelCase`.
fn camel(name: &str) -> String {
    let mut out = String::new();
    let mut upper = false;
    for c in name.chars() {
        match (c, upper) {
            ('_', _) => upper = true,
            (c, true) => {
                out.extend(c.to_uppercase());
                upper = false;
            }
            (c, false) => out.push(c),
        }
    }
    out
}

/// `PascalCase` as `kebab-case`.
fn kebab(name: &str) -> String {
    let mut out = String::new();
    for (i, c) in name.chars().enumerate() {
        if c.is_uppercase() && i > 0 {
            out.push('-');
        }
        out.extend(c.to_lowercase());
    }
    out
}

/// Every enum variant and field of the protocol's events module as written, by the item
/// it is of: variants in kebab-case (their text forms are, even without serde's renaming),
/// struct fields in camelCase unless renamed, the fields of `What`'s variants as named.
fn written_names(source: &str) -> Vec<(String, String)> {
    // Not part of the stream: the fold's own types and the move verbs (the move events'
    // types).
    const SKIP: &[&str] = &["Verb", "FoldError", "Done"];
    let mut names = vec![];
    let mut item: Option<(String, bool, bool)> = None; // name, enum, renamed to camelCase
    let mut rename: Option<String> = None;
    let mut camel_case = false;
    for line in source.lines() {
        if line.starts_with("#[cfg(test)]") {
            break;
        }
        if line.starts_with("#[serde(") {
            camel_case = line.contains("rename_all = \"camelCase\"");
            continue;
        }
        let decl = line
            .strip_prefix("pub ")
            .unwrap_or(line)
            .split_once(' ')
            .filter(|(kind, _)| matches!(*kind, "struct" | "enum"));
        if let Some((kind, rest)) = decl {
            let name = rest.split([' ', '{', '(', ';']).next().unwrap().to_string();
            item = (!SKIP.contains(&name.as_str()) && rest.trim_end().ends_with('{')).then_some((
                name,
                kind == "enum",
                camel_case,
            ));
            camel_case = false;
            continue;
        }
        if line == "}" {
            item = None;
            continue;
        }
        let Some((of, is_enum, renamed)) = &item else {
            continue;
        };
        let trimmed = line.trim_start();
        let depth = line.len() - trimmed.len();
        if trimmed.starts_with("#[serde(") {
            rename = trimmed
                .split_once("rename = \"")
                .map(|(_, r)| r.split('"').next().unwrap().to_string());
            continue;
        }
        if trimmed.starts_with("//") || trimmed.starts_with("#[") || trimmed.is_empty() {
            continue;
        }
        let ident: String = (trimmed.strip_prefix("pub ").unwrap_or(trimmed).chars())
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        if ident.is_empty() {
            continue;
        }
        let written = match (*is_enum, depth) {
            (true, 4) => kebab(&ident),
            (false, 4) if *renamed => camel(&ident),
            _ => ident.clone(),
        };
        let is_field = trimmed.contains(':');
        if (*is_enum && depth == 4 && !is_field) || is_field {
            names.push((of.clone(), rename.take().unwrap_or(written)));
        }
        rename = None;
    }
    names
}

/// The words of the spec's inline code, outside fenced blocks.
fn spec_words(spec: &str) -> BTreeSet<String> {
    let mut words = BTreeSet::new();
    let mut fenced = false;
    for line in spec.lines() {
        if line.starts_with("```") {
            fenced = !fenced;
            continue;
        }
        if fenced {
            continue;
        }
        for (i, code) in line.split('`').enumerate() {
            if i % 2 == 1 {
                words.extend(
                    code.split(|c: char| !(c.is_alphanumeric() || c == '-'))
                        .filter(|w| !w.is_empty())
                        .map(str::to_string),
                );
            }
        }
    }
    words
}

#[test]
fn the_spec_names_every_type_and_field() {
    // Written in other forms: a card as its name or `?`, a counter as its text, and the
    // fields serde flattens into the object around them.
    const INTERNAL: &[(&str, &str)] = &[
        ("CardRef", "hidden"),
        ("CardRef", "name"),
        ("CounterKey", "of"),
        ("CounterKey", "name"),
        ("Event", "what"),
        ("PileChange", "state"),
    ];
    let source = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../dominion-protocol/src/events.rs"),
    )
    .unwrap();
    let spec = std::fs::read_to_string(protocol_dir().join("events.md")).unwrap();
    let words = spec_words(&spec);
    let names = written_names(&source);
    assert!(names.len() > 200, "the source wasn't read: {names:?}");
    let mut missing: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (of, name) in names {
        if !words.contains(&name) && !INTERNAL.contains(&(of.as_str(), name.as_str())) {
            missing.entry(of).or_default().push(name);
        }
    }
    assert!(
        missing.is_empty(),
        "protocol/events.md doesn't name these (in inline code), by type: {missing:?}"
    );
}
