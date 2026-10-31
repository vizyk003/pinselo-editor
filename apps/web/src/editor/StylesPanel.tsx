/**
 * Styles (A17): recolour the whole design with a palette, apply a font pair, shuffle, and replace any
 * colour the design uses (click a document colour, pick its replacement).
 */
import { designColors } from '@oc/engine';
import { Shuffle } from 'lucide-react';
import { useMemo } from 'react';
import { t, tk } from '../i18n';
import { previewFamily } from '../lib/cssFonts';
import { useDoc, useEd } from './context';
import { SwatchInput } from './ColorPicker';

const PALETTES: Array<[string, string[]]> = [
  [tk('Sunset'), ['#2d1b4e', '#7c3aed', '#f472b6', '#fb923c', '#fde68a']],
  [tk('Ocean'), ['#0b1d3a', '#0e7490', '#22d3ee', '#a5f3fc', '#f0fdff']],
  [tk('Forest'), ['#14281d', '#2f5d3a', '#7fb069', '#e6aa68', '#fdf6e3']],
  [tk('Terracotta'), ['#3b1f1a', '#a0522d', '#e07a5f', '#f2cc8f', '#f4f1de']],
  [tk('Monochrome'), ['#0a0a0a', '#404040', '#a3a3a3', '#e5e5e5', '#ffffff']],
  [tk('Candy'), ['#3d0f4c', '#ff4fa3', '#ffb703', '#8ecae6', '#fff7fb']],
  [tk('Corporate'), ['#0f172a', '#1d4ed8', '#38bdf8', '#cbd5e1', '#f8fafc']],
  [tk('Earth'), ['#2b2118', '#6b4f3a', '#b08968', '#ddb892', '#f6efe6']],
  [tk('Neon'), ['#09090b', '#22d3ee', '#a3e635', '#f0abfc', '#fafafa']],
  [tk('Pastel'), ['#4a4e69', '#9a8c98', '#c9ada7', '#f2e9e4', '#ffffff']],
  [tk('Citrus'), ['#1b4332', '#f77f00', '#fcbf49', '#eae2b7', '#fffdf5']],
  [tk('Berry'), ['#2b0a3d', '#7b2cbf', '#c77dff', '#ff8fab', '#fff0f6']],
  [tk('Nordic'), ['#1f2933', '#52606d', '#9aa5b1', '#e4e7eb', '#f5f7fa']],
  [tk('Tropical'), ['#003049', '#0096c7', '#2ec4b6', '#ff9f1c', '#fff8e7']],
  [tk('Wine'), ['#1a0a10', '#5c1a33', '#9d4edd', '#e0aaff', '#fbf8ff']],
  [tk('Mint'), ['#0b3d2e', '#2d6a4f', '#52b788', '#b7e4c7', '#f1faee']],
];
const FONT_PAIRS: Array<{ heading: string; body: string; label: string }> = [
  { heading: 'Fraunces', body: 'Inter', label: tk('Editorial') },
  { heading: 'Playfair Display', body: 'Lato', label: tk('Elegant') },
  { heading: 'Bebas Neue', body: 'Montserrat', label: tk('Bold') },
  { heading: 'Space Grotesk', body: 'Inter', label: tk('Tech') },
  { heading: 'DM Serif Display', body: 'DM Sans', label: tk('Classic') },
  { heading: 'Syne', body: 'Manrope', label: tk('Creative') },
  { heading: 'Anton', body: 'Roboto', label: tk('Impact') },
  { heading: 'Cormorant Garamond', body: 'Raleway', label: tk('Luxury') },
  { heading: 'Fredoka', body: 'Nunito', label: tk('Playful') },
  { heading: 'Unbounded', body: 'Onest', label: tk('Modern') },
  { heading: 'Instrument Serif', body: 'Instrument Sans', label: tk('Minimal') },
  { heading: 'Pacifico', body: 'Quicksand', label: tk('Friendly') },
];

export function StylesPanel() {
  const { editor, engine } = useEd();
  const doc = useDoc();
  const colors = useMemo(() => designColors(doc).slice(0, 12), [doc]);
  const fam = (name: string) => engine.fonts.families().find((f) => f.family === name);
  const shuffle = () => {
    const [, pal] = PALETTES[Math.floor(Math.random() * PALETTES.length)]!;
    const fp = FONT_PAIRS[Math.floor(Math.random() * FONT_PAIRS.length)]!;
    editor.applyPalette(pal);
    editor.applyFonts(fp);
  };
  return (
    <>
      <button className="btn primary" onClick={shuffle}><Shuffle size={16} /> {t('Shuffle styles')}</button>
      {colors.length > 0 && (
        <>
          <h4>{t('Colours in this design')}</h4>
          <p className="faint" style={{ margin: 0, fontSize: 12 }}>{t('Click a colour to replace it everywhere.')}</p>
          <div className="row" style={{ flexWrap: 'wrap', gap: 6 }}>
            {/* Keyed by position: a replaced colour keeps its swatch (and its open picker) while the hex changes. */}
            {colors.map((c, i) => (
              <SwatchInput key={i} value={c.hex} title={t('Replace {color} ({n} uses)', { color: c.hex, n: c.count })} doc={doc}
                onChange={(next) => next.toLowerCase() !== c.hex.toLowerCase() && editor.replaceColors({ [c.hex]: next })} />
            ))}
          </div>
        </>
      )}
      <h4>{t('Colour palettes')}</h4>
      <div className="col" style={{ gap: 8 }}>
        {PALETTES.map(([name, pal]) => (
          <button key={name} className="palette-row" onClick={() => editor.applyPalette(pal)} title={t('Apply {name}', { name: t(name) })}>
            <span className="palette">{pal.map((c) => <span key={c} style={{ background: c }} />)}</span>
            <span className="faint" style={{ fontSize: 12 }}>{t(name)}</span>
          </button>
        ))}
      </div>
      <h4>{t('Font sets')}</h4>
      <div className="col" style={{ gap: 8 }}>
        {FONT_PAIRS.map((p) => {
          const h = fam(p.heading);
          const b = fam(p.body);
          return (
            <button key={p.label} className="tile" style={{ height: 'auto', padding: 10, alignItems: 'flex-start', flexDirection: 'column', gap: 2 }} onClick={() => editor.applyFonts(p)}>
              <span style={{ fontFamily: h ? previewFamily(h, true) : undefined, fontSize: 20 }}>{p.heading}</span>
              <span style={{ fontFamily: b ? previewFamily(b, true) : undefined, fontSize: 12 }} className="muted">{p.body} · {t(p.label)}</span>
            </button>
          );
        })}
      </div>
    </>
  );
}
