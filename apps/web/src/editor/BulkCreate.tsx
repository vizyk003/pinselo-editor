/** Bulk create (ED-090): data (CSV / pasted spreadsheet) → one page per row with bound text and images. */
import { bodyPlainText, type AssetId, type Element } from '@oc/engine';
import { Database, Image as ImageIcon, Type } from 'lucide-react';
import { useMemo, useState } from 'react';
import { api, ApiError, contentApi } from '../api';
import { Modal, useToast } from '../components/ui';
import { parseTable } from '../lib/csv';
import { useDoc, useEd, useEditorState } from './context';
import { t, tn } from '../i18n';

export function BulkCreateDialog({ onClose }: { onClose: () => void }) {
  const { editor } = useEd();
  const doc = useDoc();
  const state = useEditorState();
  const toast = useToast();
  const [raw, setRaw] = useState('');
  const [busy, setBusy] = useState<string | null>(null);
  const table = useMemo(() => (raw.trim() ? parseTable(raw) : null), [raw]);
  const els = editor.bindableElements(state.activePageId);
  const label = (e: Element) => (e.type === 'text' ? `“${bodyPlainText(e.body).slice(0, 40) || t('Text')}”` : e.name ?? (e.type === 'frame' ? t('Frame') : t('Image')));
  const bound = els.filter((e) => e.dataBinding);
  void doc;

  const autoConnect = () => {
    if (!table) return;
    for (const e of els) {
      if (e.dataBinding) continue;
      const key = (e.type === 'text' ? bodyPlainText(e.body) : e.name ?? '').replace(/[{}]/g, '').trim().toLowerCase();
      const col = table.headers.find((h) => h.toLowerCase() === key);
      if (col) editor.setDataBinding(e.id, col);
    }
  };

  const generate = async () => {
    if (!table?.rows.length) return;
    setBusy(t('Preparing images…'));
    try {
      // Image columns: public URLs are imported; other values match your uploads by file name.
      const imageCols = new Set(bound.filter((e) => e.type !== 'text').map((e) => e.dataBinding!.column));
      const images = new Map<string, { id: AssetId; w: number; h: number }>();
      const uploads = imageCols.size ? (await api.assets()).assets : [];
      const values = new Set(table.rows.flatMap((r) => [...imageCols].map((c) => r[c]).filter((v): v is string => Boolean(v))));
      let i = 0;
      for (const v of values) {
        setBusy(t('Preparing images… {done}/{total}', { done: ++i, total: values.size }));
        if (/^https:\/\//i.test(v)) {
          const { asset } = await contentApi.importUrl(v).catch((e) => { toast(e instanceof ApiError ? e.message ?? v : v); return { asset: null }; });
          if (asset) {
            editor.apply([{ op: 'asset', asset }], true);
            images.set(v, { id: asset.id, w: asset.w, h: asset.h });
          }
        } else {
          const u = uploads.find((a) => a.name === v || a.name.replace(/\.\w+$/, '') === v);
          if (u) {
            editor.apply([{ op: 'asset', asset: { id: u.id as AssetId, kind: u.kind, sha256: u.sha256, mime: u.mime, w: u.w, h: u.h, name: u.name } }], true);
            images.set(v, { id: u.id as AssetId, w: u.w, h: u.h });
          }
        }
      }
      setBusy(t('Creating pages…'));
      const pages = editor.bulkCreate(state.activePageId, table.rows, images);
      toast(tn(pages.length, 'Created {n} page', 'Created {n} pages'));
      if (pages[0]) editor.setActivePage(pages[0], true);
      onClose();
    } finally {
      setBusy(null);
    }
  };

  return (
    <Modal title={t('Bulk create')} onClose={onClose} size="lg">
      <p className="muted" style={{ margin: 0 }}>{t('Make one page per row of data — certificates, name badges, product cards. This page is the template.')}</p>
      <label className="field">
        <span className="row"><Database size={14} /> {t('Data — paste from a spreadsheet or upload a CSV')}</span>
        <textarea className="input" rows={5} value={raw} onChange={(e) => setRaw(e.target.value)} placeholder={'Name,Title,Photo\nAda Lovelace,Mathematician,https://…/ada.jpg'} />
      </label>
      <input type="file" accept=".csv,.tsv,text/csv,text/tab-separated-values" onChange={async (e) => { const f = e.target.files?.[0]; if (f) setRaw(await f.text()); }} />
      {table && (
        <div className="scroll" style={{ maxHeight: 160, border: '1px solid var(--border)', borderRadius: 8 }}>
          <table className="table">
            <thead><tr>{table.headers.map((h) => <th key={h}>{h}</th>)}</tr></thead>
            <tbody>{table.rows.slice(0, 5).map((r, i) => <tr key={i}>{table.headers.map((h) => <td key={h}>{r[h]}</td>)}</tr>)}</tbody>
          </table>
          {table.rows.length > 5 && <div className="faint" style={{ fontSize: 12, padding: 6 }}>{tn(table.rows.length - 5, '…and {n} more row', '…and {n} more rows')}</div>}
        </div>
      )}
      {table && (
        <>
          <div className="row"><h4 style={{ margin: 0 }} className="grow">{t('Connect data to elements')}</h4><button className="btn sm" onClick={autoConnect}>{t('Auto-connect by name')}</button></div>
          <div className="list">
            {els.map((e) => (
              <div className="item" key={e.id}>
                {e.type === 'text' ? <Type size={16} /> : <ImageIcon size={16} />}
                <div className="main">{label(e)}</div>
                <select className="input" style={{ width: 180 }} value={e.dataBinding?.column ?? ''} onChange={(ev) => editor.setDataBinding(e.id, ev.target.value || null)}>
                  <option value="">{t('Not connected')}</option>
                  {table.headers.map((h) => <option key={h} value={h}>{h}</option>)}
                </select>
              </div>
            ))}
            {!els.length && <div className="empty">{t('Add text or images to this page first.')}</div>}
          </div>
          <p className="faint" style={{ margin: 0, fontSize: 12 }}>{t('Image columns can hold https:// links or the file names of your uploads.')}</p>
        </>
      )}
      <div className="row"><span className="grow" />
        <button className="btn" onClick={onClose}>{t('Cancel')}</button>
        <button className="btn primary" disabled={!table?.rows.length || !bound.length || Boolean(busy)} onClick={generate}>{busy ?? tn(table?.rows.length ?? 0, 'Generate {n} page', 'Generate {n} pages')}</button>
      </div>
    </Modal>
  );
}
