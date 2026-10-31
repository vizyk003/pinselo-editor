/**
 * Templates: Link in bio (1080 × 1920, surface 'website'). One mobile-first page: avatar or photo,
 * name, a short bio and a column of buttons that link out. Publishes like any other website.
 */
import type { Element } from '@oc/engine';
import { B, lin, tpl } from './builder';

const CAT = 'Websites';
const last = (b: B): Element => Object.values(b.doc.elements).at(-1)!;
const link = (b: B, url: string) => { last(b).link = { kind: 'url', url }; return b; };

interface BtnStyle { fill: string; color: string; family: string; weight?: number; size?: number; r?: number; stroke?: [string, number]; shadow?: [number, number, number, string]; tracking?: number }
/** A full-width link button: a linked shape with its label centred on top. */
function button(b: B, x: number, y: number, w: number, h: number, label: string, url: string, s: BtnStyle) {
  b.rect(x, y, w, h, s.fill, { r: s.r ?? h / 2, ...(s.stroke ? { stroke: s.stroke } : {}), ...(s.shadow ? { shadow: s.shadow } : {}) });
  link(b, url);
  last(b).name = label;
  last(b).altText = label;
  const size = s.size ?? 38;
  b.text(label, x + 40, y + (h - size * 1.3) / 2, w - 80, { family: s.family, weight: s.weight ?? 600, size, color: s.color, ...(s.tracking ? { tracking: s.tracking } : {}) }, 'center', { role: 'cta' });
  return b;
}
/** Stack of buttons from y with a fixed gap. Returns the y after the last button. */
function buttons(b: B, x: number, y: number, w: number, h: number, gap: number, items: Array<[string, string]>, s: BtnStyle) {
  for (const [label, url] of items) { button(b, x, y, w, h, label, url, s); y += h + gap; }
  return y;
}

// ------------------------------------------------------------------ 1. Creator (warm, editorial)
tpl('lib-creator-terracotta', 'Creator Link in Bio', CAT, 'link-in-bio', ['link in bio', 'creator', 'influencer', 'warm', 'profile'], (b) => {
  b.bg(lin(180, '#f6e7da', '#efd3bf'));
  b.circle(540, 380, 196, '#c2410c', { opacity: 0.18 });
  b.photo('ff3692f6-3136-42d1-a73e-e50efae59c1b', 364, 204, 352, 352, { clip: 'ellipse', focus: [0.5, 0.25] });
  last(b).altText = 'Portrait of Maya Ortega';
  b.text('Maya Ortega', 90, 610, 900, { family: 'Fraunces', weight: 600, size: 76, color: '#3b1d10' }, 'center', { role: 'headline', lh: 1.1 });
  b.text('Slow living, ceramics & weekend recipes from my studio in Valencia.', 150, 718, 780, { family: 'Inter Tight', weight: 400, size: 34, color: '#6b3a22' }, 'center', { role: 'body', lh: 1.4 });
  const y = buttons(b, 120, 900, 840, 128, 38, [
    ['New: Spring ceramics drop', 'https://example.com/shop'],
    ['Watch the studio vlog', 'https://example.com/vlog'],
    ['My weekly newsletter', 'https://example.com/newsletter'],
    ['Book a pottery workshop', 'https://example.com/workshops'],
  ], { fill: '#fffaf5', color: '#3b1d10', family: 'Inter Tight', weight: 600, size: 36, shadow: [0, 8, 24, '#7c2d1222'] });
  button(b, 120, y + 10, 840, 128, 'Collaborations → hello@example.com', 'mailto:hello@example.com', { fill: '#c2410c', color: '#ffffff', family: 'Inter Tight', weight: 700, size: 34 });
  b.text('© 2026 Maya Ortega Studio', 90, 1800, 900, { family: 'Inter Tight', weight: 500, size: 26, color: '#7a4a33' }, 'center', { role: 'legal' });
});

