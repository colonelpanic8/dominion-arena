# Official dominion.games backend

`dominion-online` reads the official server's binary WebSocket protocol without a
browser. It has no dependency on `dominion-engine`. The official server decides
all game outcomes; Arena's local card data is used only for glossary text.
`dominion serve` and existing hosted-game APIs are unchanged.

This is an experimental adapter for **client 2.3.7**, guarded by the SHA-256 of
its public body bundle. `dominion online sync` downloads that bundle into the
cache and validates it; it is parsed as data, never executed or bundled in Git.
Another version or changed bytes at the same URL require a protocol update.

## Setup

Use a dedicated account or reserve an idle account exclusively for this client.
In `$XDG_CONFIG_HOME/dominion/config.toml` (or a file passed with `--config`):

```toml
online_account = "your-account"
online_password_entry = "dominion.games/your-account"
online_exclusive = true
online_endpoint = "auto" # auto, alpha, beta
online_resume = false
# online_state_dir = "/path/to/online-state"
# online_cache_dir = "/path/to/online-cache"
```

The password is read from the first line of the configured `pass` entry at
runtime. Alternatively set `online_password_file` (instead of `online_password_entry`) to a
runtime credential file, such as a systemd credential. Neither password nor
session token is written to persistent client state. Session authority is
held only in memory; state contains a process lock, non-secret connection
metadata and metadata identifying a bot table this client created. The default
state/cache locations are `$XDG_STATE_HOME/dominion/online` and
`$XDG_CACHE_HOME/dominion/online` (standard HOME fallbacks). Different locations
are independently configurable. No browser session is read or imported.

```sh
dominion online sync
dominion online probe       # no authentication
dominion online login       # authentication and 31-second heartbeat check
claude mcp add dominion -- dominion online mcp
```

