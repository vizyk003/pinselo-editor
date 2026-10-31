/** Templates: print (see index.ts for categories). */
import { B, lin, rad, solid, tpl } from './builder';

void [B, lin, rad, solid, tpl];

const CLEAR = '#00000000';
/** A colour value (for effects / table styles). */
const col = (c: string) => (solid(c) as Extract<ReturnType<typeof solid>, { k: 'solid' }>).color;
/** The most recently added element (for styling options the builder does not expose, e.g. table style). */
const last = (b: B) => { const v = Object.values(b.doc.elements); return v[v.length - 1]!; };
/** Outline-only version of any shape kind. */
function outline(b: B, kind: Parameters<B['shape']>[0], x: number, y: number, w: number, h: number, color: string, width: number) {
  b.shape(kind, x, y, w, h, CLEAR, { stroke: [color, width] });
  const e = last(b);
  if (e.type === 'shape') e.fills = [];
}
function styleTable(b: B, s: { family: string; size: number; color: string; border: string; borderWidth?: number; header?: string; headerColor?: string; stripe?: string; padding?: number }, widths?: number[], align?: Record<number, 'start' | 'center' | 'end'>) {
  const t = last(b);
  if (t.type !== 'table') return;
  t.style = { family: s.family, size: s.size, color: col(s.color), borderColor: col(s.border), borderWidth: s.borderWidth ?? 1, padding: s.padding ?? 8,
    ...(s.header ? { headerFill: solid(s.header) } : {}), ...(s.headerColor ? { headerColor: col(s.headerColor) } : {}), ...(s.stripe ? { stripeFill: solid(s.stripe) } : {}) };
  if (widths) { const sum = widths.reduce((a, c) => a + c, 0); t.cols = widths.map((w) => w / sum); }
  if (align) for (const k of Object.keys(t.cells)) { const col = Number(k.split(':')[1]); if (align[col]) t.cells[k] = { ...t.cells[k]!, align: align[col] }; }
}

// ================================================================== Posters (1728 × 2304)
const P = { W: 1728, H: 2304, M: 140 };

tpl('pr-poster-concert-synth', 'Synthwave Concert', 'Posters', 'poster', ['concert', 'music', 'gig', 'retro', 'tour'], (b) => {
  const bg = '#150d30';
  b.bg(bg);
  b.circle(864, 760, 640, '#ff5e7e', { opacity: 0.12 });
  b.circle(864, 760, 500, lin(90, '#ffd166', '#ff7a59', '#ff3d81', '#8338ec'));
  [0, 1, 2, 3, 4, 5].forEach((i) => b.rect(0, 860 + i * 70 + i * i * 4, P.W, 8 + i * 7, bg));
  b.rect(0, 1260, P.W, 1044, bg);
  for (let i = 0; i < 4; i++) b.line(0, 1290 + i * i * 9 + i * 18, P.W, '#ff3d81', 2);
  b.text('THE NEON TIDES WORLD TOUR · 2026', P.M, 150, P.W - 2 * P.M, { family: 'Space Grotesk', weight: 700, size: 36, tracking: 380, color: '#ffd166' }, 'center');
  b.text('MIDNIGHT\nSTATIC', P.M, 1430, P.W - 2 * P.M, { family: 'Anton', size: 300, tracking: 30, color: '#fff4e0' }, 'center', { lh: 0.95, role: 'headline', effect: { k: 'shadow', dx: 0, dy: 10, blur: 0, color: col('#ff3d81') } });
  b.text('with Velvet Arcade  ·  Lune & The Satellites', P.M, 2005, P.W - 2 * P.M, { family: 'Space Grotesk', weight: 500, size: 48, color: '#f7c6d9' }, 'center', { role: 'subhead' });
  b.line(P.M, 2092, P.W - 2 * P.M, '#ffd166', 3);
  const cols = ['SAT 14 NOV 2026', 'DOORS 7:30 PM', 'ORPHEUM HALL'];
  cols.forEach((c, i) => b.text(c, P.M + i * 483, 2108, 483, { family: 'Space Grotesk', weight: 700, size: 40, tracking: 120, color: '#ffffff' }, ['start', 'center', 'end'][i] as 'start'));
}, 1);

tpl('pr-poster-exhibition-forms', 'Gallery Exhibition', 'Posters', 'poster', ['exhibition', 'art', 'gallery', 'museum', 'minimal'], (b) => {
  b.bg('#f2efe9');
  const s = 714, g = 20, x0 = P.M, y0 = P.M;
  b.rect(x0, y0, s, s, '#1f3a5f');
  b.circle(x0 + s / 2, y0 + s / 2, 250, '#f2cc8f');
  b.rect(x0 + s + g, y0, s, s, '#e7dfd0');
  b.shape('arch', x0 + s + g + 150, y0 + 150, s - 300, s - 150, '#e07a5f');
  b.rect(x0, y0 + s + g, s, s, '#e07a5f');
  b.shape('diamond', x0 + 110, y0 + s + g + 110, s - 220, s - 220, '#1f3a5f');
  b.rect(x0 + s + g, y0 + s + g, s, s, '#3d405b');
  b.circle(x0 + s + g + s / 2, y0 + s + g + s / 2, 257, '#f2cc8f');
  b.circle(x0 + s + g + s / 2, y0 + s + g + s / 2, 110, '#3d405b');
  b.text('Forms of Quiet', P.M - 6, 1600, P.W - 2 * P.M, { family: 'Instrument Serif', size: 230, color: '#1b1b1b' }, 'start', { lh: 1.0, role: 'headline' });
  b.text('Paintings and works on paper by Ines Varga', P.M, 1850, P.W - 2 * P.M, { family: 'Instrument Serif', italic: true, size: 58, color: '#3d405b' }, 'start', { role: 'subhead' });
  b.line(P.M, 1965, P.W - 2 * P.M, '#1b1b1b', 3);
  const info = ['12 SEP — 30 NOV 2026\nOpening night 11 Sep, 7 pm', 'HALDEN CONTEMPORARY\n41 Mercer Lane, Riverside', 'TUE – SUN, 10 AM – 6 PM\nhalden.example'];
  info.forEach((t, i) => b.text(t, P.M + i * 490, 1995, 470, { family: 'Inter Tight', weight: 500, size: 32, color: '#1b1b1b' }, 'start', { lh: 1.45 }));
});

tpl('pr-poster-movie-lighthouse', 'Film Poster', 'Posters', 'poster', ['movie', 'film', 'cinema', 'premiere', 'dark'], (b) => {
  b.bg(lin(90, '#050f17', '#0d3346', '#12404f'));
  for (const [x, y, r] of [[220, 520, 4], [430, 300, 3], [610, 640, 5], [1500, 760, 4], [1180, 210, 3], [300, 880, 3], [1540, 330, 5], [980, 420, 3], [760, 230, 4]] as const) b.circle(x, y, r, '#f4e9cd', { opacity: 0.8 });
  b.circle(1300, 520, 300, '#f4e9cd', { opacity: 0.06 });
  b.shape('moon', 1190, 400, 220, 240, '#f4e9cd', { rot: -20 });
  b.shape('triangle', 364, 560, 200, 800, '#ffe9a8', { rot: 90, opacity: 0.14 });
  b.shape('triangle', 1164, 560, 200, 800, '#ffe9a8', { rot: -90, opacity: 0.14 });
  b.rect(800, 990, 128, 560, '#e8e0cc');
  b.rect(800, 1110, 128, 70, '#a63d3d');
  b.rect(800, 1290, 128, 70, '#a63d3d');
  b.rect(816, 930, 96, 64, '#14232b');
  b.circle(864, 962, 26, '#ffe9a8');
  b.shape('triangle', 796, 868, 136, 66, '#14232b');
  b.rect(760, 1520, 208, 40, '#14232b');
  b.rect(0, 1550, P.W, 754, '#04101a');
  for (let i = 0; i < 4; i++) b.line(160 + i * 90, 1590 + i * 36, 1400 - i * 180, '#1b4b60', 3, { dash: [40, 26] });
  b.text('MAREN HOLT     DANIEL ASHBY     IKE OSEI', P.M, 150, P.W - 2 * P.M, { family: 'Oswald', weight: 500, size: 44, tracking: 250, color: '#e3ecee' }, 'center');
  b.text('SOME SIGNALS ARE MEANT TO BE FOUND', P.M, 230, P.W - 2 * P.M, { family: 'Oswald', weight: 300, size: 32, tracking: 400, color: '#9fb7bf' }, 'center');
  b.text('THE LAST', P.M, 1720, P.W - 2 * P.M, { family: 'Cinzel', weight: 600, size: 64, tracking: 600, color: '#e8e0cc' }, 'center', { role: 'subhead' });
  b.text('LIGHTHOUSE', P.M - 40, 1790, P.W - 2 * P.M + 80, { family: 'Cinzel', weight: 700, size: 170, tracking: 60, color: '#f4e9cd' }, 'center', { role: 'headline' });
  b.text('A HARBOUR LIGHT PICTURES PRODUCTION  ·  DIRECTED BY SOLVEIG ARNASON  ·  WRITTEN BY PETRA KLINE & SOLVEIG ARNASON\nMUSIC BY ODIN MARR  ·  CINEMATOGRAPHY BY LEAH ODUYA  ·  EDITED BY RUI TAVARES  ·  PRODUCED BY AMIR SALEH', P.M, 2015, P.W - 2 * P.M, { family: 'Oswald', weight: 300, size: 24, tracking: 60, color: '#8fa9b2' }, 'center', { lh: 1.4 });
  b.text('IN CINEMAS 19 FEBRUARY 2027', P.M, 2095, P.W - 2 * P.M, { family: 'Oswald', weight: 500, size: 48, tracking: 300, color: '#ffe9a8' }, 'center', { role: 'cta' });
});

tpl('pr-poster-motivational-mountains', 'Slow Progress Print', 'Posters', 'poster', ['motivational', 'quote', 'mountains', 'wall art', 'inspiration'], (b) => {
  b.bg('#f6f0e6');
  b.text('SLOW\nPROGRESS\nIS STILL\nPROGRESS', P.M, 190, P.W - 2 * P.M, { family: 'Unbounded', weight: 800, size: 176, color: '#1d3557' }, 'start', { lh: 1.02, role: 'headline' });
  b.text('Small steps, taken daily, move mountains.', P.M, 960, P.W - 2 * P.M, { family: 'Fraunces', italic: true, weight: 400, size: 64, color: '#9c3d2f' }, 'start', { role: 'subhead' });
  b.circle(1290, 1380, 190, '#e63946');
  b.shape('triangle', -260, 1360, 1320, 1000, '#7fa7c4');
  b.shape('triangle', 560, 1180, 1520, 1180, '#1d3557');
  b.shape('triangle', -300, 1700, 1100, 700, '#2a4d6e');
  b.shape('triangle', 1000, 1640, 1100, 760, '#457b9d');
  b.rect(0, 2140, P.W, 164, '#14263f');
  b.text('STUDIO NORTH  ·  PRINT NO. 07', P.M, 2196, P.W - 2 * P.M, { family: 'Unbounded', weight: 500, size: 28, tracking: 300, color: '#f6f0e6' }, 'center');
});

tpl('pr-poster-sale-season', 'End of Season Sale', 'Posters', 'poster', ['sale', 'retail', 'discount', 'store', 'bold'], (b) => {
  b.bg('#ffe14d');
  b.text('END OF SEASON', P.M, 150, P.W - 2 * P.M, { family: 'Archivo', weight: 900, size: 132, color: '#111111' }, 'start', { role: 'subhead' });
  b.text('SALE', P.M - 14, 270, P.W, { family: 'Archivo', weight: 900, size: 520, tracking: -10, color: '#111111' }, 'start', { lh: 0.95, role: 'headline' });
  b.shape('burst', 900, 880, 720, 720, '#ff3d7f', { rot: 8, params: { points: 18, innerRatio: 0.82 } });
  b.text('UP TO', 990, 1080, 540, { family: 'Archivo', weight: 800, size: 64, color: '#ffffff' }, 'center');
  b.text('60%', 990, 1140, 540, { family: 'Archivo', weight: 900, size: 230, color: '#ffffff' }, 'center', { lh: 1.0, role: 'price' });
  b.text('OFF', 990, 1370, 540, { family: 'Archivo', weight: 800, size: 64, color: '#ffffff' }, 'center');
  b.text('Furniture, lighting and décor from our autumn collection — everything must go.', P.M, 930, 700, { family: 'Hanken Grotesk', weight: 500, size: 52, color: '#111111' }, 'start', { lh: 1.3, role: 'body' });
  b.rect(P.M, 1260, 680, 104, '#111111', { r: 52 });
  b.text('CODE HEARTH10 · EXTRA 10%', P.M, 1290, 680, { family: 'Hanken Grotesk', weight: 800, size: 36, tracking: 60, color: '#ffe14d' }, 'center', { role: 'cta' });
  b.emoji('🛋️', P.M + 40, 1430, 300, { rot: -6 });
  b.emoji('💡', P.M + 380, 1460, 220, { rot: 10 });
  b.rect(0, 1790, P.W, 514, '#111111');
  b.text('FRI 23 – SUN 25 OCTOBER 2026', P.M, 1915, P.W - 2 * P.M, { family: 'Archivo', weight: 800, size: 76, color: '#ffe14d' }, 'start');
  b.text('Hearth & Grain Home Store  ·  220 Alder Street  ·  Open 9 am – 8 pm', P.M, 2030, P.W - 2 * P.M, { family: 'Hanken Grotesk', weight: 500, size: 42, color: '#ffffff' }, 'start');
  b.text('hearthandgrain.example', P.M, 2105, P.W - 2 * P.M, { family: 'Hanken Grotesk', weight: 700, size: 40, color: '#ff7fa8' }, 'start');
});

tpl('pr-poster-research-academic', 'Research Poster', 'Posters', 'poster', ['science', 'research', 'academic', 'conference', 'university'], (b) => {
  const navy = '#0f2e4c', ink = '#1f2a37', accent = '#1f7a70';
  b.bg('#ffffff');
  b.rect(0, 0, P.W, 440, navy);
  b.text('Urban Tree Canopy Lowers Street-Level Heat by Up to 4 °C', P.M, 100, P.W - 2 * P.M, { family: 'Source Serif 4', weight: 700, size: 82, color: '#ffffff' }, 'start', { lh: 1.12, role: 'headline' });
  b.text('Priya Natarajan¹, Tomás Ferreira¹, Grace Liu²', P.M, 300, P.W - 2 * P.M, { family: 'Inter', weight: 600, size: 38, color: '#d6e6f5' });
  b.text('¹Department of Environmental Science, Northfield University  ·  ²Cityscape Climate Lab', P.M, 360, P.W - 2 * P.M, { family: 'Inter', size: 28, color: '#a9c6e0' });
  const cw = 456, gx = 40, top = 520;
  const cx = (i: number) => P.M + i * (cw + gx);
  const head = (t: string, x: number, y: number) => { b.rect(x, y + 8, 10, 46, accent); b.text(t, x + 26, y, cw - 26, { family: 'Source Serif 4', weight: 700, size: 46, color: navy }, 'start', { role: 'subhead' }); };
  const body = { family: 'Inter', size: 30, color: ink };
  head('Introduction', cx(0), top);
  b.text('Summer heat is the deadliest weather hazard in cities. Street trees cool the air through shade and evapotranspiration, but how much canopy is enough to make a measurable difference on a single block?', cx(0), top + 80, cw, body, 'start', { lh: 1.45, role: 'body' });
  head('Methods', cx(0), top + 470);
  b.text('• 48 residential blocks in three neighbourhoods\n• Canopy cover mapped from 2025 aerial imagery\n• Air temperature logged every 10 min, June–August\n• Mixed-effects model controlling for pavement, building height and wind', cx(0), top + 550, cw, body, 'start', { lh: 1.45 });
  b.frame(cx(0), top + 1060, cw, 380, 'rect', '#dbe4ec');
  b.text('Fig. 1 — Study area and logger locations', cx(0), top + 1460, cw, { family: 'Inter', italic: true, size: 24, color: '#4b5563' });
  head('Results', cx(1), top);
  b.chart('column', cx(1), top + 90, cw, 560, ['<10%', '10–25%', '25–40%', '>40%'], [{ name: 'Peak temp. (°C)', values: [36.8, 35.4, 34.1, 32.7], color: accent }]);
  { const c = last(b); if (c.type === 'chart') c.style = { ...c.style, family: 'Inter', fontSize: 24, textColor: col(ink), showValues: true }; }
  b.text('Fig. 2 — Mean afternoon peak by canopy cover', cx(1), top + 670, cw, { family: 'Inter', italic: true, size: 24, color: '#4b5563' });
  b.rect(cx(1), top + 760, cw, 330, '#e8f4f2', { r: 16 });
  b.text('−4.1 °C', cx(1), top + 800, cw, { family: 'Source Serif 4', weight: 700, size: 120, color: accent }, 'center');
  b.text('on blocks with more than 40% canopy versus less than 10%', cx(1) + 40, top + 960, cw - 80, { family: 'Inter', weight: 500, size: 28, color: ink }, 'center', { lh: 1.35 });
  b.text('Cooling grew fastest between 10% and 40% cover, then levelled off. Effects were strongest between 2 pm and 6 pm.', cx(1), top + 1130, cw, body, 'start', { lh: 1.45 });
  head('Conclusions', cx(2), top);
  b.text('• Reaching 25% canopy on every block would cut peak heat by about 2 °C\n• Planting priority should go to blocks under 10% cover\n• Results support a city-wide canopy target in the 2027 climate plan', cx(2), top + 80, cw, body, 'start', { lh: 1.45 });
  head('Next steps', cx(2), top + 620);
  b.text('We are extending the logger network to 120 blocks and adding surface temperature and resident heat-stress surveys.', cx(2), top + 700, cw, body, 'start', { lh: 1.45 });
  head('References', cx(2), top + 1000);
  b.text('1. Moreno, A. et al. (2023) Urban Climate 49, 101–118.\n2. Hale, R. & Osei, K. (2024) Landscape and Urban Planning 241.\n3. City of Northfield (2025) Street Tree Inventory.', cx(2), top + 1080, cw, { family: 'Inter', size: 22, color: '#4b5563' }, 'start', { lh: 1.45 });
  b.qr('https://northfield.example/canopy', cx(2), top + 1340, 170);
  b.text('Scan for the full dataset and paper', cx(2) + 195, top + 1390, cw - 195, { family: 'Inter', weight: 600, size: 26, color: ink }, 'start', { lh: 1.35 });
  b.rect(0, 2160, P.W, 144, '#e8eef4');
  b.text('Midwest Student Research Symposium  ·  April 2027  ·  p.natarajan@northfield.example', P.M, 2212, P.W - 2 * P.M, { family: 'Inter', weight: 500, size: 28, color: navy }, 'center');
});

