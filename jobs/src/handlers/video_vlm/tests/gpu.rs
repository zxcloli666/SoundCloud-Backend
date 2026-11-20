use std::sync::atomic::Ordering;

use backend_contracts::pipeline::{VIDEO_VLM, VIDEO_VLM_GPU, VideoLabel, VideoVlmResult};
use backend_contracts::reasons::WorkerReason;
use backend_contracts::worker_contract::WorkerLane;
use serde_json::json;
use sqlx::PgPool;
use tokio_util::sync::CancellationToken;

use super::super::Judge;
use super::super::result::{Verdict, judge};
use super::{Rig, answer, bench, failure, held_track, stage, tries};

async fn tick(vlm: &Rig) {
    vlm.dispatch(&CancellationToken::new())
        .await
        .map_err(|error| anyhow::anyhow!(error.to_string()))
        .unwrap();
}

async fn from_gpu(vlm: &Rig, result: VideoVlmResult) {
    vlm.finish(WorkerLane::VideoVlmGpu, result)
        .await
        .map_err(|error| anyhow::anyhow!(error.to_string()))
        .unwrap();
}

fn path(vlm: &Rig) -> Option<Judge> {
    *vlm.path.lock().unwrap()
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_seen_gpu_node_gets_the_check_with_the_prompt_before_haiku(pool: PgPool) {
    let (vlm, blobs, wire, eyes, _) = bench(&pool, true);
    wire.gpu.store(true, Ordering::SeqCst);
    held_track(&pool, &blobs, "11", 1).await.unwrap();

    tick(&vlm).await;

    assert_eq!(wire.subjects(), [VIDEO_VLM_GPU]);
    let (request, id) = wire.requests().pop().unwrap();
    assert!(request.get("prompt").is_some());
    assert!(request.get("frames").is_none() && request.get("label").is_none());
    assert_eq!(id, "video_vlm:11:1:1");
    assert!(eyes.seen.lock().unwrap().is_empty());
    assert_eq!(path(&vlm), Some(Judge::Gpu));
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_gpu_clip_answer_asks_worker1_for_the_cut(pool: PgPool) {
    let (vlm, blobs, wire, _, _) = bench(&pool, false);
    wire.gpu.store(true, Ordering::SeqCst);
    held_track(&pool, &blobs, "11", 1).await.unwrap();
    tick(&vlm).await;

    from_gpu(&vlm, answer("11", 1, VideoLabel::Clip)).await;

    assert_eq!(wire.subjects(), [VIDEO_VLM_GPU, VIDEO_VLM]);
    let (request, id) = wire.requests().pop().unwrap();
    assert_eq!(request["label"], json!("loop"));
    assert!(request.get("prompt").is_none());
    assert_eq!(id, "video_vlm:11:1:2");
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn without_a_gpu_node_the_check_goes_to_haiku_and_then_to_the_cpu_model(pool: PgPool) {
    let (vlm, blobs, wire, _, _) = bench(&pool, true);
    held_track(&pool, &blobs, "11", 1).await.unwrap();
    tick(&vlm).await;
    assert_eq!(wire.requests().pop().unwrap().0["frames"], json!(true));
    assert_eq!(path(&vlm), Some(Judge::Haiku));

    let (vlm, blobs, wire, _, _) = bench(&pool, false);
    held_track(&pool, &blobs, "12", 1).await.unwrap();
    tick(&vlm).await;

    assert_eq!(wire.subjects(), [VIDEO_VLM]);
    assert!(wire.requests().pop().unwrap().0.get("prompt").is_some());
    assert_eq!(path(&vlm), Some(Judge::Cpu));
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_gpu_node_that_goes_away_moves_the_next_check_to_the_cpu_model(pool: PgPool) {
    let (vlm, blobs, wire, _, _) = bench(&pool, false);
    wire.gpu.store(true, Ordering::SeqCst);
    held_track(&pool, &blobs, "11", 1).await.unwrap();
    tick(&vlm).await;

    wire.gpu.store(false, Ordering::SeqCst);
    held_track(&pool, &blobs, "12", 1).await.unwrap();
    tick(&vlm).await;

    assert_eq!(wire.subjects(), [VIDEO_VLM_GPU, VIDEO_VLM]);
    assert_eq!(path(&vlm), Some(Judge::Cpu));
}

async fn backdate(pool: &PgPool, id: &str, secs: i64) {
    sqlx::query(
        "UPDATE track_video_dispatch SET updated_at = now() - make_interval(secs => $2)
         WHERE sc_track_id = $1",
    )
    .bind(id)
    .bind(secs as f64)
    .execute(pool)
    .await
    .unwrap();
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_public_node_timeout_moves_the_check_to_haiku_without_a_try(pool: PgPool) {
    let (vlm, blobs, wire, _, _) = bench(&pool, true);
    wire.gpu.store(true, Ordering::SeqCst);
    held_track(&pool, &blobs, "11", 1).await.unwrap();
    tick(&vlm).await;

    from_gpu(&vlm, failure("11", 1, WorkerReason::PublicNodeTimeout)).await;

    let (stage_now, _, why) = stage(&pool, "11").await.unwrap();
    assert_eq!(stage_now, "pending_vlm");
    assert_eq!(why.as_deref(), Some("public_node_timeout"));
    assert_eq!(tries(&pool, "11").await.unwrap(), None);
    tick(&vlm).await;
    assert_eq!(wire.subjects(), [VIDEO_VLM_GPU, VIDEO_VLM]);
    assert_eq!(wire.requests().pop().unwrap().0["frames"], json!(true));
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn an_unanswered_gpu_check_goes_to_the_cpu_model_after_the_gpu_timeout(pool: PgPool) {
    let (vlm, blobs, wire, _, _) = bench(&pool, false);
    wire.gpu.store(true, Ordering::SeqCst);
    held_track(&pool, &blobs, "11", 1).await.unwrap();
    tick(&vlm).await;

    backdate(&pool, "11", 721).await;
    tick(&vlm).await;

    assert_eq!(wire.subjects(), [VIDEO_VLM_GPU, VIDEO_VLM]);
    let (request, id) = wire.requests().pop().unwrap();
    assert!(request.get("prompt").is_some());
    assert_eq!(id, "video_vlm:11:1:2");
    assert_eq!(tries(&pool, "11").await.unwrap(), None);
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_worker1_check_keeps_the_common_timeout(pool: PgPool) {
    let (vlm, blobs, wire, _, _) = bench(&pool, false);
    held_track(&pool, &blobs, "11", 1).await.unwrap();
    tick(&vlm).await;

    backdate(&pool, "11", 721).await;
    tick(&vlm).await;

    assert_eq!(wire.subjects(), [VIDEO_VLM]);
    assert_eq!(stage(&pool, "11").await.unwrap().0, "vlm_dispatched");
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_gpu_clip_cut_on_worker1_keeps_the_common_timeout(pool: PgPool) {
    let (vlm, blobs, wire, _, _) = bench(&pool, false);
    wire.gpu.store(true, Ordering::SeqCst);
    held_track(&pool, &blobs, "11", 1).await.unwrap();
    tick(&vlm).await;
    from_gpu(&vlm, answer("11", 1, VideoLabel::Clip)).await;

    backdate(&pool, "11", 721).await;
    tick(&vlm).await;

    assert_eq!(wire.subjects(), [VIDEO_VLM_GPU, VIDEO_VLM]);
    assert_eq!(stage(&pool, "11").await.unwrap().0, "vlm_dispatched");
}

#[test]
fn only_the_gpu_lane_may_answer_with_a_public_node_timeout() {
    let timeout = failure("11", 1, WorkerReason::PublicNodeTimeout);

    assert_eq!(
        judge(WorkerLane::VideoVlmGpu, &timeout).unwrap(),
        Verdict::Fallback(WorkerReason::PublicNodeTimeout)
    );
    assert!(judge(WorkerLane::VideoVlm, &timeout).is_err());
}
