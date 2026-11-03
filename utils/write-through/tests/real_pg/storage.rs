use chrono::{DateTime, Utc};
use serde_json::json;
use write_through as wt;
use wt::TrackStorageLost;

use crate::harness::Pair;

const URN: &str = "soundcloud:tracks:1874620031";
const STORED: &str = "INSERT INTO public.tracks (
        id, sc_track_id, urn, title, title_normalized, duration_ms, storage_state, storage_quality,
        s3_verified_at, created_at, updated_at)
    VALUES ('6f1c2b9e-4d7a-4c3e-9b8f-2a5d7e1c0b94', '1874620031', 'soundcloud:tracks:1874620031',
        'Night Drive', 'night drive', 213000, 'ok', 'hq',
        '2026-10-01 10:00:00+00', '2026-01-15 12:00:05+00', '2026-10-28 08:00:00+00')";

fn at(stamp: &str) -> DateTime<Utc> {
    stamp.parse().unwrap()
}

fn lost() -> TrackStorageLost {
    TrackStorageLost {
        urn: URN.to_owned(),
        at: at("2026-10-30T09:15:00Z"),
    }
}

#[tokio::test]
async fn a_lost_track_reads_the_same_on_api_and_core() {
    let pair = Pair::new(STORED).await;

    let marked = pair.run(&lost(), &[]).await.assert_same();

    assert_eq!(marked, json!(true));
    let (state, quality, missing): (String, Option<String>, DateTime<Utc>) = sqlx::query_as(
        "SELECT storage_state, storage_quality, s3_missing_at FROM public.tracks WHERE urn = $1",
    )
    .bind(URN)
    .fetch_one(&pair.core)
    .await
    .unwrap();
    assert_eq!(
        (state.as_str(), quality, missing),
        ("pending", None, at("2026-10-30T09:15:00Z"))
    );
    pair.finish().await;
}

#[tokio::test]
async fn a_track_already_pending_on_core_is_handed_back_not_dead_lettered() {
    let pair = Pair::new(STORED).await;
    sqlx::query(
        "UPDATE public.tracks SET storage_state = 'pending', storage_quality = NULL,
                s3_verified_at = NULL, s3_missing_at = '2026-10-30 09:00:00+00',
                updated_at = '2026-10-30 09:00:00+00'
         WHERE urn = $1",
    )
    .bind(URN)
    .execute(&pair.core)
    .await
    .unwrap();

    let run = pair.run(&lost(), &[]).await;

    assert_eq!((&run.api, &run.core), (&json!(true), &json!(false)));
    assert!(
        run.diverged
            .iter()
            .any(|d| d.contains("reasserted: 1") && d.contains("dead: 0")),
        "core must send its row back to the api node: {:#?}",
        run.diverged
    );
    pair.finish().await;
}
