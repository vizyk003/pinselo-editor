// SPDX-License-Identifier: Apache-2.0
/**
 * The pure helpers through the facade against the goldens generated from the TS packages (the same files the Rust
 * crates replay natively): tests/golden/editor/modules.json (@oc/editor-core), tests/golden/schema/*.json
 * (@oc/schema) and tests/golden/geometry/*.json (@oc/geometry). Only the exports apps/web uses are part of the
 * facade; golden cases of other functions are counted as "not exposed".
 */
import * as Y from 'yjs';
import { describe, expect, test } from 'vitest';
import * as E from '../src/index';
import { diff, fixture, fixtureNames, golden, short } from './helpers';

type Case = { fn: string; args: any[]; out?: unknown; error?: string };
type Run = (fn: string, a: any[]) => unknown;

const SKIP = Symbol('not exposed');
const report: string[] = [];

/** The goldens encode non-finite numbers as `"__num__NaN"` / `"__num__Infinity"` / `"__num__-Infinity"`. */
function numbers(v: any): any {
  if (typeof v === 'string' && v.startsWith('__num__')) return v === '__num__NaN' ? NaN : v === '__num__Infinity' ? Infinity : -Infinity;
  if (Array.isArray(v)) return v.map(numbers);
  if (v && typeof v === 'object') return Object.fromEntries(Object.entries(v).map(([k, x]) => [k, numbers(x)]));
  return v;
}

/** Replays `{ fn, args, out | error }` cases; `run` returns `SKIP` for functions the facade does not export. */
function replay(file: string, cases: Case[], run: Run, cmp: (fn: string, actual: unknown, want: unknown) => string | null = (_f, a, w) => diff(a, w)) {
  const failures: string[] = [];
  let ran = 0;
  let skipped = 0;
  cases.forEach((c, i) => {
    let out: unknown;
    try {
      out = run(c.fn, numbers(c.args));
    } catch (e) {
      if (c.error === undefined) failures.push(`${file}#${i} ${c.fn}(${short(c.args)}): threw ${String(e)}`);
      else if (!String((e as Error).message ?? e).includes(c.error) && !c.error.startsWith('Cannot ')) failures.push(`${file}#${i} ${c.fn}: error ${String(e)} != ${c.error}`);
      ran++;
      return;
    }
    if (out === SKIP) {
      skipped++;
      return;
    }
    ran++;
    if (c.error !== undefined && c.out === undefined) {
      failures.push(`${file}#${i} ${c.fn}(${short(c.args)}): expected error ${c.error}, got ${short(out)}`);
      return;
    }
    const d = cmp(c.fn, JSON.parse(JSON.stringify(out ?? null)), c.out);
    if (d) failures.push(`${file}#${i} ${c.fn}(${short(c.args)}): ${d}`);
  });
  report.push(`${file}: ${ran} cases through the facade${skipped ? `, ${skipped} not exposed` : ''}`);
  expect(failures.slice(0, 20), `${failures.length} of ${ran} failing`).toEqual([]);
  expect(ran).toBeGreaterThan(0);
}

/** Replace a top-level random `id` (factories) with `"<id>"`. */
const normId = (v: any) => (v && typeof v === 'object' && typeof v.id === 'string' ? { ...v, id: '<id>' } : v);

/** `designColors`: equal counts may come in any order (documented deviation, see crates/oc-editor-core). */
function sameColors(actual: any, want: any): string | null {
  const runs = (v: Array<{ hex: string; count: number }>) => {
    const out: Array<[number, string[]]> = [];
    for (const c of v) {
      const last = out[out.length - 1];
      if (last && last[0] === c.count) last[1].push(c.hex);
      else out.push([c.count, [c.hex]]);
    }
    return out.map(([n, hs]) => [n, hs.sort()]);
  };
  return diff(runs(actual), runs(want));
}