tpl('pr-poster-block-party', 'Block Party', 'Posters', 'poster', ['community', 'neighborhood', 'party', 'event', 'family'], (b) => {
  const ink = '#264653';
  b.bg('#fff3e2');
  b.shape('blob', -260, -240, 820, 760, '#ffcdb2');
  b.shape('blob', 1240, 1500, 760, 900, '#b5e48c', { rot: 30 });
  b.shape('blob', 1300, -200, 620, 560, '#a8dadc', { rot: 60 });
  b.text('ELM STREET NEIGHBORS PRESENT', P.M, 170, P.W - 2 * P.M, { family: 'Bricolage Grotesque', weight: 800, size: 46, tracking: 250, color: '#c4472b' }, 'center');
  b.text('Block\nParty!', P.M, 260, P.W - 2 * P.M, { family: 'Shrikhand', size: 310, color: ink }, 'center', { lh: 1.0, role: 'headline' });
  ['🎈', '🌭', '🎶', '🍉', '🎨'].forEach((e, i) => b.emoji(e, 250 + i * 260, 960, 200, { rot: [-8, 6, -4, 8, -6][i] }));
  const cards = [['WHEN', 'Saturday\nAugust 15, 2026\nnoon – 8 pm', '#e9c46a'], ['WHERE', 'Elm Street,\nbetween 4th\n& 6th Avenue', '#8ecae6'], ['BRING', 'A dish to share\n& a lawn chair\nfor the evening', '#f4a261']] as const;
  cards.forEach(([h, t, c], i) => {
    const x = P.M + i * 496;
    b.rect(x, 1240, 456, 420, c, { r: 36 });
    b.text(h, x + 40, 1280, 376, { family: 'Bricolage Grotesque', weight: 800, size: 40, tracking: 200, color: ink });
    b.text(t, x + 40, 1350, 376, { family: 'Bricolage Grotesque', weight: 600, size: 50, color: ink }, 'start', { lh: 1.2 });
  });
  b.text('Live music  ·  Kids’ bike parade  ·  Potluck\nFace painting  ·  Dunk tank  ·  Lemonade stand', P.M + 100, 1720, P.W - 2 * P.M - 200, { family: 'Bricolage Grotesque', weight: 500, size: 50, color: ink }, 'center', { lh: 1.35 });
  b.rect(P.M + 160, 1930, P.W - 2 * P.M - 320, 116, ink, { r: 58 });
  b.text('Free for everyone · Rain date August 16', P.M + 160, 1962, P.W - 2 * P.M - 320, { family: 'Bricolage Grotesque', weight: 700, size: 46, color: '#ffffff' }, 'center', { role: 'cta' });
  b.text('elmstreet.example  ·  (555) 214-0188', P.M, 2100, P.W - 2 * P.M, { family: 'Bricolage Grotesque', weight: 600, size: 34, color: ink }, 'center');
});

// ================================================================== Flyers (A5, 559 × 794)
const F = { W: 559, H: 794, M: 40 };

tpl('pr-flyer-trattoria-menu', 'Trattoria Menu', 'Flyers', 'flyer', ['menu', 'restaurant', 'italian', 'food', 'elegant'], (b) => {
  const red = '#9c3d2f', ink = '#2b2118', soft = '#6b5644';
  b.bg('#f7f1e3');
  b.box(20, 20, F.W - 40, F.H - 40, red, 1.5);
  b.box(26, 26, F.W - 52, F.H - 52, red, 0.6);
  b.text('Trattoria Lumi', F.M, 46, F.W - 2 * F.M, { family: 'Playfair Display', weight: 700, italic: true, size: 40, color: red }, 'center', { role: 'logo' });
  b.text('CUCINA DI CASA  ·  SINCE 1998', F.M, 104, F.W - 2 * F.M, { family: 'Montserrat', weight: 600, size: 9.5, tracking: 300, color: soft }, 'center');
  b.shape('diamond', F.W / 2 - 5, 126, 10, 10, red);
  b.line(F.W / 2 - 80, 131, 62, red, 0.8);
  b.line(F.W / 2 + 18, 131, 62, red, 0.8);
  const menu: Array<[string, Array<[string, string, string]>]> = [
    ['ANTIPASTI', [['Burrata', 'heirloom tomato, basil oil, grilled sourdough', '14'], ['Arancini', 'saffron risotto, smoked provola, salsa rossa', '11']]],
    ['PRIMI', [['Cacio e Pepe', 'tonnarelli, pecorino romano, cracked pepper', '19'], ['Pappardelle al Ragù', 'slow-braised beef & pork, parmigiano', '23'], ['Risotto al Limone', 'Amalfi lemon, mascarpone, fresh mint', '21']]],
    ['SECONDI', [['Pollo al Mattone', 'half chicken, salsa verde, rosemary potatoes', '27'], ['Branzino', 'whole sea bass, capers, charred lemon', '32']]],
    ['DOLCI', [['Tiramisù', 'espresso, mascarpone, cocoa', '10'], ['Panna Cotta', 'vanilla bean, honey-poached figs', '9']]],
  ];
  let y = 158;
  for (const [sec, items] of menu) {
    b.text(sec, F.M, y, F.W - 2 * F.M, { family: 'Cinzel', weight: 700, size: 13, tracking: 400, color: red }, 'center', { role: 'subhead' });
    y += 30;
    for (const [n, d, p] of items) {
      b.text(n, 62, y, 360, { family: 'EB Garamond', weight: 600, size: 15, color: ink });
      b.text(p, F.W - 62 - 80, y, 80, { family: 'EB Garamond', weight: 600, size: 15, color: ink }, 'end', { role: 'price' });
      b.text(d, 62, y + 19, 380, { family: 'EB Garamond', italic: true, size: 11.5, color: soft });
      y += 42;
    }
    y += 12;
  }
  b.line(F.W / 2 - 40, y + 4, 80, red, 0.8);
  b.text('Reservations (555) 018-2240  ·  18 Via Corta, Hillcrest  ·  lumi.example', F.M, y + 18, F.W - 2 * F.M, { family: 'EB Garamond', size: 11.5, color: soft }, 'center');
});

tpl('pr-flyer-open-house', 'Open House Flyer', 'Flyers', 'flyer', ['real estate', 'open house', 'home', 'property', 'realtor'], (b) => {
  const green = '#1f3d36', gold = '#d9b98a', ink = '#2f3a36';
  b.bg('#ffffff');
  b.photo('picsum:859', 0, 0, F.W, 330, { focus: [0.45, 0.5], filter: 'warm', filterAmount: 0.4 });
  b.rect(F.M, 300, 370, 56, green);
  b.text('OPEN HOUSE  ·  SUN 18 OCT, 1–4 PM', F.M, 319, 370, { family: 'Tenor Sans', size: 12.5, tracking: 180, color: '#ffffff' }, 'center', { role: 'headline' });
  b.text('$685,000', F.M, 380, 240, { family: 'DM Serif Display', size: 40, color: green }, 'start', { role: 'price' });
  b.text('48 Juniper Ridge Road\nMaple Falls, OR 97031', F.M, 434, 240, { family: 'Inter', weight: 500, size: 12.5, color: ink }, 'start', { lh: 1.45 });
  b.text('Sunlit brick townhouse on a quiet, tree-lined street with a chef’s kitchen, cozy reading nook and a private courtyard garden.', 300, 384, 219, { family: 'Inter', size: 11.5, color: '#4b5753' }, 'start', { lh: 1.5, role: 'body' });
  ['picsum:1008', 'picsum:999', 'picsum:365'].forEach((id, i) => b.photo(id, F.M + i * 163, 496, 153, 100, { r: 4 }));
  const feats = [['🛏️', '4 bedrooms'], ['🛁', '3 bathrooms'], ['📐', '2,450 sq ft']] as const;
  feats.forEach(([e, t], i) => {
    b.emoji(e, F.M + i * 163, 616, 26);
    b.text(t, F.M + i * 163 + 34, 620, 125, { family: 'Inter', weight: 600, size: 12.5, color: ink });
  });
  b.rect(0, 676, F.W, 118, green);
  b.frame(F.M, 699, 72, 72, 'ellipse', '#3d5c54');
  b.text('Dana Whitfield', 128, 706, 220, { family: 'Tenor Sans', size: 17, color: '#ffffff' });
  b.text('Juniper Realty Group\n(555) 304-7781  ·  dana@juniper.example', 128, 732, 260, { family: 'Inter', size: 10.5, color: '#d9e4dd' }, 'start', { lh: 1.45 });
  b.text('SCAN FOR\nPHOTOS', 390, 718, 60, { family: 'Inter', weight: 700, size: 9, tracking: 100, color: gold }, 'end', { lh: 1.3 });
  b.rect(459, 696, 62, 62, '#ffffff', { r: 4 });
  b.qr('https://juniper.example/48-juniper-ridge', 462, 699, 56);
});

tpl('pr-flyer-sunrise-yoga', 'Sunrise Yoga Class', 'Flyers', 'flyer', ['yoga', 'fitness', 'wellness', 'class', 'calm'], (b) => {
  const deep = '#3f4a3c', terra = '#9a4329';
  b.bg('#f4e8d8');
  b.shape('arch', 110, 56, 339, 420, '#ecd2b4');
  b.circle(279.5, 236, 74, '#e07a5f');
  b.shape('half-circle', 110, 360, 250, 116, '#a3b18a');
  b.shape('half-circle', 230, 386, 219, 90, '#7d8f69');
  b.text('Sunrise Flow', F.M, 500, F.W - 2 * F.M, { family: 'Fraunces', weight: 600, italic: true, size: 50, color: deep }, 'center', { role: 'headline' });
  b.text('GENTLE VINYASA  ·  ALL LEVELS WELCOME', F.M, 568, F.W - 2 * F.M, { family: 'Hanken Grotesk', weight: 700, size: 10.5, tracking: 260, color: terra }, 'center', { role: 'subhead' });
  const cols = [['MON · WED · FRI', '6:30 – 7:30 am'], ['RIVERSIDE PARK', 'North lawn pavilion'], ['$12 DROP-IN', '$90 for 10 classes']];
  cols.forEach(([h, t], i) => {
    b.text(h, F.M + i * 160, 604, 159, { family: 'Hanken Grotesk', weight: 800, size: 11, tracking: 80, color: deep }, 'center');
    b.text(t, F.M + i * 160, 621, 159, { family: 'Hanken Grotesk', size: 11.5, color: '#5c5145' }, 'center');
    if (i) b.line(F.M + i * 160, 622, 30, '#c9b49a', 1, { rot: 90 });
  });
  b.rect(165, 668, 229, 38, deep, { r: 19 });
  b.text('Book at stillwater.example', 165, 678, 229, { family: 'Hanken Grotesk', weight: 700, size: 12, color: '#ffffff' }, 'center', { role: 'cta' });
  b.text('Bring a mat and water  ·  Spare mats available', F.M, 730, F.W - 2 * F.M, { family: 'Hanken Grotesk', size: 10.5, color: '#6b5d50' }, 'center');
});

tpl('pr-flyer-garage-sale', 'Garage Sale Flyer', 'Flyers', 'flyer', ['garage sale', 'yard sale', 'bargain', 'neighborhood', 'bold'], (b) => {
  const ink = '#1b1b1b', red = '#d9481f';
  b.bg('#fdf6e3');
  b.rect(0, 0, F.W, 392, '#ffd23f');
  b.text('HUGE MULTI-FAMILY', F.M, 42, 300, { family: 'Hanken Grotesk', weight: 800, size: 14, tracking: 250, color: ink });
  b.text('GARAGE\nSALE', F.M - 3, 66, F.W - 2 * F.M, { family: 'Anton', size: 142, color: ink }, 'start', { lh: 0.92, role: 'headline' });
  b.shape('tag', 340, 214, 172, 88, red, { rot: -12 });
  b.circle(370, 264, 6, '#ffd23f');
  b.text('EVERYTHING\n$1 – $20', 382, 233, 120, { family: 'Anton', size: 20, color: '#ffffff' }, 'center', { lh: 1.1, rot: -12, role: 'price' });
  ['🪑', '📚', '🧸', '🚲', '🎸'].forEach((e, i) => b.emoji(e, 46 + i * 98, 348, 76, { rot: [-8, 6, -4, 8, -6][i] }));
  b.text('SATURDAY & SUNDAY', F.M, 462, F.W - 2 * F.M, { family: 'Hanken Grotesk', weight: 800, size: 14, tracking: 250, color: red }, 'center');
  b.text('OCTOBER 3 – 4  ·  8 AM – 2 PM', F.M, 484, F.W - 2 * F.M, { family: 'Anton', size: 34, color: ink }, 'center', { role: 'subhead' });
  b.text('1127 Birchwood Drive, Cedar Hollow', F.M, 538, F.W - 2 * F.M, { family: 'Hanken Grotesk', weight: 700, size: 17, color: ink }, 'center');
  b.text('Furniture  ·  Kids’ toys & books  ·  Bikes  ·  Kitchenware\nTools  ·  Vinyl records  ·  Winter coats  ·  Board games', F.M, 586, F.W - 2 * F.M, { family: 'Hanken Grotesk', weight: 500, size: 13, color: '#4a4a4a' }, 'center', { lh: 1.55, role: 'body' });
  b.rect(0, 682, F.W, 112, ink);
  b.text('Cash & mobile pay accepted  ·  No early birds, please!', F.M, 708, F.W - 2 * F.M, { family: 'Hanken Grotesk', weight: 700, size: 14, color: '#ffd23f' }, 'center', { role: 'cta' });
  b.text('Follow the yellow signs from Main Street  →', F.M, 738, F.W - 2 * F.M, { family: 'Hanken Grotesk', weight: 500, size: 12, color: '#ffffff' }, 'center');
});

