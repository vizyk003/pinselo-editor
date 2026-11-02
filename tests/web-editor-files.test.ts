import {
  emptyDesign, makeGroup, makeImage, makeLine, makeShape, makeText, sortedPages, xf, type AssetId, type AssetRef, type DesignDoc, type DocOp, type Element, type ElementId,
  type LineEl, type PageId, type TextEl,
} from '@oc/engine';
import { storeOf } from './engine-doc';
import { describe, expect, it } from 'vitest';
import { fileActions, folderName, intentDialog, withoutIntents } from '../apps/web/src/editor/fileMenu';
import { canInsertFrom, importElementsOps, importPageOps, pickableElements, referencedAssets } from '../apps/web/src/editor/panels/projects';

describe('editor deep links (g88)', () => {
  it('opens Download or Publish as template from the URL', () => {
    expect(intentDialog('?download=1', false)).toBe('export');
    expect(intentDialog('?publishTemplate=1', true)).toBe('publish');
    expect(intentDialog(new URLSearchParams('download=true'), true)).toBe('export');
    expect(intentDialog('?download=1&publishTemplate=1', true)).toBe('export');
  });

  it('ignores switched-off intents, and publishing for people who can’t edit', () => {
    expect(intentDialog('?publishTemplate=1', false)).toBeNull();
    expect(intentDialog('?download=0', true)).toBeNull();
    expect(intentDialog('?download=false', true)).toBeNull();
    expect(intentDialog('?share=1', true)).toBeNull();
    expect(intentDialog('', true)).toBeNull();
  });

  it('drops the one-shot intents and keeps everything else', () => {
    expect(withoutIntents('?download=1&share=1')).toBe('?share=1');
    expect(withoutIntents('?publishTemplate=1')).toBe('');
    expect(withoutIntents('?download=1&publishTemplate=1&place=x')).toBe('?place=x');
    expect(withoutIntents('?share=1')).toBeNull();
    expect(withoutIntents('')).toBeNull();
  });
});

describe('File menu actions follow the API rules (g88)', () => {
  const d = { owner_id: 'u1' };
  it('gives the owner everything', () => {
    expect(fileActions(d, 'owner', 'u1')).toEqual({ star: true, move: true, unfile: true, trash: true });
  });
  it('lets a team admin file but not trash someone else’s design', () => {
    expect(fileActions(d, 'owner', 'u2')).toEqual({ star: true, move: true, unfile: true, trash: false });
  });
  it('lets an editor move into a folder only, and viewers just star', () => {
    expect(fileActions(d, 'edit', 'u2')).toEqual({ star: true, move: true, unfile: false, trash: false });
    expect(fileActions(d, 'comment', 'u2')).toEqual({ star: true, move: false, unfile: false, trash: false });
    expect(fileActions(d, 'view', 'u2')).toEqual({ star: true, move: false, unfile: false, trash: false });
    expect(fileActions(d, 'owner', null).trash).toBe(false);
  });
  it('names the folder only when it is one of mine', () => {
    const folders = [{ id: 'f1', name: 'Clients' }];
    expect(folderName(folders, 'f1')).toBe('Clients');
    expect(folderName(folders, 'someone-elses')).toBeNull();
    expect(folderName(folders, null)).toBeNull();
    expect(folderName(null, 'f1')).toBeNull();
  });
});

// ---------------------------------------------------------------- Projects panel inserts (g84)

const asset = (id: string): AssetRef => ({ id: id as AssetId, kind: 'image', sha256: 'a'.repeat(64), mime: 'image/png', w: 100, h: 100 });

