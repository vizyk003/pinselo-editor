// SPDX-License-Identifier: Apache-2.0
// The Pinselo document contract, implemented by crates/oc-schema (see ../README.md).
import type { DesignDoc } from './design';
export interface ValidationIssue {
    path: string;
    message: string;
}
export declare function validateDoc(doc: unknown): {
    ok: true;
    doc: DesignDoc;
} | {
    ok: false;
    issues: ValidationIssue[];
};
