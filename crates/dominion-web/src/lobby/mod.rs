//! The lobby, laid out like dominion.games': the painting, the tab column on the left
//! and one framed window per tab. Automatch (searching for a game, Bot Game, the lobby's
//! chat), My Table (the server's table you sit at), Tables (the lobby's tables and the
//! games to watch), Cardlists (your Banned cards list), Leaderboard and Account.

mod account;
mod cardlists;
mod chat;
mod home;
mod leaderboard;
pub mod my_table;
pub mod rules;
pub mod seat_link;
mod selector;
pub mod store;
pub mod tables;

use crate::account::{Account, use_account};
use crate::api::{self, ApiError};
use crate::app::{Cards, Route, Settings};
use crate::platform;
use crate::session::{self, HeldSeat};
use dioxus::prelude::*;
use dioxus::router::Navigator;
use dominion_protocol::api::SeatToken;
use dominion_protocol::lobby::{
    Lobby, LobbyChange, Me, NewTable, QueueState, SeatEdit, TableInfo, TableSpec, UserKind, UserRef,
};
use futures_util::StreamExt;
use rules::TableRules;

pub use account::AccountTab;
pub use cardlists::CardlistsTab;
pub use home::AutomatchTab;
pub use leaderboard::{LeaderboardTab, UserPage};
pub use my_table::MyTableTab;
pub use tables::TablesTab;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tab {
    Automatch,
    MyTable,
    Tables,
    Cardlists,
    Leaderboard,
    Account,
}

/// The lobby as the server last gave it, kept current over its change feed, and the
/// rules of the table this account sits at.
#[derive(Clone, Copy)]
pub struct LobbyCtx {
    pub lobby: Signal<Option<Lobby>>,
    /// The feed's chat counter: changes when something is said in the lobby or at a table.
    pub chat: Signal<u64>,
    /// The rules My Table shows: the host's working copy, the table's own for everyone
    /// else.
    pub rules: Signal<Option<TableRules>>,
    /// Why the host's rules aren't the table's: the server refused them, or they can't
    /// be sent.
    pub refused: Signal<Option<String>>,
    /// The table's rules as the server has them, as far as this browser knows.
    synced: Signal<Option<(String, TableSpec)>>,
    pushing: Signal<bool>,
    /// Every expansion, once the cards have loaded.
    sets: Memo<Vec<String>>,
    account: Account,
}

/// The status line at the bottom right ("Loading game...", errors).
#[derive(Clone, Copy)]
pub struct Toast(pub Signal<Option<String>>);

/// The table `lobby`'s asker sits at.
pub fn my_table(lobby: &Lobby) -> Option<&TableInfo> {
    let id = &lobby.table.as_ref()?.id;
    lobby.tables.iter().find(|t| t.id == *id)
}

fn name_now(account: Account) -> Option<String> {
    account.me.peek().as_ref().map(|me| me.user.name.clone())
}

fn hosts(lobby: &Lobby) -> bool {
    let you = lobby.you.as_ref().map(|me| me.user.name.as_str());
    my_table(lobby).is_some_and(|t| Some(t.host.as_str()) == you)
}

/// The tokens of the table's game that its host hands out: the token seats not played
/// at this screen.
pub fn handed_out(lobby: &Lobby) -> Vec<SeatToken> {
    let Some(mine) = &lobby.table else {
        return vec![];
    };
    let here = store::here(&mine.id);
    let other = |t: &&SeatToken| !here.contains(&t.name);
    mine.tokens.iter().filter(other).cloned().collect()
}

/// Holds the seats the account plays, and for a host the token seats played at this
/// screen; remembers the other token seats' tokens to hand out.
fn hold_seats(lobby: &Lobby) {
    let hold = |game: &str, seat: u8, name: &str, token: &str| {
        let held = session::seats(game);
        if !held.iter().any(|h| h.seat == seat && h.token == token) {
            let (name, token) = (name.to_string(), token.to_string());
            session::hold(game, HeldSeat { seat, name, token });
        }
    };
    for g in &lobby.games {
        hold(&g.game_id, g.seat, &g.name, &g.token);
    }
    let game = my_table(lobby).and_then(|t| t.game.as_ref().or(t.last_game.as_ref()));
    if let (Some(mine), Some(game)) = (&lobby.table, game) {
        let links = handed_out(lobby);
        for t in mine.tokens.iter().filter(|t| !links.contains(t)) {
            hold(game, t.seat, &t.name, &t.token);
        }
        if session::links(game) != links {
            session::give_out(game, &links);
        }
    }
}

