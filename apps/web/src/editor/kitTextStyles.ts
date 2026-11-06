/**
 * Brand kit text styles (pure helpers): what a style puts on new text, and a style read back from a text box. The
 * AI connector checks assistants' text against the same styles (crates/oc-api/src/mcp/brand.rs).
 */
import type { RunStyle, TextEl } from '@oc/engine';
import type { Kit, KitFontRole, KitTextStyle } from '../api';

export const MAX_TEXT_STYLES = 30;
export const WEIGHTS = [100, 200, 300, 400, 500, 600, 700, 800, 900] as const;

/** The element role a kit role stands for (and back), so new text carries the role the AI check reads. */
const ELEMENT_ROLE: Record<KitFontRole, NonNullable<TextEl['role']>> = { heading: 'headline', subheading: 'subhead', body: 'body', quote: 'quote', caption: 'caption' };
export function elementRole(role: KitFontRole): NonNullable<TextEl['role']> {
  return ELEMENT_ROLE[role];
}
export function kitRole(role: TextEl['role'] | undefined): KitFontRole | null {
  const hit = (Object.entries(ELEMENT_ROLE) as Array<[KitFontRole, string]>).find(([, r]) => r === role);
  if (hit) return hit[0];
  return role === 'legal' || role === 'price' ? 'body' : null;
}

/** The run style a kit text style gives new text: the role's font, weight, italic, capitals, spacing, size, first colour. */
export function runStyleOf(kit: Pick<Kit, 'fonts'>, st: KitTextStyle, fill?: (hex: string) => RunStyle['fill']): Partial<RunStyle> {
  const out: Partial<RunStyle> = {};
  const family = kit.fonts[st.role] ?? kit.fonts.body ?? kit.fonts.heading;
  if (family) out.family = family;
  if (st.weight) out.weight = st.weight;
  out.italic = Boolean(st.italic);
  if (st.uppercase) out.caseTransform = 'upper';
  if (st.letterSpacing) out.tracking = st.letterSpacing;
  if (st.size) out.size = st.size;
  if (st.colors?.[0] && fill) out.fill = fill(st.colors[0]);
  return out;
}

/** The first style for a role, if the kit has one. */
export function styleFor(kit: Pick<Kit, 'textStyles'>, role: KitFontRole): KitTextStyle | undefined {
  return kit.textStyles?.find((s) => s.role === role);
}

/**
 * A text style from a text box: its weight, italic, capitals, letter spacing, line height, size and colour (when it is
 * a plain colour), with the box's role, or `fallbackRole` when it has none.
 */
export function styleFromText(el: TextEl, name: string, fallbackRole: KitFontRole, toHex: (c: unknown) => string): KitTextStyle {
  const d = el.body.defaultRun;
  const p0 = el.body.paragraphs[0];
  const st: KitTextStyle = { name, role: kitRole(el.role) ?? fallbackRole };
  if (d.weight) st.weight = Math.round(d.weight / 100) * 100;
  if (d.italic) st.italic = true;
  const plain = el.body.paragraphs.flatMap((p) => p.runs.map((r) => r.text)).join('');
  if (d.caseTransform === 'upper' || (/\p{L}/u.test(plain) && plain === plain.toUpperCase() && plain !== plain.toLowerCase())) st.uppercase = true;
  if (d.tracking) st.letterSpacing = Math.round(d.tracking);
  if (p0 && p0.lineHeight && Math.abs(p0.lineHeight - 1.3) > 0.001) st.lineHeight = Math.round(p0.lineHeight * 100) / 100;
  if (d.size) st.size = Math.round(d.size * 10) / 10;
  if (d.fill && d.fill.k === 'solid') st.colors = [toHex(d.fill.color).toLowerCase().slice(0, 7)];
  return st;
}

/** A short description of a style: "700 · italic · CAPS · +140‰ · ↕1.4 · 30px" (`words` translates italic and capitals). */
export function describeStyle(st: KitTextStyle, words: { italic: string; caps: string }): string {
  const parts: string[] = [];
  if (st.weight) parts.push(String(st.weight));
  if (st.italic) parts.push(words.italic);
  if (st.uppercase) parts.push(words.caps);
  if (st.letterSpacing) parts.push(`${st.letterSpacing > 0 ? '+' : ''}${st.letterSpacing}‰`);
  if (st.lineHeight) parts.push(`↕${st.lineHeight}`);
  if (st.size) parts.push(`${st.size}px`);
  return parts.join(' · ');
}

/** A style as the API accepts it: trimmed name, only set fields, colours as #rrggbb. */
export function cleanStyle(st: KitTextStyle): KitTextStyle {
  const out: KitTextStyle = { name: st.name.trim().slice(0, 40), role: st.role };
  if (st.weight) out.weight = Math.min(900, Math.max(100, Math.round(st.weight / 100) * 100));
  if (st.italic) out.italic = true;
  if (st.uppercase) out.uppercase = true;
  if (st.letterSpacing && Number.isFinite(st.letterSpacing)) out.letterSpacing = Math.min(2000, Math.max(-500, Math.round(st.letterSpacing)));
  if (st.lineHeight && Number.isFinite(st.lineHeight)) out.lineHeight = Math.min(4, Math.max(0.5, st.lineHeight));
  if (st.size && Number.isFinite(st.size)) out.size = Math.min(1000, Math.max(4, st.size));
  const colors = [...new Set((st.colors ?? []).map((c) => c.toLowerCase()).filter((c) => /^#[0-9a-f]{6}$/.test(c)))].slice(0, 12);
  if (colors.length) out.colors = colors;
  if (st.note?.trim()) out.note = st.note.trim().slice(0, 300);
  return out;
}
