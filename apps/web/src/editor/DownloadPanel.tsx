/**
 * Download panel (Canva-style): a compact popover under the top-bar Download button (a bottom sheet on phones).
 * File type → only the options that format needs → pages → one Download button with inline progress.
 * PDF Print adds "Crop marks and bleed", "Flatten PDF" and RGB/CMYK, with everything else under Advanced.
 */
import { sortedPages, videoSlots, type DesignDoc } from '@oc/engine';
import {
  Check, ChevronDown, Download, FileArchive, FileImage, FileText, FileType, Film, Image as ImageIcon, ImagePlay, Music,
  PenTool, Presentation, Printer, Trash2, Upload, X,
} from 'lucide-react';
import { useCallback, useEffect, useId, useLayoutEffect, useMemo, useRef, useState, type ComponentType, type KeyboardEvent as ReactKeyboardEvent, type ReactNode } from 'react';
import { createPortal } from 'react-dom';
import { useAuth } from '../App';
import { api, ApiError, contentApi, download, type Credit } from '../api';
import { interp } from '../components/ui';
import { t, tk, tn } from '../i18n';
import { track } from '../lib/analytics';
import { StandalonePreflight } from '../print/Preflight';
import {
  bleedMmFor, effectiveProfile, FALLBACK_PROFILES, loadLastFormat, loadPrintPrefs, printColorApi, printProductFor, printRequest, profileRef,
  saveLastFormat, savePrintPrefs, standardFor, type PdfStandard, type PrintPrefs, type ProfileInfo,
} from '../print/printExport';
import { useDoc, useEd } from './context';
import { takeOnePageDownload } from './downloadIntent';
import { PageThumb } from './PageStrip';
import { useWatermark, WatermarkOptions } from './WatermarkOptions';
import './download.css';

export type ExportFormat = 'png' | 'jpg' | 'pdf' | 'pdf-print' | 'pdf-flat' | 'pptx' | 'zip' | 'svg' | 'mp4' | 'gif' | 'm4a' | 'docx' | 'md';

interface FormatInfo { id: ExportFormat; name: string; desc: string; icon: ComponentType<{ size?: number }>; more?: boolean; docOnly?: boolean }

const FORMATS: FormatInfo[] = [
  { id: 'png', name: tk('PNG'), desc: tk('Sharp image, supports transparency'), icon: ImageIcon },
  { id: 'jpg', name: tk('JPG'), desc: tk('Smallest image file'), icon: FileImage },
  { id: 'pdf', name: tk('PDF Standard'), desc: tk('Documents and email; selectable text'), icon: FileText },
  { id: 'pdf-print', name: tk('PDF Print'), desc: tk('High quality for professional printing'), icon: Printer },
  { id: 'mp4', name: tk('MP4 Video'), desc: tk('Animations and transitions'), icon: Film },
  { id: 'gif', name: tk('GIF'), desc: tk('Short animated loop'), icon: ImagePlay },
  { id: 'pptx', name: tk('PowerPoint (PPTX)'), desc: tk('Editable slides'), icon: Presentation },
  { id: 'svg', name: tk('SVG'), desc: tk('Scalable vector, editable text'), icon: PenTool, more: true },
  { id: 'pdf-flat', name: tk('PDF Flattened'), desc: tk('Every page as an image'), icon: FileText, more: true },
  { id: 'zip', name: tk('ZIP'), desc: tk('Every page as PNG'), icon: FileArchive, more: true },
  { id: 'm4a', name: tk('Audio (M4A)'), desc: tk('Soundtrack only'), icon: Music, more: true },
  { id: 'docx', name: tk('Word (DOCX)'), desc: tk('Editable document'), icon: FileType, more: true, docOnly: true },
  { id: 'md', name: tk('Markdown (MD)'), desc: tk('Plain text'), icon: FileType, more: true, docOnly: true },
];

const VIDEO_FORMATS: ExportFormat[] = ['mp4', 'gif', 'm4a'];
/** Formats the export watermark applies to (g64). */
const WATERMARK_FORMATS: ExportFormat[] = ['png', 'jpg', 'pdf', 'pdf-print', 'pdf-flat', 'zip', 'svg'];
/** Formats that export the whole design (no page choice). */
const WHOLE_DESIGN: ExportFormat[] = ['zip', 'pptx', 'docx', 'md', ...VIDEO_FORMATS];

const STANDARD_HELP: Record<PdfStandard, string> = {
  none: tk('A regular PDF with no print standard.'),
  pdfx4: tk('PDF/X-4: modern print standard; keeps transparency and live text.'),
  pdfx1a: tk('PDF/X-1a: everything CMYK and flattened, for older print workflows.'),
};
const STANDARD_NAME: Record<PdfStandard, string> = { none: tk('None'), pdfx4: 'PDF/X-4', pdfx1a: 'PDF/X-1a' };

/** The format that fits the design's surface best. */
export function suggestedFormat(doc: DesignDoc, printProduct: boolean): ExportFormat {
  const s = doc.meta.surface;
  if (s === 'video' || Object.values(doc.elements).some((e) => !e.deleted && videoSlots(e).length > 0)) return 'mp4';
  if (printProduct) return 'pdf-print';
  if (s === 'presentation') return 'pptx';
  if (s === 'doc') return 'pdf';
  return 'png';
}

