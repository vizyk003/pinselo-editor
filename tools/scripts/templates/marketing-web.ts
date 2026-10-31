/** Templates: marketing-web (see index.ts for categories). */
import { hex, sortedPages, type Element } from '@oc/engine';
import { B, lin, rad, solid, tpl } from './builder';

void [B, lin, rad, solid, tpl];

// ------------------------------------------------------------------ helpers
/** The element most recently added by the builder. */
const last = (b: B): Element => Object.values(b.doc.elements).at(-1)!;
/** Straight connector between two points. */
function conn(b: B, x1: number, y1: number, x2: number, y2: number, color: string, width = 4, dash?: number[]) {
  const L = Math.hypot(x2 - x1, y2 - y1);
  const rot = (Math.atan2(y2 - y1, x2 - x1) * 180) / Math.PI;
  return b.line((x1 + x2) / 2 - L / 2, (y1 + y2) / 2, L, color, width, { rot, ...(dash ? { dash } : {}) });
}
/** Outline-only shape of any kind. */
function outline(b: B, kind: Parameters<B['shape']>[0], x: number, y: number, w: number, h: number, color: string, width = 4, rot = 0) {
  b.shape(kind, x, y, w, h, '#000000', { stroke: [color, width], rot });
  (last(b) as Extract<Element, { type: 'shape' }>).fills = [];
  return b;
}
/** Recolour the last text element as a sequence of coloured runs (same style otherwise). */
function colorRuns(b: B, parts: Array<[string, string]>) {
  const el = last(b) as Extract<Element, { type: 'text' }>;
  const p = el.body.paragraphs[0]!;
  const style = p.runs[0]!.style;
  p.runs = parts.map(([text, c]) => ({ text, style: { ...style, fill: solid(c) } }));
  return b;
}
/** Style the last chart element. */
function chartStyle(b: B, s: Partial<Extract<Element, { type: 'chart' }>['style']> & { text?: string; grid?: string }) {
  const el = last(b) as Extract<Element, { type: 'chart' }>;
  const { text, grid, ...rest } = s;
  el.style = { ...el.style, ...rest, ...(text ? { textColor: hex(text) } : {}), ...(grid ? { gridColor: hex(grid) } : {}) };
  return b;
}
/** Name the pages of a multi-page design (website sections). */
function titles(b: B, names: string[]) {
  sortedPages(b.doc).forEach((p, i) => { if (names[i]) p.title = names[i]!; });
}

// ================================================================== LOGOS (500 × 500)
const LOGO = 'Logos';

tpl('mw-logo-bakery-badge', 'Artisan Bakery Badge', LOGO, 'logo', ['bakery', 'badge', 'emblem', 'food', 'vintage'], (b) => {
  b.bg('#f4ecdf');
  b.circle(250, 250, 190, '#5a3220');
  outline(b, 'ellipse', 74, 74, 352, 352, '#f4ecdf', 2);
  b.text('ARTISAN BAKERY', 120, 100, 260, { family: 'Fraunces', weight: 600, size: 26, color: '#f4ecdf', tracking: 220 }, 'center', { curve: 34, h: 60 });
  b.text('Hearth', 80, 178, 340, { family: 'Fraunces', weight: 700, italic: true, size: 76, color: '#f4ecdf' }, 'center', { role: 'logo', lh: 1.0 });
  b.text('& Crumb', 80, 256, 340, { family: 'Fraunces', weight: 400, italic: true, size: 46, color: '#e9c793' }, 'center', { lh: 1.0 });
  b.shape('diamond', 136, 336, 12, 12, '#e9c793');
  b.shape('diamond', 352, 336, 12, 12, '#e9c793');
  b.text('EST. 2026', 160, 328, 180, { family: 'Fraunces', weight: 600, size: 20, color: '#f4ecdf', tracking: 300 }, 'center');
});

tpl('mw-logo-law-monogram', 'Law Firm Monogram', LOGO, 'logo', ['law', 'monogram', 'legal', 'elegant', 'professional'], (b) => {
  b.bg('#f7f5f0');
  b.box(180, 70, 140, 140, '#1d2b45', 2);
  b.box(188, 78, 124, 124, '#b08d4a', 1);
  b.text('WV', 180, 100, 140, { family: 'Cinzel', weight: 600, size: 62, color: '#1d2b45' }, 'center', { role: 'logo', lh: 1.1 });
  b.text('WHITFORD & VALE', 40, 262, 420, { family: 'Cinzel', weight: 600, size: 34, color: '#1d2b45', tracking: 120 }, 'center', { role: 'headline' });
  b.line(210, 330, 80, '#b08d4a', 2);
  b.text('ATTORNEYS AT LAW', 40, 352, 420, { family: 'Montserrat', weight: 500, size: 14, color: '#5a4a2a', tracking: 400 }, 'center');
});

tpl('mw-logo-fitness-bold', 'Bold Fitness Wordmark', LOGO, 'logo', ['fitness', 'gym', 'sport', 'bold', 'wordmark'], (b) => {
  b.bg('#0b0b0c');
  b.shape('lightning', 214, 72, 72, 112, '#c6ff3d');
  b.text('IRONPULSE', 40, 196, 420, { family: 'Anton', size: 86, color: '#ffffff', tracking: 20 }, 'center', { role: 'logo', lh: 1.1 });
  b.shape('parallelogram', 92, 310, 316, 44, '#c6ff3d', { params: { skew: 0.2 } });
  b.text('STRENGTH CLUB', 92, 320, 316, { family: 'Archivo', weight: 800, size: 18, color: '#0b0b0c', tracking: 320 }, 'center');
});

tpl('mw-logo-tech-nodes', 'Tech Startup Mark', LOGO, 'logo', ['tech', 'startup', 'software', 'modern', 'abstract'], (b) => {
  b.bg('#ffffff');
  conn(b, 250, 118, 206, 196, '#6366f1', 6);
  conn(b, 250, 118, 294, 196, '#22d3ee', 6);
  conn(b, 206, 196, 294, 196, '#818cf8', 6);
  b.circle(250, 118, 26, lin(135, '#6366f1', '#22d3ee'));
  b.circle(206, 196, 20, '#6366f1');
  b.circle(294, 196, 20, '#06b6d4');
  b.text('nodewise', 40, 252, 420, { family: 'Sora', weight: 700, size: 66, color: '#0f172a', tracking: -20 }, 'center', { role: 'logo', lh: 1.1 });
  b.text('data pipelines, simplified', 40, 340, 420, { family: 'Inter', weight: 400, size: 19, color: '#475569' }, 'center');
});

tpl('mw-logo-florist-bloom', 'Florist Bloom Logo', LOGO, 'logo', ['florist', 'flowers', 'feminine', 'script', 'botanical'], (b) => {
  b.bg('#fbf3f1');
  const cx = 250, cy = 150, d = 42;
  for (let i = 0; i < 6; i++) {
    const a = (i * 60 * Math.PI) / 180;
    const px = cx + Math.sin(a) * d, py = cy - Math.cos(a) * d;
    b.shape('ellipse', px - 22, py - 46, 44, 92, i % 2 ? '#e9b3ab' : '#d98b80', { rot: i * 60, opacity: 0.85 });
  }
  b.circle(cx, cy, 16, '#7a3b2e');
  b.text('Wild Fern', 40, 236, 420, { family: 'Parisienne', size: 78, color: '#7a3b2e' }, 'center', { role: 'logo', lh: 1.15 });
  b.text('FLORAL STUDIO', 40, 352, 420, { family: 'Tenor Sans', size: 16, color: '#7a3b2e', tracking: 450 }, 'center');
});

tpl('mw-logo-photo-aperture', 'Photography Studio Logo', LOGO, 'logo', ['photography', 'studio', 'camera', 'minimal', 'dark'], (b) => {
  b.bg('#111111');
  outline(b, 'ellipse', 185, 70, 130, 130, '#ffffff', 5);
  outline(b, 'hexagon', 215, 102, 70, 66, '#ffffff', 3, 30);
  b.circle(250, 135, 13, '#f5b942');
  b.text('LUMEN', 40, 236, 420, { family: 'Syne', weight: 800, size: 62, color: '#ffffff', tracking: 120 }, 'center', { role: 'logo', lh: 1.1 });
  b.text('PHOTOGRAPHY STUDIO', 40, 340, 420, { family: 'Inter', weight: 500, size: 14, color: '#c7c7c7', tracking: 420 }, 'center');
});

tpl('mw-logo-realestate-keystone', 'Real Estate Keystone', LOGO, 'logo', ['real estate', 'realty', 'homes', 'property', 'classic'], (b) => {
  b.bg('#f3f1ea');
  b.shape('arch', 196, 104, 108, 120, '#1f4d3a');
  b.rect(234, 164, 32, 60, '#f3f1ea', { r: 16 });
  b.shape('trapezoid', 228, 74, 44, 40, '#c9a24a', { rot: 180 });
  b.text('KEYSTONE', 40, 256, 420, { family: 'Marcellus', size: 56, color: '#1f4d3a', tracking: 140 }, 'center', { role: 'logo', lh: 1.1 });
  b.text('HOMES & REALTY', 40, 340, 420, { family: 'Montserrat', weight: 600, size: 14, color: '#6f561a', tracking: 450 }, 'center');
});

tpl('mw-logo-cafe-seal', 'Café Seal Badge', LOGO, 'logo', ['cafe', 'coffee', 'badge', 'retro', 'warm'], (b) => {
  b.bg('#fdf6ec');
  b.shape('seal', 55, 55, 390, 390, '#c4572e', { params: { points: 24, innerRatio: 0.93 } });
  outline(b, 'ellipse', 100, 100, 300, 300, '#fdf6ec', 2);
  b.text('SLOW COFFEE', 140, 122, 220, { family: 'Archivo', weight: 700, size: 22, color: '#fdf6ec', tracking: 260 }, 'center', { curve: 30, h: 50 });
  b.text('Oat &\nEmber', 90, 182, 320, { family: 'Shrikhand', size: 58, color: '#fdf6ec' }, 'center', { role: 'logo', lh: 0.95 });
  b.text('SINCE 2026', 140, 330, 220, { family: 'Archivo', weight: 700, size: 18, color: '#fdf6ec', tracking: 300 }, 'center');
});

tpl('mw-logo-kids-sprouts', 'Playful Kids Brand', LOGO, 'logo', ['kids', 'children', 'playful', 'colorful', 'toys'], (b) => {
  b.bg('#fffaf0');
  b.shape('drop', 214, 70, 40, 64, '#2f9e44', { rot: -45 });
  b.shape('drop', 248, 70, 40, 64, '#69db7c', { rot: 45 });
  b.rect(247, 120, 6, 50, '#2f9e44', { r: 3 });
  b.text('little', 40, 166, 420, { family: 'Fredoka', weight: 600, size: 42, color: '#2b8a3e' }, 'center', { lh: 1.1 });
  b.text('sprouts', 30, 214, 440, { family: 'Lilita One', size: 104, color: '#e8553f' }, 'center', { role: 'logo', lh: 1.1 });
  colorRuns(b, [['s', '#e03131'], ['p', '#d9480f'], ['r', '#2f9e44'], ['o', '#1c7ed6'], ['u', '#9c36b5'], ['t', '#e64980'], ['s', '#0c8599']]);
  b.text('PLAY · GROW · DISCOVER', 40, 360, 420, { family: 'Fredoka', weight: 500, size: 17, color: '#495057', tracking: 200 }, 'center');
});

tpl('mw-logo-consulting-north', 'Consulting Wordmark', LOGO, 'logo', ['consulting', 'business', 'corporate', 'navy', 'minimal'], (b) => {
  b.bg('#0e1a2b');
  b.shape('triangle', 226, 96, 48, 64, '#d4b06a');
  b.rect(248, 160, 4, 40, '#d4b06a');
  b.text('NORTHLINE', 30, 232, 440, { family: 'Inter Tight', weight: 600, size: 52, color: '#ffffff', tracking: 240 }, 'center', { role: 'logo', lh: 1.1 });
  b.line(130, 318, 240, '#2c4466', 2);
  b.text('ADVISORY GROUP', 40, 340, 420, { family: 'Inter Tight', weight: 400, size: 16, color: '#a9bdd4', tracking: 500 }, 'center');
});

// ================================================================== EMAIL HEADERS (600 × 200)
const EH = 'Email Headers';

tpl('mw-eh-newsletter-brief', 'Editorial Newsletter Header', EH, 'email-header', ['newsletter', 'editorial', 'weekly', 'minimal'], (b) => {
  b.bg('#f4efe6');
  b.rect(40, 28, 520, 2, '#1a1a1a');
  b.rect(40, 170, 520, 1, '#1a1a1a');
  b.text('ISSUE 42 · MONDAY, OCTOBER 12, 2026', 40, 44, 400, { family: 'Inter', weight: 600, size: 11, color: '#5c5346', tracking: 200 });
  b.text('The Monday Brief', 40, 66, 420, { family: 'Instrument Serif', size: 64, color: '#1a1a1a' }, 'start', { role: 'headline', lh: 1.1 });
  b.text('Five ideas worth your coffee, every week.', 40, 140, 400, { family: 'Instrument Serif', italic: true, size: 17, color: '#5c5346' });
  b.circle(510, 104, 46, '#e4572e');
  b.emoji('☕', 482, 76, 56);
});

