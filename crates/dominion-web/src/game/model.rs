//! Live instructions and the admin's manual decisions for a model seat.
use super::plain::DecisionControls;
use crate::{api, platform};
use dioxus::prelude::*;
use dominion_protocol::api::{ModelAction, ModelRequest, ModelSeat};

const CSS: &str = include_str!("../../assets/model.css");

#[component]
pub fn ModelControls(url: String, game: String, admin: bool) -> Element {
    let endpoint = use_signal(|| url);
    let mut seats = use_signal(Vec::<ModelSeat>::new);
    let mut open = use_signal(|| false);
    let mut busy = use_signal(|| false);
    let mut revision = use_signal(|| 0_u64);
    let mut selected = use_signal(|| None::<u8>);
    let mut draft = use_signal(String::new);
    let mut error = use_signal(|| None::<String>);
    let mut notice = use_signal(|| None::<String>);

    use_future(move || async move {
        loop {
            let before = *revision.peek();
            if !*busy.peek() {
                let url = endpoint.peek().clone();
                let result = api::model_seats(&url).await;
                if before == *revision.peek() && !*busy.peek() {
                    match result {
                        Ok(next) => {
                            if *seats.peek() != next {
                                seats.set(next);
                            }
                        }
                        Err(e) if matches!(e.status, 401 | 403 | 404) => {
                            seats.set(Vec::new());
                            open.set(false);
                            break;
                        }
                        Err(e) if *open.peek() => error.set(Some(e.message)),
                        _ => {}
                    }
                }
            }
            platform::sleep(if *open.peek() { 1000 } else { 5000 }).await;
        }
    });
    let available = seats();
    let current = available
        .iter()
        .find(|s| Some(s.seat) == selected())
        .or_else(|| available.iter().find(|s| s.playing.is_some()))
        .or(available.first())
        .cloned();
    let act = use_callback(move |(seat, action): (u8, ModelAction)| {
        if *busy.peek() {
            return;
        }
        busy.set(true);
        *revision.write() += 1;
        error.set(None);
        notice.set(None);
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
                        notice.set(Some(
                            "Prompt saved for the model’s next observation.".into(),
                        ));
                    }
                }
                Err(e) => error.set(Some(e.message)),
            }
            busy.set(false);
        });
    });
    let Some(current) = current else {
        return rsx! {};
    };
    let seat = current.seat;
    let yours = current.playing.is_some();
    let controlled = current.controlled;
    let status = if yours {
        "You are playing"
    } else if controlled {
        "An admin is playing"
    } else {
        "Model is playing"
    };
    rsx! {
        style { {CSS} }
        button { class: if yours { "copper" } else { "toggle" }, "aria-haspopup": "dialog",
            onclick: move |_| open.set(true),
            if yours { "Play model seat" } else { "Model controls" }
        }
        if open() {
            dialog {
                class: "model-controls", "aria-labelledby": "model-controls-title",
                onmounted: move |_| { document::eval("document.querySelector('dialog.model-controls')?.showModal()"); },
                onkeydown: move |e| { if e.key() == Key::Escape { e.prevent_default(); open.set(false); } },
                header {
                    div {
                        h2 { id: "model-controls-title", "Model controls" }
                        p { class: "model-seat-name", "{current.name}" }
                    }
                    button { class: "model-close", "aria-label": "Close model controls",
                        onclick: move |_| open.set(false), "×" }
                }
                if available.len() > 1 {
                    label { "Model seat"
                        select {
                            value: "{seat}", disabled: busy(),
                            onchange: move |e| {
                                selected.set(e.value().parse().ok());
                                draft.set(String::new()); notice.set(None); error.set(None);
                            },
                            for s in available { option { value: "{s.seat}", "{s.name}" } }
                        }
                    }
                }
                div { class: "model-status", span { class: "model-dot", "aria-hidden":"true" } "{status}" }
                if !controlled {
                    label { r#for: "model-prompt", "Prompt the model" }
                    textarea { id: "model-prompt", rows: 3, maxlength: 2000, value: "{draft}",
                        placeholder: "What should {current.name} consider?",
                        disabled: busy(),
                        oninput: move |e| { selected.set(Some(seat)); draft.set(e.value()); notice.set(None); },
                    }
                    div { class: "model-send",
                        small { "Included in its next game observation." }
                        button { class: "primary", disabled: busy() || draft.read().trim().is_empty(),
                            onclick: move |_| act.call((seat, ModelAction::Prompt { text: draft.peek().trim().into() })), "Send prompt" }
                    }
                }
                if let Some(message) = notice() { p { class:"model-notice", role:"status", "{message}" } }
                if !current.prompts.is_empty() {
                    details { class: "model-history",
                        summary { "Sent prompts ({current.prompts.len()})" }
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
                        DecisionControls { key: "{seat}-{d.id}", decision: d, busy: busy(),
                            on_answer: move |answer| act.call((seat, ModelAction::Answer { answer })) }
                    } else { p { role: "status", "Waiting for this seat’s next decision." } }
                }
                if admin {
                    footer {
                        if yours {
                            span { "Resume automatic play." }
                            button { class: "copper", disabled: busy(), onclick: move |_| act.call((seat, ModelAction::Release)), "Return to model" }
                        } else if !controlled {
                            span { "Pause the model and play this seat." }
                            button { class: "copper", disabled: busy(), onclick: move |_| act.call((seat, ModelAction::Take)), "Take over seat" }
                        }
                    }
                }
                if let Some(message) = error() { p { class:"error", role:"alert", "{message}" } }
            }
        }
    }
}
