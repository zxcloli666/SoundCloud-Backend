use std::sync::Arc;

use anyhow::Context;
use deadpool_redis::redis::AsyncCommands;
use sqlx::PgPool;
use uuid::Uuid;

use super::home_wave::HomeRequest;
use super::live_fixture::{
    LIKES, LISTENER, SPREAD, TrackSeed, dislike, install_all_vectors, install_catalog,
    likes_the_same_ten, service,
};
use super::service::RecommendationsService;
use super::smart_wave::graph::forget_user_affinity;
use super::smart_wave::{self, SmartWaveRequest, SmartWaveSeed};

const CROWD: [&str; 24] = [
    "Aster",
    "Bellwether",
    "Cinder",
    "Dovetail",
    "Ember",
    "Fallow",
    "Gossamer",
    "Hollow",
    "Ironwood",
    "Juniper",
    "Kestrel",
    "Lantern",
    "Meridian",
    "Nettle",
    "Orchard",
    "Pewter",
    "Quarry",
    "Rookery",
    "Saltmarsh",
    "Thistle",
    "Umber",
    "Vesper",
    "Willow",
    "Yarrow",
];
const SMALL_CROWD: usize = 12;
const FIRST: u64 = 961_000;
const PAGE: usize = 8;

fn sound_of(id: u64) -> u64 {
    (id - FIRST) % SPREAD as u64
}

fn of_sound(ids: &[u64], sound: u64) -> usize {
    ids.iter().filter(|id| sound_of(**id) == sound).count()
}

fn artist_of(id: u64, crowd: usize) -> &'static str {
    CROWD[((id - FIRST) % crowd as u64) as usize]
}

async fn listened(
    pg: &PgPool,
    sc_track_id: u64,
    event_type: &str,
    weight: f64,
    position: f32,
    hours_ago: i32,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO user_events
             (id, sc_user_id, sc_track_id, event_type, weight, position_pct, created_at)
         VALUES (gen_random_uuid(), $1, $2, $3, $4, $5, now() - make_interval(hours => $6))",
    )
    .bind(LISTENER)
    .bind(sc_track_id.to_string())
    .bind(event_type)
    .bind(weight)
    .bind(position)
    .bind(hours_ago)
    .execute(pg)
    .await?;
    Ok(())
}

async fn listener_with_history(
    pg: &PgPool,
    crowd: usize,
) -> anyhow::Result<Arc<RecommendationsService>> {
    let tracks: Vec<TrackSeed> = (0..120)
        .map(|index| TrackSeed {
            sc_track_id: FIRST + index,
            artist: artist_of(FIRST + index, crowd),
            plays: 5_000,
        })
        .collect();
    install_catalog(pg, &tracks).await?;
    likes_the_same_ten(pg, LISTENER, FIRST).await?;
    for id in FIRST..FIRST + LIKES {
        listened(pg, id, "full_play", 0.3, 1.0, 48).await?;
    }
    let service = service(pg.clone()).await?;
    install_all_vectors(&service, &tracks).await?;
    forget_user_affinity(&service.redis, LISTENER).await;
    Ok(service)
}

async fn page(
    service: &RecommendationsService,
    seed: SmartWaveSeed<'_>,
    cursor: Option<&str>,
) -> anyhow::Result<(Vec<u64>, String)> {
    let response = smart_wave::build(
        service,
        SmartWaveRequest {
            sc_user_id: LISTENER,
            languages: None,
            limit: PAGE,
            cursor_token: cursor,
            seed,
            hide_listened: true,
        },
    )
    .await?;
    let ids = response
        .tracks
        .iter()
        .filter_map(|track| track.id.as_u64())
        .collect();
    Ok((ids, response.cursor))
}

struct Pages {
    first: Vec<u64>,
    mine: String,
    unchanged: Vec<u64>,
}