tpl('mw-eh-flash-sale', 'Flash Sale Header', EH, 'email-header', ['sale', 'promo', 'discount', 'ecommerce', 'bold'], (b) => {
  b.bg(lin(90, '#e11d48', '#f97316'));
  b.shape('ellipse', 380, -120, 300, 300, '#ffffff', { opacity: 0.12 });
  b.text('48 HOURS ONLY', 40, 34, 300, { family: 'Archivo', weight: 800, size: 14, color: '#ffffff', tracking: 300 });
  b.text('FLASH SALE', 36, 52, 360, { family: 'Anton', size: 82, color: '#ffffff' }, 'start', { role: 'headline', lh: 1.1 });
  b.text('Everything sitewide · code FLASH40', 40, 150, 330, { family: 'Archivo', weight: 500, size: 15, color: '#ffffff' });
  b.shape('burst', 420, 22, 156, 156, '#fff7ed', { params: { points: 14, innerRatio: 0.82 } });
  b.text('40%', 420, 66, 156, { family: 'Anton', size: 46, color: '#be123c' }, 'center', { lh: 1.0 });
  b.text('OFF', 420, 114, 156, { family: 'Archivo', weight: 800, size: 18, color: '#9f1239', tracking: 300 }, 'center');
});

tpl('mw-eh-welcome-plants', 'Welcome Email Header', EH, 'email-header', ['welcome', 'onboarding', 'plants', 'fresh', 'green'], (b) => {
  b.bg('#e7f0e8');
  b.shape('blob', 380, -40, 280, 260, '#bcd9c0');
  b.emoji('🪴', 430, 30, 140);
  b.text('WELCOME TO FERNWAY', 40, 50, 340, { family: 'DM Sans', weight: 700, size: 12, color: '#2f5d3a', tracking: 300 });
  b.text('Hello, plant parent.', 40, 72, 380, { family: 'DM Serif Display', size: 36, color: '#173b22' }, 'start', { role: 'headline', lh: 1.1 });
  b.text('Your first care guide is inside — let’s grow.', 40, 124, 340, { family: 'DM Sans', weight: 500, size: 15, color: '#2f5d3a' });
});

tpl('mw-eh-event-designweek', 'Event Invite Header', EH, 'email-header', ['event', 'conference', 'invite', 'design', 'dark'], (b) => {
  b.bg('#14112b');
  b.circle(560, 20, 120, rad('#7c3aed', '#14112b'));
  b.circle(470, 200, 90, rad('#ec4899', '#14112b'));
  b.text('YOU’RE INVITED', 40, 40, 300, { family: 'Unbounded', weight: 500, size: 12, color: '#c4b5fd', tracking: 300 });
  b.text('Design Week 2026', 40, 64, 500, { family: 'Unbounded', weight: 800, size: 34, color: '#ffffff' }, 'start', { role: 'headline', lh: 1.1 });
  b.rect(40, 132, 238, 32, '#ffffff', { r: 16 });
  b.text('Nov 4–6 · Lisbon, Portugal', 40, 140, 238, { family: 'Inter', weight: 600, size: 13, color: '#14112b' }, 'center');
});

tpl('mw-eh-holiday-gifts', 'Holiday Gift Guide Header', EH, 'email-header', ['holiday', 'christmas', 'gift guide', 'seasonal', 'festive'], (b) => {
  b.bg('#0f3d2e');
  for (const [x, y, s] of [[420, 34, 16], [560, 40, 12], [410, 140, 10], [566, 156, 16]] as const) b.shape('star', x, y, s, s, '#e8c872', { params: { points: 4, innerRatio: 0.35 } });
  b.circle(500, 100, 62, '#164d3a');
  b.emoji('🎁', 456, 56, 88, { rot: -8 });
  b.rect(0, 0, 14, 200, '#b3282d');
  b.text('THE 2026 EDIT', 50, 46, 300, { family: 'Montserrat', weight: 600, size: 12, color: '#e8c872', tracking: 400 });
  b.text('Holiday Gift Guide', 50, 66, 420, { family: 'Playfair Display', weight: 500, italic: true, size: 46, color: '#fbf6ea' }, 'start', { role: 'headline', lh: 1.1 });
  b.text('120+ thoughtful finds, all under $75', 50, 130, 400, { family: 'Montserrat', weight: 500, size: 14, color: '#cfe3d6' });
});

// ================================================================== INFOGRAPHICS (800 × 2000)
const IG = 'Infographics';

tpl('mw-ig-timeline-story', 'Company Timeline', IG, 'infographic', ['timeline', 'history', 'company', 'milestones', 'story'], (b) => {
  b.bg('#fbf7f0');
  b.rect(0, 0, 800, 440, '#1f3d3a');
  b.circle(700, 80, 160, '#2b5450');
  b.text('OUR STORY · 2019 – 2026', 64, 90, 600, { family: 'Inter', weight: 700, size: 16, color: '#f2c879', tracking: 300 });
  b.text('From a garage roaster to forty cafés', 64, 130, 640, { family: 'Fraunces', weight: 600, size: 58, color: '#fbf7f0' }, 'start', { role: 'headline', lh: 1.05 });
  b.text('Seven years of Kindling Coffee, one cup at a time.', 64, 322, 640, { family: 'Inter', size: 20, color: '#cfe0dc' });
  const items: Array<[string, string, string]> = [
    ['2019', 'The first roast', 'Two friends, one 1 kg roaster and a borrowed garage in Portland.'],
    ['2020', 'Doorstep delivery', 'Weekly subscriptions kept neighbours caffeinated through lockdown.'],
    ['2021', 'Café no. 1', 'A 30-seat corner café opens on Alder Street.'],
    ['2022', 'Direct trade', 'Long-term partnerships with six farms in Colombia and Ethiopia.'],
    ['2023', 'Roastery HQ', 'A solar-powered roastery triples production capacity.'],
    ['2025', 'Coast to coast', 'Our 25th café opens — and our first in New York.'],
    ['2026', 'Forty and counting', 'Forty cafés, 380 baristas and one very old garage roaster.'],
  ];
  const top = 530, step = 196;
  b.rect(398, top - 20, 4, step * (items.length - 1) + 40, '#d9cfbd');
  items.forEach(([year, title, desc], i) => {
    const y = top + i * step;
    const left = i % 2 === 0;
    const x = left ? 64 : 456;
    b.circle(400, y + 26, 16, i === items.length - 1 ? '#e07a3f' : '#1f3d3a');
    b.circle(400, y + 26, 6, '#fbf7f0');
    b.text(year, x, y, 280, { family: 'Fraunces', weight: 700, size: 40, color: '#c05621' }, left ? 'end' : 'start', { lh: 1.1 });
    b.text(title, x, y + 54, 280, { family: 'Inter', weight: 700, size: 20, color: '#1f3d3a' }, left ? 'end' : 'start');
    b.text(desc, x, y + 86, 280, { family: 'Inter', size: 16, color: '#5b5548' }, left ? 'end' : 'start', { lh: 1.4 });
  });
  b.rect(64, 1880, 672, 1, '#d9cfbd');
  b.text('kindling.example.com', 64, 1910, 672, { family: 'Inter', weight: 600, size: 16, color: '#1f3d3a' }, 'center');
});

tpl('mw-ig-stats-remote', 'Remote Work Statistics', IG, 'infographic', ['statistics', 'data', 'chart', 'work', 'survey'], (b) => {
  b.bg('#f5f6fb');
  b.rect(0, 0, 800, 420, lin(135, '#312e81', '#4f46e5'));
  b.circle(730, 60, 170, '#6366f1', { opacity: 0.5 });
  b.text('2026 WORKPLACE SURVEY', 64, 84, 600, { family: 'Space Grotesk', weight: 700, size: 16, color: '#c7d2fe', tracking: 300 });
  b.text('How we work now', 64, 120, 672, { family: 'Space Grotesk', weight: 700, size: 72, color: '#ffffff' }, 'start', { role: 'headline', lh: 1.05 });
  b.text('What 2,400 office workers told us about hybrid life, focus time and the perks that matter.', 64, 236, 560, { family: 'Inter', size: 20, color: '#e0e7ff' }, 'start', { lh: 1.45 });
  const stats: Array<[string, string, string]> = [['68%', 'work hybrid at least part of the week', '#4f46e5'], ['2.4', 'average days per week in the office', '#0891b2'], ['41%', 'say focus time is their top need', '#db2777']];
  stats.forEach(([n, t, c], i) => {
    const x = 64 + i * 232;
    b.rect(x, 480, 208, 230, '#ffffff', { r: 20, shadow: [0, 6, 18, '#1e1b4b22'] });
    b.text(n, x + 24, 508, 170, { family: 'Space Grotesk', weight: 700, size: 60, color: c }, 'start', { lh: 1.1 });
    b.text(t, x + 24, 596, 168, { family: 'Inter', size: 17, color: '#334155' }, 'start', { lh: 1.4 });
  });
  b.text('Days in the office per week', 64, 820, 672, { family: 'Space Grotesk', weight: 700, size: 30, color: '#1e1b4b' }, 'start', { role: 'subhead' });
  b.text('Share of respondents', 64, 864, 672, { family: 'Inter', size: 17, color: '#475569' });
  b.rect(64, 906, 672, 380, '#ffffff', { r: 20 });
  b.chart('column', 88, 926, 624, 340, ['0', '1', '2', '3', '4', '5'], [{ name: 'Respondents (%)', values: [14, 12, 24, 27, 13, 10], color: '#4f46e5' }]);
  chartStyle(b, { family: 'Inter', fontSize: 16, showLegend: false, showValues: true, text: '#334155' });
  b.text('Most-wanted perks', 64, 1340, 672, { family: 'Space Grotesk', weight: 700, size: 30, color: '#1e1b4b' }, 'start', { role: 'subhead' });
  b.rect(64, 1396, 672, 400, '#ffffff', { r: 20 });
  b.chart('donut', 88, 1420, 340, 352, ['Flexible hours', 'Learning budget', 'Home office kit', 'Wellness stipend'], [{ name: 'Votes', values: [38, 24, 22, 16] }]);
  chartStyle(b, { family: 'Inter', fontSize: 16, showLegend: false, text: '#334155' });
  const legend: Array<[string, string, string]> = [['Flexible hours', '38%', '#7c3aed'], ['Learning budget', '24%', '#06b6d4'], ['Home office kit', '22%', '#f59e0b'], ['Wellness stipend', '16%', '#ec4899']];
  legend.forEach(([l, v, c], i) => {
    const y = 1480 + i * 66;
    b.rect(460, y + 4, 22, 22, c, { r: 6 });
    b.text(l, 496, y, 160, { family: 'Inter', weight: 600, size: 18, color: '#1e293b' });
    b.text(v, 640, y, 72, { family: 'Space Grotesk', weight: 700, size: 20, color: '#1e1b4b' }, 'end');
  });
  b.text('Source: illustrative survey of 2,400 full-time office workers, September 2026.', 64, 1860, 672, { family: 'Inter', size: 14, color: '#55657a' }, 'center');
  b.text('workpulse.example.com', 64, 1900, 672, { family: 'Space Grotesk', weight: 700, size: 18, color: '#4338ca' }, 'center');
});

tpl('mw-ig-compare-rent-buy', 'Renting vs Buying Comparison', IG, 'infographic', ['comparison', 'versus', 'real estate', 'finance', 'home'], (b) => {
  b.bg('#ffffff');
  b.text('THE BIG DECISION', 64, 90, 672, { family: 'Bricolage Grotesque', weight: 700, size: 16, color: '#64748b', tracking: 300 }, 'center');
  b.text('Renting vs. Buying', 64, 124, 672, { family: 'Bricolage Grotesque', weight: 800, size: 68, color: '#111827' }, 'center', { role: 'headline', lh: 1.05 });
  b.text('A side-by-side look at what each choice really means for your money and lifestyle.', 114, 220, 572, { family: 'Inter', size: 19, color: '#475569' }, 'center', { lh: 1.45 });
  b.rect(64, 340, 330, 150, '#0ea5a4', { r: 24 });
  b.rect(406, 340, 330, 150, '#ea580c', { r: 24 });
  b.emoji('🔑', 84, 372, 86);
  b.emoji('🏡', 426, 372, 86);
  b.text('RENTING', 180, 396, 200, { family: 'Bricolage Grotesque', weight: 800, size: 34, color: '#ffffff' });
  b.text('BUYING', 522, 396, 200, { family: 'Bricolage Grotesque', weight: 800, size: 34, color: '#ffffff' });
  const rows: Array<[string, string, string]> = [
    ['Upfront cost', 'Deposit of 1–2 months’ rent', '10–20% down payment plus closing fees'],
    ['Monthly cost', 'Fixed for the lease term', 'Mortgage, tax, insurance and upkeep'],
    ['Flexibility', 'Move when your lease ends', 'Selling takes months and money'],
    ['Maintenance', 'Your landlord fixes the boiler', 'Every repair is on you'],
    ['Long-term value', 'No equity built', 'Each payment builds ownership'],
    ['Best for', 'Career moves, testing a city, saving up', 'Staying put five years or more'],
  ];
  rows.forEach(([label, rent, buy], i) => {
    const y = 540 + i * 210;
    b.rect(64, y + 58, 672, 132, i % 2 ? '#ffffff' : '#f8fafc', { r: 16 });
    b.rect(300, y, 200, 44, '#111827', { r: 22 });
    b.text(label, 300, y + 10, 200, { family: 'Inter', weight: 700, size: 17, color: '#ffffff' }, 'center');
    b.text(rent, 92, y + 92, 276, { family: 'Inter', size: 19, color: '#134e4a' }, 'center', { lh: 1.4 });
    b.text(buy, 432, y + 92, 276, { family: 'Inter', size: 19, color: '#7c2d12' }, 'center', { lh: 1.4 });
    b.rect(399, y + 76, 2, 96, '#e2e8f0');
  });
  b.rect(64, 1830, 672, 110, '#fef3c7', { r: 20 });
  b.text('Rule of thumb: if you’ll stay less than five years, renting usually wins.', 100, 1858, 600, { family: 'Inter', weight: 600, size: 19, color: '#78350f' }, 'center', { lh: 1.4 });
});

