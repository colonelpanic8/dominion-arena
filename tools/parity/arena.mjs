// A throwaway local Arena serving both clients: ours at /, dominion.games' 2.3.7 client
// over the root socket. Its state lives in a temporary directory.
import { spawn } from 'node:child_process';
import fs from 'node:fs';
import net from 'node:net';
import os from 'node:os';
import path from 'node:path';

async function freePort() {
  for (;;) {
    const port = 20000 + Math.floor(Math.random() * 40000);
    const ok = await new Promise((resolve) => {
      const s = net.createServer().once('error', () => resolve(false));
      s.listen(port, '127.0.0.1', () => s.close(() => resolve(true)));
    });
    if (ok) return port;
  }
}

export class Arena {
  static async start({ binary, webDir, assetsDir, catalog, seed, log }) {
    const port = await freePort();
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'dominion-parity-'));
    const lines = [
      `listen = "127.0.0.1"`,
      `port = ${port}`,
      `state_dir = ${JSON.stringify(path.join(dir, 'games'))}`,
      `assets_dir = ${JSON.stringify(assetsDir)}`,
      `web_dir = ${JSON.stringify(webDir)}`,
      `registration = true`,
      `guests = false`,
      `web_animations = false`,
      `web_sounds = false`,
      `official_server_enabled = true`,
      `official_server_catalog_file = ${JSON.stringify(catalog)}`,
      `official_server_root_socket = true`,
      `official_preferences_dir = ${JSON.stringify(path.join(dir, 'prefs'))}`,
    ];
    if (seed !== undefined) lines.push(`table_seed = ${seed}`);
    // Accounts u1, u2, ... (registered in that order) see dominion.games' client as ours
    // is set up by default: the Inheritance painting (1; 0 is none), no animations
    // (Instant, 3), and the kingdom moving down on others' turns.
    fs.mkdirSync(path.join(dir, 'prefs'));
    for (let i = 1; i <= 6; i++) {
      fs.writeFileSync(path.join(dir, 'prefs', `u${i}.toml`), 'preferred_background = 1\nanimation_type = 3\nmove_kingdom = true\n');
    }
    const config = path.join(dir, 'config.toml');
    fs.writeFileSync(config, lines.join('\n') + '\n');
    const env = { ...process.env, XDG_STATE_HOME: dir, XDG_CONFIG_HOME: dir, XDG_CACHE_HOME: dir };
    const child = spawn(binary, ['--config', config, 'serve'], { env, stdio: ['ignore', 'pipe', 'pipe'] });
    const out = fs.createWriteStream(log);
    child.stdout.pipe(out);
    child.stderr.pipe(out);
    const arena = new Arena(port, dir, child);
    for (let i = 0; i < 300; i++) {
      if (child.exitCode !== null) throw new Error(`dominion serve exited; see ${log}`);
      try {
        const r = await fetch(`${arena.url}/api/settings`);
        if (r.ok) return arena;
      } catch {}
      await new Promise((r) => setTimeout(r, 100));
    }
    throw new Error(`dominion serve did not start; see ${log}`);
  }

  constructor(port, dir, child) {
    this.port = port;
    this.dir = dir;
    this.child = child;
    // Ours and theirs on different origins, so cookies and storage stay apart.
    this.url = `http://127.0.0.1:${port}`;
    this.officialUrl = `http://localhost:${port}`;
  }

  async stop() {
    this.child.kill('SIGTERM');
    await new Promise((r) => (this.child.exitCode !== null ? r() : this.child.once('exit', r)));
    fs.rmSync(this.dir, { recursive: true, force: true });
  }

  async api(method, route, body, cookie) {
    const headers = { 'content-type': 'application/json' };
    if (cookie) headers.cookie = cookie;
    const r = await fetch(`${this.url}/api${route}`, {
      method,
      headers,
      body: body === undefined ? undefined : JSON.stringify(body),
    });
    const text = await r.text();
    if (!r.ok) throw new Error(`${method} ${route}: ${r.status} ${text}`);
    const setCookie = r.headers.get('set-cookie');
    return { json: text ? JSON.parse(text) : null, cookie: setCookie && setCookie.split(';')[0] };
  }

  async register(name, password) {
    const { cookie } = await this.api('POST', '/auth/register', { name, password });
    return cookie;
  }

  /// Seats `host` and `guest` at a new table with `spec` and starts its game.
  async startGame(host, guest, spec) {
    const { json: table } = await this.api(
      'POST',
      '/tables',
      { spec: { undo: 'off', rated: false, timed: false, randomizeOrder: false, ...spec } },
      host,
    );
    await this.api('POST', `/tables/${table.id}/join`, {}, guest);
    await this.api('POST', `/tables/${table.id}/ready`, { ready: true }, guest);
    const { json } = await this.api('POST', `/tables/${table.id}/ready`, { ready: true }, host);
    const game = json.game ?? (await this.api('GET', `/tables/${table.id}`)).json.game;
    if (!game) throw new Error(`table ${table.id} started no game: ${JSON.stringify(json)}`);
    return game;
  }

  async view(game, cookie) {
    return (await this.api('GET', `/games/${encodeURIComponent(game)}/view`, undefined, cookie)).json;
  }

  async answer(game, cookie, answer) {
    return (await this.api('POST', `/games/${encodeURIComponent(game)}/answer`, answer, cookie)).json;
  }
}
