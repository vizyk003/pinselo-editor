/**
 * Browser media playback for the editor/presenter: one <video> per playing video (video elements, frames and grid cells) and one <audio> per
 * soundtrack clip, kept in sync with the design playhead. The painter samples each playing <video>'s current frame when it draws.
 */
import type { Editor, Painter } from '@oc/engine';
import { locateTime, pageDurationMs, descendants, mediaSourceTime, videoSlots, type DesignDoc } from '@oc/engine';
import { assetUrl } from '../api';

export const mediaUrl = (asset: string) => assetUrl(asset);

export class MediaPool {
  private videos = new Map<string, HTMLVideoElement>();
  private audios = new Map<string, HTMLAudioElement>();
  private onFrame: (() => void) | null = null;

  /** Repaint callback (seek completed, metadata loaded…). */
  setInvalidate(fn: () => void) {
    this.onFrame = fn;
  }

  private video(srcEl: string, asset: string): HTMLVideoElement {
    let v = this.videos.get(srcEl);
    if (!v || !v.src.endsWith(mediaUrl(asset))) {
      v?.pause();
      v = document.createElement('video');
      v.crossOrigin = 'anonymous';
      v.playsInline = true;
      v.preload = 'auto';
      v.src = mediaUrl(asset);
      v.addEventListener('seeked', () => this.onFrame?.());
      v.addEventListener('loadeddata', () => this.onFrame?.());
      this.videos.set(srcEl, v);
    }
    return v;
  }

  private audio(id: string, asset: string): HTMLAudioElement {
    let a = this.audios.get(id);
    if (!a || !a.src.endsWith(mediaUrl(asset))) {
      a?.pause();
      a = new Audio(mediaUrl(asset));
      a.preload = 'auto';
      this.audios.set(id, a);
    }
    return a;
  }

  /** Bring every media element in line with design time `t`. Call each frame while playing, and on seek. */
  sync(doc: DesignDoc, t: number, playing: boolean) {
    const loc = locateTime(doc, t);
    const live = new Set<string>();
    if (loc) {
      const pageMs = pageDurationMs(doc, loc.span.page);
      for (const el of descendants(doc, loc.span.page.id)) {
        if (el.visible === false) continue;
        // Video elements, frames holding a video and video grid cells (one <video> per slot key).
        for (const slot of videoSlots(el)) {
          const v = this.video(slot.key, slot.asset);
          live.add(slot.key);
          const src = mediaSourceTime(el, slot.clip, loc.pageTimeMs, pageMs);
          v.muted = slot.clip.muted || !playing;
          v.volume = Math.min(1, Math.max(0, slot.clip.volume));
          v.playbackRate = slot.clip.speed;
          if (src === null) {
            if (!v.paused) v.pause();
            continue;
          }
          const target = src / 1000;
          // A finished non-looping clip in a frame holds its last frame.
          const ended = !slot.clip.loop && src >= slot.clip.trimEndMs - 1;
          if (playing && !ended) {
            if (Math.abs(v.currentTime - target) > 0.3) v.currentTime = target;
            if (v.paused) void v.play().catch(() => {});
          } else {
            if (!v.paused) v.pause();
            if (Math.abs(v.currentTime - target) > 0.02) v.currentTime = target;
          }
        }
      }
    }
    for (const [id, v] of this.videos) if (!live.has(id) && !v.paused) v.pause();

    const clips = new Set<string>();
    for (const c of doc.meta.audio ?? []) {
      const a = this.audio(c.id, c.asset);
      clips.add(c.id);
      const local = t - c.startMs;
      const inside = local >= 0 && local < c.durationMs;
      if (!playing || !inside) {
        if (!a.paused) a.pause();
        continue;
      }
      const fadeIn = c.fadeInMs > 0 ? Math.min(1, local / c.fadeInMs) : 1;
      const fadeOut = c.fadeOutMs > 0 ? Math.min(1, (c.durationMs - local) / c.fadeOutMs) : 1;
      a.volume = Math.max(0, Math.min(1, c.volume * fadeIn * fadeOut));
      const target = (c.trimStartMs + local) / 1000;
      if (Math.abs(a.currentTime - target) > 0.3) a.currentTime = target;
      if (a.paused) void a.play().catch(() => {});
    }
    for (const [id, a] of this.audios) {
      if (!clips.has(id)) {
        a.pause();
        this.audios.delete(id);
      }
    }
  }

  pauseAll() {
    for (const v of this.videos.values()) v.pause();
    for (const a of this.audios.values()) a.pause();
  }

  /** Painter hook: the <video> playing a slot (the painter samples its current frame), null until it has one. */
  attach(painter: Painter) {
    painter.videoFrames = (asset, _src, key) => {
      const v = this.video(key, asset);
      return v.readyState < 2 || !v.videoWidth ? null : v;
    };
  }

  destroy() {
    this.pauseAll();
    for (const v of this.videos.values()) v.removeAttribute('src');
    this.videos.clear();
    this.audios.clear();
  }
}

/** Shared per-editor pool. */
const pools = new WeakMap<Editor, MediaPool>();
export function poolFor(editor: Editor): MediaPool {
  let p = pools.get(editor);
  if (!p) {
    p = new MediaPool();
    pools.set(editor, p);
  }
  return p;
}

export function fmtTime(ms: number): string {
  const s = Math.max(0, ms) / 1000;
  const m = Math.floor(s / 60);
  return `${m}:${(s - m * 60).toFixed(1).padStart(4, '0')}`;
}
