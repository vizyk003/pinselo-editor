/**
 * Design lists that keep up without a page reload: designs an AI assistant makes or changes through the connector
 * (or a teammate in another tab) show up on their own. A list reloads every few seconds while the page is visible,
 * and at once when the tab or window comes back; a reply identical to what's shown changes nothing on screen.
 */
import { useEffect, useRef } from 'react';

export const LIVE_REFRESH_MS = 10_000;

/** Run `refresh` every `everyMs` while the page is visible, and when it becomes visible again (one run at a time). */
export function useLiveRefresh(refresh: () => unknown, everyMs = LIVE_REFRESH_MS) {
  const latest = useRef(refresh);
  latest.current = refresh;
  useEffect(() => {
    let busy = false;
    const run = () => {
      if (busy || document.visibilityState !== 'visible') return;
      busy = true;
      Promise.resolve()
        .then(() => latest.current())
        .catch(() => {})
        .finally(() => { busy = false; });
    };
    const timer = window.setInterval(run, everyMs);
    const onShow = () => { if (document.visibilityState === 'visible') run(); };
    document.addEventListener('visibilitychange', onShow);
    window.addEventListener('focus', onShow);
    return () => {
      window.clearInterval(timer);
      document.removeEventListener('visibilitychange', onShow);
      window.removeEventListener('focus', onShow);
    };
  }, [everyMs]);
}

/** The new list, or the one already shown when nothing in it changed (so React keeps the screen as it is). */
export function keepIfSame<T>(prev: T[] | null, next: T[]): T[] {
  if (prev && prev.length === next.length && prev.every((p, i) => JSON.stringify(p) === JSON.stringify(next[i]))) return prev;
  return next;
}
