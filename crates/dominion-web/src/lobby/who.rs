//! Who played a seat, the same everywhere a player is shown: the mark for what played it
//! (a person, a bot, a token, an agent's maker's logo), their face (`Face`: a picture, else
//! that mark on a disc), and the words for an agent, its model and its effort.

use super::avatar::Avatar;
use dioxus::prelude::*;
use dominion_protocol::lobby::{PastPlayer, PlayedBy, SeatAgent, UserKind, UserRef};

/// A player's face, round and ringed: an account's picture (its initial without one),
/// else the mark of what played it on a disc; `None` is an open seat. An agent always
/// shows its maker's logo. `size` is a CSS length.
#[component]
pub fn Face(user: Option<UserRef>, played_by: Option<PlayedBy>, size: String) -> Element {
    let agent = matches!(played_by, Some(PlayedBy::Agent(_)));
    if let Some(user) = user.filter(|u| !agent && (u.avatar.is_some() || u.kind != UserKind::Bot)) {
        return rsx! { Avatar { user, size } };
    }
    let style = format!("--avatar-size: {size}");
    match played_by {
        Some(played_by) => rsx! {
            span { class: "lb-avatar lb-face-mark", style, "aria-hidden": "true",
                SeatIcon { played_by }
            }
        },
        None => rsx! { span { class: "lb-avatar lb-face-open", style, "aria-hidden": "true" } },
    }
}

/// What played a seat, in a few words under its name: "Claude Code · High", "rook's bot".
/// An agent whose seat isn't named after its model (an official account's) is named.
pub fn caption(p: &PastPlayer) -> String {
    let played = played_by(p);
    match &p.model {
        Some(model) => format!("{played} · with {model}"),
        None => played,
    }
}

fn played_by(p: &PastPlayer) -> String {
    match &p.played_by {
        PlayedBy::Person => "Registered".into(),
        PlayedBy::Guest => "Guest".into(),
        PlayedBy::Token => "Seat token".into(),
        PlayedBy::Bot { owner: Some(owner) } => format!("{owner}'s bot"),
        PlayedBy::Bot { owner: None } => "Bot".into(),
        PlayedBy::Official => "dominion.games".into(),
        PlayedBy::Agent(a) => {
            let product = (a.name.clone())
                .or(a.model.clone())
                .unwrap_or(a.agent.clone());
            match &a.effort {
                Some(effort) => format!("{product} · {}", effort_title(effort)),
                None => product,
            }
        }
    }
}

/// Who makes an agent's model, by its id, else what runs it, else its name: their logo
/// goes before its seat's name. Claude Code and the Claude API share Claude's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Maker {
    Claude,
    OpenAi,
    Gemini,
}

pub fn maker(a: &SeatAgent) -> Option<Maker> {
    let by_word = |text: &str| {
        let text = text.to_lowercase();
        let has = |words: &[&str]| words.iter().any(|w| text.contains(w));
        if has(&["claude", "opus", "sonnet", "haiku", "fable"]) {
            Some(Maker::Claude)
        } else if has(&["gpt", "codex", "openai"]) {
            Some(Maker::OpenAi)
        } else if has(&["gemini", "antigravity"]) {
            Some(Maker::Gemini)
        } else {
            None
        }
    };
    (a.model.as_deref().and_then(by_word))
        .or(a.program.as_deref().and_then(by_word))
        .or(a.name.as_deref().and_then(by_word))
        .or(by_word(&a.agent))
}

impl Maker {
    /// Who makes the model.
    pub fn company(self) -> &'static str {
        match self {
            Maker::Claude => "Anthropic",
            Maker::OpenAi => "OpenAI",
            Maker::Gemini => "Google",
        }
    }
}

/// An agent's model by its name, version and all: "Claude Opus 5.5", "GPT-6.2-Sol".
pub fn model_title(a: &SeatAgent) -> String {
    let name = (a.name.clone())
        .or(a.model.clone())
        .unwrap_or(a.agent.clone());
    match maker(a) {
        Some(Maker::Claude) if !name.to_lowercase().starts_with("claude") => {
            format!("Claude {name}")
        }
        _ => name,
    }
}

/// A reasoning effort as the agent picker names it: "High", "XHigh".
pub fn effort_title(effort: &str) -> String {
    match effort.to_lowercase().as_str() {
        "xhigh" => "XHigh".into(),
        _ => {
            let mut chars = effort.chars();
            (chars
                .next()
                .map(|c| c.to_uppercase().chain(chars).collect()))
            .unwrap_or_default()
        }
    }
}

