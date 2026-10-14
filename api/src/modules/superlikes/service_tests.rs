use serde_json::json;
use sqlx::PgPool;

use super::*;
use crate::common::sc_ids::{EntityKind, require_ref};
use crate::modules::likes::cold::apply_user_favorite_flag;

const USER: &str = "soundcloud:users:17";

async fn like(pool: &PgPool, tracks: &[&str]) -> anyhow::Result<()> {
    for track in tracks {
        sqlx::query(
            "INSERT INTO tracks (sc_track_id, urn, title, title_normalized, duration_ms)
             VALUES ($1, 'soundcloud:tracks:' || $1, 'Song', 'song', 200000)",
        )
        .bind(track)
        .execute(pool)
        .await?;
        sqlx::query(
            "INSERT INTO user_likes_tracks (user_id, sc_track_id, wanted_state) VALUES ('17', $1, true)",
        )
        .bind(track)
        .execute(pool)
        .await?;
    }
    Ok(())
}

async fn superlike_track(pool: &PgPool, track: &str, premium: bool) -> AppResult<Superliked> {
    let redis = deadpool_redis::Config::from_url("redis://127.0.0.1:1")
        .create_pool(Some(deadpool_redis::Runtime::Tokio1))
        .map_err(|e| AppError::internal(e.to_string()))?;
    let queue = SyncQueueService::new(pool.clone(), redis);
    superlike(
        pool,
        &queue,
        USER,
        &require_ref(EntityKind::Track, track)?,
        premium,
    )
    .await
}

async fn relike(pool: &PgPool, track: &str) -> anyhow::Result<()> {
    sqlx::query(
        "UPDATE user_likes_tracks SET created_at = created_at + interval '1 second'
         WHERE user_id = '17' AND sc_track_id = $1",
    )
    .bind(track)
    .execute(pool)
    .await?;
    Ok(())
}

fn ids(count: usize) -> Vec<String> {
    (1..=count).map(|i| i.to_string()).collect()
}

async fn queued_likes(pool: &PgPool) -> anyhow::Result<Vec<String>> {
    Ok(sqlx::query_scalar(
        "SELECT target_urn FROM sync_queue WHERE action_type = 'like_track' ORDER BY target_urn",
    )
    .fetch_all(pool)
    .await?)
}

#[sqlx::test(migrations = "./migrations")]
async fn without_star_the_eleventh_superlike_waits_for_a_free_slot(
    pool: PgPool,
) -> anyhow::Result<()> {
    let tracks = ids(11);
    let refs: Vec<&str> = tracks.iter().map(String::as_str).collect();
    like(&pool, &refs).await?;
    for track in &tracks[..10] {
        superlike_track(&pool, track, false).await?;
    }
    assert_eq!(
        quota(&pool, USER, false).await?,
        Quota {
            used: 10,
            limit: Some(FREE_LIMIT)
        }
    );

    let refused = superlike_track(&pool, "11", false).await.unwrap_err();
    assert_eq!(refused.public_code(), "superlike_limit_reached");
    assert_eq!(refused.status(), StatusCode::PAYMENT_REQUIRED);

    let again = superlike_track(&pool, "1", false).await?;
    assert_eq!(
        again.quota.used, 10,
        "a superlike that is already there is no new one"
    );

    forget(&pool, USER, "1").await?;
    assert_eq!(superlike_track(&pool, "11", false).await?.quota.used, 10);
    assert!(queued_likes(&pool).await?.is_empty());
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn a_full_quota_refuses_an_unliked_track_without_liking_it(
    pool: PgPool,
) -> anyhow::Result<()> {
    let tracks = ids(10);
    let refs: Vec<&str> = tracks.iter().map(String::as_str).collect();
    like(&pool, &refs).await?;
    for track in &tracks {
        superlike_track(&pool, track, false).await?;
    }

    let refused = superlike_track(&pool, "50", false).await.unwrap_err();

    assert_eq!(refused.public_code(), "superlike_limit_reached");
    let liked: i64 =
        sqlx::query_scalar("SELECT count(*) FROM user_likes_tracks WHERE sc_track_id = '50'")
            .fetch_one(&pool)
            .await?;
    assert_eq!(liked, 0);
    assert!(queued_likes(&pool).await?.is_empty());
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn star_has_no_limit_and_a_lapsed_star_keeps_what_it_made(
    pool: PgPool,
) -> anyhow::Result<()> {
    let tracks = ids(12);
    let refs: Vec<&str> = tracks.iter().map(String::as_str).collect();
    like(&pool, &refs).await?;
    for track in &tracks {
        superlike_track(&pool, track, true).await?;
    }
    assert_eq!(
        quota(&pool, USER, true).await?,
        Quota {
            used: 12,
            limit: None
        }
    );
    assert_eq!(quota(&pool, USER, false).await?.used, 12);
    forget(&pool, USER, "1").await?;
    let refused = superlike_track(&pool, "1", false).await.unwrap_err();
    assert_eq!(refused.public_code(), "superlike_limit_reached");
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn a_superlike_lives_only_as_long_as_its_like(pool: PgPool) -> anyhow::Result<()> {
    like(&pool, &["5", "6"]).await?;
    superlike_track(&pool, "5", false).await?;
    superlike_track(&pool, "6", false).await?;

    relike(&pool, "5").await?;
    sqlx::query("UPDATE user_likes_tracks SET wanted_state = false WHERE sc_track_id = '6'")
        .execute(&pool)
        .await?;
    assert_eq!(quota(&pool, USER, false).await?.used, 0);

    let again = superlike_track(&pool, "5", false).await?;
    assert_eq!((again.quota.used, again.newly_liked), (1, false));
    let liked_again = superlike_track(&pool, "6", false).await?;
    assert_eq!((liked_again.quota.used, liked_again.newly_liked), (2, true));
    assert_eq!(queued_likes(&pool).await?, vec!["soundcloud:tracks:6"]);
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn a_taken_down_track_frees_its_slot(pool: PgPool) -> anyhow::Result<()> {
    like(&pool, &["5", "6"]).await?;
    superlike_track(&pool, "5", false).await?;
    superlike_track(&pool, "6", false).await?;

    sqlx::query("UPDATE tracks SET deleted_at = now() WHERE sc_track_id = '6'")
        .execute(&pool)
        .await?;

    assert_eq!(quota(&pool, USER, false).await?.used, 1);
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn track_payloads_carry_the_super_heart_only_for_live_superlikes(
    pool: PgPool,
) -> anyhow::Result<()> {
    like(&pool, &["5", "6", "7"]).await?;
    superlike_track(&pool, "5", false).await?;
    superlike_track(&pool, "6", false).await?;
    relike(&pool, "6").await?;
    let mut tracks = vec![
        json!({"urn": "soundcloud:tracks:5"}),
        json!({"urn": "soundcloud:tracks:6"}),
        json!({"urn": "soundcloud:tracks:7"}),
        json!({"urn": "soundcloud:tracks:8"}),
    ];

    apply_user_favorite_flag(&pool, USER, &mut tracks).await?;

    let flags: Vec<(bool, bool)> = tracks
        .iter()
        .map(|t| {
            (
                t["user_favorite"] == true,
                t.get("user_superlike") == Some(&json!(true)),
            )
        })
        .collect();
    assert_eq!(
        flags,
        vec![(true, true), (true, false), (true, false), (false, false)]
    );
    Ok(())
}
