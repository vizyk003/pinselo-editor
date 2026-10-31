// SPDX-License-Identifier: Apache-2.0
/** Z-order commands through the facade: the paint order of the page changes as the toolbar buttons promise. */
import * as Y from 'yjs';
import { describe, expect, test } from 'vitest';
import { Editor, childrenOf, emptyDesign, encodeDesign, makeShape, sortedPages, type DesignDoc, type Element } from '../src/index';

function setup() {
  const doc: DesignDoc = emptyDesign({ w: 800, h: 600 });
  const page = sortedPages(doc)[0]!;
  const els = ['a', 'b', 'c'].map((n, i) => {
    const e = makeShape({ kind: 'rect', xf: { x: 40 + i * 30, y: 40 + i * 30, w: 200, h: 200, rot: 0 }, fill: '#888888' }) as Element;
    return { ...e, parentId: page.id, z: `a${i}`, name: n } as Element;
  });
  for (const e of els) doc.elements[e.id] = e;
  const ydoc = new Y.Doc();
  Y.applyUpdate(ydoc, encodeDesign(doc));
  const ed = new Editor({ ydoc, actor: 'test' }, { x: 0, y: 0, zoom: 1, width: 1200, height: 900 });
  const order = () => childrenOf(ed.doc, page.id).map((e) => (e as Element & { name?: string }).name);
  return { ed, els, order };
}

describe('arrange', () => {
  test('to back / to front / backward / forward', () => {
    const { ed, els, order } = setup();
    expect(order()).toEqual(['a', 'b', 'c']);
    ed.select([els[2]!.id]);
    ed.arrange('back');
    expect(order()).toEqual(['c', 'a', 'b']);
    ed.arrange('front');
    expect(order()).toEqual(['a', 'b', 'c']);
    ed.arrange('backward');
    expect(order()).toEqual(['a', 'c', 'b']);
    ed.arrange('forward');
    expect(order()).toEqual(['a', 'b', 'c']);
  });
});
