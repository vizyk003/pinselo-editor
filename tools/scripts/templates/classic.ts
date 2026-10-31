/** The original starter set (shapes + type). */
import { B, lin, rad, solid, tpl } from './builder';

void B;

// ------------------------------------------------------------------ Instagram posts
tpl('ig-sale-violet', 'Big Summer Sale', 'Instagram Posts', 'ig-post', ['sale', 'promo', 'summer', 'retail'], (b) => {
  b.bg(lin(135, '#4c1d95', '#7c3aed', '#db2777'));
  b.shape('ellipse', 640, -180, 620, 620, '#f472b6', { opacity: 0.35 });
  b.shape('ellipse', -200, 700, 560, 560, '#a78bfa', { opacity: 0.3 });
  b.text('SUMMER', 90, 230, 900, { family: 'Bebas Neue', size: 210, color: '#ffffff', tracking: 40 }, 'start', { role: 'headline' });
  b.text('SALE', 90, 420, 900, { family: 'Bebas Neue', size: 300, color: '#fde68a', tracking: 40 }, 'start');
  b.rect(90, 760, 420, 110, '#ffffff', { r: 55 });
  b.text('UP TO 50% OFF', 90, 790, 420, { family: 'Montserrat', weight: 800, size: 40, color: '#4c1d95' }, 'center', { role: 'cta' });
  b.text('Ends Sunday · shop.example.com', 90, 920, 900, { family: 'Inter', size: 30, color: '#ede9fe' });
});
tpl('ig-quote-minimal', 'Minimal Quote', 'Instagram Posts', 'ig-post', ['quote', 'minimal', 'motivation'], (b) => {
  b.bg(solid('#f5f1ea'));
  b.text('“', 90, 120, 300, { family: 'Playfair Display', size: 280, color: '#c2410c' });
  b.text('Simplicity is the ultimate sophistication.', 140, 380, 800, { family: 'Playfair Display', size: 76, italic: true, color: '#1c1917' }, 'start', { lh: 1.15, role: 'quote' });
  b.line(140, 800, 160, '#c2410c', 5);
  b.text('LEONARDO DA VINCI', 140, 840, 800, { family: 'Montserrat', weight: 600, size: 28, tracking: 200, color: '#57534e' });
});
tpl('ig-product-launch', 'Product Launch', 'Instagram Posts', 'ig-post', ['launch', 'product', 'tech', 'new'], (b) => {
  b.bg(rad('#1e293b', '#020617'));
  b.shape('ellipse', 290, 200, 500, 500, lin(90, '#22d3ee', '#6366f1'));
  b.shape('ellipse', 340, 250, 400, 400, '#020617');
  b.text('NEW', 340, 400, 400, { family: 'Space Grotesk', weight: 700, size: 110, color: '#ffffff' }, 'center');
  b.text('Introducing Nova', 90, 790, 900, { family: 'Space Grotesk', weight: 700, size: 72, color: '#ffffff' }, 'center', { role: 'headline' });
  b.text('Faster. Lighter. Yours.', 90, 890, 900, { family: 'Inter', size: 36, color: '#94a3b8' }, 'center');
});
tpl('ig-recipe-card', 'Recipe Card', 'Instagram Posts', 'ig-post', ['food', 'recipe', 'cooking'], (b) => {
  b.bg(solid('#fef3c7'));
  b.rect(60, 60, 960, 960, '#ffffff', { r: 40 });
  b.shape('ellipse', 700, 120, 260, 260, '#f59e0b');
  b.shape('ellipse', 760, 180, 140, 140, '#fde68a');
  b.text('Lemon\nPasta', 120, 140, 560, { family: 'DM Serif Display', size: 120, color: '#78350f' }, 'start', { lh: 1.0, role: 'headline' });
  b.text('15 MIN · SERVES 2', 120, 440, 700, { family: 'Montserrat', weight: 700, size: 28, tracking: 150, color: '#b45309' });
  b.text('• 200g spaghetti\n• 1 lemon, zested\n• 40g parmesan\n• Black pepper & olive oil', 120, 520, 820, { family: 'Lora', size: 40, color: '#44403c' }, 'start', { lh: 1.5 });
});
tpl('ig-event-neon', 'Neon Night Event', 'Instagram Posts', 'ig-post', ['party', 'event', 'music', 'night'], (b) => {
  b.bg(solid('#09090b'));
  b.shape('star', 760, 80, 220, 220, '#facc15', { rot: 12 });
  b.text('FRIDAY', 80, 260, 920, { family: 'Anton', size: 180, color: '#f0abfc' }, 'start', { role: 'headline' });
  b.text('NIGHT LIVE', 80, 470, 920, { family: 'Anton', size: 150, color: '#22d3ee' });
  b.rect(80, 720, 920, 4, '#f0abfc');
  b.text('DOORS 9PM · THE WAREHOUSE', 80, 760, 920, { family: 'Space Grotesk', weight: 700, size: 40, color: '#ffffff', tracking: 80 });
  b.text('Tickets at the link in bio', 80, 840, 920, { family: 'Inter', size: 32, color: '#a1a1aa' });
});

