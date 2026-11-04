/**
 * Brand kit guidelines from Markdown files (a brand book or design system README): each level-2 heading starts a
 * section; text before the first one becomes a section named after the file's title. Headings inside code fences
 * don't count.
 */
import type { GuidelineSection } from '../api';

export const GUIDELINE_LIMITS = { sections: 60, title: 120, body: 60_000, total: 300_000 } as const;

const clean = (s: string) => s.replace(/^\s*\n/, '').replace(/\s+$/, '');
const fileTitle = (name: string) => name.replace(/\.(md|markdown|txt)$/i, '').replace(/[-_]+/g, ' ').trim() || 'Guidelines';

export function splitMarkdown(fileName: string, text: string): GuidelineSection[] {
  const lines = text.replace(/\r\n?/g, '\n').split('\n');
  let title = '';
  let fence = false;
  const out: Array<{ title: string; lines: string[] }> = [{ title: '', lines: [] }];
  for (const line of lines) {
    if (/^\s*(```|~~~)/.test(line)) fence = !fence;
    const h1 = !fence && !title && /^#\s+(.+?)\s*#*\s*$/.exec(line);
    if (h1) {
      title = h1[1]!;
      continue;
    }
    const h2 = !fence && /^##\s+(.+?)\s*#*\s*$/.exec(line);
    if (h2) out.push({ title: h2[1]!, lines: [] });
    else out[out.length - 1]!.lines.push(line);
  }
  const intro = out[0]!;
  intro.title = title || fileTitle(fileName);
  return out
    .map((s) => ({ title: s.title.slice(0, GUIDELINE_LIMITS.title), body: clean(s.lines.join('\n')).slice(0, GUIDELINE_LIMITS.body) }))
    .filter((s, i) => i > 0 || s.body);
}

/** Imported sections replace existing ones with the same title (case-insensitive) and add the rest at the end. */
export function mergeSections(current: GuidelineSection[], incoming: GuidelineSection[]): GuidelineSection[] {
  const next = current.map((s) => ({ ...s }));
  for (const s of incoming) {
    const i = next.findIndex((x) => x.title.trim().toLowerCase() === s.title.trim().toLowerCase());
    if (i >= 0) next[i] = s;
    else next.push(s);
  }
  return next;
}

export const totalChars = (sections: GuidelineSection[]) => sections.reduce((n, s) => n + [...s.body].length, 0);
