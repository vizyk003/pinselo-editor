/**
 * Pure helpers for sign-in redirects (plan 12 §C, §C4): the open-redirect-safe `next` rule (shared with the
 * server's `safePath`), Supabase callback URLs and their parsing, the profile check of the needsProfile step,
 * and the user-facing text for Supabase and bridge error codes. No DOM or supabase-js here, so it unit-tests in node.
 */
import { BRAND } from '@oc/config';
import { t } from '../i18n';

/** Where a sign-in resumes. Only a same-site path: `/…` but not `//…` or `/\…`, no backslash, control character or whitespace. */
export function safeNext(n: unknown): string {
  return typeof n === 'string' && n.length <= 2000 && /^\/(?![/\\])/.test(n) && !/[\\\s\x00-\x1f\x7f]/.test(n) ? n : '/';
}

/** Which Supabase email-link flow brought the user back to /auth/callback (`signin` is the default and left out of the URL). */
export type AuthFlow = 'signin' | 'recovery' | 'email_change' | 'reauth';
const FLOWS: readonly AuthFlow[] = ['signin', 'recovery', 'email_change', 'reauth'];

/** The `redirectTo` / `emailRedirectTo` for every Supabase call: `${origin}/auth/callback?next=…&flow=…`. */
export function callbackUrl(next: string, flow: AuthFlow = 'signin', origin: string = globalThis.location?.origin ?? ''): string {
  const q = new URLSearchParams({ next: safeNext(next) });
  if (flow !== 'signin') q.set('flow', flow);
  return `${origin}/auth/callback?${q}`;
}

/**
 * `signInWithOAuth` options. Google always shows its account chooser, so someone signed in to several Gmail accounts
 * picks one instead of being signed in silently with the browser's default (and can pick the right one to reauthenticate).
 */
export function oauthOptions(provider: string, next: string, flow: AuthFlow = 'signin', origin?: string): { redirectTo: string; queryParams?: Record<string, string> } {
  const redirectTo = callbackUrl(next, flow, origin);
  return provider === 'google' ? { redirectTo, queryParams: { prompt: 'select_account' } } : { redirectTo };
}

/** Google first (most people have a Gmail account), the rest in Supabase's order. */
export const orderProviders = <P extends { id: string }>(providers: readonly P[]): P[] =>
  [...providers].sort((a, b) => Number(b.id === 'google') - Number(a.id === 'google'));

/** `verifyOtp` types a custom email template may link with (`token_hash` links work across browsers, plan 12 §E). */
export type EmailLinkType = 'signup' | 'invite' | 'magiclink' | 'recovery' | 'email_change' | 'email';
const LINK_TYPES: readonly EmailLinkType[] = ['signup', 'invite', 'magiclink', 'recovery', 'email_change', 'email'];

export interface CallbackParams {
  code: string | null;
  tokenHash: string | null;
  type: EmailLinkType | null;
  next: string;
  flow: AuthFlow;
  /** Only the code: `error_description` is attacker-controlled text and is never shown (review A7). */
  error: { code: string } | null;
  /** Supabase's notice without a code, e.g. the first of the two confirmations of a secure email change. */
  message: string | null;
}

