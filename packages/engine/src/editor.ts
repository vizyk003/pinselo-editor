// SPDX-License-Identifier: Apache-2.0
/**
 * `Editor`: the `@oc/editor-core` editor API over the Rust editor (crates/oc-editor-core via crates/oc-wasm).
 *
 * Sync. The browser's yjs `Y.Doc` stays the source of truth (TipTap, the board, `@hocuspocus/provider`). The
 * Rust editor holds a yrs mirror: it starts from `Y.encodeStateAsUpdate(ydoc)`, every yjs update not made by
 * this editor is applied to the mirror (`applyRemoteUpdate`), and the mirror's own updates are applied back to
 * the yjs doc right after each call, inside one yjs transaction whose origin is this editor's `origin`.
 *
 * Notifications. Listeners live here, not in Rust: every call returns a status word (`version * 4 + overrides * 2
 * + pending`), and when the version moved the listeners run once, after Rust has returned — so a listener may call
 * back into the editor. (TS calls listeners once per internal `emit`; here they run once per call. `version` is
 * Rust's counter and equals the TS one.)
 *
 * Snapshots are pulled lazily and at most once per version: `state` keeps its identity while only the document
 * changes (like TS); `doc` is a JS mirror patched from Rust deltas with structural sharing (unchanged elements keep
 * their identity); `view` overlays the gesture overrides like the TS getter.
 */
import type { DocOp, DocOpResult } from './types/crdt';
import type * as EC from './types/editor-core';
import type {
  AssetId, AudioClip, CaptionCue, DesignDoc, Element, ElementId, Page, PageId, Rect, TextEl, Vec2,
} from './types/schema';
import * as Y from 'yjs';
import { docRef, registerDoc, type DocOwner } from './docref';
import { descendants } from './schema';
import { EngineTextService } from './text';
import { raw } from './wasm';
import type * as V from './vector';

type TSEditor = EC.Editor;
type EditorState = V.EditorState;
type Viewport = EC.Viewport;
type Mods = EC.Mods;
type HitTarget = V.HitTarget;
type TextService = EC.TextService;
type ClipboardPayload = EC.ClipboardPayload;
type FindMatch = EC.FindMatch;

/** Transaction origin names accepted by `store.apply` (the `@oc/crdt` origin singletons map to these). */
export type OriginName = 'local' | 'agent' | 'template' | 'migration' | 'repair' | 'restore' | 'remote';

export interface EngineEditorOptions {
  /** Wall clock in ms (default: `performance.timeOrigin + performance.now()`, epoch-based); `'manual'`: set it with `setNow`. */
  clock?: (() => number) | 'manual';
  /** Start time of the manual clock. */
  now?: number;
  /** Deterministic id stream (Mulberry32 seed); default Web Crypto. */
  seed?: number;
  /** yrs client id of the mirror (default: random, different from the yjs doc's). */
  clientId?: number;
}

/** The store members apps/web uses (`editor.store.…`), backed by the Rust store. */
export interface EngineStore {
  /** The browser's yjs document (the source of truth). */
  readonly ydoc: Y.Doc;
  /** Tombstone author for writes. */
  actor: string;
  readonly doc: DesignDoc;
  /** Close the current undo capture group. */
  checkpoint(): void;
  /** A store write with an explicit origin (`'migration'` is not tracked by undo), plus the change reactions. */
  apply(ops: DocOp[], origin?: OriginName | object): DocOpResult;
  /** Replace the whole content as one undoable local change (`replaceContent(store.ydoc, doc, LocalOrigin.I, actor)`). */
  replaceContent(doc: DesignDoc): void;
  /** `undoStack.length > 0` ⇔ undo is available (lengths are 0 or 1). */
  readonly undo: { readonly undoStack: { readonly length: number }; readonly redoStack: { readonly length: number } };
}

/** Members of the TS editor this facade provides with the same signatures (`store` is {@link EngineStore}). */
type EditorApi = Omit<Pick<TSEditor, keyof TSEditor>, 'store' | 'overrides' | 'state'>;

const DEFAULT_VIEWPORT: Viewport = { x: 0, y: 0, zoom: 1, width: 800, height: 600 };
const ORIGINS: readonly OriginName[] = ['local', 'agent', 'template', 'migration', 'repair', 'restore', 'remote'];

function originName(o: OriginName | object | undefined): OriginName | undefined {
  if (o === undefined) return undefined;
  if (typeof o === 'string') return o;
  // `@oc/crdt` singletons (`MigrationOrigin.I`): matched by class name. Prefer passing the name (minifiers rename classes).
  const n = (o as { constructor?: { name?: string } }).constructor?.name?.replace(/Origin$/, '').toLowerCase();
  if (n && (ORIGINS as readonly string[]).includes(n)) return n as OriginName;
  throw new Error(`unknown transaction origin ${String(n)}`);
}

/** JSON argument array: trailing `undefined`s trimmed (TS defaults apply), the rest as `JSON.stringify` writes them. */
function argsJson(args: readonly unknown[]): string {
  let n = args.length;
  while (n > 0 && args[n - 1] === undefined) n--;
  return n === 0 ? '' : JSON.stringify(n === args.length ? args : args.slice(0, n));
}

/** Patch objects: a top-level `undefined` means "remove the key" in TS (DocOps treat it like `null`); JSON would drop it. */
function patch<T>(p: T): T {
  if (!p || typeof p !== 'object' || Array.isArray(p)) return p;
  let out: Record<string, unknown> | null = null;
  for (const k of Object.keys(p)) {
    if ((p as Record<string, unknown>)[k] === undefined) {
      out ??= { ...(p as Record<string, unknown>) };
      out[k] = null;
    }
  }
  return (out ?? p) as T;
}

const MODS = (m: Partial<Mods> & { ctrl?: boolean } | undefined): number =>
  m ? (m.shift ? 1 : 0) | (m.alt ? 2 : 0) | (m.meta ? 4 : 0) | (m.ctrl ? 8 : 0) : 0;

