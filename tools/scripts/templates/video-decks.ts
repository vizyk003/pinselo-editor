/** Templates: video-decks (see index.ts for categories). */
import { hex } from '@oc/engine';
import { B, lin, rad, solid, tpl } from './builder';

void [B, lin, rad, solid, tpl];

const YT = 'YouTube Thumbnails';
const YB = 'YouTube Banners';
const PR = 'Presentations';

/** The element most recently added (to fine-tune chart/table styling, which the builder leaves at defaults). */
const lastEl = (b: B) => { const v = Object.values(b.doc.elements); return v[v.length - 1]!; };
function chartStyle(b: B, o: { family: string; text: string; grid: string; legend?: boolean; values?: boolean; showGrid?: boolean; size?: number }) {
  const el = lastEl(b);
  if (el.type !== 'chart') return;
  el.style = { family: o.family, textColor: hex(o.text), gridColor: hex(o.grid), showLegend: o.legend ?? true, showValues: o.values ?? false, showGrid: o.showGrid ?? true, fontSize: o.size ?? 20 };
}
function gridFill(b: B, c: string) { const el = lastEl(b); if (el.type === 'grid') el.placeholderFill = solid(c); }
function tableStyle(b: B, o: { family: string; size: number; color: string; border: string; head: string; headColor: string; stripe?: string; pad?: number; bw?: number }) {
  const el = lastEl(b);
  if (el.type !== 'table') return;
  el.style = { family: o.family, size: o.size, color: hex(o.color), borderColor: hex(o.border), borderWidth: o.bw ?? 1.5, headerFill: solid(o.head), headerColor: hex(o.headColor), ...(o.stripe ? { stripeFill: solid(o.stripe) } : {}), padding: o.pad ?? 14 };
}

// =================================================================== YouTube thumbnails (1280×720)

tpl('vd-thumb-tutorial', 'Learn It Fast Tutorial', YT, 'yt-thumb', ['tutorial', 'how to', 'design', 'beginner', 'tech'], (b) => {
  b.bg(lin(135, '#0b1026', '#1b2a6b'));
  b.circle(1010, 380, 300, '#facc15');
  b.circle(1010, 380, 300, rad('#fde68a', '#facc15'), { opacity: 0.6 });
  b.emoji('💻', 790, 170, 430, { rot: -8 });
  b.emoji('⏱️', 1070, 70, 170, { rot: 12 });
  b.rect(64, 70, 330, 64, '#facc15', { r: 32 });
  b.text('BEGINNER GUIDE', 64, 84, 330, { family: 'Archivo', weight: 800, size: 30, color: '#0b1026', tracking: 60 }, 'center', { role: 'subhead' });
  b.text('LEARN\nFIGMA', 60, 170, 700, { family: 'Anton', size: 190, color: '#ffffff' }, 'start', { lh: 0.92, role: 'headline' });
  b.text('IN 10 MIN', 60, 520, 700, { family: 'Anton', size: 120, color: '#facc15' }, 'start', { effect: { k: 'shadow', dx: 6, dy: 6, blur: 0, color: hex('#000000') } });
});

tpl('vd-thumb-vlog', 'Week in the City Vlog', YT, 'yt-thumb', ['vlog', 'travel', 'lifestyle', 'city', 'diary'], (b) => {
  b.bg(lin(160, '#ff9a62', '#ff5e62'));
  b.rect(712, 72, 520, 600, '#b42318', { r: 36, opacity: 0.35 });
  b.photo('picsum:419', 690, 50, 520, 600, { clip: 'rounded', focus: [0.55, 0.5], filter: 'warm', filterAmount: 0.5 });
  b.text('a week in', 64, 120, 600, { family: 'Caveat', weight: 700, size: 110, color: '#fff7ed' }, 'start', { rot: -4 });
  b.text('LISBON', 50, 230, 640, { family: 'Anton', size: 230, color: '#ffffff' }, 'start', { role: 'headline', effect: { k: 'shadow', dx: 0, dy: 8, blur: 0, color: hex('#b42318') } });
  b.rect(64, 540, 250, 76, '#1f1300', { r: 12, rot: -3 });
  b.text('VLOG 07', 64, 553, 250, { family: 'Archivo Black', size: 40, color: '#ffd166' }, 'center', { rot: -3 });
  b.emoji('☀️', 330, 520, 120, { rot: 10 });
});

tpl('vd-thumb-review', 'Honest Product Review', YT, 'yt-thumb', ['review', 'tech', 'unboxing', 'gadget', 'product'], (b) => {
  b.bg(solid('#f8fafc'));
  b.rect(700, 0, 580, 720, '#e11d48');
  b.shape('triangle', 560, -40, 280, 800, '#e11d48', { rot: 0 });
  b.circle(990, 370, 250, '#ffffff', { opacity: 0.16 });
  b.emoji('🎧', 750, 140, 470, { rot: -10 });
  b.rect(64, 70, 330, 64, '#0f172a', { r: 8 });
  b.text('HONEST REVIEW', 64, 84, 330, { family: 'Archivo', weight: 800, size: 30, color: '#ffffff', tracking: 80 }, 'center', { role: 'subhead' });
  b.text('WORTH\n$299?', 60, 170, 640, { family: 'Anton', size: 200, color: '#0f172a' }, 'start', { lh: 0.95, role: 'headline' });
  for (let i = 0; i < 5; i++) b.shape('star', 66 + i * 82, 596, 70, 70, i < 3 ? '#f59e0b' : '#cbd5e1');
});

tpl('vd-thumb-gaming', 'Impossible Clutch Gaming', YT, 'yt-thumb', ['gaming', 'esports', 'neon', 'stream', 'highlights'], (b) => {
  b.bg(rad('#3b0764', '#0a0118'));
  b.shape('lightning', 1020, 30, 200, 300, '#a3e635', { rot: 14 });
  b.emoji('🎮', 780, 300, 380, { rot: 14 });
  b.emoji('🔥', 1080, 470, 170);
  b.text('1 VS 5', 60, 90, 900, { family: 'Russo One', size: 220, color: '#a3e635' }, 'start', { role: 'headline', effect: { k: 'shadow', dx: 0, dy: 0, blur: 30, color: hex('#65a30d') } });
  b.text('IMPOSSIBLE\nCLUTCH', 66, 370, 760, { family: 'Black Ops One', size: 100, color: '#ffffff' }, 'start', { lh: 1.0, effect: { k: 'shadow', dx: 6, dy: 6, blur: 0, color: hex('#c026d3') } });
});

tpl('vd-thumb-cooking', 'Ten-Minute Salad Recipe', YT, 'yt-thumb', ['cooking', 'recipe', 'food', 'quick', 'kitchen'], (b) => {
  b.bg(solid('#fff1d6'));
  b.shape('ring', 630, 30, 660, 660, '#f4a261', { params: { thickness: 0.06 } });
  b.photo('picsum:488', 660, 60, 600, 600, { clip: 'ellipse', filter: 'vivid', filterAmount: 0.4 });
  const pl = lastEl(b);
  if (pl.type === 'frame' && pl.content) pl.content.crop = { x: 0.28, y: 0.17, w: 0.44, h: 0.66 };
  b.text('10-MIN', 60, 100, 620, { family: 'Lilita One', size: 150, color: '#c1121f' }, 'start', { role: 'headline' });
  b.text('SALAD', 60, 280, 620, { family: 'Lilita One', size: 190, color: '#3d1f0f' });
  b.text('only 6 ingredients', 66, 530, 560, { family: 'Caveat', weight: 700, size: 70, color: '#c1121f' }, 'start', { rot: -3 });
  b.shape('burst', 1060, 470, 210, 210, '#ffbe0b', { rot: 10 });
  b.text('EASY!', 1060, 545, 210, { family: 'Lilita One', size: 48, color: '#3d1f0f' }, 'center', { rot: 10 });
});

tpl('vd-thumb-finance', 'Savings Challenge Finance', YT, 'yt-thumb', ['finance', 'money', 'savings', 'investing', 'budget'], (b) => {
  b.bg(lin(135, '#022c22', '#064e3b'));
  for (let i = 0; i < 5; i++) b.rect(780 + i * 92, 560 - i * 90, 64, 100 + i * 90, '#34d399', { r: 8, opacity: 0.25 + i * 0.15 });
  b.shape('arrow', 760, 180, 470, 150, '#bef264', { rot: -32 });
  b.emoji('💰', 1060, 70, 170, { rot: 10 });
  b.text('$0 → $10K', 60, 110, 820, { family: 'Anton', size: 170, color: '#bef264' }, 'start', { role: 'headline' });
  b.text('IN 12 MONTHS', 60, 320, 700, { family: 'Anton', size: 110, color: '#ffffff' });
  b.rect(64, 540, 470, 80, '#ffffff', { r: 40 });
  b.text('my exact budget system', 64, 558, 470, { family: 'Inter', weight: 700, size: 34, color: '#064e3b' }, 'center', { role: 'subhead' });
});

tpl('vd-thumb-travel', 'Budget Travel Guide', YT, 'yt-thumb', ['travel', 'iceland', 'budget', 'guide', 'adventure'], (b) => {
  b.bgPhoto('picsum:563', { focus: [0.6, 0.5], filter: 'crisp', filterAmount: 0.6 });
  b.rect(0, 0, 1280, 720, lin(0, '#0b2545', '#0b254500'), { opacity: 0.9 });
  b.text('ICELAND', 60, 70, 900, { family: 'Unbounded', weight: 900, size: 140, color: '#ffffff' }, 'start', { role: 'headline', effect: { k: 'shadow', dx: 0, dy: 6, blur: 18, color: hex('#0b2545') } });
  b.text('ON $80', 60, 250, 700, { family: 'Unbounded', weight: 900, size: 120, color: '#ffd166' }, 'start', { effect: { k: 'shadow', dx: 0, dy: 6, blur: 18, color: hex('#0b2545') } });
  b.text('A DAY', 60, 400, 700, { family: 'Unbounded', weight: 900, size: 90, color: '#ffffff' }, 'start', { effect: { k: 'shadow', dx: 0, dy: 6, blur: 18, color: hex('#0b2545') } });
  b.rect(64, 590, 330, 60, '#ffd166', { r: 30 });
  b.text('FULL BUDGET GUIDE', 64, 603, 330, { family: 'Archivo', weight: 800, size: 26, color: '#0b2545', tracking: 60 }, 'center');
});

tpl('vd-thumb-podcast', 'Podcast Episode Guest', YT, 'yt-thumb', ['podcast', 'interview', 'episode', 'talk', 'guest'], (b) => {
  b.bg(solid('#111111'));
  b.circle(1180, 120, 260, '#ff6b35', { opacity: 0.18 });
  b.frame(700, 150, 290, 290, 'ellipse', '#3a3a3a');
  b.frame(930, 300, 290, 290, 'ellipse', '#4a4a4a');
  b.shape('ring', 692, 142, 306, 306, '#ff6b35', { params: { thickness: 0.04 } });
  b.shape('ring', 922, 292, 306, 306, '#f7f7f7', { params: { thickness: 0.04 } });
  b.emoji('🎙️', 610, 470, 170, { rot: -12 });
  b.rect(64, 70, 200, 60, '#ff6b35', { r: 30 });
  b.text('EP. 42', 64, 82, 200, { family: 'Space Grotesk', weight: 700, size: 32, color: '#111111' }, 'center', { role: 'subhead' });
  b.text('THE\nBURNOUT\nMYTH', 60, 170, 640, { family: 'Space Grotesk', weight: 700, size: 136, color: '#ffffff' }, 'start', { lh: 0.92, role: 'headline' });
  b.text('with Dr. Lena Ortiz', 64, 620, 560, { family: 'Space Grotesk', weight: 500, size: 36, color: '#ff8a5c' });
});

// =================================================================== YouTube banners (2560×1440, safe area 1546×423 at 507,508)

tpl('vd-banner-tech', 'Code Channel Banner', YB, 'yt-banner', ['youtube', 'banner', 'tech', 'coding', 'channel'], (b) => {
  b.bg(lin(120, '#0a0f1f', '#141b3d', '#0a0f1f'));
  b.circle(2300, 200, 620, '#6366f1', { opacity: 0.25 });
  b.circle(250, 1300, 560, '#22d3ee', { opacity: 0.18 });
  for (let i = 0; i < 9; i++) b.rect(0, 140 + i * 140, 2560, 2, '#ffffff', { opacity: 0.04 });
  b.text('</>', 540, 560, 360, { family: 'Space Grotesk', weight: 700, size: 190, color: '#22d3ee' }, 'start');
  b.text('Pixel & Code', 900, 545, 1150, { family: 'Space Grotesk', weight: 700, size: 170, color: '#ffffff' }, 'start', { role: 'headline' });
  b.text('Web dev tutorials · new videos every Tuesday', 905, 780, 1150, { family: 'Inter', weight: 500, size: 52, color: '#a5b4fc' }, 'start', { role: 'subhead' });
});