The MCP entry point uses stdio. Register either this backend or Arena's hosted
backend as `dominion`. Authentication belongs to operator setup; gameplay uses
the same `state`, `new_game`, `step`, `wait`, `cards`, and `log` contract
(`dominion/play@1`, structured `{view, glossary?, clock?}` / `dominion/agent-view@2`;
`clock` carries the server's timers in a timed game), plus `turns` and
`play_treasures`. These tools are the ones Arena's own games are played through
(`dominion-server`'s `mcp/play/seat.rs`): this backend supplies only how its seat is
observed, answered and waited for, so their checks, results and errors are the same.
Optional `token` parameters are ignored because one process controls one account.

`new_game` accepts `players`, `bots`, `kingdom`, and `landscapes`. Every opponent
must be a server bot: defaults are two players and one bot. It requests unrated
play with spectators prohibited, verifies the returned rules and membership,
and refuses settings it cannot honor. The upstream client does not implement
the host-only admission serializer, so a table is **not proven unjoinable**;
Joining players and spectators update table membership without disconnecting the
client. Bot-game actions still require a table containing only this account and
server bots. Public human matchmaking is available through the server management page below;
joining arbitrary tables is not exposed.

`online_reconnect` explicitly resumes only the in-memory session owned by this
process. It invalidates previous decisions and waits for a replacement snapshot.
Answers are consumed before transmission and are never automatically retried,
even when a send fails. To resume after restarting the process, opt into
`online_resume = true`: only a table recorded in this state directory is
eligible, and its account and opponent identities are checked again. An
unrecognized existing game or `ALREADY_LOGGED_IN` stops the client. There is no
kick, session-removal or kick-and-resign recovery path.

## Server-managed play and watching

`dominion serve` can keep official connections alongside Arena's hosted games
(one per account; see "Several official accounts" below).
First register the Arena account that should own the connection. Then add these
settings to the normal flat TOML configuration, alongside the online account,
password-store entry, cache/state paths and exclusivity assertion above:

```toml
online_enabled = true
online_owner = "your-registered-arena-account"
```

Start `dominion serve` with that configuration and the packaged web client.
The server authenticates once and shares that connection between agents and the
live observer. It disconnects on shutdown; do not also run `online mcp` for the
same account. The state-directory lock prevents two processes sharing it.

- Sign in to Arena as `online_owner`, then open **`/online`** (the dominion.games
  navigation link). Choose a runner agent and a game type: unrated server bots,
  unrated public humans, or rated public humans. Human searches support two or
  three players, a relative rating range and one standard timer preset. The
  default timer is Patient. Starting one search never repeats it after a game.
- Bot games accept required kingdom piles and landscapes. Public matches use the
  server's matchmaking kingdom and preferences, with previews and extra new
  expansion games disabled.
- The page shows connection status, search status, the assigned agent, and the
  connected account's live table, with each player's time when the game has a timer
  (the server's latest timers, the running one counted down, free decision time
  first). Cancel a search, pause/resume an agent,
  reconnect, or explicitly resign. Pausing leaves the game running. A search
  that wins a cancellation race is kept and assigned to its selected agent.
- A runner slot is reserved before searching or creating a game. Only one
  official game/search can be assigned to an account at a time. Agents receive rotating,
  revocable capabilities, and only gameplay tools; lifecycle actions go through
  the authenticated management API. Agent archives stay private on the runner.
- **`GET /api/online/manage`**, **`POST /api/online/start`**,
  **`POST/DELETE /api/online/agent`**, **`DELETE /api/online/matchmaking`**,
  **`POST /api/online/reconnect`** and **`POST /api/online/resign`** provide the
  same management operations. `start` accepts `agent`, `mode` (`bots`, `unrated`,
  `rated`), and `matching` (`players`, `minRating`, `maxRating`, `timer` ordinal
  0 Reckless / 1 Hasty / 2 Friendly / 3 Patient). Bot settings are `bots`,
  `kingdom` and `landscapes`.
- Agents play through **`/mcp/online`** using `state`, `step`, `wait`, `cards`,
  `log`, `turns` and `play_treasures`. An Arena owner session or its bot API key also works. Hosted MCP
  refuses `new_game`, `resign` and `online_reconnect`; use management for those.
- **`GET /api/online/rating?type=2p`** returns the connected account's rating and record
  on dominion.games, as its player page shows them. The rating (level, skill (mu),
  deviation, games) is the open rating period's when it has games (`provisional`, with
  the period's games, score against expectation and starting level), since that counts
  every game finished so far; the leaderboard rank and the history's settled points only
  move when a period closes. Also: wins/losses/ties, best level and rank with their dates,
  longest streaks, country and title, the last 30 history points, and the 25 most recent
  games (outcome, scores, opponents, levels, expected score, kingdom). `type` is `2p`
  (default), `3p`, `2p-blitz` or `3p-blitz`. The server asks dominion.games over the
  account's connection at most once a minute per type, and again every few seconds for
  20 seconds after one of its games finishes, so a finished game shows up promptly. A
  `RATING_SERVICE_DOWN` reply fails only that request (503), not the connection. It is
  public like watching, and the `/online` page shows it.
- **`GET /api/online/view`** and **`GET /api/online/events`** provide the legal
  observation: `{connected, error, game?}`, where `game` has the shape
  `GET /api/games/{id}/watch` gives one of Arena's games (view, version, the seat
  deciding, table extras, `meta` with the clock), so the web client keeps both in the
  same model. Its log is always the whole log, and `undos` counts the snapshots
  dominion.games has sent. Responses use `Cache-Control: no-store`, omit the current decision
  and are public, including to signed-out viewers. Watching shows the playing
  account's legal view, including its hand. The stream closes on shutdown.
  Management and gameplay MCP access remain restricted to the owner and its bot keys.

Human matches require an explicit search from this process. The adapter verifies
`AutomatchFound`, the returned rating rule and the exact player roster, then
checks snapshots and later table membership against that roster. Table and player
notifications may precede match confirmation; gameplay received in that interval
is buffered with a fixed limit and cannot authorize agent actions. It is applied
only after confirmation, including the snapshot roster check. A completed result
authorizes leaving only its own table, never a subsequently assigned table. It
never joins or takes over an unrelated human table. Spectators can be present at public
matches, but the adapter uses only its own playing seat's legal view. Opponent
undo requests are declined automatically. When the server offers to resign an
opponent who left (its `TimeoutOffer`), the adapter takes it at once with a
`TimeoutRequest`, in any game. Reconnect invalidates old decisions;
process restart leaves the agent paused until explicitly resumed from the page.

### Several official accounts

`dominion serve` can keep several official accounts connected at once, each with
its own owner, credential, state directory and agent. The `online_*` settings
above are the account with id `default`; add others as tables:

```toml
[[online_accounts]]
id = "sol"                    # lowercase letters, digits and -; not "default"
account = "SolDominion"       # the official-server account
owner = "Arena Matches"       # registered Arena account that manages it
password_file = "/run/credentials/dominion.service/online-password-sol"
# password_entry = "dominion.games/SolDominion"   # instead of password_file
exclusive = true              # the same operator assertion as online_exclusive
resume = true                 # optional, default false
# endpoint = "auto"           # optional
# state_dir = "..."           # default <online_state_dir>/accounts/<id>
# enabled = false             # leave it configured but not kept by serve
```

Ids, official account names and state directories must all differ. Each account's
routes are `/online/<id>` (web), `/api/online/<id>/{view,events,rating,manage,start,agent,matchmaking,reconnect,resign}`
and `/mcp/online/<id>`. The `default` account (or, without one, the first
configured) also keeps the unprefixed `/online`, `/api/online/...` and
`/mcp/online`. **`GET /api/online/accounts`** lists the served accounts (id,
official name, whether it is the unprefixed one, connected, playing) to anyone.

Only an account's own owner (or that owner's bot keys) can manage it or use its
MCP endpoint; watching stays public. Each account has at most one assigned agent,
and the runner's job for it carries the account id, so its revocable capability
opens only that account's `/mcp/online/<id>`. The runner's capacity is shared.
At startup a settings error stops the server, but an account that fails to log in
is reported and left out while the others are served (unless none logs in).

