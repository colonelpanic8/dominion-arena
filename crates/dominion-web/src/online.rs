use crate::{
    api,
    game::{GameCtx, GameState, table},
    platform,
};
use dioxus::prelude::*;
use dominion_protocol::api::OnlineObservation;

fn apply(mut state: Signal<GameState>, mut game: Signal<String>, update: OnlineObservation) {
    table::before_change();
    let mut st = state.write();
    st.error = update
        .error
        .or_else(|| (!update.connected).then(|| "Official connection is disconnected".into()));
    st.version = update.version;
    st.undos = update.epoch;
    st.deciding = None;
    st.can_undo = false;
    if let Some(view) = &update.view {
        st.log = view.since_last_decision.clone();
        game.set(view.game.id.clone().unwrap_or_default());
    }
    st.view = update.view;
}
#[component]
pub fn OnlinePage() -> Element {
    let mut state = use_signal(GameState::default);
    let game = use_signal(String::new);
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
        resign: Callback::new(|_| {}),
        resigning,
    });
    use_future(move || async move {
        loop {
            match api::online_view().await {
                Ok(view) => {
                    apply(state, game, view);
                    api::follow_online(&mut |view| apply(state, game, view)).await;
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
    let controls = rsx! {
        div { class:"row",
            strong {"dominion.games · Live observer"}
            span {class:"muted","Connected account’s view · Read only"}
            Link {to:crate::app::Route::Account{},"Account"}
        }
        if let Some(error)=state.read().error.clone() {p {class:"error","{error}"}}
    };
    if state.read().view.is_some() {
        rsx! {table::TableGame {controls}}
    } else {
        rsx! {section {class:"panel",{controls} p {"No official match is running. Start a game with your connected agent."}}}
    }
}
