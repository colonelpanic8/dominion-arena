//! The Leaderboard tab (the rated players, best first) and a player's page: their
//! rating and tallies over their games.

use super::history::{GameBrowser, agent_line, seat_of};
use super::{LobbyFrame, Tab, UserName};
use crate::api;
use crate::app::Route;
use dioxus::prelude::*;
use dominion_protocol::lobby::{Beyond, Leaderboard, OfficialSkill, PastGame, Profile, UserKind};

const OFFICIAL_TITLE: &str =
    "Skill on dominion.games' scale (mu, not level), read off agents rated on both servers";

#[component]
pub fn LeaderboardTab() -> Element {
    let board = use_resource(|| async { api::leaderboard().await });
    rsx! {
        LobbyFrame { tab: Tab::Leaderboard,
            div { class: "lb-board-page",
                match &*board.read() {
                    None => rsx! {},
                    Some(Err(e)) => rsx! { p { class: "lb-error", "{e.message}" } },
                    Some(Ok(Leaderboard { players })) => {
                        let official = players.iter().any(|p| p.rating.official.is_some());
                        let line = if official { "lb-board-line lb-board-official" } else { "lb-board-line" };
                        rsx! {
                        div { class: "{line} lb-board-head",
                            span { class: "lb-board-rank", "#" }
                            span { class: "lb-board-name", "Player" }
                            span { class: "lb-board-num", "Rating" }
                            if official {
                                span { class: "lb-board-num", title: OFFICIAL_TITLE, "dominion.games" }
                            }
                            span { class: "lb-board-num", "Rated games" }
                            span { class: "lb-board-num", "Games" }
                            span { class: "lb-board-num", "Won" }
                        }
                        div { class: "lb-board-list",
                            if players.is_empty() {
                                p { class: "lb-note", "Nobody has played a rated game yet. A game is rated when its table or search says so and every seat is a different registered player or bot, or when it is made with a different one of the server's agents in every seat." }
                            }
                            for p in players.iter() {
                                div { key: "{p.user.name}", class: line, "data-leader": "{p.user.name}",
                                    span { class: "lb-board-rank", "{p.rank}" }
                                    span { class: "lb-board-name", UserName { user: without_rating(&p.user), link: true } }
                                    span { class: "lb-board-num", title: "± {p.rating.deviation.round()}", "{p.rating.rating.round()}" }
                                    if official {
                                        span { class: "lb-board-num", title: OFFICIAL_TITLE,
                                            {p.rating.official.map(|o| official_skill(&o, false)).unwrap_or_default()}
                                        }
                                    }
                                    span { class: "lb-board-num", "{p.rating.games}" }
                                    span { class: "lb-board-num", "{p.stats.games}" }
                                    span { class: "lb-board-num", "{p.stats.wins}" }
                                }
                            }
                        }
                    }},
                }
            }
        }
    }
}

/// A rating on dominion.games' scale: "38.2", with its deviation "38.2 ± 1.5", or
/// "< 30.0" beyond the anchors.
pub fn official_skill(o: &OfficialSkill, deviation: bool) -> String {
    match o.beyond {
        None if deviation => format!("{:.1} ± {:.1}", o.skill, o.deviation),
        None => format!("{:.1}", o.skill),
        Some(Beyond::Below) => format!("< {:.1}", o.skill),
        Some(Beyond::Above) => format!("> {:.1}", o.skill),
    }
}

fn without_rating(user: &dominion_protocol::lobby::UserRef) -> dominion_protocol::lobby::UserRef {
    let mut user = user.clone();
    user.rating = None;
    user
}

/// A player's latest rated games' ratings, oldest first: where the first began, then
/// where each left it.
pub fn rating_line(recent: &[PastGame], name: &str) -> Vec<f64> {
    let ratings = recent.iter().rev().filter_map(|g| seat_of(g, name)?.rating);
    let mut line = vec![];
    for (before, after) in ratings {
        if line.is_empty() {
            line.push(before);
        }
        line.push(after);
    }
    line
}