tpl('vd-banner-kitchen', 'Home Kitchen Banner', YB, 'yt-banner', ['youtube', 'banner', 'cooking', 'food', 'recipes'], (b) => {
  b.bgPhoto('picsum:292', { focus: [0.6, 0.5], filter: 'warm', filterAmount: 0.5 });
  b.rect(0, 0, 2560, 1440, '#2a1406', { opacity: 0.5 });
  b.rect(420, 470, 1720, 500, '#fdf0e1', { r: 24, opacity: 0.94 });
  b.text('Sunday Kitchen', 507, 530, 1546, { family: 'Fraunces', weight: 800, size: 150, color: '#5a2a0c' }, 'center', { role: 'headline' });
  b.text('easy recipes for slow weekends', 507, 725, 1546, { family: 'Caveat', weight: 700, size: 72, color: '#b23a0a' }, 'center', { role: 'subhead' });
  b.text('NEW RECIPE EVERY SUNDAY', 507, 845, 1546, { family: 'Archivo', weight: 700, size: 32, color: '#5a2a0c', tracking: 240 }, 'center');
});

tpl('vd-banner-travel', 'Wander Travel Banner', YB, 'yt-banner', ['youtube', 'banner', 'travel', 'vlog', 'adventure'], (b) => {
  b.bgPhoto('picsum:507', { focus: [0.5, 0.5], filter: 'dusk', filterAmount: 0.5 });
  b.rect(0, 0, 2560, 1440, lin(90, '#1d1438', '#3d2a6d', '#1d1438'), { opacity: 0.45 });
  b.text('WANDER WITH MIA', 507, 580, 1546, { family: 'Unbounded', weight: 800, size: 112, color: '#ffffff' }, 'center', { role: 'headline', effect: { k: 'shadow', dx: 0, dy: 6, blur: 20, color: hex('#5b3e96') } });
  b.text('slow travel · budget tips · new vlog every Friday', 507, 770, 1546, { family: 'Inter', weight: 600, size: 52, color: '#ffffff' }, 'center', { role: 'subhead', effect: { k: 'shadow', dx: 0, dy: 3, blur: 12, color: hex('#5b3e96') } });
});

tpl('vd-banner-podcast', 'Talk Show Podcast Banner', YB, 'yt-banner', ['youtube', 'banner', 'podcast', 'talk', 'interview'], (b) => {
  b.bg(solid('#111111'));
  b.rect(0, 0, 2560, 1440, lin(90, '#111111', '#1f1f1f'));
  b.circle(2330, 720, 560, '#ff6b35');
  b.circle(2330, 720, 420, '#111111');
  b.circle(2330, 720, 280, '#ff6b35', { opacity: 0.35 });
  b.emoji('🎙️', 2200, 590, 260, { rot: -10 });
  b.text('THE LONG ANSWER', 507, 540, 1250, { family: 'Space Grotesk', weight: 700, size: 130, color: '#ffffff' }, 'start', { role: 'headline' });
  b.text('Honest conversations about work, health and the in-between.', 512, 700, 1300, { family: 'Inter', size: 48, color: '#d4d4d4' }, 'start', { lh: 1.3, role: 'subhead' });
  b.rect(512, 850, 400, 70, '#ff6b35', { r: 35 });
  b.text('NEW EPISODES WEEKLY', 512, 866, 400, { family: 'Space Grotesk', weight: 700, size: 30, color: '#111111' }, 'center');
});

// =================================================================== Presentations (16:9, 1920×1080)

// ------------------------------------------------------------------ 1. Startup pitch deck
tpl('vd-deck-pitch', 'Startup Pitch Deck', PR, 'presentation', ['pitch', 'startup', 'investor', 'business', 'deck'], (b) => {
  const INK = '#0e1a2b', MINT = '#5eead4', CORAL = '#ff7a59', CORAL_D = '#d9472b', TEAL = '#0d9488', OFF = '#f6f5f1', MUTED = '#5b6474';
  const H = 'Bricolage Grotesque', T = 'Inter';
  const head = (kicker: string, title: string, dark = false) => {
    b.text(kicker.toUpperCase(), 120, 110, 1200, { family: T, weight: 700, size: 26, tracking: 200, color: dark ? MINT : '#0f766e' }, 'start', { role: 'subhead' });
    b.text(title, 120, 160, 1500, { family: H, weight: 800, size: 84, color: dark ? '#ffffff' : INK }, 'start', { lh: 1.05, role: 'headline' });
  };
  const foot = (n: number, dark = false) => {
    b.text('Returna · Seed 2027', 120, 990, 600, { family: T, weight: 600, size: 20, color: dark ? '#94a3b8' : MUTED });
    b.text(String(n).padStart(2, '0'), 1600, 990, 200, { family: T, weight: 700, size: 20, color: dark ? '#94a3b8' : MUTED }, 'end');
  };

  b.page(0).bg(solid(INK));
  b.circle(1500, 540, 520, MINT, { opacity: 0.14 });
  b.circle(1500, 540, 360, MINT, { opacity: 0.22 });
  b.emoji('🥡', 1300, 340, 400, { rot: -8 });
  b.text('returna', 120, 150, 700, { family: H, weight: 800, size: 44, color: MINT }, 'start', { role: 'logo' });
  b.text('Takeout,\nwithout the trash.', 120, 340, 1180, { family: H, weight: 800, size: 112, color: '#ffffff' }, 'start', { lh: 1.0, role: 'headline' });
  b.text('Reusable packaging for food delivery — seed round, March 2027', 120, 680, 1000, { family: T, size: 36, color: '#cbd5e1' }, 'start', { lh: 1.4, role: 'subhead' });
  b.rect(120, 850, 300, 4, CORAL);
  b.text('Priya Nair, CEO · priya@returna.example', 120, 880, 900, { family: T, weight: 600, size: 24, color: '#94a3b8' });
  b.notes('Open with the one-line pitch. Pause after "without the trash" — let the room picture their own takeout bin. Introduce yourself and say we are raising a $2.5M seed round.');

  b.page(1).bg(solid(OFF));
  head('Agenda', 'What we’ll cover');
  ([['01', 'The problem', 'Why delivery packaging is broken'], ['02', 'Our solution', 'Deposit-based reusable containers'], ['03', 'Traction', 'Ten months of live data'], ['04', 'Team & the ask', 'Who we are and what we need']] as const).forEach(([n, t, d], i) => {
    const x = 120 + i * 430;
    b.rect(x, 380, 390, 400, '#ffffff', { r: 28 });
    b.text(n, x + 40, 420, 300, { family: H, weight: 800, size: 72, color: i % 2 ? CORAL_D : TEAL });
    b.text(t, x + 40, 600, 320, { family: H, weight: 700, size: 40, color: INK }, 'start', { lh: 1.1 });
    b.text(d, x + 40, 670, 320, { family: T, size: 26, color: MUTED }, 'start', { lh: 1.4 });
  });
  foot(2);
  b.notes('Quick map of the next 15 minutes. We will finish with the team and the ask, so hold investment questions until then.');

  b.page(2).bg(solid(OFF));
  head('The problem', 'Every order ends\nin the bin.');
  b.text('4.1B', 1000, 300, 800, { family: H, weight: 800, size: 260, color: CORAL_D }, 'start', { role: 'headline' });
  b.text('single-use containers thrown away by delivery customers in the EU each year', 1010, 640, 760, { family: T, size: 34, color: INK }, 'start', { lh: 1.4 });
  b.text('• Less than 9% is recycled — grease contaminates it\n• Restaurants pay €0.35–0.80 per container\n• 62% of diners say packaging waste bothers them', 120, 480, 840, { family: T, size: 30, color: '#334155' }, 'start', { lh: 1.7, role: 'body' });
  foot(3);
  b.notes('The headline number is from the 2026 EU packaging survey (cite in appendix). Stress the cost side: this is a margin problem for restaurants, not only a green one.');

  b.page(3).bg(solid('#ffffff'));
  head('Our solution', 'Borrow the box. Drop it back.');
  ([['🥡', 'Order', 'Restaurants pack meals in durable Returna containers at no extra cost.'], ['📱', 'Scan', 'Diners pay a €2 deposit in the delivery app — one tap, no account.'], ['♻️', 'Return', 'Drop boxes at 340+ return points. Deposit refunded instantly.']] as const).forEach(([e, t, d], i) => {
    const x = 120 + i * 570;
    b.rect(x, 380, 530, 520, i === 1 ? INK : OFF, { r: 32 });
    b.emoji(e, x + 50, 430, 130);
    b.text(t, x + 50, 600, 440, { family: H, weight: 800, size: 54, color: i === 1 ? '#ffffff' : INK });
    b.text(d, x + 50, 690, 430, { family: T, size: 28, color: i === 1 ? '#cbd5e1' : '#475569' }, 'start', { lh: 1.45 });
  });
  b.notes('Walk through the loop as a customer would experience it. Mention that we wash and redistribute containers from two hubs, with a 98% return rate in the pilot.');

  b.page(4).bg(solid(OFF));
  head('Traction', 'Ten months, 3× growth.');
  b.chart('column', 120, 330, 1080, 600, ['May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec', 'Jan', 'Feb'], [{ name: 'Orders in reusables (k)', values: [12, 15, 18, 21, 22, 26, 29, 33, 35, 38], color: '#14b8a6' }]);
  chartStyle(b, { family: T, text: '#334155', grid: '#e2e8f0', legend: false, size: 20 });
  ([['186', 'partner restaurants'], ['98%', 'container return rate'], ['€41k', 'monthly revenue (Feb)']] as const).forEach(([n, l], i) => {
    b.text(n, 1320, 340 + i * 200, 480, { family: H, weight: 800, size: 96, color: i === 1 ? CORAL_D : INK });
    b.text(l, 1324, 460 + i * 200, 480, { family: T, size: 26, color: MUTED });
  });
  foot(5);
  b.notes('Orders are in thousands per month. Growth is organic — no paid acquisition yet. The return rate is the metric investors ask about most; it beats our model assumption of 92%.');

  b.page(5).bg(solid('#ffffff'));
  head('Team', 'Operators who’ve done it before.');
  ([['Priya Nair', 'CEO', 'Ex-ops lead at a 2,000-restaurant delivery network'], ['Tomás Reyes', 'COO', 'Built reverse logistics for a national grocer'], ['Hana Kobayashi', 'CTO', 'Shipped payments at two fintech scale-ups']] as const).forEach(([n, r, d], i) => {
    const x = 120 + i * 570;
    b.frame(x, 360, 220, 220, 'ellipse', '#d9f5ef');
    b.text(n, x, 610, 520, { family: H, weight: 800, size: 44, color: INK });
    b.text(r, x, 670, 520, { family: T, weight: 700, size: 26, color: '#0f766e', tracking: 100 });
    b.text(d, x, 720, 470, { family: T, size: 28, color: '#475569' }, 'start', { lh: 1.4 });
  });
  foot(6);
  b.notes('Keep this tight: one sentence per founder about why they are uniquely suited. Replace the portrait frames with real headshots.');

  b.page(6).bg(solid(MINT));
  b.text('“', 120, 80, 300, { family: H, weight: 800, size: 320, color: INK });
  b.text('We cut packaging costs by a third in the first quarter — and customers actually thank us for it.', 120, 380, 1500, { family: H, weight: 700, size: 76, color: INK }, 'start', { lh: 1.15, role: 'quote' });
  b.frame(120, 800, 110, 110, 'ellipse', '#99e6d8');
  b.text('Marco Bellini', 260, 815, 900, { family: T, weight: 700, size: 32, color: INK });
  b.text('Owner, Trattoria Sole · partner since 2026', 260, 860, 900, { family: T, size: 26, color: '#134e4a' });
  b.notes('Read the quote slowly. Marco was our third partner and now uses Returna for 100% of delivery orders.');

  b.page(7).bg(solid(INK));
  b.circle(1650, 900, 500, MINT, { opacity: 0.1 });
  b.text('THE ASK', 120, 150, 800, { family: T, weight: 700, size: 26, tracking: 200, color: MINT });
  b.text('€2.5M to reach\n10 cities by 2028.', 120, 210, 1400, { family: H, weight: 800, size: 110, color: '#ffffff' }, 'start', { lh: 1.0, role: 'headline' });
  ([['45%', 'Washing hubs'], ['30%', 'City launches'], ['25%', 'Product & team']] as const).forEach(([n, l], i) => {
    b.text(n, 120 + i * 420, 600, 380, { family: H, weight: 800, size: 88, color: i === 0 ? CORAL : MINT });
    b.text(l, 124 + i * 420, 710, 380, { family: T, size: 28, color: '#cbd5e1' });
  });
  b.text('priya@returna.example · returna.example', 120, 900, 1200, { family: T, weight: 600, size: 30, color: '#ffffff' }, 'start', { role: 'cta' });
  b.notes('State the ask clearly and stop talking. Then open for questions. Have the financial model appendix ready.');
}, 8);

