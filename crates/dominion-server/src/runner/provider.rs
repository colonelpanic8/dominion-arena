//! The two provider roles used by a playing strategy. Only the player receives seat tools.
use super::analyst::Advice;
use super::strategy::ModelRunnerStrategy;
use super::{AgentSpec, Program};
use crate::archive::Archive;
use crate::client::Client;
use crate::harness::{self, Sandbox, bridge::Bridge, process::Harness};
use crate::house::Job;
use serde_json::json;
use std::{path::Path, sync::Arc};
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

#[derive(Clone, Debug)]
pub struct AnalysisPlan {
    pub model: AgentSpec,
    pub target_seconds: u32,
    pub background: bool,
}

impl AnalysisPlan {
    fn from_legacy_profile(spec: &AgentSpec) -> Option<Self> {
        let config = spec.opening_analyst.as_ref()?;
        let mut model = spec.clone();
        model.model = Some(config.model.clone());
        model.effort = Some(config.effort.clone());
        Some(Self {
            model,
            target_seconds: config.target_seconds,
            background: config.background,
        })
    }
}

#[derive(Clone)]
pub(super) struct AnalysisModel {
    spec: AgentSpec,
    sandbox: Sandbox,
}

impl AnalysisModel {
    pub fn new(spec: &AgentSpec, sandbox: &Sandbox) -> Self {
        Self {
            spec: spec.clone(),
            sandbox: sandbox.clone(),
        }
    }

    pub fn for_model(&self, spec: &AgentSpec) -> Self {
        Self::new(spec, &self.sandbox)
    }

    pub async fn analyze(
        &self,
        dir: &Path,
        brief: &str,
        position: &str,
        deadline: Instant,
    ) -> Result<String, String> {
        let mut log = Archive::open(dir)?;
        let mut spec = self.spec.clone();
        spec.instructions = None;
        let seconds = deadline
            .saturating_duration_since(Instant::now())
            .as_secs()
            .max(1);
        spec.response_seconds = Some(
            spec.response_seconds
                .unwrap_or(u32::MAX)
                .min(seconds as u32),
        );
        let mut brief = brief.to_owned();
        harness::append_live_guidance(&mut brief, self.sandbox.playbook.as_deref())?;
        log.record("analysis_request", json!({"program":spec.program,"model":spec.model,"instructions":brief,"position":position}))?;
        let result = match spec.program {
            Program::OpenAi => crate::openai::analyze(&spec, &brief, position, &mut log).await,
            Program::Anthropic => {
                crate::anthropic::analyze(&spec, &brief, position, &mut log).await
            }
            Program::T3Code => Err("T3 Code does not offer isolated analysis".into()),
            Program::Claude | Program::Codex | Program::Antigravity => {
                let mut sandbox = self.sandbox.clone();
                sandbox.playbook = None;
                let mut session = Harness::new(&spec, &sandbox, dir)?;
                session.set_brief(&spec, brief)?;
                session.reply(&spec, position, &mut log).await
            }
        }?;
        log.record("analysis_response", json!({"text":result}))?;
        Ok(result)
    }
}

pub(crate) struct Seat<'a> {
    pub spec: &'a AgentSpec,
    pub sandbox: &'a Sandbox,
    pub job: &'a Job,
    pub client: Arc<Client>,
}

pub(crate) struct PlayerOpening {
    pub message: String,
    pub guidance: String,
    pub clocked: bool,
    pub advice: Advice,
}

pub(super) enum PlayerProvider {
    Native(Box<Harness>),
    OpenAi,
    Anthropic,
}

impl PlayerProvider {
    pub fn native(&self) -> bool {
        matches!(self, Self::Native(_))
    }
}

pub(super) trait PlayerAdapter {
    fn opening_message(&self, job: &Job, spec: &AgentSpec) -> String;
    async fn play_player(
        &mut self,
        seat: &Seat<'_>,
        archive: &mut Archive,
        opening: PlayerOpening,
    ) -> Result<Result<String, String>, String>;
}

impl PlayerAdapter for PlayerProvider {
    fn opening_message(&self, job: &Job, spec: &AgentSpec) -> String {
        let mut message = harness::opening(job, self.native());
        if !self.native()
            && let Some(text) = spec
                .instructions
                .as_deref()
                .filter(|t| !t.trim().is_empty())
        {
            message.push_str(&format!("Player instructions:\n{}\n\n", text.trim()));
        }
        message
    }

