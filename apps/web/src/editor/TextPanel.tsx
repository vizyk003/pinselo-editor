/**
 * Text panel (plan 11 g68): add a text box or a default style, brand fonts (the governing team's kit roles and
 * uploaded families), page numbers, and a searchable gallery of styled font combinations. One search filters the
 * combinations by name, mood and font, and lists matching font families to add text in.
 */
import { hex, solid, type RunStyle } from '@oc/engine';
import { Type } from 'lucide-react';
import { useMemo, useState } from 'react';
import { SearchField, useToast } from '../components/ui';
import { t, tk, tn } from '../i18n';
import { previewFamily } from '../lib/cssFonts';
import { governingKits, useBrandHub } from './brandStore';
import { useEd } from './context';
import { addText, TEXT_PRESETS } from './insert';
import { recordRecentFont } from './FontPicker';
import { elementRole, runStyleOf, styleFor } from './kitTextStyles';

type Combo = { title: string; titleFont: string; titleWeight: number; sub: string; subFont: string; tags: string };
/** Styled pairs; `tags` are English search words (the translated title and subtitle are searched too). */
const COMBOS: Combo[] = [
  { title: tk('BIG SALE'), titleFont: 'Bebas Neue', titleWeight: 400, sub: tk('This weekend only'), subFont: 'Montserrat', tags: 'sale promo shop retail bold' },
  { title: tk('Elegant'), titleFont: 'Great Vibes', titleWeight: 400, sub: tk('WEDDING SEASON'), subFont: 'Lora', tags: 'wedding script love invitation' },
  { title: tk('Bold Idea'), titleFont: 'Archivo Black', titleWeight: 400, sub: tk('Make it happen'), subFont: 'Inter', tags: 'business bold modern' },
  { title: tk('Editorial'), titleFont: 'Playfair Display', titleWeight: 700, sub: tk('Issue no. 12'), subFont: 'Lato', tags: 'magazine serif news' },
  { title: tk('Hello there'), titleFont: 'Pacifico', titleWeight: 400, sub: tk('Say hi!'), subFont: 'Nunito', tags: 'friendly fun script greeting' },
  { title: tk('TECH TALK'), titleFont: 'Space Grotesk', titleWeight: 700, sub: tk('Live Q&A'), subFont: 'Rubik', tags: 'tech event webinar modern' },
  { title: tk('Grand Opening'), titleFont: 'Abril Fatface', titleWeight: 400, sub: tk('Join us on Saturday'), subFont: 'Poppins', tags: 'event opening store announcement' },
  { title: tk('Summer Vibes'), titleFont: 'Lobster', titleWeight: 400, sub: tk('Sun, sand and sea'), subFont: 'Quicksand', tags: 'summer travel holiday beach' },
  { title: tk('Minimal'), titleFont: 'Outfit', titleWeight: 300, sub: tk('Less is more'), subFont: 'Outfit', tags: 'minimal clean simple modern' },
  { title: tk('Thank you'), titleFont: 'Dancing Script', titleWeight: 700, sub: tk('For being here'), subFont: 'Raleway', tags: 'thanks card gratitude script' },
  { title: tk('GAME ON'), titleFont: 'Bungee', titleWeight: 400, sub: tk('Tournament night'), subFont: 'Barlow Condensed', tags: 'gaming sports esports bold' },
  { title: tk('Fresh Menu'), titleFont: 'DM Serif Display', titleWeight: 400, sub: tk('Seasonal dishes'), subFont: 'Karla', tags: 'restaurant food menu cafe' },
  { title: tk('Happy Birthday'), titleFont: 'Fredoka', titleWeight: 600, sub: tk('Let’s celebrate'), subFont: 'Nunito', tags: 'birthday party kids celebration' },
  { title: tk('Annual Report'), titleFont: 'IBM Plex Sans', titleWeight: 700, sub: tk('Results and outlook'), subFont: 'Source Serif 4', tags: 'report business corporate finance' },
  { title: tk('Coming Soon'), titleFont: 'Unbounded', titleWeight: 700, sub: tk('Stay tuned'), subFont: 'Sora', tags: 'launch teaser announcement' },
  { title: tk('Notes to self'), titleFont: 'Caveat', titleWeight: 600, sub: tk('Written by hand'), subFont: 'Patrick Hand', tags: 'handwriting casual notes journal' },
  { title: tk('Classic'), titleFont: 'Cinzel', titleWeight: 700, sub: tk('Since 1925'), subFont: 'Cormorant Garamond', tags: 'luxury classic heritage elegant' },
  { title: tk('LIMITED OFFER'), titleFont: 'Anton', titleWeight: 400, sub: tk('Ends at midnight'), subFont: 'Roboto', tags: 'sale promo discount urgent' },
  { title: tk('Retro Night'), titleFont: 'Monoton', titleWeight: 400, sub: tk('Dance till dawn'), subFont: 'Josefin Sans', tags: 'retro party music night' },
  { title: tk('Pixel Quest'), titleFont: 'Press Start 2P', titleWeight: 400, sub: tk('Level up'), subFont: 'Space Grotesk', tags: 'gaming retro pixel arcade' },
  { title: tk('Wellness'), titleFont: 'Fraunces', titleWeight: 600, sub: tk('Breathe and relax'), subFont: 'Mulish', tags: 'health yoga calm spa' },
  { title: tk('Workshop'), titleFont: 'Syne', titleWeight: 700, sub: tk('Hands-on session'), subFont: 'DM Sans', tags: 'education class course event' },
  { title: tk('Love Letter'), titleFont: 'Allura', titleWeight: 400, sub: tk('Forever yours'), subFont: 'EB Garamond', tags: 'love valentine romantic script' },
  { title: tk('Headline News'), titleFont: 'Oswald', titleWeight: 700, sub: tk('Read the full story'), subFont: 'Merriweather', tags: 'news blog article serif' },
];
const BRAND_ROLES: Array<{ role: 'heading' | 'subheading' | 'body'; label: string }> = [
  { role: 'heading', label: tk('Brand heading') }, { role: 'subheading', label: tk('Brand subheading') }, { role: 'body', label: tk('Brand body text') },
];

