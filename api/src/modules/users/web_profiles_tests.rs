use super::*;
use axum::response::IntoResponse;
use serde_json::json;

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn web_profiles_cold_miss_deduplicates_and_preserves_job_cooldown(
    pool: PgPool,
) -> anyhow::Result<()> {
    let first = read(&pool, "7", "42")
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
    sqlx::query("UPDATE job_status SET available_at = now() + interval '10 minutes' WHERE dedup_key = 'web_profiles:42:public'")
        .execute(&pool).await?;
    let second = read(&pool, "7", "soundcloud:users:42")
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

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn web_profiles_serve_stale_links_and_distinguish_confirmed_empty_snapshots(
    pool: PgPool,
) -> anyhow::Result<()> {
    let links = json!([{"url":"https://example.test", "title":"Website"}]);
    sqlx::query("INSERT INTO user_web_profiles (sc_user_id, profiles, sc_observation, synced_at) VALUES ('42', $1, 1, now() - interval '2 days'), ('43', '[]', 2, now())")
        .bind(&links).execute(&pool).await?;
    assert_eq!(read(&pool, "7", "soundcloud:users:42").await?, links);
    assert_eq!(read(&pool, "7", "43").await?, json!([]));
    let keys: Vec<String> =
        sqlx::query_scalar("SELECT dedup_key FROM background_jobs WHERE kind = 'catalog.refresh'")
            .fetch_all(&pool)
            .await?;
    assert_eq!(keys, vec!["web_profiles:42:public"]);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
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
        let response = read(&pool, "7", invalid)
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

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn a_star_reader_lifts_the_refresh_it_triggers(pool: PgPool) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO subscriptions (user_urn, exp_date) VALUES
             ('7', extract(epoch FROM now())::bigint + 3600),
             ('8', extract(epoch FROM now())::bigint - 3600)",
    )
    .execute(&pool)
    .await?;

    for (reader, profile) in [("8", "41"), ("soundcloud:users:7", "42"), ("9", "43")] {
        let _ = read(&pool, reader, profile).await;
    }
    let _ = read(&pool, "9", "42").await;

    let priorities: Vec<(String, i16)> =
        sqlx::query_as("SELECT dedup_key, priority FROM background_jobs ORDER BY dedup_key")
            .fetch_all(&pool)
            .await?;
    assert_eq!(
        priorities,
        [
            ("web_profiles:41:public".into(), 5),
            ("web_profiles:42:public".into(), 10),
            ("web_profiles:43:public".into(), 5),
        ]
    );
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn a_user_soundcloud_called_absent_is_not_found_until_the_hold_runs_out(
    pool: PgPool,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO background_job_failures (
             id, kind, lane, dedup_key, payload, priority, generation, attempts, max_attempts,
             last_error, created_at, failed_at
         ) VALUES
             ($1, 'catalog.refresh', 'core_bulk', 'web_profiles:42:public', '{}', 5, 1, 1, 8,
              'not_found: SoundCloud API error (status 404)',
              now() - interval '3 hours', now() - interval '3 hours'),
             ($2, 'catalog.refresh', 'core_bulk', 'web_profiles:43:public', '{}', 5, 1, 1, 8,
              'not_found: SoundCloud API error (status 404)',
              now() - interval '25 hours', now() - interval '25 hours')",
    )
    .bind(uuid::Uuid::now_v7())
    .bind(uuid::Uuid::now_v7())
    .execute(&pool)
    .await?;

    let held = read(&pool, "7", "42")
        .await
        .err()
        .ok_or_else(|| anyhow::anyhow!("expected a held user"))?
        .into_response();
    let asked_again = read(&pool, "7", "soundcloud:users:43")
        .await
        .err()
        .ok_or_else(|| anyhow::anyhow!("expected a refresh"))?
        .into_response();

    assert_eq!(held.status(), axum::http::StatusCode::NOT_FOUND);
    assert_eq!(
        asked_again.status(),
        axum::http::StatusCode::SERVICE_UNAVAILABLE
    );
    let keys: Vec<String> =
        sqlx::query_scalar("SELECT dedup_key FROM background_jobs WHERE kind = 'catalog.refresh'")
            .fetch_all(&pool)
            .await?;
    assert_eq!(keys, vec!["web_profiles:43:public"]);
    Ok(())
}
