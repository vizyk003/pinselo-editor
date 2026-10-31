/**
 * Pure rules for this browser's Supabase session (plan 12 §C; review C1, C2, C3, C6): whose session it is, the effect
 * key that ignores token refreshes, a bounded wait, and steps that resume after a failure. No DOM or supabase-js here,
 * so it unit-tests in node.
 */

export interface SessionUser { id: string; email?: string | null }
/**
 * The signed-in Pinselo account. `supabaseId`: its linked Supabase user (GET /api/account → auth), null when not
 * linked. `supabaseEmail`: that user's address at Supabase as the server read it, null when unknown.
 */
export interface SessionAccount { email: string; supabaseId?: string | null; supabaseEmail?: string | null }

/**
 * Whether a Supabase session may act for this account (review C1). A linked account compares the Supabase user id,
 * because the two emails can differ (an email change that another account's address blocked here). Not linked yet,
 * or a server that doesn't send the id: the same email, which is also what `sync` links by.
 */
export function isOwnSession(u: SessionUser, a: SessionAccount): boolean {
  if (a.supabaseId) return u.id === a.supabaseId;
  return Boolean(u.email) && u.email!.toLowerCase() === a.email.toLowerCase();
}

/**
 * The email the re-auth panel starts with (review C1): the address this account signs in with at Supabase. Its own
 * session's address first; without one, the linked user's address from the server (F2), else the address here.
 */
export function reauthEmail(s: { user: SessionUser } | null | undefined, a: SessionAccount): string {
  if (s?.user.email && isOwnSession(s.user, a)) return s.user.email;
  return (a.supabaseId && a.supabaseEmail) || a.email;
}

/**
 * Effect key for work that depends on whose session this is (review C2). Not the access token: listing factors can
 * refresh it, and an effect keyed on the token would then run again, and again.
 */
export const sessionKey = (s: { user: { id: string } } | null | undefined): string => (s === undefined ? 'loading' : s ? `user:${s.user.id}` : 'none');

/** Waits for `p` at most `ms`: true when it fulfilled in time, false when it rejected or took longer (review C3). */
export function settledWithin(p: Promise<unknown>, ms: number): Promise<boolean> {
  return new Promise((resolve) => {
    const timer = setTimeout(() => resolve(false), ms);
    p.then(
      () => {
        clearTimeout(timer);
        resolve(true);
      },
      () => {
        clearTimeout(timer);
        resolve(false);
      },
    );
  });
}

/**
 * Runs the steps whose names aren't in `done` yet, in order, adding each name once it succeeds (review C6). A step
 * that returns false stops the run without being marked (e.g. a second factor is needed first). A retry after a
 * failure resumes at the step that failed. Resolves true when every step is done.
 */
export async function resumeSteps(done: Set<string>, steps: ReadonlyArray<readonly [string, () => Promise<boolean | void>]>): Promise<boolean> {
  for (const [name, run] of steps) {
    if (done.has(name)) continue;
    if ((await run()) === false) return false;
    done.add(name);
  }
  return true;
}
