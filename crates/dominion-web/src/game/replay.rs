//! A finished game replayed on the table: any point of it, from its first decision to its
//! end, as someone watching saw it (every player's cards, or what one seat knew).

use super::{GameState, logfmt, server_ahead, table};
use crate::api;
use crate::app::Route;
use crate::platform;
use dioxus::prelude::*;
use dominion_protocol::api::{GameSummary, Replay, SeatView};

/// How long autoplay shows each step, in milliseconds.
const AUTOPLAY_MS: u32 = 800;

/// The step to go to for a move of the replay's controls.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Go {
    Start,
    PrevTurn,
    Prev,
    Next,
    NextTurn,
    End,
}

/// The step `go` leads to from `step`.
fn target(r: &Replay, step: usize, go: Go) -> usize {
    match go {
        Go::Start => 0,
        Go::Prev => step.saturating_sub(1),
        Go::Next => (step + 1).min(r.steps),
        Go::End => r.steps,
        // The start of this turn, or of the one before when already there.
        Go::PrevTurn => (r.turns.iter().rev())
            .map(|t| t.step)
            .find(|&s| s < step)
            .unwrap_or(0),
        Go::NextTurn => (r.turns.iter())
            .map(|t| t.step)
            .find(|&s| s > step)
            .unwrap_or(r.steps),
    }
}

/// Takes in the view at another point: a step forward adds to the log held, any other
/// move replaces it.
fn take(mut state: Signal<GameState>, sv: SeatView, onward: bool) {
    table::before_change();
    let mut st = state.write();
    let ahead = server_ahead(sv.meta.now, st.server_ahead);
    let first = sv.view.since_last_decision.first().map(|e| e.seq);
    if !(onward && first.is_none_or(|seq| seq == st.log_from())) {
        let jumps = st.jumps + 1;
        *st = GameState {
            jumps,
            ..GameState::default()
        };
    }
    st.server_ahead = ahead;
    st.apply(sv);
}

