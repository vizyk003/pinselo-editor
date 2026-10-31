/** Live form on public views: real inputs placed exactly over the rendered form element. */
import { worldMatrix } from '@oc/engine';
import { formLayout, toCss, type DesignDoc, type FormEl } from '@oc/engine';
import { useState } from 'react';
import { t } from '../i18n';

export function FormOverlay({ doc, el, designId, scale }: { doc: DesignDoc; el: FormEl; designId: string; scale: number }) {
  const m = worldMatrix(doc, el.id);
  const { rows } = formLayout(el);
  const [values, setValues] = useState<Record<string, string | string[]>>({});
  const [problems, setProblems] = useState<Record<string, string>>({});
  const [done, setDone] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const st = el.style;
  const box = (r: { x: number; y: number; w: number; h: number }): React.CSSProperties => ({ position: 'absolute', left: r.x * scale, top: r.y * scale, width: r.w * scale, height: r.h * scale });
  const submit = async (e: React.FormEvent) => {
    e.preventDefault();
    setBusy(true);
    const r = await fetch(`/api/forms/${designId}/${el.id}`, { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify(values) });
    const j = (await r.json().catch(() => ({}))) as { message?: string; problems?: Record<string, string> };
    setBusy(false);
    if (r.ok) setDone(j.message ?? el.thanks);
    else setProblems(j.problems ?? {});
  };
  return (
    <form onSubmit={submit} aria-label={el.title || t('Form')} style={{ position: 'absolute', left: m[4] * scale, top: m[5] * scale, width: el.xf.w * scale, height: el.xf.h * scale, fontFamily: st.family }}>
      {done ? (
        <div role="status" style={{ position: 'absolute', inset: 0, display: 'grid', placeItems: 'center', background: 'rgba(255,255,255,.94)', color: toCss(st.color), fontSize: st.size * scale, fontWeight: 600, textAlign: 'center', padding: 16, borderRadius: st.radius * scale }}>{done}</div>
      ) : rows.map((r, i) => {
        const field = el.fields.find((f) => f.id === r.fieldId);
        if (r.kind === 'input' && field) {
          const common = { 'aria-label': field.label, required: field.required, value: (values[field.id] as string) ?? '', onChange: (ev: React.ChangeEvent<HTMLInputElement | HTMLTextAreaElement>) => setValues({ ...values, [field.id]: ev.target.value }) };
          const style: React.CSSProperties = { ...box(r), background: 'transparent', border: problems[field.id] ? `2px solid #dc2626` : 0, outline: 'none', padding: `0 ${st.size * 0.6 * scale}px`, fontSize: st.size * 0.85 * scale, color: toCss(st.color), borderRadius: st.radius * scale, boxSizing: 'border-box', fontFamily: 'inherit' };
          return field.kind === 'longtext'
            ? <textarea key={i} {...common} style={{ ...style, padding: `${st.size * 0.5 * scale}px ${st.size * 0.6 * scale}px`, resize: 'none' }} />
            : <input key={i} {...common} type={field.kind === 'email' ? 'email' : field.kind === 'number' ? 'number' : field.kind === 'date' ? 'date' : 'text'} style={{ ...style, ...(values[field.id] ? {} : { color: 'transparent' }) }} onFocus={(e) => (e.currentTarget.style.color = toCss(st.color))} />;
        }
        if (r.kind === 'option' && field) {
          const multi = field.kind === 'checkboxes';
          const cur = values[field.id];
          const on = multi ? Array.isArray(cur) && cur.includes(r.option!) : cur === r.option;
          const b = st.size * 1.1 * scale;
          return (
            <label key={i} style={{ ...box(r), cursor: 'pointer' }}>
              <input type={multi ? 'checkbox' : 'radio'} name={field.id} checked={on} aria-label={r.option} style={{ position: 'absolute', opacity: 0, width: 1, height: 1 }}
                onChange={() => setValues({ ...values, [field.id]: multi ? (on ? (cur as string[]).filter((x) => x !== r.option) : [...((cur as string[]) ?? []), r.option!]) : r.option! })} />
              {on && <span style={{ position: 'absolute', left: b * 0.25, top: (r.h * scale - b) / 2 + b * 0.25, width: b * 0.5, height: b * 0.5, borderRadius: multi ? 2 : '50%', background: toCss(st.accent) }} />}
            </label>
          );
        }
        if (r.kind === 'submit') return <button key={i} type="submit" disabled={busy} style={{ ...box(r), background: 'transparent', border: 0, cursor: 'pointer', color: 'transparent' }}>{el.submitLabel}</button>;
        return null;
      })}
      <input name="_hp" tabIndex={-1} autoComplete="off" style={{ position: 'absolute', left: -9999 }} aria-hidden onChange={(e) => setValues({ ...values, _hp: e.target.value })} />
    </form>
  );
}
