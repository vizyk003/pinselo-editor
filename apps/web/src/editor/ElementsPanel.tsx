/**
 * Elements panel (docs/plan/04-ux-spec.md §3.3; plan 11 g71, g72): recently used and recommended elements, named shape
 * sections (with flowchart symbols), line presets, and one search across shapes, lines, graphics and the stock
 * libraries with the results grouped by kind. "Recommended" is rule-based: it follows the design's surface and the
 * elements already in it (no AI).
 */
import { lineMarkerShape, shapePath, trimLineForMarkers } from '@oc/engine';
import { STICKY_COLORS } from '@oc/engine';
import {
  makeChart, parseEmbedUrl, makeLine, makeQr, makeShape, makeStroke, makeTable, SHAPE_KINDS, type ChartKind,
  type Element, type FrameClip, type LineMarker, type ShapeKind, makeGrid, GRID_LAYOUTS, makeForm, type DesignDoc,
} from '@oc/engine';
import { ChevronDown, ChevronUp, Music, QrCode } from 'lucide-react';
import { useEffect, useMemo, useState, useSyncExternalStore } from 'react';
import { ApiError, contentApi, type GraphicEntry, type StockItem, type StockKind } from '../api';
import { SearchField, useToast } from '../components/ui';
import { insertAssetAt, insertEmbed } from './insert';
import { t, tk } from '../i18n';
import { useDoc, useEd } from './context';
import { FramesSection, framesMatch } from './Frames';
import { insertGraphic } from './GraphicsPanel';
import { STOCK_MIME, stockMedia } from './LibraryPanel';
import { markDragMedia } from './dropTarget';
import { fmtTime } from './media';

// ------------------------------------------------------------------ catalogue

const CHARTS: Array<[ChartKind, string]> = [
  ['column', tk('Column')], ['bar', tk('Bar')], ['stacked-column', tk('Stacked column')], ['stacked-bar', tk('Stacked bar')], ['line', tk('Line')], ['area', tk('Area')],
  ['combo', tk('Column + line')], ['pie', tk('Pie')], ['donut', tk('Donut')], ['scatter', tk('Scatter')], ['bubble', tk('Bubble')], ['radar', tk('Radar')],
  ['funnel', tk('Funnel')], ['progress-ring', tk('Progress ring')], ['progress-bar', tk('Progress bar')], ['gauge', tk('Gauge')],
];
const SHAPE_LABELS: Record<string, string> = {
  rect: tk('Rectangle'), ellipse: tk('Ellipse'), triangle: tk('Triangle'), 'right-triangle': tk('Right triangle'), diamond: tk('Diamond'), pentagon: tk('Pentagon'),
  hexagon: tk('Hexagon'), octagon: tk('Octagon'), polygon: tk('Polygon'), star: tk('Star'), burst: tk('Burst'), seal: tk('Seal'), heart: tk('Heart'),
  arrow: tk('Arrow'), 'double-arrow': tk('Double arrow'), chevron: tk('Chevron'), cross: tk('Cross'), plus: tk('Plus'), speech: tk('Speech bubble'),
  cloud: tk('Cloud'), parallelogram: tk('Parallelogram'), trapezoid: tk('Trapezoid'), ring: tk('Ring'), 'half-circle': tk('Half circle'), arch: tk('Arch'),
  moon: tk('Moon'), drop: tk('Drop'), lightning: tk('Lightning'), tag: tk('Tag'), ribbon: tk('Ribbon'), flag: tk('Flag'), blob: tk('Blob'), custom: tk('Custom shape'),
  squircle: tk('Squircle'), pill: tk('Pill'), 'snip-corner': tk('Snipped corner'), 'snip-two': tk('Snipped corners'), 'round-one': tk('Rounded corner'),
  'folded-corner': tk('Folded corner'), plaque: tk('Plaque'), frame: tk('Square frame'), ticket: tk('Ticket'), shield: tk('Shield'), cylinder: tk('Cylinder'),
  cube: tk('Cube'), 'block-arc': tk('Block arc'), pie: tk('Pie'), 'star-6': tk('6-point star'), 'star-8': tk('8-point star'), 'starburst-12': tk('12-point star'),
  sparkle: tk('Sparkle'), badge: tk('Badge'), gear: tk('Gear'), flower: tk('Flower'), sun: tk('Sun'), leaf: tk('Leaf'), 'cross-x': tk('X mark'), check: tk('Check mark'),
  'chevron-double': tk('Double chevron'), 'curved-arrow': tk('Curved arrow'), 'u-turn-arrow': tk('U-turn arrow'), 'circular-arrow': tk('Circular arrow'),
  banner: tk('Banner'), wave: tk('Wave'), 'callout-rect': tk('Rounded callout'), 'callout-round': tk('Oval callout'), 'thought-bubble': tk('Thought bubble'),
  document: tk('Document'), 'multi-document': tk('Multiple documents'), 'predefined-process': tk('Predefined process'), 'manual-input': tk('Manual input'),
  delay: tk('Delay'), 'off-page': tk('Off-page connector'), 'internal-storage': tk('Internal storage'), display: tk('Display'),
};
/** What a shape means in a flowchart (the Flowchart section names the symbol, not the geometry). */
const FLOW_LABELS: Partial<Record<ShapeKind, string>> = {
  rect: tk('Process'), diamond: tk('Decision'), pill: tk('Start or end'), parallelogram: tk('Data (input or output)'), trapezoid: tk('Manual operation'),
  hexagon: tk('Preparation'), cylinder: tk('Database'), ellipse: tk('Connector'),
};
/** Named sections of the shape catalogue (every kind of SHAPE_KINDS appears in one of the first seven). */
const SHAPE_SECTIONS: Array<{ id: string; label: string; kinds: ShapeKind[]; flow?: true }> = [
  { id: 'basic', label: tk('Basic shapes'), kinds: ['rect', 'ellipse', 'triangle', 'right-triangle', 'diamond', 'squircle', 'pill', 'half-circle', 'arch', 'ring', 'block-arc', 'pie'] },
  { id: 'boxes', label: tk('Boxes and frames'), kinds: ['snip-corner', 'snip-two', 'round-one', 'plaque', 'folded-corner', 'frame', 'ticket', 'cylinder', 'cube'] },
  { id: 'polygons', label: tk('Polygons'), kinds: ['pentagon', 'hexagon', 'octagon', 'polygon', 'parallelogram', 'trapezoid', 'shield'] },
  { id: 'stars', label: tk('Stars and badges'), kinds: ['star', 'star-6', 'star-8', 'starburst-12', 'burst', 'seal', 'badge', 'sparkle', 'sun', 'gear', 'flower'] },
  { id: 'symbols', label: tk('Symbols'), kinds: ['heart', 'moon', 'drop', 'leaf', 'cloud', 'lightning', 'blob', 'cross', 'plus', 'cross-x', 'check'] },
  { id: 'arrows', label: tk('Arrows'), kinds: ['arrow', 'double-arrow', 'chevron', 'chevron-double', 'curved-arrow', 'u-turn-arrow', 'circular-arrow'] },
  { id: 'banners', label: tk('Banners and callouts'), kinds: ['tag', 'ribbon', 'flag', 'banner', 'wave', 'speech', 'callout-rect', 'callout-round', 'thought-bubble'] },
  {
    id: 'flowchart', label: tk('Flowchart'), flow: true,
    kinds: ['rect', 'diamond', 'pill', 'parallelogram', 'document', 'multi-document', 'predefined-process', 'manual-input', 'trapezoid', 'hexagon', 'cylinder', 'delay', 'off-page', 'internal-storage', 'display', 'ellipse'],
  },
];
/** Tiles a collapsed section shows (one row and a half in the panel). */
const SECTION_PEEK = 8;
/** Kinds inserted (and previewed) in a landscape box. */
const WIDE_SHAPES = new Set<string>(['arrow', 'chevron', 'double-arrow', 'parallelogram', 'tag', 'ribbon', 'flag', 'pill', 'ticket', 'banner', 'wave', 'chevron-double']);
/** Flowchart symbols are inserted landscape, except the round connector and the off-page marker. */
const FLOW_SQUARE = new Set<string>(['ellipse', 'off-page']);
export const shapeLabel = (k: string, flow = false) => (flow && FLOW_LABELS[k as ShapeKind] ? t(FLOW_LABELS[k as ShapeKind]!) : SHAPE_LABELS[k] ? t(SHAPE_LABELS[k]) : k);

