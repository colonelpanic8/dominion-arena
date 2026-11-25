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
use tokio_util::sync::CancellationToken;

/// The longest the guard goes without looking at the seat while a decision is open.
const POLL: Duration = Duration::from_secs(10);
/// How long one `wait` for the seat's next decision lasts.
const WAIT_S: u64 = 30;
/// How long the guard waits before reaching the server again after losing it.
const RECONNECT: Duration = Duration::from_secs(2);
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
    let mut answers = vec![least, vec![]];
    answers.extend(d.options.first().map(|o| vec![o.index]));
    answers.dedup();
    answers.truncate(ATTEMPTS);
    answers
}

fn labels(d: &AgentDecision, indices: &[u32]) -> Vec<String> {
    (indices.iter())
        .filter_map(|i| d.options.iter().find(|o| o.index == *i))
        .map(|o| o.label.clone())
        .collect()
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

    async fn call(&mut self, tool: &'static str, args: Value) -> Result<Reply, String> {
        if self.client.is_none() {
            let token = self.job.official.then(|| self.job.token.clone());
            self.client = Some(Client::connect_authenticated(&self.endpoint, token).await?);
        }
        let args = self.args(args);
        let client = self.client.as_ref().expect("connected");
        let result =
            tokio::time::timeout(crate::mcp::MAX_WAIT + POLL, client.call(tool, args)).await;
        // A session the server no longer knows (it restarted) is refused, not lost: any
        // failure starts a fresh one.
        let error = match result {
            Ok(Ok(reply)) => return Ok(reply),
            Ok(Err(CallError::Refused(e) | CallError::Lost(e))) => e,
            Err(_) => format!("{tool}: no reply"),
        };
        if let Some(client) = self.client.take() {
            client.close().await;
        }
        Err(error)
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
    let mut tool = "state";
    tokio::select! {
        () = stop.cancelled() => {},
        () = async {
            loop {
                let args = match tool {
                    "wait" => json!({"timeout_s": WAIT_S}),
                    _ => json!({}),
                };
                let reply = match session.call(tool, args).await {
                    Ok(reply) if reply.ok && reply.json.is_some() => reply,
                    // The server away, or the seat refused for now (a reconnecting
                    // official account): look again shortly.
                    _ => {
                        tool = "state";
                        tokio::time::sleep(RECONNECT).await;
                        continue;
                    }
                };
                let json = reply.json.expect("checked");
                tool = "state";
                match next(&json, &job.game_id, crate::hub::now_ms(), margin) {
                    Next::Done => return,
                    Next::Wait => tool = "wait",
                    Next::Sleep(pause) => tokio::time::sleep(pause).await,
                    Next::Answer(decision, left) => {
                        answer(&mut session, &decision, left, &notices, &log).await;
                    }
                }
            }
        } => {},
    }
    if let Some(client) = session.client.take() {
        client.close().await;
    }
}

/// Answers `d` with the least it allows; the first fallback the game takes stands.
async fn answer(
    session: &mut Session<'_>,
    d: &AgentDecision,
    left: u64,
    notices: &Notices,
    log: &Path,
) {
    let job = session.job;
    for indices in fallbacks(d) {
        let labels = labels(d, &indices);
        let args = json!({"decision": d.id, "indices": indices, "expect": labels, "timeout_s": 0});
        let reply = session.call("step", args).await;
        let accepted = reply.as_ref().is_ok_and(|r| r.ok);
        let entry = json!({
            "time": crate::hub::now_ms(), "game": job.game_id, "seat": job.seat,
            "agent": job.agent, "decision": d.id, "prompt": d.prompt, "indices": indices,
            "labels": labels, "bankLeftMs": left, "accepted": accepted,
            "error": match &reply { Ok(r) if !r.ok => Some(r.text.clone()), Err(e) => Some(e.clone()), _ => None },
        });
        record(log, &entry);
        if accepted {
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
            return;
        }
        // The player's answer got there first, or the game moved on.
        let Ok(fresh) = session.call("state", json!({})).await else {
            return;
        };
        let open = fresh
            .json
            .as_ref()
            .map(|j| game_state(j)["decision"]["id"].clone());
        if open != Some(json!(d.id)) {
            return;
        }
    }
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
        use crate::arena::Arena;
        use crate::hub::{Hub, NewGame};
        use dominion_protocol::lobby::Clock;
        use rmcp::transport::streamable_http_server::{
            StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
        };
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
        let router = axum::Router::new().nest_service("/mcp", service);
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
        let entry: Value = serde_json::from_str(
            std::fs::read_to_string(&log)
                .unwrap()
                .lines()
                .next()
                .unwrap(),
        )
        .unwrap();
        assert_eq!(entry["decision"], json!(waiting.id));
        assert_eq!(entry["accepted"], json!(true));
        assert!(entry["bankLeftMs"].as_u64().unwrap() <= 28_000);
        // The player hears of it on its next reply, whichever attempt plays then.
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
