use super::*;
use axum::response::IntoResponse;
use serde_json::json;

#[sqlx::test(migrations = "./migrations")]
async fn web_profiles_cold_miss_deduplicates_and_preserves_job_cooldown(
    pool: PgPool,
) -> anyhow::Result<()> {
    let first = read(&pool, "42")
        .await
        .err()
        .ok_or_else(|| anyhow::anyhow!("expected cold miss"))?;
    let response = first.into_response();
    assert_eq!(
        response.status(),
        axum::http::StatusCode::SERVICE_UNAVAILABLE
    );
    assert_eq!(response.headers()["retry-after"], "5");
    let body = axum::body::to_bytes(response.into_body(), 4096).await?;
    assert_eq!(
        serde_json::from_slice::<Value>(&body)?["code"],
        "web_profiles_refresh_pending"
    );
    sqlx::query("UPDATE background_jobs SET available_at = now() + interval '10 minutes', attempts = 3 WHERE dedup_key = 'web_profiles:42:public'")
        .execute(&pool).await?;
    let second = read(&pool, "soundcloud:users:42")
        .await
        .err()
        .ok_or_else(|| anyhow::anyhow!("expected cold miss"))?
        .into_response();
    let delay: i64 = second.headers()["retry-after"].to_str()?.parse()?;
    assert!((590..=600).contains(&delay));
    let jobs: Vec<(Value, i32)> = sqlx::query_as(
        "SELECT payload, attempts FROM background_jobs WHERE kind = 'catalog.refresh'",
    )
    .fetch_all(&pool)
    .await?;
    assert_eq!(
        jobs,
        vec![(
            json!({"version":"1", "payload":{"entity":"web_profiles", "sc_id":"42", "owner_id":null}}),
            3
        )]
    );
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn web_profiles_serve_stale_links_and_distinguish_confirmed_empty_snapshots(
    pool: PgPool,
) -> anyhow::Result<()> {
    let links = json!([{"url":"https://example.test", "title":"Website"}]);
    sqlx::query("INSERT INTO user_web_profiles (sc_user_id, profiles, sc_observation, synced_at) VALUES ('42', $1, 1, now() - interval '2 days'), ('43', '[]', 2, now())")
        .bind(&links).execute(&pool).await?;
    assert_eq!(read(&pool, "soundcloud:users:42").await?, links);
    assert_eq!(read(&pool, "43").await?, json!([]));
    let keys: Vec<String> =
        sqlx::query_scalar("SELECT dedup_key FROM background_jobs WHERE kind = 'catalog.refresh'")
            .fetch_all(&pool)
            .await?;
    assert_eq!(keys, vec!["web_profiles:42:public"]);
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn web_profiles_reject_invalid_ids_before_reading_or_enqueuing(
    pool: PgPool,
) -> anyhow::Result<()> {
    for invalid in [
        "soundcloud:tracks:42",
        "42/likes",
        "042",
        "0",
        "-42",
        "soundcloud:users:42:extra",
    ] {
        let response = read(&pool, invalid)
            .await
            .err()
            .ok_or_else(|| anyhow::anyhow!("expected invalid id"))?
            .into_response();
        assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
    }
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM background_jobs")
        .fetch_one(&pool)
        .await?;
    assert_eq!(count, 0);
    Ok(())
}
