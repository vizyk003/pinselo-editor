/** Content Planner client (A16): providers, connected accounts, scheduled posts. */
import { ApiError } from '../api';
import { tk } from '../i18n';

async function req<T>(method: string, url: string, body?: unknown): Promise<T> {
  const res = await fetch(url, {
    method, credentials: 'same-origin',
    headers: body === undefined ? {} : { 'content-type': 'application/json' },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  if (!res.ok) {
    let j: { error?: string; message?: string } = {};
    try { j = await res.json(); } catch { /* not json */ }
    throw new ApiError(res.status, j.error ?? 'error', j.message);
  }
  return res.json() as Promise<T>;
}

export type ProviderId = 'local' | 'facebook' | 'instagram' | 'linkedin' | 'pinterest' | 'x' | 'tiktok' | 'youtube' | 'threads' | 'bluesky';
export interface Provider {
  id: ProviderId; label: string; auth: 'oauth' | 'app-password' | 'none'; configured: boolean; env: string[];
  limits: { maxChars: number; images: number; video: boolean; imageTypes: string[]; maxImageBytes?: number };
}
export interface SocialAccount {
  id: string; provider: ProviderId; displayName: string; handle: string | null; avatarUrl: string | null; status: 'ok' | 'reauth';
  workspaceId: string | null; mine?: boolean; boards?: Array<{ id: string; name: string }>;
  /** Set when connected with a pasted Meta system-user token (absent: provider login). */
  connectedVia?: 'system_user';
  /** True when the account publishes through its team's own Meta app. */
  workspaceApp?: boolean;
}
/** A team's own Meta app (never includes the secret). */
export interface MetaApp { provider: 'meta'; appId: string; appName: string | null; graphVersion: string | null; updatedAt: string }
export interface TeamMetaApp { app: MetaApp | null; accounts: number; serverApp: boolean; canManage: boolean }
export type PostStatus = 'draft' | 'in_review' | 'changes_requested' | 'approved' | 'scheduled' | 'publishing' | 'published' | 'failed';
/** Who publishes a post: Pinselo through connected accounts, or an outside tool through /v1/planner. */
export type Publisher = 'pinselo' | 'external';
export type Channel = 'instagram' | 'facebook';
export interface TargetResult { status: 'published' | 'failed' | 'retrying' | 'unknown'; id?: string; url?: string | null; error?: string; note?: string; at: string }
export interface SocialPost {
  id: string; designId: string; designTitle: string | null; pages: number[]; format: 'png' | 'jpg' | 'mp4'; caption: string; hashtags: string[];
  accounts: string[]; options: Record<string, unknown>; scheduledAt: string | null; timezone: string; status: PostStatus;
  results: Record<string, TargetResult>; error: string | null; publishedAt: string | null; createdAt: string; ownerId: string;
  publisher: Publisher; channels: Channel[]; fields: Record<string, string>;
  submittedBy: string | null; submittedAt: string | null;
  approvedBy: string | null; approvedByName?: string | null; approvedAt: string | null; approvedVersion: number | null;
  /** The design's current content version (list endpoint). */
  designVersion?: number | null;
  /** The team (brand) the post belongs to, through its design; null for a personal post (list endpoint). */
  workspaceId?: string | null; workspaceName?: string | null;
  claimedAt: string | null;
}
export interface PostInput {
  designId: string; pages: number[]; format: 'png' | 'jpg' | 'mp4'; caption: string; hashtags: string[]; accounts: string[];
  options?: Record<string, unknown>; scheduledAt: string | null; timezone: string; status?: 'draft' | 'scheduled' | 'in_review';
  publisher?: Publisher; channels?: Channel[]; fields?: Record<string, string>;
}
export interface PostEvent { id: number; accountId: string | null; kind: string; message: string | null; createdAt: string }

export const socialApi = {
  providers: () => req<{ providers: Provider[] }>('GET', '/api/social/providers'),
  accounts: () => req<{ accounts: SocialAccount[] }>('GET', '/api/social/accounts'),
  connect: (provider: ProviderId, creds?: { identifier: string; password: string }) => req<{ url?: string; ok?: boolean }>('POST', `/api/social/connect/${provider}`, creds ?? {}),
  /** Facebook / Instagram login through a team's own Meta app; the accounts are shared with the team. */
  connectForTeam: (provider: 'facebook' | 'instagram', workspaceId: string) => req<{ url: string }>('POST', `/api/social/connect/${provider}`, { workspaceId }),
  metaApp: (workspaceId: string) => req<TeamMetaApp>('GET', `/api/workspaces/${workspaceId}/meta-app`),
  saveMetaApp: (workspaceId: string, b: { appId: string; appSecret?: string; graphVersion?: string | null }) =>
    req<{ app: MetaApp }>('PUT', `/api/workspaces/${workspaceId}/meta-app`, b),
  removeMetaApp: (workspaceId: string) => req('DELETE', `/api/workspaces/${workspaceId}/meta-app`),
  /** Connect the Pages (or their Instagram accounts) a Meta system-user token can manage. */
  connectSystemUser: (workspaceId: string, provider: 'facebook' | 'instagram', token: string) =>
    req<{ accounts: SocialAccount[] }>('POST', `/api/workspaces/${workspaceId}/meta-app/accounts`, { provider, token }),
  updateAccount: (id: string, b: { workspaceId?: string | null; defaultBoardId?: string }) => req('PATCH', `/api/social/accounts/${id}`, b),
  disconnect: (id: string) => req('DELETE', `/api/social/accounts/${id}`),
  posts: (q: { from?: string; to?: string; account?: string; designId?: string; status?: string; workspace?: string } = {}) =>
    req<{ posts: SocialPost[] }>('GET', `/api/social/posts?${new URLSearchParams(Object.entries(q).filter(([, v]) => v) as [string, string][])}`),
  create: (b: PostInput) => req<{ post: SocialPost }>('POST', '/api/social/posts', b),
  update: (id: string, b: Partial<Omit<PostInput, 'designId'>>) => req<{ post: SocialPost }>('PATCH', `/api/social/posts/${id}`, b),
  publishNow: (id: string) => req<{ post: SocialPost }>('POST', `/api/social/posts/${id}/publish-now`),
  remove: (id: string) => req('DELETE', `/api/social/posts/${id}`),
  events: (id: string) => req<{ events: PostEvent[] }>('GET', `/api/social/posts/${id}/events`),
  submit: (id: string) => req<{ post: SocialPost }>('POST', `/api/social/posts/${id}/submit`),
  approve: (id: string) => req<{ post: SocialPost }>('POST', `/api/social/posts/${id}/approve`),
  requestChanges: (id: string, note: string) => req<{ post: SocialPost }>('POST', `/api/social/posts/${id}/request-changes`, { note }),
};

/** Editorial fields a post can carry (shown in the post dialog and details, in this order). */
export const EDITORIAL_FIELDS: Array<{ key: string; label: string; multiline?: boolean }> = [
  { key: 'ref', label: tk('Post ID') },
  { key: 'formatLabel', label: tk('Post format') },
  { key: 'pillar', label: tk('Pillar') },
  { key: 'source', label: tk('Source') },
  { key: 'topic', label: tk('Topic and hook'), multiline: true },
  { key: 'factIds', label: tk('Fact IDs') },
  { key: 'reviewerNote', label: tk('Reviewer note'), multiline: true },
];

/** Shown beside the caption for reviewers who don't read the post's language; never posted. */
export const TRANSLATION_FIELD = { key: 'translationHu', label: tk('Hungarian translation') };

export const CHANNEL_LABEL: Record<Channel, string> = { instagram: 'Instagram', facebook: 'Facebook' };

/** A person can still change the post (not while it is being published, not once it is). */
export const isEditable = (p: SocialPost) => p.status !== 'publishing' && p.status !== 'published';

export const PROVIDER_COLOR: Record<ProviderId, string> = {
  local: '#64748b', facebook: '#1877f2', instagram: '#e1306c', linkedin: '#0a66c2', pinterest: '#e60023', x: '#111111',
  tiktok: '#010101', youtube: '#ff0000', threads: '#000000', bluesky: '#1185fe',
};
export const PROVIDER_SHORT: Record<ProviderId, string> = {
  local: '↓', facebook: 'f', instagram: 'IG', linkedin: 'in', pinterest: 'P', x: '𝕏', tiktok: '♪', youtube: '▶', threads: '@', bluesky: 'b',
};

export const STATUS_LABEL: Record<PostStatus, string> = {
  draft: tk('Draft'), in_review: tk('In review'), changes_requested: tk('Changes requested'), approved: tk('Approved'),
  scheduled: tk('Scheduled'), publishing: tk('Publishing'), published: tk('Published'), failed: tk('Failed'),
};

// ---------------------------------------------------------------- time zones

/** Offset (ms) of a time zone at a given instant: wall-clock-in-zone minus UTC. */
function tzOffset(tz: string, at: Date): number {
  const parts = new Intl.DateTimeFormat('en-US', { timeZone: tz, hourCycle: 'h23', year: 'numeric', month: '2-digit', day: '2-digit', hour: '2-digit', minute: '2-digit', second: '2-digit' }).formatToParts(at);
  const g = (k: string) => Number(parts.find((p) => p.type === k)?.value);
  return Date.UTC(g('year'), g('month') - 1, g('day'), g('hour') % 24, g('minute'), g('second')) - Math.floor(at.getTime() / 1000) * 1000;
}

/** "2026-10-11T09:30" wall time in `tz` → UTC ISO string. */
export function zonedToIso(local: string, tz: string): string {
  const [d, tm] = local.split('T');
  const [y, mo, da] = d!.split('-').map(Number);
  const [h, mi] = (tm ?? '00:00').split(':').map(Number);
  const guess = Date.UTC(y!, mo! - 1, da!, h!, mi!);
  let ts = guess - tzOffset(tz, new Date(guess));
  ts = guess - tzOffset(tz, new Date(ts)); // second pass handles DST edges
  return new Date(ts).toISOString();
}

/** UTC ISO → "YYYY-MM-DDTHH:mm" wall time in `tz` (for datetime-local inputs). */
export function isoToZoned(iso: string, tz: string): string {
  const at = new Date(iso);
  const w = new Date(at.getTime() + tzOffset(tz, at));
  return w.toISOString().slice(0, 16);
}

export const localTz = () => Intl.DateTimeFormat().resolvedOptions().timeZone || 'UTC';

export function timeZones(): string[] {
  const list = (Intl as unknown as { supportedValuesOf?: (k: string) => string[] }).supportedValuesOf?.('timeZone') ?? [];
  const tz = localTz();
  return [...new Set([tz, 'UTC', ...list])];
}

// ---------------------------------------------------------------- captions

export function composeText(caption: string, hashtags: string[]): string {
  const tags = hashtags.map((h) => h.replace(/^#+/, '').trim()).filter(Boolean).map((h) => `#${h}`);
  const body = caption.trim();
  const missing = tags.filter((tg) => !new RegExp(`(^|\\s)${tg.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}(?![\\p{L}\\p{N}_])`, 'u').test(body));
  return [body, missing.join(' ')].filter(Boolean).join('\n\n');
}

export function charCount(text: string, provider?: string): number {
  let s = text;
  if (provider === 'x') s = s.replace(/https?:\/\/\S+/g, 'x'.repeat(23));
  const Seg = (Intl as unknown as { Segmenter?: new (l?: string, o?: { granularity: string }) => { segment(s: string): Iterable<unknown> } }).Segmenter;
  return Seg ? [...new Seg(undefined, { granularity: 'grapheme' }).segment(s)].length : [...s].length;
}

const STOP = new Set(('a an and are as at be by for from has have in is it its of on or our the this that to was we will with you your new now get just more '
  + 'de la le les des et en un une el los las y que der die das und mit für').split(' '));

/** Hashtag suggestions from the design's own words: frequent, distinctive, title-weighted. */
export function suggestHashtags(texts: string[], title: string, max = 10): string[] {
  const score = new Map<string, { n: number; word: string }>();
  const add = (s: string, w: number) => {
    for (const m of s.matchAll(/[\p{L}][\p{L}\p{N}]{2,24}/gu)) {
      const word = m[0];
      const key = word.toLowerCase();
      if (STOP.has(key) || /^\d+$/.test(key)) continue;
      const cur = score.get(key) ?? { n: 0, word: word[0]!.toUpperCase() === word[0] && word.slice(1).toLowerCase() === word.slice(1) ? word.toLowerCase() : word };
      cur.n += w + Math.min(word.length, 10) / 20;
      score.set(key, cur);
    }
  };
  add(title, 3);
  for (const s of texts) add(s, 1);
  return [...score.values()].sort((a, b) => b.n - a.n).slice(0, max).map((x) => x.word.replace(/[^\p{L}\p{N}_]/gu, ''));
}
