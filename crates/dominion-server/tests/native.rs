#![cfg(target_os = "linux")]
use dominion_server::arena::{Arena, Ident};
use dominion_server::config::Config;
use dominion_server::house::{Job, Offer, ToRunner};
use dominion_server::hub::{Hub, NewGame};
use dominion_server::{archive::Archive, harness};
use rmcp::transport::streamable_http_server::{
    StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
};
use serde_json::Value;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

const FIXTURE: &str = r#"
import json, sys, os, re, time
from pathlib import Path
assert not Path('/home/imalison').exists()
assert not Path('/srv/dotfiles').exists()
assert 'ANTHROPIC_API_KEY' not in os.environ
import socket
try:
    socket.create_connection(('1.1.1.1', 53), timeout=2)
    raise AssertionError('the sandbox reached the network')
except OSError:
    pass
assert os.environ['HTTPS_PROXY'] == 'http://127.0.0.1:3128'
prompt = sys.stdin.read()
if "--input-format" in sys.argv: prompt = json.loads(prompt)["message"]["content"]
assert 'test-credential' not in prompt
assert '=== CARD & PILE REFERENCE ===' in prompt
assert 'Village | Action | $3' in prompt
if 'fixture:brief' in prompt:
    assert "BRIEF from the game's creator" in prompt
    assert 'your final reply is your debrief' in prompt
if 'fixture:transient' in prompt:
    # The token is the only file mounted from the host, so a mark on it outlives the attempt.
    token = Path('/home/agent/.gemini/antigravity-cli/antigravity-oauth-token')
    if 'failed' not in token.read_text():
        token.write_text(token.read_text() + ' failed')
        print(json.dumps({'event':'result', 'result':{'status':'ERROR', 'response':'', 'error':'Provider temporarily unavailable'}}), flush=True)
        sys.exit(1)
if 'fixture:streaming' in prompt:
    print(json.dumps({'type':'reasoning','text':'streaming reasoning'}), flush=True)
    time.sleep(2)
if 'fixture:timeout' in prompt:
    print(json.dumps({'type':'reasoning','text':'before timeout'}), flush=True)
    time.sleep(20)
if 'fixture:native-tool' in prompt:
    print(json.dumps({'type':'item.completed','item':{'type':'command_execution','command':'not executed'}}), flush=True)
    sys.exit(0)
if 'fixture:unsupported' in prompt:
    print(json.dumps({'type':'result','is_error':False,'result':json.dumps({'tool':'resign'})}), flush=True)
    sys.exit(0)
if Path('/work/seat-mcp.json').exists():
    import urllib.request
    url = json.loads(Path('/work/seat-mcp.json').read_text())['url']
    headers = {'Content-Type':'application/json', 'Accept':'application/json, text/event-stream'}
    next_identity = 0
    def rpc(method, params, identity=1):
        global next_identity
        body = {'jsonrpc':'2.0','method':method,'params':params}
        if identity is not None:
            next_identity += 1
            body['id'] = next_identity
        req = urllib.request.Request(url,json.dumps(body).encode(),headers)
        with urllib.request.urlopen(req,timeout=10) as response:
            if response.headers.get('Mcp-Session-Id'): headers['Mcp-Session-Id']=response.headers['Mcp-Session-Id']
            if identity is None: return None
            if 'text/event-stream' in response.headers.get('Content-Type',''):
                for line in response:
                    if line.startswith(b'data:') and line[5:].strip(): return json.loads(line[5:])
            return json.load(response)
    rpc('initialize',{'protocolVersion':'2025-03-26','capabilities':{},'clientInfo':{'name':'fixture','version':'1'}})
    rpc('notifications/initialized',{},None)
    tools = rpc('tools/list',{})['result']['tools']
    assert sorted(t['name'] for t in tools)==['cards','log','play_treasures','state','step','turns','wait']
    assert all('token' not in t['inputSchema'].get('properties',{}) for t in tools)
    assert 'error' in rpc('tools/call',{'name':'create_game','arguments':{'players':2}})
    assert 'error' in rpc('tools/call',{'name':'view','arguments':{'token':'another-seat'}})
    def call(tool, args):
        result = rpc('tools/call',{'name':tool,'arguments':args})['result']
        assert not result.get('isError',False), result
        assert 'structuredContent' not in result, result
        return result['content'][0]['text']
    call('cards',{'names':['Village']})
    call('log',{'limit':2})
    text = call('state',{})
    while 'GAME OVER' not in text:
        d = re.search(r'^DECISION (d\d+):', text, re.M)
        if d is None:
            text = call('wait',{'timeout_s':2})
            continue
        if 'fixture:activity' in prompt:
            for _ in range(4):
                print(json.dumps({'type':'reasoning','text':'considering move'}),flush=True)
                time.sleep(.3)
        opts = re.findall(r'^ +(\d+): (.+)$', text[d.start():], re.M)
        index, label = next((o for o in opts if 'End your Buy phase' in o[1]), opts[0])
        text = call('step',{'decision':d.group(1),'indices':[int(index)],'expect':[label],'timeout_s':2})
    action = {'message':'Game completed through MCP'}
