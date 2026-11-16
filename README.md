# dominion-engine

A headless rules engine for the deck-building card game
[Dominion](https://www.riograndegames.com/games/dominion/), written in Rust, aiming
to implement every expansion.

Status: every expansion through Rising Sun is implemented, with the promos and the
cards removed in second editions: every Kingdom card, non-Supply card, Event,
Landmark, Project, Way, Ally, Trait, Prophecy, Boon, Hex, State and Artifact. Arcana
is unreleased; its entries are placeholders until its cards are public.

Card data (official text, cost, types) is available without a game:

```rust,ignore
use dominion_engine::{Card, Types};

let card = Card::from_name("Throne Room").unwrap();
println!("{} ({}, {}): {}", card, card.cost().unwrap(), card.types(), card.text());
assert!(card.is(Types::ACTION));
```

The data lives in `data/cards/<set>.json` and is compiled into the `Card` enum.

```rust,ignore
use dominion_engine::{Card, Game, GameConfig};

let mut game = Game::new(GameConfig {
    players: 2,
    kingdom: vec![Card::Smithy, Card::Village],
    seed: 42,
    turn_limit: None,
})?;
while let Some(decision) = game.pending().cloned() {
    let view = game.view(decision.player); // what that seat may see
    let response = my_agent.decide(&view, &decision);
    game.respond(response)?;
}
println!("{:?}", game.outcome());
```

`dominion_engine::play` runs this loop over a set of `Agent`s.

A `Game` can be cloned or serialized at any point, including mid-decision.

## Playing over the agent protocol

`crates/dominion-agent` implements `dominion/agent-view@2`, the protocol defined in
[dominion-playbook](https://github.com/colonelpanic8/dominion-playbook) (`protocol/`): each
seat gets a JSON view of everything it may know, with perfect recall (cards it has seen
stay known until it can no longer tell them apart), plus its open decision as indexed
options; it answers with option indices. A `Table` hosts a game this way; the crate also
renders views as compact text for language models.

The `dominion` binary (`crates/dominion-server`) serves games over MCP (streamable HTTP):

```sh
nix run . -- serve                 # or: cargo run -p dominion-server -- serve
claude mcp add --transport http dominion http://127.0.0.1:7317/mcp
```

The common gameplay interface is `dominion/play@1`, specified in
[Playbook's tool contract](https://github.com/colonelpanic8/dominion-playbook/blob/main/protocol/tools.md).
Navigator uses the same gameplay loop for dominion.games; Arena hosts its own games and
keeps its seat authentication and lobby separate.

1. Call `state` to orient. To create a game, call `new_game` with optional `players`
   (default 2), `kingdom`, `landscapes`, `sets`, `names`, `seed`, `turn_limit`,
   `colonies`, `shelters`, and `watchable`. Required cards are included and the rest of
   the kingdom is dealt randomly. Arena has no automatic bot opponents: `bots` defaults
   to 0 and positive values are refused. The result joins you to seat 0 and returns
   secret tokens for every seat; give the other tokens to their players.
2. To play an existing seat, call `join_game` with `game` and `token`, or pass `token`
   to `state`, `step`, and `wait`. A token is the authority to see and act for that seat.
   A session binding ends on restart or after `session_idle_minutes` without a call;
   keep your token. `new_game` never resigns or replaces a previous game.
3. Read the returned view and call `step` with `decision` and `indices` (ordered: top
   first), or `name` when `nameAny` is true. Give `expect` with each selected option's
   label or card, in index order. Illegal, stale, and mismatched answers change nothing;
   the error includes the open decision. `step` waits for your next decision. When
   `decision` is null, call `wait`; continue until `view.game.running` is false.

`state`, `new_game`, and common `join_game` return full text; `step` and `wait` return
compact changes (full text on the session's first observation). They always carry the
full structured `{view, glossary?}` envelope. Card text arrives once per session/game
as `CARDS`, excluding basic Treasures, Victory cards, and Curse. Logs include sequence
numbers; long text logs keep the newest 50 and explicitly point to `log` for the rest.
`log` takes `after` (exclusive seq) and `limit` (default 50), returning oldest first.
`cards` takes `names` and returns `{glossary}` with official text and current costs and
types in the last observed seat's game (printed values before joining). `log` and
`cards` use the last common observation's seat, or the joined seat. `timeout_s` defaults
to 60 and is capped at 600 seconds and half the configured session lifetime; the default
server accepts at least 300 seconds. The engine automatically resolves forced decisions.

Compatibility tools remain available: `create_game` returns tokens without joining;
`view`, `answer` (`decision_id`), `wait_for_decision` (`timeout_ms`), `play_treasures`,
and `card` retain their original arguments and raw structured responses. `join_game`
with legacy `game_id` also retains its raw response and grouped reference. To assign
house-agent profiles, use `create_game`'s explicit `agents` option with a registered
account; a bot count is never silently converted into profiles. `resign`, `list_games`,
and lobby tools `sign_in`, `find_match`, `cancel_match`, `list_tables`, `join_table`,
`wait_for_game`, and `leave_table` remain available. Unknown arguments are refused.

Legacy `join_game` with `game_id`, and joining through the lobby, also include a grouped reference of official
card text, types and printed costs in the response content. It covers the visible cards,
related non-Supply piles, every member of split or mixed piles, and the Boons and Hexes
used by the game. Pile structure and member text stay together; exclusive companions
(such as Travellers, Prizes, Heirlooms and randomized set-aside cards) appear with their
source. Shared components such as Loot and Horses are defined once and list all sources
present. Boons and Hexes have their own sections; a Druid-only game includes just its
three selected Boons. Landmarks keep their official scoring rules beside their current
VP tokens and chosen piles. The live view lists Supply piles and landscapes on separate
lines. Native agents receive the reference with their initial view. Later views omit the
reference; `view` with `include_cards: true` retrieves it again, including newly visible
cards. With `format: "json"`, the reference is a separate JSON content block containing
`cards`; the structured agent-view@2 stays unchanged.

Mixed piles also carry `structure`: their public initial recipe, captured by the same
setup helpers that construct the pile. It distinguishes stacked splits (including
Castles), rotating four-card piles, and shuffled pools (including random samples such
as Ruins). The reference explains these generically with initial group sizes and order;
the live view labels the pile type and gives its current top and total. The recipe never
reveals hidden order, chosen Ruins counts, or remaining counts by card. Old saved games
without recipes still load and omit that metadata.

To print an initial view and reference locally:

```sh
cargo run -p dominion-agent --example view -- Page Tournament Joust Augurs \
  Encampment Pickaxe Sleigh BlessedVillage Druid Tormentor \
  "Way of the Mouse" Labyrinth Obelisk Cursed
```

An agent plays other people through the lobby (see "Accounts and the lobby"): `sign_in`
with a bot's API key (or send it as the connection's bearer token), or with nothing to
play as a guest; then `find_match` searches for an automatch and waits, or `list_tables`
and `join_table` sit it down at a table and wait for the table's game. Each answers with
the seat and its token once a game starts and binds the session to it; when the wait
times out the search or seat stands, and calling again (or `wait_for_game`) keeps
waiting. An agent that stops calling drops out of the queue within a minute and off its
table within a few.

In a game with Ways, every Action played asks "play it as itself, or use a Way instead".
`answer` takes `way` to answer that in advance: `"none"` (as itself) or a Way's name. It
answers each Way question the answer leads to for that seat (both plays of a Throne
Room'd card) until a decision that isn't one, and is ignored when none follows. A Way
not in the game rejects the answer; one not offered for a play leaves that question open,
and the reply's text says so. Without `way` the question is asked as usual, after "when
you play" effects (a +1 Card token's draw), for agents that want to see those first.
Decisions whose answer may play an Action say so in their prompt.

Games survive restarts: the server saves each game (with its seat tokens, so the files
are private to you) to `$XDG_STATE_HOME/dominion/games/<id>.json` after every change and
resumes them on startup. Finished games are deleted on a later startup once they are a
week old. One server uses the directory at a time. `--ephemeral` keeps games in memory
only.

The same server has a JSON API under `/api` for browser clients (types in
`crates/dominion-protocol`: `api` and `lobby`). A seat authenticates with its token as
`Authorization: Bearer <token>` (or `?token=` on the event stream), like MCP agents. An
account authenticates with its session: the `dominion_session` cookie that signing in
sets, or the same token (or a bot's API key) as a bearer token. On the game routes a
session stands in for the token of a seat its account plays (`?seat=N` says which, when
it plays several). Errors are `{"error": "..."}` with 400 (bad request), 401 (not signed
in, or a missing or wrong token), 403 (not yours to do; a request another site's page
made), 404 (unknown game, table, player or route), 409 (stale decision, not this seat's,
or it doesn't fit the state of things), 413 (a body over 64 KiB), 422 (invalid answer),
429 (too many lately, with `Retry-After`), 503 (the server hosts as many games as it
will) or 500.

| Route | Body | Returns |
| --- | --- | --- |
| `GET /api/cards` | | every card-shaped object as printed |
| `GET /api/settings` | | the web client's settings: `{webAnimations, webAnimationSpeed, webMoveKingdom, webConfirmSingle, webBackground, webSounds, webVolume, bannedCards, registration, guests}`; `bannedCards` is the asker's own list once they have set one |
| `PUT /api/settings/banned-cards` | `["Witch", ...]` | the settings, after replacing the asker's Banned cards list (signed in) |
| `GET /api/games` | | the games, as `list_games` gives them, with `joined` (per seat, whether its token has been used since the server started), `source` (`direct`, `table`, `automatch`), `rated`, `watchable` and `users` (per seat, the account playing it) |
| `POST /api/games` | `create_game`'s arguments | 201, the game id and seat tokens |
| `POST /api/deal` | `{players, sets?, kingdom?, fromSets?, landscapes?, slots?, banned?, seed?}` | `{kingdom, landscapes, seed}`: the kingdom those rules deal, without creating a game |
| `GET /api/seat` | | `{gameId, seat, name}` of the token |
| `GET /api/games/{id}/view[?logFrom=N]` | | `{view, version, deciding, table, undos, canUndo, meta}`; with `logFrom`, the log from entry N. `table` holds what the graphical table draws beside the view: each player's cards in play in the order played and the top of their discard pile, as far as the seat may see them. `undos` counts the answers taken back: when it changes, the log may differ from what was held at any entry. `canUndo`: the seat answered last. `meta`: `{source, rated, watchable, timed, users, connected, forfeitAt?, watchers, ended?, now}` (`now`: the server's clock, to read `forfeitAt` against) |
| `POST /api/games/{id}/answer` | `{decision, indices}` or `{decision, name}` | the same, after it |
| `POST /api/games/{id}/undo` | | the same with the whole log, after taking back the seat's last answer (and the Way answers that came with it); 409 when another seat has answered since. A seat can keep undoing back to the last answer of another seat. The game, with its shuffles, is restored exactly, and decision ids are never reused. The server keeps nothing to undo across a restart. 403 in a lobby game, unless its table allows undo, and in any lobby game once it is over |
| `POST /api/games/{id}/resign` | | the same, after the seat has resigned |
| `GET /api/games/{id}/watch[?logFrom=N]` | | the same shape for someone watching (signed in; the game must be watchable): no seat or decision; every player’s hand, deck and other owned cards visible |
| `GET /api/games/{id}/events` | | server-sent `change` events with `{version, chat}`: on connect, after every change (also, within seconds, when a seat connects or leaves or the number watching changes), and when something is said in the game's chat. With a seat it keeps the seat connected; without one, signed in, it is a watcher's |
| `POST /api/auth/register` | `{name, password}` | 201, the account (`Me`), and the session cookie. From a guest's session the guest becomes the account (and keeps its name when `name` is left out) |
| `POST /api/auth/login` | `{name, password}` | the account and the session cookie |
| `POST /api/auth/guest` | `{name?}` | 201, a guest account and its session cookie |
| `POST /api/auth/logout` | | 204; the session is over |
| `GET /api/me` | | `{user: {name, kind, rating?, owner?}, rating: {rating, deviation, games}, stats: {games, wins}, bots}` |
| `PUT /api/me/password` | `{current, new}` | 204; the account's other sessions are over |
| `GET /api/me/games` | | the seats the account plays, with their tokens |
| `POST /api/me/bots` | `{name}` | 201, `{bot, key}`: a bot and its API key, shown this once |
| `POST /api/me/bots/{name}/key` | | a new key for the bot; the old one stops working |
| `DELETE /api/me/bots/{name}` | | 204 |
| `GET /api/lobby` | | `{version, you, online, searching, tables, table, queue, games}`: the lobby as the asker sees it |
| `GET /api/lobby/events` | | server-sent `change` events with `{version, chat}`; while it is open the account counts as connected |
| `POST /api/tables` | `{spec?, seats?}` | 201, the new table, hosted by the asker |
| `GET /api/tables/{id}` | | the table: `{id, host, spec, seats, game?, lastGame?}` |
| `PUT /api/tables/{id}/spec` | a `TableSpec` | the table, with its new rules (host) |
| `PUT /api/tables/{id}/seats` | `[{"player": name} \| "open" \| {"token": name}, ...]` | the table, with those seats (host) |
| `POST /api/tables/{id}/join` | `{seat?}` | the table, with the asker seated |
| `POST /api/tables/{id}/ready` | `{ready}` | the table; its `game` once that has started |
| `POST /api/tables/{id}/leave` | | 204 |
| `POST /api/automatch` | `{players?, opponents?, rated?}` | the search: `{state, since, request, gameId?}` |
| `DELETE /api/automatch` | | 204 |
| `GET /api/chat/{channel}[?after=N]` | | `{messages: [{seq, at, from, kind, text}]}`; channels are `lobby`, `table-<id>` and `game-<id>` |
| `POST /api/chat/{channel}` | `{text}` | 204 |
| `GET /api/users/{name}` | | a player's page: rating, tallies and latest games |
| `GET /api/leaderboard[?limit=N]` | | the rated players, best first |
| `GET /api/history[?user=name][&before=ended][&limit=N]` | | finished lobby games, newest first |

`POST /api/deal` deals the way a dominion.games table does: the required `kingdom` piles
and `landscapes`, one random pile from each set in `fromSets`, then randomizers from
`sets` (default: every released set) shuffled into one stack fill the kingdom up to ten
piles, a landscape going to a free slot that takes its kind (`slots`: `{kinds: ["event",
"landmark", "project", "way", "trait"], optional}`; the default is the rulebook's two: any
kind, then any but Way, both optional; a slot that isn't optional is always filled). Cards
in `banned` are never dealt. Creating a game with the result's `kingdom`, `landscapes` and
`seed` sets up exactly that kingdom, and `sets` alone with a seed deals what
`POST /api/games` deals from those sets with that seed.

`/assets/` serves the asset cache (`assets_dir`), then the web client's own bundled files
(`web_dir/assets`), and every other path the built web client (`web_dir`, falling back to
its `index.html`); without a cache, `/assets/manifest.json` is 204 No Content. These
routes accept the same Host headers as `/mcp`.

### Accounts and the lobby

The server can host strangers. Anyone may register an account (a name and a password,
kept as an argon2 hash), play as a guest (a name, or `Guest <number>`, and no password:
the browser or agent that made it holds its only session), and, once registered, make up
to five bots: accounts of their own for agents, which sign in with an API key
(`dk_...`). Names are 2 to 20 letters, digits, spaces, `_`, `-` and `.`, each one
person's, whatever its case. `registration = false` and `guests = false` close either
door. A guest who registers keeps their account and its games. Sessions last sixty days
unused (a guest's thirty, after which the guest is forgotten once its games are over).

Games made by `create_game` / `POST /api/games` stay what they were: their seats are
played by token, by whoever holds it. The lobby makes games between accounts:

- **Tables.** A host creates a table with its rules (`TableSpec`: what `POST /api/deal`
  takes, plus `colonies`, `shelters`, `turnLimit`, `randomizeOrder`, `respectBanned`,
  `rated`, `watchable`, `timed`, `undo`) and its seats: open ones anyone signed in may take, and
  token seats, which the host fills by handing out the seat's token once the game starts
  (a second player at the same screen, an agent without an account). The host changes
  the rules and the seats; changed rules make everyone unready. The game starts once no
  seat is open and every player has said they are ready, with a kingdom and a seed the
  server chooses; each player's account is handed its seat's token (`GET /api/lobby`,
  `GET /api/me/games`), so the game is played as any other. An account sits at one table
  at a time, a table goes with its last player, and tables don't outlive the server
  (their games do).
- **Automatch.** `POST /api/automatch` (or `find_match`) searches for a game: the player
  counts you accept, whether you want `humans`, `agents` (bots, and anyone connected over
  MCP) or `any`, and whether a rated game is welcome. Searches that fit each other are
  paired, closest in rating first, into a timed, watchable game with a random kingdom
  that leaves out the cards on its players' Banned lists. A search ends when its player
  disconnects.
- **Ratings and records.** A game counts for ratings (Glicko-2, a game a rating period,
  a day unplayed another) when it says so and every seat is a different registered
  account or bot; a bot against its owner, or two bots of one owner, play for nothing,
  and so does any game with a guest or a token seat. Every finished lobby game leaves a
  record (`GET /api/history`), which stays when the game itself is deleted.
- **Resigning and leaving.** A seat can resign (`POST /api/games/{id}/resign`, the
  `resign` tool): the game ends there with the resigner last and the others ranked by
  their scores. In a timed game the seat that must decide forfeits the same way when
  nobody has been connected for it for `away_minutes` (an open event stream or a waiting
  `wait_for_decision` is a connection), or when it takes longer than `decision_minutes`
  over one decision; `meta.forfeitAt` says when.
- **Watching.** Anyone signed in may watch a watchable game: the view of someone with no
  seat, seeing every player's hand, deck contents and other owned cards. Accounts
  playing in a running game must use their seat view instead.
- **Chat.** The lobby, each table (for those sitting at it) and each game (for its seats
  and, marked as such, those watching) have one; the last 200 messages of each are kept
  in memory.

### The server's own agents

A server can play agents using its account's native subscription logins: Claude Code,
Codex, and Gemini through Antigravity CLI. `dominion agents` connects to `agent_socket`
and offers the configured `[[agents]]` profiles in the lobby. A registered player can
seat them at a table, request one in Bot Game, or assign any seats through ordinary
`POST /api/games` / MCP `create_game`:

```json
{"players":2,"agents":["Claude Opus","Gemini"],"seed":42,"turn_limit":100}
```

`agents` has one entry per player; `null` creates a token seat. The creator receives
only those token seats. A profile may play multiple seats. Existing game options
configure the kingdom, landscapes, seed, Colonies/Shelters, and turn limit. These games
are unrated, disallow undo, and are watchable through the server by default; an explicit
`watchable: false` makes one private. They appear in the normal lobby games listing.
Each registered player fills `agent_seats_per_day` agent seats a day; the runner's
`agents_at_once` limits simultaneous seats. Guests and bot accounts cannot assign them.

Each model plays through a private, seat-scoped MCP server exposing the common
`state`, `step`, `wait`, `cards`, and `log` loop, plus compatibility `view`, `card`,
`answer`, `play_treasures`, and `wait_for_decision`. The bridge binds the session and
attaches its seat token;
the model cannot override the seat or create, inspect, or control other games. Native
tool calls and results drive the game; assistant prose is never parsed as an action.
One native CLI process and conversation plays each seat through the entire game.
The runner sends one initial observation; subsequent observations arrive as MCP tool
results. It does not rebuild or resend a manual conversation history for each move.
The provider CLI manages its conversation context and compaction.

Shell, filesystem, browser, delegation, hooks, other MCP servers, and host instructions
are disabled. On Linux, bubblewrap hides the host home and project files and clears
the environment, exposing only the runtime, a per-seat workspace, and the provider's
native login directory. Provider network access remains available. The runner fails
if Linux user namespaces or bubblewrap are unavailable. Codex uses a sanitized model
catalog without host tools; Claude and Antigravity expose only the Dominion MCP
connection alongside their completion mechanism.

Every seat attempt gets a private directory under `agents_dir/archives`. Its
`interactions.jsonl` records resolved game configuration, agent settings, every game
request/response, complete prompts, native events, failures, and the final view.
`response-*.stdout.jsonl` and `.stderr` preserve the CLI's complete output, including
any reasoning or usage events it exposes. Credentials are redacted from the game
journal and never sent in model prompts. Archives are never automatically pruned and
are independent of the server's game retention. `history_turns` is accepted for
compatibility but no longer affects native sessions. A runner restart starts a fresh
archived attempt for each seat still running. `response_seconds` limits model inactivity
between events, excluding time spent waiting for MCP tools; it is not a whole-game
time limit.
Transient seat failures retry after 5 seconds, then 10 seconds, with at most three
failed attempts. A forbidden tool attempt stops
the seat immediately. Each attempt has a separate archive; completed moves remain
in the game.

Unrated agent games also have a live transcript at `/game/<id>/agents`, opened in a
new tab from **Game menu → Agent transcript**. It combines streamed reasoning and
replies, collapsible model prompts/observations, the agent's requested Dominion
actions, and the resulting game events. Filter by seat or hide reasoning; **Follow
latest** keeps the newest output in view. Reasoning is shown only when the CLI exposes
it. This view uses the game's ordinary visibility: anyone signed in who can watch an
unrated game can read its transcript, including private cards and plans. Rated games
do not publish live transcripts.

The server stores transcript events in SQLite and notifies viewers over SSE. Cursor
pagination and stable attempt/offset identifiers make reconnects and upload retries
safe. CLI output is captured while the process is running. Each attempt also keeps
`transcript.jsonl` beside its full local archive. Uploads retry while the runner is
active; an unfinished upload at shutdown is reported in the archive, which retains
the complete local transcript. Runs archived before live publishing was added are
not automatically imported into the server.

Subscription sign-in is performed using the native CLI before starting the runner.
Antigravity CLI uses `agy`; the Antigravity IDE can also install a different `agy`,
so set `command` to the CLI's absolute path when both are installed. Consumer Gemini
subscriptions use Antigravity, not Gemini CLI. `auth_dir` selects a provider's native
login directory; it defaults to `~/.claude`, `~/.codex`, or `~/.gemini` (and respects
`CLAUDE_CONFIG_DIR`, `CODEX_HOME`, or `GEMINI_HOME`). Codex needs its native
`models_cache.json`, populated by starting Codex once after sign-in.

For an opt-in native subscription check, set `DOMINION_LIVE_AGENT` to `claude`, `codex`,
or `antigravity` and run `cargo test -p dominion-server --test native live_native_game
-- --ignored --nocapture`. Optional `DOMINION_LIVE_COMMAND` and `DOMINION_LIVE_MODEL`
select the executable and model. This runs a short game through the same isolated
MCP bridge and writes evidence under `/tmp/dominion-native-live`.
`DOMINION_LIVE_TURN_LIMIT` caps the game (default 2). To verify two different
profiles together, set `DOMINION_LIVE_OPPONENT` to an `AgentSpec` JSON object with
its own `name`, `program`, `model`, and optional `command` / `auth_dir`. The check
has a three-minute deadline and asserts one native process per seat.

What one address or account may do is limited: accounts registered and guests made per
hour, sign-in attempts, games created by hand, kingdoms dealt, table changes, messages.
Behind a reverse proxy set `trust_proxy` so the limits see the client's address, and
`public_url` so the session cookie is HTTPS-only. A request that changes something and
names another site as its `Origin` is refused. The server hosts at most `max_games`
unfinished games; finished ones are deleted after `keep_finished_days`, unfinished ones
nobody has touched after `keep_idle_days`.

Accounts, sessions and finished games' records are kept in a SQLite database beside the
games, `dominion.sqlite` under `state_dir` (in memory with `--ephemeral`): a game's
record and what it does to its players' ratings and tallies are written together or not
at all, and a server that stopped in between finds the record there when it starts again.
Tables, searches and chat are the running server's only. The Banned cards list is each
account's own; `banned_cards` in the settings file is the list of everyone who hasn't
set theirs.

Settings live in `$XDG_CONFIG_HOME/dominion/config.toml`; flags override them:

```toml
listen = "127.0.0.1"           # serve: address to listen on
port = 7317                    # serve: port
allowed_hosts = []             # serve: extra Host header values to accept
persist = true                 # serve: save games and resume them on restart
state_dir = "/srv/dominion/games"  # serve: where (absolute; default $XDG_STATE_HOME/dominion/games)
keep_finished_days = 7         # serve: delete finished games this old
assets_dir = "/srv/dominion/assets"  # assets sync, serve: asset cache (default $XDG_CACHE_HOME/dominion/assets)
web_dir = "/srv/dominion/web"  # serve: the built web client served at / (default $DOMINION_WEB_DIR, else none)
web_animations = true          # serve: the web client shows cards moving
web_animation_speed = "standard"  # serve: slow, standard, sprightly, swift, speedy or none (dominion.games' Animation Type)
web_move_kingdom = true        # serve: on other players' turns the kingdom drops and their play row goes above it
web_confirm_single = false     # serve: a single card to trash, discard, exile or topdeck waits for a button
web_background = "inheritance" # serve: the painting under the table, from the asset cache's images/large ("" for none)
web_sounds = true              # serve: the web client plays dominion.games' sounds (from the asset cache)
web_volume = 0.5               # serve: their volume, 0.0 to 1.0
banned_cards = []              # serve: dominion.games' Banned cards list, for everyone who hasn't set their own
public_url = "https://dominion.example"  # serve: where people reach it, behind a proxy: accepted as Host; https makes cookies HTTPS-only
trust_proxy = false            # serve: take clients' addresses from X-Forwarded-For (only behind a proxy that sets it)
registration = true            # serve: new accounts can be registered
guests = true                  # serve: people can play without an account
max_games = 1000               # serve: the most unfinished games at once
keep_idle_days = 30            # serve: delete unfinished games nobody has touched for this long
away_minutes = 5               # serve: in a timed game, the deciding player forfeits after this long disconnected (0: never)
decision_minutes = 30          # serve: ... or after this long over one decision (0: never)
session_idle_minutes = 1440    # serve: close an MCP session after this long without a call (0: never)
agent_socket = "/run/dominion/agents.sock"  # serve, agents: where the runner of the server's agents connects (default: none)
agent_seats_per_day = 5        # serve: seats one registered player may fill with the server's agents a day
agents_at_once = 2             # agents: seats the runner plays at once
agents_dir = "/srv/dominion/agents"  # agents: where the agents run and log (default $XDG_STATE_HOME/dominion/agents)
# agents: what the runner offers (default: Claude Opus, Claude Sonnet, Codex); `command`
# (default claude, codex, or agy on PATH) and `about` (for the lobby) are optional too
# [[agents]]
# name = "Claude Opus"
# program = "claude"           # claude, codex, or antigravity
# model = "opus"
# effort = "high"
# instructions = "Prefer engine strategies when the kingdom supports them."
# response_seconds = 300      # maximum model inactivity; MCP waits are excluded
# [[agents]]
# name = "Gemini"
# program = "antigravity"
# command = "/path/to/antigravity-cli/bin/agy"
# model = "gemini-4-argon-medium"       # use a model ID from `agy models`
server = "http://127.0.0.1:7317/mcp"  # play: the endpoint to connect to (default: at listen and port)
reconnect_seconds = 30         # play: how long to keep trying to reach a server that went away
```

On NixOS the flake's `nixosModules.default` runs it as a service: `services.dominion.enable`,
with `services.dominion.settings` holding the settings above (`state_dir` and `assets_dir`
default to `/var/lib/dominion/games` and `/var/cache/dominion/assets`),
`services.dominion.syncAssets` to fill the asset cache at boot, and
`services.dominion.agents` (`enable`, `user`, `settings`) to run the server's own agents
as `user`, with that account's native CLI logins (and subscriptions): it sets
`agent_socket` to `/run/dominion/agents.sock`, reachable by the `dominion` group, which
the runner's service joins.

`dominion play --new [--players 2] [--kingdom Village,Smithy,...]` creates a game, takes
its first seat in the terminal and prints the other seats' tokens;
`dominion play --game ID --token T` plays a seat of an existing game. Type option
numbers, optionally followed by `way none` or `way <Way>`, or `name <card>`. It rides out a
server restart: when it loses the server it keeps trying to reach it for
`reconnect_seconds` (`--reconnect`), then gives up. `dominion card NAME` prints a card's
text.

`dominion assets sync` downloads the card art, frame templates, icons, fonts and sounds
of [dominion.games](https://dominion.games) into a local cache (`assets_dir`, or `--dir`)
and writes `manifest.json` there, which maps every card to its art and templates and
lists what couldn't be found. It reads the card tables out of the site's current client,
skips files it already has (`--force` fetches them again) and prints what is covered;
`--dry-run` only reports what it would fetch. `dominion assets path` prints the
directory. These assets belong to their owners: they are only cached locally for
personal use and are never part of this repository or its package.

## The web client

`crates/dominion-web` is a browser client (Dioxus, compiled to wasm) for the same
server: a lobby to set up games and take seats, and a game page that shows a seat's view
and answers its decisions.

The lobby is laid out and behaves like dominion.games': a tab column on the left
(Automatch, New Table / My Table, Tables, Cardlists, Leaderboard, Account) over its
painting, with who is signed in at its foot. Nobody has to sign in to look around: what
needs an account (sitting down, searching, speaking, a Banned list) signs a guest in
where the server takes guests, and otherwise asks for a login (**Account**: log in,
register, keep a guest account by giving it a password, change the password, and make
bots, whose API key is shown once with the line to give an agent).

**My Table** is the server's table you sit at. Its host sets the rules: the ten Kingdom
slots (cards you require, a random card from one set, or left to chance) with the
kingdom written out under them, the landscape slots (each set to the kinds it may be
dealt, or emptied), the expansions random cards come from, Colonies and Shelters
(Random, Yes, No), whether the players' Banned cards lists apply, a turn limit, and
whether the game is rated, may be watched and is timed. "Select Kingdom Cards" opens
dominion.games' card selector: the expansions, a search (Enter adds a typed list,
`smithy, witch, Colonies` as the table writes it), Replay Last Kingdom, and Randomize,
which deals the random slots at once (`POST /api/deal`) so the kingdom can be changed
before Ready. Changes reach the server once the editing pauses, and what it refuses is
said under the rules. Everyone else at the table sees the rules without being able to
change them. The players are listed with their rating, whether they are here and
whether they are ready; the host adds and removes slots, removes a player, and fills a
slot with a token seat: one played at this screen too (hot seat), or one whose token is
handed out when the game starts (an agent without an account, another browser), with
its link and `join_game` line beside the players, or (a registered host, while the server
offers them) with one of the server's own agents, picked in the empty slot. The table has a chat. Ready says you
are; the game starts when everyone is and no slot is empty, and everyone is taken to
it. While it runs, My Table shows its kingdom and players, locked, with Play to return
to it; when the game waits on a token seat nobody has joined, the table and the game
page say so, with the line to give that seat's agent. The last rules you hosted are remembered in the browser for your next table.
**Automatch** searches for a game (player counts; anyone, people or agents; rated or
not) and goes to it when one is found; Bot Game is the same search for agents only, and
says which of the server's own agents a 1 Bot game may get (any, or the one picked). It
also has the lobby's chat, Create Table, and joining a seat by its token. **Tables**
lists the lobby's tables (Join where a slot is empty) and the games played without one
(Play where this browser holds a seat, Watch where watching is allowed); **Cardlists**
edits your account's Banned cards list; **Leaderboard** ranks the rated players, each
name leading to that player's page (`/user/<name>`: rating, tallies, latest games).
After a game the score page has the table's Ready (another game at the same table),
Edit Table and Leave Table, as dominion.games has.

The game page is a table laid out and behaving like dominion.games': player bars and
the basic piles on the left, the kingdom sorted by cost with the turn player's play row
under it, the status bar with the prompt and the phase's end button, a row of named
buttons above it, your hand at the bottom, and the log on the right in dominion.games'
wording. Cards are clicked where they lie, their borders coloured by what a click does;
revealed or looked-at cards open in a window; right-click studies a card; hovering a
tab opens the trash or a mat. Enter confirms, Escape clears, E ends a phase and T
autoplays Treasures. Cards move as dominion.games moves them, at its Animation Type
speeds (`web_animation_speed`, `web_animations`; none with `prefers-reduced-motion`),
and its sounds play when it plays them: your turn coming back to you and the end of the
game (`web_sounds`, `web_volume`, and a mute toggle on the page for the session). A Plain toggle shows the same game as text
and buttons.

Opening `/game/<id>` shows the seat this browser holds, else the seat the account
signed in plays there (no token in the address), else, when the game is watchable, the
game as someone watching sees it (signing in as a guest if nobody is signed in): the
same table with the first player where your own would be, every hand face up and
nothing to answer. Clicking any player’s deck opens its composition: all owned cards,
remaining deck, hand, discard, in play, mats and set-aside cards. Seated players see
only their knowledge, including deductions and uncertain locations; spectators see
the full contents. In a game between accounts each player's bar has their rating (or
`guest`, `bot`) and whether they are connected; when the player who must decide has
left, the status bar counts down to their forfeit ("Bo has left: forfeits in 0:47"), as
it does for anyone's last minute, your own included. The game's chat is under the log,
with the lines of those watching marked. The menu (☰) has Resign, which asks once more
before it ends the game; the end notice and the score say who resigned or forfeited.

The client keeps the seat tokens it holds in the browser's localStorage; a seat link
`/game/<id>#token=<token>` opens a seat in another browser. Cards are drawn like
dominion.games' own from the asset cache (`dominion assets sync`), which the client only
references by URL; without a cache they are drawn in CSS and the table is silent.
`/gallery` shows every card at each size.

```sh
just web-build                  # release build into target/web-client
just web-serve --port 7317      # build, then: dominion serve --web-dir target/web-client ...
just web-dev 127.0.0.1:7317     # dx dev server with hot reload, proxying /api and the
                                # asset cache to a dominion server at that address
```

The flake builds it too: `nix build .#web` is the same bundle as `just web-build`, and
`nix run .#dominion-with-web -- serve` runs `dominion` with it. That wrapper sets
`DOMINION_WEB_DIR` to the packaged client, which `serve` uses only when neither
`--web-dir` nor `web_dir` says otherwise. Neither package contains the asset cache: fill
it with `dominion assets sync`.

## Development

`nix develop` (or direnv) provides the toolchain. Run `just check` before
committing. See [AGENTS.md](AGENTS.md) for the architecture and invariants.

Dominion is a trademark of Rio Grande Games. This project is unaffiliated with
them.

Licensed under either of MIT or Apache-2.0 at your option.
