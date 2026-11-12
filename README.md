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
[dominion-agents](https://github.com/colonelpanic8/dominion-agents) (`protocol/`): each
seat gets a JSON view of everything it may know, with perfect recall (cards it has seen
stay known until it can no longer tell them apart), plus its open decision as indexed
options; it answers with option indices. A `Table` hosts a game this way; the crate also
renders views as compact text for language models.

The `dominion` binary (`crates/dominion-server`) serves games over MCP (streamable HTTP):

```sh
nix run . -- serve                 # or: cargo run -p dominion-server -- serve
claude mcp add --transport http dominion http://127.0.0.1:7317/mcp
```

Tools: `create_game` (returns one secret token per seat), `join_game`, `view`,
`wait_for_decision`, `answer`, `card`, `list_games`. A seat's token is the only way to see
or act for it; pass it to each call. `join_game` binds a seat to the MCP session, so that
later calls in it may leave the token out; the binding ends with the session (when the
server restarts, or after `session_idle_minutes` without a call), so keep the token. Views
come back as structured JSON plus their text rendering. Arguments a tool doesn't take are
refused, as are `create_game` options that don't fit together.

Games survive restarts: the server saves each game (with its seat tokens, so the files
are private to you) to `$XDG_STATE_HOME/dominion/games/<id>.json` after every change and
resumes them on startup. Finished games are deleted on a later startup once they are a
week old. One server uses the directory at a time. `--ephemeral` keeps games in memory
only.

The same server has a JSON API under `/api` for browser clients, which act for a seat
the way MCP agents do (types in `crates/dominion-protocol`). A seat authenticates with
its token as `Authorization: Bearer <token>` (or `?token=` on the event stream). Errors
are `{"error": "..."}` with 400 (bad request), 401 (missing or wrong token), 404 (unknown
game or route), 409 (stale decision, or not this seat's), 422 (invalid answer) or 500.

| Route | Body | Returns |
| --- | --- | --- |
| `GET /api/cards` | | every card-shaped object as printed |
| `GET /api/games` | | the games, as `list_games` gives them |
| `POST /api/games` | `create_game`'s arguments | 201, the game id and seat tokens |
| `GET /api/seat` | | `{gameId, seat, name}` of the token |
| `GET /api/games/{id}/view[?logFrom=N]` | | `{view, version}`; with `logFrom`, the log from entry N |
| `POST /api/games/{id}/answer` | `{decision, indices}` or `{decision, name}` | `{view, version}` after it |
| `GET /api/games/{id}/events` | | server-sent `change` events with `{version}`: on connect and after every change |

`/assets/` serves the asset cache (`assets_dir`) and every other path the built web
client (`web_dir`, falling back to its `index.html`). These routes accept the same Host
headers as `/mcp`.

Settings live in `$XDG_CONFIG_HOME/dominion/config.toml`; flags override them:

```toml
listen = "127.0.0.1"           # serve: address to listen on
port = 7317                    # serve: port
allowed_hosts = []             # serve: extra Host header values to accept
persist = true                 # serve: save games and resume them on restart
state_dir = "/srv/dominion/games"  # serve: where (absolute; default $XDG_STATE_HOME/dominion/games)
keep_finished_days = 7         # serve: delete finished games this old at startup
assets_dir = "/srv/dominion/assets"  # serve: the asset cache served at /assets (default $XDG_CACHE_HOME/dominion/assets)
web_dir = "/srv/dominion/web"  # serve: the built web client served at / (default none)
session_idle_minutes = 1440    # serve: close an MCP session after this long without a call (0: never)
server = "http://127.0.0.1:7317/mcp"  # play: the endpoint to connect to (default: at listen and port)
reconnect_seconds = 30         # play: how long to keep trying to reach a server that went away
```

`dominion play --new [--players 2] [--kingdom Village,Smithy,...]` creates a game, takes
its first seat in the terminal and prints the other seats' tokens;
`dominion play --game ID --token T` plays a seat of an existing game. It rides out a
server restart: when it loses the server it keeps trying to reach it for
`reconnect_seconds` (`--reconnect`), then gives up. `dominion card NAME` prints a card's
text.

## Development

`nix develop` (or direnv) provides the toolchain. Run `just check` before
committing. See [AGENTS.md](AGENTS.md) for the architecture and invariants.

Dominion is a trademark of Rio Grande Games. This project is unaffiliated with
them.

Licensed under either of MIT or Apache-2.0 at your option.
