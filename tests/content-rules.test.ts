// Plan 11 batch 3 (track B) rules: background-image crop, document fonts, reading order, heading levels
// (apps/web/src/lib/contentRules.ts) and the display preferences (apps/web/src/lib/appearance.ts).
import { describe, expect, it } from 'vitest';
import { coverCrop, documentFonts, headingLevel, readingOrder } from '../apps/web/src/lib/contentRules';
import { applyAppearance, APPEARANCE_DEFAULTS, blocksBareShortcut, messageMs, setAppearance } from '../apps/web/src/lib/appearance';

describe('content rules', () => {
  it('crops an image to cover the page, centred on its own crop', () => {
    // A square photo (full crop) on a 2:1 page keeps the full width and the middle half of the height.
    expect(coverCrop({ x: 0, y: 0, w: 1, h: 1 }, 1, 2)).toEqual({ x: 0, y: 0.25, w: 1, h: 0.5 });
    // A 2:1 photo on a square page keeps the middle half of the width.
    expect(coverCrop({ x: 0, y: 0, w: 1, h: 1 }, 2, 1)).toEqual({ x: 0.25, y: 0, w: 0.5, h: 1 });
    // Within an existing crop.
    expect(coverCrop({ x: 0.2, y: 0.2, w: 0.4, h: 0.4 }, 1, 2)).toEqual({ x: 0.2, y: 0.30000000000000004, w: 0.4, h: 0.2 });
    expect(coverCrop({ x: 0, y: 0, w: 1, h: 1 }, 0, 1)).toEqual({ x: 0, y: 0, w: 1, h: 1 });
  });

  it('lists the fonts a design uses once, in first-use order', () => {
    const run = (family: string) => ({ family, weight: 400, italic: false, size: 20, fill: { k: 'solid', color: { space: 'srgb', r: 0, g: 0, b: 0, a: 1 } }, tracking: 0 });
    const doc = {
      elements: {
        a: { type: 'text', body: { defaultRun: run('Inter'), paragraphs: [{ runs: [{ text: 'x', style: { family: 'Lora' } }, { text: 'y', style: {} }] }] } },
        b: { type: 'table', style: { family: 'Roboto' } },
        c: { type: 'text', deleted: { at: 1, by: 'u' }, body: { defaultRun: run('Pacifico'), paragraphs: [] } },
        d: { type: 'chart', style: { family: 'Inter' } },
      },
    } as never;
    expect(documentFonts(doc)).toEqual(['Inter', 'Lora', 'Roboto']);
  });

  it('reads boxes on one row left to right, rows top to bottom', () => {
    const boxes = [{ x: 300, y: 10, h: 40 }, { x: 10, y: 20, h: 40 }, { x: 10, y: 200, h: 40 }];
    expect([...boxes].sort(readingOrder).map((b) => b.x + ':' + b.y)).toEqual(['10:20', '300:10', '10:200']);
  });

  it('maps headline and subhead roles to heading levels', () => {
    expect(headingLevel('headline')).toBe(1);
    expect(headingLevel('subhead')).toBe(2);
    expect(headingLevel('body')).toBeNull();
    expect(headingLevel(undefined)).toBeNull();
  });
});

describe('display preferences', () => {
  it('sets the theme, contrast and motion attributes (system = none)', () => {
    const attrs = new Map<string, string>();
    const root = { setAttribute: (k: string, v: string) => void attrs.set(k, v), removeAttribute: (k: string) => void attrs.delete(k) };
    applyAppearance({ ...APPEARANCE_DEFAULTS, theme: 'dark', contrast: true, motion: 'reduce' }, root);
    expect(Object.fromEntries(attrs)).toEqual({ 'data-theme': 'dark', 'data-contrast': 'more', 'data-motion': 'reduce' });
    applyAppearance(APPEARANCE_DEFAULTS, root);
    expect(attrs.size).toBe(0);
  });

  it('blocks bare letter shortcuts only when asked, and lengthens messages', () => {
    const key = (k: string, mod = false) => ({ key: k, metaKey: mod, ctrlKey: false, altKey: false });
    setAppearance({ shortcutsNeedModifier: false, longerMessages: false });
    expect(blocksBareShortcut(key('t'))).toBe(false);
    expect(messageMs()).toBe(3200);
    setAppearance({ shortcutsNeedModifier: true, longerMessages: true });
    expect(blocksBareShortcut(key('t'))).toBe(true);
    expect(blocksBareShortcut(key('R'))).toBe(true);
    expect(blocksBareShortcut(key('t', true))).toBe(false);
    expect(blocksBareShortcut(key('Delete'))).toBe(false);
    expect(messageMs()).toBe(8000);
    setAppearance(APPEARANCE_DEFAULTS);
  });
});
