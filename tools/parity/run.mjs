#!/usr/bin/env node
// Scores how alike our table and dominion.games' own 2.3.7 client draw the same games.
// See README.md.
import fs from 'node:fs';
import { createRequire } from 'node:module';
import path from 'node:path';
import { parseArgs } from 'node:util';
import { Arena } from './arena.mjs';
import { StaticCache, openTheirs, enterTheirs, openOurs } from './clients.mjs';
import { Driver } from './drive.mjs';
import { extract, settle } from './extract.mjs';
import { SCENARIOS } from './scenarios.mjs';
import { WEIGHTS, compareCaptures, heatmap, ssim, total } from './score.mjs';
import { writeReport } from './report.mjs';

const require = createRequire(import.meta.url);
const { chromium } = require('playwright-core');
const root = path.resolve(path.dirname(new URL(import.meta.url).pathname), '../..');
const cacheHome = process.env.XDG_CACHE_HOME ?? path.join(process.env.HOME, '.cache');

const { values: opts } = parseArgs({
  options: {
    out: { type: 'string', default: path.join(root, 'target/parity/latest') },
    only: { type: 'string' },
    viewport: { type: 'string', default: '1600x900' },
    seed: { type: 'string', default: '1' },
    catalog: { type: 'string', default: process.env.DOMINION_OFFICIAL_BODY ?? '/var/cache/dominion/online/body-2.3.7.js' },
    assets: { type: 'string', default: process.env.DOMINION_ASSETS ?? '/var/cache/dominion/assets' },
    cache: { type: 'string', default: path.join(cacheHome, 'dominion/parity/dominion.games') },
    binary: { type: 'string', default: path.join(root, 'target/debug/dominion') },
    web: { type: 'string', default: path.join(root, 'target/web-client') },
    offline: { type: 'boolean', default: false },
  },
});

const [vw, vh] = opts.viewport.split('x').map(Number);
const viewport = { width: vw, height: vh };
const scenarios = opts.only
  ? SCENARIOS.filter((s) => opts.only.split(',').includes(s.name))
  : SCENARIOS;
if (!scenarios.length) throw new Error(`no scenario named ${opts.only}`);
fs.rmSync(opts.out, { recursive: true, force: true });
fs.mkdirSync(opts.out, { recursive: true });

const cache = new StaticCache(opts.cache, opts.catalog, { offline: opts.offline });
const browser = await chromium.launch({
  executablePath: process.env.CHROME || undefined,
  // Their page is served by Playwright, so Chrome takes it for a public one reaching
  // into localhost.
  args: ['--disable-features=LocalNetworkAccessChecks,BlockInsecurePrivateNetworkRequests', '--font-render-hinting=none'],
});

const PASSWORD = 'parity-password';

// The log and chat: their client is told no history when it joins a game in progress,
// so only the table is compared.
const IGNORE = { theirs: ['.game-tabs'], ours: ['.dt-log-wrap', '.dt-below-log'] };

async function capture(page, dir, side) {
  // Over the log, where hovering opens nothing on the table.
  await page.mouse.move(viewport.width - 60, Math.round(viewport.height * 0.3));
  await page.evaluate(settle, { quiet: 1500, limit: 20000 });
  const png = await page.screenshot({ path: path.join(dir, `${side}.png`) });
  const shown = await page.evaluate(extract, { ignore: IGNORE[side] });
  fs.writeFileSync(path.join(dir, `${side}.json`), JSON.stringify(shown));
  return { png, shown };
}

async function runScenario(scenario) {
  const dir = path.join(opts.out, scenario.name);
  fs.mkdirSync(dir, { recursive: true });
  const arena = await Arena.start({
    binary: opts.binary,
    webDir: opts.web,
    assetsDir: opts.assets,
    catalog: opts.catalog,
    seed: Number(opts.seed),
    log: path.join(dir, 'server.log'),
  });
  const contexts = [];
  try {
    const alice = await arena.register('Alice', PASSWORD);
    const bob = await arena.register('Bob', PASSWORD);
    const game = await arena.startGame(alice, bob, scenario.spec);
    const driver = new Driver(arena, game, { Alice: alice, Bob: bob });
    await driver.init();
    await driver.run(scenario.steps);

    const theirs = await openTheirs(browser, arena, cache, viewport);
    contexts.push(theirs.context);
    const log = fs.createWriteStream(path.join(dir, 'theirs-console.log'));
    let loaded = false;
    theirs.page.on('console', (m) => {
      if (m.text().includes('Game has loaded')) loaded = true;
      log.write(m.text() + '\n');
    });
    theirs.page.on('pageerror', (e) => log.write(`ERROR ${e.message}\n`));
    await enterTheirs(theirs.page, arena, 'Alice', PASSWORD);
    for (let i = 0; i < 300 && !loaded; i++) await new Promise((r) => setTimeout(r, 100));
    if (!loaded) throw new Error(`their client never loaded the game; see ${dir}/theirs-console.log`);

    const ours = await openOurs(browser, arena, alice, viewport);
    contexts.push(ours.context);
    await ours.page.goto(`${arena.url}/game/${encodeURIComponent(game)}`);
    await ours.page.waitForSelector('.dt-root', { timeout: 30000 });
    // Our client opens a game on the starting cards; dominion.games goes straight in.
    const start = await ours.page.$('.dt-start-game');
    if (start) await start.click();

    const a = await capture(theirs.page, dir, 'theirs');
    const b = await capture(ours.page, dir, 'ours');
    const anchors = compareCaptures(a.shown, b.shown);
    const pixels = ssim(a.png, b.png, [...a.shown.ignored, ...b.shown.ignored]);
    fs.writeFileSync(path.join(dir, 'diff.png'), heatmap(pixels));
    const parts = { images: anchors.images.score, texts: anchors.texts.score, pixels: pixels.score };
    const result = { name: scenario.name, about: scenario.about, parts, score: total(parts), anchors };
    fs.writeFileSync(path.join(dir, 'result.json'), JSON.stringify(result, null, 1));
    return result;
  } catch (e) {
    return { name: scenario.name, about: scenario.about, error: String(e.stack ?? e), score: 0 };
  } finally {
    for (const c of contexts) await c.close();
    await arena.stop();
  }
}

const results = [];
for (const scenario of scenarios) {
  const r = await runScenario(scenario);
  results.push(r);
  const parts = r.parts
    ? Object.entries(r.parts).map(([k, v]) => `${k} ${(100 * v).toFixed(1)}`).join('  ')
    : `ERROR ${r.error.split('\n')[0]}`;
  console.log(`${r.name.padEnd(20)} ${(100 * r.score).toFixed(1).padStart(5)}   ${parts}`);
}
await browser.close();

const overall = results.reduce((s, r) => s + r.score, 0) / results.length;
const summary = {
  overall,
  weights: WEIGHTS,
  viewport,
  seed: Number(opts.seed),
  scenarios: results.map(({ name, score, parts, error }) => ({ name, score, parts, error })),
  missingStatic: [...cache.missing],
};
fs.writeFileSync(path.join(opts.out, 'summary.json'), JSON.stringify(summary, null, 1));
writeReport(opts.out, summary, results);
console.log(`${'overall'.padEnd(20)} ${(100 * overall).toFixed(1).padStart(5)}`);
console.log(`report: ${path.join(opts.out, 'index.html')}`);
process.exitCode = results.some((r) => r.error) ? 1 : 0;