// ------------------------------------------------------------------ 2. Company profile
tpl('vd-deck-company', 'Studio Company Profile', PR, 'presentation', ['company profile', 'about us', 'architecture', 'business', 'elegant'], (b) => {
  const CREAM = '#f4efe6', FOREST = '#1f3d2b', SAGE = '#8fa98a', CLAY = '#b4562a', TXT = '#3f4a42';
  const H = 'Fraunces', T = 'Inter Tight';
  const head = (kicker: string, title: string, n: number) => {
    b.text(kicker, 140, 120, 800, { family: T, weight: 600, size: 26, tracking: 160, color: CLAY }, 'start', { role: 'subhead' });
    b.text(title, 140, 170, 1300, { family: H, weight: 400, size: 80, color: FOREST }, 'start', { lh: 1.1, role: 'headline' });
    b.line(140, 990, 1640, '#d8cfbf', 2);
    b.text('FERNHILL STUDIO', 140, 1010, 600, { family: T, weight: 600, size: 18, tracking: 200, color: TXT });
    b.text(`${n} / 08`, 1580, 1010, 200, { family: T, weight: 600, size: 18, color: TXT }, 'end');
  };

  b.page(0).bg(solid(FOREST));
  b.photo('picsum:17', 1060, 0, 860, 1080, { focus: [0.5, 0.5], filter: 'matte', filterAmount: 0.4 });
  b.text('FERNHILL STUDIO', 140, 140, 800, { family: T, weight: 600, size: 28, tracking: 300, color: SAGE }, 'start', { role: 'logo' });
  b.text('Landscapes\nthat grow\nwith you.', 140, 340, 860, { family: H, weight: 400, size: 120, color: CREAM }, 'start', { lh: 1.02, role: 'headline' });
  b.text('Company profile 2027', 140, 820, 800, { family: H, weight: 400, italic: true, size: 40, color: '#d6e2d3' }, 'start', { role: 'subhead' });
  b.notes('Welcome. Fernhill is a 24-person landscape architecture studio based in Bristol. This profile takes about ten minutes; leave it with the client afterwards.');

  b.page(1).bg(solid(CREAM));
  head('CONTENTS', 'Inside this profile', 2);
  ['Who we are', 'What we do', 'Selected projects', 'Studio in numbers', 'Working with us'].forEach((t, i) => {
    const y = 380 + i * 110;
    b.text(`0${i + 1}`, 140, y, 120, { family: H, weight: 400, italic: true, size: 44, color: CLAY });
    b.text(t, 280, y, 900, { family: H, weight: 400, size: 52, color: FOREST });
    b.line(280, y + 88, 900, '#d8cfbf', 2);
  });
  b.photo('picsum:255', 1300, 380, 480, 540, { clip: 'rounded', focus: [0.5, 0.5] });
  b.notes('Use this page to set expectations. If time is short, jump straight to selected projects.');

  b.page(2).bg(solid(CREAM));
  head('01 — WHO WE ARE', 'Small studio,\nlong view.', 3);
  b.text('Since 2011 we have designed parks, schoolyards and private gardens across the South West. We plant for the next fifty years, not the next photo shoot — favouring native species, rainwater-led design and materials sourced within 80 miles.', 140, 470, 820, { family: T, size: 32, color: TXT }, 'start', { lh: 1.55, role: 'body' });
  b.photo('picsum:324', 1060, 170, 720, 760, { clip: 'rounded', focus: [0.45, 0.5] });
  b.rect(1000, 760, 380, 200, FOREST, { r: 8 });
  b.text('15', 1040, 775, 140, { family: H, weight: 400, size: 96, color: CREAM });
  b.text('years designing\nliving spaces', 1170, 805, 200, { family: T, size: 24, color: '#d6e2d3' }, 'start', { lh: 1.35 });
  b.notes('Our origin story in one breath. Emphasise long-term thinking — it is our main differentiator against larger practices.');

  b.page(3).bg(solid('#ffffff'));
  head('02 — WHAT WE DO', 'Three ways we work', 4);
  ([['🌳', 'Public parks', 'Masterplans and detailed design for councils and community trusts.'], ['🏡', 'Private gardens', 'Bespoke gardens for homes, from courtyards to country estates.'], ['🌱', 'Rewilding', 'Ecological surveys and planting plans that restore biodiversity.']] as const).forEach(([e, t, d], i) => {
    const x = 140 + i * 560;
    b.rect(x, 400, 520, 500, CREAM, { r: 6 });
    b.emoji(e, x + 50, 450, 120);
    b.text(t, x + 50, 610, 420, { family: H, weight: 400, size: 50, color: FOREST });
    b.text(d, x + 50, 690, 420, { family: T, size: 28, color: TXT }, 'start', { lh: 1.5 });
  });
  b.notes('Each service has its own case-study deck — offer to send the relevant one after the meeting.');

  b.page(4).bg(solid(CREAM));
  head('03 — SELECTED PROJECTS', 'Recent work', 5);
  b.grid(['picsum:434', 'picsum:28', 'picsum:468'], 140, 350, 1640, 520, 'cols-3', { gap: 20, radius: 6 });
  gridFill(b, '#dfd6c6');
  ([['Harbourside Park', 'Bristol · 2026'], ['Millbrook Primary', 'Bath · 2025'], ['The Walled Garden', 'Somerset · 2025']] as const).forEach(([t, d], i) => {
    const x = 140 + i * 553;
    b.text(t, x, 890, 520, { family: H, weight: 400, size: 34, color: FOREST });
    b.text(d, x, 940, 520, { family: T, size: 22, color: TXT });
  });
  b.notes('Swap in your best three project photos. Tell one short story per project — the client, the challenge, the result.');

  b.page(5).bg(solid('#ffffff'));
  head('04 — STUDIO IN NUMBERS', 'Measured impact', 6);
  b.table(140, 380, 900, 480, [['Year', 'Projects', 'Trees planted', 'Area (ha)'], ['2023', '18', '1,240', '12.5'], ['2024', '22', '2,060', '19.8'], ['2025', '27', '3,410', '26.1'], ['2026', '31', '4,780', '34.0']]);
  tableStyle(b, { family: T, size: 28, color: TXT, border: '#e3dccf', head: FOREST, headColor: '#ffffff', stripe: '#faf7f1', pad: 18 });
  ([['92%', 'of clients return for a second project'], ['40%', 'average reduction in irrigation needs']] as const).forEach(([n, l], i) => {
    b.text(n, 1180, 390 + i * 250, 600, { family: H, weight: 400, size: 110, color: i ? CLAY : FOREST });
    b.text(l, 1186, 530 + i * 250, 520, { family: T, size: 28, color: TXT }, 'start', { lh: 1.4 });
  });
  b.notes('Numbers are from our 2026 impact report. The irrigation figure compares against conventional planting schemes of the same area.');

  b.page(6).bg(solid(SAGE));
  b.text('“Fernhill listened before they drew a single line. Three years on, the park is busier than ever — and it looks better every spring.”', 240, 280, 1440, { family: H, weight: 400, italic: true, size: 68, color: '#13261b' }, 'center', { lh: 1.25, role: 'quote' });
  b.line(900, 760, 120, '#13261b', 3);
  b.text('Dana Whitfield · Parks Officer, Bristol City Council', 240, 800, 1440, { family: T, weight: 600, size: 30, color: '#13261b' }, 'center');
  b.notes('Let the quote speak. If the client knows Harbourside Park, mention they can visit it.');

  b.page(7).bg(solid(FOREST));
  b.text('Let’s grow\nsomething together.', 140, 300, 1200, { family: H, weight: 400, size: 110, color: CREAM }, 'start', { lh: 1.05, role: 'headline' });
  b.text('hello@fernhill.example\n+44 117 555 0142\n12 Quay Street, Bristol', 140, 650, 900, { family: T, size: 34, color: '#d6e2d3' }, 'start', { lh: 1.6, role: 'body' });
  b.emoji('🌿', 1420, 560, 320, { rot: 14 });
  b.notes('Close with next steps: a site visit and a fixed-fee concept study. Leave contact details on screen during questions.');
}, 8);

// ------------------------------------------------------------------ 3. Marketing plan
tpl('vd-deck-marketing', 'Bold Marketing Plan', PR, 'presentation', ['marketing', 'plan', 'strategy', 'campaign', 'colorful'], (b) => {
  const VIO = '#4c1d95', CORAL = '#ff6f61', BUTTER = '#ffe8a3', OFF = '#fffaf2', TXT = '#2e1065', MUTED = '#5b4b7a';
  const H = 'Syne', T = 'DM Sans';
  const head = (kicker: string, title: string) => {
    b.rect(120, 110, 16, 16, CORAL, { r: 8 });
    b.text(kicker.toUpperCase(), 150, 100, 900, { family: T, weight: 700, size: 24, tracking: 160, color: MUTED }, 'start', { role: 'subhead' });
    b.text(title, 120, 150, 1680, { family: H, weight: 800, size: 72, color: TXT }, 'start', { lh: 1.05, role: 'headline' });
  };

  b.page(0).bg(solid(VIO));
  b.shape('blob', 1100, -120, 1000, 900, CORAL);
  b.shape('blob', 1350, 560, 700, 640, BUTTER, { rot: 40 });
  b.emoji('🍵', 1300, 220, 360, { rot: -10 });
  b.text('BRAMBLE & CO.', 120, 130, 900, { family: T, weight: 700, size: 28, tracking: 300, color: BUTTER }, 'start', { role: 'logo' });
  b.text('Marketing\nPlan 2027', 120, 300, 1150, { family: H, weight: 800, size: 130, color: '#ffffff' }, 'start', { lh: 0.98, role: 'headline' });
  b.text('From cult favourite to household name', 120, 680, 1000, { family: T, weight: 500, size: 40, color: '#ede9fe' }, 'start', { role: 'subhead' });
  b.notes('This is our plan for taking Bramble & Co. loose-leaf tea from 40 stockists to national retail in 2027. Presented by the brand team, January 2027.');

  b.page(1).bg(solid(OFF));
  head('Agenda', 'The plan in five parts');
  ['Where we are', 'Goals for 2027', 'Who we’re talking to', 'Channels & budget', 'Campaign calendar'].forEach((t, i) => {
    const y = 360 + i * 120;
    b.rect(120, y, 1680, 100, i === 0 ? VIO : '#ffffff', { r: 50 });
    b.text(`0${i + 1}`, 170, y + 26, 100, { family: H, weight: 800, size: 40, color: i === 0 ? BUTTER : '#c2410c' });
    b.text(t, 290, y + 24, 1200, { family: T, weight: 700, size: 44, color: i === 0 ? '#ffffff' : TXT });
  });
  b.notes('Five sections, roughly four minutes each. Budget is in section four if finance wants to jump ahead.');

  b.page(2).bg(solid(OFF));
  head('Goals for 2027', 'Three numbers that matter');
  ([['3×', 'stockists', '40 → 120 shops', CORAL], ['50k', 'subscribers', 'from 18k today', VIO], ['4.6', 'star rating', 'average across retailers', '#f59e0b']] as const).forEach(([n, t, d, c], i) => {
    const x = 120 + i * 570;
    b.rect(x, 380, 530, 540, c, { r: 40 });
    const fg = i === 1 ? '#ffffff' : TXT;
    b.text(n, x + 50, 440, 440, { family: H, weight: 800, size: 130, color: fg });
    b.text(t, x + 50, 690, 440, { family: H, weight: 800, size: 40, color: fg });
    b.text(d, x + 50, 755, 440, { family: T, size: 30, color: fg });
  });
  b.notes('Every activity in this plan should move one of these three numbers. If it doesn’t, we cut it.');

  b.page(3).bg(solid('#ffffff'));
  head('Audience', 'Meet Maya, 31');
  b.frame(120, 340, 560, 620, 'rounded', '#ede9fe');
  b.emoji('🧑‍💻', 280, 520, 240);
  b.text('Works hybrid in a creative job. Swapped her third coffee for something calmer. Buys on Instagram recommendations, stays for quality and the ritual.', 760, 350, 1040, { family: T, size: 36, color: TXT }, 'start', { lh: 1.5, role: 'body' });
  ([['Where she shops', 'Indie grocers, online subscriptions'], ['What she values', 'Taste, provenance, plastic-free'], ['How we reach her', 'Creators, sampling, newsletters']] as const).forEach(([k, v], i) => {
    const y = 640 + i * 105;
    b.text(k, 760, y, 380, { family: T, weight: 700, size: 28, color: '#c2410c' });
    b.text(v, 1150, y, 650, { family: T, size: 28, color: TXT });
    b.line(760, y + 80, 1040, '#ede9fe', 2);
  });
  b.notes('Maya is our primary persona, built from 1,200 customer survey responses. Secondary persona (gift buyers) is in the appendix.');

  b.page(4).bg(solid(OFF));
  head('Channels & budget', '£480k, where it goes');
  b.chart('donut', 120, 330, 760, 640, ['Creators & social', 'Retail sampling', 'Paid search', 'Email & CRM', 'PR & events'], [{ name: 'Budget', values: [34, 26, 18, 12, 10] }]);
  const el = lastEl(b);
  if (el.type === 'chart') el.series[0]!.color = hex(VIO);
  chartStyle(b, { family: T, text: TXT, grid: '#e5e7eb', legend: true, values: true, size: 22 });
  ([['34%', 'Creators & social', 'Always-on with 25 micro-creators'], ['26%', 'Retail sampling', 'In-store tastings at launch partners'], ['18%', 'Paid search', 'High-intent “loose leaf tea” terms']] as const).forEach(([n, t, d], i) => {
    const y = 360 + i * 200;
    b.text(n, 960, y + 4, 260, { family: H, weight: 800, size: 64, color: i === 0 ? VIO : '#c2410c' });
    b.text(t, 1230, y + 6, 580, { family: T, weight: 700, size: 34, color: TXT });
    b.text(d, 1230, y + 56, 580, { family: T, size: 28, color: MUTED });
  });
  b.notes('Budget split is a proposal; the creator line can flex by ±10% depending on Q1 results.');

  b.page(5).bg(solid('#ffffff'));
  head('Campaign calendar', 'A year of moments');
  ([['Q1', 'Slow January', 'New-year ritual campaign + subscription push'], ['Q2', 'Garden Party', 'Iced tea range launch, retail sampling tour'], ['Q3', 'Back to Calm', 'Workplace bundles and creator series'], ['Q4', 'Gift Season', 'Advent calendar and gift boxes']] as const).forEach(([q, t, d], i) => {
    const x = 120 + i * 430;
    b.rect(x, 400, 390, 12, [CORAL, VIO, '#f59e0b', '#10b981'][i]!, { r: 6 });
    b.text(q, x, 450, 390, { family: H, weight: 800, size: 72, color: TXT });
    b.text(t, x, 560, 390, { family: T, weight: 700, size: 36, color: TXT });
    b.text(d, x, 620, 360, { family: T, size: 28, color: MUTED }, 'start', { lh: 1.45 });
  });
  b.notes('Each quarter has one hero moment. Detailed briefs for Q1 and Q2 are ready; Q3 and Q4 are directional.');

  b.page(6).bg(solid(CORAL));
  b.text('“People don’t buy tea.\nThey buy ten quiet minutes.”', 120, 380, 1680, { family: H, weight: 800, size: 66, color: '#2e1065' }, 'center', { lh: 1.1, role: 'quote' });
  b.text('— Our 2027 brand idea', 120, 600, 1680, { family: T, weight: 700, size: 36, color: '#2e1065' }, 'center');
  b.notes('This line is the creative platform for the year. Everything we make should feel like ten quiet minutes.');

  b.page(7).bg(solid(VIO));
  b.shape('blob', -200, 600, 800, 700, CORAL, { rot: 20 });
  b.text('Questions?', 120, 300, 1680, { family: H, weight: 800, size: 170, color: '#ffffff' }, 'center', { role: 'headline' });
  b.text('brand@bramble.example · next review 15 April 2027', 120, 560, 1680, { family: T, weight: 500, size: 36, color: BUTTER }, 'center', { role: 'cta' });
  b.notes('Ask for sign-off on the budget split today, and agree the date for the Q1 review.');
}, 8);

