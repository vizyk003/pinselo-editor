// SPDX-License-Identifier: Apache-2.0
/**
 * Whiteboard facilitation state shared through the design's Y.Doc (WB-030..): the countdown timer and vote
 * sessions. It lives in its own root map (`board`) outside the element tree, so it syncs in real time to
 * every collaborator, persists with the document, and is not part of undo history.
 *
 *   board: Y.Map { timer: TimerState, vote: VoteSession }
 *   votes: Y.Map<`${session}|${voter}|${element}`, VoteCell>
 *
 * Each voter only writes keys that carry their own id, so concurrent votes from different people never
 * conflict: merging two replicas keeps every voter's dots.
 */
import type { ElementId } from './types/schema';
import * as Y from 'yjs';

export interface TimerState {
  /** Configured length (reset target). */
  durationMs: number;
  /** Running: wall-clock end time (ms since epoch). */
  endsAt: number | null;
  /** Paused / idle: time left. */
  remainingMs: number;
  /** Play a chime for everyone when it reaches zero. */
  sound: boolean;
  /** Who last changed it (display only). */
  by?: string;
}

export interface VoteSession {
  id: string;
  /** Dots each person may place. */
  perPerson: number;
  anonymous: boolean;
  /** Elements that can receive votes. */
  scope: ElementId[];
  status: 'open' | 'ended';
  startedBy: { id: string; name: string };
  startedAt: number;
  endedAt?: number;
  /** "Highlight top 3": elements marked for everyone after the vote. */
  highlight?: ElementId[];
}

export interface VoteCell { n: number; name?: string }
export interface VoteEntry { session: string; voter: string; element: ElementId; n: number; name?: string }

export const BOARD_ORIGIN = Symbol('board');

const boardMap = (ydoc: Y.Doc) => ydoc.getMap<unknown>('board');
const votesMap = (ydoc: Y.Doc) => ydoc.getMap<VoteCell>('votes');

export function readTimer(ydoc: Y.Doc): TimerState | null {
  return (boardMap(ydoc).get('timer') as TimerState | undefined) ?? null;
}

export function writeTimer(ydoc: Y.Doc, timer: TimerState | null): void {
  ydoc.transact(() => {
    if (timer) boardMap(ydoc).set('timer', timer);
    else boardMap(ydoc).delete('timer');
  }, BOARD_ORIGIN);
}

export function readVoteSession(ydoc: Y.Doc): VoteSession | null {
  return (boardMap(ydoc).get('vote') as VoteSession | undefined) ?? null;
}

export function writeVoteSession(ydoc: Y.Doc, session: VoteSession | null): void {
  ydoc.transact(() => {
    if (session) boardMap(ydoc).set('vote', session);
    else boardMap(ydoc).delete('vote');
  }, BOARD_ORIGIN);
}

const voteKey = (session: string, voter: string, element: string) => `${session}|${voter}|${element}`;

/** All dots of a session (every voter, every element). */
export function readVotes(ydoc: Y.Doc, session: string): VoteEntry[] {
  const out: VoteEntry[] = [];
  for (const [k, v] of votesMap(ydoc).entries()) {
    const [s, voter, element] = k.split('|');
    if (s !== session || !voter || !element || !(v.n > 0)) continue;
    out.push({ session: s, voter, element: element as ElementId, n: v.n, ...(v.name ? { name: v.name } : {}) });
  }
  return out;
}

/**
 * Add (`delta` > 0) or remove (< 0) a voter's dots on one element. Returns the voter's new count there.
 * The per-person budget is enforced by the caller (see `voteAllowance` in @oc/editor-core).
 */
export function castVote(ydoc: Y.Doc, session: string, voter: { id: string; name?: string }, element: ElementId, delta: number): number {
  const m = votesMap(ydoc);
  const key = voteKey(session, voter.id, element);
  const cur = m.get(key)?.n ?? 0;
  const n = Math.max(0, cur + delta);
  ydoc.transact(() => {
    if (n === 0) m.delete(key);
    else m.set(key, { n, ...(voter.name ? { name: voter.name } : {}) });
  }, BOARD_ORIGIN);
  return n;
}

/** Drop every dot of a session (after the results are no longer needed). */
export function clearVotes(ydoc: Y.Doc, session?: string): void {
  const m = votesMap(ydoc);
  ydoc.transact(() => {
    for (const k of [...m.keys()]) if (!session || k.startsWith(`${session}|`)) m.delete(k);
  }, BOARD_ORIGIN);
}

/** Subscribe to timer / vote changes (local and remote). */
export function observeBoard(ydoc: Y.Doc, fn: () => void): () => void {
  const b = boardMap(ydoc);
  const v = votesMap(ydoc);
  b.observe(fn);
  v.observe(fn);
  return () => {
    b.unobserve(fn);
    v.unobserve(fn);
  };
}
