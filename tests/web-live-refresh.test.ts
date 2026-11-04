import { describe, expect, it } from 'vitest';
import { keepIfSame } from '../apps/web/src/lib/liveRefresh';

describe('live design lists', () => {
  const a = { id: 'a', title: 'Poster', updated_at: '1' };
  const b = { id: 'b', title: 'Card', updated_at: '1' };

  it('keeps the list on screen when the reply is the same', () => {
    const shown = [a, b];
    expect(keepIfSame(shown, [{ ...a }, { ...b }])).toBe(shown);
  });

  it('takes the new list when a design is added, removed, reordered or changed', () => {
    const shown = [a, b];
    const added = [{ id: 'c', title: 'New from Claude', updated_at: '2' }, a, b];
    expect(keepIfSame(shown, added)).toBe(added);
    expect(keepIfSame(shown, [a])).toEqual([a]);
    expect(keepIfSame(shown, [b, a])).toEqual([b, a]);
    expect(keepIfSame(shown, [a, { ...b, title: 'Card v2' }])[1]!.title).toBe('Card v2');
    expect(keepIfSame(null, [a])).toEqual([a]);
  });
});
