/** Video panel: soundtrack, captions (MV-030, MV captions). */
import type { AudioClip } from '@oc/engine';
import { Download, Music, Plus, Trash2, Upload } from 'lucide-react';
import { useCallback, useEffect, useRef, useState } from 'react';
import { api, download, type UploadedAsset } from '../api';
import { useToast } from '../components/ui';
import { useDoc, useEd, useEditorState } from './context';
import { fmtTime } from './media';
import { SwatchInput } from './ColorPicker';
import { t } from '../i18n';


// ------------------------------------------------------------------ panel

export function VideoPanel() {
  const { editor } = useEd();
  const doc = useDoc();
  const state = useEditorState();
  const toast = useToast();
  const [audioAssets, setAudioAssets] = useState<UploadedAsset[]>([]);
  const [busy, setBusy] = useState('');
  const fileRef = useRef<HTMLInputElement>(null);
  const reload = useCallback(() => api.assets().then((r) => setAudioAssets(r.assets.filter((a) => a.kind === 'audio'))), []);
  useEffect(() => {
    void reload();
  }, [reload]);

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

      <h4>{t('Captions')}</h4>
      <label className="row" style={{ fontSize: 13 }}><input type="checkbox" checked={track.visible} onChange={(e) => editor.setCaptions({ ...track, visible: e.target.checked })} /> {t('Show captions')}</label>
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
    </>
  );
}
