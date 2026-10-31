/** Live comments (QA COL-01): peers reload when another client bumps its comments revision in awareness. */
import { describe, expect, it } from 'vitest';
import { COMMENTS_REV, commentsChangedBy } from '../apps/web/src/editor/liveComments';

const aw = (states: Array<[number, Record<string, unknown>]>, clientID = 1) => ({ clientID, getStates: () => new Map(states) });

describe('live comments (COL-01)', () => {
  it('reports a change only when a known peer bumps its revision', () => {
    const seen = new Map<number, unknown>();
    // First sight of peers records their revision without a reload.
    expect(commentsChangedBy(aw([[1, {}], [2, { user: {} }], [3, { [COMMENTS_REV]: 4 }]]), seen)).toBe(false);
    // Presence-only changes (cursor, selection) are not comment changes.
    expect(commentsChangedBy(aw([[1, {}], [2, { cursor: [1, 2] }], [3, { [COMMENTS_REV]: 4 }]]), seen)).toBe(false);
    // Peer 2 posts its first comment, peer 3 its fifth.
    expect(commentsChangedBy(aw([[1, {}], [2, { [COMMENTS_REV]: 1 }], [3, { [COMMENTS_REV]: 4 }]]), seen)).toBe(true);
    expect(commentsChangedBy(aw([[1, {}], [2, { [COMMENTS_REV]: 1 }], [3, { [COMMENTS_REV]: 5 }]]), seen)).toBe(true);
    expect(commentsChangedBy(aw([[1, {}], [2, { [COMMENTS_REV]: 1 }], [3, { [COMMENTS_REV]: 5 }]]), seen)).toBe(false);
  });

  it('ignores this client’s own revision and new joiners', () => {
    const seen = new Map<number, unknown>();
    commentsChangedBy(aw([[1, { [COMMENTS_REV]: 1 }]]), seen);
    expect(commentsChangedBy(aw([[1, { [COMMENTS_REV]: 2 }]]), seen)).toBe(false);
    expect(commentsChangedBy(aw([[1, { [COMMENTS_REV]: 2 }], [9, { [COMMENTS_REV]: 7 }]]), seen)).toBe(false);
  });
});
