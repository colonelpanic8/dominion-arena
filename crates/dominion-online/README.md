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
runtime. No password or session token is written to disk. Session authority is
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
(`dominion/play@1`, structured `{view, glossary?}` / `dominion/agent-view@2`).
Optional `token` parameters are ignored because one process controls one account.

`new_game` accepts `players`, `bots`, `kingdom`, and `landscapes`. Every opponent
must be a server bot: defaults are two players and one bot. It requests unrated
play with spectators prohibited, verifies the returned rules and membership,
and refuses settings it cannot honor. The upstream client does not implement
the host-only admission serializer, so a table is **not proven unjoinable**;
if a human joins, the adapter stops sending actions. Human matchmaking and
joining arbitrary tables are not exposed.

`online_reconnect` explicitly resumes only the in-memory session owned by this
process. It invalidates previous decisions and waits for a replacement snapshot.
Answers are consumed before transmission and are never automatically retried,
even when a send fails. To resume after restarting the process, opt into
`online_resume = true`: only a bot table recorded in this state directory is
eligible, and its account and opponent identities are checked again. An
unrecognized existing game or `ALREADY_LOGGED_IN` stops the client. There is no
kick, session-removal or kick-and-resign recovery path.

## Server-managed play and watching

`dominion serve` can keep one official connection alongside Arena's hosted games.
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

- Agents use the streamable HTTP MCP endpoint **`/mcp/online`** with an Arena
  session's bearer token or an API key for a bot owned by `online_owner`. The
  common `state`, `new_game`, `step`, `wait`, `cards` and `log` tools stay the same.
- Sign in to Arena as `online_owner`, then open **`/online`** (the dominion.games
  navigation link). It renders the existing graphical table and updates over SSE.
- **`GET /api/online/view`** and **`GET /api/online/events`** provide the same
  authorized observation to other clients. Responses use `Cache-Control: no-store`.

The page is read only and shows the connected account's legal view, including
its hand. It is therefore restricted to that registered Arena owner and its bot
keys; guests and unrelated accounts cannot subscribe. The SSE stream rechecks
authorization before sending updates and closes on logout or server shutdown.
The page has no seat token or answer controls, and observer payloads omit the
current decision. Agent MCP calls still have access to the real decision.

This observes games played by this headless connection. It does not import an
active browser session or attach to arbitrary matches; the existing bot-only
policy remains in effect. `/mcp` and `/game/:id` continue to serve Arena's own
hosted games.

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

Snapshot and event parsers cover all 21 event classes in 2.3.7. The reducer keeps
conservative player knowledge: hidden snapshot allocation identities do not
become visible cards; explicit observations and moves establish knowledge;
shuffles revoke copy identities; snapshots replace knowledge and invalidate old
decisions. It preserves guaranteed composition and unlocated owned cards, but
does not claim perfect recall or complete cross-zone deduction. Duration timing is described only as precisely as the server state allows.

Questions support Choice, Number, Complex AND/OR, Delayed and Name, including
position-versus-name ordinals, decline/default answers and top-first ordered
answers. Cleanup reordering and controlled turns are supported. Metagame
requests (undo/timeout), unsolicited player replacements and changes outside the
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