const NON_PRINT_SURFACES = ['doc', 'website', 'email', 'video', 'presentation'];

export function DownloadPanel({ onClose }: { onClose: () => void }) {
  const { design, editor } = useEd();
  const { user } = useAuth();
  const doc = useDoc();
  const pages = sortedPages(doc);
  const activeIdx = Math.max(0, pages.findIndex((p) => p.id === editor.state.activePageId));
  const surface = doc.meta.surface;
  const { w, h } = doc.meta.dims;

  const product = useMemo(() => (NON_PRINT_SURFACES.includes(surface) ? { printProduct: false, bleedMm: 3 } : printProductFor({ meta: { ...doc.meta } })),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [surface, w, h, doc.meta.dims.unit]);
  const suggested = useMemo(() => suggestedFormat(doc, product.printProduct),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [product.printProduct, surface]);
  const formats = useMemo(() => FORMATS.filter((f) => !f.docOnly || surface === 'doc'), [surface]);
  const [format, setFormatState] = useState<ExportFormat>(() => {
    const last = loadLastFormat(user?.id, surface);
    return formats.some((f) => f.id === last) ? (last as ExportFormat) : suggested;
  });
  const setFormat = (f: ExportFormat) => { setFormatState(f); setPhase((p) => (p === 'done' ? 'idle' : p)); setError(''); };

  const [vq, setVq] = useState<'720p' | '1080p' | '4k'>('1080p');
  const [scale, setScale] = useState(1);
  const [quality, setQuality] = useState(90);
  const [transparent, setTransparent] = useState(false);
  const watermark = useWatermark();
  const [pageMode, setPageMode] = useState<'all' | 'current' | 'pick'>(() => (takeOnePageDownload() || pages.length <= 1 ? 'current' : 'all'));
  const [picked, setPicked] = useState<number[]>(() => [activeIdx]);
  const [phase, setPhase] = useState<'idle' | 'busy' | 'done'>('idle');
  const [progress, setProgress] = useState<{ value: number | null; label: string }>({ value: null, label: '' });
  const [error, setError] = useState('');

  // PDF Print choices, remembered per user.
  const [prefs, setPrefsState] = useState<PrintPrefs>(() => loadPrintPrefs(user?.id, product.printProduct));
  const setPrefs = (patch: Partial<PrintPrefs>) => setPrefsState((p) => {
    const next = { ...p, ...patch };
    savePrintPrefs(user?.id, product.printProduct, next);
    return next;
  });
  const [profiles, setProfiles] = useState<ProfileInfo[]>(FALLBACK_PROFILES);
  const isPrint = format === 'pdf-print';
  useEffect(() => {
    if (!isPrint) return;
    let live = true;
    printColorApi.profiles().then((r) => { if (live && Array.isArray(r.profiles) && r.profiles.length) setProfiles(r.profiles); }).catch(() => { /* bundled list */ });
    return () => { live = false; };
  }, [isPrint]);

  const video = VIDEO_FORMATS.includes(format);
  const pageChoice = pages.length > 1 && !WHOLE_DESIGN.includes(format);
  const selPages: number[] | undefined = !pageChoice || pageMode === 'all' ? undefined : pageMode === 'current' ? [activeIdx] : [...picked].sort((a, b) => a - b);
  const nothingPicked = pageChoice && pageMode === 'pick' && picked.length === 0;

  const go = async () => {
    if (phase === 'busy' || nothingPicked) return;
    setPhase('busy');
    setError('');
    setProgress({ value: null, label: t('Preparing…') });
    saveLastFormat(user?.id, surface, format);
    const base = doc.meta.title.replace(/[^\w\- ]+/g, '').trim() || 'design';
    try {
      if (video) {
        track('export_start', { format, quality: vq, background: true });
        const { id } = await api.exportJob(design.id, { format, quality: vq });
        setProgress({ value: 0, label: t('Queued') });
        for (;;) {
          await new Promise((r) => setTimeout(r, 700));
          const st = await api.jobStatus(id);
          setProgress({ value: st.progress, label: st.label });
          if (st.status === 'error') throw new Error(st.error ?? 'export failed');
          if (st.status === 'done') {
            download(await api.jobFile(id), st.filename ?? `${base}.${format}`);
            break;
          }
        }
      } else {
        const print = isPrint ? printRequest(prefs, product.bleedMm, profiles) : undefined;
        track('export_start', { format, pages: selPages?.length ?? 0, ...(print ? { standard: print.standard ?? 'none' } : {}) });
        const wm = WATERMARK_FORMATS.includes(format) ? watermark.request : undefined;
        const blob = await api.exportDesign(design.id, { format, pages: selPages, scale, quality, transparent, ...(print ? { print } : {}), ...(wm ? { watermark: wm } : {}) });
        const many = (selPages?.length ?? pages.length) > 1;
        const ext = blob.type.includes('zip') ? 'zip'
          : format.startsWith('pdf') ? 'pdf' : format === 'pptx' || format === 'docx' || format === 'md' ? format
          : format === 'zip' || many ? 'zip' : format;
        download(blob, `${base}.${ext}`);
      }
      setPhase('done');
    } catch (e) {
      setPhase('idle');
      setError(t('Export failed: {message}', { message: e instanceof ApiError ? e.message || e.code : (e as Error).message }));
    }
  };
  useEffect(() => {
    if (phase !== 'done') return;
    const timer = setTimeout(() => setPhase('idle'), 4000);
    return () => clearTimeout(timer);
  }, [phase]);

  const visiblePages = pages.filter((p) => !p.hidden);

  return (
    <Panel title={t('Download')} onClose={onClose} busy={phase === 'busy'}>
      <FormatSelect formats={formats} value={format} suggested={suggested} onChange={setFormat} />

      {(format === 'png' || format === 'jpg') && (
        <div className="dl-field">
          <div className="dl-label-row">
            <label className="dl-label" htmlFor="dl-scale">{t('Size ×')}</label>
            <output className="dl-value" htmlFor="dl-scale">{String(+scale.toFixed(2))}</output>
          </div>
          <input id="dl-scale" type="range" min={0.25} max={4} step={0.25} value={scale} onChange={(e) => setScale(+e.target.value)}
            aria-valuetext={t('{w} × {h} px', { w: Math.round(w * scale), h: Math.round(h * scale) })} />
          <span className="dl-hint">{t('{w} × {h} px', { w: Math.round(w * scale), h: Math.round(h * scale) })}</span>
        </div>
      )}
      {format === 'png' && (
        <label className="dl-check"><input type="checkbox" checked={transparent} onChange={(e) => setTransparent(e.target.checked)} /> {t('Transparent background')}</label>
      )}
      {format === 'jpg' && (
        <div className="dl-field">
          <div className="dl-label-row">
            <label className="dl-label" htmlFor="dl-quality">{t('Quality')}</label>
            <output className="dl-value" htmlFor="dl-quality">{quality}</output>
          </div>
          <input id="dl-quality" type="range" min={10} max={100} value={quality} onChange={(e) => setQuality(+e.target.value)} />
        </div>
      )}
      {format === 'mp4' && (
        <div className="dl-field">
          <label className="dl-label" htmlFor="dl-res">{t('Resolution')}</label>
          <select id="dl-res" className="input" value={vq} onChange={(e) => setVq(e.target.value as typeof vq)}>
            <option value="720p">{t('720p — fastest')}</option><option value="1080p">{t('1080p (HD)')}</option><option value="4k">{t('4K (Ultra HD)')}</option>
          </select>
        </div>
      )}
      {video && (
        <p className="dl-hint" style={{ margin: 0 }}>
          {tn(visiblePages.length, '{n} page, {seconds} s total.', '{n} pages, {seconds} s total.', { seconds: (visiblePages.reduce((sum, p) => sum + (p.durationMs ?? 5000), 0) / 1000).toFixed(1) })}
          {' '}{interp(t('Set page timing and transitions under {animate}.'), { animate: <b>{t('Animate')}</b> })}
        </p>
      )}

      {WATERMARK_FORMATS.includes(format) && <WatermarkOptions {...watermark} />}

      {isPrint && <PrintOptions prefs={prefs} setPrefs={setPrefs} autoBleedMm={product.bleedMm} profiles={profiles} setProfiles={setProfiles} />}

      {pageChoice && (
        <PagePicker pages={pages} activeIdx={activeIdx} mode={pageMode} setMode={setPageMode} picked={picked} setPicked={setPicked} w={w} h={h} />
      )}

      {isPrint && (
        <StandalonePreflight bleedMm={bleedMmFor(prefs, product.bleedMm)}
          gamut={prefs.color === 'cmyk' ? gamutArgs(prefs, profiles) : null} />
      )}

      <DownloadButton phase={phase} progress={progress} disabled={nothingPicked} onClick={() => void go()} />
      {error && <p className="dl-error" role="alert">{error}</p>}

      <div className="dl-footer">
        <Credits designId={design.id} />
      </div>
    </Panel>
  );
}

