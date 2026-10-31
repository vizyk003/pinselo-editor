/** Find CC0 photos for templates: `pnpm tpl:photos "<query>" [count] [tall|wide|square]` → contact sheet with ids. */
import { mkdirSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { Resvg } from '@resvg/resvg-js';
import { paths } from '@oc/config/node';
import { searchPhotos } from './templates/photos';

const q = process.argv[2];
if (!q) throw new Error('usage: pnpm tpl:photos "<query>" [count] [tall|wide|square]');
const n = Number(process.argv[3] ?? 12);
const list = await searchPhotos(q, n, { orientation: process.argv[4] as 'tall' | 'wide' | 'square' | undefined });
const thumbs = await Promise.all(list.map(async (x) => {
  // Straight from the source CDN (Openverse's thumb endpoint would count against the API quota).
  const r = await fetch(x.url, { headers: { 'user-agent': 'Mozilla/5.0 Pinselo' } }).catch(() => null);
  return r?.ok ? Buffer.from(await r.arrayBuffer()).toString('base64') : null;
}));
const C = 260, cols = 4;
const cells = list.map((x, i) => `${thumbs[i] ? `<image href="data:image/jpeg;base64,${thumbs[i]}" x="${(i % cols) * C + 5}" y="${Math.floor(i / cols) * C + 5}" width="${C - 10}" height="${C - 40}" preserveAspectRatio="xMidYMid meet"/>` : ''}<text x="${(i % cols) * C + C / 2}" y="${Math.floor(i / cols) * C + C - 18}" font-size="15" font-weight="bold" text-anchor="middle" font-family="sans-serif">#${i + 1}</text>`);
const out = join(paths.data, 'template-previews');
mkdirSync(out, { recursive: true });
const file = join(out, `photos-${q.replace(/\W+/g, '-')}.png`);
writeFileSync(file, new Resvg(`<svg xmlns="http://www.w3.org/2000/svg" width="${cols * C}" height="${Math.ceil(list.length / cols) * C}"><rect width="100%" height="100%" fill="#fff"/>${cells.join('')}</svg>`, { font: { loadSystemFonts: true } }).render().asPng());
list.forEach((x, i) => console.log(`#${i + 1} ${x.id} ${x.width}×${x.height} ${x.source} — ${x.title.slice(0, 60)}`));
console.log(`contact sheet: ${file}`);
