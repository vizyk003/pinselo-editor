# Contributing to Pinselo

Bug reports, fixes and improvements are welcome. For a large change, open an issue first so the approach can be agreed before you build it.

Start with [Getting started](docs/getting-started.md), [Architecture](docs/architecture.md) and [Testing](docs/testing.md).

## Project rules

- **Backend and engine changes are Rust** (`crates/`). Behaviour the web app needs goes through `crates/oc-wasm` and
  the `@oc/engine` facade; add new document fields and wasm exports to `packages/engine/src/types/` in the same change.
- **Goldens are frozen.** `tests/golden` was recorded from the former TypeScript implementation; the crates and the
  engine replay it. Change a golden only for a deliberate behaviour change, and say why in the commit. The PS-1 pixel
  baselines are the exception that is meant to be regenerated: `OC_PS1_UPDATE=1 cargo test -p oc-render --test ps1`,
  after reviewing the pages it reports.
- **Templates** live in `tools/scripts/templates`; `pnpm tpl:preview` renders them, `pnpm seed --fixtures` refreshes
  the test corpus in `fixtures/templates-seed` (then update the goldens that read it).
- **Strings:** wrap UI text in `t()`; `pnpm i18n:check` lists missing translations (web and the Rust mailer).
- Before a pull request: `pnpm typecheck`, `pnpm test`, `pnpm --filter @oc/engine test`, `cargo test --workspace` (with
  `OC_DATABASE_URL`), `cargo clippy --workspace --all-targets -- -D warnings`.

## Pull requests

- Keep each pull request to one change, and explain why it is needed.
- Add or update tests for any change in behaviour.
- Update the matching page in `docs/` when behaviour or configuration changes.

## License of contributions

By submitting a contribution, you agree that it is licensed under the license declared by the files you change. New files take the license of the crate or package they belong to. See the [license section of the README](README.md#license).
