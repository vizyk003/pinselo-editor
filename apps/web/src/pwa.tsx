/**
 * PWA glue (PWA-001..004): registers the service worker in production builds, offers "Update available"
 * when a new version is waiting, and an install button where the browser supports it.
 */
import { Download, RefreshCw, X } from 'lucide-react';
import { useEffect, useState } from 'react';
import { t } from './i18n';

interface InstallPrompt extends Event { prompt(): Promise<void>; userChoice: Promise<{ outcome: 'accepted' | 'dismissed' }> }

let waiting: ServiceWorker | null = null;
let installEvt: InstallPrompt | null = null;
const listeners = new Set<() => void>();
const changed = () => listeners.forEach((l) => l());

export function registerServiceWorker() {
  if (!import.meta.env.PROD || !('serviceWorker' in navigator)) return;
  window.addEventListener('load', async () => {
    try {
      const reg = await navigator.serviceWorker.register('/sw.js', { scope: '/' });
      const watch = (w: ServiceWorker | null) => w?.addEventListener('statechange', () => {
        if (w.state === 'installed' && navigator.serviceWorker.controller) { waiting = w; changed(); }
      });
      if (reg.waiting && navigator.serviceWorker.controller) { waiting = reg.waiting; changed(); }
      reg.addEventListener('updatefound', () => watch(reg.installing));
      // Check for a new build every hour while the app stays open.
      setInterval(() => void reg.update().catch(() => {}), 3600_000);
    } catch {
      /* unsupported or blocked: the app works without it */
    }
  });
  let reloading = false;
  navigator.serviceWorker.addEventListener('controllerchange', () => {
    if (!reloading) { reloading = true; location.reload(); }
  });
}

window.addEventListener('beforeinstallprompt', (e) => {
  e.preventDefault();
  installEvt = e as InstallPrompt;
  changed();
});
window.addEventListener('appinstalled', () => { installEvt = null; changed(); });

export const canInstall = () => installEvt !== null;
export async function install(): Promise<boolean> {
  if (!installEvt) return false;
  await installEvt.prompt();
  const { outcome } = await installEvt.userChoice;
  installEvt = null;
  changed();
  return outcome === 'accepted';
}

export function usePwa() {
  const [, force] = useState(0);
  useEffect(() => {
    const l = () => force((n) => n + 1);
    listeners.add(l);
    return () => void listeners.delete(l);
  }, []);
  return { updateReady: waiting !== null, canInstall: installEvt !== null };
}

/** "Update available" banner. Never interrupts: the user chooses when to reload. */
export function PwaUpdateBanner() {
  const { updateReady } = usePwa();
  const [hidden, setHidden] = useState(false);
  if (!updateReady || hidden) return null;
  return (
    <div className="pwa-banner" role="status">
      <RefreshCw size={16} aria-hidden />
      <span>{t('A new version of Pinselo is available.')}</span>
      <button className="btn sm primary" onClick={() => waiting?.postMessage('skip-waiting')}>{t('Reload')}</button>
      <button className="btn ghost sm icon" aria-label={t('Dismiss')} onClick={() => setHidden(true)}><X size={14} /></button>
    </div>
  );
}

export function InstallButton({ className = 'btn sm' }: { className?: string }) {
  const { canInstall: show } = usePwa();
  if (!show) return null;
  return <button className={className} onClick={() => void install()}><Download size={14} aria-hidden /> {t('Install app')}</button>;
}