/** Stable args for the colour-managed gamut check (same object while the inputs are unchanged). */
const gamutCache = new Map<string, { profile: string | { id: string }; intent: 'relative' | 'perceptual' }>();
function gamutArgs(prefs: PrintPrefs, profiles: ProfileInfo[]) {
  const p = effectiveProfile(prefs, profiles);
  if (!p) return null;
  const key = `${p.kind}:${p.id}:${prefs.intent}`;
  let v = gamutCache.get(key);
  if (!v) gamutCache.set(key, (v = { profile: profileRef(p), intent: prefs.intent }));
  return v;
}

// ---------------------------------------------------------------- panel shell

const PHONE = '(max-width: 640px)';

function usePhone(): boolean {
  const [phone, setPhone] = useState(() => typeof matchMedia !== 'undefined' && matchMedia(PHONE).matches);
  useEffect(() => {
    if (typeof matchMedia === 'undefined') return;
    const m = matchMedia(PHONE);
    const on = () => setPhone(m.matches);
    m.addEventListener('change', on);
    return () => m.removeEventListener('change', on);
  }, []);
  return phone;
}

const FOCUSABLE = 'a[href], button:not([disabled]), input:not([disabled]), select:not([disabled]), textarea:not([disabled]), summary, [tabindex]:not([tabindex="-1"])';

