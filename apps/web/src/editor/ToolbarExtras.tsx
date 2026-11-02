/** Toolbar sections for animation, tables, charts and freehand strokes. */
import { BORDER_PRESETS, canUnmerge, sliceColor, tableGeometry, TABLE_STYLE_PRESETS, tableStyleSwatch, type BorderPreset, type TableStylePreset, clipLengthMs, type VideoEl, CHART_COLORS, CHART_KINDS, hex, solid, toHex, type AnimPreset, ANIM_PRESETS, type AnimDirection, type AnimUnit, EMPHASIS_PRESETS, type EmphasisPreset, TRANSITION_KINDS, type TransitionKind, type ChartEl, type ChartKind, type DrawEl, type Element, type Page, type TableEl } from '@oc/engine';
import { AlignCenter, AlignLeft, AlignRight, AlignVerticalJustifyCenter, AlignVerticalJustifyEnd, AlignVerticalJustifyStart, SlidersHorizontal, Table, TableCellsMerge, TableCellsSplit, Volume2, VolumeX, Repeat, Scissors, Maximize, Film, BarChart3, Columns3, Minus, Play, Plus, Rows3, Sparkle, Trash2, Wand2 } from 'lucide-react';
import { useState } from 'react';
import { PopoverButton } from '../components/ui';
import { ColorButton } from './ColorPicker';
import { ChartDataEditor } from '../components/ChartDataEditor';
import { t, tk } from '../i18n';
import { useDoc, useEd, useEditorState } from './context';

const PRESET_LABEL: Record<AnimPreset, string> = {
  fade: tk('Fade'), rise: tk('Rise'), drop: tk('Drop'), pan: tk('Pan'), pop: tk('Pop'), zoom: tk('Zoom'), wipe: tk('Wipe'), blur: tk('Blur'), tumble: tk('Tumble'), breathe: tk('Breathe'),
  slide: tk('Slide'), flip: tk('Flip'), stomp: tk('Stomp'), spin: tk('Spin'), bounce: tk('Bounce'), float: tk('Float'), shrink: tk('Shrink'), typewriter: tk('Typewriter'),
};
const EMPHASIS_LABEL: Record<EmphasisPreset, string> = {
  pulse: tk('Pulse'), wiggle: tk('Wiggle'), bounce: tk('Bounce'), flicker: tk('Flicker'), spin: tk('Spin'), heartbeat: tk('Heartbeat'), float: tk('Float'), swing: tk('Swing'),
};
const TRANSITION_LABEL: Record<TransitionKind, string> = {
  fade: tk('Fade'), dissolve: tk('Dissolve'), slide: tk('Slide'), push: tk('Push'), cover: tk('Cover'), reveal: tk('Reveal'), zoom: tk('Zoom'), wipe: tk('Wipe'),
  iris: tk('Circle'), split: tk('Split'), flip: tk('Flip'), blur: tk('Blur'), spin: tk('Spin'), drop: tk('Drop'),
};
const DIRECTIONAL = new Set<string>(['slide', 'wipe', 'bounce', 'rise', 'push', 'cover', 'reveal', 'split']);
const DIR_LABEL: Record<AnimDirection, string> = { up: tk('From bottom'), down: tk('From top'), left: tk('From right'), right: tk('From left') };