type Delta = {
  full: boolean;
  meta?: DesignDoc['meta'];
  assets?: DesignDoc['assets'];
  pages?: Record<string, Page | null>;
  elements?: Record<string, Element | null>;
};

/** Report an error without throwing (the window `error` event in browsers, the console elsewhere). */
function report(what: string, e: unknown): void {
  const g = globalThis as { reportError?: (e: unknown) => void };
  if (e && typeof g.reportError === 'function') g.reportError(e);
  console.error(`[engine] ${what}`, e);
}

const hardened = new WeakSet<Y.Doc>();

/**
 * Make a yjs doc's events isolate their observers (SYNC-01). yjs 13 emits `update` (and the other transaction
 * events) from the `finally` of its transaction cleanup with a plain `forEach`: an observer that throws skips the
 * observers after it (the provider never sends that update) and leaves the doc's cleanup queue non-empty, after
 * which no transaction ever emits `update` again, so nothing more is synced for the session. Here each observer is
 * called on its own and an error is reported instead of thrown. Idempotent.
 */
export function hardenDoc(ydoc: Y.Doc): void {
  if (hardened.has(ydoc)) return;
  hardened.add(ydoc);
  const d = ydoc as unknown as { _observers: Map<string, Set<(...a: unknown[]) => void>>; emit: (name: string, args: unknown[]) => void };
  if (!(d._observers instanceof Map)) return;
  d.emit = (name: string, args: unknown[]) => {
    const fs = d._observers.get(name);
    if (!fs) return;
    for (const f of [...fs]) {
      try {
        f(...args);
      } catch (e) {
        report(`a yjs '${name}' observer threw`, e);
      }
    }
  };
}

function patchMap<T>(prev: Record<string, T>, d: Record<string, T | null>): Record<string, T> {
  const next = { ...prev };
  for (const k of Object.keys(d)) {
    const v = d[k];
    if (v === null || v === undefined) delete next[k];
    else next[k] = v;
  }
  return next;
}

/**
 * A Rust panic (or a call on an editor a panic left borrowed). The wasm build aborts on panic, which traps as a
 * `WebAssembly.RuntimeError`; the facade then rebuilds the Rust editor from the yjs doc and throws this instead, so
 * the next call works on a fresh editor (the undo history and any gesture in flight are lost, the document is not).
 */
export class EnginePanicError extends Error {
  constructor(readonly cause: unknown) {
    super(`engine panic: ${cause instanceof Error ? cause.message : String(cause)}`);
    this.name = 'EnginePanicError';
  }
}

/** A trap of the wasm instance, or wasm-bindgen refusing a handle a trap left borrowed or freed. */
export function isEngineTrap(e: unknown): boolean {
  if (typeof WebAssembly !== 'undefined' && e instanceof WebAssembly.RuntimeError) return true;
  const m = e instanceof Error ? e.message : '';
  return /recursive use of an object detected|null pointer passed to rust|attempted to take ownership of Rust value while it was borrowed/.test(m);
}

export class Editor implements EditorApi, DocOwner {
  /** The wasm-bindgen handle (for advanced use and tests). Replaced when the editor recovers from a panic. */
  get raw(): raw.Editor {
    return this._raw;
  }
  private _raw: raw.Editor;
  /** Added to Rust's version so it keeps increasing across a rebuild after a panic. */
  private _vBase = 0;
  private readonly _clock: (() => number) | undefined;
  private _actor: string | null = null;
  /** Panics recovered from (tests, diagnostics). */
  panics = 0;
  readonly store: EngineStore;
  /** The yjs transaction origin of updates this editor applies to the yjs doc. */
  readonly origin: object;
  readonly CLIPBOARD_MIME = 'application/x-opencanvas+json';

  private readonly ydoc: Y.Doc;
  private readonly listeners = new Set<() => void>();
  private _version = 0;
  private _overrides = false;
  private _state: EditorState | null = null;
  private _stateV = -1;
  private _doc: DesignDoc | null = null;
  private _docV = -1;
  private _ov: Record<string, Partial<Element>> | null = null;
  private _ovV = -1;
  private _view: DesignDoc | null = null;
  private _viewKey: [DesignDoc | null, unknown] = [null, null];
  private _ts: TextService | null = null;
  private _brandAdmin = true;
  private _user: { id: string; name: string } | null = null;
  private _destroyed = false;
  private _clip: ClipboardPayload | null = null;
  private _syncing = false;
  private readonly _retry = new Set<() => void>();
  private _resync = false;
  private readonly onUpdate = (update: Uint8Array, origin: unknown) => {
    if (origin === this.origin || this._destroyed) return;
    // Runs inside the yjs transaction that applied the update: never throw into yjs. On a panic the rebuilt editor
    // starts from the yjs doc, which already holds this update.
    try {
      this.exec(() => this._raw.applyRemoteUpdate(update));
    } catch (e) {
      console.error('[engine] applying a remote update failed', e);
    }
  };

