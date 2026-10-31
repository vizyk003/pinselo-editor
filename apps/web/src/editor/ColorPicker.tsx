import { hex, toCss, toHex, type ColorValue, type DesignDoc, type Paint } from '@oc/engine';
import { Pipette } from 'lucide-react';
import { useEffect, useMemo, useState } from 'react';
import { t } from '../i18n';
import { activeControls, brandColors as kitColors, brandStore, useBrandHub } from './brandStore';

/** Brand kit colours across every kit the user can see (A8), plus the "brand colours only" control. */
function useBrandColors(): { colors: string[]; only: string[] | null } {
  const hub = useBrandHub();
  useEffect(() => {
    void brandStore.loadKits();
  }, []);
  return { colors: kitColors(hub), only: activeControls(hub).colors };
}
import { PopoverButton } from '../components/ui';

const PALETTE = [
  '#000000', '#545454', '#737373', '#a6a6a6', '#d9d9d9', '#ffffff', '#ff3131', '#ff5757',
  '#ff66c4', '#cb6ce6', '#8c52ff', '#5e17eb', '#0097b2', '#0cc0df', '#5ce1e6', '#38b6ff',
  '#5271ff', '#004aad', '#00bf63', '#7ed957', '#c1ff72', '#ffde59', '#ffbd59', '#ff914d',
  '#7c3aed', '#db2777', '#ea580c', '#16a34a', '#0891b2', '#1e293b', '#fef3c7', '#fce7f3',
];

const GRADIENTS: Array<[string, string, number]> = [
  ['#7c3aed', '#06b6d4', 135], ['#f97316', '#ec4899', 135], ['#22c55e', '#0ea5e9', 135], ['#0f172a', '#334155', 180],
  ['#fde047', '#f97316', 90], ['#f0abfc', '#c4b5fd', 90], ['#ff5757', '#8c52ff', 45], ['#5ce1e6', '#5271ff', 180],
  ['#fef3c7', '#fda4af', 135], ['#000000', '#5e17eb', 135], ['#bbf7d0', '#22d3ee', 90], ['#ffffff', '#d4d4d8', 180],
];

/** Every solid colour used in the document (most frequent first) – Canva's "Document colours". */
export function documentColors(doc: DesignDoc): string[] {
  const count = new Map<string, number>();
  const add = (c: ColorValue | undefined) => {
    if (!c) return;
    const h = toHex(c);
    count.set(h, (count.get(h) ?? 0) + 1);
  };
  const addPaint = (p: Paint | undefined) => {
    if (!p) return;
    if (p.k === 'solid') add(p.color);
    else if (p.k === 'linear' || p.k === 'radial') p.stops.forEach((s) => add(s.color));
  };
  for (const p of Object.values(doc.pages)) p.background.forEach(addPaint);
  for (const e of Object.values(doc.elements)) {
    if (e.deleted) continue;
    if (e.type === 'shape') {
      e.fills.forEach(addPaint);
      e.strokes.forEach((s) => addPaint(s.paint));
    } else if (e.type === 'line') addPaint(e.stroke.paint);
    else if (e.type === 'text') {
      addPaint(e.body.defaultRun.fill);
      e.body.paragraphs.forEach((p) => p.runs.forEach((r) => addPaint(r.style.fill)));
    }
  }
  return [...count.entries()].sort((a, b) => b[1] - a[1]).map(([h]) => h).slice(0, 16);
}

export function paintCss(p: Paint | undefined): string {
  if (!p) return 'transparent';
  if (p.k === 'solid') return toCss(p.color);
  if (p.k === 'linear') return `linear-gradient(${p.angle + 90}deg, ${p.stops.map((s) => `${toCss(s.color)} ${s.t * 100}%`).join(', ')})`;
  if (p.k === 'radial') return `radial-gradient(${p.stops.map((s) => `${toCss(s.color)} ${s.t * 100}%`).join(', ')})`;
  return '#ccc';
}

interface Props {
  value: Paint | undefined;
  onChange: (p: Paint) => void;
  /** The design whose colours are offered as "Document colours" (none outside the editor). */
  doc?: DesignDoc;
  allowGradient?: boolean;
  allowTransparent?: boolean;
}

