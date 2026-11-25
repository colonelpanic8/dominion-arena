// The run as a page: scores, the two screenshots over each other, and what differs.
import fs from 'node:fs';
import path from 'node:path';

const esc = (s) => String(s).replace(/[&<>"]/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' })[c]);
const pct = (v) => (100 * v).toFixed(1);
const rect = (r) => `${Math.round(r.x)},${Math.round(r.y)} ${Math.round(r.w)}×${Math.round(r.h)}`;

function counted(list, label) {
  const n = new Map();
  for (const a of list) n.set(a.key, (n.get(a.key) ?? 0) + 1);
  const rows = [...n].sort((a, b) => b[1] - a[1]);
  if (!rows.length) return '';
  return `<details><summary>${label} (${list.length})</summary><ul>${rows
    .map(([k, c]) => `<li><code>${esc(k)}</code>${c > 1 ? ` ×${c}` : ''}</li>`)
    .join('')}</ul></details>`;
}

function worst(list, label) {
  if (!list.length) return '';
  return `<details><summary>${label} (${list.length})</summary><table><tr><th>anchor</th><th>score</th><th>theirs</th><th>ours</th></tr>${list
    .slice(0, 60)
    .map(
      (p) =>
        `<tr><td><code>${esc(p.theirs.key)}</code></td><td>${pct(p.score)}</td><td>${rect(p.theirs)}</td><td>${rect(p.ours)}</td></tr>`,
    )
    .join('')}</table></details>`;
}

export function writeReport(out, summary, results) {
  const sections = results
    .map((r) => {
      if (r.error) {
        return `<section><h2>${esc(r.name)} <span class="bad">error</span></h2><p>${esc(r.about)}</p><pre>${esc(r.error)}</pre></section>`;
      }
      const { images, texts } = r.anchors;
      return `<section id="${esc(r.name)}">
<h2>${esc(r.name)} <span class="score">${pct(r.score)}</span></h2>
<p>${esc(r.about)}</p>
<p class="parts">images ${pct(r.parts.images)} · texts ${pct(r.parts.texts)} · pixels ${pct(r.parts.pixels)}</p>
<div class="onion" style="--mix:50%">
  <img src="${esc(r.name)}/theirs.png" alt="dominion.games">
  <img class="top" src="${esc(r.name)}/ours.png" alt="ours">
</div>
<label>dominion.games <input type="range" min="0" max="100" value="50" oninput="this.closest('section').querySelector('.onion').style.setProperty('--mix', this.value + '%')"> ours</label>
<div class="pair">
  <figure><img src="${esc(r.name)}/theirs.png"><figcaption>dominion.games</figcaption></figure>
  <figure><img src="${esc(r.name)}/ours.png"><figcaption>ours</figcaption></figure>
  <figure><img src="${esc(r.name)}/diff.png"><figcaption>where pixels differ</figcaption></figure>
</div>
<h3>Pictures: ${images.counts.paired} paired of ${images.counts.theirs} theirs / ${images.counts.ours} ours</h3>
${counted(images.missing, 'only in dominion.games')}
${counted(images.extra, 'only in ours')}
${worst(images.worst, 'paired but placed differently (overlap)')}
<h3>Text: ${texts.counts.paired} paired of ${texts.counts.theirs} theirs / ${texts.counts.ours} ours</h3>
${counted(texts.missing, 'only in dominion.games')}
${counted(texts.extra, 'only in ours')}
${worst(texts.worst, 'paired but placed or sized differently')}
</section>`;
    })
    .join('\n');
  const rows = summary.scenarios
    .map(
      (s) =>
        `<tr><td><a href="#${esc(s.name)}">${esc(s.name)}</a></td><td>${pct(s.score)}</td>${
          s.parts ? ['images', 'texts', 'pixels'].map((k) => `<td>${pct(s.parts[k])}</td>`).join('') : '<td colspan="3" class="bad">error</td>'
        }</tr>`,
    )
    .join('');
  const html = `<!doctype html><html lang="en"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Table parity</title>
<style>
:root { color-scheme: light dark; --bg: #fafaf8; --fg: #1d1d1b; --muted: #6b6b66; --line: #ddd; --bad: #b3261e; }
@media (prefers-color-scheme: dark) { :root { --bg: #171716; --fg: #ecebe6; --muted: #9a9a92; --line: #333; --bad: #f2b8b5; } }
body { background: var(--bg); color: var(--fg); font: 15px/1.5 system-ui, sans-serif; margin: 0 auto; max-width: 1400px; padding: 16px; }
h1 { margin: 0 0 4px; } .muted { color: var(--muted); } .bad { color: var(--bad); }
table { border-collapse: collapse; } td, th { padding: 2px 10px; border-bottom: 1px solid var(--line); text-align: left; font-variant-numeric: tabular-nums; }
section { border-top: 1px solid var(--line); margin-top: 24px; }
.score { font-variant-numeric: tabular-nums; color: var(--muted); }
.onion { position: relative; max-width: 100%; }
.onion img { display: block; width: 100%; }
.onion .top { position: absolute; inset: 0; opacity: var(--mix); }
.pair { display: grid; grid-template-columns: repeat(auto-fit, minmax(280px, 1fr)); gap: 8px; margin-top: 8px; }
.pair img { width: 100%; display: block; } figure { margin: 0; } figcaption { color: var(--muted); font-size: 13px; }
code { font-size: 13px; } details { margin: 4px 0; } pre { white-space: pre-wrap; }
</style></head><body>
<h1>Table parity <span class="score">${pct(summary.overall)}</span></h1>
<p class="muted">Our table against dominion.games' client 2.3.7 on the same Arena games, ${summary.viewport.width}×${summary.viewport.height}, seed ${summary.seed}. Score = ${Object.entries(summary.weights).map(([k, w]) => `${w} × ${k}`).join(' + ')}.</p>
<table><tr><th>scenario</th><th>score</th><th>images</th><th>texts</th><th>pixels</th></tr>${rows}</table>
${sections}
</body></html>`;
  fs.writeFileSync(path.join(out, 'index.html'), html);
}
