use std::sync::atomic::Ordering;

use backend_contracts::reasons::{WorkerReason, WorkerStatus};
use backend_contracts::worker_contract::WorkerLane;
use chrono::Utc;
use serde_json::Value;
use sqlx::PgPool;

use super::super::LADDER;
use super::super::director::{Look, Verdict};
use super::{
    Bench, PLAN_USD, REVIEW_USD, Scripted, TRACK, Wire, bench, config, due_now, failed, lyrics,
    plan, queued, reply, row, star, superlike, track,
};
use crate::config::VideoGenConfig;
use crate::llm::anthropic::LlmError;
use crate::llm::ledger;

pub(super) fn accept(trim_start_ms: i32) -> Verdict {
    Verdict::Accept {
        trim_start_ms,
        note: "the lock holds".to_owned(),
    }
}

pub(super) async fn waiting(pool: &PgPool) {
    star(pool, "11").await;
    track(pool, TRACK, "90").await;
    superlike(pool, "11", TRACK).await;
    lyrics(pool, TRACK).await;
}

pub(super) fn redo(edit: &str) -> Verdict {
    Verdict::Redo {
        reason: "the face changed".to_owned(),
        edit: edit.to_owned(),
    }
}

pub(super) async fn keyframe(bench: &Bench, wire: &Wire) {
    reply(
        bench,
        wire,
        WorkerLane::VideoKeyframe,
        &[("frame.png", b"frame-png")],
        512,
    )
    .await;
}

pub(super) async fn shot(bench: &Bench, wire: &Wire) {
    let files: [(&str, &[u8]); 3] = [
        ("shot.mp4", b"shot-mp4-bytes"),
        ("still0.jpg", b"still-a"),
        ("still1.jpg", b"still-b"),
    ];
    reply(bench, wire, WorkerLane::VideoShot, &files, 5).await;
}

async fn through_shots(bench: &Bench, director: &Scripted, wire: &Wire, trims: [i32; 2]) {
    director.plans(Ok(plan()));
    bench.dispatch().await.unwrap();
    for trim in trims {
        director.verdicts(Ok(accept(0)));
        director.verdicts(Ok(accept(trim)));
        keyframe(bench, wire).await;
        bench.dispatch().await.unwrap();
        shot(bench, wire).await;
        bench.dispatch().await.unwrap();
    }
}

