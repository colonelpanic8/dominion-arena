# Official-protocol reference fixtures

These tools execute selected parsers and serializers from a separately obtained,
hash-verified public client 2.3.7 bundle in a Node VM. No network APIs, real
credentials, DOM, or browser session are provided. They generate synthetic wire
fixtures, not recordings of account traffic. The upstream bundles are never
copied into this repository.

Place the public `dominion-webclient-head-2.3.7.min.js` and
`dominion-webclient-body-2.3.7.min.js` in a directory, then:

```sh
npm ci --prefix tools/protocol-proof --ignore-scripts
node tools/protocol-proof/generate.mjs /path/to/bundles
node tools/protocol-proof/outgoing.mjs /path/to/bundles
nix develop -c cargo test -p dominion-online
```

`generate.mjs` asks each ancillary upstream parser to consume deterministic
synthetic fields, records the corresponding bytes with the upstream Writer,
then checks those bytes again with the upstream Reader and parser. Both values
of optional booleans are exercised and nested arrays contain an element. The
Rust tests require exact consumption, every truncated prefix to fail, and a
following PONG to remain aligned. The fixtures cover the 25 ancillary opcodes;
the core game/session codecs also have focused Rust fixtures.

`outgoing.mjs` invokes all 68 upstream serializers against a fake sender. The
Rust tests compare all 68 corresponding typed commands byte for byte. No
command is transmitted, including purchases, session removal and other account
changes. These codecs are not exposed as arbitrary MCP commands.

The examples prove field layouts and dispatch coverage. They do not establish
server acceptance of every possible field value, every expansion interaction,
or every account/lobby workflow. Some upstream serializers themselves reject
operations (restricted admissions and undo table settings); those are not
invented by the adapter.

## Adversarial semantic campaign

```sh
node tools/protocol-proof/questions.mjs /path/to/bundles
nix develop -c cargo test -p dominion-online campaign
```

`questions.mjs` shares the hash-checked offline loader with the other generators.
It executes the actual `ChoiceQuestion`, `NumberQuestion`, `ComplexQuestion`,
`DelayedQuestion` and `NameQuestion` parsers, `ChoiceQuestion.canAccept`, each
question's `getAnswer`, and `CleanupWindow.updateAnswers`. It does not execute
`execute`/DOM rendering or transmit answers. The synthetic card context has two
names and three instances, including two copies of one name. The checked-in
corpus contains 20 focused cases and seeds 1–128. Rust checks exact wire
consumption, deterministic projection, the canonical decision schema, accepted
answers byte-for-integer-list against upstream, and upstream-rejected selections.
The shared contract expresses card order top first; the fixture adapter reverses
only that input when comparing to upstream's bottom-first ordering.

The cases cover all five classes, nested AND/OR with explicit empty selections,
unanswered AND siblings, numeric defaults, nested naming, coin/potion overpay,
partial priorities, optional cleanup ordering, distinct-name trashing,
same-name exiling, and all eight game buttons. They found and regress:

- Ordered selections incorrectly required every candidate, including optional
  cleanup items; the adapter now honors the wire minimum and maximum.
- An empty optional branch could not be selected inside a complex question.
- Untouched AND siblings were rejected for missing a mandatory selection instead
  of emitting the empty list produced by the upstream client.
- Distinct-name and same-name constraints were not validated before consuming an
  answer (Temple's trashing and Banish's exiling are relevant triggers).
- Keep-order, autoplay, select-all and Favor actions used strings outside the
  canonical schema; labels retain their meaning with supported action values.

