import { describe, expect, it, vi } from 'vitest';
import { ApiError, type User } from '../apps/web/src/api';
import { loadSession, retryDelay } from '../apps/web/src/auth/session';

const user = { id: 'u1', email: 'a@b.test', name: 'A', color: '#000' } as User;
const noWait = () => Promise.resolve();

describe('session bootstrap (A-01: a blip is not a sign-out)', () => {
  it('returns the user on success', async () => {
    expect(await loadSession(async () => ({ user }), { sleep: noWait })).toBe(user);
  });

  it('treats 401 and an empty session as signed out, without retrying', async () => {
    const me = vi.fn(async () => { throw new ApiError(401, 'unauthorized'); });
    expect(await loadSession(me, { sleep: noWait })).toBeNull();
    expect(me).toHaveBeenCalledTimes(1);
    expect(await loadSession(async () => ({ user: null }), { sleep: noWait })).toBeNull();
  });

  it('retries network errors and 5xx (API restarting, dev proxy 502) until the session comes back', async () => {
    const errors = [new TypeError('Failed to fetch'), new ApiError(502, 'error'), new ApiError(503, 'error')];
    const me = vi.fn(async () => {
      const e = errors.shift();
      if (e) throw e;
      return { user };
    });
    const retries: number[] = [];
    const waits: number[] = [];
    const u = await loadSession(me, { sleep: async (ms) => { waits.push(ms); }, onRetry: (n) => retries.push(n) });
    expect(u).toBe(user);
    expect(me).toHaveBeenCalledTimes(4);
    expect(retries).toEqual([1, 2, 3]);
    expect(waits).toEqual([500, 1000, 2000]);
  });

  it('a 401 after a network error still means signed out', async () => {
    const errors = [new TypeError('Failed to fetch'), new ApiError(401, 'unauthorized')];
    expect(await loadSession(async () => { throw errors.shift(); }, { sleep: noWait })).toBeNull();
  });

  it('backs off up to 8 s', () => {
    expect([0, 1, 2, 3, 4, 10].map(retryDelay)).toEqual([500, 1000, 2000, 4000, 8000, 8000]);
  });

  it('stops when aborted (unmount)', async () => {
    const ac = new AbortController();
    const me = vi.fn(async () => { throw new TypeError('Failed to fetch'); });
    const p = loadSession(me, { signal: ac.signal, sleep: async () => { ac.abort(); } });
    await expect(p).rejects.toThrow();
    expect(me).toHaveBeenCalledTimes(1);
  });
});
