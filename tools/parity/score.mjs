// How alike two captures are, each part from 0 (nothing alike) to 1 (the same).
import zlib from 'node:zlib';

export const WEIGHTS = { images: 0.4, texts: 0.35, pixels: 0.25 };

/// Pairs anchors with the same key, nearest first.
function pair(theirs, ours) {
  const byKey = new Map();
  for (const [side, list] of [['a', theirs], ['b', ours]]) {
    list.forEach((item, i) => {
      if (!byKey.has(item.key)) byKey.set(item.key, { a: [], b: [] });
      byKey.get(item.key)[side].push(i);
    });
  }
  const pairs = [];
  const center = (r) => [r.x + r.w / 2, r.y + r.h / 2];
  for (const { a, b } of byKey.values()) {
    const candidates = [];
    for (const i of a) {
      for (const j of b) {
        const [ax, ay] = center(theirs[i]);
        const [bx, by] = center(ours[j]);
        candidates.push([Math.hypot(ax - bx, ay - by), i, j]);
      }
    }
    candidates.sort((x, y) => x[0] - y[0]);
    const usedA = new Set();
    const usedB = new Set();
    for (const [, i, j] of candidates) {
      if (usedA.has(i) || usedB.has(j)) continue;
      usedA.add(i);
      usedB.add(j);
      pairs.push([i, j]);
    }
  }
  return pairs;
}

function iou(a, b) {
  const w = Math.min(a.x + a.w, b.x + b.w) - Math.max(a.x, b.x);
  const h = Math.min(a.y + a.h, b.y + b.h) - Math.max(a.y, b.y);
  if (w <= 0 || h <= 0) return 0;
  const i = w * h;
  return i / (a.w * a.h + b.w * b.h - i);
}

function textScore(a, b, diag) {
  const d = Math.hypot(a.x + a.w / 2 - (b.x + b.w / 2), a.y + a.h / 2 - (b.y + b.h / 2));
  const position = Math.exp(-d / diag / 0.015);
  const size = Math.min(a.h, b.h) / Math.max(a.h, b.h);
  return 0.75 * position + 0.25 * size;
}

/// Anchors present on both sides, scored where they are; the rest count as misses.
function anchors(theirs, ours, scoreOf) {
  const pairs = pair(theirs, ours);
  const matchedA = new Set(pairs.map(([i]) => i));
  const matchedB = new Set(pairs.map(([, j]) => j));
  const scored = pairs.map(([i, j]) => ({ theirs: theirs[i], ours: ours[j], score: scoreOf(theirs[i], ours[j]) }));
  const total = theirs.length + ours.length;
  const sum = scored.reduce((s, p) => s + p.score, 0);
  return {
    score: total ? (2 * sum) / total : 1,
    counts: { theirs: theirs.length, ours: ours.length, paired: pairs.length },
    missing: theirs.filter((_, i) => !matchedA.has(i)),
    extra: ours.filter((_, j) => !matchedB.has(j)),
    worst: scored.filter((p) => p.score < 0.9).sort((x, y) => x.score - y.score),
  };
}

export function compareCaptures(theirs, ours) {
  const diag = Math.hypot(theirs.width, theirs.height);
  return {
    images: anchors(theirs.images, ours.images, iou),
    texts: anchors(theirs.texts, ours.texts, (a, b) => textScore(a, b, diag)),
  };
}

// --- pixels -------------------------------------------------------------------------

/// An 8-bit RGB(A) PNG, as Chrome writes screenshots, decoded to grey levels.
export function decodeGrey(buf) {
  let pos = 8;
  let width = 0;
  let height = 0;
  let colorType = 0;
  const idat = [];
  while (pos < buf.length) {
    const len = buf.readUInt32BE(pos);
    const type = buf.toString('ascii', pos + 4, pos + 8);
    const data = buf.subarray(pos + 8, pos + 8 + len);
    if (type === 'IHDR') {
      width = data.readUInt32BE(0);
      height = data.readUInt32BE(4);
      if (data[8] !== 8 || data[12] !== 0) throw new Error('unsupported PNG');
      colorType = data[9];
    } else if (type === 'IDAT') idat.push(data);
    pos += 12 + len;
  }
  const bpp = { 2: 3, 6: 4, 0: 1, 4: 2 }[colorType];
  if (!bpp) throw new Error(`unsupported PNG colour type ${colorType}`);
  const raw = zlib.inflateSync(Buffer.concat(idat));
  const stride = width * bpp;
  const cur = Buffer.alloc(stride);
  const prev = Buffer.alloc(stride);
  const grey = new Float32Array(width * height);
  for (let y = 0; y < height; y++) {
    const row = y * (stride + 1);
    const filter = raw[row];
    for (let x = 0; x < stride; x++) {
      const v = raw[row + 1 + x];
      const a = x >= bpp ? cur[x - bpp] : 0;
      const b = prev[x];
      const c = x >= bpp ? prev[x - bpp] : 0;
      let out;
      switch (filter) {
        case 0: out = v; break;
        case 1: out = v + a; break;
        case 2: out = v + b; break;
        case 3: out = v + ((a + b) >> 1); break;
        case 4: {
          const p = a + b - c;
          const pa = Math.abs(p - a), pb = Math.abs(p - b), pc = Math.abs(p - c);
          out = v + (pa <= pb && pa <= pc ? a : pb <= pc ? b : c);
          break;
        }
        default: throw new Error(`bad PNG filter ${filter}`);
      }
      cur[x] = out & 255;
    }
    for (let x = 0; x < width; x++) {
      const i = x * bpp;
      grey[y * width + x] = bpp >= 3 ? 0.299 * cur[i] + 0.587 * cur[i + 1] + 0.114 * cur[i + 2] : cur[i];
    }
    cur.copy(prev);
  }
  return { width, height, grey };
}

