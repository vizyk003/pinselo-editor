// SPDX-License-Identifier: Apache-2.0
/**
 * Types for vector editing (DS-100) and masks (DS-107), mirroring crates/oc-schema (`GroupMask`), crates/oc-geometry
 * (`PathNode`, `NodeRef`, `BoolOp`) and crates/oc-editor-core (`PenState`, `PathEditState`, the `pathNode` hit
 * target). Kept in this one block because the TS document types it extends (`@oc/schema`, `@oc/editor-core`) are
 * being replaced; move these next to them when the types move here.
 */
import type * as EC from './types/editor-core';
import type { Element, ElementId, GroupEl, PageId, Paint, Vec2 } from './types/schema';

/** How a mask gates the masked content: by coverage, or by brightness × coverage. */
export type MaskMode = 'alpha' | 'luminance';

/** `GroupEl.mask`: the child `id` masks the group's other children and is not drawn itself. */
export interface GroupMask {
  id: ElementId;
  mode: MaskMode;
}

/** The mask of a group element (the field is absent from the `@oc/schema` type). */
export function maskOf(el: Element | undefined): GroupMask | undefined {
  return el?.type === 'group' ? (el as GroupEl & { mask?: GroupMask }).mask : undefined;
}

/** One Bézier node: anchor and optional handles, absolute positions. */
export interface PathNode {
  p: Vec2;
  in?: Vec2;
  out?: Vec2;
  smooth: boolean;
}

export interface SubPath {
  nodes: PathNode[];
  closed: boolean;
}

/** A node address: subpath `s`, node `i`. */
export interface NodeRef {
  s: number;
  i: number;
}

export type NodePart = 'anchor' | 'in' | 'out';

export type BoolOp = 'union' | 'subtract' | 'intersect' | 'exclude';

/** A path being drawn with the pen (page space). */
export interface PenState {
  pageId: PageId;
  nodes: PathNode[];
  /** Where the next node would go (hover or keyboard cursor). */
  cursor?: Vec2;
  dragging: boolean;
}

export interface PathEditState {
  id: ElementId;
  /** Selected nodes. */
  nodes: NodeRef[];
}

/** `editor.pathEditView()`: the edited path's nodes in page space. */
export interface PathEditView {
  id: ElementId;
  subpaths: SubPath[];
  selected: NodeRef[];
}

export type Tool = EC.EditorState['tool'] | 'pen';

/** The editor state with the pen tool and node editing. */
export type EditorState = Omit<EC.EditorState, 'tool'> & {
  tool: Tool;
  pen?: PenState;
  pathEdit?: PathEditState;
};

/** Pointer targets, with the nodes and handles of the path being edited. */
export type HitTarget = EC.HitTarget | { kind: 'pathNode'; s: number; i: number; part: NodePart };

/** `Painter.textOutline`: a text element's outline in its own box and the paint of its first run. */
export interface TextOutline {
  d: string;
  fill: Paint;
}
