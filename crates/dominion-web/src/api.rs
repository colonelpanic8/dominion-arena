//! A typed client for `dominion serve`'s HTTP API, on the page's own origin.

use crate::platform::{self, encode};
use dominion_protocol::Answer;
use dominion_protocol::api::{
    AnswerRequest, CardInfo, Change, Created, DealRequest, Dealt, GameSummary, NewGame, SeatInfo,
    SeatView, WebSettings,
};
use dominion_protocol::lobby::{
    BotKey, ChatLog, Credentials, GuestRequest, HeadToHead, HeadToHeadQuery, History, HistoryQuery,
    Invite, JoinTable, Leaderboard, Lobby, LobbyChange, MatchRequest, Me, MyGame, NewBot,
    NewInvite, NewPassword, NewTable, OFFICIAL_GAME_PREFIX, Profile, QueueStatus, Ready, Say,
    SeatEdit, TableInfo, TableSpec, Venue,
};
use std::fmt;

/// A failed call: the HTTP status (0 when the server couldn't be reached or answered
/// with something unreadable) and the server's message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ApiError {
    pub status: u16,
    pub message: String,
}

impl ApiError {
    /// The answer named a decision that is no longer open, or it isn't this seat's.
    pub fn is_stale(&self) -> bool {
        self.status == 409
    }

    /// Nobody is signed in (or the seat token is wrong).
    pub fn is_unauthorized(&self) -> bool {
        self.status == 401
    }
}

impl fmt::Display for ApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.status == 0 {
            write!(f, "{}", self.message)
        } else {
            write!(f, "{} (HTTP {})", self.message, self.status)
        }
    }
}

pub type Result<T> = std::result::Result<T, ApiError>;

pub async fn cards() -> Result<Vec<CardInfo>> {
    imp::get("/api/cards", None).await
}

pub async fn settings() -> Result<WebSettings> {
    imp::get("/api/settings", None).await
}

pub async fn games() -> Result<Vec<GameSummary>> {
    imp::get("/api/games", None).await
}

pub async fn create_game(req: &NewGame) -> Result<Created> {
    imp::post("/api/games", None, req).await
}

/// The kingdom a table's rules deal, without creating a game.
pub async fn deal(req: &DealRequest) -> Result<Dealt> {
    imp::post("/api/deal", None, req).await
}

/// Replaces your Banned cards list (signed in).
pub async fn set_banned_cards(cards: &[String]) -> Result<WebSettings> {
    imp::send("PUT", "/api/settings/banned-cards", None, Some(&cards)).await
}

pub async fn seat(token: &str) -> Result<SeatInfo> {
    imp::get("/api/seat", Some(token)).await
}

/// A seat's view; with `log_from` its log starts at that sequence number.
pub async fn view(game: &str, token: &str, log_from: Option<usize>) -> Result<SeatView> {
    let mut url = format!("/api/games/{}/view", encode(game));
    if let Some(n) = log_from {
        url.push_str(&format!("?logFrom={n}"));
    }
    imp::get(&url, Some(token)).await
}

/// Sends an answer; with `way` (`none` or a Way's name) the Way question of the card it
/// plays is answered with it.
pub async fn answer(
    game: &str,
    token: &str,
    answer: &Answer,
    way: Option<String>,
) -> Result<SeatView> {
    let url = format!("/api/games/{}/answer", encode(game));
    let body = AnswerRequest {
        answer: answer.clone(),
        way,
    };
    imp::post(&url, Some(token), &body).await
}

/// Asks to take back the seat's last answer; the view returned has the whole log.
pub async fn undo(game: &str, token: &str) -> Result<SeatView> {
    let url = format!("/api/games/{}/undo", encode(game));
    imp::send("POST", &url, Some(token), None::<&()>).await
}