  constructor(store: { ydoc: Y.Doc; actor?: string } | Y.Doc, viewport: Viewport = DEFAULT_VIEWPORT, opts: EngineEditorOptions = {}) {
    const ydoc = store instanceof Y.Doc ? store : store.ydoc;
    this.ydoc = ydoc;
    this.origin = { engine: this };
    let clientId = opts.clientId;
    while (clientId === undefined || clientId === ydoc.clientID) clientId = crypto.getRandomValues(new Uint32Array(1))[0]!;
    const clock = opts.clock === 'manual' ? undefined : (opts.clock ?? (() => performance.timeOrigin + performance.now()));
    this._clock = clock;
    this._raw = new raw.Editor(Y.encodeStateAsUpdate(ydoc), { viewport, clientId, seed: opts.seed, clock, now: opts.now });
    this._version = this._raw.version;
    const self = this;
    this.store = {
      ydoc,
      get actor() {
        return self.call<string>('store.actor', []);
      },
      set actor(a: string) {
        self._actor = a;
        self.call('store.actor=', [a]);
      },
      get doc() {
        return self.doc;
      },
      checkpoint: () => this.call('store.checkpoint', []),
      apply: (ops, origin) => this.call<DocOpResult>('store.apply', [ops, originName(origin)]),
      replaceContent: (doc) => this.exec(() => this._raw.replaceContent(doc)),
      undo: {
        undoStack: { get length() { return self.guard(() => self._raw.dispatch('canUndo', '')) ? 1 : 0; } },
        redoStack: { get length() { return self.guard(() => self._raw.dispatch('canRedo', '')) ? 1 : 0; } },
      },
    };
    if (!(store instanceof Y.Doc) && store.actor) this.store.actor = store.actor;
    hardenDoc(ydoc);
    ydoc.on('update', this.onUpdate);
    ydoc.on('destroy', () => this.destroy());
  }

  /** Stop syncing and free the Rust editor (also called when the yjs doc is destroyed). */
  destroy(): void {
    if (this._destroyed) return;
    this._destroyed = true;
    this.ydoc.off('update', this.onUpdate);
    try {
      this._raw.free();
    } catch {
      /* a handle a panic left borrowed cannot be freed */
    }
  }

  /**
   * After a trap: replace the Rust editor with one built from the yjs doc (the source of truth), with the same
   * viewport, settings and services, a fresh yrs client id (updates the old mirror made but never delivered must not
   * collide with the new one's) and the default id stream. Listeners are notified.
   */
  private recover(cause: unknown): EnginePanicError {
    const err = cause instanceof EnginePanicError ? cause : new EnginePanicError(cause);
    if (this._destroyed) return err;
    this.panics++;
    console.error('[engine] the Rust editor panicked; rebuilding it from the document', cause);
    const old = this._raw;
    const viewport = this._state?.viewport ?? DEFAULT_VIEWPORT;
    let clientId: number | undefined;
    while (clientId === undefined || clientId === this.ydoc.clientID) clientId = crypto.getRandomValues(new Uint32Array(1))[0]!;
    this._raw = new raw.Editor(Y.encodeStateAsUpdate(this.ydoc), { viewport, clientId, clock: this._clock, now: this._clock ? undefined : this.safeNow(old) });
    try {
      old.free();
    } catch {
      /* still borrowed by the trapped call: leaked */
    }
    this._vBase = this._version + 1;
    this._stateV = this._docV = this._ovV = -1;
    this._doc = null;
    this._view = null;
    try {
      if (this._ts instanceof EngineTextService) this._raw.setTextService(this._ts.raw);
      else this._raw.setJsTextService(this._ts ?? null);
      if (this._actor !== null) this._raw.dispatch('store.actor=', argsJson([this._actor]));
      this._raw.dispatch('brandAdmin=', argsJson([this._brandAdmin]));
      if (this._user) this._raw.dispatch('user=', argsJson([this._user]));
      this.sync(this._raw.status());
    } catch (e) {
      console.error('[engine] restoring the editor settings failed', e);
    }
    return err;
  }

  private safeNow(r: raw.Editor): number | undefined {
    try {
      return r.now();
    } catch {
      return undefined;
    }
  }

  /** Run a Rust call that returns a status word, then {@link sync}; a trap rebuilds the editor and throws. */
  private exec(f: () => number): void {
    let status: number;
    try {
      status = f();
    } catch (e) {
      if (isEngineTrap(e) && !this._destroyed) throw this.recover(e);
      throw e;
    }
    this.sync(status);
  }

  /** A Rust call whose result is returned as is; a trap rebuilds the editor and throws. */
  private guard<T>(f: () => T): T {
    try {
      return f();
    } catch (e) {
      if (isEngineTrap(e) && !this._destroyed) throw this.recover(e);
      throw e;
    }
  }

  // ------------------------------------------------------------------ plumbing

  /**
   * After a Rust call: forward local updates to the yjs doc, notify when the version moved.
   *
   * Re-entrant and exception-safe (SYNC-01): a call made from a listener or a yjs observer while this runs only
   * marks another round, which the outer call runs after the current one (re-reading the status), so updates are
   * forwarded in order and listeners never nest. An error thrown by a yjs observer (e.g. a provider's update handler
   * running a React state update) or by a listener is reported and does not stop the remaining work: the drained
   * updates are already in the yjs doc, and every other listener still runs.
   */
  private sync(status: number): void {
    if (this._syncing) {
      this._resync = true;
      return;
    }
    this._syncing = true;
    try {
      for (let round = 0; ; round++) {
        this._resync = false;
        if (status % 2 === 1) this.forward();
        this._overrides = Math.floor(status / 2) % 2 === 1;
        const v = this._vBase + Math.floor(status / 4);
        if (v !== this._version) {
          this._version = v;
          for (const l of [...this.listeners]) {
            try {
              l();
            } catch (e) {
              report('a listener threw', e);
              this.retry(l);
            }
          }
        }
        if (!this._resync || this._destroyed) break;
        if (round > 1000) {
          report('listeners keep changing the editor; giving up this round', null);
          break;
        }
        status = this.guard(() => this._raw.status());
      }
    } finally {
      this._syncing = false;
    }
  }

  /**
   * Call a listener that threw again on a later task (e.g. React refused a store re-render in the middle of an
   * input burst), so the UI catches up with the last change even when no further change follows.
   */
  private retry(l: () => void): void {
    if (this._retry.has(l)) return;
    this._retry.add(l);
    if (this._retry.size > 1) return;
    setTimeout(() => {
      const ls = [...this._retry];
      this._retry.clear();
      for (const f of ls) {
        if (this._destroyed || !this.listeners.has(f)) continue;
        try {
          f();
        } catch (e) {
          report('a listener threw again', e);
        }
      }
    }, 16);
  }

