/**
 * The editor's scene painter: the workspace, page shadows and pages (oc-render through `@oc/engine`) painted into
 * a 2D canvas in fixed tiles.
 *
 * Tiles are anchored to the scene, not the screen: the device transform is `scale(zoom × dpr)` with the scroll
 * offset kept to whole device pixels and applied when a tile is put on the canvas. A tile therefore always
 * rasterises identically, which makes the cheap paths exact:
 *
 * - **pan**: the canvas pixels are shifted and only the exposed strips are painted;
 * - **edits**: each page's new display list is compared with the painted one (`painter.dirtyRegion`) and only the
 *   tiles meeting the changed area are repainted (a drag repaints the union of the old and new boxes);
 * - **zoom / resize**: the last frame is shown scaled at once, then tiles are repainted within a per-frame time
 *   budget (nearest the pointer first), so input stays at 60 fps on heavy pages;
 * - **gestures** (a drag, typing: consecutive frames changing the same element groups): each dirty tile keeps
 *   snapshots of what is below and above the changing groups (`painter.layering`), so a frame only draws the
 *   changing groups between them. When the changing groups only move (a drag: `painter.translation`), they are
 *   painted once into a sprite that is composited at the (whole-pixel) offset, so a drag frame is three blits per
 *   dirty tile. Tiles painted that way are repainted plainly once the gesture stops (cached layers round to 8 bits
 *   and the sprite snaps to device pixels, so they can differ by a level or two and half a pixel).
 *
 * Each tile is painted with a margin (the widest filter reach of the visible pages) so blurs and shadows see their
 * whole input.
 */
import { RenderTarget, pageSlots, type DesignDoc, type DisplayList, type PageId, type Viewport, type raw } from '@oc/engine';
import type { Engine } from '../lib/engine';

export const TILE = 256;
/** Milliseconds of tile painting per frame before the rest waits for the next frame. */
const BUDGET_MS = 10;

export interface ScenePage {
  id: PageId;
  /** Page rectangle in scene units. */
  rect: { x: number; y: number; w: number; h: number };
  dl: DisplayList;
  hidden: boolean;
}

export interface SceneFrame {
  view: DesignDoc;
  port: Viewport;
  dpr: number;
  /** Workspace colour, RGBA 0..1. */
  workspace: readonly [number, number, number, number];
  /** Visible pages. */
  pages: ScenePage[];
  /** Every page slot's id, rectangle and visibility (a change repaints everything). */
  layout: string;
}

interface Painted {
  k: number;
  tx: number;
  ty: number;
  w: number;
  h: number;
  margin: number;
  workspace: string;
  pages: Map<PageId, ScenePage>;
  layout: string;
}

export interface SceneStats {
  /** Tiles painted in the last frame and their total ms. */
  tiles: number;
  ms: number;
  pending: number;
  full: boolean;
  /** Tiles painted from cached layers (a gesture). */
  layered: number;
}

/** A page being changed by a gesture: its groups `from..to` change, the rest is cached per tile under `tag`. */
interface Layered {
  page: PageId;
  from: number;
  to: number;
  tag: string;
  /** Margin (device px) the changing groups need: their own filter reach. */
  margin: number;
}

/** The changing groups of a drag, painted once (u-space device rectangle `x, y, w, h`). */
interface Sprite {
  page: PageId;
  dl: DisplayList;
  from: number;
  to: number;
  k: number;
  x: number;
  y: number;
  snap: raw.Snapshot;
  /** Page-space bounds of the groups when painted. */
  bounds: { x: number; y: number; w: number; h: number };
}

interface TileLayers {
  tag: string;
  below: raw.Snapshot;
  above: raw.Snapshot;
}

/** Tiles whose cached layers are kept (LRU), ~0.5 MB each. */
const LAYER_CACHE = 96;

