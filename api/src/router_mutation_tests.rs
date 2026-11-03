use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

use super::router_search_tests::{LISTENER, app, session};

const NO_REDIS: &str = "redis://127.0.0.1:1";
const PLAYLIST: &str = "soundcloud:playlists:77";

async fn seed(pool: &PgPool) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO tracks (sc_track_id, urn, title, title_normalized, duration_ms, uploader_sc_user_id, sharing)
         VALUES ('42', 'soundcloud:tracks:42', 'Mine', 'mine', 200000, $1, 'public'),
                ('43', 'soundcloud:tracks:43', 'Other', 'other', 200000, '555', 'public');
         ",
    )
    .bind(LISTENER)
    .execute(pool)
    .await?;
    sqlx::query("INSERT INTO user_owned_tracks (user_id, sc_track_id) VALUES ($1, '42')")
        .bind(LISTENER)
        .execute(pool)
        .await?;
    sqlx::query(
        "INSERT INTO playlists (sc_playlist_id, urn, title, title_normalized, owner_sc_user_id, track_count, sharing)
         VALUES ('77', $1, 'Mix', 'mix', $2, 1, 'public')",
    )
    .bind(PLAYLIST)
    .bind(LISTENER)
    .execute(pool)
    .await?;
    sqlx::query("INSERT INTO user_owned_playlists (user_id, playlist_urn) VALUES ($1, $2)")
        .bind(LISTENER)
        .bind(PLAYLIST)
        .execute(pool)
        .await?;
    sqlx::query(
        "INSERT INTO playlist_membership_state (playlist_urn, projection_track_count, sync_status)
         VALUES ($1, 1, 'clean')",
    )
    .bind(PLAYLIST)
    .execute(pool)
    .await?;
    sqlx::query(
        "INSERT INTO playlist_track_projection (playlist_urn, position, sc_track_id) VALUES ($1, 0, '42')",
    )
    .bind(PLAYLIST)
    .execute(pool)
    .await?;
    Ok(())
}

async fn send(
    app: &axum::Router,
    session: Uuid,
    method: &str,
    uri: &str,
    key: Option<Uuid>,
    body: Option<Value>,
) -> anyhow::Result<(StatusCode, Value)> {
    let mut request = Request::builder()
        .method(method)
        .uri(uri)
        .header("x-session-id", session.to_string());
    if let Some(key) = key {
        request = request.header("idempotency-key", key.to_string());
    }
    let request = match body {
        Some(body) => request
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))?,
        None => request.body(Body::empty())?,
    };
    let response = app.clone().oneshot(request).await?;
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 4 * 1024 * 1024).await?;
    Ok((
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    ))
}

async fn one<T>(pool: &PgPool, sql: &str) -> anyhow::Result<T>
where
    T: for<'r> sqlx::Decode<'r, sqlx::Postgres> + sqlx::Type<sqlx::Postgres> + Send + Unpin,
{
    Ok(sqlx::query_scalar(sql).fetch_one(pool).await?)
}

async fn record_writes(pool: &PgPool) -> anyhow::Result<()> {
    let mut sql = String::from(
        "CREATE TABLE lock_order_writes (
             n bigserial PRIMARY KEY,
             xid bigint NOT NULL DEFAULT txid_current(),
             tbl text NOT NULL
         );
         CREATE FUNCTION lock_order_write() RETURNS trigger LANGUAGE plpgsql AS $f$
         BEGIN
             INSERT INTO lock_order_writes (tbl) VALUES (TG_TABLE_NAME);
             RETURN NULL;
         END
         $f$;",
    );
    for table in [
        "playlist_membership_state",
        "playlists",
        "playlist_track_projection",
        "playlist_membership_operations",
        "tracks",
        "sync_queue",
        "user_owned_playlists",
        "user_owned_tracks",
    ] {
        sql.push_str(&format!(
            "CREATE TRIGGER lock_order_write AFTER INSERT OR UPDATE OR DELETE ON {table}
             FOR EACH ROW EXECUTE FUNCTION lock_order_write();"
        ));
    }
    sqlx::raw_sql(&sql).execute(pool).await?;
    Ok(())
}

