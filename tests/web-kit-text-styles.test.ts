import { describe, expect, it } from 'vitest';
import type { TextEl } from '@oc/engine';
import { cleanStyle, describeStyle, elementRole, kitRole, runStyleOf, styleFor, styleFromText } from '../apps/web/src/editor/kitTextStyles';

const kit = {
  fonts: { heading: 'Canter', body: 'Hanken Grotesk', caption: 'Hanken Grotesk' },
  textStyles: [
    { name: 'Heading', role: 'heading' as const, weight: 700, colors: ['#943429'] },
    { name: 'Eyebrow', role: 'caption' as const, weight: 500, uppercase: true, letterSpacing: 140, size: 12.75 },
  ],
};

function textEl(over: Partial<TextEl['body']['defaultRun']>, text: string, role?: TextEl['role'], lineHeight = 1.3): TextEl {
  return {
    type: 'text', role,
    body: {
      defaultRun: { family: 'Hanken Grotesk', weight: 400, italic: false, size: 14, fill: { k: 'solid', color: { r: 107, g: 107, b: 102, a: 1 } }, tracking: 0, ...over },
      paragraphs: [{ runs: [{ text, style: {} }], align: 'start', lineHeight, spaceAfter: 0 }],
    },
  } as unknown as TextEl;
}

describe('brand kit text styles', () => {
  it('maps kit roles to element roles and back', () => {
    expect(elementRole('heading')).toBe('headline');
    expect(elementRole('subheading')).toBe('subhead');
    expect(kitRole('caption')).toBe('caption');
    expect(kitRole('legal')).toBe('body');
    expect(kitRole('cta')).toBeNull();
  });

  it('gives new text the role font and the style', () => {
    const run = runStyleOf(kit, kit.textStyles[1]!, (h) => ({ k: 'solid', color: h }) as never);
    expect(run).toEqual({ family: 'Hanken Grotesk', weight: 500, italic: false, caseTransform: 'upper', tracking: 140, size: 12.75 });
    const heading = runStyleOf(kit, kit.textStyles[0]!, (h) => ({ k: 'solid', color: h }) as never);
    expect(heading.family).toBe('Canter');
    expect(heading.fill).toEqual({ k: 'solid', color: '#943429' });
    expect(styleFor(kit, 'caption')?.name).toBe('Eyebrow');
    expect(styleFor(kit, 'quote')).toBeUndefined();
  });

  it('reads a style back from a text box, counting typed capitals as uppercase', () => {
    const el = textEl({ weight: 500, tracking: 140 }, 'PÁDUAI SZENT ANTAL PLÉBÁNIA', 'caption', 1.4);
    expect(styleFromText(el, 'Eyebrow', 'body', () => '#6B6B66')).toEqual({
      name: 'Eyebrow', role: 'caption', weight: 500, uppercase: true, letterSpacing: 140, lineHeight: 1.4, size: 14, colors: ['#6b6b66'],
    });
    const plain = styleFromText(textEl({ italic: true }, 'Fotó: Ferenc'), 'Caption', 'caption', () => '#404041');
    expect(plain).toMatchObject({ role: 'caption', italic: true });
    expect(plain.uppercase).toBeUndefined();
    expect(plain.lineHeight).toBeUndefined();
  });

  it('cleans a style for saving and describes it', () => {
    const st = cleanStyle({ name: '  Eyebrow ', role: 'caption', weight: 530, italic: false, letterSpacing: 140.4, colors: ['#6B6B66', 'red', '#6b6b66'], note: ' ' });
    expect(st).toEqual({ name: 'Eyebrow', role: 'caption', weight: 500, letterSpacing: 140, colors: ['#6b6b66'] });
    expect(describeStyle({ ...st, uppercase: true, size: 13 }, { italic: 'dőlt', caps: 'VERZÁL' })).toBe('500 · VERZÁL · +140‰ · 13px');
  });
});