export class ScenePainter {
  private readonly ctx: CanvasRenderingContext2D;
  private readonly target = new RenderTarget();
  private painted: Painted | null = null;
  /** Tiles (u-space indices `i,j`) still to paint. */
  private pending = new Set<string>();
  /**
   * Tiles that must be painted this frame whatever it costs (edits and scrolls: a lagging drag tears); the rest of
   * `pending` (a full repaint after a zoom or resize) is painted within the budget.
   */
  private urgent = new Set<string>();
  private copy: HTMLCanvasElement | null = null;
  /** Tile painting nearest this device point first (the pointer). */
  focus: [number, number] | null = null;
  last: SceneStats = { tiles: 0, ms: 0, pending: 0, full: false, layered: 0 };
  /** The layering seen in the previous frame (a second frame with the same one starts a gesture). */
  private lastLayering: string | null = null;
  private layered: Layered | null = null;
  private tileLayers = new Map<string, TileLayers>();
  /** Tiles painted from cached layers, repainted plainly when the gesture stops. */
  private approx = new Set<string>();
  private lastChange = 0;
  private sprite: Sprite | null = null;
  /** This frame's whole-pixel offset of the sprite, when the changing groups are the sprite's moved. */
  private spriteAt: [number, number] | null = null;
  /** The previous frame's changing groups (a sprite is only made once they are seen to just move). */
  private prevMid: { page: PageId; dl: DisplayList; from: number; to: number } | null = null;

  constructor(private readonly canvas: HTMLCanvasElement, private readonly engine: Engine) {
    this.ctx = canvas.getContext('2d', { alpha: false, desynchronized: true }) as CanvasRenderingContext2D;
  }

  /** Repaint everything on the next frame (images or fonts arrived). */
  invalidate(): void {
    if (this.painted) this.painted.workspace = '';
  }

  private videoDirty = false;

  /** Repaint the videos on the next frame (a paused video seeked or loaded a frame: same list, new pixels). */
  invalidateVideos(): void {
    this.videoDirty = true;
  }

  /** Whether tiles are still waiting for a frame. */
  get busy(): boolean {
    return this.pending.size > 0;
  }

