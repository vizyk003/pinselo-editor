import { createContext, lazy, Suspense, useContext, useEffect, useRef, useState } from 'react';
import { BrowserRouter, Navigate, Route, Routes, useLocation, useNavigate } from 'react-router';
import { AuthDialog, AuthGateProvider } from './auth/AuthGate';
import { loadSession } from './auth/session';
import { loadAuthConfig } from './auth/authConfig';
import { safeNext } from './auth/redirect';
import { Landing } from './pages/Landing';
import { api, type User } from './api';
import { LimitHost } from './components/LimitHost';
import { ToastHost } from './components/ui';
import { Home } from './pages/Home';
import { ConsentBanner } from './components/ConsentBanner';
import { ErrorBoundary } from './components/ErrorBoundary';
import { DevMail } from './pages/DevMail';
import { HelpArticlePage, HelpContact, HelpHome, HelpRequests, LegalPage, StatusPage } from './pages/Public';
import { ForgotPassword, InviteLanding, Login, MagicLanding, ResetPassword, Signup, VerifyLanding } from './pages/Auth';
import { AuthCallback } from './pages/AuthCallback';
import { Presenter } from './pages/Presenter';
import { OAuthConsent } from './pages/OAuthConsent';
import { PublicView } from './pages/PublicView';
import { TemplateLinkPage } from './pages/TemplateLink';
import { applyAccountLocale, t, useLocale } from './i18n';
import { clearLegacyRecents } from './searchApi';

const EditorPage = lazy(() => import('./pages/EditorPage'));

interface AuthState { user: User | null; setUser: (u: User | null) => void }
const AuthCtx = createContext<AuthState>({ user: null, setUser: () => {} });
export const useAuth = () => useContext(AuthCtx);

/** Pages that need an account show the public home with the sign-in dialog; signing in resumes the page. */
/** /login or /signup once signed in: go where the sign-in was headed (?next), not always home — the form's own nav(next) and this redirect can race. */
function SignedInRedirect() {
  const n = new URLSearchParams(useLocation().search).get('next');
  return <Navigate to={safeNext(n)} replace />;
}

function RequireAuth({ children }: { children: React.ReactNode }) {
  const { user } = useAuth();
  if (!user) return <AuthWall />;
  return <>{children}</>;
}

function AuthWall() {
  const loc = useLocation();
  const nav = useNavigate();
  const [mode, setMode] = useState<'login' | 'signup'>('login');
  return (
    <>
      <Landing />
      <AuthDialog mode={mode} next={loc.pathname + loc.search} onMode={setMode} onClose={() => nav('/', { replace: true })} />
    </>
  );
}

/** Signed out: the home and template gallery are public; everything else asks to sign in. */
function PublicHome() {
  const loc = useLocation();
  if (loc.pathname === '/') return <Landing />;
  if (loc.pathname === '/templates') return <Landing templatesOnly />;
  return <AuthWall />;
}

export function App() {
  const [user, setUser] = useState<User | null>(null);
  const [ready, setReady] = useState(false);
  /** /api/auth/me failed for a reason other than 401 (network, server restarting): retrying, not signed out. */
  const [reconnecting, setReconnecting] = useState(false);
  const wake = useRef<() => void>(() => {});
  useEffect(() => {
    // Recent searches/templates are now kept per account; remove the old browser-wide lists.
    clearLegacyRecents();
    // Which sign-in the server uses (built-in or Supabase); the sign-in forms wait for it.
    loadAuthConfig().catch(() => {});
    const ac = new AbortController();
    // Backoff sleep that "Retry now" and coming back online cut short.
    const sleep = (ms: number) => new Promise<void>((r) => {
      const id = setTimeout(r, ms);
      wake.current = () => { clearTimeout(id); r(); };
    });
    const online = () => wake.current();
    window.addEventListener('online', online);
    loadSession(api.me, { sleep, signal: ac.signal, onRetry: () => setReconnecting(true) })
      .then(async (u) => {
        if (ac.signal.aborted) return;
        setUser(u);
        // The account's language wins over the browser's once signed in.
        await applyAccountLocale(u?.locale).catch(() => {});
        setReconnecting(false);
        setReady(true);
      })
      .catch(() => { /* aborted (unmounted) */ });
    return () => {
      ac.abort();
      wake.current();
      window.removeEventListener('online', online);
    };
  }, []);
  const locale = useLocale();
  if (!ready) {
    return reconnecting ? (
      <div className="empty" role="status" style={{ display: 'grid', placeItems: 'center', gap: 12, height: '100%', alignContent: 'center' }}>
        <span>{t('Connecting…')}</span>
        <button className="btn sm" onClick={() => wake.current()}>{t('Retry now')}</button>
      </div>
    ) : null;
  }
  return (
    <AuthCtx.Provider value={{ user, setUser }} key={locale}>
      <ToastHost>
        <BrowserRouter>
          <AuthGateProvider>
          <Routes>
            <Route path="/login" element={user ? <SignedInRedirect /> : <Login />} />
            <Route path="/signup" element={user ? <SignedInRedirect /> : <Signup />} />
            <Route path="/auth/magic" element={<MagicLanding />} />
            <Route path="/auth/verify" element={<VerifyLanding />} />
            <Route path="/auth/forgot" element={<ForgotPassword />} />
            <Route path="/auth/reset" element={<ResetPassword />} />
            <Route path="/auth/callback" element={<AuthCallback />} />
            <Route path="/invite" element={<InviteLanding />} />
            <Route path="/dev/mail" element={<DevMail />} />
            <Route path="/legal/:slug" element={<LegalPage />} />
            <Route path="/help" element={<HelpHome />} />
            <Route path="/help/contact" element={<HelpContact />} />
            <Route path="/help/requests" element={<HelpRequests />} />
            <Route path="/help/:slug" element={<HelpArticlePage />} />
            <Route path="/status" element={<StatusPage />} />
            <Route path="/view/:id" element={<PublicView />} />
            <Route path="/oauth/authorize" element={<OAuthConsent />} />
            {/* Template links (g136) stay public: the page shows a preview and asks for sign-in itself. */}
            <Route path="/t/:token" element={<TemplateLinkPage />} />
            <Route path="/design/:id/presenter" element={<RequireAuth><Presenter /></RequireAuth>} />
            <Route path="/design/:id" element={<RequireAuth><ErrorBoundary label="editor"><Suspense fallback={<div className="empty">{t('Loading editor…')}</div>}><EditorPage /></Suspense></ErrorBoundary></RequireAuth>} />
            <Route path="/*" element={user ? <Home /> : <PublicHome />} />
          </Routes>
          <LimitHost />
          <ConsentBanner />
          </AuthGateProvider>
        </BrowserRouter>
      </ToastHost>
    </AuthCtx.Provider>
  );
}
