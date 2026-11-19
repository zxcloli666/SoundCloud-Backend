use std::collections::BTreeMap;

use backend_contracts::pipeline::{VideoKind, VideoLabel, VideoNext};
use backend_contracts::reasons::WorkerReason;
use serde_json::{Value, json};
use sqlx::PgPool;
use tokio_util::sync::CancellationToken;

use super::fakes::HAIKU;
use super::haiku::{cut_part, frames_result};
use super::{Rig, answer, bench_on, finish, stage, stored, vlm_config};
use crate::handlers::video_find::tests::research::{research, videos, wait_days, with_video};
use crate::handlers::video_find::tests::{self as find, Memory};

const CUT: &[u8] = b"the padded cut from the search";

async fn tick(vlm: &Rig) {
    vlm.dispatch(&CancellationToken::new())
        .await
        .map_err(|error| anyhow::anyhow!(error.to_string()))
        .unwrap();
}

async fn found_clip(pool: &PgPool, id: &str) -> anyhow::Result<(find::Rig, Memory)> {
    find::star_track(pool, id, 1, 1).await?;
    let (finder, _, blobs, _) = find::rig(pool);
    find::staged_claim(pool, &blobs, id, 2).await?;
    find::finish(&finder, &find::part_message(id, 1, CUT, &[CUT], 0)).await?;
    Ok((finder, blobs))
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_confident_clip_is_stored_from_its_search_cut_only_after_haiku_says_clip(
    pool: PgPool,
) -> anyhow::Result<()> {
    let (_, blobs) = found_clip(&pool, "11").await?;
    assert_eq!(stored(&pool, "11").await?, None);
    assert_eq!(stage(&pool, "11").await?.0, "pending_vlm");
    let (vlm, _, wire, eyes, _) = bench_on(&pool, vlm_config(true), blobs.clone());

    tick(&vlm).await;
    let (request, id) = wire.requests().pop().unwrap();
    assert_eq!(
        request["video_url"],
        "https://storage.test/video-tmp/11-1-cut.mp4"
    );
    assert_eq!(
        (request["frames"].clone(), id.as_str()),
        (json!(true), "video_vlm:11:1:1")
    );
    eyes.answers(Ok(VideoLabel::Clip));
    finish(&vlm, frames_result("11", 1)).await?;

    assert_eq!(wire.requests().len(), 1);
    let row = stored(&pool, "11").await?.unwrap();
    assert_eq!(
        (row.0.as_str(), row.1.as_str(), row.2, row.3, row.4),
        ("clip", "video/11-1.mp4", 3_839, false, 96_000)
    );
    assert_eq!(row.5["ber"], f64::from(0.02_f32));
    assert_eq!(
        row.5["vlm"],
        json!({"label": "clip", "model": HAIKU, "scores": {}})
    );
    assert_eq!(blobs.keys(), ["video/11-1.mp4"]);
    assert_eq!(blobs.get("video/11-1.mp4").as_deref(), Some(CUT));
    assert_eq!(wait_days(&pool, "11").await?, ("found".to_owned(), 90));
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_confident_clip_haiku_calls_static_is_not_stored_and_its_cut_goes(
    pool: PgPool,
) -> anyhow::Result<()> {
    let (_, blobs) = found_clip(&pool, "11").await?;
    let (vlm, _, wire, eyes, _) = bench_on(&pool, vlm_config(true), blobs.clone());

    tick(&vlm).await;
    eyes.answers(Ok(VideoLabel::Static));
    finish(&vlm, frames_result("11", 1)).await?;

    assert_eq!(wire.requests().len(), 1);
    assert_eq!(stored(&pool, "11").await?, None);
    assert!(blobs.keys().is_empty());
    assert_eq!(
        stage(&pool, "11").await?,
        ("static".to_owned(), 1, Some("static_video".to_owned()))
    );
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn without_haiku_the_local_model_judges_the_search_cut_and_no_second_cut_is_asked(
    pool: PgPool,
) -> anyhow::Result<()> {
    let (_, blobs) = found_clip(&pool, "11").await?;
    let (vlm, _, wire, _, _) = bench_on(&pool, vlm_config(false), blobs.clone());

    tick(&vlm).await;
    let (request, _) = wire.requests().pop().unwrap();
    assert!(request["prompt"].is_string());
    assert!(request.get("label").is_none());
    finish(&vlm, answer("11", 1, VideoLabel::Clip)).await?;

    assert_eq!(wire.requests().len(), 1);
    let row = stored(&pool, "11").await?.unwrap();
    assert_eq!((row.0.as_str(), row.1.as_str()), ("clip", "video/11-1.mp4"));
    assert_eq!(row.5["vlm"]["model"], "Qwen3VL-2B-Instruct-Q4_K_M");
    assert_eq!(blobs.keys(), ["video/11-1.mp4"]);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_static_from_the_rule_prefilter_never_reaches_the_vlm(
    pool: PgPool,
) -> anyhow::Result<()> {
    find::star_track(&pool, "11", 1, 1).await?;
    let (finder, _, blobs, _) = find::rig(&pool);
    find::staged_claim(&pool, &blobs, "11", 2).await?;
    let (vlm, _, wire, eyes, _) = bench_on(&pool, vlm_config(true), blobs.clone());

    find::finish(
        &finder,
        &find::done("11", 1, Some(WorkerReason::StaticVideo)),
    )
    .await?;
    tick(&vlm).await;

    assert_eq!(stage(&pool, "11").await?.0, "static");
    assert!(wire.requests().is_empty());
    assert!(eyes.seen.lock().unwrap().is_empty());
    assert!(blobs.keys().is_empty());
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_better_clip_on_a_research_replaces_the_stored_one_only_after_the_vlm(
    pool: PgPool,
) -> anyhow::Result<()> {
    find::account(&pool, "yt1", "youtube").await?;
    find::star_track(&pool, "11", 1, 1).await?;
    let (finder, _, blobs, _) = find::rig(&pool);
    with_video(&pool, &blobs, "11", "clip").await?;
    research(&pool, &finder, "11").await?;
    let (vlm, _, _, eyes, _) = bench_on(&pool, vlm_config(true), blobs.clone());

    find::finish(&finder, &find::part_message("11", 2, CUT, &[CUT], 0)).await?;
    assert_eq!(videos(&pool, "11").await?[0].2, "video/11-1.mp4");
    assert_eq!(blobs.keys(), ["video-tmp/11-2-cut.mp4", "video/11-1.mp4"]);

    tick(&vlm).await;
    eyes.answers(Ok(VideoLabel::Clip));
    finish(&vlm, frames_result("11", 2)).await?;

    assert_eq!(
        videos(&pool, "11").await?,
        [(
            "clip".to_owned(),
            Some("bbb".to_owned()),
            "video/11-2.mp4".to_owned()
        )]
    );
    assert_eq!(blobs.keys(), ["video/11-2.mp4"]);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn the_search_takes_no_track_while_the_vlm_backlog_is_full(
    pool: PgPool,
) -> anyhow::Result<()> {
    let (finder, _, _, _) = find::rig(&pool);
    sqlx::query(
        "INSERT INTO track_video_dispatch (sc_track_id, stage, attempts, payload)
         SELECT n::text, CASE WHEN n % 2 = 0 THEN 'pending_vlm' ELSE 'vlm_dispatched' END, 1, '{}'
         FROM generate_series(100, 123) AS n",
    )
    .execute(&pool)
    .await?;
    let free = || async {
        finder
            .free()
            .await
            .map_err(|error| anyhow::anyhow!(error.to_string()))
    };

    assert_eq!(free().await?, 0);
    sqlx::query("DELETE FROM track_video_dispatch WHERE sc_track_id = '100'")
        .execute(&pool)
        .await?;
    assert_eq!(free().await?, 2);
    Ok(())
}

fn runner_up(candidate: u32) -> VideoNext {
    VideoNext {
        candidate,
        kind: VideoKind::Clip,
        offset_ms: 1_000,
        pad_ms: 2_000,
        duration_ms: 120_000,
        looped: false,
        scores: BTreeMap::from([("ber".to_owned(), 0.05)]),
    }
}

async fn found_with_runners_up(
    pool: &PgPool,
    staged: u32,
    next: &[u32],
) -> anyhow::Result<(Rig, Memory, super::fakes::Wire, super::fakes::Eyes)> {
    find::star_track(pool, "11", 1, 1).await?;
    let (finder, _, blobs, _) = find::rig(pool);
    find::staged_claim(pool, &blobs, "11", staged).await?;
    let mut result = find::part_message("11", 1, CUT, &[CUT], 0);
    result.next = Some(next.iter().copied().map(runner_up).collect());
    find::finish(&finder, &result).await?;
    let (vlm, _, wire, eyes, _) = bench_on(pool, vlm_config(true), blobs.clone());
    Ok((vlm, blobs, wire, eyes))
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn when_the_first_candidate_is_static_the_next_one_is_checked_at_once_and_stored(
    pool: PgPool,
) -> anyhow::Result<()> {
    let (vlm, blobs, wire, eyes) = found_with_runners_up(&pool, 2, &[0]).await?;
    assert_eq!(
        blobs.keys(),
        ["video-tmp/11-1-0.mp4", "video-tmp/11-1-cut.mp4"]
    );

    tick(&vlm).await;
    eyes.answers(Ok(VideoLabel::Static));
    finish(&vlm, frames_result("11", 1)).await?;
    assert_eq!(stage(&pool, "11").await?.0, "pending_vlm");
    assert_eq!(blobs.keys(), ["video-tmp/11-1-0.mp4"]);

    tick(&vlm).await;
    let (request, id) = wire.requests().pop().unwrap();
    assert_eq!(
        request["video_url"],
        "https://storage.test/video-tmp/11-1-0.mp4"
    );
    assert_eq!(
        (request["frames"].clone(), id.as_str()),
        (json!(true), "video_vlm:11:1:2")
    );
    eyes.answers(Ok(VideoLabel::Clip));
    finish(&vlm, frames_result("11", 1)).await?;
    let (request, id) = wire.requests().pop().unwrap();
    assert_eq!(
        (
            request["label"].clone(),
            request["pad_ms"].clone(),
            request["offset_ms"].clone()
        ),
        (json!("clip"), json!(2_000), json!(1_000))
    );
    assert_eq!(id, "video_vlm:11:1:3");
    finish(&vlm, cut_part(VideoLabel::Clip, "11", b"the second cut")).await?;

    let row = stored(&pool, "11").await?.unwrap();
    assert_eq!(
        (row.0.as_str(), row.1.as_str(), row.2, row.3),
        ("clip", "video/11-1.mp4", 1_000, false)
    );
    assert_eq!(row.5["ber"], f64::from(0.05_f32));
    assert_eq!(row.5["vlm"]["model"], HAIKU);
    assert_eq!(blobs.keys(), ["video/11-1.mp4"]);
    assert_eq!(
        blobs.get("video/11-1.mp4").as_deref(),
        Some(&b"the second cut"[..])
    );
    assert_eq!(stage(&pool, "11").await?.0, "found");
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn at_most_three_candidates_go_to_the_vlm_and_all_static_settles_static(
    pool: PgPool,
) -> anyhow::Result<()> {
    let (vlm, blobs, wire, eyes) = found_with_runners_up(&pool, 4, &[0, 2, 3]).await?;
    assert_eq!(
        blobs.keys(),
        [
            "video-tmp/11-1-0.mp4",
            "video-tmp/11-1-2.mp4",
            "video-tmp/11-1-cut.mp4"
        ]
    );

    for _ in 0..3 {
        tick(&vlm).await;
        eyes.answers(Ok(VideoLabel::Static));
        finish(&vlm, frames_result("11", 1)).await?;
    }
    tick(&vlm).await;

    let urls: Vec<Value> = wire
        .requests()
        .into_iter()
        .map(|(request, _)| request["video_url"].clone())
        .collect();
    assert_eq!(
        urls,
        [
            "https://storage.test/video-tmp/11-1-cut.mp4",
            "https://storage.test/video-tmp/11-1-0.mp4",
            "https://storage.test/video-tmp/11-1-2.mp4"
        ]
    );
    assert_eq!(stored(&pool, "11").await?, None);
    assert!(blobs.keys().is_empty());
    assert_eq!(
        stage(&pool, "11").await?,
        ("static".to_owned(), 1, Some("static_video".to_owned()))
    );
    assert_eq!(wait_days(&pool, "11").await?, ("static".to_owned(), 90));
    Ok(())
}
