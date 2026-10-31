/**
 * Presenter view (PR-004): current + next slide, notes, timer. Runs in its own window; slide changes are shared
 * with any open Present window over BroadcastChannel so the audience screen follows.
 */
import { resolvePage, sortedPages, type DesignDoc, type PageId } from '@oc/engine';
import { ChevronLeft, ChevronRight, RotateCcw } from 'lucide-react';
import { useEffect, useRef, useState } from 'react';
import { useParams } from 'react-router';
import { api } from '../api';
import { BLACK, loadEngine, paintCanvas, type Engine } from '../lib/engine';
import { t } from '../i18n';

export const PRESENT_CHANNEL = (id: string) => `oc-present:${id}`;

function Slide({ engine, doc, pageId }: { engine: Engine; doc: DesignDoc; pageId: PageId | undefined }) {
  const ref = useRef<HTMLCanvasElement>(null);
  useEffect(() => {
    const el = ref.current!;
    const draw = () => {
      const { w, h } = doc.meta.dims;
      const W = el.clientWidth;
      const H = (W / w) * h;
      const dpr = window.devicePixelRatio || 1;
      el.style.height = `${H}px`;
      paintCanvas(engine, el, W * dpr, H * dpr, BLACK, pageId ? resolvePage(doc, pageId) : null, (W / w) * dpr);
    };
    draw();
    const off = engine.painter.onInvalidate(draw);
    window.addEventListener('resize', draw);
    return () => {
      off();
      window.removeEventListener('resize', draw);
    };
  }, [engine, doc, pageId]);
  return <canvas ref={ref} />;
}

export function Presenter() {
  const { id } = useParams();
  const [doc, setDoc] = useState<DesignDoc | null>(null);
  const [engine, setEngine] = useState<Engine | null>(null);
  const [i, setI] = useState(0);
  const [start, setStart] = useState(Date.now());
  const [now, setNow] = useState(Date.now());
  const chan = useRef<BroadcastChannel | null>(null);
  useEffect(() => {
    Promise.all([api.designDoc(id!), loadEngine()]).then(([r, e]) => {
      setDoc(r.doc);
      setEngine(e);
    });
    chan.current = new BroadcastChannel(PRESENT_CHANNEL(id!));
    chan.current.onmessage = (m) => typeof m.data?.index === 'number' && setI(m.data.index);
    const timer = setInterval(() => setNow(Date.now()), 500);
    return () => {
      clearInterval(timer);
      chan.current?.close();
    };
  }, [id]);
  const pages = doc ? sortedPages(doc).filter((p) => !p.hidden) : [];
  const go = (n: number) => {
    const k = Math.max(0, Math.min(pages.length - 1, n));
    setI(k);
    chan.current?.postMessage({ index: k });
  };
  useEffect(() => {
    const key = (e: KeyboardEvent) => {
      if (['ArrowRight', 'ArrowDown', ' ', 'PageDown'].includes(e.key)) go(i + 1);
      else if (['ArrowLeft', 'ArrowUp', 'PageUp'].includes(e.key)) go(i - 1);
    };
    window.addEventListener('keydown', key);
    return () => window.removeEventListener('keydown', key);
  });
  if (!doc || !engine) return <div className="presenter"><span>{t('Loading…')}</span></div>;
  const secs = Math.floor((now - start) / 1000);
  return (
    <div className="presenter">
      <div className="row" style={{ gridColumn: '1 / -1' }}>
        <b className="grow">{doc.meta.title}</b>
        <span style={{ fontVariantNumeric: 'tabular-nums', fontSize: 22 }}>{String(Math.floor(secs / 60)).padStart(2, '0')}:{String(secs % 60).padStart(2, '0')}</span>
        <button className="btn sm" onClick={() => setStart(Date.now())}><RotateCcw size={14} /> {t('Reset')}</button>
      </div>
      <div style={{ gridRow: '2 / 4' }}>
        <Slide engine={engine} doc={doc} pageId={pages[i]?.id} />
        <div className="row" style={{ marginTop: 12 }}>
          <button className="btn" onClick={() => go(i - 1)}><ChevronLeft size={16} /> {t('Previous')}</button>
          <span style={{ fontSize: 18 }}>{i + 1} / {pages.length}</span>
          <button className="btn primary" onClick={() => go(i + 1)}>{t('Next')} <ChevronRight size={16} /></button>
        </div>
      </div>
      <div style={{ gridColumn: 2, gridRow: 1 }} />
      <div className="notes">{pages[i]?.notes || <span style={{ opacity: 0.5 }}>{t('No notes for this slide.')}</span>}</div>
      <div style={{ gridColumn: 2, gridRow: 3 }}>
        <div style={{ opacity: 0.7, marginBottom: 6 }}>{t('Next')}</div>
        <Slide engine={engine} doc={doc} pageId={pages[i + 1]?.id} />
      </div>
    </div>
  );
}