/** Animate: element entrance (when something is selected) or page animation + transition. */
export function AnimateButton({ sel, page }: { sel: Element[]; page: Page | undefined }) {
  const { editor } = useEd();
  const forElements = sel.length > 0;
  const current = forElements ? sel[0]!.anim : page?.anim;
  const duration = current?.durationMs ?? 700;
  const setPreset = (preset: AnimPreset | null) => {
    if (forElements) {
      editor.updateSelected(() => ({ anim: preset ? { preset, durationMs: duration, delayMs: sel[0]!.anim?.delayMs ?? 0 } : null }), false, false);
    } else if (page) editor.updatePage(page.id, { anim: preset ? { preset, durationMs: duration } : undefined });
    if (preset && page) setTimeout(() => editor.previewAnimation(page.id), 30);
  };
  return (
    <PopoverButton title={t('Animate')} label={<><Wand2 size={16} /> {t('Animate')}</>} content={() => (
      <div className="col pop-md">
        <h4 style={{ margin: 0 }} className="muted">{forElements ? t('Element animation') : t('Page animation (all elements)')}</h4>
        <div style={{ display: 'grid', gridTemplateColumns: 'repeat(3, 1fr)', gap: 6 }}>
          <button className={`btn sm ${!current ? 'active' : ''}`} onClick={() => setPreset(null)}>{t('None')}</button>
          {ANIM_PRESETS.map((p) => (
            <button key={p} className={`btn sm ${current?.preset === p ? 'active' : ''}`} onClick={() => setPreset(p)}>{t(PRESET_LABEL[p])}</button>
          ))}
        </div>
        {current && (
          <label className="field">{t('Speed ({value} s)', { value: (duration / 1000).toFixed(1) })}
            <input type="range" min={200} max={3000} step={100} value={duration} onChange={(e) => {
              const d = +e.target.value;
              if (forElements) editor.updateSelected((x) => (x.anim ? { anim: { ...x.anim, durationMs: d } } : null));
              else if (page?.anim) editor.updatePage(page.id, { anim: { ...page.anim, durationMs: d } });
            }} />
          </label>
        )}
        {forElements && current && DIRECTIONAL.has(current.preset) && (
          <label className="field">{t('Direction')}
            <select className="input" value={(forElements ? sel[0]!.anim?.direction : undefined) ?? ''} onChange={(e) => {
              const direction = (e.target.value || undefined) as AnimDirection | undefined;
              if (forElements) editor.updateSelected((x) => (x.anim ? { anim: { ...x.anim, direction } } : null));
              if (page) setTimeout(() => editor.previewAnimation(page.id), 30);
            }}>
              <option value="">{t('Default')}</option>
              {(Object.keys(DIR_LABEL) as AnimDirection[]).map((d) => <option key={d} value={d}>{t(DIR_LABEL[d])}</option>)}
            </select>
          </label>
        )}
        {forElements && current && (
          <label className="field">{t('Intensity')}
            <input type="range" min={0.25} max={2} step={0.25} value={sel[0]!.anim?.intensity ?? 1} onChange={(e) => editor.updateSelected((x) => (x.anim ? { anim: { ...x.anim, intensity: +e.target.value } } : null))} />
          </label>
        )}
        {forElements && current && sel.every((x) => x.type === 'text') && (
          <label className="field">{t('Animate text')}
            <select className="input" value={sel[0]!.anim?.by ?? (current.preset === 'typewriter' ? 'char' : 'element')} onChange={(e) => {
              const by = e.target.value as AnimUnit;
              editor.updateSelected((x) => (x.anim ? { anim: { ...x.anim, by } } : null));
              if (page) setTimeout(() => editor.previewAnimation(page.id), 30);
            }}>
              <option value="element">{t('All at once')}</option>
              <option value="line">{t('By line')}</option>
              <option value="word">{t('By word')}</option>
              <option value="char">{t('By letter')}</option>
            </select>
          </label>
        )}
        {forElements && (
          <label className="field">{t('Emphasis (loops)')}
            <select className="input" value={sel[0]!.emphasis?.preset ?? ''} onChange={(e) => {
              const v = e.target.value as EmphasisPreset | '';
              editor.updateSelected(() => ({ emphasis: v ? { preset: v, periodMs: v === 'spin' ? 3000 : 1600 } : null }), false, false);
              if (page) setTimeout(() => editor.previewAnimation(page.id), 30);
            }}>
              <option value="">{t('None')}</option>
              {EMPHASIS_PRESETS.map((p) => <option key={p} value={p}>{t(EMPHASIS_LABEL[p])}</option>)}
            </select>
          </label>
        )}
        {forElements && <ExitAndPath sel={sel} />}
        {forElements && current && (
          <label className="field">{t('Start')}
            <select className="input" value={sel[0]!.anim?.onClick ?? 0} onChange={(e) => {
              const step = +e.target.value;
              editor.updateSelected((x) => (x.anim ? { anim: { ...x.anim, onClick: step || undefined } } : null));
            }}>
              <option value={0}>{t('Automatically')}</option>
              {[1, 2, 3, 4, 5, 6, 7, 8, 9].map((n) => <option key={n} value={n}>{t('On click {n}', { n })}</option>)}
            </select>
          </label>
        )}
        {forElements && current && (
          <label className="field">{t('Delay ({value} s)', { value: ((sel[0]!.anim?.delayMs ?? 0) / 1000).toFixed(1) })}
            <input type="range" min={0} max={5000} step={100} value={sel[0]!.anim?.delayMs ?? 0} onChange={(e) => editor.updateSelected((x) => (x.anim ? { anim: { ...x.anim, delayMs: +e.target.value } } : null))} />
          </label>
        )}
        {page && (
          <>
            <div className="hsep" />
            <label className="field">{t('Page transition')}
              <select className="input" value={page.transition?.kind ?? 'none'} onChange={(e) => editor.updatePage(page.id, { transition: e.target.value === 'none' ? undefined : { kind: e.target.value as TransitionKind, durationMs: page.transition?.durationMs ?? 700, ...(page.transition?.direction ? { direction: page.transition.direction } : {}) } })}>
                <option value="none">{t('None')}</option>
                {TRANSITION_KINDS.map((k) => <option key={k} value={k}>{t(TRANSITION_LABEL[k])}</option>)}
              </select>
            </label>
            {page.transition && DIRECTIONAL.has(page.transition.kind) && (
              <label className="field">{t('Direction')}
                <select className="input" value={page.transition.direction ?? 'left'} onChange={(e) => editor.updatePage(page.id, { transition: { ...page.transition!, direction: e.target.value as AnimDirection } })}>
                  {(Object.keys(DIR_LABEL) as AnimDirection[]).map((d) => <option key={d} value={d}>{t(DIR_LABEL[d])}</option>)}
                </select>
              </label>
            )}
            {page.transition && (
              <label className="field">{t('Transition speed ({value} s)', { value: (page.transition.durationMs / 1000).toFixed(1) })}
                <input type="range" min={200} max={2000} step={100} value={page.transition.durationMs} onChange={(e) => editor.updatePage(page.id, { transition: { ...page.transition!, durationMs: +e.target.value } })} />
              </label>
            )}
            <label className="field">{t('Page duration ({value} s, for video export)', { value: ((page.durationMs ?? 5000) / 1000).toFixed(1) })}
              <input type="range" min={1000} max={30000} step={500} value={page.durationMs ?? 5000} onChange={(e) => editor.updatePage(page.id, { durationMs: +e.target.value })} />
            </label>
            <button className="btn sm" onClick={() => editor.previewAnimation(page.id)}><Play size={14} /> {t('Preview page')}</button>
          </>
        )}
      </div>
    )} />
  );
}