type LinePreset = { dash?: number[]; start: LineMarker; end: LineMarker };
const DASHES: Array<{ id: string; label: string; dash?: number[] }> = [
  { id: 'solid', label: tk('Solid') }, { id: 'dashed', label: tk('Dashed'), dash: [3, 2] }, { id: 'dotted', label: tk('Dotted'), dash: [0.01, 2] },
];
const ENDS: Array<{ id: string; label: string; start: LineMarker; end: LineMarker }> = [
  { id: 'plain', label: tk('Line'), start: 'none', end: 'none' },
  { id: 'arrow', label: tk('Arrow'), start: 'none', end: 'arrow' },
  { id: 'triangle', label: tk('Triangle arrow'), start: 'none', end: 'triangle' },
  { id: 'double', label: tk('Double arrow'), start: 'arrow', end: 'arrow' },
  { id: 'circle', label: tk('Circle to arrow'), start: 'circle', end: 'triangle' },
];
/** 15 line presets: three dash styles × five end styles. */
const LINES: Array<LinePreset & { id: string; label: () => string }> = DASHES.flatMap((d) => ENDS.map((e) => ({
  id: `${d.id}-${e.id}`, dash: d.dash, start: e.start, end: e.end,
  label: () => (d.id === 'solid' ? t(e.label) : t('{style} {kind}', { style: t(d.label), kind: t(e.label).toLocaleLowerCase() })),
})));

function LineIcon({ dash, start, end }: LinePreset) {
  const [a, b] = trimLineForMarkers([[8, 20], [92, 20]], start, end, 4);
  const [x1, x2] = [a?.[0] ?? 8, b?.[0] ?? 92];
  const mark = (m: LineMarker, x: number, rot: number) => {
    const s = lineMarkerShape(m, 4);
    return s && <path d={s.d} transform={`translate(${x} 20) rotate(${rot})`} fill={s.mode === 'fill' ? 'currentColor' : 'none'} stroke={s.mode === 'stroke' ? 'currentColor' : undefined} strokeWidth={4} strokeLinejoin="round" strokeLinecap="round" />;
  };
  return (
    <>
      <line x1={x1} y1={20} x2={x2} y2={20} stroke="currentColor" strokeWidth={4} strokeDasharray={dash ? (dash[0]! < 1 ? '1 8' : '12 8') : undefined} strokeLinecap="round" />
      {mark(end, 92, 0)}
      {mark(start, 8, 180)}
    </>
  );
}

function ShapeIcon({ kind, fill = 'currentColor', wide }: { kind: ShapeKind | FrameClip; fill?: string; wide?: boolean }) {
  const w = wide ?? WIDE_SHAPES.has(kind);
  return (
    <svg viewBox="-2 -2 104 104" aria-hidden>
      <path d={shapePath(kind, 100, w ? 60 : 100, kind === 'star' ? { points: 5 } : {})} transform={w ? 'translate(0 20)' : undefined} fill={fill} />
    </svg>
  );
}

// ------------------------------------------------------------------ recently used

