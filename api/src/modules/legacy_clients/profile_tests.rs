use std::sync::Arc;

use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

use super::read;
use crate::modules::me::MeService;

const READ_BY_EVERY_OLD_CLIENT: [(&str, Wire); 10] = [
    ("id", Wire::Number),
    ("urn", Wire::Text),
    ("username", Wire::Text),
    ("avatar_url", Wire::Text),
    ("permalink_url", Wire::Text),
    ("followers_count", Wire::Number),
    ("followings_count", Wire::Number),
    ("track_count", Wire::Number),
    ("playlist_count", Wire::Number),
    ("public_favorites_count", Wire::Number),
];
const READ_WHEN_PRESENT: [(&str, Wire); 1] = [("likes_count", Wire::Number)];

#[derive(Clone, Copy)]
enum Wire {
    Number,
    Text,
}

impl Wire {
    fn holds(self, value: &Value) -> bool {
        match self {
            Self::Number => value.is_number(),
            Self::Text => value.is_string(),
        }
    }
}

#[sqlx::test(migrations = "./migrations")]
async fn a_profile_mirrored_from_apiv2_carries_every_field_old_clients_read(
    pool: PgPool,
) -> anyhow::Result<()> {
    let mut mirrored = apiv2_user();
    sc_transport::normalize_v2_to_v1(&mut mirrored);
    mirror(&pool, &mirrored).await?;

    let profile = read(&*service(&pool)?, "17").await?;

    assert_eq!(unreadable_fields(&profile), Vec::<String>::new());
    assert_eq!(profile["public_favorites_count"], mirrored["likes_count"]);
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn a_profile_mirrored_from_apiv1_reaches_old_clients_unchanged(
    pool: PgPool,
) -> anyhow::Result<()> {
    let mirrored = apiv1_me();
    mirror(&pool, &mirrored).await?;

    let profile = read(&*service(&pool)?, "17").await?;

    assert_eq!(unreadable_fields(&profile), Vec::<String>::new());
    assert_eq!(profile, mirrored);
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn a_mirrored_profile_without_any_like_counter_reads_as_no_likes(
    pool: PgPool,
) -> anyhow::Result<()> {
    let mut mirrored = apiv2_user();
    if let Some(object) = mirrored.as_object_mut() {
        object.remove("likes_count");
    }
    mirror(&pool, &mirrored).await?;

    let profile = read(&*service(&pool)?, "17").await?;

    assert_eq!(unreadable_fields(&profile), Vec::<String>::new());
    assert_eq!(profile["public_favorites_count"], json!(0));
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn the_user_mirror_fallback_carries_every_field_old_clients_read(
    pool: PgPool,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO users (sc_user_id, urn, username, username_normalized)
         VALUES ('17', 'soundcloud:users:17', 'Mirrored', 'mirrored')",
    )
    .execute(&pool)
    .await?;

    let profile = read(&*service(&pool)?, "17").await?;

    assert_eq!(unreadable_fields(&profile), Vec::<String>::new());
    assert_eq!(
        (&profile["id"], &profile["username"]),
        (&Value::from(17), &Value::from("Mirrored"))
    );
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn the_session_fallback_carries_every_field_old_clients_read(
    pool: PgPool,
) -> anyhow::Result<()> {
    let connection_id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO soundcloud_connections
            (id, soundcloud_user_id, username, access_token, refresh_token, expires_at, scope)
         VALUES ($1, '17', 'Listener', 'access', 'refresh', now(), '')",
    )
    .bind(connection_id)
    .execute(&pool)
    .await?;
    sqlx::query("INSERT INTO sessions (id, soundcloud_connection_id) VALUES ($1, $2)")
        .bind(Uuid::now_v7())
        .bind(connection_id)
        .execute(&pool)
        .await?;

    let profile = read(&*service(&pool)?, "soundcloud:users:17").await?;

    assert_eq!(unreadable_fields(&profile), Vec::<String>::new());
    assert_eq!(
        (&profile["urn"], &profile["username"]),
        (
            &Value::from("soundcloud:users:17"),
            &Value::from("Listener")
        )
    );
    Ok(())
}

fn apiv2_user() -> Value {
    json!({
        "avatar_url": "https://i1.sndcdn.com/avatars-000000000017-listener-large.jpg",
        "badges": {"pro": false, "creator_mid_tier": false, "pro_unlimited": false, "verified": false},
        "city": "",
        "comments_count": 0,
        "country_code": null,
        "created_at": "2015-04-01T10:00:00Z",
        "creator_subscription": {"product": {"id": "free"}},
        "creator_subscriptions": [{"product": {"id": "free"}}],
        "date_of_birth": null,
        "description": null,
        "first_name": "",
        "followers_count": 12,
        "followings_count": 34,
        "full_name": "",
        "groups_count": 0,
        "id": 17,
        "kind": "user",
        "last_modified": "2026-09-01T10:00:00Z",
        "last_name": "",
        "likes_count": 42,
        "permalink": "listener",
        "permalink_url": "https://soundcloud.com/listener",
        "playlist_count": 2,
        "playlist_likes_count": 3,
        "reposts_count": null,
        "station_permalink": "artist-stations:17",
        "station_urn": "soundcloud:system-playlists:artist-stations:17",
        "track_count": 0,
        "uri": "https://api.soundcloud.com/users/soundcloud%3Ausers%3A17",
        "urn": "soundcloud:users:17",
        "username": "Listener",
        "verified": false,
        "visuals": null
    })
}

fn apiv1_me() -> Value {
    json!({
        "avatar_url": "https://i1.sndcdn.com/avatars-000000000017-listener-large.jpg",
        "city": null,
        "comments_count": 0,
        "country": null,
        "created_at": "2015/04/01 10:00:00 +0000",
        "description": null,
        "followers_count": 12,
        "followings_count": 34,
        "full_name": "",
        "id": 17,
        "kind": "user",
        "likes_count": 42,
        "permalink": "listener",
        "permalink_url": "https://soundcloud.com/listener",
        "plan": "Free",
        "playlist_count": 2,
        "private_playlists_count": 1,
        "private_tracks_count": 0,
        "public_favorites_count": 40,
        "reposts_count": 0,
        "track_count": 0,
        "uri": "https://api.soundcloud.com/users/soundcloud:users:17",
        "urn": "soundcloud:users:17",
        "username": "Listener"
    })
}

async fn mirror(pool: &PgPool, profile: &Value) -> anyhow::Result<()> {
    sqlx::query("INSERT INTO user_profiles (soundcloud_user_id, profile_json) VALUES ('17', $1)")
        .bind(profile)
        .execute(pool)
        .await?;
    Ok(())
}

fn unreadable_fields(profile: &Value) -> Vec<String> {
    let required = READ_BY_EVERY_OLD_CLIENT
        .iter()
        .filter(|(field, wire)| !profile.get(*field).is_some_and(|value| wire.holds(value)));
    let optional = READ_WHEN_PRESENT
        .iter()
        .filter(|(field, wire)| profile.get(*field).is_some_and(|value| !wire.holds(value)));
    required
        .chain(optional)
        .map(|(field, _)| format!("{field}: {:?}", profile.get(*field)))
        .collect()
}

fn service(pool: &PgPool) -> anyhow::Result<Arc<MeService>> {
    let redis = deadpool_redis::Config::from_url("redis://127.0.0.1:1")
        .create_pool(Some(deadpool_redis::Runtime::Tokio1))?;
    let cold = crate::modules::cold_refresh::ColdRefreshService::new(
        pool.clone(),
        crate::config::ColdCfg {
            track_ttl_sec: 3600,
            user_ttl_sec: 3600,
            playlist_ttl_sec: 3600,
            liked_tracks_ttl_sec: 3600,
            liked_playlists_ttl_sec: 3600,
            followings_ttl_sec: 3600,
            owned_ttl_sec: 300,
            evict_after_sec: 86400,
        },
    );
    Ok(MeService::new(
        pool.clone(),
        crate::modules::sync_queue::SyncQueueService::new(pool.clone(), redis),
        cold,
    ))
}
