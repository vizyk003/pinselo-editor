/**
 * Capacitor shell for the app stores (MOB-010). Two modes:
 * - Hosted (default for production): the native shell loads your deployment (`OC_APP_URL`), so the
 *   app, API and real-time sync stay same-origin and updates ship without a store release.
 * - Bundled: leave OC_APP_URL unset to package `apps/web/dist` (useful for offline-first demos;
 *   API calls then need the deployment to allow the capacitor:// origin).
 * Generate the native projects once with `pnpm --filter @oc/mobile add:ios` / `add:android`.
 */
import type { CapacitorConfig } from '@capacitor/cli';

const url = process.env.OC_APP_URL;

const config: CapacitorConfig = {
  appId: process.env.OC_APP_ID ?? 'app.pinselo',
  appName: 'Pinselo',
  webDir: '../web/dist',
  ...(url ? { server: { url, cleartext: url.startsWith('http://'), allowNavigation: [new URL(url).host] } } : {}),
  ios: { contentInset: 'never', limitsNavigationsToAppBoundDomains: Boolean(url) },
  android: { allowMixedContent: false },
  plugins: {
    StatusBar: { overlaysWebView: false, style: 'DARK', backgroundColor: '#080e2a' },
  },
};

export default config;
