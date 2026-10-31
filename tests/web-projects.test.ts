import { afterEach, describe, expect, it, vi } from 'vitest';
import { designParams } from '../apps/web/src/api/projects';
import {
  canMoveFolderTo, childFolders, DEFAULT_LIST, filterFolders, folderPath, listStateFromParams, listStateToParams, loadViewMode, matchesQuery,
  saveViewMode, sortFolders, splitSelection, subtreeIds, toggleSelection, type TreeFolder,
} from '../apps/web/src/pages/home/projectsModel';

const f = (id: string, parent: string | null, extra: Partial<TreeFolder> = {}): TreeFolder => ({ id, name: id, parent_id: parent, workspace_id: 'p', ...extra });
// a ─ b ─ c      t (team)      orphan (parent not visible)
const tree = [f('a', null, { created_at: '2026-01-02', owner_id: 'me' }), f('b', 'a', { created_at: '2026-01-03', owner_id: 'me' }), f('c', 'b'), f('t', null, { workspace_id: 'team', created_at: '2026-01-01', owner_id: 'x' }), f('orphan', 'gone')];

describe('projects folder tree (g17)', () => {
  it('lists one level, keeping folders with an invisible parent at the top', () => {
    expect(childFolders(tree, null).map((x) => x.id)).toEqual(['a', 't', 'orphan']);
    expect(childFolders(tree, 'a').map((x) => x.id)).toEqual(['b']);
  });

  it('builds breadcrumbs from the top level down, and survives cycles', () => {
    expect(folderPath(tree, 'c').map((x) => x.id)).toEqual(['a', 'b', 'c']);
    expect(folderPath(tree, null)).toEqual([]);
    expect(folderPath(tree, 'missing')).toEqual([]);
    const loop = [f('x', 'y'), f('y', 'x')];
    expect(folderPath(loop, 'x').map((v) => v.id)).toEqual(['y', 'x']);
  });

  it('never moves a folder into itself, below itself or to another workspace', () => {
    expect([...subtreeIds(tree, 'a')].sort()).toEqual(['a', 'b', 'c']);
    expect(canMoveFolderTo(tree, 'a', 'a')).toBe(false);
    expect(canMoveFolderTo(tree, 'a', 'c')).toBe(false);
    expect(canMoveFolderTo(tree, 'b', 't')).toBe(false);
    expect(canMoveFolderTo(tree, 'c', 'a')).toBe(true);
    expect(canMoveFolderTo(tree, 'c', null)).toBe(true);
    expect(canMoveFolderTo(tree, 'nope', null)).toBe(false);
  });

  it('sorts folders by name or creation date', () => {
    const names = (s: Parameters<typeof sortFolders>[1]) => sortFolders([f('Beta', null, { created_at: '2' }), f('alpha', null, { created_at: '3' }), f('Folder 10', null, { created_at: '1' }), f('Folder 9', null, { created_at: '1' })], s).map((x) => x.name);
    expect(names('az')).toEqual(['alpha', 'Beta', 'Folder 9', 'Folder 10']);
    expect(names('za')).toEqual(['Folder 10', 'Folder 9', 'Beta', 'alpha']);
    expect(names('edited')).toEqual(['alpha', 'Beta', 'Folder 9', 'Folder 10']);
    expect(names('oldest')).toEqual(['Folder 9', 'Folder 10', 'Beta', 'alpha']);
  });

  it('filters folders by search words, type and owner', () => {
    expect(matchesQuery('Client Acme 2026', 'acme client')).toBe(true);
    expect(matchesQuery('Client Acme', 'acme beta')).toBe(false);
    expect(filterFolders(tree, { ...DEFAULT_LIST, q: 'B' }, 'me').map((x) => x.id)).toEqual(['b']);
    expect(filterFolders(tree, { ...DEFAULT_LIST, owner: 'me' }, 'me').map((x) => x.id)).toEqual(['a', 'b']);
    expect(filterFolders(tree, { ...DEFAULT_LIST, owner: 'shared' }, 'me').map((x) => x.id)).toEqual(['c', 't', 'orphan']);
    expect(filterFolders(tree, { ...DEFAULT_LIST, type: 'video' }, 'me')).toEqual([]);
    expect(filterFolders(tree, { ...DEFAULT_LIST, type: 'folder' }, 'me')).toHaveLength(5);
  });
});

