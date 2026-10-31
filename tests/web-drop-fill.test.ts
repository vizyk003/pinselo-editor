/** Drag an image onto a shape/frame/cell/image to fill it (apps/web/src/editor/dropTarget.ts + insert.ts). */
import * as Y from 'yjs';
import {
  emptyDesign, makeFrame, makeGrid, makeGroup, makeImage, makeShape, makeStroke, makeText, refitCrop, resolvePage, solid, sortedPages, xf,
  type AssetId, type AssetRef, type DesignDoc, type Element, type ElementId, type PageId, type ShapeEl,
} from '@oc/engine';
import { describe, expect, it } from 'vitest';
import { editorOf, ydocOf } from './engine-doc';
import {
  accepts, detachPhotoOps, dragMedia, fillColorOf, fillOps, findDropTarget, mediaOf, mediaOfMime, withFillColor, withoutPhotoFill, withPhotoFill,
} from '../apps/web/src/editor/dropTarget';
import { applyAll, clickTarget, detachPhoto, fillTarget, insertAssetAt, photoLockOf, removePhoto, replacePhoto } from '../apps/web/src/editor/insert';

const NO = { shift: false, alt: false, meta: false };
const PHOTO: AssetRef = { id: 'photo1' as AssetId, kind: 'image', sha256: 'x', mime: 'image/png', w: 400, h: 200, name: 'p.png' };
const PHOTO2: AssetRef = { ...PHOTO, id: 'photo2' as AssetId, w: 200, h: 400 };
const VIDEO: AssetRef = { id: 'vid1' as AssetId, kind: 'video', sha256: 'v', mime: 'video/mp4', w: 1280, h: 720, durationMs: 4000 };
const SVG: AssetRef = { id: 'svg1' as AssetId, kind: 'svg', sha256: 's', mime: 'image/svg+xml', w: 24, h: 24 };

function setup() {
  const editor = editorOf(emptyDesign({ w: 1000, h: 1000 }));
  const store = editor.store;
  const pageId = sortedPages(store.doc)[0]!.id;
  /** Adds on top of the page (or into `parent`), returns the id. */
  const add = <T extends Element>(el: T, parent: string = pageId): ElementId => {
    editor.apply([{ op: 'add', el: { ...el, parentId: parent } as Element, parent: parent as ElementId }]);
    return el.id;
  };
  const el = <T extends Element = ShapeEl>(id: ElementId) => store.doc.elements[id] as T;
  const target = (p: [number, number], media: 'photo' | 'graphic' | 'video' = 'photo', isBrandLocked?: (e: Element) => boolean) =>
    findDropTarget(store.doc, pageId, p, media, { isBrandLocked });
  return { store, editor, pageId, add, el, target };
}

