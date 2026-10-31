/**
 * supabase-js, loaded only in Supabase mode (plan 12 §C): `import()` keeps it in its own chunk, so built-in mode never
 * downloads it. One client per page with PKCE and `detectSessionInUrl: false` — /auth/callback exchanges the
 * single-use code itself. The session is kept in localStorage for the Settings actions that need it (§B7).
 */
import type { SupabaseClient } from '@supabase/supabase-js';
import { loadAuthConfig } from './authConfig';

let client: Promise<SupabaseClient> | null = null;

/** supabase-js's default storage key, `sb-<project ref>-auth-token`, set explicitly so `hasStoredSession` can find it. */
export const storageKeyFor = (url: string) => `sb-${new URL(url).hostname.split('.')[0]}-auth-token`;

export function getSupabase(): Promise<SupabaseClient> {
  client ??= (async () => {
    const c = await loadAuthConfig();
    if (!c.supabase) throw new Error('Supabase Auth is not configured on this server.');
    const { createClient } = await import('@supabase/supabase-js');
    return createClient(c.supabase.url, c.supabase.publishableKey, {
      auth: { flowType: 'pkce', persistSession: true, autoRefreshToken: true, detectSessionInUrl: false, storageKey: storageKeyFor(c.supabase.url) },
    });
  })().catch((e: unknown) => {
    client = null;
    throw e;
  });
  return client;
}

/** True once `getSupabase()` has been called on this page. */
export const supabaseLoaded = () => client !== null;

/** The Supabase session this browser keeps for `url`, as stored (without loading supabase-js); null when none. */
export function storedSession(url: string): string | null {
  try {
    return localStorage.getItem(storageKeyFor(url));
  } catch {
    return null;
  }
}

/** Whether this browser holds a Supabase session for `url` (without loading supabase-js). */
export const hasStoredSession = (url: string): boolean => storedSession(url) !== null;

/**
 * Which sign-in the session stored for `url` belongs to: its access token's `session_id`, which token refreshes keep
 * (they rewrite the stored value); the raw value when it has none. Null when nothing is stored.
 */
export function storedSignIn(url: string): string | null {
  const raw = storedSession(url);
  if (raw === null) return null;
  try {
    const token = String((JSON.parse(raw) as { access_token?: unknown }).access_token);
    const claims = JSON.parse(atob(token.split('.')[1]!.replace(/-/g, '+').replace(/_/g, '/'))) as { session_id?: unknown };
    return typeof claims.session_id === 'string' && claims.session_id ? claims.session_id : raw;
  } catch {
    return raw;
  }
}

/**
 * Forgets the Supabase session kept for `url` in this browser, but only while it is still the sign-in `expected`
 * (`storedSignIn` when signing out began): a token refresh since is the same sign-in, a new sign-in survives. For when
 * Supabase's sign-out fails or is slow.
 */
export function forgetStoredSession(url: string, expected: string | null): void {
  if (expected === null || storedSignIn(url) !== expected) return;
  try {
    const key = storageKeyFor(url);
    for (const k of [key, `${key}-code-verifier`, `${key}-user`]) localStorage.removeItem(k);
  } catch {
    /* storage blocked: nothing kept */
  }
}
