# dominion-engine

A headless Rust rules engine for Dominion: every card from every expansion but
Possession, played exactly by the current rules (second editions / Dominion Online, plus
the cards removed in second editions). It ships no bots and no UI; anything that plays
implements `Agent`.

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
  src/scribe.rs           logs what changed without an event: moves, counters, tokens
  src/cards/mod.rs        SetRules trait, dispatch, CardFrame and its helpers
  src/cards/<set>.rs      one module per set: its State, Rules and card scripts
  src/tests/mod.rs        engine tests, shared helpers, the random-game fuzz test
  src/tests/fuzz.rs       fuzz harness: kingdom families, oracles at every decision
                          (more games: FUZZ_SEEDS; switches in its module docs)
  src/tests/fuzz/unused.rs  games played twice in step, once with unused landscapes added
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
  src/game/mod.rs         GamePage: seat choice, keeping state current, `GameCtx` context,
                          the notice when the game waits on a seat nobody has joined
  src/game/plain.rs       the plain text-and-buttons page and `DecisionControls`
  src/game/interaction.rs where each option is answered on the table (`Target`, `Place`),
                          "Play Treasures" (`TreasureRun`)
  src/game/board.rs       a view arranged for drawing: piles, players, stacks, status
  src/game/table/         the graphical table, dominion.games' layout and behaviour; the game
                          page's default view (Plain is a toggle)
    mod.rs                `TableGame`, `TableCtx` (selection, study view, windows, keys,
                          the Autoplay Treasures run), `Mark`: how a card takes part in a
                          decision and the colour of its border
    model.rs              each target's `Spot` on the table; play area this turn / earlier (pure)
    motion.rs             cards seen moving: DOM snapshot before a view change, ghosts after
                          it (`moves`) with dominion.games' per-destination scripts and
                          Animation Type timings
    sound.rs              your-turn and end-of-game sounds when dominion.games plays them
    layout.rs             every zone's box from the window size, in dominion.games' vh/vw
                          rules: basic columns, kingdom grid, rows, windows (pure)
    pieces.rs             `TableCard` (action-coloured border, crosses, counts, plus, All),
                          badges, `CardName`, the study view
    supply.rs players.rs prompt.rs side.rs   Supply and tabs; player bars, peeking opponent
                          piles, play row, your hand and piles; status bar, buttons, decision
                          window; log, "more" panel, game end and score screen
  src/game/wording.rs     dominion.games' prompts and button names for a decision (pure)
  assets/table.css        table styles: the frame from --u, the middle row and hand from layout.rs
  src/game/entry.rs       log entries parsed back from their text in every form the agent
                          log writes them (pure); what logfmt and moves read
  src/game/logfmt.rs      the log in dominion.games' wording: initials, card lists with
                          articles and plurals, merged runs, turn headers, depth
  src/game/moves.rs       which cards moved where between two boards, from the log and the
                          boards (pure); ghosts summarising them
  tests/table.rs          those models and the table's spots property-tested over random games;
                          every log entry of random games read by a rule in entry.rs
  tests/moves.rs          move inference over random games, and hand-checked steps
  src/lobby/              the lobby, dominion.games' layout and behaviour
    mod.rs                the frame and tabs, `TableState` (the table this browser sits at),
                          toast, tooltips, fieldsets, rows, switches
    rules.rs              a table's rules (Kingdom Options, seats), kingdom text, the
                          selector's search, `DealRequest` and `NewGame` from them (pure)
    store.rs              the table, your name and the last kingdom in localStorage
    my_table.rs           My Table: Kingdom Options, Game Options, Players, seat links,
                          Ready (`start`) and Leave Table; shared with the score page.
                          `use_table_game` follows the table's game on the server: while
                          it runs the table shows that game, locked
    seat_link.rs          a seat handed out: its agent line, link, token, copy buttons
    selector.rs           "Select Cards", the card selector
    home.rs tables.rs cardlists.rs   Automatch (Bot Game, Create Table, Join a Seat),
                          Tables, Cardlists (the Banned list in the config file)
  assets/lobby.css        lobby styles, in dominion.games' vh/vw
  tests/lobby.rs          table rules against the engine's cards
