/**
 * Vector editing (DS-100) and masks (DS-107) in the element toolbar and the context menu: edit points, combine
 * shapes (union, subtract, intersect, exclude), outline text, turn an SVG into editable shapes, and mask with the
 * top element (alpha or brightness), switch the mask type and release it.
 */
import { maskOf, svgToElements, type BoolOp, type DesignDoc, type Editor, type Element, type ImageEl, type MaskMode, type TextEl } from '@oc/engine';
import { Blend, CircleDot, Combine, Plus, Shapes, Spline, SquareDashed, Trash2, Type, Ungroup } from 'lucide-react';
import { assetUrl } from '../api';
import { MenuItem, PopoverButton, useToast } from '../components/ui';
import { t, tk } from '../i18n';
import type { Engine } from '../lib/engine';
import { useDoc, useEd, useEditorState } from './context';

const BOOL_OPS: Array<{ op: BoolOp; label: string }> = [
  { op: 'union', label: tk('Union') },
  { op: 'subtract', label: tk('Subtract') },
  { op: 'intersect', label: tk('Intersect') },
  { op: 'exclude', label: tk('Exclude') },
];

const isSvgImage = (doc: DesignDoc, el: Element | null): el is ImageEl =>
  el?.type === 'image' && (doc.assets[el.asset]?.kind === 'svg' || doc.assets[el.asset]?.mime === 'image/svg+xml');

/** Text as a filled path in its place. False when its fonts are not loaded yet. */
export function outlineText(editor: Editor, engine: Engine, el: TextEl): boolean {
  const o = engine.painter.textOutline(editor.doc, el.id);
  if (!o) return false;
  return Boolean(editor.replaceWithPath(el.id, o.d, o.fill));
}

/** An SVG image as a group of editable path elements in its place. */
export async function convertSvg(editor: Editor, el: ImageEl): Promise<boolean> {
  const r = await fetch(`${assetUrl(el.asset)}/svg`).catch(() => null);
  const svg = r?.ok ? await r.text() : '';
  if (!svg) return false;
  const els = svgToElements(svg, el.xf.w, el.xf.h, el.crop);
  if (!els?.length) return false;
  return Boolean(editor.replaceWithElements(el.id, els));
}

/** What the selection allows. */
function useVectorScope(sel: Element[]) {
  const { editor } = useEd();
  const doc = useDoc();
  const single = sel.length === 1 ? sel[0]! : null;
  const masked = editor.maskedGroup();
  const mask = masked ? maskOf(doc.elements[masked]) : undefined;
  return {
    single,
    shapes: sel.length > 1 && sel.every((e) => e.type === 'shape'),
    canMask: sel.length > 1 && !mask && new Set(sel.map((e) => e.parentId)).size === 1,
    mask,
    svg: isSvgImage(doc, single) ? single : null,
  };
}

/** Toolbar buttons. */
export function VectorControls({ sel }: { sel: Element[] }) {
  const { editor, engine } = useEd();
  const state = useEditorState();
  const toast = useToast();
  const s = useVectorScope(sel);
  const locked = sel.some((e) => e.lock?.content);

  if (state.pathEdit) {
    const view = editor.pathEditView();
    const picked = view?.selected ?? [];
    const first = picked[0] && view?.subpaths[picked[0].s]?.nodes[picked[0].i];
    const smooth = Boolean(first?.smooth);
    return (
      <>
        <span className="muted" style={{ fontSize: 13 }}>{t('Editing points')}</span>
        <button className="btn ghost sm" aria-pressed={smooth} disabled={!picked.length} onClick={() => editor.setPathNodesSmooth(!smooth)} title={t('Smooth point (S)')}>
          <CircleDot size={16} /> {smooth ? t('Smooth') : t('Corner')}
        </button>
        <button className="btn ghost sm" onClick={() => editor.addPathNode()} title={t('Add point (+)')}><Plus size={16} /> {t('Add point')}</button>
        <button className="btn ghost sm" disabled={!picked.length} onClick={() => editor.deletePathNodes()} title={t('Delete point (Delete)')}><Trash2 size={16} /> {t('Delete point')}</button>
        <button className="btn sm" onClick={() => editor.stopPathEdit()}>{t('Done')}</button>
        <div className="sep" />
      </>
    );
  }
  return (
    <>
      {s.single?.type === 'shape' && (
        <button className="btn ghost sm" disabled={locked} onClick={() => editor.startPathEdit(s.single!.id)} title={t('Edit points')}><Spline size={16} /> {t('Edit points')}</button>
      )}
      {s.single?.type === 'text' && (
        <button className="btn ghost sm" disabled={locked} title={t('Outline text: turn the letters into an editable shape')}
          onClick={() => { if (!outlineText(editor, engine, s.single as TextEl)) toast(t('The fonts are still loading. Try again in a moment.')); }}>
          <Type size={16} /> {t('Outline text')}
        </button>
      )}
      {s.svg && (
        <button className="btn ghost sm" disabled={locked} title={t('Turn this graphic into editable shapes')}
          onClick={() => void convertSvg(editor, s.svg!).then((ok) => { if (!ok) toast(t('This graphic has no shapes that can be edited')); })}>
          <Shapes size={16} /> {t('Convert to shapes')}
        </button>
      )}
      {s.shapes && (
        <PopoverButton title={t('Combine shapes')} label={<><Combine size={16} /> {t('Combine')}</>} menu content={(close) => (
          <div className="menu" role="menu">
            {BOOL_OPS.map(({ op, label }) => (
              <MenuItem key={op} label={t(label)} onClick={() => { close(); if (!editor.booleanOp(op)) toast(t('Nothing is left after this operation')); }} />
            ))}
          </div>
        )} />
      )}
      {s.canMask && (
        <button className="btn ghost sm" onClick={() => editor.maskSelection('alpha')} title={t('Mask with the top element')}><SquareDashed size={16} /> {t('Mask')}</button>
      )}
      {s.mask && <MaskModeControls mode={s.mask.mode} />}
    </>
  );
}

