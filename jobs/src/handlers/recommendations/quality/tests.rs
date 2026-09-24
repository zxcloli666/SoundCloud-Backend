use anyhow::Context;
use sqlx::PgPool;

use crate::config::QdrantConfig;
use crate::qdrant::QdrantProvisioner;

use super::backfill::{persist, tracks_to_score};
use super::features::{FEATURE_COUNT, load_features};
use super::model::{QualityModel, TrainedModel};
use super::store;
use super::train::{changed_meaningfully, save_if_changed};

fn unreachable_qdrant() -> anyhow::Result<QdrantProvisioner> {
    QdrantProvisioner::connect(&QdrantConfig {
        grpc_url: "http://127.0.0.1:1".to_owned(),
        api_key: String::new().into(),
    })
}

fn trained_model(intercept: f32) -> TrainedModel {
    TrainedModel {
        model: QualityModel {
            means: [0.5; FEATURE_COUNT],
            scales: [0.25; FEATURE_COUNT],
            weights: [0.125; FEATURE_COUNT],
            intercept,
        },
        examples: 240,
        positives: 158,
        accuracy: 0.925,
        iterations: 400,
        converged: true,
    }
}

async fn insert_track(
    pool: &PgPool,
    sc_track_id: &str,
    indexed_minutes_ago: i32,
    quality_score: Option<f32>,
    model_version: Option<i64>,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO tracks (
             sc_track_id, urn, title, title_normalized, duration_ms,
             indexed_at, index_state, storage_state, quality_score, quality_model_version
         ) VALUES (
             $1, 'soundcloud:tracks:' || $1, 'Track ' || $1, 'track ' || $1, 180000,
             now() - make_interval(mins => $2), 'indexed', 'ok', $3, $4
         )",
    )
    .bind(sc_track_id)
    .bind(indexed_minutes_ago)
    .bind(quality_score)
    .bind(model_version)
    .execute(pool)
    .await?;
    Ok(())
}

#[sqlx::test(migrations = "../api/migrations")]
async fn a_saved_model_is_the_latest_and_versions_count_up(pool: PgPool) -> anyhow::Result<()> {
    assert!(store::latest(&pool).await?.is_none());

    let first = store::save(&pool, &trained_model(0.1)).await?;
    let second = store::save(&pool, &trained_model(-0.2)).await?;
    let latest = store::latest(&pool)
        .await?
        .context("the saved model must be read back")?;

    assert_eq!((first, second), (1, 2));
    assert_eq!(latest.version, 2);
    assert_eq!(latest.model, trained_model(-0.2).model);
    Ok(())
}

#[sqlx::test(migrations = "../api/migrations")]
async fn the_database_refuses_a_model_of_another_shape(pool: PgPool) -> anyhow::Result<()> {
    let refused = sqlx::query(
        "INSERT INTO recommendation_quality_models (
             version, feature_means, feature_scales, weights, intercept,
             examples, positives, train_accuracy
         ) VALUES (1, '{0.5}', '{1.0}', '{0.1}', 0.0, 200, 100, 0.9)",
    )
    .execute(&pool)
    .await;

    assert!(refused.is_err());
    Ok(())
}

#[sqlx::test(migrations = "../api/migrations")]
async fn without_a_model_only_unscored_tracks_are_picked(pool: PgPool) -> anyhow::Result<()> {
    insert_track(&pool, "1", 10, None, None).await?;
    insert_track(&pool, "2", 5, Some(0.4), None).await?;

    assert_eq!(tracks_to_score(&pool, None).await?, vec!["1".to_owned()]);
    Ok(())
}

#[sqlx::test(migrations = "../api/migrations")]
async fn a_new_model_rescores_older_scores_after_the_unscored(pool: PgPool) -> anyhow::Result<()> {
    insert_track(&pool, "1", 30, None, None).await?;
    insert_track(&pool, "2", 20, Some(0.4), Some(1)).await?;
    insert_track(&pool, "3", 10, Some(0.6), None).await?;
    insert_track(&pool, "4", 5, Some(0.7), Some(2)).await?;

    assert_eq!(
        tracks_to_score(&pool, Some(2)).await?,
        vec!["1".to_owned(), "3".to_owned(), "2".to_owned()]
    );
    Ok(())
}

#[sqlx::test(migrations = "../api/migrations")]
async fn a_persisted_score_remembers_the_model_that_made_it(pool: PgPool) -> anyhow::Result<()> {
    insert_track(&pool, "1", 10, None, None).await?;
    insert_track(&pool, "2", 5, None, None).await?;

    persist(
        &pool,
        &["1".to_owned(), "2".to_owned()],
        &[0.25, 0.75],
        Some(3),
    )
    .await?;
    let rows: Vec<(String, Option<f32>, Option<i64>)> = sqlx::query_as(
        "SELECT sc_track_id, quality_score, quality_model_version
         FROM tracks
         ORDER BY sc_track_id",
    )
    .fetch_all(&pool)
    .await?;

    assert_eq!(
        rows,
        vec![
            ("1".to_owned(), Some(0.25), Some(3)),
            ("2".to_owned(), Some(0.75), Some(3)),
        ]
    );
    assert!(tracks_to_score(&pool, Some(3)).await?.is_empty());
    Ok(())
}

