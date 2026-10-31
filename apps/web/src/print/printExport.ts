/**
 * Download → PDF Print: the simple choices (crop marks and bleed, flatten, RGB/CMYK), the advanced ones, how they
 * map to the export request, remembered choices, and the colour-profile / gamut API clients.
 */
import { DEFAULT_PRINT_CATALOG, type ColorValue, type DesignDoc } from '@oc/engine';
import { ApiError, type PrintExportOptions } from '../api';

export type PrintColor = 'rgb' | 'cmyk';
export type PdfStandard = NonNullable<PrintExportOptions['standard']>;
export type BleedChoice = 'auto' | 'none' | '3' | '5' | 'custom';

export interface PrintPrefs {
  /** "Crop marks and bleed". */
  marks: boolean;
  /** "Flatten PDF" (PDF/X-1a when CMYK). */
  flatten: boolean;
  color: PrintColor;
  // Advanced
  bleed: BleedChoice;
  customBleedMm: number;
  registration: boolean;
  colorBars: boolean;
  /** Selected profile id ('' = the default for the colour space). */
  profile: string;
  intent: 'relative' | 'perceptual';
  standard: 'auto' | PdfStandard;
}

export const DEFAULT_CMYK_PROFILE = 'pso-coated-v3';

export function defaultPrintPrefs(printProduct: boolean): PrintPrefs {
  return {
    marks: printProduct, flatten: false, color: printProduct ? 'cmyk' : 'rgb',
    bleed: 'auto', customBleedMm: 3, registration: false, colorBars: false, profile: '', intent: 'relative', standard: 'auto',
  };
}

const MM = 96 / 25.4;

/**
 * Is this design a print product, and which bleed does that product use? Matches the design size against the
 * print catalogue (either orientation for rotatable products, 1 mm tolerance). Print-surface designs that match
 * no product still count as print, with the standard 3 mm bleed.
 */
export function printProductFor(doc: Pick<DesignDoc, 'meta'>): { printProduct: boolean; bleedMm: number } {
  const { w, h, unit } = doc.meta.dims;
  const tol = MM;
  for (const p of DEFAULT_PRINT_CATALOG.products) {
    for (const s of p.sizes) {
      const same = Math.abs(s.w - w) <= tol && Math.abs(s.h - h) <= tol;
      const turned = p.rotatable && Math.abs(s.h - w) <= tol && Math.abs(s.w - h) <= tol;
      // Pixel designs that happen to share a size are not print products (e.g. 1080 × 1080 social posts).
      if ((same || turned) && unit !== 'px') return { printProduct: true, bleedMm: p.bleedMm > 0 ? p.bleedMm : 3 };
    }
  }
  return { printProduct: doc.meta.surface === 'print', bleedMm: 3 };
}

/** Bleed in mm for the current choices (0 when "Crop marks and bleed" is off). */
export function bleedMmFor(p: PrintPrefs, autoBleedMm: number): number {
  if (!p.marks) return 0;
  switch (p.bleed) {
    case 'auto': return autoBleedMm;
    case 'none': return 0;
    case '3': return 3;
    case '5': return 5;
    case 'custom': return Math.min(10, Math.max(0, Number.isFinite(p.customBleedMm) ? p.customBleedMm : 0));
  }
}

/**
 * The PDF standard the simple choices imply: CMYK → PDF/X-4 (PDF/X-1a when flattened); RGB → no PDF/X claim
 * (flattened RGB when "Flatten PDF" is on). An explicit Advanced choice wins.
 */
export function standardFor(p: PrintPrefs): PdfStandard {
  if (p.standard !== 'auto') return p.standard;
  if (p.color === 'cmyk') return p.flatten ? 'pdfx1a' : 'pdfx4';
  return 'none';
}

export interface ProfileInfo {
  id: string; name: string; description?: string; kind: 'bundled' | 'custom'; colorSpace: 'CMYK' | 'RGB'; conditionId?: string;
  paper?: string; default?: boolean; available?: boolean; teamId?: string | null;
}

/** Bundled profiles, used until (or when) GET /api/print/profiles answers. Mirrors crates/oc-color/icc-profiles.json. */
export const FALLBACK_PROFILES: ProfileInfo[] = [
  { id: 'pso-coated-v3', name: 'PSO Coated v3', kind: 'bundled', colorSpace: 'CMYK', conditionId: 'FOGRA51', paper: 'coated', default: true },
  { id: 'pso-uncoated-v3', name: 'PSO Uncoated v3', kind: 'bundled', colorSpace: 'CMYK', conditionId: 'FOGRA52', paper: 'uncoated' },
  { id: 'iso-coated-v2', name: 'ISO Coated v2 (ECI)', kind: 'bundled', colorSpace: 'CMYK', conditionId: 'FOGRA39', paper: 'coated' },
  { id: 'srgb', name: 'sRGB IEC61966-2.1', kind: 'bundled', colorSpace: 'RGB', conditionId: 'sRGB IEC61966-2.1' },
];

/** The profile actually used for the colour space: the selected one if it fits, else the default. */
export function effectiveProfile(p: PrintPrefs, profiles: ProfileInfo[]): ProfileInfo | undefined {
  const space = p.color === 'cmyk' ? 'CMYK' : 'RGB';
  const usable = profiles.filter((x) => x.colorSpace === space && x.available !== false);
  return usable.find((x) => x.id === p.profile)
    ?? usable.find((x) => x.default)
    ?? usable.find((x) => x.id === (space === 'CMYK' ? DEFAULT_CMYK_PROFILE : 'srgb'))
    ?? usable[0];
}

