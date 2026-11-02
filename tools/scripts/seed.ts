/**
 * Seed: build the template catalogue (tools/scripts/templates, on @oc/engine), then hand it to the Rust
 * `oc-templates seed` (crates/oc-render-svc/src/bin/oc-templates.rs), which validates the documents, uploads the
 * library assets they use (pinned CC0 photos, emoji art), renders the thumbnails with oc-render and upserts the rows.
 * Thumbnails and asset bytes go to the object store (local disk, or S3 / Supabase Storage when OC_S3_BUCKET is set),
 * so a re-run after switching stores fills it.
 *
 *   pnpm seed                       build + seed every template
 *   pnpm seed --only <prefix>       only template ids with that prefix
 *   pnpm seed --fixtures            also rewrite fixtures/templates-seed/<id>.json for templates whose document
 *                                   changed (the test corpus; regenerate the goldens that read it in the same change)
 *
 * Templates build deterministically (ids are seeded per template id), so seeding never changes tracked files unless
 * `--fixtures` is given and a template's source changed.
 */
import { join } from 'node:path';
import { paths } from '@oc/config/node';
import { runRust } from './rust';
import { buildTemplates, CATEGORY } from './templates';
import { writeBundle, writeFixtures } from './templates/bundle';

const only = process.argv.includes('--only') ? process.argv[process.argv.indexOf('--only') + 1] : undefined;

const TEMPLATES = await buildTemplates((p) => !only || p.id.startsWith(only));
const bundle = writeBundle(
  join(paths.data, 'seed-bundle'),
  TEMPLATES.map((t) => ({
    id: t.id, title: t.title, category: CATEGORY[t.preset] ?? t.category, preset: t.preset, tags: t.tags,
    doc: t.doc,
  })),
);
if (process.argv.includes('--fixtures')) {
  const written = writeFixtures(paths.templatesSeed, TEMPLATES);
  console.log(`fixtures: ${written.length ? `rewrote ${written.join(', ')}` : 'unchanged'}`);
}
process.exit(runRust('oc-templates', ['seed', bundle]));
