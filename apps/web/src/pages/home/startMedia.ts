/**
 * Starting a design from an uploaded photo or video (g15): pure helpers shared by Home and the editor's
 * `?place=<assetId>` intent. No React or API imports, so they stay unit-testable.
 */
import type { AssetRef } from '@oc/engine';

/** Longest side of a design started from media. Larger photos are scaled down (the media itself keeps its pixels). */
export const MAX_MEDIA_DESIGN_PX = 8000;

export type StartKind = 'media' | 'import' | null;

/** PDF/PPTX go through the server importer, Markdown/Word into a doc; images and videos start a design around themselves. */
export function startKind(file: { name: string; type: string }): StartKind {
  if (/\.(pdf|pptx|md|markdown|docx)$/i.test(file.name)) return 'import';
  if (/^(image|video)\//.test(file.type)) return 'media';
  if (/\.(png|jpe?g|gif|webp|avif|heic|svg|mp4|mov|webm|m4v)$/i.test(file.name)) return 'media';
  return null;
}

/** Design size (px) matching the media's pixel size, scaled down to fit {@link MAX_MEDIA_DESIGN_PX}. */
export function mediaDesignSize(w: number, h: number, kind: AssetRef['kind']): { w: number; h: number } {
  if (!(w > 0 && h > 0)) return kind === 'video' ? { w: 1920, h: 1080 } : { w: 1080, h: 1080 };
  const s = Math.min(1, MAX_MEDIA_DESIGN_PX / Math.max(w, h));
  return { w: Math.max(1, Math.round(w * s)), h: Math.max(1, Math.round(h * s)) };
}

/** A real video (not an animated GIF) starts on the video surface, everything else as a graphic. */
export const startsVideo = (a: Pick<AssetRef, 'kind' | 'animated'>) => a.kind === 'video' && !a.animated;

/** The design title: the file name without its extension. */
export function titleFromFile(name: string): string {
  return name.replace(/\.[^.]+$/, '').trim().slice(0, 200) || 'Untitled design';
}

/** Box (page coordinates) that covers a W×H page with media of aspect w/h, centred: the media fills the page edge to edge. */
export function coverBox(w: number, h: number, W: number, H: number): { x: number; y: number; w: number; h: number } {
  if (!(w > 0 && h > 0)) return { x: 0, y: 0, w: W, h: H };
  const s = Math.max(W / w, H / h);
  const bw = w * s;
  const bh = h * s;
  return { x: (W - bw) / 2, y: (H - bh) / 2, w: bw, h: bh };
}

/** Rows from GET /api/assets carry nulls for absent fields; an AssetRef wants them omitted. */
export function toAssetRef(row: Record<string, unknown>): AssetRef | null {
  const kind = row.kind as AssetRef['kind'];
  if (typeof row.id !== 'string' || !['image', 'svg', 'video', 'audio'].includes(kind)) return null;
  const ref: AssetRef = {
    id: row.id as AssetRef['id'], kind, sha256: String(row.sha256 ?? ''), mime: String(row.mime ?? ''),
    w: Number(row.w) || 0, h: Number(row.h) || 0,
  };
  if (typeof row.name === 'string' && row.name) ref.name = row.name;
  if (row.animated) ref.animated = true;
  if (typeof row.durationMs === 'number') ref.durationMs = row.durationMs;
  if (typeof row.hasAudio === 'boolean') ref.hasAudio = row.hasAudio;
  if (row.attribution && typeof row.attribution === 'object') ref.attribution = row.attribution as AssetRef['attribution'];
  return ref;
}

/** Strip the one-shot `place` param from a query string, keeping the rest (e.g. `?share=1`). */
export function withoutPlace(search: string): string {
  const p = new URLSearchParams(search);
  p.delete('place');
  const s = p.toString();
  return s ? `?${s}` : '';
}
