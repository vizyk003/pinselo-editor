/** Templates: social-b (see index.ts for categories). */
import { hex } from '@oc/engine';
import { B, lin, rad, solid, tpl } from './builder';

void [B, lin, rad, solid, tpl];

const FB = 'Facebook Posts', FBC = 'Facebook Covers', X = 'X Posts', LI = 'LinkedIn Posts', BAN = 'Social Banners', PIN = 'Pinterest Pins';
const shadow = (color: string, dy = 6, blur = 18) => ({ k: 'shadow' as const, dx: 0, dy, blur, color: hex(color) });
const chip = (color: string, padding = 14, radius = 8) => ({ k: 'background' as const, color: hex(color), padding, radius });

/** Pinned CC0 photos by theme (Openverse ids). A missing id falls back to an empty photo frame. */
const PH: Record<string, string | undefined> = {
  cafe: '73bf285f-f413-40b0-a865-1930ace6778b', cookies: 'f553f065-19d1-44f9-9526-6dbceb3b07d4', salad: 'cd216b53-b444-4638-a6d7-ab5809ecc6c4',
  baking: 'e7830194-beba-4240-b618-f635df7d6e63', deskTop: '95674b79-66e1-4c1d-a7d5-0b928fd94825', typing: '6adcab40-54a1-4192-ada8-96968da73762',
  lake: '6c2bc223-8f17-471f-8c72-f4b485c71869', fjord: '7324d55f-2fd9-4090-886b-5d6473209dce', rose: '8eb53a8a-2a32-4f2e-b3da-ffa99dab9f53',
  roses: 'b8737817-7e17-4ec6-b185-bb469a680f5a', vases: '832fea9c-9abb-4e20-a17e-0df4b07ef3a3', team: 'be224b0f-9817-49e7-8402-8198f7f4e126',
  living: '35ece517-cd3a-458b-8470-bda517c66b4b', pothos: 'a017e633-3048-4971-92d5-c8215d68c0a3', succulent: 'ea608729-87ec-4e92-82d0-aff691ea4d57',
  makeup: '9e6faf67-289c-463b-bb1a-0954f5f58df4', womanRed: 'd767e7a0-1e33-4a8e-8605-14617c044085', womanWall: 'eeee05d0-0c4a-4d31-a6d9-9016e0fbb943',
  womanSide: '02e2d1b6-e8b3-4a90-a171-92c41fab7715', womanDenim: 'c50a527d-f1f1-4e21-857f-ac286ca121b2',
}
type PicOpts = Parameters<B['photo']>[5] & { fill?: string };
const pic = (b: B, key: string, x: number, y: number, w: number, h: number, o: PicOpts = {}) => {
  const id = PH[key];
  if (id) return b.photo(id, x, y, w, h, o);
  return b.frame(x, y, w, h, o.clip ?? (o.r ? 'rounded' : 'rect'), o.fill ?? '#c9c2b8');
};

// =================================================================== Facebook posts (1200×630)

tpl('sb-fb-webinar-dark', 'Free Webinar Invite', FB, 'fb-post', ['webinar', 'event', 'tech', 'online'], (b) => {
  b.bg(solid('#0b1020'));
  b.shape('ellipse', 760, -260, 720, 720, rad('#3b5bfd', '#0b1020'), { opacity: 0.9 });
  b.shape('ring', 860, 130, 300, 300, '#7c9cff', { opacity: 0.5, params: { thickness: 0.04 } });
  b.frame(895, 165, 230, 230, 'ellipse', '#26304f');
  b.text('Speaker: Dr. Lena Ortiz', 820, 430, 380, { family: 'Space Grotesk', weight: 600, size: 22, color: '#ffffff' }, 'center');
  b.text('Head of Research, Fieldnote Labs', 820, 462, 380, { family: 'Space Grotesk', size: 18, color: '#a5b4fc' }, 'center');
  b.text('FREE LIVE WEBINAR', 80, 90, 600, { family: 'Space Grotesk', weight: 700, size: 22, tracking: 220, color: '#7c9cff' }, 'start', { role: 'subhead' });
  b.text('Designing products people trust with AI', 80, 140, 660, { family: 'Space Grotesk', weight: 700, size: 58, color: '#ffffff' }, 'start', { lh: 1.08, role: 'headline' });
  b.text('A 45-minute session on clear UX patterns, honest defaults and real user research.', 80, 310, 600, { family: 'Inter', size: 22, color: '#c7d2fe' }, 'start', { lh: 1.45, role: 'body' });
  b.rect(80, 470, 250, 64, '#ffffff', { r: 32 });
  b.text('Save my seat', 80, 487, 250, { family: 'Space Grotesk', weight: 700, size: 24, color: '#0b1020' }, 'center', { role: 'cta' });
  b.text('Thu 12 Nov 2026 · 6 PM CET', 360, 487, 380, { family: 'Space Grotesk', weight: 500, size: 22, color: '#ffffff' });
});

tpl('sb-fb-food-drive', 'Community Food Drive', FB, 'fb-post', ['nonprofit', 'charity', 'donate', 'community'], (b) => {
  b.bg(solid('#ffe8d1'));
  b.rect(0, 0, 1200, 630, lin(0, '#ffe8d1', '#ffd3b0'));
  b.shape('blob', 720, 40, 520, 520, '#ff6b35', { opacity: 0.95 });
  b.emoji('🥫', 820, 110, 170, { rot: -8 });
  b.emoji('🍎', 960, 250, 140, { rot: 10 });
  b.emoji('🥖', 790, 300, 160, { rot: -20 });
  b.text('FILL THE', 70, 70, 640, { family: 'Anton', size: 110, color: '#1f1300' }, 'start', { lh: 1.0, role: 'headline' });
  b.text('PANTRY', 70, 180, 640, { family: 'Anton', size: 150, color: '#e8481a' }, 'start', { lh: 1.0 });
  b.text('Drop off canned goods, pasta and baby supplies at any Riverbend library branch, 1–30 November.', 70, 365, 560, { family: 'DM Sans', weight: 500, size: 24, color: '#3d2a14' }, 'start', { lh: 1.4, role: 'body' });
  b.rect(70, 490, 560, 22, '#ffffff', { r: 11 });
  b.rect(70, 490, 380, 22, '#e8481a', { r: 11 });
  b.text('3,400 of 5,000 meals collected', 70, 525, 560, { family: 'DM Sans', weight: 700, size: 20, color: '#1f1300' });
  b.text('riverbendfoodbank.example.org', 70, 560, 560, { family: 'DM Sans', size: 18, color: '#7a4a1e' }, 'start', { role: 'cta' });
});

