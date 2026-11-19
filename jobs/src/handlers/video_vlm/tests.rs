use std::collections::BTreeMap;
use std::time::{Duration, SystemTime};

use backend_contracts::pipeline::{Producer, VideoClip, VideoLabel, VideoPart, VideoVlmResult};
use backend_contracts::reasons::{WorkerReason, WorkerStatus};
use base64::Engine as _;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::PgPool;

use super::queue;
use super::result::{Verdict, judge};
use super::{Step, VideoVlm};
use crate::config::VideoVlmConfig;
use crate::handlers::video_find::tests::{Memory, config};

mod fakes;
mod haiku;
mod local;

use self::fakes::{Eyes, Inbox, Wire};

type Rig = VideoVlm<Memory, Wire, Eyes, Inbox>;

fn vlm_config(haiku: bool) -> VideoVlmConfig {
    VideoVlmConfig {
        in_flight: 2,
        tick: Duration::from_secs(30),
        timeout: Duration::from_secs(4_200),
        month_usd: 5.0,
        recheck: Duration::from_secs(1_800),
        haiku: haiku.then(|| crate::config::AnthropicLadder {
            steps: Vec::new(),
            reset_day: 17,
        }),
    }
}

fn rig(pool: &PgPool) -> (Rig, Memory) {
    let (vlm, blobs, _, _, _) = bench(pool, false);
    (vlm, blobs)
}

fn bench(pool: &PgPool, haiku: bool) -> (Rig, Memory, Wire, Eyes, Inbox) {
    let blobs = Memory::default();
    let wire = Wire::default();
    let eyes = Eyes::default();
    let inbox = Inbox::default();
    let vlm = VideoVlm::new(
        pool.clone(),
        "https://storage.test/".parse().unwrap(),
        vlm_config(haiku),
        &config(),
        blobs.clone(),
        wire.clone(),
        haiku.then(|| eyes.clone()),
        inbox.clone(),
    );
    (vlm, blobs, wire, eyes, inbox)
}

async fn held_track(
    pool: &PgPool,
    blobs: &Memory,
    id: &str,
    attempt: i32,
) -> anyhow::Result<String> {
    sqlx::query(
        "INSERT INTO tracks (sc_track_id, urn, title, title_normalized, sharing, duration_ms,
                             storage_state, metadata_artist)
         VALUES ($1, 'soundcloud:tracks:' || $1, 'Song', 'song', 'public', 200000, 'ok', 'Artist')",
    )
    .bind(id)
    .execute(pool)
    .await?;
    let key = format!("video-tmp/{id}-{attempt}-1.mp4");
    blobs.put_at(&key, b"temp bytes", SystemTime::now());
    let payload = json!({
        "kind": "loop",
        "s3_key": key,
        "offset_ms": 1_200,
        "duration_ms": 30_000,
        "loop": true,
        "source": "youtube",
        "source_ref": "abc",
        "scores": {"ber": 0.04},
    });
    sqlx::query(
        "INSERT INTO track_video_dispatch (sc_track_id, stage, attempts, next_at, payload)
         VALUES ($1, 'pending_vlm', $2, now(), $3)",
    )
    .bind(id)
    .bind(attempt)
    .bind(payload)
    .execute(pool)
    .await?;
    Ok(key)
}

async fn claim(pool: &PgPool, limit: i64) -> anyhow::Result<Vec<queue::Claimed>> {
    queue::claim(pool, limit)
        .await
        .map_err(|error| anyhow::anyhow!(error.to_string()))
}

async fn dispatched(
    pool: &PgPool,
    blobs: &Memory,
    id: &str,
    attempt: i32,
) -> anyhow::Result<String> {
    let key = held_track(pool, blobs, id, attempt).await?;
    assert_eq!(claim(pool, 1).await?.len(), 1);
    Ok(key)
}

