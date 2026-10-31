/**
 * Writes fixtures/fonts/google-catalog.json: every Google Fonts family with its faces, category and
 * popularity, from Google's public catalogue metadata. Faces are fetched on demand later (crates/oc-platform/oc-gfonts,
 * whose `catalog_from_metadata` is the same mapping). Refreshing it moves the goldens that depend on it.
 */
import { writeFileSync } from 'node:fs';
import { join, resolve } from 'node:path';

/** Compact catalogue row: faces are '400', '400i', '700', … */
interface CatalogFamily { family: string; slug: string; category: string; faces: string[]; popularity: number; subsets: string[] }
interface Metadata { familyMetadataList: Array<{ family: string; category: string; fonts: Record<string, unknown>; popularity: number; subsets: string[] }> }

const CATEGORY: Record<string, string> = { 'Sans Serif': 'sans', Serif: 'serif', Display: 'display', Handwriting: 'handwriting', Monospace: 'mono' };
const slugOf = (family: string) => family.toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-|-$/g, '');

function catalogFromMetadata(meta: Metadata): CatalogFamily[] {
  return meta.familyMetadataList
    .filter((f) => Object.keys(f.fonts).length)
    .map((f) => ({ family: f.family, slug: slugOf(f.family), category: CATEGORY[f.category] ?? 'display', faces: Object.keys(f.fonts).filter((k) => /^\d{3}i?$/.test(k)).sort(), popularity: f.popularity, subsets: f.subsets.filter((s) => s !== 'menu') }))
    .filter((f) => f.faces.length)
    .sort((a, b) => a.popularity - b.popularity);
}

const ROOT = resolve(import.meta.dirname, '../..');
const r = await fetch('https://fonts.google.com/metadata/fonts', { signal: AbortSignal.timeout(60_000) });
if (!r.ok) throw new Error(`Google Fonts metadata: HTTP ${r.status}`);
const text = await r.text();
const list = catalogFromMetadata(JSON.parse(text.slice(text.indexOf('{'))) as Metadata);
writeFileSync(join(ROOT, 'fixtures/fonts/google-catalog.json'), `${JSON.stringify(list)}\n`);
console.log(`Wrote ${list.length} Google Fonts families (${list.reduce((n, f) => n + f.faces.length, 0)} faces) to fixtures/fonts/google-catalog.json`);
