// SPDX-License-Identifier: AGPL-3.0-only
// The Pinselo document contract, implemented by crates/oc-editor-core (see ../README.md).
/** Bring template pages into an existing design: fresh ids, scaled to fit the design's page size. */
import { type DesignDoc, type Element, type Page } from '../schema';
export interface ImportedPage {
    page: Page;
    elements: Element[];
}
export declare function templatePages(tpl: DesignDoc, target: {
    w: number;
    h: number;
}): ImportedPage[];
