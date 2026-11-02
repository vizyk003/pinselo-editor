/** Print preflight results with one-click fixes (A18). Used by Download → PDF Print. */
import type { ColorValue, ElementId, PageId, PreflightFix, PreflightIssue, PreflightResult } from '@oc/engine';
import { preflightPrint, printColorKey, printColors, sortedPages, standalonePrintTarget } from '@oc/engine';
import { AlertTriangle, CheckCircle2, Info, XCircle } from 'lucide-react';
import { useCallback, useEffect, useMemo, useState } from 'react';
import { t, tk, tn } from '../i18n';
import { useDoc, useEd } from '../editor/context';
import { printColorApi } from './printExport';

export function preflightMessage(i: PreflightIssue): string {
  const v = i.vars;
  switch (i.code) {
    case 'size_mismatch': return t('The design is {design} but the product is {product}. Resize the design to the product.', v);
    case 'size_scaled': return t('The design ({design}) will be scaled to the product size ({product}).', v);
    case 'page_count_over': return t('The design has {pages} pages but this product prints {expected}.', v);
    case 'page_count_under': return t('This product prints {expected} sides but the design has {pages}, so the back will be blank.', v);
    case 'low_dpi': return t('“{name}” prints at {dpi} dpi (below {min}) and will look blurry. Use a larger image or make it smaller.', v);
    case 'dpi_below_recommended': return t('“{name}” prints at {dpi} dpi; {good} dpi is recommended for sharp detail.', v);
    case 'text_outside_safe': return t('Text “{name}” is within {safe} mm of the edge and may be trimmed.', v);
    case 'bleed_missing': return t('“{name}” stops at the trim edge. Extend it {bleed} mm past the edge so no white line shows after cutting.', v);
    case 'out_of_gamut': return t('{color} is brighter than print inks can reproduce and will print duller.', v);
    case 'thin_line': return t('“{name}” has a {width} pt line; lines thinner than 0.25 pt may not print.', v);
    case 'font_missing': return t('The font “{font}” isn’t available for printing and will be replaced.', v);
    case 'not_printable': return t('“{name}” is interactive or moving and prints as a still image.', v);
    default: return i.message;
  }
}

const FIX_LABEL: Record<PreflightFix['kind'], string> = {
  'resize-design': tk('Resize design to product'),
  'extend-to-bleed': tk('Extend to bleed'),
  'thicken-line': tk('Make line thicker'),
  'add-page': tk('Add a back page'),
};

/** Applies preflight fixes through the editor (undoable like any other edit). */
export function usePreflightFixes() {
  const { editor, readOnly } = useEd();
  const apply = useCallback((fix: PreflightFix) => {
    const doc = editor.doc;
    switch (fix.kind) {
      case 'resize-design':
        editor.resizeDesign(fix.w, fix.h, fix.unit, true);
        editor.zoomToFit();
        return;
      case 'extend-to-bleed':
        editor.apply([{ op: 'xf', id: fix.elementId, xf: fix.xf }]);
        return;
      case 'thicken-line': {
        const el = doc.elements[fix.elementId];
        if (el?.type === 'line') editor.apply([{ op: 'update', id: el.id, patch: { stroke: { ...el.stroke, width: fix.width } } }]);
        else if (el?.type === 'shape') editor.apply([{ op: 'update', id: el.id, patch: { strokes: el.strokes.map((s) => ({ ...s, width: Math.max(s.width, fix.width) })) } }]);
        return;
      }
      case 'add-page': {
        const pages = sortedPages(doc);
        editor.addPage(pages.at(-1)?.id ?? null);
        return;
      }
    }
  }, [editor]);
  const show = useCallback((pageId?: PageId, elementId?: ElementId) => {
    if (pageId) editor.setActivePage(pageId, true);
    if (elementId) editor.select([elementId]);
  }, [editor]);
  return { apply, show, canFix: !readOnly };
}

const ICON = { error: <XCircle size={16} color="var(--danger)" />, warning: <AlertTriangle size={16} color="#d97706" />, info: <Info size={16} color="var(--text-3)" /> };

