use chrono::{DateTime, Utc};
use serde_json::json;
use sqlx::PgPool;
use write_through as wt;
use wt::{TrackReleaseHeld, TrackS3State, TrackTooLong};

use crate::harness::Pair;

const DERIVED: &[&str] = &["lyrics_lookup_state"];
const TRACKS: &str = "INSERT INTO public.tracks (
        id, sc_track_id, urn, title, title_normalized, duration_ms, pipeline_held,
        storage_state, storage_quality, index_state, s3_verified_at, s3_missing_at,
        created_at, updated_at, transcribe_state)
    VALUES
        ('3a6e1d20-5b8c-4f19-9d27-0c4b8e7f2a01', '2001', 'soundcloud:tracks:2001',
         'Held', 'held', 213000, true, 'pending', NULL, 'pending', NULL, NULL,
         '2026-10-01 10:00:00+00', '2026-10-01 10:00:00+00', DEFAULT),
        ('3a6e1d20-5b8c-4f19-9d27-0c4b8e7f2a02', '2002', 'soundcloud:tracks:2002',
         'Long mix', 'long mix', 3600000, false, 'pending', NULL, 'pending', NULL, NULL,
         '2026-10-01 10:00:00+00', '2026-10-01 10:00:00+00', DEFAULT),
        ('3a6e1d20-5b8c-4f19-9d27-0c4b8e7f2a03', '2003', 'soundcloud:tracks:2003',
         'Stored', 'stored', 200000, false, 'ok', 'hq', 'indexed', NULL, NULL,
         '2026-10-01 10:00:00+00', '2026-10-01 10:00:00+00', DEFAULT),
        ('3a6e1d20-5b8c-4f19-9d27-0c4b8e7f2a04', '2004', 'soundcloud:tracks:2004',
         'Lost', 'lost', 200000, false, 'missing', 'sq', 'indexed', NULL, '2026-10-20 10:00:00+00',
         '2026-10-01 10:00:00+00', '2026-10-01 10:00:00+00', DEFAULT),
        ('3a6e1d20-5b8c-4f19-9d27-0c4b8e7f2a05', '2005', 'soundcloud:tracks:2005',
         'Too long', 'too long', 9000000, false, 'too_long', NULL, 'too_long', NULL, NULL,
         '2026-10-01 10:00:00+00', '2026-10-01 10:00:00+00', 'disabled')";

fn at(stamp: &str) -> DateTime<Utc> {
    stamp.parse().unwrap()
}

fn too_long(sc_track_id: &str) -> TrackTooLong {
    TrackTooLong {
        sc_track_id: sc_track_id.to_owned(),
        at: at("2026-10-31T08:00:00Z"),
    }
}

fn release(sc_track_id: &str) -> TrackReleaseHeld {
    TrackReleaseHeld {
        sc_track_id: sc_track_id.to_owned(),
        at: at("2026-10-31T08:00:00Z"),
    }
}

fn verdicts(ok: &[&str], missing: &[&str]) -> TrackS3State {
    TrackS3State {
        ok: ok.iter().map(|id| (*id).to_owned()).collect(),
        missing: missing.iter().map(|id| (*id).to_owned()).collect(),
        at: at("2026-10-31T08:00:00Z"),
    }
}

type Storage = (
    String,
    Option<String>,
    Option<DateTime<Utc>>,
    Option<DateTime<Utc>>,
);