  /** Paint a frame. Returns true when there is more to do (call again next frame). */
  frame(f: SceneFrame): boolean {
    const t0 = performance.now();
    const W = this.canvas.width;
    const H = this.canvas.height;
    const k = f.port.zoom * f.dpr;
    const tx = Math.round(-f.port.x * k);
    const ty = Math.round(-f.port.y * k);
    let reach = 0;
    for (const p of f.pages) reach = Math.max(reach, this.engine.painter.filterReach(p.dl));
    const margin = Math.ceil((reach * k + 2) / 16) * 16;
    const layout = f.layout;
    const ws = f.workspace.join(',');
    const prev = this.painted;
    let full = false;
    let layered: Layered | null = null;
    const changedSomething = { v: false };
    if (!prev || prev.k !== k || prev.w !== W || prev.h !== H || prev.margin !== margin || prev.workspace !== ws || prev.layout !== layout) {
      full = true;
      this.tileLayers.clear();
      this.approx.clear();
      this.lastLayering = null;
      if (prev && prev.k !== k && prev.workspace) this.showScaled(prev, k, tx, ty);
      this.pending.clear();
      this.urgent.clear();
      this.markDevice(0, 0, W, H, tx, ty, false);
    } else {
      if (prev.tx !== tx || prev.ty !== ty) this.scroll(prev.tx, prev.ty, tx, ty, W, H);
      for (const p of f.pages) {
        const old = prev.pages.get(p.id);
        if (!old || old.dl === p.dl) continue;
        const r = this.engine.painter.dirtyRegion(old.dl, p.dl);
        if (r === null) continue;
        changedSomething.v = true;
        // One page changing the same groups as in the previous frame: a gesture, painted in layers.
        const l = !p.hidden && r !== 'full' ? this.engine.painter.layering(old.dl, p.dl) : null;
        const id = l ? `${p.id}|${l.from}|${l.to}|${l.below}|${l.above}` : null;
        if (l && id && !layered) {
          if (this.lastLayering === id) layered = { page: p.id, from: l.from, to: l.to, tag: `${id}|${k}|${margin}|${ws}|${layout}`, margin: Math.min(margin, Math.ceil((l.reach * k + 2) / 16) * 16) };
          this.lastLayering = id;
        } else if (!l) this.lastLayering = null;
        const a = r === 'full' ? { x: 0, y: 0, w: p.rect.w, h: p.rect.h } : r;
        // Scene → device (whole pixels, plus the anti-aliasing fringe).
        const x0 = Math.floor((p.rect.x + a.x) * k) + tx - 2;
        const y0 = Math.floor((p.rect.y + a.y) * k) + ty - 2;
        const x1 = Math.ceil((p.rect.x + a.x + a.w) * k) + tx + 2;
        const y1 = Math.ceil((p.rect.y + a.y + a.h) * k) + ty + 2;
        this.markDevice(x0, y0, x1 - x0, y1 - y0, tx, ty);
      }
    }
    if (this.videoDirty && !full) {
      this.videoDirty = false;
      for (const p of f.pages) {
        for (const b of this.engine.painter.videoBounds(p.dl)) {
          const x0 = Math.floor((p.rect.x + b.x) * k) + tx - 2;
          const y0 = Math.floor((p.rect.y + b.y) * k) + ty - 2;
          this.markDevice(x0, y0, Math.ceil(b.w * k) + 4, Math.ceil(b.h * k) + 4, tx, ty);
        }
      }
    }
    if (changedSomething.v) this.lastChange = t0;
    if (!full && !changedSomething.v && this.approx.size && !this.pending.size && t0 - this.lastChange > 250) {
      // The gesture stopped: repaint the tiles painted from cached layers, plainly.
      for (const key of this.approx) this.pending.add(key);
      // Repainting plainly is housekeeping: within the budget.
      this.approx.clear();
      for (const c of this.tileLayers.values()) {
        c.below.free();
        c.above.free();
      }
      this.tileLayers.clear();
      this.lastLayering = null;
      this.layered = null;
    } else this.layered = layered ?? (changedSomething.v ? null : this.layered);
    this.spriteAt = this.layered && changedSomething.v ? this.spriteFor(f, this.layered, k) : this.layered ? this.spriteAt : null;
    if (!this.layered && this.sprite) {
      this.sprite.snap.free();
      this.sprite = null;
    }
    if (!this.layered) this.prevMid = null;
    this.painted = { k, tx, ty, w: W, h: H, margin, workspace: ws, pages: new Map(f.pages.map((p) => [p.id, p])), layout };
    // Paint pending tiles, nearest the focus first, within the budget.
    let n = 0;
    let nl = 0;
    if (this.pending.size) {
      const [fx, fy] = this.focus ?? [W / 2, H / 2];
      const order = [...this.pending].map((key) => {
        const [i, j] = key.split(',').map(Number) as [number, number];
        const cx = tx + i * TILE + TILE / 2;
        const cy = ty + j * TILE + TILE / 2;
        return { key, i, j, d: (cx - fx) ** 2 + (cy - fy) ** 2 };
      }).sort((a, b) => a.d - b.d);
      for (const t of order) {
        if (n > 0 && !this.urgent.has(t.key) && performance.now() - t0 > BUDGET_MS) continue;
        this.pending.delete(t.key);
        this.urgent.delete(t.key);
        // Off screen (scrolled away while pending): nothing to do.
        const dx = tx + t.i * TILE;
        const dy = ty + t.j * TILE;
        if (dx >= W || dy >= H || dx + TILE <= 0 || dy + TILE <= 0) continue;
        if (this.layered && this.paintLayered(f, this.layered, t.key, t.i, t.j, k, margin, dx, dy)) nl++;
        else {
          this.approx.delete(t.key);
          this.paintTile(f, t.i, t.j, k, margin, dx, dy);
        }
        n++;
      }
    }
    this.last = { tiles: n, ms: performance.now() - t0, pending: this.pending.size, full, layered: nl };
    // Keep frames coming while tiles wait, and until layered tiles have been repainted plainly.
    return this.pending.size > 0 || this.approx.size > 0;
  }