/// The class of the icon before a seat's name.
fn icon_class(by: &PlayedBy) -> &'static str {
    match by {
        PlayedBy::Person => "lb-seat-mark person",
        PlayedBy::Guest => "lb-seat-mark guest",
        PlayedBy::Token => "lb-seat-mark token",
        PlayedBy::Bot { .. } => "lb-seat-mark bot",
        PlayedBy::Agent(a) => match maker(a) {
            Some(Maker::Claude) => "lb-seat-mark logo claude",
            Some(Maker::OpenAi) => "lb-seat-mark logo openai",
            Some(Maker::Gemini) => "lb-seat-mark logo gemini",
            None => "lb-seat-mark agent",
        },
        PlayedBy::Official => "lb-seat-mark official",
    }
}

/// A person, a machine, a token or an agent's maker's logo, drawn small before a seat's
/// name: the drawings fill the same box, 2.5 to 13.5 across and 2 to 14.5 down of 16, and
/// a logo the same share of its own.
#[component]
pub fn SeatIcon(played_by: PlayedBy) -> Element {
    let class = icon_class(&played_by);
    let logo = match &played_by {
        PlayedBy::Agent(a) => maker(a).map(|m| match m {
            Maker::Claude => CLAUDE,
            Maker::OpenAi => OPENAI,
            Maker::Gemini => GEMINI,
        }),
        _ => None,
    };
    let machine = matches!(played_by, PlayedBy::Bot { .. } | PlayedBy::Agent(_));
    let token = played_by == PlayedBy::Token;
    if let Some(d) = logo {
        return rsx! {
            svg { class, view_box: "-3 -3 30 30", "aria-hidden": "true",
                path { d }
            }
        };
    }
    rsx! {
        svg { class, view_box: "0 0 16 16", "aria-hidden": "true",
            if machine {
                path { d: "M8 2v1.5M4 3.5h8a1.5 1.5 0 0 1 1.5 1.5v5.5a1.5 1.5 0 0 1-1.5 1.5H4a1.5 1.5 0 0 1-1.5-1.5V5A1.5 1.5 0 0 1 4 3.5zM6 7v1M10 7v1M5.5 14.5h5" }
            } else if token {
                path { d: "M2.5 3.5h11v9h-11zM5.5 8h5" }
            } else {
                path { d: "M8 2a3 3 0 1 1 0 6a3 3 0 1 1 0-6zM2.5 14.5c0-3.3 2.5-5.2 5.5-5.2s5.5 1.9 5.5 5.2" }
            }
        }
    }
}