/// Allows or refuses the undo waiting for the seat.
pub async fn answer_undo(game: &str, token: &str, allow: bool) -> Result<SeatView> {
    let verb = if allow { "allow" } else { "refuse" };
    let url = format!("/api/games/{}/undo/{verb}", encode(game));
    imp::send("POST", &url, Some(token), None::<&()>).await
}

/// Withdraws the seat's undo request.
pub async fn cancel_undo(game: &str, token: &str) -> Result<SeatView> {
    let url = format!("/api/games/{}/undo/cancel", encode(game));
    imp::send("POST", &url, Some(token), None::<&()>).await
}

/// Follows a change feed until the future is dropped, calling `on_change` for every
/// `change` event (one on each connect). Reconnects with backoff.
async fn follow<T: serde::de::DeserializeOwned>(url: &str, mut on_change: impl FnMut(T)) {
    let mut delay = 500;
    loop {
        if imp::follow(url, &mut |data| {
            if let Ok(change) = serde_json::from_str(data) {
                delay = 500;
                on_change(change)
            }
        })
        .await
        .is_err()
        {
            platform::sleep(delay).await;
            delay = (delay * 2).min(30_000);
        }
    }
}

/// Follows a game's change feed for a seat.
pub async fn watch(game: &str, token: &str, on_change: impl FnMut(Change)) {
    let url = format!("/api/games/{}/events?token={}", encode(game), encode(token));
    follow(&url, on_change).await
}

/// Follows a game's change feed as someone watching.
pub async fn watch_unseated(game: &str, on_change: impl FnMut(Change)) {
    follow(&format!("/api/games/{}/events", encode(game)), on_change).await
}

/// The game as someone watching sees it (the game must be watchable): every player's
/// cards, or with `as_seat` what that seat knows.
pub async fn watch_view(
    game: &str,
    log_from: Option<usize>,
    as_seat: Option<u8>,
) -> Result<SeatView> {
    let mut query = vec![];
    if let Some(n) = log_from {
        query.push(format!("logFrom={n}"));
    }
    if let Some(seat) = as_seat {
        query.push(format!("as={seat}"));
    }
    let mut url = format!("/api/games/{}/watch", encode(game));
    if !query.is_empty() {
        url.push_str(&format!("?{}", query.join("&")));
    }
    imp::get(&url, None).await
}

/// Finished game `game` as someone watching saw it after `step` answers, with the log
/// from `log_from` on. A dominion.games game is seen as the account that recorded it saw
/// it, at each step of its recording.
pub async fn replay_view(
    game: &str,
    step: usize,
    log_from: usize,
    as_seat: Option<u8>,
) -> Result<SeatView> {
    if game.starts_with(OFFICIAL_GAME_PREFIX) {
        let url = format!(
            "/api/online/replays/{}?step={step}&logFrom={log_from}",
            encode(game)
        );
        return imp::get(&url, None).await;
    }
    let mut url = format!(
        "/api/games/{}/watch?step={step}&logFrom={log_from}",
        encode(game)
    );
    if let Some(seat) = as_seat {
        url.push_str(&format!("&as={seat}"));
    }
    imp::get(&url, None).await
}

pub async fn resign(game: &str, token: &str) -> Result<SeatView> {
    let url = format!("/api/games/{}/resign", encode(game));
    imp::send("POST", &url, Some(token), None::<&()>).await
}

// Accounts. A session is a cookie the browser keeps and sends by itself.

pub async fn me() -> Result<Me> {
    imp::get("/api/me", None).await
}

/// With `invite`, an admin's invite code, used up by the new account.
pub async fn register(name: &str, password: &str, invite: Option<&str>) -> Result<Me> {
    let credentials = Credentials {
        name: name.into(),
        password: password.into(),
        invite: invite.map(String::from),
    };
    imp::post("/api/auth/register", None, &credentials).await
}

pub async fn login(name: &str, password: &str) -> Result<Me> {
    let credentials = Credentials {
        name: name.into(),
        password: password.into(),
        invite: None,
    };
    imp::post("/api/auth/login", None, &credentials).await
}