/** Popover under the top-bar Download button (bottom sheet on phones) with a focus trap; Escape / outside click close it. */
function Panel({ title, onClose, busy, children }: { title: string; onClose: () => void; busy: boolean; children: ReactNode }) {
  const ref = useRef<HTMLDivElement>(null);
  const titleId = useId();
  const phone = usePhone();
  const [opener] = useState(() => (typeof document === 'undefined' ? null : (document.activeElement as HTMLElement | null)));
  const [pos, setPos] = useState<{ top: number; left: number; maxHeight: number } | null>(null);

  const place = useCallback(() => {
    if (phone) return;
    const anchor = [...document.querySelectorAll<HTMLElement>('[data-download-anchor]')].find((a) => a.offsetParent !== null);
    const width = Math.min(360, window.innerWidth - 16);
    const r = anchor?.getBoundingClientRect();
    const top = r ? r.bottom + 8 : 60;
    const left = Math.max(8, Math.min((r ? r.right : window.innerWidth - 12) - width, window.innerWidth - width - 8));
    setPos({ top, left, maxHeight: window.innerHeight - top - 12 });
  }, [phone]);
  useLayoutEffect(() => {
    place();
    window.addEventListener('resize', place);
    return () => window.removeEventListener('resize', place);
  }, [place]);

  // Focus moves in (to the file type) and returns to the opener on close.
  useEffect(() => {
    const el = ref.current;
    if (el && !el.contains(document.activeElement)) (el.querySelector<HTMLElement>('[data-autofocus]') ?? el).focus();
    return () => {
      const back = opener?.isConnected && opener !== document.body ? opener : [...document.querySelectorAll<HTMLElement>('[data-download-anchor]')].find((a) => a.offsetParent !== null);
      back?.focus();
    };
  }, [opener]);

  // The top-bar button toggles the panel: while open, a click on it closes instead of reopening.
  useEffect(() => {
    const anchor = [...document.querySelectorAll<HTMLElement>('[data-download-anchor]')].find((a) => a.offsetParent !== null);
    if (!anchor) return;
    const click = (e: MouseEvent) => { e.stopPropagation(); onClose(); };
    anchor.setAttribute('aria-expanded', 'true');
    anchor.addEventListener('click', click);
    return () => { anchor.removeEventListener('click', click); anchor.setAttribute('aria-expanded', 'false'); };
  }, [onClose]);

  // Outside click closes (not while a file is being prepared).
  useEffect(() => {
    if (phone) return;
    const down = (e: MouseEvent) => {
      const tgt = e.target as Element | null;
      if (!tgt || ref.current?.contains(tgt) || tgt.closest?.('[data-download-anchor], .modal-backdrop, .toast-host')) return;
      if (!busy) onClose();
    };
    window.addEventListener('mousedown', down, true);
    return () => window.removeEventListener('mousedown', down, true);
  }, [phone, busy, onClose]);
  useEffect(() => {
    const key = (e: KeyboardEvent) => {
      if (e.key === 'Escape' && !e.defaultPrevented && !ref.current?.contains(e.target as Node)) onClose();
    };
    window.addEventListener('keydown', key);
    return () => window.removeEventListener('keydown', key);
  }, [onClose]);

  const onKeyDown = (e: ReactKeyboardEvent) => {
    if (e.key === 'Escape' && !e.isDefaultPrevented()) {
      e.preventDefault();
      onClose();
    } else if (e.key === 'Tab' && ref.current) {
      const items = [...ref.current.querySelectorAll<HTMLElement>(FOCUSABLE)].filter((n) => n.offsetParent !== null && !n.closest('[role="listbox"]'));
      if (items.length) {
        const first = items[0]!, last = items[items.length - 1]!;
        if (e.shiftKey && (document.activeElement === first || document.activeElement === ref.current)) { e.preventDefault(); last.focus(); }
        else if (!e.shiftKey && document.activeElement === last) { e.preventDefault(); first.focus(); }
      }
    }
    // Keys typed in the panel never reach the editor's shortcuts.
    e.stopPropagation();
  };

  const style = phone ? undefined : pos ? { top: pos.top, left: pos.left, maxHeight: pos.maxHeight } : { visibility: 'hidden' as const, top: 0, left: 0 };
  return createPortal(
    <>
      {phone && <div className="dl-scrim" onMouseDown={() => !busy && onClose()} />}
      <div ref={ref} className={`dl-panel${phone ? ' sheet' : ''}`} role="dialog" aria-modal="true" aria-labelledby={titleId} tabIndex={-1} style={style} onKeyDown={onKeyDown}>
        {phone && <div className="dl-grab" aria-hidden />}
        <header className="dl-head">
          <h2 id={titleId}>{title}</h2>
          <button type="button" className="btn ghost icon sm" onClick={onClose} aria-label={t('Close')}><X size={18} /></button>
        </header>
        <div className="dl-body">{children}</div>
      </div>
    </>,
    document.body,
  );
}

