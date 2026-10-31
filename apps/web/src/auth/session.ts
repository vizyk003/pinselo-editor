/**
 * Who is signed in, robust to a flaky network (only a 401 means "signed out"), and signing out, which in Supabase
 * mode also ends this browser's Supabase session (plan 12 §C1).
 */
import { api, ApiError, type User } from '../api';
import { authConfigNow, loadAuthConfig } from './authConfig';
import { settledWithin } from './sessionRules';
import { forgetStoredSession, getSupabase, hasStoredSession, storedSignIn, supabaseLoaded } from './supabaseClient';

/**
 * Loading the session: a network error or a 5xx (the API restarting, the dev proxy's 502) is retried with backoff,
 * so a blip never drops someone to the signed-out page mid-edit.
 */
export const isSignedOutError = (e: unknown) => e instanceof ApiError && e.status === 401;

/** Backoff before retry `n` (0-based): 0.5 s, 1 s, 2 s, 4 s, then every 8 s. */
export const retryDelay = (n: number) => Math.min(8000, 500 * 2 ** n);

export interface LoadSessionOptions {
  /** Called before each retry (the attempt number, 1-based, and the error) — e.g. to show "Reconnecting…". */
  onRetry?: (attempt: number, error: unknown) => void;
  /** Waits before the next attempt (default setTimeout); the app cuts it short on "Retry now" or back online. */
  sleep?: (ms: number) => Promise<void>;
  signal?: AbortSignal;
}

/** Resolves with the signed-in user, or null once the server says 401 / no session. Retries everything else. */
export async function loadSession(me: () => Promise<{ user: User | null }>, opts: LoadSessionOptions = {}): Promise<User | null> {
  const sleep = opts.sleep ?? ((ms: number) => new Promise<void>((r) => setTimeout(r, ms)));
  for (let attempt = 0; ; attempt++) {
    if (opts.signal?.aborted) throw new DOMException('Aborted', 'AbortError');
    try {
      return (await me()).user ?? null;
    } catch (e) {
      if (isSignedOutError(e)) return null;
      if (opts.signal?.aborted) throw e;
      opts.onRetry?.(attempt + 1, e);
      await sleep(retryDelay(attempt));
    }
  }
}

/** How long dropping the Supabase session may take before it is forgotten in this browser anyway (review C3). */
export const SUPABASE_SIGN_OUT_WAIT_MS = 2000;

/**
 * The Pinselo session is what signs the user out, so only it is awaited. The Supabase session is dropped in the
 * background: a slow or unreachable Supabase never holds up the UI (review C3).
 */
export async function signOut(): Promise<void> {
  try {
    await api.logout();
  } finally {
    void signOutSupabaseLocal();
  }
}

/**
 * Drops the Supabase session kept in this browser: supabase-js's local sign-out (which also ends that one session at
 * Supabase), for at most SUPABASE_SIGN_OUT_WAIT_MS. When that fails or takes longer, the stored session is forgotten
 * here anyway. Never loads supabase-js in built-in mode or when there is nothing to drop. Never throws.
 */
export async function signOutSupabaseLocal(): Promise<void> {
  const url = authConfigNow()?.supabase?.url;
  const before = url ? storedSignIn(url) : null;
  const work = (async () => {
    const c = authConfigNow() ?? (await loadAuthConfig());
    if (!c.supabase || (!supabaseLoaded() && !hasStoredSession(c.supabase.url))) return;
    // supabase-js keeps the session when the logout request fails (other than 401/403/404).
    const { error } = await (await getSupabase()).auth.signOut({ scope: 'local' });
    if (error) throw error;
  })();
  if (await settledWithin(work, SUPABASE_SIGN_OUT_WAIT_MS)) return;
  if (url) forgetStoredSession(url, before);
}