// ------------------------------------------------------------------ Stories
tpl('story-gradient-announcement', 'Announcement Story', 'Instagram Stories', 'ig-story', ['announcement', 'story', 'gradient'], (b) => {
  b.bg(lin(160, '#f97316', '#ec4899', '#8b5cf6'));
  b.shape('ellipse', -200, -200, 700, 700, '#ffffff', { opacity: 0.12 });
  b.shape('ellipse', 600, 1400, 700, 700, '#ffffff', { opacity: 0.12 });
  b.text('BIG\nNEWS', 100, 560, 880, { family: 'Bebas Neue', size: 300, color: '#ffffff' }, 'center', { lh: 0.9, role: 'headline' });
  b.text('We’re opening a second store downtown this spring.', 140, 1150, 800, { family: 'Poppins', weight: 500, size: 52, color: '#ffffff' }, 'center', { lh: 1.3 });
  b.rect(340, 1560, 400, 110, '#ffffff', { r: 55 });
  b.text('Learn more', 340, 1590, 400, { family: 'Poppins', weight: 700, size: 40, color: '#db2777' }, 'center', { role: 'cta' });
});
tpl('story-poll', 'This or That Story', 'Instagram Stories', 'ig-story', ['poll', 'engagement', 'story'], (b) => {
  b.bg(solid('#ecfeff'));
  b.text('THIS or THAT?', 80, 220, 920, { family: 'Archivo Black', size: 110, color: '#0e7490' }, 'center', { role: 'headline' });
  b.rect(100, 520, 400, 600, '#22d3ee', { r: 40 });
  b.rect(580, 520, 400, 600, '#f472b6', { r: 40 });
  b.text('Coffee', 100, 760, 400, { family: 'Poppins', weight: 700, size: 70, color: '#ffffff' }, 'center');
  b.text('Tea', 580, 760, 400, { family: 'Poppins', weight: 700, size: 70, color: '#ffffff' }, 'center');
  b.text('Vote in the poll ↓', 80, 1300, 920, { family: 'Inter', size: 48, color: '#155e75' }, 'center');
});