// ---------------------------------------------------------------- file type listbox

function FormatSelect({ formats, value, suggested, onChange }: { formats: FormatInfo[]; value: ExportFormat; suggested: ExportFormat; onChange: (f: ExportFormat) => void }) {
  const id = useId();
  const [open, setOpen] = useState(false);
  const [active, setActive] = useState<ExportFormat>(value);
  const btn = useRef<HTMLButtonElement>(null);
  const list = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState<{ left: number; top: number; width: number; maxHeight: number } | null>(null);
  // The suggested format always sits in the main group.
  const main = formats.filter((f) => !f.more || f.id === suggested);
  const more = formats.filter((f) => f.more && f.id !== suggested);
  const flat = [...main, ...more];
  const cur = formats.find((f) => f.id === value) ?? formats[0]!;
  const optId = (f: ExportFormat) => `${id}-opt-${f}`;

  useLayoutEffect(() => {
    if (!open || !btn.current) return;
    const r = btn.current.getBoundingClientRect();
    const below = window.innerHeight - r.bottom - 12;
    const above = r.top - 12;
    const maxHeight = Math.min(420, Math.max(below, above));
    setPos(below >= Math.min(320, above) ? { left: r.left, top: r.bottom + 4, width: r.width, maxHeight: Math.min(420, below) } : { left: r.left, top: r.top - 4 - Math.min(maxHeight, 420), width: r.width, maxHeight });
    list.current?.focus();
  }, [open]);
  useEffect(() => {
    if (open) document.getElementById(optId(active))?.scrollIntoView({ block: 'nearest' });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open, active]);
  useEffect(() => {
    if (!open) return;
    const down = (e: MouseEvent) => {
      if (list.current?.contains(e.target as Node) || btn.current?.contains(e.target as Node)) return;
      setOpen(false);
    };
    window.addEventListener('mousedown', down, true);
    return () => window.removeEventListener('mousedown', down, true);
  }, [open]);

  const openList = () => { setActive(value); setOpen(true); };
  const choose = (f: ExportFormat) => { onChange(f); setOpen(false); btn.current?.focus(); };
  const onBtnKey = (e: ReactKeyboardEvent) => {
    if (['ArrowDown', 'ArrowUp', 'Enter', ' '].includes(e.key)) { e.preventDefault(); openList(); }
  };
  const onListKey = (e: ReactKeyboardEvent) => {
    const i = flat.findIndex((f) => f.id === active);
    const move = (n: number) => { e.preventDefault(); setActive(flat[Math.max(0, Math.min(flat.length - 1, n))]!.id); };
    switch (e.key) {
      case 'ArrowDown': return move(i + 1);
      case 'ArrowUp': return move(i - 1);
      case 'Home': return move(0);
      case 'End': return move(flat.length - 1);
      case 'Enter': case ' ': e.preventDefault(); return choose(active);
      case 'Escape': e.preventDefault(); setOpen(false); btn.current?.focus(); return;
      case 'Tab': setOpen(false); return;
      default:
        if (e.key.length === 1 && /\S/.test(e.key)) {
          const k = e.key.toLowerCase();
          const order = [...flat.slice(i + 1), ...flat.slice(0, i + 1)];
          const hit = order.find((f) => t(f.name).toLowerCase().startsWith(k));
          if (hit) { e.preventDefault(); setActive(hit.id); }
        }
    }
  };
  const option = (f: FormatInfo) => {
    const Icon = f.icon;
    return (
      <div key={f.id} id={optId(f.id)} role="option" aria-selected={f.id === value} className={`dl-opt${f.id === active ? ' active' : ''}${f.id === value ? ' selected' : ''}`}
        onMouseEnter={() => setActive(f.id)} onClick={() => choose(f.id)}>
        <span className="dl-opt-icon"><Icon size={18} /></span>
        <span className="dl-opt-text">
          <span className="dl-opt-name">{t(f.name)}{f.id === suggested && <span className="dl-badge">{t('Suggested')}</span>}</span>
          <span className="dl-opt-desc">{t(f.desc)}</span>
        </span>
        {f.id === value && <Check size={16} className="dl-opt-check" />}
      </div>
    );
  };
  const CurIcon = cur.icon;
  return (
    <div className="dl-field">
      <span className="dl-label" id={`${id}-label`}>{t('File type')}</span>
      <button ref={btn} type="button" className="dl-select dl-format" data-autofocus aria-haspopup="listbox" aria-expanded={open} aria-labelledby={`${id}-label ${id}-btn`} id={`${id}-btn`}
        onClick={() => (open ? setOpen(false) : openList())} onKeyDown={onBtnKey}>
        <span className="dl-opt-icon"><CurIcon size={18} /></span>
        <span className="dl-opt-text">
          <span className="dl-opt-name">{t(cur.name)}{cur.id === suggested && <span className="dl-badge">{t('Suggested')}</span>}</span>
          <span className="dl-opt-desc">{t(cur.desc)}</span>
        </span>
        <ChevronDown size={16} className="dl-chevron" />
      </button>
      {open && (
        <div ref={list} className="dl-listbox" role="listbox" tabIndex={-1} aria-labelledby={`${id}-label`} aria-activedescendant={optId(active)} onKeyDown={onListKey}
          style={pos ? { left: pos.left, top: pos.top, width: pos.width, maxHeight: pos.maxHeight } : { visibility: 'hidden' }}>
          {main.map(option)}
          {more.length > 0 && (
            <div role="group" aria-labelledby={`${id}-more`}>
              <div className="dl-group-label" id={`${id}-more`} role="presentation">{t('More formats')}</div>
              {more.map(option)}
            </div>
          )}
        </div>
      )}
    </div>
  );
}

