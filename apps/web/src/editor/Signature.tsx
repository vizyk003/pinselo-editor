/**
 * Draw panel additions (g55, g56): the eraser tile and signatures. A signature is typed (a text element in a script
 * font), drawn on a pad, or uploaded. Drawn and uploaded signatures are saved to the user's Uploads under a
 * `signature-` file name, so they follow the account to any device and come back in "Your signatures".
 */
import { hex, smoothPath } from '@oc/engine';
import { Eraser, PenLine, Signature as SignatureIcon, Trash2, Type, Upload } from 'lucide-react';
import { useCallback, useEffect, useId, useRef, useState } from 'react';
import { uploadsApi, type UploadRow } from '../api/uploads';
import { tabPanelProps, tabProps } from '../components/tabs';
import { Modal, useToast } from '../components/ui';
import { t } from '../i18n';
import { previewFamily } from '../lib/cssFonts';
import { useEd } from './context';
import { addText, assetRef, insertAssetAt } from './insert';
import { api, ApiError, assetUrl } from '../api';

/** File-name prefix that marks an upload as a saved signature. */
export const SIGNATURE_PREFIX = 'signature-';
const SCRIPT_FONTS = ['Great Vibes', 'Dancing Script', 'Allura', 'Homemade Apple', 'Sacramento', 'Caveat'];
const INKS = ['#1f2328', '#1d3f9e', '#0c7a80'];

export function EraserTile({ active, onClick }: { active: boolean; onClick: () => void }) {
  return (
    <button className="tile" aria-pressed={active} style={{ flexDirection: 'column', gap: 4, borderColor: active ? 'var(--accent)' : undefined }} onClick={onClick}
      title={t('Erase parts of your drawings. Drag across the strokes to remove.')}>
      <Eraser aria-hidden style={{ width: 30, height: 30 }} />
      <span style={{ fontSize: 12 }}>{t('Eraser')}</span>
    </button>
  );
}

function stamp() {
  return new Date().toISOString().replace(/[:.]/g, '-').slice(0, 19);
}

export function SignatureSection() {
  const { editor } = useEd();
  const toast = useToast();
  const [open, setOpen] = useState(false);
  const [saved, setSaved] = useState<UploadRow[] | null>(null);
  const load = useCallback(() => {
    uploadsApi.list({ q: SIGNATURE_PREFIX, type: 'images', sort: 'newest', limit: 12 })
      .then((r) => setSaved(r.assets.filter((a) => a.name.toLowerCase().startsWith(SIGNATURE_PREFIX))))
      .catch(() => setSaved([]));
  }, []);
  useEffect(load, [load]);
  const place = (a: UploadRow) => {
    editor.setTool('select');
    insertAssetAt(editor, assetRef(a), undefined);
  };
  return (
    <>
      <h4>{t('Signature')}</h4>
      <p className="faint" style={{ margin: 0, fontSize: 12 }}>{t('Sign a document by typing, drawing or uploading your signature.')}</p>
      <button className="btn" onClick={() => setOpen(true)}><SignatureIcon size={16} /> {t('Add a signature')}</button>
      {saved && saved.length > 0 && (
        <>
          <h4 style={{ margin: '4px 0 0' }} className="muted">{t('Your signatures')}</h4>
          <div className="sig-saved">
            {saved.map((a) => (
              <div key={a.id} className="sig-item">
                <button className="tile" onClick={() => place(a)} aria-label={t('Add signature {name}', { name: a.name })}>
                  <img src={assetUrl(a.id)} alt="" loading="lazy" />
                </button>
                <button className="btn ghost sm icon sig-del" title={t('Remove from your signatures')} aria-label={t('Remove from your signatures')}
                  onClick={() => void uploadsApi.moveToTrash(a.id).then(load).catch((e) => toast((e as Error).message))}><Trash2 size={14} /></button>
              </div>
            ))}
          </div>
        </>
      )}
      {open && <SignatureDialog onClose={() => setOpen(false)} onSaved={load} />}
    </>
  );
}

