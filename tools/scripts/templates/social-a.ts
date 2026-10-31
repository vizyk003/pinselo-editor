/** Templates: social-a — Instagram posts, portrait posts and stories (see index.ts for categories). */
import { B, lin, rad, solid, tpl, type PhotoOpts } from './builder';

void [B, lin, rad, solid];

const POST = 'Instagram Posts';
const STORY = 'Instagram Stories';

/** Pinned CC0 photos (Openverse ids) shared across this file's templates. */
const PH: Record<string, string> = {
  woman: 'ff3692f6-3136-42d1-a73e-e50efae59c1b', beauty: '3e6dcbb4-d863-4051-a79a-5867b16d461a', skincare: 'c9c8e571-fb3c-471b-84e4-6ea091ba3e18',
  house: '2b739227-2ec6-4440-99db-9f5df808d1e3', food: 'c95a7712-38a1-4372-a2df-6d3b02de3ae2', gym: '81471031-0862-4b68-a028-0631b190c4a9',
  mountains: 'picsum:29', coast: 'picsum:49', city: 'picsum:308', travel2: 'picsum:218', studio: 'picsum:250', studio2: 'picsum:91', laptop: 'picsum:42',
};
/** A photo by key, or an empty frame while the key is unset. */
function pic(b: B, key: keyof typeof PH, x: number, y: number, w: number, h: number, o: PhotoOpts = {}) {
  const id = PH[key];
  if (id) return b.photo(id, x, y, w, h, o);
  return b.frame(x, y, w, h, o.clip ?? (o.r ? 'rounded' : 'rect'), '#b9b2a8');
}

// =================================================================== Instagram posts (1080 × 1080)

// ---- Carousel: 3 habits for deep work (5 pages, type-led, warm editorial)
tpl('sa-ig-carousel-deep-work', 'Deep Work Habits Carousel', POST, 'ig-post', ['carousel', 'tips', 'productivity', 'listicle', 'education'], (b) => {
  const cream = '#f4eee4', ink = '#1f1b16', terra = '#b8472f', muted = '#6b5f52';
  const serif = 'Fraunces', sans = 'Inter Tight';
  const footer = (i: number) => {
    b.line(88, 980, 904, '#d9cdbd', 2);
    b.text('@northdesk.studio', 88, 1000, 500, { family: sans, weight: 600, size: 26, color: muted }, 'start', { role: 'logo' });
    b.text(`${String(i).padStart(2, '0')} / 05`, 692, 1000, 300, { family: sans, weight: 600, size: 26, color: muted, tracking: 80 }, 'end');
  };
  // cover
  b.page(0).bg(cream);
  b.circle(900, 190, 140, terra);
  b.text('A FIELD GUIDE', 88, 96, 600, { family: sans, weight: 700, size: 28, tracking: 240, color: terra }, 'start', { role: 'subhead' });
  b.text('3 habits for\ndeeper work', 88, 400, 904, { family: serif, weight: 400, size: 150, color: ink, tracking: -20 }, 'start', { lh: 1.0, role: 'headline' });
  b.text('Small, boring rituals that protect your best hours.', 88, 740, 700, { family: sans, size: 36, color: muted }, 'start', { lh: 1.35, role: 'body' });
  b.text('Swipe  →', 692, 900, 300, { family: sans, weight: 700, size: 30, color: ink, tracking: 60 }, 'end', { role: 'cta' });
  footer(1);
  // tips
  const tips = [
    ['Guard the first\n90 minutes', 'No inbox, no feeds. Open the one document that matters before anything else is allowed to ask for you.'],
    ['Batch the\nshallow stuff', 'Answer messages twice a day at fixed times. People adapt faster than you expect — and so does your focus.'],
    ['End with a\nshutdown note', 'Write tomorrow’s first task before you log off. Starting is the hardest part; make it a decision you’ve already made.'],
  ];
  tips.forEach(([h, body], i) => {
    b.page(i + 1).bg(cream);
    b.text(`0${i + 1}`, 88, 70, 400, { family: serif, weight: 300, size: 220, color: terra, italic: true }, 'start', { lh: 1.0 });
    b.rect(88, 400, 72, 8, ink);
    b.text(h!, 88, 450, 904, { family: serif, weight: 500, size: 96, color: ink, tracking: -10 }, 'start', { lh: 1.05, role: 'headline' });
    b.text(body!, 88, 700, 820, { family: sans, size: 36, color: muted }, 'start', { lh: 1.45, role: 'body' });
    footer(i + 2);
  });
  // closing
  b.page(4).bg(ink);
  b.circle(540, 330, 150, terra);
  b.emoji('🔖', 465, 255, 150);
  b.text('Save this for\nMonday morning.', 88, 540, 904, { family: serif, weight: 400, size: 96, color: cream, tracking: -10 }, 'center', { lh: 1.05, role: 'headline' });
  b.text('Follow for a new field guide every week.', 88, 790, 904, { family: sans, size: 34, color: '#cfc3b3' }, 'center', { role: 'cta' });
  b.text('@northdesk.studio', 88, 1000, 904, { family: sans, weight: 600, size: 26, color: '#cfc3b3' }, 'center', { role: 'logo' });
}, 5);

