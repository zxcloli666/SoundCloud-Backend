use backend_contracts::pipeline::{VideoClip, VideoLabel, VideoPart, VideoVlmResult};
use backend_contracts::video_gen::VideoFileSlice;
use base64::Engine as _;
use chrono::Utc;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use tokio_util::sync::CancellationToken;

use super::super::LADDER;
use super::super::haiku;
use super::super::prompt::{self, Features};
use super::super::result::{Verdict, judge};
use super::fakes::{CALL_USD, HAIKU, Wire};
use super::{Rig, base, bench, file_part, finish, held_track, stage, stored};
use crate::llm::anthropic::LlmError;
use crate::llm::ledger;

const FRAMES: [&[u8]; 3] = [b"jpeg-zero", b"jpeg-one", b"jpeg-two"];

pub(super) fn frames_result(id: &str, attempt: i64) -> VideoVlmResult {
    let blob: Vec<u8> = FRAMES.concat();
    let digest = Sha256::digest(&blob);
    let mut offset = 0;
    let files = FRAMES
        .iter()
        .enumerate()
        .map(|(index, frame)| {
            let slice = VideoFileSlice {
                name: format!("frame{index}.jpg"),
                offset,
                bytes: frame.len() as u64,
            };
            offset += frame.len() as u64;
            slice
        })
        .collect();
    let mut result = base(id, attempt);
    result.clip = Some(VideoClip {
        bytes: blob.len() as u64,
        sha256: digest.iter().map(|byte| format!("{byte:02x}")).collect(),
        parts: 1,
    });
    result.files = Some(files);
    result.part = Some(VideoPart {
        index: 0,
        data: base64::engine::general_purpose::STANDARD.encode(&blob),
    });
    result
}

pub(super) fn cut_part(label: VideoLabel, id: &str, file: &[u8]) -> VideoVlmResult {
    let mut result = file_part(label, id, 1, file, &[file], 0);
    result.model = None;
    result.scores = None;
    result
}

async fn tick(vlm: &Rig) {
    vlm.dispatch(&CancellationToken::new())
        .await
        .map_err(|error| anyhow::anyhow!(error.to_string()))
        .unwrap();
}

async fn spent(pool: &PgPool) -> f64 {
    ledger::spent(pool, LADDER, ledger::period(Utc::now(), 17))
        .await
        .unwrap()
}

async fn haiku_field(pool: &PgPool, id: &str) -> Value {
    let row: (Value,) = sqlx::query_as(
        "SELECT COALESCE(payload -> 'haiku', 'null'::jsonb) FROM track_video_dispatch WHERE sc_track_id = $1",
    )
    .bind(id)
    .fetch_one(pool)
    .await
    .unwrap();
    row.0
}

fn label_of(text: &str) -> Result<VideoLabel, LlmError> {
    haiku::reply_of(text).map(|reply| reply.label)
}

fn local_text() -> String {
    prompt::local(&Features {
        duration_s: Some(30.0),
        ..Features::default()
    })
}

fn measured() -> Features {
    Features {
        duration_s: Some(212.4),
        cuts_per_min: Some(3.26),
        motion_med: Some(0.0412),
    }
}

fn short() -> Features {
    Features {
        duration_s: Some(7.9),
        ..Features::default()
    }
}