impl LobbyCtx {
    /// The table this account sits at.
    pub fn table(&self) -> Option<TableInfo> {
        self.lobby.read().as_ref().and_then(my_table).cloned()
    }

    fn table_now(&self) -> Option<TableInfo> {
        self.lobby.peek().as_ref().and_then(my_table).cloned()
    }

    /// Whether this account hosts the table it sits at.
    pub fn is_host(&self) -> bool {
        self.lobby.read().as_ref().is_some_and(hosts)
    }

    fn is_host_now(&self) -> bool {
        self.lobby.peek().as_ref().is_some_and(hosts)
    }

    /// Whether this account's table started `game`, now or last.
    pub fn at_table_of(&self, game: &str) -> bool {
        self.table().is_some_and(|t| {
            t.game.as_deref() == Some(game) || t.last_game.as_deref() == Some(game)
        })
    }

    fn take(mut self, lobby: Lobby) {
        let held = self.lobby.peek().as_ref().map(|l| l.version);
        if held.is_some_and(|v| lobby.version < v) {
            return;
        }
        hold_seats(&lobby);
        let mut me = self.account.me;
        // What the lobby knows of the account is newer than what signing in said.
        let same = |m: &Me, you: &Me| m.user.name == you.user.name && m != you;
        if let Some(you) = &lobby.you
            && me.peek().as_ref().is_some_and(|m| same(m, you))
        {
            me.set(Some(you.clone()));
        }
        if self.lobby.peek().as_ref() != Some(&lobby) {
            self.lobby.set(Some(lobby));
        }
    }

    /// Asks the server for the lobby.
    pub async fn refresh(self) {
        let who = name_now(self.account);
        if let Ok(lobby) = api::lobby().await
            && name_now(self.account) == who
        {
            self.take(lobby);
        }
    }

    /// Puts a table the server answered with in the lobby held.
    fn took_table(mut self, info: TableInfo) {
        if let Some(lobby) = self.lobby.write().as_mut() {
            match lobby.tables.iter_mut().find(|t| t.id == info.id) {
                Some(t) => *t = info,
                None => lobby.tables.push(info),
            }
        }
    }

    /// Brings the rules shown in line with the table's: the server's, unless the host
    /// has changes of their own that it doesn't have yet.
    fn reconcile(mut self, lobby: &Lobby, sets: &[String]) {
        let Some(table) = my_table(lobby) else {
            if self.rules.peek().is_some() {
                self.rules.set(None);
                self.synced.set(None);
                self.refused.set(None);
            }
            return;
        };
        let now = (table.id.clone(), table.spec.clone());
        let before = self.synced.peek().clone();
        if before.as_ref() == Some(&now) && self.rules.peek().is_some() {
            return;
        }
        let unsent = hosts(lobby)
            && before.as_ref().is_some_and(|(id, spec)| {
                let mine = self.rules.peek().as_ref().map(|r| r.spec(sets));
                *id == table.id && mine.is_some_and(|mine| mine.as_ref() != Ok(spec))
            });
        self.synced.set(Some(now));
        if !unsent {
            self.rules
                .set(Some(TableRules::from_spec(&table.spec, sets)));
            self.refused.set(None);
        }
    }

    /// Changes the table's rules, for its host; they reach the server a moment later.
    pub fn edit(mut self, f: impl FnOnce(&mut TableRules)) {
        if !self.is_host_now() || self.table_now().is_none_or(|t| t.game.is_some()) {
            return;
        }
        if let Some(rules) = self.rules.write().as_mut() {
            f(rules);
        }
    }

