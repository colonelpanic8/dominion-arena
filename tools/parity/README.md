# Table parity

How closely our table matches dominion.games, as a number you can push up. dominion.games'
own client (2.3.7, the pinned bundle) and our web client draw the **same Arena game** at
the same moment, and their screens are compared.

```sh
just parity                     # builds, then runs every scenario
just parity --only opening      # one scenario (comma-separated names)
just parity --viewport 1920x1080
```

The report is `target/parity/latest/index.html`. `summary.json` beside it has the scores,
and each scenario's directory holds both screenshots, what was read off each page
(`theirs.json`, `ours.json`), a heatmap of where the pixels differ (`diff.png`), the
scored comparison (`result.json`), the server's log and their client's console.

## How it works

For each scenario (`scenarios.mjs`), `run.mjs`:

1. Starts a throwaway `dominion serve` (`arena.mjs`) with the dominion.games transport on
   (`official_server_*`), lobby games dealt with a fixed `table_seed`, and the two clients
   set up alike: no animations, the Inheritance painting, the kingdom moving down on
   others' turns. `official_server_unreviewed_cards` lets it deal cards whose questions the
   transport doesn't map yet: a scenario using them must stop before any of their
   questions.
2. Registers the scenario's players (Alice, Bob, then Carol, Dave, Erin and Frank for
   `players` up to 6), seats them at a table with its rules in that order, and plays its
   steps over the HTTP API (`drive.mjs`, which lists the steps).
3. Opens dominion.games' client as Alice (`clients.mjs`). Its page, templates, styles and
   images come from a cache filled from dominion.games on first use
   (`~/.cache/dominion/parity/dominion.games`; `--offline` refuses to fetch). The body
   bundle is never fetched: it is `--catalog`, checked against the pinned SHA-256. The
   one change to their client is `connection-selection.js`, which points it at the local
   Arena. Every other WebSocket is refused, so it never reaches dominion.games' servers.
4. Opens our client as Alice at `/game/<id>`, then plays the scenario's `after` steps
   with both clients watching (their client shows a game's end only to a seat that saw
   it happen).
5. Once both pages stop changing, it screenshots each and reads off them every visible
   picture (by file name: both clients use dominion.games' images) and every visible run
   of text, with their boxes (`extract.mjs`).

## The score

Each scenario scores 0 to 100 (`score.mjs`):

- **images** (0.4): pictures paired by file name, nearest first. Each pair scores the
  overlap of its two boxes (intersection over union); a picture with no partner scores
  0. The sum counts both sides: 2 × Σ overlap / (theirs + ours).
- **texts** (0.35): the same for text, paired by its words (case ignored). A pair scores
  for position (falling off over about 1.5% of the screen's diagonal) and height.
- **pixels** (0.25): mean SSIM of the two screenshots in grey, shrunk four times.

The log and chat are left out on both sides: their client is told no history when it
joins a game in progress, so it shows an empty log. Their client paints its game buttons
and the tabs of mats and the trash on a canvas, so our buttons and tabs count only in the
pixels (`IGNORE` in `run.mjs`).

The overall score is the mean over scenarios. Fixed seeds make a run repeatable: the same
code gives the same score.

## Climbing

Read a scenario's lists in the report, worst first:

- *only in dominion.games*: something we don't draw (a type line, a badge, a button).
- *only in ours*: something they don't draw.
- *paired but placed differently*: the same thing in another place or size. Its boxes
  are in viewport pixels, theirs then ours.

Then the onion-skin slider and `diff.png` show where. Change the web client, run
`just parity` again, and keep changes that raise the score without lowering another
scenario's.

Not every difference is ours to fix. Their client is fed Arena's own projection of the
game (`crates/dominion-server/src/official_host`), which can name things differently from
dominion.games' servers (for example Native Village's mat as "Set Aside (Native
Village)"). When their client looks wrong, fix the projection, not our client.

## Limits

- Steps play through Arena, so any card can be played on the way, but a scenario must
  stop at a question the transport maps (`GET /api/official/coverage`); scenarios stop at
  the turn player's Buy phase (`act`) or a move of their turn.
- The transport refuses some state outright, so a scenario can't show it yet: the Black
  Market deck, Inheritance's Estate token, more than one Trait on a pile, and Shelters
  chosen at the table. When their client refuses a game, its console log
  (`theirs-console.log`) says why.
- Their client joins each game already in progress, and some of what it draws then
  differs from a game it watched being played: it lays out a snapshot's cards before
  reading their statuses, so a Duration from last turn stacks with a copy played this
  turn. Our table follows the live game there; don't chase those points.
- One seat's view (Alice's) at one viewport per run.
- Animations, hovering and windows (the study view, mats opened) aren't compared yet.
