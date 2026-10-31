import { describe, expect, it } from 'vitest';
import { filtersFromParams, pushRecent, recentSearches, scopedType, searchParams } from '../apps/web/src/searchApi';

describe('unified search client (g16)', () => {
  it('builds short query strings (defaults left out)', () => {
    expect(searchParams('  logo  ').toString()).toBe('q=logo');
    expect(searchParams('', { type: 'all', owner: 'anyone', modified: 'any' }).toString()).toBe('');
    expect(searchParams('a b', { type: 'designs', owner: 'shared', modified: '7d', surface: 'doc' }, 4).toString())
      .toBe('q=a+b&type=designs&owner=shared&modified=7d&surface=doc&limit=4');
  });

  it('reads filters back from a URL, ignoring unknown values', () => {
    expect(filtersFromParams(new URLSearchParams('type=folders&owner=me&modified=30d&surface=video'))).toEqual({ type: 'folders', owner: 'me', modified: '30d', surface: 'video' });
    expect(filtersFromParams(new URLSearchParams('type=people&owner=x&modified=2d'))).toEqual({ type: 'all', owner: 'anyone', modified: 'any' });
  });

  it('keeps a scoped box inside its scope', () => {
    expect(scopedType('all', 'uploads')).toBe('uploads');
    expect(scopedType('designs', 'uploads')).toBe('designs');
    expect(scopedType('templates', 'all')).toBe('templates');
  });

  it('keeps recent searches newest first, deduplicated and capped', () => {
    let list: string[] = [];
    for (const q of ['logo', 'Poster', '  ', 'LOGO', 'menu  card']) list = pushRecent(list, q, 3);
    expect(list).toEqual(['menu card', 'LOGO', 'Poster']);
  });

  it('survives missing storage', () => {
    expect(recentSearches()).toEqual([]);
  });
});