    /// Sends the host's rules to the server unless it has them. An error says why the
    /// table doesn't have them.
    pub async fn push(mut self) -> Result<(), ApiError> {
        while *self.pushing.peek() {
            platform::sleep(50).await;
        }
        let Some(table) = self.table_now().filter(|t| t.game.is_none()) else {
            return Ok(());
        };
        if !self.is_host_now() {
            return Ok(());
        }
        let Some(rules) = self.rules.peek().clone() else {
            return Ok(());
        };
        let sets = self.sets.peek().clone();
        let spec = match rules.spec(&sets) {
            Ok(spec) => spec,
            Err(message) => {
                self.refused.set(Some(message.clone()));
                return Err(ApiError {
                    status: 400,
                    message,
                });
            }
        };
        let now = (table.id.clone(), spec);
        if self.synced.peek().as_ref() == Some(&now) {
            if self.refused.peek().is_some() {
                self.refused.set(None);
            }
            return Ok(());
        }
        self.pushing.set(true);
        let result = api::set_table_spec(&now.0, &now.1).await;
        self.pushing.set(false);
        match result {
            Ok(info) => {
                self.synced.set(Some(now));
                self.refused.set(None);
                store::set_rules(&rules);
                self.took_table(info);
                Ok(())
            }
            Err(e) => {
                self.refused.set(Some(e.message.clone()));
                if e.status == 404 {
                    self.refresh().await;
                }
                Err(e)
            }
        }
    }

    /// Replaces the table's seats (its host's doing).
    pub async fn set_seats(self, toast: Signal<Option<String>>, seats: Vec<SeatEdit>) {
        let Some(table) = self.table_now() else {
            return;
        };
        match api::set_table_seats(&table.id, &seats).await {
            Ok(info) => self.took_table(info),
            Err(e) => flash(toast, e.message),
        }
    }

    /// Says whether this account is ready. The host's rules go first, so that everyone
    /// is ready for what the table shows.
    pub async fn set_ready(self, toast: Signal<Option<String>>, ready: bool) {
        let Some(table) = self.table_now() else {
            return;
        };
        if ready && let Err(e) = self.push().await {
            return flash(toast, e.message);
        }
        match api::ready(&table.id, ready).await {
            // The game has started: the lobby has this account's seat in it.
            Ok(info) if info.game.is_some() => self.refresh().await,
            Ok(info) => self.took_table(info),
            Err(e) => flash(toast, e.message),
        }
    }

    pub async fn leave_table(self) {
        if let Some(table) = self.table_now() {
            let _ = api::leave_table(&table.id).await;
            self.refresh().await;
        }
    }
}

/// Provides [`LobbyCtx`], [`Toast`] and the tooltip to the app, and keeps the lobby
/// current.
pub fn use_provide_lobby() {
    let account = use_account();
    let cards = use_context::<Cards>().0;
    let sets = use_memo(move || rules::expansions(&cards.read()));
    let ctx = use_context_provider(|| LobbyCtx {
        lobby: Signal::new(None),
        chat: Signal::new(0),
        rules: Signal::new(None),
        refused: Signal::new(None),
        synced: Signal::new(None),
        pushing: Signal::new(false),
        sets,
        account,
    });
    use_context_provider(|| Toast(Signal::new(None)));
    use_context_provider(|| Tooltip(Signal::new(None)));

    // The feed is the account's: it starts over when someone else signs in.
    let who = use_memo(move || account.name());
    use_resource(move || {
        let _ = who();
        async move {
            ctx.refresh().await;
            let (tx, mut rx) = futures_channel::mpsc::unbounded::<LobbyChange>();
            let feed = api::watch_lobby(move |c| {
                let _ = tx.unbounded_send(c);
            });
            let worker = async {
                let mut ctx = ctx;
                while let Some(c) = rx.next().await {
                    if *ctx.chat.peek() != c.chat {
                        ctx.chat.set(c.chat);
                    }
                    let held = ctx.lobby.peek().as_ref().map(|l| l.version);
                    // A server that started over counts from the beginning again.
                    if held.is_some_and(|v| c.version < v)
                        && let Some(lobby) = ctx.lobby.write().as_mut()
                    {
                        lobby.version = 0;
                    }
                    if held != Some(c.version) {
                        ctx.refresh().await;
                    }
                }
            };
            futures_util::future::join(feed, worker).await;
        }
    });

    use_effect(move || {
        let sets = sets();
        let lobby = ctx.lobby.read();
        if let Some(lobby) = lobby.as_ref()
            && !sets.is_empty()
        {
            ctx.reconcile(lobby, &sets);
        }
    });

    // The host's changes go to the server once they have stopped for a moment.
    let mut edits = use_signal(|| 0u64);
    use_effect(move || {
        let _ = ctx.rules.read();
        let n = *edits.peek() + 1;
        edits.set(n);
        spawn(async move {
            platform::sleep(500).await;
            while *edits.peek() == n {
                match ctx.push().await {
                    Err(e) if e.status == 429 => platform::sleep(5000).await,
                    _ => break,
                }
            }
        });
    });
}

