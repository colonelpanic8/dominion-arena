//! The root component, routes and app-wide context.

use crate::api;
use crate::cards::CardDb;
use crate::platform;
use dioxus::prelude::*;
use dominion_protocol::api::WebSettings;
use std::rc::Rc;

const TOKENS_CSS: &str = include_str!("../assets/tokens.css");
const CSS: &str = include_str!("../assets/style.css");
const CARD_CSS: &str = include_str!("../assets/cards.css");
const LOBBY_CSS: &str = include_str!("../assets/lobby.css");
const COUNTS_CSS: &str = include_str!("../assets/counts.css");

#[derive(Routable, Clone, PartialEq, Debug)]
#[rustfmt::skip]
pub enum Route {
    #[layout(Shell)]
    #[route("/")]
    Lobby {},
    #[route("/table")]
    MyTable {},
    #[route("/tables")]
    Tables {},
    #[route("/cardlists")]
    Cardlists {},
    #[route("/leaderboard")]
    Leaderboard {},
    #[route("/history")]
    History {},
    #[route("/account")]
    Account {},
    #[route("/invite/:code")]
    Invited { code: String },
    #[route("/user/:name")]
    User { name: String },
    #[route("/game/:id/agents")]
    Transcript { id: String },
    #[route("/game/:id/replay")]
    GameReplay { id: String },
    #[route("/game/:id")]
    Game { id: String },
    #[route("/online")]
    Online {},
    #[route("/online/:account")]
    OnlineAccount { account: String },
    #[route("/gallery")]
    Gallery {},
    #[route("/:..rest")]
    NotFound { rest: Vec<String> },
}

/// Card data from `/api/cards`, empty until it has loaded.
#[derive(Clone, Copy)]
pub struct Cards(pub Signal<Rc<CardDb>>);

/// The server's settings for the web client, the defaults until they have loaded. The
/// Banned cards list among them is the signed-in account's.
#[derive(Clone, Copy)]
pub struct Settings(pub Signal<WebSettings>);

/// The card data, subscribing the calling component to its arrival.
pub fn use_cards() -> Rc<CardDb> {
    use_context::<Cards>().0.read().clone()
}

#[component]
pub fn App() -> Element {
    let mut cards = use_context_provider(|| Cards(Signal::new(Rc::new(CardDb::default()))));
    use_future(move || async move {
        loop {
            match api::cards().await {
                Ok(list) => {
                    cards.0.set(Rc::new(CardDb::new(list)));
                    break;
                }
                Err(_) => platform::sleep(3000).await,
            }
        }
    });
    let mut settings = use_context_provider(|| Settings(Signal::new(WebSettings::default())));
    let account = crate::account::use_provide_account();
    let who = use_memo(move || account.name());
    use_resource(move || {
        let _ = who();
        async move {
            for _ in 0..5 {
                match api::settings().await {
                    Ok(s) => {
                        settings.0.set(s);
                        break;
                    }
                    Err(_) => platform::sleep(3000).await,
                }
            }
        }
    });
    crate::lobby::use_provide_lobby();
    let assets = crate::assets::use_provide_assets();
    let asset_css = assets.read().css();
    rsx! {
        document::Title { "Dominion" }
        document::Style { "{TOKENS_CSS}" }
        document::Style { "{CSS}" }
        document::Style { "{CARD_CSS}" }
        document::Style { "{LOBBY_CSS}" }
        document::Style { "{COUNTS_CSS}" }
        document::Style { "{crate::rating::CSS}" }
        style { "{asset_css}" }
        Router::<Route> {}
    }
}

#[component]
fn Shell() -> Element {
    let account = crate::account::use_account();
    rsx! {
        header { class: "topbar",
            Link { class: "brand", to: Route::Lobby {}, "Dominion" span { "Arena" } }
            nav { "aria-label": "Main",
                Link { to: Route::Lobby {}, active_class: "on", "Lobby" }
                Link { to: Route::Gallery {}, active_class: "on", "Gallery" }
            }
            match account.name() {
                Some(name) => rsx! { Link { class: "who", to: Route::Account {}, "{name}" } },
                None => rsx! { Link { class: "who", to: Route::Lobby {}, "Log in" } },
            }
        }
        main { class: "content", Outlet::<Route> {} }
        crate::lobby::LobbyWatcher {}
    }
}

#[component]
fn Lobby() -> Element {
    rsx! { crate::lobby::AutomatchTab {} }
}

#[component]
fn MyTable() -> Element {
    rsx! { crate::lobby::MyTableTab {} }
}

#[component]
fn Tables() -> Element {
    rsx! { crate::lobby::TablesTab {} }
}

#[component]
fn Cardlists() -> Element {
    rsx! { crate::lobby::CardlistsTab {} }
}

#[component]
fn Leaderboard() -> Element {
    rsx! { crate::lobby::LeaderboardTab {} }
}

#[component]
fn History() -> Element {
    rsx! { crate::lobby::HistoryTab {} }
}

#[component]
fn Account() -> Element {
    rsx! { crate::lobby::AccountTab {} }
}

/// An invite link: the Account tab, its invite code filled in.
#[component]
fn Invited(code: String) -> Element {
    rsx! { crate::lobby::AccountTab { invite: code } }
}

#[component]
fn User(name: String) -> Element {
    rsx! { crate::lobby::UserPage { name } }
}

#[component]
fn Game(id: String) -> Element {
    // A keyed list, so that another game's page is a new page.
    rsx! {
        for id in [id] {
            crate::game::GamePage { key: "{id}", id }
        }
    }
}

#[component]
fn GameReplay(id: String) -> Element {
    rsx! { for id in [id] { crate::game::ReplayPage { key: "{id}", id } } }
}

#[component]
fn Transcript(id: String) -> Element {
    rsx! { for id in [id] { crate::transcript::TranscriptPage { key: "{id}", id } } }
}

#[component]
fn Gallery() -> Element {
    rsx! { crate::gallery::GalleryPage {} }
}

#[component]
fn NotFound(rest: Vec<String>) -> Element {
    rsx! {
        section { class: "panel not-found",
            h2 { "Not found" }
            span { class: "big", "404" }
            p { "Nothing lives at " code { "/{rest.join(\"/\")}" } "." }
            Link { class: "button copper", to: Route::Lobby {}, "Back to the lobby" }
        }
    }
}

#[component]
fn Online() -> Element {
    rsx! {crate::online::OnlinePage {account: None}}
}

#[component]
fn OnlineAccount(account: String) -> Element {
    rsx! {crate::online::OnlinePage {key: "{account}", account: Some(account.clone())}}
}
