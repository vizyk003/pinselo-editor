/**
 * Doc blocks (DC-030): Tiptap nodes for embedded designs, charts, callouts, a table of contents, page
 * breaks and video embeds. Attribute names and defaults are shared with the export pipeline through
 * @oc/schema (docBlocks), so what the editor stores is exactly what PDF/DOCX/Markdown read.
 */
import {
  CALLOUT_TONES, CHART_KINDS, DOC_CONTENT_WIDTH, EMBED_FRAME_HOSTS, calloutTone, docBlockWidth, docChartElement, emptyDesign, parseDocChart, parseEmbedUrl, placeAll,
  resolvePage, serializeDocChart, sliceColor, sortedPages, type CalloutTone, type ChartKind, type DocChartData,
} from '@oc/engine';
import { Node, NodeViewWrapper, ReactNodeViewRenderer, mergeAttributes, type NodeViewProps } from '@tiptap/react';
import { BarChart3, ExternalLink, Lock, RefreshCw } from 'lucide-react';
import { useEffect, useRef, useState } from 'react';
import { api, thumbUrl, type DesignSummary } from '../api';
import { ChartDataEditor } from '../components/ChartDataEditor';
import { Modal, PopoverButton } from '../components/ui';
import { t, tk } from '../i18n';
import { WHITE, loadEngine, paintCanvas } from '../lib/engine';

declare module '@tiptap/core' {
  interface Commands<ReturnType> {
    docBlocks: {
      insertDesignEmbed: (attrs: { designId: string; pageId?: string | null; title?: string; width?: number }) => ReturnType;
      insertDocChart: (kind?: ChartKind) => ReturnType;
      toggleCallout: (tone?: CalloutTone) => ReturnType;
      setCalloutTone: (tone: CalloutTone) => ReturnType;
      insertTableOfContents: () => ReturnType;
      insertPageBreak: () => ReturnType;
      insertVideoEmbed: (url: string) => ReturnType;
    };
  }
}

const dataAttr = (name: string, def: unknown, num = false) => ({
  default: def,
  parseHTML: (el: HTMLElement) => {
    const v = el.getAttribute(`data-${name}`);
    return v === null ? def : num ? Number(v) || def : v;
  },
  renderHTML: (a: Record<string, unknown>) => (a[name] === null || a[name] === undefined ? {} : { [`data-${name}`]: String(a[name]) }),
});

// ------------------------------------------------------------------ resize handle

function useResize(width: number, onDone: (w: number, dy: number) => void) {
  const [live, setLive] = useState<{ w: number; dy: number } | null>(null);
  const start = (e: React.PointerEvent) => {
    e.preventDefault();
    e.stopPropagation();
    const x0 = e.clientX;
    const y0 = e.clientY;
    const move = (ev: PointerEvent) => setLive({ w: Math.max(120, Math.min(DOC_CONTENT_WIDTH, width + (ev.clientX - x0))), dy: ev.clientY - y0 });
    const up = (ev: PointerEvent) => {
      window.removeEventListener('pointermove', move);
      window.removeEventListener('pointerup', up);
      setLive(null);
      onDone(Math.round(Math.max(120, Math.min(DOC_CONTENT_WIDTH, width + (ev.clientX - x0)))), ev.clientY - y0);
    };
    window.addEventListener('pointermove', move);
    window.addEventListener('pointerup', up);
  };
  return { live, handle: <span className="doc-resize" onPointerDown={start} role="separator" aria-label={t('Resize')} title={t('Drag to resize')} /> };
}

// ------------------------------------------------------------------ embedded design

