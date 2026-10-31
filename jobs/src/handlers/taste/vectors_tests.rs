use std::collections::BTreeMap;

use sqlx::PgPool;

use super::*;

const USERS: u32 = 61;
const ONLY_ZERO_SKIPS_LATELY: &str = "61";
const HISTORY_DAYS: u32 = 180;

impl ItemSource for Items {
    async fn vectors_of(&self, tracks: &[u64]) -> JobResult<Items> {
        Ok(tracks
            .iter()
            .filter_map(|track| Some((*track, self.get(track)?.clone())))
            .collect())
    }
}

async fn seed(pool: &PgPool) -> anyhow::Result<()> {
    sqlx::raw_sql(
        "INSERT INTO user_events (sc_user_id, sc_track_id, event_type, weight, created_at)
         SELECT CASE WHEN k % 3 = 0 THEN 'soundcloud:users:' ELSE '' END || u,
                CASE
                    WHEN u % 7 = 0 AND k <= 90 THEN 10000 + 4 * ((u * 31 + k) % 2000)
                    ELSE 1 + (u * 97 + k * 13) % 9000
                END::text,
                picked.kind,
                CASE picked.kind
                    WHEN 'like' THEN 1.0
                    WHEN 'playlist_add' THEN 0.9
                    WHEN 'full_play' THEN 0.3
                    WHEN 'dislike' THEN -1.0
                    ELSE (ARRAY[-0.8, -0.3, 0.0])[1 + k % 3]
                END,
                (now() AT TIME ZONE 'UTC') - k * interval '19 hours' - (k % 5) * interval '250 milliseconds'
         FROM generate_series(1, 60) AS u
         CROSS JOIN LATERAL generate_series(1, (u * 37) % 241) AS k
         CROSS JOIN LATERAL (
             SELECT (ARRAY['full_play', 'full_play', 'skip', 'skip', 'playlist_add', 'like', 'dislike'])
                        [1 + (k * 5 + u) % 7] AS kind
         ) AS picked;
         INSERT INTO user_events (sc_user_id, sc_track_id, event_type, weight, created_at)
         SELECT sc_user_id, sc_track_id, event_type, weight, created_at + shifts.shift
         FROM user_events
         CROSS JOIN (VALUES (interval '400 milliseconds'), (interval '600 milliseconds')) AS shifts(shift)
         WHERE event_type = 'full_play'
           AND regexp_replace(sc_user_id, '^.*:', '')::int <= 20;
         INSERT INTO user_events (sc_user_id, sc_track_id, event_type, weight, created_at)
         SELECT '61',
                (1 + (k * 41) % 9000)::text,
                CASE WHEN k <= 40 THEN 'skip' ELSE (ARRAY['full_play', 'playlist_add', 'like'])[1 + k % 3] END,
                CASE WHEN k <= 40 THEN 0.0 ELSE (ARRAY[0.3, 0.9, 1.0])[1 + k % 3] END,
                (now() AT TIME ZONE 'UTC') - k * interval '7 hours'
         FROM generate_series(1, 340) AS k;
         INSERT INTO user_likes_tracks (user_id, sc_track_id, wanted_state, created_at, liked_at)
         SELECT CASE WHEN n % 2 = 0 THEN 'soundcloud:users:' ELSE '' END || u,
                (1 + (u * 97 + n * 13) % 9000)::text,
                n % 5 <> 0,
                now() - n * interval '1 day',
                CASE WHEN n % 3 = 0 THEN NULL ELSE now() - n * interval '2 days' END
         FROM generate_series(1, 60) AS u
         CROSS JOIN generate_series(1, 12) AS n
         ON CONFLICT DO NOTHING;
         INSERT INTO user_superlikes (user_id, sc_track_id, like_created_at)
         SELECT user_id, sc_track_id, created_at
         FROM user_likes_tracks
         WHERE wanted_state
           AND regexp_replace(user_id, '^.*:', '')::int % 4 = 0
         ON CONFLICT DO NOTHING;
         INSERT INTO disliked_tracks (sc_user_id, sc_track_id, created_at)
         SELECT u::text, (1 + (u * 89) % 9000)::text, (now() AT TIME ZONE 'UTC') - u * interval '3 hours'
         FROM generate_series(1, 60, 3) AS u;",
    )
    .execute(pool)
    .await?;
    Ok(())
}

