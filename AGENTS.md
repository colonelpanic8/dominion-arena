# dominion-engine

A headless Rust rules engine for Dominion: every card from every expansion, played
exactly by the current rules (second editions / Dominion Online, plus the cards removed
in second editions). It ships no bots and no UI; anything that plays implements `Agent`.

## Checks

From the repo root (direnv loads the Nix dev shell; otherwise prefix `nix develop -c`):

- `just check`: fmt-check, clippy `-D warnings` (and dominion-web for wasm32), tests.
- `just web-build` / `web-serve` / `web-dev`: build, serve or hot-reload the web client.
- `just fmt`: format Rust and `flake.nix`.
- Nix: `packages.default` is `dominion`, `packages.web` the client (dx in the sandbox,
  with wasm-bindgen pinned to the crate's version), `packages.dominion-with-web` the binary
  wrapped with `DOMINION_WEB_DIR`. Bumping wasm-bindgen means bumping both the crate pin
  and `wasm-bindgen-cli_*` in `flake.nix`. `nix flake check` builds all but the wrapper.
- Focused: `cargo test -p dominion-engine tests::<set>`.

## Layout

```
data/cards/<set>.json     card data, the source of truth (see "Card data")
crates/dominion-engine/
  build.rs                generates Card, Types, Set and the card table from data/cards
  src/card.rs             public card API: Card, CardDef, Cost, Kind, Types
  src/game.rs             Game state, queries, primitive state changes, scoring
  src/frame.rs            the effect stack; turn, play, gain, buy, draw, trash, ... frames
  src/trigger.rs          Occasion, Trigger, and the trigger window
  src/decision.rs         Decision, DecisionKind, Move, Opt, Response, validation
  src/setup.rs            GameConfig, setup, random kingdoms
  src/agent.rs            Agent trait, play() driver, View (one player's legal knowledge)
  src/event.rs            the public event log
  src/cards/mod.rs        SetRules trait, dispatch, CardFrame and its helpers
  src/cards/<set>.rs      one module per set: its State, Rules and card scripts
  src/tests/mod.rs        engine tests, shared helpers, the random-game fuzz test
  src/tests/<set>.rs      each set's card tests
  src/trace.rs            opt-in id-level trace of card moves, shuffles and reveals
  src/knowledge.rs        per-player perfect-recall knowledge built from the trace
crates/dominion-protocol/ serde types of agent-view@2, the HTTP API and the asset manifest; engine-free,
                          builds for wasm
crates/dominion-agent/    the dominion/agent-view@2 protocol: Table, projection, text
  protocol/               the vendored JSON Schema (`just sync-protocol`)
crates/dominion-server/   the `dominion` binary: MCP server (rmcp, axum) and terminal client
  src/http.rs             the HTTP API (/api), asset cache (/assets) and web client routes
crates/dominion-web/       the browser client (Dioxus 0.7, `web` feature for wasm32); depends
                          only on dominion-protocol (engine crates are dev-dependencies)
  src/api.rs              typed client for /api and the SSE change feed
  src/session.rs          seat tokens held in localStorage
  src/cards.rs            /api/cards data by name (provided as `app::Cards` context)
  src/assets.rs           /assets/manifest.json and the card fonts (`use_assets()`); absent cache is fine
  src/card/               the card renderer: CardView / CardBack / CardHover at Full, Mini, Micro
    text.rs               official text -> lines of styled words and icons (pure)
    fit.rs                word wrapping and font-size fitting in card-width units (pure)
    layout.rs             where things sit on a card, as fractions of its size (upstream's boxes)
    measure.rs            text widths: canvas in the browser, an estimate natively
  assets/cards.css        card styles (cqw units; images through --dc-* variables)
  src/gallery.rs          /gallery: every card through the renderer, with filters
  src/game/state.rs       the game page's model (view, version, log), selection -> Answer
  src/game/mod.rs         GamePage: seat choice, keeping state current, `GameCtx` context
  src/game/plain.rs       the plain text-and-buttons page and `DecisionControls`
  src/game/interaction.rs where each option is answered on the table (`Target`, `Place`),
                          "Play Treasures" (`TreasureRun`)
  src/game/board.rs       a view arranged for drawing: piles, players, stacks, status
  src/game/logfmt.rs      the log as display lines: turn headers, depth, card spans
  tests/table.rs          those three models property-tested over random real games
  src/lobby.rs            game list, new game (the kingdom builder), join by token
  src/kingdom.rs          the builder's model: piles by set, tray, randomizer, NewGame (pure)
  assets/lobby.css        lobby styles
  tests/kingdom.rs        the randomizer and request building against the engine's cards
tools/import_cards.py     one-off importer that produced data/cards (kept for reference)
```

## Card data

`data/cards/<set>.json` holds every card-shaped object: Kingdom and basic cards,
non-Supply cards, landscapes, Boons, Hexes, States, Artifacts and pile randomizers. Each
entry has `name`, `set`, `types`, official `text` (lines separated by `\n`, `---` for
the dividing line), and as applicable `cost` (`coins`, `potions`, `debt`, `variable`,
`overpay`), `plus` (leading vanilla bonuses: `cards`, `actions`, `buys`, `coins`,
`villagers`, `coffers`, `favors`), `coins` (a Treasure's value), `potions`, `vp`,
`kingdom`, `basic`, `removed`, and `pile` (the randomizer or top card of its mixed or
split pile). `build.rs` turns it into `Card` (one variant per entry, named by
CamelCasing the name), `Types` and the `CardDef` table. Arcana is unreleased: its entries are names and types only, none are Kingdom cards yet, and it has no behavior; re-import it once its cards are public. Edit the JSON to fix data; the
vanilla bonuses were parsed automatically and must be right, because the engine applies
them.

## Architecture

- **Driving a game.** `Game::new` runs until the first real decision. `pending()` returns
  it; `respond()` validates the answer and runs until the next one. Decisions with one
  meaningful answer are answered automatically (`Decision::forced`). A game is determined
  by its config (with seed) and its responses.
- **The effect stack.** Anything that can be interrupted by a decision or a trigger runs
  as a `Frame`: a coroutine whose `pc` records where to resume. A frame returns
  `Flow::Ask` (a decision), `Flow::Call` (push a child; its result comes back as
  `Input::Return`), `Flow::Continue` (re-enter now) or `Flow::Done(value)`.
- **Turns.** Start (`StartTurn` window, then `Until::StartOfTurn` lastings made before
  this turn expire),
  Action (moves: play an Action, spend a Villager, end), Buy (`StartBuyPhase`; play
  Treasures until something is bought, buy cards / Events / Projects; `EndBuyPhase`),
  Night (`StartNight`; play Night cards), Clean-up
  (`StartCleanup`, `DiscardFromPlay` for each player with cards in play to discard,
  discard everything not kept by a Lasting, draw
  `turn.cleanup_draw`, `EndTurn`, `Until::EndOfTurn` lastings expire). Spending Coffers
  and paying off Debt are moves in the Action, Buy and Night phases alike. Extra turns queue
  in `Game::extra_turns`; each one's `ExtraLimit` says when it fails (not a third turn
  in a row: check `g.third_in_a_row(p)` before queuing one; Possession: not a second
  extra turn in a row; Seize the Day: never), and extra turns
  don't count for the tiebreaker. Between turns the player who just took a turn takes
  their extra turns first, then the others in turn order; one with different extra turns
  pending chooses which to take (`Purpose::Either`, `Opt::Card(source)`), and an extra
  turn that can't follow the last turn is lost. Once the game has ended, extra turns
  happen only before a turn of Fleet's round. `Player::skip_turns` skips the next turns
  a player would take (Lich), Fleet's included; skipped regular turns count for the
  tiebreaker. `Turn::controller` makes someone else decide and see
  the player's cards (Possession, which also replaces gains, trashing through
  `SetRules::trash_instead` and Debt through `SetRules::debt_taker`).
