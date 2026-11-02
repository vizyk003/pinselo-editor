/**
 * Drop targets for media dragged onto the canvas (Canva-style). A photo dropped onto a shape fills it — on top of
 * the shape's colour, clipped to its outline, cover-cropped — and onto a frame or a photo-grid cell fills it; a
 * photo or graphic dropped onto an image replaces it; videos fill frames and grid cells. Anything else, or empty
 * page, gets a new element. Pure (document in, target and ops out) so the hover highlight and the drop agree.
 */
import {
  apply, childrenOf, coverCrop, frameOutline, gridCellBox, hitElement, invert, makeImage, shapePath, solid, worldMatrix, type AssetId, type AssetRef,
  type DesignDoc, type DocOp, type Element, type ElementId, type FrameEl, type GridEl, type MediaClip, type PageId, type Paint, type Rect, type ShapeEl, type Vec2,
} from '@oc/engine';

/** What is dragged: a raster photo, a vector graphic (icons, emoji stickers) or a video (animated GIFs included). */
export type DropMedia = 'photo' | 'graphic' | 'video';
export type DropTarget = { kind: 'shape' | 'frame' | 'image'; id: ElementId } | { kind: 'cell'; id: ElementId; cell: number };
type ImagePaint = Extract<Paint, { k: 'image' }>;
type Doc = Pick<DesignDoc, 'elements' | 'pages'>;

export const ASSET_MIME = 'application/x-oc-asset';
/** Marks a drag with the media it carries: drag data can't be read before the drop, only its types. */
const MEDIA_MIME = 'application/x-oc-media-';

export function markDragMedia(dt: DataTransfer, media: DropMedia | null) {
  if (media) dt.setData(MEDIA_MIME + media, '1');
}

/** Drag data for an asset tile (uploads, brand kit). */
export function setAssetDrag(dt: DataTransfer, a: AssetRef) {
  dt.setData(ASSET_MIME, JSON.stringify(a));
  markDragMedia(dt, mediaOf(a));
}

export function mediaOf(a: Pick<AssetRef, 'kind' | 'mime'>): DropMedia | null {
  if (a.kind === 'video') return 'video';
  if (a.kind === 'svg' || a.mime === 'image/svg+xml') return 'graphic';
  return a.kind === 'image' ? 'photo' : null;
}

/**
 * Desktop files, before upload. A GIF counts as a photo (most fill shapes); one that turns out animated is stored as
 * a video and, if its target only takes photos, is added next to it instead.
 */
export function mediaOfMime(mime: string): DropMedia | null {
  if (mime === 'image/svg+xml') return 'graphic';
  if (mime.startsWith('video/')) return 'video';
  return mime.startsWith('image/') ? 'photo' : null;
}

/**
 * The media being dragged, from what a dragover can see: our marker, else the first desktop file that is media
 * (the drop fills with that one; a PDF dragged along with a photo doesn't hide the highlight).
 */
export function dragMedia(dt: { types: readonly string[]; items?: ArrayLike<{ kind: string; type: string }> }): DropMedia | null {
  for (const m of ['photo', 'graphic', 'video'] as const) if (dt.types.includes(MEDIA_MIME + m)) return m;
  if (dt.types.includes('Files')) {
    for (const it of Array.from(dt.items ?? [])) {
      const media = it.kind === 'file' ? mediaOfMime(it.type) : null;
      if (media) return media;
    }
  }
  return null;
}

/** Which targets take which media. */
export function accepts(kind: DropTarget['kind'], media: DropMedia): boolean {
  if (kind === 'shape') return media === 'photo';
  if (kind === 'image') return media !== 'video';
  return true;
}

export interface DropTargetOptions {
  /** Page-space point-in-element test; defaults to the geometry hit test (bounding box for curved outlines). */
  inside?: (el: Element, p: Vec2) => boolean;
}

/** A drop passes through these to whatever is under them (a shape's label, lines, pen strokes). */
const SEE_THROUGH = new Set<Element['type']>(['text', 'line', 'draw']);

/** The lock in force on an element. A `by: 'brand'` lock comes from a document of the retired brand-template locks and holds nobody. */
export const userLock = (el: Element) => (el.lock?.by === 'brand' ? undefined : el.lock);

export const userLocked = (el: Element) => Boolean(userLock(el));

/**
 * What a drop of `media` at page point `p` fills, or null for "add a new element". The topmost element under the
 * point decides, looking inside groups. Hidden elements are skipped and see-through ones let the drop reach what is
 * below; locked elements, locked groups, whiteboard sections and elements that can't take the media block it.
 */
