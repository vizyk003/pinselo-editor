/**
 * Timeline (MV-021): pages as blocks, the selected element's window/trim, soundtrack clips with waveforms and
 * caption cues, a scrubbable playhead, split, and per-clip volume/fades. All edits are DocOps via the Editor.
 */
import { clipLengthMs, elementWindow, KEYFRAME_NEUTRAL, KEYFRAME_PROPS, keyframeValue, pageDurationMs, timelineOf, type AudioClip, type CaptionCue, type Element, type KeyframeProp, type VideoEl } from '@oc/engine';
import { Captions, Circle, Music, Pause, Play, Scissors, SkipBack, Trash2, X as XIcon, ZoomIn, ZoomOut } from 'lucide-react';
import { useEffect, useMemo, useRef, useState, type PointerEvent as RPE } from 'react';
import { PopoverButton } from '../components/ui';
import { useDoc, useEd, useEditorState } from './context';
import { assetUrl } from '../api';
import { fmtTime } from './media';
import { t, tk } from '../i18n';

type Sel = { kind: 'audio'; id: string } | { kind: 'cue'; id: string } | null;

const peaksCache = new Map<string, Promise<number[]>>();
function usePeaks(asset: string): number[] {
  const [p, setP] = useState<number[]>([]);
  useEffect(() => {
    if (!peaksCache.has(asset)) peaksCache.set(asset, fetch(`${assetUrl(asset)}/waveform`).then((r) => r.json()).then((j: { peaks: number[] }) => j.peaks).catch(() => []));
    void peaksCache.get(asset)!.then(setP);
  }, [asset]);
  return p;
}

/** Pointer drag reporting horizontal delta in ms. */
function dragger(pxPerMs: number, onMove: (dMs: number) => void, onStart?: () => void) {
  return (e: RPE) => {
    e.stopPropagation();
    e.preventDefault();
    const x0 = e.clientX;
    const el = e.currentTarget as HTMLElement;
    el.setPointerCapture(e.pointerId);
    onStart?.();
    const move = (ev: PointerEvent) => onMove((ev.clientX - x0) / pxPerMs);
    const up = () => {
      el.removeEventListener('pointermove', move);
      el.removeEventListener('pointerup', up);
    };
    el.addEventListener('pointermove', move);
    el.addEventListener('pointerup', up);
  };
}

function Waveform({ asset, trimStartMs, durationMs, srcDurationMs, width, height }: { asset: string; trimStartMs: number; durationMs: number; srcDurationMs: number; width: number; height: number }) {
  const peaks = usePeaks(asset);
  if (!peaks.length || !srcDurationMs) return null;
  const n = Math.max(2, Math.floor(width / 3));
  const d: string[] = [];
  for (let i = 0; i < n; i++) {
    const t = trimStartMs + (i / n) * durationMs;
    const pi = Math.min(peaks.length - 1, Math.floor((t / srcDurationMs) * peaks.length));
    const v = (peaks[pi] ?? 0) * (height / 2 - 2);
    const x = (i / n) * width;
    d.push(`M${x.toFixed(1)} ${(height / 2 - v).toFixed(1)}V${(height / 2 + v).toFixed(1)}`);
  }
  return <svg width={width} height={height} style={{ position: 'absolute', inset: 0, pointerEvents: 'none' }}><path d={d.join('')} stroke="currentColor" strokeWidth={1.6} opacity={0.55} /></svg>;
}

