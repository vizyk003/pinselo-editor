// SPDX-License-Identifier: Apache-2.0
/**
 * Text measurement backed by oc-text (parley + swash) in WebAssembly — the same engine the server renders with.
 *
 * `new EngineTextService(manifest)` takes the font manifest (`/api/fonts/manifest.json`); the host fetches font
 * files and hands over the bytes with `registerFont(bytes)`, which finds the face by the SHA-256 pinned in the
 * manifest. Attach it with `editor.textService = service`: the editor then measures in Rust without crossing the
 * boundary. It also implements the TS `TextService` interface, so it can be used anywhere one is expected.
 */
import type { TextService } from './types/editor-core';
import type { Rect, TextBody, TextEffect, TextEl } from './types/schema';
import { raw } from './wasm';

/** A font manifest (`fixtures/fonts/manifest.json` shape). */
export interface FontManifest {
  version: number;
  families: FontFamily[];
}

export interface FontFace { weight: number; italic: boolean; file: string; sha256: string; bytes: number }

export interface FontFamily {
  family: string;
  slug: string;
  category: string;
  license: string;
  /** `'google'`: fetched on demand from the Google Fonts catalogue. */
  source?: 'google' | (string & {});
  faces: FontFace[];
}

/** Fetches font bytes by manifest-relative path (browser: `fetch`; Node: `readFile`). */
export type FontLoader = (file: string) => Promise<ArrayBuffer | Uint8Array>;

export const FALLBACK_FAMILY = 'Inter';

/** A face a layout needed and did not have. */
export interface MissingFace { family: string; weight: number; italic: boolean; file: string }

export class EngineTextService implements TextService {
  /** The wasm-bindgen handle (shared with the editor, and later with the renderer). */
  readonly raw: raw.TextService;
  private readonly loadListeners = new Set<(file: string) => void>();

  constructor(manifest: FontManifest) {
    this.raw = new raw.TextService(manifest);
  }

  /** Register a font file's bytes (looked up by content hash). Returns its manifest path. */
  registerFont(bytes: Uint8Array | ArrayBuffer): string {
    const file = this.raw.registerFont(bytes instanceof Uint8Array ? bytes : new Uint8Array(bytes));
    for (const l of this.loadListeners) l(file);
    return file;
  }

  /** Register bytes for a manifest path (faces without a pinned hash: Google catalogue, uploads). */
  registerFontFile(file: string, bytes: Uint8Array | ArrayBuffer): void {
    this.raw.registerFontFile(file, bytes instanceof Uint8Array ? bytes : new Uint8Array(bytes));
    for (const l of this.loadListeners) l(file);
  }

  /** Called after each registered face (re-measure auto-height text: `editor.remeasureText()`). */
  onLoad(fn: (file: string) => void): () => void {
    this.loadListeners.add(fn);
    return () => this.loadListeners.delete(fn);
  }

  addFamily(family: FontFamily): void {
    this.raw.addFamily(family);
  }

  hasFamily(family: string): boolean {
    return this.raw.hasFamily(family);
  }

  /** Manifest files still needed to lay out `body` exactly. */
  missingFonts(body: TextBody): string[] {
    return this.raw.missingFonts(body);
  }

  get generation(): number {
    return this.raw.generation;
  }

  height(el: TextEl, width: number): number {
    return this.raw.height(el, width);
  }

  offsetAt(el: TextEl, x: number, y: number): number {
    return this.raw.offsetAt(el, x, y);
  }

  verticalMove(el: TextEl, offset: number, dir: -1 | 1): number {
    return this.raw.verticalMove(el, offset, dir);
  }

  wordAt(el: TextEl, offset: number): [number, number] {
    const [a, b] = this.raw.wordAt(el, offset);
    return [a!, b!];
  }

  length(el: TextEl): number {
    return this.raw.length(el);
  }

  curvedSize(el: TextEl): { w: number; h: number } | null {
    return this.raw.curvedSize(el) as { w: number; h: number } | null;
  }

  /** Drop cached layouts of one element (or all). */
  dispose(key?: string): void {
    this.raw.dispose(key);
  }
}

/**
 * The font registry (`@oc/text` `FontRegistry`), over the Rust engine's: one set of fonts for measuring, caret
 * geometry and drawing. Faces load lazily through `loader` — bundled files, Google Fonts faces and uploaded brand
 * fonts alike, all by manifest path.
 *
 * The TS engine loaded missing faces whenever it laid text out (`requestFonts`). oc-text measures with the faces
 * it has and records the ones it lacked; {@link FontRegistry.pump} fetches each of those once, registers it and
 * notifies `onLoad` listeners (the painter repaints, the editor re-measures, like the TS did on `onLoad`). The
 * painter and {@link TextEngine.layout} pump after each call; hosts that measure without drawing call `pump()`
 * (e.g. on editor changes).
 */
export class FontRegistry {
  /** The editor-facing text service on the same Rust engine (`editor.textService = fonts.service`). */
  readonly service: EngineTextService;
  private readonly byFamily = new Map<string, FontFamily>();
  private readonly inflight = new Map<string, Promise<void>>();
  private readonly failed = new Set<string>();
  private readonly listeners = new Set<() => void>();
  private pumping = false;

  constructor(readonly manifest: FontManifest, private readonly loader: FontLoader) {
    this.service = new EngineTextService(manifest);
    this.service.raw.reportMissingFaces(true);
    for (const f of manifest.families) this.byFamily.set(f.family, f);
  }

  /** Bumped whenever a face finishes loading. */
  get generation(): number {
    return this.service.generation;
  }

  families(): FontFamily[] {
    return this.manifest.families;
  }

