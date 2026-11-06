/** Thin typed fetch wrapper for the Pinselo API (same origin; Vite proxies /api in dev). */
import type { AssetRef, DesignDoc, EmbedProvider } from '@oc/engine';

export class ApiError extends Error {
  /** `reason`: a 409 link_conflict's detail (e.g. `changed_during_signin` from the sign-in code step). */
  constructor(readonly status: number, readonly code: string, message?: string, readonly reason?: string) {
    super(message ?? code);
  }
}

export async function req<T>(method: string, url: string, body?: unknown, headers: Record<string, string> = {}): Promise<T> {
  const init: RequestInit = { method, credentials: 'same-origin', headers: { ...headers } };
  if (body instanceof FormData) init.body = body;
  else if (body !== undefined) {
    init.body = JSON.stringify(body);
    (init.headers as Record<string, string>)['content-type'] = 'application/json';
  }
  const res = await fetch(url, init);
  if (!res.ok) {
    let code = 'error';
    let message: string | undefined;
    let reason: string | undefined;
    try {
      const j = (await res.json()) as { error?: string; message?: string; reason?: string };
      code = j.error ?? code;
      message = j.message;
      reason = j.reason;
    } catch {
      /* not json */
    }
    // Fair-use limits surface as a global notice (see LimitHost).
    if (res.status === 402) window.dispatchEvent(new CustomEvent('oc:limit', { detail: { message } }));
    throw new ApiError(res.status, code, message, reason);
  }
  const ct = res.headers.get('content-type') ?? '';
  return (ct.includes('application/json') ? res.json() : res.blob()) as Promise<T>;
}

export interface User { id: string; email: string; name: string; color: string; verified?: boolean; mfa?: boolean; staff?: boolean; locale?: string }
export type LoginResult = { user: User } | { mfaRequired: true; ticket: string };
export type Role = 'owner' | 'edit' | 'comment' | 'view';
export interface DesignSummary {
  id: string; title: string; surface: string; width: number; height: number; unit: string; page_count: number;
  link_access: 'restricted' | 'view' | 'comment' | 'edit'; owner_id: string; owner_name: string; folder_id: string | null;
  trashed_at: string | null; created_at: string; updated_at: string; starred: boolean; role?: Role; public_view?: boolean;
}
/** PDF Print settings sent with format 'pdf-print' (bleed and marks, PDF/X standard, colour management). */
export interface PrintExportOptions {
  bleedMm: number; cropMarks: boolean; registrationMarks?: boolean; colorBars?: boolean;
  standard?: 'none' | 'pdfx4' | 'pdfx1a'; flatten?: boolean;
  /** Bundled profile id, or `{ id }` of an uploaded one. */
  profile?: string | { id: string };
  intent?: 'relative' | 'perceptual';
}
export interface TemplateSummary { id: string; title: string; category: string; preset: string; tags: string[]; width: number; height: number; team?: boolean; brand?: boolean; workspaceId?: string | null; version?: number }
export interface Folder { id: string; name: string; parent_id: string | null; designs: number; workspace_id: string; team: string | null }
export interface Comment {
  id: string; parent_id: string | null; page_id: string | null; element_id: string | null; body: string; resolved: boolean;
  created_at: string; edited_at: string | null; author_id: string; author: string; author_color: string;
  assignee_id: string | null; assignee: string | null; assignee_color: string | null; due_on: string | null;
  reactions: Array<{ emoji: string; count: number; mine: boolean }>;
}
export interface Task { id: string; body: string; due_on: string | null; created_at: string; design_id: string; design_title: string; author: string; author_color: string }
export interface UploadedAsset { id: string; mime: string; kind: 'image' | 'svg' | 'video' | 'audio'; bytes: number; w: number; h: number; name: string; sha256: string; durationMs?: number | null; hasAudio?: boolean; animated?: boolean }