export function TextPanel() {
  const { editor, engine } = useEd();
  const toast = useToast();
  const hub = useBrandHub();
  const [q, setQ] = useState('');
  const needle = q.trim().toLowerCase();
  const families = engine.fonts.families();
  const fam = (name: string) => families.find((f) => f.family === name);
  const add = (text: string, style: Partial<RunStyle>, opts?: Parameters<typeof addText>[3]) => {
    if (style.family) recordRecentFont(style.family);
    return addText(editor, text, style, opts);
  };
  // Brand fonts: the governing team's kit roles, then fonts uploaded to the team.
  const kit = governingKits(hub).find((k) => Object.values(k.fonts).some(Boolean));
  const uploaded = families.filter((f) => f.category === 'uploaded').slice(0, 6);
  const combos = useMemo(() => (needle
    ? COMBOS.filter((c) => [t(c.title), t(c.sub), c.titleFont, c.subFont, c.tags].some((s) => s.toLowerCase().includes(needle)))
    : COMBOS), [needle]);
  const fontHits = useMemo(() => (needle.length > 1 ? families.filter((f) => f.family.toLowerCase().includes(needle)).sort((a, b) => Number(!a.family.toLowerCase().startsWith(needle)) - Number(!b.family.toLowerCase().startsWith(needle))).slice(0, 6) : []), [families, needle]);
  const pageNumbers = (o: Parameters<typeof editor.addPageNumbers>[0]) => {
    const n = editor.addPageNumbers(o);
    toast(n ? tn(n, 'Added to {n} page', 'Added to {n} pages') : t('Every page already has a number'));
  };
  return (
    <>
      <button className="btn primary" onClick={() => add(t('Your text here'), { size: 32 }, { edit: true })}>
        <Type size={16} /> {t('Add a text box')}
      </button>
      <SearchField value={q} onChange={setQ} placeholder={t('Search fonts and combinations')} onClear={() => setQ('')} />
      {!needle && (
        <>
          <h4>{t('Default text styles')}</h4>
          <div className="text-add">
            <button onClick={() => add(t(TEXT_PRESETS.heading.text), TEXT_PRESETS.heading.style)} style={{ fontSize: 26, fontWeight: 800 }}>{t('Add a heading')}</button>
            <button onClick={() => add(t(TEXT_PRESETS.subheading.text), TEXT_PRESETS.subheading.style)} style={{ fontSize: 18, fontWeight: 600 }}>{t('Add a subheading')}</button>
            <button onClick={() => add(t(TEXT_PRESETS.body.text), TEXT_PRESETS.body.style)} style={{ fontSize: 13 }}>{t('Add a little bit of body text')}</button>
          </div>
        </>
      )}
      {!needle && (kit || uploaded.length > 0) && (
        <>
          <h4>{t('Brand fonts')}</h4>
          <div className="text-add">
            {kit?.textStyles?.length ? kit.textStyles.map((st, i) => {
              // The kit's own text styles: font, weight, italic, capitals, spacing and colour as the brand sets them.
              const family = kit.fonts[st.role];
              const f = family ? fam(family) : undefined;
              return (
                <button key={`${st.name}-${i}`} onClick={() => add(st.name, runStyleOf(kit, st, (h) => solid(hex(h))), { role: elementRole(st.role), lineHeight: st.lineHeight, align: 'start' })}
                  title={family ? `${st.name} · ${family}` : st.name}
                  style={{
                    fontFamily: f ? previewFamily(f, true) : undefined, fontWeight: st.weight, fontStyle: st.italic ? 'italic' : undefined, textTransform: st.uppercase ? 'uppercase' : undefined,
                    letterSpacing: st.letterSpacing ? `${st.letterSpacing / 1000}em` : undefined, fontSize: Math.max(12, Math.min(24, (st.size ?? 26) / 2)),
                  }}>
                  {st.name}
                </button>
              );
            }) : kit && BRAND_ROLES.map(({ role, label }) => {
              const family = kit.fonts[role];
              if (!family) return null;
              const preset = TEXT_PRESETS[role];
              const st = styleFor(kit, role);
              const f = fam(family);
              return (
                <button key={role} onClick={() => add(t(preset.text), st ? { ...preset.style, ...runStyleOf(kit, st, (h) => solid(hex(h))) } : { ...preset.style, family }, { role: elementRole(role), lineHeight: st?.lineHeight })} title={`${t(label)} · ${family}`}
                  style={{ fontFamily: f ? previewFamily(f, true) : undefined, fontSize: role === 'heading' ? 24 : role === 'subheading' ? 17 : 13, fontWeight: st?.weight ?? preset.style.weight }}>
                  {t(preset.text)}
                </button>
              );
            })}
            {uploaded.filter((f) => !kit || !Object.values(kit.fonts).includes(f.family)).map((f) => (
              <button key={f.family} onClick={() => add(t('Your text here'), { family: f.family, size: 40 })} style={{ fontFamily: previewFamily(f, true), fontSize: 16 }} title={t('Add text in {font}', { font: f.family })}>{f.family}</button>
            ))}
          </div>
          {kit && <span className="faint" style={{ fontSize: 11 }}>{t('From the brand kit “{kit}”', { kit: kit.name })}</span>}
        </>
      )}
      {!needle && (
        <>
          <h4>{t('Page numbers')}</h4>
          <div className="row" style={{ flexWrap: 'wrap', gap: 6 }}>
            <button className="btn sm" onClick={() => pageNumbers({ position: 'bottom-right' })}>{t('Bottom right')}</button>
            <button className="btn sm" onClick={() => pageNumbers({ position: 'bottom-center' })}>{t('Bottom centre')}</button>
            <button className="btn sm" onClick={() => pageNumbers({ position: 'bottom-right', text: t('Page {{page}} of {{pages}}') })}>{t('“Page 1 of 5”')}</button>
          </div>
        </>
      )}
      {fontHits.length > 0 && (
        <>
          <h4>{t('Fonts')}</h4>
          <div className="text-add" role="group" aria-label={t('Fonts')}>
            {fontHits.map((f) => (
              <button key={f.family} onClick={() => add(f.family, { family: f.family, size: 48 })} style={{ fontFamily: previewFamily(f, true), fontSize: 17 }} title={t('Add text in {font}', { font: f.family })}>{f.family}</button>
            ))}
          </div>
        </>
      )}
      <h4>{t('Font combinations')}</h4>
      <div className="combo-grid">
        {combos.map((c) => {
          const tf = fam(c.titleFont);
          const sf = fam(c.subFont);
          return (
            <button key={c.title} title={`${c.titleFont} + ${c.subFont}`} onClick={() => {
              const W = editor.doc.meta.dims.w;
              const k = Math.max(0.4, Math.min(3, W / 1080));
              const ids = [...add(t(c.title), { family: c.titleFont, weight: c.titleWeight, size: 72 }, { at: [W / 2, editor.doc.meta.dims.h / 2 - 30 * k] })];
              const sub = add(t(c.sub), { family: c.subFont, size: 28, tracking: c.sub === c.sub.toUpperCase() ? 150 : 0 }, { at: [W / 2, editor.doc.meta.dims.h / 2 + 50 * k] });
              editor.select([...ids, ...sub]);
              editor.group();
            }}>
              <span style={{ fontFamily: tf ? previewFamily(tf, true) : undefined, fontSize: 20, fontWeight: c.titleWeight }}>{t(c.title)}</span>
              <span style={{ fontFamily: sf ? previewFamily(sf, true) : undefined, fontSize: 11 }} className="muted">{t(c.sub)}</span>
            </button>
          );
        })}
      </div>
      {needle && !combos.length && !fontHits.length && <p className="faint" style={{ fontSize: 12, margin: 0 }}>{t('Nothing matches “{q}”.', { q })}</p>}
    </>
  );
}
