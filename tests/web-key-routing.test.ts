/** Canvas shortcuts from outside the canvas (QA K-01): toolbar buttons and popovers yes, text fields and menus no. */
import { describe, expect, it } from 'vitest';
import { isEditableTarget, overlayOpen, routesToCanvas, type TargetLike } from '../apps/web/src/editor/keyRouting';

/** A fake element: `inside` lists the selectors its ancestors (or itself) match. */
const el = (tagName: string, inside: string[] = [], extra: Partial<TargetLike> = {}): TargetLike => ({
  tagName,
  isContentEditable: false,
  getAttribute: () => null,
  closest: (sel: string) => (sel.split(',').some((s) => inside.includes(s.trim())) ? {} : null),
  ...extra,
});
const key = (k: string, target: unknown, m: { meta?: boolean; ctrl?: boolean; prevented?: boolean } = {}) => ({
  key: k, metaKey: Boolean(m.meta), ctrlKey: Boolean(m.ctrl), defaultPrevented: Boolean(m.prevented), target,
});

describe('canvas key routing (K-01)', () => {
  it('routes shortcuts from a toolbar button or with nothing focused', () => {
    const btn = el('BUTTON', ['.ctx-toolbar', '[role="toolbar"]']);
    for (const k of ['a', 'g', 'd', '[', 'z']) expect(routesToCanvas(key(k, btn, { meta: true })), k).toBe(true);
    for (const k of ['Delete', 'Backspace', 'ArrowLeft', 'ArrowDown']) expect(routesToCanvas(key(k, btn)), k).toBe(true);
    expect(routesToCanvas(key('a', el('BODY'), { meta: true }))).toBe(true);
    expect(routesToCanvas(key('Enter', el('BODY')))).toBe(true);
    expect(routesToCanvas(key('Escape', el('BODY')))).toBe(false);
  });

  it('leaves keys a focused control uses to the control', () => {
    const btn = el('BUTTON');
    for (const k of ['Enter', ' ', 'Tab', 'Escape']) expect(routesToCanvas(key(k, btn)), k).toBe(false);
    expect(routesToCanvas(key('ArrowRight', el('BUTTON', ['[role="radiogroup"]'])))).toBe(false);
    expect(routesToCanvas(key('a', btn, { meta: true, prevented: true }))).toBe(false);
  });

  it('never routes from text fields, the canvas, menus or modal dialogs', () => {
    for (const t of [el('INPUT'), el('TEXTAREA'), el('SELECT'), el('DIV', [], { isContentEditable: true }), el('DIV', [], { getAttribute: () => 'textbox' })]) {
      expect(isEditableTarget(t)).toBe(true);
      expect(routesToCanvas(key('a', t, { meta: true }))).toBe(false);
      expect(routesToCanvas(key('Backspace', t))).toBe(false);
    }
    expect(isEditableTarget(el('BUTTON'))).toBe(false);
    for (const sel of ['.canvas-wrap', '[aria-modal="true"]', '[role="dialog"]', '[role="menu"]'])
      expect(routesToCanvas(key('a', el('BUTTON', [sel]), { meta: true })), sel).toBe(false);
  });

  it('inside a popover, only ⌘/Ctrl shortcuts go through', () => {
    const inPop = el('BUTTON', ['.popover']);
    expect(routesToCanvas(key('[', inPop, { meta: true }))).toBe(true);
    expect(routesToCanvas(key('a', inPop, { ctrl: true }))).toBe(true);
    expect(routesToCanvas(key('ArrowLeft', inPop))).toBe(false);
    expect(routesToCanvas(key('r', inPop))).toBe(false);
  });

  it('detects an open dialog, but not the cookie banner', () => {
    expect(overlayOpen({ querySelector: () => null })).toBe(false);
    expect(overlayOpen({ querySelector: (s: string) => (s.includes(':not(.consent)') ? {} : null) })).toBe(true);
  });
});

describe('canvas Tab / Enter (A11Y-01: no keyboard trap)', async () => {
  const { canvasTabAction } = await import('../apps/web/src/editor/keyRouting');
  const k = (key: string, shiftKey = false, m: { meta?: boolean } = {}) => ({ key, shiftKey, metaKey: Boolean(m.meta), ctrlKey: false, altKey: false });
  const sib = ['a', 'b', 'c'];
  it('nothing selected: Tab and Shift+Tab leave the canvas, Enter enters', () => {
    expect(canvasTabAction(k('Tab'), { selection: [], siblings: sib })).toBe('native');
    expect(canvasTabAction(k('Tab', true), { selection: [], siblings: sib })).toBe('native');
    expect(canvasTabAction(k('Enter'), { selection: [], siblings: sib })).toBe('enter');
    expect(canvasTabAction(k('Enter'), { selection: [], siblings: [] })).toBeNull();
  });
  it('a selection: Tab cycles, past either end it leaves instead of wrapping', () => {
    expect(canvasTabAction(k('Tab'), { selection: ['a'], siblings: sib })).toBe('editor');
    expect(canvasTabAction(k('Tab'), { selection: ['c'], siblings: sib })).toBe('leave');
    expect(canvasTabAction(k('Tab', true), { selection: ['b'], siblings: sib })).toBe('editor');
    expect(canvasTabAction(k('Tab', true), { selection: ['a'], siblings: sib })).toBe('leave');
    expect(canvasTabAction(k('Enter'), { selection: ['a'], siblings: sib })).toBeNull();
  });
  it('modified keys are not handled here', () => {
    expect(canvasTabAction(k('Tab', false, { meta: true }), { selection: ['a'], siblings: sib })).toBeNull();
  });
});
