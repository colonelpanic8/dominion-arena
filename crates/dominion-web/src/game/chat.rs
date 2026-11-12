//! The game's chat: what its players and those watching have said, and a line to say
//! something as the seat shown (or, watching, as the account signed in).

use super::{logfmt, use_game};
use dioxus::prelude::*;
use dominion_protocol::lobby::{ChatKind, ChatMessage};

/// Who a message is from, as its line shows it: a player by seat (for their colour), or
/// someone watching.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Speaker {
    Seat(u8),
    /// A player's name no seat has (a seat renamed, a game not loaded yet).
    Player,
    Watcher,
    System,
}

pub fn speaker(m: &ChatMessage, names: &[String]) -> Speaker {
    match m.kind {
        ChatKind::System => Speaker::System,
        ChatKind::Watcher => Speaker::Watcher,
        ChatKind::Player => match names.iter().position(|n| *n == m.from) {
            Some(seat) => Speaker::Seat(seat as u8),
            None => Speaker::Player,
        },
    }
}

#[component]
fn ChatLine(message: ChatMessage, names: Vec<String>) -> Element {
    let m = &message;
    match speaker(m, &names) {
        Speaker::System => rsx! {
            li { class: "chat-line chat-system", "data-seq": "{m.seq}", "{m.text}" }
        },
        Speaker::Watcher => rsx! {
            li { class: "chat-line chat-watcher", "data-seq": "{m.seq}",
                span { class: "chat-from", "{m.from} (watching): " }
                "{m.text}"
            }
        },
        who => {
            let colour = match who {
                Speaker::Seat(s) => super::table::seat_class(s),
                _ => String::new(),
            };
            rsx! {
                li { class: "chat-line chat-player", "data-seq": "{m.seq}",
                    span { class: "chat-from dt-pc {colour}", "{m.from}: " }
                    "{m.text}"
                }
            }
        }
    }
}

/// The chat's lines, kept scrolled to the last, over its input.
#[component]
pub fn Chat() -> Element {
    let ctx = use_game();
    let mut draft = use_signal(String::new);
    let mut error = use_signal(|| None::<String>);
    let mut sending = use_signal(|| false);
    let st = ctx.state.read();
    let messages = st.chat.clone();
    let names = (st.view.as_ref()).map(logfmt::names).unwrap_or_default();
    drop(st);
    let len = messages.len();
    use_effect(use_reactive!(|len| {
        let _ = len;
        document::eval(
            "requestAnimationFrame(() => { const e = document.getElementById('game-chat'); if (e) e.scrollTop = e.scrollHeight; });",
        );
    }));
    let mut send = move || {
        let text = draft.peek().trim().to_string();
        if text.is_empty() || *sending.peek() {
            return;
        }
        sending.set(true);
        spawn(async move {
            match ctx.say(&text).await {
                Ok(()) => {
                    error.set(None);
                    if draft.peek().trim() == text {
                        draft.set(String::new());
                    }
                }
                Err(e) => error.set(Some(e.message)),
            }
            sending.set(false);
        });
    };
    let hint = if (ctx.watching)() {
        "Say something (as someone watching)…"
    } else {
        "Say something…"
    };
    rsx! {
        div { class: "game-chat", "data-chat": "{len}",
            ol { id: "game-chat", class: "game-chat-lines",
                for m in messages.into_iter() {
                    ChatLine { key: "{m.seq}", message: m, names: names.clone() }
                }
            }
            if let Some(e) = error() {
                p { class: "game-chat-error", "data-chat-error": "1", "{e}" }
            }
            input {
                class: "game-chat-input",
                placeholder: hint,
                maxlength: "500",
                value: "{draft}",
                oninput: move |e| draft.set(e.value()),
                onkeydown: move |e| {
                    if e.key() == Key::Enter {
                        send();
                    }
                },
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn someone_watching_under_a_players_name_is_still_marked_as_watching() {
        let names = vec!["Ada".to_string(), "Bo".to_string()];
        let said = |from: &str, kind| ChatMessage {
            seq: 1,
            at: 0,
            from: from.into(),
            kind,
            text: "hi".into(),
        };
        assert_eq!(
            speaker(&said("Bo", ChatKind::Player), &names),
            Speaker::Seat(1)
        );
        assert_eq!(
            speaker(&said("Bo", ChatKind::Watcher), &names),
            Speaker::Watcher
        );
        assert_eq!(
            speaker(&said("Cy", ChatKind::Player), &names),
            Speaker::Player
        );
        assert_eq!(
            speaker(&said("", ChatKind::System), &names),
            Speaker::System
        );
    }
}
