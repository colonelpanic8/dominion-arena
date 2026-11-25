//! The clock guard: the runner's last defence against a seat losing a timed game on
//! time. It watches the seat over a session of its own, beside whatever plays it (any
//! program, any attempt, and the gaps between attempts), and does nothing while the seat
//! has time. Only when one of the seat's decisions is still open with the bank all but
//! spent does it answer, with the least the decision allows: end the phase, decline,
//! choose nothing, else the first options. That is no strategy, only what keeps the seat
//! in the game; the player is told on its next reply, and every answer is logged as the
//! harness's. Decision ids keep it from racing the player: whichever answer arrives
//! second is refused as stale.
use crate::client::{CallError, Client, Reply};
use crate::harness::{game_state, is_clock_active};
use crate::house::Job;
use dominion_agent::AgentDecision;
use serde::Deserialize;
use serde_json::{Value, json};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

/// The longest the guard goes without looking at the seat while a decision is open.
const POLL: Duration = Duration::from_secs(10);
/// How long one `wait` for the seat's next decision lasts.
const MAX_WAIT: Duration = Duration::from_secs(30);
/// Local state and step calls normally take milliseconds. Reserve bank for recovery.
const MAX_CALL: Duration = Duration::from_secs(2);
const MIN_POLL: Duration = Duration::from_millis(100);
/// How long the guard waits before reaching the server again after losing it.
const RECONNECT: Duration = Duration::from_millis(500);
/// The most answers the guard tries for one decision before leaving it.
const ATTEMPTS: usize = 3;

/// `[clock_guard]` in the runner's settings.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClockGuardSettings {
    /// Whether the runner answers for a seat about to run out of time (default true).
    #[serde(default = "enabled")]
    pub enabled: bool,
    /// Seconds of bank left, after the decision's free time, at which it answers
    /// (default 6): enough for the answer to reach the game before the bank is spent.
    #[serde(default = "margin_seconds")]
    pub margin_seconds: u32,
}

fn enabled() -> bool {
    true
}

fn margin_seconds() -> u32 {
    6
}

impl Default for ClockGuardSettings {
    fn default() -> Self {
        Self {
            enabled: enabled(),
            margin_seconds: margin_seconds(),
        }
    }
}

/// What the harness has to tell the player on its next reply, whichever attempt it is.
#[derive(Clone, Default)]
pub(crate) struct Notices(Arc<Mutex<Vec<String>>>);

impl Notices {
    fn push(&self, notice: String) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(notice);
    }

    pub(crate) fn take(&self) -> Vec<String> {
        std::mem::take(&mut *self.0.lock().unwrap_or_else(PoisonError::into_inner))
    }
}

/// What the guard does next, given the seat's view.
#[derive(Debug, PartialEq)]
enum Next {
    /// The game is over, or untimed: nothing to guard.
    Done,
    /// No decision of the seat's is open.
    Wait,
    /// Look again after this long.
    Sleep(Duration),
    /// The bank is all but spent with this decision open.
    Answer(Box<AgentDecision>, u64),
}

/// The seat's bank as of `now_ms`, once the decision's free time is spent, while its own
/// clock runs; `None` when its clock isn't running or isn't known.
fn bank_left(reply: &Value, now_ms: u64) -> Option<u64> {
    let view = game_state(reply);
    let seat = view["seat"].as_u64()?;
    let (clock, info) = (&reply["clock"], &reply["clockInfo"]);
    if clock["running"].as_u64() != Some(seat) || info["status"] != "available" {
        return None;
    }
    let seat = usize::try_from(seat).ok()?;
    let bank = clock["left"].get(seat)?.as_u64()?;
    let free = clock["free"].get(seat).and_then(Value::as_u64).unwrap_or(0);
    let age = now_ms.saturating_sub(info["asOfMs"].as_u64().unwrap_or(now_ms));
    Some(bank.saturating_sub(age.saturating_sub(free)))
}

