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