function MaskModeControls({ mode }: { mode: MaskMode }) {
  const { editor } = useEd();
  return (
    <>
      <label className="row" style={{ gap: 6, fontSize: 13 }}>
        <Blend size={16} aria-hidden />
        <span className="sr-only">{t('Mask type')}</span>
        <select className="input sm" value={mode} aria-label={t('Mask type')} onChange={(e) => editor.setMaskMode(e.target.value as MaskMode)}>
          <option value="alpha">{t('Shape (alpha)')}</option>
          <option value="luminance">{t('Brightness (luminance)')}</option>
        </select>
      </label>
      <button className="btn ghost sm" onClick={() => editor.releaseMask()} title={t('Release mask')}><Ungroup size={16} /> {t('Release mask')}</button>
    </>
  );
}

/** Context menu entries. */
export function VectorMenuItems({ sel, run }: { sel: Element[]; run: (fn: () => void) => () => void }) {
  const { editor, engine } = useEd();
  const toast = useToast();
  const s = useVectorScope(sel);
  const items: React.ReactNode[] = [];
  if (s.single?.type === 'shape') items.push(<MenuItem key="pts" icon={<Spline size={16} />} label={t('Edit points')} onClick={run(() => editor.startPathEdit(s.single!.id))} />);
  if (s.single?.type === 'text') {
    items.push(<MenuItem key="ol" icon={<Type size={16} />} label={t('Outline text')}
      onClick={run(() => { if (!outlineText(editor, engine, s.single as TextEl)) toast(t('The fonts are still loading. Try again in a moment.')); })} />);
  }
  if (s.svg) {
    items.push(<MenuItem key="svg" icon={<Shapes size={16} />} label={t('Convert to shapes')}
      onClick={run(() => void convertSvg(editor, s.svg!).then((ok) => { if (!ok) toast(t('This graphic has no shapes that can be edited')); }))} />);
  }
  if (s.shapes) {
    for (const { op, label } of BOOL_OPS) {
      items.push(<MenuItem key={op} icon={<Combine size={16} />} label={t('Combine: {op}', { op: t(label) })} onClick={run(() => { if (!editor.booleanOp(op)) toast(t('Nothing is left after this operation')); })} />);
    }
  }
  if (s.canMask) {
    items.push(<MenuItem key="mask" icon={<SquareDashed size={16} />} label={t('Mask with the top element')} onClick={run(() => editor.maskSelection('alpha'))} />);
  }
  if (s.mask) {
    const other: MaskMode = s.mask.mode === 'alpha' ? 'luminance' : 'alpha';
    items.push(<MenuItem key="mode" icon={<Blend size={16} />} label={other === 'luminance' ? t('Mask by brightness') : t('Mask by shape')} onClick={run(() => editor.setMaskMode(other))} />);
    items.push(<MenuItem key="rel" icon={<Ungroup size={16} />} label={t('Release mask')} onClick={run(() => editor.releaseMask())} />);
  }
  return items.length ? <><div className="hsep" />{items}</> : null;
}
