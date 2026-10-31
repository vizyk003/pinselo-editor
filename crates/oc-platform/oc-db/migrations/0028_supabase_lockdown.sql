-- Supabase only (plan 12 §A4): the Data API lockdown. Its logic lives in packages/db/lockdown.sql, which the TS
-- migrator (`pnpm db:migrate`) and the Rust one (`oc-migrate`) run right after creating schema_migrations, inside
-- every migration's transaction (this one included) before COMMIT, and again at the end of every run. So the lockdown never depends on this file having run,
-- covers a run that fails half-way, and covers tables from later migrations or from lower-numbered ones merged after
-- this one. It is a no-op on plain Postgres.
-- An earlier version of this file defined public.oc_supabase_lockdown() and called it. Databases that ran that
-- version keep the function (only its owner may execute it); nothing calls it any more.
SELECT 1;
