use std::sync::atomic::{AtomicU64, Ordering};

use chrono::{DateTime, Utc};
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;
use write_through as wt;
use wt::{AppTokenReject, ColdRefreshClear, EnqueuedJob, JobEnqueue, LyricsWake, ObserveClaim};

use crate::harness::Pair;

static JOBS: AtomicU64 = AtomicU64::new(1);
const TRACK_ID: &str = "6f1c2b9e-4d7a-4c3e-9b8f-2a5d7e1c0b94";
const APP_ID: &str = "1d3f5a7c-2b4e-4c6a-8d0f-9e1b3a5c7d2f";
const GENERATION: &str = "5a7c9e1b-3d5f-4a7c-9e1b-3d5f7a9c1e3b";
const CORE_CLOCK: [&str; 2] = ["background_jobs", "job_status"];
const PLAYLIST: &str = "soundcloud:playlists:42";

fn at(stamp: &str) -> DateTime<Utc> {
    stamp.parse().unwrap()
}

fn job(kind: &str, dedup_key: &str, priority: i16, raise_priority: bool) -> EnqueuedJob {
    EnqueuedJob {
        id: Uuid::from_u128(JOBS.fetch_add(1, Ordering::Relaxed).into()),
        kind: kind.to_owned(),
        lane: "core_bulk".to_owned(),
        dedup_key: dedup_key.to_owned(),
        payload: json!({"version": "1", "payload": {"sc_id": "42"}}),
        priority,
        max_attempts: 8,
        raise_priority,
        star_candidates: Vec::new(),
        star_bonus: 0,
    }
}

fn enqueue(jobs: Vec<EnqueuedJob>) -> JobEnqueue {
    JobEnqueue { jobs, claim: None }
}

async fn queued(pool: &PgPool, kind: &str, dedup_key: &str) -> Option<(i16, Option<String>)> {
    sqlx::query_as(
        "SELECT j.priority, s.state FROM public.background_jobs j
         LEFT JOIN public.job_status s ON s.kind = j.kind AND s.dedup_key = j.dedup_key
         WHERE j.kind = $1 AND j.dedup_key = $2",
    )
    .bind(kind)
    .bind(dedup_key)
    .fetch_optional(pool)
    .await
    .unwrap()
}

async fn count(pool: &PgPool, table: &str) -> i64 {
    sqlx::query_scalar(&format!("SELECT count(*) FROM public.{table}"))
        .fetch_one(pool)
        .await
        .unwrap()
}

#[tokio::test]
async fn an_enqueue_inserts_once_writes_the_status_and_dedups_a_second_one() {
    let pair = Pair::new("").await;
    let first = pair
        .run(
            &enqueue(vec![job("catalog.refresh", "track:42:public", 5, true)]),
            &CORE_CLOCK,
        )
        .await;
    assert_eq!(first.core, json!({"claimed": true, "inserted": 1}));
    assert_eq!(first.assert_same(), json!(null));
    assert_eq!(
        queued(&pair.core, "catalog.refresh", "track:42:public").await,
        Some((5, Some("queued".to_owned())))
    );
    assert_eq!(count(&pair.api, "background_jobs").await, 0);
    assert_eq!(count(&pair.api, "job_status").await, 0);

    let again = pair
        .run(
            &enqueue(vec![job("catalog.refresh", "track:42:public", 5, true)]),
            &CORE_CLOCK,
        )
        .await;
    assert_eq!(again.core, json!({"claimed": true, "inserted": 0}));
    again.assert_same();
    assert_eq!(count(&pair.core, "background_jobs").await, 1);
    assert_eq!(count(&pair.core, "job_status").await, 1);
    pair.finish().await;
}

#[tokio::test]
async fn a_raising_enqueue_lifts_the_priority_and_a_plain_one_keeps_it() {
    let pair = Pair::new("").await;
    pair.run(
        &enqueue(vec![job("catalog.refresh", "track:42:public", 5, true)]),
        &CORE_CLOCK,
    )
    .await
    .assert_same();
    pair.run(
        &enqueue(vec![job("catalog.refresh", "track:42:public", 9, true)]),
        &CORE_CLOCK,
    )
    .await
    .assert_same();
    assert_eq!(
        queued(&pair.core, "catalog.refresh", "track:42:public")
            .await
            .map(|row| row.0),
        Some(9)
    );
    pair.run(
        &enqueue(vec![job("catalog.refresh", "track:42:public", 3, true)]),
        &CORE_CLOCK,
    )
    .await
    .assert_same();
    pair.run(
        &enqueue(vec![job("catalog.refresh", "track:42:public", 20, false)]),
        &CORE_CLOCK,
    )
    .await
    .assert_same();
    assert_eq!(
        queued(&pair.core, "catalog.refresh", "track:42:public")
            .await
            .map(|row| row.0),
        Some(9)
    );
    assert_eq!(count(&pair.core, "job_status").await, 1);
    pair.finish().await;
}

