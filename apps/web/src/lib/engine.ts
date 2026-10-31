/**
 * Browser engine singletons: the font registry, the text engine and the painter — all the Rust core in
 * WebAssembly (`@oc/engine`: oc-text and oc-render, the same code the render service exports with).
 *
 * One font registry serves measuring (the editor's `textService`), caret geometry and drawing. Faces load lazily
 * by manifest path from `/api/fonts/files/` — bundled families, Google Fonts faces and uploaded brand fonts
 * alike: whatever a layout or a draw lacks is reported by the Rust engine and fetched once
 * (`FontRegistry.pump`), and each arriving face triggers a repaint (`painter.onInvalidate`) and re-measuring.
 */
import { FontRegistry, Painter, RenderTarget, TextEngine, type AssetId, type DisplayList, type FontManifest } from '@oc/engine';
import { assetUrl } from '../api';

export interface Engine { fonts: FontRegistry; text: TextEngine; painter: Painter }

let enginePromise: Promise<Engine> | null = null;

const bytes = (what: string) => (r: Response) => {
  if (!r.ok) throw new Error(`${what}: ${r.status}`);
  return r.arrayBuffer().then((b) => new Uint8Array(b));
};

export function loadEngine(): Promise<Engine> {
  enginePromise ??= (async () => {
    const manifest = await fetch('/api/fonts/manifest.json').then((r) => r.json() as Promise<FontManifest>);
    const fonts = new FontRegistry(manifest, (file) => fetch(`/api/fonts/files/${file}`).then(bytes(`font ${file}`)));
    await fonts.ensure('Inter', 400, false);
    const text = new TextEngine(fonts);
    const painter = new Painter(text, (asset: AssetId) => fetch(assetUrl(asset)).then(bytes(`asset ${asset}`)));
    return { fonts, text, painter };
  })();
  return enginePromise;
}

/** RGBA (0..1) of a CSS colour (`#rrggbb`), for clearing. */
export function rgba(css: string, fallback: [number, number, number, number] = [1, 1, 1, 1]): [number, number, number, number] {
  const m = /^#([0-9a-f]{6})$/i.exec(css.trim());
  if (!m) return fallback;
  const n = parseInt(m[1]!, 16);
  return [((n >> 16) & 255) / 255, ((n >> 8) & 255) / 255, (n & 255) / 255, 1];
}

export const WHITE: [number, number, number, number] = [1, 1, 1, 1];
export const BLACK: [number, number, number, number] = [0, 0, 0, 1];

/**
 * Paint a display list into a 2D canvas (thumbnails, previews): `el` is resized to `width × height` device
 * pixels, cleared to `background`, and `dl` drawn at `scale` device pixels per page px (`null`: background only).
 */
export function paintCanvas(engine: Engine, el: HTMLCanvasElement, width: number, height: number, background: readonly [number, number, number, number], dl: DisplayList | null, scale = 1): void {
  const w = Math.max(1, Math.round(width));
  const h = Math.max(1, Math.round(height));
  const t = new RenderTarget().begin(0, 0, w, h);
  t.clear(...background);
  if (dl) {
    t.scale(scale);
    engine.painter.draw(t, dl);
  }
  const img = t.end();
  if (el.width !== w) el.width = w;
  if (el.height !== h) el.height = h;
  el.getContext('2d')?.putImageData(img, 0, 0);
}