The Rust `seeded_observation_campaign` separately generates 96 ordered protocol
transitions per seed. It tracks physical card locations independently, checks
that each claimed name/count/copy is possible, checks owned composition and the
full view schema, replays the same stream deterministically, and changes hidden
allocation names to check that they do not affect the view. It exercises repeated
public inspections, named/anonymous moves (including cross-player transfers),
shuffles with and without discard, visible/anonymous pile reorder, replacement
snapshots, and stale/single-use answers. Focused regressions cover inspection
identity revocation after shuffle/anonymous departure, repeated/disjoint reveals
of copies with the same name, and turn changes retiring question-only visibility.
A focused three-player case checks hidden passing and private inspection during
a controlled turn. These found stale copy references after inspections and
anonymous moves, and under-counted separately observed copies.

For a larger repeatable run without rewriting the committed corpus:

```sh
mkdir -p "$HOME/plans/dominion-arena/protocol-campaign"
export PROTOCOL_QUESTION_FIXTURES="$HOME/plans/dominion-arena/protocol-campaign/questions.json"
PROTOCOL_SEED=1 PROTOCOL_SEEDS=4096 \
  node tools/protocol-proof/questions.mjs /path/to/bundles
PROTOCOL_REDUCER_SEED=1 PROTOCOL_REDUCER_SEEDS=1024 \
  nix develop -c cargo test -p dominion-online campaign
unset PROTOCOL_QUESTION_FIXTURES
```

A failure includes its question label or reducer seed/step. Reproduce one
question seed with `PROTOCOL_SEED=<seed> PROTOCOL_SEEDS=1`, or one reducer stream
with `PROTOCOL_REDUCER_SEED=<seed> PROTOCOL_REDUCER_SEEDS=1`. The generator range
is bounded to 10,000 seeds per invocation. Fixture output is deterministic; running
the same seed range twice must produce identical bytes.

These are protocol semantics and knowledge tests, not simulated Dominion games.
They use no rules engine and do not establish server acceptance, reachability of
all generated trees, or expansion coverage. No live account was used for this
campaign. In particular, duration replay timing, rotating/split pile behavior,
shadow cards, Ways, projects/traits/prophecies, and full Boon/Hex sequences still
need targeted official-server games. Useful next bot-only kingdoms include:

| Target | Kingdom | Landscapes |
| --- | --- | --- |
| Name restrictions and branching | Temple, Bounty Hunter, Village, Torturer, Moat, Watchtower, Lurker, Black Market, Mystic, Hamlet | Banish, Way of the Mouse |
| Optional cleanup and duration replays | Herbalist, Alchemist, Scheme, Walled Village, Treasury, Page, Peasant, Throne Room, Haven, Royal Carriage | Capitalism |
| Mixed/rotating piles and reveals | Knights, Cultist, Castles, Catapult, Encampment, Townsfolk, Augurs, Forts, Sentinel, Night Watchman | Fated |
| Visibility and changing rules | Ninja, Fishmonger, Rice Broker, Necromancer, Druid, Fool, Leprechaun, Pixie, Kitsune, Mountain Shrine | Enlightenment, Innovation |

These setups remain untested on the official server. Resolve their ancillary
setup choices there (such as the Mouse card and Druid Boons). Reserve the
exclusive account before authentication; never reuse another process's authority.

## Reverse server protocol proof

`reverse.mjs` verifies the Arena server direction independently. It accepts the
hash-pinned **body** bundle directly (the head bundle is unnecessary), executes
its original core parsers and `GameState.parse`, and requires exact byte
consumption. Rendering constructors and post-parse effects are inert; none of the
wire-reading methods are replaced. Event parsers are called using their declared
signature, including the upstream's unused one-argument Rename parser. It checks
TableDetails, Snapshot (including allocation and parallel arrays), every Event
payload, every Question class and element tag, GameFinished, Pong and counters.
The checked-in bytes are synthetic, generated by Rust; no account traffic is used.

```sh
# From the repository root, refresh the synthetic bytes after codec changes:
DOMINION_REVERSE_PROOF_OUT="$PWD/crates/dominion-online/tests/fixtures/server.json" \
  nix develop -c cargo test -p dominion-online --test server_codec \
  export_reverse_proof -- --ignored
node tools/protocol-proof/reverse.mjs /path/to/body-2.3.7.js
nix develop -c cargo test -p dominion-online --test server_codec --test attached_receiver
```

