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
   others' turns.
2. Registers Alice and Bob, seats them at a table with the scenario's rules, and plays
   the scenario's steps over the HTTP API (`drive.mjs`).
3. Opens dominion.games' client as Alice (`clients.mjs`). Its page, templates, styles and
   images come from a cache filled from dominion.games on first use
   (`~/.cache/dominion/parity/dominion.games`; `--offline` refuses to fetch). The body
   bundle is never fetched: it is `--catalog`, checked against the pinned SHA-256. The
   one change to their client is `connection-selection.js`, which points it at the local
   Arena. Every other WebSocket is refused, so it never reaches dominion.games' servers.
4. Opens our client as Alice at `/game/<id>`.
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

- Only cards the dominion.games transport supports can be in a scenario
  (`GET /api/official/coverage`): Base, Intrigue, some of Seaside, Platinum and Colony,
  Potion and Alchemist, Settlers, Baker, Recruiter, Engineer, Expedition, Cathedral.
  More scenarios become possible as it grows.
- One seat's view (Alice's) at one viewport per run.
- Animations, hovering and windows (the study view, mats opened) aren't compared yet.