// ---- Carousel: the 50/30/20 budget (4 pages, data-led, green + butter)
tpl('sa-ig-carousel-budget', 'Budget Basics Carousel', POST, 'ig-post', ['carousel', 'finance', 'budget', 'education', 'chart'], (b) => {
  const forest = '#0e3b2e', mint = '#d7efe1', butter = '#f6c945', coral = '#ef7b5a', white = '#ffffff';
  const head = 'Bricolage Grotesque', sans = 'DM Sans';
  const chip = (s: string, i: number, dark: boolean) => {
    b.rect(88, 88, 220, 60, dark ? butter : forest, { r: 30 });
    b.text(s, 88, 101, 220, { family: sans, weight: 700, size: 24, tracking: 120, color: dark ? forest : white }, 'center');
    b.text(`${i} / 4`, 772, 101, 220, { family: sans, weight: 700, size: 26, color: dark ? mint : forest }, 'end');
  };
  // cover
  b.page(0).bg(forest);
  chip('MONEY 101', 1, true);
  b.text('50/30/20', 88, 250, 904, { family: head, weight: 800, size: 210, color: butter, tracking: -30 }, 'start', { lh: 1.0, role: 'headline' });
  b.text('The simplest budget\nyou’ll actually keep.', 88, 520, 904, { family: head, weight: 600, size: 76, color: white }, 'start', { lh: 1.1, role: 'subhead' });
  b.rect(88, 870, 904, 2, '#2d6a55');
  b.text('Penny & Pine · Personal finance, minus the jargon', 88, 905, 760, { family: sans, size: 28, color: mint }, 'start', { role: 'logo' });
  b.shape('arrow', 892, 896, 100, 48, butter);
  // split
  b.page(1).bg(mint);
  chip('THE SPLIT', 2, false);
  // treemap of the split: needs | wants / savings
  b.rect(88, 200, 452, 560, forest, { r: 28 });
  b.rect(556, 200, 436, 300, '#c9502f', { r: 28 });
  b.rect(556, 516, 436, 244, butter, { r: 28 });
  const cells = [[88, 200, 452, '50%', 'NEEDS', 'Rent, groceries, bills', white], [556, 200, 436, '30%', 'WANTS', 'Dinners out, hobbies', white], [556, 516, 436, '20%', 'SAVINGS', 'Emergency fund, debt', forest]] as const;
  cells.forEach(([x, y, w, p, t, d, c]) => {
    b.text(t, x + 40, y + 32, w - 80, { family: sans, weight: 700, size: 24, tracking: 160, color: c });
    b.text(p, x + 40, y + 66, w - 80, { family: head, weight: 800, size: 96, color: c }, 'start', { lh: 1.0 });
    b.text(d, x + 40, y + 172, w - 80, { family: sans, size: 26, color: c }, 'start', { lh: 1.3 });
  });
  b.text('Split your take-home pay three ways. That’s the whole system.', 88, 820, 904, { family: head, weight: 600, size: 48, color: forest }, 'start', { lh: 1.15, role: 'body' });
  // example
  b.page(2).bg(white);
  chip('EXAMPLE', 3, false);
  b.text('On a $3,200 monthly paycheck', 88, 190, 904, { family: head, weight: 700, size: 64, color: forest }, 'start', { lh: 1.1, role: 'headline' });
  const bars = [['Needs', 1600, forest], ['Wants', 960, '#c9502f'], ['Savings', 640, butter]] as const;
  bars.forEach(([t, v, c], i) => {
    const y = 380 + i * 170;
    b.text(t, 88, y, 400, { family: sans, weight: 700, size: 32, color: forest });
    b.text(`$${v.toLocaleString('en-US')}`, 592, y, 400, { family: head, weight: 800, size: 40, color: forest }, 'end');
    b.rect(88, y + 60, 904, 44, '#eef5f0', { r: 22 });
    b.rect(88, y + 60, Math.round((904 * v) / 1600), 44, c, { r: 22 });
  });
  b.text('Tip: automate the 20% on payday so it never feels optional.', 88, 900, 904, { family: sans, size: 30, color: '#4b6b5f' }, 'start', { lh: 1.35, role: 'body' });
  // cta
  b.page(3).bg(butter);
  chip('YOUR TURN', 4, false);
  b.text('Try it for\none month.', 88, 260, 904, { family: head, weight: 800, size: 150, color: forest, tracking: -20 }, 'start', { lh: 0.98, role: 'headline' });
  b.text('Save this post, then share it with the friend who keeps asking where their money goes.', 88, 600, 820, { family: sans, size: 36, color: forest }, 'start', { lh: 1.4, role: 'body' });
  b.rect(88, 830, 470, 100, forest, { r: 50 });
  b.text('Free template ↗ link in bio', 88, 862, 470, { family: sans, weight: 700, size: 30, color: white }, 'center', { role: 'cta' });
}, 4);

// ---- Event: design workshop (type-led, bold grid)
tpl('sa-ig-event-workshop', 'Type Workshop Event', POST, 'ig-post', ['event', 'workshop', 'design', 'class', 'creative'], (b) => {
  const bg = '#ece8ff', ink = '#1a1446', violet = '#5b3df5', lime = '#d4f25c';
  b.bg(bg);
  b.text('WORKSHOP · 2 DAYS', 88, 88, 600, { family: 'Space Grotesk', weight: 700, size: 28, tracking: 160, color: violet }, 'start', { role: 'subhead' });
  b.shape('burst', 790, 50, 220, 220, lime, { params: { points: 14 } });
  b.text('NOV\n14–15', 790, 110, 220, { family: 'Space Grotesk', weight: 700, size: 34, color: ink }, 'center', { lh: 1.05 });
  b.text('Letters\nthat\nmove.', 88, 210, 904, { family: 'Unbounded', weight: 800, size: 150, color: ink, tracking: -20 }, 'start', { lh: 0.98, role: 'headline' });
  b.rect(88, 760, 904, 3, ink);
  b.text('Hands-on variable type & kinetic lettering with Ines Okafor', 88, 790, 560, { family: 'Space Grotesk', weight: 500, size: 32, color: ink }, 'start', { lh: 1.3, role: 'body' });
  b.text('Foundry Hall, Room 3\n10am – 4pm · $180', 680, 790, 312, { family: 'Space Grotesk', weight: 500, size: 28, color: ink }, 'end', { lh: 1.4 });
  b.rect(88, 940, 340, 76, violet, { r: 38 });
  b.text('Book a seat', 88, 960, 340, { family: 'Space Grotesk', weight: 700, size: 30, color: '#ffffff' }, 'center', { role: 'cta' });
  b.text('foundryhall.example.com', 452, 962, 540, { family: 'Space Grotesk', weight: 500, size: 28, color: violet }, 'end');
});