tpl('pr-flyer-math-tutoring', 'Math Tutoring Tear-Off', 'Flyers', 'flyer', ['tutoring', 'education', 'math', 'tear off', 'school'], (b) => {
  const navy = '#22305a', mint = '#2bb3a8';
  b.bg('#ffffff');
  b.rect(0, 0, F.W, 12, mint);
  b.emoji('➗', 438, 40, 44, { rot: 10 });
  b.emoji('📐', 470, 96, 50, { rot: -8 });
  b.emoji('✏️', 418, 128, 40, { rot: 20 });
  b.text('Struggling\nwith math?', F.M, 40, 380, { family: 'Gabarito', weight: 800, size: 46, color: navy }, 'start', { lh: 1.0, role: 'headline' });
  b.text('Friendly, patient tutoring for grades 6–12', F.M, 146, F.W - 2 * F.M, { family: 'Gabarito', weight: 500, size: 16, color: '#4a5578' }, 'start', { role: 'subhead' });
  const chips = ['Pre-algebra', 'Algebra I & II', 'Geometry', 'SAT / ACT math'];
  let x = F.M;
  chips.forEach((c) => { const w = c.length * 7 + 24; b.rect(x, 184, w, 28, '#e3f6f4', { r: 14 }); b.text(c, x, 190, w, { family: 'Gabarito', weight: 600, size: 12, color: navy }, 'center'); x += w + 8; });
  b.text('✓  Certified teacher with 9 years in the classroom\n✓  First session free — no commitment\n✓  In person at the library or online\n✓  Homework help, exam prep & study skills', F.M, 236, F.W - 2 * F.M, { family: 'Gabarito', weight: 500, size: 14, color: '#2f3a5c' }, 'start', { lh: 1.7, role: 'body' });
  b.rect(F.M, 370, F.W - 2 * F.M, 96, '#22305a', { r: 14 });
  b.text('$40', F.M + 24, 386, 120, { family: 'Gabarito', weight: 800, size: 44, color: '#ffd166' }, 'start', { role: 'price' });
  b.text('per hour  ·  5-session packages $180\nMs. Hannah Brooks  ·  (555) 640-2219', F.M + 130, 392, 330, { family: 'Gabarito', weight: 500, size: 13, color: '#ffffff' }, 'start', { lh: 1.5 });
  b.text('hannah@brookstutoring.example', F.M, 486, F.W - 2 * F.M, { family: 'Gabarito', weight: 600, size: 13, color: '#0f7f77' }, 'center');
  b.text('TAKE A NUMBER', F.M, 540, F.W - 2 * F.M, { family: 'Gabarito', weight: 700, size: 10, tracking: 300, color: '#6b7280' }, 'center');
  b.line(0, 566, F.W, '#9aa3b5', 1, { dash: [5, 4] });
  const n = 8, tw = F.W / n;
  for (let i = 0; i < n; i++) {
    const cx = tw * i + tw / 2;
    if (i) b.line(tw * i - 113, 680, 226, '#9aa3b5', 1, { rot: 90, dash: [5, 4] });
    b.text('Math tutoring\n(555) 640-2219', cx - 90, 680 - 14, 180, { family: 'Gabarito', weight: 600, size: 11, color: navy }, 'center', { lh: 1.25, rot: -90 });
  }
});

tpl('pr-flyer-charity-run', 'Charity Fun Run', 'Flyers', 'flyer', ['fundraiser', 'nonprofit', 'charity', 'run', '5k'], (b) => {
  const deep = '#0b4f6c', amber = '#f6b55a';
  b.bg(deep);
  b.shape('drop', 384, 46, 140, 196, '#38b6d8');
  b.shape('drop', 424, 130, 60, 84, '#8fe0f0', { opacity: 0.7 });
  b.circle(500, 60, 8, '#8fe0f0');
  b.text('CHARITY 5K FUN RUN', F.M, 46, 320, { family: 'Hanken Grotesk', weight: 800, size: 11, tracking: 300, color: '#9ee2f0' });
  b.text('Run for\nClean Water', F.M, 70, 340, { family: 'Bricolage Grotesque', weight: 800, size: 54, color: '#ffffff' }, 'start', { lh: 0.98, role: 'headline' });
  b.text('Every registration funds a clean-water well for a family in rural Malawi. Walk, jog or sprint — every step counts.', F.M, 196, 320, { family: 'Hanken Grotesk', size: 13, color: '#d7eef5' }, 'start', { lh: 1.5, role: 'body' });
  b.rect(F.M, 300, F.W - 2 * F.M, 140, '#ffffff', { r: 16 });
  b.text('RAISED SO FAR', F.M + 24, 322, 200, { family: 'Hanken Grotesk', weight: 800, size: 10, tracking: 250, color: '#4b6b78' });
  b.text('$18,450', F.M + 24, 340, 220, { family: 'Bricolage Grotesque', weight: 800, size: 38, color: deep });
  b.text('of our $25,000 goal\n74% there — help us finish!', 300, 348, 195, { family: 'Hanken Grotesk', weight: 600, size: 12, color: '#36515c' }, 'end', { lh: 1.45 });
  b.rect(F.M + 24, 404, F.W - 2 * F.M - 48, 14, '#dceff5', { r: 7 });
  b.rect(F.M + 24, 404, (F.W - 2 * F.M - 48) * 0.74, 14, amber, { r: 7 });
  const info = [['DATE', 'Sunday\n26 April 2027'], ['START', '8:00 am\nLakeside Park'], ['ENTRY', '$35 adults\n$15 under 16']];
  info.forEach(([h, t], i) => {
    b.text(h, F.M + i * 165, 478, 150, { family: 'Hanken Grotesk', weight: 800, size: 10.5, tracking: 250, color: amber });
    b.text(t, F.M + i * 165, 498, 150, { family: 'Hanken Grotesk', weight: 600, size: 14, color: '#ffffff' }, 'start', { lh: 1.4 });
  });
  b.rect(F.M, 590, F.W - 2 * F.M, 54, amber, { r: 27 });
  b.text('Register at runforwater.example', F.M, 606, F.W - 2 * F.M, { family: 'Hanken Grotesk', weight: 800, size: 17, color: '#0b2f40' }, 'center', { role: 'cta' });
  b.text('Organised by Wellspring Partners  ·  100% of entry fees fund water projects', F.M, 690, F.W - 2 * F.M, { family: 'Hanken Grotesk', size: 11, color: '#a9d3df' }, 'center');
  b.text('Medals for every finisher  ·  Kids’ 1K dash at 9:30 am', F.M, 712, F.W - 2 * F.M, { family: 'Hanken Grotesk', weight: 700, size: 11, color: '#ffffff' }, 'center');
});

tpl('pr-flyer-roller-disco', 'Roller Disco Party', 'Flyers', 'flyer', ['party', 'disco', 'night', 'retro', 'skate'], (b) => {
  const night = '#1a0b3b';
  b.bg(night);
  b.circle(279.5, 130, 150, rad('#ff4fd8', night), { opacity: 0.55 });
  b.emoji('🛼', 214, 50, 130, { rot: -10 });
  b.text('Roller', F.M, 196, F.W - 2 * F.M, { family: 'Pacifico', size: 56, color: '#ff7ad9' }, 'center', { role: 'subhead' });
  b.text('DISCO', F.M - 20, 282, F.W - 2 * F.M + 40, { family: 'Monoton', size: 92, color: '#ffe66d' }, 'center', { role: 'headline', effect: { k: 'neon', intensity: 0.5 } });
  b.text('NIGHT', F.M, 396, F.W - 2 * F.M, { family: 'Archivo', weight: 800, size: 22, tracking: 600, color: '#ffffff' }, 'center');
  ['#ff4fd8', '#ff8a3d', '#ffe66d', '#3ee6d8'].forEach((c, i) => b.rect(0, 446 + i * 12, F.W, 8, c));
  b.text('FRIDAY 13 NOVEMBER  ·  8 PM – MIDNIGHT', F.M, 518, F.W - 2 * F.M, { family: 'Archivo', weight: 700, size: 15, tracking: 150, color: '#ffe66d' }, 'center');
  b.text('Starlight Skate Rink  ·  900 Ocean Avenue', F.M, 546, F.W - 2 * F.M, { family: 'Archivo', weight: 500, size: 15, color: '#ffffff' }, 'center');
  b.text('Skate hire included  ·  70s, 80s & disco DJ sets\nBest-dressed prize  ·  Glow bar & snacks', F.M, 580, F.W - 2 * F.M, { family: 'Archivo', size: 12.5, color: '#d9c8ff' }, 'center', { lh: 1.5, role: 'body' });
  b.rect(144, 648, 271, 46, '#ff7ad9', { r: 23 });
  b.text('Tickets $18 · skate.example', 144, 661, 271, { family: 'Archivo', weight: 800, size: 15, color: night }, 'center', { role: 'cta' });
  b.text('All ages until 10 pm  ·  18+ after 10 pm', F.M, 726, F.W - 2 * F.M, { family: 'Archivo', size: 11, color: '#b8a6e0' }, 'center');
});

// ================================================================== Business cards (3.5 × 2 in, 336 × 192)
const C = { W: 336, H: 192, M: 22 };

tpl('pr-card-elegant-monogram', 'Elegant Monogram Card', 'Business Cards', 'business-card', ['elegant', 'luxury', 'interior design', 'monogram', 'gold'], (b) => {
  const gold = '#c6a15b', dark = '#1c1a17';
  b.page(0).bg(dark);
  outline(b, 'ellipse', 132, 30, 72, 72, gold, 1);
  b.text('AV', 132, 46, 72, { family: 'Cormorant Garamond', weight: 500, size: 32, color: gold }, 'center', { lh: 1.1, role: 'logo' });
  b.text('AURELIA VANCE', C.M, 120, C.W - 2 * C.M, { family: 'Cormorant Garamond', weight: 600, size: 14, tracking: 450, color: '#efe6d6' }, 'center', { role: 'headline' });
  b.text('INTERIOR DESIGN STUDIO', C.M, 144, C.W - 2 * C.M, { family: 'Montserrat', weight: 500, size: 9, tracking: 320, color: gold }, 'center');
  b.page(1).bg('#f4efe6');
  b.text('Aurelia Vance', 26, 26, 220, { family: 'Cormorant Garamond', weight: 600, size: 24, color: dark }, 'start', { role: 'headline' });
  b.text('PRINCIPAL DESIGNER', 26, 58, 220, { family: 'Montserrat', weight: 500, size: 9, tracking: 220, color: '#7a5f30' });
  b.line(26, 84, 40, gold, 1);
  b.text('+1 (555) 210-4471\naurelia@vancestudio.example\n12 Linden Mews, Charleston SC\nvancestudio.example', 26, 98, 230, { family: 'Montserrat', size: 9.5, color: '#3a352e' }, 'start', { lh: 1.55 });
  outline(b, 'ellipse', 262, 26, 48, 48, gold, 0.8);
  b.text('AV', 262, 37, 48, { family: 'Cormorant Garamond', weight: 500, size: 20, color: '#7a5f30' }, 'center', { lh: 1.1 });
}, 2);

tpl('pr-card-bold-builder', 'Bold Contractor Card', 'Business Cards', 'business-card', ['bold', 'contractor', 'carpentry', 'orange', 'trades'], (b) => {
  const orange = '#ff5a1f', ink = '#111111';
  b.page(0).bg(orange);
  b.rect(236, 0, 100, C.H, ink);
  b.emoji('🔨', 252, 64, 64, { rot: -20 });
  b.text('KOJO\nBUILDS', C.M, 34, 210, { family: 'Archivo Black', size: 46, color: ink }, 'start', { lh: 0.92, role: 'logo' });
  b.text('CARPENTRY · RENOVATION\nCUSTOM FURNITURE', C.M, 136, 210, { family: 'Archivo', weight: 800, size: 9.5, tracking: 120, color: ink }, 'start', { lh: 1.45 });
  b.page(1).bg(ink);
  b.rect(0, 0, 8, C.H, orange);
  b.text('Kojo Mensah', 28, 24, 220, { family: 'Archivo', weight: 800, size: 19, color: '#ffffff' }, 'start', { role: 'headline' });
  b.text('Master carpenter & owner', 28, 50, 220, { family: 'Archivo', weight: 500, size: 10.5, color: '#ff8a63' });
  b.rect(28, 78, 30, 4, orange);
  b.text('(555) 771-0392\nkojo@kojobuilds.example\nkojobuilds.example\nLicensed & insured · Lic. CB-20417', 28, 94, 220, { family: 'Archivo', size: 9.5, color: '#e5e5e5' }, 'start', { lh: 1.55 });
  b.rect(244, 100, 70, 70, '#ffffff', { r: 4 });
  b.qr('https://kojobuilds.example', 249, 105, 60);
}, 2);

tpl('pr-card-minimal-architect', 'Minimal Architect Card', 'Business Cards', 'business-card', ['minimal', 'architect', 'clean', 'professional', 'white'], (b) => {
  b.bg('#fafaf7');
  b.rect(C.M, C.M, 14, 14, '#111111');
  b.text('OSTROWSKI ARCHITECTS', 140, C.M + 1, C.W - 140 - C.M, { family: 'Inter Tight', weight: 600, size: 9, tracking: 260, color: '#111111' }, 'end', { role: 'logo' });
  b.text('Lena Ostrowski', C.M, 110, 170, { family: 'Inter Tight', weight: 600, size: 16, color: '#111111' }, 'start', { role: 'headline' });
  b.text('Architect, RIBA', C.M, 132, 170, { family: 'Inter Tight', size: 10, color: '#555555' });
  b.line(C.M, 100, C.W - 2 * C.M, '#d4d4cf', 0.8);
  b.text('+44 20 5550 0182\nlena@ostrowski.example\nostrowski.example', 170, 110, C.W - 170 - C.M, { family: 'Inter Tight', size: 9.5, color: '#333333' }, 'end', { lh: 1.55 });
});

tpl('pr-card-creative-illustrator', 'Playful Illustrator Card', 'Business Cards', 'business-card', ['creative', 'illustrator', 'artist', 'playful', 'colorful'], (b) => {
  const ink = '#2b2d42';
  b.page(0).bg('#fff4e8');
  b.shape('blob', -40, -50, 140, 130, '#ff9f7a');
  b.shape('blob', 262, 118, 120, 110, '#7bdff2', { rot: 40 });
  b.circle(300, 36, 14, '#ffd166');
  b.circle(46, 160, 7, '#7bdff2');
  b.emoji('🖌️', 238, 48, 34, { rot: 20 });
  b.text('Mina Park', C.M, 62, C.W - 2 * C.M, { family: 'Caveat', weight: 700, size: 46, color: ink }, 'center', { lh: 1.1, role: 'logo' });
  b.text('ILLUSTRATION & HAND LETTERING', C.M, 122, C.W - 2 * C.M, { family: 'DM Sans', weight: 600, size: 9, tracking: 200, color: '#5b5e78' }, 'center');
  b.page(1).bg(ink);
  b.text('Let’s make something lovely.', C.M, 26, C.W - 2 * C.M, { family: 'Caveat', weight: 600, size: 24, color: '#ffd166' }, 'start', { role: 'headline' });
  b.text('hello@minapark.example\n(555) 238-9014\n@minapark.draws\nminapark.example', C.M, 76, 200, { family: 'DM Sans', weight: 500, size: 10, color: '#f1f1f6' }, 'start', { lh: 1.6 });
  b.circle(270, 120, 40, '#ff9f7a');
  b.circle(296, 96, 16, '#7bdff2');
  b.text('MP', 230, 108, 80, { family: 'Caveat', weight: 700, size: 22, color: ink }, 'center');
}, 2);

tpl('pr-card-corporate-finance', 'Corporate Finance Card', 'Business Cards', 'business-card', ['corporate', 'finance', 'professional', 'navy', 'consulting'], (b) => {
  const navy = '#0c2340', teal = '#1aa39a';
  b.page(0).bg(navy);
  b.shape('parallelogram', 124, 46, 32, 46, teal, { params: { skew: 0.45 } });
  b.shape('parallelogram', 148, 46, 32, 46, '#5fb4e6', { params: { skew: 0.45 } });
  b.shape('parallelogram', 172, 46, 32, 46, '#ffffff', { params: { skew: 0.45 } });
  b.text('NORTHBEAM', C.M, 108, C.W - 2 * C.M, { family: 'Manrope', weight: 800, size: 18, tracking: 300, color: '#ffffff' }, 'center', { role: 'logo' });
  b.text('CAPITAL PARTNERS', C.M, 134, C.W - 2 * C.M, { family: 'Manrope', weight: 600, size: 9, tracking: 450, color: '#9cc3e8' }, 'center');
  b.page(1).bg('#ffffff');
  b.rect(0, 0, 6, C.H, teal);
  b.text('Daniel Okafor, CFA', 26, 24, 220, { family: 'Manrope', weight: 800, size: 15, color: navy }, 'start', { role: 'headline' });
  b.text('Senior Portfolio Manager', 26, 46, 220, { family: 'Manrope', weight: 500, size: 10, color: '#2f5e8c' });
  b.line(26, 74, 200, '#dfe6ee', 1);
  b.text('T   +1 (555) 410-2280\nE   d.okafor@northbeam.example\nA   400 Harbor Plaza, Floor 21\n     Boston, MA 02110', 26, 86, 230, { family: 'Manrope', weight: 500, size: 9.5, color: '#2a3647' }, 'start', { lh: 1.6 });
  b.qr('https://northbeam.example/okafor', 256, 24, 58);
  b.text('northbeam.example', 200, 160, 114, { family: 'Manrope', weight: 700, size: 9, color: '#127a73' }, 'end');
}, 2);

