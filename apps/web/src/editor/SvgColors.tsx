/**
 * SVG recolouring (A4): for a selected SVG image, one swatch per colour the graphic uses (fills, strokes, gradient stops,
 * currentColor). Changing a swatch rewrites that colour everywhere in the graphic. Non-destructive: the element stores an
 * original → replacement map (ImageEl.svgColors) and renderers draw the derived "<asset>/recolor/…" raster.
 */
import { CURRENT_COLOR, extractSvgColors, hex, solid, toHex, type ImageEl } from '@oc/engine';
import { RotateCcw } from 'lucide-react';
import { useEffect, useState } from 'react';
import { assetUrl } from '../api';
import { t } from '../i18n';
import { ColorButton } from './ColorPicker';
import { useDoc, useEd } from './context';

/** Swatches shown in the toolbar (graphics with more colours show their most-used ones). */
const MAX_SWATCHES = 10;
const palettes = new Map<string, Promise<string[]>>();

function svgPalette(asset: string): Promise<string[]> {
  let p = palettes.get(asset);
  if (!p) {
    p = fetch(`${assetUrl(asset)}/svg`)
      .then((r) => (r.ok ? r.text() : ''))
      .then((svg) => (svg ? extractSvgColors(svg).slice(0, MAX_SWATCHES).map((c) => c.color) : []))
      .catch(() => []);
    palettes.set(asset, p);
  }
  return p;
}

export function SvgColorControls({ el }: { el: ImageEl }) {
  const { editor } = useEd();
  const doc = useDoc();
  const isSvg = doc.assets[el.asset]?.kind === 'svg' || doc.assets[el.asset]?.mime === 'image/svg+xml';
  const [colors, setColors] = useState<string[]>([]);
  useEffect(() => {
    let live = true;
    setColors([]);
    if (isSvg) void svgPalette(el.asset).then((c) => live && setColors(c));
    return () => {
      live = false;
    };
  }, [el.asset, isSvg]);
  if (!isSvg || !colors.length) return null;
  const map = el.svgColors ?? {};
  const set = (from: string, to: string) => {
    const next = { ...(editor.doc.elements[el.id] as ImageEl | undefined)?.svgColors };
    if (to === from) delete next[from];
    else next[from] = to;
    editor.updateSelected({ svgColors: Object.keys(next).length ? next : null });
  };
  return (
    <div className="group" role="group" aria-label={t('Graphic colours')}>
      {colors.map((c) => {
        const shown = map[c] ?? (c === CURRENT_COLOR ? '#000000' : c);
        return (
          <ColorButton key={c} title={t('Graphic colour {n}', { n: colors.indexOf(c) + 1 })} doc={doc} value={solid(hex(shown))}
            onChange={(p) => p.k === 'solid' && set(c, toHex(p.color, true))} />
        );
      })}
      {Object.keys(map).length > 0 && (
        <button className="btn ghost sm icon" title={t('Reset colours')} onClick={() => editor.updateSelected({ svgColors: null }, false, false)}><RotateCcw size={16} /></button>
      )}
    </div>
  );
}