fn last(wire: &Wire) -> (Value, String) {
    wire.requests().pop().unwrap()
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn without_haiku_the_local_model_gets_the_task_and_the_prompt_from_jobs(pool: PgPool) {
    let (vlm, blobs, wire, _, _) = bench(&pool, false);
    held_track(&pool, &blobs, "11", 1).await.unwrap();

    tick(&vlm).await;

    let (request, id) = last(&wire);
    assert_eq!(request["prompt"], local_text());
    assert!(request.get("frames").is_none() && request.get("label").is_none());
    assert_eq!(id, "video_vlm:11:1:1");
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_haiku_clip_asks_the_worker_for_the_cut_and_stores_it_under_the_haiku_model(
    pool: PgPool,
) {
    let (vlm, blobs, wire, eyes, _) = bench(&pool, true);
    held_track(&pool, &blobs, "11", 1).await.unwrap();

    tick(&vlm).await;
    let (request, id) = last(&wire);
    assert_eq!(
        (request["frames"].clone(), id.as_str()),
        (json!(true), "video_vlm:11:1:1")
    );
    assert!(request.get("prompt").is_none());

    eyes.answers(Ok(VideoLabel::Clip));
    finish(&vlm, frames_result("11", 1)).await.unwrap();

    let seen = eyes.seen.lock().unwrap().clone();
    assert_eq!(
        seen,
        vec![
            FRAMES
                .iter()
                .map(|frame| frame.to_vec())
                .collect::<Vec<_>>()
        ]
    );
    let (request, id) = last(&wire);
    assert_eq!(
        (request["label"].clone(), id.as_str()),
        (json!("clip"), "video_vlm:11:1:2")
    );
    assert!(request.get("frames").is_none() && request.get("prompt").is_none());
    assert_eq!(
        haiku_field(&pool, "11").await,
        json!({"label": "clip", "model": HAIKU})
    );
    assert!((spent(&pool).await - CALL_USD).abs() < 1e-9);

    finish(&vlm, cut_part(VideoLabel::Clip, "11", b"the cut piece"))
        .await
        .unwrap();

    let row = stored(&pool, "11").await.unwrap().unwrap();
    assert_eq!((row.0.as_str(), row.1.as_str()), ("clip", "video/11-1.mp4"));
    assert_eq!(
        row.5["vlm"],
        json!({"label": "clip", "model": HAIKU, "scores": {}})
    );
    assert_eq!(row.5["ber"], 0.04);
    assert_eq!(stage(&pool, "11").await.unwrap().0, "found");
    assert_eq!(blobs.keys(), ["video/11-1.mp4"]);
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_haiku_static_settles_the_track_without_another_worker_task(pool: PgPool) {
    let (vlm, blobs, wire, eyes, _) = bench(&pool, true);
    held_track(&pool, &blobs, "11", 1).await.unwrap();
    tick(&vlm).await;

    eyes.answers(Ok(VideoLabel::Static));
    finish(&vlm, frames_result("11", 1)).await.unwrap();

    assert_eq!(wire.requests().len(), 1);
    assert_eq!(
        stage(&pool, "11").await.unwrap(),
        ("static".to_owned(), 1, Some("static_video".to_owned()))
    );
    assert!(blobs.keys().is_empty());
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_down_api_or_a_refusal_hands_the_video_to_the_local_model(pool: PgPool) {
    for (id, failure) in [
        ("11", LlmError::Down("status 529".to_owned())),
        ("12", LlmError::Invalid("the model refused".to_owned())),
    ] {
        let (vlm, blobs, wire, eyes, _) = bench(&pool, true);
        held_track(&pool, &blobs, id, 1).await.unwrap();
        tick(&vlm).await;

        eyes.answers(Err(failure));
        finish(&vlm, frames_result(id, 1)).await.unwrap();

        let (request, message) = last(&wire);
        assert_eq!(request["prompt"], local_text());
        assert!(request.get("frames").is_none() && request.get("label").is_none());
        assert_eq!(message, format!("video_vlm:{id}:1:2"));
        assert_eq!(haiku_field(&pool, id).await, Value::Null);
        assert_eq!(stage(&pool, id).await.unwrap().0, "vlm_dispatched");
        sqlx::query("DELETE FROM track_video_dispatch")
            .execute(&pool)
            .await
            .unwrap();
    }
    assert!((spent(&pool).await - CALL_USD).abs() < 1e-9);
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_spent_cap_alerts_once_keeps_the_local_model_working_and_comes_back_by_itself(
    pool: PgPool,
) {
    let (vlm, blobs, wire, eyes, inbox) = bench(&pool, true);
    let period = ledger::period(Utc::now(), 17);
    sqlx::query(
        "INSERT INTO llm_spend (ladder, period, sc_track_id, call, provider, model, usd)
         VALUES ($1, $2, '1', 'vlm', 'haiku', 'example-haiku', 5.0),
                ('video_gen', $2, '1', 'plan', 'opus', 'example-opus', 0.5)",
    )
    .bind(LADDER)
    .bind(period)
    .execute(&pool)
    .await
    .unwrap();

    for id in ["11", "12"] {
        held_track(&pool, &blobs, id, 1).await.unwrap();
        tick(&vlm).await;
        assert_eq!(last(&wire).0["prompt"], local_text());
    }
    assert_eq!(inbox.count(), 1);
    assert!(inbox.messages.lock().unwrap()[0].contains("$5.00"));

    sqlx::query("DELETE FROM llm_spend WHERE ladder = $1")
        .bind(LADDER)
        .execute(&pool)
        .await
        .unwrap();
    held_track(&pool, &blobs, "13", 1).await.unwrap();
    sqlx::query(
        "UPDATE track_video_dispatch SET stage = 'found' WHERE sc_track_id IN ('11', '12')",
    )
    .execute(&pool)
    .await
    .unwrap();
    tick(&vlm).await;
    assert_eq!(last(&wire).0["prompt"], local_text());

    sqlx::query("UPDATE llm_pause SET paused_until = now() WHERE ladder = $1")
        .bind(LADDER)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE track_video_dispatch SET stage = 'pending_vlm' WHERE sc_track_id = '13'")
        .execute(&pool)
        .await
        .unwrap();
    tick(&vlm).await;

    assert_eq!(last(&wire).0["frames"], json!(true));
    assert!(ledger::paused(&pool, LADDER).await.unwrap().is_none());
    assert!(ledger::paused(&pool, "video_gen").await.unwrap().is_none());
    assert_eq!(inbox.count(), 1);
    assert!(eyes.seen.lock().unwrap().is_empty());
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_cut_without_a_model_needs_a_known_haiku_label(pool: PgPool) {
    let (vlm, blobs, _, _, _) = bench(&pool, true);
    held_track(&pool, &blobs, "11", 1).await.unwrap();
    tick(&vlm).await;

    assert!(
        finish(&vlm, cut_part(VideoLabel::Clip, "11", b"piece"))
            .await
            .is_err()
    );
    assert_eq!(stored(&pool, "11").await.unwrap(), None);
}

#[test]
fn frames_without_a_label_are_frames_and_a_gap_between_them_is_refused() {
    assert!(matches!(
        judge(&frames_result("11", 1)).unwrap(),
        Verdict::Frames(_)
    ));
    let mut gap = frames_result("11", 1);
    if let Some(files) = gap.files.as_mut() {
        files[1].offset += 1;
    }
    assert!(judge(&gap).is_err());
    let mut bare = frames_result("11", 1);
    bare.files = None;
    assert!(judge(&bare).is_err());
}

#[test]
fn the_haiku_request_sends_every_frame_as_a_jpeg_before_the_prompt() {
    let frames: Vec<Vec<u8>> = FRAMES.iter().map(|frame| frame.to_vec()).collect();

    let text = prompt::remote(frames.len(), &measured());
    let message = haiku::request(&frames, &text);

    let content = message["content"].as_array().unwrap();
    assert_eq!(content.len(), 4);
    assert_eq!(content[0]["source"]["media_type"], "image/jpeg");
    assert_eq!(
        content[2]["source"]["data"],
        base64::engine::general_purpose::STANDARD.encode(b"jpeg-two")
    );
    assert_eq!(content[3], json!({"type": "text", "text": text}));
}

#[test]
fn the_schema_asks_for_a_reason_next_to_a_static_or_clip_label() {
    let schema = haiku::schema();
    assert_eq!(
        schema["properties"]["label"]["enum"],
        json!(["clip", "static"])
    );
    assert_eq!(schema["properties"]["reason"]["type"], "string");
    assert_eq!(schema["required"], json!(["reason", "label"]));
}

#[test]
fn motion_up_to_30_s_is_a_loop_and_longer_is_a_clip() {
    assert_eq!(short().kind(VideoLabel::Clip), VideoLabel::Loop);
    assert_eq!(short().kind(VideoLabel::Static), VideoLabel::Static);
    assert_eq!(measured().kind(VideoLabel::Clip), VideoLabel::Clip);
    assert_eq!(measured().kind(VideoLabel::Loop), VideoLabel::Clip);
    assert_eq!(Features::default().kind(VideoLabel::Loop), VideoLabel::Clip);
    let edge = Features {
        duration_s: Some(30.0),
        ..Features::default()
    };
    assert_eq!(edge.kind(VideoLabel::Clip), VideoLabel::Loop);
}

#[test]
fn haiku_and_the_local_model_are_both_asked_only_static_or_clip() {
    let canvas = prompt::remote(8, &short());
    assert!(!canvas.contains("loop -"));
    assert!(canvas.contains("however short and even if it repeats"));
    assert!(canvas.ends_with(r#""label": "static|clip"}"#));

    for features in [short(), measured(), Features::default()] {
        let local = prompt::local(&features);
        assert!(!local.contains("loop"));
        assert!(local.ends_with("Answer with one word: static or clip."));
    }
}

#[test]
fn the_text_with_features_has_the_measured_line_and_the_json_reply() {
    let text = prompt::remote(8, &measured());

    assert!(text.starts_with("These are 8 frames from one video"));
    assert!(text.contains(
        "Measured on the file: duration 212 s, scene cuts per minute 3.3, median share of moving pixels 0.041."
    ));
    assert!(text.contains("static - no real filmed footage"));
    assert!(
        text.ends_with(
            r#"Reply as JSON: {"reason": "<one short sentence>", "label": "static|clip"}"#
        )
    );
}

#[test]
fn the_text_without_features_leaves_the_line_out_and_a_lone_feature_stays_alone() {
    let bare = prompt::remote(3, &Features::default());
    assert!(!bare.contains("Measured"));

    let lone = prompt::remote(
        3,
        &Features {
            cuts_per_min: Some(12.0),
            ..Features::default()
        },
    );
    assert!(lone.contains("Measured on the file: scene cuts per minute 12.0.\n"));
    assert!(!lone.contains("duration") && !lone.contains("moving pixels"));
}

#[test]
fn the_local_text_is_the_benched_wording_with_the_duration_and_no_other_measure() {
    assert_eq!(
        prompt::local(&measured()),
        [
            "These images are frames from one music video, in time order, spread evenly over the video. The video is 212 s long.",
            "Classify the video as one of:",
            "static - no real filmed footage. One still picture or a few photos, album art, a logo, a drawn cover, a fan edit of photos, or a lyric video (song words on screen over a background). Effects on a still are not footage: zoom, pan, shake, flashes, glow, rain, particles, grain, color shifts, a moving waveform.",
            "clip - real filmed footage or real animation that goes on and changes over time: people move, the camera moves, shots and scenes change. A phone video of a person or a concert is a clip, even if the framing barely changes.",
            "If song lyrics are written over the picture, it is static.",
            "Answer with one word: static or clip.",
        ]
        .join("\n")
    );
    let bare = prompt::local(&Features::default());
    assert!(bare.starts_with(
        "These images are frames from one music video, in time order, spread evenly over the video.\n"
    ));
    assert!(!bare.contains(" s long"));
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn haiku_gets_the_features_of_the_held_candidate(pool: PgPool) {
    let (vlm, blobs, _, eyes, _) = bench(&pool, true);
    held_track(&pool, &blobs, "11", 1).await.unwrap();
    tick(&vlm).await;
    eyes.answers(Ok(VideoLabel::Clip));

    finish(&vlm, frames_result("11", 1)).await.unwrap();

    let features = eyes.features.lock().unwrap().clone();
    assert_eq!(
        features,
        vec![Features {
            duration_s: Some(30.0),
            ..Features::default()
        }]
    );
}

#[test]
fn only_one_of_the_three_labels_is_an_answer() {
    assert_eq!(label_of(r#"{"label": "loop"}"#), Ok(VideoLabel::Loop));
    assert_eq!(
        label_of(r#"{"reason": "words over a still", "label": "static"}"#),
        Ok(VideoLabel::Static)
    );
    assert_eq!(
        haiku::reply_of(r#"{"reason": "words over a still", "label": "static"}"#)
            .unwrap()
            .reason,
        "words over a still"
    );
    for bad in [
        r#"{"label": "dance"}"#,
        r#"{"reason": "x", "label": "dance"}"#,
        r#"{"reason": "x", "label": "clip", "why": "cuts"}"#,
        "clip",
        "",
    ] {
        assert!(label_of(bad).is_err());
    }
}