tpl('sb-fb-holiday-hours', 'Holiday Opening Hours', FB, 'fb-post', ['holiday', 'christmas', 'hours', 'store', 'announcement'], (b) => {
  b.bg(solid('#0f3d2e'));
  b.box(30, 30, 1140, 570, '#e9c46a', 2, 18);
  b.emoji('🎄', 70, 380, 190, { rot: -6 });
  b.emoji('✨', 1030, 60, 100);
  b.emoji('🎁', 260, 470, 110, { rot: 8 });
  b.text('Holiday\nHours', 80, 70, 520, { family: 'Shrikhand', size: 92, color: '#fbe9c6' }, 'start', { lh: 1.0, role: 'headline' });
  b.text('Hollow Oak Books & Café', 84, 270, 520, { family: 'Fraunces', italic: true, size: 30, color: '#e9c46a' }, 'start', { role: 'subhead' });
  b.rect(640, 90, 470, 440, '#fbe9c6', { r: 16 });
  b.text('24 Dec\n25–26 Dec\n31 Dec\n1 Jan\n2 Jan onward', 680, 135, 230, { family: 'Fraunces', weight: 600, size: 26, color: '#0f3d2e' }, 'start', { lh: 1.9, role: 'body' });
  b.text('9 – 3\nClosed\n9 – 4\nClosed\nAs usual', 900, 135, 170, { family: 'Fraunces', size: 26, color: '#7a2e1c' }, 'end', { lh: 1.9 });
  b.text('Warmest wishes from our shelves to your home.', 680, 440, 400, { family: 'Fraunces', italic: true, size: 20, color: '#335c4b' }, 'start', { lh: 1.4 });
});

// =================================================================== X posts (1600×900)

tpl('sb-x-stat-insight', 'Big Stat Insight', X, 'x-post', ['stats', 'data', 'insight', 'business'], (b) => {
  b.bg(solid('#f4f1ea'));
  b.text('STATE OF REMOTE WORK · 2026', 110, 100, 900, { family: 'Inter Tight', weight: 700, size: 26, tracking: 200, color: '#5b5a55' }, 'start', { role: 'subhead' });
  b.text('73%', 100, 150, 800, { family: 'Instrument Serif', size: 340, color: '#1d1c1a' }, 'start', { lh: 1.0, role: 'headline' });
  b.text('of hybrid teams say async updates saved them at least one meeting a day.', 110, 530, 780, { family: 'Inter Tight', weight: 500, size: 42, color: '#1d1c1a' }, 'start', { lh: 1.25, role: 'body' });
  b.text('Survey of 2,140 managers, Sept 2026 · tidewater.example.com/report', 110, 770, 900, { family: 'Inter Tight', size: 22, color: '#6b6a64' });
  b.rect(960, 140, 540, 620, '#ffffff', { r: 28 });
  b.chart('column', 1000, 200, 460, 420, ['2022', '2023', '2024', '2025', '2026'], [{ name: 'Teams', values: [41, 52, 58, 66, 73], color: '#ff5a36' }]);
  b.text('Share of teams, by year', 1000, 660, 460, { family: 'Inter Tight', weight: 600, size: 24, color: '#5b5a55' }, 'center');
});

tpl('sb-x-thread-tips', 'Numbered Tips Thread', X, 'x-post', ['tips', 'thread', 'list', 'productivity'], (b) => {
  b.bg(solid('#fff6c8'));
  b.shape('ellipse', 1180, -200, 620, 620, '#ffd43b');
  b.text('A THREAD  1/6', 100, 90, 600, { family: 'Gabarito', weight: 800, size: 28, tracking: 120, color: '#7a4b00' }, 'start', { effect: chip('#ffd43b', 12, 20) });
  b.text('5 tiny habits that\nmade me a calmer\nfreelancer', 100, 170, 900, { family: 'Gabarito', weight: 900, size: 92, color: '#1b1a17' }, 'start', { lh: 1.0, role: 'headline' });
  const tips = ['Block “admin hour” every Friday', 'Invoice the day work ends', 'Say yes in writing, not on calls'];
  tips.forEach((t, i) => {
    const y = 520 + i * 90;
    b.circle(132, y + 30, 32, ['#e03131', '#1c7ed6', '#2b8a3e'][i]!);
    b.text(String(i + 1), 100, y + 8, 64, { family: 'Gabarito', weight: 800, size: 34, color: '#ffffff' }, 'center');
    b.text(t, 190, y + 8, 800, { family: 'Gabarito', weight: 600, size: 38, color: '#1b1a17' }, 'start', { role: 'body' });
  });
  b.text('+2 more below ↓', 1150, 780, 350, { family: 'Gabarito', weight: 700, size: 32, color: '#7a4b00' }, 'end', { role: 'cta' });
  b.emoji('🌱', 1220, 270, 230, { rot: 6 });
});

tpl('sb-x-editorial-quote', 'Editorial Quote', X, 'x-post', ['quote', 'editorial', 'minimal', 'thought'], (b) => {
  b.bg(solid('#14110f'));
  b.rect(100, 100, 6, 700, '#d9a15b');
  b.text('“', 150, 40, 300, { family: 'Gloock', size: 260, color: '#d9a15b' });
  b.text('Good design is mostly a series of small, kind decisions that nobody notices until they’re gone.', 160, 260, 1250, { family: 'Gloock', size: 72, color: '#f6efe6' }, 'start', { lh: 1.18, role: 'headline' });
  b.text('MARA ELLINGTON', 160, 680, 800, { family: 'Inter Tight', weight: 700, size: 26, tracking: 240, color: '#d9a15b' }, 'start', { role: 'subhead' });
  b.text('Design lead, Quiet Tools Studio', 160, 722, 800, { family: 'Inter Tight', size: 26, color: '#a8a095' });
});

// =================================================================== LinkedIn posts (1200×1200)

tpl('sb-li-hiring-grid', 'We’re Hiring Roles', LI, 'linkedin-post', ['hiring', 'jobs', 'career', 'recruiting'], (b) => {
  b.bg(solid('#f2f0ff'));
  b.shape('ellipse', 760, -160, 600, 600, '#c9bfff');
  b.emoji('👋', 900, 90, 200, { rot: 12 });
  b.text('Northwind Labs', 100, 100, 600, { family: 'Bricolage Grotesque', weight: 700, size: 34, color: '#4b2bd6' }, 'start', { role: 'logo' });
  b.text('We’re\nhiring.', 100, 180, 800, { family: 'Bricolage Grotesque', weight: 800, size: 190, color: '#170f3a' }, 'start', { lh: 0.92, role: 'headline' });
  const roles = [['Senior Frontend Engineer', 'Remote (EU)'], ['Product Designer', 'Lisbon · Hybrid'], ['Customer Success Lead', 'Remote (US)']];
  roles.forEach(([r, l], i) => {
    const y = 600 + i * 120;
    b.rect(100, y, 1000, 100, '#ffffff', { r: 20 });
    b.text(r!, 140, y + 28, 600, { family: 'Bricolage Grotesque', weight: 700, size: 36, color: '#170f3a' }, 'start', { role: 'body' });
    b.text(l!, 700, y + 32, 360, { family: 'Bricolage Grotesque', weight: 500, size: 28, color: '#5b4bb3' }, 'end');
  });
  b.text('Apply at careers.northwind.example →', 100, 990, 1000, { family: 'Bricolage Grotesque', weight: 700, size: 40, color: '#4b2bd6' }, 'start', { role: 'cta' });
  b.text('Fully paid healthcare · 4-day summer weeks · €2k learning budget', 100, 1060, 1000, { family: 'Bricolage Grotesque', size: 26, color: '#4a4466' });
});

