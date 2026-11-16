use std::sync::Arc;

use serde_json::json;

use super::*;
use crate::llm::test_support::{FIRST, SECOND, Scripted, refine_ladder};

const WANTED: MatchTarget<'static> = MatchTarget {
    artist: "Billie Eilish",
    title: "Ocean Eyes",
};

fn matcher(pool: &PgPool, worker: &Arc<Scripted>) -> anyhow::Result<AiMatcherClient> {
    let config = AiConfig {
        refine: None,
        timeout_ms: 10_000,
        daily_budget: 1,
    };
    Ok(AiMatcherClient::new(
        refine_ladder(worker.clone())?,
        Arc::new(Normalizer::new()?),
        pool.clone(),
        &config,
    ))
}

fn offer(id: u32, artist: &'static str, title: &'static str) -> MatchCandidate<'static> {
    MatchCandidate {
        id,
        artist,
        title,
        duration_sec: Some(200),
    }
}

fn tied() -> Vec<MatchCandidate<'static>> {
    vec![
        offer(0, "Billie Eilish", "Ocean Eyes"),
        offer(1, "billie", "Ocean Eyes"),
    ]
}

fn cache_key(candidates: &[MatchCandidate]) -> String {
    format!(
        "match:{}",
        request_hash(&match_request(&WANTED, candidates))
    )
}

async fn budget_left(pool: &PgPool) -> bool {
    AiStore::new(pool.clone(), 1).take_budget().await
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_clear_winner_needs_no_model_and_no_budget(pool: PgPool) -> anyhow::Result<()> {
    let worker = Arc::new(Scripted::default());
    let candidates = vec![
        offer(0, "Someone", "Different Song"),
        offer(1, "Billie Eilish", "Ocean Eyes"),
    ];

    let picked = matcher(&pool, &worker)?
        .pick(WANTED, &candidates)
        .await
        .map_err(|_| anyhow::anyhow!("the matcher was unavailable"))?
        .ok_or_else(|| anyhow::anyhow!("no match"))?;

    assert_eq!((picked.candidate_id, picked.confidence), (1, 0.95));
    assert!(worker.asked().is_empty());
    assert!(budget_left(&pool).await);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_borderline_pick_moves_past_an_answer_outside_the_shortlist(
    pool: PgPool,
) -> anyhow::Result<()> {
    let worker = Arc::new(Scripted::default());
    worker.answer(FIRST, json!({"match_id": 7, "confidence": 0.99}));
    worker.answer(SECOND, json!({"match_id": 1, "confidence": 0.8}));
    let candidates = tied();

    let picked = matcher(&pool, &worker)?
        .pick(WANTED, &candidates)
        .await
        .map_err(|_| anyhow::anyhow!("the matcher was unavailable"))?
        .ok_or_else(|| anyhow::anyhow!("no match"))?;

    assert_eq!((picked.candidate_id, picked.confidence), (1, 0.8));
    let asked = worker.asked();
    let subjects: Vec<&str> = asked.iter().map(|(subject, _)| subject.as_str()).collect();
    assert_eq!(subjects, [FIRST, SECOND]);
    let question = asked
        .first()
        .and_then(|(_, payload)| payload["messages"][0]["content"].as_str())
        .unwrap_or_default();
    assert!(
        question.contains("- id=0 artist='Billie Eilish' title='Ocean Eyes' duration_sec=200.0")
    );
    let cached: Option<MatchTrackData> = AiStore::new(pool.clone(), 1)
        .cached(&cache_key(&candidates))
        .await;
    assert_eq!(
        cached.map(|answer| (answer.match_id, answer.source)),
        Some((Some(1), RpcSource::Llm))
    );
    assert!(!budget_left(&pool).await);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn the_model_may_say_none_of_them_is_the_recording(pool: PgPool) -> anyhow::Result<()> {
    let worker = Arc::new(Scripted::default());
    worker.answer(FIRST, json!({"match_id": null, "confidence": 0.7}));
    let candidates = vec![offer(0, "Billie Eilish", "Blue Eyes")];

    let picked = matcher(&pool, &worker)?
        .pick(WANTED, &candidates)
        .await
        .map_err(|_| anyhow::anyhow!("the matcher was unavailable"))?;

    assert!(picked.is_none());
    assert_eq!(worker.asked().len(), 1);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_ladder_without_an_answer_keeps_the_deterministic_verdict_and_refunds(
    pool: PgPool,
) -> anyhow::Result<()> {
    let worker = Arc::new(Scripted::default());
    let candidates = vec![offer(0, "Billie Eilish", "Blue Eyes")];

    let picked = matcher(&pool, &worker)?
        .pick(WANTED, &candidates)
        .await
        .map_err(|_| anyhow::anyhow!("the matcher was unavailable"))?
        .ok_or_else(|| anyhow::anyhow!("the deterministic verdict was lost"))?;

    assert_eq!(picked.candidate_id, 0);
    assert!((picked.confidence - 0.784_210_5).abs() < 1e-6);
    assert_eq!(worker.asked().len(), 2);
    let cached: Option<MatchTrackData> = AiStore::new(pool.clone(), 1)
        .cached(&cache_key(&candidates))
        .await;
    assert!(cached.is_none());
    assert!(budget_left(&pool).await);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn an_exhausted_daily_budget_makes_the_matcher_unavailable(
    pool: PgPool,
) -> anyhow::Result<()> {
    let worker = Arc::new(Scripted::default());
    assert!(budget_left(&pool).await);

    let picked = matcher(&pool, &worker)?.pick(WANTED, &tied()).await;

    assert_eq!(picked.err(), Some(AiUnavailable));
    assert!(worker.asked().is_empty());
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_cached_model_answer_is_reused_without_a_call(pool: PgPool) -> anyhow::Result<()> {
    let worker = Arc::new(Scripted::default());
    let candidates = tied();
    let stored = MatchTrackData {
        match_id: Some(0),
        confidence: 0.9,
        source: RpcSource::Llm,
    };
    AiStore::new(pool.clone(), 1)
        .remember(&cache_key(&candidates), &stored)
        .await;

    let picked = matcher(&pool, &worker)?
        .pick(WANTED, &candidates)
        .await
        .map_err(|_| anyhow::anyhow!("the matcher was unavailable"))?
        .ok_or_else(|| anyhow::anyhow!("no match"))?;

    assert_eq!((picked.candidate_id, picked.confidence), (0, 0.9));
    assert!(worker.asked().is_empty());
    assert!(budget_left(&pool).await);
    Ok(())
}