fn next(reply: &Value, game: &str, now_ms: u64, margin_ms: u64) -> Next {
    let view = game_state(reply);
    if view["game"]["running"] == false || !is_clock_active(reply) {
        return Next::Done;
    }
    if view["game"]["id"].as_str() != Some(game) {
        return Next::Sleep(POLL);
    }
    let Some(decision) = view.get("decision").filter(|d| !d.is_null()) else {
        return Next::Wait;
    };
    let Some(left) = bank_left(reply, now_ms) else {
        return Next::Sleep(POLL);
    };
    if left > margin_ms {
        return Next::Sleep(Duration::from_millis(left - margin_ms).min(POLL));
    }
    match serde_json::from_value::<AgentDecision>(decision.clone()) {
        Ok(decision) => Next::Answer(Box::new(decision), left),
        Err(_) => Next::Sleep(POLL),
    }
}

/// The answers the guard tries, in order: the least the decision allows, then nothing,
/// then its first option.
fn fallbacks(d: &AgentDecision) -> Vec<Vec<u32>> {
    let least = crate::steps::idle_answer(d).indices.unwrap_or_default();
    let mut answers = vec![least];
    for choice in std::iter::once(vec![]).chain(d.options.first().map(|o| vec![o.index])) {
        if !answers.contains(&choice) {
            answers.push(choice);
        }
    }
    answers
}

fn labels(d: &AgentDecision, indices: &[u32]) -> Vec<String> {
    (indices.iter())
        .filter_map(|i| d.options.iter().find(|o| o.index == *i))
        .map(|o| o.label.clone())
        .collect()
}

/// Time available for one connection and request, leaving room to observe the result
/// and retry. A small floor lets us discover a new turn even after a stale clock expires.
fn call_budget(bank_until: Option<Instant>) -> Duration {
    bank_until.map_or(MAX_CALL, |end| {
        let remaining = end.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            MIN_POLL
        } else {
            (remaining / 3).clamp(MIN_POLL, MAX_CALL).min(remaining)
        }
    })
}

/// Remember attempts across reads and reconnects. An unknown outcome retries the same
/// answer only if a fresh observation still offers that decision.
#[derive(Default)]
struct Attempts {
    decision: String,
    sent: usize,
    fallback: usize,
}

impl Attempts {
    fn take(&mut self, d: &AgentDecision) -> Option<Vec<u32>> {
        if self.decision != d.id {
            *self = Self {
                decision: d.id.clone(),
                ..Self::default()
            };
        }
        if self.sent >= ATTEMPTS {
            return None;
        }
        let choice = fallbacks(d).into_iter().nth(self.fallback)?;
        self.sent += 1;
        Some(choice)
    }

    fn replied(&mut self, accepted: Option<bool>) {
        match accepted {
            Some(true) => self.sent = ATTEMPTS,
            Some(false) => self.fallback += 1,
            None => {}
        }
    }
}

/// The guard's own session with the seat's game.
struct Session<'a> {
    endpoint: String,
    job: &'a Job,
    client: Option<Client>,
}

impl Session<'_> {
    fn args(&self, mut args: Value) -> Value {
        if !self.job.official {
            args["token"] = json!(self.job.token);
        }
        args
    }

    async fn call(
        &mut self,
        tool: &'static str,
        args: Value,
        budget: Duration,
    ) -> Result<Reply, CallError> {
        let args = self.args(args);
        let request = async {
            if self.client.is_none() {
                let token = self.job.official.then(|| self.job.token.clone());
                self.client = Some(
                    Client::connect_authenticated(&self.endpoint, token)
                        .await
                        .map_err(CallError::Lost)?,
                );
            }
            self.client
                .as_ref()
                .expect("connected")
                .call(tool, args)
                .await
        };
        let result = tokio::time::timeout(budget, request)
            .await
            .unwrap_or_else(|_| {
                Err(CallError::Lost(format!(
                    "{tool}: no reply within {budget:?}"
                )))
            });
        if matches!(result, Err(CallError::Lost(_))) {
            // Dropping the MCP service cancels it. Waiting for its shutdown would spend
            // the very clock budget the timeout protects.
            self.client = None;
        }
        result
    }
}

