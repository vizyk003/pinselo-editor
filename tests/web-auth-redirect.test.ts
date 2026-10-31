import { afterEach, describe, expect, it, vi } from 'vitest';
import { ApiError, authApi } from '../apps/web/src/api';
import { authModeOf } from '../apps/web/src/auth/authConfig';
import {
  authErrorMessage, callbackErrorMessage, callbackUrl, errorCodeOf, MIN_AGE, needsAccountConfirm, oauthOptions, orderProviders, parseCallback, profileProblem, safeNext, supabaseErrorMessage,
} from '../apps/web/src/auth/redirect';
import { BridgeError, bridge, syncSupabase } from '../apps/web/src/auth/supabaseApi';

describe('web auth redirects (plan 12 §C4)', () => {
  it('accepts same-site paths only', () => {
    for (const ok of ['/', '/ok?x=1', '/?template=abc', '/?create=instagram-post', '/invite?token=a1b2', '/design/123#p2', '/a%20b', '/a+b'])
      expect(safeNext(ok), ok).toBe(ok);
    for (const bad of [
      '//evil.com', '/\\evil.com', '/ok\\x', '\\\\evil.com', 'https://evil.com', 'javascript:alert(1)', 'evil.com', '',
      '/a\nb', '/a\rb', '/a\tb', '/a b', '/a\u0000b', '/a\u007fb', '/a b', `/${'x'.repeat(2000)}`,
    ]) expect(safeNext(bad), JSON.stringify(bad)).toBe('/');
    // Percent-encoded control characters are inert text in a path.
    expect(safeNext('/x%0a')).toBe('/x%0a');
    for (const v of [null, undefined, 42, ['/a'], { next: '/a' }]) expect(safeNext(v)).toBe('/');
    expect(safeNext(`/${'x'.repeat(1999)}`)).toHaveLength(2000);
  });

  it('builds callback URLs with an encoded next and flow', () => {
    expect(callbackUrl('/', 'signin', 'https://app.test')).toBe('https://app.test/auth/callback?next=%2F');
    expect(callbackUrl('/?template=t1&x=y', undefined, 'https://app.test')).toBe('https://app.test/auth/callback?next=%2F%3Ftemplate%3Dt1%26x%3Dy');
    expect(callbackUrl('/settings/account', 'email_change', 'http://localhost:3900')).toBe('http://localhost:3900/auth/callback?next=%2Fsettings%2Faccount&flow=email_change');
    expect(callbackUrl('//evil.com', 'recovery', 'https://app.test')).toBe('https://app.test/auth/callback?next=%2F&flow=recovery');
    // Round trip: what the callback reads back is the same next.
    const u = new URL(callbackUrl('/?create=poster&w=10', 'reauth', 'https://app.test'));
    expect(parseCallback(u.search)).toMatchObject({ next: '/?create=poster&w=10', flow: 'reauth' });
  });

  it('asks Google for the account chooser and lists Google first', () => {
    expect(oauthOptions('google', '/?template=t1', 'signin', 'https://app.test')).toEqual({
      redirectTo: 'https://app.test/auth/callback?next=%2F%3Ftemplate%3Dt1', queryParams: { prompt: 'select_account' },
    });
    expect(oauthOptions('google', '/settings', 'reauth', 'https://app.test').redirectTo).toBe('https://app.test/auth/callback?next=%2Fsettings&flow=reauth');
    expect(oauthOptions('github', '/', 'signin', 'https://app.test')).toEqual({ redirectTo: 'https://app.test/auth/callback?next=%2F' });
    const list = [{ id: 'github', label: 'GitHub' }, { id: 'apple', label: 'Apple' }, { id: 'google', label: 'Google' }];
    expect(orderProviders(list).map((p) => p.id)).toEqual(['google', 'github', 'apple']);
    expect(list[0]!.id).toBe('github');
  });

  it('parses the callback query and hash', () => {
    expect(parseCallback('?code=abc&next=%2F%3Ftemplate%3Dt1')).toEqual({ code: 'abc', tokenHash: null, type: null, next: '/?template=t1', flow: 'signin', error: null, message: null });
    expect(parseCallback('?token_hash=th&type=magiclink&next=/x')).toMatchObject({ tokenHash: 'th', type: 'magiclink', next: '/x', flow: 'signin' });
    // The link type wins over (or fills in) the flow.
    expect(parseCallback('?token_hash=th&type=recovery')).toMatchObject({ type: 'recovery', flow: 'recovery' });
    expect(parseCallback('?code=c&flow=signin&type=email_change')).toMatchObject({ flow: 'email_change' });
    expect(parseCallback('?code=c&flow=reauth')).toMatchObject({ flow: 'reauth' });
    expect(parseCallback('?code=c&flow=bogus&type=bogus')).toMatchObject({ flow: 'signin', type: null });
    expect(parseCallback('?code=c&next=//evil.com').next).toBe('/');
    expect(parseCallback('?code=c&next=%2F%5Cevil.com').next).toBe('/');
    // PKCE errors come in the query, implicit-flow errors in the hash.
    // Only the code is kept: error_description is never shown (review A7).
    expect(parseCallback('?error=access_denied&error_code=otp_expired&error_description=Email+link+is+invalid+or+has+expired&next=/a').error)
      .toEqual({ code: 'otp_expired' });
    expect(parseCallback('?next=/a', '#error=server_error&error_description=Boom')).toMatchObject({ next: '/a', error: { code: 'server_error' } });
    expect(parseCallback('', '#error_code=otp_expired').error).toEqual({ code: 'otp_expired' });
    expect(parseCallback('', '')).toEqual({ code: null, tokenHash: null, type: null, next: '/', flow: 'signin', error: null, message: null });
    // First half of a secure email change: a notice, no code.
    expect(parseCallback('?next=%2Fsettings%2Faccount&flow=email_change', '#message=Confirmation+link+accepted')).toMatchObject({ code: null, flow: 'email_change', message: 'Confirmation link accepted' });
  });

  it('decides the mode from supabase !== null only', () => {
    expect(authModeOf(null)).toBe('builtin');
    expect(authModeOf({ supabase: null })).toBe('builtin');
    expect(authModeOf({})).toBe('builtin');
    expect(authModeOf({ supabase: { url: 'https://x.supabase.co', publishableKey: 'sb_publishable_x', providers: [], email: true, signup: true, autoconfirm: false } })).toBe('supabase');
  });

  it('checks the needsProfile step like sign-up', () => {
    const y = 2026;
    expect(profileProblem({ name: 'Ada', birthYear: 1990, terms: true }, y)).toBeNull();
    expect(profileProblem({ name: '  ', birthYear: 1990, terms: true }, y)).toBe('name');
    expect(profileProblem({ name: 'x'.repeat(81), birthYear: 1990, terms: true }, y)).toBe('name');
    expect(profileProblem({ name: 'Ada', birthYear: 0, terms: true }, y)).toBe('birthYear');
    expect(profileProblem({ name: 'Ada', birthYear: y + 1, terms: true }, y)).toBe('birthYear');
    expect(profileProblem({ name: 'Ada', birthYear: y - MIN_AGE + 1, terms: true }, y)).toBe('age');
    expect(profileProblem({ name: 'Ada', birthYear: y - MIN_AGE, terms: true }, y)).toBeNull();
    expect(profileProblem({ name: 'Ada', birthYear: 1990, terms: false }, y)).toBe('terms');
  });

  it('maps known Supabase and bridge error codes', () => {
    for (const code of [
      'invalid_credentials', 'email_not_confirmed', 'over_request_rate_limit', 'over_email_send_rate_limit', 'weak_password', 'same_password',
      'otp_expired', 'flow_state_not_found', 'bad_code_verifier', 'mfa_verification_failed', 'signup_disabled', 'user_banned',
      'invalid_token', 'session_ended', 'token_used', 'disabled', 'age', 'email_required', 'wrong_account', 'supabase_unavailable',
      'linked_elsewhere', 'unconfirmed_supabase', 'unverified_local', 'email_unverifiable', 'changed_during_signin',
    ]) expect(supabaseErrorMessage(code), code).toMatch(/\S/);
    expect(supabaseErrorMessage('changed_during_signin')).not.toBe(supabaseErrorMessage('linked_elsewhere'));
    expect(supabaseErrorMessage('invalid_credentials')).toBe('Wrong email or password.');
    expect(supabaseErrorMessage('age')).toContain(String(MIN_AGE));
    expect(supabaseErrorMessage('signup_closed')).toContain('by invitation only');
    expect(supabaseErrorMessage('linked_elsewhere')).not.toBe(supabaseErrorMessage('unverified_local'));
    expect(supabaseErrorMessage('something_new')).toBeNull();
    expect(supabaseErrorMessage(undefined)).toBeNull();
    expect(supabaseErrorMessage(null)).toBeNull();
  });

  it('asks "Continue as …?" before a token_hash link signs this browser in (review A3)', () => {
    for (const type of ['magiclink', 'recovery', 'email_change', 'signup', 'invite', 'email']) {
      const cb = parseCallback(`?token_hash=th&type=${type}&next=/x`);
      expect(needsAccountConfirm(cb, false), `${type} signed out`).toBe(true);
    }
    // Signed in, a recovery link would sign this browser in to the link's account: confirm too.
    expect(needsAccountConfirm(parseCallback('?token_hash=th&type=recovery'), true)).toBe(true);
    // Signed in, other links only `sync` the current account, which the server refuses for another Supabase user.
    for (const type of ['magiclink', 'email_change', 'signup']) expect(needsAccountConfirm(parseCallback(`?token_hash=th&type=${type}`), true)).toBe(false);
    // A PKCE code is bound to the browser that asked for it (its code verifier): no confirmation.
    for (const flow of ['signin', 'recovery', 'reauth', 'email_change']) {
      expect(needsAccountConfirm(parseCallback(`?code=c&flow=${flow}`), false), flow).toBe(false);
      expect(needsAccountConfirm(parseCallback(`?code=c&flow=${flow}`), true), flow).toBe(false);
    }
  });

  it('never shows text from the callback URL (review A7)', () => {
    const evil = 'Your account is locked. Call +1 555 0100';
    const cb = parseCallback(`?error=access_denied&error_code=bogus_code&error_description=${encodeURIComponent(evil)}`);
    expect(JSON.stringify(cb)).not.toContain('locked');
    expect(callbackErrorMessage(cb.error!)).toBe('Something went wrong. Try again.');
    expect(callbackErrorMessage({ code: 'otp_expired' })).toBe('This link has expired or was already used.');
    expect(callbackErrorMessage(parseCallback('', `#error=access_denied&error_description=${encodeURIComponent(evil)}`).error!)).toBe('Sign-in with that provider failed. Try again.');
  });

  it('maps every sign-in error to our own text, never the raw message (review C5)', () => {
    const raw = 'Raw developer-facing English from Supabase';
    const authError = (o: Record<string, unknown>) => Object.assign(new Error(raw), { __isAuthError: true }, o);
    // Codes the review found unmapped.
    expect(authErrorMessage(authError({ name: 'AuthApiError', code: 'pkce_code_verifier_not_found', status: 400 }))).toBe('Open the link in the same browser you used to request it.');
    expect(authErrorMessage(authError({ name: 'AuthPKCECodeVerifierMissingError', status: 400 }))).toBe('Open the link in the same browser you used to request it.');
    for (const code of ['email_exists', 'user_already_exists', 'otp_disabled', 'validation_failed']) {
      expect(supabaseErrorMessage(code), code).toMatch(/\S/);
      expect(authErrorMessage(authError({ code, status: 422 })), code).not.toContain('Raw');
    }
    // An unreachable service: offline fetch, supabase-js retryable errors, 5xx, timeouts.
    const unavailable = 'Sign-in is unavailable right now. Try again in a minute.';
    expect(authErrorMessage(new TypeError('Failed to fetch'))).toBe(unavailable);
    expect(authErrorMessage(authError({ name: 'AuthRetryableFetchError', status: 0 }))).toBe(unavailable);
    expect(authErrorMessage(authError({ name: 'AuthUnknownError', status: 502 }))).toBe(unavailable);
    expect(authErrorMessage(new ApiError(503, 'error', raw))).toBe(unavailable);
    expect(authErrorMessage(Object.assign(new Error(raw), { name: 'TimeoutError' }))).toBe(unavailable);
    expect(authErrorMessage(authError({ code: 'over_request_rate_limit', status: 429 }))).toBe('Too many attempts. Wait a few minutes and try again.');
    expect(authErrorMessage(authError({ status: 429 }))).toBe('Too many attempts. Wait a few minutes and try again.');
    // Unknown codes and plain errors: a generic line.
    for (const e of [authError({ code: 'brand_new_code', status: 400 }), new ApiError(400, 'error', raw), new Error(raw), raw, null, undefined, 42]) {
      expect(authErrorMessage(e)).toBe('Something went wrong. Try again.');
    }
    // A link_conflict's reason wins over its code.
    expect(authErrorMessage(new BridgeError(409, 'link_conflict', raw, 'unverified_local'))).toBe(supabaseErrorMessage('unverified_local'));
    expect(errorCodeOf(new BridgeError(409, 'link_conflict', raw, 'linked_elsewhere'))).toBe('linked_elsewhere');
    expect(errorCodeOf(authError({ name: 'AuthSessionMissingError' }))).toBe('session_not_found');
    expect(errorCodeOf({})).toBe('');
  });

  describe('link_conflict reasons reach the UI (server-auth contract)', () => {
    afterEach(() => vi.unstubAllGlobals());
    const conflict = (reason: string) => vi.fn(async () => new Response(JSON.stringify({ error: 'link_conflict', reason, message: 'English server text' }), { status: 409 }));

    it('explains email_unverifiable from both the bridge and sync', async () => {
      const text = supabaseErrorMessage('email_unverifiable');
      expect(text).toMatch(/linked by email/);
      vi.stubGlobal('fetch', conflict('email_unverifiable'));
      for (const call of [() => syncSupabase('tok'), () => bridge({ accessToken: 'tok' })]) {
        const e = await call().then(() => null, (x: unknown) => x);
        expect(e).toBeInstanceOf(BridgeError);
        expect(errorCodeOf(e)).toBe('email_unverifiable');
        expect(authErrorMessage(e)).toBe(text);
      }
      // A refused sync is still recognisable by its code (Settings signs out a wrong_account session).
      vi.stubGlobal('fetch', vi.fn(async () => new Response(JSON.stringify({ error: 'wrong_account' }), { status: 403 })));
      const wrong = await syncSupabase('tok').then(() => null, (x: unknown) => x);
      expect((wrong as BridgeError).code).toBe('wrong_account');
    });

    it('explains changed_during_signin from the bridge and from the built-in code step', async () => {
      const text = supabaseErrorMessage('changed_during_signin');
      expect(text).toMatch(/changed while you were signing in/);
      vi.stubGlobal('fetch', conflict('changed_during_signin'));
      // The bridge (its own fetch) and POST /api/auth/mfa (the shared `req`, which now keeps `reason` too).
      for (const call of [() => bridge({ accessToken: 'tok' }), () => authApi.mfa('ticket', '123456')]) {
        const e = await call().then(() => null, (x: unknown) => x);
        expect(e).toBeInstanceOf(ApiError);
        expect(errorCodeOf(e)).toBe('changed_during_signin');
        expect(authErrorMessage(e)).toBe(text);
      }
    });

    it('keeps the retired unverified_local text for older servers, and maps the code step’s expired ticket', () => {
      expect(supabaseErrorMessage('unverified_local')).toMatch(/\S/);
      expect(authErrorMessage(new ApiError(401, 'expired', 'That sign-in attempt expired. Start again.'))).toBe('Your sign-in expired. Start again.');
      expect(authErrorMessage(new ApiError(401, 'bad_code', 'English'))).toBe('That code didn’t work.');
    });
  });
});