/** An element the panel can insert again (stored per device). */
type RecentItem =
  | { k: 'shape'; kind: ShapeKind; flow?: boolean }
  | { k: 'line'; id: string }
  | { k: 'graphic'; id: string; kind: GraphicEntry['kind']; name: string }
  | { k: 'chart'; kind: ChartKind }
  | { k: 'table'; r: number; c: number }
  | { k: 'sticky'; color: string };
const RECENT_KEY = 'oc:recent-elements';
const RECENT_MAX = 12;
const recentKey = (i: RecentItem) => JSON.stringify(i.k === 'graphic' ? { k: i.k, id: i.id } : i);

function loadRecent(): RecentItem[] {
  try {
    const v = JSON.parse(localStorage.getItem(RECENT_KEY) ?? '[]') as unknown;
    return Array.isArray(v) ? (v as RecentItem[]).filter((x) => x && typeof x === 'object' && 'k' in x).slice(0, RECENT_MAX) : [];
  } catch {
    return [];
  }
}
let recent = loadRecent();
const recentListeners = new Set<() => void>();
export function recordRecentElement(item: RecentItem) {
  const key = recentKey(item);
  recent = [item, ...recent.filter((x) => recentKey(x) !== key)].slice(0, RECENT_MAX);
  try {
    localStorage.setItem(RECENT_KEY, JSON.stringify(recent));
  } catch {
    /* private mode: keep in memory */
  }
  for (const l of recentListeners) l();
}
const useRecent = () => useSyncExternalStore((cb) => {
  recentListeners.add(cb);
  return () => recentListeners.delete(cb);
}, () => recent);

// ------------------------------------------------------------------ recommended (rule-based)

/**
 * Up to eight elements for this design, from fixed rules: what suits the design's surface, then more of the shapes and
 * charts it already uses. No model or usage data from other people is involved.
 */
export function recommendElements(doc: Pick<DesignDoc, 'meta' | 'elements'>): RecentItem[] {
  const out: RecentItem[] = [];
  const used = Object.values(doc.elements).filter((e) => !e.deleted);
  for (const e of used) {
    if (e.type === 'shape' && e.kind !== 'custom' && !out.some((x) => x.k === 'shape' && x.kind === e.kind)) out.push({ k: 'shape', kind: e.kind });
    if (e.type === 'chart' && !out.some((x) => x.k === 'chart' && x.kind === e.kind)) out.push({ k: 'chart', kind: e.kind });
    if (out.length >= 3) break;
  }
  const bySurface: Record<string, RecentItem[]> = {
    whiteboard: [{ k: 'sticky', color: STICKY_COLORS[0]! }, { k: 'shape', kind: 'rect', flow: true }, { k: 'shape', kind: 'diamond', flow: true }, { k: 'shape', kind: 'pill', flow: true }, { k: 'line', id: 'solid-arrow' }, { k: 'table', r: 3, c: 3 }],
    presentation: [{ k: 'chart', kind: 'column' }, { k: 'chart', kind: 'pie' }, { k: 'table', r: 3, c: 3 }, { k: 'shape', kind: 'arrow' }, { k: 'chart', kind: 'progress-ring' }, { k: 'line', id: 'solid-plain' }],
    doc: [{ k: 'table', r: 3, c: 3 }, { k: 'chart', kind: 'bar' }, { k: 'chart', kind: 'line' }, { k: 'line', id: 'solid-plain' }],
    sheet: [{ k: 'chart', kind: 'column' }, { k: 'chart', kind: 'line' }, { k: 'chart', kind: 'pie' }],
    website: [{ k: 'shape', kind: 'pill' }, { k: 'shape', kind: 'squircle' }, { k: 'line', id: 'solid-plain' }, { k: 'shape', kind: 'ellipse' }],
    print: [{ k: 'shape', kind: 'rect' }, { k: 'line', id: 'solid-plain' }, { k: 'shape', kind: 'badge' }, { k: 'shape', kind: 'ribbon' }],
  };
  const fallback: RecentItem[] = [{ k: 'shape', kind: 'star' }, { k: 'shape', kind: 'burst' }, { k: 'shape', kind: 'ribbon' }, { k: 'shape', kind: 'speech' }, { k: 'line', id: 'solid-arrow' }, { k: 'shape', kind: 'heart' }, { k: 'shape', kind: 'banner' }, { k: 'shape', kind: 'ellipse' }];
  for (const x of [...(bySurface[doc.meta.surface] ?? []), ...fallback]) {
    if (out.length >= 8) break;
    if (!out.some((y) => recentKey(y) === recentKey(x))) out.push(x);
  }
  return out.slice(0, 8);
}

// ------------------------------------------------------------------ panel