// ---- Testimonial card (type-led, soft, with avatar frame)
tpl('sa-ig-testimonial-card', 'Client Testimonial', POST, 'ig-post', ['testimonial', 'review', 'customer', 'business', 'quote'], (b) => {
  const bg = '#fdf1ea', card = '#ffffff', ink = '#2b2118', accent = '#e2603f', muted = '#8a7566';
  b.bg(bg);
  b.circle(980, 120, 220, '#f8d9c8');
  b.circle(80, 1000, 160, '#f8d9c8');
  b.rect(88, 140, 904, 800, card, { r: 48, shadow: [0, 18, 50, '#e8c7b333'] });
  b.text('★★★★★', 160, 210, 600, { family: 'Inter Tight', weight: 700, size: 48, color: '#b8730c', tracking: 80 });
  b.text('“They redesigned our whole menu in a week and bookings went up 30% the next month. Calm, fast, and genuinely kind to work with.”', 160, 300, 760, { family: 'Instrument Serif', size: 64, color: ink }, 'start', { lh: 1.15, role: 'quote' });
  b.frame(160, 750, 120, 120, 'ellipse', '#f3c9b3');
  b.text('Dana Whitfield', 310, 768, 600, { family: 'Inter Tight', weight: 700, size: 34, color: ink }, 'start', { role: 'subhead' });
  b.text('Owner, Little Fig Bistro', 310, 818, 600, { family: 'Inter Tight', size: 28, color: muted });
  b.text('KIND WORDS', 88, 990, 600, { family: 'Inter Tight', weight: 700, size: 24, tracking: 240, color: accent });
  b.text('hearthandco.example', 492, 990, 500, { family: 'Inter Tight', weight: 600, size: 24, color: muted }, 'end', { role: 'logo' });
});

// =================================================================== Instagram portrait posts (1080 × 1350)

// ---- Hiring announcement (bold, type-led)
tpl('sa-igp-hiring-bold', 'Bold Hiring Post', POST, 'ig-portrait', ['hiring', 'jobs', 'career', 'announcement', 'tech'], (b) => {
  const bg = '#111111', lime = '#c6f432', white = '#f5f5f0', grey = '#a3a39b';
  b.bg(bg);
  b.text('KITEWORKS IS', 88, 110, 904, { family: 'Archivo', weight: 600, size: 30, tracking: 220, color: grey }, 'start', { role: 'logo' });
  b.text('WE’RE\nHIRING', 80, 170, 920, { family: 'Anton', size: 260, color: lime }, 'start', { lh: 0.92, role: 'headline' });
  b.text('3 open roles · remote-first · Lisbon or anywhere ±2h', 88, 690, 904, { family: 'Archivo', weight: 500, size: 32, color: white }, 'start', { role: 'subhead' });
  const roles = [['Senior Frontend Engineer', 'React · TypeScript'], ['Product Designer', 'Systems · Prototyping'], ['Customer Success Lead', 'B2B SaaS · EN / PT']];
  roles.forEach(([r, d], i) => {
    const y = 780 + i * 120;
    b.line(88, y, 904, '#3a3a36', 2);
    b.text(r!, 88, y + 30, 640, { family: 'Archivo', weight: 700, size: 38, color: white });
    b.text(d!, 640, y + 38, 352, { family: 'Archivo', size: 28, color: grey }, 'end');
  });
  b.line(88, 1140, 904, '#3a3a36', 2);
  b.rect(88, 1190, 380, 84, lime, { r: 42 });
  b.text('Apply by Dec 12', 88, 1212, 380, { family: 'Archivo', weight: 800, size: 32, color: bg }, 'center', { role: 'cta' });
  b.text('kiteworks.example/jobs', 492, 1214, 500, { family: 'Archivo', weight: 500, size: 30, color: white }, 'end');
});

// ---- Self-care checklist (illustrated, pastel)
tpl('sa-igp-selfcare-checklist', 'Sunday Reset Checklist', POST, 'ig-portrait', ['checklist', 'wellness', 'self care', 'lifestyle', 'tips'], (b) => {
  const bg = '#f7e9e3', ink = '#4a2c2a', rose = '#d98579', sage = '#9fb59a', card = '#fffaf6';
  b.bg(bg);
  b.shape('blob', 700, -120, 520, 480, '#f1d3c8');
  b.shape('blob', -160, 1050, 520, 460, '#dfe8d9', { rot: 40 });
  b.emoji('🌷', 820, 90, 160, { rot: 10 });
  b.text('the', 88, 120, 400, { family: 'Caveat', weight: 600, size: 72, color: '#b5554a' });
  b.text('Sunday\nReset', 88, 190, 800, { family: 'Gloock', size: 140, color: ink }, 'start', { lh: 0.98, role: 'headline' });
  b.rect(88, 520, 904, 660, card, { r: 40 });
  const items = ['Change the sheets & open a window', 'Plan three meals for the week', 'Clear your downloads folder', 'A long walk, no headphones', 'Text someone you miss', 'In bed by 10 with a paper book'];
  items.forEach((t, i) => {
    const y = 580 + i * 98;
    b.box(140, y, 48, 48, i < 2 ? rose : sage, 4, 14);
    if (i < 2) b.text('✓', 140, y - 2, 48, { family: 'Inter Tight', weight: 800, size: 40, color: rose }, 'center');
    b.text(t, 220, y + 2, 720, { family: 'DM Sans', weight: 500, size: 36, color: ink }, 'start', { role: i === 0 ? 'body' : undefined });
  });
  b.text('save for your next slow Sunday ♡', 88, 1220, 904, { family: 'Caveat', weight: 600, size: 52, color: '#a8584c' }, 'center', { role: 'cta' });
});