// ---------------------------------------------------------------- pages

function PagePicker({ pages, activeIdx, mode, setMode, picked, setPicked, w, h }: {
  pages: ReturnType<typeof sortedPages>; activeIdx: number; mode: 'all' | 'current' | 'pick'; setMode: (m: 'all' | 'current' | 'pick') => void;
  picked: number[]; setPicked: (p: number[]) => void; w: number; h: number;
}) {
  const id = useId();
  const [open, setOpen] = useState(false);
  const summary = mode === 'all' ? t('All pages ({n})', { n: pages.length })
    : mode === 'current' ? t('Current page ({n})', { n: activeIdx + 1 })
    : picked.length === 1 ? t('Page {n}', { n: picked[0]! + 1 }) : tn(picked.length, '{n} page selected', '{n} pages selected');
  const thumbH = Math.round(Math.min(48, (64 * h) / w));
  const toggle = (i: number, on: boolean) => setPicked(on ? [...new Set([...picked, i])] : picked.filter((x) => x !== i));
  return (
    <div className="dl-field">
      <span className="dl-label" id={`${id}-label`}>{t('Select pages')}</span>
      <button type="button" className="dl-select" aria-expanded={open} aria-controls={`${id}-pop`} aria-labelledby={`${id}-label ${id}-btn`} id={`${id}-btn`} onClick={() => setOpen(!open)}>
        <span className="grow" style={{ textAlign: 'start' }}>{summary}</span>
        <ChevronDown size={16} className="dl-chevron" />
      </button>
      {open && (
        <div className="dl-pages" id={`${id}-pop`}>
          <div role="radiogroup" aria-labelledby={`${id}-label`} className="dl-radios compact">
            <label className="dl-check"><input type="radio" name={`${id}-m`} checked={mode === 'all'} onChange={() => setMode('all')} /> {t('All pages ({n})', { n: pages.length })}</label>
            <label className="dl-check"><input type="radio" name={`${id}-m`} checked={mode === 'current'} onChange={() => setMode('current')} /> {t('Current page ({n})', { n: activeIdx + 1 })}</label>
            <label className="dl-check"><input type="radio" name={`${id}-m`} checked={mode === 'pick'} onChange={() => { setMode('pick'); if (!picked.length) setPicked([activeIdx]); }} /> {t('Choose pages')}</label>
          </div>
          {mode === 'pick' && (
            <div className="dl-page-grid" role="group" aria-label={t('Choose pages')}>
              {pages.map((p, i) => (
                <label key={p.id} className={`dl-page${picked.includes(i) ? ' on' : ''}`}>
                  <input type="checkbox" checked={picked.includes(i)} onChange={(e) => toggle(i, e.target.checked)} aria-label={t('Page {n}', { n: i + 1 })} />
                  {pages.length <= 40 ? <PageThumb pageId={p.id} w={w} h={h} height={thumbH} /> : <span className="dl-page-blank" style={{ height: thumbH }} />}
                  <span className="dl-page-n" aria-hidden>{i + 1}</span>
                </label>
              ))}
            </div>
          )}
          {mode === 'pick' && !picked.length && <span className="dl-error">{t('Choose at least one page.')}</span>}
        </div>
      )}
    </div>
  );
}

// ---------------------------------------------------------------- PDF Print