const TABLE_STYLE_LABEL: Record<TableStylePreset, string> = {
  minimal: tk('Minimal'), striped: tk('Striped'), 'header-dark': tk('Dark header'), bordered: tk('Bordered'), card: tk('Card'), colourful: tk('Colourful'),
};
const BORDER_LABEL: Record<BorderPreset, string> = {
  all: tk('All borders'), outer: tk('Outer borders'), inner: tk('Inner borders'), horizontal: tk('Horizontal borders'), none: tk('No borders'),
};

/** 2×2 grid glyph with the lines a border preset draws highlighted. */
function BorderIcon({ preset }: { preset: BorderPreset }) {
  const on = (outer: boolean, horizontal: boolean) =>
    preset === 'all' || (preset === 'outer' && outer) || (preset === 'inner' && !outer) || (preset === 'horizontal' && horizontal);
  const line = (x1: number, y1: number, x2: number, y2: number, outer: boolean) => {
    const lit = on(outer, y1 === y2);
    return <line key={`${x1}${y1}${x2}${y2}`} x1={x1} y1={y1} x2={x2} y2={y2} stroke="currentColor" strokeWidth={lit ? 2 : 1} strokeDasharray={lit ? undefined : '1.5 2'} opacity={lit ? 1 : 0.45} />;
  };
  return (
    <svg viewBox="0 0 20 20" width={18} height={18} aria-hidden>
      {line(2, 2, 18, 2, true)}{line(2, 18, 18, 18, true)}{line(2, 2, 2, 18, true)}{line(18, 2, 18, 18, true)}
      {line(2, 10, 18, 10, false)}{line(10, 2, 10, 18, false)}
    </svg>
  );
}