// ------------------------------------------------------------------ Presentations
tpl('deck-pitch-modern', 'Modern Pitch Deck', 'Presentations', 'presentation', ['pitch', 'startup', 'business', 'deck'], (b) => {
  b.page(0).bg(solid('#0f172a'));
  b.shape('ellipse', 1200, -300, 1000, 1000, lin(45, '#6366f1', '#22d3ee'), { opacity: 0.85 });
  b.text('Acme Robotics', 140, 380, 1100, { family: 'Space Grotesk', weight: 700, size: 130, color: '#ffffff' }, 'start', { role: 'headline' });
  b.text('Series A · 2026', 140, 560, 900, { family: 'Inter', size: 48, color: '#94a3b8' });
  b.page(1).bg(solid('#ffffff'));
  b.text('The problem', 140, 120, 1600, { family: 'Space Grotesk', weight: 700, size: 90, color: '#0f172a' }, 'start', { role: 'headline' });
  b.rect(140, 300, 500, 560, '#eef2ff', { r: 32 });
  b.rect(710, 300, 500, 560, '#ecfeff', { r: 32 });
  b.rect(1280, 300, 500, 560, '#fdf2f8', { r: 32 });
  b.text('68%', 180, 360, 420, { family: 'Space Grotesk', weight: 700, size: 110, color: '#4f46e5' });
  b.text('of warehouses still pick by hand', 180, 520, 420, { family: 'Inter', size: 40, color: '#334155' }, 'start', { lh: 1.3 });
  b.text('3×', 750, 360, 420, { family: 'Space Grotesk', weight: 700, size: 110, color: '#0891b2' });
  b.text('labour cost growth since 2020', 750, 520, 420, { family: 'Inter', size: 40, color: '#334155' }, 'start', { lh: 1.3 });
  b.text('1 in 4', 1320, 360, 420, { family: 'Space Grotesk', weight: 700, size: 110, color: '#db2777' });
  b.text('orders ship late at peak', 1320, 520, 420, { family: 'Inter', size: 40, color: '#334155' }, 'start', { lh: 1.3 });
  b.page(2).bg(lin(135, '#4f46e5', '#0891b2'));
  b.text('Thank you', 140, 400, 1640, { family: 'Space Grotesk', weight: 700, size: 150, color: '#ffffff' }, 'center', { role: 'headline' });
  b.text('hello@acme.example', 140, 620, 1640, { family: 'Inter', size: 48, color: '#e0f2fe' }, 'center');
}, 3);
tpl('deck-education-warm', 'Lesson Slides', 'Presentations', 'presentation', ['education', 'lesson', 'school', 'class'], (b) => {
  b.page(0).bg(solid('#fff7ed'));
  b.shape('ellipse', -200, 600, 700, 700, '#fdba74', { opacity: 0.6 });
  b.shape('star', 1500, 120, 240, 240, '#facc15', { rot: -10 });
  b.text('The Water Cycle', 200, 340, 1500, { family: 'Poppins', weight: 800, size: 140, color: '#9a3412' }, 'start', { role: 'headline' });
  b.text('Science · Grade 5 · Unit 3', 200, 540, 1200, { family: 'Nunito', size: 54, color: '#c2410c' });
  b.page(1).bg(solid('#ffffff'));
  b.text('Four stages', 160, 110, 1600, { family: 'Poppins', weight: 800, size: 90, color: '#9a3412' });
  const stages = [['Evaporation', '#0ea5e9'], ['Condensation', '#6366f1'], ['Precipitation', '#14b8a6'], ['Collection', '#f59e0b']] as const;
  stages.forEach(([s, c], i) => {
    b.shape('ellipse', 180 + i * 420, 360, 260, 260, c);
    b.text(String(i + 1), 180 + i * 420, 430, 260, { family: 'Poppins', weight: 800, size: 100, color: '#ffffff' }, 'center');
    b.text(s, 120 + i * 420, 680, 380, { family: 'Nunito', weight: 700, size: 46, color: '#1f2937' }, 'center');
  });
}, 2);
tpl('deck-minimal-mono', 'Minimal Report', 'Presentations', 'presentation', ['report', 'minimal', 'corporate'], (b) => {
  b.page(0).bg(solid('#fafafa'));
  b.rect(0, 0, 24, 1080, '#111827');
  b.text('Quarterly\nReview', 160, 300, 1200, { family: 'Inter', weight: 800, size: 150, color: '#111827' }, 'start', { lh: 1.0, role: 'headline' });
  b.text('Q3 2026 · Operations', 160, 680, 1200, { family: 'Inter', size: 48, color: '#6b7280' });
  b.page(1).bg(solid('#fafafa'));
  b.text('Highlights', 160, 120, 1600, { family: 'Inter', weight: 800, size: 90, color: '#111827' });
  b.text('• Revenue up 18% quarter over quarter\n• Churn down to 2.1%\n• Two new regions launched\n• NPS 61 (+7)', 160, 300, 1600, { family: 'Inter', size: 56, color: '#374151' }, 'start', { lh: 1.6 });
}, 2);