function DesignEmbedView({ node, updateAttributes, selected, editor }: NodeViewProps) {
  const designId = String(node.attrs.designId ?? '');
  const pageId = node.attrs.pageId ? String(node.attrs.pageId) : '1';
  const width = docBlockWidth(node.attrs.width);
  const [info, setInfo] = useState<(DesignSummary & { role: string }) | null | 'locked'>(null);
  const [broken, setBroken] = useState(false);
  const [bump, setBump] = useState(0);
  const { live, handle } = useResize(width, (w) => updateAttributes({ width: w }));
  useEffect(() => {
    let off = false;
    const load = () => api.design(designId).then((r) => !off && (setInfo(r.design), setBroken(false))).catch(() => !off && setInfo('locked'));
    void load();
    // Live: re-check (and so re-render) when the tab regains focus.
    const onFocus = () => void load();
    window.addEventListener('focus', onFocus);
    return () => { off = true; window.removeEventListener('focus', onFocus); };
  }, [designId, bump]);
  const locked = info === 'locked' || broken;
  const w = live?.w ?? width;
  const open = () => window.open(`/design/${designId}`, '_blank', 'noopener');
  const pages = info && info !== 'locked' ? Math.max(1, info.page_count || 1) : 1;
  return (
    <NodeViewWrapper className={`doc-block doc-embed ${selected ? 'on' : ''}`} data-drag-handle>
      <div className="doc-block-frame" style={{ width: w }} onDoubleClick={open} onClick={() => !editor.isEditable && open()}>
        {locked ? (
          <div className="doc-locked" role="img" aria-label={t('Embedded design you can’t view')}><Lock size={18} /> {t('You don’t have access to this design')}</div>
        ) : info ? (
          <img src={`/api/designs/${designId}/pages/${encodeURIComponent(pageId)}.png?w=${Math.round(width * 2)}&v=${encodeURIComponent(info.updated_at)}`} alt={info.title || t('Embedded design')} draggable={false} onError={() => setBroken(true)} style={{ width: '100%', display: 'block' }} />
        ) : <div className="doc-locked"><div className="spinner" /></div>}
        {editor.isEditable && handle}
      </div>
      {selected && editor.isEditable && (
        <div className="doc-block-bar" contentEditable={false}>
          {!locked && info && <span className="faint" style={{ fontSize: 12, maxWidth: 200, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{info.title}</span>}
          {pages > 1 && (
            <select className="input" style={{ width: 110, height: 28 }} value={pageId} aria-label={t('Page')} onChange={(e) => updateAttributes({ pageId: e.target.value })}>
              {Array.from({ length: pages }, (_, i) => <option key={i} value={String(i + 1)}>{t('Page {n}', { n: i + 1 })}</option>)}
            </select>
          )}
          <button className="btn ghost sm" onClick={() => setBump((n) => n + 1)} title={t('Refresh')}><RefreshCw size={14} /></button>
          <button className="btn ghost sm" onClick={open}><ExternalLink size={14} /> {t('Open design')}</button>
        </div>
      )}
    </NodeViewWrapper>
  );
}

export const DesignEmbed = Node.create({
  name: 'designEmbed',
  group: 'block',
  atom: true,
  draggable: true,
  selectable: true,
  addAttributes() {
    return { designId: dataAttr('design-id', null), pageId: dataAttr('page-id', null), width: dataAttr('width', DOC_CONTENT_WIDTH, true), title: dataAttr('title', '') };
  },
  parseHTML() { return [{ tag: 'div[data-design-embed]' }]; },
  renderHTML({ HTMLAttributes }) { return ['div', mergeAttributes(HTMLAttributes, { 'data-design-embed': '' })]; },
  addNodeView() { return ReactNodeViewRenderer(DesignEmbedView); },
  addCommands() {
    return {
      insertDesignEmbed: (attrs) => ({ commands }) => commands.insertContent({ type: this.name, attrs: { width: DOC_CONTENT_WIDTH, pageId: '1', ...attrs } }),
    };
  },
});

/** Search my designs and pick one to embed. */
export function DesignPicker({ exclude, onPick, onClose }: { exclude: string; onPick: (d: DesignSummary) => void; onClose: () => void }) {
  const [q, setQ] = useState('');
  const [list, setList] = useState<DesignSummary[] | null>(null);
  useEffect(() => {
    const h = setTimeout(() => {
      api.designs({ filter: 'all', ...(q.trim() ? { q: q.trim() } : {}) }).then((r) => setList(r.designs.filter((d) => d.id !== exclude && !d.trashed_at))).catch(() => setList([]));
    }, 200);
    return () => clearTimeout(h);
  }, [q, exclude]);
  return (
    <Modal title={t('Embed a design')} onClose={onClose} size="lg">
      <input className="input" autoFocus placeholder={t('Search your designs')} value={q} onChange={(e) => setQ(e.target.value)} aria-label={t('Search your designs')} />
      <div className="doc-picker">
        {!list ? <div className="spinner" /> : !list.length ? <p className="faint">{t('No designs found.')}</p> : list.map((d) => (
          <button key={d.id} className="doc-picker-item" onClick={() => onPick(d)}>
            <img src={thumbUrl(d.id, d.updated_at)} alt="" loading="lazy" onError={(e) => ((e.target as HTMLImageElement).style.visibility = 'hidden')} />
            <span>{d.title || t('Untitled')}</span>
          </button>
        ))}
      </div>
    </Modal>
  );
}

// ------------------------------------------------------------------ chart

/** Draws a doc chart with the shared engine (oc-render: same code as the editor and exports). */
function ChartCanvas({ data, w, h }: { data: DocChartData; w: number; h: number }) {
  const ref = useRef<HTMLCanvasElement>(null);
  useEffect(() => {
    let off = false;
    let unsub: (() => void) | undefined;
    void loadEngine().then((engine) => {
      const draw = () => {
        const c = ref.current;
        if (off || !c) return;
        const dpr = Math.min(2, window.devicePixelRatio || 1);
        const pw = Math.max(1, Math.round(w * dpr));
        const ph = Math.max(1, Math.round(h * dpr));
        const blank = emptyDesign({ w, h, pages: 1 });
        const page = sortedPages(blank)[0]!;
        // A new document object: the engine parses each document object once (blank's parse has no chart).
        const els = placeAll(page.id, [docChartElement(data, { x: 8, y: 8, w: w - 16, h: h - 16, rot: 0 })]);
        const d = { ...blank, elements: { ...blank.elements, ...Object.fromEntries(els.map((e) => [e.id, e])) } };
        paintCanvas(engine, c, pw, ph, WHITE, resolvePage(d, page.id), dpr);
      };
      draw();
      // Fonts load lazily: redraw when the painter has new glyphs.
      unsub = engine.painter.onInvalidate(draw);
    });
    return () => { off = true; unsub?.(); };
  }, [data, w, h]);
  return <canvas ref={ref} style={{ width: w, height: h, display: 'block' }} role="img" aria-label={t('Chart: {names}', { names: data.series.map((s) => s.name).join(', ') })} />;
}

const CHART_LABEL: Record<ChartKind, string> = {
  column: tk('Column'), bar: tk('Bar'), 'stacked-column': tk('Stacked column'), 'stacked-bar': tk('Stacked bar'), line: tk('Line'), area: tk('Area'), combo: tk('Column + line'),
  pie: tk('Pie'), donut: tk('Donut'), scatter: tk('Scatter'), bubble: tk('Bubble'), radar: tk('Radar'), funnel: tk('Funnel'),
  'progress-ring': tk('Progress ring'), 'progress-bar': tk('Progress bar'), gauge: tk('Gauge'),
};

function ChartView({ node, updateAttributes, selected, editor }: NodeViewProps) {
  const raw = String(node.attrs.data ?? '');
  const [data, setData] = useState(() => parseDocChart(raw));
  useEffect(() => setData(parseDocChart(raw)), [raw]);
  const width = docBlockWidth(node.attrs.width);
  const height = Math.max(120, Math.min(900, Number(node.attrs.height) || 360));
  const ratio = height / width;
  const { live, handle } = useResize(width, (w, dy) => updateAttributes({ width: w, height: Math.round(Math.max(120, Math.min(900, w * ratio + dy))) }));
  const w = live?.w ?? width;
  const h = live ? Math.max(120, Math.min(900, w * ratio + live.dy)) : height;
  const save = (d: DocChartData) => updateAttributes({ data: serializeDocChart(d) });
  const st = data.style;
  return (
    <NodeViewWrapper className={`doc-block doc-chart ${selected ? 'on' : ''}`} data-drag-handle>
      <div className="doc-block-frame" style={{ width: w }}>
        <ChartCanvas data={data} w={Math.round(w)} h={Math.round(h)} />
        {editor.isEditable && handle}
      </div>
      {selected && editor.isEditable && (
        <div className="doc-block-bar" contentEditable={false}>
          <BarChart3 size={15} className="faint" />
          <select className="input" style={{ width: 130, height: 28 }} value={data.kind} aria-label={t('Chart type')} onChange={(e) => save({ ...data, kind: e.target.value as ChartKind })}>
            {CHART_KINDS.map((k) => <option key={k} value={k}>{t(CHART_LABEL[k])}</option>)}
          </select>
          <PopoverButton title={t('Edit data')} label={<>{t('Data')}</>} content={() => (
            <ChartDataEditor value={data}
              sliceColorAt={(i) => sliceColor(docChartElement(data, { x: 0, y: 0, w: 1, h: 1, rot: 0 }), i)}
              onCommit={(d) => save({ ...data, ...d })}
              onSliceColor={(cols) => save({ ...data, style: { ...st, sliceColors: cols } })} />
          )} />
          <button className={`btn ghost sm ${st.showLegend ? 'active' : ''}`} onClick={() => save({ ...data, style: { ...st, showLegend: !st.showLegend } })}>{t('Legend')}</button>
          <button className={`btn ghost sm ${st.showValues ? 'active' : ''}`} onClick={() => save({ ...data, style: { ...st, showValues: !st.showValues } })}>{t('Values')}</button>
          <button className={`btn ghost sm ${st.showGrid ? 'active' : ''}`} onClick={() => save({ ...data, style: { ...st, showGrid: !st.showGrid } })}>{t('Grid')}</button>
        </div>
      )}
    </NodeViewWrapper>
  );
}

export const DocChart = Node.create({
  name: 'chart',
  group: 'block',
  atom: true,
  draggable: true,
  selectable: true,
  addAttributes() {
    return { data: dataAttr('chart', ''), width: dataAttr('width', DOC_CONTENT_WIDTH, true), height: dataAttr('height', 360, true) };
  },
  parseHTML() { return [{ tag: 'div[data-doc-chart]' }]; },
  renderHTML({ HTMLAttributes }) { return ['div', mergeAttributes(HTMLAttributes, { 'data-doc-chart': '' })]; },
  addNodeView() { return ReactNodeViewRenderer(ChartView); },
  addCommands() {
    return {
      insertDocChart: (kind = 'column') => ({ commands }) => {
        const d = parseDocChart(null);
        return commands.insertContent({ type: this.name, attrs: { data: serializeDocChart({ ...d, kind }), width: DOC_CONTENT_WIDTH, height: 360 } });
      },
    };
  },
});

// ------------------------------------------------------------------ callout

export const Callout = Node.create({
  name: 'callout',
  group: 'block',
  content: 'paragraph+',
  defining: true,
  addAttributes() {
    return {
      tone: {
        default: 'info',
        parseHTML: (el: HTMLElement) => calloutTone(el.getAttribute('data-callout')),
        renderHTML: (a: Record<string, unknown>) => {
          const tone = CALLOUT_TONES[calloutTone(a.tone)];
          return { 'data-callout': calloutTone(a.tone), style: `background:${tone.bg};border-left:4px solid ${tone.border}` };
        },
      },
    };
  },
  parseHTML() { return [{ tag: 'div[data-callout]' }]; },
  renderHTML({ HTMLAttributes }) { return ['div', mergeAttributes(HTMLAttributes, { class: 'doc-callout' }), 0]; },
  addCommands() {
    return {
      toggleCallout: (tone = 'info') => ({ editor, commands }) => (editor.isActive(this.name) ? commands.lift(this.name) : commands.wrapIn(this.name, { tone })),
      setCalloutTone: (tone) => ({ commands }) => commands.updateAttributes(this.name, { tone }),
    };
  },
});

export const CALLOUT_LABEL: Record<CalloutTone, string> = { info: tk('Info'), success: tk('Success'), warning: tk('Warning'), danger: tk('Danger'), note: tk('Idea') };

// ------------------------------------------------------------------ table of contents

function TocView({ editor }: NodeViewProps) {
  const [hs, setHs] = useState<Array<{ level: number; text: string; pos: number }>>([]);
  useEffect(() => {
    const read = () => {
      const out: Array<{ level: number; text: string; pos: number }> = [];
      editor.state.doc.descendants((n, pos) => {
        if (n.type.name === 'heading' && n.textContent.trim()) out.push({ level: n.attrs.level as number, text: n.textContent, pos });
      });
      setHs((prev) => (JSON.stringify(prev) === JSON.stringify(out) ? prev : out));
    };
    read();
    editor.on('update', read);
    return () => { editor.off('update', read); };
  }, [editor]);
  return (
    <NodeViewWrapper className="doc-block doc-toc" data-drag-handle>
      <div contentEditable={false}>
        <div className="doc-toc-title">{t('Contents')}</div>
        {hs.length ? hs.map((h, i) => (
          <a key={i} href="#" style={{ paddingLeft: (h.level - 1) * 16 }} onClick={(e) => {
            e.preventDefault();
            editor.view.domAtPos(h.pos + 1).node.parentElement?.scrollIntoView({ behavior: 'smooth', block: 'center' });
          }}>{h.text}</a>
        )) : <p className="faint" style={{ margin: 0 }}>{t('Add headings to build the table of contents.')}</p>}
      </div>
    </NodeViewWrapper>
  );
}

export const TableOfContents = Node.create({
  name: 'tableOfContents',
  group: 'block',
  atom: true,
  draggable: true,
  selectable: true,
  parseHTML() { return [{ tag: 'div[data-toc]' }]; },
  renderHTML({ HTMLAttributes }) { return ['div', mergeAttributes(HTMLAttributes, { 'data-toc': '' })]; },
  addNodeView() { return ReactNodeViewRenderer(TocView); },
  addCommands() {
    return { insertTableOfContents: () => ({ commands }) => commands.insertContent({ type: this.name }) };
  },
});

// ------------------------------------------------------------------ page break

function PageBreakView() {
  return <NodeViewWrapper className="doc-page-break" data-drag-handle><span contentEditable={false}>{t('Page break')}</span></NodeViewWrapper>;
}

export const PageBreak = Node.create({
  name: 'pageBreak',
  group: 'block',
  atom: true,
  selectable: true,
  parseHTML() { return [{ tag: 'div[data-page-break]' }, { tag: 'div', getAttrs: (el) => (/page-break-(after|before)\s*:\s*always/.test((el as HTMLElement).getAttribute('style') ?? '') ? {} : false) }]; },
  renderHTML({ HTMLAttributes }) { return ['div', mergeAttributes(HTMLAttributes, { 'data-page-break': '', style: 'page-break-after: always' })]; },
  addNodeView() { return ReactNodeViewRenderer(PageBreakView); },
  addCommands() {
    return { insertPageBreak: () => ({ commands }) => commands.insertContent({ type: this.name }) };
  },
});

// ------------------------------------------------------------------ video embed

const frameAllowed = (src: string) => EMBED_FRAME_HOSTS.some((h) => src.startsWith(`${h}/`));

function VideoView({ node, selected }: NodeViewProps) {
  const src = String(node.attrs.src ?? '');
  const url = String(node.attrs.url ?? '');
  const aspect = Number(node.attrs.aspect) || 16 / 9;
  return (
    <NodeViewWrapper className={`doc-block doc-video ${selected ? 'on' : ''}`} data-drag-handle>
      <div className="doc-block-frame" style={{ width: '100%', aspectRatio: String(aspect) }}>
        {frameAllowed(src) ? (
          <iframe title={String(node.attrs.title || node.attrs.provider || t('Video'))} src={src} style={{ width: '100%', height: '100%', border: 0, borderRadius: 8 }} loading="lazy" referrerPolicy="strict-origin-when-cross-origin"
            allow="autoplay; encrypted-media; picture-in-picture; fullscreen" allowFullScreen sandbox="allow-scripts allow-same-origin allow-presentation allow-popups allow-popups-to-escape-sandbox" />
        ) : <div className="doc-locked">{t('This video can’t be embedded.')}</div>}
      </div>
      {selected && <div className="doc-block-bar" contentEditable={false}><a className="btn ghost sm" href={url} target="_blank" rel="noopener noreferrer"><ExternalLink size={14} /> {t('Open link')}</a></div>}
    </NodeViewWrapper>
  );
}

export const VideoEmbed = Node.create({
  name: 'videoEmbed',
  group: 'block',
  atom: true,
  draggable: true,
  selectable: true,
  addAttributes() {
    return { url: dataAttr('url', ''), src: dataAttr('src', ''), provider: dataAttr('provider', ''), title: dataAttr('title', ''), aspect: dataAttr('aspect', 16 / 9, true) };
  },
  parseHTML() { return [{ tag: 'div[data-video-embed]' }]; },
  renderHTML({ HTMLAttributes }) { return ['div', mergeAttributes(HTMLAttributes, { 'data-video-embed': '' })]; },
  addNodeView() { return ReactNodeViewRenderer(VideoView); },
  addCommands() {
    return {
      insertVideoEmbed: (url) => ({ commands }) => {
        const e = parseEmbedUrl(url);
        if (!e || !frameAllowed(e.src)) return false;
        return commands.insertContent({ type: this.name, attrs: { url: url.trim(), src: e.src, provider: e.provider, aspect: e.aspect } });
      },
    };
  },
});

export const DOC_BLOCKS = [DesignEmbed, DocChart, Callout, TableOfContents, PageBreak, VideoEmbed];