function TableStyleTile({ preset }: { preset: TableStylePreset }) {
  const sw = tableStyleSwatch(preset);
  return (
    <svg viewBox="0 0 48 30" width={72} height={45} aria-hidden>
      {[0, 1, 2].map((r) => <rect key={r} x={1} y={1 + r * 9.33} width={46} height={9.33} fill={r === 0 ? sw.header : r === 2 ? sw.stripe : sw.body} />)}
      <rect x={1} y={1} width={46} height={28} fill="none" stroke={sw.border} strokeWidth={1} />
      {[1, 2].map((r) => <line key={r} x1={1} x2={47} y1={1 + r * 9.33} y2={1 + r * 9.33} stroke={sw.border} strokeWidth={0.8} />)}
    </svg>
  );
}

export function TableControls({ el }: { el: TableEl }) {
  const { editor } = useEd();
  const doc = useDoc();
  const state = useEditorState();
  const cell = state.editingCell?.id === el.id ? state.editingCell : null;
  const range = state.cellRange?.id === el.id ? state.cellRange : null;
  const r = cell?.r ?? el.rows.length - 1;
  const c = cell?.c ?? el.cols.length - 1;
  const st = el.style;
  const setStyle = (patch: Partial<TableEl['style']>) => editor.updateSelected({ style: { ...st, ...patch } });
  const target = editor.tableTargetRange(el.id);
  const first = target ? el.cells[`${target.r0}:${target.c0}`] : undefined;
  const scope = range ? t('Selected cells') : cell ? t('Cell') : t('All cells');
  const valign = first?.valign ?? 'middle';
  const halign = first?.body?.paragraphs[0]?.align ?? first?.align ?? 'start';
  const VIcon = { top: AlignVerticalJustifyStart, middle: AlignVerticalJustifyCenter, bottom: AlignVerticalJustifyEnd }[valign];
  const HIcon = { start: AlignLeft, center: AlignCenter, end: AlignRight, justify: AlignLeft }[halign];
  const nextV = { top: 'middle', middle: 'bottom', bottom: 'top' } as const;
  const nextH = { start: 'center', center: 'end', end: 'start', justify: 'start' } as const;
  const { xs } = tableGeometry(el);
  const colW = Math.round(xs[c + 1]! - xs[c]!);
  const [borderW, setBorderW] = useState(st.borderWidth || 1.5);
  const [borderC, setBorderC] = useState(st.borderColor);
  return (
    <div className="group">
      <PopoverButton title={t('Table style')} label={<><Table size={15} /> {t('Style')}</>} content={(close) => (
        <div className="pop-md" style={{ display: 'grid', gridTemplateColumns: 'repeat(3, 1fr)', gap: 8 }}>
          {TABLE_STYLE_PRESETS.map((p) => (
            <button key={p} className={`btn sm ${st.preset === p ? 'active' : ''}`} style={{ flexDirection: 'column', height: 'auto', padding: 6, gap: 4 }}
              onClick={() => { editor.setTableStyle(el.id, p); close(); }}>
              <TableStyleTile preset={p} />
              <span style={{ fontSize: 11 }}>{t(TABLE_STYLE_LABEL[p])}</span>
            </button>
          ))}
        </div>
      )} />
      <button className="btn ghost sm" title={t('Add row below')} onClick={() => editor.tableStructure(el.id, 'addRow', r)}><Rows3 size={15} /> + {t('Row')}</button>
      <button className="btn ghost sm" title={t('Add column right')} onClick={() => editor.tableStructure(el.id, 'addCol', c)}><Columns3 size={15} /> + {t('Col')}</button>
      <button className="btn ghost sm icon" title={t('Delete row')} onClick={() => editor.tableStructure(el.id, 'delRow', r)} disabled={el.rows.length < 2}><Rows3 size={15} />−</button>
      <button className="btn ghost sm icon" title={t('Delete column')} onClick={() => editor.tableStructure(el.id, 'delCol', c)} disabled={el.cols.length < 2}><Columns3 size={15} />−</button>
      <button className="btn ghost sm icon" title={t('Merge cells (select cells with Shift+click or drag)')} disabled={!range} onClick={() => editor.mergeCellRange(el.id)}><TableCellsMerge size={16} /></button>
      <button className="btn ghost sm icon" title={t('Unmerge cells')} disabled={!target || !canUnmerge(el, target)} onClick={() => editor.unmergeCellRange(el.id)}><TableCellsSplit size={16} /></button>
      <div className="sep" />
      <div className="size-box" title={t('Font size')}>
        <button onClick={() => setStyle({ size: Math.max(6, st.size - 2) })} aria-label={t('Smaller')}><Minus size={14} /></button>
        <input value={st.size} onChange={(e) => +e.target.value > 0 && setStyle({ size: +e.target.value })} aria-label={t('Table font size')} />
        <button onClick={() => setStyle({ size: st.size + 2 })} aria-label={t('Larger')}><Plus size={14} /></button>
      </div>
      <ColorButton variant="text" title={t('Text colour')} doc={doc} value={solid(st.color)} onChange={(p) => p.k === 'solid' && setStyle({ color: p.color })} />
      <ColorButton title={t('Header fill')} doc={doc} value={st.headerFill} allowTransparent onChange={(p) => setStyle({ headerFill: p })} />
      <button className={`btn ghost sm ${st.stripeFill ? 'active' : ''}`} onClick={() => setStyle({ stripeFill: st.stripeFill ? undefined : solid('#f5f3ff') })}>{t('Stripes')}</button>
      <div className="sep" />
      <span className="faint" style={{ fontSize: 12 }}>{scope}</span>
      <ColorButton title={t('Cell fill')} doc={doc} value={first?.fill} allowTransparent allowGradient onChange={(p) => editor.setCells(el.id, { fill: p })} />
      <button className={`btn ghost sm icon ${first?.bold ? 'active' : ''}`} title={t('Bold cell')} onClick={() => editor.setCells(el.id, { bold: !first?.bold })}><b>B</b></button>
      <button className="btn ghost sm icon" title={t('Horizontal alignment')} onClick={() => editor.setCellAlign(el.id, nextH[halign])}><HIcon size={16} /></button>
      <button className="btn ghost sm icon" title={t('Vertical alignment')} onClick={() => editor.setCells(el.id, { valign: nextV[valign] })}><VIcon size={16} /></button>
      <PopoverButton title={t('Borders')} label={<><BorderIcon preset="all" /> {t('Borders')}</>} content={() => (
        <div className="col pop-sm">
          <div style={{ display: 'grid', gridTemplateColumns: 'repeat(5, 1fr)', gap: 4 }}>
            {BORDER_PRESETS.map((p) => (
              <button key={p} className="btn ghost sm icon" title={t(BORDER_LABEL[p])} aria-label={t(BORDER_LABEL[p])}
                onClick={() => editor.setCellBorders(el.id, p, { width: borderW, color: borderC })}><BorderIcon preset={p} /></button>
            ))}
          </div>
          <label className="field">{t('Border width ({value})', { value: borderW })}
            <input type="range" min={0.5} max={8} step={0.5} value={borderW} onChange={(e) => setBorderW(+e.target.value)} />
          </label>
          <div className="row" style={{ gap: 8, alignItems: 'center' }}>
            <span className="faint" style={{ fontSize: 12 }}>{t('Border colour')}</span>
            <ColorButton title={t('Border colour')} doc={doc} value={solid(borderC)} onChange={(p) => p.k === 'solid' && setBorderC(p.color)} />
          </div>
          <button className="btn sm" onClick={() => setStyle({ borderColor: borderC, borderWidth: borderW })}>{t('Use for the whole table grid')}</button>
        </div>
      )} />
      <PopoverButton title={t('Cell spacing and column width')} label={<><SlidersHorizontal size={15} /> {t('Layout')}</>} content={() => (
        <div className="col pop-sm">
          <label className="field">{t('Cell padding ({value})', { value: first?.padding ?? st.padding })}
            <input type="range" min={0} max={40} step={1} value={first?.padding ?? st.padding} onChange={(e) => editor.setCells(el.id, { padding: +e.target.value })} />
          </label>
          <label className="field">{t('Column {n} width (px)', { n: c + 1 })}
            <input className="input" type="number" min={16} value={colW} onChange={(e) => +e.target.value >= 16 && editor.setColumnWidth(el.id, c, +e.target.value)} />
          </label>
          <button className="btn sm" onClick={() => editor.setColumnWidth(el.id, 'even')}>{t('Distribute columns evenly')}</button>
          <span className="faint" style={{ fontSize: 12 }}>{t('Tip: drag a column border on the canvas to resize it. Rows grow to fit their text.')}</span>
        </div>
      )} />
      {!cell && !range && <span className="faint ctx-tip" style={{ fontSize: 12 }}>{t('Double-click a cell to type')}</span>}
    </div>
  );
}