async fn writes(pool: &PgPool) -> anyhow::Result<Vec<String>> {
    let mut tables: Vec<String> = sqlx::query_scalar(
        "WITH taken AS (DELETE FROM lock_order_writes RETURNING n, xid, tbl)
         SELECT tbl FROM taken
         WHERE xid = (SELECT xid FROM taken ORDER BY n LIMIT 1)
         ORDER BY n",
    )
    .fetch_all(pool)
    .await?;
    tables.dedup();
    Ok(tables)
}

#[sqlx::test(migrations = "./migrations")]
async fn a_track_edit_and_delete_reach_the_catalog_and_the_queue_together(
    pool: PgPool,
) -> anyhow::Result<()> {
    seed(&pool).await?;
    let app = app(&pool, NO_REDIS).await?;
    let session = session(&pool).await?;

    let edit = json!({"track": {"title": "Renamed"}});
    let (status, body) = send(
        &app,
        session,
        "PUT",
        "/tracks/soundcloud:tracks:42",
        None,
        Some(edit.clone()),
    )
    .await?;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["actionType"], "track_update");
    let row: (String, Value) =
        sqlx::query_as("SELECT title, sc_desired FROM tracks WHERE sc_track_id = '42'")
            .fetch_one(&pool)
            .await?;
    assert_eq!(row.0, "Renamed");
    assert_eq!(row.1["title"], "Renamed");

    let (status, _) = send(
        &app,
        session,
        "PUT",
        "/tracks/soundcloud:tracks:43",
        None,
        Some(edit),
    )
    .await?;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(
        one::<String>(&pool, "SELECT title FROM tracks WHERE sc_track_id = '43'").await?,
        "Other"
    );
    assert_eq!(
        one::<i64>(
            &pool,
            "SELECT count(*) FROM sync_queue WHERE target_urn = 'soundcloud:tracks:43'"
        )
        .await?,
        0
    );

    let (status, body) = send(
        &app,
        session,
        "DELETE",
        "/tracks/soundcloud:tracks:42",
        None,
        None,
    )
    .await?;
    assert_eq!(status, StatusCode::OK, "{body}");
    let row: (bool, Value, String) = sqlx::query_as(
        "SELECT deleted_at IS NOT NULL, sc_desired, sharing FROM tracks WHERE sc_track_id = '42'",
    )
    .fetch_one(&pool)
    .await?;
    assert_eq!(row, (true, json!({}), "private".to_owned()));
    assert_eq!(
        one::<i64>(&pool, "SELECT count(*) FROM user_owned_tracks").await?,
        0
    );
    let queued: Vec<String> = sqlx::query_scalar(
        "SELECT action_type FROM sync_queue WHERE target_urn = 'soundcloud:tracks:42' AND NOT cancelled ORDER BY 1",
    )
    .fetch_all(&pool)
    .await?;
    assert_eq!(queued, ["track_delete"]);
    assert_eq!(
        one::<i64>(&pool, "SELECT count(*) FROM cmd_outbox").await?,
        0,
        "a single-node api runs the kinds locally and ships nothing"
    );
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn a_membership_edit_is_journalled_once_per_idempotency_key(
    pool: PgPool,
) -> anyhow::Result<()> {
    seed(&pool).await?;
    let app = app(&pool, NO_REDIS).await?;
    let session = session(&pool).await?;
    let uri = format!("/playlists/{PLAYLIST}/tracks");
    let key = Uuid::now_v7();

    for _ in 0..2 {
        let (status, body) = send(
            &app,
            session,
            "POST",
            &uri,
            Some(key),
            Some(json!({"add": "soundcloud:tracks:43"})),
        )
        .await?;
        assert_eq!(status, StatusCode::OK, "{body}");
    }
    let projection: Vec<String> = sqlx::query_scalar(
        "SELECT sc_track_id FROM playlist_track_projection WHERE playlist_urn = $1 ORDER BY position",
    )
    .bind(PLAYLIST)
    .fetch_all(&pool)
    .await?;
    assert_eq!(projection, ["42", "43"]);
    assert_eq!(
        one::<i64>(&pool, "SELECT count(*) FROM playlist_membership_operations").await?,
        1
    );
    assert_eq!(
        one::<i64>(
            &pool,
            "SELECT projection_revision FROM playlist_membership_state"
        )
        .await?,
        1
    );

    let stale = json!({
        "playlist": {"tracks": [{"urn": "soundcloud:tracks:42"}]},
        "expectedProjectionRevision": 0
    });
    let (status, body) = send(
        &app,
        session,
        "PUT",
        &format!("/playlists/{PLAYLIST}?replace=true"),
        Some(Uuid::now_v7()),
        Some(stale),
    )
    .await?;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(
        one::<i64>(&pool, "SELECT count(*) FROM playlist_membership_operations").await?,
        1
    );
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn a_playlist_edit_and_delete_reach_the_catalog_and_retire_the_membership(
    pool: PgPool,
) -> anyhow::Result<()> {
    seed(&pool).await?;
    let app = app(&pool, NO_REDIS).await?;
    let session = session(&pool).await?;
    let uri = format!("/playlists/{PLAYLIST}");

    let (status, body) = send(
        &app,
        session,
        "PUT",
        &uri,
        Some(Uuid::now_v7()),
        Some(json!({"playlist": {"title": "Renamed"}})),
    )
    .await?;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["actionType"], "playlist_update");
    let row: (String, Value) = sqlx::query_as("SELECT title, sc_desired FROM playlists")
        .fetch_one(&pool)
        .await?;
    assert_eq!(row.0, "Renamed");
    assert_eq!(row.1["title"], "Renamed");

    let (status, body) = send(&app, session, "DELETE", &uri, None, None).await?;
    assert_eq!(status, StatusCode::OK, "{body}");
    let row: (bool, i64, bool) = sqlx::query_as(
        "SELECT p.deleted_at IS NOT NULL, s.reconcile_generation, s.next_reconcile_at IS NULL
         FROM playlists p JOIN playlist_membership_state s ON s.playlist_urn = p.urn",
    )
    .fetch_one(&pool)
    .await?;
    assert_eq!(row, (true, 1, true));
    assert_eq!(
        one::<i64>(&pool, "SELECT count(*) FROM user_owned_playlists").await?,
        0
    );
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn every_edit_writes_the_state_or_catalog_row_first_and_the_queue_last(
    pool: PgPool,
) -> anyhow::Result<()> {
    seed(&pool).await?;
    record_writes(&pool).await?;
    let app = app(&pool, NO_REDIS).await?;
    let session = session(&pool).await?;
    let both = json!({
        "playlist": {"title": "Both", "tracks": [{"urn": "soundcloud:tracks:42"}, {"urn": "soundcloud:tracks:43"}]},
        "expectedProjectionRevision": 0
    });
    let (status, body) = send(
        &app,
        session,
        "PUT",
        &format!("/playlists/{PLAYLIST}?replace=true"),
        Some(Uuid::now_v7()),
        Some(both),
    )
    .await?;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        writes(&pool).await?,
        [
            "playlist_membership_state",
            "playlist_membership_operations",
            "playlist_track_projection",
            "playlists",
            "sync_queue"
        ]
    );
    let (status, body) = send(
        &app,
        session,
        "DELETE",
        &format!("/playlists/{PLAYLIST}"),
        None,
        None,
    )
    .await?;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        writes(&pool).await?,
        [
            "playlist_membership_state",
            "playlists",
            "user_owned_playlists",
            "sync_queue"
        ]
    );
    let edit = json!({"track": {"title": "Renamed"}});
    let (status, body) = send(
        &app,
        session,
        "PUT",
        "/tracks/soundcloud:tracks:42",
        None,
        Some(edit),
    )
    .await?;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(writes(&pool).await?, ["tracks", "sync_queue"]);
    let (status, body) = send(
        &app,
        session,
        "DELETE",
        "/tracks/soundcloud:tracks:42",
        None,
        None,
    )
    .await?;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        writes(&pool).await?,
        ["tracks", "user_owned_tracks", "sync_queue"]
    );
    Ok(())
}
