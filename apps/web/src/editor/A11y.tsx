/**
 * Accessibility of the editor itself (A11Y-001..014):
 * - CanvasMirror: a screen-reader-only outline of every page and element (the canvas is pixels).
 *   Activating an entry selects the element and returns focus to the canvas, where arrows nudge,
 *   Tab cycles, Delete removes and Enter edits text.
 * - Announcer: a polite live region describing the selection and the result of each gesture.
 * - AccessibilityDrawer: the design checker (alt text, contrast, text size, link names, language).
 */
import { auditDesign, describeElement, type A11yIssue, type ElementKindLabel } from '@oc/engine';
import { childrenOf, sortedPages, type DesignDoc, type Element, type ElementId, type PageId } from '@oc/engine';
import { AlertTriangle, CheckCircle2, CircleAlert, X } from 'lucide-react';
import { useEffect, useMemo, useRef, useState } from 'react';
import { t, tk, tn } from '../i18n';
import { useDoc, useEd, useEditorState } from './context';
import { TextSemantics } from './TextSemantics';

const KIND_LABELS: Record<ElementKindLabel, string> = {
  Text: tk('Text'), Image: tk('Image'), Video: tk('Video'), Shape: tk('Shape'), Line: tk('Line'), Group: tk('Group'), Frame: tk('Frame'),
  Table: tk('Table'), Chart: tk('Chart'), 'Photo grid': tk('Photo grid'), Form: tk('Form'), 'QR code': tk('QR code'), Drawing: tk('Drawing'), Embed: tk('Embed'), Element: tk('Element'),
};

export function describe(doc: DesignDoc, el: Element, withGeometry = false): string {
  const d = describeElement(doc, el);
  const kind = t(KIND_LABELS[d.kind]);
  let s = d.label ? `${kind}: ${d.label}` : el.decorative ? t('{kind} (decorative)', { kind }) : kind;
  if (d.locked) s += `, ${t('locked')}`;
  if (withGeometry) s += `, ${t('{w} by {h} at {x}, {y}', { w: d.w, h: d.h, x: d.x, y: d.y })}${d.rot ? `, ${t('rotated {deg}°', { deg: d.rot })}` : ''}`;
  return s;
}

// ---------------------------------------------------------------- announcer
let push: ((msg: string) => void) | null = null;
/** Speak a short message to screen-reader users (polite). */
export function announce(msg: string) {
  push?.(msg);
}

export function Announcer() {
  const [msg, setMsg] = useState('');
  useEffect(() => {
    push = (m) => {
      // Re-announce identical text by clearing first.
      setMsg('');
      requestAnimationFrame(() => setMsg(m));
    };
    return () => { push = null; };
  }, []);
  return <div className="sr-only" role="status" aria-live="polite" aria-atomic="true">{msg}</div>;
}

/** Describes selection changes and finished gestures. */
export function useSelectionAnnouncements() {
  const { editor } = useEd();
  const st = useEditorState();
  const prev = useRef<{ sel: string; gesturing: boolean }>({ sel: '', gesturing: false });
  useEffect(() => {
    const doc = editor.doc;
    const sel = st.selection.filter((id) => doc.elements[id] && !doc.elements[id].deleted);
    const key = sel.join(',');
    const p = prev.current;
    if (p.gesturing && !st.gesturing && sel.length) {
      announce(sel.length === 1 ? describe(doc, doc.elements[sel[0]!]!, true) : tn(sel.length, '{n} element changed', '{n} elements changed'));
    } else if (key !== p.sel && !st.gesturing) {
      if (!sel.length) {
        const gone = p.sel.split(',').filter(Boolean).some((id) => !doc.elements[id as ElementId] || doc.elements[id as ElementId]!.deleted);
        announce(gone ? t('Deleted') : t('Nothing selected'));
      } else announce(sel.length === 1 ? `${t('Selected')} ${describe(doc, doc.elements[sel[0]!]!, true)}` : tn(sel.length, '{n} element selected', '{n} elements selected'));
    }
    prev.current = { sel: key, gesturing: st.gesturing };
  }, [st.selection, st.gesturing, editor]);
}

// ---------------------------------------------------------------- canvas mirror
export function CanvasMirror() {
  const { editor } = useEd();
  const doc = useDoc();
  const st = useEditorState();
  useSelectionAnnouncements();
  const focusCanvas = () => (document.querySelector('.canvas-wrap') as HTMLElement | null)?.focus({ preventScroll: true });
  const tree = (parent: ElementId | PageId, depth: number): React.ReactNode => {
    const kids = childrenOf(doc, parent).filter((e) => !e.deleted && e.visible !== false);
    if (!kids.length) return null;
    return (
      <ul>
        {kids.map((el) => (
          <li key={el.id}>
            <button type="button" aria-pressed={st.selection.includes(el.id)} onClick={() => {
              editor.setActivePage(pageOf(doc, el), true);
              editor.select([el.id]);
              focusCanvas();
            }}>{describe(doc, el)}</button>
            {(el.type === 'group' || el.type === 'frame') && depth < 6 && tree(el.id, depth + 1)}
          </li>
        ))}
      </ul>
    );
  };
  const pages = sortedPages(doc);
  return (
    <nav className="sr-only sr-focusable" aria-label={t('Design outline')}>
      <p>{t('The canvas is visual. Use this outline to find and select elements; then use arrow keys to move them, Tab to go to the next element, Enter to edit text and Delete to remove.')}</p>
      {pages.map((p, i) => (
        <section key={p.id} aria-label={t('Page {n}', { n: i + 1 })}>
          <h3>{p.title ? t('Page {n}: {title}', { n: i + 1, title: p.title }) : t('Page {n}', { n: i + 1 })}{p.hidden ? ` (${t('hidden')})` : ''}</h3>
          {tree(p.id, 0) ?? <p>{t('Empty page')}</p>}
        </section>
      ))}
    </nav>
  );
}

