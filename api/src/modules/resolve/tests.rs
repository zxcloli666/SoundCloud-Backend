use super::input::{EntityKey, ResolveInput};
use super::repository;
use serde_json::json;
use sqlx::PgPool;

#[test]
fn resolve_input_normalizes_public_links_without_reusing_secret_capabilities() -> anyhow::Result<()>
{
    let public = ResolveInput::parse(
        " http://www.soundcloud.com/artist/song/?utm_source=share&si=abc#t=12 ",
    )?;
    assert_eq!(public.upstream, "https://soundcloud.com/artist/song");
    assert!(
        public
            .permalinks
            .contains(&"https://soundcloud.com/artist/song".into())
    );
    assert!(!public.requires_upstream);
    assert!(!ResolveInput::parse("https://soundcloud.com/s-artist/s-song")?.requires_upstream);
    for url in [
        "https://soundcloud.com/artist/song/s-secret",
        "https://soundcloud.com/artist/song?secret_token=s-secret",
    ] {
        let secret = ResolveInput::parse(url)?;
        assert!(secret.requires_upstream);
        assert!(secret.permalinks.is_empty());
        assert_eq!(secret.upstream, url);
    }
    assert!(ResolveInput::parse("https://on.soundcloud.com/AbCd")?.short_link);
    for raw in [
        "",
        "soundcloud:users:01",
        "soundcloud:tracks:0",
        "https://example.com/song",
        "https://soundcloud.com.evil.test/song",
        "https://user@soundcloud.com/song",
    ] {
        assert!(ResolveInput::parse(raw).is_err(), "{raw}");
    }
    assert_eq!(
        ResolveInput::parse("soundcloud:users:17")?
            .entity
            .expect("entity")
            .id,
        "17"
    );
    assert!(
        EntityKey::from_payload(&json!({"kind":"track","id":42,"urn":"soundcloud:users:42"}))
            .is_err()
    );
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn local_resolve_rechecks_visibility_for_an_existing_identity(
    pool: PgPool,
) -> anyhow::Result<()> {
    sqlx::query("INSERT INTO users (sc_user_id, urn, username, username_normalized, permalink_url)
        VALUES ('17', 'soundcloud:users:17', 'Local owner', 'local owner', 'https://soundcloud.com/artist')")
        .execute(&pool).await?;
    sqlx::query("INSERT INTO tracks (sc_track_id, urn, title, title_normalized, duration_ms, uploader_sc_user_id, permalink_url)
        VALUES ('42', 'soundcloud:tracks:42', 'Local track', 'local track', 120000, '17', 'https://soundcloud.com/artist/song')")
        .execute(&pool).await?;
    sqlx::query("INSERT INTO playlists (sc_playlist_id, urn, title, title_normalized, owner_sc_user_id, permalink_url)
        VALUES ('42', 'soundcloud:playlists:42', 'Local mix', 'local mix', '17', 'https://soundcloud.com/artist/sets/mix')")
        .execute(&pool).await?;
    for (url, urn) in [
        (
            "http://www.soundcloud.com/artist/song/?si=abc",
            "soundcloud:tracks:42",
        ),
        (
            "https://soundcloud.com/artist/sets/mix",
            "soundcloud:playlists:42",
        ),
        ("https://soundcloud.com/artist", "soundcloud:users:17"),
    ] {
        let key = repository::find(&pool, &ResolveInput::parse(url)?)
            .await?
            .expect("local permalink");
        assert_eq!(key.urn(), urn);
        assert_eq!(
            repository::load(&pool, &key, None, false).await?.value["urn"],
            urn
        );
    }
    let track = EntityKey::parse("soundcloud:tracks:42").expect("track key");
    let playlist = EntityKey::parse("soundcloud:playlists:42").expect("playlist key");
    sqlx::raw_sql(
        "UPDATE tracks SET sharing = 'private'; UPDATE playlists SET sharing = 'private'",
    )
    .execute(&pool)
    .await?;
    for key in [&track, &playlist] {
        assert!(repository::load(&pool, key, None, false).await.is_err());
        assert!(
            repository::load(&pool, key, Some("18"), false)
                .await
                .is_err()
        );
        assert!(
            repository::load(&pool, key, Some("soundcloud:users:17"), false)
                .await
                .is_ok()
        );
        assert!(repository::load(&pool, key, None, true).await.is_ok());
    }
    sqlx::query(
        "UPDATE tracks SET sc_desired = '{\"sharing\":\"private\"}', sc_write_confirmed = false",
    )
    .execute(&pool)
    .await?;
    sqlx::query(
        "UPDATE playlists SET sc_desired = '{\"sharing\":\"private\"}', sc_write_confirmed = false",
    )
    .execute(&pool)
    .await?;
    for key in [&track, &playlist] {
        assert!(repository::load(&pool, key, None, true).await.is_err());
        assert!(
            repository::load(&pool, key, Some("17"), false)
                .await
                .is_ok()
        );
    }
    sqlx::raw_sql("UPDATE tracks SET deleted_at = now(); UPDATE playlists SET deleted_at = now()")
        .execute(&pool)
        .await?;
    for key in [&track, &playlist] {
        assert!(
            repository::find(&pool, &ResolveInput::parse(&key.urn())?)
                .await?
                .is_some()
        );
        assert!(
            repository::load(&pool, key, Some("17"), true)
                .await
                .is_err()
        );
    }
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn resolved_observation_cannot_return_metadata_rejected_by_the_catalog(
    pool: PgPool,
) -> anyhow::Result<()> {
    let old = catalog_ingest::Observation::begin(&pool).await?;
    let current = catalog_ingest::Observation::begin(&pool).await?;
    let repo = crate::modules::users::UserRepository::new(pool.clone());
    let mut payload =
        json!({"kind":"user","id":17,"urn":"soundcloud:users:17","username":"Current name"});
    repo.upsert_from_sc(&payload, current).await?;
    payload["username"] = json!("Obsolete remote name");
    repo.upsert_from_sc(&payload, old).await?;
    let key = EntityKey::from_payload(&payload)?;
    assert_eq!(
        repository::load(&pool, &key, None, false).await?.value["username"],
        "Current name"
    );
    Ok(())
}

#[test]
fn an_unreachable_soundcloud_is_a_retryable_answer_not_a_dead_end() {
    let coded = super::handlers::upstream_unavailable(crate::error::AppError::ScUnreachable(
        "relay: no result".into(),
    ));

    assert!(matches!(
        &coded,
        crate::error::AppError::Coded {
            code: "resolve_upstream_unavailable",
            retry_after_sec: Some(_),
            ..
        }
    ));
    assert_eq!(coded.status(), axum::http::StatusCode::BAD_GATEWAY);
}

#[test]
fn a_real_upstream_answer_is_never_rewritten() {
    let not_found = crate::error::AppError::ScApi {
        status: 404,
        body: serde_json::Value::Null,
        retry_after_sec: None,
    };
    let kept = super::handlers::upstream_unavailable(not_found);
    assert_eq!(kept.status(), axum::http::StatusCode::NOT_FOUND);

    let rejected = crate::error::AppError::bad_request("bad url");
    let kept = super::handlers::upstream_unavailable(rejected);
    assert_eq!(kept.status(), axum::http::StatusCode::BAD_REQUEST);
}