describe('drop target hit-testing', () => {
  it('hits circles and stars inside their outline only', () => {
    const { add, target } = setup();
    const circle = add(makeShape({ kind: 'ellipse', xf: xf(0, 0, 200, 200) }));
    const star = add(makeShape({ kind: 'star', xf: xf(400, 0, 200, 200) }));
    expect(target([100, 100])).toEqual({ kind: 'shape', id: circle });
    expect(target([8, 8])).toBeNull(); // the circle's box corner
    expect(target([500, 100])).toEqual({ kind: 'shape', id: star });
    expect(target([410, 10])).toBeNull(); // between the star's points
    expect(target([800, 800])).toBeNull(); // empty page
  });

  it('respects z-order, hidden elements, see-through text and locks', () => {
    const { add, target, editor } = setup();
    const below = add(makeShape({ kind: 'rect', xf: xf(0, 0, 300, 300) }));
    const above = add(makeShape({ kind: 'rect', xf: xf(100, 100, 300, 300) }));
    expect(target([200, 200])).toEqual({ kind: 'shape', id: above });
    expect(target([50, 50])).toEqual({ kind: 'shape', id: below });
    editor.apply([{ op: 'update', id: above, patch: { visible: false } }]);
    expect(target([200, 200])).toEqual({ kind: 'shape', id: below });
    editor.apply([{ op: 'update', id: above, patch: { visible: true, lock: { move: true, resize: true, rotate: true, delete: true, by: 'user' } } }]);
    // A locked shape is never filled, and it blocks the drop instead of passing it to the shape below.
    expect(target([200, 200])).toBeNull();
    expect(target([380, 380])).toBeNull();
    add(makeText({ xf: xf(0, 0, 150, 60), text: 'Label' }));
    expect(target([40, 20])).toEqual({ kind: 'shape', id: below });
  });

  it('honours brand locks for non-admins only', () => {
    const { add, target, editor } = setup();
    const s = add(makeShape({ kind: 'rect', xf: xf(0, 0, 200, 200) }));
    editor.apply([{ op: 'update', id: s, patch: { lock: { by: 'brand', style: true } } }]);
    expect(target([100, 100], 'photo', () => true)).toBeNull();
    expect(target([100, 100], 'photo', () => false)).toEqual({ kind: 'shape', id: s });
  });

  it('looks inside groups, but not locked groups or sticky notes', () => {
    const { add, target, editor } = setup();
    const g = add(makeGroup({ xf: xf(100, 100, 400, 400) }));
    const inner = add(makeShape({ kind: 'ellipse', xf: xf(0, 0, 200, 200) }), g);
    expect(target([200, 200])).toEqual({ kind: 'shape', id: inner }); // group offset applies
    expect(target([110, 110])).toBeNull();
    editor.apply([{ op: 'update', id: g, patch: { lock: { move: true, by: 'user' } } }]);
    expect(target([200, 200])).toBeNull();
    editor.apply([{ op: 'update', id: g, patch: { lock: null, board: { sticky: { color: '#ff0' } } } }]);
    expect(target([200, 200])).toBeNull();
  });

  it('follows rotation and flips', () => {
    const { add, target } = setup();
    const r = add(makeShape({ kind: 'rect', xf: xf(400, 300, 200, 100, 90) })); // spans x 450..550, y 250..450
    expect(target([420, 350])).toBeNull();
    expect(target([500, 260])).toEqual({ kind: 'shape', id: r });
    const tri = add(makeShape({ kind: 'right-triangle', xf: { x: 0, y: 600, w: 200, h: 200, rot: 0, flipX: true } }));
    expect(target([180, 750])).toEqual({ kind: 'shape', id: tri }); // inside only because of the flip
    expect(target([20, 750])).toBeNull();
  });

  it('matches media to targets: photos fill shapes; graphics and videos fill frames; cells by index', () => {
    const { add, target, editor } = setup();
    const shape = add(makeShape({ kind: 'rect', xf: xf(0, 0, 200, 200) }));
    const frame = add(makeFrame({ clip: 'ellipse', xf: xf(300, 0, 200, 200) }));
    const img = add(makeImage({ xf: xf(600, 0, 200, 200), asset: 'a' as AssetId }));
    const grid = add(makeGrid({ layout: 'grid-2x2', xf: xf(0, 400, 400, 400), gap: 10 }));
    expect(target([100, 100], 'graphic')).toBeNull();
    expect(target([100, 100], 'video')).toBeNull();
    expect(target([400, 100], 'graphic')).toEqual({ kind: 'frame', id: frame });
    expect(target([400, 100], 'video')).toEqual({ kind: 'frame', id: frame });
    expect(target([700, 100], 'photo')).toEqual({ kind: 'image', id: img });
    expect(target([700, 100], 'video')).toBeNull();
    expect(target([350, 750])).toEqual({ kind: 'cell', id: grid, cell: 3 });
    expect(target([200, 600])).toBeNull(); // the gap between cells
    editor.apply([{ op: 'update', id: frame, patch: { lock: { by: 'brand', content: true } } }]);
    expect(target([400, 100], 'photo', () => true)).toBeNull();
    expect(accepts('shape', 'photo') && !accepts('shape', 'graphic') && !accepts('image', 'video')).toBe(true);
  });

  it('classifies what is dragged', () => {
    expect(mediaOf(PHOTO)).toBe('photo');
    expect(mediaOf(SVG)).toBe('graphic');
    expect(mediaOf(VIDEO)).toBe('video');
    expect(mediaOfMime('image/gif')).toBe('photo');
    expect(mediaOfMime('video/mp4')).toBe('video');
    expect(mediaOfMime('audio/mpeg')).toBeNull();
    expect(dragMedia({ types: ['application/x-oc-asset', 'application/x-oc-media-photo'] })).toBe('photo');
    expect(dragMedia({ types: ['Files'], items: [{ kind: 'file', type: 'image/jpeg' }] })).toBe('photo');
    expect(dragMedia({ types: ['text/plain'] })).toBeNull();
    // A PDF dragged along with a photo: the photo decides (it's the file the drop uses).
    expect(dragMedia({ types: ['Files'], items: [{ kind: 'file', type: 'application/pdf' }, { kind: 'file', type: 'image/png' }] })).toBe('photo');
  });
});

