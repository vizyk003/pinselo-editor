/** Node-only filesystem layout. Object storage is a local content-addressed directory in dev (S3 in cloud, ADR-0014). */
import { mkdirSync } from 'node:fs';
import { join, resolve } from 'node:path';
import { config } from './index';

const root = resolve(import.meta.dirname, '../../..');
const data = config.dataDir ? resolve(config.dataDir) : join(root, '.data');

export const paths = {
  root,
  data,
  objects: join(data, 'objects'),
  thumbs: join(data, 'thumbs'),
  fonts: join(root, 'fixtures/fonts'),
  emoji: join(root, 'fixtures/emoji'),
  graphics: join(root, 'fixtures/graphics'),
  templatesSeed: join(root, 'fixtures/templates-seed'),
};

for (const d of [paths.objects, paths.thumbs]) mkdirSync(d, { recursive: true });

/** Content-addressed key: a/{sha256[0..2]}/{sha256} (ADR-0014). */
export function objectPath(sha256: string): string {
  return join(paths.objects, 'a', sha256.slice(0, 2), sha256);
}