// ---- Nonprofit fundraiser with progress (warm, community)
tpl('sa-igp-fundraiser', 'Community Fundraiser', POST, 'ig-portrait', ['nonprofit', 'fundraiser', 'charity', 'community', 'donate'], (b) => {
  const bg = '#fff6e0', ink = '#1d3a2f', orange = '#e8682c', green = '#2f7a55';
  b.bg(bg);
  b.rect(0, 0, 1080, 560, ink);
  b.emoji('📚', 700, 120, 300, { rot: -8 });
  b.text('BRIGHT SHELVES FUND', 88, 110, 600, { family: 'Hanken Grotesk', weight: 800, size: 28, tracking: 200, color: '#f7c873' }, 'start', { role: 'logo' });
  b.text('Help us fill\n12 school\nlibraries.', 88, 180, 640, { family: 'Young Serif', size: 92, color: '#fff6e0' }, 'start', { lh: 1.05, role: 'headline' });
  b.text('$18,400', 88, 630, 600, { family: 'Young Serif', size: 110, color: ink }, 'start', { lh: 1.0 });
  b.text('raised of $25,000', 88, 760, 600, { family: 'Hanken Grotesk', weight: 600, size: 34, color: '#4b6358' });
  b.text('74%', 692, 660, 300, { family: 'Hanken Grotesk', weight: 800, size: 72, color: '#b84a14' }, 'end');
  b.rect(88, 840, 904, 40, '#f1dfb4', { r: 20 });
  b.rect(88, 840, 669, 40, lin(0, orange, '#f2a33a'), { r: 20 });
  b.text('Every $15 puts five new books on a shelf in a rural classroom. 212 neighbours have already given — will you be next?', 88, 930, 904, { family: 'Hanken Grotesk', size: 34, color: ink }, 'start', { lh: 1.4, role: 'body' });
  b.rect(88, 1160, 904, 100, green, { r: 50 });
  b.text('Donate at brightshelves.example', 88, 1189, 904, { family: 'Hanken Grotesk', weight: 800, size: 36, color: '#ffffff' }, 'center', { role: 'cta' });
});

// =================================================================== Instagram stories (1080 × 1920)
// Safe zones: key content stays between y≈250 and y≈1580.

// ---- Type-led statement story
tpl('sa-story-type-statement', 'New Drop Type Story', STORY, 'ig-story', ['announcement', 'typography', 'bold', 'launch', 'story'], (b) => {
  const red = '#ff3b1f', cream = '#fff4e6', ink = '#1a0f0a';
  b.bg(red);
  // decorative watermark rows
  for (let i = 0; i < 10; i++) b.text('NEW DROP · NEW DROP · NEW DROP', i % 2 ? -420 : -120, 20 + i * 190, 2400, { family: 'Anton', size: 150, color: '#ff5a3f' }, 'start', { lh: 1.0 });
  b.rect(80, 500, 920, 920, cream, { r: 8, rot: -3 });
  b.text('THE WINTER EDIT', 140, 600, 800, { family: 'Space Grotesk', weight: 700, size: 34, tracking: 220, color: red }, 'center', { role: 'subhead', rot: -3 });
  b.text('Wear\nit\nloud.', 140, 680, 800, { family: 'Anton', size: 210, color: ink }, 'center', { lh: 0.95, role: 'headline', rot: -3 });
  b.text('Fri 07.11 · 10:00 · online only', 140, 1320, 800, { family: 'Space Grotesk', weight: 600, size: 38, color: ink }, 'center', { rot: -3 });
  b.rect(290, 1460, 500, 100, ink, { r: 50 });
  b.text('Turn on alerts', 290, 1488, 500, { family: 'Space Grotesk', weight: 700, size: 38, color: cream }, 'center', { role: 'cta' });
  b.text('@loudhouse.studio', 0, 1590, 1080, { family: 'Space Grotesk', weight: 700, size: 34, color: cream }, 'center', { role: 'logo' });
});

// ---- Q&A / ask me anything
tpl('sa-story-ask-me', 'Ask Me Anything Story', STORY, 'ig-story', ['q&a', 'questions', 'engagement', 'interactive', 'story'], (b) => {
  const top = '#c3b5ff', bottom = '#ffd0e6', ink = '#231942', violet = '#5e3bd6';
  b.bg(lin(90, top, bottom));
  b.circle(940, 330, 200, '#ffffff', { opacity: 0.25 });
  b.circle(120, 1500, 260, '#ffffff', { opacity: 0.25 });
  b.emoji('💬', 420, 300, 240, { rot: -6 });
  b.text('Ask me\nanything', 80, 590, 920, { family: 'Fredoka', weight: 700, size: 170, color: ink }, 'center', { lh: 0.95, role: 'headline' });
  b.text('Plant care, propagation, pests — I’m answering every question live on Thursday at 7pm.', 140, 990, 800, { family: 'DM Sans', weight: 500, size: 40, color: ink }, 'center', { lh: 1.4, role: 'body' });
  b.rect(140, 1210, 800, 300, '#ffffff', { r: 44, shadow: [0, 16, 40, '#5e3bd633'] });
  b.text('Drop your question here ↓', 140, 1250, 800, { family: 'DM Sans', weight: 700, size: 36, color: violet }, 'center', { role: 'cta' });
  b.rect(200, 1330, 680, 130, '#f1edff', { r: 28 });
  b.text('Type something…', 240, 1370, 600, { family: 'DM Sans', size: 34, color: '#6b6390' }, 'start');
  b.text('@fernandfrond', 0, 1600, 1080, { family: 'DM Sans', weight: 700, size: 32, color: ink }, 'center', { role: 'logo' });
});

// ---- Countdown / launch
tpl('sa-story-countdown', 'Launch Countdown Story', STORY, 'ig-story', ['countdown', 'launch', 'announcement', 'product', 'story'], (b) => {
  const bg = '#0b0d1a', blue = '#5b7cfa', cyan = '#62e3ff', white = '#f4f6ff', grey = '#9aa3c7';
  b.bg(rad('#1b2150', bg));
  b.circle(540, 820, 470, blue, { opacity: 0.18 });
  b.circle(540, 820, 360, cyan, { opacity: 0.12 });
  b.shape('ring', 160, 400, 760, 760, lin(90, blue, cyan), { params: { thickness: 0.03 } });
  b.text('LAUNCHING IN', 80, 290, 920, { family: 'Space Grotesk', weight: 700, size: 36, tracking: 300, color: cyan }, 'center', { role: 'subhead' });
  b.text('03', 80, 560, 920, { family: 'Unbounded', weight: 800, size: 320, color: white }, 'center', { lh: 1.0, role: 'headline' });
  b.text('DAYS', 80, 920, 920, { family: 'Space Grotesk', weight: 700, size: 52, tracking: 400, color: white }, 'center');
  b.text('Orbit 2 — the quietest smart speaker we’ve ever made.', 140, 1220, 800, { family: 'Space Grotesk', weight: 500, size: 44, color: white }, 'center', { lh: 1.3, role: 'body' });
  b.text('November 20, 2026 · 9:00 AM PT', 80, 1380, 920, { family: 'Space Grotesk', size: 34, color: grey }, 'center');
  b.rect(290, 1480, 500, 96, white, { r: 48 });
  b.text('Get reminded', 290, 1506, 500, { family: 'Space Grotesk', weight: 700, size: 38, color: bg }, 'center', { role: 'cta' });
});