The ordinary `encoded_server_fixtures_match_offline_upstream_proof_inputs` test
requires the Rust encoder to reproduce these exact inputs. The proof also asserts
the pinned question/ability/mode/button enum counts used by the writer. This is
layout evidence for the bounded Arena profile, not official-browser compatibility
or validation of every expansion's gameplay semantics. Full hosted-game acceptance
belongs to the server's `official_host` integration tests.

## Original service/model/answer consumer

`client-harness.mjs` is the stronger offline acceptance layer. `upstream.mjs`
verifies the body SHA-256 before AST selection, extracts named declarations,
Angular registrations and their original initialization expressions, and evaluates
unchanged source in a separate VM. A directory containing both pinned head/body
bundles remains supported by the older producer proofs.

The consumer runs the original dispatcher, login/preferences/meta/session/table
services, `game.readState`, `GameState`, every concrete zone class, `Player`,
`CardObject`, `CardStack`, counters/tokens, animations, `QuestionModel`, question
association, question classes, original buttons, `getAnswer`/`send` and
`serverMessenger`. It does not substitute inert model constructors. Prompt facts
use the original `LogLineParser`, and answer facts contain the original serialized
command. One VM per semantic case prevents accidental state carryover.

The report explicitly lists simulated host plumbing: Angular DI/scope delivery
and digest, DOM/canvas without pixels, image completion without loading image
bytes, deterministic short timers, silent audio, fresh in-memory local/session
storage, game-area containers/remount and unopened lobby-tab service shells.
`VERSION`, the tournament marker and the head's `ObjectValues` alias are supplied
bootstrap values. Original gameplay/model processors are never patched.

```sh
npm ci --prefix tools/protocol-proof --ignore-scripts
# Generate synthetic Rust bootstrap messages; no server or official account:
DOMINION_LIFECYCLE_FIXTURE_OUTPUT="$HOME/plans/dominion-arena/lifecycle-fixture.json" \
  nix develop -c cargo test -p dominion-online --test lifecycle_codec \
  write_original_consumer_fixtures_on_explicit_request
node tools/protocol-proof/client-harness.mjs \
  --bundle /path/to/body-2.3.7.js --scenario fixtures \
  --fixture "$HOME/plans/dominion-arena/lifecycle-fixture.json" \
  --output "$HOME/plans/dominion-arena/consumer-evidence"
```

Semantic fixture format is an array of independent cases:

```json
[{"name":"case","messages":[{"hex":"Rust server bytes"}],
  "answer":{"path":[],"indices":[0]},"expected":{"values":[0]}}]
```

Each stream starts with original LoginSuccess and TableJoined, then Snapshot and
Question. `answer` supports a nested question `path`, `indices`, `decline`,
`number`, a `name` CardNames key, `button` index, or `order` option indices.
Ordered tests rearrange the simulated display stacks and invoke the unchanged
`RevealZone.enableOrdering` before original send; original drag hit-testing and
raster layout are not claimed. `ORDER_CARDS` displays Bottom → Top and sends that
order; the engine adapter reverses it. `PRIORITIZE_CARDS` displays First → Last.

For real Arena sockets, the Rust integration wrapper owns its ephemeral loopback
listener and synthetic accounts. The harness starts no server and touches no
running user server:

```sh
DOMINION_ORIGINAL_BUNDLE=/path/to/body-2.3.7.js \
DOMINION_ORIGINAL_EVIDENCE="$HOME/plans/dominion-arena/socket-evidence" \
  nix develop -c cargo test -p dominion-server --test official_lifecycle \
  original_consumer_lifecycle_and_rules -- --ignored --nocapture
```

