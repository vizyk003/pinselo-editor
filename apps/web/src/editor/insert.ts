/** Insertion helpers shared by panels, drag-and-drop and paste. */
import type { Editor } from '@oc/engine';
import {
  makeEmbed, makeImage, makeText, parseEmbedUrl, type AssetId, type AssetRef, type DocOp, type ElementId, type FrameEl, type PageId, type RunStyle, type ShapeEl,
  type Vec2,
} from '@oc/engine';
import { api, embedApi, type UploadedAsset } from '../api';
import { tk } from '../i18n';
import { detachPhotoOps, fillOps, findDropTarget, mediaOf, outlineInside, userLocked, withoutPhotoFill, type DropTarget } from './dropTarget';

/** Fit an asset into ~60% of the page, preserving aspect. */
function fitSize(editor: Editor, a: { w: number; h: number }, frac = 0.6) {
  const { w: W, h: H } = editor.doc.meta.dims;
  const s = Math.min((W * frac) / a.w, (H * frac) / a.h, 1.5);
  return { w: Math.max(8, a.w * s), h: Math.max(8, a.h * s) };
}

export const assetRef = (a: UploadedAsset): AssetRef => ({
  id: a.id as AssetId, kind: a.kind, sha256: a.sha256, mime: a.mime, w: a.w, h: a.h, name: a.name,
  ...(a.durationMs ? { durationMs: a.durationMs } : {}), ...(a.hasAudio !== undefined ? { hasAudio: a.hasAudio } : {}), ...(a.animated ? { animated: a.animated } : {}),
});

export interface DropOptions {
  /** Target picked when the drop happened (null: add a new element). Omitted: looked up at `at` now. */
  target?: DropTarget | null;
}

/**
 * Insert an asset at a page point. Dropped onto a shape, frame, grid cell or image it fills or replaces that
 * (see dropTarget.ts; Canva behaviour); otherwise a new image or video element is created.
 */
export function insertAssetAt(editor: Editor, asset: AssetRef, at: Vec2 | undefined, pageId: PageId = editor.state.activePageId, opts: DropOptions = {}) {
  if (asset.kind !== 'audio') {
    const media = mediaOf(asset);
    const target = opts.target !== undefined
      ? opts.target
      : at && media ? findDropTarget(editor.doc, pageId, at, media, { inside: outlineInside(editor.doc) }) : null;
    if (target && fillTarget(editor, target, asset)) return;
  }
  editor.apply([{ op: 'asset', asset }], true);
  if (asset.kind === 'audio') {
    editor.addAudio({ id: asset.id, durationMs: asset.durationMs, name: asset.name }, editor.state.playhead ?? editor.pageStart(pageId));
    return;
  }
  if (asset.kind === 'video') {
    editor.setActivePage(pageId);
    editor.insertVideo({ id: asset.id, w: asset.w || 1280, h: asset.h || 720, durationMs: asset.durationMs, animated: asset.animated }, at);
    return;
  }
  const { w, h } = fitSize(editor, asset);
  editor.insert([makeImage({ xf: { x: 0, y: 0, w, h, rot: 0 }, asset: asset.id, ...(asset.name ? { name: asset.name } : {}) })], at);
}

/**
 * Put an asset into a drop target and select it: fills a shape or frame or a grid cell, or replaces an image. The
 * asset's registration and the fill go in one batch: a single undo step, synced like any edit. False when the
 * target is gone, locked or can't take this asset (the caller then adds it as a new element).
 */
export function fillTarget(editor: Editor, target: DropTarget, asset: AssetRef): boolean {
  const ops = fillOps(editor.doc, target, asset);
  if (!ops || !applyAll(editor, [{ op: 'asset', asset }, ...ops])) return false;
  editor.select([clickTarget(editor, target.id)]);
  return true;
}

/** Applies `ops` as one undo step, but only if every one of them can (the document rejects a batch that hits a lock as a whole). True when all applied. */
export function applyAll(editor: Editor, ops: DocOp[]): boolean {
  const r = editor.apply(ops);
  return r.ok && r.applied === ops.length;
}

// ------------------------------------------------------------------ the photo in a shape or frame

