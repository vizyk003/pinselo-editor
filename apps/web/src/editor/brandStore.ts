/**
 * Brand Hub (A8) client state: the user's brand kits across workspaces and the open design's brand context
 * (governing team, brand-admin flag, brand controls, template link, approval status).
 */
import { useSyncExternalStore } from 'react';
import { brandHub, type BrandWorkspace, type DesignBrand, type Kit } from '../api';

interface State { kits: Kit[]; workspaces: BrandWorkspace[]; loaded: boolean; design: (DesignBrand & { id: string }) | null; activeKit: string | null }

let state: State = { kits: [], workspaces: [], loaded: false, design: null, activeKit: null };
const listeners = new Set<() => void>();
const set = (p: Partial<State>) => {
  state = { ...state, ...p };
  for (const l of listeners) l();
};

let kitsReq: Promise<void> | null = null;
export const brandStore = {
  get: () => state,
  subscribe(fn: () => void) {
    listeners.add(fn);
    return () => listeners.delete(fn);
  },
  loadKits(force = false): Promise<void> {
    if (!force && kitsReq) return kitsReq;
    kitsReq = brandHub.kits().then((r) => set({ kits: r.kits, workspaces: r.workspaces, loaded: true })).catch(() => set({ loaded: true }));
    return kitsReq;
  },
  async loadDesign(id: string): Promise<DesignBrand | null> {
    try {
      const d = await brandHub.design(id);
      set({ design: { ...d, id } });
      return d;
    } catch {
      return null;
    }
  },
  setActiveKit(id: string | null) {
    set({ activeKit: id });
  },
};

export function useBrandHub(): State {
  return useSyncExternalStore(brandStore.subscribe, brandStore.get);
}

const isAdmin = (role: string | null | undefined) => role === 'owner' || role === 'admin';

/** Kits of the team that governs the open design (falls back to every kit the user can see). */
export function governingKits(s: State): Kit[] {
  const ws = s.design?.workspaceId;
  const own = ws ? s.kits.filter((k) => k.workspaceId === ws) : [];
  return own.length ? own : s.kits;
}

/** Brand colours offered first (all kits; the governing team's kits first). */
export function brandColors(s: State): string[] {
  const ordered = [...governingKits(s), ...s.kits];
  const out = new Set<string>();
  for (const k of ordered) {
    for (const c of k.colors) out.add(c.hex.toLowerCase());
    for (const p of k.palettes) for (const c of p.colors) out.add(c.hex.toLowerCase());
  }
  return [...out];
}

/** Brand controls in force for this user on the open design (admins are exempt; empty kits never lock anyone out). */
export function activeControls(s: State): { colors: string[] | null; fonts: string[] | null; restrictUploads: boolean; requireTemplate: boolean } {
  const d = s.design;
  const c = d && !isAdmin(d.role) ? d.controls : {};
  const kits = d?.workspaceId ? s.kits.filter((k) => k.workspaceId === d.workspaceId) : [];
  const colors = kits.flatMap((k) => [...k.colors, ...k.palettes.flatMap((p) => p.colors)]).map((x) => x.hex.toLowerCase());
  const fonts = [...new Set(kits.flatMap((k) => Object.values(k.fonts).filter(Boolean) as string[]))];
  return {
    colors: c.colors && colors.length ? [...new Set(colors)] : null,
    fonts: c.fonts && fonts.length ? fonts : null,
    restrictUploads: Boolean(c.restrictUploads),
    requireTemplate: Boolean(c.requireTemplate),
  };
}