  /** Apply the mirror's pending local updates to the yjs doc (one transaction with this editor's origin). */
  private forward(): void {
    const ups = this.guard(() => this._raw.drainLocalUpdates() as Uint8Array[]);
    if (!ups.length) return;
    let applied = 0;
    try {
      Y.transact(this.ydoc, (tr) => {
        for (const u of ups) {
          Y.applyUpdate(this.ydoc, u, this.origin);
          applied++;
        }
        // `applyUpdate` marks the transaction remote; these are the local user's edits (as with the TS editor).
        // Left remote, yjs would run its remote-only YText formatting cleanup on them and write the result back.
        tr.local = true;
      }, this.origin);
    } catch (e) {
      // Thrown by a deep observer after the updates were integrated (the doc's own events are isolated, see
      // `hardenDoc`), or by an update itself: apply whatever did not make it, so no later update waits on it.
      report('a yjs observer threw while applying local updates', e);
      for (const u of ups.slice(applied)) {
        try {
          Y.applyUpdate(this.ydoc, u, this.origin);
        } catch (e2) {
          report('applying a local update failed', e2);
        }
      }
    }
  }

  /** `Editor::dispatch` by TS name. */
  private call<T = void>(method: string, args: readonly unknown[]): T {
    const r = this.guard(() => this._raw.dispatch(method, argsJson(args)) as T);
    this.sync(this.guard(() => this._raw.status()));
    return r;
  }

  subscribe(fn: () => void): () => void {
    this.listeners.add(fn);
    return () => this.listeners.delete(fn);
  }

  /** Bumped on every notification; UI caches derived values on it. */
  get version(): number {
    return this._version;
  }

  get state(): EditorState {
    if (this._stateV !== this._version || !this._state) {
      const s = this.guard(() => this._raw.stateIfChanged() as EditorState | undefined);
      if (s) this._state = s;
      this._stateV = this._version;
    }
    return this._state!;
  }

  get doc(): DesignDoc {
    if (this._docV !== this._version || !this._doc) {
      const d = this.guard(() => this._raw.docDelta() as Delta | null);
      if (d) {
        const prev = this._doc;
        const next = (d.full || !prev
          ? { meta: d.meta, pages: d.pages ?? {}, elements: d.elements ?? {}, assets: d.assets ?? {} }
          : {
            ...prev,
            ...(d.meta ? { meta: d.meta } : {}),
            ...(d.assets ? { assets: d.assets } : {}),
            ...(d.pages ? { pages: patchMap(prev.pages, d.pages) } : {}),
            ...(d.elements ? { elements: patchMap(prev.elements, d.elements) } : {}),
          }) as DesignDoc;
        this._doc = next;
        registerDoc(next, this);
      }
      this._docV = this._version;
    }
    return this._doc!;
  }

  /** Document view including transient overrides (what the renderer and overlay should show). */
  get view(): DesignDoc {
    const doc = this.doc;
    if (!this._overrides) return doc;
    const ov = this.overrideRecord();
    if (!ov) return doc;
    if (this._view && this._viewKey[0] === doc && this._viewKey[1] === ov) return this._view;
    const elements = { ...doc.elements };
    for (const id of Object.keys(ov)) if (elements[id as ElementId]) elements[id as ElementId] = { ...elements[id as ElementId], ...ov[id] } as Element;
    const view = { ...doc, elements };
    this._view = view;
    this._viewKey = [doc, ov];
    registerDoc(view, this);
    return view;
  }

  private overrideRecord(): Record<string, Partial<Element>> | null {
    if (this._ovV !== this._version) {
      const o = this.guard(() => this._raw.overridesIfChanged() as Record<string, Partial<Element>> | null | undefined);
      if (o !== undefined) this._ov = o;
      this._ovV = this._version;
    }
    return this._overrides ? this._ov : null;
  }

  /** Transient overrides merged over the document while a gesture is running. */
  get overrides(): Map<ElementId, Partial<Element>> {
    return new Map(Object.entries(this.overrideRecord() ?? {}) as Array<[ElementId, Partial<Element>]>);
  }

  /** `DocOwner`: the Rust snapshot behind a document object this editor handed out, while it is current. */
  refFor(doc: object): raw.DocRef | null {
    if (doc === this.doc) return this.guard(() => this._raw.docRef(0));
    if (doc === this.view) return this.guard(() => this._raw.docRef(1));
    return null;
  }

  get textService(): TextService | null {
    return this._ts;
  }

  /** An {@link EngineTextService} is measured in Rust directly; any other `TextService` object is called back. */
  set textService(ts: TextService | null) {
    this._ts = ts;
    this.guard(() => (ts instanceof EngineTextService ? this._raw.setTextService(ts.raw) : this._raw.setJsTextService(ts ?? null)));
  }

  /** Brand-template locks (A8). Writing the field does not notify (like TS); `setBrandAdmin` does. */
  get brandAdmin(): boolean {
    return this._brandAdmin;
  }

  set brandAdmin(v: boolean) {
    this._brandAdmin = Boolean(v);
    this.call('brandAdmin=', [this._brandAdmin]);
  }

  get user(): { id: string; name: string } | null {
    return this._user;
  }

  set user(u: { id: string; name: string } | null) {
    this._user = u ? { id: u.id, name: u.name } : null;
    this.call('user=', [this._user]);
  }

  /** The editor clock (ms). `state.preview.startedAt` and `state.playAnchor.at` are on this clock. */
  now(): number {
    return this.guard(() => this._raw.now());
  }

  /** Set the manual clock (`clock: 'manual'`). */
  setNow(ms: number): void {
    this.guard(() => this._raw.setNow(ms));
  }

