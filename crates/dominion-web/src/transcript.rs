//! A separate, resumable timeline of agent output and Dominion actions.

use crate::api;
use crate::app::Route;
use dioxus::prelude::*;
use dominion_protocol::lobby::OFFICIAL_GAME_PREFIX;
use dominion_protocol::transcript::{
    TranscriptEntry, TranscriptKind as Kind, TranscriptPage as Page,
};
use futures_util::StreamExt;
use std::collections::{BTreeMap, HashMap};

const CSS: &str = include_str!("../assets/transcript.css");

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Block {
    key: String,
    pub sequence: u64,
    pub time: u64,
    pub seat: u8,
    pub agent: String,
    pub kind: Kind,
    pub text: String,
}

#[derive(Clone, Default)]
pub struct Timeline {
    pub blocks: Vec<Block>,
    pub next: u64,
    index: HashMap<String, usize>,
}

impl Timeline {
    pub fn merge(&mut self, page: Page) {
        for entry in page.entries {
            if entry.sequence <= self.next {
                continue;
            }
            let TranscriptEntry {
                sequence,
                seat,
                agent,
                activity: a,
            } = entry;
            let key = if a.kind == Kind::Game {
                a.block.clone()
            } else {
                format!("{seat}/{}/{:?}/{}", a.attempt, a.kind, a.block)
            };
            if let Some(&i) = self.index.get(&key) {
                if a.kind == Kind::Game {
                    continue;
                }
                if a.append {
                    self.blocks[i].text.push_str(&a.text);
                } else {
                    self.blocks[i].text = a.text;
                }
            } else {
                self.index.insert(key.clone(), self.blocks.len());
                self.blocks.push(Block {
                    key,
                    sequence,
                    time: a.time,
                    seat,
                    agent,
                    kind: a.kind,
                    text: a.text,
                });
            }
        }
        self.blocks.sort_by_key(|b| (b.time, b.sequence));
        self.index = self
            .blocks
            .iter()
            .enumerate()
            .map(|(i, b)| (b.key.clone(), i))
            .collect();
        self.next = self.next.max(page.next);
    }
}

async fn refresh(
    mut timeline: Signal<Timeline>,
    game: &str,
    token: Option<&str>,
) -> api::Result<()> {
    loop {
        let after = timeline.peek().next;
        let page = api::transcript(game, token, after).await?;
        let more = page.more;
        timeline.write().merge(page);
        if !more {
            return Ok(());
        }
    }
}

fn label(kind: Kind) -> &'static str {
    match kind {
        Kind::System => "System prompt",
        Kind::Observation => "Prompt",
        Kind::Reasoning => "Reasoning",
        Kind::Message => "Model reply",
        Kind::Call => "Tool call",
        Kind::Action => "Dominion action",
        Kind::Game => "Game",
        Kind::Result => "Tool result",
        Kind::Status => "Status",
    }
}

/// A collapsed block's one-line summary: its first non-empty line, shortened, and how
/// many lines it has.
fn summary(text: &str) -> (String, usize) {
    let first = text
        .lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("")
        .trim();
    let short = match first.char_indices().nth(140) {
        Some((end, _)) => format!("{}…", &first[..end]),
        None => first.to_string(),
    };
    (short, text.lines().count().max(1))
}

/// Whether a block is short enough to show open without being asked.
fn short(text: &str) -> bool {
    text.len() <= 240 && text.lines().count() <= 3
}

fn clock(time: u64) -> String {
    let seconds = time / 1000;
    format!(
        "{:02}:{:02}:{:02}.{:03}",
        (seconds / 3600) % 24,
        (seconds / 60) % 60,
        seconds % 60,
        time % 1000
    )
}

