use serde_json::json;

use super::*;

fn outcome(fields: serde_json::Value) -> anyhow::Result<CollabResult> {
    let mut body = json!({
        "input_object": "collab-input-7c1d",
        "status": "ok",
        "trained": true,
        "object": "collab-input-7c1d-vectors",
        "dim": 128,
        "points_count": 1,
        "producer": {
            "worker_id": "gpu-main",
            "build": "2026.09.1+abc123",
            "models": {},
            "sync_version": null
        }
    });
    if let (Some(body), Some(fields)) = (body.as_object_mut(), fields.as_object()) {
        for (key, value) in fields {
            if value.is_null() {
                body.remove(key);
            } else {
                body.insert(key.clone(), value.clone());
            }
        }
    }
    Ok(serde_json::from_value(body)?)
}

fn point(id: u64, value: f32) -> CollabPoint {
    CollabPoint {
        id,
        vector: vec![value; 128],
    }
}

fn blob(points: Vec<CollabPoint>) -> CollabBlob {
    CollabBlob {
        dim: TRACKS_COLLAB_DIMENSIONS,
        points,
        metrics: None,
    }
}

#[test]
fn a_trained_result_points_at_the_vectors_the_contract_names() -> anyhow::Result<()> {
    let result = outcome(json!({}))?;

    assert_eq!(
        vectors_object_of(&result)?,
        Some("collab-input-7c1d-vectors".to_owned())
    );
    Ok(())
}

#[test]
fn a_result_naming_other_vectors_is_refused() -> anyhow::Result<()> {
    let renamed = outcome(json!({ "object": "collab-input-7c1d-vectors-1700000000000" }))?;
    let unnamed = outcome(json!({ "object": null }))?;

    assert!(vectors_object_of(&renamed).is_err());
    assert!(vectors_object_of(&unnamed).is_err());
    Ok(())
}

#[test]
fn trained_must_agree_with_the_status() -> anyhow::Result<()> {
    let untrained_ok = outcome(json!({ "trained": false }))?;
    let trained_rejection = outcome(json!({
        "status": "rejected",
        "reason": "below_baseline",
        "object": null
    }))?;

    assert!(vectors_object_of(&untrained_ok).is_err());
    assert!(vectors_object_of(&trained_rejection).is_err());
    Ok(())
}

#[test]
fn a_rejection_has_no_vectors_to_apply() -> anyhow::Result<()> {
    let result = outcome(json!({
        "status": "rejected",
        "reason": "below_baseline",
        "detail": "hr_at_20=0.1000 popularity=0.1300",
        "trained": false,
        "object": null,
        "points_count": 0
    }))?;

    assert_eq!(vectors_object_of(&result)?, None);
    assert_eq!(result.reason, Some(WorkerReason::BelowBaseline));
    Ok(())
}

fn failure(status: &str, reason: Option<&str>) -> anyhow::Result<CollabResult> {
    outcome(json!({
        "status": status,
        "reason": reason,
        "trained": false,
        "object": null,
        "points_count": 0
    }))
}

#[test]
fn an_untrained_result_must_carry_a_reason_its_lane_publishes() -> anyhow::Result<()> {
    let silent = failure("failed", None)?;
    let foreign = failure("failed", Some("public_node_timeout"))?;
    let download = failure("failed", Some("download_failed"))?;
    let mismatched = failure("empty", Some("deadline_exceeded"))?;
    let ok_with_reason = outcome(json!({ "reason": "below_baseline" }))?;

    assert!(vectors_object_of(&silent).is_err());
    assert!(vectors_object_of(&foreign).is_err());
    assert!(vectors_object_of(&download).is_err());
    assert!(vectors_object_of(&mismatched).is_err());
    assert!(vectors_object_of(&ok_with_reason).is_err());
    assert_eq!(
        vectors_object_of(&failure("empty", Some("empty_vocab"))?)?,
        None
    );
    Ok(())
}