/** A 1000×800 source design: page 1 has a shape, a text, a connector between them, an image, a group and a background photo. */
function source() {
  const store = storeOf(emptyDesign({ w: 1000, h: 800, pages: 2 }));
  const [p1, p2] = sortedPages(store.doc).map((p) => p.id) as [PageId, PageId];
  const box = makeShape({ xf: xf(100, 100, 200, 100) });
  const text = makeText({ xf: xf(50, 600, 400, 60), text: 'Hello', style: { size: 40 } });
  const line = { ...makeLine({ xf: xf(200, 200, 100, 400) }), startAttach: { id: box.id, anchor: 'bottom' as const }, endAttach: { id: text.id, anchor: 'top' as const } } as LineEl;
  const photo = makeImage({ xf: xf(700, 100, 200, 200), asset: 'as_img' as AssetId });
  const group = makeGroup({ xf: xf(600, 400, 300, 200) });
  const g1 = makeShape({ xf: xf(0, 0, 100, 100) });
  const g2 = makeShape({ xf: xf(150, 50, 100, 100) });
  const hidden = makeShape({ xf: xf(0, 0, 10, 10), visible: false });
  const huge = makeShape({ xf: xf(-100, -100, 3000, 1500) });
  const ops: DocOp[] = [
    ...['as_img', 'as_img2', 'as_bg', 'as_unused'].map((id) => ({ op: 'asset' as const, asset: asset(id) })),
    { op: 'page', action: 'update', pageId: p1, patch: { background: [{ k: 'image', asset: 'as_bg' as AssetId, fit: 'fill' }], title: 'Cover' } },
    ...[box, text, line, photo, group, hidden].map((el) => ({ op: 'add' as const, el: el as Element, parent: p1 })),
    { op: 'add', el: g1, parent: group.id },
    { op: 'add', el: g2, parent: group.id },
    { op: 'add', el: huge, parent: p2 },
  ];
  expect(store.apply(ops).ok).toBe(true);
  return { doc: store.doc, p1, p2, box, text, line, photo, group, g1, g2, hidden, huge };
}

function target(w = 1000, h = 800) {
  const store = storeOf(emptyDesign({ w, h }));
  return { store, page: sortedPages(store.doc)[0]!.id };
}

const live = (doc: DesignDoc, pageId: PageId) => Object.values(doc.elements).filter((e) => !e.deleted && e.parentId === pageId);