tpl('pr-card-florist-botanical', 'Botanical Florist Card', 'Business Cards', 'business-card', ['florist', 'botanical', 'flowers', 'sage', 'boutique'], (b) => {
  const green = '#33402c', sage = '#eef1e8';
  b.page(0).bg(sage);
  b.emoji('🌿', -18, 96, 110, { rot: 30 });
  b.emoji('🌸', 58, 142, 34, { rot: -10 });
  b.emoji('🌿', 252, -20, 100, { rot: 200 });
  b.text('Wild Fern', C.M, 62, C.W - 2 * C.M, { family: 'Italiana', size: 38, color: green }, 'center', { role: 'logo' });
  b.text('FLORAL STUDIO', C.M, 112, C.W - 2 * C.M, { family: 'Tenor Sans', size: 9, tracking: 500, color: '#5d6b52' }, 'center');
  b.page(1).bg(green);
  b.text('Rosa Delgado', C.M, 26, 200, { family: 'Italiana', size: 24, color: '#f3efe3' }, 'start', { role: 'headline' });
  b.text('FLORIST & FOUNDER', C.M, 60, 200, { family: 'Tenor Sans', size: 9, tracking: 300, color: '#c8d4b8' });
  b.text('(555) 602-1187\nrosa@wildfern.example\n77 Orchard Row, Asheville NC', C.M, 94, 210, { family: 'Tenor Sans', size: 9.5, color: '#f3efe3' }, 'start', { lh: 1.6 });
  b.text('Weddings · Events · Weekly blooms', C.M, 150, 220, { family: 'Tenor Sans', size: 9, color: '#c8d4b8' });
  b.emoji('🌸', 268, 22, 46, { rot: 12 });
}, 2);

tpl('pr-card-developer-terminal', 'Developer Terminal Card', 'Business Cards', 'business-card', ['developer', 'tech', 'code', 'dark', 'engineer'], (b) => {
  const green = '#7ee787';
  b.bg('#0d1117');
  [['#ff5f56', 0], ['#ffbd2e', 1], ['#27c93f', 2]].forEach(([c, i]) => b.circle(C.M + 5 + (i as number) * 14, C.M + 5, 4.5, c as string));
  b.text('~/portfolio', 100, C.M - 1, 136, { family: 'Space Mono', size: 9, color: '#8b949e' }, 'center');
  b.text('$ whoami', C.M, 46, 200, { family: 'Space Mono', size: 10, color: green });
  b.text('Priya Raman', C.M, 64, 260, { family: 'Space Mono', weight: 700, size: 20, color: '#ffffff' }, 'start', { role: 'headline' });
  b.text('Full-stack engineer · TypeScript & Go', C.M, 94, 290, { family: 'Space Mono', size: 9.5, color: '#c9d1d9' });
  b.text('$ contact --all', C.M, 118, 200, { family: 'Space Mono', size: 10, color: green });
  b.text('priya@raman.example\ngithub.com/priyaraman  ·  (555) 019-4420', C.M, 136, 290, { family: 'Space Mono', size: 9, color: '#c9d1d9' }, 'start', { lh: 1.5 });
  b.rect(296, 152, 8, 14, green);
});

// ================================================================== Invitations (5 × 7 in, 480 × 672)
const I = { W: 480, H: 672, M: 40 };

tpl('pr-invite-wedding-arch', 'Garden Arch Wedding', 'Invitations', 'invitation', ['wedding', 'elegant', 'script', 'gold', 'ceremony'], (b) => {
  const gold = '#a8834c', ink = '#3d3428';
  b.bg('#f7f3ec');
  outline(b, 'arch', 56, 52, 368, 560, gold, 1.2);
  outline(b, 'arch', 64, 60, 352, 544, gold, 0.5);
  b.text('TOGETHER WITH THEIR FAMILIES', 80, 150, 320, { family: 'Cormorant Garamond', weight: 600, size: 11, tracking: 300, color: '#6d5a3e' }, 'center');
  b.text('Eleanor', 80, 172, 320, { family: 'Great Vibes', size: 64, color: ink }, 'center', { lh: 1.15, role: 'headline' });
  b.text('&', 80, 250, 320, { family: 'Cormorant Garamond', italic: true, weight: 500, size: 30, color: gold }, 'center');
  b.text('James', 80, 280, 320, { family: 'Great Vibes', size: 64, color: ink }, 'center', { lh: 1.15, role: 'headline' });
  b.text('request the pleasure of your company\nat the celebration of their marriage', 80, 380, 320, { family: 'Cormorant Garamond', italic: true, size: 16, color: ink }, 'center', { lh: 1.4 });
  b.line(215, 440, 50, gold, 1);
  b.text('SATURDAY, THE TWELFTH OF JUNE\nTWO THOUSAND TWENTY-SEVEN\nAT FOUR O’CLOCK IN THE AFTERNOON', 80, 456, 320, { family: 'Cormorant Garamond', weight: 600, size: 11.5, tracking: 150, color: ink }, 'center', { lh: 1.6 });
  b.text('Fernhill Estate  ·  Sonoma, California', 80, 520, 320, { family: 'Cormorant Garamond', italic: true, weight: 500, size: 16, color: ink }, 'center');
  b.text('Dinner and dancing to follow', 80, 546, 320, { family: 'Cormorant Garamond', size: 12.5, color: '#6d5a3e' }, 'center');
  b.text('Kindly reply by May 1  ·  eleanorandjames.example', I.M, 628, I.W - 2 * I.M, { family: 'Cormorant Garamond', weight: 600, size: 11, tracking: 60, color: '#6d5a3e' }, 'center');
});

tpl('pr-invite-kids-birthday', 'Balloon Birthday Party', 'Invitations', 'invitation', ['birthday', 'kids', 'party', 'balloons', 'colorful'], (b) => {
  const ink = '#22223b';
  b.bg('#fff8e7');
  for (const [x, y, r, c] of [[60, 230, 8, '#ffbe0b'], [420, 260, 10, '#3a86ff'], [90, 420, 6, '#ff6b6b'], [400, 430, 7, '#8ac926'], [40, 330, 5, '#3a86ff'], [445, 360, 5, '#ff6b6b'], [60, 548, 5, '#8ac926'], [424, 546, 6, '#ffbe0b']] as const) b.circle(x, y, r, c);
  b.emoji('🎈', 30, 40, 110, { rot: -12 });
  b.emoji('🎈', 344, 28, 116, { rot: 12 });
  b.shape('star', 92, 186, 26, 26, '#ffbe0b', { rot: 12 });
  b.shape('star', 368, 178, 22, 22, '#ff6b6b', { rot: -8 });
  b.text('JOIN US TO CELEBRATE', 140, 80, 200, { family: 'Fredoka', weight: 600, size: 12, tracking: 220, color: '#2563c9' }, 'center');
  b.text('Leo is turning', I.M, 140, I.W - 2 * I.M, { family: 'Fredoka', weight: 700, size: 38, color: ink }, 'center', { role: 'headline' });
  b.text('5', I.M, 186, I.W - 2 * I.M, { family: 'Lilita One', size: 230, color: '#e63946' }, 'center', { lh: 1.05, effect: { k: 'shadow', dx: 6, dy: 6, blur: 0, color: col('#ffbe0b') } });
  b.text('Saturday, May 8  ·  2 – 5 pm', I.M, 452, I.W - 2 * I.M, { family: 'Fredoka', weight: 600, size: 21, color: ink }, 'center', { role: 'subhead' });
  b.text('Bounce Zone Play Center\n415 Maple Avenue, Riverton', I.M, 490, I.W - 2 * I.M, { family: 'Fredoka', size: 15, color: '#4a4e69' }, 'center', { lh: 1.4 });
  b.rect(100, 562, 280, 42, '#2563c9', { r: 21 });
  b.text('RSVP to Maya · (555) 318-2047', 100, 573, 280, { family: 'Fredoka', weight: 600, size: 14, color: '#ffffff' }, 'center', { role: 'cta' });
  b.text('Cake, games & a bouncy castle!', I.M, 618, I.W - 2 * I.M, { family: 'Fredoka', weight: 500, size: 13, color: '#4a4e69' }, 'center');
});

tpl('pr-invite-baby-shower', 'Teddy Bear Baby Shower', 'Invitations', 'invitation', ['baby shower', 'baby', 'pastel', 'soft', 'celebration'], (b) => {
  const ink = '#4a3535', sage = '#9caf88';
  b.bg('#f6ece6');
  b.circle(240, 168, 104, '#fbf6f2');
  outline(b, 'ellipse', 128, 56, 224, 224, sage, 1.5);
  b.emoji('🧸', 180, 106, 120);
  b.emoji('🌿', 18, 560, 90, { rot: 20 });
  b.emoji('🌿', 372, 560, 90, { rot: -110 });
  b.text('Oh, baby!', I.M, 286, I.W - 2 * I.M, { family: 'Dancing Script', weight: 700, size: 56, color: '#6b4f4f' }, 'center', { role: 'headline' });
  b.text('A BABY SHOWER HONORING', I.M, 370, I.W - 2 * I.M, { family: 'Nunito', weight: 700, size: 11, tracking: 300, color: '#7a5d5d' }, 'center');
  b.text('Amara Okafor', I.M, 390, I.W - 2 * I.M, { family: 'Playfair Display', italic: true, size: 30, color: ink }, 'center', { role: 'subhead' });
  [0, 1, 2].forEach((i) => b.circle(228 + i * 12, 450, 2.5, sage));
  b.text('Sunday, March 14, 2027  ·  1:00 pm\nThe Garden Room, 22 Willow Lane, Fairhaven', I.M, 466, I.W - 2 * I.M, { family: 'Nunito', weight: 600, size: 13.5, color: ink }, 'center', { lh: 1.6 });
  b.text('Registered at littlenest.example\nRSVP to Joy · (555) 482-1170 by March 1', I.M, 530, I.W - 2 * I.M, { family: 'Nunito', size: 12, color: '#6e5555' }, 'center', { lh: 1.55 });
});

tpl('pr-invite-supper-club', 'Autumn Supper Invitation', 'Invitations', 'invitation', ['dinner party', 'supper', 'autumn', 'menu', 'moody'], (b) => {
  const cream = '#f2e8d5', gold = '#c9a96e';
  b.bg('#1f2e25');
  b.box(22, 22, I.W - 44, I.H - 44, gold, 1);
  b.emoji('🍷', 208, 52, 64);
  b.text('YOU’RE INVITED TO', I.M, 136, I.W - 2 * I.M, { family: 'Inter Tight', weight: 600, size: 11, tracking: 380, color: gold }, 'center');
  b.text('An Autumn\nSupper', I.M, 158, I.W - 2 * I.M, { family: 'Fraunces', weight: 500, italic: true, size: 58, color: cream }, 'center', { lh: 1.0, role: 'headline' });
  b.line(200, 300, 80, gold, 1);
  b.text('THE MENU', I.M, 318, I.W - 2 * I.M, { family: 'Inter Tight', weight: 600, size: 10.5, tracking: 380, color: gold }, 'center');
  b.text('Roasted squash & sage soup\nWild mushroom risotto\nBraised short rib, root vegetables\nPear & almond tart', I.M, 342, I.W - 2 * I.M, { family: 'Fraunces', italic: true, size: 15, color: '#e3d7c0' }, 'center', { lh: 1.65, role: 'body' });
  b.line(200, 470, 80, gold, 1);
  b.text('Saturday, 21 November  ·  7:30 pm\nThe Lark House  ·  9 Quarry Lane', I.M, 488, I.W - 2 * I.M, { family: 'Inter Tight', weight: 500, size: 13.5, color: cream }, 'center', { lh: 1.6 });
  b.text('Hosted by Clara & Ben', I.M, 556, I.W - 2 * I.M, { family: 'Fraunces', italic: true, size: 15, color: gold }, 'center');
  b.text('RSVP by 10 November  ·  (555) 290-6611', I.M, 584, I.W - 2 * I.M, { family: 'Inter Tight', size: 11, color: '#bfb39b' }, 'center');
});

tpl('pr-invite-graduation-navy', 'Graduation Celebration', 'Invitations', 'invitation', ['graduation', 'class of 2027', 'navy', 'gold', 'party'], (b) => {
  const gold = '#d9b54a', soft = '#c9d4e8';
  b.bg('#13294b');
  b.box(22, 22, I.W - 44, I.H - 44, gold, 1.5);
  b.box(28, 28, I.W - 56, I.H - 56, gold, 0.5);
  b.emoji('🎓', 195, 54, 90);
  b.text('CLASS OF 2027', I.M, 160, I.W - 2 * I.M, { family: 'Cinzel', weight: 700, size: 15, tracking: 400, color: gold }, 'center');
  b.text('Graduation\nCelebration', I.M, 188, I.W - 2 * I.M, { family: 'Playfair Display', weight: 700, italic: true, size: 46, color: '#ffffff' }, 'center', { lh: 1.05, role: 'headline' });
  b.text('Please join us in honoring', I.M, 312, I.W - 2 * I.M, { family: 'Lora', italic: true, size: 15, color: soft }, 'center');
  b.text('Marcus J. Bennett', I.M, 338, I.W - 2 * I.M, { family: 'Cinzel', weight: 600, size: 28, color: gold }, 'center', { role: 'subhead' });
  b.text('Bachelor of Science in Mechanical Engineering\nWestbrook State University', I.M, 384, I.W - 2 * I.M, { family: 'Lora', size: 13, color: soft }, 'center', { lh: 1.5 });
  b.shape('diamond', 235, 446, 10, 10, gold);
  b.line(150, 451, 70, gold, 0.8);
  b.line(260, 451, 70, gold, 0.8);
  b.text('SATURDAY, MAY 22  ·  6 PM', I.M, 474, I.W - 2 * I.M, { family: 'Cinzel', weight: 600, size: 14, tracking: 150, color: '#ffffff' }, 'center');
  b.text('The Bennett Home  ·  88 Harmon Street, Dayton', I.M, 500, I.W - 2 * I.M, { family: 'Lora', size: 13.5, color: soft }, 'center');
  b.text('Dinner & dancing to follow\nRSVP to Denise · (555) 207-3392', I.M, 548, I.W - 2 * I.M, { family: 'Lora', italic: true, size: 12.5, color: gold }, 'center', { lh: 1.5 });
});

tpl('pr-invite-save-the-date', 'Calendar Save the Date', 'Invitations', 'invitation', ['save the date', 'wedding', 'calendar', 'minimal', 'blush'], (b) => {
  const ink = '#3b2f2f', blush = '#e8b4a8';
  b.bg('#fbf7f4');
  b.text('Save the Date', I.M, 54, I.W - 2 * I.M, { family: 'Bodoni Moda', italic: true, size: 48, color: ink }, 'center', { role: 'headline' });
  b.text('JUNE 2027', I.M, 132, I.W - 2 * I.M, { family: 'Bodoni Moda', weight: 600, size: 13, tracking: 450, color: '#8a5a50' }, 'center');
  const x0 = 70, cw = 340 / 7, y0 = 172, rh = 36;
  ['S', 'M', 'T', 'W', 'T', 'F', 'S'].forEach((d, i) => b.text(d, x0 + i * cw, y0, cw, { family: 'Inter Tight', weight: 700, size: 11, color: '#6e5f5f' }, 'center'));
  b.line(x0, y0 + 24, 340, '#e5d6d0', 1);
  for (let d = 1; d <= 30; d++) {
    const k = d + 1, c = k % 7, r = Math.floor(k / 7), cx = x0 + c * cw, cy = y0 + 34 + r * rh;
    if (d === 19) b.shape('heart', cx + cw / 2 - 19, cy - 9, 38, 36, blush);
    b.text(String(d), cx, cy, cw, { family: 'Inter Tight', weight: d === 19 ? 800 : 400, size: 13, color: d === 19 ? '#5a2a22' : ink }, 'center');
  }
  b.line(x0, 404, 340, '#e5d6d0', 1);
  b.text('Sofia & Mateo', I.M, 430, I.W - 2 * I.M, { family: 'Bodoni Moda', italic: true, size: 40, color: ink }, 'center', { role: 'subhead' });
  b.text('ARE GETTING MARRIED', I.M, 492, I.W - 2 * I.M, { family: 'Inter Tight', weight: 600, size: 11, tracking: 380, color: '#8a5a50' }, 'center');
  b.text('Quinta do Sol  ·  Sintra, Portugal', I.M, 522, I.W - 2 * I.M, { family: 'Bodoni Moda', size: 15, color: ink }, 'center');
  b.text('Formal invitation to follow  ·  sofiaandmateo.example', I.M, 604, I.W - 2 * I.M, { family: 'Inter Tight', size: 11, color: '#7a6a6a' }, 'center');
});