// ------------------------------------------------------------------ Posters & flyers
tpl('poster-concert', 'Concert Poster', 'Posters', 'poster', ['music', 'concert', 'festival', 'poster'], (b) => {
  b.bg(lin(180, '#1e1b4b', '#831843'));
  b.shape('ellipse', 264, 300, 1200, 1200, rad('#fb7185', '#831843'), { opacity: 0.9 });
  b.shape('ellipse', 564, 600, 600, 600, '#1e1b4b');
  b.text('ECHOES', 100, 1600, 1528, { family: 'Anton', size: 360, color: '#fce7f3', tracking: 60 }, 'center', { role: 'headline' });
  b.text('LIVE AT THE ARENA · 21 NOVEMBER', 100, 2050, 1528, { family: 'Space Grotesk', weight: 700, size: 64, color: '#fbcfe8', tracking: 120 }, 'center');
});
tpl('poster-farmers-market', 'Farmers Market', 'Posters', 'poster', ['market', 'community', 'food', 'local'], (b) => {
  b.bg(solid('#f0fdf4'));
  b.rect(0, 0, 1728, 900, '#166534');
  b.shape('ellipse', 1150, 160, 420, 420, '#facc15');
  b.text('Saturday\nFarmers\nMarket', 120, 120, 1100, { family: 'DM Serif Display', size: 210, color: '#f0fdf4' }, 'start', { lh: 0.95, role: 'headline' });
  b.text('Fresh produce · Local makers · Live music', 120, 1040, 1500, { family: 'Lora', size: 70, italic: true, color: '#166534' });
  b.text('Every Saturday 8am – 1pm\nTown Square, Main Street', 120, 1300, 1500, { family: 'Montserrat', weight: 700, size: 76, color: '#14532d' }, 'start', { lh: 1.4 });
  b.qr('https://example.com/market', 1320, 1900, 300);
});
tpl('flyer-yoga', 'Yoga Class Flyer', 'Flyers', 'flyer', ['yoga', 'wellness', 'fitness', 'class'], (b) => {
  b.bg(solid('#fdf4ff'));
  b.shape('ellipse', 260, 80, 300, 300, lin(90, '#f0abfc', '#c4b5fd'));
  b.text('Morning Flow', 20, 420, 520, { family: 'Great Vibes', size: 68, color: '#86198f' }, 'center', { role: 'headline' });
  b.text('YOGA FOR EVERY BODY', 40, 540, 480, { family: 'Montserrat', weight: 700, size: 22, tracking: 200, color: '#701a75' }, 'center');
  b.text('Tues & Thurs · 7:00am\nStudio 4, Riverside Centre', 40, 610, 480, { family: 'Lato', size: 24, color: '#4a044e' }, 'center', { lh: 1.5 });
});
tpl('flyer-lost-cat', 'Lost Pet Flyer', 'Flyers', 'flyer', ['lost', 'pet', 'community', 'notice'], (b) => {
  b.bg(solid('#ffffff'));
  b.rect(0, 0, 559, 150, '#dc2626');
  b.text('LOST CAT', 0, 30, 559, { family: 'Anton', size: 90, color: '#ffffff' }, 'center', { role: 'headline' });
  b.rect(60, 190, 439, 330, '#e5e7eb', { r: 12 });
  b.text('[ add a photo ]', 60, 330, 439, { family: 'Inter', size: 22, color: '#6b7280' }, 'center');
  b.text('“Biscuit” · orange tabby · shy', 30, 550, 499, { family: 'Inter', weight: 700, size: 28, color: '#111827' }, 'center');
  b.text('Call 555-0134', 30, 640, 499, { family: 'Inter', weight: 800, size: 44, color: '#dc2626' }, 'center', { role: 'cta' });
});

// ------------------------------------------------------------------ Social & video
tpl('yt-thumb-bold', 'Bold YouTube Thumbnail', 'YouTube Thumbnails', 'yt-thumb', ['youtube', 'thumbnail', 'video', 'tutorial'], (b) => {
  b.bg(lin(0, '#fde047', '#f97316'));
  b.shape('ellipse', 780, 60, 600, 600, '#ffffff', { opacity: 0.25 });
  b.text('I TRIED IT\nFOR 30 DAYS', 60, 140, 800, { family: 'Anton', size: 150, color: '#111827' }, 'start', { lh: 0.95, role: 'headline' });
  b.shape('arrow', 860, 300, 260, 140, '#dc2626', { rot: -20 });
  b.rect(60, 560, 360, 90, '#dc2626', { r: 12 });
  b.text('RESULTS', 60, 572, 360, { family: 'Anton', size: 60, color: '#ffffff' }, 'center');
});
tpl('fb-post-hiring', 'We’re Hiring', 'Facebook Posts', 'fb-post', ['hiring', 'jobs', 'career', 'business'], (b) => {
  b.bg(solid('#eef2ff'));
  b.rect(0, 0, 420, 630, '#4f46e5');
  b.text('WE’RE\nHIRING', 40, 180, 360, { family: 'Archivo Black', size: 78, color: '#ffffff' }, 'start', { lh: 1.0, role: 'headline' });
  b.text('Senior Product Designer', 480, 170, 680, { family: 'Poppins', weight: 700, size: 56, color: '#1e1b4b' }, 'start', { lh: 1.15 });
  b.text('Remote · Full-time · Apply by 30 Nov', 480, 330, 680, { family: 'Inter', size: 32, color: '#4338ca' });
  b.rect(480, 440, 300, 80, '#4f46e5', { r: 40 });
  b.text('Apply now', 480, 458, 300, { family: 'Poppins', weight: 700, size: 32, color: '#ffffff' }, 'center', { role: 'cta' });
});
tpl('linkedin-banner-pro', 'Professional Banner', 'Social Banners', 'linkedin-banner', ['linkedin', 'banner', 'profile'], (b) => {
  b.bg(lin(90, '#0f172a', '#1e3a8a'));
  b.shape('hexagon', 1250, -60, 300, 260, '#3b82f6', { opacity: 0.4 });
  b.shape('hexagon', 1380, 160, 240, 210, '#60a5fa', { opacity: 0.3 });
  b.text('Data Engineering · Cloud · Analytics', 520, 150, 760, { family: 'Inter', weight: 600, size: 40, color: '#e0e7ff' }, 'start', { role: 'subhead' });
});