export const api = {
  me: () => req<{ user: User | null }>('GET', '/api/auth/me'),
  login: (email: string, name: string) => req<LoginResult>('POST', '/api/auth/login', { email, name }),
  logout: () => req('POST', '/api/auth/logout'),

  designs: (params: Record<string, string> = {}) => req<{ designs: DesignSummary[] }>('GET', `/api/designs?${new URLSearchParams(params)}`),
  design: (id: string) => req<{ design: DesignSummary & { role: Role } }>('GET', `/api/designs/${id}`),
  createDesign: (b: { preset?: string; w?: number; h?: number; unit?: string; title?: string; templateId?: string; folderId?: string | null; workspaceId?: string | null }) =>
    req<{ id: string }>('POST', '/api/designs', b),
  updateDesign: (id: string, b: { title?: string; folderId?: string | null; linkAccess?: string; starred?: boolean }) => req('PATCH', `/api/designs/${id}`, b),
  trashDesign: (id: string, permanent = false) => req('DELETE', `/api/designs/${id}${permanent ? '?permanent=1' : ''}`),
  restoreDesign: (id: string) => req('POST', `/api/designs/${id}/restore`),
  /** Make a copy; with `w` and `h`, a copy resized to that size ("Copy and resize", g101). */
  copyDesign: (id: string, b?: { w: number; h: number; unit?: string; scaleContent?: boolean; title?: string }) => req<{ id: string; resized?: boolean }>('POST', `/api/designs/${id}/copy`, b),
  ticket: (id: string) => req<{ token: string; role: Role }>('GET', `/api/designs/${id}/ticket`),
  designDoc: (id: string) => req<{ doc: DesignDoc }>('GET', `/api/designs/${id}/doc`),

  shares: (id: string) => req<{ linkAccess: DesignSummary['link_access']; owner: User; people: Array<User & { role: Role }> }>('GET', `/api/designs/${id}/shares`),
  share: (id: string, email: string, role: Role) => req('POST', `/api/designs/${id}/shares`, { email, role }),
  unshare: (id: string, userId: string) => req('DELETE', `/api/designs/${id}/shares/${userId}`),

  versions: (id: string) => req<{ versions: Array<{ id: string; label: string | null; created_at: string; author: string | null }> }>('GET', `/api/designs/${id}/versions`),
  version: (id: string, vid: string) => req<{ doc: DesignDoc }>('GET', `/api/designs/${id}/versions/${vid}`),
  /** Latest checkpoint an AI assistant saved before changing the design (AI connector). */
  aiCheckpoint: (id: string) => req<{ checkpoint: { id: string; agent: string; createdAt: string } | null }>('GET', `/api/designs/${id}/ai-checkpoint`),
  saveVersion: (id: string, label?: string) => req('POST', `/api/designs/${id}/versions`, { label }),

  exportJob: (id: string, b: { format: string; quality?: string; print?: PrintExportOptions }) => req<{ id: string }>('POST', `/api/designs/${id}/export-jobs`, b),
  jobStatus: (job: string) => req<{ status: 'queued' | 'running' | 'done' | 'error'; progress: number; label: string; error?: string; filename?: string }>('GET', `/api/export-jobs/${job}`),
  jobFile: (job: string) => req<Blob>('GET', `/api/export-jobs/${job}/file`),
  exportDesign: (id: string, b: { format: string; pages?: number[]; scale?: number; quality?: number; transparent?: boolean; print?: PrintExportOptions; watermark?: ExportWatermark }) =>
    req<Blob>('POST', `/api/designs/${id}/export`, b),

  templates: (params: Record<string, string> = {}) =>
    req<{ templates: TemplateSummary[]; categories: Array<{ category: string; n: number }> }>('GET', `/api/templates?${new URLSearchParams(params)}`),
  template: (id: string) => req<{ template: TemplateSummary & { doc: DesignDoc } }>('GET', `/api/templates/${id}`),

  assets: () => req<{ assets: UploadedAsset[]; usedBytes: number; quotaBytes: number }>('GET', '/api/assets'),
  upload: (file: File) => {
    const fd = new FormData();
    fd.append('file', file);
    return req<{ asset: AssetRef }>('POST', '/api/assets', fd);
  },
  deleteAsset: (id: string) => req('DELETE', `/api/assets/${id}`),

  folders: () => req<{ folders: Folder[] }>('GET', '/api/folders'),
  createFolder: (name: string, workspaceId?: string) => req<{ id: string }>('POST', '/api/folders', { name, ...(workspaceId ? { workspaceId } : {}) }),
  renameFolder: (id: string, name: string) => req('PATCH', `/api/folders/${id}`, { name }),
  deleteFolder: (id: string) => req('DELETE', `/api/folders/${id}`),

  comments: (id: string) => req<{ comments: Comment[] }>('GET', `/api/designs/${id}/comments`),
  addComment: (id: string, b: { body: string; parentId?: string; pageId?: string; elementId?: string; assigneeId?: string; dueOn?: string }) => req<{ id: string }>('POST', `/api/designs/${id}/comments`, b),
  resolveComment: (cid: string, resolved: boolean) => req('PATCH', `/api/comments/${cid}`, { resolved }),
  updateComment: (cid: string, b: { body?: string; assigneeId?: string | null; dueOn?: string | null }) => req('PATCH', `/api/comments/${cid}`, b),
  react: (cid: string, emoji: string) => req<{ on: boolean }>('POST', `/api/comments/${cid}/reactions`, { emoji }),
  collaborators: (id: string) => req<{ people: Array<{ id: string; name: string; email: string; color: string }> }>('GET', `/api/designs/${id}/collaborators`),
  tasks: () => req<{ tasks: Task[] }>('GET', '/api/tasks'),
  deleteComment: (cid: string) => req('DELETE', `/api/comments/${cid}`),
};

