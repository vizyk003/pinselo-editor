/**
 * The editing surface: the scene painted by oc-render in tiles (scene.ts) into a 2D canvas, with an SVG overlay for
 * chrome (selection, handles, guides, caret, presence). docs/plan/03 §5.1.
 */
import { ANCHORS, anchorPoint, apply, hitTest, multiply, worldMatrix, worldQuad } from '@oc/engine';
import { pageSlots, screenToScene, type Editor, type Handle, type HitTarget, type Mods, type NodePart } from '@oc/engine';
import { VectorOverlay } from './VectorOverlay';
import { pageSettleMs, resolvePage, smoothPath, tableGeometry, type DisplayList } from '@oc/engine';
import { cellRect, childrenOf, locateTime, sortedPages, toCss, type AssetRef, type DesignDoc, type Element, type ElementId, type Mat2D, type PageId, type TableEl, type TextEl, type Vec2 } from '@oc/engine';
import { Lock } from 'lucide-react';
import { useCallback, useEffect, useMemo, useRef, useState, type PointerEvent as RPointerEvent } from 'react';
import { Popover, useToast } from '../components/ui';
import { ElementDialogs, ElementMenu } from './ElementMenu';
import { ImageEraserHost } from './ImageEraser';
import { PageHeaders, QuickBar } from './PageControls';
import { useEd, useEditorState, useView, type Presence } from './context';
import { blocksBareShortcut } from '../lib/appearance';
import { canvasTabAction } from './keyRouting';
import { insertAssetAt, uploadAndInsert } from './insert';
import { GRAPHIC_MIME, insertGraphic } from './GraphicsPanel';
import { poolFor } from './media';
import { ScenePainter, scenePages } from './scene';
import { rgba } from '../lib/engine';
import { STOCK_MIME, stockMedia } from './LibraryPanel';
import { ASSET_MIME, dragMedia, dropOutline, findDropTarget, mediaOf, mediaOfMime, outlineInside, sameTarget, type DropMedia, type DropTarget } from './dropTarget';
import { contentApi, type StockKind } from '../api';
import { Rulers } from './Rulers';
import { BLEED_PX, marginOf, useViewPrefs } from './viewPrefs';
import { t } from '../i18n';
import { WhiteboardLayer } from './Whiteboard';

const mods = (e: { shiftKey: boolean; altKey: boolean; metaKey: boolean; ctrlKey: boolean }): Mods => ({
  shift: e.shiftKey, alt: e.altKey, meta: e.metaKey || e.ctrlKey,
});

function cssVar(name: string, fallback: string): string {
  return getComputedStyle(document.documentElement).getPropertyValue(name).trim() || fallback;
}


