/**
 * A brand kit's written guidelines (its brand book or design system documentation): the Brand panel's summary and
 * the dialog that edits them or imports them from Markdown files. The AI connector hands them to assistants
 * (get_brand_guidelines) before they design with the kit.
 */
import { BRAND } from '@oc/config';
import { BookOpen, ChevronDown, ChevronRight, FileUp, Plus, Trash2 } from 'lucide-react';
import { useEffect, useRef, useState } from 'react';
import { ApiError, brandHub, type GuidelineSection } from '../api';
import { Modal, useToast } from '../components/ui';
import { fmtNumber, t, tn } from '../i18n';
import { GUIDELINE_LIMITS, mergeSections, splitMarkdown, totalChars } from './guidelinesMarkdown';

export function BrandGuidelines({ kitId, canEdit }: { kitId: string; canEdit: boolean }) {
  const [sections, setSections] = useState<GuidelineSection[] | null>(null);
  const [open, setOpen] = useState(false);
  useEffect(() => {
    setSections(null);
    let live = true;
    brandHub.guidelines(kitId).then((r) => live && setSections(r.sections)).catch(() => live && setSections([]));
    return () => { live = false; };
  }, [kitId]);

  return (
    <>
      <h4><BookOpen size={13} /> {t('Brand guidelines')}</h4>
      <p className="faint" style={{ margin: 0, fontSize: 12 }}>{t('Your brand book or design system documentation. AI assistants using the {brand} connector read it before designing with this kit.', { brand: BRAND.name })}</p>
      {sections === null ? null : sections.length ? (
        <p className="muted" style={{ margin: 0, fontSize: 13 }}>
          {tn(sections.length, '{n} section', '{n} sections')}: {sections.map((s) => s.title).join(' · ')}
        </p>
      ) : (
        <p className="muted" style={{ margin: 0, fontSize: 13 }}>{t('No guidelines yet.')}</p>
      )}
      {sections !== null && (canEdit || sections.length > 0) && (
        <div className="row">
          <button className="btn sm" onClick={() => setOpen(true)}>{canEdit ? t('Edit guidelines') : t('Read guidelines')}</button>
        </div>
      )}
      {open && sections && (
        <GuidelinesDialog kitId={kitId} initial={sections} canEdit={canEdit} onClose={() => setOpen(false)} onSaved={(s) => { setSections(s); setOpen(false); }} />
      )}
    </>
  );
}