export interface BrandKit { name: string; colors: Array<{ name: string; hex: string }>; fonts: { heading?: string; subheading?: string; body?: string }; logos: string[] }
export interface Notification { id: string; kind: 'mention' | 'comment' | 'share' | 'reply' | 'assign' | 'publish'; design_id: string | null; body: string; read_at: string | null; created_at: string; actor: string | null; actor_color: string | null; design_title: string | null }
export const brand = {
  get: () => req<{ kit: BrandKit }>('GET', '/api/brand'),
  put: (kit: BrandKit) => req('PUT', '/api/brand', kit),
};

export const notifications = {
  list: () => req<{ items: Notification[]; unread: number }>('GET', '/api/notifications'),
  read: (ids?: string[]) => req('POST', '/api/notifications/read', { ids }),
};

export const publicApi = {
  design: (id: string, password?: string) => req<{ title: string; owner: string; doc: DesignDoc }>('GET', `/api/public/designs/${id}`, undefined, password ? { 'x-view-password': password } : {}),
  setPublic: (id: string, enabled: boolean, extra: { expiresAt?: string | null; password?: string | null } = {}) => req('POST', `/api/designs/${id}/public`, { enabled, ...extra }),
  settings: (id: string) => req<{ enabled: boolean; expiresAt: string | null; hasPassword: boolean }>('GET', `/api/designs/${id}/public`),
};

export interface AccessRequest { id: string; role: Role; message: string | null; created_at: string; user_id: string; name: string; email: string; color: string }
export const accessApi = {
  request: (id: string, role: Role, message?: string) => req<{ ok: boolean; already?: boolean }>('POST', `/api/designs/${id}/request-access`, { role, message }),
  list: (id: string) => req<{ requests: AccessRequest[] }>('GET', `/api/designs/${id}/access-requests`),
  decide: (rid: string, approve: boolean, role?: Role) => req('POST', `/api/access-requests/${rid}`, { approve, role }),
  transfer: (id: string, userId: string) => req('POST', `/api/designs/${id}/transfer`, { userId }),
};

export const thumbUrl = (id: string, v?: string) => `/api/designs/${id}/thumbnail.png${v ? `?v=${encodeURIComponent(v)}` : ''}`;
export const templateThumbUrl = (id: string) => `/api/templates/${id}/thumbnail.png`;
/** Uploads come from the user-content origin when one is configured (see /api/config). */
let assetOrigin = '';
export function setAssetOrigin(o: string) {
  assetOrigin = o.replace(/\/$/, '');
}
export const assetUrl = (id: string) => `${assetOrigin}/api/assets/${id}`;
export function download(blob: Blob, filename: string) {
  const url = URL.createObjectURL(blob);
  const a = document.createElement('a');
  a.href = url;
  a.download = filename;
  document.body.appendChild(a);
  a.click();
  a.remove();
  setTimeout(() => URL.revokeObjectURL(url), 10_000);
}

// ---------------------------------------------------------------- accounts, teams, safety

export interface AuthProviders { providers: Array<{ id: string; label: string }>; passkeys: boolean; devLogin: boolean; termsVersion: string; signup?: 'open' | 'invite' }
export interface SessionRow { id: string; device: string; ip: string | null; created_at: string; last_seen_at: string; current: boolean }
export interface AccountInfo {
  user: User; hasPassword: boolean; recoveryCodesLeft: number;
  passkeys: Array<{ id: string; name: string | null; created_at: string; last_used_at: string | null }>;
  oauth: Array<{ provider: string; email: string; created_at: string }>;
  prefs: { mentions: boolean; comments: boolean; shares: boolean; product: boolean } | null;
}
export type TeamRole = 'owner' | 'admin' | 'designer' | 'member';
export interface Workspace { id: string; name: string; personal: boolean; role: TeamRole; members: number }
export interface Member { id: string; name: string; email: string; color: string; role: TeamRole; created_at: string }
export interface Invite { id: string; email: string; role: TeamRole; expires_at: string }