describe('fill ops', () => {
  it('puts a cover-cropped photo above the shape colour and replaces an earlier photo', () => {
    const { add, el } = setup();
    const s = add(makeShape({ kind: 'ellipse', fill: '#ff0000', xf: xf(0, 0, 200, 200) }));
    const [op] = fillOps({ elements: { [s]: el(s) } }, { kind: 'shape', id: s }, PHOTO)!;
    const fills = (op as { patch: { fills: ShapeEl['fills'] } }).patch.fills;
    expect(fills[0]).toEqual(solid('#ff0000'));
    expect(fills[1]).toEqual({ k: 'image', asset: PHOTO.id, fit: 'fill', crop: { x: 0.25, y: 0, w: 0.5, h: 1 } });
    const again = withPhotoFill({ ...el(s), fills }, PHOTO2.id, { x: 0, y: 0, w: 1, h: 1 });
    expect(again.map((p) => (p.k === 'image' ? p.asset : p.k))).toEqual(['solid', PHOTO2.id]);
    expect(withoutPhotoFill({ ...el(s), fills })).toEqual([solid('#ff0000')]);
    expect(withoutPhotoFill({ ...el(s), fills: [fills[1]!] })[0]!.k).toBe('solid');
    expect(fillColorOf({ ...el(s), fills })).toEqual(solid('#ff0000'));
    expect(withFillColor({ ...el(s), fills }, solid('#00ff00'))).toEqual([solid('#00ff00'), fills[1]]);
  });

  it('fills frames (videos get a clip), cells and images, and refuses what a target cannot take', () => {
    const { add, el, store } = setup();
    const f = add(makeFrame({ xf: xf(0, 0, 200, 200) }));
    const g = add(makeGrid({ xf: xf(0, 300, 400, 400) }));
    const i = add({ ...makeImage({ xf: xf(500, 0, 100, 100), asset: 'old' as AssetId }), svgColors: { '#000000': '#ff0000' } });
    const s = add(makeShape({ xf: xf(500, 500, 100, 100) }));
    const patch = (ops: ReturnType<typeof fillOps>) => (ops![0] as { patch: Record<string, unknown> }).patch;
    expect(patch(fillOps(store.doc, { kind: 'frame', id: f }, VIDEO))).toMatchObject({ content: { asset: VIDEO.id, video: { trimEndMs: 4000, loop: false } } });
    expect(patch(fillOps(store.doc, { kind: 'frame', id: f }, PHOTO)).content).not.toHaveProperty('video');
    expect((patch(fillOps(store.doc, { kind: 'cell', id: g, cell: 2 }, PHOTO)).cells as Array<{ asset?: string }>)[2]!.asset).toBe(PHOTO.id);
    expect(patch(fillOps(store.doc, { kind: 'image', id: i }, PHOTO))).toMatchObject({ asset: PHOTO.id, svgColors: null });
    expect(fillOps(store.doc, { kind: 'shape', id: s }, VIDEO)).toBeNull();
    expect(fillOps(store.doc, { kind: 'shape', id: 'gone' as ElementId }, PHOTO)).toBeNull();
    expect(el(s).fills).toHaveLength(1);
  });
});