describe('@oc/editor-core helpers (modules.json)', () => {
  test('modules', () => {
    const { cases } = golden('editor', 'modules.json');
    replay('editor/modules', cases, (fn, a) => {
      const doc = () => fixture(a[0]);
      switch (fn) {
        case 'designColors': return E.designColors(doc());
        case 'auditDesign': return E.auditDesign(doc());
        case 'describeElement': {
          const d = doc();
          return Object.keys(d.elements).sort().map((id) => E.describeElement(d, d.elements[id]));
        }
        case 'fontSetOps': return E.fontSetOps(doc(), a[1]);
        case 'pageSlots': return E.pageSlots(doc());
        case 'screenToScene': return E.screenToScene(a[0], a[1]);
        case 'templatePages': {
          // The golden draws ids from Mulberry32(seed): an editor seeded the same way supplies that stream.
          const ed = new E.Editor(new Y.Doc(), undefined, { seed: a[2], clock: 'manual' });
          const r = ed.templatePages(doc(), a[1]);
          ed.destroy();
          return r;
        }
        case 'coverCrop': return [[1, 2], [2, 1], [1, 1], [0.5, 0.25]].map(([x, y]) => E.coverCrop(x!, y!));
        case 'refitCrop': return E.refitCrop(a[0], a[1], a[2]);
        default: return SKIP;
      }
    }, (fn, actual, want) => {
      if (fn === 'designColors') return sameColors(actual, want);
      return diff(actual, want);
    });
  });
});