function PrintOptions({ prefs, setPrefs, autoBleedMm, profiles, setProfiles }: {
  prefs: PrintPrefs; setPrefs: (p: Partial<PrintPrefs>) => void; autoBleedMm: number; profiles: ProfileInfo[]; setProfiles: (p: ProfileInfo[]) => void;
}) {
  const id = useId();
  const file = useRef<HTMLInputElement>(null);
  const [uploading, setUploading] = useState(false);
  const [uploadErr, setUploadErr] = useState('');
  const space = prefs.color === 'cmyk' ? 'CMYK' : 'RGB';
  const usable = profiles.filter((p) => p.colorSpace === space && p.available !== false);
  const current = effectiveProfile(prefs, profiles);
  const standard = standardFor(prefs);
  const bleed = bleedMmFor({ ...prefs, marks: true }, autoBleedMm);

  const setColor = (color: 'rgb' | 'cmyk') => setPrefs({ color, ...(color === 'rgb' && prefs.standard !== 'none' ? { standard: 'auto' as const } : {}) });
  const setStandard = (s: PrintPrefs['standard']) => setPrefs({ standard: s, ...(s === 'pdfx4' || s === 'pdfx1a' ? { color: 'cmyk' as const } : {}) });
  const upload = async (f: File) => {
    setUploading(true);
    setUploadErr('');
    try {
      const p = await printColorApi.upload(f, { name: f.name.replace(/\.(icc|icm)$/i, '') });
      setProfiles([...profiles.filter((x) => x.id !== p.id), p]);
      setPrefs({ profile: p.id, color: p.colorSpace === 'RGB' ? 'rgb' : 'cmyk', ...(p.colorSpace === 'RGB' ? { standard: 'auto' as const } : {}) });
    } catch (e) {
      setUploadErr(e instanceof ApiError && e.code === 'invalid_icc' ? t('That file isn’t a valid ICC colour profile.') : t('Upload failed: {message}', { message: (e as Error).message }));
    } finally {
      setUploading(false);
      if (file.current) file.current.value = '';
    }
  };
  const remove = async (p: ProfileInfo) => {
    if (!confirm(t('Remove the colour profile “{name}”?', { name: p.name }))) return;
    try {
      await printColorApi.remove(p.id);
      setProfiles(profiles.filter((x) => x.id !== p.id));
      setPrefs({ profile: '' });
    } catch (e) {
      setUploadErr((e as Error).message);
    }
  };

  return (
    <div className="dl-print">
      <label className="dl-check">
        <input type="checkbox" checked={prefs.marks} onChange={(e) => setPrefs({ marks: e.target.checked })} />
        <span>{t('Crop marks and bleed')}{prefs.marks && <span className="dl-hint"> · {t('{mm} mm bleed', { mm: bleed })}</span>}</span>
      </label>
      <label className="dl-check">
        <input type="checkbox" checked={prefs.flatten} onChange={(e) => setPrefs({ flatten: e.target.checked })} />
        <span>{t('Flatten PDF')}</span>
      </label>

      <div className="dl-field">
        <span className="dl-label" id={`${id}-color`}>{t('Colour profile')}</span>
        <div role="radiogroup" aria-labelledby={`${id}-color`} className="dl-radios">
          {([['rgb', 'RGB', tk('Best for digital and home printing')], ['cmyk', 'CMYK', tk('Best for professional printing')]] as const).map(([v, name, desc]) => (
            <label key={v} className={`dl-radio${prefs.color === v ? ' on' : ''}`}>
              <input type="radio" name={`${id}-color`} checked={prefs.color === v} onChange={() => setColor(v)} />
              <span className="dl-opt-text"><span className="dl-opt-name">{name}</span><span className="dl-opt-desc">{t(desc)}</span></span>
            </label>
          ))}
        </div>
      </div>

      <details className="dl-advanced">
        <summary>{t('Advanced')}</summary>
        <div className="dl-adv-body">
          <div className="dl-field">
            <label className="dl-label" htmlFor={`${id}-bleed`}>{t('Bleed')}</label>
            <div className="row">
              <select id={`${id}-bleed`} className="input" value={prefs.bleed} disabled={!prefs.marks} onChange={(e) => setPrefs({ bleed: e.target.value as PrintPrefs['bleed'] })}>
                <option value="auto">{t('Automatic ({mm} mm)', { mm: autoBleedMm })}</option>
                <option value="none">{t('None')}</option>
                <option value="3">{t('{mm} mm', { mm: 3 })}</option>
                <option value="5">{t('{mm} mm', { mm: 5 })}</option>
                <option value="custom">{t('Custom')}</option>
              </select>
              {prefs.bleed === 'custom' && (
                <input className="input" type="number" min={0} max={10} step={0.5} style={{ width: 84 }} disabled={!prefs.marks} value={prefs.customBleedMm}
                  aria-label={t('Bleed in millimetres')} onChange={(e) => setPrefs({ customBleedMm: Math.min(10, Math.max(0, +e.target.value || 0)) })} />
              )}
            </div>
          </div>
          <label className="dl-check"><input type="checkbox" checked={prefs.registration} disabled={!prefs.marks} onChange={(e) => setPrefs({ registration: e.target.checked })} /> {t('Registration marks')}</label>
          <label className="dl-check"><input type="checkbox" checked={prefs.colorBars} disabled={!prefs.marks} onChange={(e) => setPrefs({ colorBars: e.target.checked })} /> {t('Colour bars')}</label>
          {!prefs.marks && <span className="dl-hint">{t('Registration marks and colour bars need crop marks and bleed.')}</span>}

          <div className="dl-field">
            <label className="dl-label" htmlFor={`${id}-profile`}>{space === 'CMYK' ? t('CMYK profile') : t('RGB profile')}</label>
            <div className="row">
              <select id={`${id}-profile`} className="input" value={current?.id ?? ''} onChange={(e) => setPrefs({ profile: e.target.value })}>
                {usable.filter((p) => p.kind !== 'custom').map((p) => <option key={p.id} value={p.id}>{p.conditionId && p.conditionId !== p.name ? `${p.name} (${p.conditionId})` : p.name}</option>)}
                {usable.some((p) => p.kind === 'custom') && (
                  <optgroup label={t('Uploaded')}>
                    {usable.filter((p) => p.kind === 'custom').map((p) => <option key={p.id} value={p.id}>{p.name}</option>)}
                  </optgroup>
                )}
              </select>
              {current?.kind === 'custom' && (
                <button type="button" className="btn ghost icon sm" title={t('Remove profile')} aria-label={t('Remove profile')} onClick={() => void remove(current)}><Trash2 size={14} /></button>
              )}
            </div>
            {current?.description && <span className="dl-hint">{current.description}</span>}
            <input ref={file} type="file" accept=".icc,.icm,application/vnd.iccprofile" hidden onChange={(e) => { const f = e.target.files?.[0]; if (f) void upload(f); }} />
            <button type="button" className="dl-link" disabled={uploading} onClick={() => file.current?.click()}>
              <Upload size={14} /> {uploading ? t('Uploading…') : t('Upload ICC profile…')}
            </button>
            {uploadErr && <span className="dl-error" role="alert">{uploadErr}</span>}
          </div>

          <div className="dl-field">
            <label className="dl-label" htmlFor={`${id}-intent`}>{t('Rendering intent')}</label>
            <select id={`${id}-intent`} className="input" value={prefs.intent} onChange={(e) => setPrefs({ intent: e.target.value as PrintPrefs['intent'] })}>
              <option value="relative" title={t('Relative colorimetric (black point compensation)')}>{t('Relative colorimetric + BPC')}</option>
              <option value="perceptual">{t('Perceptual')}</option>
            </select>
          </div>

          <div className="dl-field">
            <label className="dl-label" htmlFor={`${id}-std`}>{t('PDF standard')}</label>
            <select id={`${id}-std`} className="input" value={prefs.standard} onChange={(e) => setStandard(e.target.value as PrintPrefs['standard'])}>
              <option value="auto">{t('Automatic ({standard})', { standard: t(STANDARD_NAME[standardFor({ ...prefs, standard: 'auto' })]) })}</option>
              <option value="none">{t('None')}</option>
              <option value="pdfx4">PDF/X-4</option>
              <option value="pdfx1a">PDF/X-1a</option>
            </select>
            <span className="dl-hint">{t(STANDARD_HELP[standard])}</span>
          </div>
        </div>
      </details>
    </div>
  );
}