function shrink(img, f) {
  const w = Math.floor(img.width / f);
  const h = Math.floor(img.height / f);
  const out = new Float32Array(w * h);
  for (let y = 0; y < h; y++) {
    for (let x = 0; x < w; x++) {
      let s = 0;
      for (let dy = 0; dy < f; dy++) {
        for (let dx = 0; dx < f; dx++) s += img.grey[(y * f + dy) * img.width + x * f + dx];
      }
      out[y * w + x] = s / (f * f);
    }
  }
  return { width: w, height: h, grey: out };
}

/// Mean SSIM over 8x8 windows of the screenshots shrunk 4 times, and each window's;
/// windows touching an `ignored` box (viewport pixels) are left out.
export function ssim(pngA, pngB, ignored = []) {
  const a = shrink(decodeGrey(pngA), 4);
  const b = shrink(decodeGrey(pngB), 4);
  if (a.width !== b.width || a.height !== b.height) throw new Error('screenshots differ in size');
  const N = 8;
  const C1 = (0.01 * 255) ** 2;
  const C2 = (0.03 * 255) ** 2;
  const cols = Math.floor(a.width / N);
  const rows = Math.floor(a.height / N);
  const map = new Float32Array(cols * rows);
  const S = 4 * N;
  const skipped = (wx, wy) =>
    ignored.some((r) => wx * S < r.x + r.w && (wx + 1) * S > r.x && wy * S < r.y + r.h && (wy + 1) * S > r.y);
  let total = 0;
  let counted = 0;
  for (let wy = 0; wy < rows; wy++) {
    for (let wx = 0; wx < cols; wx++) {
      if (skipped(wx, wy)) {
        map[wy * cols + wx] = 1;
        continue;
      }
      let ma = 0, mb = 0;
      for (let y = 0; y < N; y++) {
        for (let x = 0; x < N; x++) {
          const i = (wy * N + y) * a.width + wx * N + x;
          ma += a.grey[i];
          mb += b.grey[i];
        }
      }
      ma /= N * N;
      mb /= N * N;
      let va = 0, vb = 0, cov = 0;
      for (let y = 0; y < N; y++) {
        for (let x = 0; x < N; x++) {
          const i = (wy * N + y) * a.width + wx * N + x;
          const da = a.grey[i] - ma;
          const db = b.grey[i] - mb;
          va += da * da;
          vb += db * db;
          cov += da * db;
        }
      }
      const n = N * N - 1;
      va /= n; vb /= n; cov /= n;
      const s = ((2 * ma * mb + C1) * (2 * cov + C2)) / ((ma * ma + mb * mb + C1) * (va + vb + C2));
      map[wy * cols + wx] = s;
      total += s;
      counted++;
    }
  }
  return { score: Math.max(0, counted ? total / counted : 1), map, cols, rows };
}

/// A PNG of where the screenshots differ: black alike, red unlike.
export function heatmap({ map, cols, rows }, cell = 8) {
  const w = cols * cell;
  const h = rows * cell;
  const raw = Buffer.alloc((w * 3 + 1) * h);
  for (let y = 0; y < h; y++) {
    raw[y * (w * 3 + 1)] = 0;
    for (let x = 0; x < w; x++) {
      const s = map[Math.floor(y / cell) * cols + Math.floor(x / cell)];
      const v = Math.round(255 * Math.min(1, Math.max(0, 1 - s)));
      const i = y * (w * 3 + 1) + 1 + x * 3;
      raw[i] = v;
      raw[i + 1] = Math.round(v * 0.2);
      raw[i + 2] = Math.round(v * 0.2);
    }
  }
  const chunk = (type, data) => {
    const len = Buffer.alloc(4);
    len.writeUInt32BE(data.length);
    const body = Buffer.concat([Buffer.from(type, 'ascii'), data]);
    const crc = Buffer.alloc(4);
    crc.writeUInt32BE(zlib.crc32(body) >>> 0);
    return Buffer.concat([len, body, crc]);
  };
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(w, 0);
  ihdr.writeUInt32BE(h, 4);
  ihdr[8] = 8;
  ihdr[9] = 2;
  return Buffer.concat([
    Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]),
    chunk('IHDR', ihdr),
    chunk('IDAT', zlib.deflateSync(raw)),
    chunk('IEND', Buffer.alloc(0)),
  ]);
}

export function total(parts) {
  return (
    WEIGHTS.images * parts.images + WEIGHTS.texts * parts.texts + WEIGHTS.pixels * parts.pixels
  );
}