export const profileRef = (x: ProfileInfo): string | { id: string } => (x.kind === 'custom' ? { id: x.id } : x.id);

/** The `print` field of the export request. */
export function printRequest(p: PrintPrefs, autoBleedMm: number, profiles: ProfileInfo[]): PrintExportOptions {
  const bleedMm = bleedMmFor(p, autoBleedMm);
  const standard = standardFor(p);
  const profile = effectiveProfile(p, profiles);
  const out: PrintExportOptions = {
    bleedMm, cropMarks: p.marks,
    standard, flatten: standard === 'pdfx1a' || p.flatten, intent: p.intent,
  };
  if (p.marks && p.registration) out.registrationMarks = true;
  if (p.marks && p.colorBars) out.colorBars = true;
  if (profile && (p.color === 'cmyk' || standard !== 'none')) out.profile = profileRef(profile);
  return out;
}

// ---------------------------------------------------------------- remembered choices

const prefsKey = (userId: string | undefined) => `oc.download.print.${userId ?? 'anon'}`;

export function loadPrintPrefs(userId: string | undefined, printProduct: boolean): PrintPrefs {
  const base = defaultPrintPrefs(printProduct);
  try {
    const raw = localStorage.getItem(prefsKey(userId));
    if (!raw) return base;
    const saved = JSON.parse(raw) as Record<string, unknown>;
    // "Crop marks and bleed" follows the design (on for print products) unless the user changed it for this kind of design.
    const marks = printProduct ? saved.marksPrint : saved.marksOther;
    return { ...base, ...pick(saved), marks: typeof marks === 'boolean' ? marks : base.marks };
  } catch {
    return base;
  }
}

export function savePrintPrefs(userId: string | undefined, printProduct: boolean, p: PrintPrefs): void {
  try {
    const prev = JSON.parse(localStorage.getItem(prefsKey(userId)) ?? '{}') as Record<string, unknown>;
    localStorage.setItem(prefsKey(userId), JSON.stringify({ ...prev, ...p, marks: undefined, [printProduct ? 'marksPrint' : 'marksOther']: p.marks }));
  } catch { /* storage blocked */ }
}

/** Only known fields with the right types survive (old or hand-edited storage). */
function pick(raw: Record<string, unknown>): Partial<PrintPrefs> {
  const s = raw as Partial<Record<keyof PrintPrefs, unknown>>;
  const o: Partial<PrintPrefs> = {};
  const oneOf = <T extends string>(v: unknown, all: readonly T[]): v is T => typeof v === 'string' && (all as readonly string[]).includes(v);
  if (typeof s.flatten === 'boolean') o.flatten = s.flatten;
  if (s.color === 'rgb' || s.color === 'cmyk') o.color = s.color;
  if (oneOf(s.bleed, ['auto', 'none', '3', '5', 'custom'] as const)) o.bleed = s.bleed;
  if (typeof s.customBleedMm === 'number' && s.customBleedMm >= 0 && s.customBleedMm <= 10) o.customBleedMm = s.customBleedMm;
  if (typeof s.registration === 'boolean') o.registration = s.registration;
  if (typeof s.colorBars === 'boolean') o.colorBars = s.colorBars;
  if (typeof s.profile === 'string') o.profile = s.profile;
  if (s.intent === 'relative' || s.intent === 'perceptual') o.intent = s.intent;
  if (oneOf(s.standard, ['auto', 'none', 'pdfx4', 'pdfx1a'] as const)) o.standard = s.standard;
  return o;
}

/** Last chosen download format, per surface. */
export function loadLastFormat(userId: string | undefined, surface: string): string | null {
  try { return localStorage.getItem(`oc.download.format.${userId ?? 'anon'}.${surface}`); } catch { return null; }
}
export function saveLastFormat(userId: string | undefined, surface: string, format: string): void {
  try { localStorage.setItem(`oc.download.format.${userId ?? 'anon'}.${surface}`, format); } catch { /* storage blocked */ }
}

// ---------------------------------------------------------------- API

async function call<T>(method: string, url: string, body?: unknown, signal?: AbortSignal): Promise<T> {
  const init: RequestInit = { method, credentials: 'same-origin', signal };
  if (body instanceof FormData) init.body = body;
  else if (body !== undefined) { init.body = JSON.stringify(body); init.headers = { 'content-type': 'application/json' }; }
  const res = await fetch(url, init);
  const ct = res.headers.get('content-type') ?? '';
  const j = ct.includes('application/json') ? ((await res.json().catch(() => null)) as (T & { error?: string; message?: string }) | null) : null;
  if (!res.ok) throw new ApiError(res.status, j?.error ?? 'error', j?.message);
  return j as T;
}

export interface GamutResult { inGamut: boolean; deltaE: number; cmyk: [number, number, number, number] }

export const printColorApi = {
  profiles: (teamId?: string) => call<{ profiles: ProfileInfo[] }>('GET', `/api/print/profiles${teamId ? `?teamId=${encodeURIComponent(teamId)}` : ''}`),
  upload: (file: File, opts: { name?: string; teamId?: string } = {}) => {
    const f = new FormData();
    f.append('file', file);
    if (opts.name) f.append('name', opts.name);
    if (opts.teamId) f.append('teamId', opts.teamId);
    return call<ProfileInfo>('POST', '/api/print/profiles', f);
  },
  remove: (id: string) => call<unknown>('DELETE', `/api/print/profiles/${encodeURIComponent(id)}`),
  gamut: (b: { profile: string | { id: string }; intent: 'relative' | 'perceptual'; colors: ColorValue[] }, signal?: AbortSignal) =>
    call<{ results: GamutResult[] }>('POST', '/api/print/gamut', b, signal),
};