/// A guest account under `name`, or under a name the server makes up.
pub async fn guest(name: Option<&str>) -> Result<Me> {
    let request = GuestRequest {
        name: name.map(String::from),
    };
    imp::post("/api/auth/guest", None, &request).await
}

pub async fn logout() -> Result<()> {
    imp::send_empty("POST", "/api/auth/logout", None, None::<&()>).await
}

pub async fn set_password(current: &str, new: &str) -> Result<()> {
    let passwords = NewPassword {
        current: current.into(),
        new: new.into(),
    };
    imp::send_empty("PUT", "/api/me/password", None, Some(&passwords)).await
}

/// Makes the picture in `file` the account's: the middle square of it, made small.
pub async fn set_avatar(file: &dioxus::html::FileData) -> Result<Me> {
    imp::upload_avatar(file).await
}

pub async fn clear_avatar() -> Result<Me> {
    imp::send("DELETE", "/api/me/avatar", None, None::<&()>).await
}

/// The seats this account plays, with their tokens.
pub async fn my_games() -> Result<Vec<MyGame>> {
    imp::get("/api/me/games", None).await
}

pub async fn new_bot(name: &str) -> Result<BotKey> {
    imp::post("/api/me/bots", None, &NewBot { name: name.into() }).await
}

pub async fn new_bot_key(name: &str) -> Result<BotKey> {
    let url = format!("/api/me/bots/{}/key", encode(name));
    imp::send("POST", &url, None, None::<&()>).await
}

pub async fn delete_bot(name: &str) -> Result<()> {
    let url = format!("/api/me/bots/{}", encode(name));
    imp::send_empty("DELETE", &url, None, None::<&()>).await
}

// Invite codes, for admins.

pub async fn invites() -> Result<Vec<Invite>> {
    imp::get("/api/invites", None).await
}

pub async fn new_invite(new: &NewInvite) -> Result<Invite> {
    imp::post("/api/invites", None, new).await
}

pub async fn revoke_invite(code: &str) -> Result<()> {
    let url = format!("/api/invites/{}", encode(code));
    imp::send_empty("DELETE", &url, None, None::<&()>).await
}

// The lobby.

pub async fn lobby() -> Result<Lobby> {
    imp::get("/api/lobby", None).await
}

/// Follows the lobby's change feed; while it is open this account counts as connected.
pub async fn watch_lobby(on_change: impl FnMut(LobbyChange)) {
    follow("/api/lobby/events", on_change).await
}

pub async fn new_table(table: &NewTable) -> Result<TableInfo> {
    imp::post("/api/tables", None, table).await
}

pub async fn table(id: &str) -> Result<TableInfo> {
    imp::get(&format!("/api/tables/{}", encode(id)), None).await
}

pub async fn set_table_spec(id: &str, spec: &TableSpec) -> Result<TableInfo> {
    let url = format!("/api/tables/{}/spec", encode(id));
    imp::send("PUT", &url, None, Some(spec)).await
}

pub async fn set_table_seats(id: &str, seats: &[SeatEdit]) -> Result<TableInfo> {
    let url = format!("/api/tables/{}/seats", encode(id));
    imp::send("PUT", &url, None, Some(&seats)).await
}

pub async fn join_table(id: &str, seat: Option<u8>) -> Result<TableInfo> {
    let url = format!("/api/tables/{}/join", encode(id));
    imp::post(&url, None, &JoinTable { seat }).await
}

pub async fn leave_table(id: &str) -> Result<()> {
    let url = format!("/api/tables/{}/leave", encode(id));
    imp::send_empty("POST", &url, None, None::<&()>).await
}

/// Says whether you are ready; the table's `game` is set once that has started.
pub async fn ready(id: &str, ready: bool) -> Result<TableInfo> {
    let url = format!("/api/tables/{}/ready", encode(id));
    imp::post(&url, None, &Ready { ready }).await
}

