//! dominion.games through the official accounts the server keeps connected: the page
//! watching one's game on the table, the lobby's panel where its owner sends one of the
//! server's agents to play as it, and its rating there for the Account tab.

use crate::account::use_account;
use crate::app::Route;
use crate::lobby::agent_picker::AgentPicker;
use crate::lobby::{Fieldset, use_lobby};
use crate::{
    api,
    game::{GameShell, GameState, Offers, table, use_watched_game},
    platform,
};
use dioxus::prelude::*;
use dominion_protocol::api::{
    GameSummary, OnlineManagement, OnlineMatching, OnlineMode, OnlineObservation, OnlineOutcome,
    OnlineRating, OnlineRatingType, OnlineStart,
};
use dominion_protocol::lobby::{OfficialAccount, official_game_id};

const CSS: &str = include_str!("../assets/online.css");

#[derive(Clone)]
enum Action {
    Start,
    Attach,
    Resume(String),
    Pause,
    Resign,
    Reconnect,
    Cancel,
}

fn card_names(text: &str) -> Vec<String> {
    text.split([',', '\n'])
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

/// The official accounts the signed-in player sends agents to play as.
fn use_owned() -> Vec<OfficialAccount> {
    let ctx = use_lobby();
    let account = use_account();
    let me = account.name();
    let lobby = ctx.lobby.read();
    (lobby.iter().flat_map(|l| &l.official))
        .filter(|a| me.as_deref() == Some(a.owner.as_str()))
        .cloned()
        .collect()
}

fn apply(mut state: Signal<GameState>, mut game: Signal<String>, update: OnlineObservation) {
    table::before_change();
    let mut st = state.write();
    match update.game {
        Some(sv) => {
            game.set(sv.view.game.id.clone().unwrap_or_default());
            st.server_ahead = crate::game::server_ahead(sv.meta.now, st.server_ahead);
            st.apply_whole(sv);
        }
        None => *st = GameState::default(),
    }
    st.error = update
        .error
        .or_else(|| (!update.connected).then(|| "Official connection is disconnected".into()));
}

/// An official account's game on the table, for anyone to watch: `account` none is the
/// primary account's. Its owner also gets the account's panel under the log.
#[component]
pub fn OnlinePage(account: Option<String>) -> Element {
    let id = use_signal(|| account.clone());
    let mut primary = use_signal(|| None::<String>);
    use_future(move || async move {
        if id.peek().is_none()
            && let Ok(list) = api::online_accounts().await
        {
            primary.set(list.into_iter().find(|a| a.primary).map(|a| a.id));
        }
    });
    let owned = use_owned();
    let shown = account.clone().or(primary());
    let owner = (shown.as_ref()).filter(|s| owned.iter().any(|a| &&a.id == s));
    let mut state = use_signal(GameState::default);
    let game = use_signal(String::new);
    let summary = use_signal(|| None::<GameSummary>);
    use_context_provider(|| summary);
    let offers = use_memo(move || Offers {
        chat: false,
        transcript: Some(game())
            .filter(|g| !g.is_empty())
            .map(|g| official_game_id(&g)),
        transcript_at: None,
    });
    use_watched_game(game, state, offers);
    use_future(move || async move {
        loop {
            let account = id.peek().clone();
            match api::online_view(account.as_deref()).await {
                Ok(view) => {
                    apply(state, game, view);
                    api::follow_online(account.as_deref(), &mut |view| apply(state, game, view))
                        .await;
                }
                Err(e) => {
                    let mut current = state.write();
                    if matches!(e.status, 401 | 403) {
                        current.view = None;
                        current.log.clear();
                    }
                    current.error = Some(e.to_string());
                }
            }
            platform::sleep(3000).await;
        }
    });
    let error = state.read().error.clone();
    let playing = (state.read().view.as_ref())
        .and_then(|v| v.you.as_ref())
        .map(|y| y.name.clone());
    let controls = rsx! {
        if let Some(account) = owner.cloned() {
            OfficialPanel { key: "{account}", account }
        }
        if let Some(error) = error.clone() {
            p { class: "error", "{error}" }
        }
    };
    rsx! {
        style { {CSS} }
        if state.read().view.is_some() {
            GameShell {
                title: format!("dominion.games {}", game().rsplit(':').next().unwrap_or_default()),
                who: rsx! {
                    span { class: "watching-mark", "data-watching": "1", "watching" }
                    if let Some(name) = playing {
                        span { class: "muted", "as {name}" }
                    }
                },
                extra: controls,
            }
        } else {
            section { class: "online-idle",
                h1 { class: "page-title", "dominion.games" }
                p { class: "page-lede",
                    "No game on dominion.games right now. "
                    Link { to: Route::Lobby {}, "Back to the lobby" }
                }
                div { class: "panel", {controls} }
            }
        }
    }
}

/// The lobby's dominion.games box: for the owner of an official account, sending one of
/// the server's agents to play as it.
#[component]
pub fn OfficialBox() -> Element {
    let owned = use_owned();
    let mut picked = use_signal(|| None::<String>);
    let nav = navigator();
    let Some(shown) = (owned.iter())
        .find(|a| picked().as_ref() == Some(&a.id))
        .or(owned.first())
        .cloned()
    else {
        return rsx! {};
    };
    let running = shown.game.as_ref().is_some_and(|g| g.running);
    rsx! {
        style { {CSS} }
        Fieldset { title: "dominion.games", class: "lb-official",
            p { class: "lb-note",
                "Send one of the server's agents to play on dominion.games as {shown.account}. "
                "Its game is listed on the Tables tab, and the agent's history keeps it."
            }
            if owned.len() > 1 {
                div { class: "lb-multibutton",
                    for a in owned.iter() {
                        {
                            let id = a.id.clone();
                            rsx! {
                                div {
                                    key: "{a.id}",
                                    class: if a.id == shown.id { "lb-multi active" } else { "lb-multi" },
                                    onclick: move |_| picked.set(Some(id.clone())),
                                    "{a.account}"
                                }
                            }
                        }
                    }
                }
            }
            if running {
                {
                    let account = shown.id.clone();
                    rsx! {
                        button {
                            class: "lb-button lb-gold",
                            "data-action": "watch-official",
                            onclick: move |_| { nav.push(Route::OnlineAccount { account: account.clone() }); },
                            "Watch"
                        }
                    }
                }
            }
            {
                let account = shown.id.clone();
                rsx! { OfficialPanel { key: "{account}", account } }
            }
        }
    }
}

/// What the owner of official account `account` does with it: start a game or a search
/// with an agent, cancel the search, pause or resume the agent, resign, reconnect.
#[component]
fn OfficialPanel(account: String) -> Element {
    let id = use_signal(|| account.clone());
    let mut management = use_signal(|| None::<OnlineManagement>);
    let mut agent = use_signal(String::new);
    let mut kingdom = use_signal(String::new);
    let mut landscapes = use_signal(String::new);
    let mut bots = use_signal(|| 1_u8);
    let mut mode = use_signal(|| OnlineMode::Rated);
    let mut players = use_signal(|| 2_u8);
    let mut min_rating = use_signal(|| -10_i32);
    let mut max_rating = use_signal(|| 10_i32);
    let mut timer = use_signal(|| 3_u8);
    let mut busy = use_signal(|| false);
    let mut action_error = use_signal(|| None::<String>);
    let mut confirming = use_signal(|| false);
    use_future(move || async move {
        loop {
            match api::online_management(Some(&id.peek())).await {
                Ok(status) => {
                    let first = (status.models.first())
                        .and_then(|m| m.default_choice())
                        .map(|c| &c.agent)
                        .or(status.agents.first());
                    // A selection no longer offered goes back to the default.
                    if !status.agents.contains(&agent.peek()) {
                        agent.set(first.cloned().unwrap_or_default());
                    }
                    management.set(Some(status));
                }
                Err(e) if matches!(e.status, 401 | 403) => management.set(None),
                Err(_) => {}
            }
            platform::sleep(3000).await;
        }
    });
    let execute = use_callback(move |action: Action| {
        if *busy.peek() {
            return;
        }
        busy.set(true);
        action_error.set(None);
        confirming.set(false);
        let selected = agent.peek().clone();
        let bot_game = *mode.peek() == OnlineMode::Bots;
        let request = OnlineStart {
            agent: selected.clone(),
            mode: *mode.peek(),
            matching: OnlineMatching {
                players: *players.peek(),
                min_rating: *min_rating.peek(),
                max_rating: *max_rating.peek(),
                timer: *timer.peek(),
            },
            kingdom: if bot_game {
                card_names(&kingdom.peek())
            } else {
                vec![]
            },
            landscapes: if bot_game {
                card_names(&landscapes.peek())
            } else {
                vec![]
            },
            bots: *bots.peek(),
        };
        let account = id.peek().clone();
        spawn(async move {
            let account = Some(account.as_str());
            let result = match action {
                Action::Start => api::online_start(account, &request).await,
                Action::Attach => api::online_attach(account, selected).await,
                Action::Resume(agent) => api::online_attach(account, agent).await,
                Action::Pause => api::online_manage_action(account, "pause").await,
                Action::Resign => api::online_manage_action(account, "resign").await,
                Action::Cancel => api::online_manage_action(account, "cancel").await,
                Action::Reconnect => api::online_manage_action(account, "reconnect").await,
            };
            match result {
                Ok(status) => management.set(Some(status)),
                Err(e) => action_error.set(Some(e.to_string())),
            }
            busy.set(false);
        });
    });
    rsx! {
        div { class: "online-controls",
            if let Some(status) = management.read().as_ref() {
                p { class: "muted",
                    if status.connected { "Official account connected" } else { "Official account disconnected" }
                    if let Some(account) = &status.account { " · {account}" }
                }
                if status.running || status.searching {
                    p { class: "muted",
                        match status.mode {
                            OnlineMode::Bots => "Unrated server bots",
                            OnlineMode::Unrated => "Unrated public humans",
                            OnlineMode::Rated => "Rated public humans",
                        }
                    }
                }
                // A running game keeps the agent it started with.
                if !status.running || (status.agent.is_none() && status.game_agent.is_none()) {
                label { "Agent"
                    if status.models.is_empty() {
                        select { value: "{agent}", disabled: busy(), onchange: move |e| agent.set(e.value()),
                            for name in &status.agents { option { value: "{name}", "{name}" } }
                        }
                    } else {
                        AgentPicker {
                            models: status.models.clone(),
                            value: Some(agent()).filter(|a| !a.is_empty()),
                            disabled: busy(),
                            onpick: move |name: Option<String>| agent.set(name.unwrap_or_default()),
                        }
                    }
                }
                }
                if status.agents.is_empty() {
                    p { class: "muted", "The agent runner is unavailable. No game will be started without an agent." }
                }
                for feedback in &status.queue_feedback { p { class: "muted", "{feedback}" } }
                if status.searching {
                    p { "Searching for a public match… The selected agent starts when a match is found." }
                    button { disabled: busy(), onclick: move |_| execute.call(Action::Cancel), "Cancel matchmaking" }
                } else if !status.running {
                    label { "Game type"
                        select {
                            value: match mode() { OnlineMode::Bots => "bots", OnlineMode::Unrated => "unrated", OnlineMode::Rated => "rated" },
                            disabled: busy(),
                            onchange: move |e| mode.set(match e.value().as_str() { "bots" => OnlineMode::Bots, "unrated" => OnlineMode::Unrated, _ => OnlineMode::Rated }),
                            option { value: "rated", "Public humans · rated" }
                            option { value: "unrated", "Public humans · unrated" }
                            option { value: "bots", "Server bots · unrated" }
                        }
                    }
                    if mode() == OnlineMode::Bots {
                        label { "Server bots"
                            input { r#type: "number", min: "1", max: "5", value: "{bots}", disabled: busy(), oninput: move |e| { if let Ok(n) = e.value().parse() { bots.set(n); } } }
                        }
                        label { "Kingdom (optional, comma-separated)"
                            textarea { value: "{kingdom}", disabled: busy(), oninput: move |e| kingdom.set(e.value()) }
                        }
                        label { "Landscapes (optional, comma-separated)"
                            input { value: "{landscapes}", disabled: busy(), oninput: move |e| landscapes.set(e.value()) }
                        }
                    } else {
                        label { "Players"
                            select { value: "{players}", disabled: busy(), onchange: move |e| { if let Ok(n) = e.value().parse() { players.set(n); } },
                                option { value: "2", "2" }
                                option { value: "3", "3" }
                            }
                        }
                        label { "Minimum rating difference"
                            input { r#type: "number", min: "-100", max: "100", value: "{min_rating}", disabled: busy(), oninput: move |e| { if let Ok(n) = e.value().parse() { min_rating.set(n); } } }
                        }
                        label { "Maximum rating difference"
                            input { r#type: "number", min: "-100", max: "100", value: "{max_rating}", disabled: busy(), oninput: move |e| { if let Ok(n) = e.value().parse() { max_rating.set(n); } } }
                        }
                        label { "Timer"
                            select { value: "{timer}", disabled: busy(), onchange: move |e| { if let Ok(n) = e.value().parse() { timer.set(n); } },
                                option { value: "3", "Patient" }
                                option { value: "2", "Friendly" }
                                option { value: "1", "Hasty" }
                                option { value: "0", "Reckless" }
                            }
                        }
                    }
                    button {
                        class: "primary",
                        disabled: busy() || agent.read().is_empty() || !status.connected || status.agents.is_empty(),
                        onclick: move |_| execute.call(Action::Start),
                        match mode() {
                            OnlineMode::Bots => "Start unrated bot game",
                            OnlineMode::Rated => "Find rated human match",
                            OnlineMode::Unrated => "Find unrated human match",
                        }
                    }
                } else {
                    if let Some(playing) = &status.agent {
                        p { "Playing agent: {playing}" }
                        button { disabled: busy(), onclick: move |_| execute.call(Action::Pause), "Pause agent" }
                    } else if let Some(paused) = status.game_agent.clone() {
                        p { "{paused} is paused" }
                        button { disabled: busy(), onclick: move |_| execute.call(Action::Resume(paused.clone())), "Resume {paused}" }
                    } else {
                        p { "No agent is playing this game" }
                        button { disabled: busy() || agent.read().is_empty(), onclick: move |_| execute.call(Action::Attach), "Start the selected agent" }
                    }
                    if confirming() {
                        p { "Resign this official game?" }
                        button { disabled: busy(), onclick: move |_| execute.call(Action::Resign), "Confirm resign" }
                        button { onclick: move |_| confirming.set(false), "Keep playing" }
                    } else {
                        button { disabled: busy(), onclick: move |_| confirming.set(true), "Resign…" }
                    }
                }
                button { disabled: busy(), onclick: move |_| execute.call(Action::Reconnect), "Reconnect official account" }
                if busy() { p { class: "muted", "Updating…" } }
                if let Some(error) = &status.error { p { class: "error", "{error}" } }
            } else {
                p { class: "muted", "Asking the server…" }
            }
            if let Some(error) = action_error() { p { class: "error", "{error}" } }
        }
    }
}

/// The Account tab's box for each official account the player owns: its rating and
/// latest results on dominion.games.
#[component]
pub fn OfficialRatings() -> Element {
    let owned = use_owned();
    rsx! {
        for a in owned {
            Fieldset { key: "{a.id}", title: "dominion.games: {a.account}", class: "lb-official-rating",
                style { {CSS} }
                OfficialRating { account: a.id.clone(), running: a.game.as_ref().is_some_and(|g| g.running) }
            }
        }
    }
}

#[component]
fn OfficialRating(account: String, running: bool) -> Element {
    let id = use_signal(|| account.clone());
    let mut rating_type = use_signal(OnlineRatingType::default);
    let mut refresh = use_signal(|| 0_u32);
    let rating = use_resource(move || async move {
        refresh();
        api::online_rating(Some(&id()), rating_type())
            .await
            .map_err(|e| e.to_string())
    });
    // Asked again each minute, and twice once a game ends: at once, and once dominion.games
    // has had time to count it.
    let mut was_running = use_signal(|| running);
    if *was_running.peek() != running {
        was_running.set(running);
        if !running {
            refresh += 1;
            spawn(async move {
                platform::sleep(30_000).await;
                refresh += 1;
            });
        }
    }
    use_future(move || async move {
        loop {
            platform::sleep(60_000).await;
            refresh += 1;
        }
    });
    rsx! {
        div { class: "online-controls online-rating",
            label { "Rating"
                select {
                    value: "{rating_type.read().ordinal()}",
                    onchange: move |e| {
                        if let Some(kind) = e.value().parse().ok().and_then(OnlineRatingType::from_ordinal) {
                            rating_type.set(kind);
                        }
                    },
                    for kind in OnlineRatingType::ALL { option { value: "{kind.ordinal()}", "{kind.label()}" } }
                }
            }
            match rating.read().as_ref() {
                None => rsx! { p { class: "muted", "Asking dominion.games…" } },
                Some(Err(e)) => rsx! { p { class: "muted", "{e}" } },
                Some(Ok(r)) => rsx! { RatingSummary { rating: r.clone() } },
            }
        }
    }
}

#[component]
fn RatingSummary(rating: OnlineRating) -> Element {
    let r = &rating;
    let fetched = crate::lobby::date(r.fetched);
    rsx! {
        p {
            strong {"Level {r.level:.2}"}
            " · skill {r.skill:.2} ± {r.deviation:.2} · {r.games} games"
            if let Some(rank)=r.rank {" · #{rank}"}
        }
        if let Some(period)=&r.period {
            p {class:"muted",
                "This period, not yet final: {period.results} games, {period.score} points against {period.expected_score:.2} expected, from level {period.start_level:.2}"
            }
        }
        p {class:"muted",
            "{r.wins}–{r.losses}–{r.ties} · best level {r.best_level:.2}"
            if let Some(rank)=r.best_rank {" · best rank #{rank}"}
            " · longest streaks {r.best_win_streak} won, {r.best_loss_streak} lost"
        }
        if !r.results.is_empty() {
            details {class:"online-results",
                summary {"Recent games"}
                ol {
                    for game in &r.results {
                        li {key:"{game.game}",
                            strong {match game.outcome {OnlineOutcome::Win=>"W",OnlineOutcome::Loss=>"L",OnlineOutcome::Tie=>"T"}}
                            " {game.score}"
                            for o in &game.opponents {" · {o.name} {o.score}"}
                            if game.rated {" · {game.level:.2} vs {game.opponent_level:.2}"} else {" · unrated"}
                            if game.resigned || game.opponents.iter().any(|o| o.resigned) {" · resigned"}
                            span {class:"muted"," · {crate::lobby::date(game.time.max(0) as u64)}"}
                            if !game.kingdom.is_empty() {
                                br {}
                                span {class:"muted","{game.kingdom.join(\", \")}"}
                            }
                        }
                    }
                }
            }
        }
        if !fetched.is_empty() {p {class:"muted","From dominion.games, {fetched}"}}
    }
}
