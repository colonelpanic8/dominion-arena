//! The server's agent runner. Each native harness receives game observations and
//! returns structured actions; only Dominion's bridge holds the seat credentials.

use crate::house::{FromRunner, Job, Offer, ToRunner};
pub use clock_guard::ClockGuardSettings;
use dominion_protocol::lobby::AgentSetup;
pub use dominion_protocol::lobby::RunnerStrategy;
pub(crate) mod analyst;
pub(crate) mod clock_guard;
pub(crate) mod provider;
pub(crate) mod strategy;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::sync::mpsc::{UnboundedSender, unbounded_channel};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

const RECONNECT: Duration = Duration::from_secs(5);
const GRACE: Duration = Duration::from_secs(30);
/// The longest a seat waits between attempts.
const MAX_RETRY: Duration = Duration::from_secs(60);
/// The longest an official seat waits between attempts: its clock runs meanwhile.
const OFFICIAL_MAX_RETRY: Duration = Duration::from_secs(10);
/// An attempt that played this long before failing starts the backoff again.
const SETTLED: Duration = Duration::from_secs(120);
/// How often the providers' catalogs are read again.
const REDISCOVER: Duration = Duration::from_secs(600);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Program {
    Claude,
    Codex,
    #[serde(alias = "gemini")]
    Antigravity,
    /// T3 Code's MCP server, with one persistent conversation per seat.
    T3Code,
    /// OpenAI's Responses API called directly (`crate::openai`), with no CLI.
    OpenAi,
    /// Anthropic's Messages API called directly (`crate::anthropic`), with no CLI.
    Anthropic,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OpeningAnalyst {
    pub model: String,
    pub effort: String,
    /// Target from the first playable opening observation to requesting the move.
    #[serde(default = "default_opening_target")]
    pub target_seconds: u32,
    /// Revisit strategy as public scores and supply piles change.
    #[serde(default)]
    pub background: bool,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClockAwareSettings {
    pub analyst: String,
    #[serde(default = "default_opening_target")]
    pub target_seconds: u32,
    #[serde(default)]
    pub background: bool,
}

impl ClockAwareSettings {
    pub(crate) fn resolve(&self, agents: &[AgentSpec]) -> Result<provider::AnalysisPlan, String> {
        let model = agents
            .iter()
            .find(|a| a.name.eq_ignore_ascii_case(&self.analyst))
            .ok_or_else(|| {
                format!(
                    "clock_aware.analyst names an unknown agent profile: {}",
                    self.analyst
                )
            })?;
        if model.program == Program::T3Code {
            return Err("T3 Code does not offer isolated analysis".into());
        }
        Ok(provider::AnalysisPlan {
            model: model.clone(),
            target_seconds: self.target_seconds,
            background: self.background,
        })
    }
}

fn default_opening_target() -> u32 {
    45
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentSpec {
    pub name: String,
    pub program: Program,
    pub model: Option<String>,
    pub command: Option<String>,
    pub about: Option<String>,
    pub instructions: Option<String>,
    /// Request the priority service tier for a direct OpenAI or native Codex seat.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub fast_mode: bool,
    /// Optional concurrent Antigravity analysis of the public opening in clocked games.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opening_analyst: Option<OpeningAnalyst>,
    /// Native reasoning effort.
    pub effort: Option<String>,
    /// Maximum model inactivity between events, excluding MCP calls (default 300 seconds).
    pub response_seconds: Option<u32>,
    /// Legacy setting, accepted for compatibility. Native sessions retain their own context.
    pub history_turns: Option<usize>,
    /// Provider's native credential/config directory (default its standard location).
    pub auth_dir: Option<PathBuf>,
    /// antigravity: where seats keep their conversations, a folder each, so what counts
    /// Antigravity's usage from them counts the seats' (default
    /// `<auth_dir>/antigravity-cli/conversations/dominion-arena`).
    pub conversations_dir: Option<PathBuf>,
    /// T3 Code's provider instance (account), when program is t3code.
    pub provider_instance: Option<String>,
    /// Options T3 Code advertises for this model, when program is t3code.
    #[serde(default)]
    pub options: BTreeMap<String, crate::t3::OptionValue>,
    /// openai: the environment variable holding an API key; without it the agent signs in
    /// with Codex's ChatGPT login (`auth.json` in `auth_dir`, default `$CODEX_HOME`).
    /// anthropic: the one holding its API key (default `ANTHROPIC_API_KEY`).
    pub api_key_env: Option<String>,
    /// openai: game updates kept verbatim in the conversation (default 60); older ones
    /// are replaced by a note. anthropic: the game updates the API keeps when it clears
    /// old ones (default 60).
    pub context_results: Option<usize>,
}

impl Program {
    pub(crate) fn supports_speed(self) -> bool {
        matches!(self, Self::OpenAi | Self::Codex)
    }
}

impl AgentSpec {
    pub(super) fn for_launch(
        &self,
        options: dominion_protocol::lobby::LaunchOptions,
    ) -> Result<Self, String> {
        let mut spec = self.clone();
        if let Some(speed) = options.speed {
            if !spec.program.supports_speed() {
                return Err(format!(
                    "{} does not support selectable inference speed",
                    spec.name
                ));
            }
            spec.fast_mode = speed == dominion_protocol::lobby::InferenceSpeed::Fast;
        }
        Ok(spec)
    }

    /// What plays it: "Claude Code, claude-opus-5-5, high effort".
    pub fn about(&self) -> String {
        self.about.clone().unwrap_or_else(|| {
            let parts = [Some(self.program.label().to_string()), self.model.clone()]
                .into_iter()
                .chain([self.effort.as_ref().map(|e| format!("{e} effort"))]);
            parts.flatten().collect::<Vec<_>>().join(", ")
        })
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.name.trim().is_empty() || self.name.trim() != self.name {
            return Err("agent name must be nonempty without surrounding whitespace".into());
        }
        if self.fast_mode && !self.program.supports_speed() {
            return Err(format!("{}: fast_mode requires openai or codex", self.name));
        }
        if let Some(analyst) = &self.opening_analyst
            && (self.program != Program::Antigravity
                || analyst.model.trim().is_empty()
                || !self.program.efforts().contains(&analyst.effort.as_str()))
        {
            return Err(format!(
                "{}: opening_analyst requires Antigravity, a model and a supported effort",
                self.name
            ));
        }
        if self.response_seconds == Some(0) {
            return Err("response_seconds must be positive".into());
        }
        if self.program != Program::T3Code
            && (!self.options.is_empty() || self.provider_instance.is_some())
        {
            return Err(format!(
                "{}: options and provider_instance require program = \"t3code\"",
                self.name
            ));
        }
        if self.program != Program::Antigravity && self.conversations_dir.is_some() {
            return Err(format!(
                "{}: conversations_dir requires program = \"antigravity\"",
                self.name
            ));
        }
        let direct = matches!(self.program, Program::OpenAi | Program::Anthropic);
        if direct && self.model.is_none() {
            return Err(format!("{}: a direct API agent needs a model", self.name));
        }
        if !direct && (self.api_key_env.is_some() || self.context_results.is_some()) {
            return Err(format!(
                "{}: api_key_env and context_results require program = \"openai\" or \"anthropic\"",
                self.name
            ));
        }
        if let Some(effort) = &self.effort
            && !self.program.efforts().contains(&effort.as_str())
        {
            return Err(format!("unsupported {:?} effort {effort:?}", self.program));
        }
        Ok(())
    }
}

pub fn default_agents() -> Vec<AgentSpec> {
    [
        ("Claude Opus", Program::Claude, Some("opus")),
        ("Claude Sonnet", Program::Claude, Some("sonnet")),
        ("Codex", Program::Codex, None),
    ]
    .into_iter()
    .map(|(name, program, model)| AgentSpec {
        name: name.into(),
        program,
        model: model.map(String::from),
        command: None,
        about: None,
        instructions: None,
        effort: None,
        fast_mode: false,
        response_seconds: None,
        history_turns: None,
        opening_analyst: None,
        api_key_env: None,
        context_results: None,
        auth_dir: None,
        conversations_dir: None,
        provider_instance: None,
        options: BTreeMap::new(),
    })
    .collect()
}

#[derive(Clone, Debug)]
pub struct Settings {
    pub socket: PathBuf,
    pub server: String,
    pub agents: Vec<AgentSpec>,
    pub analysis: Option<provider::AnalysisPlan>,
    /// Answers for a seat about to run out of time.
    pub clock_guard: ClockGuardSettings,
    /// Providers whose models are each offered, at each effort.
    pub catalogs: Vec<crate::catalog::CatalogSpec>,
    /// Agent seats played at once.
    pub capacity: u32,
    /// Workspaces and durable archives.
    pub dir: PathBuf,
    pub t3code: Option<crate::t3::Settings>,
    /// The playbook, tools and hosts native seats run with.
    pub sandbox: crate::harness::Sandbox,
}

type Key = (String, u8);

struct Playing {
    job: Job,
    stop: CancellationToken,
    task: JoinHandle<()>,
}

/// Runs until the process is stopped, reconnecting to a server that goes away; the
/// agents playing meanwhile go on.
pub async fn run(settings: Settings) -> Result<(), String> {
    let stop = CancellationToken::new();
    let signal = stop.clone();
    let handler = tokio::spawn(async move {
        let _ = tokio::signal::ctrl_c().await;
        signal.cancel();
    });
    let result = run_until(settings, stop).await;
    handler.abort();
    result
}

pub async fn run_until(settings: Settings, stop: CancellationToken) -> Result<(), String> {
    let logs = settings.dir.join("archives");
    std::fs::create_dir_all(&logs).map_err(|e| format!("{}: {e}", logs.display()))?;
    if settings.agents.iter().any(|a| a.program == Program::T3Code) {
        let t3 = settings
            .t3code
            .as_ref()
            .ok_or("a t3code agent needs [t3code] settings")?;
        let mut archive = crate::archive::Archive::create(&logs)?;
        let client = t3.connect().await?;
        for spec in (settings.agents.iter()).filter(|a| a.program == Program::T3Code) {
            crate::t3::validate(t3, &client, spec, &mut archive).await?;
        }
        client.close().await;
    }
    // What the seats report to the server, kept while it is away.
    let (report, mut reports) = unbounded_channel();
    let (offered, mut offers) = tokio::sync::watch::channel(vec![]);
    let discovery = tokio::spawn(discover(settings.clone(), offered, stop.clone()));
    tokio::select! {
        _ = stop.cancelled() => return Ok(()),
        _ = offers.changed() => {},
    }
    // Every agent offered since it started, by name: a game goes on with one no longer offered.
    let mut known: HashMap<String, AgentSpec> = HashMap::new();
    let mut playing: HashMap<Key, Playing> = HashMap::new();
    let mut stopping: Vec<JoinHandle<()>> = vec![];
    let mut said_unreachable = false;
    let connected = async {
        loop {
            let stream = match UnixStream::connect(&settings.socket).await {
                Ok(stream) => stream,
                Err(e) => {
                    if !said_unreachable {
                        eprintln!(
                            "dominion agents: {}: {e}; retrying",
                            settings.socket.display()
                        );
                        said_unreachable = true;
                    }
                    tokio::time::sleep(RECONNECT).await;
                    continue;
                }
            };
            said_unreachable = false;
            eprintln!(
                "dominion agents: connected to {}",
                settings.socket.display()
            );
            let (read, mut write) = stream.into_split();
            let agents = remember(&mut known, &offers.borrow_and_update());
            let hello = FromRunner::Hello {
                agents,
                capacity: settings.capacity,
            };
            if send(&mut write, &hello).await.is_err() {
                tokio::time::sleep(RECONNECT).await;
                continue;
            }
            let mut lines = BufReader::new(read).lines();
            // The jobs the server told last: an agent offered later may play one it skipped.
            let mut told: Vec<Job> = vec![];
            loop {
                tokio::select! {
                    line = lines.next_line() => {
                        let Ok(Some(line)) = line else { break };
                        match serde_json::from_str::<ToRunner>(&line) {
                            Ok(ToRunner::Jobs { jobs }) => {
                                told = jobs;
                                stopping.retain(|task| !task.is_finished());
                                stopping.extend(reconcile(&settings, &known, &mut playing, told.clone(), &logs, &report));
                            }
                            Ok(ToRunner::Refused { agent, reason }) => {
                                eprintln!("dominion agents: the server refused {agent}: {reason}");
                            }
                            Err(_) => eprintln!("dominion agents: the server said {line}"),
                        }
                    }
                    changed = offers.changed() => {
                        if changed.is_err() {
                            break;
                        }
                        let agents = remember(&mut known, &offers.borrow_and_update());
                        stopping.extend(reconcile(&settings, &known, &mut playing, told.clone(), &logs, &report));
                        if send(&mut write, &FromRunner::Offers { agents }).await.is_err() {
                            break;
                        }
                    }
                    Some(message) = reports.recv() => {
                        if send(&mut write, &message).await.is_err() {
                            break;
                        }
                    }
                }
            }
            eprintln!("dominion agents: lost the server; reconnecting");
            tokio::time::sleep(RECONNECT).await;
        }
    };
    tokio::select! {
        _ = stop.cancelled() => {},
        () = connected => {},
    }
    discovery.abort();
    for p in playing.values() {
        p.stop.cancel();
    }
    for task in playing.into_values().map(|p| p.task).chain(stopping) {
        let _ = task.await;
    }
    Ok(())
}

async fn send(
    write: &mut tokio::net::unix::OwnedWriteHalf,
    message: &FromRunner,
) -> std::io::Result<()> {
    let mut line = serde_json::to_string(message).expect("serializable");
    line.push('\n');
    write.write_all(line.as_bytes()).await
}

/// Keeps the agents of `offered` among those `known`; their offers.
fn remember(known: &mut HashMap<String, AgentSpec>, offered: &[(AgentSpec, Offer)]) -> Vec<Offer> {
    for (spec, _) in offered {
        known.insert(spec.name.clone(), spec.clone());
    }
    offered.iter().map(|(_, offer)| offer.clone()).collect()
}

/// Reads the providers' catalogs now and every [`REDISCOVER`], and publishes what the
/// runner offers when it changes. A catalog that can't be read keeps what it last listed.
async fn discover(
    settings: Settings,
    offered: tokio::sync::watch::Sender<Vec<(AgentSpec, Offer)>>,
    stop: CancellationToken,
) {
    let mut listed: Vec<Option<Vec<crate::catalog::Found>>> = vec![None; settings.catalogs.len()];
    let mut first = true;
    loop {
        let catalogs = settings.catalogs.clone();
        let read = tokio::task::spawn_blocking(move || {
            catalogs.iter().map(|c| c.discover()).collect::<Vec<_>>()
        })
        .await
        .unwrap_or_default();
        for ((catalog, found), last) in settings.catalogs.iter().zip(read).zip(&mut listed) {
            match found {
                Ok(found) => *last = Some(found),
                Err(e) => eprintln!("dominion agents: {:?} models: {e}", catalog.program),
            }
        }
        let catalogs: Vec<_> = (settings.catalogs.iter().cloned())
            .zip(listed.iter().map(|l| l.clone().unwrap_or_default()))
            .collect();
        let offers = crate::catalog::offers(&settings.agents, &catalogs);
        offered.send_if_modified(|old| {
            let same = old.len() == offers.len()
                && old
                    .iter()
                    .zip(&offers)
                    .all(|((a, x), (b, y))| a == b && x == y);
            if !same {
                *old = offers;
            }
            !same || std::mem::take(&mut first)
        });
        tokio::select! {
            _ = stop.cancelled() => return,
            () = tokio::time::sleep(REDISCOVER) => {},
        }
    }
}

/// Starts and stops seats to match `jobs`; returns the tasks of the seats it stopped,
/// which may still be finishing.
fn reconcile(
    settings: &Settings,
    known: &HashMap<String, AgentSpec>,
    playing: &mut HashMap<Key, Playing>,
    jobs: Vec<Job>,
    logs: &Path,
    report: &UnboundedSender<FromRunner>,
) -> Vec<JoinHandle<()>> {
    let wanted: HashMap<Key, Job> = (jobs.into_iter())
        .map(|j| ((j.game_id.clone(), j.seat), j))
        .collect();
    let stopped: Vec<Key> = (playing.iter())
        .filter(|(key, p)| wanted.get(*key) != Some(&p.job) || p.stop.is_cancelled())
        .map(|(key, _)| key.clone())
        .collect();
    let stopped: Vec<JoinHandle<()>> = (stopped.iter())
        .filter_map(|key| playing.remove(key))
        .map(|p| {
            p.stop.cancel();
            p.task
        })
        .collect();
    for (key, job) in wanted {
        if playing.contains_key(&key) {
            continue;
        }
        let Some(spec) = known.get(&job.agent) else {
            eprintln!(
                "dominion agents: no agent {} here for game {}",
                job.agent, job.game_id
            );
            continue;
        };
        eprintln!(
            "dominion agents: {} plays seat {} of game {}",
            spec.name, job.seat, job.game_id
        );
        let stop = CancellationToken::new();
        let log = logs.to_path_buf();
        let task = tokio::spawn(keep_playing(
            spec.clone(),
            job.clone(),
            settings.clone(),
            log,
            stop.clone(),
            report.clone(),
        ));
        playing.insert(key, Playing { job, stop, task });
    }
    stopped
}

async fn keep_playing(
    spec: AgentSpec,
    job: Job,
    settings: Settings,
    logs: PathBuf,
    stop: CancellationToken,
    report: UnboundedSender<FromRunner>,
) {
    let Settings {
        server,
        t3code,
        analysis,
        sandbox,
        clock_guard,
        dir,
        ..
    } = settings;
    let notices = clock_guard::Notices::default();
    let guarding = stop.child_token();
    let _guarding = guarding.clone().drop_guard();
    tokio::spawn(clock_guard::run(
        clock_guard,
        server.clone(),
        job.clone(),
        notices.clone(),
        dir.join("clock-guard.jsonl"),
        guarding,
    ));
    let max_retry = match job.official {
        true => OFFICIAL_MAX_RETRY,
        false => MAX_RETRY,
    };
    let mut retry = RECONNECT;
    while !stop.is_cancelled() {
        let mut archived = String::new();
        let started = std::time::Instant::now();
        let result = async {
            let mut archive = crate::archive::Archive::create(&logs)?;
            eprintln!(
                "dominion agents: {} in {}: archive {}",
                spec.name,
                job.game_id,
                archive.dir.display()
            );
            let setup = setup(&spec, &sandbox, job.options).await;
            archive.record("setup", serde_json::to_value(&setup).expect("serializable"))?;
            archived = (archive.dir.file_name())
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            if !job.official {
                let _ = report.send(FromRunner::Attempt {
                    game_id: job.game_id.clone(),
                    seat: job.seat,
                    archive: archived.clone(),
                    setup: Box::new(setup),
                });
            }
            let cancelled = CancellationToken::new();
            let play = async {
                match (spec.program, &t3code) {
                    (Program::T3Code, None) => Err("a t3code agent needs [t3code] settings".into()),
                    (Program::T3Code, Some(t3)) => {
                        crate::t3::play(t3, &spec, &job, &server, &mut archive, cancelled.clone())
                            .await
                    }
                    _ => {
                        provider::play_with_plan(
                            analysis.as_ref(),
                            &spec,
                            &sandbox,
                            &job,
                            &server,
                            &mut archive,
                            cancelled.clone(),
                            notices.clone(),
                        )
                        .await
                    }
                }
            };
            tokio::pin!(play);
            tokio::select! {
                result = &mut play => result,
                _ = stop.cancelled() => match tokio::time::timeout(GRACE, &mut play).await {
                    Ok(result) => result,
                    Err(_) => { cancelled.cancel(); play.await }
                }
            }
        }
        .await;
        let Err(error) = result else { return };
        if stop.is_cancelled() {
            return;
        }
        if !job.official && !archived.is_empty() {
            let _ = report.send(FromRunner::Failed {
                game_id: job.game_id.clone(),
                seat: job.seat,
                archive: archived,
                error: error.clone(),
            });
        }
        // Only a seat that broke its sandbox's rules is stopped. Anything else (a provider
        // error, the server away, a model that ended its conversation early) is retried
        // for as long as the game wants the seat: one that never recovers forfeits by the
        // server's timing, which ends the job.
        if error.contains("native harness attempted a forbidden tool")
            || error.contains("native harness exposed forbidden tools")
        {
            eprintln!(
                "dominion agents: {} in {}: {error}; stopped",
                spec.name, job.game_id
            );
            return;
        }
        eprintln!(
            "dominion agents: {} in {}: {error}; retrying in {} seconds",
            spec.name,
            job.game_id,
            retry.as_secs()
        );
        tokio::select! {
            _ = stop.cancelled() => return,
            () = tokio::time::sleep(retry) => {},
        }
        retry = match started.elapsed() > SETTLED {
            true => RECONNECT,
            false => (retry * 2).min(max_retry),
        };
    }
}

/// What plays a seat: the agent's profile, this build, and the playbook it is given.
async fn setup(
    spec: &AgentSpec,
    sandbox: &crate::harness::Sandbox,
    options: dominion_protocol::lobby::LaunchOptions,
) -> AgentSetup {
    let dir = sandbox
        .playbook
        .clone()
        .filter(|_| spec.program != Program::T3Code);
    let playbook = match dir {
        None => None,
        Some(dir) => tokio::task::spawn_blocking(move || crate::harness::playbook(&dir))
            .await
            .map_err(|e| e.to_string())
            .and_then(|r| r)
            .inspect_err(|e| eprintln!("dominion agents: {e}"))
            .ok(),
    };
    AgentSetup {
        agent: spec.name.clone(),
        runner_strategy: options.strategy,
        speed: options.speed.or(spec
            .fast_mode
            .then_some(dominion_protocol::lobby::InferenceSpeed::Fast)),
        program: spec.program.label().into(),
        model: spec.model.clone(),
        model_name: None,
        effort: spec.effort.clone(),
        version: crate::commit(),
        playbook,
    }
}
