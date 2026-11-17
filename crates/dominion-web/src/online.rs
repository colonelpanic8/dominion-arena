use crate::lobby::agent_picker::AgentPicker;
use crate::{
    api,
    game::{GameCtx, GameState, table},
    platform,
};
use dioxus::prelude::*;
use dominion_protocol::api::{
    GameSummary, OnlineAccountInfo, OnlineManagement, OnlineMatching, OnlineMode,
    OnlineObservation, OnlineOutcome, OnlineRating, OnlineRatingType, OnlineStart,
};
#[derive(Clone, Copy)]
enum Action {
    Start,
    Attach,
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
/// One official account's page: `account` none is the primary one's unprefixed routes.
#[component]
pub fn OnlinePage(account: Option<String>) -> Element {
    let id = use_signal(|| account.clone());
    let mut accounts = use_signal(Vec::<OnlineAccountInfo>::new);
    use_future(move || async move {
        if let Ok(list) = api::online_accounts().await {
            accounts.set(list);
        }
    });
    let mut state = use_signal(GameState::default);
    let game = use_signal(String::new);
    let summary = use_signal(|| None::<GameSummary>);
    use_context_provider(|| summary);
    let seat = use_signal(|| None);
    let watching = use_memo(|| true);
    let deciding = use_memo(|| None);
    let resigning = use_signal(|| false);
    use_context_provider(|| GameCtx {
        game,
        seat,
        watching,
        state,
        deciding,
        submit: Callback::new(|_| {}),
        undo: Callback::new(|_| {}),
        answer_undo: Callback::new(|_| {}),
        cancel_undo: Callback::new(|_| {}),
        resign: Callback::new(|_| {}),
        resigning,
    });
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
    let mut rating_type = use_signal(OnlineRatingType::default);
    let mut refresh = use_signal(|| 0_u32);
    let rating = use_resource(move || async move {
        refresh();
        api::online_rating(id().as_deref(), rating_type())
            .await
            .map_err(|e| e.to_string())
    });
    // Asked again each minute, and twice once a game ends: at once, and once dominion.games
    // has had time to count it.
    use_future(move || async move {
        let mut ended = None::<String>;
        let mut waited = 0;
        loop {
            platform::sleep(5000).await;
            waited += 5000;
            let finished = (state.peek().view.as_ref())
                .filter(|v| v.game.result.is_some())
                .map(|v| v.game.id.clone().unwrap_or_default());
            if finished.is_some() && finished != ended {
                ended = finished;
                waited = 35_000;
                refresh += 1;
            } else if waited >= 60_000 {
                waited = 0;
                refresh += 1;
            }
        }
    });
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
            match api::online_management(id.peek().as_deref()).await {
                Ok(status) => {
                    let first = (status.models.first())
                        .and_then(|m| m.default_choice())
                        .map(|c| &c.agent)
                        .or(status.agents.first());
                    if agent.peek().is_empty()
                        && let Some(first) = first
                    {
                        agent.set(first.clone());
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
        let request = OnlineStart {
            agent: selected.clone(),
            mode: *mode.peek(),
            matching: OnlineMatching {
                players: *players.peek(),
                min_rating: *min_rating.peek(),
                max_rating: *max_rating.peek(),
                timer: *timer.peek(),
            },
            kingdom: if *mode.peek() == OnlineMode::Bots {
                card_names(&kingdom.peek())
            } else {
                vec![]
            },
            landscapes: if *mode.peek() == OnlineMode::Bots {
                card_names(&landscapes.peek())
            } else {
                vec![]
            },
            bots: *bots.peek(),
        };
        let account = id.peek().clone();
        spawn(async move {
            let account = account.as_deref();
            let result = match action {
                Action::Start => api::online_start(account, &request).await,
                Action::Attach => api::online_attach(account, selected).await,
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
    let controls = rsx! {
        div {class:"panel online-controls",
            strong {"dominion.games"}
            if accounts.read().len() > 1 {
                nav {class:"online-accounts",
                    for a in accounts.read().iter() {
                        Link {
                            class: if id.read().as_deref().map_or(a.primary, |id| id == a.id) {"on"} else {""},
                            to: if a.primary {crate::app::Route::Online{}} else {crate::app::Route::OnlineAccount{account:a.id.clone()}},
                            "{a.account}"
                            if a.running {" · playing"} else if !a.connected {" · disconnected"}
                        }
                    }
                }
            }
            Link {to:crate::app::Route::Account{},"Arena login / account"}
            div {class:"online-rating",
                label {"Rating"
                    select {value:"{rating_type.read().ordinal()}",onchange:move|e|{if let Some(kind)=e.value().parse().ok().and_then(OnlineRatingType::from_ordinal){rating_type.set(kind);}},
                        for kind in OnlineRatingType::ALL {option {value:"{kind.ordinal()}","{kind.label()}"}}
                    }
                }
                match rating.read().as_ref() {
                    None => rsx! {p {class:"muted","Asking dominion.games…"}},
                    Some(Err(e)) => rsx! {p {class:"muted","{e}"}},
                    Some(Ok(r)) => rsx! {RatingSummary {rating: r.clone()}},
                }
            }
            if let Some(status)=management.read().as_ref() {
                p {class:"muted",
                    if status.connected {"Official account connected"}else{"Official account disconnected"}
                    if let Some(account)=&status.account {" · {account}"}
                    " · Your account’s view"
                }
                if status.running || status.searching {
                    p {class:"muted",match status.mode {
                        OnlineMode::Bots=>"Unrated server bots",
                        OnlineMode::Unrated=>"Unrated public humans",
                        OnlineMode::Rated=>"Rated public humans",
                    }}
                }
                label {"Agent"
                    if status.models.is_empty() {
                        select {value:"{agent}",disabled:busy(),onchange:move|e|agent.set(e.value()),
                            for name in &status.agents {option {value:"{name}","{name}"}}
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
                if status.agents.is_empty(){p {class:"muted","The agent runner is unavailable. No game will be started without an agent."}}
                for feedback in &status.queue_feedback {p {class:"muted","{feedback}"}}
                if status.searching {
                    p {"Searching for a public match… The selected agent starts when a match is found."}
                    button {disabled:busy(),onclick:move|_|execute.call(Action::Cancel),"Cancel matchmaking"}
                } else if !status.running {
                    label {"Game type"
                        select {value: match mode() {OnlineMode::Bots=>"bots",OnlineMode::Unrated=>"unrated",OnlineMode::Rated=>"rated"}, disabled:busy(), onchange:move|e|mode.set(match e.value().as_str(){"bots"=>OnlineMode::Bots,"unrated"=>OnlineMode::Unrated,_=>OnlineMode::Rated}),
                            option {value:"rated","Public humans · rated"}
                            option {value:"unrated","Public humans · unrated"}
                            option {value:"bots","Server bots · unrated"}
                        }
                    }
                    if mode() == OnlineMode::Bots {
                    label {"Server bots"
                        input {r#type:"number",min:"1",max:"5",value:"{bots}",disabled:busy(),oninput:move|e|{if let Ok(n)=e.value().parse(){bots.set(n);}}}
                    }
                    label {"Kingdom (optional, comma-separated)"
                        textarea {value:"{kingdom}",disabled:busy(),oninput:move|e|kingdom.set(e.value())}
                    }
                    label {"Landscapes (optional, comma-separated)"
                        input {value:"{landscapes}",disabled:busy(),oninput:move|e|landscapes.set(e.value())}
                    }
                    } else {
                        label {"Players"
                            select {value:"{players}",disabled:busy(),onchange:move|e|{if let Ok(n)=e.value().parse(){players.set(n);}},
                                option {value:"2","2"} option {value:"3","3"}
                            }
                        }
                        label {"Minimum rating difference"
                            input {r#type:"number",min:"-100",max:"100",value:"{min_rating}",disabled:busy(),oninput:move|e|{if let Ok(n)=e.value().parse(){min_rating.set(n);}}}
                        }
                        label {"Maximum rating difference"
                            input {r#type:"number",min:"-100",max:"100",value:"{max_rating}",disabled:busy(),oninput:move|e|{if let Ok(n)=e.value().parse(){max_rating.set(n);}}}
                        }
                        label {"Timer"
                            select {value:"{timer}",disabled:busy(),onchange:move|e|{if let Ok(n)=e.value().parse(){timer.set(n);}},
                                option {value:"3","Patient"} option {value:"2","Friendly"}
                                option {value:"1","Hasty"} option {value:"0","Reckless"}
                            }
                        }
                    }
                    button {class:"primary",disabled:busy() || agent.read().is_empty() || !status.connected || status.agents.is_empty(),onclick:move|_|execute.call(Action::Start),
                        if mode() == OnlineMode::Bots {"Start unrated bot game"} else if mode() == OnlineMode::Rated {"Find rated human match"} else {"Find unrated human match"}
                    }
                } else {
                    if let Some(playing)=&status.agent {
                        p {"Playing agent: {playing}"}
                        button {disabled:busy(),onclick:move|_|execute.call(Action::Pause),"Pause agent"}
                    } else {
                        p {"Agent paused"}
                        button {disabled:busy() || agent.read().is_empty(),onclick:move|_|execute.call(Action::Attach),"Resume with selected agent"}
                    }
                    if confirming() {
                        p {"Resign this official game?"}
                        button {disabled:busy(),onclick:move|_|execute.call(Action::Resign),"Confirm resign"}
                        button {onclick:move|_|confirming.set(false),"Keep playing"}
                    } else {
                        button {disabled:busy(),onclick:move|_|confirming.set(true),"Resign…"}
                    }
                }
                button {disabled:busy(),onclick:move|_|execute.call(Action::Reconnect),"Reconnect official account"}
                if busy(){p {class:"muted","Updating…"}}
                if let Some(error)=&status.error {p {class:"error","{error}"}}
            } else {p {"Sign in as this official account’s Arena owner to manage its games."}}
            if let Some(error)=action_error(){p {class:"error","{error}"}}
            if let Some(error)=state.read().error.clone() {p {class:"error","{error}"}}
        }
    };
    rsx! {
        style { {include_str!("../assets/online.css")} }
        if state.read().view.as_ref().is_some_and(|v| v.game.running) {
            table::TableGame {controls, arena: false}
        } else {
            section {class:"online-idle",
                h1 {class:"page-title","dominion.games"}
                p {class:"page-lede","Send one of the Arena's agents to play on the official server under this official account. Its game shows here on the table."}
                {controls}
                if let Some(result) = state.read().view.as_ref().and_then(|v| v.game.result.as_ref()) {
                    div {class:"panel",
                        h2 {"Last game"}
                        for player in &result.players {
                            p {"{player.rank}. {player.name} · {player.vp} VP"}
                        }
                    }
                }
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
