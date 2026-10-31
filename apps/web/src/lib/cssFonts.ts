/**
 * Registers CSS FontFaces so the font picker can preview each family in itself. Google Fonts families use
 * a tiny subset containing just their name; bundled families use their regular face.
 */
import type { FontFamily } from '@oc/engine';

const done = new Set<string>();

/** CSS family name for previewing `f`; `anyText` loads the whole regular face (Google families otherwise only have their name's glyphs). */
export function previewFamily(f: FontFamily, anyText = false): string {
  const subset = f.source === 'google' && !anyText;
  const css = `oc-preview-${f.slug}${f.source === 'google' && anyText ? '-full' : ''}`;
  if (!done.has(css)) {
    done.add(css);
    const face = f.faces.find((x) => x.weight === 400 && !x.italic) ?? f.faces[0];
    if (face) {
      const src = subset ? `/api/fonts/preview/${f.slug}.ttf` : `/api/fonts/files/${face.file}`;
      const ff = new FontFace(css, `url(${src})`, { weight: '400', display: 'swap' });
      ff.load().then((loaded) => document.fonts.add(loaded)).catch(() => {});
    }
  }
  return css;
}
