//! A seat a person and a model share (README "Sharing a seat with a model"). `Models`
//! keeps the game's such seats as the viewer may see them and tells the page's state who
//! answers the seat shown. The viewer's own seat gets `ModelPane`, beside the chat: who
//! holds it, what its model wrote, asking it, handing the seat over. Models' own seats
//! keep `ModelControls`: live instructions and the admin's manual decisions.
use super::plain::DecisionControls;
use super::state::Hold;
use super::{GameState, Offers, use_game};
use crate::account::use_account;
use crate::lobby::agent_picker::AgentPicker;
use crate::lobby::use_lobby;
use crate::{api, platform};
use dioxus::prelude::*;
use dominion_protocol::api::{AdviseMode, ModelAction, ModelNote, ModelRequest, ModelSeat};
use dominion_protocol::lobby::AgentSelection;

const CSS: &str = include_str!("../../assets/model.css");

/// A game's seats with a model at them, as the viewer may see them, and what the viewer
/// does with them.
#[derive(Clone, Copy, PartialEq)]
pub struct Models {
    pub seats: Signal<Vec<ModelSeat>>,
    pub busy: Signal<bool>,
    pub error: Signal<Option<String>>,
    pub act: Callback<(u8, ModelAction)>,
    /// The viewer's own seat with no model at it, in a game one may be attached in.
    pub attachable: Memo<Option<u8>>,
    /// The decision the viewer last answered for a seat they hold, which a reading of
    /// the seat from before the answer still carries.
    pub answered: Signal<Option<String>>,
}

impl Models {
    /// The seat the viewer shares with a model: their own with one attached, or a
    /// model's they hold where the page shows that seat's own view.
    pub fn shared(&self, shown: Option<u8>) -> Option<ModelSeat> {
        let seats = self.seats.read();
        let found = (seats.iter()).find(|s| Some(s.seat) == shown && (s.own || s.holding));
        found.cloned()
    }
}

/// Who answers the seat shown, given the seats with models and the decision just answered.
fn hold_of(seats: &[ModelSeat], shown: Option<u8>, answered: Option<&str>) -> Hold {
    match seats.iter().find(|s| Some(s.seat) == shown) {
        Some(s) if s.own && !s.controlled => Hold::Model,
        Some(ModelSeat {
            holding: true,
            playing: Some(playing),
            ..
        }) => Hold::Taken {
            decision: (playing.decision.clone())
                .filter(|d| Some(d.id.as_str()) != answered)
                .map(Box::new),
            version: playing.version,
        },
        _ => Hold::Served,
    }
}

/// Keeps `game`'s seats with models current for whoever is signed in, and `state` told
/// who answers the seat it shows.
pub fn use_models(
    game: Signal<String>,
    mut state: Signal<GameState>,
    offers: Memo<Offers>,
) -> Models {
    let account = use_account();
    let mut seats = use_signal(Vec::<ModelSeat>::new);
    let mut busy = use_signal(|| false);
    let mut revision = use_signal(|| 0_u64);
    let mut error = use_signal(|| None::<String>);
    let answered = use_signal(|| None::<String>);
    let endpoint = use_memo(move || {
        let signed_in = account.me.read().is_some();
        offers.read().model_controls.clone().filter(|_| signed_in)
    });
    let refresh = use_callback(move |()| {
        spawn(async move {
            let before = *revision.peek();
            let Some(url) = endpoint.peek().clone().filter(|_| !*busy.peek()) else {
                return;
            };
            let result = api::model_seats(&url).await;
            if before != *revision.peek() || *busy.peek() {
                return;
            }
            match result {
                Ok(next) if *seats.peek() != next => seats.set(next),
                Err(e) if matches!(e.status, 401 | 403 | 404) && !seats.peek().is_empty() => {
                    seats.set(Vec::new());
                }
                _ => {}
            }
        });
    });
    use_future(move || async move {
        loop {
            refresh.call(());
            platform::sleep(if seats.peek().is_empty() { 15000 } else { 2000 }).await;
        }
    });
    // A note, a request and a change of holder all bump the game's version.
    let version = use_memo(move || (state.read().version, endpoint()));
    use_effect(move || {
        version();
        refresh.call(());
    });
    let shown = use_memo(move || state.read().view.as_ref().and_then(|v| v.seat));
    use_effect(move || {
        let hold = hold_of(&seats.read(), shown(), answered.read().as_deref());
        // The view's arrival settles it under the hold kept.
        let _ = version();
        if state.peek().hold != hold {
            state.write().set_hold(hold);
        }
    });
    let act = use_callback(move |(seat, action): (u8, ModelAction)| {
        let Some(url) = endpoint.peek().clone().filter(|_| !*busy.peek()) else {
            return;
        };
        busy.set(true);
        *revision.write() += 1;
        error.set(None);
        let req = ModelRequest {
            game: game.peek().clone(),
            seat,
            action,
        };
        spawn(async move {
            match api::model_action(&url, &req).await {
                Ok(next) => seats.set(next),
                Err(e) => error.set(Some(e.message)),
            }
            busy.set(false);
        });
    });
    let attachable = use_memo(move || {
        let me = account.me.read();
        let name = &me.as_ref()?.user.name;
        let arena = endpoint().is_some_and(|url| url.starts_with("/api/games/"));
        let st = state.read();
        let view = st.view.as_ref().filter(|v| v.game.running && arena)?;
        let seat = view.seat.filter(|_| !view.spectating && !st.meta.rated)?;
        let user = st.meta.users.get(usize::from(seat))?.as_ref()?;
        let free = seats.read().iter().all(|s| s.seat != seat);
        (free && user.name == *name).then_some(seat)
    });
    Models {
        seats,
        busy,
        error,
        act,
        attachable,
        answered,
    }
}