tpl('pr-invite-deco-gala', 'Art Deco Gala', 'Invitations', 'invitation', ['gala', 'corporate', 'black tie', 'art deco', 'gold'], (b) => {
  const gold = '#d4b46a';
  b.bg('#0e0e0e');
  b.box(22, 22, I.W - 44, I.H - 44, gold, 1);
  b.box(30, 30, I.W - 60, I.H - 60, gold, 0.5);
  for (const [x, y] of [[22, 22], [I.W - 22, 22], [22, I.H - 22], [I.W - 22, I.H - 22]]) b.shape('diamond', x - 7, y - 7, 14, 14, gold);
  const ox = 240, oy = 190, L = 120;
  for (let a = 200; a <= 340; a += 10) {
    const r = (a * Math.PI) / 180;
    b.line(ox + Math.cos(r) * (L / 2 + 30) - L / 2, oy + Math.sin(r) * (L / 2 + 30), L, gold, a % 20 ? 0.6 : 1.4, { rot: a });
  }
  b.shape('half-circle', 216, 166, 48, 24, gold);
  b.text('NORTHBEAM CAPITAL PARTNERS', I.M, 214, I.W - 2 * I.M, { family: 'Marcellus', size: 11, tracking: 400, color: gold }, 'center', { role: 'logo' });
  b.text('The Founders’\nGala', I.M, 244, I.W - 2 * I.M, { family: 'Italiana', size: 54, color: '#f5ecd7' }, 'center', { lh: 1.05, role: 'headline' });
  b.text('CELEBRATING TWENTY-FIVE YEARS', I.M, 372, I.W - 2 * I.M, { family: 'Marcellus', size: 11.5, tracking: 300, color: gold }, 'center');
  b.shape('diamond', 235, 402, 10, 10, gold);
  b.text('Friday, 4 December 2026\nSeven o’clock in the evening', I.M, 428, I.W - 2 * I.M, { family: 'Marcellus', size: 15, color: '#e9dfc7' }, 'center', { lh: 1.5 });
  b.text('The Grand Atrium, Harborview Hotel\n1 Pier Avenue, Boston', I.M, 490, I.W - 2 * I.M, { family: 'Marcellus', size: 13, color: '#bfb39a' }, 'center', { lh: 1.5 });
  b.text('BLACK TIE', I.M, 556, I.W - 2 * I.M, { family: 'Marcellus', size: 12, tracking: 450, color: gold }, 'center');
  b.text('RSVP by 13 November  ·  gala@northbeam.example', I.M, 584, I.W - 2 * I.M, { family: 'Marcellus', size: 11, color: '#bfb39a' }, 'center');
});

// ================================================================== Resumes (A4, 794 × 1123)
type Style = Parameters<B['text']>[4];
/** Rough line count for wrapped text (k = average glyph width in ems). */
const lineCount = (s: string, size: number, w: number, k = 0.5) => s.split('\n').reduce((n, p) => n + Math.max(1, Math.ceil((p.length * size * k) / w)), 0);
/** Add a text block and return the y just below it. */
function flow(b: B, s: string, x: number, y: number, w: number, st: Style, lh = 1.45, align: 'start' | 'center' | 'end' = 'start', k = 0.5, role?: 'body' | 'subhead' | 'headline') {
  b.text(s, x, y, w, st, align, { lh, ...(role ? { role } : {}) });
  return y + lineCount(s, st.size ?? 12, w, k) * (st.size ?? 12) * lh;
}
const R = { W: 794, H: 1123, M: 56 };

tpl('pr-resume-modern-sidebar', 'Modern Sidebar Resume', 'Resumes', 'resume', ['resume', 'cv', 'modern', 'two column', 'product manager'], (b) => {
  const side = '#1e293b', accent = '#0369a1', sky = '#7dd3fc', ink = '#0f172a', body = '#334155';
  b.bg('#ffffff');
  b.rect(0, 0, 260, R.H, side);
  b.frame(60, 56, 140, 140, 'ellipse', '#475569');
  const sx = 40, sw = 184;
  const sh = (t: string, y: number) => { b.text(t, sx, y, sw, { family: 'Inter', weight: 700, size: 11, tracking: 220, color: sky }); return y + 24; };
  const sbody = { family: 'Inter', size: 12, color: '#e2e8f0' };
  let y = sh('CONTACT', 232);
  y = flow(b, '(555) 214-8890\nolivia.chen@mail.example\nSeattle, WA\nlinkedin.com/in/oliviachen', sx, y, sw, sbody, 1.6) + 26;
  y = sh('SKILLS', y);
  for (const [s, v] of [['Product strategy', 0.95], ['User research', 0.9], ['Roadmapping', 0.9], ['Figma & prototyping', 0.8], ['SQL & analytics', 0.75], ['Experimentation', 0.85]] as const) {
    b.text(s, sx, y, sw, sbody);
    b.rect(sx, y + 20, sw, 4, '#334155', { r: 2 });
    b.rect(sx, y + 20, sw * v, 4, '#38bdf8', { r: 2 });
    y += 38;
  }
  y = sh('LANGUAGES', y + 20);
  y = flow(b, 'English — native\nMandarin — fluent\nSpanish — conversational', sx, y, sw, sbody, 1.6) + 26;
  y = sh('CERTIFICATIONS', y);
  y = flow(b, 'Certified Scrum Product Owner\nPragmatic Institute PMC-III', sx, y, sw, sbody, 1.6) + 26;
  y = sh('INTERESTS', y);
  flow(b, 'Trail running, wheel-thrown ceramics and a monthly sci-fi book club', sx, y, sw, sbody, 1.6);
  const mx = 300, mw = R.W - R.M - mx;
  b.text('Olivia Chen', mx, 52, mw, { family: 'Inter Tight', weight: 800, size: 42, color: ink }, 'start', { role: 'headline' });
  b.text('Senior Product Manager', mx, 108, mw, { family: 'Inter Tight', weight: 500, size: 17, color: accent }, 'start', { role: 'subhead' });
  b.line(mx, 150, mw, '#e2e8f0', 1);
  const mh = (t: string, yy: number) => { b.text(t, mx, yy, mw, { family: 'Inter Tight', weight: 800, size: 12, tracking: 220, color: ink }); return yy + 26; };
  const mbody = { family: 'Inter', size: 12.5, color: body };
  y = mh('PROFILE', 170);
  y = flow(b, 'Product leader with eight years of experience shipping B2C health and fintech products used by millions. I pair qualitative research with rigorous experimentation, and I love turning messy problems into clear, measurable roadmaps that engineering and design teams rally behind.', mx, y, mw, mbody, 1.55, 'start', 0.52, 'body') + 22;
  y = mh('EXPERIENCE', y);
  const jobs = [
    ['Senior Product Manager', 'Lumen Health  ·  Seattle, WA', '2022 – Present', '• Lead a 14-person squad owning onboarding and care plans for 2.3M members\n• Redesigned onboarding, lifting 30-day retention from 41% to 58%\n• Launched in-app coaching, now 18% of new subscription revenue\n• Hired and mentored three product managers'],
    ['Product Manager', 'Parcelly  ·  Remote', '2019 – 2022', '• Owned the shipment tracking experience across web, iOS and Android\n• Cut “where is my order” support tickets by 37% with proactive alerts\n• Introduced a quarterly discovery ritual adopted by four other teams'],
    ['Associate Product Manager', 'Brightside Bank  ·  Portland, OR', '2017 – 2019', '• Shipped savings goals, the bank’s most-used feature within six months\n• Ran 40+ A/B tests on the account-opening flow (+12% completion)'],
  ];
  for (const [t, c, d, bl] of jobs) {
    b.text(t!, mx, y, mw - 120, { family: 'Inter Tight', weight: 700, size: 14, color: ink });
    b.text(d!, mx + mw - 120, y + 1, 120, { family: 'Inter', weight: 500, size: 11, color: '#64748b' }, 'end');
    b.text(c!, mx, y + 21, mw, { family: 'Inter', weight: 500, size: 11.5, color: accent });
    y = flow(b, bl!, mx, y + 46, mw, mbody, 1.6, 'start', 0.5) + 24;
  }
  y = mh('EDUCATION', y + 4);
  for (const [dg, sc, yr] of [['MBA, Technology Management', 'University of Washington, Foster School of Business', '2017'], ['B.S. Cognitive Science', 'University of California, San Diego', '2013']]) {
    b.text(dg!, mx, y, mw - 80, { family: 'Inter Tight', weight: 700, size: 13, color: ink });
    b.text(yr!, mx + mw - 80, y + 1, 80, { family: 'Inter', weight: 500, size: 11, color: '#64748b' }, 'end');
    b.text(sc!, mx, y + 21, mw, mbody);
    y += 56;
  }
  y = mh('SELECTED TALKS', y + 14);
  flow(b, '• “Retention is a product problem” — Mind the Product, Seattle 2025\n• “Research on a shoestring” — ProductCamp Portland 2023', mx, y, mw, mbody, 1.6, 'start', 0.5);
});

tpl('pr-resume-classic-serif', 'Classic Serif Resume', 'Resumes', 'resume', ['resume', 'classic', 'traditional', 'finance', 'serif'], (b) => {
  const ink = '#1a1a1a', soft = '#4a4a4a';
  b.bg('#ffffff');
  b.text('JONATHAN AYERS', R.M, 56, R.W - 2 * R.M, { family: 'Libre Baskerville', weight: 700, size: 30, tracking: 200, color: ink }, 'center', { role: 'headline' });
  b.text('Chicago, IL  ·  (555) 380-1124  ·  j.ayers@mail.example  ·  linkedin.com/in/jayers', R.M, 104, R.W - 2 * R.M, { family: 'EB Garamond', size: 13, color: soft }, 'center');
  b.line(R.M, 136, R.W - 2 * R.M, ink, 1.5);
  b.line(R.M, 140, R.W - 2 * R.M, ink, 0.5);
  const w = R.W - 2 * R.M;
  const body = { family: 'EB Garamond', size: 14, color: ink };
  const head = (t: string, y: number) => { b.text(t, R.M, y, w, { family: 'Libre Baskerville', weight: 700, size: 12, tracking: 200, color: ink }); b.line(R.M, y + 24, w, '#9a9a9a', 0.6); return y + 38; };
  let y = head('PROFESSIONAL SUMMARY', 164);
  y = flow(b, 'Financial analyst with seven years of experience in FP&A, forecasting and board reporting for consumer goods and logistics companies. Known for building models that leadership actually uses, and for translating numbers into clear recommendations.', R.M, y, w, body, 1.5, 'start', 0.4, 'body') + 26;
  y = head('EXPERIENCE', y);
  const jobs = [
    ['Senior Financial Analyst', 'Meridian Foods, Chicago, IL', 'Mar 2021 – Present', '• Own the $480M annual operating plan and monthly forecast for three business units\n• Built a driver-based pricing model that identified $6.2M in margin opportunities\n• Prepare quarterly board materials and present variance analysis to the CFO'],
    ['Financial Analyst', 'Crescent Logistics, Milwaukee, WI', 'Jun 2018 – Feb 2021', '• Automated weekly KPI reporting in Power BI, saving 12 analyst hours per week\n• Partnered with operations on a fleet-leasing analysis that reduced costs by 9%\n• Rebuilt the capital-expenditure approval model used for all projects over $250K'],
    ['Financial Analyst Intern', 'Larkspur & Co., Chicago, IL', 'Summer 2017', '• Supported due diligence on two mid-market acquisitions\n• Built a comparable-companies database later adopted by the deal team'],
  ];
  for (const [t, c, d, bl] of jobs) {
    b.text(t!, R.M, y, w - 160, { family: 'EB Garamond', weight: 700, size: 14, color: ink });
    b.text(d!, R.M + w - 160, y, 160, { family: 'EB Garamond', size: 13, color: soft }, 'end');
    b.text(c!, R.M, y + 20, w, { family: 'EB Garamond', italic: true, size: 13, color: soft });
    y = flow(b, bl!, R.M + 4, y + 44, w - 4, body, 1.5, 'start', 0.4) + 20;
  }
  y = head('EDUCATION', y + 8);
  for (const [dg, sc, yr] of [['B.B.A. Finance, magna cum laude', 'University of Illinois Urbana-Champaign', '2018'], ['CFA Level II Candidate', 'CFA Institute', '2026']]) {
    b.text(dg!, R.M, y, w - 120, { family: 'EB Garamond', weight: 700, size: 14, color: ink });
    b.text(yr!, R.M + w - 120, y, 120, { family: 'EB Garamond', size: 13, color: soft }, 'end');
    b.text(sc!, R.M, y + 21, w, { family: 'EB Garamond', italic: true, size: 13.5, color: soft });
    y += 56;
  }
  y = head('SKILLS', y + 8);
  y = flow(b, 'Financial modeling  ·  Forecasting & budgeting  ·  Variance analysis  ·  Excel (advanced)  ·  Power BI  ·  SQL  ·  NetSuite  ·  Anaplan', R.M, y, w, body, 1.5, 'start', 0.4) + 26;
  y = head('AWARDS & ACTIVITIES', y);
  flow(b, '• Meridian Foods “Finance Partner of the Year,” 2024\n• Volunteer tax preparer, Chicago Community VITA program (2019 – present)\n• Mentor, Women in Finance Chicago early-career program, 2023 – present', R.M + 4, y, w - 4, body, 1.5, 'start', 0.4);
});

tpl('pr-resume-creative-coral', 'Creative Coral Resume', 'Resumes', 'resume', ['resume', 'creative', 'designer', 'colorful', 'portfolio'], (b) => {
  const coral = '#ff6b57', ink = '#1d1d1f', soft = '#4b4b52';
  b.bg('#ffffff');
  b.rect(0, 0, R.W, 236, coral);
  b.circle(700, 30, 90, '#ff8a7a');
  b.text('Sasha\nMoreno', R.M, 48, 460, { family: 'Syne', weight: 800, size: 54, color: ink }, 'start', { lh: 0.98, role: 'headline' });
  b.text('Brand & Visual Designer', R.M, 172, 460, { family: 'Syne', weight: 600, size: 17, color: ink }, 'start', { role: 'subhead' });
  b.frame(570, 120, 168, 168, 'ellipse', '#ffd2c9');
  b.text('sasha@moreno.example   ·   (555) 912-3381   ·   Brooklyn, NY   ·   moreno.design', R.M, 262, 500, { family: 'DM Sans', weight: 500, size: 11.5, color: soft });
  const lx = R.M, lw = 450, rx = 540, rw = R.W - R.M - rx;
  const head = (t: string, x: number, y: number, w: number) => { b.text(t, x, y, w, { family: 'Syne', weight: 800, size: 15, color: ink }); b.rect(x, y + 26, 28, 3, coral); return y + 42; };
  const body = { family: 'DM Sans', size: 12.5, color: soft };
  let y = head('About', lx, 312, lw);
  y = flow(b, 'Designer with six years of experience building brand identities, packaging and campaign systems for food, beauty and cultural clients. I care about bold ideas executed with craft — and about designing systems that teams can keep using long after launch.', lx, y, lw, body, 1.55, 'start', 0.5, 'body') + 24;
  y = head('Experience', lx, y, lw);
  const jobs = [
    ['Senior Designer', 'Fieldnote Studio', '2023 – now', 'Lead designer on identity and packaging projects for 20+ clients, including a rebrand that grew a snack client’s retail distribution from 300 to 1,100 stores.'],
    ['Designer', 'Brightwork Agency', '2020 – 2023', 'Concepted and produced integrated campaigns across print, social and out-of-home. Built the agency’s first shared brand-guidelines template.'],
    ['Junior Designer', 'Halcyon Museum', '2018 – 2020', 'Designed exhibition graphics, wayfinding and event collateral for six major exhibitions.'],
  ];
  b.line(lx + 6 - (jobs.length * 120 - 24) / 2, y + 9 + (jobs.length * 120 - 24) / 2, jobs.length * 120 - 24, '#ffd2c9', 2, { rot: 90 });
  for (const [t, c, d, s] of jobs) {
    b.circle(lx + 6, y + 9, 6, coral);
    b.text(`${t}  ·  ${c}`, lx + 24, y, lw - 110, { family: 'DM Sans', weight: 700, size: 13, color: ink });
    b.text(d!, lx + lw - 100, y + 1, 100, { family: 'DM Sans', weight: 500, size: 11, color: '#c2410c' }, 'end');
    flow(b, s!, lx + 24, y + 24, lw - 24, body, 1.6);
    y += 120;
  }
  y = head('Education', lx, y + 8, lw);
  b.text('BFA Graphic Design  ·  Pratt Institute', lx, y, lw - 80, { family: 'DM Sans', weight: 700, size: 13, color: ink });
  b.text('2018', lx + lw - 80, y + 1, 80, { family: 'DM Sans', weight: 500, size: 11, color: '#c2410c' }, 'end');
  b.text('Thesis: “Edible Type” — honourable mention, ADC Young Ones', lx, y + 22, lw, body);
  y = head('Selected projects', lx, y + 70, lw);
  flow(b, '• Oat & Ember — identity and packaging for a craft bakery chain\n• City of Makers — campaign for the 2025 Brooklyn Design Week\n• Kinfolk Tea — limited-edition tin series, sold out in 3 weeks', lx, y, lw, body, 1.7);
  let ry = head('Skills', rx, 312, rw);
  const chips = ['Brand identity', 'Packaging', 'Art direction', 'Typography', 'Illustration', 'Motion basics', 'Print production'];
  let cx = rx;
  for (const c of chips) {
    const w = c.length * 6.6 + 22;
    if (cx + w > rx + rw) { cx = rx; ry += 34; }
    b.rect(cx, ry, w, 26, '#fff0ed', { r: 13 });
    b.text(c, cx, ry + 6, w, { family: 'DM Sans', weight: 600, size: 11, color: '#9a2f1e' }, 'center');
    cx += w + 8;
  }
  ry = head('Tools', rx, ry + 56, rw);
  ry = flow(b, 'Figma, Illustrator, InDesign, Photoshop, After Effects, Procreate', rx, ry, rw, body, 1.6) + 28;
  ry = head('Recognition', rx, ry, rw);
  ry = flow(b, '• Type Directors Club, Certificate of Excellence, 2025\n• Dieline Awards, Packaging Shortlist, 2024\n• Speaker, Brooklyn Design Week, 2023', rx, ry, rw, body, 1.6, 'start', 0.55) + 28;
  ry = head('Languages', rx, ry, rw);
  ry = flow(b, 'English, Spanish, conversational Portuguese', rx, ry, rw, body, 1.6) + 28;
  ry = head('Interests', rx, ry, rw);
  flow(b, 'Risograph printing, community gardening, vintage type specimens', rx, ry, rw, body, 1.6);
});

