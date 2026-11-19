//! The Leaderboard tab (the rated players, best first) and a player's page: their
//! rating and tallies over their games.

use super::history::{GameBrowser, seat_of};
use super::who::{Face, agent_title, effort_title, makers_line, model_title};
use super::{Fieldset, LobbyFrame, Tab};
use crate::api;
use crate::app::Route;
use dioxus::prelude::*;
use dominion_protocol::lobby::{
    Beyond, Leader, Leaderboard, OfficialSkill, PastGame, PlayedBy, Profile, UserKind,
};

const OFFICIAL_TITLE: &str =
    "Skill on dominion.games' scale (mu, not level), read off agents rated on both servers";

#[component]
pub fn LeaderboardTab() -> Element {
    let board = use_resource(|| async { api::leaderboard().await });
    rsx! {
        LobbyFrame { tab: Tab::Leaderboard,
            div { class: "lb-board-page",
                Fieldset { title: "Leaderboard", class: "lb-board",
                    match &*board.read() {
                        None => rsx! {},
                        Some(Err(e)) => rsx! { p { class: "lb-error", "{e.message}" } },
                        Some(Ok(Leaderboard { players })) => rsx! { Standings { players: players.clone() } },
                    }
                }
            }
        }
    }
}

/// The rated players' lines under their heading, and how to read them.
#[component]
fn Standings(players: Vec<Leader>) -> Element {
    let official = players.iter().any(|p| p.rating.official.is_some());
    let line = if official {
        "lb-board-line lb-board-official"
    } else {
        "lb-board-line"
    };
    let scale = Scale::of(&players);
    rsx! {
        div { class: "{line} lb-board-head lb-list-head",
            span { class: "lb-board-rank", "#" }
            span { "Player" }
            span { class: "lb-board-rating", "Rating" }
            if official {
                span { class: "lb-board-num", title: OFFICIAL_TITLE, "dominion.games" }
            }
            span { class: "lb-board-num", "Games" }
            span { class: "lb-board-num", "Wins" }
            span { class: "lb-board-num", "Win rate" }
        }
        div { class: "lb-list lb-board-list",
            if players.is_empty() {
                p { class: "lb-note", "Nobody has played a rated game yet. A game is rated when its table or search says so and every seat is a different registered player or bot, or when it is made with a different one of the server's agents in every seat." }
            }
            for p in players.iter() {
                Standing { key: "{p.user.name}", leader: p.clone(), class: line, scale, official }
            }
        }
        p { class: "lb-board-foot",
            "Rated games only. Ratings are Glicko-2: the bar spans each rating ± its deviation, which narrows the more rated games a player has played."
        }
    }
}

/// The span of ratings the bars are drawn on: every rating ± its deviation.
#[derive(Clone, Copy, PartialEq)]
struct Scale {
    lo: f64,
    hi: f64,
}

impl Scale {
    fn of(players: &[Leader]) -> Scale {
        let (lo, hi) = (players.iter()).fold((f64::MAX, f64::MIN), |(lo, hi), p| {
            let r = &p.rating;
            (
                lo.min(r.rating - r.deviation),
                hi.max(r.rating + r.deviation),
            )
        });
        if lo < hi {
            Scale { lo, hi }
        } else {
            Scale {
                lo: 1000.0,
                hi: 2000.0,
            }
        }
    }

    /// Where `rating` lies, in percent of the bar.
    fn at(self, rating: f64) -> f64 {
        ((rating - self.lo) / (self.hi - self.lo) * 100.0).clamp(0.0, 100.0)
    }
}

#[component]
fn Standing(leader: Leader, class: &'static str, scale: Scale, official: bool) -> Element {
    let Leader {
        rank,
        user,
        rating,
        agent,
    } = leader;
    let place = match rank {
        1 => "lb-board-rank first",
        2 => "lb-board-rank second",
        3 => "lb-board-rank third",
        _ => "lb-board-rank",
    };
    let (title, effort, caption) = match &agent {
        Some(a) => (
            model_title(a),
            a.effort.as_deref().map(effort_title),
            makers_line(a),
        ),
        None => {
            let caption = match (user.kind, &user.owner) {
                (UserKind::Registered, _) => "Registered player".to_string(),
                (UserKind::Guest, _) => "Guest".to_string(),
                (UserKind::Bot, Some(owner)) => format!("{owner}'s bot"),
                (UserKind::Bot, None) => "Bot".to_string(),
            };
            (user.name.clone(), None, caption)
        }
    };
    let (low, mid, high) = (
        scale.at(rating.rating - rating.deviation),
        scale.at(rating.rating),
        scale.at(rating.rating + rating.deviation),
    );
    let span = format!("left: {low:.2}%; width: {:.2}%", high - low);
    let tick = format!("left: {mid:.2}%");
    let win_rate =
        (rating.games > 0).then(|| (rating.wins as f64 / rating.games as f64 * 100.0).round());
    let r = rating.rating.round();
    let d = rating.deviation.round();
    rsx! {
        Link {
            class: "{class} lb-board-player",
            to: Route::User { name: user.name.clone() },
            title: "{user.name}",
            "data-leader": "{user.name}",
            span { class: place, "{rank}" }
            span { class: "lb-board-who",
                Face {
                    user: user.clone(),
                    played_by: match &agent {
                        Some(a) => Some(PlayedBy::Agent(a.clone())),
                        None if user.kind == UserKind::Bot => Some(PlayedBy::Bot { owner: user.owner.clone() }),
                        None => None,
                    },
                    size: "calc(4.4 * var(--lu))",
                }
                span { class: "lb-board-text",
                    span { class: "lb-board-name",
                        span { class: "lb-board-title", "{title}" }
                        if let Some(e) = effort {
                            span { class: "lb-board-effort", title: "Reasoning effort", "{e}" }
                        }
                    }
                    span { class: "lb-caption lb-board-caption", "{caption}" }
                }
            }
            span { class: "lb-board-rating", title: "{r} ± {d}",
                span { class: "lb-board-score", "{r}" span { class: "lb-board-dev", " ± {d}" } }
                span { class: "lb-board-bar",
                    span { class: "lb-board-span", style: span }
                    span { class: "lb-board-tick", style: tick }
                }
            }
            if official {
                span { class: "lb-board-num", title: OFFICIAL_TITLE,
                    {rating.official.map(|o| official_skill(&o, false)).unwrap_or_default()}
                }
            }
            span { class: "lb-board-num", "{rating.games}" }
            span { class: "lb-board-num", "{rating.wins}" }
            span { class: "lb-board-num lb-board-rate",
                if let Some(w) = win_rate { "{w}%" }
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
        (
            "Rated games won",
            rating.wins.to_string(),
            format!("of {}", rating.games),
        ),
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
        (_, _, Some(a)) => agent_title(a),
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
        Fieldset { title: user.name.clone(), class: "lb-profile", "data-profile": "{user.name}",
            div { class: "lb-profile-who",
                if user.kind != UserKind::Bot || user.avatar.is_some() {
                    super::avatar::Avatar { user: user.clone(), size: "4vh" }
                }
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