    async fn play_player(
        &mut self,
        seat: &Seat<'_>,
        archive: &mut Archive,
        opening: PlayerOpening,
    ) -> Result<Result<String, String>, String> {
        match self {
            Self::OpenAi => crate::openai::play_player(seat, archive, opening).await,
            Self::Anthropic => crate::anthropic::play_player(seat, archive, opening).await,
            Self::Native(session) => {
                let mut brief = harness::instructions(session.playbook(), opening.clocked);
                brief.push_str(&opening.guidance);
                if opening.clocked {
                    harness::append_live_guidance(&mut brief, seat.sandbox.playbook.as_deref())?;
                }
                session.set_brief(seat.spec, brief)?;
                let mut bridge = Bridge::start(
                    seat.client.clone(),
                    seat.job,
                    Some(&session.bridge_socket()),
                    opening.advice,
                )
                .await?;
                session.enable_mcp(
                    seat.spec,
                    &bridge.url,
                    bridge.records.take().expect("new bridge"),
                )?;
                let answer = session.reply(seat.spec, &opening.message, archive).await?;
                harness::session_end(&seat.client, archive, seat.job).await?;
                Ok(Ok(answer))
            }
        }
    }
}

pub async fn play(
    spec: &AgentSpec,
    sandbox: &Sandbox,
    job: &Job,
    server: &str,
    archive: &mut Archive,
    stop: CancellationToken,
) -> Result<(), String> {
    play_with_plan(None, spec, sandbox, job, server, archive, stop).await
}

pub(crate) async fn play_with_plan(
    plan: Option<&AnalysisPlan>,
    spec: &AgentSpec,
    sandbox: &Sandbox,
    job: &Job,
    server: &str,
    archive: &mut Archive,
    stop: CancellationToken,
) -> Result<(), String> {
    let launched = spec.for_launch(job.options)?;
    let spec = &launched;
    spec.validate()?;
    archive.publish(server, job)?;
    archive.record("seat", json!({"agent":spec,"job":job,"server":server}))?;
    let endpoint = harness::endpoint(server, job)?;
    let client =
        match Client::connect_authenticated(&endpoint, job.official.then(|| job.token.clone()))
            .await
        {
            Ok(client) => Arc::new(client.with_clock_cues(job.options.strategy.clock_cues())),
            Err(error) => {
                archive.record("seat_finished", json!({"result":{"Err":error}}))?;
                archive.finish_publish().await;
                return Err(error);
            }
        };
    let run = async {
        let mut provider = match spec.program {
            Program::OpenAi => PlayerProvider::OpenAi,
            Program::Anthropic => PlayerProvider::Anthropic,
            Program::Claude | Program::Codex | Program::Antigravity => {
                PlayerProvider::Native(Box::new(Harness::new(spec, sandbox, &archive.dir)?))
            }
            Program::T3Code => return Err("T3 Code requires its conversation adapter".into()),
        };
        if provider.native() && job.options.strategy.is_classic() && !job.official {
            // Prime the common tool stream before the classic grouped card reference.
            let bound = client
                .call("join_game", json!({"game":job.game_id,"token":job.token}))
                .await
                .map_err(|e| e.to_string())?;
            if !bound.ok {
                return Err(bound.text);
            }
        }
        let seat = Seat {
            spec,
            sandbox,
            job,
            client: client.clone(),
        };
        let legacy = AnalysisPlan::from_legacy_profile(spec);
        let plan = plan
            .or(legacy.as_ref())
            .filter(|_| !job.options.strategy.is_classic());
        super::strategy::run(&mut provider, &seat, plan, archive).await
    };
    let result = tokio::select! { biased;
        _ = stop.cancelled() => Err("seat runner cancelled".into()),
        result = run => result,
    };
    harness::record_final_result(&client, archive, job).await?;
    let result = match result {
        Ok(answer) => {
            harness::debrief(&client, archive, job, answer).await;
            Ok(())
        }
        Err(error) => Err(error),
    };
    archive.record("seat_finished", json!({"result":result}))?;
    if let Ok(client) = Arc::try_unwrap(client) {
        client.close().await;
    }
    archive.finish_publish().await;
    result
}