#[component]
pub fn TranscriptPage(id: String) -> Element {
    let game = use_signal(move || id);
    let timeline = use_signal(Timeline::default);
    let mut error = use_signal(|| None::<String>);
    let mut ready = use_signal(|| false);
    let mut selected = use_signal(|| None::<u8>);
    let reasoning = use_signal(|| true);
    let follow = use_signal(|| true);
    let inputs = use_signal(|| true);
    let tools = use_signal(|| true);
    let expand = use_signal(|| false);
    let account = crate::account::use_account();
    use_resource(move || {
        let _ = account.name();
        let game = game();
        let token = crate::session::seats(&game)
            .first()
            .map(|s| s.token.clone());
        async move {
            match refresh(timeline, &game, token.as_deref()).await {
                Ok(()) => {
                    error.set(None);
                    ready.set(true);
                }
                Err(e) => {
                    error.set(Some(e.to_string()));
                    return;
                }
            }
            let (send, mut receive) = futures_channel::mpsc::unbounded();
            let changed = async {
                while receive.next().await.is_some() {
                    match refresh(timeline, &game, token.as_deref()).await {
                        Ok(()) => error.set(None),
                        Err(e) => error.set(Some(e.to_string())),
                    }
                }
            };
            let feed = api::watch_transcript(&game, token.as_deref(), move |_| {
                let _ = send.unbounded_send(());
            });
            futures_util::future::join(feed, changed).await;
        }
    });
    use_effect(move || {
        let _ = timeline.read().next;
        if follow() {
            #[cfg(all(feature = "web", target_arch = "wasm32"))]
            document::eval(
                "document.querySelector('[data-trace-end]')?.scrollIntoView({block:'end'})",
            );
        }
    });
    // An official account's game is shown on that account's page.
    let lobby = crate::lobby::use_lobby().lobby;
    let back = match game().strip_prefix(OFFICIAL_GAME_PREFIX) {
        None => Route::Game { id: game() },
        Some(official) => (lobby.read().iter().flat_map(|l| &l.official))
            .find(|a| a.game.as_ref().is_some_and(|g| g.id == official))
            .map_or(Route::Lobby {}, |a| Route::OnlineAccount {
                account: a.id.clone(),
            }),
    };
    let state = timeline.read();
    let agents: BTreeMap<u8, String> = state
        .blocks
        .iter()
        .filter(|b| b.kind != Kind::Game)
        .map(|b| (b.seat, b.agent.clone()))
        .collect();
    let blocks: Vec<_> = state
        .blocks
        .iter()
        .filter(|b| selected().is_none_or(|s| b.seat == s || b.kind == Kind::Game))
        .filter(|b| reasoning() || b.kind != Kind::Reasoning)
        .filter(|b| inputs() || !matches!(b.kind, Kind::Observation | Kind::System))
        .filter(|b| tools() || !matches!(b.kind, Kind::Call | Kind::Result))
        .cloned()
        .collect();
    rsx! {
        document::Title { "Agent transcript · Dominion" }
        document::Style { "{CSS}" }
        section { class:"agent-transcript",
            header { class:"transcript-heading",
                div {
                    p { class:"transcript-eyebrow", "Game {game}" }
                    h1 { "Agent Transcript" }
                    p { class:"transcript-description", "Everything each model was given and everything it returned, in order: its system prompt, prompts, reasoning, replies, tool calls and what they returned, with the game's events. Long inputs and results are folded; open them, or expand everything. Transcripts may reveal hidden cards. Reasoning appears when the model provides it. Times are UTC." }
                }
                Link { class:"button copper",to:back,"Back to game" }
            }
            div { class:"transcript-body",
                aside { class:"transcript-side",
                    section { class:"panel", role:"group", "aria-label":"Seats",
                        h3 { "Seats" }
                        div { class:"transcript-seats",
                            button { class:if selected().is_none(){"toggle on"}else{"toggle"}, "aria-pressed":"{selected().is_none()}", onclick:move |_|selected.set(None), "All seats" }
                            for (seat,name) in agents {
                                button { key:"{seat}", class:if selected()==Some(seat){"toggle on"}else{"toggle"}, "aria-pressed":"{selected()==Some(seat)}", style:"--seat: var(--seat-{seat % 6})", onclick:move |_|selected.set(Some(seat)),
                                    span { class:"seat-dot" } "P{seat+1} · {name}"
                                }
                            }
                        }
                    }
                    section { class:"panel", role:"group", "aria-label":"Show",
                        h3 { "Show" }
                        Switch { label:"Reasoning", on:reasoning }
                        Switch { label:"Prompts", on:inputs }
                        Switch { label:"Tool calls and results", on:tools }
                        Switch { label:"Expand everything", on:expand }
                        Switch { label:"Follow latest", on:follow }
                    }
                }
                section { class:"panel transcript-main",
                    h3 { "Timeline" }
                    if let Some(message)=error() { p {class:"transcript-error",role:"alert","{message}"} }
                    if !ready() && error().is_none() {p {class:"transcript-empty",role:"status","Loading transcript…"}}
                    if ready() && blocks.is_empty() {p {class:"transcript-empty",role:"status","Waiting for agent activity. New output will appear here automatically."}}
                    ol {class:"transcript-timeline", "aria-label":"Agent activity",
                        for block in blocks {
                            li {key:"{block.sequence}",class:format!("trace-block trace-{:?}",block.kind).to_lowercase(), style:"--seat: var(--seat-{block.seat % 6})",
                                time {"{clock(block.time)}"}
                                div { class:"trace-content",
                                    if block.kind==Kind::Game {
                                        p { class:"trace-game-line", span {class:"trace-kind","Game"} "{block.text}" }
                                    } else {
                                        header {
                                            span {class:"trace-kind","{label(block.kind)}"}
                                            span {class:"trace-agent", span { class:"seat-dot" } "P{block.seat+1} · {block.agent}"}
                                        }
                                        Body { kind: block.kind, text: block.text.clone(), expand: expand() }
                                    }
                                }
                            }
                        }
                    }
                    div {"data-trace-end":"", "aria-hidden":"true"}
                }
            }
        }
    }
}