// ---- Birthday celebration
tpl('sa-story-birthday', 'Birthday Wishes Story', STORY, 'ig-story', ['birthday', 'celebration', 'party', 'personal', 'story'], (b) => {
  const bg = '#fff3c4', pink = '#ff5fa2', ink = '#3b1d4a', blue = '#4cc3ff';
  b.bg(bg);
  const dots = [[140, 660, 24, pink], [960, 640, 20, blue], [150, 1260, 20, blue], [940, 1220, 30, pink], [540, 230, 16, '#ffb02e'], [880, 900, 18, '#ffb02e'], [100, 960, 18, '#7bd88f']] as const;
  dots.forEach(([x, y, r, c]) => b.circle(x, y, r, c));
  b.emoji('🎈', 60, 280, 260, { rot: -12 });
  b.emoji('🎈', 780, 300, 230, { rot: 10 });
  b.emoji('🎂', 400, 1060, 280);
  b.text('Happy', 80, 560, 920, { family: 'Pacifico', size: 150, color: '#e0337f' }, 'center', { role: 'headline' });
  b.text('BIRTHDAY,', 80, 760, 920, { family: 'Lilita One', size: 120, color: ink, tracking: 40 }, 'center');
  b.text('Maya!', 80, 880, 920, { family: 'Lilita One', size: 120, color: ink, tracking: 40 }, 'center', { role: 'subhead' });
  b.text('Thirty looks ridiculously good on you. Cake at mine, Saturday 8pm — no excuses.', 140, 1390, 800, { family: 'Fredoka', weight: 500, size: 44, color: ink }, 'center', { lh: 1.35, role: 'body' });
  b.text('— love, the whole crew ♡', 140, 1580, 800, { family: 'Fredoka', weight: 600, size: 36, color: '#b8246a' }, 'center');
});

// ---- Café menu story (illustrated, warm)
tpl('sa-story-cafe-menu', 'Autumn Café Menu Story', STORY, 'ig-story', ['menu', 'cafe', 'coffee', 'food', 'seasonal'], (b) => {
  const bg = '#f3e6d3', ink = '#3d2416', rust = '#a6461f', olive = '#6c6a2e', card = '#fbf5ec';
  b.bg(bg);
  b.shape('arch', 90, 250, 900, 1380, card);
  b.shape('arch', 120, 280, 840, 1320, card, { stroke: ['#d9bf9d', 3] });
  b.emoji('🍂', 120, 160, 120, { rot: -20 });
  b.emoji('☕', 470, 300, 140);
  b.text('autumn menu', 140, 450, 800, { family: 'Dancing Script', weight: 700, size: 70, color: rust }, 'center');
  b.text('HONEY & OAK', 140, 540, 800, { family: 'Cormorant Garamond', weight: 700, size: 92, color: ink, tracking: 60 }, 'center', { role: 'headline' });
  b.line(440, 680, 200, rust, 3);
  const items = [['Maple Pecan Latte', 'oat milk, toasted pecan, sea salt', '5.80'], ['Chai Brown Butter Bun', 'cardamom glaze, warm from 8am', '4.20'], ['Spiced Pear Galette', 'crème fraîche, honeycomb', '6.50'], ['Smoky Squash Soup', 'sourdough, chilli oil', '9.00']];
  items.forEach(([n, d, pr], i) => {
    const y = 730 + i * 170;
    b.text(n!, 190, y, 560, { family: 'Cormorant Garamond', weight: 700, size: 48, color: ink });
    b.text(pr!, 750, y + 4, 140, { family: 'Cormorant Garamond', weight: 700, size: 46, color: rust }, 'end');
    b.text(d!, 190, y + 64, 700, { family: 'Lora', italic: true, size: 30, color: olive });
  });
  b.text('Open daily 7–4 · 18 Mill Lane', 140, 1440, 800, { family: 'Lora', size: 32, color: ink }, 'center', { role: 'body' });
  b.emoji('🍁', 860, 1660, 120, { rot: 18 });
});

// =================================================================== Photo-led Instagram posts (1080 × 1080)

// ---- Flash sale (split: photo + colour block)
tpl('sa-ig-sale-flash', 'Weekend Flash Sale', POST, 'ig-post', ['sale', 'fashion', 'promo', 'discount', 'retail'], (b) => {
  const mustard = '#f2b631', ink = '#191512', cream = '#fff8ea';
  b.bg(mustard);
  pic(b, 'woman', 0, 0, 520, 1080, { focus: [0.5, 0.35], filter: 'warm', filterAmount: 0.5 });
  b.text('MARLOWE & CO.', 584, 88, 420, { family: 'Syne', weight: 700, size: 26, tracking: 220, color: ink }, 'start', { role: 'logo' });
  b.text('FLASH\nSALE', 580, 170, 440, { family: 'Anton', size: 130, color: ink, tracking: 10 }, 'start', { lh: 0.95, role: 'headline' });
  b.text('40%', 576, 410, 440, { family: 'Anton', size: 200, color: '#8a2f0c' }, 'start', { lh: 1.0 });
  b.text('OFF EVERYTHING', 584, 650, 420, { family: 'Syne', weight: 700, size: 34, tracking: 120, color: ink }, 'start', { role: 'subhead' });
  b.text('Fri – Sun only. Online & in store.', 584, 705, 420, { family: 'Inter Tight', weight: 500, size: 28, color: ink }, 'start', { lh: 1.35 });
  b.rect(584, 900, 408, 92, ink, { r: 46 });
  b.text('Code: WEEKEND40', 584, 926, 408, { family: 'Inter Tight', weight: 700, size: 32, color: mustard, tracking: 40 }, 'center', { role: 'cta' });
});