answer = json.dumps(action)
print(json.dumps({'type':'reasoning', 'text':'r' * 120003}))
print('native diagnostic', file=sys.stderr)
if Path('/playbook').exists():
    # A playbook seat starts in it, read-only, and may run its lookups.
    assert os.getcwd() == '/playbook'
    assert 'Rules here.' in Path('context/rules.md').read_text()
    try:
        Path('note').write_text('x')
        raise AssertionError('the playbook is writable')
    except OSError:
        pass
    import subprocess
    out = subprocess.run([sys.executable, 'data/card-ratings.py', 'Village'], capture_output=True, text=True)
    assert out.stdout.strip() == 'Village 1.0', out
    assert 'Rules here.' not in prompt
    assert 'Plan first.' not in prompt
if '--setting-sources' in sys.argv and sys.argv[sys.argv.index('--setting-sources')+1] == 'project':
    assert '--restricted' not in sys.argv
    tools = sys.argv[sys.argv.index('--tools')+1].split(',')
    assert tools == ['Bash','Read','Grep','Glob','Skill'], tools
    print(json.dumps({'type':'system','subtype':'init','tools':tools+['mcp__dominion__step'], 'mcp_servers':[]}))
    print(json.dumps({'type':'assistant','message':{'content':[{'type':'tool_use','name':'Bash','input':{'command':'python3 data/card-ratings.py Village'}}]}}))
    print(json.dumps({'type':'result', 'is_error':False, 'result':answer}))
elif '--restricted' in sys.argv:
    assert sys.argv[sys.argv.index('--tools')+1] == ''
    print(json.dumps({'type':'system','subtype':'init','tools':[], 'mcp_servers':[]}))
    print(json.dumps({'type':'result', 'is_error':False, 'result':answer}))
elif '--agent' in sys.argv:
    agent = Path('/home/agent/.gemini/config/agents/dominion.md').read_text()
    permissions = json.loads(Path('/work/settings.json').read_text())['permissions']
    print(json.dumps({'event':'init','init':{'tools':[]}}))
    if Path('/playbook').exists():
        assert 'tools: [read_file, run_command, finish]' in agent
        assert 'read_file(/playbook)' in permissions['allow']
        assert 'command(python3 data/card-ratings.py)' in permissions['allow']
        assert not any(r.startswith(('read_file', 'command')) for r in permissions['deny'])
        for tool, args in [('read_file', {'Path':'/playbook/context/rules.md'}), ('run_command', {'CommandLine':'python3 data/card-ratings.py Village'})]:
            for state in ['ACTIVE', 'DONE']:
                print(json.dumps({'event':'step_update','step_update':{'state':state,'step_type':'tool','tool_name':tool,'tool_info':{'name':tool,'parameters':args}}}))
    else:
        assert 'tools: [finish]' in agent
        assert 'command(*)' in permissions['deny']
    print(json.dumps({'event':'result', 'result':{'status':'SUCCESS','response':answer}}))
else:
    assert Path('/home/agent/.codex/auth.json').read_text() == '{}'
    assert sorted(os.listdir('/home/agent/.codex')) == ['auth.json']
    assert '--ignore-user-config' in sys.argv
    assert '--disable' in sys.argv
    shell = sys.argv[sys.argv.index('--disable')+1] == 'shell_tool'
    assert shell != Path('/playbook').exists()
    if not shell:
        print(json.dumps({'type':'item.completed','item':{'type':'command_execution','command':'python3 data/card-ratings.py Village'}}))
    print(json.dumps({'type':'item.completed','item':{'type':'agent_message','text':answer}}))
    print(json.dumps({'type':'turn.completed'}))
"#;

struct Harness {
    config: Config,
    arena: Arc<Arena>,
    who: Ident,
    token: String,
    jobs: tokio::sync::mpsc::UnboundedReceiver<ToRunner>,
    stop: CancellationToken,
    sandbox: harness::Sandbox,
    _dir: tempfile::TempDir,
}