#[sqlx::test(migrations = "../api/migrations")]
async fn an_unreachable_vector_store_fails_the_batch_instead_of_zeroing_it(
    pool: PgPool,
) -> anyhow::Result<()> {
    insert_track(&pool, "1", 10, None, None).await?;

    let loaded = load_features(&pool, &unreachable_qdrant()?, vec!["1".to_owned()]).await;

    assert!(matches!(loaded, Err(error) if error.is_retryable()));
    Ok(())
}

#[sqlx::test(migrations = "../api/migrations")]
async fn retraining_into_the_same_model_keeps_the_current_version(
    pool: PgPool,
) -> anyhow::Result<()> {
    let first = save_if_changed(&pool, &trained_model(0.1)).await?;
    let repeated = save_if_changed(&pool, &trained_model(0.101)).await?;
    let moved = save_if_changed(&pool, &trained_model(0.6)).await?;

    assert_eq!((first, repeated, moved), (Some(1), None, Some(2)));
    Ok(())
}

async fn insert_event(pool: &PgPool, user: &str, sc_track_id: &str) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO user_events (sc_user_id, sc_track_id, event_type, weight)
         VALUES ($1, $2, 'like', 1)",
    )
    .bind(user)
    .bind(sc_track_id)
    .execute(pool)
    .await?;
    Ok(())
}

async fn insert_dislike(pool: &PgPool, user: &str, sc_track_id: &str) -> anyhow::Result<()> {
    sqlx::query("INSERT INTO disliked_tracks (sc_user_id, sc_track_id) VALUES ($1, $2)")
        .bind(user)
        .bind(sc_track_id)
        .execute(pool)
        .await?;
    Ok(())
}

#[sqlx::test(migrations = "../api/migrations")]
async fn a_good_track_is_labelled_by_its_listeners_not_by_its_popularity(
    pool: PgPool,
) -> anyhow::Result<()> {
    insert_track(&pool, "1", 10, None, None).await?;
    insert_track(&pool, "2", 10, None, None).await?;
    insert_track(&pool, "3", 10, None, None).await?;
    insert_track(&pool, "4", 10, None, None).await?;
    sqlx::query(
        "INSERT INTO sc_track_counters (sc_track_id, play_count, likes_count)
         VALUES ('1', 200, 10), ('2', 50000, 5000)",
    )
    .execute(&pool)
    .await?;
    for user in ["u1", "u2", "u3"] {
        insert_event(&pool, user, "1").await?;
    }
    insert_event(&pool, "u1", "3").await?;
    insert_event(&pool, "u1", "3").await?;
    insert_dislike(&pool, "u4", "4").await?;
    let handler = super::QualityHandler::new(pool.clone(), unreachable_qdrant()?);

    let labels = handler.labels().await?;

    assert_eq!(labels.get("1"), Some(&true));
    assert_eq!(labels.get("2"), None);
    assert_eq!(labels.get("3"), None);
    assert_eq!(labels.get("4"), Some(&false));
    Ok(())
}

#[sqlx::test(migrations = "../api/migrations")]
async fn every_training_sees_the_same_negatives(pool: PgPool) -> anyhow::Result<()> {
    for id in ["5", "4", "3", "2", "1"] {
        insert_track(&pool, id, 10, None, None).await?;
        insert_dislike(&pool, "u1", id).await?;
        insert_dislike(&pool, "u2", id).await?;
    }

    let first = sqlx::query_file_scalar!(
        "queries/recommendations/quality/select_negative_examples.sql",
        2_i64
    )
    .fetch_all(&pool)
    .await?;
    let again = sqlx::query_file_scalar!(
        "queries/recommendations/quality/select_negative_examples.sql",
        2_i64
    )
    .fetch_all(&pool)
    .await?;

    assert_eq!(first, vec!["1".to_owned(), "2".to_owned()]);
    assert_eq!(first, again);
    Ok(())
}

#[sqlx::test(migrations = "../api/migrations")]
async fn one_backfill_run_scores_more_than_a_single_batch(pool: PgPool) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO tracks (
             sc_track_id, urn, title, title_normalized, duration_ms,
             indexed_at, index_state, storage_state
         )
         SELECT 't' || n, 'soundcloud:tracks:t' || n, 'Track ' || n, 'track ' || n, 180000,
                now() - make_interval(mins => n), 'indexed', 'ok'
         FROM generate_series(1, 1100) AS n",
    )
    .execute(&pool)
    .await?;
    let handler = super::QualityHandler::new(pool.clone(), unreachable_qdrant()?);

    handler.backfill().await?;

    assert!(tracks_to_score(&pool, None).await?.is_empty());
    Ok(())
}

#[test]
fn a_single_moved_weight_is_a_meaningful_change() {
    let previous = trained_model(0.1).model;
    let nudged = |delta: f32| {
        let mut next = previous.clone();
        next.weights = std::array::from_fn(|index| if index == 3 { 0.125 + delta } else { 0.125 });
        next
    };

    assert!(!changed_meaningfully(&previous, &nudged(0.01)));
    assert!(changed_meaningfully(&previous, &nudged(0.2)));
}