export function PreflightList({ result, fixes, compact }: { result: PreflightResult; fixes?: ReturnType<typeof usePreflightFixes>; compact?: boolean }) {
  const { issues, counts } = result;
  if (!issues.length) {
    return <div className="row" style={{ gap: 8, fontSize: 13 }}><CheckCircle2 size={16} color="#16a34a" /> {t('Ready to print — no problems found.')}</div>;
  }
  const order = { error: 0, warning: 1, info: 2 } as const;
  const sorted = [...issues].sort((a, b) => order[a.severity] - order[b.severity]);
  return (
    <div className="col" style={{ gap: 6 }}>
      <div className="faint" style={{ fontSize: 12 }}>
        {[counts.error ? tn(counts.error, '{n} problem to fix', '{n} problems to fix') : '', counts.warning ? tn(counts.warning, '{n} warning', '{n} warnings') : '', counts.info ? tn(counts.info, '{n} note', '{n} notes') : ''].filter(Boolean).join(' · ')}
      </div>
      <ul className="col" style={{ gap: 6, listStyle: 'none', margin: 0, padding: 0, maxHeight: compact ? 220 : 340, overflow: 'auto' }} aria-label={t('Print check results')}>
        {sorted.map((i, k) => (
          <li key={`${i.code}-${i.elementId ?? ''}-${k}`} className="row" style={{ alignItems: 'flex-start', gap: 8, fontSize: 13 }}>
            <span style={{ marginTop: 2 }} aria-label={i.severity === 'error' ? t('Problem') : i.severity === 'warning' ? t('Warning') : t('Note')}>{ICON[i.severity]}</span>
            <span className="grow">
              {preflightMessage(i)}
              {i.pageIndex !== undefined && result.issues.some((x) => x.pageIndex !== undefined && x.pageIndex > 0) && <span className="faint"> · {t('Page {n}', { n: i.pageIndex + 1 })}</span>}
            </span>
            {fixes && (
              <span className="row" style={{ gap: 4, flexShrink: 0 }}>
                {i.elementId && <button type="button" className="btn sm ghost" onClick={() => fixes.show(i.pageId, i.elementId)}>{t('Show')}</button>}
                {i.fix && fixes.canFix && <button type="button" className="btn sm" onClick={() => fixes.apply(i.fix!)}>{t(FIX_LABEL[i.fix.kind])}</button>}
              </span>
            )}
          </li>
        ))}
      </ul>
    </div>
  );
}

/**
 * Download → PDF Print: check the design at its own size with the chosen bleed (5 mm safe zone). Quiet: only real
 * problems (errors, warnings, and out-of-gamut colours when printing CMYK) are listed. With a CMYK `gamut` profile
 * the out-of-gamut check is colour-managed (POST /api/print/gamut); the heuristic is the fallback.
 */
export function StandalonePreflight({ bleedMm = 3, gamut = null }: { bleedMm?: number; gamut?: { profile: string | { id: string }; intent: 'relative' | 'perceptual' } | null } = {}) {
  const { engine } = useEd();
  const doc = useDoc();
  const fixes = usePreflightFixes();
  const managedOn = Boolean(gamut);
  const colors = useMemo(() => (managedOn ? printColors(doc) : []), [doc, managedOn]);
  const colorsKey = useMemo(() => colors.map(printColorKey).join(';'), [colors]);
  const gamutKey = gamut ? JSON.stringify(gamut) : '';
  const [managed, setManaged] = useState<{ key: string; out: Map<string, boolean> } | null>(null);
  useEffect(() => {
    if (!gamut || !colors.length) return;
    const ac = new AbortController();
    const timer = setTimeout(() => {
      printColorApi.gamut({ profile: gamut.profile, intent: gamut.intent, colors }, ac.signal)
        .then((r) => {
          if (r.results?.length !== colors.length) return;
          setManaged({ key: gamutKey + colorsKey, out: new Map(colors.map((c, i) => [printColorKey(c), !r.results[i]!.inGamut])) });
        })
        .catch(() => { /* fall back to the heuristic */ });
    }, 400);
    return () => { clearTimeout(timer); ac.abort(); };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [gamutKey, colorsKey]);
  const lookup = managed && managed.key === gamutKey + colorsKey ? managed.out : null;
  const result = useMemo(() => {
    const target = { ...standalonePrintTarget(doc), bleedMm };
    const full = preflightPrint(doc, target, { availableFonts: engine.fonts, ...(lookup ? { outOfGamut: (c: ColorValue) => lookup.get(printColorKey(c)) } : {}) });
    // Quiet: hide notes, except colours that will print duller when the file is CMYK.
    const issues = full.issues.filter((i) => i.severity !== 'info' || (managedOn && i.code === 'out_of_gamut'));
    const counts = { error: 0, warning: 0, info: 0 };
    for (const i of issues) counts[i.severity]++;
    return { ...full, issues, counts };
  }, [doc, engine, bleedMm, lookup, managedOn]);
  if (!result.issues.length) {
    return <div className="row faint" style={{ gap: 6, fontSize: 12 }}><CheckCircle2 size={14} color="#16a34a" /> {t('Print check: no problems found')}</div>;
  }
  return (
    <details className="credits" open={result.counts.error + result.counts.warning > 0}>
      <summary>{t('Print check')} ({result.issues.length})</summary>
      <PreflightList result={result} fixes={fixes} compact />
    </details>
  );
}