tpl('mw-ig-howto-pourover', 'How-To Steps', IG, 'infographic', ['how to', 'steps', 'guide', 'coffee', 'tutorial'], (b) => {
  b.bg('#f3e9dc');
  b.text('BREW GUIDE', 64, 90, 672, { family: 'DM Sans', weight: 700, size: 16, color: '#8a5a3b', tracking: 400 }, 'center');
  b.text('The Perfect\nPour-Over', 64, 124, 672, { family: 'Young Serif', size: 64, color: '#3b2416' }, 'center', { role: 'headline', lh: 1.1 });
  b.text('Six steps · 4 minutes · 1 great cup', 64, 286, 672, { family: 'DM Sans', weight: 500, size: 20, color: '#6b4a34' }, 'center');
  const steps: Array<[string, string, string]> = [
    ['💧', 'Heat the water', 'Bring fresh water to 94 °C — just off the boil.'],
    ['⚖️', 'Weigh your beans', 'Use 20 g of coffee for 320 g of water (1:16).'],
    ['⚙️', 'Grind medium-fine', 'Aim for the texture of coarse sea salt.'],
    ['🌀', 'Let it bloom', 'Pour 50 g of water and wait 40 seconds.'],
    ['⏱️', 'Pour in circles', 'Add the rest slowly, finishing by 2:30.'],
    ['☕', 'Swirl and enjoy', 'Total brew time should land around 3:30.'],
  ];
  steps.forEach(([em, t, d], i) => {
    const y = 360 + i * 252;
    const right = i % 2 === 1;
    b.rect(64, y, 672, 220, '#fffaf3', { r: 28 });
    const ix = right ? 536 : 84;
    b.circle(ix + 90, y + 110, 84, '#e9d3bc');
    b.emoji(em, ix + 40, y + 60, 100);
    const tx = right ? 104 : 296;
    b.text(`STEP ${i + 1}`, tx, y + 42, 400, { family: 'DM Sans', weight: 700, size: 15, color: '#a0522d', tracking: 300 });
    b.text(t, tx, y + 70, 400, { family: 'Young Serif', size: 34, color: '#3b2416' }, 'start', { lh: 1.15 });
    b.text(d, tx, y + 124, 380, { family: 'DM Sans', size: 19, color: '#5c4433' }, 'start', { lh: 1.45 });
  });
  b.text('More recipes at brewbetter.example.com', 64, 1918, 672, { family: 'DM Sans', weight: 600, size: 16, color: '#6b4a34' }, 'center');
});

tpl('mw-ig-ocean-numbers', 'Ocean by the Numbers', IG, 'infographic', ['by the numbers', 'facts', 'ocean', 'nature', 'education'], (b) => {
  b.bg(lin(180, '#0b3a5b', '#06203a', '#03111f'));
  b.text('WORLD OCEANS DAY', 64, 96, 672, { family: 'Archivo', weight: 700, size: 16, color: '#7dd3fc', tracking: 400 }, 'center');
  b.text('The Ocean by the Numbers', 64, 130, 672, { family: 'Archivo', weight: 900, size: 64, color: '#ffffff' }, 'center', { role: 'headline', lh: 1.05 });
  b.circle(400, 330, 46, '#0e4f7a');
  b.emoji('🌊', 368, 298, 64);
  const facts: Array<[string, string]> = [
    ['71%', 'of Earth’s surface is covered by ocean'],
    ['50%', 'of the oxygen we breathe is produced by marine plants and plankton'],
    ['~11 km', 'deep at the bottom of the Mariana Trench — deeper than Everest is tall'],
    ['3 billion', 'people depend on the ocean for their livelihoods'],
    ['80%+', 'of the ocean floor has never been mapped in high detail'],
  ];
  facts.forEach(([n, t], i) => {
    const y = 440 + i * 270;
    const alt = i % 2 === 1;
    b.text(n, 64, y, 672, { family: 'Archivo', weight: 900, size: 100, color: alt ? '#7dd3fc' : '#fde68a' }, alt ? 'end' : 'start', { lh: 1.05 });
    b.text(t, alt ? 236 : 64, y + 118, 500, { family: 'Inter', size: 22, color: '#e0f2fe' }, alt ? 'end' : 'start', { lh: 1.45 });
    if (i < facts.length - 1) b.line(64, y + 236, 672, '#1e4a6b', 2, { dash: [6, 8] });
  });
  b.emoji('🐋', 560, 1780, 130);
  b.text('Figures are rounded global estimates.\nseaforward.example.org', 64, 1820, 460, { family: 'Inter', size: 16, color: '#bae6fd' }, 'start', { lh: 1.5 });
});

tpl('mw-ig-moving-checklist', 'Moving Checklist', IG, 'infographic', ['checklist', 'moving', 'planning', 'home', 'tips'], (b) => {
  b.bg('#fff8ef');
  b.rect(0, 0, 800, 360, '#ffd8b5');
  b.emoji('📦', 560, 100, 170, { rot: 8 });
  b.text('THE ULTIMATE', 64, 92, 460, { family: 'Gabarito', weight: 700, size: 18, color: '#9a3412', tracking: 300 });
  b.text('Moving Day Checklist', 64, 124, 480, { family: 'Gabarito', weight: 900, size: 62, color: '#431407' }, 'start', { role: 'headline', lh: 1.0 });
  b.text('Tick it off, week by week.', 64, 278, 460, { family: 'Gabarito', weight: 500, size: 22, color: '#7c2d12' });
  const groups: Array<[string, string, string[]]> = [
    ['8 weeks before', '#f97316', ['Set a moving budget', 'Book movers or a van', 'Declutter room by room', 'Collect free boxes']],
    ['4 weeks before', '#0ea5e9', ['Give notice to your landlord', 'Update your address', 'Transfer internet and utilities', 'Start packing rarely used rooms']],
    ['1 week before', '#8b5cf6', ['Confirm the moving time', 'Pack an overnight bag', 'Defrost the freezer']],
    ['Moving day', '#16a34a', ['Take meter readings', 'Check every cupboard', 'Hand over the keys']],
  ];
  let y = 420;
  for (const [h, c, list] of groups) {
    b.rect(64, y, 10, 40, c, { r: 5 });
    b.text(h.toUpperCase(), 92, y + 2, 600, { family: 'Gabarito', weight: 800, size: 26, color: '#431407', tracking: 100 });
    y += 72;
    for (const item of list) {
      b.rect(92, y, 30, 30, '#ffffff', { r: 8, stroke: [c, 3] });
      b.text(item, 146, y - 1, 560, { family: 'Inter', size: 22, color: '#3f2a1d' });
      y += 60;
    }
    y += 56;
  }
  b.rect(64, 1800, 672, 120, '#431407', { r: 20 });
  b.text('Pro tip: label boxes on the side, not the top — you’ll still read them when stacked.', 100, 1826, 600, { family: 'Inter', weight: 500, size: 19, color: '#ffedd5' }, 'center', { lh: 1.45 });
});

// ================================================================== WHITEBOARDS (4000 × 3000)
const WB = 'Whiteboards';
const HAND = 'Kalam';
/** A sticky note with optional handwritten text. */
function sticky(b: B, x: number, y: number, color: string, text = '', w = 380, h = 340, size = 44) {
  b.rect(x, y, w, h, color, { r: 6, shadow: [0, 10, 18, '#0000001f'] });
  if (text) b.text(text, x + 32, y + 30, w - 64, { family: HAND, weight: 400, size, color: '#2b2b2b' }, 'start', { lh: 1.25 });
  return b;
}
/** Board title block, top-left. */
function boardTitle(b: B, title: string, sub: string, accent: string) {
  b.rect(160, 150, 24, 210, accent, { r: 12 });
  b.text(title, 230, 140, 2600, { family: 'Gabarito', weight: 800, size: 120, color: '#1f2328' }, 'start', { role: 'headline', lh: 1.1 });
  b.text(sub, 232, 290, 2600, { family: 'Inter', weight: 500, size: 50, color: '#57606a' });
}

tpl('mw-wb-brainstorm', 'Brainstorm Board', WB, 'whiteboard', ['brainstorm', 'ideas', 'workshop', 'team', 'sticky notes'], (b) => {
  b.bg('#f6f5f1');
  boardTitle(b, 'Brainstorm: Spring Campaign', 'Diverge first, then group and dot-vote · 45 min', '#f59e0b');
  // centre question
  b.shape('ellipse', 1500, 1220, 1000, 560, '#1f2328');
  b.text('How might we reach 1,000 new members by March?', 1600, 1380, 800, { family: 'Gabarito', weight: 700, size: 76, color: '#ffffff' }, 'center', { lh: 1.15, role: 'subhead' });
  const clusters: Array<[string, string, number, number, string[]]> = [
    ['Social', '#fde68a', 260, 560, ['Member stories reel', 'Weekly live Q&A', '']],
    ['Partnerships', '#bbf7d0', 2560, 560, ['Co-host with local gyms', 'Student discount', '']],
    ['Events', '#bfdbfe', 260, 1960, ['Free open day', 'Bring-a-friend week', '']],
    ['Referral', '#fbcfe8', 2560, 1960, ['Give a month, get a month', 'Leaderboard prizes', '']],
  ];
  for (const [label, c, x, y, notes] of clusters) {
    b.rect(x, y, 1180, 760, '#ffffff', { r: 40, stroke: ['#e5e3dc', 4] });
    b.text(label, x + 60, y + 44, 900, { family: 'Gabarito', weight: 800, size: 64, color: '#1f2328' });
    notes.forEach((n, i) => sticky(b, x + 60 + i * 370, y + 180 + (i % 2) * 40, c, n, 330, 300, 42));
  }
  conn(b, 1560, 1330, 1440, 1180, '#8c959f', 6, [14, 14]);
  conn(b, 2440, 1330, 2560, 1180, '#8c959f', 6, [14, 14]);
  conn(b, 1560, 1670, 1440, 1960, '#8c959f', 6, [14, 14]);
  conn(b, 2440, 1670, 2560, 1960, '#8c959f', 6, [14, 14]);
  // how-to card
  b.rect(2960, 150, 880, 300, '#1f2328', { r: 30 });
  b.text('1 · Add one idea per sticky\n2 · Cluster similar ideas\n3 · Three dot-votes each', 3010, 186, 800, { family: 'Inter', weight: 500, size: 46, color: '#ffffff' }, 'start', { lh: 1.5 });
  b.text('Parking lot', 1660, 2440, 680, { family: 'Gabarito', weight: 800, size: 56, color: '#57606a' }, 'center');
  b.box(1560, 2540, 880, 300, '#b6bcc4', 4, 30);
});

tpl('mw-wb-retrospective', 'Sprint Retrospective', WB, 'whiteboard', ['retrospective', 'retro', 'agile', 'sprint', 'team'], (b) => {
  b.bg('#ffffff');
  boardTitle(b, 'Sprint 14 Retrospective', 'Team Atlas · 6 – 17 October 2026 · Facilitator: Priya', '#6366f1');
  const cols: Array<[string, string, string, string, string[]]> = [
    ['Went well', '😀', '#dcfce7', '#16a34a', ['Shipped search filters two days early', 'Pairing on the API migration', 'Calm, focused standups']],
    ['To improve', '🤔', '#fef3c7', '#d97706', ['Too many late scope changes', 'Flaky end-to-end tests', '']],
    ['Action items', '🎯', '#e0e7ff', '#4f46e5', ['Freeze scope at mid-sprint review — Sam', 'Quarantine flaky tests — Leo', '']],
  ];
  cols.forEach(([h, em, bgc, c, notes], i) => {
    const x = 160 + i * 1240;
    b.rect(x, 460, 1160, 2380, bgc, { r: 40 });
    b.emoji(em, x + 60, 520, 110);
    b.text(h, x + 200, 530, 900, { family: 'Gabarito', weight: 800, size: 80, color: '#1f2328' });
    b.rect(x + 60, 680, 1040, 8, c, { r: 4 });
    notes.forEach((n, k) => sticky(b, x + 60 + (k % 2) * 540, 760 + Math.floor(k / 2) * 420, '#ffffff', n, 500, 360, 44));
  });
});