tpl('pr-resume-minimal-grid', 'Minimal Grid Resume', 'Resumes', 'resume', ['resume', 'minimal', 'clean', 'engineer', 'simple'], (b) => {
  const ink = '#111111', grey = '#6b6b6b', body = { family: 'Inter', size: 12.5, color: '#2d2d2d' };
  b.bg('#ffffff');
  b.text('Noah Lindqvist', R.M, 64, 420, { family: 'Inter', weight: 600, size: 32, color: ink }, 'start', { role: 'headline' });
  b.text('Software Engineer — Infrastructure & Developer Tools', R.M, 112, 420, { family: 'Inter', size: 13, color: grey }, 'start', { role: 'subhead' });
  b.text('noah@lindqvist.example\n(555) 662-0937\nMinneapolis, MN\ngithub.com/nlindqvist', 520, 64, R.W - R.M - 520, { family: 'Inter', size: 11.5, color: '#2d2d2d' }, 'end', { lh: 1.55 });
  const lx = R.M, cx = 216, cw = R.W - R.M - cx;
  const sec = (t: string, y: number) => { b.line(R.M, y, R.W - 2 * R.M, '#e5e5e5', 1); b.text(t, lx, y + 20, 140, { family: 'Inter', weight: 600, size: 11.5, color: grey }); return y + 20; };
  let y = sec('About', 176);
  y = flow(b, 'Backend engineer with six years of experience building reliable build systems, CI pipelines and internal platforms. I like boring technology, fast feedback loops and clear documentation.', cx, y, cw, body, 1.65, 'start', 0.5, 'body') + 32;
  y = sec('Experience', y);
  const jobs = [
    ['Senior Software Engineer, Northwind Cloud', '2022 – Present', 'Lead engineer for the remote build cache; cut median CI time from 19 to 7 minutes for 600 engineers. Designed the migration of 2,000 services to a new deploy pipeline with zero downtime. On-call lead for the developer-platform rotation.'],
    ['Software Engineer, Fieldbook', '2019 – 2022', 'Built the internal feature-flag service (40k evaluations/sec). Mentored four junior engineers and ran the backend guild.'],
    ['Software Engineer Intern, Arcwell Systems', 'Summer 2018', 'Wrote a Go-based log shipper still used in production today.'],
  ];
  for (const [t, d, s] of jobs) {
    b.text(t!, cx, y, cw - 110, { family: 'Inter', weight: 600, size: 12.5, color: ink });
    b.text(d!, cx + cw - 110, y + 1, 110, { family: 'Inter', size: 11, color: grey }, 'end');
    y = flow(b, s!, cx, y + 24, cw, body, 1.65, 'start', 0.5) + 24;
  }
  y = sec('Education', y + 6);
  b.text('B.S. Computer Science, University of Minnesota', cx, y, cw - 80, { family: 'Inter', weight: 600, size: 12.5, color: ink });
  b.text('2019', cx + cw - 80, y + 1, 80, { family: 'Inter', size: 11, color: grey }, 'end');
  y = flow(b, 'Minor in Mathematics  ·  Dean’s List, six semesters', cx, y + 24, cw, body, 1.65) + 36;
  y = sec('Skills', y);
  const rows = [['Languages', 'Go, TypeScript, Python, Rust (learning)'], ['Infrastructure', 'Kubernetes, Terraform, Bazel, GitHub Actions, AWS'], ['Data', 'PostgreSQL, Redis, Kafka, ClickHouse']];
  for (const [k, v] of rows) { b.text(k!, cx, y, 120, { family: 'Inter', weight: 600, size: 11.5, color: ink }); b.text(v!, cx + 120, y, cw - 120, body); y += 28; }
  y = sec('Open source', y + 28);
  y = flow(b, 'Maintainer of “tidyci”, a CI config linter with 3.1k GitHub stars. Contributor to Bazel remote-execution APIs.', cx, y, cw, body, 1.65) + 36;
  y = sec('Talks', y);
  flow(b, '“Making CI boring again” — GopherCon 2025\n“Caching the world: lessons from a remote build cache” — Platform Summit 2024', cx, y, cw, body, 1.65);
});

tpl('pr-resume-academic-cv', 'Academic CV', 'Resumes', 'resume', ['cv', 'academic', 'research', 'professor', 'publications'], (b) => {
  const ink = '#1c2430', soft = '#4a5566', accent = '#7a1f2b';
  b.bg('#ffffff');
  b.text('Amelia R. Hart, PhD', R.M, 52, 480, { family: 'Source Serif 4', weight: 700, size: 30, color: ink }, 'start', { role: 'headline' });
  b.text('Assistant Professor of Molecular Biology\nDepartment of Biological Sciences, Eastbrook University', R.M, 98, 480, { family: 'Source Serif 4', italic: true, size: 13, color: soft }, 'start', { lh: 1.45, role: 'subhead' });
  b.text('214 Lyman Hall, Eastbrook, CT 06511\n(555) 471-0236\na.hart@eastbrook.example\nORCID 0000-0000-0000-0000', 520, 54, R.W - R.M - 520, { family: 'Source Serif 4', size: 11.5, color: soft }, 'end', { lh: 1.5 });
  b.rect(R.M, 150, R.W - 2 * R.M, 3, accent);
  const dx = R.M, dw = 96, cx = R.M + 112, cw = R.W - R.M - cx;
  const body = { family: 'Source Serif 4', size: 13, color: ink };
  const head = (t: string, y: number) => { b.text(t, R.M, y, R.W - 2 * R.M, { family: 'Source Serif 4', weight: 700, size: 13, tracking: 150, color: accent }); b.line(R.M, y + 23, R.W - 2 * R.M, '#d9d4cf', 0.6); return y + 36; };
  const rows = (items: Array<[string, string]>, y: number, gap = 10) => { for (const [d, t] of items) { b.text(d, dx, y + 1, dw, { family: 'Source Serif 4', size: 12, color: soft }); y = flow(b, t, cx, y, cw, body, 1.5, 'start', 0.5) + gap; } return y; };
  let y = head('ACADEMIC APPOINTMENTS', 172);
  y = rows([['2023 – now', 'Assistant Professor, Department of Biological Sciences, Eastbrook University'], ['2019 – 2023', 'Postdoctoral Fellow, Lindgren Institute for Cell Biology (advisor: Prof. K. Ibarra)']], y) + 10;
  y = head('EDUCATION', y);
  y = rows([['2019', 'PhD, Molecular and Cellular Biology, Halvorsen University\nDissertation: “Stress granule dynamics in neuronal ageing”'], ['2013', 'BSc (Hons), Biochemistry, University of Wellmont — first-class honours']], y) + 10;
  y = head('RESEARCH INTERESTS', y);
  y = flow(b, 'RNA–protein condensates, cellular stress responses, neurodegeneration, live-cell super-resolution imaging.', cx, y, cw, body, 1.5, 'start', 0.45) + 22;
  y = head('SELECTED PUBLICATIONS', y);
  y = rows([
    ['2025', 'Hart, A. R., Osei, M. & Ibarra, K. Phase separation of TDP-43 is tuned by RNA length in living neurons. Journal of Cell Science 138, jcs261044.'],
    ['2024', 'Moreau, L., Hart, A. R. et al. A toolkit for quantitative imaging of stress granules. Nature Methods 21, 1102–1113.'],
    ['2022', 'Hart, A. R. & Ibarra, K. Ageing alters condensate material properties in primary neurons. Cell Reports 39, 110871.'],
    ['2020', 'Hart, A. R., Chen, Y. & Patel, S. Translation arrest precedes granule assembly. Molecular Biology of the Cell 31, 2290–2302.'],
  ], y, 10) + 10;
  y = head('GRANTS & FELLOWSHIPS', y);
  y = rows([['2024 – 2029', 'NIH R35 Maximizing Investigators’ Research Award (PI), $1.9M'], ['2021 – 2023', 'Lindgren Foundation Postdoctoral Fellowship'], ['2015 – 2018', 'Graduate Research Fellowship, National Science Foundation']], y) + 10;
  y = head('TEACHING', y);
  y = rows([['2023 – now', 'BIOL 210 Cell Biology (180 students, lecture)'], ['2024 – now', 'BIOL 540 Advanced Topics in RNA Biology (graduate seminar)']], y) + 10;
  y = head('SERVICE', y);
  rows([['2024 – now', 'Graduate admissions committee, Biological Sciences'], ['2024 – now', 'Reviewer for eLife, Journal of Cell Biology and Cell Reports']], y);
});

tpl('pr-resume-entry-graduate', 'Graduate First Resume', 'Resumes', 'resume', ['resume', 'entry level', 'student', 'graduate', 'internship'], (b) => {
  const indigo = '#4338ca', ink = '#1e1b3a', soft = '#4b4a63';
  b.bg('#ffffff');
  b.rect(0, 0, R.W, 186, '#eef0ff');
  b.circle(R.W - 60, 40, 120, '#e0e3ff');
  b.text('Jordan Ellis', R.M, 50, 500, { family: 'Manrope', weight: 800, size: 38, color: ink }, 'start', { role: 'headline' });
  b.text('Communications graduate · Social media & content', R.M, 102, 500, { family: 'Manrope', weight: 600, size: 15, color: indigo }, 'start', { role: 'subhead' });
  b.text('jordan.ellis@mail.example   ·   (555) 733-0418   ·   Austin, TX   ·   linkedin.com/in/jordanellis', R.M, 140, R.W - 2 * R.M, { family: 'Manrope', weight: 500, size: 11.5, color: soft });
  const lx = R.M, lw = 452, rx = 548, rw = R.W - R.M - rx;
  const body = { family: 'Manrope', size: 13, color: soft };
  const head = (t: string, x: number, y: number, w: number) => { b.text(t, x, y, w, { family: 'Manrope', weight: 800, size: 13, tracking: 120, color: indigo }); return y + 30; };
  let y = head('PROFILE', lx, 216, lw);
  y = flow(b, 'Curious, organised communications graduate who has grown a student publication’s audience and run social campaigns for two local nonprofits. Looking for an entry-level role in content or social media where I can learn fast and ship often.', lx, y, lw, body, 1.6, 'start', 0.5, 'body') + 30;
  y = head('EDUCATION', lx, y, lw);
  b.text('B.A. Communication Studies, minor in Marketing', lx, y, lw - 70, { family: 'Manrope', weight: 700, size: 13, color: ink });
  b.text('2026', lx + lw - 70, y + 1, 70, { family: 'Manrope', weight: 600, size: 11.5, color: soft }, 'end');
  b.text('University of Texas at Austin  ·  GPA 3.7  ·  Dean’s List', lx, y + 20, lw, body);
  y = flow(b, 'Relevant courses: Digital Media Strategy, Data Storytelling, Persuasion, Visual Communication', lx, y + 46, lw, body, 1.6) + 30;
  y = head('PROJECTS', lx, y, lw);
  for (const [t, s] of [['The Daily Longhorn — Audience Editor', 'Grew newsletter subscribers from 1,200 to 4,800 in one year with a weekly “Campus in 5” format and A/B-tested subject lines.'], ['Capstone: “Shop Local ATX” campaign', 'Led a four-person team to plan and launch a social campaign for 12 small businesses; 210k impressions in six weeks.']]) {
    b.text(t!, lx, y, lw, { family: 'Manrope', weight: 700, size: 13, color: ink });
    y = flow(b, s!, lx, y + 22, lw, body, 1.6, 'start', 0.47) + 22;
  }
  y = head('EXPERIENCE', lx, y + 8, lw);
  for (const [t, d, s] of [['Social Media Intern, Greenline Food Bank', 'Summer 2025', 'Planned a three-month content calendar and produced short videos that doubled Instagram engagement.'], ['Barista & Shift Lead, Bean There Café', '2022 – 2026', 'Trained new staff and managed opening shifts while studying full time.']]) {
    b.text(t!, lx, y, lw - 100, { family: 'Manrope', weight: 700, size: 13, color: ink });
    b.text(d!, lx + lw - 100, y + 1, 100, { family: 'Manrope', weight: 600, size: 11.5, color: soft }, 'end');
    y = flow(b, s!, lx, y + 22, lw, body, 1.6, 'start', 0.47) + 22;
  }
  y = head('VOLUNTEERING', lx, y + 8, lw);
  flow(b, 'Austin Reads — weekly reading tutor for third graders (2023 – present)\nGreenline Food Bank — holiday drive volunteer coordinator (2024)', lx, y, lw, body, 1.7, 'start', 0.5);
  b.rect(rx - 20, 206, 1, R.H - 206 - R.M, '#e3e3ef');
  let ry = head('SKILLS', rx, 216, rw);
  let cx = rx;
  for (const c of ['Copywriting', 'Canva', 'Figma', 'CapCut', 'Google Analytics', 'Mailchimp', 'AP style', 'Photography']) {
    const w = c.length * 6.8 + 22;
    if (cx + w > rx + rw) { cx = rx; ry += 34; }
    b.rect(cx, ry, w, 26, '#eef0ff', { r: 13 });
    b.text(c, cx, ry + 6, w, { family: 'Manrope', weight: 600, size: 11, color: indigo }, 'center');
    cx += w + 8;
  }
  ry = head('CERTIFICATES', rx, ry + 54, rw);
  ry = flow(b, 'Google Analytics Certification (2025)\nHubSpot Content Marketing (2025)', rx, ry, rw, body, 1.6, 'start', 0.5) + 28;
  ry = head('ACTIVITIES', rx, ry, rw);
  ry = flow(b, 'Vice-president, Public Relations Student Society\nDebate team captain, 2024', rx, ry, rw, body, 1.6, 'start', 0.5) + 28;
  ry = head('LANGUAGES', rx, ry, rw);
  ry = flow(b, 'English (native)\nSpanish (professional)', rx, ry, rw, body, 1.6) + 28;
  ry = head('REFERENCES', rx, ry, rw);
  flow(b, 'Available on request', rx, ry, rw, body, 1.6);
});

// ================================================================== A4 documents (794 × 1123)
const D = { W: 794, H: 1123, M: 64 };