export function ColorPanel({ value, onChange, doc, allowGradient, allowTransparent }: Props) {
  const current = value?.k === 'solid' ? toHex(value.color) : value?.k === 'linear' || value?.k === 'radial' ? toHex(value.stops[0]!.color) : '#000000';
  const [hexText, setHexText] = useState(current);
  const { colors: brandColors, only } = useBrandColors();
  const allDocColors = useMemo(() => (doc ? documentColors(doc) : []), [doc]);
  // Brand control "brand colours only": no free picking, only kit colours (and document colours that are on-brand).
  const docColors = only ? allDocColors.filter((c) => only.includes(c.toLowerCase())) : allDocColors;
  // The current colour is ringed once, in the first list that has it (white is in Document and Default colours).
  const has = (list: string[]) => list.some((c) => c.toLowerCase() === current.toLowerCase());
  const selIn = has(brandColors) ? 'brand' : has(docColors) ? 'doc' : 'default';
  const set = (h: string) => {
    try {
      onChange({ k: 'solid', color: hex(h) });
      setHexText(h);
    } catch {
      /* invalid hex while typing */
    }
  };
  const eyedrop = async () => {
    const ED = (window as unknown as { EyeDropper?: new () => { open(): Promise<{ sRGBHex: string }> } }).EyeDropper;
    if (!ED) {
      // Fallback (Firefox, Safari): the next click on the design samples the rendered colour there.
      window.dispatchEvent(new CustomEvent('oc:eyedrop', { detail: { resolve: (hexColor: string) => set(hexColor) } }));
      return;
    }
    try {
      const r = await new ED().open();
      set(r.sRGBHex);
    } catch {
      /* cancelled */
    }
  };
  return (
    <div className="cp">
      {only && <p className="faint" style={{ margin: 0, fontSize: 12 }}>{t('Your team limits colours to the brand kit.')}</p>}
      {!only && <div className="row">
        <input type="color" value={current} onChange={(e) => set(e.target.value)} style={{ width: 40, height: 36, border: 0, padding: 0, background: 'none' }} aria-label={t('Custom colour')} />
        <input className="input" value={hexText} onChange={(e) => { setHexText(e.target.value); if (/^#[0-9a-f]{6}$/i.test(e.target.value)) set(e.target.value); }} aria-label={t('Hex colour')} />
        <button className="btn icon sm" title={'EyeDropper' in window ? t('Pick a colour from the screen') : t('Pick a colour from the design (then click it)')} onClick={eyedrop}><Pipette size={15} /></button>
      </div>}
      {brandColors.length > 0 && (
        <>
          <h4 style={{ margin: 0, fontSize: 12 }} className="muted">{t('Brand colours')}</h4>
          <div className="sw">
            {brandColors.map((c) => <button key={c} style={{ background: c }} className={selIn === 'brand' && c === current ? 'sel' : ''} onClick={() => set(c)} title={c} aria-label={c} />)}
          </div>
        </>
      )}
      {docColors.length > 0 && (
        <>
          <h4 style={{ margin: 0, fontSize: 12 }} className="muted">{t('Document colours')}</h4>
          <div className="sw">
            {docColors.map((c) => <button key={c} style={{ background: c }} className={selIn === 'doc' && c === current ? 'sel' : ''} onClick={() => set(c)} title={c} aria-label={c} />)}
          </div>
        </>
      )}
      {!only && <><h4 style={{ margin: 0, fontSize: 12 }} className="muted">{t('Default colours')}</h4>
      <div className="sw">
        {allowTransparent && (
          <button title={t('Transparent')} aria-label={t('Transparent')} onClick={() => onChange({ k: 'solid', color: { space: 'srgb', r: 1, g: 1, b: 1, a: 0 } })}
            style={{ background: 'repeating-conic-gradient(#ccc 0 25%, #fff 0 50%) 0 0 / 10px 10px' }} />
        )}
        {PALETTE.map((c) => <button key={c} style={{ background: c }} className={selIn === 'default' && c === current ? 'sel' : ''} onClick={() => set(c)} title={c} aria-label={c} />)}
      </div></>}
      {allowGradient && !only && (
        <>
          <h4 style={{ margin: 0, fontSize: 12 }} className="muted">{t('Gradients')}</h4>
          <div className="grad">
            {GRADIENTS.map(([a, b, angle]) => {
              const p: Paint = { k: 'linear', angle, stops: [{ t: 0, color: hex(a) }, { t: 1, color: hex(b) }] };
              return <button key={a + b} style={{ background: paintCss(p) }} onClick={() => onChange(p)} aria-label={t('Gradient {from} to {to}', { from: a, to: b })} />;
            })}
            {GRADIENTS.slice(0, 4).map(([a, b]) => {
              const p: Paint = { k: 'radial', stops: [{ t: 0, color: hex(a) }, { t: 1, color: hex(b) }] };
              return <button key={`r${a}${b}`} style={{ background: paintCss(p) }} onClick={() => onChange(p)} aria-label={t('Radial {from} to {to}', { from: a, to: b })} />;
            })}
          </div>
          {(value?.k === 'linear' || value?.k === 'radial') && (
            <div className="row">
              <input type="color" value={toHex(value.stops[0]!.color)} onChange={(e) => onChange({ ...value, stops: [{ t: 0, color: hex(e.target.value) }, ...value.stops.slice(1)] } as Paint)} aria-label={t('Gradient start')} />
              <input type="color" value={toHex(value.stops.at(-1)!.color)} onChange={(e) => onChange({ ...value, stops: [...value.stops.slice(0, -1), { t: 1, color: hex(e.target.value) }] } as Paint)} aria-label={t('Gradient end')} />
              {value.k === 'linear' && (
                <label className="row grow" style={{ fontSize: 12 }}>
                  {t('Angle')}
                  <input type="range" min={0} max={360} value={value.angle} onChange={(e) => onChange({ ...value, angle: +e.target.value })} className="grow" />
                </label>
              )}
            </div>
          )}
        </>
      )}
    </div>
  );
}

export function ColorButton(props: Props & { title: string; variant?: 'swatch' | 'text' }) {
  return (
    <PopoverButton
      className={props.variant === 'text' ? 'text-color-btn' : 'swatch-btn'}
      title={props.title}
      label={props.variant === 'text' ? (<><span>A</span><i style={{ background: paintCss(props.value) }} /></>) : <span style={{ background: paintCss(props.value) }} />}
      content={() => <ColorPanel {...props} />}
    />
  );
}

/**
 * A themed colour swatch for a single hex value (brush colour, caption colour, brand colour…). It opens the in-app
 * colour panel (brand, document and default colours, hex, eyedropper) rather than the browser's native picker.
 */
export function SwatchInput({ value, onChange, title, doc }: { value: string; onChange: (hex: string) => void; title: string; doc?: DesignDoc }) {
  return (
    <ColorButton title={title} doc={doc} value={{ k: 'solid', color: hex(value) }} onChange={(p) => p.k === 'solid' && onChange(toHex(p.color))} />
  );
}