tpl('sb-li-stats-report', 'Report Key Findings', LI, 'linkedin-post', ['stats', 'report', 'insight', 'data'], (b) => {
  b.bg(solid('#0e1a17'));
  b.text('2026 SUSTAINABILITY PULSE', 100, 100, 1000, { family: 'Manrope', weight: 800, size: 26, tracking: 220, color: '#7ee2b8' }, 'start', { role: 'subhead' });
  b.text('Small businesses are going green faster than expected', 100, 150, 1000, { family: 'Fraunces', weight: 600, size: 66, color: '#f1f7f4' }, 'start', { lh: 1.1, role: 'headline' });
  const stats = [['64%', 'switched to a renewable energy tariff'], ['2.3×', 'more likely to win public tenders'], ['41%', 'cut packaging waste year over year']];
  stats.forEach(([n, l], i) => {
    const y = 420 + i * 190;
    b.line(100, y, 1000, '#2f4d44', 2);
    b.text(n!, 100, y + 30, 360, { family: 'Fraunces', weight: 700, size: 104, color: '#7ee2b8' }, 'start', { lh: 1.0 });
    b.text(l!, 480, y + 52, 620, { family: 'Manrope', weight: 500, size: 34, color: '#d7e6e0' }, 'start', { lh: 1.3, role: 'body' });
  });
  b.line(100, 990, 1000, '#2f4d44', 2);
  b.text('Full report: greenledger.example.com/pulse', 100, 1050, 1000, { family: 'Manrope', weight: 700, size: 30, color: '#f1f7f4' }, 'start', { role: 'cta' });
});

tpl('sb-li-carousel-remote', 'Remote Team Habits Carousel', LI, 'linkedin-post', ['carousel', 'tips', 'remote', 'leadership', 'teams'], (b) => {
  const ink = '#1a1a2e', acc = '#e5482d', paper = '#fbf7f0';
  const footer = (n: number) => {
    b.text('@priya.raman · People & Culture', 90, 1090, 700, { family: 'Onest', weight: 500, size: 24, color: '#6b6878' });
    b.text(`${n} / 5`, 900, 1090, 210, { family: 'Onest', weight: 700, size: 24, color: ink }, 'end');
  };
  b.page(0).bg(solid(acc));
  b.shape('ellipse', 640, 620, 760, 760, '#ef6a4f');
  b.text('THE REMOTE PLAYBOOK', 90, 110, 900, { family: 'Onest', weight: 800, size: 28, tracking: 220, color: '#ffffff' }, 'start', { role: 'subhead' });
  b.text('3 habits of remote teams that actually ship', 90, 180, 980, { family: 'Unbounded', weight: 700, size: 92, color: '#ffffff' }, 'start', { lh: 1.05, role: 'headline' });
  b.emoji('🚀', 760, 680, 300, { rot: -10 });
  b.text('Swipe →', 90, 1060, 400, { family: 'Onest', weight: 800, size: 40, color: '#ffffff' }, 'start', { role: 'cta' });
  const pages = [
    ['01', 'Write it down first', 'Decisions live in docs, not DMs. A one-page brief beats a one-hour call — and new hires can read it next year.', '📝', 'Try: name a recap owner in every meeting.'],
    ['02', 'Default to async', 'Keep meetings for conflict and celebration. Everything else is a recorded update people can watch at 1.5×.', '⏱️', 'Try: swap one status call for a 3-min video.'],
    ['03', 'Overlap on purpose', 'Agree two shared “focus hours” a day across time zones. Protect them like a deadline.', '🌍', 'Try: add shared focus hours to the calendar.'],
  ] as const;
  pages.forEach(([n, h, body, e, tip], i) => {
    b.page(i + 1).bg(solid(paper));
    b.rect(0, 0, 1200, 16, acc);
    b.text(n, 90, 110, 400, { family: 'Unbounded', weight: 800, size: 160, color: acc }, 'start', { lh: 1.0 });
    b.emoji(e, 850, 120, 220);
    b.text(h, 90, 390, 1000, { family: 'Unbounded', weight: 700, size: 76, color: ink }, 'start', { lh: 1.1, role: 'headline' });
    b.rect(90, 860, 1020, 150, '#ffe6df', { r: 24 });
    b.text(tip, 130, 910, 940, { family: 'Onest', weight: 700, size: 36, color: '#9c2a14' }, 'start', { lh: 1.3 });
    b.text(body, 90, 560, 940, { family: 'Onest', size: 42, color: '#3d3b4a' }, 'start', { lh: 1.45, role: 'body' });
    footer(i + 2);
  });
  b.page(4).bg(solid(ink));
  b.text('Found this useful?', 90, 300, 1000, { family: 'Unbounded', weight: 700, size: 84, color: '#ffffff' }, 'start', { lh: 1.1, role: 'headline' });
  b.text('Repost to help another team work better — and follow for a new playbook every Tuesday.', 90, 520, 960, { family: 'Onest', size: 42, color: '#c9c7d6' }, 'start', { lh: 1.4, role: 'body' });
  b.rect(90, 800, 420, 100, acc, { r: 50 });
  b.text('Repost this', 90, 825, 420, { family: 'Onest', weight: 800, size: 40, color: '#ffffff' }, 'center', { role: 'cta' });
  b.text('5 / 5', 900, 1090, 210, { family: 'Onest', weight: 700, size: 24, color: '#ffffff' }, 'end');
}, 5);

// =================================================================== LinkedIn banners (1584×396) — keep x<400 of the lower half clear

tpl('sb-lib-consultant-gradient', 'Consultant Gradient Banner', BAN, 'linkedin-banner', ['linkedin', 'banner', 'consultant', 'gradient'], (b) => {
  b.bg(lin(100, '#0b2a4a', '#145c7a', '#2bb3a3'));
  b.shape('ellipse', 1240, -180, 520, 520, '#ffffff', { opacity: 0.07 });
  b.shape('ring', 1340, 190, 300, 300, '#ffffff', { opacity: 0.18, params: { thickness: 0.05 } });
  b.text('Helping founders turn messy finances into clear decisions.', 520, 110, 820, { family: 'Plus Jakarta Sans', weight: 700, size: 44, color: '#ffffff' }, 'start', { lh: 1.15, role: 'headline' });
  b.text('Fractional CFO  ·  SaaS & Marketplaces  ·  Book a call: clearbooks.example.com', 520, 250, 980, { family: 'Plus Jakarta Sans', weight: 500, size: 24, color: '#c9f2ea' }, 'start', { role: 'subhead' });
});

