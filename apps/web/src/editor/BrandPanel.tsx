/**
 * Brand panel (Brand Hub, A8): every brand kit the user can see (personal and team), a kit switcher, kit editing
 * (colours with names, palettes, fonts by role, logos, photos, graphics, brand voice) and "Apply kit", which
 * re-maps the design's colours (nearest hue/luminance) and fonts (by role) to the kit.
 */
import { designColors, fontSetOps, hex, solid, type AssetRef, type Editor, type TextEl } from '@oc/engine';
import { Check, Image as ImageIcon, Palette, Plus, Shapes, Sparkles, Star, Trash2, Type, Wand2 } from 'lucide-react';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { api, ApiError, assetUrl, brandHub, contentApi, type CustomFont, type Kit, type KitFontRole, type KitInput } from '../api';
import { useToast } from '../components/ui';
import { t, tk, tn } from '../i18n';
import { previewFamily } from '../lib/cssFonts';
import { activeControls, brandStore, governingKits, useBrandHub } from './brandStore';
import { useDoc, useEd } from './context';
import { addText, assetRef, insertAssetAt } from './insert';
import { setAssetDrag, withFillColor } from './dropTarget';
import { SwatchInput } from './ColorPicker';

const FONT_ROLES: Array<[KitFontRole, string, number, number]> = [
  ['heading', tk('Heading'), 64, 800], ['subheading', tk('Subheading'), 40, 600], ['body', tk('Body text'), 24, 400],
  ['quote', tk('Quote'), 32, 400], ['caption', tk('Caption'), 16, 400],
];

// ------------------------------------------------------------------ colour mapping

function hsl(h: string): [number, number, number] {
  const c = hex(h) as { r: number; g: number; b: number };
  const max = Math.max(c.r, c.g, c.b);
  const min = Math.min(c.r, c.g, c.b);
  const l = (max + min) / 2;
  const d = max - min;
  if (!d) return [0, 0, l];
  const s = d / (1 - Math.abs(2 * l - 1));
  const hue = max === c.r ? ((c.g - c.b) / d + 6) % 6 : max === c.g ? (c.b - c.r) / d + 2 : (c.r - c.g) / d + 4;
  return [hue * 60, s, l];
}

/** Distance in a hue/luminance space: hue matters in proportion to saturation (greys match by lightness). */
function colorDistance(a: string, b: string): number {
  const [ha, sa, la] = hsl(a);
  const [hb, sb, lb] = hsl(b);
  const dh = Math.min(Math.abs(ha - hb), 360 - Math.abs(ha - hb)) / 180;
  return dh * Math.min(sa, sb) * 1.5 + Math.abs(sa - sb) * 0.5 + Math.abs(la - lb) * 2;
}

/** Map each distinct design colour to its nearest brand colour. */
export function nearestBrandMap(used: string[], brand: string[]): Record<string, string> {
  const out: Record<string, string> = {};
  if (!brand.length) return out;
  for (const u of used) {
    let best = brand[0]!;
    for (const b of brand) if (colorDistance(u, b) < colorDistance(u, best)) best = b;
    if (best.toLowerCase() !== u.toLowerCase()) out[u] = best;
  }
  return out;
}

/** Apply a kit: colours to the nearest brand colours, fonts by role (heading/subheading/body, quote, caption). */
export function applyKit(editor: Editor, kit: Kit): { colors: number; fonts: number } {
  const brand = [...kit.colors, ...kit.palettes.flatMap((p) => p.colors)].map((c) => c.hex.toLowerCase());
  const colors = brand.length ? editor.replaceColors(nearestBrandMap(designColors(editor.doc).map((c) => c.hex), [...new Set(brand)])) : 0;
  let fonts = 0;
  const f = kit.fonts;
  if (f.heading || f.body) {
    const ops = fontSetOps(editor.doc, { heading: f.heading ?? f.body!, subheading: f.subheading, body: f.body ?? f.heading! });
    // Quote and caption roles get their own fonts when the kit has them.
    for (const op of ops) {
      if (op.op !== 'update') continue;
      const el = editor.doc.elements[op.id] as TextEl | undefined;
      const fam = el?.role === 'quote' ? f.quote : el?.role === 'caption' ? f.caption : undefined;
      const body = op.patch.body as TextEl['body'] | undefined;
      if (fam && body) {
        op.patch.body = { ...body, defaultRun: { ...body.defaultRun, family: fam }, paragraphs: body.paragraphs.map((p) => ({ ...p, runs: p.runs.map((r) => (r.style.family ? { ...r, style: { ...r.style, family: fam } } : r)) })) };
      }
    }
    if (ops.length) editor.apply(ops, colors > 0);
    fonts = ops.length;
  }
  return { colors, fonts };
}

