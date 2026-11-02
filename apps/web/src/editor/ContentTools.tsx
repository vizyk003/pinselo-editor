/**
 * Toolbar pieces for text and images (plan 11): superscript/subscript and the vertical anchor of a text box (g67),
 * replace image, image border and "set as background" (g63), the page background image (g63), and the grouped
 * filter library (g58).
 */
import {
  hex, solid, type Effect, type Element, type ImageEditStack, type ImageFilterPreset, type ImageEl, type Page, type RunStyle, type TextEl,
} from '@oc/engine';
import {
  AlignVerticalJustifyCenter, AlignVerticalJustifyEnd, AlignVerticalJustifyStart, ImageUp, Replace, SquareDashed, Subscript, Superscript, Wallpaper,
} from 'lucide-react';
import { useRef } from 'react';
import { api } from '../api';
import { PopoverButton, useToast } from '../components/ui';
import { t, tk } from '../i18n';
import { ColorButton } from './ColorPicker';
import { setImageAsBackground } from './ElementMenu';
import { applyAll, fillTarget } from './insert';
import { useDoc, useEd } from './context';
import { userLock } from './dropTarget';

// ------------------------------------------------------------------ text (g67)

/** Superscript and subscript toggles for the selected text (or the whole text boxes when not editing). */
export function ScriptButtons({ st, set }: { st: RunStyle; set: (m: Partial<RunStyle> & Record<string, unknown>) => void }) {
  return (
    <>
      <button className={`btn ghost sm icon ${st.baseline === 'super' ? 'active' : ''}`} title={t('Superscript')} aria-label={t('Superscript')} aria-pressed={st.baseline === 'super'}
        onClick={() => set({ baseline: (st.baseline === 'super' ? null : 'super') as RunStyle['baseline'] })}><Superscript size={16} /></button>
      <button className={`btn ghost sm icon ${st.baseline === 'sub' ? 'active' : ''}`} title={t('Subscript')} aria-label={t('Subscript')} aria-pressed={st.baseline === 'sub'}
        onClick={() => set({ baseline: (st.baseline === 'sub' ? null : 'sub') as RunStyle['baseline'] })}><Subscript size={16} /></button>
    </>
  );
}

const ANCHORS = [
  { id: 'top', label: tk('Top'), Icon: AlignVerticalJustifyStart },
  { id: 'middle', label: tk('Middle'), Icon: AlignVerticalJustifyCenter },
  { id: 'bottom', label: tk('Bottom'), Icon: AlignVerticalJustifyEnd },
] as const;

/**
 * Where the text sits in its box. Middle and bottom need a box taller than the text, so they make the box keep its
 * height (it stops growing with the text); top puts the box back to growing.
 */
export function TextAnchor({ els }: { els: TextEl[] }) {
  const { editor } = useEd();
  const plain = els.filter((e) => !e.curve);
  if (!plain.length) return null;
  const cur = plain[0]!.valign ?? 'top';
  const Icon = ANCHORS.find((a) => a.id === cur)!.Icon;
  const setAnchor = (v: (typeof ANCHORS)[number]['id']) => editor.updateSelected((x) => {
    if (x.type !== 'text' || x.curve) return null;
    if (v === 'top') return { valign: null };
    return x.frame.mode === 'fixed' ? { valign: v } : { valign: v, frame: { mode: 'fixed' }, autoFit: 'none' };
  }, true, false);
  return (
    <PopoverButton title={t('Vertical position')} className="btn ghost sm icon" label={<Icon size={16} />} content={(close) => (
      <div className="pop-sm col" role="radiogroup" aria-label={t('Vertical position')}>
        {ANCHORS.map(({ id, label, Icon: I }) => (
          <button key={id} role="radio" aria-checked={cur === id} className={`btn ghost sm ${cur === id ? 'active' : ''}`} style={{ justifyContent: 'flex-start' }} onClick={() => { setAnchor(id); close(); }}>
            <I size={16} aria-hidden /> {t(label)}
          </button>
        ))}
        <span className="faint" style={{ fontSize: 11 }}>{t('Middle and bottom keep the box height; drag its top or bottom edge to make room.')}</span>
      </div>
    )} />
  );
}

// ------------------------------------------------------------------ images (g63)

/** Replace the picture of an image element with a file, keeping its place, size and crop centre. */
export function ReplaceImageButton({ el }: { el: ImageEl }) {
  const { editor } = useEd();
  const toast = useToast();
  const input = useRef<HTMLInputElement>(null);
  const locked = Boolean(userLock(el)?.content);
  return (
    <>
      <button className="btn ghost sm" disabled={locked} onClick={() => input.current?.click()} title={locked ? t('Locked') : t('Replace with a file from your device (or drag another photo onto it)')}>
        <Replace size={16} /> {t('Replace')}
      </button>
      <input ref={input} type="file" hidden accept="image/*" onChange={(e) => {
        const f = e.target.files?.[0];
        e.target.value = '';
        if (!f) return;
        api.upload(f)
          .then(({ asset }) => fillTarget(editor, { kind: 'image', id: el.id }, asset) || toast(t('Couldn’t change the image.')))
          .catch((x) => toast(t('Upload failed: {message}', { message: (x as Error).message })));
      }} />
    </>
  );
}

