// SPDX-License-Identifier: Apache-2.0
/**
 * `@oc/engine/lazy`: the whole facade without automatic initialisation. Call `await initEngine()` (optionally
 * with the wasm bytes/URL/module) before using anything; constants are filled in at that point.
 */
export { engineReady, initEngine, onEngineReady, raw, type EngineInput } from './wasm';
export { Editor, EnginePanicError, hardenDoc, isEngineTrap, type EngineEditorOptions, type EngineStore, type OriginName } from './editor';
export { EngineTextService, FALLBACK_FAMILY, FontRegistry, TextEngine, type FontFace, type FontFamily, type FontLoader, type FontManifest, type MissingFace, type TextLayout } from './text';
export {
  DisplayList, Painter, RenderTarget, canvas2dTransitionTarget, clickStepCount, drawTransition, pageSettleMs, resolvePage, sliceColor, smoothPath, svgToElements, transitionFrame, transitionProgress,
  type AssetSource, type DLItem, type ResolveOptions, type ResolvedPaint, type TransitionCanvas, type TransitionFrame, type TransitionLayer, type VideoFrameSource,
} from './render';
export { docRef } from './docref';
export { decodeDesign, encodeDesign } from './crdt';
export * from './schema';
export * from './geometry';
export * from './editor-fns';
export type * from './types';
export { maskOf } from './vector';
export type { BoolOp, GroupMask, MaskMode, NodePart, NodeRef, PathEditState, PathEditView, PathNode, PenState, SubPath, TextOutline, Tool } from './vector';
export type { DocOp, DocOpResult } from './types/crdt';