// ------------------------------------------------------------------ 2. Fitness coach (bold, dark photo)
tpl('lib-fitness-coach', 'Fitness Coach Links', CAT, 'link-in-bio', ['link in bio', 'fitness', 'coach', 'gym', 'bold'], (b) => {
  b.bg('#0b0b0c');
  b.photo('81471031-0862-4b68-a028-0631b190c4a9', 0, 0, 1080, 900, { focus: [0.6, 0.4], adjust: { brightness: -0.1, contrast: 0.1 } });
  last(b).altText = 'Athlete training in the gym';
  b.rect(0, 520, 1080, 400, lin(90, '#0b0b0c00', '#0b0b0c'));
  b.text('TRAIN WITH', 90, 620, 900, { family: 'Archivo', weight: 800, size: 34, color: '#d9f99d', tracking: 300 }, 'start');
  b.text('JADE KOWALSKI', 84, 670, 920, { family: 'Anton', weight: 400, size: 128, color: '#ffffff' }, 'start', { role: 'headline', lh: 1.0 });
  b.text('Strength coach · 12-week programs · Online & in Leeds', 90, 822, 900, { family: 'Archivo', weight: 500, size: 32, color: '#d4d4d8' }, 'start', { role: 'body' });
  const y = buttons(b, 90, 960, 900, 128, 28, [
    ['START THE 12-WEEK PROGRAM', 'https://example.com/program'],
    ['FREE WORKOUT PLANNER', 'https://example.com/planner'],
    ['1:1 COACHING — APPLY', 'https://example.com/apply'],
  ], { fill: '#d9f99d', color: '#0b0b0c', family: 'Archivo', weight: 800, size: 36, r: 18, tracking: 40 });
  buttons(b, 90, y, 900, 128, 28, [
    ['Client results', 'https://example.com/results'],
    ['Training playlist', 'https://example.com/playlist'],
  ], { fill: '#0b0b0c', color: '#ffffff', family: 'Archivo', weight: 700, size: 34, r: 18, stroke: ['#3f3f46', 3] });
  b.text('NEXT INTAKE: 5 JANUARY 2027', 90, 1790, 900, { family: 'Archivo', weight: 700, size: 28, color: '#a1a1aa', tracking: 200 }, 'center', { role: 'caption' });
});

// ------------------------------------------------------------------ 3. Florist (soft, elegant)
tpl('lib-florist-blush', 'Florist Link in Bio', CAT, 'link-in-bio', ['link in bio', 'florist', 'flowers', 'shop', 'elegant'], (b) => {
  b.bg('#fbf3f1');
  b.photo('832fea9c-9abb-4e20-a17e-0df4b07ef3a3', 90, 90, 900, 640, { r: 36, focus: [0.5, 0.75] });
  last(b).altText = 'Fresh flowers in small glass vases';
  b.emoji('🌸', 470, 676, 140);
  b.text('Petal & Thread', 90, 840, 900, { family: 'Cormorant Garamond', weight: 600, italic: true, size: 96, color: '#5b2333' }, 'center', { role: 'headline', lh: 1.05 });
  b.text('SEASONAL FLOWERS · BROOKLYN · SAME-DAY DELIVERY', 90, 956, 900, { family: 'Hanken Grotesk', weight: 600, size: 26, color: '#8a4b5c', tracking: 180 }, 'center', { role: 'subhead' });
  buttons(b, 140, 1046, 800, 116, 30, [
    ['Order a bouquet', 'https://example.com/order'],
    ['Wedding & event florals', 'https://example.com/weddings'],
    ['Flower subscription', 'https://example.com/subscribe'],
    ['Visit the shop — 214 Bergen St', 'https://example.com/visit'],
  ], { fill: '#fbf3f1', color: '#5b2333', family: 'Hanken Grotesk', weight: 600, size: 34, stroke: ['#5b2333', 3] });
  b.text('Open Tue–Sun, 9am–6pm · (555) 014-2203', 90, 1640, 900, { family: 'Hanken Grotesk', weight: 500, size: 30, color: '#7a3e4f' }, 'center', { role: 'body' });
  b.line(440, 1730, 200, '#d9a7b4', 3);
});

// ------------------------------------------------------------------ 4. Food creator (cream & olive, arch photo)
tpl('lib-food-kitchen', 'Recipe Creator Links', CAT, 'link-in-bio', ['link in bio', 'food', 'recipes', 'cooking', 'cookbook'], (b) => {
  b.bg('#f4efe3');
  b.rect(0, 0, 1080, 560, '#556b2f');
  b.photo('e7830194-beba-4240-b618-f635df7d6e63', 240, 150, 600, 680, { clip: 'rounded', focus: [0.5, 0.4] });
  last(b).altText = 'Ingredients laid out on a kitchen table';
  b.text('The Little Kitchen', 90, 880, 900, { family: 'DM Serif Display', weight: 400, size: 88, color: '#2f3a1a' }, 'center', { role: 'headline', lh: 1.05 });
  b.text('Weeknight recipes in 30 minutes or less, by chef Sam Rivera.', 150, 990, 780, { family: 'Onest', weight: 400, size: 34, color: '#4a5530' }, 'center', { role: 'body', lh: 1.4 });
  buttons(b, 120, 1130, 840, 120, 30, [
    ['This week’s recipe: lemon orzo', 'https://example.com/recipes/lemon-orzo'],
    ['Pre-order the cookbook', 'https://example.com/cookbook'],
    ['Free meal-plan PDF', 'https://example.com/meal-plan'],
    ['Cooking classes', 'https://example.com/classes'],
  ], { fill: '#556b2f', color: '#fbf8ef', family: 'Onest', weight: 600, size: 34, r: 24 });
  b.text('Brand partnerships: sam@example.com', 90, 1780, 900, { family: 'Onest', weight: 500, size: 28, color: '#556b2f' }, 'center', { role: 'caption' });
});
