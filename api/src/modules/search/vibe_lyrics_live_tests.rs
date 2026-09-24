use std::collections::HashSet;
use std::sync::Arc;

use backend_contracts::vector_store::{QUERY_VEC_LYRICS, query_point_uuid};
use qdrant_client::qdrant::{
    CreateCollectionBuilder, Distance, PointStruct, UpsertPointsBuilder, VectorParamsBuilder,
};
use sha2::{Digest, Sha256};
use sqlx::PgPool;

use crate::cache::CacheService;
use crate::modules::lyrics::WorkerClient;
use crate::modules::recommendations::RecommendationsService;
use crate::modules::recommendations::live_fixture::{
    TrackSeed, catalogue, install_all_vectors, install_catalog, install_vectors, lyrics_request_id,
    pointing_in, service,
};

const LYRICS_DIMENSIONS: u64 = 1024;
use crate::qdrant::collections;

use super::lyrics::{LyricsMode, LyricsSearchResponse};
use super::semantic::VibeSearchService;

const LIMIT: i64 = 5;

async fn vibe(pg: PgPool) -> anyhow::Result<(Arc<VibeSearchService>, Arc<RecommendationsService>)> {
    let recommendations = service(pg.clone()).await?;
    let redis = deadpool_redis::Config::from_url(
        std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".to_owned()),
    )
    .create_pool(Some(deadpool_redis::Runtime::Tokio1))?;
    let cache = CacheService::new(redis);
    let qdrant = recommendations.qdrant.clone();
    let worker = WorkerClient::new(recommendations.nats.clone(), cache.clone(), qdrant.clone());
    let vibe = VibeSearchService::new(pg, cache, recommendations.clone(), worker, qdrant);
    Ok((vibe, recommendations))
}

async fn install_query_vector(
    service: &RecommendationsService,
    query: &str,
    vector: Vec<f32>,
) -> anyhow::Result<()> {
    let hash = hex::encode(Sha256::digest(query.trim().as_bytes()));
    let client = service.qdrant.raw();
    let _ = client.delete_collection(QUERY_VEC_LYRICS).await;
    client
        .create_collection(
            CreateCollectionBuilder::new(QUERY_VEC_LYRICS).vectors_config(
                VectorParamsBuilder::new(LYRICS_DIMENSIONS, Distance::Cosine),
            ),
        )
        .await?;
    client
        .upsert_points(
            UpsertPointsBuilder::new(
                QUERY_VEC_LYRICS,
                vec![PointStruct::new(
                    query_point_uuid(&hash),
                    vector,
                    qdrant_client::Payload::from(serde_json::Map::from_iter([(
                        crate::qdrant::QUERY_VECTOR_ENCODER_FIELD.to_owned(),
                        serde_json::json!("Qwen/Qwen3-Embedding-0.6B@97b0c614"),
                    )])),
                )],
            )
            .wait(true),
        )
        .await?;
    forget_cached_encoding(service, "vibe:vec:lyrics:v2:", &hash).await
}

async fn forget_cached_encoding(
    service: &RecommendationsService,
    prefix: &str,
    hash: &str,
) -> anyhow::Result<()> {
    use deadpool_redis::redis::AsyncCommands;
    let mut connection = service.redis.get().await?;
    let _: () = connection.del(format!("{prefix}{hash}")).await?;
    Ok(())
}

async fn install_everything(
    recommendations: &RecommendationsService,
    tracks: &[TrackSeed],
    query: &str,
) -> anyhow::Result<()> {
    install_all_vectors(recommendations, tracks).await?;
    install_vectors(recommendations, collections::TRACKS_LYRICS, tracks).await?;
    install_query_vector(recommendations, query, pointing_in(0, LYRICS_DIMENSIONS)).await
}

async fn install_lyrics(pg: &PgPool, tracks: &[TrackSeed]) -> anyhow::Result<()> {
    for track in tracks {
        write_lyrics(
            pg,
            &track.sc_track_id.to_string(),
            &format!("the sea at night, verse {}", track.sc_track_id),
        )
        .await?;
    }
    Ok(())
}

async fn write_lyrics(pg: &PgPool, sc_track_id: &str, text: &str) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO lyrics_cache (sc_track_id, plain_text, source, embedded_at,
                                   embedding_state, created_at)
         VALUES ($1, $2, 'lrclib', now(), 'done', now())
         ON CONFLICT (sc_track_id) DO UPDATE SET plain_text = EXCLUDED.plain_text",
    )
    .bind(sc_track_id)
    .bind(text)
    .execute(pg)
    .await?;
    sqlx::query(
        "INSERT INTO lyrics_embedding_wire_state (sc_track_id, status, lyrics_created_at,
                                                  request_message_id, completed_at)
         SELECT sc_track_id, 'done', created_at, $2, now()
         FROM lyrics_cache
         WHERE sc_track_id = $1
         ON CONFLICT (sc_track_id) DO NOTHING",
    )
    .bind(sc_track_id)
    .bind(lyrics_request_id(sc_track_id))
    .execute(pg)
    .await?;
    Ok(())
}

