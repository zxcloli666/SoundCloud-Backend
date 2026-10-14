use axum::http::StatusCode;
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

use super::router_identity_tests::send;
use super::router_search_tests::{LISTENER, app, get, seed_tracks, session, urns};

pub(super) const NO_REDIS: &str = "redis://127.0.0.1:1";
const ORDER: &str = "/me/likes/tracks/order";

pub(super) async fn like_oldest_first(pool: &PgPool, ids: &[&str]) -> anyhow::Result<()> {
    seed_tracks(pool, ids).await?;
    for (age, id) in ids.iter().rev().enumerate() {
        sqlx::query(
            "INSERT INTO user_likes_tracks (user_id, sc_track_id, created_at)
             VALUES ($1, $2, now() - ($3 + 1) * interval '1 minute')",
        )
        .bind(LISTENER)
        .bind(id)
        .bind(age as i32)
        .execute(pool)
        .await?;
    }
    Ok(())
}

pub(super) async fn order(app: &axum::Router, session: Uuid) -> anyhow::Result<Vec<String>> {
    let (status, body) = get(app, session, "/me/likes/tracks?limit=200").await?;
    assert_eq!(status, StatusCode::OK, "{body}");
    Ok(urns(&body)
        .into_iter()
        .map(|urn| urn.trim_start_matches("soundcloud:tracks:").to_owned())
        .collect())
}

pub(super) async fn reorder(
    app: &axum::Router,
    session: Uuid,
    body: Value,
) -> anyhow::Result<StatusCode> {
    let (status, _, _) = send(app, session, "POST", ORDER, Some(body)).await?;
    Ok(status)
}

async fn visitor_session(pool: &PgPool) -> anyhow::Result<Uuid> {
    let connection = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO soundcloud_connections
            (id, soundcloud_user_id, username, access_token, refresh_token, expires_at, scope)
         VALUES ($1, '18', 'Visitor', 'access', 'refresh', now() + interval '1 hour', '')",
    )
    .bind(connection)
    .execute(pool)
    .await?;
    let session = Uuid::now_v7();
    sqlx::query("INSERT INTO sessions (id, soundcloud_connection_id) VALUES ($1, $2)")
        .bind(session)
        .bind(connection)
        .execute(pool)
        .await?;
    Ok(session)
}

pub(super) fn move_after(track: &str, after: &str) -> Value {
    json!({
        "action": "move_after",
        "track": format!("soundcloud:tracks:{track}"),
        "after": format!("soundcloud:tracks:{after}"),
    })
}

pub(super) fn move_to_top(track: &str) -> Value {
    json!({"action": "move_to_top", "track": format!("soundcloud:tracks:{track}")})
}

fn move_to_bottom(track: &str) -> Value {
    json!({"action": "move_to_bottom", "track": format!("soundcloud:tracks:{track}")})
}

pub(super) fn expected(ids: &[&str]) -> Vec<String> {
    ids.iter().map(|id| (*id).to_owned()).collect()
}

