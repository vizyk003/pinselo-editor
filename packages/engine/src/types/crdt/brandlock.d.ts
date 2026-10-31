// SPDX-License-Identifier: Apache-2.0
// The Pinselo document contract, implemented by crates/oc-crdt (see ../README.md).
/**
 * Brand-template locks (A8). Elements locked `by: 'brand'` hold non-admins to their mask:
 *  - position & size (`move`/`resize`/`rotate`, plus `delete`): no xf change, reparent, reorder or delete;
 *  - `content`: no text edits, image/video swaps, table cell or chart data changes;
 *  - `style`: no fills, strokes, fonts, effects, opacity, … (every other property).
 * Brand admins (team owners/admins) are not restricted. The lock itself can only be changed by an admin.
 * Used by `applyOps` (editor + API writes) and by the collab server to vet raw Yjs updates.
 */
import type { Element, LockMask } from '../schema';
/** Properties that are an element's content rather than its style. */
export declare const CONTENT_KEYS: Set<string>;
/** Properties anyone may change, even on a fully brand-locked element. */
export declare const BRAND_EXEMPT: Set<string>;
export type BrandAction = {
    kind: 'patch';
    keys: string[];
    xf?: {
        from: Element['xf'];
        to: Element['xf'];
    };
    body?: {
        from: unknown;
        to: unknown;
    };
} | {
    kind: 'xf';
    from: Element['xf'];
    to: Element['xf'];
} | {
    kind: 'move';
} | {
    kind: 'delete';
} | {
    kind: 'content';
} | {
    kind: 'style';
};
export declare function isBrandLock(lock: LockMask | undefined | null): lock is LockMask;
/** Why `action` would break the brand lock (null = allowed). */
export declare function brandLockError(lock: LockMask | undefined | null, action: BrandAction): string | null;
/** What changed between two text bodies: the words, the styling, or both. */
export declare function bodyChange(a: unknown, b: unknown): {
    content: boolean;
    style: boolean;
};
/**
 * Compare two versions of the elements map and list brand-lock violations (collab server check).
 * Only elements that were brand-locked before the change are inspected.
 */
export declare function brandLockViolations(before: Record<string, Element>, after: Record<string, Element>): Array<{
    id: string;
    reason: string;
}>;