  apply(ops: DocOp[], merge = false): DocOpResult {
    return this.call('apply', [ops, merge]);
  }

  /** Re-read the whole document from the mirror and react (after host writes made on the mirror). */
  flush(): void {
    this.call('flush', []);
  }

  // ------------------------------------------------------------------ hot paths

  pointerDown(p: Vec2, mods: Mods, target: HitTarget = { kind: 'canvas' }, button = 0): void {
    this.exec(() => this._raw.pointerDown(p[0], p[1], MODS(mods), target, button));
  }

  pointerMove(p: Vec2, mods: Mods, pressure = 0.5): void {
    this.exec(() => this._raw.pointerMove(p[0], p[1], MODS(mods), pressure));
  }

  pointerUp(): void {
    this.exec(() => this._raw.pointerUp());
  }

  setHover(id: ElementId | null): void {
    this.exec(() => this._raw.setHover(id ?? undefined));
  }

  panBy(dx: number, dy: number): void {
    this.exec(() => this._raw.panBy(dx, dy));
  }

  zoomBy(factor: number, screenPt?: Vec2): void {
    this.exec(() => this._raw.zoomBy(factor, screenPt?.[0], screenPt?.[1]));
  }

  zoomTo(zoom: number, screenPt?: Vec2): void {
    this.exec(() => this._raw.zoomTo(zoom, screenPt?.[0], screenPt?.[1]));
  }

  setViewportSize(width: number, height: number): void {
    this.exec(() => this._raw.setViewportSize(width, height));
  }

  setViewport(vp: Viewport): void {
    this.exec(() => this._raw.setViewport(vp.x, vp.y, vp.zoom, vp.width, vp.height));
  }

  // ------------------------------------------------------------------ queries

  expandWhiteboard(margin = 600): void { this.call('expandWhiteboard', [margin]); }
  editShapeLabel(shapeId: ElementId): void { this.call('editShapeLabel', [shapeId]); }
  remeasureText(): void { this.call('remeasureText', []); }
  pageOrigin(pageId: PageId): Vec2 { return this.call('pageOrigin', [pageId]); }
  toPage(pageId: PageId, p: Vec2): [number, number] { return this.call('toPage', [pageId, p]); }

  /** Selected live elements (from the view, overrides included) — the view's own objects. */
  selected(): Element[] {
    const els = this.call<Element[]>('selected', []);
    const v = this.view.elements;
    return els.map((e) => v[e.id] ?? e);
  }

  selectionBox(): Rect | null { return this.call('selectionBox', []); }
  selectionFrame(): { xf: Element['xf']; matrix: [number, number, number, number, number, number] } | null { return this.call('selectionFrame', []); }

  /** Selection plus descendants of selected groups (for styling a group's contents). */
  selectedDeep(): Element[] {
    const out: Element[] = [];
    for (const el of this.selected()) {
      out.push(el);
      if (el.type === 'group') out.push(...descendants(this.doc, el.id));
    }
    return out;
  }

  get editingTableText(): boolean { return this.call('editingTableText', []); }

  // ------------------------------------------------------------------ selection & viewport

  select(ids: ElementId[], opts: { keepText?: boolean } = {}): void { this.call('select', [ids, opts]); }
  clearSelection(): void { this.call('clearSelection', []); }
  selectAll(): void { this.call('selectAll', []); }
  setActivePage(pageId: PageId, scroll = false): void { this.call('setActivePage', [pageId, scroll]); }
  zoomToFit(): void { this.call('zoomToFit', []); }
  scrollToPage(pageId: PageId): void { this.call('scrollToPage', [pageId]); }
  updateActivePageFromScroll(): void { this.call('updateActivePageFromScroll', []); }
  cancelGesture(): void { this.call('cancelGesture', []); }
  doubleClick(p: Vec2): void { this.call('doubleClick', [p]); }

  // ------------------------------------------------------------------ whiteboard

  viewCenter(pageId?: PageId): Vec2 { return this.call('viewCenter', [pageId]); }
  addSticky(color: string, opts: { text?: string; at?: Vec2; size?: number; author?: { id: string; name: string } | null } = {}): ElementId | null {
    return this.call('addSticky', [color, opts]);
  }
  setStickyColor(color: string, ids?: ElementId[]): number { return this.call('setStickyColor', [color, ids]); }
  clusterStickies(ids?: ElementId[]): number { return this.call('clusterStickies', [ids]); }
  createSection(title: string): ElementId | null { return this.call('createSection', [title]); }
  renameSection(id: ElementId, title: string): void { this.call('renameSection', [id, title]); }
  toggleSection(id: ElementId): void { this.call('toggleSection', [id]); }
  moveSection(id: ElementId, dir: -1 | 1): void { this.call('moveSection', [id, dir]); }
  zoomToElement(id: ElementId, pad = 48): void { this.call('zoomToElement', [id, pad]); }
  mindMapAdd(kind: 'child' | 'sibling'): ElementId | null { return this.call('mindMapAdd', [kind]); }
  editGroupText(groupId: ElementId): boolean { return this.call('editGroupText', [groupId]); }
  layoutMindMap(id?: ElementId): number { return this.call('layoutMindMap', [id]); }
  setTool(tool: EditorState['tool']): void { this.call('setTool', [tool]); }
  setBrush(b: Partial<EditorState['brush']>): void { this.call('setBrush', [b]); }

  // ------------------------------------------------------------------ tables

