use backend_contracts::admin_catalog::{AdminCatalogReply, AdminCatalogRequest, AdminCommand};
use sqlx::PgPool;
use uuid::Uuid;

pub async fn send(pool: &PgPool, command: AdminCommand) -> AdminCatalogReply {
    send_as(pool, Uuid::new_v4(), command).await
}

pub async fn send_as(pool: &PgPool, request_id: Uuid, command: AdminCommand) -> AdminCatalogReply {
    crate::execute(
        pool,
        &AdminCatalogRequest {
            request_id,
            command,
        },
    )
    .await
    .unwrap()
}

pub async fn artist(pool: &PgPool, name: &str) -> Uuid {
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO artists (id, name, normalized_name, source) VALUES ($1, $2, lower($2), 'genius')",
    )
    .bind(id)
    .bind(name)
    .execute(pool)
    .await
    .unwrap();
    id
}

pub async fn track(pool: &PgPool, uploader: Option<&str>) -> Uuid {
    let id = Uuid::new_v4();
    let sc_track_id = (id.as_u128() % 1_000_000_000_000).to_string();
    sqlx::query(
        "INSERT INTO tracks (id, sc_track_id, urn, title, title_normalized, duration_ms, uploader_sc_user_id)
         VALUES ($1, $2, 'soundcloud:tracks:' || $2, 'Night Drive', 'night drive', 213000, $3)",
    )
    .bind(id)
    .bind(sc_track_id)
    .bind(uploader)
    .execute(pool)
    .await
    .unwrap();
    id
}

pub async fn sc_track_id(pool: &PgPool, track: Uuid) -> String {
    sqlx::query_scalar("SELECT sc_track_id FROM tracks WHERE id = $1")
        .bind(track)
        .fetch_one(pool)
        .await
        .unwrap()
}

pub async fn credit(pool: &PgPool, track: Uuid, artist: Uuid, role: &str) {
    sqlx::query(
        "INSERT INTO track_artists (track_id, artist_id, role, source) VALUES ($1, $2, $3, 'manual')",
    )
    .bind(track)
    .bind(artist)
    .bind(role)
    .execute(pool)
    .await
    .unwrap();
}

pub async fn credits(pool: &PgPool, track: Uuid) -> Vec<(Uuid, String)> {
    sqlx::query_as(
        "SELECT artist_id, role::text FROM track_artists WHERE track_id = $1 ORDER BY role, artist_id",
    )
    .bind(track)
    .fetch_all(pool)
    .await
    .unwrap()
}

pub async fn primary(pool: &PgPool, track: Uuid) -> Option<Uuid> {
    sqlx::query_scalar("SELECT primary_artist_id FROM tracks WHERE id = $1")
        .bind(track)
        .fetch_one(pool)
        .await
        .unwrap()
}

pub async fn blocks(pool: &PgPool, track: Uuid) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM track_artist_blocks WHERE track_id = $1")
        .bind(track)
        .fetch_one(pool)
        .await
        .unwrap()
}
