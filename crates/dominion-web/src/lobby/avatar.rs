//! Accounts' pictures: round, and the name's initial on a colour of its own where there is
//! none. The signed-in account's card at the foot of the tab column, and the Account tab's
//! controls for its picture.

use super::{Route, UserName};
use crate::account::use_account;
use dioxus::prelude::*;
use dominion_protocol::lobby::{Me, UserKind, UserRef};

/// A hue for `name`, the same every time.
fn hue(name: &str) -> u32 {
    let hash = (name.to_lowercase().bytes())
        .fold(2166136261u32, |h, b| (h ^ b as u32).wrapping_mul(16777619));
    hash % 360
}

/// `user`'s picture, or their initial. `size` is a CSS length.
#[component]
pub fn Avatar(user: UserRef, size: String) -> Element {
    let style = format!("--avatar-size: {size}; --avatar-hue: {}", hue(&user.name));
    let initial: String = (user.name.chars())
        .find(|c| c.is_alphanumeric())
        .map(|c| c.to_uppercase().collect())
        .unwrap_or_default();
    rsx! {
        span { class: "lb-avatar", style, "aria-hidden": "true",
            match user.avatar {
                Some(v) => rsx! {
                    img {
                        src: "/api/users/{crate::platform::encode(&user.name)}/avatar?v={v}",
                        alt: "",
                        draggable: false,
                    }
                },
                None => rsx! { span { class: "lb-avatar-initial", "{initial}" } },
            }
        }
    }
}

/// Who is signed in, at the foot of the tab column: their picture and name, and under
/// them their rating.
#[component]
pub fn ProfileCard(me: Me) -> Element {
    let nav = navigator();
    let r = &me.rating;
    let unrated = r.games == 0;
    let games = if r.games == 1 { "game" } else { "games" };
    let title = if unrated {
        "No rated games yet".to_string()
    } else {
        format!(
            "{} ± {} after {} rated {games}",
            r.rating.round(),
            r.deviation.round(),
            r.games
        )
    };
    let rating = r.rating.round();
    let deviation = r.deviation.round();
    rsx! {
        div { class: "lb-profile-card",
            div { class: "lb-profile-card-head",
                button {
                    class: "lb-profile-card-picture",
                    title: "Your account",
                    onclick: move |_| { nav.push(Route::Account {}); },
                    Avatar { user: me.user.clone(), size: "4.4vh" }
                }
                UserName { user: UserRef { rating: None, ..me.user.clone() }, link: true }
            }
            div { class: "lb-profile-card-rating", title,
                span { class: "lb-profile-card-label", "Rating" }
                if unrated {
                    span { class: "lb-profile-card-none", "Unrated" }
                } else {
                    span { class: "lb-profile-card-value", "data-rating": "{rating}",
                        "{rating}"
                        span { class: "lb-profile-card-sub", " ± {deviation}" }
                    }
                }
            }
        }
    }
}

/// The Account tab's picture with Change and Remove.
#[component]
pub fn PictureControls(me: Me) -> Element {
    let account = use_account();
    let mut error = use_signal(|| None::<String>);
    let mut busy = use_signal(|| false);
    let has = me.user.avatar.is_some();
    let can = me.user.kind != UserKind::Bot;
    rsx! {
        div { class: "lb-picture",
            Avatar { user: me.user.clone(), size: "9vh" }
            if can {
                div { class: "lb-picture-actions",
                    label { class: if busy() { "lb-oval lb-picture-button busy" } else { "lb-oval lb-picture-button" },
                        if has { "Change picture" } else { "Add a picture" }
                        input {
                            r#type: "file",
                            accept: "image/png, image/jpeg, image/webp, image/gif",
                            disabled: busy(),
                            onchange: move |evt: FormEvent| {
                                let Some(file) = evt.files().into_iter().next() else {
                                    return;
                                };
                                busy.set(true);
                                error.set(None);
                                spawn(async move {
                                    if let Err(e) = account.set_avatar(&file).await {
                                        error.set(Some(e.message));
                                    }
                                    busy.set(false);
                                });
                            },
                        }
                    }
                    if has {
                        button {
                            class: "lb-oval lb-picture-button",
                            disabled: busy(),
                            onclick: move |_| {
                                busy.set(true);
                                error.set(None);
                                spawn(async move {
                                    if let Err(e) = account.clear_avatar().await {
                                        error.set(Some(e.message));
                                    }
                                    busy.set(false);
                                });
                            },
                            "Remove"
                        }
                    }
                    if let Some(e) = error() {
                        p { class: "lb-error", role: "alert", "{e}" }
                    }
                }
            }
        }
    }
}
