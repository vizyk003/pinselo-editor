// SPDX-License-Identifier: Apache-2.0
// The Pinselo document contract, implemented by crates/oc-geometry (see ../README.md).
/**
 * Connectors (WB-010): line ends glued to element anchors. Endpoints are derived from the attached
 * elements, so moving a shape drags its connectors along — in the renderer (live, with drag overrides)
 * and when the editor commits a move (persisted points/xf).
 */
import type { AnchorSide, DesignDoc, ElementId, LineEl, Vec2, Xf } from '../schema';
type Doc = Pick<DesignDoc, 'elements' | 'pages'>;
export declare const ANCHORS: readonly AnchorSide[];
/** Page-space point of an element anchor (follows rotation). */
export declare function anchorPoint(doc: Doc, id: ElementId, anchor: AnchorSide): [number, number] | null;
/** Nearest anchor of an element to a page-space point, within `radius`. */
export declare function nearestAnchor(doc: Doc, id: ElementId, p: Vec2, radius: number): {
    anchor: AnchorSide;
    point: [number, number];
} | null;
/** Route between two page-space points given the outward normals at each end. */
export declare function routeConnector(a: Vec2, na: Vec2, b: Vec2, nb: Vec2, route?: LineEl['route']): Array<[number, number]>;
/** Current page-space polyline of a line (attachments resolved), or null when it has none. */
export declare function connectorPagePoints(doc: Doc, line: LineEl): Array<[number, number]> | null;
/** Connector polyline in the line's own box space (for the renderer). */
export declare function connectorLocalPoints(doc: Doc, line: LineEl): Array<[number, number]> | null;
/**
 * New xf + points for a top-level connector so its box tightly wraps the routed path (persisted on commit).
 * Rotation is reset; connectors are defined by their ends.
 */
export declare function connectorGeometry(doc: Doc, line: LineEl): {
    xf: Xf;
    points: Array<[number, number]>;
} | null;
/** Lines attached to any of the given elements. */
export declare function connectorsOf(doc: Doc, ids: ReadonlySet<ElementId>): LineEl[];
