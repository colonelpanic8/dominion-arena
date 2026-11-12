//! A seat played elsewhere, as handed out: its name, the line to give an agent, and
//! buttons copying that line, the seat link or the token.

use crate::platform;
use dioxus::prelude::*;
use dominion_protocol::api::SeatToken;

/// What to tell an agent so it can take `seat` of `game`.
pub fn agent_line(origin: &str, game: &str, seat: &SeatToken) -> String {
    format!(
        "Connect to the dominion MCP server at {origin}/mcp and call join_game with {{\"game_id\": \"{game}\", \"token\": \"{}\"}}",
        seat.token
    )
}

/// The link that opens `seat` of `game` in another browser.
pub fn seat_link(origin: &str, game: &str, seat: &SeatToken) -> String {
    format!(
        "{origin}/game/{game}#token={}",
        platform::encode(&seat.token)
    )
}

#[component]
pub fn SeatLink(
    game: String,
    seat: SeatToken,
    /// Its token has been used; None when not known.
    joined: Option<bool>,
) -> Element {
    let mut copied = use_signal(|| None::<&'static str>);
    let origin = platform::origin();
    let line = agent_line(&origin, &game, &seat);
    let link = seat_link(&origin, &game, &seat);
    let token = seat.token.clone();
    let mut copy = move |text: &str| {
        copied.set(Some(if platform::copy(text) {
            "Copied."
        } else {
            "Couldn't copy: select the line and copy it."
        }));
    };
    let l2 = line.clone();
    rsx! {
        div { class: "lb-link", "data-link-seat": "{seat.seat}",
            div { class: "lb-link-name",
                "{seat.name}"
                match joined {
                    Some(true) => rsx! { span { class: "lb-link-state joined", "joined" } },
                    Some(false) => rsx! { span { class: "lb-link-state waiting", "hasn't joined yet" } },
                    None => rsx! {},
                }
            }
            div { class: "lb-link-line", "data-mcp": "1", "{line}" }
            div { class: "lb-link-buttons",
                button { class: "lb-oval", "data-copy": "agent", onclick: move |_| copy(&l2), "Copy for Agent" }
                button { class: "lb-oval", "data-copy": "link", onclick: move |_| copy(&link), "Copy Link" }
                button { class: "lb-oval", "data-copy": "token", onclick: move |_| copy(&token), "Copy Token" }
                if let Some(m) = copied() {
                    span { class: "lb-copied", role: "status", "{m}" }
                }
            }
        }
    }
}
