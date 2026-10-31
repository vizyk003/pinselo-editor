/**
 * Signing out never waits on a slow Supabase (review C3), and the sign-in config never falls back to the built-in
 * forms when it can't load (review C4). Modules are re-imported per test: they keep page-wide state.
 */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const SB_URL = 'https://abcdefgh.supabase.co';
const KEY = 'sb-abcdefgh-auth-token';
const config = { providers: [], passkeys: false, devLogin: false, termsVersion: '1', supabase: { url: SB_URL, publishableKey: 'sb_publishable_x', providers: [], email: true, signup: true, autoconfirm: false } };
const json = (body: unknown, status = 200) => new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });

function memoryStorage() {
  const m = new Map<string, string>();
  return {
    getItem: (k: string) => m.get(k) ?? null,
    setItem: (k: string, v: string) => void m.set(k, String(v)),
    removeItem: (k: string) => void m.delete(k),
    clear: () => m.clear(),
    key: (i: number) => [...m.keys()][i] ?? null,
    get length() {
      return m.size;
    },
  };
}

// supabase-js stand-in: its local sign-out is whatever the test says.
let supabaseSignOut: () => Promise<{ error: unknown }> = async () => ({ error: null });
vi.mock('../apps/web/src/auth/supabaseClient', async (importOriginal) => {
  const real = await importOriginal<typeof import('../apps/web/src/auth/supabaseClient')>();
  const client = { auth: { signOut: () => supabaseSignOut() } };
  let loaded = false;
  return {
    ...real,
    getSupabase: async () => {
      loaded = true;
      return client;
    },
    supabaseLoaded: () => loaded,
  };
});

let providers: () => Promise<Response>;
beforeEach(() => {
  vi.resetModules();
  vi.stubGlobal('localStorage', memoryStorage());
  providers = async () => json(config);
  vi.stubGlobal('fetch', vi.fn(async (url: string) => {
    if (url === '/api/auth/providers') return providers();
    if (url === '/api/auth/logout') return json({ ok: true });
    throw new TypeError(`unexpected fetch ${url}`);
  }));
});
afterEach(() => {
  vi.useRealTimers();
  vi.unstubAllGlobals();
  supabaseSignOut = async () => ({ error: null });
});