  startCellEdit(id: ElementId, r: number, c: number, pagePt?: Vec2, selectWord = false): void { this.call('startCellEdit', [id, r, c, pagePt, selectWord]); }
  stopCellEdit(): void { this.call('stopCellEdit', []); }
  selectCellRange(id: ElementId, from: Parameters<TSEditor['selectCellRange']>[1], to: Parameters<TSEditor['selectCellRange']>[2]): void {
    this.call('selectCellRange', [id, from, to]);
  }
  cellTab(dir: 1 | -1): void { this.call('cellTab', [dir]); }
  /** The `doc` argument is not supported: cells are always laid out from the current view. */
  cellTextEl(id: ElementId, r: number, c: number, _doc?: DesignDoc): { el: TextEl; box: Rect; dy: number } | null { return this.call('cellTextEl', [id, r, c]); }
  editingTextFrame(): { el: TextEl; hostId: ElementId; ox: number; oy: number } | null { return this.call('editingTextFrame', []); }
  cellAt(id: ElementId, pagePt: Vec2): { r: number; c: number } | null { return this.call('cellAt', [id, pagePt]); }
  setCell(id: ElementId, r: number, c: number, p: Parameters<TSEditor['setCell']>[3]): void { this.call('setCell', [id, r, c, patch(p)]); }
  tableTargetRange(id: ElementId): ReturnType<TSEditor['tableTargetRange']> { return this.call('tableTargetRange', [id]); }
  tableTargets(id: ElementId): ReturnType<TSEditor['tableTargets']> { return this.call('tableTargets', [id]); }
  setCells(id: ElementId, p: Parameters<TSEditor['setCells']>[1]): void { this.call('setCells', [id, patch(p)]); }
  setCellAlign(id: ElementId, align: 'start' | 'center' | 'end'): void { this.call('setCellAlign', [id, align]); }
  setCellBorders(id: ElementId, preset: Parameters<TSEditor['setCellBorders']>[1], border?: Parameters<TSEditor['setCellBorders']>[2]): void {
    this.call('setCellBorders', [id, preset, border]);
  }
  mergeCellRange(id: ElementId): void { this.call('mergeCellRange', [id]); }
  unmergeCellRange(id: ElementId): void { this.call('unmergeCellRange', [id]); }
  clearCells(id: ElementId): void { this.call('clearCells', [id]); }
  setTableStyle(id: ElementId, preset: Parameters<TSEditor['setTableStyle']>[1]): void { this.call('setTableStyle', [id, preset]); }
  setColumnWidth(id: ElementId, index: number | 'even', px?: number): undefined { this.call('setColumnWidth', [id, index, px]); }
  tableStructure(id: ElementId, action: 'addRow' | 'addCol' | 'delRow' | 'delCol', at: number): void { this.call('tableStructure', [id, action, at]); }

  // ------------------------------------------------------------------ animation & timeline

  previewAnimation(pageId?: PageId): void { this.call('previewAnimation', [pageId]); }
  stopPreview(): void { this.call('stopPreview', []); }
  /** Design time of the playhead; `now` is on the editor clock ({@link Editor.now}). */
  currentTime(now?: number): number { return this.call('currentTime', [now]); }
  play(): void { this.call('play', []); }
  pause(): void { this.call('pause', []); }
  togglePlay(): void { this.call('togglePlay', []); }
  seek(t: number | null): void { this.call('seek', [t]); }
  tick(): void { this.call('tick', []); }
  pageStart(pageId: PageId): number { return this.call('pageStart', [pageId]); }
  setPageDuration(pageId: PageId, ms: number): void { this.call('setPageDuration', [pageId, ms]); }
  setTiming(id: ElementId, timing: { startMs: number; endMs?: number } | null): void { this.call('setTiming', [id, timing]); }
  insertVideo(asset: Parameters<TSEditor['insertVideo']>[0], at?: Vec2): ReturnType<TSEditor['insertVideo']> { return this.call('insertVideo', [asset, at]); }
  splitAtPlayhead(): void { this.call('splitAtPlayhead', []); }
  audioClips(): AudioClip[] { return this.call('audioClips', []); }
  addAudio(asset: Parameters<TSEditor['addAudio']>[0], at?: number): string { return this.call('addAudio', [asset, at]); }
  updateAudio(id: string, p: Partial<AudioClip>, merge = true): void { this.call('updateAudio', [id, p, merge]); }
  removeAudio(id: string): void { this.call('removeAudio', [id]); }
  splitAudio(id: string, t?: number): void { this.call('splitAudio', [id, t]); }
  syncPagesToBeats(beatsMs: number[]): number { return this.call('syncPagesToBeats', [beatsMs]); }
  captions(): ReturnType<TSEditor['captions']> { return this.call('captions', []); }
  setCaptions(track: Parameters<TSEditor['setCaptions']>[0], merge = false): void { this.call('setCaptions', [track, merge]); }
  upsertCue(cue: Partial<CaptionCue> & { id?: string }): string { return this.call('upsertCue', [cue]); }
  setRecordKeyframes(on: boolean): void { this.call('setRecordKeyframes', [on]); }
  keyframeTime(id: ElementId): number | null { return this.call('keyframeTime', [id]); }
  setKeyframe(id: ElementId, prop: Parameters<TSEditor['setKeyframe']>[1], t: number, v: number, ease?: Parameters<TSEditor['setKeyframe']>[4]): void {
    this.call('setKeyframe', [id, prop, t, v, ease]);
  }
  removeKeyframe(id: ElementId, prop: Parameters<TSEditor['removeKeyframe']>[1], t: number): void { this.call('removeKeyframe', [id, prop, t]); }
  moveKeyframe(id: ElementId, from: number, to: number, prop?: Parameters<TSEditor['moveKeyframe']>[3]): void { this.call('moveKeyframe', [id, from, to, prop]); }
  clearKeyframes(id: ElementId): void { this.call('clearKeyframes', [id]); }

  // ------------------------------------------------------------------ text editing

