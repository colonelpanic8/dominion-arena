//! dominion.games' sounds from the asset cache, when it plays them: "your turn" when a
//! decision comes back to this seat after others decided and no click came in the last
//! second, a ping when the game ends and then the win or lose sound on the score
//! screen. Card actions and shuffles are silent. Nothing plays (and no audio element is
//! made) when `web_sounds` is off, the page is muted, or the cache has no sounds;
//! browsers allow sound only after the user has interacted with the page, so until then
//! nothing is tried.

use crate::app::Settings;
use crate::assets::{AssetDb, Assets, use_assets};
use crate::game::GameCtx;
use crate::game::board::Board;
use crate::platform;
use dioxus::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;

const SOUND_JS: &str = r#"
if (!window.__dtDown) {
  window.__dtDown = 0;
  document.addEventListener('mousedown', () => { window.__dtDown = Date.now(); }, true);
}
window.__dtSound = window.__dtSound || {
  cache: {},
  play(url, volume) {
    const ua = navigator.userActivation;
    if (ua && !ua.hasBeenActive) return;
    let a = this.cache[url];
    if (!a) a = this.cache[url] = new Audio(url);
    a.volume = volume;
    try { a.currentTime = 0; } catch (_) {}
    const p = a.play();
    if (p && p.catch) p.catch(() => {});
  },
};
"#;

const MUTE_KEY: &str = "dominion.mute";

/// The cached file of a sound, preferring Ogg.
fn sound_url(assets: &AssetDb, name: &str) -> Option<String> {
    ["ogg", "mp3"]
        .iter()
        .find_map(|ext| assets.file_url(&format!("sounds/{name}.{ext}")))
}

/// Whether the page has sounds to play at all.
pub fn available(assets: &AssetDb, settings: &dominion_protocol::api::WebSettings) -> bool {
    settings.web_sounds && sound_url(assets, "your-turn").is_some()
}

/// What happened between two boards that makes a sound, and the sound for the score
/// screen when the game ended.
fn sound_for(old: &Board, new: &Board) -> Option<(&'static str, Option<&'static str>)> {
    let seat = new.you.as_ref().map(|y| y.seat)?;
    if old.result.is_none()
        && let Some(r) = &new.result
    {
        let me = r.iter().find(|p| p.index == seat)?;
        let after = if me.rank == 1 && !me.resigned {
            "win"
        } else {
            "game-over"
        };
        return Some(("ping", Some(after)));
    }
    (new.status.you_decide && !old.status.you_decide).then_some(("your-turn", None))
}

#[derive(Default)]
struct Last {
    board: Option<Rc<Board>>,
    seat: Option<u8>,
}

pub fn use_sound(board: Memo<Option<Rc<Board>>>, game: GameCtx, muted: Signal<bool>) {
    let settings = use_context::<Settings>().0;
    let assets = use_context::<Assets>().0;
    let last = use_hook(|| Rc::new(RefCell::new(Last::default())));
    use_effect(move || {
        let b = board.read().clone();
        let st = game.state.peek();
        let seat = st.view.as_ref().and_then(|v| v.seat);
        let mut l = last.borrow_mut();
        let s = settings.peek();
        let a = assets.peek();
        if let (Some(old), Some(new)) = (&l.board, &b)
            && l.seat == seat
            && !*muted.peek()
            && available(&a, &s)
            && let Some((name, after)) = sound_for(old, new)
            && let Some(url) = sound_url(&a, name)
        {
            platform::run_js(SOUND_JS);
            let volume = s.web_volume.clamp(0.0, 1.0);
            let quiet = if name == "your-turn" {
                "Date.now() - window.__dtDown < 1000"
            } else {
                "false"
            };
            platform::run_js(&format!(
                "if (!({quiet})) window.__dtSound.play({}, {volume})",
                serde_json::Value::String(url),
            ));
            if let Some(url) = after.and_then(|n| sound_url(&a, n)) {
                platform::run_js(&format!(
                    "window.__dtResultSound = () => window.__dtSound.play({}, {volume})",
                    serde_json::Value::String(url),
                ));
            }
        }
        l.board = b;
        l.seat = seat;
    });
}

/// Whether this browser muted the table for the session.
pub fn session_muted() -> bool {
    platform::session_get(MUTE_KEY).as_deref() == Some("1")
}

/// The page's mute toggle, a session-only override of `web_sounds`.
#[component]
pub fn MuteToggle(muted: Signal<bool>) -> Element {
    let settings = use_context::<Settings>().0;
    let assets = use_assets();
    if !available(&assets, &settings.read()) {
        return rsx! {};
    }
    let on = !muted();
    rsx! {
        button {
            class: if on { "toggle on" } else { "toggle" },
            "data-action": "mute",
            title: if on { "Mute for this session" } else { "Unmute" },
            onclick: move |_| {
                let m = !muted();
                platform::session_set(MUTE_KEY, if m { "1" } else { "0" });
                muted.set(m);
            },
            if on { "Sound on" } else { "Muted" }
        }
    }
}
