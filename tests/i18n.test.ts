import { readdirSync, readFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';
import { check, DOMAINS, extract } from '../tools/scripts/i18n-extract';

describe('translation catalogs', () => {
  for (const d of DOMAINS) {
    const { keys } = extract(d);
    const files = readdirSync(d.locales).filter((f) => f.endsWith('.json'));
    it(`${d.name}: extracts strings`, () => expect(keys.size).toBeGreaterThan(d.name === 'web' ? 500 : 20));
    for (const f of files) {
      it(`${d.name}/${f}: placeholders match and nothing is stale`, () => {
        const cat = JSON.parse(readFileSync(join(d.locales, f), 'utf8')) as Record<string, unknown>;
        const r = check(f, cat, keys);
        expect(r.bad).toEqual([]);
        expect(r.stale).toEqual([]);
        // A catalog may lag behind new strings, but not by much.
        expect(r.missing.length / keys.size).toBeLessThan(0.05);
      });
    }
  }
});
