use super::*;

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn a_stale_profile_is_returned_while_one_durable_refresh_is_coalesced(
    pool: PgPool,
) -> anyhow::Result<()> {
    let profile =
        json!({ "id": 17, "username": "local profile", "private_note": "never put in a job" });
    sqlx::query("INSERT INTO user_profiles VALUES ('17', $1, now() - interval '1 hour')")
        .bind(&profile)
        .execute(&pool)
        .await?;

    let (first, second) = tokio::try_join!(read(&pool, "17"), read(&pool, "soundcloud:users:17"))?;
    assert_eq!(first, profile);
    assert_eq!(second, profile);
    let jobs: Vec<(String, String, Value)> =
        sqlx::query_as("SELECT kind, dedup_key, payload FROM background_jobs")
            .fetch_all(&pool)
            .await?;
    assert_eq!(
        jobs,
        vec![(
            "catalog.refresh".into(),
            "profile:17:17".into(),
            json!({"version": "1", "payload": {"entity": "profile", "sc_id": "17", "owner_id": "17"}}),
        )]
    );
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn a_fresh_profile_needs_no_refresh_job(pool: PgPool) -> anyhow::Result<()> {
    let profile = json!({"id": 17, "username": "fresh"});
    sqlx::query("INSERT INTO user_profiles VALUES ('17', $1, now())")
        .bind(&profile)
        .execute(&pool)
        .await?;
    assert_eq!(read(&pool, "17").await?, profile);
    let jobs: i64 = sqlx::query_scalar("SELECT count(*) FROM background_jobs")
        .fetch_one(&pool)
        .await?;
    assert_eq!(jobs, 0);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn a_missing_profile_returns_the_user_mirror_without_any_external_dependency(
    pool: PgPool,
) -> anyhow::Result<()> {
    sqlx::query("INSERT INTO users (sc_user_id, urn, username, username_normalized) VALUES ('17', 'soundcloud:users:17', 'mirrored', 'mirrored')")
        .execute(&pool).await?;
    let profile = read(&pool, "17").await?;
    assert_eq!(profile["username"], "mirrored");
    let jobs: i64 = sqlx::query_scalar("SELECT count(*) FROM background_jobs")
        .fetch_one(&pool)
        .await?;
    assert_eq!(jobs, 1);
    Ok(())
}

async fn seed_both(pool: &PgPool, profile_ago: &str, user_ago: &str) -> anyhow::Result<()> {
    let profile = json!({"id": 17, "username": "old name", "avatar_url": "https://i1.sndcdn.com/old.jpg", "plan": "Pro"});
    sqlx::query("INSERT INTO user_profiles VALUES ('17', $1, now() - $2::interval)")
        .bind(&profile)
        .bind(profile_ago)
        .execute(pool)
        .await?;
    sqlx::query(
        "INSERT INTO users (sc_user_id, urn, username, username_normalized, avatar_url, sc_synced_at)
         VALUES ('17', 'soundcloud:users:17', 'new name', 'new name', 'https://i1.sndcdn.com/new.jpg', now() - $1::interval)",
    )
    .bind(user_ago)
    .execute(pool)
    .await?;
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn a_fresher_public_user_overrides_the_owner_snapshot_and_keeps_private_fields(
    pool: PgPool,
) -> anyhow::Result<()> {
    seed_both(&pool, "1 hour", "1 minute").await?;
    let profile = read(&pool, "17").await?;
    assert_eq!(profile["avatar_url"], "https://i1.sndcdn.com/new.jpg");
    assert_eq!(profile["username"], "new name");
    assert_eq!(profile["plan"], "Pro");
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn an_older_public_user_leaves_the_owner_snapshot_alone(pool: PgPool) -> anyhow::Result<()> {
    seed_both(&pool, "1 minute", "1 hour").await?;
    let profile = read(&pool, "17").await?;
    assert_eq!(profile["avatar_url"], "https://i1.sndcdn.com/old.jpg");
    assert_eq!(profile["username"], "old name");
    Ok(())
}
