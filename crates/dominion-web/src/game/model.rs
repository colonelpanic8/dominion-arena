//! The Model panel: a seat a person and a model share. Who holds it and the switch, what
//! the model wrote for whoever holds it, what to ask it, when it writes unasked, and for
//! an admin holding a model's own seat, that seat's decisions.
use super::plain::DecisionControls;
use super::use_game;
use crate::lobby::agent_picker::AgentPicker;
use crate::lobby::use_lobby;
use crate::{api, platform};
use dioxus::prelude::*;
use dominion_protocol::api::{AdviseMode, ModelAction, ModelNote, ModelRequest, ModelSeat};
use dominion_protocol::lobby::AgentSelection;

const CSS: &str = include_str!("../../assets/model.css");

/// How a note stands to the game now.
fn freshness(note: &ModelNote, decision: Option<&str>, turn: Option<u32>) -> &'static str {
    if note.decision.is_some() && note.decision.as_deref() == decision {
        "now"
    } else if note.turn.is_some() && note.turn == turn {
        "turn"
    } else {
        "old"
    }
}

/// Who holds `seat`, as its panel says it to the caller.
fn status(seat: &ModelSeat) -> String {
    let model = seat.agent.as_deref().unwrap_or("The model");
    match (seat.holding, seat.controlled, seat.own) {
        (true, _, true) => format!("You are playing. {model} reads along."),
        (true, _, false) => "You are playing this seat.".into(),
        (false, false, true) => format!("{model} is playing your seat."),
        (false, false, false) => "The model is playing.".into(),
        (false, true, _) => "An admin is playing.".into(),
    }
}

