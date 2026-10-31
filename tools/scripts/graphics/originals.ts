/**
 * Pinselo original decorative graphics (A4), authored programmatically as SVG and dedicated to the public domain (CC0-1.0).
 * Deterministic: a seeded RNG drives the hand-drawn jitter, so re-running the sync produces identical files.
 * No <text>: the rasteriser runs without system fonts, and graphics must look the same everywhere.
 */

export interface Original { slug: string; name: string; tags: string[]; svg: string }

// ------------------------------------------------------------------ helpers

function rng(seed: number) {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = a;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

type Pt = [number, number];
const f = (n: number) => (Math.round(n * 10) / 10).toString();
const P = (p: Pt) => `${f(p[0])} ${f(p[1])}`;
const TAU = Math.PI * 2;

const svg = (w: number, h: number, body: string) => `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 ${w} ${h}">${body}</svg>`;
const path = (d: string, attrs: string) => `<path d="${d}" ${attrs}/>`;
const fill = (c: string) => `fill="${c}"`;
const stroke = (c: string, w: number, extra = '') => `fill="none" stroke="${c}" stroke-width="${w}" stroke-linecap="round" stroke-linejoin="round"${extra ? ` ${extra}` : ''}`;
const circle = (cx: number, cy: number, r: number, attrs: string) => `<circle cx="${f(cx)}" cy="${f(cy)}" r="${f(r)}" ${attrs}/>`;
const rect = (x: number, y: number, w: number, h: number, attrs: string, rx = 0) => `<rect x="${f(x)}" y="${f(y)}" width="${f(w)}" height="${f(h)}"${rx ? ` rx="${f(rx)}"` : ''} ${attrs}/>`;

const poly = (pts: Pt[], close = true) => `M${pts.map(P).join(' L')}${close ? ' Z' : ''}`;

/** Catmull-Rom spline through points as cubic Béziers. */
function smooth(pts: Pt[], closed: boolean, k = 1): string {
  const n = pts.length;
  const at = (i: number) => (closed ? pts[(i + n) % n]! : pts[Math.max(0, Math.min(n - 1, i))]!);
  let d = `M${P(pts[0]!)}`;
  const segs = closed ? n : n - 1;
  for (let i = 0; i < segs; i++) {
    const p0 = at(i - 1), p1 = at(i), p2 = at(i + 1), p3 = at(i + 2);
    const c1: Pt = [p1[0] + ((p2[0] - p0[0]) / 6) * k, p1[1] + ((p2[1] - p0[1]) / 6) * k];
    const c2: Pt = [p2[0] - ((p3[0] - p1[0]) / 6) * k, p2[1] - ((p3[1] - p1[1]) / 6) * k];
    d += ` C${P(c1)} ${P(c2)} ${P(p2)}`;
  }
  return closed ? `${d} Z` : d;
}

function starPts(cx: number, cy: number, ro: number, ri: number, n: number, rot = -Math.PI / 2): Pt[] {
  const out: Pt[] = [];
  for (let i = 0; i < n * 2; i++) {
    const r = i % 2 ? ri : ro;
    const a = rot + (i * Math.PI) / n;
    out.push([cx + Math.cos(a) * r, cy + Math.sin(a) * r]);
  }
  return out;
}

/** Concave 4-point sparkle. */
function sparkle(cx: number, cy: number, r: number, pinch = 0.18): string {
  const q = r * pinch;
  return `M${f(cx)} ${f(cy - r)} Q${f(cx + q)} ${f(cy - q)} ${f(cx + r)} ${f(cy)} Q${f(cx + q)} ${f(cy + q)} ${f(cx)} ${f(cy + r)} Q${f(cx - q)} ${f(cy + q)} ${f(cx - r)} ${f(cy)} Q${f(cx - q)} ${f(cy - q)} ${f(cx)} ${f(cy - r)} Z`;
}

/** Open arrow head (two strokes) at `tip`, pointing along `angle`. */
function head(tip: Pt, angle: number, len: number, spread = 0.5): string {
  const a1 = angle + Math.PI - spread;
  const a2 = angle + Math.PI + spread;
  return ` M${f(tip[0] + Math.cos(a1) * len)} ${f(tip[1] + Math.sin(a1) * len)} L${P(tip)} L${f(tip[0] + Math.cos(a2) * len)} ${f(tip[1] + Math.sin(a2) * len)}`;
}

const angleOf = (a: Pt, b: Pt) => Math.atan2(b[1] - a[1], b[0] - a[0]);

/** Hand-drawn ellipse: a slightly wobbly loop that overshoots its start. */
function sketchEllipse(r: () => number, cx: number, cy: number, rx: number, ry: number, turns = 1.12, wobble = 0.05): string {
  const pts: Pt[] = [];
  const n = Math.round(14 * turns);
  const a0 = -Math.PI * 0.6;
  for (let i = 0; i <= n; i++) {
    const a = a0 + (i / n) * TAU * turns;
    const k = 1 + (r() - 0.5) * wobble * 2 + (i / n) * wobble;
    pts.push([cx + Math.cos(a) * rx * k, cy + Math.sin(a) * ry * k]);
  }
  return smooth(pts, false);
}

/** Hand-drawn straight-ish line between two points. */
function sketchLine(r: () => number, a: Pt, b: Pt, wobble = 2): string {
  const m: Pt = [(a[0] + b[0]) / 2 + (r() - 0.5) * wobble * 2, (a[1] + b[1]) / 2 + (r() - 0.5) * wobble * 2];
  return `M${P(a)} Q${P(m)} ${P(b)}`;
}

function blobPts(r: () => number, cx: number, cy: number, rad: number, n: number, irregular = 0.28): Pt[] {
  return Array.from({ length: n }, (_, i) => {
    const a = (i / n) * TAU + (r() - 0.5) * 0.4;
    const k = 1 - irregular / 2 + r() * irregular;
    return [cx + Math.cos(a) * rad * k, cy + Math.sin(a) * rad * k] as Pt;
  });
}

const PALETTES: string[][] = [
  ['#7c3aed', '#f472b6', '#fde68a'], ['#0ea5e9', '#22c55e', '#facc15'], ['#f97316', '#ef4444', '#fde047'], ['#14b8a6', '#0f766e', '#99f6e4'],
  ['#1f2937', '#f59e0b', '#f3f4f6'], ['#ec4899', '#8b5cf6', '#38bdf8'], ['#84cc16', '#166534', '#fef08a'], ['#f43f5e', '#fb923c', '#ffe4e6'],
];
const INK = '#1f2328';
const pal = (i: number) => PALETTES[i % PALETTES.length]!;
const pad = (n: number) => String(n).padStart(2, '0');

// ------------------------------------------------------------------ families

export function originalGraphics(): Original[] {
  const out: Original[] = [];
  const add = (family: string, i: number, name: string, tags: string[], body: string) => out.push({ slug: `${family}-${pad(i)}`, name, tags, svg: body });

  // Blobs (14)
  for (let i = 0; i < 14; i++) {
    const r = rng(100 + i);
    const [a, b, c] = pal(i);
    const n = 6 + (i % 4);
    const main = path(smooth(blobPts(r, 100, 100, 82, n), true), fill(a!));
    let extra = '';
    let variant = 'solid';
    if (i % 4 === 1) { extra = path(smooth(blobPts(r, 116, 112, 44, n), true), fill(b!)); variant = 'two-tone'; }
    if (i % 4 === 2) { extra = Array.from({ length: 9 }, (_, k) => circle(60 + (k % 3) * 40, 60 + Math.floor(k / 3) * 40, 5, fill(c!))).join(''); variant = 'dotted'; }
    if (i % 4 === 3) { extra = path(smooth(blobPts(r, 100, 100, 82, n), true), stroke(b!, 5)); variant = 'outlined'; }
    add('blob', i + 1, `blob ${variant}`, ['blob', 'organic', 'abstract', 'shape', 'fluid', 'background', variant], svg(200, 200, main + extra));
  }

  // Squiggles (10)
  for (let i = 0; i < 10; i++) {
    const r = rng(200 + i);
    const color = pal(i)[i % 3 === 2 ? 1 : 0]!;
    const pts: Pt[] = [];
    let d: string;
    let kind: string;
    if (i < 4) {
      const waves = 3 + i;
      for (let k = 0; k <= waves * 2; k++) pts.push([10 + (k * 280) / (waves * 2), 50 + (k % 2 ? -1 : 1) * (18 + r() * 10)]);
      d = smooth(pts, false);
      kind = 'wave';
    } else if (i < 6) {
      const n = 8 + (i - 4) * 4;
      for (let k = 0; k <= n; k++) pts.push([10 + (k * 280) / n, 50 + (k % 2 ? -22 : 22)]);
      d = poly(pts, false);
      kind = 'zigzag';
    } else if (i < 8) {
      const loops = 4 + (i - 6);
      for (let k = 0; k <= loops * 12; k++) {
        const t = k / 12;
        const a = t * TAU;
        pts.push([20 + t * (260 / loops) + Math.cos(a + Math.PI) * 14 + 14, 50 + Math.sin(a + Math.PI) * 22]);
      }
      d = smooth(pts, false);
      kind = 'loop';
    } else {
      for (let k = 0; k <= 40; k++) {
        const t = k / 40;
        pts.push([15 + t * 270, 50 + Math.sin(t * TAU * (i === 8 ? 2.5 : 4)) * 28 * (1 - Math.abs(t - 0.5))]);
      }
      d = smooth(pts, false);
      kind = 'swirl';
    }
    add('squiggle', i + 1, `squiggle ${kind}`, ['squiggle', 'line', 'wave', 'doodle', 'hand drawn', kind, 'decoration'], svg(300, 100, path(d, stroke(color, 8))));
  }

  // Hand-drawn arrows (16)
  const arrows: Array<[string, (r: () => number) => string, number, number]> = [
    ['straight', (r) => sketchLine(r, [20, 60], [260, 50], 4) + head([260, 50], angleOf([20, 60], [260, 50]), 28), 280, 110],
    ['curved', () => 'M20 90 Q140 -10 260 70' + head([260, 70], angleOf([140, -10], [260, 70]), 28), 280, 110],
    ['curved down', () => 'M30 20 Q220 10 230 150' + head([230, 150], angleOf([220, 10], [230, 150]), 26), 260, 170],
    ['loop', (r) => { const pts: Pt[] = [[20, 110], [90, 90], [130, 40], [100, 20], [80, 50], [120, 90], [200, 100], [260, 70]]; void r; return smooth(pts, false) + head([260, 70], angleOf([200, 100], [260, 70]), 26); }, 280, 130],
    ['zigzag', () => poly([[20, 80], [70, 30], [120, 80], [170, 30], [230, 70]], false) + head([230, 70], angleOf([170, 30], [230, 70]), 26), 260, 110],
    ['double', (r) => sketchLine(r, [30, 55], [250, 55], 3) + head([250, 55], 0, 26) + head([30, 55], Math.PI, 26), 280, 110],
    ['circular', () => { const pts: Pt[] = []; for (let k = 0; k <= 20; k++) { const a = -Math.PI * 0.35 + (k / 20) * TAU * 0.85; pts.push([110 + Math.cos(a) * 75, 110 + Math.sin(a) * 75]); } const e = pts[pts.length - 1]!; return smooth(pts, false) + head(e, angleOf(pts[pts.length - 2]!, e), 24); }, 220, 220],
    ['u-turn', () => 'M200 30 L80 30 Q30 30 30 80 Q30 130 80 130 L190 130' + head([190, 130], 0, 26), 230, 160],
    ['wavy', () => { const pts: Pt[] = []; for (let k = 0; k <= 16; k++) pts.push([20 + k * 14, 60 + Math.sin(k * 0.9) * 18]); const e = pts[pts.length - 1]!; return smooth(pts, false) + head(e, angleOf(pts[pts.length - 2]!, e), 24); }, 270, 120],
    ['spiral', () => { const pts: Pt[] = []; for (let k = 0; k <= 30; k++) { const a = (k / 30) * TAU * 1.4; const rr = 10 + k * 2.4; pts.push([100 + Math.cos(a) * rr, 100 + Math.sin(a) * rr]); } const e = pts[pts.length - 1]!; return smooth(pts, false) + head(e, angleOf(pts[pts.length - 2]!, e), 22); }, 200, 200],
    ['bouncing', () => 'M15 120 Q50 20 90 120 Q125 50 160 120 Q190 80 225 110' + head([225, 110], angleOf([190, 80], [225, 110]), 24), 250, 140],
    ['dashed', () => 'M20 100 Q150 0 260 80', 280, 120],
    ['brush', () => '', 280, 110],
    ['swoosh', () => 'M20 40 C80 140 200 140 260 50' + head([260, 50], angleOf([200, 140], [260, 50]), 28), 280, 130],
    ['corner', () => 'M30 20 L30 120 Q30 140 50 140 L200 140' + head([200, 140], 0, 26), 230, 170],
    ['scribble', (r) => { const pts: Pt[] = []; for (let k = 0; k <= 10; k++) pts.push([20 + k * 22 + (r() - 0.5) * 8, 70 + (r() - 0.5) * 40]); const e = pts[pts.length - 1]!; return smooth(pts, false) + head(e, angleOf(pts[pts.length - 2]!, e), 24); }, 270, 140],
  ];
  arrows.forEach(([kind, make, w, h], i) => {
    const r = rng(300 + i);
    const color = i % 3 === 0 ? INK : pal(i)[0]!;
    let body: string;
    if (kind === 'dashed') body = path(make(r), stroke(color, 7, 'stroke-dasharray="14 12"')) + path(head([260, 80], angleOf([150, 0], [260, 80]), 28).trim(), stroke(color, 7));
    else if (kind === 'brush') {
      const pts: Pt[] = [[20, 62], [100, 50], [190, 48], [190, 25], [262, 55], [190, 88], [190, 66], [100, 66], [22, 72]];
      body = path(smooth(pts.slice(0, 3), false).replace(/^M/, 'M') + ` L${P(pts[3]!)} L${P(pts[4]!)} L${P(pts[5]!)} L${P(pts[6]!)} Q100 70 22 72 Z`, fill(color));
    } else body = path(make(r), stroke(color, 7));
    add('arrow', i + 1, `hand drawn arrow ${kind}`, ['arrow', 'hand drawn', 'doodle', 'pointer', 'direction', 'sketch', ...kind.split(' ')], svg(w, h, body));
  });

  // Underlines & highlights (10)
  const underlines: Array<[string, (r: () => number, c: string) => string]> = [
    ['marker swoosh', (_r, c) => path('M10 62 C80 40 200 36 290 44 C292 52 290 58 286 60 C200 54 90 60 14 74 Z', fill(c))],
    ['double', (r, c) => path(sketchLine(r, [10, 40], [290, 36], 3) + ' ' + sketchLine(r, [24, 62], [276, 58], 3), stroke(c, 6))],
    ['wavy', (_r, c) => { const pts: Pt[] = []; for (let k = 0; k <= 24; k++) pts.push([10 + k * 11.6, 50 + (k % 2 ? -9 : 9)]); return path(smooth(pts, false), stroke(c, 6)); }],
    ['scribble', (_r, c) => { const pts: Pt[] = []; for (let k = 0; k <= 18; k++) pts.push([12 + k * 15 + (k % 2 ? 8 : 0), k % 2 ? 36 : 64]); return path(poly(pts, false), stroke(c, 5)); }],
    ['dashed', (_r, c) => path('M14 50 L286 50', stroke(c, 8, 'stroke-dasharray="22 14"'))],
    ['brush stroke', (r, c) => { const top: Pt[] = []; const bot: Pt[] = []; for (let k = 0; k <= 12; k++) { top.push([10 + k * 23, 34 + (r() - 0.5) * 6]); bot.push([10 + k * 23, 66 + (r() - 0.5) * 6]); } return path(`${poly(top, false)} L${bot.reverse().map(P).join(' L')} Z`, fill(c)); }],
    ['highlight bar', (_r, c) => rect(8, 28, 284, 44, `${fill(c)} opacity="0.45"`, 10)],
    ['smile', (_r, c) => path('M14 30 Q150 90 286 30', stroke(c, 7))],
    ['tapered', (_r, c) => path('M10 52 Q150 40 290 46 Q150 56 10 58 Z', fill(c))],
    ['circle highlight', (r, c) => path(sketchEllipse(r, 150, 50, 138, 36, 1.15, 0.03), stroke(c, 5))],
  ];
  underlines.forEach(([kind, make], i) => {
    const r = rng(400 + i);
    add('underline', i + 1, `underline ${kind}`, ['underline', 'highlight', 'emphasis', 'marker', 'text decoration', ...kind.split(' ')], svg(300, 100, make(r, pal(i)[i % 2]!)));
  });

  // Badges & seals (12)
  for (let i = 0; i < 12; i++) {
    const [a, b, c] = pal(i + 2);
    let body = '';
    let kind = '';
    switch (i % 6) {
      case 0: { const n = 16 + (i % 3) * 4; body = path(poly(starPts(100, 100, 92, 80, n)), fill(a!)) + circle(100, 100, 64, `fill="none" stroke="${c}" stroke-width="3"`); kind = 'starburst seal'; break; }
      case 1: { const pts: Pt[] = []; for (let k = 0; k < 14; k++) { const a0 = (k / 14) * TAU; pts.push([100 + Math.cos(a0) * 76, 100 + Math.sin(a0) * 76]); } body = pts.map((p) => circle(p[0], p[1], 18, fill(a!))).join('') + circle(100, 100, 76, fill(a!)) + circle(100, 100, 58, fill(c!)); kind = 'scalloped'; break; }
      case 2: body = path('M70 120 L50 192 L78 178 L92 200 L104 126 Z', fill(b!)) + path('M130 120 L150 192 L122 178 L108 200 L96 126 Z', fill(b!)) + path(poly(starPts(100, 84, 78, 68, 12)), fill(a!)) + circle(100, 84, 50, fill(c!)); kind = 'rosette'; break;
      case 3: body = path('M100 12 L176 40 Q176 140 100 190 Q24 140 24 40 Z', fill(a!)) + path('M100 32 L158 54 Q158 130 100 170 Q42 130 42 54 Z', `fill="none" stroke="${c}" stroke-width="4"`); kind = 'shield'; break;
      case 4: body = path(poly([[100, 10], [178, 55], [178, 145], [100, 190], [22, 145], [22, 55]]), fill(a!)) + path(poly([[100, 32], [158, 66], [158, 134], [100, 168], [42, 134], [42, 66]]), `fill="none" stroke="${c}" stroke-width="4" stroke-dasharray="8 6"`); kind = 'hexagon'; break;
      default: body = circle(100, 100, 88, fill(a!)) + circle(100, 100, 76, `fill="none" stroke="${c}" stroke-width="3"`) + circle(100, 100, 64, `fill="none" stroke="${c}" stroke-width="2" stroke-dasharray="4 5"`) + path(poly(starPts(100, 100, 26, 11, 5)), fill(b!)); kind = 'stamp';
    }
    add('badge', i + 1, `badge ${kind}`, ['badge', 'seal', 'sticker', 'label', 'award', 'emblem', ...kind.split(' ')], svg(200, 200, body));
  }

  // Frames & borders (12)
  const frames: Array<[string, (r: () => number, c: string, d: string) => string]> = [
    ['double line', (_r, c) => rect(8, 8, 284, 184, `fill="none" stroke="${c}" stroke-width="4"`) + rect(20, 20, 260, 160, `fill="none" stroke="${c}" stroke-width="2"`)],
    ['dashed rounded', (_r, c) => rect(10, 10, 280, 180, `fill="none" stroke="${c}" stroke-width="5" stroke-dasharray="16 10" stroke-linecap="round"`, 24)],
    ['double circle', (_r, c, d) => circle(100, 100, 90, `fill="none" stroke="${c}" stroke-width="5"`) + circle(100, 100, 78, `fill="none" stroke="${d}" stroke-width="3"`)],
    ['sketch rectangle', (r, c) => path([sketchLine(r, [12, 14], [288, 10]), sketchLine(r, [286, 6], [290, 188]), sketchLine(r, [292, 186], [10, 190]), sketchLine(r, [14, 194], [8, 12]), sketchLine(r, [8, 20], [284, 16], 4)].join(' '), stroke(c, 4))],
    ['sketch circle', (r, c) => path(sketchEllipse(r, 100, 100, 86, 86, 1.18, 0.04), stroke(c, 5))],
    ['corner brackets', (_r, c) => path('M10 60 L10 10 L60 10 M240 10 L290 10 L290 60 M290 140 L290 190 L240 190 M60 190 L10 190 L10 140', stroke(c, 7))],
    ['scalloped', (_r, c) => { let d = 'M20 20'; for (let k = 0; k < 13; k++) d += ` q10 -14 20 0`; for (let k = 0; k < 8; k++) d += ` q14 10 0 20`; for (let k = 0; k < 13; k++) d += ` q-10 14 -20 0`; for (let k = 0; k < 8; k++) d += ` q-14 -10 0 -20`; return path(`${d} Z`, stroke(c, 4)); }],
    ['instant photo', (_r, c, d) => path('M10 10 H190 V230 H10 Z M26 26 V180 H174 V26 Z', `${fill(c)} fill-rule="evenodd"`) + rect(26, 26, 148, 154, fill(d))],
    ['arch', (_r, c) => path('M20 230 V100 A80 80 0 0 1 180 100 V230 Z', `fill="none" stroke="${c}" stroke-width="6"`)],
    ['ornate corners', (_r, c) => ['M12 70 C12 30 30 12 70 12 M30 50 C30 38 38 30 50 30', 'M288 70 C288 30 270 12 230 12 M270 50 C270 38 262 30 250 30', 'M12 130 C12 170 30 188 70 188 M30 150 C30 162 38 170 50 170', 'M288 130 C288 170 270 188 230 188 M270 150 C270 162 262 170 250 170'].map((d) => path(d, stroke(c, 5))).join('')],
    ['dotted circle', (_r, c) => circle(100, 100, 86, `fill="none" stroke="${c}" stroke-width="8" stroke-dasharray="0.1 18" stroke-linecap="round"`)],
    ['wavy circle', (_r, c) => { const pts: Pt[] = []; for (let k = 0; k < 48; k++) { const a = (k / 48) * TAU; const rr = 84 + (k % 2 ? 6 : -6); pts.push([100 + Math.cos(a) * rr, 100 + Math.sin(a) * rr]); } return path(smooth(pts, true), stroke(c, 5)); }],
  ];
  frames.forEach(([kind, make], i) => {
    const r = rng(500 + i);
    const [a, b] = pal(i + 1);
    const square = /circle/.test(kind);
    const tall = kind === 'instant photo' || kind === 'arch';
    add('frame', i + 1, `frame ${kind}`, ['frame', 'border', 'outline', 'decoration', ...kind.split(' ')], svg(square ? 200 : tall ? 200 : 300, square ? 200 : tall ? 240 : 200, make(r, i % 2 ? a! : INK, b!)));
  });

  // Dividers (10)
  const dividers: Array<[string, (c: string, d: string) => string]> = [
    ['diamond', (c, d) => path('M10 30 H128 M172 30 H290', stroke(c, 3)) + path(poly([[150, 16], [164, 30], [150, 44], [136, 30]]), fill(d))],
    ['three dots', (c, d) => circle(110, 30, 7, fill(c)) + circle(150, 30, 7, fill(d)) + circle(190, 30, 7, fill(c))],
    ['wave', (c) => { const pts: Pt[] = []; for (let k = 0; k <= 20; k++) pts.push([10 + k * 14, 30 + (k % 2 ? -8 : 8)]); return path(smooth(pts, false), stroke(c, 4)); }],
    ['line dot line', (c, d) => path('M10 30 H135 M165 30 H290', stroke(c, 3)) + circle(150, 30, 8, fill(d))],
    ['stars', (c, d) => [100, 150, 200].map((x, k) => path(poly(starPts(x, 30, k === 1 ? 16 : 11, k === 1 ? 7 : 5, 5)), fill(k === 1 ? d : c))).join('') + path('M10 30 H80 M220 30 H290', stroke(c, 2))],
    ['zigzag', (c) => { const pts: Pt[] = []; for (let k = 0; k <= 28; k++) pts.push([10 + k * 10, k % 2 ? 22 : 38]); return path(poly(pts, false), stroke(c, 3)); }],
    ['double line', (c) => path('M10 24 H290 M10 36 H290', stroke(c, 2.5))],
    ['vine', (c, d) => path('M10 30 C80 10 220 50 290 30', stroke(c, 3)) + [50, 95, 140, 185, 230].map((x, k) => path(`M${x} ${k % 2 ? 34 : 26} q12 ${k % 2 ? 18 : -18} 26 ${k % 2 ? 6 : -6} q-14 ${k % 2 ? -12 : 12} -26 ${k % 2 ? -6 : 6} Z`, fill(d))).join('')],
    ['scroll', (c) => path('M30 30 H120 C130 30 135 18 145 18 C158 18 158 34 150 36 C142 38 140 26 150 26 M270 30 H180 C170 30 165 42 155 42 C142 42 142 26 150 24', stroke(c, 3))],
    ['dashed ends', (c, d) => path('M30 30 H270', stroke(c, 3, 'stroke-dasharray="10 8"')) + circle(20, 30, 8, fill(d)) + circle(280, 30, 8, fill(d))],
  ];
  dividers.forEach(([kind, make], i) => {
    const [a, b] = pal(i);
    add('divider', i + 1, `divider ${kind}`, ['divider', 'separator', 'line', 'rule', 'ornament', ...kind.split(' ')], svg(300, 60, make(i % 2 ? INK : a!, b!)));
  });

  // Sparkles & emphasis marks (12)
  const sparkles: Array<[string, (r: () => number, a: string, b: string) => string]> = [
    ['four point', (_r, a) => path(sparkle(100, 100, 90), fill(a))],
    ['cluster', (_r, a, b) => path(sparkle(80, 110, 70), fill(a)) + path(sparkle(160, 50, 32), fill(b)) + path(sparkle(160, 160, 22), fill(a))],
    ['eight point', (_r, a) => path(poly(starPts(100, 100, 90, 22, 8)), fill(a))],
    ['twinkle', (_r, a, b) => path(sparkle(100, 100, 88, 0.08), fill(a)) + path('M100 50 V150 M50 100 H150', stroke(b, 3))],
    ['emphasis lines', (_r, a) => path('M40 120 L15 100 M70 80 L58 45 M110 76 L122 40 M140 98 L170 78', stroke(a, 9))],
    ['burst lines', (_r, a) => { let d = ''; for (let k = 0; k < 12; k++) { const t = (k / 12) * TAU; d += ` M${f(100 + Math.cos(t) * 42)} ${f(100 + Math.sin(t) * 42)} L${f(100 + Math.cos(t) * 86)} ${f(100 + Math.sin(t) * 86)}`; } return path(d.trim(), stroke(a, 8)); }],
    ['shine', (_r, a, b) => path(sparkle(90, 110, 70), fill(a)) + circle(160, 50, 9, fill(b)) + circle(40, 40, 6, fill(b)) + circle(170, 150, 6, fill(b))],
    ['asterisk', (_r, a) => { let d = ''; for (let k = 0; k < 6; k++) { const t = (k / 6) * Math.PI; d += ` M${f(100 - Math.cos(t) * 80)} ${f(100 - Math.sin(t) * 80)} L${f(100 + Math.cos(t) * 80)} ${f(100 + Math.sin(t) * 80)}`; } return path(d.trim(), stroke(a, 16)); }],
    ['doodle star', (r, a) => path(smooth(starPts(100, 104, 88, 38, 5).map(([x, y]) => [x + (r() - 0.5) * 6, y + (r() - 0.5) * 6] as Pt), true, 0.25), stroke(a, 6))],
    ['starburst', (_r, a, b) => path(poly(starPts(100, 100, 92, 58, 14)), fill(a)) + path(poly(starPts(100, 100, 56, 36, 14, 0)), fill(b))],
    ['glitter', (r, a, b) => Array.from({ length: 14 }, (_, k) => (k % 3 === 0 ? path(sparkle(20 + r() * 160, 20 + r() * 160, 10 + r() * 14), fill(a)) : circle(15 + r() * 170, 15 + r() * 170, 3 + r() * 5, fill(k % 2 ? a : b)))).join('')],
    ['comic burst', (r, a, b) => path(poly(starPts(100, 100, 94, 60, 11).map(([x, y]) => [x + (r() - 0.5) * 14, y + (r() - 0.5) * 14] as Pt)), `${fill(a)} stroke="${b}" stroke-width="5" stroke-linejoin="round"`)],
  ];
  sparkles.forEach(([kind, make], i) => {
    const r = rng(600 + i);
    const [a, b] = pal(i + 3);
    add('sparkle', i + 1, `sparkle ${kind}`, ['sparkle', 'star', 'shine', 'twinkle', 'magic', 'emphasis', ...kind.split(' ')], svg(200, 200, make(r, a!, b!)));
  });

  // Hand-drawn doodles (12)
  const doodles: Array<[string, (r: () => number, a: string, b: string) => string, string[]]> = [
    ['scribble circle', (r, a) => path(sketchEllipse(r, 100, 100, 84, 70, 2.1, 0.06), stroke(a, 5)), ['circle', 'highlight']],
    ['cross out', (r, a) => path(sketchLine(r, [30, 30], [170, 170], 6) + ' ' + sketchLine(r, [170, 34], [34, 168], 6), stroke(a, 12)), ['x', 'wrong', 'no', 'cancel']],
    ['check mark', (_r, a) => path('M30 110 Q55 125 80 160 Q110 80 175 35', stroke(a, 14)), ['tick', 'yes', 'done', 'correct']],
    ['heart', (_r, a) => path('M100 170 C40 130 20 95 32 62 C45 30 88 32 100 66 C112 32 158 30 170 62 C182 95 160 130 100 170 Z', stroke(a, 7)), ['love', 'like', 'valentine']],
    ['cloud', (_r, a) => path('M50 140 C20 140 18 100 48 96 C44 64 86 54 100 78 C110 50 160 52 158 88 C190 88 192 140 160 140 Z', stroke(a, 6)), ['weather', 'sky']],
    ['sun', (_r, a, b) => { let d = ''; for (let k = 0; k < 10; k++) { const t = (k / 10) * TAU; d += ` M${f(100 + Math.cos(t) * 56)} ${f(100 + Math.sin(t) * 56)} L${f(100 + Math.cos(t) * 84)} ${f(100 + Math.sin(t) * 84)}`; } return circle(100, 100, 40, fill(b)) + circle(100, 100, 40, stroke(a, 5)) + path(d.trim(), stroke(a, 6)); }, ['weather', 'summer', 'sunshine']],
    ['lightning', (_r, a, b) => path('M118 10 L50 112 L96 112 L80 190 L152 82 L104 82 Z', `${fill(b)} stroke="${a}" stroke-width="6" stroke-linejoin="round"`), ['bolt', 'energy', 'power', 'flash']],
    ['smiley', (_r, a, b) => circle(100, 100, 80, fill(b)) + circle(100, 100, 80, stroke(a, 6)) + circle(74, 82, 8, fill(a)) + circle(126, 82, 8, fill(a)) + path('M62 118 Q100 156 138 118', stroke(a, 6)), ['smile', 'happy', 'face', 'emoji']],
    ['flower', (_r, a, b) => Array.from({ length: 6 }, (_, k) => { const t = (k / 6) * TAU; return `<ellipse cx="${f(100 + Math.cos(t) * 46)}" cy="${f(100 + Math.sin(t) * 46)}" rx="34" ry="22" transform="rotate(${f((t * 180) / Math.PI)} ${f(100 + Math.cos(t) * 46)} ${f(100 + Math.sin(t) * 46)})" ${fill(a)}/>`; }).join('') + circle(100, 100, 28, fill(b)), ['floral', 'spring', 'nature', 'bloom']],
    ['hatching', (_r, a) => { let d = ''; for (let k = 0; k < 12; k++) d += ` M${20 + k * 14} 180 L${60 + k * 14} 20`; return path(d.trim(), stroke(a, 5)); }, ['texture', 'shading', 'lines']],
    ['spiral', (_r, a) => { const pts: Pt[] = []; for (let k = 0; k <= 60; k++) { const t = (k / 60) * TAU * 3.2; const rr = 4 + k * 1.4; pts.push([100 + Math.cos(t) * rr, 100 + Math.sin(t) * rr]); } return path(smooth(pts, false), stroke(a, 6)); }, ['swirl', 'hypnotic', 'twist']],
    ['exclamation', (_r, a) => path('M90 30 Q100 26 110 30 L104 130 Q100 134 96 130 Z', fill(a)) + circle(100, 162, 12, fill(a)), ['alert', 'attention', 'important']],
  ];
  doodles.forEach(([kind, make, extra], i) => {
    const r = rng(700 + i);
    const [a, b] = pal(i + 5);
    add('doodle', i + 1, `doodle ${kind}`, ['doodle', 'hand drawn', 'sketch', 'scribble', ...kind.split(' '), ...extra], svg(200, 200, make(r, i % 3 === 0 ? INK : a!, b!)));
  });

  // Abstract shapes (20)
  const shapes: Array<[string, (r: () => number, p: string[]) => string, string[]]> = [
    ['half circles', (_r, [a, b]) => path('M20 100 A80 80 0 0 1 180 100 Z', fill(a!)) + path('M20 110 A80 80 0 0 0 180 110 Z', fill(b!)), ['semicircle', 'geometric']],
    ['quarter circles', (_r, [a, b, c]) => path('M10 10 H95 A85 85 0 0 1 10 95 Z', fill(a!)) + path('M190 10 V95 A85 85 0 0 1 105 10 Z', fill(b!)) + path('M10 190 V105 A85 85 0 0 1 95 190 Z', fill(c!)) + path('M190 190 H105 A85 85 0 0 1 190 105 Z', fill(a!)), ['geometric', 'bauhaus']],
    ['concentric rings', (_r, [a, b, c]) => circle(100, 100, 84, stroke(a!, 12)) + circle(100, 100, 56, stroke(b!, 12)) + circle(100, 100, 28, stroke(c!, 12)), ['circles', 'target', 'geometric']],
    ['rainbow', (_r, [a, b, c]) => path('M20 170 A80 80 0 0 1 180 170', stroke(a!, 18, '').replace('stroke-linecap="round"', 'stroke-linecap="butt"')) + path('M44 170 A56 56 0 0 1 156 170', stroke(b!, 18).replace('stroke-linecap="round"', 'stroke-linecap="butt"')) + path('M68 170 A32 32 0 0 1 132 170', stroke(c!, 18).replace('stroke-linecap="round"', 'stroke-linecap="butt"')), ['arc', 'boho', 'retro']],
    ['memphis', (r, [a, b, c]) => path(poly([[30, 150], [80, 60], [120, 150]]), fill(a!)) + path('M120 40 l15 15 l15 -15 l15 15 l15 -15', stroke(b!, 6)) + Array.from({ length: 6 }, () => circle(110 + r() * 70, 110 + r() * 70, 6, fill(c!))).join(''), ['80s', 'retro', 'geometric', 'pattern']],
    ['pill stack', (_r, [a, b, c]) => rect(20, 30, 160, 40, fill(a!), 20) + rect(50, 80, 130, 40, fill(b!), 20) + rect(20, 130, 110, 40, fill(c!), 20), ['capsule', 'rounded']],
    ['split circle', (_r, [a, b]) => path('M100 15 A85 85 0 0 0 100 185 Z', fill(a!)) + path('M100 15 A85 85 0 0 1 100 185 Z', fill(b!)), ['half', 'yin yang', 'geometric']],
    ['arch', (_r, [a, b]) => path('M30 190 V100 A70 70 0 0 1 170 100 V190 Z', fill(a!)) + path('M60 190 V110 A40 40 0 0 1 140 110 V190 Z', fill(b!)), ['doorway', 'boho']],
    ['leaf', (_r, [a, b]) => path('M30 170 C30 80 90 30 170 30 C170 120 110 170 30 170 Z', fill(a!)) + path('M30 170 L150 50', stroke(b!, 4)), ['nature', 'plant', 'organic']],
    ['drop', (_r, [a]) => path('M100 15 C130 70 160 100 160 130 A60 60 0 0 1 40 130 C40 100 70 70 100 15 Z', fill(a!)), ['water', 'liquid', 'tear']],
    ['crescent', (_r, [a]) => path('M130 20 A85 85 0 1 0 180 140 A70 70 0 1 1 130 20 Z', fill(a!)), ['moon', 'night']],
    ['crosses', (_r, [a, b]) => [[50, 50, a], [150, 50, b], [50, 150, b], [150, 150, a]].map(([x, y, c]) => path(`M${Number(x) - 30} ${y} H${Number(x) + 30} M${x} ${Number(y) - 30} V${Number(y) + 30}`, stroke(String(c), 14))).join(''), ['plus', 'geometric', 'swiss']],
    ['tilted squares', (_r, [a, b]) => `<rect x="40" y="40" width="120" height="120" rx="18" transform="rotate(15 100 100)" ${fill(a!)}/><rect x="60" y="60" width="80" height="80" rx="12" transform="rotate(-10 100 100)" ${fill(b!)}/>`, ['square', 'rounded', 'geometric']],
    ['wave band', (_r, [a, b]) => path('M0 80 C50 40 100 120 150 80 S250 40 300 80 V140 C250 180 200 100 150 140 S50 180 0 140 Z', fill(a!)) + path('M0 110 C50 70 100 150 150 110 S250 70 300 110', stroke(b!, 5)), ['wave', 'flow', 'organic', 'background']],
    ['rising sun', (_r, [a, b]) => { let d = ''; for (let k = 0; k <= 8; k++) { const t = Math.PI + (k / 8) * Math.PI; d += ` M${f(100 + Math.cos(t) * 62)} ${f(150 + Math.sin(t) * 62)} L${f(100 + Math.cos(t) * 90)} ${f(150 + Math.sin(t) * 90)}`; } return path('M50 150 A50 50 0 0 1 150 150 Z', fill(a!)) + path(d.trim(), stroke(b!, 6)) + path('M10 160 H190', stroke(b!, 6)); }, ['sunrise', 'sunset', 'boho', 'retro']],
    ['mountains', (_r, [a, b, c]) => path(poly([[10, 180], [75, 70], [140, 180]]), fill(a!)) + path(poly([[65, 180], [128, 40], [190, 180]]), fill(b!)) + path(poly([[107.5, 85], [128, 40], [148.5, 85], [138, 79], [128, 91], [118, 79]]), fill(c!)), ['landscape', 'nature', 'outdoor', 'peak']],
    ['steps', (_r, [a, b, c]) => rect(20, 130, 50, 50, fill(a!)) + rect(75, 90, 50, 90, fill(b!)) + rect(130, 40, 50, 140, fill(c!)), ['bars', 'growth', 'chart']],
    ['dot grid circles', (_r, [a, b]) => Array.from({ length: 16 }, (_, k) => circle(32 + (k % 4) * 45, 32 + Math.floor(k / 4) * 45, 16, fill((k + Math.floor(k / 4)) % 2 ? a! : b!))).join(''), ['dots', 'grid', 'polka']],
    ['vesica', (_r, [a, b]) => path('M20 100 Q100 10 180 100 Q100 190 20 100 Z', fill(a!)) + circle(100, 100, 30, fill(b!)), ['eye', 'lens', 'geometric']],
    ['triangle trio', (_r, [a, b, c]) => path(poly([[20, 170], [70, 80], [120, 170]]), fill(a!)) + path(poly([[80, 170], [130, 60], [180, 170]]), `${fill(b!)} opacity="0.85"`) + path(poly([[60, 120], [100, 30], [140, 120]]), `${fill(c!)} opacity="0.85"`), ['triangles', 'geometric']],
  ];
  shapes.forEach(([kind, make, extra], i) => {
    const r = rng(800 + i);
    const w = kind === 'wave band' ? 300 : 200;
    add('shape', i + 1, `abstract ${kind}`, ['abstract', 'shape', 'geometric', 'decoration', ...kind.split(' '), ...extra], svg(w, w === 300 ? 220 : 200, make(r, pal(i))));
  });

  // Patterns (16)
  const patterns: Array<[string, (r: () => number, a: string, b: string) => string]> = [
    ['dot grid', (_r, a) => Array.from({ length: 100 }, (_, k) => circle(10 + (k % 10) * 20, 10 + Math.floor(k / 10) * 20, 4, fill(a))).join('')],
    ['diagonal stripes', (_r, a) => { let d = ''; for (let k = -10; k < 20; k++) d += ` M${k * 20} 0 L${k * 20 + 200} 200`; return path(d.trim(), `fill="none" stroke="${a}" stroke-width="7"`); }],
    ['checker', (_r, a) => Array.from({ length: 64 }, (_, k) => ((k % 8) + Math.floor(k / 8)) % 2 ? rect((k % 8) * 25, Math.floor(k / 8) * 25, 25, 25, fill(a)) : '').join('')],
    ['waves', (_r, a) => { let d = ''; for (let y = 20; y < 200; y += 30) { d += ` M0 ${y}`; for (let x = 0; x < 200; x += 40) d += ` q10 -12 20 0 q10 12 20 0`; } return path(d.trim(), stroke(a, 4)); }],
    ['triangles', (_r, a, b) => Array.from({ length: 25 }, (_, k) => { const x = (k % 5) * 40; const y = Math.floor(k / 5) * 40; return path(poly([[x + 5, y + 35], [x + 20, y + 8], [x + 35, y + 35]]), fill(k % 2 ? a : b)); }).join('')],
    ['plus signs', (_r, a) => { let d = ''; for (let k = 0; k < 36; k++) { const x = 17 + (k % 6) * 33; const y = 17 + Math.floor(k / 6) * 33; d += ` M${x - 8} ${y} H${x + 8} M${x} ${y - 8} V${y + 8}`; } return path(d.trim(), stroke(a, 4)); }],
    ['alternating circles', (_r, a, b) => Array.from({ length: 25 }, (_, k) => circle(20 + (k % 5) * 40, 20 + Math.floor(k / 5) * 40, 14, k % 2 ? fill(a) : stroke(b, 4))).join('')],
    ['confetti', (r, a, b) => Array.from({ length: 40 }, (_, k) => { const x = r() * 190; const y = r() * 190; const c = k % 3 === 0 ? b : a; return k % 2 ? `<rect x="${f(x)}" y="${f(y)}" width="12" height="5" rx="2" transform="rotate(${f(r() * 180)} ${f(x + 6)} ${f(y + 2.5)})" ${fill(c)}/>` : circle(x, y, 3.5, fill(c)); }).join('')],
    ['halftone', (_r, a) => Array.from({ length: 144 }, (_, k) => { const col = k % 12; const row = Math.floor(k / 12); return circle(8 + col * 16.5 + (row % 2 ? 8 : 0), 8 + row * 16.5, Math.max(0.6, 7 * (1 - col / 12)), fill(a)); }).join('')],
    ['zigzag rows', (_r, a, b) => Array.from({ length: 7 }, (_, row) => { const pts: Pt[] = []; for (let k = 0; k <= 20; k++) pts.push([k * 10, 18 + row * 28 + (k % 2 ? -7 : 7)]); return path(poly(pts, false), stroke(row % 2 ? a : b, 4)); }).join('')],
    ['grid lines', (_r, a) => { let d = ''; for (let k = 0; k <= 10; k++) d += ` M${k * 20} 0 V200 M0 ${k * 20} H200`; return path(d.trim(), `fill="none" stroke="${a}" stroke-width="1.5"`); }],
    ['scales', (_r, a) => { let d = ''; for (let row = 0; row < 9; row++) for (let k = 0; k < 6; k++) { const x = k * 40 + (row % 2 ? 20 : 0) - 20; const y = row * 22 + 20; d += ` M${x} ${y} a20 20 0 0 0 40 0`; } return path(d.trim(), stroke(a, 3)); }],
    ['hexagons', (_r, a) => { let d = ''; const s = 18; for (let row = 0; row < 8; row++) for (let k = 0; k < 7; k++) { const cx = k * s * 1.732 + (row % 2 ? s * 0.866 : 0); const cy = row * s * 1.5; d += ` ${poly(Array.from({ length: 6 }, (_, j) => { const t = Math.PI / 6 + (j / 6) * TAU; return [cx + Math.cos(t) * s, cy + Math.sin(t) * s] as Pt; }))}`; } return path(d.trim(), `fill="none" stroke="${a}" stroke-width="2"`); }],
    ['polka', (_r, a, b) => Array.from({ length: 50 }, (_, k) => { const row = Math.floor(k / 5); return circle(20 + (k % 5) * 40 + (row % 2 ? 20 : 0), 10 + row * 20, 7, fill(row % 2 ? a : b)); }).join('')],
    ['crosshatch', (_r, a) => { let d = ''; for (let k = -10; k < 20; k++) d += ` M${k * 16} 0 L${k * 16 + 200} 200 M${k * 16 + 200} 0 L${k * 16} 200`; return path(d.trim(), `fill="none" stroke="${a}" stroke-width="1.5"`); }],
    ['terrazzo', (r, a, b) => Array.from({ length: 30 }, (_, k) => path(smooth(blobPts(r, 10 + r() * 180, 10 + r() * 180, 4 + r() * 9, 5, 0.6), true), fill(k % 3 ? a : b))).join('')],
  ];
  patterns.forEach(([kind, make], i) => {
    const r = rng(900 + i);
    const [a, b] = pal(i + 1);
    const body = `<defs><clipPath id="c"><rect width="200" height="200"/></clipPath></defs><g clip-path="url(#c)">${make(r, i % 4 === 0 ? INK : a!, b!)}</g>`;
    add('pattern', i + 1, `pattern ${kind}`, ['pattern', 'texture', 'background', 'seamless', 'tile', ...kind.split(' ')], svg(200, 200, body));
  });

  // Speech bubbles & callouts (8)
  const bubbles: Array<[string, (a: string, b: string) => string, number, number]> = [
    ['round', (a) => path('M110 20 C170 20 200 50 200 90 C200 130 170 160 110 160 C98 160 86 158 76 155 L40 180 L50 145 C30 132 20 112 20 90 C20 50 50 20 110 20 Z', fill(a)), 220, 200],
    ['rectangle', (a) => path('M20 20 H200 Q210 20 210 30 V130 Q210 140 200 140 H80 L50 175 L56 140 H20 Q10 140 10 130 V30 Q10 20 20 20 Z', fill(a)), 220, 190],
    ['thought', (a) => path('M60 120 C20 120 20 70 55 66 C50 30 100 20 115 45 C130 15 185 25 180 62 C215 66 210 120 170 120 Z', fill(a)) + circle(60, 150, 12, fill(a)) + circle(38, 176, 7, fill(a)), 220, 200],
    ['shout', (a, b) => path(poly(starPts(110, 90, 100, 64, 13).map(([x, y]) => [x, (y - 90) * 0.8 + 90] as Pt)), `${fill(a)} stroke="${b}" stroke-width="5" stroke-linejoin="round"`), 220, 180],
    ['outline', (a) => path('M110 20 C170 20 200 50 200 90 C200 130 170 160 110 160 C98 160 86 158 76 155 L40 180 L50 145 C30 132 20 112 20 90 C20 50 50 20 110 20 Z', `fill="none" stroke="${a}" stroke-width="6" stroke-linejoin="round"`), 220, 200],
    ['conversation', (a, b) => path('M20 20 H130 Q140 20 140 30 V90 Q140 100 130 100 H60 L40 122 L44 100 H20 Q10 100 10 90 V30 Q10 20 20 20 Z', fill(a)) + path('M200 80 H100 Q90 80 90 90 V150 Q90 160 100 160 H170 L190 182 L186 160 H200 Q210 160 210 150 V90 Q210 80 200 80 Z', fill(b)), 220, 200],
    ['arrow box', (a) => path('M10 20 H170 L210 80 L170 140 H10 Z', fill(a)), 220, 160],
    ['tape label', (a, b) => rect(20, 50, 180, 90, fill(a), 6) + `<rect x="80" y="30" width="60" height="34" transform="rotate(-6 110 47)" ${fill(b)} opacity="0.8"/>`, 220, 170],
  ];
  bubbles.forEach(([kind, make, w, h], i) => {
    const [a, b] = pal(i + 4);
    add('bubble', i + 1, `speech bubble ${kind}`, ['speech bubble', 'callout', 'chat', 'comment', 'talk', 'message', ...kind.split(' ')], svg(w, h, make(i % 4 === 0 ? INK : a!, b!)));
  });

  return out;
}
