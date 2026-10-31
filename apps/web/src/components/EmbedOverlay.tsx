/**
 * Live embeds (A11) on view links and in Present mode: the poster is replaced by the provider's player
 * once clicked (or immediately for maps/designs), sandboxed and allow-listed.
 */
import { worldMatrix } from '@oc/engine';
import { EMBED_FRAME_HOSTS, type DesignDoc, type EmbedEl } from '@oc/engine';
import { useState } from 'react';
import { t } from '../i18n';

const allowed = (src: string) => EMBED_FRAME_HOSTS.some((h) => src.startsWith(`${h}/`));

export function EmbedOverlay({ doc, el, scale, offset = [0, 0] }: { doc: DesignDoc; el: EmbedEl; scale: number; offset?: [number, number] }) {
  const m = worldMatrix(doc, el.id);
  const media = el.provider === 'youtube' || el.provider === 'vimeo' || el.provider === 'loom';
  const [live, setLive] = useState(!media);
  if (!allowed(el.src)) return null;
  const style: React.CSSProperties = { position: 'absolute', left: offset[0] + m[4] * scale, top: offset[1] + m[5] * scale, width: el.xf.w * scale, height: el.xf.h * scale, border: 0, borderRadius: Math.min(el.xf.w, el.xf.h) * 0.04 * scale };
  if (!live) {
    return <button style={{ ...style, background: 'transparent', cursor: 'pointer' }} onClick={(e) => { e.stopPropagation(); setLive(true); }} aria-label={t('Play {title}', { title: el.title || el.provider })} />;
  }
  const src = media ? `${el.src}${el.src.includes('?') ? '&' : '?'}autoplay=1` : el.src;
  return (
    <iframe title={el.title || el.provider} src={src} style={style} loading="lazy" referrerPolicy="strict-origin-when-cross-origin"
      allow="autoplay; encrypted-media; picture-in-picture; fullscreen; clipboard-write" allowFullScreen
      sandbox="allow-scripts allow-same-origin allow-presentation allow-popups allow-popups-to-escape-sandbox" onClick={(e) => e.stopPropagation()} />
  );
}
