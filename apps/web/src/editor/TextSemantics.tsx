/**
 * Edit text semantics (plan 11 g04, C7 AX-100): every text box of the design in reading order, with what it is for a
 * screen reader — heading level 1 or 2, paragraph, quote, caption and so on (the element `role`). Exports and the
 * screen-reader outline can map the roles to heading levels with `headingLevel` (lib/contentRules.ts).
 */
import { bodyPlainText, sortedPages, type Element, type PageId, type SemanticRole, type TextEl } from '@oc/engine';
import { useMemo } from 'react';
import { setAppearance, useAppearance } from '../lib/appearance';
import { readingOrder } from '../lib/contentRules';
import { t, tk } from '../i18n';
import { useDoc, useEd } from './context';

/** Text roles offered in the list ('' = decided automatically from size). */
const ROLES: Array<{ id: SemanticRole | ''; label: string }> = [
  { id: '', label: tk('Automatic') }, { id: 'headline', label: tk('Heading 1') }, { id: 'subhead', label: tk('Heading 2') }, { id: 'body', label: tk('Paragraph') },
  { id: 'quote', label: tk('Quote') }, { id: 'caption', label: tk('Caption') }, { id: 'cta', label: tk('Button or call to action') }, { id: 'price', label: tk('Price') },
  { id: 'legal', label: tk('Fine print') }, { id: 'decor', label: tk('Decoration (skipped)') },
];

/** Text boxes per page in reading order (top to bottom, then left to right; rows within half a line count as one). */
export function textsInReadingOrder(doc: { pages: Record<string, { id: PageId; order: string }>; elements: Record<string, Element> }): Array<{ pageId: PageId; els: TextEl[] }> {
  const pageIds = new Set(Object.keys(doc.pages));
  const pageOf = (el: Element): PageId | null => {
    let cur: Element | undefined = el;
    for (let i = 0; cur && i < 64; i++) {
      if (pageIds.has(cur.parentId)) return cur.parentId as PageId;
      cur = doc.elements[cur.parentId];
    }
    return null;
  };
  const by = new Map<PageId, TextEl[]>();
  for (const el of Object.values(doc.elements)) {
    if (el.deleted || el.type !== 'text' || el.visible === false) continue;
    const p = pageOf(el);
    if (p) by.set(p, [...(by.get(p) ?? []), el]);
  }
  return sortedPages(doc as never).filter((p) => by.has(p.id)).map((p) => ({
    pageId: p.id,
    els: by.get(p.id)!.sort((a, b) => readingOrder(a.xf, b.xf)),
  }));
}

export function TextSemantics() {
  const { editor, readOnly } = useEd();
  const doc = useDoc();
  const pages = useMemo(() => textsInReadingOrder(doc), [doc]);
  const appearance = useAppearance();
  const order = sortedPages(doc).map((p) => p.id);
  const setRole = (el: TextEl, role: SemanticRole | '') => {
    editor.apply([{ op: 'update', id: el.id, patch: { role: role || null, ...(role === 'decor' ? { decorative: true } : el.decorative ? { decorative: null } : {}) } }]);
  };
  return (
    <div className="col" style={{ gap: 10 }}>
      <p className="muted" style={{ margin: 0 }}>{t('Tell screen readers what each text is. Headings let people jump through the design; decoration is skipped.')}</p>
      {!pages.length && <p className="faint">{t('This design has no text yet.')}</p>}
      {pages.map(({ pageId, els }) => (
        <section key={pageId} aria-label={t('Page {n}', { n: order.indexOf(pageId) + 1 })}>
          <h4 style={{ margin: '4px 0' }}>{t('Page {n}', { n: order.indexOf(pageId) + 1 })}</h4>
          <ul className="plain sem-list">
            {els.map((el) => {
              const text = bodyPlainText(el.body).trim().replace(/\s+/g, ' ');
              return (
                <li key={el.id} style={{ display: 'grid', gridTemplateColumns: 'minmax(0, 1fr) 150px', gap: 8, alignItems: 'center', marginBottom: 6 }}>
                  <button className="linkish sem-text" style={{ textAlign: 'start', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }} onClick={() => { editor.setActivePage(pageId, true); editor.select([el.id]); }} title={text}>
                    {text.slice(0, 60) || t('(empty)')}
                  </button>
                  <select className="input sm" aria-label={t('Role of “{text}”', { text: text.slice(0, 40) || t('(empty)') })} disabled={readOnly} value={el.decorative ? 'decor' : (el.role ?? '')}
                    onChange={(e) => setRole(el, e.target.value as SemanticRole | '')}>
                    {ROLES.map((r) => <option key={r.id || 'auto'} value={r.id}>{t(r.label)}</option>)}
                  </select>
                </li>
              );
            })}
          </ul>
        </section>
      ))}
      <label className="check" style={{ marginTop: 6 }}>
        <input type="checkbox" checked={appearance.motion === 'reduce'} onChange={(e) => setAppearance({ motion: e.target.checked ? 'reduce' : 'system' })} />
        {t('Reduce motion in the editor')}
      </label>
    </div>
  );
}
