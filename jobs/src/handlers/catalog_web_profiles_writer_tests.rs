use super::*;
use serde_json::json;

async fn setup(pool: &PgPool) -> anyhow::Result<(CatalogWriter, LeasedJob, CatalogRefreshPayload)> {
    let (writer, job) = super::tests::setup(pool).await?;
    sqlx::raw_sql(include_str!(
        "../../../api/migrations/0099_user_web_profiles.sql"
    ))
    .execute(pool)
    .await?;
    Ok((
        writer,
        job,
        CatalogRefreshPayload {
            entity: CatalogEntity::WebProfiles,
            sc_id: "42".into(),
            owner_id: None,
        },
    ))
}

#[sqlx::test(migrations = false)]
async fn web_profiles_snapshot_ordering_preserves_new_links_until_confirmed_removal(
    pool: PgPool,
) -> anyhow::Result<()> {
    let (writer, job, payload) = setup(&pool).await?;
    let first = writer.begin_observation().await?;
    let intermediate = writer.begin_observation().await?;
    let latest = writer.begin_observation().await?;
    let links = json!([{"url":"https://example.test", "service":"personal"}]);
    writer.persist(&job, &payload, &links, first).await?;
    writer.persist(&job, &payload, &links, latest).await?;
    writer
        .persist(&job, &payload, &json!([]), intermediate)
        .await?;
    let stored: (Value, i64) = sqlx::query_as(
        "SELECT profiles, sc_observation FROM user_web_profiles WHERE sc_user_id = '42'",
    )
    .fetch_one(&pool)
    .await?;
    assert_eq!(stored, (links, latest.sequence()));
    let empty = writer.begin_observation().await?;
    writer.persist(&job, &payload, &json!([]), empty).await?;
    let stored: Value =
        sqlx::query_scalar("SELECT profiles FROM user_web_profiles WHERE sc_user_id = '42'")
            .fetch_one(&pool)
            .await?;
    assert_eq!(stored, json!([]));
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn web_profiles_malformed_responses_and_lost_leases_cannot_replace_the_snapshot(
    pool: PgPool,
) -> anyhow::Result<()> {
    let (writer, job, payload) = setup(&pool).await?;
    let first = writer.begin_observation().await?;
    let links = json!([{"url":"https://example.test"}]);
    writer.persist(&job, &payload, &links, first).await?;
    assert!(
        writer
            .persist(
                &job,
                &payload,
                &json!({"error":"unavailable"}),
                writer.begin_observation().await?
            )
            .await
            .is_err()
    );
    let mut stale = job.clone();
    stale.lease_id = Uuid::new_v4();
    writer
        .persist(
            &stale,
            &payload,
            &json!([]),
            writer.begin_observation().await?,
        )
        .await?;
    stale.lease_id = job.lease_id;
    stale.generation += 1;
    writer
        .persist(
            &stale,
            &payload,
            &json!([]),
            writer.begin_observation().await?,
        )
        .await?;
    sqlx::query("UPDATE background_jobs SET lease_expires_at = now() - interval '1 second'")
        .execute(&pool)
        .await?;
    writer
        .persist(
            &job,
            &payload,
            &json!([]),
            writer.begin_observation().await?,
        )
        .await?;
    let stored: (Value, i64) = sqlx::query_as(
        "SELECT profiles, sc_observation FROM user_web_profiles WHERE sc_user_id = '42'",
    )
    .fetch_one(&pool)
    .await?;
    assert_eq!(stored, (links, first.sequence()));
    Ok(())
}