tpl('sb-lib-designer-playful', 'Playful Designer Banner', BAN, 'linkedin-banner', ['linkedin', 'banner', 'designer', 'creative', 'colorful'], (b) => {
  b.bg(solid('#fff4e6'));
  b.shape('half-circle', 1290, 196, 220, 200, '#ff8a5b');
  b.shape('ellipse', 1400, 40, 140, 140, '#4361ee');
  b.shape('star', 1260, 50, 96, 96, '#ffd23f', { rot: 12 });
  b.rect(1490, 220, 94, 176, '#2ec4b6');
  b.text('Brand & product designer', 440, 80, 760, { family: 'Syne', weight: 700, size: 26, tracking: 120, color: '#4361ee' }, 'start', { role: 'subhead' });
  b.text('Making serious tools\nfeel delightful.', 440, 130, 800, { family: 'Syne', weight: 800, size: 44, color: '#1b1b1f' }, 'start', { lh: 1.05, role: 'headline' });
  b.text('Open for Q1 2027 projects · hello@junoparks.example', 440, 290, 760, { family: 'DM Sans', weight: 500, size: 22, color: '#5a5048' });
});

tpl('sb-lib-speaker-dark', 'Speaker & Author Banner', BAN, 'linkedin-banner', ['linkedin', 'banner', 'speaker', 'author', 'elegant'], (b) => {
  b.bg(solid('#121212'));
  b.rect(0, 0, 1584, 396, lin(90, '#121212', '#121212', '#2a1f14'));
  b.line(480, 198, 60, '#c9a46c', 2);
  b.text('Keynote speaker · Author of “The Patient Leader”', 560, 120, 960, { family: 'Inter Tight', weight: 600, size: 24, tracking: 120, color: '#c9a46c' }, 'start', { role: 'subhead' });
  b.text('Leadership for the long game.', 560, 165, 960, { family: 'Cormorant Garamond', weight: 600, size: 74, color: '#f5efe6' }, 'start', { role: 'headline' });
  b.text('Speaking enquiries: talks@danielokafor.example', 560, 280, 960, { family: 'Inter Tight', size: 22, color: '#9b9488' });
});

// =================================================================== Pinterest pins (1000×1500)

tpl('sb-pin-morning-tips', 'Morning Routine Tips', PIN, 'pinterest-pin', ['tips', 'infographic', 'wellness', 'routine', 'self care'], (b) => {
  b.bg(solid('#fdf1e7'));
  b.rect(0, 0, 1000, 470, '#f7c8a8');
  b.shape('ellipse', 640, 40, 300, 300, '#ffe3b3');
  b.emoji('☀️', 690, 90, 200);
  b.text('THE 10-MINUTE', 80, 110, 600, { family: 'Fredoka', weight: 600, size: 40, tracking: 120, color: '#8a3b12' }, 'start', { role: 'subhead' });
  b.text('Morning\nReset', 80, 165, 600, { family: 'Fraunces', weight: 800, size: 120, color: '#3b1d0c' }, 'start', { lh: 0.95, role: 'headline' });
  const steps = [['💧', 'Drink a glass of water', 'before your phone, before coffee.'], ['🧘', 'Two minutes of stretching', 'neck, shoulders, hamstrings.'], ['📓', 'Write three lines', 'one worry, one win, one plan.'], ['🌿', 'Open a window', 'fresh air and daylight wake you up.'], ['✅', 'Pick one priority', 'the task that makes today a good day.']];
  steps.forEach(([e, h, s], i) => {
    const y = 530 + i * 170;
    b.rect(80, y, 840, 145, '#ffffff', { r: 28 });
    b.circle(165, y + 72, 52, '#fde5d3');
    b.emoji(e!, 127, y + 34, 76);
    b.text(h!, 250, y + 28, 640, { family: 'Fredoka', weight: 600, size: 38, color: '#3b1d0c' }, 'start', { role: 'body' });
    b.text(s!, 250, y + 80, 640, { family: 'Fredoka', size: 28, color: '#8a5a3c' });
  });
  b.text('Save this for tomorrow · slowmornings.example.com', 80, 1400, 840, { family: 'Fredoka', weight: 500, size: 26, color: '#8a3b12' }, 'center', { role: 'cta' });
});

// =================================================================== Facebook covers (1640×624) — key content centred

tpl('sb-fbc-gym-bold', 'Bold Gym Cover', FBC, 'fb-cover', ['fitness', 'gym', 'sport', 'bold', 'cover'], (b) => {
  b.bg(solid('#0d0d0d'));
  b.shape('parallelogram', 1260, 0, 500, 624, '#c6f432', { params: { skew: 0.35 } });
  b.shape('parallelogram', 1180, 0, 110, 624, '#c6f432', { opacity: 0.4, params: { skew: 0.6 } });
  b.shape('parallelogram', -180, 0, 420, 624, '#2a2a2a', { params: { skew: 0.35 } });
  b.text('IRONBARK STRENGTH CLUB', 370, 80, 900, { family: 'Archivo', weight: 800, size: 26, tracking: 300, color: '#c6f432' }, 'center', { role: 'logo' });
  b.text('STRONGER\nEVERY DAY', 370, 125, 900, { family: 'Anton', size: 128, color: '#ffffff' }, 'center', { lh: 0.98, role: 'headline' });
  b.rect(560, 420, 520, 70, '#c6f432', { r: 35 });
  b.text('First week free · Open 5am–11pm', 560, 438, 520, { family: 'Archivo', weight: 700, size: 26, color: '#0d0d0d' }, 'center', { role: 'cta' });
});

tpl('sb-fbc-community-garden', 'Community Garden Cover', FBC, 'fb-cover', ['community', 'garden', 'nonprofit', 'nature', 'cover'], (b) => {
  b.bg(lin(180, '#e9f5db', '#cfe8b8'));
  b.shape('ellipse', -200, 420, 2040, 500, '#97c26b');
  b.shape('ellipse', 600, 470, 1400, 400, '#6fa34a');
  b.emoji('🌻', 1300, 230, 210, { rot: 4 });
  b.emoji('🌱', 1180, 380, 110);
  b.emoji('🥕', 1450, 400, 120, { rot: -14 });
  b.emoji('🐝', 1220, 120, 80, { rot: 12 });
  b.emoji('🌿', 160, 60, 110, { rot: -12 });
  b.text('MAPLE STREET', 320, 110, 1000, { family: 'Fredoka', weight: 600, size: 32, tracking: 300, color: '#3d6b22' }, 'center', { role: 'subhead' });
  b.text('Community Garden', 270, 155, 1100, { family: 'Fraunces', weight: 800, size: 96, color: '#1f3d10' }, 'center', { lh: 1.0, role: 'headline' });
  b.text('Grow together · Volunteer Saturdays from 9 am', 320, 290, 1000, { family: 'Fredoka', weight: 500, size: 32, color: '#2f5419' }, 'center', { role: 'body' });
});

// =================================================================== X: illustrated product launch