// ------------------------------------------------------------------ panel

export function BrandPanel() {
  const { editor, engine } = useEd();
  const doc = useDoc();
  const toast = useToast();
  const hub = useBrandHub();
  const [newColor, setNewColor] = useState('#7c3aed');
  const [creating, setCreating] = useState(false);
  useEffect(() => {
    void brandStore.loadKits(true);
  }, []);
  const kits = hub.kits;
  const preferred = governingKits(hub);
  const kit = kits.find((k) => k.id === hub.activeKit) ?? preferred.find((k) => k.isDefault) ?? preferred[0] ?? kits[0] ?? null;
  const controls = activeControls(hub);
  const editableWs = hub.workspaces.filter((w) => w.canEdit);

  const save = async (patch: Partial<KitInput>) => {
    if (!kit) return;
    try {
      await brandHub.updateKit(kit.id, patch);
      await brandStore.loadKits(true);
    } catch (e) {
      toast(t('Could not save brand kit: {message}', { message: (e as Error).message }));
    }
  };
  const create = async (workspaceId: string) => {
    try {
      const { id } = await brandHub.createKit({ name: t('New brand kit'), workspaceId });
      await brandStore.loadKits(true);
      brandStore.setActiveKit(id);
    } catch (e) {
      if (!(e instanceof ApiError && e.status === 402)) toast(e instanceof ApiError ? e.message : t('Could not create the brand kit.'));
    } finally {
      setCreating(false);
    }
  };

  if (!hub.loaded) return <div className="empty">{t('Loading…')}</div>;
  if (!kit)
    return (
      <div className="col" style={{ gap: 16 }}>
        <p className="muted" style={{ margin: 0 }}>{t('No brand kits yet. Create one to keep colours, fonts and logos at hand.')}</p>
        {editableWs.map((w) => <button key={w.id} className="btn" onClick={() => create(w.id)}><Plus size={14} /> {w.personal ? t('Create a brand kit') : t('Create a kit for {team}', { team: w.name })}</button>)}
      </div>
    );

  const canEdit = Boolean(kit.canEdit);
  const applyColor = (h: string) => {
    if (controls.colors && !controls.colors.includes(h.toLowerCase())) return;
    const sel = editor.selectedDeep();
    if (!sel.length) return editor.updatePage(editor.state.activePageId, { background: [solid(h)] });
    if (sel.some((e) => e.type === 'text')) editor.setTextStyle({ fill: solid(h) });
    // A photo filling a shape stays; the colour goes under it.
    editor.updateSelected((e) => (e.type === 'shape' ? { fills: withFillColor(e, solid(h)) } : null), true, false);
  };
  const otherColors = [...new Set(kits.filter((k) => k.id !== kit.id).flatMap((k) => k.colors.map((c) => c.hex.toLowerCase())))].filter((h) => !kit.colors.some((c) => c.hex.toLowerCase() === h));
  const byWs = [...new Map(kits.map((k) => [k.workspaceId, k.workspace ?? ''])).entries()];

  return (
    <>
      <div className="row">
        <select className="input grow" value={kit.id} onChange={(e) => brandStore.setActiveKit(e.target.value)} aria-label={t('Brand kit')}>
          {byWs.map(([ws, name]) => (
            <optgroup key={ws} label={kits.find((k) => k.workspaceId === ws)?.personal ? t('Personal') : name}>
              {kits.filter((k) => k.workspaceId === ws).map((k) => <option key={k.id} value={k.id}>{k.name}{k.isDefault ? ` ★` : ''}</option>)}
            </optgroup>
          ))}
        </select>
        {editableWs.length > 0 && <button className="btn sm icon" title={t('New brand kit')} aria-label={t('New brand kit')} onClick={() => (editableWs.length === 1 ? create(editableWs[0]!.id) : setCreating(!creating))}><Plus size={14} /></button>}
      </div>
      {creating && (
        <div className="col" style={{ gap: 4 }}>
          {editableWs.map((w) => <button key={w.id} className="btn sm" onClick={() => create(w.id)}>{w.personal ? t('Personal') : w.name}</button>)}
        </div>
      )}
      <button className="btn primary" onClick={() => {
        const r = applyKit(editor, kit);
        toast(t('Applied “{kit}”: {colors} colour changes, {fonts} text boxes', { kit: kit.name, colors: r.colors, fonts: r.fonts }));
      }}><Wand2 size={15} /> {t('Apply kit to design')}</button>
      {canEdit && (
        <div className="row">
          <input className="input grow" defaultValue={kit.name} key={kit.id} onBlur={(e) => e.target.value.trim() && e.target.value !== kit.name && save({ name: e.target.value.trim() })} aria-label={t('Brand kit name')} />
          {!kit.isDefault && <button className="btn sm icon" title={t('Make default kit')} aria-label={t('Make default kit')} onClick={async () => { await brandHub.makeDefault(kit.id); await brandStore.loadKits(true); }}><Star size={14} /></button>}
          <button className="btn sm icon" title={t('Delete kit')} aria-label={t('Delete kit')} onClick={async () => {
            if (!confirm(t('Delete the brand kit “{name}”?', { name: kit.name }))) return;
            await brandHub.deleteKit(kit.id);
            brandStore.setActiveKit(null);
            await brandStore.loadKits(true);
          }}><Trash2 size={14} /></button>
        </div>
      )}

      <h4><Palette size={13} /> {t('Colours')}</h4>
      <div className="row" style={{ flexWrap: 'wrap', gap: 6 }}>
        {kit.colors.map((c, i) => (
          <div key={i} style={{ position: 'relative' }}>
            <button title={t('{name} {hex} — click to apply', { name: c.name, hex: c.hex })} aria-label={c.name} style={{ width: 40, height: 40, borderRadius: 10, background: c.hex, border: '1px solid var(--border)', cursor: 'pointer' }} onClick={() => applyColor(c.hex)} />
            {canEdit && (
              <button className="btn sm icon" style={{ position: 'absolute', top: -8, right: -8, width: 20, height: 20, borderRadius: 10 }} aria-label={t('Remove colour')}
                onClick={() => save({ colors: kit.colors.filter((_, j) => j !== i) })}><Trash2 size={10} /></button>
            )}
          </div>
        ))}
        {canEdit && (
          <div className="row" style={{ gap: 4 }}>
            <SwatchInput value={newColor} onChange={setNewColor} title={t('New brand colour')} />
            <button className="btn sm" onClick={() => {
              const name = prompt(t('Colour name'), t('Colour {n}', { n: kit.colors.length + 1 }));
              if (name !== null) void save({ colors: [...kit.colors, { name: name.slice(0, 40) || t('Colour {n}', { n: kit.colors.length + 1 }), hex: newColor }] });
            }}><Plus size={14} /> {t('Add')}</button>
          </div>
        )}
      </div>
      {kit.palettes.map((p, i) => (
        <div key={i} className="row" style={{ gap: 4 }}>
          <span className="grow" style={{ fontSize: 12 }}>{p.name}</span>
          {p.colors.map((c) => <button key={c.hex + c.name} title={`${c.name} ${c.hex}`} aria-label={c.name} onClick={() => applyColor(c.hex)} style={{ width: 22, height: 22, borderRadius: 6, background: c.hex, border: '1px solid var(--border)' }} />)}
          <button className="btn sm" onClick={() => editor.applyPalette(p.colors.map((c) => c.hex))} title={t('Recolour the design with this palette')}><Sparkles size={12} /></button>
          {canEdit && <button className="btn ghost sm icon" aria-label={t('Remove palette')} onClick={() => save({ palettes: kit.palettes.filter((_, j) => j !== i) })}><Trash2 size={12} /></button>}
        </div>
      ))}
      {canEdit && (
        <button className="btn sm" onClick={() => {
          const used = designColors(doc).slice(0, 8);
          if (!used.length) return;
          const name = prompt(t('Palette name'), t('Palette {n}', { n: kit.palettes.length + 1 }));
          if (name !== null) void save({ palettes: [...kit.palettes, { name: name.slice(0, 60) || t('Palette {n}', { n: kit.palettes.length + 1 }), colors: used.map((c, i) => ({ name: t('Colour {n}', { n: i + 1 }), hex: c.hex })) }] });
        }}><Plus size={14} /> {t('Save this design’s colours as a palette')}</button>
      )}
      {otherColors.length > 0 && (
        <>
          <span className="faint" style={{ fontSize: 12 }}>{t('From your other kits')}</span>
          <div className="row" style={{ flexWrap: 'wrap', gap: 4 }}>
            {otherColors.map((h) => <button key={h} title={h} aria-label={h} onClick={() => applyColor(h)} style={{ width: 24, height: 24, borderRadius: 6, background: h, border: '1px solid var(--border)' }} />)}
          </div>
        </>
      )}

      <h4><Type size={13} /> {t('Fonts')}</h4>
      {FONT_ROLES.map(([role, label, size, weight]) => {
        const fam = kit.fonts[role];
        const f = fam ? engine.fonts.families().find((x) => x.family === fam) : undefined;
        return (
          <div key={role} className="row">
            <button className="btn grow" style={{ justifyContent: 'flex-start', fontFamily: f ? previewFamily(f, true) : undefined, fontSize: Math.min(20, size / 2.5) }}
              onClick={() => addText(editor, t(label), { family: fam ?? 'Inter', size, weight })} title={t('Add {label}', { label: t(label) })}>{t(label)}</button>
            {canEdit ? (
              <FontSelect value={fam ?? ''} onChange={(v) => save({ fonts: { ...kit.fonts, [role]: v || undefined } })} label={t('{label} font', { label: t(label) })} />
            ) : <span className="faint" style={{ fontSize: 12 }}>{fam ?? '—'}</span>}
          </div>
        );
      })}
      {(() => {
        // Kit fonts that are neither Google fonts nor uploaded draw in a fallback until someone uploads them.
        const known = new Set(engine.fonts.families().map((x) => x.family.toLowerCase()));
        const missing = [...new Set(Object.values(kit.fonts).filter((f): f is string => Boolean(f) && !known.has(f!.toLowerCase())))];
        return missing.length ? (
          <p className="faint" role="status" style={{ margin: 0, fontSize: 12 }}>
            {t('Not uploaded yet, so a fallback font shows instead: {fonts}. Upload the font files under Brand fonts.', { fonts: missing.join(', ') })}
          </p>
        ) : null;
      })()}
      <BrandFonts canUpload={!controls.restrictUploads} />

      <AssetGroup title={t('Logos')} icon={<ImageIcon size={13} />} ids={kit.logos} canEdit={canEdit} onChange={(logos) => save({ logos })} />
      <AssetGroup title={t('Brand photos')} icon={<ImageIcon size={13} />} ids={kit.photos} canEdit={canEdit} onChange={(photos) => save({ photos })} />
      <AssetGroup title={t('Brand graphics')} icon={<Shapes size={13} />} ids={kit.graphics} canEdit={canEdit} onChange={(graphics) => save({ graphics })} />

      <h4><Check size={13} /> {t('Brand voice')}</h4>
      {canEdit ? (
        <textarea className="input" rows={4} key={`voice-${kit.id}`} defaultValue={kit.voice} maxLength={4000} placeholder={t('How your brand sounds: tone, words to use and avoid…')}
          onBlur={(e) => e.target.value !== kit.voice && save({ voice: e.target.value })} aria-label={t('Brand voice')} />
      ) : <p className="muted" style={{ margin: 0, fontSize: 13, whiteSpace: 'pre-wrap' }}>{kit.voice || t('No brand voice notes yet.')}</p>}
    </>
  );
}

