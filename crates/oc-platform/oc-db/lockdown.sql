-- The Supabase Data API lockdown (plan 12 §A4). This file is the only copy: the TS migrator
-- (packages/db/src/migrations.ts, `pnpm db:migrate`) runs its text verbatim, right after creating schema_migrations
-- (same transaction), inside every migration's transaction before COMMIT, and once more when a run ends, also after
-- a failed one.
-- The Rust migrator (crates/oc-platform/oc-db/src/migrate.rs, `oc-migrate`) compiles this file in (include_str!) and
-- runs it at the same three points, so either migrator may be used on Supabase (docs/supabase.md).
--
-- Supabase's Data API (PostgREST) serves the public schema to the anon and authenticated roles, and Supabase's
-- default privileges grant them every new table, sequence and function. OpenCanvas never uses the Data API (the app
-- connects as the owner of its tables). So, for every object in public that the current role can act for (it owns it,
-- directly or through membership), this revokes all privileges of PUBLIC, anon, authenticated and service_role,
-- column grants included (CASCADE: also what an API role passed on with a grant option), revokes EXECUTE on functions and procedures (SECURITY DEFINER ones too) and enables RLS
-- without policies on tables (owners bypass RLS; the API roles see nothing). It also stops the default grants on
-- objects the current role creates later. Extension members are left alone.
--
-- Self-contained (it needs no migration to have run) and idempotent; it only touches objects that still grant
-- something or lack RLS. A no-op unless the database looks like Supabase: the auth schema and the anon and
-- authenticated roles exist. Objects it cannot act for are named in a WARNING, and so are objects that still grant
-- an API role something afterwards (another role passed it on with a grant option, which only that role's grant
-- chain can revoke). `pnpm db:migrate` prints the exposure report after every run.
DO $oc_lockdown$
DECLARE
  saved_path text := pg_catalog.current_setting('search_path');
  api_roles text;
  grantees oid[];
  o record;
  skipped text[] := '{}';
  leftover text[] := '{}';
