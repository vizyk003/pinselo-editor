import { describe, expect, it } from 'vitest';
import { coverBox, MAX_MEDIA_DESIGN_PX, mediaDesignSize, startKind, startsVideo, titleFromFile, toAssetRef, withoutPlace } from '../apps/web/src/pages/home/startMedia';

describe('start from upload (g15)', () => {
  it('classifies files', () => {
    expect(startKind({ name: 'a.jpg', type: 'image/jpeg' })).toBe('media');
    expect(startKind({ name: 'clip', type: 'video/mp4' })).toBe('media');
    expect(startKind({ name: 'IMG.HEIC', type: '' })).toBe('media');
    expect(startKind({ name: 'deck.PPTX', type: '' })).toBe('import');
    expect(startKind({ name: 'notes.md', type: 'text/markdown' })).toBe('import');
    expect(startKind({ name: 'song.mp3', type: 'audio/mpeg' })).toBeNull();
  });

  it('sizes the design to the media, capped', () => {
    expect(mediaDesignSize(1200, 800, 'image')).toEqual({ w: 1200, h: 800 });
    expect(mediaDesignSize(16000, 4000, 'image')).toEqual({ w: MAX_MEDIA_DESIGN_PX, h: 2000 });
    expect(mediaDesignSize(0, 0, 'video')).toEqual({ w: 1920, h: 1080 });
    expect(mediaDesignSize(Number.NaN, 5, 'image')).toEqual({ w: 1080, h: 1080 });
  });

  it('starts real videos, not GIFs, on the video surface', () => {
    expect(startsVideo({ kind: 'video' })).toBe(true);
    expect(startsVideo({ kind: 'video', animated: true })).toBe(false);
    expect(startsVideo({ kind: 'image' })).toBe(false);
  });

  it('titles from the file name', () => {
    expect(titleFromFile('Beach day.final.jpg')).toBe('Beach day.final');
    expect(titleFromFile('.png')).toBe('Untitled design');
  });

  it('covers the page', () => {
    expect(coverBox(100, 50, 100, 50)).toEqual({ x: 0, y: 0, w: 100, h: 50 });
    const b = coverBox(100, 100, 200, 100);
    expect(b).toEqual({ x: 0, y: -50, w: 200, h: 200 });
  });

  it('converts asset rows, dropping nulls', () => {
    expect(toAssetRef({ id: 'a1', kind: 'video', sha256: 'x', mime: 'video/mp4', w: 10, h: 20, name: 'c.mp4', durationMs: 3000, hasAudio: null, animated: false, attribution: null }))
      .toEqual({ id: 'a1', kind: 'video', sha256: 'x', mime: 'video/mp4', w: 10, h: 20, name: 'c.mp4', durationMs: 3000 });
    expect(toAssetRef({ id: 'a1', kind: 'pdf' })).toBeNull();
  });

  it('strips only the place param', () => {
    expect(withoutPlace('?place=abc')).toBe('');
    expect(withoutPlace('?place=abc&share=1')).toBe('?share=1');
  });
});