  startTextEdit(id: ElementId, pagePt?: Vec2, selectWord = false): void { this.call('startTextEdit', [id, pagePt, selectWord]); }
  stopTextEdit(): void { this.call('stopTextEdit', []); }
  typeText(str: string): void { this.call('typeText', [str]); }
  deleteText(dir: -1 | 1, word = false): void { this.call('deleteText', [dir, word]); }
  moveCaret(key: 'left' | 'right' | 'up' | 'down' | 'home' | 'end', extend: boolean, word = false): void { this.call('moveCaret', [key, extend, word]); }
  plainText(el: TextEl): string { return this.call('plainText', [el]); }
  selectedText(): string { return this.call('selectedText', []); }
  setTextStyle(marks: Parameters<TSEditor['setTextStyle']>[0]): void { this.call('setTextStyle', [patch(marks)]); }
  setParagraphStyle(attrs: Parameters<TSEditor['setParagraphStyle']>[0]): void { this.call('setParagraphStyle', [patch(attrs)]); }
  currentTextStyle(): ReturnType<TSEditor['currentTextStyle']> { return this.call('currentTextStyle', []); }
  currentParagraph(): ReturnType<TSEditor['currentParagraph']> { return this.call('currentParagraph', []); }

  /** Patch every selected element; a function patch runs here on the view's elements (`null` skips). */
  updateSelected(p: Record<string, unknown> | ((el: Element) => Record<string, unknown> | null), deep = false, merge = true): void {
    if (typeof p !== 'function') {
      this.call('updateSelected', [patch(p), deep, merge]);
      return;
    }
    const patches: Record<string, Record<string, unknown> | null> = {};
    for (const el of deep ? this.selectedDeep() : this.selected()) {
      const x = p(el);
      if (x) patches[el.id] = patch(x);
    }
    this.call('updateSelectedMap', [patches, deep, merge]);
  }

  // ------------------------------------------------------------------ element commands

  addPageNumbers(o: { text?: string; position?: 'bottom-right' | 'bottom-center' | 'bottom-left' | 'top-right'; size?: number } = {}): number {
    return this.call('addPageNumbers', [o]);
  }
  insert(els: Element[], at?: Vec2, opts: { select?: boolean; edit?: boolean } = {}): ElementId[] { return this.call('insert', [els, at, opts]); }
  deleteSelection(): void { this.call('deleteSelection', []); }
  duplicate(): void { this.call('duplicate', []); }
  copy(): ClipboardPayload | null { return (this._clip = this.call('copy', [])); }
  cut(): ClipboardPayload | null { return (this._clip = this.call('cut', [])); }
  /**
   * Paste a payload, or the internal clipboard when `payload` is omitted or is the object `copy()`/`cut()`
   * returned (TS compares with its internal clipboard by identity: pasting it shifts the next paste).
   */
  paste(payload?: ClipboardPayload | null, offset = 20): void {
    if (payload === undefined || (payload !== null && payload === this._clip)) this.exec(() => this._raw.pasteClipboard(offset));
    else this.call('paste', [payload, offset]);
  }
  nudge(dx: number, dy: number): void { this.call('nudge', [dx, dy]); }
  arrange(where: 'forward' | 'backward' | 'front' | 'back'): void { this.call('arrange', [where]); }
  reorderTo(id: ElementId, after: ElementId | 'top' | 'bottom'): void { this.call('reorderTo', [id, after]); }
  align(edge: Parameters<TSEditor['align']>[0]): void { this.call('align', [edge]); }
  distribute(axis: 'x' | 'y'): void { this.call('distribute', [axis]); }
  tidy(): void { this.call('tidy', []); }
  flip(axis: 'x' | 'y'): void { this.call('flip', [axis]); }
  setBrandAdmin(admin: boolean): void {
    this._brandAdmin = Boolean(admin);
    this.call('setBrandAdmin', [admin]);
  }
  isBrandLocked(el: Element | undefined): boolean { return this.call('isBrandLocked', [el]); }
  setBrandLock(levels: { position?: boolean; content?: boolean; style?: boolean } | null): void { this.call('setBrandLock', [levels]); }
  toggleLock(): void { this.call('toggleLock', []); }
  group(): void { this.call('group', []); }
  ungroup(): void { this.call('ungroup', []); }

  // ------------------------------------------------------------------ vector editing (DS-100) and masks (DS-107)

