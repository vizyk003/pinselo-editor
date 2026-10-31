/** The part of the collab provider's awareness that live comments use (y-protocols `Awareness`). */
export interface CommentsAwareness {
  clientID: number;
  getStates(): Map<number, Record<string, unknown>>;
  getLocalState(): Record<string, unknown> | null;
  setLocalStateField(field: string, value: unknown): void;
  on(event: 'change', cb: () => void): void;
  off(event: 'change', cb: () => void): void;
}

/**
 * Awareness field a client bumps after it changes a comment (post, reply, edit, react, resolve, delete). Like
 * presence, it travels over the collab room's awareness, so the other people in the design reload the comments
 * right away instead of on the next 15-second poll (COL-01).
 */
export const COMMENTS_REV = 'commentsRev';

/** Peers whose comments revision changed since the last call (peers seen for the first time only record theirs). */
export function commentsChangedBy(aw: Pick<CommentsAwareness, 'clientID' | 'getStates'>, seen: Map<number, unknown>): boolean {
  let changed = false;
  aw.getStates().forEach((s, id) => {
    if (id === aw.clientID) return;
    const rev = s[COMMENTS_REV];
    if (seen.has(id) && seen.get(id) !== rev) changed = true;
    seen.set(id, rev);
  });
  return changed;
}
