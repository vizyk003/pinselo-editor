#!/usr/bin/env bash
# Nightly backup on the Oracle server (docs/deploy-oracle.md): dumps the database (keeps the last 7) and copies the
# Supabase file bucket here, so both survive losing Supabase. OC_BACKUP_S3_BUCKET adds a further copy off this server.
# Holds user data: stays private to this account.
set -euo pipefail
cd "$(dirname "$0")"
umask 077

v() { grep "^$1=" .env | cut -d= -f2- || true; }
stamp=$(date -u +%Y-%m-%dT%H%M%SZ)
mkdir -p backups
dump="backups/db-$stamp.dump"

docker run --rm -e PGSSLMODE=verify-full -e PGSSLROOTCERT=/ca.crt -v "$PWD/supabase-ca.crt:/ca.crt:ro" \
  -e "PGURL=$(v OC_DATABASE_URL)" postgres:17 sh -c 'pg_dump "$PGURL" --schema=public --format=custom --no-owner --no-privileges' > "$dump.tmp"
[ -s "$dump.tmp" ] || { echo "backup: empty dump" >&2; rm -f "$dump.tmp"; exit 1; }
mv "$dump.tmp" "$dump"
ls -1t backups/db-*.dump | tail -n +8 | xargs -r rm -f

aws() { docker run --rm -v "$PWD/backups:/b" -e "AWS_ACCESS_KEY_ID=$1" -e "AWS_SECRET_ACCESS_KEY=$2" -e "AWS_DEFAULT_REGION=$3" -e AWS_REQUEST_CHECKSUM_CALCULATION=when_required -e AWS_RESPONSE_CHECKSUM_VALIDATION=when_required amazon/aws-cli --endpoint-url "$4" "${@:5}"; }
src=$(v OC_S3_ENDPOINT)
from=("$(v AWS_ACCESS_KEY_ID)" "$(v AWS_SECRET_ACCESS_KEY)" "$(v OC_S3_REGION)" "${src%/}")
# Objects are named by their hash and never change, so files only ever get added here.
mkdir -p backups/files
aws "${from[@]}" s3 sync "s3://$(v OC_S3_BUCKET)" /b/files --only-show-errors

target=$(v OC_BACKUP_S3_BUCKET)
if [ -n "$target" ]; then
  to=("$(v OC_BACKUP_S3_ACCESS_KEY_ID)" "$(v OC_BACKUP_S3_SECRET_ACCESS_KEY)" "$(v OC_BACKUP_S3_REGION)" "$(v OC_BACKUP_S3_ENDPOINT)")
  aws "${to[@]}" s3 cp "/b/$(basename "$dump")" "s3://$target/db/$(basename "$dump")" --only-show-errors
  aws "${to[@]}" s3 sync /b/files "s3://$target/files" --only-show-errors
fi
echo "backup: ok $dump $(du -h "$dump" | cut -f1)${target:+ (off-site)}"