tpl('pr-doc-impact-report', 'Annual Impact Report', 'Documents', 'a4', ['report', 'annual report', 'nonprofit', 'cover', 'impact'], (b) => {
  const green = '#1d3b2a', moss = '#4f7a4a', cream = '#f5f0e6', ink = '#1f2a22', soft = '#4d5a50';
  b.page(0).bg(cream);
  b.photo('picsum:1043', 0, 0, D.W, 700, { filter: 'matte', filterAmount: 0.5 });
  b.rect(0, 0, D.W, 700, lin(90, '#1d3b2a00', '#1d3b2a00', '#1d3b2acc'));
  b.text('RIVERBEND CONSERVANCY', D.M, 56, 400, { family: 'Inter Tight', weight: 700, size: 13, tracking: 300, color: '#ffffff' }, 'start', { role: 'logo', effect: { k: 'shadow', dx: 0, dy: 1, blur: 6, color: col('#00000080') } });
  b.text('2026', D.M, 560, 300, { family: 'Fraunces', weight: 700, size: 96, color: '#ffffff' }, 'start', { lh: 1.0 });
  b.text('Impact\nReport', D.M, 740, 520, { family: 'Fraunces', weight: 600, size: 92, color: green }, 'start', { lh: 0.98, role: 'headline' });
  b.text('Protecting rivers, forests and the communities that depend on them.', D.M, 940, 460, { family: 'Inter Tight', size: 17, color: soft }, 'start', { lh: 1.45, role: 'subhead' });
  b.rect(D.W - D.M - 180, 760, 180, 180, moss, { r: 90 });
  b.text('12,400\nacres protected', D.W - D.M - 180, 812, 180, { family: 'Inter Tight', weight: 700, size: 17, color: '#ffffff' }, 'center', { lh: 1.3 });
  b.line(D.M, 1040, D.W - 2 * D.M, '#cfc6b4', 1);
  b.text('riverbend.example', D.M, 1056, 300, { family: 'Inter Tight', weight: 600, size: 12, color: soft });
  b.text('Published March 2027', D.W - D.M - 300, 1056, 300, { family: 'Inter Tight', size: 12, color: soft }, 'end');

  b.page(1).bg('#ffffff');
  b.rect(0, 0, D.W, 8, moss);
  b.text('RIVERBEND CONSERVANCY  ·  2026 IMPACT REPORT', D.M, 40, 500, { family: 'Inter Tight', weight: 700, size: 10.5, tracking: 200, color: moss });
  b.text('02', D.W - D.M - 60, 40, 60, { family: 'Inter Tight', weight: 700, size: 10.5, color: moss }, 'end');
  b.text('A year of\nputting down roots', D.M, 90, 640, { family: 'Fraunces', weight: 600, size: 46, color: green }, 'start', { lh: 1.05, role: 'headline' });
  b.text('In 2026 more than 3,200 volunteers joined us to restore riverbanks, plant native trees and monitor water quality across the Riverbend watershed. Thanks to their work and your support, we protected more land than in any year since our founding.', D.M, 216, D.W - 2 * D.M, { family: 'Inter Tight', size: 15, color: soft }, 'start', { lh: 1.55, role: 'body' });
  const stats = [['12,400', 'acres of habitat\npermanently protected'], ['48,000', 'native trees and\nshrubs planted'], ['3,200', 'volunteers gave\n21,500 hours']];
  stats.forEach(([n, t], i) => {
    const x = D.M + i * 228;
    b.rect(x, 340, 210, 150, '#eef3ea', { r: 14 });
    b.text(n!, x + 22, 362, 170, { family: 'Fraunces', weight: 700, size: 40, color: green });
    b.text(t!, x + 22, 422, 170, { family: 'Inter Tight', weight: 500, size: 13, color: soft }, 'start', { lh: 1.4 });
  });
  b.text('Acres protected each year', D.M, 530, 320, { family: 'Inter Tight', weight: 700, size: 15, color: ink });
  b.chart('column', D.M, 560, 320, 260, ['2022', '2023', '2024', '2025', '2026'], [{ name: 'Acres', values: [1400, 1900, 2300, 2700, 4100], color: moss }]);
  { const c = last(b); if (c.type === 'chart') c.style = { ...c.style, family: 'Inter Tight', fontSize: 12, showLegend: false, textColor: col(soft) }; }
  const cx = D.M + 360, cw = D.W - D.M - cx;
  b.text('Where the money goes (%)', cx, 530, cw, { family: 'Inter Tight', weight: 700, size: 15, color: ink });
  b.chart('bar', cx, 560, cw, 260, ['Land', 'Restoration', 'Education', 'Operations'], [{ name: 'Share of spending (%)', values: [46, 31, 13, 10], color: '#a3b18a' }]);
  { const c = last(b); if (c.type === 'chart') c.style = { ...c.style, family: 'Inter Tight', fontSize: 12, showLegend: false, showValues: true, textColor: col(soft) }; }
  b.rect(D.M, 860, 6, 120, moss);
  b.text('“Watching the salmon come back to Alder Creek after twenty years was the moment I knew this work matters.”', D.M + 28, 860, D.W - 2 * D.M - 28, { family: 'Fraunces', italic: true, size: 21, color: green }, 'start', { lh: 1.4, role: 'quote' });
  b.text('— Maria Okonkwo, volunteer since 2019', D.M + 28, 950, 500, { family: 'Inter Tight', weight: 600, size: 13, color: soft });
  b.line(D.M, 1040, D.W - 2 * D.M, '#e2e2dc', 1);
  b.text('riverbend.example/impact', D.M, 1056, 300, { family: 'Inter Tight', size: 11.5, color: soft });
}, 2);

tpl('pr-doc-letterhead-studio', 'Studio Letterhead', 'Documents', 'a4', ['letterhead', 'letter', 'business', 'stationery', 'modern'], (b) => {
  const ink = '#1c1c1c', soft = '#55524d', accent = '#e2583e';
  b.bg('#ffffff');
  b.circle(D.M + 18, 72, 18, accent);
  b.circle(D.M + 40, 72, 18, '#1c1c1c', { opacity: 0.9 });
  b.text('Fieldwork', D.M + 72, 54, 260, { family: 'Bricolage Grotesque', weight: 800, size: 26, color: ink }, 'start', { role: 'logo' });
  b.text('Landscape architecture & urban design', D.M + 72, 88, 300, { family: 'Inter', size: 11, color: soft });
  b.text('48 Canal Street, Unit 3\nRotterdam 3011 BB\n+31 10 555 0144\nhello@fieldwork.example', D.W - D.M - 220, 52, 220, { family: 'Inter', size: 11, color: soft }, 'end', { lh: 1.55 });
  b.line(D.M, 140, D.W - 2 * D.M, '#e8e4dd', 1);
  const body = { family: 'Inter', size: 12.5, color: '#2b2b2b' };
  b.text('14 September 2026', D.M, 186, 300, body);
  b.text('Ms. Anouk de Vries\nHead of Public Space\nMunicipality of Westerhaven\nStationsplein 1, Westerhaven', D.M, 228, 360, body, 'start', { lh: 1.55 });
  b.text('Re: Harbour Park design competition — shortlist submission', D.M, 340, D.W - 2 * D.M, { family: 'Inter', weight: 700, size: 12.5, color: ink });
  b.text('Dear Ms. de Vries,\n\nThank you for inviting Fieldwork to the second stage of the Harbour Park competition. Please find enclosed our concept boards, planting strategy and preliminary cost plan.\n\nOur proposal treats the park as a working landscape: a tidal garden that floods and drains with the river, shaded promenades that link the new housing to the ferry stop, and a play meadow designed with children from the neighbouring school. We have paid particular attention to maintenance, using robust native planting and reclaimed harbour stone throughout.\n\nWe would welcome the chance to present the scheme in person and to walk the site with your team. I can be reached directly at the number above.\n\nKind regards,', D.M, 380, D.W - 2 * D.M, body, 'start', { lh: 1.6, role: 'body' });
  b.text('Sanne Verbeek', D.M, 740, 300, { family: 'Inter', weight: 700, size: 12.5, color: ink });
  b.text('Founding Partner, Fieldwork', D.M, 760, 300, { family: 'Inter', size: 12, color: soft });
  b.rect(0, D.H - 70, D.W, 70, '#f6f3ee');
  b.rect(0, D.H - 70, D.W, 4, accent);
  b.text('fieldwork.example   ·   KvK 00000000   ·   Rotterdam  ·  Copenhagen', D.M, D.H - 42, D.W - 2 * D.M, { family: 'Inter', weight: 500, size: 10.5, color: soft }, 'center');
});

tpl('pr-doc-certificate-achievement', 'Certificate of Achievement', 'Documents', 'a4', ['certificate', 'award', 'achievement', 'diploma', 'formal'], (b) => {
  const navy = '#14284b', gold = '#b8913a', soft = '#4a5468';
  b.bg('#fbf8f1');
  b.box(32, 32, D.W - 64, D.H - 64, navy, 6);
  b.box(48, 48, D.W - 96, D.H - 96, gold, 1.5);
  b.box(56, 56, D.W - 112, D.H - 112, gold, 0.6);
  for (const [x, y] of [[48, 48], [D.W - 48, 48], [48, D.H - 48], [D.W - 48, D.H - 48]]) b.shape('diamond', x - 12, y - 12, 24, 24, gold);
  b.text('NORTHFIELD ACADEMY OF SCIENCE', D.M + 40, 150, D.W - 2 * D.M - 80, { family: 'Cinzel', weight: 600, size: 15, tracking: 300, color: soft }, 'center', { role: 'logo' });
  b.text('Certificate', D.M, 210, D.W - 2 * D.M, { family: 'Cinzel', weight: 700, size: 64, color: navy }, 'center', { role: 'headline' });
  b.text('OF ACHIEVEMENT', D.M, 300, D.W - 2 * D.M, { family: 'Cinzel', weight: 600, size: 18, tracking: 600, color: gold }, 'center');
  b.text('This certificate is proudly presented to', D.M, 400, D.W - 2 * D.M, { family: 'EB Garamond', italic: true, size: 20, color: soft }, 'center');
  b.text('Isabella Martínez', D.M, 440, D.W - 2 * D.M, { family: 'Parisienne', size: 68, color: navy }, 'center', { role: 'subhead' });
  b.line(D.W / 2 - 220, 545, 440, gold, 1);
  b.text('for outstanding performance and first place in the\n2027 Regional Science Olympiad, Chemistry Division', D.M + 60, 572, D.W - 2 * D.M - 120, { family: 'EB Garamond', size: 19, color: '#2b3245' }, 'center', { lh: 1.5, role: 'body' });
  b.shape('seal', D.W / 2 - 75, 700, 150, 150, gold, { params: { points: 30 } });
  b.circle(D.W / 2, 775, 58, '#fbf8f1');
  outline(b, 'ellipse', D.W / 2 - 52, 723, 104, 104, gold, 1.5);
  b.text('2027', D.W / 2 - 50, 750, 100, { family: 'Cinzel', weight: 700, size: 24, color: navy }, 'center');
  b.text('FIRST PLACE', D.W / 2 - 50, 784, 100, { family: 'Cinzel', weight: 600, size: 9, tracking: 150, color: gold }, 'center');
  for (const [x, n, r] of [[120, 'Dr. Helen Shaw', 'Principal'], [D.W - 120 - 220, 'Marcus Webb', 'Olympiad Coordinator']] as const) {
    b.text(n, x, 900, 220, { family: 'Dancing Script', weight: 600, size: 26, color: navy }, 'center');
    b.line(x, 945, 220, navy, 1);
    b.text(`${n.toUpperCase()}  ·  ${r.toUpperCase()}`, x - 20, 956, 260, { family: 'Cinzel', size: 10, tracking: 100, color: soft }, 'center');
  }
  b.text('Awarded on 18 April 2027', D.M, 1000, D.W - 2 * D.M, { family: 'EB Garamond', italic: true, size: 15, color: soft }, 'center');
});

tpl('pr-doc-invoice-clean', 'Clean Invoice', 'Documents', 'a4', ['invoice', 'billing', 'business', 'freelance', 'table'], (b) => {
  const ink = '#111827', soft = '#4b5563', accent = '#0f766e';
  b.bg('#ffffff');
  b.rect(0, 0, D.W, 10, accent);
  b.rect(D.M, 60, 40, 40, accent, { r: 10 });
  b.text('B', D.M, 66, 40, { family: 'Space Grotesk', weight: 700, size: 24, color: '#ffffff' }, 'center');
  b.text('Bluefinch Studio', D.M + 54, 60, 300, { family: 'Space Grotesk', weight: 700, size: 20, color: ink }, 'start', { role: 'logo' });
  b.text('Brand & web design', D.M + 54, 86, 300, { family: 'Inter', size: 11.5, color: soft });
  b.text('INVOICE', D.W - D.M - 300, 56, 300, { family: 'Space Grotesk', weight: 700, size: 40, tracking: 60, color: ink }, 'end', { role: 'headline' });
  b.text('No. INV-2026-0418', D.W - D.M - 300, 108, 300, { family: 'Inter', weight: 600, size: 12, color: accent }, 'end');
  const lab = { family: 'Inter', weight: 700, size: 10.5, tracking: 150, color: '#6b7280' };
  const val = { family: 'Inter', size: 12, color: ink };
  b.text('BILLED TO', D.M, 170, 220, lab);
  b.text('Harbor & Vine Wine Bar\nAttn: Leila Haddad\n212 Pearl Street\nPortland, ME 04101', D.M, 192, 240, val, 'start', { lh: 1.55 });
  b.text('FROM', 330, 170, 200, lab);
  b.text('Bluefinch Studio LLC\n85 Fore Street, Suite 4\nPortland, ME 04101\nbilling@bluefinch.example', 330, 192, 220, val, 'start', { lh: 1.55 });
  b.text('ISSUED', D.W - D.M - 140, 170, 140, lab, 'end');
  b.text('12 Oct 2026', D.W - D.M - 140, 190, 140, val, 'end');
  b.text('DUE', D.W - D.M - 140, 226, 140, lab, 'end');
  b.text('11 Nov 2026', D.W - D.M - 140, 246, 140, { ...val, weight: 700 }, 'end');
  b.table(D.M, 330, D.W - 2 * D.M, 300, [
    ['Description', 'Qty', 'Rate', 'Amount'],
    ['Brand identity — logo, palette & type system', '1', '$3,200.00', '$3,200.00'],
    ['Menu design (food, wine & cocktail)', '3', '$450.00', '$1,350.00'],
    ['Website design — 5 page templates', '5', '$520.00', '$2,600.00'],
    ['Signage artwork & print management', '1', '$680.00', '$680.00'],
    ['Photography art direction (half day)', '4 h', '$95.00', '$380.00'],
  ]);
  styleTable(b, { family: 'Inter', size: 12, color: ink, border: '#e5e7eb', header: '#f0fdfa', headerColor: accent, padding: 12 }, [52, 10, 18, 20], { 1: 'center', 2: 'end', 3: 'end' });
  const tx = D.W - D.M - 260;
  const tot = [['Subtotal', '$8,210.00'], ['Tax (5.5%)', '$451.55']] as const;
  tot.forEach(([k, v], i) => { b.text(k, tx, 660 + i * 28, 130, { family: 'Inter', size: 12.5, color: soft }); b.text(v, tx + 130, 660 + i * 28, 130, { family: 'Inter', size: 12.5, color: ink }, 'end'); });
  b.rect(tx - 20, 728, 280, 52, accent, { r: 8 });
  b.text('Total due', tx, 744, 130, { family: 'Inter', weight: 700, size: 14, color: '#ffffff' });
  b.text('$8,661.55', tx + 100, 740, 144, { family: 'Space Grotesk', weight: 700, size: 20, color: '#ffffff' }, 'end', { role: 'price' });
  b.text('PAYMENT DETAILS', D.M, 860, 300, lab);
  b.text('Bank transfer to Bluefinch Studio LLC\nAccount 0000 1234 5678  ·  Routing 000000000\nPlease include the invoice number as reference.', D.M, 882, 380, val, 'start', { lh: 1.6 });
  b.text('NOTES', 480, 860, 250, lab);
  b.text('Thank you for your business — it was a pleasure working on Harbor & Vine. Payment is due within 30 days.', 480, 882, D.W - D.M - 480, { family: 'Inter', size: 12, color: soft }, 'start', { lh: 1.6 });
  b.line(D.M, 1040, D.W - 2 * D.M, '#e5e7eb', 1);
  b.text('bluefinch.example  ·  (555) 230-7716', D.M, 1056, D.W - 2 * D.M, { family: 'Inter', size: 11, color: soft }, 'center');
});

