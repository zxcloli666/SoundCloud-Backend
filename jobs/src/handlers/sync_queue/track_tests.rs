use serde_json::{Value, json};
use sqlx::PgPool;
use std::time::Duration;

use super::repository::SyncQueueRepository;

async fn update(pool: &PgPool, body: &Value) -> anyhow::Result<()> {
    let patch = catalog_ingest::TrackUpdate::parse(body).map_err(anyhow::Error::msg)?;
    let mut tx = pool.begin().await?;
    let edit = wt::TrackEdit {
        sc_track_id: "42".to_owned(),
        uploader: "17".to_owned(),
        desired: patch.desired().clone(),
        observation: catalog_ingest::Observation::begin(pool).await?.sequence(),
        at: wt::now(),
    };
    anyhow::ensure!(wt::call(&mut tx, &edit).await?.is_some());
    sqlx::query_file!(
        "../api/queries/sync_queue/service/enqueue.sql",
        "17",
        "track_update",
        "soundcloud:tracks:42",
        body,
        false
    )
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

pub(super) async fn connect_owner(pool: &PgPool) -> anyhow::Result<()> {
    let app = uuid::Uuid::now_v7();
    sqlx::query(
        "INSERT INTO oauth_apps (id, name, client_id, client_secret, redirect_uri)
         VALUES ($1, 'test', $2, 'secret', 'https://app.test/')",
    )
    .bind(app)
    .bind(app.to_string())
    .execute(pool)
    .await?;
    sqlx::query(
        "INSERT INTO soundcloud_connections (
             id, soundcloud_user_id, oauth_app_id, access_token, refresh_token, expires_at, scope
         ) VALUES ($1, '17', $2, 'access', 'refresh', now() + interval '1 day', '')",
    )
    .bind(uuid::Uuid::now_v7())
    .bind(app)
    .execute(pool)
    .await?;
    Ok(())
}

