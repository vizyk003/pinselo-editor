/** Contextual toolbar (docs/plan/04-ux-spec.md §3.4): contents depend on the selection. */
import type { AlignEdge } from '@oc/engine';
import {
  GRID_LAYOUTS, hex, solid, toHex, type BlendMode, type Effect, type Element, type FrameEl, type GridEl, type ImageEditStack, type LineMarker, LINE_MARKERS,
  type Paint, type RunStyle, type ShapeEl, type Stroke, type TextEffect, type TextEl,
} from '@oc/engine';
import {
  AlignCenter, AlignHorizontalJustifyCenter, AlignHorizontalJustifyEnd, AlignHorizontalJustifyStart, AlignJustify, AlignLeft, AlignRight,
  AlignVerticalJustifyCenter, AlignVerticalJustifyEnd, AlignVerticalJustifyStart, ArrowDownToLine, ArrowUpToLine, Bold, CaseSensitive, Copy,
  Crop, Group, Italic, Layers, Link2, List, ListOrdered, Lock, Minus, Plus, SlidersHorizontal, Sparkles,
  Strikethrough, Trash2, Underline, Ungroup, Unlock, ArrowUp, ArrowDown, Blend, LayoutGrid, Maximize2, Paintbrush, Accessibility, Spline, Replace, ImageOff,
  SquareRoundCorner, TrianglesCenterlineDashedHorizontal, TrianglesCenterlineDashedVertical,
} from 'lucide-react';
import { useMemo, useRef, useState } from 'react';
import { PopoverButton, useToast } from '../components/ui';
import { ColorButton } from './ColorPicker';
import { t, tk } from '../i18n';
import { useDoc, useEd, useEditorState } from './context';
import { FilterGroups, ImageBorder, PageBackgroundImage, PixelateControl, ReplaceImageButton, ScriptButtons, SetAsBackgroundButton, TextAnchor } from './ContentTools';
import { FontPicker } from './FontPicker';
import { FormControls } from './FormTools';
import { SvgColorControls } from './SvgColors';
import { VectorControls } from './VectorTools';
import { FrameVideoControls } from './Frames';
import { AnimateButton, ChartControls, DrawControls, TableControls, VideoControls } from './ToolbarExtras';
import { fillColorOf, shapePhotoFill, withFillColor } from './dropTarget';
import { detachPhoto, removePhoto, replacePhoto } from './insert';
import { userLock, userLocked } from './dropTarget';

export function Toolbar() {
  const { editor, readOnly } = useEd();
  const state = useEditorState();
  const doc = useDoc();
  if (readOnly) return <div className="ctx-toolbar"><span className="muted">{t('View only — you can comment if the owner allowed it.')}</span></div>;
  const sel = state.selection.map((id) => doc.elements[id]).filter((e): e is Element => Boolean(e && !e.deleted));
  const single = sel.length === 1 ? sel[0]! : null;
  const deep = editor.selectedDeep();
  // Table cells being edited (caret or cell range) use the same text toolbar (A12).
  const hasText = deep.some((e) => e.type === 'text') || editor.editingTableText;

  if (state.croppingId) return <CropBar />;

  if (!sel.length) {
    const page = doc.pages[state.activePageId];
    return (
      <div className="ctx-toolbar">
        <span className="muted" style={{ fontSize: 13, marginRight: 4 }}>{t('Background')}</span>
        {page && (
          <ColorButton title={t('Background colour')} doc={doc} value={page.background[0]} allowGradient allowTransparent
            onChange={(p) => editor.updatePage(page.id, { background: [p] })} />
        )}
        {page && <PageBackgroundImage page={page} />}
        <div className="sep" />
        <button className="btn ghost sm" onClick={() => page && editor.updatePage(page.id, { hidden: !page.hidden })}>{page?.hidden ? t('Show page') : t('Hide page')}</button>
        <AnimateButton sel={[]} page={page} />
        <div className="grow" />
        <span className="faint ctx-tip" style={{ fontSize: 12 }}>{t('Tip: drag images onto the page · double-click text to edit · hold Space to pan')}</span>
      </div>
    );
  }

  return (
    <div
      className="ctx-toolbar" role="toolbar" aria-label={t('Element options')}
      // Keep focus in the text IME proxy while editing so typing continues after a toolbar click.
      onMouseDown={(e) => editor.state.editingText && (e.target as HTMLElement).closest('button') && e.preventDefault()}
    >
      {hasText && <TextControls />}
      {single?.type === 'shape' && <ShapeControls el={single} />}
      {single?.type === 'line' && <LineControls el={single} />}
      {single && (single.type === 'image' || single.type === 'frame' || single.type === 'video') && <ImageControls el={single} />}
      {single?.type === 'image' && <SvgColorControls el={single} />}
      {single?.type === 'frame' && single.content?.video && <FrameVideoControls el={single} />}
      {single?.type === 'qr' && <QrControls el={single} />}
      {single?.type === 'table' && <TableControls el={single} />}
      {single?.type === 'chart' && <ChartControls el={single} />}
      {single?.type === 'draw' && <DrawControls el={single} />}
      {single?.type === 'video' && <VideoControls el={single} />}
      {single?.type === 'grid' && <GridControls el={single} />}
      {single?.type === 'form' && <FormControls el={single} />}
      <VectorControls sel={sel} />
      {sel.length > 1 && (
        <button className="btn ghost sm" onClick={() => editor.group()} title={t('Group (⌘G)')}><Group size={16} /> {t('Group')}</button>
      )}
      {single?.type === 'group' && (
        <button className="btn ghost sm" onClick={() => editor.ungroup()} title={t('Ungroup (⇧⌘G)')}><Ungroup size={16} /> {t('Ungroup')}</button>
      )}
      <AnimateButton sel={sel} page={doc.pages[state.activePageId]} />
      <div className="grow" />
      <CommonControls sel={sel} />
    </div>
  );
}