const outlineOf = (el: Element) => el.effects?.find((e): e is Extract<Effect, { k: 'outline' }> => e.k === 'outline');

/** Image border: the `outline` effect, drawn as an inside stroke along the image's (rounded) edge. */
export function ImageBorder({ el }: { el: ImageEl }) {
  const { editor } = useEd();
  const doc = useDoc();
  const o = outlineOf(el);
  const max = Math.max(4, Math.round(Math.min(el.xf.w, el.xf.h) / 4));
  const set = (patch: { width?: number; color?: ReturnType<typeof hex> } | null) => editor.updateSelected((x) => {
    if (x.type !== 'image') return null;
    const rest = (x.effects ?? []).filter((e) => e.k !== 'outline');
    const cur = outlineOf(x);
    if (!patch) return { effects: rest.length ? rest : null };
    const next: Effect = { k: 'outline', width: patch.width ?? cur?.width ?? Math.max(2, Math.round(Math.min(x.xf.w, x.xf.h) / 40)), color: patch.color ?? cur?.color ?? hex('#ffffff') };
    return { effects: [...rest, next] };
  });
  return (
    <PopoverButton title={t('Border')} className={`btn ghost sm icon ${o ? 'active' : ''}`} label={<SquareDashed size={16} />} content={() => (
      <div className="col pop-sm">
        <label className="field">{t('Border weight ({value})', { value: o?.width ?? 0 })}
          <input type="range" min={0} max={max} value={o?.width ?? 0} onChange={(e) => (+e.target.value ? set({ width: +e.target.value }) : set(null))} />
        </label>
        <div className="row">
          <span className="grow" style={{ fontSize: 13 }}>{t('Border colour')}</span>
          <ColorButton title={t('Border colour')} doc={doc} value={solid(o?.color ?? hex('#ffffff'))} onChange={(p) => p.k === 'solid' && set({ color: p.color })} />
        </div>
        {o && <button className="btn sm" onClick={() => set(null)}>{t('Remove border')}</button>}
      </div>
    )} />
  );
}

/** Make the selected image the page background (covering the page, centred on its crop) and remove it from the page. */
export function SetAsBackgroundButton({ el }: { el: ImageEl }) {
  const { editor } = useEd();
  if (el.svgColors) return null;
  return (
    <button className="btn ghost sm" title={t('Use this image as the page background')} onClick={() => setImageAsBackground(editor, el)}>
      <Wallpaper size={16} /> {t('Set as background')}
    </button>
  );
}

/** Page background image: upload one, change how it fits, or remove it (the colour picker sets plain colours). */
export function PageBackgroundImage({ page }: { page: Page }) {
  const { editor } = useEd();
  const toast = useToast();
  const input = useRef<HTMLInputElement>(null);
  const img = page.background.find((p): p is Extract<typeof p, { k: 'image' }> => p.k === 'image');
  return (
    <>
      <input ref={input} type="file" hidden accept="image/*" onChange={(e) => {
        const f = e.target.files?.[0];
        e.target.value = '';
        if (!f) return;
        api.upload(f)
          // The asset is registered with the design in the same undo step (exports look it up there).
          .then(({ asset }) => applyAll(editor, [{ op: 'asset', asset }, { op: 'page', action: 'update', pageId: page.id, patch: { background: [{ k: 'image', asset: asset.id, fit: 'fill' }] } }]))
          .catch((x) => toast(t('Upload failed: {message}', { message: (x as Error).message })));
      }} />
      {!img ? (
        <button className="btn ghost sm" onClick={() => input.current?.click()} title={t('Upload an image for the page background')}><ImageUp size={16} /> {t('Background image')}</button>
      ) : (
        <PopoverButton title={t('Background image')} label={<><ImageUp size={16} /> {t('Background image')}</>} content={(close) => (
          <div className="col pop-sm">
            <label className="field">{t('Fit')}
              <select className="input" value={img.crop ? 'fill' : img.fit} onChange={(e) => editor.updatePage(page.id, { background: [{ k: 'image', asset: img.asset, fit: e.target.value as 'fill' | 'fit' | 'tile' }] })}>
                <option value="fill">{t('Fill the page')}</option>
                <option value="fit">{t('Fit inside the page')}</option>
                <option value="tile">{t('Tile')}</option>
              </select>
            </label>
            <button className="btn sm" onClick={() => input.current?.click()}><Replace size={14} /> {t('Replace image')}</button>
            <button className="btn sm" onClick={() => { editor.updatePage(page.id, { background: [solid(hex('#ffffff'))] }); close(); }}>{t('Remove background image')}</button>
          </div>
        )} />
      )}
    </>
  );
}

