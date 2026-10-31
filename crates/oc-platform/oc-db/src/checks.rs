//! Startup self-check (port of `packages/db/src/checks.ts`, plan 12 §A5), also printed by `oc-migrate` after every
//! run. On Supabase, the Data API serves the public schema to the anon and authenticated roles; the lockdown
//! (`crates/oc-platform/oc-db/lockdown.sql`, run by the migrator) revokes their grants and turns on RLS. This reports anything
//! still reachable through it, and refuses to run when RLS would hide rows from the app itself (the app must connect
//! as the tables' owner).

use std::time::Duration;

use sqlx::PgPool;

/// One object a Data API role can reach.
#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct Exposed {
    pub object: String,
    /// `table`, `sequence` or `function`.
    pub kind: String,
    pub role: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExposureReport {
    /// auth schema + anon + authenticated present.
    pub supabase: bool,
    pub exposed: Vec<Exposed>,
    /// Public tables with RLS off (not exposed if `exposed` is empty).
    pub no_rls: Vec<String>,
    /// RLS on, but current_user neither owns the table (pg_has_role … 'USAGE') nor has BYPASSRLS.
    pub rls_blocks_app: Vec<String>,
}

/// Reads the report for the database behind `pool`.
pub async fn check_database_exposure(pool: &PgPool) -> Result<ExposureReport, sqlx::Error> {
    let (auth, roles): (bool, Vec<String>) = sqlx::query_as(
        "SELECT to_regnamespace('auth') IS NOT NULL AS auth,
                ARRAY(SELECT rolname::text FROM pg_roles WHERE rolname IN ('anon', 'authenticated') ORDER BY rolname) AS roles",
    )
    .fetch_one(pool)
    .await?;
    let supabase = auth && roles.len() == 2;

    let rls_blocks_app: Vec<String> = sqlx::query_scalar(
        "SELECT c.oid::regclass::text AS rel
           FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace
          WHERE n.nspname = 'public' AND c.relkind IN ('r', 'p') AND c.relrowsecurity
            AND NOT EXISTS (SELECT 1 FROM pg_roles r WHERE r.rolname = current_user AND (r.rolsuper OR r.rolbypassrls))
            AND (c.relforcerowsecurity OR NOT pg_has_role(current_user, c.relowner, 'USAGE'))
          ORDER BY 1",
    )
    .fetch_all(pool)
    .await?;
    if !supabase {
        return Ok(ExposureReport { supabase, exposed: Vec::new(), no_rls: Vec::new(), rls_blocks_app });
    }

    let exposed: Vec<Exposed> = sqlx::query_as(
        "WITH api(role) AS (SELECT unnest($1::text[]))
         SELECT c.oid::regclass::text AS object, 'table' AS kind, api.role
           FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace, api
          WHERE n.nspname = 'public' AND c.relkind IN ('r', 'p', 'v', 'm', 'f')
            AND (has_table_privilege(api.role, c.oid, 'SELECT, INSERT, UPDATE, DELETE, TRUNCATE, REFERENCES, TRIGGER')
                 OR has_any_column_privilege(api.role, c.oid, 'SELECT, INSERT, UPDATE, REFERENCES'))
         UNION ALL
         SELECT c.oid::regclass::text, 'sequence', api.role
           FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace, api
          WHERE n.nspname = 'public' AND c.relkind = 'S' AND has_sequence_privilege(api.role, c.oid, 'USAGE, SELECT, UPDATE')
         UNION ALL
         SELECT p.oid::regprocedure::text, 'function', api.role
           FROM pg_proc p JOIN pg_namespace n ON n.oid = p.pronamespace, api
          WHERE n.nspname = 'public' AND has_function_privilege(api.role, p.oid, 'EXECUTE')
            AND NOT EXISTS (SELECT 1 FROM pg_depend d WHERE d.classid = 'pg_proc'::regclass AND d.objid = p.oid AND d.deptype = 'e')
          ORDER BY 2, 1, 3",
    )
    .bind(&roles)
    .fetch_all(pool)
    .await?;

    let no_rls: Vec<String> = sqlx::query_scalar(
        "SELECT c.oid::regclass::text AS rel FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace
          WHERE n.nspname = 'public' AND c.relkind IN ('r', 'p') AND NOT c.relrowsecurity ORDER BY 1",
    )
    .fetch_all(pool)
    .await?;

    Ok(ExposureReport { supabase, exposed, no_rls, rls_blocks_app })
}

/// Where the report is printed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckContext {
    Startup,
    Migrate,
}

fn list(xs: &[String]) -> String {
    const MAX: usize = 10;
    let mut s = xs.iter().take(MAX).cloned().collect::<Vec<_>>().join(", ");
    if xs.len() > MAX {
        s.push_str(&format!(", … ({} more)", xs.len() - MAX));
    }
    s
}