  /** Finish the path being drawn with the pen (`close` joins it). Returns the new path's id. */
  penFinish(close = false): ElementId | null { return this.call('penFinish', [close]); }
  penCancel(): void { this.call('penCancel', []); }
  /** Edit the nodes of a shape (it becomes a path on the first edit). */
  startPathEdit(id: ElementId): boolean { return this.call('startPathEdit', [id]); }
  stopPathEdit(): void { this.call('stopPathEdit', []); }
  selectPathNodes(nodes: V.NodeRef[]): void { this.call('selectPathNodes', [nodes]); }
  /** The edited path's nodes in page space (for the overlay). */
  pathEditView(): V.PathEditView | null { return this.call('pathEditView', []); }
  nudgePathNodes(dx: number, dy: number): boolean { return this.call('nudgePathNodes', [dx, dy]); }
  /** Make the selected nodes smooth or corners (`null` toggles). */
  setPathNodesSmooth(smooth: boolean | null = null): boolean { return this.call('setPathNodesSmooth', [smooth]); }
  deletePathNodes(): boolean { return this.call('deletePathNodes', []); }
  addPathNode(): boolean { return this.call('addPathNode', []); }
  /** Combine the selected shapes into one path. */
  booleanOp(op: V.BoolOp): ElementId | null { return this.call('booleanOp', [op]); }
  /** Replace an element by a filled path (`d` in its own box): outline text. */
  replaceWithPath(id: ElementId, d: string, fill?: V.TextOutline['fill']): ElementId | null { return this.call('replaceWithPath', [id, d, fill]); }
  /** Replace an element by a group of elements in its own box (SVG as editable vectors). */
  replaceWithElements(id: ElementId, els: Element[]): ElementId | null { return this.call('replaceWithElements', [id, els]); }
  /** The top selected element masks the others. Returns the masked group. */
  maskSelection(mode: V.MaskMode = 'alpha'): ElementId | null { return this.call('maskSelection', [mode]); }
  /** The masked group the selection is (or is in). */
  maskedGroup(): ElementId | null { return this.call('maskedGroup', []); }
  setMaskMode(mode: V.MaskMode): boolean { return this.call('setMaskMode', [mode]); }
  releaseMask(): boolean { return this.call('releaseMask', []); }
  startCrop(id: ElementId): void { this.call('startCrop', [id]); }
  stopCrop(): void { this.call('stopCrop', []); }
  setCropZoom(id: ElementId, zoom: number): void { this.call('setCropZoom', [id, zoom]); }
  cropZoom(id: ElementId): number { return this.call('cropZoom', [id]); }
  /** The part of the photo shown in an image, a filled frame or a photo-filled shape (null when there is none). */
  mediaCrop(el: Element): ReturnType<TSEditor['mediaCrop']> { return this.call('mediaCrop', [el]); }
  replaceImage(id: ElementId, asset: AssetId, aspect: number): void { this.call('replaceImage', [id, asset, aspect]); }
  copyStyle(): ReturnType<TSEditor['copyStyle']> { return this.call('copyStyle', []); }
  cancelStyleBrush(): void { this.call('cancelStyleBrush', []); }
  pasteStyle(ids?: ElementId[], style?: Parameters<TSEditor['pasteStyle']>[1]): void { this.call('pasteStyle', [ids, style]); }
  find(query: string, opts: { matchCase?: boolean; wholeWord?: boolean } = {}): FindMatch[] { return this.call('find', [query, opts]); }
  revealMatch(m: FindMatch): void { this.call('revealMatch', [m]); }
  replace(query: string, replacement: string, opts: { matchCase?: boolean; wholeWord?: boolean; only?: FindMatch } = {}): number {
    return this.call('replace', [query, replacement, opts]);
  }
  setGeometry(g: Partial<{ x: number; y: number; w: number; h: number; rot: number }>): void { this.call('setGeometry', [g]); }
  setGuides(pageId: PageId, guides: Array<{ axis: 'x' | 'y'; pos: number }>): void { this.call('setGuides', [pageId, guides]); }
  addGuide(pageId: PageId, axis: 'x' | 'y', pos: number): void { this.call('addGuide', [pageId, axis, pos]); }
  gridCellAt(id: ElementId, pagePoint: Vec2): number { return this.call('gridCellAt', [id, pagePoint]); }
  setGridCell(id: ElementId, index: number, asset: AssetId | null): void { this.call('setGridCell', [id, index, asset]); }
  fillFrame(id: ElementId, asset: AssetId, aspect?: number): void { this.call('fillFrame', [id, asset, aspect]); }
  dropIntoContainer(pageId: PageId, local: Vec2, asset: AssetId): boolean { return this.call('dropIntoContainer', [pageId, local, asset]); }
  bindableElements(pageId?: PageId): Element[] {
    const els = this.call<Element[]>('bindableElements', [pageId]);
    const d = this.doc.elements;
    return els.map((e) => d[e.id] ?? e);
  }
  setDataBinding(id: ElementId, column: string | null): void { this.call('setDataBinding', [id, column]); }
  bulkCreate(pageId: PageId, rows: Array<Record<string, string>>, images: ReadonlyMap<string, { id: AssetId; w: number; h: number }> = new Map()): PageId[] {
    return this.call('bulkCreate', [pageId, rows, [...images.entries()]]);
  }

  // ------------------------------------------------------------------ pages & design

  addPage(after?: PageId | null): PageId | null { return this.call('addPage', [after]); }
  duplicatePage(pageId?: PageId): void { this.call('duplicatePage', [pageId]); }
  deletePage(pageId?: PageId): void { this.call('deletePage', [pageId]); }
  movePage(pageId: PageId, toIndex: number): void { this.call('movePage', [pageId, toIndex]); }
  updatePage(pageId: PageId, p: Partial<Page>): void { this.call('updatePage', [pageId, patch(p)]); }
  replaceColors(map: Record<string, string>): number { return this.call('replaceColors', [map]); }
  applyPalette(colors: string[]): number { return this.call('applyPalette', [colors]); }
  applyFonts(set: Parameters<TSEditor['applyFonts']>[0]): number { return this.call('applyFonts', [set]); }
  setLanguage(lang: string): void { this.call('setLanguage', [lang]); }
  resizeDesign(w: number, h: number, unit?: DesignDoc['meta']['dims']['unit'], scaleContent = false): void { this.call('resizeDesign', [w, h, unit, scaleContent]); }

  // ------------------------------------------------------------------ history, present, keys

  undo(): void { this.call('undo', []); }
  redo(): void { this.call('redo', []); }
  setPresenting(on: boolean): void { this.call('setPresenting', [on]); }
  textShortcut(key: string, mods: Mods & { ctrl?: boolean }, code?: string): boolean { return this.call('textShortcut', [key, mods, code]); }
  handleKey(key: string, mods: Mods & { ctrl?: boolean }, code?: string): boolean { return this.call('handleKey', [key, mods, code]); }

  // ------------------------------------------------------------------ engine additions

  /** Call any `Editor::dispatch` member by its TS name (golden replay, debugging). */
  dispatch(method: string, ...args: unknown[]): unknown {
    return this.call(method, args);
  }

  /** `templatePages(tpl, { w, h })` drawing ids from this editor's random stream (deterministic with `seed`). */
  templatePages(tpl: DesignDoc, target: { w: number; h: number }): ReturnType<typeof raw.templatePages> {
    return this.guard(() => this._raw.templatePages(tpl, target.w, target.h));
  }

  /** A `DocRef` for this editor's current document or view (for the pure helpers; normally implicit). */
  docRef(which: 'doc' | 'view' = 'doc'): raw.DocRef {
    return docRef(which === 'view' ? this.view : this.doc);
  }
}
