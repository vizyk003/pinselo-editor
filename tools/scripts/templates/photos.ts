/**
 * Template photography: CC0 photos from Openverse (StockSnap and other CC0 sources), pinned by id in
 * photos.json so every build uses the same pictures. Files are cached in fixtures/template-photos/.
 *
 *   pnpm tpl:photos "coffee shop" [n]   → contact sheet PNG of candidates (with ids) to choose from
 *   b.photo('<openverse id>', x, y, w, h) in a template pins and uses it
 *   pnpm tpl:picsum [page]               → contact sheet of Lorem Picsum photos (Unsplash License), use 'picsum:<n>'
 */
import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { join, resolve } from 'node:path';

const ROOT = resolve(import.meta.dirname, '../../..');
export const PHOTO_DIR = join(ROOT, 'fixtures/template-photos');
const PINS = join(import.meta.dirname, 'photos.json');
const SEEN = join(PHOTO_DIR, 'search-cache.json');
const API = 'https://api.openverse.org/v1/images';

export interface PinnedPhoto { id: string; url: string; width: number; height: number; title: string; creator?: string; creatorUrl?: string; license: string; licenseUrl?: string; landing?: string; source: string }
interface OvImage { id: string; url: string; width: number; height: number; title: string; creator?: string; creator_url?: string; license: string; license_url?: string; foreign_landing_url?: string; source: string; thumbnail?: string }

const pins = (): Record<string, PinnedPhoto> => (existsSync(PINS) ? JSON.parse(readFileSync(PINS, 'utf8')) : {});
const toPin = (x: OvImage): PinnedPhoto => ({ id: x.id, url: x.url, width: x.width, height: x.height, title: x.title, creator: x.creator, creatorUrl: x.creator_url, license: x.license, licenseUrl: x.license_url, landing: x.foreign_landing_url, source: x.source });

export async function searchPhotos(q: string, n = 12, opts: { orientation?: 'tall' | 'wide' | 'square' } = {}): Promise<OvImage[]> {
  const u = new URL(`${API}/`);
  u.search = new URLSearchParams({ q, license: 'cc0', category: 'photograph', page_size: '20', mature: 'false', ...(opts.orientation ? { aspect_ratio: opts.orientation } : {}) }).toString();
  const r = await fetch(u, { headers: { 'user-agent': 'Pinselo template tooling' } });
  if (!r.ok) throw new Error(`Openverse ${r.status}`);
  const raw = ((await r.json()) as { results: OvImage[] }).results;
  // Remember results so pinning a chosen photo needs no further API call (anonymous quota: 200/day).
  mkdirSync(PHOTO_DIR, { recursive: true });
  const cache = existsSync(SEEN) ? (JSON.parse(readFileSync(SEEN, 'utf8')) as Record<string, OvImage>) : {};
  for (const x of raw) cache[x.id] = x;
  writeFileSync(SEEN, JSON.stringify(cache));
  const all = raw.filter((x) => x.width >= 1200 && x.height >= 800 || x.height >= 1200 && x.width >= 800);
  // Prefer curated modern stock sources.
  return all.sort((a, b) => Number(b.source === 'stocksnap') - Number(a.source === 'stocksnap')).slice(0, n);
}

async function pin(id: string): Promise<PinnedPhoto> {
  const all = pins();
  if (all[id]) return all[id]!;
  const seen = existsSync(SEEN) ? (JSON.parse(readFileSync(SEEN, 'utf8')) as Record<string, OvImage>)[id] : undefined;
  let x = seen;
  if (!x) {
    const r = await fetch(`${API}/${id}/`, { headers: { 'user-agent': 'Pinselo template tooling' } });
    if (!r.ok) throw new Error(`Openverse photo ${id}: ${r.status}`);
    x = (await r.json()) as OvImage;
  }
  if (x.license !== 'cc0' && x.license !== 'pdm') throw new Error(`photo ${id} is ${x.license}, not CC0`);
  const p = toPin(x);
  writeFileSync(PINS, `${JSON.stringify({ ...pins(), [id]: p }, null, 1)}\n`);
  return p;
}

