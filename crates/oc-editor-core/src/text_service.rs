// SPDX-License-Identifier: Apache-2.0
//! Text geometry, injected by the host (backed by oc-text / the browser's text engine). Box-space coordinates;
//! offsets are UTF-16 code units into the paragraphs joined by `'\n'`.

use oc_schema::elements::TextEl;

pub trait TextService {
    /// Laid-out height of `el` at `width`.
    fn height(&self, el: &TextEl, width: f64) -> f64;
    /// Caret offset nearest a box-space point.
    fn offset_at(&self, el: &TextEl, x: f64, y: f64) -> i64;
    /// Caret offset one line up (`dir = -1`) or down (`dir = 1`).
    fn vertical_move(&self, el: &TextEl, offset: i64, dir: i32) -> i64;
    /// Word boundaries around an offset.
    fn word_at(&self, el: &TextEl, offset: i64) -> (i64, i64);
    /// Number of caret positions minus one (text length incl. paragraph breaks).
    fn length(&self, el: &TextEl) -> i64;
    /// Natural box size of curved text (ED-041); `None` while fonts load.
    fn curved_size(&self, _el: &TextEl) -> Option<(f64, f64)> {
        None
    }
}
