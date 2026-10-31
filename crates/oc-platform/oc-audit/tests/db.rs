//! DB round-trip (skipped unless OC_DATABASE_URL is set; runs in a throwaway database): rows written by the former
//! TS service (tests/golden/platform/audit.json) + Rust-appended rows form one chain the verifier accepts, and it flags
//! a tampered row.
use oc_audit::*;
use oc_db::testing::TestDb;
use serde_json::{Value, json};
use uuid::Uuid;

fn golden_rows() -> Vec<AuditRow> {
    let p = concat!(env!("CARGO_MANIFEST_DIR"), "/../../../tests/golden/platform/audit.json");
    let v: Value = serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap();
    serde_json::from_value(v["rows"].clone()).unwrap()
}

#[tokio::test]
async fn mixed_chain_verifies() {
    let Some(t) = TestDb::create().await else {
        eprintln!("skipped: OC_DATABASE_URL not set");
        return;
    };
    let pool = &t.pool;
    // A pre-chain row (written before chain v2) must be ignored by both verifiers.
    sqlx::query("INSERT INTO audit_events(id, action) VALUES (1, 'legacy')").execute(pool).await.unwrap();
    for r in golden_rows() {
        let f = &r.fields;
        sqlx::query("INSERT INTO audit_events(id, workspace_id, actor_id, action, target, meta, ip, created_at, prev_hash, hash) VALUES ($1,$2,$3,$4,$5,$6::jsonb,$7,$8,$9,$10)")
            .bind(r.id + 100)
            .bind(f.workspace_id)
            .bind(f.actor_id)
            .bind(&f.action)
            .bind(&f.target)
            .bind(f.meta.as_ref().map(|m| m.to_string()))
            .bind(&f.ip)
            .bind(f.created_at)
            .bind(&r.prev_hash)
            .bind(&r.hash)
            .execute(pool)
            .await
            .unwrap();
    }
    sqlx::query("SELECT setval(pg_get_serial_sequence('audit_events','id'), (SELECT max(id) FROM audit_events))").execute(pool).await.unwrap();
    assert_eq!(verify_audit_chain(pool).await.unwrap(), None, "TS-written chain read back from Postgres");

    let events = [
        AuditEvent { action: "rust.login".into(), actor_id: Some(Uuid::new_v4()), ip: Some("198.51.100.1".into()), ..Default::default() },
        AuditEvent {
            action: "rust.meta".into(),
            workspace_id: Some(Uuid::new_v4()),
            target: Some("design:x".into()),
            meta: Some(json!({"f": 0.1, "big": 1e21, "tiny": 5e-324, "int": 9007199254740993_u64, "s": "q\"\\\n\u{1}✨", "\u{1F600}": 1, "\u{FF61}": 2, "nested": [{"b": null, "a": [true, false]}]})),
            ..Default::default()
        },
        AuditEvent { action: "rust.null-meta".into(), meta: Some(Value::Null), ..Default::default() },
        AuditEvent { action: "rust.scalar".into(), meta: Some(json!(42.5)), target: Some(String::new()), ..Default::default() },
    ];
    for e in events {
        audit(pool, e).await.unwrap();
    }
    let chain = load_chain(pool).await.unwrap();
    assert_eq!(chain.len(), golden_rows().len() + 4);
    assert_eq!(verify_audit_chain(pool).await.unwrap(), None, "Rust-appended rows");

    // Tamper with a Rust-written row: the verifier must point at it.
    let victim = chain[chain.len() - 3].id;
    sqlx::query("UPDATE audit_events SET meta = jsonb_set(meta, '{f}', '0.2') WHERE id = $1").bind(victim).execute(pool).await.unwrap();
    assert_eq!(verify_audit_chain(pool).await.unwrap(), Some(victim));
    t.drop_db().await;
}
