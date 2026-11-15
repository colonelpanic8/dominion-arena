//! A separate, resumable timeline of agent output and Dominion actions.

use crate::api;
use crate::app::Route;
use dioxus::prelude::*;
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
        Kind::Observation => "Model input",
        Kind::Reasoning => "Reasoning",
        Kind::Message => "Model reply",
        Kind::Action => "Dominion action",
        Kind::Game => "Game",
        Kind::Result => "Tool result",
        Kind::Status => "Status",
    }
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
    let mut reasoning = use_signal(|| true);
    let mut follow = use_signal(|| true);
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
        .cloned()
        .collect();
    rsx! {
        document::Title { "Agent transcript · Dominion" }
        document::Style { "{CSS}" }
        section { class:"agent-transcript",
            header { class:"transcript-heading",
                div { p { class:"transcript-eyebrow", "GAME {game}" } h1 { "Agent transcript" } }
                Link { class:"button",to:Route::Game {id:game()},"Back to game" }
            }
            p { class:"transcript-description", "Live model output and Dominion actions, in order. Transcripts may reveal hidden cards. Reasoning appears when the model provides it. Times are UTC." }
            div { class:"transcript-controls", role:"group", "aria-label":"Transcript filters",
                button { class:if selected().is_none(){"on"}else{""}, onclick:move |_|selected.set(None), "All seats" }
                for (seat,name) in agents {
                    button { key:"{seat}",class:if selected()==Some(seat){"on"}else{""},onclick:move |_|selected.set(Some(seat)), "P{seat+1} · {name}" }
                }
                label { input {r#type:"checkbox",checked:reasoning(),onchange:move |e|reasoning.set(e.checked())} "Reasoning" }
                label { input {r#type:"checkbox",checked:follow(),onchange:move |e|follow.set(e.checked())} "Follow latest" }
            }
            if let Some(message)=error() { p {class:"transcript-error",role:"alert","{message}"} }
            if !ready() && error().is_none() {p {role:"status","Loading transcript…"}}
            if ready() && blocks.is_empty() {p {class:"transcript-empty",role:"status","Waiting for agent activity. New output will appear here automatically."}}
            ol {class:"transcript-timeline", "aria-label":"Agent activity",
                for block in blocks {
                    li {key:"{block.sequence}",class:format!("trace-block trace-{:?}",block.kind).to_lowercase(),
                        header {
                            span {class:"trace-kind","{label(block.kind)}"}
                            if block.kind!=Kind::Game {span {class:"trace-agent","P{block.seat+1} · {block.agent}"}}
                            time {"{clock(block.time)}"}
                        }
                        if block.kind==Kind::Observation {
                            details { summary {"Prompt and game observation"} pre {"{block.text}"} }
                        } else { pre {"{block.text}"} }
                    }
                }
            }
            div {"data-trace-end":"", "aria-hidden":"true"}
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
