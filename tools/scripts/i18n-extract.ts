/**
 * i18n string extraction (`pnpm i18n:check`). Two domains:
 * - web: t('…'), tk('…') and tn(n, '…', '…') in apps/web/src, plus the size preset names and groups the UI
 *   translates at render (crates/oc-schema/src/presets.rs) → apps/web/src/i18n/locales/*.json
 * - mail: `.tr("…", …)` / `t("…")` in crates/oc-platform/oc-mail/src → crates/oc-platform/oc-mail/locales/*.json
 *   (compiled into the mailer)
 * Writes each domain's strings.json (the list translators work from) and reports, per locale, missing,
 * stale and placeholder-mismatched entries. `--prune` drops stale keys from catalogs.
 */
import { readdirSync, readFileSync, statSync, writeFileSync } from 'node:fs';
import { join, relative } from 'node:path';

const ROOT = join(import.meta.dirname, '../..');
const PRESETS = join(ROOT, 'crates/oc-schema/src/presets.rs');
// A Rust string literal ("…" with \" \\ \n \u{…} escapes).
const RS = String.raw`"((?:[^"\\]|\\.)*)"`;
export const DOMAINS = [
  {
    name: 'web', src: join(ROOT, 'apps/web/src'), locales: join(ROOT, 'apps/web/src/i18n/locales'), call: String.raw`\bt[k]?`, exts: /\.(tsx?|mts)$/,
    // Shared data shown in the UI (size preset names, groups and Create-dialog categories) is translated at render with t().
    extra: [
      { file: PRESETS, re: new RegExp(String.raw`\bp\(\s*"[^"]*",\s*${RS}`, 'g') }, // SIZE_PRESETS labels
      { file: PRESETS, re: new RegExp(String.raw`\bc\(\s*C::\w+,\s*${RS}`, 'g') }, // PRESET_CATEGORIES labels
      { file: PRESETS, re: new RegExp(String.raw`\bl\(\s*"[^"]*",\s*${RS}`, 'g') }, // POPULAR_LAYOUTS labels
      { file: PRESETS, re: new RegExp(String.raw`\b[A-Z]\w* = ${RS}`, 'g'), within: 'pub enum PresetGroup' }, // PresetGroup names
    ],
  },
  { name: 'mail', src: join(ROOT, 'crates/oc-platform/oc-mail/src'), locales: join(ROOT, 'crates/oc-platform/oc-mail/locales'), call: String.raw`(?:\.tr|\bt)`, exts: /\.rs$/ },
];
type Domain = (typeof DOMAINS)[number];

const files = (d: string, exts: RegExp): string[] => readdirSync(d).flatMap((f) => {
  const p = join(d, f);
  return statSync(p).isDirectory() ? files(p, exts) : exts.test(f) && !/\.test\./.test(f) ? [p] : [];
});

/** Rust source without its `#[cfg(test)]` modules (their strings are test data). */
const rustCode = (src: string) => src.split(/\n#\[cfg\(test\)\]/)[0]!;
const unRust = (s: string) => JSON.parse(`"${s.replace(/\\u\{([0-9a-fA-F]+)\}/g, (_, h: string) => `\\u${h.padStart(4, '0')}`)}"`) as string;

// String literal: '…' "…" or `…` without ${}.
const LIT = String.raw`('(?:[^'\\]|\\.)*'|"(?:[^"\\]|\\.)*"|\x60[^\x60$]*\x60)`;
const TN = new RegExp(String.raw`\btn\(\s*[^,]+,\s*${LIT}\s*,\s*${LIT}`, 'g');
const unquote = (s: string) => (s[0] === '`' ? s.slice(1, -1) : (JSON.parse(s[0] === "'" ? `"${s.slice(1, -1).replace(/\\'/g, "'").replace(/"/g, '\\"')}"` : s) as string));

export function extract(domain: Domain = DOMAINS[0]!): { keys: Map<string, string[]>; plurals: Set<string> } {
  const keys = new Map<string, string[]>();
  const plurals = new Set<string>();
  const add = (k: string, where: string) => keys.set(k, [...(keys.get(k) ?? []), where]);
  for (const x of 'extra' in domain ? domain.extra : []) {
    let src = rustCode(readFileSync(x.file, 'utf8'));
    if ('within' in x && x.within) src = src.slice(src.indexOf(x.within), src.indexOf('}', src.indexOf(x.within)));
    for (const m of src.matchAll(x.re)) add(unRust(m[1]!), relative(ROOT, x.file));
  }
  const rust = domain.exts.test('x.rs');
  const T = new RegExp(String.raw`${domain.call}\(\s*${rust ? RS : LIT}`, 'g');
  for (const f of files(domain.src, domain.exts)) {
    const src = rust ? rustCode(readFileSync(f, 'utf8')) : readFileSync(f, 'utf8');
    const where = relative(domain.src, f);
    for (const m of src.matchAll(T)) add(rust ? unRust(m[1]!) : unquote(m[1]!), where);
    if (rust) continue;
    for (const m of src.matchAll(TN)) { const k = `${unquote(m[1]!)}|${unquote(m[2]!)}`; plurals.add(k); add(k, where); }
  }
  return { keys, plurals };
}

export const placeholders = (s: string) => [...s.matchAll(/\{(\w+)\}/g)].map((m) => m[1]).sort().join(',');

export function check(code: string, cat: Record<string, unknown>, keys: Map<string, string[]>) {
  const missing = [...keys.keys()].filter((k) => !(k in cat));
  const stale = Object.keys(cat).filter((k) => !keys.has(k));
  const bad: string[] = [];
  for (const [k, v] of Object.entries(cat)) {
    if (!keys.has(k)) continue;
    const want = placeholders(k.includes('|') ? k.split('|')[1]! : k).replace(/(^|,)n(,|$)/, '$1$2').replace(/^,|,$/g, '');
    const forms = typeof v === 'string' ? [v] : Object.values(v as Record<string, string>);
    for (const f of forms) {
      const got = placeholders(f).replace(/(^|,)n(,|$)/, '$1$2').replace(/^,|,$/g, '');
      if (got !== want) bad.push(`${k} → ${f}`);
    }
  }
  return { code, missing, stale, bad };
}

if (import.meta.url === `file://${process.argv[1]}`) {
  for (const d of DOMAINS) {
    const { keys } = extract(d);
    writeFileSync(join(d.locales, '..', 'strings.json'), `${JSON.stringify(Object.fromEntries([...keys.keys()].sort().map((k) => [k, keys.get(k)![0]])), null, 1)}\n`);
    console.log(`[${d.name}] ${keys.size} strings`);
    for (const f of readdirSync(d.locales).filter((x) => x.endsWith('.json'))) {
      const cat = JSON.parse(readFileSync(join(d.locales, f), 'utf8')) as Record<string, unknown>;
      const r = check(f.replace('.json', ''), cat, keys);
      console.log(`  ${r.code}: ${keys.size - r.missing.length}/${keys.size} translated, ${r.stale.length} stale, ${r.bad.length} placeholder mismatches`);
      for (const b of r.bad.slice(0, 10)) console.log(`     ✗ ${b}`);
      if (process.argv.includes('--prune') && r.stale.length) {
        for (const k of r.stale) delete cat[k];
        writeFileSync(join(d.locales, f), `${JSON.stringify(cat, null, 1)}\n`);
      }
    }
  }
}
