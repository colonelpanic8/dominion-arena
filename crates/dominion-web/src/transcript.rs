//! A separate, resumable timeline of agent output and Dominion actions: beside a game's chat
//! (`AgentsPane`), over it in full, or on a page of its own.

use crate::api;
use crate::app::Route;
use dioxus::prelude::*;
use dominion_protocol::lobby::OFFICIAL_GAME_PREFIX;
use dominion_protocol::transcript::{
    TranscriptEntry, TranscriptKind as Kind, TranscriptPage as Page,
};
use futures_util::StreamExt;
use std::collections::{BTreeMap, HashMap, HashSet};

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
    /// A tool call's result block's key, whether or not it has arrived.
    result: Option<String>,
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
            let result = (a.kind == Kind::Call)
                .then(|| result_block(&a.block))
                .flatten()
                .map(|block| format!("{seat}/{}/{:?}/{block}", a.attempt, Kind::Result));
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
                    result,
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

/// The block a tool call's result has: the call's with `call` replaced by `result`.
fn result_block(call: &str) -> Option<String> {
    if let Some(id) = call.strip_prefix("call/") {
        return Some(format!("result/{id}"));
    }
    let (before, id) = call.rsplit_once("/call/")?;
    Some(format!("{before}/result/{id}"))
}

/// Each shown block, a tool call with its result: a result whose call is shown is shown
/// with it instead of in its own place.
fn paired(blocks: Vec<Block>) -> Vec<(Block, Option<Block>)> {
    let calls: HashSet<&str> = blocks.iter().filter_map(|b| b.result.as_deref()).collect();
    let mut results: HashMap<String, Block> = blocks
        .iter()
        .filter(|b| b.kind == Kind::Result && calls.contains(b.key.as_str()))
        .map(|b| (b.key.clone(), b.clone()))
        .collect();
    let with_call: HashSet<String> = results.keys().cloned().collect();
    let mut shown = vec![];
    for b in blocks {
        if b.kind == Kind::Result && with_call.contains(&b.key) {
            continue;
        }
        let result = b.result.as_ref().and_then(|key| results.remove(key));
        shown.push((b, result));
    }
    shown
}