export function Timeline({ onClose }: { onClose: () => void }) {
  const { editor, readOnly } = useEd();
  const doc = useDoc();
  const state = useEditorState();
  const wrap = useRef<HTMLDivElement>(null);
  const [width, setWidth] = useState(800);
  const [zoom, setZoom] = useState(1);
  const [sel, setSel] = useState<Sel>(null);
  const [, force] = useState(0);
  const { spans, totalMs } = useMemo(() => timelineOf(doc), [doc]);
  const mediaDur = useMemo(() => Object.fromEntries(Object.values(doc.assets).map((a) => [a.id, a.durationMs ?? 0])), [doc.assets]);

  useEffect(() => {
    const el = wrap.current!;
    const ro = new ResizeObserver(() => setWidth(el.clientWidth));
    ro.observe(el);
    return () => ro.disconnect();
  }, []);
  // Repaint the playhead while playing.
  useEffect(() => {
    if (!state.playing) return;
    let raf = 0;
    const loop = () => {
      force((x) => x + 1);
      raf = requestAnimationFrame(loop);
    };
    raf = requestAnimationFrame(loop);
    return () => cancelAnimationFrame(raf);
  }, [state.playing]);

  const pxPerMs = (Math.max(200, width - 40) / Math.max(1000, totalMs)) * zoom;
  const X = (ms: number) => 20 + ms * pxPerMs;
  const now = state.playing ? editor.currentTime() : (state.playhead ?? 0);
  // 20px of room either side of the time range: at zoom 1 the content is exactly as wide as the scroller.
  const contentW = Math.max(width, X(totalMs) + 20);

  const seekFromEvent = (clientX: number) => {
    const r = wrap.current!.getBoundingClientRect();
    const sx = wrap.current!.scrollLeft;
    editor.seek((clientX - r.left + sx - 20) / pxPerMs);
  };
  const scrub = (e: RPE) => {
    if (state.playing) editor.pause();
    seekFromEvent(e.clientX);
    const el = e.currentTarget as HTMLElement;
    el.setPointerCapture(e.pointerId);
    const move = (ev: PointerEvent) => seekFromEvent(ev.clientX);
    const up = () => {
      el.removeEventListener('pointermove', move);
      el.removeEventListener('pointerup', up);
    };
    el.addEventListener('pointermove', move);
    el.addEventListener('pointerup', up);
  };

  // Selected element on the page under the playhead (or active page).
  const selected = state.selection.length === 1 ? (doc.elements[state.selection[0]!] as Element | undefined) : undefined;
  const selSpan = selected ? spans.find((s) => s.page.id === selected.parentId) : undefined;

  const clips = doc.meta.audio ?? [];
  const cues = doc.meta.captions?.cues ?? [];
  const selClip = sel?.kind === 'audio' ? clips.find((c) => c.id === sel.id) : undefined;
  const selCue = sel?.kind === 'cue' ? cues.find((c) => c.id === sel.id) : undefined;

  const ticks: number[] = [];
  const step = [250, 500, 1000, 2000, 5000, 10000, 30000, 60000].find((s) => s * pxPerMs >= 60) ?? 60000;
  for (let ms = 0; ms <= totalMs; ms += step) ticks.push(ms);

  return (
    <div className="timeline" aria-label={t('Timeline')}>
      <div className="tl-bar">
        <button className="btn sm icon" title={t('Back to start')} onClick={() => editor.seek(0)}><SkipBack size={15} /></button>
        <button className="btn primary sm icon" title={t('Play / pause (Space)')} onClick={() => editor.togglePlay()}>{state.playing ? <Pause size={15} /> : <Play size={15} />}</button>
        <span className="tl-time">{fmtTime(now)} / {fmtTime(totalMs)}</span>
        {!readOnly && <button className="btn ghost sm" title={t('Split the selected video, or the page, at the playhead (S)')} onClick={() => editor.splitAtPlayhead()}><Scissors size={15} /> {t('Split')}</button>}
        <div className="sep" />
        {selClip && !readOnly && (
          <div className="row" style={{ gap: 6 }}>
            <Music size={14} />
            <span style={{ maxWidth: 140, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{selClip.name ?? t('Audio')}</span>
            <label className="row" style={{ fontSize: 12 }}>{t('Vol')}<input type="range" min={0} max={1.5} step={0.05} value={selClip.volume} onChange={(e) => editor.updateAudio(selClip.id, { volume: +e.target.value })} style={{ width: 70 }} /></label>
            <label className="row" style={{ fontSize: 12 }}>{t('Fade in')}<input type="range" min={0} max={5000} step={100} value={selClip.fadeInMs} onChange={(e) => editor.updateAudio(selClip.id, { fadeInMs: +e.target.value })} style={{ width: 60 }} /></label>
            <label className="row" style={{ fontSize: 12 }}>{t('out')}<input type="range" min={0} max={5000} step={100} value={selClip.fadeOutMs} onChange={(e) => editor.updateAudio(selClip.id, { fadeOutMs: +e.target.value })} style={{ width: 60 }} /></label>
            <button className="btn ghost sm" onClick={() => editor.splitAudio(selClip.id, now)} title={t('Split at playhead')}><Scissors size={14} /></button>
            <button className="btn ghost sm" onClick={() => { editor.removeAudio(selClip.id); setSel(null); }} title={t('Remove')}><Trash2 size={14} /></button>
          </div>
        )}
        {selCue && !readOnly && (
          <div className="row" style={{ gap: 6, flex: 1 }}>
            <Captions size={14} />
            <input className="input" style={{ height: 30 }} value={selCue.text} onChange={(e) => editor.upsertCue({ id: selCue.id, text: e.target.value })} aria-label={t('Caption text')} />
            <button className="btn ghost sm" onClick={() => { editor.setCaptions({ ...editor.captions(), cues: cues.filter((c) => c.id !== selCue.id) }); setSel(null); }}><Trash2 size={14} /></button>
          </div>
        )}
        {!readOnly && (
          <button className={`btn ghost sm ${state.recordKeyframes ? 'active rec' : ''}`} aria-pressed={state.recordKeyframes}
            title={t('Record motion: move or rotate an element at the playhead to add keyframes')} onClick={() => editor.setRecordKeyframes(!state.recordKeyframes)}>
            <Circle size={12} fill={state.recordKeyframes ? 'currentColor' : 'none'} /> {t('Record')}
          </button>
        )}
        <div className="grow" />
        <button className="btn ghost sm icon" onClick={() => setZoom((z) => Math.max(0.5, z / 1.5))} title={t('Zoom out')}><ZoomOut size={15} /></button>
        <button className="btn ghost sm icon" onClick={() => setZoom((z) => Math.min(20, z * 1.5))} title={t('Zoom in')}><ZoomIn size={15} /></button>
        <button className="btn ghost sm icon" onClick={onClose} title={t('Close timeline')}><XIcon size={15} /></button>
      </div>
      <div className="tl-scroll" ref={wrap}>
        <div style={{ width: contentW, position: 'relative' }}>
          <div className="tl-ruler" onPointerDown={scrub}>
            {ticks.map((ms, i) => (
              <span key={ms} className={i > 0 && i === ticks.length - 1 && X(ms) > contentW - 40 ? 'end' : undefined} style={{ left: X(ms) }}>{tickLabel(ms, step)}</span>
            ))}
          </div>

          {/* pages */}
          <div className="tl-track" onPointerDown={scrub}>
            {spans.map((s) => (
              <div key={s.page.id} className={`tl-block page ${s.page.id === state.activePageId ? 'active' : ''}`} style={{ left: X(s.startMs), width: s.durationMs * pxPerMs - 2 }}
                onPointerDown={(e) => { e.stopPropagation(); editor.setActivePage(s.page.id, true); editor.seek(s.startMs); }}>
                <span>{t('Page {n}', { n: s.index + 1 })} · {(s.durationMs / 1000).toFixed(1)}s{s.page.transition ? ` · ${s.page.transition.kind}` : ''}</span>
                {!readOnly && <i className="tl-handle right" onPointerDown={dragger(pxPerMs, (d) => editor.setPageDuration(s.page.id, s.durationMs + d))} />}
              </div>
            ))}
          </div>

          {/* selected element */}
          {selected && selSpan && <ElementTrack el={selected} span={selSpan} X={X} pxPerMs={pxPerMs} mediaDur={mediaDur[(selected as VideoEl).asset] ?? 0} />}
          {selected && selSpan && <KeyframeTrack el={selected} span={selSpan} X={X} pxPerMs={pxPerMs} now={now} />}

          {/* soundtrack */}
          <div className="tl-track audio" onPointerDown={scrub}>
            {!clips.length && <span className="tl-empty">{t('Add music from Uploads or the Video panel')}</span>}
            {clips.map((c) => <AudioBar key={c.id} c={c} X={X} pxPerMs={pxPerMs} selected={sel?.kind === 'audio' && sel.id === c.id} onSelect={() => setSel({ kind: 'audio', id: c.id })} srcDur={mediaDur[c.asset] ?? c.durationMs + c.trimStartMs} readOnly={readOnly} />)}
          </div>

          {/* captions */}
          {(cues.length > 0 || doc.meta.captions) && (
            <div className="tl-track captions" onPointerDown={scrub}>
              {cues.map((c) => <CueBar key={c.id} c={c} X={X} pxPerMs={pxPerMs} selected={sel?.kind === 'cue' && sel.id === c.id} onSelect={() => setSel({ kind: 'cue', id: c.id })} readOnly={readOnly} />)}
            </div>
          )}

          <div className="tl-playhead" style={{ left: X(now) }} onPointerDown={scrub} />
        </div>
      </div>
    </div>
  );
}

function ElementTrack({ el, span, X, pxPerMs, mediaDur }: { el: Element; span: { startMs: number; durationMs: number; page: { id: string } }; X: (ms: number) => number; pxPerMs: number; mediaDur: number }) {
  const { editor, readOnly } = useEd();
  const pageMs = span.durationMs;
  const w = elementWindow(el, pageMs);
  const isVideo = el.type === 'video';
  const v = el as VideoEl;
  const start = useRef({ w, clip: isVideo ? { ...v.clip } : null, timing: el.timing });
  const snap = () => (start.current = { w: elementWindow(el, pageMs), clip: isVideo ? { ...v.clip } : null, timing: el.timing });
  const move = dragger(pxPerMs, (d) => {
    const s0 = start.current.w;
    const len = s0.end - s0.start;
    const ns = Math.max(0, Math.min(pageMs - 100, s0.start + d));
    editor.setTiming(el.id, { startMs: Math.round(ns), ...(el.timing?.endMs !== undefined || !isVideo ? { endMs: Math.round(Math.min(pageMs, ns + len)) } : {}) });
  }, snap);
  const left = dragger(pxPerMs, (d) => {
    const s0 = start.current;
    const ns = Math.max(0, Math.min(s0.w.end - 100, s0.w.start + d));
    if (isVideo && s0.clip) {
      const dd = ns - s0.w.start;
      const trimStart = Math.max(0, Math.min(s0.clip.trimEndMs - 100, s0.clip.trimStartMs + dd * s0.clip.speed));
      editor.apply([{ op: 'update', id: el.id, patch: { clip: { ...s0.clip, trimStartMs: Math.round(trimStart) }, timing: { ...(s0.timing ?? {}), startMs: Math.round(ns) } } }], true);
    } else editor.setTiming(el.id, { startMs: Math.round(ns), endMs: Math.round(s0.w.end) });
  }, snap);
  const right = dragger(pxPerMs, (d) => {
    const s0 = start.current;
    const ne = Math.max(s0.w.start + 100, Math.min(pageMs, s0.w.end + d));
    if (isVideo && s0.clip && !s0.clip.loop) {
      const trimEnd = Math.max(s0.clip.trimStartMs + 100, Math.min(mediaDur || s0.clip.trimEndMs, s0.clip.trimStartMs + (ne - s0.w.start) * s0.clip.speed));
      editor.apply([{ op: 'update', id: el.id, patch: { clip: { ...s0.clip, trimEndMs: Math.round(trimEnd) } } }], true);
    } else editor.setTiming(el.id, { startMs: Math.round(s0.w.start), endMs: Math.round(ne) });
  }, snap);
  const label = isVideo ? `${t('Video')} · ${(clipLengthMs(v.clip) / 1000).toFixed(1)}s${v.clip.speed !== 1 ? ` · ${v.clip.speed}×` : ''}` : `${el.type} · ${((w.end - w.start) / 1000).toFixed(1)}s`;
  return (
    <div className="tl-track element">
      <div className="tl-block el" style={{ left: X(span.startMs + w.start), width: Math.max(8, (w.end - w.start) * pxPerMs - 2) }} onPointerDown={readOnly ? undefined : move}>
        <span>{label}</span>
        {!readOnly && <i className="tl-handle left" onPointerDown={left} />}
        {!readOnly && <i className="tl-handle right" onPointerDown={right} />}
      </div>
      {el.timing && !readOnly && <button className="btn ghost sm tl-reset" style={{ left: X(span.startMs + pageDurationMs(editor.doc, editor.doc.pages[span.page.id as never]!)) + 6 }} onClick={() => editor.setTiming(el.id, null)}>{t('Show whole page')}</button>}
    </div>
  );
}

function AudioBar({ c, X, pxPerMs, selected, onSelect, srcDur, readOnly }: { c: AudioClip; X: (ms: number) => number; pxPerMs: number; selected: boolean; onSelect: () => void; srcDur: number; readOnly: boolean }) {
  const { editor } = useEd();
  const s0 = useRef(c);
  const snap = () => {
    s0.current = c;
    onSelect();
  };
  const move = dragger(pxPerMs, (d) => editor.updateAudio(c.id, { startMs: Math.round(Math.max(0, s0.current.startMs + d)) }), snap);
  const left = dragger(pxPerMs, (d) => {
    const o = s0.current;
    const dd = Math.max(-o.trimStartMs, Math.min(o.durationMs - 200, d));
    editor.updateAudio(c.id, { startMs: Math.round(o.startMs + dd), trimStartMs: Math.round(o.trimStartMs + dd), durationMs: Math.round(o.durationMs - dd) });
  }, snap);
  const right = dragger(pxPerMs, (d) => {
    const o = s0.current;
    editor.updateAudio(c.id, { durationMs: Math.round(Math.max(200, Math.min(srcDur - o.trimStartMs, o.durationMs + d))) });
  }, snap);
  const w = Math.max(8, c.durationMs * pxPerMs - 2);
  return (
    <div className={`tl-block audio ${selected ? 'sel' : ''}`} style={{ left: X(c.startMs), width: w }} onPointerDown={readOnly ? (e) => { e.stopPropagation(); onSelect(); } : move}>
      <Waveform asset={c.asset} trimStartMs={c.trimStartMs} durationMs={c.durationMs} srcDurationMs={srcDur} width={w} height={34} />
      <span><Music size={11} /> {c.name ?? t('Audio')}</span>
      {c.fadeInMs > 0 && <b className="tl-fade in" style={{ width: c.fadeInMs * pxPerMs }} />}
      {c.fadeOutMs > 0 && <b className="tl-fade out" style={{ width: c.fadeOutMs * pxPerMs }} />}
      {!readOnly && <i className="tl-handle left" onPointerDown={left} />}
      {!readOnly && <i className="tl-handle right" onPointerDown={right} />}
    </div>
  );
}

function CueBar({ c, X, pxPerMs, selected, onSelect, readOnly }: { c: CaptionCue; X: (ms: number) => number; pxPerMs: number; selected: boolean; onSelect: () => void; readOnly: boolean }) {
  const { editor } = useEd();
  const s0 = useRef(c);
  const snap = () => {
    s0.current = c;
    onSelect();
  };
  const move = dragger(pxPerMs, (d) => {
    const o = s0.current;
    const st = Math.max(0, o.startMs + d);
    editor.upsertCue({ id: c.id, startMs: Math.round(st), endMs: Math.round(st + (o.endMs - o.startMs)) });
  }, snap);
  const left = dragger(pxPerMs, (d) => editor.upsertCue({ id: c.id, startMs: Math.round(Math.max(0, Math.min(s0.current.endMs - 100, s0.current.startMs + d))) }), snap);
  const right = dragger(pxPerMs, (d) => editor.upsertCue({ id: c.id, endMs: Math.round(Math.max(s0.current.startMs + 100, s0.current.endMs + d)) }), snap);
  return (
    <div className={`tl-block cue ${selected ? 'sel' : ''}`} style={{ left: X(c.startMs), width: Math.max(8, (c.endMs - c.startMs) * pxPerMs - 2) }} onPointerDown={readOnly ? undefined : move} title={c.text}>
      <span>{c.text || '…'}</span>
      {!readOnly && <i className="tl-handle left" onPointerDown={left} />}
      {!readOnly && <i className="tl-handle right" onPointerDown={right} />}
    </div>
  );
}

const KF_LABEL: Record<KeyframeProp, string> = { x: tk('Move X'), y: tk('Move Y'), scale: tk('Scale'), rotate: tk('Rotate'), opacity: tk('Opacity'), blur: tk('Blur') };

/** Keyframes of the selected element (MV-100): diamonds per key time, drag to retime, inspector to edit values. */
function KeyframeTrack({ el, span, X, pxPerMs, now }: { el: Element; span: { startMs: number; durationMs: number; page: { id: string } }; X: (ms: number) => number; pxPerMs: number; now: number }) {
  const { editor, readOnly } = useEd();
  const [open, setOpen] = useState<number | null>(null);
  const k = el.keyframes ?? {};
  const times = [...new Set(Object.values(k).flatMap((keys) => (keys ?? []).map((x) => x.t)))].sort((a, b) => a - b);
  const tNow = Math.max(0, Math.round(now - span.startMs));
  const at = open ?? tNow;
  const value = (p: KeyframeProp) => keyframeValue(k[p], at, KEYFRAME_NEUTRAL[p]);
  const keyed = (p: KeyframeProp) => (k[p] ?? []).some((x) => Math.abs(x.t - at) <= 20);
  const drag = (t0: number) => (e: RPE) => {
    if (readOnly) return;
    e.stopPropagation();
    setOpen(t0);
    const x0 = e.clientX;
    let last = t0;
    const move = (ev: PointerEvent) => {
      const nt = Math.max(0, Math.min(span.durationMs, Math.round(t0 + (ev.clientX - x0) / pxPerMs)));
      if (nt !== last) { editor.moveKeyframe(el.id, last, nt); last = nt; setOpen(nt); }
    };
    const up = () => { window.removeEventListener('pointermove', move); window.removeEventListener('pointerup', up); };
    window.addEventListener('pointermove', move);
    window.addEventListener('pointerup', up);
  };
  if (!times.length && readOnly) return null;
  return (
    <div className="tl-track keyframes">
      {times.map((t0) => (
        <button key={t0} className={`tl-kf ${open === t0 ? 'on' : ''}`} style={{ left: X(span.startMs + t0) - 7 }} onPointerDown={drag(t0)}
          title={t('Keyframe at {s} s', { s: (t0 / 1000).toFixed(2) })} aria-label={t('Keyframe at {s} s', { s: (t0 / 1000).toFixed(2) })} />
      ))}
      {!readOnly && (
        <PopoverButton className="btn ghost sm tl-kf-add" title={t('Keyframes')} label={<>◆ {open !== null ? t('Edit keyframe') : t('Add keyframe')}</>} content={(close) => (
          <div className="col pop-md">
            <h4 style={{ margin: 0 }} className="muted">{t('Keyframe at {s} s', { s: (at / 1000).toFixed(2) })}</h4>
            {KEYFRAME_PROPS.map((p) => (
              <label key={p} className="row" style={{ gap: 6, fontSize: 13 }}>
                <span className="grow">{t(KF_LABEL[p])}{keyed(p) ? ' ◆' : ''}</span>
                <input className="input" style={{ width: 90, height: 28 }} type="number" step={p === 'scale' || p === 'opacity' ? 0.05 : 1}
                  defaultValue={Math.round(value(p) * 100) / 100} onBlur={(e) => { const v = +e.target.value; if (Number.isFinite(v) && Math.abs(v - value(p)) > 1e-6) editor.setKeyframe(el.id, p, at, v); }} />
              </label>
            ))}
            <div className="row">
              <button className="btn sm" onClick={() => { for (const p of KEYFRAME_PROPS) if (k[p]?.length || p === 'x' || p === 'y') editor.setKeyframe(el.id, p, at, value(p)); close(); }}>{t('Key all here')}</button>
              {times.includes(at) && <button className="btn ghost sm" onClick={() => { for (const p of KEYFRAME_PROPS) editor.removeKeyframe(el.id, p, at); setOpen(null); close(); }}>{t('Delete keyframe')}</button>}
              {times.length > 0 && <button className="btn ghost sm" onClick={() => { editor.clearKeyframes(el.id); setOpen(null); close(); }}>{t('Clear all')}</button>}
            </div>
            <p className="faint" style={{ margin: 0, fontSize: 12 }}>{t('Tip: turn on Record, move the playhead, then drag or rotate the element.')}</p>
          </div>
        )} />
      )}
    </div>
  );
}

/** Ruler label: whole seconds as 0:05, the ticks between them with as many decimals as the step needs (0:00.25, 0:00.5). */
export function tickLabel(ms: number, step: number): string {
  const s = ms / 1000;
  const m = Math.floor(s / 60);
  const dp = ms % 1000 === 0 ? 0 : step % 500 === 0 ? 1 : 2;
  const sec = (s - m * 60).toFixed(dp);
  return `${m}:${sec.padStart(dp ? 3 + dp : 2, '0')}`;
}