#[component]
pub fn ReplayPage(id: String) -> Element {
    let game = use_signal(|| id.clone());
    let state = use_signal(GameState::default);
    let mut step = use_signal(|| 0usize);
    let mut watch_as = use_signal(|| None::<u8>);
    let mut replay = use_signal(|| None::<Replay>);
    let mut playing = use_signal(|| false);
    let mut failed = use_signal(|| None::<String>);
    // What is shown: the step and seat of the last view taken in.
    let mut shown = use_signal(|| None::<(usize, Option<u8>)>);
    let summary = use_signal(|| None::<GameSummary>);
    use_context_provider(|| summary);
    super::use_watched_game(game, state, use_memo(super::Offers::default));

    // Fetches the view whenever the step or seat wanted differs from the one shown.
    use_effect(move || {
        let want = (step(), watch_as());
        if *shown.peek() == Some(want) {
            return;
        }
        let id = game.peek().clone();
        spawn(async move {
            let onward = shown
                .peek()
                .is_some_and(|(s, a)| a == want.1 && s + 1 == want.0);
            let from = if onward { state.peek().log_from() } else { 0 };
            let result = api::replay_view(&id, want.0, from, want.1).await;
            if (step(), watch_as()) != want {
                return;
            }
            match result {
                Ok(sv) => {
                    replay.set(sv.replay.clone());
                    take(state, sv, onward);
                    shown.set(Some(want));
                    failed.set(None);
                }
                Err(e) => {
                    playing.set(false);
                    failed.set(Some(e.message));
                }
            }
        });
    });

    use_future(move || async move {
        loop {
            platform::sleep(AUTOPLAY_MS).await;
            if !*playing.peek() || *shown.peek() != Some((*step.peek(), *watch_as.peek())) {
                continue;
            }
            match replay.peek().as_ref() {
                Some(r) if *step.peek() < r.steps => step += 1,
                _ => playing.set(false),
            }
        }
    });

    if let Some(e) = failed()
        && state.read().view.is_none()
    {
        return rsx! {
            div { class: "game-page",
                section { class: "panel", "data-replay-error": "1",
                    h2 { "This game can't be replayed" }
                    p { "The server said: {e}." }
                    Link { class: "button primary", to: Route::Game { id: id.clone() }, "Open the game" }
                    " "
                    Link { class: "button", to: Route::Lobby {}, "Back to the lobby" }
                }
            }
        };
    }

    let names = state
        .read()
        .view
        .as_ref()
        .map(logfmt::names)
        .unwrap_or_default();
    let r = replay();
    let now = step();
    let mut go = move |to: Go| {
        if let Some(r) = replay.peek().as_ref() {
            playing.set(false);
            let to = target(r, *step.peek(), to);
            step.set(to);
        }
    };
    let turn_line = r.as_ref().and_then(|r| {
        let t = r.turns.iter().rev().find(|t| t.step <= now)?;
        let name = names
            .get(usize::from(t.player))
            .cloned()
            .unwrap_or_default();
        Some(format!("Turn {}: {name}", t.turn))
    });
    let steps = r.as_ref().map_or(0, |r| r.steps);
    let controls = rsx! {
        div { class: "seat-bar replay-bar", "data-replay-step": "{now}",
            span { class: "game-title", "Replay of game {game}" }
            Link { class: "replay-open", to: Route::Game { id: id.clone() }, "open game" }
        }
        div { class: "seat-bar replay-bar",
            button { class: "toggle", title: "First decision", "data-replay": "start", disabled: now == 0, onclick: move |_| go(Go::Start), "⏮" }
            button { class: "toggle", title: "Previous turn", "data-replay": "prev-turn", disabled: now == 0, onclick: move |_| go(Go::PrevTurn), "«" }
            button { class: "toggle", title: "Previous step", "data-replay": "prev", disabled: now == 0, onclick: move |_| go(Go::Prev), "‹" }
            button {
                class: if playing() { "toggle on" } else { "toggle" },
                title: if playing() { "Pause" } else { "Play" },
                "data-replay": "play",
                disabled: r.is_none() || (now >= steps && !playing()),
                onclick: move |_| playing.toggle(),
                if playing() { "❚❚" } else { "▶" }
            }
            button { class: "toggle", title: "Next step", "data-replay": "next", disabled: now >= steps, onclick: move |_| go(Go::Next), "›" }
            button { class: "toggle", title: "Next turn", "data-replay": "next-turn", disabled: now >= steps, onclick: move |_| go(Go::NextTurn), "»" }
            button { class: "toggle", title: "The end", "data-replay": "end", disabled: now >= steps, onclick: move |_| go(Go::End), "⏭" }
        }
        if r.is_some() {
            div { class: "seat-bar replay-bar",
                input {
                    class: "replay-slider",
                    r#type: "range",
                    min: "0",
                    max: "{steps}",
                    value: "{now}",
                    oninput: move |e| {
                        if let Ok(n) = e.value().parse::<usize>() {
                            playing.set(false);
                            step.set(n.min(steps));
                        }
                    },
                }
                span { class: "muted", "data-replay-at": "{now}/{steps}", "step {now} of {steps}" }
                if let Some(line) = turn_line {
                    span { "{line}" }
                }
            }
        }
        div { class: "seat-bar replay-bar watch-as", "data-watch-as": "{watch_as().map(i32::from).unwrap_or(-1)}",
            span { class: "muted", "as:" }
            button {
                class: if watch_as().is_none() { "toggle on" } else { "toggle" },
                title: "Every player's cards",
                onclick: move |_| watch_as.set(None),
                "Everyone"
            }
            for (i, name) in names.iter().cloned().enumerate() {
                {
                    let i = i as u8;
                    rsx! {
                        button {
                            key: "{i}",
                            class: if watch_as() == Some(i) { "toggle on" } else { "toggle" },
                            title: "What {name} knew",
                            onclick: move |_| watch_as.set(Some(i)),
                            "{name}"
                        }
                    }
                }
            }
        }
        if let Some(e) = failed() {
            p { class: "error", "{e}" }
        }
    };
    rsx! {
        table::TableGame { controls }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dominion_protocol::api::ReplayTurn;

    #[test]
    fn turn_moves_go_to_turn_starts() {
        let turn = |step, turn| ReplayTurn {
            step,
            player: 0,
            turn,
        };
        let r = Replay {
            step: 0,
            steps: 30,
            turns: vec![turn(0, 1), turn(10, 1), turn(20, 2)],
        };
        assert_eq!(target(&r, 15, Go::PrevTurn), 10);
        assert_eq!(target(&r, 10, Go::PrevTurn), 0);
        assert_eq!(target(&r, 0, Go::PrevTurn), 0);
        assert_eq!(target(&r, 10, Go::NextTurn), 20);
        assert_eq!(target(&r, 25, Go::NextTurn), 30);
        assert_eq!(target(&r, 30, Go::Next), 30);
    }
}