export const authApi = {
  providers: () => req<AuthProviders>('GET', '/api/auth/providers'),
  signup: (b: { email: string; password: string; name: string; birthYear: number; acceptTerms: true }) => req<{ user: User }>('POST', '/api/auth/signup', b),
  login: (email: string, password: string) => req<LoginResult>('POST', '/api/auth/login', { email, password }),
  mfa: (ticket: string, code: string) => req<{ user: User }>('POST', '/api/auth/mfa', { ticket, code }),
  magic: (email: string, next?: string) => req('POST', '/api/auth/magic', { email, next }),
  magicVerify: (b: { token: string; name?: string; birthYear?: number; acceptTerms?: boolean }) =>
    req<LoginResult | { needsProfile: true; email: string }>('POST', '/api/auth/magic/verify', b),
  verify: (token: string) => req('POST', '/api/auth/verify', { token }),
  resendVerify: () => req('POST', '/api/auth/verify/resend', {}),
  resetRequest: (email: string) => req('POST', '/api/auth/reset/request', { email }),
  reset: (token: string, password: string) => req('POST', '/api/auth/reset', { token, password }),
  passkeyLoginOptions: () => req<Record<string, unknown>>('POST', '/api/auth/passkey/login/options', {}),
  passkeyLoginVerify: (response: unknown) => req<LoginResult>('POST', '/api/auth/passkey/login/verify', { response }),
  passkeyRegisterOptions: () => req<Record<string, unknown>>('POST', '/api/auth/passkey/register/options', {}),
  passkeyRegisterVerify: (response: unknown, name?: string) => req('POST', '/api/auth/passkey/register/verify', { response, name }),
};

export const accountApi = {
  get: () => req<AccountInfo>('GET', '/api/account'),
  update: (b: { name?: string; locale?: string }) => req('PATCH', '/api/account', b),
  prefs: (b: NonNullable<AccountInfo['prefs']>) => req('PUT', '/api/account/email-preferences', b),
  password: (current: string | undefined, password: string) => req('POST', '/api/account/password', { current, password }),
  sessions: () => req<{ sessions: SessionRow[] }>('GET', '/api/account/sessions'),
  revokeSession: (id: string) => req('DELETE', `/api/account/sessions/${id}`),
  revokeOthers: () => req('POST', '/api/account/sessions/revoke-others', {}),
  totpSetup: () => req<{ secret: string; otpauthUrl: string }>('POST', '/api/account/2fa/setup', {}),
  totpEnable: (code: string) => req<{ recoveryCodes: string[] }>('POST', '/api/account/2fa/enable', { code }),
  totpDisable: (code: string) => req('POST', '/api/account/2fa/disable', { code }),
  removePasskey: (id: string) => req('DELETE', `/api/account/passkeys/${encodeURIComponent(id)}`),
  exportData: () => req<{ jobId: string }>('POST', '/api/account/export', {}),
  exportStatus: (job: string) => req<{ status: string; progress: number; label: string } | Blob>('GET', `/api/account/exports/${job}`),
  deleteAccount: (b: { password?: string; code?: string; confirm: 'DELETE' }) => req('DELETE', '/api/account', b),
};

export const teamApi = {
  list: () => req<{ workspaces: Workspace[] }>('GET', '/api/workspaces'),
  create: (name: string) => req<{ id: string }>('POST', '/api/workspaces', { name }),
  rename: (id: string, name: string) => req('PATCH', `/api/workspaces/${id}`, { name }),
  members: (id: string) => req<{ members: Member[]; invites: Invite[] }>('GET', `/api/workspaces/${id}/members`),
  invite: (id: string, emails: string[], role: Exclude<TeamRole, 'owner'>) => req<{ sent: number }>('POST', `/api/workspaces/${id}/invites`, { emails, role }),
  cancelInvite: (id: string, inv: string) => req('DELETE', `/api/workspaces/${id}/invites/${inv}`),
  accept: (token: string) => req<{ workspaceId: string }>('POST', '/api/invites/accept', { token }),
  setRole: (id: string, userId: string, role: TeamRole) => req('PATCH', `/api/workspaces/${id}/members/${userId}`, { role }),
  remove: (id: string, userId: string) => req('DELETE', `/api/workspaces/${id}/members/${userId}`),
};

