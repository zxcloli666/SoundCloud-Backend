use super::*;

async fn setup(pool: &PgPool) -> anyhow::Result<TrackMutations> {
    sqlx::query("INSERT INTO tracks (sc_track_id, urn, title, title_normalized, duration_ms, uploader_sc_user_id)
        VALUES ('42', 'soundcloud:tracks:42', 'Original', 'original', 120000, '17')")
        .execute(pool).await?;
    let redis = deadpool_redis::Config::from_url("redis://127.0.0.1:1")
        .create_pool(Some(deadpool_redis::Runtime::Tokio1))?;
    Ok(TrackMutations::new(
        pool.clone(),
        SyncQueueService::new(pool.clone(), redis),
    ))
}

#[sqlx::test(migrations = "./migrations")]
async fn track_mutations_persist_and_merge_without_sessions_tokens_or_redis(
    pool: PgPool,
) -> anyhow::Result<()> {
    let service = setup(&pool).await?;
    service.update("17", "42", &json!({"track": {"title": "Updated", "downloadable": false, "isrc": "USABC2600042", "metadata_artist": "Artist"}})).await?;
    service
        .update(
            "soundcloud:users:17",
            "soundcloud:tracks:42",
            &json!({"track": {"sharing": "private"}}),
        )
        .await?;
    let (title, sharing, metadata, desired): (String, String, Value, Value) = sqlx::query_as(
        "SELECT title, sharing, sc_metadata, sc_desired FROM tracks WHERE sc_track_id = '42'",
    )
    .fetch_one(&pool)
    .await?;
    assert_eq!(title, "Updated");
    assert_eq!(sharing, "private");
    assert_eq!(metadata["downloadable"], false);
    assert_eq!(desired["title"], "Updated");
    assert_eq!(desired["sharing"], "private");
    let (payload, generation): (Value, i64) = sqlx::query_as("SELECT payload, generation FROM sync_queue WHERE user_id = '17' AND action_type = 'track_update'")
        .fetch_one(&pool).await?;
    assert_eq!(
        payload,
        json!({"track": {"title": "Updated", "downloadable": false, "isrc": "USABC2600042", "metadata_artist": "Artist", "sharing": "private"}})
    );
    assert_eq!(generation, 2);
    let projected = super::super::project_many(&pool, &["42".into()])
        .await?
        .into_iter()
        .flatten()
        .next()
        .ok_or_else(|| anyhow::anyhow!("track was not projected"))?;
    assert_eq!(projected["metadata_artist"], "Artist");
    assert_eq!(projected["isrc"], "USABC2600042");
    assert_eq!(projected["publisher_metadata"]["artist"], "Artist");
    assert_eq!(projected["downloadable"], false);
    let access = sqlx::query_file!("queries/tracks/service/read_access.sql", "42", "18")
        .fetch_one(&pool)
        .await?;
    assert_eq!(access.can_read, Some(false));
    assert!(!access.secret_ready);
    assert!(access.sc_mutation_observation > 0);
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn track_mutations_roll_back_the_queue_on_wrong_owner_or_failed_local_write(
    pool: PgPool,
) -> anyhow::Result<()> {
    let service = setup(&pool).await?;
    let body = json!({"track": {"title": "Blocked"}});
    assert!(service.update("18", "42", &body).await.is_err());
    assert!(service.delete("18", "42").await.is_err());
    sqlx::query("ALTER TABLE tracks ADD CHECK (title <> 'Blocked')")
        .execute(&pool)
        .await?;
    assert!(service.update("17", "42", &body).await.is_err());
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM sync_queue")
        .fetch_one(&pool)
        .await?;
    assert_eq!(count, 0);
    let title: String = sqlx::query_scalar("SELECT title FROM tracks WHERE sc_track_id = '42'")
        .fetch_one(&pool)
        .await?;
    assert_eq!(title, "Original");
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn track_delete_supersedes_updates_and_prevents_refresh_resurrection(
    pool: PgPool,
) -> anyhow::Result<()> {
    let service = setup(&pool).await?;
    sqlx::query("INSERT INTO user_owned_tracks (user_id, sc_track_id) VALUES ('17', '42')")
        .execute(&pool)
        .await?;
    service
        .update("17", "42", &json!({"track": {"title": "Pending edit"}}))
        .await?;
    service.delete("17", "42").await?;
    service.delete("17", "42").await?;
    assert!(
        service
            .update("17", "42", &json!({"track": {"sharing": "public"}}))
            .await
            .is_err()
    );
    let access = sqlx::query_file!("queries/tracks/service/read_access.sql", "42", "17")
        .fetch_one(&pool)
        .await?;
    assert!(access.deleted);
    assert!(
        super::super::project_many(&pool, &["42".into()])
            .await?
            .into_iter()
            .all(|track| track.is_none())
    );
    let fields = catalog_ingest::ScTrackFields::from_sc(&json!({"urn": "soundcloud:tracks:42", "title": "Stale public", "sharing": "public", "duration": 120000})).unwrap();
    let ingest = catalog_ingest::upsert_from_sc(
        &pool,
        &fields,
        catalog_ingest::TrackPriority::Discovery,
        catalog_ingest::TrackPriority::Discovery,
        catalog_ingest::Observation::begin(&pool).await?,
    )
    .await?;
    assert!(!ingest.metadata_applied);
    let actions: Vec<String> = sqlx::query_scalar("SELECT action_type FROM sync_queue")
        .fetch_all(&pool)
        .await?;
    assert_eq!(actions, ["track_delete"]);
    let owned: i64 = sqlx::query_scalar("SELECT count(*) FROM user_owned_tracks")
        .fetch_one(&pool)
        .await?;
    assert_eq!(owned, 0);
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn coalesced_track_update_cannot_exceed_the_delivery_payload_limit(
    pool: PgPool,
) -> anyhow::Result<()> {
    let service = setup(&pool).await?;
    service
        .update(
            "17",
            "42",
            &json!({"track": {"description": "a".repeat(40000)}}),
        )
        .await?;
    assert!(
        service
            .update(
                "17",
                "42",
                &json!({"track": {"tag_list": "b".repeat(40000)}})
            )
            .await
            .is_err()
    );
    let (generation, tags): (i64, Vec<String>) = sqlx::query_as(
        "SELECT q.generation, t.tags FROM sync_queue q JOIN tracks t ON t.urn = q.target_urn",
    )
    .fetch_one(&pool)
    .await?;
    assert_eq!(generation, 1);
    assert!(tags.is_empty());
    Ok(())
}

async fn wait_for_blocked_track_writes(pool: &PgPool, count: i64) -> anyhow::Result<()> {
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let blocked: i64 = sqlx::query_scalar("SELECT count(*) FROM pg_stat_activity WHERE datname = current_database() AND wait_event_type = 'Lock'")
                .fetch_one(pool).await?;
            if blocked >= count { return Ok::<_, anyhow::Error>(()); }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    }).await??;
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn track_delete_cancels_an_update_that_was_uncommitted_when_delete_started(
    pool: PgPool,
) -> anyhow::Result<()> {
    let service = Arc::new(setup(&pool).await?);
    let mut blocker = pool.begin().await?;
    sqlx::query("SELECT id FROM tracks WHERE sc_track_id = '42' FOR UPDATE")
        .execute(&mut *blocker)
        .await?;
    let updating = {
        let service = service.clone();
        tokio::spawn(async move {
            service
                .update(
                    "17",
                    "42",
                    &json!({"track": {"title": "Concurrent update"}}),
                )
                .await
        })
    };
    wait_for_blocked_track_writes(&pool, 1).await?;
    let deleting = tokio::spawn(async move { service.delete("17", "42").await });
    wait_for_blocked_track_writes(&pool, 2).await?;
    blocker.commit().await?;
    updating.await??;
    deleting.await??;
    let actions: Vec<String> = sqlx::query_scalar("SELECT action_type FROM sync_queue")
        .fetch_all(&pool)
        .await?;
    assert_eq!(actions, ["track_delete"]);
    Ok(())
}
