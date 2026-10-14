use serde_json::json;
use sqlx::PgPool;

use super::*;
use crate::modules::likes::cold::apply_user_favorite_flag;

const USER: &str = "soundcloud:users:17";

async fn like(pool: &PgPool, tracks: &[&str]) -> anyhow::Result<()> {
    for track in tracks {
        sqlx::query(
            "INSERT INTO user_likes_tracks (user_id, sc_track_id, wanted_state) VALUES ('17', $1, true)",
        )
        .bind(track)
        .execute(pool)
        .await?;
    }
    Ok(())
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

#[sqlx::test(migrations = "./migrations")]
async fn without_star_the_eleventh_superlike_waits_for_a_free_slot(
    pool: PgPool,
) -> anyhow::Result<()> {
    let tracks = ids(11);
    let refs: Vec<&str> = tracks.iter().map(String::as_str).collect();
    like(&pool, &refs).await?;
    for track in &tracks[..10] {
        superlike(&pool, USER, track, false).await?;
    }
    assert_eq!(
        quota(&pool, USER, false).await?,
        Quota {
            used: 10,
            limit: Some(FREE_LIMIT)
        }
    );

    let refused = superlike(&pool, USER, "11", false).await.unwrap_err();
    assert_eq!(refused.public_code(), "superlike_limit_reached");
    assert_eq!(refused.status(), StatusCode::PAYMENT_REQUIRED);
    assert!(ensure_room(&pool, USER, false).await.is_err());

    let again = superlike(&pool, USER, "1", false).await?;
    assert_eq!(
        again.used, 10,
        "a superlike that is already there is no new one"
    );

    forget(&pool, USER, "1").await?;
    assert_eq!(superlike(&pool, USER, "11", false).await?.used, 10);
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
        superlike(&pool, USER, track, true).await?;
    }
    assert_eq!(
        quota(&pool, USER, true).await?,
        Quota {
            used: 12,
            limit: None
        }
    );
    assert_eq!(quota(&pool, USER, false).await?.used, 12);
    assert!(ensure_room(&pool, USER, false).await.is_err());
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn a_superlike_lives_only_as_long_as_its_like(pool: PgPool) -> anyhow::Result<()> {
    like(&pool, &["5", "6"]).await?;
    superlike(&pool, USER, "5", false).await?;
    superlike(&pool, USER, "6", false).await?;

    relike(&pool, "5").await?;
    sqlx::query("UPDATE user_likes_tracks SET wanted_state = false WHERE sc_track_id = '6'")
        .execute(&pool)
        .await?;
    assert_eq!(quota(&pool, USER, false).await?.used, 0);
    assert!(like_is_alive(&pool, USER, "5").await?);
    assert!(!like_is_alive(&pool, USER, "6").await?);

    assert_eq!(superlike(&pool, USER, "5", false).await?.used, 1);
    let refused = superlike(&pool, USER, "6", false).await.unwrap_err();
    assert_eq!(refused.status(), StatusCode::CONFLICT);
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn track_payloads_carry_the_super_heart_only_for_live_superlikes(
    pool: PgPool,
) -> anyhow::Result<()> {
    like(&pool, &["5", "6", "7"]).await?;
    superlike(&pool, USER, "5", false).await?;
    superlike(&pool, USER, "6", false).await?;
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
