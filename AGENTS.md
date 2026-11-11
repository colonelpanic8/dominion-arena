# dominion-engine

A headless Rust rules engine for Dominion: every card from every expansion, played
exactly by the current rules (second editions / Dominion Online, plus the cards removed
in second editions). It ships no bots and no UI; anything that plays implements `Agent`.

## Checks

From the repo root (direnv loads the Nix dev shell; otherwise prefix `nix develop -c`):

- `just check`: fmt-check, clippy `-D warnings`, tests.
- `just fmt`: format Rust and `flake.nix`.
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
- **Turns.** Start (`StartTurn` window, then `Until::StartOfTurn` lastings expire),
  Action (moves: play an Action, spend a Villager, end), Buy (`StartBuyPhase`; play
  Treasures until something is bought, buy cards / Events / Projects, spend Coffers, pay
  Debt; `EndBuyPhase`), Night (`StartNight`; play Night cards), Clean-up
  (`StartCleanup`, `DiscardFromPlay`, discard everything not kept by a Lasting, draw
  `turn.cleanup_draw`, `EndTurn`, `Until::EndOfTurn` lastings expire). Extra turns queue
  in `Game::extra_turns`; nobody takes more than two turns in a row, and extra turns
  don't count for the tiebreaker. `Turn::controller` makes someone else decide and see
  the player's cards (Possession, which also replaces gains, trashing through
  `SetRules::trash_instead` and Debt through `SetRules::debt_taker`).
- **Playing a card** (`Frame::play`, the only way): move it into play, `Play` window
  (triggers may set `g.plays.last_mut()`'s `replaced` / `blocked`), choose a Way if the
  game has Ways, open an `Attack` window per other player for Attacks, then apply the
  vanilla bonuses (`CardDef::plus`, Treasure `coins` / `potions`, through
  `SetRules::modify_bonus`) and run the card's `Hook::Main`, then the `AfterPlay`
  window. `chain` lists the cards playing it (Throne Room ...); pass
  `f.chain_for_child()` when a card plays another.
- **Gaining** (`Frame::gain`, `Frame::gain_card`): `WouldGain` window (replacement
  effects edit `g.gains.last_mut()`: `to`, `player`, `cancelled`), move, `Gain` window.
  **Buying** (`Move::Buy`): pay (coins, Potions, Debt taken), overpay (the card's
  `Hook::Ability(frame::OVERPAY)` with `f.data` = amount), `Buy` window, then gain.
  Events and Projects: pay, `BuyLandscape` window, then the landscape's `Hook::Main`
  (a Project also gets the buyer's cube in `Player::projects`).
- **Triggers** (`trigger.rs`). Each set's `SetRules::triggers(g, occ, out)` adds the
  `Trigger`s that apply right now; the window resolves them (turn player first, then
  in turn order, each player ordering their own; optional ones can be declined) and
  re-asks after each resolution, so check every condition (including where the
  card is: the stop-moving rule) when producing a trigger. A trigger runs as the card's
  `Hook::Ability(n)` with `f.occ`, `f.lasting`, `f.data` set. Finishing an `Attack`
  trigger with `Value::Bool(true)` makes the target unaffected.
- **Lasting effects** (`Lasting`): anything that outlives a play. Durations call
  `f.add_duration(g, data)` (keeps the card and its `chain` in play) only when the play
  has something left to do, and produce a `StartTurn` trigger from that lasting
  (`cards::due_durations` lists the ones due: made on an earlier turn of the owner's;
  `cards::duration_trigger` builds the trigger), whose ability removes it
  (`f.end_lasting(g)`). "This turn" effects use `Until::EndOfTurn`, "until your next
  turn" `Until::StartOfTurn(owner)`. Sets read `g.lasting()` for cost changes, triggers,
  immunity and so on.
- **Modifiers.** Rules code asks the game, never the static data: `g.cost_of(id)` /
  `g.cost(card)` (through `SetRules::modify_cost`), `g.types(id)` / `g.is(id, types)`
  (through `modify_types`), `g.card(id)` (what a card currently is: `Game::masks` make
  Band of Misfits-style cards another card until they leave play; `g.printed(id)` is
  what is printed). `SetRules::behavior` lets a card use another card's abilities
  (Inheritance).
- **State.** Generic state is on `Game`, `Player` (mats, Coffers, Villagers, Favors,
  Debt, VP tokens, Projects, States, Adventures tokens, ...), `Pile` (VP / Debt /
  Embargo / Trade Route / player tokens, Traits) and `Turn`. Take Debt with
  `g.add_debt` (never `debt +=`), so Possession can redirect it. A set's own state goes in
  its `State` struct (`g.ext.<set>`). Everything must stay `Clone + Serialize`.
- **Decisions** are generic: `Turn { moves }`, `Cards`, `Pile`, `Choose` (options such as
  `Opt::Yes`, `Opt::Mode(i)`, `Opt::Card`, `Opt::Trigger`), `NameCard`, `Number`. Each
  carries a `Purpose` and its `source` card. Never add a card-specific decision kind.
- **Agents** get a `View`, never the `Game`.

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
- `+$` goes through `g.add_coins(p, n)` (the -$1 token). Decisions are asked with the
  `CardFrame::ask_*` helpers.
- Respect the stop-moving rule: before moving a card an effect "expects" somewhere,
  check `g.zone(id)` is still that zone.
- Setup: `build_kingdom_pile` for unusual Kingdom piles, `setup` for extra piles
  (`setup::add_pile`, `add_mixed_pile`), decks of Boons/Hexes, tokens and so on,
  `modify_starting_deck` for Heirlooms/Shelters-like changes.

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