  /** Mark the tiles meeting a device rectangle (`urgent`: painted this frame, see {@link urgent}). */
  private markDevice(x: number, y: number, w: number, h: number, tx: number, ty: number, urgent = true) {
    const W = this.canvas.width;
    const H = this.canvas.height;
    const x0 = Math.max(0, x);
    const y0 = Math.max(0, y);
    const x1 = Math.min(W, x + w);
    const y1 = Math.min(H, y + h);
    if (x1 <= x0 || y1 <= y0) return;
    for (let j = Math.floor((y0 - ty) / TILE); j <= Math.floor((y1 - 1 - ty) / TILE); j++) {
      for (let i = Math.floor((x0 - tx) / TILE); i <= Math.floor((x1 - 1 - tx) / TILE); i++) {
        this.pending.add(`${i},${j}`);
        if (urgent) this.urgent.add(`${i},${j}`);
      }
    }
  }

  /** Shift the painted pixels by a scroll and mark the exposed strips. */
  private scroll(ptx: number, pty: number, tx: number, ty: number, W: number, H: number) {
    const dx = tx - ptx;
    const dy = ty - pty;
    if (Math.abs(dx) >= W || Math.abs(dy) >= H) {
      this.markDevice(0, 0, W, H, tx, ty);
      return;
    }
    this.ctx.drawImage(this.canvas, dx, dy);
    if (dx > 0) this.markDevice(0, 0, dx, H, tx, ty);
    if (dx < 0) this.markDevice(W + dx, 0, -dx, H, tx, ty);
    if (dy > 0) this.markDevice(0, 0, W, dy, tx, ty);
    if (dy < 0) this.markDevice(0, H + dy, W, -dy, tx, ty);
  }

  /** While a zoom repaints tile by tile: the last frame scaled into place. */
  private showScaled(prev: Painted, k: number, tx: number, ty: number) {
    const W = this.canvas.width;
    const H = this.canvas.height;
    this.copy ??= document.createElement('canvas');
    if (this.copy.width !== W || this.copy.height !== H) {
      this.copy.width = W;
      this.copy.height = H;
    }
    const cc = this.copy.getContext('2d')!;
    cc.drawImage(this.canvas, 0, 0);
    const s = k / prev.k;
    // Scene point p is at prev.t + p·prev.k before and at t + p·k now.
    this.ctx.save();
    this.ctx.fillStyle = `rgb(${prev.workspace.split(',').slice(0, 3).map((v) => Math.round(+v * 255)).join(',')})`;
    this.ctx.fillRect(0, 0, W, H);
    this.ctx.setTransform(s, 0, 0, s, tx - prev.tx * s, ty - prev.ty * s);
    this.ctx.imageSmoothingQuality = 'low';
    this.ctx.drawImage(this.copy, 0, 0);
    this.ctx.restore();
  }

  private paintTile(f: SceneFrame, i: number, j: number, k: number, m: number, dx: number, dy: number) {
    const t = this.target;
    t.begin(i * TILE - m, j * TILE - m, TILE + 2 * m, TILE + 2 * m);
    this.drawScene(t, f, i, j, k, m, { mode: 'all' });
    this.ctx.putImageData(t.endCrop(m, m, TILE, TILE), dx, dy);
  }

  /**
   * A tile of a gesture: the cached layers below and above the changing groups (made on first use), the changing
   * groups drawn between them. False when the tile does not show the changing page (painted plainly instead).
   */
  private paintLayered(f: SceneFrame, l: Layered, key: string, i: number, j: number, k: number, m: number, dx: number, dy: number): boolean {
    const page = f.pages.find((p) => p.id === l.page);
    if (!page) return false;
    const t = this.target;
    let cached = this.tileLayers.get(key);
    if (cached && cached.tag !== l.tag) cached = undefined;
    if (!cached) {
      t.begin(i * TILE - m, j * TILE - m, TILE + 2 * m, TILE + 2 * m);
      this.drawScene(t, f, i, j, k, m, { mode: 'below', page: l.page, from: l.from });
      const below = t.snapshot(m, m, TILE, TILE);
      t.end();
      t.begin(i * TILE - m, j * TILE - m, TILE + 2 * m, TILE + 2 * m);
      this.drawScene(t, f, i, j, k, m, { mode: 'above', page: l.page, from: l.to });
      const above = t.snapshot(m, m, TILE, TILE);
      t.end();
      cached = { tag: l.tag, below, above };
      this.tileLayers.delete(key);
      this.tileLayers.set(key, cached);
      if (this.tileLayers.size > LAYER_CACHE) {
        const oldest = this.tileLayers.keys().next().value as string;
        this.tileLayers.get(oldest)?.below.free();
        this.tileLayers.get(oldest)?.above.free();
        this.tileLayers.delete(oldest);
      }
    }
    // The changing groups need only their own filter reach around the tile (usually none).
    const mm = l.margin;
    t.begin(i * TILE - mm, j * TILE - mm, TILE + 2 * mm, TILE + 2 * mm);
    t.compose(cached.below, mm, mm);
    const sp = this.sprite;
    if (sp && this.spriteAt) t.compose(sp.snap, sp.x + this.spriteAt[0] - (i * TILE - mm), sp.y + this.spriteAt[1] - (j * TILE - mm));
    else this.drawScene(t, f, i, j, k, mm, { mode: 'mid', page: l.page, from: l.from, to: l.to });
    t.compose(cached.above, mm, mm);
    this.ctx.putImageData(t.endCrop(mm, mm, TILE, TILE), dx, dy);
    this.approx.add(key);
    return true;
  }

