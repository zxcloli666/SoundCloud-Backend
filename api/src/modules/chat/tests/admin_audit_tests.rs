use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

use super::audit;
use super::reports::{AUDIT_SHOWN, VIEWS_SHOWN, audit_rows};
use crate::modules::chat::moderation::testing::state;

async fn audited(pool: &PgPool, report: Uuid, action: &str, seconds_ago: f64) {
    sqlx::query(
        "INSERT INTO chat.admin_audit (id, admin, action, target, detail, created_at)
         VALUES ($1, 'owner', $2, $3, $4, now() - $5 * interval '1 second')",
    )
    .bind(Uuid::now_v7())
    .bind(action)
    .bind(audit::report(report))
    .bind(json!({"report": report}))
    .bind(seconds_ago)
    .execute(pool)
    .await
    .expect("an audit row");
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn image_views_never_push_the_verdict_and_actions_out_of_the_trail(pool: PgPool) {
    let chat = state(pool.clone(), None);
    let report = Uuid::now_v7();
    for (action, seconds_ago) in [
        ("verdict", 4_000.0),
        ("decrypt", 3_900.0),
        ("delete_messages", 3_800.0),
        ("restore", 3_700.0),
    ] {
        audited(&pool, report, action, seconds_ago).await;
    }
    for view in 0..300 {
        audited(&pool, report, "view_media", 3_000.0 - f64::from(view)).await;
    }
    audited(&pool, Uuid::now_v7(), "verdict", 10.0).await;

    let trail = audit_rows(&chat, report).await.expect("a trail");
    let actions: Vec<&str> = trail
        .iter()
        .map(|row| row.action.as_str())
        .filter(|action| !matches!(*action, "view_media" | "decrypt"))
        .collect();
    assert_eq!(actions, ["verdict", "delete_messages", "restore"]);
    let seen: Vec<i64> = trail
        .iter()
        .filter(|row| matches!(row.action.as_str(), "view_media" | "decrypt"))
        .map(|row| row.at)
        .collect();
    assert_eq!(seen.len(), VIEWS_SHOWN as usize);
    assert!(
        trail.iter().all(|row| row.action != "decrypt"),
        "the oldest decrypt fell out with the oldest views"
    );
    assert!(trail.windows(2).all(|pair| pair[0].at <= pair[1].at));
    let newest_view = trail.last().expect("a row");
    assert_eq!(newest_view.action, "view_media");
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn the_trail_keeps_the_newest_actions_up_to_its_cap(pool: PgPool) {
    let chat = state(pool.clone(), None);
    let report = Uuid::now_v7();
    let total = AUDIT_SHOWN + 20;
    for at in 0..total {
        audited(&pool, report, "restore", (total - at) as f64).await;
    }
    audited(&pool, report, "view_media", 0.5).await;
    let trail = audit_rows(&chat, report).await.expect("a trail");
    assert_eq!(trail.len(), AUDIT_SHOWN as usize + 1);
    assert_eq!(
        trail.iter().filter(|row| row.action == "restore").count(),
        AUDIT_SHOWN as usize
    );
}
