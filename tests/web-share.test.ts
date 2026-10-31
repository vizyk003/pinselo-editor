import { describe, expect, it } from 'vitest';
import { embedSnippet, looksLikeEmail, previewSlug, splitTyped, templateLinkUrl, viewUrl } from '../apps/web/src/editor/shareTools';

describe('share dialog helpers', () => {
  it('uses the custom slug for the public link when there is one', () => {
    expect(viewUrl('https://app.example.com', '7JnEpHSuGjKGgRvE', null)).toBe('https://app.example.com/view/7JnEpHSuGjKGgRvE');
    expect(viewUrl('https://app.example.com', '7JnEpHSuGjKGgRvE', 'summer-sale')).toBe('https://app.example.com/view/summer-sale');
    expect(templateLinkUrl('https://app.example.com', 'abc_DEF-123')).toBe('https://app.example.com/t/abc_DEF-123');
  });
  it('builds a responsive, escaped embed snippet pointing at the embed viewer', () => {
    const code = embedSnippet('https://app.example.com/view/x', 1920, 1080, 'Fish & "chips"');
    expect(code).toContain('padding-top:56.25%');
    expect(code).toContain('src="https://app.example.com/view/x?embed=1"');
    expect(code).toContain('title="Fish &amp; &quot;chips&quot;"');
    expect(embedSnippet('https://a/view/x?lang=de', 100, 100, 't')).toContain('src="https://a/view/x?lang=de&amp;embed=1"');
    expect(embedSnippet('https://a/view/x', 0, 0, 't')).toContain('padding-top:56.25%');
  });
  it('turns typed or pasted addresses into chips and keeps the rest as search text', () => {
    expect(looksLikeEmail('ana@example.com')).toBe(true);
    expect(looksLikeEmail('ana@example')).toBe(false);
    expect(splitTyped('Ana@Example.com, bo@x.io; design team')).toEqual({ emails: ['ana@example.com', 'bo@x.io'], rest: 'design team' });
    expect(splitTyped('ana')).toEqual({ emails: [], rest: 'ana' });
  });
  it('previews the custom link the way the server will store it', () => {
    expect(previewSlug('  Summer Sale!! 2026')).toBe('summer-sale-2026');
    expect(previewSlug('-lead')).toBe('lead');
    expect(previewSlug('a'.repeat(80))).toHaveLength(60);
  });
});