// ---- Quote over landscape photo
tpl('sa-ig-quote-landscape', 'Mountain Quote', POST, 'ig-post', ['quote', 'inspiration', 'nature', 'motivation', 'photo'], (b) => {
  pic(b, 'mountains', 0, 0, 1080, 1080, { filter: 'matte', filterAmount: 0.6 });
  b.rect(0, 0, 1080, 1080, lin(90, '#0d141a33', '#0d141acc'));
  b.text('“', 88, 250, 904, { family: 'Instrument Serif', size: 220, color: '#f3e9d8' }, 'center', { lh: 0.8 });
  b.text('The mountains don’t ask you to be fast. They ask you to keep going.', 140, 420, 800, { family: 'Instrument Serif', italic: true, size: 80, color: '#ffffff' }, 'center', { lh: 1.12, role: 'quote' });
  b.line(490, 800, 100, '#f3e9d8', 3);
  b.text('NORA LINDQVIST · TRAIL NOTES', 88, 840, 904, { family: 'Inter Tight', weight: 600, size: 26, tracking: 220, color: '#f3e9d8' }, 'center', { role: 'caption' });
});

// ---- Product launch: skincare (pastel, arch frame)
tpl('sa-ig-product-serum', 'Serum Product Launch', POST, 'ig-post', ['product', 'beauty', 'skincare', 'launch', 'pastel'], (b) => {
  const bg = '#f6e4dc', ink = '#3f2a2b', rose = '#c86f6a', card = '#fff6f1';
  b.bg(bg);
  b.rect(536, 112, 432, 880, '#efcfc4', { r: 216 });
  pic(b, 'skincare', 560, 88, 432, 880, { r: 216, focus: [0.35, 0.5], filter: 'blush', filterAmount: 0.4 });
  b.text('NEW IN', 88, 120, 400, { family: 'Tenor Sans', size: 26, tracking: 300, color: '#a24e49' }, 'start', { role: 'subhead' });
  b.text('Dew\nDrop\nSerum', 88, 180, 460, { family: 'Fraunces', weight: 300, size: 118, color: ink, italic: true }, 'start', { lh: 0.98, role: 'headline' });
  b.text('Hyaluronic acid + peptides for skin that looks rested, even when you aren’t.', 88, 580, 430, { family: 'DM Sans', size: 28, color: ink }, 'start', { lh: 1.45, role: 'body' });
  ['Fragrance-free', 'Vegan', '30 ml'].forEach((t, i) => {
    const w = [200, 106, 100][i]!, x = [88, 300, 418][i]!;
    b.rect(x, 760, w, 56, card, { r: 28 });
    b.text(t, x, 774, w, { family: 'DM Sans', weight: 600, size: 22, color: ink }, 'center');
  });
  b.text('$34', 88, 880, 300, { family: 'Fraunces', weight: 500, size: 72, color: ink }, 'start', { lh: 1.0 });
  b.text('softday.example', 250, 905, 280, { family: 'DM Sans', weight: 600, size: 24, color: '#8e5b57' }, 'start', { role: 'logo' });
});

// ---- Real estate: just listed
tpl('sa-ig-just-listed', 'Just Listed Home', POST, 'ig-post', ['real estate', 'property', 'listing', 'home', 'realtor'], (b) => {
  const navy = '#14213d', gold = '#c9a227', white = '#ffffff', grey = '#5c6477';
  b.bg(white);
  pic(b, 'house', 0, 0, 1080, 660, { filter: 'crisp', filterAmount: 0.5 });
  b.rect(88, 600, 300, 120, navy);
  b.text('JUST\nLISTED', 88, 612, 300, { family: 'Tenor Sans', size: 40, tracking: 200, color: white }, 'center', { lh: 1.15, role: 'headline' });
  b.text('$685,000', 440, 700, 552, { family: 'Playfair Display', weight: 700, size: 76, color: navy }, 'end', { lh: 1.0 });
  b.text('214 Willow Creek Lane, Ashford', 88, 790, 904, { family: 'Manrope', weight: 700, size: 38, color: navy }, 'start', { role: 'subhead' });
  b.text('4 BEDS   ·   3 BATHS   ·   2,450 SQ FT   ·   0.4 ACRE', 88, 850, 904, { family: 'Manrope', weight: 600, size: 26, tracking: 80, color: grey });
  b.line(88, 930, 904, '#e3e6ee', 2);
  b.text('Open house Sat 1–4 PM', 88, 965, 520, { family: 'Manrope', weight: 700, size: 30, color: '#8a6d12' }, 'start', { role: 'cta' });
  b.text('Avery Lane Realty · 555-0147', 540, 967, 452, { family: 'Manrope', weight: 500, size: 26, color: grey }, 'end', { role: 'logo' });
});

// ---- Food: bowl of the week
tpl('sa-ig-food-special', 'Bowl of the Week', POST, 'ig-post', ['food', 'restaurant', 'special', 'menu', 'healthy'], (b) => {
  const bg = '#1f3b2c', cream = '#f7efdc', saffron = '#f2a541';
  b.bg(bg);
  b.circle(860, 560, 380, '#2a4d3a');
  pic(b, 'food', 540, 240, 640, 640, { clip: 'ellipse', filter: 'vivid', filterAmount: 0.4 });
  b.shape('seal', 500, 170, 190, 190, saffron, { params: { points: 16 } });
  b.text('$14', 500, 228, 190, { family: 'Gloock', size: 64, color: bg }, 'center', { lh: 1.0 });
  b.text('BOWL OF\nTHE WEEK', 88, 110, 600, { family: 'Inter Tight', weight: 800, size: 30, tracking: 240, color: saffron }, 'start', { lh: 1.3, role: 'subhead' });
  b.text('Green\nGoddess', 88, 380, 460, { family: 'Gloock', size: 88, color: cream }, 'start', { lh: 1.0, role: 'headline' });
  b.text('Charred broccoli, herbed farro, soft egg, pickled shallot & tahini.', 88, 610, 380, { family: 'Inter Tight', size: 28, color: '#d7dccb' }, 'start', { lh: 1.45, role: 'body' });
  b.text('Mon–Fri lunch · while it lasts', 88, 920, 600, { family: 'Inter Tight', weight: 600, size: 28, color: cream }, 'start');
  b.text('@verdekitchen', 88, 965, 600, { family: 'Inter Tight', weight: 600, size: 26, color: saffron }, 'start', { role: 'logo' });
});

