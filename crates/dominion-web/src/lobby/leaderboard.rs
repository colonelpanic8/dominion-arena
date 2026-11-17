//! The Leaderboard tab (the rated players, best first) and a player's page: their
//! rating, tallies and latest games.

use super::{Fieldset, LobbyFrame, OptionRow, Tab, UserName, date};
use crate::api;
use crate::app::Route;
use dioxus::prelude::*;
use dominion_protocol::lobby::{
    Beyond, GameRecord, Leaderboard, OfficialSkill, Profile, RecordPlayer, Source,
};

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
                                p { class: "lb-note", "Nobody has played a rated game yet. A game is rated when its table or search says so and every seat is a different registered player or bot." }
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

/// How a game ended, in words: the server's `provinces-gone` as "Provinces gone".
pub fn reason(code: &str) -> String {
    let words = code.replace('-', " ");
    let mut chars = words.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// One player of a finished game: place, name, points and, in a rated game, how their
/// rating moved.
pub fn record_line(p: &RecordPlayer) -> String {
    let mut line = format!("{}. {} {} VP", p.rank, p.name, p.vp);
    if p.resigned {
        line.push_str(", resigned");
    }
    if let Some((before, after)) = p.rating {
        let change = (after - before).round() as i64;
        line.push_str(&format!(" ({change:+})"));
    }
    line
}

#[component]
fn Record(game: GameRecord) -> Element {
    let source = match game.source {
        Source::Automatch => "Automatch",
        Source::Table => "Table",
        Source::Direct => "By token",
    };
    let kingdom = game
        .kingdom
        .iter()
        .chain(&game.landscapes)
        .cloned()
        .collect::<Vec<_>>()
        .join(", ");
    let mut players = game.players.clone();
    players.sort_by_key(|p| (p.rank, p.seat));
    rsx! {
        div { class: "lb-record", "data-record": "{game.game_id}", title: "{kingdom}",
            div { class: "lb-record-head",
                span { "{date(game.ended)}" }
                span { "{source}" }
                span { if game.rated { "Rated" } else { "Unrated" } }
                span { "{reason(&game.reason)}" }
            }
            div { class: "lb-record-players",
                for p in players.iter() {
                    span { key: "{p.seat}", class: if p.rank == 1 { "lb-record-player won" } else { "lb-record-player" },
                        "{record_line(p)}"
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
            div { class: "lb-columns lb-user-page",
                match &*profile.read() {
                    None => rsx! {},
                    Some(Err(e)) => rsx! {
                        div { class: "lb-column",
                            p { class: "lb-error", "{e.message}" }
                            Link { class: "lb-note-link", to: Route::Leaderboard {}, "Leaderboard" }
                        }
                    },
                    Some(Ok(Profile { user, rating, stats, recent })) => rsx! {
                        div { class: "lb-column", "data-profile": "{user.name}",
                            Fieldset { title: "{user.name}", class: "lb-account-summary",
                                OptionRow { label: "Player",
                                    UserName { user: without_rating(user) }
                                }
                                OptionRow { label: "Rating",
                                    span { class: "lb-value", "data-rating": "{rating.rating.round()}", "{rating.rating.round()} ± {rating.deviation.round()}" }
                                }
                                if let Some(o) = &rating.official {
                                    OptionRow { label: "dominion.games skill",
                                        span { class: "lb-value", title: OFFICIAL_TITLE, "{official_skill(o, true)}" }
                                    }
                                }
                                OptionRow { label: "Rated games",
                                    span { class: "lb-value", "{rating.games}" }
                                }
                                OptionRow { label: "Games won",
                                    span { class: "lb-value", "{stats.wins} of {stats.games}" }
                                }
                            }
                            Link { class: "lb-note-link", to: Route::Leaderboard {}, "Leaderboard" }
                        }
                        div { class: "lb-column",
                            Fieldset { title: "Latest Games", class: "lb-records",
                                div { class: "lb-records-list",
                                    if recent.is_empty() {
                                        p { class: "lb-note", "No finished games yet." }
                                    }
                                    for g in recent.iter() {
                                        Record { key: "{g.game_id}", game: g.clone() }
                                    }
                                }
                            }
                        }
                    },
                }
            }
        }
    }
}