tpl('sb-x-app-launch', 'App Launch Announcement', X, 'x-post', ['launch', 'app', 'product', 'tech', 'startup'], (b) => {
  b.bg(lin(135, '#2b1a6b', '#5b2bd6', '#e0569b'));
  b.shape('ellipse', 1050, -260, 760, 760, '#ffffff', { opacity: 0.08 });
  b.shape('ellipse', 900, 520, 600, 600, '#ffb3d1', { opacity: 0.18 });
  // Phone mock-up
  b.rect(1040, 110, 380, 760, '#120b2e', { r: 56, shadow: [0, 30, 60, '#00000066'] });
  b.rect(1060, 130, 340, 720, '#fbf8ff', { r: 42 });
  b.rect(1180, 148, 100, 22, '#120b2e', { r: 11 });
  b.text('This month', 1090, 200, 280, { family: 'Sora', weight: 600, size: 22, color: '#6b5ca5' });
  b.text('$2,418', 1090, 232, 280, { family: 'Sora', weight: 800, size: 56, color: '#1d1440' });
  b.chart('column', 1080, 330, 300, 200, ['M', 'T', 'W', 'T', 'F'], [{ name: 'Spend', values: [32, 54, 41, 68, 47], color: '#7c4dff' }]);
  ['Groceries', 'Rent', 'Transit'].forEach((c, i) => {
    const y = 570 + i * 80;
    b.rect(1085, y, 290, 64, '#efe8ff', { r: 16 });
    b.circle(1117, y + 32, 16, ['#7c4dff', '#e0569b', '#20c997'][i]!);
    b.text(c, 1145, y + 18, 200, { family: 'Sora', weight: 600, size: 22, color: '#1d1440' });
  });
  b.text('NEW · VERSION 2.0', 110, 170, 700, { family: 'Sora', weight: 700, size: 26, tracking: 200, color: '#ffffff' }, 'start', { effect: chip('#ffffff33', 14, 24) });
  b.text('Meet Tally 2.0', 110, 250, 860, { family: 'Sora', weight: 800, size: 116, color: '#ffffff' }, 'start', { lh: 1.0, role: 'headline' });
  b.text('Budgets that update themselves. Shared wallets for roommates. Zero spreadsheets.', 110, 410, 820, { family: 'Sora', size: 36, color: '#f1e9ff' }, 'start', { lh: 1.4, role: 'body' });
  b.rect(110, 640, 380, 90, '#ffffff', { r: 45 });
  b.text('Download free', 110, 663, 380, { family: 'Sora', weight: 700, size: 34, color: '#3a1fa8' }, 'center', { role: 'cta' });
  b.text('iOS & Android', 520, 666, 400, { family: 'Sora', weight: 500, size: 30, color: '#ffffff' });
});

tpl('sb-fbc-cafe-photo', 'Cosy Café Cover', FBC, 'fb-cover', ['cafe', 'coffee', 'restaurant', 'photo', 'cover'], (b) => {
  b.bgPhoto('73bf285f-f413-40b0-a865-1930ace6778b', { focus: [0.5, 0.78], filter: 'warm', filterAmount: 0.6 });
  b.rect(0, 0, 1640, 624, lin(90, '#1a0f08cc', '#1a0f0866', '#1a0f08cc'));
  b.rect(0, 0, 1640, 624, '#1a0f08', { opacity: 0.35 });
  b.box(470, 110, 700, 404, '#f3d9b1', 2, 4);
  b.text('EST. 2019 · OLD TOWN', 470, 160, 700, { family: 'Marcellus', size: 24, tracking: 300, color: '#f3d9b1' }, 'center', { role: 'subhead' });
  b.text('Larkspur', 470, 200, 700, { family: 'Instrument Serif', size: 140, color: '#fff8ef' }, 'center', { lh: 1.0, role: 'logo' });
  b.text('Coffee & Kitchen', 470, 345, 700, { family: 'Instrument Serif', italic: true, size: 48, color: '#f3d9b1' }, 'center', { role: 'headline' });
  b.text('Open daily 7 – 18  ·  12 Wren Lane', 470, 430, 700, { family: 'Marcellus', size: 24, tracking: 80, color: '#fff8ef' }, 'center', { role: 'body' });
});

// =================================================================== Photo-led Facebook posts

tpl('sb-fb-bakery-weekend', 'Weekend Bakery Special', FB, 'fb-post', ['bakery', 'food', 'cafe', 'special', 'weekend'], (b) => {
  b.bg(solid('#f6eee3'));
  pic(b, 'cookies', 600, 0, 600, 630, { filter: 'warm', filterAmount: 0.5 });
  b.circle(600, 470, 95, '#c2410c');
  b.text('ONLY\n$4', 505, 418, 190, { family: 'Fraunces', weight: 800, size: 40, color: '#ffffff' }, 'center', { lh: 1.0 });
  b.text('THIS WEEKEND ONLY', 70, 80, 480, { family: 'DM Sans', weight: 700, size: 20, tracking: 260, color: '#c2410c' }, 'start', { role: 'subhead' });
  b.text('Brown butter cookies, warm from the oven', 70, 125, 470, { family: 'Fraunces', weight: 700, size: 62, color: '#2b1a0e' }, 'start', { lh: 1.05, role: 'headline' });
  b.text('Baked every morning, Saturday & Sunday, until they’re gone. Add a flat white for $7.', 70, 365, 420, { family: 'DM Sans', size: 22, color: '#5a4636' }, 'start', { lh: 1.45, role: 'body' });
  b.text('Crumb & Co. Bakery · 48 Mill St', 70, 540, 480, { family: 'DM Sans', weight: 700, size: 20, color: '#2b1a0e' }, 'start', { role: 'logo' });
});

tpl('sb-fb-retreat-escape', 'Mountain Retreat Escape', FB, 'fb-post', ['travel', 'retreat', 'wellness', 'yoga', 'nature'], (b) => {
  b.bg(solid('#1f2a2e'));
  pic(b, 'lake', 0, 0, 1200, 630, { focus: [0.6, 0.5], fill: '#5b6f73' });
  b.rect(0, 0, 1200, 630, lin(90, '#0d1416dd', '#0d141688', '#0d141600'));
  b.text('AUTUMN RETREAT · 16–19 OCT 2026', 80, 110, 620, { family: 'Tenor Sans', size: 22, tracking: 260, color: '#e8dccb' }, 'start', { role: 'subhead' });
  b.text('Four quiet days\nin the mountains', 76, 160, 660, { family: 'Instrument Serif', size: 82, color: '#ffffff' }, 'start', { lh: 1.0, role: 'headline' });
  b.text('Slow yoga, forest walks and long dinners at Alder Lodge. Twelve places only.', 80, 360, 500, { family: 'Tenor Sans', size: 22, color: '#e8dccb' }, 'start', { lh: 1.5, role: 'body' });
  b.rect(80, 480, 260, 62, '#e8dccb', { r: 4 });
  b.text('Reserve a place', 80, 497, 260, { family: 'Tenor Sans', size: 22, color: '#1f2a2e' }, 'center', { role: 'cta' });
  b.text('from €640', 370, 497, 260, { family: 'Instrument Serif', italic: true, size: 30, color: '#ffffff' });
});

