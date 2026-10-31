// SPDX-License-Identifier: Apache-2.0
// The Pinselo document contract, implemented by crates/oc-schema (see ../README.md).
import type { Id } from './primitives';
/** 16-character base62 id, generated client-side. */
export declare function newId<T extends string>(): Id<T>;
export declare function isId(s: unknown): s is string;
/**
 * Fractional index between `a` and `b` (null = open end).
 * Jittered: picks one of several evenly spaced candidate keys at random so two
 * clients inserting at the same spot concurrently are unlikely to collide.
 * Remaining ties are broken by id at read time.
 */
export declare function keyBetween(a: string | null | undefined, b: string | null | undefined, jitter?: boolean): string;
export declare function keysBetween(a: string | null | undefined, b: string | null | undefined, n: number): string[];
export declare function compareOrder(a: {
    key: string;
    id: string;
}, b: {
    key: string;
    id: string;
}): number;