pub async fn search(request: &MatchRequest) -> Result<QueueStatus> {
    imp::post("/api/automatch", None, request).await
}

pub async fn cancel_search() -> Result<()> {
    imp::send_empty("DELETE", "/api/automatch", None, None::<&()>).await
}

/// A chat channel's messages numbered after `after`: `lobby`, `table-<id>` or
/// `game-<id>` (with `token`, as that seat; else as the signed-in account).
pub async fn chat(channel: &str, after: u64, token: Option<&str>) -> Result<ChatLog> {
    let url = format!("/api/chat/{}?after={after}", encode(channel));
    imp::get(&url, token).await
}

pub async fn say(channel: &str, text: &str, token: Option<&str>) -> Result<()> {
    let url = format!("/api/chat/{}", encode(channel));
    imp::send_empty("POST", &url, token, Some(&Say { text: text.into() })).await
}

pub async fn profile(name: &str) -> Result<Profile> {
    imp::get(&format!("/api/users/{}", encode(name)), None).await
}

pub async fn leaderboard() -> Result<Leaderboard> {
    imp::get("/api/leaderboard", None).await
}

/// Finished games, newest first, as `q` asks.
pub async fn history(q: &HistoryQuery) -> Result<History> {
    let mut url = format!("/api/history?limit={}", q.limit.unwrap_or(20));
    if let Some(user) = &q.user {
        url.push_str(&format!("&user={}", encode(user)));
    }
    if let Some(before) = q.before {
        url.push_str(&format!("&before={before}"));
    }
    match q.venue {
        Some(Venue::Arena) => url.push_str("&venue=arena"),
        Some(Venue::Official) => url.push_str("&venue=official"),
        None => {}
    }
    if q.rated {
        url.push_str("&rated=true");
    }
    if q.voided {
        url.push_str("&voided=true");
    }
    if let Some(text) = q.q.as_deref().map(str::trim).filter(|t| !t.is_empty()) {
        url.push_str(&format!("&q={}", encode(text)));
    }
    if let Some(vs) = &q.vs {
        url.push_str(&format!("&vs={}", encode(vs)));
    }
    imp::get(&url, None).await
}

/// Player `name`'s record against everyone they played, over the games `q` picks.
pub async fn head_to_head(name: &str, q: &HeadToHeadQuery) -> Result<HeadToHead> {
    let mut url = format!("/api/users/{}/opponents?", encode(name));
    match q.venue {
        Some(Venue::Arena) => url.push_str("venue=arena&"),
        Some(Venue::Official) => url.push_str("venue=official&"),
        None => {}
    }
    if q.rated {
        url.push_str("rated=true");
    }
    imp::get(url.trim_end_matches(['?', '&']), None).await
}

#[cfg_attr(not(all(feature = "web", target_arch = "wasm32")), allow(dead_code))]
fn error_message(status: u16, body: &str) -> ApiError {
    let message = serde_json::from_str::<dominion_protocol::api::ApiError>(body)
        .map(|e| e.error)
        .unwrap_or_else(|_| {
            if body.trim().is_empty() {
                format!("the server answered HTTP {status}")
            } else {
                body.trim().to_string()
            }
        });
    ApiError { status, message }
}

#[cfg(all(feature = "web", target_arch = "wasm32"))]
mod imp {
    use super::{ApiError, Result, error_message};
    use futures_util::StreamExt;
    use gloo_net::eventsource::futures::EventSource;
    use gloo_net::http::{Request, RequestBuilder};
    use serde::Serialize;
    use serde::de::DeserializeOwned;

    fn unreachable(e: impl std::fmt::Display) -> ApiError {
        ApiError {
            status: 0,
            message: format!("could not reach the server: {e}"),
        }
    }