export type ReportReason = 'csam' | 'ncii' | 'violence' | 'hate' | 'harassment' | 'self_harm' | 'spam' | 'phishing' | 'copyright' | 'trademark' | 'impersonation' | 'privacy' | 'other';
export interface ReportInput {
  targetType: 'design' | 'asset' | 'comment' | 'user' | 'public_link'; targetId: string; reason: ReportReason; details?: string; email?: string;
  legal?: { name?: string; address?: string; work?: string; goodFaith?: boolean; accurate?: boolean; signature?: string; location?: string };
}
export const reportApi = {
  file: (b: ReportInput) => req<{ ok: boolean; id?: string; message?: string }>('POST', '/api/reports', b),
};

export interface AdminReport {
  id: string; target_type: string; target_id: string; reason: string; details: string | null; reporter_email: string | null; reporter_name: string | null;
  legal: Record<string, unknown> | null; status: string; priority: number; decision: string | null; created_at: string;
}
export const adminApi = {
  overview: () => req<{ users: number; designs: number; openReports: number; flaggedAssets: number; failedJobs24h: number }>('GET', '/api/admin/overview'),
  reports: (status = 'open') => req<{ reports: AdminReport[] }>('GET', `/api/admin/reports?status=${status}`),
  decide: (id: string, b: { action: string; note: string; blockHash?: boolean }) => req('POST', `/api/admin/reports/${id}/decide`, b),
  users: (q: string) => req<{ users: Array<{ id: string; email: string; name: string; is_staff: boolean; disabled_at: string | null; disabled_reason: string | null; email_verified_at: string | null; created_at: string; designs: number }> }>('GET', `/api/admin/users?q=${encodeURIComponent(q)}`),
  userAction: (id: string, action: 'disable' | 'enable', reason?: string) => req('POST', `/api/admin/users/${id}/${action}`, { reason }),
  audit: () => req<{ brokenAt: number | null; events: Array<{ id: number; action: string; target: string | null; meta: unknown; ip: string | null; created_at: string; actor: string | null }> }>('GET', '/api/admin/audit'),
  jobs: () => req<{ jobs: Array<{ id: string; queue: string; status: string; attempts: number; label: string | null; error: string | null; created_at: string; finished_at: string | null }> }>('GET', '/api/admin/jobs'),
  addHash: (b: { hash: string; source: string; category: string }) => req('POST', '/api/admin/hashes', b),
};

export const devApi = {
  mail: (to?: string) => req<{ messages: Array<{ id: string; to: string; subject: string; text: string; html: string; status: string; createdAt: string }> }>('GET', `/api/dev/mail${to ? `?to=${encodeURIComponent(to)}` : ''}`),
};

// ---------------------------------------------------------------- growth, status

export interface SupportTicket { id: string; topic: string; subject: string; status: string; created_at: string; updated_at: string; email?: string; messages: Array<{ fromStaff: boolean; body: string; at: string }> | null }
export const supportApi = {
  create: (b: { topic: string; subject: string; body: string; email?: string }) => req<{ id: string; ref: string }>('POST', '/api/support', b),
  mine: () => req<{ tickets: SupportTicket[] }>('GET', '/api/support'),
  reply: (id: string, body: string) => req('POST', `/api/support/${id}/reply`, { body }),
  adminList: (status: string) => req<{ tickets: SupportTicket[] }>('GET', `/api/admin/support?status=${status}`),
  adminReply: (id: string, body: string, close?: boolean) => req('POST', `/api/admin/support/${id}/reply`, { body, close }),
};

export interface StatusInfo {
  overall: 'operational' | 'degraded' | 'outage';
  components: Array<{ id: string; name: string; state: 'operational' | 'degraded' | 'outage' | 'maintenance' }>;
  incidents: Array<{ id: string; title: string; impact: string; components: string[]; status: string; created_at: string; resolved_at: string | null; updates: Array<{ status: string; body: string; at: string }> | null }>;
  checkedAt: string;
}
export const statusApi = {
  get: () => req<StatusInfo>('GET', '/api/status'),
  open: (b: { title: string; impact: string; components: string[]; status: string; body: string }) => req<{ id: string }>('POST', '/api/admin/incidents', b),
  update: (id: string, b: { status: string; body: string }) => req('POST', `/api/admin/incidents/${id}/updates`, b),
};