fn found(page: &LyricsSearchResponse) -> Vec<String> {
    page.collection
        .iter()
        .filter_map(|hit| {
            hit.track
                .get("id")
                .and_then(|id| id.as_u64().map(|id| id.to_string()))
        })
        .collect()
}

fn fresh(base: &str) -> String {
    format!("{base} {}", std::process::id())
}

async fn search(
    vibe: &VibeSearchService,
    query: &str,
    page: i64,
) -> anyhow::Result<LyricsSearchResponse> {
    search_in(vibe, query, LyricsMode::Semantic, page).await
}

async fn search_in(
    vibe: &VibeSearchService,
    query: &str,
    mode: LyricsMode,
    page: i64,
) -> anyhow::Result<LyricsSearchResponse> {
    Ok(Box::pin(vibe.lyrics(query, mode, Some(page), Some(LIMIT))).await?)
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Qdrant, Redis and NATS"]
async fn a_query_we_already_have_a_vector_for_is_answered_without_the_worker(
    pg: PgPool,
) -> anyhow::Result<()> {
    let query = fresh("a song about the sea at night");
    let tracks = catalogue(700_000, 30);
    install_catalog(&pg, &tracks).await?;
    install_lyrics(&pg, &tracks).await?;

    let (vibe, recommendations) = vibe(pg).await?;
    install_everything(&recommendations, &tracks, &query).await?;

    let page = search(&vibe, &query, 0).await?;
    let ids = found(&page);

    assert_eq!(
        ids.len(),
        LIMIT as usize,
        "the vector for this query is already in Qdrant, so the answer must be full and \
         must not wait for the worker: {page:?}"
    );
    let known: HashSet<String> = tracks
        .iter()
        .map(|track| track.sc_track_id.to_string())
        .collect();
    assert!(
        ids.iter().all(|id| known.contains(id)),
        "the answer holds a track that is not in the catalogue: {ids:?}"
    );
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Qdrant, Redis and NATS"]
async fn a_track_made_private_stops_answering_searches_even_from_the_cache(
    pg: PgPool,
) -> anyhow::Result<()> {
    let query = fresh("a song about a private harbour");
    let tracks = catalogue(710_000, 30);
    install_catalog(&pg, &tracks).await?;
    install_lyrics(&pg, &tracks).await?;

    let (vibe, recommendations) = vibe(pg.clone()).await?;
    install_everything(&recommendations, &tracks, &query).await?;

    let before = found(&search(&vibe, &query, 0).await?);
    let doomed = before.first().cloned().expect("the search found something");

    sqlx::query("UPDATE tracks SET sharing = 'private' WHERE sc_track_id = $1")
        .bind(&doomed)
        .execute(&pg)
        .await?;

    let after = found(&search(&vibe, &query, 0).await?);

    assert!(
        !after.contains(&doomed),
        "{doomed} was made private and the same search still answers with it, cache and all: \
         {after:?}"
    );
    assert!(
        !after.is_empty(),
        "the whole answer disappeared, so this test would pass for the wrong reason"
    );
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Qdrant, Redis and NATS"]
async fn the_second_page_survives_a_filter_that_emptied_the_first_fetch(
    pg: PgPool,
) -> anyhow::Result<()> {
    let query = fresh("a song about the deep second page");
    let tracks = catalogue(720_000, 40);
    install_catalog(&pg, &tracks).await?;
    install_lyrics(&pg, &tracks).await?;
    let hidden: Vec<String> = (0..10).map(|index| (720_000 + index).to_string()).collect();
    sqlx::query("UPDATE tracks SET sharing = 'private' WHERE sc_track_id = ANY($1)")
        .bind(&hidden)
        .execute(&pg)
        .await?;

    let (vibe, recommendations) = vibe(pg).await?;
    install_everything(&recommendations, &tracks, &query).await?;

    let first = found(&search(&vibe, &query, 0).await?);
    let second = found(&search(&vibe, &query, 1).await?);

    assert_eq!(
        first.len(),
        LIMIT as usize,
        "thirty tracks are eligible, so the first page must be full: {first:?}"
    );
    assert_eq!(
        second.len(),
        LIMIT as usize,
        "twenty five tracks are still eligible after the first page, yet the second came back \
         with {}: {second:?}",
        second.len()
    );
    assert!(
        second.iter().all(|id| !first.contains(id)),
        "the second page repeats the first: {first:?} then {second:?}"
    );
    Ok(())
}

const RARE_WORD: &str = "lighthouse";

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Qdrant, Redis and NATS"]
async fn the_mixed_mode_puts_the_words_you_typed_before_what_merely_sounds_alike(
    pg: PgPool,
) -> anyhow::Result<()> {
    let spoken = fresh(RARE_WORD);
    let tracks = catalogue(730_000, 30);
    install_catalog(&pg, &tracks).await?;
    install_lyrics(&pg, &tracks).await?;
    let spelled_out = ["730025", "730026"];
    for id in spelled_out {
        write_lyrics(&pg, id, &format!("a {spoken} on the far shore")).await?;
    }

    let (vibe, recommendations) = vibe(pg).await?;
    install_everything(&recommendations, &tracks, &spoken).await?;

    let text = found(&search_in(&vibe, &spoken, LyricsMode::Text, 0).await?);
    let mixed = found(&search_in(&vibe, &spoken, LyricsMode::Auto, 0).await?);

    assert_eq!(
        text.len(),
        spelled_out.len(),
        "only two songs carry the word, so the text mode must find exactly those: {text:?}"
    );
    assert_eq!(
        &mixed[..spelled_out.len()],
        &text[..],
        "the songs that actually say `{RARE_WORD}` must come before the ones that only sound \
         like the query: {mixed:?}"
    );
    assert!(
        mixed.len() > spelled_out.len(),
        "the mixed mode stopped at the text matches and never used the vector at all: {mixed:?}"
    );
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Qdrant, Redis and NATS"]
async fn a_song_found_by_both_halves_of_the_mixed_mode_is_shown_once(
    pg: PgPool,
) -> anyhow::Result<()> {
    let spoken = fresh(RARE_WORD);
    let tracks = catalogue(740_000, 30);
    install_catalog(&pg, &tracks).await?;
    install_lyrics(&pg, &tracks).await?;
    write_lyrics(&pg, "740000", &format!("a {spoken} on the far shore")).await?;

    let (vibe, recommendations) = vibe(pg).await?;
    install_everything(&recommendations, &tracks, &spoken).await?;

    let semantic = found(&search_in(&vibe, &spoken, LyricsMode::Semantic, 0).await?);
    let mixed = found(&search_in(&vibe, &spoken, LyricsMode::Auto, 0).await?);

    assert!(
        semantic.contains(&"740000".to_owned()),
        "the fixture was meant to make one song reachable both ways, and the vector half does \
         not reach it: {semantic:?}"
    );
    let mut seen = HashSet::new();
    for id in &mixed {
        assert!(
            seen.insert(id.clone()),
            "{id} is found by the words and by the vector, and the mixed mode shows it twice: \
             {mixed:?}"
        );
    }
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Qdrant, Redis and NATS"]
async fn the_loser_of_a_merge_stops_answering_the_words_that_found_it(
    pg: PgPool,
) -> anyhow::Result<()> {
    let spoken = fresh(RARE_WORD);
    let tracks = catalogue(770_000, 30);
    install_catalog(&pg, &tracks).await?;
    install_lyrics(&pg, &tracks).await?;
    let singers: Vec<String> = (0..=LIMIT as u64)
        .map(|index| (770_000 + index).to_string())
        .collect();
    for id in &singers {
        write_lyrics(&pg, id, &format!("a {spoken} on the far shore")).await?;
    }
    let loser = singers[1].clone();

    let (vibe, recommendations) = vibe(pg.clone()).await?;
    install_everything(&recommendations, &tracks, &spoken).await?;

    let before = found(&search_in(&vibe, &spoken, LyricsMode::Text, 0).await?);
    assert_eq!(
        before.len(),
        LIMIT as usize,
        "the fixture must start with a full page, or a page that shrinks proves nothing: \
         {before:?}"
    );
    assert!(
        before.contains(&loser),
        "the track that will lose the merge must start out reachable: {before:?}"
    );

    sqlx::query(
        "UPDATE tracks SET superseded_by = (SELECT id FROM tracks WHERE sc_track_id = $1)
         WHERE sc_track_id = $2",
    )
    .bind(&singers[0])
    .bind(&loser)
    .execute(&pg)
    .await?;

    let after = found(&search_in(&vibe, &spoken, LyricsMode::Text, 0).await?);
    assert!(
        !after.contains(&loser),
        "the track that lost the merge is hidden everywhere else and must not come back through \
         the lyrics: {after:?}"
    );
    assert_eq!(
        after.len(),
        LIMIT as usize,
        "the merged-away track must be refused by the query, not thrown away after it: dropping \
         it later leaves a hole in the page and the listener never learns there was more: \
         {after:?}"
    );
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Qdrant, Redis and NATS"]
async fn the_mixed_mode_answers_from_the_catalogue_and_not_from_a_stale_cache(
    pg: PgPool,
) -> anyhow::Result<()> {
    let query = fresh("a song about a fading harbour light");
    let tracks = catalogue(750_000, 30);
    install_catalog(&pg, &tracks).await?;
    install_lyrics(&pg, &tracks).await?;

    let (vibe, recommendations) = vibe(pg.clone()).await?;
    install_everything(&recommendations, &tracks, &query).await?;

    let before = found(&search_in(&vibe, &query, LyricsMode::Auto, 0).await?);
    let doomed = before
        .first()
        .cloned()
        .expect("the mixed mode found something");
    sqlx::query("UPDATE tracks SET sharing = 'private' WHERE sc_track_id = $1")
        .bind(&doomed)
        .execute(&pg)
        .await?;
    let after = found(&search_in(&vibe, &query, LyricsMode::Auto, 0).await?);

    assert!(
        !after.contains(&doomed),
        "{doomed} was made private and the mixed mode still offers it: {after:?}"
    );
    assert!(
        !after.is_empty(),
        "the whole answer disappeared, so this test would pass for the wrong reason"
    );
    Ok(())
}

fn matched_lines(page: &LyricsSearchResponse) -> Vec<Option<String>> {
    page.collection
        .iter()
        .map(|hit| hit.matched_line.clone())
        .collect()
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Qdrant, Redis and NATS"]
async fn the_order_the_search_chose_survives_the_projection(pg: PgPool) -> anyhow::Result<()> {
    let tracks = catalogue(760_000, 12);
    install_catalog(&pg, &tracks).await?;
    install_lyrics(&pg, &tracks).await?;
    write_lyrics(
        &pg,
        "760007",
        &format!("{RARE_WORD} {RARE_WORD} {RARE_WORD} on every shore"),
    )
    .await?;
    for id in ["760001", "760003", "760005"] {
        write_lyrics(&pg, id, &format!("one {RARE_WORD} in the distance")).await?;
    }

    let (vibe, recommendations) = vibe(pg).await?;
    install_everything(&recommendations, &tracks, RARE_WORD).await?;

    let page = search_in(&vibe, RARE_WORD, LyricsMode::Text, 0).await?;
    let ids = found(&page);

    assert_eq!(
        ids.len(),
        4,
        "four songs carry the word, so four must come back: {ids:?}"
    );
    assert_eq!(
        ids.first().map(String::as_str),
        Some("760007"),
        "the song that says it three times ranks above the ones that say it once, and the \
         projection must not shuffle that away: {ids:?}"
    );
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Qdrant, Redis and NATS"]
async fn a_text_hit_arrives_with_the_line_that_matched(pg: PgPool) -> anyhow::Result<()> {
    let tracks = catalogue(770_000, 12);
    install_catalog(&pg, &tracks).await?;
    install_lyrics(&pg, &tracks).await?;
    write_lyrics(&pg, "770004", &format!("a {RARE_WORD} on the far shore")).await?;

    let (vibe, recommendations) = vibe(pg).await?;
    install_everything(&recommendations, &tracks, RARE_WORD).await?;

    let page = search_in(&vibe, RARE_WORD, LyricsMode::Text, 0).await?;
    let lines = matched_lines(&page);

    assert_eq!(found(&page), vec!["770004".to_owned()]);
    let line = lines
        .first()
        .cloned()
        .flatten()
        .expect("a text hit must show which line matched");
    assert!(
        line.contains(RARE_WORD),
        "the line handed to the listener does not contain what they searched for: {line:?}"
    );
    assert!(
        !line.contains("<<") && !line.contains(">>"),
        "the highlight markers leaked into the answer: {line:?}"
    );
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Qdrant, Redis and NATS"]
async fn lyrics_left_behind_by_a_deleted_track_answer_nothing(pg: PgPool) -> anyhow::Result<()> {
    let tracks = catalogue(780_000, 12);
    install_catalog(&pg, &tracks).await?;
    install_lyrics(&pg, &tracks).await?;
    write_lyrics(&pg, "780002", &format!("a {RARE_WORD} on the far shore")).await?;
    write_lyrics(&pg, "999999999", &format!("a {RARE_WORD} nobody owns")).await?;

    let (vibe, recommendations) = vibe(pg).await?;
    install_everything(&recommendations, &tracks, RARE_WORD).await?;

    let ids = found(&search_in(&vibe, RARE_WORD, LyricsMode::Text, 0).await?);

    assert_eq!(
        ids,
        vec!["780002".to_owned()],
        "a lyrics row whose track is not in the catalogue must answer nothing at all: {ids:?}"
    );
    Ok(())
}