tpl('mw-wb-kanban', 'Kanban Board', WB, 'whiteboard', ['kanban', 'project', 'tasks', 'workflow', 'planning'], (b) => {
  b.bg('#f3f4f6');
  boardTitle(b, 'Website Relaunch · Kanban', 'Move cards left to right · WIP limit 3 for In progress', '#0ea5e9');
  const cols: Array<[string, string, Array<[string, string, string]>]> = [
    ['Backlog', '#94a3b8', [['Blog migration plan', 'Content', 'MK'], ['Cookie banner copy', 'Legal', 'JS'], ['Image compression', 'Dev', 'AL']]],
    ['To do', '#0ea5e9', [['Pricing page wireframe', 'Design', 'RB'], ['Set up analytics', 'Dev', 'AL']]],
    ['In progress', '#f59e0b', [['New homepage hero', 'Design', 'RB'], ['CMS content model', 'Dev', 'TN']]],
    ['Review', '#8b5cf6', [['About page copy', 'Content', 'MK']]],
    ['Done', '#22c55e', [['Brand colour tokens', 'Design', 'RB'], ['Domain & hosting', 'Dev', 'TN']]],
  ];
  const tagC: Record<string, string> = { Content: '#fce7f3', Legal: '#fef9c3', Dev: '#dbeafe', Design: '#ede9fe' };
  cols.forEach(([h, c, cards], i) => {
    const x = 160 + i * 740;
    b.rect(x, 460, 700, 2380, '#e5e7eb', { r: 32 });
    b.circle(x + 70, 552, 18, c);
    b.text(h, x + 110, 516, 440, { family: 'Gabarito', weight: 800, size: 60, color: '#1f2328' });
    b.rect(x + 560, 520, 90, 64, '#ffffff', { r: 32 });
    b.text(String(cards.length), x + 560, 527, 90, { family: 'Inter', weight: 700, size: 40, color: '#374151' }, 'center');
    cards.forEach(([t, tag, who], k) => {
      const y = 650 + k * 330;
      b.rect(x + 40, y, 620, 290, '#ffffff', { r: 20, shadow: [0, 6, 14, '#0000001a'] });
      b.rect(x + 40, y, 12, 290, c, { r: 6 });
      b.rect(x + 80, y + 36, 220, 60, tagC[tag]!, { r: 30 });
      b.text(tag, x + 80, y + 44, 220, { family: 'Inter', weight: 700, size: 36, color: '#374151' }, 'center');
      b.text(t, x + 80, y + 120, 540, { family: 'Inter', weight: 600, size: 46, color: '#111827' }, 'start', { lh: 1.2 });
      b.circle(x + 600, y + 66, 36, '#1f2937');
      b.text(who, x + 564, y + 46, 72, { family: 'Inter', weight: 700, size: 34, color: '#ffffff' }, 'center');
    });
  });
});

tpl('mw-wb-mindmap', 'Mind Map', WB, 'whiteboard', ['mind map', 'ideas', 'planning', 'brainstorm', 'diagram'], (b) => {
  b.bg('#fbfaf7');
  boardTitle(b, 'Mind Map: Launching Our Podcast', 'Start in the middle, branch out, keep each node to a few words', '#ec4899');
  const cx = 2000, cy = 1700;
  const branches: Array<[string, string, number, number, string[]]> = [
    ['Format', '#db2777', 900, 900, ['30-min episodes', 'Two co-hosts']],
    ['Guests', '#8b5cf6', 2000, 700, ['Customers', 'Industry experts']],
    ['Equipment', '#0284c7', 3100, 900, ['USB mics', 'Quiet room']],
    ['Promotion', '#c2410c', 3100, 2500, ['Audiogram clips', 'Newsletter']],
    ['Schedule', '#15803d', 2000, 2700, ['Bi-weekly', 'Batch record']],
    ['Budget', '#ef4444', 900, 2500, ['$1,200 setup', 'Hosting $20/mo']],
  ];
  for (const [label, c, bx, by, leaves] of branches) {
    conn(b, cx, cy, bx, by, c, 12);
    leaves.forEach((l, i) => {
      const dx = bx < cx - 10 ? -1 : bx > cx + 10 ? 1 : (i ? 1 : -1);
      const lx = bx + dx * 520 + (bx === cx ? 0 : 0), ly = by + (by < cy ? -1 : 1) * (i ? 260 : -40) + (bx === cx ? (by < cy ? 230 : -230) : 0);
      conn(b, bx, by, lx, ly, c, 6);
      b.rect(lx - 230, ly - 60, 460, 120, '#ffffff', { r: 60, stroke: [c, 6] });
      b.text(l, lx - 210, ly - 30, 420, { family: 'Inter', weight: 600, size: 44, color: '#1f2328' }, 'center');
    });
    b.rect(bx - 250, by - 80, 500, 160, c, { r: 80 });
    b.text(label, bx - 230, by - 38, 460, { family: 'Gabarito', weight: 800, size: 64, color: '#ffffff' }, 'center');
  }
  b.circle(cx, cy, 330, '#1f2328');
  b.text('Our\nPodcast', cx - 280, cy - 130, 560, { family: 'Gabarito', weight: 800, size: 110, color: '#ffffff' }, 'center', { lh: 1.05, role: 'subhead' });
});

tpl('mw-wb-journey-map', 'Customer Journey Map', WB, 'whiteboard', ['journey map', 'ux', 'customer', 'research', 'service design'], (b) => {
  b.bg('#ffffff');
  boardTitle(b, 'Customer Journey Map', 'Persona: Maya, 34, booking her first home cleaning service', '#14b8a6');
  const stages = ['Discover', 'Compare', 'Book', 'Service day', 'Follow-up'];
  const rows: Array<[string, string]> = [['Actions', '#f0fdfa'], ['Thoughts', '#f8fafc'], ['Feelings', '#ffffff'], ['Pain points', '#fef2f2'], ['Opportunities', '#f0fdf4']];
  const x0 = 640, colW = 640, y0 = 480;
  stages.forEach((s, i) => {
    b.rect(x0 + i * colW + 10, y0, colW - 20, 140, '#134e4a', { r: 24 });
    b.text(s, x0 + i * colW + 10, y0 + 40, colW - 20, { family: 'Gabarito', weight: 800, size: 56, color: '#ffffff' }, 'center');
  });
  const rh = [400, 400, 440, 400, 400];
  let y = y0 + 170;
  const content: Record<string, string[]> = {
    Actions: ['Searches “cleaner near me”', 'Reads reviews and prices', 'Picks a slot online', 'Lets the cleaner in', 'Gets a rating request'],
    Thoughts: ['“Can I trust a stranger at home?”', '“Why are prices so different?”', '“Hope this is easy.”', '“Wow, the oven is spotless.”', '“Should I book again?”'],
    'Pain points': ['Few verified reviews', 'Hidden extra fees', '', 'Cleaner arrived late', ''],
    Opportunities: ['Show vetting badges', 'Flat, upfront pricing', 'One-tap rebooking', 'Live arrival tracker', 'Loyalty discount'],
  };
  rows.forEach(([r, c], ri) => {
    b.rect(160, y, 3680, rh[ri]!, c, { r: 24 });
    b.text(r, 200, y + 40, 420, { family: 'Gabarito', weight: 800, size: 52, color: '#134e4a' });
    if (r === 'Feelings') {
      const ys = [0.5, 0.75, 0.35, 0.15, 0.4].map((v) => y + 60 + v * (rh[ri]! - 120));
      ys.forEach((py, i) => {
        const px = x0 + i * colW + colW / 2;
        if (i < ys.length - 1) conn(b, px, py, x0 + (i + 1) * colW + colW / 2, ys[i + 1]!, '#14b8a6', 10);
      });
      ys.forEach((py, i) => {
        const px = x0 + i * colW + colW / 2;
        b.circle(px, py, 56, '#ffffff');
        b.emoji(['😐', '😟', '🙂', '😄', '🙂'][i]!, px - 50, py - 50, 100);
      });
    } else {
      content[r]!.forEach((t, i) => {
        if (!t) return;
        sticky(b, x0 + i * colW + 50, y + 40, r === 'Pain points' ? '#fecaca' : r === 'Opportunities' ? '#bbf7d0' : r === 'Thoughts' ? '#fde68a' : '#99f6e4', t, colW - 100, rh[ri]! - 80, 46);
      });
    }
    y += rh[ri]! + 30;
  });
});

tpl('mw-wb-weekly-planner', 'Weekly Planner Board', WB, 'whiteboard', ['planner', 'weekly', 'schedule', 'productivity', 'organize'], (b) => {
  b.bg('#faf7f2');
  boardTitle(b, 'Week of 19 October 2026', 'Plan the week, then review it on Friday', '#e11d48');
  const days = ['Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat', 'Sun'];
  const x0 = 160, w = 380, gap = 20;
  days.forEach((d, i) => {
    const x = x0 + i * (w + gap);
    const weekend = i >= 5;
    b.rect(x, 460, w, 2380, weekend ? '#fff1f2' : '#ffffff', { r: 28, stroke: ['#ece6dc', 4] });
    b.text(d.toUpperCase(), x, 500, w, { family: 'Gabarito', weight: 800, size: 64, color: weekend ? '#be123c' : '#1f2328' }, 'center');
    b.text(String(19 + i), x, 580, w, { family: 'Inter', weight: 500, size: 44, color: '#4b5563' }, 'center');
    b.rect(x + 40, 660, w - 80, 4, '#ece6dc');
  });
  sticky(b, 200, 720, '#fde68a', 'Team kickoff 9:30', 300, 240, 40);
  sticky(b, 600, 720, '#bfdbfe', 'Dentist 2 pm', 300, 240, 40);
  sticky(b, 1000, 1020, '#bbf7d0', 'Gym + yoga', 300, 240, 40);
  sticky(b, 1800, 720, '#fbcfe8', 'Ship v2.3!', 300, 240, 40);
  sticky(b, 2200, 720, '#fde68a', 'Farmers market', 300, 240, 40);
  // side panel
  const sx = 3000;
  b.rect(sx, 460, 840, 1120, '#1f2328', { r: 32 });
  b.text('Top 3 priorities', sx + 60, 510, 720, { family: 'Gabarito', weight: 800, size: 64, color: '#ffffff' });
  ['Finalise Q4 budget', 'Interview two designers', 'Draft launch email'].forEach((t, i) => {
    const y = 660 + i * 280;
    b.circle(sx + 100, y + 40, 40, '#e11d48');
    b.text(String(i + 1), sx + 60, y + 12, 80, { family: 'Gabarito', weight: 800, size: 46, color: '#ffffff' }, 'center');
    b.text(t, sx + 170, y + 10, 620, { family: 'Inter', weight: 500, size: 48, color: '#f3f4f6' }, 'start', { lh: 1.25 });
    b.rect(sx + 60, y + 200, 720, 2, '#3b3f45');
  });
  b.rect(sx, 1640, 840, 1200, '#ffffff', { r: 32, stroke: ['#ece6dc', 4] });
  b.text('Habits', sx + 60, 1690, 720, { family: 'Gabarito', weight: 800, size: 64, color: '#1f2328' });
  ['Read 20 min', 'Walk 8k steps', 'No phone after 10'].forEach((t, i) => {
    const y = 1840 + i * 200;
    b.text(t, sx + 60, y, 720, { family: 'Inter', weight: 500, size: 42, color: '#374151' });
    for (let k = 0; k < 7; k++) b.box(sx + 60 + k * 104, y + 70, 76, 76, '#c9c2b6', 4, 14);
  });
  b.text('Notes', sx + 60, 2440, 720, { family: 'Gabarito', weight: 800, size: 64, color: '#1f2328' });
  for (let k = 0; k < 3; k++) b.rect(sx + 60, 2600 + k * 80, 720, 3, '#e5dfd4');
});

// ================================================================== WEBSITES (1366 × 768 per section)
/** Pinned CC0 photos (shared pool, see photos.json). */
const PH = {
  portraitA: '02e2d1b6-e8b3-4a90-a171-92c41fab7715', portraitB: 'd767e7a0-1e33-4a8e-8605-14617c044085', portraitC: 'eeee05d0-0c4a-4d31-a6d9-9016e0fbb943', portraitD: 'c50a527d-f1f1-4e21-857f-ac286ca121b2',
  kitchen: 'e7830194-beba-4240-b618-f635df7d6e63', salad: 'cd216b53-b444-4638-a6d7-ab5809ecc6c4', cookies: 'f553f065-19d1-44f9-9526-6dbceb3b07d4',
  laptop: '6adcab40-54a1-4192-ada8-96968da73762', desk: '95674b79-66e1-4c1d-a7d5-0b928fd94825', interior: '35ece517-cd3a-458b-8470-bda517c66b4b',
  rose: '8eb53a8a-2a32-4f2e-b3da-ffa99dab9f53', vasePlant: 'a017e633-3048-4971-92d5-c8215d68c0a3', succulent: 'ea608729-87ec-4e92-82d0-aff691ea4d57', vases: '832fea9c-9abb-4e20-a17e-0df4b07ef3a3',
};
const WEB = 'Websites';
/** Pill / rounded button with centred label. */
function btn(b: B, x: number, y: number, w: number, h: number, label: string, bgc: Paint | string, fg: string, family: string, o: { r?: number; size?: number; stroke?: string } = {}) {
  if (bgc === 'none') b.box(x, y, w, h, o.stroke ?? fg, 2, o.r ?? h / 2);
  else b.rect(x, y, w, h, bgc, { r: o.r ?? h / 2 });
  const size = o.size ?? 16;
  b.text(label, x, y + (h - size * 1.3) / 2, w, { family, weight: 700, size, color: fg }, 'center', { role: 'cta' });
  return b;
}
type Paint = Parameters<B['bg']>[0];
/** Top navigation bar. */
function nav(b: B, logo: string, links: string[], o: { family: string; logoFamily?: string; color: string; muted: string; cta?: [string, Paint, string]; y?: number; logoSize?: number; mark?: string }) {
  const y = o.y ?? 36;
  if (o.mark) { b.circle(94, y + 17, 14, o.mark); b.circle(94, y + 17, 6, '#ffffff'); }
  b.text(logo, o.mark ? 118 : 80, y, 360, { family: o.logoFamily ?? o.family, weight: 800, size: o.logoSize ?? 26, color: o.color }, 'start', { role: 'logo' });
  let x = o.cta ? 1286 - 150 - 40 : 1286;
  for (const l of [...links].reverse()) {
    const w = l.length * 10 + 16;
    x -= w;
    b.text(l, x, y + 6, w, { family: o.family, weight: 500, size: 16, color: o.muted }, 'center');
    x -= 28;
  }
  if (o.cta) btn(b, 1286 - 150, y - 4, 150, 44, o.cta[0], o.cta[1], o.cta[2], o.family, { size: 15 });
}