describe('dropping onto the page (insertAssetAt)', () => {
  it('fills the circle under the point in one undo step, and redoes', () => {
    const { add, el, editor, store, pageId } = setup();
    const c = add(makeShape({ kind: 'ellipse', fill: '#00aa00', xf: xf(100, 100, 300, 300), stroke: makeStroke('#000', 6) }));
    const before = structuredClone(el(c));
    const n = Object.keys(store.doc.elements).length;
    store.checkpoint();
    insertAssetAt(editor, PHOTO, [250, 250], pageId);
    expect(Object.keys(store.doc.elements)).toHaveLength(n); // no new element
    expect(el(c).fills.at(-1)).toMatchObject({ k: 'image', asset: PHOTO.id });
    expect(store.doc.assets[PHOTO.id]).toBeTruthy();
    expect(editor.state.selection).toEqual([c]);
    // One undo step takes the whole drop back (the engine reports undo availability, not depth).
    expect(store.undo.undoStack.length).toBeGreaterThan(0);
    editor.undo();
    expect(el(c)).toEqual(before);
    editor.redo();
    expect(el(c).fills.at(-1)).toMatchObject({ k: 'image', asset: PHOTO.id });
  });

  it('adds a free image on empty page, over locked shapes and for alt-drops (target null)', () => {
    const { add, el, editor, store, pageId } = setup();
    const locked = add({ ...makeShape({ xf: xf(0, 0, 200, 200) }), lock: { move: true, by: 'user' } });
    const images = () => Object.values(store.doc.elements).filter((e) => e.type === 'image' && !e.deleted).length;
    insertAssetAt(editor, PHOTO, [700, 700], pageId);
    expect(images()).toBe(1);
    insertAssetAt(editor, PHOTO, [100, 100], pageId);
    expect(images()).toBe(2);
    expect(el(locked).fills).toHaveLength(1);
    const free = add(makeShape({ xf: xf(500, 0, 200, 200) }));
    insertAssetAt(editor, PHOTO, [600, 100], pageId, { target: null });
    expect(images()).toBe(3);
    expect(el(free).fills).toHaveLength(1);
  });

  it('selects the group (what a click would) when filling a grouped shape', () => {
    const { add, el, editor, pageId } = setup();
    const g = add(makeGroup({ xf: xf(0, 0, 400, 400) }));
    const s = add(makeShape({ xf: xf(0, 0, 200, 200) }), g);
    insertAssetAt(editor, PHOTO, [100, 100], pageId);
    expect(el(s).fills.at(-1)?.k).toBe('image');
    expect(editor.state.selection).toEqual([g]);
    expect(clickTarget(editor, s)).toBe(g);
  });

  it('falls back to a new element when the target cannot take the asset (animated GIF on a shape)', () => {
    const { add, el, editor, store, pageId } = setup();
    const s = add(makeShape({ xf: xf(0, 0, 200, 200) }));
    insertAssetAt(editor, { ...VIDEO, animated: true }, [100, 100], pageId, { target: { kind: 'shape', id: s } });
    expect(el(s).fills).toHaveLength(1);
    expect(Object.values(store.doc.elements).some((e) => e.type === 'video')).toBe(true);
  });

  it('detaches the photo into an image above the shape, same box and crop, one undo step', () => {
    const { add, el, editor, store, pageId } = setup();
    const s = add(makeShape({ kind: 'star', fill: '#123456', xf: xf(100, 100, 300, 300, 30) }));
    insertAssetAt(editor, PHOTO, [250, 250], pageId);
    const crop = editor.mediaCrop(el(s))!;
    const r = detachPhotoOps(el(s), crop)!;
    store.checkpoint();
    editor.apply(r.ops);
    const img = el<Extract<Element, { type: 'image' }>>(r.imageId);
    expect(img).toMatchObject({ type: 'image', asset: PHOTO.id, crop, xf: el(s).xf, parentId: pageId });
    expect(el(s).fills).toEqual([solid('#123456')]);
    editor.undo();
    expect(store.doc.elements[r.imageId]?.deleted ?? store.doc.elements[r.imageId] === undefined).toBeTruthy();
    expect(el(s).fills.at(-1)?.k).toBe('image');
  });
});