describe('sign-out (review C3)', () => {
  it('signs out of Pinselo without waiting for a hung Supabase, then forgets the stored session', async () => {
    vi.useFakeTimers();
    const { loadAuthConfig } = await import('../apps/web/src/auth/authConfig');
    const { signOut, SUPABASE_SIGN_OUT_WAIT_MS } = await import('../apps/web/src/auth/session');
    await loadAuthConfig();
    localStorage.setItem(KEY, '{"access_token":"a"}');
    localStorage.setItem(`${KEY}-code-verifier`, 'v');
    localStorage.setItem('unrelated', 'keep');
    supabaseSignOut = () => new Promise(() => {});
    // Resolves as soon as the Pinselo session has ended: the UI updates right away.
    await signOut();
    expect(localStorage.getItem(KEY)).not.toBeNull();
    await vi.advanceTimersByTimeAsync(SUPABASE_SIGN_OUT_WAIT_MS);
    expect(SUPABASE_SIGN_OUT_WAIT_MS).toBeLessThanOrEqual(2000);
    expect(localStorage.getItem(KEY)).toBeNull();
    expect(localStorage.getItem(`${KEY}-code-verifier`)).toBeNull();
    expect(localStorage.getItem('unrelated')).toBe('keep');
  });

  it('forgets the session when Supabase’s sign-out fails, but keeps a sign-in that happened meanwhile', async () => {
    const { loadAuthConfig } = await import('../apps/web/src/auth/authConfig');
    const { signOutSupabaseLocal } = await import('../apps/web/src/auth/session');
    await loadAuthConfig();
    localStorage.setItem(KEY, '{"access_token":"a"}');
    supabaseSignOut = async () => ({ error: Object.assign(new Error('fetch failed'), { status: 0 }) });
    await signOutSupabaseLocal();
    expect(localStorage.getItem(KEY)).toBeNull();

    localStorage.setItem(KEY, '{"access_token":"old"}');
    supabaseSignOut = async () => {
      localStorage.setItem(KEY, '{"access_token":"new"}');
      return { error: new Error('offline') };
    };
    await signOutSupabaseLocal();
    expect(localStorage.getItem(KEY)).toBe('{"access_token":"new"}');
  });

  it('still forgets the session when supabase-js refreshed its token meanwhile (same sign-in)', async () => {
    vi.useFakeTimers();
    const jwt = (claims: object) => `h.${Buffer.from(JSON.stringify(claims)).toString('base64url')}.s`;
    const stored = (claims: object) => JSON.stringify({ access_token: jwt(claims), refresh_token: 'r' });
    const { loadAuthConfig } = await import('../apps/web/src/auth/authConfig');
    const { signOutSupabaseLocal } = await import('../apps/web/src/auth/session');
    await loadAuthConfig();
    // A short-lived token is refreshed when supabase-js loads (before it can sign out), and the logout then hangs.
    localStorage.setItem(KEY, stored({ sub: 'u1', session_id: 's1', exp: 1 }));
    supabaseSignOut = () => {
      localStorage.setItem(KEY, stored({ sub: 'u1', session_id: 's1', exp: 2 }));
      return new Promise(() => {});
    };
    void signOutSupabaseLocal();
    await vi.advanceTimersByTimeAsync(2000);
    expect(localStorage.getItem(KEY)).toBeNull();

    // A different sign-in (another session_id) that happened meanwhile is kept.
    localStorage.setItem(KEY, stored({ sub: 'u1', session_id: 's1', exp: 1 }));
    supabaseSignOut = () => {
      localStorage.setItem(KEY, stored({ sub: 'u2', session_id: 's2', exp: 1 }));
      return new Promise(() => {});
    };
    void signOutSupabaseLocal();
    await vi.advanceTimersByTimeAsync(2000);
    expect(JSON.parse(localStorage.getItem(KEY)!).access_token).toBe(jwt({ sub: 'u2', session_id: 's2', exp: 1 }));
  });

  it('never throws, even when the config can’t load', async () => {
    providers = async () => {
      throw new TypeError('Failed to fetch');
    };
    const { signOutSupabaseLocal } = await import('../apps/web/src/auth/session');
    await expect(signOutSupabaseLocal()).resolves.toBeUndefined();
  });
});

describe('sign-in config (review C4)', () => {
  it('reports a failed load as failed (never the built-in mode) and loads again on retry', async () => {
    let fail = true;
    providers = async () => (fail ? json({ error: 'internal' }, 503) : json(config));
    const m = await import('../apps/web/src/auth/authConfig');
    const seen: string[] = [];
    expect(m.authConfigState()).toEqual({ kind: 'loading' });
    await expect(m.loadAuthConfig()).rejects.toThrow();
    expect(m.authConfigState()).toEqual({ kind: 'failed' });
    expect(m.authConfigNow()).toBeNull();

    fail = false;
    m.retryAuthConfig();
    seen.push(m.authConfigState().kind);
    await m.loadAuthConfig();
    seen.push(m.authConfigState().kind);
    expect(seen).toEqual(['loading', 'ready']);
    expect(m.authModeOf(m.authConfigNow())).toBe('supabase');
  });

  it('shares one request between concurrent loads and never reloads once ready', async () => {
    const m = await import('../apps/web/src/auth/authConfig');
    await Promise.all([m.loadAuthConfig(), m.loadAuthConfig(), m.loadAuthConfig()]);
    m.retryAuthConfig();
    await m.loadAuthConfig();
    expect(vi.mocked(fetch).mock.calls.filter(([u]) => u === '/api/auth/providers')).toHaveLength(1);
  });

  it('keeps built-in mode for a server without Supabase', async () => {
    providers = async () => json({ ...config, supabase: null });
    const m = await import('../apps/web/src/auth/authConfig');
    await m.loadAuthConfig();
    expect(m.authConfigState()).toMatchObject({ kind: 'ready', config: { supabase: null } });
    expect(m.authModeOf(m.authConfigNow())).toBe('builtin');
  });
});