`dominion online accounts` prints the configured accounts and where each password
comes from without connecting. `dominion online login --account sol` and
`dominion online mcp --account sol` pick an account; without `--account` they use
`default`, or the only account configured.

With the NixOS module, set `services.dominion.onlinePasswordCredential` to an
on-host systemd-creds encrypted password path. The module loads it into the
service's runtime credential directory and configures `online_password_file`;
it also gives official state/cache their own directories and syncs the pinned
catalog before startup. No plaintext credential enters the Nix store.

Further accounts go in `services.dominion.onlineAccounts.<id>`, which writes their
`[[online_accounts]]` tables. Each password becomes the service credential
`online-password-<id>`, from `passwordFile` (a root-readable runtime file such as
an agenix secret, loaded with `LoadCredential`) or `encryptedCredential`
(systemd-creds, with embedded name `online-password-<id>`):

```nix
services.dominion.onlineAccounts.sol = {
  account = "SolDominion";
  owner = "Arena Matches";
  exclusive = true;
  encryptedCredential = "/var/lib/private/dominion-online-sol-password.cred";
  # or: passwordFile = config.age.secrets.dominion-online-sol.path;
  settings.resume = true;
};
```

This observes games played by this headless connection. It does not import an
active browser session or attach to arbitrary matches. `/mcp` and `/game/:id`
continue to serve Arena's own games.

## Coverage and limits

The codec checks lengths, counts, UTF-8, booleans, safe integer longs, counters
and structural ordinals. Unknown messages stop the connection; messages cannot
be skipped because this protocol has no generic message length. Application
PING follows ten idle seconds; PONG has a separate five-second deadline.
Animation compensation is acknowledged only after the preceding batch has been
applied successfully.

The pinned upstream dispatch tables contain **62 incoming messages, 68 outgoing
commands, 21 game-event classes and five question classes**. Every one has a
codec here. Ancillary payloads (commerce, mail, statistics, profiles,
tournaments) have typed decoders; all outbound commands have typed encoders.
The MCP service exposes gameplay and session operations, not arbitrary account
commands. Encoding support does not perform a purchase, change an account, or
remove a session.

| Layer | Coverage | Verification |
| --- | --- | --- |
| Incoming dispatch | 62/62 message types | Exhaustiveness gate, core fixtures and 50 upstream-generated ancillary examples |
| Outgoing commands | 68/68 command types | Every typed command compared byte for byte with the actual upstream serializer |
| Game events | 21/21 classes | Exact payload consumption and truncation tests |
| Questions | 5/5 classes, all element shapes | Nested AND/OR, names, numbers, delayed, selection, cleanup and ordering fixtures |
| Snapshot | Complete wire structure | Exact consumption, truncated prefixes, reset and knowledge tests |
| Runtime | Bot setup, gameplay, reconnect and results | Local fake-WebSocket tests and completed live bot games |

