//! A chat channel: the lobby's, or a table's. What was said, kept current when the
//! lobby feed says something was, and a line to say more.

use super::{Fieldset, flash, signed_in, use_lobby, use_toast};
use crate::account::use_account;
use crate::api;
use dioxus::prelude::*;
use dominion_protocol::lobby::{ChatKind, ChatMessage};

/// `channel` is `lobby` or `table-<id>`. Give it a key: another channel is another chat.
#[component]
pub fn Chat(channel: String, title: String, #[props(default)] class: String) -> Element {
    let ctx = use_lobby();
    let account = use_account();
    let toast = use_toast();
    let nav = navigator();
    let mut messages = use_signal(Vec::<ChatMessage>::new);
    let mut draft = use_signal(String::new);
    let name = use_signal(|| channel.clone());
    use_effect(move || {
        let _ = ctx.chat.read();
        spawn(async move {
            let after = messages.peek().last().map_or(0, |m| m.seq);
            let Ok(log) = api::chat(&name.peek(), after, None).await else {
                return;
            };
            let last = messages.peek().last().map_or(0, |m| m.seq);
            let new: Vec<ChatMessage> = (log.messages.into_iter())
                .filter(|m| m.seq > last)
                .collect();
            if !new.is_empty() {
                messages.write().extend(new);
            }
        });
    });
    let id = format!("lb-chat-{channel}");
    let count = messages.read().len();
    use_effect(use_reactive!(|count, id| {
        let _ = count;
        document::eval(&format!(
            "requestAnimationFrame(() => {{ const e = document.getElementById('{id}'); if (e) e.scrollTop = e.scrollHeight; }});"
        ));
    }));
    let mut say = move || {
        let text = draft().trim().to_string();
        if text.is_empty() {
            return;
        }
        draft.set(String::new());
        spawn(async move {
            if !signed_in(account, toast, nav).await {
                return;
            }
            if let Err(e) = api::say(&name.peek(), &text, None).await {
                draft.set(text);
                flash(toast, e.message);
            }
        });
    };
    rsx! {
        Fieldset { title, class: "lb-chat {class}",
            ol { class: "lb-chat-log", id, "data-chat": "{channel}",
                for m in messages.read().iter() {
                    li {
                        key: "{m.seq}",
                        class: if m.kind == ChatKind::System { "lb-chat-line system" } else { "lb-chat-line" },
                        if m.kind != ChatKind::System {
                            span { class: "lb-chat-from", "{m.from}: " }
                        }
                        span { class: "lb-chat-text", "{m.text}" }
                    }
                }
            }
            input {
                class: "lb-input lb-chat-input",
                "data-chat-input": "{channel}",
                maxlength: "500",
                placeholder: "Say something",
                value: "{draft}",
                oninput: move |e| draft.set(e.value()),
                onkeydown: move |e| {
                    if e.key() == Key::Enter {
                        say();
                    }
                },
            }
        }
    }
}