describe('photo-filled shapes in the editor and renderer', () => {
  it('crops (double-click), pans the photo and keeps it unstretched on edge resize', () => {
    const { add, el, editor, pageId } = setup();
    const s = add(makeShape({ kind: 'rect', xf: xf(100, 100, 200, 200) }));
    insertAssetAt(editor, PHOTO, [200, 200], pageId); // 2:1 photo in a square: crop {0.25,0,0.5,1}
    editor.doubleClick([200, 200]);
    expect(editor.state.croppingId).toBe(s);
    editor.pointerDown([200, 200], NO);
    editor.pointerMove([250, 200], NO);
    editor.pointerUp();
    const crop = editor.mediaCrop(el(s))!;
    expect(crop.x).toBeCloseTo(0.125); // dragged right → shows more of the left of the photo
    editor.stopCrop();
    editor.select([s]);
    editor.pointerDown([300, 200], NO, { kind: 'handle', handle: 'e' });
    editor.pointerMove([400, 200], NO);
    editor.pointerUp();
    const after = el(s);
    const c = editor.mediaCrop(after)!;
    expect(after.xf.w).toBeCloseTo(300);
    // Shown part of the 2:1 photo has the box's aspect: no stretching.
    expect(((c.w * PHOTO.w) / (c.h * PHOTO.h))).toBeCloseTo(after.xf.w / after.xf.h);
  });

  it('refitCrop keeps aspect, centre and bounds', () => {
    expect(refitCrop({ x: 0.25, y: 0, w: 0.5, h: 1 }, { w: 100, h: 100 }, { w: 200, h: 200 })).toEqual({ x: 0.25, y: 0, w: 0.5, h: 1 });
    const wide = refitCrop({ x: 0.25, y: 0, w: 0.5, h: 1 }, { w: 100, h: 100 }, { w: 150, h: 100 });
    expect(wide.w).toBeCloseTo(0.75);
    expect(wide.x).toBeCloseTo(0.125);
    const huge = refitCrop({ x: 0, y: 0, w: 1, h: 1 }, { w: 100, h: 100 }, { w: 400, h: 100 });
    expect(huge.w).toBeCloseTo(1);
    expect(huge.h).toBeCloseTo(0.25);
    expect(huge.y).toBeCloseTo(0.375);
  });

  it('draws the photo clipped to the outline between the colour and the border (all renderers and exports)', () => {
    const { add, editor, store, pageId } = setup();
    const s = add(makeShape({ kind: 'hexagon', fill: '#ff0000', xf: xf(0, 0, 200, 100), stroke: makeStroke('#000', 4) }));
    insertAssetAt(editor, PHOTO, [100, 50], pageId);
    const items = resolvePage(store.doc, pageId).items.filter((i) => 'srcEl' in i && i.srcEl === s);
    expect(items.map((i) => i.op)).toEqual(['path', 'image', 'path']);
    const [under, photo, over] = items as Array<Extract<(typeof items)[number], { op: 'path' }> & Extract<(typeof items)[number], { op: 'image' }>>;
    expect(under!.fills).toEqual([{ kind: 'solid', color: (solid('#ff0000') as { color: unknown }).color }]);
    expect(photo).toMatchObject({ op: 'image', asset: PHOTO.id, dst: { x: 0, y: 0, w: 200, h: 100 }, crop: { x: 0, y: 0, w: 1, h: 1 }, clip: under!.path });
    expect(over!.fills).toEqual([]);
    expect(over!.stroke?.stroke.width).toBe(4);
    expect(resolvePage(store.doc, pageId).assets.has(PHOTO.id)).toBe(true);
  });

  it('older image fills without a crop draw cover-fit when the asset size is known', () => {
    const shape = { ...makeShape({ xf: xf(0, 0, 100, 100) }), fills: [{ k: 'image' as const, asset: PHOTO.id, fit: 'fill' as const }] };
    /** The photo item the shape draws (a cover crop), or undefined when it is drawn as a paint shader. */
    const drawn = (el: Element, assets: Record<string, AssetRef>) => {
      // Built whole before the engine first sees it (documents are mirrored into Rust on first use).
      const empty = emptyDesign({ w: 200, h: 200 });
      const page = Object.keys(empty.pages)[0]! as PageId;
      const doc: DesignDoc = { ...empty, elements: { [el.id]: { ...el, parentId: page } as Element }, assets: assets as DesignDoc['assets'] };
      const it = resolvePage(doc, page).items.find((i) => i.op === 'image');
      return it?.op === 'image' ? it.crop : undefined;
    };
    expect(drawn(shape, { [PHOTO.id]: PHOTO })).toEqual({ x: 0.25, y: 0, w: 0.5, h: 1 });
    expect(drawn(shape, {})).toBeUndefined(); // unknown size: drawn as a paint shader, as before
    expect(drawn({ ...shape, fills: [{ k: 'image', asset: PHOTO.id, fit: 'tile' }] }, { [PHOTO.id]: PHOTO })).toBeUndefined();
  });
});

