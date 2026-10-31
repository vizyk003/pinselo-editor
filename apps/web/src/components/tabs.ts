/**
 * ARIA wiring for a row of tabs (WAI-ARIA tabs pattern, automatic activation): the selected tab is the row's only Tab
 * stop, Left/Right move to the previous/next tab (mirrored right-to-left) and Home/End to the first/last, and the
 * shown content is a tabpanel labelled by its tab. `base` is a per-row unique id (useId()).
 */
import type { KeyboardEvent } from 'react';
import { isRtl } from '../i18n';

const tabId = (base: string, tab: string) => `${base}-tab-${tab}`;
const panelId = (base: string) => `${base}-panel`;

/** Props for one tab button of `tabs`. */
export function tabProps<T extends string>(base: string, tabs: readonly T[], current: T, select: (tab: T) => void, tab: T) {
  return {
    role: 'tab' as const,
    id: tabId(base, tab),
    'aria-selected': current === tab,
    'aria-controls': panelId(base),
    tabIndex: current === tab ? 0 : -1,
    onClick: () => select(tab),
    onKeyDown: (e: KeyboardEvent<HTMLElement>) => {
      const i = tabs.indexOf(tab);
      const [prev, next] = isRtl() ? ['ArrowRight', 'ArrowLeft'] : ['ArrowLeft', 'ArrowRight'];
      const to = e.key === next ? (i + 1) % tabs.length : e.key === prev ? (i - 1 + tabs.length) % tabs.length
        : e.key === 'Home' ? 0 : e.key === 'End' ? tabs.length - 1 : -1;
      if (to < 0) return;
      e.preventDefault();
      select(tabs[to]!);
      document.getElementById(tabId(base, tabs[to]!))?.focus();
    },
  };
}

/** Props for the element that holds the selected tab's content. */
export const tabPanelProps = (base: string, current: string) => ({ role: 'tabpanel' as const, id: panelId(base), 'aria-labelledby': tabId(base, current) });
