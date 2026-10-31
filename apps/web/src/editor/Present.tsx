/**
 * Present mode (PR, docs/plan/04 §4.8): full-screen, keyboard navigation; speaker notes only in the presenter view.
 * Click builds (PS-010): Next reveals the page's next build step before moving on. Autoplay (A) with loop,
 * laser pointer (L), black/white screen (B/W), and number + Enter to jump to a page.
 */
import { RenderTarget, canvas2dTransitionTarget, clickStepCount, drawTransition, pageSettleMs, resolvePage, transitionFrame, transitionProgress } from '@oc/engine';
import { childrenOf, pageDurationMs, sortedPages, timelineOf, videoSlots, type EmbedEl } from '@oc/engine';
import { EmbedOverlay } from '../components/EmbedOverlay';
import { MediaPool } from './media';
import { ChevronLeft, ChevronRight, Crosshair, Pause, Play, Repeat, X } from 'lucide-react';
import { useCallback, useEffect, useRef, useState } from 'react';
import { PRESENT_CHANNEL } from '../pages/Presenter';
import { useEd } from './context';
import { t } from '../i18n';

export function Present({ onExit }: { onExit: () => void }) {
  const { editor, engine, design } = useEd();
  const pages = sortedPages(editor.doc).filter((p) => !p.hidden);
  const start = Math.max(0, pages.findIndex((p) => p.id === editor.state.activePageId));
  const [i, setI] = useState(start);
  // Follow the presenter window (and drive it) over BroadcastChannel.
  useEffect(() => {
    const ch = new BroadcastChannel(PRESENT_CHANNEL(design.id));
    ch.onmessage = (m) => typeof m.data?.index === 'number' && setI(m.data.index);
    return () => ch.close();
  }, [design.id]);
  const ref = useRef<HTMLCanvasElement>(null);
  const rootRef = useRef<HTMLDivElement>(null);
  /** Settled pages painted once per size (`pageId|w×h|steps`), composited by the transition on the GPU. */
  const bitmaps = useRef(new Map<string, HTMLCanvasElement>());

  // Enter fullscreen once. `onExit` is read through a ref: callers pass an inline closure, and re-running this effect
  // on every parent render would exit fullscreen, whose `fullscreenchange` then closed the presentation at once.
  const onExitRef = useRef(onExit);
  onExitRef.current = onExit;
  useEffect(() => {
    void rootRef.current?.requestFullscreen?.().catch(() => {});
    const onFs = () => !document.fullscreenElement && onExitRef.current();
    document.addEventListener('fullscreenchange', onFs);
    return () => {
      document.removeEventListener('fullscreenchange', onFs);
      if (document.fullscreenElement) void document.exitFullscreen();
    };
  }, []);

  // Build steps revealed on the current page (page time of each click).
  const steps = useRef<number[]>([]);
  const [, setTick] = useState(0);
  const [auto, setAuto] = useState(false);
  const [loop, setLoop] = useState(false);
  const [laser, setLaser] = useState<{ x: number; y: number } | null>(null);
  const [laserOn, setLaserOn] = useState(false);
  const [blank, setBlank] = useState<'black' | 'white' | null>(null);
  const digits = useRef('');
  const stepsOf = (idx: number) => {
    const p = pages[idx];
    return p ? clickStepCount(childrenOf(editor.doc, p.id)) : 0;
  };
  const next = useCallback(() => {
    if (steps.current.length < stepsOf(lastI.current)) {
      steps.current = [...steps.current, performance.now() - shownAt.current];
      setTick((n) => n + 1);
      return;
    }
    setI((x) => (x + 1 < pages.length ? x + 1 : loop ? 0 : x));
  }, [pages.length, loop]); // eslint-disable-line react-hooks/exhaustive-deps
  const back = useCallback(() => {
    if (steps.current.length) {
      steps.current = steps.current.slice(0, -1);
      setTick((n) => n + 1);
      return;
    }
    setI((x) => Math.max(0, x - 1));
  }, []);

  useEffect(() => {
    const key = (e: KeyboardEvent) => {
      if (/^\d$/.test(e.key)) { digits.current = (digits.current + e.key).slice(-3); return; }
      if (e.key === 'Enter' && digits.current) {
        const n = Number(digits.current) - 1;
        digits.current = '';
        if (n >= 0 && n < pages.length) setI(n);
        e.preventDefault();
        return;
      }
      digits.current = '';
      const k = e.key.toLowerCase();
      if (blank && !['b', 'w', 'escape'].includes(k)) { setBlank(null); e.preventDefault(); return; }
      if (['ArrowRight', 'ArrowDown', ' ', 'PageDown', 'Enter'].includes(e.key)) next();
      else if (['ArrowLeft', 'ArrowUp', 'PageUp', 'Backspace'].includes(e.key)) back();
      else if (e.key === 'Home') setI(0);
      else if (e.key === 'End') setI(pages.length - 1);
      else if (k === 'b') setBlank((b) => (b === 'black' ? null : 'black'));
      else if (k === 'w') setBlank((b) => (b === 'white' ? null : 'white'));
      else if (k === 'l') setLaserOn((v) => !v);
      else if (k === 'a') setAuto((v) => !v);
      else if (e.key === 'Escape') onExit();
      else return;
      e.preventDefault();
    };
    window.addEventListener('keydown', key);
    return () => window.removeEventListener('keydown', key);
  }, [pages.length, onExit, next, back, blank]);

  // Autoplay: each build step after its predecessor settles, then the page after its duration.
  useEffect(() => {
    if (!auto) return;
    const page = pages[i];
    if (!page) return;
    const wait = steps.current.length < stepsOf(i) ? 1500 : Math.max(pageSettleMs(page, childrenOf(editor.doc, page.id)) + 600, pageDurationMs(editor.doc, page));
    const timer = setTimeout(next, wait);
    return () => clearTimeout(timer);
  }); // eslint-disable-line react-hooks/exhaustive-deps

  const prev = useRef<number | null>(null);
  const shownAt = useRef(performance.now());
  const lastI = useRef(i);
  if (lastI.current !== i) {
    // Forward: start with no build steps; backward: show the page fully built.
    steps.current = i < lastI.current ? Array.from({ length: stepsOf(i) }, () => -1e9) : [];
    prev.current = lastI.current;
    lastI.current = i;
    shownAt.current = performance.now();
  }

  // Present plays media too: its own pool, attached to the painter while mounted.
  const pool = useRef<MediaPool | null>(null);
  useEffect(() => {
    const p = new MediaPool();
    const prev = engine.painter.videoFrames;
    p.attach(engine.painter);
    pool.current = p;
    return () => {
      p.destroy();
      engine.painter.videoFrames = prev;
    };
  }, [engine]);

  useEffect(() => {
    const el = ref.current!;
    const ctx = el.getContext('2d', { alpha: false })!;
    let raf = 0;
    const sp = (pageId: string) => timelineOf(editor.doc).spans.find((x) => x.page.id === pageId);
    /** A page painted by oc-render at `pw × ph` device px: at page time `time` (animated) or settled (cached). */
    const pageBitmap = (pageId: string, pw: number, ph: number, time: number | undefined): HTMLCanvasElement => {
      const key = `${pageId}|${pw}x${ph}|${steps.current.join(',')}`;
      const cached = time === undefined ? bitmaps.current.get(key) : undefined;
      if (cached) return cached;
      const dl = resolvePage(editor.doc, pageId as never, time === undefined ? {} : { timeMs: time, captionTimeMs: (sp(pageId)?.startMs ?? 0) + time, clickSteps: steps.current });
      const t = new RenderTarget().begin(0, 0, pw, ph);
      t.clear(1, 1, 1, 1);
      t.scale(pw / dl.width, ph / dl.height);
      engine.painter.draw(t, dl);
      const c = time === undefined ? document.createElement('canvas') : (scratch ??= document.createElement('canvas'));
      c.width = pw;
      c.height = ph;
      c.getContext('2d')!.putImageData(t.end(), 0, 0);
      if (time === undefined) {
        if (bitmaps.current.size > 8) bitmaps.current.clear();
        bitmaps.current.set(key, c);
      }
      return c;
    };
    let scratch: HTMLCanvasElement | undefined;
    const draw = () => {
      cancelAnimationFrame(raf);
      const dpr = window.devicePixelRatio || 1;
      const W = el.clientWidth;
      const H = el.clientHeight;
      if (el.width !== Math.round(W * dpr) || el.height !== Math.round(H * dpr)) {
        el.width = Math.round(W * dpr);
        el.height = Math.round(H * dpr);
      }
      const page = pages[i];
      if (!page) return;
      const { w, h } = editor.doc.meta.dims;
      const z = Math.min(W / w, H / h);
      const t = performance.now() - shownAt.current;
      const tp = transitionProgress(page, t);
      const pageMs = pageDurationMs(editor.doc, page);
      const span = sp(page.id);
      pool.current?.sync(editor.doc, (span?.startMs ?? 0) + Math.min(t, pageMs - 1), t < pageMs);
      const els = childrenOf(editor.doc, page.id);
      const timed = els.some((e) => videoSlots(e).length > 0 || e.timing || e.animOut || e.motionPath || e.emphasis || e.keyframes) || Boolean(editor.doc.meta.audio?.length || editor.doc.meta.captions?.cues.length);
      const settle = Math.max(pageSettleMs(page, els), page.transition?.durationMs ?? 0, timed ? pageMs : 0, (steps.current.at(-1) ?? 0) + 3000);
      const animating = timed || t < pageSettleMs(page, els) + 50 || steps.current.some((s) => s >= 0 && t < s + 3000);
      const pw = Math.max(1, Math.round(w * z * dpr));
      const ph = Math.max(1, Math.round(h * z * dpr));
      const before = prev.current !== null ? pages[prev.current] : undefined;
      const tr = page.transition;
      ctx.setTransform(1, 0, 0, 1, 0, 0);
      ctx.fillStyle = '#000';
      ctx.fillRect(0, 0, el.width, el.height);
      // Page box, letterboxed and clipped (transitions never spill outside the slide).
      ctx.save();
      ctx.translate(Math.round(((W - w * z) / 2) * dpr), Math.round(((H - h * z) / 2) * dpr));
      ctx.beginPath();
      ctx.rect(0, 0, pw, ph);
      ctx.clip();
      ctx.scale(pw / w, ph / h);
      const tf = tr && tp < 1 ? transitionFrame(tr.kind, tp, tr.direction) : { prev: null, next: { alpha: 1, dx: 0, dy: 0, scale: 1, rot: 0, sx: 1, blur: 0 } };
      drawTransition(canvas2dTransitionTarget(ctx, pw / w), w, h, tf, (which, c) => {
        const bmp = which === 'next' ? pageBitmap(page.id, pw, ph, animating ? Math.min(t, pageMs - 1) : undefined) : before ? pageBitmap(before.id, pw, ph, undefined) : null;
        if (bmp) c.ctx.drawImage(bmp, 0, 0, w, h);
      });
      ctx.restore();
      if (t < settle + 100) raf = requestAnimationFrame(draw);
    };
    draw();
    const offP = engine.painter.onInvalidate(() => {
      bitmaps.current.clear();
      draw();
    });
    const offE = editor.subscribe(() => {
      bitmaps.current.clear();
      draw();
    });
    window.addEventListener('resize', draw);
    return () => {
      cancelAnimationFrame(raf);
      offP();
      offE();
      window.removeEventListener('resize', draw);
    };
  }, [i, editor, engine, pages]);

  return (
    <div className="present" ref={rootRef} onClick={next} style={{ cursor: laserOn ? 'none' : undefined }}
      onMouseMove={(e) => laserOn && setLaser({ x: e.clientX, y: e.clientY })} onMouseLeave={() => setLaser(null)}>
      <canvas ref={ref} />
      {pages[i] && (() => {
        // Live embeds over the letterboxed slide (A11).
        const { w, h } = editor.doc.meta.dims;
        const W = window.innerWidth, H = window.innerHeight;
        const z = Math.min(W / w, H / h);
        const embeds = childrenOf(editor.doc, pages[i]!.id).filter((e): e is EmbedEl => e.type === 'embed' && e.visible !== false && !e.xf.rot);
        return embeds.map((e) => <EmbedOverlay key={`${i}:${e.id}`} doc={editor.doc} el={e} scale={z} offset={[(W - w * z) / 2, (H - h * z) / 2]} />);
      })()}
      {blank && <div style={{ position: 'absolute', inset: 0, background: blank }} aria-label={t('Screen blanked — press any key')} />}
      {laserOn && laser && <div className="laser" style={{ left: laser.x, top: laser.y }} aria-hidden />}
      <div className="hud" onClick={(e) => e.stopPropagation()}>
        <button className="btn sm icon" onClick={back} aria-label={t('Previous')}><ChevronLeft size={16} /></button>
        <span style={{ fontSize: 13 }}>{i + 1} / {pages.length}</span>
        <button className="btn sm icon" onClick={next} aria-label={t('Next')}><ChevronRight size={16} /></button>
        <button className={`btn sm icon ${auto ? 'active' : ''}`} onClick={() => setAuto((v) => !v)} aria-pressed={auto} title={t('Autoplay (A)')}>{auto ? <Pause size={16} /> : <Play size={16} />}</button>
        <button className={`btn sm icon ${loop ? 'active' : ''}`} onClick={() => setLoop((v) => !v)} aria-pressed={loop} title={t('Loop')}><Repeat size={16} /></button>
        <button className={`btn sm icon ${laserOn ? 'active' : ''}`} onClick={() => setLaserOn((v) => !v)} aria-pressed={laserOn} title={t('Laser pointer (L)')} aria-label={t('Laser pointer (L)')}><Crosshair size={16} /></button>
        {/* Speaker notes are for the presenter only (the presenter view); the audience never sees them here. */}
        <button className="btn sm icon" onClick={onExit} aria-label={t('Exit')}><X size={16} /></button>
      </div>
    </div>
  );
}
