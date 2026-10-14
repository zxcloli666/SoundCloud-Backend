use axum::http::StatusCode;
use sqlx::PgPool;

use super::router_identity_tests::send;
use super::router_likes_order_tests::{
    NO_REDIS, expected, like_oldest_first, move_after, move_to_top, order, reorder,
};
use super::router_search_tests::{app, get, seed_tracks, session, urns};

async fn like_days_ago(pool: &PgPool, user_id: &str, id: &str, days: i32) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO user_likes_tracks (user_id, sc_track_id, created_at)
         VALUES ($1, $2, now() - $3 * interval '1 day')",
    )
    .bind(user_id)
    .bind(id)
    .bind(days)
    .execute(pool)
    .await?;
    Ok(())
}

async fn set_like_date(pool: &PgPool, id: &str, days_ago: i32) -> anyhow::Result<()> {
    sqlx::query(
        "UPDATE user_likes_tracks SET created_at = now() - $2 * interval '1 day'
         WHERE user_id = '17' AND sc_track_id = $1",
    )
    .bind(id)
    .bind(days_ago)
    .execute(pool)
    .await?;
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn a_like_with_a_new_date_comes_back_unranked_at_the_top(pool: PgPool) -> anyhow::Result<()> {
    like_oldest_first(&pool, &["1", "2", "3", "4"]).await?;
    let app = app(&pool, NO_REDIS).await?;
    let session = session(&pool).await?;
    assert_eq!(
        reorder(&app, session, move_after("1", "4")).await?,
        StatusCode::NO_CONTENT
    );
    assert_eq!(order(&app, session).await?, expected(&["4", "1", "3", "2"]));

    set_like_date(&pool, "2", 0).await?;
    assert_eq!(order(&app, session).await?, expected(&["2", "4", "1", "3"]));
    assert_eq!(
        reorder(&app, session, move_after("3", "2")).await?,
        StatusCode::NO_CONTENT
    );
    assert_eq!(order(&app, session).await?, expected(&["2", "3", "4", "1"]));

    set_like_date(&pool, "4", 30).await?;
    assert_eq!(order(&app, session).await?, expected(&["4", "2", "3", "1"]));
    assert_eq!(
        reorder(&app, session, move_after("1", "4")).await?,
        StatusCode::NO_CONTENT
    );
    assert_eq!(order(&app, session).await?, expected(&["4", "1", "2", "3"]));
    assert_eq!(
        reorder(&app, session, move_to_top("3")).await?,
        StatusCode::NO_CONTENT
    );
    assert_eq!(order(&app, session).await?, expected(&["3", "4", "1", "2"]));
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn likes_added_between_moves_keep_their_place(pool: PgPool) -> anyhow::Result<()> {
    seed_tracks(&pool, &["1", "2", "3", "4", "5"]).await?;
    for (id, days) in [("1", 10), ("2", 9), ("3", 8)] {
        like_days_ago(&pool, "17", id, days).await?;
    }
    let app = app(&pool, NO_REDIS).await?;
    let session = session(&pool).await?;
    assert_eq!(
        reorder(&app, session, move_after("3", "1")).await?,
        StatusCode::NO_CONTENT
    );
    assert_eq!(order(&app, session).await?, expected(&["2", "1", "3"]));

    like_days_ago(&pool, "17", "4", 5).await?;
    like_days_ago(&pool, "17", "5", 0).await?;
    assert_eq!(
        reorder(&app, session, move_after("5", "1")).await?,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        order(&app, session).await?,
        expected(&["4", "2", "1", "5", "3"])
    );
    assert_eq!(
        reorder(&app, session, move_to_top("3")).await?,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        order(&app, session).await?,
        expected(&["3", "4", "2", "1", "5"])
    );
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn liking_a_liked_track_again_keeps_its_place(pool: PgPool) -> anyhow::Result<()> {
    like_oldest_first(&pool, &["1", "2", "3"]).await?;
    let app = app(&pool, NO_REDIS).await?;
    let session = session(&pool).await?;
    assert_eq!(
        reorder(&app, session, move_to_top("1")).await?,
        StatusCode::NO_CONTENT
    );
    assert_eq!(order(&app, session).await?, expected(&["1", "3", "2"]));

    let (status, _, _) = send(&app, session, "POST", "/likes/tracks/2", None).await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(order(&app, session).await?, expected(&["1", "3", "2"]));
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn likes_under_the_legacy_user_urn_are_ordered_and_paged(pool: PgPool) -> anyhow::Result<()> {
    seed_tracks(&pool, &["1", "2", "3", "4", "5"]).await?;
    let legacy = "soundcloud:users:17";
    for (user_id, id, days) in [
        (legacy, "1", 10),
        ("17", "2", 9),
        (legacy, "3", 8),
        ("17", "3", 1),
        (legacy, "4", 7),
        ("17", "5", 6),
    ] {
        like_days_ago(&pool, user_id, id, days).await?;
    }
    let app = app(&pool, NO_REDIS).await?;
    let session = session(&pool).await?;
    assert_eq!(
        order(&app, session).await?,
        expected(&["3", "5", "4", "2", "1"])
    );
    assert_eq!(
        reorder(&app, session, move_after("3", "1")).await?,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        reorder(&app, session, move_to_top("1")).await?,
        StatusCode::NO_CONTENT
    );
    let full = expected(&["1", "5", "4", "2", "3"]);
    assert_eq!(order(&app, session).await?, full);

    let mut paged = Vec::new();
    for page in 0.. {
        let uri = format!("/me/likes/tracks?limit=2&page={page}");
        let (status, body) = get(&app, session, &uri).await?;
        assert_eq!(status, StatusCode::OK, "{body}");
        paged.extend(
            urns(&body)
                .into_iter()
                .map(|urn| urn.trim_start_matches("soundcloud:tracks:").to_owned()),
        );
        if body["has_more"] != true {
            break;
        }
    }
    assert_eq!(paged, full);
    Ok(())
}
