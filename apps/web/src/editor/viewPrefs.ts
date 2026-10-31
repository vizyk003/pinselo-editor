/** Per-viewer editor view settings (ED-061/062): rulers & guides, margins, print bleed. Stored locally. */
import { useSyncExternalStore } from 'react';

export interface ViewPrefs { rulers: boolean; margins: boolean; bleed: boolean }
const KEY = 'oc:view-prefs';
const DEFAULTS: ViewPrefs = { rulers: false, margins: false, bleed: false };

function load(): ViewPrefs {
  try {
    return { ...DEFAULTS, ...(JSON.parse(localStorage.getItem(KEY) ?? '{}') as Partial<ViewPrefs>) };
  } catch {
    return DEFAULTS;
  }
}

let prefs = load();
const listeners = new Set<() => void>();

export const getViewPrefs = (): ViewPrefs => prefs;

export function setViewPrefs(patch: Partial<ViewPrefs>) {
  prefs = { ...prefs, ...patch };
  try {
    localStorage.setItem(KEY, JSON.stringify(prefs));
  } catch {
    /* private mode: keep in memory */
  }
  for (const l of listeners) l();
}

export function useViewPrefs(): ViewPrefs {
  return useSyncExternalStore((cb) => {
    listeners.add(cb);
    return () => listeners.delete(cb);
  }, () => prefs);
}

/** Margin inset (page px) and bleed (3 mm at 96 dpi). */
export const marginOf = (w: number, h: number) => Math.round(Math.min(w, h) * 0.05);
export const BLEED_PX = (3 / 25.4) * 96;