async fn stage(pool: &PgPool, id: &str) -> anyhow::Result<(String, i32, Option<String>)> {
    Ok(sqlx::query_as(
        "SELECT stage, attempts, last_error FROM track_video_dispatch WHERE sc_track_id = $1",
    )
    .bind(id)
    .fetch_one(pool)
    .await?)
}

async fn tries(pool: &PgPool, id: &str) -> anyhow::Result<Option<String>> {
    let row: (Option<String>,) = sqlx::query_as(
        "SELECT payload ->> 'vlm_tries' FROM track_video_dispatch WHERE sc_track_id = $1",
    )
    .bind(id)
    .fetch_one(pool)
    .await?;
    Ok(row.0)
}

async fn stored(
    pool: &PgPool,
    id: &str,
) -> anyhow::Result<Option<(String, String, i32, bool, i32, Value)>> {
    Ok(sqlx::query_as(
        "SELECT kind, s3_key, offset_ms, loop, duration_ms, verify
         FROM track_videos WHERE sc_track_id = $1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?)
}

fn producer() -> Producer {
    Producer {
        worker_id: "sc-worker1".to_owned(),
        build: "test".to_owned(),
        models: BTreeMap::new(),
        sync_version: None,
    }
}

fn base(id: &str, attempt: i64) -> VideoVlmResult {
    VideoVlmResult {
        sc_track_id: id.to_owned(),
        attempt,
        status: WorkerStatus::Ok,
        reason: None,
        detail: None,
        producer: producer(),
        label: None,
        model: None,
        scores: None,
        duration_ms: None,
        clip: None,
        files: None,
        part: None,
    }
}

fn answer(id: &str, attempt: i64, label: VideoLabel) -> VideoVlmResult {
    let mut result = base(id, attempt);
    result.label = Some(label);
    result.model = Some("Qwen3VL-2B-Instruct-Q4_K_M".to_owned());
    result.scores = Some(BTreeMap::from([(label.as_str().to_owned(), 0.9)]));
    result
}

fn failure(id: &str, attempt: i64, reason: WorkerReason) -> VideoVlmResult {
    let mut result = base(id, attempt);
    result.status = reason.status();
    result.reason = Some(reason);
    result
}

fn clip_part(id: &str, attempt: i64, file: &[u8], parts: &[&[u8]], index: usize) -> VideoVlmResult {
    file_part(VideoLabel::Clip, id, attempt, file, parts, index)
}

fn loop_part(id: &str, attempt: i64, file: &[u8], parts: &[&[u8]], index: usize) -> VideoVlmResult {
    file_part(VideoLabel::Loop, id, attempt, file, parts, index)
}

fn file_part(
    label: VideoLabel,
    id: &str,
    attempt: i64,
    file: &[u8],
    parts: &[&[u8]],
    index: usize,
) -> VideoVlmResult {
    let digest = Sha256::digest(file);
    let mut result = answer(id, attempt, label);
    result.duration_ms = Some(25_000);
    result.clip = Some(VideoClip {
        bytes: file.len() as u64,
        sha256: digest.iter().map(|byte| format!("{byte:02x}")).collect(),
        parts: parts.len() as u32,
    });
    result.part = Some(VideoPart {
        index: index as u32,
        data: base64::engine::general_purpose::STANDARD.encode(parts[index]),
    });
    result
}

async fn finish(vlm: &Rig, result: VideoVlmResult) -> anyhow::Result<()> {
    vlm.finish(result)
        .await
        .map_err(|error| anyhow::anyhow!(error.to_string()))
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn only_due_held_rows_are_claimed_once_with_the_track_duration(
    pool: PgPool,
) -> anyhow::Result<()> {
    let (_, blobs) = rig(&pool);
    held_track(&pool, &blobs, "11", 2).await?;
    held_track(&pool, &blobs, "12", 1).await?;
    sqlx::query("UPDATE track_video_dispatch SET next_at = now() + interval '1 hour' WHERE sc_track_id = '12'")
        .execute(&pool)
        .await?;
    sqlx::query("INSERT INTO track_video_dispatch (sc_track_id, stage, attempts) VALUES ('13', 'dispatched', 1)")
        .execute(&pool)
        .await?;

    let claimed = claim(&pool, 10).await?;

    assert_eq!(claimed.len(), 1);
    assert_eq!(claimed[0].sc_track_id, "11");
    assert_eq!(claimed[0].attempt, 2);
    assert_eq!(claimed[0].duration_ms, Some(200_000));
    assert_eq!(claimed[0].held.as_ref().unwrap().vlm_sent, 1);
    assert_eq!(stage(&pool, "11").await?.0, "vlm_dispatched");
    assert_eq!(stage(&pool, "12").await?.0, "pending_vlm");
    assert_eq!(stage(&pool, "13").await?.0, "dispatched");
    assert_eq!(queue::busy(&pool).await.unwrap(), 1);
    assert!(claim(&pool, 10).await?.is_empty());
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn the_request_names_the_temp_file_by_its_storage_url_and_a_resend_gets_a_new_message_id(
    pool: PgPool,
) -> anyhow::Result<()> {
    let (vlm, blobs) = rig(&pool);
    held_track(&pool, &blobs, "11", 2).await?;
    let first = claim(&pool, 1).await?.remove(0);
    let held = first.held.clone().unwrap();

    let request = vlm.request(&first, &held, Step::Classify).unwrap();

    assert_eq!(
        request.video_url,
        "https://storage.test/video-tmp/11-2-1.mp4"
    );
    assert_eq!(request.attempt, 2);
    assert_eq!(request.duration_ms, Some(200_000));
    assert_eq!(request.offset_ms, Some(1_200));

    sqlx::query("UPDATE track_video_dispatch SET updated_at = now() - interval '2 hours' WHERE sc_track_id = '11'")
        .execute(&pool)
        .await?;
    queue::requeue_unanswered(&pool, Duration::from_secs(3_600))
        .await
        .unwrap();
    assert_eq!(stage(&pool, "11").await?.0, "pending_vlm");
    let second = claim(&pool, 1).await?.remove(0);

    assert_eq!(first.message_id(&held), "video_vlm:11:2:1");
    assert_eq!(
        second.message_id(second.held.as_ref().unwrap()),
        "video_vlm:11:2:2"
    );
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_clip_arrives_in_parts_and_is_stored_whole_with_the_temp_file_deleted(
    pool: PgPool,
) -> anyhow::Result<()> {
    let (vlm, blobs) = rig(&pool);
    dispatched(&pool, &blobs, "11", 1).await?;
    let file = b"the whole cut segment";
    let parts: [&[u8]; 3] = [b"the whole", b" cut ", b"segment"];

    finish(&vlm, clip_part("11", 1, file, &parts, 2)).await?;
    finish(&vlm, clip_part("11", 1, file, &parts, 0)).await?;
    assert_eq!(stored(&pool, "11").await?, None);
    assert_eq!(stage(&pool, "11").await?.0, "vlm_dispatched");
    finish(&vlm, clip_part("11", 1, file, &parts, 1)).await?;

    assert_eq!(blobs.keys(), ["video/11-1.mp4"]);
    assert_eq!(blobs.get("video/11-1.mp4").as_deref(), Some(&file[..]));
    let row = stored(&pool, "11").await?.unwrap();
    assert_eq!(
        (row.0.as_str(), row.1.as_str(), row.2, row.3, row.4),
        ("clip", "video/11-1.mp4", 1_200, false, 25_000)
    );
    assert_eq!(row.5["ber"], 0.04);
    assert_eq!(row.5["vlm"]["model"], "Qwen3VL-2B-Instruct-Q4_K_M");
    assert_eq!(stage(&pool, "11").await?.0, "found");
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_clip_whose_checksum_fails_is_not_stored_and_the_temp_file_stays(
    pool: PgPool,
) -> anyhow::Result<()> {
    let (vlm, blobs) = rig(&pool);
    let key = dispatched(&pool, &blobs, "11", 1).await?;
    let parts: [&[u8]; 2] = [b"abc", b"def"];

    finish(&vlm, clip_part("11", 1, b"abcdef", &parts, 0)).await?;
    let corrupted = clip_part("11", 1, b"abcdef", &[b"abc", b"xyz"], 1);

    assert!(finish(&vlm, corrupted).await.is_err());
    assert_eq!(stored(&pool, "11").await?, None);
    assert_eq!(blobs.keys(), [key]);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_loop_arrives_in_parts_and_is_stored_from_them_not_from_the_temp_file(
    pool: PgPool,
) -> anyhow::Result<()> {
    let (vlm, blobs) = rig(&pool);
    dispatched(&pool, &blobs, "11", 1).await?;
    let file = b"the encoded loop";
    let parts: [&[u8]; 2] = [b"the encoded", b" loop"];

    finish(&vlm, loop_part("11", 1, file, &parts, 1)).await?;
    assert_eq!(stored(&pool, "11").await?, None);
    finish(&vlm, loop_part("11", 1, file, &parts, 0)).await?;

    assert_eq!(blobs.keys(), ["video/11-1.mp4"]);
    assert_eq!(blobs.get("video/11-1.mp4").as_deref(), Some(&file[..]));
    let row = stored(&pool, "11").await?.unwrap();
    assert_eq!(
        (row.0.as_str(), row.1.as_str(), row.2, row.3, row.4),
        ("loop", "video/11-1.mp4", 0, true, 25_000)
    );
    assert_eq!(row.5["vlm"]["label"], "loop");
    assert_eq!(stage(&pool, "11").await?.0, "found");
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_static_answer_settles_the_track_and_deletes_the_temp_file(
    pool: PgPool,
) -> anyhow::Result<()> {
    let (vlm, blobs) = rig(&pool);
    dispatched(&pool, &blobs, "11", 1).await?;

    finish(&vlm, answer("11", 1, VideoLabel::Static)).await?;

    assert!(blobs.keys().is_empty());
    assert_eq!(stored(&pool, "11").await?, None);
    assert_eq!(
        stage(&pool, "11").await?,
        ("static".to_owned(), 1, Some("static_video".to_owned()))
    );
    let waits: (bool,) = sqlx::query_as(
        "SELECT next_at > now() + interval '80 days' FROM track_video_dispatch WHERE sc_track_id = '11'",
    )
    .fetch_one(&pool)
    .await?;
    assert!(waits.0);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_gone_file_sends_the_track_back_to_the_search_until_the_attempts_run_out(
    pool: PgPool,
) -> anyhow::Result<()> {
    let (vlm, blobs) = rig(&pool);
    dispatched(&pool, &blobs, "11", 1).await?;

    finish(&vlm, failure("11", 1, WorkerReason::InputsGone)).await?;

    assert!(blobs.keys().is_empty());
    assert_eq!(
        stage(&pool, "11").await?,
        ("retry".to_owned(), 1, Some("inputs_gone".to_owned()))
    );

    dispatched(&pool, &blobs, "12", 3).await?;
    finish(&vlm, failure("12", 3, WorkerReason::InputsGone)).await?;
    assert_eq!(stage(&pool, "12").await?.0, "none");
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_transient_failure_resends_the_same_file_until_the_tries_run_out(
    pool: PgPool,
) -> anyhow::Result<()> {
    let (vlm, blobs) = rig(&pool);
    let key = dispatched(&pool, &blobs, "11", 1).await?;

    for expected in ["1", "2"] {
        finish(&vlm, failure("11", 1, WorkerReason::EngineCrashed)).await?;
        assert_eq!(stage(&pool, "11").await?.0, "pending_vlm");
        assert_eq!(tries(&pool, "11").await?.as_deref(), Some(expected));
        assert_eq!(blobs.keys(), std::slice::from_ref(&key));
        sqlx::query("UPDATE track_video_dispatch SET next_at = now() WHERE sc_track_id = '11'")
            .execute(&pool)
            .await?;
        assert_eq!(claim(&pool, 1).await?.len(), 1);
    }
    finish(&vlm, failure("11", 1, WorkerReason::OutOfMemory)).await?;

    assert_eq!(
        stage(&pool, "11").await?,
        ("none".to_owned(), 1, Some("out_of_memory".to_owned()))
    );
    assert!(blobs.keys().is_empty());
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_deterministic_failure_drops_the_candidate_and_the_temp_file(
    pool: PgPool,
) -> anyhow::Result<()> {
    let (vlm, blobs) = rig(&pool);
    dispatched(&pool, &blobs, "11", 1).await?;

    finish(&vlm, failure("11", 1, WorkerReason::UndecodableAudio)).await?;

    assert!(blobs.keys().is_empty());
    assert_eq!(
        stage(&pool, "11").await?,
        ("none".to_owned(), 1, Some("undecodable_audio".to_owned()))
    );
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_result_for_another_attempt_or_an_idle_row_changes_nothing(
    pool: PgPool,
) -> anyhow::Result<()> {
    let (vlm, blobs) = rig(&pool);
    let key = dispatched(&pool, &blobs, "11", 1).await?;

    finish(&vlm, answer("11", 2, VideoLabel::Static)).await?;
    finish(&vlm, failure("11", 2, WorkerReason::InputsGone)).await?;
    finish(&vlm, answer("11", 1, VideoLabel::Static)).await?;
    finish(&vlm, answer("11", 1, VideoLabel::Loop)).await?;

    assert_eq!(stage(&pool, "11").await?.0, "static");
    assert_eq!(stored(&pool, "11").await?, None);
    assert!(blobs.get(&key).is_none());
    Ok(())
}

#[test]
fn an_ok_answer_needs_its_label_and_exactly_the_file_its_label_allows() {
    let mut no_label = answer("11", 1, VideoLabel::Loop);
    no_label.label = None;
    let mut static_with_file = clip_part("11", 1, b"ab", &[b"ab"], 0);
    static_with_file.label = Some(VideoLabel::Static);
    let mut half = clip_part("11", 1, b"ab", &[b"ab"], 0);
    half.part = None;

    for bad in [no_label, static_with_file, half] {
        assert!(judge(&bad).is_err());
    }
    assert!(matches!(
        judge(&clip_part("11", 1, b"ab", &[b"ab"], 0)).unwrap(),
        Verdict::Labelled(_)
    ));
    assert!(matches!(
        judge(&loop_part("11", 1, b"ab", &[b"ab"], 0)).unwrap(),
        Verdict::Labelled(_)
    ));
    assert!(matches!(
        judge(&answer("11", 1, VideoLabel::Static)).unwrap(),
        Verdict::Labelled(_)
    ));
}

#[test]
fn failures_split_into_gone_again_and_drop_and_foreign_reasons_are_refused() {
    let verdict = |reason| judge(&failure("11", 1, reason)).unwrap();

    assert_eq!(verdict(WorkerReason::InputsGone), Verdict::Gone);
    assert_eq!(
        verdict(WorkerReason::DeadlineExceeded),
        Verdict::Again(WorkerReason::DeadlineExceeded)
    );
    assert_eq!(
        verdict(WorkerReason::EngineRestarted),
        Verdict::Again(WorkerReason::EngineRestarted)
    );
    assert_eq!(
        verdict(WorkerReason::ModelOutputInvalid),
        Verdict::Drop(WorkerReason::ModelOutputInvalid)
    );
    assert!(judge(&failure("11", 1, WorkerReason::NoVideoFound)).is_err());
    assert!(judge(&failure("abc", 1, WorkerReason::InputsGone)).is_err());
}
