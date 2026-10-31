/**
 * Image eraser brush (g55): paint over parts of a picture to make them transparent. The result is uploaded as a new
 * PNG and replaces the image's file (same size, so the crop is kept); the original upload stays in Uploads.
 */
import type { AssetRef, ElementId, ImageEl } from '@oc/engine';
import { RotateCcw, Undo2 } from 'lucide-react';
import { useEffect, useRef, useState } from 'react';
import { api, assetUrl } from '../api';
import { Modal, useToast } from '../components/ui';
import { t } from '../i18n';
import { useEd } from './context';

const EVENT = 'oc:image-eraser';
const MAX_SIDE = 2048;

/** Open the image eraser for an image element (the context menu does this). */
export function openImageEraser(id: ElementId) {
  window.dispatchEvent(new CustomEvent<ElementId>(EVENT, { detail: id }));
}

export function ImageEraserHost() {
  const { editor } = useEd();
  const [id, setId] = useState<ElementId | null>(null);
  useEffect(() => {
    const on = (e: Event) => setId((e as CustomEvent<ElementId>).detail);
    window.addEventListener(EVENT, on);
    return () => window.removeEventListener(EVENT, on);
  }, []);
  const el = id ? editor.doc.elements[id] : undefined;
  if (!el || el.type !== 'image') return null;
  return <ImageEraserDialog el={el} onClose={() => setId(null)} />;
}

function ImageEraserDialog({ el, onClose }: { el: ImageEl; onClose: () => void }) {
  const { editor } = useEd();
  const toast = useToast();
  const ref = useRef<HTMLCanvasElement>(null);
  const [ready, setReady] = useState(false);
  const [busy, setBusy] = useState(false);
  const [size, setSize] = useState(40);
  const [steps, setSteps] = useState(0);
  const history = useRef<ImageData[]>([]);
  const original = useRef<ImageBitmap | null>(null);
  const last = useRef<[number, number] | null>(null);

  useEffect(() => {
    let off = false;
    void (async () => {
      try {
        const r = await fetch(assetUrl(el.asset), { credentials: 'include' });
        if (!r.ok) throw new Error(String(r.status));
        const bmp = await createImageBitmap(await r.blob());
        if (off) return;
        original.current = bmp;
        reset();
        setReady(true);
      } catch {
        toast(t('This image could not be opened for erasing.'));
        onClose();
      }
    })();
    return () => { off = true; };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [el.asset]);

  const reset = () => {
    const c = ref.current, bmp = original.current;
    if (!c || !bmp) return;
    const k = Math.min(1, MAX_SIDE / Math.max(bmp.width, bmp.height));
    c.width = Math.max(1, Math.round(bmp.width * k));
    c.height = Math.max(1, Math.round(bmp.height * k));
    const g = c.getContext('2d')!;
    g.clearRect(0, 0, c.width, c.height);
    g.drawImage(bmp, 0, 0, c.width, c.height);
    history.current = [];
    setSteps(0);
  };

  /** Pointer position in canvas pixels, and canvas pixels per screen pixel. */
  const at = (e: React.PointerEvent<HTMLCanvasElement>): { p: [number, number]; k: number } => {
    const c = e.currentTarget;
    const r = c.getBoundingClientRect();
    const k = c.width / r.width;
    return { p: [(e.clientX - r.left) * k, (e.clientY - r.top) * k], k };
  };
  const erase = (from: [number, number], to: [number, number], k: number) => {
    const g = ref.current?.getContext('2d');
    if (!g) return;
    g.save();
    g.globalCompositeOperation = 'destination-out';
    g.lineCap = 'round';
    g.lineJoin = 'round';
    g.lineWidth = size * k;
    g.beginPath();
    g.moveTo(from[0], from[1]);
    g.lineTo(to[0], to[1]);
    g.stroke();
    g.restore();
  };

  const save = async () => {
    const c = ref.current;
    if (!c) return;
    setBusy(true);
    try {
      const blob = await new Promise<Blob | null>((r) => c.toBlob(r, 'image/png'));
      if (!blob) throw new Error('encode');
      const base = (editor.doc.assets[el.asset]?.name ?? 'image').replace(/\.\w+$/, '');
      const { asset } = await api.upload(new File([blob], `${base}-erased.png`, { type: 'image/png' }));
      const a: AssetRef = asset;
      // One undo step: register the new file and point the image at it (same pixel size, so the crop still fits).
      editor.apply([{ op: 'asset', asset: a }, { op: 'update', id: el.id, patch: { asset: a.id } }]);
      onClose();
    } catch (e) {
      toast((e as Error).message || t('Could not save the image.'));
    } finally {
      setBusy(false);
    }
  };

  return (
    <Modal title={t('Erase parts of the image')} onClose={onClose} size="lg">
      <p className="muted" style={{ margin: 0 }}>{t('Paint over the parts to remove. They become transparent.')}</p>
      <div className="eraser-stage">
        <canvas ref={ref} className="eraser-canvas" aria-label={t('Image to erase')} style={{ touchAction: 'none', cursor: 'crosshair', visibility: ready ? 'visible' : 'hidden' }}
          onPointerDown={(e) => {
            if (!ready || busy) return;
            e.currentTarget.setPointerCapture(e.pointerId);
            const c = ref.current!;
            history.current = [...history.current.slice(-9), c.getContext('2d')!.getImageData(0, 0, c.width, c.height)];
            setSteps(history.current.length);
            const { p, k } = at(e);
            last.current = p;
            erase(p, p, k);
          }}
          onPointerMove={(e) => {
            if (!last.current) return;
            const { p, k } = at(e);
            erase(last.current, p, k);
            last.current = p;
          }}
          onPointerUp={() => { last.current = null; }}
          onPointerCancel={() => { last.current = null; }} />
        {!ready && <span className="faint">{t('Loading…')}</span>}
      </div>
      <div className="row" style={{ flexWrap: 'wrap' }}>
        <label className="field grow" style={{ minWidth: 180 }}>{t('Eraser size ({value})', { value: size })}
          <input type="range" min={4} max={160} value={size} onChange={(e) => setSize(+e.target.value)} />
        </label>
        <button className="btn sm" disabled={!steps} onClick={() => {
          const d = history.current.pop();
          if (d) ref.current?.getContext('2d')?.putImageData(d, 0, 0);
          setSteps(history.current.length);
        }}><Undo2 size={14} /> {t('Undo')}</button>
        <button className="btn sm" disabled={!ready} onClick={reset}><RotateCcw size={14} /> {t('Start over')}</button>
      </div>
      <div className="row" style={{ justifyContent: 'flex-end' }}>
        <button className="btn" onClick={onClose}>{t('Cancel')}</button>
        <button className="btn primary" disabled={!ready || busy || !steps} onClick={() => void save()}>{busy ? t('Saving…') : t('Apply')}</button>
      </div>
    </Modal>
  );
}