function FontSelect({ value, onChange, label }: { value: string; onChange: (v: string) => void; label: string }) {
  const { engine } = useEd();
  const fams = useMemo(() => engine.fonts.families().map((f) => f.family), [engine]);
  const id = useMemo(() => `fonts-${Math.random().toString(36).slice(2)}`, []);
  return (
    <>
      <input className="input" style={{ width: 130 }} list={id} defaultValue={value} key={value} aria-label={label}
        onBlur={(e) => { const v = e.target.value.trim(); if (v !== value && (!v || fams.includes(v))) onChange(v); }} />
      <datalist id={id}>{fams.slice(0, 400).map((f) => <option key={f} value={f} />)}</datalist>
    </>
  );
}

const BRAND_IMAGE_TYPES = ['image/png', 'image/svg+xml', 'image/jpeg', 'image/webp'];

function AssetGroup({ title, icon, ids, canEdit, onChange }: { title: string; icon: React.ReactNode; ids: string[]; canEdit: boolean; onChange: (ids: string[]) => void }) {
  const { editor } = useEd();
  const toast = useToast();
  const input = useRef<HTMLInputElement>(null);
  const [dropping, setDropping] = useState(false);
  const [uploading, setUploading] = useState(0);
  // Your copies of the kit's files, so tiles can be dragged onto the page (or into a shape or frame).
  const [mine, setMine] = useState<Map<string, AssetRef>>(new Map());
  const key = ids.join();
  useEffect(() => {
    if (!key) return;
    let live = true;
    api.assets().then((r) => live && setMine(new Map(r.assets.map((a) => [a.id, assetRef(a)])))).catch(() => {});
    return () => {
      live = false;
    };
  }, [key]);
  const addFiles = async (files: File[]) => {
    const added: string[] = [];
    setUploading((n) => n + files.length);
    for (const f of files) {
      try {
        if (!BRAND_IMAGE_TYPES.includes(f.type)) throw new Error(f.name);
        const { asset } = await api.upload(f);
        added.push(asset.id);
      } catch (x) {
        toast(t('Upload failed: {message}', { message: (x as Error).message }));
      }
      setUploading((n) => n - 1);
    }
    if (added.length) onChange([...new Set([...ids, ...added])]);
  };
  // Only files from the computer: dragging one of the panel's own tiles carries an asset, not a file.
  const hasFiles = (e: React.DragEvent) => canEdit && e.dataTransfer.types.includes('Files');
  if (!ids.length && !canEdit) return null;
  return (
    <>
      <h4>{icon} {title}</h4>
      <div className="tiles" style={dropping ? { outline: '2px dashed var(--accent)', outlineOffset: 2, borderRadius: 8 } : undefined}
        onDragOver={(e) => { if (!hasFiles(e)) return; e.preventDefault(); e.stopPropagation(); e.dataTransfer.dropEffect = 'copy'; setDropping(true); }}
        onDragLeave={(e) => { if (!e.currentTarget.contains(e.relatedTarget as Node | null)) setDropping(false); }}
        onDrop={(e) => { if (!hasFiles(e)) return; e.preventDefault(); e.stopPropagation(); setDropping(false); void addFiles([...e.dataTransfer.files]); }}>
        {ids.map((id) => (
          <div key={id} style={{ position: 'relative' }}>
            <button className="tile" style={{ width: '100%' }} draggable={mine.has(id)} onDragStart={(e) => {
              const a = mine.get(id);
              if (a) setAssetDrag(e.dataTransfer, a);
            }} onClick={async () => {
              const a = mine.get(id) ?? (await api.assets().then((r) => r.assets.find((x) => x.id === id)).then((x) => (x ? assetRef(x) : undefined)));
              if (a) insertAssetAt(editor, a, undefined);
              else toast(t('This file isn’t in your uploads.'));
            }}><img src={assetUrl(id)} alt={title} style={{ objectFit: 'contain' }} draggable={false} /></button>
            {canEdit && <button className="btn sm icon" style={{ position: 'absolute', top: 4, right: 4, width: 22, height: 22 }} aria-label={t('Remove')} onClick={() => onChange(ids.filter((x) => x !== id))}><Trash2 size={11} /></button>}
          </div>
        ))}
        {Array.from({ length: uploading }, (_, i) => (
          <div key={i} className="tile" style={{ display: 'grid', placeItems: 'center' }} aria-hidden><div className="spinner" /></div>
        ))}
        {canEdit && <button className="tile" onClick={() => input.current?.click()} title={t('Upload')} aria-label={t('Upload')}><Plus size={22} /></button>}
      </div>
      {uploading > 0 && <p className="faint" role="status" style={{ margin: '4px 0 0', fontSize: 12 }}>{tn(uploading, 'Uploading {n} file…', 'Uploading {n} files…')}</p>}
      <input ref={input} type="file" accept={BRAND_IMAGE_TYPES.join()} multiple hidden onChange={(e) => {
        const files = [...(e.target.files ?? [])];
        e.target.value = '';
        void addFiles(files);
      }} />
    </>
  );
}