async fn twin_pages<'a>(
    service: &RecommendationsService,
    seed: impl Fn() -> SmartWaveSeed<'a>,
) -> anyhow::Result<Pages> {
    let (first, mine) = page(service, seed(), None).await?;
    let (twin, control) = page(service, seed(), None).await?;
    assert_eq!(
        first, twin,
        "two fresh waves must start alike, or the control page below compares nothing"
    );
    let (unchanged, _) = page(service, seed(), Some(&control)).await?;
    assert_eq!(unchanged.len(), PAGE, "the control page came back short");
    Ok(Pages {
        first,
        mine,
        unchanged,
    })
}

async fn send_feedback(
    service: &RecommendationsService,
    cursor: &str,
    negatives: usize,
    positives: usize,
) -> anyhow::Result<String> {
    smart_wave::record_feedback(&service.redis, LISTENER, cursor, negatives, positives)
        .await
        .context("feedback lost the wave cursor")
}

fn least_heard_on(first: &[u64], unchanged: &[u64]) -> anyhow::Result<u64> {
    first
        .iter()
        .copied()
        .filter(|id| *id >= FIRST + LIKES)
        .min_by_key(|id| of_sound(unchanged, sound_of(*id)))
        .context("the first page holds only tracks the listener already liked")
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Qdrant and Redis"]
async fn skipping_a_sound_takes_it_off_the_next_river_page(pg: PgPool) -> anyhow::Result<()> {
    let service = listener_with_history(&pg, SMALL_CROWD).await?;
    let pages = twin_pages(&service, || SmartWaveSeed::User).await?;
    let coming = sound_of(pages.unchanged[0]);
    let skipped: Vec<u64> = pages
        .first
        .iter()
        .copied()
        .filter(|id| sound_of(*id) == coming)
        .collect();
    assert!(
        !skipped.is_empty(),
        "the first page has nothing of the sound the river plays next: {:?}",
        pages.first
    );

    for id in &skipped {
        listened(&pg, *id, "skip", -0.8, 0.1, 0).await?;
    }
    let mine = send_feedback(&service, &pages.mine, skipped.len(), 0).await?;
    let (next, _) = page(&service, SmartWaveSeed::User, Some(&mine)).await?;

    assert!(!next.is_empty(), "the river ran dry after a skip");
    assert_eq!(
        of_sound(&next, coming),
        0,
        "skipping {skipped:?} left their sound on the next page: {next:?}, without feedback {:?}",
        pages.unchanged
    );
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Qdrant and Redis"]
async fn a_full_listen_pulls_the_next_river_page_toward_its_sound(
    pg: PgPool,
) -> anyhow::Result<()> {
    let service = listener_with_history(&pg, SMALL_CROWD).await?;
    let pages = twin_pages(&service, || SmartWaveSeed::User).await?;
    let kept = least_heard_on(&pages.first, &pages.unchanged)?;

    listened(&pg, kept, "full_play", 0.3, 1.0, 0).await?;
    let mine = send_feedback(&service, &pages.mine, 0, 1).await?;
    let (next, _) = page(&service, SmartWaveSeed::User, Some(&mine)).await?;

    let sound = sound_of(kept);
    assert!(
        of_sound(&next, sound) >= PAGE / 2
            && of_sound(&next, sound) > of_sound(&pages.unchanged, sound),
        "a full listen of {kept} did not steer the next page: {next:?}, without feedback {:?}",
        pages.unchanged
    );
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Qdrant and Redis"]
async fn a_full_listen_pulls_an_artist_wave_toward_its_sound(pg: PgPool) -> anyhow::Result<()> {
    let service = listener_with_history(&pg, SMALL_CROWD).await?;
    let artist: Uuid = sqlx::query_scalar("SELECT id FROM artists WHERE name = $1")
        .bind(CROWD[4])
        .fetch_one(&pg)
        .await?;
    let top = service.load_artist_top_track_ids(artist, 20).await?;
    let pages = twin_pages(&service, || SmartWaveSeed::Artist(artist, &top)).await?;
    let kept = least_heard_on(&pages.first, &pages.unchanged)?;

    listened(&pg, kept, "full_play", 0.3, 1.0, 0).await?;
    let mine = send_feedback(&service, &pages.mine, 0, 1).await?;
    let (next, _) = page(&service, SmartWaveSeed::Artist(artist, &top), Some(&mine)).await?;

    let sound = sound_of(kept);
    assert!(
        of_sound(&next, sound) >= PAGE / 2
            && of_sound(&next, sound) > of_sound(&pages.unchanged, sound),
        "a full listen of {kept} did not steer the artist wave: {next:?}, without feedback {:?}",
        pages.unchanged
    );
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Qdrant and Redis"]
async fn feedback_keeps_a_track_wave_going_where_it_was(pg: PgPool) -> anyhow::Result<()> {
    let service = listener_with_history(&pg, SMALL_CROWD).await?;
    let seed = FIRST + 3;
    let (first, cursor) = page(&service, SmartWaveSeed::Track(seed), None).await?;

    let kept = send_feedback(&service, &cursor, 1, 0).await?;
    let (next, _) = page(&service, SmartWaveSeed::Track(seed), Some(&kept)).await?;

    assert_eq!(kept, cursor, "feedback swapped the track wave cursor");
    assert!(!next.is_empty());
    assert!(
        next.iter().all(|id| !first.contains(id)),
        "the track wave started over after feedback: {first:?} then {next:?}"
    );
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Qdrant and Redis"]
async fn a_run_of_skips_sinks_their_artists_on_the_next_track_wave_page(
    pg: PgPool,
) -> anyhow::Result<()> {
    let service = listener_with_history(&pg, SMALL_CROWD).await?;
    let seed = FIRST + 3;
    let pages = twin_pages(&service, || SmartWaveSeed::Track(seed)).await?;
    let skipped_artists: Vec<&str> = pages
        .first
        .iter()
        .map(|id| artist_of(*id, SMALL_CROWD))
        .collect();
    let by_skipped = |ids: &[u64]| {
        ids.iter()
            .filter(|id| skipped_artists.contains(&artist_of(**id, SMALL_CROWD)))
            .count()
    };
    assert!(
        by_skipped(&pages.unchanged) > 0,
        "no artist of the first page comes back on the control page: {:?} then {:?}",
        pages.first,
        pages.unchanged
    );

    for id in &pages.first {
        listened(&pg, *id, "skip", -0.8, 0.1, 0).await?;
    }
    let mine = send_feedback(&service, &pages.mine, pages.first.len(), 0).await?;
    let (next, _) = page(&service, SmartWaveSeed::Track(seed), Some(&mine)).await?;

    assert_eq!(
        next.len(),
        PAGE,
        "the track wave ran short after a run of skips"
    );
    assert!(
        by_skipped(&next) < by_skipped(&pages.unchanged),
        "skipping {:?} kept their artists on the next track wave page: {next:?}, without \
         feedback {:?}",
        pages.first,
        pages.unchanged
    );
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Qdrant and Redis"]
async fn a_dislike_takes_its_artist_off_the_next_river_page(pg: PgPool) -> anyhow::Result<()> {
    let service = listener_with_history(&pg, SMALL_CROWD).await?;
    let pages = twin_pages(&service, || SmartWaveSeed::User).await?;
    let disliked = pages
        .first
        .iter()
        .copied()
        .filter(|id| *id >= FIRST + LIKES)
        .find(|id| {
            let artist = artist_of(*id, SMALL_CROWD);
            pages
                .unchanged
                .iter()
                .any(|next| artist_of(*next, SMALL_CROWD) == artist)
        })
        .context("no artist of the first page comes back on the control page")?;

    dislike(&pg, LISTENER, &disliked.to_string()).await?;
    let mine = send_feedback(&service, &pages.mine, 1, 0).await?;
    let (next, _) = page(&service, SmartWaveSeed::User, Some(&mine)).await?;

    let artist = artist_of(disliked, SMALL_CROWD);
    assert!(!next.is_empty(), "the river ran dry after a dislike");
    assert!(
        next.iter().all(|id| artist_of(*id, SMALL_CROWD) != artist),
        "disliking {disliked} left {artist} on the next page: {next:?}, without feedback {:?}",
        pages.unchanged
    );
    Ok(())
}

async fn home_shelf(service: &RecommendationsService) -> anyhow::Result<(Vec<u64>, String)> {
    let home = Box::pin(service.home_wave(HomeRequest {
        sc_user_id: LISTENER.to_owned(),
        languages: None,
        per_cluster: 6,
        hide_listened: true,
    }))
    .await?;
    let shelf = home
        .clusters
        .into_iter()
        .find(|cluster| cluster.id == "wave")
        .context("the home page has no wave shelf")?;
    let cursor = shelf
        .cursor
        .context("the wave shelf came without a cursor")?;
    let ids = shelf
        .track_ids
        .iter()
        .filter_map(|id| id.parse().ok())
        .collect();
    Ok((ids, cursor))
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Qdrant and Redis"]
async fn the_river_goes_on_from_the_home_wave_shelf_and_learns_from_it(
    pg: PgPool,
) -> anyhow::Result<()> {
    let service = listener_with_history(&pg, CROWD.len()).await?;
    let (shelf, mine) = home_shelf(&service).await?;
    let (twin, control) = home_shelf(&service).await?;
    assert_eq!(shelf, twin, "two home loads must start alike");
    let ttl: i64 = service
        .redis
        .get()
        .await?
        .ttl(format!("smartwave:handle:{LISTENER}:{mine}"))
        .await?;
    assert!(
        ttl > 2 * 60 * 60,
        "the shelf cursor expires before the river gets through its queue: {ttl}s"
    );
    let (unchanged, _) = page(&service, SmartWaveSeed::User, Some(&control)).await?;
    let coming = sound_of(unchanged[0]);
    let skipped: Vec<u64> = shelf
        .iter()
        .copied()
        .filter(|id| sound_of(*id) == coming)
        .collect();
    assert!(
        !skipped.is_empty(),
        "the shelf has nothing of the sound the river plays next: {shelf:?}"
    );

    for id in &skipped {
        listened(&pg, *id, "skip", -0.8, 0.1, 0).await?;
    }
    let (next, _) = page(&service, SmartWaveSeed::User, Some(&mine)).await?;

    assert!(!next.is_empty(), "the river ran dry after the shelf");
    assert!(
        next.iter().all(|id| !shelf.contains(id)),
        "the river repeated the shelf: {next:?}"
    );
    assert_eq!(
        of_sound(&next, coming),
        0,
        "skips on the shelf {skipped:?} left their sound on the river: {next:?}, without them \
         {unchanged:?}"
    );
    Ok(())
}

async fn track_wave_after_dislikes(
    pg: &PgPool,
    pick: impl Fn(&[u64]) -> Vec<u64>,
) -> anyhow::Result<(Pages, Vec<u64>, Vec<u64>)> {
    let service = listener_with_history(pg, SMALL_CROWD).await?;
    let seed = FIRST + 3;
    let pages = twin_pages(&service, || SmartWaveSeed::Track(seed)).await?;
    let disliked = pick(&pages.first);
    for id in &disliked {
        dislike(pg, LISTENER, &id.to_string()).await?;
    }
    let mine = send_feedback(&service, &pages.mine, disliked.len(), 0).await?;
    let (next, _) = page(&service, SmartWaveSeed::Track(seed), Some(&mine)).await?;
    Ok((pages, disliked, next))
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Qdrant and Redis"]
async fn a_dislike_of_the_seed_artist_keeps_the_track_wave_full(pg: PgPool) -> anyhow::Result<()> {
    let seed_artist = artist_of(FIRST + 3, SMALL_CROWD);
    let (pages, disliked, next) = track_wave_after_dislikes(&pg, |first| {
        first
            .iter()
            .copied()
            .filter(|id| artist_of(*id, SMALL_CROWD) == seed_artist)
            .take(1)
            .collect()
    })
    .await?;

    assert_eq!(
        disliked.len(),
        1,
        "the first page has nothing by the seed artist: {:?}",
        pages.first
    );
    assert_eq!(
        next.len(),
        PAGE,
        "one dislike of {disliked:?} emptied the track wave: {next:?}, without it {:?}",
        pages.unchanged
    );
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Qdrant and Redis"]
async fn disliking_half_a_page_takes_its_artists_off_a_full_track_wave_page(
    pg: PgPool,
) -> anyhow::Result<()> {
    let seed_artist = artist_of(FIRST + 3, SMALL_CROWD);
    let (pages, disliked, next) =
        track_wave_after_dislikes(&pg, |first| first[..first.len() / 2].to_vec()).await?;
    let out: Vec<&str> = disliked
        .iter()
        .map(|id| artist_of(*id, SMALL_CROWD))
        .filter(|artist| *artist != seed_artist)
        .collect();
    let by_out = |ids: &[u64]| {
        ids.iter()
            .filter(|id| out.contains(&artist_of(**id, SMALL_CROWD)))
            .count()
    };
    assert!(
        by_out(&pages.unchanged) > 0,
        "no disliked artist comes back on the control page: {:?} then {:?}",
        pages.first,
        pages.unchanged
    );

    assert_eq!(
        next.len(),
        PAGE,
        "disliking {disliked:?} emptied the track wave: {next:?}, without it {:?}",
        pages.unchanged
    );
    assert_eq!(
        by_out(&next),
        0,
        "disliking {disliked:?} left their artists on the track wave: {next:?}"
    );
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Qdrant and Redis"]
async fn a_track_wave_opened_again_after_a_run_of_skips_still_fills(
    pg: PgPool,
) -> anyhow::Result<()> {
    let service = listener_with_history(&pg, SMALL_CROWD).await?;
    let seed = FIRST + 3;
    let (first, _) = page(&service, SmartWaveSeed::Track(seed), None).await?;

    for id in &first {
        listened(&pg, *id, "skip", -0.8, 0.1, 0).await?;
    }
    let (again, _) = page(&service, SmartWaveSeed::Track(seed), None).await?;

    assert_eq!(
        again.len(),
        PAGE,
        "a lost wave session turned its skips {first:?} into a dry track wave: {again:?}"
    );
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Qdrant and Redis"]
async fn skipping_a_sound_sinks_it_on_the_next_artist_wave_page(pg: PgPool) -> anyhow::Result<()> {
    let service = listener_with_history(&pg, SMALL_CROWD).await?;
    let artist: Uuid = sqlx::query_scalar("SELECT id FROM artists WHERE name = $1")
        .bind(CROWD[4])
        .fetch_one(&pg)
        .await?;
    let top = service.load_artist_top_track_ids(artist, 20).await?;
    let pages = twin_pages(&service, || SmartWaveSeed::Artist(artist, &top)).await?;
    let coming = sound_of(pages.unchanged[0]);
    let skipped: Vec<u64> = pages
        .first
        .iter()
        .copied()
        .filter(|id| sound_of(*id) == coming)
        .collect();
    assert!(
        !skipped.is_empty(),
        "the first page has nothing of the sound the artist wave plays next: {:?}",
        pages.first
    );

    for id in &skipped {
        listened(&pg, *id, "skip", -0.8, 0.1, 0).await?;
    }
    let mine = send_feedback(&service, &pages.mine, skipped.len(), 0).await?;
    let (next, _) = page(&service, SmartWaveSeed::Artist(artist, &top), Some(&mine)).await?;

    assert_eq!(next.len(), PAGE, "the artist wave ran short after skips");
    assert!(
        of_sound(&next, coming) < of_sound(&pages.unchanged, coming),
        "skipping {skipped:?} kept their sound on the artist wave: {next:?}, without feedback {:?}",
        pages.unchanged
    );
    Ok(())
}
