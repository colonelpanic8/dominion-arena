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

Settings live in `$XDG_CONFIG_HOME/dominion/config.toml`; flags override them:

```toml
listen = "127.0.0.1"           # serve: address to listen on
port = 7317                    # serve: port
allowed_hosts = []             # serve: extra Host header values to accept
persist = true                 # serve: save games and resume them on restart
state_dir = "/srv/dominion/games"  # serve: where (absolute; default $XDG_STATE_HOME/dominion/games)
keep_finished_days = 7         # serve: delete finished games this old at startup
session_idle_minutes = 1440    # serve: close an MCP session after this long without a call (0: never)
server = "http://127.0.0.1:7317/mcp"  # play: the endpoint to connect to (default: at listen and port)
reconnect_seconds = 30         # play: how long to keep trying to reach a server that went away
solo_dir = "/srv/dominion/solo"    # solo: where its games are kept (default $XDG_STATE_HOME/dominion/solo)
```

`dominion play --new [--players 2] [--kingdom Village,Smithy,...]` creates a game, takes
its first seat in the terminal and prints the other seats' tokens;
`dominion play --game ID --token T` plays a seat of an existing game. Type option
numbers, optionally followed by `way none` or `way <Way>`, or `name <card>`. It rides out a
server restart: when it loses the server it keeps trying to reach it for
`reconnect_seconds` (`--reconnect`), then gives up. `dominion card NAME` prints a card's
text.

## Playing a deck out alone

`dominion solo` plays one seat of a game whose other seats sit idle: they end every phase,
decline what they may and otherwise take the first options offered. It needs no server.
The game is a file, so each command is one step of a script, and it answers "how fast is
this deck with nobody in the way" with real shuffles and the engine's rules.

```sh
dominion solo new --kingdom Village,Smithy,Market,... [--landscapes ...] [--seed 7]
dominion solo do "treasures; buy Silver; next"
dominion solo do "play Village, Smithy x2; treasures; buy Province, Market; next"
dominion solo show [--log]     # the whole game as it stands; --log adds everything so far
dominion solo undo             # take back the last `do`
```

`new` also takes `--colonies`, `--shelters`, `--players N` and `--setup CARD=CHOICE` for
what setup would choose at random (`--setup "Young Witch=Cellar"`,
`--setup "Reckless=Smithy"`). `--game NAME` keeps several games side by side.

`do` takes steps separated by `;` and stops at the first that can't be done, keeping the
ones before it:

- `play A, B x2`: play cards, the Action phase's or Treasures. `play A as Way of the Ox`
  uses a Way; otherwise a card is played as itself.
- `treasures`: play every Treasure in hand.
- `buy A, B`: buy cards, Events or Projects, paying off Debt first when there is any.
- `pay` pays off Debt; `coffers N` and `villagers N` spend tokens.
- `end` ends the Action phase (and does nothing in a later one) and `next` the turn. A turn with nothing left to do ends by itself.
- `pick A, B x2` answers any question by card name, option text or option number, a
  turn's moves included; `pick none` chooses nothing. `name CARD` answers "name a card".

A step that belongs to a later phase ends the current one first. `do` prints what
happened, your cards and the open question; `--full` adds the Supply, the landscapes and
the idle seats, as `show` does. A step that can't be done says what is open instead.

## Development

`nix develop` (or direnv) provides the toolchain. Run `just check` before
committing. See [AGENTS.md](AGENTS.md) for the architecture and invariants.

Dominion is a trademark of Rio Grande Games. This project is unaffiliated with
them.

Licensed under either of MIT or Apache-2.0 at your option.
