/** Find and replace across every page (ED-072). ⌘F opens it. */
import type { FindMatch } from '@oc/engine';
import { ChevronDown, ChevronUp } from 'lucide-react';
import { useEffect, useMemo, useState } from 'react';
import { Modal, useToast } from '../components/ui';
import { useDoc, useEd } from './context';
import { t, tn } from '../i18n';

export function FindReplaceDialog({ onClose }: { onClose: () => void }) {
  const { editor, readOnly } = useEd();
  const doc = useDoc();
  const toast = useToast();
  const [q, setQ] = useState(() => editor.selectedText?.() || '');
  const [r, setR] = useState('');
  const [matchCase, setMatchCase] = useState(false);
  const [wholeWord, setWholeWord] = useState(false);
  const [i, setI] = useState(0);
  const matches: FindMatch[] = useMemo(() => editor.find(q, { matchCase, wholeWord }), [editor, q, matchCase, wholeWord, doc]);
  const cur = matches[Math.min(i, matches.length - 1)];
  useEffect(() => setI(0), [q, matchCase, wholeWord]);
  const go = (d: number) => {
    if (!matches.length) return;
    const n = (i + d + matches.length) % matches.length;
    setI(n);
    editor.revealMatch(matches[n]!);
  };
  return (
    <Modal title={t('Find and replace')} onClose={onClose} size="sm">
      <div className="row">
        <input className="input grow" placeholder={t('Find')} value={q} onChange={(e) => setQ(e.target.value)} autoFocus onKeyDown={(e) => e.key === 'Enter' && go(e.shiftKey ? -1 : 1)} />
        <span className="faint" style={{ minWidth: 64, textAlign: 'right', fontSize: 12 }}>{q ? (matches.length ? t('{current} of {total}', { current: Math.min(i, matches.length - 1) + 1, total: matches.length }) : t('No results')) : ''}</span>
        <button className="btn sm icon" onClick={() => go(-1)} disabled={!matches.length} aria-label={t('Previous match')}><ChevronUp size={16} /></button>
        <button className="btn sm icon" onClick={() => go(1)} disabled={!matches.length} aria-label={t('Next match')}><ChevronDown size={16} /></button>
      </div>
      <div className="row">
        <label className="check"><input type="checkbox" checked={matchCase} onChange={(e) => setMatchCase(e.target.checked)} /> {t('Match case')}</label>
        <label className="check"><input type="checkbox" checked={wholeWord} onChange={(e) => setWholeWord(e.target.checked)} /> {t('Whole words')}</label>
      </div>
      {!readOnly && (
        <>
          <input className="input" placeholder={t('Replace with')} value={r} onChange={(e) => setR(e.target.value)} />
          <div className="row">
            <span className="grow" />
            <button className="btn" disabled={!cur} onClick={() => { if (cur && editor.replace(q, r, { matchCase, wholeWord, only: cur })) toast(t('Replaced 1')); }}>{t('Replace')}</button>
            <button className="btn primary" disabled={!matches.length} onClick={() => toast(tn(editor.replace(q, r, { matchCase, wholeWord }), 'Replaced {n} match', 'Replaced {n} matches'))}>{t('Replace all')}</button>
          </div>
        </>
      )}
      {cur && <p className="faint" style={{ margin: 0, fontSize: 12 }}>…{cur.context}…</p>}
    </Modal>
  );
}