#[sqlx::test(migrations = "./migrations")]
async fn the_owner_order_survives_moves_new_likes_relikes_and_reset(
    pool: PgPool,
) -> anyhow::Result<()> {
    like_oldest_first(&pool, &["1", "2", "3", "4", "5"]).await?;
    seed_tracks(&pool, &["6"]).await?;
    let app = app(&pool, NO_REDIS).await?;
    let session = session(&pool).await?;
    assert_eq!(
        order(&app, session).await?,
        expected(&["5", "4", "3", "2", "1"])
    );

    assert_eq!(
        reorder(&app, session, move_after("1", "4")).await?,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        order(&app, session).await?,
        expected(&["5", "4", "1", "3", "2"])
    );
    assert_eq!(
        reorder(&app, session, move_to_bottom("5")).await?,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        order(&app, session).await?,
        expected(&["4", "1", "3", "2", "5"])
    );
    assert_eq!(
        reorder(&app, session, move_to_top("2")).await?,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        order(&app, session).await?,
        expected(&["2", "4", "1", "3", "5"])
    );

    let (status, _, _) = send(&app, session, "POST", "/likes/tracks/6", None).await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        order(&app, session).await?,
        expected(&["6", "2", "4", "1", "3", "5"])
    );
    assert_eq!(
        reorder(&app, session, move_after("6", "4")).await?,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        order(&app, session).await?,
        expected(&["2", "4", "6", "1", "3", "5"])
    );

    let (status, _, _) = send(&app, session, "DELETE", "/likes/tracks/4", None).await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        order(&app, session).await?,
        expected(&["2", "6", "1", "3", "5"])
    );
    let (status, _, _) = send(&app, session, "POST", "/likes/tracks/4", None).await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        order(&app, session).await?,
        expected(&["4", "2", "6", "1", "3", "5"])
    );

    assert_eq!(
        reorder(&app, session, json!({"action": "reset"})).await?,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        order(&app, session).await?,
        expected(&["4", "6", "5", "3", "2", "1"])
    );
    let ranks: i64 = sqlx::query_scalar("SELECT count(*) FROM user_like_ranks")
        .fetch_one(&pool)
        .await?;
    assert_eq!(ranks, 0);
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn exhausted_gaps_are_renumbered_without_changing_the_order(
    pool: PgPool,
) -> anyhow::Result<()> {
    like_oldest_first(&pool, &["1", "2", "3", "4"]).await?;
    let app = app(&pool, NO_REDIS).await?;
    let session = session(&pool).await?;
    let mut model = expected(&["4", "3", "2", "1"]);
    for round in 0..40 {
        let track = if round % 2 == 0 { "1" } else { "2" };
        assert_eq!(
            reorder(&app, session, move_after(track, "4")).await?,
            StatusCode::NO_CONTENT
        );
        model.retain(|id| id != track);
        model.insert(1, track.to_owned());
        assert_eq!(order(&app, session).await?, model, "round {round}");
    }
    let low: i64 = sqlx::query_scalar("SELECT min(rank) FROM user_like_ranks")
        .fetch_one(&pool)
        .await?;
    assert!(low > 0, "the ranks were renumbered: {low}");
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn only_live_likes_of_the_owner_can_be_moved(pool: PgPool) -> anyhow::Result<()> {
    like_oldest_first(&pool, &["1", "2"]).await?;
    seed_tracks(&pool, &["3"]).await?;
    sqlx::query("INSERT INTO user_likes_tracks (user_id, sc_track_id) VALUES ('99', '3')")
        .execute(&pool)
        .await?;
    let app = app(&pool, NO_REDIS).await?;
    let session = session(&pool).await?;
    for body in [move_to_top("3"), move_after("1", "3")] {
        let (status, _, error) = send(&app, session, "POST", ORDER, Some(body)).await?;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(error["code"], "like_not_found");
    }
    assert_eq!(
        reorder(&app, session, move_after("1", "1")).await?,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        reorder(
            &app,
            session,
            json!({"action": "move_to_top", "track": "soundcloud:users:1"})
        )
        .await?,
        StatusCode::BAD_REQUEST
    );
    for body in [
        json!({"action": "move_after", "track": "soundcloud:tracks:1"}),
        json!({"action": "move_after", "track": "soundcloud:tracks:1", "afterUrn": "soundcloud:tracks:2"}),
        json!({"action": "move_to_top", "track": "soundcloud:tracks:1", "after": null}),
        json!({"action": "move", "track": "soundcloud:tracks:1", "after": null}),
    ] {
        assert_eq!(
            reorder(&app, session, body.clone()).await?,
            StatusCode::UNPROCESSABLE_ENTITY,
            "{body}"
        );
    }
    assert_eq!(order(&app, session).await?, expected(&["2", "1"]));
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn other_listeners_still_see_the_likes_by_date(pool: PgPool) -> anyhow::Result<()> {
    like_oldest_first(&pool, &["1", "2", "3"]).await?;
    let app = app(&pool, NO_REDIS).await?;
    let session = session(&pool).await?;
    assert_eq!(
        reorder(&app, session, move_to_bottom("3")).await?,
        StatusCode::NO_CONTENT
    );
    assert_eq!(order(&app, session).await?, expected(&["2", "1", "3"]));

    let visitor = visitor_session(&pool).await?;
    let (status, body) = get(
        &app,
        visitor,
        "/users/soundcloud:users:17/likes/tracks?limit=200",
    )
    .await?;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        urns(&body),
        vec![
            "soundcloud:tracks:3".to_owned(),
            "soundcloud:tracks:2".to_owned(),
            "soundcloud:tracks:1".to_owned()
        ]
    );
    Ok(())
}