function GuidelinesDialog({ kitId, initial, canEdit, onClose, onSaved }: {
  kitId: string; initial: GuidelineSection[]; canEdit: boolean; onClose: () => void; onSaved: (s: GuidelineSection[]) => void;
}) {
  const toast = useToast();
  const [sections, setSections] = useState(initial);
  const [expanded, setExpanded] = useState<number | null>(initial.length === 1 ? 0 : null);
  const [busy, setBusy] = useState(false);
  const fileRef = useRef<HTMLInputElement>(null);
  const total = totalChars(sections);
  const tooMany = sections.length > GUIDELINE_LIMITS.sections;
  const tooLong = total > GUIDELINE_LIMITS.total;
  const untitled = sections.some((s) => !s.title.trim());

  const update = (i: number, patch: Partial<GuidelineSection>) => setSections((all) => all.map((s, j) => (j === i ? { ...s, ...patch } : s)));
  const importFiles = async (files: FileList | null) => {
    if (!files?.length) return;
    let incoming: GuidelineSection[] = [];
    for (const f of [...files]) incoming = incoming.concat(splitMarkdown(f.name, await f.text()));
    setSections((all) => mergeSections(all, incoming));
    toast(tn(incoming.length, 'Imported {n} section', 'Imported {n} sections'));
  };
  const save = async () => {
    setBusy(true);
    try {
      const clean = sections.map((s) => ({ title: s.title.trim(), body: s.body }));
      await brandHub.setGuidelines(kitId, clean);
      toast(t('Guidelines saved'));
      onSaved(clean);
    } catch (e) {
      toast(e instanceof ApiError ? e.message : t('That didn’t work.'));
      setBusy(false);
    }
  };

  return (
    <Modal title={t('Brand guidelines')} onClose={onClose} size="xl">
      <div className="col" style={{ gap: 12 }}>
        {canEdit && (
          <>
            <p className="muted" style={{ margin: 0, fontSize: 13 }}>
              {t('Import Markdown files (a design system README, brand book pages): each “##” heading becomes a section, and a section with the same title is replaced. You can also write sections here.')}
            </p>
            <div className="row" style={{ gap: 8, flexWrap: 'wrap' }}>
              <button className="btn sm" onClick={() => fileRef.current?.click()}><FileUp size={14} /> {t('Import Markdown')}</button>
              <input ref={fileRef} type="file" accept=".md,.markdown,.txt,text/markdown,text/plain" multiple hidden onChange={(e) => { void importFiles(e.target.files); e.target.value = ''; }} />
              <button className="btn sm" onClick={() => { setSections((all) => [...all, { title: '', body: '' }]); setExpanded(sections.length); }}><Plus size={14} /> {t('Add section')}</button>
              <span className="grow" />
              <span className={tooLong ? 'form-error' : 'faint'} style={{ fontSize: 12, alignSelf: 'center' }}>
                {t('{used} of {max} characters', { used: fmtNumber(total), max: fmtNumber(GUIDELINE_LIMITS.total) })}
              </span>
            </div>
          </>
        )}
        {!sections.length && <div className="faint" style={{ fontSize: 13 }}>{t('No guidelines yet.')}</div>}
        <div className="col" style={{ gap: 0 }}>
          {sections.map((s, i) => {
            const isOpen = expanded === i;
            return (
              <div key={i} className="col" style={{ gap: 8, padding: '8px 0', borderTop: '1px solid var(--border)' }}>
                <div className="row" style={{ gap: 6 }}>
                  <button className="btn ghost sm icon" aria-expanded={isOpen} aria-label={isOpen ? t('Collapse') : t('Expand')} onClick={() => setExpanded(isOpen ? null : i)}>
                    {isOpen ? <ChevronDown size={14} /> : <ChevronRight size={14} />}
                  </button>
                  {canEdit ? (
                    <input className="input grow" value={s.title} maxLength={GUIDELINE_LIMITS.title} placeholder={t('Section title, e.g. Colour')} aria-label={t('Section title')} onChange={(e) => update(i, { title: e.target.value })} onFocus={() => setExpanded(i)} />
                  ) : (
                    <button className="btn ghost sm grow" style={{ justifyContent: 'flex-start' }} onClick={() => setExpanded(isOpen ? null : i)}><b>{s.title}</b></button>
                  )}
                  <span className="faint" style={{ fontSize: 12, whiteSpace: 'nowrap' }}>{fmtNumber([...s.body].length)}</span>
                  {canEdit && (
                    <button className="btn ghost sm icon danger" title={t('Remove section')} aria-label={s.title ? t('Remove {name}', { name: s.title }) : t('Remove section')} onClick={() => setSections((all) => all.filter((_, j) => j !== i))}><Trash2 size={14} /></button>
                  )}
                </div>
                {isOpen && (canEdit ? (
                  <textarea className="input" rows={14} value={s.body} maxLength={GUIDELINE_LIMITS.body} aria-label={t('Section text')} style={{ fontFamily: 'ui-monospace, monospace', fontSize: 12.5 }} onChange={(e) => update(i, { body: e.target.value })} />
                ) : (
                  <div style={{ whiteSpace: 'pre-wrap', fontSize: 13, lineHeight: 1.55, paddingLeft: 34 }}>{s.body}</div>
                ))}
              </div>
            );
          })}
        </div>
        {canEdit && (
          <>
            {tooMany && <div className="form-error">{t('Up to {n} sections. Merge or remove some.', { n: GUIDELINE_LIMITS.sections })}</div>}
            {untitled && <div className="form-error">{t('Give every section a title.')}</div>}
            <div className="row" style={{ justifyContent: 'flex-end' }}>
              <button className="btn" onClick={onClose}>{t('Cancel')}</button>
              <button className="btn primary" disabled={busy || tooMany || tooLong || untitled} onClick={() => void save()}>{t('Save')}</button>
            </div>
          </>
        )}
      </div>
    </Modal>
  );
}
