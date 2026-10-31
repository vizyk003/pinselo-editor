// SPDX-License-Identifier: AGPL-3.0-only
// The Pinselo document contract, implemented by crates/oc-editor-core (see ../README.md).
/**
 * Whiteboard facilitation (WB-020..): pure helpers for sticky notes, sections, mind maps, the shared timer
 * and dot voting. DOM- and Yjs-free; the Editor methods and the web UI call these.
 */
import type { TimerState, VoteEntry, VoteSession } from '../crdt';
import { type DesignDoc, type Element, type ElementId, type PageId } from '../schema';
type Doc = Pick<DesignDoc, 'elements' | 'pages'>;
/** The eight sticky-note colours (Canva-style pastel set). */
export declare const STICKY_COLORS: readonly ["#fef08a", "#fed7aa", "#fecaca", "#fbcfe8", "#ddd6fe", "#bfdbfe", "#a5f3fc", "#bbf7d0"];
export declare function isSticky(el: Element | undefined): boolean;
/** Group selected stickies by colour into tidy grids, placed left-to-right from the selection's top-left. */
export declare function clusterLayout(doc: Doc, ids: readonly ElementId[], gap?: number): Map<ElementId, {
    x: number;
    y: number;
}>;
export declare function isSection(el: Element | undefined): boolean;
/**
 * Elements that belong to a section: top-level siblings whose bounds lie fully inside it (sections nested
 * inside count too, with their members). A collapsed section's members are the ones it hid.
 */
export declare function sectionMembers(doc: Doc, sectionId: ElementId): ElementId[];
/** Sections of a page in navigator order. */
export declare function sectionsOf(doc: Doc, pageId: PageId): Element[];
/** Expand a set of moving ids with the members of any section among them (transitively). */
export declare function withSectionMembers(doc: Doc, ids: readonly ElementId[]): ElementId[];
export declare function isMindNode(el: Element | undefined): boolean;
/** The element a mind-map key acts on: the nearest top-level ancestor (stickies and labelled shapes are groups). */
export declare function mindNodeOf(doc: Doc, id: ElementId): Element | null;
export declare function mindChildren(doc: Doc, id: ElementId): Element[];
export declare function mindRoot(doc: Doc, id: ElementId): ElementId;
export interface MindSpacing {
    gapX: number;
    gapY: number;
}
/** Where a new child of `parentId` goes: to the right, centred on the parent or below the last child. */
export declare function mindChildPosition(doc: Doc, parentId: ElementId, size: {
    w: number;
    h: number;
}): {
    x: number;
    y: number;
};
/**
 * Balanced tree layout to the right of the root: every subtree gets a band as tall as its contents and the
 * parent is centred on its children's band. The root stays where it is. Returns new top-left positions.
 */
export declare function mindLayout(doc: Doc, rootId: ElementId, spacing?: Partial<MindSpacing>): Map<ElementId, {
    x: number;
    y: number;
}>;
export declare function makeTimer(durationMs: number, sound?: boolean): TimerState;
export declare function timerRemaining(t: TimerState, now: number): number;
export declare const timerRunning: (t: TimerState, now: number) => boolean;
export declare function timerStart(t: TimerState, now: number): TimerState;
export declare function timerPause(t: TimerState, now: number): TimerState;
export declare function timerReset(t: TimerState): TimerState;
export declare function timerAdd(t: TimerState, ms: number, now: number): TimerState;
export declare function formatClock(ms: number): string;
export interface VoteTally {
    element: ElementId;
    count: number;
    voters: Array<{
        id: string;
        name?: string;
        n: number;
    }>;
}
/** Dots per element for one session (only elements in scope), most votes first; ties keep scope order. */
export declare function tallyVotes(session: Pick<VoteSession, 'id' | 'scope'>, entries: readonly VoteEntry[]): VoteTally[];
/** Elements with the top `n` vote counts (ties at the cut-off are included; zero-vote elements never are). */
export declare function topVoted(tally: readonly VoteTally[], n?: number): ElementId[];
export declare function votesUsed(session: Pick<VoteSession, 'id'>, entries: readonly VoteEntry[], voter: string): number;
/** Dots the voter may still place. */
export declare function voteAllowance(session: Pick<VoteSession, 'id' | 'perPerson' | 'status'>, entries: readonly VoteEntry[], voter: string): number;
/** Default vote scope: the selection if any, else every sticky and shape (top-level, visible) on the page. */
export declare function voteScope(doc: Doc, pageId: PageId, selection: readonly ElementId[]): ElementId[];
