//! The agents' transcript docked left of the table: what the models think and do as the
//! game goes, following its end, or in a replay the point shown.

use super::use_game;
use crate::app::Route;
use crate::platform;
use crate::transcript::{self, Block, Body, Feed, label};
use dioxus::prelude::*;
use dominion_protocol::transcript::TranscriptKind as Kind;
use std::collections::BTreeMap;

/// The narrowest the table is laid out beside the dock; narrower windows get the dock
/// over the table.
const TABLE_MIN: f64 = 720.0;

/// The dock's width in a window `width` wide, and whether the table makes room for it.
pub fn width(width: f64) -> (f64, bool) {
    let w = (width * 0.3).clamp(300.0, 520.0).min(width);
    (w, width - w >= TABLE_MIN)
}

/// Whether the dock is open: as last left by someone watching, or by a seat (closed
/// until opened), since someone playing may not want to read their opponents' reasoning.
#[derive(Clone, Copy, PartialEq)]
pub struct DockOpen {
    watching: Memo<bool>,
    changed: Signal<u32>,
    pub open: Memo<bool>,
}

pub fn use_open(watching: Memo<bool>) -> DockOpen {
    let changed = use_signal(|| 0);
    let open = use_memo(move || {
        let _ = changed();
        let watching = watching();
        platform::storage_get(open_key(watching)).map_or(watching, |v| v == "1")
    });
    DockOpen {
        watching,
        changed,
        open,
    }
}

impl DockOpen {
    pub fn set(mut self, now: bool) {
        platform::storage_set(open_key(*self.watching.peek()), if now { "1" } else { "0" });
        self.changed += 1;
    }
}

fn open_key(watching: bool) -> &'static str {
    if watching {
        "dominion.agentLog.watching"
    } else {
        "dominion.agentLog.playing"
    }
}

/// How many of `blocks` were written by the time the game had logged `seen` entries:
/// those before the first game line past them. All of them when `seen` is none.
pub fn written_by(blocks: &[Block], seen: Option<usize>) -> usize {
    let Some(seen) = seen else {
        return blocks.len();
    };
    (blocks.iter())
        .position(|b| b.game_seq().is_some_and(|s| s >= seen))
        .unwrap_or(blocks.len())
}

/// What the compact view shows: the models' own words and moves.
fn compact(kind: Kind) -> bool {
    matches!(
        kind,
        Kind::Reasoning | Kind::Message | Kind::Action | Kind::Status
    )
}