// ------------------------------------------------------------------ text

const EFFECTS: Array<{ id: string; label: string; make: (c: string) => TextEffect | undefined }> = [
  { id: 'none', label: tk('None'), make: () => undefined },
  { id: 'shadow', label: tk('Shadow'), make: () => ({ k: 'shadow', dx: 3, dy: 4, blur: 6, color: hex('#00000066') }) },
  { id: 'lift', label: tk('Lift'), make: () => ({ k: 'shadow', dx: 0, dy: 6, blur: 18, color: hex('#00000040') }) },
  { id: 'hollow', label: tk('Hollow'), make: () => ({ k: 'hollow', width: 4 }) },
  { id: 'outline', label: tk('Outline'), make: () => ({ k: 'outline', width: 8, color: hex('#000000') }) },
  { id: 'neon', label: tk('Neon'), make: () => ({ k: 'neon', intensity: 60 }) },
  { id: 'background', label: tk('Background'), make: () => ({ k: 'background', color: hex('#fde047'), padding: 12, radius: 8 }) },
  { id: 'echo', label: tk('Echo'), make: () => ({ k: 'echo', dx: 4, dy: 4 }) },
  { id: 'glitch', label: tk('Glitch'), make: () => ({ k: 'glitch', offset: 4 }) },
];

function TextControls() {
  const { editor } = useEd();
  const doc = useDoc();
  useEditorState();
  const st = editor.currentTextStyle();
  const para = editor.currentParagraph();
  if (!st) return null;
  const set = (m: Partial<RunStyle> & Record<string, unknown>) => editor.setTextStyle(m);
  const textEls = editor.selectedDeep().filter((e) => e.type === 'text');
  const effect = textEls[0]?.type === 'text' ? textEls[0].effect : undefined;
  const nextAlign = { start: 'center', center: 'end', end: 'justify', justify: 'start' } as const;
  const AlignIcon = { start: AlignLeft, center: AlignCenter, end: AlignRight, justify: AlignJustify }[para?.align ?? 'start'];
  const sizeStep = (d: number) => set({ size: Math.max(1, Math.round(st.size + d)) });
  return (
    <div className="group">
      <FontPicker value={st.family} onPick={(family) => set({ family })} />
      <div className="size-box" title={t('Font size')}>
        <button onClick={() => sizeStep(st.size > 30 ? -4 : -1)} aria-label={t('Decrease font size')}><Minus size={14} /></button>
        <input value={Math.round(st.size * 10) / 10} onChange={(e) => +e.target.value > 0 && set({ size: +e.target.value })} aria-label={t('Font size')} />
        <button onClick={() => sizeStep(st.size >= 30 ? 4 : 1)} aria-label={t('Increase font size')}><Plus size={14} /></button>
      </div>
      <ColorButton variant="text" title={t('Text colour')} doc={doc} value={st.fill} onChange={(fill) => set({ fill })} />
      <button className={`btn ghost sm icon ${st.weight >= 600 ? 'active' : ''}`} title={t('Bold (⌘B)')} onClick={() => set({ weight: st.weight >= 600 ? 400 : 700 })}><Bold size={16} /></button>
      <button className={`btn ghost sm icon ${st.italic ? 'active' : ''}`} title={t('Italic (⌘I)')} onClick={() => set({ italic: !st.italic })}><Italic size={16} /></button>
      <button className={`btn ghost sm icon ${st.underline ? 'active' : ''}`} title={t('Underline (⌘U)')} onClick={() => set({ underline: !st.underline })}><Underline size={16} /></button>
      <button className={`btn ghost sm icon ${st.strike ? 'active' : ''}`} title={t('Strikethrough')} onClick={() => set({ strike: !st.strike })}><Strikethrough size={16} /></button>
      <button className={`btn ghost sm icon ${st.caseTransform === 'upper' ? 'active' : ''}`} title={t('Uppercase')} onClick={() => set({ caseTransform: st.caseTransform === 'upper' ? 'none' : 'upper' })}><CaseSensitive size={16} /></button>
      <ScriptButtons st={st} set={set} />
      <button className="btn ghost sm icon" title={t('Alignment')} onClick={() => editor.setParagraphStyle({ align: nextAlign[para?.align ?? 'start'] })}><AlignIcon size={16} /></button>
      <button className={`btn ghost sm icon ${para?.list?.kind === 'bullet' ? 'active' : ''}`} title={t('Bulleted list')} onClick={() => editor.setParagraphStyle({ list: para?.list?.kind === 'bullet' ? undefined : { kind: 'bullet', level: 0 } })}><List size={16} /></button>
      <button className={`btn ghost sm icon ${para?.list?.kind === 'number' ? 'active' : ''}`} title={t('Numbered list')} onClick={() => editor.setParagraphStyle({ list: para?.list?.kind === 'number' ? undefined : { kind: 'number', level: 0 } })}><ListOrdered size={16} /></button>
      {textEls.length > 0 && <TextAnchor els={textEls as TextEl[]} />}
      <PopoverButton title={t('Spacing')} label={<><SlidersHorizontal size={16} /> {t('Spacing')}</>} content={() => (
        <div className="col pop-sm">
          <label className="field">{t('Letter spacing ({value})', { value: st.tracking })}
            <input type="range" min={-200} max={800} step={10} value={st.tracking} onChange={(e) => set({ tracking: +e.target.value })} />
          </label>
          <label className="field">{t('Line spacing ({value})', { value: para?.lineHeight.toFixed(2) ?? '' })}
            <input type="range" min={0.5} max={2.5} step={0.05} value={para?.lineHeight ?? 1.3} onChange={(e) => editor.setParagraphStyle({ lineHeight: +e.target.value })} />
          </label>
          <label className="field">{t('Paragraph spacing')}
            <input type="range" min={0} max={st.size * 2} step={1} value={para?.spaceAfter ?? 0} onChange={(e) => editor.setParagraphStyle({ spaceAfter: +e.target.value })} />
          </label>
          {textEls[0]?.type === 'text' && !textEls[0].curve && (
            <label className="check" title={t('Keep the box size and scale the text down when it overflows')}>
              <input type="checkbox" checked={textEls[0].autoFit === 'shrink'} onChange={(e) => editor.updateSelected((x) => (x.type === 'text'
                ? (e.target.checked ? { autoFit: 'shrink', frame: { mode: 'fixed' } } : { autoFit: 'none', frame: { mode: 'autoHeight' } })
                : null), true, false)} />
              {t('Shrink text to fit the box')}
            </label>
          )}
        </div>
      )} />
      {textEls.length > 0 && <PopoverButton title={t('Effects')} label={<><Sparkles size={16} /> {t('Effects')}</>} content={(close) => (
        <div className="pop-sm" style={{ display: 'grid', gridTemplateColumns: 'repeat(3, 1fr)', gap: 8 }}>
          {EFFECTS.map((fx) => (
            <button key={fx.id} className={`btn sm ${(effect?.k ?? 'none') === (fx.make('#000')?.k ?? 'none') && (fx.id !== 'lift' || (effect?.k === 'shadow' && effect.dx === 0)) ? 'active' : ''}`}
              onClick={() => { editor.updateSelected((e) => (e.type === 'text' ? { effect: fx.make('#000') ?? null } : null), true, false); close(); }}>
              {t(fx.label)}
            </button>
          ))}
          <div style={{ gridColumn: '1 / -1' }}>
            <CurveControl el={textEls[0] as TextEl | undefined} />
          </div>
          {effect?.k === 'background' && (
            <div style={{ gridColumn: '1 / -1' }}>
              <ColorButton title={t('Background colour')} doc={doc} value={solid(effect.color)} onChange={(p) => p.k === 'solid' && editor.updateSelected((e) => (e.type === 'text' ? { effect: { ...effect, color: p.color } } : null), true)} />
            </div>
          )}
        </div>
      )} />}
    </div>
  );
}