// ------------------------------------------------------------------ 4. Project status report
tpl('vd-deck-status', 'Project Status Report', PR, 'presentation', ['report', 'project', 'status', 'corporate', 'data'], (b) => {
  const NAVY = '#0f2747', BLUE = '#2563eb', BG = '#f5f7fb', TXT = '#1e293b', MUTED = '#526075', GREEN = '#15803d', AMBER = '#b45309', RED = '#b91c1c';
  const H = 'Inter Tight', T = 'Inter';
  const head = (title: string, n: number) => {
    b.rect(0, 0, 1920, 8, BLUE);
    b.text(title, 120, 100, 1400, { family: H, weight: 700, size: 64, color: NAVY }, 'start', { role: 'headline' });
    b.text('ATLAS MIGRATION · STATUS REPORT · WEEK 22', 120, 1000, 1000, { family: T, weight: 600, size: 18, tracking: 120, color: MUTED });
    b.text(String(n), 1700, 1000, 100, { family: T, weight: 700, size: 18, color: MUTED }, 'end');
  };

  b.page(0).bg(solid(NAVY));
  b.rect(1180, 0, 740, 1080, BLUE);
  for (let i = 0; i < 6; i++) b.rect(1260 + i * 100, 820 - i * 90, 60, 160 + i * 90, '#ffffff', { opacity: 0.12 + i * 0.05, r: 4 });
  b.text('PROJECT STATUS REPORT', 120, 140, 900, { family: T, weight: 700, size: 26, tracking: 200, color: '#93c5fd' }, 'start', { role: 'subhead' });
  b.text('Atlas Platform\nMigration', 120, 360, 1000, { family: H, weight: 800, size: 120, color: '#ffffff' }, 'start', { lh: 1.0, role: 'headline' });
  b.text('Week 22 · 2 June 2027\nPrepared by the PMO for the steering committee', 120, 660, 1000, { family: T, size: 32, color: '#cbd5e1' }, 'start', { lh: 1.5 });
  b.notes('Fortnightly steering committee update. Overall status is amber: on budget, two weeks behind on data migration. Decision needed on page 7.');

  b.page(1).bg(solid(BG));
  head('Agenda', 2);
  ([['Executive summary', '5 min'], ['Milestones', '5 min'], ['Budget', '5 min'], ['Risks & issues', '10 min'], ['Decisions needed', '5 min']] as const).forEach(([t, m], i) => {
    const y = 270 + i * 130;
    b.rect(120, y, 1680, 110, '#ffffff', { r: 12 });
    b.text(String(i + 1), 160, y + 26, 60, { family: H, weight: 800, size: 44, color: BLUE });
    b.text(t, 260, y + 30, 1100, { family: H, weight: 600, size: 40, color: TXT });
    b.text(m, 1560, y + 36, 200, { family: T, weight: 500, size: 30, color: MUTED }, 'end');
  });
  b.notes('Thirty minutes total. Spend most of the time on risks — that is where we need the committee’s help.');

  b.page(2).bg(solid(BG));
  head('Executive summary', 3);
  ([['Overall', 'AMBER', AMBER, '#fef3c7', 'Data migration two weeks late; go-live date still achievable.'], ['Budget', 'GREEN', GREEN, '#dcfce7', '€1.82M spent of €3.4M (54%) at 58% complete.'], ['Scope', 'GREEN', GREEN, '#dcfce7', 'No change requests this period.'], ['Resourcing', 'RED', RED, '#fee2e2', 'Two data engineers short from week 24.']] as const).forEach(([k, s, c, bg, d], i) => {
    const x = 120 + (i % 2) * 850, y = 260 + Math.floor(i / 2) * 340;
    b.rect(x, y, 830, 310, '#ffffff', { r: 16 });
    b.text(k, x + 48, y + 44, 500, { family: H, weight: 700, size: 40, color: TXT });
    b.rect(x + 590, y + 44, 190, 56, bg, { r: 28 });
    b.text(s, x + 590, y + 56, 190, { family: T, weight: 800, size: 26, color: c, tracking: 100 }, 'center');
    b.text(d, x + 48, y + 140, 720, { family: T, size: 30, color: '#475569' }, 'start', { lh: 1.45 });
  });
  b.notes('Headline: amber overall. Budget and scope are green. Resourcing is red — we need two data engineers from week 24.');

  b.page(3).bg(solid('#ffffff'));
  head('Milestones', 4);
  b.rect(160, 520, 1600, 8, '#e2e8f0', { r: 4 });
  b.rect(160, 520, 900, 8, BLUE, { r: 4 });
  ([['Discovery', 'Feb', 'done'], ['Design sign-off', 'Mar', 'done'], ['Infra ready', 'Apr', 'done'], ['Data migration', 'Jun', 'late'], ['UAT', 'Aug', 'next'], ['Go-live', 'Oct', 'next']] as const).forEach(([t, m, st], i) => {
    const cx = 200 + i * 304;
    const c = st === 'done' ? BLUE : st === 'late' ? '#d97706' : '#cbd5e1';
    b.circle(cx, 524, 28, '#ffffff');
    b.circle(cx, 524, 20, c);
    b.text(m.toUpperCase(), cx - 140, 400, 280, { family: T, weight: 700, size: 24, tracking: 150, color: MUTED }, 'center');
    b.text(t, cx - 140, 590, 280, { family: H, weight: 700, size: 32, color: TXT }, 'center', { lh: 1.2 });
    b.text(st === 'done' ? 'Complete' : st === 'late' ? '2 weeks late' : 'On track', cx - 140, 680, 280, { family: T, size: 24, color: st === 'late' ? AMBER : MUTED }, 'center');
  });
  b.rect(120, 790, 1680, 150, BG, { r: 16 });
  b.rect(120, 790, 10, 150, '#d97706', { r: 4 });
  b.text('Critical path', 170, 815, 400, { family: H, weight: 700, size: 30, color: TXT });
  b.text('Data migration now finishes 30 June. UAT can still start 1 August if the CRM cleanse lands by 16 June.', 170, 862, 1580, { family: T, size: 28, color: '#475569' });
  b.notes('Three milestones complete. Data migration is late because the legacy CRM export took longer than planned. UAT and go-live remain on track if we get the extra engineers.');

  b.page(4).bg(solid(BG));
  head('Budget: planned vs actual (€k)', 5);
  b.rect(120, 230, 1120, 720, '#ffffff', { r: 16 });
  b.chart('column', 160, 270, 1040, 640, ['Jan', 'Feb', 'Mar', 'Apr', 'May'], [{ name: 'Planned', values: [320, 360, 380, 400, 420], color: '#cbd5e1' }, { name: 'Actual', values: [300, 345, 395, 380, 400], color: BLUE }]);
  chartStyle(b, { family: T, text: '#334155', grid: '#e2e8f0', size: 20 });
  ([['€1.82M', 'spent to date'], ['€1.58M', 'remaining'], ['−2.1%', 'variance vs plan']] as const).forEach(([n, l], i) => {
    b.text(n, 1320, 260 + i * 230, 480, { family: H, weight: 800, size: 80, color: i === 2 ? GREEN : NAVY });
    b.text(l, 1324, 360 + i * 230, 480, { family: T, size: 28, color: MUTED });
  });
  b.notes('We are slightly under plan, mainly because licence costs landed later than expected. Expect that to reverse in June.');

  b.page(5).bg(solid('#ffffff'));
  head('Risks & issues', 6);
  b.table(120, 240, 1680, 560, [['Risk', 'Impact', 'Likelihood', 'Owner', 'Mitigation'], ['Data engineer shortage', 'High', 'High', 'J. Okafor', 'Contract two engineers by wk 24'], ['Legacy CRM data quality', 'High', 'Medium', 'S. Lindqvist', 'Automated cleansing scripts'], ['UAT tester availability', 'Medium', 'Medium', 'M. Duarte', 'Book business testers now'], ['Vendor API rate limits', 'Low', 'Low', 'A. Chen', 'Batch overnight jobs']]);
  tableStyle(b, { family: T, size: 24, color: TXT, border: '#e2e8f0', head: NAVY, headColor: '#ffffff', stripe: '#f8fafc', pad: 16 });
  const tb = lastEl(b);
  if (tb.type === 'table') tb.cols = [0.26, 0.11, 0.13, 0.15, 0.35];
  b.text('Decision needed: approve €96k for two contract data engineers (weeks 24–36).', 120, 860, 1680, { family: H, weight: 700, size: 34, color: RED }, 'start', { role: 'cta' });
  b.notes('The first risk is the one we need help with today. The other three are under control and shown for visibility.');

  b.page(6).bg(solid(BLUE));
  b.text('“The new platform already saves my team a day a week — we just need the data in it.”', 200, 330, 1520, { family: H, weight: 700, size: 72, color: '#ffffff' }, 'center', { lh: 1.2, role: 'quote' });
  b.text('Rosa Martín · Head of Customer Operations', 200, 690, 1520, { family: T, weight: 600, size: 32, color: '#dbeafe' }, 'center');
  b.notes('Feedback from the pilot team in Madrid. A good reminder of why the data migration matters.');

  b.page(7).bg(solid(NAVY));
  b.text('Next steps', 120, 160, 1200, { family: H, weight: 800, size: 96, color: '#ffffff' }, 'start', { role: 'headline' });
  ([['Approve contractor budget', 'Steering committee · today'], ['Complete CRM data cleanse', 'S. Lindqvist · 16 June'], ['Start UAT preparation', 'M. Duarte · 30 June']] as const).forEach(([t, d], i) => {
    const y = 400 + i * 170;
    b.circle(150, y + 30, 30, BLUE);
    b.text(String(i + 1), 120, y + 8, 60, { family: H, weight: 800, size: 34, color: '#ffffff' }, 'center');
    b.text(t, 220, y, 1400, { family: H, weight: 700, size: 44, color: '#ffffff' });
    b.text(d, 220, y + 64, 1400, { family: T, size: 28, color: '#93c5fd' });
  });
  b.text('Next report: 16 June 2027 · pmo@atlas.example', 120, 960, 1400, { family: T, size: 26, color: '#cbd5e1' });
  b.notes('Confirm owners and dates out loud and record the decision in the minutes.');
}, 8);

