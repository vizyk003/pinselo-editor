/**
 * Editor File menu (g88) and editor deep links, pure parts: which actions a person may take on the open design
 * (mirroring the API's rules so the menu never offers something the server refuses), the info line, and the
 * one-shot URL intents dashboard cards use to open a dialog straight away.
 */
import type { Role } from '../api';

/** One-shot URL intents → the editor dialog they open. Names are part of the URL contract (dashboard cards link to them). */
export const INTENT_PARAMS = { download: 'export', publishTemplate: 'publish' } as const;
export type IntentDialog = (typeof INTENT_PARAMS)[keyof typeof INTENT_PARAMS];

const isOn = (v: string | null) => v !== null && v !== '0' && v !== 'false';

/**
 * The dialog a URL asks for: `?download=1` opens Download for anyone who can open the design;
 * `?publishTemplate=1` opens Publish as template for people who can edit it (the menu hides it otherwise).
 */
export function intentDialog(search: string | URLSearchParams, canEdit: boolean): IntentDialog | null {
  const p = typeof search === 'string' ? new URLSearchParams(search) : search;
  if (isOn(p.get('download'))) return INTENT_PARAMS.download;
  if (isOn(p.get('publishTemplate')) && canEdit) return INTENT_PARAMS.publishTemplate;
  return null;
}

/** The query string without the one-shot intents (anything else, such as `?share=1`, stays); null when it has none. */
export function withoutIntents(search: string): string | null {
  const p = new URLSearchParams(search);
  const keys = Object.keys(INTENT_PARAMS).filter((k) => p.has(k));
  if (!keys.length) return null;
  for (const k of keys) p.delete(k);
  const s = p.toString();
  return s ? `?${s}` : '';
}

export interface FileActions { star: boolean; move: boolean; unfile: boolean; trash: boolean }

/**
 * Same rules as the API: anyone who can open a design may star it (stars are personal); moving it into a folder
 * needs edit rights, taking it out of folders needs the owner role; only the person who owns it can trash it
 * (team admins rank as owners for editing, but the trash belongs to the design's owner).
 */
export function fileActions(d: { owner_id: string }, role: Role, userId: string | null | undefined): FileActions {
  const canEdit = role === 'owner' || role === 'edit';
  return { star: true, move: canEdit, unfile: role === 'owner', trash: Boolean(userId) && d.owner_id === userId };
}

/** Name of the folder a design sits in, when the person can see that folder (another person's personal folders stay private). */
export function folderName(folders: ReadonlyArray<{ id: string; name: string }> | null, folderId: string | null): string | null {
  if (!folderId || !folders) return null;
  return folders.find((f) => f.id === folderId)?.name ?? null;
}
