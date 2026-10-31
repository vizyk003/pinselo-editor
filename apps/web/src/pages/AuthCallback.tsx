/**
 * /auth/callback — where every Supabase email link and OAuth sign-in comes back (plan 12 §C). Exchanges the PKCE
 * code (or a template's token_hash) once, then by flow: recovery → /auth/reset; re-auth, email change or any link
 * while signed in → sync the signed-in account; otherwise bridge to an Pinselo session and resume `next`.
 * A token_hash link first asks "Continue as …?" (review A3): unlike a PKCE code it isn't tied to this browser.
 */
import { useEffect, useRef, useState } from 'react';
import { Link, useLocation, useNavigate } from 'react-router';
import { useAuth } from '../App';
import { authConfigNow } from '../auth/authConfig';
import { EMAIL_SLOT, withEmail } from '../auth/bidi';
import { callbackErrorMessage, needsAccountConfirm, parseCallback } from '../auth/redirect';
import { signOutSupabaseLocal } from '../auth/session';
import { authErrorText, errorCode, FinishStep, must, useSupabaseFinish } from '../auth/SupabaseAuth';
import { syncSupabase } from '../auth/supabaseApi';
import { forgetStoredSession, getSupabase, storedSignIn } from '../auth/supabaseClient';
import { useToast } from '../components/ui';
import { t } from '../i18n';
import { Shell } from './Auth';

type State =
  | { kind: 'working' }
  | { kind: 'error'; text: string }
  | { kind: 'notice' }
  | { kind: 'confirm'; email: string }
  // Supabase now has a new address for this account, but another account here uses it (sync's emailConflict).
  | { kind: 'conflict'; email: string };

export function AuthCallback() {
  const { user, setUser } = useAuth();
  const nav = useNavigate();
  const loc = useLocation();
  const toast = useToast();
  const [cb] = useState(() => parseCallback(loc.search, loc.hash));
  const f = useSupabaseFinish(cb.next);
  const [state, setState] = useState<State>({ kind: 'working' });
  // The code is single-use: StrictMode's second effect run must not exchange it again.
  const started = useRef(false);
  // Set once "Continue as …?" is answered; leaving it unanswered counts as Cancel.
  const answered = useRef(false);
  const fail = (e: unknown) => setState({ kind: 'error', text: authErrorText(e) });

  // Once the code or link is verified (and, for a link, the account confirmed).
  const proceed = async () => {
    if (cb.flow === 'recovery') return nav(`/auth/reset?next=${encodeURIComponent(cb.next)}`, { replace: true });
    if (user) {
      // Already signed in here (re-auth, email change, or any link): check the Supabase user is this account, and sync its email.
      const session = (await (await getSupabase()).auth.getSession()).data.session;
      try {
        const r = await syncSupabase(session?.access_token ?? '');
        setUser(r.user);
        if (r.emailConflict) return setState({ kind: 'conflict', email: session?.user.email ?? '' });
        if (r.user.email !== user.email) toast(t('Email address changed to {email}', { email: r.user.email }));
      } catch (e) {
        if (errorCode(e) === 'wrong_account') await signOutSupabaseLocal();
        throw e;
      }
      return nav(cb.next, { replace: true });
    }
    await f.finish();
  };

  useEffect(() => {
    if (started.current) return;
    started.current = true;
    const run = async () => {
      // Only text we wrote: the URL's error_description is whatever the link's author put there (review A7).
      if (cb.error) return setState({ kind: 'error', text: callbackErrorMessage(cb.error) });
      const sb = await getSupabase();
      if (cb.code) must(await sb.auth.exchangeCodeForSession(cb.code));
      else if (cb.tokenHash && cb.type) {
        const { session } = must(await sb.auth.verifyOtp({ token_hash: cb.tokenHash, type: cb.type }));
        // The first of the two confirmations of a secure email change verifies without a session.
        if (!session) return setState(cb.flow === 'email_change' ? { kind: 'notice' } : { kind: 'error', text: t('This link has expired or was already used.') });
        if (needsAccountConfirm(cb, Boolean(user))) return setState({ kind: 'confirm', email: session.user.email ?? '' });
      }
      // The first of the two confirmations of a secure email change carries only a notice.
      else if (cb.message || cb.flow === 'email_change') return setState({ kind: 'notice' });
      else return setState({ kind: 'error', text: t('This link has expired or was already used.') });
      await proceed();
    };
    run().catch(fail);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // Leaving "Continue as …?" unanswered (another page here, or closing the tab) must not keep the link's session in
  // this browser: drop it in the app, or forget it synchronously when the page goes away.
  useEffect(() => {
    if (state.kind !== 'confirm') return;
    const url = authConfigNow()?.supabase?.url;
    const forget = () => {
      if (!answered.current && url) forgetStoredSession(url, storedSignIn(url));
    };
    window.addEventListener('pagehide', forget);
    return () => {
      window.removeEventListener('pagehide', forget);
      if (!answered.current) void signOutSupabaseLocal();
    };
  }, [state.kind]);

  if (f.step.kind !== 'form') return <FinishStep f={f} />;
  if (state.kind === 'confirm') {
    const cont = () => {
      answered.current = true;
      setState({ kind: 'working' });
      proceed().catch(fail);
    };
    // Not this person's link: forget the session it created here, and sign nobody in.
    const cancel = async () => {
      answered.current = true;
      setState({ kind: 'working' });
      await signOutSupabaseLocal();
      nav(user ? cb.next : '/login', { replace: true });
    };
    return (
      <Shell title={withEmail(t('Continue as {email}?', { email: EMAIL_SLOT }), state.email)} sub={t('Only continue if you asked for this link. It signs this browser in to that account.')}>
        <button className="btn primary tall" onClick={cont} autoFocus>{t('Continue')}</button>
        <button className="btn ghost" onClick={() => void cancel()}>{t('Cancel')}</button>
      </Shell>
    );
  }
  if (state.kind === 'conflict') {
    return (
      <Shell title={t('Email address not changed')} sub={withEmail(t('You now sign in with {email}. Another account here already uses that address, so the email on this account stays the same.', { email: EMAIL_SLOT }), state.email)}>
        <Link className="btn primary tall" to={cb.next}>{t('Continue')}</Link>
      </Shell>
    );
  }
  if (state.kind === 'notice') {
    return (
      <Shell title={t('Check your email')} sub={t('Now confirm the change from the email we sent to your other address.')}>
        <Link className="btn primary tall" to={user ? cb.next : '/'}>{t('Continue')}</Link>
      </Shell>
    );
  }
  if (state.kind === 'error') {
    return (
      <Shell title={t('That link didn’t work')} sub={state.text}>
        {user
          ? <Link className="btn primary tall" to={cb.next}>{t('Back')}</Link>
          : <Link className="btn primary tall" to={`/login${cb.next !== '/' ? `?next=${encodeURIComponent(cb.next)}` : ''}`}>{t('Back to log in')}</Link>}
      </Shell>
    );
  }
  return <Shell title={t('Signing you in…')}><div className="spinner" /></Shell>;
}