    fn auth(req: RequestBuilder, token: Option<&str>) -> RequestBuilder {
        match token {
            Some(t) => req.header("Authorization", &format!("Bearer {t}")),
            None => req,
        }
    }

    async fn perform(
        method: &str,
        url: &str,
        token: Option<&str>,
        body: Option<&impl Serialize>,
    ) -> Result<String> {
        let req = match method {
            "POST" => Request::post(url),
            "PUT" => Request::put(url),
            "DELETE" => Request::delete(url),
            _ => Request::get(url),
        };
        let req = auth(req, token);
        let req = match body {
            Some(body) => req.json(body),
            None => req.build(),
        }
        .map_err(unreachable)?;
        let resp = req.send().await.map_err(unreachable)?;
        let status = resp.status();
        let text = resp.text().await.map_err(unreachable)?;
        if !resp.ok() {
            return Err(error_message(status, &text));
        }
        Ok(text)
    }

    /// A request, with or without a body, whose answer is JSON.
    pub async fn send<T: DeserializeOwned>(
        method: &str,
        url: &str,
        token: Option<&str>,
        body: Option<&impl Serialize>,
    ) -> Result<T> {
        let text = perform(method, url, token, body).await?;
        serde_json::from_str(&text).map_err(|e| ApiError {
            status: 0,
            message: format!("unreadable response: {e}"),
        })
    }

    /// A request whose answer has no body.
    pub async fn send_empty(
        method: &str,
        url: &str,
        token: Option<&str>,
        body: Option<&impl Serialize>,
    ) -> Result<()> {
        perform(method, url, token, body).await.map(|_| ())
    }

    pub async fn get<T: DeserializeOwned>(url: &str, token: Option<&str>) -> Result<T> {
        send("GET", url, token, None::<&()>).await
    }

    pub async fn post<T: DeserializeOwned>(
        url: &str,
        token: Option<&str>,
        body: &impl Serialize,
    ) -> Result<T> {
        send("POST", url, token, Some(body)).await
    }

    /// The side of the square a picture is made, in pixels.
    const AVATAR_SIDE: u32 = 256;

    /// `file`'s middle square at `AVATAR_SIDE`, as a JPEG.
    async fn shrink(file: &dioxus::html::FileData) -> std::result::Result<web_sys::Blob, String> {
        use wasm_bindgen::{JsCast, JsValue};
        use wasm_bindgen_futures::JsFuture;
        let unreadable = |_| "this browser can't read that file as a picture".to_string();
        let file =
            (file.inner().downcast_ref::<web_sys::File>().cloned()).ok_or("no file was picked")?;
        let window = web_sys::window().ok_or("no window")?;
        let decoding = window
            .create_image_bitmap_with_blob(&file)
            .map_err(unreadable)?;
        let bitmap: web_sys::ImageBitmap = JsFuture::from(decoding)
            .await
            .map_err(unreadable)?
            .unchecked_into();
        let (w, h) = (bitmap.width() as f64, bitmap.height() as f64);
        let crop = w.min(h);
        let canvas: web_sys::HtmlCanvasElement = (window.document())
            .and_then(|d| d.create_element("canvas").ok())
            .and_then(|e| e.dyn_into().ok())
            .ok_or("no canvas")?;
        canvas.set_width(AVATAR_SIDE);
        canvas.set_height(AVATAR_SIDE);
        let ctx: web_sys::CanvasRenderingContext2d = (canvas.get_context("2d").ok().flatten())
            .and_then(|c| c.dyn_into().ok())
            .ok_or("no canvas")?;
        let side = AVATAR_SIDE as f64;
        ctx.set_fill_style_str("#fff");
        ctx.fill_rect(0.0, 0.0, side, side);
        ctx.draw_image_with_image_bitmap_and_sw_and_sh_and_dx_and_dy_and_dw_and_dh(
            &bitmap,
            (w - crop) / 2.0,
            (h - crop) / 2.0,
            crop,
            crop,
            0.0,
            0.0,
            side,
            side,
        )
        .map_err(unreadable)?;
        bitmap.close();
        let encoding = js_sys::Promise::new(&mut |resolve, reject| {
            let quality = JsValue::from_f64(0.85);
            if canvas
                .to_blob_with_type_and_encoder_options(&resolve, "image/jpeg", &quality)
                .is_err()
            {
                let _ = reject.call0(&JsValue::NULL);
            }
        });
        let blob = JsFuture::from(encoding).await.map_err(unreadable)?;
        blob.dyn_into().map_err(unreadable)
    }

