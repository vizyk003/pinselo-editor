/** Runtime configuration. Every variable is OC_*-prefixed and has a local-dev default. */
import { BRAND } from './brand';

export { BRAND } from './brand';

const env = (globalThis as { process?: { env: Record<string, string | undefined> } }).process?.env ?? {};

function str(name: string, dflt: string): string {
  return env[name] ?? dflt;
}
function int(name: string, dflt: number): number {
  const v = env[name];
  return v ? Number.parseInt(v, 10) : dflt;
}

export const config = {
  brand: BRAND,
  env: str('NODE_ENV', 'development'),
  databaseUrl: str('OC_DATABASE_URL', 'postgres://localhost:5432/opencanvas'),
  /**
   * CA certificate for the database connection (plan 12 §A1): PEM text, an absolute path, or a path relative to
   * the repo root. When set, the server certificate and hostname are verified (Supabase's CA is not in system stores).
   */
  databaseSslCa: str('OC_DATABASE_SSL_CA', ''),
  /** Connections per process pool (Supabase session pooler: the project's Pool Size is shared by every service). */
  databasePoolMax: int('OC_DATABASE_POOL_MAX', 10),
  /**
   * Optional Supabase Auth (plan 12): on only when url and publishableKey are both set. The publishable key is the
   * only one that ever reaches the browser; secretKey (Auth Admin API) and jwtSecret (legacy HS256) are server-only.
   */
  supabase: {
    url: str('OC_SUPABASE_URL', '').replace(/\/+$/, ''),
    publishableKey: str('OC_SUPABASE_PUBLISHABLE_KEY', ''),
    secretKey: str('OC_SUPABASE_SECRET_KEY', ''),
    jwtSecret: str('OC_SUPABASE_JWT_SECRET', ''),
    /** Override for the expected `iss` claim (custom domains); default `${url}/auth/v1`. */
    jwtIssuer: str('OC_SUPABASE_JWT_ISSUER', ''),
  },
  /** Shared secret for session signing and collab/render tickets. MUST be set in production. */
  secret: str('OC_SECRET', 'dev-only-secret-change-me'),
  webOrigin: str('OC_WEB_ORIGIN', 'http://localhost:3000'),
  /**
   * Origin that serves user uploads (PL-008). Must be a different registrable domain from webOrigin in
   * production (e.g. https://usercontent.example.net) so uploaded content can never run with app cookies.
   * Empty = same origin (local dev).
   */
  usercontentOrigin: str('OC_USERCONTENT_ORIGIN', ''),
  apiPort: int('OC_API_PORT', 4000),
  collabPort: int('OC_COLLAB_PORT', 4100),
  renderPort: int('OC_RENDER_PORT', 4200),
  renderUrl: str('OC_RENDER_URL', 'http://localhost:4200'),
  apiUrl: str('OC_API_URL', 'http://localhost:4000'),
  dataDir: str('OC_DATA_DIR', ''),
  /** Upload quota per user in bytes (Free plan: 5 GB). */
  uploadQuota: int('OC_UPLOAD_QUOTA', 5 * 1024 ** 3),
  maxUploadBytes: int('OC_MAX_UPLOAD_BYTES', 50 * 1024 ** 2),
  maxMediaBytes: int('OC_MAX_MEDIA_BYTES', 1024 ** 3),
};

export type Config = typeof config;

/** Supabase Auth is optional: enabled when both the project URL and the publishable key are set. Checked per call. */
export function supabaseAuthEnabled(): boolean {
  return Boolean(config.supabase.url && config.supabase.publishableKey);
}

/** The `iss` claim Supabase access tokens must carry. */
export function supabaseIssuer(): string {
  return config.supabase.jwtIssuer || `${config.supabase.url.replace(/\/+$/, '')}/auth/v1`;
}

/** The `role` claim of a JWT-shaped key (legacy anon/service_role keys), or null. Never verifies anything. */
function jwtRole(key: string): string | null {
  const part = key.split('.')[1];
  if (key.split('.').length !== 3 || !part) return null;
  try {
    const json = atob(part.replace(/-/g, '+').replace(/_/g, '/').padEnd(Math.ceil(part.length / 4) * 4, '='));
    const role = (JSON.parse(json) as { role?: unknown }).role;
    return typeof role === 'string' ? role : null;
  } catch {
    return null;
  }
}

/** Refuse to boot a production service with development secrets or origins. Messages name variables, never values. */
export function assertProductionConfig(): void {
  const sb = config.supabase;
  // Served to browsers, so a secret key here is refused in every environment.
  const pk = sb.publishableKey;
  if (pk && (pk.startsWith('sb_secret_') || jwtRole(pk) === 'service_role')) {
    console.error('Refusing to start: OC_SUPABASE_PUBLISHABLE_KEY holds a secret (service_role) key. Use the publishable (anon) key; it is sent to browsers.');
    process.exit(1);
  }
  const half = Boolean(sb.url) !== Boolean(pk);
  if (config.env !== 'production') {
    if (half) console.warn('Supabase Auth is off: set both OC_SUPABASE_URL and OC_SUPABASE_PUBLISHABLE_KEY (only one is set).');
    return;
  }
  const problems: string[] = [];
  if (config.secret === 'dev-only-secret-change-me' || config.secret.length < 32) problems.push('OC_SECRET must be a random value of 32+ characters');
  if (!config.webOrigin.startsWith('https://')) problems.push('OC_WEB_ORIGIN must be https');
  if (config.usercontentOrigin && new URL(config.usercontentOrigin).hostname.endsWith(new URL(config.webOrigin).hostname)) {
    problems.push('OC_USERCONTENT_ORIGIN must not be a subdomain of the app origin');
  }
  if (half) problems.push('OC_SUPABASE_URL and OC_SUPABASE_PUBLISHABLE_KEY must be set together (or both left empty for the built-in sign-in)');
  if (supabaseAuthEnabled()) {
    if (!sb.url.startsWith('https://')) problems.push('OC_SUPABASE_URL must be https');
    if (!sb.secretKey) problems.push('OC_SUPABASE_SECRET_KEY must be set when Supabase Auth is on (account deletion removes the Supabase user)');
  }
  if (problems.length) {
    console.error(`Refusing to start in production:\n - ${problems.join('\n - ')}`);
    process.exit(1);
  }
}