#[component]
pub fn AgentDock(id: String, width: f64, beside: bool, open: DockOpen) -> Element {
    let ctx = use_game();
    let game = use_memo(use_reactive!(|id| id));
    let Feed {
        timeline,
        error,
        ready,
    } = transcript::use_feed(game, true);
    let mut follow = use_signal(|| true);
    let mut everything = use_signal(|| {
        platform::storage_get("dominion.agentLog.everything").as_deref() == Some("1")
    });
    let mut seat = use_signal(|| None::<u8>);
    let mut dragging = use_signal(|| false);
    let until = ctx.offers.read().transcript_at;

    let state = timeline.read();
    let written = &state.blocks[..written_by(&state.blocks, until)];
    let agents: BTreeMap<u8, String> = (written.iter())
        .filter(|b| b.kind != Kind::Game)
        .map(|b| (b.seat, b.agent.clone()))
        .collect();
    let blocks: Vec<&Block> = (written.iter())
        .filter(|b| seat().is_none_or(|s| b.seat == s || b.kind == Kind::Game))
        .filter(|b| everything() || compact(b.kind))
        .collect();
    // Each seat's latest reasoning stays open; earlier reasoning folds to its first line.
    let mut latest = BTreeMap::new();
    for b in blocks.iter().filter(|b| b.kind == Kind::Reasoning) {
        latest.insert(b.seat, b.sequence);
    }
    let tick = (
        blocks.len(),
        blocks.last().map_or(0, |b| b.text.len()),
        until,
    );
    let blocks: Vec<Block> = blocks.into_iter().cloned().collect();
    drop(state);

    use_effect(use_reactive!(|tick| {
        let _ = tick;
        if *follow.peek() {
            document::eval(
                "requestAnimationFrame(() => { const e = document.getElementById('agent-dock-body'); if (e) e.scrollTop = e.scrollHeight; });",
            );
        }
    }));

    rsx! {
        document::Style { "{transcript::CSS}" }
        aside {
            class: if beside { "agent-dock" } else { "agent-dock over" },
            style: "width:{width:.0}px;",
            "aria-label": "Agent log",
            header { class: "agent-dock-head",
                span { class: "agent-dock-title", "Agent log" }
                button {
                    class: if follow() { "toggle on" } else { "toggle" },
                    title: if until.is_some() { "Keep the replay's point in view" } else { "Keep the latest in view" },
                    "aria-pressed": "{follow()}",
                    "data-agent-log": "follow",
                    onclick: move |_| {
                        let now = !follow();
                        follow.set(now);
                        if now {
                            document::eval("const e = document.getElementById('agent-dock-body'); if (e) e.scrollTop = e.scrollHeight;");
                        }
                    },
                    "Follow"
                }
                button {
                    class: if everything() { "toggle on" } else { "toggle" },
                    title: "Also the prompts, tool calls and their results, and the game's lines",
                    "aria-pressed": "{everything()}",
                    "data-agent-log": "everything",
                    onclick: move |_| {
                        let now = !everything();
                        platform::storage_set("dominion.agentLog.everything", if now { "1" } else { "0" });
                        everything.set(now);
                    },
                    "All"
                }
                a {
                    class: "button",
                    href: Route::Transcript { id: game() }.to_string(),
                    target: "_blank",
                    rel: "noopener",
                    title: "The whole transcript on its own page",
                    "↗"
                }
                button {
                    class: "toggle",
                    title: "Close the agent log",
                    "data-agent-log": "close",
                    onclick: move |_| open.set(false),
                    "✕"
                }
            }
            if agents.len() > 1 {
                div { class: "agent-dock-seats", role: "group", "aria-label": "Seats",
                    button {
                        class: if seat().is_none() { "toggle on" } else { "toggle" },
                        onclick: move |_| seat.set(None),
                        "All"
                    }
                    for (s, name) in agents {
                        button {
                            key: "{s}",
                            class: if seat() == Some(s) { "toggle on" } else { "toggle" },
                            style: "--seat: var(--seat-{s % 6})",
                            title: "{name}",
                            onclick: move |_| seat.set(Some(s)),
                            span { class: "seat-dot" }
                            "P{s + 1}"
                        }
                    }
                }
            }
            div {
                id: "agent-dock-body",
                class: "agent-dock-body",
                // Only the reader stops following (scrolling up, or dragging the
                // scrollbar away from the end); reaching the end starts it again.
                onwheel: move |e| {
                    if e.delta().strip_units().y < 0.0 && *follow.peek() {
                        follow.set(false);
                    }
                },
                onpointerdown: move |_| dragging.set(true),
                onpointerup: move |_| dragging.set(false),
                onpointerleave: move |_| dragging.set(false),
                onscroll: move |e| {
                    let end = e.scroll_top() + f64::from(e.client_height()) >= f64::from(e.scroll_height()) - 40.0;
                    if end && !*follow.peek() {
                        follow.set(true);
                    } else if !end && *dragging.peek() && *follow.peek() {
                        follow.set(false);
                    }
                },
                if let Some(message) = error() {
                    p { class: "transcript-error", role: "alert", "{message}" }
                }
                if !ready() && error().is_none() {
                    p { class: "transcript-empty", role: "status", "Loading…" }
                }
                if ready() && blocks.is_empty() {
                    p { class: "transcript-empty", role: "status",
                        if until.is_some() { "Nothing from the agents by this point." } else { "Waiting for agent activity." }
                    }
                }
                ol { class: "transcript-timeline",
                    for block in blocks {
                        li {
                            key: "{block.sequence}",
                            class: format!("trace-block trace-{:?}", block.kind).to_lowercase(),
                            style: "--seat: var(--seat-{block.seat % 6})",
                            title: "{transcript::clock(block.time)} UTC",
                            div { class: "trace-content",
                                if block.kind == Kind::Game {
                                    p { class: "trace-game-line", "{block.text}" }
                                } else {
                                    header {
                                        span { class: "trace-agent", span { class: "seat-dot" } "P{block.seat + 1}" }
                                        span { class: "trace-kind", "{label(block.kind)}" }
                                    }
                                    if block.kind == Kind::Reasoning && latest.get(&block.seat) != Some(&block.sequence) {
                                        details {
                                            summary { "{transcript::summary(&block.text).0}" }
                                            pre { "{block.text}" }
                                        }
                                    } else {
                                        Body { kind: block.kind, text: block.text.clone(), expand: false }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transcript::Timeline;
    use dominion_protocol::transcript::{TranscriptEntry, TranscriptInput, TranscriptPage};

    fn entry(sequence: u64, kind: Kind, block: &str) -> TranscriptEntry {
        TranscriptEntry {
            sequence,
            seat: 1,
            agent: "Agent".into(),
            activity: TranscriptInput {
                attempt: "a".into(),
                offset: sequence,
                time: sequence,
                kind,
                block: block.into(),
                text: block.into(),
                append: false,
            },
        }
    }

    #[test]
    fn a_replay_shows_what_was_written_before_the_game_went_past_it() {
        let mut t = Timeline::default();
        t.merge(TranscriptPage {
            entries: vec![
                entry(1, Kind::Game, "game/0"),
                entry(2, Kind::Game, "game/1"),
                entry(3, Kind::Reasoning, "think"),
                entry(4, Kind::Action, "buy"),
                entry(5, Kind::Game, "game/2"),
                entry(6, Kind::Reasoning, "later"),
            ],
            next: 6,
            more: false,
        });
        let b = &t.blocks;
        assert_eq!(written_by(b, None), 6);
        // Log entries 0 and 1 shown: the reasoning and move that came after them.
        assert_eq!(written_by(b, Some(2)), 4);
        assert_eq!(written_by(b, Some(1)), 1);
        assert_eq!(written_by(b, Some(0)), 0);
        assert_eq!(written_by(b, Some(9)), 6);
    }
}