/// Guards `job`'s seat until `stop`, or until its game is over or turns out untimed.
pub(crate) async fn run(
    settings: ClockGuardSettings,
    server: String,
    job: Job,
    notices: Notices,
    log: PathBuf,
    stop: CancellationToken,
) {
    if !settings.enabled {
        return;
    }
    let endpoint = match crate::harness::endpoint(&server, &job) {
        Ok(endpoint) => endpoint,
        Err(e) => return eprintln!("dominion agents: clock guard for {}: {e}", job.game_id),
    };
    let mut session = Session {
        endpoint,
        job: &job,
        client: None,
    };
    let margin = u64::from(settings.margin_seconds) * 1000;
    let mut wait = Duration::ZERO;
    let mut bank_until = None;
    let mut attempts = Attempts::default();
    let guard = async {
        loop {
            let hold = std::mem::take(&mut wait);
            let (tool, args) = if hold.is_zero() {
                ("state", json!({}))
            } else {
                ("wait", json!({"timeout_s": hold.as_secs()}))
            };
            let budget = hold + call_budget(bank_until);
            let reply = match session.call(tool, args, budget).await {
                Ok(reply) if reply.ok && reply.json.is_some() => reply,
                _ => {
                    tokio::time::sleep(RECONNECT.min(call_budget(bank_until))).await;
                    continue;
                }
            };
            let json = reply.json.expect("checked");
            let now = crate::hub::now_ms();
            let seat = game_state(&json)["seat"]
                .as_u64()
                .and_then(|s| usize::try_from(s).ok());
            let own_bank = seat
                .and_then(|s| json["clock"]["left"].get(s))
                .and_then(Value::as_u64);
            // A paused clock may start at any point during the next request.
            let remaining = bank_left(&json, now).or(own_bank);
            bank_until = remaining.map(|ms| Instant::now() + Duration::from_millis(ms));
            match next(&json, &job.game_id, now, margin) {
                Next::Done => return,
                Next::Wait => {
                    // Even an opponent's turn can end as soon as this wait starts.
                    // Bound a stalled wait by the time our seat could then lose.
                    let left = remaining.unwrap_or(margin);
                    wait = Duration::from_secs((left / 3000).min(MAX_WAIT.as_secs()));
                    if wait.is_zero() {
                        tokio::time::sleep(MIN_POLL).await;
                    }
                }
                Next::Sleep(pause) => tokio::time::sleep(pause).await,
                Next::Answer(d, left) => {
                    if let Some(indices) = attempts.take(&d) {
                        let accepted = answer(
                            &mut session,
                            &d,
                            indices,
                            left,
                            call_budget(bank_until),
                            &notices,
                            &log,
                        )
                        .await;
                        attempts.replied(accepted);
                    } else {
                        tokio::time::sleep(RECONNECT.min(call_budget(bank_until))).await;
                    }
                }
            }
        }
    };
    tokio::select! {
        () = stop.cancelled() => {},
        () = guard => {},
    }
}

/// Submit once. Only the server's reply can tell us whether our step was accepted.
async fn answer(
    session: &mut Session<'_>,
    d: &AgentDecision,
    indices: Vec<u32>,
    left: u64,
    budget: Duration,
    notices: &Notices,
    log: &Path,
) -> Option<bool> {
    let job = session.job;
    let labels = labels(d, &indices);
    let args = json!({"decision": d.id, "indices": indices, "expect": labels, "timeout_s": 0});
    let reply = session.call("step", args, budget).await;
    let accepted = match &reply {
        Ok(reply) => Some(reply.ok),
        Err(CallError::Refused(_)) => Some(false),
        Err(CallError::Lost(_)) => None,
    };
    let entry = json!({
        "time": crate::hub::now_ms(), "game": job.game_id, "seat": job.seat,
        "agent": job.agent, "decision": d.id, "prompt": d.prompt, "indices": indices,
        "labels": labels, "bankLeftMs": left, "accepted": accepted,
        "error": match &reply { Ok(r) if !r.ok => Some(r.text.clone()), Err(e) => Some(e.to_string()), _ => None },
    });
    record(log, &entry);
    if accepted == Some(true) {
        let chosen = match labels.is_empty() {
            true => "nothing".to_string(),
            false => labels.join(", "),
        };
        eprintln!(
            "dominion agents: clock guard answered {} in {} for {} with {chosen}: {:.1} s of bank left",
            d.id,
            job.game_id,
            job.agent,
            left as f64 / 1000.0
        );
        notices.push(format!(
            "HARNESS CLOCK GUARD: your clock was about to run out ({:.1} s of bank left) with decision {} open, so the harness answered it for you with the least it allows ({chosen}). Decide faster: answer each decision promptly.",
            left as f64 / 1000.0,
            d.id
        ));
    }
    accepted
}