// ------------------------------------------------------------------ Print: cards, invitations, resumes
tpl('card-business-clean', 'Clean Business Card', 'Business Cards', 'business-card', ['business card', 'contact', 'minimal'], (b) => {
  b.bg(solid('#ffffff'));
  b.rect(0, 0, 12, 192, '#7c3aed');
  b.text('Alex Morgan', 34, 34, 280, { family: 'Poppins', weight: 700, size: 22, color: '#111827' }, 'start', { role: 'headline' });
  b.text('Product Designer', 34, 64, 280, { family: 'Inter', size: 12, color: '#7c3aed' });
  b.text('alex@example.com\n+1 555 0100\nexample.com', 34, 104, 220, { family: 'Inter', size: 10, color: '#4b5563' }, 'start', { lh: 1.5 });
  b.qr('https://example.com', 262, 110, 62);
});
tpl('invite-wedding', 'Wedding Invitation', 'Invitations', 'invitation', ['wedding', 'invitation', 'elegant'], (b) => {
  b.bg(solid('#fffbf5'));
  b.text('Together with their families', 40, 90, 400, { family: 'Lora', italic: true, size: 18, color: '#78716c' }, 'center');
  b.text('Maya\n&\nJonah', 40, 150, 400, { family: 'Great Vibes', size: 72, color: '#9a3412' }, 'center', { lh: 1.05, role: 'headline' });
  b.line(190, 420, 100, '#d6b98c', 2);
  b.text('SATURDAY, 12 JUNE 2027\nFOUR O’CLOCK\nTHE OLD MILL, OAKRIDGE', 40, 450, 400, { family: 'Montserrat', weight: 500, size: 14, tracking: 150, color: '#57534e' }, 'center', { lh: 1.8 });
});
tpl('resume-modern', 'Modern Resume', 'Resumes', 'resume', ['resume', 'cv', 'job', 'career'], (b) => {
  b.bg(solid('#ffffff'));
  b.rect(0, 0, 260, 1123, '#1f2937');
  b.shape('ellipse', 55, 60, 150, 150, lin(135, '#a78bfa', '#22d3ee'));
  b.text('JORDAN\nLEE', 30, 250, 200, { family: 'Montserrat', weight: 800, size: 34, color: '#ffffff', tracking: 60 }, 'start', { lh: 1.1, role: 'headline' });
  b.text('CONTACT', 30, 380, 200, { family: 'Montserrat', weight: 700, size: 13, tracking: 200, color: '#a78bfa' });
  b.text('jordan@example.com\n+1 555 0199\nPortland, OR', 30, 410, 210, { family: 'Inter', size: 12, color: '#e5e7eb' }, 'start', { lh: 1.6 });
  b.text('SKILLS', 30, 520, 200, { family: 'Montserrat', weight: 700, size: 13, tracking: 200, color: '#a78bfa' });
  b.text('Figma · Prototyping\nDesign systems\nUser research\nHTML / CSS', 30, 550, 210, { family: 'Inter', size: 12, color: '#e5e7eb' }, 'start', { lh: 1.6 });
  b.text('Senior UX Designer', 300, 70, 460, { family: 'Montserrat', weight: 800, size: 30, color: '#111827' });
  b.text('I design calm, accessible products for complex workflows.', 300, 120, 450, { family: 'Inter', size: 14, color: '#4b5563' }, 'start', { lh: 1.5 });
  b.text('EXPERIENCE', 300, 200, 460, { family: 'Montserrat', weight: 700, size: 13, tracking: 200, color: '#7c3aed' });
  b.text('Lead Designer — Northwind (2022–now)\nShipped a design system used by 40 teams; cut onboarding time 35%.\n\nProduct Designer — Fabrikam (2018–2022)\nRedesigned the analytics suite; NPS +22.', 300, 230, 450, { family: 'Inter', size: 13, color: '#374151' }, 'start', { lh: 1.55 });
  b.text('EDUCATION', 300, 470, 460, { family: 'Montserrat', weight: 700, size: 13, tracking: 200, color: '#7c3aed' });
  b.text('BFA Interaction Design — State University, 2018', 300, 500, 450, { family: 'Inter', size: 13, color: '#374151' });
});