Layouts were derived from the pinned public client's Reader/Writer,
`serverToClientProcessor`, `fullGameStateProcessor`, `gameEventProcessor`,
`questionTypeProcessor`, and `serverMessenger`. Reproducible reference tooling
and its exact method are documented in
[tools/protocol-proof](../../tools/protocol-proof/README.md). It runs selected
upstream functions in an offline VM against synthetic data; no network or account
traffic is used. The production Rust client parses the cached bundle as data.

Complete wire coverage is distinct from exhaustive live validation. The shared
view projects legal knowledge, counters, reminders, projects, states, Boons,
Artifacts, pile/adventure tokens, lasting-card details and visible associations.
It does not expose allocation identities from hidden zones. Cosmetic image
changes are retained in the raw model but do not affect text gameplay. Upstream
itself lacks serializers for a few table settings; this client rejects those
rather than inventing undocumented server requests.

Snapshot and event parsers cover all 21 event classes in 2.3.7. The reducer uses
`dominion-knowledge`, the same inference core as Arena's local engine, through an
adapter for legal server observations. It tracks known composition, ownership, and
candidate groups spanning zones, so later reveals can determine previously unknown
cards in the hand or deck. Hidden allocation identities are never observations;
shuffles revoke copy associations. Partially unidentified groups remain conservative.
A group with cards never seen says how many of its cards are unknown. Before the first
turn, the log's "starts with" entries name each player's cards. A replacement snapshot
of the position the reducer was at keeps what was known of the players' cards; one of
any other position resets knowledge. A count correction changes only the corrected
zone: missing cards arrive unknown and surplus ones leave for no zone. Nothing
reconstructs unobserved history. Deck order and hand probabilities are not tracked.
Duration timing is described only as precisely as the server state allows.

Questions support Choice, Number, Complex AND/OR, Delayed and Name, including
position-versus-name ordinals, decline/default answers and top-first ordered
answers. Cleanup reordering and controlled turns are supported. Undo requests in
bot games, unsolicited player replacements and changes outside the
bot-only runtime policy stop gameplay for operator inspection. All of their wire
payloads are decoded. Tournament and account workflows are library codecs, not
MCP gameplay tools. Startup and reconnect must produce a supported snapshot before
an answer can be sent.

## Verification

Run from the Nix development shell:

```sh
cargo test -p dominion-online
cargo test -p dominion-server online::runtime
cargo clippy -p dominion-online -p dominion-server --all-targets -- -D warnings
cargo fmt --all -- --check
```

The tests use synthetic binary fixtures, not copyrighted client bundles or
account traffic. They cover exact consumption, every truncated snapshot prefix,
all event classes, nested answers, schema validation, anonymity, shuffle and
snapshot resets, single-use answers, login contention, privacy gates, and local
fake-WebSocket heartbeats. The optional catalog test reads a separately obtained
pinned public bundle:

```sh
DOMINION_ONLINE_TEST_BUNDLE=/path/to/body-2.3.7.js \
  cargo test -p dominion-online extracts_verified_bundle -- --ignored
```

No production credentials, session secrets, player profiles, or captured raw
login frames belong in fixtures. Re-run live validation only with an authorized
idle account and server bots; synthetic success is not proof of live coverage.

### Live validation (2026-11-15)

Using an authorized idle account, the native client authenticated and remained
connected through a 31-second heartbeat check. Two unrated games against Lord
Rattington completed through the stdio MCP tools with this required kingdom:
Village, Smithy, Market, Moat, Festival, Laboratory, Chapel, Cellar, Workshop,
Militia. The throwaway test responder bought money and Provinces; it did not
exercise every card in that kingdom. The official results were 43–33 after 23
turns and 36–21 after 20 turns, both won by the server bot.

The second run verified in-process socket reconnect, replacement snapshot and
rejection of an old decision without consuming its replacement. Restart recovery
was also exercised while correcting decoder mismatches. These runs found
migrated preference types, anonymous reordering, absent pile-top sentinels, and
duplicate snapshot requests; each has a synthetic regression test. There is no live evidence yet for every expansion or games with more than one bot.