/// Appends `entry` to the guard's log, kept beside the runner's archives.
fn record(log: &Path, entry: &Value) {
    let mut options = std::fs::OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let written = options.open(log).and_then(|mut file| {
        let mut line = serde_json::to_vec(entry).expect("serializable");
        line.push(b'\n');
        file.write_all(&line)
    });
    if let Err(e) = written {
        eprintln!("dominion agents: clock guard log {}: {e}", log.display());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view(left: u64, free: u64, as_of: u64) -> Value {
        json!({"view":{"seat":1,"game":{"id":"g","running":true},
            "decision":{"id":"d1","prompt":"Buy a card","kind":"choose","purpose":"turn","ordered":false,"nameAny":false,
                "min":1,"max":1,"groups":[],"options":[
                    {"index":0,"group":0,"action":"buy","label":"Buy Gold","card":"Gold"},
                    {"index":1,"group":0,"action":"end-phase","label":"End Buy phase"}]}},
            "clock":{"running":1,"left":[90_000,left],"free":[0,free]},
            "clockInfo":{"status":"available","asOfMs":as_of}})
    }

    #[test]
    fn the_guard_waits_while_the_seat_has_time_and_answers_at_the_margin() {
        let margin = 6_000;
        // 30 s of bank: look again in at most ten seconds.
        assert_eq!(
            next(&view(30_000, 0, 1000), "g", 1000, margin),
            Next::Sleep(POLL)
        );
        // Free time is spent first: 7 s of bank and 5 s free, 4 s on: one second to go.
        assert_eq!(
            next(&view(7_000, 5_000, 1000), "g", 5_000, margin),
            Next::Sleep(Duration::from_millis(1_000))
        );
        let Next::Answer(d, left) = next(&view(7_000, 5_000, 1000), "g", 7_000, margin) else {
            panic!("the bank is at the margin")
        };
        assert_eq!((d.id.as_str(), left), ("d1", 6_000));
        // The least it allows: end the phase, not buy Gold.
        assert_eq!(crate::steps::idle_answer(&d).indices, Some(vec![1]));
        assert_eq!(fallbacks(&d), vec![vec![1], vec![], vec![0]]);
        assert_eq!(labels(&d, &[1]), vec!["End Buy phase".to_string()]);
    }

    #[test]
    fn a_call_reserves_time_for_recovery_as_the_bank_runs_down() {
        let now = Instant::now();
        assert_eq!(call_budget(None), MAX_CALL);
        assert_eq!(call_budget(Some(now + Duration::from_secs(60))), MAX_CALL);
        assert!(call_budget(Some(now + Duration::from_secs(3))) <= Duration::from_secs(1));
        assert_eq!(call_budget(Some(now)), MIN_POLL);
    }

    #[test]
    fn repeated_observations_of_a_decision_share_one_attempt_limit() {
        let Next::Answer(mut d, _) = next(&view(1_000, 0, 1_000), "g", 1_000, 6_000) else {
            panic!("guard should answer")
        };
        let mut attempts = Attempts::default();
        assert_eq!(attempts.take(&d), Some(vec![1]));
        attempts.replied(None);
        assert_eq!(attempts.take(&d), Some(vec![1]));
        attempts.replied(Some(false));
        assert_eq!(attempts.take(&d), Some(vec![]));
        attempts.replied(None);
        assert_eq!(attempts.take(&d), None);
        d.id = "d2".into();
        assert_eq!(attempts.take(&d), Some(vec![1]));
        attempts.replied(Some(true));
        assert_eq!(attempts.take(&d), None);
    }

    #[test]
    fn the_guard_leaves_other_seats_clocks_untimed_games_and_finished_ones() {
        let margin = 6_000;
        let mut theirs = view(1_000, 0, 1000);
        theirs["clock"]["running"] = json!(0);
        assert_eq!(next(&theirs, "g", 1000, margin), Next::Sleep(POLL));
        let mut none = view(1_000, 0, 1000);
        none["view"]["decision"] = Value::Null;
        assert_eq!(next(&none, "g", 1000, margin), Next::Wait);
        let mut unknown = view(1_000, 0, 1000);
        unknown["clockInfo"]["status"] = json!("unavailable");
        assert_eq!(next(&unknown, "g", 1000, margin), Next::Sleep(POLL));
        assert_eq!(
            next(&view(1_000, 0, 1000), "other", 1000, margin),
            Next::Sleep(POLL)
        );
        let mut untimed = view(1_000, 0, 1000);
        untimed["clockInfo"]["status"] = json!("off");
        assert_eq!(next(&untimed, "g", 1000, margin), Next::Done);
        let mut over = view(1_000, 0, 1000);
        over["view"]["game"]["running"] = json!(false);
        assert_eq!(next(&over, "g", 1000, margin), Next::Done);
    }

    /// A seat whose player never answers keeps its timed game: the guard answers its
    /// decision once the bank reaches the margin, logs it, and leaves a notice.
    #[tokio::test]
    async fn the_guard_answers_a_silent_seat_before_its_clock_runs_out() {
        guard_game(false).await;
    }

    #[tokio::test]
    async fn the_guard_reads_state_after_a_committed_step_loses_its_reply() {
        guard_game(true).await;
    }

    async fn guard_game(lose_first_reply: bool) {
        use crate::arena::Arena;
        use crate::hub::{Hub, NewGame};
        use axum::{
            body::{Body, to_bytes},
            extract::Request,
            middleware::{Next, from_fn},
        };
        use dominion_protocol::lobby::Clock;
        use rmcp::transport::streamable_http_server::{
            StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
        };
        use std::sync::atomic::{AtomicBool, Ordering};
        let hub = Arc::new(Hub::default());
        let arena = Arena::open(hub.clone(), Default::default()).unwrap();
        let service = StreamableHttpService::new(
            move || {
                Ok(crate::mcp::Session::new(
                    arena.clone(),
                    Duration::from_secs(2),
                    false,
                ))
            },
            Arc::new(LocalSessionManager::default()),
            StreamableHttpServerConfig::default(),
        );
        let mut router = axum::Router::new().nest_service("/mcp", service);
        let calls = Arc::new(Mutex::new(Vec::new()));
        if lose_first_reply {
            let received = calls.clone();
            let lose = Arc::new(AtomicBool::new(true));
            router = router.layer(from_fn(move |req: Request, next: Next| {
                let received = received.clone();
                let lose = lose.clone();
                async move {
                    let (parts, body) = req.into_parts();
                    let bytes = to_bytes(body, 1 << 20).await.unwrap();
                    let message: Value = serde_json::from_slice(&bytes).unwrap_or_default();
                    let mut stall = false;
                    if message["method"] == "tools/call" {
                        let name = message["params"]["name"].as_str().unwrap();
                        let decision = message["params"]["arguments"]["decision"]
                            .as_str()
                            .map(str::to_owned);
                        received.lock().unwrap().push((name.to_owned(), decision));
                        stall = name == "step" && lose.swap(false, Ordering::Relaxed);
                    }
                    let response = next
                        .run(Request::from_parts(parts, Body::from(bytes)))
                        .await;
                    if stall {
                        std::future::pending::<()>().await;
                    }
                    response
                }
            }));
        }
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/mcp", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, router).await });
        let created = hub
            .create(&NewGame {
                players: 2,
                seed: Some(7),
                kingdom: ["Village", "Smithy", "Market", "Militia", "Moat"]
                    .map(String::from)
                    .to_vec(),
                clock: Some(Clock::new(30, 0)),
                ..Default::default()
            })
            .unwrap();
        let job = Job {
            options: Default::default(),
            official: false,
            official_account: None,
            transcript: false,
            configuration: None,
            agent: "Silent".into(),
            brief: None,
            debrief: None,
            game_id: created.game_id.clone(),
            seat: 1,
            name: "P2".into(),
            token: created.seats[1].token.clone(),
        };
        let first = hub
            .claim(&created.game_id, &created.seats[0].token)
            .unwrap();
        let second = hub
            .claim(&created.game_id, &created.seats[1].token)
            .unwrap();
        // The first player ends its turn at once.
        while let Some(d) = hub.view(&first).unwrap().0.decision {
            hub.answer(&first, &crate::steps::idle_answer(&d)).unwrap();
        }
        let waiting = hub.view(&second).unwrap().0.decision.unwrap();
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("clock-guard.jsonl");
        let notices = Notices::default();
        let stop = CancellationToken::new();
        let settings = ClockGuardSettings {
            enabled: true,
            margin_seconds: 28,
        };
        let guard = tokio::spawn(run(
            settings,
            url,
            job,
            notices.clone(),
            log.clone(),
            stop.clone(),
        ));
        let started = std::time::Instant::now();
        while hub
            .view(&second)
            .unwrap()
            .0
            .decision
            .as_ref()
            .map(|d| &d.id)
            == Some(&waiting.id)
        {
            assert!(
                started.elapsed() < Duration::from_secs(20),
                "the guard never answered"
            );
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        // Not before the bank reached the margin.
        assert!(started.elapsed() >= Duration::from_secs(1));
        assert!(hub.view(&second).unwrap().0.game.running);
        if lose_first_reply {
            tokio::time::timeout(Duration::from_secs(5), async {
                loop {
                    {
                        let history = calls.lock().unwrap();
                        if let Some(i) = history.iter().position(|(name, _)| name == "step")
                            && history.len() >= i + 3
                        {
                            assert_eq!(history[i + 1].0, "state");
                            assert_eq!(
                                history
                                    .iter()
                                    .filter(|(n, d)| n == "step" && d.as_ref() == Some(&waiting.id))
                                    .count(),
                                1
                            );
                            break;
                        }
                    }
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
            })
            .await
            .expect("the guard must reconnect and read the committed state");
        }
        let entry: Value = serde_json::from_str(
            std::fs::read_to_string(&log)
                .unwrap()
                .lines()
                .next()
                .unwrap(),
        )
        .unwrap();
        assert_eq!(entry["decision"], json!(waiting.id));
        assert_eq!(
            entry["accepted"],
            if lose_first_reply {
                Value::Null
            } else {
                json!(true)
            }
        );
        assert!(entry["bankLeftMs"].as_u64().unwrap() <= 28_000);
        // The player hears of it on its next reply, whichever attempt plays then.
        if !lose_first_reply {
            let advice = crate::runner::analyst::Advice::with_notices(notices.clone());
            let mut reply = Reply {
                model_prompts: vec![],
                ok: true,
                text: "DECISION d2".into(),
                json: Some(json!({"view":{}})),
            };
            advice.deliver(&mut reply);
            assert!(
                reply.text.starts_with("HARNESS CLOCK GUARD"),
                "{}",
                reply.text
            );
            assert!(reply.text.contains(&waiting.id) && reply.text.ends_with("DECISION d2"));
            assert!(notices.take().is_empty());
        }
        stop.cancel();
        tokio::time::timeout(Duration::from_secs(5), guard)
            .await
            .unwrap()
            .unwrap();
    }

    #[test]
    fn settings_default_on_with_a_six_second_margin() {
        let settings: ClockGuardSettings = toml::from_str("").unwrap();
        assert_eq!(settings, ClockGuardSettings::default());
        assert!(settings.enabled && settings.margin_seconds == 6);
        let off: ClockGuardSettings = toml::from_str("enabled = false").unwrap();
        assert!(!off.enabled);
    }
}