tpl('mw-web-app-tempo', 'Focus App Landing Page', WEB, 'website', ['app', 'landing page', 'saas', 'startup', 'productivity'], (b) => {
  const F = 'Plus Jakarta Sans', ink = '#16123a', muted = '#4a4668', primary = '#5b4bff', soft = '#f5f3ff';
  titles(b, ['Hero', 'Features', 'Testimonials', 'Pricing', 'Get started']);
  // 1 · hero
  b.page(0).bg(soft);
  b.circle(1060, 400, 330, '#e6e1ff');
  b.circle(1240, 160, 90, '#ffe1d6');
  nav(b, 'tempo', ['Features', 'Reviews', 'Pricing'], { family: F, color: ink, muted, mark: primary, cta: ['Get the app', primary, '#ffffff'] });
  b.rect(80, 170, 214, 36, '#ffffff', { r: 18 });
  b.text('NEW · Focus rooms for teams', 80, 179, 214, { family: F, weight: 700, size: 13, color: primary }, 'center');
  b.text('Deep work, one tempo at a time.', 80, 230, 600, { family: F, weight: 800, size: 64, color: ink, tracking: -20 }, 'start', { role: 'headline', lh: 1.05 });
  b.text('Tempo blends a gentle focus timer, ambient soundscapes and honest stats so you finish what matters — without burning out.', 80, 400, 520, { family: F, size: 19, color: muted }, 'start', { lh: 1.55 });
  btn(b, 80, 524, 200, 56, 'Download for iOS', ink, '#ffffff', F);
  btn(b, 296, 524, 200, 56, 'Get it on Android', 'none', ink, F, { stroke: ink });
  b.text('★★★★★  4.9 average from 12,000+ reviews', 80, 610, 520, { family: F, weight: 600, size: 15, color: muted });
  // phone mockup
  b.rect(920, 110, 290, 590, ink, { r: 46, shadow: [0, 30, 60, '#16123a40'] });
  b.rect(934, 124, 262, 562, '#ffffff', { r: 36 });
  b.rect(1025, 138, 80, 22, ink, { r: 11 });
  b.text('Writing session', 934, 196, 262, { family: F, weight: 700, size: 16, color: ink }, 'center');
  b.shape('ring', 975, 250, 180, 180, primary, { params: { thickness: 0.12 } });
  b.circle(1065, 340, 76, '#ffffff');
  b.circle(1065, 261, 14, '#ffffff');
  b.circle(1065, 261, 8, '#ff7a59');
  b.text('18:42', 934, 312, 262, { family: F, weight: 800, size: 40, color: ink }, 'center');
  b.text('of 25:00', 934, 362, 262, { family: F, size: 14, color: muted }, 'center');
  b.rect(964, 470, 202, 54, primary, { r: 27 });
  b.text('Pause', 964, 486, 202, { family: F, weight: 700, size: 16, color: '#ffffff' }, 'center');
  b.rect(964, 544, 202, 104, soft, { r: 18 });
  b.text('Today · 3 sessions\n1 h 15 min focused', 984, 562, 180, { family: F, weight: 600, size: 14, color: ink }, 'start', { lh: 1.5 });
  // 2 · features
  b.page(1).bg('#ffffff');
  b.text('FEATURES', 80, 120, 1206, { family: F, weight: 800, size: 14, color: primary, tracking: 300 }, 'center');
  b.text('Everything you need to focus — nothing you don’t', 133, 152, 1100, { family: F, weight: 800, size: 42, color: ink, tracking: -10 }, 'center', { role: 'headline', lh: 1.15 });
  const feats: Array<[string, string, string, string]> = [
    ['⏱️', 'Flexible timers', 'Pomodoro, 52/17 or your own rhythm. Tempo adapts when you run long.', '#f5f3ff'],
    ['🎧', 'Ambient sound', 'Rain, café murmur and brown noise — mixed to keep you in the zone.', '#fff4ef'],
    ['📊', 'Honest insights', 'See when you focus best and where your week really went.', '#eefbf4'],
  ];
  feats.forEach(([em, t, d, c], i) => {
    const x = 80 + i * 410;
    b.rect(x, 280, 386, 330, c, { r: 28 });
    b.rect(x + 36, 316, 84, 84, '#ffffff', { r: 22 });
    b.emoji(em, x + 52, 332, 52);
    b.text(t, x + 36, 436, 320, { family: F, weight: 800, size: 26, color: ink });
    b.text(d, x + 36, 486, 314, { family: F, size: 17, color: muted }, 'start', { lh: 1.55 });
  });
  // 3 · testimonials
  b.page(2).bg(ink);
  b.text('Loved by 400,000 makers', 80, 130, 1206, { family: F, weight: 800, size: 46, color: '#ffffff', tracking: -10 }, 'center', { role: 'headline' });
  b.text('Writers, developers and students use Tempo to protect their best hours.', 80, 204, 1206, { family: F, size: 19, color: '#c9c5ea' }, 'center');
  const quotes: Array<[string, string, string]> = [
    ['“I finished my thesis two weeks early. The soft chime beats any alarm.”', 'Lena Hart', 'PhD student'],
    ['“Our whole team runs focus rooms on Tuesdays. Meetings dropped by a third.”', 'Danielle Brooks', 'Engineering lead'],
    ['“The weekly report is the only stats screen I actually look forward to.”', 'Karen Olsen', 'Freelance writer'],
  ];
  const avatars = [PH.portraitB, PH.portraitC, PH.portraitD];
  quotes.forEach(([q, n, r], i) => {
    const x = 80 + i * 410;
    b.rect(x, 290, 386, 330, '#241f52', { r: 28 });
    b.text('★★★★★', x + 36, 326, 300, { family: F, size: 20, color: '#ffb84d' });
    b.text(q, x + 36, 370, 314, { family: F, weight: 500, size: 20, color: '#ffffff' }, 'start', { lh: 1.5 });
    b.photo(avatars[i]!, x + 36, 532, 56, 56, { clip: 'ellipse', focus: [0.5, 0.3] });
    b.text(n, x + 108, 536, 240, { family: F, weight: 700, size: 17, color: '#ffffff' });
    b.text(r, x + 108, 562, 240, { family: F, size: 15, color: '#c9c5ea' });
  });
  // 4 · pricing
  b.page(3).bg(soft);
  b.text('Simple pricing', 80, 80, 1206, { family: F, weight: 800, size: 46, color: ink, tracking: -10 }, 'center', { role: 'headline' });
  b.text('Start free. Upgrade when Tempo becomes a habit.', 80, 150, 1206, { family: F, size: 19, color: muted }, 'center');
  const plans: Array<[string, string, string, string[], boolean]> = [
    ['Free', '$0', 'forever', ['Classic timer', '3 soundscapes', '7-day history'], false],
    ['Pro', '$6', 'per month', ['Custom rhythms', 'All 40 soundscapes', 'Unlimited insights', 'Calendar sync'], true],
    ['Teams', '$12', 'per user / month', ['Everything in Pro', 'Shared focus rooms', 'Admin & SSO'], false],
  ];
  plans.forEach(([name, price, per, list, hot], i) => {
    const x = 128 + i * 380;
    const bg = hot ? primary : '#ffffff', fg = hot ? '#ffffff' : ink, mu = hot ? '#ffffff' : muted;
    b.rect(x, hot ? 220 : 240, 350, hot ? 480 : 440, bg, { r: 28, shadow: hot ? [0, 20, 40, '#5b4bff55'] : undefined });
    if (hot) { b.rect(x + 210, 244, 110, 30, '#ffffff', { r: 15 }); b.text('POPULAR', x + 210, 251, 110, { family: F, weight: 800, size: 12, color: primary, tracking: 150 }, 'center'); }
    const y = hot ? 250 : 270;
    b.text(name, x + 36, y, 200, { family: F, weight: 700, size: 22, color: fg });
    b.text(price, x + 36, y + 44, 160, { family: F, weight: 800, size: 54, color: fg });
    b.text(per, x + 36, y + 120, 280, { family: F, size: 15, color: mu });
    b.text(list.map((l) => `✓  ${l}`).join('\n'), x + 36, y + 166, 280, { family: F, weight: 500, size: 16, color: fg }, 'start', { lh: 1.9 });
    btn(b, x + 36, (hot ? 220 + 480 : 240 + 440) - 84, 278, 52, hot ? 'Start 14-day trial' : 'Choose plan', hot ? '#ffffff' : ink, hot ? primary : '#ffffff', F);
  });
  // 5 · CTA + footer
  b.page(4).bg('#ffffff');
  b.rect(80, 70, 1206, 380, lin(135, '#4a3aff', '#8a6bff'), { r: 36 });
  b.circle(1180, 120, 140, '#ffffff', { opacity: 0.12 });
  b.text('Ready to find your tempo?', 140, 150, 800, { family: F, weight: 800, size: 52, color: '#ffffff', tracking: -10 }, 'start', { role: 'headline', lh: 1.1 });
  b.text('Free on iPhone, iPad, Mac and Android. No credit card needed.', 140, 230, 700, { family: F, size: 20, color: '#ffffff' });
  btn(b, 140, 320, 200, 56, 'Download free', '#ffffff', primary, F);
  btn(b, 356, 320, 180, 56, 'Talk to sales', 'none', '#ffffff', F, { stroke: '#ffffff' });
  const cols: Array<[string, string[]]> = [['Product', ['Features', 'Pricing', 'Changelog']], ['Company', ['About', 'Careers', 'Press']], ['Support', ['Help centre', 'Contact', 'Privacy']]];
  b.circle(94, 547, 14, primary);
  b.circle(94, 547, 6, '#ffffff');
  b.text('tempo', 118, 530, 300, { family: F, weight: 800, size: 26, color: ink }, 'start', { role: 'logo' });
  b.text('Focus, made gentle.', 80, 576, 300, { family: F, size: 16, color: muted });
  cols.forEach(([h, ls], i) => {
    b.text(h, 700 + i * 200, 530, 180, { family: F, weight: 800, size: 15, color: ink });
    b.text(ls.join('\n'), 700 + i * 200, 566, 180, { family: F, size: 15, color: muted }, 'start', { lh: 1.9 });
  });
  b.rect(80, 690, 1206, 1, '#e7e5f0');
  b.text('© 2026 Tempo Labs Inc. · hello@tempo.example.com', 80, 712, 800, { family: F, size: 14, color: muted });
}, 5);