  has(family: string): boolean {
    return this.byFamily.has(family);
  }

  /** Closest available face: exact italic match preferred, then nearest weight. */
  pickFace(family: string, weight: number, italic: boolean): { family: FontFamily; face: FontFace } {
    const fam = this.byFamily.get(family) ?? this.byFamily.get(FALLBACK_FAMILY) ?? this.manifest.families[0]!;
    const pool = fam.faces.filter((f) => f.italic === italic);
    const faces = pool.length ? pool : fam.faces;
    let best = faces[0]!;
    for (const f of faces) if (Math.abs(f.weight - weight) < Math.abs(best.weight - weight)) best = f;
    return { family: fam, face: best };
  }

  isLoaded(family: string, weight: number, italic: boolean): boolean {
    return this.service.raw.isFileLoaded(this.pickFace(family, weight, italic).face.file);
  }

  /** Ensure the face used for (family, weight, italic) is registered. Resolves when ready. */
  ensure(family: string, weight: number, italic: boolean): Promise<void> {
    return this.ensureFile(this.pickFace(family, weight, italic).face.file);
  }

  /** Load every listed face. */
  async ensureAll(requests: Array<{ family: string; weight: number; italic: boolean }>): Promise<void> {
    await Promise.all(requests.map((r) => this.ensure(r.family, r.weight, r.italic)));
  }

  /** Load one manifest file (once; concurrent calls share the fetch). */
  loadFile(file: string): Promise<void> {
    return this.ensureFile(file);
  }

  private ensureFile(file: string): Promise<void> {
    if (this.service.raw.isFileLoaded(file)) return Promise.resolve();
    let p = this.inflight.get(file);
    if (!p) {
      p = this.loader(file).then(
        (buf) => {
          this.inflight.delete(file);
          this.service.registerFontFile(file, buf instanceof Uint8Array ? buf : new Uint8Array(buf));
          for (const l of this.listeners) l();
        },
        (e: unknown) => {
          this.inflight.delete(file);
          this.failed.add(file);
          throw e;
        },
      );
      this.inflight.set(file, p);
    }
    return p;
  }

  /**
   * Load the faces layouts reported missing since the last call (each once; a failed face is not retried).
   * Returns the loads started.
   */
  pump(): Promise<void>[] {
    const r = this.service.raw;
    if (this.pumping || r.pendingMissingFaces === 0) return [];
    this.pumping = true;
    try {
      const faces = r.takeMissingFaces() as MissingFace[];
      return faces.filter((f) => !this.failed.has(f.file)).map((f) => this.ensureFile(f.file).catch(() => {}));
    } finally {
      this.pumping = false;
    }
  }

  /** Resolves once no face is loading (pumping first: faces reported missing start loading). */
  async idle(): Promise<void> {
    for (;;) {
      this.pump();
      if (!this.inflight.size) return;
      await Promise.allSettled([...this.inflight.values()]);
    }
  }

  /** Called after each face that finishes loading. */
  onLoad(fn: () => void): () => void {
    this.listeners.add(fn);
    return () => this.listeners.delete(fn);
  }

  /** Add (or replace) a family at runtime (uploaded brand fonts, workspace fonts); faces load lazily. */
  addFamily(f: FontFamily): void {
    const i = this.manifest.families.findIndex((x) => x.family === f.family);
    if (i >= 0) this.manifest.families[i] = f;
    else this.manifest.families.push(f);
    this.byFamily.set(f.family, f);
    this.service.addFamily(f);
  }

  /** Family name actually used for layout (falls back when the family is unknown). */
  resolveFamily(family: string): string {
    return this.byFamily.has(family) ? family : FALLBACK_FAMILY;
  }
}

/** A laid-out body: its box and the Rust layout (caret and selection geometry). */
export interface TextLayout {
  readonly width: number;
  readonly height: number;
  /** Flat (UTF-16) length of the text. */
  readonly length: number;
  readonly raw: raw.TextLayoutRef;
}

/**
 * Paragraph layout and caret geometry (`@oc/text` `TextEngine`) from oc-text — the engine the painter draws with,
 * under the same cache keys, so the caret sits on the drawn glyphs.
 */
export class TextEngine {
  constructor(readonly fonts: FontRegistry) {}

  /** Lay out `body` at `width` under `key` (an element id). Missing faces start loading (`onLoad` re-lays out). */
  layout(key: string, body: TextBody, width: number, effect?: TextEffect): TextLayout {
    const l = this.fonts.service.raw.layout(key, body, width, effect ?? null);
    this.fonts.pump();
    return { width: l.width, height: l.height, length: l.length, raw: l };
  }

  /** Start loading every face the body needs; true when all are already available. */
  requestFonts(body: TextBody): boolean {
    const missing = this.fonts.service.missingFonts(body);
    for (const file of missing) void this.fonts.loadFile(file).catch(() => {});
    return missing.length === 0;
  }

  caretRect(l: TextLayout, offset: number): Rect {
    return l.raw.caretRect(offset) as Rect;
  }

  selectionRects(l: TextLayout, from: number, to: number): Rect[] {
    return l.raw.selectionRects(from, to) as Rect[];
  }

  offsetAt(l: TextLayout, x: number, y: number): number {
    return l.raw.offsetAt(x, y);
  }

  wordAt(l: TextLayout, offset: number): [number, number] {
    const [a, b] = l.raw.wordAt(offset);
    return [a!, b!];
  }

  verticalMove(l: TextLayout, offset: number, dir: -1 | 1): number {
    return l.raw.verticalMove(offset, dir);
  }

  dispose(key: string): void {
    this.fonts.service.dispose(key);
  }

  disposeAll(): void {
    this.fonts.service.dispose();
  }
}