const CHART_LABEL: Record<ChartKind, string> = {
  column: tk('Column'), bar: tk('Bar'), 'stacked-column': tk('Stacked column'), 'stacked-bar': tk('Stacked bar'), line: tk('Line'), area: tk('Area'), combo: tk('Column + line'),
  pie: tk('Pie'), donut: tk('Donut'), scatter: tk('Scatter'), bubble: tk('Bubble'), radar: tk('Radar'), funnel: tk('Funnel'),
  'progress-ring': tk('Progress ring'), 'progress-bar': tk('Progress bar'), gauge: tk('Gauge'),
};

export function ChartControls({ el }: { el: ChartEl }) {
  const { editor, readOnly } = useEd();
  const doc = useDoc();
  const set = (patch: Partial<ChartEl>) => editor.updateSelected(patch as Record<string, unknown>, false, false);
  const st = el.style;
  return (
    <div className="group">
      <select className="input" style={{ width: 110 }} value={el.kind} onChange={(e) => set({ kind: e.target.value as ChartKind })} aria-label={t('Chart type')}>
        {CHART_KINDS.map((k) => <option key={k} value={k}>{t(CHART_LABEL[k])}</option>)}
      </select>
      <PopoverButton title={t('Edit data')} label={<><BarChart3 size={16} /> {t('Data')}</>} content={() => <ChartData el={el} />} />
      <PopoverButton title={t('Number format')} label={<>{t('Format')}</>} content={() => (
        <div className="col pop-sm">
          <label className="field">{t('Prefix')}<input className="input" value={st.format?.prefix ?? ''} placeholder="$" onChange={(e) => set({ style: { ...st, format: { ...st.format, prefix: e.target.value } } })} /></label>
          <label className="field">{t('Suffix')}<input className="input" value={st.format?.suffix ?? ''} placeholder="%" onChange={(e) => set({ style: { ...st, format: { ...st.format, suffix: e.target.value } } })} /></label>
          <label className="field">{t('Decimals')}<input className="input" type="number" min={0} max={4} value={st.format?.decimals ?? ''} onChange={(e) => set({ style: { ...st, format: { ...st.format, decimals: e.target.value === '' ? undefined : Math.max(0, Math.min(4, +e.target.value)) } } })} /></label>
          <label className="row" style={{ gap: 6 }}><input type="checkbox" checked={st.format?.thousands !== false} onChange={(e) => set({ style: { ...st, format: { ...st.format, thousands: e.target.checked } } })} />{t('Thousands separators')}</label>
          {(el.kind === 'progress-ring' || el.kind === 'progress-bar' || el.kind === 'gauge') && (
            <label className="field">{t('Maximum')}<input className="input" type="number" value={st.max ?? 100} onChange={(e) => set({ style: { ...st, max: Math.max(1e-6, +e.target.value || 100) } })} /></label>
          )}
        </div>
      )} />
      <button className={`btn ghost sm ${st.showLegend ? 'active' : ''}`} onClick={() => set({ style: { ...st, showLegend: !st.showLegend } })}>{t('Legend')}</button>
      <button className={`btn ghost sm ${st.showValues ? 'active' : ''}`} onClick={() => set({ style: { ...st, showValues: !st.showValues } })}>{t('Values')}</button>
      <button className={`btn ghost sm ${st.showGrid ? 'active' : ''}`} onClick={() => set({ style: { ...st, showGrid: !st.showGrid } })}>{t('Grid')}</button>
      <ColorButton variant="text" title={t('Label colour')} doc={doc} value={solid(st.textColor)} onChange={(p) => p.k === 'solid' && set({ style: { ...st, textColor: p.color } })} />
      <div className="size-box" title={t('Label size')}>
        <button onClick={() => set({ style: { ...st, fontSize: Math.max(8, st.fontSize - 2) } })} aria-label={t('Smaller labels')}><Minus size={14} /></button>
        <input value={st.fontSize} onChange={(e) => +e.target.value > 0 && set({ style: { ...st, fontSize: +e.target.value } })} aria-label={t('Label size')} />
        <button onClick={() => set({ style: { ...st, fontSize: st.fontSize + 2 } })} aria-label={t('Larger labels')}><Plus size={14} /></button>
      </div>
    </div>
  );
}