// ---- Fitness class (duotone, high energy)
tpl('sa-ig-fitness-hiit', 'HIIT Class Promo', POST, 'ig-post', ['fitness', 'gym', 'workout', 'class', 'sport'], (b) => {
  const black = '#0a0a0a', volt = '#d7ff3a', white = '#ffffff';
  b.bg(black);
  pic(b, 'gym', 0, 0, 1080, 1080, { duotone: [black, '#7d8f3a'], focus: [0.5, 0.4] });
  b.rect(0, 560, 1080, 520, lin(90, '#0a0a0a00', '#0a0a0af2'));
  b.text('IRONHOUSE GYM', 88, 88, 600, { family: 'Archivo', weight: 800, size: 28, tracking: 240, color: volt }, 'start', { role: 'logo' });
  b.text('45 MIN', 792, 88, 200, { family: 'Archivo', weight: 800, size: 28, tracking: 120, color: white }, 'end');
  b.text('SWEAT\nSMARTER.', 80, 560, 920, { family: 'Anton', size: 170, color: white }, 'start', { lh: 0.95, role: 'headline' });
  b.text('New HIIT Circuit · Tue & Thu 6:30 AM', 88, 910, 640, { family: 'Archivo', weight: 600, size: 32, color: white }, 'start', { role: 'subhead' });
  b.text('First class free', 88, 960, 640, { family: 'Archivo', weight: 500, size: 28, color: '#c9c9c9' });
  b.rect(752, 920, 240, 80, volt, { r: 40 });
  b.text('Book now', 752, 942, 240, { family: 'Archivo', weight: 800, size: 30, color: black }, 'center', { role: 'cta' });
});

// =================================================================== Photo-led portrait posts (1080 × 1350)

// ---- Editorial beauty cover
tpl('sa-igp-editorial-cover', 'Editorial Beauty Cover', POST, 'ig-portrait', ['beauty', 'magazine', 'editorial', 'fashion', 'elegant'], (b) => {
  const white = '#ffffff', blush = '#f6d6cc';
  pic(b, 'beauty', 0, 0, 1080, 1350, { focus: [0.5, 0.3], filter: 'pastel', filterAmount: 0.4 });
  b.rect(0, 0, 1080, 420, lin(90, '#1a101099', '#1a101000'));
  b.rect(0, 900, 1080, 450, lin(90, '#1a101000', '#1a1010cc'));
  b.text('LUMEN', 60, 70, 960, { family: 'Bodoni Moda', weight: 700, size: 220, color: white, tracking: 80 }, 'center', { lh: 1.0, role: 'logo' });
  b.text('ISSUE 07 · WINTER 2026', 60, 320, 960, { family: 'Tenor Sans', size: 24, tracking: 300, color: white }, 'center');
  b.text('The Glow\nIssue', 88, 1000, 600, { family: 'Bodoni Moda', italic: true, size: 96, color: white }, 'start', { lh: 1.0, role: 'headline' });
  b.text('12 rituals for\nskin that thrives\nin the cold', 700, 1060, 292, { family: 'Tenor Sans', size: 28, color: blush }, 'end', { lh: 1.4, role: 'subhead' });
});

// ---- Travel guide
tpl('sa-igp-travel-guide', 'Weekend Travel Guide', POST, 'ig-portrait', ['travel', 'guide', 'itinerary', 'vacation', 'tips'], (b) => {
  const bg = '#eef3f6', ink = '#0f2a3f', sea = '#1f7a8c', sand = '#f1c27d';
  b.bg(bg);
  pic(b, 'coast', 88, 88, 904, 620, { r: 36, filter: 'summer', filterAmount: 0.5 });
  b.rect(120, 120, 250, 64, '#ffffff', { r: 32 });
  b.text('TRAVEL GUIDE', 120, 137, 250, { family: 'Outfit', weight: 700, size: 22, tracking: 160, color: sea }, 'center');
  b.text('48 hours on the coast', 88, 750, 904, { family: 'DM Serif Display', size: 78, color: ink }, 'start', { lh: 1.05, role: 'headline' });
  const days = [['FRI', 'Sunset swim, grilled sardines at the harbour'], ['SAT', 'Cliff walk to the lighthouse, market lunch'], ['SUN', 'Slow breakfast, one last dip, train home']];
  days.forEach(([d, t], i) => {
    const y = 890 + i * 100;
    b.rect(88, y, 96, 64, i === 1 ? sand : '#ffffff', { r: 16 });
    b.text(d!, 88, y + 16, 96, { family: 'Outfit', weight: 700, size: 26, color: ink }, 'center');
    b.text(t!, 210, y + 14, 782, { family: 'Outfit', size: 32, color: ink });
  });
  b.text('Full guide → wanderlog.example', 88, 1220, 904, { family: 'Outfit', weight: 600, size: 30, color: sea }, 'start', { role: 'cta' });
});

// =================================================================== Photo-led stories (1080 × 1920)