tpl('mw-web-portfolio-mira', 'Designer Portfolio Site', WEB, 'website', ['portfolio', 'designer', 'personal', 'minimal', 'creative'], (b) => {
  const S = 'Instrument Serif', F = 'Inter', ink = '#141414', muted = '#55524c', paper = '#f2efe9', accent = '#b23f1d';
  titles(b, ['Home', 'Selected work', 'About', 'Contact']);
  // 1 · hero
  b.page(0).bg(paper);
  nav(b, 'Mira Okafor', ['Work', 'About', 'Contact'], { family: F, logoFamily: S, color: ink, muted, logoSize: 30 });
  b.rect(80, 150, 230, 36, '#ffffff', { r: 18 });
  b.circle(102, 168, 6, '#2f9e44');
  b.text('Available from January', 114, 159, 190, { family: F, weight: 500, size: 14, color: ink });
  b.text('Brand & product designer making calm, useful things.', 80, 210, 760, { family: S, size: 88, color: ink }, 'start', { role: 'headline', lh: 1.0 });
  b.text('Ten years shaping identities and interfaces for climate startups, museums and independent publishers. Based in Lisbon, working everywhere.', 80, 500, 560, { family: F, size: 18, color: muted }, 'start', { lh: 1.6 });
  b.text('See selected work  →', 80, 640, 300, { family: F, weight: 600, size: 17, color: accent }, 'start', { role: 'cta' });
  b.photo(PH.portraitA, 900, 130, 386, 520, { clip: 'rounded', focus: [0.4, 0.4] });
  b.shape('star', 846, 560, 110, 110, accent, { params: { points: 8, innerRatio: 0.55 } });
  // 2 · work
  b.page(1).bg(paper);
  b.text('Selected work', 80, 70, 600, { family: S, size: 60, color: ink }, 'start', { role: 'headline' });
  b.text('2022 – 2026', 986, 96, 300, { family: F, weight: 500, size: 16, color: muted }, 'end');
  const work: Array<[string, string, string]> = [['Tidewater Energy', 'Brand identity · 2026', PH.laptop], ['Casa Verde', 'Interior brand · 2025', PH.vasePlant], ['Fieldnote', 'iOS app · 2024', PH.interior], ['Paper & Rose Press', 'Editorial system · 2023', PH.rose]];
  work.forEach(([t, d, ph], i) => {
    const x = 80 + i * 306;
    b.photo(ph, x, 160, 288, 400, { filter: 'matte', filterAmount: 0.6 });
    b.text(t, x, 580, 288, { family: S, size: 28, color: ink });
    b.text(d, x, 622, 288, { family: F, size: 15, color: muted });
  });
  b.line(80, 700, 1206, '#d6d0c4', 1);
  // 3 · about
  b.page(2).bg('#ffffff');
  b.photo(PH.desk, 80, 80, 460, 608, { focus: [0.45, 0.5] });
  b.text('About', 620, 90, 600, { family: F, weight: 600, size: 14, color: accent, tracking: 300 });
  b.text('I believe good design feels inevitable — like it could never have been any other way.', 620, 124, 640, { family: S, size: 40, color: ink }, 'start', { role: 'headline', lh: 1.15 });
  b.text('Before going independent I led design at two early-stage startups and taught typography at ESAD. I work best in small teams, close to the people who build.', 620, 256, 600, { family: F, size: 17, color: muted }, 'start', { lh: 1.6 });
  const services: Array<[string, string]> = [['Brand identity', 'Strategy, naming, logo and visual systems'], ['Product design', 'Research, UX flows and polished UI'], ['Design systems', 'Tokens, components and documentation']];
  services.forEach(([t, d], i) => {
    const y = 420 + i * 80;
    b.line(620, y, 666, '#e6e1d8', 1);
    b.text(t, 620, y + 22, 260, { family: F, weight: 600, size: 18, color: ink });
    b.text(d, 880, y + 24, 406, { family: F, size: 16, color: muted });
  });
  // 4 · contact
  b.page(3).bg(ink);
  b.text('Have a project in mind?', 80, 140, 900, { family: F, weight: 500, size: 18, color: '#bdb8ae' });
  b.text('Let’s make something good together.', 80, 180, 1000, { family: S, size: 96, color: paper }, 'start', { role: 'headline', lh: 1.0 });
  b.text('hello@miraokafor.example', 80, 430, 700, { family: S, italic: true, size: 44, color: '#f0a487' }, 'start', { role: 'cta' });
  b.line(80, 500, 640, '#f0a487', 2);
  b.text('Instagram\nDribbble\nLinkedIn', 1000, 400, 286, { family: F, weight: 500, size: 18, color: paper }, 'end', { lh: 1.9 });
  b.line(80, 660, 1206, '#3a3833', 1);
  b.text('© 2026 Mira Okafor · Lisbon, Portugal', 80, 690, 600, { family: F, size: 14, color: '#bdb8ae' });
  b.text('Back to top ↑', 986, 690, 300, { family: F, size: 14, color: '#bdb8ae' }, 'end');
}, 4);

tpl('mw-web-restaurant-lume', 'Restaurant Website', WEB, 'website', ['restaurant', 'food', 'italian', 'menu', 'dining'], (b) => {
  const S = 'Cormorant Garamond', F = 'Montserrat', dark = '#1b1512', cream = '#f3e9dc', gold = '#d6a756', muted = '#c9bba8';
  titles(b, ['Home', 'Our story', 'Menu', 'Gallery', 'Reservations']);
  // 1 · hero
  b.page(0).bg(dark);
  b.bgPhoto(PH.kitchen, { filter: 'warm', filterAmount: 0.5 });
  b.rect(0, 0, 1366, 768, lin(180, '#1b1512b3', '#1b1512f0'));
  nav(b, 'OSTERIA LUME', ['Menu', 'Story', 'Gallery', 'Visit'], { family: F, logoFamily: S, color: cream, muted: cream, cta: ['Book a table', gold, dark], logoSize: 28 });
  b.text('EST. 2019 · BROOKLYN', 80, 250, 1206, { family: F, weight: 600, size: 14, color: gold, tracking: 400 }, 'center');
  b.text('Handmade pasta,\nnatural wine, slow evenings.', 133, 290, 1100, { family: S, weight: 500, italic: true, size: 84, color: cream }, 'center', { role: 'headline', lh: 1.0 });
  btn(b, 503, 520, 170, 54, 'Reserve', gold, dark, F, { r: 4 });
  btn(b, 693, 520, 170, 54, 'View menu', 'none', cream, F, { r: 4, stroke: cream });
  b.text('Tue – Sun · 5 pm – 11 pm · 214 Wythe Ave', 80, 690, 1206, { family: F, size: 14, color: muted }, 'center');
  // 2 · story
  b.page(1).bg(cream);
  b.text('OUR STORY', 80, 140, 500, { family: F, weight: 600, size: 14, color: '#8a5a1c', tracking: 400 });
  b.text('A small kitchen with a long memory', 80, 176, 520, { family: S, weight: 600, size: 56, color: dark }, 'start', { role: 'headline', lh: 1.05 });
  b.text('Chef Giulia Ferro rolls every sheet of pasta by hand each morning, following recipes from her grandmother’s trattoria in Bologna. Our wine list favours small growers who farm without shortcuts.', 80, 330, 500, { family: F, size: 16, color: '#4a3b30' }, 'start', { lh: 1.75 });
  b.text('— Giulia Ferro, chef & owner', 80, 520, 500, { family: S, italic: true, size: 24, color: '#8a5a1c' });
  b.photo(PH.salad, 680, 80, 300, 608, { filter: 'warm', filterAmount: 0.4 });
  b.photo(PH.cookies, 1000, 80, 286, 290, { filter: 'warm', filterAmount: 0.4 });
  b.rect(1000, 398, 286, 290, dark);
  b.text('12', 1000, 450, 286, { family: S, weight: 600, size: 96, color: gold }, 'center');
  b.text('fresh pastas\non the menu daily', 1000, 580, 286, { family: F, weight: 500, size: 15, color: cream }, 'center', { lh: 1.5 });
  // 3 · menu
  b.page(2).bg(dark);
  b.text('Menu', 80, 64, 1206, { family: S, weight: 500, italic: true, size: 64, color: cream }, 'center', { role: 'headline' });
  b.text('Seasonal · changes weekly', 80, 144, 1206, { family: F, weight: 500, size: 14, color: gold, tracking: 300 }, 'center');
  const menu: Array<[string, Array<[string, string, string]>]> = [
    ['Antipasti', [['Burrata', 'roasted grapes, saba', '16'], ['Carpaccio', 'beef, rocket, parmigiano', '18'], ['Fritto misto', 'squid, lemon aioli', '17']]],
    ['Primi', [['Tagliatelle al ragù', 'eight-hour Bolognese', '26'], ['Cacio e pepe', 'tonnarelli, pecorino', '22'], ['Agnolotti', 'pumpkin, sage butter', '24']]],
    ['Secondi', [['Branzino', 'salsa verde, fennel', '34'], ['Pollo al mattone', 'charred lemon', '29'], ['Tiramisù', 'for the table', '12']]],
  ];
  menu.forEach(([h, items], i) => {
    const x = 80 + i * 410;
    b.text(h.toUpperCase(), x, 220, 380, { family: F, weight: 700, size: 15, color: gold, tracking: 300 });
    b.line(x, 256, 380, '#4a3b30', 1);
    items.forEach(([n, d, p], k) => {
      const y = 286 + k * 120;
      b.text(n, x, y, 300, { family: S, weight: 600, size: 30, color: cream });
      b.text(p, x + 300, y + 2, 80, { family: S, weight: 600, size: 28, color: gold }, 'end');
      b.text(d, x, y + 44, 300, { family: F, size: 15, color: muted });
    });
  });
  b.text('Please tell your server about any allergies.', 80, 680, 1206, { family: F, size: 14, color: muted }, 'center');
  // 4 · gallery
  b.page(3).bg(cream);
  b.text('From our table', 80, 60, 1206, { family: S, weight: 500, italic: true, size: 56, color: dark }, 'center', { role: 'headline' });
  b.grid([PH.kitchen, PH.salad, PH.cookies], 80, 150, 1206, 540, 'big-left', { gap: 14 });
  // 5 · reservations
  b.page(4).bg(dark);
  b.text('Reserve your table', 80, 110, 600, { family: S, weight: 500, italic: true, size: 64, color: cream }, 'start', { role: 'headline' });
  b.text('Walk-ins welcome at the bar. Parties of seven or more, please call ahead.', 80, 200, 520, { family: F, size: 16, color: muted }, 'start', { lh: 1.6 });
  const info: Array<[string, string]> = [['HOURS', 'Tue – Thu  5 pm – 10 pm\nFri – Sun  5 pm – 11 pm'], ['ADDRESS', '214 Wythe Avenue\nBrooklyn, NY 11249'], ['CONTACT', '(555) 014-2290\nciao@osterialume.example']];
  info.forEach(([h, t], i) => {
    const y = 300 + i * 110;
    b.text(h, 80, y, 200, { family: F, weight: 700, size: 13, color: gold, tracking: 300 });
    b.text(t, 80, y + 26, 500, { family: F, size: 16, color: cream }, 'start', { lh: 1.55 });
  });
  b.frame(720, 110, 566, 480, 'rect', '#3a2c24');
  b.text('Map', 720, 330, 566, { family: F, size: 16, color: muted }, 'center');
  btn(b, 80, 640, 220, 54, 'Book online', gold, dark, F, { r: 4 });
  b.text('© 2026 Osteria Lume', 986, 656, 300, { family: F, size: 14, color: muted }, 'end');
}, 5);