/** Chart data for the selected chart (the shared spreadsheet editor). */
function ChartData({ el }: { el: ChartEl }) {
  const { editor } = useEd();
  return (
    <ChartDataEditor value={el} sliceColorAt={(i) => sliceColor(el, i)}
      onCommit={(d) => editor.apply([{ op: 'update', id: el.id, patch: d }], true)}
      onSliceColor={(cols) => editor.apply([{ op: 'update', id: el.id, patch: { style: { ...el.style, sliceColors: cols } } }], true)} />
  );
}

export function DrawControls({ el }: { el: DrawEl }) {
  const { editor } = useEd();
  const doc = useDoc();
  return (
    <div className="group">
      <Sparkle size={15} className="faint" />
      <ColorButton title={t('Stroke colour')} doc={doc} value={solid(el.color)} onChange={(p) => p.k === 'solid' && editor.updateSelected({ color: p.color })} />
      <label className="row" style={{ fontSize: 13 }}>{t('Weight')}
        <input type="range" min={1} max={40} value={el.size} onChange={(e) => editor.updateSelected({ size: +e.target.value })} />
      </label>
      <select className="input" style={{ width: 120 }} value={el.brush} onChange={(e) => editor.updateSelected({ brush: e.target.value }, false, false)} aria-label={t('Brush')}>
        <option value="pen">{t('Pen')}</option><option value="marker">{t('Marker')}</option><option value="highlighter">{t('Highlighter')}</option>
      </select>
    </div>
  );
}

