/** Form element tools (FM-001): edit fields, see responses, export CSV. */
import type { FormEl, FormField, FormFieldKind } from '@oc/engine';
import { ArrowDown, ArrowUp, ClipboardList, Download, ListChecks, Plus, Trash2 } from 'lucide-react';
import { useEffect, useState } from 'react';
import { formsApi } from '../api';
import { Modal } from '../components/ui';
import { ColorButton } from './ColorPicker';
import { useDoc, useEd } from './context';
import { solid } from '@oc/engine';
import { t, tk, tn } from '../i18n';

const KINDS: Array<[FormFieldKind, string]> = [
  ['text', tk('Short answer')], ['longtext', tk('Paragraph')], ['email', tk('Email')], ['number', tk('Number')], ['date', tk('Date')], ['choice', tk('Multiple choice')], ['checkboxes', tk('Checkboxes')],
];

export function FormControls({ el }: { el: FormEl }) {
  const { editor } = useEd();
  const doc = useDoc();
  const [edit, setEdit] = useState(false);
  const [responses, setResponses] = useState(false);
  return (
    <div className="group">
      <button className="btn ghost sm" onClick={() => setEdit(true)}><ListChecks size={16} /> {t('Edit form')}</button>
      <button className="btn ghost sm" onClick={() => setResponses(true)}><ClipboardList size={16} /> {t('Responses')}</button>
      <ColorButton title={t('Button colour')} doc={doc} value={solid(el.style.accent)} onChange={(p) => p.k === 'solid' && editor.updateSelected({ style: { ...el.style, accent: p.color } })} />
      <span className="faint" style={{ fontSize: 12 }}>{t('Live on public links and published websites')}</span>
      {edit && <FormEditor el={el} onClose={() => setEdit(false)} />}
      {responses && <ResponsesDialog el={el} onClose={() => setResponses(false)} />}
    </div>
  );
}

function FormEditor({ el, onClose }: { el: FormEl; onClose: () => void }) {
  const { editor } = useEd();
  const [f, setF] = useState({ title: el.title, submitLabel: el.submitLabel, thanks: el.thanks, fields: el.fields });
  const setField = (i: number, patch: Partial<FormField>) => setF({ ...f, fields: f.fields.map((x, k) => (k === i ? { ...x, ...patch } : x)) });
  const move = (i: number, d: number) => {
    const list = [...f.fields];
    const [x] = list.splice(i, 1);
    list.splice(Math.max(0, Math.min(list.length, i + d)), 0, x!);
    setF({ ...f, fields: list });
  };
  return (
    <Modal title={t('Edit form')} onClose={onClose} size="lg">
      <label className="field">{t('Title')}<input className="input" value={f.title} onChange={(e) => setF({ ...f, title: e.target.value })} /></label>
      <div className="col" style={{ gap: 10 }}>
        {f.fields.map((x, i) => (
          <div key={x.id} className="box" style={{ padding: 12, gap: 8 }}>
            <div className="row">
              <input className="input grow" value={x.label} onChange={(e) => setField(i, { label: e.target.value })} aria-label={t('Question')} />
              <select className="input" style={{ width: 150 }} value={x.kind} onChange={(e) => setField(i, { kind: e.target.value as FormFieldKind, ...(['choice', 'checkboxes'].includes(e.target.value) && !x.options?.length ? { options: [t('Option 1'), t('Option 2')] } : {}) })}>
                {KINDS.map(([k, l]) => <option key={k} value={k}>{t(l)}</option>)}
              </select>
              <button className="btn ghost sm icon" onClick={() => move(i, -1)} aria-label={t('Move up')}><ArrowUp size={14} /></button>
              <button className="btn ghost sm icon" onClick={() => move(i, 1)} aria-label={t('Move down')}><ArrowDown size={14} /></button>
              <button className="btn ghost sm icon" onClick={() => setF({ ...f, fields: f.fields.filter((_, k) => k !== i) })} aria-label={t('Remove question')}><Trash2 size={14} /></button>
            </div>
            {(x.kind === 'choice' || x.kind === 'checkboxes') && (
              <textarea className="input" rows={3} value={(x.options ?? []).join('\n')} onChange={(e) => setField(i, { options: e.target.value.split('\n').map((o) => o.trim()).filter(Boolean) })} placeholder={t('One option per line')} />
            )}
            {(x.kind === 'text' || x.kind === 'longtext' || x.kind === 'email' || x.kind === 'number') && (
              <input className="input" value={x.placeholder ?? ''} onChange={(e) => setField(i, { placeholder: e.target.value || undefined })} placeholder={t('Placeholder (optional)')} />
            )}
            <label className="check"><input type="checkbox" checked={Boolean(x.required)} onChange={(e) => setField(i, { required: e.target.checked })} /> {t('Required')}</label>
          </div>
        ))}
        <button className="btn sm" onClick={() => setF({ ...f, fields: [...f.fields, { id: `q${Date.now().toString(36)}`, kind: 'text', label: t('New question') }] })}><Plus size={14} /> {t('Add question')}</button>
      </div>
      <div className="row">
        <label className="field grow">{t('Button')}<input className="input" value={f.submitLabel} onChange={(e) => setF({ ...f, submitLabel: e.target.value })} /></label>
        <label className="field grow">{t('Thank-you message')}<input className="input" value={f.thanks} onChange={(e) => setF({ ...f, thanks: e.target.value })} /></label>
      </div>
      <div className="row"><span className="grow" /><button className="btn" onClick={onClose}>{t('Cancel')}</button>
        <button className="btn primary" onClick={() => { editor.updateSelected({ title: f.title, submitLabel: f.submitLabel || t('Send'), thanks: f.thanks, fields: f.fields }, false, false); onClose(); }}>{t('Save')}</button>
      </div>
    </Modal>
  );
}

function ResponsesDialog({ el, onClose }: { el: FormEl; onClose: () => void }) {
  const { design } = useEd();
  const [data, setData] = useState<Awaited<ReturnType<typeof formsApi.responses>> | null>(null);
  const load = () => formsApi.responses(design.id, el.id).then(setData);
  useEffect(() => {
    void load();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
  return (
    <Modal title={t('Responses — {title}', { title: el.title || t('Form') })} onClose={onClose} size="xl">
      {!data ? <div className="spinner" /> : data.responses.length === 0 ? (
        <p className="muted" style={{ margin: 0 }}>{t('No responses yet. Share a public view link or publish the design as a website — the form works there.')}</p>
      ) : (
        <>
          <div className="row"><span className="grow">{tn(data.responses.length, '{n} response', '{n} responses')}</span>
            <a className="btn sm" href={`/api/designs/${design.id}/forms/${el.id}/responses?format=csv`}><Download size={14} /> {t('Download CSV')}</a>
          </div>
          <div className="scroll" style={{ maxHeight: 440 }}>
            <table className="table">
              <thead><tr><th>{t('Submitted')}</th>{data.fields.map((f) => <th key={f.id}>{f.label}</th>)}<th /></tr></thead>
              <tbody>
                {data.responses.map((r) => (
                  <tr key={r.id}>
                    <td>{new Date(r.created_at).toLocaleString()}</td>
                    {data.fields.map((f) => <td key={f.id}>{Array.isArray(r.data[f.id]) ? (r.data[f.id] as string[]).join(', ') : String(r.data[f.id] ?? '')}</td>)}
                    <td><button className="btn ghost sm icon" aria-label={t('Delete response')} onClick={() => formsApi.deleteResponse(design.id, el.id, r.id).then(load)}><Trash2 size={13} /></button></td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        </>
      )}
    </Modal>
  );
}