/** Shown part of the photo, as width/height in source pixels (equals the box aspect when nothing stretches). */
const shownAspect = (c: { w: number; h: number }, a: AssetRef) => (c.w * a.w) / (c.h * a.h);
const photoItem = (store: { doc: DesignDoc }, pageId: string, id: ElementId) =>
  resolvePage(store.doc, pageId as PageId).items.find((i) => i.op === 'image' && i.srcEl === id) as Extract<ReturnType<typeof resolvePage>['items'][number], { op: 'image' }>;

describe('photo in a locked shape or frame (brand templates, user locks)', () => {
  /** A team member's editor on a brand template: the admin locked it, then the member opens it (setBrandAdmin(false), as EditorPage does). */
  function lockedSetup() {
    const t = setup();
    const shape = t.add(makeShape({ kind: 'ellipse', fill: '#ff0000', xf: xf(0, 0, 200, 200) }));
    const frame = t.add(makeFrame({ clip: 'ellipse', xf: xf(400, 0, 200, 200) }));
    insertAssetAt(t.editor, PHOTO, [100, 100], t.pageId);
    insertAssetAt(t.editor, PHOTO, [500, 100], t.pageId);
    t.editor.select([shape]);
    t.editor.setBrandLock({ style: true });
    t.editor.select([frame]);
    t.editor.setBrandLock({ content: true });
    t.editor.clearSelection();
    t.editor.setBrandAdmin(false);
    t.store.checkpoint();
    return { ...t, shape, frame };
  }

  it('offers nothing that would change the photo, and every action changes nothing', async () => {
    const { editor, store, el, shape, frame, pageId } = lockedSetup();
    const before = JSON.stringify(store.doc.elements);
    const depth = store.undo.undoStack.length;
    expect(photoLockOf(editor, el(shape))).toBe('brand');
    expect(photoLockOf(editor, el<Extract<Element, { type: 'frame' }>>(frame))).toBe('brand');
    expect(fillTarget(editor, { kind: 'shape', id: shape }, PHOTO2)).toBe(false);
    expect(detachPhoto(editor, el(shape))).toBe(false);
    expect(detachPhoto(editor, el<Extract<Element, { type: 'frame' }>>(frame))).toBe(false); // no movable copy of locked content
    expect(removePhoto(editor, el(shape))).toBe(false);
    let uploads = 0;
    const upload = async () => { uploads++; return { asset: PHOTO2 }; };
    expect(await replacePhoto(editor, el(shape), new File([], 'x.png'), upload)).toBe(false);
    expect(await replacePhoto(editor, el<Extract<Element, { type: 'frame' }>>(frame), new File([], 'x.png'), upload)).toBe(false);
    expect(uploads).toBe(0);
    // Not even the asset registration or a stray image: the document and the undo history are untouched.
    expect(JSON.stringify(store.doc.elements)).toBe(before);
    expect(store.doc.assets[PHOTO2.id]).toBeUndefined();
    expect(store.undo.undoStack.length).toBe(depth);
    // A drop onto them adds a separate image instead (it's the member's own element).
    insertAssetAt(editor, PHOTO2, [100, 100], pageId);
    expect(Object.values(store.doc.elements).filter((e) => e.type === 'image' && !e.deleted)).toHaveLength(1);
    expect(el(shape).fills.at(-1)).toMatchObject({ asset: PHOTO.id });
  });

  it('applies a batch whole or not at all', () => {
    const { editor, store, el, shape } = lockedSetup();
    const n = Object.keys(store.doc.elements).length;
    const img = makeImage({ xf: xf(0, 0, 10, 10), asset: PHOTO.id });
    expect(applyAll(editor, [{ op: 'update', id: shape, patch: { fills: [solid('#000000')] } }, { op: 'add', el: { ...img, parentId: el(shape).parentId }, parent: el(shape).parentId }])).toBe(false);
    expect(Object.keys(store.doc.elements)).toHaveLength(n);
  });

  it('admins (and everyone on unlocked photos) can still replace, detach and remove', async () => {
    const { editor, el, shape, frame } = lockedSetup();
    editor.setBrandAdmin(true);
    expect(photoLockOf(editor, el(shape))).toBeNull();
    expect(await replacePhoto(editor, el(shape), new File([], 'x.png'), async () => ({ asset: PHOTO2 }))).toBe(true);
    expect(el(shape).fills.at(-1)).toMatchObject({ asset: PHOTO2.id });
    expect(detachPhoto(editor, el<Extract<Element, { type: 'frame' }>>(frame))).toBe(true);
    expect(removePhoto(editor, el(shape))).toBe(true);
    expect(el(shape).fills.some((p) => p.k === 'image')).toBe(false);
  });

  it("a user's own lock blocks the photo actions too", () => {
    const { add, el, editor, store, pageId } = setup();
    const s = add(makeShape({ xf: xf(0, 0, 200, 200) }));
    insertAssetAt(editor, PHOTO, [100, 100], pageId);
    editor.select([s]);
    editor.toggleLock();
    const n = Object.keys(store.doc.elements).length;
    expect(photoLockOf(editor, el(s))).toBe('user');
    expect(detachPhoto(editor, el(s))).toBe(false);
    expect(removePhoto(editor, el(s))).toBe(false);
    expect(fillTarget(editor, { kind: 'shape', id: s }, PHOTO2)).toBe(false);
    expect(Object.keys(store.doc.elements)).toHaveLength(n);
    expect(el(s).fills.at(-1)).toMatchObject({ asset: PHOTO.id });
  });
});