/// The lines [`report_exposure`] prints, as `(is_error_stream, line)` with `console.error`/`console.warn` on stderr
/// and `console.log` on stdout, plus whether the caller must exit non-zero: always when RLS would hide rows from the
/// app (startup only; for the migrator it is a warning about its own role), and in production when anything is
/// exposed.
pub fn exposure_messages(service: &str, r: &ExposureReport, context: CheckContext, production: bool) -> (Vec<(bool, String)>, bool) {
    let startup = context == CheckContext::Startup;
    let mut out = Vec::new();
    let mut fatal = false;
    if !r.rls_blocks_app.is_empty() {
        if startup {
            fatal = true;
        }
        out.push((
            true,
            format!(
                "[{service}] Row-level security would hide rows from {} on: {}. Connect as the role that owns the tables (the one that ran the migrations).",
                if startup { "Pinselo" } else { "this role" },
                list(&r.rls_blocks_app)
            ),
        ));
    }
    if !r.exposed.is_empty() {
        if production {
            fatal = true;
        }
        let what: Vec<String> = r.exposed.iter().map(|x| format!("{} {} ({})", x.kind, x.object, x.role)).collect();
        let advice = if startup {
            "Run `pnpm db:migrate` as the role that owns them: every run re-applies the Supabase lockdown. Access that another role passed on with a grant option stays until that role (or its grant option) is revoked; `db:migrate` warns about it."
        } else {
            "The lockdown could not remove this access: the objects belong to another role, another role passed the access on with a grant option, or the API roles reach them through a role they are members of. Revoke it by hand (as the owning or granting role), or migrate as the owning role."
        };
        out.push((true, format!("[{service}] Supabase Data API roles can reach Pinselo objects: {}. {advice}", list(&what))));
    }
    if r.supabase && !r.no_rls.is_empty() {
        out.push((true, format!("[{service}] Row-level security is off on: {}. Run pnpm db:migrate as the role that owns the tables (every run re-applies the Supabase lockdown).", list(&r.no_rls))));
    }
    if !fatal && r.supabase && r.exposed.is_empty() && r.no_rls.is_empty() {
        out.push((false, format!("[{service}] Supabase exposure check: OK")));
    } else if !r.supabase && context == CheckContext::Migrate {
        out.push((false, format!("[{service}] Supabase exposure check: not a Supabase database, nothing to lock down")));
    }
    (out, fatal)
}

/// Prints the report under `[service]` and returns whether the caller must exit non-zero (see
/// [`exposure_messages`]).
pub fn report_exposure(service: &str, r: &ExposureReport, context: CheckContext, production: bool) -> bool {
    let (lines, fatal) = exposure_messages(service, r, context, production);
    for (err, line) in lines {
        if err {
            eprintln!("{line}");
        } else {
            println!("{line}");
        }
    }
    fatal
}

/// Prints the report; exits 1 when `rls_blocks_app` is non-empty (always) or `exposed` is non-empty (production).
/// An unreachable database (or a check slower than 10 s) only warns: the service starts and requests fail until it
/// is back.
pub async fn startup_checks(service: &str, pool: &PgPool) {
    let r = match tokio::time::timeout(Duration::from_secs(10), check_database_exposure(pool)).await {
        Ok(Ok(r)) => r,
        Ok(Err(e)) => return eprintln!("[{service}] database exposure check skipped: {e}"),
        Err(_) => return eprintln!("[{service}] database exposure check skipped: timed out after 10 s"),
    };
    if report_exposure(service, &r, CheckContext::Startup, oc_config::config().env == "production") {
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages() {
        let plain = ExposureReport::default();
        let (l, fatal) = exposure_messages("migrate", &plain, CheckContext::Migrate, true);
        assert_eq!(l, vec![(false, "[migrate] Supabase exposure check: not a Supabase database, nothing to lock down".to_owned())]);
        assert!(!fatal);
        assert!(exposure_messages("api", &plain, CheckContext::Startup, true).0.is_empty());

        let ok = ExposureReport { supabase: true, ..Default::default() };
        assert_eq!(exposure_messages("migrate", &ok, CheckContext::Migrate, true).0, vec![(false, "[migrate] Supabase exposure check: OK".to_owned())]);

        let exposed = ExposureReport {
            supabase: true,
            exposed: (0..12).map(|i| Exposed { object: format!("t{i:02}"), kind: "table".into(), role: "anon".into() }).collect(),
            no_rls: vec!["t00".into()],
            rls_blocks_app: vec![],
        };
        let (l, fatal) = exposure_messages("migrate", &exposed, CheckContext::Migrate, false);
        assert!(!fatal);
        assert_eq!(l.len(), 2);
        assert!(l[0].1.starts_with("[migrate] Supabase Data API roles can reach Pinselo objects: table t00 (anon), "));
        assert!(l[0].1.contains("table t09 (anon), … (2 more). The lockdown could not remove this access"));
        assert!(exposure_messages("migrate", &exposed, CheckContext::Migrate, true).1);

        let blocks = ExposureReport { rls_blocks_app: vec!["public.users".into()], ..Default::default() };
        assert!(exposure_messages("api", &blocks, CheckContext::Startup, false).1);
        let (l, fatal) = exposure_messages("migrate", &blocks, CheckContext::Migrate, false);
        assert!(!fatal);
        assert!(l[0].1.contains("hide rows from this role on: public.users"));
    }
}
