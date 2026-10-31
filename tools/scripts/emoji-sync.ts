/**
 * Emoji stickers (EL-020): Noto Emoji SVGs (Apache-2.0, via @svgmoji/noto's sprite) split into one file per
 * emoji, with names, groups and search tags from emojibase-data (MIT). Writes fixtures/emoji/.
 */
import { execFileSync } from 'node:child_process';
import { cpSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';

const ROOT = resolve(import.meta.dirname, '../..');
const OUT = join(ROOT, 'fixtures/emoji');
const tmp = mkdtempSync(join(tmpdir(), 'oc-emoji-'));

function unpack(pkg: string): string {
  const dir = join(tmp, pkg.replace(/[@/]/g, '_'));
  mkdirSync(dir, { recursive: true });
  const tgz = execFileSync('npm', ['pack', pkg, '--silent', '--pack-destination', dir], { encoding: 'utf8' }).trim().split('\n').pop()!;
  execFileSync('tar', ['xzf', join(dir, tgz), '-C', dir]);
  return join(dir, 'package');
}

const noto = unpack('@svgmoji/noto@3.2.0');
const base = unpack('emojibase-data@17.0.0');

// Split the sprite: top-level <svg ... id="HEX"> children of the root <svg>.
const sprite = readFileSync(join(noto, 'sprites/all.svg'), 'utf8');
const svgs = new Map<string, string>();
// Attribute order varies between entries, so match the tag and read id/viewBox separately.
const re = /<svg\b([^>]*\bid="([0-9A-F-]+)"[^>]*)>/g;
let m: RegExpExecArray | null;
const starts: Array<{ id: string; viewBox: string; at: number; bodyAt: number }> = [];
while ((m = re.exec(sprite))) {
  const viewBox = /viewBox="([^"]+)"/.exec(m[1]!)?.[1] ?? '0 0 128 128';
  starts.push({ id: m[2]!, viewBox, at: m.index, bodyAt: m.index + m[0].length });
}
for (const [i, s] of starts.entries()) {
  const end = i + 1 < starts.length ? starts[i + 1]!.at : sprite.lastIndexOf('</svg>');
  const inner = sprite.slice(s.bodyAt, end).replace(/<\/svg>\s*$/, '');
  svgs.set(s.id, `<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" viewBox="${s.viewBox}">${inner}</svg>`);
}

interface Entry { hexcode: string; label: string; group?: number; order?: number; tags?: string[] }
const data = JSON.parse(readFileSync(join(base, 'en/compact.json'), 'utf8')) as Entry[];
const groups = (JSON.parse(readFileSync(join(base, 'meta/groups.json'), 'utf8')) as { groups: Record<string, string> }).groups;

rmSync(OUT, { recursive: true, force: true });
mkdirSync(join(OUT, 'svg'), { recursive: true });
const index: Array<{ h: string; l: string; g: string; t: string[] }> = [];
for (const e of data.sort((a, b) => (a.order ?? 1e9) - (b.order ?? 1e9))) {
  if (e.group === undefined || groups[e.group] === 'component') continue;
  const candidates = [e.hexcode, e.hexcode.replace(/-FE0F/g, ''), `${e.hexcode}-FE0F`];
  const id = candidates.find((c) => svgs.has(c));
  if (!id) continue;
  writeFileSync(join(OUT, 'svg', `${e.hexcode}.svg`), svgs.get(id)!);
  index.push({ h: e.hexcode, l: e.label, g: groups[e.group]!, t: (e.tags ?? []).slice(0, 8) });
}
writeFileSync(join(OUT, 'index.json'), JSON.stringify(index));
cpSync(join(noto, 'LICENSE'), join(OUT, 'LICENSE-svgmoji.txt'));
writeFileSync(join(OUT, 'NOTICE.txt'), 'Emoji artwork: Noto Emoji by Google, Apache License 2.0 (https://github.com/googlefonts/noto-emoji).\nMetadata: emojibase-data, MIT.\n');
rmSync(tmp, { recursive: true, force: true });
console.log(`Wrote ${index.length} emoji to fixtures/emoji`);