#[tokio::test]
async fn a_batch_inserts_each_new_job_and_only_the_api_read_kinds_get_a_status() {
    let pair = Pair::new("").await;
    let run = pair
        .run(
            &enqueue(vec![
                job("catalog.collection", "owned-tracks:1:public", 5, false),
                job("catalog.collection", "owned-tracks:2:public", 5, false),
                job("lyrics.lookup", "42", 10, false),
            ]),
            &CORE_CLOCK,
        )
        .await;
    assert_eq!(run.core, json!({"claimed": true, "inserted": 3}));
    run.assert_same();
    assert_eq!(count(&pair.core, "background_jobs").await, 3);
    assert_eq!(count(&pair.core, "job_status").await, 2);
    assert_eq!(
        queued(&pair.core, "lyrics.lookup", "42").await,
        Some((10, None))
    );
    pair.finish().await;
}

#[tokio::test]
async fn the_enqueue_writes_a_status_for_exactly_the_kinds_the_status_list_names() {
    let pair = Pair::new(
        "ALTER TABLE public.background_jobs DISABLE TRIGGER zz_job_status_insert;
        CREATE OR REPLACE FUNCTION public.scd_job_status_kinds() RETURNS text[] LANGUAGE sql IMMUTABLE
        AS $f$ SELECT ARRAY['lyrics.lookup'] $f$",
    )
    .await;
    let run = pair
        .run(
            &enqueue(vec![
                job("catalog.refresh", "track:42:public", 5, false),
                job("lyrics.lookup", "42", 10, false),
            ]),
            &CORE_CLOCK,
        )
        .await;
    assert_eq!(run.core, json!({"claimed": true, "inserted": 2}));
    run.assert_same();
    assert_eq!(
        queued(&pair.core, "lyrics.lookup", "42").await,
        Some((10, Some("queued".to_owned())))
    );
    assert_eq!(
        queued(&pair.core, "catalog.refresh", "track:42:public").await,
        Some((5, None))
    );
    pair.finish().await;
}

#[tokio::test]
async fn a_replayed_enqueue_command_is_a_no_op() {
    let pair = Pair::new("").await;
    pair.run(
        &enqueue(vec![job("catalog.refresh", "track:42:public", 5, true)]),
        &CORE_CLOCK,
    )
    .await
    .assert_same();
    assert!(pair.redeliver().await);
    assert_eq!(count(&pair.core, "background_jobs").await, 1);
    assert_eq!(count(&pair.core, "job_status").await, 1);
    pair.finish().await;
}

const PLAYLIST_FIXTURE: &str =
    "INSERT INTO public.playlists (urn, sc_playlist_id, title, title_normalized, owner_sc_user_id)
    VALUES ('soundcloud:playlists:42', '42', 'Mix', 'mix', '17');
    INSERT INTO public.playlist_membership_state (playlist_urn, owner_sc_user_id, next_reconcile_at)
    VALUES ('soundcloud:playlists:42', '17', '2026-10-30 09:00:00+00')";

fn observe(stamp: &str) -> JobEnqueue {
    JobEnqueue {
        jobs: vec![job("playlists.observe_shadow", PLAYLIST, 0, false)],
        claim: Some(ObserveClaim {
            playlist_urn: PLAYLIST.to_owned(),
            at: at(stamp),
        }),
    }
}

async fn reconcile_at(pool: &PgPool) -> DateTime<Utc> {
    sqlx::query_scalar("SELECT next_reconcile_at FROM public.playlist_membership_state")
        .fetch_one(pool)
        .await
        .unwrap()
}

#[tokio::test]
async fn a_view_claims_the_playlist_once_and_a_second_view_inside_the_window_adds_nothing() {
    let pair = Pair::new(PLAYLIST_FIXTURE).await;
    let first = pair
        .run(&observe("2026-10-30T10:00:00Z"), &CORE_CLOCK)
        .await;
    assert_eq!(first.core, json!({"claimed": true, "inserted": 1}));
    first.assert_same();
    assert_eq!(reconcile_at(&pair.core).await, at("2026-10-30T10:00:30Z"));
    assert_eq!(
        queued(&pair.core, "playlists.observe_shadow", PLAYLIST).await,
        Some((0, Some("queued".to_owned())))
    );

    let second = pair
        .run(&observe("2026-10-30T10:00:10Z"), &CORE_CLOCK)
        .await;
    assert_eq!(second.core, json!({"claimed": false, "inserted": 0}));
    second.assert_same();
    assert_eq!(reconcile_at(&pair.core).await, at("2026-10-30T10:00:30Z"));

    let late = pair
        .run(&observe("2026-10-30T10:00:31Z"), &CORE_CLOCK)
        .await;
    assert_eq!(late.core, json!({"claimed": true, "inserted": 0}));
    late.assert_same();
    assert_eq!(reconcile_at(&pair.core).await, at("2026-10-30T10:01:01Z"));
    assert_eq!(count(&pair.core, "background_jobs").await, 1);
    pair.finish().await;
}