/// A block's text: folded behind a summary when it is model input or a tool's result
/// too long to read at a glance.
#[component]
fn Body(kind: Kind, text: String, expand: bool) -> Element {
    let (first, lines) = summary(&text);
    let open = expand || short(&text);
    match kind {
        Kind::System => rsx! {
            details { open: expand, summary { "{lines} lines" } pre { "{text}" } }
        },
        Kind::Observation => rsx! {
            details { open: expand, summary { "{first}" span { class:"trace-lines", " · {lines} lines" } } pre { "{text}" } }
        },
        Kind::Result | Kind::Call if !short(&text) => rsx! {
            details { open, summary { "{first}" span { class:"trace-lines", " · {lines} lines" } } pre { "{text}" } }
        },
        _ => rsx! { pre { "{text}" } },
    }
}

/// A labelled on/off switch, the lobby's, over a rule.
#[component]
fn Switch(label: String, on: Signal<bool>) -> Element {
    let mut on = on;
    rsx! {
        div { class:"transcript-switch-row",
            span { "{label}" }
            span { class:"transcript-rule" }
            button {
                class: if on() { "transcript-switch active" } else { "transcript-switch" },
                role: "switch",
                "aria-checked": "{on()}",
                "aria-label": "{label}",
                onclick: move |_| on.toggle(),
                span { class:"transcript-knob" }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dominion_protocol::transcript::TranscriptInput;

    fn entry(
        sequence: u64,
        time: u64,
        seat: u8,
        kind: Kind,
        block: &str,
        text: &str,
        append: bool,
    ) -> TranscriptEntry {
        TranscriptEntry {
            sequence,
            seat,
            agent: format!("Agent {seat}"),
            activity: TranscriptInput {
                attempt: "attempt".into(),
                offset: sequence,
                time,
                kind,
                block: block.into(),
                text: text.into(),
                append,
            },
        }
    }

    #[test]
    fn replay_deduplicates_deltas_and_snapshots_without_losing_action_order() {
        let mut model = Timeline::default();
        let entries = vec![
            entry(1, 1, 0, Kind::Reasoning, "thought", "Buy ", true),
            entry(2, 2, 0, Kind::Reasoning, "thought", "Silver", true),
            entry(3, 3, 0, Kind::Reasoning, "thought", "Buy Silver", false),
            entry(4, 4, 0, Kind::Action, "action", "Buy Silver ($3)", false),
        ];
        let page = Page {
            entries,
            next: 4,
            more: false,
        };
        model.merge(page.clone());
        model.merge(page);
        assert_eq!(model.blocks.len(), 2);
        assert_eq!(model.blocks[0].text, "Buy Silver");
        assert_eq!(model.blocks[1].kind, Kind::Action);
    }

    #[test]
    fn delayed_uploads_sort_by_capture_time_and_seats_do_not_share_blocks() {
        let mut model = Timeline::default();
        model.merge(Page {
            entries: vec![entry(1, 20, 0, Kind::Action, "a", "Action", false)],
            next: 1,
            more: false,
        });
        model.merge(Page {
            entries: vec![
                entry(2, 10, 1, Kind::Reasoning, "a", "Earlier reasoning", false),
                entry(3, 30, 0, Kind::Action, "a", "Corrected action", false),
            ],
            next: 3,
            more: false,
        });
        assert_eq!(model.blocks.len(), 2);
        assert_eq!(model.blocks[0].text, "Earlier reasoning");
        assert_eq!(model.blocks[1].text, "Corrected action");
    }
}