/// How a note stands to the game now: about the decision open, written this turn, or
/// older.
/// How current a note is: about the open decision, of this turn, or passed. An
/// analyst's plan (`latest_plan`: the last of them) stands until its next.
fn freshness(
    note: &ModelNote,
    latest_plan: bool,
    decision: Option<&str>,
    turn: Option<u32>,
) -> &'static str {
    if note.decision.is_some() && note.decision.as_deref() == decision {
        "now"
    } else if latest_plan || (note.turn.is_some() && note.turn == turn) {
        "turn"
    } else {
        "old"
    }
}

/// What a model wrote, the last note last and in view.
#[component]
fn Notes(notes: Vec<ModelNote>, empty: String) -> Element {
    let ctx = use_game();
    let (decision, turn) = {
        let state = ctx.state.read();
        let view = state.view.as_ref();
        (
            view.and_then(|v| v.decision.as_ref().map(|d| d.id.clone())),
            view.and_then(|v| v.turn.number),
        )
    };
    let count = notes.len();
    let plan = notes.iter().rposition(|n| n.from.is_some());
    use_effect(use_reactive!(|count| {
        let _ = count;
        document::eval(
            "requestAnimationFrame(() => { const e = document.getElementById('model-notes'); const last = e && e.lastElementChild; if (last) e.scrollTop = last.offsetTop - e.offsetTop; });",
        );
    }));
    rsx! {
        ol { id: "model-notes", class: "model-notes", "data-notes": "{count}",
            if notes.is_empty() {
                li { class: "model-note-none", "{empty}" }
            }
            for (i, note) in notes.iter().enumerate() {
                li { key: "{i}", class: "model-note", "data-from": note.from.clone(),
                    "data-fresh": freshness(note, plan == Some(i), decision.as_deref(), turn),
                    span { class: "model-note-when",
                        if let Some(from) = &note.from { "{from} · " }
                        if let Some(turn) = note.turn { "Turn {turn} " }
                    }
                    "{note.text}"
                }
            }
        }
    }
}

const MODES: [(AdviseMode, &str, &str); 3] = [
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
        "It writes at every decision of yours, often after you have moved on.",
    ),
];

