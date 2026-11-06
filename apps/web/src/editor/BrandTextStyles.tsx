/**
 * A brand kit's text styles in the Brand panel: each adds text in that style (the role's font, weight, italic,
 * capitals, letter spacing, line height, size, first colour, and the role itself); editors add, change and remove
 * them, or save the selected text box as a style. Assistants get the same styles through the AI connector, which
 * flags their text when it strays.
 */
import { hex, solid, toHex, type TextEl } from '@oc/engine';
import { Pencil, Plus, Trash2, Type } from 'lucide-react';
import { useState } from 'react';
import type { Kit, KitFontRole, KitInput, KitTextStyle } from '../api';
import { Modal, useToast } from '../components/ui';
import { t, tk } from '../i18n';
import { previewFamily } from '../lib/cssFonts';
import { useEd } from './context';
import { addText } from './insert';
import { cleanStyle, describeStyle, elementRole, MAX_TEXT_STYLES, runStyleOf, styleFromText, WEIGHTS } from './kitTextStyles';

export const ROLE_LABELS: Array<[KitFontRole, string]> = [
  ['heading', tk('Heading')], ['subheading', tk('Subheading')], ['body', tk('Body text')], ['quote', tk('Quote')], ['caption', tk('Caption')],
];

export function BrandTextStyles({ kit, canEdit, save }: { kit: Kit; canEdit: boolean; save: (patch: Partial<KitInput>) => Promise<void> }) {
  const { editor, engine } = useEd();
  const toast = useToast();
  const styles = kit.textStyles ?? [];
  const [editing, setEditing] = useState<{ index: number | null; style: KitTextStyle } | null>(null);
  const words = { italic: t('italic'), caps: t('capitals') };

  const insert = (st: KitTextStyle) => {
    const run = runStyleOf(kit, st, (h) => solid(hex(h)));
    addText(editor, st.name, run, { role: elementRole(st.role), lineHeight: st.lineHeight, align: 'start' });
  };
  const fromSelection = () => {
    const el = editor.selectedDeep().find((e): e is TextEl => e.type === 'text');
    if (!el) return toast(t('Select a text box first.'));
    setEditing({ index: null, style: styleFromText(el, t('Style {n}', { n: styles.length + 1 }), 'body', (c) => toHex(c as Parameters<typeof toHex>[0])) });
  };
  const store = async (next: KitTextStyle[]) => save({ textStyles: next.map(cleanStyle) });

  return (
    <>
      <h4><Type size={13} /> {t('Text styles')}</h4>
      <p className="faint" style={{ margin: 0, fontSize: 12 }}>{t('How each kind of text looks beyond its font. Click a style to add text in it; AI assistants follow these styles and are told when their text strays.')}</p>
      {!styles.length && <p className="muted" style={{ margin: 0, fontSize: 13 }}>{t('No text styles yet.')}</p>}
      {styles.map((st, i) => {
        const fam = kit.fonts[st.role];
        const f = fam ? engine.fonts.families().find((x) => x.family === fam) : undefined;
        return (
          <div key={`${st.name}-${i}`} className="row" style={{ gap: 4 }}>
            <button className="btn grow" title={t('Add {label}', { label: st.name })} onClick={() => insert(st)}
              style={{ justifyContent: 'flex-start', flexDirection: 'column', alignItems: 'flex-start', gap: 0, height: 'auto', padding: '6px 10px' }}>
              <span style={{
                fontFamily: f ? previewFamily(f, true) : undefined, fontWeight: st.weight, fontStyle: st.italic ? 'italic' : undefined,
                textTransform: st.uppercase ? 'uppercase' : undefined, letterSpacing: st.letterSpacing ? `${st.letterSpacing / 1000}em` : undefined,
                fontSize: Math.max(12, Math.min(20, (st.size ?? 20) / 2)), color: st.colors?.[0],
              }}>{st.name}</span>
              <span className="faint" style={{ fontSize: 11 }}>{t(ROLE_LABELS.find(([r]) => r === st.role)?.[1] ?? '')}{fam ? ` · ${fam}` : ''}{describeStyle(st, words) ? ` · ${describeStyle(st, words)}` : ''}</span>
            </button>
            {canEdit && (
              <>
                <button className="btn ghost sm icon" aria-label={t('Edit {name}', { name: st.name })} title={t('Edit {name}', { name: st.name })} onClick={() => setEditing({ index: i, style: st })}><Pencil size={13} /></button>
                <button className="btn ghost sm icon danger" aria-label={t('Remove {name}', { name: st.name })} title={t('Remove {name}', { name: st.name })} onClick={() => void store(styles.filter((_, j) => j !== i))}><Trash2 size={13} /></button>
              </>
            )}
          </div>
        );
      })}
      {canEdit && styles.length < MAX_TEXT_STYLES && (
        <div className="row" style={{ gap: 6, flexWrap: 'wrap' }}>
          <button className="btn sm" onClick={() => setEditing({ index: null, style: { name: t('Style {n}', { n: styles.length + 1 }), role: 'body' } })}><Plus size={14} /> {t('Add a text style')}</button>
          <button className="btn sm" onClick={fromSelection}>{t('From selected text')}</button>
        </div>
      )}
      {editing && (
        <StyleDialog kit={kit} initial={editing.style} onClose={() => setEditing(null)} onSave={async (st) => {
          const next = editing.index === null ? [...styles, st] : styles.map((s, j) => (j === editing.index ? st : s));
          await store(next);
          setEditing(null);
        }} />
      )}
    </>
  );
}