// ------------------------------------------------------------------ filters (g58)

export const FILTER_LABELS: Record<string, string> = {
  none: tk('None'), vivid: tk('Vivid'), crisp: tk('Crisp'), chrome: tk('Chrome'), dramatic: tk('Dramatic'), warm: tk('Warm'), summer: tk('Summer'),
  cool: tk('Cool'), 'teal-orange': tk('Teal Orange'), dusk: tk('Dusk'), fade: tk('Fade'), matte: tk('Matte'), pastel: tk('Pastel'), blush: tk('Blush'),
  forest: tk('Forest'), vintage: tk('Vintage'), retro: tk('Retro'), lomo: tk('Lomo'), sepia: tk('Sepia'), mono: tk('Mono'), noir: tk('Noir'),
  silver: tk('Silver'), contrast: tk('High contrast'), selenium: tk('Selenium'), cyanotype: tk('Cyanotype'), invert: tk('Invert'), infrared: tk('Infrared'),
  golden: tk('Golden'), arctic: tk('Arctic'), rose: tk('Rose'), mint: tk('Mint'), lavender: tk('Lavender'), sunset: tk('Sunset'), duo: tk('Duo'), neon: tk('Neon'),
};
const FILTER_GROUPS: Array<{ id: string; label: string; presets: ImageFilterPreset[] }> = [
  { id: 'colour', label: tk('Colour'), presets: ['vivid', 'crisp', 'chrome', 'dramatic', 'warm', 'summer', 'golden', 'cool', 'arctic', 'teal-orange', 'dusk', 'forest'] },
  { id: 'soft', label: tk('Soft and retro'), presets: ['fade', 'matte', 'pastel', 'blush', 'vintage', 'retro', 'lomo'] },
  { id: 'bw', label: tk('Black and white'), presets: ['mono', 'noir', 'silver', 'contrast', 'selenium', 'sepia'] },
  { id: 'tint', label: tk('Tinted'), presets: ['cyanotype', 'rose', 'mint', 'lavender', 'sunset'] },
  { id: 'special', label: tk('Special'), presets: ['invert', 'infrared', 'duo', 'neon'] },
];
const titleCase = (s: string) => s.split('-').map((w) => w[0]!.toUpperCase() + w.slice(1)).join(' ');

/** The filter library, grouped (Colour, Soft and retro, Black and white, Tinted, Special). */
export function FilterGroups({ cur, setEdits }: { cur: ImageEditStack; setEdits: (e: ImageEditStack) => void }) {
  const active = cur.filter?.preset ?? 'none';
  const btn = (f: ImageFilterPreset) => (
    <button key={f} className={`btn sm ${active === f ? 'active' : ''}`} aria-pressed={active === f} onClick={() => setEdits({ ...cur, filter: { preset: f, amount: cur.filter?.amount ?? 1 } })}>
      {FILTER_LABELS[f] ? t(FILTER_LABELS[f]) : titleCase(f)}
    </button>
  );
  return (
    <div className="scroll filter-groups" style={{ maxHeight: 220, display: 'grid', gap: 6 }}>
      <div style={{ display: 'grid', gridTemplateColumns: 'repeat(3, 1fr)', gap: 6 }}>{btn('none')}</div>
      {FILTER_GROUPS.map((g) => (
        <div key={g.id} role="group" aria-labelledby={`flt-${g.id}`}>
          <div className="font-sec" id={`flt-${g.id}`}>{t(g.label)}</div>
          <div style={{ display: 'grid', gridTemplateColumns: 'repeat(3, 1fr)', gap: 6 }}>{g.presets.map(btn)}</div>
        </div>
      ))}
    </div>
  );
}

/** Pixelate (g61): blocks the whole picture for redaction; 0 turns it off. Also available on photos in frames. */
export function PixelateControl({ cur, setEdits }: { cur: ImageEditStack; setEdits: (e: ImageEditStack) => void }) {
  const v = cur.adjust?.pixelate ?? 0;
  return (
    <label className="field">{t('Pixelate ({value})', { value: v })}
      <input type="range" min={0} max={100} value={v} aria-valuetext={v ? t('{value} of 100', { value: v }) : t('Off')}
        onChange={(e) => setEdits({ ...cur, adjust: { ...cur.adjust, pixelate: +e.target.value || undefined } })} />
    </label>
  );
}