export interface PhotoFile { id: string; file: string; sha256: string; w: number; h: number; pin: PinnedPhoto }
const ready = new Map<string, PhotoFile>();

/** Lorem Picsum (Unsplash photos, Unsplash License): ids look like `picsum:1015`. */
async function pinPicsum(id: string): Promise<PinnedPhoto> {
  const all = pins();
  if (all[id]) return all[id]!;
  const n = id.slice('picsum:'.length);
  const r = await fetch(`https://picsum.photos/id/${n}/info`);
  if (!r.ok) throw new Error(`picsum ${n}: ${r.status}`);
  const x = (await r.json()) as { id: string; author: string; width: number; height: number; url: string };
  const w = Math.min(2000, x.width);
  const h = Math.round((w * x.height) / x.width);
  const p: PinnedPhoto = { id, url: `https://picsum.photos/id/${n}/${w}/${h}`, width: x.width, height: x.height, title: `Photo by ${x.author}`, creator: x.author, license: 'unsplash', licenseUrl: 'https://unsplash.com/license', landing: x.url, source: 'unsplash' };
  writeFileSync(PINS, `${JSON.stringify({ ...pins(), [id]: p }, null, 1)}\n`);
  return p;
}

/** Download (once) and normalise to a ≤ 2000 px JPEG. */
export async function preparePhoto(id: string): Promise<PhotoFile> {
  if (ready.has(id)) return ready.get(id)!;
  const p = id.startsWith('picsum:') ? await pinPicsum(id) : await pin(id);
  mkdirSync(PHOTO_DIR, { recursive: true });
  const file = join(PHOTO_DIR, `${id.replace(':', '-')}.jpg`);
  if (!existsSync(file)) {
    const r = await fetch(p.url, { headers: { 'user-agent': 'Mozilla/5.0 Pinselo' }, redirect: 'follow' });
    if (!r.ok) throw new Error(`download ${id}: ${r.status}`);
    const raw = join(PHOTO_DIR, `${id.replace(':', '-')}.src`);
    writeFileSync(raw, Buffer.from(await r.arrayBuffer()));
    execFileSync('ffmpeg', ['-loglevel', 'error', '-y', '-i', raw, '-vf', "scale='min(2000,iw)':-2", '-q:v', '3', file]);
    execFileSync('rm', ['-f', raw]);
  }
  const dims = execFileSync('ffprobe', ['-v', 'error', '-select_streams', 'v:0', '-show_entries', 'stream=width,height', '-of', 'csv=p=0', file]).toString().trim().split(',').map(Number);
  const bytes = readFileSync(file);
  const out = { id, file, sha256: createHash('sha256').update(bytes).digest('hex'), w: dims[0]!, h: dims[1]!, pin: p };
  ready.set(id, out);
  return out;
}
export const preparedPhoto = (id: string) => ready.get(id);

/** Every photo id referenced by template sources (`photo('…'` / `bgPhoto('…'`), so they can be fetched before building. */
export function referencedPhotoIds(dir = import.meta.dirname): string[] {
  const ids = new Set<string>();
  for (const f of execFileSync('ls', [dir]).toString().split('\n').filter((x) => x.endsWith('.ts'))) {
    for (const m of readFileSync(join(dir, f), 'utf8').matchAll(/\b(?:photo|bgPhoto|gridPhotos?)\(\s*'([0-9a-f-]{36})'/g)) ids.add(m[1]!);
    for (const m of readFileSync(join(dir, f), 'utf8').matchAll(/'([0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12})'/g)) ids.add(m[1]!);
    for (const m of readFileSync(join(dir, f), 'utf8').matchAll(/'(picsum:\d+)'/g)) ids.add(m[1]!);
  }
  return [...ids];
}