// ---- Photo-led fashion story
tpl('sa-story-photo-lookbook', 'Lookbook Photo Story', STORY, 'ig-story', ['fashion', 'lookbook', 'photo', 'new collection', 'minimal'], (b) => {
  const white = '#ffffff';
  pic(b, 'woman', 0, 0, 1080, 1920, { focus: [0.5, 0.4], filter: 'matte', filterAmount: 0.5 });
  b.rect(0, 820, 1080, 1100, lin(90, '#00000000', '#000000b3', '#000000e6'));
  b.text('MARLOWE & CO.', 0, 270, 1080, { family: 'Syne', weight: 700, size: 30, tracking: 300, color: white }, 'center', { role: 'logo' });
  b.text('the soft\ntailoring edit', 88, 1180, 904, { family: 'Instrument Serif', italic: true, size: 120, color: white }, 'start', { lh: 0.98, role: 'headline' });
  b.text('Look 04 — wool overshirt & wide trousers', 88, 1440, 904, { family: 'Inter Tight', weight: 500, size: 34, color: '#e8e8e8' }, 'start');
  b.rect(88, 1510, 340, 84, white, { r: 42 });
  b.text('Shop the look', 88, 1533, 340, { family: 'Inter Tight', weight: 700, size: 32, color: '#111111' }, 'center', { role: 'cta' });
});

// ---- Travel diary (polaroids)
tpl('sa-story-travel-diary', 'Travel Diary Story', STORY, 'ig-story', ['travel', 'diary', 'vacation', 'scrapbook', 'photo'], (b) => {
  const paper = '#efe6d6', ink = '#2d2a26', red = '#c0452b';
  b.bg(paper);
  b.text('day 03', 88, 250, 600, { family: 'Caveat', weight: 700, size: 80, color: red });
  b.text('Lost in the\nold town', 88, 340, 904, { family: 'DM Serif Display', size: 110, color: ink }, 'start', { lh: 1.0, role: 'headline' });
  b.rect(130, 640, 560, 660, '#ffffff', { rot: -5, shadow: [0, 12, 30, '#00000026'] });
  pic(b, 'city', 162, 672, 496, 520, { rot: -5 });
  b.text('morning light', 140, 1215, 330, { family: 'Caveat', weight: 600, size: 44, color: ink }, 'center', { rot: -5 });
  b.rect(470, 1020, 500, 600, '#ffffff', { rot: 6, shadow: [0, 12, 30, '#00000026'] });
  pic(b, 'travel2', 500, 1050, 440, 460, { rot: 6 });
  b.text('best swim ever', 490, 1520, 460, { family: 'Caveat', weight: 600, size: 44, color: ink }, 'center', { rot: 6 });
  b.rect(560, 990, 150, 46, '#f1c27d', { rot: -8, opacity: 0.85 });
  b.rect(240, 615, 150, 46, '#9fc5c9', { rot: 4, opacity: 0.85 });
  b.text('more on the blog · link in bio', 88, 1660, 904, { family: 'Caveat', weight: 600, size: 40, color: '#6b6359' }, 'start');
});

// ---- Behind the scenes (film strip)
tpl('sa-story-behind-scenes', 'Behind the Scenes Story', STORY, 'ig-story', ['behind the scenes', 'studio', 'creative', 'process', 'bts'], (b) => {
  const bg = '#141414', amber = '#ffb547', white = '#f2f2f2', grey = '#8c8c8c';
  b.bg(bg);
  b.text('● REC', 88, 260, 300, { family: 'Space Grotesk', weight: 700, size: 32, color: '#ff4d4d' });
  b.text('BEHIND\nTHE SCENES', 88, 320, 904, { family: 'Bebas Neue', size: 170, color: white }, 'start', { lh: 0.9, role: 'headline' });
  b.rect(0, 680, 1080, 760, '#000000');
  for (let i = 0; i < 14; i++) { b.rect(20 + i * 76, 700, 40, 26, '#3a3a3a', { r: 5 }); b.rect(20 + i * 76, 1394, 40, 26, '#3a3a3a', { r: 5 }); }
  pic(b, 'studio', 30, 750, 500, 620, { filter: 'retro', filterAmount: 0.6 });
  pic(b, 'studio2', 550, 750, 500, 620, { filter: 'retro', filterAmount: 0.6 });
  b.text('Shooting the spring catalogue at our tiny Leeds studio — 2 days, 64 looks, a lot of coffee.', 88, 1480, 904, { family: 'Space Grotesk', size: 36, color: white }, 'start', { lh: 1.4, role: 'body' });
  b.text('FILM 02 / 03', 88, 1640, 600, { family: 'Space Grotesk', weight: 700, size: 26, tracking: 200, color: amber });
  b.text('@paperkite.studio', 492, 1640, 500, { family: 'Space Grotesk', weight: 500, size: 26, color: grey }, 'end', { role: 'logo' });
});

// ---- Link / CTA story (new blog post)
tpl('sa-story-link-blog', 'New Blog Post Story', STORY, 'ig-story', ['link', 'blog', 'cta', 'announcement', 'workspace'], (b) => {
  const bg = '#e9eefc', ink = '#151b33', blue = '#3355ff', white = '#ffffff';
  b.bg(bg);
  pic(b, 'laptop', 0, 0, 1080, 1060, { filter: 'cool', filterAmount: 0.4 });
  b.rect(80, 820, 920, 820, white, { r: 48, shadow: [0, 20, 60, '#151b3333'] });
  b.rect(140, 880, 260, 60, '#e3e8ff', { r: 30 });
  b.text('NEW ON THE BLOG', 140, 896, 260, { family: 'Onest', weight: 700, size: 22, tracking: 120, color: blue }, 'center', { role: 'subhead' });
  b.text('How we cut our meeting hours in half (and shipped more)', 140, 980, 800, { family: 'Onest', weight: 800, size: 66, color: ink }, 'start', { lh: 1.1, role: 'headline' });
  b.text('A 6-minute read on async updates, written decisions and the one meeting we kept.', 140, 1230, 800, { family: 'Onest', size: 34, color: '#4a5272' }, 'start', { lh: 1.4, role: 'body' });
  b.rect(140, 1420, 520, 104, blue, { r: 52 });
  b.text('↗  Read the post', 140, 1452, 520, { family: 'Onest', weight: 700, size: 38, color: white }, 'center', { role: 'cta' });
  b.text('tidework.example/blog', 0, 1690, 1080, { family: 'Onest', weight: 600, size: 30, color: '#4a5272' }, 'center', { role: 'logo' });
});