/** Pull the photo out as an image right above the shape or frame (same box, crop and look) and select it. All or nothing. */
export function detachPhoto(editor: Editor, el: ShapeEl | FrameEl): boolean {
  const crop = editor.mediaCrop(el);
  const r = crop ? detachPhotoOps(el, crop) : null;
  if (!r || userLocked(el) || !applyAll(editor, r.ops)) return false;
  editor.select([r.imageId]);
  return true;
}

/** Take the photo out of a shape (back to its colour) or a frame (empty). */
export function removePhoto(editor: Editor, el: ShapeEl | FrameEl): boolean {
  if (userLocked(el)) return false;
  return applyAll(editor, [{ op: 'update', id: el.id, patch: el.type === 'shape' ? { fills: withoutPhotoFill(el) } : { content: null } }]);
}

/**
 * Replace the photo with a file from the device. Never adds a free image: false (nothing changed in the design)
 * when the photo is locked or the file can't go into this target (e.g. a video into a shape).
 */
export async function replacePhoto(editor: Editor, el: ShapeEl | FrameEl, file: File, upload: (f: File) => Promise<{ asset: AssetRef }> = api.upload): Promise<boolean> {
  if (userLocked(el)) return false;
  const { asset } = await upload(file);
  return fillTarget(editor, { kind: el.type, id: el.id }, asset);
}

/** What a click on `id` selects: the element itself, or its outermost group unless that group is entered. */
export function clickTarget(editor: Editor, id: ElementId): ElementId {
  const els = editor.doc.elements;
  let pick = id;
  const seen = new Set<string>([id]);
  for (let p = els[id]?.parentId as ElementId | undefined; p && els[p] && p !== editor.state.enteredGroup && !seen.has(p); p = els[p]!.parentId as ElementId) {
    seen.add(p);
    pick = p;
  }
  return pick;
}

export async function uploadAndInsert(editor: Editor, file: File, at?: Vec2, pageId?: PageId, opts?: DropOptions) {
  const { asset } = await api.upload(file);
  insertAssetAt(editor, asset, at, pageId, opts);
  return asset;
}

export type TextPreset = 'heading' | 'subheading' | 'body';

/** Preset texts are marked with tk(); translate them with t() at insertion time. */
export const TEXT_PRESETS: Record<TextPreset, { text: string; style: Partial<RunStyle> }> = {
  heading: { text: tk('Add a heading'), style: { size: 64, weight: 800 } },
  subheading: { text: tk('Add a subheading'), style: { size: 40, weight: 600 } },
  body: { text: tk('Add a little bit of body text'), style: { size: 24, weight: 400 } },
};

export function addText(editor: Editor, text: string, style: Partial<RunStyle>, opts: { edit?: boolean; at?: Vec2; align?: 'start' | 'center' } = {}) {
  const W = editor.doc.meta.dims.w;
  const size = style.size ?? 32;
  // Scale default sizes to the page so headings look right on both a 500px logo and a 2560px banner.
  const k = Math.max(0.4, Math.min(3, W / 1080));
  const s = { ...style, size: Math.round(size * k) };
  const width = Math.min(W * 0.8, Math.max(s.size * text.length * 0.55, s.size * 4));
  const el = makeText({ xf: { x: 0, y: 0, w: width, h: s.size * 1.3, rot: 0 }, text, style: s, align: opts.align ?? 'center' });
  return editor.insert([el], opts.at, { edit: opts.edit });
}

/** Embed a YouTube/Vimeo/Spotify/… link (A11). Returns false when the URL isn't embeddable. */
export async function insertEmbed(editor: Editor, url: string, at?: Vec2): Promise<boolean> {
  if (!parseEmbedUrl(url)) return false;
  const r = await embedApi.resolve(url);
  if (r.poster) editor.apply([{ op: 'asset', asset: r.poster }], true);
  const { w: W, h: H } = editor.doc.meta.dims;
  const w = Math.min(W * 0.6, H * 0.6 * r.aspect);
  const h = w / r.aspect;
  editor.insert([makeEmbed({ xf: { x: 0, y: 0, w, h, rot: 0 }, url: r.url, provider: r.provider, src: r.src, title: r.title, ...(r.poster ? { poster: r.poster.id } : {}) })], at);
  return true;
}
