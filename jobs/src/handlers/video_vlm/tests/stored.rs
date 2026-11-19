use std::time::SystemTime;

use backend_contracts::pipeline::VideoLabel;
use backend_contracts::reasons::WorkerReason;
use serde_json::{Value, json};
use sqlx::PgPool;
use tokio_util::sync::CancellationToken;

use super::fakes::{Eyes, HAIKU, Wire};
use super::haiku::frames_result;
use super::{Rig, answer, bench_on, failure, finish, held_track, stage, vlm_config};
use crate::handlers::video_find::tests::Memory;

const FILE: &[u8] = b"stored video";

fn verifier(pool: &PgPool, blobs: &Memory, haiku: bool, per_tick: i64) -> (Rig, Wire, Eyes) {
    let mut config = vlm_config(haiku);
    config.verify_stored = per_tick;
    let (vlm, _, wire, eyes, _) = bench_on(pool, config, blobs.clone());
    (vlm, wire, eyes)
}

async fn tick(vlm: &Rig) {
    vlm.dispatch(&CancellationToken::new())
        .await
        .map_err(|error| anyhow::anyhow!(error.to_string()))
        .unwrap();
}

async fn kept(
    pool: &PgPool,
    blobs: &Memory,
    id: &str,
    kind: &str,
    key: &str,
    verify: Value,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO track_videos (sc_track_id, kind, source, source_ref, s3_key, offset_ms, loop, duration_ms, verify)
         VALUES ($1, $2, 'youtube', 'aaa', $3, 1500, $2 = 'loop', 90000, $4)",
    )
    .bind(id)
    .bind(kind)
    .bind(key)
    .bind(verify)
    .execute(pool)
    .await?;
    sqlx::query(
        "INSERT INTO track_video_dispatch (sc_track_id, stage, attempts, next_at)
         VALUES ($1, 'found', 2, now() + interval '40 days')
         ON CONFLICT (sc_track_id) DO NOTHING",
    )
    .bind(id)
    .execute(pool)
    .await?;
    blobs.put_at(key, FILE, SystemTime::now());
    Ok(())
}

async fn rows(pool: &PgPool, id: &str) -> anyhow::Result<Vec<(String, String, i32, Value)>> {
    Ok(sqlx::query_as(
        "SELECT kind, s3_key, offset_ms, verify FROM track_videos WHERE sc_track_id = $1 ORDER BY kind",
    )
    .bind(id)
    .fetch_all(pool)
    .await?)
}

async fn days(pool: &PgPool, id: &str) -> anyhow::Result<i64> {
    let row: (i64,) = sqlx::query_as(
        "SELECT round(extract(epoch FROM next_at - now()) / 86400)::bigint
         FROM track_video_dispatch WHERE sc_track_id = $1",
    )
    .bind(id)
    .fetch_one(pool)
    .await?;
    Ok(row.0)
}