async fn storage(pool: &PgPool, sc_track_id: &str) -> Storage {
    sqlx::query_as(
        "SELECT storage_state, storage_quality, s3_verified_at, s3_missing_at
         FROM public.tracks WHERE sc_track_id = $1",
    )
    .bind(sc_track_id)
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn on_core(pair: &Pair, sql: &str) {
    sqlx::raw_sql(sql).execute(&pair.core).await.unwrap();
}

fn handed_back(diverged: &[String]) -> bool {
    diverged
        .iter()
        .any(|d| d.contains("reasserted: 1") && d.contains("dead: 0"))
}

#[tokio::test]
async fn a_too_long_track_is_marked_the_same_on_api_and_core() {
    let pair = Pair::new(TRACKS).await;

    let marked = pair.run(&too_long("2002"), &[]).await.assert_same();

    assert_eq!(marked, json!(true));
    let states: (String, String, Option<String>, bool, DateTime<Utc>) = sqlx::query_as(
        "SELECT storage_state, index_state, transcribe_state, hq_upgrade_pending, updated_at
         FROM public.tracks WHERE sc_track_id = '2002'",
    )
    .fetch_one(&pair.core)
    .await
    .unwrap();
    assert_eq!(
        states,
        (
            "too_long".to_owned(),
            "too_long".to_owned(),
            Some("disabled".to_owned()),
            false,
            at("2026-10-31T08:00:00Z")
        )
    );
    assert!(
        pair.redeliver().await,
        "a redelivered command changes nothing"
    );
    pair.finish().await;
}

#[tokio::test]
async fn marking_a_track_core_already_marked_hands_the_row_back() {
    let pair = Pair::new(TRACKS).await;
    on_core(
        &pair,
        "UPDATE public.tracks SET storage_state = 'too_long', index_state = 'too_long',
                transcribe_state = 'disabled', updated_at = '2026-10-30 08:00:00+00'
         WHERE sc_track_id = '2002'",
    )
    .await;

    let run = pair.run(&too_long("2002"), &[]).await;

    assert_eq!((&run.api, &run.core), (&json!(true), &json!(false)));
    assert!(handed_back(&run.diverged), "{:#?}", run.diverged);
    pair.finish().await;
}

#[tokio::test]
async fn a_track_already_too_long_is_not_touched() {
    let pair = Pair::new(TRACKS).await;

    let marked = pair.run(&too_long("2005"), &[]).await.assert_same();

    assert_eq!(marked, json!(false));
    pair.finish().await;
}

#[tokio::test]
async fn a_held_track_is_released_once_and_core_derives_its_lyrics_state() {
    let pair = Pair::new(TRACKS).await;

    let duration = pair.run(&release("2001"), DERIVED).await.assert_same();

    assert_eq!(duration, json!(213000));
    let held: (bool, i64) = sqlx::query_as(
        "SELECT pipeline_held,
                (SELECT count(*) FROM public.lyrics_lookup_state WHERE sc_track_id = '2001')
         FROM public.tracks WHERE sc_track_id = '2001'",
    )
    .fetch_one(&pair.core)
    .await
    .unwrap();
    assert_eq!(held, (false, 1));
    let api_lyrics: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM public.lyrics_lookup_state WHERE sc_track_id = '2001'",
    )
    .fetch_one(&pair.api)
    .await
    .unwrap();
    assert_eq!(api_lyrics, 0, "the api node never derives rows");
    let again = pair.run(&release("2001"), DERIVED).await.assert_same();
    assert_eq!(again, json!(null), "a released track is released once");
    pair.finish().await;
}

#[tokio::test]
async fn a_track_core_released_already_is_handed_back() {
    let pair = Pair::new(TRACKS).await;
    on_core(
        &pair,
        "UPDATE public.tracks SET pipeline_held = false, updated_at = '2026-10-30 08:00:00+00'
         WHERE sc_track_id = '2001'",
    )
    .await;

    let run = pair.run(&release("2001"), DERIVED).await;

    assert_eq!((&run.api, &run.core), (&json!(213000), &json!(null)));
    assert!(handed_back(&run.diverged), "{:#?}", run.diverged);
    pair.finish().await;
}

#[tokio::test]
async fn s3_verdicts_ship_from_the_api_untouched_land_on_core_and_skip_too_long_tracks() {
    let pair = Pair::new(TRACKS).await;

    let run = pair
        .run(&verdicts(&["2003", "9999"], &["2004", "2005"]), &[])
        .await;

    assert_eq!((&run.api, &run.core), (&json!(null), &json!(2)));
    run.assert_same();
    let stamp = Some(at("2026-10-31T08:00:00Z"));
    assert_eq!(
        storage(&pair.core, "2003").await,
        ("ok".to_owned(), Some("hq".to_owned()), stamp, None)
    );
    assert_eq!(
        storage(&pair.core, "2004").await,
        ("pending".to_owned(), None, None, stamp)
    );
    assert_eq!(
        storage(&pair.core, "2005").await,
        ("too_long".to_owned(), None, None, None)
    );
    assert!(
        pair.redeliver().await,
        "a redelivered command changes nothing"
    );
    pair.finish().await;
}

#[tokio::test]
async fn a_track_found_and_missing_in_one_batch_ends_missing_and_is_touched_once() {
    let pair = Pair::new(TRACKS).await;

    let batch = verdicts(&["2003", "2003"], &["2003"]);
    let mut probe = pair.core.begin().await.unwrap();
    let touched: i32 = sqlx::query_scalar(
        "SELECT jsonb_array_length(public.scd_cmd_track_s3_state($1) -> 'touched')",
    )
    .bind(serde_json::to_value(&batch).unwrap())
    .fetch_one(&mut *probe)
    .await
    .unwrap();
    probe.rollback().await.unwrap();
    assert_eq!(touched, 1);

    let run = pair.run(&batch, &[]).await;

    assert_eq!(run.core, json!(1));
    run.assert_same();
    assert_eq!(
        storage(&pair.core, "2003").await,
        (
            "pending".to_owned(),
            None,
            None,
            Some(at("2026-10-31T08:00:00Z"))
        )
    );
    pair.finish().await;
}

#[tokio::test]
async fn a_verdict_on_a_track_core_marked_too_long_changes_nothing_anywhere() {
    let pair = Pair::new(TRACKS).await;
    on_core(
        &pair,
        "UPDATE public.tracks SET storage_state = 'too_long', storage_quality = NULL,
                updated_at = '2026-10-30 08:00:00+00'
         WHERE sc_track_id = '2003'",
    )
    .await;

    let run = pair.run(&verdicts(&["2003"], &[]), &[]).await;

    assert_eq!((&run.api, &run.core), (&json!(null), &json!(0)));
    run.assert_same();
    assert_eq!(
        storage(&pair.core, "2003").await,
        ("too_long".to_owned(), None, None, None)
    );
    pair.finish().await;
}