function CurveControl({ el }: { el: TextEl | undefined }) {
  const { editor } = useEd();
  if (!el) return null;
  const curve = el.curve ?? 0;
  return (
    <label className="field">{t('Curve ({value})', { value: curve })}
      <input type="range" min={-100} max={100} value={curve} aria-label={t('Curve text')}
        onChange={(e) => {
          const v = +e.target.value;
          // Curved text is one line on an arc; straightening restores an auto-height box.
          editor.updateSelected((x) => (x.type === 'text' ? (v ? { curve: v, autoFit: 'none' } : { curve: null, frame: { mode: 'autoHeight' } }) : null), true);
        }} />
    </label>
  );
}

// ------------------------------------------------------------------ shapes & lines

function StrokeControls({ stroke, onChange, allowNone = true }: { stroke: Stroke | undefined; onChange: (s: Stroke | null) => void; allowNone?: boolean }) {
  const doc = useDoc();
  const s: Stroke = stroke ?? { paint: solid('#1f2328'), width: 4, align: 'center', cap: 'round', join: 'round' };
  const styles: Array<[string, number[] | undefined]> = [[t('Solid'), undefined], [t('Dashed'), [3, 2]], [t('Dotted'), [0.01, 2]]];
  return (
    <div className="col pop-sm">
      <div className="row">
        {allowNone && <button className={`btn sm ${!stroke ? 'active' : ''}`} onClick={() => onChange(null)}>{t('None')}</button>}
        {styles.map(([name, dash]) => (
          <button key={name} className={`btn sm ${stroke && JSON.stringify(stroke.dash) === JSON.stringify(dash) ? 'active' : ''}`} onClick={() => onChange({ ...s, dash, cap: dash?.[0] === 0.01 ? 'round' : s.cap })}>{name}</button>
        ))}
      </div>
      <label className="field">{t('Weight ({value})', { value: s.width })}
        <input type="range" min={0} max={60} value={s.width} onChange={(e) => onChange({ ...s, width: +e.target.value })} />
      </label>
      <div className="row">
        <span className="muted" style={{ fontSize: 12 }}>{t('Colour')}</span>
        <ColorButton title={t('Border colour')} doc={doc} value={s.paint} onChange={(paint) => onChange({ ...s, paint })} />
      </div>
    </div>
  );
}

function ShapeControls({ el }: { el: ShapeEl }) {
  const { editor } = useEd();
  const doc = useDoc();
  // A shape filled with a photo (dropped onto it): the colour sits under the photo and stays when it changes.
  const photo = Boolean(shapePhotoFill(el));
  const locked = photo && userLocked(el);
  return (
    <div className="group">
      <ColorButton title={t('Fill colour')} doc={doc} value={fillColorOf(el)} allowGradient allowTransparent
        onChange={(p) => editor.updateSelected((x) => (x.type === 'shape' ? { fills: withFillColor(x, p) } : null))} />
      {photo && !locked && editor.mediaCrop(el) && (
        <button className="btn ghost sm" onClick={() => editor.startCrop(el.id)} title={t('Crop (double-click)')}>
          <Crop size={16} /> {t('Crop')}
        </button>
      )}
      {photo && <PhotoActions el={el} />}
      <PopoverButton title={t('Border')} label={<><Blend size={16} /> {t('Border')}</>} content={() => (
        <StrokeControls stroke={el.strokes[0]} onChange={(s) => editor.updateSelected({ strokes: s ? [s] : [] })} />
      )} />
      {el.kind === 'rect' && (
        <PopoverButton title={t('Corner rounding')} label={<><SquareRoundCorner size={16} /> {t('Corners')}</>} content={() => (
          <label className="field" style={{ width: 220 }}>{t('Corner rounding ({value})', { value: el.params.cornerRadius ?? 0 })}
            <input type="range" min={0} max={Math.min(el.xf.w, el.xf.h) / 2} value={el.params.cornerRadius ?? 0} onChange={(e) => editor.updateSelected({ params: { ...el.params, cornerRadius: +e.target.value } })} />
          </label>
        )} />
      )}
      {(el.kind === 'star' || el.kind === 'polygon') && (
        <PopoverButton title={t('Points')} label={el.kind === 'star' ? `★ ${t('Points')}` : `⬠ ${t('Sides')}`} content={() => (
          <div className="col pop-sm">
            <label className="field">{el.kind === 'star' ? t('Points') : t('Sides')}
              <input type="range" min={3} max={16} value={el.params[el.kind === 'star' ? 'points' : 'sides'] ?? 5} onChange={(e) => editor.updateSelected({ params: { ...el.params, [el.kind === 'star' ? 'points' : 'sides']: +e.target.value } })} />
            </label>
            {el.kind === 'star' && (
              <label className="field">{t('Inner radius')}
                <input type="range" min={0.1} max={0.95} step={0.01} value={el.params.innerRatio ?? 0.45} onChange={(e) => editor.updateSelected({ params: { ...el.params, innerRatio: +e.target.value } })} />
              </label>
            )}
          </div>
        )} />
      )}
    </div>
  );
}

