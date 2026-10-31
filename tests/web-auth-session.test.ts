import { afterEach, describe, expect, it, vi } from 'vitest';
import { isOwnSession, reauthEmail, resumeSteps, sessionKey, settledWithin } from '../apps/web/src/auth/sessionRules';

const session = (id: string, email: string | null, token: string) => ({ user: { id, email }, access_token: token });

describe('web auth session rules (review C1, C2, C3, C6)', () => {
  it('tells this account’s Supabase session by the linked id, whatever its email (C1)', () => {
    const linked = { email: 'old@example.com', supabaseId: 'sb-1' };
    // The Supabase email changed to an address another account here owns: still this account's session.
    expect(isOwnSession({ id: 'sb-1', email: 'new@example.com' }, linked)).toBe(true);
    expect(isOwnSession({ id: 'sb-1', email: null }, linked)).toBe(true);
    // Someone else's session with this account's email is not.
    expect(isOwnSession({ id: 'sb-2', email: 'old@example.com' }, linked)).toBe(false);
    // Not linked yet (or a server without supabaseId): the same email, as `sync` links by.
    for (const a of [{ email: 'Ada@Example.com', supabaseId: null }, { email: 'Ada@Example.com' }]) {
      expect(isOwnSession({ id: 'x', email: 'ada@example.com' }, a)).toBe(true);
      expect(isOwnSession({ id: 'x', email: 'eve@example.com' }, a)).toBe(false);
      expect(isOwnSession({ id: 'x', email: null }, a)).toBe(false);
      expect(isOwnSession({ id: 'x' }, a)).toBe(false);
    }
  });

  it('pre-fills the re-auth email with the address this account signs in with at Supabase (C1)', () => {
    const a = { email: 'old@example.com', supabaseId: 'sb-1' };
    expect(reauthEmail({ user: { id: 'sb-1', email: 'new@example.com' } }, a)).toBe('new@example.com');
    // Another account's session never leaks its address into the form.
    expect(reauthEmail({ user: { id: 'sb-2', email: 'eve@example.com' } }, a)).toBe('old@example.com');
    expect(reauthEmail(null, a)).toBe('old@example.com');
    expect(reauthEmail(undefined, a)).toBe('old@example.com');
    expect(reauthEmail({ user: { id: 'sb-1', email: null } }, a)).toBe('old@example.com');
  });

  it('without a Supabase session, pre-fills the linked user’s Supabase address that the server sent (F2)', () => {
    const a = { email: 'old@example.com', supabaseId: 'sb-1', supabaseEmail: 'new@example.com' };
    expect(reauthEmail(null, a)).toBe('new@example.com');
    expect(reauthEmail(undefined, a)).toBe('new@example.com');
    // This account's own session still wins; another account's session never leaks its address.
    expect(reauthEmail({ user: { id: 'sb-1', email: 'newest@example.com' } }, a)).toBe('newest@example.com');
    expect(reauthEmail({ user: { id: 'sb-2', email: 'eve@example.com' } }, a)).toBe('new@example.com');
    // Unknown (or not linked): the address here.
    expect(reauthEmail(null, { ...a, supabaseEmail: null })).toBe('old@example.com');
    expect(reauthEmail(null, { email: 'old@example.com', supabaseId: null, supabaseEmail: 'x@example.com' })).toBe('old@example.com');
  });

  it('keys session effects on whose session it is, never the token (C2)', () => {
    expect(sessionKey(undefined)).toBe('loading');
    expect(sessionKey(null)).toBe('none');
    expect(sessionKey(session('u1', 'a@x.test', 't1'))).toBe(sessionKey(session('u1', 'b@x.test', 't2')));
    expect(sessionKey(session('u1', 'a@x.test', 't1'))).not.toBe(sessionKey(session('u2', 'a@x.test', 't1')));
    expect(new Set([sessionKey(undefined), sessionKey(null), sessionKey(session('u1', null, 't'))]).size).toBe(3);
  });

  it('runs the factor-list effect once although listing factors refreshes the token (C2)', () => {
    // A minimal effect loop: re-render after every session change; run the effect when its deps change.
    const simulate = (key: (s: ReturnType<typeof session>) => string) => {
      let s = session('u1', 'a@x.test', 't0');
      let deps: string | null = null;
      let runs = 0;
      for (let render = 0; render < 50; render++) {
        const k = key(s);
        if (k === deps) continue;
        deps = k;
        runs++;
        // listFactors() may refresh the access token: onAuthStateChange hands over a new session object.
        s = session('u1', 'a@x.test', `t${runs}`);
      }
      return runs;
    };
    expect(simulate(sessionKey)).toBe(1);
    // The old key (`${id}:${access_token}`) ran on every render: the refresh loop of review C2.
    expect(simulate((s) => `${s.user.id}:${s.access_token}`)).toBe(50);
  });

  describe('settledWithin (C3)', () => {
    afterEach(() => vi.useRealTimers());
    it('is true when the work finishes in time, false when it fails or takes longer', async () => {
      vi.useFakeTimers();
      expect(await settledWithin(Promise.resolve('ok'), 2000)).toBe(true);
      expect(await settledWithin(Promise.reject(new Error('offline')), 2000)).toBe(false);
      const hung = settledWithin(new Promise(() => {}), 2000);
      await vi.advanceTimersByTimeAsync(1999);
      let done: boolean | null = null;
      void hung.then((v) => (done = v));
      await Promise.resolve();
      expect(done).toBeNull();
      await vi.advanceTimersByTimeAsync(1);
      expect(await hung).toBe(false);
    });
  });

  it('resumes a password reset at the step that failed (C6)', async () => {
    const calls: string[] = [];
    let bridgeOk = false;
    const steps = [
      ['password', async () => void calls.push('password')],
      ['others', async () => void calls.push('others')],
      ['bridge', async () => {
        calls.push('bridge');
        if (!bridgeOk) throw new Error('bridge down');
      }],
    ] as const;
    const done = new Set<string>();
    await expect(resumeSteps(done, steps)).rejects.toThrow('bridge down');
    expect([...done]).toEqual(['password', 'others']);
    bridgeOk = true;
    // "Try again" bridges again without setting the password a second time (Supabase would say same_password).
    expect(await resumeSteps(done, steps)).toBe(true);
    expect(calls).toEqual(['password', 'others', 'bridge', 'bridge']);
    // Everything done: nothing runs again.
    expect(await resumeSteps(done, steps)).toBe(true);
    expect(calls).toHaveLength(4);
  });

  it('stops without marking a step that asks for something first (a second factor)', async () => {
    const done = new Set<string>();
    let aal2 = false;
    const steps = [
      ['password', async () => aal2],
      ['bridge', async () => {}],
    ] as const;
    expect(await resumeSteps(done, steps)).toBe(false);
    expect(done.size).toBe(0);
    aal2 = true;
    expect(await resumeSteps(done, steps)).toBe(true);
    expect([...done]).toEqual(['password', 'bridge']);
  });
});
