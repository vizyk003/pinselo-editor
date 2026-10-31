#!/usr/bin/env bash
# Deploy the latest images on the Oracle server (docs/deploy-oracle.md): pull, migrate, restart what changed and
# remove old images so the disk does not fill up. Run it from this directory.
#   ./update.sh        everything: migrations run and the backend restarts
#   ./update.sh web    only the gateway (web app): the backend keeps running
set -euo pipefail
cd "$(dirname "$0")"

for f in .env Caddyfile supabase-ca.crt; do
  [ -f "$f" ] || { echo "update: $f is missing next to compose.yml (see docs/deploy-oracle.md)" >&2; exit 1; }
done

if [ "${1:-}" = web ]; then
  docker compose pull --ignore-pull-failures gateway
  docker compose up -d --no-deps gateway
  docker image prune -f
  docker compose ps
  exit 0
elif [ -n "${1:-}" ]; then
  echo "usage: ./update.sh [web]" >&2
  exit 1
fi

# Images built on this server (build-on-server.sh) exist only here, so a failed pull is not an error.
docker compose pull --ignore-pull-failures
# Supabase's session pool is small: free the running services' connections so the migration can connect.
docker compose stop api collab render worker
docker compose up -d --remove-orphans
docker image prune -f
docker compose ps
