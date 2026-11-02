/**
 * Quick-create tiles of the Home and public Landing hero (g06). A tile starts a size preset or an action;
 * tiles with a preset group get a 'See all' popover listing that group's sizes, and More opens the full Create dialog.
 */
import { formatDims, PRESET_CATEGORIES, SIZE_PRESETS, type PresetCategoryId, type SizePreset } from '@oc/engine';
import {
  ChevronDown, Clapperboard, FileText, Globe, Image as ImageIcon, MoreHorizontal, Plus, Presentation, Printer,
  Smartphone, Upload,
} from 'lucide-react';
import type { ReactNode } from 'react';
import { PopoverButton, useScrollFade } from '../../components/ui';
import { t, tk } from '../../i18n';

export type QuickAction = 'photo' | 'custom' | 'upload' | 'more';
export interface QuickTile {
  id: string;
  label: string;
  icon: ReactNode;
  /** Size preset the tile creates. */
  preset?: string;
  /** Non-preset tiles. */
  action?: QuickAction;
  /** Preset group listed by the tile's 'See all' popover. */
  group?: SizePreset['group'];
}

export const QUICK_TILES: QuickTile[] = [
  { id: 'presentation', label: tk('Presentation'), preset: 'presentation', group: 'Presentations', icon: <Presentation size={22} /> },
  { id: 'social', label: tk('Social media'), preset: 'ig-post', group: 'Social media', icon: <Smartphone size={22} /> },
  { id: 'video', label: tk('Video'), preset: 'video', group: 'Video', icon: <Clapperboard size={22} /> },
  { id: 'doc', label: tk('Doc'), preset: 'doc', group: 'Documents', icon: <FileText size={22} /> },
  { id: 'website', label: tk('Website'), preset: 'website', group: 'Web', icon: <Globe size={22} /> },
  { id: 'print', label: tk('Print'), preset: 'poster', group: 'Print', icon: <Printer size={22} /> },
  { id: 'photo', label: tk('Photo editor'), action: 'photo', icon: <ImageIcon size={22} /> },
  { id: 'custom', label: tk('Custom size'), action: 'custom', icon: <Plus size={22} /> },
  { id: 'upload', label: tk('Upload'), action: 'upload', icon: <Upload size={22} /> },
  { id: 'more', label: tk('More'), action: 'more', icon: <MoreHorizontal size={22} /> },
];

/** Sizes a tile's 'See all' popover lists before 'See all sizes' (popular ones first). */
const POPOVER_MAX = 10;
export function tileSizes(group: SizePreset['group'] | undefined): SizePreset[] {
  if (!group) return [];
  const all = SIZE_PRESETS.filter((p) => p.group === group);
  return [...all.filter((p) => p.popular), ...all.filter((p) => !p.popular)].slice(0, POPOVER_MAX);
}

/** The Create dialog category a tile's 'See all sizes' (or the Custom size tile) opens on. */
export function tileCategory(tile: QuickTile): PresetCategoryId | undefined {
  if (tile.action === 'custom') return 'custom';
  return PRESET_CATEGORIES.find((c) => c.group && c.group === tile.group)?.id;
}

/** Preset-only tiles; kept for existing importers of `QUICK` (PWA shortcuts, Landing). */
export const QUICK = QUICK_TILES.filter((q): q is QuickTile & { preset: string } => Boolean(q.preset))
  .map((q) => ({ label: q.label, preset: q.preset, icon: q.icon }));

/**
 * The `?create=` intent value a tile resumes after signing in: a preset id, or custom / upload / photo / more
 * (More opens the Create dialog on Suggestions, Custom size on its Custom size tab).
 */
export function tileIntent(tile: QuickTile): string {
  return tile.preset ?? tile.action!;
}

/** The `?create=` intent of a tile's 'See all sizes': the Create dialog on that category, else on Suggestions. */
export const seeAllIntent = (category: PresetCategoryId | undefined) => (category ? `cat:${category}` : 'more');

export type CreateIntent =
  | { kind: 'dialog'; category?: PresetCategoryId }
  | { kind: 'upload' | 'photo' }
  | { kind: 'preset'; preset: string };

/** What a `?create=` value asks Home to do (null for anything unknown). */
export function parseCreateIntent(v: string | null): CreateIntent | null {
  if (!v) return null;
  if (v === 'more') return { kind: 'dialog' };
  if (v === 'custom') return { kind: 'dialog', category: 'custom' };
  if (v === 'upload' || v === 'photo') return { kind: v };
  if (v.startsWith('cat:')) {
    const cat = PRESET_CATEGORIES.find((c) => c.id === v.slice(4));
    return cat ? { kind: 'dialog', category: cat.id } : null;
  }
  return SIZE_PRESETS.some((p) => p.id === v) ? { kind: 'preset', preset: v } : null;
}

/**
 * The tile row. `onPreset` creates a design from a preset id, `onAction` handles the non-preset tiles;
 * `busy` maps a tile id to a progress label shown in place of its name.
 */
export function QuickTiles({ onPreset, onAction, onSeeAll, busy }: {
  onPreset: (preset: string) => void; onAction: (a: QuickAction) => void; busy?: Record<string, string | null>;
  /** 'See all sizes' in a tile's popover; defaults to the More action. */
  onSeeAll?: (category: PresetCategoryId | undefined) => void;
}) {
  const row = useScrollFade();
  return (
    <div className="quick-create" ref={row}>
      {QUICK_TILES.map((tile) => {
        const group = tileSizes(tile.group);
        const label = busy?.[tile.id] ?? t(tile.label);
        return (
          <span key={tile.id} className={`qc-tile${group.length > 1 ? ' split' : ''}`}>
            <button onClick={() => (tile.preset ? onPreset(tile.preset) : onAction(tile.action!))} disabled={Boolean(busy?.[tile.id])}>
              <span className="ic">{tile.icon}</span>
              {label}
            </button>
            {group.length > 1 && (
              <PopoverButton menu className="qc-more" anchorClosest=".qc-tile" title={t('See all {kind} sizes', { kind: t(tile.label) })} label={<ChevronDown size={14} />} content={(close) => (
                <div className="menu qc-menu" role="menu" aria-label={t(tile.group!)}>
                  <div className="qc-menu-head" role="presentation">{t(tile.group!)}</div>
                  {group.map((p) => (
                    <button key={p.id} role="menuitem" onClick={() => { close(); onPreset(p.id); }}>
                      <span>{t(p.label)}</span>
                      <span className="faint">{formatDims(p.w, p.h, p.unit)}</span>
                    </button>
                  ))}
                  <button className="qc-menu-all" role="menuitem" onClick={() => { close(); if (onSeeAll) onSeeAll(tileCategory(tile)); else onAction('more'); }}>{t('See all sizes')}</button>
                </div>
              )} />
            )}
          </span>
        );
      })}
    </div>
  );
}