function pageOf(doc: DesignDoc, el: Element): PageId {
  let cur: Element | undefined = el;
  for (let i = 0; i < 32 && cur; i++) {
    if (doc.pages[cur.parentId as PageId]) return cur.parentId as PageId;
    cur = doc.elements[cur.parentId as ElementId];
  }
  return sortedPages(doc)[0]!.id;
}

// ---------------------------------------------------------------- checker
const MESSAGES: Record<A11yIssue['code'], { title: string; help: string }> = {
  'alt-missing': { title: tk('Missing alternative text'), help: tk('Describe the image for people who can’t see it, or mark it as decorative.') },
  'low-contrast': { title: tk('Low text contrast'), help: tk('Contrast is {ratio}:1; at least {needed}:1 is needed. Darken the text or lighten the background (or the reverse).') },
  'small-text': { title: tk('Text may be too small'), help: tk('{size} px is hard to read at this design size; use at least {min} px.') },
  'link-name': { title: tk('Link without a name'), help: tk('Add alternative text that says where the link goes.') },
  'no-language': { title: tk('Design language not set'), help: tk('Screen readers use it to pronounce your text correctly.') },
  'empty-text': { title: tk('Empty text box'), help: tk('Delete it or add text; screen readers announce empty boxes.') },
};

export function AccessibilityDrawer({ onClose }: { onClose: () => void }) {
  const { editor, readOnly } = useEd();
  const doc = useDoc();
  const issues = useMemo(() => auditDesign(doc), [doc]);
  const [alt, setAlt] = useState<Record<string, string>>({});
  const [tab, setTab] = useState<'check' | 'semantics'>('check');
  const errors = issues.filter((i) => i.severity === 'error').length;
  const go = (i: A11yIssue) => {
    if (!i.id || !i.pageId) return;
    editor.setActivePage(i.pageId, true);
    editor.select([i.id]);
  };
  return (
    <aside className="drawer" aria-label={t('Accessibility')}>
      <div className="drawer-head">
        <h3>{t('Accessibility')}</h3>
        <button className="btn ghost sm icon" onClick={onClose} aria-label={t('Close')}><X size={16} /></button>
      </div>
      <div className="drawer-body a11y-list">
        <div className="lib-tabs" role="tablist" aria-label={t('Accessibility')}>
          <button role="tab" id="a11y-tab-check" aria-selected={tab === 'check'} aria-controls="a11y-panel" className={tab === 'check' ? 'on' : ''} onClick={() => setTab('check')}>{t('Check')}</button>
          <button role="tab" id="a11y-tab-semantics" aria-selected={tab === 'semantics'} aria-controls="a11y-panel" className={tab === 'semantics' ? 'on' : ''} onClick={() => setTab('semantics')}>{t('Text semantics')}</button>
        </div>
        <div id="a11y-panel" role="tabpanel" aria-labelledby={`a11y-tab-${tab}`}>
        {tab === 'semantics' ? <TextSemantics /> : <>
        {!issues.length ? (
          <p className="row" style={{ gap: 8 }}><CheckCircle2 size={18} color="var(--ok)" aria-hidden /> {t('No issues found. Nice work!')}</p>
        ) : (
          <p className="muted">{tn(issues.length, '{n} issue', '{n} issues')}{errors ? ` · ${tn(errors, '{n} to fix', '{n} to fix')}` : ''}</p>
        )}
        <ul className="plain">
          {issues.map((i, k) => {
            const m = MESSAGES[i.code];
            const el = i.id ? doc.elements[i.id] : undefined;
            return (
              <li key={`${i.code}:${i.id ?? k}`} className={`a11y-issue ${i.severity}`}>
                <div className="row" style={{ gap: 8, alignItems: 'flex-start' }}>
                  {i.severity === 'error' ? <CircleAlert size={16} color="var(--danger)" aria-label={t('Error')} /> : <AlertTriangle size={16} color="#d97706" aria-label={t('Warning')} />}
                  <div className="grow">
                    <strong>{t(m.title)}</strong>
                    {el && <button className="linkish" onClick={() => go(i)}>{describe(doc, el)}</button>}
                    <div className="muted">{t(m.help, i.data)}</div>
                    {i.code === 'alt-missing' && el && !readOnly && (
                      <form className="row" style={{ marginTop: 6 }} onSubmit={(e) => {
                        e.preventDefault();
                        editor.select([el.id]);
                        editor.updateSelected({ altText: (alt[el.id] ?? '').trim(), decorative: false });
                      }}>
                        <input className="input grow" value={alt[el.id] ?? ''} placeholder={t('Describe the image')} aria-label={t('Alternative text')} onChange={(e) => setAlt({ ...alt, [el.id]: e.target.value })} />
                        <button className="btn sm" disabled={!(alt[el.id] ?? '').trim()}>{t('Save')}</button>
                        <button type="button" className="btn ghost sm" onClick={() => { editor.select([el.id]); editor.updateSelected({ decorative: true }); }}>{t('Decorative')}</button>
                      </form>
                    )}
                    {i.code === 'no-language' && !readOnly && (
                      <button className="btn sm" style={{ marginTop: 6 }} onClick={() => editor.setLanguage(document.documentElement.lang || 'en')}>{t('Use {lang}', { lang: new Intl.DisplayNames([document.documentElement.lang || 'en'], { type: 'language' }).of(document.documentElement.lang || 'en') ?? 'English' })}</button>
                    )}
                  </div>
                </div>
              </li>
            );
          })}
        </ul>
        </>}
        </div>
      </div>
    </aside>
  );
}
