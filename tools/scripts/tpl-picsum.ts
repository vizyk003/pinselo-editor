/** Browse Lorem Picsum photos for templates: `pnpm tpl:picsum [page 1-11]` → contact sheet of 100 photos with ids (use 'picsum:<id>'). */
import { mkdirSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { Resvg } from '@resvg/resvg-js';
import { paths } from '@oc/config/node';

const page = Number(process.argv[2] ?? 1);
const list = (await (await fetch(`https://picsum.photos/v2/list?page=${page}&limit=100`)).json()) as Array<{ id: string; author: string; width: number; height: number }>;
const thumbs = await Promise.all(list.map(async (x) => {
  const r = await fetch(`https://picsum.photos/id/${x.id}/240/160`).catch(() => null);
  return r?.ok ? Buffer.from(await r.arrayBuffer()).toString('base64') : null;
}));
const C = 250, cols = 10;
const cells = list.map((x, i) => `${thumbs[i] ? `<image href="data:image/jpeg;base64,${thumbs[i]}" x="${(i % cols) * C + 5}" y="${Math.floor(i / cols) * (C - 50) + 5}" width="${C - 10}" height="160"/>` : ''}<text x="${(i % cols) * C + C / 2}" y="${Math.floor(i / cols) * (C - 50) + 186}" font-size="16" font-weight="bold" text-anchor="middle" font-family="sans-serif">${x.id}${x.height > x.width ? ' (tall)' : ''}</text>`);
const out = join(paths.data, 'template-previews');
mkdirSync(out, { recursive: true });
const file = join(out, `picsum-${page}.png`);
writeFileSync(file, new Resvg(`<svg xmlns="http://www.w3.org/2000/svg" width="${cols * C}" height="${Math.ceil(list.length / cols) * (C - 50)}"><rect width="100%" height="100%" fill="#fff"/>${cells.join('')}</svg>`, { font: { loadSystemFonts: true } }).render().asPng());
console.log(`page ${page}: ${list.length} photos (ids ${list[0]?.id}–${list.at(-1)?.id}) → ${file}`);