#[tokio::test]
async fn a_deleted_playlist_is_not_claimed() {
    let pair = Pair::new(&format!(
        "{PLAYLIST_FIXTURE}; UPDATE public.playlists SET deleted_at = '2026-10-29 00:00:00+00'"
    ))
    .await;
    let run = pair
        .run(&observe("2026-10-30T10:00:00Z"), &CORE_CLOCK)
        .await;
    assert_eq!(run.core, json!({"claimed": false, "inserted": 0}));
    run.assert_same();
    assert_eq!(count(&pair.core, "background_jobs").await, 0);
    pair.finish().await;
}

const LYRICS_FIXTURE: &str = "INSERT INTO public.tracks (id, sc_track_id, urn, title, title_normalized, duration_ms, created_at, updated_at)
    VALUES ('6f1c2b9e-4d7a-4c3e-9b8f-2a5d7e1c0b94', '1874620031', 'soundcloud:tracks:1874620031',
        'Night Drive', 'night drive', 213000, '2026-01-15 12:00:05+00', '2026-10-28 08:00:00+00');
    DELETE FROM public.lyrics_lookup_state";

fn wake(stamp: &str) -> LyricsWake {
    LyricsWake {
        sc_track_id: "1874620031".to_owned(),
        at: at(stamp),
    }
}

async fn lookup(pool: &PgPool) -> (String, i64, Option<i64>, bool) {
    sqlx::query_as(
        "SELECT status, generation, wake_generation, wake_message_id IS NOT NULL
         FROM public.lyrics_lookup_state",
    )
    .fetch_one(pool)
    .await
    .unwrap()
}

#[tokio::test]
async fn a_wake_creates_the_lookup_state_and_a_second_one_changes_nothing() {
    let pair = Pair::new(LYRICS_FIXTURE).await;
    let first = pair.run(&wake("2026-10-30T10:00:00Z"), &[]).await;
    let state = first.assert_same();
    assert_eq!(state["status"], "pending");
    assert_eq!(
        lookup(&pair.api).await,
        ("pending".to_owned(), 1, Some(1), true)
    );
    assert_eq!(
        lookup(&pair.core).await,
        ("pending".to_owned(), 1, Some(1), true)
    );
    pair.run(&wake("2026-10-30T10:00:05Z"), &[])
        .await
        .assert_same();
    assert_eq!(
        lookup(&pair.core).await,
        ("pending".to_owned(), 1, Some(1), true)
    );
    assert!(pair.redeliver().await);
    pair.finish().await;
}

#[tokio::test]
async fn a_wake_reopens_a_missed_lookup_only_when_its_time_has_come() {
    let pair = Pair::new(&format!(
        "{LYRICS_FIXTURE}; INSERT INTO public.lyrics_lookup_state
            (track_id, sc_track_id, status, generation, priority, next_run_at, input_title, input_artist,
             input_duration_ms, created_at, updated_at)
         VALUES ('{TRACK_ID}', '1874620031', 'not_found', 3, 3, '2026-10-31 00:00:00+00', 'Night Drive', '',
             213000, '2026-01-15 12:00:05+00', '2026-10-29 00:00:00+00')"
    ))
    .await;
    let early = pair.run(&wake("2026-10-30T10:00:00Z"), &[]).await;
    assert_eq!(early.assert_same()["status"], "not_found");
    assert_eq!(lookup(&pair.core).await.1, 3);
    let due = pair.run(&wake("2026-10-31T00:00:01Z"), &[]).await;
    assert_eq!(due.assert_same()["status"], "pending");
    assert_eq!(
        lookup(&pair.core).await,
        ("pending".to_owned(), 4, Some(4), true)
    );
    pair.finish().await;
}

#[tokio::test]
async fn a_wake_for_an_unknown_track_answers_nothing() {
    let pair = Pair::new("").await;
    let run = pair.run(&wake("2026-10-30T10:00:00Z"), &[]).await;
    assert_eq!(run.assert_same(), json!(null));
    pair.finish().await;
}

