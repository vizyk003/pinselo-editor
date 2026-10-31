// SPDX-License-Identifier: Apache-2.0
// The Pinselo document contract, implemented by crates/oc-schema (see ../README.md).
/**
 * Embeds (A11): which URLs can be embedded and the iframe each becomes. Only these providers and
 * their official embed hosts are allowed (the iframe `src` is derived here, never taken from oEmbed HTML).
 */
import type { EmbedProvider } from './elements';
export interface EmbedTarget {
    provider: EmbedProvider;
    src: string;
    kind: 'video' | 'audio' | 'map' | 'design' | 'code';
    aspect: number;
    oembed?: string;
}
/** Hosts an embed iframe may load (CSP frame-src and the website builder use this list). */
export declare const EMBED_FRAME_HOSTS: string[];
export declare function parseEmbedUrl(raw: string): EmbedTarget | null;
