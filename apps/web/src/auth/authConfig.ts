/**
 * Which sign-in this server uses (plan 12 §B1). `GET /api/auth/providers` carries `supabase: null` for the built-in
 * auth, or the public Supabase settings (URL, publishable key, OAuth providers) when Supabase Auth is on. The mode is
 * decided from `supabase !== null` only. Loaded once at start-up (App) and cached. A failed load is never mistaken
 * for the built-in mode (review C4): the forms show "try again", and it retries on demand and when the browser is
 * back online.
 */
import { useEffect, useSyncExternalStore } from 'react';
import { authApi, type AuthProviders } from '../api';

export interface SupabasePublicConfig {
  url: string;
  publishableKey: string;
  /** OAuth providers enabled in the Supabase project. */
  providers: Array<{ id: string; label: string }>;
  /** Email + password and magic links enabled. */
  email: boolean;
  /** New accounts allowed (Supabase's `disable_signup` off). */
  signup: boolean;
  /** `signUp` returns a session at once (no confirmation email). */
  autoconfirm: boolean;
}
export type AuthConfig = AuthProviders & { supabase: SupabasePublicConfig | null };

/** `failed`: the last load failed and nothing loaded before; `retryAuthConfig()` (or coming back online) tries again. */
export type AuthConfigState = { kind: 'loading' } | { kind: 'ready'; config: AuthConfig } | { kind: 'failed' };

let pending: Promise<AuthConfig> | null = null;
let loaded: AuthConfig | null = null;
let state: AuthConfigState = { kind: 'loading' };
const listeners = new Set<() => void>();
const publish = (s: AuthConfigState) => {
  state = s;
  for (const l of listeners) l();
};

export function loadAuthConfig(): Promise<AuthConfig> {
  if (loaded) return Promise.resolve(loaded);
  if (!pending) {
    if (state.kind === 'failed') publish({ kind: 'loading' });
    pending = authApi.providers()
      .then((r) => {
        const c = r as AuthProviders & { supabase?: SupabasePublicConfig | null };
        loaded = { ...c, supabase: c.supabase ?? null };
        publish({ kind: 'ready', config: loaded });
        return loaded;
      })
      .catch((e: unknown) => {
        pending = null;
        publish({ kind: 'failed' });
        throw e;
      });
  }
  return pending;
}

/** Loads again after a failure (a Retry button, the `online` event). Never rejects. */
export function retryAuthConfig(): void {
  loadAuthConfig().catch(() => {});
}

/** The config if it has already loaded (no request). */
export const authConfigNow = (): AuthConfig | null => loaded;
export const authConfigState = (): AuthConfigState => state;

const subscribe = (l: () => void) => {
  listeners.add(l);
  return () => {
    listeners.delete(l);
  };
};

/** The auth config's state: render a spinner while `loading` (never the built-in form, to avoid a flash), "try again" when `failed`. */
export function useAuthConfigState(): AuthConfigState {
  const s = useSyncExternalStore(subscribe, authConfigState, authConfigState);
  useEffect(() => {
    // Starts the first load (a no-op while one is pending); after a failure, retries once the browser is back online.
    if (s.kind === 'loading') retryAuthConfig();
    if (s.kind !== 'failed') return;
    window.addEventListener('online', retryAuthConfig);
    return () => window.removeEventListener('online', retryAuthConfig);
  }, [s.kind]);
  return s;
}

export const authModeOf = (c: { supabase?: unknown } | null | undefined): 'builtin' | 'supabase' => (c?.supabase ? 'supabase' : 'builtin');