tpl('sb-fb-customer-review', 'Customer Review Card', FB, 'fb-post', ['testimonial', 'review', 'customer', 'beauty', 'quote'], (b) => {
  b.bg(solid('#f9e4e4'));
  b.shape('blob', -120, 220, 560, 520, '#f2c4c4');
  pic(b, 'makeup', 90, 105, 320, 420, { clip: 'rounded', focus: [0.6, 0.5], fill: '#e3b5b5' });
  b.text('★★★★★', 500, 100, 600, { family: 'Inter', size: 40, color: '#b8323c' }, 'start');
  b.text('“My skin has never felt this calm. The night balm is a small jar of magic.”', 500, 170, 620, { family: 'DM Serif Display', size: 46, color: '#3a1418' }, 'start', { lh: 1.15, role: 'headline' });
  b.text('— Amira K., verified customer', 500, 430, 600, { family: 'Inter', weight: 600, size: 22, color: '#7a2d34' }, 'start', { role: 'subhead' });
  b.text('Petal & Fern Skincare', 500, 520, 600, { family: 'DM Serif Display', italic: true, size: 26, color: '#b8323c' }, 'start', { role: 'logo' });
});

// =================================================================== Photo-led Facebook covers

tpl('sb-fbc-realtor-split', 'Real Estate Agent Cover', FBC, 'fb-cover', ['real estate', 'realtor', 'home', 'property', 'cover'], (b) => {
  b.bg(solid('#f5f2ec'));
  pic(b, 'living', 1030, 0, 610, 624, { focus: [0.55, 0.5], fill: '#d7cfc2' });
  b.rect(400, 120, 6, 384, '#b08d57');
  b.text('HARBOUR & HEARTH REALTY', 440, 130, 640, { family: 'Inter Tight', weight: 700, size: 22, tracking: 260, color: '#8a6a3a' }, 'start', { role: 'logo' });
  b.text('Homes that fit your next chapter.', 440, 175, 540, { family: 'Playfair Display', weight: 600, size: 58, color: '#1d2a33' }, 'start', { lh: 1.08, role: 'headline' });
  b.text('Buying · Selling · Free valuations\n(555) 014-2290 · harbourhearth.example.com', 440, 370, 640, { family: 'Inter Tight', size: 24, color: '#4a5560' }, 'start', { lh: 1.6, role: 'body' });
});

tpl('sb-fbc-florist-elegant', 'Florist Studio Cover', FBC, 'fb-cover', ['florist', 'flowers', 'wedding', 'elegant', 'cover'], (b) => {
  b.bg(solid('#fbf5f1'));
  pic(b, 'vases', 0, 0, 1640, 624, { focus: [0.5, 0.82], filter: 'pastel', filterAmount: 0.5, fill: '#ead7cf' });
  b.rect(0, 0, 1640, 624, '#fbf5f1', { opacity: 0.25 });
  b.rect(470, 90, 700, 444, '#fbf5f1', { opacity: 0.94 });
  b.box(490, 110, 660, 404, '#b5838d', 1.5);
  b.text('Wild Rose', 490, 165, 660, { family: 'Italiana', size: 104, color: '#4a2c32' }, 'center', { lh: 1.0, role: 'logo' });
  b.text('FLORAL STUDIO', 490, 290, 660, { family: 'Tenor Sans', size: 26, tracking: 500, color: '#8e5b66' }, 'center', { role: 'subhead' });
  b.text('Weddings · Events · Weekly bouquets', 490, 380, 660, { family: 'Cormorant Garamond', italic: true, weight: 500, size: 32, color: '#4a2c32' }, 'center', { role: 'body' });
});

// =================================================================== X: meetup

tpl('sb-x-design-meetup', 'Design Meetup Event', X, 'x-post', ['event', 'meetup', 'design', 'community', 'tech'], (b) => {
  b.bg(solid('#111111'));
  pic(b, 'deskTop', 800, 0, 800, 900, { filter: 'mono', fill: '#3a3a3a' });
  b.rect(800, 0, 800, 900, '#ff4d00', { opacity: 0.25 });
  b.rect(0, 0, 820, 900, '#111111');
  b.text('DESIGN\nNIGHT', 100, 100, 700, { family: 'Bebas Neue', size: 220, color: '#ffffff' }, 'start', { lh: 0.88, role: 'headline' });
  b.text('#14', 560, 110, 220, { family: 'Bebas Neue', size: 90, color: '#ff4d00' }, 'end');
  b.text('Three lightning talks on type, motion and accessible colour — then pizza.', 100, 520, 620, { family: 'Space Grotesk', weight: 500, size: 34, color: '#d6d6d6' }, 'start', { lh: 1.35, role: 'body' });
  b.line(100, 700, 620, '#ff4d00', 4);
  b.text('WED 21 OCT · 18:30\nTHE FOUNDRY, PIER 4', 100, 730, 620, { family: 'Space Grotesk', weight: 700, size: 30, tracking: 60, color: '#ffffff' }, 'start', { lh: 1.4, role: 'subhead' });
  b.text('Free · RSVP at designnight.example', 1150, 800, 400, { family: 'Space Grotesk', weight: 700, size: 26, color: '#ffffff' }, 'end', { role: 'cta', effect: chip('#ff4d00', 16, 8) });
});

// =================================================================== LinkedIn: photo-led

tpl('sb-li-webinar-panel', 'Webinar Panel Speakers', LI, 'linkedin-post', ['webinar', 'event', 'panel', 'speakers', 'online'], (b) => {
  b.bg(solid('#0f2b5b'));
  b.shape('ellipse', 700, -300, 800, 800, '#1d4ed8', { opacity: 0.5 });
  b.rect(90, 80, 380, 56, '#fbbf24', { r: 6 });
  b.text('LIVE PANEL · FREE', 90, 92, 380, { family: 'Archivo', weight: 800, size: 26, tracking: 220, color: '#0f2b5b' }, 'center', { role: 'subhead' });
  b.text('The future of\nhybrid work', 90, 170, 1000, { family: 'Archivo', weight: 900, size: 104, color: '#ffffff' }, 'start', { lh: 1.0, role: 'headline' });
  b.text('Three HR leaders on what actually works in 2027.', 90, 405, 1000, { family: 'Archivo', size: 36, color: '#bfdbfe' }, 'start', { role: 'subhead' });
  const sp = [['womanRed', 'Kate Brennan', 'CPO, Brightlane'], ['womanSide', 'Mei Tanaka', 'VP People, Kitewing'], ['womanWall', 'Grace Adeyemi', 'Founder, TeamOS']];
  sp.forEach(([k, n, r], i) => {
    const x = 90 + i * 350;
    pic(b, k!, x, 510, 300, 300, { clip: 'ellipse', focus: [0.5, 0.2], fill: '#2a4a85' });
    b.text(n!, x - 20, 830, 340, { family: 'Archivo', weight: 700, size: 32, color: '#ffffff' }, 'center');
    b.text(r!, x - 20, 875, 340, { family: 'Archivo', size: 26, color: '#93c5fd' }, 'center');
  });
  b.rect(90, 990, 1020, 120, '#ffffff', { r: 20 });
  b.text('Tue 19 Jan 2027 · 4 PM GMT', 130, 1028, 560, { family: 'Archivo', weight: 700, size: 34, color: '#0f2b5b' });
  b.text('Register → link in comments', 640, 1028, 430, { family: 'Archivo', weight: 700, size: 34, color: '#1d4ed8' }, 'end', { role: 'cta' });
});

