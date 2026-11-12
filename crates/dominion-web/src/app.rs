//! The root component, routes and app-wide context.

use crate::api;
use crate::cards::CardDb;
use crate::platform;
use dioxus::prelude::*;
use std::rc::Rc;

const CSS: &str = include_str!("../assets/style.css");
const CARD_CSS: &str = include_str!("../assets/cards.css");
const LOBBY_CSS: &str = include_str!("../assets/lobby.css");

#[derive(Routable, Clone, PartialEq, Debug)]
#[rustfmt::skip]
pub enum Route {
    #[layout(Shell)]
    #[route("/")]
    Lobby {},
    #[route("/game/:id")]
    Game { id: String },
    #[route("/gallery")]
    Gallery {},
    #[route("/:..rest")]
    NotFound { rest: Vec<String> },
}

/// Card data from `/api/cards`, empty until it has loaded.
#[derive(Clone, Copy)]
pub struct Cards(pub Signal<Rc<CardDb>>);

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
    let assets = crate::assets::use_provide_assets();
    let asset_css = assets.read().css();
    rsx! {
        document::Title { "Dominion" }
        document::Style { "{CSS}" }
        document::Style { "{CARD_CSS}" }
        document::Style { "{LOBBY_CSS}" }
        style { "{asset_css}" }
        Router::<Route> {}
    }
}

#[component]
fn Shell() -> Element {
    rsx! {
        header { class: "topbar",
            Link { class: "brand", to: Route::Lobby {}, "Dominion" }
            nav {
                Link { to: Route::Lobby {}, "Lobby" }
                Link { to: Route::Gallery {}, "Gallery" }
            }
        }
        main { class: "content", Outlet::<Route> {} }
    }
}

#[component]
fn Lobby() -> Element {
    rsx! { crate::lobby::LobbyPage {} }
}

#[component]
fn Game(id: String) -> Element {
    rsx! { crate::game::GamePage { key: "{id}", id } }
}

#[component]
fn Gallery() -> Element {
    rsx! { crate::gallery::GalleryPage {} }
}

#[component]
fn NotFound(rest: Vec<String>) -> Element {
    rsx! {
        section { class: "panel",
            h2 { "Not found" }
            p { "Nothing lives at /{rest.join(\"/\")}." }
            Link { to: Route::Lobby {}, "Back to the lobby" }
        }
    }
}