    pub async fn upload_avatar(
        file: &dioxus::html::FileData,
    ) -> Result<dominion_protocol::lobby::Me> {
        let picture = shrink(file)
            .await
            .map_err(|message| ApiError { status: 0, message })?;
        let req = Request::put("/api/me/avatar")
            .header("Content-Type", "image/jpeg")
            .body(picture)
            .map_err(unreachable)?;
        let resp = req.send().await.map_err(unreachable)?;
        let status = resp.status();
        let text = resp.text().await.map_err(unreachable)?;
        if !resp.ok() {
            return Err(error_message(status, &text));
        }
        serde_json::from_str(&text).map_err(|e| ApiError {
            status: 0,
            message: format!("unreadable response: {e}"),
        })
    }

    /// Reads one connection's events, giving each `change` event's data to `on_change`;
    /// returns when it drops.
    pub async fn follow(url: &str, on_change: &mut dyn FnMut(&str)) -> std::result::Result<(), ()> {
        let mut source = EventSource::new(url).map_err(|_| ())?;
        let mut changes = source.subscribe("change").map_err(|_| ())?;
        while let Some(Ok((_, event))) = changes.next().await {
            if let Some(data) = event.data().as_string() {
                on_change(&data);
            }
        }
        source.close();
        Err(())
    }
}

#[cfg(not(all(feature = "web", target_arch = "wasm32")))]
mod imp {
    use super::{ApiError, Result};
    use serde::Serialize;
    use serde::de::DeserializeOwned;

    fn unavailable<T>() -> Result<T> {
        Err(ApiError {
            status: 0,
            message: "the API client only runs in a browser".into(),
        })
    }

    pub async fn send<T: DeserializeOwned>(
        _method: &str,
        _url: &str,
        _token: Option<&str>,
        _body: Option<&impl Serialize>,
    ) -> Result<T> {
        unavailable()
    }

    pub async fn send_empty(
        _method: &str,
        _url: &str,
        _token: Option<&str>,
        _body: Option<&impl Serialize>,
    ) -> Result<()> {
        unavailable()
    }

    pub async fn get<T: DeserializeOwned>(_url: &str, _token: Option<&str>) -> Result<T> {
        unavailable()
    }

    pub async fn post<T: DeserializeOwned>(
        _url: &str,
        _token: Option<&str>,
        _body: &impl Serialize,
    ) -> Result<T> {
        unavailable()
    }

    pub async fn upload_avatar(
        _file: &dioxus::html::FileData,
    ) -> Result<dominion_protocol::lobby::Me> {
        unavailable()
    }

    pub async fn follow(
        _url: &str,
        _on_change: &mut dyn FnMut(&str),
    ) -> std::result::Result<(), ()> {
        std::future::pending().await
    }
}

pub async fn transcript(
    game: &str,
    token: Option<&str>,
    after: u64,
) -> Result<dominion_protocol::transcript::TranscriptPage> {
    imp::get(
        &format!("/api/games/{}/transcript?after={after}", encode(game)),
        token,
    )
    .await
}

pub async fn watch_transcript(game: &str, token: Option<&str>, on_change: impl FnMut(Change)) {
    let mut url = format!("/api/games/{}/transcript/events", encode(game));
    if let Some(token) = token {
        url.push_str(&format!("?token={}", encode(token)));
    }
    follow(&url, on_change).await
}

