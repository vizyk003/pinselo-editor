import { SIZE_PRESETS } from '@oc/engine';
import { describe, expect, it } from 'vitest';
import { parseCreateIntent, QUICK_TILES, seeAllIntent, tileCategory, tileIntent, tileSizes } from '../apps/web/src/pages/home/quickTiles';

describe('home quick tiles', () => {
  it('every preset tile points at an existing size preset', () => {
    for (const tile of QUICK_TILES) if (tile.preset) expect(SIZE_PRESETS.some((p) => p.id === tile.preset), tile.preset).toBe(true);
  });
  it('the Video tile creates a video-surface design', () => {
    const video = QUICK_TILES.find((x) => x.id === 'video')!;
    expect(SIZE_PRESETS.find((p) => p.id === video.preset)?.surface).toBe('video');
  });
  it('popovers list at most 10 sizes of the group, popular first', () => {
    const social = tileSizes('Social media');
    expect(social.length).toBeLessThanOrEqual(10);
    expect(social.every((p) => p.group === 'Social media')).toBe(true);
    expect(social[0]!.popular).toBe(true);
    expect(tileSizes(undefined)).toEqual([]);
  });
  it('See all sizes opens the matching Create dialog category', () => {
    const by = (id: string) => tileCategory(QUICK_TILES.find((x) => x.id === id)!);
    expect(by('social')).toBe('social');
    expect(by('print')).toBe('print');
    expect(by('custom')).toBe('custom');
    expect(by('more')).toBeUndefined();
  });
  it('signed-out tiles resume the right Create dialog view after signing in', () => {
    const intent = (id: string) => parseCreateIntent(tileIntent(QUICK_TILES.find((x) => x.id === id)!));
    expect(intent('more')).toEqual({ kind: 'dialog' });
    expect(intent('custom')).toEqual({ kind: 'dialog', category: 'custom' });
    expect(intent('upload')).toEqual({ kind: 'upload' });
    expect(intent('social')).toEqual({ kind: 'preset', preset: 'ig-post' });
    expect(parseCreateIntent(seeAllIntent('social'))).toEqual({ kind: 'dialog', category: 'social' });
    expect(parseCreateIntent(seeAllIntent(undefined))).toEqual({ kind: 'dialog' });
    expect(parseCreateIntent('cat:nope')).toBeNull();
    expect(parseCreateIntent('nope')).toBeNull();
    expect(parseCreateIntent(null)).toBeNull();
  });
});