// ------------------------------------------------------------------ 5. Lesson (education)
tpl('vd-deck-lesson', 'Photosynthesis Lesson', PR, 'presentation', ['education', 'lesson', 'science', 'school', 'teacher'], (b) => {
  const LEAF = '#2d6a4f', DEEP = '#1b4332', MINT = '#d8f3dc', SUN = '#ffd166', SKY = '#e0f2fe', TXT = '#1b4332';
  const H = 'Fredoka', T = 'Nunito';
  const head = (title: string, color = DEEP) => b.text(title, 120, 100, 1500, { family: H, weight: 700, size: 80, color }, 'start', { role: 'headline' });

  b.page(0).bg(solid(MINT));
  b.circle(1600, 160, 300, SUN);
  b.emoji('☀️', 1450, 30, 280);
  b.shape('half-circle', -100, 820, 2120, 520, '#95d5b2');
  b.emoji('🌱', 1300, 560, 340);
  b.text('SCIENCE · YEAR 7 · LESSON 4', 140, 220, 1200, { family: T, weight: 800, size: 30, tracking: 150, color: LEAF }, 'start', { role: 'subhead' });
  b.text('How Plants\nMake Food', 140, 290, 1200, { family: H, weight: 700, size: 150, color: DEEP }, 'start', { lh: 1.0, role: 'headline' });
  b.text('An introduction to photosynthesis', 140, 620, 1100, { family: T, weight: 600, size: 44, color: LEAF });
  b.notes('Hook: ask the class what a plant “eats”. Collect three or four answers on the board before moving on.');

  b.page(1).bg(solid('#ffffff'));
  head('Today we will…');
  ([['🔍', 'Name the three ingredients plants need'], ['✏️', 'Write the word equation for photosynthesis'], ['🧪', 'Read results from a simple experiment'], ['💬', 'Explain why leaves are green']] as const).forEach(([e, t], i) => {
    const y = 300 + i * 160;
    b.rect(120, y, 1680, 130, i % 2 ? '#ffffff' : MINT, { r: 65 });
    b.emoji(e, 160, y + 20, 90);
    b.text(t, 290, y + 34, 1400, { family: T, weight: 700, size: 46, color: TXT });
  });
  b.notes('Read the objectives aloud. Students copy them into books — we revisit them at the end.');

  b.page(2).bg(solid(LEAF));
  b.text('PART 1', 120, 360, 1680, { family: T, weight: 800, size: 36, tracking: 300, color: SUN }, 'center');
  b.text('The ingredients', 120, 420, 1680, { family: H, weight: 700, size: 150, color: '#ffffff' }, 'center', { role: 'headline' });
  b.emoji('🍃', 870, 660, 180);
  b.notes('Section divider. Quick think-pair-share: what do you think a plant needs to grow?');

  b.page(3).bg(solid(SKY));
  head('The word equation');
  ([['💧', 'water'], ['🌫️', 'carbon\ndioxide'], ['☀️', 'light'], ['🍬', 'glucose'], ['💨', 'oxygen']] as const).forEach(([e, t], i) => {
    const x = 140 + i * 340 + (i >= 3 ? 40 : 0);
    b.rect(x, 340, 280, 360, '#ffffff', { r: 40 });
    b.emoji(e, x + 70, 380, 140);
    b.text(t, x, 550, 280, { family: H, weight: 600, size: 40, color: TXT }, 'center', { lh: 1.05 });
  });
  b.text('+', 420, 470, 60, { family: H, weight: 700, size: 70, color: LEAF }, 'center');
  b.text('+', 760, 470, 60, { family: H, weight: 700, size: 70, color: LEAF }, 'center');
  b.shape('arrow', 1125, 495, 50, 50, LEAF);
  b.text('+', 1480, 470, 60, { family: H, weight: 700, size: 70, color: LEAF }, 'center');
  b.text('Light is the energy source, not an ingredient — that’s why we write it above the arrow.', 140, 790, 1640, { family: T, weight: 600, size: 36, color: TXT }, 'center', { lh: 1.4, role: 'body' });
  b.notes('Build the equation one card at a time. Common misconception: plants get their food from the soil. Address it here.');

  b.page(4).bg(solid('#ffffff'));
  head('Our experiment: height in mm');
  b.chart('line', 120, 260, 1100, 700, ['Day 1', 'Day 3', 'Day 5', 'Day 7', 'Day 9', 'Day 11'], [{ name: 'Sunny windowsill', values: [5, 14, 26, 41, 55, 72], color: LEAF }, { name: 'Dark cupboard', values: [5, 8, 11, 12, 13, 13], color: '#f4a261' }]);
  chartStyle(b, { family: T, text: TXT, grid: '#e5e7eb', size: 22 });
  b.rect(1300, 280, 500, 600, MINT, { r: 32 });
  b.text('What do you notice?', 1340, 320, 420, { family: H, weight: 700, size: 46, color: DEEP }, 'start', { lh: 1.1 });
  b.text('• Which plant grew more?\n• When did the dark plant stop growing?\n• What does this tell us about light?', 1340, 450, 420, { family: T, size: 32, color: TXT }, 'start', { lh: 1.55 });
  b.notes('Data is from last term’s class experiment with cress seedlings. Give pairs three minutes to answer the questions before discussing.');

  b.page(5).bg(solid(SUN));
  b.text('Quick quiz', 120, 100, 1680, { family: H, weight: 700, size: 80, color: DEEP }, 'center', { role: 'headline' });
  b.text('Which gas do plants release during photosynthesis?', 160, 260, 1600, { family: T, weight: 800, size: 54, color: DEEP }, 'center', { lh: 1.25 });
  ([['A', 'Carbon dioxide'], ['B', 'Oxygen'], ['C', 'Nitrogen']] as const).forEach(([l, t], i) => {
    const x = 160 + i * 540;
    b.rect(x, 520, 500, 270, '#ffffff', { r: 40 });
    b.circle(x + 250, 600, 50, i === 1 ? LEAF : DEEP);
    b.text(l, x + 200, 568, 100, { family: H, weight: 700, size: 52, color: '#ffffff' }, 'center');
    b.text(t, x, 690, 500, { family: H, weight: 600, size: 46, color: DEEP }, 'center');
  });
  b.notes('Students hold up A, B or C cards. Answer: B, oxygen. Ask a student who chose A to explain their thinking — it is a useful misconception.');

  b.page(6).bg(solid(DEEP));
  b.emoji('🌳', 160, 300, 380);
  b.text('“The leaves of a single large tree can release enough oxygen in a day for a family of four.”', 640, 330, 1160, { family: H, weight: 500, size: 60, color: '#ffffff' }, 'start', { lh: 1.25, role: 'quote' });
  b.text('DID YOU KNOW?', 640, 260, 800, { family: T, weight: 800, size: 30, tracking: 250, color: SUN });
  b.notes('A fun fact to end the main content. Ask: so why does protecting forests matter?');

  b.page(7).bg(solid(MINT));
  head('Remember…');
  b.text('carbon dioxide + water → glucose + oxygen', 120, 290, 1680, { family: H, weight: 600, size: 60, color: LEAF }, 'start');
  b.text('• Photosynthesis happens in the chloroplasts of leaf cells\n• Chlorophyll absorbs light — and makes leaves green\n• Plants use glucose for energy and growth', 120, 430, 1680, { family: T, weight: 600, size: 42, color: TXT }, 'start', { lh: 1.6, role: 'body' });
  b.rect(120, 800, 1680, 140, '#ffffff', { r: 30 });
  b.text('Homework: label the leaf diagram on page 42 · due Friday', 170, 838, 1600, { family: T, weight: 800, size: 40, color: DEEP });
  b.notes('Revisit the objectives from slide 2 — thumbs up, sideways or down for each. Set homework.');
}, 8);

// ------------------------------------------------------------------ 6. Workshop / training
tpl('vd-deck-workshop', 'Design Sprint Workshop', PR, 'presentation', ['workshop', 'training', 'design', 'team', 'agenda'], (b) => {
  const BLK = '#121212', LIME = '#d4ff3a', PAPER = '#f2f2ec', GRAY = '#5c5c56';
  const H = 'Space Grotesk', T = 'Instrument Sans';
  const head = (n: string, title: string) => {
    b.text(n, 120, 100, 300, { family: H, weight: 700, size: 28, color: GRAY }, 'start', { role: 'subhead' });
    b.text(title, 120, 140, 1600, { family: H, weight: 700, size: 84, color: BLK }, 'start', { lh: 1.05, role: 'headline' });
  };

  b.page(0).bg(solid(BLK));
  b.rect(120, 140, 210, 56, LIME, { r: 28 });
  b.text('WORKSHOP', 120, 152, 210, { family: H, weight: 700, size: 26, color: BLK, tracking: 100 }, 'center', { role: 'subhead' });
  b.text('Design Sprint\nin a Day', 120, 300, 1300, { family: H, weight: 700, size: 170, color: '#ffffff' }, 'start', { lh: 0.98, role: 'headline' });
  b.text('From fuzzy problem to tested prototype — Thursday 18 March 2027, Room 4B', 120, 700, 1100, { family: T, size: 36, color: '#c9c9c0' }, 'start', { lh: 1.4 });
  b.shape('star', 1440, 300, 360, 360, LIME, { params: { points: 8, innerRatio: 0.6 } });
  b.text('Facilitated by Noor Haddad', 120, 930, 1000, { family: T, weight: 600, size: 28, color: LIME });
  b.notes('Welcome everyone, do a 60-second round of names and roles. Phones face down — we will take proper breaks.');

  b.page(1).bg(solid(PAPER));
  head('Today', 'The plan');
  b.table(120, 300, 1680, 620, [['Time', 'Session', 'Output'], ['09:30', 'Understand: map the problem', 'Problem map'], ['10:45', 'Sketch: Crazy 8s', '8 ideas each'], ['12:00', 'Lunch', '—'], ['13:00', 'Decide: dot vote & storyboard', 'One storyboard'], ['14:30', 'Prototype', 'Clickable mock-up'], ['16:00', 'Test with 3 users', 'Findings']]);
  tableStyle(b, { family: T, size: 28, color: BLK, border: '#d6d6cc', head: BLK, headColor: LIME, stripe: '#ffffff', pad: 16 });
  const tb = lastEl(b);
  if (tb.type === 'table') tb.cols = [0.16, 0.52, 0.32];
  b.notes('Walk through the day. The timings are tight — I will keep us honest with a timer on screen.');

  b.page(2).bg(solid(LIME));
  b.text('01', 120, 200, 800, { family: H, weight: 700, size: 300, color: BLK });
  b.text('Understand', 120, 540, 1600, { family: H, weight: 700, size: 160, color: BLK }, 'start', { role: 'headline' });
  b.text('Before we solve anything, we agree on what the problem actually is.', 120, 760, 1300, { family: T, size: 40, color: BLK }, 'start', { lh: 1.35 });
  b.notes('Section divider. Set up the challenge statement on the wall before this slide.');

  b.page(3).bg(solid(PAPER));
  head('Exercise', 'Crazy 8s');
  for (let i = 0; i < 8; i++) {
    const x = 120 + (i % 4) * 230, y = 330 + Math.floor(i / 4) * 300;
    b.rect(x, y, 210, 280, '#ffffff', { r: 8, stroke: ['#d6d6cc', 2] });
    b.text(String(i + 1), x + 16, y + 12, 60, { family: H, weight: 700, size: 28, color: GRAY });
  }
  b.text('Fold an A4 sheet into eight panels. Sketch eight different ideas — one per minute. Quantity over quality; stick figures welcome.', 1140, 340, 660, { family: T, size: 36, color: BLK }, 'start', { lh: 1.45, role: 'body' });
  b.rect(1140, 720, 340, 120, BLK, { r: 60 });
  b.text('8 minutes', 1140, 757, 340, { family: H, weight: 700, size: 38, color: LIME }, 'center');
  b.notes('Start the timer and call out each minute. Nobody shares yet — we do silent review next.');

  b.page(4).bg(solid('#ffffff'));
  head('The framework', 'Five steps, one day');
  ['Understand', 'Sketch', 'Decide', 'Prototype', 'Test'].forEach((t, i) => {
    const x = 120 + i * 340;
    b.shape('chevron', x, 400, 330, 200, i === 2 ? LIME : i < 2 ? BLK : '#e5e5dc', { params: { depth: 0.25 } });
    b.text(t, x + 75, 476, 210, { family: H, weight: 700, size: 30, color: i < 2 ? '#ffffff' : BLK }, 'center');
  });
  ([['AM', 'Diverge — map the problem and generate as many ideas as possible.'], ['PM', 'Converge — pick one direction, build it fast and put it in front of real users.']] as const).forEach(([k, d], i) => {
    b.text(k, 120 + i * 860, 700, 120, { family: H, weight: 700, size: 48, color: BLK });
    b.text(d, 260 + i * 860, 706, 640, { family: T, size: 32, color: GRAY }, 'start', { lh: 1.4 });
  });
  b.notes('We are here: step three, Decide. Remind people the goal is learning, not a perfect design.');

  b.page(5).bg(solid(PAPER));
  head('Why sprint?', 'What past teams told us');
  b.chart('bar', 120, 320, 1100, 600, ['Faster calls', 'Priorities', 'Teamwork', 'Fewer meetings'], [{ name: '% of participants who agree', values: [91, 84, 78, 66], color: BLK }]);
  chartStyle(b, { family: T, text: BLK, grid: '#d6d6cc', legend: false, values: true, size: 22 });
  b.text('91%', 1320, 340, 480, { family: H, weight: 700, size: 160, color: BLK });
  b.text('said decisions were made faster after their sprint', 1324, 540, 460, { family: T, size: 34, color: GRAY }, 'start', { lh: 1.4 });
  b.text('Survey of 142 participants, 2025–2026', 1324, 880, 480, { family: T, size: 22, color: GRAY });
  b.notes('Survey of our internal sprint participants. Use this if anyone asks why we are spending a whole day on this.');

  b.page(6).bg(solid(BLK));
  b.text('“', 120, 120, 300, { family: H, weight: 700, size: 300, color: LIME });
  b.text('Getting started is more important than getting it right.', 120, 400, 1500, { family: H, weight: 700, size: 96, color: '#ffffff' }, 'start', { lh: 1.1, role: 'quote' });
  b.text('— A good rule for the rest of today', 120, 800, 1400, { family: T, size: 34, color: '#c9c9c0' });
  b.notes('A reminder before we move into prototyping. Perfection is the enemy of a testable prototype.');

  b.page(7).bg(solid(LIME));
  b.text('Thanks for\nsprinting.', 120, 200, 1300, { family: H, weight: 700, size: 170, color: BLK }, 'start', { lh: 0.98, role: 'headline' });
  b.text('Slides, photos of the wall and test notes will be shared by Friday.\nQuestions: noor@studio.example', 120, 640, 1400, { family: T, size: 38, color: BLK }, 'start', { lh: 1.5 });
  b.shape('star', 1440, 560, 360, 360, BLK, { params: { points: 8, innerRatio: 0.6 } });
  b.notes('Do a quick plus/delta retro on sticky notes before people leave.');
}, 8);

