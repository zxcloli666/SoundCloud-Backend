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

#[test]
fn resolve_input_strips_every_param_but_the_secret_token() -> anyhow::Result<()> {
    let public = ResolveInput::parse(
        "https://soundcloud.com/artist/song?in=artist/sets/mix&t=1:23&ref=clipboard&si=abc",
    )?;
    assert!(!public.requires_upstream);
    assert_eq!(public.upstream, "https://soundcloud.com/artist/song");
    assert!(
        public
            .permalinks
            .contains(&"https://soundcloud.com/artist/song".into())
    );
    let secret = ResolveInput::parse(
        "https://soundcloud.com/artist/song?in=a/sets/b&secret_token=s-tok&t=5",
    )?;
    assert!(secret.requires_upstream);
    assert!(secret.permalinks.is_empty());
    assert_eq!(
        secret.upstream,
        "https://soundcloud.com/artist/song?secret_token=s-tok"
    );
    let short = ResolveInput::parse("https://on.soundcloud.com/AbCd?si=abc&utm_medium=text")?;
    assert!(!short.requires_upstream);
    assert_eq!(short.upstream, "https://on.soundcloud.com/AbCd");
    Ok(())
}

#[test]
fn resolve_input_tells_secret_paths_from_playlists_named_like_tokens() -> anyhow::Result<()> {
    for (url, upstream) in [
        (
            "https://soundcloud.com/artist/song/s-SeCrEt/",
            "https://soundcloud.com/artist/song/s-SeCrEt",
        ),
        (
            "https://m.soundcloud.com/Artist/Song/s-SeCrEt?si=abc",
            "https://soundcloud.com/artist/song/s-SeCrEt",
        ),
        (
            "https://soundcloud.com/artist/Sets/Mix/s-SeCrEt/",
            "https://soundcloud.com/artist/sets/mix/s-SeCrEt",
        ),
    ] {
        let secret = ResolveInput::parse(url)?;
        assert!(secret.requires_upstream, "{url}");
        assert!(secret.permalinks.is_empty(), "{url}");
        assert_eq!(secret.upstream, upstream);
    }
    for url in [
        "https://soundcloud.com/artist/sets/s-mix",
        "https://soundcloud.com/artist/sets/s-mix/",
    ] {
        let playlist = ResolveInput::parse(url)?;
        assert!(!playlist.requires_upstream, "{url}");
        assert_eq!(
            playlist.upstream,
            "https://soundcloud.com/artist/sets/s-mix"
        );
        assert!(
            playlist
                .permalinks
                .contains(&"https://soundcloud.com/artist/sets/s-mix".into())
        );
    }
    Ok(())
}

#[test]
fn resolve_input_asks_soundcloud_with_the_canonical_link() -> anyhow::Result<()> {
    for url in [
        "https://m.soundcloud.com/OfficialMetallica/Nothing-Else-Matters-Live-12/",
        "http://www.SoundCloud.com/officialmetallica/nothing-else-matters-live-12?t=10",
        "https://soundcloud.com/officialmetallica/nothing-else-matters-live-12#t=1:00",
    ] {
        let input = ResolveInput::parse(url)?;
        assert_eq!(
            input.upstream, "https://soundcloud.com/officialmetallica/nothing-else-matters-live-12",
            "{url}"
        );
    }
    let token = ResolveInput::parse("https://SoundCloud.com/Artist/Song?secret_token=s-AbC")?;
    assert_eq!(
        token.upstream,
        "https://soundcloud.com/artist/song?secret_token=s-AbC"
    );
    assert!(ResolveInput::parse("http://on.soundcloud.com/AbCd")?.short_link);
    assert_eq!(
        ResolveInput::parse("http://on.soundcloud.com/AbCd?si=1")?.upstream,
        "https://on.soundcloud.com/AbCd"
    );
    assert!(ResolveInput::parse("https://snd.sc/abc").is_err());
    let expanded = ResolveInput::expanded(
        "https://soundcloud.com/lagolago/kuskus-at-lago-lago-2022?si=abc&utm_source=tumblr",
    )?;
    assert_eq!(
        expanded.upstream,
        "https://soundcloud.com/lagolago/kuskus-at-lago-lago-2022"
    );
    assert!(!expanded.short_link && !expanded.requires_upstream);
    for location in [
        "https://on.soundcloud.com/AbCd",
        "https://evil.example/lagolago/kuskus",
        "soundcloud:tracks:1",
        "/relative",
    ] {
        assert!(ResolveInput::expanded(location).is_err(), "{location}");
    }
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn local_resolve_matches_permalinks_regardless_of_case(pool: PgPool) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO users (sc_user_id, urn, username, username_normalized, permalink_url)
        VALUES ('17', 'soundcloud:users:17', 'Owner', 'owner', 'https://soundcloud.com/DJ-Artist')",
    )
    .execute(&pool)
    .await?;
    sqlx::query("INSERT INTO tracks (sc_track_id, urn, title, title_normalized, duration_ms, uploader_sc_user_id, permalink_url)
        VALUES ('42', 'soundcloud:tracks:42', 'Song', 'song', 120000, '17', 'https://soundcloud.com/DJ-Artist/Song-Name')")
        .execute(&pool).await?;
    sqlx::query("INSERT INTO playlists (sc_playlist_id, urn, title, title_normalized, owner_sc_user_id, permalink_url)
        VALUES ('43', 'soundcloud:playlists:43', 'Mix', 'mix', '17', 'https://soundcloud.com/dj-artist/sets/s-mix/')")
        .execute(&pool).await?;
    for (url, urn) in [
        (
            "https://soundcloud.com/dj-artist/song-name?in=dj-artist/sets/s-mix&t=42",
            "soundcloud:tracks:42",
        ),
        (
            "https://M.SoundCloud.com/DJ-ARTIST/SONG-NAME/",
            "soundcloud:tracks:42",
        ),
        (
            "https://soundcloud.com/DJ-Artist/sets/S-Mix",
            "soundcloud:playlists:43",
        ),
        ("http://www.soundcloud.com/dj-artist", "soundcloud:users:17"),
    ] {
        let input = ResolveInput::parse(url)?;
        assert!(!input.requires_upstream, "{url}");
        let key = repository::find(&pool, &input)
            .await?
            .expect("local permalink");
        assert_eq!(key.urn(), urn, "{url}");
    }
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