export function ElementsPanel() {
  const { editor } = useEd();
  const doc = useDoc();
  const toast = useToast();
  const [q, setQ] = useState('');
  const [open, setOpen] = useState<Record<string, boolean>>({});
  const recentItems = useRecent();
  const recommended = useMemo(() => recommendElements(doc), [doc]);
  const base = Math.min(doc.meta.dims.w, doc.meta.dims.h) * 0.3;
  const dragData = (els: Element[], item?: RecentItem) => (e: React.DragEvent) => {
    e.dataTransfer.setData('application/x-oc-element', JSON.stringify(els));
    if (item) recordRecentElement(item);
  };
  const shape = (kind: ShapeKind, flow = false) => {
    const wide = flow ? !FLOW_SQUARE.has(kind) : WIDE_SHAPES.has(kind);
    return makeShape({
      kind, xf: { x: 0, y: 0, w: wide ? base * 1.4 : base, h: wide ? base * (flow ? 0.9 : 0.8) : kind === 'half-circle' ? base / 2 : base, rot: 0 },
      fill: '#8b5cf6', params: kind === 'star' ? { points: 5, innerRatio: 0.45 } : kind === 'polygon' ? { sides: 6 } : {},
    });
  };
  const table = (r: number, c: number) => makeTable({ xf: { x: 0, y: 0, w: base * 2.4, h: base * 0.35 * r, rot: 0 }, data: Array.from({ length: r }, (_, i) => Array.from({ length: c }, (_, j) => (i === 0 ? t('Heading {n}', { n: j + 1 }) : ''))) });
  const chart = (kind: ChartKind) => {
    const single = kind === 'progress-ring' || kind === 'progress-bar' || kind === 'gauge';
    const sq = single && kind !== 'progress-bar';
    const xf = { x: 0, y: 0, w: base * (sq ? 1.4 : 2.2), h: base * (kind === 'progress-bar' ? 0.5 : sq ? 1.4 : 1.6), rot: 0 };
    if (single) return makeChart({ kind, xf, labels: [t('Complete')], series: [{ name: t('Progress'), values: [72] }] });
    if (kind === 'pie' || kind === 'donut') return makeChart({ kind, xf, labels: [t('Design'), t('Build'), t('Ship')], series: [{ name: t('Share'), values: [45, 35, 20] }] });
    if (kind === 'funnel') return makeChart({ kind, xf, labels: [t('Visitors'), t('Sign-ups'), t('Trials'), t('Customers')], series: [{ name: t('People'), values: [1000, 420, 180, 75] }] });
    if (kind === 'scatter' || kind === 'bubble') return makeChart({ kind, xf, labels: ['1', '3', '4', '6', '8'], series: [{ name: t('Value'), values: [12, 30, 22, 40, 34] }, { name: t('Size'), values: [5, 12, 8, 20, 10] }] });
    return makeChart({ kind, xf });
  };
  // Sticky notes carry their colour and author (board.sticky); on whiteboards they keep a fixed size as the board grows.
  const addSticky = (color: string) => {
    editor.addSticky(color, { text: t('Write an idea…'), size: doc.meta.surface === 'whiteboard' ? 240 : base * 1.1 });
    recordRecentElement({ k: 'sticky', color });
  };
  const line = (l: LinePreset) => makeLine({ xf: { x: 0, y: 0, w: base * 1.6, h: 24, rot: 0 }, color: '#1f2328', width: Math.max(3, base / 60), dash: l.dash, startMarker: l.start, endMarker: l.end });
  const insert = (els: Element[], item?: RecentItem) => {
    editor.insert(els);
    if (item) recordRecentElement(item);
  };
  const addGraphic = async (g: { id: string; kind: GraphicEntry['kind']; name: string }) => {
    try {
      await insertGraphic(editor, g.id, undefined, undefined, g.kind);
      recordRecentElement({ k: 'graphic', id: g.id, kind: g.kind, name: g.name });
    } catch {
      toast(t('Couldn’t add that graphic.'));
    }
  };

  /** One tile for a catalogue item (recent, recommended, sections, search results). */
  const tileFor = (item: RecentItem, key?: string) => {
    switch (item.k) {
      case 'shape': {
        const label = shapeLabel(item.kind, item.flow);
        const els = () => [shape(item.kind, item.flow)];
        return (
          <button key={key ?? recentKey(item)} className="tile" title={label} aria-label={label} draggable onDragStart={dragData(els(), item)} onClick={() => insert(els(), item)} style={{ color: 'var(--text-2)' }}>
            <ShapeIcon kind={item.kind} wide={item.flow ? !FLOW_SQUARE.has(item.kind) : undefined} />
          </button>
        );
      }
      case 'line': {
        const l = LINES.find((x) => x.id === item.id);
        if (!l) return null;
        return (
          <button key={key ?? recentKey(item)} className="tile" title={l.label()} aria-label={l.label()} draggable onDragStart={dragData([line(l)], item)} onClick={() => insert([line(l)], item)}>
            <svg viewBox="0 0 100 40" aria-hidden><LineIcon {...l} /></svg>
          </button>
        );
      }
      case 'graphic': {
        const name = item.name.replace(/-/g, ' ');
        return (
          <button key={key ?? recentKey(item)} className="tile" title={name} aria-label={name} style={{ background: '#fff', padding: item.kind === 'icon' ? 12 : 6 }} onClick={() => void addGraphic(item)}>
            <img src={`/api/graphics/${item.id}.svg`} alt="" loading="lazy" draggable={false} style={{ objectFit: 'contain', borderRadius: 0 }} />
          </button>
        );
      }
      case 'chart': {
        const label = t('{type} chart', { type: t(CHARTS.find(([k]) => k === item.kind)?.[1] ?? item.kind) });
        return (
          <button key={key ?? recentKey(item)} className="tile" title={label} aria-label={label} onClick={() => insert([chart(item.kind)], item)} style={{ color: 'var(--text-2)' }}>
            <ChartIcon kind={item.kind} />
          </button>
        );
      }
      case 'table': {
        const label = t('{rows} × {cols} table', { rows: item.r, cols: item.c });
        return (
          <button key={key ?? recentKey(item)} className="tile" title={label} aria-label={label} onClick={() => insert([table(item.r, item.c)], item)}>
            <TableIcon r={item.r} c={item.c} />
          </button>
        );
      }
      case 'sticky':
        return (
          <button key={key ?? recentKey(item)} className="tile" title={t('Sticky note')} aria-label={t('Sticky note')} onClick={() => addSticky(item.color)}>
            <span style={{ width: '70%', height: '70%', background: item.color, boxShadow: '0 3px 6px rgba(0,0,0,.18)', borderRadius: 3 }} />
          </button>
        );
    }
  };

  const needle = q.trim().toLowerCase();
  const match = (s: string) => !needle || s.toLowerCase().includes(needle);
  const shapeMatch = (k: ShapeKind, flow = false) => match(k) || match(shapeLabel(k, flow)) || match(shapeLabel(k));
  const sections = SHAPE_SECTIONS.map((s) => ({ ...s, kinds: needle && !match(t(s.label)) ? s.kinds.filter((k) => shapeMatch(k, s.flow)) : s.kinds })).filter((s) => s.kinds.length);
  const lines = needle && !match(t('Lines & arrows')) && !match('lines') ? LINES.filter((l) => match(l.label()) || match(l.id)) : LINES;
  const section = (id: string, label: string, tiles: React.ReactNode[]) => {
    const expanded = Boolean(needle) || open[id] || tiles.length <= SECTION_PEEK;
    return (
      <div key={id} className="el-section">
        <div className="row el-section-head">
          <h4 className="grow" id={`el-sec-${id}`}>{label}</h4>
          {!needle && tiles.length > SECTION_PEEK && (
            <button className="btn ghost sm" aria-expanded={Boolean(open[id])} aria-controls={`el-sec-${id}-tiles`} onClick={() => setOpen((o) => ({ ...o, [id]: !o[id] }))}>
              {open[id] ? <>{t('Show less')} <ChevronUp size={14} aria-hidden /></> : <>{t('See all')} <ChevronDown size={14} aria-hidden /></>}
            </button>
          )}
        </div>
        <div className="tiles" id={`el-sec-${id}-tiles`} role="group" aria-labelledby={`el-sec-${id}`}>{expanded ? tiles : tiles.slice(0, SECTION_PEEK)}</div>
      </div>
    );
  };

  return (
    <>
      <SearchField value={q} onChange={setQ} placeholder={t('Search elements')} onClear={() => setQ('')} />
      {!needle && recentItems.length > 0 && section('recent', t('Recently used'), recentItems.map((x) => tileFor(x)))}
      {!needle && section('recommended', t('Recommended for this design'), recommended.map((x) => tileFor(x, `rec:${recentKey(x)}`)))}
      {needle && <h4 className="faint" style={{ margin: '4px 0 0', fontSize: 12 }}>{t('All results')}</h4>}
      {(!needle || match('embed') || match('video') || match('youtube') || match('map') || match('music')) && <EmbedBox />}
      {sections.map((s) => section(`shape-${s.id}`, t(s.label), s.kinds.map((k) => tileFor({ k: 'shape', kind: k, ...(s.flow ? { flow: true } : {}) }, `${s.id}:${k}`))))}
      {lines.length > 0 && section('lines', t('Lines & arrows'), lines.map((l) => tileFor({ k: 'line', id: l.id })))}
      {needle && <RemoteResults q={needle} onGraphic={(g) => void addGraphic(g)} />}
      {(match('grids') || match('photo') || match('collage')) && (
        <>
          <h4>{t('Photo grids')}</h4>
          <p className="faint" style={{ margin: 0, fontSize: 12 }}>{t('Drop photos into the cells. Adjust the gap in the toolbar.')}</p>
          <div className="tiles">
            {Object.entries(GRID_LAYOUTS).map(([id, l]) => {
              const g = () => makeGrid({ layout: id, xf: { x: 0, y: 0, w: base * 2.6, h: base * 1.8, rot: 0 }, gap: Math.max(4, Math.round(base / 40)) });
              // Layout labels come from @oc/schema; they are marked for extraction in Toolbar.tsx (GRID_LAYOUT_LABELS).
              return (
                <button key={id} className="tile" title={t(l.label)} aria-label={t(l.label)} draggable onDragStart={dragData([g()])} onClick={() => editor.insert([g()])}>
                  <svg viewBox="0 0 30 20" aria-hidden>{l.cells.map((c, k) => <rect key={k} x={c.x * 30 + 0.8} y={c.y * 20 + 0.8} width={c.w * 30 - 1.6} height={c.h * 20 - 1.6} rx={1} fill="#cbd5e1" />)}</svg>
                </button>
              );
            })}
          </div>
        </>
      )}
      {(match('forms') || match('survey') || match('contact')) && (
        <>
          <h4>{t('Forms')}</h4>
          <p className="faint" style={{ margin: 0, fontSize: 12 }}>{t('Collect responses on public links and websites.')}</p>
          <div className="row" style={{ flexWrap: 'wrap', gap: 6 }}>
            {([
              ['Contact', t('Contact form'), t('Get in touch'), undefined],
              ['Sign-up', t('Sign-up form'), t('Sign-up'), [{ id: 'name', kind: 'text', label: t('Full name'), required: true }, { id: 'email', kind: 'email', label: t('Email'), required: true }, { id: 'updates', kind: 'checkboxes', label: t('Keep me posted'), options: [t('Product news'), t('Events')] }]],
              ['RSVP', t('RSVP form'), t('RSVP'), [{ id: 'name', kind: 'text', label: t('Your name'), required: true }, { id: 'attending', kind: 'choice', label: t('Will you attend?'), required: true, options: [t('Yes'), t('No'), t('Maybe')] }, { id: 'guests', kind: 'number', label: t('Guests') }]],
              ['Feedback', t('Feedback form'), t('Feedback'), [{ id: 'rating', kind: 'choice', label: t('How was it?'), required: true, options: [t('Great'), t('Good'), t('Okay'), t('Poor')] }, { id: 'comments', kind: 'longtext', label: t('Comments') }]],
            ] as const).map(([key, button, title, fields]) => {
              const make = () => makeForm({ xf: { x: 0, y: 0, w: Math.min(520, doc.meta.dims.w * 0.8), h: 100, rot: 0 }, title, ...(fields ? { fields: fields.map((f) => ({ ...f, options: 'options' in f ? [...f.options] : undefined })) as never } : {}) });
              return <button key={key} className="btn sm" draggable onDragStart={dragData([make()])} onClick={() => editor.insert([make()])}>{button}</button>;
            })}
          </div>
        </>
      )}
      {(match('connectors') || match('lines') || match('flow')) && (
        <>
          <h4>{t('Connectors')}</h4>
          <p className="faint" style={{ margin: 0, fontSize: 12 }}>{t('Drag an end onto a shape to glue it; it follows when the shape moves.')}</p>
          <div className="tiles">
            {(['straight', 'elbow', 'curved'] as const).map((route) => {
              const c = () => ({ ...line({ start: 'none', end: 'arrow' }), route: route === 'straight' ? undefined : route });
              const label = { straight: t('Straight connector'), elbow: t('Elbow connector'), curved: t('Curved connector') }[route];
              return (
                <button key={route} className="tile" title={label} aria-label={label} draggable onDragStart={dragData([c() as Element])} onClick={() => editor.insert([c() as Element])}>
                  <svg viewBox="0 0 100 60" aria-hidden>
                    <path d={route === 'elbow' ? 'M10 15 H50 V45 H86' : route === 'curved' ? 'M10 15 C60 15 40 45 86 45' : 'M10 15 L86 45'} fill="none" stroke="currentColor" strokeWidth={4} />
                    <circle cx={10} cy={15} r={5} fill="currentColor" />
                    <path d="M76 37 L90 45 L76 53" fill="none" stroke="currentColor" strokeWidth={4} />
                  </svg>
                </button>
              );
            })}
          </div>
        </>
      )}
      {framesMatch(q) && <FramesSection q={q} base={base} />}
      {(match('table') || match('grid') || match(t('Tables'))) && section('tables', t('Tables'), ([[3, 3], [4, 3], [5, 4]] as const).map(([r, c]) => tileFor({ k: 'table', r, c })))}
      {(match('chart') || match('graph') || match(t('Charts')) || CHARTS.some(([k, l]) => match(k) || match(t(l)))) && (
        <>
          <h4>{t('Charts')}</h4>
          <div className="tiles">
            {CHARTS.filter(([k, l]) => !needle || match('chart') || match('graph') || match(t('Charts')) || match(k) || match(t(l))).map(([k, label]) => (
              <button key={k} className="tile" title={t('{type} chart', { type: t(label) })} onClick={() => insert([chart(k)], { k: 'chart', kind: k })} style={{ flexDirection: 'column', fontSize: 11, color: 'var(--text-2)' }}>
                <ChartIcon kind={k} />
                {t(label)}
              </button>
            ))}
          </div>
        </>
      )}
      {(match('sticky') || match('note') || match(t('Sticky notes'))) && section('sticky', t('Sticky notes'), STICKY_COLORS.map((c) => tileFor({ k: 'sticky', color: c })))}
      {(match('qr') || match('code')) && (
        <>
          <h4>{t('QR code')}</h4>
          <button className="btn" onClick={() => editor.insert([makeQr({ xf: { x: 0, y: 0, w: base, h: base, rot: 0 }, data: 'https://example.com' })])}>
            <QrCode size={16} /> {t('Add QR code')}
          </button>
        </>
      )}
      {match('decor') && (
        <>
          <h4>{t('Quick combos')}</h4>
          <div className="tiles two">
            <button className="tile" style={{ aspectRatio: '2' }} onClick={() => editor.insert([
              makeShape({ kind: 'rect', xf: { x: 0, y: 0, w: base * 2, h: base * 0.5, rot: 0 }, fill: '#111827', params: { cornerRadius: base * 0.25 } }),
            ])}>
              <span style={{ background: '#111827', color: '#fff', borderRadius: 999, padding: '6px 14px', fontWeight: 700 }}>{t('Pill button')}</span>
            </button>
            <button className="tile" style={{ aspectRatio: '2' }} onClick={() => editor.insert([
              makeShape({ kind: 'rect', xf: { x: 0, y: 0, w: base * 2, h: base * 1.2, rot: 0 }, fill: '#ffffff', stroke: makeStroke('#e5e7eb', 2), params: { cornerRadius: 16 } }),
            ])}>
              <span style={{ border: '2px solid #e5e7eb', borderRadius: 10, padding: '12px 20px', background: '#fff', color: '#111' }}>{t('Card')}</span>
            </button>
          </div>
        </>
      )}
      {needle && !sections.length && !lines.length && <p className="faint" style={{ fontSize: 12, margin: 0 }}>{t('No shapes or lines match. Graphics and stock results appear above when they load.')}</p>}
    </>
  );
}

