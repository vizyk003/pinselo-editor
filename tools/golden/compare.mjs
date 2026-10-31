// SPDX-License-Identifier: Apache-2.0
/**
 * Drift classifier for tools/golden/check.sh (ADR-0001 "Drift check"). Not a generator.
 *
 *   node tools/golden/compare.mjs
 *
 * Run after something rewrote tests/golden/ (a deliberate golden update from a Rust test, or a test that should not
 * have written at all; check.sh runs it after `cargo test`). Every changed file is parsed and compared with its committed
 * (index) version. Node/V8 versions compute some Math results (Math.pow, hence `**`, luminance, easing curves...) to a
 * different last digit — e.g. Node 22 and Node 24 disagree on `10 ** -4` — so a regenerated golden can differ from
 * the committed one only by float noise (the goldens were written by the deleted TS implementation under several Node
 * versions). Files whose only differences are numbers within 1e-12 (relative, absolute
 * below 1) are restored from the index (`git checkout`); every other change — a different string, structure, key
 * order or a larger numeric change, a new or a deleted file — is drift: it is reported and the exit code is 1.
 *
 * Engine tolerances (ENGINE below): a few values come from external tools whose output differs between versions
 * (ffmpeg 6.1 on CI's Ubuntu 24.04 and 8.x on Homebrew; poppler 24.02 and 26.x). They are compared within the
 * same tolerance the Rust tests apply, so goldens regenerated on macOS and Linux are the same goldens.
 */
import { execFileSync } from 'node:child_process';
import { readFileSync } from 'node:fs';

const TOL = 1e-12;
const MAX_REPORTED = 5;

const git = (args, opts = {}) => execFileSync('git', args, { encoding: 'utf8', maxBuffer: 1 << 30, ...opts });