const CLEAR_FIXTURE: &str = "INSERT INTO public.catalog_audience (subject_urn, relation, user_urn, created_at)
    VALUES ('soundcloud:tracks:42', 'track-favoriters', 'soundcloud:users:1', '2026-10-01 00:00:00+00'),
           ('soundcloud:tracks:42', 'track-reposters', 'soundcloud:users:2', '2026-10-01 00:00:00+00'),
           ('soundcloud:tracks:43', 'track-favoriters', 'soundcloud:users:1', '2026-10-01 00:00:00+00');
    INSERT INTO public.catalog_collection_sync (subject_id, collection, scope, job_id, generation, snapshot_id)
    VALUES ('42', 'track-favoriters', 'public', gen_random_uuid(), 1, gen_random_uuid()),
           ('42', 'track-comments', 'public', gen_random_uuid(), 1, gen_random_uuid()),
           ('42', 'liked-tracks', 'owner', gen_random_uuid(), 1, gen_random_uuid());
    INSERT INTO public.track_comments (id, sc_track_id, user_urn, body, created_at)
    VALUES (gen_random_uuid(), '42', 'soundcloud:users:1', 'one', '2026-10-01 00:00:00+00'),
           (gen_random_uuid(), '43', 'soundcloud:users:1', 'two', '2026-10-01 00:00:00+00')";

#[tokio::test]
async fn clearing_an_audience_removes_its_rows_and_sync_state_on_both_nodes() {
    let pair = Pair::new(CLEAR_FIXTURE).await;
    let run = pair
        .run(
            &ColdRefreshClear::Audience {
                subject_urn: "soundcloud:tracks:42".to_owned(),
                subject_id: "42".to_owned(),
                relations: vec!["track-favoriters".to_owned(), "track-reposters".to_owned()],
            },
            &[],
        )
        .await;
    assert_eq!(run.assert_same(), json!(3));
    for pool in [&pair.api, &pair.core] {
        assert_eq!(count(pool, "catalog_audience").await, 1);
        assert_eq!(count(pool, "catalog_collection_sync").await, 2);
    }
    assert!(pair.redeliver().await);
    pair.finish().await;
}

#[tokio::test]
async fn clearing_comments_removes_one_tracks_comments_and_its_sync_state() {
    let pair = Pair::new(CLEAR_FIXTURE).await;
    let run = pair
        .run(
            &ColdRefreshClear::Comments {
                sc_track_id: "42".to_owned(),
            },
            &[],
        )
        .await;
    assert_eq!(run.assert_same(), json!(2));
    for pool in [&pair.api, &pair.core] {
        assert_eq!(count(pool, "track_comments").await, 1);
        assert_eq!(count(pool, "catalog_collection_sync").await, 2);
    }
    let again = pair
        .run(
            &ColdRefreshClear::Comments {
                sc_track_id: "42".to_owned(),
            },
            &[],
        )
        .await;
    assert_eq!(again.assert_same(), json!(0));
    pair.finish().await;
}

const TOKEN_FIXTURE: &str = "INSERT INTO public.oauth_apps (id, name, client_id, client_secret, redirect_uri)
    VALUES ('1d3f5a7c-2b4e-4c6a-8d0f-9e1b3a5c7d2f', 'app', 'cid', 'secret', 'https://x.test/cb');
    INSERT INTO public.oauth_app_tokens (oauth_app_id, access_token, expires_at, generation)
    VALUES ('1d3f5a7c-2b4e-4c6a-8d0f-9e1b3a5c7d2f', 'tok', '2026-10-31 00:00:00+00', '5a7c9e1b-3d5f-4a7c-9e1b-3d5f7a9c1e3b')";

fn reject(generation: &str) -> AppTokenReject {
    AppTokenReject {
        oauth_app_id: APP_ID.parse().unwrap(),
        generation: generation.parse().unwrap(),
        at: at("2026-10-30T10:00:00Z"),
    }
}

async fn expires(pool: &PgPool) -> DateTime<Utc> {
    sqlx::query_scalar("SELECT expires_at FROM public.oauth_app_tokens")
        .fetch_one(pool)
        .await
        .unwrap()
}

#[tokio::test]
async fn a_rejected_generation_expires_now_and_a_stale_generation_changes_nothing() {
    let pair = Pair::new(TOKEN_FIXTURE).await;
    let stale = pair
        .run(&reject("00000000-0000-0000-0000-000000000001"), &[])
        .await;
    assert_eq!(stale.assert_same(), json!(false));
    assert_eq!(expires(&pair.core).await, at("2026-10-31T00:00:00Z"));
    let run = pair.run(&reject(GENERATION), &[]).await;
    assert_eq!(run.assert_same(), json!(true));
    assert_eq!(expires(&pair.api).await, at("2026-10-30T10:00:00Z"));
    assert_eq!(expires(&pair.core).await, at("2026-10-30T10:00:00Z"));
    assert!(pair.redeliver().await);
    let again = pair.run(&reject(GENERATION), &[]).await;
    assert_eq!(again.assert_same(), json!(true));
    assert_eq!(expires(&pair.core).await, at("2026-10-30T10:00:00Z"));
    pair.finish().await;
}