  /**
   * The offset at which the drag sprite shows this frame's changing groups (painting the sprite on first use), or
   * null when they are not the sprite's moved, or would leave the page (its clip does not move with them).
   */
  private spriteFor(f: SceneFrame, l: Layered, k: number): [number, number] | null {
    const page = f.pages.find((p) => p.id === l.page);
    if (!page) return null;
    const painter = this.engine.painter;
    let sp = this.sprite;
    let d: [number, number] | null = null;
    if (sp && sp.page === l.page && sp.k === k) d = painter.translation(sp.dl, [sp.from, sp.to], page.dl, [l.from, l.to]);
    if (!d) {
      sp?.snap.free();
      this.sprite = sp = null;
      // Only a move gets a sprite: typing, resizing or recolouring change the groups every frame.
      const pm = this.prevMid;
      this.prevMid = { page: l.page, dl: page.dl, from: l.from, to: l.to };
      if (!pm || pm.page !== l.page || !painter.translation(pm.dl, [pm.from, pm.to], page.dl, [l.from, l.to])) return null;
      const b = painter.groupsBounds(page.dl, l.from, l.to);
      if (!b) return null;
      const reach = painter.filterReach(page.dl);
      const pad = Math.ceil(reach * k) + 2;
      const x = Math.floor((page.rect.x + b.x) * k) - pad;
      const y = Math.floor((page.rect.y + b.y) * k) - pad;
      const w = Math.ceil((page.rect.x + b.x + b.w) * k) + pad - x;
      const h = Math.ceil((page.rect.y + b.y + b.h) * k) + pad - y;
      if (w * h > 16e6 || w <= 0 || h <= 0) return null;
      const t = this.target;
      t.begin(x, y, w, h);
      t.scale(k);
      t.translate(page.rect.x, page.rect.y);
      painter.drawGroups(t, page.dl, l.from, l.to, false);
      t.translate(-page.rect.x, -page.rect.y);
      t.scale(1 / k);
      const snap = t.snapshot(0, 0, w, h);
      t.end();
      this.sprite = sp = { page: l.page, dl: page.dl, from: l.from, to: l.to, k, x, y, snap, bounds: b };
      d = [0, 0];
    }
    if (!sp) return null;
    // The page clips the groups: a sprite moved across the page edge would not be.
    const b = sp.bounds;
    const r = page.rect;
    if (b.x + d[0] < 0 || b.y + d[1] < 0 || b.x + b.w + d[0] > r.w || b.y + b.h + d[1] > r.h) return null;
    return [Math.round(d[0] * k), Math.round(d[1] * k)];
  }

