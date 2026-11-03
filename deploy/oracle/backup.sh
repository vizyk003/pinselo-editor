#!/usr/bin/env bash
# Scheduled backup on the Oracle server (docs/deploy-oracle.md): dumps the database (keeps the last 7), test-restores the
# dump, copies the Supabase file bucket here, then sends everything to two S3-compatible buckets at different providers
# (OC_BACKUP_S3_* and OC_BACKUP2_S3_*). A failure in one step or bucket never stops the others; it fails the run at the
# end, and OC_BACKUP_PING_URL reports the result to a monitor that also notices when no run happens at all.
# The database dump and .env leave this server encrypted to OC_BACKUP_AGE_RECIPIENT. The private key is not on this server.
# Holds user data: stays private to this account.
set -euo pipefail
cd "$(dirname "$0")"
umask 077

# Runs must not overlap (cron and a manual run): they would write the same files. A refused run leaves the monitor alone.
mkdir -p backups
exec 9>backups/.lock
flock -n 9 || { echo "backup: another run is still going" >&2; exit 1; }

v() { grep "^$1=" .env | cut -d= -f2- || true; }
ping_monitor() { [ -z "$ping_url" ] || curl -fsS -m 10 --retry 3 -o /dev/null "$ping_url$1" || true; }
fail() { echo "backup: FAILED: $1" >&2; failures=$((failures + 1)); }

stamp=$(date -u +%Y-%m-%dT%H%M%SZ)
ping_url=$(v OC_BACKUP_PING_URL)
ping_url=${ping_url%/}
failures=0
finished=
check_db=oc-backup-check
finish() {
  docker rm -f "$check_db" >/dev/null 2>&1 || true
  # Reached with no result only when the script itself stopped early.
  [ -n "$finished" ] || ping_monitor /fail
}
trap finish EXIT
ping_monitor /start

recipient=$(v OC_BACKUP_AGE_RECIPIENT)
[ -n "$recipient" ] || { echo "backup: OC_BACKUP_AGE_RECIPIENT is empty: nothing may leave this server unencrypted" >&2; exit 1; }
command -v age >/dev/null || { echo "backup: age is not installed (sudo apt-get install -y age)" >&2; exit 1; }

dump="backups/db-$stamp.dump"
secrets="backups/env-$stamp.env.age"

dump_flags='--schema=public --format=custom --no-owner --no-privileges'
case "$(v OC_DATABASE_URL)" in
  *@db:*) docker compose exec -T db sh -c "pg_dump -U \"\$POSTGRES_USER\" -d \"\$POSTGRES_DB\" $dump_flags" > "$dump.tmp" ;;
  *) docker run --rm -e PGSSLMODE=verify-full -e PGSSLROOTCERT=/ca.crt -v "$PWD/supabase-ca.crt:/ca.crt:ro" \
    -e "PGURL=$(v OC_DATABASE_URL)" postgres:17 sh -c "pg_dump \"\$PGURL\" $dump_flags" > "$dump.tmp" ;;
esac
[ -s "$dump.tmp" ] || { echo "backup: empty dump" >&2; rm -f "$dump.tmp"; exit 1; }
mv "$dump.tmp" "$dump"
ls -1t backups/db-*.dump | tail -n +8 | xargs -r rm -f

# A dump that does not restore is not a backup. Restores over TCP: during its first start the image's temporary server
# already answers on the local socket.
restore_check() {
  docker rm -f "$check_db" >/dev/null 2>&1 || true
  docker run -d --name "$check_db" --network none -e POSTGRES_HOST_AUTH_METHOD=trust postgres:17 >/dev/null || return 1
  local tries=0
  until docker exec "$check_db" pg_isready -q -h 127.0.0.1 -U postgres; do
    tries=$((tries + 1))
    [ "$tries" -lt 60 ] || { echo "backup: the check database did not start" >&2; return 1; }
    sleep 1
  done
  # --clean: the dump creates the public schema, which a new database already has.
  docker exec -i "$check_db" pg_restore -h 127.0.0.1 -U postgres -d postgres --no-owner --clean --if-exists --exit-on-error < "$dump" || return 1
  local counts
  counts=$(docker exec "$check_db" psql -h 127.0.0.1 -U postgres -Atc 'select (select count(*) from schema_migrations), (select count(*) from users)') || return 1
  echo "backup: restore check: ${counts%|*} migrations, ${counts#*|} users"
  [ "${counts%|*}" -gt 0 ] && [ "${counts#*|}" -gt 0 ]
}
restore_check || fail "the dump did not pass the restore check"
docker rm -f "$check_db" >/dev/null 2>&1 || true

age -r "$recipient" -o "$dump.age.tmp" "$dump"
age -r "$recipient" -o "$secrets.tmp" .env
mv "$dump.age.tmp" "$dump.age"
mv "$secrets.tmp" "$secrets"
for kept in 'db-*.dump.age' 'env-*.env.age'; do
  # shellcheck disable=SC2086 # the pattern is meant to expand here
  ls -1t backups/$kept | tail -n +8 | xargs -r rm -f