// ---------------------------------------------------------------- download button

function DownloadButton({ phase, progress, disabled, onClick }: {
  phase: 'idle' | 'busy' | 'done'; progress: { value: number | null; label: string }; disabled: boolean; onClick: () => void;
}) {
  const pct = progress.value === null ? null : Math.round(progress.value * 100);
  const status = phase === 'busy' ? (pct === null ? progress.label : `${progress.label} · ${pct}%`) : phase === 'done' ? t('Downloaded') : '';
  return (
    <>
      <button type="button" className={`dl-go${phase === 'busy' ? ' busy' : ''}${phase === 'done' ? ' done' : ''}`} onClick={onClick} disabled={disabled} aria-busy={phase === 'busy'} aria-disabled={phase === 'busy' || disabled}>
        {phase === 'busy' && <span className={`dl-go-fill${pct === null ? ' indeterminate' : ''}`} style={pct === null ? undefined : { width: `${pct}%` }} aria-hidden />}
        <span className="dl-go-label">
          {phase === 'idle' && <><Download size={17} /> {t('Download')}</>}
          {phase === 'busy' && status}
          {phase === 'done' && <><Check size={17} /> {t('Downloaded')}</>}
        </span>
      </button>
      <span className="sr-only" aria-live="polite">{status}</span>
    </>
  );
}

// ---------------------------------------------------------------- credits

/** Attribution for licensed library media used in the design (CC BY etc. require credit when shared). */
export function Credits({ designId }: { designId: string }) {
  const [credits, setCredits] = useState<Credit[] | null>(null);
  useEffect(() => {
    contentApi.credits(designId).then((r) => setCredits(r.credits)).catch(() => setCredits([]));
  }, [designId]);
  const needed = credits?.filter((c) => c.provider !== 'noto-emoji') ?? [];
  if (!needed.length) return null;
  const text = needed.map((c) => `${t('“{title}” by {author} ({license})', { title: c.title ?? c.name, author: c.author ?? t('unknown'), license: c.license ?? t('see source') })}${c.sourceUrl ? ` — ${c.sourceUrl}` : ''}`).join('\n');
  return (
    <details className="credits">
      <summary>{t('Credits ({n}) — include these when you share', { n: needed.length })}</summary>
      <ul>
        {needed.map((c) => (
          <li key={c.asset}>
            {interp(t('“{title}” by {author}'), {
              title: c.sourceUrl ? <a href={c.sourceUrl} target="_blank" rel="noreferrer">{c.title ?? c.name}</a> : c.title ?? c.name,
              author: c.authorUrl ? <a href={c.authorUrl} target="_blank" rel="noreferrer">{c.author ?? t('unknown')}</a> : c.author ?? t('unknown'),
            })}
            {c.license && <> · {c.licenseUrl ? <a href={c.licenseUrl} target="_blank" rel="noreferrer">{c.license}</a> : c.license}</>}
          </li>
        ))}
      </ul>
      <button className="btn sm" onClick={() => void navigator.clipboard.writeText(text)}>{t('Copy credits')}</button>
    </details>
  );
}