BEGIN
  IF pg_catalog.to_regnamespace('auth') IS NULL
     OR pg_catalog.to_regnamespace('public') IS NULL
     OR NOT EXISTS (SELECT 1 FROM pg_catalog.pg_roles WHERE rolname = 'anon')
     OR NOT EXISTS (SELECT 1 FROM pg_catalog.pg_roles WHERE rolname = 'authenticated') THEN
    RETURN;
  END IF;
  -- Catalog names only, and regclass/regprocedure print schema-qualified. Restored below, so the rest of the
  -- transaction (the next migration, when this runs in a hand-made script) keeps its search_path.
  PERFORM pg_catalog.set_config('search_path', 'pg_catalog, pg_temp', true);

  SELECT string_agg(quote_ident(rolname), ', ' ORDER BY rolname), array_agg(oid) || 0::oid  -- grantee 0 is PUBLIC
    INTO api_roles, grantees
    FROM pg_roles WHERE rolname IN ('anon', 'authenticated', 'service_role');

  -- Tables, partitioned tables, views, materialized views, foreign tables and sequences. A table-level REVOKE also
  -- removes the column grants.
  FOR o IN
    SELECT c.oid::regclass AS obj, c.relkind,
           pg_has_role(c.relowner, 'USAGE') AS mine,
           c.relkind IN ('r', 'p') AND NOT c.relrowsecurity AS needs_rls,
           EXISTS (SELECT 1 FROM aclexplode(c.relacl) a WHERE a.grantee = ANY (grantees))
             OR EXISTS (SELECT 1 FROM pg_attribute att, aclexplode(att.attacl) a
                         WHERE att.attrelid = c.oid AND att.attacl IS NOT NULL AND a.grantee = ANY (grantees)) AS granted
      FROM pg_class c
     WHERE c.relnamespace = 'public'::regnamespace AND c.relkind IN ('r', 'p', 'v', 'm', 'f', 'S')
     ORDER BY 1
  LOOP
    CONTINUE WHEN NOT (o.granted OR o.needs_rls);
    IF NOT o.mine THEN
      skipped := skipped || o.obj::text;
      CONTINUE;
    END IF;
    IF o.granted THEN
      EXECUTE format('REVOKE ALL ON %s %s FROM PUBLIC, %s CASCADE', CASE o.relkind WHEN 'S' THEN 'SEQUENCE' ELSE 'TABLE' END, o.obj, api_roles);
      IF EXISTS (SELECT 1 FROM pg_class c, aclexplode(c.relacl) a WHERE c.oid = o.obj AND a.grantee = ANY (grantees))
         OR EXISTS (SELECT 1 FROM pg_attribute att, aclexplode(att.attacl) a
                     WHERE att.attrelid = o.obj AND att.attacl IS NOT NULL AND a.grantee = ANY (grantees)) THEN
        leftover := leftover || o.obj::text;
      END IF;
    END IF;
    IF o.needs_rls THEN
      EXECUTE format('ALTER TABLE %s ENABLE ROW LEVEL SECURITY', o.obj);
    END IF;
  END LOOP;

  -- Functions, procedures and aggregates. A NULL ACL is the default, which lets PUBLIC execute.
  FOR o IN
    SELECT p.oid::regprocedure AS obj, pg_has_role(p.proowner, 'USAGE') AS mine
      FROM pg_proc p
     WHERE p.pronamespace = 'public'::regnamespace
       AND (p.proacl IS NULL OR EXISTS (SELECT 1 FROM aclexplode(p.proacl) a WHERE a.grantee = ANY (grantees)))
       AND NOT EXISTS (SELECT 1 FROM pg_depend d WHERE d.classid = 'pg_proc'::regclass AND d.objid = p.oid AND d.deptype = 'e')
     ORDER BY 1
  LOOP
    IF NOT o.mine THEN
      skipped := skipped || o.obj::text;
      CONTINUE;
    END IF;
    EXECUTE format('REVOKE ALL ON ROUTINE %s FROM PUBLIC, %s CASCADE', o.obj, api_roles);
    IF EXISTS (SELECT 1 FROM pg_proc p, aclexplode(p.proacl) a WHERE p.oid = o.obj AND a.grantee = ANY (grantees)) THEN
      leftover := leftover || o.obj::text;
    END IF;
  END LOOP;

  -- Default privileges belong to the creating role: these cover objects the current role (postgres on Supabase)
  -- creates later. Supabase's grants to the API roles are per-schema entries. PUBLIC's EXECUTE on new functions is a
  -- built-in global default that only a global entry can remove (it also covers this role's functions elsewhere).
  EXECUTE format('ALTER DEFAULT PRIVILEGES IN SCHEMA public REVOKE ALL ON TABLES FROM %s', api_roles);
  EXECUTE format('ALTER DEFAULT PRIVILEGES IN SCHEMA public REVOKE ALL ON SEQUENCES FROM %s', api_roles);
  EXECUTE format('ALTER DEFAULT PRIVILEGES IN SCHEMA public REVOKE ALL ON FUNCTIONS FROM %s', api_roles);
  ALTER DEFAULT PRIVILEGES REVOKE EXECUTE ON FUNCTIONS FROM PUBLIC;

  PERFORM pg_catalog.set_config('search_path', saved_path, true);
  IF cardinality(skipped) > 0 THEN
    RAISE WARNING 'Supabase lockdown: % object(s) in schema public belong to roles that % cannot act for, so their Data API access was left as it is: %',
      cardinality(skipped), current_user,
      array_to_string(skipped[1:10], ', ') || CASE WHEN cardinality(skipped) > 10 THEN ', ...' ELSE '' END
      USING HINT = 'Run the migrations as the role that owns them, or revoke their grants and enable RLS by hand.';
  END IF;
  IF cardinality(leftover) > 0 THEN
    RAISE WARNING 'Supabase lockdown: % object(s) in schema public still grant Data API roles access that another role passed on with a grant option, which % cannot revoke: %',
      cardinality(leftover), current_user,
      array_to_string(leftover[1:10], ', ') || CASE WHEN cardinality(leftover) > 10 THEN ', ...' ELSE '' END
      USING HINT = 'Revoke the grant option from the role that passed it on (REVOKE GRANT OPTION FOR ALL ON ... FROM that_role CASCADE), or revoke as that role.';
  END IF;
END
$oc_lockdown$;
