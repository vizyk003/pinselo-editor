/**
 * Projects panel (g84), pure parts: bringing a page or elements of another design into the open one. The source
 * document is a read-only snapshot (GET /api/designs/:id/doc); everything here returns plain ops for `editor.apply`,
 * so it can be unit-tested without an editor. (Folder navigation reuses the Projects hub's pages/home/projectsModel.ts.)
 */
import {
  childrenOf, descendants, newId, scalePatches, selectionBounds, templatePages, type AssetRef, type DesignDoc, type DocOp, type Element, type ElementId,
  type LineAttach, type Page, type PageId, type Rect,
} from '@oc/engine';

/** Docs and older sheets keep their content outside the page/element model: they can be opened, not inserted from. */
export function canInsertFrom(surface: string): boolean {
  return surface !== 'doc' && surface !== 'sheet';
}

/** Elements a user can pick one by one: the page's top-level, visible, live elements (groups come whole). */
export function pickableElements(doc: DesignDoc, pageId: PageId): Element[] {
  return childrenOf(doc, pageId).filter((e) => e.visible !== false);
}

/** Asset refs of `src` that the given JSON-able value mentions (element fields, paints, page backgrounds…). */
export function referencedAssets(src: Pick<DesignDoc, 'assets'>, value: unknown): AssetRef[] {
  const json = JSON.stringify(value);
  // Quoted, so one id can't match as a prefix of another; "<id>/…" covers derived keys such as recolour variants.
  return Object.values(src.assets ?? {}).filter((a) => json.includes(`"${a.id}"`) || json.includes(`"${a.id}/`));
}

/** Point references between elements (connector glue, mind-map parents, collapsed sections) at the new ids; drop the ones left dangling. */
function remapRefs(el: Element, map: ReadonlyMap<string, string>): Element {
  let out = el;
  if (out.type === 'line') {
    const fix = (a: LineAttach | undefined) => (a && map.has(a.id) ? { ...a, id: map.get(a.id) as ElementId } : undefined);
    const { startAttach, endAttach, ...rest } = out;
    const s = fix(startAttach);
    const e = fix(endAttach);
    out = { ...rest, ...(s ? { startAttach: s } : {}), ...(e ? { endAttach: e } : {}) } as Element;
  }
  const board = out.board;
  if (board?.mind?.parent || board?.section?.collapsed) {
    const next = { ...board };
    if (board.mind?.parent) next.mind = { ...board.mind, parent: (map.get(board.mind.parent) as ElementId | undefined) ?? null };
    if (board.section?.collapsed) {
      next.section = { ...board.section, collapsed: { ...board.section.collapsed, hidden: board.section.collapsed.hidden.flatMap((id) => (map.has(id) ? [map.get(id) as ElementId] : [])) } };
    }
    out = { ...out, board: next };
  }
  return out;
}

export interface InsertPlan { ops: DocOp[]; created: string[] }

/**
 * "Add page": copy one page of `src` into the open design as a new page after `after`. Content is scaled to fit
 * the open design's page size and centred (same rule as applying a template); all ids are fresh.
 */
export function importPageOps(src: DesignDoc, pageId: PageId, target: { w: number; h: number }, after: PageId | null): InsertPlan | null {
  const page = src.pages[pageId];
  if (!page) return null;
  const one = templatePages({ ...src, pages: { [pageId]: page } as Record<PageId, Page> }, target)[0];
  if (!one) return null;
  // templatePages keeps the order of descendants(), which gives the old → new id map.
  const old = descendants(src, pageId);
  const map = new Map<string, string>();
  if (old.length === one.elements.length) old.forEach((e, i) => map.set(e.id, one.elements[i]!.id));
  const elements = one.elements.map((e) => remapRefs(e, map));
  const ops: DocOp[] = referencedAssets(src, { page: one.page, elements }).map((asset) => ({ op: 'asset', asset }));
  ops.push({ op: 'page', action: 'add', page: one.page, ...(after ? { after } : {}) });
  for (const el of elements) ops.push({ op: 'add', el, parent: el.parentId });
  return { ops, created: [one.page.id] };
}

/**
 * Insert picked top-level elements of `src` onto page `pageId` of the open design. They keep their relative layout;
 * when the group is larger than ~90% of the page it is scaled down, and when it would sit outside the page it is centred.
 * Returns the ops and the new root ids (to select).
 */
export function importElementsOps(src: DesignDoc, ids: readonly ElementId[], target: { w: number; h: number }, pageId: PageId): InsertPlan | null {
  const roots = [...new Set(ids)].map((id) => src.elements[id]).filter((e): e is Element => Boolean(e && !e.deleted && src.pages[e.parentId as PageId]));
  if (!roots.length) return null;
  const box = selectionBounds(src, roots.map((e) => e.id));
  if (!box) return null;
  const s = Math.min(1, (target.w * 0.9) / Math.max(1, box.w), (target.h * 0.9) / Math.max(1, box.h));
  const fitted: Rect = { x: box.x, y: box.y, w: box.w * s, h: box.h * s };
  const inside = fitted.x >= 0 && fitted.y >= 0 && fitted.x + fitted.w <= target.w && fitted.y + fitted.h <= target.h;
  const dx = inside ? 0 : (target.w - fitted.w) / 2 - fitted.x;
  const dy = inside ? 0 : (target.h - fitted.h) / 2 - fitted.y;

  const all: Element[] = [];
  for (const r of roots) all.push(r, ...descendants(src, r.id));
  const map = new Map<string, string>(all.map((e) => [e.id, newId<'el'>() as string]));
  const rootIds = new Set(roots.map((r) => r.id));
  // Text sizes, line points and group children follow the scale (a root's own box is placed below).
  const patches = new Map<string, Partial<Element>>();
  if (s !== 1) for (const r of roots) for (const [id, p] of scalePatches(src, r, s, s)) patches.set(id, p);
  const out: Element[] = [];
  for (const e of all) {
    const isRoot = rootIds.has(e.id);
    let el = { ...structuredClone(e), ...patches.get(e.id), id: map.get(e.id) as ElementId, parentId: (isRoot ? pageId : map.get(e.parentId)) as ElementId } as Element;
    // Roots: scale about the group's top-left, then move onto the page.
    if (isRoot) el = { ...el, xf: { ...e.xf, x: box.x + (e.xf.x - box.x) * s + dx, y: box.y + (e.xf.y - box.y) * s + dy, w: e.xf.w * s, h: e.xf.h * s } };
    out.push(remapRefs(el, map));
  }
  const ops: DocOp[] = referencedAssets(src, out).map((asset) => ({ op: 'asset', asset }));
  for (const el of out) ops.push({ op: 'add', el, parent: el.parentId });
  return { ops, created: roots.map((r) => map.get(r.id)!) };
}