export function findDropTarget(doc: Doc, pageId: PageId, p: Vec2, media: DropMedia, o: DropTargetOptions = {}): DropTarget | null {
  const inside = o.inside ?? ((el: Element, q: Vec2) => hitElement(doc, el, q));
  const targetOf = (el: Element): DropTarget | null => {
    if (userLocked(el)) return null;
    switch (el.type) {
      case 'shape':
        return media === 'photo' && !el.board?.section ? { kind: 'shape', id: el.id } : null;
      case 'frame':
        return { kind: 'frame', id: el.id };
      case 'image':
        return media === 'video' ? null : { kind: 'image', id: el.id };
      case 'grid': {
        const cell = gridCellAt(doc, el, p);
        return cell < 0 ? null : { kind: 'cell', id: el.id, cell };
      }
      default:
        return null;
    }
  };
  // undefined: nothing here was hit; null: something was hit and blocks the drop.
  const walk = (parentId: PageId | ElementId): DropTarget | null | undefined => {
    const kids = childrenOf(doc, parentId);
    for (let i = kids.length - 1; i >= 0; i--) {
      const el = kids[i]!;
      if (el.visible === false) continue;
      if (el.type === 'group') {
        const r = walk(el.id);
        if (r === undefined) continue;
        return userLocked(el) ? null : r;
      }
      if (SEE_THROUGH.has(el.type) || !inside(el, p)) continue;
      return targetOf(el);
    }
    return undefined;
  };
  return walk(pageId) ?? null;
}

/** Index of the photo-grid cell under page point `p` (-1 in a gap or outside). */
export function gridCellAt(doc: Doc, el: GridEl, p: Vec2): number {
  const [x, y] = apply(invert(worldMatrix(doc, el.id)), p);
  return el.cells.findIndex((c) => {
    const b = gridCellBox(el, c.rect);
    return x >= b.x && y >= b.y && x <= b.x + b.w && y <= b.y + b.h;
  });
}

const rectPath = (r: Rect) => `M${r.x} ${r.y} L${r.x + r.w} ${r.y} L${r.x + r.w} ${r.y + r.h} L${r.x} ${r.y + r.h} Z`;

/** Outline of a drop target in its element's box space (for the hover highlight). */
export function dropOutline(doc: Pick<DesignDoc, 'elements'>, t: DropTarget): string | null {
  const el = doc.elements[t.id];
  if (!el) return null;
  const { w, h } = el.xf;
  if (el.type === 'shape') return shapePath(el.kind, w, h, el.params, el.d);
  if (el.type === 'frame') return frameOutline(el, w, h);
  if (el.type === 'grid' && t.kind === 'cell') {
    const c = el.cells[t.cell];
    return c ? rectPath(gridCellBox(el, c.rect)) : null;
  }
  return rectPath({ x: 0, y: 0, w, h });
}

let pathCtx: CanvasRenderingContext2D | null | undefined;

/** How far a shape's border reaches outside its outline (centre-aligned: half the width; outside: all of it). */
function borderReach(el: Element): number {
  const s = el.type === 'shape' ? el.strokes[0] : undefined;
  if (!s || s.width <= 0) return 0;
  return s.align === 'outside' ? s.width : s.align === 'inside' ? 0 : s.width / 2;
}

/**
 * Exact outline test for the browser (Path2D): curved shapes and frame outlines hit only inside the outline (plus
 * the outer part of a shape's border), not their whole box. Falls back to the geometry hit test where Path2D is
 * missing.
 */
export function outlineInside(doc: Doc): (el: Element, p: Vec2) => boolean {
  pathCtx ??= typeof document !== 'undefined' && typeof Path2D !== 'undefined' ? document.createElement('canvas').getContext('2d') : null;
  const ctx = pathCtx;
  return (el, p) => {
    if (!ctx) return hitElement(doc, el, p);
    if (el.type !== 'shape' && el.type !== 'frame') return hitElement(doc, el, p);
    const reach = borderReach(el);
    if (!hitElement(doc, el, p, reach)) return false;
    const [x, y] = apply(invert(worldMatrix(doc, el.id)), p);
    const path = new Path2D(el.type === 'shape' ? shapePath(el.kind, el.xf.w, el.xf.h, el.params, el.d) : frameOutline(el, el.xf.w, el.xf.h));
    if (ctx.isPointInPath(path, x, y)) return true;
    if (!reach) return false;
    ctx.lineWidth = reach * 2;
    return ctx.isPointInStroke(path, x, y);
  };
}

export const sameTarget = (a: DropTarget | null, b: DropTarget | null) =>
  a === b || Boolean(a && b && a.id === b.id && a.kind === b.kind && (a.kind !== 'cell' || (b.kind === 'cell' && a.cell === b.cell)));

// ------------------------------------------------------------------ filling a target

/** Playback for a video filling a frame or grid cell (as `Editor.fillFrame`): GIFs and stickers loop silently. */
export function videoClipFor(a: Pick<AssetRef, 'kind' | 'durationMs' | 'animated'>): MediaClip | undefined {
  if (a.kind !== 'video') return undefined;
  return { trimStartMs: 0, trimEndMs: a.durationMs || 5000, speed: 1, volume: 1, muted: Boolean(a.animated), loop: Boolean(a.animated) };
}

