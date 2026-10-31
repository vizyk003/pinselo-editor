// SPDX-License-Identifier: Apache-2.0
/**
 * Documents cross the boundary as `DocRef`s, cached per JS object (see crates/oc-wasm/src/doc.rs).
 *
 * * Objects an `Editor` handed out (`editor.doc`, `editor.view`) resolve to the editor's own parsed snapshot
 *   (an `Arc` clone in Rust): no serialisation at all.
 * * `Pick<DesignDoc, …>` objects whose parts are the parts of such a document (`{ elements: view.elements }`)
 *   resolve to that document's snapshot as well.
 * * Anything else is parsed once and cached for the lifetime of the object (`WeakMap`; the Rust side is freed by
 *   wasm-bindgen's FinalizationRegistry once the object is garbage).
 */
import { raw } from './wasm';

type Doc = object;

/** Resolves an editor-owned document object to a `DocRef` (or `null` if the editor has moved on). */
export interface DocOwner {
  refFor(doc: Doc): raw.DocRef | null;
}

const refs = new WeakMap<Doc, raw.DocRef>();
const owned = new WeakMap<Doc, DocOwner>();
const PARTS = ['meta', 'pages', 'elements', 'assets'] as const;
/** Document part object (e.g. `doc.elements`) → the document it belongs to. */
const partOf = new WeakMap<object, Doc>();

/** Register a document object produced by an editor. */
export function registerDoc(doc: Doc, owner: DocOwner): void {
  owned.set(doc, owner);
  const d = doc as Record<string, unknown>;
  for (const k of PARTS) {
    const p = d[k];
    // The newest document wins: parts are shared between revisions (structural sharing), and the newest one is
    // the one most likely to still be current.
    if (p && typeof p === 'object') partOf.set(p, doc);
  }
}

/** The document an object's parts all come from (a `Pick` of an editor document), if any. */
function ownerDoc(doc: Doc): Doc | null {
  const d = doc as Record<string, unknown>;
  let found: Doc | null = null;
  for (const k of Object.keys(d)) {
    if (!(PARTS as readonly string[]).includes(k)) return null;
    const p = d[k];
    const o = p && typeof p === 'object' ? partOf.get(p) : undefined;
    if (!o || (found && o !== found)) return null;
    if ((o as Record<string, unknown>)[k] !== p) return null;
    found = o;
  }
  return found;
}

export function docRef(doc: Doc): raw.DocRef {
  const hit = refs.get(doc);
  if (hit) return hit;
  let r: raw.DocRef | null = null;
  const owner = owned.get(doc);
  if (owner) r = owner.refFor(doc);
  else {
    const o = ownerDoc(doc);
    if (o) r = docRef(o);
  }
  r ??= raw.DocRef.fromJs(doc);
  refs.set(doc, r);
  return r;
}