async fn setup(program: &str) -> Harness {
    let dir = tempfile::tempdir().unwrap();
    let stop = CancellationToken::new();
    let hub = Arc::new(Hub::default());
    let arena = Arena::open(hub, Default::default()).unwrap();
    let (_, token) = arena
        .register(
            "127.0.0.1".parse().unwrap(),
            &dominion_protocol::lobby::Credentials {
                name: "Host".into(),
                password: "correct horse".into(),
            },
            None,
        )
        .unwrap();
    let token = token.unwrap();
    let who = arena.identify(&token).unwrap();
    let (send, jobs) = tokio::sync::mpsc::unbounded_channel();
    arena.runner_connected(
        vec![Offer {
            name: "Test".into(),
            about: "T3 Code".into(),
            ..Default::default()
        }],
        2,
        send,
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let server = format!("http://{}/mcp", listener.local_addr().unwrap());
    let sessions = Arc::new(LocalSessionManager::default());
    let served = arena.clone();
    let service = StreamableHttpService::new(
        move || {
            Ok(dominion_server::mcp::Session::new(
                served.clone(),
                std::time::Duration::from_secs(600),
                false,
            ))
        },
        sessions,
        StreamableHttpServerConfig::default(),
    );
    let router = dominion_server::http::router(
        arena.clone(),
        &Default::default(),
        vec!["127.0.0.1".into()],
        stop.clone(),
    )
    .nest_service("/mcp", service);
    let stopped = stop.clone();
    tokio::spawn(async move {
        axum::serve(listener, router)
            .with_graceful_shutdown(stopped.cancelled_owned())
            .await
            .unwrap();
    });
    let auth = dir.path().join("auth");
    std::fs::create_dir_all(auth.join("antigravity-cli")).unwrap();
    std::fs::write(
        auth.join("antigravity-cli/antigravity-oauth-token"),
        "token",
    )
    .unwrap();
    std::fs::write(auth.join("auth.json"), "{}").unwrap();
    std::fs::write(auth.join("guidance"), "host instructions").unwrap();
    std::os::unix::fs::symlink("guidance", auth.join("AGENTS.md")).unwrap();
    std::fs::write(
        auth.join("models_cache.json"),
        r#"{"models":[{"slug":"fixture","visibility":"list","priority":0}]}"#,
    )
    .unwrap();
    let command = dir.path().join("fixture");
    let python = std::process::Command::new("which")
        .arg("python3")
        .output()
        .unwrap();
    let python = std::fs::canonicalize(String::from_utf8(python.stdout).unwrap().trim()).unwrap();
    let source = format!("#!{}\n{}", python.display(), FIXTURE);
    std::fs::write(&command, source).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&command, std::fs::Permissions::from_mode(0o700)).unwrap();
    let config: Config = toml::from_str(&format!(
        r#"
        server = {server:?}
        agents_dir = {:?}
        [[agents]]
        name = "Test"
        program = {program:?}
        command = {:?}
        auth_dir = {:?}
        instructions = "Buy Provinces."
        response_seconds = 5
    "#,
        dir.path().join("agents"),
        command,
        auth
    ))
    .unwrap();
    Harness {
        config,
        arena,
        who,
        token,
        jobs,
        stop,
        sandbox: sandbox(),
        _dir: dir,
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        self.stop.cancel();
    }
}

/// The sandbox the tests run seats in: this package's `dominion` forwards its ports.
fn sandbox() -> harness::Sandbox {
    harness::Sandbox {
        forwarder: Some(env!("CARGO_BIN_EXE_dominion").into()),
        ..Default::default()
    }
}

fn request() -> NewGame {
    NewGame {
        players: 2,
        agents: vec![Some("Test".into()), Some("Test".into())],
        kingdom: vec!["Village".into()],
        seed: Some(42),
        turn_limit: Some(2),
        ..NewGame::default()
    }
}

fn dispatched(jobs: &mut tokio::sync::mpsc::UnboundedReceiver<ToRunner>) -> Vec<Job> {
    let mut result = vec![];
    while let Ok(message) = jobs.try_recv() {
        if let ToRunner::Jobs { jobs } = message {
            result = jobs;
        }
    }
    result
}

async fn play(h: &mut Harness, jobs: Vec<Job>) -> (std::path::PathBuf, Vec<Result<(), String>>) {
    let root = h._dir.path().join("archive");
    let mut tasks = tokio::task::JoinSet::new();
    for job in jobs {
        let mut archive = Archive::open(&root.join(format!("seat-{}", job.seat))).unwrap();

        let spec = h.config.agents.as_ref().unwrap()[0].clone();
        let server = h.config.server();
        let sandbox = h.sandbox.clone();
        tasks.spawn(async move {
            let result = harness::play(
                &spec,
                &sandbox,
                &job,
                &server,
                &mut archive,
                CancellationToken::new(),
            )
            .await;
            if let Err(error) = &result {
                eprintln!("Seat {} failed: {error}", job.seat);
                for entry in std::fs::read_dir(&archive.dir).unwrap().flatten() {
                    if entry.path().extension().is_some_and(|x| x == "stderr") {
                        eprintln!("{}", std::fs::read_to_string(entry.path()).unwrap());
                    }
                }
            }
            result
        });
    }
    let mut results = vec![];
    while let Some(result) = tasks.join_next().await {
        results.push(result.unwrap());
    }
    (root, results)
}

