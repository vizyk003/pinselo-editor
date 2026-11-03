/**
 * Display and accessibility preferences (plan 11 g03; C7 AX-103): theme (system, light, dark), more contrast, reduced
 * motion, single-key shortcuts and how long messages stay on screen. Stored per device and applied as attributes on
 * <html> (`data-theme`, `data-contrast`, `data-motion`) that styles.css reads.
 */
import { useSyncExternalStore } from 'react';

export type ThemePref = 'system' | 'light' | 'dark';
export type MotionPref = 'system' | 'reduce';
export interface Appearance {
  theme: ThemePref;
  /** Stronger text, borders and focus rings. */
  contrast: boolean;
  /** `reduce` stops animations and transitions even when the system setting allows them. */
  motion: MotionPref;
  /** Editor shortcuts that are a single letter (T, R, C, L) only work with ⌘/Ctrl or Alt held. */
  shortcutsNeedModifier: boolean;
  /** Messages (toasts) stay on screen longer. */
  longerMessages: boolean;
}

const KEY = 'oc:appearance';
export const APPEARANCE_DEFAULTS: Appearance = { theme: 'light', contrast: false, motion: 'system', shortcutsNeedModifier: false, longerMessages: false };

function load(): Appearance {
  try {
    const v = JSON.parse(localStorage.getItem(KEY) ?? '{}') as Partial<Appearance>;
    return {
      theme: v.theme === 'system' || v.theme === 'dark' ? v.theme : 'light',
      contrast: v.contrast === true,
      motion: v.motion === 'reduce' ? 'reduce' : 'system',
      shortcutsNeedModifier: v.shortcutsNeedModifier === true,
      longerMessages: v.longerMessages === true,
    };
  } catch {
    return APPEARANCE_DEFAULTS;
  }
}

let prefs: Appearance = typeof localStorage === 'undefined' ? APPEARANCE_DEFAULTS : load();
const listeners = new Set<() => void>();

/** Set the <html> attributes for `a` (system theme and motion = no attribute, so the media queries decide). */
export function applyAppearance(a: Appearance = prefs, root: { setAttribute: (k: string, v: string) => void; removeAttribute: (k: string) => void } | undefined = typeof document === 'undefined' ? undefined : document.documentElement) {
  if (!root) return;
  if (a.theme === 'system') root.removeAttribute('data-theme');
  else root.setAttribute('data-theme', a.theme);
  if (a.contrast) root.setAttribute('data-contrast', 'more');
  else root.removeAttribute('data-contrast');
  if (a.motion === 'reduce') root.setAttribute('data-motion', 'reduce');
  else root.removeAttribute('data-motion');
}

export const getAppearance = (): Appearance => prefs;

export function setAppearance(patch: Partial<Appearance>) {
  prefs = { ...prefs, ...patch };
  try {
    localStorage.setItem(KEY, JSON.stringify(prefs));
  } catch {
    /* private mode: keep in memory */
  }
  applyAppearance(prefs);
  for (const l of listeners) l();
}

export function useAppearance(): Appearance {
  return useSyncExternalStore((cb) => {
    listeners.add(cb);
    return () => listeners.delete(cb);
  }, () => prefs);
}

/** True when animations should be skipped: the in-app setting, or the system setting. */
export function reducedMotion(): boolean {
  return prefs.motion === 'reduce' || (typeof matchMedia !== 'undefined' && matchMedia('(prefers-reduced-motion: reduce)').matches);
}

/** How long a message (toast) stays up, ms. */
export const messageMs = () => (prefs.longerMessages ? 8000 : 3200);

/** With "shortcuts need a modifier" on: a bare letter key must not run an editor shortcut. */
export function blocksBareShortcut(e: { key: string; metaKey: boolean; ctrlKey: boolean; altKey: boolean }): boolean {
  return prefs.shortcutsNeedModifier && !e.metaKey && !e.ctrlKey && !e.altKey && /^[a-z]$/i.test(e.key);
}
