/**
 * Native shell integration (MOB-010/011): when running inside the Capacitor app, the Android back
 * button navigates history (and exits at the root), the status bar follows the theme, and sharing
 * uses the system share sheet. Plugins load lazily, so the web bundle never pays for them.
 */
import { Capacitor } from '@capacitor/core';

export const isNative = () => Capacitor.isNativePlatform();

export async function initNative() {
  if (!isNative()) return;
  document.documentElement.classList.add('native', `native-${Capacitor.getPlatform()}`);
  const [{ App }, { StatusBar, Style }] = await Promise.all([import('@capacitor/app'), import('@capacitor/status-bar')]);
  void App.addListener('backButton', ({ canGoBack }) => (canGoBack ? history.back() : void App.exitApp()));
  const dark = matchMedia('(prefers-color-scheme: dark)');
  const apply = () => void StatusBar.setStyle({ style: dark.matches ? Style.Dark : Style.Light }).catch(() => {});
  apply();
  dark.addEventListener('change', apply);
}

/** Whether a system share sheet exists (the app, or browsers with the Web Share API — mostly phones). */
export const canShareNatively = () => isNative() || typeof navigator.share === 'function';

/** System share sheet in the app; Web Share API in browsers that have it; false when neither exists. */
export async function shareLink(url: string, title: string): Promise<boolean> {
  if (isNative()) {
    const { Share } = await import('@capacitor/share');
    await Share.share({ title, url, dialogTitle: title });
    return true;
  }
  if (navigator.share) {
    await navigator.share({ title, url }).catch(() => {});
    return true;
  }
  return false;
}