pub fn use_lobby() -> LobbyCtx {
    use_context::<LobbyCtx>()
}

pub fn use_toast() -> Signal<Option<String>> {
    use_context::<Toast>().0
}

/// Shows `message` in the status line for a few seconds.
pub fn flash(mut toast: Signal<Option<String>>, message: String) {
    toast.set(Some(message.clone()));
    // Whatever asked for it may be gone before the message is.
    dioxus::core::spawn_forever(async move {
        platform::sleep(4000).await;
        if toast.peek().as_deref() == Some(message.as_str()) {
            toast.set(None);
        }
    });
}

/// Your Banned cards list.
pub fn use_banned() -> Vec<String> {
    use_context::<Settings>().0.read().banned_cards.clone()
}

/// Makes sure someone is signed in (as a guest, where the server takes them); else says
/// why not and goes to the Account tab.
pub async fn signed_in(account: Account, toast: Signal<Option<String>>, nav: Navigator) -> bool {
    match account.ensure().await {
        Ok(_) => true,
        Err(e) => {
            flash(toast, e.message);
            nav.push(Route::Account {});
            false
        }
    }
}

/// Sits down at a new table with the rules hosted here last, unless this account
/// already sits at one. Whether it sits at a table now.
pub async fn create_table(
    ctx: LobbyCtx,
    account: Account,
    toast: Signal<Option<String>>,
    nav: Navigator,
) -> bool {
    if !signed_in(account, toast, nav).await {
        return false;
    }
    ctx.refresh().await;
    if ctx.table_now().is_some() {
        return true;
    }
    let sets = ctx.sets.peek().clone();
    let fresh = TableRules::new(&sets).spec(&sets).unwrap_or_default();
    let last = store::rules().and_then(|r| r.spec(&sets).ok());
    let spec = last.unwrap_or_else(|| fresh.clone());
    let mut made = api::new_table(&NewTable { spec, seats: None }).await;
    // Rules remembered from another server may name what this one doesn't have.
    if made.as_ref().is_err_and(|e| e.status == 400) {
        let spec = fresh;
        made = api::new_table(&NewTable { spec, seats: None }).await;
    }
    match made {
        Ok(_) => {
            ctx.refresh().await;
            true
        }
        Err(e) => {
            flash(toast, e.message);
            false
        }
    }
}

/// Sits down at table `id`. Whether that worked.
pub async fn join_table(
    ctx: LobbyCtx,
    account: Account,
    toast: Signal<Option<String>>,
    nav: Navigator,
    id: String,
) -> bool {
    if !signed_in(account, toast, nav).await {
        return false;
    }
    let joined = api::join_table(&id, None).await;
    ctx.refresh().await;
    match joined {
        Ok(_) => true,
        Err(e) => {
            flash(toast, e.message);
            false
        }
    }
}

/// Takes the page to a game this account's table or search has started, once: the
/// first time the lobby says so. A host with tokens to hand out goes to My Table,
/// where they are.
#[component]
pub fn LobbyWatcher() -> Element {
    let ctx = use_lobby();
    let nav = navigator();
    let route = use_route::<Route>();
    use_effect(use_reactive!(|route| {
        let lobby = ctx.lobby.read();
        let Some(lobby) = lobby.as_ref() else {
            return;
        };
        let started = my_table(lobby).and_then(|t| t.game.clone());
        let found = lobby.queue.as_ref();
        let found = found.filter(|q| q.state == QueueState::Matched);
        let Some(id) = started.clone().or(found.and_then(|q| q.game_id.clone())) else {
            return;
        };
        let playing = |game: &str| lobby.games.iter().any(|g| g.game_id == game && g.running);
        if !playing(&id) {
            return;
        }
        if let Route::Game { id: at } = &route
            && *at != id
            && playing(at)
        {
            return;
        }
        if store::enter(&id) || matches!(&route, Route::Game { id: at } if *at == id) {
            return;
        }
        if started.is_some() {
            let game = id.clone();
            spawn(async move {
                if let Ok(games) = api::games().await
                    && let Some(g) = games.into_iter().find(|g| g.game_id == game)
                {
                    store::set_last_kingdom(&tables::kingdom_code(&g));
                }
            });
        }
        if started.is_some() && hosts(lobby) && !handed_out(lobby).is_empty() {
            nav.push(Route::MyTable {});
        } else {
            nav.push(Route::Game { id });
        }
    }));
    rsx! {}
}