const SPEEDS = [0.25, 0.5, 0.75, 1, 1.25, 1.5, 2, 3];

export function VideoControls({ el }: { el: VideoEl }) {
  const { editor } = useEd();
  const set = (clip: Partial<VideoEl['clip']>, merge = true) => editor.updateSelected({ clip: { ...el.clip, ...clip } }, false, merge);
  const dur = editor.doc.assets[el.asset]?.durationMs ?? el.clip.trimEndMs;
  return (
    <div className="group">
      <Film size={15} className="faint" />
      <PopoverButton title={t('Trim')} label={<><Scissors size={15} /> {t('Trim')}</>} content={() => (
        <div className="col pop-md">
          <label className="field">{t('Start ({value} s)', { value: (el.clip.trimStartMs / 1000).toFixed(1) })}
            <input type="range" min={0} max={Math.max(0, el.clip.trimEndMs - 200)} step={50} value={el.clip.trimStartMs} onChange={(e) => set({ trimStartMs: +e.target.value })} />
          </label>
          <label className="field">{t('End ({value} s of {total} s)', { value: (el.clip.trimEndMs / 1000).toFixed(1), total: (dur / 1000).toFixed(1) })}
            <input type="range" min={el.clip.trimStartMs + 200} max={dur} step={50} value={el.clip.trimEndMs} onChange={(e) => set({ trimEndMs: +e.target.value })} />
          </label>
          <span className="faint" style={{ fontSize: 12 }}>{t('Plays {value} s on the page. Use Split in the timeline to cut at the playhead.', { value: (clipLengthMs(el.clip) / 1000).toFixed(1) })}</span>
        </div>
      )} />
      <select className="input" style={{ width: 84 }} value={el.clip.speed} onChange={(e) => set({ speed: +e.target.value }, false)} aria-label={t('Playback speed')}>
        {SPEEDS.map((s) => <option key={s} value={s}>{s}×</option>)}
      </select>
      <button className={`btn ghost sm icon ${el.clip.muted ? 'active' : ''}`} title={el.clip.muted ? t('Unmute') : t('Mute')} onClick={() => set({ muted: !el.clip.muted }, false)}>{el.clip.muted ? <VolumeX size={16} /> : <Volume2 size={16} />}</button>
      {!el.clip.muted && <input type="range" min={0} max={1} step={0.05} value={el.clip.volume} onChange={(e) => set({ volume: +e.target.value })} style={{ width: 70 }} aria-label={t('Volume')} />}
      <button className={`btn ghost sm icon ${el.clip.loop ? 'active' : ''}`} title={t('Loop')} onClick={() => set({ loop: !el.clip.loop }, false)}><Repeat size={16} /></button>
      <button className="btn ghost sm" title={t('Fill the page (background video)')} onClick={() => {
        const { w, h } = editor.doc.meta.dims;
        const a = editor.doc.assets[el.asset];
        const aspect = a?.w && a.h ? a.w / a.h : el.xf.w / el.xf.h;
        const crop = aspect > w / h ? { x: (1 - (w / h) / aspect) / 2, y: 0, w: (w / h) / aspect, h: 1 } : { x: 0, y: (1 - aspect / (w / h)) / 2, w: 1, h: aspect / (w / h) };
        editor.apply([{ op: 'update', id: el.id, patch: { crop, xf: { x: 0, y: 0, w, h, rot: 0 } } }, { op: 'reorder', id: el.id, after: 'bottom' }]);
      }}><Maximize size={15} /> {t('Fill page')}</button>
      <PopoverButton title={t('Rounded corners')} label="◜" content={() => (
        <label className="field" style={{ width: 220 }}>{t('Corner radius')}
          <input type="range" min={0} max={Math.min(el.xf.w, el.xf.h) / 2} value={el.cornerRadius ?? 0} onChange={(e) => editor.updateSelected({ cornerRadius: +e.target.value })} />
        </label>
      )} />
    </div>
  );
}