function StyleDialog({ kit, initial, onClose, onSave }: { kit: Kit; initial: KitTextStyle; onClose: () => void; onSave: (st: KitTextStyle) => Promise<void> }) {
  const [st, setSt] = useState<KitTextStyle>(initial);
  const [busy, setBusy] = useState(false);
  const set = (patch: Partial<KitTextStyle>) => setSt((s) => ({ ...s, ...patch }));
  const num = (v: string) => (v.trim() === '' ? undefined : Number(v));
  const colors = st.colors ?? [];
  const kitColors = [...new Map([...kit.colors, ...kit.palettes.flatMap((p) => p.colors)].map((c) => [c.hex.toLowerCase(), c])).values()];
  const extra = colors.filter((c) => !kitColors.some((k) => k.hex.toLowerCase() === c));
  return (
    <Modal title={t('Text style')} onClose={onClose}>
      <div className="col" style={{ gap: 10 }}>
        <label className="col" style={{ gap: 4 }}>{t('Name')}
          <input className="input" value={st.name} maxLength={40} autoFocus onChange={(e) => set({ name: e.target.value })} placeholder={t('e.g. Eyebrow, Lead, Pull quote')} />
        </label>
        <label className="col" style={{ gap: 4 }}>{t('Kind of text')}
          <select className="input" value={st.role} onChange={(e) => set({ role: e.target.value as KitFontRole })}>
            {ROLE_LABELS.map(([r, label]) => <option key={r} value={r}>{t(label)}{kit.fonts[r] ? ` (${kit.fonts[r]})` : ''}</option>)}
          </select>
        </label>
        <div className="row" style={{ gap: 10, flexWrap: 'wrap' }}>
          <label className="col" style={{ gap: 4 }}>{t('Weight')}
            <select className="input" value={st.weight ?? ''} onChange={(e) => set({ weight: num(e.target.value) })}>
              <option value="">{t('Any')}</option>
              {WEIGHTS.map((w) => <option key={w} value={w}>{w}</option>)}
            </select>
          </label>
          <label className="col" style={{ gap: 4 }}>{t('Size (px)')}
            <input className="input" type="number" min={4} max={1000} step={0.5} style={{ width: 90 }} value={st.size ?? ''} onChange={(e) => set({ size: num(e.target.value) })} />
          </label>
          <label className="col" style={{ gap: 4 }}>{t('Letter spacing (‰ em)')}
            <input className="input" type="number" min={-500} max={2000} step={5} style={{ width: 110 }} value={st.letterSpacing ?? ''} onChange={(e) => set({ letterSpacing: num(e.target.value) })} />
          </label>
          <label className="col" style={{ gap: 4 }}>{t('Line height')}
            <input className="input" type="number" min={0.5} max={4} step={0.05} style={{ width: 90 }} value={st.lineHeight ?? ''} onChange={(e) => set({ lineHeight: num(e.target.value) })} />
          </label>
        </div>
        <div className="row" style={{ gap: 16 }}>
          <label className="check"><input type="checkbox" checked={Boolean(st.italic)} onChange={(e) => set({ italic: e.target.checked })} /> {t('Italic')}</label>
          <label className="check"><input type="checkbox" checked={Boolean(st.uppercase)} onChange={(e) => set({ uppercase: e.target.checked })} /> {t('All capitals')}</label>
        </div>
        <div className="col" style={{ gap: 4 }}>
          <span>{t('Allowed colours')} <span className="faint" style={{ fontSize: 12 }}>{t('(none chosen: any kit colour)')}</span></span>
          <div className="row" style={{ gap: 6, flexWrap: 'wrap' }}>
            {kitColors.map((c) => {
              const h = c.hex.toLowerCase();
              const on = colors.includes(h);
              return (
                <button key={h} type="button" aria-pressed={on} title={`${c.name} ${c.hex}`} aria-label={c.name}
                  onClick={() => set({ colors: on ? colors.filter((x) => x !== h) : [...colors, h] })}
                  style={{ width: 28, height: 28, borderRadius: 8, background: c.hex, cursor: 'pointer', border: on ? '3px solid var(--accent)' : '1px solid var(--border)', outline: on ? '1px solid var(--bg)' : undefined, outlineOffset: -4 }} />
              );
            })}
            {extra.map((h) => (
              <button key={h} type="button" aria-pressed title={h} aria-label={h} onClick={() => set({ colors: colors.filter((x) => x !== h) })}
                style={{ width: 28, height: 28, borderRadius: 8, background: h, cursor: 'pointer', border: '3px solid var(--accent)' }} />
            ))}
          </div>
        </div>
        <label className="col" style={{ gap: 4 }}>{t('Note (optional)')}
          <input className="input" value={st.note ?? ''} maxLength={300} onChange={(e) => set({ note: e.target.value })} placeholder={t('e.g. Above headings, never longer than one line')} />
        </label>
        <div className="row" style={{ justifyContent: 'flex-end' }}>
          <button className="btn" onClick={onClose}>{t('Cancel')}</button>
          <button className="btn primary" disabled={busy || !st.name.trim()} onClick={async () => { setBusy(true); try { await onSave(st); } finally { setBusy(false); } }}>{t('Save')}</button>
        </div>
      </div>
    </Modal>
  );
}