/// A player's name with what everyone sees of them: guest or bot, and their rating.
#[component]
pub fn UserName(user: UserRef, #[props(default)] link: bool) -> Element {
    let mark = match user.kind {
        UserKind::Registered => None,
        UserKind::Guest => Some("guest"),
        UserKind::Bot => Some("bot"),
    };
    let title = match (&user.kind, &user.owner) {
        (UserKind::Bot, Some(owner)) => format!("{owner}'s bot"),
        (UserKind::Bot, None) => "A bot".into(),
        (UserKind::Guest, _) => "Playing as a guest".into(),
        (UserKind::Registered, _) => String::new(),
    };
    rsx! {
        span { class: "lb-user", "data-user": "{user.name}", title,
            if link {
                Link { class: "lb-user-name", to: Route::User { name: user.name.clone() }, "{user.name}" }
            } else {
                span { class: "lb-user-name", "{user.name}" }
            }
            if let Some(m) = mark {
                span { class: "lb-user-kind", "{m}" }
            }
            if let Some(r) = user.rating {
                span { class: "lb-user-rating", "{r}" }
            }
        }
    }
}

/// Who is signed in, under the tabs: their name and rating and Log out, or Log in.
#[component]
fn Who() -> Element {
    let account = use_account();
    let ctx = use_lobby();
    let nav = navigator();
    let mut sure = use_signal(|| false);
    let Some(me) = account.me.read().clone() else {
        return rsx! {
            div { class: "lb-who",
                if *account.loaded.read() {
                    button {
                        class: "lb-oval lb-who-button",
                        "data-action": "log-in",
                        onclick: move |_| { nav.push(Route::Account {}); },
                        "Log in"
                    }
                }
            }
        };
    };
    let guest = me.user.kind == UserKind::Guest;
    rsx! {
        div { class: "lb-who", "data-me": "{me.user.name}",
            UserName { user: me.user.clone(), link: true }
            button {
                class: "lb-oval lb-who-button",
                "data-action": "log-out",
                title: if guest { "A guest account can't be signed in to again" } else { "" },
                onclick: move |_| {
                    if guest && !sure() {
                        sure.set(true);
                        return;
                    }
                    sure.set(false);
                    spawn(async move {
                        ctx.leave_table().await;
                        let _ = api::cancel_search().await;
                        account.logout().await;
                        nav.push(Route::Lobby {});
                    });
                },
                if sure() { "Lose this guest?" } else { "Log out" }
            }
        }
    }
}

#[component]
pub fn LobbyFrame(tab: Tab, children: Element) -> Element {
    let ctx = use_lobby();
    let account = use_account();
    let toast = use_toast();
    let nav = navigator();
    let has_table = ctx.lobby.read().as_ref().is_some_and(|l| l.table.is_some());
    let class = match tab {
        Tab::Automatch => "lb-window lb-automatch",
        Tab::MyTable => "lb-window lb-my-table",
        Tab::Tables => "lb-window lb-tables",
        Tab::Cardlists => "lb-window lb-cardlists",
        Tab::Leaderboard => "lb-window lb-leaderboard",
        Tab::Account => "lb-window lb-account",
    };
    let tabs = [
        (Tab::Automatch, "automatch", "Automatch", Route::Lobby {}),
        (
            Tab::MyTable,
            "table",
            if has_table { "My Table" } else { "New Table" },
            Route::MyTable {},
        ),
        (Tab::Tables, "tables", "Tables", Route::Tables {}),
        (
            Tab::Cardlists,
            "cardlists",
            "Cardlists",
            Route::Cardlists {},
        ),
        (
            Tab::Leaderboard,
            "leaderboard",
            "Leaderboard",
            Route::Leaderboard {},
        ),
        (Tab::Account, "account", "Account", Route::Account {}),
    ];
    rsx! {
        div { class: "lb-page",
            div { class: "lb-spear lb-spear-left" }
            div { class: "lb-spear lb-spear-right" }
            div {
                class: "lb-fullscreen",
                title: "Fullscreen",
                onclick: move |_| crate::platform::run_js(
                    "document.fullscreenElement ? document.exitFullscreen() : document.documentElement.requestFullscreen()",
                ),
            }
            ul { class: "lb-tabs",
                for (t, name, label, to) in tabs {
                    li {
                        key: "{name}",
                        class: if tab == t { "lb-tab selected" } else { "lb-tab" },
                        "data-tab": "{name}",
                        onclick: move |_| {
                            let to = to.clone();
                            if t == Tab::MyTable {
                                spawn(async move {
                                    if create_table(ctx, account, toast, nav).await {
                                        nav.push(to);
                                    }
                                });
                            } else {
                                nav.push(to);
                            }
                        },
                        button { class: "lb-tab-button", "{label}" }
                    }
                }
                Who {}
            }
            div { class: "lb-window-container",
                div { class, {children} }
            }
            if let Some(m) = toast() {
                div { class: "lb-status", role: "status", "{m}" }
            }
        }
    }
}