tpl('mw-web-conference-frontier', 'Tech Conference Website', WEB, 'website', ['conference', 'event', 'tech', 'summit', 'tickets'], (b) => {
  const D = 'Unbounded', F = 'Inter', bgc = '#0b0d1f', lime = '#c8ff4d', violet = '#8b7bff', text2 = '#b9bdd8', card = '#151936';
  titles(b, ['Home', 'Speakers', 'Schedule', 'Tickets', 'Venue']);
  // 1 · hero
  b.page(0).bg(bgc);
  b.circle(1150, 620, 420, rad('#3b2f9e', '#0b0d1f'));
  nav(b, 'FRONTIER/26', ['Speakers', 'Schedule', 'Venue'], { family: F, logoFamily: D, color: '#ffffff', muted: text2, cta: ['Get tickets', lime, bgc], logoSize: 22 });
  b.text('MAY 12 – 14, 2027 · AMSTERDAM', 80, 180, 900, { family: F, weight: 700, size: 16, color: lime, tracking: 300 });
  b.text('The future\nis built here.', 80, 220, 900, { family: D, weight: 800, size: 92, color: '#ffffff', tracking: -20 }, 'start', { role: 'headline', lh: 1.0 });
  b.text('Three days, 60 speakers and 2,000 builders exploring AI, robotics and the next wave of the open web.', 80, 450, 560, { family: F, size: 19, color: text2 }, 'start', { lh: 1.55 });
  btn(b, 80, 570, 220, 58, 'Register now', lime, bgc, F);
  btn(b, 316, 570, 200, 58, 'Watch 2026 talks', 'none', '#ffffff', F, { stroke: '#ffffff' });
  const cd: Array<[string, string]> = [['214', 'days'], ['08', 'hours'], ['36', 'mins']];
  cd.forEach(([n, l], i) => {
    const x = 900 + i * 132;
    b.rect(x, 470, 116, 120, '#1a1d3a', { r: 16, stroke: ['#3a3f6e', 1] });
    b.text(n, x, 488, 116, { family: D, weight: 700, size: 40, color: '#ffffff' }, 'center');
    b.text(l, x, 548, 116, { family: F, size: 14, color: text2 }, 'center');
  });
  // 2 · speakers
  b.page(1).bg(bgc);
  b.text('Speakers', 80, 70, 700, { family: D, weight: 800, size: 48, color: '#ffffff' }, 'start', { role: 'headline' });
  b.text('60+ researchers, founders and makers', 80, 140, 700, { family: F, size: 18, color: text2 });
  btn(b, 1086, 80, 200, 48, 'All speakers →', 'none', lime, F, { stroke: lime, size: 15 });
  const sp: Array<[string, string]> = [['Dr. Amara Osei', 'Robotics Lab, Delft'], ['Kenji Watanabe', 'Founder, Loomwork'], ['Sofia Lindqvist', 'CTO, Northwave AI'], ['Rafael Duarte', 'Open-source maintainer'], ['Priya Natarajan', 'Head of ML, Quillon'], ['Tom Becker', 'Partner, Atlas Ventures'], ['Lucía Moreno', 'Climate data researcher'], ['Noah Klein', 'Hardware hacker']];
  sp.forEach(([n, r], i) => {
    const x = 80 + (i % 4) * 306, y = 210 + Math.floor(i / 4) * 270;
    b.frame(x, y, 140, 140, 'ellipse', i % 2 ? '#2a2f5c' : '#3b2f7a');
    b.circle(x + 126, y + 122, 14, i % 3 ? violet : lime);
    b.text(n, x, y + 158, 280, { family: F, weight: 700, size: 18, color: '#ffffff' });
    b.text(r, x, y + 186, 280, { family: F, size: 15, color: text2 });
  });
  // 3 · schedule
  b.page(2).bg('#f4f5fb');
  b.text('Day one at a glance', 80, 70, 800, { family: D, weight: 800, size: 44, color: bgc }, 'start', { role: 'headline' });
  ['Day 1', 'Day 2', 'Day 3'].forEach((d, i) => btn(b, 900 + i * 132, 76, 120, 44, d, i === 0 ? bgc : '#ffffff', i === 0 ? '#ffffff' : bgc, F, { size: 15 }));
  const sched: Array<[string, string, string, string]> = [
    ['09:00', 'Opening keynote: Machines that learn by doing', 'Dr. Amara Osei', 'Main stage'],
    ['10:30', 'Small models, big impact', 'Priya Natarajan', 'Main stage'],
    ['12:00', 'Lunch & demo floor', 'All attendees', 'Hall B'],
    ['13:30', 'Funding deep tech in 2027', 'Tom Becker & panel', 'Stage 2'],
    ['15:00', 'Workshop: build a robot arm in 90 minutes', 'Noah Klein', 'Lab'],
    ['17:30', 'Canal-side welcome drinks', 'Everyone', 'Westerpark'],
  ];
  sched.forEach(([t, title, who, room], i) => {
    const y = 160 + i * 88;
    b.rect(80, y, 1206, 76, '#ffffff', { r: 14 });
    b.text(t, 110, y + 24, 100, { family: D, weight: 700, size: 20, color: '#5b4bdb' });
    b.text(title, 240, y + 14, 640, { family: F, weight: 700, size: 18, color: bgc });
    b.text(who, 240, y + 42, 640, { family: F, size: 14, color: '#4b5070' });
    b.rect(1116, y + 22, 140, 32, '#eef0ff', { r: 16 });
    b.text(room, 1116, y + 29, 140, { family: F, weight: 600, size: 13, color: '#3d3a99' }, 'center');
  });
  // 4 · tickets
  b.page(3).bg(bgc);
  b.text('Tickets', 80, 70, 1206, { family: D, weight: 800, size: 48, color: '#ffffff' }, 'center', { role: 'headline' });
  b.text('Early-bird pricing ends 31 January 2027', 80, 140, 1206, { family: F, size: 18, color: lime }, 'center');
  const tix: Array<[string, string, string[], boolean]> = [
    ['Student', '€149', ['All talks', 'Demo floor', 'Student ID required'], false],
    ['Builder', '€449', ['All talks & workshops', 'Lunch every day', 'Welcome drinks', 'Recordings'], true],
    ['Team of 5', '€1,890', ['Five Builder passes', 'Reserved seating', 'Private meetup room'], false],
  ];
  tix.forEach(([n, p, list, hot], i) => {
    const x = 128 + i * 380;
    b.rect(x, 220, 350, 420, hot ? lime : card, { r: 24, stroke: hot ? undefined : ['#2a2f5c', 1] });
    const fg = hot ? bgc : '#ffffff', mu = hot ? '#2b3300' : text2;
    b.text(n.toUpperCase(), x + 36, 256, 280, { family: F, weight: 700, size: 14, color: mu, tracking: 300 });
    b.text(p, x + 36, 290, 280, { family: D, weight: 800, size: 52, color: fg });
    b.text(list.map((l) => `+  ${l}`).join('\n'), x + 36, 386, 280, { family: F, weight: 500, size: 16, color: fg }, 'start', { lh: 1.9 });
    btn(b, x + 36, 556, 278, 52, 'Buy ticket', hot ? bgc : '#ffffff', hot ? lime : bgc, F);
  });
  // 5 · venue + footer
  b.page(4).bg(bgc);
  b.photo('picsum:192', 80, 80, 600, 460, { clip: 'rounded', duotone: ['#0b0d1f', '#8b7bff'] });
  b.text('THE VENUE', 740, 100, 500, { family: F, weight: 700, size: 14, color: lime, tracking: 300 });
  b.text('Gashouder, Westergas', 740, 132, 546, { family: D, weight: 800, size: 30, color: '#ffffff' }, 'start', { role: 'headline', lh: 1.15 });
  b.text('A restored 1902 gasholder turned event hall, ten minutes by tram from Amsterdam Centraal. Step-free access throughout.', 740, 200, 520, { family: F, size: 17, color: text2 }, 'start', { lh: 1.6 });
  b.text('Klönneplein 1, 1014 DD Amsterdam', 740, 330, 520, { family: F, weight: 600, size: 16, color: '#ffffff' });
  btn(b, 740, 380, 210, 50, 'Plan your trip', 'none', '#ffffff', F, { stroke: '#ffffff', size: 15 });
  b.text('PARTNERS', 80, 590, 300, { family: F, weight: 700, size: 13, color: text2, tracking: 300 });
  ['NORTHWAVE', 'loomwork', 'Quillon', 'ATLAS', 'helio·labs'].forEach((s, i) => b.text(s, 80 + i * 245, 624, 220, { family: i % 2 ? F : D, weight: 700, size: 22, color: '#8a8fb5' }));
  b.line(80, 690, 1206, '#22264a', 1);
  b.text('© 2027 Frontier Summit · hello@frontier.example.com', 80, 712, 800, { family: F, size: 14, color: text2 });
}, 5);

tpl('mw-web-services-greenleaf', 'Landscaping Business Site', WEB, 'website', ['small business', 'services', 'landscaping', 'local', 'garden'], (b) => {
  const D = 'Bricolage Grotesque', F = 'DM Sans', green = '#1e4d2b', lime = '#a3d977', cream = '#f6f4ec', ink = '#1b2a1f', muted = '#4b5a4f';
  titles(b, ['Home', 'Services', 'How it works', 'Reviews', 'Contact']);
  // 1 · hero
  b.page(0).bg(cream);
  nav(b, 'greenleaf', ['Services', 'Process', 'Reviews'], { family: F, logoFamily: D, color: green, muted, cta: ['Free quote', green, '#ffffff'] });
  b.text('LANDSCAPING · PORTLAND, OR', 80, 180, 560, { family: F, weight: 700, size: 14, color: '#3d7a4a', tracking: 300 });
  b.text('Gardens that grow with you.', 80, 214, 600, { family: D, weight: 800, size: 72, color: green, tracking: -20 }, 'start', { role: 'headline', lh: 1.0 });
  b.text('Design, planting and year-round care for homes and small businesses. Family-run since 2011.', 80, 400, 520, { family: F, size: 19, color: muted }, 'start', { lh: 1.55 });
  btn(b, 80, 510, 220, 58, 'Get a free quote', green, '#ffffff', F);
  b.text('or call (555) 017-3344', 320, 528, 300, { family: F, weight: 600, size: 17, color: green });
  b.text('★ 4.9 on 310 reviews   ·   Licensed & insured', 80, 620, 600, { family: F, weight: 500, size: 15, color: muted });
  b.shape('blob', 720, 90, 600, 600, lime);
  b.photo(PH.succulent, 760, 130, 520, 520, { clip: 'ellipse', focus: [0.5, 0.6] });
  b.rect(700, 560, 220, 96, '#ffffff', { r: 20, shadow: [0, 12, 30, '#1e4d2b22'] });
  b.text('1,200+', 724, 576, 180, { family: D, weight: 800, size: 30, color: green });
  b.text('gardens cared for', 724, 618, 180, { family: F, size: 14, color: muted });
  // 2 · services
  b.page(1).bg('#ffffff');
  b.text('What we do', 80, 70, 1206, { family: D, weight: 800, size: 48, color: green }, 'center', { role: 'headline' });
  b.text('One crew for everything outside your door.', 80, 140, 1206, { family: F, size: 18, color: muted }, 'center');
  const svc: Array<[string, string, string]> = [
    ['🌿', 'Garden design', 'Plans and 3D sketches built around how you live.'],
    ['✂️', 'Lawn & hedge care', 'Weekly or monthly visits that keep things tidy.'],
    ['💧', 'Smart irrigation', 'Drip systems that cut water bills by up to 40%.'],
    ['🌸', 'Seasonal planting', 'Bulbs, beds and containers for every season.'],
    ['🪨', 'Patios & paths', 'Natural stone, gravel and reclaimed brick.'],
    ['🍂', 'Clean-ups', 'Spring and autumn resets, green waste removed.'],
  ];
  svc.forEach(([em, t, d], i) => {
    const x = 80 + (i % 3) * 410, y = 210 + Math.floor(i / 3) * 250;
    b.rect(x, y, 386, 226, cream, { r: 24 });
    b.circle(x + 66, y + 66, 36, '#ffffff');
    b.emoji(em, x + 44, y + 44, 44);
    b.text(t, x + 120, y + 50, 240, { family: D, weight: 700, size: 24, color: ink });
    b.text(d, x + 36, y + 124, 314, { family: F, size: 16, color: muted }, 'start', { lh: 1.55 });
  });
  // 3 · process
  b.page(2).bg(green);
  b.text('How it works', 80, 90, 600, { family: D, weight: 800, size: 52, color: '#ffffff' }, 'start', { role: 'headline' });
  b.text('From first call to first bloom in about three weeks.', 80, 166, 600, { family: F, size: 19, color: '#d4e6d2' });
  const steps: Array<[string, string]> = [['Free site visit', 'We walk the garden with you, measure up and listen.'], ['Design & quote', 'A clear plan and a fixed price within five days.'], ['We build, you relax', 'Our crew handles everything and leaves it spotless.']];
  steps.forEach(([t, d], i) => {
    const x = 80 + i * 410;
    b.text(`0${i + 1}`, x, 280, 120, { family: D, weight: 800, size: 64, color: lime });
    b.line(x, 380, 360, '#3f7a4c', 2);
    b.text(t, x, 410, 360, { family: D, weight: 700, size: 26, color: '#ffffff' });
    b.text(d, x, 458, 340, { family: F, size: 17, color: '#d4e6d2' }, 'start', { lh: 1.55 });
  });
  btn(b, 80, 620, 230, 56, 'Book a site visit', lime, green, F);
  // 4 · reviews
  b.page(3).bg(cream);
  b.text('Neighbours say it best', 80, 70, 700, { family: D, weight: 800, size: 48, color: green }, 'start', { role: 'headline' });
  b.photo('picsum:33', 80, 170, 290, 340, { clip: 'rounded', filter: 'fade', filterAmount: 0.6 });
  b.photo('picsum:17', 386, 170, 290, 340, { clip: 'rounded', filter: 'vivid', filterAmount: 0.5 });
  b.rect(96, 186, 80, 30, '#ffffff', { r: 15 });
  b.text('Before', 96, 192, 80, { family: F, weight: 700, size: 13, color: ink }, 'center');
  b.rect(402, 186, 80, 30, '#ffffff', { r: 15 });
  b.text('After', 402, 192, 80, { family: F, weight: 700, size: 13, color: ink }, 'center');
  const rev: Array<[string, string]> = [['“They turned a muddy slope into the best room of our house. On time, on budget.”', 'Hannah & Joel, Sellwood'], ['“Our café patio finally feels like a place people want to linger.”', 'Diego, Little Fern Café']];
  rev.forEach(([q, n], i) => {
    const y = 170 + i * 260;
    b.rect(726, y, 560, 236, '#ffffff', { r: 24 });
    b.text('★★★★★', 762, y + 32, 300, { family: F, size: 18, color: '#a86f00' });
    b.text(q, 762, y + 70, 490, { family: D, weight: 500, size: 22, color: ink }, 'start', { lh: 1.4 });
    b.text(n, 762, y + 180, 490, { family: F, weight: 600, size: 15, color: muted });
  });
  b.text('Sellwood backyard, spring 2026', 80, 530, 596, { family: F, size: 14, color: muted });
  // 5 · contact + footer
  b.page(4).bg('#ffffff');
  b.text('Get your free quote', 80, 80, 560, { family: D, weight: 800, size: 48, color: green }, 'start', { role: 'headline' });
  b.text('Tell us a little about your space and we’ll be in touch within one working day.', 80, 150, 500, { family: F, size: 18, color: muted }, 'start', { lh: 1.55 });
  const info: Array<[string, string]> = [['📞', '(555) 017-3344'], ['✉️', 'hello@greenleaf.example.com'], ['📍', '88 Alder Lane, Portland, OR']];
  info.forEach(([em, t], i) => {
    b.emoji(em, 80, 270 + i * 56, 30);
    b.text(t, 124, 272 + i * 56, 460, { family: F, weight: 500, size: 17, color: ink });
  });
  b.rect(700, 70, 586, 560, cream, { r: 28 });
  const fields = ['Your name', 'Email or phone', 'Postcode'];
  fields.forEach((f, i) => {
    const y = 110 + i * 96;
    b.text(f, 740, y, 500, { family: F, weight: 600, size: 14, color: ink });
    b.rect(740, y + 26, 506, 50, '#ffffff', { r: 12, stroke: ['#d8d4c4', 1] });
  });
  b.text('What can we help with?', 740, 398, 500, { family: F, weight: 600, size: 14, color: ink });
  b.rect(740, 424, 506, 100, '#ffffff', { r: 12, stroke: ['#d8d4c4', 1] });
  btn(b, 740, 552, 506, 54, 'Request my quote', green, '#ffffff', F, { r: 12 });
  b.line(80, 680, 1206, '#e6e3d6', 1);
  b.text('© 2026 Greenleaf Landscaping LLC · CCB #000000', 80, 704, 700, { family: F, size: 14, color: muted });
  b.text('Instagram · Facebook', 786, 704, 500, { family: F, size: 14, color: muted }, 'end');
}, 5);