pub(crate) async fn model_seats(url: &str) -> Result<Vec<dominion_protocol::api::ModelSeat>> {
    imp::get(url, None).await
}

pub(crate) async fn model_action(
    url: &str,
    req: &dominion_protocol::api::ModelRequest,
) -> Result<Vec<dominion_protocol::api::ModelSeat>> {
    imp::post(url, None, req).await
}

/// An official account's API path: the primary one's unprefixed when `account` is none.
pub(crate) fn online_path(account: Option<&str>, rest: &str) -> String {
    match account {
        Some(id) => format!("/api/online/{}/{rest}", encode(id)),
        None => format!("/api/online/{rest}"),
    }
}
pub async fn online_accounts() -> Result<Vec<dominion_protocol::api::OnlineAccountInfo>> {
    imp::get("/api/online/accounts", None).await
}
pub async fn online_view(
    account: Option<&str>,
) -> Result<dominion_protocol::api::OnlineObservation> {
    imp::get(&online_path(account, "view"), None).await
}
pub async fn follow_online(
    account: Option<&str>,
    on_change: &mut dyn FnMut(dominion_protocol::api::OnlineObservation),
) {
    let _ = imp::follow(&online_path(account, "events"), &mut |data| {
        if let Ok(view) = serde_json::from_str(data) {
            on_change(view)
        }
    })
    .await;
}

pub async fn online_rating(
    account: Option<&str>,
    kind: dominion_protocol::api::OnlineRatingType,
) -> Result<dominion_protocol::api::OnlineRating> {
    let kind = serde_json::to_value(kind).ok();
    let kind = kind.as_ref().and_then(|v| v.as_str()).unwrap_or("2p");
    imp::get(&online_path(account, &format!("rating?type={kind}")), None).await
}
pub async fn online_management(
    account: Option<&str>,
) -> Result<dominion_protocol::api::OnlineManagement> {
    imp::get(&online_path(account, "manage"), None).await
}
pub async fn online_start(
    account: Option<&str>,
    request: &dominion_protocol::api::OnlineStart,
) -> Result<dominion_protocol::api::OnlineManagement> {
    imp::post(&online_path(account, "start"), None, request).await
}
pub async fn online_attach(
    account: Option<&str>,
    agent: String,
) -> Result<dominion_protocol::api::OnlineManagement> {
    imp::post(
        &online_path(account, "agent"),
        None,
        &dominion_protocol::api::OnlineAttach { agent },
    )
    .await
}
/// Says `text` at official account `account`'s table on dominion.games, as the account.
pub async fn online_say(account: Option<&str>, text: &str) -> Result<()> {
    let say = Say { text: text.into() };
    imp::send_empty("POST", &online_path(account, "chat"), None, Some(&say)).await
}
pub async fn online_manage_action(
    account: Option<&str>,
    action: &str,
) -> Result<dominion_protocol::api::OnlineManagement> {
    let path = |rest| online_path(account, rest);
    match action {
        "cancel" => imp::send("DELETE", &path("matchmaking"), None, None::<&()>).await,
        "pause" => imp::send("DELETE", &path("agent"), None, None::<&()>).await,
        "resign" => imp::post(&path("resign"), None, &()).await,
        "reconnect" => imp::post(&path("reconnect"), None, &()).await,
        _ => Err(ApiError {
            status: 400,
            message: "unknown official game action".into(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_keep_the_server_message() {
        let e = error_message(
            409,
            r#"{"error":"decision d3 is not open; the open decision is d4"}"#,
        );
        assert_eq!(e.status, 409);
        assert!(e.is_stale());
        assert_eq!(
            e.message,
            "decision d3 is not open; the open decision is d4"
        );
        assert_eq!(error_message(502, "Bad Gateway").message, "Bad Gateway");
    }
}
