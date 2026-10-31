/** Pure helpers for the Share dialog (links, embed snippet, typed emails). Tested in tests/web-share.test.ts. */

/** Public view link: the custom slug when there is one, else the design id. */
export function viewUrl(origin: string, designId: string, slug?: string | null): string {
  return `${origin}/view/${encodeURIComponent(slug || designId)}`;
}

export const templateLinkUrl = (origin: string, token: string) => `${origin}/t/${encodeURIComponent(token)}`;

const esc = (s: string) => s.replace(/&/g, '&amp;').replace(/"/g, '&quot;').replace(/</g, '&lt;').replace(/>/g, '&gt;');

/**
 * Responsive iframe snippet for the embed-mode viewer: a box with the design's aspect ratio that fills
 * the width of wherever it's pasted.
 */
export function embedSnippet(viewLink: string, w: number, h: number, title: string): string {
  const pad = w > 0 && h > 0 ? +((h / w) * 100).toFixed(4) : 56.25;
  const src = `${viewLink}${viewLink.includes('?') ? '&' : '?'}embed=1`;
  return `<div style="position:relative;width:100%;height:0;padding-top:${pad}%;overflow:hidden">`
    + `<iframe src="${esc(src)}" title="${esc(title)}" loading="lazy" allowfullscreen style="position:absolute;inset:0;width:100%;height:100%;border:0"></iframe></div>`;
}

export const looksLikeEmail = (s: string) => /^[^\s@,;]+@[^\s@,;]+\.[^\s@,;]+$/.test(s.trim());

/** Emails typed or pasted into the Add people box ("a@x.com, b@y.org; c@z.io"); the rest is search text. */
export function splitTyped(input: string): { emails: string[]; rest: string } {
  const parts = input.split(/[\s,;]+/).filter(Boolean);
  const emails = [...new Set(parts.filter(looksLikeEmail).map((e) => e.toLowerCase()))];
  return { emails, rest: parts.filter((p) => !looksLikeEmail(p)).join(' ') };
}

/** Custom link as the user types it: the server normalizes the same way, this just previews it. */
export function previewSlug(raw: string): string {
  return raw.trim().toLowerCase().replace(/[\s_]+/g, '-').replace(/[^a-z0-9-]/g, '').replace(/-{2,}/g, '-').replace(/^-+/, '').slice(0, 60);
}
