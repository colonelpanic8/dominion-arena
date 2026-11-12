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
| `GET /api/settings` | | the web client's settings: `{webAnimations, webAnimationSpeed, webMoveKingdom, webConfirmSingle, webBackground, webSounds, webVolume, bannedCards}` |
| `PUT /api/settings/banned-cards` | `["Witch", ...]` | the settings, after writing `banned_cards` to the settings file (409 when the server has none) |
| `GET /api/games` | | the games, as `list_games` gives them |
| `POST /api/games` | `create_game`'s arguments | 201, the game id and seat tokens |
| `POST /api/deal` | `{players, sets?, kingdom?, fromSets?, landscapes?, slots?, banned?, seed?}` | `{kingdom, landscapes, seed}`: the kingdom those rules deal, without creating a game |
| `GET /api/seat` | | `{gameId, seat, name}` of the token |
| `GET /api/games/{id}/view[?logFrom=N]` | | `{view, version, deciding, table}`; with `logFrom`, the log from entry N. `table` holds what the graphical table draws beside the view: each player's cards in play in the order played and the top of their discard pile, as far as the seat may see them |
| `POST /api/games/{id}/answer` | `{decision, indices}` or `{decision, name}` | the same, after it |
| `GET /api/games/{id}/events` | | server-sent `change` events with `{version}`: on connect and after every change |

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

Settings live in `$XDG_CONFIG_HOME/dominion/config.toml`; flags override them:

```toml
listen = "127.0.0.1"           # serve: address to listen on
port = 7317                    # serve: port
allowed_hosts = []             # serve: extra Host header values to accept
persist = true                 # serve: save games and resume them on restart
state_dir = "/srv/dominion/games"  # serve: where (absolute; default $XDG_STATE_HOME/dominion/games)
keep_finished_days = 7         # serve: delete finished games this old at startup
assets_dir = "/srv/dominion/assets"  # assets sync, serve: asset cache (default $XDG_CACHE_HOME/dominion/assets)
web_dir = "/srv/dominion/web"  # serve: the built web client served at / (default $DOMINION_WEB_DIR, else none)
web_animations = true          # serve: the web client shows cards moving
web_animation_speed = "standard"  # serve: slow, standard, sprightly, swift, speedy or none (dominion.games' Animation Type)
web_move_kingdom = true        # serve: on other players' turns the kingdom drops and their play row goes above it
web_confirm_single = false     # serve: a single card to trash, discard, exile or topdeck waits for a button
web_background = "inheritance" # serve: the painting under the table, from the asset cache's images/large ("" for none)
web_sounds = true              # serve: the web client plays dominion.games' sounds (from the asset cache)
web_volume = 0.5               # serve: their volume, 0.0 to 1.0
banned_cards = []              # serve: dominion.games' Banned cards list (the web client's Cardlists tab writes it)
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
server: a lobby to create games and take seats, and a game page that shows a seat's view
and answers its decisions.

The lobby's kingdom builder picks the sets, then either leaves the kingdom to the server
(ten piles dealt at random from those sets) or fills a tray of ten slots by clicking
cards, with Randomize filling the unlocked slots. A chosen kingdom is played exactly as
given, also with fewer or more than ten piles; its landscapes are the ones in the tray,
plus an Ally or Prophecy the kingdom needs.

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