/// The viewer's side of a seat they share with a model: what it wrote, a note asked
/// for, a question; or the agents to attach one of.
#[component]
fn ModelPane() -> Element {
    let ctx = use_game();
    let models = ctx.models;
    let lobby = use_lobby();
    let mut draft = use_signal(String::new);
    let mut picked = use_signal(|| None::<AgentSelection>);
    let shown = (ctx.state.read().view.as_ref()).and_then(|v| v.seat);
    let busy = (models.busy)();
    let act = models.act;
    let error = (models.error)();
    let Some(current) = models.shared(shown) else {
        let Some(seat) = (models.attachable)() else {
            return rsx! {};
        };
        let offered = lobby.lobby.read().as_ref().map(|l| l.house.models.clone());
        let offered = offered.unwrap_or_default();
        return rsx! {
            div { class: "model-pane", "data-model": "none",
                p { class: "model-about",
                    "Attach one of the server's agents to your seat. You keep playing; it sees what you see and writes you a note when you ask. You can hand it the seat and take it back."
                }
                if offered.is_empty() {
                    p { class: "model-about", "The server's agents aren't available now." }
                } else {
                    div { class: "model-row model-pick",
                        AgentPicker {
                            models: offered,
                            value: picked(),
                            none: "Pick a model",
                            action: "pick-model",
                            disabled: busy,
                            onpick: move |choice| picked.set(choice),
                        }
                        button { class: "primary", "data-action": "attach-model",
                            disabled: busy || picked.read().is_none(),
                            title: "It counts as one of your agent seats for today",
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
                if let Some(message) = error { p { class: "game-chat-error", role: "alert", "{message}" } }
            }
        };
    };
    let seat = current.seat;
    let model = current
        .agent
        .clone()
        .unwrap_or_else(|| current.name.clone());
    let mut ask = move || {
        let text = draft.peek().trim().to_string();
        if !text.is_empty() && !*models.busy.peek() {
            act.call((seat, ModelAction::Prompt { text }));
            draft.set(String::new());
        }
    };
    rsx! {
        div { class: "model-pane", "data-model": if current.holding { "advising" } else { "playing" },
            if current.holding {
                Notes { notes: current.notes.clone(), empty: "Nothing written yet. Ask for a note on your next decision, or ask a question." }
                div { class: "model-row",
                    button { class: "primary", "data-action": "request-advice", disabled: busy || current.requested,
                        title: "One note, on the decision you are at or reach next",
                        onclick: move |_| act.call((seat, ModelAction::Request)),
                        if current.requested { "Writing…" } else { "Advise my next decision" }
                    }
                    select { class: "model-mode", "aria-label": "When it writes unasked", disabled: busy,
                        title: MODES.iter().find(|m| m.0 == current.advise).map_or("", |m| m.2),
                        onchange: move |e| {
                            if let Some(mode) = MODES.iter().find(|m| m.1 == e.value()) {
                                act.call((seat, ModelAction::Advise { mode: mode.0 }));
                            }
                        },
                        for (mode, name, _) in MODES {
                            option { value: name, selected: current.advise == mode, "{name}" }
                        }
                    }
                }
            } else {
                p { class: "model-about", "{model} is playing your seat until you take it back. What you tell it reaches it with its next decision." }
            }
            if let Some(message) = error { p { class: "game-chat-error", role: "alert", "{message}" } }
            input {
                class: "game-chat-input",
                placeholder: if current.holding { format!("Ask {model}…") } else { format!("Tell {model} what to consider…") },
                maxlength: "2000",
                value: "{draft}",
                oninput: move |e| draft.set(e.value()),
                onkeydown: move |e| {
                    if e.key() == Key::Enter {
                        ask();
                    }
                },
            }
        }
    }
}

/// The chat, and under further tabs the viewer's side of a seat they share with a model
/// (`ModelPane`), with who holds that seat changed from the tabs' row, and the agents'
/// transcript (`AgentsPane`). `heading`: the chat alone is headed, as a panel of the plain
/// page is.
#[component]
pub fn Aside(heading: bool) -> Element {
    let ctx = use_game();
    let models = ctx.models;
    let offers = ctx.offers;
    let chat = offers.read().chat;
    let transcript = offers.read().transcript.clone();
    let shown = (ctx.state.read().view.as_ref()).and_then(|v| v.seat);
    let shared = models.shared(shown);
    let has_model = shared.is_some() || (models.attachable)().is_some();
    let mut tab = use_signal(|| "chat");
    // The transcript is loaded once its tab is first opened, and followed from then on.
    let mut agents_opened = use_signal(|| false);
    let feed_game = use_memo(move || {
        agents_opened()
            .then(|| offers.read().transcript.clone())
            .flatten()
    });
    let feed = crate::transcript::use_feed(feed_game);
    let show = crate::transcript::use_show();
    // Notes written before the page had the seat are no news.
    let mut seen = use_signal(|| None::<usize>);
    let tabs: Vec<&str> = [
        chat.then_some("chat"),
        has_model.then_some("model"),
        transcript.is_some().then_some("agents"),
    ]
    .into_iter()
    .flatten()
    .collect();
    let current = tabs
        .iter()
        .copied()
        .find(|&t| t == tab())
        .or(tabs.first().copied());
    let on_model = current == Some("model");
    let on_agents = current == Some("agents");
    if on_agents && !agents_opened() {
        agents_opened.set(true);
    }
    let notes = shared.as_ref().map(|s| s.notes.len());
    if notes.is_some() && (on_model || seen().is_none()) && seen() != notes {
        seen.set(notes);
    }
    let unread = notes.unwrap_or(0).saturating_sub(seen().unwrap_or(0));
    if tabs.len() <= 1 && !on_model && !on_agents {
        return rsx! {
            if chat {
                if heading { h3 { "Chat" } }
                super::chat::Chat {}
            }
        };
    }
    let name = (shared.as_ref()).map(|s| s.agent.clone().unwrap_or_else(|| s.name.clone()));
    let (busy, act) = ((models.busy)(), models.act);
    let tab_title = match &name {
        Some(name) => format!("{name}, the model at your seat"),
        None => "Attach a model to your seat".to_string(),
    };
    rsx! {
        style { {CSS} }
        div { class: "aside-tabs",
            span { class: "aside-tablist", role: "tablist",
                if chat {
                    button { class: if current == Some("chat") { "toggle on" } else { "toggle" }, role: "tab", "data-tab": "chat",
                        "aria-selected": "{current == Some(\"chat\")}",
                        onclick: move |e| { e.stop_propagation(); tab.set("chat"); },
                        "Chat"
                    }
                }
                if has_model {
                    button { class: if on_model { "toggle on" } else { "toggle" }, role: "tab", "data-tab": "model",
                        "aria-selected": "{on_model}",
                        title: tab_title,
                        onclick: move |e| { e.stop_propagation(); tab.set("model"); },
                        "Model"
                        if unread > 0 { span { class: "model-unread", "data-unread": "{unread}", "{unread}" } }
                    }
                }
                if transcript.is_some() {
                    button { class: if on_agents { "toggle on" } else { "toggle" }, role: "tab", "data-tab": "agents",
                        "aria-selected": "{on_agents}",
                        title: "What the agents at this game were given and returned",
                        onclick: move |e| { e.stop_propagation(); tab.set("agents"); },
                        "Agents"
                    }
                }
            }
            if let (Some(s), Some(name), true) = (shared, name, on_model) {
                span { class: "aside-seat",
                    if s.holding {
                        button { class: "toggle", "data-action": "hand-over", disabled: busy,
                            title: "Let {name} play your seat until you take it back",
                            onclick: move |_| act.call((s.seat, ModelAction::Release)), "Hand over" }
                    } else if !s.controlled {
                        button { class: "toggle", "data-action": "take-seat", disabled: busy,
                            title: "Play your seat yourself again",
                            onclick: move |_| act.call((s.seat, ModelAction::Take)), "Take back" }
                    }
                    if s.own {
                        button { class: "toggle", "data-action": "detach-model", disabled: busy,
                            title: "Send {name} away: your seat is yours alone again",
                            onclick: move |_| act.call((s.seat, ModelAction::Detach)), "Send away" }
                    }
                }
            }
        }
        if on_model {
            ModelPane {}
        } else if let (true, Some(game)) = (on_agents, transcript) {
            crate::transcript::AgentsPane { feed, show, game }
        } else {
            super::chat::Chat {}
        }
    }
}

/// Says that the model attached to the seat shown is playing it, with the way back.
#[component]
pub fn HeldNotice(#[props(default)] style: String) -> Element {
    let ctx = use_game();
    let models = ctx.models;
    let shown = (ctx.state.read().view.as_ref()).and_then(|v| v.seat);
    let Some(seat) = models.shared(shown).filter(|s| s.own && !s.controlled) else {
        return rsx! {};
    };
    let model = seat.agent.clone().unwrap_or_else(|| "The model".into());
    let (number, act) = (seat.seat, models.act);
    rsx! {
        div { class: "waiting-notice", role: "status", style, "data-held": "model",
            p { class: "waiting-title", "{model} is playing your seat." }
            button { "data-action": "take-seat", disabled: (models.busy)(),
                onclick: move |_| act.call((number, ModelAction::Take)), "Take back" }
        }
    }
}

/// Live instructions for a model's own seat, and the manual decisions of whoever may
/// hold it (`take`: an admin, a dominion.games account's owner).
#[component]
pub fn ModelControls(take: bool) -> Element {
    let ctx = use_game();
    let models = ctx.models;
    let mut open = use_signal(|| false);
    let mut selected = use_signal(|| None::<u8>);
    let mut draft = use_signal(String::new);
    let mut notice = use_signal(|| None::<String>);
    let shown = (ctx.state.read().view.as_ref()).and_then(|v| v.seat);
    let busy = models.busy;
    let act = models.act;
    // The seat the viewer shares on the page itself has its pane beside the chat.
    let pane = models.shared(shown).map(|s| s.seat);
    let available: Vec<ModelSeat> = (models.seats.read().iter())
        .filter(|s| Some(s.seat) != pane)
        .cloned()
        .collect();
    let current = available
        .iter()
        .find(|s| Some(s.seat) == selected())
        .or_else(|| available.iter().find(|s| s.playing.is_some()))
        .or(available.first())
        .cloned();
    let Some(current) = current else {
        return rsx! {};
    };
    let seat = current.seat;
    let yours = current.playing.is_some();
    let controlled = current.controlled;
    let status = if yours {
        "You are playing"
    } else if controlled {
        "A person is playing"
    } else {
        "Model is playing"
    };
    let error = (models.error)();
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
                                draft.set(String::new()); notice.set(None);
                            },
                            for s in available { option { value: "{s.seat}", "{s.name}" } }
                        }
                    }
                }
                div { class: "model-status", span { class: "model-dot", "aria-hidden":"true" } "{status}" }
                if yours {
                    Notes { notes: current.notes.clone(), empty: "The model reads along and writes here when you ask." }
                    div { class: "model-send",
                        small { if current.requested { "It is writing about your next decision." } else { "One note, on the decision you are at or reach next." } }
                        button { class: "copper", "data-action": "request-advice", disabled: busy() || current.requested,
                            onclick: move |_| act.call((seat, ModelAction::Request)), "Advise my next decision" }
                    }
                }
                label { r#for: "model-prompt", if yours { "Ask the model" } else { "Prompt the model" } }
                textarea { id: "model-prompt", rows: 3, maxlength: 2000, value: "{draft}",
                    placeholder: "What should {current.name} consider?",
                    disabled: busy(),
                    oninput: move |e| { selected.set(Some(seat)); draft.set(e.value()); notice.set(None); },
                }
                div { class: "model-send",
                    small { if controlled { "It answers in a note for whoever plays the seat." } else { "Included in its next game observation." } }
                    button { class: "primary", disabled: busy() || draft.read().trim().is_empty(),
                        onclick: move |_| {
                            act.call((seat, ModelAction::Prompt { text: draft.peek().trim().into() }));
                            draft.set(String::new());
                            notice.set(Some("Prompt saved for the model’s next observation.".into()));
                        },
                        "Send prompt" }
                }
                if let Some(message) = notice().filter(|_| error.is_none()) { p { class:"model-notice", role:"status", "{message}" } }
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
                if take {
                    footer {
                        if yours {
                            span { "Resume automatic play." }
                            button { class: "copper", disabled: busy(), onclick: move |_| act.call((seat, ModelAction::Release)), "Return to model" }
                        } else if !controlled {
                            span { "Play this seat yourself; its model reads along." }
                            button { class: "copper", disabled: busy(), onclick: move |_| act.call((seat, ModelAction::Take)), "Take over seat" }
                        }
                    }
                }
                if let Some(message) = error { p { class:"error", role:"alert", "{message}" } }
            }
        }
    }
}
