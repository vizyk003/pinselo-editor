/**
 * Template link landing (g136): /t/<token> shows what the link hands out, then makes the visitor their own
 * copy. Signing in goes through the AuthGate resume flow (`?use=1` makes the copy right after). The copy
 * never gives access to the original design.
 */
import { BRAND } from '@oc/config';
import { formatDims, type Unit } from '@oc/engine';
import { LayoutTemplate } from 'lucide-react';
import { useEffect, useRef, useState } from 'react';
import { Link, useNavigate, useParams, useSearchParams } from 'react-router';
import { ApiError } from '../api';
import { sharingApi, type TemplateLinkInfo } from '../api/sharing';
import { useAuth } from '../App';
import { useAuthGate } from '../auth/AuthGate';
import { t, tn } from '../i18n';

/**
 * Marks that this browser asked for a copy of `token` just before signing in, so the `?use=1` return makes it.
 * Without the mark (say, someone sent a signed-in person a ?use=1 link) the page waits for a click instead.
 * localStorage, not sessionStorage: a magic sign-in link may open in another tab. Expires after 30 minutes.
 */
const INTENT_KEY = 'oc:template-link-use';
function markIntent(token: string) {
  try {
    localStorage.setItem(INTENT_KEY, JSON.stringify({ token, at: Date.now() }));
  } catch {
    /* storage blocked: the visitor clicks once more after signing in */
  }
}
function takeIntent(token: string): boolean {
  try {
    const v = JSON.parse(localStorage.getItem(INTENT_KEY) ?? 'null') as { token?: string; at?: number } | null;
    if (v?.token !== token) return false;
    localStorage.removeItem(INTENT_KEY);
    return Date.now() - (v.at ?? 0) < 30 * 60_000;
  } catch {
    return false;
  }
}

export function TemplateLinkPage() {
  const { token } = useParams() as { token: string };
  const [params] = useSearchParams();
  const { user } = useAuth();
  const { requireAuth } = useAuthGate();
  const nav = useNavigate();
  const [info, setInfo] = useState<TemplateLinkInfo | null>(null);
  const [err, setErr] = useState('');
  const [busy, setBusy] = useState(false);
  const resumed = useRef(false);

  useEffect(() => {
    sharingApi.templateLink(token).then((i) => {
      setInfo(i);
      document.title = `${i.title} – ${BRAND.name}`;
    }).catch(() => setErr(t('This template link was turned off or doesn’t exist.')));
  }, [token]);

  const use = () => {
    // Signed out: the sign-in dialog comes back here with ?use=1, which makes the copy straight away.
    if (!requireAuth(`/t/${encodeURIComponent(token)}?use=1`)) {
      markIntent(token);
      return;
    }
    setBusy(true);
    sharingApi.useTemplateLink(token)
      .then((r) => nav(`/design/${r.id}`, { replace: true }))
      .catch((e) => {
        setBusy(false);
        setErr(e instanceof ApiError && e.status === 404 ? t('This template link was turned off or doesn’t exist.') : t('That didn’t work. Try again in a moment.'));
      });
  };
  // Signing in in the dialog sets the user and then navigates to ?use=1 (a transition that lands later), so this
  // waits for both; only a copy this browser asked for is made without a click.
  const wantUse = params.get('use') === '1';
  useEffect(() => {
    if (user && info && wantUse && !resumed.current) {
      resumed.current = true;
      if (takeIntent(token)) use();
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [user, info, wantUse]);

  if (err) {
    return (
      <div className="auth"><div className="auth-card">
        <h2 style={{ margin: 0 }}>{t('Not available')}</h2>
        <p className="muted">{err}</p>
        <Link className="btn" to="/">{t('Go to {brand}', { brand: BRAND.name })}</Link>
      </div></div>
    );
  }
  if (!info) return <div className="empty">{t('Loading…')}</div>;
  return (
    <div className="auth">
      <div className="auth-card" style={{ gap: 14 }}>
        <div style={{ display: 'grid', placeItems: 'center', background: 'var(--workspace)', borderRadius: 'var(--radius)', minHeight: 160, overflow: 'hidden' }}>
          {info.thumb
            ? <img src={sharingApi.templateThumbUrl(token)} alt="" style={{ maxWidth: '100%', maxHeight: 280, display: 'block' }} />
            : <LayoutTemplate size={48} className="faint" />}
        </div>
        <div className="col" style={{ gap: 4 }}>
          <h2 style={{ margin: 0 }}>{info.title}</h2>
          <span className="faint" style={{ fontSize: 13 }}>
            {t('by {owner}', { owner: info.owner })} · {tn(info.pages, '{n} page', '{n} pages')} · {formatDims(info.width, info.height, info.unit as Unit)}
          </span>
        </div>
        <p className="muted" style={{ margin: 0 }}>{t('You’ll get your own copy to edit. The original stays with {owner}, and your changes never touch it.', { owner: info.owner })}</p>
        <button className="btn primary tall" onClick={use} disabled={busy}>{busy ? t('Making your copy…') : t('Use this template')}</button>
        {!user && <p className="faint" style={{ margin: 0, fontSize: 12, textAlign: 'center' }}>{t('You’ll sign in or create a free account first.')}</p>}
      </div>
    </div>
  );
}