- **Playing a card** (`Frame::play`, the only way): move it into play, `Play` window
  (triggers may set `g.plays.last_mut()`'s `replaced` / `blocked`: they are the card's
  player's, and the one resolved last applies), choose a Way if the game has Ways (it
  can be used instead of either), open an `Attack` window per other player for Attacks (in turn
  order from the turn's player; also when the play is replaced or blocked: Reactions
  answer the card being played), then apply the
  vanilla bonuses (`CardDef::plus`, Treasure `coins` / `potions`, through
  `SetRules::modify_bonus`; a Way's own instead, unmodified; an Omen's +1 Sun comes
  before them: `cards::prelude`) and run the card's `Hook::Main`, then the `AfterPlay`
  window. `chain` lists the cards playing it (Throne Room ...); pass
  `f.chain_for_child()` when a card plays another more than once (only then does it stay
  in play with a Duration).
  `SetRules::can_play` can forbid a play from the hand (Warlord, Voyage): `Frame::play`
  then does nothing, so offer only what may be played (`g.playable`; `cards::can_play`
  for a Reaction that plays itself from the hand). `Turn::hand_plays` counts the turn's
  plays from the hand.
- **Gaining** (`Frame::gain`, `Frame::gain_card`): `WouldGain` window (replacement
  effects edit `g.gains.last_mut()`: `to`, `player`, `cancelled`), move (recording the
  card's cost then in `GainState::cost` and the phase in `GainState::phase`: a gain
  "in your Buy phase" is tested with `g.gain_phase(id)`, because Cavalry and Villa change
  `turn.phase` while their gain's window is open), `Gain` window. An effect that goes on
  with the card it gained asks `g.gained_to(p, id)` where `p` gained it to (`None` if
  someone else gained it instead: Possession) rather than assuming the discard pile.
  **Buying** (`Move::Buy`): pay (coins, Potions, Debt taken), overpay
  (`Frame::overpay`: $, then Potions; the card's `Hook::Ability(frame::OVERPAY)` with
  `f.data` = the $ and `f.s.cost` = all of it), `Buy` window (`Occasion::Buy` carries
  what the card cost when bought), then gain the pile's top card if it still has the
  bought card's name (an on-buy ability may have gained the copy that was on top; what
  was bought is a card of that name, not one physical copy).
  Events and Projects: pay, `BuyLandscape` window, then the landscape's `Hook::Main`
  (a Project also gets the buyer's cube in `Player::projects`).
- **Triggers** (`trigger.rs`). Each set's `SetRules::triggers(g, occ, out)` adds the
  `Trigger`s that apply right now; the window resolves them (turn player first, then
  in turn order, each player ordering their own; optional ones can be declined) and
  re-asks after each resolution, so check every condition (including where the
  card is: the stop-moving rule) when producing a trigger. An effect that stacks ("this
  turn, when you gain ...") produces one trigger per lasting effect or card in play,
  never one trigger carrying a count: the window tracks what has resolved per trigger,
  and more can come into effect while it is open. It doesn't ask for an order
  when a player's mandatory triggers all have the same effect, and resolves
  `Trigger::quiet` ones (bookkeeping, or bonuses nothing can interact with, like Fair's
  +1 Buy) first without asking. A trigger runs as the card's
  `Hook::Ability(n)` with `f.occ`, `f.lasting`, `f.data` set. Finishing an `Attack`
  trigger with `Value::Bool(true)` makes the target unaffected. Players choose among
  triggers by card name: give one `Trigger::labelled` when the name doesn't say what it
  is (the Exile rule on a gained card, which card a Way's ability is on).
- **Lasting effects** (`Lasting`): anything that outlives a play. Durations call
  `f.add_duration(g, data)` (keeps the card in play with what played it:
  `CardFrame::keeps`) only when the play has something left to do, and produce a `StartTurn` trigger from that lasting
  (`cards::due_durations` lists the ones due: made on an earlier turn of the owner's;
  `cards::turn_durations` lists them all, for "at the start of each of your turns",
  which includes a turn the card is played at the start of;
  `cards::duration_trigger` builds the trigger), whose ability removes it
  (`f.end_lasting(g)`). "This turn" effects use `Until::EndOfTurn`, "until your next
  turn" `Until::StartOfTurn(owner)`. Sets read `g.lasting()` for cost changes, triggers,
  immunity and so on.
- **Modifiers.** Rules code asks the game, never the static data: `g.cost_of(id)` /
  `g.cost(card)` (through `SetRules::modify_cost`; Events and Projects always cost what
  they say), `g.types(id)` / `g.is(id, types)`
  (through `modify_types`), `g.card(id)` (what a card currently is: `Game::masks` make
  Band of Misfits-style cards another card until they leave play; `g.printed(id)` is
  what is printed). `SetRules::behavior` lets a card use another card's abilities
  (Inheritance). Effects that look back at the turn ("the first Action you play", "the
  third Action you gain") count the types cards had then: `Turn::played_types` /
  `gained_types` (`played_of`, `gained_of`), never `Card::is` on `played` / `gained`.
  "During your turns" and "this turn" modifiers ask `g.turn_player()`, which is no one
  between turns and when the game is scored, never `g.turn.player`.
- **State.** Generic state is on `Game`, `Player` (mats, Coffers, Villagers, Favors,
  Debt, VP tokens, Projects, States, Adventures tokens, ...), `Pile` (VP / Debt /
  Embargo / Trade Route / player tokens, Traits) and `Turn`. Take Debt with
  `g.add_debt` (never `debt +=`), so Possession can redirect it. A set's own state goes in
  its `State` struct (`g.ext.<set>`). Everything must stay `Clone + Serialize`.
- **Shuffling** goes through `Frame::shuffle` (a discard pile into its deck) or
  `Frame::shuffle_deck` (a whole deck: Inn, Famine), never by hand: `WouldShuffle`,
  shuffle, cards `SetRules::bottom_when_shuffled` says go to the bottom (Shadow cards, in
  their owner's order), then `Shuffled { count }` (`g.shuffled(p)`, the bottom cards of
  the deck, is the shuffled ones still in it; "when you shuffle" placements such as Stash,
  Star Chart and Fated resolve there). One draw shuffles at most once. A script step that must draw before its
  own flow goes on calls the draw and `CardFrame::hold`s that flow (Way of the Chameleon).
- **Decisions** are generic: `Turn { moves }`, `Cards`, `Pile`, `Choose` (options such as
  `Opt::Yes`, `Opt::Mode(i)`, `Opt::Card`, `Opt::Trigger`), `NameCard`, `Number`. Each
  carries a `Purpose` and its `source` card. Never add a card-specific decision kind.
  `SetRules::modify_decision` may change what a card ability asks: Elder adds one pick
  to a `Purpose::Mode` "choose" (`ask_modes`), so scripts must handle every chosen mode,
  in printed order (`mode_mask` / `take_mode`). Alternatives that aren't a "choose"
  ("either ... or", which Artifact) use `ask_either` (`Purpose::Either`).
- **Agents** get a `View`, never the `Game`: everything public (piles and their
  tokens through `PileView`, each player's tokens, mats and zones through
  `PlayerView`, landscapes, Artifacts, the Prophecy, Boon/Hex discards, lasting effects,
  extra turns, sets' `SetRules::notes`), plus what that player may look at. Never hand
  out card ids from hidden places: ids are allocated deterministically, so an id gives
  its card away.

- **Knowledge.** `Game::new_traced` records a `Trace`: every card created and moved (with
  its zones), cards mixed where no one can follow them (shuffles, privately ordered
  cards), cards shown to everyone or one player, turns, identity swaps. `Knowledge`
  consumes it and keeps, per player, groups of cards they can't tell apart with each
  group's known identities (a Stash, with its own back, is always known). Anything that
  reveals a specific card must record it: `g.reveal(p, id)` instead of emitting
  `Event::Revealed` by hand, `g.show` / `g.show_to` for other reveals and looks. Cards
  that change zones must go through `move_card` (or record a `Trace::Moved`); identity
  changes record `Trace::Changed`. Untraced games pay nothing.
- **Agent protocol.** `dominion-agent` projects a game and a seat's `Knowledge` onto
  agent-view@2 and maps option indices back to `Response`s. The schema is owned by
  `dominion-agents/protocol/`; change it there first, then `just sync-protocol`. The
  tests validate every view, decision and answer against the vendored copy.

## Writing a set

A set module (`src/cards/<set>.rs`) has `State`, `Rules: SetRules` and its card
scripts. A card script is `fn(&mut Game, &mut CardFrame, Input) -> Flow` matching on
`f.pc`; see `cards/base.rs`. Conventions:

- Number each card's abilities with module consts (`const HAVEN_START: u8 = 0;`); match
  `(f.card, f.hook)` in `run`.
- Use frames for anything cards react to: `Frame::draw / reveal / look / gain /
  gain_card / trash / discard / play / receive / shuffle`. Primitive helpers
  (`g.move_card`, `top_deck`, `put_in_hand`, `set_aside`, `put_on_mat`,
  `return_to_pile`, `add_coins`) trigger nothing.
- "When you discard ... from play" abilities answer `DiscardFromPlay { player }` for the
  cards in `g.leaving_play(player)`: in Clean-up the cards nothing keeps in play (each
  player's, not only the turn's), at other times the cards given to
  `Frame::discard_from_play` (Highwayman). Effects that only use that window as a
  moment of Clean-up check `g.cleaning_up(player)`.
- `+$` goes through `g.add_coins(p, n)` (the -$1 token), +Actions and +Buys through
  `g.add_actions(p, n)` and `g.add_buys(p, n)`: all three do nothing for a player whose
  turn it isn't, so never add to `g.turn` directly. Decisions are asked with the
  `CardFrame::ask_*` helpers.
- "Draw until you have N cards in hand" is `Frame::draw_to(p, n)`, never a draw of
  `n - hand`: the -1 Card token replaces a draw without making the hand smaller.
- Set cards aside face up with `g.set_aside` (or `g.set_aside_revealed` for a card
  just revealed); `move_card` to `SetAside` leaves it face down, seen only by its owner.
  Public state kept in the set's `State` goes to agents through `SetRules::notes`.
- Respect the stop-moving rule: before moving a card an effect "expects" somewhere,
  check `g.zone(id)` is still that zone. That includes cards from `Frame::look` /
  `Frame::reveal`: they stay on the deck, where a "when trashed" draw in the middle of
  the effect can take one. An on-gain ability that moves the gained card (or plays it)
  checks `g.still_where_gained(id)`.
- `Frame::reveal` / `g.reveal` only where the text says "reveal" (Patron reacts to it);
  "discard / trash the top card" takes it with `Frame::look`.
- Setup: `build_kingdom_pile` for unusual Kingdom piles, `modify_starting_deck` for
  Heirlooms/Shelters-like changes, and two phases (`setup::complete`). `ensure` adds
  what the cards in the game need (extra piles, Boons/Hexes, set-aside cards, an Ally
  or Prophecy): it asks `setup::in_game`, never which set added a card, and adds only
  what is missing, because it runs until nothing changes, at the start and again for
  cards joining mid-game (Divine Wind). `setup` then runs once for what joined
  (`Setup::joined`): tokens on piles, starting resources. Random setup choices go
  through `Setup::choose`, which applies `GameConfig::setup_choices`: pass it every
  card the rules allow (a configured choice can be any of them; a random one is never
  an unused removed card), and narrow only the random pick with `choose_or_random`.
  `Pile::removed` marks a pile out of the game (or a setup holder): name lookups skip it.
  What marks a pile keeps applying to its cards (the Bane, Obelisk, Traits): ask
  `g.origin_pile(card)`. "In games using this" asks `setup::in_game`, not for a pile.
- A pile's types are those of the card it is named after (`Pile::card`: its randomizer
  or top card), whatever is in it now: that decides an "Action Supply pile".
- "In an Action phase" asks `g.in_action_phase()`: the start of a turn is part of it
  (for a gain, `g.gain_phase(id)` and `Phase::is_action`).
- "Play a card from your hand" offers `g.playable(p, types)`, which includes cards
  playable as if in hand (Shadow cards in the deck). "Return to your Action phase" is
  `Frame::return_to_action`, never a write to `turn.phase`: the Buy phase ends there
  (`EndBuyPhase`), and a turn can have several. `Turn::gained_this_buy_phase` is the
  current one's gains, `Turn::gained_in_buy_phase` every Buy phase's.

Tests: `src/tests/<set>.rs` (register it in `tests/mod.rs` and add the set to
`IMPLEMENTED`, which feeds the fuzz test). Give every card with more than vanilla
bonuses a focused test, plus tests for tricky rulings and interactions. Tests use the
helpers in `tests/mod.rs` (`setup`, `hand`, `deck`, `play`, `choose_kinds`, `yes`, ...).

## Invariants

- Every physical card (`CardId`) is in exactly one zone and `Game::zones` agrees. The
  fuzz test checks this at every decision. Cards held by an effect go in a real zone
  (`SetAside`, a mat), never nowhere.
- `Game` is `Clone + Serialize` at every point. Frames and state hold plain data only.
- All randomness comes from `Game::rng`.
- Primitive helpers on `Game` never trigger anything; anything a card can react to goes
  through its frame.
- One way to play a card: `Frame::play`.
- Rules code asks the game for types, costs and identity per card instance.
- No bots or AI strategies in this repo. Tests may use throwaway responders.

## References

- Official text: `data/cards`. Rulings: the Dominion Strategy wiki (one page per card),
  and `~/projects/dominion-agents/context/rules.md` for the general rules.
- Prior art: DominionSim-v2 (Rust, no license: read its docs for rulings and pitfalls,
  never copy its code), pydominion, Geronimoo's DominionSim.