#[test]
fn an_interrupted_training_is_reopened_and_a_verdict_is_not() -> anyhow::Result<()> {
    let restarted = failure("failed", Some("engine_restarted"))?;
    let lost = failure("failed", Some("worker_lost"))?;
    let crashed = failure("failed", Some("engine_crashed"))?;
    let below = failure("rejected", Some("below_baseline"))?;

    assert_eq!(vectors_object_of(&restarted)?, None);
    assert!(needs_reopen(&restarted));
    assert!(needs_reopen(&lost));
    assert!(!needs_reopen(&crashed));
    assert!(!needs_reopen(&below));
    Ok(())
}

#[test]
fn a_redelivered_interruption_queues_one_training() -> anyhow::Result<()> {
    let first = reopen_job("collab-input-7c1d")?;
    let again = reopen_job("collab-input-7c1d")?;
    let other = reopen_job("collab-input-9e2f")?;

    assert_eq!(first.id, again.id);
    assert_ne!(first.id, other.id);
    assert_eq!(first.kind, JobKind::CollabTrain);
    assert_eq!(first.dedup_key.as_deref(), Some(REOPEN_DEDUP_KEY));
    let payload: Versioned<CollabTrainPayload> = serde_json::from_value(first.payload)?;
    assert_eq!(payload.into_latest(), CollabTrainPayload::default());
    Ok(())
}

#[test]
fn a_result_in_another_dimension_is_refused() -> anyhow::Result<()> {
    let result = outcome(json!({ "dim": 64 }))?;

    assert!(vectors_object_of(&result).is_err());
    Ok(())
}

#[test]
fn the_worker_example_from_the_contract_is_read() -> anyhow::Result<()> {
    let result: CollabResult = serde_json::from_str(
        r#"{"input_object":"collab-input-7c1d","status":"ok","trained":true,"object":"collab-input-7c1d-vectors","dim":128,"points_count":18342,"producer":{"worker_id":"gpu-main","build":"2026.09.1+abc123","models":{},"sync_version":null}}"#,
    )?;

    assert_eq!(result.points_count, 18_342);
    assert_eq!(
        vectors_object_of(&result)?.as_deref(),
        Some("collab-input-7c1d-vectors")
    );
    Ok(())
}

#[test]
fn accepts_well_formed_points() -> anyhow::Result<()> {
    let points = validate_blob(blob(vec![point(1, 0.25), point(2, -0.5)]), 2)?;

    assert_eq!(points.len(), 2);
    Ok(())
}

#[test]
fn rejects_duplicate_ids_and_invalid_vectors() {
    let duplicate = validate_blob(blob(vec![point(1, 0.5), point(1, 0.75)]), 2);
    let non_finite = validate_blob(blob(vec![point(1, f32::NAN)]), 1);
    let zero_id = validate_blob(blob(vec![point(0, 0.5)]), 1);
    let short = validate_blob(
        blob(vec![CollabPoint {
            id: 1,
            vector: vec![0.5; 64],
        }]),
        1,
    );

    assert!(duplicate.is_err());
    assert!(non_finite.is_err());
    assert!(zero_id.is_err());
    assert!(short.is_err());
}

#[test]
fn vectors_must_hold_what_the_result_announced() {
    let fewer = validate_blob(blob(vec![point(1, 0.5)]), 2);
    let empty = validate_blob(blob(Vec::new()), 0);
    let other_dimension = validate_blob(
        CollabBlob {
            dim: 64,
            points: vec![point(1, 0.5)],
            metrics: None,
        },
        1,
    );

    assert!(fewer.is_err());
    assert!(empty.is_err());
    assert!(other_dimension.is_err());
}

#[test]
fn the_worker_vectors_file_is_read_with_its_metrics() -> anyhow::Result<()> {
    let vector = vec![0.125_f32; 128];
    let payload = json!({
        "dim": 128,
        "points": [{ "id": 42, "vec": vector }],
        "metrics": { "hr_at_20": 0.31, "popularity_hr_at_20": 0.12, "sessions": 5000, "vocab": 1 }
    });

    let mut blob: CollabBlob = serde_json::from_value(payload)?;
    let metrics = blob.metrics.take();
    let points = validate_blob(blob, 1)?;

    assert_eq!(points.first().map(|(id, _)| *id), Some(42));
    assert_eq!(metrics.map(|metrics| metrics.vocab), Some(1));
    Ok(())
}