type Mode = 'type' | 'draw' | 'upload';
const MODES: readonly Mode[] = ['type', 'draw', 'upload'];

function SignatureDialog({ onClose, onSaved }: { onClose: () => void; onSaved: () => void }) {
  const { editor, engine } = useEd();
  const toast = useToast();
  const id = useId();
  const [mode, setMode] = useState<Mode>('type');
  const [name, setName] = useState('');
  const [font, setFont] = useState(SCRIPT_FONTS[0]!);
  const [ink, setInk] = useState(INKS[0]!);
  const [strokes, setStrokes] = useState<Array<Array<[number, number]>>>([]);
  const [busy, setBusy] = useState(false);
  const fileRef = useRef<HTMLInputElement>(null);
  const drawing = useRef(false);
  const families = engine.fonts.families();

  const done = () => {
    editor.setTool('select');
    onClose();
  };
  const uploadSig = async (file: File) => {
    setBusy(true);
    try {
      const { asset } = await api.upload(file);
      editor.setTool('select');
      insertAssetAt(editor, asset, undefined);
      onSaved();
      onClose();
    } catch (e) {
      toast(e instanceof ApiError ? e.message ?? t('Upload failed') : (e as Error).message);
    } finally {
      setBusy(false);
    }
  };
  const addTyped = () => {
    const v = name.trim();
    if (!v) return;
    editor.setTool('select');
    addText(editor, v.slice(0, 80), { family: font, weight: 400, size: 56, fill: { k: 'solid', color: hex(ink) } });
    done();
  };
  const PAD_W = 480, PAD_H = 180;
  const svgOf = () => {
    const paths = strokes.filter((s) => s.length).map((s) => `<path d="${smoothPath(s.length === 1 ? [s[0]!, [s[0]![0] + 0.5, s[0]![1]]] : s)}"/>`).join('');
    return `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 ${PAD_W} ${PAD_H}" width="${PAD_W}" height="${PAD_H}"><g fill="none" stroke="${ink}" stroke-width="3.5" stroke-linecap="round" stroke-linejoin="round">${paths}</g></svg>`;
  };
  const point = (e: React.PointerEvent<SVGSVGElement>): [number, number] => {
    const r = e.currentTarget.getBoundingClientRect();
    return [Math.round(((e.clientX - r.left) / r.width) * PAD_W * 10) / 10, Math.round(((e.clientY - r.top) / r.height) * PAD_H * 10) / 10];
  };

  const tabs: Array<[Mode, string, typeof Type]> = [['type', t('Type'), Type], ['draw', t('Draw'), PenLine], ['upload', t('Upload'), Upload]];
  return (
    <Modal title={t('Add a signature')} onClose={onClose} size="md">
      <div className="lib-tabs" role="tablist" aria-label={t('How to sign')}>
        {tabs.map(([m, label, Icon]) => (
          <button key={m} {...tabProps(id, MODES, mode, setMode, m)} className={mode === m ? 'on' : ''}><Icon size={14} /> {label}</button>
        ))}
      </div>
      <div className="row" role="radiogroup" aria-label={t('Ink colour')}>
        <span className="muted" style={{ fontSize: 13 }}>{t('Ink colour')}</span>
        {INKS.map((c) => (
          <button key={c} role="radio" aria-checked={ink === c} aria-label={c} onClick={() => setInk(c)}
            style={{ width: 26, height: 26, borderRadius: '50%', background: c, border: ink === c ? '3px solid var(--accent)' : '1px solid var(--border-strong)', cursor: 'pointer' }} />
        ))}
      </div>
      {mode === 'type' && (
        <div {...tabPanelProps(id, mode)} className="col">
          <label className="field">{t('Your name')}
            <input className="input" value={name} maxLength={80} onChange={(e) => setName(e.target.value)} autoFocus onKeyDown={(e) => e.key === 'Enter' && addTyped()} />
          </label>
          <div className="sig-fonts" role="radiogroup" aria-label={t('Signature style')}>
            {SCRIPT_FONTS.map((f) => {
              const fam = families.find((x) => x.family === f);
              if (!fam) return null;
              return (
                <button key={f} role="radio" aria-checked={font === f} className={`tile${font === f ? ' active' : ''}`} onClick={() => setFont(f)}
                  style={{ fontFamily: previewFamily(fam, true), color: ink, borderColor: font === f ? 'var(--accent)' : undefined }} aria-label={f}>
                  {name.trim() || t('Your name')}
                </button>
              );
            })}
          </div>
          <div className="row" style={{ justifyContent: 'flex-end' }}>
            <button className="btn primary" disabled={!name.trim()} onClick={addTyped}>{t('Add to design')}</button>
          </div>
        </div>
      )}
      {mode === 'draw' && (
        <div {...tabPanelProps(id, mode)} className="col">
          <svg className="sig-pad" viewBox={`0 0 ${PAD_W} ${PAD_H}`} role="img" aria-label={t('Signature pad. Draw your signature with a mouse, pen or finger.')}
            onPointerDown={(e) => { e.currentTarget.setPointerCapture(e.pointerId); drawing.current = true; const p = point(e); setStrokes((s) => [...s, [p]]); }}
            onPointerMove={(e) => {
              if (!drawing.current) return;
              const p = point(e);
              setStrokes((s) => {
                const last = s[s.length - 1]!;
                const q = last[last.length - 1]!;
                if (Math.hypot(p[0] - q[0], p[1] - q[1]) < 1.5) return s;
                return [...s.slice(0, -1), [...last, p]];
              });
            }}
            onPointerUp={() => { drawing.current = false; }} onPointerCancel={() => { drawing.current = false; }}>
            <line x1={24} x2={PAD_W - 24} y1={PAD_H - 36} y2={PAD_H - 36} stroke="var(--border-strong)" strokeDasharray="4 4" />
            <g fill="none" stroke={ink} strokeWidth={3.5} strokeLinecap="round" strokeLinejoin="round">
              {strokes.map((s, i) => <path key={i} d={smoothPath(s.length === 1 ? [s[0]!, [s[0]![0] + 0.5, s[0]![1]]] : s)} />)}
            </g>
          </svg>
          <p className="faint" style={{ margin: 0, fontSize: 12 }}>{t('Drawn signatures are saved to your Uploads, so you can reuse them.')}</p>
          <div className="row" style={{ justifyContent: 'flex-end' }}>
            <button className="btn" disabled={!strokes.length} onClick={() => setStrokes([])}>{t('Clear')}</button>
            <button className="btn primary" disabled={!strokes.length || busy}
              onClick={() => void uploadSig(new File([svgOf()], `${SIGNATURE_PREFIX}${stamp()}.svg`, { type: 'image/svg+xml' }))}>
              {busy ? t('Saving…') : t('Add to design')}
            </button>
          </div>
        </div>
      )}
      {mode === 'upload' && (
        <div {...tabPanelProps(id, mode)} className="col">
          <p className="muted" style={{ margin: 0 }}>{t('Upload a photo or scan of your signature, ideally a PNG with a transparent background.')}</p>
          <input ref={fileRef} type="file" accept="image/png,image/jpeg,image/webp,image/svg+xml" hidden
            onChange={(e) => {
              const f = e.target.files?.[0];
              e.target.value = '';
              if (f) void uploadSig(new File([f], `${SIGNATURE_PREFIX}${stamp()}.${(f.name.split('.').pop() ?? 'png').toLowerCase()}`, { type: f.type }));
            }} />
          <div className="row" style={{ justifyContent: 'flex-end' }}>
            <button className="btn primary" disabled={busy} onClick={() => fileRef.current?.click()}><Upload size={16} /> {busy ? t('Uploading…') : t('Choose a file')}</button>
          </div>
        </div>
      )}
    </Modal>
  );
}
