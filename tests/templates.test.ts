import { describe, expect, it } from 'vitest';
import {
  groupByCategory, parseTemplateFilters, recentTemplates, recommendTemplates, recordRecentTemplate, RECENT_KEY, RECENT_MAX, recentTemplatesKey,
  templateFiltersToParams, templateSurface, toggleTag, type KV,
} from '../apps/web/src/lib/templates';
import type { TemplateSummary } from '../apps/web/src/api';

const tpl = (id: string, o: Partial<TemplateSummary> = {}): TemplateSummary => ({ id, title: id, category: 'Social', preset: 'ig-post', tags: [], width: 1080, height: 1080, ...o });

function memory(): KV & { data: Map<string, string> } {
  const data = new Map<string, string>();
  return { data, getItem: (k) => data.get(k) ?? null, setItem: (k, v) => void data.set(k, v) };
}

describe('recently used templates', () => {
  it('moves a template to the front without duplicates and caps the list', () => {
    const s = memory();
    recordRecentTemplate('u1', tpl('a'), s);
    recordRecentTemplate('u1', tpl('b'), s);
    expect(recordRecentTemplate('u1', tpl('a'), s).map((x) => x.id)).toEqual(['a', 'b']);
    for (let i = 0; i < 20; i++) recordRecentTemplate('u1', tpl(`t${i}`), s);
    expect(recentTemplates('u1', s)).toHaveLength(RECENT_MAX);
    expect(recentTemplates('u1', s)[0]!.id).toBe('t19');
  });

  it('keeps one list per account and none signed out', () => {
    const s = memory();
    recordRecentTemplate('u1', tpl('private', { team: true }), s);
    expect(recentTemplates('u2', s)).toEqual([]);
    expect(recentTemplates(null, s)).toEqual([]);
    expect(recordRecentTemplate(null, tpl('x'), s)).toEqual([]);
    expect(s.data.has(RECENT_KEY)).toBe(false);
    expect(s.data.get(recentTemplatesKey('u1')!)).toContain('private');
  });

  it('survives corrupt or blocked storage', () => {
    const s = memory();
    s.data.set(recentTemplatesKey('u1')!, '{nope');
    expect(recentTemplates('u1', s)).toEqual([]);
    expect(recentTemplates('u1', null)).toEqual([]);
    const throwing: KV = { getItem: () => { throw new Error('blocked'); }, setItem: () => { throw new Error('blocked'); } };
    expect(recordRecentTemplate('u1', tpl('a'), throwing).map((x) => x.id)).toEqual(['a']);
  });
});

describe('recommendTemplates', () => {
  const list = [
    tpl('slide', { preset: 'presentation', width: 1920, height: 1080, category: 'Presentations' }),
    tpl('post', { category: 'Social' }),
    tpl('post-sale', { category: 'Sale' }),
    tpl('poster', { preset: 'poster', width: 1728, height: 2304, category: 'Print' }),
  ];
  it('prefers the same surface and the source template category', () => {
    const r = recommendTemplates(list, { surface: 'graphic', w: 1080, h: 1080, category: 'Sale' });
    expect(r.map((x) => x.id)).toEqual(['post-sale', 'post']);
  });
  it('excludes templates and respects the limit', () => {
    const r = recommendTemplates(list, { surface: 'presentation', w: 1920, h: 1080 }, { exclude: ['slide'], limit: 1 });
    expect(r.every((x) => x.id !== 'slide')).toBe(true);
    expect(r.length).toBeLessThanOrEqual(1);
  });
  it('maps presets to surfaces', () => {
    expect(templateSurface({ preset: 'presentation' })).toBe('presentation');
    expect(templateSurface({ preset: 'unknown' })).toBeNull();
  });
});

describe('template browse filters', () => {
  it('round-trips through the query string', () => {
    const f = parseTemplateFilters('?q=sale&category=Social&tag=bold,%20minimal,bold&view=team&t=x1');
    expect(f).toEqual({ q: 'sale', category: 'Social', tags: ['bold', 'minimal'], view: 'team', preview: 'x1' });
    expect(parseTemplateFilters(templateFiltersToParams(f))).toEqual(f);
    expect(templateFiltersToParams({ view: 'all', tags: [] }).toString()).toBe('');
    expect(parseTemplateFilters('view=bogus').view).toBe('all');
  });
  it('toggles tags', () => {
    expect(toggleTag(['a'], 'b')).toEqual(['a', 'b']);
    expect(toggleTag(['a', 'b'], 'a')).toEqual(['b']);
  });
  it('groups by category, largest first', () => {
    const g = groupByCategory([tpl('1', { category: 'B' }), tpl('2', { category: 'A' }), tpl('3', { category: 'B' })], 1);
    expect(g.map((x) => [x.category, x.n, x.templates.length])).toEqual([['B', 2, 1], ['A', 1, 1]]);
  });
});
