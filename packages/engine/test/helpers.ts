// SPDX-License-Identifier: Apache-2.0
/** Shared test helpers: golden loading, the goldens' tolerant comparison, the deterministic text service. */
import { readFileSync, readdirSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import type { TextEl, TextService } from '../src/index';

export const ROOT = join(dirname(fileURLToPath(import.meta.url)), '../../..');

export const readJson = <T = any>(...parts: string[]): T => JSON.parse(readFileSync(join(ROOT, ...parts), 'utf8')) as T;

export const golden = <T = any>(dir: string, name: string): T => readJson<T>('tests/golden', dir, name.endsWith('.json') ? name : `${name}.json`);

export const fixtureNames = (): string[] => readdirSync(join(ROOT, 'fixtures/templates-seed')).filter((f) => f.endsWith('.json')).sort();

const fixtures = new Map<string, any>();
export const fixture = (name: string): any => {
  if (!fixtures.has(name)) fixtures.set(name, readJson('fixtures/templates-seed', name));
  return fixtures.get(name);
};

export const unb64 = (s: string): Uint8Array => new Uint8Array(Buffer.from(s, 'base64'));

/** The goldens' `"__undefined__"` / `"__num__NaN"` markers. */
const UNDEF = '__undefined__';

/**
 * Structural comparison like the Rust golden tests: objects order-insensitive, numbers within 1e-9 (relative above
 * 1), `undefined` ≡ missing, non-finite markers ≡ `null`. Returns the first difference or `null`.
 */
export function diff(actual: unknown, expected: unknown, path = '$'): string | null {
  if (expected === UNDEF) expected = undefined;
  if (typeof expected === 'string' && expected.startsWith('__num__')) {
    const want = expected === '__num__NaN' ? NaN : expected === '__num__Infinity' ? Infinity : -Infinity;
    if (actual === null || Object.is(actual, want)) return null;
    return `${path}: ${String(actual)} != ${expected}`;
  }
  if (typeof actual === 'number' && typeof expected === 'number') {
    const tol = 1e-9 * Math.max(1, Math.abs(actual), Math.abs(expected));
    return Math.abs(actual - expected) <= tol || (Number.isNaN(actual) && Number.isNaN(expected)) ? null : `${path}: ${actual} != ${expected} (expected)`;
  }
  if (Array.isArray(actual) && Array.isArray(expected)) {
    if (actual.length !== expected.length) return `${path}: length ${actual.length} != ${expected.length}\n  actual:   ${short(actual)}\n  expected: ${short(expected)}`;
    for (let i = 0; i < actual.length; i++) {
      const d = diff(actual[i], expected[i], `${path}[${i}]`);
      if (d) return d;
    }
    return null;
  }
  if (actual && expected && typeof actual === 'object' && typeof expected === 'object' && !Array.isArray(actual) && !Array.isArray(expected)) {
    const a = actual as Record<string, unknown>;
    const e = expected as Record<string, unknown>;
    const keys = new Set([...Object.keys(a).filter((k) => a[k] !== undefined), ...Object.keys(e).filter((k) => e[k] !== undefined && e[k] !== UNDEF)]);
    for (const k of keys) {
      if (!(k in e) || e[k] === undefined || e[k] === UNDEF) return `${path}.${k}: unexpected ${short(a[k])}`;
      if (!(k in a) || a[k] === undefined) return `${path}.${k}: missing (expected ${short(e[k])})`;
      const d = diff(a[k], e[k], `${path}.${k}`);
      if (d) return d;
    }
    return null;
  }
  // `undefined` results were recorded as `null`.
  if (actual == null && expected == null) return null;
  return Object.is(actual, expected) ? null : `${path}: ${short(actual)} != ${short(expected)} (expected)`;
}

export function short(v: unknown): string {
  const s = JSON.stringify(v) ?? String(v);
  return s.length > 400 ? `${s.slice(0, 400)}…` : s;
}

/** JSON round trip (what the goldens recorded). */
export const json = <T>(v: T): T => (v === undefined ? (null as T) : (JSON.parse(JSON.stringify(v)) as T));

/**
 * Documented deviation (crates/oc-editor-core): turning a list off writes `list: null` where TS removes the
 * paragraph attribute. Paragraph objects compare with a `null` list equal to none (same as the Rust replay).
 */
export function withoutNullList<T>(v: T): T {
  if (Array.isArray(v)) v.forEach(withoutNullList);
  else if (v && typeof v === 'object') {
    const m = v as Record<string, unknown>;
    const paragraph = 'runs' in m || ('align' in m && 'lineHeight' in m && 'spaceAfter' in m);
    if (paragraph && m.list === null) delete m.list;
    Object.values(m).forEach(withoutNullList);
  }
  return v;
}

const lines = (el: TextEl) => el.body.paragraphs.map((p) => p.runs.map((r) => r.text).join(''));

/** tools/golden/editor.ts `goldText`: the deterministic text service the editor goldens were recorded with. */
export const goldText: TextService = {
  height: (el, w) => lines(el).reduce((h, l) => h + Math.max(1, Math.ceil((l.length * 10) / Math.max(1, w))) * 20, 0),
  offsetAt: (el, x, y) => {
    const ls = lines(el);
    if (!ls.length) return 0;
    const li = Math.max(0, Math.min(ls.length - 1, Math.floor(y / 20)));
    let off = 0;
    for (let i = 0; i < li; i++) off += ls[i]!.length + 1;
    return off + Math.max(0, Math.min(ls[li]!.length, Math.round(x / 10)));
  },
  verticalMove: (el, o, dir) => {
    const ls = lines(el);
    if (!ls.length) return 0;
    let li = 0;
    let start = 0;
    while (li < ls.length - 1 && o > start + ls[li]!.length) {
      start += ls[li]!.length + 1;
      li++;
    }
    const col = Math.max(0, o - start);
    const ti = li + dir;
    if (ti < 0) return 0;
    if (ti >= ls.length) return ls.join('\n').length;
    let ts = 0;
    for (let i = 0; i < ti; i++) ts += ls[i]!.length + 1;
    return ts + Math.min(col, ls[ti]!.length);
  },
  wordAt: (el, o) => {
    const t = lines(el).join('\n');
    const c = Math.max(0, Math.min(t.length, o));
    let a = c;
    let b = c;
    while (a > 0 && /\w/.test(t[a - 1]!)) a--;
    while (b < t.length && /\w/.test(t[b]!)) b++;
    return [a, b];
  },
  length: (el) => lines(el).join('\n').length,
  curvedSize: (el) => ({ w: Math.max(40, lines(el).join('\n').length * 12), h: 60 + Math.round(Math.abs(el.curve ?? 0) / 5) }),
};
