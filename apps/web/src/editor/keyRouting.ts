/**
 * Which keydowns outside the canvas still go to the editor (QA K-01). Clicking a toolbar button or a popover moves
 * focus off the canvas; canvas shortcuts (⌘A, ⌘G, ⌘D, ⌘[, Delete, arrows, …) keep working from there, as in Canva,
 * but never from a text field, a modal dialog, a menu, or a widget that uses the key itself.
 */

/** The parts of a DOM element this module reads (kept structural so it can be tested without a DOM). */
export interface TargetLike {
  tagName?: string;
  isContentEditable?: boolean;
  closest?: (selector: string) => unknown;
  getAttribute?: (name: string) => string | null;
}

export interface KeyLike {
  key: string;
  metaKey: boolean;
  ctrlKey: boolean;
  defaultPrevented: boolean;
  target: unknown;
}

const TEXT_ROLES = new Set(['textbox', 'searchbox', 'combobox', 'spinbutton', 'slider']);

/** Text inputs, selects, contenteditable and ARIA text widgets: keys there belong to the field. */
export function isEditableTarget(t: unknown): boolean {
  const el = t as TargetLike | null;
  if (!el || typeof el !== 'object') return false;
  const tag = el.tagName?.toUpperCase();
  if (tag === 'INPUT' || tag === 'TEXTAREA' || tag === 'SELECT') return true;
  if (el.isContentEditable) return true;
  const role = el.getAttribute?.('role');
  return Boolean(role && TEXT_ROLES.has(role));
}

/** Keys a focused control acts on itself (activate, move focus, close). */
const CONTROL_KEYS = new Set(['Tab', 'Enter', ' ', 'Escape']);
const NAV_KEYS = new Set(['ArrowLeft', 'ArrowRight', 'ArrowUp', 'ArrowDown', 'Home', 'End', 'PageUp', 'PageDown']);
/**
 * Widgets whose arrow keys move within them. Not role="toolbar": the element toolbar has no arrow-key navigation,
 * so arrows there nudge the selection, as on the canvas.
 */
const COMPOSITE = '[role="listbox"],[role="radiogroup"],[role="tablist"],[role="tree"],[role="grid"],[role="menubar"]';
/** Places where no canvas shortcut applies: the canvas handles its own keys; modals and menus own the keyboard. */
const NEVER = '.canvas-wrap,[aria-modal="true"],[role="dialog"],[role="menu"]';

/** Dialogs that own the keyboard while open, wherever focus is (the cookie banner doesn't). */
const OVERLAY = '[aria-modal="true"],[role="dialog"]:not(.consent)';

/** True when a dialog (Share, Download, page grid, recording studio, …) is open over the editor. */
export function overlayOpen(doc: { querySelector: (selector: string) => unknown }): boolean {
  return Boolean(doc.querySelector(OVERLAY));
}

/** True when a keydown that reached the window should run as a canvas shortcut. */
export function routesToCanvas(e: KeyLike): boolean {
  if (e.defaultPrevented) return false;
  const el = e.target as TargetLike | null;
  const isElement = Boolean(el && typeof el === 'object' && typeof el.closest === 'function');
  const tag = el?.tagName?.toUpperCase();
  // Nothing focused (focus fell back to <body>): the canvas is the only thing the key can be for.
  // (Escape stays with whatever popover listens for it; Tab moves focus.)
  if (!isElement || tag === 'BODY' || tag === 'HTML') return e.key !== 'Escape' && e.key !== 'Tab';
  if (isEditableTarget(el)) return false;
  if (el!.closest!(NEVER)) return false;
  if (CONTROL_KEYS.has(e.key)) return false;
  const cmd = e.metaKey || e.ctrlKey;
  // Inside an open popover only ⌘/Ctrl shortcuts go through; plain keys may be for its controls.
  if (!cmd && el!.closest!('.popover')) return false;
  if (NAV_KEYS.has(e.key) && el!.closest!(COMPOSITE)) return false;
  return true;
}

/**
 * Tab, Shift+Tab and Enter on the focused canvas (QA A11Y-01, WCAG 2.1.2 No Keyboard Trap). The canvas is one tab
 * stop, like an ARIA composite widget:
 * - nothing selected: Tab / Shift+Tab leave the canvas (browser focus order); Enter selects the first element;
 * - something selected: Tab / Shift+Tab move to the next / previous element; past the last (or before the first)
 *   the selection is cleared and focus leaves the canvas instead of wrapping;
 * - Esc clears the selection (the editor's own Escape), so Esc then Tab always leaves.
 * Whiteboards keep Tab = add a mind-map child while one element is selected (the editor decides).
 *
 * `siblings` are the elements Tab cycles through (children of the entered group or the active page), in order.
 */
export type CanvasTabAction = 'editor' | 'native' | 'leave' | 'enter';

export function canvasTabAction(
  e: { key: string; shiftKey: boolean; metaKey: boolean; ctrlKey: boolean; altKey: boolean },
  s: { selection: readonly string[]; siblings: readonly string[]; whiteboard: boolean },
): CanvasTabAction | null {
  if (e.metaKey || e.ctrlKey || e.altKey) return null;
  if (e.key === 'Enter') return !e.shiftKey && s.selection.length === 0 && s.siblings.length ? 'enter' : null;
  if (e.key !== 'Tab') return null;
  if (!s.selection.length) return 'native';
  if (s.whiteboard && !e.shiftKey && s.selection.length === 1) return 'editor';
  const i = s.siblings.indexOf(s.selection[0]!);
  if (i < 0) return 'editor';
  const atEdge = e.shiftKey ? i === 0 : i === s.siblings.length - 1;
  return atEdge ? 'leave' : 'editor';
}