fn took(ms: u64) -> String {
    if ms < 1000 {
        format!("{ms} ms")
    } else {
        format!("{:.1} s", ms as f64 / 1000.0)
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

/// A game's transcript: loaded, then followed as it grows, while `game` names it.
#[derive(Clone, Copy, PartialEq)]
pub struct Feed {
    pub timeline: Signal<Timeline>,
    pub error: Signal<Option<String>>,
    pub ready: Signal<bool>,
}

pub fn use_feed(game: Memo<Option<String>>) -> Feed {
    let mut timeline = use_signal(Timeline::default);
    let mut error = use_signal(|| None::<String>);
    let mut ready = use_signal(|| false);
    let mut loaded = use_signal(|| None::<String>);
    let account = crate::account::use_account();
    use_resource(move || {
        let _ = account.name();
        let game = game();
        async move {
            let Some(game) = game else { return };
            if loaded.peek().as_ref() != Some(&game) {
                timeline.set(Timeline::default());
                ready.set(false);
                error.set(None);
                loaded.set(Some(game.clone()));
            }
            let token = crate::session::seats(&game)
                .first()
                .map(|s| s.token.clone());
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
    Feed {
        timeline,
        error,
        ready,
    }
}

/// What of a transcript is shown: one seat's or all, and which kinds of block.
#[derive(Clone, Copy, PartialEq)]
pub struct Show {
    seat: Signal<Option<u8>>,
    reasoning: Signal<bool>,
    inputs: Signal<bool>,
    tools: Signal<bool>,
    expand: Signal<bool>,
    follow: Signal<bool>,
}

pub fn use_show() -> Show {
    Show {
        seat: use_signal(|| None),
        reasoning: use_signal(|| true),
        inputs: use_signal(|| true),
        tools: use_signal(|| true),
        expand: use_signal(|| false),
        follow: use_signal(|| true),
    }
}

impl Show {
    fn blocks(&self, timeline: &Timeline) -> Vec<(Block, Option<Block>)> {
        let (seat, reasoning, inputs, tools) = (
            (self.seat)(),
            (self.reasoning)(),
            (self.inputs)(),
            (self.tools)(),
        );
        paired(
            timeline
                .blocks
                .iter()
                .filter(|b| seat.is_none_or(|s| b.seat == s || b.kind == Kind::Game))
                .filter(|b| reasoning || b.kind != Kind::Reasoning)
                .filter(|b| inputs || !matches!(b.kind, Kind::Observation | Kind::System))
                .filter(|b| tools || !matches!(b.kind, Kind::Call | Kind::Result))
                .cloned()
                .collect(),
        )
    }
}

/// The agents in a transcript, by seat.
fn agents(timeline: &Timeline) -> BTreeMap<u8, String> {
    (timeline.blocks.iter())
        .filter(|b| b.kind != Kind::Game)
        .map(|b| (b.seat, b.agent.clone()))
        .collect()
}

/// Keeps the newest block in sight while `follow` is on: `script` scrolls to it.
fn use_follow(feed: Feed, show: Show, script: &'static str) {
    use_effect(move || {
        let _ = feed.timeline.read().next;
        if (show.follow)() {
            #[cfg(all(feature = "web", target_arch = "wasm32"))]
            document::eval(script);
            #[cfg(not(all(feature = "web", target_arch = "wasm32")))]
            let _ = script;
        }
    });
}

/// Scrolls every transcript that scrolls by itself (`data-trace-scroll`) to its end.
const SCROLL_OWN: &str = "for (const e of document.querySelectorAll('[data-trace-scroll]')) e.scrollTop = e.scrollHeight";

const DESCRIPTION: &str = "Everything each model was given and everything it returned, in order: its system prompt, prompts, reasoning, replies, tool calls and what they returned, with the game's events. Long inputs and results are folded; open them, or expand everything. Transcripts may reveal hidden cards. Reasoning appears when the model provides it. Times are UTC.";

#[component]
pub fn TranscriptPage(id: String) -> Element {
    let game = use_signal(move || id);
    let feed = use_feed(use_memo(move || Some(game())));
    let show = use_show();
    use_follow(
        feed,
        show,
        "document.querySelector('[data-trace-end]')?.scrollIntoView({block:'end'})",
    );
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
    rsx! {
        document::Title { "Agent transcript · Dominion" }
        document::Style { "{CSS}" }
        Full { feed, show, game: game(),
            Link { class:"button copper",to:back,"Back to game" }
        }
    }
}

/// The whole transcript: the seats and what is shown beside the timeline. `children` is
/// the way back to the game.
#[component]
fn Full(feed: Feed, show: Show, game: String, children: Element) -> Element {
    rsx! {
        section { class:"agent-transcript",
            header { class:"transcript-heading",
                div {
                    p { class:"transcript-eyebrow", "Game {game}" }
                    h1 { "Agent Transcript" }
                    p { class:"transcript-description", "{DESCRIPTION}" }
                }
                {children}
            }
            div { class:"transcript-body",
                aside { class:"transcript-side",
                    section { class:"panel", role:"group", "aria-label":"Seats",
                        h3 { "Seats" }
                        Seats { feed, show }
                    }
                    section { class:"panel", role:"group", "aria-label":"Show",
                        h3 { "Show" }
                        Switch { label:"Reasoning", on:show.reasoning }
                        Switch { label:"Prompts", on:show.inputs }
                        Switch { label:"Tool calls and results", on:show.tools }
                        Switch { label:"Expand everything", on:show.expand }
                        Switch { label:"Follow latest", on:show.follow }
                    }
                }
                section { class:"panel transcript-main",
                    h3 { "Timeline" }
                    Entries { feed, show }
                    div {"data-trace-end":"", "aria-hidden":"true"}
                }
            }
        }
    }
}

#[component]
fn Seats(feed: Feed, show: Show) -> Element {
    let mut selected = show.seat;
    let agents = agents(&feed.timeline.read());
    rsx! {
        div { class:"transcript-seats",
            button { class:if selected().is_none(){"toggle on"}else{"toggle"}, "aria-pressed":"{selected().is_none()}", onclick:move |_|selected.set(None), "All seats" }
            for (seat,name) in agents {
                button { key:"{seat}", class:if selected()==Some(seat){"toggle on"}else{"toggle"}, "aria-pressed":"{selected()==Some(seat)}", style:"--seat: var(--seat-{seat % 6})", onclick:move |_|selected.set(Some(seat)),
                    span { class:"seat-dot" } "P{seat+1} · {name}"
                }
            }
        }
    }
}

/// The timeline, with what is wrong or why it is empty.
#[component]
fn Entries(feed: Feed, show: Show) -> Element {
    let (error, ready, expand) = ((feed.error)(), (feed.ready)(), (show.expand)());
    let blocks = show.blocks(&feed.timeline.read());
    rsx! {
        if let Some(message)=error.clone() { p {class:"transcript-error",role:"alert","{message}"} }
        if !ready && error.is_none() {p {class:"transcript-empty",role:"status","Loading transcript…"}}
        if ready && blocks.is_empty() {p {class:"transcript-empty",role:"status","Waiting for agent activity. New output will appear here automatically."}}
        ol {class:"transcript-timeline", "aria-label":"Agent activity",
            for (block, result) in blocks {
                li {key:"{block.sequence}",class:format!("trace-block trace-{:?}",block.kind).to_lowercase(), style:"--seat: var(--seat-{block.seat % 6})",
                    time {"{clock(block.time)}"}
                    div { class:"trace-content",
                        if block.kind==Kind::Game {
                            p { class:"trace-game-line", span {class:"trace-kind","Game"} "{block.text}" }
                        } else {
                            header {
                                span {class:"trace-kind", if result.is_some() { "Tool call and result" } else { "{label(block.kind)}" }}
                                span {class:"trace-agent", span { class:"seat-dot" } "P{block.seat+1} · {block.agent}"}
                            }
                            if block.kind == Kind::Call {
                                div { class:"trace-call-body",
                                    Body { kind: block.kind, text: block.text.clone(), expand }
                                }
                            } else {
                                Body { kind: block.kind, text: block.text.clone(), expand }
                            }
                            if let Some(result) = result {
                                div { class:"trace-result trace-paired",
                                    span { class:"trace-kind", "Result" }
                                    span { class:"trace-lines", " · {took(result.time.saturating_sub(block.time))}" }
                                    Body { kind: result.kind, text: result.text.clone(), expand }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// The transcript beside the game's chat: a seat to follow and the timeline, and the
/// whole transcript over the page on asking.
#[component]
pub fn AgentsPane(feed: Feed, show: Show, game: String) -> Element {
    let mut full = use_signal(|| false);
    let mut selected = show.seat;
    let agents = agents(&feed.timeline.read());
    use_follow(feed, show, SCROLL_OWN);
    rsx! {
        style { {CSS} }
        div { class: "transcript-pane",
            div { class: "transcript-pane-bar",
                if agents.len() > 1 {
                    select { "aria-label": "Seat",
                        value: selected().map_or(String::new(), |s| s.to_string()),
                        onchange: move |e| selected.set(e.value().parse().ok()),
                        option { value: "", "All seats" }
                        for (seat, name) in agents {
                            option { key: "{seat}", value: "{seat}", "P{seat + 1} · {name}" }
                        }
                    }
                }
                button { class: "toggle", "data-action": "transcript-full", "aria-haspopup": "dialog",
                    title: "The whole transcript, over the game",
                    onclick: move |e| { e.stop_propagation(); full.set(true); },
                    "Full screen"
                }
            }
            div { class: "transcript-pane-scroll", "data-trace-scroll": "",
                Entries { feed, show }
            }
        }
        if full() {
            dialog { class: "transcript-full", "data-trace-scroll": "", "aria-label": "Agent transcript",
                onmounted: move |_| {
                    #[cfg(all(feature = "web", target_arch = "wasm32"))]
                    document::eval(&format!("document.querySelector('dialog.transcript-full')?.showModal(); {SCROLL_OWN}"));
                },
                onkeydown: move |e| { if e.key() == Key::Escape { e.prevent_default(); full.set(false); } },
                onclick: move |e| e.stop_propagation(),
                Full { feed, show, game,
                    button { class: "copper", "data-action": "transcript-close", autofocus: true,
                        onclick: move |_| full.set(false), "Close" }
                }
            }
        }
    }
}

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
    fn results_are_shown_with_their_calls() {
        let mut model = Timeline::default();
        model.merge(Page {
            entries: vec![
                entry(1, 1, 0, Kind::Call, "call/c1", "state", false),
                entry(2, 2, 0, Kind::Action, "action-2", "Play Treasures", false),
                entry(3, 3, 0, Kind::Result, "result/c1", "STATUS", false),
                entry(4, 4, 0, Kind::Call, "r1/call/2", "answer", false),
                entry(5, 5, 0, Kind::Result, "r1/result/2", "GAME", false),
                entry(6, 6, 0, Kind::Result, "result/orphan", "lost", false),
                entry(7, 7, 0, Kind::Call, "call/c2", "wait", false),
            ],
            next: 7,
            more: false,
        });
        let shown: Vec<_> = paired(model.blocks)
            .into_iter()
            .map(|(b, r)| (b.text, r.map(|r| r.text)))
            .collect();
        assert_eq!(
            shown,
            [
                ("state".into(), Some("STATUS".into())),
                ("Play Treasures".into(), None),
                ("answer".into(), Some("GAME".into())),
                ("lost".into(), None),
                ("wait".into(), None),
            ]
        );
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