// ------------------------------------------------------------------ 7. Photography portfolio (photo-led)
tpl('vd-deck-portfolio', 'Photography Portfolio', PR, 'presentation', ['portfolio', 'photography', 'photo', 'creative', 'gallery'], (b) => {
  const PAPER = '#f3f0ea', INK = '#161616', RUST = '#9c3f27', GREY = '#5f5a52';
  const H = 'Instrument Serif', T = 'Inter';
  const foot = (n: number, dark = false) => {
    b.text('ELENA MARSH — PORTFOLIO 2027', 120, 1000, 900, { family: T, weight: 500, size: 18, tracking: 200, color: dark ? '#d6d2ca' : GREY });
    b.text(String(n).padStart(2, '0'), 1700, 1000, 100, { family: T, weight: 500, size: 18, color: dark ? '#d6d2ca' : GREY }, 'end');
  };

  b.page(0).bg(solid(PAPER));
  b.photo('picsum:350', 820, 0, 1100, 1080, { focus: [0.45, 0.5], filter: 'matte', filterAmount: 0.5 });
  b.text('Elena Marsh', 120, 120, 650, { family: T, weight: 600, size: 26, tracking: 150, color: INK }, 'start', { role: 'logo' });
  b.text('Light,\nslowly.', 120, 380, 650, { family: H, size: 160, color: INK }, 'start', { lh: 0.95, role: 'headline' });
  b.text('Selected photographs\n2023 – 2027', 124, 720, 600, { family: H, italic: true, size: 44, color: RUST }, 'start', { lh: 1.2, role: 'subhead' });
  foot(1);
  b.notes('Replace the large photo with your strongest single image — it sets the tone for everything that follows.');

  b.page(1).bg(solid(PAPER));
  b.photo('picsum:628', 120, 120, 560, 740, { focus: [0.5, 0.45] });
  b.text('About', 800, 120, 600, { family: H, size: 96, color: INK }, 'start', { role: 'headline' });
  b.text('I’m a documentary and landscape photographer based in Edinburgh. My work looks for quiet moments — coastlines in winter, kitchens at dawn, people between tasks.', 800, 260, 960, { family: T, size: 30, color: '#3a3631' }, 'start', { lh: 1.6, role: 'body' });
  ['Coastlines', 'Kitchens', 'Portraits', 'Commissions'].forEach((t, i) => {
    const y = 560 + i * 90;
    b.text(`0${i + 1}`, 800, y, 100, { family: T, weight: 600, size: 22, color: RUST });
    b.text(t, 900, y - 12, 700, { family: H, size: 52, color: INK });
    b.line(900, y + 66, 860, '#d8d2c6', 2);
  });
  foot(2);
  b.notes('A short introduction. The list doubles as the agenda for the four series in this deck.');

  b.page(2).bg(solid(INK));
  b.bgPhoto('picsum:483', { filter: 'matte', filterAmount: 0.4 });
  b.rect(0, 0, 1920, 1080, '#0b1215', { opacity: 0.45 });
  b.text('SERIES 01', 120, 640, 800, { family: T, weight: 600, size: 26, tracking: 300, color: '#ffffff' });
  b.text('Coastlines', 120, 690, 1400, { family: H, size: 180, color: '#ffffff' }, 'start', { role: 'headline' });
  foot(3, true);
  b.notes('Section divider. Use a full-bleed image from the series; the dark gradient keeps the title legible on any photo.');

  b.page(3).bg(solid(PAPER));
  b.grid(['picsum:374', 'picsum:16', 'picsum:603'], 120, 120, 1680, 770, 'big-left', { gap: 16 });
  b.text('North Berwick, January 2026', 120, 912, 1200, { family: H, italic: true, size: 34, color: GREY });
  foot(4);
  b.notes('Gallery layout: one hero image and two supporting frames. Keep captions short — place and month is enough.');

  b.page(4).bg(solid('#ffffff'));
  b.photo('picsum:365', 120, 120, 820, 760, { focus: [0.4, 0.5] });
  b.photo('picsum:625', 980, 120, 820, 760, { focus: [0.45, 0.5] });
  b.text('Kitchens at dawn', 120, 900, 820, { family: H, size: 48, color: INK });
  b.text('A diptych from the Kitchens series, shot on 35mm film over a single winter.', 980, 910, 820, { family: T, size: 26, color: GREY }, 'start', { lh: 1.5 });
  b.notes('Diptych layout for paired images. Talk about the process: the light, the camera, the waiting.');

  b.page(5).bg(solid(PAPER));
  b.text('Commissions', 120, 120, 1200, { family: H, size: 96, color: INK }, 'start', { role: 'headline' });
  b.table(120, 300, 1680, 480, [['Service', 'Includes', 'From'], ['Editorial day rate', 'Up to 8 hours, 40 edited images', '£950'], ['Brand story', 'Half day, 25 images, usage licence', '£1,400'], ['Portrait session', '90 minutes, 15 images', '£380'], ['Print sales', 'Archival giclée, signed, A3–A1', '£180']]);
  tableStyle(b, { family: T, size: 26, color: INK, border: '#d8d2c6', head: INK, headColor: PAPER, pad: 18 });
  const tb = lastEl(b);
  if (tb.type === 'table') tb.cols = [0.32, 0.5, 0.18];
  b.text('Prices exclude VAT and travel. Licensing quoted per project.', 120, 830, 1200, { family: T, size: 24, color: GREY });
  foot(6);
  b.notes('Rates for 2027. Adjust per client; the table is easy to edit.');

  b.page(6).bg(solid(PAPER));
  b.photo('picsum:485', 1240, 120, 560, 760, { focus: [0.62, 0.5] });
  b.text('“Elena has a rare patience. She waited three mornings for the right light — and it shows in every frame.”', 120, 220, 1020, { family: H, italic: true, size: 68, color: INK }, 'start', { lh: 1.2, role: 'quote' });
  b.text('Jonah Reid · Creative Director, Northlight Magazine', 120, 700, 1000, { family: T, weight: 500, size: 28, color: RUST });
  foot(7);
  b.notes('A client testimonial paired with an image from that commission.');

  b.page(7).bg(solid(INK));
  b.text('Let’s make\nsomething quiet.', 120, 300, 1200, { family: H, size: 140, color: PAPER }, 'start', { lh: 1.0, role: 'headline' });
  b.text('elena@marsh.example  ·  +44 131 555 0177  ·  @elenamarsh.photo', 124, 700, 1600, { family: T, size: 30, color: '#d6d2ca' }, 'start', { role: 'cta' });
  foot(8, true);
  b.notes('Close with contact details and availability for spring 2027.');
}, 8);

// ------------------------------------------------------------------ 8. Wedding / event slideshow
tpl('vd-deck-wedding', 'Wedding Slideshow', PR, 'presentation', ['wedding', 'slideshow', 'event', 'love', 'elegant'], (b) => {
  const IVORY = '#fbf7f1', BLUSH = '#ecd5cb', SAGE = '#7d8f6c', INK = '#3b302a', GOLD = '#8a6a3b';
  const S = 'Great Vibes', H = 'Cormorant Garamond', T = 'Tenor Sans';
  const leaf = () => { b.emoji('🌿', 1680, 60, 160, { rot: 30, opacity: 0.8 }); b.emoji('🌿', 80, 860, 160, { rot: -150, opacity: 0.8 }); };

  b.page(0).bg(solid(IVORY));
  leaf();
  b.photo('picsum:129', 760, 140, 400, 520, { clip: 'ellipse', focus: [0.68, 0.5], filter: 'blush', filterAmount: 0.5 });
  b.text('Amelia & Theo', 260, 680, 1400, { family: S, size: 130, color: INK }, 'center', { role: 'headline' });
  b.text('12 · 06 · 2027  —  THE OLD MILL, OAKRIDGE', 260, 880, 1400, { family: T, size: 28, tracking: 200, color: GOLD }, 'center', { role: 'subhead' });
  b.notes('Plays on the screen as guests arrive. Replace the oval with your favourite photo of the two of you.');

  b.page(1).bg(solid(IVORY));
  b.text('Order of the Day', 260, 120, 1400, { family: S, size: 110, color: INK }, 'center', { role: 'headline' });
  ([['2:00 pm', 'Ceremony', 'The Garden Lawn'], ['3:00 pm', 'Drinks & photos', 'The Orchard'], ['5:00 pm', 'Wedding breakfast', 'The Barn'], ['8:00 pm', 'First dance & party', 'The Barn']] as const).forEach(([t, e, p], i) => {
    const y = 360 + i * 150;
    b.text(t, 500, y, 400, { family: T, size: 34, tracking: 100, color: GOLD }, 'end');
    b.circle(960, y + 24, 10, SAGE);
    if (i < 3) b.rect(958, y + 40, 4, 126, BLUSH);
    b.text(e, 1020, y - 10, 700, { family: H, weight: 600, size: 50, color: INK });
    b.text(p, 1020, y + 52, 700, { family: H, italic: true, size: 32, color: '#6b5d52' });
  });
  b.notes('The running order for the day. Update times and places to match your venue.');

  b.page(2).bg(solid(BLUSH));
  b.text('our story', 260, 400, 1400, { family: S, size: 200, color: INK }, 'center', { role: 'headline' });
  b.text('HOW IT ALL BEGAN', 260, 680, 1400, { family: T, size: 30, tracking: 300, color: INK }, 'center');
  b.notes('Section divider before the photo story.');

  b.page(3).bg(solid(IVORY));
  b.photo('picsum:65', 140, 140, 560, 720, { clip: 'rounded', focus: [0.55, 0.5] });
  b.photo('picsum:82', 560, 420, 420, 500, { clip: 'rounded', focus: [0.55, 0.5] });
  b.text('How we met', 1100, 220, 700, { family: S, size: 100, color: INK }, 'start', { role: 'headline' });
  b.text('A rainy Tuesday in 2019, one umbrella and a bus that never came. Theo offered to share; Amelia offered coffee. Eight years later, we’re still sharing umbrellas.', 1100, 420, 680, { family: H, size: 40, color: INK }, 'start', { lh: 1.45, role: 'body' });
  b.notes('Tell the story in two or three sentences — guests will read it while the music plays.');

  b.page(4).bg(solid(IVORY));
  b.text('Through the years', 260, 110, 1400, { family: S, size: 100, color: INK }, 'center', { role: 'headline' });
  b.table(360, 330, 1200, 540, [['Year', 'Moment'], ['2019', 'Met at the No. 23 bus stop in the rain'], ['2021', 'Moved into our first flat on Rose Street'], ['2023', 'Adopted Biscuit, the world’s calmest dog'], ['2025', 'Theo proposed on the cliffs at Sennen Cove'], ['2027', 'Today — the best day yet']]);
  tableStyle(b, { family: H, size: 34, color: INK, border: '#e8dcd2', head: SAGE, headColor: '#ffffff', stripe: '#f6eee7', pad: 16, bw: 1 });
  const tb = lastEl(b);
  if (tb.type === 'table') tb.cols = [0.2, 0.8];
  b.notes('A simple timeline of milestones. Add or remove rows as you like.');

  b.page(5).bg(solid(IVORY));
  b.grid(['picsum:399', 'picsum:25', 'picsum:360', 'picsum:106'], 140, 120, 1640, 840, 'mosaic', { gap: 18, radius: 8 });
  b.notes('A collage of favourite photos. Drop your own images onto each cell; they crop automatically.');

  b.page(6).bg(solid(SAGE));
  b.text('“Whatever our souls are made of,\nhis and mine are the same.”', 200, 360, 1520, { family: H, italic: true, size: 80, color: '#ffffff' }, 'center', { lh: 1.25, role: 'quote' });
  b.text('EMILY BRONTË', 200, 640, 1520, { family: T, size: 30, tracking: 300, color: '#f3f1ea' }, 'center');
  b.notes('A favourite line, read at the ceremony.');

  b.page(7).bg(solid(IVORY));
  leaf();
  b.text('Thank you', 260, 330, 1400, { family: S, size: 180, color: INK }, 'center', { role: 'headline' });
  b.text('for celebrating with us', 260, 560, 1400, { family: H, italic: true, size: 56, color: INK }, 'center');
  b.text('SHARE YOUR PHOTOS · #AMELIAANDTHEO', 260, 720, 1400, { family: T, size: 28, tracking: 200, color: GOLD }, 'center', { role: 'cta' });
  b.notes('Loops at the end of the evening. Remind guests of the photo hashtag.');
}, 8);