fn catalog(dimensions: usize) -> Items {
    (1..20_000_u64)
        .filter(|track| track % 4 != 0)
        .map(|track| {
            let vector = (0..dimensions as u64)
                .map(|slot| ((track * 31 + slot * 17) % 23) as f32 - 11.0)
                .collect();
            (track, vector)
        })
        .collect()
}

fn pooling(window: usize, min_positives: usize) -> anyhow::Result<Pooling> {
    Pooling::from_json(&serde_json::json!({
        "w": {
            "like": 1.0,
            "like_import": 0.7,
            "playlist_add": 0.9,
            "full_play": 0.4,
            "skip": -0.5,
            "dislike": -1.0
        },
        "tau_days": 30.0,
        "window": window,
        "strength": "abs_weight",
        "min_positives": min_positives
    }))
}

fn failed(error: JobError) -> anyhow::Error {
    anyhow::anyhow!("{error}")
}

fn largest_gap(expected: &BTreeMap<String, Vec<f32>>, pooled: &BTreeMap<String, Vec<f32>>) -> f32 {
    expected
        .iter()
        .flat_map(|(user, vector)| {
            let other = pooled.get(user).map(Vec::as_slice).unwrap_or_default();
            vector
                .iter()
                .zip(other)
                .map(|(left, right)| (left - right).abs())
        })
        .fold(0.0, f32::max)
}

#[sqlx::test(migrations = "../api/migrations")]
async fn the_newest_events_pool_every_user_like_the_whole_history(
    pool: PgPool,
) -> anyhow::Result<()> {
    seed(&pool).await?;
    let dimensions = dimensions().map_err(failed)?;
    let catalog = catalog(dimensions);
    let users: Vec<String> = (1..=USERS).map(|user| user.to_string()).collect();
    let now_unix = chrono::Utc::now().timestamp();
    let days = i32::try_from(HISTORY_DAYS)?;

    for (window, min_positives, cut_users) in [(5, 1, true), (5, 12, true), (200, 1, false)] {
        let spec = pooling(window, min_positives)?;
        let whole = full_histories(&pool, days, &spec, &users)
            .await
            .map_err(failed)?;
        let items = catalog
            .vectors_of(&tracks_of(whole.iter()))
            .await
            .map_err(failed)?;
        let expected: BTreeMap<String, Vec<f32>> =
            pool_known(whole, &spec, &items, now_unix, dimensions)
                .into_iter()
                .collect();
        let per_kind = i64::try_from(window * RECENT_EVENTS_PER_WINDOW)?;
        let cut = recent::histories(&pool, days, &variants_of(&users), per_kind)
            .await
            .map_err(failed)?
            .iter()
            .filter(|recent| recent.cutoff_unix_s.is_some())
            .count();

        let pooled = pool_users(&pool, &catalog, HISTORY_DAYS, &spec, &users, now_unix)
            .await
            .map_err(failed)?;
        let full_reads = pooled.full_reads;
        let pooled: BTreeMap<String, Vec<f32>> = pooled.vectors.into_iter().collect();

        assert!(
            expected.len() > 10,
            "window {window}: too few vectors to compare"
        );
        assert!(
            expected.contains_key(ONLY_ZERO_SKIPS_LATELY),
            "window {window}: the user with only zero-weight skips lately has no vector"
        );
        assert_eq!(
            pooled.keys().collect::<Vec<_>>(),
            expected.keys().collect::<Vec<_>>(),
            "window {window}, min positives {min_positives}"
        );
        assert!(
            largest_gap(&expected, &pooled) <= 1e-6,
            "window {window}, min positives {min_positives}"
        );
        if cut_users {
            assert!(full_reads > 0, "window {window}: no user was read in full");
            assert!(
                cut > full_reads,
                "window {window}: no cut user was pooled from its newest events"
            );
        } else {
            assert_eq!((cut, full_reads), (0, 0));
        }
    }
    Ok(())
}
