use super::*;
use serde_json::json;

pub(super) async fn setup(pool: &PgPool) -> anyhow::Result<(CatalogWriter, LeasedJob)> {
    sqlx::raw_sql(include_str!(
        "../../queries/catalog_refresh/test_schema.sql"
    ))
    .execute(pool)
    .await?;
    let job = LeasedJob {
        id: Uuid::now_v7(),
        kind: JobKind::CatalogRefresh,
        dedup_key: None,
        payload: Value::Null,
        generation: 1,
        attempts: 1,
        max_attempts: 8,
        lease_id: Uuid::new_v4(),
    };
    sqlx::query("INSERT INTO background_jobs VALUES ($1, $2, 1, 1, now() + interval '1 minute')")
        .bind(job.id)
        .bind(job.lease_id)
        .execute(pool)
        .await?;
    Ok((CatalogWriter::new(pool.clone(), 420_000), job))
}

fn payload() -> CatalogRefreshPayload {
    CatalogRefreshPayload {
        entity: CatalogEntity::Profile,
        sc_id: "17".into(),
        owner_id: Some("17".into()),
    }
}

fn profile() -> Value {
    json!({"id": 17, "urn": "soundcloud:users:17", "username": "Alice", "private_field": "owner-only"})
}

#[sqlx::test(migrations = false)]
async fn profile_and_user_are_persisted_under_the_current_lease(
    pool: PgPool,
) -> anyhow::Result<()> {
    let (writer, job) = setup(&pool).await?;
    writer
        .persist(
            &job,
            &payload(),
            &profile(),
            catalog_ingest::Observation::begin(&pool).await?,
        )
        .await?;
    writer
        .persist(
            &job,
            &payload(),
            &profile(),
            catalog_ingest::Observation::begin(&pool).await?,
        )
        .await?;
    let result: (String, Value) = sqlx::query_as("SELECT users.username, profile_json FROM users JOIN user_profiles ON sc_user_id = soundcloud_user_id")
        .fetch_one(&pool).await?;
    assert_eq!(result, ("Alice".into(), profile()));
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn a_late_profile_response_cannot_replace_a_newer_profile_or_user(
    pool: PgPool,
) -> anyhow::Result<()> {
    let (writer, job) = setup(&pool).await?;
    let older = catalog_ingest::Observation::begin(&pool).await?;
    let newer = catalog_ingest::Observation::begin(&pool).await?;
    let mut current = profile();
    current["username"] = json!("Current");
    writer.persist(&job, &payload(), &current, newer).await?;
    writer.persist(&job, &payload(), &profile(), older).await?;
    let stored: (String, Value) = sqlx::query_as(
        "SELECT users.username, profile_json FROM users JOIN user_profiles ON sc_user_id = soundcloud_user_id",
    ).fetch_one(&pool).await?;
    assert_eq!(stored, ("Current".into(), current));
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn an_identical_profile_observation_advances_the_fence_against_intermediate_reads(
    pool: PgPool,
) -> anyhow::Result<()> {
    let (writer, job) = setup(&pool).await?;
    let first = catalog_ingest::Observation::begin(&pool).await?;
    let intermediate = catalog_ingest::Observation::begin(&pool).await?;
    let latest = catalog_ingest::Observation::begin(&pool).await?;
    writer.persist(&job, &payload(), &profile(), first).await?;
    writer.persist(&job, &payload(), &profile(), latest).await?;
    let mut stale = profile();
    stale["username"] = json!("Stale");
    writer
        .persist(&job, &payload(), &stale, intermediate)
        .await?;
    let stored: (String, Value) = sqlx::query_as(
        "SELECT users.username, profile_json FROM users JOIN user_profiles ON sc_user_id = soundcloud_user_id",
    ).fetch_one(&pool).await?;
    assert_eq!(stored, ("Alice".into(), profile()));
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn user_versions_reject_older_sources_and_unverified_payloads(
    pool: PgPool,
) -> anyhow::Result<()> {
    setup(&pool).await?;
    let earlier = catalog_ingest::Observation::begin(&pool).await?;
    let later = catalog_ingest::Observation::begin(&pool).await?;
    let mut older = profile();
    older["last_modified"] = json!("2026-09-01T00:00:00Z");
    let mut newer = older.clone();
    newer["username"] = json!("Newer source");
    newer["last_modified"] = json!("2026-09-02T00:00:00Z");
    catalog_ingest::upsert_user_from_sc(&pool, &older, later).await?;
    catalog_ingest::upsert_user_from_sc(&pool, &newer, earlier).await?;
    catalog_ingest::upsert_user_from_sc(
        &pool,
        &older,
        catalog_ingest::Observation::begin(&pool).await?,
    )
    .await?;
    let mut unverified = profile();
    unverified["last_modified"] = json!("2026-09-03T00:00:00Z");
    catalog_ingest::upsert_user_from_sc(
        &pool,
        &unverified,
        catalog_ingest::Observation::UNVERIFIED,
    )
    .await?;
    let stored: String = sqlx::query_scalar("SELECT username FROM users WHERE sc_user_id = '17'")
        .fetch_one(&pool)
        .await?;
    assert_eq!(stored, "Newer source");
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn an_owner_profile_write_fences_a_public_read_started_before_login(
    pool: PgPool,
) -> anyhow::Result<()> {
    let (writer, job) = setup(&pool).await?;
    let earlier = catalog_ingest::Observation::begin(&pool).await?;
    let login = catalog_ingest::Observation::begin(&pool).await?;
    let mut current = profile();
    current["username"] = json!("After login");
    let mut transaction = pool.begin().await?;
    catalog_ingest::upsert_profile_in(&mut transaction, "17", &current, login).await?;
    transaction.commit().await?;
    let mut public = payload();
    public.entity = CatalogEntity::User;
    public.owner_id = None;
    writer.persist(&job, &public, &profile(), earlier).await?;
    let stored: (String, Value) = sqlx::query_as(
        "SELECT users.username, profile_json FROM users JOIN user_profiles ON sc_user_id = soundcloud_user_id",
    ).fetch_one(&pool).await?;
    assert_eq!(stored, ("After login".into(), current));
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn a_thin_login_profile_preserves_absent_fields_but_can_clear_explicit_fields(
    pool: PgPool,
) -> anyhow::Result<()> {
    setup(&pool).await?;
    let mut complete = profile();
    complete["avatar_url"] = json!("https://example.test/avatar.png");
    complete["verified"] = json!(true);
    catalog_ingest::upsert_user_from_sc(
        &pool,
        &complete,
        catalog_ingest::Observation::begin(&pool).await?,
    )
    .await?;
    let mut transaction = pool.begin().await?;
    catalog_ingest::upsert_profile_in(
        &mut transaction,
        "17",
        &profile(),
        catalog_ingest::Observation::begin(&pool).await?,
    )
    .await?;
    transaction.commit().await?;
    let stored: (Option<String>, bool) =
        sqlx::query_as("SELECT avatar_url, verified FROM users WHERE sc_user_id = '17'")
            .fetch_one(&pool)
            .await?;
    assert_eq!(
        stored,
        (Some("https://example.test/avatar.png".into()), true)
    );
    let mut cleared = profile();
    cleared["avatar_url"] = Value::Null;
    cleared["verified"] = json!(false);
    catalog_ingest::upsert_user_from_sc(
        &pool,
        &cleared,
        catalog_ingest::Observation::begin(&pool).await?,
    )
    .await?;
    let stored: (Option<String>, bool) =
        sqlx::query_as("SELECT avatar_url, verified FROM users WHERE sc_user_id = '17'")
            .fetch_one(&pool)
            .await?;
    assert_eq!(stored, (None, false));
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn malformed_profiles_fail_without_persisting_either_projection(
    pool: PgPool,
) -> anyhow::Result<()> {
    let (writer, job) = setup(&pool).await?;
    let mut invalid = profile();
    invalid.as_object_mut().unwrap().remove("username");
    assert!(
        writer
            .persist(
                &job,
                &payload(),
                &invalid,
                catalog_ingest::Observation::begin(&pool).await?,
            )
            .await
            .is_err()
    );
    let count: i64 = sqlx::query_scalar(
        "SELECT (SELECT count(*) FROM users) + (SELECT count(*) FROM user_profiles)",
    )
    .fetch_one(&pool)
    .await?;
    assert_eq!(count, 0);
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn lost_or_expired_leases_cannot_persist_catalog_data(pool: PgPool) -> anyhow::Result<()> {
    let (writer, mut job) = setup(&pool).await?;
    let original_lease = job.lease_id;
    job.lease_id = Uuid::new_v4();
    writer
        .persist(
            &job,
            &payload(),
            &profile(),
            catalog_ingest::Observation::begin(&pool).await?,
        )
        .await?;
    job.lease_id = original_lease;
    job.generation = 2;
    writer
        .persist(
            &job,
            &payload(),
            &profile(),
            catalog_ingest::Observation::begin(&pool).await?,
        )
        .await?;
    job.generation = 1;
    sqlx::query("UPDATE background_jobs SET generation = 2")
        .execute(&pool)
        .await?;
    writer
        .persist(
            &job,
            &payload(),
            &profile(),
            catalog_ingest::Observation::begin(&pool).await?,
        )
        .await?;
    sqlx::query(
        "UPDATE background_jobs SET generation = 1, lease_expires_at = now() - interval '1 second'",
    )
    .execute(&pool)
    .await?;
    writer
        .persist(
            &job,
            &payload(),
            &profile(),
            catalog_ingest::Observation::begin(&pool).await?,
        )
        .await?;
    let count: i64 = sqlx::query_scalar(
        "SELECT (SELECT count(*) FROM users) + (SELECT count(*) FROM user_profiles)",
    )
    .fetch_one(&pool)
    .await?;
    assert_eq!(count, 0);
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn profile_write_failure_rolls_back_the_user_upsert(pool: PgPool) -> anyhow::Result<()> {
    let (writer, job) = setup(&pool).await?;
    sqlx::query("ALTER TABLE user_profiles ADD CHECK (soundcloud_user_id <> '17')")
        .execute(&pool)
        .await?;
    assert!(
        writer
            .persist(
                &job,
                &payload(),
                &profile(),
                catalog_ingest::Observation::begin(&pool).await?
            )
            .await
            .is_err()
    );
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM users")
        .fetch_one(&pool)
        .await?;
    assert_eq!(count, 0);
    Ok(())
}
