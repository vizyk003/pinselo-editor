import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { setAssetOrigin } from './api';
import { detectLocale, setLocale } from './i18n';
import { PwaUpdateBanner, registerServiceWorker } from './pwa';
import { initNative, isNative } from './native';
import { applyAppearance } from './lib/appearance';
import './styles.css';

// Theme, contrast and motion preferences (Settings → Accessibility) before the first paint.
applyAppearance();

// The app's modules import @oc/engine, which instantiates the Rust core (WebAssembly) with top-level await: start
// loading them now, alongside the runtime config, and render once both are in.
const app = import('./App');

// Runtime config (user-content origin); the app still boots if it is unavailable.
await fetch('/api/config')
  .then((r) => (r.ok ? r.json() : null))
  .then((c: { assetOrigin?: string } | null) => {
    if (c?.assetOrigin) setAssetOrigin(c.assetOrigin);
  })
  .catch(() => {});

await setLocale(detectLocale()).catch(() => {});

// The native shell has its own update channel; the service worker is for browsers.
if (!isNative()) registerServiceWorker();
void initNative().catch(() => {});

const { App } = await app;
createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <App />
    <PwaUpdateBanner />
  </StrictMode>,
);