tpl('sb-li-thought-quote', 'Thought Leadership Quote', LI, 'linkedin-post', ['quote', 'thought leadership', 'leadership', 'insight'], (b) => {
  b.bg(solid('#f3efe7'));
  pic(b, 'womanDenim', 700, 0, 500, 1200, { focus: [0.45, 0.5], filter: 'mono', fill: '#cfc6b6' });
  b.rect(0, 0, 760, 1200, '#f3efe7');
  b.shape('ellipse', 690, 960, 140, 140, '#d9542b');
  b.text('“', 80, 40, 300, { family: 'Young Serif', size: 300, color: '#d9542b' });
  b.text('Culture isn’t the perks. It’s what happens when a deadline slips and nobody is watching.', 90, 330, 600, { family: 'Young Serif', size: 60, color: '#1c1a17' }, 'start', { lh: 1.18, role: 'headline' });
  b.line(90, 900, 120, '#1c1a17', 3);
  b.text('Rachel Lindqvist', 90, 940, 600, { family: 'Hanken Grotesk', weight: 700, size: 38, color: '#1c1a17' }, 'start', { role: 'subhead' });
  b.text('COO, Fernhill Logistics', 90, 992, 600, { family: 'Hanken Grotesk', size: 30, color: '#5e574c' });
});

tpl('sb-li-milestone-announcement', 'Company Milestone Announcement', LI, 'linkedin-post', ['announcement', 'milestone', 'company', 'team', 'news'], (b) => {
  b.bg(solid('#ffffff'));
  pic(b, 'team', 0, 0, 1200, 620, { fill: '#d4dde8' });
  b.rect(0, 560, 1200, 640, '#ffffff');
  b.rect(80, 480, 300, 150, '#16a34a', { r: 20 });
  b.text('10,000', 80, 500, 300, { family: 'Gabarito', weight: 900, size: 64, color: '#ffffff' }, 'center');
  b.text('CUSTOMERS', 80, 576, 300, { family: 'Gabarito', weight: 700, size: 24, tracking: 260, color: '#dcfce7' }, 'center');
  b.text('Ten thousand teams now plan their week with Fieldday.', 80, 690, 1040, { family: 'Gabarito', weight: 800, size: 64, color: '#0f172a' }, 'start', { lh: 1.1, role: 'headline' });
  b.text('Thank you to every customer who sent feedback, filed bugs and told a friend. We built this with you — and we’re just getting started.', 80, 870, 1000, { family: 'Inter', size: 32, color: '#475569' }, 'start', { lh: 1.45, role: 'body' });
  b.text('fieldday.example.com', 80, 1080, 600, { family: 'Gabarito', weight: 700, size: 30, color: '#16a34a' }, 'start', { role: 'cta' });
});

// =================================================================== LinkedIn banners: photo-led

tpl('sb-lib-workspace-photo', 'Developer Workspace Banner', BAN, 'linkedin-banner', ['linkedin', 'banner', 'developer', 'tech', 'photo'], (b) => {
  b.bg(solid('#0b1220'));
  pic(b, 'typing', 1000, 0, 584, 396, { fill: '#26324a' });
  b.rect(1000, 0, 260, 396, lin(0, '#0b1220', '#0b122000'));
  b.text('const focus = “developer tools”;', 460, 90, 640, { family: 'JetBrains Mono', size: 22, color: '#60a5fa' }, 'start', { role: 'subhead' });
  b.text('Staff Engineer building fast,\nfriendly developer platforms.', 460, 135, 640, { family: 'Space Grotesk', weight: 700, size: 38, color: '#ffffff' }, 'start', { lh: 1.15, role: 'headline' });
  b.text('TypeScript · Rust · Platform teams', 460, 300, 600, { family: 'Space Grotesk', size: 22, color: '#94a3b8' });
});

tpl('sb-lib-company-mountains', 'Company Mission Banner', BAN, 'linkedin-banner', ['linkedin', 'banner', 'company', 'climate', 'nature'], (b) => {
  b.bg(solid('#173d34'));
  pic(b, 'fjord', 0, 0, 1584, 396, { focus: [0.5, 0.6], duotone: ['#0f2a24', '#9fd8c0'], fill: '#2e5a4e' });
  b.rect(0, 0, 1584, 396, '#0f2a24', { opacity: 0.35 });
  b.text('Software for the teams cutting carbon.', 470, 120, 1060, { family: 'Fraunces', weight: 600, size: 52, color: '#ffffff' }, 'start', { role: 'headline' });
  b.text('TERRALOOP  ·  WE’RE HIRING ENGINEERS IN OSLO & REMOTE', 474, 230, 1000, { family: 'Manrope', weight: 700, size: 22, tracking: 200, color: '#c9f0de' }, 'start', { role: 'subhead' });
});

// =================================================================== Pinterest pins: photo-led

tpl('sb-pin-recipe-salad', 'Quick Salad Recipe Pin', PIN, 'pinterest-pin', ['recipe', 'food', 'cooking', 'salad', 'healthy'], (b) => {
  b.bg(solid('#fff8ee'));
  pic(b, 'salad', 0, 0, 1000, 860, { focus: [0.5, 0.45], fill: '#e8d3b8' });
  b.rect(60, 760, 880, 640, '#fff8ee', { r: 32 });
  b.text('15-MINUTE LUNCH', 60, 810, 880, { family: 'DM Sans', weight: 700, size: 28, tracking: 260, color: '#b4441a' }, 'center', { role: 'subhead' });
  b.text('Crunchy Tomato\nHerb Salad', 60, 860, 880, { family: 'Fraunces', weight: 800, size: 100, color: '#2a160b' }, 'center', { lh: 1.0, role: 'headline' });
  const facts = [['15', 'MIN'], ['2', 'SERVES'], ['1', 'BOWL']];
  facts.forEach(([n, l], i) => {
    const x = 160 + i * 240;
    b.text(n!, x, 1105, 200, { family: 'Fraunces', weight: 700, size: 60, color: '#b4441a' }, 'center', { lh: 1.0 });
    b.text(l!, x, 1175, 200, { family: 'DM Sans', weight: 700, size: 22, tracking: 200, color: '#6b4a35' }, 'center');
  });
  b.text('Get the recipe → tinykitchen.example.com', 60, 1300, 880, { family: 'DM Sans', weight: 700, size: 30, color: '#2a160b' }, 'center', { role: 'cta' });
});