/// dominion.games' fieldset: a title on a rule, framed left, right and below.
#[component]
pub fn Fieldset(
    title: String,
    #[props(default)] class: String,
    #[props(default = VNode::empty())] after_title: Element,
    children: Element,
) -> Element {
    rsx! {
        div { class: "lb-fieldset {class}",
            div { class: "lb-fieldset-title", span { "{title}" } {after_title} }
            {children}
        }
    }
}

/// A row of a fieldset: label, gold rule, control.
#[component]
pub fn OptionRow(label: String, children: Element) -> Element {
    rsx! {
        div { class: "lb-row",
            div { class: "lb-row-label", "{label}" }
            div { class: "lb-row-rule" }
            {children}
        }
    }
}

/// dominion.games' on/off switch.
#[component]
pub fn Switch(on: bool, #[props(default)] fixed: bool, onclick: EventHandler<()>) -> Element {
    rsx! {
        div {
            class: if on { "lb-switch active" } else { "lb-switch" },
            class: if fixed { "fixed" },
            role: "switch",
            "aria-checked": "{on}",
            onclick: move |_| if !fixed { onclick.call(()) },
            div { class: "lb-switch-knob" }
        }
    }
}

/// The table-wide tooltip under the cursor's target.
#[derive(Clone, Copy)]
pub struct Tooltip(pub Signal<Option<(String, f64, f64)>>);

pub fn use_tooltip() -> Signal<Option<(String, f64, f64)>> {
    use_context::<Tooltip>().0
}

/// Shows `text` above the element under a mouse event.
pub fn show_tooltip(mut tip: Signal<Option<(String, f64, f64)>>, text: String, e: &MouseEvent) {
    let p = e.client_coordinates();
    tip.set(Some((text, p.x, p.y)));
}

#[component]
pub fn TooltipLayer() -> Element {
    let tip = use_tooltip();
    match tip() {
        Some((text, x, y)) => rsx! {
            div { class: "lb-tooltip", style: "left: {x}px; top: {y}px;", "{text}" }
        },
        None => rsx! {},
    }
}

/// Milliseconds since the Unix epoch.
#[cfg(all(feature = "web", target_arch = "wasm32"))]
pub fn now_ms() -> u64 {
    js_sys::Date::now() as u64
}

#[cfg(not(all(feature = "web", target_arch = "wasm32")))]
pub fn now_ms() -> u64 {
    0
}

/// A moment (milliseconds since the Unix epoch) as the reader's own calendar writes it.
#[cfg(all(feature = "web", target_arch = "wasm32"))]
pub fn date(ms: u64) -> String {
    let date = js_sys::Date::new(&wasm_bindgen::JsValue::from_f64(ms as f64));
    let day = date.to_locale_date_string("default", &wasm_bindgen::JsValue::UNDEFINED);
    let time = date.to_locale_time_string("default");
    format!("{day} {}", String::from(time))
}

#[cfg(not(all(feature = "web", target_arch = "wasm32")))]
pub fn date(_ms: u64) -> String {
    String::new()
}