pub(super) fn payload(wire: &Wire) -> Value {
    wire.last().1
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn accepted_shots_become_a_generated_video(pool: PgPool) {
    waiting(&pool).await;
    let (bench, director, store, wire, inbox) = bench(&pool, config());

    through_shots(&bench, &director, &wire, [500, 99_999]).await;

    assert_eq!(
        wire.subjects(),
        [
            "video.keyframe.new",
            "video.shot.new",
            "video.keyframe.new",
            "video.shot.new",
            "video.assemble.new"
        ]
    );
    let brief = director.briefs.lock().unwrap()[0].clone();
    assert_eq!(
        brief.cover_url,
        "https://i1.sndcdn.com/artworks-501-t500x500.jpg"
    );
    assert!(brief.lyrics.unwrap().contains("first line"));
    assert_eq!(
        *director.reviews.lock().unwrap(),
        [
            (Look::Keyframe, 0, 1, 0),
            (Look::Shot, 0, 2, 1),
            (Look::Keyframe, 1, 1, 1),
            (Look::Shot, 1, 2, 2)
        ]
    );
    let assemble = payload(&wire);
    assert_eq!(assemble["audio_offset_ms"], 30_000);
    assert_eq!(assemble["shots"][0]["trim_start_ms"], 500);
    assert_eq!(assemble["shots"][1]["trim_start_ms"], 562);
    assert_eq!(assemble["shots"][1]["duration_ms"], 4_500);
    assert!(
        assemble["shots"][0]["url"]
            .as_str()
            .unwrap()
            .starts_with("https://storage.example/video-tmp/gen/501-1-")
    );
    assert!(
        assemble["audio_url"]
            .as_str()
            .unwrap()
            .starts_with("https://storage.example/")
    );
    assert!(
        store
            .keys()
            .iter()
            .all(|key| key.ends_with("shot.mp4") || key.ends_with("frame.png"))
    );

    reply(
        &bench,
        &wire,
        WorkerLane::VideoAssemble,
        &[("video.mp4", b"final-video")],
        512,
    )
    .await;

    assert_eq!(row(&pool, TRACK).await.0, "done");
    assert_eq!(store.keys(), ["video/501-gen-1.mp4"]);
    let video: (String, String, i32, i32, bool) = sqlx::query_as(
        "SELECT kind, s3_key, offset_ms, duration_ms, loop FROM track_videos WHERE sc_track_id = $1",
    )
    .bind(TRACK)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        video,
        (
            "generated".to_owned(),
            "video/501-gen-1.mp4".to_owned(),
            30_000,
            8_500,
            true
        )
    );
    let spent = ledger::spent(&pool, LADDER, ledger::period(Utc::now(), 17))
        .await
        .unwrap();
    assert!((spent - (PLAN_USD + 4.0 * REVIEW_USD)).abs() < 1e-9);
    assert_eq!(inbox.count(), 0);
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_keyframe_redone_too_often_without_re_plans_rejects_the_video_and_keeps_its_media(
    pool: PgPool,
) {
    waiting(&pool).await;
    let (bench, director, store, wire, _) = bench(
        &pool,
        VideoGenConfig {
            shot_replans: 0,
            ..config()
        },
    );
    director.plans(Ok(plan()));
    bench.dispatch().await.unwrap();

    for edit in ["fix the face", "keep the coat", "same hair"] {
        director.verdicts(Ok(redo(edit)));
        keyframe(&bench, &wire).await;
        bench.dispatch().await.unwrap();
    }

    assert_eq!(wire.subjects().len(), 3);
    assert!(
        wire.sent.lock().unwrap()[1].1["prompt"]
            .as_str()
            .unwrap()
            .contains("fix the face")
    );
    let (stage, reason, rejections, _) = row(&pool, TRACK).await;
    assert_eq!(stage, "rejected");
    assert!(
        reason
            .unwrap()
            .contains("keyframe 1 failed after 0 re-plans")
    );
    assert_eq!(rejections.as_array().unwrap().len(), 3);
    assert_eq!(rejections[2]["edit"], "same hair");
    assert_eq!(
        rejections[0]["media"][0],
        "video-tmp/gen-rejected/501-1-1-frame.png"
    );
    assert_eq!(store.keys().len(), 3);
    assert!(
        store
            .keys()
            .iter()
            .all(|key| key.starts_with("video-tmp/gen-rejected/501-1-"))
    );
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_rejected_shot_is_drawn_again_from_the_same_keyframe(pool: PgPool) {
    waiting(&pool).await;
    let (bench, director, _, wire, _) = bench(&pool, config());
    director.plans(Ok(plan()));
    director.verdicts(Ok(accept(0)));
    director.verdicts(Ok(redo("slower")));
    bench.dispatch().await.unwrap();
    keyframe(&bench, &wire).await;
    bench.dispatch().await.unwrap();
    let first = payload(&wire);
    shot(&bench, &wire).await;
    bench.dispatch().await.unwrap();

    let again = payload(&wire);
    assert_eq!(wire.last().0, "video.shot.new");
    assert_eq!(again["frame_url"], first["frame_url"]);
    assert!(again["prompt"].as_str().unwrap().contains("slower"));
    assert_ne!(again["seed"], first["seed"]);
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_gone_keyframe_draws_it_again(pool: PgPool) {
    waiting(&pool).await;
    let (bench, director, _, wire, _) = bench(&pool, config());
    director.plans(Ok(plan()));
    director.verdicts(Ok(accept(0)));
    bench.dispatch().await.unwrap();
    keyframe(&bench, &wire).await;
    bench.dispatch().await.unwrap();

    let request = payload(&wire);
    bench
        .finish(
            WorkerLane::VideoShot,
            failed(
                &request,
                WorkerStatus::Failed,
                Some(WorkerReason::InputsGone),
            ),
        )
        .await
        .unwrap();
    bench.dispatch().await.unwrap();

    assert_eq!(
        wire.subjects(),
        ["video.keyframe.new", "video.shot.new", "video.keyframe.new"]
    );
    assert_eq!(row(&pool, TRACK).await.0, "keyframe");
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn gone_shots_at_assembly_start_again_from_the_first_shot(pool: PgPool) {
    waiting(&pool).await;
    let (bench, director, _, wire, _) = bench(&pool, config());
    through_shots(&bench, &director, &wire, [0, 0]).await;

    let request = payload(&wire);
    bench
        .finish(
            WorkerLane::VideoAssemble,
            failed(
                &request,
                WorkerStatus::Failed,
                Some(WorkerReason::InputsGone),
            ),
        )
        .await
        .unwrap();
    bench.dispatch().await.unwrap();

    assert_eq!(wire.last().0, "video.keyframe.new");
    assert!(
        payload(&wire)["prompt"]
            .as_str()
            .unwrap()
            .starts_with("she stands under a street lamp")
    );
    let shot: i32 = sqlx::query_scalar("SELECT shot FROM track_video_gen WHERE sc_track_id = $1")
        .bind(TRACK)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(shot, 0);
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_gone_cover_fails_the_video(pool: PgPool) {
    waiting(&pool).await;
    let (bench, director, _, wire, _) = bench(&pool, config());
    director.plans(Ok(plan()));
    bench.dispatch().await.unwrap();

    let request = payload(&wire);
    bench
        .finish(
            WorkerLane::VideoKeyframe,
            failed(
                &request,
                WorkerStatus::Failed,
                Some(WorkerReason::InputsGone),
            ),
        )
        .await
        .unwrap();

    let (stage, reason, _, _) = row(&pool, TRACK).await;
    assert_eq!(stage, "failed");
    assert_eq!(reason.as_deref(), Some("the cover is gone"));
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_stale_or_repeated_answer_is_ignored(pool: PgPool) {
    waiting(&pool).await;
    let (bench, director, store, wire, _) = bench(&pool, config());
    director.plans(Ok(plan()));
    director.verdicts(Ok(accept(0)));
    bench.dispatch().await.unwrap();
    let request = payload(&wire);
    keyframe(&bench, &wire).await;
    let frames = store.keys();
    for result in super::answered(&request, &[("frame.png", b"other")], 512) {
        bench
            .finish(WorkerLane::VideoKeyframe, result)
            .await
            .unwrap();
    }
    bench.dispatch().await.unwrap();
    for result in super::answered(&request, &[("frame.png", b"late")], 512) {
        bench
            .finish(WorkerLane::VideoKeyframe, result)
            .await
            .unwrap();
    }

    assert_eq!(store.keys(), frames);
    assert_eq!(row(&pool, TRACK).await.0, "shot");
    assert_eq!(wire.subjects(), ["video.keyframe.new", "video.shot.new"]);
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_failed_publish_is_sent_again_later(pool: PgPool) {
    waiting(&pool).await;
    let (bench, director, _, wire, _) = bench(&pool, config());
    director.plans(Ok(plan()));
    wire.down.store(true, Ordering::SeqCst);

    bench.dispatch().await.unwrap();

    let sent: Option<chrono::DateTime<Utc>> =
        sqlx::query_scalar("SELECT sent_at FROM track_video_gen WHERE sc_track_id = $1")
            .bind(TRACK)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(sent.is_none());
    assert_eq!(row(&pool, TRACK).await.0, "keyframe");

    wire.down.store(false, Ordering::SeqCst);
    due_now(&pool).await;
    bench.dispatch().await.unwrap();

    assert_eq!(wire.subjects(), ["video.keyframe.new"]);
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_bad_plan_is_retried_and_then_fails(pool: PgPool) {
    waiting(&pool).await;
    let (bench, director, _, wire, _) = bench(&pool, config());
    let mut short = plan();
    short.shots.truncate(1);
    director.plans(Ok(short.clone()));
    director.plans(Ok(short));
    director.plans(Err(LlmError::Invalid("no tool call".to_owned())));
    let mut gap = plan();
    gap.shots[1].start_ms = 35_000;
    director.plans(Ok(gap.clone()));
    director.plans(Ok(gap));

    for _ in 0..3 {
        bench.dispatch().await.unwrap();
        due_now(&pool).await;
    }

    let (stage, reason, rejections, faults) = row(&pool, TRACK).await;
    assert_eq!(stage, "failed");
    assert!(
        reason
            .unwrap()
            .contains("does not start where the last one ended")
    );
    assert_eq!(rejections.as_array().unwrap().len(), 5);
    assert_eq!(faults, 3);
    assert!(wire.sent.lock().unwrap().is_empty());
}

async fn sent_at(pool: &PgPool) -> Option<chrono::DateTime<Utc>> {
    sqlx::query_scalar("SELECT sent_at FROM track_video_gen WHERE sc_track_id = $1")
        .bind(TRACK)
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn sent_long_ago(pool: &PgPool) {
    sqlx::query("UPDATE track_video_gen SET sent_at = now() - interval '5 hours'")
        .execute(pool)
        .await
        .unwrap();
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn without_a_generating_node_nothing_is_planned_or_paid_for(pool: PgPool) {
    waiting(&pool).await;
    let (bench, director, _, wire, _) = bench(&pool, config());
    director.plans(Ok(plan()));
    wire.absent
        .lock()
        .unwrap()
        .extend([WorkerLane::VideoKeyframe, WorkerLane::VideoShot]);

    for _ in 0..3 {
        bench.dispatch().await.unwrap();
        due_now(&pool).await;
    }

    assert!(queued(&pool).await.is_empty());
    assert!(director.briefs.lock().unwrap().is_empty());
    assert!(wire.sent.lock().unwrap().is_empty());

    wire.absent.lock().unwrap().clear();
    bench.dispatch().await.unwrap();

    assert_eq!(director.briefs.lock().unwrap().len(), 1);
    assert_eq!(wire.subjects(), ["video.keyframe.new"]);
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_sent_step_waits_for_its_node_without_faults_or_resends(pool: PgPool) {
    waiting(&pool).await;
    let (bench, director, _, wire, _) = bench(&pool, config());
    director.plans(Ok(plan()));
    bench.dispatch().await.unwrap();
    assert_eq!(wire.subjects(), ["video.keyframe.new"]);
    wire.absent.lock().unwrap().push(WorkerLane::VideoKeyframe);

    for _ in 0..3 {
        sent_long_ago(&pool).await;
        due_now(&pool).await;
        bench.dispatch().await.unwrap();
    }

    let (stage, _, _, faults) = row(&pool, TRACK).await;
    assert_eq!((stage.as_str(), faults), ("keyframe", 0));
    assert!(
        sent_at(&pool)
            .await
            .is_some_and(|at| Utc::now() - at < chrono::Duration::minutes(1))
    );
    assert_eq!(wire.subjects(), ["video.keyframe.new"]);

    wire.absent.lock().unwrap().clear();
    bench.dispatch().await.unwrap();

    assert_eq!(row(&pool, TRACK).await.3, 0);
    assert_eq!(wire.subjects(), ["video.keyframe.new"]);
    keyframe(&bench, &wire).await;
    assert_eq!(row(&pool, TRACK).await.0, "keyframe_review");
}
