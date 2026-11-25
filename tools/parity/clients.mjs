// The two clients under comparison, each in its own browser context.
import crypto from 'node:crypto';
import fs from 'node:fs';
import path from 'node:path';

export const BODY_SHA256 = '53f5163219f75d782e971d9dd96337a016c1f4d2c23a51476f50700ec6a816dc';
const BODY = '/js/dominion-webclient-body-2.3.7.min.js';
const ORIGIN = 'https://dominion.games';

/// dominion.games' static files (page, templates, styles, images), fetched once into
/// `cache`. The body bundle is the pinned one from `catalog`, never fetched.
export class StaticCache {
  constructor(dir, catalog, { offline = false } = {}) {
    this.dir = dir;
    this.offline = offline;
    const body = fs.readFileSync(catalog);
    const hash = crypto.createHash('sha256').update(body).digest('hex');
    if (hash !== BODY_SHA256) throw new Error(`${catalog}: sha256 ${hash}, expected ${BODY_SHA256}`);
    this.body = body;
    this.missing = new Set();
  }

  async get(pathname) {
    if (pathname === BODY) return this.body;
    const rel = pathname === '/' ? 'index.html' : pathname.replace(/^\/+/, '');
    if (rel.split('/').includes('..')) return null;
    const file = path.join(this.dir, rel);
    if (fs.existsSync(file)) return fs.readFileSync(file);
    if (this.offline) {
      this.missing.add(rel);
      return null;
    }
    const r = await fetch(`${ORIGIN}/${rel}`, { headers: { 'user-agent': 'Mozilla/5.0' } });
    if (!r.ok) {
      this.missing.add(rel);
      return null;
    }
    const bytes = Buffer.from(await r.arrayBuffer());
    fs.mkdirSync(path.dirname(file), { recursive: true });
    fs.writeFileSync(file, bytes);
    return bytes;
  }
}

const TYPES = {
  '.html': 'text/html', '.js': 'application/javascript', '.css': 'text/css',
  '.png': 'image/png', '.jpg': 'image/jpeg', '.webp': 'image/webp', '.svg': 'image/svg+xml',
  '.ttf': 'font/ttf', '.woff': 'font/woff', '.woff2': 'font/woff2', '.otf': 'font/otf',
  '.mp3': 'audio/mpeg', '.json': 'application/json', '.ico': 'image/x-icon',
};

/// dominion.games' client served at `arena.officialUrl`, its socket pointed at Arena.
export async function openTheirs(browser, arena, cache, viewport) {
  const context = await browser.newContext({ viewport });
  const page = await context.newPage();
  const origin = arena.officialUrl;
  // The one change to their client: the server it picks (connection-selection.js) is
  // Arena. No other socket may open.
  const selection = `USED_ALPHA_CONNECTION = new Connection({ port: ${arena.port}, host: 'localhost', ssl: false });
USED_BETA_CONNECTION = USED_ALPHA_CONNECTION;
`;
  const local = `ws://localhost:${arena.port}/`;
  await context.routeWebSocket(
    (url) => url.href !== local,
    (ws) => ws.close({ code: 1008, reason: 'parity: only the local Arena' }),
  );
  await context.route('**/*', async (route) => {
    const url = new URL(route.request().url());
    if (url.origin !== origin) return route.abort();
    if (url.pathname === '/connection-selection.js') {
      return route.fulfill({ body: selection, contentType: 'application/javascript' });
    }
    const bytes = await cache.get(url.pathname);
    if (!bytes) return route.fulfill({ status: 404 });
    const type = TYPES[path.extname(url.pathname)] ?? (url.pathname === '/' ? 'text/html' : undefined);
    return route.fulfill({ body: bytes, contentType: type });
  });
  return { context, page };
}

/// Logs dominion.games' client in; Arena resumes it into the account's running game.
export async function enterTheirs(page, arena, name, password) {
  await page.goto(`${arena.officialUrl}/`);
  await page.waitForSelector('input[name=username]', { timeout: 30000 });
  await page.fill('input[name=username]', name);
  await page.fill('input[name=password]', password);
  await page.click('.login-button');
}

/// Our client at `/game/<id>`, signed in as the account.
export async function openOurs(browser, arena, cookie, viewport) {
  const context = await browser.newContext({ viewport });
  const [key, value] = cookie.split('=');
  await context.addCookies([{ name: key, value, url: arena.url }]);
  const page = await context.newPage();
  return { context, page };
}