describe('@oc/schema helpers', () => {
  test('constants', () => {
    const g = golden('schema', 'constants');
    const names = ['ANIM_PRESETS', 'BORDER_PRESETS', 'CALLOUT_TONES', 'CHART_COLORS', 'CHART_KINDS', 'CURRENT_COLOR', 'DEFAULT_PRINT_CATALOG', 'DOC_CONTENT_WIDTH',
      'EMBED_FRAME_HOSTS', 'EMPHASIS_PRESETS', 'FILTER_PRESETS', 'FRAME_CATEGORIES', 'FRAME_LIBRARY', 'GRID_LAYOUTS', 'KEYFRAME_NEUTRAL', 'KEYFRAME_PROPS', 'LINE_MARKERS',
      'POPULAR_LAYOUTS', 'PRESET_CATEGORIES', 'SHAPE_KINDS', 'SIZE_PRESETS', 'TABLE_STYLE_PRESETS', 'TRANSITION_KINDS'] as const;
    for (const n of names) expect(diff(JSON.parse(JSON.stringify((E as any)[n])), g[n], n), n).toBeNull();
    expect(Object.isFrozen(E.SIZE_PRESETS)).toBe(true);
    report.push(`schema/constants: ${names.length} constants`);
  });

  test('color', () => replay('schema/color', golden('schema', 'color'), (fn, a) => {
    switch (fn) {
      case 'hex': return E.hex(a[0]);
      case 'toHex': return E.toHex(a[0], a[1]);
      case 'toCss': return E.toCss(a[0]);
      case 'solid': return E.solid(a[0]);
      default: return SKIP;
    }
  }));

  test('presets', () => {
    const g = golden('schema', 'presets');
    replay('schema/presets', g.cases, (fn, a) => {
      const ids = (l: Array<{ id: string }>) => l.map((p) => p.id);
      switch (fn) {
        case 'searchPresets': {
          const dict = a[1] === undefined ? undefined : g.translations[a[1]];
          return ids(a[1] === undefined ? E.searchPresets(a[0]) : E.searchPresets(a[0], (k: string) => dict?.[k] ?? k));
        }
        case 'findPreset': return E.findPreset(a[0] === '__undefined__' ? undefined : a[0]);
        case 'presetsInCategory': return ids(E.presetsInCategory(a[0]));
        case 'presetPlatforms': return E.presetPlatforms(a[0].map((id: string) => E.findPreset(id)!));
        case 'popularPresets': return ids(E.popularPresets());
        case 'tryNewPresets': return ids(E.tryNewPresets(a[0], a[1]));
        case 'pushRecentPreset': return E.pushRecentPreset(a[0], a[1], a[2]);
        case 'lockedSide': return E.lockedSide(a[0], a[1], a[2], a[3]);
        default: return SKIP;
      }
    });
  });

  test('units and keyframes', () => replay('schema/presets_keyframes', golden('schema', 'presets_keyframes'), (fn, a) => {
    switch (fn) {
      case 'pxToUnit': return E.pxToUnit(a[0], a[1]);
      case 'unitToPx': return E.unitToPx(a[0], a[1]);
      case 'formatDims': return E.formatDims(a[0], a[1], a[2]);
      case 'keyframeValue': return E.keyframeValue(a[0] === '__undefined__' ? undefined : a[0], a[1], a[2]);
      default: return SKIP;
    }
  }));

  test('tree', () => {
    const g = golden('schema', 'tree');
    const docOf = (n: string) => g.docs[n] ?? fixture(n);
    replay('schema/tree', g.cases, (fn, a) => {
      const d = docOf(a[0]);
      switch (fn) {
        case 'sortedPages': return E.sortedPages(d).map((p) => p.id);
        case 'childrenOf': return E.childrenOf(d, a[1]).map((e) => e.id);
        case 'descendants': return E.descendants(d, a[1]).map((e) => e.id);
        case 'pageOf': return E.pageOf(d, a[1]);
        default: return SKIP;
      }
    }, (_fn, actual, want) => diff(actual, want));
  });

  test('timeline', () => {
    const g = golden('schema', 'timeline');
    const d = g.doc;
    const span = (s: any) => ({ pageId: s.page.id, index: s.index, startMs: s.startMs, durationMs: s.durationMs });
    replay('schema/timeline', g.cases, (fn, a) => {
      switch (fn) {
        case 'pageDurationMs': return E.pageDurationMs(d, d.pages[a[0]]);
        case 'timelineOf': {
          const t = E.timelineOf(d, a[0]);
          return { spans: t.spans.map(span), totalMs: t.totalMs };
        }
        case 'locateTime': {
          const r = E.locateTime(d, a[0]);
          return r ? { span: span(r.span), pageTimeMs: r.pageTimeMs } : null;
        }
        case 'elementWindow': return E.elementWindow(d.elements[a[0]], a[1]);
        case 'videoSlots': return E.videoSlots(d.elements[a[0]]);
        case 'mediaSourceTime': {
          const el = d.elements[a[0]];
          const slot = E.videoSlots(el).find((x) => x.key === a[1])!;
          return E.mediaSourceTime(el, slot.clip, a[2], a[3]);
        }
        default: return SKIP;
      }
    });
  });

  test('table', () => {
    const g = golden('schema', 'table');
    replay('schema/table', g.cases, (fn, a) => {
      const t = () => g.tables[a[0]];
      switch (fn) {
        case 'tableGeometry': return E.tableGeometry(t());
        case 'cellRect': return E.cellRect(t(), a[1], a[2]);
        case 'canUnmerge': return E.canUnmerge(t(), a[1]);
        case 'tableStyleSwatch': return E.tableStyleSwatch(a[0]);
        default: return SKIP;
      }
    });
  });

  test('factory', () => replay('schema/factory', golden('schema', 'factory'), (fn, a) => {
    const init = () => ({ ...a[0], ...(a[1] ?? {}) });
    switch (fn) {
      case 'makeText': return normId(E.makeText(init()));
      case 'makeShape': return normId(E.makeShape(init()));
      case 'makeLine': return normId(E.makeLine(init()));
      case 'makeImage': return normId(E.makeImage({ ...a[0], asset: a[1] }));
      case 'makeFrame': return normId(E.makeFrame(init()));
      case 'makeGrid': return normId(E.makeGrid(init()));
      case 'makeQr': return normId(E.makeQr({ ...a[0], data: a[1] }));
      case 'makeTable': return normId(E.makeTable(init()));
      case 'makeChart': return normId(E.makeChart(init()));
      case 'makeVideo': return normId(E.makeVideo({ ...a[0], asset: a[1], durationMs: a[2] }));
      case 'makeForm': return normId(E.makeForm(init()));
      case 'makeEmbed': return normId(E.makeEmbed(init()));
      case 'makeStroke': return E.makeStroke(a[0] ?? undefined, a[1] ?? undefined, a[2] ?? undefined);
      case 'clipLengthMs': return E.clipLengthMs(a[0]);
      case 'formLayout': return E.formLayout(a[0]);
      case 'emptyDesign': {
        const d = E.emptyDesign(a[0]);
        return { meta: d.meta, pages: E.sortedPages(d).map(normId), elements: d.elements, assets: d.assets };
      }
      case 'placeAll': return E.placeAll(a[0], a[1], a[2] ?? null);
      default: return SKIP;
    }
  }));

  test('embeds, svg, doc blocks', () => {
    replay('schema/embeds', golden('schema', 'embeds'), (fn, a) => {
      if (fn !== 'parseEmbedUrl') return SKIP;
      const r = E.parseEmbedUrl(a[0]);
      // Known difference (crates/oc-schema goldens): TS throws URIError on a malformed escape; Rust rejects the URL.
      if ((a[0] as string).includes('bad%E0%A4%A')) {
        expect(r).toBeNull();
        return SKIP;
      }
      return r;
    });
    replay('schema/svg', golden('schema', 'svg'), (fn, a) => (fn === 'extractSvgColors' ? E.extractSvgColors(a[0]) : SKIP));
    replay('schema/doc_blocks', golden('schema', 'doc_blocks'), (fn, a) => {
      const arg = (v: unknown) => (v === '__undefined__' ? undefined : v);
      switch (fn) {
        case 'parseDocChart': return E.parseDocChart(arg(a[0]));
        case 'serializeDocChart': return JSON.parse(E.serializeDocChart(E.parseDocChart(arg(a[0]))));
        case 'docBlockWidth': return E.docBlockWidth(arg(a[0]), a[1] ?? undefined);
        case 'calloutTone': return E.calloutTone(arg(a[0]));
        default: return SKIP;
      }
    });
  });

  test('print', () => {
    const g = golden('schema', 'print');
    const docOf = (n: string) => g.docs[n] ?? fixture(n);
    replay('schema/print', g.cases, (fn, a) => {
      switch (fn) {
        case 'standalonePrintTarget': return E.standalonePrintTarget(docOf(a[0]));
        case 'preflightPrint': {
          const fonts: string[] | null = a[2];
          return E.preflightPrint(docOf(a[0]), a[1], fonts ? { availableFonts: { has: (f: string) => fonts.includes(f) } } : {});
        }
        default: return SKIP;
      }
    });
  });

  test('print colours and the colour-managed gamut hook (tests/golden/engine/print-colors.json, recorded from @oc/schema)', () => {
    const G = golden<{
      docs: Record<string, { colors: E.ColorValue[]; colors2: E.ColorValue[]; keys: string[]; preflight: unknown }>;
      samples: Array<{ color: E.ColorValue; key: string }>;
    }>('engine', 'print-colors');
    let checked = 0;
    expect(Object.keys(G.docs).sort()).toEqual(fixtureNames());
    for (const [name, want] of Object.entries(G.docs)) {
      const doc = fixture(name);
      expect(diff(E.printColors(doc), want.colors), name).toBeNull();
      expect(diff(E.printColors(doc, 2), want.colors2), name).toBeNull();
      expect(want.colors.map(E.printColorKey), name).toEqual(want.keys);
      // Every other colour flagged out of gamut, the rest left to the heuristic (undefined) or in gamut.
      const verdict = (c: E.ColorValue) => {
        const i = want.keys.indexOf(E.printColorKey(c));
        return i < 0 ? undefined : i % 3 === 0 ? true : i % 3 === 1 ? false : undefined;
      };
      const target = E.standalonePrintTarget(doc);
      const d = diff(JSON.parse(JSON.stringify(E.preflightPrint(doc, target, { outOfGamut: verdict }))), want.preflight);
      expect(d, name).toBeNull();
      checked += want.colors.length;
    }
    expect(checked).toBeGreaterThan(0);
    expect(G.samples.map((s) => E.printColorKey(s.color))).toEqual(G.samples.map((s) => s.key));
  });
});