tpl('sb-pin-diy-propagation', 'DIY Plant Steps Pin', PIN, 'pinterest-pin', ['diy', 'plants', 'how to', 'home', 'steps'], (b) => {
  b.bg(solid('#eef3e6'));
  pic(b, 'pothos', 0, 0, 1000, 600, { focus: [0.55, 0.4], fill: '#b9c9a6' });
  b.text('DIY · BEGINNER FRIENDLY', 70, 650, 860, { family: 'Space Grotesk', weight: 700, size: 26, tracking: 200, color: '#4d7c0f' }, 'start', { role: 'subhead' });
  b.text('Propagate a pothos\nin a jar', 70, 700, 860, { family: 'Bricolage Grotesque', weight: 800, size: 80, color: '#1a2e05' }, 'start', { lh: 1.0, role: 'headline' });
  const steps = ['Cut below a node, with 2–3 leaves on top.', 'Stand the cutting in a clear jar of water.', 'Keep it in bright, indirect light.', 'Pot it once the roots reach 5 cm.'];
  steps.forEach((t, i) => {
    const y = 900 + i * 115;
    b.circle(110, y + 34, 38, '#4d7c0f');
    b.text(String(i + 1), 72, y + 12, 76, { family: 'Bricolage Grotesque', weight: 800, size: 36, color: '#ffffff' }, 'center');
    b.text(t, 175, y + 14, 760, { family: 'Space Grotesk', weight: 500, size: 32, color: '#1a2e05' }, 'start', { role: 'body' });
  });
  b.text('greenshelf.example.com', 70, 1400, 860, { family: 'Space Grotesk', weight: 700, size: 26, color: '#4d7c0f' }, 'start', { role: 'cta' });
});

tpl('sb-pin-travel-guide', 'Mountain Travel Guide Pin', PIN, 'pinterest-pin', ['travel', 'guide', 'itinerary', 'mountains', 'road trip'], (b) => {
  b.bg(solid('#0f3b4a'));
  pic(b, 'lake', 0, 0, 1000, 880, { focus: [0.55, 0.5], filter: 'cool', filterAmount: 0.4, fill: '#5f9ea0' });
  b.rect(0, 0, 1000, 300, lin(180, '#0f3b4acc', '#0f3b4a00'));
  b.rect(0, 600, 1000, 290, lin(180, '#0f3b4a00', '#0f3b4a'));
  b.text('Road trip guide', 70, 70, 860, { family: 'Caveat', weight: 700, size: 72, color: '#ffd166' }, 'start', { role: 'subhead' });
  b.text('3 DAYS IN THE\nCANADIAN ROCKIES', 70, 690, 900, { family: 'Anton', size: 96, color: '#ffffff' }, 'start', { lh: 1.0, role: 'headline' });
  const days = [['DAY 1', 'Lakeshore sunrise & canoe hire'], ['DAY 2', 'Glacier hike + hot springs'], ['DAY 3', 'Scenic parkway & mountain town']];
  days.forEach(([d, t], i) => {
    const y = 940 + i * 120;
    b.rect(70, y, 860, 100, '#1b4d5e', { r: 18 });
    b.text(d!, 100, y + 30, 150, { family: 'Anton', size: 36, color: '#ffd166' });
    b.text(t!, 250, y + 30, 660, { family: 'DM Sans', weight: 600, size: 34, color: '#ffffff' }, 'start', { role: 'body' });
  });
  b.rect(270, 1330, 460, 80, '#ffd166', { r: 40 });
  b.text('Get the full itinerary', 270, 1352, 460, { family: 'DM Sans', weight: 800, size: 30, color: '#0f3b4a' }, 'center', { role: 'cta' });
});

tpl('sb-pin-floral-quote', 'Floral Quote Pin', PIN, 'pinterest-pin', ['quote', 'inspiration', 'flowers', 'self care', 'feminine'], (b) => {
  b.bg(solid('#f6e7e1'));
  pic(b, 'roses', 0, 0, 1000, 1500, { filter: 'blush', filterAmount: 0.4, fill: '#ecd2c8' });
  b.rect(110, 330, 780, 840, '#fffaf6', { opacity: 0.96, r: 400 });
  b.text('Bloom', 110, 430, 780, { family: 'Allura', size: 130, color: '#b5677a' }, 'center', { role: 'subhead' });
  b.text('where you are planted,\nand grow at your\nown pace.', 170, 640, 660, { family: 'Cormorant Garamond', weight: 600, size: 62, color: '#4a2a33' }, 'center', { lh: 1.15, role: 'headline' });
  b.line(450, 940, 100, '#b5677a', 2);
  b.text('SOFT DAYS JOURNAL', 170, 980, 660, { family: 'Tenor Sans', size: 22, tracking: 400, color: '#8e5b66' }, 'center', { role: 'logo' });
});

tpl('sb-pin-gift-roundup', 'Gift Guide Round-up Pin', PIN, 'pinterest-pin', ['gift guide', 'product', 'shopping', 'roundup', 'holiday'], (b) => {
  b.bg(solid('#1e3a2f'));
  b.text('GIFT GUIDE 2026', 70, 80, 860, { family: 'Archivo', weight: 800, size: 28, tracking: 300, color: '#f2c14e' }, 'center', { role: 'subhead' });
  b.text('Cosy gifts\nunder $50', 70, 125, 860, { family: 'Young Serif', size: 112, color: '#fbf3e4' }, 'center', { lh: 1.0, role: 'headline' });
  const items = [['makeup', 'Travel beauty kit', '$38'], ['rose', 'Garden roses', '$35'], ['succulent', 'Succulent pot', '$24'], ['baking', 'Baking box', '$29']];
  items.forEach(([k, n, p], i) => {
    const x = 70 + (i % 2) * 440, y = 400 + Math.floor(i / 2) * 500;
    b.rect(x, y, 420, 470, '#fbf3e4', { r: 24 });
    pic(b, k!, x + 20, y + 20, 380, 330, { r: 16, fill: '#d9cdb8' });
    b.text(n!, x + 30, y + 372, 260, { family: 'Archivo', weight: 700, size: 30, color: '#1e3a2f' }, 'start');
    b.text(p!, x + 280, y + 368, 110, { family: 'Young Serif', size: 36, color: '#b5452b' }, 'end');
    b.text(`0${i + 1}`, x + 30, y + 414, 200, { family: 'Archivo', weight: 600, size: 22, color: '#6b7f73' }, 'start');
  });
  b.text('Shop the list → hearthandhome.example.com', 70, 1420, 860, { family: 'Archivo', weight: 700, size: 28, color: '#f2c14e' }, 'center', { role: 'cta' });
});