/** sRGB bytes → CIE L*a*b* (D65), as crates/oc-render-svc/tests/video.rs `lab`. */
function lab(r, g, b) {
  const lin = (c) => {
    const v = c / 255;
    return v <= 0.04045 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4;
  };
  const [R, G, B] = [lin(r), lin(g), lin(b)];
  const f = (t) => (t > 0.008856 ? Math.cbrt(t) : 7.787 * t + 16 / 116);
  const x = f((0.4124 * R + 0.3576 * G + 0.1805 * B) / 0.95047);
  const y = f(0.2126 * R + 0.7152 * G + 0.0722 * B);
  const z = f((0.0193 * R + 0.1192 * G + 0.9505 * B) / 1.08883);
  return [116 * y - 16, 500 * (x - y), 200 * (y - z)];
}
/** Mean ΔE76 between two base64 RGB rasters of the same size (null when the sizes differ). */
function meanDeltaE(a64, b64) {
  const a = Buffer.from(a64, 'base64');
  const b = Buffer.from(b64, 'base64');
  if (a.length !== b.length || a.length % 3) return null;
  let sum = 0;
  for (let i = 0; i < a.length; i += 3) {
    const p = lab(a[i], a[i + 1], a[i + 2]);
    const q = lab(b[i], b[i + 1], b[i + 2]);
    sum += Math.hypot(p[0] - q[0], p[1] - q[1], p[2] - q[2]);
  }
  return sum / Math.max(1, a.length / 3);
}
const within = (tol) => (a, b) => typeof a === 'number' && typeof b === 'number' && Math.abs(a - b) <= tol;
/** [file, JSON path, accept(regenerated, committed)]: tool-version differences, with the Rust tests' tolerances. */
const ENGINE = [
  // Text colours sampled from pdftocairo renders (crates/oc-import/tests/pdf.rs: 0.05).
  [/^tests\/golden\/import\/pdf\//, /\.body\.defaultRun\.fill\.color\.[rgb]$/, within(0.05)],
  // Decoded frames of the real H.264 export (crates/oc-render-svc/tests/video.rs REAL_FRAME_TOL: mean ΔE 2;
  // libx264 of ffmpeg 6.1 vs 8.1 measured 1.06).
  [/^tests\/golden\/render-video\/real\.json$/, /^\$\.mp4\.frames\[\d+\]\.rgb$/, (a, b) => typeof a === 'string' && typeof b === 'string' && (meanDeltaE(a, b) ?? Infinity) <= 2],
  // Video frames ffmpeg decodes and scales for the painter, as 4×3 mean RGB (video.rs VIDEO_SIG_TOL: 4).
  [/^tests\/golden\/render-video\/plans\//, /^\$\.mp4\.renders\[\d+\]\.videos\.[^.[]+\[\d+\]$/, within(4)],
];

/** First difference between the regenerated value `a` and the committed value `b` (null when equal within TOL). */
function diff(a, b, path, out, accept = []) {
  if (a !== b && accept.some(([re, ok]) => re.test(path) && ok(a, b))) return;
  if (typeof a === 'number' && typeof b === 'number') {
    if (a !== b && !(Math.abs(a - b) <= TOL * Math.max(1, Math.abs(a), Math.abs(b)))) out.push(`${path}: ${b} -> ${a}`);
    return;
  }
  if (Array.isArray(a) && Array.isArray(b)) {
    if (a.length !== b.length) {
      out.push(`${path}: length ${b.length} -> ${a.length}`);
      return;
    }
    for (let i = 0; i < a.length && out.length < MAX_REPORTED; i++) diff(a[i], b[i], `${path}[${i}]`, out, accept);
    return;
  }
  if (a && b && typeof a === 'object' && typeof b === 'object' && !Array.isArray(a) && !Array.isArray(b)) {
    const ka = Object.keys(a);
    const kb = Object.keys(b);
    if (ka.join('\0') !== kb.join('\0')) {
      out.push(`${path}: keys ${JSON.stringify(kb).slice(0, 200)} -> ${JSON.stringify(ka).slice(0, 200)}`);
      return;
    }
    for (const k of ka) {
      if (out.length >= MAX_REPORTED) return;
      diff(a[k], b[k], `${path}.${k}`, out, accept);
    }
    return;
  }
  if (a !== b) out.push(`${path}: ${JSON.stringify(b)?.slice(0, 200)} -> ${JSON.stringify(a)?.slice(0, 200)}`);
}

// `git status --porcelain -z`: "XY path\0", with an extra "\0orig" entry after renames/copies.
const entries = git(['status', '--porcelain', '-z', '--untracked-files=all', '--', 'tests/golden']).split('\0');
const noise = [];
const drift = [];
for (let i = 0; i < entries.length; i++) {
  const e = entries[i];
  if (!e) continue;
  const xy = e.slice(0, 2);
  const path = e.slice(3);
  if (xy[0] === 'R' || xy[0] === 'C') i++;
  if (xy === '??') {
    drift.push({ path, why: ['new file (not committed)'] });
    continue;
  }
  if (xy.includes('D')) {
    drift.push({ path, why: ['deleted'] });
    continue;
  }
  let now;
  let committed;
  try {
    now = JSON.parse(readFileSync(path, 'utf8'));
    committed = JSON.parse(git(['show', `:${path}`]));
  } catch (err) {
    drift.push({ path, why: [`not comparable as JSON: ${err instanceof Error ? err.message : err}`] });
    continue;
  }
  const out = [];
  diff(now, committed, '$', out, ENGINE.filter(([file]) => file.test(path)).map(([, p, ok]) => [p, ok]));
  if (out.length) drift.push({ path, why: out });
  else noise.push(path);
}

if (noise.length) {
  git(['checkout', '--', ...noise]);
  console.log(`compare: restored ${noise.length} golden file(s) that differ only by float noise (<= ${TOL} relative) or within the engine tolerances:`);
  for (const p of noise) console.log(`  ${p}`);
}
if (drift.length) {
  console.error(`compare: ${drift.length} golden file(s) drifted from the committed version:`);
  for (const d of drift) {
    console.error(`  ${d.path}`);
    for (const w of d.why) console.error(`    ${w}`);
  }
  process.exit(1);
}
