// SPDX-License-Identifier: Apache-2.0
//! Golden parity with the TS render service (tests/golden/render-svc/, written by tools/golden/render-svc.ts).

mod common;

#[path = "parity/docs.rs"]
mod docs;
#[path = "parity/live.rs"]
mod live;
#[path = "parity/mobile.rs"]
mod mobile;
#[path = "parity/office.rs"]
mod office;
#[path = "parity/reflow.rs"]
mod reflow;
#[path = "parity/replay.rs"]
mod replay;
#[path = "parity/site.rs"]
mod site;
