/** The cover of a video post (`options.cover`): the frame Instagram and Facebook show for it. */
import { getLocale, t } from '../i18n';

/** `'last_frame'` (the default), `'first_frame'`, or a time in the video in seconds. */
export type Cover = 'last_frame' | 'first_frame' | number;

/** Read a stored or typed cover; anything unreadable is the default. */
export function parseCover(v: unknown): Cover {
  if (v === 'first_frame' || v === 'last_frame') return v;
  const n = typeof v === 'number' ? v : typeof v === 'string' && v.trim() ? Number(v) : NaN;
  return Number.isFinite(n) && n >= 0 ? n : 'last_frame';
}

export function coverLabel(c: Cover): string {
  if (c === 'first_frame') return t('First frame');
  if (c === 'last_frame') return t('Last frame');
  return t('At {s} s', { s: c.toLocaleString(getLocale(), { maximumFractionDigits: 2 }) });
}

/** The frame the cover picks, rendered by the server as the published video draws it (`w` px wide). */
export const coverUrl = (designId: string, c: Cover, w: number, v?: string) =>
  `/api/designs/${designId}/cover.png?cover=${encodeURIComponent(String(c))}&w=${w}${v ? `&v=${encodeURIComponent(v)}` : ''}`;
