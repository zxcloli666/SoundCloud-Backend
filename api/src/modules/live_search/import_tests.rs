use std::time::Duration;

use serde_json::{Value, json};
use sqlx::PgPool;

use super::match_search;
use super::matching::Wanted;
use super::meta::{LivePage, LiveState};
use super::query::{LiveClass, LiveKind};
use super::service_tests::{
    Harness, Script, base, harness_with, hits, redis, rows, sources, titles,
};
use super::stash_tests::indexing;
use crate::cache::CacheService;
use crate::config::{LiveMode, LiveSearchCfg};
use crate::modules::search::SearchService;
use crate::modules::tracks::TrackPriority;

fn ranked() -> LiveSearchCfg {
    LiveSearchCfg {
        mode: LiveMode::Explicit,
        db_rescue: false,
        max_in_flight: 8,
        ranked: true,
    }
}

fn sc_track(id: u64, title: &str, username: &str) -> Value {
    json!({
        "id": id,
        "kind": "track",
        "urn": format!("soundcloud:tracks:{id}"),
        "title": title,
        "policy": "ALLOW",
        "duration": 240000,
        "user": {"id": 77, "kind": "user", "urn": "soundcloud:users:77", "username": username}
    })
}

async fn import(lab: &Harness, words: &str, local: Vec<Value>) -> LivePage {
    let request = lab.request(LiveKind::Tracks, &lab.phrase(words), None, None, 3, true);
    assert_eq!(request.class, LiveClass::Import);
    assert_eq!(
        request.local_limit, 20,
        "the ranked import weighs the local top 20"
    );
    lab.ask(&request, local, Duration::ZERO).await
}