async fn setup(pool: &PgPool) -> anyhow::Result<SyncQueueRepository> {
    crate::db::migrations::run_core(pool, None).await?;
    connect_owner(pool).await?;
    sqlx::query("INSERT INTO tracks (sc_track_id, urn, title, title_normalized, duration_ms, uploader_sc_user_id)
        VALUES ('42', 'soundcloud:tracks:42', 'Track', 'track', 120000, '17')")
        .execute(pool).await?;
    update(pool, &json!({"track": {"title": "First edit"}})).await?;
    Ok(SyncQueueRepository::new(
        pool.clone(),
        Duration::from_secs(60),
    ))
}

#[sqlx::test(migrations = false)]
async fn newer_track_intent_fences_old_ack_and_confirmation_is_durable(
    pool: PgPool,
) -> anyhow::Result<()> {
    let repository = setup(&pool).await?;
    let old = repository
        .claim(1)
        .await?
        .into_iter()
        .next()
        .ok_or_else(|| anyhow::anyhow!("track mutation was not claimed"))?;
    assert!(repository.record_remote_attempt(&old).await?);
    assert!(
        repository
            .record_remote_success(&old, &json!({"desired": old.desired}))
            .await?
    );
    update(&pool, &json!({"track": {"sharing": "private"}})).await?;
    repository.finalize(&old).await?;
    let row: (String, bool) =
        sqlx::query_as("SELECT sharing, sc_write_confirmed FROM tracks WHERE sc_track_id = '42'")
            .fetch_one(&pool)
            .await?;
    assert_eq!(row, ("private".into(), false));
    repository.adopt().await?;
    let current = repository
        .claim(1)
        .await?
        .into_iter()
        .next()
        .ok_or_else(|| anyhow::anyhow!("track mutation was not claimed"))?;
    assert!(current.generation > old.generation);
    assert!(repository.record_remote_attempt(&current).await?);
    assert!(
        repository
            .record_remote_success(&current, &json!({"desired": current.desired}))
            .await?
    );
    repository.finalize(&current).await?;
    repository.finalize(&current).await?;
    let confirmed: bool =
        sqlx::query_scalar("SELECT sc_write_confirmed FROM tracks WHERE sc_track_id = '42'")
            .fetch_one(&pool)
            .await?;
    assert!(confirmed);
    let pending: i64 = sqlx::query_scalar("SELECT count(*) FROM sync_queue")
        .fetch_one(&pool)
        .await?;
    assert_eq!(pending, 0);
    let jobs: Vec<Value> =
        sqlx::query_scalar("SELECT payload FROM background_jobs WHERE kind = 'catalog.refresh'")
            .fetch_all(&pool)
            .await?;
    assert_eq!(
        jobs,
        [json!({"version": "1", "payload": {"entity": "track", "sc_id": "42", "owner_id": "17"}})]
    );
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn failed_confirmation_enqueue_preserves_the_stored_remote_result(
    pool: PgPool,
) -> anyhow::Result<()> {
    let repository = setup(&pool).await?;
    let mutation = repository
        .claim(1)
        .await?
        .into_iter()
        .next()
        .ok_or_else(|| anyhow::anyhow!("track mutation was not claimed"))?;
    assert!(repository.record_remote_attempt(&mutation).await?);
    assert!(
        repository
            .record_remote_success(&mutation, &json!({"desired": mutation.desired}))
            .await?
    );
    sqlx::query("ALTER TABLE background_jobs ADD CONSTRAINT no_confirmation CHECK (kind <> 'catalog.refresh')")
        .execute(&pool).await?;
    assert!(repository.finalize(&mutation).await.is_err());
    let confirmed: bool =
        sqlx::query_scalar("SELECT sc_write_confirmed FROM tracks WHERE sc_track_id = '42'")
            .fetch_one(&pool)
            .await?;
    assert!(!confirmed);
    let completed: Option<i64> =
        sqlx::query_scalar("SELECT remote_completed_generation FROM sync_queue WHERE id = $1")
            .bind(mutation.id)
            .fetch_one(&pool)
            .await?;
    assert_eq!(completed, Some(mutation.generation));
    sqlx::query("ALTER TABLE background_jobs DROP CONSTRAINT no_confirmation")
        .execute(&pool)
        .await?;
    repository.finalize(&mutation).await?;
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn a_like_of_a_missing_track_is_closed_and_heal_leaves_it_alone(
    pool: PgPool,
) -> anyhow::Result<()> {
    crate::db::migrations::run_core(&pool, None).await?;
    connect_owner(&pool).await?;
    sqlx::raw_sql(
        "INSERT INTO user_likes_tracks (user_id, sc_track_id, progress, created_at)
         VALUES ('17', '404', true, now() - interval '1 hour');
         INSERT INTO sync_queue (user_id, action_type, target_urn)
         VALUES ('17', 'like_track', 'soundcloud:tracks:404')",
    )
    .execute(&pool)
    .await?;
    let repository = SyncQueueRepository::new(pool.clone(), Duration::from_secs(60));
    let like = repository
        .claim(1)
        .await?
        .into_iter()
        .next()
        .ok_or_else(|| anyhow::anyhow!("like was not claimed"))?;
    assert!(repository.record_remote_attempt(&like).await?);
    assert!(
        repository
            .record_remote_success(&like, &Value::Null)
            .await?
    );

    repository.finalize(&like).await?;
    sqlx::raw_sql(include_str!(
        "../../../queries/maintenance/heal_track_likes.sql"
    ))
    .execute(&pool)
    .await?;

    let progress: bool = sqlx::query_scalar(
        "SELECT progress FROM user_likes_tracks WHERE user_id = '17' AND sc_track_id = '404'",
    )
    .fetch_one(&pool)
    .await?;
    let queued: i64 = sqlx::query_scalar("SELECT count(*) FROM sync_queue")
        .fetch_one(&pool)
        .await?;
    assert!(!progress);
    assert_eq!(queued, 0);
    Ok(())
}

async fn age_edit(pool: &PgPool, table: &str) -> anyhow::Result<()> {
    let mut tx = pool.begin().await?;
    sqlx::query("SET LOCAL session_replication_role = replica")
        .execute(&mut *tx)
        .await?;
    sqlx::query(&format!(
        "UPDATE {table} SET sc_mutation_observation = sc_mutation_observation - (3600000000::bigint << 4)"
    ))
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

pub(super) async fn heal(pool: &PgPool, table: &str) -> anyhow::Result<()> {
    age_edit(pool, table).await?;
    crate::handlers::maintenance::MaintenanceHandler::new(pool.clone())
        .heal_sync_queue()
        .await
        .map_err(|error| anyhow::anyhow!("{error:?}"))
}

#[sqlx::test(migrations = false)]
async fn a_push_carries_every_merged_field_and_a_later_core_edit_is_healed_into_a_new_push(
    pool: PgPool,
) -> anyhow::Result<()> {
    let repository = setup(&pool).await?;
    update(&pool, &json!({"track": {"sharing": "private"}})).await?;
    repository.adopt().await?;
    let pushed = repository
        .claim(1)
        .await?
        .into_iter()
        .next()
        .ok_or_else(|| anyhow::anyhow!("track mutation was not claimed"))?;
    let desired = pushed.desired.clone().unwrap_or_default();
    assert_eq!(desired["title"], json!("First edit"));
    assert_eq!(desired["sharing"], json!("private"));
    assert!(repository.record_remote_attempt(&pushed).await?);
    assert!(
        repository
            .record_remote_success(&pushed, &json!({"desired": desired}))
            .await?
    );
    let late = catalog_ingest::TrackUpdate::parse(&json!({"track": {"description": "Late"}}))
        .map_err(anyhow::Error::msg)?;
    let edit = wt::TrackEdit {
        sc_track_id: "42".to_owned(),
        uploader: "17".to_owned(),
        desired: late.desired().clone(),
        observation: catalog_ingest::Observation::begin(&pool).await?.sequence(),
        at: wt::now(),
    };
    anyhow::ensure!(
        wt::call(&mut *pool.acquire().await?, &edit)
            .await?
            .is_some()
    );
    repository.finalize(&pushed).await?;
    let confirmed: bool =
        sqlx::query_scalar("SELECT sc_write_confirmed FROM tracks WHERE sc_track_id = '42'")
            .fetch_one(&pool)
            .await?;
    assert!(!confirmed);
    assert!(repository.claim(1).await?.is_empty());

    heal(&pool, "tracks").await?;
    let healed = repository
        .claim(1)
        .await?
        .into_iter()
        .next()
        .ok_or_else(|| anyhow::anyhow!("healed track edit was not claimed"))?;
    let desired = healed.desired.clone().unwrap_or_default();
    assert_eq!(desired["description"], json!("Late"));
    assert_eq!(desired["sharing"], json!("private"));
    assert!(repository.record_remote_attempt(&healed).await?);
    assert!(
        repository
            .record_remote_success(&healed, &json!({"desired": desired}))
            .await?
    );
    repository.finalize(&healed).await?;
    let confirmed: bool =
        sqlx::query_scalar("SELECT sc_write_confirmed FROM tracks WHERE sc_track_id = '42'")
            .fetch_one(&pool)
            .await?;
    assert!(confirmed);
    heal(&pool, "tracks").await?;
    assert!(repository.claim(1).await?.is_empty());
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn an_edit_whose_desired_state_was_already_observed_is_dropped_unsent(
    pool: PgPool,
) -> anyhow::Result<()> {
    let repository = setup(&pool).await?;
    sqlx::query("UPDATE tracks SET sc_desired = '{}' WHERE sc_track_id = '42'")
        .execute(&pool)
        .await?;
    let mutation = repository
        .claim(1)
        .await?
        .into_iter()
        .next()
        .ok_or_else(|| anyhow::anyhow!("track mutation was not claimed"))?;
    assert_eq!(mutation.desired, Some(json!({})));
    assert!(repository.drop_superseded(&mutation).await?);
    let left: i64 = sqlx::query_scalar("SELECT count(*) FROM sync_queue")
        .fetch_one(&pool)
        .await?;
    assert_eq!(left, 0);
    Ok(())
}
