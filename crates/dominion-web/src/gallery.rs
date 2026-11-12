//! Every card, for checking the card renderer. Until it exists, a list of names.

use crate::app::use_cards;
use dioxus::prelude::*;

#[component]
pub fn GalleryPage() -> Element {
    let cards = use_cards();
    rsx! {
        section { class: "panel",
            h2 { "Gallery" }
            p { class: "muted", "The card renderer will show every card here. For now, their names." }
            ul { class: "gallery",
                for c in cards.all().iter() {
                    li { key: "{c.name}", title: cards.tooltip(&c.name), "{c.name}" }
                }
            }
        }
    }
}