async fn kept(pool: &PgPool, id: u64) -> anyhow::Result<Option<(i16, i64)>> {
    Ok(
        sqlx::query_as("SELECT index_priority, sc_observation FROM tracks WHERE sc_track_id = $1")
            .bind(id.to_string())
            .fetch_optional(pool)
            .await?,
    )
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Redis"]
async fn a_confident_import_answers_one_soundcloud_track_and_keeps_it(
    pool: PgPool,
) -> anyhow::Result<()> {
    let at = base();
    let lab = harness_with(&pool, Script::Silent, ranked())?;
    lab.live.install_indexing(indexing(&pool).await?);
    let title = format!("Lucid Dreams {}", lab.tag);
    *lab.relay.script.lock().unwrap() = Script::Answers(vec![
        sc_track(at, &format!("Juice WRLD - {title} (Lyrics)"), "vault"),
        sc_track(at + 1, &title, "Juice WRLD"),
    ]);

    let page = import(&lab, "Juice WRLD Lucid Dreams", rows(at + 500, 3, "local")).await;

    assert_eq!(
        titles(&page),
        [title],
        "the official upload, not the top hit"
    );
    assert_eq!(sources(&page), ["soundcloud"]);
    assert!(
        page.page.collection[0]["_scd_search"]["score"]
            .as_f64()
            .is_some_and(|score| score >= 0.8)
    );
    assert!(!page.page.has_more);
    assert_eq!(lab.relay.calls(), 1);
    assert_eq!(
        kept(&pool, at + 1).await?,
        Some((TrackPriority::Playlist.as_i16(), 0)),
        "a live pick lands in the catalog as an unverified playlist track"
    );
    assert_eq!(kept(&pool, at).await?, None, "only the pick is kept");
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Redis"]
async fn an_unsure_import_answers_nothing_and_keeps_nothing(pool: PgPool) -> anyhow::Result<()> {
    let at = base();
    let lab = harness_with(&pool, Script::Answers(hits(at, 5)), ranked())?;
    lab.live.install_indexing(indexing(&pool).await?);

    let page = import(
        &lab,
        "Someone Else Another Song",
        rows(at + 600, 3, "local"),
    )
    .await;

    assert!(
        page.page.collection.is_empty(),
        "no pick is better than a wrong track in the playlist: {:?}",
        titles(&page)
    );
    assert_eq!(page.state(), LiveState::Fresh);
    assert_eq!(lab.relay.calls(), 1);
    assert_eq!(kept(&pool, at).await?, None);
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Redis"]
async fn a_confident_local_import_never_asks_soundcloud(pool: PgPool) -> anyhow::Result<()> {
    let at = base();
    let lab = harness_with(&pool, Script::Answers(hits(at, 5)), ranked())?;
    let known = format!("Robbery {}", lab.tag);
    let local = vec![json!({
        "urn": format!("soundcloud:tracks:{}", at + 700),
        "title": known,
        "user": {"username": "Juice WRLD"}
    })];

    let page = import(&lab, "Juice WRLD Robbery", local).await;

    assert_eq!(titles(&page), [known]);
    assert_eq!(sources(&page), ["local"]);
    assert_eq!(page.state(), LiveState::Local);
    assert_eq!(
        lab.relay.calls(),
        0,
        "a confident local answer never asks SoundCloud"
    );
    Ok(())
}

async fn seed_artist_track(pool: &PgPool, artist: &str, title: &str) -> anyhow::Result<()> {
    let artist_id: uuid::Uuid = sqlx::query_scalar(
        "INSERT INTO artists (name, normalized_name, source, track_count_primary)
         VALUES ($1, $2, 'test', 1) RETURNING id",
    )
    .bind(artist)
    .bind(catalog_normalize::normalize_name(artist))
    .fetch_one(pool)
    .await?;
    sqlx::query(
        "INSERT INTO tracks (sc_track_id, urn, title, title_normalized, duration_ms, sharing,
                             uploader_username, primary_artist_id, play_count_sc)
         VALUES ('31', 'soundcloud:tracks:31', $1, $2, 239000, 'public', $3, $4, 1000)",
    )
    .bind(title)
    .bind(catalog_normalize::normalize_title(title))
    .bind(artist)
    .bind(artist_id)
    .execute(pool)
    .await?;
    Ok(())
}

fn ranked_search(pool: &PgPool) -> std::sync::Arc<SearchService> {
    SearchService::new(pool.clone(), CacheService::new(redis()), true)
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Redis"]
async fn a_match_comes_from_the_catalog_first_and_then_from_soundcloud(
    pool: PgPool,
) -> anyhow::Result<()> {
    let at = base();
    let lab = harness_with(&pool, Script::Silent, ranked())?;
    lab.live.install_indexing(indexing(&pool).await?);
    let artist = format!("Artist {}", lab.tag);
    seed_artist_track(&pool, &artist, "Lucid Dreams").await?;
    let search = ranked_search(&pool);

    let wanted = Wanted::parse(Some(&artist), Some("Lucid Dreams"), Some("239000"))?;
    let local = match_search::find(&search, &lab.live, &pool, &wanted, "listener").await?;
    let found = local.found.expect("the catalog knows this track");
    assert_eq!(found.urn, "soundcloud:tracks:31");
    assert_eq!(found.source, "local");
    assert_eq!(
        lab.relay.calls(),
        0,
        "a catalog match never asks SoundCloud"
    );

    *lab.relay.script.lock().unwrap() = Script::Answers(vec![
        sc_track(at, "Robbery (Sped Up)", &artist),
        sc_track(at + 1, "Robbery", &artist),
    ]);
    let wanted = Wanted::parse(Some(&artist), Some("Robbery"), Some("240000"))?;
    let live = match_search::find(&search, &lab.live, &pool, &wanted, "listener").await?;
    let found = live.found.expect("SoundCloud has it");
    assert_eq!(found.urn, format!("soundcloud:tracks:{}", at + 1));
    assert_eq!(found.source, "soundcloud");
    assert_eq!(live.live.state, LiveState::Fresh);
    assert_eq!(lab.relay.calls(), 1);
    assert_eq!(
        kept(&pool, at + 1).await?,
        Some((TrackPriority::Playlist.as_i16(), 0)),
        "a confident live match is kept for the playlist it goes into"
    );
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Redis"]
async fn a_match_that_cannot_reach_soundcloud_now_asks_the_importer_to_wait(
    pool: PgPool,
) -> anyhow::Result<()> {
    let lab = harness_with(
        &pool,
        Script::Answers(hits(base(), 3)),
        LiveSearchCfg {
            max_in_flight: 0,
            ..ranked()
        },
    )?;
    let wanted = Wanted::parse(Some(&lab.phrase("nobody")), Some("Nothing"), None)?;
    let paced = match_search::find(&ranked_search(&pool), &lab.live, &pool, &wanted, "listener")
        .await
        .expect_err("a full house answers 429");
    assert_eq!(paced.public_code(), "search_paced");
    let response = axum::response::IntoResponse::into_response(paced);
    assert_eq!(response.status(), axum::http::StatusCode::TOO_MANY_REQUESTS);
    assert!(response.headers().contains_key("retry-after"));
    assert_eq!(lab.relay.calls(), 0);
    Ok(())
}
