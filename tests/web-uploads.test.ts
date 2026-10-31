import { describe, expect, it } from 'vitest';
import { ApiError } from '../apps/web/src/api';
import { MAX_FILES, uploadMany } from '../apps/web/src/lib/uploadFiles';
import {
  appendPage, chunk, daysLeft, fmtBytes, fmtDuration, isUploadable, pruneSelection, selectionState, toggleId, uploadParams, uploadTypeOf,
} from '../apps/web/src/pages/home/uploadsModel';

const DAY = 86_400_000;

describe('uploads library helpers (g19, g22, g87)', () => {
  it('builds short query strings', () => {
    expect(uploadParams({}).toString()).toBe('');
    expect(uploadParams({ q: '  cat ', type: 'all', sort: 'newest' }).toString()).toBe('q=cat');
    expect(uploadParams({ type: 'videos', sort: 'size', limit: 60, offset: 120 }).toString()).toBe('type=videos&sort=size&limit=60&offset=120');
  });

  it('groups uploads by type exactly like the server', () => {
    // The API filter (crates/oc-api/src/uploads.rs): images = image, svg and animated video; videos = other video.
    const server: Array<[{ kind: string; animated?: boolean }, string]> = [
      [{ kind: 'image' }, 'images'], [{ kind: 'svg' }, 'images'], [{ kind: 'video', animated: true }, 'images'],
      [{ kind: 'video', animated: false }, 'videos'], [{ kind: 'audio' }, 'audio'], [{ kind: 'pdf' }, 'files'],
    ];
    for (const [a, want] of server) expect(uploadTypeOf(a)).toBe(want);
  });

  it('counts down to the 30-day purge', () => {
    const now = Date.parse('2026-10-15T12:00:00Z');
    expect(daysLeft(new Date(now).toISOString(), now)).toBe(30);
    expect(daysLeft(new Date(now - 1000).toISOString(), now)).toBe(30);
    expect(daysLeft(new Date(now - 29.5 * DAY).toISOString(), now)).toBe(1);
    expect(daysLeft(new Date(now - 31 * DAY).toISOString(), now)).toBe(0);
    expect(daysLeft('not a date', now)).toBe(30);
  });

  it('formats sizes and durations', () => {
    expect(fmtBytes(0)).toBe('0 B');
    expect(fmtBytes(512)).toBe('512 B');
    expect(fmtBytes(1536)).toBe('1.5 KB');
    expect(fmtBytes(5 * 1024 ** 3)).toBe('5 GB');
    expect(fmtBytes(250 * 1024 ** 2)).toBe('250 MB');
    // The number follows the locale's separators.
    expect(fmtBytes(1.4 * 1024 ** 2, 'de')).toBe('1,4 MB');
    expect(fmtDuration(65_400)).toBe('1:05');
    expect(fmtDuration(3_723_000)).toBe('1:02:03');
  });

  it('skips files the upload pipeline would refuse', () => {
    expect(isUploadable({ name: 'a.png', type: 'image/png' })).toBe(true);
    expect(isUploadable({ name: 'clip.MOV', type: '' })).toBe(true);
    expect(isUploadable({ name: 'song', type: 'audio/mpeg' })).toBe(true);
    expect(isUploadable({ name: '.DS_Store', type: '' })).toBe(false);
    expect(isUploadable({ name: 'notes.txt', type: 'text/plain' })).toBe(false);
    expect(isUploadable({ name: 'photo.heic', type: 'image/heic' })).toBe(false);
  });

  it('manages selection and paging', () => {
    const s = toggleId(toggleId(new Set(), 'a'), 'b');
    expect([...toggleId(s, 'a')]).toEqual(['b']);
    expect(selectionState(s, ['a', 'b'])).toBe('all');
    expect(selectionState(s, ['a', 'b', 'c'])).toBe('some');
    expect(selectionState(new Set(), ['a'])).toBe('none');
    expect([...pruneSelection(s, ['b', 'c'])]).toEqual(['b']);
    expect(appendPage([{ id: 'a' }, { id: 'b' }], [{ id: 'b' }, { id: 'c' }]).map((r) => r.id)).toEqual(['a', 'b', 'c']);
    expect(chunk([1, 2, 3, 4, 5], 2)).toEqual([[1, 2], [3, 4], [5]]);
  });
});

describe('uploading many files', () => {
  const file = (name: string, type = 'image/png') => ({ name, type }) as File;
  const ok = (f: File) => Promise.resolve({ asset: { id: f.name, kind: 'image', sha256: '', mime: 'image/png', w: 1, h: 1 } as never });

  it('uploads supported files in order, reporting progress', async () => {
    const seen: string[] = [];
    const r = await uploadMany([file('a.png'), file('.DS_Store', ''), file('b.mp3', 'audio/mpeg')], (d, n) => seen.push(`${d}/${n}`), ok);
    expect(r.assets.map((a) => a.id)).toEqual(['a.png', 'b.mp3']);
    expect(r.skipped).toBe(1);
    expect(seen).toEqual(['0/2', '1/2', '2/2']);
  });

  it('collects failures and stops when the storage quota is full', async () => {
    let calls = 0;
    const flaky = (f: File) => {
      calls++;
      if (f.name === 'bad.png') return Promise.reject(new ApiError(415, 'corrupt_image', 'Broken'));
      if (f.name === 'full.png') return Promise.reject(new ApiError(402, 'upgrade_required', 'Full'));
      return ok(f);
    };
    const r = await uploadMany([file('a.png'), file('bad.png'), file('full.png'), file('c.png')], undefined, flaky);
    expect(r.failed).toEqual([{ name: 'bad.png', message: 'Broken' }, { name: 'full.png', message: 'Full' }]);
    expect(r.stopped).toBe(true);
    expect(calls).toBe(3);
  });

  it('caps one batch', async () => {
    const many = Array.from({ length: MAX_FILES + 5 }, (_, i) => file(`${i}.png`));
    expect((await uploadMany(many, undefined, ok)).assets.length).toBe(MAX_FILES);
  });
});
