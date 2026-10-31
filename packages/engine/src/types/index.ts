// SPDX-License-Identifier: Apache-2.0
/**
 * Type-only exports of the replaced packages, re-exported unchanged. The facade's runtime is Rust; the TS type
 * declarations stay the contract until phase 6 moves them here (they are erased at build time).
 */
export type {
  AddressField, AddressProblem, AnchorSide, AnimDirection, AnimPreset, AnimUnit, AssetId, AssetRef, AudioClip,
  BlendMode, BoardMeta, BorderPreset, Brush, CalloutTone, CaptionCue, CaptionTrack, CellBorder, CellPos, CellRange,
  CellSide, ChartEl, ChartFormat, ChartKind, ChartSeries, ChartSource, ColorValue, DesignDoc, DesignId, DesignMeta,
  DesignOrigin, DocChartData, DrawEl, Duotone, Effect, Element, ElementAnim, ElementAnimOut, ElementBase,
  ElementEmphasis, ElementId, ElementOf, ElementTiming, ElementType, EmbedEl, EmbedProvider, EmbedTarget,
  EmphasisPreset, FormEl, FormField, FormFieldKind, FormRow, FrameCategory, FrameClip, FrameEl, FrameLibraryItem,
  FrameOverlay, GelatoMapping, GradientStop, GridCell, GridEl, GroupEl, Id, ImageAdjust, ImageEditStack, ImageEl,
  ImageFilterPreset, Keyframe, KeyframeEase, KeyframeProp, Keyframes, LineAttach, LineEl, LineMarker, LinkTarget,
  LockMask, Mat2D, MediaClip, Migrator, MotionPath, Page, PageId, PageSpan, Paint, Paragraph, ParagraphAttrs,
  PreflightCode, PreflightFix, PreflightIssue, PreflightResult, PreflightTarget, PresetCategoryId, PresetGroup,
  PriceBreakdown, PriceLine, PriceTier, PrintCatalog, PrintCategory, PrintChoice, PrintOptionGroup, PrintProduct,
  PrintSelection, PrintSize, PrintfulMapping, QrEl, Rect, Rgba, Run, RunStyle, SemanticRole, ShapeEl, ShapeKind,
  ShippingAddress, ShippingMethod, SizePreset, Stroke, Surface, SvgColorUse, TableCell, TableEdge, TableEl,
  TableStylePreset, TextBody, TextEffect, TextEl, TransitionKind, Unit, ValidationIssue, Vec2, VideoEl, VideoSlot,
  Xf,
} from './schema';
export type { AlignEdge, Guide, MarkerShape, SnapResult, SpacingMark } from './geometry';
export type {
  A11yIssue, A11yIssueCode, ClipboardPayload, CopiedStyle, ElementKindLabel, FindMatch, FontSet,
  Handle, ImportedPage, MindSpacing, Mods, PageSlot, ResizeResult, TextEditState, TextService, Viewport,
  VoteTally,
} from './editor-core';
// The editor state and hit targets with the pen tool and node editing (DS-100).
export type { EditorState, HitTarget } from '../vector';
export type {
  CellAddr, CellFormat, CellValue, ChartData, Merge, NumberFormat, Range, SheetLayoutOpts, TabSnap, TableOpts, ValueKind,
} from './sheet';