/** Exit animation + motion path, shown inside Animate for elements. */
export function ExitAndPath({ sel }: { sel: Element[] }) {
  const { editor } = useEd();
  const el = sel[0]!;
  return (
    <>
      <div className="hsep" />
      <label className="field">{t('Exit animation')}
        <select className="input" value={el.animOut?.preset ?? 'none'} onChange={(e) => editor.updateSelected(() => ({ animOut: e.target.value === 'none' ? null : { preset: e.target.value, durationMs: el.animOut?.durationMs ?? 500 } }), false, false)}>
          <option value="none">{t('None')}</option>
          {ANIM_PRESETS.map((p) => <option key={p} value={p}>{t(PRESET_LABEL[p])}</option>)}
        </select>
      </label>
      <label className="field">{t('Motion path')}
        <select className="input" value={el.motionPath ? pathKind(el.motionPath.points) : 'none'} onChange={(e) => {
          const k = e.target.value;
          const w = Math.max(80, el.xf.w);
          const pts: Record<string, number[]> = {
            right: [-w, 0, 0, 0], left: [w, 0, 0, 0], up: [0, w * 0.6, 0, 0], arc: [-w, w * 0.5, -w * 0.5, -w * 0.2, 0, 0],
            zigzag: [-w, 0, -w * 0.66, -w * 0.25, -w * 0.33, w * 0.25, 0, 0], circle: Array.from({ length: 25 }, (_, i) => { const a = (i / 24) * Math.PI * 2; return [Math.cos(a) * w * 0.3 - w * 0.3, Math.sin(a) * w * 0.3]; }).flat(),
          };
          editor.updateSelected(() => ({ motionPath: k === 'none' ? null : { points: pts[k]!, durationMs: 1500, delayMs: 0, easing: 'ease' } }), false, false);
        }}>
          <option value="none">{t('None')}</option><option value="right">{t('Slide in from left')}</option><option value="left">{t('Slide in from right')}</option><option value="up">{t('Float up')}</option>
          <option value="arc">{t('Arc')}</option><option value="zigzag">{t('Zig-zag')}</option><option value="circle">{t('Loop around')}</option>
        </select>
      </label>
    </>
  );
}

function pathKind(pts: number[]): string {
  if (pts.length === 4) return pts[0]! < 0 ? 'right' : pts[0]! > 0 ? 'left' : 'up';
  if (pts.length === 6) return 'arc';
  if (pts.length === 8) return 'zigzag';
  return 'circle';
}
