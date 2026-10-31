use std::sync::Arc;

use serde_json::{Value, json};
use sqlx::PgPool;

use super::IndexingService;

async fn indexing(pool: &PgPool) -> anyhow::Result<Arc<IndexingService>> {
    let nats = crate::bus::nats::NatsService::connect(
        "nats://127.0.0.1:1",
        tokio_util::sync::CancellationToken::new(),
    )
    .await?;
    let background = crate::background_jobs::BackgroundJobs::new(nats);
    Ok(IndexingService::new(
        pool.clone(),
        background.clone(),
        crate::background_jobs::IndexingJobs::new(background),
        420000,
    ))
}

fn uploaded(id: u64, duration: u64) -> Value {
    json!({
        "kind": "track",
        "id": id,
        "urn": format!("soundcloud:tracks:{id}"),
        "title": "Demo",
        "duration": duration,
        "full_duration": duration,
        "access": "playable",
        "policy": "ALLOW",
        "sharing": "private",
        "user": {"kind": "user", "id": 17, "urn": "soundcloud:users:17", "username": "me"}
    })
}

fn owned(sc_track_id: &str) -> wt::OwnedInsert {
    wt::OwnedInsert {
        user_id: "17".to_owned(),
        sc_track_id: sc_track_id.to_owned(),
        variants: crate::common::sc_ids::user_id_variants("17"),
        at: wt::now(),
    }
}

async fn stored(pool: &PgPool, sc_track_id: &str) -> anyhow::Result<(Option<String>, bool)> {
    Ok(sqlx::query_as(
        "SELECT (SELECT storage_state FROM tracks WHERE sc_track_id = $1),
                EXISTS (SELECT 1 FROM user_owned_tracks WHERE user_id = '17' AND sc_track_id = $1)",
    )
    .bind(sc_track_id)
    .fetch_one(pool)
    .await?)
}

#[sqlx::test(migrations = "./migrations")]
async fn an_upload_lands_with_its_owned_row(pool: PgPool) -> anyhow::Result<()> {
    let indexing = indexing(&pool).await?;

    indexing
        .ingest_upload(
            &uploaded(31, 200_000),
            &owned("31"),
            catalog_ingest::Observation::begin(&pool).await?,
        )
        .await?;

    let (state, mine) = stored(&pool, "31").await?;
    assert!(state.is_some_and(|state| state != "too_long"));
    assert!(mine);
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn a_too_long_upload_is_marked_and_still_owned(pool: PgPool) -> anyhow::Result<()> {
    let indexing = indexing(&pool).await?;

    indexing
        .ingest_upload(
            &uploaded(32, 3_600_000),
            &owned("32"),
            catalog_ingest::Observation::begin(&pool).await?,
        )
        .await?;

    assert_eq!(stored(&pool, "32").await?, (Some("too_long".to_owned()), true));
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn an_upload_without_track_fields_still_records_the_owned_row(
    pool: PgPool,
) -> anyhow::Result<()> {
    let indexing = indexing(&pool).await?;

    indexing
        .ingest_upload(
            &json!({"urn": "soundcloud:tracks:33"}),
            &owned("33"),
            catalog_ingest::Observation::begin(&pool).await?,
        )
        .await?;

    assert_eq!(stored(&pool, "33").await?, (None, true));
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn a_held_track_over_the_cap_is_released_as_too_long(pool: PgPool) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO tracks (sc_track_id, urn, title, title_normalized, duration_ms, pipeline_held)
         VALUES ('34', 'soundcloud:tracks:34', 'Set', 'set', 3600000, true),
                ('35', 'soundcloud:tracks:35', 'Song', 'song', 200000, true)",
    )
    .execute(&pool)
    .await?;
    let indexing = indexing(&pool).await?;

    assert!(indexing.release_held("34").await?);
    assert!(indexing.release_held("35").await?);
    assert!(!indexing.release_held("35").await?, "released once");
    assert!(!indexing.release_held("36").await?, "an unknown track");

    let states: Vec<(String, bool, String)> = sqlx::query_as(
        "SELECT sc_track_id, pipeline_held, storage_state FROM tracks ORDER BY sc_track_id",
    )
    .fetch_all(&pool)
    .await?;
    assert_eq!(
        states,
        [
            ("34".to_owned(), false, "too_long".to_owned()),
            ("35".to_owned(), false, "pending".to_owned())
        ]
    );
    Ok(())
}
