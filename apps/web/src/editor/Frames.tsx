/** Frame library (A13): the Frames section of the Elements panel and toolbar controls for videos inside frames. */
import { scaleUnitPath } from '@oc/engine';
import { FRAME_CATEGORIES, FRAME_LIBRARY, makeFrame, toHex, type Element, type FrameCategory, type FrameEl, type FrameLibraryItem, type MediaClip } from '@oc/engine';
import { Film, Repeat, Volume2, VolumeX } from 'lucide-react';
import { useState } from 'react';
import { t, tk } from '../i18n';
import { useEd } from './context';

const CATEGORY_LABELS: Record<FrameCategory, string> = {
  basic: tk('Basic'), shapes: tk('Shapes'), organic: tk('Organic'), photo: tk('Photo'), devices: tk('Devices'), letters: tk('Letters'), numbers: tk('Numbers'),
};

/** Display names for library frames (letters, numbers and blobs are formatted). */
const NAME_LABELS: Record<string, string> = {
  square: tk('Square'), portrait: tk('Portrait'), rounded: tk('Rounded square'), circle: tk('Circle'), oval: tk('Oval'), squircle: tk('Squircle'),
  arch: tk('Arch'), pill: tk('Pill'), heart: tk('Heart'), star: tk('Star'), burst: tk('Starburst'), hexagon: tk('Hexagon'), octagon: tk('Octagon'),
  diamond: tk('Diamond'), triangle: tk('Triangle'), speech: tk('Speech bubble'), 'speech-rect': tk('Chat bubble'), flower: tk('Flower'),
  stamp: tk('Stamp'), cross: tk('Cross'), wavy: tk('Wavy circle'), pebble: tk('Pebble'), brush: tk('Brush stroke'), torn: tk('Torn paper'),
  polaroid: tk('Polaroid'), 'instant-mini': tk('Instant mini'), film: tk('Film strip'), slide: tk('Photo slide'), phone: tk('Phone'),
  tablet: tk('Tablet'), laptop: tk('Laptop'), monitor: tk('Desktop monitor'), watch: tk('Smartwatch'),
};

export function frameName(f: FrameLibraryItem): string {
  if (f.category === 'letters') return t('Letter {letter}', { letter: f.id.slice(-1) });
  if (f.category === 'numbers') return t('Number {digit}', { digit: f.id.slice(-1) });
  const blob = /^blob-(\d)$/.exec(f.id);
  if (blob) return t('Blob {n}', { n: Number(blob[1]) });
  return NAME_LABELS[f.id] ? t(NAME_LABELS[f.id]!) : f.name;
}

/** Fit a unit-box frame into a square preview of side `s`. */
function FramePreview({ f }: { f: FrameLibraryItem }) {
  const s = 100;
  const w = f.aspect >= 1 ? s : s * f.aspect;
  const h = f.aspect >= 1 ? s / f.aspect : s;
  return (
    <svg viewBox={`${-(s - w) / 2 - 2} ${-(s - h) / 2 - 2} ${s + 4} ${s + 4}`} aria-hidden>
      <path d={scaleUnitPath(f.d, w, h)} fill="#cbd5e1" />
      {f.overlay?.map((o, i) => <path key={i} d={scaleUnitPath(o.d, w, h)} fill={o.fill.k === 'solid' ? toHex(o.fill.color) : '#94a3b8'} />)}
    </svg>
  );
}

/** Does the library have something for this search? (Lets the Elements panel show the section.) */
export function framesMatch(q: string): boolean {
  if (!q) return true;
  const s = q.toLowerCase();
  return 'frames'.includes(s) || s.includes('frame') || FRAME_LIBRARY.some((f) => matches(f, s));
}

function matches(f: FrameLibraryItem, s: string): boolean {
  return [f.name, frameName(f), f.category, t(CATEGORY_LABELS[f.category]), f.tags ?? ''].some((v) => v.toLowerCase().includes(s));
}

const PREVIEW = 8;

export function FramesSection({ q, base }: { q: string; base: number }) {
  const { editor } = useEd();
  const [open, setOpen] = useState<FrameCategory | null>(null);
  const s = q.trim().toLowerCase();
  const all = !s || 'frames'.includes(s) || s === 'frame';
  const make = (f: FrameLibraryItem): Element => {
    const side = base * 1.2;
    const k = Math.sqrt(f.aspect);
    return makeFrame({ preset: f.id, xf: { x: 0, y: 0, w: side * k, h: side / k, rot: 0 } });
  };
  const drag = (f: FrameLibraryItem) => (e: React.DragEvent) => e.dataTransfer.setData('application/x-oc-element', JSON.stringify([make(f)]));
  return (
    <>
      <h4>{t('Frames')}</h4>
      <p className="faint" style={{ margin: 0, fontSize: 12 }}>{t('Drop a photo or video onto a frame to fill it.')}</p>
      {FRAME_CATEGORIES.map((c) => {
        const items = FRAME_LIBRARY.filter((f) => f.category === c && (all || matches(f, s)));
        if (!items.length) return null;
        const expanded = open === c || !all;
        const shown = expanded ? items : items.slice(0, PREVIEW);
        return (
          <div key={c} className="col" style={{ gap: 6 }}>
            <div className="row" style={{ justifyContent: 'space-between' }}>
              <h5 className="panel-sub">{t(CATEGORY_LABELS[c])}</h5>
              {all && items.length > PREVIEW && (
                <button className="btn ghost sm" style={{ fontSize: 12 }} onClick={() => setOpen(open === c ? null : c)}>{open === c ? t('Show less') : t('See all')}</button>
              )}
            </div>
            <div className="tiles" style={{ gridTemplateColumns: 'repeat(4, 1fr)' }}>
              {shown.map((f) => (
                <button key={f.id} className="tile" title={frameName(f)} aria-label={frameName(f)} draggable onDragStart={drag(f)} onClick={() => editor.insert([make(f)])}>
                  <FramePreview f={f} />
                </button>
              ))}
            </div>
          </div>
        );
      })}
    </>
  );
}

const SPEEDS = [0.5, 1, 1.5, 2];

/** Playback settings for a video inside a frame (A13). */
export function FrameVideoControls({ el }: { el: FrameEl }) {
  const { editor } = useEd();
  const c = el.content;
  if (!c?.video) return null;
  const v = c.video;
  const set = (patch: Partial<MediaClip>, merge = true) => editor.updateSelected({ content: { ...c, video: { ...v, ...patch } } }, false, merge);
  return (
    <div className="group">
      <Film size={15} className="faint" />
      <select className="input" style={{ width: 72 }} value={v.speed} onChange={(e) => set({ speed: +e.target.value }, false)} aria-label={t('Playback speed')}>
        {(SPEEDS.includes(v.speed) ? SPEEDS : [...SPEEDS, v.speed]).map((sp) => <option key={sp} value={sp}>{sp}×</option>)}
      </select>
      <button className={`btn ghost sm icon ${v.muted ? 'active' : ''}`} title={v.muted ? t('Unmute') : t('Mute')} onClick={() => set({ muted: !v.muted }, false)}>{v.muted ? <VolumeX size={16} /> : <Volume2 size={16} />}</button>
      {!v.muted && <input type="range" min={0} max={1} step={0.05} value={v.volume} onChange={(e) => set({ volume: +e.target.value })} style={{ width: 70 }} aria-label={t('Volume')} />}
      <button className={`btn ghost sm icon ${v.loop ? 'active' : ''}`} title={t('Loop')} onClick={() => set({ loop: !v.loop }, false)}><Repeat size={16} /></button>
    </div>
  );
}