// ------------------------------------------------------------------ Logos & marketing
tpl('logo-badge', 'Coffee Badge Logo', 'Logos', 'logo', ['logo', 'coffee', 'badge', 'brand'], (b) => {
  b.bg(solid('#fef3c7'));
  b.shape('ellipse', 50, 50, 400, 400, '#78350f');
  b.shape('ellipse', 70, 70, 360, 360, '#fef3c7');
  b.shape('ellipse', 85, 85, 330, 330, '#78350f');
  b.text('NORTH', 85, 150, 330, { family: 'Bebas Neue', size: 70, color: '#fef3c7', tracking: 100 }, 'center', { role: 'logo' });
  b.text('ROASTERS', 85, 220, 330, { family: 'Bebas Neue', size: 46, color: '#fcd34d', tracking: 200 }, 'center');
  b.text('EST. 2026', 85, 300, 330, { family: 'Montserrat', weight: 700, size: 16, color: '#fef3c7', tracking: 300 }, 'center');
});
tpl('logo-geometric', 'Geometric Logo', 'Logos', 'logo', ['logo', 'tech', 'geometric', 'modern'], (b) => {
  b.bg(solid('#ffffff'));
  b.shape('diamond', 175, 90, 150, 150, lin(135, '#6366f1', '#ec4899'));
  b.shape('diamond', 215, 130, 70, 70, '#ffffff');
  b.text('prism', 50, 280, 400, { family: 'Space Grotesk', weight: 700, size: 80, color: '#111827' }, 'center', { role: 'logo' });
});
tpl('email-header-launch', 'Email Header', 'Email Headers', 'email-header', ['email', 'newsletter', 'header'], (b) => {
  b.bg(lin(90, '#0ea5e9', '#6366f1'));
  b.text('The Monthly Brief', 40, 50, 520, { family: 'Poppins', weight: 800, size: 46, color: '#ffffff' }, 'start', { role: 'headline' });
  b.text('October 2026 · Product news & tips', 40, 120, 520, { family: 'Inter', size: 20, color: '#e0f2fe' });
});
tpl('infographic-steps', 'Process Infographic', 'Infographics', 'infographic', ['infographic', 'process', 'steps', 'data'], (b) => {
  b.bg(solid('#f8fafc'));
  b.text('How recycling works', 60, 80, 680, { family: 'Poppins', weight: 800, size: 64, color: '#0f172a' }, 'center', { role: 'headline' });
  const steps = [['Collect', '#22c55e'], ['Sort', '#0ea5e9'], ['Clean', '#a855f7'], ['Process', '#f59e0b'], ['Reuse', '#ef4444']] as const;
  steps.forEach(([s, c], i) => {
    const y = 300 + i * 330;
    b.shape('ellipse', 100, y, 200, 200, c);
    b.text(String(i + 1), 100, y + 55, 200, { family: 'Poppins', weight: 800, size: 80, color: '#ffffff' }, 'center');
    b.text(s, 350, y + 40, 400, { family: 'Poppins', weight: 700, size: 56, color: '#0f172a' });
    b.text('A short line describing this step.', 350, y + 120, 400, { family: 'Inter', size: 28, color: '#475569' });
    if (i < steps.length - 1) b.rect(195, y + 210, 10, 110, '#cbd5e1', { r: 5 });
  });
});