describe('the photo never stretches, however the shape changes size', () => {
  it('copy style from a photo circle onto a wide rectangle: the photo is refit (cover), not stretched', () => {
    const { add, el, editor, store, pageId } = setup();
    const circle = add(makeShape({ kind: 'ellipse', xf: xf(0, 0, 300, 300) }));
    const wide = add(makeShape({ kind: 'rect', xf: xf(0, 400, 480, 120) }));
    insertAssetAt(editor, PHOTO, [150, 150], pageId);
    editor.select([circle]);
    const style = editor.copyStyle()!;
    editor.pasteStyle([wide], style);
    expect(el(wide).fills.at(-1)).toMatchObject({ k: 'image', asset: PHOTO.id });
    const item = photoItem(store, pageId, wide);
    expect(shownAspect(item.crop, PHOTO)).toBeCloseTo(4);
    expect(item.crop).toEqual({ x: 0, y: expect.closeTo(0.25, 6), w: 1, h: expect.closeTo(0.5, 6) }); // the 4:1 cover crop of a 2:1 photo
    expect(shownAspect(editor.mediaCrop(el(wide))!, PHOTO)).toBeCloseTo(4); // crop mode sees what is drawn
  });

  it('W/H fields (setGeometry), group scaling (scalePatches) and a crop made for another box', () => {
    const { add, el, editor, store, pageId } = setup();
    const s = add(makeShape({ kind: 'rect', xf: xf(0, 0, 200, 200) }));
    insertAssetAt(editor, PHOTO, [100, 100], pageId);
    editor.select([s]);
    editor.setGeometry({ w: 500, h: 300 });
    expect(el(s).xf.w).toBe(500);
    expect(shownAspect(photoItem(store, pageId, s).crop, PHOTO)).toBeCloseTo(5 / 3);

    const g = add(makeGroup({ xf: xf(0, 500, 400, 200) }));
    const inner = add(makeShape({ kind: 'ellipse', xf: xf(0, 0, 200, 200) }), g);
    insertAssetAt(editor, PHOTO, [100, 600], pageId);
    editor.select([g]);
    editor.pointerDown([400, 600], NO, { kind: 'handle', handle: 'e' });
    editor.pointerMove([700, 600], NO);
    editor.pointerUp();
    expect(el(inner).xf.w).toBeCloseTo(350);
    expect(shownAspect(photoItem(store, pageId, inner).crop, PHOTO)).toBeCloseTo(350 / 200);

    // A collaborator resized the box while the drop's 1:1 crop was on its way.
    const c = add(makeShape({ xf: xf(600, 0, 500, 300) }));
    editor.apply([{ op: 'update', id: c, patch: { fills: [{ k: 'image', asset: PHOTO.id, fit: 'fill', crop: { x: 0.25, y: 0, w: 0.5, h: 1 } }] } }]);
    expect(shownAspect(photoItem(store, pageId, c).crop, PHOTO)).toBeCloseTo(5 / 3);
  });
});