describe('@oc/geometry helpers', () => {
  test('matrix, shapes, markers', () => {
    replay('geometry/matrix', golden('geometry', 'matrix'), (fn, a) => (fn === 'apply' ? E.apply(a[0], a[1]) : fn === 'multiply' ? E.multiply(a[0], a[1]) : SKIP));
    replay('geometry/shapes', golden('geometry', 'shapes'), (fn, a) => {
      if (fn === 'shapePath') return E.shapePath(a[0], a[1], a[2], a[3] === '__undefined__' ? undefined : a[3], a[4] === '__undefined__' ? undefined : a[4]);
      if (fn === 'scaleUnitPath') return E.scaleUnitPath(a[0], a[1], a[2]);
      return SKIP;
    });
    replay('geometry/markers', golden('geometry', 'markers'), (fn, a) => {
      if (fn === 'lineMarkerShape') return E.lineMarkerShape(a[0], a[1]);
      if (fn === 'trimLineForMarkers') return E.trimLineForMarkers(a[0], a[1], a[2], a[3]);
      return SKIP;
    });
    replay('geometry/connectors', golden('geometry', 'connectors'), (fn) => (fn === 'ANCHORS' ? E.ANCHORS : SKIP));
  });

  // Hand-built scenes and every fixture-derived scene: world matrices/quads/bounds, hit testing on a grid (with
  // tolerance and entered groups) and anchor points, through `DocRef`s built from plain `{ elements }` objects.
  test.each(['scenes', 'templates'])('%s', (file) => {
    const GRID = 9;
    const failures: string[] = [];
    let checks = 0;
    const check = (label: string, actual: unknown, want: unknown) => {
      checks++;
      const d = diff(JSON.parse(JSON.stringify(actual ?? null)), want);
      if (d) failures.push(`${label}: ${d}`);
    };
    for (const sc of golden('geometry', file) as any[]) {
      // Hand scenes carry their elements; template scenes are a fixture with synthetic groups/connectors patched in.
      const elements = sc.elements ?? { ...fixture(sc.file).elements, ...sc.patch };
      const doc = { elements } as any;
      const ids = Object.keys(elements);
      const idx = (e: { id: string } | null) => (e ? ids.indexOf(e.id) : -1);
      const r = sc.results;
      for (const er of r.elements) {
        const l = `${sc.name ?? sc.file} ${er.id}`;
        check(`${l} world`, E.worldMatrix(doc, er.id), er.world);
        check(`${l} quad`, E.worldQuad(doc, er.id), er.quad);
        check(`${l} bounds`, E.worldBounds(doc, er.id), er.bounds);
      }
      for (const pr of r.pages) {
        const grid: Array<[number, number]> = [];
        for (let i = 0; i < GRID; i++) for (let j = 0; j < GRID; j++) grid.push([((i + 0.5) * pr.w) / GRID, ((j + 0.5) * pr.h) / GRID]);
        check(`${sc.name} page ${pr.id} hitTest`, grid.map((p) => [0, 4].map((t) => idx(E.hitTest(doc, pr.id, p, { tolerance: t })))), pr.hit);
        for (const er of pr.enter) check(`${sc.name} enter ${er.group}`, grid.map((p) => idx(E.hitTest(doc, pr.id, p, { enterGroup: er.group }))), er.out);
      }
      for (const ar of r.anchors) check(`${sc.name} anchorPoint ${ar.id} ${ar.anchor}`, E.anchorPoint(doc, ar.id, ar.anchor), ar.out);
    }
    report.push(`geometry/${file}: ${checks} scene checks through the facade`);
    expect(failures.slice(0, 20), `${failures.length} of ${checks} failing`).toEqual([]);
  });

  test('report', () => {
    console.log(report.join('\n'));
  });
});