/** Make sure every SHAPE_KINDS entry has a section (a new kind added to the schema must be placed). */
export const UNSECTIONED_SHAPES: readonly ShapeKind[] = SHAPE_KINDS.filter((k) => !SHAPE_SECTIONS.slice(0, 7).some((s) => s.kinds.includes(k)) && !SHAPE_SECTIONS[7]!.kinds.includes(k));

// ------------------------------------------------------------------ grouped search results

let providersCache: Promise<Array<{ id: string; label: string; kinds: StockKind[] }>> | null = null;
const providers = () => (providersCache ??= contentApi.providers().then((r) => r.providers).catch(() => []));

const GROUPS: Array<{ kind: StockKind; label: string }> = [{ kind: 'image', label: tk('Photos') }, { kind: 'video', label: tk('Videos') }, { kind: 'audio', label: tk('Audio') }];
const GROUP_PEEK = 6;

/** Graphics and stock media for a query, one group per kind (each shows a few results and can show more). */
function RemoteResults({ q, onGraphic }: { q: string; onGraphic: (g: GraphicEntry) => void }) {
  const [query, setQuery] = useState(q);
  useEffect(() => {
    const timer = setTimeout(() => setQuery(q), 300);
    return () => clearTimeout(timer);
  }, [q]);
  const [graphics, setGraphics] = useState<GraphicEntry[] | null>(null);
  const [stock, setStock] = useState<Partial<Record<StockKind, StockItem[]>>>({});
  const [more, setMore] = useState<Record<string, boolean>>({});
  useEffect(() => {
    let cancelled = false;
    setGraphics(null);
    setStock({});
    setMore({});
    contentApi.graphics(query, 'all', 1).then((r) => !cancelled && setGraphics(r.items)).catch(() => !cancelled && setGraphics([]));
    void providers().then((ps) => {
      for (const g of GROUPS) {
        const p = ps.find((x) => x.kinds.includes(g.kind));
        if (!p) continue;
        contentApi.search(p.id, g.kind, query, 1).then((r) => !cancelled && setStock((s) => ({ ...s, [g.kind]: r.items }))).catch(() => {});
      }
    });
    return () => {
      cancelled = true;
    };
  }, [query]);
  const head = (id: string, label: string, n: number) => (
    <div className="row el-section-head">
      <h4 className="grow" id={`el-res-${id}`}>{label}</h4>
      {n > GROUP_PEEK && <button className="btn ghost sm" aria-expanded={Boolean(more[id])} onClick={() => setMore((m) => ({ ...m, [id]: !m[id] }))}>{more[id] ? t('Show less') : t('See all')}</button>}
    </div>
  );
  return (
    <>
      {graphics === null && <div className="spinner" aria-label={t('Searching…')} />}
      {graphics && graphics.length > 0 && (
        <div className="el-section">
          {head('graphics', t('Graphics'), graphics.length)}
          <div className="tiles" role="group" aria-labelledby="el-res-graphics" style={{ gridTemplateColumns: 'repeat(4, 1fr)', gap: 6 }}>
            {(more.graphics ? graphics : graphics.slice(0, GROUP_PEEK)).map((g) => (
              <button key={g.id} className="tile" title={g.name.replace(/-/g, ' ')} aria-label={g.name.replace(/-/g, ' ')} style={{ background: '#fff', padding: g.kind === 'icon' ? 12 : 6 }} onClick={() => onGraphic(g)}>
                <img src={`/api/graphics/${g.id}.svg`} alt="" loading="lazy" draggable={false} style={{ objectFit: 'contain', borderRadius: 0 }} />
              </button>
            ))}
          </div>
        </div>
      )}
      {GROUPS.map((g) => {
        const items = stock[g.kind];
        if (!items?.length) return null;
        return <StockGroup key={g.kind} label={t(g.label)} items={more[g.kind] ? items : items.slice(0, GROUP_PEEK)} head={head(g.kind, t(g.label), items.length)} />;
      })}
    </>
  );
}

