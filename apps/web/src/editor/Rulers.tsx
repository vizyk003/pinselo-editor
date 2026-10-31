/**
 * Rulers and guides (ED-061): top/left rulers in page px of the active page. Drag from a ruler to add a
 * guide; drag a guide's marker to move it, off the page to remove it, or double-click it to remove it.
 * Guides are stored on the page and snapped to while moving elements.
 */
import type { pageSlots } from '@oc/engine';
import { useRef, useState, type PointerEvent as RPE } from 'react';
import { useEd, useEditorState } from './context';
import { t } from '../i18n';

type Slot = ReturnType<typeof pageSlots>[number];
const SIZE = 20;

function niceStep(zoom: number): number {
  const target = 70 / zoom; // ~70 screen px between labels
  const pow = 10 ** Math.floor(Math.log10(target));
  for (const m of [1, 2, 5, 10]) if (pow * m >= target) return pow * m;
  return pow * 10;
}

export function Rulers({ slots }: { slots: Slot[] }) {
  const { editor, readOnly } = useEd();
  const state = useEditorState();
  const vp = state.viewport;
  const slot = slots.find((s) => s.page.id === state.activePageId) ?? slots[0];
  const [drag, setDrag] = useState<{ axis: 'x' | 'y'; pos: number; index: number | null } | null>(null);
  const wrap = useRef<HTMLDivElement>(null);
  if (!slot) return null;
  const ox = slot.rect.x;
  const oy = slot.rect.y;
  const step = niceStep(vp.zoom);
  const guides = slot.page.guides ?? [];

  // Screen ↔ page-local conversions for this page.
  const sx = (px: number) => (px + ox - vp.x) * vp.zoom;
  const sy = (py: number) => (py + oy - vp.y) * vp.zoom;
  const toLocal = (e: { clientX: number; clientY: number }) => {
    const r = wrap.current!.parentElement!.getBoundingClientRect();
    return { x: (e.clientX - r.left) / vp.zoom + vp.x - ox, y: (e.clientY - r.top) / vp.zoom + vp.y - oy };
  };

  const ticks = (len: number, axis: 'x' | 'y') => {
    const out: React.ReactNode[] = [];
    const startScene = axis === 'x' ? vp.x - ox : vp.y - oy;
    const endScene = startScene + len / vp.zoom;
    for (let v = Math.floor(startScene / step) * step; v <= endScene; v += step / 5) {
      const major = Math.abs(v / step - Math.round(v / step)) < 1e-6;
      const p = axis === 'x' ? sx(v) : sy(v);
      out.push(axis === 'x'
        ? <line key={v} x1={p} y1={major ? 6 : 13} x2={p} y2={SIZE} />
        : <line key={v} y1={p} x1={major ? 6 : 13} y2={p} x2={SIZE} />);
      if (major) out.push(axis === 'x'
        ? <text key={`t${v}`} x={p + 3} y={9}>{Math.round(v)}</text>
        : <text key={`t${v}`} x={9} y={p - 3} transform={`rotate(-90 9 ${p - 3})`}>{Math.round(v)}</text>);
    }
    return out;
  };

  const begin = (e: RPE, axis: 'x' | 'y', index: number | null) => {
    if (readOnly) return;
    e.preventDefault();
    e.stopPropagation();
    (e.target as Element).setPointerCapture(e.pointerId);
    const l = toLocal(e);
    setDrag({ axis, pos: axis === 'x' ? l.x : l.y, index });
  };
  const move = (e: RPE) => {
    if (!drag) return;
    const l = toLocal(e);
    setDrag({ ...drag, pos: drag.axis === 'x' ? l.x : l.y });
  };
  const end = (e: RPE) => {
    if (!drag) return;
    const l = toLocal(e);
    const { w, h } = editor.doc.meta.dims;
    const inside = drag.axis === 'x' ? l.x >= 0 && l.x <= w && l.y > -SIZE / vp.zoom : l.y >= 0 && l.y <= h && l.x > -SIZE / vp.zoom;
    const next = guides.filter((_, i) => i !== drag.index);
    if (inside) next.push({ axis: drag.axis, pos: drag.pos });
    editor.setGuides(slot.page.id, next);
    setDrag(null);
  };

  const W = state.viewport.width;
  const H = state.viewport.height;
  return (
    <div ref={wrap} className="rulers" onPointerMove={move} onPointerUp={end}>
      <svg className="ruler top" width={W} height={SIZE} onPointerDown={(e) => begin(e, 'y', null)} aria-label={t('Horizontal ruler: drag down to add a guide')}>
        <rect x={sx(0)} y={0} width={editor.doc.meta.dims.w * vp.zoom} height={SIZE} className="page-span" />
        {ticks(W, 'x')}
        {guides.map((g, i) => g.axis === 'x' && (
          <path key={i} d={`M${sx(g.pos) - 5} 2 h10 l-5 7 z`} className="marker" onPointerDown={(e) => begin(e, 'x', i)} onDoubleClick={() => editor.setGuides(slot.page.id, guides.filter((_, k) => k !== i))} />
        ))}
      </svg>
      <svg className="ruler left" width={SIZE} height={H} onPointerDown={(e) => begin(e, 'x', null)} aria-label={t('Vertical ruler: drag right to add a guide')}>
        <rect x={0} y={sy(0)} width={SIZE} height={editor.doc.meta.dims.h * vp.zoom} className="page-span" />
        {ticks(H, 'y')}
        {guides.map((g, i) => g.axis === 'y' && (
          <path key={i} d={`M2 ${sy(g.pos) - 5} v10 l7 -5 z`} className="marker" onPointerDown={(e) => begin(e, 'y', i)} onDoubleClick={() => editor.setGuides(slot.page.id, guides.filter((_, k) => k !== i))} />
        ))}
      </svg>
      <div className="ruler-corner" title={t('Rulers (⇧R)')} />
      {drag && (
        <svg className="ruler-drag" width={W} height={H}>
          {drag.axis === 'x'
            ? <line x1={sx(drag.pos)} y1={0} x2={sx(drag.pos)} y2={H} />
            : <line x1={0} y1={sy(drag.pos)} x2={W} y2={sy(drag.pos)} />}
          <text x={drag.axis === 'x' ? sx(drag.pos) + 6 : 26} y={drag.axis === 'x' ? 34 : sy(drag.pos) - 6}>{Math.round(drag.pos)}</text>
        </svg>
      )}
    </div>
  );
}
