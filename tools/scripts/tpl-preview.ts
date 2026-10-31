/**
 * Render templates to PNG for review: `pnpm tpl:preview [--id a,b] [--category Posters] [--preset ig-post] [--prefix social] [--pages] [--out dir]`.
 * Writes one PNG per page (rendered by the Rust `oc-templates preview`, i.e. oc-render) plus `sheet.png`, a contact
 * sheet of first pages with ids. Fonts come from the configured database and object store, like the render service.
 */
import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { Resvg } from '@resvg/resvg-js';
import { paths } from '@oc/config/node';
import { auditDesign } from '@oc/engine';
import { runRust } from './rust';
import { buildTemplates } from './templates';
import { writeBundle } from './templates/bundle';

const arg = (k: string) => { const i = process.argv.indexOf(`--${k}`); return i > 0 ? process.argv[i + 1] : undefined; };
const ids = arg('id')?.split(',');
const out = arg('out') ?? join(paths.data, 'template-previews');
mkdirSync(out, { recursive: true });
const all = await buildTemplates((p) => (!ids || ids.includes(p.id)) && (!arg('category') || p.category === arg('category')) && (!arg('preset') || p.preset === arg('preset')) && (!arg('prefix') || p.id.startsWith(arg('prefix')!)));
for (const t of all) {
  // Same checks as the editor's accessibility checker (alt text on photos is set automatically).
  const issues = auditDesign({ ...t.doc, meta: { ...t.doc.meta, lang: t.doc.meta.lang || 'en' } }).filter((i) => i.code === 'low-contrast' || i.code === 'small-text');
  for (const i of issues) {
    const el = i.id ? t.doc.elements[i.id] : undefined;
    const txt = el?.type === 'text' ? el.body.paragraphs.map((p) => p.runs.map((r) => r.text).join('')).join(' ').slice(0, 40) : '';
    console.log(`  ⚠ ${t.id}: ${i.code} ${JSON.stringify(i.data)} “${txt}”`);
  }
}
const bundle = writeBundle(join(paths.data, 'preview-bundle'), all.map((t) => ({ id: t.id, title: t.title, category: t.category, preset: t.preset, tags: t.tags, premium: false, doc: t.doc })));
const code = runRust('oc-templates', ['preview', bundle, out, '--width', '900', ...(process.argv.includes('--pages') ? ['--pages'] : [])]);
if (code !== 0) process.exitCode = code;

const tiles = all.flatMap((t) => {
  const file = join(out, `${t.id}.png`);
  if (!existsSync(file)) return [];
  const scale = Math.min(1, 900 / Math.max(t.doc.meta.dims.w, t.doc.meta.dims.h));
  return [{ id: t.id, png: readFileSync(file), w: Math.round(t.doc.meta.dims.w * scale), h: Math.round(t.doc.meta.dims.h * scale) }];
});
// Contact sheet: 4 columns of 300-px cells with ids.
const C = 300, cols = 4, rows = Math.ceil(tiles.length / cols);
const cells = tiles.map((t, i) => {
  const s = Math.min((C - 16) / t.w, (C - 40) / t.h);
  const w = t.w * s, h = t.h * s, x = (i % cols) * C + (C - w) / 2, y = Math.floor(i / cols) * C + 8;
  return `<image href="data:image/png;base64,${t.png.toString('base64')}" x="${x}" y="${y}" width="${w}" height="${h}"/><text x="${(i % cols) * C + C / 2}" y="${Math.floor(i / cols) * C + C - 10}" font-size="13" text-anchor="middle" font-family="sans-serif">${t.id}</text>`;
});
if (tiles.length) {
  const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="${cols * C}" height="${rows * C}"><rect width="100%" height="100%" fill="#e5e7eb"/>${cells.join('')}</svg>`;
  writeFileSync(join(out, 'sheet.png'), new Resvg(svg, { font: { loadSystemFonts: true } }).render().asPng());
}
console.log(`rendered ${tiles.length} templates → ${out}${tiles.length ? ' (sheet.png)' : ''}`);