tpl('pr-doc-community-newsletter', 'Community Newsletter', 'Documents', 'a4', ['newsletter', 'community', 'school', 'bulletin', 'news'], (b) => {
  const ink = '#1f2937', soft = '#4b5563', red = '#c2410c', paper = '#fffdf8';
  b.bg(paper);
  b.text('THE MAPLE LEAF', D.M, 48, D.W - 2 * D.M, { family: 'Playfair Display', weight: 900, size: 58, tracking: 40, color: ink }, 'center', { role: 'logo' });
  b.line(D.M, 132, D.W - 2 * D.M, ink, 2);
  b.text('MAPLE GROVE NEIGHBOURHOOD NEWS   ·   ISSUE 27   ·   OCTOBER 2026', D.M, 142, D.W - 2 * D.M, { family: 'Inter', weight: 600, size: 10.5, tracking: 200, color: soft }, 'center');
  b.line(D.M, 168, D.W - 2 * D.M, ink, 0.8);
  const lw = 430, rx = D.M + lw + 30, rw = D.W - D.M - rx;
  b.photo('picsum:838', D.M, 192, lw, 260, { filter: 'warm', filterAmount: 0.4 });
  b.text('Photo: the Okafor family at last month’s street fair', D.M, 458, lw, { family: 'Inter', italic: true, size: 10.5, color: soft });
  b.text('New playground opens on Linden Square', D.M, 486, lw, { family: 'Playfair Display', weight: 700, size: 30, color: ink }, 'start', { lh: 1.12, role: 'headline' });
  b.text('After two years of fundraising and three weekends of volunteer build days, the new Linden Square playground opens to families on Saturday, 17 October. The design was chosen by local children, who voted for a climbing tower, a sensory garden and — by a wide margin — the longest slide in the district.\n\nThe Residents’ Association thanks the 140 neighbours who donated, and the Brookside Hardware team for supplying materials at cost. Join us at 10 am for the ribbon cutting, followed by free lemonade and face painting.', D.M, 570, lw, { family: 'Lora', size: 13, color: ink }, 'start', { lh: 1.6, role: 'body' });
  b.rect(rx, 192, rw, 330, '#fdf0e6', { r: 6 });
  b.text('DATES FOR THE DIARY', rx + 18, 210, rw - 36, { family: 'Inter', weight: 800, size: 11, tracking: 200, color: red });
  const dates = [['17 OCT', 'Playground opening, 10 am'], ['24 OCT', 'Pumpkin carving at the library'], ['31 OCT', 'Trick-or-treat trail, 5–7 pm'], ['7 NOV', 'Leaf-raking volunteer day'], ['14 NOV', 'Residents’ meeting, 7 pm']];
  dates.forEach(([d, t], i) => {
    b.text(d!, rx + 18, 244 + i * 54, 60, { family: 'Inter', weight: 800, size: 12, color: ink });
    b.text(t!, rx + 84, 244 + i * 54, rw - 102, { family: 'Inter', size: 12, color: soft }, 'start', { lh: 1.4 });
  });
  b.text('Neighbour\nof the month', rx, 546, rw, { family: 'Playfair Display', weight: 700, size: 20, color: ink }, 'start', { lh: 1.15 });
  b.text('Retired teacher Dorothy Kim has run the Saturday reading circle for six years. Thank you, Dorothy!', rx, 604, rw, { family: 'Lora', size: 12.5, color: soft }, 'start', { lh: 1.55 });
  b.photo('picsum:835', rx, 740, rw, 140, { r: 4 });
  b.text('Recipe swap: share your favourite autumn dish at mapleleaf.example', rx, 890, rw, { family: 'Lora', italic: true, size: 12, color: soft }, 'start', { lh: 1.5 });
  b.line(D.M, 800, lw, '#d6d0c4', 1);
  b.text('Garden club’s bulb sale is back', D.M, 818, lw, { family: 'Playfair Display', weight: 700, size: 21, color: ink });
  b.text('Tulips, daffodils and alliums will be on sale outside the community hall every Saturday morning in October. All proceeds go to planting the new pollinator strip along Elm Walk — volunteers with trowels are very welcome.', D.M, 856, lw, { family: 'Lora', size: 13, color: ink }, 'start', { lh: 1.6 });
  b.line(D.M, 1000, D.W - 2 * D.M, ink, 0.8);
  b.text('Published by the Maple Grove Residents’ Association  ·  news@mapleleaf.example  ·  Contributions welcome by the 20th of each month', D.M, 1014, D.W - 2 * D.M, { family: 'Inter', size: 10.5, color: soft }, 'center', { lh: 1.5 });
});

// ================================================================== US Letter documents (816 × 1056)
const L = { W: 816, H: 1056, M: 72 };

tpl('pr-letter-letterhead-law', 'Classic Law Letterhead', 'Documents', 'letter', ['letterhead', 'law firm', 'formal', 'stationery', 'serif'], (b) => {
  const navy = '#1b2a41', gold = '#a8874f', soft = '#545b66';
  b.bg('#ffffff');
  b.rect(0, 0, 14, L.H, navy);
  b.rect(14, 0, 3, L.H, gold);
  b.text('HOLLIS & PARK', L.M, 60, 420, { family: 'Cormorant Garamond', weight: 700, size: 30, tracking: 250, color: navy }, 'start', { role: 'logo' });
  b.text('ATTORNEYS AT LAW', L.M, 100, 420, { family: 'Montserrat', weight: 500, size: 9.5, tracking: 450, color: '#86662f' });
  b.text('1200 Market Street, Suite 900\nPhiladelphia, PA 19107\n(555) 410-8800\nhollispark.example', L.W - L.M - 240, 58, 240, { family: 'Cormorant Garamond', weight: 500, size: 13, color: soft }, 'end', { lh: 1.4 });
  b.line(L.M, 150, L.W - 2 * L.M, gold, 1);
  const body = { family: 'Cormorant Garamond', weight: 500, size: 15, color: '#1f2530' };
  b.text('October 5, 2026', L.M, 196, 300, body);
  b.text('Mr. Thomas Reilly\nReilly Family Bakery, LLC\n44 Spruce Avenue\nCamden, NJ 08102', L.M, 240, 360, body, 'start', { lh: 1.4 });
  b.text('Re: Commercial lease renewal — 44 Spruce Avenue', L.M, 350, L.W - 2 * L.M, { ...body, weight: 700 });
  b.text('Dear Mr. Reilly,\n\nThank you for meeting with us last week. As discussed, we have reviewed the proposed five-year renewal of your lease and have prepared a summary of recommended changes, enclosed with this letter.\n\nIn brief, we recommend capping annual rent increases at three percent, clarifying responsibility for roof and HVAC repairs, and adding a renewal option for a further five years on the same terms. We believe the landlord is likely to accept these points.\n\nPlease review the summary at your convenience. We would be glad to discuss any questions by phone, or to meet again before we respond to the landlord’s counsel on October 20.\n\nSincerely,', L.M, 392, L.W - 2 * L.M, body, 'start', { lh: 1.5, role: 'body' });
  b.text('Catherine Park', L.M, 812, 300, { ...body, weight: 700 });
  b.text('Partner', L.M, 834, 300, { ...body, size: 14, color: soft });
  b.line(L.M, L.H - 72, L.W - 2 * L.M, '#e3ddd1', 1);
  b.text('PHILADELPHIA   ·   NEW YORK   ·   WILMINGTON', L.M, L.H - 56, L.W - 2 * L.M, { family: 'Montserrat', weight: 500, size: 9, tracking: 300, color: soft }, 'center');
});

tpl('pr-letter-project-proposal', 'One-Page Proposal', 'Documents', 'letter', ['proposal', 'business', 'project', 'agency', 'quote'], (b) => {
  const ink = '#16181d', soft = '#4a4f5a', accent = '#5b3df5';
  b.bg('#ffffff');
  b.rect(0, 0, L.W, 210, '#f3f1ff');
  b.text('PROPOSAL  ·  NO. 2026-031', L.M, 52, 400, { family: 'Instrument Sans', weight: 700, size: 11, tracking: 250, color: accent });
  b.text('Website redesign for\nCedar & Stone Dental', L.M, 78, 520, { family: 'Instrument Sans', weight: 700, size: 34, color: ink }, 'start', { lh: 1.12, role: 'headline' });
  b.text('Prepared by\nOrbit Digital\n\nPrepared for\nDr. Mei Nakamura\n\nOctober 2026', L.W - L.M - 170, 52, 170, { family: 'Instrument Sans', size: 11, color: soft }, 'end', { lh: 1.35 });
  const head = (t: string, y: number) => { b.text(t, L.M, y, 200, { family: 'Instrument Sans', weight: 700, size: 14, color: ink }); b.rect(L.M, y + 24, 24, 3, accent); return y; };
  const cx = 250, cw = L.W - L.M - cx, body = { family: 'Instrument Sans', size: 12.5, color: soft };
  head('Overview', 250);
  b.text('Cedar & Stone’s current site is hard to use on phones, and new patients can’t book online. We will design and build a fast, accessible website that turns visitors into booked appointments and reflects the practice’s calm, family-friendly care.', cx, 250, cw, body, 'start', { lh: 1.6, role: 'body' });
  head('Scope', 370);
  b.text('• Discovery workshop and patient survey\n• Sitemap, wireframes and visual design for 8 pages\n• Online booking integrated with your scheduling system\n• Content migration, SEO setup and staff training', cx, 370, cw, body, 'start', { lh: 1.7 });
  head('Timeline', 500);
  b.table(cx, 500, cw, 150, [['Phase', 'Weeks', 'Outcome'], ['Discovery', '1 – 2', 'Goals, survey results, sitemap'], ['Design', '3 – 5', 'Approved page designs'], ['Build & launch', '6 – 9', 'Live site, trained staff']]);
  styleTable(b, { family: 'Instrument Sans', size: 12, color: ink, border: '#e4e2f3', header: '#f3f1ff', headerColor: accent, padding: 9 }, [28, 18, 54]);
  head('Investment', 690);
  b.text('Fixed fee of $18,400, billed 40% at kickoff, 30% at design approval and 30% at launch. Hosting and care plan optional at $240 per month.', cx, 690, cw, body, 'start', { lh: 1.6 });
  b.rect(cx, 770, cw, 64, '#f3f1ff', { r: 10 });
  b.text('Total project investment', cx + 20, 792, 260, { family: 'Instrument Sans', weight: 600, size: 13, color: ink });
  b.text('$18,400', cx + cw - 200, 784, 180, { family: 'Instrument Sans', weight: 700, size: 24, color: accent }, 'end', { role: 'price' });
  head('Next steps', 870);
  b.text('Sign below to accept, and we’ll schedule the discovery workshop within two weeks.', cx, 870, cw, body, 'start', { lh: 1.6 });
  b.line(cx, 960, 220, '#9aa0ab', 1);
  b.line(cx + 260, 960, cw - 260, '#9aa0ab', 1);
  b.text('Client signature', cx, 968, 220, { family: 'Instrument Sans', size: 11, color: soft });
  b.text('Date', cx + 260, 968, 120, { family: 'Instrument Sans', size: 11, color: soft });
});

tpl('pr-letter-concert-program', 'Concert Program', 'Documents', 'letter', ['program', 'concert', 'event', 'music', 'choir'], (b) => {
  const ink = '#2a1f1a', wine = '#7b2d26', soft = '#5f524a';
  b.bg('#fbf6ee');
  b.box(36, 36, L.W - 72, L.H - 72, wine, 1);
  b.text('THE HARTWELL CHAMBER CHOIR PRESENTS', L.M, 80, L.W - 2 * L.M, { family: 'Cormorant Garamond', weight: 700, size: 13, tracking: 400, color: wine }, 'center');
  b.text('Songs of Spring', L.M, 108, L.W - 2 * L.M, { family: 'Playfair Display', italic: true, size: 56, color: ink }, 'center', { role: 'headline' });
  b.text('Sunday, April 18, 2027  ·  4:00 pm  ·  St. Brendan’s Hall, Hartwell', L.M, 190, L.W - 2 * L.M, { family: 'Cormorant Garamond', weight: 500, size: 16, color: soft }, 'center');
  b.shape('diamond', L.W / 2 - 5, 232, 10, 10, wine);
  b.line(L.W / 2 - 110, 237, 90, wine, 0.8);
  b.line(L.W / 2 + 20, 237, 90, wine, 0.8);
  b.text('PROGRAM', L.M, 266, L.W - 2 * L.M, { family: 'Cormorant Garamond', weight: 700, size: 14, tracking: 500, color: wine }, 'center');
  const items: Array<[string, string] | string> = [
    'I. Morning',
    ['The Lark in the Clear Air', 'traditional, arr. E. Dunne'],
    ['Sing Joyfully', 'William Byrd'],
    ['Now Is the Month of Maying', 'Thomas Morley'],
    'II. Rain',
    ['Rain on the Green Hills', 'Clara Voss (world premiere)'],
    ['Water Night', 'Eric Whitacre'],
    'Intermission',
    'III. Bloom',
    ['Spring Song', 'Felix Mendelssohn'],
    ['The Blue Bird', 'Charles Villiers Stanford'],
    ['Seasons of Light', 'Ama Boateng'],
  ];
  let y = 310;
  const x0 = 150, w0 = L.W - 300;
  for (const it of items) {
    if (typeof it === 'string') {
      const intermission = it === 'Intermission';
      b.text(intermission ? '— Intermission —' : it, x0, y + 8, w0, { family: 'Playfair Display', italic: true, weight: intermission ? 400 : 700, size: intermission ? 14 : 16, color: intermission ? soft : wine }, intermission ? 'center' : 'start');
      y += 46;
    } else {
      b.text(it[0], x0, y, w0 * 0.55, { family: 'Cormorant Garamond', weight: 600, size: 18, color: ink });
      b.text(it[1], x0 + w0 * 0.45, y + 2, w0 * 0.55, { family: 'Cormorant Garamond', italic: true, size: 16, color: soft }, 'end');
      y += 36;
    }
  }
  b.line(L.M + 80, y + 26, L.W - 2 * L.M - 160, '#d9c9b6', 1);
  b.text('Elena Marsh, conductor   ·   David Osei, piano   ·   Ruth Abernathy, soprano', L.M, y + 52, L.W - 2 * L.M, { family: 'Cormorant Garamond', weight: 600, size: 15, color: ink }, 'center');
  b.text('Please silence your phones. Photography without flash is welcome during applause.\nThank you to the Hartwell Arts Council and our generous members for supporting this season.', L.M + 40, y + 96, L.W - 2 * L.M - 80, { family: 'Cormorant Garamond', italic: true, size: 15, color: soft }, 'center', { lh: 1.5 });
  b.text('hartwellchoir.example', L.M, L.H - 92, L.W - 2 * L.M, { family: 'Cormorant Garamond', weight: 700, size: 13, tracking: 200, color: wine }, 'center');
});

tpl('pr-letter-recipe-cards', 'Recipe Card Sheet', 'Documents', 'letter', ['recipe', 'cooking', 'cards', 'kitchen', 'printable'], (b) => {
  const ink = '#3a2a1e', soft = '#6a5646';
  b.bg('#ffffff');
  b.text('Cut along the dashed line', L.M, 30, 300, { family: 'Nunito', weight: 600, size: 10, color: '#6f6052' });
  b.line(0, L.H / 2, L.W, '#b9ab9b', 1, { dash: [6, 5] });
  const cards = [
    { y: 52, bg: '#fff4d6', band: '#f2b632', emoji: '🍋', title: 'Grandma Rose’s Lemon Bars', meta: 'MAKES 16  ·  PREP 20 MIN  ·  BAKE 45 MIN',
      ing: '1 cup butter, softened\n½ cup powdered sugar\n2 cups flour + 3 tbsp\n4 eggs\n1½ cups sugar\n⅔ cup fresh lemon juice\nZest of 2 lemons',
      steps: '1. Heat oven to 350 °F. Line a 9 × 13 in pan.\n2. Mix butter, powdered sugar and 2 cups flour; press into pan and bake 20 min.\n3. Whisk eggs, sugar, 3 tbsp flour, juice and zest.\n4. Pour over the warm crust; bake 22–25 min until set.\n5. Cool completely, dust with sugar and slice.' },
    { y: L.H / 2 + 12, bg: '#fde8e4', band: '#d9573f', emoji: '🍅', title: 'Weeknight Tomato Soup', meta: 'SERVES 4  ·  PREP 10 MIN  ·  COOK 30 MIN',
      ing: '2 tbsp olive oil\n1 onion, chopped\n3 garlic cloves\n2 cans (28 oz) whole tomatoes\n2 cups vegetable stock\n½ cup cream\nFresh basil, salt & pepper',
      steps: '1. Soften onion in oil for 8 min; add garlic for 1 min.\n2. Add tomatoes and stock; simmer 20 minutes.\n3. Blend until smooth, then stir in the cream.\n4. Season to taste and finish with torn basil.\n5. Serve with grilled cheese for dipping.' },
  ];
  for (const c of cards) {
    const x = L.M, w = L.W - 2 * L.M, h = L.H / 2 - 104;
    b.rect(x, c.y + 20, w, h, c.bg, { r: 16 });
    b.rect(x, c.y + 20, w, 14, c.band, { r: 0 });
    b.emoji(c.emoji, x + w - 110, c.y + 50, 80, { rot: 10 });
    b.text(c.title, x + 30, c.y + 52, w - 160, { family: 'Fraunces', weight: 700, size: 28, color: ink }, 'start', { role: 'headline' });
    b.text(c.meta, x + 30, c.y + 96, w - 160, { family: 'Nunito', weight: 800, size: 10.5, tracking: 180, color: soft });
    b.text('INGREDIENTS', x + 30, c.y + 132, 200, { family: 'Nunito', weight: 800, size: 11, tracking: 200, color: c.band === '#f2b632' ? '#8a5a00' : '#a63a26' });
    b.text(c.ing, x + 30, c.y + 156, 222, { family: 'Nunito', size: 13.5, color: ink }, 'start', { lh: 1.7 });
    b.line(x + 262 - 130, c.y + 290, 260, '#e3d5c3', 1, { rot: 90 });
    b.text('METHOD', x + 290, c.y + 132, 200, { family: 'Nunito', weight: 800, size: 11, tracking: 200, color: c.band === '#f2b632' ? '#8a5a00' : '#a63a26' });
    b.text(c.steps, x + 290, c.y + 156, w - 320, { family: 'Nunito', size: 13.5, color: ink }, 'start', { lh: 1.7, role: 'body' });
    b.text('FROM THE KITCHEN OF', x + 290, c.y + h - 30, 170, { family: 'Nunito', weight: 800, size: 10, tracking: 150, color: soft });
    b.line(x + 460, c.y + h - 18, w - 490, '#cbb9a4', 1);
  }
});