The equivalent CLI takes `--endpoint ws://127.0.0.1:<owned-port>/ --scenario
lifecycle-and-rules --fixture FILE --output DIR`. Its fixture has `accounts`
(username/password pairs for synthetic accounts already created by the wrapper).
Empty `steps` runs table creation/join/readiness, ordinary completion and a second
game on the same sockets. `games`, `rules`, `maxDecisions` and `timeoutMs` can be
specified. Optional flags:

- `bootstrap:true`: actual original `connection`/`serverConnection` discovery,
  two probes/direct endpoint, wrong version, password/session login, reconnect,
  and invalid session; uses the special probe envelope and original session store.
- `originalNewTable:true`: original LobbyController New Table producer requests
  its default expansion set; subsequent original rule edits select the fixture
  kingdom before readiness.
- `chat:true`: original chat producer/consumer, including `tN`/`pN` identities.
- `mixedNative:1`: the second account uses native HTTP login/join/ready/answers
  while the first remains an original consumer. Cookies stay in memory.

Scripted `steps` can select client, `command`/`args`, `answer`, `waitOpcode` or
`expect` facts. Supported game commands still come from the original messenger.
Only credential-free `ws:` loopback endpoints are accepted; HTTP uses the same
loopback origin and disallows redirects. Production bootstrap constants never
execute. Original console/timeLog output is suppressed; reports omit passwords,
sessions and login/probe command bytes, including synthetic credentials.

Checks: `node --check tools/protocol-proof/{upstream,consumer,client-harness,bootstrap,assets}.mjs`
(one path at a time), `node --test tools/protocol-proof/harness.test.mjs`, and
focused Rust `server_codec`, `attached_receiver`, `lifecycle_codec` targets.
Set `DOMINION_ORIGINAL_BUNDLE` for the hash/isolation test to also load the bundle.

## Graphical asset inventory

```sh
node tools/protocol-proof/assets.mjs --bundle /path/to/body-2.3.7.js \
  --cache "$HOME/.cache/dominion/assets" --output /path/to/evidence
```

`assets-report.json` enumerates cached files, manifest paths, missing literal
references, template paths and dynamic path prefixes, plus absent application
components. It never downloads assets. The available native `dominion/assets@1`
cache is not an official application distribution. At the next-phase checkpoint,
it contained 1,289 files but lacked original HTML/bootstrap, head, Angular runtime,
stylesheets and HTML templates. Full graphical acceptance is unavailable until a
complete locally supplied distribution exists; the service/model report makes no
claim of official-browser rendering or seamless animation parity.

Semantic cases may also carry `expectedSemantic`: exact flat-leaf
`questionTexts`, per-leaf `targetNames`, `playerCounters` keyed by seat and
upstream counter symbol/ordinal, and required `projects`/`reminders` subsets by
seat. Unknown assertion keys fail. Name answers execute the original `nameInput`
controller's search and `name` path. `{send:true}` invokes Done with unchanged
defaults. Every passed case records which semantic assertions were actually made.

Lifecycle `sessionResume:true` recreates a fresh original VM/socket mid-decision
and resumes through original SessionService using only local Arena authority.
`finalReconnect:true` recreates the original finished model and verifies no new
question appears. Every completed original model's score totals, used turns,
ranks, resignation and seat identities are compared to native Arena's authoritative
finished result; score parts are captured as model facts. Reports never include the
session used for that read.

`lifecycleSafety:true` adds the original-consumer lifecycle regressions discovered
at the second review gate: voluntary/post-finish/native leave, host kick,
RETURN_TO_LOBBY and rejoin/new-table behavior; retained original score/model after
native next-game undo/timing edits; and attachment refusal for an already-running
native undo-enabled game. The refused socket must remain responsive to original
Ping and SessionService Logout, failure traffic must become quiescent without a
state change, and native answers continue. These checks use only the wrapper's
synthetic accounts/listener and terminate their own extra sockets.
