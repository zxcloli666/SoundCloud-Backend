use std::sync::Arc;

use anyhow::Context;
use sqlx::PgPool;
use uuid::Uuid;

use super::live_fixture::{
    LIKES, LISTENER, SPREAD, TrackSeed, install_all_vectors, install_catalog, likes_the_same_ten,
    service,
};
use super::service::RecommendationsService;
use super::smart_wave::graph::forget_user_affinity;
use super::smart_wave::{self, SmartWaveRequest, SmartWaveSeed};

const CROWD: [&str; 12] = [
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
];
const FIRST: u64 = 961_000;
const PAGE: usize = 8;

fn sound_of(id: u64) -> u64 {
    (id - FIRST) % SPREAD as u64
}

fn of_sound(ids: &[u64], sound: u64) -> usize {
    ids.iter().filter(|id| sound_of(**id) == sound).count()
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

async fn listener_with_history(pg: &PgPool) -> anyhow::Result<Arc<RecommendationsService>> {
    let tracks: Vec<TrackSeed> = (0..120)
        .map(|index| TrackSeed {
            sc_track_id: FIRST + index,
            artist: CROWD[index as usize % CROWD.len()],
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
    let service = listener_with_history(&pg).await?;
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
    let service = listener_with_history(&pg).await?;
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
    let service = listener_with_history(&pg).await?;
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
    let service = listener_with_history(&pg).await?;
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