  /**
   * Draw the scene into a begun tile. `all`: everything. For a gesture on `page`: `below` = the workspace, the
   * other pages and `page` up to group `from`; `mid` = `page`'s groups `from..to` (over the composed below);
   * `above` = `page`'s groups from `from` on, on transparent.
   */
  private drawScene(t: RenderTarget, f: SceneFrame, i: number, j: number, k: number, m: number, o: { mode: 'all' | 'below' | 'mid' | 'above'; page?: PageId; from?: number; to?: number }) {
    const ux = i * TILE;
    const uy = j * TILE;
    if (o.mode === 'all' || o.mode === 'below') t.clear(...f.workspace);
    t.scale(k);
    // The tile in scene units (with the margin), to skip pages and dots outside it.
    const s0x = (ux - m) / k;
    const s0y = (uy - m) / k;
    const s1x = (ux + TILE + m) / k;
    const s1y = (uy + TILE + m) / k;
    const zoom = f.port.zoom;
    const shadowReach = 6 / zoom * 3 + 2 / zoom;
    const painter = this.engine.painter;
    for (const p of f.pages) {
      const target = o.page === p.id;
      if ((o.mode === 'mid' || o.mode === 'above') && !target) continue;
      const r = p.rect;
      if (r.x - shadowReach > s1x || r.y - shadowReach > s1y || r.x + r.w + shadowReach < s0x || r.y + r.h + shadowReach < s0y) continue;
      if (o.mode === 'all' || o.mode === 'below') t.shadowRect(r.x, r.y + 2 / zoom, r.w, r.h, 6 / zoom, [0, 0, 0, 0.12], p.dl.raw.opaque && !p.hidden ? r : undefined);
      if (r.x > s1x || r.y > s1y || r.x + r.w < s0x || r.y + r.h < s0y) continue;
      t.save();
      t.translate(r.x, r.y);
      // Tiles inside the page need no clip (an anti-aliased clip mask slows every fill down).
      const inside = r.x <= s0x && r.y <= s0y && r.x + r.w >= s1x && r.y + r.h >= s1y;
      if (!inside) t.clipRect(0, 0, r.w, r.h);
      if (p.hidden) t.saveLayer(0.4);
      const n = painter.groupCount(p.dl);
      const [g0, g1] = !target || o.mode === 'all' ? [0, n] : o.mode === 'below' ? [0, o.from!] : o.mode === 'mid' ? [o.from!, o.to!] : [o.from!, n];
      const background = !target || o.mode === 'all' || o.mode === 'below';
      if (background) painter.drawGroups(t, p.dl, 0, 0, true);
      if (g1 > g0) painter.drawGroups(t, p.dl, g0, g1, false);
      if (p.hidden) t.restore();
      t.restore();
    }
    // Back to the tile's own transform (compose needs no saves in effect).
    t.scale(1 / k);
  }

  /** One pixel of a page (exactly what the design shows, without editor chrome), as `#rrggbb`. */
  samplePixel(dl: DisplayList, x: number, y: number): string {
    const t = new RenderTarget().begin(0, 0, 1, 1);
    t.clear(1, 1, 1, 1);
    t.translate(-x, -y);
    this.engine.painter.draw(t, dl);
    const px = t.end().data;
    return `#${[px[0]!, px[1]!, px[2]!].map((v) => v.toString(16).padStart(2, '0')).join('')}`;
  }
}

/** The visible page slots of a view as scene pages (display lists from `dlFor`), and the layout key of all slots. */
export function scenePages(view: DesignDoc, port: Viewport, dlFor: (pageId: PageId) => DisplayList): { pages: ScenePage[]; layout: string } {
  // Shadows reach a little outside the page.
  const pad = 30 / port.zoom;
  const visible = { x0: port.x - pad, y0: port.y - pad, x1: port.x + port.width / port.zoom + pad, y1: port.y + port.height / port.zoom + pad };
  const pages: ScenePage[] = [];
  const keys: string[] = [];
  for (const slot of pageSlots(view)) {
    const r = slot.rect;
    keys.push(`${slot.page.id}:${r.x},${r.y},${r.w},${r.h}:${slot.page.hidden ? 1 : 0}`);
    if (r.y > visible.y1 || r.y + r.h < visible.y0 || r.x > visible.x1 || r.x + r.w < visible.x0) continue;
    pages.push({ id: slot.page.id, rect: { x: r.x, y: r.y, w: r.w, h: r.h }, dl: dlFor(slot.page.id), hidden: Boolean(slot.page.hidden) });
  }
  return { pages, layout: keys.join('|') };
}