describe('detach and undo with a collaborator', () => {
  it('detached photo keeps the opacity, blend mode and effects it had in the shape', () => {
    const { add, el, editor, pageId } = setup();
    const s = add({ ...makeShape({ xf: xf(0, 0, 200, 200) }), opacity: 0.6, blend: 'multiply' as const, effects: [{ k: 'blur' as const, radius: 4 }] });
    insertAssetAt(editor, PHOTO, [100, 100], pageId);
    expect(detachPhoto(editor, el(s))).toBe(true);
    const img = el<Element>(editor.state.selection[0]!);
    expect(img).toMatchObject({ type: 'image', opacity: 0.6, blend: 'multiply', effects: [{ k: 'blur', radius: 4 }] });
  });

  it("undoing a drop after a collaborator recoloured the shape leaves their edit alone (fills is one value; see fillOps)", () => {
    const ya = ydocOf(emptyDesign({ w: 1000, h: 1000 }));
    const yb = new Y.Doc();
    Y.applyUpdate(yb, Y.encodeStateAsUpdate(ya));
    ya.on('update', (u: Uint8Array, o: unknown) => { if (o !== 'remote') Y.applyUpdate(yb, u, 'remote'); });
    yb.on('update', (u: Uint8Array, o: unknown) => { if (o !== 'remote') Y.applyUpdate(ya, u, 'remote'); });
    const A = editorOf(ya, { x: 0, y: 0, zoom: 1, width: 1000, height: 1000 });
    const B = editorOf(yb, { x: 0, y: 0, zoom: 1, width: 1000, height: 1000 });
    const pageId = sortedPages(A.doc)[0]!.id;
    const s = makeShape({ kind: 'ellipse', fill: '#ff0000', xf: xf(0, 0, 200, 200) });
    A.apply([{ op: 'add', el: { ...s, parentId: pageId } as Element, parent: pageId }]);
    A.apply([{ op: 'update', id: s.id, patch: { name: 'kept' } }]);
    insertAssetAt(A, PHOTO, [100, 100], pageId);
    const photo = (B.doc.elements[s.id] as ShapeEl).fills.at(-1)!;
    B.apply([{ op: 'update', id: s.id, patch: { fills: [solid('#00ff00'), photo] } }]);
    A.undo();
    // B's colour and the photo stay; A's earlier edit is not undone instead.
    expect((A.doc.elements[s.id] as ShapeEl).fills).toEqual([solid('#00ff00'), photo]);
    expect(A.doc.elements[s.id]!.name).toBe('kept');
    // "Remove image" still takes just the photo out, keeping B's colour.
    expect(removePhoto(A, A.doc.elements[s.id] as ShapeEl)).toBe(true);
    expect((B.doc.elements[s.id] as ShapeEl).fills).toEqual([solid('#00ff00')]);
  });
});