export function Canvas({ presence, commentPins }: { presence: Presence[]; commentPins: Map<string, number> }) {
  const { editor, engine, readOnly, provider } = useEd();
  const state = useEditorState();
  const view = useView();
  const toast = useToast();
  const wrapRef = useRef<HTMLDivElement>(null);
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const imeRef = useRef<HTMLTextAreaElement>(null);
  const sceneRef = useRef<ScenePainter | null>(null);
  const dirty = useRef(true);
  const dlCache = useRef(new Map<PageId, { v: DesignDoc; dl: DisplayList; editing: ElementId | null }>());
  const prefs = useViewPrefs();
  const pan = useRef<{ x: number; y: number } | null>(null);
  const spaceDown = useRef(false);
  // Touch: two fingers pinch-zoom and pan; long-press opens the context menu; double-tap = double-click.
  const touches = useRef(new Map<number, { x: number; y: number }>());
  const pinch = useRef<{ dist: number; mid: [number, number] } | null>(null);
  const touchBlocked = useRef(false);
  const longPress = useRef<{ timer: number; x: number; y: number } | null>(null);
  const lastTap = useRef({ t: 0, x: 0, y: 0 });
  const coarse = useMemo(() => typeof matchMedia === 'function' && matchMedia('(pointer: coarse)').matches, []);
  const [menu, setMenu] = useState<{ x: number; y: number } | null>(null);
  // A state ref, not useRef: the Popover needs the anchor element, which exists only after the render that opens
  // the menu, so attaching it must render again (CM-02: right-click on an already-selected element showed nothing).
  const [menuAnchor, setMenuAnchor] = useState<HTMLDivElement | null>(null);
  const [dropping, setDropping] = useState(false);
  // Shape, frame, grid cell or image a drag would fill (highlighted while dragging).
  const [dropTarget, setDropTarget] = useState<DropTarget | null>(null);
  const composing = useRef(false);
  const vp = state.viewport;

  // ---------------------------------------------------------------- surface + render loop
  const makeSurface = useCallback(() => {
    const el = canvasRef.current;
    const wrap = wrapRef.current;
    if (!el || !wrap) return;
    const dpr = window.devicePixelRatio || 1;
    const w = wrap.clientWidth;
    const h = wrap.clientHeight;
    const pw = Math.max(1, Math.round(w * dpr));
    const ph = Math.max(1, Math.round(h * dpr));
    // Assigning width/height clears the canvas even when the value is unchanged; the painter only repaints on a real
    // size change, so a same-size resize callback used to leave the cleared tiles black until the next edit or zoom.
    if (el.width !== pw) el.width = pw;
    if (el.height !== ph) el.height = ph;
    // One scene painter per canvas for its whole life; a resize repaints it.
    sceneRef.current ??= new ScenePainter(el, engine);
    // The page was fitted (whole and centred) before the canvas area changed size — the timeline or notes opened or
    // closed, a panel collapsed, the window resized: fit it again rather than leave it cut off or off-centre.
    const before = editor.state.viewport;
    const refit = (Math.abs(before.width - w) > 1 || Math.abs(before.height - h) > 1) && pageFitted(editor);
    editor.setViewportSize(w, h);
    if (refit) editor.zoomToFit();
    dirty.current = true;
  }, [editor, engine]);

  useEffect(() => {
    const wrap = wrapRef.current!;
    makeSurface();
    const ro = new ResizeObserver(() => makeSurface());
    ro.observe(wrap);
    // Initial fit on the first page.
    requestAnimationFrame(() => editor.zoomToFit());
    return () => {
      ro.disconnect();
    };
  }, [editor, makeSurface]);

  useEffect(() => {
    if (import.meta.env.DEV) (window as unknown as { __ocCanvas: unknown }).__ocCanvas = { editor, engine, sceneRef, dirty };
    const pool = poolFor(editor);
    pool.attach(engine.painter);
    // A video seeked or loaded a frame: its pixels changed without the display list changing.
    pool.setInvalidate(() => {
      sceneRef.current?.invalidateVideos();
      dirty.current = true;
    });
    let lastSync = '';
    const offE = editor.subscribe(() => {
      dirty.current = true;
      // Seek / pause / doc edits: re-sync media to the playhead.
      const st = editor.state;
      const key = `${st.playhead}:${st.playing}:${editor.version}`;
      if (st.playhead !== null && key !== lastSync && !st.playing) {
        lastSync = key;
        pool.sync(editor.doc, st.playhead, false);
      }
      if (st.playhead === null) pool.pauseAll();
    });
    // Images or fonts arrived: repaint everything; faces change metrics, so re-measure auto-height text.
    const offP = engine.painter.onInvalidate(() => {
      sceneRef.current?.invalidate();
      dirty.current = true;
      editor.remeasureText();
    });
    let raf = 0;
    const frame = () => {
      raf = requestAnimationFrame(frame);
      if (editor.state.playing) {
        editor.tick();
        pool.sync(editor.doc, editor.currentTime(), editor.state.playing);
        // The playhead moves every frame: display lists with videos, animations and captions change, and the
        // dirty regions repaint just those (each video op names its source time, so a new frame is a change).
        dirty.current = true;
      }
      const scene = sceneRef.current;
      if (!dirty.current || !scene) return;
      dirty.current = false;
      try {
        // Tiles left over (a zoom repaints within a per-frame budget) keep the loop going.
        if (draw(scene)) dirty.current = true;
      } catch (e) {
        // Keep the editor alive; surface the error for debugging.
        (window as unknown as { __ocDrawError?: string }).__ocDrawError = String((e as Error)?.stack ?? e);
        console.error('[oc] draw failed', e);
      }
      // Faces the layouts lacked (measuring included) start loading.
      engine.fonts.pump();
    };
    raf = requestAnimationFrame(frame);
    return () => {
      cancelAnimationFrame(raf);
      offE();
      offP();
      pool.pauseAll();
      engine.painter.videoFrames = null;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [editor, engine]);

  const draw = (scene: ScenePainter): boolean => {
    const dpr = window.devicePixelRatio || 1;
    const v = editor.view;
    const port = editor.state.viewport;
    const dlFor = (pageId: PageId): DisplayList => {
      const pv = editor.state.preview;
      let hit = dlCache.current.get(pageId);
      const ph = editor.state.playhead;
      const at = ph !== null ? locateTime(v, editor.state.playing ? editor.currentTime() : ph) : null;
      if (at && at.span.page.id === pageId) {
        hit = { v, dl: resolvePage(v, pageId, { timeMs: at.pageTimeMs, captionTimeMs: at.span.startMs + at.pageTimeMs }), editing: null };
        dlCache.current.set(pageId, { ...hit, v: null as unknown as DesignDoc });
      } else if (pv && pv.pageId === pageId) {
        const elapsed = editor.now() - pv.startedAt;
        hit = { v, dl: resolvePage(v, pageId, { timeMs: elapsed }), editing: null };
        dlCache.current.set(pageId, { ...hit, v: null as unknown as DesignDoc });
        const page = v.pages[pageId]!;
        if (elapsed > pageSettleMs(page, childrenOf(v, pageId)) + 400) queueMicrotask(() => editor.stopPreview());
        else dirty.current = true;
      } else if (!hit || hit.v !== v || hit.editing !== (editor.state.editingText?.id ?? null)) {
        const editing = editor.state.editingText?.id ?? null;
        hit = { v, dl: resolvePage(v, pageId, { editingText: editing }), editing };
        dlCache.current.set(pageId, hit);
      }
      return hit.dl;
    };
    const { pages, layout } = scenePages(v, port, dlFor);
    return scene.frame({ view: v, port, dpr, workspace: rgba(cssVar('--workspace', '#e8e8ee'), [0.91, 0.91, 0.93, 1]), whiteboard: v.meta.surface === 'whiteboard', pages, layout });
  };

  // ---------------------------------------------------------------- coordinates
  const toScene = (e: { clientX: number; clientY: number }): [number, number] => {
    const r = wrapRef.current!.getBoundingClientRect();
    return screenToScene(editor.state.viewport, [e.clientX - r.left, e.clientY - r.top]);
  };
  const S = (p: Vec2): [number, number] => [(p[0] - vp.x) * vp.zoom, (p[1] - vp.y) * vp.zoom];
  const slots = useMemo(() => pageSlots(view), [view]);
  const origin = (pageId: string | undefined): Vec2 => {
    const s = slots.find((x) => x.page.id === pageId);
    return s ? [s.rect.x, s.rect.y] : [0, 0];
  };
  const pageOfEl = (id: ElementId): PageId | undefined => {
    let cur: string | undefined = id;
    const seen = new Set<string>();
    while (cur && !seen.has(cur)) {
      seen.add(cur);
      if (view.pages[cur as PageId]) return cur as PageId;
      cur = view.elements[cur as ElementId]?.parentId;
    }
    return undefined;
  };
  const quadScreen = (id: ElementId): Array<[number, number]> | null => {
    if (!view.elements[id]) return null;
    const o = origin(pageOfEl(id));
    return worldQuad(view, id).map(([x, y]) => S([x + o[0], y + o[1]]));
  };
  const elMatrixScreen = (id: ElementId): Mat2D => {
    const o = origin(pageOfEl(id));
    const m = worldMatrix(view, id);
    return multiply([vp.zoom, 0, 0, vp.zoom, (o[0] - vp.x) * vp.zoom, (o[1] - vp.y) * vp.zoom], m);
  };

  // ---------------------------------------------------------------- eyedropper fallback
  const [picking, setPicking] = useState<((hex: string) => void) | null>(null);
  useEffect(() => {
    const on = (e: Event) => {
      const resolve = (e as CustomEvent<{ resolve: (hex: string) => void }>).detail.resolve;
      setPicking(() => resolve);
      toast(t('Click anywhere on the design to pick its colour'));
    };
    window.addEventListener('oc:eyedrop', on);
    return () => window.removeEventListener('oc:eyedrop', on);
  }, [toast]);
  /** Render one pixel of a page (exactly what the design shows, without editor chrome). */
  const samplePixel = (p: Vec2): string | null => {
    const slot = slots.find((s) => p[0] >= s.rect.x && p[1] >= s.rect.y && p[0] <= s.rect.x + s.rect.w && p[1] <= s.rect.y + s.rect.h);
    if (!slot || !sceneRef.current) return null;
    return sceneRef.current.samplePixel(resolvePage(view, slot.page.id), p[0] - slot.rect.x, p[1] - slot.rect.y);
  };

  // ---------------------------------------------------------------- pointer input
  const pinchState = () => {
    const [a, b] = [...touches.current.values()];
    const r = wrapRef.current!.getBoundingClientRect();
    return { dist: Math.hypot(a!.x - b!.x, a!.y - b!.y), mid: [(a!.x + b!.x) / 2 - r.left, (a!.y + b!.y) / 2 - r.top] as [number, number] };
  };
  const clearLongPress = () => {
    if (longPress.current) clearTimeout(longPress.current.timer);
    longPress.current = null;
  };

  const onPointerDown = (e: RPointerEvent<HTMLDivElement>) => {
    if (e.pointerType === 'touch') {
      touches.current.set(e.pointerId, { x: e.clientX, y: e.clientY });
      if (touches.current.size >= 2) {
        // A second finger: abandon whatever the first one started and pinch instead.
        clearLongPress();
        editor.cancelGesture();
        pan.current = null;
        pinch.current = pinchState();
        touchBlocked.current = true;
        e.currentTarget.setPointerCapture(e.pointerId);
        return;
      }
      touchBlocked.current = false;
      const now = performance.now();
      const lt = lastTap.current;
      if (!readOnly && now - lt.t < 320 && Math.hypot(e.clientX - lt.x, e.clientY - lt.y) < 24) {
        lastTap.current = { t: 0, x: 0, y: 0 };
        editor.doubleClick(toScene(e));
        touchBlocked.current = true;
        return;
      }
      lastTap.current = { t: now, x: e.clientX, y: e.clientY };
      const { clientX, clientY } = e;
      longPress.current = { x: clientX, y: clientY, timer: window.setTimeout(() => {
        longPress.current = null;
        editor.cancelGesture();
        touchBlocked.current = true;
        navigator.vibrate?.(10);
        openMenuAt(clientX, clientY);
      }, 520) };
    }
    if (menu) setMenu(null);
    if (picking) {
      const hexColor = samplePixel(toScene(e));
      if (hexColor) picking(hexColor);
      setPicking(null);
      return;
    }
    wrapRef.current?.focus({ preventScroll: true });
    if (e.button === 1 || spaceDown.current || (readOnly && e.button === 0)) {
      pan.current = { x: e.clientX, y: e.clientY };
      e.currentTarget.setPointerCapture(e.pointerId);
      return;
    }
    if (e.button !== 0) return;
    if (editor.state.playing) editor.pause();
    const tgt = e.target as HTMLElement;
    const handle = tgt.closest?.('[data-handle]')?.getAttribute('data-handle') as Handle | null;
    const rotate = tgt.closest?.('[data-rotate]');
    const endpoint = tgt.closest?.('[data-endpoint]')?.getAttribute('data-endpoint');
    const tableCol = tgt.closest?.('[data-tablecol]')?.getAttribute('data-tablecol');
    const pathNode = tgt.closest?.('[data-pathnode]')?.getAttribute('data-pathnode')?.split(':');
    const target: HitTarget = pathNode ? { kind: 'pathNode', s: +pathNode[0]!, i: +pathNode[1]!, part: pathNode[2] as NodePart } : endpoint ? { kind: 'endpoint', end: endpoint === '1' ? 1 : 0 } : handle ? { kind: 'handle', handle } : rotate ? { kind: 'rotate' }
      : tableCol && single?.type === 'table' ? { kind: 'tableCol', id: single.id, index: +tableCol } : { kind: 'canvas' };
    e.currentTarget.setPointerCapture(e.pointerId);
    editor.pointerDown(toScene(e), mods(e), target, e.button);
  };

  const lastHover = useRef(0);
  const onPointerMove = (e: RPointerEvent<HTMLDivElement>) => {
    if (e.pointerType === 'touch') {
      if (!touches.current.has(e.pointerId)) return;
      touches.current.set(e.pointerId, { x: e.clientX, y: e.clientY });
      if (longPress.current && Math.hypot(e.clientX - longPress.current.x, e.clientY - longPress.current.y) > 8) clearLongPress();
      if (pinch.current && touches.current.size >= 2) {
        const next = pinchState();
        const prev = pinch.current;
        if (prev.dist > 0) editor.zoomBy(next.dist / prev.dist, next.mid);
        editor.panBy(prev.mid[0] - next.mid[0], prev.mid[1] - next.mid[1]);
        pinch.current = next;
        return;
      }
      if (touchBlocked.current) return;
    }
    const p = toScene(e);
    provider.awareness?.setLocalStateField('cursor', p);
    if (pan.current) {
      editor.panBy(pan.current.x - e.clientX, pan.current.y - e.clientY);
      pan.current = { x: e.clientX, y: e.clientY };
      return;
    }
    if (e.buttons & 1) {
      editor.pointerMove(p, mods(e), e.pressure || 0.5);
      return;
    }
    // The pen's rubber band follows the pointer.
    if (state.tool === 'pen') {
      editor.pointerMove(p, mods(e));
      return;
    }
    const now = performance.now();
    if (state.tool !== 'select' || now - lastHover.current < 30) return;
    lastHover.current = now;
    const slot = slots.find((s) => p[1] >= s.rect.y && p[1] <= s.rect.y + s.rect.h && p[0] >= s.rect.x && p[0] <= s.rect.x + s.rect.w);
    const hit = slot ? hitTest(view, slot.page.id, [p[0] - slot.rect.x, p[1] - slot.rect.y], { enterGroup: state.enteredGroup }) : null;
    editor.setHover(hit?.id ?? null);
  };

  const onPointerUp = (e: RPointerEvent<HTMLDivElement>) => {
    if (e.pointerType === 'touch') {
      touches.current.delete(e.pointerId);
      clearLongPress();
      if (pinch.current) {
        if (touches.current.size < 2) {
          pinch.current = null;
          editor.updateActivePageFromScroll();
        }
        return;
      }
      if (touchBlocked.current) {
        if (!touches.current.size) touchBlocked.current = false;
        return;
      }
    }
    if (pan.current) {
      pan.current = null;
      editor.updateActivePageFromScroll();
      return;
    }
    editor.pointerUp();
    void e;
  };

  const openMenuAt = (clientX: number, clientY: number) => {
    if (readOnly) return;
    const p = toScene({ clientX, clientY });
    const slot = slots.find((s) => p[1] >= s.rect.y && p[1] <= s.rect.y + s.rect.h);
    if (slot) {
      const hit = hitTest(view, slot.page.id, [p[0] - slot.rect.x, p[1] - slot.rect.y], { enterGroup: state.enteredGroup });
      if (hit && !state.selection.includes(hit.id)) editor.select([hit.id]);
    }
    const r = wrapRef.current!.getBoundingClientRect();
    setMenu({ x: clientX - r.left, y: clientY - r.top });
  };

  /** The menu under a button (the quick bar's '…') or, from the keyboard, under the selection; no hit-testing. */
  const openMenuFor = (anchor: HTMLElement | null) => {
    if (readOnly) return;
    const r = wrapRef.current!.getBoundingClientRect();
    const a = anchor?.getBoundingClientRect();
    const b = quickBarBox();
    if (a) setMenu({ x: a.left - r.left, y: a.bottom - r.top + 4 });
    else if (b) setMenu({ x: (b.x0 + b.x1) / 2, y: Math.max(8, b.y1 > r.height ? r.height / 2 : b.y1) });
    else setMenu({ x: r.width / 2, y: r.height / 2 });
  };

  // wheel: pan, ctrl/⌘ (and trackpad pinch) zoom at cursor
  useEffect(() => {
    const wrap = wrapRef.current!;
    const onWheel = (e: WheelEvent) => {
      e.preventDefault();
      const r = wrap.getBoundingClientRect();
      if (e.ctrlKey || e.metaKey) editor.zoomBy(Math.exp(-e.deltaY * 0.01), [e.clientX - r.left, e.clientY - r.top]);
      else {
        editor.panBy(e.shiftKey ? e.deltaY : e.deltaX, e.shiftKey ? 0 : e.deltaY);
        editor.updateActivePageFromScroll();
      }
    };
    wrap.addEventListener('wheel', onWheel, { passive: false });
    const down = (e: KeyboardEvent) => {
      if (e.code === 'Space' && editor.state.playhead === null && !editor.state.editingText && editor.state.tool !== 'pen' && !(e.target instanceof HTMLInputElement || e.target instanceof HTMLTextAreaElement)) {
        spaceDown.current = true;
        wrap.style.cursor = 'grab';
      }
    };
    const up = (e: KeyboardEvent) => {
      if (e.code === 'Space') {
        spaceDown.current = false;
        wrap.style.cursor = '';
      }
    };
    window.addEventListener('keydown', down);
    window.addEventListener('keyup', up);
    return () => {
      wrap.removeEventListener('wheel', onWheel);
      window.removeEventListener('keydown', down);
      window.removeEventListener('keyup', up);
    };
  }, [editor]);

  // ---------------------------------------------------------------- IME proxy (text editing)
  const editing = state.editingText;
  // No state update here: an effect that sets state on every keystroke keeps React's nested-update counters climbing
  // during an input burst until it refuses updates (SYNC-01). The caret blinks with CSS (.text-caret), restarted on each move.
  useEffect(() => {
    if (editing) imeRef.current?.focus({ preventScroll: true });
  }, [editing?.id, editing?.focus, editing?.anchor, editing]);

  const onImeInput = () => {
    if (composing.current) return;
    const ta = imeRef.current!;
    if (ta.value) {
      editor.typeText(ta.value);
      ta.value = '';
    }
  };
  const onImeKey = (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
    if (composing.current || e.nativeEvent.isComposing) return;
    const cmd = e.metaKey || e.ctrlKey;
    const word = e.altKey || (cmd && (e.key === 'Backspace' || e.key === 'Delete'));
    switch (e.key) {
      case 'Backspace': e.preventDefault(); editor.deleteText(-1, word); return;
      case 'Delete': e.preventDefault(); editor.deleteText(1, word); return;
      case 'Enter': e.preventDefault(); editor.typeText('\n'); return;
      case 'ArrowLeft': e.preventDefault(); editor.moveCaret(cmd ? 'home' : 'left', e.shiftKey, e.altKey); return;
      case 'ArrowRight': e.preventDefault(); editor.moveCaret(cmd ? 'end' : 'right', e.shiftKey, e.altKey); return;
      case 'ArrowUp': e.preventDefault(); editor.moveCaret('up', e.shiftKey); return;
      case 'ArrowDown': e.preventDefault(); editor.moveCaret('down', e.shiftKey); return;
      case 'Home': e.preventDefault(); editor.moveCaret('home', e.shiftKey); return;
      case 'End': e.preventDefault(); editor.moveCaret('end', e.shiftKey); return;
      case 'Escape': e.preventDefault(); editor.handleKey('Escape', mods(e)); wrapRef.current?.focus(); return;
      case 'Tab': e.preventDefault(); if (editor.state.editingCell) editor.cellTab(e.shiftKey ? -1 : 1); else if (!e.shiftKey && editor.doc.meta.surface === 'whiteboard') editor.mindMapAdd('child'); return;
    }
    if (cmd) {
      const k = e.key.toLowerCase();
      if (editor.textShortcut(e.key, mods(e), e.code)) { e.preventDefault(); return; }
      if (k === 'a') { e.preventDefault(); editor.selectAll(); }
      else if (k === 'z') { e.preventDefault(); if (e.shiftKey) editor.redo(); else editor.undo(); }
    }
  };

  // ---------------------------------------------------------------- keyboard (canvas focused)
  const onKeyDown = (e: React.KeyboardEvent<HTMLDivElement>) => {
    if (e.target !== wrapRef.current) return;
    const st = editor.state;
    // Pen and node editing own their keys (Space, arrows, Enter, Escape, Tab between points; DS-100).
    if (!readOnly && (st.tool === 'pen' || st.pathEdit) && !e.metaKey && !e.ctrlKey && (e.key !== 'Tab' || st.pathEdit)) {
      if (editor.handleKey(e.key, mods(e), e.code)) {
        e.preventDefault();
        return;
      }
    }
    // Tab / Shift+Tab / Enter: the canvas is one tab stop that keyboard users can always leave (A11Y-01).
    const tab = canvasTabAction(e, {
      selection: st.selection,
      siblings: childrenOf(editor.doc, st.enteredGroup ?? st.activePageId).map((el) => el.id),
      whiteboard: !readOnly && editor.doc.meta.surface === 'whiteboard',
    });
    if (tab === 'native') return;
    // Shift+F10 / the context-menu key: the element (or page) menu, at the selection.
    if ((e.key === 'F10' && e.shiftKey) || e.key === 'ContextMenu') {
      e.preventDefault();
      openMenuFor(null);
      return;
    }
    // A11Y-02: PageDown / PageUp on the canvas go to the next / previous page.
    if ((e.key === 'PageDown' || e.key === 'PageUp') && !e.metaKey && !e.ctrlKey && !e.altKey) {
      const ps = sortedPages(editor.doc);
      const j = ps.findIndex((p) => p.id === st.activePageId) + (e.key === 'PageDown' ? 1 : -1);
      if (ps[j]) {
        e.preventDefault();
        editor.setActivePage(ps[j]!.id, true);
      }
      return;
    }
    if (tab === 'leave') {
      editor.clearSelection();
      return;
    }
    if (tab === 'enter' || (tab === 'editor' && readOnly)) {
      e.preventDefault();
      editor.handleKey('Tab', { ...mods(e), ctrl: false });
      return;
    }
    if (readOnly) {
      if (e.key === 'Escape') editor.clearSelection();
      return;
    }
    if (blocksBareShortcut(e)) return;
    if (editor.handleKey(e.key, { ...mods(e), ctrl: e.ctrlKey }, e.code)) e.preventDefault();
    else if (e.key.length === 1 && !e.metaKey && !e.ctrlKey && !e.altKey) {
      // Typing over selected table cells starts editing the first one.
      if (editor.state.cellRange) {
        e.preventDefault();
        editor.typeText(e.key);
        return;
      }
      // Typing on a selected text box starts editing it (and replaces nothing — like Canva, it appends).
      const sel = editor.selected();
      if (sel.length === 1 && sel[0]!.type === 'text') {
        editor.startTextEdit(sel[0]!.id);
        editor.moveCaret('end', false);
      }
    }
  };

  // ---------------------------------------------------------------- drag & drop
  /** Page slot and page point under a drag event. */
  const dropPoint = (e: { clientX: number; clientY: number }) => {
    const p = toScene(e);
    const slot = slots.find((s) => p[1] >= s.rect.y - 20 && p[1] <= s.rect.y + s.rect.h + 20) ?? slots.find((s) => s.page.id === state.activePageId);
    return slot ? { slot, local: [p[0] - slot.rect.x, p[1] - slot.rect.y] as Vec2 } : null;
  };
  /** What a drop of `media` at the event point fills; Alt (Option) drops always add a new element. */
  const targetAt = (e: React.DragEvent, media: DropMedia | null): DropTarget | null => {
    const at = dropPoint(e);
    if (!at || !media || e.altKey) return null;
    return findDropTarget(editor.doc, at.slot.page.id, at.local, media, { isBrandLocked: (el) => editor.isBrandLocked(el), inside: outlineInside(editor.doc) });
  };

  // dragover repeats while a drag is over the canvas, so the hint and highlight are cleared by a short timer: after a
  // dragleave (which also fires when crossing the canvas's own children — the next dragover cancels it), and as a
  // fallback when a cancelled drag never reports leaving.
  const dragTimer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const endDragHover = useCallback(() => {
    clearTimeout(dragTimer.current);
    setDropping(false);
    setDropTarget(null);
  }, []);
  useEffect(() => {
    window.addEventListener('dragend', endDragHover);
    return () => {
      window.removeEventListener('dragend', endDragHover);
      clearTimeout(dragTimer.current);
    };
  }, [endDragHover]);

  const onDragOver = (e: React.DragEvent) => {
    e.preventDefault();
    if (readOnly) return;
    clearTimeout(dragTimer.current);
    dragTimer.current = setTimeout(endDragHover, 1000);
    setDropping(true);
    const next = targetAt(e, dragMedia(e.dataTransfer));
    setDropTarget((cur) => (sameTarget(cur, next) ? cur : next));
  };

  const onDragLeave = (e: React.DragEvent) => {
    if (e.relatedTarget instanceof Node && wrapRef.current?.contains(e.relatedTarget)) return;
    clearTimeout(dragTimer.current);
    dragTimer.current = setTimeout(endDragHover, 80);
  };

  const onDrop = async (e: React.DragEvent) => {
    e.preventDefault();
    endDragHover();
    if (readOnly) return;
    const at = dropPoint(e);
    if (!at) return;
    const { slot, local } = at;
    editor.setActivePage(slot.page.id);
    // Targets are picked now, before any upload or import finishes: the highlighted shape is the one filled.
    const asset = e.dataTransfer.getData(ASSET_MIME);
    if (asset) {
      const a = JSON.parse(asset) as AssetRef;
      insertAssetAt(editor, a, local, slot.page.id, { target: targetAt(e, mediaOf(a)) });
      return;
    }
    const stock = e.dataTransfer.getData(STOCK_MIME);
    if (stock) {
      const it = JSON.parse(stock) as { provider: string; kind: StockKind; id: string };
      const target = targetAt(e, stockMedia(it.kind));
      try {
        const { asset } = await contentApi.import(it.provider, it.kind, it.id);
        insertAssetAt(editor, asset, local, slot.page.id, { target });
      } catch (x) {
        toast((x as Error).message || t('Couldn’t add it.'));
      }
      return;
    }
    const graphic = e.dataTransfer.getData(GRAPHIC_MIME);
    if (graphic) {
      const g = JSON.parse(graphic) as { id: string; kind?: 'icon' | 'graphic' };
      await insertGraphic(editor, g.id, local, slot.page.id, g.kind, { target: targetAt(e, 'graphic') }).catch(() => toast(t('Couldn’t add that graphic.')));
      return;
    }
    const emoji = e.dataTransfer.getData('application/x-oc-emoji');
    if (emoji) {
      // Emoji are stickers: they go into frames and replace images, but don't fill shapes.
      const target = targetAt(e, 'graphic');
      const { asset } = await contentApi.emojiAsset(emoji);
      insertAssetAt(editor, asset, local, slot.page.id, { target });
      return;
    }
    const element = e.dataTransfer.getData('application/x-oc-element');
    if (element) {
      const els = JSON.parse(element) as Element[];
      editor.insert(els, local);
      return;
    }
    const files = [...e.dataTransfer.files].filter((f) => /^(image|video|audio)\//.test(f.type));
    // The first file fills what is under the pointer; the rest are added next to it.
    const target = files[0] ? targetAt(e, mediaOfMime(files[0].type)) : null;
    for (const [i, f] of files.entries()) {
      try {
        await uploadAndInsert(editor, f, [local[0] + i * 24, local[1] + i * 24], slot.page.id, { target: i ? null : target });
      } catch (x) {
        toast(t('Upload failed: {message}', { message: (x as Error).message }));
      }
    }
  };

  // ---------------------------------------------------------------- overlay rendering
  const sel = state.selection.filter((id) => view.elements[id] && !view.elements[id].deleted);
  const single = sel.length === 1 ? view.elements[sel[0]!]! : null;
  const showHandles = !readOnly && !state.editingText && !state.presenting && sel.length > 0 && !state.pathEdit && !sel.some((id) => view.elements[id]!.lock?.move);
  // Finger-sized handles on touch screens.
  const handleSize = coarse ? 18 : 9;

  const handlesFor = (): Array<{ h: Handle; p: [number, number] }> => {
    if (single) {
      const m = elMatrixScreen(single.id);
      const { w, h } = single.xf;
      const pts: Record<Handle, Vec2> = { nw: [0, 0], n: [w / 2, 0], ne: [w, 0], e: [w, h / 2], se: [w, h], s: [w / 2, h], sw: [0, h], w: [0, h / 2] };
      let list: Handle[] = ['nw', 'n', 'ne', 'e', 'se', 's', 'sw', 'w'];
      if (single.type === 'text') list = ['nw', 'ne', 'e', 'se', 'sw', 'w'];
      if (single.type === 'group' || single.type === 'qr') list = ['nw', 'ne', 'se', 'sw'];
      if (single.type === 'line') list = [];
      // Hide edge handles on tiny boxes.
      const sw = w * vp.zoom;
      const sh = h * vp.zoom;
      if (sw < 40) list = list.filter((x) => x !== 'n' && x !== 's');
      if (sh < 40) list = list.filter((x) => x !== 'e' && x !== 'w' || single.type === 'line' || single.type === 'text');
      return list.map((hh) => ({ h: hh, p: apply(m, pts[hh]) }));
    }
    const b = editor.selectionBox();
    if (!b) return [];
    const o = origin(state.activePageId);
    const tl = S([b.x + o[0], b.y + o[1]]);
    const br = S([b.x + b.w + o[0], b.y + b.h + o[1]]);
    return [
      { h: 'nw', p: tl }, { h: 'ne', p: [br[0], tl[1]] }, { h: 'se', p: br }, { h: 'sw', p: [tl[0], br[1]] },
    ];
  };

  const selQuad = (): Array<[number, number]> | null => {
    if (single) return quadScreen(single.id);
    const b = editor.selectionBox();
    if (!b) return null;
    const o = origin(state.activePageId);
    return [S([b.x + o[0], b.y + o[1]]), S([b.x + b.w + o[0], b.y + o[1]]), S([b.x + b.w + o[0], b.y + b.h + o[1]]), S([b.x + o[0], b.y + b.h + o[1]])];
  };
  const quad = sel.length ? selQuad() : null;
  /** Screen bounds of the selection (axis-aligned), for the quick bar and keyboard-opened menus. */
  function quickBarBox(): { x0: number; y0: number; x1: number; y1: number } | null {
    const q = editor.state.selection.length ? selQuad() : null;
    if (!q) return null;
    const xs = q.map((p) => p[0]), ys = q.map((p) => p[1]);
    return { x0: Math.min(...xs), y0: Math.min(...ys), x1: Math.max(...xs), y1: Math.max(...ys) };
  }
  const quickBar = !readOnly && sel.length > 0 && !state.gesturing && !state.editingText && !state.editingCell && !state.croppingId && !state.presenting
    && state.tool === 'select' && !menu && !state.styleBrush && !state.pathEdit ? quickBarBox() : null;
  const poly = (q: Array<[number, number]>) => q.map((p) => p.join(',')).join(' ');

  // rotate handle: below the bottom-centre, along the box's local down axis
  const rotateHandle = (): [number, number] | null => {
    if (!quad) return null;
    const bc: [number, number] = [(quad[2]![0] + quad[3]![0]) / 2, (quad[2]![1] + quad[3]![1]) / 2];
    const tc: [number, number] = [(quad[0]![0] + quad[1]![0]) / 2, (quad[0]![1] + quad[1]![1]) / 2];
    const dx = bc[0] - tc[0];
    const dy = bc[1] - tc[1];
    const len = Math.hypot(dx, dy) || 1;
    return [bc[0] + (dx / len) * 26, bc[1] + (dy / len) * 26];
  };

  // text caret / selection
  const textChrome = () => {
    if (!editing) return null;
    const fr = editor.editingTextFrame();
    if (!fr) return null;
    const el = fr.el;
    const layout = engine.text.layout(el.id, el.body, el.xf.w, el.effect);
    const m = multiply(elMatrixScreen(fr.hostId), [1, 0, 0, 1, fr.ox, fr.oy]);
    const rects = editing.anchor !== editing.focus ? engine.text.selectionRects(layout, editing.anchor, editing.focus) : [];
    const caret = engine.text.caretRect(layout, editing.focus);
    const toPoly = (r: { x: number; y: number; w: number; h: number }) =>
      poly([apply(m, [r.x, r.y]), apply(m, [r.x + r.w, r.y]), apply(m, [r.x + r.w, r.y + r.h]), apply(m, [r.x, r.y + r.h])]);
    const c0 = apply(m, [caret.x, caret.y]);
    const c1 = apply(m, [caret.x, caret.y + caret.h]);
    return (
      <g>
        {rects.map((r, i) => <polygon key={i} points={toPoly(r)} fill="var(--accent)" opacity={0.28} />)}
        {editing.anchor === editing.focus && (
          <line key={`${editing.id}:${editing.focus}`} className="text-caret" x1={c0[0]} y1={c0[1]} x2={c1[0]} y2={c1[1]} stroke="var(--accent)" strokeWidth={2} />
        )}
      </g>
    );
  };

  // keep the IME proxy near the caret so the OS candidate window appears in the right place
  const imePos = (() => {
    if (!editing) return { left: -100, top: -100 };
    const fr = editor.editingTextFrame();
    if (!fr) return { left: -100, top: -100 };
    const el = fr.el;
    const layout = engine.text.layout(el.id, el.body, el.xf.w, el.effect);
    const c = engine.text.caretRect(layout, editing.focus);
    const p = apply(elMatrixScreen(fr.hostId), [c.x + fr.ox, c.y + c.h + fr.oy]);
    return { left: p[0], top: p[1] };
  })();

  // table cell chrome (A12): caret cell outline, cell-range highlight, column border drag handles
  const tableChrome = () => {
    if (!single || single.type !== 'table' || state.presenting || readOnly) return null;
    const tb = single as TableEl;
    const m = elMatrixScreen(tb.id);
    const quadOf = (r: { x: number; y: number; w: number; h: number }) =>
      poly([apply(m, [r.x, r.y]), apply(m, [r.x + r.w, r.y]), apply(m, [r.x + r.w, r.y + r.h]), apply(m, [r.x, r.y + r.h])]);
    const geo = tableGeometry(tb);
    const out: React.ReactNode[] = [];
    const cr = state.cellRange?.id === tb.id ? state.cellRange : null;
    if (cr) {
      const r = { x: geo.xs[cr.c0]!, y: geo.ys[cr.r0]!, w: geo.xs[cr.c1 + 1]! - geo.xs[cr.c0]!, h: geo.ys[cr.r1 + 1]! - geo.ys[cr.r0]! };
      out.push(<polygon key="range" points={quadOf(r)} fill="var(--accent)" fillOpacity={0.14} stroke="var(--accent)" strokeWidth={2} pointerEvents="none" />);
    } else if (state.editingCell?.id === tb.id) {
      out.push(<polygon key="cell" points={quadOf(cellRect(tb, state.editingCell.r, state.editingCell.c, geo))} fill="none" stroke="var(--accent)" strokeWidth={2} pointerEvents="none" />);
    }
    for (let i = 0; i < tb.cols.length - 1; i++) {
      const a = apply(m, [geo.xs[i + 1]!, 0]);
      const b = apply(m, [geo.xs[i + 1]!, tb.xf.h]);
      out.push(<line key={`col${i}`} className="hit" data-tablecol={i} x1={a[0]} y1={a[1]} x2={b[0]} y2={b[1]} stroke="transparent" strokeWidth={8} style={{ cursor: 'col-resize' }}>
        <title>{t('Drag to resize column')}</title>
      </line>);
    }
    return <g>{out}</g>;
  };

  const cropGhost = () => {
    const id = state.croppingId;
    const el = id ? view.elements[id] : null;
    const crop = el ? editor.mediaCrop(el) : null;
    if (!el || !crop) return null;
    const m = elMatrixScreen(el.id);
    const W = el.xf.w / crop.w;
    const H = el.xf.h / crop.h;
    const x = -crop.x * W;
    const y = -crop.y * H;
    return <polygon points={poly([apply(m, [x, y]), apply(m, [x + W, y]), apply(m, [x + W, y + H]), apply(m, [x, y + H])])} fill="none" stroke="var(--accent)" strokeDasharray="6 4" strokeWidth={1.5} />;
  };

  const sizeBadge = () => {
    if (!state.gesturing || !single || !quad) return null;
    const p = quad[2]!;
    return (
      <g transform={`translate(${p[0] + 10},${p[1] + 10})`}>
        <rect width={single.xf.rot ? 104 : 86} height={22} rx={6} fill="var(--text)" opacity={0.85} />
        <text x={8} y={15} fontSize={11} fill="var(--surface)" fontFamily="Inter, sans-serif">
          {Math.round(single.xf.w)} × {Math.round(single.xf.h)}{single.xf.rot ? ` · ${Math.round(single.xf.rot)}°` : ''}
        </text>
      </g>
    );
  };

  const hover = state.hover && !sel.includes(state.hover) && !state.gesturing ? quadScreen(state.hover) : null;

  /** The drop target's outline while dragging media over it. */
  const dropHighlight = () => {
    const d = dropTarget && view.elements[dropTarget.id] ? dropOutline(view, dropTarget) : null;
    if (!d || !dropTarget) return null;
    return (
      <path className="drop-target" d={d} transform={`matrix(${elMatrixScreen(dropTarget.id).join(' ')})`} fill="var(--accent)" fillOpacity={0.14}
        stroke="var(--accent)" strokeWidth={3} vectorEffect="non-scaling-stroke" strokeLinejoin="round" />
    );
  };
  const DROP_LABELS: Record<DropTarget['kind'], string> = {
    shape: t('Drop to fill the shape'), frame: t('Drop to fill the frame'), cell: t('Drop to fill the cell'), image: t('Drop to replace the image'),
  };

  return (
    <div
      ref={wrapRef}
      className="canvas-wrap"
      tabIndex={0}
      aria-label={t('Design canvas')}
      aria-describedby="canvas-keys-hint"
      aria-keyshortcuts="Enter Tab Shift+Tab Escape"
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onPointerUp={onPointerUp}
      onPointerLeave={() => {
        provider.awareness?.setLocalStateField('cursor', null);
        editor.setHover(null);
      }}
      onKeyDown={onKeyDown}
      onDoubleClick={(e) => !readOnly && editor.doubleClick(toScene(e))}
      onContextMenu={(e) => {
        e.preventDefault();
        // Touch long-press is handled by its own timer (some browsers also fire contextmenu).
        if ((e.nativeEvent as PointerEvent).pointerType === 'touch' || touchBlocked.current) return;
        openMenuAt(e.clientX, e.clientY);
      }}
      onPointerCancel={(e) => {
        touches.current.delete(e.pointerId);
        clearLongPress();
        pinch.current = null;
        pan.current = null;
        editor.cancelGesture();
      }}
      onDragOver={onDragOver}
      onDragLeave={onDragLeave}
      onDrop={onDrop}
      style={{ cursor: state.tool === 'draw' || state.tool === 'erase' || state.tool === 'pen' || picking ? 'crosshair' : state.styleBrush ? 'copy' : state.croppingId ? 'move' : undefined }}
    >
      {prefs.rulers && <Rulers slots={slots} />}
      <canvas ref={canvasRef} className="scene" />
      <svg className="overlay">
        {hover && <polygon points={poly(hover)} fill="none" stroke="var(--accent)" strokeWidth={1.5} opacity={0.7} />}
        {dropping && dropHighlight()}
        {/* remote selections */}
        {presence.flatMap((pr) =>
          (pr.selection ?? []).map((id) => {
            const q = quadScreen(id as ElementId);
            return q ? <polygon key={`${pr.clientId}:${id}`} points={poly(q)} fill="none" stroke={pr.user.color} strokeWidth={2} /> : null;
          }),
        )}
        {sel.length > 1 && sel.map((id) => {
          const q = quadScreen(id);
          return q ? <polygon key={id} points={poly(q)} fill="none" stroke="var(--accent)" strokeWidth={1} opacity={0.6} /> : null;
        })}
        {quad && !state.presenting && <polygon points={poly(quad)} fill="none" stroke="var(--accent)" strokeWidth={editing ? 1 : 2} strokeDasharray={sel.length > 1 ? '5 4' : undefined} />}
        {cropGhost()}
        {tableChrome()}
        {textChrome()}
        {state.marquee && (() => {
          const a = S([state.marquee.x, state.marquee.y]);
          const b = S([state.marquee.x + state.marquee.w, state.marquee.y + state.marquee.h]);
          return <rect x={a[0]} y={a[1]} width={b[0] - a[0]} height={b[1] - a[1]} fill="var(--accent)" fillOpacity={0.08} stroke="var(--accent)" strokeWidth={1} />;
        })()}
        {/* margins, bleed and ruler guides per page */}
        {slots.map((s) => {
          const r = s.rect;
          const els: React.ReactNode[] = [];
          if (prefs.margins) {
            const m = marginOf(r.w, r.h);
            const a = S([r.x + m, r.y + m]);
            const b = S([r.x + r.w - m, r.y + r.h - m]);
            els.push(<rect key="m" x={a[0]} y={a[1]} width={b[0] - a[0]} height={b[1] - a[1]} fill="none" stroke="#22d3ee" strokeDasharray="6 4" strokeWidth={1} />);
          }
          if (prefs.bleed) {
            const a = S([r.x - BLEED_PX, r.y - BLEED_PX]);
            const b = S([r.x + r.w + BLEED_PX, r.y + r.h + BLEED_PX]);
            els.push(<rect key="b" x={a[0]} y={a[1]} width={b[0] - a[0]} height={b[1] - a[1]} fill="none" stroke="#f43f5e" strokeDasharray="3 3" strokeWidth={1} />);
          }
          if (prefs.rulers) {
            for (const [i, g] of (s.page.guides ?? []).entries()) {
              const a = g.axis === 'x' ? S([r.x + g.pos, r.y]) : S([r.x, r.y + g.pos]);
              const b = g.axis === 'x' ? S([r.x + g.pos, r.y + r.h]) : S([r.x + r.w, r.y + g.pos]);
              els.push(<line key={`g${i}`} x1={a[0]} y1={a[1]} x2={b[0]} y2={b[1]} stroke="#06b6d4" strokeWidth={1} />);
            }
          }
          return els.length ? <g key={`v:${s.page.id}`} pointerEvents="none">{els}</g> : null;
        })}
        {state.spacing.map((m, i) => {
          const o = origin(state.activePageId);
          const a = m.axis === 'x' ? S([m.from + o[0], m.at + o[1]]) : S([m.at + o[0], m.from + o[1]]);
          const b = m.axis === 'x' ? S([m.to + o[0], m.at + o[1]]) : S([m.at + o[0], m.to + o[1]]);
          const mid: [number, number] = [(a[0] + b[0]) / 2, (a[1] + b[1]) / 2];
          return (
            <g key={`sp${i}`} pointerEvents="none">
              <line x1={a[0]} y1={a[1]} x2={b[0]} y2={b[1]} stroke="#ec4899" strokeWidth={1} />
              {[a, b].map((p, k) => (m.axis === 'x'
                ? <line key={k} x1={p[0]} y1={p[1] - 4} x2={p[0]} y2={p[1] + 4} stroke="#ec4899" />
                : <line key={k} x1={p[0] - 4} y1={p[1]} x2={p[0] + 4} y2={p[1]} stroke="#ec4899" />))}
              <rect x={mid[0] - 14} y={mid[1] - 9} width={28} height={16} rx={4} fill="#ec4899" />
              <text x={mid[0]} y={mid[1] + 3} fontSize={10} fill="#fff" textAnchor="middle" fontFamily="Inter, sans-serif">{m.gap}</text>
            </g>
          );
        })}
        {/* connector glue targets */}
        {state.connectTarget && (() => {
          const ct = state.connectTarget;
          const o = origin(pageOfEl(ct.id));
          return (
            <g pointerEvents="none">
              {ANCHORS.map((a) => {
                const p = anchorPoint(view, ct.id, a);
                if (!p) return null;
                const sp = S([p[0] + o[0], p[1] + o[1]]);
                const on = a === ct.anchor;
                return <circle key={a} cx={sp[0]} cy={sp[1]} r={on ? 6 : 4} fill={on ? 'var(--accent)' : '#fff'} stroke="var(--accent)" strokeWidth={1.5} />;
              })}
            </g>
          );
        })()}
        {state.guides.map((g, i) => {
          const o = origin(state.activePageId);
          const a = g.axis === 'x' ? S([g.pos + o[0], g.from + o[1]]) : S([g.from + o[0], g.pos + o[1]]);
          const b = g.axis === 'x' ? S([g.pos + o[0], g.to + o[1]]) : S([g.to + o[0], g.pos + o[1]]);
          return <line key={i} x1={a[0]} y1={a[1]} x2={b[0]} y2={b[1]} stroke="#ec4899" strokeWidth={1} strokeDasharray={g.kind === 'page' ? '4 3' : undefined} />;
        })}
        {showHandles && quad && (() => {
          const rh = rotateHandle();
          const hs = handlesFor();
          return (
            <g>
              {rh && single?.type !== 'line' && (
                <g className="hit" data-rotate="1" style={{ cursor: 'grab' }}>
                  <circle cx={rh[0]} cy={rh[1]} r={11} fill="var(--surface)" stroke="var(--border-strong)" />
                  <path d={`M${rh[0] - 4} ${rh[1] - 2} a5 5 0 1 0 5 -3`} fill="none" stroke="var(--text)" strokeWidth={1.6} />
                  <path d={`M${rh[0] + 1} ${rh[1] - 7} l3 2 -3 2`} fill="none" stroke="var(--text)" strokeWidth={1.6} />
                </g>
              )}
              {single?.type === 'line' && single.points.length >= 2 && [0, 1].map((end) => {
                const m = elMatrixScreen(single.id);
                const pt = apply(m, end === 0 ? single.points[0]! : single.points[single.points.length - 1]!);
                const glued = end === 0 ? single.startAttach : single.endAttach;
                return <circle key={`ep${end}`} className="hit" data-endpoint={end} cx={pt[0]} cy={pt[1]} r={7} fill={glued ? 'var(--accent)' : '#fff'} stroke="var(--accent)" strokeWidth={1.5} style={{ cursor: 'crosshair' }} />;
              })}
              {hs.map(({ h, p }) => {
                const corner = h.length === 2;
                return corner ? (
                  <circle key={h} className="hit" data-handle={h} cx={p[0]} cy={p[1]} r={handleSize / 2 + 1.5} fill="#fff" stroke="var(--accent)" strokeWidth={1.5} style={{ cursor: h === 'nw' || h === 'se' ? 'nwse-resize' : 'nesw-resize' }} />
                ) : (
                  <rect key={h} className="hit" data-handle={h} x={p[0] - (h === 'n' || h === 's' ? 9 : 3.5)} y={p[1] - (h === 'n' || h === 's' ? 3.5 : 9)} width={h === 'n' || h === 's' ? 18 : 7} height={h === 'n' || h === 's' ? 7 : 18} rx={3.5} fill="#fff" stroke="var(--accent)" strokeWidth={1.5} style={{ cursor: h === 'n' || h === 's' ? 'ns-resize' : 'ew-resize' }} />
                );
              })}
            </g>
          );
        })()}
        {sel.length === 1 && single?.lock?.move && quad && (
          <g transform={`translate(${quad[1]![0] - 8},${quad[1]![1] - 26})`}>
            <rect width={22} height={22} rx={6} fill="var(--surface)" stroke="var(--border)" />
            <Lock x={4} y={4} size={14} />
          </g>
        )}
        {sizeBadge()}
        {state.drawing && (() => {
          const o = origin(state.drawing.pageId);
          const pts: Array<[number, number]> = [];
          const d = state.drawing.points;
          for (let i = 0; i + 1 < d.length; i += 3) pts.push(S([d[i]! + o[0], d[i + 1]! + o[1]]));
          const b = state.brush;
          // Eraser (g55): a soft trail as wide as the eraser.
          if (state.tool === 'erase') return <path d={smoothPath(pts)} fill="none" stroke="var(--text-2)" strokeOpacity={0.25} strokeWidth={2 * Math.max(4, b.size) * vp.zoom} strokeLinecap="round" strokeLinejoin="round" />;
          const w = b.size * vp.zoom * (b.kind === 'highlighter' ? 3 : b.kind === 'marker' ? 1.6 : 1);
          return <path d={smoothPath(pts)} fill="none" stroke={b.color} strokeOpacity={b.kind === 'highlighter' ? 0.38 : 1} strokeWidth={w} strokeLinecap={b.kind === 'highlighter' ? 'square' : 'round'} strokeLinejoin="round" />;
        })()}
        <VectorOverlay S={S} origin={origin} pageOfEl={pageOfEl} coarse={coarse} />
        {/* comment pins */}
        {[...commentPins.entries()].map(([id, n]) => {
          const q = quadScreen(id as ElementId);
          if (!q) return null;
          const p = q[1]!;
          return (
            <g key={id} transform={`translate(${p[0] + 4},${p[1] - 22})`}>
              <path d="M0 9 a9 9 0 1 1 4 7.5 L0 20 Z" fill="#f59e0b" />
              <text x={9} y={13} fontSize={10} fontWeight={700} textAnchor="middle" fill="#fff" fontFamily="Inter, sans-serif">{n}</text>
            </g>
          );
        })}
        {/* remote cursors */}
        {presence.map((pr) => {
          if (!pr.cursor) return null;
          const p = S(pr.cursor);
          return (
            <g key={pr.clientId} transform={`translate(${p[0]},${p[1]})`} style={{ transition: 'transform 80ms linear' }}>
              <path d="M0 0 L0 16 L4.5 12 L8 19 L10.5 18 L7 11 L13 11 Z" fill={pr.user.color} stroke="#fff" strokeWidth={1.2} />
              <rect x={12} y={16} width={pr.user.name.length * 6.6 + 12} height={18} rx={9} fill={pr.user.color} />
              <text x={18} y={29} fontSize={11} fontWeight={600} fill="#fff" fontFamily="Inter, sans-serif">{pr.user.name}</text>
            </g>
          );
        })}
      </svg>
      {!state.presenting && <PageHeaders slots={slots} toScreen={S} zoom={vp.zoom} height={vp.height} />}
      {quickBar && <QuickBar box={quickBar} wrapWidth={vp.width} onMore={openMenuFor} />}
      {view.meta.surface === 'whiteboard' && <WhiteboardLayer />}
      {state.tool === 'pen' && (
        <p className="sr-only" role="status">
          {t('Pen: click to add points, drag to curve, click the first point to close. Keys: arrows move the cursor, Space adds a point, C closes, Enter finishes, Escape cancels.')}
        </p>
      )}
      {state.pathEdit && (
        <p className="sr-only" role="status">
          {t('Editing points: Tab selects the next point, arrow keys move it, S switches smooth and corner, + adds a point, Delete removes it, Escape finishes.')}
        </p>
      )}
      <p id="canvas-keys-hint" className="sr-only">
        {t('Enter selects the first element. Tab and Shift+Tab move between elements, and past the last one leave the canvas. Escape deselects, then Tab leaves the canvas.')}
      </p>
      <textarea
        ref={imeRef}
        className="ime-proxy"
        aria-label={state.editingCell ? t('Cell {row}, {col}', { row: state.editingCell.r + 1, col: state.editingCell.c + 1 }) : t('Text editor')}
        style={imePos}
        onInput={onImeInput}
        onKeyDown={onImeKey}
        onCompositionStart={() => (composing.current = true)}
        onCompositionEnd={(e) => {
          composing.current = false;
          editor.typeText(e.data);
          e.currentTarget.value = '';
        }}
        onBlur={() => {
          // Clicking toolbar controls keeps editing; clicking elsewhere on canvas is handled by pointerDown.
        }}
      />
      {dropping && (dropTarget ? (
        <div className="drop-hint on-target" role="status">
          <span>{DROP_LABELS[dropTarget.kind]}<small>{t('Hold Alt (⌥) to add it separately')}</small></span>
        </div>
      ) : <div className="drop-hint">{t('Drop images, video or audio to add them to your design')}</div>)}
      {menu && <div ref={setMenuAnchor} style={{ position: 'absolute', left: menu.x, top: menu.y, width: 1, height: 1 }} />}
      {menu && menuAnchor && (
        // At the cursor, flipped up/left near the window edges, and never over the app header.
        <Popover anchor={menuAnchor} onClose={() => setMenu(null)} placement="point" className="ctx-menu" menu
          minTop={(document.querySelector('.editor-header')?.getBoundingClientRect().bottom ?? 0) + 4}>
          {/* The popover is portalled to <body>, but React still bubbles its events up the component tree to this
              wrapper, whose pointerdown closed the menu (and moved the selection) before an item's click ran. */}
          <div
            onPointerDown={(e) => e.stopPropagation()}
            onPointerMove={(e) => e.stopPropagation()}
            onPointerUp={(e) => e.stopPropagation()}
            onDoubleClick={(e) => e.stopPropagation()}
            onContextMenu={(e) => { e.stopPropagation(); e.preventDefault(); }}
          >
            <ElementMenu close={() => setMenu(null)} />
          </div>
        </Popover>
      )}
      <ElementDialogs />
      <ImageEraserHost />
    </div>
  );
}

/** Whether the active page shows whole and centred in the viewport, as zoomToFit leaves it (2px tolerance). */
function pageFitted(editor: Editor): boolean {
  const vp = editor.state.viewport;
  const page = editor.state.activePageId;
  if (!page || !vp.width || !vp.height) return false;
  const [ox, oy] = editor.pageOrigin(page);
  const { w, h } = editor.doc.meta.dims;
  const left = (ox - vp.x) * vp.zoom, top = (oy - vp.y) * vp.zoom;
  const right = vp.width - (left + w * vp.zoom), bottom = vp.height - (top + h * vp.zoom);
  if (Math.min(left, top, right, bottom) < -1) return false;
  return Math.abs(left - right) <= 2 && Math.abs(top - bottom) <= 2;
}
