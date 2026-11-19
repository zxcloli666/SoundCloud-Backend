use backend_contracts::pipeline::{VideoLabel, VideoVlmResult};
use serde_json::{Value, json};
use sqlx::PgPool;
use tokio_util::sync::CancellationToken;

use super::super::queue::Known;
use super::super::result::{Verdict, judge};
use super::fakes::Wire;
use super::{Rig, answer, bench, file_part, finish, held_track, stage, stored};

const LOCAL: &str = "Qwen3VL-2B-Instruct-Q4_K_M";

async fn tick(vlm: &Rig) {
    vlm.dispatch(&CancellationToken::new())
        .await
        .map_err(|error| anyhow::anyhow!(error.to_string()))
        .unwrap();
}

async fn known_field(pool: &PgPool, id: &str) -> Value {
    let row: (Value,) = sqlx::query_as(
        "SELECT COALESCE(payload -> 'haiku', 'null'::jsonb) FROM track_video_dispatch WHERE sc_track_id = $1",
    )
    .bind(id)
    .fetch_one(pool)
    .await
    .unwrap();
    row.0
}

async fn lasting(pool: &PgPool, id: &str, seconds: f64) {
    sqlx::query(
        "UPDATE track_video_dispatch
         SET payload = jsonb_set(payload, '{scores,duration_s}', to_jsonb($2::float8))
         WHERE sc_track_id = $1",
    )
    .bind(id)
    .bind(seconds)
    .execute(pool)
    .await
    .unwrap();
}

fn cut(label: VideoLabel, id: &str, file: &[u8]) -> VideoVlmResult {
    let mut result = file_part(label, id, 1, file, &[file], 0);
    result.model = None;
    result.scores = None;
    result
}

fn last(wire: &Wire) -> (Value, String) {
    wire.requests().pop().unwrap()
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_short_local_clip_answer_asks_for_a_loop_cut_and_stores_it_under_the_local_model(
    pool: PgPool,
) {
    let (vlm, blobs, wire, _, _) = bench(&pool, false);
    held_track(&pool, &blobs, "11", 1).await.unwrap();
    tick(&vlm).await;
    assert!(last(&wire).0.get("prompt").is_some());

    finish(&vlm, answer("11", 1, VideoLabel::Clip))
        .await
        .unwrap();

    let (request, id) = last(&wire);
    assert_eq!(
        (request["label"].clone(), id.as_str()),
        (json!("loop"), "video_vlm:11:1:2")
    );
    assert!(request.get("prompt").is_none() && request.get("frames").is_none());
    assert_eq!(stored(&pool, "11").await.unwrap(), None);
    let known = known_field(&pool, "11").await;
    assert_eq!(
        (&known["label"], &known["model"]),
        (&json!("loop"), &json!(LOCAL))
    );
    assert!((known["scores"]["clip"].as_f64().unwrap() - 0.9).abs() < 1e-6);

    finish(&vlm, cut(VideoLabel::Loop, "11", b"the looped piece"))
        .await
        .unwrap();

    let row = stored(&pool, "11").await.unwrap().unwrap();
    assert_eq!((row.0.as_str(), row.2, row.3), ("loop", 0, true));
    assert_eq!(row.5["vlm"]["model"], LOCAL);
    assert_eq!(row.5["vlm"]["label"], "loop");
    assert!((row.5["vlm"]["scores"]["clip"].as_f64().unwrap() - 0.9).abs() < 1e-6);
    assert_eq!(stage(&pool, "11").await.unwrap().0, "found");
    assert_eq!(blobs.keys(), ["video/11-1.mp4"]);
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_long_local_answer_is_cut_as_a_clip_even_when_the_model_says_loop(pool: PgPool) {
    let (vlm, blobs, wire, _, _) = bench(&pool, false);
    held_track(&pool, &blobs, "11", 1).await.unwrap();
    lasting(&pool, "11", 212.4).await;
    tick(&vlm).await;

    finish(&vlm, answer("11", 1, VideoLabel::Loop))
        .await
        .unwrap();

    assert_eq!(last(&wire).0["label"], "clip");
    finish(&vlm, cut(VideoLabel::Clip, "11", b"the cut piece"))
        .await
        .unwrap();
    let row = stored(&pool, "11").await.unwrap().unwrap();
    assert_eq!((row.0.as_str(), row.2, row.3), ("clip", 1_200, false));
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_local_static_answer_settles_without_a_cut(pool: PgPool) {
    let (vlm, blobs, wire, _, _) = bench(&pool, false);
    held_track(&pool, &blobs, "11", 1).await.unwrap();
    tick(&vlm).await;

    finish(&vlm, answer("11", 1, VideoLabel::Static))
        .await
        .unwrap();

    assert_eq!(wire.requests().len(), 1);
    assert_eq!(stage(&pool, "11").await.unwrap().0, "static");
    assert!(blobs.keys().is_empty());
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_cut_for_another_label_than_the_known_one_is_refused(pool: PgPool) {
    let (vlm, blobs, _, _, _) = bench(&pool, false);
    held_track(&pool, &blobs, "11", 1).await.unwrap();
    tick(&vlm).await;
    finish(&vlm, answer("11", 1, VideoLabel::Clip))
        .await
        .unwrap();

    assert!(
        finish(&vlm, cut(VideoLabel::Clip, "11", b"piece"))
            .await
            .is_err()
    );
    assert_eq!(stored(&pool, "11").await.unwrap(), None);
}

#[test]
fn a_motion_answer_without_a_file_is_the_local_answer_and_needs_its_model() {
    let verdict = judge(&answer("11", 1, VideoLabel::Clip)).unwrap();
    let Verdict::Answered(known) = verdict else {
        panic!("not an answer: {verdict:?}");
    };
    assert_eq!(
        (known.label, known.model.as_str()),
        (VideoLabel::Clip, LOCAL)
    );
    assert!(matches!(
        judge(&answer("11", 1, VideoLabel::Loop)).unwrap(),
        Verdict::Answered(_)
    ));
    let mut unnamed = answer("11", 1, VideoLabel::Clip);
    unnamed.model = None;
    assert!(judge(&unnamed).is_err());
}

#[test]
fn a_known_label_from_haiku_keeps_its_old_shape_on_the_wire() {
    let known = Known {
        label: VideoLabel::Clip,
        model: "example-haiku".to_owned(),
        scores: serde_json::Map::new(),
    };
    assert_eq!(
        serde_json::to_value(&known).unwrap(),
        json!({"label": "clip", "model": "example-haiku"})
    );
    let old: Known = serde_json::from_value(json!({"label": "loop", "model": "m"})).unwrap();
    assert!(old.scores.is_empty());
}