fn sent(wire: &Wire) -> Vec<(String, String)> {
    wire.requests()
        .into_iter()
        .map(|(request, id)| (request["video_url"].as_str().unwrap().to_owned(), id))
        .collect()
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn only_unchecked_stored_clips_and_loops_go_and_only_while_haiku_is_open_and_the_queue_empty(
    pool: PgPool,
) -> anyhow::Result<()> {
    let blobs = Memory::default();
    kept(&pool, &blobs, "11", "clip", "video/11-2.mp4", json!({})).await?;
    kept(
        &pool,
        &blobs,
        "12",
        "clip",
        "video/12-2.mp4",
        json!({"vlm": {"label": "clip"}}),
    )
    .await?;
    kept(&pool, &blobs, "13", "loop", "video/13-2.mp4", json!({})).await?;
    kept(
        &pool,
        &blobs,
        "14",
        "generated",
        "video/14-2.mp4",
        json!({}),
    )
    .await?;
    kept(&pool, &blobs, "15", "clip", "video/15-2.mp4", json!({})).await?;
    sqlx::query("UPDATE track_video_dispatch SET stage = 'dispatched' WHERE sc_track_id = '15'")
        .execute(&pool)
        .await?;

    let (local, local_wire, _) = verifier(&pool, &blobs, false, 2);
    tick(&local).await;
    assert!(local_wire.requests().is_empty());

    let (vlm, wire, _) = verifier(&pool, &blobs, true, 2);
    held_track(&pool, &blobs, "16", 1).await?;
    tick(&vlm).await;
    assert_eq!(
        sent(&wire),
        [(
            "https://storage.test/video-tmp/16-1-1.mp4".to_owned(),
            "video_vlm:16:1:1".to_owned()
        )]
    );
    assert_eq!(stage(&pool, "11").await?.0, "found");
    finish(&vlm, answer("16", 1, VideoLabel::Static)).await?;

    tick(&vlm).await;
    let mut checked = sent(&wire)[1..].to_vec();
    checked.sort();
    assert_eq!(
        checked,
        [
            (
                "https://storage.test/video/11-2.mp4".to_owned(),
                "video_vlm:11:2:stored:1".to_owned()
            ),
            (
                "https://storage.test/video/13-2.mp4".to_owned(),
                "video_vlm:13:2:stored:1".to_owned()
            ),
        ]
    );
    for (id, expected) in [("12", "found"), ("14", "found"), ("15", "dispatched")] {
        assert_eq!(stage(&pool, id).await?.0, expected);
    }
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_stored_video_haiku_calls_static_is_deleted_with_its_file_and_waits_like_any_static(
    pool: PgPool,
) -> anyhow::Result<()> {
    let blobs = Memory::default();
    kept(&pool, &blobs, "11", "clip", "video/11-2.mp4", json!({})).await?;
    let (vlm, wire, eyes) = verifier(&pool, &blobs, true, 1);

    tick(&vlm).await;
    eyes.answers(Ok(VideoLabel::Static));
    finish(&vlm, frames_result("11", 2)).await?;

    assert_eq!(wire.requests().len(), 1);
    assert!(rows(&pool, "11").await?.is_empty());
    assert!(blobs.keys().is_empty());
    assert_eq!(
        stage(&pool, "11").await?,
        ("static".to_owned(), 2, Some("static_video".to_owned()))
    );
    assert_eq!(days(&pool, "11").await?, 90);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_stored_video_haiku_calls_clip_stays_untouched_with_the_verdict_recorded(
    pool: PgPool,
) -> anyhow::Result<()> {
    let blobs = Memory::default();
    kept(
        &pool,
        &blobs,
        "11",
        "clip",
        "video/11-2.mp4",
        json!({"ber": 0.03}),
    )
    .await?;
    let (vlm, wire, eyes) = verifier(&pool, &blobs, true, 1);

    tick(&vlm).await;
    eyes.answers(Ok(VideoLabel::Clip));
    finish(&vlm, frames_result("11", 2)).await?;
    tick(&vlm).await;

    assert_eq!(wire.requests().len(), 1);
    assert_eq!(
        rows(&pool, "11").await?,
        [(
            "clip".to_owned(),
            "video/11-2.mp4".to_owned(),
            1500,
            json!({"ber": 0.03, "vlm": {"label": "clip", "model": HAIKU, "scores": {}}})
        )]
    );
    assert_eq!(blobs.get("video/11-2.mp4").as_deref(), Some(FILE));
    assert_eq!(stage(&pool, "11").await?, ("found".to_owned(), 2, None));
    assert_eq!(days(&pool, "11").await?, 40);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn when_haiku_gives_no_label_the_local_model_finishes_the_stored_check(
    pool: PgPool,
) -> anyhow::Result<()> {
    let blobs = Memory::default();
    kept(&pool, &blobs, "11", "clip", "video/11-2.mp4", json!({})).await?;
    let (vlm, wire, _) = verifier(&pool, &blobs, true, 1);

    tick(&vlm).await;
    finish(&vlm, frames_result("11", 2)).await?;
    let (request, id) = wire.requests().pop().unwrap();
    assert!(request["prompt"].is_string());
    assert_eq!(id, "video_vlm:11:2:stored:2");
    finish(&vlm, answer("11", 2, VideoLabel::Loop)).await?;

    let row = rows(&pool, "11").await?.remove(0);
    assert_eq!(row.3["vlm"]["label"], "clip");
    assert_eq!(row.3["vlm"]["model"], "Qwen3VL-2B-Instruct-Q4_K_M");
    assert_eq!(wire.requests().len(), 2);
    assert_eq!(stage(&pool, "11").await?.0, "found");
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_restart_in_the_middle_of_the_recheck_loses_nothing(pool: PgPool) -> anyhow::Result<()> {
    let blobs = Memory::default();
    for id in ["11", "12", "13"] {
        kept(
            &pool,
            &blobs,
            id,
            "clip",
            &format!("video/{id}-2.mp4"),
            json!({}),
        )
        .await?;
    }
    let (before, _, _) = verifier(&pool, &blobs, true, 2);
    tick(&before).await;
    assert_eq!(stage(&pool, "11").await?.0, "vlm_dispatched");
    assert_eq!(stage(&pool, "13").await?.0, "found");
    drop(before);
    sqlx::query("UPDATE track_video_dispatch SET updated_at = now() - interval '2 hours' WHERE stage = 'vlm_dispatched'")
        .execute(&pool)
        .await?;

    let (vlm, wire, eyes) = verifier(&pool, &blobs, true, 2);
    tick(&vlm).await;
    let mut ids: Vec<String> = wire.requests().into_iter().map(|(_, id)| id).collect();
    ids.sort();
    assert_eq!(ids, ["video_vlm:11:2:stored:2", "video_vlm:12:2:stored:2"]);
    eyes.answers(Ok(VideoLabel::Static));
    finish(&vlm, frames_result("11", 2)).await?;
    eyes.answers(Ok(VideoLabel::Clip));
    finish(&vlm, frames_result("12", 2)).await?;
    tick(&vlm).await;
    eyes.answers(Ok(VideoLabel::Clip));
    finish(&vlm, frames_result("13", 2)).await?;

    assert!(rows(&pool, "11").await?.is_empty());
    for id in ["12", "13"] {
        assert_eq!(rows(&pool, id).await?[0].3["vlm"]["label"], "clip");
        assert_eq!(stage(&pool, id).await?.0, "found");
    }
    assert_eq!(blobs.keys(), ["video/12-2.mp4", "video/13-2.mp4"]);
    let left: (i64,) = sqlx::query_as(
        "SELECT count(*) FROM track_video_dispatch WHERE stage IN ('pending_vlm', 'vlm_dispatched')",
    )
    .fetch_one(&pool)
    .await?;
    assert_eq!(left.0, 0);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_failed_stored_check_keeps_the_video_unsent_and_a_gone_file_drops_its_row(
    pool: PgPool,
) -> anyhow::Result<()> {
    let blobs = Memory::default();
    kept(&pool, &blobs, "11", "clip", "video/11-2.mp4", json!({})).await?;
    kept(&pool, &blobs, "12", "clip", "video/12-2.mp4", json!({})).await?;
    let (vlm, wire, _) = verifier(&pool, &blobs, true, 2);

    tick(&vlm).await;
    finish(&vlm, failure("11", 2, WorkerReason::UndecodableAudio)).await?;
    finish(&vlm, failure("12", 2, WorkerReason::InputsGone)).await?;
    tick(&vlm).await;

    assert_eq!(wire.requests().len(), 2);
    assert_eq!(
        rows(&pool, "11").await?[0].3,
        json!({"vlm": {"failed": "undecodable_audio"}})
    );
    assert_eq!(stage(&pool, "11").await?, ("found".to_owned(), 2, None));
    assert_eq!(blobs.get("video/11-2.mp4").as_deref(), Some(FILE));
    assert!(rows(&pool, "12").await?.is_empty());
    assert_eq!(stage(&pool, "12").await?.0, "retry");
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_static_clip_next_to_a_loop_leaves_the_loop_and_the_loop_is_checked_next(
    pool: PgPool,
) -> anyhow::Result<()> {
    let blobs = Memory::default();
    kept(&pool, &blobs, "11", "clip", "video/11-2.mp4", json!({})).await?;
    kept(&pool, &blobs, "11", "loop", "video/11-1.mp4", json!({})).await?;
    let (vlm, wire, eyes) = verifier(&pool, &blobs, true, 1);

    tick(&vlm).await;
    eyes.answers(Ok(VideoLabel::Static));
    finish(&vlm, frames_result("11", 2)).await?;
    assert_eq!(stage(&pool, "11").await?.0, "found");
    tick(&vlm).await;
    eyes.answers(Ok(VideoLabel::Clip));
    finish(&vlm, frames_result("11", 2)).await?;

    assert_eq!(
        sent(&wire)
            .into_iter()
            .map(|(url, _)| url)
            .collect::<Vec<_>>(),
        [
            "https://storage.test/video/11-2.mp4",
            "https://storage.test/video/11-1.mp4"
        ]
    );
    let left = rows(&pool, "11").await?;
    assert_eq!((left.len(), left[0].0.as_str()), (1, "loop"));
    assert_eq!(left[0].3["vlm"]["label"], "clip");
    assert_eq!(blobs.keys(), ["video/11-1.mp4"]);
    Ok(())
}
