import { beforeEach, describe, expect, it } from 'vitest';

// The i18n module touches localStorage and <html lang dir>; give it minimal stand-ins (node environment).
const store = new Map<string, string>();
Object.assign(globalThis, {
  localStorage: { getItem: (k: string) => store.get(k) ?? null, setItem: (k: string, v: string) => void store.set(k, v), removeItem: (k: string) => void store.delete(k) },
  document: { documentElement: { lang: 'en', dir: 'ltr' } },
});
const i18n = await import('../apps/web/src/i18n');

describe('language switch (I-01)', () => {
  beforeEach(async () => {
    store.clear();
    await i18n.setLocale('en');
    store.clear();
  });

  it('a choice applies live and is remembered, including back to English', async () => {
    await i18n.chooseLocale('de');
    expect(i18n.getLocale()).toBe('de');
    expect(i18n.t('Language')).toBe('Sprache');
    expect(store.get('oc.locale')).toBe('de');
    expect(document.documentElement.lang).toBe('de');
    await i18n.chooseLocale('en');
    expect(i18n.getLocale()).toBe('en');
    expect(i18n.t('Language')).toBe('Language');
    expect(store.get('oc.locale')).toBe('en');
  });

  it("the account's default 'en' doesn't override the browser language", async () => {
    await i18n.setLocale('fr'); // detected from the browser
    await i18n.applyAccountLocale('en');
    expect(i18n.getLocale()).toBe('fr');
  });

  it("an account language other than 'en' is applied at sign-in", async () => {
    await i18n.applyAccountLocale('de');
    expect(i18n.getLocale()).toBe('de');
  });

  it('switching back to English on another device applies here too (stale oc.locale)', async () => {
    await i18n.applyAccountLocale('de'); // this browser saw the account in German
    expect(i18n.getLocale()).toBe('de');
    await i18n.applyAccountLocale('en'); // changed to English elsewhere
    expect(i18n.getLocale()).toBe('en');
    expect(store.get('oc.locale')).toBe('en');
  });

  it('a local choice survives a reload when the account matches it', async () => {
    await i18n.chooseLocale('en');
    await i18n.setLocale('en');
    await i18n.applyAccountLocale('en');
    expect(i18n.getLocale()).toBe('en');
    await i18n.chooseLocale('ja');
    await i18n.applyAccountLocale('ja');
    expect(i18n.getLocale()).toBe('ja');
  });
});
