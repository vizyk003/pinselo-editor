#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# Golden check (ADR-0001 parity, plan 13 stage 4): the Rust crates still match the committed goldens in tests/golden/.
# The goldens were written by the TypeScript reference implementation, which is deleted; they are frozen inputs now.
# A deliberate behaviour change updates the affected golden in the same commit (e.g. `OC_PS1_UPDATE=1 cargo test -p
# oc-render --test ps1` for the PS-1 pixel baselines) and says why.
#
#   bash tools/golden/check.sh
#
# OC_DATABASE_URL (optional): a Postgres URL. The configured database is never written to; the Rust DB tests create
# their own throwaway databases on that server (oc_db::testing::TestDb). Without it, DB tests are skipped.
set -euo pipefail
cd "$(dirname "$0")/../.."

if [ -n "$(git status --porcelain -- tests/golden)" ]; then
  echo "check: tests/golden/ already has uncommitted changes; commit or stash them first:" >&2
  git status --short -- tests/golden >&2
  exit 1
fi

# The committed Google Fonts catalogue is an input of the goldens (platform/gfonts, api/misc); a refreshed copy
# (`pnpm fonts:catalog` fetches the live one) changes what the crates compute.
if [ -n "$(git status --porcelain -- fixtures/fonts/google-catalog.json)" ]; then
  echo "check: fixtures/fonts/google-catalog.json differs from the committed catalogue; restore it (git checkout) or commit it with updated goldens" >&2
  exit 1
fi

[ -n "${OC_DATABASE_URL:-}" ] || echo "check: OC_DATABASE_URL not set; DB tests are skipped"

status=0
cargo test --workspace --no-fail-fast || status=$?

# The tests only read the goldens. A file that changed anyway is drift, unless it differs from the committed one by
# last-digit float noise only (compare.mjs restores those).
if [ -n "$(git status --porcelain --untracked-files=all -- tests/golden)" ] && ! node tools/golden/compare.mjs; then
  echo "check: a test rewrote tests/golden/; commit the change deliberately or fix the test" >&2
  status=1
fi
exit $status