fn records(path: &std::path::Path) -> Vec<Value> {
    std::fs::read_to_string(path.join("interactions.jsonl"))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

#[tokio::test]
#[ignore = "requires bubblewrap and Linux user namespaces"]
async fn playbook_seats_start_in_it_read_only_and_may_run_its_lookups() {
    for program in ["claude", "codex", "antigravity"] {
        let mut h = setup(program).await;
        let playbook = h._dir.path().join("playbook");
        for (path, text) in [
            ("AGENTS.md", "Read @context/rules.md\n"),
            ("context/rules.md", "Rules here."),
            ("skills/play-game/SKILL.md", "Plan first."),
            (
                "data/card-ratings.py",
                "import sys\nprint(sys.argv[1], '1.0')\n",
            ),
        ] {
            let path = playbook.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
        h.sandbox.playbook = Some(playbook);
        let mut game = request();
        game.turn_limit = Some(4);
        h.arena.create_game(Some(&h.who), &game).unwrap();
        let jobs = dispatched(&mut h.jobs);
        let (_, results) =
            tokio::time::timeout(std::time::Duration::from_secs(30), play(&mut h, jobs))
                .await
                .unwrap();
        assert!(results.iter().all(Result::is_ok), "{program}: {results:?}");
    }
}

#[tokio::test]
#[ignore = "requires bubblewrap and Linux user namespaces"]
async fn a_native_seat_reads_its_brief_and_its_final_reply_is_its_debrief() {
    let mut h = setup("claude").await;
    let mut game = request();
    game.briefs = vec![Some("fixture:brief Play Villages.".into()), None];
    game.debrief = Some("What decided the game?".into());
    let created = h.arena.create_game(Some(&h.who), &game).unwrap();
    let jobs = dispatched(&mut h.jobs);
    assert_eq!(
        jobs[0].brief.as_deref(),
        Some("fixture:brief Play Villages.")
    );
    assert_eq!(jobs[1].brief, None);
    let (_, results) = tokio::time::timeout(std::time::Duration::from_secs(30), play(&mut h, jobs))
        .await
        .unwrap();
    assert!(results.iter().all(Result::is_ok), "{results:?}");
    let report = h.arena.hub.report(&created.game_id).unwrap();
    assert!(report.settled);
    for debrief in &report.meta.debriefs {
        assert!(
            debrief
                .as_deref()
                .is_some_and(|d| d.contains("Game completed through MCP")),
            "{debrief:?}"
        );
    }
    h.arena.sweep();
    assert!(dispatched(&mut h.jobs).is_empty());
}

#[tokio::test]
#[ignore = "requires bubblewrap and Linux user namespaces"]
async fn all_native_adapters_play_watchable_games_and_preserve_complete_logs() {
    for program in ["claude", "codex", "antigravity"] {
        let mut h = setup(program).await;
        let mut game = request();
        game.turn_limit = Some(6);
        let created = h.arena.create_game(Some(&h.who), &game).unwrap();
        assert!(created.seats.is_empty());
        let jobs = dispatched(&mut h.jobs);
        assert_eq!(jobs.len(), 2);
        assert_eq!(jobs[0].configuration.as_ref().unwrap().seed, 42);
        assert!(
            h.arena
                .hub
                .watch_view(&created.game_id, 0, None)
                .unwrap()
                .view
                .spectating
        );
        let (root, results) =
            tokio::time::timeout(std::time::Duration::from_secs(30), play(&mut h, jobs))
                .await
                .unwrap();
        assert!(results.iter().all(Result::is_ok), "{program}: {results:?}");
        assert!(!h.arena.hub.list()[0].running);
        assert!(h.arena.hub.list()[0].watchable);
        let mut cursor = 0;
        let mut central = vec![];
        loop {
            let page = h.arena.transcript(&created.game_id, cursor).unwrap();
            cursor = page.next;
            central.extend(page.entries);
            if !page.more {
                break;
            }
        }
        assert!(
            central.iter().any(
                |e| e.activity.kind == dominion_protocol::transcript::TranscriptKind::Reasoning
            )
        );
        assert!(
            central
                .iter()
                .any(|e| e.activity.kind == dominion_protocol::transcript::TranscriptKind::Action)
        );
        assert!(
            central
                .iter()
                .any(|e| e.activity.kind == dominion_protocol::transcript::TranscriptKind::Game)
        );

        for seat in 0..2 {
            let path = root.join(format!("seat-{seat}"));
            let journal = records(&path);
            assert_eq!(
                journal
                    .iter()
                    .filter(|r| r["kind"] == "native_request")
                    .count(),
                1
            );
            assert!(
                journal
                    .iter()
                    .filter(|r| r["kind"] == "request" && r["data"]["tool"] == "step")
                    .count()
                    >= 3
            );
            assert!(
                !journal
                    .iter()
                    .filter(|r| r["kind"] == "native_request")
                    .any(|r| r["data"]["prompt"]
                        .as_str()
                        .unwrap()
                        .contains("Earlier interactions"))
            );

            assert!(journal.iter().any(|r| r["kind"] == "result"));
            assert!(journal.iter().any(|r| {
                r["kind"] == "native_event"
                    && r["data"]["text"]
                        .as_str()
                        .is_some_and(|s| s.len() == 120_003)
            }));
            assert!(
                journal
                    .iter()
                    .any(|r| r["kind"] == "request" && r["data"]["tool"] == "cards")
            );
            assert!(
                std::fs::read_to_string(path.join("response-000001.stderr"))
                    .unwrap()
                    .contains("native diagnostic")
            );
            assert_eq!(journal[0]["data"]["job"]["configuration"]["seed"], 42);
            let joined = journal
                .iter()
                .find(|r| r["kind"] == "request" && r["data"]["tool"] == "join_game")
                .unwrap();
            assert_eq!(joined["data"]["arguments"]["token"], "[redacted]");
        }
    }
}

#[tokio::test]
async fn http_creation_assigns_only_requested_seats_and_respects_privacy() {
    let mut h = setup("claude").await;
    let mut request = request();
    request.agents = vec![None, Some("Test".into())];
    request.watchable = Some(false);
    let base = h.config.server().trim_end_matches("/mcp").to_string();
    let response = reqwest::Client::new()
        .post(format!("{base}/api/games"))
        .bearer_auth(&h.token)
        .body(serde_json::to_string(&request).unwrap())
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::CREATED);
    let created: Value = serde_json::from_str(&response.text().await.unwrap()).unwrap();
    assert_eq!(created["seats"].as_array().unwrap().len(), 1);
    assert_eq!(created["seats"][0]["seat"], 0);
    assert_eq!(dispatched(&mut h.jobs).len(), 1);
    assert!(!h.arena.hub.list()[0].watchable);
}

#[tokio::test]
async fn creation_checks_permissions_capacity_and_seat_assignments() {
    let h = setup("claude").await;
    assert!(h.arena.create_game(None, &request()).is_err());
    let mut malformed = request();
    malformed.agents.pop();
    assert!(h.arena.create_game(Some(&h.who), &malformed).is_err());
    malformed.agents = vec![Some("Missing".into()), None];
    assert!(h.arena.create_game(Some(&h.who), &malformed).is_err());
    assert!(h.arena.hub.list().is_empty());
    h.arena.create_game(Some(&h.who), &request()).unwrap();
    assert!(h.arena.create_game(Some(&h.who), &request()).is_err());
}

#[tokio::test]
#[ignore = "requires bubblewrap and Linux user namespaces"]
async fn active_sessions_and_opponent_waits_outlive_the_inactivity_limit() {
    let mut h = setup("claude").await;
    let spec = &mut h.config.agents.as_mut().unwrap()[0];
    spec.instructions = Some("fixture:activity".into());
    spec.response_seconds = Some(1);
    let mut game = request();
    game.turn_limit = Some(6);
    h.arena.create_game(Some(&h.who), &game).unwrap();
    let jobs = dispatched(&mut h.jobs);
    let start = std::time::Instant::now();
    let (root, results) =
        tokio::time::timeout(std::time::Duration::from_secs(20), play(&mut h, jobs))
            .await
            .unwrap();
    assert!(results.iter().all(Result::is_ok), "{results:?}");
    assert!(start.elapsed() > std::time::Duration::from_secs(3));
    for seat in 0..2 {
        let journal = records(&root.join(format!("seat-{seat}")));
        assert_eq!(
            journal
                .iter()
                .filter(|r| r["kind"] == "native_request")
                .count(),
            1
        );
        assert!(!journal.iter().any(|r| r["kind"] == "native_timeout"));
    }
}

#[tokio::test]
#[ignore = "requires bubblewrap and Linux user namespaces"]
async fn the_existing_runner_dispatches_native_seats_and_archives_game_completion() {
    runner_game(false).await;
}

#[tokio::test]
#[ignore = "requires bubblewrap and Linux user namespaces"]
async fn runner_recovers_a_failed_native_turn_without_abandoning_the_seat() {
    runner_game(true).await;
}

async fn runner_game(transient_failure: bool) {
    let mut h = setup(if transient_failure {
        "antigravity"
    } else {
        "claude"
    })
    .await;
    if transient_failure {
        h.config.agents.as_mut().unwrap()[0].instructions = Some("fixture:transient".into());
    }
    let socket = h._dir.path().join("agents.sock");
    h.config.agent_socket = Some(socket.clone());
    let arena = h.arena.clone();
    let server_stop = h.stop.clone();
    let mut listening = Box::pin(async move {
        dominion_server::house::listen(arena, &socket, server_stop)
            .await
            .unwrap();
    });
    assert!(futures_util::poll!(listening.as_mut()).is_pending());
    let listener_task = tokio::spawn(listening);
    let (mut changes, _) = h.arena.subscribe();
    let runner_stop = CancellationToken::new();
    let mut settings = h.config.runner().unwrap();
    settings.sandbox = sandbox();
    let runner = tokio::spawn(dominion_server::runner::run_until(
        settings,
        runner_stop.clone(),
    ));
    tokio::time::timeout(std::time::Duration::from_secs(10), changes.changed())
        .await
        .unwrap()
        .unwrap();
    let created = h.arena.create_game(Some(&h.who), &request()).unwrap();
    let hub = h.arena.hub.clone();
    let mut game_changes = hub.subscribe(&created.game_id).unwrap().version;
    tokio::time::timeout(std::time::Duration::from_secs(30), async {
        while hub.list()[0].running {
            game_changes.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
    runner_stop.cancel();
    tokio::time::timeout(std::time::Duration::from_secs(10), runner)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let root = h.config.agents_dir.as_ref().unwrap().join("archives");
    let mut finished = 0;
    let mut failed = 0;
    for entry in std::fs::read_dir(root).unwrap() {
        let records = records(&entry.unwrap().path());
        if records.iter().any(|r| r["kind"] == "seat") {
            assert!(records.iter().any(|r| r["kind"] == "seat_finished"));
            if records
                .iter()
                .any(|r| r["kind"] == "seat_finished" && !r["data"]["result"]["Err"].is_null())
            {
                assert!(
                    !records
                        .iter()
                        .any(|r| r["kind"] == "request" && r["data"]["tool"] == "cards")
                );
                failed += 1;
            } else {
                assert!(records.iter().any(|r| r["kind"] == "result"));
                finished += 1;
            }
        }
    }
    assert_eq!(finished, 2);
    assert_eq!(failed, usize::from(transient_failure));
    listener_task.abort();
}

#[tokio::test]
#[ignore = "uses native subscription credentials and real model requests"]
async fn live_native_game() {
    let program = std::env::var("DOMINION_LIVE_AGENT").expect("DOMINION_LIVE_AGENT");
    let mut h = setup(&program).await;
    let mut spec: dominion_server::runner::AgentSpec =
        serde_json::from_value(serde_json::json!({"name":"Test","program":program})).unwrap();
    spec.command = std::env::var("DOMINION_LIVE_COMMAND").ok();
    spec.response_seconds = Some(60);
    spec.model = std::env::var("DOMINION_LIVE_MODEL").ok();
    spec.instructions = Some(std::env::var("DOMINION_LIVE_INSTRUCTIONS").unwrap_or_else(|_| "This short integration game should finish quickly: end your Buy phase as soon as it is offered.".into()));
    h.sandbox.playbook = std::env::var_os("DOMINION_LIVE_PLAYBOOK").map(Into::into);
    let mut game = request();
    if let Ok(seed) = std::env::var("DOMINION_LIVE_SEED") {
        // A random kingdom from this seed, as an arena game deals it.
        game.kingdom = vec![];
        game.seed = Some(seed.parse().expect("DOMINION_LIVE_SEED"));
    }
    if let Ok(limit) = std::env::var("DOMINION_LIVE_TURN_LIMIT") {
        game.turn_limit = Some(limit.parse().expect("DOMINION_LIVE_TURN_LIMIT"));
    }
    let mut agents = vec![spec];
    if let Ok(opponent) = std::env::var("DOMINION_LIVE_OPPONENT") {
        let mut opponent: dominion_server::runner::AgentSpec =
            serde_json::from_str(&opponent).expect("DOMINION_LIVE_OPPONENT agent profile");
        opponent.instructions = agents[0].instructions.clone();
        opponent.response_seconds = Some(60);
        game.agents[1] = Some(opponent.name.clone());
        agents.push(opponent);
        let (send, receive) = tokio::sync::mpsc::unbounded_channel();
        h.arena.runner_connected(
            agents
                .iter()
                .map(|a| Offer {
                    name: a.name.clone(),
                    about: "Bounded native verification".into(),
                    ..Default::default()
                })
                .collect(),
            2,
            send,
        );
        h.jobs = receive;
    }
    h.config.agents = Some(agents);
    h.arena.create_game(Some(&h.who), &game).unwrap();
    let jobs = dispatched(&mut h.jobs);
    let root = Archive::create(std::path::Path::new("/tmp/dominion-native-live"))
        .unwrap()
        .dir;
    let mut tasks = tokio::task::JoinSet::new();
    for job in jobs {
        let mut archive = Archive::open(&root.join(format!("seat-{}", job.seat))).unwrap();
        let spec = h
            .config
            .agents
            .as_ref()
            .unwrap()
            .iter()
            .find(|a| a.name == job.agent)
            .unwrap()
            .clone();
        let server = h.config.server();
        let sandbox = h.sandbox.clone();
        tasks.spawn(async move {
            let result = if spec.program == dominion_server::runner::Program::Anthropic {
                dominion_server::anthropic::play(
                    &spec,
                    &sandbox,
                    &job,
                    &server,
                    &mut archive,
                    CancellationToken::new(),
                )
                .await
            } else if spec.program == dominion_server::runner::Program::OpenAi {
                dominion_server::openai::play(
                    &spec,
                    &sandbox,
                    &job,
                    &server,
                    &mut archive,
                    CancellationToken::new(),
                )
                .await
            } else {
                harness::play(
                    &spec,
                    &sandbox,
                    &job,
                    &server,
                    &mut archive,
                    CancellationToken::new(),
                )
                .await
            };
            if let Err(error) = &result {
                eprintln!("Seat {} failed: {error}", job.seat);
                for entry in std::fs::read_dir(&archive.dir).unwrap().flatten() {
                    if entry.path().extension().is_some_and(|x| x == "stderr") {
                        eprintln!("{}", std::fs::read_to_string(entry.path()).unwrap());
                    }
                }
            }
            result
        });
    }
    eprintln!("Live archive: {}", root.display());
    tokio::time::timeout(std::time::Duration::from_secs(180), async {
        while let Some(result) = tasks.join_next().await {
            let result = result.unwrap();
            assert!(result.is_ok(), "{result:?}");
        }
    })
    .await
    .expect("native verification exceeded three minutes");
    assert!(!h.arena.hub.list()[0].running);
    assert!(h.arena.hub.list()[0].watchable);
    for seat in 0..2 {
        // One CLI process per seat, or (the direct OpenAI agent) none.
        let journal = records(&root.join(format!("seat-{seat}")));
        let cli = journal
            .iter()
            .filter(|r| r["kind"] == "native_request")
            .count();
        let direct = journal.iter().any(|r| r["kind"] == "direct_request");
        assert!(cli == 1 && !direct || cli == 0 && direct, "seat {seat}");
        assert!(
            records(&root.join(format!("seat-{seat}")))
                .iter()
                .any(|r| r["kind"] == "result")
        );
    }
    let game_id = h.arena.hub.list()[0].game_id.clone();
    let (mut cursor, mut trace) = (0, vec![]);
    loop {
        let page = h.arena.transcript(&game_id, cursor).unwrap();
        cursor = page.next;
        trace.extend(page.entries);
        if !page.more {
            break;
        }
    }
    use dominion_protocol::transcript::TranscriptKind;
    assert!(
        trace
            .iter()
            .any(|e| e.activity.kind == TranscriptKind::Message)
    );
    assert!(
        trace
            .iter()
            .any(|e| e.activity.kind == TranscriptKind::Action)
    );
    eprintln!("Completed native game: {}", root.display());
}

#[tokio::test]
#[ignore = "requires bubblewrap and Linux user namespaces"]
async fn forbidden_actions_tools_and_timeouts_preserve_failure_logs_without_resigning() {
    for mode in ["unsupported", "native-tool", "timeout"] {
        let mut h = setup("claude").await;
        let spec = &mut h.config.agents.as_mut().unwrap()[0];
        spec.instructions = Some(format!("fixture:{mode}"));
        spec.response_seconds = Some(1);
        h.arena.create_game(Some(&h.who), &request()).unwrap();
        let mut jobs = dispatched(&mut h.jobs);
        jobs.retain(|j| j.seat == 0);
        let (root, results) = play(&mut h, jobs).await;
        let error = results[0].as_ref().unwrap_err();
        assert!(
            error.contains(match mode {
                "unsupported" => "session ended before the game finished",
                "native-tool" => "forbidden tool",
                _ => "timed out",
            }),
            "{error}"
        );
        assert!(h.arena.hub.list()[0].running);
        let path = root.join("seat-0");
        assert!(records(&path).iter().any(|r| r["kind"] == "seat_finished"));
        assert!(
            !std::fs::read_to_string(path.join("response-000001.stdout.jsonl"))
                .unwrap()
                .is_empty()
        );
    }
}

#[test]
fn profiles_validate_provider_options_and_reject_ambiguous_configuration() {
    let read = |text: &str| toml::from_str::<Config>(text).unwrap().runner();
    let base = "agent_socket='/tmp/fixture.sock'\n";
    assert!(
        read(&format!(
            "{base}[[agents]]\nname='A'\nprogram='antigravity'\neffort='high'\nhistory_turns=0"
        ))
        .is_ok()
    );
    assert!(
        read(&format!(
            "{base}[[agents]]\nname='A'\nprogram='claude'\neffort='ultra'"
        ))
        .is_err()
    );
    assert!(
        read(&format!(
            "{base}[[agents]]\nname='A'\nprogram='codex'\nresponse_seconds=0"
        ))
        .is_err()
    );
    assert!(
        read(&format!(
            "{base}[[agents]]\nname='A'\nprogram='codex'\n[[agents]]\nname='a'\nprogram='claude'"
        ))
        .is_err()
    );
    assert!(toml::from_str::<Config>(&format!("{base}[[agents]]\nname='A'\nprogram='codex'\narguments=['--dangerously-bypass-approvals-and-sandbox']")).is_err());
}

#[tokio::test]
async fn mcp_creation_accepts_a_registered_bearer_session_and_hides_agent_tokens() {
    let mut h = setup("codex").await;
    let guest = dominion_server::client::Client::connect(&h.config.server())
        .await
        .unwrap();
    let denied = guest
        .call("create_game", serde_json::to_value(request()).unwrap())
        .await
        .unwrap();
    assert!(!denied.ok);
    guest.close().await;
    let client = dominion_server::client::Client::connect_authenticated(
        &h.config.server(),
        Some(h.token.clone()),
    )
    .await
    .unwrap();
    let reply = client
        .call("create_game", serde_json::to_value(request()).unwrap())
        .await
        .unwrap();
    assert!(reply.ok, "{}", reply.text);
    assert_eq!(reply.json.unwrap()["seats"], serde_json::json!([]));
    assert_eq!(dispatched(&mut h.jobs).len(), 2);
    assert!(h.arena.hub.list()[0].watchable);
    client.close().await;
}

#[tokio::test]
#[ignore = "requires bubblewrap and Linux user namespaces"]
async fn reasoning_reaches_the_server_before_the_native_response_finishes() {
    use dominion_protocol::transcript::TranscriptKind;
    let mut h = setup("claude").await;
    h.config.agents.as_mut().unwrap()[0].instructions = Some("fixture:streaming".into());
    let created = h.arena.create_game(Some(&h.who), &request()).unwrap();
    let job = dispatched(&mut h.jobs)
        .into_iter()
        .find(|j| j.seat == 0)
        .unwrap();
    let mut changes = h.arena.hub.transcript_changes(&created.game_id).unwrap();
    let stop = CancellationToken::new();
    let cancelled = stop.clone();
    let spec = h.config.agents.as_ref().unwrap()[0].clone();
    let server = h.config.server();
    let mut archive = Archive::create(h._dir.path()).unwrap();
    let task = tokio::spawn(async move {
        harness::play(&spec, &sandbox(), &job, &server, &mut archive, cancelled).await
    });
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let page = h.arena.transcript(&created.game_id, 0).unwrap();
            if page.entries.iter().any(|e| {
                e.activity.kind == TranscriptKind::Reasoning
                    && e.activity.text == "streaming reasoning"
            }) {
                assert!(
                    !page
                        .entries
                        .iter()
                        .any(|e| e.activity.kind == TranscriptKind::Action)
                );
                break;
            }
            changes.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
    assert!(!task.is_finished());
    stop.cancel();
    assert!(task.await.unwrap().is_err());
}

#[tokio::test]
async fn transcripts_follow_game_visibility_and_retry_without_duplicate_entries() {
    use dominion_protocol::transcript::{TranscriptInput, TranscriptKind, TranscriptPage};
    let mut h = setup("codex").await;
    let created = h.arena.create_game(Some(&h.who), &request()).unwrap();
    let jobs = dispatched(&mut h.jobs);
    let input = TranscriptInput {
        attempt: "attempt-1".into(),
        offset: 1,
        time: 10,
        kind: TranscriptKind::Reasoning,
        block: "thought".into(),
        append: false,
        text: "Choose a card".into(),
    };
    let base = h.config.server().trim_end_matches("/mcp").to_string();
    let url = format!("{base}/api/games/{}/transcript", created.game_id);
    let http = reqwest::Client::new();
    let (_, viewer) = h
        .arena
        .guest("127.0.0.2".parse().unwrap(), Some("Viewer"))
        .unwrap();
    let feed = http.get(format!("{url}/events")).send().await.unwrap();
    assert_eq!(feed.status(), reqwest::StatusCode::OK);
    use futures_util::StreamExt;
    let mut updates = feed.bytes_stream();
    assert!(
        String::from_utf8_lossy(&updates.next().await.unwrap().unwrap()).contains("event: change")
    );
    for _ in 0..2 {
        let reply = http
            .post(&url)
            .bearer_auth(&jobs[0].token)
            .header("Content-Type", "application/json")
            .body(serde_json::to_string(&input).unwrap())
            .send()
            .await
            .unwrap();
        assert_eq!(reply.status(), reqwest::StatusCode::NO_CONTENT);
    }
    let change = tokio::time::timeout(std::time::Duration::from_secs(2), updates.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(String::from_utf8_lossy(&change).contains("event: change"));
    drop(updates);
    let reply = http.get(&url).bearer_auth(&viewer).send().await.unwrap();
    assert_eq!(reply.status(), reqwest::StatusCode::OK);
    let page: TranscriptPage = serde_json::from_str(&reply.text().await.unwrap()).unwrap();
    assert_eq!(page.entries.len(), 1);
    assert_eq!(page.entries[0].activity, input);
    let empty = h.arena.transcript(&created.game_id, page.next).unwrap();
    assert!(empty.entries.is_empty());
    assert_eq!(
        http.get(&url).send().await.unwrap().status(),
        reqwest::StatusCode::OK
    );
    let mut private = request();
    private.watchable = Some(false);
    h.arena
        .hub
        .resign(
            &h.arena.hub.claim(&created.game_id, &jobs[0].token).unwrap(),
            "finished",
        )
        .unwrap();
    let private = h.arena.create_game(Some(&h.who), &private).unwrap();
    let reply = http
        .get(format!("{base}/api/games/{}/transcript", private.game_id))
        .bearer_auth(viewer)
        .send()
        .await
        .unwrap();
    assert_eq!(reply.status(), reqwest::StatusCode::FORBIDDEN);
    assert_eq!(
        http.get(format!("{base}/api/games/{}/transcript", private.game_id))
            .send()
            .await
            .unwrap()
            .status(),
        reqwest::StatusCode::FORBIDDEN
    );
}
