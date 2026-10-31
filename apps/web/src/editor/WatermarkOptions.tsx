/**
 * Export watermark (plan 11 g64): a text mark the render service adds on top of every exported page (images, PDFs,
 * SVG and ZIP). The design itself is not changed. The last settings are remembered on this device.
 */
import { useId, useState } from 'react';
import { t, tk } from '../i18n';

export type WatermarkPosition = 'center' | 'tiled' | 'top-left' | 'top-right' | 'bottom-left' | 'bottom-right';
export interface Watermark { text: string; position: WatermarkPosition; opacity: number; rotation: number; size: number; color: string }
export interface WatermarkState { on: boolean; mark: Watermark }

const KEY = 'oc:export-watermark';
const DEFAULT: WatermarkState = { on: false, mark: { text: '', position: 'center', opacity: 0.3, rotation: -30, size: 0.08, color: '#ffffff' } };
const POSITIONS: Array<[WatermarkPosition, string]> = [
  ['center', tk('Centre')], ['tiled', tk('Tiled across the page')], ['top-left', tk('Top left')], ['top-right', tk('Top right')],
  ['bottom-left', tk('Bottom left')], ['bottom-right', tk('Bottom right')],
];
const COLORS: Array<[string, string]> = [['#ffffff', tk('White')], ['#000000', tk('Black')], ['#6b7280', tk('Grey')]];

function load(): WatermarkState {
  try {
    const v = JSON.parse(localStorage.getItem(KEY) ?? 'null') as Partial<WatermarkState> | null;
    return v && typeof v === 'object' ? { on: Boolean(v.on), mark: { ...DEFAULT.mark, ...v.mark } } : DEFAULT;
  } catch {
    return DEFAULT;
  }
}

/** Watermark settings with persistence; `request` is what the export API takes (undefined when off or empty). */
export function useWatermark() {
  const [state, setState] = useState<WatermarkState>(load);
  const update = (p: Partial<WatermarkState> | ((s: WatermarkState) => WatermarkState)) => setState((s) => {
    const next = typeof p === 'function' ? p(s) : { ...s, ...p };
    try {
      localStorage.setItem(KEY, JSON.stringify(next));
    } catch {
      /* private mode */
    }
    return next;
  });
  const request = state.on && state.mark.text.trim() ? { ...state.mark, text: state.mark.text.trim() } : undefined;
  return { state, update, request };
}

export function WatermarkOptions({ state, update }: ReturnType<typeof useWatermark>) {
  const id = useId();
  const m = state.mark;
  const setMark = (p: Partial<Watermark>) => update((s) => ({ ...s, mark: { ...s.mark, ...p } }));
  return (
    <div className="dl-field">
      <label className="dl-check"><input type="checkbox" checked={state.on} onChange={(e) => update({ on: e.target.checked })} /> {t('Add a watermark')}</label>
      {state.on && (
        <div className="dl-watermark">
          <label className="dl-label" htmlFor={`${id}-text`}>{t('Watermark text')}</label>
          <input id={`${id}-text`} className="input" maxLength={120} value={m.text} placeholder={t('For example: DRAFT or © Your name')} onChange={(e) => setMark({ text: e.target.value })} />
          <label className="dl-label" htmlFor={`${id}-pos`}>{t('Position')}</label>
          <select id={`${id}-pos`} className="input" value={m.position} onChange={(e) => {
            const position = e.target.value as WatermarkPosition;
            const corner = position !== 'center' && position !== 'tiled';
            setMark({ position, rotation: corner ? 0 : m.rotation || -30 });
          }}>
            {POSITIONS.map(([v, l]) => <option key={v} value={v}>{t(l)}</option>)}
          </select>
          <div className="dl-label-row">
            <label className="dl-label" htmlFor={`${id}-op`}>{t('Opacity')}</label>
            <output className="dl-value" htmlFor={`${id}-op`}>{Math.round(m.opacity * 100)}%</output>
          </div>
          <input id={`${id}-op`} type="range" min={0.05} max={1} step={0.05} value={m.opacity} onChange={(e) => setMark({ opacity: +e.target.value })} />
          <div className="dl-label-row">
            <label className="dl-label" htmlFor={`${id}-rot`}>{t('Rotation')}</label>
            <output className="dl-value" htmlFor={`${id}-rot`}>{m.rotation}°</output>
          </div>
          <input id={`${id}-rot`} type="range" min={-90} max={90} step={5} value={m.rotation} onChange={(e) => setMark({ rotation: +e.target.value })} />
          <div className="dl-label-row">
            <label className="dl-label" htmlFor={`${id}-size`}>{t('Size')}</label>
            <output className="dl-value" htmlFor={`${id}-size`}>{Math.round(m.size * 100)}</output>
          </div>
          <input id={`${id}-size`} type="range" min={0.03} max={0.2} step={0.01} value={m.size} onChange={(e) => setMark({ size: +e.target.value })} />
          <div className="row" role="radiogroup" aria-label={t('Watermark colour')} style={{ gap: 6 }}>
            {COLORS.map(([c, l]) => (
              <button key={c} type="button" role="radio" aria-checked={m.color === c} className={`btn sm ${m.color === c ? 'active' : ''}`} onClick={() => setMark({ color: c })}>
                <span aria-hidden style={{ width: 12, height: 12, borderRadius: 3, background: c, border: '1px solid var(--border-strong)', display: 'inline-block' }} /> {t(l)}
              </button>
            ))}
          </div>
        </div>
      )}
    </div>
  );
}
