import { describe, expect, it } from 'vitest';
import { mergeSections, splitMarkdown, totalChars } from '../apps/web/src/editor/guidelinesMarkdown';

describe('brand guidelines from Markdown', () => {
  it('splits a brand book at level-2 headings, with the intro under the title', () => {
    const md = '# Pasaréti Ferencesek\n\nThe parish brand.\n\n## Hangnem\n\n- Hungarian.\n\n## Szín\n\nOne accent.\n';
    expect(splitMarkdown('README.md', md)).toEqual([
      { title: 'Pasaréti Ferencesek', body: 'The parish brand.' },
      { title: 'Hangnem', body: '- Hungarian.' },
      { title: 'Szín', body: 'One accent.' },
    ]);
  });

  it('names a file without a title after the file, and drops an empty intro', () => {
    expect(splitMarkdown('logo-rules.md', 'Never recolour it.')).toEqual([{ title: 'logo rules', body: 'Never recolour it.' }]);
    expect(splitMarkdown('x.md', '# T\n\n## Only\nbody')).toEqual([{ title: 'Only', body: 'body' }]);
  });

  it('ignores headings inside code fences and keeps deeper headings in the body', () => {
    const md = '## Code\n```md\n## not a section\n```\n### Detail\ntext';
    const s = splitMarkdown('a.md', md);
    expect(s.map((x) => x.title)).toEqual(['Code']);
    expect(s[0]!.body).toContain('## not a section');
    expect(s[0]!.body).toContain('### Detail');
  });

  it('replaces sections with the same title on import and appends new ones', () => {
    const cur = [{ title: 'Colour', body: 'old' }, { title: 'Voice', body: 'v' }];
    expect(mergeSections(cur, [{ title: 'colour', body: 'new' }, { title: 'Logo', body: 'l' }])).toEqual([
      { title: 'colour', body: 'new' }, { title: 'Voice', body: 'v' }, { title: 'Logo', body: 'l' },
    ]);
    expect(totalChars([{ title: 'a', body: 'ő€' }])).toBe(2);
  });
});