describe('Projects panel: insert from another design (g84)', () => {
  it('only offers element-based designs for inserting', () => {
    expect(canInsertFrom('presentation')).toBe(true);
    expect(canInsertFrom('doc')).toBe(false);
    expect(canInsertFrom('sheet')).toBe(false);
  });

  it('lists the visible top-level elements of a page', () => {
    const s = source();
    expect(pickableElements(s.doc, s.p1).map((e) => e.id).sort()).toEqual([s.box.id, s.text.id, s.line.id, s.photo.id, s.group.id].sort());
    expect(pickableElements(s.doc, s.p2).map((e) => e.id)).toEqual([s.huge.id]);
  });

  it('finds the assets a value mentions, without prefix collisions', () => {
    const s = source();
    expect(referencedAssets(s.doc, [s.photo]).map((a) => a.id)).toEqual(['as_img']);
    expect(referencedAssets(s.doc, { asset: 'as_img2/recolor/x' }).map((a) => a.id)).toEqual(['as_img2']);
    expect(referencedAssets(s.doc, [s.box])).toEqual([]);
  });

  it('adds a page as a copy, scaled to fit, with fresh ids and connectors re-glued', () => {
    const s = source();
    const t = target(500, 400);
    const plan = importPageOps(s.doc, s.p1, { w: 500, h: 400 }, t.page)!;
    // Assets first (the photo and the background, not the unused one), then the page, then its elements.
    expect(plan.ops.filter((o) => o.op === 'asset').map((o) => (o as { asset: AssetRef }).asset.id).sort()).toEqual(['as_bg', 'as_img']);
    expect(t.store.apply(plan.ops).ok).toBe(true);
    const doc = t.store.doc;
    const pages = sortedPages(doc);
    expect(pages.map((p) => p.id)).toEqual([t.page, plan.created[0]]);
    const newPage = pages[1]!;
    expect(newPage.id).not.toBe(s.p1);
    expect(newPage.title).toBe('Cover');
    expect(newPage.background[0]).toMatchObject({ k: 'image', asset: 'as_bg' });

    const top = live(doc, newPage.id);
    expect(top).toHaveLength(6); // box, text, line, photo, group, hidden (a page copy keeps hidden layers)
    for (const e of top) expect(s.doc.elements[e.id as ElementId]).toBeUndefined();
    const box = top.find((e) => e.type === 'shape' && e.xf.w === 100 && e.xf.h === 50)!;
    expect(box.xf).toMatchObject({ x: 50, y: 50 }); // half size: 1000×800 → 500×400
    const text = top.find((e) => e.type === 'text') as TextEl;
    expect(text.body.defaultRun.size).toBe(20);
    const line = top.find((e) => e.type === 'line') as LineEl;
    expect(line.startAttach?.id).toBe(box.id);
    expect(line.endAttach?.id).toBe(text.id);
    const group = top.find((e) => e.type === 'group')!;
    expect(live(doc, group.id as unknown as PageId)).toHaveLength(2);
  });

  it('inserts after the given page and leaves the source untouched', () => {
    const s = source();
    const before = JSON.stringify(s.doc);
    const t = target();
    t.store.apply([{ op: 'page', action: 'add', after: t.page }]);
    const plan = importPageOps(s.doc, s.p2, { w: 1000, h: 800 }, t.page)!;
    expect(t.store.apply(plan.ops).ok).toBe(true);
    expect(sortedPages(t.store.doc)[1]!.id).toBe(plan.created[0]);
    expect(JSON.stringify(s.doc)).toBe(before);
    expect(importPageOps(s.doc, 'nope' as PageId, { w: 1, h: 1 }, null)).toBeNull();
  });

  it('inserts one element where it was when it fits the page', () => {
    const s = source();
    const t = target();
    const plan = importElementsOps(s.doc, [s.photo.id], { w: 1000, h: 800 }, t.page)!;
    expect(plan.ops.filter((o) => o.op === 'asset').map((o) => (o as { asset: AssetRef }).asset.id)).toEqual(['as_img']);
    expect(t.store.apply(plan.ops).ok).toBe(true);
    const [el] = live(t.store.doc, t.page);
    expect(el!.id).toBe(plan.created[0]);
    expect(el!.id).not.toBe(s.photo.id);
    expect(el!.xf).toMatchObject({ x: 700, y: 100, w: 200, h: 200 });
    expect(t.store.doc.assets['as_img' as AssetId]).toBeDefined();
  });

  it('scales an oversized element down and centres it on the page', () => {
    const s = source();
    const t = target();
    const plan = importElementsOps(s.doc, [s.huge.id], { w: 1000, h: 800 }, t.page)!;
    expect(t.store.apply(plan.ops).ok).toBe(true);
    const [el] = live(t.store.doc, t.page);
    // s = min(900/3000, 720/1500) = 0.3 → 900×450, centred on 1000×800.
    expect(el!.xf.w).toBeCloseTo(900);
    expect(el!.xf.h).toBeCloseTo(450);
    expect(el!.xf.x).toBeCloseTo(50);
    expect(el!.xf.y).toBeCloseTo(175);
  });

  it('brings groups whole, scaling their children with them', () => {
    const s = source();
    const t = target(500, 400);
    const plan = importElementsOps(s.doc, [s.group.id], { w: 500, h: 400 }, t.page)!;
    expect(t.store.apply(plan.ops).ok).toBe(true);
    const group = live(t.store.doc, t.page)[0]!;
    expect(group.type).toBe('group');
    // 300×200 fits 450×360, so no scaling — but at (600,400) it sits off a 500×400 page: centred.
    expect(group.xf).toMatchObject({ x: 100, y: 100, w: 300, h: 200 });
    const kids = live(t.store.doc, group.id as unknown as PageId);
    expect(kids.map((k) => k.xf.x).sort((a, b) => a - b)).toEqual([0, 150]);
  });

  it('keeps connectors glued between elements inserted together and drops dangling glue', () => {
    const s = source();
    const t = target();
    const all = importElementsOps(s.doc, [s.box.id, s.text.id, s.line.id], { w: 1000, h: 800 }, t.page)!;
    expect(all.created).toHaveLength(3);
    expect(t.store.apply(all.ops).ok).toBe(true);
    const els = live(t.store.doc, t.page);
    const line = els.find((e) => e.type === 'line') as LineEl;
    expect(line.startAttach?.id).toBe(els.find((e) => e.type === 'shape')!.id);
    expect(line.endAttach?.id).toBe(els.find((e) => e.type === 'text')!.id);

    const alone = importElementsOps(s.doc, [s.line.id], { w: 1000, h: 800 }, t.page)!;
    const added = alone.ops.find((o) => o.op === 'add') as { el: LineEl };
    expect(added.el.startAttach).toBeUndefined();
    expect(added.el.endAttach).toBeUndefined();
  });

  it('refuses nested or unknown ids', () => {
    const s = source();
    expect(importElementsOps(s.doc, [s.g1.id], { w: 1000, h: 800 }, 'p' as PageId)).toBeNull();
    expect(importElementsOps(s.doc, ['nope' as ElementId], { w: 1000, h: 800 }, 'p' as PageId)).toBeNull();
  });
});