// ------------------------------------------------------------------ 9. Minimal keynote
tpl('vd-deck-keynote', 'Minimal Keynote', PR, 'presentation', ['keynote', 'minimal', 'talk', 'conference', 'black'], (b) => {
  const BLK = '#0a0a0a', WHITE = '#f5f5f0', ORANGE = '#ff5a1f', GREY = '#9a9a92';
  const H = 'Instrument Serif', T = 'Inter';
  const mark = (n: number) => {
    b.text('THE QUIET INTERFACE', 120, 80, 900, { family: T, weight: 600, size: 18, tracking: 300, color: GREY });
    b.text(String(n).padStart(2, '0'), 1700, 80, 100, { family: T, weight: 600, size: 18, color: GREY }, 'end');
  };

  b.page(0).bg(solid(BLK));
  b.rect(124, 720, 90, 4, ORANGE);
  b.text('The quiet\ninterface.', 120, 330, 1500, { family: H, size: 200, color: WHITE }, 'start', { lh: 0.95, role: 'headline' });
  b.text('Designing technology that knows when to step back', 124, 760, 1400, { family: T, size: 36, color: GREY }, 'start', { role: 'subhead' });
  b.text('Sam Okoye · Form & Function Conference · Lisbon 2027', 124, 960, 1400, { family: T, weight: 500, size: 22, color: GREY });
  b.notes('Walk on, pause, then begin: "How many times has your phone asked for your attention since this morning?"');

  b.page(1).bg(solid(BLK));
  mark(2);
  ['Noise', 'Signal', 'Silence'].forEach((t, i) => {
    b.text(`0${i + 1}`, 120 + i * 580, 380, 400, { family: T, weight: 600, size: 26, color: ORANGE });
    b.text(t, 120 + i * 580, 430, 540, { family: H, size: 130, color: WHITE });
  });
  b.line(120, 700, 1680, '#2a2a2a', 2);
  b.text('Three parts, twenty minutes, no notifications.', 120, 760, 1600, { family: T, size: 32, color: GREY });
  b.notes('The talk has three movements. Keep this slide up for just a moment.');

  b.page(2).bg(solid(ORANGE));
  b.text('01', 120, 200, 600, { family: T, weight: 600, size: 40, color: BLK });
  b.text('Noise', 110, 340, 1700, { family: H, size: 360, color: BLK }, 'start', { role: 'headline' });
  b.notes('Section one: the problem. Let the colour change reset the room.');

  b.page(3).bg(solid(BLK));
  mark(4);
  b.text('We unlock our phones', 120, 300, 1680, { family: H, size: 110, color: WHITE }, 'start', { role: 'headline' });
  b.text('144 times a day.', 120, 440, 1680, { family: H, italic: true, size: 110, color: ORANGE });
  b.text('Most of those moments, we didn’t decide to.', 124, 680, 1400, { family: T, size: 36, color: GREY });
  b.notes('Let the number sit. Then: "Most of those unlocks were the interface’s idea, not ours."');

  b.page(4).bg(solid(BLK));
  mark(5);
  b.text('Notifications per person, per day', 120, 160, 1200, { family: H, size: 72, color: WHITE }, 'start', { role: 'headline' });
  b.chart('area', 120, 300, 1180, 640, ['2016', '2018', '2020', '2022', '2024', '2026'], [{ name: 'Notifications', values: [46, 63, 79, 96, 112, 127], color: ORANGE }]);
  chartStyle(b, { family: T, text: '#bdbdb5', grid: '#262626', legend: false, size: 20 });
  b.text('2.8×', 1400, 330, 400, { family: H, size: 200, color: WHITE });
  b.text('growth in ten years — while our attention didn’t grow at all.', 1404, 580, 400, { family: T, size: 30, color: GREY }, 'start', { lh: 1.45 });
  b.notes('Figures are illustrative averages from industry attention studies; cite your preferred source here.');

  b.page(5).bg(solid(WHITE));
  b.text('THREE PRINCIPLES', 120, 80, 900, { family: T, weight: 600, size: 18, tracking: 300, color: '#5c5c55' });
  ([['Wait', 'Batch what can wait. Most messages aren’t urgent — design for the 5% that are.'], ['Whisper', 'Use the quietest signal that works: a dot before a badge, a badge before a sound.'], ['Leave', 'Make the exit as easy as the entry. A good session has an ending.']] as const).forEach(([t, d], i) => {
    const x = 120 + i * 580;
    b.text(`0${i + 1}`, x, 300, 400, { family: T, weight: 600, size: 26, color: '#c2410c' });
    b.text(t, x, 350, 520, { family: H, size: 120, color: BLK });
    b.text(d, x, 540, 480, { family: T, size: 30, color: '#3f3f3a' }, 'start', { lh: 1.5 });
  });
  b.notes('The constructive part of the talk. One example per principle from the case studies.');

  b.page(6).bg(solid(BLK));
  b.text('“Technology should require the smallest possible amount of our attention.”', 220, 330, 1480, { family: H, italic: true, size: 96, color: WHITE }, 'center', { lh: 1.15, role: 'quote' });
  b.rect(930, 700, 60, 3, ORANGE);
  b.text('Calm technology principle', 220, 740, 1480, { family: T, size: 28, color: GREY }, 'center');
  b.notes('Credit the calm technology movement. This is the thesis of the talk in one line.');

  b.page(7).bg(solid(BLK));
  b.text('Thank you.', 120, 380, 1500, { family: H, size: 200, color: WHITE }, 'start', { role: 'headline' });
  b.rect(124, 640, 90, 4, ORANGE);
  b.text('sam@quiet.example  ·  slides at quiet.example/talk', 124, 680, 1400, { family: T, size: 34, color: GREY }, 'start', { role: 'cta' });
  b.notes('End quietly. Leave this on screen for Q&A.');
}, 8);

// =================================================================== Presentations (4:3, 1024×768)

// ------------------------------------------------------------------ 4:3 · Research findings
tpl('vd-43-research', 'Research Findings 4:3', PR, 'presentation-43', ['research', 'academic', 'study', 'findings', 'report'], (b) => {
  const PAPER = '#fbfaf7', INK = '#1d2433', BLUE = '#2f4b7c', RED = '#a23b2a', MUTED = '#5b6270';
  const H = 'Newsreader', T = 'Inter';
  const head = (k: string, title: string, n: number) => {
    b.text(k.toUpperCase(), 64, 56, 700, { family: T, weight: 600, size: 14, tracking: 200, color: RED }, 'start', { role: 'subhead' });
    b.text(title, 64, 82, 900, { family: H, weight: 500, size: 44, color: INK }, 'start', { lh: 1.1, role: 'headline' });
    b.line(64, 712, 896, '#dedbd2', 1.5);
    b.text('Urban Heat & Green Roofs · City Research Lab', 64, 724, 700, { family: T, size: 12, color: MUTED });
    b.text(String(n), 860, 724, 100, { family: T, weight: 600, size: 12, color: MUTED }, 'end');
  };

  b.page(0).bg(solid(BLUE));
  b.rect(0, 560, 1024, 208, '#24395f');
  b.text('RESEARCH FINDINGS · MAY 2027', 64, 120, 800, { family: T, weight: 600, size: 16, tracking: 200, color: '#c7d4ec' }, 'start', { role: 'subhead' });
  b.text('Do green roofs cool\nthe streets below?', 64, 180, 900, { family: H, weight: 500, size: 64, color: '#ffffff' }, 'start', { lh: 1.08, role: 'headline' });
  b.text('A 14-month study of 36 rooftops in three neighbourhoods', 64, 360, 860, { family: H, italic: true, size: 26, color: '#dbe4f3' });
  b.text('Dr. Amara Osei · Lena Fischer · Tom Achterberg', 64, 610, 900, { family: T, weight: 600, size: 18, color: '#ffffff' });
  b.text('City Research Lab · cityresearch.example', 64, 640, 900, { family: T, size: 16, color: '#c7d4ec' });
  b.notes('Introduce the team and the question. This is a 15-minute findings briefing for the city sustainability board.');

  b.page(1).bg(solid(PAPER));
  head('Overview', 'What we’ll cover', 2);
  ['Research question', 'Method and sites', 'Key findings', 'What residents said', 'Recommendations'].forEach((t, i) => {
    const y = 210 + i * 86;
    b.text(String(i + 1), 64, y, 40, { family: H, weight: 500, size: 36, color: RED });
    b.text(t, 120, y + 4, 760, { family: H, size: 32, color: INK });
    b.line(120, y + 64, 840, '#e6e2d8', 1.5);
  });
  b.notes('Agenda slide. Findings are on slide 4 if the board wants to skip ahead.');

  b.page(2).bg(solid(PAPER));
  head('Method', 'Thirty-six roofs, three neighbourhoods', 3);
  ([['36', 'rooftops monitored'], ['14', 'months of readings'], ['2.1M', 'temperature samples']] as const).forEach(([n, l], i) => {
    const x = 64 + i * 300;
    b.rect(x, 220, 280, 180, '#eef1f6', { r: 6 });
    b.text(n, x + 24, 240, 240, { family: H, weight: 500, size: 64, color: BLUE });
    b.text(l, x + 24, 340, 240, { family: T, size: 16, color: MUTED });
  });
  b.text('Half the roofs were planted with sedum or meadow mixes; half were conventional bitumen. Sensors at roof level and at street level (1.5 m) logged temperature every ten minutes from March 2026 to April 2027.', 64, 440, 880, { family: T, size: 19, color: INK }, 'start', { lh: 1.6, role: 'body' });
  b.notes('Briefly explain the paired design: each green roof was matched to a nearby conventional roof of similar height and orientation.');

  b.page(3).bg(solid(PAPER));
  head('Key finding', 'Green roofs ran up to 4.2 °C cooler', 4);
  b.chart('line', 64, 180, 600, 500, ['Jun', 'Jul', 'Aug', 'Sep'], [{ name: 'Conventional roof (°C)', values: [41.2, 46.8, 45.1, 37.4], color: RED }, { name: 'Green roof (°C)', values: [37.9, 42.6, 41.3, 34.8], color: BLUE }]);
  chartStyle(b, { family: T, text: INK, grid: '#e6e2d8', size: 13 });
  b.text('−4.2 °C', 700, 200, 260, { family: H, weight: 500, size: 52, color: BLUE });
  b.text('peak roof-surface difference in July afternoons', 700, 270, 260, { family: T, size: 16, color: MUTED }, 'start', { lh: 1.5 });
  b.text('−0.8 °C', 700, 400, 260, { family: H, weight: 500, size: 52, color: INK });
  b.text('at street level — small, but measurable on the hottest days', 700, 470, 260, { family: T, size: 16, color: MUTED }, 'start', { lh: 1.5 });
  b.notes('Monthly mean of daily maximum surface temperature. The street-level effect is smaller but statistically significant (p < 0.01).');

  b.page(4).bg(solid(BLUE));
  b.text('“On the hottest days you can feel it walking past — the corner by the library is the one place I don’t cross the road for shade.”', 96, 220, 832, { family: H, italic: true, size: 36, color: '#ffffff' }, 'start', { lh: 1.35, role: 'quote' });
  b.text('Resident interview, Northgate · July 2026', 96, 480, 832, { family: T, weight: 600, size: 16, color: '#c7d4ec' });
  b.notes('One of 48 resident interviews. Perceived cooling matched the sensor data closely.');

  b.page(5).bg(solid(PAPER));
  head('Recommendations', 'Three next steps for the city', 6);
  ([['Prioritise', 'Target roofs above bus stops, schools and care homes first.'], ['Incentivise', 'Extend the green roof grant to buildings over 400 m².'], ['Monitor', 'Keep 12 sensor sites running to track long-term effects.']] as const).forEach(([t, d], i) => {
    const y = 210 + i * 150;
    b.rect(64, y, 6, 110, RED);
    b.text(t, 96, y, 820, { family: H, weight: 500, size: 34, color: INK });
    b.text(d, 96, y + 52, 820, { family: T, size: 19, color: MUTED }, 'start', { lh: 1.5 });
  });
  b.notes('End with the asks. Full report and data are available on request; offer to share the appendix.');
}, 6);

