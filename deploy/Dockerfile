# syntax=docker/dockerfile:1.7
# Production images (plan 13 stage 3.2). The backend is Rust: oc-api, oc-collab, oc-worker, oc-render-svc and the
# oc-migrate migrator in one `runtime` image; the compose file picks each service's binary. The web app is built
# with Node, including @oc/engine (crates/oc-wasm compiled to WebAssembly), and served by the `gateway` image.

ARG RUST_VERSION=1.99.0
ARG NODE_IMAGE=node:24-bookworm-slim

FROM ${NODE_IMAGE} AS node

# ---------------------------------------------------------------------------------------------- backend binaries
# The crates resolve the repository root from their source location at compile time (oc_config::repo_root), so
# the build and the runtime image both keep the repository layout under /app.
FROM rust:${RUST_VERSION}-bookworm AS backend
WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY .cargo .cargo
COPY crates crates
# Compiled in with include_str!/include_bytes! from crates/ (mail locales, ICC profile table, Supabase lockdown SQL)
# and fixtures/webauthn.
COPY fixtures/webauthn fixtures/webauthn
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/app/target,sharing=locked \
    cargo build --release --locked -p oc-api -p oc-collab -p oc-worker -p oc-render-svc -p oc-db \
 && mkdir -p /out \
 && cp target/release/oc-api target/release/oc-collab target/release/oc-worker target/release/oc-render-svc target/release/oc-migrate /out/

# ------------------------------------------------------------------------------------------------ build tools
FROM rust:${RUST_VERSION}-bookworm AS tools
COPY --from=node /usr/local/bin/node /usr/local/bin/node
COPY --from=node /usr/local/lib/node_modules /usr/local/lib/node_modules
RUN ln -s ../lib/node_modules/corepack/dist/corepack.js /usr/local/bin/corepack && corepack enable \
 && ln -s ../lib/node_modules/npm/bin/npm-cli.js /usr/local/bin/npm \
 && ln -s ../lib/node_modules/npm/bin/npx-cli.js /usr/local/bin/npx
WORKDIR /app

# ------------------------------------------------------------------------------------------- runtime fixtures
# Only the runtime image reads these, and the stage copies only what the syncs read: a web app change leaves the
# runtime image untouched. Graphics run last because they need the crates (oc-schema's SVG sanitiser).
FROM tools AS fixtures
COPY package.json ./
RUN npm install -g "tsx@$(node -p "require('./package.json').devDependencies.tsx")"
COPY fixtures/fonts fixtures/fonts
COPY crates/oc-color/icc-profiles.json crates/oc-color/icc-profiles.json
COPY tools/scripts/fonts-sync.ts tools/scripts/fonts-pins.json tools/scripts/emoji-sync.ts tools/scripts/icc-sync.ts tools/scripts/
RUN tsx tools/scripts/fonts-sync.ts && tsx tools/scripts/emoji-sync.ts && tsx tools/scripts/icc-sync.ts
COPY Cargo.toml Cargo.lock ./
COPY .cargo .cargo
COPY crates crates
COPY tools/scripts/graphics-sync.ts tools/scripts/
COPY tools/scripts/graphics tools/scripts/graphics
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/app/target,sharing=locked \
    tsx tools/scripts/graphics-sync.ts

# ------------------------------------------------------------------------------------------ web app and engine
FROM tools AS web
RUN apt-get update && apt-get install -y --no-install-recommends brotli && rm -rf /var/lib/apt/lists/* \
 && rustup target add wasm32-unknown-unknown
# The wasm-bindgen CLI must equal the wasm-bindgen crate in Cargo.lock.
COPY Cargo.lock ./
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    cargo install wasm-bindgen-cli --locked --version "$(sed -n '/^name = "wasm-bindgen"$/{n;s/^version = "\(.*\)"$/\1/p;}' Cargo.lock)"
COPY . .
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/app/target,sharing=locked \
    pnpm install --frozen-lockfile \
 && pnpm --filter @oc/engine build \
 && pnpm --filter @oc/web build
# Caddy serves these .br/.gz files as they are (file_server precompressed); it never compresses with brotli itself.
RUN find apps/web/dist -type f -size +1k \( -name '*.wasm' -o -name '*.js' -o -name '*.css' -o -name '*.html' -o -name '*.svg' -o -name '*.json' -o -name '*.webmanifest' \) \
      -exec brotli -kq 11 {} \; -exec gzip -k9 {} \;

# ------------------------------------------------------------------------------------------ backend services
FROM debian:bookworm-slim AS runtime
# ffmpeg: video export and ingest (render), social video (worker), media hashing (api). Imports need no poppler
# any more (oc-import rasterises PDFs with hayro).
RUN apt-get update && apt-get install -y --no-install-recommends ffmpeg ca-certificates && rm -rf /var/lib/apt/lists/* \
 && useradd --system --uid 1000 --home-dir /app oc \
 && mkdir -p /data && chown oc /data
ENV NODE_ENV=production
WORKDIR /app
COPY --from=backend /out/ /usr/local/bin/
COPY crates/oc-platform/oc-db/migrations crates/oc-platform/oc-db/migrations
COPY --from=fixtures /app/fixtures/fonts fixtures/fonts
COPY --from=fixtures /app/fixtures/emoji fixtures/emoji
COPY --from=fixtures /app/fixtures/graphics fixtures/graphics
COPY --from=fixtures /app/fixtures/icc fixtures/icc
COPY fixtures/templates-seed fixtures/templates-seed
USER oc
CMD ["oc-api"]

# ------------------------------------------------------------------------------------------------- gateway
FROM caddy:2.10-alpine AS gateway
COPY deploy/Caddyfile /etc/caddy/Caddyfile
COPY --from=web /app/apps/web/dist /srv/web