// The makers' marks, from Simple Icons (CC0), in a 24 by 24 box.
const CLAUDE: &str = "m4.7144 15.9555 4.7174-2.6471.079-.2307-.079-.1275h-.2307l-.7893-.0486-2.6956-.0729-2.3375-.0971-2.2646-.1214-.5707-.1215-.5343-.7042.0546-.3522.4797-.3218.686.0608 1.5179.1032 2.2767.1578 1.6514.0972 2.4468.255h.3886l.0546-.1579-.1336-.0971-.1032-.0972L6.973 9.8356l-2.55-1.6879-1.3356-.9714-.7225-.4918-.3643-.4614-.1578-1.0078.6557-.7225.8803.0607.2246.0607.8925.686 1.9064 1.4754 2.4893 1.8336.3643.3035.1457-.1032.0182-.0728-.164-.2733-1.3539-2.4467-1.445-2.4893-.6435-1.032-.17-.6194c-.0607-.255-.1032-.4674-.1032-.7285L6.287.1335 6.6997 0l.9957.1336.419.3642.6192 1.4147 1.0018 2.2282 1.5543 3.0296.4553.8985.2429.8318.091.255h.1579v-.1457l.1275-1.706.2368-2.0947.2307-2.6957.0789-.7589.3764-.9107.7468-.4918.5828.2793.4797.686-.0668.4433-.2853 1.8517-.5586 2.9021-.3643 1.9429h.2125l.2429-.2429.9835-1.3053 1.6514-2.0643.7286-.8196.85-.9046.5464-.4311h1.0321l.759 1.1293-.34 1.1657-1.0625 1.3478-.8804 1.1414-1.2628 1.7-.7893 1.36.0729.1093.1882-.0183 2.8535-.607 1.5421-.2794 1.8396-.3157.8318.3886.091.3946-.3278.8075-1.967.4857-2.3072.4614-3.4364.8136-.0425.0304.0486.0607 1.5482.1457.6618.0364h1.621l3.0175.2247.7892.522.4736.6376-.079.4857-1.2142.6193-1.6393-.3886-3.825-.9107-1.3113-.3279h-.1822v.1093l1.0929 1.0686 2.0035 1.8092 2.5075 2.3314.1275.5768-.3218.4554-.34-.0486-2.2039-1.6575-.85-.7468-1.9246-1.621h-.1275v.17l.4432.6496 2.3436 3.5214.1214 1.0807-.17.3521-.6071.2125-.6679-.1214-1.3721-1.9246L14.38 17.959l-1.1414-1.9428-.1397.079-.674 7.2552-.3156.3703-.7286.2793-.6071-.4614-.3218-.7468.3218-1.4753.3886-1.9246.3157-1.53.2853-1.9004.17-.6314-.0121-.0425-.1397.0182-1.4328 1.9672-2.1796 2.9446-1.7243 1.8456-.4128.164-.7164-.3704.0667-.6618.4008-.5889 2.386-3.0357 1.4389-1.882.929-1.0868-.0062-.1579h-.0546l-6.3385 4.1164-1.1293.1457-.4857-.4554.0608-.7467.2307-.2429 1.9064-1.3114Z";
const OPENAI: &str = "M22.2819 9.8211a5.9847 5.9847 0 0 0-.5157-4.9108 6.0462 6.0462 0 0 0-6.5098-2.9A6.0651 6.0651 0 0 0 4.9807 4.1818a5.9847 5.9847 0 0 0-3.9977 2.9 6.0462 6.0462 0 0 0 .7427 7.0966 5.98 5.98 0 0 0 .511 4.9107 6.051 6.051 0 0 0 6.5146 2.9001A5.9847 5.9847 0 0 0 13.2599 24a6.0557 6.0557 0 0 0 5.7718-4.2058 5.9894 5.9894 0 0 0 3.9977-2.9001 6.0557 6.0557 0 0 0-.7475-7.0729zm-9.022 12.6081a4.4755 4.4755 0 0 1-2.8764-1.0408l.1419-.0804 4.7783-2.7582a.7948.7948 0 0 0 .3927-.6813v-6.7369l2.02 1.1686a.071.071 0 0 1 .038.052v5.5826a4.504 4.504 0 0 1-4.4945 4.4944zm-9.6607-4.1254a4.4708 4.4708 0 0 1-.5346-3.0137l.142.0852 4.783 2.7582a.7712.7712 0 0 0 .7806 0l5.8428-3.3685v2.3324a.0804.0804 0 0 1-.0332.0615L9.74 19.9502a4.4992 4.4992 0 0 1-6.1408-1.6464zM2.3408 7.8956a4.485 4.485 0 0 1 2.3655-1.9728V11.6a.7664.7664 0 0 0 .3879.6765l5.8144 3.3543-2.0201 1.1685a.0757.0757 0 0 1-.071 0l-4.8303-2.7865A4.504 4.504 0 0 1 2.3408 7.872zm16.5963 3.8558L13.1038 8.364 15.1192 7.2a.0757.0757 0 0 1 .071 0l4.8303 2.7913a4.4944 4.4944 0 0 1-.6765 8.1042v-5.6772a.79.79 0 0 0-.407-.667zm2.0107-3.0231l-.142-.0852-4.7735-2.7818a.7759.7759 0 0 0-.7854 0L9.409 9.2297V6.8974a.0662.0662 0 0 1 .0284-.0615l4.8303-2.7866a4.4992 4.4992 0 0 1 6.6802 4.66zM8.3065 12.863l-2.02-1.1638a.0804.0804 0 0 1-.038-.0567V6.0742a4.4992 4.4992 0 0 1 7.3757-3.4537l-.142.0805L8.704 5.459a.7948.7948 0 0 0-.3927.6813zm1.0976-2.3654l2.602-1.4998 2.6069 1.4998v2.9994l-2.5974 1.4997-2.6067-1.4997Z";
const GEMINI: &str = "M11.04 19.32Q12 21.51 12 24q0-2.49.93-4.68.96-2.19 2.58-3.81t3.81-2.55Q21.51 12 24 12q-2.49 0-4.68-.93a12.3 12.3 0 0 1-3.81-2.58 12.3 12.3 0 0 1-2.58-3.81Q12 2.49 12 0q0 2.49-.96 4.68-.93 2.19-2.55 3.81a12.3 12.3 0 0 1-3.81 2.58Q2.49 12 0 12q2.49 0 4.68.96 2.19.93 3.81 2.55t2.55 3.81";

/// Who makes an agent's model and what runs it, where that isn't its maker: "Anthropic ·
/// Claude Code", "OpenAI".
pub fn makers_line(a: &SeatAgent) -> String {
    let mut parts: Vec<String> = maker(a)
        .map(|m| m.company().to_string())
        .into_iter()
        .collect();
    if let Some(program) = a.program.as_ref().filter(|p| !parts.contains(p)) {
        parts.push(program.clone());
    }
    parts.join(" · ")
}

/// One of the server's agents in a line: "Claude Opus 5.5 · High effort · Anthropic ·
/// Claude Code".
pub fn agent_title(a: &SeatAgent) -> String {
    let mut parts = vec![model_title(a)];
    parts.extend(
        a.effort
            .as_deref()
            .map(|e| format!("{} effort", effort_title(e))),
    );
    parts.push(makers_line(a));
    parts.retain(|p| !p.is_empty());
    parts.join(" · ")
}