export const analyticsAdminApi = {
  get: (days = 30) => req<{
    daily: Array<{ day: string; signups: number; active: number; designs: number; exports: number }>;
    events: Array<{ name: string; n: number }>;
    funnel: { signed_up: number; created_design: number; exported: number };
  }>('GET', `/api/admin/analytics?days=${days}`),
};

// ---------------------------------------------------------------- content library

export type StockKind = 'image' | 'video' | 'audio' | 'gif' | 'sticker';
export interface StockItem {
  provider: string; id: string; kind: StockKind; title: string; thumb: string; w: number; h: number; durationMs?: number;
  author?: string; authorUrl?: string; license: string; licenseUrl?: string; sourceUrl?: string;
}
export interface EmojiEntry { h: string; l: string; g: string; t: string[] }
/** Graphics & icon library (A4). */
export interface GraphicEntry { id: string; set: string; style?: string; name: string; license: string; viewBox: string; kind: 'icon' | 'graphic' }
export interface GraphicSet { id: string; label: string; kind: 'icon' | 'graphic'; license: string; licenseUrl: string; source: string; count: number }
export interface CustomFont { id: string; family: string; weight: number; italic: boolean; workspaceId: string; workspace: string }
export interface Credit { asset: string; name: string; provider: string; title?: string; author?: string; authorUrl?: string; license?: string; licenseUrl?: string; sourceUrl?: string }

/** Export watermark (g64): text on every exported page; position, opacity 0..1, rotation (deg), size (share of the short side). */
export interface ExportWatermark { text: string; position: 'center' | 'tiled' | 'top-left' | 'top-right' | 'bottom-left' | 'bottom-right'; opacity: number; rotation: number; size: number; color: string }

export const contentApi = {
  emoji: () => req<EmojiEntry[]>('GET', '/api/emoji/index.json'),
  emojiAsset: (hex: string) => req<{ asset: AssetRef }>('POST', `/api/emoji/${hex}/asset`, {}),
  graphicSets: () => req<{ sets: GraphicSet[] }>('GET', '/api/graphics/sets'),
  graphics: (q: string, set: string, page = 1, style?: string) =>
    req<{ items: GraphicEntry[]; total: number; hasMore: boolean }>('GET', `/api/graphics/search?${new URLSearchParams({ q, set, page: String(page), ...(style ? { style } : {}) })}`),
  graphicAsset: (id: string) => req<{ asset: AssetRef }>('POST', `/api/graphics/${id}/asset`, {}),
  providers: () => req<{ providers: Array<{ id: string; label: string; kinds: StockKind[] }> }>('GET', '/api/stock/providers'),
  search: (provider: string, kind: StockKind, q: string, page = 1) =>
    req<{ items: StockItem[]; hasMore: boolean }>('GET', `/api/stock/search?${new URLSearchParams({ provider, kind, q, page: String(page) })}`),
  import: (provider: string, kind: StockKind, id: string) => req<{ asset: AssetRef }>('POST', '/api/stock/import', { provider, kind, id }),
  importUrl: (url: string) => req<{ asset: AssetRef }>('POST', '/api/assets/import-url', { url }),
  credits: (designId: string) => req<{ credits: Credit[] }>('GET', `/api/designs/${designId}/credits`),
  customFonts: () => req<{ fonts: CustomFont[] }>('GET', '/api/fonts/custom'),
  uploadFont: (file: File, workspaceId?: string) => {
    const fd = new FormData();
    fd.append('licenseAck', 'true');
    if (workspaceId) fd.append('workspaceId', workspaceId);
    fd.append('file', file);
    return req<{ font: { id: string; family: string; weight: number; italic: boolean; file: string; sha256: string; bytes: number } }>('POST', '/api/fonts/custom', fd);
  },
  deleteFont: (id: string) => req('DELETE', `/api/fonts/custom/${id}`),
  designFonts: (designId: string) => req<{ families: Array<{ family: string; slug: string; category: string; license: string; faces: Array<{ weight: number; italic: boolean; file: string; sha256: string; bytes: number }> }> }>('GET', `/api/designs/${designId}/fonts`),
  publishTemplate: (b: { designId: string; title: string; category: string; tags: string[]; description?: string; scope: 'public' | 'team'; brand?: boolean }) =>
    req<{ id: string }>('POST', '/api/templates', b),
  updateTemplate: (id: string, b: { title?: string; category?: string; tags?: string[]; published?: boolean }) => req('PATCH', `/api/templates/${id}`, b),
  deleteTemplate: (id: string) => req('DELETE', `/api/templates/${id}`),
  adminTemplates: () => req<{ templates: Array<{ id: string; title: string; category: string; tags: string[]; published: boolean; uses: number; updated_at: string }> }>('GET', '/api/admin/templates'),
};