/** Reads /auth/callback: PKCE `code` or `token_hash` + `type`, our `next`/`flow`, and Supabase's errors (query for PKCE, hash for implicit). */
export function parseCallback(search: string, hash = ''): CallbackParams {
  const q = new URLSearchParams(search.replace(/^\?/, ''));
  const h = new URLSearchParams(hash.replace(/^#/, ''));
  const get = (k: string) => q.get(k) || h.get(k) || null;
  const type = get('type');
  const linkType = LINK_TYPES.includes(type as EmailLinkType) ? (type as EmailLinkType) : null;
  const flowParam = get('flow');
  // A recovery or email-change link is that flow whatever the URL says (custom templates may omit `flow`).
  const flow: AuthFlow = linkType === 'recovery' ? 'recovery' : linkType === 'email_change' ? 'email_change'
    : FLOWS.includes(flowParam as AuthFlow) ? (flowParam as AuthFlow) : 'signin';
  const errCode = get('error_code') ?? get('error');
  return {
    code: get('code'),
    tokenHash: get('token_hash'),
    type: linkType,
    next: safeNext(get('next')),
    flow,
    error: errCode ? { code: errCode } : null,
    message: get('message'),
  };
}

/**
 * Login CSRF guard (review A3). A `token_hash` link isn't bound to this browser (a PKCE code is), so anyone can send
 * one for their own account. Before it signs this browser in, or resets a password, the user confirms the account.
 * Signed in, other flows only `sync` the current account, which the server refuses for a different Supabase user.
 */
export function needsAccountConfirm(cb: Pick<CallbackParams, 'tokenHash' | 'flow'>, signedIn: boolean): boolean {
  return Boolean(cb.tokenHash) && (!signedIn || cb.flow === 'recovery');
}

export const MIN_AGE = 13;

/** What's missing from the needsProfile step (same rules as sign-up and the magic-link landing); null when complete. */
export function profileProblem(p: { name: string; birthYear: number; terms: boolean }, thisYear: number): 'name' | 'birthYear' | 'age' | 'terms' | null {
  if (!p.name.trim() || p.name.trim().length > 80) return 'name';
  if (!Number.isInteger(p.birthYear) || p.birthYear < 1900 || p.birthYear > thisYear) return 'birthYear';
  if (thisYear - p.birthYear < MIN_AGE) return 'age';
  if (!p.terms) return 'terms';
  return null;
}

/** OC_SIGNUP=invite: the note on the sign-up form and the refusal's text. */
export const signupClosedText = (): string => t('New accounts are by invitation only. Use the email address your invitation was sent to.');

/**
 * Text for a Supabase Auth error code (supabase-js `AuthError.code`, or the `error_code` of a callback URL) or an
 * Pinselo bridge error code / `link_conflict` reason. Null for unknown codes: show the caller's fallback.
 */
export function supabaseErrorMessage(code: string | null | undefined): string | null {
  switch (code) {
    case 'signup_closed':
      return signupClosedText();
    // Supabase Auth
    case 'invalid_credentials':
      return t('Wrong email or password.');
    case 'email_not_confirmed':
    case 'provider_email_needs_verification':
    case 'unconfirmed_supabase':
      return t('Confirm your email address first — use the link we emailed you — then sign in again.');
    case 'over_request_rate_limit':
    case 'over_email_send_rate_limit':
      return t('Too many attempts. Wait a few minutes and try again.');
    case 'weak_password':
      return t('Choose a stronger password: at least 10 characters that aren’t easy to guess.');
    case 'same_password':
      return t('Choose a password you haven’t used here before.');
    case 'otp_expired':
    case 'flow_state_expired':
      return t('This link has expired or was already used.');
    case 'flow_state_not_found':
    case 'bad_code_verifier':
    case 'pkce_code_verifier_not_found':
      return t('Open the link in the same browser you used to request it.');
    case 'mfa_verification_failed':
    case 'mfa_challenge_expired':
    case 'bad_code':
      return t('That code didn’t work.');
    case 'signup_disabled':
    case 'email_provider_disabled':
    case 'provider_disabled':
    case 'otp_disabled':
    case 'supabase_off':
    case 'supabase_auth':
      return t('This sign-in method isn’t available here.');
    case 'access_denied':
      return t('Sign-in with that provider failed. Try again.');
    case 'email_exists':
    case 'user_already_exists':
      return t('An account with this email address already exists.');
    case 'validation_failed':
      return t('Check what you entered and try again.');
    case 'email_address_not_authorized':
      return t('We can’t send email to that address.');
    case 'reauth_required':
      return t('For your security, sign in again to continue.');
    case 'mfa_required':
    case 'insufficient_aal':
      return t('Enter the 6-digit code from your authenticator app.');
    case 'user_banned':
    case 'disabled':
      return t('This account is disabled. Contact support if you think this is a mistake.');
    case 'reauthentication_not_valid':
      return t('That code didn’t work.');
    case 'email_address_invalid':
      return t('Enter a valid email address.');
    // Pinselo bridge
    case 'invalid_token':
    case 'session_ended':
    case 'token_used':
    case 'expired': // the built-in code step's ticket (POST /api/auth/mfa)
    case 'session_not_found':
    case 'session_expired':
    case 'refresh_token_not_found':
    case 'refresh_token_already_used':
    case 'bad_jwt':
    case 'invalid_jwt':
      return t('Your sign-in expired. Start again.');
    case 'linked_elsewhere':
      return t('This email address is already used by a different sign-in. Contact support.');
    // The account changed between the first step and the code step (a concurrent link, email change or delete).
    case 'changed_during_signin':
      return t('Your account changed while you were signing in. Sign in again.');
    // The project confirms email addresses automatically, or its settings couldn't be read: no linking by email.
    case 'email_unverifiable':
      return t('Accounts can’t be linked by email on this server. Sign in with your original method or contact support.');
    // No longer sent (an unconfirmed account here now gives way to a fresh one); kept for older servers.
    case 'unverified_local':
      return t('An account with this email already exists here, but its address was never confirmed, so it can’t be linked automatically. Contact support.');
    case 'age':
      return t('You need to be at least {age} to use {brand}.', { age: MIN_AGE, brand: BRAND.name });
    case 'email_required':
      return t('Sign in with an account that has an email address.');
    case 'anonymous_user':
    case 'bad_origin':
      return t('That didn’t work. Try again.');
    case 'wrong_account':
      return t('That’s a different account. Sign in as the account you’re using here.');
    case 'supabase_unavailable':
    case 'server_error':
    case 'unexpected_failure':
    case 'request_timeout':
    case 'internal':
      return t('Sign-in is unavailable right now. Try again in a minute.');
    default:
      return null;
  }
}

/** What a thrown error carries, duck-typed: Pinselo `ApiError`/`BridgeError`, supabase-js `AuthError`, fetch's `TypeError`. */
interface ErrorLike { code?: unknown; reason?: unknown; name?: unknown; status?: unknown }

/** supabase-js errors that come without a code. */
const NAMED: Record<string, string> = {
  AuthSessionMissingError: 'session_not_found',
  AuthPKCECodeVerifierMissingError: 'pkce_code_verifier_not_found',
  AuthRetryableFetchError: 'supabase_unavailable',
};

/** The code to show text for: a link_conflict's `reason`, the error's `code`, or one implied by a supabase-js error name. */
export function errorCodeOf(e: unknown): string {
  if (!e || typeof e !== 'object') return '';
  const { code, reason, name } = e as ErrorLike;
  if (typeof reason === 'string' && reason) return reason;
  if (typeof code === 'string' && code) return code;
  return typeof name === 'string' ? NAMED[name] ?? '' : '';
}

/**
 * User-facing text for any sign-in error (review C5): a known code's translated text; a service that can't be
 * reached (network failure, timeout, 5xx) → "unavailable"; 429 → "too many attempts"; anything else a generic line.
 * Never the error's own message: Supabase's are English and developer-facing.
 */
export function authErrorMessage(e: unknown): string {
  const known = supabaseErrorMessage(errorCodeOf(e));
  if (known) return known;
  const { name, status } = (e && typeof e === 'object' ? e : {}) as ErrorLike;
  if (status === 429) return t('Too many attempts. Wait a few minutes and try again.');
  // fetch() rejects with a TypeError when offline; supabase-js and our API report 0 or 5xx for an unreachable service.
  if (e instanceof TypeError || name === 'TimeoutError' || (typeof status === 'number' && (status === 0 || status >= 500))) {
    return t('Sign-in is unavailable right now. Try again in a minute.');
  }
  return t('Something went wrong. Try again.');
}

/** The text for an error that came back in /auth/callback's URL: a known code's, never its `error_description` (review A7). */
export const callbackErrorMessage = (error: { code: string }): string => supabaseErrorMessage(error.code) ?? t('Something went wrong. Try again.');

