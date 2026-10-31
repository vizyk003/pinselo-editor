// SPDX-License-Identifier: Apache-2.0
/** Design ⇄ yjs update helpers backed by crates/oc-crdt (same Y.Doc layout as `@oc/crdt`'s `toY` / `fromY`). */
import type { DesignDoc } from './types/schema';
import { raw } from './wasm';

/** `Y.encodeStateAsUpdate` of a fresh doc holding `doc` (seed a browser `Y.Doc` with `Y.applyUpdate`). */
export const encodeDesign = (doc: DesignDoc): Uint8Array => raw.encodeDesign(doc);

/** The design a yjs update encodes (`fromY` after applying it to a fresh doc). */
export const decodeDesign = (update: Uint8Array): DesignDoc => raw.decodeDesign(update) as DesignDoc;