/// `mine`: the caller's own seat, where they may attach one of the server's agents.
#[component]
pub fn ModelControls(url: String, game: String, admin: bool, mine: Option<u8>) -> Element {
    let ctx = use_game();
    let lobby = use_lobby();
    let endpoint = use_signal(|| url);
    let mut seats = use_signal(Vec::<ModelSeat>::new);
    let mut open = use_signal(|| false);
    let mut busy = use_signal(|| false);
    let mut revision = use_signal(|| 0_u64);
    let selected = use_signal(|| None::<u8>);
    let mut draft = use_signal(String::new);
    let mut error = use_signal(|| None::<String>);
    let mut seen = use_signal(|| 0_usize);
    let mut picked = use_signal(|| None::<AgentSelection>);

    let refresh = use_callback(move |()| {
        spawn(async move {
            let before = *revision.peek();
            if *busy.peek() {
                return;
            }
            let url = endpoint.peek().clone();
            let result = api::model_seats(&url).await;
            if before != *revision.peek() || *busy.peek() {
                return;
            }
            match result {
                Ok(next) if *seats.peek() != next => seats.set(next),
                Err(e) if *open.peek() => error.set(Some(e.message)),
                _ => {}
            }
        });
    });
    use_future(move || async move {
        loop {
            refresh.call(());
            let near = *open.peek() || !seats.peek().is_empty();
            platform::sleep(if near { 2000 } else { 15000 }).await;
        }
    });
    // A note, a request or a change of holder bumps the game's version.
    let version = use_memo(move || ctx.state.read().version);
    use_effect(move || {
        version();
        if !seats.peek().is_empty() {
            refresh.call(());
        }
    });

    let act = use_callback(move |(seat, action): (u8, ModelAction)| {
        if *busy.peek() {
            return;
        }
        busy.set(true);
        *revision.write() += 1;
        error.set(None);
        let url = endpoint.peek().clone();
        let req = ModelRequest {
            game: game.clone(),
            seat,
            action,
        };
        spawn(async move {
            match api::model_action(&url, &req).await {
                Ok(next) => {
                    seats.set(next);
                    if matches!(req.action, ModelAction::Prompt { .. }) {
                        draft.set(String::new());
                    }
                }
                Err(e) => error.set(Some(e.message)),
            }
            busy.set(false);
        });
    });

    let available = seats();
    let current = available
        .iter()
        .find(|s| Some(s.seat) == selected())
        .or_else(|| available.iter().find(|s| s.holding))
        .or_else(|| available.iter().find(|s| s.own))
        .or(available.first())
        .cloned();
    let attachable = mine.filter(|seat| available.iter().all(|s| s.seat != *seat));
    if current.is_none() && attachable.is_none() {
        return rsx! {};
    }
    let notes = current.as_ref().map_or(0, |s| s.notes.len());
    if open() && seen() != notes {
        seen.set(notes);
    }
    let unread = notes.saturating_sub(seen());
    let (decision, turn) = {
        let state = ctx.state.read();
        let view = state.view.as_ref();
        (
            view.and_then(|v| v.decision.as_ref().map(|d| d.id.clone())),
            view.and_then(|v| v.turn.number),
        )
    };
    let models = lobby.lobby.read().as_ref().map(|l| l.house.models.clone());
    let models = models.unwrap_or_default();
    let holding = current.as_ref().is_some_and(|s| s.holding);
    rsx! {
        style { {CSS} }
        button { class: if holding { "copper" } else { "toggle" }, "aria-expanded": "{open()}",
            "data-action": "model-panel",
            onclick: move |_| open.toggle(),
            "Model"
            if unread > 0 { span { class: "model-unread", "{unread}" } }
        }
        if open() {
            aside { class: "model-panel", "aria-labelledby": "model-panel-title",
                onkeydown: move |e| { if e.key() == Key::Escape { e.prevent_default(); open.set(false); } },
                header {
                    div {
                        h2 { id: "model-panel-title", "Model" }
                        if let Some(s) = &current { p { class: "model-seat-name", "{s.name}" } }
                    }
                    button { class: "model-close", "aria-label": "Close the model panel",
                        onclick: move |_| open.set(false), "×" }
                }
                if let Some(current) = current {
                    Shared { current, available, decision, turn, admin, busy: busy(), draft, selected, act }
                } else if let Some(seat) = attachable {
                    p { class: "model-status", "No model is at your seat." }
                    p { class: "model-about",
                        "Attach one of the server's agents: you go on playing, it sees what you see and writes notes here when you ask. You can hand it the seat and take it back. It counts as one of your agent seats for today."
                    }
                    if models.is_empty() {
                        p { class: "muted", "The server's agents aren't available now." }
                    } else {
                        div { class: "model-send",
                            AgentPicker {
                                models,
                                value: picked(),
                                none: "Pick a model",
                                action: "pick-model",
                                disabled: busy(),
                                onpick: move |choice| picked.set(choice),
                            }
                            button { class: "primary", "data-action": "attach-model",
                                disabled: busy() || picked.read().is_none(),
                                onclick: move |_| {
                                    if let Some(choice) = picked.peek().clone() {
                                        let action = ModelAction::Attach { agent: choice.name().into(), options: choice.options() };
                                        act.call((seat, action));
                                    }
                                },
                                "Attach"
                            }
                        }
                    }
                }
                if let Some(message) = error() { p { class: "error", role: "alert", "{message}" } }
            }
        }
    }
}

