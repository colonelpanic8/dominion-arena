//! Signing in, and the Account tab: who you are, keeping a guest account, changing the
//! password, the bots (accounts for agents, each with an API key shown once), and for
//! admins, invite codes.

use super::{Fieldset, LobbyFrame, OptionRow, Tab, UserName};
use crate::account::use_account;
use crate::api;
use crate::app::Settings;
use crate::platform;
use dioxus::prelude::*;
use dominion_protocol::lobby::{BotKey, Invite, Me, NewInvite, UserKind};

#[derive(Clone, Copy, PartialEq)]
enum How {
    Login,
    Register,
    Guest,
}

/// Name and password: Log in, Register, and Play as guest where the server takes them.
/// Where it takes new accounts by invitation, or an invite link brought `invite`, an
/// invite code too.
#[component]
pub fn SignIn(invite: Option<String>) -> Element {
    let account = use_account();
    let settings = use_context::<Settings>().0;
    let mut name = use_signal(String::new);
    let mut password = use_signal(String::new);
    let linked = invite.is_some();
    let mut code = use_signal(|| invite.unwrap_or_default());
    let mut error = use_signal(|| None::<String>);
    let mut busy = use_signal(|| false);
    let (registration, guests, invites) = {
        let s = settings.read();
        (s.registration, s.guests, s.invites)
    };
    let ask_code = invites && (!registration || linked);
    let mut sign_in = move |how: How| {
        if busy() {
            return;
        }
        busy.set(true);
        error.set(None);
        let (n, p, c) = (
            name().trim().to_string(),
            password(),
            code().trim().to_string(),
        );
        spawn(async move {
            let result = match how {
                How::Login => account.login(&n, &p).await,
                How::Register => {
                    let c = (ask_code && !c.is_empty()).then_some(c.as_str());
                    account.register(&n, &p, c).await
                }
                How::Guest => account.guest((!n.is_empty()).then_some(n.as_str())).await,
            };
            busy.set(false);
            match result {
                Ok(_) => password.set(String::new()),
                Err(e) => error.set(Some(e.message)),
            }
        });
    };
    rsx! {
        Fieldset { title: "Log in", class: "lb-signin",
            form {
                class: "lb-form",
                onsubmit: move |e| {
                    e.prevent_default();
                    sign_in(How::Login);
                },
                input {
                    class: "lb-input",
                    name: "username",
                    autocomplete: "username",
                    maxlength: "20",
                    placeholder: "Username",
                    value: "{name}",
                    oninput: move |e| name.set(e.value()),
                }
                input {
                    class: "lb-input",
                    r#type: "password",
                    name: "password",
                    autocomplete: "current-password",
                    placeholder: "Password",
                    value: "{password}",
                    oninput: move |e| password.set(e.value()),
                }
                if ask_code {
                    input {
                        class: "lb-input",
                        name: "invite",
                        "data-field": "invite",
                        autocomplete: "off",
                        placeholder: "Invite code (to register)",
                        value: "{code}",
                        oninput: move |e| code.set(e.value()),
                    }
                }
                div { class: "lb-form-buttons",
                    button { class: "lb-button lb-gold", r#type: "submit", "data-action": "login", disabled: busy(), "Log in" }
                    if registration || invites {
                        button {
                            class: "lb-button lb-copper",
                            r#type: "button",
                            "data-action": "register",
                            title: "A new account under this name and password (at least 8 characters)",
                            disabled: busy(),
                            onclick: move |_| sign_in(How::Register),
                            "Register"
                        }
                    }
                    if guests {
                        button {
                            class: "lb-button lb-stone",
                            r#type: "button",
                            "data-action": "guest",
                            title: "No password: under the name above, or one the server makes up",
                            disabled: busy(),
                            onclick: move |_| sign_in(How::Guest),
                            "Play as guest"
                        }
                    }
                }
                if let Some(e) = error() {
                    p { class: "lb-error", role: "alert", "{e}" }
                }
            }
        }
    }
}

/// The Account tab; `invite` is the code an invite link brought.
#[component]
pub fn AccountTab(invite: Option<String>) -> Element {
    let account = use_account();
    let me = account.me.read().clone();
    let loaded = *account.loaded.read();
    rsx! {
        LobbyFrame { tab: Tab::Account,
            div { class: "lb-columns lb-account-page",
                match me {
                    None => rsx! {
                        div { class: "lb-column",
                            if loaded { SignIn { invite } }
                        }
                        div { class: "lb-column", About {} }
                    },
                    Some(me) => rsx! {
                        div { class: "lb-column",
                            Summary { me: me.clone() }
                            match me.user.kind {
                                UserKind::Guest => rsx! { KeepAccount { name: me.user.name.clone(), invite } },
                                UserKind::Registered => rsx! { ChangePassword {} },
                                UserKind::Bot => rsx! {},
                            }
                        }
                        div { class: "lb-column",
                            if me.admin {
                                Invites {}
                            }
                            if me.user.kind == UserKind::Registered {
                                Bots { me }
                            }
                            crate::online::OfficialRatings {}
                            About {}
                        }
                    },
                }
            }
        }
    }
}

#[component]
fn Summary(me: Me) -> Element {
    let rating = me.rating.rating.round();
    let deviation = me.rating.deviation.round();
    rsx! {
        Fieldset { title: "Account", class: "lb-account-summary",
            super::avatar::PictureControls { me: me.clone() }
            OptionRow { label: "Name",
                UserName { user: me.user.clone(), link: true }
            }
            OptionRow { label: "Rating",
                span { class: "lb-value", "data-rating": "{rating}", "{rating} ± {deviation}" }
            }
            OptionRow { label: "Rated games",
                span { class: "lb-value", "{me.rating.games}" }
            }
            OptionRow { label: "Games won",
                span { class: "lb-value", "{me.stats.wins} of {me.stats.games}" }
            }
        }
    }
}

/// The commit this client was built from, linked to the repository, and its time.
#[component]
fn About() -> Element {
    let commit = env!("DOMINION_COMMIT");
    let hash = commit.trim_end_matches("-dirty");
    let short = &hash[..hash.len().min(10)];
    let dirty = if hash.len() < commit.len() {
        " (modified)"
    } else {
        ""
    };
    let url = format!("{}/commit/{hash}", env!("CARGO_PKG_REPOSITORY"));
    let time = commit_time(env!("DOMINION_COMMIT_TIME"));
    rsx! {
        Fieldset { title: "About", class: "lb-about",
            OptionRow { label: "Version",
                if hash.is_empty() {
                    span { class: "lb-value", "unknown" }
                } else {
                    a { class: "lb-value lb-commit", href: "{url}", target: "_blank", rel: "noopener",
                        "{short}{dirty}"
                    }
                }
            }
            if let Some(time) = time {
                OptionRow { label: "Committed",
                    span { class: "lb-value", "{time}" }
                }
            }
        }
    }
}

/// `YYYYMMDDHHMMSS` (UTC) as `YYYY-MM-DD HH:MM UTC`.
fn commit_time(t: &str) -> Option<String> {
    if t.len() != 14 || !t.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some(format!(
        "{}-{}-{} {}:{} UTC",
        &t[..4],
        &t[4..6],
        &t[6..8],
        &t[8..10],
        &t[10..12]
    ))
}

/// A guest registers: the account stays theirs, with its games.
#[component]
fn KeepAccount(name: String, invite: Option<String>) -> Element {
    let account = use_account();
    let settings = use_context::<Settings>().0;
    let mut new_name = use_signal(|| name.clone());
    let mut password = use_signal(String::new);
    let linked = invite.is_some();
    let mut code = use_signal(|| invite.unwrap_or_default());
    let mut error = use_signal(|| None::<String>);
    let (registration, invites) = {
        let s = settings.read();
        (s.registration, s.invites)
    };
    let ask_code = invites && (!registration || linked);
    if !registration && !invites {
        return rsx! {
            Fieldset { title: "Guest", class: "lb-keep",
                p { class: "lb-note", "You are playing as a guest, and this browser holds the account's only session. This server takes no new accounts." }
            }
        };
    }
    rsx! {
        Fieldset { title: "Keep this Account", class: "lb-keep",
            p { class: "lb-note", "You are playing as a guest: this browser holds the account's only session. Give it a password to keep it, with its games and its Banned cards list." }
            form {
                class: "lb-form",
                onsubmit: move |e| {
                    e.prevent_default();
                    let (n, p, c) = (new_name().trim().to_string(), password(), code().trim().to_string());
                    spawn(async move {
                        let c = (ask_code && !c.is_empty()).then_some(c.as_str());
                        match account.register(&n, &p, c).await {
                            Ok(_) => error.set(None),
                            Err(e) => error.set(Some(e.message)),
                        }
                    });
                },
                input {
                    class: "lb-input",
                    name: "username",
                    autocomplete: "username",
                    maxlength: "20",
                    placeholder: "Username",
                    value: "{new_name}",
                    oninput: move |e| new_name.set(e.value()),
                }
                input {
                    class: "lb-input",
                    r#type: "password",
                    name: "password",
                    autocomplete: "new-password",
                    placeholder: "Password (at least 8 characters)",
                    value: "{password}",
                    oninput: move |e| password.set(e.value()),
                }
                if ask_code {
                    input {
                        class: "lb-input",
                        name: "invite",
                        "data-field": "invite",
                        autocomplete: "off",
                        placeholder: "Invite code",
                        value: "{code}",
                        oninput: move |e| code.set(e.value()),
                    }
                }
                div { class: "lb-form-buttons",
                    button { class: "lb-button lb-gold", r#type: "submit", "data-action": "keep", "Register" }
                }
                if let Some(e) = error() {
                    p { class: "lb-error", role: "alert", "{e}" }
                }
            }
        }
    }
}

#[component]
fn ChangePassword() -> Element {
    let mut current = use_signal(String::new);
    let mut new = use_signal(String::new);
    let mut result = use_signal(|| None::<Result<(), String>>);
    rsx! {
        Fieldset { title: "Change Password", class: "lb-password",
            form {
                class: "lb-form",
                onsubmit: move |e| {
                    e.prevent_default();
                    let (c, n) = (current(), new());
                    spawn(async move {
                        match api::set_password(&c, &n).await {
                            Ok(()) => {
                                current.set(String::new());
                                new.set(String::new());
                                result.set(Some(Ok(())));
                            }
                            Err(e) => result.set(Some(Err(e.message))),
                        }
                    });
                },
                input {
                    class: "lb-input",
                    r#type: "password",
                    name: "current",
                    autocomplete: "current-password",
                    placeholder: "Current password",
                    value: "{current}",
                    oninput: move |e| current.set(e.value()),
                }
                input {
                    class: "lb-input",
                    r#type: "password",
                    name: "new",
                    autocomplete: "new-password",
                    placeholder: "New password (at least 8 characters)",
                    value: "{new}",
                    oninput: move |e| new.set(e.value()),
                }
                div { class: "lb-form-buttons",
                    button { class: "lb-button lb-copper", r#type: "submit", "data-action": "change-password", "Change Password" }
                }
                match result() {
                    Some(Ok(())) => rsx! { p { class: "lb-note", role: "status", "Password changed. Your other sessions are signed out." } },
                    Some(Err(e)) => rsx! { p { class: "lb-error", role: "alert", "{e}" } },
                    None => rsx! {},
                }
            }
        }
    }
}

/// What to tell an agent so it can play as the bot whose key this is.
pub fn bot_line(origin: &str, key: &str) -> String {
    format!(
        "Connect to the dominion MCP server at {origin}/mcp and call sign_in with {{\"api_key\": \"{key}\"}}"
    )
}

/// The account's bots: make one, replace its key, delete it. A key is shown once, with
/// the line to give the agent.
#[component]
fn Bots(me: Me) -> Element {
    let account = use_account();
    let mut name = use_signal(String::new);
    let mut error = use_signal(|| None::<String>);
    let mut shown = use_signal(|| None::<BotKey>);
    let mut deleting = use_signal(|| None::<String>);
    let mut copied = use_signal(|| None::<&'static str>);
    let took = move |result: api::Result<BotKey>| {
        let (mut error, mut copied, mut shown) = (error, copied, shown);
        match result {
            Ok(key) => {
                error.set(None);
                copied.set(None);
                shown.set(Some(key));
            }
            Err(e) => error.set(Some(e.message)),
        }
    };
    let make = move || {
        let n = name().trim().to_string();
        if n.is_empty() {
            return;
        }
        spawn(async move {
            let made = api::new_bot(&n).await;
            if made.is_ok() {
                name.set(String::new());
            }
            took(made);
            account.refresh().await;
        });
    };
    let mut copy = move |text: &str| {
        copied.set(Some(if platform::copy(text) {
            "Copied."
        } else {
            "Couldn't copy: select the line and copy it."
        }));
    };
    let origin = platform::origin();
    rsx! {
        Fieldset { title: "Bots ({me.bots.len()}/5)", class: "lb-bots",
            p { class: "lb-note", "A bot is an account of its own for an agent: it signs in with an API key, searches for games and sits at tables like anyone, and has its own rating." }
            div { class: "lb-bots-list",
                for bot in me.bots.iter().cloned() {
                    {
                        let (n1, n2) = (bot.name.clone(), bot.name.clone());
                        let sure = deleting().as_deref() == Some(bot.name.as_str());
                        rsx! {
                            div { key: "{bot.name}", class: "lb-bot", "data-bot": "{bot.name}",
                                UserName { user: bot.clone(), link: true }
                                button {
                                    class: "lb-oval",
                                    "data-action": "new-key",
                                    title: "A new API key; the old one stops working",
                                    onclick: move |_| {
                                        let n = n1.clone();
                                        spawn(async move { took(api::new_bot_key(&n).await) });
                                    },
                                    "New Key"
                                }
                                button {
                                    class: "lb-oval",
                                    "data-action": "delete-bot",
                                    onclick: move |_| {
                                        let n = n2.clone();
                                        if !sure {
                                            deleting.set(Some(n));
                                            return;
                                        }
                                        deleting.set(None);
                                        spawn(async move {
                                            match api::delete_bot(&n).await {
                                                Ok(()) => {
                                                    if shown.peek().as_ref().is_some_and(|k| k.bot.name == n) {
                                                        shown.set(None);
                                                    }
                                                }
                                                Err(e) => error.set(Some(e.message)),
                                            }
                                            account.refresh().await;
                                        });
                                    },
                                    if sure { "Really delete?" } else { "Delete" }
                                }
                            }
                        }
                    }
                }
            }
            div { class: "lb-join-row",
                input {
                    class: "lb-input",
                    "data-field": "bot-name",
                    maxlength: "20",
                    placeholder: "A name for a new bot",
                    value: "{name}",
                    oninput: move |e| name.set(e.value()),
                    onkeydown: move |e| {
                        if e.key() == Key::Enter {
                            make();
                        }
                    },
                }
                button { class: "lb-button lb-copper lb-join-button", "data-action": "make-bot", onclick: move |_| make(), "Make Bot" }
            }
            if let Some(e) = error() {
                p { class: "lb-error", role: "alert", "{e}" }
            }
            if let Some(key) = shown() {
                {
                    let line = bot_line(&origin, &key.key);
                    let (l2, k2) = (line.clone(), key.key.clone());
                    rsx! {
                        div { class: "lb-bot-key", "data-key-of": "{key.bot.name}",
                            p { class: "lb-links-note", "{key.bot.name}'s API key, shown this once:" }
                            div { class: "lb-link-line", "data-key": "1", "{key.key}" }
                            p { class: "lb-note", "Give its agent this line, or have it send the key as the connection's bearer token (Authorization: Bearer <key>):" }
                            div { class: "lb-link-line", "data-mcp": "1", "{line}" }
                            div { class: "lb-link-buttons",
                                button { class: "lb-oval", "data-copy": "agent", onclick: move |_| copy(&l2), "Copy for Agent" }
                                button { class: "lb-oval", "data-copy": "key", onclick: move |_| copy(&k2), "Copy Key" }
                                if let Some(m) = copied() {
                                    span { class: "lb-copied", role: "status", "{m}" }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// An invite code's link, which opens the Account tab with the code filled in.
pub fn invite_link(origin: &str, code: &str) -> String {
    format!("{origin}/invite/{code}")
}

fn invite_state(invite: &Invite, now: u64) -> String {
    let used = invite.used_by.len() as u32;
    if used >= invite.uses {
        return "used up".into();
    }
    match invite.expires {
        Some(e) if e * 1000 <= now => "expired".into(),
        Some(e) => format!(
            "{used} of {} used, until {}",
            invite.uses,
            super::date(e * 1000)
        ),
        None => format!("{used} of {} used", invite.uses),
    }
}

/// An admin's invite codes: make one, copy its link, revoke it.
#[component]
fn Invites() -> Element {
    let mut list = use_signal(Vec::<Invite>::new);
    let mut uses = use_signal(|| "1".to_string());
    let mut days = use_signal(|| "30".to_string());
    let mut note = use_signal(String::new);
    let mut error = use_signal(|| None::<String>);
    let mut revoking = use_signal(|| None::<String>);
    let mut copied = use_signal(|| None::<(String, &'static str)>);
    let reload = move || {
        spawn(async move {
            match api::invites().await {
                Ok(l) => list.set(l),
                Err(e) => error.set(Some(e.message)),
            }
        });
    };
    use_hook(reload);
    let make = move || {
        let parse = |s: String| s.trim().parse::<u32>().ok();
        let new = NewInvite {
            uses: parse(uses()),
            days: parse(days()),
            note: Some(note().trim().to_string()).filter(|n| !n.is_empty()),
        };
        spawn(async move {
            match api::new_invite(&new).await {
                Ok(_) => {
                    error.set(None);
                    note.set(String::new());
                }
                Err(e) => error.set(Some(e.message)),
            }
            reload();
        });
    };
    let origin = platform::origin();
    let now = super::now_ms();
    rsx! {
        Fieldset { title: "Invites", class: "lb-invites",
            p { class: "lb-note", "An invite code lets someone register, and so play the server's agents, even while the server takes no new accounts. Send them its link." }
            div { class: "lb-bots-list",
                for invite in list().into_iter() {
                    {
                        let link = invite_link(&origin, &invite.code);
                        let (c1, c2) = (invite.code.clone(), invite.code.clone());
                        let sure = revoking().as_deref() == Some(invite.code.as_str());
                        let state = invite_state(&invite, now);
                        let used_by = invite.used_by.join(", ");
                        let shown = copied().filter(|(c, _)| *c == invite.code).map(|(_, m)| m);
                        rsx! {
                            div { key: "{invite.code}", class: "lb-invite", "data-invite": "{invite.code}",
                                div { class: "lb-bot",
                                    span { class: "lb-invite-code", "{invite.code}" }
                                    button {
                                        class: "lb-oval",
                                        "data-action": "copy-invite",
                                        onclick: move |_| {
                                            let m = if platform::copy(&link) { "Link copied." } else { "Couldn't copy: select the link." };
                                            copied.set(Some((c1.clone(), m)));
                                        },
                                        "Copy Link"
                                    }
                                    button {
                                        class: "lb-oval",
                                        "data-action": "revoke-invite",
                                        onclick: move |_| {
                                            let c = c2.clone();
                                            if !sure {
                                                revoking.set(Some(c));
                                                return;
                                            }
                                            revoking.set(None);
                                            spawn(async move {
                                                if let Err(e) = api::revoke_invite(&c).await {
                                                    error.set(Some(e.message));
                                                }
                                                reload();
                                            });
                                        },
                                        if sure { "Really revoke?" } else { "Revoke" }
                                    }
                                }
                                div { class: "lb-invite-about",
                                    if let Some(n) = &invite.note {
                                        span { class: "lb-invite-note", "{n} · " }
                                    }
                                    "{state}"
                                    if !invite.used_by.is_empty() {
                                        " · registered: {used_by}"
                                    }
                                }
                                if let Some(m) = shown {
                                    div { class: "lb-link-line", "{invite_link(&origin, &invite.code)}" }
                                    span { class: "lb-copied", role: "status", "{m}" }
                                }
                            }
                        }
                    }
                }
            }
            div { class: "lb-join-row lb-invite-row",
                input {
                    class: "lb-input lb-invite-number",
                    r#type: "number",
                    min: "1",
                    max: "100",
                    title: "How many people may register with it",
                    "data-field": "invite-uses",
                    value: "{uses}",
                    oninput: move |e| uses.set(e.value()),
                }
                span { class: "lb-invite-unit", "uses" }
                input {
                    class: "lb-input lb-invite-number",
                    r#type: "number",
                    min: "0",
                    max: "365",
                    title: "Days until it expires (0: never)",
                    "data-field": "invite-days",
                    value: "{days}",
                    oninput: move |e| days.set(e.value()),
                }
                span { class: "lb-invite-unit", "days" }
            }
            div { class: "lb-join-row",
                input {
                    class: "lb-input",
                    "data-field": "invite-note",
                    maxlength: "100",
                    placeholder: "Who it is for (a note)",
                    value: "{note}",
                    oninput: move |e| note.set(e.value()),
                    onkeydown: move |e| {
                        if e.key() == Key::Enter {
                            make();
                        }
                    },
                }
                button { class: "lb-button lb-copper lb-join-button", "data-action": "make-invite", onclick: move |_| make(), "Make Invite" }
            }
            if let Some(e) = error() {
                p { class: "lb-error", role: "alert", "{e}" }
            }
        }
    }
}