/// The points of a line through `values`, in a `w` by `h` box.
fn spark(values: &[f64], w: f64, h: f64) -> String {
    let (lo, hi) = values
        .iter()
        .fold((f64::MAX, f64::MIN), |(lo, hi), &v| (lo.min(v), hi.max(v)));
    let span = (hi - lo).max(1.0);
    let step = w / (values.len() - 1) as f64;
    (values.iter().enumerate())
        .map(|(i, v)| format!("{:.1},{:.1}", i as f64 * step, h - (v - lo) / span * h))
        .collect::<Vec<_>>()
        .join(" ")
}

#[component]
fn ProfileCard(profile: Profile) -> Element {
    let Profile {
        user,
        rating,
        stats,
        official_games,
        agent,
        recent,
    } = profile;
    let line = rating_line(&recent, &user.name);
    let mut numbers = vec![
        (
            "Rating",
            format!("{}", rating.rating.round()),
            format!("± {}", rating.deviation.round()),
        ),
        ("Rated games", rating.games.to_string(), String::new()),
        (
            "Arena games won",
            stats.wins.to_string(),
            format!("of {}", stats.games),
        ),
        (
            "dominion.games games",
            official_games.to_string(),
            String::new(),
        ),
    ];
    let what = match (user.kind, &user.owner, &agent) {
        (_, _, Some(a)) => agent_line(a),
        (UserKind::Registered, _, _) => "Registered player".to_string(),
        (UserKind::Guest, _, _) => "Playing as a guest".to_string(),
        (UserKind::Bot, Some(owner), _) => format!("{owner}'s bot"),
        (UserKind::Bot, None, _) => "One of the server's agents".to_string(),
    };
    if let Some(o) = &rating.official {
        numbers.insert(
            1,
            (
                "dominion.games skill",
                official_skill(o, false),
                format!("± {:.1}", o.deviation),
            ),
        );
    }
    rsx! {
        div { class: "lb-fieldset lb-profile", "data-profile": "{user.name}",
            div { class: "lb-fieldset-title",
                span { "{user.name}" }
            }
            div { class: "lb-profile-who",
                span { "{what}" }
                Link { class: "lb-note-link", to: Route::Leaderboard {}, "Leaderboard" }
            }
            div { class: "lb-profile-numbers",
                for (label, value, sub) in numbers {
                    div { key: "{label}", class: "lb-profile-number",
                        span { class: "lb-profile-label", "{label}" }
                        span { class: "lb-profile-value", "{value} " span { class: "lb-profile-sub", "{sub}" } }
                    }
                }
            }
            if line.len() >= 2 {
                div { class: "lb-profile-line",
                    span { class: "lb-profile-label",
                        "Rating over the latest rated games: {line[0].round()} → {line[line.len() - 1].round()}"
                    }
                    svg {
                        view_box: "0 0 100 20",
                        preserve_aspect_ratio: "none",
                        role: "img",
                        "aria-label": "Rating over the latest rated games",
                        polyline { points: spark(&line, 100.0, 20.0) }
                    }
                }
            }
        }
    }
}

#[component]
pub fn UserPage(name: String) -> Element {
    let profile = use_resource(use_reactive!(
        |name| async move { api::profile(&name).await }
    ));
    rsx! {
        LobbyFrame { tab: Tab::Leaderboard,
            match &*profile.read() {
                None => rsx! {},
                Some(Err(e)) => rsx! {
                    div { class: "lb-column",
                        p { class: "lb-error", "{e.message}" }
                        Link { class: "lb-note-link", to: Route::Leaderboard {}, "Leaderboard" }
                    }
                },
                Some(Ok(p)) => rsx! {
                    GameBrowser {
                        key: "{p.user.name}",
                        title: "Games",
                        user: Some(p.user.name.clone()),
                        top: rsx! { ProfileCard { profile: p.clone() } },
                    }
                },
            }
        }
    }
}