export const importApi = {
  /** `folderId`: import straight into that folder (Projects, g17). */
  start: (file: File, folderId?: string | null) => {
    const fd = new FormData();
    fd.append('file', file);
    return req<{ jobId: string }>('POST', `/api/imports${folderId ? `?folderId=${encodeURIComponent(folderId)}` : ''}`, fd);
  },
  status: (job: string) => req<{ status: 'queued' | 'running' | 'done' | 'error'; progress: number; label: string; error?: string; designId?: string; pages?: number }>('GET', `/api/imports/${job}`),
};

export const formsApi = {
  responses: (designId: string, elementId: string) =>
    req<{ fields: Array<{ id: string; label: string }>; responses: Array<{ id: string; data: Record<string, unknown>; created_at: string }> }>('GET', `/api/designs/${designId}/forms/${elementId}/responses`),
  deleteResponse: (designId: string, elementId: string, id: string) => req('DELETE', `/api/designs/${designId}/forms/${elementId}/responses/${id}`),
};

export interface SiteInfo {
  site: { slug: string; title: string; description: string | null; published: boolean; hasPassword: boolean; noindex: boolean; version: number; publishedAt: string | null } | null;
  url: string | null;
  domains: Array<{ domain: string; verified_at: string | null; token: string }>;
}
export const siteApi = {
  get: (id: string) => req<SiteInfo>('GET', `/api/designs/${id}/site`),
  save: (id: string, b: { slug: string; title: string; description?: string; noindex?: boolean; password?: string | null }) => req('PUT', `/api/designs/${id}/site`, b),
  preview: (id: string) => req<{ html: string }>('POST', `/api/designs/${id}/site/preview`, {}),
  publish: (id: string) => req<{ url: string }>('POST', `/api/designs/${id}/site/publish`, {}),
  unpublish: (id: string) => req('POST', `/api/designs/${id}/site/unpublish`, {}),
  addDomain: (id: string, domain: string) => req<{ domain: string; txt: { name: string; value: string }; cname: { name: string; value: string } }>('POST', `/api/designs/${id}/site/domains`, { domain }),
  verifyDomain: (id: string, domain: string) => req('POST', `/api/designs/${id}/site/domains/${domain}/verify`, {}),
  removeDomain: (id: string, domain: string) => req('DELETE', `/api/designs/${id}/site/domains/${domain}`),
};

export interface DevToken { id: string; name: string; prefix: string; scopes: string[]; last_used_at: string | null; expires_at: string | null; created_at: string }
export interface DevWebhook { id: string; url: string; events: string[]; active: boolean; failures: number; created_at: string; deliveries: { id: string; event: string; status: string; response_code: number | null; error: string | null; attempts: number; created_at: string }[] | null }
export const developerApi = {
  get: () => req<{ tokens: DevToken[]; webhooks: DevWebhook[]; scopes: string[]; events: string[] }>('GET', '/api/developer'),
  createToken: (b: { name: string; scopes: string[]; expiresInDays?: number }) => req<{ id: string; token: string }>('POST', '/api/developer/tokens', b),
  revokeToken: (id: string) => req('DELETE', `/api/developer/tokens/${id}`),
  createWebhook: (b: { url: string; events: string[] }) => req<{ id: string; secret: string }>('POST', '/api/developer/webhooks', b),
  pingWebhook: (id: string) => req('POST', `/api/developer/webhooks/${id}/ping`, {}),
  deleteWebhook: (id: string) => req('DELETE', `/api/developer/webhooks/${id}`),
};