const MARKERS: readonly LineMarker[] = LINE_MARKERS;
const MARKER_LABELS: Record<LineMarker, string> = {
  none: tk('none'), arrow: tk('arrow'), triangle: tk('triangle'), circle: tk('circle'), square: tk('square'), bar: tk('bar'),
  stealth: tk('stealth arrow'), diamond: tk('diamond'), 'triangle-outline': tk('triangle outline'), 'circle-outline': tk('circle outline'),
  'square-outline': tk('square outline'), 'diamond-outline': tk('diamond outline'),
};
function LineControls({ el }: { el: Extract<Element, { type: 'line' }> }) {
  const { editor } = useEd();
  const doc = useDoc();
  return (
    <div className="group">
      <ColorButton title={t('Line colour')} doc={doc} value={el.stroke.paint} onChange={(paint) => editor.updateSelected({ stroke: { ...el.stroke, paint } })} />
      <PopoverButton title={t('Line style')} label={`━ ${t('Style')}`} content={() => <StrokeControls stroke={el.stroke} allowNone={false} onChange={(s) => s && editor.updateSelected({ stroke: s })} />} />
      <select className="input" style={{ width: 110 }} value={el.startMarker} onChange={(e) => editor.updateSelected({ startMarker: e.target.value }, false, false)} aria-label={t('Line start')}>
        {MARKERS.map((m) => <option key={m} value={m}>{t('Start: {marker}', { marker: t(MARKER_LABELS[m] ?? m) })}</option>)}
      </select>
      <select className="input" style={{ width: 110 }} value={el.endMarker} onChange={(e) => editor.updateSelected({ endMarker: e.target.value }, false, false)} aria-label={t('Line end')}>
        {MARKERS.map((m) => <option key={m} value={m}>{t('End: {marker}', { marker: t(MARKER_LABELS[m] ?? m) })}</option>)}
      </select>
      <select className="input" style={{ width: 118 }} value={el.route ?? 'straight'} onChange={(e) => editor.updateSelected({ route: e.target.value === 'straight' ? null : e.target.value }, false, false)} aria-label={t('Line type')}>
        <option value="straight">{t('Straight')}</option>
        <option value="elbow">{t('Elbowed')}</option>
        <option value="curved">{t('Curved')}</option>
      </select>
      {(el.startAttach || el.endAttach) && <span className="faint" title={t('Drag an end onto a shape to connect it')} style={{ fontSize: 12 }}><Spline size={14} /> {t('Connected')}</span>}
    </div>
  );
}

// ------------------------------------------------------------------ images

const GRID_LAYOUT_LABELS: Record<string, string> = {
  'cols-2': tk('2 columns'), 'rows-2': tk('2 rows'), 'cols-3': tk('3 columns'), 'grid-2x2': tk('2 × 2'), 'grid-3x3': tk('3 × 3'), 'grid-2x3': tk('2 × 3'),
  'big-left': tk('Feature left'), 'big-top': tk('Feature top'), mosaic: tk('Mosaic'), 'strip-4': tk('Film strip'),
};
const DUOTONES: Array<[string, string, string]> = [
  [tk('Midnight'), '#1e1b4b', '#f472b6'], [tk('Ocean'), '#0c4a6e', '#7dd3fc'], [tk('Sunset'), '#7c2d12', '#fde68a'], [tk('Forest'), '#14532d', '#bef264'],
  [tk('Grape'), '#3b0764', '#fbcfe8'], [tk('Steel'), '#111827', '#e5e7eb'], [tk('Candy'), '#be185d', '#fef08a'], [tk('Mint'), '#134e4a', '#ccfbf1'],
];
const titleCase = (s: string) => s.split('-').map((w) => w[0]!.toUpperCase() + w.slice(1)).join(' ');
const ADJUST: Array<[keyof NonNullable<ImageEditStack['adjust']>, string, number, number]> = [
  ['brightness', tk('Brightness'), -100, 100], ['contrast', tk('Contrast'), -100, 100], ['saturation', tk('Saturation'), -100, 100],
  ['highlights', tk('Highlights'), -100, 100], ['shadows', tk('Shadows'), -100, 100], ['whites', tk('Whites'), -100, 100], ['blacks', tk('Blacks'), -100, 100],
  ['temperature', tk('Temperature'), -100, 100], ['tint', tk('Tint'), -100, 100],
  ['sharpen', tk('Sharpen'), 0, 100], ['clarity', tk('Clarity'), -100, 100], ['fade', tk('Fade'), 0, 100],
  ['blur', tk('Blur'), 0, 100], ['vignette', tk('Vignette'), 0, 100],
];
/** One-click enhance: gentle, photo-safe defaults (lift shadows, recover highlights, a touch of pop). */
const AUTO_ENHANCE = { contrast: 8, saturation: 10, shadows: 25, highlights: -15, whites: 8, clarity: 15, sharpen: 15 };

