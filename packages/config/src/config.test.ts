/** Supabase settings in @oc/config (plan 12 §1): mode detection, issuer, and the startup refusals. */
import { afterEach, describe, expect, it, vi } from 'vitest';
import { assertProductionConfig, config, supabaseAuthEnabled, supabaseIssuer } from './index';

const saved = structuredClone({ env: config.env, supabase: config.supabase, secret: config.secret, webOrigin: config.webOrigin, uc: config.usercontentOrigin });
afterEach(() => {
  Object.assign(config, { env: saved.env, secret: saved.secret, webOrigin: saved.webOrigin, usercontentOrigin: saved.uc });
  config.supabase = { ...saved.supabase };
  vi.restoreAllMocks();
});

const b64url = (o: object) => btoa(JSON.stringify(o)).replace(/=+$/, '').replace(/\+/g, '-').replace(/\//g, '_');
const legacyKey = (role: string) => `${b64url({ alg: 'HS256', typ: 'JWT' })}.${b64url({ iss: 'supabase', role })}.sig`;

/** Runs assertProductionConfig and returns the exit code (null = started) and everything logged. */
function boot(): { exit: number | null; logged: string } {
  const exit = vi.spyOn(process, 'exit').mockImplementation(((code?: number) => {
    throw Object.assign(new Error('exit'), { code });
  }) as never);
  const lines: string[] = [];
  vi.spyOn(console, 'error').mockImplementation((m) => void lines.push(String(m)));
  vi.spyOn(console, 'warn').mockImplementation((m) => void lines.push(String(m)));
  try {
    assertProductionConfig();
    return { exit: null, logged: lines.join('\n') };
  } catch (e) {
    return { exit: (e as { code?: number }).code ?? -1, logged: lines.join('\n') };
  } finally {
    exit.mockRestore();
  }
}

const production = () => Object.assign(config, { env: 'production', secret: 'x'.repeat(40), webOrigin: 'https://design.example.com', usercontentOrigin: '' });

describe('supabaseAuthEnabled / supabaseIssuer', () => {
  it('is on only with both URL and publishable key, evaluated per call', () => {
    config.supabase = { url: '', publishableKey: '', secretKey: '', jwtSecret: '', jwtIssuer: '' };
    expect(supabaseAuthEnabled()).toBe(false);
    config.supabase.url = 'https://abc.supabase.co';
    expect(supabaseAuthEnabled()).toBe(false);
    config.supabase.publishableKey = 'sb_publishable_x';
    expect(supabaseAuthEnabled()).toBe(true);
  });

  it('derives the issuer from the URL unless overridden', () => {
    config.supabase = { url: 'https://abc.supabase.co', publishableKey: 'k', secretKey: '', jwtSecret: '', jwtIssuer: '' };
    expect(supabaseIssuer()).toBe('https://abc.supabase.co/auth/v1');
    config.supabase.url = 'https://abc.supabase.co/';
    expect(supabaseIssuer()).toBe('https://abc.supabase.co/auth/v1');
    config.supabase.jwtIssuer = 'https://auth.example.com/auth/v1';
    expect(supabaseIssuer()).toBe('https://auth.example.com/auth/v1');
  });
});

describe('assertProductionConfig: Supabase', () => {
  const secret = 'sb_secret_DO_NOT_LOG_123';

  it('refuses a secret key as the publishable key in every environment, without logging it', () => {
    for (const env of ['development', 'production']) {
      config.env = env;
      config.supabase = { url: 'https://abc.supabase.co', publishableKey: secret, secretKey: '', jwtSecret: '', jwtIssuer: '' };
      const r = boot();
      expect(r.exit).toBe(1);
      expect(r.logged).toMatch(/OC_SUPABASE_PUBLISHABLE_KEY/);
      expect(r.logged).not.toContain(secret);
      config.supabase.publishableKey = legacyKey('service_role');
      expect(boot().exit).toBe(1);
    }
  });

  it('accepts the legacy anon key and starts with Supabase off', () => {
    config.env = 'development';
    config.supabase = { url: 'https://abc.supabase.co', publishableKey: legacyKey('anon'), secretKey: '', jwtSecret: '', jwtIssuer: '' };
    expect(boot().exit).toBeNull();
    config.supabase = { url: '', publishableKey: '', secretKey: '', jwtSecret: '', jwtIssuer: '' };
    expect(boot()).toEqual({ exit: null, logged: '' });
  });

  it('warns in development and refuses in production when only one of URL / key is set', () => {
    config.env = 'development';
    config.supabase = { url: 'https://abc.supabase.co', publishableKey: '', secretKey: '', jwtSecret: '', jwtIssuer: '' };
    const dev = boot();
    expect(dev.exit).toBeNull();
    expect(dev.logged).toMatch(/OC_SUPABASE_URL and OC_SUPABASE_PUBLISHABLE_KEY/);
    production();
    expect(boot().exit).toBe(1);
  });

  it('in production requires https and the secret key', () => {
    production();
    config.supabase = { url: 'http://abc.supabase.co', publishableKey: 'sb_publishable_x', secretKey: secret, jwtSecret: '', jwtIssuer: '' };
    const r = boot();
    expect(r.exit).toBe(1);
    expect(r.logged).toMatch(/OC_SUPABASE_URL must be https/);
    expect(r.logged).not.toContain(secret);
    config.supabase = { url: 'https://abc.supabase.co', publishableKey: 'sb_publishable_x', secretKey: '', jwtSecret: '', jwtIssuer: '' };
    expect(boot().logged).toMatch(/OC_SUPABASE_SECRET_KEY/);
    config.supabase.secretKey = secret;
    expect(boot().exit).toBeNull();
  });
});