// ================================================================== EMAILS (600 × 1600)
const EM = 'Emails';
/** Standard email footer. */
function emailFooter(b: B, y: number, brand: string, addr: string, color: string, family: string) {
  b.text(`${brand}\n${addr}\nYou’re receiving this because you signed up at ${brand.toLowerCase().replace(/[^a-z]/g, '')}.example.com.\nUnsubscribe · Update preferences · View in browser`, 40, y, 520, { family, size: 13, color }, 'center', { lh: 1.7 });
}

tpl('mw-email-product-update', 'Product Update Newsletter', EM, 'email', ['newsletter', 'product update', 'saas', 'announcement', 'app'], (b) => {
  const F = 'Onest', ink = '#101828', muted = '#475467', blue = '#2e5bff';
  b.bg('#eef1f6');
  b.rect(20, 20, 560, 1480, '#ffffff', { r: 16 });
  b.rect(40, 48, 28, 28, blue, { r: 8 });
  b.text('fieldnote', 78, 46, 200, { family: F, weight: 800, size: 22, color: ink }, 'start', { role: 'logo' });
  b.text('October 2026', 360, 52, 200, { family: F, weight: 500, size: 14, color: muted }, 'end');
  b.rect(40, 100, 520, 300, lin(135, '#2e5bff', '#7a5cff'), { r: 14 });
  b.circle(520, 140, 90, '#ffffff', { opacity: 0.12 });
  b.text('WHAT’S NEW', 72, 140, 400, { family: F, weight: 700, size: 13, color: '#dfe6ff', tracking: 300 });
  b.text('Offline mode is finally here', 72, 166, 440, { family: F, weight: 800, size: 40, color: '#ffffff' }, 'start', { role: 'headline', lh: 1.1 });
  b.text('Capture notes on the trail, in the lab or 30,000 feet up — Fieldnote syncs the moment you’re back online.', 72, 280, 440, { family: F, size: 16, color: '#eef1ff' }, 'start', { lh: 1.55 });
  b.text('Hi Sam,', 40, 440, 520, { family: F, weight: 600, size: 17, color: ink });
  b.text('This month we shipped the three features you asked for most. Here’s a quick tour — it takes about two minutes.', 40, 472, 520, { family: F, size: 16, color: muted }, 'start', { lh: 1.6 });
  b.photo(PH.laptop, 40, 560, 520, 260, { clip: 'rounded', filter: 'cool', filterAmount: 0.4 });
  b.text('1 · Offline mode', 40, 846, 520, { family: F, weight: 800, size: 22, color: ink });
  b.text('Every notebook is stored on your device, encrypted. Edits merge automatically, even when two people change the same page.', 40, 882, 520, { family: F, size: 16, color: muted }, 'start', { lh: 1.6 });
  const two: Array<[string, string, string]> = [['🏷️', '2 · Smart tags', 'Fieldnote suggests tags as you type, based on your past notes.'], ['🔍', '3 · Search in photos', 'Find handwritten words inside the pictures you capture.']];
  two.forEach(([em, t, d], i) => {
    const x = 40 + i * 270;
    b.rect(x, 990, 250, 220, '#f4f6fb', { r: 14 });
    b.emoji(em, x + 24, 1012, 40);
    b.text(t, x + 24, 1066, 210, { family: F, weight: 800, size: 18, color: ink });
    b.text(d, x + 24, 1100, 206, { family: F, size: 15, color: muted }, 'start', { lh: 1.55 });
  });
  btn(b, 170, 1250, 260, 56, 'Update Fieldnote', blue, '#ffffff', F, { r: 12 });
  b.text('Questions? Just reply — a real human reads every email.', 40, 1330, 520, { family: F, size: 15, color: muted }, 'center');
  b.text('— Ines & the Fieldnote team', 40, 1360, 520, { family: F, weight: 600, size: 15, color: ink }, 'center');
  b.line(40, 1420, 520, '#e4e7ec', 1);
  emailFooter(b, 1516, 'Fieldnote', '1200 Harbor Way, Seattle, WA 98101', '#475467', F);
}, 1);

tpl('mw-email-autumn-sale', 'Autumn Sale Promo Email', EM, 'email', ['sale', 'promo', 'ecommerce', 'autumn', 'discount'], (b) => {
  const D = 'Abril Fatface', F = 'Hanken Grotesk', ink = '#2b1a12', rust = '#b5441b', cream = '#fbf3e6';
  b.bg(cream);
  b.text('HEARTH & HOME GOODS', 40, 36, 520, { family: F, weight: 800, size: 15, color: ink, tracking: 400 }, 'center', { role: 'logo' });
  b.text('New in · Living · Kitchen · Sale', 40, 66, 520, { family: F, weight: 500, size: 14, color: '#6b4e3d' }, 'center');
  b.rect(0, 110, 600, 470, rust);
  b.circle(40, 260, 120, '#c8582c');
  b.circle(560, 500, 70, '#9e3914');
  b.emoji('🍂', 440, 140, 110, { rot: 15 });
  b.emoji('🍁', 70, 150, 70, { rot: -20 });
  b.text('THIS WEEKEND ONLY', 40, 200, 520, { family: F, weight: 800, size: 15, color: '#ffe8d6', tracking: 400 }, 'center');
  b.text('The Autumn\nSale', 40, 232, 520, { family: D, size: 76, color: '#ffffff' }, 'center', { role: 'headline', lh: 1.0 });
  b.text('30% off', 40, 400, 520, { family: D, size: 44, color: '#ffd9a8' }, 'center');
  b.text('vases, planters and terrariums · code COZY30', 40, 466, 520, { family: F, weight: 600, size: 17, color: '#ffffff' }, 'center');
  const prods: Array<[string, string, string, string]> = [['Pinecone terrarium', '$89', '$62', PH.interior], ['Recycled glass vase', '$48', '$34', PH.vasePlant], ['Speckled planter', '$32', '$22', PH.succulent], ['Bud vase trio', '$54', '$38', PH.vases]];
  prods.forEach(([n, was, now, ph], i) => {
    const x = 40 + (i % 2) * 270, y = 620 + Math.floor(i / 2) * 340;
    b.photo(ph, x, y, 250, 230, { filter: 'warm', filterAmount: 0.4 });
    b.rect(x + 12, y + 12, 64, 28, rust, { r: 14 });
    b.text('-30%', x + 12, y + 17, 64, { family: F, weight: 800, size: 13, color: '#ffffff' }, 'center');
    b.text(n, x, y + 246, 250, { family: F, weight: 700, size: 17, color: ink });
    b.text(now, x, y + 274, 60, { family: F, weight: 800, size: 16, color: rust });
    b.text(was, x + 50, y + 274, 80, { family: F, weight: 500, size: 16, color: '#6b4e3d', strike: true });
  });
  btn(b, 160, 1310, 280, 58, 'Shop the sale', ink, '#ffffff', F, { r: 4 });
  b.text('Ends Sunday 11:59 pm · Free shipping over $75', 40, 1390, 520, { family: F, size: 15, color: '#6b4e3d' }, 'center');
  b.line(40, 1440, 520, '#e6d6c2', 1);
  emailFooter(b, 1470, 'Hearth & Home Goods', '48 Mill Street, Asheville, NC 28801', '#6b4e3d', F);
}, 1);

tpl('mw-email-event-invite', 'Event Invitation Email', EM, 'email', ['event', 'invite', 'rsvp', 'gallery', 'art'], (b) => {
  const D = 'Syne', F = 'Instrument Sans', ink = '#111111', paper = '#f4f1ea', pink = '#ff5c8a', yellow = '#ffd84d';
  b.bg(paper);
  b.text('STUDIO NINE', 40, 36, 300, { family: D, weight: 800, size: 18, color: ink, tracking: 200 }, 'start', { role: 'logo' });
  b.text('Issue · Invitation', 260, 40, 300, { family: F, size: 14, color: '#4a4a4a' }, 'end');
  b.rect(40, 90, 520, 560, ink);
  b.circle(470, 200, 80, pink);
  b.shape('star', 90, 420, 150, 150, yellow, { params: { points: 12, innerRatio: 0.6 }, rot: 10 });
  b.rect(330, 400, 180, 180, '#4d7cff', { r: 90 });
  b.text('You’re\ninvited', 72, 130, 330, { family: D, weight: 800, size: 56, color: '#ffffff' }, 'start', { role: 'headline', lh: 0.95 });
  b.text('Open Studio Night', 72, 290, 360, { family: D, weight: 700, size: 26, color: yellow });
  b.text('An evening of new work, live screen-printing and natural wine with the nine artists of our autumn residency.', 40, 690, 520, { family: F, size: 18, color: ink }, 'start', { lh: 1.6 });
  const det: Array<[string, string]> = [['WHEN', 'Friday 20 November 2026\n6 pm – 10 pm'], ['WHERE', 'Studio Nine, Unit 4\n19 Canal Road, London E2'], ['DRESS', 'Come as you are']];
  det.forEach(([h, t], i) => {
    const y = 820 + i * 100;
    b.line(40, y, 520, '#cfc8ba', 1);
    b.text(h, 40, y + 22, 140, { family: D, weight: 700, size: 14, color: '#4a4a4a', tracking: 200 });
    b.text(t, 180, y + 18, 380, { family: F, weight: 600, size: 18, color: ink }, 'start', { lh: 1.4 });
  });
  btn(b, 40, 1150, 250, 58, 'RSVP — I’ll be there', pink, ink, F, { r: 29 });
  btn(b, 306, 1150, 160, 58, 'Can’t make it', 'none', ink, F, { r: 29, stroke: ink });
  b.text('Free entry, but space is limited — please reply by 13 November.', 40, 1236, 520, { family: F, size: 15, color: '#4a4a4a' });
  b.line(40, 1300, 520, '#cfc8ba', 1);
  emailFooter(b, 1340, 'Studio Nine', '19 Canal Road, London E2 8AA', '#4a4a4a', F);
}, 1);

tpl('mw-email-welcome', 'Welcome Email', EM, 'email', ['welcome', 'onboarding', 'new customer', 'plants', 'friendly'], (b) => {
  const D = 'Fraunces', F = 'Nunito', green = '#1f4a33', muted = '#4f5f55', mint = '#e3efe5';
  b.bg('#ffffff');
  b.rect(0, 0, 600, 520, mint);
  b.text('fernway', 40, 40, 520, { family: D, weight: 700, italic: true, size: 30, color: green }, 'center', { role: 'logo' });
  b.shape('blob', 160, 120, 280, 230, '#c6dfcb');
  b.emoji('🪴', 210, 120, 180);
  b.text('Welcome to the family, Jo!', 40, 340, 520, { family: D, weight: 600, size: 40, color: green }, 'center', { role: 'headline', lh: 1.1 });
  b.text('Your account is ready. Here’s how to keep every plant in your home happy.', 70, 440, 460, { family: F, size: 17, color: muted }, 'center', { lh: 1.55 });
  const steps: Array<[string, string, string]> = [['📸', 'Add your plants', 'Snap a photo and we’ll identify the species in seconds.'], ['💧', 'Get care reminders', 'Watering, feeding and repotting, timed to your climate.'], ['💬', 'Ask a plant doctor', 'Real horticulturists answer within 24 hours.']];
  steps.forEach(([em, t, d], i) => {
    const y = 580 + i * 130;
    b.circle(84, y + 40, 40, mint);
    b.emoji(em, 62, y + 18, 44);
    b.text(t, 150, y + 8, 410, { family: F, weight: 800, size: 19, color: green });
    b.text(d, 150, y + 40, 410, { family: F, size: 16, color: muted }, 'start', { lh: 1.5 });
  });
  btn(b, 160, 990, 280, 58, 'Add my first plant', green, '#ffffff', F, { r: 29 });
  b.box(40, 1100, 520, 150, '#9cc5a6', 2, 16);
  b.text('A little welcome gift', 40, 1126, 520, { family: D, weight: 600, size: 22, color: green }, 'center');
  b.text('15% off your first pot or soil order with code', 40, 1166, 520, { family: F, size: 16, color: muted }, 'center');
  b.text('HELLOGREEN', 40, 1198, 520, { family: F, weight: 900, size: 22, color: green, tracking: 300 }, 'center');
  b.line(40, 1300, 520, '#e1e8e2', 1);
  emailFooter(b, 1340, 'Fernway', '7 Greenhouse Row, Austin, TX 78701', '#4f5f55', F);
}, 1);
