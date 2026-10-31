/**
 * Sign-in on demand: browsing (home, templates, help, pricing, public views) never asks for an account.
 * Anything that needs one — creating or opening a design, using a template, account pages — calls
 * `requireAuth(next)`, which opens this dialog. After signing in (or up) the app navigates to `next`,
 * an URL that resumes the action (e.g. `/?template=<id>` creates the design from that template).
 */
import { createContext, useCallback, useContext, useEffect, useMemo, useState, type ReactNode } from 'react';
import { useLocation } from 'react-router';
import { useAuth } from '../App';
import { Modal } from '../components/ui';
import { t } from '../i18n';
import { Login, Signup } from '../pages/Auth';

type Mode = 'login' | 'signup';
interface Gate {
  /** True when signed in; otherwise opens the sign-in dialog and returns false. */
  requireAuth: (next: string, mode?: Mode) => boolean;
  openAuth: (mode: Mode, next?: string) => void;
}
const GateCtx = createContext<Gate>({ requireAuth: () => false, openAuth: () => {} });
export const useAuthGate = () => useContext(GateCtx);

export function AuthGateProvider({ children }: { children: ReactNode }) {
  const { user } = useAuth();
  const [state, setState] = useState<{ mode: Mode; next: string; onClose?: () => void } | null>(null);
  const openAuth = useCallback((mode: Mode, next = '/') => setState({ mode, next }), []);
  const requireAuth = useCallback((next: string, mode: Mode = 'signup') => {
    if (user) return true;
    setState({ mode, next });
    return false;
  }, [user]);
  // Signing in fulfils the dialog (so it can't resurface on sign-out); following a link inside it (Forgot?, dev mailbox) leaves it.
  const { pathname } = useLocation();
  useEffect(() => { if (user) setState(null); }, [user]);
  useEffect(() => setState(null), [pathname]);
  const gate = useMemo(() => ({ requireAuth, openAuth }), [requireAuth, openAuth]);
  return (
    <GateCtx.Provider value={gate}>
      {children}
      {state && !user && <AuthDialog mode={state.mode} next={state.next} onMode={(mode) => setState({ ...state, mode })} onClose={() => { state.onClose?.(); setState(null); }} />}
    </GateCtx.Provider>
  );
}

export function AuthDialog({ mode, next, onMode, onClose }: { mode: Mode; next: string; onMode: (m: Mode) => void; onClose: () => void }) {
  return (
    <Modal title={mode === 'login' ? t('Log in') : t('Sign up')} onClose={onClose} size="sm">
      <div className="auth-dialog">
        {mode === 'login'
          ? <Login embedded next={next} onSwitch={() => onMode('signup')} />
          : <Signup embedded next={next} onSwitch={() => onMode('login')} />}
      </div>
    </Modal>
  );
}