A further Seaside/Intrigue game completed after restart recovery at turn 9,
finishing 59–11 for Lord Rattington. Its required kingdom was Haven, Native
Village, Fishing Village, Masquerade, Baron, Nobles, Courtyard, Mining Village,
Bridge, Upgrade. The responder bought and played Action cards, exercising
Duration bonuses, set-aside cards, passing, topdecking and modes. This exposed a
normal decision-log replacement incorrectly invalidating a newer question; a
regression test now preserves the question while replacing those log entries.

A separate live lifecycle check verified two consecutive games in one MCP
process: resigning returns the server's finished result, `new_game` leaves only
the completed table and waits for a different game snapshot, and the second
game can also be resigned cleanly.

### Offline adversarial validation (2026-11-16)

The [semantic campaign](../../tools/protocol-proof/README.md#adversarial-semantic-campaign)
compares nested answers with actual pinned upstream methods and checks reducer
knowledge against independent synthetic card-location traces. It found and fixed
partial/optional ordering, empty complex branches, untouched AND siblings,
card-name selection constraints, schema-invalid action values, and inspection
copy identities surviving shuffles or anonymous departures. Separately observed
copies now contribute to guaranteed composition without counting repeated reveals
twice. Turn changes also retire question-only visibility.

The default tests retain 20 focused upstream cases plus 128 deterministic seeds,
and 64 reducer streams of 96 transitions each. Larger ranges and single-seed
reproduction are documented with the tooling. These offline traces do not run
local Dominion rules and do not add live expansion coverage or prove official
server acceptance of every generated tree.

### Managed live validation (2026-11-16)

The deployed server's Terra agent completed two unrated official-server bot games
through the managed runner and common MCP tools. The Base kingdom listed above
finished 31–25 to Lord Rattington (15 turns for the bot, 14 for the agent), with
an explicit socket reconnect and agent resume during the game.

A Nocturne kingdom finished 46–42 for the agent after 18 turns each. Required
piles were Necromancer, Druid, Cemetery, Secret Cave, Shepherd, Monastery,
Devil's Workshop, Exorcist, Cobbler and Crypt, with Way of the Turtle and
Innovation. This run survived both an explicit socket reconnect and a full
server-process restart at turn 5: the same game was restored with a new decision
epoch, and the selected agent was reattached through management. Observed
interactive sources included Cemetery, Cobbler, Devil's Workshop and Shepherd;
Way of the Turtle's set-aside/replay choice was also used. A card's presence in
the kingdom alone is not evidence that its abilities were exercised: Druid's
Boons and Innovation's effect were not verified by this run.

The deployed browser flow verified all three game-type selectors, bot creation,
live table rendering, pause/resume, explicit resign, result display and reconnect.
A subsequent game after a completed-game reconnect exposed stale table
membership; the client now handles the server's already-left-table response
only while leaving a verified completed table. Browser checks also exposed a
cached generated-script URL and a missing shared-table context; both have
regression fixes. Native Arena chat and native rating/clock headers are omitted
from the official table; its active game mode comes from official management.

Public human matchmaking criteria, verified-roster/rating gates, queue capacity,
notification gaps and cancellation races have synthetic coverage. No public
human match was started during this live validation.

### Arena lifecycle server codecs

`server::decode_command` and the five fixed-seat `ClientCommand` variants are
preserved. `decode_lifecycle_command` adds the selected bootstrap/table/chat and
visual-preference surface as `LifecycleCommand`. Passwords, session tokens and
login codes use redacted `Secret`; unsupported account/commerce producers are
validated for framing but return only `Unsupported { opcode }`. All 68 original
producer layouts have exact-consumption/truncation checks, and no codec grants
mutation authority. Arena remains responsible for authentication and permissions.

`ServerEncoder` writes bounded, counted normal lifecycle/game batches.
`encode_probe` is separate because its response is counter + ServerStates ordinal
without a normal message opcode. `LoginSuccess::new` provides non-deleted typed
client preferences, local card availability and timer descriptions; host policy
still decides which settings and game setups are supported. Synthetic session
values are not printed by Debug.

The [original consumer harness](../../tools/protocol-proof/README.md#original-servicemodelanswer-consumer)
runs the pinned upstream services, real model/zone/card/question associations,
buttons and answer serializer against Rust fixtures and local Arena sockets.
Reports distinguish those checks from full graphical acceptance. The cached
asset inventory is incomplete, and no production login/download is part of the
Arena compatibility workflow.