function ImageControls({ el }: { el: Extract<Element, { type: 'image' | 'frame' | 'video' }> }) {
  const { editor } = useEd();
  const edits = el.type === 'frame' ? el.content?.edits : el.edits;
  const setEdits = (e: ImageEditStack) =>
    editor.updateSelected((x) => (x.type === 'image' || x.type === 'video' ? { edits: e } : x.type === 'frame' && x.content ? { content: { ...x.content, edits: e } } : null));
  const cur: ImageEditStack = edits ?? { version: 1 };
  const hasImage = el.type === 'image' || el.type === 'video' || Boolean(el.content);
  const video = el.type === 'video';
  // A frame's photo follows the frame's locks (content); images and videos keep their own controls.
  const locked = el.type === 'frame' && Boolean(el.content) && userLocked(el);
  return (
    <div className="group">
      {hasImage && (
        <PopoverButton title={t('Edit image')} label={<><SlidersHorizontal size={16} /> {t('Edit')}</>} content={() => (
          <div className="col pop-md">
            <h4 style={{ margin: 0 }} className="muted">{t('Filters')}</h4>
            <FilterGroups cur={cur} setEdits={setEdits} />
            {cur.filter && cur.filter.preset !== 'none' && (
              <label className="field">{t('Intensity')}
                <input type="range" min={0} max={1} step={0.01} value={cur.filter.amount} onChange={(e) => setEdits({ ...cur, filter: { ...cur.filter!, amount: +e.target.value } })} />
              </label>
            )}
            <h4 style={{ margin: '6px 0 0' }} className="muted">{t('Duotone')}</h4>
            <div style={{ display: 'grid', gridTemplateColumns: 'repeat(4, 1fr)', gap: 6 }}>
              <button className={`btn sm ${!cur.duotone ? 'active' : ''}`} onClick={() => setEdits({ ...cur, duotone: undefined })}>{t('None')}</button>
              {DUOTONES.map(([name, a, b]) => (
                <button key={name} title={t(name)} className={`btn sm ${cur.duotone && toHex(cur.duotone.shadow) === a && toHex(cur.duotone.highlight) === b ? 'active' : ''}`}
                  style={{ background: `linear-gradient(90deg, ${a}, ${b})`, color: '#fff' }}
                  onClick={() => setEdits({ ...cur, duotone: { shadow: hex(a), highlight: hex(b), amount: cur.duotone?.amount ?? 1 } })}>{t(name)}</button>
              ))}
            </div>
            {cur.duotone && (
              <div className="row">
                <label className="row" style={{ fontSize: 12 }}>{t('Shadows')} <input type="color" value={toHex(cur.duotone.shadow)} onChange={(e) => setEdits({ ...cur, duotone: { ...cur.duotone!, shadow: hex(e.target.value) } })} /></label>
                <label className="row" style={{ fontSize: 12 }}>{t('Highlights')} <input type="color" value={toHex(cur.duotone.highlight)} onChange={(e) => setEdits({ ...cur, duotone: { ...cur.duotone!, highlight: hex(e.target.value) } })} /></label>
                <input type="range" min={0} max={1} step={0.01} value={cur.duotone.amount} aria-label={t('Duotone intensity')} onChange={(e) => setEdits({ ...cur, duotone: { ...cur.duotone!, amount: +e.target.value } })} />
              </div>
            )}
            <div className="row" style={{ margin: '6px 0 0' }}>
              <h4 style={{ margin: 0 }} className="muted grow">{t('Adjust')}</h4>
              <button className="btn sm" onClick={() => setEdits({ ...cur, adjust: { ...cur.adjust, ...AUTO_ENHANCE } })}><Sparkles size={14} /> {t('Auto-enhance')}</button>
            </div>
            {ADJUST.map(([k, label, min, max]) => (
              <label key={k} className="field">{t('{label} ({value})', { label: t(label), value: cur.adjust?.[k] ?? 0 })}
                <input type="range" min={min} max={max} value={cur.adjust?.[k] ?? 0} onChange={(e) => setEdits({ ...cur, adjust: { ...cur.adjust, [k]: +e.target.value } })} />
              </label>
            ))}
            {!video && <PixelateControl cur={cur} setEdits={setEdits} />}
            <button className="btn sm" onClick={() => setEdits({ version: 1 })}>{t('Reset')}</button>
          </div>
        )} />
      )}
      {hasImage && !video && !locked && (
        <button className="btn ghost sm" onClick={() => editor.startCrop(el.id)} title={t('Crop (double-click)')}>
          <Crop size={16} /> {t('Crop')}
        </button>
      )}
      {!video && <button className="btn ghost sm icon" onClick={() => editor.flip('x')} title={t('Flip horizontal')} aria-label={t('Flip horizontal')}><TrianglesCenterlineDashedVertical size={16} /></button>}
      {!video && <button className="btn ghost sm icon" onClick={() => editor.flip('y')} title={t('Flip vertical')} aria-label={t('Flip vertical')}><TrianglesCenterlineDashedHorizontal size={16} /></button>}
      {el.type === 'image' && (
        <PopoverButton title={t('Rounded corners')} className="btn ghost sm icon" label={<SquareRoundCorner size={16} />} content={() => (
          <label className="field" style={{ width: 220 }}>{t('Corner radius')}
            <input type="range" min={0} max={Math.min(el.xf.w, el.xf.h) / 2} value={el.cornerRadius ?? 0} onChange={(e) => editor.updateSelected({ cornerRadius: +e.target.value })} />
          </label>
        )} />
      )}
      {el.type === 'image' && <ImageBorder el={el} />}
      {el.type === 'image' && <ReplaceImageButton el={el} />}
      {el.type === 'image' && <SetAsBackgroundButton el={el} />}
      {el.type === 'frame' && el.content && <PhotoActions el={el} />}
      {el.type === 'frame' && !el.content && <span className="faint" style={{ fontSize: 12 }}>{t('Drag an image into the frame')}</span>}
    </div>
  );
}