done

aws() { docker run --rm -v "$PWD/backups:/b" -e "AWS_ACCESS_KEY_ID=$1" -e "AWS_SECRET_ACCESS_KEY=$2" -e "AWS_DEFAULT_REGION=$3" -e AWS_REQUEST_CHECKSUM_CALCULATION=when_required -e AWS_RESPONSE_CHECKSUM_VALIDATION=when_required amazon/aws-cli --endpoint-url "$4" "${@:5}"; }
src=$(v OC_S3_ENDPOINT)
from=("$(v AWS_ACCESS_KEY_ID)" "$(v AWS_SECRET_ACCESS_KEY)" "$(v OC_S3_REGION)" "${src%/}")
# Objects are named by their hash and never change, so files only ever get added here. One get-object at a time:
# Supabase's S3 endpoint fails the downloads of `aws s3 sync`.
mkdir -p backups/files
docker run --rm -v "$PWD/backups:/b" -e "AWS_ACCESS_KEY_ID=${from[0]}" -e "AWS_SECRET_ACCESS_KEY=${from[1]}" -e "AWS_DEFAULT_REGION=${from[2]}" \
  -e AWS_REQUEST_CHECKSUM_CALCULATION=when_required -e AWS_RESPONSE_CHECKSUM_VALIDATION=when_required \
  -e "EP=${from[3]}" -e "BUCKET=$(v OC_S3_BUCKET)" --entrypoint sh amazon/aws-cli -c '
  failed=0; fetched=0
  keys=$(aws --endpoint-url "$EP" s3api list-objects-v2 --bucket "$BUCKET" --query "Contents[].Key" --output text) || exit 1
  for k in $keys; do
    [ "$k" != None ] || continue
    [ -s "/b/files/$k" ] && continue
    mkdir -p "$(dirname "/b/files/$k")"
    if aws --endpoint-url "$EP" s3api get-object --bucket "$BUCKET" --key "$k" "/b/files/$k.tmp" >/dev/null && mv "/b/files/$k.tmp" "/b/files/$k"; then
      fetched=$((fetched + 1))
    else
      rm -f "/b/files/$k.tmp"; failed=$((failed + 1)); echo "backup: could not fetch $k" >&2
    fi
  done
  echo "backup: files: $fetched new, $failed failed"
  [ "$failed" -eq 0 ]' || fail "copying the file bucket here"

# Reads the database copy back, so a bucket counts only once it has really returned what was sent.
send_to() {
  local target=$1
  local to=("$(v "${2}_ACCESS_KEY_ID")" "$(v "${2}_SECRET_ACCESS_KEY")" "$(v "${2}_REGION")" "$(v "${2}_ENDPOINT")")
  local name
  name=$(basename "$dump.age")
  aws "${to[@]}" s3 cp "/b/$name" "s3://$target/db/$name" --only-show-errors || return 1
  aws "${to[@]}" s3 cp "/b/$(basename "$secrets")" "s3://$target/env/$(basename "$secrets")" --only-show-errors || return 1
  # The 03:00 UTC run on the 1st is also kept under monthly/, which the bucket's lifecycle rule keeps for longer.
  if [ "$(date -u +%d%H)" = 0103 ]; then
    aws "${to[@]}" s3 cp "/b/$name" "s3://$target/monthly/$name" --only-show-errors || return 1
    aws "${to[@]}" s3 cp "/b/$(basename "$secrets")" "s3://$target/monthly/$(basename "$secrets")" --only-show-errors || return 1
  fi
  aws "${to[@]}" s3 sync /b/files "s3://$target/files" --only-show-errors || return 1
  local check="backups/.check-$target"
  aws "${to[@]}" s3 cp "s3://$target/db/$name" "/b/.check-$target" --only-show-errors || return 1
  cmp -s "$dump.age" "$check" || { rm -f "$check"; echo "backup: the copy read back from $target differs" >&2; return 1; }
  rm -f "$check"
}

sent=0
for prefix in OC_BACKUP_S3 OC_BACKUP2_S3; do
  target=$(v "${prefix}_BUCKET")
  [ -n "$target" ] || continue
  if send_to "$target" "$prefix"; then sent=$((sent + 1)); else fail "sending to bucket $target"; fi
done
[ "$sent" -ge 2 ] || fail "only $sent of 2 off-site buckets hold this backup"

finished=1
if [ "$failures" -gt 0 ]; then
  ping_monitor /fail
  echo "backup: $failures problem(s), $dump $(du -h "$dump" | cut -f1)" >&2
  exit 1
fi
ping_monitor ""
echo "backup: ok $dump $(du -h "$dump" | cut -f1) (restore checked, $sent off-site)"
