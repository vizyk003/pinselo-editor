/**
 * Print colour profiles: downloads the bundled CMYK ICC profiles (ECI, freely redistributable) listed in
 * packages/schema/src/icc-profiles.json into fixtures/icc/, verifying the package zip and the extracted .icc
 * against their pinned sha256. Profiles already present with the right hash are skipped, so a second run does
 * not touch the network. Builtin profiles (sRGB) ship in crates/oc-color/icc/ and are not downloaded.
 * Writes fixtures/icc/manifest.json. OC_ICC_DIR overrides the directory (the API and render service read it too).
 */
import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { existsSync, mkdirSync, mkdtempSync, readFileSync, renameSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';

const ROOT = resolve(import.meta.dirname, '../..');
const OUT = process.env.OC_ICC_DIR ? resolve(process.env.OC_ICC_DIR) : join(ROOT, 'fixtures/icc');
const REGISTRY = join(ROOT, 'crates/oc-color/icc-profiles.json');

interface Profile {
  id: string; name: string; colorSpace: string; conditionId: string; file: string; sha256: string;
  package?: { url: string; sha256: string; member: string };
  builtin?: boolean;
}

const sha256 = (b: Uint8Array) => createHash('sha256').update(b).digest('hex');
const registry = JSON.parse(readFileSync(REGISTRY, 'utf8')) as { default: string; profiles: Profile[] };

mkdirSync(OUT, { recursive: true });
let tmp: string | null = null;
const manifest: Array<{ id: string; file: string; sha256: string; size: number; source: string }> = [];
let downloaded = 0;

try {
  for (const p of registry.profiles) {
    if (p.builtin || !p.package) continue;
    const dest = join(OUT, p.file);
    const have = existsSync(dest) ? readFileSync(dest) : null;
    if (have && sha256(have) === p.sha256) {
      manifest.push({ id: p.id, file: p.file, sha256: p.sha256, size: have.length, source: p.package.url });
      continue;
    }
    tmp ??= mkdtempSync(join(tmpdir(), 'oc-icc-'));
    const res = await fetch(p.package.url, { redirect: 'follow' });
    if (!res.ok) throw new Error(`${p.id}: GET ${p.package.url} → ${res.status}`);
    const zip = new Uint8Array(await res.arrayBuffer());
    const zipSha = sha256(zip);
    if (zipSha !== p.package.sha256) throw new Error(`${p.id}: ${p.package.url} sha256 ${zipSha}, expected ${p.package.sha256}`);
    const zipPath = join(tmp, `${p.id}.zip`);
    writeFileSync(zipPath, zip);
    // `unzip -p` writes just the member to stdout (no path traversal from archive entries).
    const icc = execFileSync('unzip', ['-p', zipPath, p.package.member], { maxBuffer: 64 * 1024 * 1024 });
    const iccSha = sha256(icc);
    if (iccSha !== p.sha256) throw new Error(`${p.id}: ${p.package.member} sha256 ${iccSha}, expected ${p.sha256}`);
    writeFileSync(`${dest}.tmp`, icc);
    renameSync(`${dest}.tmp`, dest);
    manifest.push({ id: p.id, file: p.file, sha256: p.sha256, size: icc.length, source: p.package.url });
    downloaded++;
    console.log(`icc: ${p.id} → ${p.file} (${icc.length} bytes)`);
  }
} finally {
  if (tmp) rmSync(tmp, { recursive: true, force: true });
}

const manifestPath = join(OUT, 'manifest.json');
const body = `${JSON.stringify({ default: registry.default, profiles: manifest }, null, 2)}\n`;
if (!existsSync(manifestPath) || readFileSync(manifestPath, 'utf8') !== body) writeFileSync(manifestPath, body);
console.log(downloaded ? `icc: downloaded ${downloaded}, ${manifest.length} profiles in ${OUT}` : `icc: ${manifest.length} profiles up to date in ${OUT}`);