/**
 * The photo in a shape or frame: replace it from a file, pull it out as a separate image, or remove it. Shown but
 * hidden under the user's own lock (unlock it first).
 */
function PhotoActions({ el }: { el: ShapeEl | FrameEl }) {
  const { editor } = useEd();
  const toast = useToast();
  const input = useRef<HTMLInputElement>(null);
  if (userLocked(el)) return null;
  const video = el.type === 'frame' && Boolean(el.content?.video);
  // Acts on the element as it is now (a collaborator may have changed or locked it since this toolbar rendered).
  const cur = () => editor.doc.elements[el.id] as ShapeEl | FrameEl | undefined;
  const failed = () => toast(t('Couldn’t change the image.'));
  return (
    <>
      <button className="btn ghost sm" onClick={() => input.current?.click()} title={t('Replace with a file from your device (or drag another photo onto it)')}>
        <Replace size={16} /> {t('Replace')}
      </button>
      <input ref={input} type="file" hidden accept={el.type === 'shape' ? 'image/*' : 'image/*,video/*'} onChange={(e) => {
        const f = e.target.files?.[0];
        const target = cur();
        e.target.value = '';
        if (f && target) replacePhoto(editor, target, f).then((ok) => ok || failed()).catch((x) => toast(t('Upload failed: {message}', { message: (x as Error).message })));
      }} />
      {!video && (
        <button className="btn ghost sm" onClick={() => { const c = cur(); if (!c || !detachPhoto(editor, c)) failed(); }} title={t('Pull the image out as a separate element')}>
          <ImageOff size={16} /> {t('Detach image')}
        </button>
      )}
      <button className="btn ghost sm" onClick={() => { const c = cur(); if (!c || !removePhoto(editor, c)) failed(); }}>{t('Remove image')}</button>
    </>
  );
}

function GridControls({ el }: { el: GridEl }) {
  const { editor } = useEd();
  const doc = useDoc();
  const filled = el.cells.filter((c) => c.asset).length;
  return (
    <div className="group">
      <PopoverButton title={t('Grid layout')} label={<><LayoutGrid size={16} /> {t('Layout')}</>} content={(close) => (
        <div className="pop-sm" style={{ display: 'grid', gridTemplateColumns: 'repeat(3, 1fr)', gap: 6 }}>
          {Object.entries(GRID_LAYOUTS).map(([id, l]) => (
            <button key={id} className="btn sm" title={t(GRID_LAYOUT_LABELS[id] ?? l.label)} style={{ height: 52, padding: 4 }} onClick={() => {
              // Keep photos in order when switching layouts.
              const kept = el.cells.filter((c) => c.asset);
              editor.updateSelected({ cells: l.cells.map((rect, i) => (kept[i] ? { rect, asset: kept[i]!.asset, ...(kept[i]!.video ? { video: kept[i]!.video } : {}) } : { rect })) }, false, false);
              close();
            }}>
              <svg viewBox="0 0 30 20" width="42" height="28">{l.cells.map((c, i) => <rect key={i} x={c.x * 30 + 0.8} y={c.y * 20 + 0.8} width={c.w * 30 - 1.6} height={c.h * 20 - 1.6} rx={1} fill="currentColor" opacity={0.55} />)}</svg>
            </button>
          ))}
        </div>
      )} />
      <PopoverButton title={t('Spacing')} label={<><SlidersHorizontal size={16} /> {t('Spacing')}</>} content={() => (
        <div className="col pop-sm">
          <label className="field">{t('Gap ({value}px)', { value: el.gap })}
            <input type="range" min={0} max={60} value={el.gap} onChange={(e) => editor.updateSelected({ gap: +e.target.value })} />
          </label>
          <label className="field">{t('Corner rounding ({value})', { value: el.radius })}
            <input type="range" min={0} max={80} value={el.radius} onChange={(e) => editor.updateSelected({ radius: +e.target.value })} />
          </label>
        </div>
      )} />
      <ColorButton title={t('Empty cell colour')} doc={doc} value={el.placeholderFill} onChange={(p) => editor.updateSelected({ placeholderFill: p })} />
      <span className="faint" style={{ fontSize: 12 }}>{t('{filled}/{total} photos · drag images into cells', { filled, total: el.cells.length })}</span>
      {filled > 0 && <button className="btn ghost sm" onClick={() => editor.updateSelected({ cells: el.cells.map((c) => ({ rect: c.rect })) }, false, false)}>{t('Clear photos')}</button>}
    </div>
  );
}

function CropBar() {
  const { editor } = useEd();
  const state = useEditorState();
  useDoc();
  const id = state.croppingId!;
  const zoom = editor.cropZoom(id);
  return (
    <div className="ctx-toolbar">
      <Crop size={16} />
      <span style={{ fontWeight: 600 }}>{t('Crop')}</span>
      <span className="muted" style={{ fontSize: 13 }}>{t('Drag the image to reposition it inside the frame.')}</span>
      <label className="row" style={{ fontSize: 13 }}>
        {t('Zoom')}
        <input type="range" min={1} max={4} step={0.01} value={zoom} onChange={(e) => editor.setCropZoom(id, +e.target.value)} />
      </label>
      <div className="grow" />
      <button className="btn primary sm" onClick={() => editor.stopCrop()}>{t('Done')}</button>
    </div>
  );
}

