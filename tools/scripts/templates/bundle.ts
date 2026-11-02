/**
 * The hand-off from the TS template builder to the Rust `oc-templates` binary (crates/oc-render-svc/src/bin):
 * `templates.json` (the built documents with their catalogue fields) and `assets.json` (the library assets they
 * reference, each with the file that holds its bytes). See that binary for the format.
 */
import { readFileSync, existsSync, mkdirSync, rmSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { LIB_ASSETS, type TemplateDef } from './builder';

export interface BundledTemplate { id: string; title: string; category: string; preset: string; tags: string[]; doc: TemplateDef['doc'] }

/** Write a bundle into `dir` (replacing it) and return `dir`. */
export function writeBundle(dir: string, templates: BundledTemplate[]): string {
  rmSync(dir, { recursive: true, force: true });
  mkdirSync(join(dir, 'assets'), { recursive: true });
  const assets = [...LIB_ASSETS].map(([id, a]) => {
    let file = a.file;
    if (!file) {
      file = join(dir, 'assets', id);
      writeFileSync(file, a.bytes());
    }
    return { id, sha256: a.ref.sha256, mime: a.ref.mime, w: a.ref.w, h: a.ref.h, name: a.ref.name ?? null, source: a.source, attribution: a.attribution ?? null, file };
  });
  writeFileSync(join(dir, 'templates.json'), JSON.stringify(templates));
  writeFileSync(join(dir, 'assets.json'), JSON.stringify(assets));
  return dir;
}

/**
 * Write `fixtures/templates-seed/<id>.json` for each template whose document changed (the fixtures are a test corpus:
 * the Rust and engine goldens read them). Returns the ids written.
 */
export function writeFixtures(dir: string, templates: Array<Pick<TemplateDef, 'id' | 'doc'>>): string[] {
  mkdirSync(dir, { recursive: true });
  const written: string[] = [];
  for (const t of templates) {
    const file = join(dir, `${t.id}.json`);
    const text = `${JSON.stringify(t.doc)}\n`;
    if (existsSync(file) && readFileSync(file, 'utf8') === text) continue;
    writeFileSync(file, text);
    written.push(t.id);
  }
  return written;
}