/**
 * The update that puts `asset` into a drop target, cover-cropped to the box: a shape's photo fill, a frame's content,
 * a grid cell (as `Editor.fillFrame` / `setGridCell` / `replaceImage`). Null when the target is gone or can't take
 * this asset. Applied in one batch with the asset's registration, so the drop is a single undo step.
 *
 * A shape's photo is part of `fills`, which the document stores as one value: when a collaborator changes the shape's
 * colour after the drop, they rewrite `fills`, and undoing the drop then does nothing — collaborative undo never
 * overwrites someone else's newer edit (the same holds for every property). "Remove image" still takes the photo out.
 */
export function fillOps(doc: Pick<DesignDoc, 'elements'>, t: DropTarget, asset: AssetRef): DocOp[] | null {
  const el = doc.elements[t.id];
  const media = mediaOf(asset);
  if (!el || el.deleted || !media || !accepts(t.kind, media)) return null;
  const crop = asset.w > 0 && asset.h > 0 && el.xf.w > 0 && el.xf.h > 0 ? coverCrop(el.xf.w / el.xf.h, asset.w / asset.h) : { x: 0, y: 0, w: 1, h: 1 };
  const video = videoClipFor(asset);
  let patch: Record<string, unknown> | null = null;
  if (t.kind === 'shape' && el.type === 'shape') patch = { fills: withPhotoFill(el, asset.id, crop) };
  else if (t.kind === 'frame' && el.type === 'frame') patch = { content: { asset: asset.id, crop, ...(video ? { video } : {}) } };
  else if (t.kind === 'image' && el.type === 'image') patch = { asset: asset.id, crop, ...(el.svgColors ? { svgColors: null } : {}) }; // recolours belong to the old SVG
  else if (t.kind === 'cell' && el.type === 'grid' && el.cells[t.cell]) {
    patch = { cells: el.cells.map((c, i) => (i === t.cell ? { rect: c.rect, asset: asset.id, ...(video ? { video } : {}) } : c)) };
  }
  return patch ? [{ op: 'update', id: el.id, patch }] : null;
}

// ------------------------------------------------------------------ shapes filled with a photo

/** The photo filling a shape: its topmost image fill. */
export function shapePhotoFill(el: ShapeEl): ImagePaint | undefined {
  return el.fills.findLast((p): p is ImagePaint => p.k === 'image');
}

/** The shape's fills with `asset` as the photo on top (replacing any earlier photo), showing `crop` of it. */
export function withPhotoFill(el: ShapeEl, asset: AssetId, crop: Rect): Paint[] {
  return [...el.fills.filter((p) => p.k !== 'image'), { k: 'image', asset, fit: 'fill', crop }];
}

/** The shape's fills without its photo; a shape that only had the photo gets a neutral grey so it stays visible. */
export function withoutPhotoFill(el: ShapeEl): Paint[] {
  const rest = el.fills.filter((p) => p.k !== 'image');
  return rest.length ? rest : [solid('#d0d7de')];
}

/** The shape's fills with its colour (the paint under the photo) set to `p`; the photo stays on top. */
export function withFillColor(el: ShapeEl, p: Paint): Paint[] {
  const photo = shapePhotoFill(el);
  return photo && p.k !== 'image' ? [p, photo] : [p];
}

/** The colour a shape shows (under its photo, if it has one). */
export const fillColorOf = (el: ShapeEl): Paint | undefined => el.fills.find((p) => p.k !== 'image') ?? el.fills[0];

/**
 * Ops that pull the photo out of a shape or frame into an image element right above it (same box, same visible
 * part, same edits) and leave the shape with its colour (the frame empty). Apply them together: one undo step.
 * `crop` is the part shown now (`Editor.mediaCrop`).
 */
export function detachPhotoOps(el: ShapeEl | FrameEl, crop: Rect): { ops: DocOp[]; imageId: ElementId } | null {
  const asset = el.type === 'shape' ? shapePhotoFill(el)?.asset : el.content && !el.content.video ? el.content.asset : undefined;
  if (!asset) return null;
  const img = makeImage({ xf: { ...el.xf }, asset });
  const edits = el.type === 'frame' ? el.content?.edits : undefined;
  // The photo keeps how it looked in the shape: same opacity, blend mode and effects (shadows, blur, outline).
  const look = { opacity: el.opacity, ...(el.blend ? { blend: el.blend } : {}), ...(el.effects?.length ? { effects: structuredClone(el.effects) } : {}) };
  const image: Element = { ...img, ...look, parentId: el.parentId, crop, ...(edits ? { edits } : {}), ...(el.altText ? { altText: el.altText } : {}) };
  const patch = el.type === 'shape' ? { fills: withoutPhotoFill(el) } : { content: null };
  return { ops: [{ op: 'update', id: el.id, patch }, { op: 'add', el: image, parent: el.parentId, after: el.id }], imageId: img.id };
}