function QrControls({ el }: { el: Extract<Element, { type: 'qr' }> }) {
  const { editor } = useEd();
  const doc = useDoc();
  const [v, setV] = useState(el.data);
  return (
    <div className="group">
      <input className="input" style={{ width: 260 }} value={v} onChange={(e) => setV(e.target.value)} onBlur={() => v !== el.data && editor.updateSelected({ data: v }, false, false)} onKeyDown={(e) => e.key === 'Enter' && (e.target as HTMLInputElement).blur()} aria-label={t('QR code URL')} />
      <ColorButton title={t('Foreground')} doc={doc} value={solid(el.fg)} onChange={(p) => p.k === 'solid' && editor.updateSelected({ fg: p.color })} />
      <ColorButton title={t('Background')} doc={doc} value={solid(el.bg)} onChange={(p) => p.k === 'solid' && editor.updateSelected({ bg: p.color })} />
    </div>
  );
}

// ------------------------------------------------------------------ common

const ALIGN: Array<[AlignEdge, typeof AlignHorizontalJustifyStart, string]> = [
  ['left', AlignHorizontalJustifyStart, tk('Left')], ['hcenter', AlignHorizontalJustifyCenter, tk('Centre')], ['right', AlignHorizontalJustifyEnd, tk('Right')],
  ['top', AlignVerticalJustifyStart, tk('Top')], ['vcenter', AlignVerticalJustifyCenter, tk('Middle')], ['bottom', AlignVerticalJustifyEnd, tk('Bottom')],
];

