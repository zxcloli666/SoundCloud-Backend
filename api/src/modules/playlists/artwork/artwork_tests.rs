use std::io::Cursor;

use aws_sdk_s3::config::{BehaviorVersion, Credentials, Region};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use image::{DynamicImage, ImageFormat, Rgb, RgbImage};
use serde_json::{Value, json};
use sqlx::PgPool;

use crate::error::AppError;
use crate::modules::sync_queue::SyncQueueService;

use super::{ArtworkStore, PlaylistArtwork};

const PLAYLIST: &str = "soundcloud:playlists:42";

async fn artwork(pool: &PgPool, store: Option<ArtworkStore>) -> anyhow::Result<PlaylistArtwork> {
    catalog_ingest::upsert_playlist_from_sc(
        pool,
        &json!({
            "urn": PLAYLIST, "title": "Mine", "sharing": "public", "track_count": 0,
            "user": {"urn": "soundcloud:users:17"}
        }),
        catalog_ingest::Observation::begin(pool).await?,
    )
    .await?;
    sqlx::query("INSERT INTO user_owned_playlists (user_id, playlist_urn) VALUES ('17', $1)")
        .bind(PLAYLIST)
        .execute(pool)
        .await?;
    let redis = deadpool_redis::Config::from_url("redis://127.0.0.1:1")
        .create_pool(Some(deadpool_redis::Runtime::Tokio1))?;
    Ok(PlaylistArtwork::new(
        pool.clone(),
        SyncQueueService::new(pool.clone(), redis),
        store,
    ))
}

fn unreachable_store() -> ArtworkStore {
    ArtworkStore::new("http://127.0.0.1:1", "covers", "key", "secret")
}

fn cover(width: u32, height: u32) -> String {
    let mut png = Cursor::new(Vec::new());
    DynamicImage::from(RgbImage::from_pixel(width, height, Rgb([200, 30, 90])))
        .write_to(&mut png, ImageFormat::Png)
        .expect("test cover encodes");
    STANDARD.encode(png.into_inner())
}

fn refusal(result: Result<Value, AppError>) -> (u16, &'static str) {
    let error = result.expect_err("the cover is refused");
    (error.status().as_u16(), error.public_code())
}

async fn queued(pool: &PgPool) -> anyhow::Result<Vec<(String, String, Value)>> {
    Ok(sqlx::query_as(
        "SELECT user_id, target_urn, payload FROM sync_queue WHERE action_type = 'playlist_artwork'",
    )
    .fetch_all(pool)
    .await?)
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn only_the_owner_can_change_a_cover(pool: PgPool) -> anyhow::Result<()> {
    let artwork = artwork(&pool, Some(unreachable_store())).await?;

    let stranger = artwork.change("18", "42", &cover(400, 400)).await;

    assert_eq!(
        stranger
            .expect_err("a stranger is refused")
            .status()
            .as_u16(),
        404
    );
    assert!(queued(&pool).await?.is_empty());
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn a_cover_waits_for_a_configured_store(pool: PgPool) -> anyhow::Result<()> {
    let artwork = artwork(&pool, None).await?;

    let answer = artwork.change("17", PLAYLIST, &cover(400, 400)).await;

    assert_eq!(refusal(answer), (503, "artwork_unavailable"));
    assert!(queued(&pool).await?.is_empty());
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn a_bad_cover_is_refused_before_anything_is_staged(pool: PgPool) -> anyhow::Result<()> {
    let artwork = artwork(&pool, Some(unreachable_store())).await?;

    assert_eq!(
        refusal(artwork.change("17", PLAYLIST, "%%%").await),
        (400, "artwork_invalid")
    );
    assert_eq!(
        refusal(
            artwork
                .change("17", PLAYLIST, &STANDARD.encode(b"GIF89a...."))
                .await
        ),
        (415, "artwork_unsupported")
    );
    assert_eq!(
        refusal(artwork.change("17", PLAYLIST, &cover(120, 120)).await),
        (422, "artwork_too_small")
    );
    let oversized = STANDARD.encode(vec![0_u8; super::IMAGE_MAX_BYTES + 1]);
    assert_eq!(
        refusal(artwork.change("17", PLAYLIST, &oversized).await),
        (413, "artwork_too_large")
    );
    assert!(queued(&pool).await?.is_empty());
    Ok(())
}

fn live_env(name: &str) -> String {
    std::env::var(name)
        .unwrap_or_else(|_| panic!("{name} is not set: this test needs a live SeaweedFS"))
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "needs a live SeaweedFS (CHAT_TEST_S3_ENDPOINT)"]
async fn an_owner_cover_is_staged_as_a_square_jpeg_and_queued_s3_live(
    pool: PgPool,
) -> anyhow::Result<()> {
    let (endpoint, access_key, secret_key) = (
        live_env("CHAT_TEST_S3_ENDPOINT"),
        live_env("CHAT_TEST_S3_ACCESS_KEY"),
        live_env("CHAT_TEST_S3_SECRET_KEY"),
    );
    let bucket = format!(
        "covers-live-{}",
        &uuid::Uuid::new_v4().simple().to_string()[..12]
    );
    let admin = aws_sdk_s3::Client::from_conf(
        aws_sdk_s3::Config::builder()
            .behavior_version(BehaviorVersion::latest())
            .region(Region::new("us-east-1"))
            .endpoint_url(&endpoint)
            .credentials_provider(Credentials::new(
                &access_key,
                &secret_key,
                None,
                None,
                "covers-live",
            ))
            .force_path_style(true)
            .build(),
    );
    admin.create_bucket().bucket(&bucket).send().await?;
    let artwork = artwork(
        &pool,
        Some(ArtworkStore::new(
            &endpoint,
            &bucket,
            &access_key,
            &secret_key,
        )),
    )
    .await?;

    let answer = artwork.change("17", "42", &cover(1600, 1200)).await?;

    assert_eq!(
        answer,
        json!({"status": "queued", "actionType": "playlist_artwork", "targetUrn": PLAYLIST})
    );
    let queued = queued(&pool).await?;
    assert_eq!(queued.len(), 1);
    let (user, target, payload) = &queued[0];
    assert_eq!((user.as_str(), target.as_str()), ("17", PLAYLIST));
    let key = payload["key"].as_str().expect("a staged key");
    assert!(key.starts_with("42/") && key.ends_with(".jpg"));
    let staged = admin.get_object().bucket(&bucket).key(key).send().await?;
    assert_eq!(staged.content_type(), Some("image/jpeg"));
    let staged = image::load_from_memory(&staged.body.collect().await?.into_bytes())?;
    assert_eq!((staged.width(), staged.height()), (1000, 1000));
    Ok(())
}
