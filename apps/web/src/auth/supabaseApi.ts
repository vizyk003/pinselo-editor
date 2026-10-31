/**
 * Pinselo endpoints for Supabase Auth (plan 12 §B3, §B9). The bridge trades a Supabase access token for the
 * normal `oc_session` cookie; the account endpoints take the user's current token as fresh proof from Supabase.
 */
import { ApiError, req, type User } from '../api';

export type BridgeResult = (
  | { user: User }
  | { mfaRequired: true; ticket: string }
  | { mfaRequired: true; supabase: true }
  | { needsProfile: true; email: string; name: string | null }
) & { next?: string };

export interface BridgeBody {
  accessToken: string;
  next?: string;
  profile?: { name: string; birthYear: number; acceptTerms: true };
  /** After a password reset: end the account's other Pinselo sessions. */
  revokeOtherSessions?: boolean;
}

/**
 * An API error from the bridge or sync, with its `reason` (409 link_conflict: linked_elsewhere | unconfirmed_supabase |
 * email_unverifiable | changed_during_signin; unverified_local from older servers).
 */
export class BridgeError extends ApiError {
  constructor(status: number, code: string, message: string | undefined, reason?: string) {
    super(status, code, message, reason);
  }
}

/** A JSON POST with its own fetch (not `req`): its errors are BridgeErrors carrying the link_conflict `reason`. */
async function post<T>(url: string, body: unknown): Promise<T> {
  const res = await fetch(url, { method: 'POST', credentials: 'same-origin', headers: { 'content-type': 'application/json' }, body: JSON.stringify(body) });
  const j = (await res.json().catch(() => ({}))) as T & { error?: string; message?: string; reason?: string };
  if (!res.ok) throw new BridgeError(res.status, j.error ?? 'error', j.message, j.reason);
  return j;
}

/** POST /api/auth/supabase/session. */
export const bridge = (body: BridgeBody) => post<BridgeResult>('/api/auth/supabase/session', body);

/**
 * Re-auth and email-change callbacks: link or re-check the signed-in account against the Supabase user, and sync its
 * email. Linking can be refused with a link_conflict reason (e.g. `email_unverifiable`).
 */
export const syncSupabase = (accessToken: string) => post<{ user: User; emailConflict?: true }>('/api/account/supabase/sync', { accessToken });

/** "Log out everywhere else": the other Pinselo sessions, and with a token the other Supabase sessions too. */
export const revokeOthersWithToken = (accessToken?: string) =>
  req<{ ok: true; supabase?: 'revoked' | 'skipped' | 'failed' }>('POST', '/api/account/sessions/revoke-others', { accessToken });

/** DELETE /api/account for a linked account: a recent Supabase sign-in (≤ 10 min, aal2 with MFA) replaces the password. */
export const deleteAccountSupabase = (b: { confirm: 'DELETE'; code?: string; accessToken?: string }) => req('DELETE', '/api/account', b);

/**
 * `GET /api/account` → `auth` (plan 12 §B9); absent on servers without Supabase support. `supabaseId`: the linked
 * Supabase user, so Settings can tell this account's Supabase session from someone else's when the emails differ
 * (review C1); null when not linked, absent on servers that don't send it yet (Settings then compares emails).
 * `supabaseEmail`: that user's address at Supabase (review F2), pre-filled in the re-auth form when this browser has
 * no Supabase session; null when the server couldn't read it.
 */
export interface AccountAuth { mode: 'builtin' | 'supabase'; supabaseLinked: boolean; supabaseId?: string | null; supabaseEmail?: string | null }