describe('projects selection (g17)', () => {
  const order = ['f:a', 'd:1', 'd:2', 'd:3', 'd:4'];
  it('toggles single items', () => {
    const one = toggleSelection(order, new Set(), 'd:2', { shift: false, anchor: null });
    expect([...one]).toEqual(['d:2']);
    expect([...toggleSelection(order, one, 'd:2', { shift: false, anchor: 'd:2' })]).toEqual([]);
  });

  it('selects shift-click ranges in either direction and keeps the rest', () => {
    const start = new Set(['f:a']);
    expect([...toggleSelection(order, start, 'd:3', { shift: true, anchor: 'd:1' })].sort()).toEqual(['d:1', 'd:2', 'd:3', 'f:a']);
    expect([...toggleSelection(order, new Set(), 'd:1', { shift: true, anchor: 'd:4' })].sort()).toEqual(['d:1', 'd:2', 'd:3', 'd:4']);
    // No anchor yet: a shift-click is a plain toggle.
    expect([...toggleSelection(order, new Set(), 'd:4', { shift: true, anchor: null })]).toEqual(['d:4']);
  });

  it('splits selected keys into designs and folders', () => {
    expect(splitSelection(['f:a', 'd:1', 'd:x:y'])).toEqual({ designs: ['1', 'x:y'], folders: ['a'] });
  });
});

describe('projects list state (g17/g20)', () => {
  it('reads filters and sort from the URL, ignoring unknown values', () => {
    expect(listStateFromParams(new URLSearchParams('q=logo&sort=az&type=video&owner=me'))).toEqual({ q: 'logo', sort: 'az', type: 'video', owner: 'me' });
    expect(listStateFromParams(new URLSearchParams('sort=sideways&type=%3Cx%3E&owner=them'))).toEqual(DEFAULT_LIST);
  });

  it('writes short URLs and keeps unrelated parameters', () => {
    expect(listStateToParams(DEFAULT_LIST).toString()).toBe('');
    expect(listStateToParams({ q: 'a b', sort: 'za', type: 'folder', owner: 'shared' }, new URLSearchParams('x=1&sort=az')).toString()).toBe('x=1&sort=za&q=a+b&type=folder&owner=shared');
  });

  it('builds the design list query', () => {
    expect(designParams({ filter: 'folder' }).toString()).toBe('filter=folder');
    expect(designParams({ filter: 'folder', folder: 'f1', q: '  hi ', surface: 'doc', sort: 'oldest', owner: 'me', deep: true }).toString())
      .toBe('filter=folder&folder=f1&q=hi&surface=doc&sort=oldest&owner=me&deep=1');
    expect(designParams({ filter: 'starred', sort: 'edited', owner: 'anyone', deep: false }).toString()).toBe('filter=starred');
  });
});

describe('projects view preference (g17)', () => {
  afterEach(() => vi.unstubAllGlobals());

  it('remembers grid or list per user', () => {
    const store = new Map<string, string>();
    vi.stubGlobal('localStorage', { getItem: (k: string) => store.get(k) ?? null, setItem: (k: string, v: string) => void store.set(k, v) });
    expect(loadViewMode('u1')).toBe('grid');
    saveViewMode('u1', 'list');
    expect(loadViewMode('u1')).toBe('list');
    expect(loadViewMode('u2')).toBe('grid');
  });

  it('falls back to grid when storage is blocked', () => {
    vi.stubGlobal('localStorage', { getItem: () => { throw new Error('blocked'); }, setItem: () => { throw new Error('blocked'); } });
    expect(() => saveViewMode('u1', 'list')).not.toThrow();
    expect(loadViewMode('u1')).toBe('grid');
  });
});