/** Uploaded brand fonts (FT-020): TTF/OTF files your organisation licenses; usable by everyone in the workspace. */
function BrandFonts({ canUpload }: { canUpload: boolean }) {
  const { engine, editor } = useEd();
  const toast = useToast();
  const [fonts, setFonts] = useState<CustomFont[]>([]);
  const [ack, setAck] = useState(false);
  const input = useRef<HTMLInputElement>(null);
  const load = useCallback(() => contentApi.customFonts().then((r) => setFonts(r.fonts)).catch(() => {}), []);
  useEffect(() => {
    void load();
  }, [load]);
  const families = [...new Set(fonts.map((f) => f.family))];
  return (
    <>
      <h4><Type size={13} /> {t('Brand fonts')}</h4>
      {families.map((fam) => {
        const faces = fonts.filter((f) => f.family === fam);
        const f = engine.fonts.families().find((x) => x.family === fam);
        return (
          <div key={fam} className="row">
            <button className="btn grow" style={{ justifyContent: 'flex-start', fontFamily: f ? previewFamily(f, true) : undefined }} onClick={() => addText(editor, fam, { family: fam, size: 48 })}>{fam}</button>
            <span className="faint" style={{ fontSize: 11 }}>{tn(faces.length, '{n} style', '{n} styles')}</span>
            {canUpload && <button className="btn ghost sm icon" aria-label={t('Remove {name}', { name: fam })} onClick={async () => { for (const x of faces) await contentApi.deleteFont(x.id); void load(); }}><Trash2 size={13} /></button>}
          </div>
        );
      })}
      {!canUpload ? <p className="faint" style={{ margin: 0, fontSize: 12 }}>{t('Your team only lets owners and admins upload brand fonts.')}</p> : (
        <>
          <label className="check" style={{ fontSize: 12 }}><input type="checkbox" checked={ack} onChange={(e) => setAck(e.target.checked)} /> {t('I have the right to use these fonts in this workspace')}</label>
          <button className="btn sm" disabled={!ack} onClick={() => input.current?.click()}><Plus size={14} /> {t('Upload a font (TTF, OTF)')}</button>
        </>
      )}
      <input ref={input} type="file" accept=".ttf,.otf,font/ttf,font/otf" multiple hidden onChange={async (e) => {
        for (const file of [...(e.target.files ?? [])]) {
          try {
            const { font } = await contentApi.uploadFont(file);
            const existing = engine.fonts.families().find((x) => x.family === font.family);
            const face = { weight: font.weight, italic: font.italic, file: font.file, sha256: font.sha256, bytes: font.bytes };
            engine.fonts.addFamily(existing
              ? { ...existing, faces: [...existing.faces.filter((x) => x.weight !== face.weight || x.italic !== face.italic), face] }
              : { family: font.family, slug: `custom-${font.id}`, category: 'uploaded', license: 'Uploaded', faces: [face] });
            toast(font.italic ? t('Added {family} {weight} italic', { family: font.family, weight: font.weight }) : t('Added {family} {weight}', { family: font.family, weight: font.weight }));
          } catch (x) {
            toast(x instanceof ApiError ? x.message ?? t('Upload failed.') : t('Upload failed.'));
          }
        }
        e.target.value = '';
        void load();
      }} />
    </>
  );
}