function StockGroup({ items, head }: { label: string; items: StockItem[]; head: React.ReactNode }) {
  const { editor } = useEd();
  const toast = useToast();
  const [adding, setAdding] = useState<string | null>(null);
  const add = async (it: StockItem) => {
    setAdding(it.id);
    try {
      const { asset } = await contentApi.import(it.provider, it.kind, it.id);
      insertAssetAt(editor, asset, undefined);
    } catch (e) {
      toast(e instanceof ApiError ? e.message ?? t('Couldn’t add it.') : t('Couldn’t add it.'));
    } finally {
      setAdding(null);
    }
  };
  return (
    <div className="el-section">
      {head}
      {items[0]?.kind === 'audio' ? (
        <div className="list">
          {items.map((it) => (
            <button key={it.id} className="item audio-row" onClick={() => add(it)} disabled={adding === it.id} title={`${it.title} — ${it.author ?? ''} · ${it.license}`}
              draggable onDragStart={(e) => e.dataTransfer.setData(STOCK_MIME, JSON.stringify({ provider: it.provider, kind: it.kind, id: it.id }))}>
              <Music size={16} aria-hidden />
              <div className="main"><div className="t">{it.title}</div><div className="sub">{it.author ?? ''} · {it.license}</div></div>
              <span className="faint" style={{ fontSize: 12 }}>{it.durationMs ? fmtTime(it.durationMs) : ''}</span>
            </button>
          ))}
        </div>
      ) : (
        <div className="masonry">
          {items.map((it) => (
            <button key={`${it.provider}:${it.id}`} className={`m-tile${adding === it.id ? ' busy' : ''}`} onClick={() => add(it)} title={`${it.title}${it.author ? ` — ${it.author}` : ''} · ${it.license}`}
              draggable onDragStart={(e) => {
                e.dataTransfer.setData(STOCK_MIME, JSON.stringify({ provider: it.provider, kind: it.kind, id: it.id }));
                markDragMedia(e.dataTransfer, stockMedia(it.kind));
              }}>
              <img src={it.thumb} alt={it.title} loading="lazy" referrerPolicy="no-referrer" style={{ aspectRatio: it.w && it.h ? `${it.w} / ${it.h}` : undefined }} />
              {it.kind === 'video' && it.durationMs ? <span className="dur">{fmtTime(it.durationMs)}</span> : null}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}

// ------------------------------------------------------------------ icons

function TableIcon({ r, c }: { r: number; c: number }) {
  return (
    <svg viewBox="0 0 100 80" aria-hidden>
      {Array.from({ length: r + 1 }, (_, i) => <line key={`r${i}`} x1={4} x2={96} y1={4 + (i * 72) / r} y2={4 + (i * 72) / r} stroke="currentColor" strokeWidth={2} />)}
      {Array.from({ length: c + 1 }, (_, i) => <line key={`c${i}`} y1={4} y2={76} x1={4 + (i * 92) / c} x2={4 + (i * 92) / c} stroke="currentColor" strokeWidth={2} />)}
      <rect x={4} y={4} width={92} height={72 / r} fill="#7fd8d0" opacity={0.6} />
    </svg>
  );
}

function ChartIcon({ kind }: { kind: ChartKind }) {
  const c = ['#c93316', '#ff8a75', '#f59e0b'];
  const small = { width: '55%', height: '55%' };
  if (kind === 'progress-ring' || kind === 'gauge')
    return (
      <svg viewBox="0 0 60 60" style={small} aria-hidden>
        <circle cx={30} cy={30} r={22} fill="none" stroke="var(--border)" strokeWidth={8} />
        <path d={kind === 'gauge' ? 'M12 43 A22 22 0 0 1 44 13' : 'M30 8 A22 22 0 1 1 9 37'} fill="none" stroke={c[0]} strokeWidth={8} />
      </svg>
    );
  if (kind === 'progress-bar') return <svg viewBox="0 0 60 20" style={{ width: '65%' }} aria-hidden><rect x={2} y={6} width={56} height={8} rx={4} fill="var(--border)" /><rect x={2} y={6} width={38} height={8} rx={4} fill={c[0]} /></svg>;
  if (kind === 'funnel') return <svg viewBox="0 0 60 50" style={small} aria-hidden><path d="M4 4 H56 L48 18 H12 Z" fill={c[0]} /><path d="M13 20 H47 L40 33 H20 Z" fill={c[1]} /><path d="M21 35 H39 L34 46 H26 Z" fill={c[2]} /></svg>;
  if (kind === 'radar') return <svg viewBox="0 0 60 60" style={small} aria-hidden><path d="M30 4 L55 22 L45 52 L15 52 L5 22 Z" fill="none" stroke="var(--border-strong)" /><path d="M30 14 L48 24 L40 44 L20 40 L14 24 Z" fill={c[0]} opacity={0.5} stroke={c[0]} /></svg>;
  if (kind === 'scatter' || kind === 'bubble')
    return <svg viewBox="0 0 60 40" style={{ width: '60%', height: '50%' }} aria-hidden>{[[10, 30, 3], [22, 18, 5], [34, 24, 4], [46, 8, 6], [54, 16, 3]].map(([x, y, r], i) => <circle key={i} cx={x} cy={y} r={kind === 'bubble' ? r! : 3} fill={c[i % 2]} opacity={kind === 'bubble' ? 0.7 : 1} />)}</svg>;
  if (kind === 'stacked-column' || kind === 'stacked-bar' || kind === 'combo')
    return (
      <svg viewBox="0 0 60 40" style={{ width: '60%', height: '50%', transform: kind === 'stacked-bar' ? 'rotate(90deg)' : undefined }} aria-hidden>
        {[[6, 24], [20, 14], [34, 20], [48, 8]].map(([x, y], i) => <g key={i}><rect x={x} y={y} width={9} height={38 - y!} fill={c[0]} />{kind !== 'combo' && <rect x={x} y={y! - 8} width={9} height={8} fill={c[1]} />}</g>)}
        {kind === 'combo' && <path d="M10 20 L24 10 L38 16 L52 4" fill="none" stroke={c[1]} strokeWidth={3} />}
      </svg>
    );
  if (kind === 'pie' || kind === 'donut')
    return (
      <svg viewBox="0 0 60 60" style={{ width: '55%', height: '55%' }} aria-hidden>
        <circle cx={30} cy={30} r={26} fill={c[1]} />
        <path d="M30 30 L30 4 A26 26 0 0 1 54 40 Z" fill={c[0]} />
        <path d="M30 30 L54 40 A26 26 0 0 1 20 54 Z" fill={c[2]} />
        {kind === 'donut' && <circle cx={30} cy={30} r={13} fill="var(--surface-2)" />}
      </svg>
    );
  if (kind === 'line' || kind === 'area')
    return (
      <svg viewBox="0 0 60 40" style={{ width: '60%', height: '50%' }} aria-hidden>
        {kind === 'area' && <path d="M4 34 L18 20 L32 26 L56 6 L56 36 L4 36 Z" fill={c[0]} opacity={0.3} />}
        <path d="M4 34 L18 20 L32 26 L56 6" fill="none" stroke={c[0]} strokeWidth={3} />
      </svg>
    );
  return (
    <svg viewBox="0 0 60 40" style={{ width: '60%', height: '50%' }} aria-hidden>
      {(kind === 'bar' ? [[4, 6, 40], [4, 18, 28], [4, 30, 50]] : [[6, 14, 24], [22, 4, 34], [38, 20, 18]]).map(([x, y, l], i) =>
        kind === 'bar' ? <rect key={i} x={x} y={y} width={l} height={8} fill={c[i]} rx={2} /> : <rect key={i} x={x} y={y} width={12} height={l} fill={c[i]} rx={2} />)}
    </svg>
  );
}

/** Paste a YouTube, Vimeo, Spotify, SoundCloud, Google Maps, Loom, Figma or CodePen link (A11). */
function EmbedBox() {
  const { editor } = useEd();
  const toast = useToast();
  const [url, setUrl] = useState('');
  const [busy, setBusy] = useState(false);
  const ok = Boolean(parseEmbedUrl(url));
  return (
    <form className="row" style={{ gap: 6 }} onSubmit={async (e) => {
      e.preventDefault();
      setBusy(true);
      try {
        if (await insertEmbed(editor, url.trim())) setUrl('');
      } catch (x) {
        toast((x as Error).message);
      } finally {
        setBusy(false);
      }
    }}>
      <input className="input grow" value={url} onChange={(e) => setUrl(e.target.value)} placeholder={t('Paste a YouTube, Spotify or Maps link')} aria-label={t('Embed a link')} title={t('Embed a link (YouTube, Spotify, Maps…)')} />
      <button className="btn" disabled={!ok || busy}>{t('Embed')}</button>
    </form>
  );
}
