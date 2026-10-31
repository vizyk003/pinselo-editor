/** Video panel: recording studio, soundtrack, captions (MV-040, MV-030, MV captions). */
import { newId, resolvePage, sortedPages, timelineOf, type AudioClip, type CaptionCue, type PageId } from '@oc/engine';
import { Camera, Captions, Download, Mic, MonitorUp, Music, Plus, Sparkles, Square, Trash2, Upload, Video, Waves } from 'lucide-react';
import { useCallback, useEffect, useRef, useState } from 'react';
import { api, download, type UploadedAsset } from '../api';
import { interp, useToast } from '../components/ui';
import { WHITE, paintCanvas, type Engine } from '../lib/engine';
import { useDoc, useEd, useEditorState } from './context';
import { fmtTime } from './media';
import { SwatchInput } from './ColorPicker';
import { t, tn } from '../i18n';


// ------------------------------------------------------------------ panel

export function VideoPanel() {
  const { editor } = useEd();
  const doc = useDoc();
  const state = useEditorState();
  const toast = useToast();
  const [audioAssets, setAudioAssets] = useState<UploadedAsset[]>([]);
  const [studio, setStudio] = useState<'camera' | null>(null);
  const [caps, setCaps] = useState<{ stt: boolean } | null>(null);
  const [busy, setBusy] = useState('');
  const fileRef = useRef<HTMLInputElement>(null);
  const reload = useCallback(() => api.assets().then((r) => setAudioAssets(r.assets.filter((a) => a.kind === 'audio'))), []);
  useEffect(() => {
    void reload();
    fetch('/api/capabilities').then((r) => r.json()).then(setCaps).catch(() => setCaps({ stt: false }));
  }, [reload]);

  const recordScreen = async () => {
    try {
      const screen = await navigator.mediaDevices.getDisplayMedia({ video: { frameRate: 30 }, audio: true });
      let mic: MediaStream | null = null;
      try {
        mic = await navigator.mediaDevices.getUserMedia({ audio: true });
      } catch {
        /* screen audio only */
      }
      const stream = new MediaStream([...screen.getVideoTracks(), ...(mic?.getAudioTracks() ?? screen.getAudioTracks())]);
      const rec = new MediaRecorder(stream, { mimeType: pickMime() });
      const chunks: Blob[] = [];
      rec.ondataavailable = (e) => e.data.size && chunks.push(e.data);
      setBusy(t('Recording your screen… stop sharing to finish'));
      screen.getVideoTracks()[0]!.addEventListener('ended', () => rec.state !== 'inactive' && rec.stop());
      rec.onstop = async () => {
        stream.getTracks().forEach((t) => t.stop());
        mic?.getTracks().forEach((t) => t.stop());
        setBusy(t('Uploading recording…'));
        try {
          const { asset } = await api.upload(new File(chunks, `screen-${Date.now()}.webm`, { type: 'video/webm' }));
          const page = editor.addPage();
          if (page) {
            const { w, h } = editor.doc.meta.dims;
            const s = Math.min(w / asset.w, h / asset.h);
            editor.insertVideo({ id: asset.id, w: asset.w * s, h: asset.h * s, durationMs: asset.durationMs });
          }
          toast(t('Screen recording added on a new page'));
        } catch (e) {
          toast(t('Upload failed: {message}', { message: (e as Error).message }));
        } finally {
          setBusy('');
        }
      };
      rec.start(500);
    } catch {
      toast(t('Screen recording was cancelled or isn’t allowed'));
    }
  };

  const uploadAudio = async (files: FileList | null) => {
    for (const f of [...(files ?? [])]) {
      setBusy(t('Uploading {name}…', { name: f.name }));
      try {
        const { asset } = await api.upload(f);
        if (asset.kind === 'audio') {
          editor.apply([{ op: 'asset', asset }], true);
          editor.addAudio({ id: asset.id, durationMs: asset.durationMs, name: asset.name });
        } else toast(t('{name} isn’t an audio file', { name: f.name }));
      } catch (e) {
        toast(`${f.name}: ${(e as Error).message}`);
      }
    }
    setBusy('');
    void reload();
  };

  // ---- captions
  const track = editor.captions();
  const autoCaptions = async () => {
    setBusy(t('Transcribing speech…'));
    const cues: CaptionCue[] = [];
    try {
      const { spans } = timelineOf(editor.doc);
      for (const s of spans) {
        for (const el of Object.values(editor.doc.elements)) {
          if (el.type !== 'video' || el.deleted || el.parentId !== s.page.id || el.clip.muted) continue;
          const r = await fetch(`/api/assets/${el.asset}/transcribe`, { method: 'POST', headers: { 'content-type': 'application/json' }, body: '{}' });
          const j = await r.json();
          if (!r.ok) throw new Error(j.message ?? j.error);
          const base = s.startMs + (el.timing?.startMs ?? 0);
          for (const seg of j.segments as Array<{ startMs: number; endMs: number; text: string }>) {
            if (seg.endMs <= el.clip.trimStartMs || seg.startMs >= el.clip.trimEndMs) continue;
            cues.push({ id: newId(), text: seg.text, startMs: Math.round(base + (Math.max(seg.startMs, el.clip.trimStartMs) - el.clip.trimStartMs) / el.clip.speed), endMs: Math.round(base + (Math.min(seg.endMs, el.clip.trimEndMs) - el.clip.trimStartMs) / el.clip.speed) });
          }
        }
      }
      for (const c of editor.audioClips()) {
        const r = await fetch(`/api/assets/${c.asset}/transcribe`, { method: 'POST', headers: { 'content-type': 'application/json' }, body: '{}' });
        const j = await r.json();
        if (!r.ok) throw new Error(j.message ?? j.error);
        for (const seg of j.segments as Array<{ startMs: number; endMs: number; text: string }>) {
          if (seg.endMs <= c.trimStartMs || seg.startMs >= c.trimStartMs + c.durationMs) continue;
          cues.push({ id: newId(), text: seg.text, startMs: c.startMs + Math.max(0, seg.startMs - c.trimStartMs), endMs: c.startMs + Math.min(c.durationMs, seg.endMs - c.trimStartMs) });
        }
      }
      if (!cues.length) toast(t('No speech found in this design’s video or audio'));
      else {
        editor.setCaptions({ ...track, visible: true, cues });
        toast(tn(cues.length, 'Added {n} caption', 'Added {n} captions'));
      }
    } catch (e) {
      toast((e as Error).message);
    } finally {
      setBusy('');
    }
  };

  const srt = (vtt: boolean) => {
    const ts = (ms: number) => {
      const h = Math.floor(ms / 3600000);
      const m = Math.floor((ms % 3600000) / 60000);
      const s = Math.floor((ms % 60000) / 1000);
      const r = ms % 1000;
      return `${String(h).padStart(2, '0')}:${String(m).padStart(2, '0')}:${String(s).padStart(2, '0')}${vtt ? '.' : ','}${String(r).padStart(3, '0')}`;
    };
    const body = track.cues.map((c, i) => `${vtt ? '' : `${i + 1}\n`}${ts(c.startMs)} --> ${ts(c.endMs)}\n${c.text}\n`).join('\n');
    download(new Blob([vtt ? `WEBVTT\n\n${body}` : body], { type: 'text/plain' }), `${doc.meta.title || 'captions'}.${vtt ? 'vtt' : 'srt'}`);
  };

  return (
    <>
      {busy && <div className="badge" style={{ padding: 8 }}>{busy}</div>}
      <h4>{t('Record')}</h4>
      <button className="btn" onClick={() => setStudio('camera')}><Camera size={16} /> {t('Record yourself presenting')}</button>
      <button className="btn" onClick={recordScreen}><MonitorUp size={16} /> {t('Record your screen')}</button>
      <p className="faint" style={{ margin: 0, fontSize: 12 }}>{t('Recordings are uploaded to your library and placed into the design. Click through your pages while recording — each page gets its own part of the clip.')}</p>

      <h4>{t('Soundtrack')}</h4>
      {editor.audioClips().map((c: AudioClip) => (
        <div key={c.id} className="row" style={{ fontSize: 13 }}>
          <Music size={14} className="faint" />
          <span className="grow" style={{ overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{c.name ?? t('Audio')}</span>
          <span className="faint">{fmtTime(c.startMs)}–{fmtTime(c.startMs + c.durationMs)}</span>
          <button className="btn ghost sm icon" aria-label={t('Remove')} onClick={() => editor.removeAudio(c.id)}><Trash2 size={13} /></button>
        </div>
      ))}
      <button className="btn sm" onClick={() => fileRef.current?.click()}><Upload size={14} /> {t('Upload music or voice-over')}</button>
      <input ref={fileRef} type="file" accept="audio/*" multiple hidden onChange={(e) => uploadAudio(e.target.files)} />
      {audioAssets.length > 0 && (
        <div className="col" style={{ gap: 4 }}>
          <span className="faint" style={{ fontSize: 12 }}>{t('Your audio — click to add at the playhead')}</span>
          {audioAssets.map((a) => (
            <button key={a.id} className="btn ghost sm" style={{ justifyContent: 'flex-start' }} onClick={() => {
              editor.apply([{ op: 'asset', asset: { id: a.id as never, kind: 'audio', sha256: a.sha256, mime: a.mime, w: 0, h: 0, name: a.name, durationMs: a.durationMs ?? undefined } }], true);
              editor.addAudio({ id: a.id as never, durationMs: a.durationMs ?? undefined, name: a.name }, state.playhead ?? 0);
              toast(t('Added to the soundtrack'));
            }}><Plus size={13} /> {a.name} <span className="faint">{fmtTime(a.durationMs ?? 0)}</span></button>
          ))}
        </div>
      )}
      <p className="faint" style={{ margin: 0, fontSize: 12 }}><Waves size={11} /> {interp(t('Select a clip in the timeline and use {name} to cut pages on the beat.'), { name: <b>{t('Beat sync')}</b> })}</p>

      <h4>{t('Captions')}</h4>
      <label className="row" style={{ fontSize: 13 }}><input type="checkbox" checked={track.visible} onChange={(e) => editor.setCaptions({ ...track, visible: e.target.checked })} /> {t('Show captions')}</label>
      <button className="btn sm" disabled={!caps?.stt || Boolean(busy)} onClick={autoCaptions} title={caps?.stt ? '' : t('Needs a local speech-to-text model on the server')}><Sparkles size={14} /> {t('Auto-generate from speech')}</button>
      {caps && !caps.stt && <p className="faint" style={{ margin: 0, fontSize: 12 }}>{t('Automatic captions aren’t set up on this server yet. You can still add captions by hand.')}</p>}
      <button className="btn sm" onClick={() => {
        const at = state.playhead ?? 0;
        editor.upsertCue({ startMs: Math.round(at), endMs: Math.round(at + 2000), text: t('New caption') });
        editor.setCaptions({ ...editor.captions(), visible: true });
      }}><Plus size={14} /> {t('Add caption at playhead ({time})', { time: fmtTime(state.playhead ?? 0) })}</button>
      {track.cues.map((c) => (
        <div key={c.id} className="cue-row">
          <input className="input" style={{ height: 28, fontSize: 12 }} type="number" step={0.1} value={(c.startMs / 1000).toFixed(1)} onChange={(e) => editor.upsertCue({ id: c.id, startMs: Math.round(+e.target.value * 1000) })} aria-label={t('Start (s)')} />
          <input className="input" style={{ height: 28, fontSize: 12 }} type="number" step={0.1} value={(c.endMs / 1000).toFixed(1)} onChange={(e) => editor.upsertCue({ id: c.id, endMs: Math.round(+e.target.value * 1000) })} aria-label={t('End (s)')} />
          <input className="input" style={{ height: 28, fontSize: 12 }} value={c.text} onChange={(e) => editor.upsertCue({ id: c.id, text: e.target.value })} aria-label={t('Caption text')} />
          <button className="btn ghost sm icon" aria-label={t('Delete caption')} onClick={() => editor.setCaptions({ ...track, cues: track.cues.filter((x) => x.id !== c.id) })}><Trash2 size={13} /></button>
        </div>
      ))}
      {track.cues.length > 0 && (
        <>
          <div className="row">
            <label className="row" style={{ fontSize: 12 }}>{t('Size')}<input type="range" min={14} max={140} value={track.style.size} onChange={(e) => editor.setCaptions({ ...track, style: { ...track.style, size: +e.target.value } }, true)} /></label>
            <SwatchInput value={track.style.color} onChange={(c) => editor.setCaptions({ ...track, style: { ...track.style, color: c } }, true)} title={t('Caption colour')} doc={editor.doc} />
            <SwatchInput value={track.style.background} onChange={(c) => editor.setCaptions({ ...track, style: { ...track.style, background: c } }, true)} title={t('Caption background')} doc={editor.doc} />
            <select className="input" style={{ width: 90, height: 30 }} value={track.style.position} onChange={(e) => editor.setCaptions({ ...track, style: { ...track.style, position: e.target.value as 'bottom' } })} aria-label={t('Caption position')}>
              <option value="bottom">{t('Bottom')}</option><option value="middle">{t('Middle')}</option><option value="top">{t('Top')}</option>
            </select>
          </div>
          <div className="row">
            <button className="btn ghost sm" onClick={() => srt(false)}><Download size={13} /> .srt</button>
            <button className="btn ghost sm" onClick={() => srt(true)}><Download size={13} /> .vtt</button>
          </div>
        </>
      )}
      {studio && <RecordStudio onClose={() => setStudio(null)} />}
    </>
  );
}

function pickMime(): string {
  for (const m of ['video/webm;codecs=vp9,opus', 'video/webm;codecs=vp8,opus', 'video/webm', 'video/mp4']) if (MediaRecorder.isTypeSupported(m)) return m;
  return '';
}

// ------------------------------------------------------------------ recording studio

function PagePreview({ engine, pageId }: { engine: Engine; pageId: PageId }) {
  const { editor } = useEd();
  const ref = useRef<HTMLCanvasElement>(null);
  useEffect(() => {
    const el = ref.current!;
    const { w, h } = editor.doc.meta.dims;
    const W = Math.min(1280, window.innerWidth - 64);
    const H = Math.min((W / w) * h, window.innerHeight - 180);
    const scale = Math.min(W / w, H / h);
    const draw = () => paintCanvas(engine, el, w * scale, h * scale, WHITE, resolvePage(editor.doc, pageId), scale);
    draw();
    // Images and fonts arrive after the first paint.
    return engine.painter.onInvalidate(draw);
  }, [engine, editor, pageId]);
  return <canvas ref={ref} />;
}

/**
 * "Record yourself presenting": camera + mic while you click through pages. On stop, the clip is uploaded once
 * and each visited page gets a camera bubble trimmed to its segment, with the page duration set to match.
 */
function RecordStudio({ onClose }: { onClose: () => void }) {
  const { editor, engine } = useEd();
  const toast = useToast();
  const pages = sortedPages(editor.doc).filter((p) => !p.hidden);
  const [i, setI] = useState(Math.max(0, pages.findIndex((p) => p.id === editor.state.activePageId)));
  const [recording, setRecording] = useState(false);
  const [elapsed, setElapsed] = useState(0);
  const [status, setStatus] = useState('');
  const cam = useRef<HTMLVideoElement>(null);
  const stream = useRef<MediaStream | null>(null);
  const rec = useRef<MediaRecorder | null>(null);
  const chunks = useRef<Blob[]>([]);
  const marks = useRef<Array<{ pageId: PageId; at: number }>>([]);
  const t0 = useRef(0);

  useEffect(() => {
    navigator.mediaDevices.getUserMedia({ video: { width: 720, height: 720 }, audio: true })
      .then((s) => {
        stream.current = s;
        if (cam.current) cam.current.srcObject = s;
      })
      .catch(() => setStatus(t('Camera or microphone permission was denied.')));
    return () => stream.current?.getTracks().forEach((tr) => tr.stop());
  }, []);
  useEffect(() => {
    if (!recording) return;
    const iv = setInterval(() => setElapsed(performance.now() - t0.current), 200);
    return () => clearInterval(iv);
  }, [recording]);

  const go = (k: number) => {
    const n = Math.max(0, Math.min(pages.length - 1, k));
    if (n === i) return;
    setI(n);
    if (recording) marks.current.push({ pageId: pages[n]!.id, at: performance.now() - t0.current });
  };

  const start = () => {
    if (!stream.current) return;
    chunks.current = [];
    const r = new MediaRecorder(stream.current, { mimeType: pickMime() });
    r.ondataavailable = (e) => e.data.size && chunks.current.push(e.data);
    r.onstop = finish;
    rec.current = r;
    t0.current = performance.now();
    marks.current = [{ pageId: pages[i]!.id, at: 0 }];
    r.start(500);
    setRecording(true);
  };

  const finish = async () => {
    const total = performance.now() - t0.current;
    setRecording(false);
    setStatus(t('Uploading your recording…'));
    try {
      const { asset } = await api.upload(new File(chunks.current, `talk-${Date.now()}.webm`, { type: 'video/webm' }));
      editor.apply([{ op: 'asset', asset }], true);
      const segs = marks.current.map((m, k) => ({ pageId: m.pageId, start: m.at, end: marks.current[k + 1]?.at ?? Math.min(total, asset.durationMs ?? total) }));
      const { w, h } = editor.doc.meta.dims;
      const size = Math.min(w, h) * 0.3;
      for (const s of segs) {
        if (s.end - s.start < 300) continue;
        editor.setActivePage(s.pageId);
        const ids = editor.insertVideo({ id: asset.id, w: size, h: size, durationMs: asset.durationMs }, [w - size * 0.62, h - size * 0.62]);
        const id = ids[0];
        if (!id) continue;
        // Square-crop the camera frame and round it into a bubble.
        const aspect = asset.w / asset.h;
        const crop = aspect > 1 ? { x: (1 - 1 / aspect) / 2, y: 0, w: 1 / aspect, h: 1 } : { x: 0, y: (1 - aspect) / 2, w: 1, h: aspect };
        editor.apply([
          { op: 'update', id, patch: { crop, cornerRadius: size / 2, clip: { trimStartMs: Math.round(s.start), trimEndMs: Math.round(s.end), speed: 1, volume: 1, muted: false, loop: false } } },
          { op: 'page', action: 'update', pageId: s.pageId, patch: { durationMs: Math.max(1000, Math.round(s.end - s.start)) } },
        ]);
      }
      toast(tn(segs.filter((s) => s.end - s.start >= 300).length, 'Recording added to {n} page', 'Recording added to {n} pages'));
      onClose();
    } catch (e) {
      setStatus(t('Upload failed: {message}', { message: (e as Error).message }));
    }
  };

  useEffect(() => {
    const key = (e: KeyboardEvent) => {
      if (e.key === 'ArrowRight' || e.key === ' ') go(i + 1);
      else if (e.key === 'ArrowLeft') go(i - 1);
      else if (e.key === 'Escape' && !recording) onClose();
    };
    window.addEventListener('keydown', key);
    return () => window.removeEventListener('keydown', key);
  });

  return (
    <div className="rec" role="dialog" aria-label={t('Recording studio')}>
      <header>
        <b>{t('Recording studio')}</b>
        {recording && <><span className="dot" /> <span>{fmtTime(elapsed)}</span></>}
        <span className="grow" />
        <span className="faint">{t('Page {page} / {count} · use ← → to change pages', { page: i + 1, count: pages.length })}</span>
        {!recording && <button className="btn sm" onClick={onClose}>{t('Close')}</button>}
      </header>
      <div className="stage">
        {pages[i] && <PagePreview engine={engine} pageId={pages[i]!.id} />}
        <video ref={cam} className="cam" autoPlay muted playsInline />
      </div>
      <footer>
        <button className="btn" onClick={() => go(i - 1)}>‹ {t('Prev')}</button>
        {recording ? (
          <button className="btn danger" onClick={() => rec.current?.stop()}><Square size={16} /> {t('Stop & save')}</button>
        ) : (
          <button className="btn primary" onClick={start} disabled={!stream.current && !status}><Mic size={16} /> {t('Start recording')}</button>
        )}
        <button className="btn" onClick={() => go(i + 1)}>{t('Next')} ›</button>
        {status && <span>{status}</span>}
      </footer>
    </div>
  );
}
