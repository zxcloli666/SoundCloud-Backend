use std::collections::{HashMap, HashSet};

use sqlx::PgPool;
use uuid::Uuid;

use super::artist_tracks::ArtistTopTracks;
use super::cursor::{SeedKind, WaveCursor};
use super::graph::{Affinity, GraphEdges, GraphSeed, affinity_from};
use super::rank::{Candidate, RankedTrack};
use super::reach::{self, Reach};
use super::{
    ARTIST_CAP_IN_WINDOW, CONTENT_FLOOR, GRAPH_ARTISTS, GRAPH_FLOOR, GRAPH_PER_ARTIST,
    GRAPH_TRACKS_TOTAL, top_affinity_artists,
};
use crate::modules::recommendations::service::util::user_id_variants;

const LISTENER: &str = "soundcloud:users:990001";
const LIKED: usize = 48;
const CLOSE: usize = 40;
const FAR: usize = 200;
const DEEP: usize = 3;
const DEEP_HEARD: usize = 62;
const LIMIT: usize = 24;
const ROUNDS: u64 = 400;

type Maker = fn(u64, Option<Uuid>) -> Candidate;

fn mix(seed: u64) -> u64 {
    let mut z = seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

fn unit(seed: u64) -> f32 {
    (mix(seed) >> 40) as f32 / (1_u64 << 24) as f32
}

fn listened(track: u64, artist: Option<Uuid>) -> Candidate {
    Candidate {
        sc_track_id: track,
        artist,
        content: 0.3 + 0.7 * unit(track),
        skip_damp: 1.0 - 0.1 * (mix(track ^ 0x5eed) % 4) as f32,
    }
}

fn rough(track: u64, artist: Option<Uuid>) -> Candidate {
    if track.is_multiple_of(13) {
        return Candidate {
            sc_track_id: track,
            artist,
            content: 1.0,
            skip_damp: 1.0,
        };
    }
    Candidate {
        sc_track_id: track,
        artist,
        content: 0.2 + 0.8 * unit(track ^ 0xc0de),
        skip_damp: 0.4 + 0.6 * unit(track ^ 0xda4b),
    }
}

fn scored(artist_of: HashMap<u64, Option<Uuid>>, make: Maker) -> Vec<Candidate> {
    artist_of
        .into_iter()
        .map(|(track, artist)| make(track, artist))
        .collect()
}

fn pick(reach: &Reach<'_>, cands: &[Candidate]) -> Vec<RankedTrack> {
    let mut cands = cands.to_vec();
    cands.sort_by_key(|cand| cand.sc_track_id);
    reach.pick(&cands)
}

fn flat(picked: &[RankedTrack]) -> Vec<(u64, u32, Option<Uuid>)> {
    picked
        .iter()
        .map(|track| (track.sc_track_id, track.score.to_bits(), track.artist))
        .collect()
}

fn mert_map(mert: &[(u64, Option<Uuid>)]) -> HashMap<u64, Option<Uuid>> {
    let mut artist_of = HashMap::new();
    for (track, artist) in mert {
        artist_of.entry(*track).or_insert(*artist);
    }
    artist_of
}

fn main_wave(
    reach: &Reach<'_>,
    ranked: &[Uuid],
    rows: &HashMap<Uuid, Vec<u64>>,
    mert: &HashMap<u64, Option<Uuid>>,
    make: Maker,
) -> Vec<RankedTrack> {
    let every = reach::in_rank_order(rows_of(rows, ranked), ranked, GRAPH_TRACKS_TOTAL);
    pick(reach, &scored(reach::attribution(&every, mert), make))
}

fn reached_wave(
    reach: &Reach<'_>,
    ranked: &[Uuid],
    rows: &HashMap<Uuid, Vec<u64>>,
    mert: &HashMap<u64, Option<Uuid>>,
    make: Maker,
) -> (Vec<RankedTrack>, usize) {
    let (head, rest) = reach.split(ranked);
    let mut read = reach::in_rank_order(rows_of(rows, head), ranked, GRAPH_TRACKS_TOTAL);
    let settled: HashSet<u64> = read.iter().map(|(track, _)| *track).collect();
    let mut cands = scored(reach::attribution(&read, mert), make);
    let tail = reach.tail(rest, &cands, &settled);
    let room = GRAPH_TRACKS_TOTAL.saturating_sub(read.len());
    read.extend(reach::in_rank_order(rows_of(rows, &tail), ranked, room));
    let fresh = reach::reclaim(&mut cands, &read, mert);
    cands.extend(scored(fresh, make));
    (pick(reach, &cands), head.len() + tail.len())
}

struct World {
    affinity: Affinity,
    rows: HashMap<Uuid, Vec<u64>>,
    mert: Vec<(u64, Option<Uuid>)>,
    cursor: WaveCursor,
    disliked: HashSet<Uuid>,
    picks: usize,
    floor: f32,
}

fn world(round: u64) -> World {
    let artists = 20 + mix(round) % 141;
    let skew = 1.0 + 3.0 * unit(round ^ 0xa11);
    let mut affinity = Affinity::new();
    let mut rows = HashMap::new();
    let mut mert = Vec::new();
    let mut cursor = WaveCursor::new(SeedKind::User, "reach".to_owned());
    for index in 0..artists {
        let artist = Uuid::from_u128(u128::from(mix(round * 1_000 + index)));
        let value = match mix(round ^ (index << 20)) % 10 {
            0 => 1.25,
            1 => 0.5,
            _ => unit(round * 1_000 + index).powf(skew),
        };
        affinity.insert(artist, value);
        let first = round * 1_000_000 + index * 100;
        let count = mix(first) % 13;
        if count > 0 && mix(first ^ 1).is_multiple_of(5) {
            mert.push((first + mix(first ^ 2) % count, Some(artist)));
        }
        if count > 1 && mix(first ^ 5).is_multiple_of(4) {
            mert.push((first + count - 1, None));
        }
        if mix(first ^ 3).is_multiple_of(11) {
            cursor.mark_served(first, Some(artist));
        }
        rows.insert(artist, (first..first + count).collect());
    }
    for extra in 0..mix(round ^ 0x3e7) % 60 {
        let track = round * 1_000_000 + 900_000 + extra;
        let artist = match extra % 3 {
            0 => None,
            _ => Some(Uuid::from_u128(u128::from(mix(track)) | 1)),
        };
        mert.push((track, artist));
    }
    let disliked = affinity
        .keys()
        .filter(|artist| artist.as_u128().is_multiple_of(17))
        .copied()
        .collect();
    World {
        affinity,
        rows,
        mert,
        cursor,
        disliked,
        picks: 8 + (mix(round ^ 0x9) % 73) as usize,
        floor: [0.12, 0.06, 0.0][(round % 3) as usize],
    }
}

fn rows_of(rows: &HashMap<Uuid, Vec<u64>>, artists: &[Uuid]) -> Vec<(u64, Uuid)> {
    let mut artists = artists.to_vec();
    artists.sort();
    artists
        .iter()
        .flat_map(|artist| {
            rows.get(artist)
                .into_iter()
                .flatten()
                .map(move |track| (*track, *artist))
        })
        .collect()
}

#[test]
fn reading_only_the_artists_that_can_reach_the_cut_keeps_every_pick() {
    let mut pruned = 0;
    let mut capped = 0;
    for round in 0..ROUNDS {
        let world = world(round);
        let ranked = top_affinity_artists(&world.affinity, GRAPH_ARTISTS);
        let reach = Reach {
            affinity: &world.affinity,
            disliked_artists: &world.disliked,
            cursor: &world.cursor,
            picks: world.picks,
            artist_cap: ARTIST_CAP_IN_WINDOW,
            graph_floor: world.floor,
            content_floor: CONTENT_FLOOR,
        };

        if rows_of(&world.rows, &ranked).len() > GRAPH_TRACKS_TOTAL {
            capped += 1;
        }
        let mert = mert_map(&world.mert);
        let full = main_wave(&reach, &ranked, &world.rows, &mert, rough);
        let (reached, read) = reached_wave(&reach, &ranked, &world.rows, &mert, rough);
        if read < ranked.len() {
            pruned += 1;
        }

        assert_eq!(flat(&reached), flat(&full), "round {round}");
    }
    assert!(
        pruned > ROUNDS / 4,
        "the cut pruned artists in {pruned} rounds"
    );
    assert!(capped > 10, "the row cap bound in {capped} rounds");
}

#[test]
fn a_tail_track_that_mert_brought_without_an_artist_is_scored_as_its_tail_artist() {
    assert_the_tail_artist_claims(None);
}

#[test]
fn a_tail_track_that_mert_brought_under_another_artist_is_scored_as_its_tail_artist() {
    assert_the_tail_artist_claims(Some(Uuid::from_u128(1)));
}

const LOUD: u64 = 10;

fn even(track: u64, artist: Option<Uuid>) -> Candidate {
    Candidate {
        sc_track_id: track,
        artist,
        content: if track == LOUD { 1.0 } else { 0.9 },
        skip_damp: 1.0,
    }
}

fn assert_the_tail_artist_claims(mert_artist: Option<Uuid>) {
    let mut affinity = Affinity::new();
    for (artist, value) in [(1, 1.0), (2, 0.95), (3, 0.9), (4, 0.05)] {
        affinity.insert(Uuid::from_u128(artist), value);
    }
    let rows: HashMap<Uuid, Vec<u64>> = [
        (1, vec![1, 2]),
        (2, vec![3, 4]),
        (3, vec![LOUD, 11]),
        (4, vec![12]),
    ]
    .into_iter()
    .map(|(artist, tracks)| (Uuid::from_u128(artist), tracks))
    .collect();
    let mert = HashMap::from([(LOUD, mert_artist)]);
    let ranked = top_affinity_artists(&affinity, GRAPH_ARTISTS);
    let cursor = WaveCursor::new(SeedKind::User, "claims".to_owned());
    let disliked = HashSet::new();
    let reach = Reach {
        affinity: &affinity,
        disliked_artists: &disliked,
        cursor: &cursor,
        picks: 2,
        artist_cap: ARTIST_CAP_IN_WINDOW,
        graph_floor: GRAPH_FLOOR,
        content_floor: CONTENT_FLOOR,
    };

    let (reached, read) = reached_wave(&reach, &ranked, &rows, &mert, even);

    assert_eq!(read, 3, "the tail artist is read, the far one is not");
    assert!(
        reached
            .iter()
            .any(|track| track.sc_track_id == LOUD && track.artist == Some(Uuid::from_u128(3))),
        "{reached:?}"
    );
    assert_eq!(
        flat(&reached),
        flat(&main_wave(&reach, &ranked, &rows, &mert, even))
    );
}

#[test]
fn the_row_cap_keeps_the_closest_artists_whatever_their_ids() {
    let ranked: Vec<Uuid> = [9_u128, 3, 7, 1].map(Uuid::from_u128).to_vec();
    let rows: Vec<(u64, Uuid)> = [1_u128, 3, 7, 9]
        .iter()
        .flat_map(|id| (0..3).map(move |n| (*id as u64 * 10 + n, Uuid::from_u128(*id))))
        .collect();
    let kept = reach::in_rank_order(rows, &ranked, 7);
    let ids: Vec<u64> = kept.iter().map(|(track, _)| *track).collect();
    assert_eq!(ids, vec![90, 91, 92, 30, 31, 32, 70]);
}

fn artist_id(index: usize) -> Uuid {
    Uuid::from_u128((u128::from(mix(index as u64)) << 64) | index as u128)
}

fn first_track(index: usize) -> u64 {
    100_000 + index as u64 * 1_000
}

fn catalog(index: usize) -> u64 {
    if index < DEEP {
        90
    } else if index < LIKED {
        40
    } else if index < LIKED + CLOSE {
        20
    } else {
        3
    }
}

fn play(track: u64) -> u64 {
    track * 7919 % 100_003
}

struct Library {
    exclude: Vec<String>,
    mert: Vec<(u64, Option<Uuid>)>,
    far: HashSet<Uuid>,
}

async fn library(pool: &PgPool) -> anyhow::Result<Library> {
    let total = LIKED + CLOSE + FAR;
    let ids: Vec<Uuid> = (0..total).map(artist_id).collect();
    sqlx::query(
        "INSERT INTO artists (id, name, normalized_name, source)
         SELECT id, id::text, id::text, 'test' FROM unnest($1::uuid[]) AS id",
    )
    .bind(&ids)
    .execute(pool)
    .await?;
    let plan: Vec<String> = (0..total)
        .map(|index| {
            format!(
                "('{}'::uuid, {}, {})",
                artist_id(index),
                first_track(index),
                catalog(index)
            )
        })
        .collect();
    crate::db::insert_replicated(
        pool,
        &format!(
            "WITH plan (artist_id, first, count) AS (VALUES {}),
                  made AS (
                      INSERT INTO tracks (sc_track_id, urn, title, title_normalized, duration_ms,
                                          sharing, storage_state, index_state)
                      SELECT n::text, 'urn:' || n, 'T' || n, 't' || n, 1000, 'public', 'ok', 'indexed'
                      FROM plan p
                               CROSS JOIN LATERAL generate_series(p.first, p.first + p.count - 1) AS n
                      RETURNING id, sc_track_id
                  ),
                  credited AS (
                      INSERT INTO track_artists (track_id, artist_id, role, source)
                      SELECT m.id, p.artist_id, 'primary', 'test'
                      FROM made m
                               JOIN plan p
                                    ON m.sc_track_id::bigint BETWEEN p.first AND p.first + p.count - 1
                  )
             INSERT INTO sc_track_counters (sc_track_id, play_count)
             SELECT sc_track_id, (sc_track_id::bigint * 7919) % 100003 FROM made",
            plan.join(", ")
        ),
    )
    .await?;

    let mut from = Vec::new();
    let mut to = Vec::new();
    let mut weight: Vec<f32> = Vec::new();
    for liked in 0..LIKED {
        from.extend([artist_id(liked), artist_id(liked)]);
        to.extend([
            artist_id(LIKED + liked % CLOSE),
            artist_id(LIKED + (liked + 7) % CLOSE),
        ]);
        weight.extend([1.0 - 0.3 * liked as f32 / LIKED as f32, 0.5]);
    }
    for far in 0..FAR {
        from.push(artist_id(far % LIKED));
        to.push(artist_id(LIKED + CLOSE + far));
        weight.push(0.02 + 0.0002 * far as f32);
    }
    sqlx::query(
        "INSERT INTO artist_coplay (a_id, b_id, weight)
         SELECT LEAST(a, b), GREATEST(a, b), w
         FROM unnest($1::uuid[], $2::uuid[], $3::real[]) AS e(a, b, w)",
    )
    .bind(&from)
    .bind(&to)
    .bind(&weight)
    .execute(pool)
    .await?;

    let mut liked_tracks: Vec<i64> = Vec::new();
    for liked in 0..LIKED {
        let count = 5 - liked / 16;
        for offset in 0..count {
            liked_tracks.push((first_track(liked) + offset as u64) as i64);
        }
    }
    sqlx::query(
        "INSERT INTO user_likes_tracks (user_id, sc_track_id, wanted_state, created_at)
         SELECT $1, id::text, true, now() - interval '1 hour' FROM unnest($2::int8[]) AS id",
    )
    .bind(LISTENER)
    .bind(&liked_tracks)
    .execute(pool)
    .await?;

    let mut exclude: Vec<String> = liked_tracks.iter().map(i64::to_string).collect();
    for deep in 0..DEEP {
        let mut heard: Vec<u64> = (first_track(deep)..first_track(deep) + catalog(deep)).collect();
        heard.sort_by_key(|track| (std::cmp::Reverse(play(*track)), track.to_string()));
        exclude.extend(heard.iter().take(DEEP_HEARD).map(u64::to_string));
    }
    exclude.sort();
    exclude.dedup();

    let mut mert: Vec<(u64, Option<Uuid>)> = (0..40_u64)
        .map(|n| {
            (
                9_000_000 + n,
                Some(Uuid::from_u128(0xdead_0000 + u128::from(n))),
            )
        })
        .collect();
    mert.extend((0..LIKED).map(|liked| (first_track(liked) + 30, Some(artist_id(liked)))));
    mert.extend((0..60).map(|far| {
        let index = LIKED + CLOSE + far;
        (first_track(index), Some(artist_id(index)))
    }));
    Ok(Library {
        exclude,
        mert,
        far: (LIKED + CLOSE..total).map(artist_id).collect(),
    })
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn a_large_library_gets_the_same_wave_from_fewer_artist_reads(
    pool: PgPool,
) -> anyhow::Result<()> {
    let library = library(&pool).await?;
    let variants = user_id_variants(LISTENER);
    let affinity = affinity_from(&GraphEdges::new(), &pool, &variants, &[], &GraphSeed::User).await;
    let ranked = top_affinity_artists(&affinity, GRAPH_ARTISTS);
    assert_eq!(
        ranked.len(),
        GRAPH_ARTISTS,
        "every graph slot must be taken"
    );
    let mut cursor = WaveCursor::new(SeedKind::User, LISTENER.to_owned());
    for track in [first_track(0) + 5, first_track(0) + 6] {
        cursor.mark_served(track, Some(artist_id(0)));
    }
    let disliked = HashSet::new();
    let reach = Reach {
        affinity: &affinity,
        disliked_artists: &disliked,
        cursor: &cursor,
        picks: LIMIT * 2,
        artist_cap: ARTIST_CAP_IN_WINDOW,
        graph_floor: GRAPH_FLOOR,
        content_floor: CONTENT_FLOOR,
    };

    let old_rows = ArtistTopTracks::new()
        .collect(
            &pool,
            &ranked,
            &library.exclude,
            GRAPH_PER_ARTIST,
            GRAPH_TRACKS_TOTAL as i64,
        )
        .await;
    assert!(
        old_rows.len() < GRAPH_TRACKS_TOTAL,
        "the old read must stay under its row cap: {}",
        old_rows.len()
    );
    let mert = mert_map(&library.mert);
    let old = pick(
        &reach,
        &scored(reach::attribution(&old_rows, &mert), listened),
    );

    let tops = ArtistTopTracks::new();
    let (head, rest) = reach.split(&ranked);
    let head_rows = reach::read(
        &tops,
        &pool,
        head,
        &ranked,
        &library.exclude,
        GRAPH_PER_ARTIST,
        GRAPH_TRACKS_TOTAL,
    )
    .await;
    let settled: HashSet<u64> = head_rows.iter().map(|(track, _)| *track).collect();
    let mut cands = scored(reach::attribution(&head_rows, &mert), listened);
    let tail = reach.tail(rest, &cands, &settled);
    let tail_rows = reach::read(
        &tops,
        &pool,
        &tail,
        &ranked,
        &library.exclude,
        GRAPH_PER_ARTIST,
        GRAPH_TRACKS_TOTAL - head_rows.len(),
    )
    .await;
    let read_graph: Vec<(u64, Uuid)> = head_rows.iter().chain(&tail_rows).copied().collect();
    let fresh = reach::reclaim(&mut cands, &read_graph, &mert);
    cands.extend(scored(fresh, listened));
    let new = pick(&reach, &cands);

    assert_eq!(old.len(), reach.picks);
    assert_eq!(flat(&new), flat(&old));

    let read: Vec<Uuid> = head.iter().chain(&tail).copied().collect();
    assert!(
        read.len() <= LIKED + CLOSE,
        "read {} of {} artists",
        read.len(),
        ranked.len()
    );
    assert!(read.iter().all(|artist| !library.far.contains(artist)));
    let read_rows: HashSet<(u64, Uuid)> = head_rows.iter().chain(&tail_rows).copied().collect();
    let old_read_rows: HashSet<(u64, Uuid)> = old_rows
        .iter()
        .filter(|(_, artist)| read.contains(artist))
        .copied()
        .collect();
    assert_eq!(read_rows, old_read_rows);

    let excluded: HashSet<&str> = library.exclude.iter().map(String::as_str).collect();
    for deep in 0..DEEP {
        let rows: Vec<u64> = read_rows
            .iter()
            .filter(|(_, artist)| *artist == artist_id(deep))
            .map(|(track, _)| *track)
            .collect();
        assert_eq!(rows.len(), GRAPH_PER_ARTIST as usize, "deep artist {deep}");
        assert!(
            rows.iter()
                .all(|track| !excluded.contains(track.to_string().as_str()))
        );
    }
    Ok(())
}
