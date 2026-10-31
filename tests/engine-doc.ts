/** Test helpers: designs as browser Y.Docs and stores, the way apps/web holds them (on @oc/engine). */
import * as Y from 'yjs';
import { Editor, encodeDesign, type DesignDoc, type EngineStore, type Viewport } from '@oc/engine';

/** A fresh Y.Doc holding `doc` (what `toY(doc)` gave). */
export function ydocOf(doc: DesignDoc): Y.Doc {
  const ydoc = new Y.Doc();
  Y.applyUpdate(ydoc, encodeDesign(doc));
  return ydoc;
}

/** An editor over a fresh Y.Doc holding `doc`. */
export function editorOf(doc: DesignDoc | Y.Doc, viewport: Viewport = { x: 0, y: 0, zoom: 1, width: 1200, height: 1200 }): Editor {
  return new Editor(doc instanceof Y.Doc ? doc : ydocOf(doc), viewport);
}

/** The store of an editor over `doc` (`new DocStore(toY(doc))` in the TS packages). */
export function storeOf(doc: DesignDoc): EngineStore {
  return editorOf(doc).store;
}