// AI connector: the OAuth consent page and the apps connected to the account.
export interface OAuthRequestInfo {
  /** `verified`: a client ID metadata document client, identified by `domain`; otherwise the name is self-asserted. */
  client: { id: string; name: string; uri: string | null; logo: string | null; redirectHost: string; verified: boolean; domain: string | null };
  scopes: string[]; user: { email: string; name: string };
}
export interface OAuthConnection { clientId: string; name: string; uri: string | null; logo: string | null; scopes: string[]; connectedAt: string; lastUsedAt: string | null; domain: string | null }
export const oauthApi = {
  info: (params: Record<string, string>) => req<OAuthRequestInfo>('GET', `/api/oauth/authorize/info?${new URLSearchParams(params)}`),
  decide: (params: Record<string, string>, approve: boolean) => req<{ redirect: string }>('POST', '/api/oauth/authorize', { ...params, approve }),
  connections: () => req<{ connections: OAuthConnection[] }>('GET', '/api/oauth/connections'),
  disconnect: (clientId: string) => req('DELETE', `/api/oauth/connections/${encodeURIComponent(clientId)}`),
};

export const embedApi = {
  resolve: (url: string) => req<{ provider: EmbedProvider; src: string; kind: string; aspect: number; title: string; url: string; poster: AssetRef | null }>('POST', '/api/embeds', { url }),
};

// ---------------------------------------------------------------- Brand Hub (A8)
export interface KitColor { name: string; hex: string }
export type KitFontRole = 'heading' | 'subheading' | 'body' | 'quote' | 'caption';
/**
 * A kit text style: how one kind of text looks beyond its font, which comes from the kit's `fonts[role]`. Letter
 * spacing is in thousandths of an em and line height a multiple of the size, as in the editor; `size` (px) is a
 * reference for a typical page; `colors` are the colours the text may take (any, when empty).
 */
export interface KitTextStyle {
  name: string; role: KitFontRole; weight?: number; italic?: boolean; uppercase?: boolean; letterSpacing?: number;
  lineHeight?: number; size?: number; colors?: string[]; note?: string;
}
export interface Kit {
  id: string; workspaceId: string; workspace?: string; personal?: boolean; isDefault: boolean; canEdit?: boolean;
  name: string; colors: KitColor[]; palettes: Array<{ name: string; colors: KitColor[] }>; fonts: Partial<Record<KitFontRole, string>>;
  logos: string[]; photos: string[]; graphics: string[]; voice: string; textStyles?: KitTextStyle[];
}
/** A section of a brand kit's written guidelines (its brand book), in Markdown. */
export interface GuidelineSection { title: string; body: string }
export type KitInput = Omit<Kit, 'id' | 'workspaceId' | 'workspace' | 'personal' | 'isDefault' | 'canEdit'>;
export interface BrandControls { colors?: boolean; fonts?: boolean; requireTemplate?: boolean; restrictUploads?: boolean }
export interface BrandWorkspace { id: string; name: string; personal: boolean; role: string; canEdit: boolean; controls: BrandControls }
export interface DesignBrand {
  workspaceId: string | null; role: string | null; controls: BrandControls;
  template: { id: string; title: string | null; brand: boolean; version: number; latest: number } | null;
  published: Array<{ id: string; title: string; version: number; brand: boolean }>;
}

export const brandHub = {
  kits: () => req<{ kits: Kit[]; workspaces: BrandWorkspace[] }>('GET', '/api/brand-kits'),
  createKit: (b: Partial<KitInput> & { name: string; workspaceId?: string }) => req<{ id: string }>('POST', '/api/brand-kits', b),
  updateKit: (id: string, b: Partial<KitInput>) => req('PATCH', `/api/brand-kits/${id}`, b),
  deleteKit: (id: string) => req('DELETE', `/api/brand-kits/${id}`),
  makeDefault: (id: string) => req('POST', `/api/brand-kits/${id}/default`),
  guidelines: (id: string) => req<{ sections: GuidelineSection[]; canEdit: boolean }>('GET', `/api/brand-kits/${id}/guidelines`),
  setGuidelines: (id: string, sections: GuidelineSection[]) => req<{ ok: true }>('PUT', `/api/brand-kits/${id}/guidelines`, { sections }),
  controls: (ws: string) => req<{ controls: BrandControls; canEdit: boolean; role: string }>('GET', `/api/workspaces/${ws}/brand-controls`),
  setControls: (ws: string, c: BrandControls) => req<{ controls: BrandControls }>('PUT', `/api/workspaces/${ws}/brand-controls`, c),
  design: (id: string) => req<DesignBrand>('GET', `/api/designs/${id}/brand`),
  republish: (templateId: string) => req<{ version: number }>('POST', `/api/templates/${templateId}/republish`),
};
