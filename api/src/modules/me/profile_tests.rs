use super::*;

async fn install(pool: &PgPool) -> anyhow::Result<()> {
    sqlx::raw_sql(
        "CREATE TABLE user_profiles (
            soundcloud_user_id text PRIMARY KEY, profile_json jsonb NOT NULL, synced_at timestamptz NOT NULL
         );
         CREATE TABLE background_jobs (
            id uuid PRIMARY KEY, kind text NOT NULL, lane text NOT NULL, dedup_key text,
            payload jsonb NOT NULL, priority smallint NOT NULL, max_attempts smallint NOT NULL
         );
         CREATE UNIQUE INDEX job_dedup ON background_jobs (kind, dedup_key) WHERE dedup_key IS NOT NULL;
         CREATE TABLE users (
            sc_user_id text PRIMARY KEY, urn text NOT NULL, username text, full_name text,
            avatar_url text, permalink_url text, followers_count bigint, followings_count bigint,
            tracks_count bigint, playlists_count bigint
         );",
    ).execute(pool).await?;
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn a_stale_profile_is_returned_while_one_durable_refresh_is_coalesced(
    pool: PgPool,
) -> anyhow::Result<()> {
    install(&pool).await?;
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

#[sqlx::test(migrations = false)]
async fn a_fresh_profile_needs_no_refresh_job(pool: PgPool) -> anyhow::Result<()> {
    install(&pool).await?;
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

#[sqlx::test(migrations = false)]
async fn a_missing_profile_returns_the_user_mirror_without_any_external_dependency(
    pool: PgPool,
) -> anyhow::Result<()> {
    install(&pool).await?;
    sqlx::query("INSERT INTO users (sc_user_id, urn, username) VALUES ('17', 'soundcloud:users:17', 'mirrored')")
        .execute(&pool).await?;
    let profile = read(&pool, "17").await?;
    assert_eq!(profile["username"], "mirrored");
    let jobs: i64 = sqlx::query_scalar("SELECT count(*) FROM background_jobs")
        .fetch_one(&pool)
        .await?;
    assert_eq!(jobs, 1);
    Ok(())
}