function CommonControls({ sel }: { sel: Element[] }) {
  const { editor, setDrawer } = useEd();
  const locked = sel.some((e) => userLock(e)?.move);
  const opacity = sel[0]?.opacity ?? 1;
  const link = sel.length === 1 ? sel[0]!.link : undefined;
  const shadow = sel[0]?.effects?.find((e): e is Extract<Effect, { k: 'dropShadow' }> => e.k === 'dropShadow');
  const nonText = sel.length === 1 && sel[0]!.type !== 'text';
  const [url, setUrl] = useState(link?.kind === 'url' ? link.url : '');
  const multi = sel.length > 1;
  const pct = useMemo(() => Math.round(opacity * 100), [opacity]);
  return (
    <div className="group ctx-end">
      <button className={`btn ghost sm icon ${editor.state.styleBrush ? 'active' : ''}`} title={t('Copy style (⌥⌘C) — then click an element')} onClick={() => (editor.state.styleBrush ? editor.cancelStyleBrush() : editor.copyStyle())}><Paintbrush size={16} /></button>
      <PopoverButton title={t('Position')} label={<><Layers size={16} /> {t('Position')}</>} content={(close) => (
        <div className="col pop-md">
          <GeometryInputs sel={sel} />
          <h4 style={{ margin: 0 }} className="muted">{t('Arrange')}</h4>
          <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 6 }}>
            <button className="btn sm" onClick={() => editor.arrange('forward')}><ArrowUp size={14} /> {t('Forward')}</button>
            <button className="btn sm" onClick={() => editor.arrange('backward')}><ArrowDown size={14} /> {t('Backward')}</button>
            <button className="btn sm" onClick={() => editor.arrange('front')}><ArrowUpToLine size={14} /> {t('To front')}</button>
            <button className="btn sm" onClick={() => editor.arrange('back')}><ArrowDownToLine size={14} /> {t('To back')}</button>
          </div>
          <h4 style={{ margin: '6px 0 0' }} className="muted">{multi ? t('Align elements') : t('Align to page')}</h4>
          <div style={{ display: 'grid', gridTemplateColumns: 'repeat(3, 1fr)', gap: 6 }}>
            {ALIGN.map(([edge, Icon, label]) => (
              <button key={edge} className="btn sm" onClick={() => editor.align(edge)}><Icon size={14} /> {t(label)}</button>
            ))}
          </div>
          {sel.length >= 3 && (
            <>
              <h4 style={{ margin: '6px 0 0' }} className="muted">{t('Spacing')}</h4>
              <div className="row">
                <button className="btn sm grow" onClick={() => editor.distribute('x')}>{t('Distribute')} ↔</button>
                <button className="btn sm grow" onClick={() => editor.distribute('y')}>{t('Distribute')} ↕</button>
              </div>
            </>
          )}
          {multi && <button className="btn sm" onClick={() => editor.tidy()}><LayoutGrid size={14} /> {t('Tidy up')}</button>}
          <button className="btn sm" onClick={() => { setDrawer('layers'); close(); }}><Layers size={14} /> {t('Open layers')}</button>
        </div>
      )} />
      <PopoverButton title={t('Transparency')} label={<><Maximize2 size={14} style={{ opacity: 0.6 }} /> {pct}%</>} content={() => (
        <div className="col pop-sm">
          <label className="field">{t('Transparency ({value}%)', { value: pct })}
            <input type="range" min={0} max={100} value={pct} onChange={(e) => editor.updateSelected({ opacity: +e.target.value / 100 })} />
          </label>
          <label className="field">{t('Blend mode')}
            <select className="input" value={sel[0]?.blend ?? 'normal'} onChange={(e) => editor.updateSelected({ blend: e.target.value === 'normal' ? null : (e.target.value as BlendMode) }, false, false)}>
              {BLENDS.map((b) => <option key={b} value={b}>{BLEND_LABELS[b] ? t(BLEND_LABELS[b]) : titleCase(b)}</option>)}
            </select>
          </label>
          {nonText && (
            <label className="row" style={{ fontSize: 13 }}>
              <input type="checkbox" checked={Boolean(shadow)} onChange={(e) => editor.updateSelected((x) => ({
                effects: e.target.checked ? [...(x.effects ?? []).filter((f) => f.k !== 'dropShadow'), { k: 'dropShadow', dx: 6, dy: 10, blur: 24, color: hex('#0000004d') }] : (x.effects ?? []).filter((f) => f.k !== 'dropShadow'),
              }), false, false)} />
              {t('Drop shadow')}
            </label>
          )}
          {shadow && (
            <label className="field">{t('Shadow blur ({value})', { value: shadow.blur })}
              <input type="range" min={0} max={80} value={shadow.blur} onChange={(e) => editor.updateSelected((x) => ({ effects: (x.effects ?? []).map((f) => (f.k === 'dropShadow' ? { ...f, blur: +e.target.value } : f)) }))} />
            </label>
          )}
          {shadow && (
            <label className="row" style={{ fontSize: 13 }}>{t('Shadow colour')}
              <input type="color" value={toHex(shadow.color)} onChange={(e) => editor.updateSelected((x) => ({ effects: (x.effects ?? []).map((f) => (f.k === 'dropShadow' ? { ...f, color: { ...hex(e.target.value), a: 0.35 } } : f)) }))} />
            </label>
          )}
        </div>
      )} />
      {sel.length === 1 && (
        <PopoverButton title={t('Link')} label={<Link2 size={16} />} className={`btn ghost sm icon ${link ? 'active' : ''}`} content={(close) => (
          <form className="row pop-md" onSubmit={(e) => { e.preventDefault(); editor.updateSelected({ link: url ? { kind: 'url', url: /^https?:\/\//.test(url) ? url : `https://${url}` } : null }, false, false); close(); }}>
            <input className="input" placeholder="https://" value={url} onChange={(e) => setUrl(e.target.value)} autoFocus />
            <button className="btn primary sm">{t('Apply')}</button>
          </form>
        )} />
      )}
      {sel.length === 1 && ['image', 'video', 'frame', 'shape', 'grid', 'chart', 'qr', 'draw', 'group'].includes(sel[0]!.type) && <AltText el={sel[0]!} />}
      <button className="btn ghost sm icon" title={locked ? t('Unlock') : t('Lock')} onClick={() => editor.toggleLock()}>{locked ? <Lock size={16} /> : <Unlock size={16} />}</button>
      <button className="btn ghost sm icon" title={t('Duplicate (⌘D)')} onClick={() => editor.duplicate()}><Copy size={16} /></button>
      <button className="btn ghost sm icon" title={t('Delete')} onClick={() => editor.deleteSelection()} disabled={locked}><Trash2 size={16} /></button>
    </div>
  );
}

const BLENDS: BlendMode[] = ['normal', 'multiply', 'screen', 'overlay', 'darken', 'lighten', 'color-dodge', 'color-burn', 'hard-light', 'soft-light', 'difference', 'exclusion', 'hue', 'saturation', 'color', 'luminosity'] as BlendMode[];
const BLEND_LABELS: Record<string, string> = {
  normal: tk('Normal'), multiply: tk('Multiply'), screen: tk('Screen'), overlay: tk('Overlay'), darken: tk('Darken'), lighten: tk('Lighten'),
  'color-dodge': tk('Color Dodge'), 'color-burn': tk('Color Burn'), 'hard-light': tk('Hard Light'), 'soft-light': tk('Soft Light'),
  difference: tk('Difference'), exclusion: tk('Exclusion'), hue: tk('Hue'), saturation: tk('Saturation'), color: tk('Color'), luminosity: tk('Luminosity'),
};

/** Exact X / Y / W / H / rotation (page px). */
function GeometryInputs({ sel }: { sel: Element[] }) {
  const { editor } = useEd();
  const b = editor.selectionBox();
  if (!b) return null;
  const single = sel.length === 1 ? sel[0]! : null;
  const field = (label: string, value: number, key: 'x' | 'y' | 'w' | 'h' | 'rot', disabled = false) => (
    <label className="field geo" key={key}>
      {label}
      <input className="input" type="number" disabled={disabled} step={key === 'rot' ? 1 : 0.5}
        defaultValue={Math.round(value * 10) / 10}
        key={`${key}:${Math.round(value * 10)}`}
        onKeyDown={(e) => e.key === 'Enter' && (e.target as HTMLInputElement).blur()}
        onBlur={(e) => {
          const v = Number(e.target.value);
          if (Number.isFinite(v) && Math.abs(v - value) > 0.04) editor.setGeometry({ [key]: v });
        }} />
    </label>
  );
  return (
    <>
      <h4 style={{ margin: 0 }} className="muted">{t('Size and position')}</h4>
      <div style={{ display: 'grid', gridTemplateColumns: 'repeat(3, 1fr)', gap: 6 }}>
        {field('X', b.x, 'x')}
        {field('Y', b.y, 'y')}
        {field(t('Rotate°'), single?.xf.rot ?? 0, 'rot', !single)}
        {field(t('Width'), single ? single.xf.w : b.w, 'w', !single)}
        {field(t('Height'), single ? single.xf.h : b.h, 'h', !single || (single.type === 'text' && single.frame.mode === 'autoHeight'))}
      </div>
    </>
  );
}

/** Alternative text for screen readers and accessible PDF exports (A11-010). */
function AltText({ el }: { el: Element }) {
  const { editor } = useEd();
  const [v, setV] = useState(el.altText ?? '');
  return (
    <PopoverButton title={t('Alternative text')} className={`btn ghost sm icon ${el.altText || el.decorative ? 'active' : ''}`} label={<Accessibility size={16} />} content={(close) => (
      <form className="col pop-md" onSubmit={(e) => { e.preventDefault(); editor.updateSelected({ altText: v.trim() || null }, false, false); close(); }}>
        <label className="field">{t('Alternative text')}
          <textarea className="input" rows={3} maxLength={500} value={v} onChange={(e) => setV(e.target.value)} placeholder={t('Describe what this shows for people using screen readers')} autoFocus />
        </label>
        <label className="check">
          <input type="checkbox" checked={Boolean(el.decorative)} onChange={(e) => editor.updateSelected({ decorative: e.target.checked || null }, false, false)} />
          {t('Decorative — screen readers skip it')}
        </label>
        <button className="btn primary sm">{t('Save')}</button>
      </form>
    )} />
  );
}

export type { Paint };