/// The panel of a seat with a model at it.
#[component]
fn Shared(
    current: ModelSeat,
    available: Vec<ModelSeat>,
    decision: Option<String>,
    turn: Option<u32>,
    admin: bool,
    busy: bool,
    draft: Signal<String>,
    selected: Signal<Option<u8>>,
    act: Callback<(u8, ModelAction)>,
) -> Element {
    let seat = current.seat;
    let (holding, own) = (current.holding, current.own);
    let model = current
        .agent
        .clone()
        .unwrap_or_else(|| current.name.clone());
    let ask = if current.controlled {
        "Ask"
    } else {
        "Send prompt"
    };
    let modes = [
        (
            AdviseMode::Off,
            "When I ask",
            "It writes only when you ask.",
        ),
        (
            AdviseMode::Buys,
            "At my buys",
            "It writes once a turn, when you can buy.",
        ),
        (
            AdviseMode::Always,
            "Always",
            "It writes at every decision of yours: often after you have moved on.",
        ),
    ];
    rsx! {
        if available.len() > 1 {
            label { "Seat"
                select {
                    value: "{seat}", disabled: busy,
                    onchange: move |e| { selected.set(e.value().parse().ok()); draft.set(String::new()); },
                    for s in available { option { value: "{s.seat}", "{s.name}" } }
                }
            }
        }
        div { class: "model-status", span { class: "model-dot", "aria-hidden": "true" } "{status(&current)}" }
        if holding {
            div { class: "model-notes", "data-notes": "{current.notes.len()}",
                if current.notes.is_empty() {
                    p { class: "muted", "Nothing written yet." }
                }
                for (i, note) in current.notes.iter().enumerate().rev() {
                    article { key: "{i}", class: "model-note", "data-fresh": freshness(note, decision.as_deref(), turn),
                        if let Some(turn) = note.turn { span { class: "model-note-when", "Turn {turn}" } }
                        p { "{note.text}" }
                    }
                }
            }
            div { class: "model-send",
                small { if current.requested { "{model} is writing about your next decision." } else { "One note, on the decision you are at or reach next." } }
                button { class: "primary", "data-action": "request-advice", disabled: busy || current.requested,
                    onclick: move |_| act.call((seat, ModelAction::Request)), "Advise my next decision" }
            }
            div { class: "model-modes", role: "group", "aria-label": "When it writes",
                for (mode, name, about) in modes {
                    button { class: if current.advise == mode { "toggle on" } else { "toggle" }, title: about,
                        disabled: busy, "aria-pressed": "{current.advise == mode}",
                        onclick: move |_| act.call((seat, ModelAction::Advise { mode })), "{name}" }
                }
            }
        }
        label { r#for: "model-prompt", if current.controlled { "Ask {model}" } else { "Prompt the model" } }
        textarea { id: "model-prompt", rows: 2, maxlength: 2000, value: "{draft}",
            placeholder: if current.controlled { "What should I watch for?" } else { "What should it consider?" },
            disabled: busy,
            oninput: move |e| { selected.set(Some(seat)); draft.set(e.value()); },
        }
        div { class: "model-send",
            small { if current.controlled { "It answers in a note." } else { "Included in its next game observation." } }
            button { class: "copper", disabled: busy || draft.read().trim().is_empty(),
                onclick: move |_| act.call((seat, ModelAction::Prompt { text: draft.peek().trim().into() })), "{ask}" }
        }
        if !current.prompts.is_empty() {
            details { class: "model-history",
                summary { "Sent ({current.prompts.len()})" }
                ol { for text in current.prompts.iter().rev() { li { "{text}" } } }
            }
        }
        if let Some(playing) = current.playing {
            div { class: "model-hand",
                h3 { "Hand" }
                if playing.hand.count == 0 { span { "Empty" } }
                for (card, count) in playing.hand.known {
                    span { key: "{card}", "{card}" if count > 1 { " ×{count}" } }
                }
                if playing.hand.unknown > 0 { span { "{playing.hand.unknown} unknown" } }
            }
            if let Some(d) = playing.decision {
                DecisionControls { key: "{seat}-{d.id}", decision: d, busy,
                    on_answer: move |answer| act.call((seat, ModelAction::Answer { answer })) }
            } else { p { role: "status", "Waiting for this seat’s next decision." } }
        }
        footer {
            if holding {
                span { if own { "Let {model} play your seat." } else { "Resume automatic play." } }
                button { class: "copper", "data-action": "hand-over", disabled: busy,
                    onclick: move |_| act.call((seat, ModelAction::Release)),
                    if own { "Hand over" } else { "Return to model" } }
            } else if !current.controlled && (own || admin) {
                span { if own { "Play your seat yourself again." } else { "Play this seat yourself; its model reads along." } }
                button { class: "copper", "data-action": "take-seat", disabled: busy,
                    onclick: move |_| act.call((seat, ModelAction::Take)),
                    if own { "Take back" } else { "Take over seat" } }
            }
            if own {
                button { class: "toggle", "data-action": "detach-model", disabled: busy, title: "Send {model} away: your seat is yours alone again",
                    onclick: move |_| act.call((seat, ModelAction::Detach)), "Send away" }
            }
        }
    }
}