// ------------------------------------------------------------------ 4:3 · Quarterly sales review
tpl('vd-43-sales', 'Quarterly Sales Review 4:3', PR, 'presentation-43', ['sales', 'quarterly', 'business', 'review', 'kpi'], (b) => {
  const TEAL = '#0f766e', DARK = '#0b2b2a', MINT = '#ccfbf1', BG = '#f6faf9', TXT = '#17302f', MUTED = '#4d6664', GOLD = '#f59e0b';
  const H = 'Manrope', T = 'Manrope';
  const head = (title: string, n: number) => {
    b.rect(64, 60, 40, 6, TEAL, { r: 3 });
    b.text(title, 64, 80, 900, { family: H, weight: 800, size: 40, color: TXT }, 'start', { role: 'headline' });
    b.text(`Q2 2027 · Sales Review · ${n}`, 64, 720, 900, { family: T, weight: 600, size: 12, color: MUTED });
  };

  b.page(0).bg(solid(DARK));
  b.circle(880, 160, 260, TEAL, { opacity: 0.5 });
  b.circle(880, 160, 150, GOLD, { opacity: 0.9 });
  b.text('Q2 2027', 64, 300, 800, { family: H, weight: 800, size: 30, color: GOLD }, 'start', { role: 'subhead' });
  b.text('Sales Review', 64, 350, 860, { family: H, weight: 800, size: 88, color: '#ffffff' }, 'start', { role: 'headline' });
  b.text('Harbor Supply Co. · Commercial team · 8 July 2027', 64, 480, 860, { family: T, weight: 500, size: 20, color: MINT });
  b.notes('Welcome the regional leads. We’ll run through highlights, the numbers by region, and priorities for Q3.');

  b.page(1).bg(solid(BG));
  head('Q2 at a glance', 2);
  ([['$4.82M', 'Revenue', '+12% vs Q1'], ['318', 'New accounts', '+41 vs target'], ['27%', 'Win rate', '+3 pts'], ['$15.2k', 'Avg. deal size', '−4% vs Q1']] as const).forEach(([n, l, d], i) => {
    const x = 64 + (i % 2) * 456, y = 170 + Math.floor(i / 2) * 260;
    b.rect(x, y, 440, 240, '#ffffff', { r: 16 });
    b.text(n, x + 32, y + 32, 380, { family: H, weight: 800, size: 60, color: i === 3 ? TXT : TEAL });
    b.text(l, x + 32, y + 120, 380, { family: T, weight: 700, size: 22, color: TXT });
    b.text(d, x + 32, y + 160, 380, { family: T, weight: 600, size: 18, color: i === 3 ? '#b45309' : '#15803d' });
  });
  b.notes('Strong quarter overall. Deal size dipped slightly because of the SMB push — that was intentional.');

  b.page(2).bg(solid('#ffffff'));
  head('Monthly revenue ($k)', 3);
  b.chart('column', 64, 160, 896, 520, ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun'], [{ name: '2026', values: [1180, 1210, 1340, 1300, 1390, 1450], color: '#99d5cc' }, { name: '2027', values: [1350, 1390, 1560, 1520, 1610, 1690], color: TEAL }]);
  chartStyle(b, { family: T, text: TXT, grid: '#e2ece9', size: 14 });
  b.notes('Every month of 2027 beat the same month last year. June was our best month on record.');

  b.page(3).bg(solid(BG));
  head('Performance by region', 4);
  b.table(64, 170, 896, 400, [['Region', 'Revenue', 'vs target', 'Win rate'], ['North', '$1.42M', '108%', '31%'], ['South', '$1.18M', '96%', '24%'], ['East', '$1.31M', '112%', '29%'], ['West', '$0.91M', '88%', '22%']]);
  tableStyle(b, { family: T, size: 18, color: TXT, border: '#d5e4e1', head: TEAL, headColor: '#ffffff', stripe: '#ffffff', pad: 12 });
  b.text('West is behind target after losing two senior reps in April — both roles are now filled.', 64, 600, 896, { family: T, weight: 600, size: 18, color: MUTED }, 'start', { lh: 1.5 });
  b.notes('Give credit to East and North. Be direct about West and the recovery plan.');

  b.page(4).bg(solid(TEAL));
  b.text('“Harbor’s team understood our stock problem in one call and had a fix delivered by Friday.”', 80, 230, 864, { family: H, weight: 700, size: 40, color: '#ffffff' }, 'center', { lh: 1.3, role: 'quote' });
  b.text('Operations Manager, Bayline Marine — new account, May 2027', 80, 460, 864, { family: T, weight: 600, size: 18, color: MINT }, 'center');
  b.notes('Our largest new account this quarter. Customer stories like this are what we want more of in Q3.');

  b.page(5).bg(solid(DARK));
  b.text('Q3 priorities', 64, 100, 900, { family: H, weight: 800, size: 56, color: '#ffffff' }, 'start', { role: 'headline' });
  ['Recover West to 100% of target', 'Launch the marine parts bundle', 'Lift average deal size back above $16k'].forEach((t, i) => {
    const y = 250 + i * 120;
    b.circle(90, y + 24, 26, GOLD);
    b.text(String(i + 1), 64, y + 6, 52, { family: H, weight: 800, size: 26, color: DARK }, 'center');
    b.text(t, 140, y + 4, 820, { family: T, weight: 700, size: 28, color: '#ffffff' });
  });
  b.text('Questions → sales-ops@harbor.example', 64, 660, 900, { family: T, weight: 600, size: 18, color: MINT }, 'start', { role: 'cta' });
  b.notes('Close with the three priorities and owners. Open the floor for questions.');
}, 6);

// ------------------------------------------------------------------ 4:3 · Nonprofit fundraiser
tpl('vd-43-fundraiser', 'Community Fundraiser 4:3', PR, 'presentation-43', ['nonprofit', 'fundraiser', 'community', 'charity', 'garden'], (b) => {
  const CREAM = '#fff8ec', GREEN = '#2f5d3a', TOMATO = '#c2410c', SUN = '#fcd34d', TXT = '#2b2a26', MUTED = '#5e5a50';
  const H = 'Young Serif', T = 'Figtree';

  b.page(0).bg(solid(CREAM));
  b.circle(820, 600, 280, SUN, { opacity: 0.6 });
  b.emoji('🌻', 690, 440, 260, { rot: 8 });
  b.emoji('🥕', 560, 590, 120, { rot: -20 });
  b.text('ROSE STREET COMMUNITY GARDEN', 64, 80, 800, { family: T, weight: 800, size: 16, tracking: 200, color: GREEN }, 'start', { role: 'logo' });
  b.text('Grow a\nGarden for\nEveryone', 64, 150, 640, { family: H, size: 72, color: TXT }, 'start', { lh: 1.05, role: 'headline' });
  b.text('Spring 2027 fundraising appeal', 64, 430, 560, { family: T, weight: 600, size: 24, color: TOMATO }, 'start', { role: 'subhead' });
  b.notes('Thank everyone for coming to the spring supper. This short presentation explains where the money goes.');

  b.page(1).bg(solid(GREEN));
  b.text('Our mission', 64, 120, 900, { family: H, size: 52, color: SUN }, 'start', { role: 'headline' });
  b.text('We turn an empty lot into fresh food, shared skills and a place where neighbours meet — free for everyone, run by volunteers.', 64, 230, 860, { family: H, size: 36, color: '#ffffff' }, 'start', { lh: 1.35, role: 'body' });
  ([['140', 'volunteers'], ['2.1 t', 'food grown in 2026'], ['60', 'families fed weekly']] as const).forEach(([n, l], i) => {
    const x = 64 + i * 300;
    b.line(x, 470, 260, SUN, 3);
    b.text(n, x, 500, 280, { family: H, size: 56, color: '#ffffff' });
    b.text(l, x, 580, 280, { family: T, weight: 600, size: 18, color: '#d9e8dc' });
  });
  b.notes('Our mission in one sentence. Mention we started with eight volunteers in 2022 and now have 140.');

  b.page(2).bg(solid(CREAM));
  b.text('Where your money goes', 64, 70, 900, { family: H, size: 44, color: TXT }, 'start', { role: 'headline' });
  b.chart('bar', 64, 170, 480, 460, ['Raised beds', 'Workshops', 'Tools & water', 'Food boxes'], [{ name: '% of budget', values: [40, 25, 20, 15], color: GREEN }]);
  chartStyle(b, { family: T, text: TXT, grid: '#ebe3d3', legend: false, values: true, size: 15 });
  ([['£12,000', 'goal for spring 2027'], ['£7,450', 'raised so far'], ['62%', 'of the way there']] as const).forEach(([n, l], i) => {
    b.text(n, 580, 190 + i * 150, 380, { family: H, size: 48, color: i === 2 ? TOMATO : GREEN });
    b.text(l, 582, 256 + i * 150, 380, { family: T, size: 18, color: MUTED });
  });
  b.notes('Every pound is spent locally. The food bank boxes go to 60 families each week in the growing season.');

  b.page(3).bg(solid(SUN));
  b.text('“My two boys had never pulled a carrot out of the ground. Now they ask to go to the garden every Saturday.”', 80, 200, 864, { family: H, size: 38, color: TXT }, 'center', { lh: 1.35, role: 'quote' });
  b.text('Fatima, parent and volunteer since 2025', 80, 460, 864, { family: T, weight: 700, size: 20, color: '#5a3a00' }, 'center');
  b.notes('Fatima is here tonight — invite her to say a few words if she’s comfortable.');

  b.page(4).bg(solid(CREAM));
  b.text('Three ways to help', 64, 70, 900, { family: H, size: 44, color: TXT }, 'start', { role: 'headline' });
  ([['💚', 'Give', 'Sponsor a raised bed for £75 or give monthly.'], ['🧤', 'Volunteer', 'Saturday mornings, no experience needed.'], ['📣', 'Share', 'Tell a neighbour and follow @rosestreetgarden.']] as const).forEach(([e, t, d], i) => {
    const x = 64 + i * 304;
    b.rect(x, 180, 288, 330, '#ffffff', { r: 20 });
    b.emoji(e, x + 32, 210, 80);
    b.text(t, x + 32, 320, 230, { family: H, size: 32, color: GREEN });
    b.text(d, x + 32, 380, 230, { family: T, size: 19, color: TXT }, 'start', { lh: 1.5 });
  });
  b.notes('Pass round the sign-up sheet now.');

  b.page(5).bg(solid(GREEN));
  b.text('Thank you!', 64, 200, 896, { family: H, size: 84, color: '#ffffff' }, 'center', { role: 'headline' });
  b.text('Donate at rosestreet.example/give', 64, 340, 896, { family: T, weight: 700, size: 28, color: SUN }, 'center', { role: 'cta' });
  b.qr('https://rosestreet.example/give', 432, 420, 160);
  b.notes('Leave the QR code on screen during dessert so people can donate from their phones.');
}, 6);
