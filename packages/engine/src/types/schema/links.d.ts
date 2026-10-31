// SPDX-License-Identifier: Apache-2.0
// The Pinselo document contract, implemented by crates/oc-schema (see ../README.md).
/**
 * Link targets that are safe to write into exports and published sites (SEC-01). A `url` link may use
 * http(s), mailto or tel, or be relative (no scheme); `page` links point at a page of the design. Everything else
 * (`javascript:`, `data:`, `vbscript:`, `file:`, …) is rejected on write and dropped again at every output.
 * Mirrored in crates/oc-schema/src/links.rs.
 */
import type { DesignDoc } from './design';
import type { Element } from './elements';
import type { LinkTarget } from './primitives';
export declare const LINK_SCHEMES: readonly string[];
export declare const MAX_LINK_URL = 4096;
/**
 * True for a URL a link may point at. Browsers drop ASCII control characters and whitespace inside a scheme
 * (`java\tscript:`), so they are removed before the scheme is read.
 */
export declare function isSafeLinkUrl(url: unknown): url is string;
/** A link value is acceptable: absent, not an object (left to the schema), a page link, or a safe url link. */
export declare function isSafeLink(link: unknown): boolean;
/** The link if it is safe to output, else `undefined`. */
export declare function safeLink(link: LinkTarget | undefined): LinkTarget | undefined;
/** An element without unsafe links (the same object when it has none). */
export declare function withSafeElementLinks(el: Element): Element;
/** The design with every unsafe link removed (the same object when there is none). Exporters call this first. */
export declare function withSafeLinks(doc: DesignDoc): DesignDoc;