tools/import_cards.py     one-off importer that produced data/cards (kept for reference)
```

## Card data

`data/cards/<set>.json` holds every card-shaped object: Kingdom and basic cards,
non-Supply cards, landscapes, Boons, Hexes, States, Artifacts and pile randomizers. Each
entry has `name`, `set`, `types`, official `text` (lines separated by `\n`, `---` for
the dividing line), and as applicable `cost` (`coins`, `potions`, `debt`, `variable`,
`overpay`), `plus` (leading vanilla bonuses: `cards`, `actions`, `buys`, `coins`,
`villagers`, `coffers`, `favors`), `coins` (a Treasure's value), `potions`, `vp`,
`kingdom`, `unimplemented`, `basic`, `removed`, and `pile` (the randomizer or top card
of its mixed or split pile). A Kingdom randomizer's `cost` is the one printed on it: what
its pile costs for a setup choice (a Bane, Ferryman's pile). A randomizer is no card
(`Kind::Randomizer`), so nothing gains, buys or names one, and its cards cost their own. `build.rs` turns it into `Card` (one variant per entry, named by
CamelCasing the name), `Types` and the `CardDef` table. Arcana is unreleased: its entries are names and types only, none are Kingdom cards yet, and it has no behavior; re-import it once its cards are public.
Possession is deliberately unimplemented: its entry is marked `unimplemented`, which
makes it no Kingdom card to the engine, and setup rejects it (`SetupError::Unimplemented`).
Don't add back what it needed (another player deciding or seeing a player's cards,
redirected gains, trashing or Debt) for any other card. Edit the JSON to fix data; the
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
- **Turns.** Start (`Until::StartOfTurn` lastings made before this turn expire, then the
  `StartTurn` window),
  Action (moves: play an Action, spend a Villager, end), Buy (`StartBuyPhase`; play
  Treasures until something is bought, buy cards / Events / Projects; `EndBuyPhase`),
  Night (`StartNight`; play Night cards), Clean-up
  (`StartCleanup`, `DiscardFromPlay` for each player with cards in play to discard,
  discard everything not kept by a Lasting, draw
  `turn.cleanup_draw`, `EndTurn`, `Until::EndOfTurn` lastings expire). Spending Coffers
  and paying off Debt are moves in the Action, Buy and Night phases alike (outside the Buy
  phase, offered only where they could matter: `push_token_moves`); Black Market asks for
  both between revealing its cards and selling one. Extra turns queue
  in `Game::extra_turns`; each one's `ExtraLimit` says when it fails (not a third turn
  in a row, decided between turns: queue it anyway, and a card kept in play for it stays
  until the Clean-up after it fails; Seize the Day: never), and
  extra turns don't count for the tiebreaker. Between turns the player who just took a turn takes
  their extra turns first, then the others in turn order; one with different extra turns
  pending chooses which to take (`Purpose::Either`, `Opt::Card(source)`), and an extra
  turn that can't follow the last turn is lost. A card kept in play for an extra turn
  (Outpost, Voyage) is kept by an `Until::ExtraTurn` lasting named in its
  `ExtraTurn::lasting`, which ends when that turn starts, is skipped or is lost. Once
  the game has ended, extra turns happen only before a turn of Fleet's round. `Player::skip_turns` skips the next turns
  a player would take (Lich), Fleet's included; skipped regular turns count for the
  tiebreaker.
- **Playing a card** (`Frame::play`, the only way): move it into play, `Play` window
  (triggers may set `g.plays.last_mut()`'s `replaced` / `blocked`: they are the card's
  player's, and the one resolved last applies), for an Attack one window holding an
  `Attack` occasion per other player (they answer in turn order from the turn's player,
  and a Reaction one of them uses gives the others another chance; also when the play is
  replaced or blocked: Reactions answer the card being played), choose a Way if the game
  has Ways (`Purpose::Way`; it can be used instead of either), then apply the
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
  effects edit `g.gains.last_mut()`: `to`, `player`, `cancelled`), move (the card it
  began with if it is still where it was, else from a pile the top card if it has the
  same name, or any card of a randomizer pile ("gain a Loot"), else nothing; recording the
  card's cost then in `GainState::cost` and the phase in `GainState::phase`: a gain
  "in your Buy phase" is tested with `g.gain_phase(id)`, because Cavalry and Villa change
  `turn.phase` while their gain's window is open), `Gain` window. An effect that goes on
  to move the card it had `p` gain asks `g.gained(p, id)`: the card where it was gained
  to, which need not be the discard pile, if it has stayed there.
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
  card is: the stop-moving rule) when producing a trigger. A declined ability stays
  declined, except that a Reaction (a Reaction card's own ability: its producer says so
  with `Trigger::reaction`, never inferred from a card's types, since an ability about a
  gained Tunnel is no Reaction) another player uses gives the players who declined theirs
  another chance, until everyone has declined. Things that happen together
  share one window (`Window::open_all`): cards trashed together (`Frame::trash`) or
  discarded together (`Frame::discard`) are all moved first, then their player orders
  the abilities, and a card that has moved meanwhile (a Trail shuffled away by the first
  Trail's draw) has lost its own; the players an Attack is played against share one too.
  An effect that stacks ("this
  turn, when you gain ...") produces one trigger per lasting effect or card in play,
  never one trigger carrying a count: the window tracks what has resolved per trigger,
  and more can come into effect while it is open. It doesn't ask for an order
  when a player's mandatory triggers all have the same effect (lasting effects are
  compared by their data: a set whose lastings carry what only says when they apply, a
  stamp or the players an Attack reaches, takes it out in `SetRules::plain_effect`),
  offers triggers the
  player can't tell apart and whose effect is the same (two Moats, two Tunnels
  discarded together) as one option, drops
  `Trigger::protects` ones (all they do is make the attacked player unaffected) once the
  player is unaffected, and resolves mandatory `protects` ones (Lighthouse) and
  `Trigger::quiet` ones (bookkeeping, or bonuses nothing can interact with, like Fair's
  +1 Buy) first without asking. A trigger runs as the card's
  `Hook::Ability(n)` with `f.occ`, `f.lasting`, `f.data` set. Finishing an `Attack`
  trigger with `Value::Bool(true)` makes the target unaffected (the window finishes with
  `Value::Flags`, one per occasion). Players choose among
  triggers by card name: give one `Trigger::labelled` when the name doesn't say what it
  is (the Exile rule on a gained card, which card a Way's ability is on), and say what an
  optional one does (`Trigger::optional("reveal it to be unaffected")`), and a mandatory
  one another player's lasting effect makes (`Trigger::doing("gain a Curse")`; its label
  names that player). Two options
  with different effects under one name say what tells them apart (where the card is,
  the lasting effect's cards, its count). The window's
  decision has no `source`; its `question` names the moment ("P2 attacks you with
  Militia").
- **Lasting effects** (`Lasting`): anything that outlives a play. Durations call
  `f.add_duration(g, data)` (keeps the card in play with what played it:
  `CardFrame::keeps`) only when the play has something left to do, and produce a `StartTurn` trigger from that lasting
  (`cards::due_durations` lists the ones due: made on an earlier turn of the owner's;
  `cards::turn_durations` lists them all, for "at the start of each of your turns",
  which includes a turn the card is played at the start of;
  `cards::duration_trigger` builds the trigger), whose ability removes it
  (`f.end_lasting(g)`). "This turn" effects use `Until::EndOfTurn`, "until your next
  turn" `Until::StartOfTurn(owner)`; a Duration whose "until then" part comes with an "at
  the start of your next turn" one keeps its lasting and asks `g.before_next_turn(l)`, as
  the effect ends the instant that turn starts. Sets read `g.lasting()` for cost changes, triggers,
  immunity and so on. Players are told when an effect ends: an `Until::Removed` one at
  the start of its owner's next turn (a Duration's), unless its set's
  `SetRules::lasting_ends` says otherwise (the end of the turn, or `Until::Removed` for
  one that waits for something: Champion, Search); the fuzz test holds effects to it. The
  number shown with an effect is only tokens on its card (`SetRules::lasting_tokens`,
  Garrison's), never its data. A lasting that keeps its `source` in play is that Duration's
  effect: when the card leaves play (`Game::move_card` notices), or was not in play when
  the lasting was made, the lasting is `ending`: it goes on working this turn and is
  removed at its end, whatever its `until`, with the extra turn it was for (Outpost);
  the cards it set aside stay set aside. A replay's lasting (Throne Room's second play,
  ...) ends the same way with the card that replayed it, its `tracker`
  (`PlayState::tracker`; none for Citadel and other non-cards). Never re-check
  `g.zone(source)` in card code. A later part of a play's effect (the card Cargo Ship
  sets aside) is added with `g.continue_lasting`; a lasting that starts keeping cards in
  play later does it with `g.keep_in_play`.
- **Modifiers.** Rules code asks the game, never the static data: `g.cost_of(id)` /
  `g.cost(card)` (through `SetRules::modify_cost`; Events and Projects always cost what
  they say), `g.types(id)` / `g.is(id, types)`, and `g.card_types(card)` for a card
  asked by name, a cost modifier's included (through `modify_types`, which sees only the
  card's name: types never depend on the copy or where it is), `g.card(id)` (what a card
  currently is: `Game::masks` make Band of Misfits-style cards another card until they
  leave play; `g.printed(id)` is what is printed). `SetRules::behavior` lets a card use another card's abilities
  (Inheritance). Effects that look back at the turn ("the first Action you play", "the
  third Action you gain") count the types cards had then: `Turn::played_types` /
  `gained_types` (`played_of`, `gained_of`), never `Card::is` on `played` / `gained`.
  Plays are recorded for every player, off-turn ones too: count one player's (`plays_by`).
  A play is in `played` from its start, so cards can be played while it still resolves:
  "the first X you play" and "if this is the first time you played" ask
  `g.plays_before(p, id)`, and an effect made mid-turn that counts what was played before
  it asks `g.finished_plays(p)` (Merchant played during the first Silver's play).
  "During your turns" and "this turn" modifiers ask `g.turn_player()`, which is no one
  between turns and when the game is scored, never `g.turn.player`.
- **State.** Generic state is on `Game`, `Player` (mats, Coffers, Villagers, Favors,
  Debt, VP tokens, Projects, States, Adventures tokens, ...), `Pile` (VP / Debt /
  Embargo / Trade Route / player tokens, Traits) and `Turn`. Take Debt with
  `g.add_debt` (never `debt +=`). A set's own state goes in
  its `State` struct (`g.ext.<set>`). Everything must stay `Clone + Serialize`.
- **Shuffling** goes through `Frame::shuffle` (a discard pile into its deck) or
  `Frame::shuffle_deck` (a whole deck: Inn, Famine), never by hand: `WouldShuffle`,
  shuffle, cards `SetRules::bottom_when_shuffled` says go to the bottom (Shadow cards, in
  their owner's order), then `Shuffled { count }` (`g.shuffled(p)`, the bottom cards of
  the deck, is the shuffled ones still in it; "when you shuffle" placements such as Stash,
  Star Chart and Fated resolve there, under any cards left on the deck). A draw, look or
  reveal that needs more cards than the deck holds shuffles first and then takes them all
  (for a draw the -1 Card token is one of them: the deck needs one card fewer); one draw
  shuffles at most once.
  `Frame::draw_to` ("draw until you have N cards in hand") takes one card at a time
  instead: it shuffles only once the deck has run out and another card is needed, and
  again if that shuffle left cards in the discard pile (Order of Masons, Avoid).
- **Looking and revealing** (`Frame::look` / `Frame::reveal`, the only way to take cards
  from the top of a deck to look at or reveal): the cards go to `Zone::Looking` of the
  deck's owner, off the deck, held (`Game::holds`) for the frame that called it, face up
  if revealed. Draws and looks meanwhile take the cards under them. When that frame
  finishes, those it didn't move go back on top of the deck in their order, over
  anything put there meanwhile. Put them back in a chosen order with `cards::put_back`;
  a card the text leaves "on the deck" is put back at once if the effect goes on to
  anything other cards can answer or that touches the deck (a gain, trash, discard, play,
  draw, look or shuffle). Reveal-until scripts keep what they reveal there too (set aside
  only what the text sets aside).
- **The stop-moving rule.** An effect moves a card it "expects" somewhere (where it was
  gained to, in the trash since it was trashed, in play since it was played) only if the
  card has stayed there, and where the card is now doesn't tell: one that left and came
  back, was shuffled, was looked at and put back, or was covered up on top of its deck
  has moved (one covered up in a discard pile has not). The game stamps a card each time
  that happens (`Game::move_card` and `Frame::shuffle`; putting a card into the hand it
  is in, or onto the deck it is on top of, moves nothing), and `g.stayed(id, since)`
  compares the stamp with a `Mark` (`g.now()`). The frames keep the marks: a gain in its
  `GainState` (`g.still_where_gained(id)` while its window is open; `g.gained(p, id)`
  once it has returned), a trash and a discard while their windows are open
  (`g.still_where_trashed(id)`, `g.still_where_discarded(id)`), a trash, discard or
  play for the effect that called it (`g.put(id)`, asked right after the frame
  returns), a look or reveal in its `Hold` (`g.held(p, id)`), each play of the turn in
  `Turn::played_at` (`g.stayed_in_play()`: Scepter, Way of the Frog). `gained` and `put`
  give a `Placed`, and so does `g.placed(id)` for a card a script put somewhere itself:
  keep it in `f.s.placed` over later steps and ask `g.still(placed)`. Agents never see
  marks.
- **Way of the Chameleon** exchanges what the card's own instructions give its player
  as +Cards and +$ during the turn of that play, and nothing else. Its frame gives the
  vanilla bonuses exchanged and runs the card's `Hook::Main` marked
  `CardFrame::exchange`; the lasting effects a step of an exchanged ability makes are
  listed for the turn (`menagerie::State::chameleon_effects`), and their abilities are
  exchanged until that turn ends, even once the lasting has. While a step of such an
  ability runs, `g.add_coins` for that player counts the $ instead of giving it
  (`Game::coins_as_cards`); after the step those cards are drawn, and the step's flow
  waits for the draw (`CardFrame::hold`). A `Frame::draw` the step returns for a card
  whose text says "+N Cards" beyond its vanilla bonus becomes +$.
- **Decisions** are generic: `Turn { moves }`, `Cards`, `Pile`, `Choose` (options such as
  `Opt::Yes`, `Opt::Mode(i)`, `Opt::Card`, `Opt::Trigger`), `NameCard`, `Number`. Each
  carries a `Purpose` and its `source` card. Never add a card-specific decision kind.
  A question about cards the player looks at but isn't offered as candidates (Navigator's
  "discard them all?") lists them in `Decision::about` (`f.ask_*(..).about(cards)`), so
  the player's view shows them.
  `SetRules::modify_decision` may change what a card ability asks: Elder adds one pick
  to a `Purpose::Mode` "choose" (`ask_modes`) asked by its play of a card, or for the
  rest of that turn by an ability of a lasting effect that play made (`Lasting::play`),
  so scripts, abilities included, must handle every chosen mode, in printed order
  (`mode_mask` / `take_mode`). Alternatives that aren't a "choose"
  ("either ... or", which Artifact) use `ask_either` (`Purpose::Either`).
  Every question says what its answers do, in the card's words: `ask_yes_no(purpose,
  "Trash Engineer to gain a card costing up to $4")` labels Yes (`ask_yes_or` when No
  does something too), and labels name the cards they are about where the player can see
  them. Where the purpose doesn't say what a card, pile or number choice is for, add
  `Flow::question` ("put onto your Island mat", "choose a Supply pile for an Embargo
  token", "how many cards go above it? 0 puts it on top"). Text that counts something,
  here and in the agent crate, says it with `amount` ("1 token", "2 Cards").
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
  group's known identities (a Stash or a Shadow card, with its own back, is always
  known; a group split off when some of its cards left a set held out of sight is some
  of its family's identities). Anything that reveals a specific card must record it: `g.reveal(p, id)`
  instead of emitting `Event::Revealed` by hand, `g.show` / `g.show_to` for other
  reveals and looks, `g.look_through_discard(p)` before a player picks from their
  discard pile, `g.look_through_shuffled(p)` (and `g.look_over_shuffled(p)` for the cards
  left on the deck) where a "when you shuffle" choice looks through the shuffled cards,
  then `g.reshuffle_unplaced(p, placed)` once the others are shuffled again. Cards that change zones must go through `move_card` (or record a
  `Trace::Moved`); identity changes record `Trace::Changed`. Untraced games pay nothing.
  Only its owner follows a card that leaves cards they hold out of sight: a hand,
  cards taken off their deck to look at, cards set aside face down together (move them one after the other), the cards of a
  deck or discard pile they looked through. `g.show` such a card first when everyone
  knows which it is; other cards of a deck or discard pile are followed by everyone.
- **The log** (`scribe.rs`) is complete by itself: before each event and each decision
  the game logs what changed without an event of its own. Cards moved by `move_card`
  that no event reports become `Event::Moved` (naming a card only if everyone knew which
  it was: shown with `g.show`, or leaving or entering a public zone; Clean-up's discards
  stay silent, `g.move_unlogged`); counters and tokens (Coffers, Villagers, Favors,
  Debt, VP tokens, Adventures tokens, -1 Card / -$1 / Journey, pile tokens, Sun tokens,
  States returned) become `Counter` / `TokenMoved` / `Marker` / `Returned`. A triggered
  ability's `Resolved` header (unless the trigger is `labelled`) names its lasting
  effect's owner and is logged before the first thing it does, or as it ends if all it
  did was change what the play in progress does (Enchantress, Highwayman); one that did
  nothing has no line. An "In games using this" ability every player has
  (`Trigger::game_wide`: Shaman, Footpad) is headed `ResolvedFor` instead, as no card
  of the player's. `+Actions` / `+Buys` / `+$` given by a trigger, landscape or
  Boon rather than a card's own play are logged as `Bonus`. So never emit a move or
  counter event by hand; log what has its own event (draws, gains, discards, ...) with
  it. An exchange (Travellers, Changeling, Trader, ...) is `g.exchange`, logged as one
  `Exchanged` line. Public modes and alternatives are announced centrally as `Chosen`;
  decisions mark their labels public with `public_choice`, and labels referring to private
  `about` cards are never announced. Set counters on Sinister Plot and Family of Inventors,
  and the tokens on a lasting effect's card (Garrison), are tallied as `CardCounter`. What is in the game changing during play (Divine Wind) is
  logged too: `PileRemoved` before the tokens leaving that pile; `setup::complete` logs
  what joined (`PileAdded`, `Added` for landscapes, Artifacts and cards put elsewhere,
  `Noted` for new `SetRules::notes`: a Bane, Druid's Boons) before what its setup gives.
- **Agent protocol.** `dominion-agent` projects a game and a seat's `Knowledge` onto
  agent-view@2 and maps option indices back to `Response`s. The schema is owned by
  `dominion-agents/protocol/`; change it there first, then `just sync-protocol`. The
  tests validate every view, decision and answer against the vendored copy.
  `Table::answer_with` (the MCP `answer` tool's `way`) also answers the `Purpose::Way`
  questions of the physical card an answer plays and that card's consecutive replays,
  also after other seats are asked something first (Reactions to an Attack): the table
  keeps the pre-answer, saved with it, until its seat is asked anything else or the play
  is over; a different card's question remains open. The engine's questions and labels
  call players by seat ("P2 attacks you"); `project::named` puts the players' names in,
  for every question, label and announced choice. Decisions whose answer may play an
  Action in a game with Ways end their prompt with `WAY_HINT` (a choice's options are
  tried on a copy of the game: one plays a card if its player starts playing one before
  being asked anything else, which is also the card a pre-answer applies to).

## Writing a set

A set module (`src/cards/<set>.rs`) has `State`, `Rules: SetRules` and its card
scripts. A card script is `fn(&mut Game, &mut CardFrame, Input) -> Flow` matching on
`f.pc`; see `cards/base.rs`. Conventions:

- Number each card's abilities with module consts (`const HAVEN_START: u8 = 0;`); match
  `(f.card, f.hook)` in `run`.
- Use frames for anything cards react to: `Frame::draw / reveal / look / gain /
  gain_card / trash / discard / play / receive / shuffle`. Primitive helpers
  (`g.move_card`, `top_deck`, `put_in_hand`, `set_aside`, `put_on_mat`,
  `return_to_pile`, `add_coins`) trigger nothing. A card such a helper takes from a
  Supply pile (Exiled, set aside) is followed at once by `cards::left_supply`, the
  window that answers it (Search on the pile it emptied, Invest).
- "When you discard ... from play" abilities answer `DiscardFromPlay { player }` for the
  cards in `g.leaving_play(player)`: in Clean-up the cards nothing keeps in play (each
  player's, not only the turn's), at other times the cards given to
  `Frame::discard_from_play` (Highwayman). Effects that only use that window as a
  moment of Clean-up check `g.cleaning_up(player)`.
- A card gained by name ("gain a Rats", "gain another Port") comes from its pile in or
  out of the Supply (Ferryman's): `g.pile_of(card)`. One gained by type or cost ("gain
  an Augur", "a card costing up to $4") comes only from the Supply.
- `+$` goes through `g.add_coins(p, n)` (the -$1 token), +Actions and +Buys through
  `g.add_actions(p, n)` and `g.add_buys(p, n)`: all three do nothing for a player whose
  turn it isn't, so never add to `g.turn` directly. Decisions are asked with the
  `CardFrame::ask_*` helpers.
- Way of the Chameleon makes a script's `g.add_coins` a draw that happens when the step
  returns, before the step's flow. So give +$ where the card's text does (after the
  discard of "discard 2 cards for +$2"), unconditionally (no test of whose turn it is),
  and end the step there (`Flow::Continue`) if the script goes on to list or move cards
  of the player's hand, deck or discard pile. "+N Cards" is a `Frame::draw` the script
  returns as its flow; $ that isn't "+$" (Fortune's doubling) is `g.get_coins`.
- "Draw until you have N cards in hand" is `Frame::draw_to(p, n)`, never a draw of
  `n - hand`: the -1 Card token replaces a draw without making the hand smaller.
- Set cards aside face up with `g.set_aside` (or `g.set_aside_revealed` for a card
  just revealed); `move_card` to `SetAside` leaves it face down, seen only by its owner.
  Public state kept in the set's `State` goes to agents through `SetRules::notes`.
- Respect the stop-moving rule: before moving a card an effect "expects" somewhere, ask
  the game whether it has stayed there, never `g.zone(id)` alone. An on-gain ability
  that moves the gained card (or plays it) checks `g.still_where_gained(id)`; an effect
  that goes on with the card it gained, `g.gained(p, id)`; with a card it trashed,
  discarded or played, `g.put(id)`; a "when you trash / discard this" ability,
  `g.still_where_trashed(id)` / `g.still_where_discarded(id)`; cards from `Frame::look`
  / `Frame::reveal`, which something may move while they are held, `g.held(p, id)`
  (`cards::looking` filters a list). A zone comparison is enough where the text names
  the place the card comes from ("from your hand", "discard this from play", a played
  card that moves itself: it expects to be in play), right after the script's own
  question, and for a card only the script's own lasting effect moves (one it set
  aside).
- `Frame::reveal` / `g.reveal` only where the text says "reveal" (Patron reacts to it);
  "discard / trash the top card" takes it with `Frame::look`.
- Follow the text in order: "trash a card; if it's an Action ..." reads the card's types
  once `Frame::trash` (or the discard, gain, ...) has returned, not when it is chosen.
  A Prophecy coming into effect in between can change them (Sacrifice, Transmute).
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
  `Pile::removed` marks a pile out of the game: name lookups skip it. `Pile::holder` marks
  cards set aside in setup that are no pile by the rules (the Black Market deck,
  Riverboat's and the Mouse's cards): no lookup by name ever finds it. What marks a pile
  keeps applying to its cards (the Bane, Obelisk, Traits): ask `g.origin_pile(card)`. "In games using this" asks `setup::in_game`, not for a pile.
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
`IMPLEMENTED`, which feeds the fuzz tests). Give every card with more than vanilla
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
